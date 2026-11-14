//! Finding the beat of a piece of sound: where it hits (the onsets), how fast it goes (the tempo)
//! and where the regular beats fall. Onsets come from the spectral flux of the sound (how much
//! the spectrum grows from one short moment to the next), the tempo from how the onsets repeat
//! (autocorrelation), and the beats from the tempo and the phase that fits the onsets best.
//!
//! Plain numbers in and out, no files: the caller reads the sound. Mono samples at any rate of
//! 4 kHz or more work; [`crate::sync::SYNC_RATE`] is enough.

use std::f32::consts::PI;

/// A sound has to be this long (in seconds) for a tempo to mean anything.
pub const MIN_SECONDS: f64 = 4.0;

/// One hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Onset {
    /// Seconds from the start of the sound.
    pub seconds: f64,
    /// 0 to 1, against the strongest hit.
    pub strength: f32,
}

/// What was found.
#[derive(Clone, Debug, PartialEq)]
pub struct Beats {
    pub onsets: Vec<Onset>,
    /// Beats a minute, when a tempo was found that can be trusted.
    pub bpm: Option<f64>,
    /// 0 to 1: how clearly the onsets repeat at that tempo.
    pub confidence: f64,
    /// The regular beats, in seconds from the start (empty without a tempo).
    pub beats: Vec<f64>,
}

/// The size of the window the spectrum is taken over, in samples, and how far it moves.
const WINDOW: usize = 512;
const HOP: usize = 128;

/// An in-place radix-2 FFT (`re`, `im`: a power of two long).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f32;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

/// How much new sound there is at each step of [`HOP`] samples: the growth of the (log) spectrum.
fn novelty(samples: &[f32]) -> Vec<f32> {
    if samples.len() < WINDOW + HOP {
        return Vec::new();
    }
    let window: Vec<f32> = (0..WINDOW)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WINDOW as f32).cos())
        .collect();
    let frames = (samples.len() - WINDOW) / HOP + 1;
    let mut previous = vec![0.0f32; WINDOW / 2];
    let mut out = Vec::with_capacity(frames);
    let (mut re, mut im) = (vec![0.0f32; WINDOW], vec![0.0f32; WINDOW]);
    for f in 0..frames {
        let chunk = &samples[f * HOP..f * HOP + WINDOW];
        for i in 0..WINDOW {
            re[i] = chunk[i] * window[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        let mut flux = 0.0f32;
        for (k, prev) in previous.iter_mut().enumerate() {
            let magnitude = (1.0 + 20.0 * re[k].hypot(im[k]) / WINDOW as f32).ln();
            flux += (magnitude - *prev).max(0.0);
            *prev = magnitude;
        }
        out.push(flux);
    }
    // The first frame has nothing before it.
    if let Some(first) = out.first_mut() {
        *first = 0.0;
    }
    out
}

/// The novelty with its slow average taken away and the negative part cut, so only the peaks
/// stand out whatever the loudness.
fn flatten(novelty: &[f32], step_seconds: f64) -> Vec<f32> {
    let radius = ((0.4 / step_seconds) as usize).max(2);
    let n = novelty.len();
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + f64::from(novelty[i]);
    }
    (0..n)
        .map(|i| {
            let (lo, hi) = (i.saturating_sub(radius), (i + radius + 1).min(n));
            let mean = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            (f64::from(novelty[i]) - mean).max(0.0) as f32
        })
        .collect()
}

