//! Compositing the tracks of a project into a frame.

use std::fmt;

use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use picasso_core::{MediaItem, MediaSource, PictureWarp, Project, TrackKind};

use crate::cache::MediaCache;
use crate::source;
use crate::temporal;
use crate::transform;
use picasso_core::BlendMode;

/// Largest picture that will be rendered, in pixels.
const MAX_PIXELS: u64 = 100_000_000;

/// Why a frame could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// The size is zero or too big.
    Size,
    Decode(String),
    Encode(String),
    /// An exported frame could not be written.
    Sink(String),
    Cancelled,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size => f.write_str("the picture size is zero or too big"),
            Self::Decode(e) => write!(f, "the image cannot be read: {e}"),
            Self::Encode(e) => write!(f, "the frame cannot be encoded: {e}"),
            Self::Sink(e) => write!(f, "the frame cannot be written: {e}"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A picture: 8 bits per channel, RGBA, straight alpha, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Renders timeline frame `at` of `project` at `width` x `height`.
///
/// Tracks are drawn from the bottom up over black. A muted track, a track whose picture is off
/// and a track without a clip at that frame add nothing.
pub fn render_frame(
    project: &Project,
    cache: &MediaCache,
    at: u64,
    width: u32,
    height: u32,
) -> Result<Frame, RenderError> {
    render_frame_exact(project, cache, at, width, height).map(|(frame, _)| frame)
}

/// Like [`render_frame`], and also says whether every picture in it was final (no video stand-in
/// or placeholder, no image still loading), so the frame may be kept in a cache.
pub fn render_frame_exact(
    project: &Project,
    cache: &MediaCache,
    at: u64,
    width: u32,
    height: u32,
) -> Result<(Frame, bool), RenderError> {
    render_frame_alpha(project, cache, at, width, height, false)
}

/// Like [`render_frame_exact`]; with `alpha`, what no clip covers is left transparent (alpha 0,
/// partly covered pixels partly transparent) instead of black, so a viewer can show a
/// checkerboard behind it.
pub fn render_frame_alpha(
    project: &Project,
    cache: &MediaCache,
    at: u64,
    width: u32,
    height: u32,
    alpha: bool,
) -> Result<(Frame, bool), RenderError> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_PIXELS {
        return Err(RenderError::Size);
    }
    let (w, h) = (width as usize, height as usize);
    let (rgba, exact) = compose_tracks(project, &project.tracks, cache, at, (w, h), alpha, 0);
    Ok((
        Frame {
            width,
            height,
            rgba,
        },
        exact,
    ))
}

/// How many compound clips deep pictures are drawn; deeper ones (or a compound clip that holds
/// itself) are left empty.
const MAX_DEPTH: u32 = 8;

/// Draws frame `at` of `tracks` (a timeline of `project`), as [`render_frame_alpha`] does.
fn compose_tracks(
    project: &Project,
    tracks: &[picasso_core::Track],
    cache: &MediaCache,
    at: u64,
    (w, h): (usize, usize),
    alpha: bool,
    depth: u32,
) -> (Vec<u8>, bool) {
    let mut out = [0u8, 0, 0, 255].repeat(w * h);
    let mut layer = vec![0u8; w * h * 4];
    let mut moved = Vec::new();
    let mut exact = true;
    // How much of each pixel the clips cover, 0 to 1 (only kept with `alpha`).
    let mut coverage = if alpha { vec![0f32; w * h] } else { Vec::new() };
    for track in tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video && !t.muted && t.picture)
    {
        if let Some((left, right, kind, t)) = picasso_core::transition_at(track, at) {
            // Both pictures, then one made of the two.
            // A disabled clip contributes nothing, in a transition as anywhere else.
            let mut first = vec![0u8; w * h * 4];
            if left.enabled {
                exact &= draw_clip(
                    project,
                    left,
                    cache,
                    at,
                    (w, h),
                    depth,
                    &mut first,
                    &mut moved,
                );
            }
            if right.enabled {
                exact &= draw_clip(
                    project,
                    right,
                    cache,
                    at,
                    (w, h),
                    depth,
                    &mut layer,
                    &mut moved,
                );
            } else {
                layer.fill(0);
            }
            mix_transition(&first, &mut layer, kind, t, (w, h));
            let (lo, ro) = (left.opacity_at(at), right.opacity_at(at));
            let opacity = lo + (ro - lo) * t;
            transform::blend(&mut out, &layer, opacity, right.blend);
            for (c, px) in coverage.iter_mut().zip(layer.chunks_exact(4)) {
                let a = f32::from(px[3]) / 255.0 * opacity;
                *c += a * (1.0 - *c);
            }
            continue;
        }
        let Some(clip) = Project::clip_at(track, at) else {
            continue;
        };
        let Some(item) = project.media(clip.media) else {
            continue;
        };
        let opacity = clip.opacity_at(at);
        if opacity <= 0.0 || !clip.enabled {
            continue;
        }
        // An effects clip takes what has been composited below it, applies its own grade and
        // effects, and blends the result back. It has no footprint of its own.
        if matches!(item.source, MediaSource::EffectsClip) {
            layer.copy_from_slice(&out);
            if alpha {
                // What is below is kept over black; straighten it so the grade and the effects
                // see the colours themselves, and hand its coverage on as the alpha.
                for (px, c) in layer.chunks_exact_mut(4).zip(&coverage) {
                    if *c > 0.0 && *c < 1.0 {
                        for v in &mut px[..3] {
                            *v = (f32::from(*v) / c).round().min(255.0) as u8;
                        }
                    }
                    px[3] = (c * 255.0).round() as u8;
                }
            } else {
                for px in layer.chunks_exact_mut(4) {
                    px[3] = 255;
                }
            }
            clip.apply_nodes(&mut layer, w, h);
            picasso_color::apply_effects_at(
                &clip.effects,
                &mut layer,
                w,
                h,
                at,
                project.fps.as_f64() as f32,
            );
            let placed = clip.transform_at(at);
            let warp = clip_warp(clip, at, (w, h));
            if !placed.is_identity() || warp.is_some() {
                moved.resize(w * h * 4, 0);
                transform::apply(&layer, &mut moved, (w, h), &placed, warp.as_ref());
                layer.copy_from_slice(&moved);
            }
            // It replaces the composite below rather than painting over it, so a transform moves
            // the picture instead of leaving a copy of it behind.
            transform::cover(&mut out, &layer, opacity, clip.blend, &mut coverage);
        } else {
            // A compound clip's own timeline is drawn with what it leaves empty transparent.
            exact &= draw_clip(
                project,
                clip,
                cache,
                at,
                (w, h),
                depth,
                &mut layer,
                &mut moved,
            );
            transform::blend(&mut out, &layer, opacity, clip.blend);
            for (c, px) in coverage.iter_mut().zip(layer.chunks_exact(4)) {
                let a = f32::from(px[3]) / 255.0 * opacity;
                *c += a * (1.0 - *c);
            }
        }
    }
    // Drawn over black, the colours are the covered colour times its coverage: dividing by the
    // coverage gives the colour itself, with the coverage as its alpha.
    for (px, c) in out.chunks_exact_mut(4).zip(&coverage) {
        if *c <= 0.0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for v in &mut px[..3] {
            *v = (f32::from(*v) / c).round().min(255.0) as u8;
        }
        px[3] = (c * 255.0).round() as u8;
    }
    (out, exact)
}

