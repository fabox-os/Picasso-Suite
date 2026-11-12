//! The viewer: the picture at the playhead.

use egui::{pos2, vec2, Color32, ColorImage, Context, Rect, TextureHandle, TextureOptions, Ui};
use picasso_core::{MediaId, Project, TrackKind};

use crate::actions::Overlays;
use std::sync::Arc;

use crate::composer::{Composer, Job};
use crate::render_cache::RenderCache;
use picasso_render::{Frame, FrameKey, MediaCache};

/// The longest side of the picture the viewer renders; exports render at the project's size.
const PREVIEW_SIDE: u32 = 960;

/// What the rendered picture depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewerKey {
    pub epoch: u64,
    pub revision: u64,
    pub media_generation: u64,
    /// Changes when a video picture arrives from the decoder.
    pub video: u64,
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    /// Whether empty parts are rendered transparent (the checkerboard background).
    pub alpha: bool,
    /// The content of the highlighted copy of the project, when the key is shown.
    pub highlight: u64,
}

/// The size of the preview of a project: its shape, at most [`PREVIEW_SIDE`] on the long side.
pub(crate) fn preview_size(project: &Project) -> (u32, u32) {
    let (w, h) = (project.width.max(1), project.height.max(1));
    let long = w.max(h);
    if long <= PREVIEW_SIDE {
        return (w, h);
    }
    let scale = PREVIEW_SIDE as f32 / long as f32;
    (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}

#[derive(Default)]
pub(crate) struct Viewer {
    texture: Option<TextureHandle>,
    key: Option<ViewerKey>,
    /// The picture that was last rendered, for the scopes.
    pub frame: Option<Arc<Frame>>,
    /// Counts the pictures rendered, so a scope knows when to measure again.
    pub generation: u64,
    /// Puts the pictures together away from the interface ([`crate::composer`]).
    composer: Composer,
    /// The project as the compositing thread sees it, by its content hash: copied only when
    /// the project changes, not on every frame that plays.
    snapshot: Option<(u64, Arc<Project>)>,
    /// Whether the picture on screen was put together from optimised copies, for the badge.
    optimised: bool,
    /// Whether the current video is being decoded on the processor instead of the graphics card
    /// (a hardware read error, or the speed trial choosing the processor), for the CPU badge.
    fell_back: bool,
}

impl Viewer {
    /// Shows the picture for `key`: at once when the render cache has it, or else asks the
    /// compositing thread for it and goes on showing the picture there is until it arrives.
    pub fn ensure(
        &mut self,
        ctx: &Context,
        project: &Project,
        cache: &MediaCache,
        key: ViewerKey,
        frames: &mut RenderCache,
        frame_key: FrameKey,
    ) {
        self.receive(ctx, frames);
        self.optimised = cache.uses_copies();
        self.fell_back = media_shown(project, key.frame).is_some_and(|m| cache.fell_back(m));
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if let Some(frame) = frames.get(&frame_key) {
            let image = color_image(&frame);
            self.show(ctx, frame, image, key);
            // A picture asked for before this one must not replace it when it arrives.
            self.composer.outdate();
            return;
        }
        // Already on its way.
        if self.composer.asked.as_ref() == Some(&key) {
            return;
        }
        let project = match &self.snapshot {
            Some((content, p)) if *content == frame_key.content => p.clone(),
            _ => {
                let p = Arc::new(project.clone());
                self.snapshot = Some((frame_key.content, p.clone()));
                p
            }
        };
        self.composer.submit(
            ctx,
            Job {
                id: 0,
                viewer: key,
                key: frame_key,
                project,
                media: cache.clone(),
            },
        );
        // The browser puts it together at once.
        self.receive(ctx, frames);
    }

    /// Shows the picture the compositing thread finished, if there is one, and keeps it in the
    /// render cache when it is final.
    fn receive(&mut self, ctx: &Context, frames: &mut RenderCache) {
        if let Some(done) = self.composer.take_done() {
            if done.exact {
                frames.put(done.key, done.frame.clone(), done.slow);
            }
            // Kept either way, but shown only if nothing newer was shown since it was asked for.
            if self.composer.is_current(&done) {
                self.show(ctx, done.frame, done.image, done.viewer);
            }
        }
    }

    fn show(&mut self, ctx: &Context, frame: Arc<Frame>, image: ColorImage, key: ViewerKey) {
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("viewer", image, TextureOptions::LINEAR)),
        }
        self.frame = Some(frame);
        self.generation += 1;
        self.key = Some(key);
    }

    /// Draws the picture as large as fits in `area`, and returns where it is.
    pub fn paint(
        &self,
        ui: &mut Ui,
        area: Rect,
        aspect: f32,
        background: Color32,
        picture: bool,
        overlays: Overlays,
    ) -> Rect {
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, background);
        let inner = area.shrink(8.0);
        if inner.width() < 4.0 || inner.height() < 4.0 {
            return inner;
        }
        let fit = if inner.width() / inner.height() > aspect {
            vec2(inner.height() * aspect, inner.height())
        } else {
            vec2(inner.width(), inner.width() / aspect)
        };
        let rect = Rect::from_center_size(inner.center(), fit);
        if !picture {
            return rect;
        }
        if overlays.checkerboard {
            draw_checkerboard(&painter, rect);
        } else {
            painter.rect_filled(rect, 0.0, Color32::BLACK);
        }
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        draw_overlays(&painter, rect, overlays);
        let labels = badge_labels(self.optimised, self.fell_back);
        if !labels.is_empty() {
            let at = rect.left_top() + vec2(10.0, 10.0);
            let mut x = at.x;
            for label in labels {
                let text = painter.layout_no_wrap(
                    label.to_string(),
                    egui::FontId::proportional(11.0),
                    Color32::from_rgb(235, 235, 235),
                );
                let text_width = text.size().x;
                let box_rect = egui::Rect::from_min_size(egui::pos2(x, at.y), text.size())
                    .expand2(vec2(6.0, 3.0));
                painter.rect_filled(box_rect, 3.0, Color32::from_black_alpha(150));
                painter.galley(egui::pos2(x + 6.0, at.y + 3.0), text, Color32::WHITE);
                x += text_width + 18.0;
            }
        }
        rect
    }
}

