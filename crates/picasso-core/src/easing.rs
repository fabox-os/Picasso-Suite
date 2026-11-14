//! The curves a keyframe can ease along to the next one: a straight line, a jump (hold), the
//! usual cubic curves (sine, quad, cubic, quart, quint, expo, circ, back; each in, out and
//! in-out) and the ones that cannot be written as a cubic: elastic (it overshoots and rings) and
//! bounce (it lands and bounces). A curve maps how far along the time between two keyframes
//! (0 to 1) is to how far along the value is (0 to 1, and past it for the curves that overshoot).

use serde::{Deserialize, Serialize};

/// How the value moves from a keyframe to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum KeyCurve {
    /// An even rate.
    #[default]
    Linear,
    /// Stays at the first value until the next keyframe, then jumps.
    Hold,
    /// Slow near both keyframes (a smooth step).
    Smooth,
    /// A cubic curve, as written in CSS: the two handles `x1, y1, x2, y2`.
    Bezier([f32; 4]),
    ElasticIn,
    ElasticOut,
    ElasticInOut,
    BounceIn,
    BounceOut,
    BounceInOut,
}

impl KeyCurve {
    pub fn is_linear(&self) -> bool {
        *self == Self::Linear
    }

    /// The progress of the value at progress `u` of the time. The ends are exact (0 at 0 and 1
    /// at 1), so a segment always lands on its keyframes.
    pub fn eval(self, u: f32) -> f32 {
        if !u.is_finite() || u <= 0.0 {
            return 0.0;
        }
        if u >= 1.0 {
            return 1.0;
        }
        match self {
            Self::Linear => u,
            Self::Hold => 0.0,
            Self::Smooth => u * u * (3.0 - 2.0 * u),
            Self::Bezier(h) => bezier(h, u),
            Self::ElasticIn => elastic_in(u),
            Self::ElasticOut => 1.0 - elastic_in(1.0 - u),
            Self::ElasticInOut => {
                if u < 0.5 {
                    0.5 * elastic_in(2.0 * u)
                } else {
                    1.0 - 0.5 * elastic_in(2.0 - 2.0 * u)
                }
            }
            Self::BounceIn => 1.0 - bounce_out(1.0 - u),
            Self::BounceOut => bounce_out(u),
            Self::BounceInOut => {
                if u < 0.5 {
                    0.5 * (1.0 - bounce_out(1.0 - 2.0 * u))
                } else {
                    0.5 + 0.5 * bounce_out(2.0 * u - 1.0)
                }
            }
        }
    }
}

/// Elastic, slow to start: the usual formula with a period of 0.3.
fn elastic_in(u: f32) -> f32 {
    let period = 0.3f32;
    let s = period / 4.0;
    let v = u - 1.0;
    -(2f32.powf(10.0 * v)) * ((v - s) * std::f32::consts::TAU / period).sin()
}

/// A ball dropped on the floor and bouncing, landing at 1.
fn bounce_out(u: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if u < 1.0 / D {
        N * u * u
    } else if u < 2.0 / D {
        let u = u - 1.5 / D;
        N * u * u + 0.75
    } else if u < 2.5 / D {
        let u = u - 2.25 / D;
        N * u * u + 0.9375
    } else {
        let u = u - 2.625 / D;
        N * u * u + 0.984_375
    }
}

/// The cubic curve through (0, 0) and (1, 1) with handles (x1, y1) and (x2, y2): the y of the
/// point whose x is `u`.
fn bezier([x1, y1, x2, y2]: [f32; 4], u: f32) -> f32 {
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let at = |a: f32, b: f32, t: f32| 3.0 * a * (1.0 - t) * (1.0 - t) * t + 3.0 * b * (1.0 - t) * t * t + t * t * t;
    // x grows with t, so it can be solved by halving.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut t = u;
    for _ in 0..40 {
        let x = at(x1, x2, t);
        if (x - u).abs() < 1e-6 {
            break;
        }
        if x < u {
            lo = t;
        } else {
            hi = t;
        }
        t = 0.5 * (lo + hi);
    }
    at(y1, y2, t)
}

/// A named curve, in a family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePreset {
    pub id: &'static str,
    pub label: &'static str,
    pub family: &'static str,
    pub curve: KeyCurve,
}

const fn preset(id: &'static str, label: &'static str, family: &'static str, curve: KeyCurve) -> CurvePreset {
    CurvePreset {
        id,
        label,
        family,
        curve,
    }
}

const fn bez(x1: f32, y1: f32, x2: f32, y2: f32) -> KeyCurve {
    KeyCurve::Bezier([x1, y1, x2, y2])
}

