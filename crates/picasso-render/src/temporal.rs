//! Effects that read other frames of the clip: Echo, Strobe, Motion Trails, Smear, Motion Blur,
//! Frame Replacer, Deflicker, Color Stabilizer and Automatic Dirt Removal.
//!
//! The compositor draws the clip at the frames an effect asks for ([`offsets`]) and hands them
//! over with the current picture, so the viewer, playback and export draw the same picture for the
//! same frame and nothing builds up from frame to frame. A frame past either end of the clip is
//! the nearest one the clip has. Each effect has one strength, 0 to 100, that scales its main
//! setting from nothing (0) through the usual value (50) to twice as much (100).

use picasso_color::EffectKind;

use crate::smooth_cut::find_motion;
use crate::transitions::{cadd, cmix, cscale, v2, Pic, C4};

/// Which frames, as offsets from the current one (negative: earlier), the effect reads at
/// strength `a` (0 to 1) when the current frame is `local` frames into the clip.
pub(crate) fn offsets(kind: EffectKind, a: f32, local: u64) -> Vec<i64> {
    match kind {
        EffectKind::Echo => (1..=3).map(|i| -2 * i).collect(),
        EffectKind::Strobe => vec![-((local % strobe_hold(a)) as i64)],
        EffectKind::MotionTrails => (1..=4).map(|i| -i).collect(),
        EffectKind::Smear => vec![-2, -1, 1, 2],
        EffectKind::MotionBlur => vec![-1, 1],
        EffectKind::FrameReplacerPrevious => vec![-1],
        EffectKind::FrameReplacerNext => vec![1],
        EffectKind::FrameReplacerMix => vec![-1, 1],
        EffectKind::Deflicker => vec![-2, -1, 1, 2],
        EffectKind::ColorStabilizer => vec![-(local as i64)],
        EffectKind::DirtRemoval => vec![-1, 1],
        _ => Vec::new(),
    }
}

fn strobe_hold(a: f32) -> u64 {
    ((a * 6.0).round() as u64).max(2)
}

fn pixel(data: &[u8], i: usize) -> C4 {
    let a = f32::from(data[i + 3]) / 255.0;
    [
        f32::from(data[i]) / 255.0 * a,
        f32::from(data[i + 1]) / 255.0 * a,
        f32::from(data[i + 2]) / 255.0 * a,
        a,
    ]
}

