//! Transform effects: Feathered Crop (all four edges, or one side) and Video Collage (the
//! picture again and again in a grid of tiles with a soft shadow).
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100. For the
//! crops it is how much is cut away, from nothing (0) to a third of the picture (100); for the
//! collages it is the space between the tiles, from none (0) through 8 lines at 1080 (50). A
//! strength of 0 switches the effect off, so a collage at 0 shows the whole picture. Sizes are in
//! thousandths of the picture's height.

use crate::fx::EffectKind;
use crate::look::{par_pixels, put, Src};

fn unit(h: usize) -> f32 {
    h as f32 / 1080.0
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy)]
struct Sides {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

/// Crops the chosen edges with a soft feather running inwards from the crop line.
fn feathered_crop(rgba: &mut [u8], w: usize, h: usize, a: f32, sides: Sides) {
    let cut = a * 0.34;
    let (wf, hf) = (w as f32, h as f32);
    let feather = (20.0 * unit(h)).max(1.0);
    let x0 = if sides.left { cut * wf } else { 0.0 };
    let x1 = wf - if sides.right { cut * wf } else { 0.0 };
    let y0 = if sides.top { cut * hf } else { 0.0 };
    let y1 = hf - if sides.bottom { cut * hf } else { 0.0 };
    let ramp = |d: f32, on: bool| {
        if on {
            smoothstep(0.0, 1.0, (d / feather).clamp(0.0, 1.0))
        } else {
            1.0
        }
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let m = ramp(xf - x0, sides.left)
            * ramp(x1 - xf, sides.right)
            * ramp(yf - y0, sides.top)
            * ramp(y1 - yf, sides.bottom);
        px[3] = (f32::from(px[3]) * m + 0.5) as u8;
    });
}

