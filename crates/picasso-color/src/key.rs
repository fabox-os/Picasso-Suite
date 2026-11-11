//! Green and blue screen keying: the screen's colour is taken out of a picture (the alpha falls
//! to 0 there) and the colour it spilled onto the subject is pulled back.
//!
//! The distance of a colour from the screen is measured on the chroma plane (CbCr divided by
//! luma plus a bias), so a darker or brighter part of the screen (shading, wrinkles) sits at about
//! the same place. It is split into "along the screen's chroma direction" and "across it":
//! anything at least as saturated as the screen in its direction counts as screen, so a brighter,
//! purer green is keyed too.

use crate::LUMA;

/// Luma offset of the chroma plane: keeps near-black pixels from exploding.
const LUMA_BIAS: f32 = 0.1;
/// Smallest screen chroma length used as the distance unit (a grey key stays finite).
const MIN_KEY_CHROMA: f32 = 0.05;
/// Smallest ramp width: a smoothstep needs a width.
const MIN_RAMP: f32 = 1e-3;

/// Typical chroma key paint colours.
pub const GREEN_SCREEN: [f32; 3] = [0.0, 177.0 / 255.0, 64.0 / 255.0];
pub const BLUE_SCREEN: [f32; 3] = [0.0, 71.0 / 255.0, 187.0 / 255.0];

/// Which channel the screen spills: green or blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spill {
    Green,
    Blue,
}

/// How a picture is keyed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChromaKey {
    /// The screen's colour, 0 to 1.
    pub screen: [f32; 3],
    /// The distance from the screen still fully transparent, 0 to 1.
    pub tolerance: f32,
    /// The width of the ramp from transparent to opaque, 0 to 1.
    pub softness: f32,
    /// How much spill is taken off the subject, 0 to 1.
    pub spill_amount: f32,
    pub spill: Spill,
}

impl ChromaKey {
    /// A green screen with the usual softness and spill removal.
    pub fn green(tolerance: f32) -> Self {
        Self {
            screen: GREEN_SCREEN,
            tolerance,
            softness: 0.2,
            spill_amount: 0.5,
            spill: Spill::Green,
        }
    }

    /// A screen of any other colour, with the usual softness and no spill removal (spill is only
    /// taken off for green and blue screens).
    pub fn plain(screen: [f32; 3], tolerance: f32) -> Self {
        Self {
            screen,
            tolerance,
            softness: 0.2,
            spill_amount: 0.0,
            spill: Spill::Green,
        }
    }

    /// A blue screen with the usual softness and spill removal.
    pub fn blue(tolerance: f32) -> Self {
        Self {
            screen: BLUE_SCREEN,
            tolerance,
            softness: 0.2,
            spill_amount: 0.5,
            spill: Spill::Blue,
        }
    }
}

fn luma(c: [f32; 3]) -> f32 {
    c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]
}

