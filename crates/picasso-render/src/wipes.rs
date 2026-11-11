//! The wipes, irises and shapes: a moving edge (or a growing shape) shows the second picture.
//!
//! Every one of them gives each pixel an "arrival" `s` from 0 to 1: how late the moving edge
//! reaches it (0 where the edge starts, 1 for the last pixels). The edge travels from 0 to
//! `1 + border + feather` as the transition runs:
//!
//! * `s >= edge`: the outgoing picture;
//! * `edge - border <= s < edge`: the border colour;
//! * `s < edge - border`: the incoming picture,
//!
//! with the feather softening both boundaries. Only the arrival field differs between them. For
//! the irises and shapes it is a "gauge" of the shape: the factor by which the shape (centred,
//! turned, stretched) has to grow to touch the pixel, divided by the factor that reaches the
//! farthest corner, so the shape itself grows with the edge.
//!
//! Drawn per pixel on the processor, in bands of rows on every core.

use picasso_core::TransitionKind as K;

use crate::transitions::{cadd, cmix, cscale, fract, smoothstep, v2, Pic, C4, V2};

const PI: f32 = std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    /// A plain wipe: an edge moving in a direction.
    Wipe,
    /// The second picture pushes the first out.
    Push,
    Polygon,
    Oval,
    Cross,
    Eye,
    Arrow,
    Heart,
    Star,
    Box,
    TriangleSide,
    Band,
    CenterWipe,
    Clock,
    Radial,
    Spiral,
    Venetian,
    X,
}

/// The settings of one wipe, at their usual values.
#[derive(Clone, Copy, Debug)]
struct Spec {
    field: Field,
    /// Sides of a polygon, points of a star, bands or slats.
    n: i32,
    /// Plain wipe and push: the way the edge travels. Triangle side: from the right edge.
    dir: V2,
    flag: bool,
    flag2: bool,
    /// Box: 0, 0.5 or 1 per axis.
    anchor: V2,
    /// Clockwise rotation of the shape, radians.
    rot: f32,
    shape_aspect: f32,
    angle: f32,
    /// Softness of a plain wipe, 0 to 1.
    softness: f32,
}

impl Spec {
    fn new(field: Field) -> Self {
        Self {
            field,
            n: 4,
            dir: v2(-1.0, 0.0),
            flag: false,
            flag2: false,
            anchor: v2(0.0, 0.0),
            rot: 0.0,
            shape_aspect: 1.0,
            angle: 0.0,
            softness: 0.1,
        }
    }
}

const LEFT: V2 = v2(-1.0, 0.0);
const RIGHT: V2 = v2(1.0, 0.0);
const UP: V2 = v2(0.0, -1.0);
const DOWN: V2 = v2(0.0, 1.0);

