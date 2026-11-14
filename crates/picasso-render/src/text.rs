//! Titles: text drawn into a picture with the Noto Sans fonts (SIL Open Font License, in
//! `crates/picasso-ui/assets/fonts`), outlined and filled by `ab_glyph`, with the Text+ look:
//! a background box, a drop shadow, an outline, letter spacing and alignment.

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use picasso_core::{TextAlign, TextStyle, TitleText};

static REGULAR: &[u8] = include_bytes!("../../picasso-ui/assets/fonts/NotoSans-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../../picasso-ui/assets/fonts/NotoSans-SemiBold.ttf");

/// The most lines of a title that are drawn.
const MAX_LINES: usize = 64;

/// Draws `title` over `out` (`w` x `h`, RGBA, straight alpha): its background box, shadow,
/// outline and letters, in that order, the lines lined up as the style says around the
/// title's place.
pub(crate) fn draw_title(out: &mut [u8], w: usize, h: usize, title: &TitleText) {
    let Some(len) = w.checked_mul(h) else {
        return;
    };
    if len == 0 || out.len() < len * 4 {
        return;
    }
    let style = title.style.clone().sanitized();
    let mask = text_mask(w, h, title, &style);
    let Some(bounds) = covered(&mask, w, h) else {
        return;
    };
    let unit = h as f32 / 1000.0;
    if style.background && style.background_opacity > 0 {
        let pad = (style.padding as f32 * unit).round() as i64;
        let (x0, y0, x1, y1) = bounds;
        let alpha = style.background_opacity as f32 / 100.0;
        let mut boxed = vec![0f32; len];
        for y in (y0 as i64 - pad).max(0)..(y1 as i64 + pad + 1).min(h as i64) {
            for x in (x0 as i64 - pad).max(0)..(x1 as i64 + pad + 1).min(w as i64) {
                boxed[y as usize * w + x as usize] = alpha;
            }
        }
        paint(out, &boxed, style.background_color);
    }
    if style.shadow > 0 && style.shadow_opacity > 0 {
        let offset = (style.shadow as f32 * unit).round().max(1.0) as usize;
        let alpha = style.shadow_opacity as f32 / 100.0;
        let mut shadow = vec![0f32; len];
        for y in 0..h.saturating_sub(offset) {
            for x in 0..w.saturating_sub(offset) {
                shadow[(y + offset) * w + x + offset] = mask[y * w + x] * alpha;
            }
        }
        let radius = (style.shadow_blur as f32 * unit).round() as usize;
        if radius > 0 {
            shadow = blur(&shadow, w, h, radius);
        }
        paint(out, &shadow, style.shadow_color);
    }
    if style.outline > 0 {
        let radius = (style.outline as f32 * unit).round().max(1.0) as usize;
        let grown = dilate(&mask, w, h, radius);
        paint(out, &grown, style.outline_color);
    }
    let opacity = style.opacity as f32 / 100.0;
    if opacity < 1.0 {
        let faded: Vec<f32> = mask.iter().map(|a| a * opacity).collect();
        paint(out, &faded, title.color);
    } else {
        paint(out, &mask, title.color);
    }
}

/// Draws `title` as it looks `time_us` microseconds after the start of its clip: the plain title
/// once its animation (if it has one) is over.
pub(crate) fn draw_title_at(out: &mut [u8], w: usize, h: usize, title: &TitleText, time_us: i64) {
    use picasso_core::TitleAnimation;
    let kind = title.style.animation;
    let ms = title.style.animation_ms.clamp(100, 5000) as f32;
    let p = (time_us.max(0) as f32 / 1000.0 / ms).clamp(0.0, 1.0);
    let Some(len) = w.checked_mul(h) else {
        return;
    };
    if kind == TitleAnimation::None || p >= 1.0 || len == 0 || out.len() < len * 4 {
        draw_title(out, w, h, title);
        return;
    }
    let mut layer = vec![0u8; len * 4];
    draw_title(&mut layer, w, h, title);
    crate::title_motion::animate(&mut layer, w, h, title, kind, p);
    crate::title_motion::over(&mut out[..len * 4], &layer);
}

