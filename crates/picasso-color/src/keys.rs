//! More keyers: chroma keys for screens of other colours, HSL keys for each main hue, 3D keys
//! that take out colours in colour space, and Matte Shrink, Matte Grow and Matte Soften for the
//! edge of a keyed (or any transparent) picture.
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100. For the keys
//! it is how wide a range is taken out, from nothing (0) through the usual width (50) to twice
//! that (100); for the matte effects it is how far the edge moves or softens. Sizes are in
//! thousandths of the picture's height.

use crate::fx::EffectKind;
use crate::key::{apply_chroma_key, ChromaKey};
use crate::look::par_pixels;

fn unit(h: usize) -> f32 {
    h as f32 / 1080.0
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn rgb_of(px: &[u8]) -> [f32; 3] {
    [0, 1, 2].map(|i| f32::from(px[i]) / 255.0)
}

/// Multiplies the pixel's alpha by `m`.
fn keep(px: &mut [u8], m: f32) {
    px[3] = (f32::from(px[3]) * m.clamp(0.0, 1.0) + 0.5) as u8;
}

// ---- HSL key ----

fn to_hsl(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let l = 0.5 * (mx + mn);
    let s = if d <= 1e-6 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6)
    };
    let mut h = 0.0;
    if d > 1e-6 {
        h = if mx == c[0] {
            ((c[1] - c[2]) / d + 6.0) % 6.0
        } else if mx == c[1] {
            (c[2] - c[0]) / d + 2.0
        } else {
            (c[0] - c[1]) / d + 4.0
        };
    }
    [h * 60.0, s, l]
}

fn range_mask(low: f32, high: f32, soft: f32, x: f32) -> f32 {
    smoothstep(low - soft, low, x) * (1.0 - smoothstep(high, high + soft, x))
}

/// Takes out a range of hues (width `width` degrees around `hue`), saturated enough and not too
/// dark or light to be a grey.
fn hsl_key(rgba: &mut [u8], w: usize, h: usize, hue: f32, a: f32) {
    let half = 0.5 * (a * 2.0 * 60.0).min(360.0);
    par_pixels(rgba, w, h, &|_, _, px| {
        if px[3] == 0 {
            return;
        }
        let [ph, s, l] = to_hsl(rgb_of(px));
        let dh = ((ph - hue + 540.0) % 360.0 - 180.0).abs();
        let qh = 1.0 - smoothstep(half, half + 20.0, dh);
        let m = 1.0 - qh * range_mask(0.2, 1.0, 0.1, s) * range_mask(0.0, 1.0, 0.1, l);
        keep(px, m);
    });
}

// ---- 3D keyer ----

/// Takes out colours near the given ones in a space of chroma and brightness.
fn keyer_3d(rgba: &mut [u8], w: usize, h: usize, keys: &[[f32; 3]], a: f32) {
    let k = a * 2.0;
    let tolerance = 0.125 * k;
    let softness = (0.075 * k).max(1e-3);
    let weight = 0.3f32.sqrt();
    let space = move |c: [f32; 3]| {
        let y = luma(c);
        [(c[2] - y) / 1.8556, (c[0] - y) / 1.5748, y * weight]
    };
    let keys: Vec<[f32; 3]> = keys.iter().map(|c| space(*c)).collect();
    par_pixels(rgba, w, h, &|_, _, px| {
        if px[3] == 0 {
            return;
        }
        let v = space(rgb_of(px));
        let d = keys
            .iter()
            .map(|k| ((v[0] - k[0]).powi(2) + (v[1] - k[1]).powi(2) + (v[2] - k[2]).powi(2)).sqrt())
            .fold(f32::MAX, f32::min);
        keep(px, smoothstep(tolerance, tolerance + softness, d));
    });
}

// ---- Matte edge ----

