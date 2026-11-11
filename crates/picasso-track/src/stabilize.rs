//! Measures the camera movement of a clip. Every pair of neighbouring frames goes through the
//! [`PlanarTracker`] (a grid of patches over the whole picture, RANSAC against moving objects);
//! the patches that agree are fitted with the model of the chosen [`Mode`] (a move, a move + turn
//! + scale, or the homography itself), and the pair motions are chained into the camera path.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::homography::{homography_from_points, similarity_from_points, Homography, Quad};
use crate::image::{GrayImage, Vec2};
use crate::planar::{PlanarSettings, PlanarTracker};

/// The region measured keeps this fraction free at each edge.
const REGION_MARGIN: f64 = 0.1;
/// How far a patch may move between frames, of the width.
const SEARCH_FRACTION: f64 = 0.06;
const MIN_SEARCH: i64 = 8;
const GRID_SIZE: usize = 9;
const INLIER_PIXELS: f64 = 1.5;
const TRIM_ROUNDS: usize = 4;
const MIN_FIT_PAIRS: usize = 8;
/// Pairs further than this many median residuals are dropped.
const TRIM_FACTOR: f64 = 2.5;
const TRIM_FLOOR_PIXELS: f64 = 0.3;

/// What the camera movement is described by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A move.
    Translation,
    /// A move, a turn and a scale.
    Similarity,
    /// The full homography.
    Perspective,
}

/// Motion of the scene between two frames, in normalised picture coordinates (origin at the
/// picture centre, one unit = the picture width): a point at `p` in the first frame is at `m * p`
/// in the second.
#[derive(Clone, Copy, Debug)]
pub struct FrameMotion {
    /// False: the frames could not be related (the motion is the identity).
    pub ok: bool,
    pub m: Homography,
    /// Patches that agree.
    pub inliers: usize,
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        0.5 * (v[mid] + v[mid - 1])
    } else {
        v[mid]
    }
}

/// Pixels -> normalised picture coordinates.
fn normalised(p: Vec2, width: usize, height: usize) -> Vec2 {
    let w = width as f64;
    Vec2::new(
        (p.x - 0.5 * (w - 1.0)) / w,
        (p.y - 0.5 * (height as f64 - 1.0)) / w,
    )
}

fn translation(dx: f64, dy: f64) -> Homography {
    Homography {
        m: [1.0, 0.0, dx, 0.0, 1.0, dy, 0.0, 0.0, 1.0],
    }
}

/// `m` rebuilt inside the model of `mode`, so a chained path cannot drift off it.
pub fn project_to_model(mode: Mode, m: &Homography) -> Homography {
    let a = &m.m;
    match mode {
        Mode::Translation => translation(a[2], a[5]),
        Mode::Similarity => {
            let angle = a[3].atan2(a[0]);
            let s = a[0].hypot(a[3]).max(1e-9);
            let (n, c) = angle.sin_cos();
            let (c, n) = (s * c, s * n);
            Homography {
                m: [c, -n, a[2], n, c, a[5], 0.0, 0.0, 1.0],
            }
        }
        Mode::Perspective => {
            let w = if a[8].abs() > 1e-12 { a[8] } else { 1.0 };
            let mut r = [0.0; 9];
            for i in 0..8 {
                r[i] = a[i] / w;
            }
            r[8] = 1.0;
            Homography { m: r }
        }
    }
}

fn residuals(from: &[Vec2], to: &[Vec2], map: &dyn Fn(Vec2) -> Vec2) -> Vec<f64> {
    from.iter()
        .zip(to)
        .map(|(f, t)| map(*f).distance(*t))
        .collect()
}

/// Keeps refitting on the pairs that agree with the fit, so the edge of a moving object or a
/// wrong match cannot bias it (trimmed least squares). `fit` returns the model's map.
fn trimmed_fit<M>(
    mut from: Vec<Vec2>,
    mut to: Vec<Vec2>,
    width: usize,
    fit: impl Fn(&[Vec2], &[Vec2]) -> Option<M>,
    map: impl Fn(&M, Vec2) -> Vec2,
) -> Option<M> {
    let mut model = fit(&from, &to)?;
    for _ in 0..TRIM_ROUNDS {
        if from.len() <= MIN_FIT_PAIRS {
            break;
        }
        let mut res = residuals(&from, &to, &|p| map(&model, p));
        let limit = (TRIM_FLOOR_PIXELS / width as f64).max(TRIM_FACTOR * median(&mut res));
        let kept: Vec<usize> = (0..from.len())
            .filter(|&i| map(&model, from[i]).distance(to[i]) <= limit)
            .collect();
        if kept.len() < MIN_FIT_PAIRS || kept.len() == from.len() {
            break;
        }
        let f: Vec<Vec2> = kept.iter().map(|&i| from[i]).collect();
        let t: Vec<Vec2> = kept.iter().map(|&i| to[i]).collect();
        let Some(refit) = fit(&f, &t) else { break };
        from = f;
        to = t;
        model = refit;
    }
    Some(model)
}

