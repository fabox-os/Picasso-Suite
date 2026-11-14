//! The track effects' signal processing: the three-band equaliser as biquad filters (the
//! formulas of R. Bristow-Johnson's "Audio EQ Cookbook") and a stereo-linked peak compressor.
//! Allocation-free, for the audio thread.

use picasso_core::{Compressor, Equalizer, TrackFx};

use crate::SAMPLE_RATE;

/// Biquad coefficients, normalised: b0, b1, b2, a1, a2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefficients([f64; 5]);

impl Coefficients {
    const FLAT: Self = Self([1.0, 0.0, 0.0, 0.0, 0.0]);

    fn new(b: [f64; 3], a: [f64; 3]) -> Self {
        let a0 = if a[0].abs() < 1e-12 { 1.0 } else { a[0] };
        Self([b[0] / a0, b[1] / a0, b[2] / a0, a[1] / a0, a[2] / a0])
    }

    fn peaking(freq: f32, gain_db: f32, q: f32) -> Self {
        if gain_db == 0.0 {
            return Self::FLAT;
        }
        let a = 10f64.powf(f64::from(gain_db) / 40.0);
        let w0 = std::f64::consts::TAU * f64::from(freq) / f64::from(SAMPLE_RATE);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * f64::from(q));
        Self::new(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        )
    }

    fn shelf(freq: f32, gain_db: f32, high: bool) -> Self {
        if gain_db == 0.0 {
            return Self::FLAT;
        }
        let a = 10f64.powf(f64::from(gain_db) / 40.0);
        let w0 = std::f64::consts::TAU * f64::from(freq) / f64::from(SAMPLE_RATE);
        let (sin, cos) = w0.sin_cos();
        // A shelf slope of 1.
        let alpha = sin / 2.0 * std::f64::consts::SQRT_2;
        let k = 2.0 * a.sqrt() * alpha;
        if high {
            Self::new(
                [
                    a * ((a + 1.0) + (a - 1.0) * cos + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - k),
                ],
                [
                    (a + 1.0) - (a - 1.0) * cos + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - k,
                ],
            )
        } else {
            Self::new(
                [
                    a * ((a + 1.0) - (a - 1.0) * cos + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - k),
                ],
                [
                    (a + 1.0) + (a - 1.0) * cos + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - k,
                ],
            )
        }
    }

    /// The gain at `freq` hertz, in decibels.
    fn response_db(&self, freq: f32) -> f32 {
        let w = std::f64::consts::TAU * f64::from(freq) / f64::from(SAMPLE_RATE);
        let [b0, b1, b2, a1, a2] = self.0;
        // |B(e^jw)| / |A(e^jw)|
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (br, bi) = (b0 + b1 * c1 + b2 * c2, -(b1 * s1 + b2 * s2));
        let (ar, ai) = (1.0 + a1 * c1 + a2 * c2, -(a1 * s1 + a2 * s2));
        let mag = ((br * br + bi * bi) / (ar * ar + ai * ai).max(1e-30)).sqrt();
        (20.0 * mag.max(1e-12).log10()) as f32
    }
}

/// The three bands of `eq`, low to high.
fn bands(eq: &Equalizer) -> [Coefficients; 3] {
    [
        Coefficients::shelf(eq.low_freq, eq.low_gain, false),
        Coefficients::peaking(eq.mid_freq, eq.mid_gain, eq.mid_q),
        Coefficients::shelf(eq.high_freq, eq.high_gain, true),
    ]
}

/// The equaliser's gain at `freq` hertz, in decibels (for drawing its curve).
pub fn eq_response_db(eq: &Equalizer, freq: f32) -> f32 {
    if !eq.enabled {
        return 0.0;
    }
    bands(eq).iter().map(|c| c.response_db(freq)).sum()
}

/// One filter's memory, direct form I.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl State {
    fn run(&mut self, c: &Coefficients, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = c.0;
        let y = b0 * x + b1 * self.x1 + b2 * self.x2 - a1 * self.y1 - a2 * self.y2;
        (self.x2, self.x1, self.y2, self.y1) = (self.x1, x, self.y1, y);
        y
    }
}

