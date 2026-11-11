//! More video effects: colour (Exposure, Posterize, Vibrance, Duotone, Luma Key, Flicker),
//! distortions (Twirl, Bulge, Pinch, Kaleidoscope, Mirror Split, Tile, Camera Shake) and looks
//! (Scanlines, Halftone, Hexagon and Dot Pixelate, Pencil Sketch, Abstraction, Prism).
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100, that drives
//! its main setting; the rest sit at their usual values. Sizes are in thousandths of 1080 lines
//! of the picture's height, so an effect looks the same at any resolution. The time-based ones
//! (Flicker, Camera Shake) follow the clip's frame rate (24 frames a second when it is not known).

use crate::fx::{gaussian, EffectKind};
use crate::LUMA;

/// A picture to read from: RGBA8 with straight alpha, sampled with premultiplied alpha.
pub(crate) struct Src<'a> {
    data: &'a [u8],
    w: usize,
    h: usize,
}

impl<'a> Src<'a> {
    pub(crate) fn new(data: &'a [u8], w: usize, h: usize) -> Self {
        Self { data, w, h }
    }

    pub(crate) fn texel(&self, x: usize, y: usize) -> [f32; 4] {
        let i = (y * self.w + x) * 4;
        let a = f32::from(self.data[i + 3]) / 255.0;
        [
            f32::from(self.data[i]) / 255.0 * a,
            f32::from(self.data[i + 1]) / 255.0 * a,
            f32::from(self.data[i + 2]) / 255.0 * a,
            a,
        ]
    }

