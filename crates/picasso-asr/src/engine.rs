//! The model and the loop that reads a whole sound with it: the sound is taken in windows of 30
//! seconds; for each the model is shown its spectrogram and says what it hears, with times (the
//! rules of [`crate::rules`] keep those in order); the next window starts where the last thing
//! said ended. The language is the one asked for, or what the model hears in the first speech.

use std::path::Path;

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper as m;

use crate::cues::{self, Segment, Window, WINDOW_SECONDS};
use crate::mel::{self, FRAMES_PER_SECOND, SAMPLE_RATE, WINDOW_FRAMES};
use crate::rules::Rules;
use crate::tokens::Tokens;
use crate::{AsrError, Result};

/// The most tokens one window may say.
const MAX_TOKENS: usize = 224;
/// A tail of the sound shorter than this many columns (1.5 seconds) is not read on its own.
const TAIL_FRAMES: usize = 150;
/// A window quieter than this (RMS) is not read: the model makes up words for silence.
const SILENCE: f32 = 0.002;

/// What was heard.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    /// The language the model read the sound in (its code), when it has more than one.
    pub language: Option<String>,
    /// In order, in seconds from the start of the sound.
    pub segments: Vec<Segment>,
}

/// A loaded model.
pub struct Whisper {
    model: m::model::Whisper,
    config: m::Config,
    tokens: Tokens,
    filters: Vec<f32>,
    rules: Rules,
    device: Device,
}

impl Whisper {
    /// Loads the model that is in `dir` (`config.json`, `tokenizer.json`, `model.safetensors`).
    pub fn load(dir: &Path) -> Result<Self> {
        let config: m::Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let tokens = Tokens::from_json(&std::fs::read_to_string(dir.join("tokenizer.json"))?)?;
        let device = Device::Cpu;
        // Read whole rather than mapped, so the file can be replaced while a model is loaded.
        let weights = std::fs::read(dir.join("model.safetensors"))?;
        let vb = VarBuilder::from_buffered_safetensors(weights, m::DTYPE, &device)?;
        let model = m::model::Whisper::load(&vb, config.clone())?;
        let filters = mel::mel_filters(config.num_mel_bins);
        // Never said: what the model's settings list, the start, and the language and task tokens.
        let mut suppress = config.suppress_tokens.clone();
        suppress.push(tokens.sot);
        suppress.extend(tokens.transcribe);
        suppress.extend(tokens.languages.iter().map(|(_, id)| *id));
        let rules = Rules {
            eot: tokens.eot,
            no_timestamps: tokens.no_timestamps,
            timestamp_begin: tokens.timestamp_begin,
            suppress,
            max_initial_step: 50,
        };
        Ok(Self {
            model,
            config,
            tokens,
            filters,
            rules,
            device,
        })
    }

    /// Whether the model knows more than English: the multilingual ones have one more token (the
    /// language tokens are in every model's word list, but only these use them).
    pub fn multilingual(&self) -> bool {
        self.config.vocab_size >= 51_865
    }

    /// The codes of the languages the model knows (none for an English-only model).
    pub fn languages(&self) -> Vec<String> {
        if !self.multilingual() {
            return Vec::new();
        }
        self.tokens.languages.iter().map(|(code, _)| code.clone()).collect()
    }

    /// Which language the model hears in this window: the likeliest of its languages.
    fn detect_language(&mut self, features: &Tensor) -> Result<u32> {
        let start = Tensor::new(&[self.tokens.sot][..], &self.device)?.unsqueeze(0)?;
        let ys = self.model.decoder.forward(&start, features, true)?;
        let (_, seq, _) = ys.dims3()?;
        let logits = self.model.decoder.final_linear(&ys.i((..1, seq - 1..))?)?.i(0)?.i(0)?;
        let logits: Vec<f32> = logits.to_vec1()?;
        self.tokens
            .languages
            .iter()
            .max_by(|a, b| {
                let (x, y) = (
                    logits.get(a.1 as usize).copied().unwrap_or(f32::NEG_INFINITY),
                    logits.get(b.1 as usize).copied().unwrap_or(f32::NEG_INFINITY),
                );
                x.total_cmp(&y)
            })
            .map(|(_, id)| *id)
            .ok_or_else(|| AsrError::msg("the model has no languages"))
    }

