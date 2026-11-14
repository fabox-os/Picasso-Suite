//! The look transitions: 18 ways to turn one picture into the next that are made of noise, cells
//! and light (ink spreading, bubbles, honeycomb, shattering glass, digital rain, a VHS roll, ...).
//! Each is the outgoing picture at progress 0 and the incoming one at progress 1.
//!
//! They are drawn per pixel on the processor, in bands of rows on every core, the way the other
//! transitions are. Positions are in picture units (`u` 0 to 1 from the left, `v` 0 to 1 from the
//! top); where a shape has to stay round, `u` is first multiplied by the picture's aspect ratio.

use picasso_core::TransitionKind as K;

use crate::transitions::{
    cmix, cscale, fbm, fract, hash1, hash2, mix, over, smoothstep, step, v2, vnoise, Pic, C4, V2,
};

const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;

struct Frame<'a> {
    a: Pic<'a>,
    b: Pic<'a>,
    aspect: f32,
    /// Pixels across and down.
    res: V2,
    t: f32,
}

pub(crate) fn handles(kind: K) -> bool {
    K::looks().contains(&kind)
}

fn norm(v: V2) -> V2 {
    v / (v.length() + 1e-6)
}

fn floor2(v: V2) -> V2 {
    v2(v.x.floor(), v.y.floor())
}

fn fract2(v: V2) -> V2 {
    v2(fract(v.x), fract(v.y))
}

/// A pair of numbers from a cell.
fn hash22(p: V2) -> V2 {
    v2(hash2(p), hash2(p + v2(31.7, 17.3)))
}

/// Adds `light` (a colour) to a premultiplied colour, as much as the colour is there.
fn glow(c: C4, light: [f32; 3], k: f32) -> C4 {
    [
        (c[0] + light[0] * k * c[3]).min(c[3]),
        (c[1] + light[1] * k * c[3]).min(c[3]),
        (c[2] + light[2] * k * c[3]).min(c[3]),
        c[3],
    ]
}

/// The red, green and blue of one picture read `off` apart (red to the right, blue to the left).
fn split(pic: &Pic, p: V2, off: f32) -> C4 {
    let mid = pic.tex(p);
    let (r, b) = (pic.tex(p + v2(off, 0.0)), pic.tex(p - v2(off, 0.0)));
    [r[0], mid[1], b[2], mid[3]]
}

/// A glyph drawn from the number `seed`: a 3 by 5 grid of dots, some on, in a cell.
fn glyph(seed: V2, inside: V2) -> f32 {
    if inside.x < 0.12 || inside.x > 0.88 || inside.y < 0.1 || inside.y > 0.9 {
        return 0.0;
    }
    let (cx, cy) = (
        ((inside.x - 0.12) / 0.76 * 3.0).floor(),
        ((inside.y - 0.1) / 0.8 * 5.0).floor(),
    );
    if hash2(seed + v2(cx * 7.1, cy * 3.3)) > 0.45 {
        1.0
    } else {
        0.0
    }
}

