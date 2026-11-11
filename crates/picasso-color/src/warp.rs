//! Warp effects: Lens Barrel and Lens Pincushion, Ripple, Wave and Vertical Wave, Horizontal and
//! Vertical Bow, Square Bulge and Square Pinch, and Corner Pin from each of the four sides.
//!
//! Each moves the pixels of one picture (RGBA8, straight alpha) and has one strength, 0 to 100,
//! that scales its settings from nothing (0) through their usual values (50) to twice as much
//! (100). Sizes are in thousandths of the picture's height. The animated ones (Ripple, Wave)
//! follow the clip's frame rate.

use crate::fx::EffectKind;
use crate::look::{par_pixels, put, remap, Src};

const TAU: f32 = std::f32::consts::TAU;

fn unit(h: usize) -> f32 {
    h as f32 / 1080.0
}

// ---- Lens distortion ----

/// Barrel (positive `k`) or pincushion (negative) distortion; what lies outside the picture is
/// clear.
fn lens(rgba: &mut [u8], w: usize, h: usize, k: f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let norm = cx.hypot(cy).max(1.0);
    remap(rgba, w, h, &move |x, y| {
        let (px, py) = (x - cx, y - cy);
        let r2 = (px * px + py * py) / (norm * norm);
        let f = 1.0 + k * r2;
        let (sx, sy) = (cx + px * f, cy + py * f);
        if sx < 0.0 || sx > w as f32 || sy < 0.0 || sy > h as f32 {
            None
        } else {
            Some((sx, sy))
        }
    });
}

// ---- Ripple and Wave ----

fn ripple(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64, rate: f32) {
    let k = a * 2.0;
    let u = unit(h);
    let amplitude = 10.0 * u * k;
    let wavelength = (60.0 * u).max(2.0);
    let phase = TAU * frame as f32 / rate;
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    remap(rgba, w, h, &move |x, y| {
        let (px, py) = (x - cx, y - cy);
        let r = px.hypot(py);
        if r < 1e-4 {
            return Some((x, y));
        }
        let o = amplitude * (TAU * r / wavelength - phase).sin();
        Some((x + px / r * o, y + py / r * o))
    });
}

/// Straight waves; `angle` is the direction they travel, in radians.
fn wave(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64, rate: f32, angle: f32) {
    let k = a * 2.0;
    let u = unit(h);
    let amplitude = 10.0 * u * k;
    let wavelength = (120.0 * u).max(2.0);
    let phase = TAU * frame as f32 / rate;
    let dir = (angle.cos(), -angle.sin());
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    remap(rgba, w, h, &move |x, y| {
        let (px, py) = (x - cx, y - cy);
        let s = px * dir.0 + py * dir.1;
        let o = amplitude * (TAU * s / wavelength - phase).sin();
        Some((x - dir.1 * o, y + dir.0 * o))
    });
}

// ---- Dent ----

#[derive(Clone, Copy)]
enum Dent {
    BowHorizontal,
    BowVertical,
    SquareBulge,
    SquarePinch,
}

/// Bows the picture out or in around its middle, along one direction or square; the edges repeat.
fn dent(rgba: &mut [u8], w: usize, h: usize, a: f32, kind: Dent) {
    let s = a.min(0.9);
    let radius = (400.0 * unit(h)).max(1.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    remap(rgba, w, h, &move |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let (sx, sy) = match kind {
            Dent::BowHorizontal => {
                let wt = (1.0 - dy * dy / (radius * radius)).max(0.0);
                (dx * (1.0 - s * wt), dy)
            }
            Dent::BowVertical => {
                let wt = (1.0 - dx * dx / (radius * radius)).max(0.0);
                (dx, dy * (1.0 - s * wt))
            }
            Dent::SquareBulge | Dent::SquarePinch => {
                let r = dx.abs().max(dy.abs()) / radius;
                let wt = if r < 1.0 { 1.0 - r * r } else { 0.0 };
                let f = if matches!(kind, Dent::SquareBulge) {
                    1.0 - s * wt
                } else {
                    1.0 / (1.0 - s * wt)
                };
                (dx * f, dy * f)
            }
        };
        Some((
            (sx + cx).clamp(0.0, w as f32),
            (sy + cy).clamp(0.0, h as f32),
        ))
    });
}

// ---- Corner Pin ----

