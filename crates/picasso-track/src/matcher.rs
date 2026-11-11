//! The template matching steps shared by the point and the planar tracker: exhaustive normalised
//! correlation and a Gauss-Newton sub-pixel refinement.

use crate::image::{GrayImage, Vec2};

const MAX_ITERATIONS: usize = 25;

/// A zero-mean pattern: `size * size` values and the root of their sum of squares.
#[derive(Clone, Debug, Default)]
pub struct Patch {
    pub size: usize,
    pub values: Vec<f32>,
    pub norm: f64,
}

impl Patch {
    /// The pattern of `image` centred on the real position `position` (bicubic samples).
    pub fn extract(image: &GrayImage, position: Vec2, size: usize) -> Patch {
        let half = (size as f64 - 1.0) / 2.0;
        let mut values = Vec::with_capacity(size * size);
        for j in 0..size {
            for i in 0..size {
                values
                    .push(image.sample(position.x + i as f64 - half, position.y + j as f64 - half));
            }
        }
        let norm = zero_mean(&mut values);
        Patch { size, values, norm }
    }
}

/// Subtracts the mean and returns the root of the sum of squares.
pub(crate) fn zero_mean(values: &mut [f32]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().map(|&v| f64::from(v)).sum::<f64>() / values.len() as f64;
    let mut sq = 0.0;
    for v in values.iter_mut() {
        *v = (f64::from(*v) - mean) as f32;
        sq += f64::from(*v) * f64::from(*v);
    }
    sq.sqrt()
}

/// Normalised correlation of `values` (zero mean, norm `norm`, side `size`) with the window whose
/// centre pixel is (cx, cy). The window must lie inside the picture.
fn correlation_at(
    image: &GrayImage,
    cx: usize,
    cy: usize,
    values: &[f32],
    size: usize,
    norm: f64,
) -> f64 {
    let half = size / 2;
    let (mut sum, mut sum_sq, mut dot) = (0.0f64, 0.0f64, 0.0f64);
    for j in 0..size {
        let Some(row) = image.row(cx - half, cy - half + j, size) else {
            return 0.0;
        };
        let t = &values[j * size..(j + 1) * size];
        for (&v, &tv) in row.iter().zip(t) {
            let v = f64::from(v);
            sum += v;
            sum_sq += v * v;
            dot += v * f64::from(tv); // the pattern has zero mean: the window mean drops out
        }
    }
    let n = (size * size) as f64;
    let var = sum_sq - sum * sum / n;
    if var < 1e-9 || norm < 1e-9 {
        return 0.0;
    }
    dot / (var.sqrt() * norm)
}

/// Normalised correlation (-1..1) of the pattern with the window centred on the real position
/// `p`. 0 for a flat window or pattern.
pub fn correlation_real(image: &GrayImage, p: Vec2, values: &[f32], size: usize, norm: f64) -> f64 {
    let half = (size as f64 - 1.0) / 2.0;
    let (mut sum, mut sum_sq, mut dot) = (0.0f64, 0.0f64, 0.0f64);
    for j in 0..size {
        for i in 0..size {
            let v = f64::from(image.sample(p.x + i as f64 - half, p.y + j as f64 - half));
            sum += v;
            sum_sq += v * v;
            dot += v * f64::from(values[j * size + i]);
        }
    }
    let n = (size * size) as f64;
    let var = sum_sq - sum * sum / n;
    if var < 1e-9 || norm < 1e-9 {
        return 0.0;
    }
    dot / (var.sqrt() * norm)
}

/// Best integer position in `[cx - radius, cx + radius] x [cy - radius, cy + radius]` whose whole
/// window lies inside the picture, and the correlation there. Ties go to the first candidate
/// (nearest the top left, as in the original). `None` when no candidate fits.
pub fn best_integer(
    image: &GrayImage,
    cx: i64,
    cy: i64,
    radius: i64,
    values: &[f32],
    size: usize,
    norm: f64,
) -> Option<(i64, i64, f64)> {
    let half = (size / 2) as i64;
    let x0 = (cx - radius).max(half);
    let x1 = (cx + radius).min(image.width() as i64 - 1 - half);
    let y0 = (cy - radius).max(half);
    let y1 = (cy + radius).min(image.height() as i64 - 1 - half);
    if x0 > x1 || y0 > y1 {
        return None;
    }
    let mut best = (x0, y0, -2.0f64);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = correlation_at(image, x as usize, y as usize, values, size, norm);
            if c > best.2 + 1e-12 {
                best = (x, y, c);
            }
        }
    }
    Some(best)
}

/// Gauss-Newton refinement of a real position: minimises the difference between the normalised
/// pattern and the normalised window. Returns `start` when it does not converge to a place within
/// `max_move` of the start.
pub fn refine(
    image: &GrayImage,
    start: Vec2,
    values: &[f32],
    size: usize,
    norm: f64,
    max_move: f64,
) -> Vec2 {
    let mut p = start;
    let half = (size as f64 - 1.0) / 2.0;
    let count = values.len();
    let mut window = vec![0.0f32; count];
    let mut gx = vec![0.0f32; count];
    let mut gy = vec![0.0f32; count];
    for _ in 0..MAX_ITERATIONS {
        for j in 0..size {
            for i in 0..size {
                let x = p.x + i as f64 - half;
                let y = p.y + j as f64 - half;
                let k = j * size + i;
                window[k] = image.sample(x, y);
                gx[k] = 0.5 * (image.sample(x + 1.0, y) - image.sample(x - 1.0, y));
                gy[k] = 0.5 * (image.sample(x, y + 1.0) - image.sample(x, y - 1.0));
            }
        }
        let wnorm = zero_mean(&mut window);
        if wnorm < 1e-6 || norm < 1e-9 {
            return start;
        }
        // residual r = window/|window| - pattern/|pattern|; Jacobian = gradient/|window|.
        let (mut a, mut b, mut c, mut bx, mut by) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for k in 0..count {
            let r = f64::from(window[k]) / wnorm - f64::from(values[k]) / norm;
            let jx = f64::from(gx[k]) / wnorm;
            let jy = f64::from(gy[k]) / wnorm;
            a += jx * jx;
            b += jx * jy;
            c += jy * jy;
            bx += jx * r;
            by += jy * r;
        }
        let det = a * c - b * b;
        if det < 1e-18 {
            break;
        }
        let dx = -(c * bx - b * by) / det;
        let dy = -(a * by - b * bx) / det;
        p.x += dx;
        p.y += dy;
        if p.distance(start) > max_move {
            return start;
        }
        if dx.hypot(dy) < 1e-4 {
            break;
        }
    }
    p
}
