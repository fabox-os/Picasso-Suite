//! The project as pure data, and reading and writing it as JSON.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::EditError;
use crate::time::Fps;
use picasso_color::{ColorGrade, NodeMode};

/// The newest project file version this build writes and reads.
pub const FILE_VERSION: u32 = 1;
/// A project file bigger than this is refused before it is parsed.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Positions and lengths are kept below this, so adding two of them cannot overflow.
pub const MAX_FRAMES: u64 = 1 << 40;
const MAX_MEDIA: usize = 100_000;
const MAX_TRACKS: usize = 256;
const MAX_CLIPS: usize = 1_000_000;
const MAX_SIDE: u32 = 16_384;
const MAX_PIXELS: u64 = 100_000_000;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

id_type!(
    /// Names an item of the media pool.
    MediaId
);
id_type!(
    /// Names a track.
    TrackId
);
id_type!(
    /// Names a clip; stays the same when the clip moves.
    ClipId
);

/// Where the pictures of a media item come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaSource {
    /// One colour.
    Solid { rgb: [u8; 3] },
    /// A left-to-right gradient.
    Gradient { from: [u8; 3], to: [u8; 3] },
    /// Colour bars for checking a monitor.
    ColorBars,
    /// A still image. `path` is where it was imported from; the bytes are not in the project.
    Image { path: String },
    /// A video file (desktop only). Read from `path` when a picture is needed.
    Video { path: String },
    /// A sound file with no picture (desktop only). It goes on audio tracks.
    Audio { path: String },
    /// A compound clip: the clips of timeline `timeline` (one of the project's timelines that
    /// is kept for it), shown and heard as one clip.
    Compound { timeline: u64 },
    /// An effects clip: transparent, with no picture of its own. Its grade and effects apply to
    /// the composite of the tracks below it, like Resolve's Effects Clip.
    EffectsClip,
    /// Text over the picture (a title): transparent where there is no text.
    Title(TitleText),
}

/// The words of a title and how they look. Sizes and places are in thousandths of the picture,
/// so a title looks the same at any resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleText {
    pub text: String,
    /// The height of a line, in thousandths of the picture's height.
    #[serde(default = "title_size")]
    pub size: u32,
    #[serde(default = "title_white")]
    pub color: [u8; 3],
    /// Where the middle of the text is, from the left and from the top.
    #[serde(default = "title_half")]
    pub x: u32,
    #[serde(default = "title_half")]
    pub y: u32,
    #[serde(default)]
    pub bold: bool,
    /// The Text+ look: outline, shadow, background, spacing and alignment.
    #[serde(default, skip_serializing_if = "TextStyle::is_plain")]
    pub style: TextStyle,
}

/// How a title's lines line up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// The Text+ look of a title. Widths and offsets are in thousandths of the picture's height,
/// opacities in percent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    /// The width of the outline (0: none) and its colour.
    pub outline: u32,
    pub outline_color: [u8; 3],
    /// How far down and right the shadow falls (0: none), its colour, opacity and softness.
    pub shadow: u32,
    pub shadow_color: [u8; 3],
    pub shadow_opacity: u32,
    pub shadow_blur: u32,
    /// A box behind the text, its colour, opacity and the room around the text.
    pub background: bool,
    pub background_color: [u8; 3],
    pub background_opacity: u32,
    pub padding: u32,
    /// Extra room between letters, in thousandths of the text's size (may be negative).
    pub tracking: i32,
    /// The distance between lines, in percent of the usual.
    pub line_spacing: u32,
    pub align: TextAlign,
    /// The text's own opacity, in percent.
    pub opacity: u32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            outline: 0,
            outline_color: [0, 0, 0],
            shadow: 0,
            shadow_color: [0, 0, 0],
            shadow_opacity: 75,
            shadow_blur: 0,
            background: false,
            background_color: [0, 0, 0],
            background_opacity: 60,
            padding: 20,
            tracking: 0,
            line_spacing: 100,
            align: TextAlign::Center,
            opacity: 100,
        }
    }
}

impl TextStyle {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// The same style with its numbers in range.
    pub fn sanitized(mut self) -> Self {
        self.outline = self.outline.min(100);
        self.shadow = self.shadow.min(200);
        self.shadow_opacity = self.shadow_opacity.min(100);
        self.shadow_blur = self.shadow_blur.min(100);
        self.background_opacity = self.background_opacity.min(100);
        self.padding = self.padding.min(300);
        self.tracking = self.tracking.clamp(-300, 1000);
        self.line_spacing = self.line_spacing.clamp(50, 400);
        self.opacity = self.opacity.min(100);
        self
    }
}

/// The most characters a title holds.
pub const MAX_TITLE_CHARS: usize = 2000;

fn title_size() -> u32 {
    80
}

fn title_white() -> [u8; 3] {
    [255, 255, 255]
}

fn title_half() -> u32 {
    500
}

impl Default for TitleText {
    fn default() -> Self {
        Self {
            text: "Basic Title".into(),
            size: title_size(),
            color: title_white(),
            x: title_half(),
            y: title_half(),
            bold: false,
            style: TextStyle::default(),
        }
    }
}

impl TitleText {
    /// The same title with its values in range: at most [`MAX_TITLE_CHARS`] characters, a size
    /// of 10 to 500 and a place inside the picture.
    pub fn sanitized(mut self) -> Self {
        if self.text.chars().count() > MAX_TITLE_CHARS {
            self.text = self.text.chars().take(MAX_TITLE_CHARS).collect();
        }
        self.size = self.size.clamp(10, 500);
        self.x = self.x.min(1000);
        self.y = self.y.min(1000);
        self.style = self.style.sanitized();
        self
    }
}

impl MediaSource {
    /// The kind of track this media goes on first: sound files on audio tracks, everything else
    /// on video tracks.
    pub fn track_kind(&self) -> TrackKind {
        match self {
            Self::Audio { .. } => TrackKind::Audio,
            _ => TrackKind::Video,
        }
    }

    /// Whether a clip of this media may sit on a track of `kind`. A video's sound may also have
    /// a clip of its own on an audio track, linked to the picture, so it can be cut apart from
    /// it (a J-cut or an L-cut).
    pub fn fits(&self, kind: TrackKind) -> bool {
        kind == self.track_kind()
            || (kind == TrackKind::Audio && matches!(self, Self::Video { .. }))
    }
}

