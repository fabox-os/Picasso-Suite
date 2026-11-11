//! The built-in generators: pictures drawn from a formula, pixel by pixel, at any size.

use picasso_core::{GeneratorKind, TextStyle, TitleText};

/// The 32-bit integer hash behind the noise.
fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

fn unit(h: u32) -> f32 {
    (h & 0xffff) as f32 / 65535.0
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn grey(v: f32) -> [u8; 4] {
    let b = byte(v);
    [b, b, b, 255]
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let c = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t + 0.5) as u8;
    [c(0), c(1), c(2), 255]
}

/// Straight RGBA of `a` over `b` laid on a transparent picture: `cover` of `b` at full alpha
/// over `a` at full alpha, i.e. both opaque colours blended.
fn rgb(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

// ---- Colour bars ----

const FULL_BARS: [[u8; 3]; 8] = [
    [255, 255, 255],
    [255, 255, 0],
    [0, 255, 255],
    [0, 255, 0],
    [255, 0, 255],
    [255, 0, 0],
    [0, 0, 255],
    [0, 0, 0],
];

const EBU_BARS: [[u8; 3]; 8] = [
    [255, 255, 255],
    [191, 191, 0],
    [0, 191, 191],
    [0, 191, 0],
    [191, 0, 191],
    [191, 0, 0],
    [0, 0, 191],
    [0, 0, 0],
];

fn bar_of(table: &[[u8; 3]; 8], u: f32) -> [u8; 4] {
    rgb(table[((u * 8.0) as usize).min(7)])
}

// ---- ITU-R BT.2111 ----

type Code = [i32; 3];
const fn gr(v: i32) -> Code {
    [v, v, v]
}

/// The 39 patches, 10-bit codes as printed in the Recommendation (Tables 2, 3 and 4):
/// 0 side grey, 1..7 top strip, 8..14 main bars, 15 unused, 16..28 the stair, 29..32 the black
/// row, 33..38 the BT.709 bars at the ends of the black row.
const HLG_NARROW: [Code; 39] = [
    gr(414),
    gr(940),
    [940, 940, 64],
    [64, 940, 940],
    [64, 940, 64],
    [940, 64, 940],
    [940, 64, 64],
    [64, 64, 940],
    gr(721),
    [721, 721, 64],
    [64, 721, 721],
    [64, 721, 64],
    [721, 64, 721],
    [721, 64, 64],
    [64, 64, 721],
    gr(0),
    gr(4),
    gr(64),
    gr(152),
    gr(239),
    gr(327),
    gr(414),
    gr(502),
    gr(590),
    gr(677),
    gr(765),
    gr(852),
    gr(940),
    gr(1019),
    gr(64),
    gr(48),
    gr(80),
    gr(99),
    [713, 719, 316],
    [538, 709, 718],
    [512, 706, 296],
    [651, 286, 705],
    [639, 269, 164],
    [227, 147, 702],
];

const PQ_NARROW: [Code; 39] = [
    gr(414),
    gr(940),
    [940, 940, 64],
    [64, 940, 940],
    [64, 940, 64],
    [940, 64, 940],
    [940, 64, 64],
    [64, 64, 940],
    gr(573),
    [573, 573, 64],
    [64, 573, 573],
    [64, 573, 64],
    [573, 64, 573],
    [573, 64, 64],
    [64, 64, 573],
    gr(0),
    gr(4),
    gr(64),
    gr(152),
    gr(239),
    gr(327),
    gr(414),
    gr(502),
    gr(590),
    gr(677),
    gr(765),
    gr(852),
    gr(940),
    gr(1019),
    gr(64),
    gr(48),
    gr(80),
    gr(99),
    [569, 572, 381],
    [485, 566, 571],
    [474, 565, 368],
    [537, 362, 564],
    [531, 351, 257],
    [318, 236, 563],
];

const PQ_FULL: [Code; 39] = [
    gr(409),
    gr(1023),
    [1023, 1023, 0],
    [0, 1023, 1023],
    [0, 1023, 0],
    [1023, 0, 1023],
    [1023, 0, 0],
    [0, 0, 1023],
    gr(594),
    [594, 594, 0],
    [0, 594, 594],
    [0, 594, 0],
    [594, 0, 594],
    [594, 0, 0],
    [0, 0, 594],
    gr(0),
    gr(0),
    gr(0),
    gr(102),
    gr(205),
    gr(307),
    gr(409),
    gr(512),
    gr(614),
    gr(716),
    gr(818),
    gr(921),
    gr(1023),
    gr(1023),
    gr(0),
    gr(0),
    gr(19),
    gr(41),
    [589, 593, 370],
    [491, 586, 592],
    [479, 585, 355],
    [552, 348, 584],
    [545, 335, 225],
    [296, 201, 582],
];

const BAR_EDGES: [i32; 6] = [446, 652, 858, 1062, 1268, 1474];
const STAIR_EDGES: [i32; 11] = [549, 652, 755, 858, 960, 1062, 1165, 1268, 1371, 1474, 1577];

/// The patch at pixel (`x`, `y`) of the 1920 x 1080 reference picture, or -1 inside the ramp.
fn bt2111_patch(x: i32, y: i32) -> i32 {
    let side = !(240..1680).contains(&x);
    if y < 630 {
        if side {
            return 0;
        }
        let bar = BAR_EDGES.iter().filter(|e| x >= **e).count() as i32;
        return (if y < 90 { 1 } else { 8 }) + bar;
    }
    if y < 720 {
        if side {
            return 8;
        }
        if x < BAR_EDGES[0] {
            return 16;
        }
        return 17 + STAIR_EDGES.iter().filter(|e| x >= **e).count() as i32;
    }
    if y < 810 {
        return if x < 240 { 29 } else { -1 };
    }
    // The black row: BT.709 bars, then black with -2 / +2 / +4 % steps, 75 % / 58 % white and
    // black, then the BT.709 bars again.
    const ROW: [(i32, i32); 17] = [
        (80, 33),
        (160, 34),
        (240, 35),
        (376, 29),
        (446, 30),
        (514, 29),
        (584, 31),
        (652, 29),
        (722, 32),
        (960, 29),
        (1398, 8),
        (1680, 29),
        (1760, 36),
        (1840, 37),
        (1920, 38),
        (1920, 38),
        (1920, 38),
    ];
    ROW.iter().find(|(end, _)| x < *end).map_or(38, |(_, p)| *p)
}

fn bt2111_ramp(variant: GeneratorKind, x: i32) -> i32 {
    let x = x - 240;
    if variant == GeneratorKind::Bt2111PqFull {
        return if x < 618 {
            0
        } else if x < 618 + 1022 {
            1 + (x - 618)
        } else {
            1023
        };
    }
    if x < 559 {
        4
    } else if x < 559 + 1014 {
        5 + (x - 559)
    } else {
        1019
    }
}

fn bt2111(variant: GeneratorKind, u: f32, v: f32) -> [u8; 4] {
    let x = ((u * 1920.0) as i32).clamp(0, 1919);
    let y = ((v * 1080.0) as i32).clamp(0, 1079);
    let table = match variant {
        GeneratorKind::Bt2111HlgNarrow => &HLG_NARROW,
        GeneratorKind::Bt2111PqNarrow => &PQ_NARROW,
        _ => &PQ_FULL,
    };
    let level = |code: i32| {
        if variant == GeneratorKind::Bt2111PqFull {
            code as f32 / 1023.0
        } else {
            (code - 64) as f32 / 876.0
        }
    };
    let patch = bt2111_patch(x, y);
    if patch < 0 {
        return grey(level(bt2111_ramp(variant, x)));
    }
    let c = table[patch as usize];
    [byte(level(c[0])), byte(level(c[1])), byte(level(c[2])), 255]
}

// ---- YCbCr ramp ----

fn ycbcr(y: f32, cb: f32, cr: f32) -> [u8; 4] {
    [
        byte(y + 1.5748 * cr),
        byte(y - 0.187_324 * cb - 0.468_124 * cr),
        byte(y + 1.8556 * cb),
        255,
    ]
}

fn ycbcr_ramp(band: usize, t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    match band {
        0 => ycbcr(t, 0.0, 0.0),
        1 => ycbcr(0.5, t - 0.5, 0.0),
        _ => ycbcr(0.5, 0.0, t - 0.5),
    }
}

// ---- Noise gradient ----

fn lattice(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    unit(hash(
        x as u32,
        y as u32,
        seed ^ (z as u32).wrapping_mul(0x9e37_79b9),
    ))
}

fn value_noise(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty, tz) = (smooth(x - fx), smooth(y - fy), smooth(z - fz));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let plane = |dz: i32| {
        let a = lattice(ix, iy, iz + dz, seed);
        let b = lattice(ix + 1, iy, iz + dz, seed);
        let c = lattice(ix, iy + 1, iz + dz, seed);
        let d = lattice(ix + 1, iy + 1, iz + dz, seed);
        lerp(lerp(a, b, tx), lerp(c, d, tx), ty)
    };
    lerp(plane(0), plane(1), tz)
}

