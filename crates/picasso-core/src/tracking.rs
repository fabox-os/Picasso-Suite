//! Tracking and stabilization data for clips.
//!
//! A clip can carry two related kinds of tracked data:
//!
//! * **Stabilization** (`StabilizationSettings`): the camera path measured by
//!   `picasso-track::stabilize::analyse`, plus what the user asked for (method, smoothness, crop).
//!   The viewer and the Deliver render use [`warp_for_frame`] to smooth that path and warp the
//!   picture so the footage holds steady.
//! * **Tracking data** (`TrackData`): a tracked point (Match Move) with the position it had in
//!   every frame. The picture is held so that the point stays where it was in the first frame.
//!
//! Conventions, shared by everything in this module:
//!
//! * A matrix stored for clip frame `i` maps a point at `p` in the first analysed frame to where
//!   it is in frame `i` (`m * p`). That is the camera path, as `picasso-track` measures it.
//! * Matrices are in **normalized picture coordinates**: the origin is the picture centre and one
//!   unit is the picture *width* on both axes. A picture therefore spans `-0.5..0.5` in x and
//!   `-aspect/2..aspect/2` in y, where `aspect` = height / width.
//! * Frame numbers are clip-relative: frame 0 is the clip's first frame.
//! * `TrackPoint` positions are fractions of the picture (0..1, origin at the top left), which is
//!   what the Edit page's point controls edit.

use serde::{Deserialize, Serialize};

/// A 3x3 matrix, row-major.
pub type Matrix3 = [[f32; 3]; 3];

/// The identity map.
pub const IDENTITY: Matrix3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// How the camera movement measured during stabilization is modeled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StabilizationMethod {
    /// The camera only moved (a 2D translation).
    Translation,
    /// The camera moved, turned and zoomed.
    #[default]
    Similarity,
    /// The camera moved, turned, zoomed and tilted (a full perspective map).
    Perspective,
}

impl StabilizationMethod {
    pub const ALL: [Self; 3] = [Self::Translation, Self::Similarity, Self::Perspective];
    pub fn label(self) -> &'static str {
        match self {
            Self::Translation => "Translation",
            Self::Similarity => "Similarity",
            Self::Perspective => "Perspective",
        }
    }
}

/// What to do when stabilization reveals picture at the edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StabilizationCropMode {
    /// Keep the frame full; revealed edges are transparent.
    NoCrop,
    /// Zoom in just enough that no revealed edge ever shows, plus the extra crop percentage.
    #[default]
    AutoZoom,
    /// Zoom in by the crop percentage, always, without fitting the frame.
    Overscan,
}

impl StabilizationCropMode {
    pub const ALL: [Self; 3] = [Self::NoCrop, Self::AutoZoom, Self::Overscan];
    pub fn label(self) -> &'static str {
        match self {
            Self::NoCrop => "None",
            Self::AutoZoom => "Auto-zoom",
            Self::Overscan => "Overscan",
        }
    }
}

/// One transform measured at one clip frame: `matrix` maps the first analysed frame's picture
/// to this frame's (see the module docs).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FrameTransform {
    /// The clip-relative frame this matrix belongs to.
    pub frame_number: i64,
    pub matrix: Matrix3,
}

impl Default for FrameTransform {
    fn default() -> Self {
        Self {
            frame_number: 0,
            matrix: IDENTITY,
        }
    }
}

/// What is being tracked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerType {
    #[default]
    Point,
    Planar,
    Surface,
    MatchMove,
}

/// One point tracked through a clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TrackPoint {
    pub id: u32,
    /// Position in the first frame, as fractions of the picture (0..1, origin at the top left).
    pub x: f32,
    pub y: f32,
    /// The search box side, pixels.
    pub search_width: u32,
    pub search_height: u32,
    /// The template box side, pixels.
    pub template_width: u32,
    pub template_height: u32,
}

impl Default for TrackPoint {
    fn default() -> Self {
        Self {
            id: 0,
            x: 0.5,
            y: 0.5,
            search_width: 64,
            search_height: 64,
            template_width: 24,
            template_height: 24,
        }
    }
}

/// Most points a track keeps.
const MAX_POINTS: usize = 50;
/// Most frames a track or stabilization keeps: about 5 hours at 30 fps.
const MAX_FRAMES: usize = 500_000;
/// Most frames a stabilization is averaged over on each side.
const MAX_SMOOTHNESS: f32 = 100.0;

