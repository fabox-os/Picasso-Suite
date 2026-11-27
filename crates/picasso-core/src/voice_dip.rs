//! Voice dip: the music gets quieter while someone speaks, and comes back after.
//!
//! Adapted from Drift 0.9.4 by CutWire Studios (GPL-3.0,
//! <https://github.com/CutWire-Studios/Drift>), written again in Rust for Picasso's mixer.
//!
//! Speech is found as "louder than a threshold" in windows of 20 ms ([`level_speech_spans`]); the
//! spans are merged so two ramps never overlap and the music does not pump between words
//! ([`merge_spans`]); and a clip keeps the result as a [`VoiceDip`] in timeline frames, which the
//! mixer turns into a gain for every sample ([`voice_dip_factor`]).

use serde::{Deserialize, Serialize};

/// Dips shorter than this (in seconds) are dropped.
pub const MIN_DIP_SECONDS: f64 = 0.05;
/// The most dips one clip keeps.
pub const MAX_DIPS: usize = 4096;
/// The lowest the music is taken, in decibels.
pub const MIN_DIP_DB: f32 = -60.0;
/// The last frame a dip may reach (as for the clips of a project).
pub const MAX_DIP_FRAME: u64 = 1 << 40;
/// The longest ramp, in seconds.
pub const MAX_RAMP_SECONDS: f64 = 10.0;

/// How the music is dipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceDipSettings {
    /// How far the music is taken down, in decibels (0 or less).
    pub reduce_db: f64,
    /// Seconds over which it goes down, ending where the speech starts.
    pub attack: f64,
    /// Seconds over which it comes back, starting where the speech ends.
    pub release: f64,
    /// Pauses shorter than this (plus the two ramps) keep the music down.
    pub hold: f64,
}

impl Default for VoiceDipSettings {
    fn default() -> Self {
        Self {
            reduce_db: -12.0,
            attack: 0.2,
            release: 0.5,
            hold: 0.5,
        }
    }
}

/// A stretch of time, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

fn merge_within(mut spans: Vec<Span>, gap: f64) -> Vec<Span> {
    spans.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<Span> = Vec::new();
    for span in spans {
        if span.start.is_nan() || span.end.is_nan() || span.end < span.start {
            continue;
        }
        match out.last_mut() {
            Some(last) if span.start - last.end < gap => last.end = last.end.max(span.end),
            _ => out.push(span),
        }
    }
    out
}

/// The speech spans sorted, with neighbours closer than hold + attack + release merged, so two
/// ramps never overlap; spans under 50 ms are dropped.
pub fn merge_spans(speech: Vec<Span>, settings: &VoiceDipSettings) -> Vec<Span> {
    let gap = settings.hold + settings.attack + settings.release;
    merge_within(speech, gap)
        .into_iter()
        .filter(|s| s.end - s.start >= MIN_DIP_SECONDS)
        .collect()
}

/// Speech as "louder than a threshold" in windows of 20 ms of `mono` (at `rate` samples a second).
/// `sensitivity` from 0 to 1 sets the threshold from -20 dBFS (only the loud) to -50 dBFS (the
/// quiet too). Gaps under 150 ms are bridged and runs under 100 ms dropped. The times are
/// `start_seconds` plus the offset into `mono`.
pub fn level_speech_spans(mono: &[f32], rate: u32, start_seconds: f64, sensitivity: f64) -> Vec<Span> {
    let window = ((f64::from(rate) * 0.02) as usize).max(1);
    if rate == 0 || mono.len() < window {
        return Vec::new();
    }
    let sensitivity = if sensitivity.is_finite() {
        sensitivity.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let threshold = 10f64.powf((-20.0 - 30.0 * sensitivity) / 20.0);
    let window_seconds = window as f64 / f64::from(rate);
    let count = mono.len() / window;
    let mut runs = Vec::new();
    let mut run_start: Option<usize> = None;
    for w in 0..=count {
        let loud = w < count && {
            let sum: f64 = mono[w * window..(w + 1) * window]
                .iter()
                .map(|&v| f64::from(v) * f64::from(v))
                .sum();
            (sum / window as f64).sqrt() >= threshold
        };
        match (loud, run_start) {
            (true, None) => run_start = Some(w),
            (false, Some(first)) => {
                runs.push(Span {
                    start: start_seconds + first as f64 * window_seconds,
                    end: start_seconds + w as f64 * window_seconds,
                });
                run_start = None;
            }
            _ => {}
        }
    }
    merge_within(runs, 0.15)
        .into_iter()
        .filter(|s| s.end - s.start >= 0.1)
        .collect()
}

/// What a clip keeps of its voice dip: the frames of the timeline where the music is down, and how.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoiceDip {
    /// Where the music is down, as (first frame, end frame) of the timeline, in order.
    pub dips: Vec<(u64, u64)>,
    /// How far it is taken down, in decibels (0 or less).
    pub reduce_db: f32,
    /// Frames over which it goes down, ending at the start of a dip.
    pub attack: u64,
    /// Frames over which it comes back, starting at the end of a dip.
    pub release: u64,
}

