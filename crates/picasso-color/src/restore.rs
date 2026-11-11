//! Colour repair and finishing effects: Despill, Gamut Limiter, Contrast Pop, Chromatic
//! Aberration Fix, False Color, Dehaze, Deband, Dead Pixel Fixer and Beauty.
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100, that scales
//! its main setting from nothing (0) through the usual value (50) to twice as much (100); for
//! Despill, Gamut Limiter and False Color it is how much of the effect is mixed in. Sizes are in
//! thousandths of the picture's height. Deband follows the clip's frame number for its dither.

use crate::fx::{gaussian, EffectKind};
use crate::keys::{blur_plane, morph_plane};
use crate::look::{par_pixels, put, Src};

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

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn rgb_of(px: &[u8]) -> [f32; 3] {
    [0, 1, 2].map(|i| f32::from(px[i]) / 255.0)
}

fn straight(q: [f32; 4]) -> [f32; 3] {
    if q[3] <= 1e-6 {
        return [0.0; 3];
    }
    [q[0] / q[3], q[1] / q[3], q[2] / q[3]]
}

/// Writes a straight colour into a pixel, keeping its alpha.
fn set_rgb(px: &mut [u8], c: [f32; 3]) {
    for i in 0..3 {
        px[i] = (c[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
}

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn hash2(x: i32, y: i32, f: u32) -> f32 {
    (hash32(
        (x as u32).wrapping_mul(0x8da6_b343)
            ^ (y as u32).wrapping_mul(0xd816_3841)
            ^ f.wrapping_mul(0xcb1a_b31f),
    ) >> 8) as f32
        / 16_777_215.0
}

// ---- Despill ----

/// Removes a colour cast left by a screen: channel `ch` (0 red, 1 green, 2 blue) is limited to
/// the mean of the other two.
fn despill(rgba: &mut [u8], w: usize, h: usize, a: f32, ch: usize) {
    par_pixels(rgba, w, h, &|_, _, px| {
        let mut c = rgb_of(px);
        let limit = 0.5 * (c[(ch + 1) % 3] + c[(ch + 2) % 3]);
        c[ch] -= a * (c[ch] - limit).max(0.0);
        set_rgb(px, c);
    });
}

// ---- Gamut Limiter ----

type Mat3 = [f64; 9];

/// RGB to CIE XYZ (D65) of the gamuts, from their published primaries.
fn to_xyz(p3: bool) -> Mat3 {
    if p3 {
        [
            0.4865709, 0.2656677, 0.1982173, 0.2289746, 0.6917385, 0.0792869, 0.0, 0.0451134,
            1.0439444,
        ]
    } else {
        [
            0.4124564, 0.3575761, 0.1804375, 0.2126729, 0.7151522, 0.0721750, 0.0193339, 0.1191920,
            0.9503041,
        ]
    }
}

const REC2020: Mat3 = [
    0.6369580, 0.1446169, 0.1688810, 0.2627002, 0.6779981, 0.0593017, 0.0, 0.0280727, 1.0609851,
];

fn multiply(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut r = [0.0; 9];
    for i in 0..3 {
        for j in 0..3 {
            for n in 0..3 {
                r[i * 3 + j] += a[i * 3 + n] * b[n * 3 + j];
            }
        }
    }
    r
}

fn inverse(m: &Mat3) -> Mat3 {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let s = 1.0 / det;
    [
        (e * i - f * h) * s,
        (c * h - b * i) * s,
        (b * f - c * e) * s,
        (f * g - d * i) * s,
        (a * i - c * g) * s,
        (c * d - a * f) * s,
        (d * h - e * g) * s,
        (b * g - a * h) * s,
        (a * e - b * d) * s,
    ]
}

/// Pulls colours of a Rec. 2020 picture that fall outside the smaller gamut back inside it,
/// keeping their brightness (they are desaturated toward grey just far enough).
fn gamut_limiter(rgba: &mut [u8], w: usize, h: usize, a: f32, target_p3: bool) {
    let dst = to_xyz(target_p3);
    let to = multiply(&inverse(&dst), &REC2020);
    let from = inverse(&to);
    let luma_t = [dst[3], dst[4], dst[5]];
    let gamma = 2.4f64;
    let amount = f64::from(a);
    par_pixels(rgba, w, h, &|_, _, px| {
        let c = rgb_of(px).map(f64::from);
        let lin = c.map(|v| v.max(0.0).powf(gamma));
        let t =
            [0, 1, 2].map(|i| to[i * 3] * lin[0] + to[i * 3 + 1] * lin[1] + to[i * 3 + 2] * lin[2]);
        if t.iter().all(|v| (0.0..=1.0).contains(v)) {
            return;
        }
        let y = (luma_t[0] * t[0] + luma_t[1] * t[1] + luma_t[2] * t[2]).clamp(0.0, 1.0);
        let mut s = 1.0f64;
        for v in t {
            if v < 0.0 {
                s = s.min(y / (y - v));
            } else if v > 1.0 {
                s = s.min((1.0 - y) / (v - y));
            }
        }
        let r = t.map(|v| y + s * (v - y));
        let back = [0, 1, 2].map(|i| {
            (from[i * 3] * r[0] + from[i * 3 + 1] * r[1] + from[i * 3 + 2] * r[2])
                .clamp(0.0, 1.0)
                .powf(1.0 / gamma)
        });
        set_rgb(
            px,
            [0, 1, 2].map(|i| (c[i] + (back[i] - c[i]) * amount) as f32),
        );
    });
}

// ---- Contrast Pop ----

/// Local contrast: the difference between a pixel's brightness and a wide blur of its
/// surroundings is boosted, most in the midtones, with the colour scaled along.
fn contrast_pop(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let gain = a * 1.2;
    let mut blurred = rgba.to_vec();
    gaussian(
        &mut blurred,
        w,
        h,
        (40.0 * unit(h)).round().max(1.0) as usize,
    );
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let i = (y * w + x) * 4;
        let c = rgb_of(px);
        let yc = luma(c);
        let yb = if blurred[i + 3] > 0 {
            luma(rgb_of(&blurred[i..i + 4]))
        } else {
            yc
        };
        let wt = 1.0 - 0.5 * (2.0 * yc - 1.0).abs();
        let target = (yc + gain * wt * (yc - yb)).max(0.0);
        let o = if yc < 1e-4 {
            [target; 3]
        } else {
            c.map(|v| v * (target / yc))
        };
        set_rgb(px, o);
    });
}

// ---- Chromatic Aberration Fix ----

/// Scales the red and blue pictures about the middle (`red` and `blue` as fractions), then pulls
/// colour toward grey on edges (`defringe`, 0 to 1).
fn chromatic_fix(rgba: &mut [u8], w: usize, h: usize, red: f32, blue: f32, defringe: f32) {
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let half_diagonal = cx.hypot(cy);
    let tap = 1.5 + red.abs().max(blue.abs()) * half_diagonal;
    par_pixels(rgba, w, h, &|x, y, px| {
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let (dx, dy) = (xf - cx, yf - cy);
        let r = src.at(cx + dx * (1.0 + red), cy + dy * (1.0 + red));
        let g = src.at(xf, yf);
        let b = src.at(cx + dx * (1.0 + blue), cy + dy * (1.0 + blue));
        let mut o = [r[0], g[1], b[2]];
        let alpha = g[3].max(r[3]).max(b[3]);
        if defringe > 0.0 && alpha > 0.0 {
            let l = |ox: f32, oy: f32| {
                luma(src.at(xf + ox, yf + oy)[..3].try_into().unwrap_or([0.0; 3]))
            };
            let edge = smoothstep(
                0.04,
                0.25,
                (l(tap, 0.0) - l(-tap, 0.0)).hypot(l(0.0, tap) - l(0.0, -tap)),
            );
            let y = luma(o);
            o = o.map(|v| mix(v, y, defringe * edge));
        }
        put(px, [o[0], o[1], o[2], alpha]);
    });
}

// ---- False Color ----

struct Stop(f32, [f32; 3], bool);

#[derive(Clone, Copy)]
enum Palette {
    Exposure,
    Thermal,
    Rainbow,
}

fn stops(p: Palette) -> Vec<Stop> {
    let s = |pos: f32, rgb: [f32; 3]| Stop(pos, rgb, false);
    let grey = |pos: f32| Stop(pos, [0.0; 3], true);
    match p {
        Palette::Exposure => vec![
            s(0.00, [0.40, 0.00, 0.55]),
            s(0.02, [0.00, 0.15, 0.85]),
            s(0.08, [0.00, 0.50, 0.55]),
            grey(0.20),
            s(0.42, [0.15, 0.70, 0.20]),
            grey(0.50),
            s(0.62, [0.95, 0.60, 0.70]),
            grey(0.72),
            s(0.92, [0.95, 0.85, 0.10]),
            s(0.98, [0.95, 0.10, 0.10]),
        ],
        Palette::Thermal => vec![
            s(0.0, [0.0, 0.0, 0.0]),
            s(0.2, [0.1, 0.0, 0.4]),
            s(0.4, [0.6, 0.0, 0.5]),
            s(0.6, [0.95, 0.2, 0.1]),
            s(0.8, [1.0, 0.8, 0.1]),
            s(1.0, [1.0, 1.0, 0.9]),
        ],
        Palette::Rainbow => vec![
            s(0.0, [0.3, 0.0, 0.5]),
            s(0.2, [0.0, 0.2, 1.0]),
            s(0.4, [0.0, 0.9, 0.9]),
            s(0.6, [0.1, 0.9, 0.1]),
            s(0.8, [1.0, 0.9, 0.0]),
            s(1.0, [1.0, 0.1, 0.1]),
        ],
    }
}

/// Paints the picture by brightness: exposure bands (the rest keeps its grey), a thermal scale or
/// a rainbow.
fn false_color(rgba: &mut [u8], w: usize, h: usize, a: f32, palette: Palette) {
    let table = stops(palette);
    let banded = matches!(palette, Palette::Exposure);
    par_pixels(rgba, w, h, &|_, _, px| {
        if px[3] == 0 {
            return;
        }
        let c = rgb_of(px);
        let y = luma(c).clamp(0.0, 1.0);
        let o = if banded {
            let i = (1..table.len()).fold(0, |i, n| if y >= table[n].0 { n } else { i });
            if table[i].2 {
                [y; 3]
            } else {
                table[i].1
            }
        } else {
            let i = (1..table.len() - 1).fold(0, |i, n| if y >= table[n].0 { n } else { i });
            let t = ((y - table[i].0) / (table[i + 1].0 - table[i].0)).clamp(0.0, 1.0);
            [0, 1, 2].map(|j| mix(table[i].1[j], table[i + 1].1[j], t))
        };
        set_rgb(px, [0, 1, 2].map(|j| mix(c[j], o[j], a)));
    });
}

// ---- Dehaze ----

/// Takes haze out with the dark channel prior (He, Sun and Tang, 2009): in a clear area one
/// colour channel is dark somewhere, so the darkest value of a patch, after dividing by the haze
/// colour, measures the haze there; the picture is restored as `(I - A) / t + A`.
fn dehaze(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let strength = (0.95 * a * 1.2).clamp(0.0, 0.95);
    let haze = [
        0xd8 as f32 / 255.0,
        0xdc as f32 / 255.0,
        0xe2 as f32 / 255.0,
    ];
    let dark: Vec<f32> = rgba
        .chunks_exact(4)
        .map(|p| {
            let c = rgb_of(p);
            (0..3)
                .map(|i| c[i] / haze[i].max(0.02))
                .fold(f32::MAX, f32::min)
        })
        .collect();
    let patch = (15.0 * unit(h)).round() as usize;
    let dark = if patch >= 1 {
        morph_plane(&dark, w, h, patch, false)
    } else {
        dark
    };
    let soft = (8.0 * unit(h) * 0.5).round() as usize;
    let dark = if soft >= 1 {
        blur_plane(&dark, w, h, soft)
    } else {
        dark
    };
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let t = (1.0 - strength * dark[y * w + x]).max(0.1);
        let c = rgb_of(px);
        set_rgb(px, [0, 1, 2].map(|i| (c[i] - haze[i]) / t + haze[i]));
    });
}

