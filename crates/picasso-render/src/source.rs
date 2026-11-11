//! Draws one media item into a buffer of the output size.

use picasso_core::{MediaItem, MediaSource};

use crate::cache::{Decoded, MediaCache, VideoFrame};

/// Colour bars at 75 %: white, yellow, cyan, green, magenta, red, blue.
const BARS: [[u8; 3]; 7] = [
    [191, 191, 191],
    [191, 191, 0],
    [0, 191, 191],
    [0, 191, 0],
    [191, 0, 191],
    [191, 0, 0],
    [0, 0, 191],
];

/// Fills `out` (`w * h * 4` bytes) with the picture of `item`; pixels the picture does not cover
/// (the bars beside a picture of another shape) get alpha 0. Returns whether this is the final
/// picture: not a stand-in, a placeholder or the offline pattern of something still loading.
pub(crate) fn draw(
    item: &MediaItem,
    cache: &MediaCache,
    (w, h): (usize, usize),
    time_us: i64,
    fps: u64,
    out: &mut [u8],
) -> bool {
    match &item.source {
        MediaSource::Solid { rgb } => fill(out, [rgb[0], rgb[1], rgb[2], 255]),
        MediaSource::Gradient { from, to } => gradient(out, w, *from, *to),
        MediaSource::ColorBars => bars(out, w, h),
        MediaSource::Generator { pattern, colors } => crate::generators::draw(
            *pattern,
            colors,
            (w, h),
            crate::generators::Moment { time_us, fps },
            out,
        ),
        MediaSource::Image { .. } => match cache.get(item.id) {
            Some(image) => fit_image(out, w, h, image),
            None => {
                offline(out, w, h);
                return false;
            }
        },
        // A sound has no picture; its thumbnail is a dark tile that the media pool draws a
        // note on.
        MediaSource::Audio { .. } => fill(out, [30, 52, 56, 255]),
        MediaSource::Title(title) => {
            fill(out, [0, 0, 0, 0]);
            crate::text::draw_title(out, w, h, title);
        }
        // A compound clip is drawn from its timeline by the compositor, which knows the project.
        MediaSource::Compound { .. } => fill(out, [0, 0, 0, 0]),
        // An effects clip is transparent; its picture is whatever the tracks below compose,
        // processed by the clip's own grade and effects in `compose_tracks`.
        MediaSource::EffectsClip => fill(out, [0, 0, 0, 0]),
        MediaSource::Video { path } => match cache.video_frame(item.id, path, time_us) {
            VideoFrame::Ready(image) => fit_image(out, w, h, &image),
            VideoFrame::StandIn(image) => {
                fit_image(out, w, h, &image);
                return false;
            }
            VideoFrame::Pending => {
                fill(out, [0, 0, 0, 0]);
                return false;
            }
            VideoFrame::Offline => {
                offline(out, w, h);
                return false;
            }
        },
    }
    true
}

fn fill(out: &mut [u8], px: [u8; 4]) {
    for p in out.chunks_exact_mut(4) {
        p.copy_from_slice(&px);
    }
}

fn gradient(out: &mut [u8], w: usize, from: [u8; 3], to: [u8; 3]) {
    if w == 0 {
        return;
    }
    let last = (w.max(2) - 1) as f32;
    let row: Vec<[u8; 4]> = (0..w)
        .map(|x| {
            let t = x as f32 / last;
            let mix = |c: usize| {
                (f32::from(from[c]) + (f32::from(to[c]) - f32::from(from[c])) * t + 0.5) as u8
            };
            [mix(0), mix(1), mix(2), 255]
        })
        .collect();
    for line in out.chunks_exact_mut(w * 4) {
        for (px, c) in line.chunks_exact_mut(4).zip(&row) {
            px.copy_from_slice(c);
        }
    }
}