impl VoiceDip {
    /// The same values made safe: a level in range, the dips in order, none empty, none close
    /// enough to another for their ramps to overlap (those are joined), and not too many.
    pub fn sanitized(self) -> Self {
        let reduce_db = if self.reduce_db.is_finite() {
            self.reduce_db.clamp(MIN_DIP_DB, 0.0)
        } else {
            VoiceDipSettings::default().reduce_db as f32
        };
        let (attack, release) = (self.attack.min(1 << 20), self.release.min(1 << 20));
        let mut dips = self.dips;
        dips.retain(|d| d.1 > d.0 && d.0 < MAX_DIP_FRAME);
        for d in &mut dips {
            d.1 = d.1.min(MAX_DIP_FRAME);
        }
        dips.sort_unstable();
        let mut joined: Vec<(u64, u64)> = Vec::with_capacity(dips.len());
        for d in dips {
            match joined.last_mut() {
                Some(last) if d.0 < last.1.saturating_add(attack).saturating_add(release) => {
                    last.1 = last.1.max(d.1);
                }
                _ => joined.push(d),
            }
        }
        joined.truncate(MAX_DIPS);
        Self {
            dips: joined,
            reduce_db,
            attack,
            release,
        }
    }

    /// The linear gain of the music at the bottom of a dip.
    pub fn gain(&self) -> f32 {
        10f32.powf(self.reduce_db.min(0.0) / 20.0)
    }
}

