//! Looks: stacks of effects that give a clip a style in one click (a cinematic grade, a neon
//! night, a retro tape, a glitch cut...). Each is a short list of effects with their strengths;
//! applying one adds them after the clip's own effects, and each can be changed or taken off as
//! any other effect.

use picasso_color::{Effect, EffectKind};

/// One look.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub id: &'static str,
    pub label: &'static str,
    /// The effects, in the order they are applied, with their strength (0 to 100).
    pub effects: &'static [(EffectKind, f32)],
}

impl Look {
    /// The effects of the look, ready to add to a clip.
    pub fn effect_list(&self) -> Vec<Effect> {
        self.effects
            .iter()
            .map(|&(kind, amount)| Effect {
                amount,
                ..Effect::new(kind)
            })
            .collect()
    }
}

use EffectKind as K;

pub const LOOKS: [Look; 12] = [
    Look {
        id: "cinematic",
        label: "Cinematic",
        effects: &[(K::CinematicGrade, 65.0), (K::Halation, 45.0), (K::Vignette, 30.0)],
    },
    Look {
        id: "dream",
        label: "Dream",
        effects: &[(K::BokehDream, 60.0), (K::Halation, 55.0), (K::Glow, 30.0)],
    },
    Look {
        id: "neon_night",
        label: "Neon Night",
        effects: &[(K::NeonEdges, 70.0), (K::Bloom, 45.0), (K::Duotone, 55.0)],
    },
    Look {
        id: "retro_tape",
        label: "Retro Tape",
        effects: &[(K::Super8, 65.0), (K::LightLeak, 45.0), (K::VhsCrt, 35.0)],
    },
    Look {
        id: "sunset_leak",
        label: "Sunset Leak",
        effects: &[(K::LightLeak, 60.0), (K::Halation, 50.0), (K::Saturation, 45.0)],
    },
    Look {
        id: "bling",
        label: "Bling",
        effects: &[(K::Sparkle, 40.0), (K::Halation, 50.0), (K::Bloom, 30.0)],
    },
    Look {
        id: "glitch_cut",
        label: "Glitch Cut",
        effects: &[(K::DigitalGlitch, 50.0), (K::RgbSplit, 40.0), (K::BlockGlitch, 35.0)],
    },
    Look {
        id: "anime_punch",
        label: "Anime Punch",
        effects: &[(K::Halftone, 60.0), (K::ZoomPunch, 60.0), (K::Contrast, 40.0)],
    },
    Look {
        id: "beat_drop",
        label: "Beat Drop",
        effects: &[(K::ZoomPunch, 65.0), (K::CameraShake, 50.0), (K::Flicker, 30.0)],
    },
    Look {
        id: "rgb_rave",
        label: "RGB Rave",
        effects: &[(K::RgbSplit, 45.0), (K::NeonEdges, 55.0), (K::Flicker, 40.0)],
    },
    Look {
        id: "slowmo_drop",
        label: "Slow-Mo Drop",
        effects: &[(K::ZoomPunch, 65.0), (K::Halation, 40.0), (K::Bloom, 35.0)],
    },
    Look {
        id: "snap",
        label: "Snap",
        effects: &[(K::ZoomPunch, 70.0), (K::RadialBlur, 40.0), (K::Shockwave, 40.0)],
    },
];

/// The look with this id.
pub fn look_by_id(id: &str) -> Option<&'static Look> {
    LOOKS.iter().find(|l| l.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_color::MAX_EFFECTS;

    #[test]
    fn every_look_is_listed_once_and_fits_in_a_clip() {
        let mut ids: Vec<&str> = LOOKS.iter().map(|l| l.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), LOOKS.len());
        let mut labels: Vec<&str> = LOOKS.iter().map(|l| l.label).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), LOOKS.len());
        for look in &LOOKS {
            assert!(!look.effects.is_empty() && look.effects.len() <= MAX_EFFECTS, "{}", look.id);
            for (kind, amount) in look.effects {
                assert!((1.0..=100.0).contains(amount), "{} {kind:?}", look.id);
            }
            let list = look.effect_list();
            assert_eq!(list.len(), look.effects.len());
            assert!(list.iter().all(|e| e.enabled));
        }
        assert_eq!(look_by_id("retro_tape").map(|l| l.label), Some("Retro Tape"));
        assert!(look_by_id("nothing").is_none());
    }
}