/// The 3 x 3 matrix, row by row, that carries the unit square onto the quadrilateral `q`
/// (corners in the order top left, top right, bottom right, bottom left).
fn square_to_quad(q: [[f32; 2]; 4]) -> [f32; 9] {
    let [p0, p1, p2, p3] = q;
    let (dx1, dy1) = (p1[0] - p2[0], p1[1] - p2[1]);
    let (dx2, dy2) = (p3[0] - p2[0], p3[1] - p2[1]);
    let (dx3, dy3) = (p0[0] - p1[0] + p2[0] - p3[0], p0[1] - p1[1] + p2[1] - p3[1]);
    let (g, hh) = if dx3.abs() < 1e-6 && dy3.abs() < 1e-6 {
        (0.0, 0.0)
    } else {
        let den = dx1 * dy2 - dy1 * dx2;
        if den.abs() < 1e-9 {
            (0.0, 0.0)
        } else {
            ((dx3 * dy2 - dy3 * dx2) / den, (dx1 * dy3 - dy1 * dx3) / den)
        }
    };
    [
        p1[0] - p0[0] + g * p1[0],
        p3[0] - p0[0] + hh * p3[0],
        p0[0],
        p1[1] - p0[1] + g * p1[1],
        p3[1] - p0[1] + hh * p3[1],
        p0[1],
        g,
        hh,
        1.0,
    ]
}

