//! The Deliver page: the render settings at the left, the viewer with its range and transport in
//! the middle, the render queue at the right, and the timeline under them.
//!
//! Positions are those of the reference screen, in the 1872-point-wide window (see `chrome.rs`).

// Folder browsing, render settings and the queue only run on the desktop; the browser build keeps
// the types so the page still draws.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use egui::{pos2, vec2, Align2, Color32, Context, Rect, RichText, Sense, Stroke, Ui};
use picasso_core::{Command, Fps};
use picasso_render::{encode_png, render_frame};

use crate::app::StudioApp;
use crate::chrome::{Screen, Tool, DESIGN_WIDTH};
use crate::icons::{paint, Icon};
use crate::panels::Split;
use crate::platform;
use crate::queue::JobState;
use crate::theme::{self, regular, semibold};
use crate::widgets::heading;

/// The page is laid out at a larger size than Resolve draws it and shown scaled down by this
/// much (see `deliver_scaled`), so that its columns, rows and text come out as Resolve's at 1080p
/// (`docs/reference/resolve-1080p`). Positions below are in the larger units.
pub(crate) const DELIVER_SCALE: f32 = 0.64;
pub(crate) const LEFT: f32 = 0.0;
/// 449 pixels of Resolve's 1919: the render settings column.
pub(crate) const LEFT_RIGHT: f32 = 685.0;
/// 1498 pixels of Resolve's 1919: where the render queue starts.
const MIDDLE_RIGHT: f32 = 2283.0;
const RIGHT: f32 = DESIGN_WIDTH / DELIVER_SCALE - 1.0;
/// Under the toolbar: 79 points of the window.
pub(crate) const TOP: f32 = 123.5;
pub(crate) const BAR: f32 = 33.5;

/// A row of Resolve at 1080p (counted from the top of its window, whose toolbar is at 0 to 39),
/// in this page's units.
fn rv(y: f32) -> f32 {
    (40.0 + y * crate::chrome::R) / DELIVER_SCALE
}
const TEXT: Color32 = Color32::from_rgb(244, 240, 236);
const DIM: Color32 = Color32::from_rgb(150, 150, 156);
const LINE: Color32 = Color32::from_rgb(14, 14, 18);

/// The export frame rate a render starts with: 60 frames per second (the frame rate list holds
/// the timeline's rate first, then the presets, so this is the position of
/// [`Fps::FPS_60`] in [`Fps::PRESETS`] plus one).
pub(crate) const DEFAULT_FPS_CHOICE: usize = 8;

/// The three groups of settings.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Video,
    Audio,
    File,
}

/// The sizes the Resolution drop-down offers besides the timeline's and a custom one.
pub(crate) const RESOLUTIONS: [(&str, u32, u32); 6] = [
    ("HD 1280 x 720", 1280, 720),
    ("Full HD 1920 x 1080", 1920, 1080),
    ("QHD 2560 x 1440", 2560, 1440),
    ("4K UHD 3840 x 2160", 3840, 2160),
    ("Vertical 1080 x 1920", 1080, 1920),
    ("Square 1080 x 1080", 1080, 1080),
];

/// The colour spaces the file can be tagged with, and the FFmpeg name of each (`None`: as the
/// project, which is BT.709).
pub(crate) const COLOR_TAGS: [(&str, Option<&str>); 6] = [
    ("Same as project", None),
    ("Rec.709", Some("bt709")),
    ("Rec.2020", Some("bt2020")),
    ("Rec.601 (NTSC)", Some("smpte170m")),
    ("Rec.601 (PAL)", Some("bt470bg")),
    ("P3-D65", Some("smpte432")),
];

/// The gamma (transfer) tags and the FFmpeg name of each.
pub(crate) const GAMMA_TAGS: [(&str, Option<&str>); 8] = [
    ("Same as project", None),
    ("Rec.709", Some("bt709")),
    ("Gamma 2.2", Some("gamma22")),
    ("Gamma 2.8", Some("gamma28")),
    ("sRGB", Some("iec61966-2-1")),
    ("PQ (ST 2084)", Some("smpte2084")),
    ("HLG", Some("arib-std-b67")),
    ("Linear", Some("linear")),
];

/// What can be burned into the picture.
pub(crate) const BURN_INS: [&str; 5] = [
    "Same as project",
    "None",
    "Timecode",
    "Clip name",
    "Timecode and clip name",
];

