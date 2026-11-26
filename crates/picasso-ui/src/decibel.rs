//! The Audio page (the music tab): a console with level meters, a loudness panel, a transport
//! row, a tool row, a timeline and a mixer, laid out like the reference screen. Positions are
//! written in pixels of the 2000-pixel-wide reference and scaled to the 1872-point window
//! (`K`) and then onto Resolve's 1080p columns and rows (`at`); `at` measures from the top of the window, `from_bottom` from its bottom.

#![allow(clippy::too_many_arguments)]

use egui::{Align2, Color32, Context, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};
use picasso_core::{Command, MAX_GAIN_DB, MIN_GAIN_DB};

use crate::app::StudioApp;
use crate::capture::fit;
use crate::chrome::{Screen, Tool, R};
use crate::icons::Icon;
use crate::panels::Split;
use crate::theme;

/// Points per reference pixel.
const K: f32 = 0.936;
/// Points per reference pixel for sizes (widths, radii): Resolve at 1080p draws the page about
/// two thirds the size of the older reference.
const Q: f32 = 0.64 * R;
/// The same for text and icons.
const SZ: f32 = 0.68 * R;

/// Where the older reference's columns land on Resolve's at 1080p, for the console (x, in its pixels)...
const CONSOLE_X: [(f32, f32); 6] = [
    (3.0, 0.0),
    (873.0, 1076.0),
    (977.0, 1143.0),
    (1147.0, 1253.0),
    (1447.0, 1444.0),
    (2000.0, 1911.0),
];
/// ...for the transport row...
const TRANSPORT_X: [(f32, f32); 19] = [
    (3.0, 0.0),
    (95.0, 60.0),
    (422.0, 270.0),
    (537.0, 343.0),
    (734.0, 685.0),
    (790.0, 721.0),
    (851.0, 755.0),
    (900.0, 787.0),
    (950.0, 819.0),
    (1006.0, 853.0),
    (1050.0, 880.0),
    (1107.0, 918.0),
    (1167.0, 958.0),
    (1531.0, 1615.0),
    (1643.0, 1688.0),
    (1746.0, 1752.0),
    (1778.0, 1772.0),
    (1900.0, 1850.0),
    (2000.0, 1911.0),
];
/// ...for the tool row...
const TOOLS_X: [(f32, f32); 17] = [
    (3.0, 0.0),
    (358.0, 230.0),
    (418.0, 267.0),
    (479.0, 306.0),
    (685.0, 715.0),
    (748.0, 756.0),
    (810.0, 796.0),
    (891.0, 887.0),
    (955.0, 928.0),
    (1015.0, 966.0),
    (1086.0, 1052.0),
    (1154.0, 1096.0),
    (1249.0, 1157.0),
    (1310.0, 1195.0),
    (1351.0, 1221.0),
    (1695.0, 1440.0),
    (2000.0, 1911.0),
];
/// ...and for the timeline with its headers and mixer.
const TIMELINE_X: [(f32, f32); 4] = [
    (3.0, 0.0),
    (358.0, 230.0),
    (1783.0, 1712.0),
    (2000.0, 1911.0),
];
/// The older reference's rows that land on Resolve's: the console, the transport row, the tool
/// row, the ruler. Below the ruler a row of 94 pixels becomes 60.
const ROWS: [(f32, f32); 5] = [
    (99.0, 37.0),
    (413.0, 237.0),
    (470.0, 275.0),
    (552.0, 325.0),
    (612.0, 365.0),
];

fn piece(map: &[(f32, f32)], v: f32) -> f32 {
    for w in map.windows(2) {
        let ((a0, b0), (a1, b1)) = (w[0], w[1]);
        if v <= a1 {
            return b0 + (v - a0) / (a1 - a0) * (b1 - b0);
        }
    }
    let (a, b) = map[map.len() - 1];
    b + (v - a)
}

const PANEL: Color32 = Color32::from_rgb(28, 28, 31);
const HEADER: Color32 = Color32::from_rgb(34, 33, 38);
const LINE: Color32 = Color32::from_rgb(14, 14, 17);
const DIM: Color32 = Color32::from_rgb(96, 96, 102);
const TEXT: Color32 = Color32::from_rgb(214, 214, 218);
const SCALE: Color32 = Color32::from_rgb(98, 98, 104);
const BLUE: Color32 = Color32::from_rgb(70, 140, 235);
const TOOL: Color32 = Color32::from_rgb(150, 150, 154);
const GREEN: Color32 = Color32::from_rgb(64, 170, 110);

/// The height of a track row (and its header), in reference pixels.
const ROW: f32 = 94.0;

/// A drawing surface that takes reference pixels.
struct Canvas<'a> {
    p: &'a Painter,
    s: &'a Screen,
    /// The bottom of the page body, in points.
    bl: f32,
}

impl Canvas<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        let (x, y) = self.at_points(x, y);
        self.s.p(x, y)
    }
    /// `at` as the screen's own points, before the dividers move them.
    fn at_points(&self, x: f32, y: f32) -> (f32, f32) {
        let map = if y <= 413.0 {
            &CONSOLE_X[..]
        } else if y <= 470.0 {
            &TRANSPORT_X[..]
        } else if y <= 552.0 {
            &TOOLS_X[..]
        } else {
            &TIMELINE_X[..]
        };
        let rx = piece(map, x);
        let ry = if y >= 1100.0 {
            return (rx * R, self.bl);
        } else if y > 612.0 {
            365.0 + (y - 612.0) * 0.638
        } else {
            piece(&ROWS, y)
        };
        self.rv_points(rx, ry)
    }
    /// A point given in pixels of Resolve at 1080p.
    fn rv(&self, x: f32, y: f32) -> Pos2 {
        let (x, y) = self.rv_points(x, y);
        self.s.p(x, y)
    }
    /// `rv` as the screen's own points, before the dividers move them.
    fn rv_points(&self, x: f32, y: f32) -> (f32, f32) {
        if y >= 1000.0 {
            return (x * R, self.bl);
        }
        (x * R, 92.0 + (y - 37.0) * self.vf())
    }
    /// Points per pixel of Resolve's rows: its 963 pixels of body fill the window's.
    fn vf(&self) -> f32 {
        (self.bl - 92.0).max(300.0) / 963.0
    }
    fn rr(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        self.s
            .between(self.rv_points(x0, y0), self.rv_points(x1, y1))
    }
    fn rband(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.p.rect_filled(self.rr(x0, y0, x1, y1), 0.0, c);
    }
    fn rtext(&self, x: f32, y: f32, align: Align2, text: &str, size: f32, c: Color32, bold: bool) {
        let (px, py) = self.rv_points(x, y);
        let size = size * R * self.s.shrink(px, py);
        let font = if bold {
            theme::semibold(size)
        } else {
            theme::regular(size)
        };
        crate::i18n::paint_text(&self.p,self.rv(x, y), align, text, font, c);
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        self.s
            .between(self.at_points(x0, y0), self.at_points(x1, y1))
    }
    fn band(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.p.rect_filled(self.rect(x0, y0, x1, y1), 0.0, c);
    }
    fn text(&self, x: f32, y: f32, align: Align2, text: &str, width: f32, c: Color32, bold: bool) {
        crate::i18n::paint_text(&self.p,
            self.at(x, y),
            align,
            text,
            fit(self.p, text, width * Q * self.shrink(x, y), bold),
            c,
        );
    }
    /// Text at a fixed size in points.
    fn label(&self, x: f32, y: f32, align: Align2, text: &str, size: f32, c: Color32) {
        crate::i18n::paint_text(&self.p,
            self.at(x, y),
            align,
            text,
            theme::regular(size * SZ * self.shrink(x, y)),
            c,
        );
    }
    /// How much smaller than usual text and icons at a reference point are drawn: as much as
    /// the dividers squeezed the panel there, so they stay inside it.
    fn shrink(&self, x: f32, y: f32) -> f32 {
        let (x, y) = self.at_points(x, y);
        self.s.shrink(x, y)
    }
    fn line(&self, a: (f32, f32), b: (f32, f32), w: f32, c: Color32) {
        let ends = self
            .s
            .segment(self.at_points(a.0, a.1), self.at_points(b.0, b.1));
        self.p.line_segment(ends, Stroke::new(w, c));
    }
    fn vline(&self, x: f32, y0: f32, y1: f32, c: Color32) {
        self.line((x, y0), (x, y1), 1.0, c);
    }
    fn icon(&self, x: f32, y: f32, icon: Icon, c: Color32, size: f32) {
        let size = size * SZ / 0.936 * 0.936 * self.shrink(x, y);
        crate::icons::paint_scaled(self.p, self.at(x, y), icon, c, size);
    }
    fn poly(&self, pts: &[(f32, f32)], c: Color32) {
        self.p.add(Shape::convex_polygon(
            pts.iter().map(|(x, y)| self.at(*x, *y)).collect(),
            c,
            Stroke::NONE,
        ));
    }
    fn path(&self, pts: &[(f32, f32)], w: f32, c: Color32) {
        self.p.add(Shape::line(
            pts.iter().map(|(x, y)| self.at(*x, *y)).collect(),
            Stroke::new(w, c),
        ));
    }
    fn circle(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_filled(self.at(x, y), r * Q, c);
    }
    fn ring(&self, x: f32, y: f32, r: f32, w: f32, c: Color32) {
        self.p
            .circle_stroke(self.at(x, y), r * Q, Stroke::new(w, c));
    }
    fn outline(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, w: f32, c: Color32) {
        self.p.rect_stroke(
            self.rect(x0, y0, x1, y1),
            r * Q,
            Stroke::new(w, c),
            egui::StrokeKind::Inside,
        );
    }
}