/// The inverse of a 3 x 3 matrix, or `None` when it has none.
fn invert3(m: [f32; 9]) -> Option<[f32; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-12 {
        return None;
    }
    let i = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * i,
        (m[2] * m[7] - m[1] * m[8]) * i,
        (m[1] * m[5] - m[2] * m[4]) * i,
        (m[5] * m[6] - m[3] * m[8]) * i,
        (m[0] * m[8] - m[2] * m[6]) * i,
        (m[2] * m[3] - m[0] * m[5]) * i,
        (m[3] * m[7] - m[4] * m[6]) * i,
        (m[1] * m[6] - m[0] * m[7]) * i,
        (m[0] * m[4] - m[1] * m[3]) * i,
    ])
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Tilts the picture in perspective so one side looks farther away: the two corners of that
/// side move toward each other and a little inward. Edges are smooth.
fn corner_pin(rgba: &mut [u8], w: usize, h: usize, a: f32, side: Side) {
    let (wf, hf) = (w as f32, h as f32);
    let shrink = a * 0.18 * hf;
    let inward = shrink * 0.4;
    // Corners: top left, top right, bottom right, bottom left.
    let mut q = [[0.0, 0.0], [wf, 0.0], [wf, hf], [0.0, hf]];
    match side {
        Side::Left => {
            q[0] = [inward, shrink];
            q[3] = [inward, hf - shrink];
        }
        Side::Right => {
            q[1] = [wf - inward, shrink];
            q[2] = [wf - inward, hf - shrink];
        }
        Side::Top => {
            q[0] = [shrink * wf / hf, inward];
            q[1] = [wf - shrink * wf / hf, inward];
        }
        Side::Bottom => {
            q[3] = [shrink * wf / hf, hf - inward];
            q[2] = [wf - shrink * wf / hf, hf - inward];
        }
    }
    let Some(inv) = invert3(square_to_quad(q)) else {
        return;
    };
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let project = move |x: f32, y: f32| -> Option<(f32, f32)> {
        let z = inv[6] * x + inv[7] * y + inv[8];
        if z <= 1e-6 {
            return None;
        }
        Some((
            (inv[0] * x + inv[1] * y + inv[2]) / z,
            (inv[3] * x + inv[4] * y + inv[5]) / z,
        ))
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let Some((tx, ty)) = project(xf, yf) else {
            px.copy_from_slice(&[0, 0, 0, 0]);
            return;
        };
        // Coverage at the edge of the picture, from how far the point lies outside it in pixels.
        let (nx, ny) = (
            project(xf + 1.0, yf).unwrap_or((tx, ty)),
            project(xf, yf + 1.0).unwrap_or((tx, ty)),
        );
        let fw = [
            (nx.0 - tx).abs().max((ny.0 - tx).abs()).max(1e-6),
            (nx.1 - ty).abs().max((ny.1 - ty).abs()).max(1e-6),
        ];
        let cover = |t: f32, f: f32| ((t.min(1.0 - t)) / f + 0.5).clamp(0.0, 1.0);
        let m = cover(tx, fw[0]) * cover(ty, fw[1]);
        if m <= 0.0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        let c = src.at(tx.clamp(0.0, 1.0) * wf, ty.clamp(0.0, 1.0) * hf);
        put(px, [c[0] * m, c[1] * m, c[2] * m, c[3] * m]);
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(
    kind: EffectKind,
    a: f32,
    rgba: &mut [u8],
    w: usize,
    h: usize,
    frame: u64,
    rate: f32,
) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    match kind {
        EffectKind::LensBarrel => lens(rgba, w, h, a * 0.3),
        EffectKind::LensPincushion => lens(rgba, w, h, -a * 0.3),
        EffectKind::Ripple => ripple(rgba, w, h, a, frame, rate),
        EffectKind::Wave => wave(rgba, w, h, a, frame, rate, 0.0),
        EffectKind::VerticalWave => wave(rgba, w, h, a, frame, rate, std::f32::consts::FRAC_PI_2),
        EffectKind::HorizontalBow => dent(rgba, w, h, a, Dent::BowHorizontal),
        EffectKind::VerticalBow => dent(rgba, w, h, a, Dent::BowVertical),
        EffectKind::SquareBulge => dent(rgba, w, h, a, Dent::SquareBulge),
        EffectKind::SquarePinch => dent(rgba, w, h, a, Dent::SquarePinch),
        EffectKind::CornerPinLeft => corner_pin(rgba, w, h, a, Side::Left),
        EffectKind::CornerPinRight => corner_pin(rgba, w, h, a, Side::Right),
        EffectKind::CornerPinTop => corner_pin(rgba, w, h, a, Side::Top),
        EffectKind::CornerPinBottom => corner_pin(rgba, w, h, a, Side::Bottom),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 160;
    const H: usize = 90;

    const KINDS: [EffectKind; 13] = [
        EffectKind::LensBarrel,
        EffectKind::LensPincushion,
        EffectKind::Ripple,
        EffectKind::Wave,
        EffectKind::VerticalWave,
        EffectKind::HorizontalBow,
        EffectKind::VerticalBow,
        EffectKind::SquareBulge,
        EffectKind::SquarePinch,
        EffectKind::CornerPinLeft,
        EffectKind::CornerPinRight,
        EffectKind::CornerPinTop,
        EffectKind::CornerPinBottom,
    ];

    /// Every pixel different, opaque.
    fn picture() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                v.extend_from_slice(&[
                    (x * 255 / W) as u8,
                    (y * 255 / H) as u8,
                    ((x ^ y) * 3) as u8,
                    255,
                ]);
            }
        }
        v
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

    #[test]
    fn nothing_changes_at_zero_and_all_move_pixels_at_the_usual_strength() {
        let pic = picture();
        for kind in KINDS {
            assert_eq!(run(kind, 0.0, &pic, 5), pic, "{kind:?} at zero");
            assert_ne!(run(kind, kind.default_amount(), &pic, 5), pic, "{kind:?}");
        }
    }

    #[test]
    fn the_middle_of_a_lens_or_dent_stays_and_the_size_is_kept() {
        let pic = picture();
        for kind in [
            EffectKind::LensBarrel,
            EffectKind::LensPincushion,
            EffectKind::HorizontalBow,
            EffectKind::VerticalBow,
            EffectKind::SquareBulge,
            EffectKind::SquarePinch,
        ] {
            let out = run(kind, 40.0, &pic, 0);
            assert_eq!(out.len(), pic.len());
            let (a, b) = (px(&out, W / 2, H / 2), px(&pic, W / 2, H / 2));
            assert!(
                a[0].abs_diff(b[0]) <= 4 && a[1].abs_diff(b[1]) <= 4,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_barrel_shrinks_the_picture_into_clear_corners_and_a_pincushion_fills_them() {
        let pic = picture();
        let barrel = run(EffectKind::LensBarrel, 100.0, &pic, 0);
        assert_eq!(px(&barrel, 0, 0)[3], 0, "barrel leaves the corner clear");
        let pin = run(EffectKind::LensPincushion, 100.0, &pic, 0);
        assert_eq!(px(&pin, 0, 0)[3], 255);
    }

    #[test]
    fn ripples_and_waves_move_with_time_and_keep_their_alpha() {
        let pic = picture();
        for kind in [
            EffectKind::Ripple,
            EffectKind::Wave,
            EffectKind::VerticalWave,
        ] {
            let a = run(kind, 100.0, &pic, 2);
            assert_eq!(a, run(kind, 100.0, &pic, 2), "{kind:?}");
            assert_ne!(a, run(kind, 100.0, &pic, 5), "{kind:?}");
            assert!(a.chunks_exact(4).all(|p| p[3] == 255), "{kind:?}");
        }
        // A vertical wave moves pixels across.
        let v = run(EffectKind::VerticalWave, 100.0, &pic, 0);
        assert_ne!(px(&v, 10, 40), px(&pic, 10, 40));
    }

    #[test]
    fn square_dents_bow_the_edges_and_a_pin_leaves_clear_wedges() {
        let pic = picture();
        let pinch = run(EffectKind::SquarePinch, 100.0, &pic, 0);
        assert_ne!(pinch, pic);
        for (kind, corner) in [
            (EffectKind::CornerPinLeft, (0usize, 0usize)),
            (EffectKind::CornerPinTop, (0, 0)),
            (EffectKind::CornerPinRight, (W - 1, H - 1)),
            (EffectKind::CornerPinBottom, (W - 1, H - 1)),
        ] {
            let out = run(kind, 100.0, &pic, 0);
            assert_eq!(
                px(&out, corner.0, corner.1)[3],
                0,
                "{kind:?} leaves a wedge clear"
            );
            assert_eq!(px(&out, W / 2, H / 2)[3], 255, "{kind:?} middle");
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
}
