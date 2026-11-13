//! Moving, scaling, turning and cropping a layer, and mixing layers with a blend mode.

use picasso_core::{BlendMode, Transform};

/// Draws `layer` (`w * h` RGBA) into `out` (the same size) with `t` applied. Where the moved
/// picture does not reach, `out` is transparent.
///
/// When `stab` is `Some`, it is a stabilization matrix (frame 0 -> current frame, normalized
/// picture coordinates). Each output pixel is mapped through `t` into the current frame and then
/// back through `stab`'s inverse to frame 0, so the footage is shown with the camera movement
/// removed. Crop bounds are applied to the stabilized position, so auto-zoom can cut off the
/// edges the stabilization reveals.
pub(crate) fn apply(
    layer: &[u8],
    out: &mut [u8],
    (w, h): (usize, usize),
    t: &Transform,
    stab: Option<&[[f32; 3]; 3]>,
) {
    out.fill(0);
    if w == 0 || h == 0 || layer.len() < w * h * 4 || out.len() < w * h * 4 {
        return;
    }
    let (wf, hf) = (w as f32, h as f32);
    let angle = t.rotation.to_radians();
    let (sin, cos) = angle.sin_cos();
    let (left, right) = (t.crop_left * wf, (1.0 - t.crop_right) * wf);
    let (top, bottom) = (t.crop_top * hf, (1.0 - t.crop_bottom) * hf);
    let stab_inv = stab.and_then(homography_inverse);
    for y in 0..h {
        for x in 0..w {
            // The output pixel, relative to where the picture's middle now is, in pixels.
            let dx = (x as f32 + 0.5) - wf * (0.5 + t.x);
            let dy = (y as f32 + 0.5) - hf * (0.5 + t.y);
            // Undo the turn (clockwise on screen) and the scale to find the source pixel in the
            // current (unstabilized) frame.
            let rx = dx * cos + dy * sin;
            let ry = -dx * sin + dy * cos;
            let mut sx = rx / t.scale_x + wf * 0.5;
            let mut sy = ry / t.scale_y + hf * 0.5;
            // Remove the camera movement: map the current-frame point back to frame 0.
            if let Some(inv) = stab_inv {
                let nx = (sx - wf * 0.5) / wf;
                let ny = (sy - hf * 0.5) / hf;
                let (nx2, ny2) = homography_map(&inv, nx, ny);
                sx = (nx2 + 0.5) * wf;
                sy = (ny2 + 0.5) * hf;
            }
            if sx < left || sx >= right || sy < top || sy >= bottom {
                continue;
            }
            let dst = (y * w + x) * 4;
            sample(layer, (w, h), sx - 0.5, sy - 0.5, &mut out[dst..dst + 4]);
        }
    }
}

/// Inverts a 3x3 homography matrix, or `None` when it is singular.
fn homography_inverse(m: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let a = &m[0];
    let b = &m[1];
    let c = &m[2];
    let det = a[0] * (b[1] * c[2] - b[2] * c[1])
        - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0]);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let mut r = [[0.0; 3]; 3];
    r[0][0] = (b[1] * c[2] - b[2] * c[1]) * inv;
    r[0][1] = (a[2] * c[1] - a[1] * c[2]) * inv;
    r[0][2] = (a[1] * b[2] - a[2] * b[1]) * inv;
    r[1][0] = (b[2] * c[0] - b[0] * c[2]) * inv;
    r[1][1] = (a[0] * c[2] - a[2] * c[0]) * inv;
    r[1][2] = (a[2] * b[0] - a[0] * b[2]) * inv;
    r[2][0] = (b[0] * c[1] - b[1] * c[0]) * inv;
    r[2][1] = (a[1] * c[0] - a[0] * c[1]) * inv;
    r[2][2] = (a[0] * b[1] - a[1] * b[0]) * inv;
    Some(r)
}

/// Maps a normalized point through a 3x3 homography.
fn homography_map(m: &[[f32; 3]; 3], x: f32, y: f32) -> (f32, f32) {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    if w.abs() < 1e-6 {
        return (x * 1e12, y * 1e12);
    }
    let nx = (m[0][0] * x + m[0][1] * y + m[0][2]) / w;
    let ny = (m[1][0] * x + m[1][1] * y + m[1][2]) / w;
    (nx, ny)
}

