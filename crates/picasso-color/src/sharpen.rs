//! Sharpen effects: Sharpen Edges and Soften and Sharpen.
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100, that scales
//! its settings from nothing (0) through their usual values (50) to twice as much (100). Sizes
//! are in thousandths of the picture's height.

use crate::fx::{gaussian, EffectKind};
use crate::look::par_pixels;

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

fn rgb_at(d: &[u8], i: usize) -> [f32; 3] {
    [0, 1, 2].map(|c| f32::from(d[i + c]) / 255.0)
}

/// Sharpens only the real edges: the difference from a blur is added back where a neighbour
/// differs enough in brightness, and the change at any pixel is capped (no halos).
fn sharpen_edges(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let strength = 1.0 * k;
    let radius = 1.5 * unit(h);
    let threshold = 0.1f32;
    let halo = 0.25f32;
    let spacing = radius.max(1.0).round() as i64;
    let mut blurred = rgba.to_vec();
    gaussian(&mut blurred, w, h, (2.0 * radius).round().max(1.0) as usize);
    let original = rgba.to_vec();
    let (wi, hi) = (w as i64, h as i64);
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let i = (y * w + x) * 4;
        let c = rgb_at(&original, i);
        let b = rgb_at(&blurred, i);
        let at = |dx: i64, dy: i64| {
            let xx = (x as i64 + dx).clamp(0, wi - 1) as usize;
            let yy = (y as i64 + dy).clamp(0, hi - 1) as usize;
            luma(rgb_at(&original, (yy * w + xx) * 4))
        };
        let yc = luma(c);
        let edge = [(spacing, 0), (-spacing, 0), (0, spacing), (0, -spacing)]
            .iter()
            .map(|(dx, dy)| (at(*dx, *dy) - yc).abs())
            .fold(0.0f32, f32::max);
        let m = smoothstep(0.5 * threshold, threshold, edge);
        for ch in 0..3 {
            let d = ((c[ch] - b[ch]) * strength * m).clamp(-halo, halo);
            px[ch] = ((c[ch] + d).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    });
}