/// How full a meter bar is for a level (1.0 is 0 dB), from -50 dB at the bottom.
fn fill_of(level: f32) -> f32 {
    let db = 20.0 * level.max(1e-6).log10();
    ((db + 50.0) / 50.0).clamp(0.0, 1.0)
}

impl StudioApp {
    /// The first toolbar of the Audio page.
    pub(crate) fn decibel_toolbar1(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen, y: f32) {
        // (x in reference pixels, icon, hint)
        let left: [(f32, Icon, &str); 6] = [
            (45.0, Icon::PhotoMusic, "Import media"),
            (125.0, Icon::Wand, "Effects"),
            (203.0, Icon::ListBullets, "Index"),
            (283.0, Icon::SubtitleBox, "Subtitles"),
            (364.0, Icon::MusicLines, "Sound library"),
            (444.0, Icon::Mic, "Record"),
        ];
        let mut planned: Option<&str> = None;
        for (x, icon, hint) in left {
            let size = if icon == Icon::Mic { 0.85 } else { 0.9 };
            let tool = Tool::new(hint)
                .size(size)
                .color(if icon == Icon::PhotoMusic {
                    Color32::from_rgb(196, 196, 200)
                } else {
                    theme::ICON
                });
            if self.tool(ui, s.p(x * K, y), icon, tool).clicked() {
                match icon {
                    Icon::PhotoMusic => self.import_dialog(ctx),
                    Icon::Wand => self.page = crate::app::Page::Effects,
                    Icon::ListBullets => self.do_act(ctx, crate::menu::Act::ShowEditIndex),
                    Icon::SubtitleBox => self.do_act(ctx, crate::menu::Act::CreateSubtitleRegion),
                    Icon::Mic => self.toggle_recording(),
                    _ => planned = Some(hint),
                }
            }
        }
        let right: [(f32, Icon, &str, f32); 4] = [
            (1724.0, Icon::Mixer, "Mixer", 0.9),
            (1803.0, Icon::Bars, "Meters", 0.9),
            (1883.0, Icon::LabelTag, "Labels", 0.9),
            (1962.0, Icon::Tools, "Project settings", 0.82),
        ];
        for (x, icon, hint, size) in right {
            if self
                .tool(ui, s.p(x * K, y), icon, Tool::new(hint).size(size))
                .clicked()
            {
                if icon == Icon::Tools {
                    self.show_settings = true;
                } else {
                    planned = Some(hint);
                }
            }
        }
        if let Some(what) = planned {
            self.planned(what);
        }
    }

