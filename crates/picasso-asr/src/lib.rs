//! Speech to text, for the subtitles Picasso Suite makes from a clip's sound: OpenAI's Whisper
//! run in pure Rust (on the processor, with `candle`), with the models downloaded the first time
//! one is wanted. Nothing is sent anywhere: the sound never leaves the computer.
//!
//! * [`models`]: which models there are, where they live and how they are downloaded and checked;
//! * [`mel`]: the spectrogram Whisper reads;
//! * [`tokens`]: the model's numbers and words;
//! * [`rules`]: the rules that keep the model's timestamps in order;
//! * [`cues`]: the model's segments cut into subtitle cues;
//! * [`engine`]: the model itself and the loop that reads a whole sound with it.

pub mod cues;
pub mod engine;
pub mod mel;
pub mod models;
pub mod rules;
pub mod tokens;

pub use cues::Cue;
pub use engine::{Transcript, Whisper};
pub use models::Model;

/// The language codes of the multilingual models, in the order of their tokens. English-only
/// models have none.
pub fn languages() -> Vec<String> {
    LANGUAGES.iter().map(|c| (*c).to_string()).collect()
}

/// The 99 languages of Whisper, in the order of its tokens.
const LANGUAGES: [&str; 99] = [
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi",
    "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk",
    "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw",
    "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su",
];

/// What went wrong, in words for the person.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct AsrError(String);

impl AsrError {
    pub(crate) fn msg(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl From<candle_core::Error> for AsrError {
    fn from(e: candle_core::Error) -> Self {
        Self(format!("the speech model failed: {e}"))
    }
}

impl From<std::io::Error> for AsrError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<serde_json::Error> for AsrError {
    fn from(e: serde_json::Error) -> Self {
        Self(format!("a model file could not be read: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, AsrError>;
