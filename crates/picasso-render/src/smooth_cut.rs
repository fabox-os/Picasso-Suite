//! Smooth Cut: hides a jump cut by morphing the outgoing picture into the incoming one along
//! the motion between them.
//!
//! The motion is found at a small size by block matching, coarse to fine over three sizes, and
//! smoothed; each pixel of the in-between picture then reads the outgoing picture a little back
//! along the motion and the incoming one a little ahead, and mixes them. A thing that moved
//! shows between its two places and not at both. Where the motion is not found (the pictures
//! have nothing in common) the result is a plain dissolve, the best a morph can do.

use picasso_core::TransitionKind;

use crate::transitions::{cmix, v2, Pic};

/// The longest side of the picture the motion is searched in.
const SEARCH_SIDE: usize = 160;
/// Pixels between the flow samples at every level, and the side of the block matched around one.
const STEP: usize = 4;
const BLOCK: i32 = 8;
/// How far, in pixels of the level, one level looks around the motion the coarser one found.
const REACH: i32 = 5;
/// The most sizes searched, and the smallest side (in pixels) a size may have.
const LEVELS: usize = 4;
const SMALLEST: usize = 12;

pub(crate) fn handles(kind: TransitionKind) -> bool {
    kind == TransitionKind::SmoothCut
}

/// A grey picture to search in.
struct Grey {
    w: usize,
    h: usize,
    d: Vec<f32>,
}

impl Grey {
    /// The picture (premultiplied luma) reduced by `factor` with a box mean.
    fn reduced(data: &[u8], w: usize, h: usize, factor: usize) -> Grey {
        let (w2, h2) = (w.div_ceil(factor), h.div_ceil(factor));
        let mut d = vec![0.0f32; w2 * h2];
        for y in 0..h2 {
            for x in 0..w2 {
                let (mut sum, mut n) = (0.0, 0.0);
                for yy in y * factor..((y + 1) * factor).min(h) {
                    for xx in x * factor..((x + 1) * factor).min(w) {
                        let i = (yy * w + xx) * 4;
                        let a = f32::from(data[i + 3]) / 255.0;
                        let l = 0.2126 * f32::from(data[i])
                            + 0.7152 * f32::from(data[i + 1])
                            + 0.0722 * f32::from(data[i + 2]);
                        sum += l * a;
                        n += 1.0;
                    }
                }
                d[y * w2 + x] = sum / n;
            }
        }
        Grey { w: w2, h: h2, d }
    }

    fn half(&self) -> Grey {
        let (w2, h2) = (self.w.div_ceil(2).max(1), self.h.div_ceil(2).max(1));
        let mut d = vec![0.0f32; w2 * h2];
        for y in 0..h2 {
            for x in 0..w2 {
                let (mut sum, mut n) = (0.0, 0.0);
                for yy in y * 2..(y * 2 + 2).min(self.h) {
                    for xx in x * 2..(x * 2 + 2).min(self.w) {
                        sum += self.d[yy * self.w + xx];
                        n += 1.0;
                    }
                }
                d[y * w2 + x] = sum / n;
            }
        }
        Grey { w: w2, h: h2, d }
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        let x = x.clamp(0, self.w as i32 - 1) as usize;
        let y = y.clamp(0, self.h as i32 - 1) as usize;
        self.d[y * self.w + x]
    }
}

/// Motion samples on a grid: one every `STEP` pixels of a level.
pub(crate) struct Flow {
    gw: usize,
    gh: usize,
    v: Vec<[f32; 2]>,
}

impl Flow {
    fn new(w: usize, h: usize) -> Flow {
        let (gw, gh) = (w.div_ceil(STEP).max(1), h.div_ceil(STEP).max(1));
        Flow {
            gw,
            gh,
            v: vec![[0.0; 2]; gw * gh],
        }
    }

    /// The motion at the pixel (`x`, `y`) of the level it was found in, interpolated.
    pub(crate) fn at(&self, x: f32, y: f32) -> [f32; 2] {
        let gx = ((x - STEP as f32 / 2.0) / STEP as f32).clamp(0.0, (self.gw - 1) as f32);
        let gy = ((y - STEP as f32 / 2.0) / STEP as f32).clamp(0.0, (self.gh - 1) as f32);
        let (x0, y0) = (gx as usize, gy as usize);
        let (x1, y1) = ((x0 + 1).min(self.gw - 1), (y0 + 1).min(self.gh - 1));
        let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
        let get = |x: usize, y: usize| self.v[y * self.gw + x];
        let mix2 =
            |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        mix2(
            mix2(get(x0, y0), get(x1, y0), fx),
            mix2(get(x0, y1), get(x1, y1), fx),
            fy,
        )
    }