/// One tracked point or plane across the frames of a clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TrackData {
    pub track_id: u64,
    pub tracker_type: TrackerType,
    pub points: Vec<TrackPoint>,
    /// One matrix per clip frame (frame `i` is entry `i`), see the module docs.
    pub analyzed_frames: Vec<FrameTransform>,
}

impl Default for TrackData {
    fn default() -> Self {
        Self {
            track_id: 0,
            tracker_type: TrackerType::Point,
            points: Vec::new(),
            analyzed_frames: Vec::new(),
        }
    }
}

/// What the user asked for when stabilizing a clip, plus the camera path the analysis measured.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StabilizationSettings {
    pub enabled: bool,
    pub method: StabilizationMethod,
    /// How many frames each side of the current one the camera path is averaged over. 0 keeps
    /// the camera's own movement (no stabilization); larger values give a steadier picture.
    pub smoothness: f32,
    pub crop_mode: StabilizationCropMode,
    /// Extra crop, in percent, applied on top of the crop mode.
    pub crop_percent: f32,
    /// The measured camera path: one matrix per clip frame, frame `i` is entry `i`.
    pub analyzed_transforms: Vec<FrameTransform>,
}

impl Default for StabilizationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            method: StabilizationMethod::Similarity,
            smoothness: 5.0,
            crop_mode: StabilizationCropMode::AutoZoom,
            crop_percent: 5.0,
            analyzed_transforms: Vec::new(),
        }
    }
}

impl StabilizationSettings {
    /// Whether there is anything to show: the feature is on and the clip was analysed.
    pub fn has_analysis(&self) -> bool {
        self.enabled && !self.analyzed_transforms.is_empty()
    }

    /// The matrix that samples the footage for clip frame `frame`, or `None` when stabilization
    /// is off or nothing was analysed.
    ///
    /// The camera path `P_i` is smoothed into `Q_i` (an average of the path over a window of
    /// `smoothness` frames each side). The picture is then sampled at `P_i * Q_i^-1`, which moves
    /// each frame from where the camera was to where the smoothed camera would have been. Frames
    /// past either end of the analysis reuse the nearest analysed frame.
    pub fn sample_matrix(&self, frame: i64) -> Option<Matrix3> {
        if !self.has_analysis() {
            return None;
        }
        let path = &self.analyzed_transforms;
        let n = path.len();
        let i = frame.clamp(0, n as i64 - 1) as usize;
        let radius = self.smoothness.clamp(0.0, MAX_SMOOTHNESS).round() as usize;
        let (lo, hi) = (i.saturating_sub(radius), (i + radius).min(n - 1));
        let mut sum = [[0.0f64; 3]; 3];
        for t in &path[lo..=hi] {
            let p = normalized(&t.matrix);
            for (r, row) in p.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    sum[r][c] += f64::from(*v);
                }
            }
        }
        let count = (hi - lo + 1) as f64;
        let mut smooth = [[0.0f32; 3]; 3];
        for (r, row) in sum.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                smooth[r][c] = (v / count) as f32;
            }
        }
        let inv = inverse(&smooth)?;
        let current = normalized(&path[i].matrix);
        Some(multiply(&current, &inv))
    }
}

impl TrackData {
    /// The matrix that holds the tracked point still for clip frame `frame`: the point's
    /// position in the first frame is where it is drawn in every frame. `None` when nothing
    /// was tracked.
    pub fn lock_matrix(&self, frame: i64) -> Option<Matrix3> {
        if self.analyzed_frames.is_empty() {
            return None;
        }
        let i = frame.clamp(0, self.analyzed_frames.len() as i64 - 1) as usize;
        Some(self.analyzed_frames[i].matrix)
    }

    /// The same values made safe: counts in range, transforms finite. Frame numbers stay in
    /// order: a broken transform becomes the identity rather than being dropped.
    pub fn sanitized(self) -> Self {
        Self {
            track_id: self.track_id,
            tracker_type: self.tracker_type,
            points: self
                .points
                .into_iter()
                .take(MAX_POINTS)
                .filter(|p| p.x.is_finite() && p.y.is_finite())
                .map(|p| TrackPoint {
                    id: p.id,
                    x: p.x.clamp(0.0, 1.0),
                    y: p.y.clamp(0.0, 1.0),
                    search_width: p.search_width.clamp(4, 256),
                    search_height: p.search_height.clamp(4, 256),
                    template_width: p.template_width.clamp(4, 128),
                    template_height: p.template_height.clamp(4, 128),
                })
                .collect(),
            analyzed_frames: sanitize_frames(self.analyzed_frames),
        }
    }
}