    /// Bilinear value at (x, y) in pixels, texel centres at +0.5, the edge held outside.
    pub(crate) fn at(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = (x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = (y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let mix4 =
            |a: [f32; 4], b: [f32; 4], t: f32| [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t);
        let top = mix4(self.texel(x0, y0), self.texel(x1, y0), tx);
        let bottom = mix4(self.texel(x0, y1), self.texel(x1, y1), tx);
        mix4(top, bottom, ty)
    }
}

/// Writes a premultiplied colour into a straight RGBA8 pixel.
pub(crate) fn put(px: &mut [u8], c: [f32; 4]) {
    let a = c[3].clamp(0.0, 1.0);
    let k = if a > 1e-6 { 1.0 / a } else { 0.0 };
    for i in 0..3 {
        px[i] = (c[i] * k * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    px[3] = (a * 255.0 + 0.5) as u8;
}

/// The straight colour of a pixel, 0 to 1.
fn rgb_of(px: &[u8]) -> [f32; 3] {
    [0, 1, 2].map(|i| f32::from(px[i]) / 255.0)
}

fn set_rgb(px: &mut [u8], c: [f32; 3]) {
    for i in 0..3 {
        px[i] = (c[i] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
}

fn luma(c: [f32; 3]) -> f32 {
    c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// GLSL `mod` for a positive period.
fn modulo(x: f32, period: f32) -> f32 {
    x - period * (x / period).floor()
}

/// Calls `f(x, y, pixel)` for every pixel, in bands of rows on every core.
pub(crate) fn par_pixels(
    rgba: &mut [u8],
    w: usize,
    h: usize,
    f: &(dyn Fn(usize, usize, &mut [u8]) + Sync),
) {
    let run = |first_row: usize, band: &mut [u8]| {
        for (i, px) in band.chunks_exact_mut(4).enumerate() {
            let n = first_row * w + i;
            f(n % w, n / w, px);
        }
    };
    if w * h < 16_384 {
        run(0, rgba);
        return;
    }
    crate::par::par_rows(rgba, w * 4, h, &|first_row, band| run(first_row, band));
}

/// Moves pixels: `source(x, y)` says where in the original the pixel at (x, y) (centre at +0.5)
/// comes from, or `None` for transparent.
pub(crate) fn remap(
    rgba: &mut [u8],
    w: usize,
    h: usize,
    source: &(dyn Fn(f32, f32) -> Option<(f32, f32)> + Sync),
) {
    let original = rgba.to_vec();
    let src = Src {
        data: &original,
        w,
        h,
    };
    par_pixels(
        rgba,
        w,
        h,
        &|x, y, px| match source(x as f32 + 0.5, y as f32 + 0.5) {
            Some((sx, sy)) => put(px, src.at(sx, sy)),
            None => px.copy_from_slice(&[0, 0, 0, 0]),
        },
    );
}

/// Folds a position back into 0..size like a mirror (edges repeat mirrored).
fn mirror_into(v: f32, size: f32) -> f32 {
    let t = v / size;
    (1.0 - ((t % 2.0 + 2.0) % 2.0 - 1.0).abs()) * size
}

// ---- colour ----

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = modulo(h, 360.0) / 60.0;
    let x = c * (1.0 - (modulo(hp, 2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

fn rgb_to_hue_sat(c: [f32; 3]) -> (f32, f32) {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let l = 0.5 * (mx + mn);
    let s = if d <= 1e-6 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6)
    };
    let hue = if d <= 1e-6 {
        0.0
    } else if mx == c[0] {
        modulo((c[1] - c[2]) / d + 6.0, 6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    (hue * 60.0, s)
}

fn pointwise(rgba: &mut [u8], f: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
    let run = |band: &mut [u8]| {
        for px in band.chunks_exact_mut(4) {
            let out = f(rgb_of(px));
            set_rgb(px, out);
        }
    };
    if rgba.len() < 65_536 {
        run(rgba);
        return;
    }
    let pixels = rgba.len() / 4;
    crate::par::par_rows(rgba, 4, pixels, &|_, band| run(band));
}

fn exposure(rgba: &mut [u8], a: f32) {
    // Up to three stops brighter, worked out in linear light (gamma 2.2), by a table.
    let gain = (a * 3.0).exp2();
    let table: Vec<u8> = (0..=255u32)
        .map(|v| {
            let lin = (v as f32 / 255.0).powf(2.2) * gain;
            (lin.powf(1.0 / 2.2).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        })
        .collect();
    for px in rgba.chunks_exact_mut(4) {
        for v in &mut px[..3] {
            *v = table[usize::from(*v)];
        }
    }
}

fn posterize(rgba: &mut [u8], a: f32) {
    let n = (16.0 - a * 14.0).round().clamp(2.0, 16.0);
    pointwise(rgba, |c| {
        c.map(|v| ((v.clamp(0.0, 1.0) * n).floor().min(n - 1.0)) / (n - 1.0))
    });
}

fn vibrance(rgba: &mut [u8], a: f32) {
    // Positive vibrance boosts muted colours most and skin tones half as much.
    const SKIN_HUE: f32 = 25.0;
    const SKIN_HALF_WIDTH: f32 = 25.0;
    pointwise(rgba, |c| {
        let y = luma(c);
        let sat = c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
        let (hue, _) = rgb_to_hue_sat(c);
        let dh = ((hue - SKIN_HUE + 540.0) % 360.0 - 180.0).abs();
        let skin = 1.0 - smoothstep(0.0, SKIN_HALF_WIDTH, dh);
        let k = 1.0 + a * (1.0 - sat) * (1.0 - 0.5 * skin);
        c.map(|v| (y + (v - y) * k).clamp(0.0, 1.0))
    });
}

fn duotone(rgba: &mut [u8], a: f32) {
    let (shadow, highlight) = ([0.07, 0.05, 0.30], [1.0, 0.82, 0.5]);
    pointwise(rgba, |c| {
        let y = luma(c).clamp(0.0, 1.0);
        let tone = [0, 1, 2].map(|i| mix(shadow[i], highlight[i], y));
        [0, 1, 2].map(|i| mix(c[i], tone[i], a))
    });
}

fn luma_key(rgba: &mut [u8], a: f32) {
    // The picture is kept where it is brighter than a threshold (softly).
    let (low, soft) = (a * 0.9, 0.1);
    for px in rgba.chunks_exact_mut(4) {
        let y = luma(rgb_of(px));
        let matte = smoothstep(low - soft, low, y);
        px[3] = (f32::from(px[3]) * matte).round() as u8;
    }
}

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// Smooth noise from -1 to 1 along time.
fn value_noise(t: f32, seed: u32) -> f32 {
    let fl = t.floor();
    let i = fl as i64;
    let f = t - fl;
    let f = f * f * (3.0 - 2.0 * f);
    let lattice = |n: i64| {
        hash32((n as u32).wrapping_mul(0x9e37_79b9) ^ hash32(seed.wrapping_add(0x51))) as f32
            / u32::MAX as f32
            * 2.0
            - 1.0
    };
    let (a, b) = (lattice(i), lattice(i + 1));
    a + (b - a) * f
}

fn flicker(rgba: &mut [u8], a: f32, frame: u64, rate: f32) {
    let t = frame as f32 / rate;
    let (freq, random) = (8.0, 0.7);
    let wave = (std::f32::consts::TAU * freq * t).sin();
    let noise = value_noise(freq * t, 1);
    let gain = (1.0 + a * 0.5 * (wave * (1.0 - random) + noise * random)).max(0.0);
    pointwise(rgba, |c| c.map(|v| (v * gain).clamp(0.0, 1.0)));
}

// ---- distortions ----

fn twirl(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let radius = (0.28 * h as f32).max(1.0);
    let angle = a * std::f32::consts::TAU;
    remap(rgba, w, h, &move |x, y| {
        let (px, py) = (x - cx, y - cy);
        let r = px.hypot(py);
        if r >= radius {
            return Some((x, y));
        }
        let t = r / radius;
        let ang = angle * (1.0 - t) * (1.0 - t);
        let (s, c) = ang.sin_cos();
        Some((cx + c * px - s * py, cy + s * px + c * py))
    });
}

fn bulge(rgba: &mut [u8], w: usize, h: usize, exponent: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let radius = (0.28 * h as f32).max(1.0);
    remap(rgba, w, h, &move |x, y| {
        let (px, py) = (x - cx, y - cy);
        let r = px.hypot(py);
        if r >= radius {
            return Some((x, y));
        }
        let k = (r / radius).max(1e-4).powf(exponent);
        Some((cx + px * k, cy + py * k))
    });
}

fn kaleidoscope(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let segments = (2.0 + (a * 10.0).round()).max(2.0);
    let segment = std::f32::consts::TAU / segments;
    let (wf, hf) = (w as f32, h as f32);
    remap(rgba, w, h, &move |x, y| {
        let (px, py) = (x - cx, y - cy);
        let r = px.hypot(py);
        let mut ang = modulo(py.atan2(px), segment);
        if ang > 0.5 * segment {
            ang = segment - ang; // every other copy mirrored: seamless edges
        }
        let (s, c) = ang.sin_cos();
        Some((mirror_into(cx + r * c, wf), mirror_into(cy + r * s, hf)))
    });
}

fn mirror_split(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    // The left of the line is mirrored onto the right.
    let line = a.clamp(0.02, 0.98) * w as f32;
    remap(rgba, w, h, &move |x, y| {
        Some((if x > line { 2.0 * line - x } else { x }, y))
    });
}

fn tile(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let zoom = (100.0 - 0.9 * a * 100.0).clamp(1.0, 100.0);
    let inv = 100.0 / zoom;
    let (wf, hf) = (w as f32, h as f32);
    remap(rgba, w, h, &move |x, y| {
        let u = 0.5 + (x / wf - 0.5) * inv;
        let v = 0.5 + (y / hf - 0.5) * inv;
        Some((fract(u) * wf, fract(v) * hf))
    });
}

fn camera_shake(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64, rate: f32) {
    const SAMPLES: usize = 8;
    let (wf, hf) = (w as f32, h as f32);
    let (cx, cy) = (wf / 2.0, hf / 2.0);
    let amplitude = 12.0 * (hf / 1080.0) * (a * 2.0);
    let rotation = 1.0f32.to_radians() * (a * 2.0);
    let inv_zoom = 100.0 / 105.0;
    let sway = |t: f32, seed: u32| {
        0.7 * value_noise(t, seed) + 0.3 * value_noise(2.3 * t + 17.0, seed + 101)
    };
    let shakes: Vec<[f32; 3]> = (0..SAMPLES)
        .map(|i| {
            // Sub-frames over half a frame (a 180 degree shutter), centred on the frame.
            let sub = (i as f32 / (SAMPLES - 1) as f32 - 0.5) * 0.5;
            let t = (frame as f32 + sub) / rate * 4.0;
            [
                amplitude * sway(t, 3),
                amplitude * sway(t, 4),
                rotation * sway(t, 5),
            ]
        })
        .collect();
    let original = rgba.to_vec();
    let src = Src {
        data: &original,
        w,
        h,
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (qx, qy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let mut sum = [0.0f32; 4];
        for s in &shakes {
            let (dx, dy) = (qx - s[0], qy - s[1]);
            let (sn, c) = (-s[2]).sin_cos();
            let sx = (c * dx - sn * dy) * inv_zoom + cx;
            let sy = (sn * dx + c * dy) * inv_zoom + cy;
            let v = src.at(mirror_into(sx, wf), mirror_into(sy, hf));
            for i in 0..4 {
                sum[i] += v[i];
            }
        }
        put(px, sum.map(|v| v / SAMPLES as f32));
    });
}

// ---- looks ----

fn scanlines(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let spacing = (4.0 * h as f32 / 1080.0).max(2.0);
    let thickness = 0.5;
    par_pixels(rgba, w, h, &|_, y, px| {
        let s = y as f32 + 0.5;
        let phase = fract(s / spacing) * spacing;
        let width = thickness * spacing;
        let line =
            smoothstep(-0.5, 0.5, phase) * (1.0 - smoothstep(width - 0.5, width + 0.5, phase));
        for v in &mut px[..3] {
            *v = (f32::from(*v) * (1.0 - a * line)).round() as u8;
        }
    });
}

fn halftone(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let cell = (8.0 * h as f32 / 1080.0).max(1.5);
    let (sn, cs) = 45f32.to_radians().sin_cos();
    let (ox, oy) = (w as f32 / 2.0, h as f32 / 2.0);
    let original = rgba.to_vec();
    let src = Src {
        data: &original,
        w,
        h,
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (rx, ry) = (x as f32 + 0.5 - ox, y as f32 + 0.5 - oy);
        let (qx, qy) = ((cs * rx + sn * ry) / cell, (-sn * rx + cs * ry) / cell);
        let (cqx, cqy) = (qx.floor() + 0.5, qy.floor() + 0.5);
        let (gx, gy) = (cqx * cell, cqy * cell);
        let centre = (cs * gx - sn * gy + ox, sn * gx + cs * gy + oy);
        let s = src.at(
            centre.0.clamp(0.5, w as f32 - 0.5),
            centre.1.clamp(0.5, h as f32 - 0.5),
        );
        let straight = if s[3] > 1e-6 {
            [s[0] / s[3], s[1] / s[3], s[2] / s[3]]
        } else {
            [0.0; 3]
        };
        let ink = 1.0 - luma(straight).clamp(0.0, 1.0);
        let radius = ink.sqrt() * std::f32::consts::FRAC_1_SQRT_2 * cell;
        let d = (qx - cqx).hypot(qy - cqy) * cell;
        let dot = 1.0 - smoothstep(radius - 0.5, radius + 0.5, d);
        let orig = rgb_of(px);
        set_rgb(px, orig.map(|v| mix(v, 1.0 - dot, a)));
    });
}

fn pixelate_hexagons(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let cell = ((4.0 + a * 60.0) * h as f32 / 1080.0).max(2.0);
    let (ox, oy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (rx, ry) = (1.0f32, 1.732_050_8f32);
    remap(rgba, w, h, &move |x, y| {
        let (px, py) = ((x - ox) / cell, (y - oy) / cell);
        let (ax, ay) = (modulo(px, rx) - 0.5 * rx, modulo(py, ry) - 0.5 * ry);
        let (bx, by) = (
            modulo(px - 0.5 * rx, rx) - 0.5 * rx,
            modulo(py - 0.5 * ry, ry) - 0.5 * ry,
        );
        let (gx, gy) = if ax * ax + ay * ay < bx * bx + by * by {
            (ax, ay)
        } else {
            (bx, by)
        };
        Some(((px - gx) * cell + ox, (py - gy) * cell + oy))
    });
}

fn pixelate_dots(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let cell = ((4.0 + a * 60.0) * h as f32 / 1080.0).max(2.0);
    let (ox, oy) = (w as f32 / 2.0, h as f32 / 2.0);
    let original = rgba.to_vec();
    let src = Src {
        data: &original,
        w,
        h,
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (qx, qy) = ((x as f32 + 0.5 - ox) / cell, (y as f32 + 0.5 - oy) / cell);
        let (cqx, cqy) = (qx.floor() + 0.5, qy.floor() + 0.5);
        let c = src.at(
            (cqx * cell + ox).clamp(0.5, w as f32 - 0.5),
            (cqy * cell + oy).clamp(0.5, h as f32 - 0.5),
        );
        // Round dots, antialiased over one pixel, on black.
        let d = (qx - cqx).hypot(qy - cqy) * cell;
        let cover = 1.0 - smoothstep(0.47 * cell - 0.5, 0.47 * cell + 0.5, d);
        put(px, [c[0] * cover, c[1] * cover, c[2] * cover, c[3]]);
    });
}

fn pencil_sketch(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let mut blurred = rgba.to_vec();
    gaussian(
        &mut blurred,
        w,
        h,
        (8.0 * h as f32 / 1080.0).round().max(2.0) as usize,
    );
    for (px, b) in rgba.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
        let c = rgb_of(px);
        let (g, bl) = (luma(c), luma(rgb_of(b)));
        // Colour dodge of the grey picture with its blurred inverse: flat areas become paper
        // white, edges lines. Strength 100 % is three times the plain dodge's darkness.
        let sketch = (g / bl.max(1e-3)).clamp(0.0, 1.0);
        let sketch = (1.0 - (1.0 - sketch) * 3.0).clamp(0.0, 1.0);
        set_rgb(px, c.map(|v| mix(v, sketch, a)));
    }
}

fn abstraction(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    // A Kuwahara filter: the mean of the quadrant around the pixel with the least variance, so
    // flat regions become smooth patches and edges stay sharp; colours in eight steps; dark
    // outlines where the picture changes quickly.
    let radius = ((1.0 + a * 7.0) * h as f32 / 1080.0).max(1.0);
    let step = radius / 3.0;
    let levels = 8.0f32;
    let original = rgba.to_vec();
    let src = Src {
        data: &original,
        w,
        h,
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut best = src.at(fx, fy);
        let mut best_var = 1e9f32;
        for q in 0..4 {
            let dir = (
                if q == 0 || q == 3 { -1.0 } else { 1.0 },
                if q < 2 { -1.0 } else { 1.0 },
            );
            let mut sum = [0.0f32; 4];
            let mut sum2 = 0.0;
            for j in 0..4 {
                for i in 0..4 {
                    let c = src.at(fx + dir.0 * i as f32 * step, fy + dir.1 * j as f32 * step);
                    let l = luma([c[0], c[1], c[2]]);
                    for k in 0..4 {
                        sum[k] += c[k];
                    }
                    sum2 += l * l;
                }
            }
            let mean = sum.map(|v| v / 16.0);
            let m = luma([mean[0], mean[1], mean[2]]);
            let var = sum2 / 16.0 - m * m;
            if var < best_var {
                best_var = var;
                best = mean;
            }
        }
        if best[3] <= 0.0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        let straight = |c: [f32; 4]| {
            if c[3] > 1e-6 {
                luma([c[0] / c[3], c[1] / c[3], c[2] / c[3]])
            } else {
                0.0
            }
        };
        let l = straight(src.at(fx + 1.0, fy)) - straight(src.at(fx - 1.0, fy));
        let m = straight(src.at(fx, fy + 1.0)) - straight(src.at(fx, fy - 1.0));
        let edge = smoothstep(0.08, 0.3, l.hypot(m));
        let k = 1.0 - 0.4 * edge;
        let c = [0, 1, 2].map(|i| {
            let v = (best[i] / best[3]).clamp(0.0, 1.0);
            (v * levels).floor().min(levels - 1.0) / (levels - 1.0) * k
        });
        put(
            px,
            [c[0] * best[3], c[1] * best[3], c[2] * best[3], best[3]],
        );
    });
}

fn prism(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    const TAPS: usize = 16;
    let spread = a * 24.0 * h as f32 / 1080.0;
    if spread < 0.5 {
        return;
    }
    // Spectral weights: tap t gets the colour of hue 0 (red) to 270 degrees (violet),
    // normalised so a uniform picture keeps its colour.
    let mut weights = [[0.0f32; 3]; TAPS];
    let mut total = [0.0f32; 3];
    for (i, wt) in weights.iter_mut().enumerate() {
        *wt = hsl_to_rgb(270.0 * (i as f32 + 0.5) / TAPS as f32, 1.0, 0.5);
        for k in 0..3 {
            total[k] += wt[k];
        }
    }
    for wt in &mut weights {
        for k in 0..3 {
            wt[k] /= total[k];
        }
    }
    let original = rgba.to_vec();
    let src = Src {
        data: &original,
        w,
        h,
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut rgb = [0.0f32; 3];
        let mut alpha = 0.0;
        for (i, wt) in weights.iter().enumerate() {
            let t = (i as f32 + 0.5) / TAPS as f32 - 0.5;
            let c = src.at(fx + spread * t, fy);
            for k in 0..3 {
                rgb[k] += c[k] * wt[k];
            }
            alpha += c[3];
        }
        alpha /= TAPS as f32;
        put(
            px,
            [
                rgb[0].min(alpha),
                rgb[1].min(alpha),
                rgb[2].min(alpha),
                alpha,
            ],
        );
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture of frame `frame`.
pub(crate) fn apply(
    kind: EffectKind,
    a: f32,
    rgba: &mut [u8],
    w: usize,
    h: usize,
    frame: u64,
    rate: f32,
) {
    match kind {
        EffectKind::Exposure => exposure(rgba, a),
        EffectKind::Posterize => posterize(rgba, a),
        EffectKind::Vibrance => vibrance(rgba, a),
        EffectKind::Duotone => duotone(rgba, a),
        EffectKind::LumaKey => luma_key(rgba, a),
        EffectKind::Flicker => flicker(rgba, a, frame, rate),
        EffectKind::Twirl => twirl(rgba, w, h, a),
        EffectKind::Bulge => bulge(rgba, w, h, 0.7 * a),
        EffectKind::Pinch => bulge(rgba, w, h, -0.7 * a),
        EffectKind::Kaleidoscope => kaleidoscope(rgba, w, h, a),
        EffectKind::MirrorSplit => mirror_split(rgba, w, h, a),
        EffectKind::Tile => tile(rgba, w, h, a),
        EffectKind::CameraShake => camera_shake(rgba, w, h, a, frame, rate),
        EffectKind::Scanlines => scanlines(rgba, w, h, a),
        EffectKind::Halftone => halftone(rgba, w, h, a),
        EffectKind::HexagonPixelate => pixelate_hexagons(rgba, w, h, a),
        EffectKind::DotPixelate => pixelate_dots(rgba, w, h, a),
        EffectKind::PencilSketch => pencil_sketch(rgba, w, h, a),
        EffectKind::Abstraction => abstraction(rgba, w, h, a),
        EffectKind::Prism => prism(rgba, w, h, a),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 64;
    const H: usize = 48;

    /// A picture with a different colour almost everywhere, opaque.
    fn picture() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                v.extend_from_slice(&[(x * 4) as u8, (y * 5) as u8, ((x + y) * 2) as u8, 255]);
            }
        }
        v
    }

    fn flat(rgb: [u8; 3]) -> Vec<u8> {
        (0..W * H)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    fn run(kind: EffectKind, amount: f32, pic: &[u8], frame: u64) -> Vec<u8> {
        let mut out = pic.to_vec();
        Effect {
            amount,
            ..Effect::new(kind)
        }
        .apply(&mut out, W, H, frame);
        out
    }

    fn px(p: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * W + x) * 4;
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    }

    const NEW: [EffectKind; 20] = [
        EffectKind::Exposure,
        EffectKind::Posterize,
        EffectKind::Vibrance,
        EffectKind::Duotone,
        EffectKind::LumaKey,
        EffectKind::Flicker,
        EffectKind::Twirl,
        EffectKind::Bulge,
        EffectKind::Pinch,
        EffectKind::Kaleidoscope,
        EffectKind::MirrorSplit,
        EffectKind::Tile,
        EffectKind::CameraShake,
        EffectKind::Scanlines,
        EffectKind::Halftone,
        EffectKind::HexagonPixelate,
        EffectKind::DotPixelate,
        EffectKind::PencilSketch,
        EffectKind::Abstraction,
        EffectKind::Prism,
    ];

    #[test]
    fn every_new_effect_changes_a_picture_keeps_its_size_and_is_off_at_zero() {
        let pic = picture();
        for kind in NEW {
            let out = run(kind, Effect::new(kind).amount, &pic, 7);
            assert_eq!(out.len(), pic.len(), "{kind:?}");
            assert_ne!(out, pic, "{kind:?} does nothing at its starting strength");
            assert_eq!(run(kind, 0.0, &pic, 7), pic, "{kind:?} at 0");
        }
    }

    #[test]
    fn every_new_effect_survives_tiny_pictures_and_any_strength() {
        for kind in NEW {
            for (w, h) in [(1usize, 1usize), (2, 1), (1, 3), (5, 5)] {
                for amount in [1.0, 50.0, 100.0] {
                    let mut p: Vec<u8> = (0..w * h * 4).map(|i| (i * 37) as u8).collect();
                    Effect {
                        amount,
                        ..Effect::new(kind)
                    }
                    .apply(&mut p, w, h, 3);
                }
            }
            let mut empty: Vec<u8> = Vec::new();
            Effect::new(kind).apply(&mut empty, 0, 0, 0);
        }
    }

    #[test]
    fn the_effects_are_the_same_every_time_for_the_same_frame() {
        let pic = picture();
        for kind in NEW {
            assert_eq!(
                run(kind, 60.0, &pic, 5),
                run(kind, 60.0, &pic, 5),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn exposure_brightens_and_keeps_black() {
        let out = run(EffectKind::Exposure, 50.0, &flat([60, 60, 60]), 0);
        assert!(px(&out, 3, 3)[0] > 60);
        let black = run(EffectKind::Exposure, 100.0, &flat([0, 0, 0]), 0);
        assert_eq!(px(&black, 3, 3), [0, 0, 0, 255]);
    }

    #[test]
    fn posterize_leaves_few_levels() {
        let out = run(EffectKind::Posterize, 100.0, &picture(), 0);
        let mut levels: Vec<u8> = out.chunks_exact(4).map(|p| p[0]).collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() <= 2, "{levels:?}");
    }

    #[test]
    fn vibrance_raises_a_muted_colour_more_than_a_vivid_one() {
        let sat = |p: [u8; 4]| f32::from(p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]));
        // how many times more saturated the picture got
        let factor = |rgb: [u8; 3]| {
            let pic = flat(rgb);
            sat(px(&run(EffectKind::Vibrance, 100.0, &pic, 0), 2, 2)) / sat(px(&pic, 2, 2))
        };
        let (muted, vivid) = (factor([100, 130, 110]), factor([60, 200, 90]));
        assert!(muted > 1.3 && muted > vivid, "muted {muted}, vivid {vivid}");
    }

    #[test]
    fn duotone_makes_black_and_white_the_two_colours() {
        let black = px(&run(EffectKind::Duotone, 100.0, &flat([0, 0, 0]), 0), 1, 1);
        let white = px(
            &run(EffectKind::Duotone, 100.0, &flat([255, 255, 255]), 0),
            1,
            1,
        );
        assert!(black[2] > black[0], "shadows lean blue: {black:?}");
        assert!(white[0] > white[2], "highlights lean warm: {white:?}");
    }

    #[test]
    fn luma_key_takes_out_the_dark_and_keeps_the_light() {
        assert_eq!(
            px(&run(EffectKind::LumaKey, 50.0, &flat([0, 0, 0]), 0), 1, 1)[3],
            0
        );
        assert_eq!(
            px(
                &run(EffectKind::LumaKey, 50.0, &flat([255, 255, 255]), 0),
                1,
                1
            )[3],
            255
        );
    }

    #[test]
    fn flicker_changes_the_level_from_frame_to_frame() {
        let pic = flat([128, 128, 128]);
        let levels: Vec<u8> = (0..12)
            .map(|f| px(&run(EffectKind::Flicker, 80.0, &pic, f), 0, 0)[0])
            .collect();
        assert!(levels.iter().max() != levels.iter().min(), "{levels:?}");
    }

    #[test]
    fn twirl_leaves_the_centre_and_the_far_corners_and_turns_between() {
        let pic = picture();
        let out = run(EffectKind::Twirl, 50.0, &pic, 0);
        assert_eq!(px(&out, 0, 0), px(&pic, 0, 0));
        assert_eq!(px(&out, W - 1, H - 1), px(&pic, W - 1, H - 1));
        let moved = (0..W)
            .filter(|&x| px(&out, x, H / 2 + 6) != px(&pic, x, H / 2 + 6))
            .count();
        assert!(moved > 4, "{moved}");
    }

    #[test]
    fn bulge_magnifies_the_middle_and_pinch_shrinks_it() {
        // a bright dot on dark; the dot's width in the middle row
        let mut pic = flat([0, 0, 0]);
        for y in 0..H {
            for x in 0..W {
                if (x as i32 - 32).abs() < 4 && (y as i32 - 24).abs() < 4 {
                    pic[(y * W + x) * 4..(y * W + x) * 4 + 3].copy_from_slice(&[255, 255, 255]);
                }
            }
        }
        let width = |p: &[u8]| (0..W).filter(|&x| px(p, x, 24)[0] > 128).count();
        assert!(width(&run(EffectKind::Bulge, 100.0, &pic, 0)) > width(&pic));
        assert!(width(&run(EffectKind::Pinch, 100.0, &pic, 0)) < width(&pic));
    }

    #[test]
    fn mirror_split_copies_the_left_onto_the_right() {
        let out = run(EffectKind::MirrorSplit, 50.0, &picture(), 0);
        for x in 2..30 {
            let (l, r) = (px(&out, 32 - x, 10), px(&out, 31 + x, 10));
            assert!(
                l.iter().zip(&r).all(|(a, b)| a.abs_diff(*b) <= 8),
                "x {x}: {l:?} {r:?}"
            );
        }
    }

    #[test]
    fn tile_repeats_the_picture() {
        let out = run(EffectKind::Tile, 50.0, &picture(), 0);
        // zoom 55 %: a period of 55 % of the width, 35 pixels here
        let (a, b) = (px(&out, 5, 6), px(&out, 5, 6 + 26));
        assert!(
            a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 40),
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn camera_shake_moves_by_frame_and_stays_opaque() {
        let pic = picture();
        let (a, b) = (
            run(EffectKind::CameraShake, 100.0, &pic, 3),
            run(EffectKind::CameraShake, 100.0, &pic, 40),
        );
        assert_ne!(a, b);
        assert!(a.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn scanlines_darken_some_rows_only() {
        let out = run(EffectKind::Scanlines, 100.0, &flat([200, 200, 200]), 0);
        let rows: Vec<u8> = (0..H).map(|y| px(&out, 4, y)[0]).collect();
        assert!(
            rows.iter().any(|&v| v < 60) && rows.contains(&200),
            "{rows:?}"
        );
    }

    #[test]
    fn halftone_is_black_and_white_dots() {
        let out = run(EffectKind::Halftone, 100.0, &picture(), 0);
        assert!(out.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2]));
        assert!(out.chunks_exact(4).any(|p| p[0] < 40) && out.chunks_exact(4).any(|p| p[0] > 215));
    }

    #[test]
    fn pixelating_makes_flat_patches_and_leaves_a_flat_picture_alone() {
        let pic = picture();
        let unique = |p: &[u8]| {
            let mut v: Vec<[u8; 4]> = p
                .chunks_exact(4)
                .map(|c| [c[0], c[1], c[2], c[3]])
                .collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(unique(&run(EffectKind::HexagonPixelate, 40.0, &pic, 0)) < unique(&pic) / 2);
        let still = flat([90, 120, 200]);
        let hex = run(EffectKind::HexagonPixelate, 40.0, &still, 0);
        assert!(hex.chunks_exact(4).all(|p| p[..3] == [90, 120, 200]));
    }

    #[test]
    fn pencil_sketch_turns_flat_areas_to_paper_white_and_keeps_alpha() {
        let out = run(EffectKind::PencilSketch, 100.0, &flat([40, 90, 160]), 0);
        assert!(out.chunks_exact(4).all(|p| p[0] > 240 && p[3] == 255));
    }

    #[test]
    fn abstraction_and_prism_leave_a_flat_picture_alone() {
        let still = flat([90, 120, 200]);
        for kind in [EffectKind::Abstraction, EffectKind::Prism] {
            let out = run(kind, 60.0, &still, 0);
            let off = out
                .iter()
                .zip(&still)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            // abstraction steps the colours to eight levels; prism keeps them
            assert!(
                off <= if kind == EffectKind::Abstraction {
                    20
                } else {
                    3
                },
                "{kind:?} {off}"
            );
        }
    }

    /// Not a check, a measurement: `cargo test -p picasso-color --release timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_of_each_at_1080p() {
        let (w, h) = (1920, 1080);
        let pic: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        for kind in NEW {
            let mut p = pic.clone();
            let start = std::time::Instant::now();
            Effect::new(kind).apply(&mut p, w, h, 3);
            eprintln!(
                "{:>18}: {:>4} ms",
                kind.label(),
                start.elapsed().as_millis()
            );
        }
    }

    #[test]
    fn transparent_pictures_stay_transparent() {
        let clear = vec![0u8; W * H * 4];
        for kind in NEW {
            let out = run(kind, 60.0, &clear, 2);
            assert!(out.chunks_exact(4).all(|p| p[3] == 0), "{kind:?}");
        }
    }

    #[test]
    fn the_clips_rate_decides_what_time_a_frame_is() {
        let pic = picture();
        for kind in [
            EffectKind::Flicker,
            EffectKind::CameraShake,
            EffectKind::Ripple,
            EffectKind::Wave,
            EffectKind::VerticalWave,
        ] {
            let effect = Effect {
                amount: 80.0,
                ..Effect::new(kind)
            };
            // Half a second in, whatever the rate: frame 12 at 24, 24 at 48, 15 at 30.
            let at = |frame: u64, rate: f32| {
                let mut out = pic.clone();
                effect.apply_at(&mut out, W, H, frame, rate);
                out
            };
            // (Camera Shake blurs over a share of a frame, which is shorter at a higher rate, so
            // it is the one that is not exactly the same picture.)
            if kind != EffectKind::CameraShake {
                assert_eq!(at(12, 24.0), at(24, 48.0), "{kind:?}");
                assert_eq!(at(12, 24.0), at(15, 30.0), "{kind:?}");
            }
            // And the same frame number is a different moment at another rate.
            assert_ne!(at(12, 24.0), at(12, 60.0), "{kind:?}");
            // `apply` is the 24-a-second case.
            let mut plain = pic.clone();
            effect.apply(&mut plain, W, H, 12);
            assert_eq!(plain, at(12, 24.0), "{kind:?}");
            // A rate that makes no sense falls back to 24.
            assert_eq!(at(12, 0.0), at(12, 24.0), "{kind:?}");
            assert_eq!(at(12, f32::NAN), at(12, 24.0), "{kind:?}");
        }
    }
}
