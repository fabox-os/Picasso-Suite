//! "Choose your video layout": a platform (YouTube, Instagram, Facebook, TikTok, More), one of
//! its templates and a quality give the picture size of the project. The window opens by itself
//! on a new, empty project, and from File > Choose Video Layout... at any time. "Decide later"
//! keeps the size the project has.

use egui::{pos2, vec2, Align2, Color32, Context, Rect, RichText, Sense, Stroke, StrokeKind};
use picasso_core::video_layout::{self, Platform, Quality, Template};
use picasso_core::Command;

use crate::app::StudioApp;
use crate::theme;

/// The window's state: what is picked so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VideoLayoutChooser {
    pub platform: Platform,
    pub template: &'static str,
    pub quality: Quality,
    /// Opened from the menu, not by itself on a new project.
    pub from_menu: bool,
}

impl VideoLayoutChooser {
    /// What a new project starts the choice with: a YouTube video at 1080p.
    pub(crate) fn first_run() -> Self {
        let template = video_layout::template_by_id(video_layout::DEFAULT.0)
            .unwrap_or(&video_layout::TEMPLATES[0]);
        Self {
            platform: template.platform,
            template: template.id,
            quality: video_layout::DEFAULT.1,
            from_menu: false,
        }
    }

    /// The choice that matches a picture of `width` x `height`, for a project that has a size.
    pub(crate) fn matching(width: u32, height: u32) -> Self {
        let (template, quality) = video_layout::nearest(width, height);
        Self {
            platform: template.platform,
            template: template.id,
            quality,
            from_menu: true,
        }
    }

    pub(crate) fn selected(&self) -> &'static Template {
        video_layout::template_by_id(self.template).unwrap_or(&video_layout::TEMPLATES[0])
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        video_layout::size_for(self.selected(), self.quality)
    }

    /// Picks a platform, and its first template unless the one picked belongs to it.
    pub(crate) fn pick_platform(&mut self, platform: Platform) {
        self.platform = platform;
        if self.selected().platform != platform {
            if let Some(first) = video_layout::templates_of(platform).next() {
                self.template = first.id;
            }
        }
    }
}

impl StudioApp {
    /// Opens the window from File > Choose Video Layout...: with the choice that matches the
    /// project's size now.
    pub(crate) fn open_video_layout(&mut self) {
        let (w, h) = (self.project().width, self.project().height);
        self.extra.video_layout = Some(VideoLayoutChooser::matching(w, h));
    }

    /// Sets the project's size to the choice (its frame rate stays) and closes the window.
    pub(crate) fn apply_video_layout(&mut self, chosen: &VideoLayoutChooser) {
        let (width, height) = chosen.size();
        let fps = self.project().fps;
        if self
            .run(Command::SetFormat { width, height, fps })
            .is_some()
        {
            self.info(format!(
                "The video is {} ({width}x{height})",
                chosen.selected().label
            ));
        }
        self.extra.video_layout = None;
        self.extra.video_layout_decided = true;
    }

    /// "Decide later" or Cancel: the size stays, and the window does not come back by itself for
    /// this project.
    pub(crate) fn dismiss_video_layout(&mut self) {
        self.extra.video_layout = None;
        self.extra.video_layout_decided = true;
    }

    /// Whether the project is new: nothing in the media pool and nothing on the timeline.
    fn project_is_blank(&self) -> bool {
        let p = self.project();
        p.media.is_empty() && p.tracks.iter().all(|t| t.clips.is_empty())
    }