impl StabilizationSettings {
    /// The same values made safe: numbers in range (anything that is not a number becomes the
    /// default), transforms finite.
    pub fn sanitized(self) -> Self {
        let d = Self::default();
        let fix = |v: f32, default: f32, lo: f32, hi: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        Self {
            enabled: self.enabled,
            method: self.method,
            smoothness: fix(self.smoothness, d.smoothness, 0.0, MAX_SMOOTHNESS),
            crop_mode: self.crop_mode,
            crop_percent: fix(self.crop_percent, d.crop_percent, 0.0, 50.0),
            analyzed_transforms: sanitize_frames(self.analyzed_transforms),
        }
    }
}

/// Keeps the frames in order and numbered by position, with non-finite matrices replaced by the
/// identity, and at most `MAX_FRAMES` of them.
fn sanitize_frames(frames: Vec<FrameTransform>) -> Vec<FrameTransform> {
    frames
        .into_iter()
        .take(MAX_FRAMES)
        .enumerate()
        .map(|(i, t)| FrameTransform {
            frame_number: i as i64,
            matrix: if t.matrix.iter().flatten().all(|v| v.is_finite()) {
                t.matrix
            } else {
                IDENTITY
            },
        })
        .collect()
}

/// The picture warp for one clip frame, ready for the renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureWarp {
    /// Where to sample the footage: the output point `p` takes the picture at `matrix * p`.
    pub matrix: Matrix3,
    /// Zoom about the picture centre, applied before `matrix` (1 = none). Larger values crop in.
    pub zoom: f32,
}

/// The warp that stabilization and match move apply to clip frame `frame` (clip-relative), or
/// `None` when neither changes the picture. `aspect` is the picture's height / width.
pub fn warp_for_frame(
    stabilization: &StabilizationSettings,
    tracking: &[TrackData],
    frame: i64,
    aspect: f32,
) -> Option<PictureWarp> {
    let mut matrix = None;
    let mut stabilized = false;
    if let Some(s) = stabilization.sample_matrix(frame) {
        matrix = Some(s);
        stabilized = true;
    }
    if let Some(lock) = tracking
        .iter()
        .find(|t| t.tracker_type == TrackerType::Point)
        .and_then(|t| t.lock_matrix(frame))
    {
        matrix = Some(match matrix {
            Some(m) => multiply(&m, &lock),
            None => lock,
        });
    }
    let matrix = matrix?;
    let zoom = if stabilized {
        fit_zoom(
            &matrix,
            aspect,
            stabilization.crop_mode,
            stabilization.crop_percent,
        )
    } else {
        1.0
    };
    Some(PictureWarp { matrix, zoom })
}