fn shade(kind: K, f: &Frame, uv: V2) -> C4 {
    let (t, aspect) = (f.t, f.aspect);
    let sq = v2(aspect, 1.0);
    let ends = (t * PI).sin();
    match kind {
        // A ring of static runs out from the middle and pushes the picture apart as it passes.
        K::InterferenceBlast => {
            let d = (uv - v2(0.5, 0.5)) * sq;
            let r = d.length();
            let dir = norm(d);
            let width = 0.14;
            let front = t * (0.5 * (aspect * aspect + 1.0).sqrt() + width * 2.0);
            let live = smoothstep(0.0, 0.04, t);
            let ring = smoothstep(width, 0.0, (r - front).abs()) * live;
            let off = dir * (ring * 0.5 * 0.18) / sq;
            let (a, b) = (f.a.tex(uv - off), f.b.tex(uv + off * 0.5));
            let c = cmix(a, b, smoothstep(0.0, width, front - r));
            let n = hash2(uv * f.res + v2((t * 60.0).floor(), 0.0));
            let c = [
                mix(c[0], n * c[3], ring * 0.6),
                mix(c[1], n * c[3], ring * 0.6),
                mix(c[2], n * c[3], ring * 0.6),
                c[3],
            ];
            glow(c, [1.0; 3], ring * 0.35)
        }
        // Soap bubbles rise and grow; where one is, the next picture shows, a little bent.
        K::Bubbles => {
            let cells = 12.0;
            let g = v2(uv.x * aspect, uv.y) * cells;
            let (id, inside) = (floor2(g), fract2(g) - v2(0.5, 0.5));
            let (r1, r2) = (hash2(id), hash2(id + v2(11.3, 11.3)));
            let stagger = 0.6;
            let tt = (t * (1.0 + stagger) - r1 * stagger).clamp(0.0, 1.0);
            let centre = v2((r2 - 0.5) * 0.4, (r1 - 0.5) * 0.3 - 0.45 * tt);
            let radius = tt * (0.55 + r2 * 0.35) * 1.3;
            let d = (inside - centre).length();
            let m = (1.0 - smoothstep(radius - 0.07, radius + 0.07, d)) * smoothstep(0.0, 0.08, tt);
            let m = m.max(smoothstep(0.88, 1.0, t));
            // A bubble bends the picture behind it a little, like a lens.
            let bend = norm(inside - centre) * (m * (1.0 - m) * 0.5 * 0.06);
            cmix(f.a.tex(uv), f.b.tex(uv + bend / sq), m)
        }
        // Each picture zooms and twists towards the middle, and the second comes out of the same
        // spin; the picture is blurred along the zoom.
        K::ZoomSwirl => {
            let d = (uv - v2(0.5, 0.5)) * sq;
            let r = d.length();
            let angle = d.y.atan2(d.x) + ends * 1.6 * (1.0 - smoothstep(0.0, 0.75, r));
            let dir = v2(angle.cos(), angle.sin());
            let (zoom_a, zoom_b) = (1.0 + ends * 0.6, 1.0 - ends * 0.6 * 0.8);
            let m = smoothstep(0.35, 0.65, t);
            let mut sum = [0.0f32; 4];
            const TAPS: usize = 12;
            for i in 0..TAPS {
                let s = i as f32 / (TAPS - 1) as f32 - 0.5;
                let scale = 1.0 + s * ends * 0.5 * 0.25;
                let pa = v2(0.5, 0.5) + dir * (r * zoom_a * scale) / sq;
                let pb = v2(0.5, 0.5) + dir * (r * zoom_b * scale) / sq;
                let c = cmix(f.a.tex(pa), f.b.tex(pb), m);
                for k in 0..4 {
                    sum[k] += c[k] / TAPS as f32;
                }
            }
            sum
        }
        // A hexagon grows in every cell of a honeycomb, cell after cell.
        K::Honeycomb => {
            let p = v2(uv.x * aspect, uv.y) * 8.0;
            let r = v2(1.0, 1.732_050_8);
            let h = r * 0.5;
            let modr = |q: V2| v2(q.x - r.x * (q.x / r.x).floor(), q.y - r.y * (q.y / r.y).floor());
            let (a, b) = (modr(p) - h, modr(p - h) - h);
            let gv = if a.dot(a) < b.dot(b) { a } else { b };
            let id = p - gv;
            let stagger = 0.5;
            let tt = (t * (1.0 + stagger) - hash2(id) * stagger).clamp(0.0, 1.0);
            let q = v2(gv.x.abs(), gv.y.abs());
            let d = q.x.max(q.dot(v2(0.5, 0.866_025_4)));
            let soft = 0.03;
            let m = smoothstep(tt * 0.64 + soft, tt * 0.64 - soft, d) * smoothstep(0.0, 0.05, tt);
            cmix(f.a.tex(uv), f.b.tex(uv), m)
        }
        // Ink drops soak into the paper, spreading along its fibres, dark where they are wet.
        K::InkBleed => {
            let p = v2(uv.x * aspect, uv.y);
            let drops = [v2(0.30 * aspect, 0.35), v2(0.78 * aspect, 0.62), v2(0.52 * aspect, 0.18)];
            let d = drops
                .iter()
                .map(|c| (p - *c).length())
                .fold(f32::MAX, f32::min);
            let n = fbm(uv * 6.0) - 0.5;
            let soft = 0.04;
            // The front is slow at first, then runs, so that half of the picture has changed
            // halfway; it is dry (the first picture) at the start for certain.
            let th = d * 2.2 + n * 0.55 + 0.4 * (1.0 - t) - 2.4 * t.powf(1.5);
            let m = (1.0 - smoothstep(-soft, soft, th)).max(smoothstep(0.96, 1.0, t));
            let c = cmix(f.a.tex(uv), f.b.tex(uv), m);
            let rim = smoothstep(soft * 3.0, 0.0, th.abs());
            let alive = 1.0 - smoothstep(0.9, 1.0, t);
            let ink = [0.04, 0.04, 0.10];
            let k = rim * 0.7 * alive;
            [
                mix(c[0], ink[0] * c[3], k),
                mix(c[1], ink[1] * c[3], k),
                mix(c[2], ink[2] * c[3], k),
                c[3],
            ]
        }
        // The picture folds into a kaleidoscope that turns, and back out of it.
        K::KaleidoscopeSpin => {
            let d = (uv - v2(0.5, 0.5)) * sq;
            let r = d.length();
            let angle = d.y.atan2(d.x);
            let segment = TAU / 6.0;
            let turned = (angle + t * TAU).rem_euclid(segment);
            let folded = (turned - segment * 0.5).abs();
            let a2 = mix(angle, folded, ends);
            let rr = r * (1.0 - 0.4 * ends);
            let p = v2(0.5, 0.5) + v2(a2.cos(), a2.sin()) * rr / sq;
            cmix(f.a.tex(p), f.b.tex(p), smoothstep(0.4, 0.6, t))
        }
        // A bright streak sweeps across; the pictures change behind it.
        K::LensFlareWipe => {
            let sweep = mix(-0.3, 1.3, t);
            let d = uv.x - sweep;
            let width = 0.3f32;
            let core = (-d * d / (width * width * 0.02)).exp();
            let halo = (-d * d / (width * width * 0.4)).exp()
                * (-((uv.y - 0.5).abs() * 2.0).powi(2) / 0.7).exp();
            let live = smoothstep(0.0, 0.06, t) * (1.0 - smoothstep(0.94, 1.0, t));
            let light = (core + halo * 0.55) * 1.4 * live;
            let push = v2(core * 0.03 * 0.5 * live, 0.0);
            let m = smoothstep(-0.08, 0.08, -d);
            let c = cmix(f.a.tex(uv + push), f.b.tex(uv - push), m);
            glow(c, [1.0, 0.85, 0.6], light)
        }
        // The picture is smeared like wet paint, and the next comes through where it is smeared most.
        K::LiquidSmudge => {
            let q = uv * 5.0;
            let w = v2(fbm(q) - 0.5, fbm(q + v2(17.3, 17.3)) - 0.5) * 2.0;
            let amt = ends * 0.25;
            let (a, b) = (f.a.tex(uv + w * amt), f.b.tex(uv - w * amt * 0.6));
            let m = smoothstep(0.2, 0.85, t + (w.length() - 0.3) * 0.5 * ends);
            cmix(a, b, m)
        }
        // Dark parts of the first picture turn into the second first, then the bright ones.
        K::LumaFade => {
            let a = f.a.tex(uv);
            let straight = if a[3] > 1e-6 { 1.0 / a[3] } else { 0.0 };
            let l = (0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2]) * straight;
            let soft = 0.25;
            let th = mix(-soft, 1.0 + soft, t);
            let m = 1.0 - smoothstep(th, th + soft, l);
            cmix(a, f.b.tex(uv), m)
        }
        // Green characters fall down the screen in columns and leave the next picture behind.
        K::DigitalRain => {
            let cols = 48.0f32;
            let rows = (cols / aspect).floor().max(1.0);
            let (col, row) = ((uv.x * cols).floor(), (uv.y * rows).floor());
            let r = hash1(col);
            let trail = 0.45;
            let head = t * (1.0 + trail) * (0.75 + r * 0.5);
            let dist = head - row / rows;
            let lit = (1.0 - dist / trail).clamp(0.0, 1.0) * step(0.0, dist);
            let spark = smoothstep(trail * 0.2, 0.0, dist.abs());
            // The characters change a dozen times.
            let tick = (t * 12.0).floor();
            let inside = v2(fract(uv.x * cols), fract(uv.y * rows));
            let g = glyph(v2(col, row + tick * 7.0), inside);
            let m = smoothstep(0.0, 0.06, dist);
            let c = cmix(f.a.tex(uv), f.b.tex(uv), m);
            let alive = smoothstep(0.0, 0.04, t) * (1.0 - smoothstep(0.85, 1.0, t));
            let green = [0.2, 1.0, 0.4];
            let white = [0.9, 1.0, 0.95];
            let rain = [0, 1, 2].map(|i| mix(green[i], white[i], spark));
            glow(c, rain, g * lit.max(spark) * 1.1 * alive)
        }
        // The first picture blows away as grains, in the wind, and the second is under it.
        K::ParticleWind => {
            let grain = vnoise(uv * 30.0);
            let flow = v2(fbm(uv * 3.0) - 0.5, fbm(uv * 3.0 + v2(9.1, 9.1)) - 0.5) * 2.0;
            let wind = v2(1.0, 0.0);
            let tt = t * t;
            let off = (wind + flow * 0.5) * (tt * 0.45 * 0.6) / sq;
            let a = f.a.tex(uv - off);
            let lead = (uv - v2(0.5, 0.5)).dot(wind) * 0.5 + 0.5;
            let th = t * 1.6 - (grain * 0.7 + lead * 0.3) * 0.6 - 0.3;
            let a = cscale(a, 1.0 - smoothstep(0.0, 0.3, th));
            let a = glow(a, [1.0; 3], smoothstep(0.8, 1.0, grain) * 0.12 * (4.0 * t * (1.0 - t)));
            over(a, f.b.tex(uv))
        }
        // The picture turns to big blocks, and back to sharp: the colours swap in some of them.
        K::PixelSwap => {
            let amt = ends * 0.8;
            // Whole blocks only; at the ends they are single pixels.
            let columns = mix(f.res.x, 6.0, amt).round().max(1.0);
            let grid = v2(columns, (columns / aspect).round().max(1.0));
            let cell = floor2(uv * grid);
            let p = (cell + v2(0.5, 0.5)) / grid;
            let c = cmix(f.a.tex(p), f.b.tex(p), smoothstep(0.35, 0.65, t));
            let swap = amt * 0.6 * step(0.35, hash2(cell));
            [
                mix(c[0], c[1], swap),
                mix(c[1], c[2], swap),
                mix(c[2], c[0], swap),
                c[3],
            ]
        }
        // Noise burns the first picture away: a glowing front runs through it.
        K::PlasmaBurn => {
            let n = fbm(uv * 5.0 + v2(t * 0.4, t * 0.25));
            let front = mix(-0.15, 1.15, t);
            let d = n - front;
            let m = 1.0 - smoothstep(0.0, 0.04, d);
            let c = cmix(f.a.tex(uv), f.b.tex(uv), m);
            let band = smoothstep(0.07, 0.0, d.abs());
            let alive = 1.0 - smoothstep(0.88, 1.0, t);
            let hot = smoothstep(0.07, 0.0, (d + 0.02).abs());
            let (cool, warm) = ([0.9, 0.1, 0.05], [1.0, 0.75, 0.25]);
            let fire = [0, 1, 2].map(|i| mix(cool[i], warm[i], hot));
            glow(c, fire, band * alive)
        }
        // Both pictures are streaked towards the middle, most in the middle of the transition.
        K::RadialZoomBlur => {
            let amt = ends * 0.6;
            let centre = v2(0.5, 0.5);
            const TAPS: usize = 10;
            let (mut a, mut b) = ([0.0f32; 4], [0.0f32; 4]);
            for k in 0..TAPS {
                let p = centre + (uv - centre) * (1.0 - k as f32 * 0.12 * amt / TAPS as f32 * 4.0);
                let (ca, cb) = (f.a.tex(p), f.b.tex(p));
                for i in 0..4 {
                    a[i] += ca[i] / TAPS as f32;
                    b[i] += cb[i] / TAPS as f32;
                }
            }
            cmix(a, b, smoothstep(0.4, 0.6, t))
        }
        // Rows tear sideways and the colours come apart, in steps.
        K::RgbDisplacement => {
            let tick = (t * 24.0).floor();
            let band = (uv.y * 28.0).floor();
            let h = hash2(v2(band, tick));
            let shift = (h - 0.5) * 2.0 * 0.12 * ends * step(0.45, h);
            let p = v2(uv.x + shift, uv.y);
            let off = 18.0 * ends / f.res.x.max(1.0);
            cmix(split(&f.a, p, off), split(&f.b, p, off), smoothstep(0.35, 0.65, t))
        }
        // Little triangles flip over, one after the other, and show the next picture on the back.
        K::TriangleMosaic => {
            let grid = v2(10.0, (10.0 / aspect).floor().max(1.0));
            let g = uv * grid;
            let (id, inside) = (floor2(g), fract2(g));
            let upper = step(inside.x + inside.y, 1.0);
            let stagger = 0.5;
            let tt = (t * (1.0 + stagger) - hash2(id + v2(upper * 17.31, upper * 17.31)) * stagger)
                .clamp(0.0, 1.0);
            let squash = (tt * PI).cos();
            let reveal = step(squash, 0.0);
            let centre = (id + v2(0.5, 0.5)) / grid;
            let p = centre + (uv - centre) / v2(squash.abs().max(0.06), 1.0);
            let c = cmix(f.a.tex(p), f.b.tex(p), reveal);
            cscale_rgb(c, 1.0 - 0.45 * (1.0 - squash.abs()))
        }
        // A tape that rolls: a bar runs down the frame, rows jitter, colours bleed and it is noisy.
        K::VhsRoll => {
            let tick = (t * 48.0).floor();
            let bar = fract(uv.y + t * 1.5);
            let in_bar = step(bar, 0.16);
            let mut j = (hash2(v2((uv.y * 220.0).floor(), tick)) - 0.5) * 0.03 * ends;
            j += in_bar * (hash1(tick) - 0.5) * 0.08 * ends;
            let p = v2(uv.x + j, uv.y);
            let off = 14.0 * ends / f.res.x.max(1.0);
            let c = cmix(split(&f.a, p, off), split(&f.b, p, off), smoothstep(0.4, 0.6, t));
            let n = hash2(uv * f.res + v2(tick, 0.0));
            let k = 0.45 * ends * (0.25 + in_bar * 0.75);
            let c = [
                mix(c[0], n * c[3], k),
                mix(c[1], n * c[3], k),
                mix(c[2], n * c[3], k),
                c[3],
            ];
            let lines = 1.0 - 0.35 * ends * (0.5 + 0.5 * (uv.y * f.res.y * PI).sin());
            cscale_rgb(c, lines)
        }
        // The first picture breaks into shards that fly off, turning, and fade.
        K::Shatter => {
            let cells = 8.0;
            let g = v2(uv.x * aspect, uv.y) * cells;
            let base = floor2(g);
            let (mut best, mut id) = (f32::MAX, base);
            for j in -1..=1 {
                for i in -1..=1 {
                    let nb = base + v2(i as f32, j as f32);
                    let d = (nb + hash22(nb) - g).length();
                    if d < best {
                        (best, id) = (d, nb);
                    }
                }
            }
            let stagger = 0.4;
            let tt = (t * (1.0 + stagger) - hash2(id) * stagger).clamp(0.0, 1.0);
            let dir = norm(hash22(id + v2(3.7, 3.7)) * 2.0 - v2(1.0, 1.0));
            let angle = (hash2(id + v2(9.1, 9.1)) - 0.5) * 1.5 * tt;
            let (c, s) = (angle.cos(), angle.sin());
            let rot = v2(c * dir.x - s * dir.y, s * dir.x + c * dir.y);
            let off = rot * (tt * tt * 0.6) / sq;
            let a = f.a.tex(uv + off);
            let a = cscale(a, 1.0 - smoothstep(0.35, 1.0, tt));
            over(a, f.b.tex(uv))
        }
        _ => f.b.tex(uv),
    }
}

