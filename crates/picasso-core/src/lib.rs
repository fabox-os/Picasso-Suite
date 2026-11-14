//! The project model of Picasso Suite.
//!
//! A [`Project`] is pure data: a media pool and tracks of clips on a frame-based timeline.
//! Every change to it is a [`Command`] (an id, parameters, tests) that a [`Session`] dispatches
//! and remembers for undo. The UI, and later a CLI, only ever change a project through commands.
//!
//! The crate does no I/O and never panics; numbers that come from a project file are checked by
//! [`Project::validate`] before the project is used.

pub mod audio_effect;
pub mod audio_fx;
pub mod base64;
pub mod beats;
mod command;
pub mod easing;
pub mod effect_looks;
pub mod edl;
pub mod error;
pub mod generator;
pub mod keyframe;
pub mod multicam;
pub mod otio;
pub mod project;
pub mod session;
pub mod sync;
pub mod time;
pub mod title_animation;
pub mod video_layout;
pub mod tracking;
pub mod xmeml;

pub use audio_effect::{
    AudioEffect, AudioEffectGroup, AudioEffectKind, AudioParam, MAX_PARAMS,
};
pub use audio_fx::{Compressor, Equalizer, TrackFx, EQ_RANGE_DB, MAX_CHAIN};
pub use command::{Command, Outcome, MAX_RAMP_STEPS};
pub use easing::{CurvePreset, KeyCurve};
pub use error::EditError;
pub use generator::GeneratorKind;
pub use keyframe::{KeyParam, KeyTrack, Keyframe, MAX_KEYFRAMES};
pub use multicam::{Angle, Multicam, MAX_ANGLES, MAX_MULTICAMS};
pub use picasso_color::{ColorGrade, Effect, EffectKind, MAX_EFFECTS};
pub use project::{
    media_kind, transition_at, transition_frames, Bin, BlendMode, Clip, ClipAudio, ClipId,
    ImportedClip, ImportedTrack, Marker, MediaId, MediaItem, MediaSource, Project, SmartBin, Still,
    TextAlign, TextStyle, Timeline, TitleText, Track, TrackId, TrackKind, Transform, Transition,
    TransitionKind, CLIP_COLORS, MARKER_COLORS, MAX_BINS, MAX_GAIN_DB, MAX_SPEED, MAX_STILLS,
    MAX_TIMELINES, MAX_TITLE_CHARS, MAX_TRANSITION, MIN_GAIN_DB, MIN_SPEED,
};
pub use session::Session;
pub use time::Fps;
pub use title_animation::TitleAnimation;
pub use tracking::{
    fit_zoom, warp_for_frame, FrameTransform, Matrix3, PictureWarp, StabilizationCropMode, IDENTITY,
    StabilizationMethod, StabilizationSettings, TrackData, TrackPoint, TrackerType,
};