/// Estimates the motion between two frames of equal size with the model of `mode`.
pub fn estimate_frame_motion(previous: &GrayImage, next: &GrayImage, mode: Mode) -> FrameMotion {
    let none = FrameMotion {
        ok: false,
        m: Homography::default(),
        inliers: 0,
    };
    if previous.is_empty()
        || next.is_empty()
        || previous.width() != next.width()
        || previous.height() != next.height()
    {
        return none;
    }
    let (w, h) = (previous.width(), previous.height());
    let wf = w as f64;
    let settings = PlanarSettings {
        grid_size: GRID_SIZE,
        search_radius: MIN_SEARCH.max((SEARCH_FRACTION * wf) as i64),
        min_correlation: 0.5,
        min_inlier_fraction: 0.2,
        refresh_below: 0.0, // the pattern is the previous frame, nothing to refresh
        inlier_threshold: INLIER_PIXELS * (wf / 640.0).max(1.0),
        ..PlanarSettings::default()
    };
    let mut tracker = PlanarTracker::new(settings);
    let (mx, my) = (REGION_MARGIN * wf, REGION_MARGIN * h as f64);
    let (r, b) = (wf - 1.0 - mx, h as f64 - 1.0 - my);
    let region: Quad = [
        Vec2::new(mx, my),
        Vec2::new(r, my),
        Vec2::new(r, b),
        Vec2::new(mx, b),
    ];
    if !tracker.reset(previous, &region) {
        return none;
    }
    let mut found = tracker.track(next);
    if found.ok {
        // A second look starts from the first answer (the patches are warped by it), which
        // removes the bias of matching turned and scaled patches without the warp.
        let again = tracker.track(next);
        if again.ok && again.inlier_from.len() >= 3 {
            found = again;
        }
    }
    if !found.ok || found.inlier_from.len() < 3 {
        return none;
    }
    let from: Vec<Vec2> = found
        .inlier_from
        .iter()
        .map(|p| normalised(*p, w, h))
        .collect();
    let to: Vec<Vec2> = found
        .inlier_to
        .iter()
        .map(|p| normalised(*p, w, h))
        .collect();
    let inliers = from.len();
    let m = match mode {
        Mode::Translation => {
            let mut dx: Vec<f64> = from.iter().zip(&to).map(|(f, t)| t.x - f.x).collect();
            let mut dy: Vec<f64> = from.iter().zip(&to).map(|(f, t)| t.y - f.y).collect();
            translation(median(&mut dx), median(&mut dy))
        }
        Mode::Similarity => {
            let Some(s) = trimmed_fit(from, to, w, similarity_from_points, |s, p| s.map(p)) else {
                return none;
            };
            let (sn, c) = s.angle_rad.sin_cos();
            let (c, sn) = (s.scale * c, s.scale * sn);
            Homography {
                m: [c, -sn, s.dx, sn, c, s.dy, 0.0, 0.0, 1.0],
            }
        }
        Mode::Perspective => {
            // Refitted in normalised coordinates: better conditioned than the pixel homography.
            let Some(hm) = trimmed_fit(from, to, w, homography_from_points, |hm, p| hm.map(p))
            else {
                return none;
            };
            if !hm.is_finite() {
                return none;
            }
            hm.normalised()
        }
    };
    FrameMotion {
        ok: true,
        m,
        inliers,
    }
}

/// The camera path of a clip: where the camera is in each measured frame, relative to the first.
#[derive(Clone, Debug)]
pub struct CameraPath {
    pub mode: Mode,
    /// One matrix per frame (the first is the identity), in normalised picture coordinates.
    pub frames: Vec<Homography>,
    /// Height / width of the pictures measured.
    pub aspect: f64,
    /// Frame pairs measured.
    pub pairs: usize,
    /// ... of which could not be related (treated as no movement).
    pub lost: usize,
}

/// Why [`analyse`] gave no path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnalyseError {
    /// Fewer than two frames.
    TooShort,
    /// This frame could not be read (its number, counted from 0).
    Unreadable(usize),
    Cancelled,
}