/// Softens fine texture (small differences from a blur) and sharpens the larger edges.
fn soften_and_sharpen(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let soften = (0.5 * k).min(1.0);
    let sharpen = 0.6 * k;
    let threshold = 0.06f32;
    let mut blurred = rgba.to_vec();
    gaussian(
        &mut blurred,
        w,
        h,
        (4.0 * unit(h)).round().max(1.0) as usize,
    );
    let original = rgba.to_vec();
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let i = (y * w + x) * 4;
        let c = rgb_at(&original, i);
        let b = rgb_at(&blurred, i);
        let d = [0, 1, 2].map(|ch| c[ch] - b[ch]);
        let biggest = d.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let small = 1.0 - smoothstep(threshold, 2.0 * threshold, biggest);
        let gain = (1.0 + sharpen) + ((1.0 - soften) - (1.0 + sharpen)) * small;
        for ch in 0..3 {
            px[ch] = ((b[ch] + d[ch] * gain).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    match kind {
        EffectKind::SharpenEdges => sharpen_edges(rgba, w, h, a),
        EffectKind::SoftenAndSharpen => soften_and_sharpen(rgba, w, h, a),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 160;
    const H: usize = 90;

    fn flat(g: u8) -> Vec<u8> {
        (0..W * H).flat_map(|_| [g, g, g, 255]).collect()
    }

    /// A vertical edge: dark on the left, light on the right, over three pixels.
    fn edge() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for _ in 0..H {
            for x in 0..W {
                let t = ((x as f32 - 78.0) / 3.0).clamp(0.0, 1.0);
                let g = (80.0 + 90.0 * t) as u8;
                v.extend_from_slice(&[g, g, g, 255]);
            }
        }
        v
    }

    fn run(kind: EffectKind, amount: f32, pic: &[u8]) -> Vec<u8> {
        let mut out = pic.to_vec();
        Effect {
            amount,
            ..Effect::new(kind)
        }
        .apply(&mut out, W, H, 0);
        out
    }

    fn at(p: &[u8], x: usize, y: usize) -> u8 {
        p[(y * W + x) * 4]
    }

    const KINDS: [EffectKind; 2] = [EffectKind::SharpenEdges, EffectKind::SoftenAndSharpen];

    #[test]
    fn nothing_changes_at_zero_and_flat_areas_stay_flat() {
        let pic = edge();
        for kind in KINDS {
            assert_eq!(run(kind, 0.0, &pic), pic, "{kind:?} at zero");
            assert_eq!(run(kind, 100.0, &flat(120)), flat(120), "{kind:?} on flat");
            assert_ne!(run(kind, kind.default_amount(), &pic), pic, "{kind:?}");
        }
    }

    #[test]
    fn an_edge_gets_steeper_with_a_little_overshoot_on_both_sides() {
        let pic = edge();
        let out = run(EffectKind::SharpenEdges, 100.0, &pic);
        let steep = |p: &[u8]| i32::from(at(p, 81, 45)) - i32::from(at(p, 78, 45));
        assert!(steep(&out) > steep(&pic));
        assert!(at(&out, 78, 45) <= at(&pic, 78, 45), "dark side");
        assert!(at(&out, 81, 45) >= at(&pic, 81, 45), "light side");
    }

    #[test]
    fn soften_and_sharpen_overshoots_a_hard_edge_at_a_size_it_is_made_for() {
        // At 960 x 540 its radius is two pixels and a hard step is far above the texture limit.
        let (w, h) = (960usize, 540usize);
        let mut pic = Vec::with_capacity(w * h * 4);
        for _ in 0..h {
            for x in 0..w {
                let g = if x < 480 { 60u8 } else { 190 };
                pic.extend_from_slice(&[g, g, g, 255]);
            }
        }
        let mut out = pic.clone();
        Effect {
            amount: 100.0,
            ..Effect::new(EffectKind::SoftenAndSharpen)
        }
        .apply(&mut out, w, h, 0);
        let at = |p: &[u8], x: usize| p[(270 * w + x) * 4];
        assert!(at(&out, 479) < at(&pic, 479), "dark side undershoots");
        assert!(at(&out, 480) > at(&pic, 480), "light side overshoots");
    }

    #[test]
    fn halos_are_capped_for_sharpen_edges() {
        let mut hard = Vec::new();
        for _ in 0..H {
            for x in 0..W {
                let g = if x < 80 { 20u8 } else { 235 };
                hard.extend_from_slice(&[g, g, g, 255]);
            }
        }
        let out = run(EffectKind::SharpenEdges, 100.0, &hard);
        // 0.25 of full scale is 64 levels: nothing moves further than that.
        for (a, b) in out.chunks_exact(4).zip(hard.chunks_exact(4)) {
            assert!(a[0].abs_diff(b[0]) <= 65);
        }
    }

    #[test]
    fn soften_and_sharpen_smooths_fine_texture_and_keeps_alpha() {
        let mut texture = flat(120);
        for (i, px) in texture.chunks_exact_mut(4).enumerate() {
            px[..3].fill(if (i % W + i / W).is_multiple_of(2) {
                126
            } else {
                114
            });
        }
        let out = run(EffectKind::SoftenAndSharpen, 100.0, &texture);
        let spread = |p: &[u8]| {
            let (lo, hi) = p
                .chunks_exact(4)
                .fold((255u8, 0u8), |(lo, hi), q| (lo.min(q[0]), hi.max(q[0])));
            hi - lo
        };
        assert!(spread(&out) < spread(&texture), "fine texture is calmer");
        let mut clear = flat(120);
        clear.chunks_exact_mut(4).take(W).for_each(|p| p[3] = 0);
        let out = run(EffectKind::SoftenAndSharpen, 100.0, &clear);
        assert!(out.chunks_exact(4).take(W).all(|p| p[3] == 0));
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
}