    pub(crate) fn decibel_page(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        let painter = ui.painter().clone();
        let bottom_line = s.h - 46.8;
        // The dividers: under the console (everything below moves with it, and the timeline
        // gets what is left), the meters and the video in the console, the track headers and
        // the mixer beside the timeline.
        let splits = {
            let c = Canvas {
                p: &painter,
                s,
                bl: bottom_line,
            };
            let (left, right) = (c.at(3.0, 200.0).x, c.at(2000.0, 200.0).x);
            let (console, below) = (c.at(3.0, 413.0).y, bottom_line + s.o.y);
            [
                Split::column(
                    "audio meters",
                    s,
                    c.at(873.0, 99.0),
                    c.at(873.0, 411.0),
                    left,
                    c.at(1447.0, 200.0).x,
                ),
                Split::column(
                    "audio video",
                    s,
                    c.at(1447.0, 99.0),
                    c.at(1447.0, 411.0),
                    c.at(1147.0, 200.0).x,
                    right,
                ),
                Split::column(
                    "audio headers",
                    s,
                    // From the tool row down, as the column's line and its timecodes are.
                    c.at(358.0, 470.0),
                    c.at(358.0, 1100.0),
                    left,
                    c.at(1783.0, 700.0).x,
                ),
                Split::column(
                    "audio mixer",
                    s,
                    c.at(1783.0, 553.0),
                    c.at(1783.0, 1100.0),
                    c.at(358.0, 700.0).x,
                    right,
                ),
                Split::row(
                    "audio console",
                    s,
                    c.at(3.0, 412.0),
                    c.at(2000.0, 412.0),
                    c.at(3.0, 99.0).y,
                    below,
                )
                .keeping(below - console - 1.0),
            ]
        };
        let warped = self.warped(s, &splits);
        let s = &warped;
        let c = Canvas {
            p: &painter,
            s,
            bl: bottom_line,
        };
        // ---- the console: meters, buses and loudness (reference y 99 to 413)
        c.band(3.0, 99.0, 2000.0, 413.0, theme::VIEWER_BG);
        self.meters_strip(&c);
        self.bus_panel(&c);
        self.control_room_panel(&c);
        self.loudness_panel(ui, &c);
        c.band(1447.0, 99.0, 2000.0, 413.0, Color32::BLACK);
        let aspect = self.project().width as f32 / self.project().height.max(1) as f32;
        let has_picture = self.project().duration() > 0;
        let overlays = self.overlays;
        self.viewer.paint(
            ui,
            c.rect(1449.0, 101.0, 2000.0, 411.0),
            aspect,
            Color32::BLACK,
            has_picture,
            overlays,
        );
        c.vline(873.0, 99.0, 413.0, LINE);
        c.vline(977.0, 99.0, 413.0, LINE);
        c.vline(1147.0, 99.0, 413.0, LINE);
        c.vline(1447.0, 99.0, 413.0, LINE);
        c.icon(
            1487.0,
            377.0,
            Icon::Sliders,
            Color32::from_rgb(190, 190, 194),
            0.85,
        );
        c.outline(
            1465.0,
            360.0,
            1510.0,
            396.0,
            3.0,
            1.0,
            Color32::from_rgb(60, 60, 64),
        );
        c.p.rect_stroke(
            c.rect(1941.0, 361.0, 1985.0, 393.0),
            0.0,
            Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        c.icon(
            1963.0,
            377.0,
            Icon::WindowLayout,
            Color32::from_rgb(190, 190, 194),
            0.9,
        );

        // ---- the transport row (413 to 470)
        c.band(3.0, 413.0, 2000.0, 470.0, HEADER);
        c.band(3.0, 411.0, 2000.0, 413.0, LINE);
        self.decibel_transport(ui, &c);

        // ---- the tool row (470 to 552) and the timecodes on its left
        c.band(3.0, 470.0, 2000.0, 552.0, HEADER);
        c.band(3.0, 470.0, 358.0, 612.0, Color32::from_rgb(30, 29, 33));
        c.band(3.0, 468.0, 2000.0, 470.0, LINE);
        let (mark_in, mark_out) = (self.project().mark_in, self.project().mark_out);
        let span = match (mark_in, mark_out) {
            (Some(a), Some(b)) => b.saturating_sub(a),
            _ => 0,
        };
        let codes = [
            self.tc(mark_in.unwrap_or(0) as i64),
            self.tc(mark_out.unwrap_or(0) as i64),
            self.project().fps.timecode(span),
        ];
        for (i, (y, code)) in [495.0, 542.0, 590.0].into_iter().zip(codes).enumerate() {
            if i > 0 {
                c.band(3.0, y - 24.0, 358.0, y - 23.0, LINE);
            }
            c.text(109.0, y, Align2::LEFT_CENTER, &code, 109.0, TEXT, false);
        }
        c.icon(59.0, 494.0, Icon::SkipStartSmall, DIM, 0.8);
        c.icon(59.0, 541.0, Icon::SkipEndSmall, DIM, 0.8);
        c.ring(60.0, 589.0, 8.0, 1.1, DIM);
        c.path(&[(60.0, 584.0), (60.0, 589.0), (64.0, 591.0)], 1.1, DIM);
        c.vline(358.0, 470.0, 1100.0, LINE);
        self.decibel_tools(ui, &c);

        // ---- the timeline (552 to the bottom) and the mixer on its right
        let tl = Rect::from_min_max(
            c.at(360.0, 552.0),
            Pos2::new(c.at(1783.0, 600.0).x, bottom_line),
        );
        c.band(3.0, 612.0, 358.0, 1100.0, Color32::from_rgb(30, 29, 33));
        self.decibel_timeline(ui, ctx, &c, tl);
        self.decibel_headers(ui, ctx, &c);
        self.decibel_mixer(ui, ctx, &c, bottom_line);
        self.dividers(ui, s, &splits);
    }

    fn meters_strip(&self, c: &Canvas) {
        c.band(3.0, 99.0, 873.0, 413.0, theme::VIEWER_BG);
        // Resolve at 1080p: 34 meters 30 pixels apart, the scale on the left.
        for (label, y) in [
            ("0", 85.0),
            ("-5", 103.0),
            ("-10", 120.0),
            ("-15", 137.0),
            ("-20", 154.0),
            ("-30", 181.0),
            ("-40", 203.0),
            ("-50", 213.0),
        ] {
            c.rtext(23.0, y, Align2::RIGHT_CENTER, label, 8.0, SCALE, false);
            c.p.circle_filled(c.rv(27.0, y), 0.9 * R, SCALE);
        }
        let audio: Vec<usize> = self
            .project()
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == picasso_core::TrackKind::Audio)
            .map(|(i, _)| i)
            .collect();
        for n in 0..34usize {
            let x = 36.5 + 30.0 * n as f32;
            if n < audio.len() {
                c.rband(x - 9.0, 48.0, x + 9.0, 50.0, GREEN);
                c.rtext(
                    x,
                    64.0,
                    Align2::CENTER_CENTER,
                    &(n + 1).to_string(),
                    8.0,
                    SCALE,
                    false,
                );
            }
            c.p.rect_stroke(
                c.rr(x - 6.0, 84.0, x + 6.0, 224.0),
                1.0,
                Stroke::new(1.0, Color32::from_rgb(62, 62, 66)),
                egui::StrokeKind::Inside,
            );
            c.rband(x - 5.0, 85.0, x + 5.0, 223.0, Color32::from_rgb(22, 22, 24));
            for t in 0..8 {
                let y = 85.0 + 18.0 * t as f32;
                for (a, b) in [(x - 15.0, x - 9.0), (x + 9.0, x + 15.0)] {
                    c.rband(a, y, b, y + 1.0, Color32::from_rgb(66, 66, 70));
                }
            }
            if let Some(i) = audio.get(n) {
                let (l, r) = self.track_meters.get(*i).copied().unwrap_or((0.0, 0.0));
                let h = fill_of(l.max(r)) * 138.0;
                if h > 0.5 {
                    c.rband(
                        x - 5.0,
                        223.0 - h,
                        x + 5.0,
                        223.0,
                        Color32::from_rgb(70, 190, 100),
                    );
                }
            }
        }
    }

    fn bus_panel(&self, c: &Canvas) {
        c.band(873.0, 99.0, 977.0, 413.0, PANEL);
        c.band(905.0, 119.0, 951.0, 122.0, BLUE);
        c.text(
            927.0,
            142.0,
            Align2::CENTER_CENTER,
            "Bus 1",
            43.0,
            TEXT,
            false,
        );
        scale_labels(c, 890.0, 175.0, 391.0);
        let (l, r) = self.meter;
        two_bars(c, 920.0, 943.0, 175.0, 391.0, l, r);
    }

    fn control_room_panel(&self, c: &Canvas) {
        c.band(977.0, 99.0, 1147.0, 413.0, PANEL);
        c.band(977.0, 99.0, 1147.0, 137.0, HEADER);
        c.text(
            1054.0,
            118.0,
            Align2::CENTER_CENTER,
            "Control Room",
            121.0,
            TEXT,
            false,
        );
        c.text(1005.0, 154.0, Align2::LEFT_CENTER, "TP", 23.0, TEXT, false);
        let tp = self
            .loudness
            .true_peak_db()
            .map_or("-100".to_string(), |db| format!("{db:.1}"));
        c.text(
            1040.0,
            154.0,
            Align2::LEFT_CENTER,
            &tp,
            9.5 * tp.len() as f32,
            BLUE,
            false,
        );
        scale_labels(c, 1028.0, 175.0, 391.0);
        let peak = self.meter.0.max(self.meter.1);
        bar(c, 1062.0, 1076.0, 175.0, 391.0, peak);
    }

