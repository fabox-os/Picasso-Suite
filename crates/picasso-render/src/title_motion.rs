//! How a title comes in ([`picasso_core::TitleAnimation`]): the title is drawn whole, as it will
//! stand, and then moved, scaled, turned, blurred, faded or uncovered by how far into its
//! animation the clip is. At the end of the animation it is the plain title, exactly.

use picasso_core::{TitleAnimation as A, TitleText};

const PI: f32 = std::f32::consts::PI;

/// Quick at first and then settling: 0 to 1.
fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p).powi(3)
}

/// Goes past 1 and comes back: 0 at 0, 1 at 1.
fn back_out(p: f32) -> f32 {
    let (c1, c3) = (1.701_58f32, 2.701_58f32);
    1.0 + c3 * (p - 1.0).powi(3) + c1 * (p - 1.0).powi(2)
}

fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / 16_777_216.0
}

/// What happens to the picture of the title, all at once.
#[derive(Clone, Copy)]
struct Pose {
    /// About the middle of the title.
    scale: f32,
    /// Radians.
    turn: f32,
    /// In pixels.
    dx: f32,
    dy: f32,
    alpha: f32,
}

impl Pose {
    const REST: Pose = Pose {
        scale: 1.0,
        turn: 0.0,
        dx: 0.0,
        dy: 0.0,
        alpha: 1.0,
    };
}

/// The box that holds the pixels of the title that can be seen.
fn bounds(layer: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if layer[(y * w + x) * 4 + 3] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x1 >= x0 && y1 >= y0).then_some((x0, y0, x1, y1))
}