/// The stabilization and match-move warp of `clip` at timeline frame `at`, on a picture of
/// `(w, h)` pixels. The clip's frames are counted from its first frame.
fn clip_warp(clip: &picasso_core::Clip, at: u64, (w, h): (usize, usize)) -> Option<PictureWarp> {
    if clip.stabilization.analyzed_transforms.is_empty() && clip.tracking_data.is_empty() {
        return None;
    }
    let frame = at.saturating_sub(clip.start) as i64;
    let aspect = if w == 0 { 1.0 } else { h as f32 / w as f32 };
    picasso_core::warp_for_frame(&clip.stabilization, &clip.tracking_data, frame, aspect)
}

/// Draws the picture of `clip` at timeline frame `at` into `layer`, with its grade and
/// transform. Past either end of the clip (in a transition) it shows the footage beyond it
/// where there is some, else holds its first or last frame. Returns whether it was final.
#[allow(clippy::too_many_arguments)]
fn draw_clip(
    project: &Project,
    clip: &picasso_core::Clip,
    cache: &MediaCache,
    at: u64,
    (w, h): (usize, usize),
    depth: u32,
    layer: &mut [u8],
    moved: &mut Vec<u8>,
) -> bool {
    let Some(item) = project.media(clip.media) else {
        layer.fill(0);
        return true;
    };
    let mut frame = clip.source_frame_at(at);
    if item.length.is_some_and(|l| frame >= l) {
        frame = clip.source_frame_at(clip.end().saturating_sub(1).max(clip.start));
    }
    let mut exact = true;
    if let MediaSource::Compound { timeline } = item.source {
        match project.timeline_tracks(timeline) {
            Some(inner) if depth < MAX_DEPTH => {
                let (nested, ok) =
                    compose_tracks(project, inner, cache, frame, (w, h), true, depth + 1);
                layer.copy_from_slice(&nested);
                exact = ok;
            }
            _ => layer.fill(0),
        }
    } else {
        exact = source::draw(
            item,
            cache,
            (w, h),
            frame_time_us(project, frame),
            project.fps.nominal(),
            layer,
        );
    }
    clip.apply_nodes(layer, w, h);
    apply_clip_effects(project, clip, item, cache, at, (w, h), layer);
    let placed = clip.transform_at(at);
    let warp = clip_warp(clip, at, (w, h));
    if !placed.is_identity() || warp.is_some() {
        moved.resize(w * h * 4, 0);
        transform::apply(layer, moved, (w, h), &placed, warp.as_ref());
        layer.copy_from_slice(moved);
    }
    exact
}

/// Applies the effects of `clip` in order. The ones that read other frames of the clip (Echo,
/// Motion Trails, Smear, Motion Blur, Deflicker...) get those frames drawn here, from the clip's
/// own footage and grade; a frame past either end of the clip is the nearest one it has.
fn apply_clip_effects(
    project: &Project,
    clip: &picasso_core::Clip,
    item: &picasso_core::MediaItem,
    cache: &MediaCache,
    at: u64,
    (w, h): (usize, usize),
    layer: &mut [u8],
) {
    let rate = project.fps.as_f64() as f32;
    let effects = &clip.effects;
    let mut done = 0;
    for (i, e) in effects.iter().enumerate() {
        if !(e.enabled && e.amount > 0.0 && e.kind.is_temporal()) {
            continue;
        }
        picasso_color::apply_effects_at(&effects[done..i], layer, w, h, at, rate);
        done = i + 1;
        let a = e.amount / 100.0;
        let local = at.saturating_sub(clip.start);
        let wanted = temporal::offsets(e.kind, a, local);
        let last = clip.end().saturating_sub(1).max(clip.start);
        let frames: Vec<Vec<u8>> = wanted
            .iter()
            .map(|offset| {
                let frame = (at as i64 + offset).clamp(clip.start as i64, last as i64) as u64;
                if frame == at || matches!(item.source, MediaSource::Compound { .. }) {
                    return layer.to_vec();
                }
                let mut source_frame = clip.source_frame_at(frame);
                if item.length.is_some_and(|l| source_frame >= l) {
                    source_frame = clip.source_frame_at(last);
                }
                let mut picture = vec![0u8; w * h * 4];
                source::draw(
                    item,
                    cache,
                    (w, h),
                    frame_time_us(project, source_frame),
                    project.fps.nominal(),
                    &mut picture,
                );
                clip.apply_nodes(&mut picture, w, h);
                picture
            })
            .collect();
        temporal::apply(e.kind, a, layer, (w, h), local, &frames);
    }
    picasso_color::apply_effects_at(&effects[done..], layer, w, h, at, rate);
}

