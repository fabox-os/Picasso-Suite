//! More looks: glitches (Block, Digital, Scanline, RGB Split), film and tape (Film Burn, Light
//! Leak, Super 8, VHS and CRT), light (Star Filter, Bokeh Dream, Sparkle), painting (Oil Paint,
//! Neon Edges), distortions that move (Infinite Zoom, Shockwave, Zoom Punch) and colour (Cinematic
//! Grade, Hue Shift, Brightness, Contrast, Gamma, Saturation).
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100. The ones that
//! move follow time (`frame / rate`), so a clip looks the same at any frame rate; the ones with
//! chance use a hash of the frame, so the same frame is always drawn the same way. Sizes are in
//! thousandths of the picture's height.

use std::f32::consts::{PI, TAU};

use crate::fx::{gaussian, EffectKind};
use crate::look::{par_pixels, put, Src};

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// 0 to 1, from a number.
fn h01(x: u32) -> f32 {
    (hash32(x) >> 8) as f32 / 16_777_216.0
}

/// 0 to 1, from a place and a moment.
fn h3(x: i32, y: i32, f: u32) -> f32 {
    h01(
        (x as u32).wrapping_mul(0x9E37_79B1)
            ^ (y as u32).wrapping_mul(0x85EB_CA6B)
            ^ f.wrapping_mul(0xC2B2_AE35),
    )
}

fn unit(h: usize) -> f32 {
    h as f32 / 1080.0
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn straight(px: &[u8]) -> [f32; 3] {
    [0, 1, 2].map(|i| f32::from(px[i]) / 255.0)
}

fn set_rgb(px: &mut [u8], c: [f32; 3]) {
    for i in 0..3 {
        px[i] = (c[i] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
}

/// `f(x, y, colour)` gives the new straight colour of every pixel; the alpha stays.
fn each_colour(
    rgba: &mut [u8],
    w: usize,
    h: usize,
    f: &(dyn Fn(usize, usize, [f32; 3]) -> [f32; 3] + Sync),
) {
    par_pixels(rgba, w, h, &|x, y, px| {
        let c = f(x, y, straight(px));
        set_rgb(px, c);
    });
}

/// `f(source, x, y)` gives the new premultiplied colour of every pixel, read from a copy.
fn resample(
    rgba: &mut [u8],
    w: usize,
    h: usize,
    f: &(dyn Fn(&Src, usize, usize) -> [f32; 4] + Sync),
) {
    let copy = rgba.to_vec();
    let src = Src::new(&copy, w, h);
    par_pixels(rgba, w, h, &|x, y, px| put(px, f(&src, x, y)));
}

/// The colour at (x, y) with red read `off` pixels to the right and blue `off` to the left.
fn split_sample(src: &Src, x: f32, y: f32, off: f32) -> [f32; 4] {
    let mid = src.at(x, y);
    let r = src.at(x + off, y);
    let b = src.at(x - off, y);
    [r[0], mid[1], b[2], mid[3]]
}

fn screen(base: f32, light: f32) -> f32 {
    1.0 - (1.0 - base) * (1.0 - light.clamp(0.0, 1.0))
}

// ---- glitches --------------------------------------------------------------------------------

/// Blocks of the picture jump sideways, and change every other frame.
fn block_glitch(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let size = ((32.0 * unit(h)).round() as usize).max(4);
    let moment = (frame / 2) as u32;
    let chance = 0.05 + 0.35 * a;
    let reach = (24.0 * unit(h)).max(3.0) * (0.5 + a);
    resample(rgba, w, h, &|src, x, y| {
        let (bx, by) = ((x / size) as i32, (y / size) as i32);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        if h3(bx, by, moment) < chance {
            let dx = (h3(bx, by, moment ^ 0x5bd1) * 2.0 - 1.0) * reach;
            split_sample(src, fx + dx, fy, 2.0 * unit(h).max(0.5))
        } else {
            src.at(fx, fy)
        }
    });
}

/// Bands shift and split into red, green and blue, with a flash now and then.
fn digital_glitch(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let moment = (frame / 2) as u32;
    let band = ((18.0 * unit(h)).round() as i32).max(2);
    let split = (8.0 * unit(h)).max(1.0) * (0.5 + a);
    let reach = (40.0 * unit(h)).max(3.0) * a;
    let flash = if h01(moment.wrapping_mul(7919) ^ 0x51) < 0.08 * a {
        0.25 * a
    } else {
        0.0
    };
    resample(rgba, w, h, &|src, x, y| {
        let id = y as i32 / band;
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut out = if h3(id, 3, moment) < 0.15 + 0.4 * a {
            let dx = (h3(id, 4, moment) * 2.0 - 1.0) * reach;
            split_sample(src, fx + dx, fy, split)
        } else {
            src.at(fx, fy)
        };
        let alpha = out[3];
        for v in &mut out[..3] {
            *v = (*v + flash * alpha).min(alpha);
        }
        out
    });
}

/// Rows jump sideways and every other row is darker, like a screen that is failing.
fn scanline_glitch(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let f = frame as u32;
    let jump = (30.0 * unit(h)).max(3.0) * a;
    let shift = (4.0 * unit(h)).max(1.0) * a;
    resample(rgba, w, h, &|src, x, y| {
        let row = y as i32;
        let dx = if h3(row, 1, f) < 0.06 + 0.2 * a {
            (h3(row, 2, f) * 2.0 - 1.0) * jump
        } else {
            0.0
        };
        let mut c = split_sample(src, x as f32 + 0.5 + dx, y as f32 + 0.5, shift);
        if y % 2 == 0 {
            let dark = 1.0 - 0.35 * a;
            for v in &mut c[..3] {
                *v *= dark;
            }
        }
        c
    });
}

/// The red and blue pictures slide apart.
fn rgb_split(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let off = a * 24.0 * unit(h).max(0.05);
    resample(rgba, w, h, &|src, x, y| {
        split_sample(src, x as f32 + 0.5, y as f32 + 0.5, off)
    });
}

// ---- moving distortions ----------------------------------------------------------------------

/// The picture repeats inside itself, over and over, and zooms into the middle: every circle that
/// is half as wide shows the picture again.
fn infinite_zoom(rgba: &mut [u8], w: usize, h: usize, a: f32, t: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let r_min = 0.25 * h as f32;
    let period = std::f32::consts::LN_2;
    let phase = (t * 0.5).fract() * period;
    resample(rgba, w, h, &|src, x, y| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let r = dx.hypot(dy).max(1e-3);
        let turns = ((r / r_min).ln() - phase).rem_euclid(period);
        let sampled = r_min * (turns + phase).exp();
        let k = sampled / r;
        let drawn = src.at(cx + dx * k, cy + dy * k);
        let own = src.at(x as f32 + 0.5, y as f32 + 0.5);
        [0, 1, 2, 3].map(|i| mix(own[i], drawn[i], a))
    });
}

/// A ring runs out from the middle and bends the picture where it passes.
fn shockwave(rgba: &mut [u8], w: usize, h: usize, a: f32, t: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let size = h as f32;
    let ring = (t * 0.5).fract() * 0.8 * size;
    let width = 0.08 * size;
    let push = 0.35 * a * width;
    resample(rgba, w, h, &|src, x, y| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let r = dx.hypot(dy).max(1e-3);
        let d = (r - ring) / width;
        // Pulls the picture in front of the ring, pushes it behind: a bump that moves.
        let bump = (-d * d).exp() * (d * PI * 0.5).sin();
        let k = (r - bump * push * 3.0) / r;
        src.at(cx + dx * k, cy + dy * k)
    });
}

/// A quick zoom in, every second, that settles back, with the streaks of the zoom.
fn zoom_punch(rgba: &mut [u8], w: usize, h: usize, a: f32, t: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let punch = (-5.0 * t.fract()).exp();
    let scale = 1.0 + 0.3 * a * punch;
    const TAPS: usize = 8;
    resample(rgba, w, h, &|src, x, y| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let mut sum = [0.0f32; 4];
        for k in 0..TAPS {
            let s = 1.0 + (scale - 1.0) * k as f32 / (TAPS - 1) as f32;
            let c = src.at(cx + dx / s, cy + dy / s);
            for i in 0..4 {
                sum[i] += c[i] / TAPS as f32;
            }
        }
        sum
    });
}