fn spec(kind: K) -> Option<Spec> {
    let s = Spec::new;
    let deg = f32::to_radians;
    let box_at = |x, y| Spec {
        anchor: v2(x, y),
        ..s(Field::Box)
    };
    let band = |vertical, bilinear| Spec {
        n: 8,
        flag: vertical,
        flag2: bilinear,
        ..s(Field::Band)
    };
    Some(match kind {
        K::WipeLeft => Spec {
            dir: LEFT,
            ..s(Field::Wipe)
        },
        K::WipeRight => Spec {
            dir: RIGHT,
            ..s(Field::Wipe)
        },
        K::WipeUp => Spec {
            dir: UP,
            ..s(Field::Wipe)
        },
        K::WipeDown => Spec {
            dir: DOWN,
            ..s(Field::Wipe)
        },
        K::PushLeft => Spec {
            dir: LEFT,
            ..s(Field::Push)
        },
        K::PushRight => Spec {
            dir: RIGHT,
            ..s(Field::Push)
        },
        K::PushUp => Spec {
            dir: UP,
            ..s(Field::Push)
        },
        K::PushDown => Spec {
            dir: DOWN,
            ..s(Field::Push)
        },
        K::BandWipe => band(false, false),
        K::BandWipeVertical => band(true, false),
        K::BandWipeBilinear => band(false, true),
        K::BandWipeVerticalBilinear => band(true, true),
        K::CenterWipe => s(Field::CenterWipe),
        K::ClockWipe => Spec {
            flag: true,
            ..s(Field::Clock)
        },
        K::RadialWipe => s(Field::Radial),
        K::SpiralWipe => s(Field::Spiral),
        K::VenetianBlindWipe => Spec {
            n: 8,
            angle: deg(90.0),
            ..s(Field::Venetian)
        },
        K::XWipe => s(Field::X),
        K::ArrowIris => s(Field::Arrow),
        K::CrossIris => s(Field::Cross),
        K::DiamondIris => Spec {
            n: 4,
            ..s(Field::Polygon)
        },
        K::EyeIris => s(Field::Eye),
        K::HexagonIris => Spec {
            n: 6,
            rot: deg(30.0),
            ..s(Field::Polygon)
        },
        K::OvalIris => Spec {
            shape_aspect: 1.5,
            ..s(Field::Oval)
        },
        K::PentagonIris => Spec {
            n: 5,
            ..s(Field::Polygon)
        },
        K::SquareIris => Spec {
            n: 4,
            rot: deg(45.0),
            ..s(Field::Polygon)
        },
        K::TriangleIris => Spec {
            n: 3,
            ..s(Field::Polygon)
        },
        K::ArrowIrisDown => Spec {
            rot: deg(180.0),
            ..s(Field::Arrow)
        },
        K::ArrowIrisLeft => Spec {
            rot: deg(270.0),
            ..s(Field::Arrow)
        },
        K::ArrowIrisRight => Spec {
            rot: deg(90.0),
            ..s(Field::Arrow)
        },
        K::HexagonIrisVertical => Spec {
            n: 6,
            ..s(Field::Polygon)
        },
        K::OvalIrisTall => Spec {
            shape_aspect: 1.0 / 1.5,
            ..s(Field::Oval)
        },
        K::PentagonIrisDown => Spec {
            n: 5,
            rot: deg(180.0),
            ..s(Field::Polygon)
        },
        K::TriangleIrisDown => Spec {
            n: 3,
            rot: deg(180.0),
            ..s(Field::Polygon)
        },
        K::TriangleIrisLeft => Spec {
            n: 3,
            rot: deg(270.0),
            ..s(Field::Polygon)
        },
        K::TriangleIrisRight => Spec {
            n: 3,
            rot: deg(90.0),
            ..s(Field::Polygon)
        },
        K::BoxUpperLeft => box_at(0.0, 0.0),
        K::BoxUpperRight => box_at(1.0, 0.0),
        K::BoxLowerLeft => box_at(0.0, 1.0),
        K::BoxLowerRight => box_at(1.0, 1.0),
        K::BoxLeftCenter => box_at(0.0, 0.5),
        K::BoxTopCenter => box_at(0.5, 0.0),
        K::BoxRightCenter => box_at(1.0, 0.5),
        K::BoxBottomCenter => box_at(0.5, 1.0),
        K::HeartShape => s(Field::Heart),
        K::StarShape => Spec {
            n: 5,
            ..s(Field::Star)
        },
        K::StarShape4 => Spec {
            n: 4,
            ..s(Field::Star)
        },
        K::StarShape6 => Spec {
            n: 6,
            ..s(Field::Star)
        },
        K::TriangleLeftShape => Spec {
            flag: true,
            ..s(Field::TriangleSide)
        },
        K::TriangleRightShape => s(Field::TriangleSide),
        _ => return None,
    })
}

/// Whether `kind` is drawn here.
pub(crate) fn handles(kind: K) -> bool {
    spec(kind).is_some()
}

fn polygon_gauge(q: V2, n: i32) -> f32 {
    let step = 2.0 * PI / n as f32;
    let mut g = -1e9f32;
    for i in 0..n.min(8) {
        // face normals, vertex 0 straight up
        let a = -0.5 * PI + (i as f32 + 0.5) * step;
        g = g.max(q.dot(v2(a.cos(), a.sin())));
    }
    g
}

fn arrow_gauge(q: V2) -> f32 {
    // Head: triangle (0,-1) (-0.7,0.1) (0.7,0.1); shaft: |x| <= 0.3, -0.5 <= y <= 1. A union of
    // convex shapes that contain the centre has the smaller of their gauges.
    let n = v2(0.8437, 0.5369);
    let head = (q.dot(v2(-n.x, -n.y)).max(q.dot(v2(n.x, -n.y))) / 0.5369).max(q.y / 0.1);
    let shaft = (q.x.abs() / 0.3).max(q.y).max(-q.y / 0.5);
    head.min(shaft)
}

fn inside_heart(q: V2, r: f32) -> bool {
    let x = q.x / r;
    let y = -q.y / r; // the classic curve has y up
    let a = x * x + y * y - 1.0;
    a * a * a - x * x * y * y * y <= 0.0
}