    fn loudness_panel(&mut self, ui: &mut Ui, c: &Canvas) {
        let pause = c.rect(1160.0, 368.0, 1240.0, 398.0);
        let reset = c.rect(1254.0, 368.0, 1337.0, 398.0);
        if ui
            .interact(pause, ui.id().with("loud_pause"), Sense::click())
            .on_hover_text("Stop or go on measuring")
            .clicked()
        {
            self.loudness_paused = !self.loudness_paused;
        }
        if ui
            .interact(reset, ui.id().with("loud_reset"), Sense::click())
            .on_hover_text("Start measuring again")
            .clicked()
        {
            self.loudness.reset();
        }
        c.band(1147.0, 99.0, 1447.0, 413.0, PANEL);
        c.band(1147.0, 99.0, 1447.0, 137.0, HEADER);
        c.text(
            1161.0,
            117.0,
            Align2::LEFT_CENTER,
            "Loudness",
            83.0,
            TEXT,
            false,
        );
        c.text(
            1277.0,
            117.0,
            Align2::LEFT_CENTER,
            "BS.1770-1 (LU)",
            116.0,
            DIM,
            false,
        );
        for dx in [0.0, 12.0, 24.0] {
            c.circle(1409.0 + dx, 118.0, 2.2, TEXT);
        }
        c.text(1188.0, 154.0, Align2::LEFT_CENTER, "M", 15.0, TEXT, false);
        // Values in LU: relative to the -23 LUFS target.
        let lu = |v: Option<f64>| v.map(|l| l - picasso_audio::TARGET_LUFS);
        let shown = |v: Option<f64>| v.map_or("--".to_string(), |x| format!("{x:.1}"));
        let momentary = lu(self.loudness.momentary());
        let m_text = momentary.map_or("---".to_string(), |x| format!("{x:.1}"));
        c.text(
            1288.0,
            155.0,
            Align2::LEFT_CENTER,
            &m_text,
            8.0 * m_text.len() as f32,
            BLUE,
            false,
        );
        for (label, x, y) in [
            ("+9", 1253.0, 176.0),
            ("0", 1257.0, 232.0),
            ("-9", 1254.0, 289.0),
            ("-18", 1252.0, 343.0),
        ] {
            c.label(x, y, Align2::CENTER_CENTER, label, 12.0, TEXT);
        }
        for n in 0..10 {
            let y = 180.0 + 18.6 * n as f32;
            for x in [1167.0, 1222.0, 1273.0] {
                c.line((x - 3.0, y), (x + 3.0, y), 1.0, SCALE);
            }
        }
        // The momentary level (two bars) and the short-term one, on the +9 to -18 LU scale.
        let short = lu(self.loudness.short_term());
        lu_bar(c, 1183.0, 1196.0, 175.0, 348.0, momentary);
        lu_bar(c, 1198.0, 1211.0, 175.0, 348.0, momentary);
        lu_bar(c, 1290.0, 1306.0, 175.0, 348.0, short);
        let values = [
            shown(short),
            shown(lu(self.loudness.short_max())),
            shown(self.loudness.range()),
            shown(lu(self.loudness.integrated())),
        ];
        for ((label, y), value) in [
            ("Short", 163.0),
            ("Short Max", 228.0),
            ("Range", 292.0),
            ("Integrated", 356.0),
        ]
        .into_iter()
        .zip(values)
        {
            let w = match label {
                "Short" => 39.0,
                "Short Max" => 75.0,
                "Range" => 44.0,
                _ => 75.0,
            };
            c.text(1351.0, y, Align2::LEFT_CENTER, label, w, TEXT, false);
            c.text(
                1352.0,
                y + 22.0,
                Align2::LEFT_CENTER,
                &value,
                8.0 * value.len() as f32,
                BLUE,
                false,
            );
        }
        let paused = self.loudness_paused;
        for (label, x0, x1, w) in [
            (
                if paused { "Resume" } else { "Pause" },
                1160.0,
                1240.0,
                48.0,
            ),
            ("Reset", 1254.0, 1337.0, 45.0),
        ] {
            c.outline(
                x0,
                368.0,
                x1,
                398.0,
                15.0,
                1.0,
                Color32::from_rgb(70, 70, 76),
            );
            c.text(
                (x0 + x1) / 2.0,
                383.0,
                Align2::CENTER_CENTER,
                label,
                w,
                Color32::from_rgb(128, 128, 134),
                false,
            );
        }
    }