/// Draws `layer` (RGBA, straight alpha, `w` x `h`) moved by `pose` about (`cx`, `cy`).
fn pose_layer(layer: &mut [u8], w: usize, h: usize, (cx, cy): (f32, f32), pose: Pose) {
    let copy = layer.to_vec();
    let premult = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        let a = f32::from(copy[i + 3]) / 255.0;
        [
            f32::from(copy[i]) * a,
            f32::from(copy[i + 1]) * a,
            f32::from(copy[i + 2]) * a,
            a * 255.0,
        ]
    };
    let (sin, cos) = (-pose.turn).sin_cos();
    let scale = pose.scale.max(0.01);
    for y in 0..h {
        for x in 0..w {
            // Back from the place on the screen to the place in the title's own picture.
            let (px, py) = (x as f32 + 0.5 - cx - pose.dx, y as f32 + 0.5 - cy - pose.dy);
            let (rx, ry) = ((px * cos - py * sin) / scale, (px * sin + py * cos) / scale);
            let (sx, sy) = (rx + cx - 0.5, ry + cy - 0.5);
            let out = &mut layer[(y * w + x) * 4..(y * w + x) * 4 + 4];
            if sx < -1.0 || sy < -1.0 || sx > w as f32 || sy > h as f32 {
                out.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let (fx, fy) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - fx, sy - fy);
            let at = |ix: f32, iy: f32| {
                if ix < 0.0 || iy < 0.0 || ix >= w as f32 || iy >= h as f32 {
                    [0.0f32; 4]
                } else {
                    premult(ix as usize, iy as usize)
                }
            };
            let (a, b, c, d) = (at(fx, fy), at(fx + 1.0, fy), at(fx, fy + 1.0), at(fx + 1.0, fy + 1.0));
            let mut v = [0.0f32; 4];
            for k in 0..4 {
                let top = a[k] + (b[k] - a[k]) * tx;
                let bottom = c[k] + (d[k] - c[k]) * tx;
                v[k] = top + (bottom - top) * ty;
            }
            let alpha = v[3] * pose.alpha.clamp(0.0, 1.0);
            let k = if v[3] > 1e-3 { 1.0 / v[3] } else { 0.0 };
            out[0] = (v[0] * k).round().clamp(0.0, 255.0) as u8;
            out[1] = (v[1] * k).round().clamp(0.0, 255.0) as u8;
            out[2] = (v[2] * k).round().clamp(0.0, 255.0) as u8;
            out[3] = alpha.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Makes `layer` (the finished title, RGBA straight alpha, `w` x `h`) what it looks like `p`
/// (0 to 1) of the way through the animation `kind`.
pub(crate) fn animate(layer: &mut [u8], w: usize, h: usize, title: &TitleText, kind: A, p: f32) {
    let p = p.clamp(0.0, 1.0);
    let e = ease_out(p);
    let centre = (title.x.min(1000) as f32 / 1000.0 * w as f32, title.y.min(1000) as f32 / 1000.0 * h as f32);
    let (wf, hf) = (w as f32, h as f32);
    let pose = match kind {
        A::None => return,
        A::Fade => Pose { alpha: e, ..Pose::REST },
        A::SlideLeft => Pose { dx: -(1.0 - e) * 0.3 * wf, alpha: e, ..Pose::REST },
        A::SlideRight => Pose { dx: (1.0 - e) * 0.3 * wf, alpha: e, ..Pose::REST },
        A::SlideUp => Pose { dy: -(1.0 - e) * 0.3 * hf, alpha: e, ..Pose::REST },
        A::SlideDown => Pose { dy: (1.0 - e) * 0.3 * hf, alpha: e, ..Pose::REST },
        A::Rise => Pose { dy: (1.0 - e) * 0.08 * hf, alpha: e, ..Pose::REST },
        A::Drop => Pose { dy: -(1.0 - e) * 0.08 * hf, alpha: e, ..Pose::REST },
        A::ZoomIn => Pose { scale: 0.5 + 0.5 * e, alpha: e, ..Pose::REST },
        A::ZoomOut => Pose { scale: 1.5 - 0.5 * e, alpha: e, ..Pose::REST },
        A::Pop => Pose { scale: back_out(p), alpha: (p * 4.0).min(1.0), ..Pose::REST },
        A::Stamp => Pose {
            scale: 1.0 + (1.0 - e),
            alpha: (p * 5.0).min(1.0),
            ..Pose::REST
        },
        A::SpinIn => Pose {
            scale: 0.3 + 0.7 * e,
            turn: -(1.0 - e) * PI * 0.5,
            alpha: e,
            ..Pose::REST
        },
        A::BlurIn => {
            // Out of focus and faint, then sharp.
            let radius = ((1.0 - e) * 24.0 * hf / 1000.0).round() as usize;
            if radius > 0 {
                blur_layer(layer, w, h, radius);
            }
            Pose { alpha: e, ..Pose::REST }
        }
        A::Wipe | A::Typewriter => {
            let Some((x0, _, x1, _)) = bounds(layer, w, h) else {
                return;
            };
            let share = if kind == A::Typewriter {
                // A letter at a time: the title is as many steps as its longest line has letters.
                let letters = title
                    .text
                    .lines()
                    .map(|l| l.chars().count())
                    .max()
                    .unwrap_or(1)
                    .max(1) as f32;
                ((p * letters + 1e-4).floor() / letters).min(1.0)
            } else {
                e
            };
            let edge = x0 as f32 + (x1 - x0 + 1) as f32 * share;
            let soft = if kind == A::Wipe { (hf / 100.0).max(2.0) } else { 0.5 };
            for y in 0..h {
                for x in 0..w {
                    let k = ((edge - x as f32) / soft).clamp(0.0, 1.0);
                    let a = &mut layer[(y * w + x) * 4 + 3];
                    *a = (f32::from(*a) * k).round() as u8;
                }
            }
            return;
        }
        A::Flicker => {
            let step = (p * 14.0).floor() as u32;
            let on = hash(step) < 0.3 + 0.7 * p;
            Pose { alpha: if on { 1.0 } else { 0.0 }, ..Pose::REST }
        }
        A::GlitchIn => {
            // Slices of the title jump sideways, less and less.
            let band = (h / 40).max(1);
            let step = (p * 12.0).floor() as u32;
            let copy = layer.to_vec();
            for y in 0..h {
                let n = hash((y / band) as u32 * 31 + step);
                let jump = if n < 0.5 { (hash((y / band) as u32 + step * 7) - 0.5) * 2.0 * (1.0 - p) * 0.12 * wf } else { 0.0 };
                for x in 0..w {
                    let sx = x as f32 - jump;
                    let dst = (y * w + x) * 4;
                    if sx < 0.0 || sx >= wf {
                        layer[dst..dst + 4].copy_from_slice(&[0, 0, 0, 0]);
                    } else {
                        let src = (y * w + sx as usize) * 4;
                        layer[dst..dst + 4].copy_from_slice(&copy[src..src + 4]);
                    }
                }
            }
            Pose { alpha: (p * 2.5).min(1.0), ..Pose::REST }
        }
    };
    pose_layer(layer, w, h, centre, pose);
}

/// Blurs the title's picture (each of its four channels, with the colour weighted by alpha).
fn blur_layer(layer: &mut [u8], w: usize, h: usize, radius: usize) {
    let mut planes = [vec![0f32; w * h], vec![0f32; w * h], vec![0f32; w * h], vec![0f32; w * h]];
    for (i, px) in layer.chunks_exact(4).enumerate() {
        let a = f32::from(px[3]) / 255.0;
        planes[0][i] = f32::from(px[0]) * a;
        planes[1][i] = f32::from(px[1]) * a;
        planes[2][i] = f32::from(px[2]) * a;
        planes[3][i] = a * 255.0;
    }
    let planes: Vec<Vec<f32>> = planes.iter().map(|p| crate::text::blur(p, w, h, radius)).collect();
    for (i, px) in layer.chunks_exact_mut(4).enumerate() {
        let a = planes[3][i];
        let k = if a > 1e-3 { 255.0 / a } else { 0.0 };
        px[0] = (planes[0][i] * k).round().clamp(0.0, 255.0) as u8;
        px[1] = (planes[1][i] * k).round().clamp(0.0, 255.0) as u8;
        px[2] = (planes[2][i] * k).round().clamp(0.0, 255.0) as u8;
        px[3] = a.round().clamp(0.0, 255.0) as u8;
    }
}

/// Puts `layer` over `out` (both straight alpha RGBA).
pub(crate) fn over(out: &mut [u8], layer: &[u8]) {
    for (o, l) in out.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
        let la = f32::from(l[3]) / 255.0;
        if la <= 0.0 {
            continue;
        }
        let oa = f32::from(o[3]) / 255.0;
        let alpha = la + oa * (1.0 - la);
        for k in 0..3 {
            let v = (f32::from(l[k]) * la + f32::from(o[k]) * oa * (1.0 - la)) / alpha.max(1e-6);
            o[k] = v.round().clamp(0.0, 255.0) as u8;
        }
        o[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_core::TextStyle;

    const W: usize = 240;
    const H: usize = 120;

    fn title(kind: A) -> TitleText {
        TitleText {
            text: "Hello".into(),
            size: 200,
            style: TextStyle {
                animation: kind,
                animation_ms: 1000,
                ..TextStyle::default()
            },
            ..TitleText::default()
        }
    }

    fn at(kind: A, ms: i64) -> Vec<u8> {
        let mut out = vec![0u8; W * H * 4];
        crate::text::draw_title_at(&mut out, W, H, &title(kind), ms * 1000);
        out
    }

    fn weight(buf: &[u8]) -> f32 {
        buf.chunks_exact(4).map(|p| f32::from(p[3])).sum::<f32>() / 255.0
    }

    /// Where the middle of the title is, as a share of the picture: (across, down).
    fn middle(buf: &[u8]) -> (f32, f32) {
        let (mut sx, mut sy, mut total) = (0.0f32, 0.0f32, 0.0f32);
        for (i, p) in buf.chunks_exact(4).enumerate() {
            let a = f32::from(p[3]);
            sx += a * (i % W) as f32;
            sy += a * (i / W) as f32;
            total += a;
        }
        (sx / total / W as f32, sy / total / H as f32)
    }

    fn width_of(buf: &[u8]) -> usize {
        let (x0, _, x1, _) = bounds(buf, W, H).unwrap();
        x1 - x0
    }

    #[test]
    fn every_animation_starts_empty_or_off_screen_and_ends_as_the_plain_title() {
        let plain = at(A::None, 0);
        assert!(weight(&plain) > 40.0);
        for kind in A::ALL.into_iter().filter(|k| *k != A::None) {
            // At the end, and after it: the title as it stands.
            assert_eq!(at(kind, 1000), plain, "{kind:?} at the end");
            assert_eq!(at(kind, 5000), plain, "{kind:?} after the end");
            // At the start there is (next to) nothing to see.
            let start = weight(&at(kind, 0));
            assert!(start < weight(&plain) * 0.05, "{kind:?} at the start: {start}");
            // Before the clip there is nothing either, and it never panics.
            let _ = at(kind, -500);
        }
    }

    #[test]
    fn titles_fade_and_slide_into_place() {
        let plain = at(A::None, 0);
        let (px, py) = middle(&plain);
        let early = weight(&at(A::Fade, 200));
        let late = weight(&at(A::Fade, 600));
        assert!(early > 0.0 && early < late && late < weight(&plain));
        let (lx, _) = middle(&at(A::SlideLeft, 300));
        let (rx, _) = middle(&at(A::SlideRight, 300));
        assert!(lx < px - 0.02 && rx > px + 0.02, "{lx} {rx} {px}");
        let (_, uy) = middle(&at(A::SlideUp, 300));
        let (_, dy) = middle(&at(A::SlideDown, 300));
        assert!(uy < py - 0.02 && dy > py + 0.02, "{uy} {dy} {py}");
        // Rise comes up from below, Drop down from above.
        assert!(middle(&at(A::Rise, 300)).1 > py);
        assert!(middle(&at(A::Drop, 300)).1 < py);
    }

    #[test]
    fn zooms_and_pops_change_the_size() {
        let plain = width_of(&at(A::None, 0));
        assert!(width_of(&at(A::ZoomIn, 300)) < plain);
        assert!(width_of(&at(A::ZoomOut, 300)) > plain);
        assert!(width_of(&at(A::Stamp, 150)) > plain);
        // A pop goes past its size on the way.
        let biggest = (200..=900)
            .step_by(50)
            .map(|ms| width_of(&at(A::Pop, ms)))
            .max()
            .unwrap();
        assert!(biggest > plain + 2, "{biggest} {plain}");
    }

    #[test]
    fn wipe_and_typewriter_uncover_from_the_left() {
        let plain = at(A::None, 0);
        let half = at(A::Wipe, 400);
        let (_, _, x1, _) = bounds(&half, W, H).unwrap();
        let (_, _, full_x1, _) = bounds(&plain, W, H).unwrap();
        assert!(x1 < full_x1);
        // The letters are the same where they show.
        assert!(half.chunks_exact(4).zip(plain.chunks_exact(4)).all(|(a, b)| a[3] == 0 || a[..3] == b[..3]));
        // A typewriter grows a letter at a time: the same step for a while, then the next.
        let steps: Vec<f32> = (0..=1000).step_by(100).map(|ms| weight(&at(A::Typewriter, ms))).collect();
        assert!(steps.windows(2).all(|w| w[0] <= w[1] + 0.5), "{steps:?}");
        assert!(steps.windows(2).filter(|w| w[1] > w[0] + 5.0).count() >= 2);
        // Five letters: at 0.45 of the time, two are typed (floor of 2.25), same as at 0.4.
        assert_eq!(at(A::Typewriter, 400), at(A::Typewriter, 450));
    }

    #[test]
    fn blur_in_softens_and_the_others_are_repeatable() {
        let sharp = at(A::None, 0);
        let soft = at(A::BlurIn, 500);
        let partial = |buf: &[u8]| buf.chunks_exact(4).filter(|p| p[3] > 10 && p[3] < 245).count();
        assert!(partial(&soft) > partial(&sharp));
        for kind in [A::Flicker, A::GlitchIn, A::SpinIn] {
            assert_eq!(at(kind, 300), at(kind, 300), "{kind:?}");
            assert_ne!(at(kind, 300), at(kind, 600), "{kind:?}");
        }
    }

    #[test]
    fn a_title_without_an_animation_ignores_time() {
        assert_eq!(at(A::None, 0), at(A::None, 700));
    }
}
