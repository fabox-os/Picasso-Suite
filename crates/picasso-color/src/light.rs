//! Light effects: Light Rays, Bloom, Halation, Lens Flare, Aperture Diffraction and Lens
//! Reflections.
//!
//! Each works on one picture (RGBA8, straight alpha) and has one strength, 0 to 100, that scales
//! its settings from nothing (0) through their usual values (50) to twice as much (100). They all
//! add light to the picture: the added colour raises the alpha with it, so light also shows over
//! clear areas. Sizes are in thousandths of the picture's height. The ones that gather light
//! along long paths (rays, spikes, reflections) work at half the size and are laid over the
//! picture with a smooth upscale, which keeps them fast and loses nothing, as they are soft.

use crate::fx::{gaussian, EffectKind};
use crate::look::{par_pixels, put, Src};

const TAU: f32 = std::f32::consts::TAU;

fn unit(h: usize) -> f32 {
    h as f32 / 1080.0
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn straight(q: [f32; 4]) -> [f32; 3] {
    if q[3] <= 1e-6 {
        return [0.0; 3];
    }
    [q[0] / q[3], q[1] / q[3], q[2] / q[3]]
}

/// Adds light `g` (colour, premultiplied) to the pixel: the alpha rises with it.
fn add_light(px: &mut [u8], g: [f32; 3]) {
    let a = f32::from(px[3]) / 255.0;
    let g = g.map(|v| v.max(0.0));
    let s = [0, 1, 2].map(|i| f32::from(px[i]) / 255.0 * a);
    let na = a + (1.0 - a) * g[0].max(g[1]).max(g[2]).clamp(0.0, 1.0);
    let rgb = [0, 1, 2].map(|i| (s[i] + g[i]).min(na));
    put(px, [rgb[0], rgb[1], rgb[2], na]);
}

/// A picture of floats (three channels) read with bilinear sampling, the edge held.
struct Field {
    w: usize,
    h: usize,
    d: Vec<f32>,
}

impl Field {
    /// The picture at half the size: each cell the mean of the (up to four) pixels under it
    /// after `f` has turned the premultiplied pixel into a colour.
    fn half(rgba: &[u8], w: usize, h: usize, f: &(dyn Fn([f32; 4]) -> [f32; 3] + Sync)) -> Field {
        let (w2, h2) = (w.div_ceil(2), h.div_ceil(2));
        let src = Src::new(rgba, w, h);
        let mut d = vec![0.0f32; w2 * h2 * 3];
        for y in 0..h2 {
            for x in 0..w2 {
                let mut sum = [0.0f32; 3];
                let mut n = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (px, py) = (x * 2 + dx, y * 2 + dy);
                    if px < w && py < h {
                        let c = f(src.texel(px, py));
                        for i in 0..3 {
                            sum[i] += c[i];
                        }
                        n += 1.0;
                    }
                }
                for i in 0..3 {
                    d[(y * w2 + x) * 3 + i] = sum[i] / n;
                }
            }
        }
        Field { w: w2, h: h2, d }
    }

    fn at(&self, x: f32, y: f32) -> [f32; 3] {
        let fx = (x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = (y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let get = |x: usize, y: usize, i: usize| self.d[(y * self.w + x) * 3 + i];
        [0, 1, 2].map(|i| {
            mix(
                mix(get(x0, y0, i), get(x1, y0, i), tx),
                mix(get(x0, y1, i), get(x1, y1, i), tx),
                ty,
            )
        })
    }
}

/// Lays half-size light (`light`, RGBA8 holding the colour) over the picture, upscaled smoothly.
fn lay_over(rgba: &mut [u8], w: usize, h: usize, light: &[u8], w2: usize, h2: usize) {
    let src = Src::new(light, w2, h2);
    par_pixels(rgba, w, h, &|x, y, px| {
        let c = src.at((x as f32 + 0.5) / 2.0, (y as f32 + 0.5) / 2.0);
        add_light(px, [c[0], c[1], c[2]]);
    });
}

/// Runs `f(u, v)` (the light at a point, `u` and `v` 0 to 1) at half the picture's size and
/// lays the result over the picture. The light is held to 0 to 1, which changes nothing: more
/// than full light shows the same.
fn half_light(rgba: &mut [u8], w: usize, h: usize, f: &(dyn Fn(f32, f32) -> [f32; 3] + Sync)) {
    let (w2, h2) = (w.div_ceil(2), h.div_ceil(2));
    let mut light = vec![0u8; w2 * h2 * 4];
    par_pixels(&mut light, w2, h2, &|x, y, px| {
        let g = f((x as f32 + 0.5) / w2 as f32, (y as f32 + 0.5) / h2 as f32);
        for i in 0..3 {
            px[i] = (g[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        px[3] = 255;
    });
    lay_over(rgba, w, h, &light, w2, h2);
}

// ---- Light Rays ----

fn light_rays(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let length = (0.4 * k).clamp(0.0, 0.95);
    if length <= 0.0 {
        return;
    }
    let intensity = 2.0 * k;
    let tint = [1.0, 244.0 / 255.0, 214.0 / 255.0].map(|v| v * intensity);
    let bright = Field::half(rgba, w, h, &|t| {
        let m = smoothstep(0.5, 0.7, luma(straight(t)));
        [t[0] * m, t[1] * m, t[2] * m]
    });
    let log_scale = (1.0 - length).ln();
    const TAPS: usize = 32;
    let (fw, fh) = (bright.w as f32, bright.h as f32);
    half_light(rgba, w, h, &|u, v| {
        let (dx, dy) = (u - 0.5, v - 0.5);
        let mut sum = [0.0f32; 3];
        for i in 0..TAPS {
            let t = (i as f32 + 0.5) / TAPS as f32;
            let s = (log_scale * t).exp();
            let c = bright.at((0.5 + dx * s) * fw, (0.5 + dy * s) * fh);
            for j in 0..3 {
                sum[j] += c[j];
            }
        }
        [0, 1, 2].map(|j| sum[j] / TAPS as f32 * tint[j])
    });
}

// ---- Bloom and Halation ----

/// The premultiplied picture where its luma is above a soft threshold.
fn bright_pass(rgba: &[u8], threshold: f32, knee: f32) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        let a = f32::from(px[3]) / 255.0;
        let c = [0, 1, 2].map(|i| f32::from(px[i]) / 255.0);
        let m = smoothstep(threshold - knee, threshold + knee, luma(c));
        // Keep the colour (straight) and scale the coverage: the blur then spreads premultiplied
        // light.
        for v in px.iter_mut().take(3) {
            *v = (f32::from(*v) * a * m + 0.5) as u8;
        }
        px[3] = (a * m * 255.0 + 0.5) as u8;
    }
    out
}

fn bloom(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let radius = 30.0 * unit(h) * k;
    let intensity = 0.8 * k;
    let bright = bright_pass(rgba, 0.6, 0.2);
    let blur = |r: f32| {
        let mut b = bright.clone();
        gaussian(&mut b, w, h, (r.round() as usize).max(1));
        b
    };
    let (l1, l2, l3) = (blur(radius * 0.5), blur(radius), blur(radius * 2.0));
    let weights = [0.5 * intensity, 0.35 * intensity, 0.35 * intensity];
    par_pixels(rgba, w, h, &|x, y, px| {
        let i = (y * w + x) * 4;
        let g = [0, 1, 2].map(|c| {
            (f32::from(l1[i + c]) * weights[0]
                + f32::from(l2[i + c]) * weights[1]
                + f32::from(l3[i + c]) * weights[2])
                / 255.0
        });
        add_light(px, g);
    });
}

fn halation(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let radius = 8.0 * unit(h) * k;
    let amount = 1.0 * k;
    let tint = [1.0, 0.28, 0.1].map(|v| v * amount);
    let bright = bright_pass(rgba, 0.65, 0.1);
    let mut halo = bright.clone();
    gaussian(&mut halo, w, h, (radius * 1.2).round().max(1.0) as usize);
    par_pixels(rgba, w, h, &|x, y, px| {
        let i = (y * w + x) * 4;
        let hl = luma([0, 1, 2].map(|c| f32::from(halo[i + c]) / 255.0));
        // The halo stays off the bright core itself.
        let core = if bright[i + 3] > 0 {
            luma([0, 1, 2].map(|c| f32::from(px[c]) / 255.0))
        } else {
            0.0
        };
        let g = tint.map(|t| t * hl * (1.0 - core));
        add_light(px, g);
    });
}

// ---- Lens Flare ----

fn lens_flare(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let u = unit(h);
    let (wf, hf) = (w as f32, h as f32);
    let light = [wf / 2.0 - 400.0 * u, hf / 2.0 - 220.0 * u];
    let axis = [wf / 2.0, hf / 2.0];
    let size = (160.0 * u).max(1.0);
    let streaks = 6.0f32;
    let ghosts = 1.0f32;
    let color = [1.0, 220.0 / 255.0, 170.0 / 255.0].map(|v| v * k);
    const POS: [f32; 5] = [0.45, 0.8, 1.15, 1.5, -0.35];
    const SIZE: [f32; 5] = [0.25, 0.12, 0.4, 0.18, 0.15];
    const TINT: [[f32; 3]; 5] = [
        [0.4, 0.8, 1.0],
        [1.0, 0.6, 0.3],
        [0.5, 1.0, 0.6],
        [0.9, 0.5, 1.0],
        [1.0, 0.9, 0.5],
    ];
    par_pixels(rgba, w, h, &|x, y, px| {
        let p = [x as f32 + 0.5, y as f32 + 0.5];
        let d = [p[0] - light[0], p[1] - light[1]];
        let r = d[0].hypot(d[1]);
        let core = (-(r / (0.18 * size)).powi(2)).exp() + 0.35 * (-r / (0.6 * size)).exp();
        let streak = (streaks * d[1].atan2(d[0])).cos().max(0.0).powf(60.0)
            * (-r / (2.5 * size)).exp()
            * 0.8;
        let ring = |off: f32| (-((r - off * size) / (0.06 * size)).powi(2)).exp() * 0.18;
        let rings = [ring(1.55), ring(1.5), ring(1.45)];
        let mut g = [0, 1, 2].map(|i| color[i] * (core + streak) + rings[i] * color[i]);
        let ax = [axis[0] - light[0], axis[1] - light[1]];
        for i in 0..5 {
            let gp = [
                light[0] + ax[0] * (1.0 + POS[i]),
                light[1] + ax[1] * (1.0 + POS[i]),
            ];
            let gr = SIZE[i] * size;
            let disc = 1.0 - smoothstep(gr * 0.75, gr, (p[0] - gp[0]).hypot(p[1] - gp[1]));
            for j in 0..3 {
                g[j] += TINT[i][j] * color[j] * disc * 0.12 * ghosts;
            }
        }
        add_light(px, g);
    });
}

// ---- Aperture Diffraction ----

fn aperture_diffraction(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let u = unit(h);
    let spikes = 6usize;
    let length = (120.0 * u * k.min(1.5)).max(1.0);
    let threshold = 0.8f32;
    let intensity = 1.0 * k;
    let scale = 1.0 / (1.0 - threshold).max(0.02);
    let field = Field::half(rgba, w, h, &|t| {
        straight(t).map(|c| (c - threshold).max(0.0) * scale * t[3])
    });
    const S: usize = 24;
    let (fw, fh) = (field.w as f32, field.h as f32);
    half_light(rgba, w, h, &|uu, vv| {
        let (cx, cy) = (uu * fw, vv * fh);
        let mut acc = [0.0f32; 3];
        for i in 0..spikes {
            let ang = TAU * i as f32 / spikes as f32;
            let (dy, dx) = ang.sin_cos();
            for s in 1..=S {
                let t = s as f32 / S as f32;
                // Light at p - dir * d spreads along +dir: brightest next to the light, nothing
                // at the full length. Half-size cells: half the offset.
                let off = t * length * 0.5;
                let c = field.at(cx - dx * off, cy - dy * off);
                let wgt = (1.0 - t) * (1.0 - t);
                for j in 0..3 {
                    acc[j] += c[j] * wgt;
                }
            }
        }
        let norm = intensity * 0.1 * (length / S as f32);
        acc.map(|v| v * norm)
    });
}

// ---- Lens Reflections ----

fn lens_reflections(rgba: &mut [u8], w: usize, h: usize, a: f32) {
    let k = a * 2.0;
    let u = unit(h);
    let n = 4usize;
    let threshold = 0.7f32;
    let intensity = 0.8 * k;
    let spread = 1.0f32;
    let tint = [1.0, 0xd8 as f32 / 255.0, 0xb0 as f32 / 255.0];
    let scale = 1.0 / (1.0 - threshold).max(0.02);
    let mut soft = rgba.to_vec();
    gaussian(&mut soft, w, h, (12.0 * u * 2.0).round().max(1.0) as usize);
    let src = Src::new(&soft, w, h);
    let (wf, hf) = (w as f32, h as f32);
    half_light(rgba, w, h, &|uu, vv| {
        let mut acc = [0.0f32; 3];
        for kk in 1..=n {
            let f = (spread * kk as f32 / n as f32).max(0.05);
            let sx = 0.5 - (uu - 0.5) / f;
            let sy = 0.5 - (vv - 0.5) / f;
            if !(0.0..=1.0).contains(&sx) || !(0.0..=1.0).contains(&sy) {
                continue;
            }
            let s = straight(src.at(sx * wf, sy * hf));
            let l = luma(s);
            let mut around = 0.0;
            for i in 0..8 {
                let ang = std::f32::consts::FRAC_PI_4 * i as f32;
                let q = src.at((sx + 0.06 * ang.cos()) * wf, (sy + 0.06 * ang.sin()) * hf);
                around += luma(straight(q));
            }
            around /= 8.0;
            let ex = smoothstep(0.0, 0.1, sx.min(1.0 - sx));
            let ey = smoothstep(0.0, 0.1, sy.min(1.0 - sy));
            let hh = (l - threshold.max(around + 0.1)).max(0.0) * scale * ex * ey;
            let ratio = hh / l.max(1e-3);
            let hue =
                [0.0f32, 0.33, 0.67].map(|o| 0.5 + 0.5 * (TAU * (o + kk as f32 * 0.17)).cos());
            for j in 0..3 {
                acc[j] += s[j] * ratio * mix(1.0, hue[j], 0.6) / (1.0 + 0.5 * kk as f32);
            }
        }
        [0, 1, 2].map(|j| acc[j] * tint[j] * intensity)
    });
}

/// Applies `kind` at strength `a` (0 to 1) to a `w` x `h` RGBA8 picture.
pub(crate) fn apply(kind: EffectKind, a: f32, rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || a <= 0.0 {
        return;
    }
    match kind {
        EffectKind::LightRays => light_rays(rgba, w, h, a),
        EffectKind::Bloom => bloom(rgba, w, h, a),
        EffectKind::Halation => halation(rgba, w, h, a),
        EffectKind::LensFlare => lens_flare(rgba, w, h, a),
        EffectKind::ApertureDiffraction => aperture_diffraction(rgba, w, h, a),
        EffectKind::LensReflections => lens_reflections(rgba, w, h, a),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Effect;

    const W: usize = 160;
    const H: usize = 90;

    const KINDS: [EffectKind; 6] = [
        EffectKind::LightRays,
        EffectKind::Bloom,
        EffectKind::Halation,
        EffectKind::LensFlare,
        EffectKind::ApertureDiffraction,
        EffectKind::LensReflections,
    ];

    fn flat(g: u8) -> Vec<u8> {
        (0..W * H).flat_map(|_| [g, g, g, 255]).collect()
    }

    /// A dark picture with one bright square at (x0..x0+8, y0..y0+8).
    fn lamp(x0: usize, y0: usize) -> Vec<u8> {
        let mut p = flat(10);
        for y in y0..y0 + 8 {
            for x in x0..x0 + 8 {
                let i = (y * W + x) * 4;
                p[i..i + 3].fill(255);
            }
        }
        p
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

    fn sum(p: &[u8]) -> u64 {
        p.chunks_exact(4).map(|q| u64::from(q[0])).sum()
    }

    fn at(p: &[u8], x: usize, y: usize) -> u8 {
        p[(y * W + x) * 4]
    }

    #[test]
    fn nothing_changes_at_zero_and_all_add_light_to_a_lamp_at_the_usual_strength() {
        let pic = lamp(100, 60);
        for kind in KINDS {
            assert_eq!(run(kind, 0.0, &pic), pic, "{kind:?} at zero");
            let out = run(kind, kind.default_amount(), &pic);
            assert!(sum(&out) > sum(&pic), "{kind:?} added no light");
        }
    }

    #[test]
    fn light_never_darkens_and_the_picture_keeps_its_size() {
        let pic = lamp(40, 30);
        for kind in KINDS {
            let out = run(kind, 100.0, &pic);
            assert_eq!(out.len(), pic.len());
            for (a, b) in out.chunks_exact(4).zip(pic.chunks_exact(4)) {
                assert!(a[0] >= b[0].saturating_sub(1), "{kind:?}");
            }
        }
    }

    #[test]
    fn a_flat_dark_picture_has_nothing_to_glow() {
        for kind in [
            EffectKind::LightRays,
            EffectKind::Bloom,
            EffectKind::Halation,
            EffectKind::ApertureDiffraction,
            EffectKind::LensReflections,
        ] {
            let pic = flat(20);
            assert_eq!(sum(&run(kind, 100.0, &pic)), sum(&pic), "{kind:?}");
        }
    }

    #[test]
    fn bloom_spreads_around_the_lamp_and_halation_is_red() {
        let pic = lamp(76, 41);
        let bloom = run(EffectKind::Bloom, 100.0, &pic);
        assert!(at(&bloom, 70, 45) > at(&pic, 70, 45) + 3);
        let halo = run(EffectKind::Halation, 100.0, &pic);
        let i = (45 * W + 75) * 4;
        assert!(halo[i] > halo[i + 2], "the halo is warm");
    }

    #[test]
    fn rays_run_away_from_the_middle_and_spikes_spread_from_the_lamp() {
        let pic = lamp(120, 41);
        let rays = run(EffectKind::LightRays, 100.0, &pic);
        // The rays run from the lamp away from the middle of the frame.
        assert!(at(&rays, 142, 45) > at(&pic, 142, 45) + 3);
        let spikes = run(EffectKind::ApertureDiffraction, 100.0, &lamp(76, 41));
        // A spike along the horizontal axis (angle 0).
        assert!(at(&spikes, 95, 45) > at(&spikes, 95, 20) + 3);
    }

    #[test]
    fn a_lens_flare_is_drawn_around_its_light_and_over_clear_pictures() {
        let pic = flat(0);
        let out = run(EffectKind::LensFlare, 100.0, &pic);
        // The light sits left of and above the middle.
        let (lx, ly) = (
            W / 2 - (400.0 * H as f32 / 1080.0) as usize,
            H / 2 - (220.0 * H as f32 / 1080.0) as usize,
        );
        assert!(at(&out, lx, ly) > 100);
        let clear = vec![0u8; W * H * 4];
        let out = run(EffectKind::LensFlare, 100.0, &clear);
        assert!(
            out.chunks_exact(4).any(|p| p[3] > 0),
            "light shows over clear areas"
        );
    }

    #[test]
    fn a_lens_reflection_appears_on_the_far_side_of_the_middle() {
        let pic = lamp(30, 41);
        let out = run(EffectKind::LensReflections, 100.0, &pic);
        let mirrored = out
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, _)| i % W > W / 2)
            .map(|(_, p)| u64::from(p[0]))
            .sum::<u64>();
        let before = pic
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, _)| i % W > W / 2)
            .map(|(_, p)| u64::from(p[0]))
            .sum::<u64>();
        assert!(mirrored > before);
    }

    #[test]
    fn they_work_on_tiny_pictures() {
        for kind in KINDS {
            for (w, h) in [(1usize, 1usize), (2, 3), (5, 1)] {
                let mut p = vec![250u8; w * h * 4];
                Effect {
                    amount: 100.0,
                    ..Effect::new(kind)
                }
                .apply(&mut p, w, h, 2);
                assert_eq!(p.len(), w * h * 4);
            }
        }
    }

    /// Not a check, a measurement:
    /// `cargo test -p picasso-color --release timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_of_each_at_1080p() {
        let (w, h) = (1920, 1080);
        let pic: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        for kind in EffectKind::ALL {
            let mut p = pic.clone();
            let start = std::time::Instant::now();
            Effect::new(kind).apply(&mut p, w, h, 3);
            eprintln!(
                "{:>20}: {:>5} ms",
                kind.label(),
                start.elapsed().as_millis()
            );
        }
    }
}