/// Measures `frames` pictures, read with `frame_at(0..frames)`, and returns the camera path.
/// `progress(done, total)` is called after each frame; `cancel` is checked between frames.
pub fn analyse(
    mode: Mode,
    frames: usize,
    frame_at: &mut dyn FnMut(usize) -> Option<GrayImage>,
    progress: &mut dyn FnMut(usize, usize),
    cancel: Option<&AtomicBool>,
) -> Result<CameraPath, AnalyseError> {
    if frames < 2 {
        return Err(AnalyseError::TooShort);
    }
    let mut previous = frame_at(0)
        .filter(|f| !f.is_empty())
        .ok_or(AnalyseError::Unreadable(0))?;
    let aspect = previous.height() as f64 / previous.width() as f64;
    let mut camera = Homography::default();
    let mut path = Vec::with_capacity(frames);
    path.push(camera);
    progress(1, frames);
    let (mut pairs, mut lost) = (0, 0);
    for i in 1..frames {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(AnalyseError::Cancelled);
        }
        let next = frame_at(i)
            .filter(|f| !f.is_empty())
            .ok_or(AnalyseError::Unreadable(i))?;
        let motion = estimate_frame_motion(&previous, &next, mode);
        pairs += 1;
        if motion.ok {
            camera = motion.m.after(&camera);
        } else {
            lost += 1;
        }
        camera = project_to_model(mode, &camera); // chain in the model (no drift off it)
        path.push(camera);
        previous = next;
        progress(i + 1, frames);
    }
    Ok(CameraPath {
        mode,
        frames: path,
        aspect,
        pairs,
        lost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{frame, Warp, W};

    #[test]
    fn measures_a_known_shift() {
        let a = frame(&Warp::shift(0.0, 0.0), 0.0, 1);
        let b = frame(&Warp::shift(4.0, -2.0), 0.0, 2);
        for mode in [Mode::Translation, Mode::Similarity, Mode::Perspective] {
            let m = estimate_frame_motion(&a, &b, mode);
            assert!(m.ok, "{mode:?}");
            // normalised units: one unit = the picture width
            let dx = m.m.m[2] / m.m.m[8];
            let dy = m.m.m[5] / m.m.m[8];
            assert!((dx - 4.0 / W as f64).abs() < 0.0008, "{mode:?} dx {dx}");
            assert!((dy + 2.0 / W as f64).abs() < 0.0008, "{mode:?} dy {dy}");
        }
    }

    #[test]
    fn measures_a_turn() {
        let a = frame(&Warp::shift(0.0, 0.0), 0.0, 1);
        let b = frame(&Warp::similarity(2.0, 1.0, 0.0, 0.0), 0.0, 2);
        let m = estimate_frame_motion(&a, &b, Mode::Similarity);
        assert!(m.ok);
        let angle = m.m.m[3].atan2(m.m.m[0]);
        assert!((angle - 2f64.to_radians()).abs() < 0.003, "angle {angle}");
    }

    #[test]
    fn different_sizes_or_flat_frames_are_not_related() {
        let a = frame(&Warp::shift(0.0, 0.0), 0.0, 1);
        let small = GrayImage::new(10, 10, vec![0.5; 100]).unwrap();
        assert!(!estimate_frame_motion(&a, &small, Mode::Translation).ok);
        let flat = GrayImage::new(320, 240, vec![0.5; 320 * 240]).unwrap();
        assert!(!estimate_frame_motion(&a, &flat, Mode::Translation).ok);
    }

    #[test]
    fn chains_a_path_and_reports_progress() {
        let mut seen = 0;
        let path = analyse(
            Mode::Translation,
            4,
            &mut |i| Some(frame(&Warp::shift(3.0 * i as f64, 0.0), 0.0, 1 + i as u32)),
            &mut |done, _| seen = done,
            None,
        )
        .unwrap();
        assert_eq!(seen, 4);
        assert_eq!(path.frames.len(), 4);
        assert_eq!(path.lost, 0);
        let last = path.frames[3].m[2];
        assert!((last - 9.0 / W as f64).abs() < 0.002, "last {last}");
    }

    #[test]
    fn errors_and_cancel() {
        let mut none = |_: usize| None;
        let mut nop = |_: usize, _: usize| {};
        assert_eq!(
            analyse(Mode::Translation, 1, &mut none, &mut nop, None).unwrap_err(),
            AnalyseError::TooShort
        );
        assert_eq!(
            analyse(Mode::Translation, 3, &mut none, &mut nop, None).unwrap_err(),
            AnalyseError::Unreadable(0)
        );
        let stop = AtomicBool::new(true);
        let mut f = |_: usize| Some(frame(&Warp::shift(0.0, 0.0), 0.0, 1));
        assert_eq!(
            analyse(Mode::Translation, 3, &mut f, &mut nop, Some(&stop)).unwrap_err(),
            AnalyseError::Cancelled
        );
    }
}