    /// What the model says for one window: `prompt` first, then the tokens it picks, one at a time
    /// (the likeliest each time) until it says it is done.
    fn say(&mut self, features: &Tensor, prompt: &[u32]) -> Result<Vec<u32>> {
        let mut tokens = prompt.to_vec();
        let mut said: Vec<u32> = Vec::new();
        for step in 0..MAX_TOKENS.min(self.config.max_target_positions / 2) {
            let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
            let ys = self.model.decoder.forward(&input, features, step == 0)?;
            let (_, seq, _) = ys.dims3()?;
            let logits = self.model.decoder.final_linear(&ys.i((..1, seq - 1..))?)?.i(0)?.i(0)?;
            let mut logits: Vec<f32> = logits.to_vec1()?;
            self.rules.apply(&mut logits, &said);
            let next = logits
                .iter()
                .enumerate()
                .filter(|(_, l)| l.is_finite())
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(self.tokens.eot, |(i, _)| i as u32);
            said.push(next);
            tokens.push(next);
            if next == self.tokens.eot {
                break;
            }
            // Stuck on the same token: a model that has lost its way.
            if said.len() >= 8 && said[said.len() - 8..].iter().all(|t| *t == next) {
                break;
            }
        }
        Ok(said)
    }

    /// Reads `pcm` (mono, 16 kHz). `language` is a code ("en", "de"...) or `None` to let the model
    /// hear it. `progress(share, what)` (0 to 1) gives `false` to stop.
    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        language: Option<&str>,
        progress: &mut dyn FnMut(f32, &str) -> bool,
    ) -> Result<Transcript> {
        if pcm.is_empty() {
            return Ok(Transcript {
                language: None,
                segments: Vec::new(),
            });
        }
        if !progress(0.0, "Preparing the sound") {
            return Err(AsrError::msg("cancelled"));
        }
        let (spectrogram, frames) = mel::log_mel(&self.config, pcm, &self.filters);
        let mel = Tensor::from_vec(spectrogram, (1, self.config.num_mel_bins, frames), &self.device)?;
        let content_frames = pcm.len() / (SAMPLE_RATE / FRAMES_PER_SECOND);
        let multilingual = self.multilingual();
        let mut language_token = match (multilingual, language) {
            (true, Some(code)) => Some(
                self.tokens
                    .language(code)
                    .ok_or_else(|| AsrError::msg(format!("the model does not know the language {code}")))?,
            ),
            _ => None,
        };
        let mut segments: Vec<Segment> = Vec::new();
        let mut seek = 0usize;
        while seek < content_frames {
            let share = seek as f32 / content_frames as f32;
            if !progress(share, "Listening") {
                return Err(AsrError::msg("cancelled"));
            }
            // The sound this window covers, to see whether there is anything to hear.
            let from = seek * (SAMPLE_RATE / FRAMES_PER_SECOND);
            let to = (from + WINDOW_FRAMES * (SAMPLE_RATE / FRAMES_PER_SECOND)).min(pcm.len());
            let heard = &pcm[from.min(pcm.len())..to];
            let rms = (heard.iter().map(|s| s * s).sum::<f32>() / heard.len().max(1) as f32).sqrt();
            if rms < SILENCE {
                seek += WINDOW_FRAMES;
                continue;
            }
            let window = mel.narrow(2, seek, WINDOW_FRAMES)?;
            let features = self.model.encoder.forward(&window, true)?;
            if multilingual && language_token.is_none() {
                language_token = Some(self.detect_language(&features)?);
            }
            let mut prompt = vec![self.tokens.sot];
            if multilingual {
                prompt.extend(language_token);
                prompt.extend(self.tokens.transcribe);
            }
            let said = self.say(&features, &prompt)?;
            let tokens = &self.tokens;
            let Window { segments: found, advance } = cues::parse_window(
                &said,
                tokens.timestamp_begin,
                tokens.eot,
                &|ids| tokens.decode(ids),
            );
            let offset = seek as f64 / FRAMES_PER_SECOND as f64;
            segments.extend(found.into_iter().map(|s| Segment {
                start: s.start + offset,
                end: s.end + offset,
                text: s.text,
            }));
            let step = ((advance.clamp(1.0, WINDOW_SECONDS) * FRAMES_PER_SECOND as f64) as usize).max(1);
            // What is left is a moment of the end of a word: the model only makes words up out of
            // so little ("Thank you."), so the last thing said is let to run to the end.
            if content_frames.saturating_sub(seek + step) < TAIL_FRAMES {
                let end = content_frames as f64 / FRAMES_PER_SECOND as f64;
                if let Some(last) = segments.last_mut() {
                    last.end = last.end.max(end);
                }
                break;
            }
            seek += step;
        }
        progress(1.0, "Done");
        let code = language_token.and_then(|id| {
            self.tokens
                .languages
                .iter()
                .find(|(_, l)| *l == id)
                .map(|(code, _)| code.clone())
        });
        Ok(Transcript {
            language: code,
            segments: cues::drop_repeats(segments),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_a_model_is_refused_in_words() {
        let e = Whisper::load(Path::new("/nonexistent/picasso/model")).err();
        assert!(e.is_some());
    }
}