// ---- Deband ----

/// Smooths the visible steps in skies and gradients and dithers them away: where a pixel is close
/// to a wide tent-weighted average the area is a gradient and the average is used; real edges
/// and texture are kept.
fn deband(rgba: &mut [u8], w: usize, h: usize, a: f32, frame: u64) {
    let k = a * 2.0;
    let threshold = (8.0 / 255.0 * k).max(1e-5);
    let radius = (16.0 * unit(h)).max(1.0);
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    let (wf, hf) = (w as f32, h as f32);
    let fr = frame as u32;
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let c = rgb_of(px);
        let ext = [radius.min(xf.min(wf - xf)), radius.min(yf.min(hf - yf))];
        let mut b = [0.0f32; 3];
        let mut wsum = 0.0;
        for j in -4i32..=4 {
            for i in -4i32..=4 {
                let wt = ((5 - i.abs()) * (5 - j.abs())) as f32;
                let s =
                    straight(src.at(xf + i as f32 * ext[0] * 0.26, yf + j as f32 * ext[1] * 0.26));
                for ch in 0..3 {
                    b[ch] += wt * s[ch];
                }
                wsum += wt;
            }
        }
        let b = b.map(|v| v / wsum);
        let diff = (0..3).map(|i| (c[i] - b[i]).abs()).fold(0.0f32, f32::max);
        let m = 1.0 - smoothstep(0.5 * threshold, threshold, diff);
        let n = [0u32, 1, 2]
            .map(|i| hash2(x as i32, y as i32, fr.wrapping_mul(3).wrapping_add(i)) - 0.5);
        set_rgb(
            px,
            [0, 1, 2].map(|i| mix(c[i], b[i], m) + n[i] * (2.0 / 255.0) * 0.4 * m),
        );
    });
}