/// The picture again and again in `n` columns and rows of tiles, each with a soft shadow down and
/// to the right, over a clear background.
fn collage(rgba: &mut [u8], w: usize, h: usize, a: f32, n: usize) {
    let spacing = 16.0 * unit(h) * a;
    let (wf, hf) = (w as f32, h as f32);
    let grid = n as f32;
    let (cell_w, cell_h) = (wf / grid, hf / grid);
    let tile = [(cell_w - spacing).max(1.0), (cell_h - spacing).max(1.0)];
    let half = [tile[0] / 2.0, tile[1] / 2.0];
    let soft = spacing * 0.4 + 1.0;
    let shadow_strength = 0.3;
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let box_distance = |q: [f32; 2]| {
        let d = [q[0].abs() - half[0], q[1].abs() - half[1]];
        d[0].max(0.0).hypot(d[1].max(0.0)) + d[0].max(d[1]).min(0.0)
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let cx = (xf / cell_w).floor().min(grid - 1.0);
        let cy = (yf / cell_h).floor().min(grid - 1.0);
        let q = [xf - (cx + 0.5) * cell_w, yf - (cy + 0.5) * cell_h];
        let cover = (0.5 - box_distance(q)).clamp(0.0, 1.0);
        let t = [q[0] / tile[0] + 0.5, q[1] / tile[1] + 0.5];
        let tex = if (0.0..=1.0).contains(&t[0]) && (0.0..=1.0).contains(&t[1]) {
            src.at(t[0] * wf, t[1] * hf)
        } else {
            [0.0; 4]
        };
        let sd = box_distance([q[0] - spacing * 0.2, q[1] - spacing * 0.2]);
        let shadow = shadow_strength * (1.0 - smoothstep(-soft, soft, sd));
        let under = 1.0 - tex[3] * cover;
        put(
            px,
            [
                tex[0] * cover,
                tex[1] * cover,
                tex[2] * cover,
                tex[3] * cover + shadow * under,
            ],
        );
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    let all = Sides {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };
    let none = Sides {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };
    match kind {
        EffectKind::FeatheredCrop => feathered_crop(rgba, w, h, a, all),
        EffectKind::FeatheredCropLeft => {
            feathered_crop(rgba, w, h, a, Sides { left: true, ..none })
        }
        EffectKind::FeatheredCropRight => feathered_crop(
            rgba,
            w,
            h,
            a,
            Sides {
                right: true,
                ..none
            },
        ),
        EffectKind::FeatheredCropTop => feathered_crop(rgba, w, h, a, Sides { top: true, ..none }),
        EffectKind::FeatheredCropBottom => feathered_crop(
            rgba,
            w,
            h,
            a,
            Sides {
                bottom: true,
                ..none
            },
        ),
        EffectKind::Collage2x2 => collage(rgba, w, h, a, 2),
        EffectKind::Collage3x3 => collage(rgba, w, h, a, 3),
        EffectKind::Collage4x4 => collage(rgba, w, h, a, 4),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 160;
    const H: usize = 90;

    const CROPS: [EffectKind; 5] = [
        EffectKind::FeatheredCrop,
        EffectKind::FeatheredCropLeft,
        EffectKind::FeatheredCropRight,
        EffectKind::FeatheredCropTop,
        EffectKind::FeatheredCropBottom,
    ];
    const COLLAGES: [EffectKind; 3] = [
        EffectKind::Collage2x2,
        EffectKind::Collage3x3,
        EffectKind::Collage4x4,
    ];

    fn picture() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                v.extend_from_slice(&[(x * 255 / W) as u8, (y * 255 / H) as u8, 90, 255]);
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

    fn alpha(p: &[u8], x: usize, y: usize) -> u8 {
        p[(y * W + x) * 4 + 3]
    }

    #[test]
    fn nothing_changes_at_zero_and_all_change_the_picture_at_the_usual_strength() {
        let pic = picture();
        for kind in CROPS.into_iter().chain(COLLAGES) {
            assert_eq!(run(kind, 0.0, &pic), pic, "{kind:?} at zero");
            assert_ne!(run(kind, kind.default_amount(), &pic), pic, "{kind:?}");
        }
    }

    #[test]
    fn a_crop_clears_its_edge_softly_and_keeps_the_middle() {
        let pic = picture();
        let all = run(EffectKind::FeatheredCrop, 100.0, &pic);
        for (x, y) in [(0, 45), (W - 1, 45), (80, 0), (80, H - 1)] {
            assert_eq!(alpha(&all, x, y), 0, "edge ({x}, {y})");
        }
        assert_eq!(alpha(&all, 80, 45), 255);
        let left = run(EffectKind::FeatheredCropLeft, 100.0, &pic);
        assert_eq!(alpha(&left, 0, 45), 0);
        assert_eq!(alpha(&left, W - 1, 45), 255);
        assert_eq!(alpha(&left, 80, 0), 255);
        // Soft: between the cut and the full picture the alpha rises in steps.
        let row: Vec<u8> = (0..W).map(|x| alpha(&left, x, 45)).collect();
        assert!(row.iter().any(|a| *a > 0 && *a < 255), "{row:?}");
        assert!(row.windows(2).all(|p| p[0] <= p[1]), "rises one way");
    }

    #[test]
    fn a_collage_shows_the_whole_picture_in_each_tile() {
        // Big enough for the space between tiles to be more than a pixel.
        let (w, h) = (960usize, 540usize);
        let mut pic = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                pic.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 90, 255]);
            }
        }
        let mut out = pic.clone();
        Effect {
            amount: 50.0,
            ..Effect::new(EffectKind::Collage2x2)
        }
        .apply(&mut out, w, h, 0);
        // The middle of each tile shows the middle of the picture.
        let at = |p: &[u8], x: usize, y: usize| {
            let i = (y * w + x) * 4;
            [p[i], p[i + 1], p[i + 2], p[i + 3]]
        };
        let middle = at(&pic, w / 2, h / 2);
        for (x, y) in [(240, 135), (720, 135), (240, 405), (720, 405)] {
            let t = at(&out, x, y);
            assert!(
                t[0].abs_diff(middle[0]) <= 8 && t[1].abs_diff(middle[1]) <= 8,
                "tile at ({x}, {y})"
            );
            assert_eq!(t[3], 255);
        }
        // Between the tiles the picture is gone (a shadow at most).
        assert!(at(&out, w / 2, h / 2)[3] < 100);
    }

    #[test]
    fn they_work_on_tiny_pictures() {
        for kind in CROPS.into_iter().chain(COLLAGES) {
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