/// A running min or max over a window of `2r + 1` along rows (`rows`) or columns.
fn window(buf: &[f32], w: usize, h: usize, r: usize, rows: bool, grow: bool) -> Vec<f32> {
    let mut out = vec![0.0f32; buf.len()];
    let (len, lines) = if rows { (w, h) } else { (h, w) };
    for line in 0..lines {
        let at = |i: usize| if rows { line * w + i } else { i * w + line };
        for i in 0..len {
            let (lo, hi) = (i.saturating_sub(r), (i + r).min(len - 1));
            let mut v = buf[at(lo)];
            for j in lo + 1..=hi {
                let b = buf[at(j)];
                v = if grow { v.max(b) } else { v.min(b) };
            }
            out[at(i)] = v;
        }
    }
    out
}

/// Moves the edge of the alpha out (grow) or in by `r` pixels.
pub(crate) fn morph_plane(plane: &[f32], w: usize, h: usize, r: usize, grow: bool) -> Vec<f32> {
    let rows = window(plane, w, h, r, true, grow);
    window(&rows, w, h, r, false, grow)
}

/// Softens the alpha with a blur close to a Gaussian (three box blurs).
pub(crate) fn blur_plane(plane: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut cur = plane.to_vec();
    for _ in 0..3 {
        for rows in [true, false] {
            let (len, lines) = if rows { (w, h) } else { (h, w) };
            let src = cur.clone();
            for line in 0..lines {
                let at = |i: i64| {
                    let i = i.clamp(0, len as i64 - 1) as usize;
                    if rows {
                        line * w + i
                    } else {
                        i * w + line
                    }
                };
                let width = (2 * r + 1) as f32;
                let mut sum: f32 = (-(r as i64)..=r as i64).map(|j| src[at(j)]).sum();
                for i in 0..len as i64 {
                    cur[at(i)] = sum / width;
                    sum += src[at(i + r as i64 + 1)] - src[at(i - r as i64)];
                }
            }
        }
    }
    cur
}

#[derive(Clone, Copy)]
enum Matte {
    Shrink,
    Grow,
    Soften,
}