/// What the page remembers: the choices of the form and how the timeline is shown.
pub(crate) struct DeliverUi {
    pub(crate) tab: Tab,
    /// The render preset last chosen ("Custom Export", "YouTube 1080p" ...).
    pub(crate) preset_name: String,
    pub(crate) file_name: String,
    pub(crate) location: String,
    /// The next folder chosen is the location, not a place to render into.
    pub(crate) browsing: bool,
    /// Render every clip as a file of its own.
    pub(crate) individual: bool,
    /// Never write over a file: add (1), (2) ... to the name.
    pub(crate) unique: bool,
    /// 0 is the timeline's size, 1 ... the sizes of [`RESOLUTIONS`], then a custom size.
    pub(crate) res_mode: usize,
    pub(crate) custom: (u32, u32),
    /// The custom height is the vertical resolution, the width follows the timeline's shape.
    pub(crate) vertical: bool,
    /// 0 is the timeline's frame rate, then the presets in order. It starts at 60 frames per
    /// second ([`DEFAULT_FPS_CHOICE`]).
    pub(crate) fps_choice: usize,
    pub(crate) every: u32,
    pub(crate) key_auto: bool,
    pub(crate) advanced_open: bool,
    pub(crate) subtitle_open: bool,
    /// Subtitle Settings: whether the timeline's titles are exported as subtitles, how (0 as a
    /// separate file, 1 burnt into the video, 2 as embedded captions) and as which kind of file.
    pub(crate) export_subtitle: bool,
    pub(crate) subtitle_format: usize,
    pub(crate) subtitle_kind: crate::subtitles::SubtitleKind,
    // The Advanced Settings.
    /// Anamorphic pixels (twice as wide as tall) instead of square ones.
    pub(crate) cinemascope: bool,
    /// 0 Auto, 1 Video levels, 2 Full range.
    pub(crate) levels: usize,
    pub(crate) retain_blacks: bool,
    pub(crate) color_tag: usize,
    pub(crate) gamma_tag: usize,
    pub(crate) burn: usize,
    pub(crate) bypass_reencode: bool,
    pub(crate) proxy_media: bool,
    pub(crate) cached_images: bool,
    pub(crate) force_sizing: bool,
    pub(crate) force_debayer: bool,
    pub(crate) flat_pass: bool,
    pub(crate) vision_bypass: bool,
    pub(crate) disable_sizing: bool,
    /// Run a program at the start or the end of a render job.
    pub(crate) script_on: bool,
    pub(crate) script_at_end: bool,
    pub(crate) script_path: String,
    /// How much of the timeline fits, 0 … 1 (the slider's place).
    pub(crate) zoom: f32,
    /// The first frame the timeline shows.
    pub(crate) scroll: f32,
}

impl Default for DeliverUi {
    fn default() -> Self {
        Self {
            tab: Tab::Video,
            preset_name: "Custom Export".into(),
            file_name: "Untitled".into(),
            location: String::new(),
            browsing: false,
            individual: false,
            unique: false,
            res_mode: 0,
            custom: (1920, 1080),
            vertical: false,
            fps_choice: DEFAULT_FPS_CHOICE,
            every: 30,
            key_auto: true,
            advanced_open: false,
            subtitle_open: false,
            export_subtitle: false,
            subtitle_format: 0,
            subtitle_kind: crate::subtitles::SubtitleKind::Imsc1,
            cinemascope: false,
            levels: 0,
            retain_blacks: false,
            color_tag: 0,
            gamma_tag: 0,
            burn: 0,
            bypass_reencode: true,
            proxy_media: false,
            cached_images: false,
            force_sizing: false,
            force_debayer: false,
            flat_pass: false,
            vision_bypass: false,
            disable_sizing: false,
            script_on: false,
            script_at_end: false,
            script_path: String::new(),
            zoom: 0.5,
            scroll: 0.0,
        }
    }
}

fn fill(p: &egui::Painter, s: &Screen, x0: f32, y0: f32, x1: f32, y1: f32, color: Color32) {
    p.rect_filled(s.r(x0, y0, x1, y1), 0.0, color);
}

