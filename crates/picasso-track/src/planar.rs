//! Follows a flat surface (a four-corner region) from frame to frame. A grid of patches on the
//! plane is matched in every new frame with the point tracker's matcher, the patches first warped
//! by the homography found for the previous frame so that perspective and rotation do not spoil
//! the match. RANSAC with a normalised DLT turns the matches into the plane's homography, which
//! re-warps the patches for a second, finer pass. The patches are taken from the start frame (no
//! drift) and re-taken from the current frame when the matches get weak.

use crate::homography::{
    estimate_homography, homography_from_quads, is_convex_quad, quad_area, Homography, Quad,
    RansacSettings,
};
use crate::image::{GrayImage, Vec2};
use crate::matcher::{best_integer, correlation_real, refine, zero_mean};

/// Fewer usable patches cannot pin down a homography reliably.
const MIN_PATCHES: usize = 6;
const MIN_PATCH: i64 = 9;
const MAX_PATCH: i64 = 31;
/// Patch side / mean side of the region.
const PATCH_FRACTION: f64 = 0.18;
/// The grid keeps this fraction of the region free at the edge.
const GRID_MARGIN: f64 = 0.1;
/// Standard deviation of a patch below which it is flat.
const MIN_CONTRAST: f64 = 0.012;
/// The second pass looks only this far from the first result.
const FINE_PASS_RADIUS: i64 = 3;
/// Smallest region worth tracking.
const MIN_AREA_PIXELS: f64 = 64.0;
/// The region may grow / shrink this much from the start.
const AREA_CHANGE_LIMIT: f64 = 40.0;
const REFINE_MAX_MOVE: f64 = 1.5;
/// Searches wider than this many pixels look first on half-size pictures (about 10 times faster
/// at the default radius), then around that result at full size.
const COARSE_ABOVE: i64 = 6;
const COARSE_TO_FINE_RADIUS: i64 = 3;
const MIN_COARSE_PATCH: usize = 5;
/// The second pass asks at least this much of a patch (a partly hidden patch would only pull the
/// fit), unless that leaves too few patches.
const FINE_PASS_MIN_CORRELATION: f64 = 0.85;

/// How a plane is followed.
#[derive(Clone, Copy, Debug)]
pub struct PlanarSettings {
    /// Patches per side of the grid laid on the plane (at least 3).
    pub grid_size: usize,
    /// Side of a patch, pixels; 0 = from the size of the region.
    pub patch_size: usize,
    /// How far a patch may move between two frames, pixels.
    pub search_radius: i64,
    /// A patch matching worse than this is ignored.
    pub min_correlation: f64,
    /// Fraction of the usable patches that must agree on one homography; fewer: the plane is lost.
    pub min_inlier_fraction: f64,
    /// Mean correlation of the agreeing patches below which the patches are re-taken from the
    /// current frame. 0 = never.
    pub refresh_below: f64,
    /// RANSAC distance, pixels.
    pub inlier_threshold: f64,
}

impl Default for PlanarSettings {
    fn default() -> Self {
        Self {
            grid_size: 7,
            patch_size: 0,
            search_radius: 24,
            min_correlation: 0.6,
            min_inlier_fraction: 0.4,
            refresh_below: 0.85,
            inlier_threshold: 1.0,
        }
    }
}

/// Result of following the plane into one more frame.
#[derive(Clone, Debug)]
pub struct PlanarResult {
    /// False: the plane was lost (the corners stay where they were).
    pub ok: bool,
    /// The region's corners in the new frame, image pixels.
    pub corners: Quad,
    /// Start-frame pixels -> this frame's pixels.
    pub motion: Homography,
    /// Mean correlation of the patches that agree.
    pub correlation: f64,
    /// Patches that agree with the homography.
    pub inliers: usize,
    /// Patches that were matched at all.
    pub patches: usize,
    /// The patches that agree with `motion`: where each was on the plane (start-frame pixels) and
    /// where it was found in this frame. Equal length; empty when the plane was lost.
    pub inlier_from: Vec<Vec2>,
    pub inlier_to: Vec<Vec2>,
}

