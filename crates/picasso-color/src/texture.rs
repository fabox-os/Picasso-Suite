//! Texture effects: Fast Noise, Texture Pop, Detail Recovery, Analog Damage, Film Damage and
//! JPEG Damage.
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100, that scales
//! its settings from nothing (0) through their usual values (50) to twice as much (100). Sizes
//! are in thousandths of the picture's height. All randomness comes from an integer hash of the
//! pixel and the frame number, so the same frame draws the same way every time and an export
//! repeats. The time-based ones follow the clip's frame number.

use crate::fx::{gaussian, EffectKind};
use crate::look::{par_pixels, Src};

/// Pixels of one 1080-line unit in a picture `h` lines high.
fn unit(h: usize) -> f32 {
    h as f32 / 1080.0
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// A number from 0 to 1 for `x`.
fn hash01(x: u32) -> f32 {
    (hash32(x) >> 8) as f32 / 16_777_215.0
}

/// A number from 0 to 1 for the cell (`x`, `y`) in generation `f`.
fn hash2(x: i32, y: i32, f: u32) -> f32 {
    hash01(
        (x as u32).wrapping_mul(0x8da6_b343)
            ^ (y as u32).wrapping_mul(0xd816_3841)
            ^ f.wrapping_mul(0xcb1a_b31f),
    )
}

/// The colour of a premultiplied sample, straight.
fn straight(q: [f32; 4]) -> [f32; 3] {
    if q[3] <= 1e-6 {
        return [0.0; 3];
    }
    [q[0] / q[3], q[1] / q[3], q[2] / q[3]]
}

/// Writes a straight colour into a pixel, keeping its alpha.
fn set_rgb(px: &mut [u8], c: [f32; 3]) {
    for i in 0..3 {
        px[i] = (c[i] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
}

fn rgb_of(px: &[u8]) -> [f32; 3] {
    [0, 1, 2].map(|i| f32::from(px[i]) / 255.0)
}

// ---- Fast Noise ----

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i32, fy as i32);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (smooth(x - fx), smooth(y - fy));
    let at = |dx: i32, dy: i32| hash2(ix + dx, iy + dy, seed);
    mix(mix(at(0, 0), at(1, 0), tx), mix(at(0, 1), at(1, 1), tx), ty)
}

fn fractal(x: f32, y: f32, octaves: u32) -> f32 {
    let (mut sum, mut norm, mut amp, mut freq) = (0.0, 0.0, 1.0, 1.0);
    for o in 0..octaves {
        let off = 37.0 * o as f32;
        sum += amp * value_noise(x * freq + off, y * freq + off, 1 + o);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// Cloudy noise laid over the picture with the Overlay blend; the strength is the blend.
fn fast_noise(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let scale = (200.0 * unit(h)).max(2.0);
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let n = fractal((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale, 4);
        let t = ((n - 0.5) * 2.0 + 0.5).clamp(0.0, 1.0);
        let c = rgb_of(px);
        let o = c.map(|c| {
            let overlay = if c < 0.5 {
                2.0 * c * t
            } else {
                1.0 - 2.0 * (1.0 - c) * (1.0 - t)
            };
            mix(c, overlay, a)
        });
        set_rgb(px, o);
    });
}

// ---- Texture Pop and Detail Recovery ----

/// Makes the fine texture stand out: the difference between the picture and a blur of it is
/// added back, less in the highlights and at black.
fn texture_pop(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let gain = a * 2.0;
    let mut blurred = rgba.to_vec();
    gaussian(
        &mut blurred,
        w,
        h,
        (2.0 * unit(h)).round().max(1.0) as usize,
    );
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let i = (y * w + x) * 4;
        let c = rgb_of(px);
        let yc = luma(c);
        let yb = luma(rgb_of(&blurred[i..i + 4]));
        let weight = (1.0 - 0.4 * smoothstep(0.55, 1.0, yc)) * smoothstep(0.0, 0.08, yc);
        let d = gain * weight * (yc - yb);
        set_rgb(px, c.map(|v| (v + d).clamp(0.0, 1.0)));
    });
}

