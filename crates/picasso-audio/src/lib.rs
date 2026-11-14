//! What the timeline sounds like. The mix itself (which clips sound at which sample, and how
//! they add up) is plain code that works everywhere; the decoders that feed it and the sound
//! card output exist on the desktop only (the browser build has no sound yet).

pub mod dsp;
pub mod effects;
mod loudness;
mod mix;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub mod recorder;
pub mod stretch;
mod wav;

pub use loudness::{KWeighting, Loudness, TruePeak, LOUDNESS_BLOCK, TARGET_LUFS};
pub use mix::{
    frames_to_samples, mix_block, mixdown, plan, AudioPlan, ClipSound, MixdownError, Mixer,
    SampleSource, METERED_TRACKS, SAMPLE_RATE,
};
#[cfg(not(target_arch = "wasm32"))]
pub use native::{AudioOutput, AudioPool, Offline, Peaks};
pub use wav::{encode as encode_wav, header as wav_header, WavFormat};