/// The patch of `image` around `centre`, sampled at `map(centre + offset)`, zero mean. Returns the
/// values, the root of their sum of squares and their standard deviation; `None` when a sample
/// falls outside the picture. `step` 2 is for a half-size `image`: patch pixels are two full-size
/// pixels apart and the positions are halved before sampling.
fn warped_patch(
    image: &GrayImage,
    map: &Homography,
    centre: Vec2,
    size: usize,
    step: f64,
) -> Option<(Vec<f32>, f64, f64)> {
    let half = (size / 2) as f64;
    let mut values = Vec::with_capacity(size * size);
    for j in 0..size {
        for i in 0..size {
            let mut p = map.map(Vec2::new(
                centre.x + (i as f64 - half) * step,
                centre.y + (j as f64 - half) * step,
            ));
            p.x /= step;
            p.y /= step;
            if !(p.x >= 0.0
                && p.y >= 0.0
                && p.x <= image.width() as f64 - 1.0
                && p.y <= image.height() as f64 - 1.0)
            {
                return None;
            }
            values.push(image.sample(p.x, p.y));
        }
    }
    let norm = zero_mean(&mut values);
    let contrast = norm / (values.len() as f64).sqrt();
    (norm > 0.0).then_some((values, norm, contrast))
}

/// One patch match: the plane point, where it was found and how well.
struct Match {
    from: Vec2,
    to: Vec2,
    correlation: f64,
}

/// Follows one plane through a sequence of frames, in either direction. Create, [`reset`] on the
/// first frame, then [`track`] each next frame.
///
/// [`reset`]: PlanarTracker::reset
/// [`track`]: PlanarTracker::track
#[derive(Clone, Debug)]
pub struct PlanarTracker {
    settings: PlanarSettings,
    start_corners: Quad,
    corners: Quad,
    template: GrayImage,
    template_half: GrayImage,
    /// Start-frame pixels -> template pixels.
    template_map: Homography,
    /// Start-frame pixels -> latest frame pixels.
    motion: Homography,
    /// Patch centres on the plane, in start-frame pixels.
    points: Vec<Vec2>,
    /// Patch side, pixels (odd).
    patch: usize,
    ready: bool,
}

impl PlanarTracker {
    pub fn new(mut settings: PlanarSettings) -> Self {
        settings.grid_size = settings.grid_size.max(3);
        settings.search_radius = settings.search_radius.max(2);
        let zero = [Vec2::default(); 4];
        Self {
            settings,
            start_corners: zero,
            corners: zero,
            template: GrayImage::default(),
            template_half: GrayImage::default(),
            template_map: Homography::default(),
            motion: Homography::default(),
            points: Vec::new(),
            patch: 0,
            ready: false,
        }
    }

    pub fn corners(&self) -> &Quad {
        &self.corners
    }

    pub fn settings(&self) -> &PlanarSettings {
        &self.settings
    }

    /// Starts on `frame` with the region `corners` (a convex quad inside the picture). False if the
    /// region is degenerate or has too little detail (fewer than 6 usable patches).
    pub fn reset(&mut self, frame: &GrayImage, corners: &Quad) -> bool {
        self.ready = false;
        if frame.is_empty() || !is_convex_quad(corners, MIN_AREA_PIXELS) {
            return false;
        }
        self.start_corners = *corners;
        self.corners = *corners;
        self.template = frame.clone();
        self.template_half = frame.halved();
        self.template_map = Homography::default();
        self.motion = Homography::default();

        let side = (0..4)
            .map(|i| corners[i].distance(corners[(i + 1) % 4]))
            .sum::<f64>()
            / 4.0;
        self.patch = if self.settings.patch_size > 0 {
            self.settings.patch_size.max(5) | 1
        } else {
            ((side * PATCH_FRACTION)
                .round()
                .clamp(MIN_PATCH as f64, MAX_PATCH as f64) as usize)
                | 1
        };

        // Lay the grid on the plane (not on the screen), so a region seen in perspective gets its
        // patches spread evenly over the surface.
        let unit: Quad = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let Some(plane) = homography_from_quads(&unit, corners) else {
            return false;
        };
        self.points.clear();
        let g = self.settings.grid_size;
        let identity = Homography::default();
        for j in 0..g {
            for i in 0..g {
                let u = GRID_MARGIN + (1.0 - 2.0 * GRID_MARGIN) * i as f64 / (g - 1) as f64;
                let v = GRID_MARGIN + (1.0 - 2.0 * GRID_MARGIN) * j as f64 / (g - 1) as f64;
                let p = plane.map(Vec2::new(u, v));
                if let Some((_, _, contrast)) = warped_patch(frame, &identity, p, self.patch, 1.0) {
                    if contrast >= MIN_CONTRAST {
                        self.points.push(p);
                    }
                }
            }
        }
        self.ready = self.points.len() >= MIN_PATCHES;
        self.ready
    }

