//! Video effects on a clip's picture, after its grade: blurs, sharpening, glow, mosaic, colour
//! looks, a vignette and film grain. Each effect has one strength, 0 to 100; sizes are in
//! thousandths of the picture's height, so an effect looks the same at any resolution.

use serde::{Deserialize, Serialize};

use crate::LUMA;

/// The most effects one clip has.
pub const MAX_EFFECTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectKind {
    GaussianBlur,
    Sharpen,
    Mosaic,
    Invert,
    BlackAndWhite,
    Sepia,
    Glow,
    Vignette,
    FilmGrain,
    /// Adjust the contribution of each source channel to each output channel (3x3 matrix).
    /// params[0..8] maps to [R→R, R→G, R→B, G→R, G→G, G→B, B→R, B→G, B→B];
    /// each is 0..2 where 1.0 = full channel passthrough.
    RgbMixer,
    /// Shift the white balance: amount is blend strength (0–100, default 100),\n    /// params[0] is temperature shift (-100 = cool/10000K, 0 = neutral/6600K, +100 = warm/3200K).
    ColorTemperature,
    /// Blur along a direction: amount is strength (0–100), params[0] is angle in degrees (0–360).
    DirectionalBlur,
    /// A soft glow: duplicates the picture, blurs it, and blends it back with the original
    /// (screen blend mode), creating a diffusion/halation look.
    Diffusion,
}

impl EffectKind {
    pub const ALL: [Self; 13] = [
        Self::GaussianBlur,
        Self::Sharpen,
        Self::Mosaic,
        Self::Invert,
        Self::BlackAndWhite,
        Self::Sepia,
        Self::Glow,
        Self::Vignette,
        Self::FilmGrain,
        Self::RgbMixer,
        Self::ColorTemperature,
        Self::DirectionalBlur,
        Self::Diffusion,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::GaussianBlur => "Gaussian Blur",
            Self::Sharpen => "Sharpen",
            Self::Mosaic => "Mosaic",
            Self::Invert => "Invert Colors",
            Self::BlackAndWhite => "Black and White",
            Self::Sepia => "Sepia",
            Self::Glow => "Glow",
            Self::Vignette => "Vignette",
            Self::FilmGrain => "Film Grain",
            Self::RgbMixer => "RGB Mixer",
            Self::ColorTemperature => "Color Temperature",
            Self::DirectionalBlur => "Directional Blur",
            Self::Diffusion => "Diffusion",
        }
    }

    /// The strength a new effect starts with.
    pub fn default_amount(self) -> f32 {
        match self {
            Self::GaussianBlur => 20.0,
            Self::Sharpen => 50.0,
            Self::Mosaic => 20.0,
            Self::Invert | Self::BlackAndWhite | Self::Sepia => 100.0,
            Self::Glow => 50.0,
            Self::Vignette => 50.0,
            Self::FilmGrain => 30.0,
            Self::RgbMixer => 100.0,
            Self::ColorTemperature => 100.0,
            Self::DirectionalBlur => 50.0,
            Self::Diffusion => 50.0,
        }
    }
}

/// One effect of a clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub kind: EffectKind,
    /// The strength, 0 to 100.
    pub amount: f32,
    #[serde(default = "on")]
    pub enabled: bool,
    /// Extra parameters for effects that need more than `amount`. Stored as a fixed array so
    /// the struct stays `Copy`; defaults to zeros for backward compatibility (projects saved
    /// before this field existed load with all params at 0).
    #[serde(default)]
    pub params: [f32; 9],
}