/// Lifts the deep shadows and tames the bright highlights of the slowly changing tone and
/// keeps and strengthens the fast detail on top of it, so what was crushed shows again.
fn detail_recovery(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let (shadows, highlights, detail) = (a * 0.8, a * 0.8, a * 0.8);
    let mut blurred = rgba.to_vec();
    gaussian(
        &mut blurred,
        w,
        h,
        (30.0 * unit(h)).round().max(1.0) as usize,
    );
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let i = (y * w + x) * 4;
        let c = rgb_of(px);
        let yc = luma(c);
        let yb = luma(rgb_of(&blurred[i..i + 4]));
        let lift = shadows * (1.0 - smoothstep(0.0, 0.5, yb)) * (0.5 - yb) * 0.8;
        let drop = highlights * smoothstep(0.5, 1.0, yb) * (yb - 0.5) * 0.8;
        let yo = yb + lift - drop + (yc - yb) * (1.0 + detail);
        let d = yo - yc;
        set_rgb(px, c.map(|v| (v + d).clamp(0.0, 1.0)));
    });
}

// ---- Analog Damage ----

fn to_ycc(c: [f32; 3]) -> [f32; 3] {
    let y = luma(c);
    [y, (c[2] - y) / 1.8556, (c[0] - y) / 1.5748]
}

fn from_ycc(v: [f32; 3]) -> [f32; 3] {
    let r = v[0] + 1.5748 * v[2];
    let b = v[0] + 1.8556 * v[1];
    [r, (v[0] - 0.2126 * r - 0.0722 * b) / 0.7152, b]
}

/// Worn videotape: line jitter, colour bleed, noise, a tracking error band and scanlines.
fn analog_damage(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let k = a * 2.0;
    let u = unit(h);
    let jitter = 6.0 * k * u;
    let bleed = 8.0 * k * u;
    let noise = (0.2 * k).min(1.0);
    let band = (0.25 * k).min(1.0) * 0.35;
    let scanlines = (0.2 * k).min(1.0);
    let line_h = (2.0 * u).max(1.0);
    let fr = frame as u32;
    let seed = 1u32;
    let fs = fr
        .wrapping_mul(7919)
        .wrapping_add(seed.wrapping_mul(104_729));
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let hf = h as f32;
    par_pixels(rgba, w, h, &|x, y, px| {
        let (pxf, pyf) = (x as f32 + 0.5, y as f32 + 0.5);
        let line = (pyf / line_h).floor();
        let li = line as u32;
        let mut off = jitter * 0.35 * (line * 0.045 + fr as f32 * 0.9 + seed as f32).sin();
        if hash01(li.wrapping_mul(2_654_435_761) ^ fs) > 0.96 {
            off += jitter * (hash01(li.wrapping_mul(40_503) ^ fs) - 0.5) * 3.0;
        }
        let depth = if band > 0.0 {
            ((pyf / hf - (1.0 - band)) / band).clamp(0.0, 1.0)
        } else {
            0.0
        };
        off += jitter * 3.0 * depth * depth * (0.5 + hash01(li.wrapping_mul(977) ^ fs));
        let q = src.at(pxf + off, pyf);
        if q[3] <= 0.0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        let mut ycc = to_ycc(straight(q));
        if bleed > 0.0 {
            let mut cc = [0.0f32; 2];
            for i in 0..7 {
                let s = to_ycc(straight(src.at(pxf + off - bleed * i as f32 / 6.0, pyf)));
                cc[0] += s[1];
                cc[1] += s[2];
            }
            ycc[1] = cc[0] / 7.0;
            ycc[2] = cc[1] / 7.0;
        }
        let n = hash2(x as i32, y as i32, fr.wrapping_add(seed.wrapping_mul(131))) - 0.5;
        ycc[0] += n * noise * 0.4;
        let cn = hash2(
            (x / 3) as i32,
            (y / 3) as i32,
            fr.wrapping_add(7).wrapping_add(seed.wrapping_mul(131)),
        ) - 0.5;
        ycc[1] += cn * noise * 0.15;
        ycc[2] += (hash2((y / 3) as i32, (x / 3) as i32, fr.wrapping_add(8)) - 0.5) * noise * 0.15;
        if depth > 0.0 && hash01(li.wrapping_mul(6151) ^ fs) < depth * 0.5 {
            let white = hash2(x as i32, y as i32, fr.wrapping_add(99));
            ycc = [
                mix(ycc[0], white, 0.6 * depth),
                mix(ycc[1], 0.0, 0.6 * depth),
                mix(ycc[2], 0.0, 0.6 * depth),
            ];
        }
        let mut o = from_ycc(ycc);
        let dim = 1.0 - scanlines * 0.45 * (line % 2.0);
        o = o.map(|v| (v * dim).clamp(0.0, 1.0));
        set_rgb(px, o);
        px[3] = (q[3] * 255.0 + 0.5) as u8;
    });
}

