//! Tracking and stabilization data for clips.
//!
//! A clip can carry two related kinds of tracked data:
//!
//! * **Stabilization** (`StabilizationSettings`): measured camera movement that the viewer and
//!   the Deliver render compensate for, so the footage holds steady on screen. The analysis
//!   itself lives in [`picasso-track`] (`Mode`, `analyse`, `estimate_frame_motion`); this crate
//!   only stores what the analysis produced and what the user asked for.
//! * **Tracking data** (`TrackData`): one or more tracked points or planes, each with the frames
//!   it was measured on and the resulting transform at each frame. Used by Match Move (a clip's
//!   picture follows another clip's movement) and by window tracking on the Color page.

use serde::{Deserialize, Serialize};

use picasso_track::Homography;

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
    pub const ALL: [Self; 3] = [
        Self::Translation,
        Self::Similarity,
        Self::Perspective,
    ];
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
    /// Crop in just enough that no revealed edge ever shows.
    #[default]
    AutoZoom,
    /// The picture is larger than the frame and the stabilized result is taken from its centre.
    Overscan,
}

impl StabilizationCropMode {
    pub const ALL: [Self; 3] = [
        Self::NoCrop,
        Self::AutoZoom,
        Self::Overscan,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::NoCrop => "None",
            Self::AutoZoom => "Auto-zoom",
            Self::Overscan => "Overscan",
        }
    }
}

/// One transform measured at one frame, in normalized picture coordinates (origin at the picture
/// centre, one unit = the picture width): a point at `p` in the first frame of the analyzed range
/// is at `matrix * p` in the frame this row belongs to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FrameTransform {
    /// The timeline frame this matrix belongs to.
    pub frame_number: i64,
    /// The map from the first analyzed frame's picture to this one.
    pub matrix: [[f32; 3]; 3],
}

impl Default for FrameTransform {
    fn default() -> Self {
        Self {
            frame_number: 0,
            matrix: [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
        }
    }
}

/// One kind of thing being tracked.
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
    /// Normalised picture coordinates (origin at the picture centre, one unit = the picture width).
    pub x: f32,
    pub y: f32,
    /// The template box side, pixels.
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

/// One tracked point or plane across the frames of a clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TrackData {
    pub track_id: u64,
    pub tracker_type: TrackerType,
    pub points: Vec<TrackPoint>,
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

/// What the user asked for when stabilizing a clip, plus what the analysis produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StabilizationSettings {
    pub enabled: bool,
    pub method: StabilizationMethod,
    /// Smoothness factor (about 1 to 20).
    pub smoothness: f32,
    pub crop_mode: StabilizationCropMode,
    /// Additional crop percentage applied when `AutoZoom` is on.
    pub crop_percent: f32,
    /// One matrix per analyzed frame, produced by [`picasso-track::analyse`].
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
    /// Whether there is anything to show (the feature is on and some frames were analyzed).
    pub fn has_analysis(&self) -> bool {
        self.enabled && !self.analyzed_transforms.is_empty()
    }

    /// The stabilization matrix for `frame` (timeline frame), or the identity when none was
    /// analyzed for that frame. Matrices are stored in normalized picture coordinates.
    pub fn matrix_at(&self, frame: i64) -> Option<&FrameTransform> {
        self.analyzed_transforms
            .iter()
            .find(|t| t.frame_number == frame)
    }

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
            smoothness: fix(self.smoothness, d.smoothness, 0.0, 100.0),
            crop_mode: self.crop_mode,
            crop_percent: fix(self.crop_percent, d.crop_percent, 0.0, 100.0),
            analyzed_transforms: self
                .analyzed_transforms
                .into_iter()
                .filter(|t| t.matrix.iter().flatten().all(|v| v.is_finite()))
                .collect(),
        }
    }
}