/// Makes `second` the picture `t` (0 to 1) of the way through a transition from `first`.
fn mix_transition(
    first: &[u8],
    second: &mut [u8],
    kind: picasso_core::TransitionKind,
    t: f32,
    (w, h): (usize, usize),
) {
    use picasso_core::TransitionKind;
    match kind {
        TransitionKind::CrossDissolve => {
            for (b, a) in second.iter_mut().zip(first) {
                *b = (f32::from(*a) * (1.0 - t) + f32::from(*b) * t).round() as u8;
            }
        }
        TransitionKind::DipToBlack => {
            // Down to black by the middle, then up into the second picture.
            let (source, level) = if t < 0.5 {
                (Some(first), 1.0 - t * 2.0)
            } else {
                (None, t * 2.0 - 1.0)
            };
            if let Some(first) = source {
                second.copy_from_slice(first);
            }
            for px in second.chunks_exact_mut(4) {
                for v in &mut px[..3] {
                    *v = (f32::from(*v) * level).round() as u8;
                }
            }
        }
        // The more transitions are drawn pixel by pixel.
        other => crate::transitions::mix_into(other, first, second, w, h, t),
    }
}

/// Where frame `frame` of the source starts, in microseconds, at the project's frame rate.
fn frame_time_us(project: &Project, frame: u64) -> i64 {
    let fps = project.fps;
    let us = u128::from(frame) * 1_000_000 * u128::from(fps.den) / u128::from(fps.num.max(1));
    i64::try_from(us).unwrap_or(i64::MAX)
}

/// A small picture of one media item over black, for the media pool. A compound clip shows
/// the first frame of its timeline, which is in `project`.
pub fn render_thumbnail(
    project: &Project,
    item: &MediaItem,
    cache: &MediaCache,
    width: u32,
    height: u32,
) -> Result<Frame, RenderError> {
    render_thumbnail_exact(project, item, cache, width, height).map(|(frame, _)| frame)
}

/// Like [`render_thumbnail`], and also says whether the picture is final (see
/// [`render_frame_exact`]). It never waits for a video: a picture still being decoded shows
/// what is there, so the caller can make the thumbnail again once the real one arrives.
pub fn render_thumbnail_exact(
    project: &Project,
    item: &MediaItem,
    cache: &MediaCache,
    width: u32,
    height: u32,
) -> Result<(Frame, bool), RenderError> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_PIXELS {
        return Err(RenderError::Size);
    }
    let (w, h) = (width as usize, height as usize);
    let mut out = [0u8, 0, 0, 255].repeat(w * h);
    let mut layer = vec![0u8; w * h * 4];
    let mut exact = true;
    if let MediaSource::Compound { timeline } = item.source {
        if let Some(inner) = project.timeline_tracks(timeline) {
            let (nested, ok) = compose_tracks(project, inner, cache, 0, (w, h), true, 1);
            layer.copy_from_slice(&nested);
            exact = ok;
        }
    } else {
        exact = source::draw(item, cache, (w, h), 0, project.fps.nominal(), &mut layer);
    }
    transform::blend(&mut out, &layer, 1.0, BlendMode::Normal);
    Ok((
        Frame {
            width,
            height,
            rgba: out,
        },
        exact,
    ))
}

/// The picture of one media item at frame `frame` of the media (for the source viewer). A
/// video picture still being decoded shows the nearest one, or nothing yet.
pub fn render_media_frame(
    project: &Project,
    item: &MediaItem,
    cache: &MediaCache,
    frame: u64,
    width: u32,
    height: u32,
) -> Result<Frame, RenderError> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_PIXELS {
        return Err(RenderError::Size);
    }
    let (w, h) = (width as usize, height as usize);
    let mut out = [0u8, 0, 0, 255].repeat(w * h);
    let mut layer = vec![0u8; w * h * 4];
    if let MediaSource::Compound { timeline } = item.source {
        if let Some(inner) = project.timeline_tracks(timeline) {
            let (nested, _) = compose_tracks(project, inner, cache, frame, (w, h), true, 1);
            layer.copy_from_slice(&nested);
        }
    } else {
        source::draw(
            item,
            cache,
            (w, h),
            frame_time_us(project, frame),
            project.fps.nominal(),
            &mut layer,
        );
    }
    transform::blend(&mut out, &layer, 1.0, BlendMode::Normal);
    Ok(Frame {
        width,
        height,
        rgba: out,
    })
}

/// Encodes a frame as a PNG file.
pub fn encode_png(frame: &Frame) -> Result<Vec<u8>, RenderError> {
    let expected = u64::from(frame.width) * u64::from(frame.height) * 4;
    if frame.rgba.len() as u64 != expected {
        return Err(RenderError::Encode(
            "the buffer does not match the size".into(),
        ));
    }
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            &frame.rgba,
            frame.width,
            frame.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|e| RenderError::Encode(e.to_string()))?;
    Ok(bytes)
}