    pub(crate) fn video_layout_window(&mut self, ctx: &Context) {
        // A new project asks once, before anything is put in it.
        if self.extra.video_layout.is_none()
            && !self.extra.video_layout_decided
            && self.fps_prompt.is_none()
            && self.project_is_blank()
        {
            self.extra.video_layout = Some(VideoLayoutChooser::first_run());
        }
        // A project that has something in it is past the question.
        if !self.extra.video_layout_decided && !self.project_is_blank() {
            self.extra.video_layout_decided = true;
        }
        let Some(mut chosen) = self.extra.video_layout.take() else {
            return;
        };
        let mut open = true;
        let mut answer = None;
        egui::Window::new("Choose your video layout")
            .id(egui::Id::new("video_layout_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .fixed_size(vec2(520.0, 0.0))
            .show(ctx, |ui| {
                ui.label(if chosen.from_menu {
                    "Pick a platform template and quality. This updates the project video size."
                } else {
                    "Pick a category, then a template and quality. You can change this anytime in File > Choose Video Layout."
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for platform in Platform::ALL {
                        if ui
                            .selectable_label(chosen.platform == platform, platform.label())
                            .clicked()
                        {
                            chosen.pick_platform(platform);
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label("Template");
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    for t in video_layout::templates_of(chosen.platform) {
                        template_row(ui, &mut chosen, t);
                    }
                });
                ui.add_space(6.0);
                ui.label("Quality");
                ui.horizontal(|ui| {
                    for q in Quality::ALL {
                        ui.selectable_value(&mut chosen.quality, q, q.label());
                    }
                });
                ui.add_space(8.0);
                preview(ui, &chosen);
                ui.add_space(8.0);
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("Continue").strong()).clicked() {
                        answer = Some(true);
                    }
                    let later = if chosen.from_menu { "Cancel" } else { "Decide later" };
                    if ui.button(later).clicked() {
                        answer = Some(false);
                    }
                });
            });
        if answer.is_none() {
            answer = ctx.input(|i| {
                if i.key_pressed(egui::Key::Enter) {
                    Some(true)
                } else if i.key_pressed(egui::Key::Escape) {
                    Some(false)
                } else {
                    None
                }
            });
        }
        match answer {
            Some(true) => self.apply_video_layout(&chosen),
            Some(false) => self.dismiss_video_layout(),
            None if !open => self.dismiss_video_layout(),
            None => self.extra.video_layout = Some(chosen),
        }
    }
}

/// One template in the list: its name, its shape in words and a small picture of the shape.
fn template_row(ui: &mut egui::Ui, chosen: &mut VideoLayoutChooser, t: &Template) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let selected = chosen.template == t.id;
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 3.0, Color32::from_rgb(52, 42, 16));
    } else if response.hovered() {
        painter.rect_filled(rect, 3.0, Color32::from_rgb(44, 44, 48));
    }
    let accent = if selected {
        Color32::from_rgb(250, 190, 40)
    } else {
        Color32::from_rgb(222, 222, 226)
    };
    painter.text(
        rect.left_top() + vec2(10.0, 8.0),
        Align2::LEFT_TOP,
        t.label,
        theme::semibold(14.0),
        accent,
    );
    painter.text(
        rect.left_top() + vec2(10.0, 26.0),
        Align2::LEFT_TOP,
        t.aspect.label(),
        theme::regular(12.0),
        Color32::from_rgb(146, 146, 150),
    );
    let (rw, rh) = t.aspect.ratio();
    let scale = 26.0 / rw.max(rh) as f32;
    let shape = Rect::from_center_size(
        pos2(rect.right() - 28.0, rect.center().y),
        vec2(rw as f32 * scale, rh as f32 * scale),
    );
    painter.rect_stroke(shape, 3.0, Stroke::new(1.2, accent), StrokeKind::Inside);
    if response.clicked() {
        chosen.template = t.id;
    }
}

/// The shape of the picture in a box, and its size in words.
fn preview(ui: &mut egui::Ui, chosen: &VideoLayoutChooser) {
    let (w, h) = chosen.size();
    let template = chosen.selected();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(88.0, 88.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 24, 27));
        let scale = (88.0 / w.max(1) as f32).min(88.0 / h.max(1) as f32) * 0.9;
        let frame = Rect::from_center_size(
            rect.center(),
            vec2((w as f32 * scale).max(8.0), (h as f32 * scale).max(8.0)),
        );
        painter.rect_filled(frame, 2.0, Color32::from_rgb(38, 38, 42));
        painter.rect_stroke(
            frame,
            2.0,
            Stroke::new(1.2, Color32::from_rgb(250, 190, 40)),
            StrokeKind::Inside,
        );
        painter.text(
            frame.center(),
            Align2::CENTER_CENTER,
            template.aspect.label(),
            egui::FontId::monospace(12.0),
            Color32::from_rgb(222, 222, 226),
        );
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(template.label).strong());
            ui.label(
                RichText::new(format!("{w}×{h} · {}", template.aspect.label()))
                    .monospace()
                    .weak(),
            );
            ui.label(RichText::new("Preview shows the canvas aspect ratio").small().weak());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_starts_at_a_youtube_video_and_moves_with_the_platform() {
        let mut c = VideoLayoutChooser::first_run();
        assert_eq!((c.platform, c.template, c.size()), (Platform::YouTube, "yt_video", (1920, 1080)));
        c.pick_platform(Platform::Instagram);
        assert_eq!(c.template, "ig_reel");
        assert_eq!(c.size(), (1080, 1920));
        // Picking the platform again keeps the template picked in it.
        c.template = "ig_post";
        c.pick_platform(Platform::Instagram);
        assert_eq!(c.template, "ig_post");
        c.pick_platform(Platform::TikTok);
        assert_eq!(c.template, "tiktok");
        c.quality = Quality::Uhd4k;
        assert_eq!(c.size(), (2160, 3840));
    }

    #[test]
    fn a_project_size_selects_its_own_layout() {
        let c = VideoLayoutChooser::matching(1080, 1350);
        assert_eq!((c.template, c.quality, c.from_menu), ("ig_feed", Quality::P1080, true));
        assert_eq!(c.platform, Platform::Instagram);
    }
}