impl StudioApp {
    /// Draws the page scaled down by `DELIVER_SCALE` in a layer of its own, which also scales
    /// where the pointer is.
    pub(crate) fn deliver_scaled(&mut self, ctx: &Context, s: &Screen) {
        let id = egui::Id::new("deliver_layer");
        let layer = egui::LayerId::new(egui::Order::Middle, id);
        ctx.set_transform_layer(layer, egui::emath::TSTransform::from_scaling(DELIVER_SCALE));
        let k = DELIVER_SCALE;
        let sv = Screen {
            o: pos2(s.o.x / k, s.o.y / k),
            h: s.h / k,
            warp: Default::default(),
        };
        // egui draws an Area's first frame invisible (its size is not known yet); repeat that
        // pass before anything is shown, so the page does not flash black when first opened.
        let first = ctx.memory(|m| m.area_rect(id)).is_none();
        egui::Area::new(id)
            .order(egui::Order::Middle)
            .fixed_pos(sv.o)
            .constrain(false)
            .movable(false)
            // An Area fades in from transparent each time it shows again, which made the page
            // black for a moment on every visit; a page appears at once.
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(Rect::from_min_size(sv.o, vec2(DESIGN_WIDTH / k, sv.h)));
                self.deliver_page(ui, ctx, &sv);
            });
        if first {
            ctx.request_discard("page layer sizing");
        }
    }

    pub(crate) fn deliver_page(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        let painter = ui.painter().clone();
        let bottom = s.h - 46.8 / DELIVER_SCALE;
        let y_view = rv(521.0).min(bottom - 300.0);
        let y_tools = y_view + (rv(657.0) - rv(521.0));
        let y_ruler = y_tools + (rv(698.0) - rv(657.0));
        // The dividers: the viewer and the render queue; the render settings column; and the
        // timeline under the viewer (the clip strip and the tool row move with it unstretched).
        let splits = [
            Split::column(
                "deliver queue",
                s,
                s.p(MIDDLE_RIGHT, TOP),
                s.p(MIDDLE_RIGHT, y_view),
                s.p(LEFT_RIGHT, 0.0).x,
                s.p(RIGHT, 0.0).x,
            ),
            Split::column(
                "deliver settings",
                s,
                s.p(LEFT_RIGHT, TOP),
                s.p(LEFT_RIGHT, bottom),
                s.p(LEFT, 0.0).x,
                s.p(MIDDLE_RIGHT, 0.0).x,
            ),
            Split::row(
                "deliver timeline",
                s,
                s.p(LEFT_RIGHT, y_view),
                s.p(RIGHT, y_view),
                s.p(0.0, TOP + BAR).y,
                s.p(0.0, bottom).y,
            )
            .keeping(y_ruler - y_view),
        ];
        let warped = self.warped(s, &splits);
        let s = &warped;

        fill(&painter, s, LEFT, TOP, RIGHT, bottom, theme::POOL_BG);
        fill(
            &painter,
            s,
            LEFT,
            TOP,
            RIGHT,
            TOP + BAR,
            theme::POOL_HEADER_BG,
        );
        fill(
            &painter,
            s,
            LEFT_RIGHT,
            TOP + BAR,
            MIDDLE_RIGHT,
            y_view,
            theme::VIEWER_BG,
        );
        fill(
            &painter,
            s,
            LEFT_RIGHT,
            y_view,
            RIGHT,
            y_tools,
            theme::POOL_BG,
        );
        fill(
            &painter,
            s,
            LEFT_RIGHT,
            y_tools,
            RIGHT,
            y_ruler,
            theme::POOL_HEADER_BG,
        );
        fill(
            &painter,
            s,
            LEFT_RIGHT,
            y_ruler,
            RIGHT,
            bottom,
            theme::TIMELINE_BG,
        );
        for x in [LEFT_RIGHT, MIDDLE_RIGHT] {
            fill(&painter, s, x - 1.0, TOP, x + 1.0, y_view, LINE);
        }
        fill(
            &painter,
            s,
            LEFT_RIGHT - 1.0,
            TOP,
            LEFT_RIGHT + 1.0,
            bottom,
            LINE,
        );
        for y in [TOP + BAR, y_view, y_tools] {
            fill(&painter, s, LEFT_RIGHT, y - 1.0, RIGHT, y + 1.0, LINE);
        }

        self.render_form(ui, ctx, s, TOP, bottom);
        self.deliver_bars(ui, s);
        self.deliver_viewer(ui, s, y_view);
        self.render_queue_panel(ui, ctx, s, y_view);
        self.deliver_timeline(ui, ctx, s, y_tools, y_ruler, bottom);
        // The timeline's video clips under the viewer.
        let strip = s.r(LEFT_RIGHT + 1.0, y_view + 1.0, RIGHT, y_tools - 1.0);
        self.clip_strip(ui, ctx, s, strip);
        self.system_section(&painter, s, y_view - 160.0, y_view - 56.0);
        self.dividers(ui, s, &splits);
    }

    /// What this computer renders with, in the band under the viewer: the graphics card and the
    /// graphics interface the window draws through.
    fn system_section(&self, painter: &egui::Painter, s: &Screen, y0: f32, y1: f32) {
        if y1 - y0 < 70.0 {
            return;
        }
        if !self.jobs().is_empty() {
            return;
        }
        let x = MIDDLE_RIGHT + 18.0;
        // As much smaller as the dividers made the queue narrower.
        let k = s.shrink(x + 150.0, y0 + 62.0);
        crate::i18n::paint_text(&painter,
            s.p(x, y0 + 26.0),
            Align2::LEFT_CENTER,
            "System",
            semibold(17.0 * k),
            TEXT,
        );
        let (card, api) = self.graphics.clone().unwrap_or_else(|| {
            (
                "Not reported".to_string(),
                if cfg!(target_arch = "wasm32") {
                    "WebGL 2 (browser)".to_string()
                } else {
                    "Not reported".to_string()
                },
            )
        });
        for (n, (label, value)) in [("Graphics card", card), ("Drawn with", api)]
            .into_iter()
            .enumerate()
        {
            let y = y0 + 62.0 + n as f32 * 30.0;
            crate::i18n::paint_text(&painter,
                s.p(x + 150.0, y),
                Align2::RIGHT_CENTER,
                label,
                regular(16.0 * k),
                DIM,
            );
            crate::i18n::paint_text(&painter,
                s.p(x + 166.0, y),
                Align2::LEFT_CENTER,
                value,
                regular(16.0 * k),
                TEXT,
            );
        }
    }

    /// The titles of the three parts and the viewer's tools.
    fn deliver_bars(&mut self, ui: &mut Ui, s: &Screen) {
        let painter = ui.painter().clone();
        let y = TOP + BAR / 2.0;
        let t = |hint| Tool::new(hint).wide(36.0);
        let mut planned: Option<&str> = None;
        // Text and buttons are as much smaller as the dividers squeezed their column; the
        // settings title also stops short of the menu button.
        let k = |x: f32| s.shrink(x, y);
        let title = "Render Settings - Custom Export";
        let room = s.p(LEFT_RIGHT - 52.0, y).x - s.p(LEFT + 16.0, y).x;
        let wide = painter
            .layout_no_wrap(title.to_string(), semibold(19.0), TEXT)
            .size()
            .x;
        crate::i18n::paint_text(&painter,
            s.p(LEFT + 16.0, y),
            Align2::LEFT_CENTER,
            title,
            semibold(19.0 * (room / wide.max(1.0)).min(1.0)),
            TEXT,
        );
        if self
            .tool(
                ui,
                s.p(LEFT_RIGHT - 30.0, y),
                Icon::Menu,
                s.squeezed(t("More").wide(34.0), LEFT_RIGHT - 30.0, y),
            )
            .clicked()
        {
            planned = Some("The render settings menu");
        }
        // The viewer: its size, the clip, and the options.
        crate::i18n::paint_text(&painter,
            s.p(LEFT_RIGHT + 18.0, y),
            Align2::LEFT_CENTER,
            "22%",
            regular(19.0 * k(LEFT_RIGHT + 18.0)),
            TEXT,
        );
        paint(
            &painter,
            s.p(LEFT_RIGHT + 66.0, y),
            Icon::Chevron,
            theme::ICON,
        );
        painter.circle_filled(
            s.p(LEFT_RIGHT + 112.0, y),
            2.5,
            Color32::from_rgb(70, 70, 76),
        );
        let name = self.project().timeline_name.clone();
        crate::i18n::paint_text(&painter,
            s.p(LEFT_RIGHT + 282.0, y),
            Align2::RIGHT_CENTER,
            &name,
            regular(19.0 * k(LEFT_RIGHT + 282.0)),
            TEXT,
        );
        paint(
            &painter,
            s.p(LEFT_RIGHT + 300.0, y),
            Icon::Chevron,
            theme::ICON,
        );
        let picker = ui
            .interact(
                s.r(LEFT_RIGHT + 150.0, y - 16.0, LEFT_RIGHT + 312.0, y + 16.0),
                ui.id().with("deliver_timeline_pick"),
                Sense::click(),
            )
            .on_hover_text("Timelines of the project");
        let mut chosen: Option<Option<u64>> = None;
        egui::Popup::menu(&picker).show(|ui| {
            let current = self.project().timeline_id;
            for (id, name) in self.project().timeline_list() {
                if ui.selectable_label(id == current, name).clicked() {
                    chosen = Some(Some(id));
                }
            }
            ui.separator();
            if ui.button("New Timeline...").clicked() {
                chosen = Some(None);
            }
        });
        match chosen {
            Some(Some(id)) => self.switch_timeline(id),
            Some(None) => self.new_timeline(),
            None => {}
        }
        let time = self.tc(self.playhead as i64);
        crate::i18n::paint_text(&painter,
            s.p(MIDDLE_RIGHT - 148.0, y),
            Align2::RIGHT_CENTER,
            &time,
            semibold(19.0 * k(MIDDLE_RIGHT - 148.0)),
            TEXT,
        );
        paint(
            &painter,
            s.p(MIDDLE_RIGHT - 130.0, y),
            Icon::Chevron,
            theme::ICON,
        );
        if self
            .tool(
                ui,
                s.p(MIDDLE_RIGHT - 90.0, y),
                Icon::ColorWheel,
                s.squeezed(t("Viewer colour"), MIDDLE_RIGHT - 90.0, y),
            )
            .clicked()
        {
            planned = Some("Viewer colour");
        }
        if self
            .tool(
                ui,
                s.p(MIDDLE_RIGHT - 44.0, y),
                Icon::Menu,
                s.squeezed(t("More").wide(34.0), MIDDLE_RIGHT - 44.0, y),
            )
            .clicked()
        {
            planned = Some("The viewer menu");
        }
        crate::i18n::paint_text(&painter,
            s.p(MIDDLE_RIGHT + 16.0, y),
            Align2::LEFT_CENTER,
            "Render Queue",
            semibold(19.0 * k(MIDDLE_RIGHT + 16.0)),
            TEXT,
        );
        if self
            .tool(
                ui,
                s.p(RIGHT - 24.0, y),
                Icon::Menu,
                s.squeezed(t("More").wide(34.0), RIGHT - 24.0, y),
            )
            .clicked()
        {
            planned = Some("The render queue menu");
        }
        if let Some(what) = planned {
            self.planned(what);
        }
    }

    /// The range, the picture, the scrub line and the transport.
    fn deliver_viewer(&mut self, ui: &mut Ui, s: &Screen, y_view: f32) {
        let painter = ui.painter().clone();
        // IN, OUT and DURATION.
        let project = self.project();
        let length = project.duration();
        let start = project.mark_in.unwrap_or(0).min(length);
        let end = project.mark_out.unwrap_or(length).min(length).max(start);
        let (tc_in, tc_out, tc_dur) = (
            self.tc(start as i64),
            self.tc(end as i64),
            self.project().fps.timecode(end - start),
        );
        let y = TOP + BAR + 18.0;
        let x = LEFT_RIGHT;
        for (dx, name, value) in [
            (14.0, "IN", &tc_in),
            (192.0, "OUT", &tc_out),
            (466.0, "DURATION", &tc_dur),
        ] {
            // As much smaller as the dividers made the viewer narrower.
            let k = s.shrink(x + dx, y);
            let galley = painter.layout_no_wrap(name.to_string(), regular(17.0 * k), DIM);
            let width = galley.size().x;
            let at = s.p(x + dx, y);
            crate::i18n::paint_text(&painter,at, Align2::LEFT_CENTER, name, regular(17.0 * k), DIM);
            crate::i18n::paint_text(&painter,
                at + vec2(width + 12.0 * k, 0.0),
                Align2::LEFT_CENTER,
                value,
                regular(19.0 * k),
                theme::WHITE,
            );
        }
        // The picture.
        let pic_area = s.r(
            LEFT_RIGHT + 1.0,
            TOP + BAR + 36.0,
            MIDDLE_RIGHT - 1.0,
            y_view - 70.0,
        );
        let aspect = self.project().width as f32 / self.project().height.max(1) as f32;
        self.viewer
            .paint(ui, pic_area, aspect, theme::VIEWER_BG, true, self.overlays);
        // The scrub line.
        let line_y = y_view - 53.0;
        let (x0, x1) = (LEFT_RIGHT + 36.0, MIDDLE_RIGHT - 38.0);
        let (a, b) = (s.p(x0, line_y), s.p(x1, line_y));
        painter.line_segment([a, b], Stroke::new(1.5, Color32::from_rgb(60, 60, 64)));
        let duration = length.max(1);
        let at = a.x + (b.x - a.x) * (self.playhead as f32 / duration as f32).clamp(0.0, 1.0);
        painter.rect_filled(
            Rect::from_center_size(pos2(at, a.y), vec2(4.0, 14.0)),
            1.0,
            theme::WHITE,
        );
        let scrub = ui.interact(
            Rect::from_min_max(pos2(a.x, a.y - 10.0), pos2(b.x, a.y + 10.0)),
            ui.id().with("deliver_scrub"),
            Sense::click_and_drag(),
        );
        if let Some(p) = scrub
            .interact_pointer_pos()
            .filter(|_| scrub.clicked() || scrub.dragged())
        {
            let share = ((p.x - a.x) / (b.x - a.x)).clamp(0.0, 1.0);
            self.go_to((share * duration as f32) as u64);
        }

        // The transport.
        let y = y_view - 20.0;
        let mut planned: Option<&str> = None;
        let color = theme::TRANSPORT_ICON;
        let base = Tool::new("").wide(40.0).color(color);
        let (first, step) = (LEFT_RIGHT + 311.0, 46.0);
        let mut button = |app: &mut Self, n: f32, icon: Icon, hint: &'static str, col: Color32| {
            let tool = Tool {
                hint,
                color: Some(col),
                ..base
            };
            app.tool(
                ui,
                s.p(first + n * step, y),
                icon,
                s.squeezed(tool, first + n * step, y),
            )
            .clicked()
        };
        if button(self, 0.0, Icon::Speaker, "Audio", theme::ICON_BRIGHT) {
            planned = Some("Audio");
        }
        if button(self, 1.0, Icon::ToStart, "Go to the start (Home)", color) {
            self.playing = false;
            self.go_to(0);
        }
        if button(self, 2.0, Icon::PlayReverse, "Play backwards", color) {
            planned = Some("Playing backwards");
        }
        if button(self, 3.0, Icon::Stop, "Stop", color) {
            self.playing = false;
        }
        let play = if self.playing {
            theme::ICON_BRIGHT
        } else {
            color
        };
        if button(
            self,
            4.0,
            Icon::Play,
            if self.playing {
                "Pause (Space)"
            } else {
                "Play (Space)"
            },
            play,
        ) {
            self.toggle_play();
        }
        if button(self, 5.0, Icon::ToEnd, "Go to the end (End)", color) {
            self.playing = false;
            self.go_to(u64::MAX);
        }
        let looped = if self.looping {
            theme::ICON_BRIGHT
        } else {
            color
        };
        if button(self, 6.0, Icon::Loop, "Loop", looped) {
            self.looping = !self.looping;
        }
        if button(self, 7.5, Icon::NextEdit, "Next clip boundary", color) {
            self.go_to_edit(true);
        }
        if button(self, 8.6, Icon::PrevEdit, "Previous clip boundary", color) {
            self.go_to_edit(false);
        }
        if let Some(what) = planned {
            self.planned(what);
        }
    }

    /// The jobs of the render queue, and the button that starts them.
    fn render_queue_panel(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen, y_view: f32) {
        let painter = ui.painter().clone();
        let area = s.r(MIDDLE_RIGHT + 1.0, TOP + BAR, RIGHT, y_view - 56.0);
        let jobs = self.jobs();
        if jobs.is_empty() {
            let message = if self.can_render_here() {
                "No jobs in queue"
            } else {
                "Rendering needs the desktop app"
            };
            painter.text(
                pos2(area.center().x, area.center().y - 20.0),
                Align2::CENTER_CENTER,
                message,
                // Smaller when the queue was made narrow, so it stays inside it.
                crate::media::fitting(&painter, message, 46.0, area.width() - 24.0),
                theme::EMPTY_TITLE,
            );
        }
        let mut cancel = None;
        let mut remove = None;
        let mut y = area.top() + 8.0;
        for job in &jobs {
            let row =
                Rect::from_min_size(pos2(area.left() + 8.0, y), vec2(area.width() - 16.0, 64.0));
            painter.rect_filled(row, 3.0, Color32::from_rgb(34, 33, 38));
            crate::i18n::paint_text(&painter,
                row.min + vec2(12.0, 16.0),
                Align2::LEFT_CENTER,
                &job.name,
                semibold(17.0),
                TEXT,
            );
            let (status, color) = match &job.state {
                JobState::Queued => ("Waiting".to_string(), DIM),
                JobState::Running => (format!("{:.0}%", job.fraction * 100.0), theme::ACCENT),
                JobState::Done(took) => (
                    format!("Done in {took:.1} s"),
                    Color32::from_rgb(70, 190, 100),
                ),
                JobState::Failed(why) => (format!("Failed: {why}"), theme::ERROR),
                JobState::Cancelled => ("Cancelled".to_string(), DIM),
            };
            crate::i18n::paint_text(&painter,
                row.min + vec2(12.0, 40.0),
                Align2::LEFT_CENTER,
                format!("{}   {status}", job.label),
                regular(15.0),
                color,
            );
            if job.state == JobState::Running {
                let bar = Rect::from_min_size(
                    row.left_bottom() + vec2(12.0, -8.0),
                    vec2(row.width() - 24.0, 3.0),
                );
                painter.rect_filled(bar, 0.0, Color32::from_rgb(20, 20, 23));
                painter.rect_filled(
                    Rect::from_min_size(
                        bar.min,
                        vec2(bar.width() * job.fraction.clamp(0.0, 1.0), 3.0),
                    ),
                    0.0,
                    theme::ACCENT,
                );
            }
            let x_rect = Rect::from_center_size(
                pos2(row.right() - 20.0, row.top() + 18.0),
                vec2(26.0, 26.0),
            );
            let r = ui.interact(x_rect, ui.id().with(("job_x", job.id)), Sense::click());
            let c = if r.hovered() {
                theme::ICON_HOVER
            } else {
                theme::ICON
            };
            for (a, b) in [((-5.0, -5.0), (5.0, 5.0)), ((-5.0, 5.0), (5.0, -5.0))] {
                painter.line_segment(
                    [
                        x_rect.center() + vec2(a.0, a.1),
                        x_rect.center() + vec2(b.0, b.1),
                    ],
                    Stroke::new(1.5, c),
                );
            }
            if r.on_hover_text("Cancel or remove").clicked() {
                if matches!(job.state, JobState::Queued | JobState::Running) {
                    cancel = Some(job.id);
                } else {
                    remove = Some(job.id);
                }
            }
            y += 72.0;
            if y > area.bottom() - 60.0 {
                break;
            }
        }
        if let Some(id) = cancel {
            self.cancel_job(id);
        }
        if let Some(id) = remove {
            self.remove_job(id);
        }
        // Render All.
        let waiting = jobs.iter().any(|j| j.state == JobState::Queued) && !self.render_busy();
        let pill = s.r(RIGHT - 190.0, y_view - 46.0, RIGHT - 14.0, y_view - 14.0);
        let r = ui.interact(pill, ui.id().with("render_all"), Sense::click());
        let hot = waiting && r.hovered();
        painter.rect_filled(
            pill,
            16.0,
            if hot {
                Color32::from_rgb(52, 51, 58)
            } else {
                Color32::from_rgb(38, 37, 43)
            },
        );
        painter.rect_stroke(
            pill,
            16.0,
            Stroke::new(1.0, Color32::from_rgb(58, 57, 63)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            pill.center(),
            Align2::CENTER_CENTER,
            "Render All",
            // Smaller when the queue was made narrow, so it stays inside the button.
            crate::media::fitting(&painter, "Render All", 17.0, pill.width() - 20.0),
            if waiting {
                TEXT
            } else {
                Color32::from_rgb(86, 86, 92)
            },
        );
        if waiting && r.clicked() {
            self.render_all(ctx);
        }
        if jobs.iter().any(|j| {
            matches!(
                j.state,
                JobState::Done(_) | JobState::Failed(_) | JobState::Cancelled
            )
        }) {
            let clear = Rect::from_min_size(pill.left_top() - vec2(110.0, 0.0), vec2(100.0, 32.0));
            let r = ui.interact(clear, ui.id().with("clear_jobs"), Sense::click());
            crate::i18n::paint_text(&painter,
                clear.center(),
                Align2::CENTER_CENTER,
                "Clear done",
                regular(16.0),
                if r.hovered() { TEXT } else { DIM },
            );
            if r.clicked() {
                self.clear_finished_jobs();
            }
        }
    }

    /// The row under the viewer with what is rendered and the zoom, then the ruler and the clips.
    fn deliver_timeline(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        s: &Screen,
        y_tools: f32,
        y_ruler: f32,
        bottom: f32,
    ) {
        let painter = ui.painter().clone();
        let y = y_tools + 29.0;
        // Text and buttons are as much smaller as the dividers made the timeline narrower.
        let k = s.shrink(LEFT_RIGHT + 500.0, y);
        let mut planned: Option<&str> = None;
        if self
            .tool(
                ui,
                s.p(LEFT_RIGHT + 46.0, y),
                Icon::ListBullets,
                s.squeezed(
                    Tool::new("Timeline options").wide(40.0),
                    LEFT_RIGHT + 46.0,
                    y,
                ),
            )
            .clicked()
        {
            planned = Some("Timeline options");
        }
        // What is rendered.
        crate::i18n::paint_text(&painter,
            s.p(LEFT_RIGHT + 420.0, y),
            Align2::RIGHT_CENTER,
            "Render",
            regular(19.0 * k),
            DIM,
        );
        let marks = self.project().mark_in.is_some() || self.project().mark_out.is_some();
        let field = s.r(LEFT_RIGHT + 430.0, y - 16.0, LEFT_RIGHT + 640.0, y + 16.0);
        let r = ui.interact(field, ui.id().with("render_range"), Sense::click());
        painter.rect_filled(field, 3.0, Color32::from_rgb(30, 30, 34));
        painter.rect_stroke(field, 3.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let whole = self.range_is_whole();
        crate::i18n::paint_text(&painter,
            field.left_center() + vec2(12.0 * k, 0.0),
            Align2::LEFT_CENTER,
            if whole {
                "Entire Timeline"
            } else {
                "In/Out Range"
            },
            regular(18.0 * k),
            TEXT,
        );
        paint(
            &painter,
            field.right_center() - vec2(14.0 * k, 0.0),
            Icon::Chevron,
            theme::ICON,
        );
        let mut choose: Option<bool> = None;
        egui::Popup::menu(&r).show(|ui| {
            if ui.selectable_label(whole, "Entire Timeline").clicked() {
                choose = Some(true);
            }
            ui.add_enabled_ui(marks, |ui| {
                if ui.selectable_label(!whole, "In/Out Range").clicked() {
                    choose = Some(false);
                }
            });
        });
        if let Some(whole) = choose {
            self.set_range_whole(whole);
        }
        painter.line_segment(
            [
                s.p(LEFT_RIGHT + 700.0, y - 18.0),
                s.p(LEFT_RIGHT + 700.0, y + 18.0),
            ],
            Stroke::new(1.0, Color32::from_rgb(14, 14, 18)),
        );
        // The zoom.
        let track = s.r(LEFT_RIGHT + 760.0, y - 12.0, LEFT_RIGHT + 900.0, y + 12.0);
        crate::i18n::paint_text(&painter,
            s.p(LEFT_RIGHT + 744.0, y),
            Align2::CENTER_CENTER,
            "-",
            regular(22.0 * k),
            DIM,
        );
        crate::i18n::paint_text(&painter,
            s.p(LEFT_RIGHT + 922.0, y),
            Align2::CENTER_CENTER,
            "+",
            regular(22.0 * k),
            DIM,
        );
        let slide = ui.interact(track, ui.id().with("deliver_zoom"), Sense::click_and_drag());
        if let Some(p) = slide
            .interact_pointer_pos()
            .filter(|_| slide.clicked() || slide.dragged())
        {
            self.deliver_ui.zoom = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
        }
        painter.line_segment(
            [
                pos2(track.left(), y + s.o.y * 0.0),
                pos2(track.right(), y + s.o.y * 0.0),
            ],
            Stroke::new(2.0, Color32::from_rgb(60, 60, 66)),
        );
        painter.circle_filled(
            pos2(
                track.left() + track.width() * self.deliver_ui.zoom,
                track.center().y,
            ),
            5.5 * k,
            theme::ICON_BRIGHT,
        );
        if self
            .tool(
                ui,
                s.p(RIGHT - 30.0, y),
                Icon::MusicLines,
                s.squeezed(Tool::new("Audio tracks").wide(40.0), RIGHT - 30.0, y),
            )
            .clicked()
        {
            planned = Some("Audio tracks");
        }

        // The big time and the ruler.
        let ruler_left = LEFT_RIGHT + 381.0;
        crate::i18n::paint_text(&painter,
            s.p(LEFT_RIGHT + 250.0, y_ruler + 36.0),
            Align2::CENTER_CENTER,
            self.tc(self.playhead as i64),
            semibold(36.0 * k),
            TEXT,
        );
        let fps = self.project().fps;
        let per_second = 8.0 + 52.0 * self.deliver_ui.zoom; // points a second
        let per_frame = per_second / fps.as_f64() as f32;
        let length = self.project().duration() as f32;
        let visible = (RIGHT - ruler_left) / per_frame;
        let max_scroll = (length + visible * 0.25 - visible).max(0.0);
        self.deliver_ui.scroll = self.deliver_ui.scroll.clamp(0.0, max_scroll);
        let first = self.deliver_ui.scroll;
        let x_of = |frame: f32| ruler_left + (frame - first) * per_frame;
        let area = s.r(ruler_left, y_ruler, RIGHT, bottom);
        let clip = painter.with_clip_rect(area.intersect(painter.clip_rect()));
        // Marks every second, a number every eight.
        let nominal = fps.nominal().max(1) as f32;
        let mut second = (first / nominal).floor().max(0.0) as i64;
        loop {
            let frame = second as f32 * nominal;
            let x = x_of(frame);
            if x > area.right() + 1.0 {
                break;
            }
            let (major, h) = if second % 8 == 0 {
                (true, 26.0)
            } else {
                (false, 9.0)
            };
            let r = s.o.y;
            let _ = r;
            clip.line_segment(
                [
                    pos2(x, y_ruler_pos(s, y_ruler)),
                    pos2(x, y_ruler_pos(s, y_ruler) + h),
                ],
                Stroke::new(1.0, theme::RULER_TICK),
            );
            if major {
                crate::i18n::paint_text(&clip,
                    pos2(x + 6.0, y_ruler_pos(s, y_ruler) + 28.0),
                    Align2::LEFT_CENTER,
                    self.tc(frame as i64),
                    regular(14.0),
                    theme::RULER_LABEL_OVERVIEW,
                );
                clip.line_segment(
                    [
                        pos2(x, y_ruler_pos(s, y_ruler) + 40.0),
                        pos2(x, area.bottom()),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(44, 43, 50)),
                );
            }
            second += 1;
        }
        // What is rendered: a bar under the ruler from its start to its end.
        let length_frames = self.project().duration();
        let (r0, r1) = if self.range_is_whole() {
            (0, length_frames)
        } else {
            let p = self.project();
            let a = p.mark_in.unwrap_or(0).min(length_frames);
            (
                a,
                p.mark_out
                    .unwrap_or(length_frames)
                    .min(length_frames)
                    .max(a),
            )
        };
        let bar_y = y_ruler_pos(s, y_ruler) + 44.0;
        clip.line_segment(
            [pos2(x_of(r0 as f32), bar_y), pos2(x_of(r1 as f32), bar_y)],
            Stroke::new(4.0, Color32::from_rgb(110, 110, 116)),
        );
        clip.circle_filled(
            pos2(x_of(r0 as f32), bar_y),
            4.5,
            Color32::from_rgb(150, 150, 156),
        );
        // The tracks, with their headers on the left.
        let header = s.r(LEFT_RIGHT + 1.0, y_ruler, ruler_left, bottom);
        let lane_top = y_ruler_pos(s, y_ruler) + 56.0;
        self.deliver_tracks(ui, ctx, header, area, first, per_frame, lane_top);
        // The playhead, and a click or a drag on the ruler moves it.
        let px = x_of(self.playhead as f32);
        if px >= area.left() && px <= area.right() {
            painter.line_segment(
                [
                    pos2(px, y_ruler_pos(s, y_ruler)),
                    pos2(px, area.bottom() - 14.0),
                ],
                Stroke::new(1.5, theme::PLAYHEAD_GRAY),
            );
            painter.rect_filled(
                Rect::from_center_size(pos2(px, y_ruler_pos(s, y_ruler) + 7.0), vec2(12.0, 16.0)),
                4.0,
                theme::PLAYHEAD_GRAY,
            );
        }
        // Right-clicking the timeline sets what is rendered: the in and out at the playhead, or all.
        let body = ui.interact(area, ui.id().with("deliver_body"), Sense::click());
        let ruler = ui.interact(
            Rect::from_min_max(area.min, pos2(area.right(), area.top() + 46.0)),
            ui.id().with("deliver_ruler"),
            Sense::click_and_drag(),
        );
        if let Some(p) = ruler
            .interact_pointer_pos()
            .filter(|_| ruler.clicked() || ruler.dragged())
        {
            let frame = (first + (p.x - ruler_left) / per_frame).max(0.0);
            self.go_to(frame as u64);
        }
        let mut chosen: Option<crate::menu::Act> = None;
        let mut entire = false;
        ruler.context_menu(|ui| marks_menu(ui, &mut chosen, &mut entire));
        body.context_menu(|ui| marks_menu(ui, &mut chosen, &mut entire));
        if let Some(act) = chosen {
            self.do_act(ctx, act);
            // Marking a range renders it.
            if self.project().mark_in.is_some() || self.project().mark_out.is_some() {
                self.set_range_whole(false);
            }
        }
        if entire {
            self.do_act(ctx, crate::menu::Act::ClearInOut);
            self.set_range_whole(true);
        }
        // The scroll bar along the bottom.
        let bar = Rect::from_min_max(
            pos2(area.left(), area.bottom() - 14.0),
            pos2(area.right() - 8.0, area.bottom() - 6.0),
        );
        let total = length + visible * 0.25;
        let thumb_share = (visible / total.max(visible)).clamp(0.05, 1.0);
        let thumb_w = bar.width() * thumb_share;
        let travel = bar.width() - thumb_w;
        let thumb_x = bar.left()
            + if max_scroll > 0.0 {
                travel * first / max_scroll
            } else {
                0.0
            };
        let thumb = Rect::from_min_size(pos2(thumb_x, bar.top()), vec2(thumb_w, bar.height()));
        painter.rect_filled(thumb, 4.0, Color32::from_rgb(90, 90, 96));
        let drag = ui.interact(
            bar,
            ui.id().with("deliver_hscroll"),
            Sense::click_and_drag(),
        );
        if let Some(p) = drag
            .interact_pointer_pos()
            .filter(|_| drag.dragged() || drag.clicked())
        {
            if travel > 0.0 {
                let share = ((p.x - bar.left() - thumb_w / 2.0) / travel).clamp(0.0, 1.0);
                self.deliver_ui.scroll = share * max_scroll;
            }
        }
        if let Some(what) = planned {
            self.planned(what);
        }
        let _ = ctx;
    }

    /// The timeline's size and frame rate (the Project settings window).
    pub(crate) fn format_settings(&mut self, ui: &mut Ui, ctx: &Context) {
        heading(ui, "Format");
        let (mut width, mut height, fps) = (
            self.project().width,
            self.project().height,
            self.project().fps,
        );
        let current = RESOLUTIONS
            .iter()
            .find(|(_, w, h)| *w == width && *h == height)
            .map_or("Custom", |p| p.0);
        egui::ComboBox::from_label("Resolution")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (label, w, h) in RESOLUTIONS {
                    if ui
                        .selectable_label(w == width && h == height, label)
                        .clicked()
                    {
                        self.run(Command::SetFormat {
                            width: w,
                            height: h,
                            fps,
                        });
                    }
                }
            });
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut width)
                        .range(16..=16_384)
                        .suffix(" px"),
                )
                .changed();
            ui.label("x");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut height)
                        .range(16..=16_384)
                        .suffix(" px"),
                )
                .changed();
        });
        if changed {
            self.run_gesture(ctx, Command::SetFormat { width, height, fps });
        }
        let label = crate::fps_prompt::fps_text;
        egui::ComboBox::from_label("Frame rate")
            .selected_text(label(fps))
            .show_ui(ui, |ui| {
                for preset in Fps::PRESETS {
                    if ui.selectable_label(preset == fps, label(preset)).clicked() {
                        self.run(Command::SetFormat {
                            width,
                            height,
                            fps: preset,
                        });
                    }
                }
            });
        ui.label(
            RichText::new("Changing the frame rate keeps every clip on the same frames.")
                .color(theme::TEXT_DIM)
                .size(12.0),
        );
    }

    pub(crate) fn export_frame(&mut self, ctx: &Context) {
        let (w, h) = (self.project().width, self.project().height);
        // What an export would read: the files themselves, unless the page was told to use the
        // optimised copies.
        let frame = match render_frame(
            self.session.project(),
            &self.export_cache().blocking(),
            self.playhead,
            w,
            h,
        )
        .and_then(|f| encode_png(&f))
        {
            Ok(png) => png,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let name = format!("frame_{:06}.png", self.playhead);
        platform::save_as(
            self.inbox.clone(),
            ctx.clone(),
            name,
            ("PNG image", vec!["png"]),
            frame,
            false,
        );
    }
}

/// The menu of the timeline: the in and out at the playhead, or the whole of it.
fn marks_menu(ui: &mut Ui, chosen: &mut Option<crate::menu::Act>, entire: &mut bool) {
    if ui.button("Mark In").clicked() {
        *chosen = Some(crate::menu::Act::MarkIn);
        ui.close();
    }
    if ui.button("Mark Out").clicked() {
        *chosen = Some(crate::menu::Act::MarkOut);
        ui.close();
    }
    if ui.button("Entire Timeline").clicked() {
        *entire = true;
        ui.close();
    }
}

/// The y of the top of the ruler in window positions.
fn y_ruler_pos(s: &Screen, y_ruler: f32) -> f32 {
    s.p(0.0, y_ruler).y
}

#[cfg(test)]
mod default_rate_tests {
    #[test]
    fn exports_start_at_60_frames_per_second() {
        let presets = picasso_core::Fps::PRESETS;
        assert_eq!(
            presets.get(super::DEFAULT_FPS_CHOICE - 1),
            Some(&picasso_core::Fps::FPS_60)
        );
    }
}
