//! The more transitions: 30 ways to turn one picture into the next (zooms, rotations, glitch,
//! camera shake, slides, pans, a film strip, ...). Each is the outgoing picture at progress 0 and
//! the incoming one at progress 1, whatever happens between.
//!
//! They are drawn per pixel on the processor, in bands of rows on every core. Positions are in
//! picture units: `u` runs 0 to 1 from left to right and `v` 0 to 1 from the top down. Pictures
//! are sampled with bilinear filtering, clamped at the edges, and mixed with premultiplied alpha.

use std::ops::{Add, Div, Mul, Sub};

use picasso_core::TransitionKind as K;

const PI: f32 = std::f32::consts::PI;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct V2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

pub(crate) const fn v2(x: f32, y: f32) -> V2 {
    V2 { x, y }
}

impl Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        v2(self.x + o.x, self.y + o.y)
    }
}

impl Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        v2(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for V2 {
    type Output = V2;
    fn mul(self, k: f32) -> V2 {
        v2(self.x * k, self.y * k)
    }
}

impl Mul<V2> for V2 {
    type Output = V2;
    fn mul(self, o: V2) -> V2 {
        v2(self.x * o.x, self.y * o.y)
    }
}

impl Div<f32> for V2 {
    type Output = V2;
    fn div(self, k: f32) -> V2 {
        v2(self.x / k, self.y / k)
    }
}

impl Div<V2> for V2 {
    type Output = V2;
    fn div(self, o: V2) -> V2 {
        v2(self.x / o.x, self.y / o.y)
    }
}

impl V2 {
    pub(crate) fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
    pub(crate) fn dot(self, o: V2) -> f32 {
        self.x * o.x + self.y * o.y
    }
}

/// A colour with premultiplied alpha.
pub(crate) type C4 = [f32; 4];

pub(crate) fn cmix(a: C4, b: C4, t: f32) -> C4 {
    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
}

pub(crate) fn cscale(a: C4, k: f32) -> C4 {
    a.map(|v| v * k)
}

pub(crate) fn cadd(a: C4, b: C4) -> C4 {
    [0, 1, 2, 3].map(|i| a[i] + b[i])
}

/// `top` over `bottom` (both premultiplied).
fn over(top: C4, bottom: C4) -> C4 {
    cadd(top, cscale(bottom, 1.0 - top[3]))
}

fn darken(c: C4, amount: f32) -> C4 {
    [
        c[0] * (1.0 - amount),
        c[1] * (1.0 - amount),
        c[2] * (1.0 - amount),
        c[3],
    ]
}

pub(crate) fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// GLSL `step`: 0 below the edge, 1 from it.
fn step(edge: f32, x: f32) -> f32 {
    if x < edge {
        0.0
    } else {
        1.0
    }
}

pub(crate) fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn hash2(p: V2) -> f32 {
    fract((p.x * 127.1 + p.y * 311.7).sin() * 43758.547)
}

fn vnoise(p: V2) -> f32 {
    let i = v2(p.x.floor(), p.y.floor());
    let f = v2(fract(p.x), fract(p.y));
    let f = v2(f.x * f.x * (3.0 - 2.0 * f.x), f.y * f.y * (3.0 - 2.0 * f.y));
    mix(
        mix(hash2(i), hash2(i + v2(1.0, 0.0)), f.x),
        mix(hash2(i + v2(0.0, 1.0)), hash2(i + v2(1.0, 1.0)), f.x),
        f.y,
    )
}

fn fbm(p: V2) -> f32 {
    0.5 * vnoise(p) + 0.3 * vnoise(p * 2.1) + 0.2 * vnoise(p * 4.3)
}

fn hash1(n: f32) -> f32 {
    fract((n * 127.1 + 311.7).sin() * 43758.547)
}

fn noise1(p: V2) -> f32 {
    let (ix, iy) = (p.x.floor(), p.y.floor());
    let f = v2(fract(p.x), fract(p.y));
    let f = v2(f.x * f.x * (3.0 - 2.0 * f.x), f.y * f.y * (3.0 - 2.0 * f.y));
    let a = hash1(ix + 57.0 * iy);
    let b = hash1(ix + 1.0 + 57.0 * iy);
    let c = hash1(ix + 57.0 * (iy + 1.0));
    let d = hash1(ix + 1.0 + 57.0 * (iy + 1.0));
    mix(mix(a, b, f.x), mix(c, d, f.x), f.y)
}

/// A picture to sample: RGBA8 with straight alpha.
pub(crate) struct Pic<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) w: usize,
    pub(crate) h: usize,
}

