//! The sound effects that can be added to a track, as settings: reverb, echo, chorus, voice
//! changers and so on. Each has up to [`MAX_PARAMS`] knobs described by [`AudioEffectKind::params`].
//! The signal processing is in `picasso-audio`.

use serde::{Deserialize, Serialize};

/// The most knobs one effect has.
pub const MAX_PARAMS: usize = 6;

/// One knob of an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioParam {
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Shown after the number: " ms", " Hz", " dB"...
    pub unit: &'static str,
    /// Whether a slider moves it on a logarithmic scale (frequencies and times).
    pub log: bool,
}

const fn knob(label: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> AudioParam {
    AudioParam {
        label,
        min,
        max,
        default,
        unit,
        log: false,
    }
}

const fn log_knob(
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
) -> AudioParam {
    AudioParam {
        label,
        min,
        max,
        default,
        unit,
        log: true,
    }
}

/// Where an effect is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioEffectGroup {
    /// Rooms, echoes and movement between the speakers.
    Space,
    /// Lo-fi and worn-out sound.
    Texture,
    /// Sound that went through a device: a phone, a radio, water.
    Transmission,
    /// Gates, limiters and other tools for fixing a recording.
    Utility,
    /// Voice changers.
    Voice,
}

impl AudioEffectGroup {
    pub const ALL: [Self; 5] = [
        Self::Space,
        Self::Texture,
        Self::Transmission,
        Self::Utility,
        Self::Voice,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Space => "Space",
            Self::Texture => "Texture",
            Self::Transmission => "Transmission",
            Self::Utility => "Utility",
            Self::Voice => "Voice",
        }
    }
}

/// The sound effects of a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioEffectKind {
    Reverb,
    Chorus,
    Flanger,
    Phaser,
    Echo,
    Slapback,
    Tremolo,
    AutoPan,
    StereoWiden,
    Bitcrush,
    EightBit,
    Crystalizer,
    TapeSaturation,
    Vinyl,
    Megaphone,
    Muffled,
    Telephone,
    Underwater,
    WalkieTalkie,
    Gate,
    DeEsser,
    Leveler,
    Limiter,
    Alien,
    Chipmunk,
    DeepVoice,
    Robot,
    DarkLord,
    Wobble,
}