/// Analyses `samples` (mono, at `rate` hertz).
pub fn analyze(samples: &[f32], rate: u32) -> Beats {
    let none = Beats {
        onsets: Vec::new(),
        bpm: None,
        confidence: 0.0,
        beats: Vec::new(),
    };
    if rate < 4000 || samples.is_empty() {
        return none;
    }
    let step = HOP as f64 / f64::from(rate);
    let raw = novelty(samples);
    if raw.len() < 8 {
        return none;
    }
    let curve = flatten(&raw, step);
    let peak = curve.iter().copied().fold(0.0f32, f32::max);
    // Nothing stands out against the sound's own movement: silence, or an even drone.
    let floor = raw.iter().sum::<f32>() / raw.len() as f32;
    if peak <= 1e-6 || peak < floor * 8.0 {
        return none;
    }
    // Onsets: peaks of the curve that are the highest around them and not small.
    let spread = ((0.05 / step) as usize).max(1);
    let gap = ((0.1 / step) as usize).max(1);
    let threshold = peak * 0.2;
    let mut onsets: Vec<(usize, f32)> = Vec::new();
    for i in 0..curve.len() {
        let v = curve[i];
        if v < threshold {
            continue;
        }
        let (lo, hi) = (i.saturating_sub(spread), (i + spread + 1).min(curve.len()));
        if curve[lo..hi].iter().any(|o| *o > v) {
            continue;
        }
        // A plateau counts once.
        if let Some((last, _)) = onsets.last() {
            if i - last < gap {
                continue;
            }
        }
        onsets.push((i, v));
    }
    let onsets_out: Vec<Onset> = onsets
        .iter()
        .map(|&(i, v)| Onset {
            seconds: (i as f64 * step) + WINDOW as f64 / 2.0 / f64::from(rate),
            strength: v / peak,
        })
        .collect();
    if (samples.len() as f64 / f64::from(rate)) < MIN_SECONDS || onsets.len() < 4 {
        return Beats {
            onsets: onsets_out,
            ..none
        };
    }
    // Tempo: how well the curve matches itself `lag` steps later, for 60 to 180 beats a minute,
    // with a mild liking for the tempi people play at (about 120).
    let mean = curve.iter().sum::<f32>() / curve.len() as f32;
    let centred: Vec<f32> = curve.iter().map(|v| v - mean).collect();
    let (min_lag, max_lag) = ((60.0 / 180.0 / step) as usize, (1.0 / step) as usize);
    let mut scores: Vec<f64> = vec![0.0; max_lag + 2];
    for lag in min_lag.max(1)..=max_lag.min(centred.len() / 2) {
        let sum: f32 = (lag..centred.len()).map(|i| centred[i] * centred[i - lag]).sum();
        scores[lag] = f64::from(sum) / (centred.len() - lag) as f64;
    }
    let weight = |lag: usize| {
        let bpm = 60.0 / (lag as f64 * step);
        let octaves = (bpm / 120.0).log2();
        (-0.5 * (octaves / 1.0).powi(2)).exp()
    };
    let best = (min_lag.max(1)..=max_lag.min(centred.len() / 2))
        .max_by(|a, b| {
            (scores[*a] * weight(*a))
                .partial_cmp(&(scores[*b] * weight(*b)))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .filter(|lag| scores[*lag] > 0.0);
    let Some(best) = best else {
        return Beats {
            onsets: onsets_out,
            ..none
        };
    };
    // Between the steps: the top of the parabola through the score and its neighbours.
    let (a, b, c) = (scores[best - 1], scores[best], scores[best + 1]);
    let bend = if (a - 2.0 * b + c).abs() > 1e-12 {
        (0.5 * (a - c) / (a - 2.0 * b + c)).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let period = (best as f64 + bend) * step;
    let bpm = 60.0 / period;
    let positive: Vec<f64> = scores[min_lag.max(1)..=max_lag.min(centred.len() / 2)]
        .iter()
        .map(|s| s.max(0.0))
        .collect();
    let average = positive.iter().sum::<f64>() / positive.len().max(1) as f64;
    let confidence = if scores[best] > 0.0 && average > 0.0 {
        (1.0 - average / scores[best]).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if confidence < 0.3 {
        return Beats {
            onsets: onsets_out,
            confidence,
            ..none
        };
    }
    // The phase: the first beat is where a grid of this tempo lies on the most onset strength.
    let period_steps = period / step;
    let mut best_phase = (0.0f64, 0.0f64);
    let mut phase = 0.0f64;
    while phase < period_steps {
        let mut total = 0.0f64;
        let mut at = phase;
        while (at.round() as usize) < curve.len() {
            total += f64::from(curve[at.round() as usize]);
            at += period_steps;
        }
        if total > best_phase.1 {
            best_phase = (phase, total);
        }
        phase += 0.25;
    }
    let shift = WINDOW as f64 / 2.0 / f64::from(rate);
    let length = samples.len() as f64 / f64::from(rate);
    let mut beats = Vec::new();
    let mut t = best_phase.0 * step + shift;
    while t < length {
        beats.push(t);
        t += period;
    }
    Beats {
        onsets: onsets_out,
        bpm: Some(bpm),
        confidence,
        beats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 8000;

    /// A click track: a short burst of 1 kHz (and some noise) at every `period` seconds from
    /// `first`, over `seconds`.
    fn clicks(period: f64, first: f64, seconds: f64) -> Vec<f32> {
        let n = (seconds * f64::from(RATE)) as usize;
        let mut v = vec![0.0f32; n];
        let mut noise = 12_345u32;
        let mut t = first;
        while t < seconds {
            let at = (t * f64::from(RATE)) as usize;
            for k in 0..(0.05 * f64::from(RATE)) as usize {
                if at + k >= n {
                    break;
                }
                noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let hiss = (noise >> 16) as f32 / 65_535.0 - 0.5;
                let decay = (-(k as f32) / (0.012 * RATE as f32)).exp();
                v[at + k] += decay * (0.7 * (2.0 * PI * 1000.0 * k as f32 / RATE as f32).sin() + 0.3 * hiss);
            }
            t += period;
        }
        v
    }

    fn near(beats: &[f64], t: f64, within: f64) -> bool {
        beats.iter().any(|b| (b - t).abs() <= within)
    }

    #[test]
    fn the_hits_of_a_click_track_are_found_where_they_are() {
        let found = analyze(&clicks(0.5, 0.25, 20.0), RATE);
        let times: Vec<f64> = found.onsets.iter().map(|o| o.seconds).collect();
        assert!((38..=41).contains(&times.len()), "{} onsets", times.len());
        for k in 0..39 {
            let at = 0.25 + 0.5 * k as f64;
            assert!(near(&times, at, 0.06), "no onset near {at}: {times:?}");
        }
        assert!(found.onsets.iter().all(|o| (0.0..=1.0).contains(&o.strength)));
    }

    #[test]
    fn the_tempo_and_the_beats_of_a_steady_pulse_are_found() {
        for (bpm, first) in [(120.0f64, 0.3f64), (90.0, 0.1), (150.0, 0.2)] {
            let period = 60.0 / bpm;
            let found = analyze(&clicks(period, first, 24.0), RATE);
            let got = found.bpm.unwrap_or_else(|| panic!("no tempo at {bpm}"));
            assert!((got - bpm).abs() < 2.0, "{bpm} found as {got}");
            assert!(found.confidence > 0.3);
            // Every beat of the grid is on a click, and the first is the first click.
            assert!(
                found.beats.iter().take(20).all(|b| near(&(0..).map(|k| first + k as f64 * period).take(80).collect::<Vec<_>>(), *b, 0.06)),
                "{bpm}: {:?}",
                &found.beats[..4.min(found.beats.len())]
            );
            assert!((found.beats[0] - first).abs() < 0.06 || (found.beats[0] - first) % period < 0.06);
        }
    }

    #[test]
    fn silence_and_too_short_sound_have_no_tempo() {
        assert_eq!(analyze(&vec![0.0; 8000 * 10], RATE).bpm, None);
        assert!(analyze(&vec![0.0; 8000 * 10], RATE).onsets.is_empty());
        let short = analyze(&clicks(0.5, 0.2, 3.0), RATE);
        assert_eq!(short.bpm, None);
        assert!(!short.onsets.is_empty(), "the hits are still found");
        assert_eq!(analyze(&[], RATE).bpm, None);
        assert_eq!(analyze(&clicks(0.5, 0.2, 10.0), 1000).bpm, None);
        // A steady hum has no hits.
        let hum: Vec<f32> = (0..8000 * 10).map(|i| 0.3 * (2.0 * PI * 220.0 * i as f32 / 8000.0).sin()).collect();
        assert!(analyze(&hum, RATE).bpm.is_none());
    }

    #[test]
    fn the_fft_finds_a_tone() {
        let n = 512;
        let mut re: Vec<f32> = (0..n).map(|i| (2.0 * PI * 32.0 * i as f32 / n as f32).sin()).collect();
        let mut im = vec![0.0f32; n];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..n / 2).map(|k| re[k].hypot(im[k])).collect();
        let top = mags.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
        assert_eq!(top, 32);
        assert!((mags[32] - 256.0).abs() < 1.0);
    }
}
