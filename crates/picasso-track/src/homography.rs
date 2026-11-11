//! The maths of the planar tracker: a 3x3 projective map, its estimation from point pairs
//! (normalised direct linear transform, with RANSAC against outliers) and the similarity (move,
//! turn, scale) that best describes a quad's motion. Textbook methods (Hartley and Zisserman,
//! "Multiple View Geometry", chapters 4 and 11).

use crate::image::Vec2;

/// Where a point sent to infinity is put instead.
const FAR_AWAY: f64 = 1e12;
/// |w| below this counts as "at infinity".
const MIN_W: f64 = 1e-12;
const JACOBI_SWEEPS: usize = 60;
/// Second-smallest eigenvalue of the DLT system relative to the largest: below this the map is
/// not determined by the points (all on one line, or fewer than four distinct ones).
const DEGENERATE_RATIO: f64 = 1e-9;
/// Sine of the angle under which three points are "in line".
const COLLINEAR_SINE: f64 = 0.02;

type Mat3 = [f64; 9];

fn multiply(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut r = [0.0; 9];
    for i in 0..3 {
        for j in 0..3 {
            r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
        }
    }
    r
}

/// A projective map of the plane: `(x, y) -> ((m0 x + m1 y + m2) / w, (m3 x + m4 y + m5) / w)`
/// with `w = m6 x + m7 y + m8`. Row-major. The default is the identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography {
    pub m: [f64; 9],
}