    fn smoothed(&self) -> Flow {
        let mut out = Flow::new(self.gw * STEP, self.gh * STEP);
        for y in 0..self.gh {
            for x in 0..self.gw {
                let (mut sum, mut n) = ([0.0f32; 2], 0.0);
                for yy in y.saturating_sub(1)..(y + 2).min(self.gh) {
                    for xx in x.saturating_sub(1)..(x + 2).min(self.gw) {
                        let v = self.v[yy * self.gw + xx];
                        sum[0] += v[0];
                        sum[1] += v[1];
                        n += 1.0;
                    }
                }
                out.v[y * self.gw + x] = [sum[0] / n, sum[1] / n];
            }
        }
        out
    }
}

/// The motion from picture `a` to picture `b` at one level, starting from `prior` (the motion of
/// the coarser level, in its own pixels).
fn level_flow(a: &Grey, b: &Grey, prior: Option<&Flow>) -> Flow {
    let mut flow = Flow::new(a.w, a.h);
    for gy in 0..flow.gh {
        for gx in 0..flow.gw {
            let (cx, cy) = ((gx * STEP + STEP / 2) as i32, (gy * STEP + STEP / 2) as i32);
            let start = prior.map_or([0.0, 0.0], |p| {
                let v = p.at(cx as f32 / 2.0, cy as f32 / 2.0);
                [(v[0] * 2.0).round(), (v[1] * 2.0).round()]
            });
            let (mut best, mut best_cost) = (start, f32::MAX);
            for dy in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    let (ox, oy) = (start[0] as i32 + dx, start[1] as i32 + dy);
                    let mut sad = 0.0;
                    for by in -BLOCK / 2..BLOCK / 2 {
                        for bx in -BLOCK / 2..BLOCK / 2 {
                            sad +=
                                (a.at(cx + bx, cy + by) - b.at(cx + bx + ox, cy + by + oy)).abs();
                        }
                    }
                    // A small lean to the prior motion keeps flat areas from wandering.
                    let cost = sad + 0.4 * (dx.abs() + dy.abs()) as f32;
                    if cost < best_cost {
                        best_cost = cost;
                        best = [ox as f32, oy as f32];
                    }
                }
            }
            flow.v[gy * flow.gw + gx] = best;
        }
    }
    flow.smoothed()
}

/// The motion from `first` to `second` (both `w` x `h` RGBA8) as a field over the reduced
/// picture, and the factor from its pixels to the picture's.
pub(crate) fn find_motion(
    first: &[u8],
    second: &[u8],
    w: usize,
    h: usize,
) -> (Flow, usize, usize, f32) {
    let factor = w.max(h).div_ceil(SEARCH_SIDE).max(1);
    let mut a = vec![Grey::reduced(first, w, h, factor)];
    let mut b = vec![Grey::reduced(second, w, h, factor)];
    while a.len() < LEVELS {
        let last = &a[a.len() - 1];
        if last.w.min(last.h) / 2 < SMALLEST {
            break;
        }
        let (na, nb) = (last.half(), b[b.len() - 1].half());
        a.push(na);
        b.push(nb);
    }
    let mut flow: Option<Flow> = None;
    for level in (0..a.len()).rev() {
        flow = Some(level_flow(&a[level], &b[level], flow.as_ref()));
    }
    let flow = flow.unwrap_or_else(|| Flow::new(a[0].w, a[0].h));
    (flow, a[0].w, a[0].h, factor as f32)
}