fn heart_gauge(q: V2) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 20.0f32);
    for _ in 0..16 {
        let mid = 0.5 * (lo + hi);
        if inside_heart(q, mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

fn star_gauge(q: V2, n: i32) -> f32 {
    let sector = 2.0 * PI / n as f32;
    let theta = q.x.atan2(-q.y);
    // angle to the nearest tip
    let shifted = theta + 0.5 * sector;
    let a = ((shifted - sector * (shifted / sector).floor()) - 0.5 * sector).abs();
    let inner = match n {
        4 => 0.45,
        5 => 0.4,
        _ => 0.55,
    };
    let tip = v2(1.0, 0.0);
    let valley = v2((0.5 * sector).cos(), (0.5 * sector).sin()) * inner;
    let d = v2(a.cos(), a.sin());
    let e = valley - tip;
    let radius = (tip.x * valley.y - tip.y * valley.x) / (d.x * e.y - d.y * e.x);
    q.length() / radius
}

struct Wipe {
    spec: Spec,
    aspect: f32,
    /// The gauge of the farthest corner or edge middle (for the shapes that grow).
    farthest: f32,
    t: f32,
}

impl Wipe {
    fn shape_gauge(&self, q: V2) -> f32 {
        match self.spec.field {
            Field::Polygon => polygon_gauge(q, self.spec.n),
            Field::Oval => q.length(),
            Field::Cross => (4.0 * q.x.abs())
                .max(q.y.abs())
                .min(q.x.abs().max(4.0 * q.y.abs())),
            Field::Eye => 1.5 * q.y.abs() + q.length(),
            Field::Arrow => arrow_gauge(q),
            Field::Heart => heart_gauge(q),
            _ => star_gauge(q, self.spec.n),
        }
    }

    /// The point in the shape's own frame.
    fn shape_frame(&self, uv: V2) -> V2 {
        let p = (uv - v2(0.5, 0.5)) * v2(self.aspect, 1.0);
        let (s, c) = self.spec.rot.sin_cos();
        let q = v2(c * p.x + s * p.y, -s * p.x + c * p.y);
        v2(q.x / self.spec.shape_aspect, q.y)
    }

    fn is_growing_shape(&self) -> bool {
        matches!(
            self.spec.field,
            Field::Polygon
                | Field::Oval
                | Field::Cross
                | Field::Eye
                | Field::Arrow
                | Field::Heart
                | Field::Star
        )
    }

    fn farthest_of(&self) -> f32 {
        let mut farthest = 0.0f32;
        for i in 0..8 {
            let c = if i < 4 {
                v2(((i & 1) as f32 - 0.5) * 2.0, ((i >> 1) as f32 - 0.5) * 2.0)
            } else {
                [v2(0.0, 1.0), v2(0.0, -1.0), v2(1.0, 0.0), v2(-1.0, 0.0)][i - 4]
            };
            farthest = farthest.max(self.shape_gauge(self.shape_frame(v2(0.5, 0.5) + c * 0.5)));
        }
        farthest.max(1e-6)
    }

    /// How late the edge reaches `uv`.
    fn arrival(&self, uv: V2) -> f32 {
        let sp = &self.spec;
        let p = (uv - v2(0.5, 0.5)) * v2(self.aspect, 1.0);
        if self.is_growing_shape() {
            return self.shape_gauge(self.shape_frame(uv)) / self.farthest;
        }
        match sp.field {
            Field::Box => {
                let d = v2((uv.x - sp.anchor.x).abs(), (uv.y - sp.anchor.y).abs());
                let m = v2(
                    sp.anchor.x.max(1.0 - sp.anchor.x),
                    sp.anchor.y.max(1.0 - sp.anchor.y),
                );
                (d.x / m.x).max(d.y / m.y)
            }
            Field::TriangleSide => {
                let u = if sp.flag { 1.0 - uv.x } else { uv.x };
                (u + 2.0 * (uv.y - 0.5).abs()) * 0.5
            }
            Field::Band => {
                // Bands across the frame (rows moving sideways, or columns moving up / down).
                let (along, across) = if sp.flag { (uv.y, uv.x) } else { (uv.x, uv.y) };
                let band = (across * sp.n as f32).floor();
                let odd = band.rem_euclid(2.0) > 0.5;
                if sp.flag2 {
                    let centred = (along - 0.5).abs() * 2.0;
                    if odd {
                        1.0 - centred
                    } else {
                        centred
                    }
                } else if odd {
                    1.0 - along
                } else {
                    along
                }
            }
            Field::CenterWipe => {
                let dir = v2(sp.angle.cos(), sp.angle.sin());
                let extent = 0.5 * (dir.x.abs() * self.aspect + dir.y.abs());
                p.dot(dir).abs() / extent
            }
            Field::Clock => {
                let theta = p.x.atan2(-p.y) - sp.angle; // clockwise from straight up
                let s = fract(theta / (2.0 * PI));
                if sp.flag {
                    s
                } else {
                    1.0 - s
                }
            }
            Field::Radial => {
                // from the bottom centre, y up
                let d = v2((uv.x - 0.5) * self.aspect, 1.0 - uv.y);
                1.0 - d.y.atan2(d.x) / PI
            }
            Field::Spiral => {
                let rmax = 0.5 * v2(self.aspect, 1.0).length();
                let turn = fract(p.x.atan2(-p.y) / (2.0 * PI));
                (turn + 2.0 * p.length() / rmax) / 3.0
            }
            Field::Venetian => {
                let dir = v2(sp.angle.cos(), sp.angle.sin());
                fract(p.dot(dir) * sp.n as f32)
            }
            _ => {
                // X wipe: opens along the two diagonals.
                let d = v2((uv.x - 0.5).abs() * 2.0, (uv.y - 0.5).abs() * 2.0);
                (d.x - d.y).abs()
            }
        }
    }
}

/// The picture at `uv` for the wipe `w`, the pictures `a` (outgoing) and `b` (incoming).
fn shade(w: &Wipe, a: &Pic, b: &Pic, uv: V2) -> C4 {
    let sp = &w.spec;
    let t = w.t;
    match sp.field {
        Field::Push => {
            // A moves out by dir * t, B follows from the opposite side.
            cadd(a.inside(uv - sp.dir * t), b.inside(uv - sp.dir * (t - 1.0)))
        }
        Field::Wipe => {
            let p = (uv - v2(0.5, 0.5)) * v2(w.aspect, 1.0);
            let extent = 0.5 * (sp.dir.x.abs() * w.aspect + sp.dir.y.abs());
            let s = 0.5 + p.dot(sp.dir) / (2.0 * extent);
            let soft = sp.softness.max(1e-4);
            let edge = t * (1.0 + soft);
            let incoming = 1.0 - smoothstep(edge - soft, edge, s);
            cmix(a.tex(uv), b.tex(uv), incoming)
        }
        _ => {
            let s = w.arrival(uv).clamp(0.0, 1.0);
            // a small feather keeps the edge from being jagged
            let feather = 0.003f32;
            let edge = t * (1.0 + feather);
            let outgoing = smoothstep(edge - feather, edge, s);
            let incoming = 1.0 - outgoing;
            cadd(cscale(a.tex(uv), outgoing), cscale(b.tex(uv), incoming))
        }
    }
}

/// Makes `second` (the incoming picture, `width` x `height` RGBA8) the picture `t` (0 to 1) of
/// the way through the wipe `kind` from `first`. Returns false, leaving `second` alone, when
/// `kind` is not drawn here or the pictures do not fit the size.
pub(crate) fn mix_into(
    kind: K,
    first: &[u8],
    second: &mut [u8],
    width: usize,
    height: usize,
    t: f32,
) -> bool {
    let Some(sp) = spec(kind) else {
        return false;
    };
    let Some(len) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
        return true;
    };
    if width == 0 || height == 0 || first.len() < len || second.len() < len {
        return true;
    }
    let incoming = second[..len].to_vec();
    let a = Pic {
        data: &first[..len],
        w: width,
        h: height,
    };
    let b = Pic {
        data: &incoming,
        w: width,
        h: height,
    };
    let mut wipe = Wipe {
        spec: sp,
        aspect: width as f32 / height as f32,
        farthest: 1.0,
        t: if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        },
    };
    if wipe.is_growing_shape() {
        wipe.farthest = wipe.farthest_of();
    }
    let wipe = &wipe;
    let (a, b) = (&a, &b);
    picasso_color::par::par_rows(&mut second[..len], width * 4, height, &|first_row, out| {
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let n = first_row * width + i;
            let (x, y) = (n % width, n / width);
            let uv = v2(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let c = shade(wipe, a, b, uv);
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

    const W: usize = 48;
    const H: usize = 27;

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
        assert!(mix_into(kind, &a, &mut b, W, H, t));
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
    fn there_are_50_and_each_starts_as_the_first_picture_and_ends_as_the_second() {
        assert_eq!(K::wipes().len(), 50);
        let (a, b) = (picture(0), picture(90));
        for kind in K::wipes() {
            assert!(handles(*kind), "{kind:?} is not drawn");
            let start = worst(&run(*kind, 0.0), &a);
            let end = worst(&run(*kind, 1.0), &b);
            assert!(
                start <= 12,
                "{kind:?} does not start as the first picture ({start})"
            );
            assert!(
                end <= 12,
                "{kind:?} does not end as the second picture ({end})"
            );
        }
    }

    #[test]
    fn the_middle_of_each_shows_both_pictures_or_moves_them() {
        let (a, b) = (picture(0), picture(90));
        for kind in K::wipes() {
            let mid = run(*kind, 0.5);
            assert!(
                worst(&mid, &a) > 12 && worst(&mid, &b) > 12,
                "{kind:?} is not between the two"
            );
            assert!(
                mid.chunks_exact(4).all(|p| p[3] == 255),
                "{kind:?} left a hole"
            );
        }
    }

    #[test]
    fn a_wipe_left_starts_at_the_right_edge() {
        let (a, b) = (picture(0), picture(90));
        let mid = run(K::WipeLeft, 0.3);
        let at = |p: &[u8], x: usize| p[((H / 2) * W + x) * 4..((H / 2) * W + x) * 4 + 3].to_vec();
        // the right end already shows the second picture, the left end still the first
        assert_eq!(at(&mid, W - 1), at(&b, W - 1));
        assert_eq!(at(&mid, 0), at(&a, 0));
    }

    #[test]
    fn a_push_moves_both_pictures() {
        let (a, b) = (picture(0), picture(90));
        let mid = run(K::PushLeft, 0.5);
        let px = |p: &[u8], x: usize, y: usize| p[(y * W + x) * 4..(y * W + x) * 4 + 3].to_vec();
        // the left half of the frame shows the right half of the first picture, and so on
        assert_eq!(px(&mid, 0, 5), px(&a, W / 2, 5));
        assert_eq!(px(&mid, W - 1, 5), px(&b, W / 2 - 1, 5));
    }

    #[test]
    fn irises_grow_from_the_middle() {
        let b = picture(90);
        for kind in [
            K::DiamondIris,
            K::OvalIris,
            K::SquareIris,
            K::HexagonIris,
            K::ArrowIrisDown,
            K::ArrowIrisLeft,
            K::ArrowIrisRight,
            K::HexagonIrisVertical,
            K::OvalIrisTall,
            K::PentagonIrisDown,
            K::TriangleIrisDown,
            K::TriangleIrisLeft,
            K::TriangleIrisRight,
            K::HeartShape,
            K::StarShape,
            K::StarShape4,
            K::StarShape6,
        ] {
            let mid = run(kind, 0.4);
            let px =
                |p: &[u8], x: usize, y: usize| p[(y * W + x) * 4..(y * W + x) * 4 + 3].to_vec();
            assert_eq!(
                px(&mid, W / 2, H / 2),
                px(&b, W / 2, H / 2),
                "{kind:?} middle"
            );
            assert_ne!(px(&mid, 0, 0), px(&b, 0, 0), "{kind:?} corner already open");
        }
    }

    #[test]
    fn a_box_grows_from_its_corner() {
        let (a, b) = (picture(0), picture(90));
        let mid = run(K::BoxUpperLeft, 0.4);
        let px = |p: &[u8], x: usize, y: usize| p[(y * W + x) * 4..(y * W + x) * 4 + 3].to_vec();
        assert_eq!(px(&mid, 0, 0), px(&b, 0, 0));
        assert_eq!(px(&mid, W - 1, H - 1), px(&a, W - 1, H - 1));
    }

    #[test]
    fn odd_sizes_short_buffers_and_bad_times_do_not_crash() {
        for kind in K::wipes() {
            let mut one = vec![10, 20, 30, 255];
            mix_into(*kind, &[1, 2, 3, 255], &mut one, 1, 1, 0.5);
            mix_into(*kind, &[], &mut [], 0, 0, 0.5);
            let mut short = vec![0u8; 8];
            mix_into(*kind, &[0u8; 8], &mut short, 4, 4, 0.5);
            assert_eq!(short, vec![0u8; 8]);
            for t in [-1.0, 2.0, f32::NAN] {
                let _ = run(*kind, t);
            }
        }
    }

    #[test]
    fn other_kinds_are_not_drawn_here() {
        let mut b = picture(90);
        assert!(!mix_into(K::CrossDissolve, &picture(0), &mut b, W, H, 0.5));
        assert!(!mix_into(K::ZoomIn, &picture(0), &mut b, W, H, 0.5));
        assert_eq!(b, picture(90));
    }
}