/// Returns the badge labels to draw, in order: optimised first, then CPU if the video fell
/// back from the graphics card to the processor.
fn badge_labels(optimised: bool, fell_back: bool) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if optimised {
        labels.push("OPTIMISED");
    }
    if fell_back {
        labels.push("CPU");
    }
    labels
}

/// The media of the topmost picture clip at `frame`: the file the viewer shows.
fn media_shown(project: &Project, frame: u64) -> Option<MediaId> {
    project
        .tracks
        .iter()
        .rev()
        .filter(|t| t.kind == TrackKind::Video && !t.muted && t.picture)
        .find_map(|t| {
            Project::clip_at(t, frame)
                .filter(|c| c.enabled)
                .map(|c| c.media)
        })
}

/// A rendered picture as the viewer's texture takes it.
pub(crate) fn color_image(frame: &Frame) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.rgba)
}

/// Grey squares behind a picture with transparent parts.
fn draw_checkerboard(painter: &egui::Painter, rect: Rect) {
    const SQUARE: f32 = 12.0;
    let (light, dark) = (Color32::from_gray(102), Color32::from_gray(64));
    painter.rect_filled(rect, 0.0, dark);
    let (cols, rows) = (
        (rect.width() / SQUARE).ceil() as usize,
        (rect.height() / SQUARE).ceil() as usize,
    );
    for row in 0..rows {
        for col in (row % 2..cols).step_by(2) {
            let min = rect.min + vec2(col as f32 * SQUARE, row as f32 * SQUARE);
            let square = Rect::from_min_size(min, vec2(SQUARE, SQUARE)).intersect(rect);
            painter.rect_filled(square, 0.0, light);
        }
    }
}