// ---- film and tape ---------------------------------------------------------------------------

/// A warm blot of light moves over the picture, flickering, like film that was let in the light.
fn film_burn(rgba: &mut [u8], w: usize, h: usize, a: f32, t: f32, frame: u64) {
    let (px, py) = (0.5 + 0.55 * (0.9 * t + 1.0).sin(), 0.5 + 0.45 * (0.6 * t).cos());
    let aspect = w as f32 / h as f32;
    let flicker = 1.0 + 0.6 * (h01((frame as u32).wrapping_mul(7919)) - 0.5);
    each_colour(rgba, w, h, &|x, y, c| {
        let (u, v) = (x as f32 / w as f32 - px, y as f32 / h as f32 - py);
        let d = (u * aspect).hypot(v);
        let core = (-(d / 0.3).powi(2)).exp();
        let outer = (-(d / 0.75).powi(2)).exp();
        let strength = a * flicker * 0.9;
        let light = [
            (outer * 1.0 + core * 0.4) * strength,
            (outer * 0.42 + core * 0.55) * strength,
            (outer * 0.08 + core * 0.35) * strength,
        ];
        [0, 1, 2].map(|i| screen(c[i], light[i]))
    });
}

/// A soft band of orange light slides across the picture at a slant.
fn light_leak(rgba: &mut [u8], w: usize, h: usize, a: f32, t: f32) {
    let (cos, sin) = (30.0f32.to_radians().cos(), 30.0f32.to_radians().sin());
    let span = w as f32 * cos + h as f32 * sin;
    let centre = (t * 0.15).fract() * 1.4 - 0.2;
    each_colour(rgba, w, h, &|x, y, c| {
        let u = (x as f32 * cos + y as f32 * sin) / span;
        let band = (-((u - centre) / 0.18).powi(2)).exp() * a * 0.9;
        let light = [1.0 * band, 0.45 * band, 0.2 * band];
        [0, 1, 2].map(|i| screen(c[i], light[i]))
    });
}