    fn decibel_transport(&mut self, ui: &mut Ui, c: &Canvas) {
        let tc = self.tc(self.playhead as i64);
        c.text(
            95.0,
            441.0,
            Align2::LEFT_CENTER,
            &tc,
            167.0,
            theme::WHITE,
            true,
        );
        let name = self.project().timeline_name.clone();
        c.label(422.0, 442.0, Align2::LEFT_CENTER, &name, 19.0, theme::WHITE);
        c.path(
            &[(530.0, 438.0), (537.0, 446.0), (544.0, 438.0)],
            1.6,
            Color32::from_rgb(150, 150, 154),
        );
        let picker = ui
            .interact(
                c.rect(410.0, 420.0, 555.0, 464.0),
                ui.id().with("fl_timeline_pick"),
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
        // The transport buttons.
        for (x, hint) in [
            (734.0, "Rewind"),
            (790.0, "Fast forward"),
            (851.0, "Play (Space)"),
            (900.0, "Stop"),
            (950.0, "Record"),
            (1006.0, "Loop"),
        ] {
            let r = c.rect(x - 22.0, 422.0, x + 22.0, 462.0);
            if ui
                .interact(r, ui.id().with(("fl_tr", x as i32)), Sense::click())
                .on_hover_text(hint)
                .clicked()
            {
                match hint {
                    "Rewind" => self.go_to(
                        self.playhead
                            .saturating_sub(self.project().fps.nominal() * 5),
                    ),
                    "Fast forward" => self.go_to(self.playhead + self.project().fps.nominal() * 5),
                    "Play (Space)" => self.toggle_play(),
                    "Stop" => self.playing = false,
                    "Loop" => self.looping = !self.looping,
                    _ => self.toggle_recording(),
                }
            }
        }
        let g = Color32::from_rgb(150, 150, 154);
        // Rewind and fast forward: two triangles each.
        let x = 741.0;
        c.poly(&[(x + 14.0, 430.0), (x + 14.0, 452.0), (x + 1.0, 441.0)], g);
        c.poly(&[(x, 430.0), (x, 452.0), (x - 13.0, 441.0)], g);
        let x = 797.0;
        c.poly(&[(x - 14.0, 430.0), (x - 14.0, 452.0), (x - 1.0, 441.0)], g);
        c.poly(&[(x, 430.0), (x, 452.0), (x + 13.0, 441.0)], g);
        // Play, stop, record, loop.
        c.poly(&[(844.0, 430.0), (844.0, 452.0), (862.0, 441.0)], g);
        c.band(890.0, 431.0, 911.0, 452.0, g);
        // The record button glows red while recording, brighter with the input's level.
        let record = if self.recording.active() {
            let lit = (0.55 + self.recording.level.min(1.0) * 0.45) * 255.0;
            Color32::from_rgb(lit as u8, 40, 36)
        } else {
            g
        };
        c.circle(950.0, 441.0, 10.0, record);
        let l = Icon::Loop;
        c.icon(1006.0, 441.0, l, g, 0.95);
        c.vline(1050.0, 420.0, 462.0, LINE);
        c.icon(1107.0, 442.0, Icon::FaderGear, g, 0.95);
        c.icon(1167.0, 442.0, Icon::FaderGear2, g, 0.95);
        c.text(
            1531.0,
            442.0,
            Align2::LEFT_CENTER,
            "Bus 1",
            50.0,
            Color32::from_rgb(120, 120, 126),
            false,
        );
        c.path(
            &[(1602.0, 442.0), (1621.0, 442.0)],
            1.0,
            Color32::from_rgb(120, 120, 126),
        );
        c.path(
            &[(1616.0, 437.0), (1622.0, 442.0), (1616.0, 447.0)],
            1.0,
            Color32::from_rgb(120, 120, 126),
        );
        c.text(
            1643.0,
            442.0,
            Align2::LEFT_CENTER,
            "Auto",
            44.0,
            Color32::from_rgb(170, 170, 174),
            false,
        );
        // The speakers: silent or not, their volume, and DIM.
        let silent = self.monitor.bus_muted;
        c.icon(
            1746.0,
            441.0,
            Icon::Speaker,
            if silent {
                Color32::from_rgb(200, 70, 56)
            } else {
                theme::WHITE
            },
            0.9,
        );
        if ui
            .interact(
                c.rect(1726.0, 422.0, 1766.0, 462.0),
                ui.id().with("fl_speaker"),
                Sense::click(),
            )
            .on_hover_text("Mute the speakers")
            .clicked()
        {
            self.set_monitor(|m| m.bus_muted = !m.bus_muted);
        }
        let volume = self.monitor.volume.clamp(0.0, 1.0);
        let knob = 1778.0 + 122.0 * volume;
        c.band(1778.0, 440.0, 1900.0, 443.0, Color32::from_rgb(60, 60, 66));
        c.band(1778.0, 440.0, knob, 443.0, Color32::from_rgb(70, 190, 100));
        c.circle(knob, 442.0, 7.0, Color32::from_rgb(235, 235, 235));
        let slider = ui
            .interact(
                c.rect(1772.0, 428.0, 1906.0, 456.0),
                ui.id().with("fl_volume"),
                Sense::click_and_drag(),
            )
            .on_hover_text("Volume of the speakers");
        if let Some(p) = slider
            .interact_pointer_pos()
            .filter(|_| slider.clicked() || slider.dragged())
        {
            let (a, b) = (c.at(1778.0, 0.0).x, c.at(1900.0, 0.0).x);
            let v = ((p.x - a) / (b - a)).clamp(0.0, 1.0);
            self.set_monitor(|m| m.volume = v);
        }
        let dim = self.monitor.dim;
        if dim {
            c.p.rect_filled(
                c.rect(1919.0, 428.0, 1981.0, 456.0),
                14.0 * Q,
                theme::SOLO,
            );
        }
        c.outline(
            1919.0,
            428.0,
            1981.0,
            456.0,
            14.0,
            1.0,
            Color32::from_rgb(90, 90, 96),
        );
        c.text(
            1950.0,
            442.0,
            Align2::CENTER_CENTER,
            "DIM",
            36.0,
            if dim { Color32::BLACK } else { theme::WHITE },
            false,
        );
        if ui
            .interact(
                c.rect(1919.0, 428.0, 1981.0, 456.0),
                ui.id().with("fl_dim"),
                Sense::click(),
            )
            .on_hover_text("DIM: lower the speakers by 20 dB")
            .clicked()
        {
            self.set_monitor(|m| m.dim = !m.dim);
        }
    }

    fn decibel_tools(&mut self, ui: &mut Ui, c: &Canvas) {
        let g = TOOL;
        // The tool buttons answer clicks with a note; they are drawn here.
        let mut planned: Option<&str> = None;
        let y = 510.0;
        for (x, hint) in [
            (418.0, "Track options"),
            (479.0, "Timeline view"),
            (685.0, "Selection mode"),
            (748.0, "Range mode"),
            (810.0, "Edit mode"),
            (891.0, "Razor"),
            (955.0, "Snapping"),
            (1015.0, "Linked selection"),
            (1249.0, "Waveform zoom"),
            (1310.0, "Track height"),
        ] {
            let r = Rect::from_center_size(c.at(x, y), egui::vec2(46.0, 40.0));
            if ui
                .interact(r, ui.id().with(("fl_tool", x as i32)), Sense::click())
                .on_hover_text(hint)
                .clicked()
            {
                planned = Some(hint);
            }
        }
        // Tracks list with a tick.
        c.icon(418.0, y, Icon::RailChecks, g, 0.9);
        // A grid.
        for i in 0..4 {
            let o = 463.0 + i as f32 * 10.5;
            c.line((o, y - 13.0), (o, y + 13.0), 1.0, g);
        }
        for i in 0..4 {
            let o = y - 13.0 + i as f32 * 8.7;
            c.line((463.0, o), (495.0, o), 1.0, g);
        }
        c.outline(463.0, y - 13.0, 495.0, y + 13.0, 2.0, 1.0, g);
        // The selection arrow in red, the same pointer as the Edit page's. (A hand-made polygon
        // here was concave, and egui fills a convex one, which bent it out of shape.)
        c.icon(686.0, y, Icon::Pointer, theme::TOOL_RED, 0.85);
        // A cross.
        c.line((748.0, y - 13.0), (748.0, y + 13.0), 1.6, g);
        c.line((735.0, y), (761.0, y), 1.6, g);
        // A text cursor with a hand.
        c.line((803.0, y - 11.0), (803.0, y + 9.0), 1.5, g);
        c.line((797.0, y - 11.0), (809.0, y - 11.0), 1.5, g);
        c.line((797.0, y + 9.0), (809.0, y + 9.0), 1.5, g);
        c.circle(822.0, y + 12.0, 5.0, g);
        // Scissors, a magnet and a link.
        c.icon(891.0, y, Icon::Scissors, g, 0.95);
        c.path(
            &[
                (946.0, y - 10.0),
                (946.0, y + 2.0),
                (954.0, y + 10.0),
                (964.0, y + 6.0),
            ],
            2.6,
            theme::WHITE,
        );
        c.line((946.0, y - 11.0), (951.0, y - 13.0), 2.6, theme::WHITE);
        c.icon(1015.0, y, Icon::LinkChain, theme::WHITE, 1.0);
        c.vline(851.0, 490.0, 530.0, LINE);
        c.vline(1055.0, 490.0, 530.0, LINE);
        c.vline(1210.0, 490.0, 530.0, LINE);
        // Two flags, blue.
        for (x, chevron) in [(1086.0, 1116.0), (1154.0, 1185.0)] {
            c.poly(
                &[
                    (x - 8.0, y - 11.0),
                    (x + 8.0, y - 11.0),
                    (x + 8.0, y + 5.0),
                    (x, y + 12.0),
                    (x - 8.0, y + 5.0),
                ],
                BLUE,
            );
            c.path(
                &[
                    (chevron - 5.0, y - 2.0),
                    (chevron, y + 3.0),
                    (chevron + 5.0, y - 2.0),
                ],
                1.2,
                g,
            );
        }
        // A waveform and up-down arrows.
        c.path(
            &[
                (1230.0, y + 4.0),
                (1235.0, y - 10.0),
                (1240.0, y + 12.0),
                (1246.0, y - 8.0),
                (1252.0, y + 6.0),
                (1258.0, y - 4.0),
                (1266.0, y + 4.0),
            ],
            1.5,
            g,
        );
        c.path(
            &[(1305.0, y - 5.0), (1310.0, y - 10.0), (1315.0, y - 5.0)],
            1.4,
            g,
        );
        c.path(
            &[(1305.0, y + 5.0), (1310.0, y + 10.0), (1315.0, y + 5.0)],
            1.4,
            g,
        );
        // Two sliders with a dot.
        for (x0, x1, knob) in [(1351.0, 1485.0, 1351.0), (1553.0, 1695.0, 1598.0)] {
            c.line((x0, y), (x1, y), 1.0, Color32::from_rgb(80, 80, 86));
            c.circle(knob, y, 6.0, Color32::from_rgb(214, 214, 218));
        }
        c.path(&[(1514.0, y - 5.0), (1508.0, y), (1514.0, y + 5.0)], 1.2, g);
        c.path(&[(1525.0, y - 5.0), (1531.0, y), (1525.0, y + 5.0)], 1.2, g);
        if let Some(what) = planned {
            self.planned(what);
        }
    }

    fn decibel_timeline(&mut self, ui: &mut Ui, ctx: &Context, c: &Canvas, area: Rect) {
        let has_tracks = self
            .project()
            .tracks
            .iter()
            .any(|t| t.kind == picasso_core::TrackKind::Audio);
        if has_tracks {
            // With sound tracks the real timeline fills the area.
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                self.timeline_bare(ui, ctx, 60.0 * c.vf())
            });
            return;
        }
        c.p.rect_filled(area, 0.0, Color32::from_rgb(45, 44, 49));
        // The play position: a red line with a flag at the top.
        let x = c.at(362.0, 0.0).x;
        c.p.line_segment(
            [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
            Stroke::new(1.4, Color32::from_rgb(222, 70, 56)),
        );
        c.p.add(Shape::convex_polygon(
            vec![
                Pos2::new(x - 1.0, area.top()),
                Pos2::new(x + 11.0, area.top()),
                Pos2::new(x + 11.0, area.top() + 7.0),
                Pos2::new(x + 5.0, area.top() + 12.0),
                Pos2::new(x - 1.0, area.top() + 12.0),
            ],
            Color32::from_rgb(222, 70, 56),
            Stroke::NONE,
        ));
    }