/// The curves the keyframe menu offers, family by family.
pub const PRESETS: [CurvePreset; 34] = [
    preset("linear", "Straight", "Basic", KeyCurve::Linear),
    preset("hold", "Jump", "Basic", KeyCurve::Hold),
    preset("smooth", "Smooth", "Basic", KeyCurve::Smooth),
    preset("ease", "Ease", "Basic", bez(0.25, 0.1, 0.25, 1.0)),
    preset("sine_in", "Sine In", "Sine", bez(0.12, 0.0, 0.39, 0.0)),
    preset("sine_out", "Sine Out", "Sine", bez(0.61, 1.0, 0.88, 1.0)),
    preset("sine_in_out", "Sine In-Out", "Sine", bez(0.37, 0.0, 0.63, 1.0)),
    preset("quad_in", "Quad In", "Quad", bez(0.11, 0.0, 0.5, 0.0)),
    preset("quad_out", "Quad Out", "Quad", bez(0.5, 1.0, 0.89, 1.0)),
    preset("quad_in_out", "Quad In-Out", "Quad", bez(0.45, 0.0, 0.55, 1.0)),
    preset("cubic_in", "Cubic In", "Cubic", bez(0.32, 0.0, 0.67, 0.0)),
    preset("cubic_out", "Cubic Out", "Cubic", bez(0.33, 1.0, 0.68, 1.0)),
    preset("cubic_in_out", "Cubic In-Out", "Cubic", bez(0.65, 0.0, 0.35, 1.0)),
    preset("quart_in", "Quart In", "Quart", bez(0.5, 0.0, 0.75, 0.0)),
    preset("quart_out", "Quart Out", "Quart", bez(0.25, 1.0, 0.5, 1.0)),
    preset("quart_in_out", "Quart In-Out", "Quart", bez(0.76, 0.0, 0.24, 1.0)),
    preset("quint_in", "Quint In", "Quint", bez(0.64, 0.0, 0.78, 0.0)),
    preset("quint_out", "Quint Out", "Quint", bez(0.22, 1.0, 0.36, 1.0)),
    preset("quint_in_out", "Quint In-Out", "Quint", bez(0.83, 0.0, 0.17, 1.0)),
    preset("expo_in", "Expo In", "Expo", bez(0.7, 0.0, 0.84, 0.0)),
    preset("expo_out", "Expo Out", "Expo", bez(0.16, 1.0, 0.3, 1.0)),
    preset("expo_in_out", "Expo In-Out", "Expo", bez(0.87, 0.0, 0.13, 1.0)),
    preset("circ_in", "Circ In", "Circ", bez(0.55, 0.0, 1.0, 0.45)),
    preset("circ_out", "Circ Out", "Circ", bez(0.0, 0.55, 0.45, 1.0)),
    preset("circ_in_out", "Circ In-Out", "Circ", bez(0.85, 0.0, 0.15, 1.0)),
    preset("back_in", "Back In", "Back", bez(0.36, 0.0, 0.66, -0.56)),
    preset("back_out", "Back Out", "Back", bez(0.34, 1.56, 0.64, 1.0)),
    preset("back_in_out", "Back In-Out", "Back", bez(0.68, -0.6, 0.32, 1.6)),
    preset("elastic_in", "Elastic In", "Elastic", KeyCurve::ElasticIn),
    preset("elastic_out", "Elastic Out", "Elastic", KeyCurve::ElasticOut),
    preset("elastic_in_out", "Elastic In-Out", "Elastic", KeyCurve::ElasticInOut),
    preset("bounce_in", "Bounce In", "Bounce", KeyCurve::BounceIn),
    preset("bounce_out", "Bounce Out", "Bounce", KeyCurve::BounceOut),
    preset("bounce_in_out", "Bounce In-Out", "Bounce", KeyCurve::BounceInOut),
];

/// The families of [`PRESETS`], in order.
pub const FAMILIES: [&str; 11] = [
    "Basic", "Sine", "Quad", "Cubic", "Quart", "Quint", "Expo", "Circ", "Back", "Elastic", "Bounce",
];

