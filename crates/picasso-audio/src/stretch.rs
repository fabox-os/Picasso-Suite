//! Pitch-preserving time stretching (WSOLA) for clips played faster or slower than normal.
//!
//! The output is built from Hann-windowed grains of [`WINDOW`] samples every [`HOP`] output
//! samples (50 % overlap, the windows sum to exactly 1). Grain `k` sits at output offset
//! `k * HOP` and takes its samples from the source position the speed maps that offset to, moved
//! by up to `TOLERANCE` samples so that it continues the previous grain's waveform (best
//! cross-correlation of the overlap). The pitch therefore stays while the tempo follows the speed.
//!
//! Random access: the grain chain restarts every `ANCHOR_GRAINS` grains, computed from
//! `WARMUP_GRAINS` grains earlier, so rendering a range gives the same samples no matter where
//! playback started or how the range is split into calls. Sequential calls continue the chain.

/// Grain length (21 ms at 48 kHz).
pub const WINDOW: usize = 1024;
/// Output hop (50 % overlap).
pub const HOP: usize = WINDOW / 2;
/// Alignment search range (+-5 ms).
const TOLERANCE: i64 = 256;
/// Chain restart period (about 0.68 s).
const ANCHOR_GRAINS: i64 = 64;
/// Grains computed before each restart.
const WARMUP_GRAINS: i64 = 12;
/// A forward jump up to this many frames is read through instead of seeking.
const READ_THROUGH: i64 = 96_000;
/// Largest alignment window read at once (very high speeds skip the search).
const MAX_SEARCH: i64 = 1 << 18;
/// History kept before the newest request (frames): covers the look-back.
const HISTORY: i64 = 16_384;

/// Reads `buf.len() / 2` stereo frames of the source from frame `first` into `buf` (interleaved).
/// `first` can be negative (before the start of the file); what is not there must read as silence.
pub type Reader<'a> = &'a mut dyn FnMut(i64, &mut [f32]);

/// Recently read source samples (interleaved stereo). Stretched audio reads overlapping and
/// slightly backward ranges on every call; the cache serves them and only reads the new samples
/// after its end, so the decoder keeps its contiguous path.
#[derive(Debug, Default)]
struct SourceCache {
    buffer: Vec<f32>,
    start: i64,
    frames: i64,
}

impl SourceCache {
    /// Frames `[first, first + count)`, valid until the next fetch.
    fn fetch(&mut self, first: i64, count: usize, read: Reader) -> &[f32] {
        let end = first + count as i64;
        let cache_end = self.start + self.frames;
        let inside = self.frames > 0 && first >= self.start && end <= cache_end;
        if !inside {
            if self.frames > 0 && first >= self.start && first <= cache_end + READ_THROUGH {
                // Extend forward: drop history older than HISTORY before the request, then read
                // [cache_end, end).
                let keep_from = (first - HISTORY).clamp(self.start, cache_end);
                let drop = keep_from - self.start;
                if drop > 0 {
                    self.buffer.drain(..(drop * 2) as usize);
                    self.start = keep_from;
                    self.frames -= drop;
                }
                if end > cache_end {
                    let old = (self.frames * 2) as usize;
                    self.buffer
                        .resize(old + ((end - cache_end) * 2) as usize, 0.0);
                    read(cache_end, &mut self.buffer[old..]);
                    self.frames += end - cache_end;
                }
            } else {
                // Before the cache or far ahead: start over (the decoder seeks once).
                self.buffer.clear();
                self.buffer.resize(count * 2, 0.0);
                read(first, &mut self.buffer);
                self.start = first;
                self.frames = count as i64;
            }
        }
        let off = ((first - self.start) * 2) as usize;
        &self.buffer[off..off + count * 2]
    }
}

/// A time stretcher for one clip. Not thread-safe by itself; the mixer owns one per clip.
#[derive(Debug)]
pub struct TimeStretch {
    window: Vec<f32>,
    mono: Vec<f32>,
    cache: SourceCache,
    last_grain: Option<i64>,
    last_source: i64,
}

