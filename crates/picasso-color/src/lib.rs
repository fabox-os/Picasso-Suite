//! Colour grading maths and video scopes.
//!
//! A [`ColorGrade`] is the primary correction of one clip: lift, gamma, gain and offset per
//! channel, contrast around a pivot, saturation and an optional look-up table read from a `.cube`
//! file ([`Lut`]). [`ColorGrade::apply_rgba8`] grades a whole
//! frame. The scopes in [`scopes`] measure a frame (histogram, waveform, vectorscope).
//!
//! Nothing here allocates per pixel, does I/O or can panic, so it is safe to run on any thread.

pub mod curves;
pub mod fx;
pub mod grade;
pub mod hdr;
pub mod key;
pub mod lut;
pub mod mask;
pub mod scopes;

pub use curves::Curves;
pub use fx::{apply_effects, Effect, EffectKind, MAX_EFFECTS};
pub use grade::{ColorGrade, NodeMode};
pub use hdr::{map_rgba8_to_sdr, map_to_sdr, HdrMapping};
pub use lut::{Lut, LutError, LutKind};
pub use mask::{PowerWindow, Qualifier, WindowShape, MAX_WINDOWS};
pub use scopes::{curve_histogram, CurveInput, Histogram, Parade, Vectorscope, Waveform};

/// Rec. 709 luma weights.
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