impl TrackData {
    /// The same values made safe: ids and counts in range, transforms finite.
    pub fn sanitized(self) -> Self {
        let max_points = 50usize;
        let max_frames = 1000usize;
        Self {
            track_id: self.track_id,
            tracker_type: self.tracker_type,
            points: self
                .points
                .into_iter()
                .take(max_points)
                .filter(|p| p.x.is_finite() && p.y.is_finite())
                .map(|p| TrackPoint {
                    id: p.id,
                    x: p.x,
                    y: p.y,
                    search_width: p.search_width.clamp(4, 256),
                    search_height: p.search_height.clamp(4, 256),
                    template_width: p.template_width.clamp(4, 128),
                    template_height: p.template_height.clamp(4, 128),
                })
                .collect(),
            analyzed_frames: self
                .analyzed_frames
                .into_iter()
                .take(max_frames)
                .filter(|t| t.matrix.iter().flatten().all(|v| v.is_finite()))
                .collect(),
        }
    }
}

/// Converts a [`Homography`] (f64, from picasso-track) into a `[[f32; 3]; 3]`, clamped so a
/// degenerate map never poisons the render.
#[allow(dead_code)]
pub(crate) fn homography_to_matrix(h: &Homography) -> [[f32; 3]; 3] {
    let m = &h.m;
    let mut out = [
        [1.0f32, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ];
    for (i, cell) in out.iter_mut().flatten().enumerate() {
        let v = m[i] as f32;
        *cell = if v.is_finite() { v.clamp(-1e6, 1e6) } else { 0.0 };
    }
    out
}

/// Converts a `[[f32; 3]; 3]` back into a [`Homography`].
#[allow(dead_code)]
pub(crate) fn matrix_to_homography(matrix: &[[f32; 3]; 3]) -> Homography {
    let mut m = [0.0f64; 9];
    for (i, cell) in matrix.iter().flatten().enumerate() {
        m[i] = f64::from(*cell);
    }
    Homography { m }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_supported() {
        let s = StabilizationSettings::default();
        assert!(!s.enabled);
        assert!(!s.has_analysis());
        assert_eq!(s.method, StabilizationMethod::Similarity);
        assert_eq!(s.crop_mode, StabilizationCropMode::AutoZoom);

        let t = TrackData::default();
        assert_eq!(t.tracker_type, TrackerType::Point);
        assert!(t.points.is_empty());
        assert!(t.analyzed_frames.is_empty());

        let f = FrameTransform::default();
        assert_eq!(f.frame_number, 0);
        assert!(s.matrix_at(0).is_none());
        assert!(s.matrix_at(1).is_none());
    }

    #[test]
    fn matrix_at_finds_the_right_frame() {
        let s = StabilizationSettings {
            enabled: true,
            analyzed_transforms: vec![
                FrameTransform {
                    frame_number: 5,
                    matrix: [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.0]],
                },
                FrameTransform {
                    frame_number: 9,
                    matrix: [[3.0, 0.0, 1.0], [0.0, 3.0, 2.0], [0.0, 0.0, 1.0]],
                },
            ],
            ..Default::default()
        };
        assert!(s.matrix_at(4).is_none());
        assert_eq!(s.matrix_at(5).unwrap().matrix[0][0], 2.0);
        assert_eq!(s.matrix_at(9).unwrap().matrix[0][2], 1.0);
        assert!(s.matrix_at(10).is_none());
    }

    #[test]
    fn homography_conversion_round_trips() {
        let h = Homography {
            m: [1.2, 0.01, 3.0, -0.02, 0.8, -4.0, 0.0, 0.0, 1.0],
        };
        let m = homography_to_matrix(&h);
        let back = matrix_to_homography(&m);
        for (a, b) in h.m.iter().zip(back.m.iter()) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
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
                matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: StabilizationSettings = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.method, StabilizationMethod::Perspective);
        assert!((back.smoothness - 12.5).abs() < 1e-5);
        assert_eq!(back.crop_mode, StabilizationCropMode::Overscan);
        assert!((back.crop_percent - 15.0).abs() < 1e-5);
        assert_eq!(back.analyzed_transforms.len(), 1);
        assert_eq!(back.analyzed_transforms[0].frame_number, 7);

        // Older files without the field load as defaults.
        let older: StabilizationSettings = serde_json::from_str("{}").unwrap();
        assert!(!older.enabled);
        assert!(older.analyzed_transforms.is_empty());
    }
}
