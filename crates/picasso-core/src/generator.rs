//! The built-in generators: pictures made from a formula (gradients, test patterns, noise,
//! a window, a grid, a countdown, a timecode). A generator is a kind and up to four colours.

use serde::{Deserialize, Serialize};

/// Which picture a generator makes. The names are stored in project files: do not rename them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorKind {
    /// Four colours, one in each corner.
    FourColorGradient,
    /// From the first colour at the top to the second at the bottom.
    VerticalGradient,
    /// From the first colour at the top left to the second at the bottom right.
    DiagonalGradient,
    /// The first colour in the middle fading to the second at the edge.
    RadialGradient,
    /// Eight full-field colour bars (100 % white, 100 % colours, black).
    FullColorBars,
    /// The eight EBU bars (100 % white, 75 % colours, black).
    EbuBars,
    /// A smooth black-to-white ramp.
    GreyRamp,
    /// Eleven grey steps from black to white.
    GreyScale,
    /// Eight steps of exactly 100 mV of a 700 mV signal.
    #[serde(rename = "steps_100mv")]
    Steps100mV,
    /// Luma, blue difference and red difference ramps, one above the other.
    YcbcrRamp,
    /// The HDR colour bars of ITU-R BT.2111 for HLG, narrow range.
    #[serde(rename = "bt2111_hlg_narrow")]
    Bt2111HlgNarrow,
    /// The HDR colour bars of ITU-R BT.2111 for PQ, narrow range.
    #[serde(rename = "bt2111_pq_narrow")]
    Bt2111PqNarrow,
    /// The HDR colour bars of ITU-R BT.2111 for PQ, full range.
    #[serde(rename = "bt2111_pq_full")]
    Bt2111PqFull,
    /// Grey television noise; a new pattern every frame.
    Noise,
    /// Coloured noise; a new pattern every frame.
    ColorNoise,
    /// Soft clouds that move slowly over time, between the first and second colour.
    NoiseGradient,
    /// Lines on a background, with every fourth line heavier.
    Grid,
    /// A see-through picture with a soft rectangle of colour in the middle.
    WindowRectangle,
    /// A see-through picture with a soft ellipse of colour in the middle.
    WindowEllipse,
    /// A countdown with a sweeping wedge, rings and the number of seconds left.
    Countdown,
    /// The time, burnt in over a transparent picture.
    TimecodeBurnIn,
}

impl GeneratorKind {
    /// Every generator, in menu order.
    pub const ALL: [GeneratorKind; 21] = [
        GeneratorKind::FourColorGradient,
        GeneratorKind::VerticalGradient,
        GeneratorKind::DiagonalGradient,
        GeneratorKind::RadialGradient,
        GeneratorKind::FullColorBars,
        GeneratorKind::EbuBars,
        GeneratorKind::GreyRamp,
        GeneratorKind::GreyScale,
        GeneratorKind::Steps100mV,
        GeneratorKind::YcbcrRamp,
        GeneratorKind::Bt2111HlgNarrow,
        GeneratorKind::Bt2111PqNarrow,
        GeneratorKind::Bt2111PqFull,
        GeneratorKind::Noise,
        GeneratorKind::ColorNoise,
        GeneratorKind::NoiseGradient,
        GeneratorKind::Grid,
        GeneratorKind::WindowRectangle,
        GeneratorKind::WindowEllipse,
        GeneratorKind::Countdown,
        GeneratorKind::TimecodeBurnIn,
    ];