impl Pic<'_> {
    fn texel(&self, x: usize, y: usize) -> C4 {
        let i = (y * self.w + x) * 4;
        let a = f32::from(self.data[i + 3]) / 255.0;
        [
            f32::from(self.data[i]) / 255.0 * a,
            f32::from(self.data[i + 1]) / 255.0 * a,
            f32::from(self.data[i + 2]) / 255.0 * a,
            a,
        ]
    }

    /// Bilinear sample at (u, v), the position clamped into the picture.
    pub(crate) fn tex(&self, p: V2) -> C4 {
        let x = (p.x.clamp(0.0, 1.0) * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (p.y.clamp(0.0, 1.0) * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let top = cmix(self.texel(x0, y0), self.texel(x1, y0), fx);
        let bottom = cmix(self.texel(x0, y1), self.texel(x1, y1), fx);
        cmix(top, bottom, fy)
    }

    /// The same, transparent outside the picture.
    pub(crate) fn inside(&self, p: V2) -> C4 {
        if p.x < 0.0 || p.x > 1.0 || p.y < 0.0 || p.y > 1.0 {
            [0.0; 4]
        } else {
            self.tex(p)
        }
    }
}

/// A ball dropped on the floor: 0 to 1 with three bounces off 1.
fn bounce_out(mut t: f32) -> f32 {
    if t < 1.0 / 2.75 {
        return 7.5625 * t * t;
    }
    if t < 2.0 / 2.75 {
        t -= 1.5 / 2.75;
        return 7.5625 * t * t + 0.75;
    }
    if t < 2.5 / 2.75 {
        t -= 2.25 / 2.75;
        return 7.5625 * t * t + 0.9375;
    }
    t -= 2.625 / 2.75;
    7.5625 * t * t + 0.984375
}

/// Distance outside the unit square (negative inside).
fn outside(q: V2) -> f32 {
    (-q.x).max(q.x - 1.0).max((-q.y).max(q.y - 1.0))
}

/// The settings of one transition: what the shader called uniforms, at their defaults.
#[derive(Clone, Copy, Debug)]
struct Look {
    shape: Shape,
    /// Zoom: extra scale; rotate: the whole angle in radians.
    zoom: f32,
    strength: f32,
    speed: f32,
    /// Noise: cell size in picture heights; tunnel: glow radius.
    size: f32,
    soft: f32,
    glow: f32,
    count: i32,
    angle: f32,
    burn: f32,
    /// Slide, pan: the way the pictures travel; fall: the way the second one falls.
    dir: V2,
    blur: f32,
    color: C4,
    border: f32,
    flag: bool,
    pivot: V2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Flash,
    ZoomIn,
    ZoomInOut,
    CrashZoom,
    Rotate,
    NoiseDissolve,
    Glitch,
    CameraShake,
    Circles,
    Flip3d,
    SlicePush,
    Warp,
    DropWarp,
    Tunnel,
    BurnAway,
    Slide,
    Pan,
    FallAndBounce,
    FilmStrip,
    ForegroundWipe,
    PaintOn,
    RoundAndDown,
    BlurDissolve,
    AdditiveDissolve,
    NonAdditiveDissolve,
    SlideOut,
    BarnDoor,
    Split,
}

impl Look {
    fn new(shape: Shape) -> Self {
        Self {
            shape,
            zoom: 1.0,
            strength: 1.0,
            speed: 12.0,
            size: 0.1,
            soft: 0.2,
            glow: 1.0,
            count: 8,
            angle: 0.0,
            burn: 0.4,
            dir: v2(-1.0, 0.0),
            blur: 0.5,
            color: [0.0, 0.0, 0.0, 1.0],
            border: 0.0,
            flag: false,
            pivot: v2(0.0, 1.0),
        }
    }
}

const SLIDE_LEFT: V2 = v2(-1.0, 0.0);
const SLIDE_RIGHT: V2 = v2(1.0, 0.0);
const SLIDE_UP: V2 = v2(0.0, -1.0);
const SLIDE_DOWN: V2 = v2(0.0, 1.0);

/// The settings a transition starts with. `None` for the transitions that are not drawn here
/// (the cross dissolve and the dip to black, which the compositor mixes itself).
fn look(kind: K, width: usize, height: usize) -> Option<Look> {
    let l = Look::new;
    let rgba = |r: f32, g: f32, b: f32| [r / 255.0, g / 255.0, b / 255.0, 1.0];
    Some(match kind {
        // The dissolves are mixed by the compositor itself; the wipes are drawn in `wipes`.
        K::CrossDissolve | K::DipToBlack => return None,
        K::BrightnessFlash => Look {
            strength: 1.0,
            ..l(Shape::Flash)
        },
        K::ZoomIn => Look {
            zoom: 1.0,
            ..l(Shape::ZoomIn)
        },
        K::ZoomInOut => Look {
            zoom: 1.0,
            ..l(Shape::ZoomInOut)
        },
        K::CrashZoom => Look {
            zoom: 1.5,
            ..l(Shape::CrashZoom)
        },
        K::Rotate => Look {
            zoom: PI,
            ..l(Shape::Rotate)
        },
        K::Rotate90 => Look {
            zoom: PI / 2.0,
            ..l(Shape::Rotate)
        },
        K::NoiseDissolve => Look {
            size: 40.0 / height as f32,
            soft: 0.2,
            ..l(Shape::NoiseDissolve)
        },
        K::Glitch => Look {
            strength: 0.5,
            ..l(Shape::Glitch)
        },
        K::CameraShakeTransition => Look {
            speed: 12.0,
            strength: 0.3,
            ..l(Shape::CameraShake)
        },
        K::Circles => Look {
            count: 8,
            ..l(Shape::Circles)
        },
        K::Flip3d => l(Shape::Flip3d),
        K::SlicePush => Look {
            count: 6,
            ..l(Shape::SlicePush)
        },
        K::Warp => Look {
            strength: 0.5,
            ..l(Shape::Warp)
        },
        K::DropWarp => Look {
            strength: 0.5,
            ..l(Shape::DropWarp)
        },
        K::TunnelOfLight => Look {
            glow: 1.0,
            size: 0.2,
            ..l(Shape::Tunnel)
        },
        K::BurnAway => Look {
            angle: 0.0,
            strength: 0.5,
            glow: 1.0,
            burn: 0.4,
            ..l(Shape::BurnAway)
        },
        K::SlideLeft => Look {
            dir: SLIDE_LEFT,
            ..l(Shape::Slide)
        },
        K::SlideRight => Look {
            dir: SLIDE_RIGHT,
            ..l(Shape::Slide)
        },
        K::SlideUp => Look {
            dir: SLIDE_UP,
            ..l(Shape::Slide)
        },
        K::SlideDown => Look {
            dir: SLIDE_DOWN,
            ..l(Shape::Slide)
        },
        K::PanLeft => Look {
            dir: SLIDE_LEFT,
            blur: 0.5,
            ..l(Shape::Pan)
        },
        K::PanRight => Look {
            dir: SLIDE_RIGHT,
            blur: 0.5,
            ..l(Shape::Pan)
        },
        K::PanUp => Look {
            dir: SLIDE_UP,
            blur: 0.5,
            ..l(Shape::Pan)
        },
        K::PanDown => Look {
            dir: SLIDE_DOWN,
            blur: 0.5,
            ..l(Shape::Pan)
        },
        K::FallAndBounce => {
            let a = 90f32.to_radians();
            Look {
                dir: v2(a.cos(), a.sin()),
                ..l(Shape::FallAndBounce)
            }
        }
        K::FilmStrip => Look {
            color: rgba(28.0, 28.0, 32.0),
            zoom: 0.55,
            ..l(Shape::FilmStrip)
        },
        K::ForegroundWipe => Look {
            color: rgba(255.0, 255.0, 255.0),
            border: 4.0 / width as f32,
            soft: 0.5,
            flag: false,
            ..l(Shape::ForegroundWipe)
        },
        K::PaintOn => Look {
            count: 8,
            soft: 0.5,
            ..l(Shape::PaintOn)
        },
        K::RoundAndDown => Look {
            angle: 90f32.to_radians(),
            pivot: v2(0.0, 1.0),
            ..l(Shape::RoundAndDown)
        },
        K::BlurDissolve => Look {
            blur: 0.04,
            ..l(Shape::BlurDissolve)
        },
        K::SlideOutLeft => Look {
            dir: SLIDE_LEFT,
            ..l(Shape::SlideOut)
        },
        K::SlideOutRight => Look {
            dir: SLIDE_RIGHT,
            ..l(Shape::SlideOut)
        },
        K::SlideOutUp => Look {
            dir: SLIDE_UP,
            ..l(Shape::SlideOut)
        },
        K::SlideOutDown => Look {
            dir: SLIDE_DOWN,
            ..l(Shape::SlideOut)
        },
        // `dir` points along y for the horizontal presets; `flag` is the reverse.
        K::BarnDoorVertical => l(Shape::BarnDoor),
        K::BarnDoorHorizontal => Look {
            dir: v2(0.0, 1.0),
            ..l(Shape::BarnDoor)
        },
        K::BarnDoorVerticalReverse => Look {
            flag: true,
            ..l(Shape::BarnDoor)
        },
        K::BarnDoorHorizontalReverse => Look {
            dir: v2(0.0, 1.0),
            flag: true,
            ..l(Shape::BarnDoor)
        },
        K::SplitHorizontal => Look {
            dir: v2(0.0, 1.0),
            ..l(Shape::Split)
        },
        K::SplitVertical => l(Shape::Split),
        K::SplitHorizontalReverse => Look {
            dir: v2(0.0, 1.0),
            flag: true,
            ..l(Shape::Split)
        },
        K::SplitVerticalReverse => Look {
            flag: true,
            ..l(Shape::Split)
        },
        K::AdditiveDissolve => l(Shape::AdditiveDissolve),
        K::NonAdditiveDissolve => l(Shape::NonAdditiveDissolve),
        // The wipes are drawn in `wipes`.
        _ => return None,
    })
}

/// What a pixel needs to know about the frame.
struct Frame<'a> {
    a: Pic<'a>,
    b: Pic<'a>,
    aspect: f32,
    t: f32,
}