// ---- Film Damage ----

/// Old film: dirt, scratches, a soft dark vignette, gate weave and flicker.
fn film_damage(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let k = a * 2.0;
    let u = unit(h);
    let dirt_amount = (0.3 * k).min(1.0);
    let scratches = (0.25 * k).min(1.0);
    let vignette = (0.25 * k).min(1.0);
    let weave = 1.5 * k * u;
    let flicker = (0.05 * k).min(0.5);
    let soft = 0.8 * k * u;
    let fr = frame as u32;
    let seed = 1u32;
    let mut softened = rgba.to_vec();
    if soft >= 0.5 {
        gaussian(&mut softened, w, h, (soft * 2.0).round().max(1.0) as usize);
    }
    let src = Src::new(&softened, w, h);
    let (wf, hf) = (w as f32, h as f32);
    let sc = u.max(0.5);
    let wv = [
        (hash01(fr.wrapping_mul(2).wrapping_add(seed.wrapping_mul(977))) - 0.5) * 2.0 * weave,
        (hash01(
            fr.wrapping_mul(2)
                .wrapping_add(1)
                .wrapping_add(seed.wrapping_mul(977)),
        ) - 0.5)
            * 2.0
            * weave,
    ];
    let gain = 1.0
        + (hash01(
            fr.wrapping_mul(31)
                .wrapping_add(seed.wrapping_mul(131))
                .wrapping_add(5),
        ) - 0.5)
            * 2.0
            * flicker;
    let cell = 24.0 * sc;
    let ff = fr / 2;
    let sf = fr / 3;
    par_pixels(rgba, w, h, &|x, y, px| {
        let (pxf, pyf) = (x as f32 + 0.5, y as f32 + 0.5);
        let q = src.at(pxf + wv[0], pyf + wv[1]);
        if q[3] <= 0.0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        let mut c = straight(q).map(|v| v * gain);
        let d = [(pxf / wf - 0.5) * 2.0, (pyf / hf - 0.5) * 2.0];
        let r = d[0].hypot(d[1]) * std::f32::consts::FRAC_1_SQRT_2;
        c = c.map(|v| v * (1.0 - vignette * smoothstep(0.35, 1.0, r)));
        // Dirt: a grid of cells, some holding one speck; specks stay for two frames.
        let (cx, cy) = ((pxf / cell).floor() as i32, (pyf / cell).floor() as i32);
        let mut dirt = 1.0f32;
        let mut bright = 0.0f32;
        for j in -1..=1 {
            for i in -1..=1 {
                let (ex, ey) = (cx + i, cy + j);
                if hash2(
                    ex,
                    ey,
                    ff.wrapping_mul(5).wrapping_add(seed.wrapping_mul(131)),
                ) >= dirt_amount * 0.15
                {
                    continue;
                }
                let centre = [
                    (ex as f32 + hash2(ex, ey, ff.wrapping_add(11).wrapping_add(seed))) * cell,
                    (ey as f32 + hash2(ex, ey, ff.wrapping_add(13).wrapping_add(seed))) * cell,
                ];
                let rad = (0.7 + 2.3 * hash2(ex, ey, ff.wrapping_add(17).wrapping_add(seed))) * sc;
                let cov =
                    1.0 - smoothstep(0.55 * rad, rad, (pxf - centre[0]).hypot(pyf - centre[1]));
                if hash2(ex, ey, ff.wrapping_add(19).wrapping_add(seed)) < 0.2 {
                    bright = bright.max(cov);
                } else {
                    dirt *= 1.0 - 0.9 * cov;
                }
            }
        }
        c = c.map(|v| mix(v * dirt, 0.92, bright * 0.85));
        // Scratches: thin vertical lines, kept for three frames.
        for i in 0..5u32 {
            let id = sf
                .wrapping_mul(8)
                .wrapping_add(i)
                .wrapping_add(seed.wrapping_mul(977));
            if hash01(id.wrapping_mul(3).wrapping_add(1)) >= scratches {
                continue;
            }
            let sx = hash01(id.wrapping_mul(3).wrapping_add(2)) * wf;
            let y0 = hash01(id.wrapping_mul(3).wrapping_add(3)) * hf * 0.5;
            let y1 = y0 + (0.3 + 0.7 * hash01(id.wrapping_mul(5).wrapping_add(7))) * hf;
            let wob = 0.6 * sc * (pyf * 0.03 + i as f32).sin();
            let cov = (1.0 - smoothstep(0.2 * sc, 1.1 * sc, (pxf - sx - wob).abs()))
                * if pyf >= y0 && pyf <= y1 { 1.0 } else { 0.0 };
            c = c.map(|v| mix(v, 0.9, 0.7 * cov));
        }
        set_rgb(px, c.map(|v| v.clamp(0.0, 1.0)));
        px[3] = (q[3] * 255.0 + 0.5) as u8;
    });
}