/// An item of the media pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: MediaId,
    pub name: String,
    pub source: MediaSource,
    /// Length of the source in frames; `None` for a still, which can be held for any time.
    pub length: Option<u64>,
    /// How long a clip of this media is when it is first placed.
    pub default_duration: u64,
    /// The bin it is kept in (None: the Master bin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<u64>,
    /// The start timecode the file carries ("01:00:00:00"), when it has one. Used by
    /// Audio > Auto Align Clips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecode: Option<String>,
}

/// A bin of the media pool: a folder that media can be kept in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bin {
    pub id: u64,
    pub name: String,
}

/// A smart bin: it shows the media whose name contains `contains` (any name when empty) and,
/// when `kind` is set, only media of that type ("Video", "Audio", "Still", "Title"...).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartBin {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub contains: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// The most bins (and smart bins) a project keeps.
pub const MAX_BINS: usize = 500;
/// The longest bin name, in characters.
pub const MAX_BIN_NAME: usize = 120;

/// The type of a media item, as the pool and smart bins name it.
pub fn media_kind(source: &MediaSource) -> &'static str {
    match source {
        MediaSource::Video { .. } => "Video",
        MediaSource::Audio { .. } => "Audio",
        MediaSource::Image { .. } => "Still",
        MediaSource::Title(_) => "Title",
        MediaSource::Compound { .. } => "Compound Clip",
        MediaSource::EffectsClip => "Effects Clip",
        _ => "Generator",
    }
}

impl SmartBin {
    /// Whether `item` belongs in the smart bin.
    pub fn matches(&self, item: &MediaItem) -> bool {
        let needle = self.contains.trim().to_lowercase();
        (needle.is_empty() || item.name.to_lowercase().contains(&needle))
            && self
                .kind
                .as_deref()
                .is_none_or(|k| k == media_kind(&item.source))
    }
}

/// A bin name with control characters taken out, cut to [`MAX_BIN_NAME`].
pub(crate) fn bin_name(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_BIN_NAME)
        .collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        "Bin".to_string()
    } else {
        name
    }
}

/// Only video exists so far; audio tracks come with the audio page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
}

/// The quietest and loudest gain a clip or a track can have, in decibels.
pub const MIN_GAIN_DB: f32 = -60.0;
pub const MAX_GAIN_DB: f32 = 24.0;

/// How a clip sounds: a gain, and fades at its two ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipAudio {
    /// Gain in decibels; 0 leaves the sound as it is.
    pub gain_db: f32,
    /// Frames over which the sound rises from silence at the start of the clip.
    pub fade_in: u64,
    /// Frames over which it falls to silence at the end.
    pub fade_out: u64,
}

impl ClipAudio {
    /// The same values made safe for a clip of `duration` frames: a gain in range (zero if it is
    /// not a number) and fades that fit in the clip together.
    pub fn sanitized(self, duration: u64) -> Self {
        let gain_db = if self.gain_db.is_finite() {
            self.gain_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB)
        } else {
            0.0
        };
        let fade_in = self.fade_in.min(duration);
        let fade_out = self.fade_out.min(duration - fade_in);
        Self {
            gain_db,
            fade_in,
            fade_out,
        }
    }
}

/// The slowest and fastest a clip can play, as a factor of normal speed.
pub const MIN_SPEED: f32 = 0.05;
pub const MAX_SPEED: f32 = 20.0;

fn speed_default() -> f32 {
    1.0
}

/// Where a clip's picture sits in the frame: moved, scaled, turned and cropped.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Offset from the middle of the frame, as a fraction of its width and height (right and
    /// down are positive).
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    /// Degrees, clockwise, around the middle of the picture.
    pub rotation: f32,
    /// The part of each side of the picture that is cut off, as a fraction of the picture.
    pub crop_left: f32,
    pub crop_right: f32,
    pub crop_top: f32,
    pub crop_bottom: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            crop_left: 0.0,
            crop_right: 0.0,
            crop_top: 0.0,
            crop_bottom: 0.0,
        }
    }
}

impl Transform {
    /// True when the picture is left exactly as it is.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// The same values made safe: numbers in range (anything that is not a number becomes the
    /// default), and crops that leave some picture.
    pub fn sanitized(self) -> Self {
        let d = Self::default();
        let fix = |v: f32, default: f32, lo: f32, hi: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        let mut t = Self {
            x: fix(self.x, d.x, -4.0, 4.0),
            y: fix(self.y, d.y, -4.0, 4.0),
            scale_x: fix(self.scale_x, d.scale_x, 0.01, 16.0),
            scale_y: fix(self.scale_y, d.scale_y, 0.01, 16.0),
            rotation: fix(self.rotation, d.rotation, -3600.0, 3600.0),
            crop_left: fix(self.crop_left, 0.0, 0.0, 0.95),
            crop_right: fix(self.crop_right, 0.0, 0.0, 0.95),
            crop_top: fix(self.crop_top, 0.0, 0.0, 0.95),
            crop_bottom: fix(self.crop_bottom, 0.0, 0.0, 0.95),
        };
        // Never crop everything away.
        let width = t.crop_left + t.crop_right;
        if width > 0.98 {
            t.crop_left *= 0.98 / width;
            t.crop_right *= 0.98 / width;
        }
        let height = t.crop_top + t.crop_bottom;
        if height > 0.98 {
            t.crop_top *= 0.98 / height;
            t.crop_bottom *= 0.98 / height;
        }
        t
    }
}

/// How a clip's picture is mixed with what is below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Difference,
}

impl BlendMode {
    pub const ALL: [Self; 8] = [
        Self::Normal,
        Self::Add,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::Darken,
        Self::Lighten,
        Self::Difference,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Add => "Add",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Overlay => "Overlay",
            Self::Darken => "Darken",
            Self::Lighten => "Lighten",
            Self::Difference => "Difference",
        }
    }
}