impl Frame<'_> {
    fn a(&self, p: V2) -> C4 {
        self.a.tex(p)
    }
    fn b(&self, p: V2) -> C4 {
        self.b.tex(p)
    }
    /// The first picture to the second at weight `w`, sampled at `p`.
    fn mix_ab(&self, p: V2, w: f32) -> C4 {
        cmix(self.a(p), self.b(p), w)
    }
    fn asp(&self) -> V2 {
        v2(self.aspect, 1.0)
    }
}

fn centre() -> V2 {
    v2(0.5, 0.5)
}

fn zoom_about(p: V2, s: f32) -> V2 {
    centre() + (p - centre()) / s
}

fn rotate_about(f: &Frame, p: V2, angle: f32) -> V2 {
    let q = (p - centre()) * f.asp();
    let (s, c) = angle.sin_cos();
    let q = v2(c * q.x + s * q.y, -s * q.x + c * q.y);
    q / f.asp() + centre()
}

/// The picture at `uv` for settings `l`.
fn shade(f: &Frame, l: &Look, uv: V2) -> C4 {
    let t = f.t;
    let bell = (PI * t).sin();
    let aspect = f.aspect;
    match l.shape {
        Shape::Flash => {
            let m = f.mix_ab(uv, smoothstep(0.3, 0.7, t));
            let flash = bell * bell * l.strength;
            [
                m[0] + flash * m[3],
                m[1] + flash * m[3],
                m[2] + flash * m[3],
                m[3],
            ]
        }
        Shape::ZoomIn => {
            let sa = 1.0 + l.zoom * t;
            let sb = (1.0 + l.zoom * t) / (1.0 + l.zoom);
            cmix(f.a(zoom_about(uv, sa)), f.b(zoom_about(uv, sb)), t)
        }
        Shape::ZoomInOut => {
            let sa = 1.0 + l.zoom * t;
            let sb = 1.0 + l.zoom * (1.0 - t);
            cmix(f.a(zoom_about(uv, sa)), f.b(zoom_about(uv, sb)), t)
        }
        Shape::CrashZoom => {
            // A fast zoom into the first picture, then a fast zoom out of the second.
            let first = t < 0.5;
            let q = if first { 2.0 * t } else { 2.0 * (1.0 - t) };
            let s = 1.0 + l.zoom * q * q * q;
            let mut sum = [0.0; 4];
            for i in 0..8 {
                let ss = 1.0 + (s - 1.0) * (1.0 - 0.25 * i as f32 / 7.0);
                let p = zoom_about(uv, ss);
                sum = cadd(sum, if first { f.a(p) } else { f.b(p) });
            }
            cscale(sum, 1.0 / 8.0)
        }
        Shape::Rotate => cmix(
            f.a(rotate_about(f, uv, l.zoom * t)),
            f.b(rotate_about(f, uv, l.zoom * (t - 1.0))),
            t,
        ),
        Shape::NoiseDissolve => {
            let n = vnoise(uv * f.asp() / l.size);
            let s = l.soft * 0.5 + 0.001;
            let th = mix(-s, 1.0 + s, t);
            let b = 1.0 - smoothstep(th - s, th + s, n);
            cmix(f.a(uv), f.b(uv), b)
        }
        Shape::Glitch => {
            let g = bell * l.strength;
            let row = (uv.y * 24.0).floor();
            let frame = (t * 14.0).floor();
            let shift =
                (hash2(v2(row, frame)) - 0.5) * 0.4 * g * step(0.55, hash2(v2(row + 7.0, frame)));
            let pick = step(hash2(v2(row, 3.7)), t);
            let split = g * 0.02;
            let p = v2(uv.x + shift, uv.y);
            let r = f.mix_ab(p + v2(split, 0.0), pick);
            let gg = f.mix_ab(p, pick);
            let bb = f.mix_ab(p - v2(split, 0.0), pick);
            [r[0], gg[1], bb[2], gg[3]]
        }
        Shape::CameraShake => {
            let sh = bell * l.strength * 0.05;
            let ph = t * l.speed * std::f32::consts::TAU;
            let off = v2((ph * 1.3).sin(), (ph * 1.7).cos()) * sh;
            let split = sh * 0.3;
            let w = smoothstep(0.3, 0.7, t);
            let r = f.mix_ab(uv + off + v2(split, 0.0), w);
            let gg = f.mix_ab(uv + off, w);
            let bb = f.mix_ab(uv + off - v2(split, 0.0), w);
            [r[0], gg[1], bb[2], gg[3]]
        }
        Shape::Circles => {
            let p = (uv - centre()) * f.asp();
            let r = p.length() / (0.5 * f.asp().length());
            let q = fract(r * l.count as f32);
            let w = 0.03;
            let e = t * (1.0 + w);
            let b = 1.0 - smoothstep(e - w, e, q);
            cmix(f.a(uv), f.b(uv), b)
        }
        Shape::Flip3d => {
            let phi = PI * t;
            let c = phi.cos();
            let x = uv.x - 0.5;
            let dx = c.abs().max(0.001);
            let side = if c >= 0.0 { 1.0 } else { -1.0 };
            let src_x = 0.5 + x / dx;
            let y_scale = 1.0 + 0.5 * phi.sin() * x * side;
            let src = v2(src_x, 0.5 + (uv.y - 0.5) * y_scale);
            if side > 0.0 {
                f.a.inside(src)
            } else {
                f.b.inside(src)
            }
        }
        Shape::SlicePush => {
            let n = l.count as f32;
            let i = (uv.x * n).floor();
            let p = (t * 1.6 - 0.6 * i / n).clamp(0.0, 1.0);
            let d = if i.rem_euclid(2.0) < 0.5 { 1.0 } else { -1.0 };
            cadd(
                f.a.inside(uv - v2(0.0, d * p)),
                f.b.inside(uv - v2(0.0, d * (p - 1.0))),
            )
        }
        Shape::Warp => {
            // A swirl that draws the picture in.
            let p = (uv - centre()) * f.asp();
            let r = p.length();
            let ang = l.strength * 6.0 * bell * (-r * 2.5).exp();
            let (s, c) = ang.sin_cos();
            let p = v2(c * p.x - s * p.y, s * p.x + c * p.y) * (1.0 - 0.3 * bell * l.strength);
            f.mix_ab(centre() + p / f.asp(), smoothstep(0.35, 0.65, t))
        }
        Shape::DropWarp => {
            let p = (uv - centre()) * f.asp();
            let r = p.length();
            let rmax = 0.5 * f.asp().length();
            let w = 0.03;
            let front = t * (rmax + w);
            let d = r - front;
            let wave = (d * 40.0).sin() * (-d * d * 60.0).exp() * l.strength * 0.04 * bell;
            let n = p + v2(1e-5, 1e-5);
            let uv2 = uv + (n / n.length()) * wave / f.asp();
            let b = 1.0 - smoothstep(front - w, front, r);
            cmix(f.a(uv2), f.b(uv2), b)
        }
        Shape::Tunnel => {
            let p = (uv - centre()) * f.asp();
            let r = p.length();
            let a =
                f.a.inside(centre() + (uv - centre()) / (1.0 - t).max(0.001));
            let b = f.b.inside(centre() + (uv - centre()) / t.max(0.001));
            let glow = (-(r / l.size).powi(2)).exp() * bell * l.glow;
            cadd(over(b, a), [1.0 * glow, 0.95 * glow, 0.8 * glow, glow])
        }
        Shape::BurnAway => {
            let dir = v2(l.angle.cos(), l.angle.sin());
            let w = (uv - centre()) * f.asp();
            let extent = 0.5 * (dir.x.abs() * aspect + dir.y.abs());
            let mut s = 0.5 + w.dot(dir) / (2.0 * extent);
            s += (fbm(uv * f.asp() * 4.0) - 0.5) * 0.8 * l.strength;
            let char_w = 0.02 + 0.2 * l.burn;
            let edge = mix(-0.6, 1.6 + char_w, t);
            let alive = smoothstep(edge - 0.02, edge, s);
            let char_mask = smoothstep(edge - char_w - 0.02, edge - char_w, s);
            let a = f.a(uv);
            let burnt = [
                mix(a[0] * 0.12, a[0], alive),
                mix(a[1] * 0.12, a[1], alive),
                mix(a[2] * 0.12, a[2], alive),
                a[3],
            ];
            let m = cmix(f.b(uv), burnt, char_mask);
            let glow = (-((s - edge) / 0.03).powi(2)).exp() * l.glow;
            let k = glow * m[3].max(0.5);
            [m[0] + 1.0 * k, m[1] + 0.45 * k, m[2] + 0.1 * k, m[3]]
        }
        Shape::Slide => {
            let e = t;
            let q = uv - l.dir * (e - 1.0);
            let d = outside(q);
            // A soft shadow of the sliding picture on the one beneath, gone at both ends.
            let shadow = if d > 0.0 {
                0.45 * bell * (1.0 - smoothstep(0.0, 0.1, d))
            } else {
                0.0
            };
            over(f.b.inside(q), darken(f.a.tex(uv), shadow))
        }
        Shape::Pan => {
            let e = t;
            let len = l.blur * 0.3 * bell;
            let mut sum = [0.0; 4];
            for i in 0..8 {
                let s = e + (i as f32 / 7.0 - 0.5) * len;
                sum = cadd(
                    sum,
                    cadd(
                        f.a.inside(uv - l.dir * s),
                        f.b.inside(uv - l.dir * (s - 1.0)),
                    ),
                );
            }
            cscale(sum, 1.0 / 8.0)
        }
        Shape::FallAndBounce => {
            let q = uv + l.dir * (1.0 - bounce_out(t));
            let d = outside(q);
            let shadow = if d > 0.0 {
                0.5 * bell * (1.0 - smoothstep(0.0, 0.08, d))
            } else {
                0.0
            };
            over(f.b.inside(q), darken(f.a.tex(uv), shadow))
        }
        Shape::FilmStrip => {
            let margin = 0.12; // film edge with the holes, in frame widths
            let gap = 0.15; // space between two frames, in frame heights
            let pitch = 1.0 + gap;
            let z = mix(1.0, l.zoom, bell);
            let u = (uv.x - 0.5) / z + 0.5;
            let yy = (uv.y - 0.5) / z + t * pitch;
            if !(u > -margin && u < 1.0 + margin) {
                return [
                    l.color[0] * 0.35,
                    l.color[1] * 0.35,
                    l.color[2] * 0.35,
                    l.color[3] * 0.35 + 0.65,
                ];
            }
            if (0.0..=1.0).contains(&u) {
                if yy.abs() <= 0.5 {
                    return f.a.tex(v2(u, yy + 0.5));
                }
                if (yy - pitch).abs() <= 0.5 {
                    return f.b.tex(v2(u, yy - pitch + 0.5));
                }
            }
            // Film base, with a row of holes along both edges.
            let side = if u < 0.5 {
                -0.5 * margin
            } else {
                1.0 + 0.5 * margin
            };
            let hole = step((u - side).abs(), 0.035)
                * step(0.3, fract(yy * 8.0))
                * step(fract(yy * 8.0), 0.7);
            // step(edge, x) with edge = |u - side|: 1 when the distance is under 0.035.
            cmix(l.color, [0.92, 0.92, 0.92, 1.0], hole)
        }
        Shape::ForegroundWipe => {
            let half_width = 0.14;
            let zoomed = 1.35;
            let ss = 0.02 + 0.2 * l.soft;
            let reach = half_width + l.border + ss + 0.01;
            let x = if l.flag { 1.0 - uv.x } else { uv.x };
            let c = mix(-reach, 1.0 + reach, t);
            let d = (x - c).abs() - half_width;
            if d < 0.0 {
                // The column: the incoming picture, magnified about the column's centre.
                let sx = c + (x - c) / zoomed;
                return f.b.tex(v2(
                    if l.flag { 1.0 - sx } else { sx },
                    0.5 + (uv.y - 0.5) / zoomed,
                ));
            }
            if d < l.border {
                return l.color;
            }
            let base = if x < c { f.b.tex(uv) } else { f.a.tex(uv) };
            darken(base, 0.5 * (1.0 - smoothstep(0.0, ss, d - l.border)))
        }
        Shape::PaintOn => {
            let n = l.count as f32;
            let row = (uv.y * n).floor();
            let fy = fract(uv.y * n);
            // Every stroke starts at its own time; all are finished at t = 1.
            let p = ((t - 0.5 * hash1(row)) / 0.5).clamp(0.0, 1.0);
            let head = p * 1.25 - 0.1;
            let rounded = 0.06 * (1.0 - (PI * fy).sin().max(0.0).sqrt());
            let edge_x = head - rounded + 0.015 * (noise1(v2(uv.y * 40.0, row)) - 0.5);
            let painted = 1.0 - smoothstep(edge_x - 0.004, edge_x + 0.004, uv.x);
            let shadow = l.soft * 0.5 * (1.0 - smoothstep(0.0, 0.05, uv.x - edge_x));
            cmix(darken(f.a.tex(uv), shadow), f.b.tex(uv), painted)
        }
        Shape::SlideOut => {
            // The outgoing picture slides away, uncovering the incoming one.
            let q = uv - l.dir * t;
            over(f.a.inside(q), f.b.tex(uv))
        }
        Shape::BarnDoor => {
            // The picture opens like a barn door from the middle (or closes, reversed): its two
            // halves move apart along the axis and nothing is left in the gap.
            let horizontal = l.dir.y.abs() > 0.5;
            let c = if horizontal { uv.y } else { uv.x };
            let h = 0.5 * if l.flag { 1.0 - t } else { t };
            let halves = |pic: &Pic| -> C4 {
                let moved = if c <= 0.5 - h {
                    c + h
                } else if c >= 0.5 + h {
                    c - h
                } else {
                    return [0.0; 4];
                };
                pic.tex(if horizontal {
                    v2(uv.x, moved)
                } else {
                    v2(moved, uv.y)
                })
            };
            if l.flag {
                over(halves(&f.b), f.a.tex(uv))
            } else {
                over(halves(&f.a), f.b.tex(uv))
            }
        }
        Shape::Split => {
            // The two halves of the screen push away from each other, the next picture coming
            // in behind each.
            let horizontal = l.dir.y.abs() > 0.5;
            let first = if horizontal { uv.y } else { uv.x } < 0.5;
            let along = if horizontal {
                v2(1.0, 0.0)
            } else {
                v2(0.0, 1.0)
            };
            let sign = (if first { -1.0 } else { 1.0 }) * if l.flag { -1.0 } else { 1.0 };
            let d = along * sign;
            cadd(f.a.inside(uv - d * t), f.b.inside(uv - d * (t - 1.0)))
        }
        Shape::AdditiveDissolve | Shape::NonAdditiveDissolve => {
            // Each picture at full strength from its half on: in the middle they add up (or the
            // brighter wins).
            let a = cscale(f.a.tex(uv), (2.0 * (1.0 - t)).min(1.0));
            let b = cscale(f.b.tex(uv), (2.0 * t).min(1.0));
            if matches!(l.shape, Shape::AdditiveDissolve) {
                [0, 1, 2, 3].map(|i| (a[i] + b[i]).min(1.0))
            } else {
                [0, 1, 2, 3].map(|i| a[i].max(b[i]))
            }
        }
        Shape::BlurDissolve => {
            // Both pictures are blurred by a disc that is widest in the middle, and mix.
            let radius = bell * l.blur;
            let (mut a, mut b) = ([0.0f32; 4], [0.0f32; 4]);
            for i in 0..32 {
                let r = ((i as f32 + 0.5) / 32.0).sqrt();
                let ang = i as f32 * 2.399_963;
                let p = uv + v2(ang.cos(), ang.sin()) * (r * radius);
                a = cadd(a, f.a(p));
                b = cadd(b, f.b(p));
            }
            cmix(cscale(a, 1.0 / 32.0), cscale(b, 1.0 / 32.0), t)
        }
        Shape::RoundAndDown => {
            let e = t;
            let d = (uv - l.pivot) * f.asp();
            let a = l.angle * e; // the picture turns clockwise about the pivot
            let (s, c) = a.sin_cos();
            let src = v2(c * d.x + s * d.y, -s * d.x + c * d.y);
            // The swinging picture fades out over the full incoming picture.
            over(
                cscale(f.a.inside(l.pivot + src / f.asp()), 1.0 - t),
                f.b.tex(uv),
            )
        }
    }
}

