//! Synthetic pictures for the tests: a texture of random plane waves, exact at any real
//! position, drawn through a known projective warp.

use crate::homography::Homography;
use crate::image::{GrayImage, Vec2};

pub const W: usize = 320;
pub const H: usize = 240;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        f64::from(self.0) / f64::from(u32::MAX)
    }
}

fn waves() -> Vec<[f64; 4]> {
    let mut rng = Rng(12345);
    (0..48)
        .map(|_| {
            let k = 2.0 * std::f64::consts::PI / (5.0 + 25.0 * rng.next());
            let a = 2.0 * std::f64::consts::PI * rng.next();
            [
                k * a.cos(),
                k * a.sin(),
                2.0 * std::f64::consts::PI * rng.next(),
                0.06,
            ]
        })
        .collect()
}

fn texture(waves: &[[f64; 4]], x: f64, y: f64) -> f64 {
    0.5 + waves
        .iter()
        .map(|w| w[3] * (w[0] * x + w[1] * y + w[2]).sin())
        .sum::<f64>()
}

/// Where the texture is drawn: `to_frame` takes texture coordinates to picture pixels.
pub struct Warp(pub Homography);

impl Warp {
    pub fn shift(dx: f64, dy: f64) -> Warp {
        Warp(Homography {
            m: [1.0, 0.0, dx, 0.0, 1.0, dy, 0.0, 0.0, 1.0],
        })
    }

    /// Rotation by `deg` degrees about (160, 120), scale `s`, then a shift.
    pub fn similarity(deg: f64, s: f64, dx: f64, dy: f64) -> Warp {
        let (sn, cs) = deg.to_radians().sin_cos();
        let (c, n) = (s * cs, s * sn);
        let (cx, cy) = (160.0, 120.0);
        Warp(Homography {
            m: [
                c,
                -n,
                cx - c * cx + n * cy + dx,
                n,
                c,
                cy - n * cx - c * cy + dy,
                0.0,
                0.0,
                1.0,
            ],
        })
    }
}

/// The texture drawn through `warp`, plus noise of deviation `noise` (a fixed `seed`).
pub fn frame(warp: &Warp, noise: f64, seed: u32) -> GrayImage {
    let waves = waves();
    let inv = warp.0.inverse().expect("invertible warp");
    let mut rng = Rng(seed.wrapping_mul(2_654_435_761).max(1));
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let p = inv.map(Vec2::new(x as f64, y as f64));
            // sum of 12 uniforms ~ a normal curve
            let n = if noise > 0.0 {
                (0..12).map(|_| rng.next()).sum::<f64>() - 6.0
            } else {
                0.0
            };
            data.push((texture(&waves, p.x, p.y) + noise * n) as f32);
        }
    }
    GrayImage::new(W, H, data).expect("size")
}