    /// The name in menus and in the media pool.
    pub fn label(self) -> &'static str {
        match self {
            GeneratorKind::FourColorGradient => "Four-Color Gradient",
            GeneratorKind::VerticalGradient => "Vertical Gradient",
            GeneratorKind::DiagonalGradient => "Diagonal Gradient",
            GeneratorKind::RadialGradient => "Radial Gradient",
            GeneratorKind::FullColorBars => "Full-Field Color Bars",
            GeneratorKind::EbuBars => "EBU Color Bars",
            GeneratorKind::GreyRamp => "Grey Ramp",
            GeneratorKind::GreyScale => "Grey Scale",
            GeneratorKind::Steps100mV => "100 mV Steps",
            GeneratorKind::YcbcrRamp => "YCbCr Ramp",
            GeneratorKind::Bt2111HlgNarrow => "HDR Bars (HLG, Narrow)",
            GeneratorKind::Bt2111PqNarrow => "HDR Bars (PQ, Narrow)",
            GeneratorKind::Bt2111PqFull => "HDR Bars (PQ, Full)",
            GeneratorKind::Noise => "Noise",
            GeneratorKind::ColorNoise => "Color Noise",
            GeneratorKind::NoiseGradient => "Noise Gradient",
            GeneratorKind::Grid => "Grid",
            GeneratorKind::WindowRectangle => "Window (Rectangle)",
            GeneratorKind::WindowEllipse => "Window (Ellipse)",
            GeneratorKind::Countdown => "Countdown",
            GeneratorKind::TimecodeBurnIn => "Timecode Burn-In",
        }
    }

    /// The colours a new generator of this kind starts with. Gradients use all four or the
    /// first two; the window, grid, noise gradient and countdown use the first two (the
    /// second is the colour drawn over the first); the rest ignore them.
    pub fn default_colors(self) -> [[u8; 3]; 4] {
        match self {
            GeneratorKind::FourColorGradient => {
                [[235, 64, 52], [255, 200, 40], [40, 90, 220], [40, 190, 120]]
            }
            GeneratorKind::VerticalGradient | GeneratorKind::DiagonalGradient => {
                [[250, 120, 40], [60, 20, 110], [0; 3], [0; 3]]
            }
            GeneratorKind::RadialGradient => [[255, 255, 255], [0, 0, 0], [0; 3], [0; 3]],
            GeneratorKind::NoiseGradient => [[20, 30, 70], [180, 210, 255], [0; 3], [0; 3]],
            GeneratorKind::Grid => [[0, 0, 0], [255, 255, 255], [0; 3], [0; 3]],
            GeneratorKind::WindowRectangle | GeneratorKind::WindowEllipse => {
                [[0, 0, 0], [255, 255, 255], [0; 3], [0; 3]]
            }
            GeneratorKind::Countdown => [[20, 20, 24], [70, 130, 220], [235, 235, 235], [0; 3]],
            GeneratorKind::TimecodeBurnIn => [[0, 0, 0], [255, 255, 255], [0; 3], [0; 3]],
            _ => [[0; 3]; 4],
        }
    }

    /// Whether the picture changes from frame to frame.
    pub fn is_animated(self) -> bool {
        matches!(
            self,
            GeneratorKind::Noise
                | GeneratorKind::ColorNoise
                | GeneratorKind::NoiseGradient
                | GeneratorKind::Countdown
                | GeneratorKind::TimecodeBurnIn
        )
    }

    /// Finds a kind by its label, ignoring case; `None` when there is no such generator.
    pub fn from_name(name: &str) -> Option<GeneratorKind> {
        let name = name.trim();
        GeneratorKind::ALL
            .into_iter()
            .find(|k| k.label().eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generator_has_its_own_name_that_reads_back() {
        for k in GeneratorKind::ALL {
            assert_eq!(GeneratorKind::from_name(k.label()), Some(k));
            assert_eq!(GeneratorKind::ALL.iter().filter(|o| o.label() == k.label()).count(), 1);
        }
    }

    #[test]
    fn stored_names_are_stable() {
        let json = serde_json::to_string(&GeneratorKind::Steps100mV).unwrap();
        assert_eq!(json, "\"steps_100mv\"");
        let back: GeneratorKind = serde_json::from_str("\"four_color_gradient\"").unwrap();
        assert_eq!(back, GeneratorKind::FourColorGradient);
    }
}