impl AudioEffectKind {
    pub const ALL: [Self; 29] = [
        Self::Reverb,
        Self::Chorus,
        Self::Flanger,
        Self::Phaser,
        Self::Echo,
        Self::Slapback,
        Self::Tremolo,
        Self::AutoPan,
        Self::StereoWiden,
        Self::Bitcrush,
        Self::EightBit,
        Self::Crystalizer,
        Self::TapeSaturation,
        Self::Vinyl,
        Self::Megaphone,
        Self::Muffled,
        Self::Telephone,
        Self::Underwater,
        Self::WalkieTalkie,
        Self::Gate,
        Self::DeEsser,
        Self::Leveler,
        Self::Limiter,
        Self::Alien,
        Self::Chipmunk,
        Self::DeepVoice,
        Self::Robot,
        Self::DarkLord,
        Self::Wobble,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Reverb => "Reverb",
            Self::Chorus => "Chorus",
            Self::Flanger => "Flanger",
            Self::Phaser => "Phaser",
            Self::Echo => "Echo",
            Self::Slapback => "Slapback",
            Self::Tremolo => "Tremolo",
            Self::AutoPan => "Auto Pan",
            Self::StereoWiden => "Stereo Widen",
            Self::Bitcrush => "Bitcrush",
            Self::EightBit => "8-Bit",
            Self::Crystalizer => "Crystalizer",
            Self::TapeSaturation => "Tape",
            Self::Vinyl => "Vinyl",
            Self::Megaphone => "Megaphone",
            Self::Muffled => "Muffled",
            Self::Telephone => "Telephone",
            Self::Underwater => "Underwater",
            Self::WalkieTalkie => "Walkie-Talkie",
            Self::Gate => "Noise Gate",
            Self::DeEsser => "De-Esser",
            Self::Leveler => "Leveler",
            Self::Limiter => "Limiter",
            Self::Alien => "Alien",
            Self::Chipmunk => "Chipmunk",
            Self::DeepVoice => "Deep Voice",
            Self::Robot => "Robot",
            Self::DarkLord => "Dark Lord",
            Self::Wobble => "Wobble",
        }
    }

    pub fn group(self) -> AudioEffectGroup {
        use AudioEffectGroup::*;
        match self {
            Self::Reverb
            | Self::Chorus
            | Self::Flanger
            | Self::Phaser
            | Self::Echo
            | Self::Slapback
            | Self::Tremolo
            | Self::AutoPan
            | Self::StereoWiden => Space,
            Self::Bitcrush
            | Self::EightBit
            | Self::Crystalizer
            | Self::TapeSaturation
            | Self::Vinyl => Texture,
            Self::Megaphone
            | Self::Muffled
            | Self::Telephone
            | Self::Underwater
            | Self::WalkieTalkie => Transmission,
            Self::Gate | Self::DeEsser | Self::Leveler | Self::Limiter => Utility,
            Self::Alien
            | Self::Chipmunk
            | Self::DeepVoice
            | Self::Robot
            | Self::DarkLord
            | Self::Wobble => Voice,
        }
    }

    /// The effect's knobs, in the order of [`AudioEffect::params`].
    pub fn params(self) -> &'static [AudioParam] {
        match self {
            Self::Reverb => const { &[
                knob("Room Size", 0.0, 1.0, 0.5, ""),
                knob("Damping", 0.0, 1.0, 0.5, ""),
                knob("Stereo Width", 0.0, 1.0, 1.0, ""),
                knob("Wet/Dry Mix", 0.0, 1.0, 0.3, ""),
            ] },
            Self::Chorus => const { &[
                log_knob("Delay", 1.0, 100.0, 20.0, " ms"),
                log_knob("Rate", 0.1, 5.0, 0.5, " Hz"),
                knob("Depth", 0.0, 10.0, 2.0, " ms"),
                knob("Mix", 0.0, 1.0, 0.5, ""),
            ] },
            Self::Flanger => const { &[
                log_knob("Rate", 0.1, 10.0, 0.5, " Hz"),
                knob("Delay", 0.0, 30.0, 5.0, " ms"),
                knob("Depth", 0.0, 10.0, 2.0, " ms"),
                knob("Feedback", -95.0, 95.0, 0.0, " %"),
                knob("Mix", 0.0, 100.0, 70.0, " %"),
                knob("Stereo Phase", 0.0, 360.0, 90.0, "°"),
            ] },
            Self::Phaser => const { &[
                log_knob("Rate", 0.1, 2.0, 0.5, " Hz"),
                log_knob("Centre", 200.0, 1500.0, 720.0, " Hz"),
                knob("Depth", 0.0, 1.0, 1.0, ""),
                knob("Feedback", 0.0, 0.95, 0.4, ""),
                knob("Mix", 0.0, 1.0, 0.5, ""),
            ] },
            Self::Echo => const { &[
                log_knob("Delay", 10.0, 900.0, 250.0, " ms"),
                knob("Feedback", 0.0, 0.9, 0.4, ""),
                knob("Mix", 0.0, 1.0, 0.4, ""),
            ] },
            Self::Slapback => const { &[
                knob("Delay", 5.0, 120.0, 80.0, " ms"),
                knob("Level", 0.1, 0.9, 0.4, ""),
            ] },
            Self::Tremolo => const { &[
                log_knob("Rate", 0.1, 20.0, 5.0, " Hz"),
                knob("Depth", 0.0, 1.0, 0.7, ""),
            ] },
            Self::AutoPan => const { &[
                log_knob("Rate", 0.01, 10.0, 0.5, " Hz"),
                knob("Amount", 0.0, 1.0, 1.0, ""),
            ] },
            Self::StereoWiden => const { &[
                knob("Delay", 1.0, 100.0, 20.0, " ms"),
                knob("Feedback", 0.0, 0.9, 0.3, ""),
                knob("Crossfeed", 0.0, 0.8, 0.3, ""),
                knob("Dry Level", 0.0, 1.0, 0.8, ""),
            ] },
            Self::Bitcrush => const { &[
                knob("Bits", 1.0, 16.0, 6.0, ""),
                knob("Downsample", 1.0, 64.0, 16.0, "x"),
                knob("Mix", 0.0, 1.0, 0.7, ""),
            ] },
            Self::EightBit => const { &[
                knob("Bits", 1.0, 16.0, 4.0, ""),
                knob("Downsample", 1.0, 64.0, 8.0, "x"),
                knob("Mix", 0.0, 1.0, 1.0, ""),
            ] },
            Self::Crystalizer => const { &[
                knob("Intensity", 0.0, 10.0, 2.0, ""),
                knob("Levels", 2.0, 64.0, 8.0, ""),
            ] },
            Self::TapeSaturation => const { &[
                knob("Threshold", -40.0, 0.0, -14.0, " dB"),
                knob("Ratio", 1.0, 20.0, 4.0, ":1"),
                log_knob("Attack", 1.0, 200.0, 5.0, " ms"),
                log_knob("Release", 10.0, 1000.0, 50.0, " ms"),
                knob("Saturation", 0.0, 1.0, 0.3, ""),
            ] },
            Self::Vinyl => const { &[
                log_knob("Low Cut", 50.0, 500.0, 200.0, " Hz"),
                log_knob("High Cut", 2000.0, 12_000.0, 6000.0, " Hz"),
                log_knob("Wobble Rate", 0.1, 5.0, 0.5, " Hz"),
                knob("Flutter", 0.0, 0.5, 0.15, ""),
                knob("Crackle", 0.0, 1.0, 0.2, ""),
            ] },
            Self::Megaphone => const { &[
                log_knob("Centre", 300.0, 4000.0, 1500.0, " Hz"),
                log_knob("Width", 200.0, 3000.0, 1200.0, " Hz"),
                knob("Grit", 0.0, 1.0, 0.5, ""),
            ] },
            Self::Muffled => const { &[
                log_knob("Cutoff", 200.0, 4000.0, 800.0, " Hz"),
                knob("Gain", 0.1, 2.0, 1.0, "x"),
            ] },
            Self::Telephone => const { &[
                log_knob("Low Cut", 100.0, 800.0, 300.0, " Hz"),
                log_knob("High Cut", 2000.0, 6000.0, 3400.0, " Hz"),
            ] },
            Self::Underwater => const { &[
                log_knob("Cutoff", 100.0, 2000.0, 500.0, " Hz"),
                knob("Wet", 0.0, 1.0, 0.6, ""),
                log_knob("Motion", 0.1, 2.0, 0.25, " Hz"),
            ] },
            Self::WalkieTalkie => const { &[
                knob("Grit", 0.0, 1.0, 0.4, ""),
                log_knob("Low Cut", 200.0, 800.0, 400.0, " Hz"),
                log_knob("High Cut", 1500.0, 5000.0, 3000.0, " Hz"),
            ] },
            Self::Gate => const { &[
                knob("Threshold", -80.0, -10.0, -34.0, " dB"),
                knob("Ratio", 1.0, 9.0, 3.0, ":1"),
                log_knob("Attack", 1.0, 100.0, 10.0, " ms"),
                log_knob("Release", 20.0, 1000.0, 250.0, " ms"),
            ] },
            Self::DeEsser => const { &[
                knob("Intensity", 0.0, 1.0, 0.35, ""),
                knob("Max Cut", 0.0, 1.0, 0.5, ""),
                log_knob("Frequency", 2000.0, 12_000.0, 7000.0, " Hz"),
            ] },
            Self::Leveler => const { &[
                knob("Strength", 1.0, 20.0, 6.0, "x"),
                knob("Target Peak", 0.5, 1.0, 0.95, ""),
            ] },
            Self::Limiter => const { &[
                knob("Drive", 0.0, 12.0, 0.0, " dB"),
                knob("Ceiling", -6.0, 0.0, -0.5, " dB"),
            ] },
            Self::Alien => const { &[
                log_knob("Speed", 0.1, 5.0, 1.5, " Hz"),
                knob("Delay", 1.0, 20.0, 10.0, " ms"),
                knob("Depth", 1.0, 20.0, 8.0, " ms"),
                knob("Feedback", 0.0, 95.0, 60.0, " %"),
            ] },
            Self::Chipmunk => const { &[knob("Pitch", 1.05, 2.0, 1.5, "x")] },
            Self::DeepVoice => const { &[knob("Pitch", 0.5, 0.95, 0.7, "x")] },
            Self::Robot => const { &[
                log_knob("Frequency", 10.0, 200.0, 50.0, " Hz"),
                knob("Depth", 0.0, 1.0, 1.0, ""),
            ] },
            Self::DarkLord => const { &[
                knob("Pitch", 0.6, 0.95, 0.8, "x"),
                knob("Echo Delay", 10.0, 100.0, 40.0, " ms"),
                knob("Echo Decay", 0.1, 0.9, 0.4, ""),
                knob("Grit", 0.0, 1.0, 0.3, ""),
            ] },
            Self::Wobble => const { &[
                log_knob("Rate", 0.1, 20.0, 4.0, " Hz"),
                knob("Depth", 0.0, 1.0, 0.6, ""),
            ] },
        }
    }
}