    /// The headers left of the timeline: each sound track (number, name, level, lock, record,
    /// solo, mute and a meter), then Bus 1.
    fn decibel_headers(&mut self, ui: &mut Ui, ctx: &Context, c: &Canvas) {
        let tracks: Vec<(usize, picasso_core::TrackId, String, f32, bool, bool)> = self
            .project()
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == picasso_core::TrackKind::Audio)
            .map(|(i, t)| (i, t.id, t.name.clone(), t.gain_db, t.muted, t.locked))
            .collect();
        let top = 613.0;
        let rows = tracks.len() + 1;
        for row in 0..rows {
            let y0 = top + ROW * row as f32;
            let bus = row == tracks.len();
            let track = tracks.get(row).cloned();
            c.band(3.0, y0, 6.0, y0 + ROW - 1.0, if bus { BLUE } else { GREEN });
            c.line((3.0, y0 + ROW), (358.0, y0 + ROW), 1.0, LINE);
            c.vline(92.0, y0, y0 + 40.0, LINE);
            c.vline(300.0, y0, y0 + ROW, LINE);
            let (code, name) = match &track {
                Some((_, _, name, ..)) => (name.clone(), format!("Audio {}", row + 1)),
                None => ("B1".to_string(), "Bus 1".to_string()),
            };
            c.label(62.0, y0 + 20.0, Align2::CENTER_CENTER, &code, 15.0, TEXT);
            c.label(110.0, y0 + 20.0, Align2::LEFT_CENTER, &name, 15.0, TEXT);
            c.line((3.0, y0 + 40.0), (300.0, y0 + 40.0), 1.0, LINE);
            // The level, which a drag changes.
            let gain = match &track {
                Some((_, _, _, g, ..)) => *g,
                None => self.monitor.bus_gain_db,
            };
            let field = c.rect(45.0, y0 + 51.0, 112.0, y0 + 81.0);
            c.p.rect_filled(field, 2.0, Color32::from_rgb(22, 22, 25));
            c.p.rect_stroke(
                field,
                2.0,
                Stroke::new(1.0, Color32::from_rgb(52, 52, 58)),
                egui::StrokeKind::Inside,
            );
            crate::i18n::paint_text(&c.p,
                field.center(),
                Align2::CENTER_CENTER,
                format!("{gain:.1}"),
                theme::regular(14.0 * SZ * c.shrink(78.0, y0 + 66.0)),
                TEXT,
            );
            let drag = ui
                .interact(
                    field,
                    ui.id().with(("fl_gain", row)),
                    Sense::click_and_drag(),
                )
                .on_hover_text("Drag to change the level, double-click for 0 dB");
            let new_gain = if drag.double_clicked() {
                Some(0.0)
            } else if drag.dragged() {
                let dy = drag.drag_delta().y;
                (dy != 0.0).then(|| (gain - dy * 0.2).clamp(MIN_GAIN_DB, MAX_GAIN_DB))
            } else {
                None
            };
            if let Some(g) = new_gain {
                match &track {
                    Some((_, id, ..)) => {
                        let pan = self.project().track(*id).map_or(0.0, |t| t.pan);
                        self.run_gesture(
                            ctx,
                            Command::SetTrackMix {
                                track: *id,
                                gain_db: g,
                                pan,
                            },
                        );
                    }
                    None => self.set_monitor(|m| m.bus_gain_db = g),
                }
            }
            // Lock, record, solo and mute; the bus has mute only.
            let buttons: &[(&str, f32)] = if bus {
                &[("M", 276.0)]
            } else {
                &[("L", 145.0), ("R", 188.0), ("S", 232.0), ("M", 276.0)]
            };
            for (what, x) in buttons {
                let lit = match (&track, *what) {
                    (Some((_, id, ..)), "R") => self.armed.contains(id),
                    (Some((_, id, ..)), "S") => self.monitor.solo.contains(id),
                    (Some((.., muted, _)), "M") => *muted,
                    (Some((.., locked)), "L") => *locked,
                    (None, _) => self.monitor.bus_muted,
                    _ => false,
                };
                // As much smaller as the dividers squeezed the headers, so they stay apart.
                let k = c.shrink(*x, y0 + 66.0);
                let r = Rect::from_center_size(
                    c.at(*x, y0 + 66.0),
                    egui::vec2(28.0 * Q * k, 28.0 * Q * k),
                );
                if *what == "L" {
                    // A padlock.
                    let col = if lit { theme::WHITE } else { DIM };
                    c.p.rect_filled(
                        Rect::from_center_size(
                            r.center() + egui::vec2(0.0, 3.0 * k),
                            egui::vec2(12.0 * SZ * k, 9.0 * SZ * k),
                        ),
                        1.5,
                        col,
                    );
                    c.p.circle_stroke(
                        r.center() + egui::vec2(0.0, -2.0 * k),
                        4.0 * k,
                        Stroke::new(1.5, col),
                    );
                } else {
                    let fill = match (*what, lit) {
                        ("R", true) => Color32::from_rgb(214, 60, 50),
                        ("S", true) => theme::SOLO,
                        ("M", true) => Color32::from_rgb(200, 80, 60),
                        _ => Color32::from_rgb(64, 64, 70),
                    };
                    c.p.rect_filled(r, 3.0, fill);
                    crate::i18n::paint_text(&c.p,
                        r.center(),
                        Align2::CENTER_CENTER,
                        *what,
                        theme::regular(12.0 * SZ * k),
                        if lit { theme::WHITE } else { TEXT },
                    );
                }
                let hint = match *what {
                    "L" => "Lock the track",
                    "R" => "Arm for recording",
                    "S" => "Solo: hear only soloed tracks",
                    _ => "Mute",
                };
                if ui
                    .interact(r, ui.id().with(("fl_head", row, *what)), Sense::click())
                    .on_hover_text(hint)
                    .clicked()
                {
                    match (&track, *what) {
                        (Some((_, id, ..)), "L") => {
                            self.run(Command::SetTrackLocked {
                                track: *id,
                                locked: !lit,
                            });
                        }
                        (Some((_, id, ..)), "R") => {
                            if !self.armed.remove(id) {
                                self.armed.insert(*id);
                            }
                        }
                        (Some((_, id, ..)), "S") => {
                            let id = *id;
                            self.set_monitor(|m| {
                                if !m.solo.remove(&id) {
                                    m.solo.insert(id);
                                }
                            });
                        }
                        (Some((_, id, ..)), _) => {
                            self.run(Command::SetTrackMuted {
                                track: *id,
                                muted: !lit,
                            });
                        }
                        (None, _) => self.set_monitor(|m| m.bus_muted = !m.bus_muted),
                    }
                }
            }
            // A meter on the right of the header.
            let (l, r) = match &track {
                Some((i, ..)) => self.track_meters.get(*i).copied().unwrap_or((0.0, 0.0)),
                None => self.meter,
            };
            two_bars(c, 318.0, 340.0, y0 + 10.0, y0 + ROW - 10.0, l, r);
        }
    }

    /// The mixer on the right: Resolve's rows (input, track effects, order, effects, dynamics, EQ,
    /// sends and pan), then the name, the buttons, the level and the fader of each strip. Written
    /// in pixels of Resolve at 1080p.
    fn decibel_mixer(&mut self, ui: &mut Ui, ctx: &Context, c: &Canvas, _bottom_line: f32) {
        let right = 1915.0;
        c.rband(1711.0, 325.0, 1713.0, 1000.0, LINE);
        c.rband(1713.0, 325.0, right + 1.0, 352.0, HEADER);
        c.rtext(
            1720.0,
            338.0,
            Align2::LEFT_CENTER,
            "Mixer",
            12.5,
            theme::WHITE,
            true,
        );
        for dx in [0.0, 8.0, 16.0] {
            c.p.circle_filled(c.rv(1890.0 + dx, 337.0), 1.6 * R, TEXT);
        }
        c.rband(1713.0, 352.0, right + 1.0, 1000.0, PANEL);
        for (label, y) in [
            ("Input", 391.0),
            ("Track FX", 418.0),
            ("Order", 445.0),
            ("Effects", 472.0),
            ("Effects In", 499.0),
            ("Dynamics", 527.0),
            ("EQ", 595.0),
            ("Bus Sends", 635.0),
            ("Pan", 662.0),
        ] {
            c.rtext(1774.0, y, Align2::RIGHT_CENTER, label, 8.5, DIM, false);
        }
        let tracks: Vec<_> = self
            .project()
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == picasso_core::TrackKind::Audio)
            .map(|(i, t)| (i, t.id, t.name.clone(), t.gain_db, t.muted))
            .collect();
        let strips = tracks.len() + 1;
        let box_fill = Color32::from_rgb(40, 40, 44);
        #[allow(clippy::needless_range_loop)]
        for n in 0..strips {
            // The bus is at the right edge; tracks to its left, each 67 pixels wide.
            let x1 = right - 67.0 * (strips - 1 - n) as f32;
            let x0 = x1 - 67.0;
            if x0 < 1780.0 {
                continue;
            }
            let bus = n == strips - 1;
            let (name, gain, muted, level, id) = if bus {
                (
                    "Bus1".to_string(),
                    self.monitor.bus_gain_db,
                    self.monitor.bus_muted,
                    self.meter.0.max(self.meter.1),
                    None,
                )
            } else {
                let (i, id, name, gain, muted) = tracks[n].clone();
                let lv = self.track_meters.get(i).map_or(0.0, |m| m.0.max(m.1));
                (name, gain, muted, lv, Some(id))
            };
            let mid = (x0 + x1) / 2.0;
            c.rband(x0, 351.0, x1, 353.0, if bus { BLUE } else { GREEN });
            c.rtext(mid, 366.0, Align2::CENTER_CENTER, &name, 9.0, TEXT, false);
            let small = |y0: f32, y1: f32| c.rr(x0 + 4.0, y0, x1 - 4.0, y1);
            if !bus {
                for (y, text, color) in [
                    (391.0, "No Input", DIM),
                    (418.0, "Dial Lev", Color32::from_rgb(120, 120, 126)),
                ] {
                    c.p.rect_filled(small(y - 8.0, y + 8.0), 1.0, box_fill);
                    c.rtext(mid, y, Align2::CENTER_CENTER, text, 8.0, color, false);
                }
                c.rtext(
                    mid,
                    445.0,
                    Align2::CENTER_CENTER,
                    "FX DY EQ",
                    8.0,
                    Color32::from_rgb(222, 190, 60),
                    true,
                );
            }
            for (y, text) in [(472.0, "+"), (499.0, "In")] {
                c.p.rect_filled(small(y - 7.0, y + 7.0), 1.0, box_fill);
                c.rtext(mid, y, Align2::CENTER_CENTER, text, 8.0, DIM, false);
            }
            // The dynamics box with its transfer line, and the EQ box.
            let dyn_box = small(520.0, 575.0);
            c.p.rect_filled(dyn_box, 1.0, box_fill);
            c.p.line_segment(
                [dyn_box.left_bottom(), dyn_box.right_top()],
                Stroke::new(1.0, Color32::from_rgb(190, 190, 60)),
            );
            let eq_box = small(587.0, 614.0);
            c.p.rect_filled(eq_box, 1.0, box_fill);
            match id {
                Some(t) => {
                    let fx = self.track_fx_of(t);
                    let color = if fx.eq.is_active() {
                        Color32::from_rgb(90, 200, 120)
                    } else {
                        TOOL
                    };
                    crate::track_fx::draw_eq_curve(c.p, eq_box.shrink(2.0), &fx.eq, color);
                    if fx.dynamics.enabled {
                        c.p.circle_filled(
                            dyn_box.right_top() + egui::vec2(-4.0, 4.0),
                            2.0,
                            Color32::from_rgb(230, 170, 60),
                        );
                    }
                    let r = ui
                        .interact(eq_box, ui.id().with(("strip_eq", n)), Sense::click())
                        .on_hover_text("EQ (right click: Dynamics)");
                    if r.clicked() {
                        self.extra.track_fx = Some((t, crate::track_fx::FxTab::Eq));
                    }
                    if r.secondary_clicked() {
                        self.extra.track_fx = Some((t, crate::track_fx::FxTab::Dynamics));
                    }
                }
                None => {
                    c.p.line_segment(
                        [eq_box.left_center(), eq_box.right_center()],
                        Stroke::new(1.0, TOOL),
                    );
                }
            }
            c.p.rect_filled(small(628.0, 642.0), 1.0, box_fill);
            c.rtext(
                mid,
                635.0,
                Align2::CENTER_CENTER,
                "+",
                8.0,
                Color32::from_rgb(70, 70, 76),
                false,
            );
            if !bus {
                let pan = small(654.0, 708.0);
                c.p.rect_filled(pan, 1.0, box_fill);
                for (dx, dy) in [(0.1, 0.1), (0.5, 0.1), (0.9, 0.1)] {
                    c.p.circle_filled(
                        pan.min + egui::vec2(pan.width() * dx, pan.height() * dy + 2.0),
                        1.3 * R,
                        Color32::from_rgb(70, 190, 100),
                    );
                }
            }
            c.rband(x0 + 2.0, 711.0, x1 - 2.0, 712.0, LINE);
            let (sub, sub_color) = if bus {
                ("Bus 1".to_string(), TEXT)
            } else {
                (format!("Audio {}", n + 1), Color32::from_rgb(226, 150, 60))
            };
            c.rtext(
                mid,
                721.0,
                Align2::CENTER_CENTER,
                &sub,
                9.0,
                sub_color,
                false,
            );
            // Record, solo and mute for a track; mute for the bus.
            let buttons: &[(&str, f32)] = if bus {
                &[("M", 33.0)]
            } else {
                &[("R", 13.0), ("S", 33.0), ("M", 54.0)]
            };
            for (letter, dx) in buttons {
                let lit = match (*letter, id) {
                    ("R", Some(t)) => self.armed.contains(&t),
                    ("S", Some(t)) => self.monitor.solo.contains(&t),
                    ("M", Some(_)) => muted,
                    ("M", None) => self.monitor.bus_muted,
                    _ => false,
                };
                let fill = match (*letter, lit) {
                    ("R", true) => Color32::from_rgb(214, 60, 50),
                    ("S", true) => theme::SOLO,
                    ("M", true) => Color32::from_rgb(200, 80, 60),
                    _ => Color32::from_rgb(64, 64, 70),
                };
                // As much smaller as the dividers squeezed the mixer.
                let (px, py) = c.rv_points(x0 + dx, 748.0);
                let k = c.s.shrink(px, py);
                let r = Rect::from_center_size(
                    c.rv(x0 + dx, 748.0),
                    egui::vec2(15.0 * R * k, 14.0 * R * k),
                );
                c.p.rect_filled(r, 2.0, fill);
                crate::i18n::paint_text(&c.p,
                    r.center(),
                    Align2::CENTER_CENTER,
                    *letter,
                    theme::regular(8.5 * R * k),
                    if lit { theme::WHITE } else { TEXT },
                );
                if ui
                    .interact(r, ui.id().with(("mix_btn", n, *letter)), Sense::click())
                    .on_hover_text(match *letter {
                        "R" => "Arm for recording",
                        "S" => "Solo: hear only soloed tracks",
                        _ => "Mute",
                    })
                    .clicked()
                {
                    match (*letter, id) {
                        ("R", Some(t)) => {
                            if !self.armed.remove(&t) {
                                self.armed.insert(t);
                            }
                        }
                        ("S", Some(t)) => self.set_monitor(|m| {
                            if !m.solo.remove(&t) {
                                m.solo.insert(t);
                            }
                        }),
                        ("M", Some(t)) => {
                            self.run(Command::SetTrackMuted {
                                track: t,
                                muted: !muted,
                            });
                        }
                        _ => self.set_monitor(|m| m.bus_muted = !m.bus_muted),
                    }
                }
            }
            // A small waveform glyph under the buttons.
            c.p.add(Shape::line(
                vec![
                    c.rv(mid - 7.0, 777.0),
                    c.rv(mid - 2.0, 771.0),
                    c.rv(mid + 2.0, 777.0),
                    c.rv(mid + 7.0, 773.0),
                ],
                Stroke::new(1.0, TOOL),
            ));
            c.rtext(
                x0 + 8.0,
                799.0,
                Align2::LEFT_CENTER,
                &format!("{gain:.1}"),
                8.0,
                TEXT,
                false,
            );
            // The fader: 0 dB sits at row 859, the most gain at the top and the least at the bottom.
            let (ty0, zero, ty1) = (811.0, 859.0, 975.0);
            let fader_x = x0 + 15.0;
            let knob_y = if gain >= 0.0 {
                zero - gain / MAX_GAIN_DB * (zero - ty0)
            } else {
                zero + gain / MIN_GAIN_DB * (ty1 - zero)
            };
            c.rband(
                fader_x - 1.0,
                ty0,
                fader_x + 1.0,
                ty1 + 6.0,
                Color32::from_rgb(20, 20, 22),
            );
            for (label, y) in [
                ("0", 815.0),
                ("5", 836.0),
                ("10", 857.0),
                ("15", 878.0),
                ("20", 899.0),
                ("30", 933.0),
                ("40", 959.0),
                ("50", 972.0),
            ] {
                c.rtext(x0 + 38.0, y, Align2::RIGHT_CENTER, label, 8.0, SCALE, false);
                c.p.circle_filled(c.rv(x0 + 42.0, y), 0.9 * R, SCALE);
                c.rband(x0 + 3.0, y - 0.5, x0 + 6.0, y + 0.5, SCALE);
            }
            c.rband(
                fader_x - 6.0,
                knob_y - 6.0,
                fader_x + 6.0,
                knob_y + 6.0,
                Color32::from_rgb(150, 150, 154),
            );
            c.rband(
                fader_x - 5.0,
                knob_y - 0.5,
                fader_x + 5.0,
                knob_y + 0.5,
                Color32::from_rgb(60, 60, 64),
            );
            // The level bar on the right of the strip.
            let (bx0, bx1) = (x1 - 15.0, x1 - 6.0);
            c.p.rect_stroke(
                c.rr(bx0 - 1.0, 805.0, bx1 + 1.0, 986.0),
                1.0,
                Stroke::new(1.0, Color32::from_rgb(62, 62, 66)),
                egui::StrokeKind::Inside,
            );
            c.rband(bx0, 806.0, bx1, 985.0, Color32::from_rgb(22, 22, 24));
            let h = fill_of(level) * 179.0;
            if h > 0.5 {
                c.rband(bx0, 985.0 - h, bx1, 985.0, Color32::from_rgb(70, 190, 100));
            }
            let r = Rect::from_min_max(c.rv(fader_x - 10.0, ty0), c.rv(fader_x + 10.0, ty1));
            let response = ui
                .interact(r, ui.id().with(("fader", n)), Sense::click_and_drag())
                .on_hover_text("Drag to change the level, double-click for 0 dB");
            if response.double_clicked() {
                match id {
                    Some(id) => {
                        let pan = self.project().track(id).map_or(0.0, |t| t.pan);
                        self.run(Command::SetTrackMix {
                            track: id,
                            gain_db: 0.0,
                            pan,
                        });
                    }
                    None => self.set_monitor(|m| m.bus_gain_db = 0.0),
                }
            } else if response.dragged() || response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let row = |v: f32| c.rv(0.0, v).y;
                    let gain_db = if p.y <= row(zero) {
                        (row(zero) - p.y) / (row(zero) - row(ty0)) * MAX_GAIN_DB
                    } else {
                        (p.y - row(zero)) / (row(ty1) - row(zero)) * MIN_GAIN_DB
                    }
                    .clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                    match id {
                        Some(id) => {
                            let pan = self.project().track(id).map_or(0.0, |t| t.pan);
                            self.run_gesture(
                                ctx,
                                Command::SetTrackMix {
                                    track: id,
                                    gain_db,
                                    pan,
                                },
                            );
                        }
                        None => self.set_monitor(|m| m.bus_gain_db = gain_db),
                    }
                }
            }
            c.rband(x0, 353.0, x0 + 1.0, 1000.0, LINE);
        }
    }
}