/// Bilinear sample of `layer` at (x, y), in pixel-centre coordinates; the edges repeat.
fn sample(layer: &[u8], (w, h): (usize, usize), x: f32, y: f32, out: &mut [u8]) {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |px: usize, py: usize, c: usize| f32::from(layer[(py * w + px) * 4 + c]);
    for (c, o) in out.iter_mut().enumerate() {
        let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let bottom = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *o = (top * (1.0 - fy) + bottom * fy + 0.5) as u8;
    }
}

/// What a blend mode makes of the colour below (`d`) and above (`s`), both 0.0 to 1.0.
fn mix(mode: BlendMode, d: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Add => (d + s).min(1.0),
        BlendMode::Multiply => d * s,
        BlendMode::Screen => 1.0 - (1.0 - d) * (1.0 - s),
        BlendMode::Overlay => {
            if d < 0.5 {
                2.0 * d * s
            } else {
                1.0 - 2.0 * (1.0 - d) * (1.0 - s)
            }
        }
        BlendMode::Darken => d.min(s),
        BlendMode::Lighten => d.max(s),
        BlendMode::Difference => (d - s).abs(),
    }
}

/// Draws an effects clip's picture over `out`. `layer` is the picture in straight colour, with how
/// much of each pixel it covers in its alpha; `out` is the composite below it, kept over black,
/// and `coverage` (empty when no alpha was asked for) says how much of each pixel that covers. The
/// clip covers the frame at `opacity`, mixed as `mode` says, and what its transform leaves empty
/// stays empty rather than showing the picture again.
pub(crate) fn cover(
    out: &mut [u8],
    layer: &[u8],
    opacity: f32,
    mode: BlendMode,
    coverage: &mut [f32],
) {
    let tracked = !coverage.is_empty();
    for (i, (dst, src)) in out
        .chunks_exact_mut(4)
        .zip(layer.chunks_exact(4))
        .enumerate()
    {
        let below = if tracked { coverage[i] } else { 1.0 };
        let pic = f32::from(src[3]) / 255.0;
        for c in 0..3 {
            let d = if below > 0.0 {
                (f32::from(dst[c]) / 255.0 / below).min(1.0)
            } else {
                0.0
            };
            let s = f32::from(src[c]) / 255.0;
            let mixed = mix(mode, d, s);
            let v = d * below * (1.0 - opacity) + mixed * pic * opacity;
            dst[c] = (v * 255.0 + 0.5).min(255.0) as u8;
        }
        if tracked {
            coverage[i] = below * (1.0 - opacity) + pic * opacity;
        }
    }
}