    fn match_patches(
        &self,
        next: &GrayImage,
        next_half: &GrayImage,
        predicted: &Homography,
        radius: i64,
        min_correlation: f64,
    ) -> Vec<Match> {
        let mut out = Vec::new();
        let Some(back) = predicted.inverse() else {
            return out;
        };
        // Current-frame pixel -> template pixel: undo the predicted motion, then go to the template.
        let to_template = self.template_map.after(&back);
        for &p in &self.points {
            let q = predicted.map(p);
            if !(q.x.abs() < 1e7 && q.y.abs() < 1e7) {
                continue;
            }
            let Some((values, norm, contrast)) =
                warped_patch(&self.template, &to_template, q, self.patch, 1.0)
            else {
                continue;
            };
            if contrast < MIN_CONTRAST {
                continue;
            }
            let (mut bx, mut by) = (q.x.round() as i64, q.y.round() as i64);
            let mut search_radius = radius;
            if radius > COARSE_ABOVE {
                // Coarse: the same patch at half resolution, searched on the half-size picture.
                let coarse = (self.patch / 2).max(MIN_COARSE_PATCH) | 1;
                if let Some((cv, cn, _)) =
                    warped_patch(&self.template_half, &to_template, q, coarse, 2.0)
                {
                    if let Some((cx, cy, _)) = best_integer(
                        next_half,
                        (q.x / 2.0).round() as i64,
                        (q.y / 2.0).round() as i64,
                        radius / 2 + 1,
                        &cv,
                        coarse,
                        cn,
                    ) {
                        bx = cx * 2;
                        by = cy * 2;
                        search_radius = COARSE_TO_FINE_RADIUS;
                    }
                }
            }
            let Some((bx, by, best)) =
                best_integer(next, bx, by, search_radius, &values, self.patch, norm)
            else {
                continue;
            };
            let mut pos = Vec2::new(bx as f64, by as f64);
            let fine = refine(next, pos, &values, self.patch, norm, REFINE_MAX_MOVE);
            let mut corr = correlation_real(next, fine, &values, self.patch, norm);
            if corr >= best {
                pos = fine;
            } else {
                corr = best; // the refinement never makes a match worse
            }
            if corr < min_correlation {
                continue;
            }
            out.push(Match {
                from: p,
                to: pos,
                correlation: corr,
            });
        }
        out
    }

    /// Finds the plane in `next`.
    pub fn track(&mut self, next: &GrayImage) -> PlanarResult {
        let mut result = PlanarResult {
            ok: false,
            corners: self.corners,
            motion: self.motion,
            correlation: 0.0,
            inliers: 0,
            patches: 0,
            inlier_from: Vec::new(),
            inlier_to: Vec::new(),
        };
        if !self.ready || next.is_empty() {
            return result;
        }
        let next_half = next.halved();
        let mut current = self.motion;
        let mut inliers: Vec<bool> = Vec::new();
        let mut matches: Vec<Match> = Vec::new();
        let mut matched = 0;
        let mut coarse_pass = true;
        for radius in [self.settings.search_radius, FINE_PASS_RADIUS] {
            let mut found = self.match_patches(
                next,
                &next_half,
                &current,
                radius,
                if coarse_pass {
                    self.settings.min_correlation
                } else {
                    self.settings.min_correlation.max(FINE_PASS_MIN_CORRELATION)
                },
            );
            if !coarse_pass && found.len() < MIN_PATCHES.max(inliers.len() / 2) {
                // Too few good matches for the strict pass: match again with the normal limit.
                found = self.match_patches(
                    next,
                    &next_half,
                    &current,
                    radius,
                    self.settings.min_correlation,
                );
            }
            coarse_pass = false;
            matched = found.len();
            if matched < MIN_PATCHES {
                return result;
            }
            let from: Vec<Vec2> = found.iter().map(|m| m.from).collect();
            let to: Vec<Vec2> = found.iter().map(|m| m.to).collect();
            let ransac = RansacSettings {
                inlier_threshold: self.settings.inlier_threshold,
                ..RansacSettings::default()
            };
            let Some(fit) = estimate_homography(&from, &to, &ransac) else {
                return result;
            };
            if fit.inlier_count < MIN_PATCHES
                || (fit.inlier_count as f64)
                    < self.settings.min_inlier_fraction * self.points.len() as f64
            {
                return result;
            }
            current = fit.h;
            inliers = fit.inliers;
            matches = found;
        }

        let mut found_corners = self.start_corners;
        for c in &mut found_corners {
            *c = current.map(*c);
        }
        let area = quad_area(&found_corners);
        let start_area = quad_area(&self.start_corners);
        if !is_convex_quad(&found_corners, MIN_AREA_PIXELS)
            || area > start_area * AREA_CHANGE_LIMIT
            || area < start_area / AREA_CHANGE_LIMIT
        {
            return result;
        }

        let mut sum = 0.0;
        let mut count = 0;
        for (m, &inlier) in matches.iter().zip(&inliers) {
            if inlier {
                sum += m.correlation;
                count += 1;
                result.inlier_from.push(m.from);
                result.inlier_to.push(m.to);
            }
        }
        result.ok = true;
        result.corners = found_corners;
        result.motion = current;
        result.inliers = count;
        result.patches = matched;
        result.correlation = if count > 0 { sum / count as f64 } else { 0.0 };
        self.corners = found_corners;
        self.motion = current;
        if result.correlation < self.settings.refresh_below {
            // The look has changed: cut the patches from this frame from now on.
            self.template = next.clone();
            self.template_half = next_half;
            self.template_map = current;
        }
        result
    }
}