/// A piece of media placed on a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub media: MediaId,
    pub name: String,
    /// First frame on the timeline.
    pub start: u64,
    /// First frame of the media that is shown.
    pub source_in: u64,
    /// Number of frames; never zero.
    pub duration: u64,
    /// 0 is invisible, 1 covers what is below.
    pub opacity: f32,
    pub grade: ColorGrade,
    /// A disabled clip stays on the timeline but is not shown.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub audio: ClipAudio,
    /// How many frames of the media pass in one frame of the timeline (1 is normal speed).
    #[serde(default = "speed_default")]
    pub speed: f32,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub blend: BlendMode,
    /// Clips that share a link move, trim, split and are deleted together, while Linked
    /// Selection is on: a video and its sound.
    #[serde(default)]
    pub link: Option<u64>,
    /// The serial nodes after the first one (`grade`), applied in order: node 02, 03 and on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<ColorGrade>,
    /// Whether the clip's sound is heard. A video clip whose sound has its own linked clip on an
    /// audio track is silent, so the sound is not heard twice.
    #[serde(default = "enabled_by_default")]
    pub sound: bool,
    /// The clip plays its footage backwards: its first frame shows the last frame it uses.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reverse: bool,
    /// The clip holds one frame of its footage (`source_in`) for its whole length.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub freeze: bool,
    /// A transition from the clip that ends where this one starts, centred on the cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// A locked clip cannot be moved, trimmed, cut or deleted (it can still be graded).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// The clip's colour on the timeline, one of [`CLIP_COLORS`] (None is the track's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
    /// Animated properties: opacity, position, zoom and rotation over the clip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<crate::keyframe::KeyTrack>,
    /// Video effects, applied in order after the grade.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<picasso_color::Effect>,
}

/// The colours a clip can be given (Mark > Set Clip Color), as names and sRGB.
pub const CLIP_COLORS: [(&str, [u8; 3]); 16] = [
    ("Orange", [235, 110, 1]),
    ("Apricot", [255, 168, 51]),
    ("Yellow", [226, 169, 2]),
    ("Lime", [159, 198, 22]),
    ("Olive", [94, 153, 32]),
    ("Green", [68, 143, 101]),
    ("Teal", [0, 152, 153]),
    ("Navy", [21, 98, 132]),
    ("Blue", [67, 117, 196]),
    ("Purple", [144, 99, 203]),
    ("Violet", [209, 82, 152]),
    ("Pink", [233, 140, 181]),
    ("Tan", [185, 175, 151]),
    ("Beige", [198, 160, 119]),
    ("Brown", [153, 102, 0]),
    ("Chocolate", [140, 90, 63]),
];

/// How one clip turns into the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionKind {
    /// The pictures mix (and the sounds cross-fade).
    #[default]
    CrossDissolve,
    /// The first picture fades to black, then the second fades in.
    DipToBlack,
}

/// A transition at the start of a clip: `frames` long, half before the cut and half after.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    #[serde(default)]
    pub kind: TransitionKind,
    pub frames: u64,
}

/// The longest transition, in frames.
pub const MAX_TRANSITION: u64 = 100_000;

/// A track of a timeline read from another program, for [`crate::Command::ImportTimeline`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedTrack {
    pub kind: TrackKind,
    pub muted: bool,
    pub clips: Vec<ImportedClip>,
}

/// A clip of an imported timeline: media of the pool, where it sits and what of it it shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedClip {
    pub media: MediaId,
    pub name: String,
    /// First frame on the timeline.
    pub start: u64,
    /// First frame of the media that is shown.
    pub source_in: u64,
    /// Frames on the timeline.
    pub duration: u64,
    pub speed: f32,
    pub reverse: bool,
    pub freeze: bool,
    pub enabled: bool,
    /// A transition from the clip before, centred on the cut.
    pub transition: Option<Transition>,
}

/// How long the transition between `left` and the clip after it, `right`, plays: None when
/// there is none, or the clips do not touch. It is never longer than twice either clip, since
/// each gives at most its whole length to it.
pub fn transition_frames(left: &Clip, right: &Clip) -> Option<u64> {
    let t = right.transition?;
    if left.end() != right.start {
        return None;
    }
    let frames = t
        .frames
        .min(left.duration.saturating_mul(2))
        .min(right.duration.saturating_mul(2));
    (frames >= 2).then_some(frames)
}

/// The transition at frame `at` of `track`, if one is playing there: the clip before the cut,
/// the clip after it, and how far through the transition `at` is (0 to 1).
pub fn transition_at(track: &Track, at: u64) -> Option<(&Clip, &Clip, TransitionKind, f32)> {
    track.clips.windows(2).find_map(|pair| {
        let (left, right) = (&pair[0], &pair[1]);
        let t = right.transition?;
        let frames = transition_frames(left, right)?;
        let first = right.start.checked_sub(frames / 2)?;
        if at < first || at >= first + frames {
            return None;
        }
        let progress = ((at - first) as f32 + 0.5) / frames as f32;
        Some((left, right, t.kind, progress))
    })
}

fn enabled_by_default() -> bool {
    true
}

/// A marker on the timeline: a frame, a name and one of eight colours.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub frame: u64,
    pub name: String,
    /// 0 to 7.
    pub color: u8,
}

/// The most markers a project may have.
pub const MAX_MARKERS: usize = 10_000;
/// The number of marker colours.
pub const MARKER_COLORS: u8 = 8;

impl Clip {
    /// How many frames of the media `frames` frames of the timeline use at this clip's speed.
    pub fn source_frames(&self, frames: u64) -> u64 {
        if self.freeze {
            return 0;
        }
        (frames as f64 * f64::from(self.speed)).round() as u64
    }

    /// How many frames of the media the whole clip uses.
    pub fn source_span(&self) -> u64 {
        if self.freeze {
            return 1;
        }
        (self.duration as f64 * f64::from(self.speed)).ceil() as u64
    }

    /// The frame of the media shown at timeline frame `frame`.
    pub fn source_frame_at(&self, frame: u64) -> u64 {
        let along = self.source_frames(frame.saturating_sub(self.start));
        if self.reverse {
            // Counted back from the last frame the clip uses.
            let last = self.source_in + self.source_span().saturating_sub(1);
            return last.saturating_sub(along).max(self.source_in);
        }
        self.source_in + along
    }