/// An old home movie: warm, grainy, with scratches, a picture that wobbles in the gate and a
/// light that flickers.
fn super8(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let f = frame as u32;
    let weave = (3.0 * unit(h)).max(0.5) * a;
    let (wx, wy) = (
        (h01(f.wrapping_mul(31) ^ 1) - 0.5) * 2.0 * weave,
        (h01(f.wrapping_mul(31) ^ 2) - 0.5) * 2.0 * weave,
    );
    let flicker = 1.0 + (h01(f.wrapping_mul(17) ^ 3) - 0.5) * 0.3 * a;
    let scratch_x = (h01((f / 2).wrapping_mul(13) ^ 4) * w as f32) as usize;
    let scratch_on = h01((f / 2).wrapping_mul(13) ^ 5) < 0.5 * a;
    resample(rgba, w, h, &|src, x, y| {
        let c = src.at(x as f32 + 0.5 + wx, y as f32 + 0.5 + wy);
        let alpha = c[3];
        // Warm: the reds lifted, the blues pulled down, the colour a little faded.
        let l = luma([c[0], c[1], c[2]]);
        let mut out = [
            mix(c[0], l, 0.2 * a) * (1.0 + 0.12 * a) + 0.02 * a * alpha,
            mix(c[1], l, 0.2 * a) * (1.0 + 0.03 * a),
            mix(c[2], l, 0.2 * a) * (1.0 - 0.18 * a),
            alpha,
        ];
        let grain = (h3(x as i32, y as i32, f) - 0.5) * 0.18 * a * alpha;
        for v in &mut out[..3] {
            *v = (*v * flicker + grain).clamp(0.0, alpha);
        }
        if scratch_on && x == scratch_x {
            for v in &mut out[..3] {
                *v = (*v + 0.35 * a * alpha).min(alpha);
            }
        }
        out
    });
}

/// A screen that is bending, soaking colour, striped and noisy: a tape on an old television.
fn vhs_crt(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let f = frame as u32;
    let bleed = (3.0 * unit(h)).max(1.0) * (0.5 + a);
    let bend = 0.12 * a;
    resample(rgba, w, h, &|src, x, y| {
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let (u, v) = ((x as f32 + 0.5 - cx) / cx, (y as f32 + 0.5 - cy) / cy);
        let r2 = u * u + v * v;
        let k = 1.0 + bend * r2;
        let (sx, sy) = (cx + u * k * cx, cy + v * k * cy);
        if sx < 0.0 || sy < 0.0 || sx > w as f32 || sy > h as f32 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let mut c = split_sample(src, sx, sy, bleed);
        let l = luma([c[0], c[1], c[2]]);
        let line = if y % 2 == 0 { 1.0 - 0.3 * a } else { 1.0 };
        let vignette = 1.0 - 0.3 * a * r2.min(1.5);
        let alpha = c[3];
        let noise = (h3(x as i32, y as i32, f) - 0.5) * 0.16 * a * alpha;
        for v in &mut c[..3] {
            *v = (mix(*v, l, 0.15 * a) * line * vignette + noise).clamp(0.0, alpha);
        }
        c
    });
}

// ---- light -----------------------------------------------------------------------------------

/// How much of a pixel counts as a highlight, 0 to 1.
fn highlight(c: [f32; 3], threshold: f32) -> f32 {
    ((luma(c) - threshold) / (1.0 - threshold)).clamp(0.0, 1.0)
}

/// Bright points grow streaks along the diagonals.
fn star_filter(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let plane: Vec<f32> = rgba
        .chunks_exact(4)
        .map(|px| highlight(straight(px), 0.75))
        .collect();
    let reach = (0.2 * h as f32 * (0.5 + a)).max(4.0);
    let step = (reach / 16.0).max(1.0);
    let dirs = [(1.0f32, 1.0f32), (1.0, -1.0)].map(|(x, y)| {
        let n = x.hypot(y);
        (x / n, y / n)
    });
    each_colour(rgba, w, h, &|x, y, mut c| {
        let mut add = 0.0f32;
        for (dx, dy) in dirs {
            for sign in [-1.0f32, 1.0] {
                let mut k = step;
                while k <= reach {
                    let (sx, sy) = (x as f32 + sign * dx * k, y as f32 + sign * dy * k);
                    if sx >= 0.0 && sy >= 0.0 && (sx as usize) < w && (sy as usize) < h {
                        let fall = 1.0 - k / reach;
                        add += plane[sy as usize * w + sx as usize] * fall * fall;
                    }
                    k += step;
                }
            }
        }
        let add = add * step / 2.0 * a * 1.5;
        for v in &mut c {
            *v = (*v + add).min(1.0);
        }
        c
    });
}

