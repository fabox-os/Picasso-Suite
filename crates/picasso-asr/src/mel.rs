//! The input of Whisper: the log-mel spectrogram of the sound (80 bands, a column every 10 ms).
//! The mel filters are the ones of the reference implementation (Slaney's scale and normalising,
//! 400-point FFT at 16 kHz); the spectrogram itself is `candle`'s.

use candle_transformers::models::whisper as m;

/// The rate the sound must be at, in hertz.
pub const SAMPLE_RATE: usize = m::SAMPLE_RATE;
/// Columns of the spectrogram in 30 seconds, the window Whisper looks at.
pub const WINDOW_FRAMES: usize = m::N_FRAMES;
/// Columns a second.
pub const FRAMES_PER_SECOND: usize = 100;

const N_BINS: usize = m::N_FFT / 2 + 1;

fn hz_to_mel(hz: f64) -> f64 {
    // Linear up to 1 kHz, logarithmic above (Slaney).
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let log_step = 6.4f64.ln() / 27.0;
    if hz >= min_log_hz {
        min_log_mel + (hz / min_log_hz).ln() / log_step
    } else {
        hz / f_sp
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let log_step = 6.4f64.ln() / 27.0;
    if mel >= min_log_mel {
        min_log_hz * (log_step * (mel - min_log_mel)).exp()
    } else {
        mel * f_sp
    }
}

/// The triangular mel filters, `n_mels` rows of 201 weights (one per FFT bin), each normalised
/// to cover an area of 1.
pub fn mel_filters(n_mels: usize) -> Vec<f32> {
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(SAMPLE_RATE as f64 / 2.0));
    let edges: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (n_mels + 1) as f64))
        .collect();
    let bin_hz = SAMPLE_RATE as f64 / m::N_FFT as f64;
    let mut out = vec![0.0f32; n_mels * N_BINS];
    for i in 0..n_mels {
        let (lower, centre, upper) = (edges[i], edges[i + 1], edges[i + 2]);
        let norm = 2.0 / (upper - lower);
        for k in 0..N_BINS {
            let f = k as f64 * bin_hz;
            let w = ((f - lower) / (centre - lower)).min((upper - f) / (upper - centre));
            out[i * N_BINS + k] = (w.max(0.0) * norm) as f32;
        }
    }
    out
}

/// The log-mel spectrogram of `pcm` (mono, 16 kHz): `n_mels` rows of columns, row after row, and
/// the number of columns. Thirty seconds of silence are put after the sound, so every window of
/// [`WINDOW_FRAMES`] columns from the sound's start to its end is whole.
pub fn log_mel(config: &m::Config, pcm: &[f32], filters: &[f32]) -> (Vec<f32>, usize) {
    let mut padded = Vec::with_capacity(pcm.len() + m::N_SAMPLES);
    padded.extend_from_slice(pcm);
    padded.resize(pcm.len() + m::N_SAMPLES, 0.0);
    let mel = m::audio::pcm_to_mel(config, &padded, filters);
    let frames = mel.len() / config.num_mel_bins.max(1);
    (mel, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_runs_both_ways() {
        for hz in [0.0, 100.0, 999.0, 1000.0, 2500.0, 8000.0] {
            assert!((mel_to_hz(hz_to_mel(hz)) - hz).abs() < 1e-6, "{hz}");
        }
        // The linear part: 1 kHz is 15 mel, and each mel is 66.7 Hz.
        assert!((hz_to_mel(1000.0) - 15.0).abs() < 1e-9);
        assert!(hz_to_mel(2000.0) > hz_to_mel(1000.0));
    }

    #[test]
    fn the_filters_are_triangles_that_climb_the_scale_and_cover_an_area_of_one() {
        let f = mel_filters(80);
        assert_eq!(f.len(), 80 * 201);
        assert!(f.iter().all(|w| *w >= 0.0 && w.is_finite()));
        let peak_bin = |row: usize| {
            (0..201)
                .max_by(|a, b| f[row * 201 + a].total_cmp(&f[row * 201 + b]))
                .unwrap_or(0)
        };
        let mut last = 0;
        for row in 0..80 {
            let p = peak_bin(row);
            assert!(p >= last, "row {row} peaks at {p} after {last}");
            last = p;
            assert!(f[row * 201..(row + 1) * 201].iter().any(|w| *w > 0.0), "row {row} is empty");
        }
        // The area (the sum times the bin width) is near one for the wide filters up the scale.
        for row in [40, 60, 79] {
            let area: f32 = f[row * 201..(row + 1) * 201].iter().sum::<f32>() * 40.0;
            assert!((area - 1.0).abs() < 0.12, "row {row}: {area}");
        }
        // The last filter reaches up to the top of the band.
        assert!(f[79 * 201 + 200] > 0.0 || f[79 * 201 + 199] > 0.0);
    }
}