/// One effect on a track: which, whether it is on, and where its knobs are set.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioEffect {
    pub kind: AudioEffectKind,
    #[serde(default = "on")]
    pub enabled: bool,
    /// The knobs, in the order of [`AudioEffectKind::params`]; the rest are unused.
    #[serde(default)]
    pub params: [f32; MAX_PARAMS],
}

fn on() -> bool {
    true
}

impl AudioEffect {
    /// The effect with every knob at its default.
    pub fn new(kind: AudioEffectKind) -> Self {
        let mut params = [0.0; MAX_PARAMS];
        for (slot, p) in params.iter_mut().zip(kind.params()) {
            *slot = p.default;
        }
        Self {
            kind,
            enabled: true,
            params,
        }
    }

    /// Every knob inside its range (a missing or broken value goes back to the default).
    pub fn sanitized(mut self) -> Self {
        let specs = self.kind.params();
        for (i, slot) in self.params.iter_mut().enumerate() {
            *slot = match specs.get(i) {
                Some(p) if slot.is_finite() => slot.clamp(p.min, p.max),
                Some(p) => p.default,
                None => 0.0,
            };
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_has_a_name_knobs_and_a_group() {
        let mut names = std::collections::HashSet::new();
        for kind in AudioEffectKind::ALL {
            assert!(names.insert(kind.label()), "{} is listed twice", kind.label());
            let params = kind.params();
            assert!(!params.is_empty() && params.len() <= MAX_PARAMS, "{kind:?}");
            for p in params {
                assert!(p.min < p.max, "{kind:?} {}", p.label);
                assert!((p.min..=p.max).contains(&p.default), "{kind:?} {}", p.label);
                assert!(!p.log || p.min > 0.0, "{kind:?} {} is log from zero", p.label);
            }
            assert!(AudioEffectGroup::ALL.contains(&kind.group()));
        }
        // Each group has something in it.
        for group in AudioEffectGroup::ALL {
            assert!(AudioEffectKind::ALL.iter().any(|k| k.group() == group));
        }
    }

    #[test]
    fn a_new_effect_has_default_knobs_and_sanitizing_keeps_them_in_range() {
        let fx = AudioEffect::new(AudioEffectKind::Echo);
        assert_eq!(fx.params[0], 250.0);
        assert!(fx.enabled);
        let broken = AudioEffect {
            params: [f32::NAN, 99.0, -5.0, 7.0, 7.0, 7.0],
            ..fx
        }
        .sanitized();
        // Delay (no number): its default; feedback: the top; mix: the bottom; unused: zero.
        assert_eq!(broken.params, [250.0, 0.9, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn stored_names_are_stable() {
        let json = serde_json::to_string(&AudioEffect::new(AudioEffectKind::WalkieTalkie)).unwrap();
        assert!(json.contains("\"walkie_talkie\""), "{json}");
        // A file with only the kind still loads: switched on, knobs at zero until sanitized.
        let back: AudioEffect = serde_json::from_str(r#"{"kind":"reverb"}"#).unwrap();
        assert!(back.enabled);
        assert_eq!(back.kind, AudioEffectKind::Reverb);
    }
}