/// Bright points bloom into soft discs: lights out of focus.
fn bokeh_dream(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let radius = (10.0 * unit(h)).max(2.0);
    let mut lights: Vec<u8> = rgba.to_vec();
    for px in lights.chunks_exact_mut(4) {
        let c = straight(px);
        let k = smoothstep(0.45, 0.75, luma(c));
        set_rgb(px, c.map(|v| v * k));
        px[3] = 255;
    }
    const TAPS: usize = 40;
    let src = Src::new(&lights, w, h);
    let golden = PI * (3.0 - 5.0f32.sqrt());
    each_colour(rgba, w, h, &|x, y, c| {
        let mut sum = [0.0f32; 3];
        for k in 0..TAPS {
            let r = radius * ((k as f32 + 0.5) / TAPS as f32).sqrt();
            let angle = k as f32 * golden;
            let s = src.at(x as f32 + 0.5 + r * angle.cos(), y as f32 + 0.5 + r * angle.sin());
            for i in 0..3 {
                sum[i] += s[i] / TAPS as f32;
            }
        }
        [0, 1, 2].map(|i| (c[i] + sum[i] * 1.6 * a).min(1.0))
    });
}

/// Little stars twinkle on the brightest points.
fn sparkle(rgba: &mut [u8], w: usize, h: usize, a: f32, t: f32) {
    let cell = ((24.0 * unit(h)).round() as usize).max(6);
    let reach = (cell as f32 * 0.8).max(4.0);
    let mut stars: Vec<(f32, f32, f32)> = Vec::new();
    for cy in 0..h.div_ceil(cell) {
        for cx in 0..w.div_ceil(cell) {
            if h3(cx as i32, cy as i32, 11) > 0.5 {
                continue;
            }
            // The brightest point of the cell.
            let mut best = (0.0f32, 0usize, 0usize);
            for y in cy * cell..((cy + 1) * cell).min(h) {
                for x in cx * cell..((cx + 1) * cell).min(w) {
                    let l = luma(straight(&rgba[(y * w + x) * 4..]));
                    if l > best.0 {
                        best = (l, x, y);
                    }
                }
            }
            let (_, x, y) = best;
            let lit = highlight(straight(&rgba[(y * w + x) * 4..]), 0.7);
            if lit <= 0.0 {
                continue;
            }
            let phase = h3(cx as i32, cy as i32, 14);
            let twinkle = (0.5 + 0.5 * (TAU * (t + phase)).sin()).powi(2);
            stars.push((x as f32 + 0.5, y as f32 + 0.5, lit * twinkle));
        }
    }
    if stars.is_empty() {
        return;
    }
    each_colour(rgba, w, h, &|x, y, mut c| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut add = 0.0f32;
        for &(sx, sy, strength) in &stars {
            let (dx, dy) = ((fx - sx).abs(), (fy - sy).abs());
            if dx > reach || dy > reach {
                continue;
            }
            // A cross: bright along both axes, thin across them.
            let along = dx.max(dy);
            let across = dx.min(dy);
            let ray = (1.0 - along / reach).max(0.0).powi(2) * (-(across / 1.5).powi(2)).exp();
            let core = (-(dx * dx + dy * dy) / 4.0).exp();
            add += (ray + core) * strength;
        }
        let add = add * a * 1.2;
        for v in &mut c {
            *v = (*v + add).min(1.0);
        }
        c
    });
}

// ---- painting --------------------------------------------------------------------------------

/// Edges glow in their own colour on a darker picture.
fn neon_edges(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    if w < 3 || h < 3 {
        return;
    }
    let l: Vec<f32> = rgba.chunks_exact(4).map(|px| luma(straight(px))).collect();
    let at = |x: usize, y: usize| l[y * w + x];
    let mut glow = vec![0u8; w * h * 4];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = -at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1)
                + at(x + 1, y - 1)
                + 2.0 * at(x + 1, y)
                + at(x + 1, y + 1);
            let gy = -at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1)
                + at(x - 1, y + 1)
                + 2.0 * at(x, y + 1)
                + at(x + 1, y + 1);
            let edge = (gx.hypot(gy) * 1.5).clamp(0.0, 1.0);
            let c = straight(&rgba[(y * w + x) * 4..]);
            let top = c[0].max(c[1]).max(c[2]);
            // The colour of the edge at full strength.
            let hue = if top > 0.05 { c.map(|v| v / top) } else { [1.0; 3] };
            let i = (y * w + x) * 4;
            set_rgb(&mut glow[i..], hue.map(|v| v * edge));
            glow[i + 3] = 255;
        }
    }
    let core = glow.clone();
    gaussian(&mut glow, w, h, ((8.0 * unit(h)).round() as usize).max(2));
    let copy = rgba.to_vec();
    par_pixels(rgba, w, h, &|x, y, px| {
        let i = (y * w + x) * 4;
        let (base, edge, haze) = (straight(&copy[i..]), straight(&core[i..]), straight(&glow[i..]));
        let out = [0, 1, 2].map(|k| base[k] * (1.0 - 0.65 * a) + edge[k] * a + haze[k] * 1.5 * a);
        set_rgb(px, out.map(|v| v.clamp(0.0, 1.0)));
    });
}