/// Draws `layer` over `out` with its alpha times `opacity`, mixing colours as `mode` says.
pub(crate) fn blend(out: &mut [u8], layer: &[u8], opacity: f32, mode: BlendMode) {
    let opaque = mode == BlendMode::Normal && opacity >= 0.999;
    for (dst, src) in out.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
        if opaque && src[3] == 255 {
            dst[..3].copy_from_slice(&src[..3]);
            continue;
        }
        let a = f32::from(src[3]) / 255.0 * opacity;
        if a <= 0.0 {
            continue;
        }
        if mode == BlendMode::Normal && a >= 0.999 {
            dst[..3].copy_from_slice(&src[..3]);
            continue;
        }
        for c in 0..3 {
            let (d, s) = (f32::from(dst[c]) / 255.0, f32::from(src[c]) / 255.0);
            let mixed = mix(mode, d, s);
            dst[c] = ((d * (1.0 - a) + mixed * a) * 255.0 + 0.5) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 8 x 8 picture: left half red, right half blue.
    fn halves() -> Vec<u8> {
        let mut v = Vec::new();
        for _ in 0..8 {
            for x in 0..8 {
                v.extend_from_slice(if x < 4 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }
        v
    }

    fn px(buf: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * 8 + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    fn run(t: Transform) -> Vec<u8> {
        let mut out = vec![9; 8 * 8 * 4];
        apply(&halves(), &mut out, (8, 8), &t, None);
        out
    }

    #[test]
    fn the_identity_leaves_the_picture_alone() {
        assert_eq!(run(Transform::default()), halves());
    }

    #[test]
    fn scaling_shrinks_around_the_middle() {
        let out = run(Transform {
            scale_x: 0.5,
            scale_y: 0.5,
            ..Transform::default()
        });
        assert_eq!(
            px(&out, 0, 0),
            [0, 0, 0, 0],
            "the corner is outside the picture"
        );
        assert_eq!(px(&out, 7, 7)[3], 0);
        assert_eq!(
            px(&out, 3, 4)[..3],
            [255, 0, 0],
            "left of the middle is still red"
        );
        assert_eq!(
            px(&out, 4, 3)[..3],
            [0, 0, 255],
            "right of the middle is still blue"
        );
        // Twice as big shows only the middle part, stretched (the seam blends smoothly).
        let big = run(Transform {
            scale_x: 2.0,
            scale_y: 2.0,
            ..Transform::default()
        });
        assert_eq!(px(&big, 2, 0)[..3], [255, 0, 0]);
        assert_eq!(px(&big, 5, 0)[..3], [0, 0, 255]);
        assert!(
            big.chunks_exact(4).all(|p| p[3] == 255),
            "no gaps when zoomed in"
        );
    }

    #[test]
    fn moving_shifts_the_picture_and_leaves_a_gap() {
        // A quarter of the width to the right: two pixels.
        let out = run(Transform {
            x: 0.25,
            ..Transform::default()
        });
        assert_eq!(px(&out, 0, 4)[3], 0);
        assert_eq!(px(&out, 1, 4)[3], 0);
        assert_eq!(px(&out, 2, 4)[..3], [255, 0, 0]);
        assert_eq!(px(&out, 6, 4)[..3], [0, 0, 255]);
    }

    #[test]
    fn turning_by_180_degrees_swaps_the_halves_and_a_quarter_turn_moves_them() {
        let out = run(Transform {
            rotation: 180.0,
            ..Transform::default()
        });
        assert_eq!(px(&out, 0, 0)[..3], [0, 0, 255]);
        assert_eq!(px(&out, 7, 7)[..3], [255, 0, 0]);
        // Clockwise by 90 degrees: the left (red) half ends up on top.
        let quarter = run(Transform {
            rotation: 90.0,
            ..Transform::default()
        });
        assert_eq!(px(&quarter, 4, 1)[..3], [255, 0, 0]);
        assert_eq!(px(&quarter, 4, 6)[..3], [0, 0, 255]);
    }

    #[test]
    fn cropping_cuts_the_sides_off() {
        let out = run(Transform {
            crop_left: 0.25,
            crop_bottom: 0.5,
            ..Transform::default()
        });
        assert_eq!(px(&out, 1, 0)[3], 0, "cropped on the left");
        assert_eq!(px(&out, 2, 0)[..3], [255, 0, 0]);
        assert_eq!(px(&out, 5, 5)[3], 0, "cropped at the bottom");
    }

    #[test]
    fn blend_modes_mix_as_expected() {
        let over = |mode, below: [u8; 3], above: [u8; 3], opacity: f32| {
            let mut out = [below[0], below[1], below[2], 255];
            blend(
                &mut out,
                &[above[0], above[1], above[2], 255],
                opacity,
                mode,
            );
            [out[0], out[1], out[2]]
        };
        let (grey, other) = ([100, 100, 100], [200, 50, 250]);
        assert_eq!(over(BlendMode::Normal, grey, other, 1.0), other);
        assert_eq!(over(BlendMode::Normal, grey, other, 0.0), grey);
        assert_eq!(over(BlendMode::Add, grey, other, 1.0), [255, 150, 255]);
        assert_eq!(
            over(BlendMode::Multiply, [255, 255, 255], other, 1.0),
            other
        );
        assert_eq!(over(BlendMode::Multiply, [0, 0, 0], other, 1.0), [0, 0, 0]);
        assert_eq!(over(BlendMode::Screen, [0, 0, 0], other, 1.0), other);
        assert_eq!(
            over(BlendMode::Screen, [255, 255, 255], other, 1.0),
            [255, 255, 255]
        );
        assert_eq!(over(BlendMode::Darken, grey, other, 1.0), [100, 50, 100]);
        assert_eq!(over(BlendMode::Lighten, grey, other, 1.0), [200, 100, 250]);
        assert_eq!(
            over(BlendMode::Difference, grey, other, 1.0),
            [100, 50, 150]
        );
        // Overlay: dark colours below are multiplied, light ones screened.
        assert_eq!(over(BlendMode::Overlay, [0, 0, 0], other, 1.0), [0, 0, 0]);
        assert_eq!(
            over(BlendMode::Overlay, [255, 255, 255], other, 1.0),
            [255, 255, 255]
        );
        // Half opacity of add is half way between the below and the result.
        assert_eq!(over(BlendMode::Add, grey, grey, 0.5), [150, 150, 150]);
    }

    #[test]
    fn a_transparent_layer_changes_nothing() {
        let mut out = vec![7u8; 16];
        blend(&mut out, &[255, 0, 0, 0].repeat(4), 1.0, BlendMode::Add);
        assert_eq!(out, vec![7u8; 16]);
    }
}