impl Default for PlanarTracker {
    fn default() -> Self {
        Self::new(PlanarSettings::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{frame, Warp};

    const REGION: Quad = [
        Vec2::new(80.0, 60.0),
        Vec2::new(240.0, 60.0),
        Vec2::new(240.0, 180.0),
        Vec2::new(80.0, 180.0),
    ];

    #[test]
    fn follows_a_moving_and_turning_plane() {
        let mut t = PlanarTracker::default();
        assert!(t.reset(&frame(&Warp::shift(0.0, 0.0), 0.0, 1), &REGION));
        let warp = Warp::similarity(3.0, 1.04, 7.5, -4.25);
        let r = t.track(&frame(&warp, 0.0, 2));
        assert!(r.ok, "plane lost");
        for (c, start) in r.corners.iter().zip(&REGION) {
            assert!(c.distance(warp.0.map(*start)) < 0.3, "corner off: {c:?}");
        }
        assert!(r.inliers >= 20);
    }

    #[test]
    fn follows_through_several_frames() {
        let mut t = PlanarTracker::default();
        assert!(t.reset(&frame(&Warp::shift(0.0, 0.0), 0.0, 1), &REGION));
        let mut last = None;
        for k in 1..=5 {
            let warp = Warp::similarity(
                f64::from(k),
                1.0 + 0.01 * f64::from(k),
                3.0 * f64::from(k),
                2.0 * f64::from(k),
            );
            let r = t.track(&frame(&warp, 0.01, 10 + k as u32));
            assert!(r.ok, "lost at frame {k}");
            last = Some((r, warp));
        }
        let (r, warp) = last.unwrap();
        assert!(r.corners[2].distance(warp.0.map(REGION[2])) < 0.5);
    }

    #[test]
    fn loses_a_plane_that_vanishes() {
        let mut t = PlanarTracker::default();
        assert!(t.reset(&frame(&Warp::shift(0.0, 0.0), 0.0, 1), &REGION));
        let flat = GrayImage::new(320, 240, vec![0.5; 320 * 240]).unwrap();
        let r = t.track(&flat);
        assert!(!r.ok);
        assert_eq!(r.corners, REGION);
        assert!(r.inlier_from.is_empty());
    }

    #[test]
    fn refuses_a_flat_or_degenerate_region() {
        let flat = GrayImage::new(320, 240, vec![0.5; 320 * 240]).unwrap();
        assert!(!PlanarTracker::default().reset(&flat, &REGION));
        let line = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(20.0, 0.0),
            Vec2::new(30.0, 0.0),
        ];
        let f = frame(&Warp::shift(0.0, 0.0), 0.0, 1);
        assert!(!PlanarTracker::default().reset(&f, &line));
    }
}