/// Brushstrokes: each pixel takes the average of the calmest of the four squares around it
/// (a Kuwahara filter), so flat areas turn to patches of colour and edges stay sharp.
fn oil_paint(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let radius = (((2.0 + 6.0 * a) * (h as f32 / 540.0).clamp(0.5, 2.0)).round() as usize)
        .clamp(1, 12);
    let stride = w + 1;
    // Sums over rectangles of: red, green, blue, luma, luma squared.
    let mut sums = vec![[0.0f64; 5]; stride * (h + 1)];
    for y in 0..h {
        let mut row = [0.0f64; 5];
        for x in 0..w {
            let c = straight(&rgba[(y * w + x) * 4..]);
            let l = f64::from(luma(c));
            let v = [f64::from(c[0]), f64::from(c[1]), f64::from(c[2]), l, l * l];
            for i in 0..5 {
                row[i] += v[i];
                sums[(y + 1) * stride + x + 1][i] = sums[y * stride + x + 1][i] + row[i];
            }
        }
    }
    let area = |x0: usize, y0: usize, x1: usize, y1: usize| {
        // The rectangle [x0, x1) by [y0, y1).
        let (a, b, c, d) = (
            sums[y0 * stride + x0],
            sums[y0 * stride + x1],
            sums[y1 * stride + x0],
            sums[y1 * stride + x1],
        );
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        [0, 1, 2, 3, 4].map(|i| (d[i] - b[i] - c[i] + a[i]) / n)
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (x0, x1) = (x.saturating_sub(radius), (x + radius + 1).min(w));
        let (y0, y1) = (y.saturating_sub(radius), (y + radius + 1).min(h));
        let mut best = (f64::MAX, [0.0f64; 3]);
        for (qx0, qx1, qy0, qy1) in [
            (x0, x + 1, y0, y + 1),
            (x, x1, y0, y + 1),
            (x0, x + 1, y, y1),
            (x, x1, y, y1),
        ] {
            let m = area(qx0, qy0, qx1, qy1);
            let variance = m[4] - m[3] * m[3];
            if variance < best.0 {
                best = (variance, [m[0], m[1], m[2]]);
            }
        }
        set_rgb(px, best.1.map(|v| v as f32));
    });
}

// ---- colour ----------------------------------------------------------------------------------

/// Cool shadows and warm highlights, with a little more contrast: the look of the cinema.
fn cinematic_grade(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    each_colour(rgba, w, h, &|_, _, c| {
        let l = luma(c);
        let shadow = (1.0 - l).powi(2);
        let light = l * l;
        // Teal into the shadows (less red, more blue), orange into the lights.
        let tint = [
            -0.07 * shadow + 0.09 * light,
            0.02 * shadow + 0.03 * light,
            0.09 * shadow - 0.10 * light,
        ];
        // An S curve about the middle.
        let curve = |v: f32| mix(v, smoothstep(0.0, 1.0, v), 0.45);
        [0, 1, 2].map(|i| mix(c[i], curve(c[i] + tint[i]), a).clamp(0.0, 1.0))
    });
}

/// Turns every colour round the colour wheel by up to a whole turn (the luma is kept).
fn hue_shift(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let (sin, cos) = (-(a * TAU)).sin_cos();
    // A rotation about the grey axis, in the YIQ colour space.
    let m = [
        [0.299 + 0.701 * cos + 0.168 * sin, 0.587 - 0.587 * cos + 0.330 * sin, 0.114 - 0.114 * cos - 0.497 * sin],
        [0.299 - 0.299 * cos - 0.328 * sin, 0.587 + 0.413 * cos + 0.035 * sin, 0.114 - 0.114 * cos + 0.292 * sin],
        [0.299 - 0.300 * cos + 1.250 * sin, 0.587 - 0.588 * cos - 1.050 * sin, 0.114 + 0.886 * cos - 0.203 * sin],
    ];
    each_colour(rgba, w, h, &|_, _, c| {
        m.map(|r| (r[0] * c[0] + r[1] * c[1] + r[2] * c[2]).clamp(0.0, 1.0))
    });
}

/// Brighter, by up to half of the way to white.
fn brightness(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    each_colour(rgba, w, h, &|_, _, c| c.map(|v| mix(v, 1.0, a * 0.5)));
}

/// More contrast about the middle grey.
fn contrast(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = 1.0 + a * 1.5;
    each_colour(rgba, w, h, &|_, _, c| c.map(|v| ((v - 0.5) * k + 0.5).clamp(0.0, 1.0)));
}

/// Lifts the middle tones and keeps black and white where they are.
fn gamma(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let g = 1.0 / (1.0 + a * 1.5);
    each_colour(rgba, w, h, &|_, _, c| c.map(|v| v.powf(g)));
}

/// Stronger colours: up to twice as much as they have.
fn saturation(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    each_colour(rgba, w, h, &|_, _, c| {
        let l = luma(c);
        c.map(|v| (l + (v - l) * (1.0 + a)).clamp(0.0, 1.0))
    });
}

pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize, frame: u64, rate: f32) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let t = frame as f32 / rate.max(1.0);
    match kind {
        EffectKind::BlockGlitch => block_glitch(rgba, w, h, a, frame),
        EffectKind::DigitalGlitch => digital_glitch(rgba, w, h, a, frame),
        EffectKind::ScanlineGlitch => scanline_glitch(rgba, w, h, a, frame),
        EffectKind::RgbSplit => rgb_split(rgba, w, h, a),
        EffectKind::InfiniteZoom => infinite_zoom(rgba, w, h, a, t),
        EffectKind::Shockwave => shockwave(rgba, w, h, a, t),
        EffectKind::ZoomPunch => zoom_punch(rgba, w, h, a, t),
        EffectKind::FilmBurn => film_burn(rgba, w, h, a, t, frame),
        EffectKind::LightLeak => light_leak(rgba, w, h, a, t),
        EffectKind::Super8 => super8(rgba, w, h, a, frame),
        EffectKind::VhsCrt => vhs_crt(rgba, w, h, a, frame),
        EffectKind::StarFilter => star_filter(rgba, w, h, a),
        EffectKind::BokehDream => bokeh_dream(rgba, w, h, a),
        EffectKind::Sparkle => sparkle(rgba, w, h, a, t),
        EffectKind::NeonEdges => neon_edges(rgba, w, h, a),
        EffectKind::OilPaint => oil_paint(rgba, w, h, a),
        EffectKind::CinematicGrade => cinematic_grade(rgba, w, h, a),
        EffectKind::HueShift => hue_shift(rgba, w, h, a),
        EffectKind::Brightness => brightness(rgba, w, h, a),
        EffectKind::Contrast => contrast(rgba, w, h, a),
        EffectKind::Gamma => gamma(rgba, w, h, a),
        EffectKind::Saturation => saturation(rgba, w, h, a),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    pub(super) const KINDS: [EffectKind; 22] = [
        EffectKind::BlockGlitch,
        EffectKind::DigitalGlitch,
        EffectKind::ScanlineGlitch,
        EffectKind::RgbSplit,
        EffectKind::InfiniteZoom,
        EffectKind::Shockwave,
        EffectKind::ZoomPunch,
        EffectKind::FilmBurn,
        EffectKind::LightLeak,
        EffectKind::Super8,
        EffectKind::VhsCrt,
        EffectKind::StarFilter,
        EffectKind::BokehDream,
        EffectKind::Sparkle,
        EffectKind::NeonEdges,
        EffectKind::OilPaint,
        EffectKind::CinematicGrade,
        EffectKind::HueShift,
        EffectKind::Brightness,
        EffectKind::Contrast,
        EffectKind::Gamma,
        EffectKind::Saturation,
    ];

    const W: usize = 64;
    const H: usize = 48;

    /// Stripes of colour and a bright dot: something for every effect to change.
    fn picture() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let stripe = (x / 4 + y / 6) % 3;
                let mut c = match stripe {
                    0 => [200u8, 60, 50],
                    1 => [40, 160, 90],
                    _ => [60, 70, 210],
                };
                if (x as i32 - 32).abs() < 2 && (y as i32 - 24).abs() < 2 {
                    c = [255, 255, 255];
                }
                v.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        v
    }

    fn run(kind: EffectKind, amount: f32, src: &[u8], frame: u64) -> Vec<u8> {
        let mut out = src.to_vec();
        let mut e = Effect::new(kind);
        e.amount = amount;
        e.apply(&mut out, W, H, frame);
        out
    }

    fn px(buf: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * W + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    fn mean(buf: &[u8]) -> f32 {
        buf.chunks_exact(4)
            .map(|p| (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) as f32 / 3.0)
            .sum::<f32>()
            / (buf.len() / 4) as f32
    }

    fn diff(a: &[u8], b: &[u8]) -> usize {
        a.iter().zip(b).filter(|(x, y)| x != y).count()
    }

    #[test]
    fn every_effect_changes_the_picture_keeps_its_alpha_and_is_repeatable() {
        let src = picture();
        for kind in KINDS {
            let out = run(kind, 80.0, &src, 3);
            assert_eq!(out.len(), src.len(), "{kind:?}");
            assert!(diff(&out, &src) > 20, "{kind:?} leaves the picture alone");
            assert_eq!(out, run(kind, 80.0, &src, 3), "{kind:?} is not repeatable");
            // At no strength it does nothing.
            assert_eq!(run(kind, 0.0, &src, 3), src, "{kind:?}");
            // The inside of the picture stays opaque.
            assert!(px(&out, W / 2, H / 2)[3] == 255, "{kind:?}");
        }
    }

    #[test]
    fn effects_that_move_look_different_a_moment_later() {
        let src = picture();
        for kind in [
            EffectKind::BlockGlitch,
            EffectKind::DigitalGlitch,
            EffectKind::ScanlineGlitch,
            EffectKind::FilmBurn,
            EffectKind::LightLeak,
            EffectKind::Super8,
            EffectKind::Shockwave,
            EffectKind::InfiniteZoom,
            EffectKind::Sparkle,
        ] {
            let (a, b) = (run(kind, 100.0, &src, 2), run(kind, 100.0, &src, 11));
            assert_ne!(a, b, "{kind:?} is the same at two moments");
        }
    }

    #[test]
    fn rgb_split_pulls_the_red_and_the_blue_apart() {
        // A white bar on black: red shows on one side of it and blue on the other.
        let mut src = vec![0u8; W * H * 4];
        for y in 0..H {
            for x in 0..W {
                let i = (y * W + x) * 4;
                let on = (28..36).contains(&x);
                src[i..i + 4].copy_from_slice(&[if on { 255 } else { 0 }, if on { 255 } else { 0 }, if on { 255 } else { 0 }, 255]);
            }
        }
        let out = run(EffectKind::RgbSplit, 100.0, &src, 0);
        // The red is read from the right and the blue from the left: so the bar's left side
        // shows red, and its right side shows blue.
        let right = px(&out, 36, 10);
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
        let left = px(&out, 27, 10);
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        // The middle of the bar stays white.
        assert_eq!(&px(&out, 32, 10)[..3], &[255, 255, 255]);
    }

    #[test]
    fn infinite_zoom_repeats_the_picture_at_half_the_size() {
        let src = picture();
        let out = run(EffectKind::InfiniteZoom, 100.0, &src, 0);
        let (cx, cy) = (W as f32 / 2.0, H as f32 / 2.0);
        // 0.35 of the height out is where the picture is; half that is a copy of it.
        let r = 0.35 * H as f32;
        let at = |r: f32| px(&out, (cx + r) as usize, cy as usize);
        let (outer, inner) = (at(r), at(r / 2.0));
        for i in 0..3 {
            assert!((i32::from(outer[i]) - i32::from(inner[i])).abs() <= 30, "{outer:?} {inner:?}");
        }
    }

    #[test]
    fn a_shockwave_leaves_what_the_ring_has_not_reached() {
        let src = picture();
        // At 0.5 s the ring is at 0.2 of the height; the corner is far from it.
        let out = run(EffectKind::Shockwave, 100.0, &src, 12);
        assert_eq!(px(&out, 0, 0), px(&src, 0, 0));
        assert!(diff(&out, &src) > 50);
    }

    #[test]
    fn zoom_punch_is_strongest_on_the_beat_and_keeps_the_middle() {
        let src = picture();
        let on_beat = run(EffectKind::ZoomPunch, 100.0, &src, 0);
        let settled = run(EffectKind::ZoomPunch, 100.0, &src, 18);
        let distance = |a: &[u8]| -> u64 {
            a.iter().zip(&src).map(|(x, y)| u64::from(x.abs_diff(*y))).sum()
        };
        assert!(distance(&on_beat) > distance(&settled) * 3, "{} {}", distance(&on_beat), distance(&settled));
        // The dot in the middle stays where it is.
        assert!(px(&on_beat, 32, 24)[0] > 200);
    }

    #[test]
    fn star_filter_streaks_along_the_diagonals_and_not_across() {
        let mut src = vec![0u8; W * H * 4];
        for p in src.chunks_exact_mut(4) {
            p[3] = 255;
        }
        let i = (24 * W + 32) * 4;
        src[i..i + 3].copy_from_slice(&[255, 255, 255]);
        let out = run(EffectKind::StarFilter, 100.0, &src, 0);
        let diagonal = px(&out, 32 + 3, 24 + 3)[0];
        let across = px(&out, 32 + 3, 24 + 1)[0];
        assert!(diagonal > across + 10, "{diagonal} {across}");
        // Far from the star it stays dark.
        assert_eq!(px(&out, 2, 2)[0], 0);
    }

    #[test]
    fn bokeh_blooms_bright_points_and_leaves_far_dark_alone() {
        let mut src = vec![0u8; W * H * 4];
        for p in src.chunks_exact_mut(4) {
            p[3] = 255;
        }
        let i = (24 * W + 32) * 4;
        src[i..i + 3].copy_from_slice(&[255, 255, 255]);
        let out = run(EffectKind::BokehDream, 100.0, &src, 0);
        assert!(px(&out, 34, 24)[0] > 0, "the light did not spread");
        assert_eq!(px(&out, 2, 2)[0], 0);
    }

    #[test]
    fn sparkle_lights_bright_points_only() {
        let dark = vec![[0u8, 0, 0, 255]; W * H].concat();
        assert_eq!(run(EffectKind::Sparkle, 100.0, &dark, 4), dark);
        let bright = vec![[250u8, 250, 250, 255]; W * H].concat();
        let out = run(EffectKind::Sparkle, 100.0, &bright, 4);
        assert!(out.chunks_exact(4).all(|p| p[0] >= 250));
        assert!(mean(&out) >= mean(&bright));
    }

    #[test]
    fn neon_edges_light_the_edge_and_darken_the_flat() {
        // Dark on the left, mid-grey on the right.
        let mut src = vec![0u8; W * H * 4];
        for y in 0..H {
            for x in 0..W {
                let i = (y * W + x) * 4;
                let v = if x < 32 { 20 } else { 120 };
                src[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let out = run(EffectKind::NeonEdges, 100.0, &src, 0);
        let edge = px(&out, 32, 24)[0];
        let flat = px(&out, 50, 24)[0];
        assert!(edge > flat + 40, "{edge} {flat}");
        assert!(flat < 120);
    }

    #[test]
    fn oil_paint_flattens_noise_and_keeps_a_hard_edge() {
        let mut src = vec![0u8; W * H * 4];
        for y in 0..H {
            for x in 0..W {
                let i = (y * W + x) * 4;
                let base = if x < 32 { 40i32 } else { 200 };
                let v = (base + (h3(x as i32, y as i32, 1) * 30.0) as i32 - 15).clamp(0, 255) as u8;
                src[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let out = run(EffectKind::OilPaint, 60.0, &src, 0);
        let spread = |buf: &[u8], x0: usize, x1: usize| {
            let v: Vec<f32> = (0..H)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .map(|(x, y)| f32::from(px(buf, x, y)[0]))
                .collect();
            let m = v.iter().sum::<f32>() / v.len() as f32;
            v.iter().map(|a| (a - m).powi(2)).sum::<f32>() / v.len() as f32
        };
        assert!(spread(&out, 4, 24) < spread(&src, 4, 24) * 0.5);
        // The pixels beside the edge keep their side's brightness.
        assert!(px(&out, 31, 24)[0] < 100, "{:?}", px(&out, 31, 24));
        assert!(px(&out, 32, 24)[0] > 140, "{:?}", px(&out, 32, 24));
    }

    #[test]
    fn colour_effects_do_what_they_say() {
        let grey = |v: u8| vec![[v, v, v, 255u8]; W * H].concat();
        let dark = run(EffectKind::CinematicGrade, 100.0, &grey(40), 0);
        let light = run(EffectKind::CinematicGrade, 100.0, &grey(215), 0);
        let (d, l) = (px(&dark, 3, 3), px(&light, 3, 3));
        assert!(d[2] > d[0], "shadows are not teal: {d:?}");
        assert!(l[0] > l[2], "lights are not warm: {l:?}");
        assert!(mean(&run(EffectKind::Brightness, 100.0, &grey(80), 0)) > 80.0 + 30.0);
        let darker = px(&run(EffectKind::Contrast, 100.0, &grey(80), 0), 3, 3)[0];
        let lighter = px(&run(EffectKind::Contrast, 100.0, &grey(180), 0), 3, 3)[0];
        assert!(darker < 80 && lighter > 180, "{darker} {lighter}");
        // Gamma lifts the middle and keeps black and white.
        assert!(px(&run(EffectKind::Gamma, 100.0, &grey(100), 0), 3, 3)[0] > 130);
        assert_eq!(px(&run(EffectKind::Gamma, 100.0, &grey(0), 0), 3, 3)[0], 0);
        assert_eq!(px(&run(EffectKind::Gamma, 100.0, &grey(255), 0), 3, 3)[0], 255);
        // Saturation leaves grey alone and strengthens colour.
        assert_eq!(px(&run(EffectKind::Saturation, 100.0, &grey(120), 0), 3, 3)[0], 120);
        let muted = vec![[150u8, 120, 110, 255]; W * H].concat();
        let out = px(&run(EffectKind::Saturation, 100.0, &muted, 0), 3, 3);
        assert!(i32::from(out[0]) - i32::from(out[2]) > 40 + 5);
    }

    #[test]
    fn hue_shift_a_third_of_a_turn_turns_red_to_green() {
        let red = vec![[255u8, 0, 0, 255]; W * H].concat();
        let out = px(&run(EffectKind::HueShift, 100.0 / 3.0, &red, 0), 3, 3);
        assert!(out[1] > out[0] && out[1] > out[2], "{out:?}");
        // A whole turn gets the colour back.
        let back = px(&run(EffectKind::HueShift, 100.0, &red, 0), 3, 3);
        assert!(back[0] > 240 && back[1] < 15 && back[2] < 15, "{back:?}");
    }

    #[test]
    fn film_looks_warm_and_a_leak_lights_a_dark_picture() {
        let dark = vec![[20u8, 20, 20, 255]; W * H].concat();
        for kind in [EffectKind::FilmBurn, EffectKind::LightLeak] {
            let out = run(kind, 100.0, &dark, 8);
            let lit = out.chunks_exact(4).max_by_key(|p| p[0]).unwrap();
            assert!(lit[0] > 60 && mean(&out) > 21.0, "{kind:?}: {lit:?}");
            assert!(lit[0] > lit[2], "{kind:?}: the light is not warm {lit:?}");
        }
        let grey = vec![[128u8, 128, 128, 255]; W * H].concat();
        let out = run(EffectKind::Super8, 100.0, &grey, 5);
        let m = out.chunks_exact(4).fold([0u32; 3], |s, p| [s[0] + u32::from(p[0]), s[1] + u32::from(p[1]), s[2] + u32::from(p[2])]);
        assert!(m[0] > m[2], "Super 8 is not warm");
    }

    #[test]
    fn crt_and_tape_stripe_the_picture_and_fill_the_corners() {
        let grey = vec![[128u8, 128, 128, 255]; W * H].concat();
        let out = run(EffectKind::VhsCrt, 100.0, &grey, 0);
        // Even rows are darker than odd ones.
        let row = |y: usize| (8..56).map(|x| f32::from(px(&out, x, y)[0])).sum::<f32>();
        assert!(row(20) < row(21));
        // The bend leaves the corners black or near it.
        assert!(px(&out, 0, 0)[0] < 60);
        let scan = run(EffectKind::ScanlineGlitch, 100.0, &grey, 0);
        let row = |y: usize| (8..56).map(|x| f32::from(px(&scan, x, y)[0])).sum::<f32>();
        assert!(row(20) < row(21));
    }
}