fn fractal(x: f32, y: f32, scale: f32, octaves: u32, evolution: f32, seed: u32) -> f32 {
    let (mut sum, mut norm, mut amp, mut freq) = (0.0, 0.0, 1.0, 1.0);
    for o in 0..octaves {
        let off = 37.0 * o as f32;
        sum += amp
            * value_noise(
                x / scale * freq + off,
                y / scale * freq + off,
                evolution * freq + off,
                seed,
            );
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

// ---- Drawing ----

/// What a generator needs to know about the moment it draws.
#[derive(Clone, Copy)]
pub(crate) struct Moment {
    /// Time into the clip, in microseconds.
    pub time_us: i64,
    /// Frames in a second, as a whole number.
    pub fps: u64,
}

/// The frame at `time_us` microseconds for a whole number of frames a second. Rounded, not cut:
/// the time of frame 13 at 24 frames a second is 541 666 microseconds, a hair under 13 frames.
fn frame_of(time_us: i64, fps: u64) -> u64 {
    ((time_us.max(0) as u128 * u128::from(fps) + 500_000) / 1_000_000) as u64
}

/// "HH:MM:SS:FF" of `frames` at `fps` frames a second.
fn timecode(frames: u64, fps: u64) -> String {
    let fps = fps.max(1);
    let seconds = frames / fps;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        frames % fps
    )
}

/// Fills `out` (`w * h * 4` bytes, straight alpha) with the picture of a generator.
pub(crate) fn draw(
    pattern: GeneratorKind,
    colors: &[[u8; 3]; 4],
    (w, h): (usize, usize),
    at: Moment,
    out: &mut [u8],
) {
    if w == 0 || h == 0 || out.len() < w * h * 4 {
        return;
    }
    let frame = frame_of(at.time_us, at.fps);
    let seconds = at.time_us.max(0) as f32 / 1_000_000.0;
    let (wf, hf) = (w as f32, h as f32);
    let unit_px = hf / 1080.0;
    let pixel = |x: usize, y: usize| -> [u8; 4] {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let (u, v) = (px / wf, py / hf);
        let (cx, cy) = (px - wf / 2.0, py - hf / 2.0);
        match pattern {
            GeneratorKind::FourColorGradient => {
                let top = mix(colors[0], colors[1], u);
                let bottom = mix(colors[2], colors[3], u);
                mix(
                    [top[0], top[1], top[2]],
                    [bottom[0], bottom[1], bottom[2]],
                    v,
                )
            }
            GeneratorKind::VerticalGradient => mix(colors[0], colors[1], v),
            GeneratorKind::DiagonalGradient => {
                let t = (cx + cy) / (wf + hf) + 0.5;
                mix(colors[0], colors[1], t)
            }
            GeneratorKind::RadialGradient => {
                let radius = (wf * wf + hf * hf).sqrt() / 2.0;
                mix(colors[0], colors[1], cx.hypot(cy) / radius)
            }
            GeneratorKind::FullColorBars => bar_of(&FULL_BARS, u),
            GeneratorKind::EbuBars => bar_of(&EBU_BARS, u),
            GeneratorKind::GreyRamp => grey(u),
            GeneratorKind::GreyScale => grey(((u * 11.0).floor().min(10.0)) / 10.0),
            GeneratorKind::Steps100mV => grey(((u * 8.0).floor().min(7.0)) / 7.0),
            GeneratorKind::YcbcrRamp => ycbcr_ramp(((v * 3.0) as usize).min(2), u),
            GeneratorKind::Bt2111HlgNarrow
            | GeneratorKind::Bt2111PqNarrow
            | GeneratorKind::Bt2111PqFull => bt2111(pattern, u, v),
            GeneratorKind::Noise | GeneratorKind::ColorNoise => {
                let cell = (x as f32 / unit_px.max(1.0)).floor() as u32;
                let row = (y as f32 / unit_px.max(1.0)).floor() as u32;
                let channel = |salt: u32| {
                    0.5 + (unit(hash(
                        cell,
                        row,
                        (frame as u32).wrapping_mul(7919).wrapping_add(salt),
                    )) - 0.5)
                };
                if pattern == GeneratorKind::Noise {
                    grey(channel(0))
                } else {
                    [byte(channel(0)), byte(channel(1)), byte(channel(2)), 255]
                }
            }
            GeneratorKind::NoiseGradient => {
                let n = fractal(px / unit_px, py / unit_px, 400.0, 4, seconds * 0.5, 1);
                mix(colors[0], colors[1], (n - 0.5) * 2.0 + 0.5)
            }
            GeneratorKind::Grid => {
                let (cell_w, cell_h) = (wf / 16.0, hf / 9.0);
                let axis = |p: f32, cell: f32| {
                    let line = (p / cell + 0.5).floor();
                    let d = (p - line * cell).abs();
                    let major = (line as i64).rem_euclid(4) == 0;
                    let width = (if major { 4.0 } else { 2.0 }) * unit_px;
                    (width * 0.5 - d + 0.5).clamp(0.0, 1.0)
                };
                mix(colors[0], colors[1], axis(px, cell_w).max(axis(py, cell_h)))
            }
            GeneratorKind::WindowRectangle | GeneratorKind::WindowEllipse => {
                let (hw, hh) = (wf * 0.25, hf * 0.25);
                let d = if pattern == GeneratorKind::WindowEllipse {
                    ((cx / hw).hypot(cy / hh) - 1.0) * hw.min(hh)
                } else {
                    (cx.abs() - hw).max(cy.abs() - hh)
                };
                let soft = 8.0 * unit_px;
                let cover = (0.5 - d / soft).clamp(0.0, 1.0);
                let c = mix(colors[0], colors[1], cover);
                [c[0], c[1], c[2], 255]
            }
            GeneratorKind::Countdown => {
                let ring = 0.012 * hf;
                let outer = 0.36 * hf;
                let r = cx.hypot(cy);
                if (r - outer).abs() < ring
                    || (r - outer * 0.86).abs() < ring * 0.5
                    || cx.abs() < ring * 0.4
                    || cy.abs() < ring * 0.4
                {
                    return rgb(colors[2]);
                }
                if r < outer {
                    let mut turn = cx.atan2(-cy) / std::f32::consts::TAU;
                    if turn < 0.0 {
                        turn += 1.0;
                    }
                    if turn < seconds.fract() {
                        return rgb(colors[1]);
                    }
                }
                rgb(colors[0])
            }
            GeneratorKind::TimecodeBurnIn => [0, 0, 0, 0],
        }
    };
    let row_len = w * 4;
    picasso_color::par::par_rows(&mut out[..w * h * 4], row_len, h, &|first, band| {
        for (r, line) in band.chunks_exact_mut(row_len).enumerate() {
            for (x, px) in line.chunks_exact_mut(4).enumerate() {
                px.copy_from_slice(&pixel(x, first + r));
            }
        }
    });
    match pattern {
        GeneratorKind::Countdown => {
            let left = 10 - (at.time_us.max(0) / 1_000_000) % 10;
            let title = TitleText {
                text: left.to_string(),
                size: 420,
                color: colors[2],
                bold: true,
                ..TitleText::default()
            };
            crate::text::draw_title(out, w, h, &title);
        }
        GeneratorKind::TimecodeBurnIn => {
            let title = TitleText {
                text: timecode(frame, at.fps),
                size: 90,
                color: colors[1],
                y: 860,
                bold: true,
                style: TextStyle {
                    background: true,
                    background_color: colors[0],
                    background_opacity: 70,
                    ..TextStyle::default()
                },
                ..TitleText::default()
            };
            crate::text::draw_title(out, w, h, &title);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(pattern: GeneratorKind, size: (usize, usize), time_us: i64) -> Vec<u8> {
        let mut out = vec![9u8; size.0 * size.1 * 4];
        let colors = pattern.default_colors();
        draw(
            pattern,
            &colors,
            size,
            Moment { time_us, fps: 25 },
            &mut out,
        );
        out
    }

    fn px(out: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [out[i], out[i + 1], out[i + 2], out[i + 3]]
    }

    #[test]
    fn every_generator_draws_at_a_tiny_and_a_normal_size() {
        for k in GeneratorKind::ALL {
            for size in [(1, 1), (3, 2), (320, 180)] {
                let out = render(k, size, 1_500_000);
                assert_eq!(out.len(), size.0 * size.1 * 4, "{k:?}");
                assert!(out.iter().any(|b| *b != 9), "{k:?} drew nothing");
            }
        }
    }

    #[test]
    fn the_four_color_gradient_has_its_colours_in_the_corners() {
        let (w, h) = (200, 100);
        let out = render(GeneratorKind::FourColorGradient, (w, h), 0);
        let c = GeneratorKind::FourColorGradient.default_colors();
        let near =
            |p: [u8; 4], c: [u8; 3]| (0..3).all(|i| (i32::from(p[i]) - i32::from(c[i])).abs() <= 6);
        assert!(near(px(&out, w, 0, 0), c[0]));
        assert!(near(px(&out, w, w - 1, 0), c[1]));
        assert!(near(px(&out, w, 0, h - 1), c[2]));
        assert!(near(px(&out, w, w - 1, h - 1), c[3]));
    }

    #[test]
    fn full_bars_run_white_to_black() {
        let w = 160;
        let out = render(GeneratorKind::FullColorBars, (w, 10), 0);
        assert_eq!(px(&out, w, 2, 5), [255, 255, 255, 255]);
        assert_eq!(px(&out, w, 25, 5), [255, 255, 0, 255]);
        assert_eq!(px(&out, w, w - 2, 5), [0, 0, 0, 255]);
        let ebu = render(GeneratorKind::EbuBars, (w, 10), 0);
        assert_eq!(px(&ebu, w, 25, 5), [191, 191, 0, 255]);
    }

    #[test]
    fn the_100mv_steps_are_eight_equal_steps() {
        let w = 800;
        let out = render(GeneratorKind::Steps100mV, (w, 4), 0);
        assert_eq!(px(&out, w, 10, 1)[0], 0);
        assert_eq!(px(&out, w, w - 10, 1)[0], 255);
        let third = px(&out, w, 3 * 100 + 50, 1)[0];
        assert_eq!(third, byte(3.0 / 7.0));
    }

    #[test]
    fn the_hdr_bars_match_the_recommendation() {
        let (w, h) = (1920, 1080);
        let hlg = render(GeneratorKind::Bt2111HlgNarrow, (w, h), 0);
        // The top strip's first bar is 100 % white, the side column is 40 % grey (code 414).
        assert_eq!(px(&hlg, w, 300, 20), [255, 255, 255, 255]);
        let side = px(&hlg, w, 100, 300);
        assert_eq!(side[0], byte((414.0 - 64.0) / 876.0));
        let pq = render(GeneratorKind::Bt2111PqFull, (w, h), 0);
        assert_eq!(px(&pq, w, 100, 300)[0], byte(409.0 / 1023.0));
        // The ramp rises from left to right.
        let a = px(&pq, w, 900, 760)[0];
        let b = px(&pq, w, 1500, 760)[0];
        assert!(b > a);
    }

    #[test]
    fn the_ycbcr_ramp_has_three_bands() {
        let w = 90;
        let out = render(GeneratorKind::YcbcrRamp, (w, 30), 0);
        // Luma is grey; the colour difference bands are not.
        let y = px(&out, w, 60, 2);
        assert!(y[0] == y[1] && y[1] == y[2]);
        let cb = px(&out, w, 60, 15);
        assert!(cb[2] > cb[0]);
        let cr = px(&out, w, 60, 27);
        assert!(cr[0] > cr[2]);
    }

    #[test]
    fn noise_changes_every_frame_and_stays_the_same_within_one() {
        let a = render(GeneratorKind::Noise, (64, 36), 0);
        let again = render(GeneratorKind::Noise, (64, 36), 10_000);
        let next = render(GeneratorKind::Noise, (64, 36), 80_000);
        assert_eq!(a, again);
        assert_ne!(a, next);
        let color = render(GeneratorKind::ColorNoise, (64, 36), 0);
        assert!(color.chunks_exact(4).any(|p| p[0] != p[1]));
    }

    #[test]
    fn noise_gradient_drifts_and_stays_between_its_colours() {
        let a = render(GeneratorKind::NoiseGradient, (80, 45), 0);
        let b = render(GeneratorKind::NoiseGradient, (80, 45), 2_000_000);
        assert_ne!(a, b);
        let c = GeneratorKind::NoiseGradient.default_colors();
        let (lo, hi) = (c[0][0].min(c[1][0]), c[0][0].max(c[1][0]));
        assert!(a.chunks_exact(4).all(|p| p[0] >= lo && p[0] <= hi));
    }

    #[test]
    fn the_grid_has_lines_and_background() {
        let (w, h) = (1920, 1080);
        let out = render(GeneratorKind::Grid, (w, h), 0);
        assert_eq!(px(&out, w, 1, 540)[0], 255); // the heavy line at the edge
        assert_eq!(px(&out, w, 60, 60), [0, 0, 0, 255]); // inside a cell
        assert_eq!(px(&out, w, 120, 60)[0], 255); // the next line
    }

    #[test]
    fn the_window_is_colour_inside_and_background_outside() {
        for k in [GeneratorKind::WindowRectangle, GeneratorKind::WindowEllipse] {
            let (w, h) = (200, 100);
            let out = render(k, (w, h), 0);
            assert_eq!(px(&out, w, 100, 50), [255, 255, 255, 255], "{k:?}");
            assert_eq!(px(&out, w, 2, 2), [0, 0, 0, 255], "{k:?}");
        }
    }

    #[test]
    fn the_countdown_sweeps_and_shows_a_number() {
        let (w, h) = (320, 180);
        let start = render(GeneratorKind::Countdown, (w, h), 0);
        let half = render(GeneratorKind::Countdown, (w, h), 500_000);
        let sweep = |o: &[u8]| {
            o.chunks_exact(4)
                .filter(|p| p[..3] == GeneratorKind::Countdown.default_colors()[1])
                .count()
        };
        assert!(sweep(&half) > sweep(&start));
        // Next second, a different number: the pictures differ in the middle.
        let next = render(GeneratorKind::Countdown, (w, h), 1_000_000);
        assert_ne!(start, next);
    }

    #[test]
    fn a_time_that_is_a_hair_under_a_frame_is_that_frame() {
        for fps in [24u64, 25, 30, 60] {
            for frame in 0..500u64 {
                // The time the compositor gives frame `frame`, cut to whole microseconds.
                let us = (frame as u128 * 1_000_000 / u128::from(fps)) as i64;
                assert_eq!(frame_of(us, fps), frame, "frame {frame} at {fps}");
            }
        }
        // 29.97 frames a second counts in 30s and still lands on the frame.
        for frame in 0..500u64 {
            let us = (frame as u128 * 1_000_000 * 1001 / 30_000) as i64;
            assert_eq!(frame_of(us, 30), frame);
        }
    }

    #[test]
    fn the_timecode_is_burnt_in_over_nothing() {
        let (w, h) = (320, 180);
        let out = render(GeneratorKind::TimecodeBurnIn, (w, h), 3_000_000);
        assert_eq!(px(&out, w, 2, 2), [0, 0, 0, 0]);
        assert!(out.chunks_exact(4).any(|p| p[3] > 0));
        assert_eq!(timecode(25 * 3661 + 7, 25), "01:01:01:07");
    }
}
