//! Stylising effects: Colored Border, Chromatic Aberration, Edge Detect, Emboss, Stroke,
//! Watercolor and Stylize (relief, ink outline, cutout).
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100, that scales
//! its main setting from nothing (0) through the usual value (50) to twice as much (100); for
//! Edge Detect, Emboss and Stylize it is how much of the effect is mixed in. Sizes are in
//! thousandths of the picture's height.

use crate::fx::{gaussian, EffectKind};
use crate::keys::morph_plane;
use crate::look::{par_pixels, put, Src};

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

fn rgb_of(px: &[u8]) -> [f32; 3] {
    [0, 1, 2].map(|i| f32::from(px[i]) / 255.0)
}

fn straight(q: [f32; 4]) -> [f32; 3] {
    if q[3] <= 1e-6 {
        return [0.0; 3];
    }
    [q[0] / q[3], q[1] / q[3], q[2] / q[3]]
}

fn premult_luma(q: [f32; 4]) -> f32 {
    luma([q[0], q[1], q[2]])
}

fn set_rgb(px: &mut [u8], c: [f32; 3]) {
    for i in 0..3 {
        px[i] = (c[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
}

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn hash2(x: i32, y: i32, f: u32) -> f32 {
    (hash32(
        (x as u32).wrapping_mul(0x8da6_b343)
            ^ (y as u32).wrapping_mul(0xd816_3841)
            ^ f.wrapping_mul(0xcb1a_b31f),
    ) >> 8) as f32
        / 16_777_215.0
}

// ---- Colored Border ----

/// Paints a coloured frame inside the edges; `radius` rounds its inner corners (both in 1080-line
/// pixels at the usual strength).
fn colored_border(rgba: &mut [u8], w: usize, h: usize, a: f32, radius: f32) {
    let k = a * 2.0;
    let width = 24.0 * unit(h) * k;
    if width < 0.5 {
        return;
    }
    let colour = [
        0xf2 as f32 / 255.0,
        0xb1 as f32 / 255.0,
        0x34 as f32 / 255.0,
    ];
    let (wf, hf) = (w as f32, h as f32);
    let inner = [0.5 * wf - width, 0.5 * hf - width];
    let r = (radius * unit(h)).clamp(0.0, inner[0].min(inner[1]).max(0.0));
    par_pixels(rgba, w, h, &|x, y, px| {
        let p = [x as f32 + 0.5 - 0.5 * wf, y as f32 + 0.5 - 0.5 * hf];
        let d = [p[0].abs() - inner[0] + r, p[1].abs() - inner[1] + r];
        let sd = d[0].max(0.0).hypot(d[1].max(0.0)) + d[0].max(d[1]).min(0.0) - r;
        let cover = (0.5 + sd).clamp(0.0, 1.0);
        if cover <= 0.0 {
            return;
        }
        let al = f32::from(px[3]) / 255.0;
        let under = [0, 1, 2].map(|i| f32::from(px[i]) / 255.0 * al);
        let out = [0, 1, 2].map(|i| colour[i] * cover + under[i] * (1.0 - cover));
        put(px, [out[0], out[1], out[2], cover + al * (1.0 - cover)]);
    });
}

// ---- Chromatic Aberration ----

/// Red and blue fringes toward the edges, like a cheap lens: red and blue are scaled about the
/// middle by `5` pixels at the corners at the usual strength.
fn chromatic_aberration(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let kk = 5.0 * unit(h) * a * 2.0 / cx.hypot(cy).max(1.0);
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    par_pixels(rgba, w, h, &|x, y, px| {
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let (dx, dy) = (xf - cx, yf - cy);
        let r = src.at(cx + dx * (1.0 - kk), cy + dy * (1.0 - kk));
        let g = src.at(xf, yf);
        let b = src.at(cx + dx * (1.0 + kk), cy + dy * (1.0 + kk));
        put(px, [r[0], g[1], b[2], g[3].max(r[3]).max(b[3])]);
    });
}

// ---- Edge Detect ----

/// Bright lines where the picture has edges (a Sobel gradient of the brightness).
fn edge_detect(rgba: &mut [u8], w: usize, h: usize, a: f32, invert: bool) {
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let (wi, hi) = (w as i64, h as i64);
    let amount = (a * 2.0).clamp(0.0, 1.0);
    par_pixels(rgba, w, h, &|x, y, px| {
        let l = |dx: i64, dy: i64| {
            let xx = (x as i64 + dx).clamp(0, wi - 1) as usize;
            let yy = (y as i64 + dy).clamp(0, hi - 1) as usize;
            premult_luma(src.texel(xx, yy))
        };
        let gx = -l(-1, -1) - 2.0 * l(-1, 0) - l(-1, 1) + l(1, -1) + 2.0 * l(1, 0) + l(1, 1);
        let gy = -l(-1, -1) - 2.0 * l(0, -1) - l(1, -1) + l(-1, 1) + 2.0 * l(0, 1) + l(1, 1);
        let mut e = (gx.hypot(gy)).clamp(0.0, 1.0);
        if invert {
            e = 1.0 - e;
        }
        let c = rgb_of(px);
        set_rgb(px, [0, 1, 2].map(|i| mix(c[i], e, amount)));
    });
}

// ---- Emboss ----

/// A relief look lit from the upper left: the difference in brightness across a small step.
fn emboss(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let angle = 135.0f32.to_radians();
    let step = (2.0 * unit(h)).max(0.5);
    let (ox, oy) = (angle.cos() * step, -angle.sin() * step);
    let amount = (a * 2.0).clamp(0.0, 1.0);
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    par_pixels(rgba, w, h, &|x, y, px| {
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let v = 0.5
            + 2.0
                * (premult_luma(src.at(xf + ox, yf + oy)) - premult_luma(src.at(xf - ox, yf - oy)));
        let c = rgb_of(px);
        set_rgb(px, [0, 1, 2].map(|i| mix(c[i], v.clamp(0.0, 1.0), amount)));
    });
}

// ---- Stroke ----

/// A white border around the shape of the clip (titles, keyed or scaled-down clips), outside
/// its edge or inside it.
fn stroke(rgba: &mut [u8], w: usize, h: usize, a: f32, inside: bool) {
    let width = (4.0 * unit(h) * a * 2.0).round() as usize;
    if width == 0 {
        return;
    }
    let alpha: Vec<f32> = rgba
        .chunks_exact(4)
        .map(|p| f32::from(p[3]) / 255.0)
        .collect();
    let moved = morph_plane(&alpha, w, h, width, !inside);
    par_pixels(rgba, w, h, &|x, y, px| {
        let i = y * w + x;
        let m = moved[i];
        let s_a = alpha[i];
        let s = [0, 1, 2].map(|c| f32::from(px[c]) / 255.0 * s_a);
        if inside {
            let band = (s_a - m).clamp(0.0, 1.0);
            let out = [0, 1, 2].map(|c| band + s[c] * (1.0 - band));
            put(px, [out[0], out[1], out[2], band + s_a * (1.0 - band)]);
        } else {
            let k = m;
            let out = [0, 1, 2].map(|c| s[c] + k * (1.0 - s_a));
            put(px, [out[0], out[1], out[2], s_a + k * (1.0 - s_a)]);
        }
    });
}

// ---- Watercolor ----

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i32, fy as i32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (s(x - fx), s(y - fy));
    let at = |dx: i32, dy: i32| hash2(ix + dx, iy + dy, seed);
    mix(mix(at(0, 0), at(1, 0), tx), mix(at(0, 1), at(1, 1), tx), ty)
}

/// Paints the picture in flat washes of colour with darker pooled edges and paper grain: each
/// pixel takes the mean colour of the calmest of four quadrants around it (the Kuwahara filter).
fn watercolor(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let radius = (5.0 * unit(h) * a * 2.0).clamp(1.0, 40.0);
    let n = radius.min(7.0) as i32;
    let step = radius / n as f32;
    let (edge_dark, paper, saturation) = (0.35f32, 0.3f32, 0.15f32);
    let scale = unit(h).max(0.25);
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut best = rgb_of(px);
        let mut best_var = f32::MAX;
        for q in 0..4 {
            let sg = [
                if q & 1 == 0 { 1.0 } else { -1.0 },
                if q & 2 == 0 { 1.0 } else { -1.0 },
            ];
            let (mut sum, mut sum2, mut cnt) = ([0.0f32; 3], 0.0f32, 0.0f32);
            for j in 0..=n {
                for i in 0..=n {
                    let s = straight(
                        src.at(xf + sg[0] * i as f32 * step, yf + sg[1] * j as f32 * step),
                    );
                    for c in 0..3 {
                        sum[c] += s[c];
                    }
                    let l = luma(s);
                    sum2 += l * l;
                    cnt += 1.0;
                }
            }
            let mean = sum.map(|v| v / cnt);
            let ml = luma(mean);
            let var = sum2 / cnt - ml * ml;
            if var < best_var {
                best_var = var;
                best = mean;
            }
        }
        let mut c = best;
        let e = (0.5 * radius).max(1.0);
        let l = |ox: f32, oy: f32| luma(straight(src.at(xf + ox, yf + oy)));
        let ed = edge_dark
            * smoothstep(
                0.04,
                0.25,
                (l(e, 0.0) - l(-e, 0.0)).hypot(l(0.0, e) - l(0.0, -e)),
            );
        c = c.map(|v| v * mix(1.0, v, ed));
        let y_c = luma(c);
        c = c.map(|v| mix(y_c, v, 1.0 + saturation));
        let grain = (value_noise(xf / (1.5 * scale), yf / (1.5 * scale), 3) - 0.5) * 0.14
            + (value_noise(xf / (9.0 * scale), yf / (9.0 * scale), 5) - 0.5) * 0.10;
        set_rgb(
            px,
            c.map(|v| v * (1.0 + grain * paper * 2.0) + 0.04 * paper),
        );
    });
}