/// Whether `kind` is drawn by [`mix`]; the cross dissolve and the dip to black are mixed by the
/// compositor itself.
pub fn is_drawn_here(kind: K) -> bool {
    look(kind, 1, 1).is_some() || crate::wipes::handles(kind) || crate::smooth_cut::handles(kind)
}

/// Makes `second` (the incoming picture, `width` x `height` RGBA8) the picture `t` (0 to 1) of
/// the way through the transition `kind` from `first` (the outgoing one). Does nothing for kinds
/// that are not drawn here, or when the pictures do not fit the size.
pub fn mix_into(kind: K, first: &[u8], second: &mut [u8], width: usize, height: usize, t: f32) {
    if crate::wipes::mix_into(kind, first, second, width, height, t)
        || crate::smooth_cut::mix_into(kind, first, second, width, height, t)
    {
        return;
    }
    let Some(l) = look(kind, width.max(1), height.max(1)) else {
        return;
    };
    let Some(len) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
        return;
    };
    if width == 0 || height == 0 || first.len() < len || second.len() < len {
        return;
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
        t: t.clamp(0.0, 1.0),
    };
    let (frame, l) = (&frame, &l);
    picasso_color::par::par_rows(&mut second[..len], width * 4, height, &|first_row, out| {
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let n = first_row * width + i;
            let (x, y) = (n % width, n / width);
            let uv = v2(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let c = shade(frame, l, uv);
            let a = c[3].clamp(0.0, 1.0);
            let k = if a > 1e-6 { 1.0 / a } else { 0.0 };
            for ch in 0..3 {
                px[ch] = (c[ch] * k * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
            px[3] = (a * 255.0 + 0.5) as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 48;
    const H: usize = 27;

    /// A picture with a different colour in every area, so a wrong move shows.
    fn picture(seed: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                v.extend_from_slice(&[
                    (x * 5) as u8 ^ seed,
                    (y * 9) as u8,
                    ((x + y) * 3) as u8 ^ seed.wrapping_mul(7),
                    255,
                ]);
            }
        }
        v
    }

    fn run(kind: K, t: f32) -> Vec<u8> {
        let (a, mut b) = (picture(0), picture(90));
        mix_into(kind, &a, &mut b, W, H, t);
        b
    }

    fn worst(a: &[u8], b: &[u8]) -> u8 {
        a.iter()
            .zip(b)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn slide_out_takes_the_first_picture_away_and_a_barn_door_opens_from_the_middle() {
        let (a, b) = (picture(0), picture(90));
        let px = |p: &[u8], x: usize, y: usize| p[(y * W + x) * 4..(y * W + x) * 4 + 3].to_vec();
        // Slide out to the left: the left half of the frame shows the right half of the first
        // picture, the right half the second picture.
        let out = run(K::SlideOutLeft, 0.5);
        assert_eq!(px(&out, 0, 5), px(&a, W / 2, 5));
        assert_eq!(px(&out, W - 1, 5), px(&b, W - 1, 5));
        // A barn door opens: the middle already shows the second picture, the edges the first.
        let door = run(K::BarnDoorVertical, 0.5);
        assert_eq!(px(&door, W / 2, 5), px(&b, W / 2, 5));
        assert_eq!(px(&door, 0, 5), px(&a, W / 4, 5));
        // Reversed, it closes: it starts open and ends shut over the first picture.
        let closed = run(K::BarnDoorVerticalReverse, 0.9);
        assert_ne!(px(&closed, W / 2, 5), px(&b, W / 2, 5));
        // A split sends the halves apart: the top left shows what came from the left.
        let split = run(K::SplitVertical, 0.5);
        assert_ne!(split, a);
        assert_ne!(split, b);
    }

    #[test]
    fn additive_adds_the_pictures_and_non_additive_keeps_the_brighter() {
        let (a, b) = (picture(0), picture(90));
        let add = run(K::AdditiveDissolve, 0.5);
        let max = run(K::NonAdditiveDissolve, 0.5);
        for i in 0..a.len() / 4 {
            for c in 0..3 {
                let j = i * 4 + c;
                let sum = (u16::from(a[j]) + u16::from(b[j])).min(255);
                assert!(add[j].abs_diff(sum as u8) <= 2, "additive at {j}");
                assert!(max[j].abs_diff(a[j].max(b[j])) <= 2, "non-additive at {j}");
            }
        }
    }

    #[test]
    fn every_extra_transition_starts_as_the_first_picture_and_ends_as_the_second() {
        let (a, b) = (picture(0), picture(90));
        let mut checked = 0;
        for kind in K::extra().iter().copied() {
            if !is_drawn_here(kind) {
                continue;
            }
            checked += 1;
            // Tunnel of light and the rotations resample, so a few levels of difference are fine.
            let start = worst(&run(kind, 0.0), &a);
            let end = worst(&run(kind, 1.0), &b);
            assert!(
                start <= 12,
                "{kind:?} does not start as the first picture ({start})"
            );
            assert!(
                end <= 12,
                "{kind:?} does not end as the second picture ({end})"
            );
        }
        assert_eq!(checked, 45);
    }

    #[test]
    fn the_middle_of_each_one_is_neither_picture_and_is_a_whole_picture() {
        let (a, b) = (picture(0), picture(90));
        for kind in K::extra().iter().copied() {
            if !is_drawn_here(kind) {
                continue;
            }
            let mid = run(kind, 0.5);
            assert_eq!(mid.len(), a.len());
            assert!(
                worst(&mid, &a) > 12 || worst(&mid, &b) > 12,
                "{kind:?} shows nothing in the middle"
            );
        }
    }

    #[test]
    fn a_slide_left_brings_the_second_picture_in_from_the_right() {
        let b = picture(90);
        let mid = run(K::SlideLeft, 0.5);
        // Halfway: the right half of the frame shows the left half of the second picture.
        let px = |p: &[u8], x: usize, y: usize| p[(y * W + x) * 4..(y * W + x) * 4 + 3].to_vec();
        let (y, x) = (H / 2, W * 3 / 4);
        let want = px(&b, x - W / 2, y);
        let got = px(&mid, x, y);
        assert!(
            want.iter().zip(&got).all(|(a, b)| a.abs_diff(*b) <= 12),
            "{want:?} vs {got:?}"
        );
    }

    #[test]
    fn transparent_pictures_stay_transparent_where_neither_shows() {
        let mut a = picture(0);
        let mut b = picture(90);
        for p in a.chunks_exact_mut(4).chain(b.chunks_exact_mut(4)) {
            p[3] = 0;
        }
        mix_into(K::ZoomIn, &a.clone(), &mut b, W, H, 0.5);
        assert!(b.chunks_exact(4).all(|p| p[3] == 0));
    }

    #[test]
    fn odd_sizes_and_short_buffers_do_not_crash() {
        let mut one = vec![10, 20, 30, 255];
        mix_into(K::Glitch, &[1, 2, 3, 255], &mut one, 1, 1, 0.5);
        mix_into(K::Flip3d, &[], &mut [], 0, 0, 0.5);
        let mut short = vec![0u8; 8];
        mix_into(K::Warp, &[0u8; 8], &mut short, 4, 4, 0.5);
        assert_eq!(short, vec![0u8; 8]);
        for t in [-1.0, 2.0, f32::NAN] {
            let _ = run(K::TunnelOfLight, t);
        }
    }

    /// Not a check, a measurement: `cargo test -p picasso-render --release timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_of_each_at_1080p() {
        let (w, h) = (1920, 1080);
        let a = vec![120u8; w * h * 4];
        for kind in K::extra() {
            let mut b = vec![200u8; w * h * 4];
            let start = std::time::Instant::now();
            mix_into(*kind, &a, &mut b, w, h, 0.5);
            eprintln!(
                "{:>18}: {:>4} ms",
                kind.label(),
                start.elapsed().as_millis()
            );
        }
    }

    #[test]
    fn the_cross_dissolve_and_the_dip_are_left_to_the_compositor() {
        assert!(!is_drawn_here(K::CrossDissolve) && !is_drawn_here(K::DipToBlack));
        let b = picture(90);
        assert_eq!(run(K::CrossDissolve, 0.5), b);
    }
}