/// The position of `c` on the chroma plane.
fn chroma_of(c: [f32; 3]) -> (f32, f32) {
    let y = luma(c);
    let scale = 1.0 / (y + LUMA_BIAS);
    ((c[2] - y) / 1.8556 * scale, (c[0] - y) / 1.5748 * scale)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far `c` is from the screen: about 0 on the screen and about 1 for neutral colours.
pub fn chroma_distance(key: &ChromaKey, c: [f32; 3]) -> f32 {
    let (ku, kv) = chroma_of(key.screen);
    let (cu, cv) = chroma_of(c);
    let key_len = ku.hypot(kv).max(MIN_KEY_CHROMA);
    let (du, dv) = (ku / key_len, kv / key_len);
    let along_raw = cu * du + cv * dv;
    let along = along_raw / key_len;
    let (pu, pv) = (cu - along_raw * du, cv - along_raw * dv);
    let across = pu.hypot(pv) / key_len;
    let short = (1.0 - along).max(0.0);
    across.hypot(short)
}

/// How much of `c` is kept: 0 on the screen, 1 away from it.
pub fn chroma_matte(key: &ChromaKey, c: [f32; 3]) -> f32 {
    let d = chroma_distance(key, c);
    smoothstep(key.tolerance, key.tolerance + key.softness.max(MIN_RAMP), d)
}

/// Pulls the screen's spill off `c`: the screen channel is limited to the mean of the other two.
pub fn despill(key: &ChromaKey, c: [f32; 3]) -> [f32; 3] {
    let mut r = c;
    match key.spill {
        Spill::Blue => r[2] = c[2] - key.spill_amount * (c[2] - 0.5 * (c[0] + c[1])).max(0.0),
        Spill::Green => r[1] = c[1] - key.spill_amount * (c[1] - 0.5 * (c[0] + c[2])).max(0.0),
    }
    r
}

/// Keys an RGBA8 picture in place (straight alpha): the alpha is multiplied by the matte and the
/// spill is taken off the colours.
pub fn apply_chroma_key(rgba: &mut [u8], key: &ChromaKey) {
    for px in rgba.chunks_exact_mut(4) {
        let c = [0, 1, 2].map(|i| f32::from(px[i]) / 255.0);
        let matte = chroma_matte(key, c);
        let kept = despill(key, c);
        for i in 0..3 {
            px[i] = (kept[i] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        px[3] = (f32::from(px[3]) * matte).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(key: &ChromaKey, rgb: [u8; 3]) -> [u8; 4] {
        let mut px = [rgb[0], rgb[1], rgb[2], 255];
        apply_chroma_key(&mut px, key);
        px
    }

    #[test]
    fn the_screen_goes_transparent_and_a_skin_tone_stays() {
        let key = ChromaKey::green(0.3);
        assert_eq!(keyed(&key, [0, 177, 64])[3], 0, "the screen itself");
        assert!(keyed(&key, [20, 200, 80])[3] < 10, "a brighter screen");
        assert!(keyed(&key, [0, 90, 32])[3] < 10, "a shaded screen");
        assert_eq!(keyed(&key, [224, 172, 140])[3], 255, "skin");
        assert_eq!(keyed(&key, [128, 128, 128])[3], 255, "grey");
        assert_eq!(keyed(&key, [20, 20, 200])[3], 255, "blue");
    }

    #[test]
    fn a_blue_screen_keys_blue_not_green() {
        let key = ChromaKey::blue(0.3);
        assert_eq!(keyed(&key, [0, 71, 187])[3], 0);
        assert_eq!(keyed(&key, [0, 177, 64])[3], 255);
    }

    #[test]
    fn a_wider_tolerance_takes_out_more() {
        let near = [60, 170, 90];
        let narrow = keyed(&ChromaKey::green(0.05), near)[3];
        let wide = keyed(&ChromaKey::green(0.6), near)[3];
        assert!(wide < narrow, "{wide} vs {narrow}");
    }

    #[test]
    fn spill_is_pulled_back_and_the_alpha_of_the_picture_is_kept() {
        let key = ChromaKey::green(0.3);
        // a pinkish skin pixel with green spill
        let out = keyed(&key, [200, 190, 150]);
        assert!(out[1] < 190, "green came down: {out:?}");
        assert_eq!(out[0], 200);
        let mut half = [200, 190, 150, 100];
        apply_chroma_key(&mut half, &key);
        assert_eq!(half[3], 100, "an opaque-enough subject keeps its own alpha");
        let mut gone = [0, 177, 64, 100];
        apply_chroma_key(&mut gone, &key);
        assert_eq!(gone[3], 0);
    }

    #[test]
    fn the_matte_is_never_outside_0_to_1_even_for_black_and_a_grey_key() {
        let grey = ChromaKey {
            screen: [0.5; 3],
            ..ChromaKey::green(0.3)
        };
        for key in [ChromaKey::green(0.3), grey] {
            for r in [0.0, 0.5, 1.0] {
                for g in [0.0, 0.5, 1.0] {
                    for b in [0.0, 0.5, 1.0] {
                        let m = chroma_matte(&key, [r, g, b]);
                        assert!((0.0..=1.0).contains(&m) && m.is_finite());
                    }
                }
            }
        }
    }
}