// ---- Stylize ----

#[derive(Clone, Copy)]
enum Style {
    Relief,
    Ink,
    Cutout,
}

/// Turns the picture into a relief, an ink drawing or a flat cutout.
fn stylize(rgba: &mut [u8], w: usize, h: usize, a: f32, style: Style) {
    let amount = a.clamp(0.0, 1.0);
    let step = (2.0 * unit(h)).max(1.0);
    let levels = 5.0f32;
    let angle = 135.0f32.to_radians();
    let mut soft = rgba.to_vec();
    gaussian(&mut soft, w, h, (2.0 * step).round().max(1.0) as usize);
    let blurred = soft;
    let src = Src::new(&blurred, w, h);
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let c = rgb_of(px);
        let b = straight(src.at(xf, yf));
        let l = |ox: f32, oy: f32| luma(straight(src.at(xf + ox, yf + oy)));
        let gx = l(step, 0.0) - l(-step, 0.0);
        let gy = l(0.0, step) - l(0.0, -step);
        let o = match style {
            Style::Relief => {
                let lit = gx * angle.cos() + gy * angle.sin();
                b.map(|v| v * (1.0 + 4.0 * lit).clamp(0.0, 2.2))
            }
            Style::Ink => {
                let post = b.map(|v| (v * levels + 0.5).floor() / levels);
                let edge = smoothstep(0.04, 0.14, gx.hypot(gy));
                post.map(|v| mix(v, 0.0, edge))
            }
            Style::Cutout => b.map(|v| (v * levels + 0.5).floor() / levels),
        };
        set_rgb(px, [0, 1, 2].map(|i| mix(c[i], o[i], amount)));
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    match kind {
        EffectKind::ColoredBorder => colored_border(rgba, w, h, a, 0.0),
        EffectKind::ColoredBorderRounded => colored_border(rgba, w, h, a, 80.0),
        EffectKind::ChromaticAberration => chromatic_aberration(rgba, w, h, a),
        EffectKind::EdgeDetect => edge_detect(rgba, w, h, a, false),
        EffectKind::EdgeDetectInverted => edge_detect(rgba, w, h, a, true),
        EffectKind::Emboss => emboss(rgba, w, h, a),
        EffectKind::Stroke => stroke(rgba, w, h, a, false),
        EffectKind::StrokeInside => stroke(rgba, w, h, a, true),
        EffectKind::Watercolor => watercolor(rgba, w, h, a),
        EffectKind::StylizeRelief => stylize(rgba, w, h, a, Style::Relief),
        EffectKind::StylizeInk => stylize(rgba, w, h, a, Style::Ink),
        EffectKind::StylizeCutout => stylize(rgba, w, h, a, Style::Cutout),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const KINDS: [EffectKind; 12] = [
        EffectKind::ColoredBorder,
        EffectKind::ColoredBorderRounded,
        EffectKind::ChromaticAberration,
        EffectKind::EdgeDetect,
        EffectKind::EdgeDetectInverted,
        EffectKind::Emboss,
        EffectKind::Stroke,
        EffectKind::StrokeInside,
        EffectKind::Watercolor,
        EffectKind::StylizeRelief,
        EffectKind::StylizeInk,
        EffectKind::StylizeCutout,
    ];

    fn solid(w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
        (0..w * h)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    /// Dark left half, light right half.
    fn step(w: usize, h: usize) -> Vec<u8> {
        let mut v = solid(w, h, [40, 40, 40]);
        for y in 0..h {
            for x in w / 2..w {
                v[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(210);
            }
        }
        v
    }

    fn noisy(w: usize, h: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let n = (((x * 73) ^ (y * 151)) % 200) as u8 + 20;
                v.extend_from_slice(&[n, 255 - n, n / 2 + 40, 255]);
            }
        }
        v
    }

    fn run(kind: EffectKind, amount: f32, pic: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut out = pic.to_vec();
        Effect {
            amount,
            ..Effect::new(kind)
        }
        .apply(&mut out, w, h, 3);
        out
    }

    fn px(p: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    }

    #[test]
    fn nothing_changes_at_zero_and_each_changes_a_picture() {
        let (w, h) = (160, 90);
        let pic = noisy(w, h);
        for kind in KINDS {
            assert_eq!(run(kind, 0.0, &pic, w, h), pic, "{kind:?} at zero");
            let mut subject = pic.clone();
            // The strokes need a shape: a clear picture with an opaque square.
            if matches!(kind, EffectKind::Stroke | EffectKind::StrokeInside) {
                subject = vec![0u8; w * h * 4];
                for y in 20..60 {
                    for x in 50..110 {
                        subject[(y * w + x) * 4..(y * w + x) * 4 + 4]
                            .copy_from_slice(&[200, 80, 40, 255]);
                    }
                }
            }
            let out = run(kind, 100.0, &subject, w, h);
            assert_ne!(out, subject, "{kind:?} changed nothing");
        }
    }

    #[test]
    fn a_coloured_border_frames_the_picture_and_leaves_the_middle() {
        let (w, h) = (960, 540);
        let pic = solid(w, h, [10, 10, 10]);
        let out = run(EffectKind::ColoredBorder, 50.0, &pic, w, h);
        assert_eq!(px(&out, w, 0, 270)[..3], [0xf2, 0xb1, 0x34]);
        assert_eq!(px(&out, w, 480, 0)[..3], [0xf2, 0xb1, 0x34]);
        assert_eq!(px(&out, w, 480, 270)[..3], [10, 10, 10]);
        // Rounded: the inner corner is rounder, so the picture shows in less of the corner.
        let round = run(EffectKind::ColoredBorderRounded, 50.0, &pic, w, h);
        assert_ne!(px(&round, w, 14, 14), px(&out, w, 14, 14));
    }

    #[test]
    fn chromatic_aberration_pulls_the_colours_apart_toward_the_corners() {
        let (w, h) = (960, 540);
        let mut sq = solid(w, h, [0, 0, 0]);
        for y in 20..80 {
            for x in 20..80 {
                sq[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(255);
            }
        }
        let out = run(EffectKind::ChromaticAberration, 100.0, &sq, w, h);
        // Near the square's inner edge the channels no longer agree.
        let differs = (15..90).any(|x| {
            let p = px(&out, w, x, 50);
            p[0] != p[2]
        });
        assert!(differs);
        // The middle of the picture is the same in all three.
        let mid = solid(w, h, [90, 120, 60]);
        let out = run(EffectKind::ChromaticAberration, 100.0, &mid, w, h);
        assert_eq!(px(&out, w, 480, 270)[..3], [90, 120, 60]);
    }

    #[test]
    fn edge_detect_finds_the_edge_and_the_inverse_is_its_opposite() {
        let (w, h) = (160, 90);
        let pic = step(w, h);
        let e = run(EffectKind::EdgeDetect, 100.0, &pic, w, h);
        assert!(px(&e, w, w / 2, 40)[0] > 200, "the edge is bright");
        assert_eq!(px(&e, w, 20, 40)[0], 0, "flat is dark");
        let inv = run(EffectKind::EdgeDetectInverted, 100.0, &pic, w, h);
        assert!(px(&inv, w, w / 2, 40)[0] < 50);
        assert_eq!(px(&inv, w, 20, 40)[0], 255);
    }

    #[test]
    fn emboss_is_grey_on_flat_and_lit_on_one_side_of_an_edge() {
        let (w, h) = (160, 90);
        let flat = run(EffectKind::Emboss, 100.0, &solid(w, h, [90, 90, 90]), w, h);
        assert!(px(&flat, w, 20, 20)[0].abs_diff(128) <= 1);
        let e = run(EffectKind::Emboss, 100.0, &step(w, h), w, h);
        let (a, b) = (px(&e, w, w / 2 - 1, 40)[0], px(&e, w, w / 2, 40)[0]);
        assert!(a != 128 || b != 128, "{a} {b}");
    }

    #[test]
    fn a_stroke_borders_the_shape_outside_or_inside() {
        let (w, h) = (960, 540);
        let mut sq = vec![0u8; w * h * 4];
        for y in 200..340 {
            for x in 400..560 {
                sq[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[200, 80, 40, 255]);
            }
        }
        let out = run(EffectKind::Stroke, 50.0, &sq, w, h);
        let outer = px(&out, w, 399, 270);
        assert!(
            outer[3] > 200 && outer[0] > 200 && outer[1] > 200,
            "white outside the edge: {outer:?}"
        );
        assert_eq!(
            px(&out, w, 480, 270)[..3],
            [200, 80, 40],
            "the picture is untouched"
        );
        assert_eq!(
            px(&out, w, 380, 270)[3],
            0,
            "not further out than the width"
        );
        let inside = run(EffectKind::StrokeInside, 50.0, &sq, w, h);
        let rim = px(&inside, w, 401, 270);
        assert!(
            rim[0] > 200 && rim[1] > 200 && rim[3] == 255,
            "white rim inside: {rim:?}"
        );
        assert_eq!(px(&inside, w, 480, 270)[..3], [200, 80, 40]);
    }

    #[test]
    fn watercolor_flattens_texture_and_stylize_makes_flat_looks() {
        let (w, h) = (480, 270);
        let pic = noisy(w, h);
        let roughness = |p: &[u8]| {
            let total: u64 = (0..h)
                .flat_map(|y| (0..w - 1).map(move |x| (x, y)))
                .map(|(x, y)| u64::from(p[(y * w + x) * 4].abs_diff(p[(y * w + x + 1) * 4])))
                .sum();
            total / ((w - 1) * h) as u64
        };
        let wc = run(EffectKind::Watercolor, 100.0, &pic, w, h);
        assert!(
            roughness(&wc) < roughness(&pic),
            "{} vs {}",
            roughness(&wc),
            roughness(&pic)
        );
        // Cutout has only a few levels per channel.
        let cut = run(EffectKind::StylizeCutout, 100.0, &pic, w, h);
        let levels = cut
            .chunks_exact(4)
            .map(|p| p[0])
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert!(levels <= 6, "{levels} levels");
        // Ink draws dark lines at an edge.
        let w = 160;
        let ink = run(EffectKind::StylizeInk, 100.0, &step(160, 90), 160, 90);
        let darkest = (w / 2 - 4..w / 2 + 4)
            .map(|x| px(&ink, w, x, 40)[0])
            .min()
            .unwrap_or(255);
        assert!(darkest < 40, "{darkest}");
        let relief = run(EffectKind::StylizeRelief, 100.0, &step(160, 90), 160, 90);
        assert_ne!(relief, step(160, 90));
    }

    #[test]
    fn they_keep_clear_pixels_clear_where_they_should_and_work_on_tiny_pictures() {
        let (w, h) = (160, 90);
        let mut clear = noisy(w, h);
        clear.chunks_exact_mut(4).take(w).for_each(|p| p[3] = 0);
        for kind in [
            EffectKind::Watercolor,
            EffectKind::StylizeRelief,
            EffectKind::StylizeInk,
            EffectKind::StylizeCutout,
        ] {
            let out = run(kind, 100.0, &clear, w, h);
            assert!(out.chunks_exact(4).take(w).all(|p| p[3] == 0), "{kind:?}");
        }
        for kind in KINDS {
            for (w, h) in [(1usize, 1usize), (2, 3), (5, 1)] {
                let mut p = vec![140u8; w * h * 4];
                Effect {
                    amount: 100.0,
                    ..Effect::new(kind)
                }
                .apply(&mut p, w, h, 2);
                assert_eq!(p.len(), w * h * 4);
            }
        }
    }
}