fn on() -> bool {
    true
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            amount: kind.default_amount(),
            enabled: true,
            params: Effect::default_params(kind),
        }
    }

    /// Default `params` value for each effect kind. RGB Mixer defaults to the identity matrix
    /// (1.0 on the diagonal, 0 elsewhere) so it starts as a no-op. All others start at zeros.
    fn default_params(kind: EffectKind) -> [f32; 9] {
        match kind {
            EffectKind::RgbMixer => {
                // [R→R, R→G, R→B, G→R, G→G, G→B, B→R, B→G, B→B]
                [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
            }
            EffectKind::ColorTemperature => {
                // params[0] = temperature shift: 0 = neutral (6600K)
                [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
            }
            _ => [0.0; 9],
        }
    }

    pub fn sanitized(mut self) -> Self {
        self.amount = if self.amount.is_finite() {
            self.amount.clamp(0.0, 100.0)
        } else {
            self.kind.default_amount()
        };
        // Sanitize params: replace NaN/Inf with 0, clamp to safe ranges per effect kind.
        for p in &mut self.params {
            if !p.is_finite() {
                *p = 0.0;
            }
        }
        match self.kind {
            EffectKind::RgbMixer => {
                // RGB Mixer matrix values: 0..2 where 1.0 = passthrough.
                for p in &mut self.params {
                    *p = p.clamp(0.0, 2.0);
                }
            }
            EffectKind::ColorTemperature => {
                // params[0] is the temperature shift: -100..100.
                self.params[0] = self.params[0].clamp(-100.0, 100.0);
            }
            EffectKind::DirectionalBlur => {
                // params[0] is the angle: wrap to 0..360.
                self.params[0] = self.params[0].rem_euclid(360.0);
            }
            _ => {}
        }
        self
    }

    /// Applies the effect to `rgba` (`w` x `h`, alpha kept). `frame` seeds the film grain, so
    /// it moves from frame to frame.
    pub fn apply(&self, rgba: &mut [u8], w: usize, h: usize, frame: u64) {
        let Some(len) = w.checked_mul(h) else {
            return;
        };
        if !self.enabled || self.amount <= 0.0 || len == 0 || rgba.len() < len * 4 {
            return;
        }
        let rgba = &mut rgba[..len * 4];
        let a = self.amount / 100.0;
        let unit = h as f32 / 1000.0;
        match self.kind {
            EffectKind::GaussianBlur => {
                let radius = (a * 40.0 * unit).round() as usize;
                if radius > 0 {
                    gaussian(rgba, w, h, radius);
                }
            }
            EffectKind::Sharpen => {
                let mut soft = rgba.to_vec();
                gaussian(&mut soft, w, h, (2.0 * unit).round().max(1.0) as usize);
                let k = a * 2.0;
                for (px, s) in rgba.chunks_exact_mut(4).zip(soft.chunks_exact(4)) {
                    for c in 0..3 {
                        let v = f32::from(px[c]) + (f32::from(px[c]) - f32::from(s[c])) * k;
                        px[c] = v.round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            EffectKind::Mosaic => {
                let size = ((a * 80.0 * unit).round() as usize).max(1);
                if size > 1 {
                    mosaic(rgba, w, h, size);
                }
            }
            EffectKind::Invert => per_pixel(rgba, a, |rgb| rgb.map(|v| 1.0 - v)),
            EffectKind::BlackAndWhite => per_pixel(rgba, a, |rgb| [luma(rgb); 3]),
            EffectKind::Sepia => per_pixel(rgba, a, |[r, g, b]| {
                [
                    (0.393 * r + 0.769 * g + 0.189 * b).min(1.0),
                    (0.349 * r + 0.686 * g + 0.168 * b).min(1.0),
                    (0.272 * r + 0.534 * g + 0.131 * b).min(1.0),
                ]
            }),
            EffectKind::Glow => {
                // The bright parts, blurred, added back over the picture (screen).
                let mut bright: Vec<u8> = rgba
                    .chunks_exact(4)
                    .flat_map(|px| {
                        let y = luma([0, 1, 2].map(|c| f32::from(px[c]) / 255.0));
                        let keep = ((y - 0.6) / 0.4).clamp(0.0, 1.0);
                        [
                            (f32::from(px[0]) * keep) as u8,
                            (f32::from(px[1]) * keep) as u8,
                            (f32::from(px[2]) * keep) as u8,
                            px[3],
                        ]
                    })
                    .collect();
                gaussian(&mut bright, w, h, (20.0 * unit).round().max(1.0) as usize);
                for (px, g) in rgba.chunks_exact_mut(4).zip(bright.chunks_exact(4)) {
                    for c in 0..3 {
                        let (x, y) = (f32::from(px[c]) / 255.0, f32::from(g[c]) / 255.0 * a);
                        px[c] = ((1.0 - (1.0 - x) * (1.0 - y)) * 255.0).round() as u8;
                    }
                }
            }
            EffectKind::Vignette => {
                let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
                let reach = (cx * cx + cy * cy).sqrt().max(1.0);
                for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let (x, y) = ((i % w) as f32 + 0.5 - cx, (i / w) as f32 + 0.5 - cy);
                    let d = (x * x + y * y).sqrt() / reach;
                    let t = ((d - 0.45) / 0.55).clamp(0.0, 1.0);
                    let dark = 1.0 - a * t * t * (3.0 - 2.0 * t);
                    for v in &mut px[..3] {
                        *v = (f32::from(*v) * dark).round() as u8;
                    }
                }
            }
            EffectKind::FilmGrain => {
                let strength = a * 40.0;
                for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let n = noise(i as u64, frame) * strength;
                    for v in &mut px[..3] {
                        *v = (f32::from(*v) + n).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            EffectKind::RgbMixer => {
                // Each output channel is a linear blend of the input channels:
                //   out_R = R*rr + G*gr + B*br
                //   out_G = R*rg + G*gg + B*bg
                //   out_B = R*rb + G*gb + B*bb
                // params[0..9] hold the 3x3 matrix as row-major: [R→R, R→G, R→B, G→R, G→G, G→B, B→R, B→G, B→B].
                let m = self.params;
                for px in rgba.chunks_exact_mut(4) {
                    let r = f32::from(px[0]) / 255.0;
                    let g = f32::from(px[1]) / 255.0;
                    let b = f32::from(px[2]) / 255.0;
                    let nr = (r * m[0] + g * m[3] + b * m[6]).clamp(0.0, 1.0);
                    let ng = (r * m[1] + g * m[4] + b * m[7]).clamp(0.0, 1.0);
                    let nb = (r * m[2] + g * m[5] + b * m[8]).clamp(0.0, 1.0);
                    // Interpolate between the original and the mixed result by `a`.
                    px[0] = ((nr * a + r * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
                    px[1] = ((ng * a + g * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
                    px[2] = ((nb * a + b * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
            EffectKind::ColorTemperature => {
                // Shift the white point by multiplying with a blackbody color.
                // params[0] is the temperature shift: -100 (cool/10000K), 0 (neutral/6600K),
                // +100 (warm/3200K). The amount blends between the original and the tinted
                // result. We interpolate the temperature along the Planckian locus.
                let shift = self.params[0].clamp(-100.0, 100.0);
                let k = 6600.0 - shift * 34.0; // 6600K at shift=0, ±3400K at ±100
                let rgb = blackbody(k);
                for px in rgba.chunks_exact_mut(4) {
                    let r = f32::from(px[0]) / 255.0;
                    let g = f32::from(px[1]) / 255.0;
                    let b = f32::from(px[2]) / 255.0;
                    // Multiply the pixel by the blackbody color, then blend with original by `a`.
                    px[0] = ((r * rgb[0] * a + r * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
                    px[1] = ((g * rgb[1] * a + g * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
                    px[2] = ((b * rgb[2] * a + b * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
            EffectKind::DirectionalBlur => {
                // Blur along an angle (params[0], degrees) with strength `a`. We sample
                // `distance` pixels each side along the direction vector, averaging. This
                // matches the Resolve Directional Blur look (one-axis motion blur).
                let angle = self.params[0].clamp(0.0, 359.0);
                let rad = angle * std::f32::consts::PI / 180.0;
                let (dx, dy) = (rad.cos(), rad.sin());
                let distance = (a * 40.0 * unit).round() as usize;
                if distance == 0 {
                    return;
                }
                let src = rgba.to_vec();
                for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let x = (i % w) as f32;
                    let y = (i / w) as f32;
                    let mut sum = [0u64; 3];
                    let mut n = 0u64;
                    // Sample in both directions: -distance..=distance along the vector.
                    for step in -(distance as i32)..=(distance as i32) {
                        let sx = (x + dx * step as f32).round() as isize;
                        let sy = (y + dy * step as f32).round() as isize;
                        if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                            let si = (sy as usize * w + sx as usize) * 4;
                            sum[0] += u64::from(src[si]);
                            sum[1] += u64::from(src[si + 1]);
                            sum[2] += u64::from(src[si + 2]);
                            n += 1;
                        }
                    }
                    n = n.max(1);
                    px[0] = (sum[0] / n) as u8;
                    px[1] = (sum[1] / n) as u8;
                    px[2] = (sum[2] / n) as u8;
                }
            }
            EffectKind::Diffusion => {
                // Soft glow: duplicate the picture, blur it, then screen-blend the blurred
                // version back onto the original. This produces the classic "diffusion" /
                // soft-focus look where bright areas glow.
                let mut blurred = rgba.to_vec();
                let radius = (a * 15.0 * unit).round().max(1.0) as usize;
                gaussian(&mut blurred, w, h, radius);
                for (px, bg) in rgba.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
                    for c in 0..3 {
                        let x = f32::from(px[c]) / 255.0;
                        let y = f32::from(bg[c]) / 255.0 * a;
                        // Screen blend: 1 - (1-x)(1-y), then interpolate by `a`.
                        px[c] = ((1.0 - (1.0 - x) * (1.0 - y)) * 255.0).round() as u8;
                    }
                }
            }
        }
    }
}

fn luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * LUMA[0] + rgb[1] * LUMA[1] + rgb[2] * LUMA[2]
}

/// Mixes each pixel towards `look` of it by `a` (0 to 1).
fn per_pixel(rgba: &mut [u8], a: f32, look: impl Fn([f32; 3]) -> [f32; 3]) {
    for px in rgba.chunks_exact_mut(4) {
        let rgb = [0, 1, 2].map(|c| f32::from(px[c]) / 255.0);
        let out = look(rgb);
        for c in 0..3 {
            let v = rgb[c] + (out[c] - rgb[c]) * a;
            px[c] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Noise from -1 to 1 for pixel `i` of frame `frame` (a hash, the same every time).
fn noise(i: u64, frame: u64) -> f32 {
    let mut x = i ^ frame.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    (x & 0xffff) as f32 / 32767.5 - 1.0
}

/// Approximates the RGB color of a blackbody radiator at temperature `k` in Kelvin.
/// Uses piecewise-linear interpolation between known anchor points on the Planckian locus.
/// Returns RGB values normalized to 0..1.
///
/// At 6600K the result is near-white (neutral). Below 6600K shifts toward orange/red (warm).
/// Above 6600K shifts toward blue (cool). This matches how Resolve's Color Temperature
/// effect shifts all three channels multiplicatively.
fn blackbody(k: f32) -> [f32; 3] {
    // Anchor points on the Planckian locus: (K, [R, G, B]) in 0..1.
    // 3200K = candlelight/warm (red-heavy). 6600K = daylight neutral. 10000K = cool/blue.
    const ANCHORS: [(f32, [f32; 3]); 4] = [
        (3200.0, [1.0, 0.55, 0.22]),
        (4500.0, [1.0, 0.82, 0.48]),
        (6600.0, [0.95, 0.97, 1.0]),
        (10000.0, [0.61, 0.72, 1.0]),
    ];
    let k = k.clamp(3200.0, 10000.0);
    // Find the two surrounding anchors and lerp.
    for w in 0..ANCHORS.len() - 1 {
        if k <= ANCHORS[w + 1].0 || w == ANCHORS.len() - 2 {
            let (t0, c0) = ANCHORS[w];
            let (t1, c1) = ANCHORS[w + 1];
            let s = (k - t0) / (t1 - t0);
            return [
                c0[0] + (c1[0] - c0[0]) * s,
                c0[1] + (c1[1] - c0[1]) * s,
                c0[2] + (c1[2] - c0[2]) * s,
            ];
        }
    }
    ANCHORS[0].1
}

/// Blocks of `size` pixels, each the average of its pixels.
fn mosaic(rgba: &mut [u8], w: usize, h: usize, size: usize) {
    for by in (0..h).step_by(size) {
        for bx in (0..w).step_by(size) {
            let (x1, y1) = ((bx + size).min(w), (by + size).min(h));
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for y in by..y1 {
                for x in bx..x1 {
                    let i = (y * w + x) * 4;
                    for c in 0..4 {
                        sum[c] += u64::from(rgba[i + c]);
                    }
                    n += 1;
                }
            }
            let mean = sum.map(|s| (s / n.max(1)) as u8);
            for y in by..y1 {
                for x in bx..x1 {
                    let i = (y * w + x) * 4;
                    rgba[i..i + 4].copy_from_slice(&mean);
                }
            }
        }
    }
}

/// A blur close to a Gaussian: three box blurs of `radius / 2` (rows, then columns).
fn gaussian(rgba: &mut [u8], w: usize, h: usize, radius: usize) {
    let r = (radius / 2).max(1).min(w.max(h));
    let mut buf: Vec<f32> = rgba.iter().map(|v| f32::from(*v)).collect();
    for _ in 0..3 {
        box_pass(&mut buf, w, h, r, true);
        box_pass(&mut buf, w, h, r, false);
    }
    for (v, b) in rgba.iter_mut().zip(buf) {
        *v = b.round().clamp(0.0, 255.0) as u8;
    }
}

/// One box blur of RGBA floats along rows (`along_rows`) or columns, edges held.
fn box_pass(buf: &mut [f32], w: usize, h: usize, r: usize, along_rows: bool) {
    let (lines, len) = if along_rows { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| {
        if along_rows {
            (line * w + i) * 4
        } else {
            (i * w + line) * 4
        }
    };
    let mut row = vec![0f32; len * 4];
    let window = (2 * r + 1) as f32;
    for line in 0..lines {
        for i in 0..len {
            row[i * 4..i * 4 + 4].copy_from_slice(&buf[at(line, i)..at(line, i) + 4]);
        }
        let get = |i: i64, c: usize| row[(i.clamp(0, len as i64 - 1) as usize) * 4 + c];
        for c in 0..4 {
            let mut sum: f32 = (-(r as i64)..=r as i64).map(|i| get(i, c)).sum();
            for i in 0..len {
                buf[at(line, i) + c] = sum / window;
                sum += get(i as i64 + r as i64 + 1, c) - get(i as i64 - r as i64, c);
            }
        }
    }
}

/// Applies `effects` in order.
pub fn apply_effects(effects: &[Effect], rgba: &mut [u8], w: usize, h: usize, frame: u64) {
    for e in effects {
        e.apply(rgba, w, h, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 32 x 32 picture: left half black, right half white.
    fn halves() -> Vec<u8> {
        (0..32 * 32)
            .flat_map(|i| {
                let v = if i % 32 < 16 { 0 } else { 255 };
                [v, v, v, 255]
            })
            .collect()
    }

    fn at(p: &[u8], x: usize, y: usize) -> u8 {
        p[(y * 32 + x) * 4]
    }

    #[test]
    fn blur_softens_the_edge_and_sharpen_hardens_it() {
        let mut p = halves();
        Effect::new(EffectKind::GaussianBlur).apply(&mut p, 32, 32, 0);
        // A 32-pixel-high picture: strength 100 for a visible blur.
        let mut q = halves();
        Effect {
            amount: 100.0,
            ..Effect::new(EffectKind::GaussianBlur)
        }
        .apply(&mut q, 32, 32, 0);
        assert!(
            at(&q, 15, 16) > 0 && at(&q, 16, 16) < 255,
            "the edge is soft"
        );
        assert_eq!(at(&q, 0, 0), 0, "far from the edge it stays");
        let mut grey: Vec<u8> = halves().iter().map(|v| v / 2 + 64).collect();
        let before = grey.clone();
        Effect::new(EffectKind::Sharpen).apply(&mut grey, 32, 32, 0);
        assert!(
            at(&grey, 15, 16) < at(&before, 15, 16),
            "darker on the dark side"
        );
        assert!(
            at(&grey, 16, 16) > at(&before, 16, 16),
            "brighter on the bright side"
        );
    }

    #[test]
    fn colour_looks() {
        let red = || vec![200u8, 40, 40, 255];
        let mut p = red();
        Effect::new(EffectKind::Invert).apply(&mut p, 1, 1, 0);
        assert_eq!(p, vec![55, 215, 215, 255]);
        let mut p = red();
        Effect::new(EffectKind::BlackAndWhite).apply(&mut p, 1, 1, 0);
        assert!(p[0] == p[1] && p[1] == p[2]);
        let mut p = red();
        Effect::new(EffectKind::Sepia).apply(&mut p, 1, 1, 0);
        assert!(p[0] > p[1] && p[1] > p[2], "warm brown");
        let mut p = red();
        Effect {
            enabled: false,
            ..Effect::new(EffectKind::Invert)
        }
        .apply(&mut p, 1, 1, 0);
        assert_eq!(p, red(), "switched off");
    }

    #[test]
    fn vignette_mosaic_glow_and_grain() {
        let mut p = vec![200u8; 32 * 32 * 4];
        Effect {
            amount: 100.0,
            ..Effect::new(EffectKind::Vignette)
        }
        .apply(&mut p, 32, 32, 0);
        assert!(at(&p, 0, 0) < 100 && at(&p, 16, 16) == 200);
        let mut p = halves();
        Effect {
            amount: 100.0,
            ..Effect::new(EffectKind::Mosaic)
        }
        .apply(&mut p, 32, 32, 0);
        assert_eq!(at(&p, 0, 0), at(&p, 1, 1));
        let mut p = halves();
        Effect::new(EffectKind::Glow).apply(&mut p, 32, 32, 0);
        assert!(at(&p, 15, 16) > 0, "light spills into the dark");
        let (mut a, mut b) = (vec![128u8; 64 * 4], vec![128u8; 64 * 4]);
        Effect::new(EffectKind::FilmGrain).apply(&mut a, 8, 8, 1);
        Effect::new(EffectKind::FilmGrain).apply(&mut b, 8, 8, 2);
        assert_ne!(a, b, "the grain moves");
        assert!(a.chunks_exact(4).any(|p| p[0] != 128));
        // Sizes that do not fit are left alone.
        let mut tiny = vec![1u8; 3];
        Effect::new(EffectKind::GaussianBlur).apply(&mut tiny, 4, 4, 0);
        assert_eq!(tiny, vec![1, 1, 1]);
    }

    #[test]
    fn rgb_mixer_identity_leaves_pixels_unchanged() {
        // With the identity matrix and amount=100, RGB Mixer should leave pixels unchanged.
        let mut p = vec![200u8, 100, 50, 255, 30, 200, 80, 255];
        let before = p.clone();
        Effect::new(EffectKind::RgbMixer).apply(&mut p, 2, 1, 0);
        assert_eq!(p, before, "identity matrix is a no-op");
    }

    #[test]
    fn rgb_mixer_swaps_channels() {
        // Swap R and B channels: set the matrix to swap red and blue.
        let mut p = vec![200u8, 100, 50, 255];
        let mut e = Effect::new(EffectKind::RgbMixer);
        // params: [R→R=0, R→G=0, R→B=1, G→R=0, G→G=1, G→B=0, B→R=1, B→G=0, B→B=0]
        e.params = [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        e.apply(&mut p, 1, 1, 0);
        assert_eq!(p[0], 50, "red channel now holds original blue");
        assert_eq!(p[2], 200, "blue channel now holds original red");
        assert_eq!(p[1], 100, "green is unchanged");
    }

    #[test]
    fn rgb_mixer_amount_zero_is_noop() {
        let mut p = vec![200u8, 100, 50, 255, 30, 200, 80, 255];
        let before = p.clone();
        let e = Effect { amount: 0.0, kind: EffectKind::RgbMixer, ..Effect::new(EffectKind::RgbMixer) };
        e.apply(&mut p, 2, 1, 0);
        assert_eq!(p, before, "amount=0 leaves pixels unchanged");
    }

    #[test]
    fn rgb_mixer_keeps_alpha() {
        let mut p = vec![200u8, 100, 50, 128];
        let mut e = Effect::new(EffectKind::RgbMixer);
        e.apply(&mut p, 1, 1, 0);
        assert_eq!(p[3], 128, "alpha is preserved");
    }

    #[test]
    fn color_temperature_warm_is_more_red_than_cool() {
        // A neutral grey pixel: warming should boost red, cooling should boost blue.
        let grey = || vec![128u8; 4 * 32 * 32];
        let mut warm = grey();
        let mut cool = grey();
        let mut e = Effect::new(EffectKind::ColorTemperature);
        e.params[0] = 100.0; // warm (3200K)
        e.apply(&mut warm, 32, 32, 0);
        let mut e = Effect::new(EffectKind::ColorTemperature);
        e.params[0] = -100.0; // cool (10000K)
        e.apply(&mut cool, 32, 32, 0);
        assert!(warm[0] > cool[0], "warm has more red: {} vs {}", warm[0], cool[0]);
        assert!(warm[2] < cool[2], "warm has less blue: {} vs {}", warm[2], cool[2]);
    }

    #[test]
    fn color_temperature_neutral_is_near_identity() {
        // At shift=0 (6600K), the blackbody color is near-white (≈0.95×, 0.97×, 1.0×),
        // so a grey pixel should change but only slightly.
        let mut p = vec![128u8; 4 * 8 * 8];
        let before = p.clone();
        Effect::new(EffectKind::ColorTemperature).apply(&mut p, 8, 8, 0);
        for (i, (&v, &b)) in p.iter().zip(&before).enumerate() {
            assert!(
                (i32::from(v) - i32::from(b)).abs() < 10,
                "pixel {i}: {v} vs {b} — too much change at neutral"
            );
        }
    }

    #[test]
    fn directional_blur_blurs_along_the_axis() {
        // A sharp edge: left half black, right half white. A horizontal blur (angle=0)
        // should soften the edge; a vertical blur (angle=90) should not.
        let mut horizontal = halves();
        let mut vertical = halves();
        let mut e = Effect::new(EffectKind::DirectionalBlur);
        e.amount = 100.0;
        e.params[0] = 0.0; // horizontal
        e.apply(&mut horizontal, 32, 32, 0);
        e.params[0] = 90.0; // vertical
        e.apply(&mut vertical, 32, 32, 0);
        // Horizontal blur softens the vertical edge.
        assert!(
            at(&horizontal, 15, 16) > 0 && at(&horizontal, 16, 16) < 255,
            "horizontal blur softens the edge"
        );
        // Vertical blur does not soften the vertical edge (all pixels in a row are the same).
        let mut unchanged = true;
        for x in 0..16 {
            if at(&vertical, x, 16) != 0 {
                unchanged = false;
            }
        }
        assert!(unchanged, "vertical blur does not soften horizontal edge");
    }

    #[test]
    fn directional_blur_keeps_alpha() {
        let mut p = vec![200u8, 100, 50, 128];
        let mut e = Effect::new(EffectKind::DirectionalBlur);
        e.amount = 100.0;
        e.params[0] = 0.0;
        e.apply(&mut p, 1, 1, 0);
        assert_eq!(p[3], 128, "alpha is preserved");
    }

    #[test]
    fn directional_blur_amount_zero_is_noop() {
        let mut p = halves();
        let before = p.clone();
        let e = Effect { amount: 0.0, kind: EffectKind::DirectionalBlur, ..Effect::new(EffectKind::DirectionalBlur) };
        e.apply(&mut p, 32, 32, 0);
        assert_eq!(p, before, "amount=0 leaves pixels unchanged");
    }

    #[test]
    fn diffusion_glow_is_brightened() {
        // A dark pixel should get brighter from diffusion (blurred duplicate + screen blend).
        let mut p = vec![50u8; 4 * 32 * 32];
        Effect { amount: 100.0, kind: EffectKind::Diffusion, ..Effect::new(EffectKind::Diffusion) }
            .apply(&mut p, 32, 32, 0);
        assert!(p[0] > 50, "diffusion brightens dark pixels");
    }

    #[test]
    fn diffusion_keeps_alpha() {
        let mut p = vec![200u8, 100, 50, 128];
        let mut e = Effect::new(EffectKind::Diffusion);
        e.amount = 100.0;
        e.apply(&mut p, 1, 1, 0);
        assert_eq!(p[3], 128, "alpha is preserved");
    }

    #[test]
    fn diffusion_amount_zero_is_noop() {
        let mut p = halves();
        let before = p.clone();
        let e = Effect { amount: 0.0, kind: EffectKind::Diffusion, ..Effect::new(EffectKind::Diffusion) };
        e.apply(&mut p, 32, 32, 0);
        assert_eq!(p, before, "amount=0 leaves pixels unchanged");
    }

    #[test]
    fn all_nine_effect_kinds_have_labels() {
        for kind in EffectKind::ALL {
            assert!(!kind.label().is_empty(), "{:?} has a label", kind);
        }
    }

    #[test]
    fn rgb_mixer_sanitizes_params_to_0_2() {
        let e = Effect {
            kind: EffectKind::RgbMixer,
            amount: 50.0,
            params: [3.0, -1.0, f32::NAN, 0.5, 1.0, 2.0, 0.0, f32::INFINITY, 0.5],
            ..Effect::new(EffectKind::RgbMixer)
        };
        let s = e.sanitized();
        assert!(s.params[0] == 2.0, "clamped to max");
        assert!(s.params[1] == 0.0, "clamped to min");
        assert!(s.params[2] == 0.0, "NaN becomes 0");
        assert!(s.params[7] == 0.0, "Inf becomes 0");
    }

    #[test]
    fn color_temperature_sanitizes_angle_range() {
        let e = Effect {
            kind: EffectKind::ColorTemperature,
            amount: 50.0,
            params: [150.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..Effect::new(EffectKind::ColorTemperature)
        };
        let s = e.sanitized();
        assert!(s.params[0] == 100.0, "clamped to +100");
    }

    #[test]
    fn directional_blur_sanitizes_angle() {
        let e = Effect {
            kind: EffectKind::DirectionalBlur,
            amount: 50.0,
            params: [720.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..Effect::new(EffectKind::DirectionalBlur)
        };
        let s = e.sanitized();
        assert_eq!(s.params[0] % 360.0, 0.0, "720 wraps to 0");

        let e = Effect {
            kind: EffectKind::DirectionalBlur,
            amount: 50.0,
            params: [-90.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..Effect::new(EffectKind::DirectionalBlur)
        };
        let s = e.sanitized();
        assert_eq!(s.params[0], 270.0, "-90 wraps to 270");
    }
}