/// The guides over the picture: the safe areas, the centre and the thirds.
fn draw_overlays(painter: &egui::Painter, rect: Rect, overlays: Overlays) {
    let stroke = egui::Stroke::new(1.0, Color32::from_white_alpha(150));
    if overlays.safe_area {
        for share in [0.93, 0.9] {
            let r = Rect::from_center_size(rect.center(), rect.size() * share);
            painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
        }
    }
    if overlays.crosshair {
        let c = rect.center();
        painter.line_segment([pos2(c.x - 14.0, c.y), pos2(c.x + 14.0, c.y)], stroke);
        painter.line_segment([pos2(c.x, c.y - 14.0), pos2(c.x, c.y + 14.0)], stroke);
    }
    if overlays.grid {
        for n in 1..3 {
            let f = n as f32 / 3.0;
            let x = rect.left() + rect.width() * f;
            let y = rect.top() + rect.height() * f;
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], stroke);
            painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays a portrait 1080x1920 clip at 60 fps (a key frame every 2 seconds) through the viewer
    /// in real time, twice (the second time the render cache has some pictures), as the app does
    /// on every frame. Returns how often the picture on show went back in time, and how many
    /// ticks it lagged more than 10 frames behind the playhead. `None` without `ffmpeg`.
    #[cfg(not(target_arch = "wasm32"))]
    fn play_through_the_viewer(name: &str, size: &str) -> Option<(usize, usize, Vec<u64>)> {
        use picasso_core::{Command, Fps, MediaSource, Outcome};
        let path = std::env::temp_dir().join(name);
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=duration=6:size={size}:rate=60"))
            .args([
                "-pix_fmt", "yuv420p", "-g", "120", "-c:v", "libx264", "-preset", "veryfast",
            ])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return None;
        }
        let ctx = Context::default();
        let mut p = Project::new("t");
        (p.width, p.height, p.fps) = (1920, 1080, Fps::FPS_60);
        let Ok(Outcome::Media(m)) = (Command::AddMedia {
            name: "v".into(),
            source: MediaSource::Video {
                path: path.to_string_lossy().into_owned(),
            },
            length: Some(360),
            default_duration: 360,
        })
        .apply(&mut p) else {
            return None;
        };
        let track = p.tracks[0].id;
        let _ = Command::AddClip {
            track,
            media: m,
            start: 0,
            duration: None,
            with_sound: false,
        }
        .apply(&mut p);
        // As the app: a 960 x 540 preview, the video decoded at half size.
        let cache = crate::video::new_cache(&ctx).with_reduction(2);
        let (mut viewer, mut frames) = (Viewer::default(), RenderCache::new());
        let content = frames.content((0, 0, 0), &p);
        let (mut back, mut late, mut shown) = (0, 0, Vec::new());
        for _pass in 0..2 {
            let start = std::time::Instant::now();
            let mut last: Option<u64> = None;
            loop {
                let frame = (start.elapsed().as_secs_f64() * 60.0) as u64;
                if frame >= 300 {
                    break;
                }
                let key = ViewerKey {
                    epoch: 0,
                    revision: 0,
                    media_generation: 0,
                    video: cache.video_generation(),
                    frame,
                    width: 960,
                    height: 540,
                    alpha: false,
                    highlight: 0,
                };
                let frame_key = FrameKey {
                    content,
                    frame,
                    width: 960,
                    height: 540,
                    reduce: 2,
                    alpha: false,
                };
                viewer.ensure(&ctx, &p, &cache, key, &mut frames, frame_key);
                if let Some(on) = viewer.key.as_ref().map(|k| k.frame) {
                    // (`l <= frame`: the last picture of the pass before does not count.)
                    if last.is_some_and(|l| on < l && l <= frame) {
                        back += 1;
                    }
                    if frame > on + 10 {
                        late += 1;
                    }
                    last = Some(on);
                    shown.push(on);
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
        let _ = std::fs::remove_file(path);
        Some((back, late, shown))
    }

    /// Plays in real time for about 25 seconds: run it with `--ignored` when playback is in doubt.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore]
    fn playback_never_shows_an_older_picture_after_a_newer_one() {
        // A clip that decodes in time, and a 4K one that does not on most computers.
        for (name, size) in [
            ("picasso_viewer_play.mp4", "1080x1920"),
            ("picasso_viewer_play_4k.mp4", "2160x3840"),
        ] {
            let Some((back, late, shown)) = play_through_the_viewer(name, size) else {
                return;
            };
            eprintln!(
                "PLAYBACK {size}: back={back} late={late} ticks={}",
                shown.len()
            );
            assert_eq!(
                back, 0,
                "{size}: the picture went back in time {back} times: {shown:?}"
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_picture_is_put_together_away_from_the_interface_and_kept() {
        use picasso_core::{Command, MediaSource, Outcome};
        let mut p = Project::new("t");
        (p.width, p.height) = (64, 36);
        if let Ok(Outcome::Media(m)) = (Command::AddMedia {
            name: "c".into(),
            source: MediaSource::Solid { rgb: [10, 20, 30] },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p)
        {
            let track = p.tracks[0].id;
            let _ = Command::AddClip {
                track,
                media: m,
                start: 0,
                duration: None,
                with_sound: false,
            }
            .apply(&mut p);
        }
        let ctx = Context::default();
        let (mut viewer, mut frames, media) =
            (Viewer::default(), RenderCache::new(), MediaCache::new());
        let content = frames.content((0, 0, 0), &p);
        let key = |frame| ViewerKey {
            epoch: 0,
            revision: 0,
            media_generation: 0,
            video: 0,
            frame,
            width: 64,
            height: 36,
            alpha: false,
            highlight: 0,
        };
        let frame_key = |frame| FrameKey {
            content,
            frame,
            width: 64,
            height: 36,
            reduce: 1,
            alpha: false,
        };
        // The interface does not wait: nothing is on show when the call returns.
        viewer.ensure(&ctx, &p, &media, key(3), &mut frames, frame_key(3));
        assert!(
            viewer.frame.is_none(),
            "it was put together on the interface thread"
        );
        // It arrives on a later frame.
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while viewer.frame.is_none() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(1));
            viewer.ensure(&ctx, &p, &media, key(3), &mut frames, frame_key(3));
        }
        let shown = viewer.frame.clone().expect("the picture never arrived");
        assert_eq!(&shown.rgba[..4], &[10, 20, 30, 255]);
        // It was kept: going back to it later is answered at once, from the render cache.
        assert!(frames.get(&frame_key(3)).is_some());
        viewer.ensure(&ctx, &p, &media, key(4), &mut frames, frame_key(4));
        viewer.ensure(&ctx, &p, &media, key(3), &mut frames, frame_key(3));
        assert_eq!(viewer.key, Some(key(3)));

        // A picture asked for (5, not cached), then a newer one shown from the cache (3 again,
        // standing for any frame the cache has): when 5 arrives late, it must not replace it.
        viewer.ensure(&ctx, &p, &media, key(5), &mut frames, frame_key(5));
        viewer.key = None;
        viewer.ensure(&ctx, &p, &media, key(3), &mut frames, frame_key(3));
        std::thread::sleep(std::time::Duration::from_millis(300));
        viewer.receive(&ctx, &mut frames);
        assert_eq!(
            viewer.key,
            Some(key(3)),
            "an older picture came back on top"
        );
        // It was still kept for later.
        assert!(frames.get(&frame_key(5)).is_some());
    }

    #[test]
    fn preview_keeps_the_shape_and_caps_the_long_side() {
        let mut p = Project::new("t");
        p.width = 3840;
        p.height = 2160;
        assert_eq!(preview_size(&p), (960, 540));
        p.width = 1080;
        p.height = 1920;
        assert_eq!(preview_size(&p), (540, 960));
        p.width = 640;
        p.height = 360;
        assert_eq!(preview_size(&p), (640, 360));
        p.width = 16_000;
        p.height = 1;
        assert_eq!(preview_size(&p), (960, 1));
    }

    /// The badge labels cover all four combinations of optimised and CPU-fell-back.
    #[test]
    fn badge_labels_cover_all_four_combinations() {
        assert_eq!(badge_labels(false, false), Vec::<&str>::new());
        assert_eq!(badge_labels(true, false), vec!["OPTIMISED"]);
        assert_eq!(badge_labels(false, true), vec!["CPU"]);
        assert_eq!(badge_labels(true, true), vec!["OPTIMISED", "CPU"]);
    }
}
