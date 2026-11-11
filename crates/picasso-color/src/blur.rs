//! Blurs and their relatives: Box Blur, Radial (spin) Blur, Zoom Blur, Tilt-Shift, Lens Blur,
//! Noise Reduction and Unsharp Mask.
//!
//! Each has the one strength every effect has, 0 to 100, driving its main setting; the rest sit
//! at their usual values. Sizes follow the picture's height (1080 lines is the reference), so an
//! effect looks the same at any resolution. The blurs that follow a path (spin, zoom) average
//! 24 samples along it; a path too long for 24 samples to cover is done in two passes (the first
//! over a 24th of the path, the second over the whole), which together equal a continuous average
//! along it. Samples outside the picture repeat its edge.

use crate::fx::{box_pass, gaussian, EffectKind};
use crate::look::{par_pixels, put, Src};
use crate::LUMA;

/// Samples along a path.
const PATH_TAPS: usize = 24;
/// A path whose samples would be farther apart than this (pixels) is done in two passes.
const MAX_TAP_SPACING: f32 = 3.0;
/// Zoom Blur never reaches the centre.
const MAX_ZOOM_AMOUNT: f32 = 0.9;
/// Samples of the disc of Lens Blur.
const BOKEH_TAPS: usize = 64;
/// Largest radius of Noise Reduction's filter, pixels.
const MAX_DENOISE_RADIUS: i64 = 6;

fn unit(h: usize) -> f32 {
    h as f32 / 1080.0
}