/// Renders the frames `start..end` at `(width, height)` and hands each as PNG bytes to `sink` with its frame number.
///
/// `progress` is called after each frame with the number done and the total; it returns `false`
/// to stop (the result is then [`RenderError::Cancelled`]).
pub fn export_sequence(
    project: &Project,
    cache: &MediaCache,
    start: u64,
    end: u64,
    (width, height): (u32, u32),
    mut sink: impl FnMut(u64, Vec<u8>) -> Result<(), String>,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<u64, RenderError> {
    let total = end.saturating_sub(start);
    // Every exported frame must be the exact picture, so wait for the decoder.
    let cache = &cache.blocking();
    for (done, frame_no) in (start..end).enumerate() {
        let frame = render_frame(project, cache, frame_no, width, height)?;
        sink(frame_no, encode_png(&frame)?).map_err(RenderError::Sink)?;
        if !progress(done as u64 + 1, total) {
            return Err(RenderError::Cancelled);
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {

    #[test]
    fn keyframed_opacity_fades_the_picture_in() {
        let mut p = Project::demo();
        let a = p.tracks[0].clips[0].id;
        for (at, v) in [(0, 0.0), (40, 1.0)] {
            picasso_core::Command::SetKeyframe {
                clip: a,
                param: picasso_core::KeyParam::Opacity,
                at,
                value: Some(v),
                ease: false,
            }
            .apply(&mut p)
            .unwrap();
        }
        let cache = MediaCache::new();
        let bright = |f: u64| -> u64 {
            render_frame(&p, &cache, f, 16, 16)
                .unwrap()
                .rgba
                .iter()
                .map(|v| u64::from(*v))
                .sum()
        };
        let (dark, half, full) = (bright(0), bright(20), bright(40));
        assert!(dark < half && half < full, "{dark} {half} {full}");
    }

    use super::*;
    use crate::cache::Decoded;
    use picasso_color::ColorGrade;
    use picasso_core::{Command, MediaSource, Outcome};

    fn px(f: &Frame, x: usize, y: usize) -> [u8; 4] {
        let i = (y * f.width as usize + x) * 4;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
    }

    /// An empty project with a solid-colour media item, and its track ids.
    fn project_with(rgb: [u8; 3]) -> (Project, picasso_core::MediaId) {
        let mut p = Project::new("t");
        let Outcome::Media(m) = (Command::AddMedia {
            name: "c".into(),
            source: MediaSource::Solid { rgb },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        (p, m)
    }

    fn place(
        p: &mut Project,
        track: usize,
        media: picasso_core::MediaId,
        start: u64,
    ) -> picasso_core::ClipId {
        let t = p.tracks[track].id;
        let Outcome::Clip(c) = (Command::AddClip {
            track: t,
            media,
            start,
            duration: None,
            with_sound: false,
        })
        .apply(p)
        .unwrap() else {
            panic!()
        };
        c
    }

    #[test]
    fn a_title_comes_in_over_its_animation_and_then_stays() {
        let mut p = Project::new("t");
        let title = picasso_core::TitleText {
            text: "Hi".into(),
            size: 300,
            style: picasso_core::TextStyle {
                animation: picasso_core::TitleAnimation::Fade,
                animation_ms: 1000,
                ..Default::default()
            },
            ..Default::default()
        };
        let Outcome::Media(m) = (Command::AddMedia {
            name: "t".into(),
            source: MediaSource::Title(title),
            length: None,
            default_duration: 120,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        place(&mut p, 0, m, 0);
        let light = |frame| {
            let f = render_frame(&p, &MediaCache::new(), frame, 160, 90).unwrap();
            f.rgba.chunks_exact(4).map(|px| u32::from(px[1])).sum::<u32>()
        };
        let fps = p.fps.nominal();
        let (start, middle, end, later) = (light(0), light(fps / 2), light(fps), light(fps * 2));
        assert_eq!(start, 0, "the first frame is empty");
        assert!(middle > start && end > middle, "{start} {middle} {end}");
        assert_eq!(end, later, "after its animation the title stays as it is");
    }

    #[test]
    fn empty_timeline_is_black() {
        let (p, _) = project_with([9, 9, 9]);
        let f = render_frame(&p, &MediaCache::new(), 0, 4, 2).unwrap();
        assert!(f.rgba.chunks_exact(4).all(|px| px == [0, 0, 0, 255]));
    }

    #[test]
    fn a_clip_shows_only_inside_its_frames() {
        let (mut p, m) = project_with([10, 200, 30]);
        place(&mut p, 0, m, 5);
        let cache = MediaCache::new();
        assert_eq!(
            px(&render_frame(&p, &cache, 4, 2, 2).unwrap(), 0, 0),
            [0, 0, 0, 255]
        );
        assert_eq!(
            px(&render_frame(&p, &cache, 5, 2, 2).unwrap(), 1, 1),
            [10, 200, 30, 255]
        );
        assert_eq!(
            px(&render_frame(&p, &cache, 14, 2, 2).unwrap(), 0, 0),
            [10, 200, 30, 255]
        );
        assert_eq!(
            px(&render_frame(&p, &cache, 15, 2, 2).unwrap(), 0, 0),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn a_tracked_point_is_held_still_on_an_untransformed_clip() {
        let (mut p, m) = project_with([10, 200, 30]);
        let clip = place(&mut p, 0, m, 0);
        // The point has moved half the picture width to the right by this frame, so the picture
        // is drawn half a width to the left to hold it where it was: the left half shows the
        // footage, the right half is empty.
        Command::SetTrackingData {
            clip,
            tracking_data: vec![picasso_core::TrackData {
                analyzed_frames: vec![picasso_core::FrameTransform {
                    frame_number: 0,
                    matrix: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                }],
                ..Default::default()
            }],
        }
        .apply(&mut p)
        .unwrap();
        let f = render_frame(&p, &MediaCache::new(), 0, 4, 2).unwrap();
        assert_eq!(px(&f, 0, 0), [10, 200, 30, 255]);
        assert_eq!(px(&f, 3, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn upper_tracks_cover_lower_ones_and_opacity_blends() {
        let (mut p, red) = project_with([200, 0, 0]);
        let Outcome::Media(blue) = (Command::AddMedia {
            name: "b".into(),
            source: MediaSource::Solid { rgb: [0, 0, 100] },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        place(&mut p, 0, red, 0);
        let top = place(&mut p, 1, blue, 0);
        let cache = MediaCache::new();
        assert_eq!(
            px(&render_frame(&p, &cache, 0, 2, 2).unwrap(), 0, 0),
            [0, 0, 100, 255]
        );
        Command::SetOpacity {
            clip: top,
            opacity: 0.5,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(&render_frame(&p, &cache, 0, 2, 2).unwrap(), 0, 0),
            [100, 0, 50, 255]
        );
        Command::SetOpacity {
            clip: top,
            opacity: 0.0,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(&render_frame(&p, &cache, 0, 2, 2).unwrap(), 0, 0),
            [200, 0, 0, 255]
        );
    }

    #[test]
    fn a_green_screen_clip_shows_the_track_below_through_it() {
        let (mut p, under) = project_with([200, 0, 0]);
        let Outcome::Media(screen) = (Command::AddMedia {
            name: "screen".into(),
            source: MediaSource::Solid { rgb: [0, 177, 64] },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        place(&mut p, 0, under, 0);
        let top = place(&mut p, 1, screen, 0);
        let cache = MediaCache::new();
        assert_eq!(
            px(&render_frame(&p, &cache, 0, 2, 2).unwrap(), 0, 0),
            [0, 177, 64, 255]
        );
        Command::AddEffect {
            clip: top,
            kind: picasso_core::EffectKind::GreenScreen,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(&render_frame(&p, &cache, 0, 2, 2).unwrap(), 0, 0),
            [200, 0, 0, 255],
            "the screen is keyed out"
        );
    }

    #[test]
    fn muted_tracks_are_skipped() {
        let (mut p, m) = project_with([50, 60, 70]);
        place(&mut p, 0, m, 0);
        let t = p.tracks[0].id;
        Command::SetTrackMuted {
            track: t,
            muted: true,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(
                &render_frame(&p, &MediaCache::new(), 0, 2, 2).unwrap(),
                0,
                0
            ),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn tracks_whose_picture_is_off_are_skipped() {
        let (mut p, m) = project_with([50, 60, 70]);
        place(&mut p, 0, m, 0);
        let t = p.tracks[0].id;
        Command::SetTrackPicture {
            track: t,
            picture: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(
                &render_frame(&p, &MediaCache::new(), 0, 2, 2).unwrap(),
                0,
                0
            ),
            [0, 0, 0, 255]
        );
        // The sound is not what the picture switch turns off.
        Command::SetTrackPicture {
            track: t,
            picture: true,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(
                &render_frame(&p, &MediaCache::new(), 0, 2, 2).unwrap(),
                0,
                0
            ),
            [50, 60, 70, 255]
        );
    }

    #[test]
    fn the_grade_of_the_clip_is_applied() {
        let (mut p, m) = project_with([100, 100, 100]);
        let c = place(&mut p, 0, m, 0);
        let grade = ColorGrade {
            gain: [2.0, 1.0, 0.5],
            ..Default::default()
        };
        Command::SetGrade {
            clip: c,
            grade,
            node: 0,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(
                &render_frame(&p, &MediaCache::new(), 0, 2, 2).unwrap(),
                0,
                0
            ),
            [200, 100, 50, 255]
        );
    }

    #[test]
    fn with_alpha_what_no_clip_covers_is_transparent() {
        let (mut p, m) = project_with([200, 100, 50]);
        let v1 = p.tracks[0].id;
        Command::AddClip {
            track: v1,
            media: m,
            start: 0,
            duration: Some(10),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        let cache = MediaCache::new();
        let (covered, _) = render_frame_alpha(&p, &cache, 0, 4, 4, true).unwrap();
        assert_eq!(px(&covered, 1, 1), [200, 100, 50, 255]);
        let (empty, _) = render_frame_alpha(&p, &cache, 20, 4, 4, true).unwrap();
        assert_eq!(px(&empty, 1, 1), [0, 0, 0, 0]);
        let id = p.tracks[0].clips[0].id;
        Command::SetOpacity {
            clip: id,
            opacity: 0.5,
        }
        .apply(&mut p)
        .unwrap();
        let (half, _) = render_frame_alpha(&p, &cache, 0, 4, 4, true).unwrap();
        let [r, g, b, a] = px(&half, 1, 1);
        assert!(
            a.abs_diff(128) <= 1
                && r.abs_diff(200) <= 2
                && g.abs_diff(100) <= 2
                && b.abs_diff(50) <= 2
        );
        // Without alpha, the same frame is over black, as exports want.
        assert_eq!(
            px(&render_frame(&p, &cache, 20, 4, 4).unwrap(), 1, 1),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn a_dissolve_mixes_the_two_pictures_around_the_cut() {
        let (mut p, red) = project_with([200, 0, 0]);
        let Ok(picasso_core::Outcome::Media(blue)) = (Command::AddMedia {
            name: "b".into(),
            source: MediaSource::Solid { rgb: [0, 0, 200] },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p) else {
            panic!()
        };
        let v1 = p.tracks[0].id;
        for (media, start) in [(red, 0), (blue, 10)] {
            Command::AddClip {
                track: v1,
                media,
                start,
                duration: Some(10),
                with_sound: false,
            }
            .apply(&mut p)
            .unwrap();
        }
        let second = p.tracks[0].clips[1].id;
        Command::SetTransition {
            clip: second,
            transition: Some(picasso_core::Transition {
                kind: picasso_core::TransitionKind::CrossDissolve,
                frames: 4,
            }),
        }
        .apply(&mut p)
        .unwrap();
        let cache = MediaCache::new();
        let at = |p: &Project, f| px(&render_frame(p, &cache, f, 2, 2).unwrap(), 0, 0);
        assert_eq!(at(&p, 7), [200, 0, 0, 255], "before the transition");
        let [r, _, b, _] = at(&p, 9);
        assert!(
            r > b && b > 0,
            "early on, mostly the first picture: {r} {b}"
        );
        let [r, _, b, _] = at(&p, 11);
        assert!(b > r && r > 0, "late on, mostly the second: {r} {b}");
        assert_eq!(at(&p, 12), [0, 0, 200, 255], "after it");
        Command::SetTransition {
            clip: second,
            transition: Some(picasso_core::Transition {
                kind: picasso_core::TransitionKind::DipToBlack,
                frames: 4,
            }),
        }
        .apply(&mut p)
        .unwrap();
        let [r, _, b, _] = at(&p, 9);
        assert!(r < 200 && b == 0, "the first picture going dark: {r}");
        // A split clip keeps no transition at the cut.
        Command::Split {
            clip: second,
            at: 15,
        }
        .apply(&mut p)
        .unwrap();
        assert!(p.tracks[0].clips[2].transition.is_none());
    }

    #[test]
    fn the_extra_transitions_play_between_two_clips_and_leave_the_ends_alone() {
        for kind in picasso_core::TransitionKind::extra()
            .iter()
            .chain(picasso_core::TransitionKind::wipes())
            .chain(picasso_core::TransitionKind::looks())
        {
            let (mut p, m) = project_with([200, 0, 0]);
            let v1 = p.tracks[0].id;
            let Outcome::Media(blue) = (Command::AddMedia {
                name: "b".into(),
                source: MediaSource::Solid { rgb: [0, 0, 200] },
                length: None,
                default_duration: 10,
            })
            .apply(&mut p)
            .unwrap() else {
                panic!()
            };
            for (media, start) in [(m, 0), (blue, 10)] {
                Command::AddClip {
                    track: v1,
                    media,
                    start,
                    duration: Some(10),
                    with_sound: false,
                }
                .apply(&mut p)
                .unwrap();
            }
            let second = p.tracks[0].clips[1].id;
            Command::SetTransition {
                clip: second,
                transition: Some(picasso_core::Transition {
                    kind: *kind,
                    frames: 8,
                }),
            }
            .apply(&mut p)
            .unwrap();
            let cache = MediaCache::new();
            let frame = |f| render_frame(&p, &cache, f, 64, 36).unwrap().rgba;
            assert_eq!(&frame(5)[..4], [200, 0, 0, 255], "{kind:?} before");
            assert_eq!(&frame(15)[..4], [0, 0, 200, 255], "{kind:?} after");
            // Every frame of it is a whole, opaque picture (what each does to a picture with
            // something in it is checked in `transitions`).
            for f in 6..14 {
                let rgba = frame(f);
                assert_eq!(rgba.len(), 64 * 36 * 4);
                assert!(
                    rgba.chunks_exact(4).all(|px| px[3] == 255),
                    "{kind:?} frame {f}"
                );
            }
        }
    }

    #[test]
    fn a_compound_clip_shows_its_clips() {
        let (mut p, m) = project_with([10, 200, 30]);
        let v1 = p.tracks[0].id;
        let Ok(picasso_core::Outcome::Clip(inner)) = (Command::AddClip {
            track: v1,
            media: m,
            start: 0,
            duration: Some(10),
            with_sound: false,
        })
        .apply(&mut p) else {
            panic!()
        };
        let Ok(picasso_core::Outcome::Clip(compound)) = (Command::CreateCompound {
            clips: vec![inner],
            name: "C".into(),
        })
        .apply(&mut p) else {
            panic!()
        };
        let cache = MediaCache::new();
        assert_eq!(
            px(&render_frame(&p, &cache, 5, 4, 4).unwrap(), 1, 1),
            [10, 200, 30, 255]
        );
        // Half opacity on the compound clip: half of its picture over black.
        Command::SetOpacity {
            clip: compound,
            opacity: 0.5,
        }
        .apply(&mut p)
        .unwrap();
        let [_, g, _, _] = px(&render_frame(&p, &cache, 5, 4, 4).unwrap(), 1, 1);
        assert!(g.abs_diff(100) <= 1, "{g}");
        let item = p.media.last().unwrap().clone();
        let thumb = render_thumbnail(&p, &item, &cache, 4, 4).unwrap();
        assert_eq!(px(&thumb, 1, 1), [10, 200, 30, 255]);
    }

    #[test]
    fn a_disabled_clip_stays_out_of_a_dissolve() {
        let (mut p, red) = project_with([200, 0, 0]);
        let Outcome::Media(blue) = (Command::AddMedia {
            name: "b".into(),
            source: MediaSource::Solid { rgb: [0, 0, 200] },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let v1 = p.tracks[0].id;
        for (media, start) in [(red, 0), (blue, 10)] {
            Command::AddClip {
                track: v1,
                media,
                start,
                duration: Some(10),
                with_sound: false,
            }
            .apply(&mut p)
            .unwrap();
        }
        let first = p.tracks[0].clips[0].id;
        let second = p.tracks[0].clips[1].id;
        Command::SetTransition {
            clip: second,
            transition: Some(picasso_core::Transition {
                kind: picasso_core::TransitionKind::CrossDissolve,
                frames: 4,
            }),
        }
        .apply(&mut p)
        .unwrap();
        Command::SetClipEnabled {
            clip: first,
            enabled: false,
        }
        .apply(&mut p)
        .unwrap();
        let cache = MediaCache::new();
        // Frame 9 is inside the dissolve, which starts two frames before the cut at 10.
        let [r, _, b, _] = px(&render_frame(&p, &cache, 9, 2, 2).unwrap(), 0, 0);
        assert_eq!(r, 0, "the disabled clip does not show in the dissolve");
        assert!(b > 0, "the enabled clip does");
    }

    #[test]
    fn letterbox_bars_are_black_not_transparent() {
        let mut p = Project::new("t");
        let Outcome::Media(m) = (Command::AddMedia {
            name: "i".into(),
            source: MediaSource::Image {
                path: "x.png".into(),
            },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        place(&mut p, 0, m, 0);
        let mut cache = MediaCache::new();
        cache.insert(
            m,
            Decoded {
                width: 2,
                height: 1,
                rgba: vec![255, 255, 255, 255, 255, 255, 255, 255],
            },
        );
        let f = render_frame(&p, &cache, 0, 8, 8).unwrap();
        assert_eq!(px(&f, 0, 0), [0, 0, 0, 255]);
        assert_eq!(px(&f, 4, 4), [255, 255, 255, 255]);
    }

    /// A video source that paints the time it was asked for into the red channel (in tenths of
    /// a second) so a test can see which time the compositor asked for.
    #[derive(Debug)]
    struct Clock;

    impl crate::FrameSource for Clock {
        fn video_frame(
            &self,
            _: picasso_core::MediaId,
            _: &str,
            time_us: i64,
            _: u32,
            _: bool,
        ) -> crate::VideoFrame {
            let tenths = u8::try_from(time_us / 100_000).unwrap_or(255);
            crate::VideoFrame::Ready(std::sync::Arc::new(Decoded {
                width: 1,
                height: 1,
                rgba: vec![tenths, 0, 0, 255],
            }))
        }
    }

    #[test]
    fn a_video_clip_asks_for_the_time_inside_the_source() {
        let mut p = Project::new("t");
        let Outcome::Media(m) = (Command::AddMedia {
            name: "v".into(),
            source: MediaSource::Video {
                path: "v.mp4".into(),
            },
            length: Some(1000),
            default_duration: 100,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let clip = place(&mut p, 0, m, 50);
        Command::TrimStart { clip, start: 60 }.apply(&mut p).ok();
        let cache = MediaCache::with_video_source(std::sync::Arc::new(Clock));
        // The project runs at 24 fps. Timeline frame 70 is 10 frames into the visible part of
        // the clip, which starts at source frame `source_in`.
        let c = p.tracks[0].clips[0].clone();
        let f = render_frame(&p, &cache, 70, 2, 2).unwrap();
        let source_frame = c.source_in + (70 - c.start);
        let expected = (source_frame * 10 / 24) as u8; // tenths of a second
        assert_eq!(px(&f, 0, 0), [expected, 0, 0, 255]);
        // Without a video source the clip shows the offline picture, not a crash.
        let f = render_frame(&p, &MediaCache::new(), 70, 48, 48).unwrap();
        assert_eq!(px(&f, 0, 0), [200, 0, 200, 255]);
    }

    #[test]
    fn thumbnails_show_the_media_over_black() {
        let (p, m) = project_with([5, 6, 7]);
        let item = p.media(m).unwrap();
        let t = render_thumbnail(&p, item, &MediaCache::new(), 3, 2).unwrap();
        assert_eq!(px(&t, 2, 1), [5, 6, 7, 255]);
        assert_eq!(
            render_thumbnail(&p, item, &MediaCache::new(), 0, 2),
            Err(RenderError::Size)
        );
    }

    #[test]
    fn bad_sizes_are_refused() {
        let (p, _) = project_with([0; 3]);
        let c = MediaCache::new();
        assert_eq!(render_frame(&p, &c, 0, 0, 10), Err(RenderError::Size));
        assert_eq!(render_frame(&p, &c, 0, 10, 0), Err(RenderError::Size));
        assert_eq!(
            render_frame(&p, &c, 0, u32::MAX, u32::MAX),
            Err(RenderError::Size)
        );
    }

    #[test]
    fn png_round_trip() {
        let (mut p, m) = project_with([12, 34, 56]);
        place(&mut p, 0, m, 0);
        let frame = render_frame(&p, &MediaCache::new(), 0, 6, 4).unwrap();
        let png = encode_png(&frame).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let back = Decoded::from_bytes(&png).unwrap();
        assert_eq!((back.width, back.height, back.rgba), (6, 4, frame.rgba));
        assert!(encode_png(&Frame {
            width: 3,
            height: 3,
            rgba: vec![0; 4]
        })
        .is_err());
    }

    #[test]
    fn export_sequence_writes_every_frame_and_can_be_cancelled() {
        let (mut p, m) = project_with([1, 2, 3]);
        place(&mut p, 0, m, 0);
        let cache = MediaCache::new();
        let mut written = Vec::new();
        let n = export_sequence(
            &p,
            &cache,
            2,
            6,
            (4, 4),
            |no, bytes| {
                assert!(!bytes.is_empty());
                written.push(no);
                Ok(())
            },
            |_, _| true,
        )
        .unwrap();
        assert_eq!((n, written), (4, vec![2, 3, 4, 5]));

        let mut count = 0;
        let cancelled = export_sequence(
            &p,
            &cache,
            0,
            10,
            (4, 4),
            |_, _| {
                count += 1;
                Ok(())
            },
            |done, total| {
                assert_eq!(total, 10);
                done < 3
            },
        );
        assert_eq!(cancelled, Err(RenderError::Cancelled));
        assert_eq!(count, 3);

        let failed = export_sequence(
            &p,
            &cache,
            0,
            3,
            (4, 4),
            |_, _| Err("disk full".into()),
            |_, _| true,
        );
        assert_eq!(failed, Err(RenderError::Sink("disk full".into())));
        assert_eq!(
            export_sequence(&p, &cache, 5, 5, (4, 4), |_, _| Ok(()), |_, _| true),
            Ok(0)
        );
    }

    #[test]
    fn a_disabled_clip_is_not_drawn() {
        let (mut p, m) = project_with([90, 80, 70]);
        let c = place(&mut p, 0, m, 0);
        Command::SetClipEnabled {
            clip: c,
            enabled: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(
                &render_frame(&p, &MediaCache::new(), 0, 2, 2).unwrap(),
                0,
                0
            ),
            [0, 0, 0, 255]
        );
        Command::SetClipEnabled {
            clip: c,
            enabled: true,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            px(
                &render_frame(&p, &MediaCache::new(), 0, 2, 2).unwrap(),
                0,
                0
            ),
            [90, 80, 70, 255]
        );
    }

    #[test]
    fn a_clips_transform_and_blend_apply_when_it_is_drawn() {
        let (mut p, red) = project_with([200, 0, 0]);
        let blue = {
            let Outcome::Media(m) = (Command::AddMedia {
                name: "b".into(),
                source: MediaSource::Solid { rgb: [0, 0, 100] },
                length: None,
                default_duration: 10,
            })
            .apply(&mut p)
            .unwrap() else {
                panic!()
            };
            m
        };
        place(&mut p, 0, red, 0);
        let top = place(&mut p, 1, blue, 0);
        // A quarter-size blue square in the middle over red.
        Command::SetClipTransform {
            clip: top,
            transform: picasso_core::Transform {
                scale_x: 0.5,
                scale_y: 0.5,
                ..Default::default()
            },
        }
        .apply(&mut p)
        .unwrap();
        let f = render_frame(&p, &MediaCache::new(), 0, 16, 16).unwrap();
        assert_eq!(px(&f, 0, 0)[..3], [200, 0, 0], "the corner is red");
        assert_eq!(px(&f, 8, 8)[..3], [0, 0, 100], "the middle is blue");
        // Added, the middle is both.
        Command::SetBlend {
            clip: top,
            blend: BlendMode::Add,
        }
        .apply(&mut p)
        .unwrap();
        let f = render_frame(&p, &MediaCache::new(), 0, 16, 16).unwrap();
        assert_eq!(px(&f, 8, 8)[..3], [200, 0, 100]);
        assert_eq!(px(&f, 0, 0)[..3], [200, 0, 0]);
    }

    #[test]
    fn a_fast_clip_asks_for_later_footage() {
        let mut p = Project::new("t");
        let Outcome::Media(m) = (Command::AddMedia {
            name: "v".into(),
            source: MediaSource::Video {
                path: "v.mp4".into(),
            },
            length: Some(1000),
            default_duration: 100,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let clip = place(&mut p, 0, m, 0);
        Command::SetClipSpeed { clip, speed: 2.0 }
            .apply(&mut p)
            .unwrap();
        let cache = MediaCache::with_video_source(std::sync::Arc::new(Clock));
        // At 24 fps and twice the speed, timeline frame 12 shows footage frame 24: one second in.
        let f = render_frame(&p, &cache, 12, 2, 2).unwrap();
        assert_eq!(px(&f, 0, 0)[0], 10, "one second is ten tenths");
    }

    #[test]
    fn an_effects_clip_applies_its_effect_to_clips_below() {
        let (mut p, red) = project_with([200, 0, 0]);
        // Red clip on V1.
        place(&mut p, 0, red, 0);
        // An effects clip on V2, above red.
        let Outcome::Clip(effects) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        // An invert effect on the effects clip: 200 -> 55, 0 -> 255.
        Command::AddEffect {
            clip: effects,
            kind: picasso_core::EffectKind::Invert,
        }
        .apply(&mut p)
        .unwrap();
        let cache = MediaCache::new();
        let f = render_frame(&p, &cache, 0, 2, 2).unwrap();
        // Red 200 inverts to ~55; the empty blue channel inverts to 255.
        assert_eq!(f.rgba[0], 55, "red channel inverted");
        assert_eq!(f.rgba[2], 255, "blue channel inverted");
    }

    #[test]
    fn an_effects_clip_with_no_effect_leaves_clips_below_unchanged() {
        let (mut p, red) = project_with([200, 0, 0]);
        place(&mut p, 0, red, 0);
        let Outcome::Clip(effects) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let _ = effects;
        let cache = MediaCache::new();
        let f = render_frame(&p, &cache, 0, 2, 2).unwrap();
        assert_eq!(px(&f, 0, 0), [200, 0, 0, 255]);
    }

    #[test]
    fn an_effects_clip_moves_the_composite_instead_of_doubling_it() {
        let (mut p, red) = project_with([200, 0, 0]);
        place(&mut p, 0, red, 0);
        let Outcome::Clip(effects) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        // Moved half the frame to the right: the left half is empty, not the picture again.
        Command::SetClipTransform {
            clip: effects,
            transform: picasso_core::Transform {
                x: 0.5,
                ..picasso_core::Transform::default()
            },
        }
        .apply(&mut p)
        .unwrap();
        let f = render_frame(&p, &MediaCache::new(), 0, 4, 2).unwrap();
        assert_eq!(px(&f, 0, 0), [0, 0, 0, 255], "the gap is empty");
        assert_eq!(px(&f, 3, 0), [200, 0, 0, 255], "the picture moved");
    }

    #[test]
    fn an_effects_clip_with_no_grade_changes_nothing_in_the_alpha_viewer() {
        let (mut p, red) = project_with([200, 0, 0]);
        let clip = place(&mut p, 0, red, 0);
        // Half transparent: the viewer keeps it over black until the coverage is divided out.
        Command::SetOpacity { clip, opacity: 0.5 }
            .apply(&mut p)
            .unwrap();
        let cache = MediaCache::new();
        let before = render_frame_alpha(&p, &cache, 0, 2, 2, true).unwrap().0;
        let Outcome::Clip(_effects) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let after = render_frame_alpha(&p, &cache, 0, 2, 2, true).unwrap().0;
        let (was, now) = (px(&before, 0, 0), px(&after, 0, 0));
        assert_eq!(now[3], was[3], "the coverage stays");
        for c in 0..3 {
            assert!(
                i16::from(now[c]).abs_diff(i16::from(was[c])) <= 1,
                "a clip with no grade leaves the colour alone: {was:?} {now:?}"
            );
        }
    }

    #[test]
    fn an_effects_clip_does_not_make_its_own_timeline() {
        let (mut p, _red) = project_with([200, 0, 0]);
        let before = p.timelines.len();
        let Outcome::Clip(_) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(p.timelines.len(), before, "no timeline added");
    }

    /// A clip of changing noise (a new picture every frame) with `kind` on it.
    fn noisy_clip_with(kind: picasso_color::EffectKind) -> Project {
        let mut p = Project::new("t");
        let Outcome::Media(m) = (Command::AddMedia {
            name: "n".into(),
            source: MediaSource::Generator {
                pattern: picasso_core::GeneratorKind::Noise,
                colors: [[0; 3]; 4],
            },
            length: None,
            default_duration: 30,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        place(&mut p, 0, m, 0);
        if let Some(clip) = p.tracks[0].clips.first_mut() {
            clip.effects.push(picasso_color::Effect::new(kind));
        }
        p
    }

    #[test]
    fn effects_that_read_other_frames_get_them_from_the_clip() {
        use picasso_color::EffectKind as K;
        let cache = MediaCache::new();
        let plain = {
            let mut p = noisy_clip_with(K::Echo);
            if let Some(c) = p.tracks[0].clips.first_mut() {
                c.effects.clear();
            }
            p
        };
        let frame = |p: &Project, at: u64| render_frame(p, &cache, at, 64, 36).unwrap().rgba;
        for kind in [
            K::Echo,
            K::MotionTrails,
            K::Smear,
            K::FrameReplacerPrevious,
            K::FrameReplacerNext,
            K::FrameReplacerMix,
        ] {
            let p = noisy_clip_with(kind);
            assert_ne!(frame(&p, 12), frame(&plain, 12), "{kind:?} did nothing");
            // The same frame draws the same way every time.
            assert_eq!(frame(&p, 12), frame(&p, 12), "{kind:?}");
        }
        // Strobe holds three frames: 12, 13 and 14 are one picture, 15 the next.
        let p = noisy_clip_with(K::Strobe);
        assert_eq!(frame(&p, 12), frame(&p, 13));
        assert_eq!(frame(&p, 12), frame(&p, 14));
        assert_ne!(frame(&p, 12), frame(&p, 15));
        // The rest run on the first and last frames of the clip, where a neighbour is missing.
        for kind in [
            K::MotionBlur,
            K::Deflicker,
            K::ColorStabilizer,
            K::DirtRemoval,
        ] {
            let p = noisy_clip_with(kind);
            for at in [0, 12, 29] {
                assert_eq!(frame(&p, at).len(), 64 * 36 * 4, "{kind:?} at {at}");
            }
        }
    }
}