/// The memory of one track's effects between blocks.
#[derive(Clone, Debug, Default)]
pub struct TrackDsp {
    /// Three bands, two channels.
    eq: [[State; 2]; 3],
    /// The compressor's smoothed gain change, in dB (0 or less).
    envelope: f32,
    /// The deepest gain change in the last block, for a meter.
    pub reduction_db: f32,
    /// The sound effects (reverb, echo, voice changers...), after the EQ and the compressor.
    chain: crate::effects::EffectChain,
}

impl TrackDsp {
    /// Runs `buf` (interleaved stereo) through the track's EQ, then its compressor, then its
    /// sound effects.
    pub fn process(&mut self, fx: &TrackFx, buf: &mut [f32]) {
        if fx.eq.is_active() {
            let coefficients = bands(&fx.eq);
            for frame in buf.chunks_exact_mut(2) {
                for (ch, v) in frame.iter_mut().enumerate() {
                    let mut x = f64::from(*v);
                    for (band, c) in coefficients.iter().enumerate() {
                        x = self.eq[band][ch].run(c, x);
                    }
                    *v = x as f32;
                }
            }
        }
        self.reduction_db = 0.0;
        if fx.dynamics.enabled {
            self.compress(&fx.dynamics, buf);
        }
        self.chain.process(&fx.chain, buf);
    }

    fn compress(&mut self, c: &Compressor, buf: &mut [f32]) {
        let coefficient = |ms: f32| (-1.0 / (ms * SAMPLE_RATE as f32 / 1000.0).max(1.0)).exp();
        let (attack, release) = (coefficient(c.attack_ms), coefficient(c.release_ms));
        for frame in buf.chunks_exact_mut(2) {
            let peak = frame[0].abs().max(frame[1].abs());
            let level = 20.0 * peak.max(1e-9).log10();
            let target = c.reduction_db(level);
            // Down at the attack's speed, back up at the release's.
            let k = if target < self.envelope {
                attack
            } else {
                release
            };
            self.envelope = target + k * (self.envelope - target);
            let gain = 10f32.powf((self.envelope + c.makeup) / 20.0);
            frame[0] *= gain;
            frame[1] *= gain;
            self.reduction_db = self.reduction_db.min(self.envelope);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32, seconds: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        (0..n)
            .flat_map(|i| {
                let v = amp * (std::f32::consts::TAU * freq * i as f32 / SAMPLE_RATE as f32).sin();
                [v, v]
            })
            .collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        // The second half, after the filters settle.
        buf[buf.len() / 2..]
            .iter()
            .fold(0.0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn each_band_lifts_its_part_of_the_sound() {
        let eq = Equalizer {
            enabled: true,
            low_gain: 12.0,
            ..Default::default()
        };
        assert!((eq_response_db(&eq, 20.0) - 12.0).abs() < 0.5);
        assert!(eq_response_db(&eq, 5000.0).abs() < 0.5);
        let mid = Equalizer {
            enabled: true,
            mid_gain: -12.0,
            ..Default::default()
        };
        assert!((eq_response_db(&mid, 1000.0) + 12.0).abs() < 0.1);
        // A 1 kHz tone through the mid cut comes out a quarter as loud.
        let mut buf = sine(1000.0, 0.5, 0.2);
        TrackDsp::default().process(
            &TrackFx {
                eq: mid,
                ..Default::default()
            },
            &mut buf,
        );
        let p = peak(&buf);
        assert!((p - 0.5 * 0.251).abs() < 0.01, "{p}");
        let off = Equalizer {
            enabled: false,
            ..mid
        };
        assert_eq!(eq_response_db(&off, 1000.0), 0.0);
    }

    #[test]
    fn the_compressor_holds_loud_sound_down() {
        let fx = TrackFx {
            dynamics: Compressor {
                enabled: true,
                threshold: -20.0,
                ratio: 4.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // A tone peaking at -6 dBFS: 14 dB over, so about 10.5 dB less.
        let mut loud = sine(200.0, 0.5, 0.5);
        let mut dsp = TrackDsp::default();
        dsp.process(&fx, &mut loud);
        let out_db = 20.0 * peak(&loud).log10();
        assert!((out_db - (-16.5)).abs() < 1.5, "{out_db}");
        assert!(dsp.reduction_db < -9.0);
        // Under the threshold it is left alone.
        let mut quiet = sine(200.0, 0.05, 0.5);
        TrackDsp::default().process(&fx, &mut quiet);
        assert!((peak(&quiet) - 0.05).abs() < 0.002);
    }
}