fn store(out: &mut [u8], i: usize, c: C4) {
    let a = c[3].clamp(0.0, 1.0);
    let k = if a > 1e-6 { 1.0 / a } else { 0.0 };
    for ch in 0..3 {
        out[i + ch] = (c[ch] * k * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    out[i + 3] = (a * 255.0 + 0.5) as u8;
}

/// Every pixel of `layer` becomes `f(current, neighbours)`, with the neighbours' pixels at the
/// same place.
fn per_pixel(layer: &mut [u8], neighbours: &[&[u8]], f: &(dyn Fn(C4, &[C4]) -> C4 + Sync)) {
    let len = layer.len();
    let w4 = 4;
    let original = layer.to_vec();
    picasso_color::par::par_rows(layer, w4, len / w4, &|first, band| {
        let mut around = vec![[0.0f32; 4]; neighbours.len()];
        for (n, px) in band.chunks_exact_mut(4).enumerate() {
            let i = (first + n) * 4;
            for (slot, nb) in around.iter_mut().zip(neighbours) {
                *slot = pixel(nb, i);
            }
            store(px, 0, f(pixel(&original, i), &around));
        }
    });
}

fn trail_weights(trails: usize, fade: f32, opacity: f32) -> Vec<f32> {
    let keep = 1.0 - fade.clamp(0.0, 1.0);
    let mut w = vec![1.0];
    let mut trail = opacity;
    for _ in 0..trails {
        trail *= keep;
        w.push(trail);
    }
    w
}

/// The mean colour of a picture, premultiplied.
fn mean(data: &[u8]) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    let mut n = 0.0;
    for px in data.chunks_exact(4) {
        let a = f64::from(px[3]) / 255.0;
        for c in 0..3 {
            sum[c] += f64::from(px[c]) / 255.0 * a;
        }
        n += 1.0;
    }
    if n == 0.0 {
        return [0.0; 3];
    }
    sum.map(|v| (v / n) as f32)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Per-channel gains that bring brightness (or colour) toward a target, limited and mixed in by
/// the strength.
fn limited(gain: [f32; 3], strength: f32, limit: f32) -> [f32; 3] {
    let m = 1.0 + limit;
    gain.map(|g| 1.0 + strength * (g.clamp(1.0 / m, m) - 1.0))
}

fn apply_gain(layer: &mut [u8], gain: [f32; 3]) {
    for px in layer.chunks_exact_mut(4) {
        for c in 0..3 {
            px[c] = (f32::from(px[c]) * gain[c]).round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn median4(mut v: [f32; 4]) -> (f32, f32) {
    v.sort_by(f32::total_cmp);
    (0.5 * (v[1] + v[2]), v[2] - v[1])
}

/// The middle of the neighbours at a pixel (the mean of the two middle values of four, with the
/// two neighbours doubled) and how far apart the middle values are.
fn median_of(neighbours: &[C4]) -> (C4, f32) {
    let pick = |i: usize| neighbours[i.min(neighbours.len() - 1)];
    let n = [pick(0), pick(1), pick(0), pick(1)];
    let mut out = [0.0f32; 4];
    let mut widest = 0.0f32;
    for c in 0..4 {
        let (m, spread) = median4([n[0][c], n[1][c], n[2][c], n[3][c]]);
        out[c] = m;
        widest = widest.max(spread);
    }
    (out, widest)
}

/// Blurs the layer along the motion measured to the next frame and from the previous one.
fn motion_blur(layer: &mut [u8], w: usize, h: usize, a: f32, previous: &[u8], next: &[u8]) {
    let shutter = (a * 2.0 * 180.0) / 360.0;
    if shutter <= 0.0 {
        return;
    }
    let samples = 12usize;
    let (fwd, fw, fh, _) = find_motion(layer, next, w, h);
    let (bwd, ..) = find_motion(previous, layer, w, h);
    let (sx, sy) = (fw as f32 / w as f32, fh as f32 / h as f32);
    let original = layer.to_vec();
    let src = Pic {
        data: &original,
        w,
        h,
    };
    picasso_color::par::par_rows(layer, w * 4, h, &|first, band| {
        for (n, px) in band.chunks_exact_mut(4).enumerate() {
            let idx = first * w + n;
            let (x, y) = ((idx % w) as f32 + 0.5, (idx / w) as f32 + 0.5);
            let still = |v: [f32; 2]| {
                // Motion under one pixel is below what the search can tell from noise.
                let len = v[0].hypot(v[1]);
                let t = ((len - 0.5) / 0.5).clamp(0.0, 1.0);
                let k = t * t * (3.0 - 2.0 * t);
                [v[0] / sx * k, v[1] / sy * k]
            };
            let f = still(fwd.at(x * sx, y * sy));
            let b = still(bwd.at(x * sx, y * sy));
            let mut sum = [0.0f32; 4];
            for i in 0..samples {
                let t = ((i as f32 + 0.5) / samples as f32 - 0.5) * shutter;
                let m = if t > 0.0 { f } else { b };
                let p = v2((x - m[0] * t) / w as f32, (y - m[1] * t) / h as f32);
                sum = cadd(sum, src.tex(p));
            }
            store(px, 0, cscale(sum, 1.0 / samples as f32));
        }
    });
}

/// Applies `kind` at strength `a` (0 to 1) to `layer` (`w` x `h` RGBA8). `local` is how many
/// frames into the clip it is; `frames` holds the pictures asked for by [`offsets`], in order.
pub(crate) fn apply(
    kind: EffectKind,
    a: f32,
    layer: &mut [u8],
    (w, h): (usize, usize),
    local: u64,
    frames: &[Vec<u8>],
) {
    let want = offsets(kind, a, local).len();
    if a <= 0.0 || frames.len() != want || w == 0 || h == 0 || layer.len() < w * h * 4 {
        return;
    }
    let len = w * h * 4;
    if frames.iter().any(|f| f.len() < len) {
        return;
    }
    let refs: Vec<&[u8]> = frames.iter().map(|f| &f[..len]).collect();
    let strength = (a * 2.0).clamp(0.0, 1.0);
    let layer = &mut layer[..len];
    match kind {
        EffectKind::Echo => {
            // The frame and its echoes, each fainter, as an average that still adds up to one.
            let w = trail_weights(3, 0.5, strength);
            let total: f32 = w.iter().sum();
            per_pixel(layer, &refs, &move |cur, n| {
                let mut out = cscale(cur, w[0] / total);
                for (i, c) in n.iter().enumerate() {
                    out = cadd(out, cscale(*c, w[i + 1] / total));
                }
                out
            });
        }
        EffectKind::Strobe => {
            if let Some(held) = refs.first() {
                layer.copy_from_slice(held);
            }
        }
        EffectKind::MotionTrails => {
            // Lighten: the newest light wins; an old trail fades into the picture, not black.
            let w = trail_weights(4, 0.35, strength);
            per_pixel(layer, &refs, &move |cur, n| {
                let mut acc = cur;
                for (i, c) in n.iter().enumerate() {
                    let faded = cmix(cur, *c, w[i + 1]);
                    for ch in 0..4 {
                        acc[ch] = acc[ch].max(faded[ch]);
                    }
                }
                acc
            });
        }
        EffectKind::Smear => {
            let keep = 0.7f32;
            let distances = [2.0f32, 1.0, 1.0, 2.0];
            let raw: Vec<f32> = distances.iter().map(|d| keep.powf(*d)).collect();
            let total = 1.0 + raw.iter().sum::<f32>();
            let centre = 1.0 - strength + strength / total;
            per_pixel(layer, &refs, &move |cur, n| {
                let mut out = cscale(cur, centre);
                for (i, c) in n.iter().enumerate() {
                    out = cadd(out, cscale(*c, strength * raw[i] / total));
                }
                out
            });
        }
        EffectKind::MotionBlur => {
            if let [previous, next] = refs[..] {
                motion_blur(layer, w, h, a, previous, next);
            }
        }
        EffectKind::FrameReplacerPrevious | EffectKind::FrameReplacerNext => {
            per_pixel(layer, &refs, &move |cur, n| cmix(cur, n[0], strength));
        }
        EffectKind::FrameReplacerMix => {
            per_pixel(layer, &refs, &move |cur, n| {
                cmix(cur, cscale(cadd(n[0], n[1]), 0.5), strength)
            });
        }
        EffectKind::Deflicker => {
            // Pull the frame's brightness toward the mean of the frames around it.
            let own = mean(layer);
            let mut around = own;
            for f in &refs {
                let m = mean(f);
                for c in 0..3 {
                    around[c] += m[c];
                }
            }
            let count = (refs.len() + 1) as f32;
            let target = around.map(|v| v / count);
            let lc = luma(own);
            if lc > 1e-4 {
                let x = luma(target) / lc;
                apply_gain(layer, limited([x; 3], strength, 0.5));
            }
        }
        EffectKind::ColorStabilizer => {
            // Match the frame's colour to the first frame of the clip.
            let own = mean(layer);
            let reference = mean(refs[0]);
            let (lc, lr) = (luma(own), luma(reference));
            if lc > 1e-4 && lr > 1e-4 {
                let gain = [0, 1, 2].map(|c| reference[c] / own[c].max(1e-4));
                apply_gain(layer, limited(gain, strength, 0.5));
            }
        }
        EffectKind::DirtRemoval => {
            // Dust and hairs show in one frame only: where the frame differs from the middle of
            // the frames around it by more than the contrast, and those two agree, it is dirt.
            let (contrast, motion) = (0.15f32, 0.08f32);
            // Dirt is grown by a pixel so its soft edge goes too.
            let dirt: Vec<bool> = {
                let mut d = vec![false; w * h];
                for (i, flag) in d.iter_mut().enumerate() {
                    let cur = pixel(layer, i * 4);
                    let n = [pixel(refs[0], i * 4), pixel(refs[1], i * 4)];
                    let (m, spread) = median_of(&n);
                    let diff = (0..4).map(|c| (cur[c] - m[c]).abs()).fold(0.0f32, f32::max);
                    *flag = diff > contrast && spread < motion;
                }
                d
            };
            let grown: Vec<bool> = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    (y.saturating_sub(1)..(y + 2).min(h))
                        .any(|yy| (x.saturating_sub(1)..(x + 2).min(w)).any(|xx| dirt[yy * w + xx]))
                })
                .collect();
            let original = layer.to_vec();
            for (index, px) in layer.chunks_exact_mut(4).enumerate() {
                if grown[index] {
                    let n = [pixel(refs[0], index * 4), pixel(refs[1], index * 4)];
                    let (m, _) = median_of(&n);
                    let cur = pixel(&original, index * 4);
                    store(px, 0, cmix(cur, m, strength));
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 96;
    const H: usize = 54;

    fn solid(g: u8) -> Vec<u8> {
        (0..W * H).flat_map(|_| [g, g, g, 255]).collect()
    }

    fn at(p: &[u8], x: usize, y: usize) -> u8 {
        p[(y * W + x) * 4]
    }

    const ALL: [EffectKind; 11] = [
        EffectKind::Echo,
        EffectKind::Strobe,
        EffectKind::MotionTrails,
        EffectKind::Smear,
        EffectKind::MotionBlur,
        EffectKind::FrameReplacerPrevious,
        EffectKind::FrameReplacerNext,
        EffectKind::FrameReplacerMix,
        EffectKind::Deflicker,
        EffectKind::ColorStabilizer,
        EffectKind::DirtRemoval,
    ];

    fn run(kind: EffectKind, a: f32, layer: &[u8], local: u64, frames: &[Vec<u8>]) -> Vec<u8> {
        let mut out = layer.to_vec();
        apply(kind, a, &mut out, (W, H), local, frames);
        out
    }

    #[test]
    fn every_effect_says_which_frames_it_reads_and_only_those_are_temporal() {
        for kind in ALL {
            assert!(kind.is_temporal(), "{kind:?}");
            assert!(!offsets(kind, 0.5, 7).is_empty(), "{kind:?}");
        }
        assert!(!EffectKind::Glow.is_temporal());
        assert!(offsets(EffectKind::Glow, 0.5, 7).is_empty());
        // The clip's first frame is the stabiliser's reference, wherever the clip is.
        assert_eq!(offsets(EffectKind::ColorStabilizer, 0.5, 9), vec![-9]);
        // A strobe holds frames in fours of the clip, not of the timeline.
        assert_eq!(offsets(EffectKind::Strobe, 0.5, 7), vec![-1]);
        assert_eq!(offsets(EffectKind::Strobe, 0.5, 9), vec![0]);
    }

    #[test]
    fn nothing_changes_at_zero_or_with_the_wrong_frames() {
        let cur = solid(80);
        for kind in ALL {
            let frames: Vec<Vec<u8>> = offsets(kind, 0.5, 5).iter().map(|_| solid(200)).collect();
            assert_eq!(run(kind, 0.0, &cur, 5, &frames), cur, "{kind:?} at zero");
            assert_eq!(run(kind, 0.5, &cur, 5, &[]), cur, "{kind:?} without frames");
            assert_eq!(
                run(kind, 0.5, &cur, 5, &[vec![0; 4]]),
                cur,
                "{kind:?} with a short frame"
            );
        }
    }

    #[test]
    fn echo_trails_and_smear_blend_the_other_frames_in() {
        let cur = solid(0);
        let white = |n: usize| vec![solid(255); n];
        let echo = run(EffectKind::Echo, 0.5, &cur, 5, &white(3));
        assert!(
            at(&echo, 3, 3) > 40 && at(&echo, 3, 3) < 200,
            "{}",
            at(&echo, 3, 3)
        );
        let trails = run(EffectKind::MotionTrails, 0.5, &cur, 5, &white(4));
        assert!(at(&trails, 3, 3) > 100, "lighten keeps the bright trail");
        let smear = run(EffectKind::Smear, 0.5, &cur, 5, &white(4));
        assert!(at(&smear, 3, 3) > 40 && at(&smear, 3, 3) < 255);
        // A still picture stays as it is, whatever the weights.
        let still = solid(120);
        for (kind, n) in [(EffectKind::Echo, 3), (EffectKind::Smear, 4)] {
            let out = run(kind, 0.5, &still, 5, &vec![solid(120); n]);
            assert!(at(&out, 3, 3).abs_diff(120) <= 1, "{kind:?}");
        }
    }

    #[test]
    fn strobe_and_the_frame_replacers_show_the_frame_asked_for() {
        let cur = solid(10);
        let held = solid(200);
        assert_eq!(
            run(
                EffectKind::Strobe,
                0.5,
                &cur,
                5,
                std::slice::from_ref(&held)
            ),
            held
        );
        assert_eq!(
            run(
                EffectKind::FrameReplacerPrevious,
                0.5,
                &cur,
                5,
                std::slice::from_ref(&held)
            ),
            held
        );
        assert_eq!(
            run(
                EffectKind::FrameReplacerNext,
                0.5,
                &cur,
                5,
                std::slice::from_ref(&held)
            ),
            held
        );
        let mix = run(
            EffectKind::FrameReplacerMix,
            0.5,
            &cur,
            5,
            &[solid(100), solid(200)],
        );
        assert!(at(&mix, 3, 3).abs_diff(150) <= 1);
        // Half strength is half way.
        let half = run(
            EffectKind::FrameReplacerPrevious,
            0.25,
            &cur,
            5,
            &[solid(110)],
        );
        assert!(at(&half, 3, 3).abs_diff(60) <= 1);
    }

    #[test]
    fn deflicker_and_the_stabiliser_pull_brightness_toward_their_target() {
        let dark = solid(80);
        let around = vec![solid(120); 4];
        let out = run(EffectKind::Deflicker, 0.5, &dark, 5, &around);
        assert!(
            at(&out, 3, 3) > 95 && at(&out, 3, 3) <= 120,
            "{}",
            at(&out, 3, 3)
        );
        let out = run(
            EffectKind::ColorStabilizer,
            0.5,
            &solid(100),
            5,
            &[solid(140)],
        );
        assert!(
            at(&out, 3, 3) > 125 && at(&out, 3, 3) <= 140,
            "{}",
            at(&out, 3, 3)
        );
        // Already matching: untouched.
        assert_eq!(
            run(EffectKind::Deflicker, 0.5, &solid(120), 5, &around),
            solid(120)
        );
    }

    #[test]
    fn dirt_in_one_frame_is_removed_and_a_real_edge_in_all_of_them_is_not() {
        let flat = solid(100);
        let mut dirty = solid(100);
        for y in 20..23 {
            for x in 40..43 {
                dirty[(y * W + x) * 4..(y * W + x) * 4 + 3].fill(250);
            }
        }
        let out = run(
            EffectKind::DirtRemoval,
            0.5,
            &dirty,
            5,
            &[flat.clone(), flat.clone()],
        );
        assert!(
            at(&out, 41, 21).abs_diff(100) <= 2,
            "the speck is gone: {}",
            at(&out, 41, 21)
        );
        // The same bright patch in all three frames is picture, not dirt.
        let kept = run(
            EffectKind::DirtRemoval,
            0.5,
            &dirty,
            5,
            &[dirty.clone(), dirty.clone()],
        );
        assert_eq!(at(&kept, 41, 21), 250);
    }

    #[test]
    fn motion_blur_smears_a_moving_square_along_its_path() {
        let square = |x0: usize| {
            let mut v = solid(20);
            for y in 20..34 {
                for x in x0..x0 + 14 {
                    v[(y * W + x) * 4..(y * W + x) * 4 + 3].fill(240);
                }
            }
            v
        };
        let (prev, cur, next) = (square(18), square(30), square(42));
        let out = run(EffectKind::MotionBlur, 1.0, &cur, 5, &[prev, next]);
        // Just beyond the square's edge, where it came from or is going, there is some of it.
        assert!(
            at(&out, 29, 27) > 40 || at(&out, 44, 27) > 40,
            "{} {}",
            at(&out, 29, 27),
            at(&out, 44, 27)
        );
        // Far from the square nothing happens.
        assert_eq!(at(&out, 5, 5), 20);
        // A still scene is not blurred.
        let still = square(30);
        let same = run(
            EffectKind::MotionBlur,
            1.0,
            &still,
            5,
            &[still.clone(), still.clone()],
        );
        assert!(same.iter().zip(&still).all(|(a, b)| a.abs_diff(*b) <= 2));
    }
}