/// Moves or softens the edge of the transparency. Where the picture grows into clear pixels, the
/// colour comes from the nearest picture (the strongest of the colours around), never from black.
fn matte_edge(rgba: &mut [u8], w: usize, h: usize, a: f32, kind: Matte) {
    let k = a * 2.0;
    let px_amount = 4.0 * unit(h) * k;
    let r = px_amount.round() as usize;
    if r == 0 {
        return;
    }
    let alpha: Vec<f32> = rgba
        .chunks_exact(4)
        .map(|p| f32::from(p[3]) / 255.0)
        .collect();
    let matte = match kind {
        Matte::Shrink => morph_plane(&alpha, w, h, r, false),
        Matte::Grow => morph_plane(&alpha, w, h, r, true),
        Matte::Soften => blur_plane(&alpha, w, h, r),
    };
    // Premultiplied colour planes, dilated, for pixels the picture grows into.
    let grown: Option<[Vec<f32>; 3]> = matches!(kind, Matte::Grow).then(|| {
        [0, 1, 2].map(|c| {
            let plane: Vec<f32> = rgba
                .chunks_exact(4)
                .zip(&alpha)
                .map(|(p, a)| f32::from(p[c]) / 255.0 * a)
                .collect();
            morph_plane(&plane, w, h, r, true)
        })
    });
    let dilated_alpha = grown.as_ref().map(|_| morph_plane(&alpha, w, h, r, true));
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let m = matte[i];
        if let (Some(g), Some(da)) = (&grown, &dilated_alpha) {
            if px[3] == 0 && da[i] > 1e-6 {
                for c in 0..3 {
                    px[c] = (g[c][i] / da[i] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                }
            }
        }
        px[3] = (m * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    let plain = |screen: [f32; 3]| ChromaKey::plain(screen, a);
    match kind {
        EffectKind::RedScreen => {
            apply_chroma_key(rgba, &plain([220.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0]))
        }
        EffectKind::MagentaScreen => apply_chroma_key(rgba, &plain([1.0, 0.0, 1.0])),
        EffectKind::CyanScreen => apply_chroma_key(rgba, &plain([0.0, 1.0, 1.0])),
        EffectKind::YellowScreen => apply_chroma_key(rgba, &plain([1.0, 1.0, 0.0])),
        EffectKind::HslKeyRed => hsl_key(rgba, w, h, 0.0, a),
        EffectKind::HslKeyYellow => hsl_key(rgba, w, h, 60.0, a),
        EffectKind::HslKeyGreen => hsl_key(rgba, w, h, 120.0, a),
        EffectKind::HslKeyCyan => hsl_key(rgba, w, h, 180.0, a),
        EffectKind::HslKeyBlue => hsl_key(rgba, w, h, 240.0, a),
        EffectKind::HslKeyMagenta => hsl_key(rgba, w, h, 300.0, a),
        EffectKind::Keyer3dGreen => keyer_3d(rgba, w, h, &[crate::key::GREEN_SCREEN], a),
        EffectKind::Keyer3dBlue => keyer_3d(rgba, w, h, &[crate::key::BLUE_SCREEN], a),
        EffectKind::Keyer3dGreenAndBlue => keyer_3d(
            rgba,
            w,
            h,
            &[crate::key::GREEN_SCREEN, crate::key::BLUE_SCREEN],
            a,
        ),
        EffectKind::Keyer3dRed => keyer_3d(
            rgba,
            w,
            h,
            &[[220.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0]],
            a,
        ),
        EffectKind::MatteShrink => matte_edge(rgba, w, h, a, Matte::Shrink),
        EffectKind::MatteGrow => matte_edge(rgba, w, h, a, Matte::Grow),
        EffectKind::MatteSoften => matte_edge(rgba, w, h, a, Matte::Soften),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 120;
    const H: usize = 80;

    const KEYS: [EffectKind; 14] = [
        EffectKind::RedScreen,
        EffectKind::MagentaScreen,
        EffectKind::CyanScreen,
        EffectKind::YellowScreen,
        EffectKind::HslKeyRed,
        EffectKind::HslKeyYellow,
        EffectKind::HslKeyGreen,
        EffectKind::HslKeyCyan,
        EffectKind::HslKeyBlue,
        EffectKind::HslKeyMagenta,
        EffectKind::Keyer3dGreen,
        EffectKind::Keyer3dBlue,
        EffectKind::Keyer3dGreenAndBlue,
        EffectKind::Keyer3dRed,
    ];
    const MATTES: [EffectKind; 3] = [
        EffectKind::MatteShrink,
        EffectKind::MatteGrow,
        EffectKind::MatteSoften,
    ];

    fn solid(rgb: [u8; 3]) -> Vec<u8> {
        (0..W * H)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    fn run(kind: EffectKind, amount: f32, pic: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut out = pic.to_vec();
        Effect {
            amount,
            ..Effect::new(kind)
        }
        .apply(&mut out, w, h, 0);
        out
    }

    fn alpha_of(kind: EffectKind, rgb: [u8; 3]) -> u8 {
        run(kind, 60.0, &solid(rgb), W, H)[3]
    }

    #[test]
    fn each_key_takes_out_its_colour_and_keeps_a_grey() {
        let table: [(EffectKind, [u8; 3]); 14] = [
            (EffectKind::RedScreen, [220, 40, 40]),
            (EffectKind::MagentaScreen, [255, 0, 255]),
            (EffectKind::CyanScreen, [0, 255, 255]),
            (EffectKind::YellowScreen, [255, 255, 0]),
            (EffectKind::HslKeyRed, [230, 20, 20]),
            (EffectKind::HslKeyYellow, [230, 230, 20]),
            (EffectKind::HslKeyGreen, [20, 200, 20]),
            (EffectKind::HslKeyCyan, [20, 220, 220]),
            (EffectKind::HslKeyBlue, [20, 20, 220]),
            (EffectKind::HslKeyMagenta, [220, 20, 220]),
            (EffectKind::Keyer3dGreen, [0, 177, 64]),
            (EffectKind::Keyer3dBlue, [0, 71, 187]),
            (EffectKind::Keyer3dGreenAndBlue, [0, 71, 187]),
            (EffectKind::Keyer3dRed, [220, 40, 40]),
        ];
        for (kind, colour) in table {
            assert_eq!(alpha_of(kind, colour), 0, "{kind:?} keeps its own colour");
            assert_eq!(
                alpha_of(kind, [128, 128, 128]),
                255,
                "{kind:?} keeps a grey"
            );
        }
    }

    #[test]
    fn a_key_leaves_the_other_hues_alone() {
        assert_eq!(alpha_of(EffectKind::HslKeyGreen, [220, 20, 20]), 255);
        assert_eq!(alpha_of(EffectKind::HslKeyRed, [20, 200, 20]), 255);
        assert_eq!(alpha_of(EffectKind::Keyer3dGreen, [220, 40, 40]), 255);
        assert_eq!(alpha_of(EffectKind::RedScreen, [0, 177, 64]), 255);
    }

    #[test]
    fn nothing_changes_at_zero_and_a_wider_strength_keys_more() {
        let pic = solid([60, 190, 40]);
        for kind in KEYS {
            assert_eq!(run(kind, 0.0, &pic, W, H), pic, "{kind:?} at zero");
        }
        // A green a little off the pure hue: a wider key takes it, a narrow one does not.
        let narrow = run(EffectKind::HslKeyGreen, 20.0, &solid([160, 190, 20]), W, H)[3];
        let wide = run(EffectKind::HslKeyGreen, 100.0, &solid([160, 190, 20]), W, H)[3];
        assert!(wide < narrow, "{wide} vs {narrow}");
    }

    fn area(p: &[u8]) -> u64 {
        p.chunks_exact(4).map(|q| u64::from(q[3])).sum()
    }

    #[test]
    fn the_matte_shrinks_grows_and_softens() {
        // 540 lines high: one unit is half a pixel, so a full-strength matte moves 4 pixels.
        let (hw, hh) = (1080usize, 540usize);
        let mut tall = vec![0u8; hw * hh * 4];
        for y in 200..340 {
            for x in 400..560 {
                tall[(y * hw + x) * 4..(y * hw + x) * 4 + 4].copy_from_slice(&[200, 90, 30, 255]);
            }
        }
        let shrink = run(EffectKind::MatteShrink, 100.0, &tall, hw, hh);
        let grow = run(EffectKind::MatteGrow, 100.0, &tall, hw, hh);
        let soft = run(EffectKind::MatteSoften, 100.0, &tall, hw, hh);
        assert!(area(&shrink) < area(&tall), "shrinks");
        assert!(area(&grow) > area(&tall), "grows");
        // Soft: a ramp of alpha at the edge, about as much area as before.
        let row: Vec<u8> = (390..420).map(|x| soft[(270 * hw + x) * 4 + 3]).collect();
        assert!(row.iter().any(|a| *a > 10 && *a < 245), "{row:?}");
        let (a0, a1) = (area(&soft) as f64, area(&tall) as f64);
        assert!((a0 - a1).abs() < a1 * 0.05);
        // The grown rim has the picture's colour, not black.
        let rim = &grow[(270 * hw + 398) * 4..(270 * hw + 398) * 4 + 4];
        assert!(rim[3] > 0 && rim[0] > 100, "{rim:?}");
    }

    #[test]
    fn they_work_on_tiny_pictures() {
        for kind in KEYS.into_iter().chain(MATTES) {
            for (w, h) in [(1usize, 1usize), (2, 3), (5, 1)] {
                let p = vec![120u8; w * h * 4];
                assert_eq!(run(kind, 100.0, &p, w, h).len(), w * h * 4);
            }
        }
    }
}