    /// The first frame after the clip.
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.duration)
    }

    pub fn contains(&self, frame: u64) -> bool {
        frame >= self.start && frame < self.end()
    }

    /// How many colour nodes the clip has: the first (`grade`) and the serial ones after it.
    pub fn node_count(&self) -> usize {
        1 + self.nodes.len()
    }

    /// Node `n` (0 is the first, `grade`).
    pub fn node(&self, n: usize) -> Option<&ColorGrade> {
        match n {
            0 => Some(&self.grade),
            n => self.nodes.get(n - 1),
        }
    }

    pub fn node_mut(&mut self, n: usize) -> Option<&mut ColorGrade> {
        match n {
            0 => Some(&mut self.grade),
            n => self.nodes.get_mut(n - 1),
        }
    }

    /// Whether any node changes the picture.
    pub fn is_graded(&self) -> bool {
        self.grade.has_effect() || self.nodes.iter().any(ColorGrade::has_effect)
    }

    /// Grades a frame of the clip's picture through every node, in order.
    pub fn apply_nodes(&self, rgba: &mut [u8], width: usize, height: usize) {
        let all: Vec<&ColorGrade> = std::iter::once(&self.grade).chain(&self.nodes).collect();
        // The picture the last serial or layer node was given, for the parallel nodes after it.
        let mut branch: Option<Vec<u8>> = None;
        for (i, node) in all.iter().enumerate() {
            let active = node.has_effect() || (node.highlight && node.enabled);
            match node.mode {
                NodeMode::Parallel if i > 0 => {
                    if !active {
                        continue;
                    }
                    let input = branch.clone().unwrap_or_else(|| rgba.to_vec());
                    let mut out = input.clone();
                    node.apply_frame(&mut out, width, height);
                    let k = node.mix / 100.0;
                    for ((px, a), b) in rgba
                        .chunks_exact_mut(4)
                        .zip(input.chunks_exact(4))
                        .zip(out.chunks_exact(4))
                    {
                        for c in 0..3 {
                            let d = (f32::from(b[c]) - f32::from(a[c])) * k;
                            px[c] = (f32::from(px[c]) + d).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
                mode => {
                    if all.get(i + 1).is_some_and(|n| n.mode == NodeMode::Parallel) {
                        branch = Some(rgba.to_vec());
                    }
                    if !active {
                        continue;
                    }
                    if mode == NodeMode::Layer && i > 0 && node.mix < 100.0 {
                        let before = rgba.to_vec();
                        node.apply_frame(rgba, width, height);
                        let k = node.mix / 100.0;
                        for (px, old) in rgba.chunks_exact_mut(4).zip(before.chunks_exact(4)) {
                            for c in 0..3 {
                                let a = f32::from(old[c]);
                                px[c] = (a + (f32::from(px[c]) - a) * k + 0.5) as u8;
                            }
                        }
                    } else {
                        node.apply_frame(rgba, width, height);
                    }
                }
            }
        }
    }
}

/// A track's picture is on unless the project was saved before the switch existed.
fn picture_on() -> bool {
    true
}

/// A row of clips, sorted by start and never overlapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
    pub muted: bool,
    pub locked: bool,
    /// When any track is soloed, only the soloed tracks are heard.
    #[serde(default)]
    pub solo: bool,
    /// Whether the track's picture is drawn. Off keeps it out of the program.
    #[serde(default = "picture_on")]
    pub picture: bool,
    /// What the track adds to the mix of its clips, in decibels.
    #[serde(default)]
    pub gain_db: f32,
    /// -1 is all left, 0 the middle, 1 all right.
    #[serde(default)]
    pub pan: f32,
    /// The track's EQ and dynamics.
    #[serde(default, skip_serializing_if = "crate::audio_fx::TrackFx::is_plain")]
    pub fx: crate::audio_fx::TrackFx,
}

/// The whole project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub fps: Fps,
    pub width: u32,
    pub height: u32,
    pub media: Vec<MediaItem>,
    /// Bottom track first: `tracks[0]` is V1 and later tracks are drawn over it.
    pub tracks: Vec<Track>,
    /// Sorted by frame, at most one per frame.
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// The start of the range that playback and export use, when set.
    #[serde(default)]
    pub mark_in: Option<u64>,
    /// The end of that range (the first frame after it), when set.
    #[serde(default)]
    pub mark_out: Option<u64>,
    /// Whether editing a linked clip also edits the clips linked to it (Timeline › Timeline
    /// Settings › Linked Selection).
    #[serde(default = "enabled_by_default")]
    pub linked_selection: bool,
    /// The name of the timeline being edited (the one in `tracks`, `markers` and the marks).
    #[serde(default = "first_timeline_name")]
    pub timeline_name: String,
    /// Its id; 0 for the first timeline of a project.
    #[serde(default)]
    pub timeline_id: u64,
    /// The bins of the media pool, besides Master, and the smart bins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bins: Vec<Bin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_bins: Vec<SmartBin>,
    /// The multicam clips.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub multicams: Vec<crate::Multicam>,
    /// The gallery: stills grabbed on the Color page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stills: Vec<Still>,
    /// The project's other timelines, kept as they were left.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timelines: Vec<Timeline>,
    next_id: u64,
}

/// A still of the gallery: a small picture of a frame and the grade it had, which can be
/// applied to another clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Still {
    pub id: u64,
    pub name: String,
    /// The timeline frame it was grabbed at.
    pub frame: u64,
    /// The clip's nodes: the first, then the serial ones.
    pub grade: ColorGrade,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<ColorGrade>,
    /// The picture, a PNG in base64.
    pub png: String,
}

/// The most stills a gallery holds, and the largest picture of one (base64 characters).
pub const MAX_STILLS: usize = 200;
pub const MAX_STILL_PNG: usize = 2 << 20;

/// A timeline that is not being edited: its tracks, markers and marks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub id: u64,
    pub name: String,
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub mark_in: Option<u64>,
    #[serde(default)]
    pub mark_out: Option<u64>,
    /// The timeline holds the clips of a compound clip; it is not listed with the others.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compound: bool,
}

/// The most timelines one project may have.
pub const MAX_TIMELINES: usize = 64;

fn first_timeline_name() -> String {
    "Timeline 1".into()
}

impl Project {
    /// An empty 1920 x 1080, 24 fps project with two video tracks.
    pub fn new(name: impl Into<String>) -> Self {
        let mut p = Self {
            version: FILE_VERSION,
            name: name.into(),
            fps: Fps::FPS_24,
            width: 1920,
            height: 1080,
            media: Vec::new(),
            tracks: Vec::new(),
            markers: Vec::new(),
            mark_in: None,
            mark_out: None,
            linked_selection: true,
            timeline_name: first_timeline_name(),
            timeline_id: 0,
            stills: Vec::new(),
            bins: Vec::new(),
            smart_bins: Vec::new(),
            multicams: Vec::new(),
            timelines: Vec::new(),
            next_id: 1,
        };
        for n in 1..=2 {
            let id = TrackId(p.fresh_id());
            p.tracks.push(Track {
                id,
                name: format!("V{n}"),
                kind: TrackKind::Video,
                clips: Vec::new(),
                muted: false,
                locked: false,
                solo: false,
                picture: true,
                gain_db: 0.0,
                pan: 0.0,
                fx: Default::default(),
            });
        }
        p
    }