/// How much of each pixel the letters cover, 0 to 1.
fn text_mask(w: usize, h: usize, title: &TitleText, style: &TextStyle) -> Vec<f32> {
    let mut mask = vec![0f32; w * h];
    let Ok(font) = FontRef::try_from_slice(if title.bold { BOLD } else { REGULAR }) else {
        return mask;
    };
    let px = (title.size.clamp(10, 500) as f32 / 1000.0 * h as f32).max(1.0);
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let lines: Vec<&str> = title.text.lines().take(MAX_LINES).collect();
    let line_height = (scaled.height() + scaled.line_gap()) * style.line_spacing as f32 / 100.0;
    let total = line_height * lines.len() as f32;
    let anchor_x = title.x.min(1000) as f32 / 1000.0 * w as f32;
    let top = title.y.min(1000) as f32 / 1000.0 * h as f32 - total / 2.0;
    let tracking = style.tracking as f32 / 1000.0 * px;
    // Lines line up on the widest one, which is centred on the place.
    let widths: Vec<f32> = lines
        .iter()
        .map(|line| line_width(&scaled, line, tracking))
        .collect();
    let widest = widths.iter().copied().fold(0.0f32, f32::max);
    for (i, line) in lines.iter().enumerate() {
        let width = widths.get(i).copied().unwrap_or(0.0);
        let mut caret = match style.align {
            TextAlign::Center => anchor_x - width / 2.0,
            TextAlign::Left => anchor_x - widest / 2.0,
            TextAlign::Right => anchor_x + widest / 2.0 - width,
        };
        let baseline = top + line_height * i as f32 + scaled.ascent();
        let mut previous = None;
        for c in line.chars() {
            let id = font.glyph_id(c);
            if let Some(p) = previous {
                caret += scaled.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, point(caret, baseline));
            caret += scaled.h_advance(id) + tracking;
            previous = Some(id);
            let Some(outlined) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let x = bounds.min.x as i64 + i64::from(gx);
                let y = bounds.min.y as i64 + i64::from(gy);
                if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    return;
                }
                if let Some(m) = mask.get_mut(y as usize * w + x as usize) {
                    // Letters that touch add up, never past full.
                    *m = (*m + coverage.clamp(0.0, 1.0)).min(1.0);
                }
            });
        }
    }
    mask
}