/// The factor (1 for none) that voice dip puts on the music at `at`: one of the dips in order and
/// apart by at least `attack + release`, a ramp down over the `attack` before it starts, `gain`
/// inside it and a ramp back up over the `release` after it. All in the same unit (frames or
/// samples).
pub fn voice_dip_factor(dips: &[(u64, u64)], gain: f32, attack: u64, release: u64, at: u64) -> f32 {
    // The first dip whose release is not over yet.
    let i = dips.partition_point(|&(_, end)| end.saturating_add(release) <= at);
    let Some(&(start, end)) = dips.get(i) else {
        return 1.0;
    };
    let ramp_start = start.saturating_sub(attack);
    if at < ramp_start {
        1.0
    } else if at < start {
        let t = (at - ramp_start) as f32 / (start - ramp_start).max(1) as f32;
        1.0 + (gain - 1.0) * t
    } else if at <= end {
        gain
    } else {
        let t = (at - end) as f32 / release.max(1) as f32;
        gain + (1.0 - gain) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, seconds: &[(f64, f64)], total: f64, level: f32) -> Vec<f32> {
        (0..(total * f64::from(rate)) as usize)
            .map(|i| {
                let t = i as f64 / f64::from(rate);
                if seconds.iter().any(|&(a, b)| t >= a && t < b) {
                    level * if i % 2 == 0 { 1.0 } else { -1.0 }
                } else {
                    0.0
                }
            })
            .collect()
    }

    #[test]
    fn loud_stretches_are_found_and_short_gaps_and_blips_are_not() {
        let rate = 8000;
        // Speech from 1.0 to 2.0 s and 2.1 to 3.0 s (a gap under 150 ms), a blip of 40 ms.
        let mono = tone(rate, &[(1.0, 2.0), (2.1, 3.0), (5.0, 5.04)], 6.0, 0.3);
        let found = level_speech_spans(&mono, rate, 10.0, 0.5);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!((found[0].start - 11.0).abs() < 0.03 && (found[0].end - 13.0).abs() < 0.03, "{found:?}");
    }

    #[test]
    fn a_quiet_voice_needs_more_sensitivity() {
        let rate = 8000;
        // 0.02 is -34 dBFS: under the threshold at 0.2 (-26) and over it at 0.8 (-44).
        let mono = tone(rate, &[(1.0, 2.0)], 3.0, 0.02);
        assert!(level_speech_spans(&mono, rate, 0.0, 0.2).is_empty());
        assert_eq!(level_speech_spans(&mono, rate, 0.0, 0.8).len(), 1);
        // Nothing, or nonsense, gives nothing.
        assert!(level_speech_spans(&[], rate, 0.0, 0.5).is_empty());
        assert!(level_speech_spans(&mono, 0, 0.0, 0.5).is_empty());
        // A sensitivity that is not a number is taken as the middle one (-35 dBFS).
        assert_eq!(level_speech_spans(&mono, rate, 0.0, f64::NAN).len(), 1);
    }

    #[test]
    fn near_spans_join_and_far_ones_stay_apart() {
        let s = VoiceDipSettings::default();
        let spans = vec![
            Span { start: 10.0, end: 11.0 },
            Span { start: 1.0, end: 2.0 },
            // 0.9 s after the end of the first: under hold + attack + release (1.2 s).
            Span { start: 2.9, end: 3.5 },
            Span { start: 20.0, end: 20.01 },
        ];
        let merged = merge_spans(spans, &s);
        // The first two are joined; the one of 10 ms is dropped.
        assert_eq!(merged, vec![Span { start: 1.0, end: 3.5 }, Span { start: 10.0, end: 11.0 }]);
    }

    #[test]
    fn the_factor_ramps_down_holds_and_ramps_up() {
        let dips = [(100, 200), (1000, 1100)];
        let g = 0.25;
        let f = |at| voice_dip_factor(&dips, g, 20, 50, at);
        assert_eq!(f(0), 1.0);
        assert_eq!(f(79), 1.0);
        // Halfway down the attack.
        assert!((f(90) - 0.625).abs() < 1e-6, "{}", f(90));
        assert_eq!(f(100), g);
        assert_eq!(f(150), g);
        assert_eq!(f(200), g);
        // Halfway up the release.
        assert!((f(225) - 0.625).abs() < 1e-6, "{}", f(225));
        assert_eq!(f(250), 1.0);
        assert_eq!(f(900), 1.0);
        assert_eq!(f(1050), g);
        assert_eq!(f(5000), 1.0);
        // No dips, no voice dip; no ramps, a step.
        assert_eq!(voice_dip_factor(&[], g, 20, 50, 7), 1.0);
        assert_eq!(voice_dip_factor(&dips, g, 0, 0, 99), 1.0);
        assert_eq!(voice_dip_factor(&dips, g, 0, 0, 100), g);
        assert_eq!(voice_dip_factor(&dips, g, 0, 0, 201), 1.0);
    }

    #[test]
    fn a_voice_dip_is_made_safe() {
        let voice_dip = VoiceDip {
            dips: vec![(500, 600), (10, 10), (100, 200), (210, 300), (50, 40)],
            reduce_db: f32::NAN,
            attack: 5,
            release: 10,
        }
        .sanitized();
        // The empty ones are gone, the order is right, and 100-200 and 210-300 (10 apart, under
        // 15) are one.
        assert_eq!(voice_dip.dips, vec![(100, 300), (500, 600)]);
        assert_eq!(voice_dip.reduce_db, -12.0);
        let deep = VoiceDip { reduce_db: -500.0, ..voice_dip.clone() }.sanitized();
        assert_eq!(deep.reduce_db, MIN_DIP_DB);
        let loud = VoiceDip { reduce_db: 9.0, ..voice_dip }.sanitized();
        assert_eq!(loud.reduce_db, 0.0);
        assert!((VoiceDip { reduce_db: -6.0, ..loud }.gain() - 0.501_187).abs() < 1e-4);
    }
}