fn bars(out: &mut [u8], w: usize, h: usize) {
    if w == 0 {
        return;
    }
    for (y, line) in out.chunks_exact_mut(w * 4).enumerate() {
        for (x, px) in line.chunks_exact_mut(4).enumerate() {
            let bar = (x * 7 / w).min(6);
            let rgb = if y * 3 < h * 2 {
                BARS[bar]
            } else if y * 12 < h * 9 {
                // Reverse blue strip: blue, black, magenta, black, cyan, black, grey.
                [
                    BARS[6],
                    [19, 19, 19],
                    BARS[4],
                    [19, 19, 19],
                    BARS[2],
                    [19, 19, 19],
                    BARS[0],
                ][bar]
            } else {
                // Black, white, a ramp and black again.
                let t = (x as f32 / w as f32 * 4.0).min(3.999);
                match t as usize {
                    0 => [16, 16, 16],
                    1 => [235, 235, 235],
                    2 => {
                        let v = (16.0 + (t - 2.0) * 219.0) as u8;
                        [v, v, v]
                    }
                    _ => [16, 16, 16],
                }
            };
            px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
}

/// The picture shown for an image that could not be loaded: a magenta and black checkerboard.
fn offline(out: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 {
        return;
    }
    let cell = (w.max(h) / 24).max(4);
    for (y, line) in out.chunks_exact_mut(w * 4).enumerate() {
        for (x, px) in line.chunks_exact_mut(4).enumerate() {
            let on = (x / cell + y / cell).is_multiple_of(2);
            px.copy_from_slice(if on {
                &[200, 0, 200, 255]
            } else {
                &[20, 20, 20, 255]
            });
        }
    }
}

/// Scales the image to fit inside `w` x `h` without distorting it, centred, with bilinear
/// filtering. Whole numbers only (8-bit weights), and a picture that is already the right size is
/// copied row by row, because this runs for every picture of a playing video.
fn fit_image(out: &mut [u8], w: usize, h: usize, image: &Decoded) {
    let (iw, ih) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 || iw == 0 || ih == 0 || image.rgba.len() < iw * ih * 4 {
        fill(out, [0, 0, 0, 0]);
        return;
    }
    let scale = (w as f32 / iw as f32).min(h as f32 / ih as f32);
    let (dw, dh) = (
        ((iw as f32 * scale).round() as usize).clamp(1, w),
        ((ih as f32 * scale).round() as usize).clamp(1, h),
    );
    let (ox, oy) = ((w - dw) / 2, (h - dh) / 2);
    if dw < w || dh < h {
        fill(out, [0, 0, 0, 0]);
    }
    if (dw, dh) == (iw, ih) {
        for dy in 0..dh {
            let src = &image.rgba[dy * iw * 4..(dy + 1) * iw * 4];
            let dst = ((oy + dy) * w + ox) * 4;
            out[dst..dst + dw * 4].copy_from_slice(src);
        }
        return;
    }
    // For each column: the two source pixels and the weight (0..=256) of the second.
    let columns: Vec<(usize, usize, u32)> = (0..dw)
        .map(|dx| {
            let sx = ((dx as f32 + 0.5) * iw as f32 / dw as f32 - 0.5).clamp(0.0, (iw - 1) as f32);
            let x0 = sx as usize;
            (
                x0 * 4,
                (x0 + 1).min(iw - 1) * 4,
                (sx.fract() * 256.0) as u32,
            )
        })
        .collect();
    for dy in 0..dh {
        let sy = ((dy as f32 + 0.5) * ih as f32 / dh as f32 - 0.5).clamp(0.0, (ih - 1) as f32);
        let (y0, wy) = (sy as usize, (sy.fract() * 256.0) as u32);
        let y1 = (y0 + 1).min(ih - 1);
        let top = &image.rgba[y0 * iw * 4..(y0 + 1) * iw * 4];
        let bottom = &image.rgba[y1 * iw * 4..(y1 + 1) * iw * 4];
        let row = &mut out[((oy + dy) * w + ox) * 4..((oy + dy) * w + ox + dw) * 4];
        for (px, &(a, b, wx)) in row.chunks_exact_mut(4).zip(&columns) {
            for c in 0..4 {
                let t = u32::from(top[a + c]) * (256 - wx) + u32::from(top[b + c]) * wx;
                let u = u32::from(bottom[a + c]) * (256 - wx) + u32::from(bottom[b + c]) * wx;
                px[c] = ((t * (256 - wy) + u * wy + 32768) >> 16) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_core::MediaId;

    fn item(source: MediaSource) -> MediaItem {
        MediaItem {
            id: MediaId(1),
            name: "m".into(),
            source,
            length: None,
            default_duration: 10,
            bin: None,
            timecode: None,
        }
    }

    fn px(out: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [out[i], out[i + 1], out[i + 2], out[i + 3]]
    }

    #[test]
    fn solid_fills_every_pixel() {
        let mut out = vec![0; 4 * 3 * 4];
        draw(
            &item(MediaSource::Solid { rgb: [1, 2, 3] }),
            &MediaCache::new(),
            (4, 3),
            0,
             25,
            &mut out,
        );
        assert!(out.chunks_exact(4).all(|p| p == [1, 2, 3, 255]));
    }

    #[test]
    fn gradient_runs_from_one_colour_to_the_other() {
        let mut out = vec![0; 5 * 2 * 4];
        draw(
            &item(MediaSource::Gradient {
                from: [0, 0, 0],
                to: [200, 100, 50],
            }),
            &MediaCache::new(),
            (5, 2),
            0,
             25,
            &mut out,
        );
        assert_eq!(px(&out, 5, 0, 0), [0, 0, 0, 255]);
        assert_eq!(px(&out, 5, 4, 1), [200, 100, 50, 255]);
        assert_eq!(px(&out, 5, 2, 0), [100, 50, 25, 255]);
    }

    #[test]
    fn bars_have_seven_bars_on_top() {
        let (w, h) = (70, 30);
        let mut out = vec![0; w * h * 4];
        draw(
            &item(MediaSource::ColorBars),
            &MediaCache::new(),
            (w, h),
            0,
             25,
            &mut out,
        );
        let top: Vec<[u8; 4]> = (0..7).map(|b| px(&out, w, b * 10 + 5, 2)).collect();
        for (bar, p) in top.iter().enumerate() {
            assert_eq!(&p[..3], &BARS[bar]);
        }
        assert!(out.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn missing_image_shows_the_offline_pattern() {
        let mut out = vec![0; 48 * 48 * 4];
        draw(
            &item(MediaSource::Image {
                path: "gone.png".into(),
            }),
            &MediaCache::new(),
            (48, 48),
            0,
             25,
            &mut out,
        );
        assert_eq!(px(&out, 48, 0, 0), [200, 0, 200, 255]);
        assert_eq!(px(&out, 48, 4, 0), [20, 20, 20, 255]);
    }

    #[test]
    fn an_image_of_another_shape_is_letterboxed_not_stretched() {
        let mut cache = MediaCache::new();
        // 2 x 1 image, red | blue, in a square output: it fills the width and the middle half.
        cache.insert(
            MediaId(1),
            Decoded {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
            },
        );
        let mut out = vec![0; 8 * 8 * 4];
        draw(
            &item(MediaSource::Image {
                path: "a.png".into(),
            }),
            &cache,
            (8, 8),
            0,
             25,
            &mut out,
        );
        assert_eq!(px(&out, 8, 0, 0)[3], 0, "above the picture is transparent");
        assert_eq!(px(&out, 8, 0, 7)[3], 0);
        assert_eq!(px(&out, 8, 0, 4), [255, 0, 0, 255]);
        assert_eq!(px(&out, 8, 7, 4), [0, 0, 255, 255]);
    }

    #[test]
    fn a_truncated_image_buffer_does_not_panic() {
        let mut cache = MediaCache::new();
        cache.insert(
            MediaId(1),
            Decoded {
                width: 100,
                height: 100,
                rgba: vec![0; 10],
            },
        );
        let mut out = vec![0; 8 * 8 * 4];
        draw(
            &item(MediaSource::Image {
                path: "a.png".into(),
            }),
            &cache,
            (8, 8),
            0,
             25,
            &mut out,
        );
    }
}