    /// An empty project as Resolve starts one: a picture track, V1, and a sound track, A1.
    pub fn start(name: impl Into<String>) -> Self {
        let mut p = Self::new(name);
        p.tracks.truncate(1);
        let id = TrackId(p.fresh_id());
        p.tracks.push(Track {
            id,
            name: "A1".into(),
            kind: TrackKind::Audio,
            clips: Vec::new(),
            muted: false,
            locked: false,
            solo: false,
            picture: true,
            gain_db: 0.0,
            pan: 0.0,
            fx: Default::default(),
        });
        p
    }

    /// A small project to start with: three generated media items and a few clips.
    pub fn demo() -> Self {
        let mut p = Self::new("Demo Project");
        let fps = p.fps.nominal();
        let sources = [
            ("Color Bars", MediaSource::ColorBars),
            (
                "Sunset",
                MediaSource::Gradient {
                    from: [250, 120, 40],
                    to: [60, 20, 110],
                },
            ),
            (
                "Teal",
                MediaSource::Solid {
                    rgb: [20, 140, 150],
                },
            ),
        ];
        let mut ids = Vec::new();
        for (name, source) in sources {
            let id = MediaId(p.fresh_id());
            p.media.push(MediaItem {
                id,
                name: name.into(),
                source,
                length: None,
                default_duration: fps * 4,
                bin: None,
                timecode: None,
            });
            ids.push(id);
        }
        let v1 = p.tracks[0].id;
        let mut at = 0;
        for id in &ids {
            let clip_id = ClipId(p.fresh_id());
            let name = p.media(*id).map(|m| m.name.clone()).unwrap_or_default();
            if let Some(track) = p.track_mut(v1) {
                track.clips.push(Clip {
                    id: clip_id,
                    media: *id,
                    name,
                    start: at,
                    source_in: 0,
                    duration: fps * 4,
                    opacity: 1.0,
                    grade: ColorGrade::default(),
                    enabled: true,
                    audio: ClipAudio::default(),
                    speed: 1.0,
                    transform: Transform::default(),
                    blend: BlendMode::Normal,
                    link: None,
                    sound: true,
                    nodes: Vec::new(),
                    reverse: false,
                    freeze: false,
                    transition: None,
                    locked: false,
                    color: None,
                    keyframes: Vec::new(),
                    effects: Vec::new(),
                });
            }
            at += fps * 4;
        }
        p
    }

    /// Swaps the timeline being edited with `other`.
    pub(crate) fn swap_timeline(&mut self, other: &mut Timeline) {
        std::mem::swap(&mut self.timeline_id, &mut other.id);
        std::mem::swap(&mut self.timeline_name, &mut other.name);
        std::mem::swap(&mut self.tracks, &mut other.tracks);
        std::mem::swap(&mut self.markers, &mut other.markers);
        std::mem::swap(&mut self.mark_in, &mut other.mark_in);
        std::mem::swap(&mut self.mark_out, &mut other.mark_out);
    }

    /// Every timeline as (id, name), in the order they were made; the one being edited is
    /// among them.
    pub fn timeline_list(&self) -> Vec<(u64, String)> {
        let mut list: Vec<(u64, String)> = self
            .timelines
            .iter()
            .filter(|t| !t.compound)
            .map(|t| (t.id, t.name.clone()))
            .chain(std::iter::once((
                self.timeline_id,
                self.timeline_name.clone(),
            )))
            .collect();
        list.sort_by_key(|(id, _)| *id);
        list
    }

