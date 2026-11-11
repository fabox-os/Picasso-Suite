//! Follows one image point from frame to frame: normalised cross-correlation template matching,
//! coarse to fine over an image pyramid, then a Gauss-Newton refinement for sub-pixel accuracy.
//! The result is invariant to brightness and contrast changes.

use crate::image::{GrayImage, Vec2};
use crate::matcher::{best_integer, correlation_real, refine, Patch};

/// The coarsest level keeps a pattern at least this wide.
const MIN_PATTERN_AT_TOP: usize = 9;
/// Integer search around the scaled-up coarse result.
const REFINE_RADIUS: i64 = 2;
const REFINE_MAX_MOVE: f64 = 1.5;

/// How a point is followed.
#[derive(Clone, Copy, Debug)]
pub struct TrackerSettings {
    /// Side of the square pattern box, pixels (at least 5).
    pub pattern_size: usize,
    /// Side of the square search box around the last position, pixels.
    pub search_size: usize,
    /// A match below this normalised correlation (-1..1) is a failure.
    pub min_correlation: f64,
    /// A match below this (but above `min_correlation`) re-takes the pattern from the current
    /// frame, so a slowly changing target stays followed. 1 = every frame, 0 = never.
    pub refresh_below: f64,
    /// Pyramid levels at most (the pattern is kept at least about 8 px wide).
    pub max_levels: usize,
}

impl Default for TrackerSettings {
    fn default() -> Self {
        Self {
            pattern_size: 31,
            search_size: 101,
            min_correlation: 0.6,
            refresh_below: 0.85,
            max_levels: 4,
        }
    }
}

/// Result of following the point into one more frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrackResult {
    /// False: the pattern was lost (or the point left the picture).
    pub ok: bool,
    /// The new position (the old one when `!ok`).
    pub position: Vec2,
    /// Normalised correlation of the final match, -1..1.
    pub correlation: f64,
}

/// Follows one point through a sequence of frames, in either direction. Create, [`reset`] on the
/// first frame, then [`track`] each next frame.
///
/// [`reset`]: PointTracker::reset
/// [`track`]: PointTracker::track
#[derive(Clone, Debug)]
pub struct PointTracker {
    settings: TrackerSettings,
    position: Vec2,
    levels: usize,
    patches: Vec<Patch>,
    ready: bool,
}

impl PointTracker {
    pub fn new(mut settings: TrackerSettings) -> Self {
        settings.pattern_size = settings.pattern_size.max(5);
        settings.search_size = settings.search_size.max(settings.pattern_size);
        settings.max_levels = settings.max_levels.max(1);
        Self {
            settings,
            position: Vec2::default(),
            levels: 1,
            patches: Vec::new(),
            ready: false,
        }
    }

    pub fn position(&self) -> Vec2 {
        self.position
    }

    /// Moves the point without taking a new pattern (a mesh tracker carries a lost point along
    /// with its neighbours).
    pub fn set_position(&mut self, position: Vec2) {
        self.position = position;
    }

    pub fn settings(&self) -> &TrackerSettings {
        &self.settings
    }

    fn level_count(&self, image: &GrayImage) -> usize {
        let mut levels = 1;
        let mut pattern = self.settings.pattern_size;
        let (mut w, mut h) = (image.width(), image.height());
        while levels < self.settings.max_levels
            && pattern / 2 >= MIN_PATTERN_AT_TOP
            && w >= 2
            && h >= 2
        {
            pattern /= 2;
            w /= 2;
            h /= 2;
            levels += 1;
        }
        levels
    }

    fn pyramid(image: &GrayImage, levels: usize) -> Vec<GrayImage> {
        let mut out = Vec::with_capacity(levels);
        out.push(image.clone());
        for l in 1..levels {
            let next = out[l - 1].halved();
            out.push(next);
        }
        out
    }

    fn take_patches(&mut self, pyramid: &[GrayImage], position: Vec2) {
        self.patches = pyramid
            .iter()
            .enumerate()
            .map(|(l, image)| {
                let scale = 1.0 / f64::from(1u32 << l);
                let size = ((self.settings.pattern_size >> l).max(5)) | 1;
                Patch::extract(
                    image,
                    Vec2::new(position.x * scale, position.y * scale),
                    size,
                )
            })
            .collect();
    }

    /// Starts on `frame` with the point at `position`: the pattern is taken from there. False if
    /// the picture is empty or the pattern is flat (nothing to follow).
    pub fn reset(&mut self, frame: &GrayImage, position: Vec2) -> bool {
        self.ready = false;
        if frame.is_empty() || !position.is_finite() {
            return false;
        }
        self.position = position;
        self.levels = self.level_count(frame);
        let pyramid = Self::pyramid(frame, self.levels);
        self.take_patches(&pyramid, position);
        self.ready = self.patches.first().is_some_and(|p| p.norm > 1e-6);
        self.ready
    }