// ---- Dead Pixel Fixer ----

/// Replaces pixels much brighter or darker than all eight around them by the mean of the eight.
fn dead_pixels(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let threshold = 0.25 / (a * 2.0).max(1e-3);
    let d = unit(h).max(1.0);
    let r = std::f32::consts::FRAC_1_SQRT_2 * d;
    let offsets = [
        (d, 0.0),
        (-d, 0.0),
        (0.0, d),
        (0.0, -d),
        (r, r),
        (-r, r),
        (r, -r),
        (-r, -r),
    ];
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let yc = luma(rgb_of(px));
        let (mut sum, mut mn, mut mx) = ([0.0f32; 3], f32::MAX, f32::MIN);
        for (ox, oy) in offsets {
            let s = straight(src.at(xf + ox, yf + oy));
            for i in 0..3 {
                sum[i] += s[i];
            }
            let l = luma(s);
            mn = mn.min(l);
            mx = mx.max(l);
        }
        if yc > mx + threshold || yc < mn - threshold {
            set_rgb(px, sum.map(|v| v / 8.0));
        }
    });
}

// ---- Beauty ----

/// Smooths blotchy skin and noise with a bilateral filter (neighbours count by distance and by
/// how close their colour is, so edges are not crossed) and gives back a share of the fine
/// texture. With `skin_only` only skin-coloured areas are smoothed.
fn beauty(rgba: &mut [u8], w: usize, h: usize, a: f32, skin_only: bool) {
    let smoothing = a.clamp(0.0, 1.0);
    let spacing = (6.0 * unit(h) / 3.0).max(0.5);
    let sigma = 0.05f32;
    let inv2s = 1.0 / (2.0 * sigma * sigma);
    let recover = 0.2;
    let original = rgba.to_vec();
    let src = Src::new(&original, w, h);
    par_pixels(rgba, w, h, &|x, y, px| {
        if px[3] == 0 {
            return;
        }
        let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
        let c = rgb_of(px);
        let (mut sum, mut wsum) = ([0.0f32; 3], 0.0);
        for j in -3i32..=3 {
            for i in -3i32..=3 {
                let q = src.at(xf + i as f32 * spacing, yf + j as f32 * spacing);
                if q[3] <= 0.0 {
                    continue;
                }
                let s = straight(q);
                let dc = (s[0] - c[0]).powi(2) + (s[1] - c[1]).powi(2) + (s[2] - c[2]).powi(2);
                let wt = (-((i * i + j * j) as f32) / 4.5).exp() * (-dc * inv2s).exp();
                for ch in 0..3 {
                    sum[ch] += s[ch] * wt;
                }
                wsum += wt;
            }
        }
        let smooth = sum.map(|v| v / wsum.max(1e-6));
        let soft = [0, 1, 2].map(|i| smooth[i] + recover * (c[i] - smooth[i]));
        let skin = if skin_only {
            let cb = -0.168_736 * c[0] - 0.331_264 * c[1] + 0.5 * c[2] + 0.5;
            let cr = 0.5 * c[0] - 0.418_688 * c[1] - 0.081_312 * c[2] + 0.5;
            let m = smoothstep(0.28, 0.32, cb) * (1.0 - smoothstep(0.48, 0.52, cb));
            m * smoothstep(0.50, 0.54, cr)
                * (1.0 - smoothstep(0.66, 0.70, cr))
                * smoothstep(0.08, 0.2, luma(c))
        } else {
            1.0
        };
        let k = smoothing * skin;
        set_rgb(px, [0, 1, 2].map(|i| mix(c[i], soft[i], k)));
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize, frame: u64) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    match kind {
        EffectKind::DespillGreen => despill(rgba, w, h, a, 1),
        EffectKind::DespillBlue => despill(rgba, w, h, a, 2),
        EffectKind::DespillRed => despill(rgba, w, h, a, 0),
        EffectKind::GamutLimiterRec709 => gamut_limiter(rgba, w, h, a, false),
        EffectKind::GamutLimiterP3 => gamut_limiter(rgba, w, h, a, true),
        EffectKind::ContrastPop => contrast_pop(rgba, w, h, a),
        EffectKind::FringeFixRed => chromatic_fix(rgba, w, h, -a * 0.02, 0.0, 0.0),
        EffectKind::FringeFixBlue => chromatic_fix(rgba, w, h, 0.0, -a * 0.02, 0.0),
        EffectKind::Defringe => chromatic_fix(rgba, w, h, 0.0, 0.0, a),
        EffectKind::FalseColorExposure => false_color(rgba, w, h, a, Palette::Exposure),
        EffectKind::FalseColorThermal => false_color(rgba, w, h, a, Palette::Thermal),
        EffectKind::FalseColorRainbow => false_color(rgba, w, h, a, Palette::Rainbow),
        EffectKind::Dehaze => dehaze(rgba, w, h, a),
        EffectKind::Deband => deband(rgba, w, h, a, frame),
        EffectKind::DeadPixelFixer => dead_pixels(rgba, w, h, a),
        EffectKind::Beauty => beauty(rgba, w, h, a, false),
        EffectKind::BeautySkin => beauty(rgba, w, h, a, true),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 160;
    const H: usize = 90;

    const KINDS: [EffectKind; 17] = [
        EffectKind::DespillGreen,
        EffectKind::DespillBlue,
        EffectKind::DespillRed,
        EffectKind::GamutLimiterRec709,
        EffectKind::GamutLimiterP3,
        EffectKind::ContrastPop,
        EffectKind::FringeFixRed,
        EffectKind::FringeFixBlue,
        EffectKind::Defringe,
        EffectKind::FalseColorExposure,
        EffectKind::FalseColorThermal,
        EffectKind::FalseColorRainbow,
        EffectKind::Dehaze,
        EffectKind::Deband,
        EffectKind::DeadPixelFixer,
        EffectKind::Beauty,
        EffectKind::BeautySkin,
    ];

    fn solid(rgb: [u8; 3]) -> Vec<u8> {
        (0..W * H)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    /// Every pixel different, saturated in places, with edges.
    fn busy() -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let n = (((x * 73) ^ (y * 151)) % 60) as u8;
                let block = if (x / 20 + y / 15) % 2 == 0 { 170 } else { 40 };
                v.extend_from_slice(&[block, 90 + n, (x * 255 / W) as u8, 255]);
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
        .apply(&mut out, W, H, 4);
        out
    }

    fn px(p: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * W + x) * 4;
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    }

    #[test]
    fn nothing_changes_at_zero_and_each_changes_a_busy_picture() {
        let pic = busy();
        for kind in KINDS {
            assert_eq!(run(kind, 0.0, &pic), pic, "{kind:?} at zero");
        }
        for kind in KINDS {
            // Gamut Limiter only touches colours outside the gamut: give it a Rec. 2020 green.
            let subject = if matches!(
                kind,
                EffectKind::GamutLimiterRec709 | EffectKind::GamutLimiterP3
            ) {
                solid([0, 255, 0])
            } else if kind == EffectKind::DeadPixelFixer {
                let mut p = solid([100, 100, 100]);
                p[(45 * W + 80) * 4..(45 * W + 80) * 4 + 3].fill(255);
                p
            } else if kind == EffectKind::BeautySkin {
                // Needs skin-coloured, blotchy pixels.
                let mut p = solid([200, 150, 120]);
                for (i, q) in p.chunks_exact_mut(4).enumerate() {
                    q[0] = q[0].wrapping_add(((i * 7) % 9) as u8);
                }
                p
            } else {
                pic.clone()
            };
            let out = run(kind, kind.default_amount(), &subject);
            assert_ne!(out, subject, "{kind:?} changed nothing");
            assert_eq!(out.len(), subject.len());
        }
    }

    #[test]
    fn despill_pulls_the_cast_channel_down_to_the_mean_of_the_others() {
        let g = run(EffectKind::DespillGreen, 100.0, &solid([100, 200, 100]));
        assert_eq!(px(&g, 5, 5)[1], 100);
        let b = run(EffectKind::DespillBlue, 100.0, &solid([100, 100, 220]));
        assert_eq!(px(&b, 5, 5)[2], 100);
        let r = run(EffectKind::DespillRed, 100.0, &solid([220, 100, 100]));
        assert_eq!(px(&r, 5, 5)[0], 100);
        // A colour without the cast stays.
        let same = run(EffectKind::DespillGreen, 100.0, &solid([100, 90, 100]));
        assert_eq!(px(&same, 5, 5)[..3], [100, 90, 100]);
    }

    #[test]
    fn the_gamut_limiter_pulls_in_wide_colours_and_leaves_greys_and_alpha() {
        for kind in [EffectKind::GamutLimiterRec709, EffectKind::GamutLimiterP3] {
            let grey = run(kind, 100.0, &solid([128, 128, 128]));
            assert_eq!(px(&grey, 3, 3)[..3], [128, 128, 128], "{kind:?} grey");
            let green = run(kind, 100.0, &solid([0, 255, 0]));
            let p = px(&green, 3, 3);
            assert!(
                p[0] > 0 || p[2] > 0,
                "{kind:?} desaturates a Rec. 2020 green: {p:?}"
            );
            assert_eq!(p[3], 255);
        }
    }

    #[test]
    fn contrast_pop_adds_punch_to_a_spot_and_leaves_flat_areas() {
        // Its blur is 40 lines at 1080: use a picture big enough for that to be a real blur.
        let (w, h) = (960usize, 540usize);
        let flat: Vec<u8> = (0..w * h).flat_map(|_| [120u8, 120, 120, 255]).collect();
        let mut out = flat.clone();
        Effect {
            amount: 100.0,
            ..Effect::new(EffectKind::ContrastPop)
        }
        .apply(&mut out, w, h, 0);
        assert_eq!(out, flat);
        let mut spot = flat.clone();
        for y in 220..320 {
            for x in 430..530 {
                spot[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(160);
            }
        }
        let mut out = spot.clone();
        Effect {
            amount: 100.0,
            ..Effect::new(EffectKind::ContrastPop)
        }
        .apply(&mut out, w, h, 0);
        assert!(
            out[(270 * w + 432) * 4] > 160,
            "the inside of the spot edge is brighter"
        );
        assert!(out[(270 * w + 427) * 4] < 120, "and the outside darker");
    }

    #[test]
    fn fringe_fix_moves_only_its_channel_and_defringe_greys_edges() {
        // A white square on black, off the middle, so the scaling shifts its edge.
        let mut sq = solid([0, 0, 0]);
        for y in 10..30 {
            for x in 10..40 {
                sq[(y * W + x) * 4..(y * W + x) * 4 + 3].fill(255);
            }
        }
        let red = run(EffectKind::FringeFixRed, 100.0, &sq);
        let mut green_same = true;
        let mut red_differs = false;
        for i in 0..W * H {
            green_same &= red[i * 4 + 1] == sq[i * 4 + 1];
            red_differs |= red[i * 4] != sq[i * 4];
        }
        assert!(green_same && red_differs);
        let blue = run(EffectKind::FringeFixBlue, 100.0, &sq);
        assert!(blue.iter().zip(&sq).any(|(a, b)| a != b));
        // A red-to-blue edge loses colour.
        let mut edge = solid([220, 30, 30]);
        for y in 0..H {
            for x in W / 2..W {
                edge[(y * W + x) * 4..(y * W + x) * 4 + 3].copy_from_slice(&[30, 30, 220]);
            }
        }
        let out = run(EffectKind::Defringe, 100.0, &edge);
        let sat =
            |p: [u8; 4]| i32::from(p[0].max(p[1]).max(p[2])) - i32::from(p[0].min(p[1]).min(p[2]));
        assert!(sat(px(&out, W / 2 - 1, 40)) < sat(px(&edge, W / 2 - 1, 40)));
    }

    #[test]
    fn false_colour_paints_by_brightness() {
        let mid = run(
            EffectKind::FalseColorExposure,
            100.0,
            &solid([128, 128, 128]),
        );
        // 0.5 is a grey band: it stays grey.
        assert_eq!(px(&mid, 2, 2)[0], px(&mid, 2, 2)[1]);
        let green = run(
            EffectKind::FalseColorExposure,
            100.0,
            &solid([110, 110, 110]),
        );
        let p = px(&green, 2, 2);
        assert!(p[1] > p[0] && p[1] > p[2], "the 0.42 band is green: {p:?}");
        let dark = run(EffectKind::FalseColorThermal, 100.0, &solid([0, 0, 0]));
        assert_eq!(px(&dark, 2, 2)[..3], [0, 0, 0]);
        let hot = run(
            EffectKind::FalseColorThermal,
            100.0,
            &solid([255, 255, 255]),
        );
        assert!(px(&hot, 2, 2)[0] > 200 && px(&hot, 2, 2)[1] > 200);
        let rb = run(
            EffectKind::FalseColorRainbow,
            100.0,
            &solid([255, 255, 255]),
        );
        let p = px(&rb, 2, 2);
        assert!(
            p[0] > 200 && p[1] < 80,
            "the top of the rainbow is red: {p:?}"
        );
    }

    #[test]
    fn dehaze_darkens_a_hazy_picture_and_deband_adds_levels() {
        let out = run(EffectKind::Dehaze, 100.0, &solid([190, 195, 200]));
        assert!(px(&out, 5, 5)[0] < 190);
        // A banded ramp: steps 20 pixels wide and one level high.
        let mut ramp = Vec::new();
        for _ in 0..H {
            for x in 0..W {
                let g = 100 + (x / 20) as u8;
                ramp.extend_from_slice(&[g, g, g, 255]);
            }
        }
        let out = run(EffectKind::Deband, 100.0, &ramp);
        let levels = |p: &[u8]| {
            (0..W)
                .map(|x| p[(45 * W + x) * 4])
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        };
        assert!(levels(&out) >= levels(&ramp));
        // And it is not turned into noise: nothing moves by more than a couple of levels.
        assert!(out.iter().zip(&ramp).all(|(a, b)| a.abs_diff(*b) <= 3));
    }

    #[test]
    fn dead_pixels_are_filled_in_and_beauty_calms_blotches_but_keeps_edges() {
        let mut p = solid([100, 100, 100]);
        p[(45 * W + 80) * 4..(45 * W + 80) * 4 + 3].fill(255);
        let out = run(EffectKind::DeadPixelFixer, 50.0, &p);
        let fixed = px(&out, 80, 45)[0];
        assert!(fixed < 125, "the bright pixel is mostly gone: {fixed}");
        let mut blotchy = solid([200, 150, 120]);
        for (i, q) in blotchy.chunks_exact_mut(4).enumerate() {
            q[0] = q[0].wrapping_add(((i * 7) % 9) as u8);
        }
        let spread = |p: &[u8]| {
            let (lo, hi) = p
                .chunks_exact(4)
                .fold((255u8, 0u8), |(lo, hi), q| (lo.min(q[0]), hi.max(q[0])));
            hi - lo
        };
        for kind in [EffectKind::Beauty, EffectKind::BeautySkin] {
            assert!(
                spread(&run(kind, 100.0, &blotchy)) < spread(&blotchy),
                "{kind:?}"
            );
        }
        // A hard edge survives.
        let mut edge = solid([30, 30, 30]);
        for y in 0..H {
            for x in W / 2..W {
                edge[(y * W + x) * 4..(y * W + x) * 4 + 3].fill(220);
            }
        }
        let out = run(EffectKind::Beauty, 100.0, &edge);
        assert!(px(&out, W / 2 + 2, 40)[0] > 180 && px(&out, W / 2 - 3, 40)[0] < 70);
    }

    #[test]
    fn they_keep_clear_pixels_clear_and_work_on_tiny_pictures() {
        let mut clear = busy();
        clear.chunks_exact_mut(4).take(W).for_each(|p| p[3] = 0);
        for kind in KINDS {
            let out = run(kind, 100.0, &clear);
            // Scaling the red or blue picture can bring a little coverage across the edge.
            if !matches!(kind, EffectKind::FringeFixRed | EffectKind::FringeFixBlue) {
                assert!(out.chunks_exact(4).take(W).all(|p| p[3] == 0), "{kind:?}");
            }
            for (w, h) in [(1usize, 1usize), (2, 3), (5, 1)] {
                let mut p = vec![140u8; w * h * 4];
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