    /// The tracks of timeline `id`, the edited one or a stored one.
    pub fn timeline_tracks(&self, id: u64) -> Option<&[Track]> {
        if id == self.timeline_id {
            return Some(&self.tracks);
        }
        self.timelines
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.tracks.as_slice())
    }

    /// Whether some timeline, edited or not, uses media `media`.
    pub fn uses_media(&self, media: MediaId) -> bool {
        self.tracks
            .iter()
            .chain(self.timelines.iter().flat_map(|t| t.tracks.iter()))
            .flat_map(|t| t.clips.iter())
            .any(|c| c.media == media)
    }

    pub(crate) fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn media(&self, id: MediaId) -> Option<&MediaItem> {
        self.media.iter().find(|m| m.id == id)
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Whether any track is soloed: only the soloed tracks are heard then.
    pub fn any_soloed(&self) -> bool {
        self.tracks.iter().any(|t| t.solo)
    }

    pub(crate) fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// The track and the clip with this id.
    pub fn clip(&self, id: ClipId) -> Option<(&Track, &Clip)> {
        self.tracks
            .iter()
            .find_map(|t| t.clips.iter().find(|c| c.id == id).map(|c| (t, c)))
    }

    /// Index of the track and of the clip in it.
    /// The clip `id`, to change it (outside commands, only on a copy of the project).
    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        let (t, c) = self.locate(id)?;
        self.tracks.get_mut(t)?.clips.get_mut(c)
    }

    pub(crate) fn locate(&self, id: ClipId) -> Option<(usize, usize)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(ti, t)| t.clips.iter().position(|c| c.id == id).map(|ci| (ti, ci)))
    }

    /// The other clips linked to clip `id`. Empty when Linked Selection is off, so edits then
    /// touch only the clip itself.
    pub fn partners(&self, id: ClipId) -> Vec<ClipId> {
        if !self.linked_selection {
            return Vec::new();
        }
        self.linked_to(id)
    }

    /// The other clips linked to clip `id`, whatever Linked Selection says.
    pub fn linked_to(&self, id: ClipId) -> Vec<ClipId> {
        let Some(link) = self.clip(id).and_then(|(_, c)| c.link) else {
            return Vec::new();
        };
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.link == Some(link) && c.id != id)
            .map(|c| c.id)
            .collect()
    }

    /// The media that clips on the timeline and on every other timeline use, without repeats and
    /// in pool order. Media that no clip uses is left out, and so is a clip whose media is gone.
    pub fn media_used(&self) -> Vec<MediaId> {
        let mut used: Vec<MediaId> = self
            .tracks
            .iter()
            .chain(self.timelines.iter().flat_map(|t| t.tracks.iter()))
            .flat_map(|t| t.clips.iter())
            .map(|c| c.media)
            .collect();
        used.sort_by_key(|m| m.0);
        used.dedup();
        used.retain(|m| self.media(*m).is_some());
        used
    }

    /// The first frame after the last clip.
    pub fn duration(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(Clip::end)
            .max()
            .unwrap_or(0)
    }

    /// The clip of `track` that shows `frame`.
    pub fn clip_at(track: &Track, frame: u64) -> Option<&Clip> {
        track.clips.iter().find(|c| c.contains(frame))
    }

    /// Whether `[start, start + duration)` is free on the track, not counting the clip `ignore`.
    pub fn can_place(
        &self,
        track: TrackId,
        start: u64,
        duration: u64,
        ignore: Option<ClipId>,
    ) -> bool {
        let Some(track) = self.track(track) else {
            return false;
        };
        let Some(end) = start.checked_add(duration) else {
            return false;
        };
        duration > 0
            && end <= MAX_FRAMES
            && !track
                .clips
                .iter()
                .any(|c| Some(c.id) != ignore && c.start < end && start < c.end())
    }

    /// Writes the project as pretty JSON.
    pub fn to_json(&self) -> Result<String, EditError> {
        serde_json::to_string_pretty(self).map_err(|e| EditError::BadFile(e.to_string()))
    }

    /// Reads and checks a project. Numbers that cannot be right are repaired where that is safe
    /// (opacity, grades) and refused where it is not (overlapping clips, missing media).
    pub fn from_json(text: &str) -> Result<Self, EditError> {
        if text.len() > MAX_FILE_BYTES {
            return Err(EditError::BadFile("the file is too big".into()));
        }
        tracing::debug!(bytes = text.len(), "reading a project");
        let mut p: Project =
            serde_json::from_str(text).map_err(|e| EditError::BadFile(e.to_string()))?;
        if p.version == 0 || p.version > FILE_VERSION {
            return Err(EditError::BadFile(format!(
                "version {} is not supported",
                p.version
            )));
        }
        for clip in p.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
            clip.opacity = if clip.opacity.is_finite() {
                clip.opacity.clamp(0.0, 1.0)
            } else {
                1.0
            };
            clip.grade = std::mem::take(&mut clip.grade).sanitized();
            // A file may claim any number of nodes; keep the first ones a clip may have.
            clip.nodes.truncate(crate::command::MAX_NODES - 1);
            for node in &mut clip.nodes {
                *node = std::mem::take(node).sanitized();
            }
        }
        p.validate()?;
        Ok(p)
    }

    /// Checks everything a command relies on and makes `next_id` safe to use.
    /// Checks the timeline being edited (see [`Project::validate`]), noting its ids.
    fn check_timeline(
        &mut self,
        ids: &mut HashSet<u64>,
        max_id: &mut u64,
    ) -> Result<(), EditError> {
        let bad = |why: &str| Err(EditError::BadFile(why.to_string()));
        let mut clip_count = 0usize;
        for t in &self.tracks {
            if !ids.insert(t.id.0) {
                return bad("two items share an id");
            }
            *max_id = (*max_id).max(t.id.0);
            clip_count = clip_count.saturating_add(t.clips.len());
            if clip_count > MAX_CLIPS {
                return bad("too many clips");
            }
            if !t.gain_db.is_finite() || !t.pan.is_finite() {
                return bad("a track mix value is not a number");
            }
            let mut previous_end = 0;
            for c in &t.clips {
                if !ids.insert(c.id.0) {
                    return bad("two items share an id");
                }
                *max_id = (*max_id).max(c.id.0);
                if self.media(c.media).is_none() {
                    return bad("a clip uses media that is not in the pool");
                }
                let Some(end) = c.start.checked_add(c.duration) else {
                    return bad("a clip is too long");
                };
                if c.duration == 0 || end > MAX_FRAMES || c.source_in > MAX_FRAMES {
                    return bad("a clip has no length or is too long");
                }
                if c.start < previous_end {
                    return bad("clips overlap or are not in order");
                }
                previous_end = end;
            }
        }
        // Gains and fades that are out of range are brought back into it.
        for t in &mut self.tracks {
            t.gain_db = t.gain_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
            t.pan = t.pan.clamp(-1.0, 1.0);
            t.fx = t.fx.sanitized();
            for c in &mut t.clips {
                c.audio = c.audio.sanitized(c.duration);
                c.keyframes = std::mem::take(&mut c.keyframes)
                    .into_iter()
                    .map(crate::KeyTrack::sanitized)
                    .filter(|k| !k.keys.is_empty())
                    .collect();
                c.keyframes.sort_by_key(|k| k.param);
                c.effects.truncate(picasso_color::MAX_EFFECTS);
                for e in &mut c.effects {
                    *e = e.sanitized();
                }
                c.keyframes.dedup_by_key(|k| k.param);
                c.speed = if c.speed.is_finite() {
                    c.speed.clamp(MIN_SPEED, MAX_SPEED)
                } else {
                    1.0
                };
                c.transform = c.transform.sanitized();
            }
        }
        if self.markers.len() > MAX_MARKERS {
            return bad("too many markers");
        }
        let mut last: Option<u64> = None;
        for m in &self.markers {
            if m.frame > MAX_FRAMES
                || m.color >= MARKER_COLORS
                || last.is_some_and(|l| l >= m.frame)
            {
                return bad("the markers are not valid");
            }
            last = Some(m.frame);
        }
        for mark in [self.mark_in, self.mark_out].into_iter().flatten() {
            if mark > MAX_FRAMES {
                return bad("a mark is too far");
            }
        }
        if let (Some(a), Some(b)) = (self.mark_in, self.mark_out) {
            if a >= b {
                return bad("the in mark is not before the out mark");
            }
        }
        Ok(())
    }

    pub fn validate(&mut self) -> Result<(), EditError> {
        let bad = |why: &str| Err(EditError::BadFile(why.to_string()));
        if !self.fps.is_valid() {
            return bad("the frame rate is not valid");
        }
        let pixels = u64::from(self.width) * u64::from(self.height);
        if self.width == 0
            || self.height == 0
            || self.width > MAX_SIDE
            || self.height > MAX_SIDE
            || pixels > MAX_PIXELS
        {
            return bad("the picture size is not valid");
        }
        if self.media.len() > MAX_MEDIA || self.tracks.len() > MAX_TRACKS {
            return bad("too many media items or tracks");
        }
        let mut ids = HashSet::new();
        let mut max_id = 0;
        for m in &self.media {
            if !ids.insert(m.id.0) || m.default_duration == 0 || m.default_duration > MAX_FRAMES {
                return bad("a media item is not valid");
            }
            if m.length.is_some_and(|l| l == 0 || l > MAX_FRAMES) {
                return bad("a media length is not valid");
            }
            max_id = max_id.max(m.id.0);
        }
        if self.timelines.len() >= MAX_TIMELINES {
            return bad("too many timelines");
        }
        let mut timeline_ids = HashSet::from([self.timeline_id]);
        if self.timelines.iter().any(|t| !timeline_ids.insert(t.id)) {
            return bad("two timelines share an id");
        }
        for id in &timeline_ids {
            max_id = max_id.max(*id);
        }
        let mut stored = std::mem::take(&mut self.timelines);
        let mut checked = self.check_timeline(&mut ids, &mut max_id);
        for t in &mut stored {
            if checked.is_err() {
                break;
            }
            self.swap_timeline(t);
            checked = self.check_timeline(&mut ids, &mut max_id);
            self.swap_timeline(t);
        }
        self.timelines = stored;
        checked?;
        if self.stills.len() > MAX_STILLS || self.stills.iter().any(|s| s.png.len() > MAX_STILL_PNG)
        {
            return bad("the gallery is too big");
        }
        if self.bins.len() > MAX_BINS || self.smart_bins.len() > MAX_BINS {
            return bad("too many bins");
        }
        for b in &mut self.bins {
            max_id = max_id.max(b.id);
            b.name = bin_name(&b.name);
        }
        for b in &mut self.smart_bins {
            max_id = max_id.max(b.id);
            b.name = bin_name(&b.name);
            b.contains = b.contains.chars().take(MAX_BIN_NAME).collect();
        }
        if self.multicams.len() > crate::multicam::MAX_MULTICAMS {
            return bad("too many multicam clips");
        }
        let media_ids: Vec<MediaId> = self.media.iter().map(|m| m.id).collect();
        for m in &mut self.multicams {
            max_id = max_id.max(m.id);
            m.name = crate::Multicam::clean_name(&m.name);
            m.angles.retain(|a| media_ids.contains(&a.media));
            m.angles.truncate(crate::multicam::MAX_ANGLES);
            for a in &mut m.angles {
                a.offset = a.offset.clamp(
                    -crate::multicam::MAX_ANGLE_OFFSET,
                    crate::multicam::MAX_ANGLE_OFFSET,
                );
            }
        }
        self.multicams.retain(|m| m.angles.len() >= 2);
        let bin_ids: Vec<u64> = self.bins.iter().map(|b| b.id).collect();
        for m in &mut self.media {
            if m.bin.is_some_and(|b| !bin_ids.contains(&b)) {
                m.bin = None;
            }
        }
        for s in &mut self.stills {
            max_id = max_id.max(s.id);
            s.grade = s.grade.clone().sanitized();
            s.nodes.truncate(crate::command::MAX_NODES - 1);
            for n in &mut s.nodes {
                *n = n.clone().sanitized();
            }
        }
        for m in &mut self.media {
            if let MediaSource::Title(t) = &mut m.source {
                *t = t.clone().sanitized();
            }
        }
        // A compound clip's media names a timeline kept for it.
        for m in &self.media {
            if let MediaSource::Compound { timeline } = m.source {
                if !self
                    .timelines
                    .iter()
                    .any(|t| t.id == timeline && t.compound)
                {
                    return bad("a compound clip has no timeline");
                }
            }
        }
        self.next_id = self.next_id.max(max_id.saturating_add(1));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_valid_and_round_trips() {
        let p = Project::demo();
        assert!(p.clone().validate().is_ok());
        assert_eq!(p.duration(), 24 * 12);
        let text = p.to_json().unwrap();
        assert_eq!(Project::from_json(&text).unwrap(), p);
    }

    #[test]
    fn media_used_is_what_clips_use_on_every_timeline() {
        let mut p = Project::demo();
        let all: Vec<MediaId> = p.media.iter().map(|m| m.id).collect();
        assert_eq!(p.media_used(), all, "the demo uses all three");
        // Media that no clip uses is left out.
        p.media.push(MediaItem {
            id: MediaId(9999),
            name: "unused".into(),
            source: MediaSource::Solid { rgb: [0, 0, 0] },
            length: None,
            default_duration: 24,
            bin: None,
            timecode: None,
        });
        assert_eq!(p.media_used(), all, "nothing uses the fourth");
        // A clip on another timeline counts too.
        let first = p.tracks.remove(0);
        let id = p.fresh_id();
        p.timelines.push(Timeline {
            id,
            name: "T2".into(),
            tracks: vec![first],
            markers: Vec::new(),
            mark_in: None,
            mark_out: None,
            compound: false,
        });
        assert_eq!(p.media_used(), all, "the other timeline's clips count");
    }

    #[test]
    fn fresh_ids_are_unique_after_loading() {
        let mut loaded = Project::from_json(&Project::demo().to_json().unwrap()).unwrap();
        let used: HashSet<u64> = loaded.media.iter().map(|m| m.id.0).collect();
        assert!(!used.contains(&loaded.fresh_id()));
    }

    #[test]
    fn can_place_respects_neighbours_and_ignore() {
        let p = Project::demo();
        let v1 = p.tracks[0].id;
        let first = p.tracks[0].clips[0].id;
        assert!(!p.can_place(v1, 10, 5, None));
        assert!(p.can_place(v1, 10, 5, Some(first)));
        assert!(p.can_place(v1, 24 * 12, 10, None));
        assert!(!p.can_place(v1, 0, 0, None));
        assert!(!p.can_place(v1, u64::MAX, 2, None));
        assert!(!p.can_place(TrackId(9999), 0, 1, None));
    }

    fn broken(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Project, EditError> {
        let mut v: serde_json::Value =
            serde_json::from_str(&Project::demo().to_json().unwrap()).unwrap();
        edit(&mut v);
        Project::from_json(&v.to_string())
    }

    #[test]
    fn malformed_files_are_refused_not_panicked_on() {
        assert!(Project::from_json("").is_err());
        assert!(Project::from_json("{").is_err());
        assert!(Project::from_json("[]").is_err());
        assert!(broken(|v| v["version"] = 99.into()).is_err());
        assert!(broken(|v| v["version"] = 0.into()).is_err());
        assert!(broken(|v| v["fps"]["den"] = 0.into()).is_err());
        assert!(broken(|v| v["width"] = 0.into()).is_err());
        assert!(broken(|v| {
            v["width"] = 16_384.into();
            v["height"] = 16_384.into();
        })
        .is_err()); // inside the side limit, over the pixel limit
        assert!(broken(|v| v["width"] = 16_385.into()).is_err());
        assert!(broken(|v| v["tracks"][0]["clips"][0]["duration"] = 0.into()).is_err());
        assert!(broken(|v| v["tracks"][0]["clips"][1]["start"] = 1.into()).is_err()); // overlap
        assert!(broken(|v| v["tracks"][0]["clips"][0]["media"] = 4242.into()).is_err());
        assert!(broken(
            |v| v["tracks"][0]["clips"][1]["id"] = v["tracks"][0]["clips"][0]["id"].clone()
        )
        .is_err());
        // Overflow of start + duration.
        assert!(broken(|v| {
            v["tracks"][0]["clips"][2]["start"] = u64::MAX.into();
            v["tracks"][0]["clips"][2]["duration"] = 5.into();
        })
        .is_err());
        assert!(broken(|v| v["media"][0]["default_duration"] = 0.into()).is_err());
        // A timeline that is not being edited is checked as carefully.
        let stored = |v: &mut serde_json::Value| {
            v["timelines"] = serde_json::json!([{
                "id": 900, "name": "Other", "tracks": v["tracks"].clone()
            }]);
        };
        assert!(
            broken(stored).is_err(),
            "its clips have the same ids as the edited timeline's"
        );
        assert!(broken(|v| {
            v["timelines"] = serde_json::json!([{ "id": 0, "name": "Same id", "tracks": [] }]);
        })
        .is_err());
        let ok = broken(|v| {
            v["timelines"] = serde_json::json!([{ "id": 900, "name": "Empty", "tracks": [] }]);
        })
        .unwrap();
        assert_eq!(ok.timeline_list().len(), 2);
        assert!(ok.clone().validate().is_ok());
    }

    #[test]
    fn bad_opacity_is_repaired() {
        let p = broken(|v| {
            v["tracks"][0]["clips"][0]["opacity"] = 7.5.into();
            v["tracks"][0]["clips"][0]["grade"]["gain"] = serde_json::json!([1e30, 1.0, 1.0]);
        })
        .unwrap();
        let clip = &p.tracks[0].clips[0];
        assert_eq!(clip.opacity, 1.0);
        assert!(clip.grade.gain[0] <= 10.0);
    }

    #[test]
    fn markers_and_marks_are_checked() {
        assert!(broken(
            |v| v["markers"] = serde_json::json!([{"frame": 5, "name": "a", "color": 1}])
        )
        .is_ok());
        // Out of order, duplicated, bad colour.
        assert!(broken(|v| v["markers"] = serde_json::json!([{"frame": 9, "name": "a", "color": 1}, {"frame": 5, "name": "b", "color": 1}])).is_err());
        assert!(broken(|v| v["markers"] = serde_json::json!([{"frame": 5, "name": "a", "color": 1}, {"frame": 5, "name": "b", "color": 1}])).is_err());
        assert!(broken(
            |v| v["markers"] = serde_json::json!([{"frame": 5, "name": "a", "color": 8}])
        )
        .is_err());
        assert!(broken(|v| v["mark_in"] = 10.into()).is_ok());
        assert!(broken(|v| {
            v["mark_in"] = 10.into();
            v["mark_out"] = 10.into();
        })
        .is_err());
        assert!(broken(|v| v["mark_out"] = u64::MAX.into()).is_err());
    }

    #[test]
    fn files_from_before_markers_and_enabling_still_open() {
        let mut v: serde_json::Value =
            serde_json::from_str(&Project::demo().to_json().unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("markers");
        v.as_object_mut().unwrap().remove("mark_in");
        v.as_object_mut().unwrap().remove("mark_out");
        v["tracks"][0]["clips"][0]
            .as_object_mut()
            .unwrap()
            .remove("enabled");
        let p = Project::from_json(&v.to_string()).unwrap();
        assert!(p.markers.is_empty() && p.mark_in.is_none());
        assert!(p.tracks[0].clips[0].enabled);
    }

    #[test]
    fn oversized_text_is_refused() {
        let big = " ".repeat(MAX_FILE_BYTES + 1);
        assert!(matches!(
            Project::from_json(&big),
            Err(EditError::BadFile(_))
        ));
    }

    #[test]
    fn the_project_schema_has_its_expected_fields() {
        // A regression guard: if a top-level field is removed or renamed, this fails.
        let v: serde_json::Value =
            serde_json::from_str(&Project::demo().to_json().unwrap()).unwrap();
        let o = v.as_object().unwrap();
        for field in [
            "version", "width", "height", "fps", "name", "tracks", "media",
        ] {
            assert!(o.contains_key(field), "missing required field: {field}");
        }
        assert_eq!(v["version"], FILE_VERSION);
        assert_eq!(v["fps"]["num"], 24);
        assert_eq!(v["fps"]["den"], 1);
    }

    #[test]
    fn a_truncated_project_is_refused_without_panicking() {
        // Simulates a crash mid-write: the file is valid JSON up to a point, then cuts off.
        let full = Project::demo().to_json().unwrap();
        for cut in [10, 50, 200, 500, full.len() / 2].into_iter().take(5) {
            let truncated = &full[..cut];
            assert!(
                Project::from_json(truncated).is_err(),
                "truncated at {cut} should be refused, not panic"
            );
        }
    }

    #[test]
    fn unknown_fields_are_ignored() {
        // Forward compatibility: a project from a future version with extra fields still opens.
        let mut v: serde_json::Value =
            serde_json::from_str(&Project::demo().to_json().unwrap()).unwrap();
        v["future_feature"] = serde_json::json!({ "new_thing": 42 });
        v["tracks"][0]["clips"][0]["future_clip_field"] = "hello".into();
        let mut p = Project::from_json(&v.to_string()).unwrap();
        assert!(p.validate().is_ok());
    }
}