    /// Finds the point in `next`, searching around the last position.
    pub fn track(&mut self, next: &GrayImage) -> TrackResult {
        let mut result = TrackResult {
            position: self.position,
            ..TrackResult::default()
        };
        if !self.ready || next.is_empty() {
            return result;
        }
        let pyramid = Self::pyramid(next, self.levels);
        let top = self.levels - 1;
        let top_scale = 1.0 / f64::from(1u32 << top);
        let top_patch = &self.patches[top];
        let radius =
            (((self.settings.search_size as f64 - 1.0) / 2.0 * top_scale).ceil() as i64).max(1);
        let Some((mut bx, mut by, mut best)) = best_integer(
            &pyramid[top],
            (self.position.x * top_scale).round() as i64,
            (self.position.y * top_scale).round() as i64,
            radius,
            &top_patch.values,
            top_patch.size,
            top_patch.norm,
        ) else {
            return result;
        };
        // Finer levels: scale up and look a few pixels around.
        for l in (0..top).rev() {
            let patch = &self.patches[l];
            let Some((nx, ny, c)) = best_integer(
                &pyramid[l],
                bx * 2,
                by * 2,
                REFINE_RADIUS,
                &patch.values,
                patch.size,
                patch.norm,
            ) else {
                return result;
            };
            bx = nx;
            by = ny;
            best = c;
        }
        let base = &self.patches[0];
        let image = &pyramid[0];
        let mut p = refine(
            image,
            Vec2::new(bx as f64, by as f64),
            &base.values,
            base.size,
            base.norm,
            REFINE_MAX_MOVE,
        );
        let mut corr = correlation_real(image, p, &base.values, base.size, base.norm);
        if corr < best {
            // refinement never makes the match worse
            p = Vec2::new(bx as f64, by as f64);
            corr = best;
        }
        result.correlation = corr;
        if corr < self.settings.min_correlation {
            return result;
        }
        result.ok = true;
        result.position = p;
        self.position = p;
        if corr < self.settings.refresh_below {
            self.take_patches(&pyramid, p);
        }
        result
    }
}

impl Default for PointTracker {
    fn default() -> Self {
        Self::new(TrackerSettings::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{frame, Warp};

    #[test]
    fn follows_sub_pixel_offsets() {
        let start = Vec2::new(150.0, 110.0);
        for (dx, dy) in [
            (0.0, 0.0),
            (0.5, 0.5),
            (3.37, -2.81),
            (-4.25, 1.1),
            (0.13, 0.9),
        ] {
            let mut t = PointTracker::default();
            assert!(t.reset(&frame(&Warp::shift(0.0, 0.0), 0.0, 1), start));
            let r = t.track(&frame(&Warp::shift(dx, dy), 0.0, 2));
            assert!(r.ok, "lost at ({dx}, {dy})");
            let err = r.position.distance(Vec2::new(start.x + dx, start.y + dy));
            assert!(err < 0.05, "error {err} at ({dx}, {dy})");
        }
    }

    #[test]
    fn follows_through_noise_and_brightness_change() {
        let start = Vec2::new(150.0, 110.0);
        let mut t = PointTracker::default();
        assert!(t.reset(&frame(&Warp::shift(0.0, 0.0), 0.0, 1), start));
        let mut next = frame(&Warp::shift(6.0, -3.0), 0.02, 3);
        let data: Vec<f32> = next.data().iter().map(|v| v * 0.7 + 0.1).collect();
        next = GrayImage::new(next.width(), next.height(), data).unwrap();
        let r = t.track(&next);
        assert!(r.ok);
        assert!(r.position.distance(Vec2::new(156.0, 107.0)) < 0.3);
    }

    #[test]
    fn loses_a_vanished_pattern_and_refuses_a_flat_one() {
        let mut t = PointTracker::default();
        assert!(t.reset(
            &frame(&Warp::shift(0.0, 0.0), 0.0, 1),
            Vec2::new(150.0, 110.0)
        ));
        let flat = GrayImage::new(320, 240, vec![0.5; 320 * 240]).unwrap();
        let r = t.track(&flat);
        assert!(!r.ok);
        assert_eq!(r.position, Vec2::new(150.0, 110.0));
        assert!(!PointTracker::default().reset(&flat, Vec2::new(100.0, 100.0)));
    }

    #[test]
    fn a_point_near_the_edge_does_not_panic() {
        let mut t = PointTracker::default();
        let f = frame(&Warp::shift(0.0, 0.0), 0.0, 1);
        let _ = t.reset(&f, Vec2::new(2.0, 2.0));
        let _ = t.track(&f);
        let _ = t.reset(&f, Vec2::new(f64::NAN, 0.0));
    }
}
