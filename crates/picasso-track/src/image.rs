//! Grey pictures and positions.

/// A position in image pixels. Pixel (i, j) has its centre at exactly (i, j), so the top-left
/// pixel is at (0, 0) and the picture spans -0.5 .. width - 0.5.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Distance to `other`.
    pub fn distance(self, other: Vec2) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// A grey picture, values 0..1, row by row.
#[derive(Clone, Debug, Default)]
pub struct GrayImage {
    width: usize,
    height: usize,
    data: Vec<f32>,
}

impl GrayImage {
    /// A picture from `data` (`width * height` values); `None` when the length does not match.
    pub fn new(width: usize, height: usize, data: Vec<f32>) -> Option<Self> {
        (width.checked_mul(height)? == data.len()).then_some(Self {
            width,
            height,
            data,
        })
    }

    /// Luma (Rec. 709 weights on the stored values) of an RGBA8 picture. `None` when the length
    /// does not match.
    pub fn from_rgba8(width: usize, height: usize, rgba: &[u8]) -> Option<Self> {
        if width.checked_mul(height)?.checked_mul(4)? != rgba.len() {
            return None;
        }
        let data = rgba
            .chunks_exact(4)
            .map(|p| {
                (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
                    / 255.0
            })
            .collect();
        Some(Self {
            width,
            height,
            data,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn data(&self) -> &[f32] {
        &self.data
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Pixel (x, y); 0 outside the picture.
    pub fn at(&self, x: usize, y: usize) -> f32 {
        if x >= self.width {
            return 0.0;
        }
        self.data.get(y * self.width + x).copied().unwrap_or(0.0)
    }

    /// One row of the picture from `x` for `len` pixels, if it lies inside.
    pub(crate) fn row(&self, x: usize, y: usize, len: usize) -> Option<&[f32]> {
        if y >= self.height || x.checked_add(len)? > self.width {
            return None;
        }
        let start = y * self.width + x;
        self.data.get(start..start + len)
    }

    /// Bicubic (Catmull-Rom) value at (x, y); positions outside the picture read the nearest edge
    /// pixel. Bilinear would smooth the picture and bias sub-pixel estimates by up to a tenth of
    /// a pixel.
    pub fn sample(&self, x: f64, y: f64) -> f32 {
        if self.is_empty() || !x.is_finite() || !y.is_finite() {
            return 0.0;
        }
        let x = x.clamp(0.0, (self.width - 1) as f64);
        let y = y.clamp(0.0, (self.height - 1) as f64);
        let (x0, y0) = (x as usize, y as usize);
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let wx = cubic_weights(fx);
        let wy = cubic_weights(fy);
        let mut sum = 0.0;
        for (j, wyj) in wy.iter().enumerate() {
            let yy = (y0 + j).saturating_sub(1).min(self.height - 1);
            let mut row = 0.0;
            for (i, wxi) in wx.iter().enumerate() {
                let xx = (x0 + i).saturating_sub(1).min(self.width - 1);
                row += wxi * self.at(xx, yy);
            }
            sum += wyj * row;
        }
        sum
    }

    /// Half-size picture (2x2 box filter); at least 1x1.
    pub fn halved(&self) -> GrayImage {
        if self.is_empty() {
            return GrayImage::default();
        }
        let w = (self.width / 2).max(1);
        let h = (self.height / 2).max(1);
        let mut data = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let sx = (2 * x).min(self.width - 1);
                let sy = (2 * y).min(self.height - 1);
                let sx1 = (sx + 1).min(self.width - 1);
                let sy1 = (sy + 1).min(self.height - 1);
                data.push(
                    0.25 * (self.at(sx, sy)
                        + self.at(sx1, sy)
                        + self.at(sx, sy1)
                        + self.at(sx1, sy1)),
                );
            }
        }
        GrayImage {
            width: w,
            height: h,
            data,
        }
    }
}

fn cubic_weights(t: f32) -> [f32; 4] {
    [
        0.5 * (-t * t * t + 2.0 * t * t - t),
        0.5 * (3.0 * t * t * t - 5.0 * t * t + 2.0),
        0.5 * (-3.0 * t * t * t + 4.0 * t * t + t),
        0.5 * (t * t * t - t * t),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_checks_length() {
        assert!(GrayImage::new(2, 2, vec![0.0; 4]).is_some());
        assert!(GrayImage::new(2, 2, vec![0.0; 3]).is_none());
    }

    #[test]
    fn sample_is_exact_on_pixels_and_smooth_between() {
        let img = GrayImage::new(3, 1, vec![0.0, 1.0, 0.0]).unwrap();
        assert!((img.sample(1.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((img.sample(-5.0, 0.0)).abs() < 1e-6);
        assert!(img.sample(0.5, 0.0) > 0.4);
    }

    #[test]
    fn halved_averages() {
        let img = GrayImage::new(2, 2, vec![0.0, 1.0, 1.0, 0.0]).unwrap();
        let h = img.halved();
        assert_eq!((h.width(), h.height()), (1, 1));
        assert!((h.at(0, 0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rgba_luma() {
        let img = GrayImage::from_rgba8(1, 1, &[255, 255, 255, 255]).unwrap();
        assert!((img.at(0, 0) - 1.0).abs() < 1e-4);
    }
}