fn luma_of(c: [f32; 3]) -> f32 {
    c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Averages `PATH_TAPS` samples; `path(x, y, t, fraction)` gives where the sample at position
/// `t` (0 to 1) of the path through the pixel (x, y) is read, for `fraction` of the whole path.
fn path_blur(
    rgba: &mut [u8],
    w: usize,
    h: usize,
    length_px: f32,
    path: &(dyn Fn(f32, f32, f32, f32) -> (f32, f32) + Sync),
) {
    let pass = |rgba: &mut [u8], fraction: f32| {
        let original = rgba.to_vec();
        let src = Src::new(&original, w, h);
        par_pixels(rgba, w, h, &|x, y, px| {
            let mut sum = [0.0f32; 4];
            for i in 0..PATH_TAPS {
                let t = (i as f32 + 0.5) / PATH_TAPS as f32;
                let (sx, sy) = path(x as f32 + 0.5, y as f32 + 0.5, t, fraction);
                let c = src.at(sx, sy);
                for k in 0..4 {
                    sum[k] += c[k];
                }
            }
            put(px, sum.map(|v| v / PATH_TAPS as f32));
        });
    };
    if length_px / PATH_TAPS as f32 > MAX_TAP_SPACING {
        pass(rgba, 1.0 / PATH_TAPS as f32);
    }
    pass(rgba, 1.0);
}

/// The farthest corner of the picture from (cx, cy), pixels.
fn farthest(w: usize, h: usize, cx: f32, cy: f32) -> f32 {
    (cx.max(w as f32 - cx)).hypot(cy.max(h as f32 - cy))
}

fn box_blur(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let r = (a * 40.0 * unit(h)).round() as usize;
    if r == 0 {
        return;
    }
    let r = r.min(w.max(h));
    let mut buf: Vec<f32> = rgba.iter().map(|v| f32::from(*v)).collect();
    box_pass(&mut buf, w, h, r, true);
    box_pass(&mut buf, w, h, r, false);
    for (v, b) in rgba.iter_mut().zip(buf) {
        *v = b.round().clamp(0.0, 255.0) as u8;
    }
}

fn radial_blur(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    // Up to 20 degrees of turn about the middle.
    let angle = (a * 20.0).to_radians();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let length = angle * farthest(w, h, cx, cy);
    if length < 0.5 {
        return;
    }
    path_blur(rgba, w, h, length, &move |x, y, t, fraction| {
        let ang = angle * fraction * (t - 0.5);
        let (s, c) = ang.sin_cos();
        let (px, py) = (x - cx, y - cy);
        (cx + c * px - s * py, cy + s * px + c * py)
    });
}

fn zoom_blur(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let amount = (a * 0.4).clamp(0.0, MAX_ZOOM_AMOUNT);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let length = amount * farthest(w, h, cx, cy);
    if length < 0.5 {
        return;
    }
    let log_scale = (1.0 - amount).ln();
    path_blur(rgba, w, h, length, &move |x, y, t, fraction| {
        let k = (log_scale * fraction * t).exp();
        (cx + (x - cx) * k, cy + (y - cy) * k)
    });
}

fn tilt_shift(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let radius = (a * 24.0 * unit(h)).round() as usize;
    if radius < 1 {
        return;
    }
    let mut strong = rgba.to_vec();
    gaussian(&mut strong, w, h, radius);
    let mut light = rgba.to_vec();
    gaussian(&mut light, w, h, (radius / 3).max(1));
    let u = unit(h);
    let half_band = (0.5 * 120.0 * u).max(0.0);
    let transition = (200.0 * u).max(1.0);
    let cy = h as f32 / 2.0;
    par_pixels(rgba, w, h, &|x, y, px| {
        let d = (y as f32 + 0.5 - cy).abs();
        let m = smoothstep(half_band, half_band + transition, d);
        let i = (y * w + x) * 4;
        // Three levels so the blur grows gradually away from the band.
        for c in 0..4 {
            let (s, l, st) = (
                f32::from(px[c]),
                f32::from(light[i + c]),
                f32::from(strong[i + c]),
            );
            let v = if m < 0.5 {
                s + (l - s) * (m * 2.0)
            } else {
                l + (st - l) * (m * 2.0 - 1.0)
            };
            px[c] = v.round().clamp(0.0, 255.0) as u8;
        }
    });
}

fn lens_blur(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let radius = a * 20.0 * unit(h);
    if radius < 0.5 {
        return;
    }
    // Highlights spread into bright discs, not grey haze: bright samples weigh more.
    let (boost, threshold) = (8.0 * 0.4, 0.75);
    let golden = 2.399_963_2f32;
    let offsets: Vec<(f32, f32)> = (0..BOKEH_TAPS)
        .map(|i| {
            let r = ((i as f32 + 0.5) / BOKEH_TAPS as f32).sqrt() * radius;
            let ang = i as f32 * golden;
            (ang.cos() * r, ang.sin() * r)
        })
        .collect();
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    par_pixels(rgba, w, h, &|x, y, px| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut sum = [0.0f32; 4];
        let mut wsum = 0.0;
        for (ox, oy) in &offsets {
            let c = src.at(fx + ox, fy + oy);
            let straight = if c[3] > 1e-6 {
                [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
            } else {
                [0.0; 3]
            };
            let wt = 1.0 + boost * smoothstep(threshold, 1.0, luma_of(straight));
            for k in 0..4 {
                sum[k] += c[k] * wt;
            }
            wsum += wt;
        }
        put(px, sum.map(|v| v / wsum));
    });
}

fn noise_reduction(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    // A bilateral filter: neighbours count by distance and by colour similarity, so edges keep
    // their sharpness while flat noise averages out.
    let radius_px = (2.0 * unit(h)).clamp(1.0, MAX_DENOISE_RADIUS as f32);
    let radius = (radius_px - 1e-6).ceil() as i64;
    let sigma_s = (radius_px * 0.6).max(0.5);
    // Strength 100 % treats colour differences up to about 0.15 as noise.
    let sigma_r = 0.01 + 0.14 * a;
    let (spatial, range) = (
        1.0 / (2.0 * sigma_s * sigma_s),
        1.0 / (2.0 * sigma_r * sigma_r),
    );
    let mix = (0.5 + a).min(1.0);
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let (wi, hi) = (w as i64, h as i64);
    par_pixels(rgba, w, h, &|x, y, px| {
        let centre = src.texel(x, y);
        let mut sum = [0.0f32; 4];
        let mut wsum = 0.0;
        for dy in -radius..=radius {
            let yy = (y as i64 + dy).clamp(0, hi - 1) as usize;
            for dx in -radius..=radius {
                let xx = (x as i64 + dx).clamp(0, wi - 1) as usize;
                let c = src.texel(xx, yy);
                let d2: f32 = (0..4)
                    .map(|k| (c[k] - centre[k]) * (c[k] - centre[k]))
                    .sum();
                let wt = (-((dx * dx + dy * dy) as f32) * spatial - d2 * range).exp();
                for k in 0..4 {
                    sum[k] += c[k] * wt;
                }
                wsum += wt;
            }
        }
        let out: Vec<f32> = (0..4)
            .map(|k| centre[k] + (sum[k] / wsum - centre[k]) * mix)
            .collect();
        put(px, [out[0], out[1], out[2], out[3]]);
    });
}

fn unsharp_mask(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let mut blurred = rgba.to_vec();
    gaussian(
        &mut blurred,
        w,
        h,
        (2.0 * unit(h)).round().max(1.0) as usize,
    );
    let amount = a * 2.0;
    for (px, b) in rgba.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
        for c in 0..3 {
            let (s, bl) = (f32::from(px[c]), f32::from(b[c]));
            px[c] = (s + amount * (s - bl)).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize) {
    match kind {
        EffectKind::BoxBlur => box_blur(rgba, w, h, a),
        EffectKind::RadialBlur => radial_blur(rgba, w, h, a),
        EffectKind::ZoomBlur => zoom_blur(rgba, w, h, a),
        EffectKind::TiltShift => tilt_shift(rgba, w, h, a),
        EffectKind::LensBlur => lens_blur(rgba, w, h, a),
        EffectKind::NoiseReduction => noise_reduction(rgba, w, h, a),
        EffectKind::UnsharpMask => unsharp_mask(rgba, w, h, a),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 240;
    const H: usize = 180;

    /// A picture full of detail: a hash of the position in every channel.
    fn noisy() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let h = ((x * 73856093) ^ (y * 19349663)) as u32;
                let h = h.wrapping_mul(2654435761);
                v.extend_from_slice(&[(h >> 8) as u8, (h >> 16) as u8, (h >> 24) as u8, 255]);
            }
        }
        v
    }

    fn flat(rgb: [u8; 3]) -> Vec<u8> {
        (0..W * H)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    /// Black on the left half, white on the right.
    fn halves() -> Vec<u8> {
        (0..W * H)
            .flat_map(|i| {
                let v = if i % W < W / 2 { 0 } else { 255 };
                [v, v, v, 255]
            })
            .collect()
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

    fn px(p: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * W + x) * 4;
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    }

    /// How much the picture varies between neighbouring pixels (smaller = blurrier).
    fn roughness(p: &[u8], rows: std::ops::Range<usize>) -> f32 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in rows {
            for x in 1..W {
                sum += (f32::from(px(p, x, y)[0]) - f32::from(px(p, x - 1, y)[0])).abs();
                n += 1.0;
            }
        }
        sum / n
    }

    const ALL: [EffectKind; 7] = [
        EffectKind::BoxBlur,
        EffectKind::RadialBlur,
        EffectKind::ZoomBlur,
        EffectKind::TiltShift,
        EffectKind::LensBlur,
        EffectKind::NoiseReduction,
        EffectKind::UnsharpMask,
    ];

    #[test]
    fn each_changes_a_picture_keeps_its_size_and_is_off_at_zero() {
        let pic = noisy();
        for kind in ALL {
            let out = run(kind, Effect::new(kind).amount, &pic);
            assert_eq!(out.len(), pic.len());
            assert_ne!(out, pic, "{kind:?} does nothing at its starting strength");
            assert_eq!(run(kind, 0.0, &pic), pic, "{kind:?} at 0");
        }
    }

    #[test]
    fn each_survives_tiny_pictures_and_any_strength() {
        for kind in ALL {
            for (w, h) in [(1usize, 1usize), (2, 1), (1, 3), (7, 5), (40, 30)] {
                for amount in [1.0, 50.0, 100.0] {
                    let mut p: Vec<u8> = (0..w * h * 4).map(|i| (i * 37) as u8).collect();
                    Effect {
                        amount,
                        ..Effect::new(kind)
                    }
                    .apply(&mut p, w, h, 0);
                }
            }
            Effect::new(kind).apply(&mut [], 0, 0, 0);
        }
    }

    #[test]
    fn each_is_the_same_every_time() {
        let pic = noisy();
        for kind in ALL {
            assert_eq!(run(kind, 70.0, &pic), run(kind, 70.0, &pic), "{kind:?}");
        }
    }

    #[test]
    fn a_flat_picture_stays_flat_and_transparent_stays_transparent() {
        let still = flat([90, 120, 200]);
        let clear = vec![0u8; W * H * 4];
        for kind in ALL {
            let out = run(kind, 80.0, &still);
            let off = out
                .iter()
                .zip(&still)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(off <= 2, "{kind:?} changed a flat picture by {off}");
            assert!(
                run(kind, 80.0, &clear).chunks_exact(4).all(|p| p[3] == 0),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn the_blurs_smooth_a_noisy_picture_more_as_the_strength_grows() {
        let pic = noisy();
        let base = roughness(&pic, 0..H);
        for kind in [
            EffectKind::BoxBlur,
            EffectKind::LensBlur,
            EffectKind::NoiseReduction,
        ] {
            let (low, high) = (
                roughness(&run(kind, 20.0, &pic), 0..H),
                roughness(&run(kind, 100.0, &pic), 0..H),
            );
            assert!(low < base && high < low, "{kind:?}: {base} {low} {high}");
        }
    }

    #[test]
    fn a_box_blur_softens_an_edge_and_leaves_the_far_side_alone() {
        let out = run(EffectKind::BoxBlur, 100.0, &halves());
        let mid = px(&out, W / 2, 50)[0];
        assert!(mid > 20 && mid < 235, "the edge is soft: {mid}");
        assert_eq!(px(&out, 5, 50)[0], 0);
        assert_eq!(px(&out, W - 5, 50)[0], 255);
    }

    #[test]
    fn spin_and_zoom_blur_leave_the_middle_and_smear_the_outside() {
        // a picture with detail that is not white noise: a pixel of it differs from its neighbour
        // by a little, so a small move changes it a little and a large one a lot
        let pic: Vec<u8> = (0..W * H)
            .flat_map(|i| {
                let (x, y) = ((i % W) as f32, (i / W) as f32);
                let v = (128.0 + 100.0 * (x / 5.0).sin() * (y / 5.0).cos()) as u8;
                [v, v, v, 255]
            })
            .collect();
        for kind in [EffectKind::RadialBlur, EffectKind::ZoomBlur] {
            let out = run(kind, 100.0, &pic);
            // how far the picture moved from where it was, in a small square
            let moved = |cx: usize, cy: usize| {
                let mut sum = 0u32;
                for y in cy - 3..cy + 3 {
                    for x in cx - 3..cx + 3 {
                        sum += u32::from(px(&out, x, y)[0].abs_diff(px(&pic, x, y)[0]));
                    }
                }
                sum
            };
            assert!(
                moved(W / 2, H / 2) * 2 < moved(W - 8, 8),
                "{kind:?}: the middle should move least"
            );
            assert!(
                roughness(&out, 0..30) < roughness(&pic, 0..30),
                "{kind:?} corner not smoothed"
            );
        }
    }

    #[test]
    fn tilt_shift_keeps_the_middle_band_sharp_and_blurs_away_from_it() {
        let pic = noisy();
        let out = run(EffectKind::TiltShift, 100.0, &pic);
        assert_eq!(
            roughness(&out, 85..95),
            roughness(&pic, 85..95),
            "the band stays as it was"
        );
        assert!(
            roughness(&out, 0..10) < 0.5 * roughness(&pic, 0..10),
            "the top is blurred"
        );
        assert!(
            roughness(&out, 0..10) < roughness(&out, 62..70),
            "more blur farther out"
        );
    }

    #[test]
    fn lens_blur_spreads_a_bright_dot_into_a_disc_that_keeps_its_light() {
        let mut pic = flat([0, 0, 0]);
        let i = ((H / 2) * W + W / 2) * 4;
        pic[i..i + 3].copy_from_slice(&[255, 255, 255]);
        let out = run(EffectKind::LensBlur, 100.0, &pic);
        assert!(px(&out, W / 2, H / 2)[0] < 255, "the dot spread");
        let lit = out.chunks_exact(4).filter(|p| p[0] > 0).count();
        assert!(lit > 20, "{lit} pixels lit");
        let total: u32 = out.chunks_exact(4).map(|p| u32::from(p[0])).sum();
        assert!((total as f32 - 255.0).abs() < 90.0, "light kept: {total}");
    }

    #[test]
    fn noise_reduction_keeps_a_hard_edge() {
        // an edge with noise on top: the noise goes, the edge stays
        let mut pic = halves();
        for (i, p) in pic.chunks_exact_mut(4).enumerate() {
            let n = (((i as u32).wrapping_mul(2654435761) >> 24) % 9) as i32 - 4;
            for v in &mut p[..3] {
                *v = (i32::from(*v) + n).clamp(0, 255) as u8;
            }
        }
        let out = run(EffectKind::NoiseReduction, 60.0, &pic);
        assert!(px(&out, W / 2 - 2, 50)[0] < 30, "dark side stays dark");
        assert!(px(&out, W / 2 + 2, 50)[0] > 225, "light side stays light");
        assert!(
            roughness(&out, 20..40) < roughness(&pic, 20..40),
            "noise reduced"
        );
    }

    #[test]
    fn unsharp_mask_overshoots_at_an_edge() {
        let mut pic = flat([100, 100, 100]);
        for y in 0..H {
            for x in W / 2..W {
                pic[(y * W + x) * 4..(y * W + x) * 4 + 3].copy_from_slice(&[150, 150, 150]);
            }
        }
        let out = run(EffectKind::UnsharpMask, 100.0, &pic);
        assert!(px(&out, W / 2 - 1, 50)[0] < 100, "darker next to the edge");
        assert!(px(&out, W / 2, 50)[0] > 150, "lighter next to the edge");
        assert_eq!(px(&out, 5, 50)[0], 100);
    }

    /// Not a check, a measurement: `cargo test -p picasso-color --release blur::tests::timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_of_each_at_1080p() {
        let (w, h) = (1920, 1080);
        let pic: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        for kind in ALL {
            let mut p = pic.clone();
            let start = std::time::Instant::now();
            Effect::new(kind).apply(&mut p, w, h, 0);
            eprintln!(
                "{:>16}: {:>4} ms",
                kind.label(),
                start.elapsed().as_millis()
            );
        }
    }
}