/// Makes `second` the picture `t` of the way through a smooth cut from `first`. Returns whether
/// `kind` is drawn here.
pub(crate) fn mix_into(
    kind: TransitionKind,
    first: &[u8],
    second: &mut [u8],
    w: usize,
    h: usize,
    t: f32,
) -> bool {
    if !handles(kind) {
        return false;
    }
    let Some(len) = w.checked_mul(h).and_then(|n| n.checked_mul(4)) else {
        return true;
    };
    if w == 0 || h == 0 || first.len() < len || second.len() < len {
        return true;
    }
    let t = t.clamp(0.0, 1.0);
    let incoming = second[..len].to_vec();
    let (flow, rw, rh, _) = find_motion(&first[..len], &incoming, w, h);
    let (a, b) = (
        Pic {
            data: &first[..len],
            w,
            h,
        },
        Pic {
            data: &incoming,
            w,
            h,
        },
    );
    let (sx, sy) = (rw as f32 / w as f32, rh as f32 / h as f32);
    picasso_color::par::par_rows(&mut second[..len], w * 4, h, &|first_row, out| {
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let n = first_row * w + i;
            let (x, y) = ((n % w) as f32 + 0.5, (n / w) as f32 + 0.5);
            // The motion in the picture's own pixels.
            let f = flow.at(x * sx, y * sy);
            let (fx, fy) = (f[0] / sx, f[1] / sy);
            let pa = v2((x - t * fx) / w as f32, (y - t * fy) / h as f32);
            let pb = v2(
                (x + (1.0 - t) * fx) / w as f32,
                (y + (1.0 - t) * fy) / h as f32,
            );
            let c = cmix(a.tex(pa), b.tex(pb), t);
            let al = c[3].clamp(0.0, 1.0);
            let k = if al > 1e-6 { 1.0 / al } else { 0.0 };
            for ch in 0..3 {
                px[ch] = (c[ch] * k * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
            px[3] = (al * 255.0 + 0.5) as u8;
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 160;
    const H: usize = 90;

    /// Smooth value noise from 0 to 1 that never repeats.
    fn noise(x: f32, y: f32) -> f32 {
        let lattice = |ix: i32, iy: i32| {
            let mut h =
                (ix as u32).wrapping_mul(0x8da6_b343) ^ (iy as u32).wrapping_mul(0xd816_3841);
            h ^= h >> 15;
            h = h.wrapping_mul(0x2c1b_3c6d);
            h ^= h >> 12;
            (h & 0xffff) as f32 / 65535.0
        };
        let (fx, fy) = (x.floor(), y.floor());
        let (ix, iy) = (fx as i32, fy as i32);
        let s = |t: f32| t * t * (3.0 - 2.0 * t);
        let (tx, ty) = (s(x - fx), s(y - fy));
        let top = lattice(ix, iy) + (lattice(ix + 1, iy) - lattice(ix, iy)) * tx;
        let bottom = lattice(ix, iy + 1) + (lattice(ix + 1, iy + 1) - lattice(ix, iy + 1)) * tx;
        top + (bottom - top) * ty
    }

    /// A textured scene seen from `x0` pixels to the right: a pan moves it to the left.
    fn scene(x0: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let g = 40.0 + 170.0 * noise((x + x0) as f32 / 9.0, y as f32 / 9.0);
                let g = g.clamp(0.0, 255.0) as u8;
                v.extend_from_slice(&[g, g, g, 255]);
            }
        }
        v
    }

    fn run(a: &[u8], b: &[u8], t: f32) -> Vec<u8> {
        let mut out = b.to_vec();
        assert!(mix_into(TransitionKind::SmoothCut, a, &mut out, W, H, t));
        out
    }

    #[test]
    fn it_starts_as_the_first_picture_and_ends_as_the_second() {
        let (a, b) = (scene(0), scene(24));
        let start = run(&a, &b, 0.0);
        let end = run(&a, &b, 1.0);
        let worst =
            |x: &[u8], y: &[u8]| x.iter().zip(y).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
        assert!(worst(&start, &a) <= 12, "{}", worst(&start, &a));
        assert!(worst(&end, &b) <= 12, "{}", worst(&end, &b));
    }

    #[test]
    fn a_pan_is_played_as_a_pan_and_not_as_two_pictures_fading() {
        let (a, b) = (scene(0), scene(24));
        let mid = run(&a, &b, 0.5);
        // Halfway the view is 12 pixels along; a plain dissolve would blur two views together.
        let ideal = scene(12);
        let error = |p: &[u8]| {
            (20..W - 20)
                .flat_map(|x| (0..H).map(move |y| (x, y)))
                .map(|(x, y)| u64::from(p[(y * W + x) * 4].abs_diff(ideal[(y * W + x) * 4])))
                .sum::<u64>()
        };
        let dissolve: Vec<u8> = a
            .iter()
            .zip(&b)
            .map(|(p, q)| ((u16::from(*p) + u16::from(*q)) / 2) as u8)
            .collect();
        assert!(
            error(&mid) * 3 < error(&dissolve),
            "morph {} vs dissolve {}",
            error(&mid),
            error(&dissolve)
        );
    }

    #[test]
    fn the_same_picture_stays_as_it_is() {
        let a = scene(50);
        let mid = run(&a, &a, 0.5);
        let worst = mid
            .iter()
            .zip(&a)
            .map(|(p, q)| p.abs_diff(*q))
            .max()
            .unwrap();
        assert!(worst <= 6, "{worst}");
    }

    #[test]
    fn odd_sizes_and_short_buffers_do_not_crash() {
        let mut one = vec![10, 20, 30, 255];
        mix_into(
            TransitionKind::SmoothCut,
            &[1, 2, 3, 255],
            &mut one,
            1,
            1,
            0.5,
        );
        mix_into(TransitionKind::SmoothCut, &[], &mut [], 0, 0, 0.5);
        let mut short = vec![0u8; 8];
        mix_into(TransitionKind::SmoothCut, &[0u8; 8], &mut short, 5, 5, 0.5);
        assert!(!mix_into(
            TransitionKind::CrossDissolve,
            &[],
            &mut [],
            0,
            0,
            0.5
        ));
    }

    /// Not a check, a measurement:
    /// `cargo test -p picasso-render --release smooth_cut::tests::timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_at_1080p() {
        let (w, h) = (1920, 1080);
        let a: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let mut b: Vec<u8> = a.iter().rev().copied().collect();
        let start = std::time::Instant::now();
        mix_into(TransitionKind::SmoothCut, &a, &mut b, w, h, 0.5);
        eprintln!("smooth cut at 1080p: {} ms", start.elapsed().as_millis());
    }
}
