//! Showing HDR (HLG / PQ) and wide-gamut BT.2020 video on the SDR BT.709 timeline.
//!
//! Phones record "HDR video" as HLG or PQ with BT.2020 primaries; shown as if it were BT.709 the
//! picture looks flat and grey. [`map_to_sdr`] converts one value and [`map_rgba8_to_sdr`] a whole
//! frame (with lookup tables):
//!
//! 1. linearise: HLG inverse OETF and the BT.2100 OOTF (system gamma 1.2, 1000 cd/m²); PQ EOTF;
//!    wide gamut: gamma 2.4;
//! 2. scale so that HDR reference white (203 cd/m², BT.2408) is 1.0;
//! 3. BT.2020 to BT.709 primaries (linear light, negative values clipped);
//! 4. tone map (HLG / PQ): linear below 0.8, highlights rolled off smoothly towards 1.0;
//! 5. encode with the BT.1886 display gamma (1 / 2.4), as SDR video is shown.

use std::sync::OnceLock;

/// How a frame's colours are mapped to the SDR BT.709 working space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrMapping {
    /// SDR BT.709 / BT.601 (or untagged): shown as it is.
    None,
    /// BT.2100 HLG with BT.2020 primaries (phones' "HDR video").
    Hlg,
    /// BT.2100 PQ (HDR10) with BT.2020 primaries.
    Pq,
    /// SDR transfer with BT.2020 primaries: the gamut conversion only.
    WideGamut,
}

