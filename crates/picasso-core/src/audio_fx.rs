//! The sound effects of a track, as settings: a three-band equaliser and a compressor (the
//! mixer's EQ and Dynamics). The signal processing is in `picasso-audio`.

use serde::{Deserialize, Serialize};

use crate::audio_effect::AudioEffect;

/// A three-band equaliser: a low shelf, a bell in the middle and a high shelf.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Equalizer {
    pub enabled: bool,
    /// Each band: its frequency in hertz and its gain in decibels.
    pub low_freq: f32,
    pub low_gain: f32,
    pub mid_freq: f32,
    pub mid_gain: f32,
    /// How narrow the middle band is (0.3 wide to 8 narrow).
    pub mid_q: f32,
    pub high_freq: f32,
    pub high_gain: f32,
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            enabled: false,
            low_freq: 100.0,
            low_gain: 0.0,
            mid_freq: 1000.0,
            mid_gain: 0.0,
            mid_q: 1.0,
            high_freq: 8000.0,
            high_gain: 0.0,
        }
    }
}

/// The most a band boosts or cuts, in decibels.
pub const EQ_RANGE_DB: f32 = 20.0;

fn finite(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

impl Equalizer {
    /// Whether it changes the sound.
    pub fn is_active(&self) -> bool {
        self.enabled && (self.low_gain != 0.0 || self.mid_gain != 0.0 || self.high_gain != 0.0)
    }

    pub fn sanitized(mut self) -> Self {
        let d = Self::default();
        self.low_freq = finite(self.low_freq, d.low_freq).clamp(20.0, 1000.0);
        self.mid_freq = finite(self.mid_freq, d.mid_freq).clamp(100.0, 10_000.0);
        self.high_freq = finite(self.high_freq, d.high_freq).clamp(1000.0, 20_000.0);
        self.mid_q = finite(self.mid_q, d.mid_q).clamp(0.3, 8.0);
        for g in [&mut self.low_gain, &mut self.mid_gain, &mut self.high_gain] {
            *g = finite(*g, 0.0).clamp(-EQ_RANGE_DB, EQ_RANGE_DB);
        }
        self
    }
}

/// A compressor: above the threshold the level rises only 1/ratio as fast.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Compressor {
    pub enabled: bool,
    /// In dBFS.
    pub threshold: f32,
    pub ratio: f32,
    /// How fast it reacts and lets go, in milliseconds.
    pub attack_ms: f32,
    pub release_ms: f32,
    /// Gain added after it, in decibels.
    pub makeup: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: -20.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 120.0,
            makeup: 0.0,
        }
    }
}

impl Compressor {
    pub fn sanitized(mut self) -> Self {
        let d = Self::default();
        self.threshold = finite(self.threshold, d.threshold).clamp(-60.0, 0.0);
        self.ratio = finite(self.ratio, d.ratio).clamp(1.0, 20.0);
        self.attack_ms = finite(self.attack_ms, d.attack_ms).clamp(0.1, 200.0);
        self.release_ms = finite(self.release_ms, d.release_ms).clamp(5.0, 2000.0);
        self.makeup = finite(self.makeup, 0.0).clamp(0.0, 24.0);
        self
    }

    /// How much the level is lowered (in dB, 0 or less) for an input level of `db`.
    pub fn reduction_db(&self, db: f32) -> f32 {
        if db <= self.threshold {
            0.0
        } else {
            (self.threshold + (db - self.threshold) / self.ratio) - db
        }
    }
}

/// The most sound effects one track can carry.
pub const MAX_CHAIN: usize = 8;

/// The effects of one track.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackFx {
    pub eq: Equalizer,
    pub dynamics: Compressor,
    /// The sound effects (reverb, echo, voice changers...), run in this order after the EQ and
    /// the compressor.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<AudioEffect>,
}

impl TrackFx {
    pub fn is_plain(&self) -> bool {
        !self.eq.is_active() && !self.dynamics.enabled && self.chain.is_empty()
    }

    pub fn sanitized(self) -> Self {
        Self {
            eq: self.eq.sanitized(),
            dynamics: self.dynamics.sanitized(),
            chain: self
                .chain
                .into_iter()
                .take(MAX_CHAIN)
                .map(AudioEffect::sanitized)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_kept_in_range() {
        let eq = Equalizer {
            low_gain: 99.0,
            mid_q: f32::NAN,
            high_freq: 1.0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!((eq.low_gain, eq.mid_q, eq.high_freq), (20.0, 1.0, 1000.0));
        let c = Compressor {
            ratio: 0.0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(c.ratio, 1.0);
        let c = Compressor::default();
        assert_eq!(c.reduction_db(-30.0), 0.0);
        // 8 dB over a 4:1 threshold comes out 2 dB over: 6 dB less.
        assert_eq!(c.reduction_db(-12.0), -6.0);
        assert!(TrackFx::default().is_plain());
    }
}