impl Default for TimeStretch {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeStretch {
    pub fn new() -> Self {
        // Periodic Hann: w[n] + w[n + N/2] == 1 exactly, so 50 % overlap-add has unity gain.
        let window = (0..WINDOW)
            .map(|n| {
                (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / WINDOW as f64).cos()) as f32
            })
            .collect();
        Self {
            window,
            mono: Vec::new(),
            cache: SourceCache::default(),
            last_grain: None,
            last_source: 0,
        }
    }

    /// Forgets the grain chain and the cached samples (the clip or its speed changed).
    pub fn invalidate(&mut self) {
        self.last_grain = None;
        self.cache = SourceCache::default();
    }

    /// Writes output samples `[from, from + out.len() / 2)` (offsets from the stretch origin, for
    /// instance the clip's start) into `out` (interleaved stereo, overwritten). `position` gives
    /// the source sample (fractional) shown at an output offset.
    pub fn render(
        &mut self,
        from: i64,
        out: &mut [f32],
        position: &dyn Fn(i64) -> f64,
        read: Reader,
    ) {
        out.fill(0.0);
        let frames = (out.len() / 2) as i64;
        if frames <= 0 {
            return;
        }
        let to = from + frames;
        let hop = HOP as i64;
        let k0 = (from - WINDOW as i64).div_euclid(hop) + 1; // first grain reaching `from`
        let k1 = (to - 1).div_euclid(hop);
        for k in k0..=k1 {
            let source = self.grain_source(k, position, read);
            let src = self.cache.fetch(source, WINDOW, read);
            let o = k * hop;
            let n0 = (from - o).max(0) as usize;
            let n1 = (to - o).min(WINDOW as i64) as usize;
            for n in n0..n1 {
                let dst = ((o + n as i64 - from) * 2) as usize;
                let w = self.window[n];
                out[dst] += w * src[n * 2];
                out[dst + 1] += w * src[n * 2 + 1];
            }
        }
    }

    /// Source start of grain `k` (chain; uses and updates the last grain).
    fn grain_source(&mut self, k: i64, position: &dyn Fn(i64) -> f64, read: Reader) -> i64 {
        let hop = HOP as i64;
        if self.last_grain == Some(k) {
            return self.last_source;
        }
        let ideal = |grain: i64| position(grain * hop).round() as i64;
        let phase = k.rem_euclid(ANCHOR_GRAINS);
        if self.last_grain == Some(k - 1) && phase != 0 {
            self.last_source = self.align(ideal(k), self.last_source + hop, read);
            self.last_grain = Some(k);
            return self.last_source;
        }
        // Restart the chain WARMUP_GRAINS before the anchor grain, aligned to the unshifted grain
        // before it, and continue grain by grain up to k (the same values a sequential render
        // produces). The warm-up lets the restarted chain lock onto the waveform.
        let first = k - phase - WARMUP_GRAINS;
        let mut source = self.align(ideal(first), ideal(first - 1) + hop, read);
        for j in first + 1..=k {
            source = self.align(ideal(j), source + hop, read);
        }
        self.last_grain = Some(k);
        self.last_source = source;
        source
    }