/// The zoom for a stabilized picture. `AutoZoom` is the smallest zoom (about the centre) at which
/// the warped picture still covers the whole frame, times the extra crop percentage. `Overscan`
/// is the crop percentage alone, and `NoCrop` never zooms.
pub fn fit_zoom(
    matrix: &Matrix3,
    aspect: f32,
    mode: StabilizationCropMode,
    crop_percent: f32,
) -> f32 {
    let extra = 1.0 + crop_percent.clamp(0.0, 50.0) / 100.0;
    match mode {
        StabilizationCropMode::NoCrop => 1.0,
        StabilizationCropMode::Overscan => extra,
        StabilizationCropMode::AutoZoom => {
            const MAX_FIT: f32 = 8.0;
            let half_h = aspect / 2.0;
            // Points along the frame's edges (in frame units), where the warped picture would
            // first run out.
            let mut edge = Vec::new();
            for k in 0..=8 {
                let t = k as f32 / 8.0 * 2.0 - 1.0;
                edge.push((0.5 * t, half_h));
                edge.push((0.5 * t, -half_h));
                edge.push((0.5, half_h * t));
                edge.push((-0.5, half_h * t));
            }
            let covers = |z: f32| {
                edge.iter().all(|&(x, y)| {
                    let (sx, sy) = map(matrix, x / z, y / z);
                    // NaN (a map sent to infinity) fails both tests.
                    sx.abs() <= 0.5 + 1e-4 && sy.abs() <= half_h + 1e-4
                })
            };
            let fit = if covers(1.0) {
                1.0
            } else if !covers(MAX_FIT) {
                MAX_FIT
            } else {
                let (mut lo, mut hi) = (1.0f32, MAX_FIT);
                for _ in 0..30 {
                    let mid = 0.5 * (lo + hi);
                    if covers(mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                hi
            };
            fit * extra
        }
    }
}

/// `m` with its projective scale removed (so the bottom right entry is 1 when it is not 0).
fn normalized(m: &Matrix3) -> Matrix3 {
    let w = if m[2][2].abs() > 1e-12 { m[2][2] } else { 1.0 };
    let mut r = [[0.0f32; 3]; 3];
    for (r_row, m_row) in r.iter_mut().zip(m) {
        for (out, v) in r_row.iter_mut().zip(m_row) {
            *out = v / w;
        }
    }
    r
}

fn multiply(a: &Matrix3, b: &Matrix3) -> Matrix3 {
    let mut r = [[0.0f32; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

/// The inverse of a 3x3 matrix, or `None` when it is singular or not finite.
fn inverse(m: &Matrix3) -> Option<Matrix3> {
    let [a, b, c] = m;
    let det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0]);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let k = 1.0 / det;
    Some([
        [
            (b[1] * c[2] - b[2] * c[1]) * k,
            (a[2] * c[1] - a[1] * c[2]) * k,
            (a[1] * b[2] - a[2] * b[1]) * k,
        ],
        [
            (b[2] * c[0] - b[0] * c[2]) * k,
            (a[0] * c[2] - a[2] * c[0]) * k,
            (a[2] * b[0] - a[0] * b[2]) * k,
        ],
        [
            (b[0] * c[1] - b[1] * c[0]) * k,
            (a[1] * c[0] - a[0] * c[1]) * k,
            (a[0] * b[1] - a[1] * b[0]) * k,
        ],
    ])
}

/// Maps a normalized point through a 3x3 matrix; NaN when it is sent to infinity.
fn map(m: &Matrix3, x: f32, y: f32) -> (f32, f32) {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    if w.abs() < 1e-9 {
        return (f32::NAN, f32::NAN);
    }
    (
        (m[0][0] * x + m[0][1] * y + m[0][2]) / w,
        (m[1][0] * x + m[1][1] * y + m[1][2]) / w,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shift(dx: f32, dy: f32) -> Matrix3 {
        [[1.0, 0.0, dx], [0.0, 1.0, dy], [0.0, 0.0, 1.0]]
    }

    fn stab(path: Vec<Matrix3>, smoothness: f32) -> StabilizationSettings {
        StabilizationSettings {
            enabled: true,
            smoothness,
            crop_mode: StabilizationCropMode::NoCrop,
            analyzed_transforms: path
                .into_iter()
                .enumerate()
                .map(|(i, matrix)| FrameTransform {
                    frame_number: i as i64,
                    matrix,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn defaults_are_supported() {
        let s = StabilizationSettings::default();
        assert!(!s.enabled);
        assert!(!s.has_analysis());
        assert_eq!(s.method, StabilizationMethod::Similarity);
        assert_eq!(s.crop_mode, StabilizationCropMode::AutoZoom);
        assert!(s.sample_matrix(0).is_none());

        let t = TrackData::default();
        assert_eq!(t.tracker_type, TrackerType::Point);
        assert!(t.points.is_empty());
        assert!(t.lock_matrix(0).is_none());
        assert_eq!(FrameTransform::default().matrix, IDENTITY);
    }

    #[test]
    fn zero_smoothness_leaves_the_camera_alone() {
        // With nothing smoothed, the camera path is the path itself: no correction.
        let s = stab(vec![IDENTITY, shift(0.1, 0.0), shift(0.2, 0.0)], 0.0);
        let m = s.sample_matrix(2).unwrap();
        assert_close(&m, &IDENTITY);
    }

    #[test]
    fn stabilization_samples_where_the_scene_was() {
        // The camera drifted: at frame 2 the scene is 0.2 to the right of where it was in frame
        // 0. The smoothed camera (frames 1..=2 averaged) is 0.15 along, so the picture must be
        // sampled 0.05 further along (P * Q^-1), which moves the scene back towards the left.
        let path = vec![IDENTITY, shift(0.1, 0.0), shift(0.2, 0.0)];
        let s = stab(path, 1.0);
        let m = s.sample_matrix(2).unwrap();
        let (x, y) = map(&m, 0.0, 0.0);
        assert!((x - 0.05).abs() < 1e-4, "x {x}");
        assert!(y.abs() < 1e-4);
    }

    #[test]
    fn frames_past_the_analysis_hold_the_last_one() {
        let s = stab(vec![IDENTITY, shift(0.1, 0.0)], 1.0);
        assert_eq!(s.sample_matrix(50), s.sample_matrix(1));
        assert_eq!(s.sample_matrix(-3), s.sample_matrix(0));
    }

    #[test]
    fn lock_matrix_holds_the_point_at_its_first_place() {
        let t = TrackData {
            analyzed_frames: vec![
                FrameTransform {
                    frame_number: 0,
                    matrix: IDENTITY,
                },
                FrameTransform {
                    frame_number: 1,
                    matrix: shift(0.3, -0.1),
                },
            ],
            ..Default::default()
        };
        assert_eq!(t.lock_matrix(1), Some(shift(0.3, -0.1)));
        assert_eq!(t.lock_matrix(9), Some(shift(0.3, -0.1)));
    }

    #[test]
    fn auto_zoom_covers_the_frame_and_nothing_else_is_cropped() {
        // A 0.25 shift: the right edge of the frame samples the picture at 0.5 / z + 0.25, which
        // must stay within 0.5, so z must be at least 2.
        let m = shift(0.25, 0.0);
        let z = fit_zoom(&m, 0.5625, StabilizationCropMode::AutoZoom, 0.0);
        assert!((z - 2.0).abs() < 0.02, "zoom {z}");
        // The extra crop multiplies it.
        let z5 = fit_zoom(&m, 0.5625, StabilizationCropMode::AutoZoom, 5.0);
        assert!((z5 - z * 1.05).abs() < 0.02, "zoom {z5}");
        // Without motion nothing is zoomed by auto-zoom...
        assert!(
            (fit_zoom(&IDENTITY, 0.5625, StabilizationCropMode::AutoZoom, 0.0) - 1.0).abs() < 1e-3
        );
        // ...and overscan always zooms by the crop percentage.
        assert!(
            (fit_zoom(&IDENTITY, 0.5625, StabilizationCropMode::Overscan, 10.0) - 1.1).abs() < 1e-5
        );
        assert_eq!(
            fit_zoom(&m, 0.5625, StabilizationCropMode::NoCrop, 10.0),
            1.0
        );
    }

    #[test]
    fn warp_combines_stabilization_and_the_point_lock() {
        let s = StabilizationSettings {
            enabled: false,
            ..stab(vec![IDENTITY], 0.0)
        };
        let t = TrackData {
            analyzed_frames: vec![FrameTransform {
                frame_number: 0,
                matrix: shift(0.1, 0.0),
            }],
            ..Default::default()
        };
        // Stabilization off, point tracked: only the lock applies, with no zoom.
        let w = warp_for_frame(&s, std::slice::from_ref(&t), 0, 0.5625).unwrap();
        assert_close(&w.matrix, &shift(0.1, 0.0));
        assert_eq!(w.zoom, 1.0);
        // Nothing on: no warp at all.
        assert!(warp_for_frame(&s, &[], 0, 0.5625).is_none());
    }

    #[test]
    fn sanitized_keeps_frames_in_order() {
        let s = StabilizationSettings {
            analyzed_transforms: vec![
                FrameTransform {
                    frame_number: 7,
                    matrix: [[f32::NAN; 3]; 3],
                },
                FrameTransform {
                    frame_number: 9,
                    matrix: shift(0.1, 0.0),
                },
            ],
            smoothness: f32::NAN,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(s.analyzed_transforms.len(), 2);
        assert_eq!(s.analyzed_transforms[0].matrix, IDENTITY);
        assert_eq!(s.analyzed_transforms[1].frame_number, 1);
        assert_eq!(s.smoothness, 5.0);
    }

    #[test]
    fn serde_round_trip() {
        let s = StabilizationSettings {
            enabled: true,
            method: StabilizationMethod::Perspective,
            smoothness: 12.5,
            crop_mode: StabilizationCropMode::Overscan,
            crop_percent: 15.0,
            analyzed_transforms: vec![FrameTransform {
                frame_number: 7,
                matrix: IDENTITY,
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: StabilizationSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);

        // Older files without the field load as defaults.
        let older: StabilizationSettings = serde_json::from_str("{}").unwrap();
        assert!(!older.enabled);
        assert!(older.analyzed_transforms.is_empty());
    }

    fn assert_close(a: &Matrix3, b: &Matrix3) {
        for (ra, rb) in a.iter().zip(b) {
            for (x, y) in ra.iter().zip(rb) {
                assert!((x - y).abs() < 1e-4, "{a:?} vs {b:?}");
            }
        }
    }
}