/// The preset a curve is (or nearly is), for showing its name.
pub fn preset_of(curve: KeyCurve) -> Option<&'static CurvePreset> {
    PRESETS.iter().find(|p| match (p.curve, curve) {
        (KeyCurve::Bezier(a), KeyCurve::Bezier(b)) => a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3),
        (a, b) => a == b,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_lands_on_its_keyframes() {
        for p in &PRESETS {
            let c = p.curve;
            assert_eq!(c.eval(0.0), 0.0, "{}", p.id);
            assert_eq!(c.eval(1.0), 1.0, "{}", p.id);
            assert_eq!(c.eval(-1.0), 0.0);
            assert_eq!(c.eval(2.0), 1.0);
            assert_eq!(c.eval(f32::NAN), 0.0);
            for i in 1..20 {
                assert!(c.eval(i as f32 / 20.0).is_finite(), "{} at {i}", p.id);
            }
        }
    }

    #[test]
    fn the_presets_are_listed_once_in_their_families() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        for p in &PRESETS {
            assert!(FAMILIES.contains(&p.family), "{}", p.id);
        }
        for family in FAMILIES {
            assert!(PRESETS.iter().any(|p| p.family == family), "{family}");
        }
        assert_eq!(preset_of(KeyCurve::BounceOut).map(|p| p.id), Some("bounce_out"));
        assert_eq!(preset_of(bez(0.25, 0.1, 0.25, 1.0)).map(|p| p.id), Some("ease"));
        assert!(preset_of(bez(0.1, 0.2, 0.3, 0.4)).is_none());
    }

    #[test]
    fn in_curves_start_slow_and_out_curves_start_fast() {
        for family in ["sine", "quad", "cubic", "quart", "quint", "expo"] {
            let find = |suffix: &str| {
                PRESETS
                    .iter()
                    .find(|p| p.id == format!("{family}_{suffix}"))
                    .unwrap()
                    .curve
            };
            assert!(find("in").eval(0.3) < 0.3, "{family} in");
            assert!(find("out").eval(0.3) > 0.3, "{family} out");
            let io = find("in_out");
            assert!(io.eval(0.25) < 0.25 && io.eval(0.75) > 0.75, "{family} in-out");
            assert!((io.eval(0.5) - 0.5).abs() < 0.02, "{family} in-out middle");
        }
    }

    #[test]
    fn back_goes_past_and_elastic_rings_and_bounce_lands_and_bounces() {
        let back_out = KeyCurve::Bezier([0.34, 1.56, 0.64, 1.0]);
        assert!(back_out.eval(0.5) > 1.0, "back out overshoots");
        let back_in = KeyCurve::Bezier([0.36, 0.0, 0.66, -0.56]);
        assert!(back_in.eval(0.2) < 0.0, "back in pulls back first");
        // Elastic out passes 1 and comes back across it.
        let samples: Vec<f32> = (1..40).map(|i| KeyCurve::ElasticOut.eval(i as f32 / 40.0)).collect();
        assert!(samples.iter().any(|v| *v > 1.0) && samples.iter().any(|v| *v < 1.0 && *v > 0.9));
        // Bounce out hits 1 several times on the way down and never passes it.
        let b: Vec<f32> = (1..100).map(|i| KeyCurve::BounceOut.eval(i as f32 / 100.0)).collect();
        assert!(b.iter().all(|v| *v <= 1.0 + 1e-5));
        let dips = b.windows(2).filter(|w| w[1] < w[0] - 0.005).count();
        assert!(dips >= 3, "{dips} bounces");
        // Bounce in is bounce out the other way round.
        assert!((KeyCurve::BounceIn.eval(0.3) - (1.0 - KeyCurve::BounceOut.eval(0.7))).abs() < 1e-6);
    }

    #[test]
    fn hold_jumps_at_the_end_and_smooth_is_a_smooth_step() {
        assert_eq!(KeyCurve::Hold.eval(0.999), 0.0);
        assert_eq!(KeyCurve::Smooth.eval(0.5), 0.5);
        assert!(KeyCurve::Smooth.eval(0.1) < 0.1);
        // A bezier with its handles on the diagonal is a straight line.
        let line = KeyCurve::Bezier([0.25, 0.25, 0.75, 0.75]);
        for i in 1..10 {
            let u = i as f32 / 10.0;
            assert!((line.eval(u) - u).abs() < 1e-3);
        }
    }

    #[test]
    fn a_curve_is_stored_by_name_and_linear_is_the_default() {
        assert!(KeyCurve::default().is_linear());
        let json = serde_json::to_string(&KeyCurve::BounceOut).unwrap();
        assert_eq!(json, "\"BounceOut\"");
        let back: KeyCurve = serde_json::from_str(&serde_json::to_string(&bez(0.1, 0.2, 0.3, 0.4)).unwrap()).unwrap();
        assert_eq!(back, bez(0.1, 0.2, 0.3, 0.4));
    }
}
