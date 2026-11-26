//! The curve of a fade: how fast the sound comes in or goes out over the length of the fade.
//!
//! Adapted from the audio crossfade curves of KinoKlip 1.14.2 by SeniorMatt (GPL-3.0-or-later,
//! <https://github.com/SeniorMatt/kinoklip>), which Drift 0.9.4 by CutWire Studios (GPL-3.0) also
//! has in another form (linear, smooth, equal power); written again in Rust for Picasso's mixer.
//! Every curve rises from 0 to 1 over `t` from 0 to 1; a fade out reads it backwards.

use std::f32::consts::{FRAC_PI_2, PI};

use serde::{Deserialize, Serialize};

/// How steep the slow start and the fast start are.
const LATE_RISE_K: f32 = 4.0;
const EARLY_RISE_K: f32 = 9.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FadeCurve {
    /// A straight line in gain: the two sounds of a crossfade add up to a dip of about 3 dB in
    /// the middle.
    #[default]
    Straight,
    /// A quarter of a sine wave: the power stays the same through a crossfade (0.707 in the middle).
    EvenPower,
    /// Between the two: half of the linear and half of the equal power (0.60 in the middle).
    Midway,
    /// Quiet for a while and fast at the end.
    LateRise,
    /// Fast at first and slow at the end.
    EarlyRise,
    /// Gentle at both ends and 0.5 in the middle (a raised cosine).
    Silk,
}

impl FadeCurve {
    /// Every curve, in the order of the lists.
    pub const ALL: [Self; 6] = [
        Self::Straight,
        Self::EvenPower,
        Self::Midway,
        Self::LateRise,
        Self::EarlyRise,
        Self::Silk,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Straight => "Straight",
            Self::EvenPower => "Even power",
            Self::Midway => "Midway",
            Self::LateRise => "Late rise",
            Self::EarlyRise => "Early rise",
            Self::Silk => "Silk",
        }
    }

    /// The gain `t` of the way through a fade in (0 to 1; outside it is cut to it): 0 at the
    /// start and 1 at the end.
    pub fn gain(self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Self::Straight => t,
            Self::EvenPower => (t * FRAC_PI_2).sin(),
            Self::Midway => 0.5 * (t + (t * FRAC_PI_2).sin()),
            Self::LateRise => (LATE_RISE_K * t).exp_m1() / LATE_RISE_K.exp_m1(),
            Self::EarlyRise => (EARLY_RISE_K * t).ln_1p() / EARLY_RISE_K.ln_1p(),
            Self::Silk => 0.5 - 0.5 * (t * PI).cos(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_runs_from_nothing_to_everything_without_going_back() {
        for curve in FadeCurve::ALL {
            assert!(curve.gain(0.0).abs() < 1e-6, "{curve:?}");
            assert!((curve.gain(1.0) - 1.0).abs() < 1e-6, "{curve:?}");
            let mut last = -1.0;
            for i in 0..=100 {
                let g = curve.gain(i as f32 / 100.0);
                assert!(g >= last - 1e-6 && (0.0..=1.0 + 1e-6).contains(&g), "{curve:?} at {i}: {g}");
                last = g;
            }
            // Outside the fade, or not a number: the ends.
            assert_eq!(curve.gain(-3.0), curve.gain(0.0));
            assert_eq!(curve.gain(9.0), curve.gain(1.0));
            assert_eq!(curve.gain(f32::NAN), curve.gain(0.0));
        }
    }

    #[test]
    fn the_middle_of_each_curve_is_where_it_should_be() {
        let mid = |s: FadeCurve| s.gain(0.5);
        assert!((mid(FadeCurve::Straight) - 0.5).abs() < 1e-6);
        assert!((mid(FadeCurve::EvenPower) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!((mid(FadeCurve::Midway) - 0.603_553_4).abs() < 1e-5);
        assert!((mid(FadeCurve::Silk) - 0.5).abs() < 1e-6);
        // A slow start is under the line at the middle, a fast start over it.
        assert!(mid(FadeCurve::LateRise) < 0.2);
        assert!(mid(FadeCurve::EarlyRise) > 0.7);
        // Two equal-power sounds add up to the same power all the way through a crossfade.
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let (a, b) = (FadeCurve::EvenPower.gain(t), FadeCurve::EvenPower.gain(1.0 - t));
            assert!((a * a + b * b - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_curve_is_saved_by_name_and_the_default_is_straight() {
        assert_eq!(FadeCurve::default(), FadeCurve::Straight);
        let text = serde_json::to_string(&FadeCurve::LateRise).unwrap();
        assert_eq!(text, "\"LateRise\"");
        assert_eq!(serde_json::from_str::<FadeCurve>(&text).unwrap(), FadeCurve::LateRise);
    }
}