impl Default for Homography {
    fn default() -> Self {
        Self {
            m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl Homography {
    /// Where `p` goes. A point sent to infinity comes back as the far-away point 1e12 along its
    /// direction, never as NaN.
    pub fn map(&self, p: Vec2) -> Vec2 {
        let m = &self.m;
        let w = m[6] * p.x + m[7] * p.y + m[8];
        let x = m[0] * p.x + m[1] * p.y + m[2];
        let y = m[3] * p.x + m[4] * p.y + m[5];
        if w.abs() < MIN_W {
            let len = x.hypot(y).max(1e-300);
            return Vec2::new(x / len * FAR_AWAY, y / len * FAR_AWAY);
        }
        Vec2::new(x / w, y / w)
    }

    /// The inverse map, or `None` when the matrix is singular.
    pub fn inverse(&self) -> Option<Homography> {
        let a = &self.m;
        let c00 = a[4] * a[8] - a[5] * a[7];
        let c01 = a[5] * a[6] - a[3] * a[8];
        let c02 = a[3] * a[7] - a[4] * a[6];
        let det = a[0] * c00 + a[1] * c01 + a[2] * c02;
        let scale = a[0].abs().max(a[4].abs()).max(a[8].abs()).max(1e-300);
        if !det.is_finite() || det.abs() < 1e-14 * scale * scale * scale {
            return None;
        }
        Some(Homography {
            m: [
                c00 / det,
                (a[2] * a[7] - a[1] * a[8]) / det,
                (a[1] * a[5] - a[2] * a[4]) / det,
                c01 / det,
                (a[0] * a[8] - a[2] * a[6]) / det,
                (a[2] * a[3] - a[0] * a[5]) / det,
                c02 / det,
                (a[1] * a[6] - a[0] * a[7]) / det,
                (a[0] * a[4] - a[1] * a[3]) / det,
            ],
        })
    }

    /// The map "first `first`, then this".
    pub fn after(&self, first: &Homography) -> Homography {
        Homography {
            m: multiply(&self.m, &first.m),
        }
    }

    /// Scaled so that `m8 = 1` (or, when `m8` is about 0, to unit length); keeps the map.
    pub fn normalised(&self) -> Homography {
        let mut d = self.m[8];
        if d.abs() < 1e-12 {
            d = self.m.iter().map(|v| v * v).sum::<f64>().sqrt();
        }
        let mut r = *self;
        if d != 0.0 {
            for v in &mut r.m {
                *v /= d;
            }
        }
        r
    }

    pub fn is_finite(&self) -> bool {
        self.m.iter().all(|v| v.is_finite())
    }
}

/// Four corners of a quad: top-left, top-right, bottom-right, bottom-left (for a picture, y down).
pub type Quad = [Vec2; 4];

/// Moves the points to their centroid and scales them to a mean distance of sqrt(2).
fn normalisation(pts: &[Vec2]) -> Option<Mat3> {
    let n = pts.len() as f64;
    let cx = pts.iter().map(|p| p.x).sum::<f64>() / n;
    let cy = pts.iter().map(|p| p.y).sum::<f64>() / n;
    let d = pts.iter().map(|p| (p.x - cx).hypot(p.y - cy)).sum::<f64>() / n;
    if d < 1e-9 {
        return None;
    }
    let s = 2f64.sqrt() / d;
    Some([s, 0.0, -s * cx, 0.0, s, -s * cy, 0.0, 0.0, 1.0])
}

/// Eigen decomposition of the symmetric n x n matrix `a` (row-major, destroyed) by cyclic Jacobi
/// rotations. Returns the eigenvalues and the eigenvectors as columns of a row-major matrix.
fn jacobi_eigen(a: &mut [f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..JACOBI_SWEEPS {
        let mut off = 0.0;
        for i in 0..n {
            for j in i + 1..n {
                off += a[i * n + j] * a[i * n + j];
            }
        }
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = a[p * n + q];
                if apq.abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * apq);
                let t = (if theta >= 0.0 { 1.0 } else { -1.0 })
                    / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    ((0..n).map(|i| a[i * n + i]).collect(), v)
}

fn cross(o: Vec2, a: Vec2, b: Vec2) -> f64 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// True if the three points are nearly on one line (or two of them coincide).
fn nearly_collinear(a: Vec2, b: Vec2, c: Vec2) -> bool {
    let lab = a.distance(b);
    let lac = a.distance(c);
    if lab < 1e-9 || lac < 1e-9 {
        return true;
    }
    cross(a, b, c).abs() < COLLINEAR_SINE * lab * lac
}

fn sample_is_degenerate(pts: &[Vec2], idx: &[usize; 4]) -> bool {
    (0..4).any(|skip| {
        let t: Vec<Vec2> = (0..4).filter(|&i| i != skip).map(|i| pts[idx[i]]).collect();
        nearly_collinear(t[0], t[1], t[2])
    })
}

/// xorshift32: a tiny deterministic generator, so a fit is reproducible.
fn next_random(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

/// Least-squares homography taking `from[i]` to `to[i]` (normalised DLT: both point sets are
/// first moved to their centroid and scaled to a mean distance of sqrt(2), which keeps the linear
/// system well conditioned for pixel coordinates). Needs at least 4 pairs; exactly 4 give the
/// exact map. `None` for fewer pairs, mismatched sizes, non-finite input and degenerate
/// configurations (all points equal or on one line).
pub fn homography_from_points(from: &[Vec2], to: &[Vec2]) -> Option<Homography> {
    let n = from.len();
    if n < 4 || to.len() != n {
        return None;
    }
    if !from.iter().chain(to).all(|p| p.is_finite()) {
        return None;
    }
    let ta = normalisation(from)?;
    let tb = normalisation(to)?;
    let mut ata = vec![0.0; 81];
    let mut add_row = |row: &[f64; 9]| {
        for i in 0..9 {
            for j in 0..9 {
                ata[i * 9 + j] += row[i] * row[j];
            }
        }
    };
    for i in 0..n {
        let x = ta[0] * from[i].x + ta[2];
        let y = ta[4] * from[i].y + ta[5];
        let u = tb[0] * to[i].x + tb[2];
        let v = tb[4] * to[i].y + tb[5];
        add_row(&[-x, -y, -1.0, 0.0, 0.0, 0.0, u * x, u * y, u]);
        add_row(&[0.0, 0.0, 0.0, -x, -y, -1.0, v * x, v * y, v]);
    }
    let (values, vectors) = jacobi_eigen(&mut ata, 9);
    let mut order: Vec<usize> = (0..9).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let largest = values[order[8]].max(1e-300);
    if values[order[1]] < DEGENERATE_RATIO * largest {
        return None; // the null space is more than one-dimensional: not determined
    }
    let mut hn = [0.0; 9];
    for (i, h) in hn.iter_mut().enumerate() {
        *h = vectors[i * 9 + order[0]];
    }
    let tb_inv = Homography { m: tb }.inverse()?;
    let h = Homography {
        m: multiply(&multiply(&tb_inv.m, &hn), &ta),
    }
    .normalised();
    h.is_finite().then_some(h)
}

/// The homography taking quad `from` to quad `to`; `None` if either is degenerate.
pub fn homography_from_quads(from: &Quad, to: &Quad) -> Option<Homography> {
    homography_from_points(from, to)
}

/// How RANSAC searches.
#[derive(Clone, Copy, Debug)]
pub struct RansacSettings {
    /// Random 4-point samples tried.
    pub iterations: usize,
    /// A pair is an inlier if the map lands this close (pixels).
    pub inlier_threshold: f64,
    /// The generator is deterministic: same input, same result.
    pub seed: u32,
}

impl Default for RansacSettings {
    fn default() -> Self {
        Self {
            iterations: 300,
            inlier_threshold: 1.5,
            seed: 12345,
        }
    }
}

/// Result of [`estimate_homography`].
#[derive(Clone, Debug)]
pub struct HomographyFit {
    pub h: Homography,
    /// One flag per input pair.
    pub inliers: Vec<bool>,
    pub inlier_count: usize,
    /// Mean distance of the inliers from their target, pixels.
    pub mean_error: f64,
}

fn score(h: &Homography, from: &[Vec2], to: &[Vec2], threshold: f64) -> (Vec<bool>, usize, f64) {
    let mut count = 0;
    let mut sum = 0.0;
    let flags = from
        .iter()
        .zip(to)
        .map(|(f, t)| {
            let e = h.map(*f).distance(*t);
            let inlier = e <= threshold;
            if inlier {
                count += 1;
                sum += e;
            }
            inlier
        })
        .collect();
    (flags, count, sum)
}

/// Robust homography: RANSAC over 4-point samples (samples with three nearly collinear points are
/// skipped), then a least-squares refit on all inliers, repeated until the inlier set stops
/// growing. `None` when no model with at least 4 inliers exists.
pub fn estimate_homography(
    from: &[Vec2],
    to: &[Vec2],
    settings: &RansacSettings,
) -> Option<HomographyFit> {
    let n = from.len();
    if n < 4 || to.len() != n || !from.iter().chain(to).all(|p| p.is_finite()) {
        return None;
    }
    let threshold = settings.inlier_threshold.max(1e-6);
    let mut rng = if settings.seed == 0 { 1 } else { settings.seed };
    let mut best: Option<(Homography, Vec<bool>, usize, f64)> = None;
    for _ in 0..settings.iterations.max(1) {
        let mut idx = [0usize; 4];
        for k in 0..4 {
            loop {
                idx[k] = next_random(&mut rng) as usize % n;
                if !idx[..k].contains(&idx[k]) {
                    break;
                }
            }
        }
        if sample_is_degenerate(from, &idx) || sample_is_degenerate(to, &idx) {
            continue;
        }
        let a: Vec<Vec2> = idx.iter().map(|&k| from[k]).collect();
        let b: Vec<Vec2> = idx.iter().map(|&k| to[k]).collect();
        let Some(h) = homography_from_points(&a, &b) else {
            continue;
        };
        let (flags, count, err) = score(&h, from, to, threshold);
        let better = match &best {
            None => count > 0,
            Some((_, _, bc, be)) => count > *bc || (count == *bc && count > 0 && err < *be),
        };
        if better {
            best = Some((h, flags, count, err));
        }
    }
    let (mut best_h, mut best_flags, mut best_count, mut best_error) = best?;
    if best_count < 4 {
        return None;
    }
    // Refit on all inliers until the inlier set stops growing.
    for _ in 0..6 {
        let (a, b): (Vec<Vec2>, Vec<Vec2>) = from
            .iter()
            .zip(to)
            .zip(&best_flags)
            .filter(|(_, &f)| f)
            .map(|((f, t), _)| (*f, *t))
            .unzip();
        let Some(h) = homography_from_points(&a, &b) else {
            break;
        };
        let (flags, count, err) = score(&h, from, to, threshold);
        if count < best_count {
            break;
        }
        let same = flags == best_flags;
        best_h = h;
        best_flags = flags;
        best_count = count;
        best_error = err;
        if same {
            break;
        }
    }
    Some(HomographyFit {
        h: best_h,
        inliers: best_flags,
        inlier_count: best_count,
        mean_error: if best_count > 0 {
            best_error / best_count as f64
        } else {
            0.0
        },
    })
}

/// A similarity: `to = scale * R(angle) * from + (dx, dy)`, R a rotation in the picture's own
/// coordinates (for y-down pictures, a positive angle turns clockwise on screen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Similarity {
    pub scale: f64,
    pub angle_rad: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Default for Similarity {
    fn default() -> Self {
        Self {
            scale: 1.0,
            angle_rad: 0.0,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

impl Similarity {
    pub fn map(&self, p: Vec2) -> Vec2 {
        let (s, c) = self.angle_rad.sin_cos();
        let (c, s) = (c * self.scale, s * self.scale);
        Vec2::new(c * p.x - s * p.y + self.dx, s * p.x + c * p.y + self.dy)
    }

    /// The opposite similarity (undoes this one). The scale must not be 0.
    pub fn inverse(&self) -> Similarity {
        let scale = 1.0 / self.scale;
        let angle_rad = -self.angle_rad;
        let (s, c) = angle_rad.sin_cos();
        let (c, s) = (c * scale, s * scale);
        Similarity {
            scale,
            angle_rad,
            dx: -(c * self.dx - s * self.dy),
            dy: -(s * self.dx + c * self.dy),
        }
    }
}

/// Least-squares similarity from `from` to `to` (closed form, Umeyama 1991). `None` for fewer than
/// 2 pairs, mismatched sizes, or when all `from` points coincide.
pub fn similarity_from_points(from: &[Vec2], to: &[Vec2]) -> Option<Similarity> {
    let n = from.len();
    if n < 2 || to.len() != n || !from.iter().chain(to).all(|p| p.is_finite()) {
        return None;
    }
    let nf = n as f64;
    let ca = Vec2::new(
        from.iter().map(|p| p.x).sum::<f64>() / nf,
        from.iter().map(|p| p.y).sum::<f64>() / nf,
    );
    let cb = Vec2::new(
        to.iter().map(|p| p.x).sum::<f64>() / nf,
        to.iter().map(|p| p.y).sum::<f64>() / nf,
    );
    let (mut dot, mut crs, mut var_a) = (0.0, 0.0, 0.0);
    for (f, t) in from.iter().zip(to) {
        let (ax, ay) = (f.x - ca.x, f.y - ca.y);
        let (bx, by) = (t.x - cb.x, t.y - cb.y);
        dot += ax * bx + ay * by;
        crs += ax * by - ay * bx;
        var_a += ax * ax + ay * ay;
    }
    if var_a < 1e-12 {
        return None;
    }
    let angle_rad = crs.atan2(dot);
    let scale = dot.hypot(crs) / var_a;
    if scale < 1e-9 {
        return None;
    }
    let (sn, c) = angle_rad.sin_cos();
    let (c, sn) = (c * scale, sn * scale);
    Some(Similarity {
        scale,
        angle_rad,
        dx: cb.x - (c * ca.x - sn * ca.y),
        dy: cb.y - (sn * ca.x + c * ca.y),
    })
}

/// The similarity that best describes how quad `from` became quad `to`.
pub fn similarity_from_quads(from: &Quad, to: &Quad) -> Option<Similarity> {
    similarity_from_points(from, to)
}

/// Area of `q` (shoelace formula, absolute value).
pub fn quad_area(q: &Quad) -> f64 {
    let a: f64 = (0..4)
        .map(|i| {
            let (p, r) = (q[i], q[(i + 1) % 4]);
            p.x * r.y - r.x * p.y
        })
        .sum();
    a.abs() / 2.0
}

/// True if `q` is a usable plane: convex, one winding all round, and an area of at least
/// `min_area` square pixels.
pub fn is_convex_quad(q: &Quad, min_area: f64) -> bool {
    if !q.iter().all(|p| p.is_finite()) {
        return false;
    }
    let mut sign = 0.0;
    for i in 0..4 {
        let c = cross(q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        if c.abs() < 1e-9 {
            return false;
        }
        let s = if c > 0.0 { 1.0 } else { -1.0 };
        if sign == 0.0 {
            sign = s;
        } else if s != sign {
            return false;
        }
    }
    quad_area(q) >= min_area
}

/// The point where the diagonals of `q` cross: the centre of the plane as seen in the picture
/// (for a perspective view it is not the average of the corners).
pub fn quad_centre(q: &Quad) -> Vec2 {
    let (d1x, d1y) = (q[2].x - q[0].x, q[2].y - q[0].y);
    let (d2x, d2y) = (q[3].x - q[1].x, q[3].y - q[1].y);
    let den = d1x * d2y - d1y * d2x;
    if den.abs() < 1e-12 {
        return Vec2::new(
            (q[0].x + q[1].x + q[2].x + q[3].x) / 4.0,
            (q[0].y + q[1].y + q[2].y + q[3].y) / 4.0,
        );
    }
    let t = ((q[1].x - q[0].x) * d2y - (q[1].y - q[0].y) * d2x) / den;
    Vec2::new(q[0].x + t * d1x, q[0].y + t * d1y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT: Quad = [
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 0.0),
        Vec2::new(100.0, 100.0),
        Vec2::new(0.0, 100.0),
    ];

    fn near(a: Vec2, b: Vec2, tol: f64) -> bool {
        a.distance(b) < tol
    }

    #[test]
    fn four_points_give_the_exact_map() {
        let to = [
            Vec2::new(10.0, 20.0),
            Vec2::new(150.0, 30.0),
            Vec2::new(130.0, 160.0),
            Vec2::new(5.0, 120.0),
        ];
        let h = homography_from_quads(&UNIT, &to).unwrap();
        for (f, t) in UNIT.iter().zip(&to) {
            assert!(near(h.map(*f), *t, 1e-6));
        }
        let back = h.inverse().unwrap();
        assert!(near(back.map(to[2]), UNIT[2], 1e-6));
    }

    #[test]
    fn degenerate_input_is_refused() {
        let line: Vec<Vec2> = (0..6)
            .map(|i| Vec2::new(f64::from(i), 2.0 * f64::from(i)))
            .collect();
        assert!(homography_from_points(&line, &line).is_none());
        let same = vec![Vec2::new(1.0, 1.0); 5];
        assert!(homography_from_points(&same, &same).is_none());
        assert!(homography_from_points(&UNIT[..3], &UNIT[..3]).is_none());
        let nan = [Vec2::new(f64::NAN, 0.0), UNIT[1], UNIT[2], UNIT[3]];
        assert!(homography_from_points(&nan, &UNIT).is_none());
    }

    #[test]
    fn ransac_ignores_outliers() {
        let truth = Homography {
            m: [1.1, 0.05, 12.0, -0.04, 0.95, -7.0, 0.0002, 0.0001, 1.0],
        };
        let mut from = Vec::new();
        let mut to = Vec::new();
        for j in 0..7 {
            for i in 0..7 {
                let p = Vec2::new(20.0 + 40.0 * f64::from(i), 15.0 + 30.0 * f64::from(j));
                from.push(p);
                to.push(truth.map(p));
            }
        }
        for k in [3usize, 11, 20, 31, 40] {
            to[k].x += 25.0;
            to[k].y -= 18.0;
        }
        let fit = estimate_homography(&from, &to, &RansacSettings::default()).unwrap();
        assert_eq!(fit.inlier_count, 44);
        assert!(!fit.inliers[3]);
        for p in &from {
            assert!(near(fit.h.map(*p), truth.map(*p), 0.01));
        }
        // deterministic
        let again = estimate_homography(&from, &to, &RansacSettings::default()).unwrap();
        assert_eq!(fit.inliers, again.inliers);
    }

    #[test]
    fn similarity_round_trips() {
        let s = Similarity {
            scale: 1.3,
            angle_rad: 0.4,
            dx: 5.0,
            dy: -9.0,
        };
        let to: Vec<Vec2> = UNIT.iter().map(|p| s.map(*p)).collect();
        let fit = similarity_from_points(&UNIT, &to).unwrap();
        assert!((fit.scale - 1.3).abs() < 1e-9);
        assert!((fit.angle_rad - 0.4).abs() < 1e-9);
        let inv = s.inverse();
        assert!(near(
            inv.map(s.map(Vec2::new(7.0, 3.0))),
            Vec2::new(7.0, 3.0),
            1e-9
        ));
        assert!(similarity_from_points(&[Vec2::new(1.0, 1.0); 3], &UNIT[..3]).is_none());
    }

    #[test]
    fn quads() {
        assert!((quad_area(&UNIT) - 10000.0).abs() < 1e-9);
        assert!(is_convex_quad(&UNIT, 1.0));
        let bow = [UNIT[0], UNIT[2], UNIT[1], UNIT[3]];
        assert!(!is_convex_quad(&bow, 1.0));
        assert!(near(quad_centre(&UNIT), Vec2::new(50.0, 50.0), 1e-9));
    }

    #[test]
    fn a_point_at_infinity_is_finite() {
        let h = Homography {
            m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0],
        };
        assert!(h.map(Vec2::new(0.0, 5.0)).is_finite());
    }
}