    /// Best start near `ideal` continuing the waveform at `continuation`.
    fn align(&mut self, ideal: i64, continuation: i64, read: Reader) -> i64 {
        if ideal == continuation {
            return ideal; // 100 %: the natural continuation is the ideal position
        }
        let overlap = (WINDOW - HOP) as i64;
        let lo = continuation.min(ideal - TOLERANCE);
        let hi = (continuation + overlap).max(ideal + TOLERANCE + overlap);
        if hi - lo > MAX_SEARCH {
            return ideal;
        }
        let span = (hi - lo) as usize;
        let p = self.cache.fetch(lo, span, read);
        self.mono.clear();
        self.mono.extend(p.chunks_exact(2).map(|f| f[0] + f[1]));
        let overlap = overlap as usize;
        let tmpl = &self.mono[(continuation - lo) as usize..][..overlap];
        let base = (ideal - lo) as usize;
        let mono = &self.mono;
        let score = |d: i64, step: usize| -> f32 {
            let cand = &mono[(base as i64 + d) as usize..][..overlap];
            tmpl.iter()
                .zip(cand)
                .step_by(step)
                .map(|(a, b)| a * b)
                .sum()
        };
        // Coarse: every second offset and sample; then +-1 at full resolution. Offset 0 wins ties
        // (silence, or a pure DC template).
        let mut best = 0;
        let mut best_score = score(0, 2);
        let mut d = -TOLERANCE;
        while d <= TOLERANCE {
            let s = score(d, 2);
            if s > best_score + 1e-4 * best_score.abs() + 1e-9 {
                best_score = s;
                best = d;
            }
            d += 2;
        }
        let mut refined = best;
        let mut refined_score = score(best, 1);
        for d in [best - 1, best + 1] {
            if !(-TOLERANCE..=TOLERANCE).contains(&d) {
                continue;
            }
            let s = score(d, 1);
            if s > refined_score {
                refined_score = s;
                refined = d;
            }
        }
        ideal + refined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// A stereo sine of `hz`, read from anywhere (silence before 0).
    fn sine_reader(hz: f64) -> impl FnMut(i64, &mut [f32]) {
        move |first, buf| {
            for (i, f) in buf.chunks_exact_mut(2).enumerate() {
                let n = first + i as i64;
                let v = if n < 0 {
                    0.0
                } else {
                    (2.0 * std::f64::consts::PI * hz * n as f64 / RATE).sin() as f32
                };
                f[0] = v;
                f[1] = v;
            }
        }
    }

    /// Zero crossings per second of the left channel of `out`.
    fn frequency(out: &[f32]) -> f64 {
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        let crossings = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        crossings as f64 * RATE / left.len() as f64
    }

    fn render(speed: f64, from: i64, frames: usize) -> Vec<f32> {
        let mut ts = TimeStretch::new();
        let mut out = vec![0.0; frames * 2];
        let mut read = sine_reader(440.0);
        ts.render(from, &mut out, &|o| 10_000.0 + o as f64 * speed, &mut read);
        out
    }

    #[test]
    fn the_pitch_stays_when_the_speed_changes() {
        for speed in [0.5, 0.75, 1.5, 2.0] {
            let out = render(speed, 20_000, 24_000);
            let hz = frequency(&out);
            assert!((hz - 440.0).abs() < 12.0, "speed {speed}: {hz} Hz");
        }
    }

    #[test]
    fn the_level_is_kept() {
        let out = render(1.5, 20_000, 12_000);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.85 && peak < 1.15, "peak {peak}");
    }

    #[test]
    fn splitting_a_range_into_calls_gives_the_same_samples() {
        let whole = render(1.3, 30_000, 9_000);
        let mut ts = TimeStretch::new();
        let mut read = sine_reader(440.0);
        let position = |o: i64| 10_000.0 + o as f64 * 1.3;
        let mut parts = vec![0.0; 9_000 * 2];
        for (i, chunk) in parts.chunks_mut(1_500 * 2).enumerate() {
            ts.render(30_000 + (i * 1_500) as i64, chunk, &position, &mut read);
        }
        let worst = whole
            .iter()
            .zip(&parts)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "difference {worst}");
    }

    #[test]
    fn starting_in_the_middle_matches_starting_at_the_beginning() {
        // a fresh stretcher asked for a late range, against one that walked there
        let late = render(1.3, 90_000, 4_000);
        let mut ts = TimeStretch::new();
        let mut read = sine_reader(440.0);
        let position = |o: i64| 10_000.0 + o as f64 * 1.3;
        let mut sink = vec![0.0; 2_000 * 2];
        let mut at = 0;
        while at + 2_000 <= 90_000 {
            ts.render(at, &mut sink, &position, &mut read);
            at += 2_000;
        }
        let mut walked = vec![0.0; 4_000 * 2];
        ts.render(90_000, &mut walked, &position, &mut read);
        let worst = late
            .iter()
            .zip(&walked)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.05, "difference {worst}");
    }

    #[test]
    fn reading_before_the_start_of_the_file_is_silence_not_a_crash() {
        let mut ts = TimeStretch::new();
        let mut out = vec![0.0; 2_000];
        let mut read = sine_reader(440.0);
        ts.render(0, &mut out, &|o| o as f64 * 2.0, &mut read);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn extreme_speeds_do_not_crash() {
        for speed in [0.01, 0.1, 16.0, 200.0] {
            let out = render(speed, 5_000, 3_000);
            assert!(out.iter().all(|v| v.is_finite()), "speed {speed}");
        }
    }
}