const HLG_A: f64 = 0.178_832_77;
const HLG_B: f64 = 0.284_668_92;
const HLG_C: f64 = 0.559_910_73;
const PQ_M1: f64 = 0.159_301_757_812_5;
const PQ_M2: f64 = 78.843_75;
const PQ_C1: f64 = 0.835_937_5;
const PQ_C2: f64 = 18.851_562_5;
const PQ_C3: f64 = 18.687_5;
/// HDR reference white (BT.2408), cd/m².
const REFERENCE_WHITE_NITS: f64 = 203.0;
const HLG_SYSTEM_GAMMA: f64 = 1.2;
/// Linear-light values up to this level are kept; above it highlights are compressed.
const KNEE: f64 = 0.8;
const DISPLAY_GAMMA: f64 = 2.4;
/// BT.2020 to BT.709 primaries in linear light (row-major).
const BT2020_TO_709: [[f64; 3]; 3] = [
    [1.660_491_0, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];
/// BT.2020 luminance weights (the HLG OOTF works on scene luminance).
const LUMA: [f64; 3] = [0.2627, 0.6780, 0.0593];

fn hlg_inverse_oetf(e: f64) -> f64 {
    let e = e.clamp(0.0, 1.0);
    if e <= 0.5 {
        e * e / 3.0
    } else {
        (((e - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

fn pq_eotf_nits(e: f64) -> f64 {
    let p = e.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    10000.0 * ((p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p)).powf(1.0 / PQ_M1)
}

/// Linear display light of the HLG OOTF at reference white (75 % signal).
fn hlg_reference_white() -> f64 {
    hlg_inverse_oetf(0.75).powf(HLG_SYSTEM_GAMMA)
}

fn tone_map(x: f64) -> f64 {
    if x <= KNEE {
        return x;
    }
    let t = (x - KNEE) / (1.0 - KNEE);
    KNEE + (1.0 - KNEE) * t / (1.0 + t)
}

fn encode(linear: f64) -> f64 {
    linear.clamp(0.0, 1.0).powf(1.0 / DISPLAY_GAMMA)
}

/// Step 1 and 2 for one channel (HLG: scene light only; the OOTF needs all three channels).
fn linearise(e: f64, mapping: HdrMapping) -> f64 {
    match mapping {
        HdrMapping::Hlg => hlg_inverse_oetf(e),
        HdrMapping::Pq => pq_eotf_nits(e) / REFERENCE_WHITE_NITS,
        HdrMapping::WideGamut => e.clamp(0.0, 1.0).powf(DISPLAY_GAMMA),
        HdrMapping::None => e,
    }
}

/// HLG OOTF gain for scene luminance `ys`, normalised to reference white = 1.0.
fn hlg_gain(ys: f64) -> f64 {
    if ys > 0.0 {
        ys.powf(HLG_SYSTEM_GAMMA - 1.0) / hlg_reference_white()
    } else {
        0.0
    }
}

/// Maps one non-linear R'G'B' value (0..1, BT.2020 signal) to display-referred BT.709 R'G'B'
/// (0..1). The reference for [`map_rgba8_to_sdr`].
pub fn map_to_sdr(rgb: [f32; 3], mapping: HdrMapping) -> [f32; 3] {
    if mapping == HdrMapping::None {
        return rgb;
    }
    let mut lin = rgb.map(|v| linearise(f64::from(v), mapping));
    if mapping == HdrMapping::Hlg {
        let gain = hlg_gain(LUMA[0] * lin[0] + LUMA[1] * lin[1] + LUMA[2] * lin[2]);
        for v in &mut lin {
            *v *= gain;
        }
    }
    let mut out = [0.0f32; 3];
    for (row, o) in BT2020_TO_709.iter().zip(&mut out) {
        let v = (row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2]).max(0.0);
        *o = encode(if mapping == HdrMapping::WideGamut {
            v
        } else {
            tone_map(v)
        }) as f32;
    }
    out
}

/// Lookup tables of [`map_rgba8_to_sdr`] for one mapping.
struct Tables {
    decode: [f32; 256],
    /// HLG only: gain over scene luminance 0..1.
    gain: Vec<f32>,
    /// Tone map and encode; index = linear light * `ENCODE_STEPS`.
    out: Vec<u8>,
}

const GAIN_SIZE: usize = 4096;
const ENCODE_STEPS: usize = 1024;
/// Linear light above this clips (tone mapped it is about 1).
const ENCODE_MAX: usize = 16;

impl Tables {
    fn new(mapping: HdrMapping) -> Self {
        let mut decode = [0.0f32; 256];
        for (i, d) in decode.iter_mut().enumerate() {
            *d = linearise(i as f64 / 255.0, mapping) as f32;
        }
        let gain = if mapping == HdrMapping::Hlg {
            (0..=GAIN_SIZE)
                .map(|i| hlg_gain(i as f64 / GAIN_SIZE as f64) as f32)
                .collect()
        } else {
            Vec::new()
        };
        let tone_mapped = mapping != HdrMapping::WideGamut;
        let out = (0..=ENCODE_MAX * ENCODE_STEPS)
            .map(|i| {
                let x = i as f64 / ENCODE_STEPS as f64;
                (255.0 * encode(if tone_mapped { tone_map(x) } else { x })).round() as u8
            })
            .collect();
        Self { decode, gain, out }
    }

    fn encode_byte(&self, linear: f32) -> u8 {
        let scaled = linear.clamp(0.0, ENCODE_MAX as f32) * ENCODE_STEPS as f32;
        self.out
            .get((scaled + 0.5) as usize)
            .copied()
            .unwrap_or(255)
    }
}

fn tables_for(mapping: HdrMapping) -> Option<&'static Tables> {
    static HLG: OnceLock<Tables> = OnceLock::new();
    static PQ: OnceLock<Tables> = OnceLock::new();
    static WIDE: OnceLock<Tables> = OnceLock::new();
    match mapping {
        HdrMapping::None => None,
        HdrMapping::Hlg => Some(HLG.get_or_init(|| Tables::new(HdrMapping::Hlg))),
        HdrMapping::Pq => Some(PQ.get_or_init(|| Tables::new(HdrMapping::Pq))),
        HdrMapping::WideGamut => Some(WIDE.get_or_init(|| Tables::new(HdrMapping::WideGamut))),
    }
}

/// Maps every pixel of an RGBA8 picture to SDR BT.709 in place (alpha untouched). Nothing for
/// [`HdrMapping::None`].
pub fn map_rgba8_to_sdr(rgba: &mut [u8], mapping: HdrMapping) {
    let Some(t) = tables_for(mapping) else { return };
    let hlg = mapping == HdrMapping::Hlg;
    let m = BT2020_TO_709.map(|r| r.map(|v| v as f32));
    for px in rgba.chunks_exact_mut(4) {
        let mut r = t.decode[usize::from(px[0])];
        let mut g = t.decode[usize::from(px[1])];
        let mut b = t.decode[usize::from(px[2])];
        if hlg {
            let ys = (LUMA[0] as f32 * r + LUMA[1] as f32 * g + LUMA[2] as f32 * b).min(1.0);
            let gain = t
                .gain
                .get((ys * GAIN_SIZE as f32 + 0.5) as usize)
                .copied()
                .unwrap_or(1.0);
            r *= gain;
            g *= gain;
            b *= gain;
        }
        px[0] = t.encode_byte(m[0][0] * r + m[0][1] * g + m[0][2] * b);
        px[1] = t.encode_byte(m[1][0] * r + m[1][1] * g + m[1][2] * b);
        px[2] = t.encode_byte(m[2][0] * r + m[2][1] * g + m[2][2] * b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_leaves_the_picture_alone() {
        let mut px = [10u8, 20, 30, 40];
        map_rgba8_to_sdr(&mut px, HdrMapping::None);
        assert_eq!(px, [10, 20, 30, 40]);
        assert_eq!(
            map_to_sdr([0.2, 0.4, 0.6], HdrMapping::None),
            [0.2, 0.4, 0.6]
        );
    }

    #[test]
    fn black_stays_black_and_white_stays_white() {
        for mapping in [HdrMapping::Hlg, HdrMapping::Pq, HdrMapping::WideGamut] {
            let black = map_to_sdr([0.0; 3], mapping);
            assert!(black.iter().all(|&v| v < 0.01), "{mapping:?} {black:?}");
            let white = map_to_sdr([1.0; 3], mapping);
            assert!(
                white.iter().all(|&v| v > 0.95 && v <= 1.0),
                "{mapping:?} {white:?}"
            );
        }
    }

    #[test]
    fn grey_stays_grey_and_is_monotonic() {
        for mapping in [HdrMapping::Hlg, HdrMapping::Pq, HdrMapping::WideGamut] {
            let mut last = -1.0;
            for i in 0..=20 {
                let v = i as f32 / 20.0;
                let out = map_to_sdr([v; 3], mapping);
                assert!((out[0] - out[1]).abs() < 0.002 && (out[1] - out[2]).abs() < 0.002);
                assert!(out[0] >= last - 1e-6, "{mapping:?} not monotonic at {v}");
                last = out[0];
            }
        }
    }

    #[test]
    fn hlg_reference_white_lands_near_sdr_white() {
        // 75 % HLG signal is reference white (203 cd/m²).
        let out = map_to_sdr([0.75; 3], HdrMapping::Hlg);
        assert!(out[0] > 0.9, "{out:?}");
    }

    #[test]
    fn tables_match_the_reference() {
        for mapping in [HdrMapping::Hlg, HdrMapping::Pq, HdrMapping::WideGamut] {
            for &(r, g, b) in &[
                (200u8, 120u8, 60u8),
                (30, 30, 30),
                (255, 255, 255),
                (90, 200, 140),
            ] {
                let mut px = [r, g, b, 77];
                map_rgba8_to_sdr(&mut px, mapping);
                let want = map_to_sdr(
                    [
                        f32::from(r) / 255.0,
                        f32::from(g) / 255.0,
                        f32::from(b) / 255.0,
                    ],
                    mapping,
                );
                for c in 0..3 {
                    let diff = (f32::from(px[c]) - want[c] * 255.0).abs();
                    assert!(
                        diff <= 2.0,
                        "{mapping:?} ({r},{g},{b}) channel {c}: {} vs {}",
                        px[c],
                        want[c] * 255.0
                    );
                }
                assert_eq!(px[3], 77);
            }
        }
    }

    #[test]
    fn a_pixel_buffer_with_a_stray_byte_is_safe() {
        let mut px = [1u8, 2, 3, 4, 5];
        map_rgba8_to_sdr(&mut px, HdrMapping::Hlg);
    }
}