/// A colour's red, green and blue made lighter or darker (the alpha stays).
fn cscale_rgb(c: C4, k: f32) -> C4 {
    [c[0] * k, c[1] * k, c[2] * k, c[3]]
}

/// Makes `second` (the incoming picture, `width` x `height` RGBA8) the picture `t` (0 to 1) of the
/// way through the look transition `kind` from `first`. Returns whether `kind` is drawn here.
pub(crate) fn mix_into(
    kind: K,
    first: &[u8],
    second: &mut [u8],
    width: usize,
    height: usize,
    t: f32,
) -> bool {
    if !handles(kind) {
        return false;
    }
    let Some(len) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
        return true;
    };
    if width == 0 || height == 0 || first.len() < len || second.len() < len {
        return true;
    }
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    if t <= 0.0 {
        second[..len].copy_from_slice(&first[..len]);
        return true;
    }
    if t >= 1.0 {
        return true;
    }
    let incoming = second[..len].to_vec();
    let frame = Frame {
        a: Pic {
            data: &first[..len],
            w: width,
            h: height,
        },
        b: Pic {
            data: &incoming,
            w: width,
            h: height,
        },
        aspect: width as f32 / height as f32,
        res: v2(width as f32, height as f32),
        t,
    };
    let frame = &frame;
    picasso_color::par::par_rows(&mut second[..len], width * 4, height, &|first_row, out| {
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let n = first_row * width + i;
            let (x, y) = (n % width, n / width);
            let uv = v2(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let c = shade(kind, frame, uv);
            let alpha = c[3].clamp(0.0, 1.0);
            let k = if alpha > 1e-6 { 1.0 / alpha } else { 0.0 };
            for ch in 0..3 {
                px[ch] = (c[ch] * k * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
            px[3] = (alpha * 255.0 + 0.5) as u8;
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 96;
    const H: usize = 54;

    fn solid(c: [u8; 3]) -> Vec<u8> {
        (0..W * H).flat_map(|_| [c[0], c[1], c[2], 255]).collect()
    }

    /// Vertical stripes, so that anything that moves, bends or blurs shows.
    fn stripes(a: [u8; 3], b: [u8; 3]) -> Vec<u8> {
        (0..W * H)
            .flat_map(|i| {
                let c = if (i % W / 6).is_multiple_of(2) { a } else { b };
                [c[0], c[1], c[2], 255]
            })
            .collect()
    }

    fn run(kind: K, first: &[u8], second: &[u8], t: f32) -> Vec<u8> {
        let mut out = second.to_vec();
        crate::transitions::mix_into(kind, first, &mut out, W, H, t);
        out
    }

    /// The average difference of a colour channel, 0 to 255.
    fn gap(a: &[u8], b: &[u8]) -> f32 {
        let sum: u64 = a.iter().zip(b).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
        sum as f32 / a.len() as f32
    }

    fn pair() -> (Vec<u8>, Vec<u8>) {
        (stripes([220, 40, 40], [240, 200, 60]), stripes([30, 60, 220], [40, 220, 200]))
    }

    #[test]
    fn there_are_18_looks_in_their_own_list() {
        assert_eq!(K::looks().len(), 18);
        assert!(K::looks().iter().all(|k| handles(*k)));
        assert!(K::wipes().iter().chain(K::extra()).all(|k| !handles(*k)));
        let mut names: Vec<&str> = K::looks().iter().map(|k| k.label()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 18);
    }

    #[test]
    fn every_look_is_the_first_picture_at_the_start_and_the_second_at_the_end() {
        let (a, b) = pair();
        for kind in K::looks() {
            assert_eq!(run(*kind, &a, &b, 0.0), a, "{kind:?} at 0");
            assert_eq!(run(*kind, &a, &b, 1.0), b, "{kind:?} at 1");
            // And a moment in, it is still (nearly) the same.
            let start = gap(&run(*kind, &a, &b, 0.004), &a);
            let end = gap(&run(*kind, &a, &b, 0.996), &b);
            assert!(start < 8.0, "{kind:?} just after the start: {start}");
            assert!(end < 8.0, "{kind:?} just before the end: {end}");
        }
    }

    #[test]
    fn every_look_makes_something_new_in_the_middle_and_stays_opaque() {
        let (a, b) = pair();
        for kind in K::looks() {
            let out = run(*kind, &a, &b, 0.5);
            assert!(gap(&out, &a) > 6.0 && gap(&out, &b) > 6.0, "{kind:?}");
            assert!(out.chunks_exact(4).all(|p| p[3] == 255), "{kind:?} lets the picture through");
            assert_eq!(out, run(*kind, &a, &b, 0.5), "{kind:?} is not repeatable");
            // Sizes that are not even are fine.
            let mut small = vec![0u8; 3 * 2 * 4];
            for p in small.chunks_exact_mut(4) {
                p[3] = 255;
            }
            let first = small.clone();
            crate::transitions::mix_into(*kind, &first, &mut small, 3, 2, 0.5);
        }
    }

    #[test]
    fn the_cell_looks_change_in_pieces() {
        // Red into blue: halfway, some of the picture has changed and some has not.
        let (a, b) = (solid([220, 30, 30]), solid([30, 30, 220]));
        for kind in [K::Honeycomb, K::Bubbles, K::Shatter, K::TriangleMosaic, K::ParticleWind] {
            let out = run(kind, &a, &b, 0.5);
            // Pieces that are more red or more blue than the other (shards fade, so they mix).
            let leaning = |toward_red: bool| {
                out.chunks_exact(4)
                    .filter(|p| {
                        let (r, b) = (i32::from(p[0]), i32::from(p[2]));
                        if toward_red { r - b > 20 } else { b - r > 20 }
                    })
                    .count()
            };
            let (red, blue) = (leaning(true), leaning(false));
            // Shards fade rather than turn, so a few are enough.
            assert!(red > W * H / 20 && blue > 30, "{kind:?}: {red} red, {blue} blue");
        }
    }

    #[test]
    fn luma_fade_hands_over_dark_before_bright() {
        // The first picture: dark on the left, bright on the right.
        let a: Vec<u8> = (0..W * H)
            .flat_map(|i| if i % W < W / 2 { [20, 20, 20, 255] } else { [230, 230, 230, 255] })
            .collect();
        let b = solid([200, 0, 0]);
        let out = run(K::LumaFade, &a, &b, 0.4);
        let at = |x: usize| &out[(10 * W + x) * 4..(10 * W + x) * 4 + 3];
        assert_eq!(at(10)[0], 200, "the dark side is not yet the second picture");
        assert_eq!(at(W - 10), &[230, 230, 230], "the bright side changed too early");
    }

    #[test]
    fn digital_rain_draws_green_that_neither_picture_has() {
        let (a, b) = (solid([220, 30, 30]), solid([30, 30, 220]));
        let out = run(K::DigitalRain, &a, &b, 0.5);
        let green = out.chunks_exact(4).filter(|p| p[1] > 120).count();
        assert!(green > 20, "{green}");
    }

    #[test]
    fn colour_looks_pull_the_channels_apart() {
        let (a, b) = pair();
        for kind in [K::RgbDisplacement, K::VhsRoll] {
            let out = run(kind, &a, &b, 0.3);
            // Where the stripes are, red and blue are not read from the same place: some pixels
            // have a red that neither stripe colour has.
            let odd = out
                .chunks_exact(4)
                .filter(|p| {
                    let known = |c: [u8; 3]| p[0].abs_diff(c[0]) < 25 && p[1].abs_diff(c[1]) < 25 && p[2].abs_diff(c[2]) < 25;
                    !(known([220, 40, 40]) || known([240, 200, 60]) || known([30, 60, 220]) || known([40, 220, 200]))
                })
                .count();
            assert!(odd > W * H / 40, "{kind:?}: {odd}");
        }
    }

    #[test]
    fn the_blur_and_swirl_looks_smooth_the_stripes() {
        let (a, b) = pair();
        let sharpness = |buf: &[u8]| {
            let row = &buf[20 * W * 4..21 * W * 4];
            (1..W).map(|x| u64::from(row[x * 4].abs_diff(row[(x - 1) * 4]))).sum::<u64>()
        };
        for kind in [K::RadialZoomBlur, K::ZoomSwirl] {
            let out = run(kind, &a, &b, 0.5);
            assert!(sharpness(&out) < sharpness(&a), "{kind:?}");
        }
    }

    #[test]
    fn pixel_swap_makes_big_blocks_in_the_middle() {
        let (a, b) = pair();
        let out = run(K::PixelSwap, &a, &b, 0.5);
        // Most pixels are the same as the one on their right: wide blocks.
        let same = out
            .chunks_exact(4)
            .zip(out.chunks_exact(4).skip(1))
            .filter(|(p, q)| p == q)
            .count();
        assert!(same > W * H * 7 / 10, "{same}");
    }

    #[test]
    fn lens_flare_adds_light() {
        let (a, b) = (solid([60, 60, 60]), solid([60, 60, 60]));
        let out = run(K::LensFlareWipe, &a, &b, 0.5);
        assert!(out.chunks_exact(4).any(|p| p[0] > 120));
        // And only near the streak: the far edge is untouched.
        assert_eq!(&out[..3], &[60, 60, 60]);
    }
}