/// Labels of a level scale (0 at the top, -50 at the bottom) at the reference's rows, for a
/// meter from `y0` to `y1` (175 to 391 in the reference).
fn scale_labels(c: &Canvas, x: f32, y0: f32, y1: f32) {
    for (label, at) in [
        ("0", 178.0),
        ("-5", 206.0),
        ("-10", 233.0),
        ("-15", 260.0),
        ("-20", 286.0),
        ("-30", 330.0),
        ("-40", 362.0),
        ("-50", 378.0),
    ] {
        let y = y0 + (at - 175.0) / (391.0 - 175.0) * (y1 - y0);
        c.label(x + 4.0, y, Align2::RIGHT_CENTER, label, 11.5, SCALE);
        c.circle(x + 10.0, y, 1.3, SCALE);
    }
}

/// A loudness bar on the +9 to -18 LU scale (None draws it empty).
fn lu_bar(c: &Canvas, x0: f32, x1: f32, y0: f32, y1: f32, lu: Option<f64>) {
    c.outline(
        x0 - 1.0,
        y0 - 1.0,
        x1 + 1.0,
        y1 + 1.0,
        1.0,
        1.0,
        Color32::from_rgb(62, 62, 66),
    );
    c.band(x0, y0, x1, y1, Color32::from_rgb(22, 22, 24));
    if let Some(v) = lu {
        let share = ((v + 18.0) / 27.0).clamp(0.0, 1.0) as f32;
        let h = share * (y1 - y0);
        if h > 0.5 {
            // Over the target it turns yellow, past +4 LU red.
            let color = if v > 4.0 {
                Color32::from_rgb(220, 70, 56)
            } else if v > 0.0 {
                Color32::from_rgb(222, 190, 60)
            } else {
                Color32::from_rgb(70, 190, 100)
            };
            c.band(x0, y1 - h, x1, y1, color);
        }
    }
}

/// A vertical level bar with its outline.
fn bar(c: &Canvas, x0: f32, x1: f32, y0: f32, y1: f32, level: f32) {
    c.outline(
        x0 - 1.0,
        y0 - 1.0,
        x1 + 1.0,
        y1 + 1.0,
        1.0,
        1.0,
        Color32::from_rgb(62, 62, 66),
    );
    c.band(x0, y0, x1, y1, Color32::from_rgb(22, 22, 24));
    let h = fill_of(level) * (y1 - y0);
    if h > 0.5 {
        c.band(x0, y1 - h, x1, y1, Color32::from_rgb(70, 190, 100));
    }
}

fn two_bars(c: &Canvas, x0: f32, x1: f32, y0: f32, y1: f32, l: f32, r: f32) {
    let mid = (x0 + x1) / 2.0;
    bar(c, x0, mid - 1.0, y0, y1, l);
    bar(c, mid + 1.0, x1, y0, y1, r);
}