// ---- JPEG Damage ----

/// Recompresses the picture as a badly compressed JPEG: blocks, banding and blotchy colour.
fn jpeg_damage(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let quality = (100.0 - a * 140.0).clamp(1.0, 100.0);
    if quality >= 100.0 {
        return;
    }
    let q = 1.0 - quality / 100.0;
    let qy = 0.002 + 0.12 * q * q;
    let qc = qy * 1.5;
    let block = (8.0 * unit(h)).max(2.0);
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let mean = |ox: f32, oy: f32, size: f32| -> [f32; 3] {
        let mut m = [0.0f32; 3];
        for j in 0..4 {
            for i in 0..4 {
                let s = straight(src.at(
                    ox + (i as f32 + 0.5) * size * 0.25,
                    oy + (j as f32 + 0.5) * size * 0.25,
                ));
                for c in 0..3 {
                    m[c] += s[c];
                }
            }
        }
        m.map(|v| v / 16.0)
    };
    let round_to = |v: f32, q: f32| (v / q + 0.5).floor() * q;
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let (pxf, pyf) = (x as f32 + 0.5, y as f32 + 0.5);
        let c = rgb_of(px);
        let m = mean(
            (pxf / block).floor() * block,
            (pyf / block).floor() * block,
            block,
        );
        let big = 2.0 * block;
        let cm = mean((pxf / big).floor() * big, (pyf / big).floor() * big, big);
        let (yc, my) = (luma(c), luma(m));
        let yq = round_to(my, qy * 1.5) + round_to(yc - my, qy);
        let ycm = luma(cm);
        let cb = round_to((cm[2] - ycm) / 1.8556, qc);
        let cr = round_to((cm[0] - ycm) / 1.5748, qc);
        let o = from_ycc([yq, cb, cr]);
        set_rgb(px, o.map(|v| v.clamp(0.0, 1.0)));
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize, frame: u64) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    match kind {
        EffectKind::FastNoise => fast_noise(rgba, w, h, a),
        EffectKind::TexturePop => texture_pop(rgba, w, h, a),
        EffectKind::DetailRecovery => detail_recovery(rgba, w, h, a),
        EffectKind::AnalogDamage => analog_damage(rgba, w, h, a, frame),
        EffectKind::FilmDamage => film_damage(rgba, w, h, a, frame),
        EffectKind::JpegDamage => jpeg_damage(rgba, w, h, a),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 160;
    const H: usize = 90;

    const KINDS: [EffectKind; 6] = [
        EffectKind::FastNoise,
        EffectKind::TexturePop,
        EffectKind::DetailRecovery,
        EffectKind::AnalogDamage,
        EffectKind::FilmDamage,
        EffectKind::JpegDamage,
    ];

    /// A smooth picture with a few edges, opaque.
    fn picture() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let edge = if (x / 20 + y / 15) % 2 == 0 { 40 } else { 0 };
                v.extend_from_slice(&[
                    (x * 255 / W) as u8,
                    (y * 255 / H) as u8,
                    (100 + edge) as u8,
                    255,
                ]);
            }
        }
        v
    }

    fn flat(g: u8) -> Vec<u8> {
        (0..W * H).flat_map(|_| [g, g, g, 255]).collect()
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

    fn diff(a: &[u8], b: &[u8]) -> u64 {
        a.iter()
            .zip(b)
            .map(|(x, y)| u64::from(x.abs_diff(*y)))
            .sum()
    }

    #[test]
    fn none_of_them_changes_a_picture_at_zero_and_all_change_it_at_the_usual_strength() {
        let pic = picture();
        for kind in KINDS {
            assert_eq!(run(kind, 0.0, &pic, 3), pic, "{kind:?} at zero");
            assert!(
                diff(&run(kind, kind_default(kind), &pic, 3), &pic) > 0,
                "{kind:?}"
            );
        }
    }

    fn kind_default(kind: EffectKind) -> f32 {
        kind.default_amount()
    }

    #[test]
    fn they_keep_the_size_and_the_alpha_of_a_clear_picture() {
        let mut pic = picture();
        for px in pic.chunks_exact_mut(4).take(W * 3) {
            px[3] = 0;
        }
        for kind in KINDS {
            let out = run(kind, 80.0, &pic, 5);
            assert_eq!(out.len(), pic.len());
            assert!(
                out[..W * 3 * 4].chunks_exact(4).all(|p| p[3] == 0),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn they_work_on_tiny_pictures() {
        for kind in KINDS {
            for (w, h) in [(1usize, 1usize), (2, 3), (5, 1)] {
                let mut p = vec![120u8; w * h * 4];
                Effect {
                    amount: 100.0,
                    ..Effect::new(kind)
                }
                .apply(&mut p, w, h, 2);
                assert_eq!(p.len(), w * h * 4);
            }
        }
    }

    #[test]
    fn the_same_frame_draws_the_same_way_and_the_next_one_differently() {
        let pic = picture();
        for kind in [EffectKind::AnalogDamage, EffectKind::FilmDamage] {
            let a = run(kind, 100.0, &pic, 4);
            assert_eq!(a, run(kind, 100.0, &pic, 4), "{kind:?}");
            assert_ne!(a, run(kind, 100.0, &pic, 5), "{kind:?}");
        }
    }

    #[test]
    fn fast_noise_makes_a_flat_grey_cloudy_but_leaves_black_alone() {
        let out = run(EffectKind::FastNoise, 100.0, &flat(128), 0);
        let (lo, hi) = out
            .chunks_exact(4)
            .fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
        assert!(hi - lo > 40, "{lo}..{hi}");
        assert_eq!(run(EffectKind::FastNoise, 100.0, &flat(0), 0), flat(0));
    }

    #[test]
    fn texture_pop_adds_contrast_to_fine_detail_only() {
        let mut fine = flat(100);
        for (i, px) in fine.chunks_exact_mut(4).enumerate() {
            if (i % W + i / W).is_multiple_of(2) {
                px[..3].fill(130);
            }
        }
        let spread = |p: &[u8]| {
            let (lo, hi) = p
                .chunks_exact(4)
                .fold((255u8, 0u8), |(lo, hi), q| (lo.min(q[0]), hi.max(q[0])));
            hi - lo
        };
        assert!(spread(&run(EffectKind::TexturePop, 100.0, &fine, 0)) > spread(&fine));
        // A flat area has no texture: it stays as it is.
        assert_eq!(run(EffectKind::TexturePop, 100.0, &flat(100), 0), flat(100));
    }

    #[test]
    fn detail_recovery_lifts_shadows_and_tames_highlights() {
        let dark = run(EffectKind::DetailRecovery, 100.0, &flat(20), 0);
        assert!(dark[0] > 20);
        let bright = run(EffectKind::DetailRecovery, 100.0, &flat(240), 0);
        assert!(bright[0] < 240);
        let mid = run(EffectKind::DetailRecovery, 100.0, &flat(128), 0);
        assert!(mid[0].abs_diff(128) <= 2);
    }

    #[test]
    fn analog_damage_adds_scanlines_and_film_damage_darkens_the_corners() {
        let out = run(EffectKind::AnalogDamage, 100.0, &flat(200), 1);
        // Scanlines make neighbouring lines differ.
        let row = |y: usize| out[(y * W + 5) * 4] as i32;
        assert!((0..H - 4).any(|y| (row(y) - row(y + 2)).abs() > 10));
        let film = run(EffectKind::FilmDamage, 100.0, &flat(200), 0);
        let c = film[(H / 2 * W + W / 2) * 4];
        let corner = film[(2 * W + 2) * 4];
        assert!(corner < c);
    }

    #[test]
    fn jpeg_damage_makes_blocks_that_a_gradient_did_not_have() {
        let mut ramp = Vec::new();
        for _ in 0..H {
            for x in 0..W {
                let v = (x * 255 / W) as u8;
                ramp.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let out = run(EffectKind::JpegDamage, 100.0, &ramp, 0);
        let mut distinct = std::collections::BTreeSet::new();
        for x in 0..W {
            distinct.insert(out[(10 * W + x) * 4]);
        }
        assert!(distinct.len() < 60, "{} levels", distinct.len());
        // Higher quality keeps the picture closer.
        assert!(diff(&run(EffectKind::JpegDamage, 20.0, &ramp, 0), &ramp) < diff(&out, &ramp));
    }
}
