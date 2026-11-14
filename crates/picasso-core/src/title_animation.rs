//! How a title comes in: it fades, slides, zooms, pops, types itself out and so on, over a
//! number of milliseconds from the start of its clip. The drawing is in `picasso-render`.

use serde::{Deserialize, Serialize};

/// How a title comes in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleAnimation {
    /// The title is there from the first frame.
    #[default]
    None,
    Fade,
    SlideLeft,
    SlideRight,
    SlideUp,
    SlideDown,
    /// A short slide up while it fades in.
    Rise,
    /// A short slide down while it fades in.
    Drop,
    ZoomIn,
    ZoomOut,
    /// Grows past its size and settles back.
    Pop,
    /// Lands from twice the size, quickly.
    Stamp,
    SpinIn,
    BlurIn,
    /// Uncovered from the left to the right.
    Wipe,
    /// Letter by letter, from the left.
    Typewriter,
    Flicker,
    GlitchIn,
}

impl TitleAnimation {
    pub const ALL: [Self; 18] = [
        Self::None,
        Self::Fade,
        Self::SlideLeft,
        Self::SlideRight,
        Self::SlideUp,
        Self::SlideDown,
        Self::Rise,
        Self::Drop,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::Pop,
        Self::Stamp,
        Self::SpinIn,
        Self::BlurIn,
        Self::Wipe,
        Self::Typewriter,
        Self::Flicker,
        Self::GlitchIn,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Fade => "Fade",
            Self::SlideLeft => "Slide Left",
            Self::SlideRight => "Slide Right",
            Self::SlideUp => "Slide Up",
            Self::SlideDown => "Slide Down",
            Self::Rise => "Rise",
            Self::Drop => "Drop",
            Self::ZoomIn => "Zoom In",
            Self::ZoomOut => "Zoom Out",
            Self::Pop => "Pop",
            Self::Stamp => "Stamp",
            Self::SpinIn => "Spin In",
            Self::BlurIn => "Blur In",
            Self::Wipe => "Wipe",
            Self::Typewriter => "Typewriter",
            Self::Flicker => "Flicker",
            Self::GlitchIn => "Glitch In",
        }
    }
}

/// The shortest and longest time a title takes to come in, in milliseconds.
pub const ANIMATION_MS: std::ops::RangeInclusive<u32> = 100..=5000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_animation_has_its_own_name_and_stored_name() {
        let mut labels = std::collections::HashSet::new();
        for kind in TitleAnimation::ALL {
            assert!(labels.insert(kind.label()), "{kind:?}");
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(serde_json::from_str::<TitleAnimation>(&json).unwrap(), kind);
        }
        assert_eq!(serde_json::to_string(&TitleAnimation::SpinIn).unwrap(), "\"spin_in\"");
        assert_eq!(TitleAnimation::default(), TitleAnimation::None);
    }
}