fn line_width(scaled: &ab_glyph::PxScaleFont<&FontRef<'_>>, line: &str, tracking: f32) -> f32 {
    let mut width = 0.0;
    let mut previous = None;
    let mut n = 0usize;
    for c in line.chars() {
        let id = scaled.glyph_id(c);
        if let Some(p) = previous {
            width += scaled.kern(p, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
        n += 1;
    }
    width + tracking * n.saturating_sub(1) as f32
}

/// The box the letters cover: left, top, right, bottom (inclusive), or None when empty.
fn covered(mask: &[f32], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (i, m) in mask.iter().enumerate() {
        if *m > 0.0 {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    (x0 <= x1).then_some((x0, y0, x1, y1))
}

/// Paints `color` over `out` where `alpha` (0 to 1 per pixel) says, straight alpha.
fn paint(out: &mut [u8], alpha: &[f32], color: [u8; 3]) {
    for (px, a) in out.chunks_exact_mut(4).zip(alpha) {
        let a = a.clamp(0.0, 1.0);
        if a <= 0.0 {
            continue;
        }
        let below = f32::from(px[3]) / 255.0;
        let total = a + below * (1.0 - a);
        for (v, c) in px[..3].iter_mut().zip(color) {
            let mixed = (f32::from(c) * a + f32::from(*v) * below * (1.0 - a)) / total;
            *v = mixed.round().clamp(0.0, 255.0) as u8;
        }
        px[3] = (total * 255.0).round() as u8;
    }
}

/// The largest value within `radius` of each pixel (a round brush), for the outline.
fn dilate(mask: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    let r = radius.min(64) as i64;
    let offsets: Vec<(i64, i64)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| dx * dx + dy * dy <= r * r + r)
        .collect();
    let mut out = mask.to_vec();
    // Only the pixels near the letters can change.
    for (i, m) in mask.iter().enumerate() {
        if *m <= 0.0 {
            continue;
        }
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dx, dy) in &offsets {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let o = &mut out[ny as usize * w + nx as usize];
            *o = o.max(*m);
        }
    }
    out
}

/// A box blur of `radius`, rows then columns, for a soft shadow.
pub(crate) fn blur(v: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    let pass = |src: &[f32], len: usize, stride: usize, lines: usize, step: usize| {
        let mut dst = vec![0f32; src.len()];
        for line in 0..lines {
            let at = |i: usize| line * step + i * stride;
            let mut sum = 0.0;
            let mut count = 0.0;
            for i in 0..radius.min(len) {
                sum += src[at(i)];
                count += 1.0;
            }
            for i in 0..len {
                if i + radius < len {
                    sum += src[at(i + radius)];
                    count += 1.0;
                }
                if i > radius {
                    sum -= src[at(i - radius - 1)];
                    count -= 1.0;
                }
                dst[at(i)] = sum / count;
            }
        }
        dst
    };
    let rows = pass(v, w, 1, h, w);
    pass(&rows, h, w, w, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(title: &TitleText) -> Vec<u8> {
        let mut out = vec![0u8; 200 * 100 * 4];
        draw_title(&mut out, 200, 100, title);
        out
    }

    fn count(out: &[u8], f: impl Fn(&[u8]) -> bool) -> usize {
        out.chunks_exact(4).filter(|p| f(p)).count()
    }

    #[test]
    fn text_is_drawn_in_its_colour_around_its_place() {
        let title = TitleText {
            text: "HHHH".into(),
            size: 300,
            color: [255, 0, 0],
            ..Default::default()
        };
        let out = drawn(&title);
        let covered: Vec<(usize, usize)> = out
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, p)| p[3] > 200)
            .map(|(i, _)| (i % 200, i / 200))
            .collect();
        assert!(covered.len() > 100, "some text is drawn");
        let (sx, sy) = covered.iter().fold((0, 0), |(a, b), (x, y)| (a + x, b + y));
        let (cx, cy) = (sx / covered.len(), sy / covered.len());
        assert!(
            cx.abs_diff(100) < 10 && cy.abs_diff(50) < 15,
            "centred: {cx} {cy}"
        );
        assert!(out
            .chunks_exact(4)
            .filter(|p| p[3] > 200)
            .all(|p| p[0] > 200 && p[1] == 0));
        // Away from the text it stays transparent.
        assert_eq!(&out[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn odd_text_and_places_do_not_break_anything() {
        for text in ["", "\n\n\n", "日本語 🎬 \u{0}", &"W".repeat(2000)] {
            let title = TitleText {
                text: text.into(),
                x: 1000,
                y: 0,
                size: 500,
                style: TextStyle {
                    outline: 100,
                    shadow: 200,
                    shadow_blur: 100,
                    background: true,
                    tracking: -300,
                    ..Default::default()
                },
                ..Default::default()
            };
            let _ = drawn(&title);
        }
    }

    #[test]
    fn text_plus_draws_outline_shadow_and_box() {
        let plain = TitleText {
            text: "H".into(),
            size: 400,
            ..Default::default()
        };
        let base = drawn(&plain);
        let letters = count(&base, |p| p[3] > 128);
        let outlined = drawn(&TitleText {
            style: TextStyle {
                outline: 20,
                outline_color: [255, 0, 0],
                ..Default::default()
            },
            ..plain.clone()
        });
        assert!(
            count(&outlined, |p| p[3] > 128) > letters,
            "the outline adds a rim"
        );
        assert!(count(&outlined, |p| p[0] > 200 && p[1] < 50 && p[3] > 200) > 0);
        let shadowed = drawn(&TitleText {
            style: TextStyle {
                shadow: 40,
                shadow_opacity: 100,
                shadow_color: [0, 0, 255],
                ..Default::default()
            },
            ..plain.clone()
        });
        assert!(count(&shadowed, |p| p[2] > 200 && p[0] < 50 && p[3] > 200) > 0);
        let boxed = drawn(&TitleText {
            style: TextStyle {
                background: true,
                background_opacity: 100,
                background_color: [0, 255, 0],
                ..Default::default()
            },
            ..plain
        });
        assert!(count(&boxed, |p| p[1] > 200 && p[0] < 50) > letters / 2);
    }

    #[test]
    fn lines_line_up_left_and_right() {
        let mean_x = |align| {
            let out = drawn(&TitleText {
                text: "WWWW\nI".into(),
                size: 150,
                style: TextStyle {
                    align,
                    ..Default::default()
                },
                ..Default::default()
            });
            // The short second line only, below the middle.
            let xs: Vec<usize> = out
                .chunks_exact(4)
                .enumerate()
                .filter(|(i, p)| p[3] > 128 && i / 200 > 50)
                .map(|(i, _)| i % 200)
                .collect();
            xs.iter().sum::<usize>() / xs.len().max(1)
        };
        let (l, c, r) = (
            mean_x(TextAlign::Left),
            mean_x(TextAlign::Center),
            mean_x(TextAlign::Right),
        );
        assert!(l < c && c < r, "{l} {c} {r}");
    }
}
