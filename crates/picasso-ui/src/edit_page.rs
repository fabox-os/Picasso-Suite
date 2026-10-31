//! The Edit page (the timeline tab), laid out like the reference screen: the bins and the media
//! pool on the left, the source viewer and the timeline viewer side by side, their transport
//! rows, the timeline's tool row, and the timeline with its column of track headers. The
//! inspector takes the timeline viewer's place when it is open.
//!
//! Positions are written in pixels of the 2000-pixel-wide reference and scaled to the
//! 1872-point window (`K`). The bars keep their reference heights; the viewers and the tracks
//! share what is left, in the reference's proportions. The dividers between the columns and the
//! one between the viewers and the timeline can be dragged (`crate::panels`).

#![allow(clippy::too_many_arguments)]

use egui::{
    pos2, vec2, Align2, Color32, Context, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui,
    UiBuilder,
};
use picasso_core::{BlendMode, ColorGrade, Command, Transform, MAX_SPEED, MIN_SPEED};

use crate::app::StudioApp;
use crate::capture::fit;
use crate::chrome::{Screen, Tool};
use crate::icons::Icon;
use crate::menu::Act;
use crate::panels::{divider, Divider, Panels};
use crate::theme;
use crate::timeline::Mode;
use crate::widgets::heading;

/// Points per reference pixel (the older reference's, which the page is written in).
const K: f32 = 0.936;
/// Points per pixel of the older reference for the rows that keep their height; Resolve at
/// 1080p draws them about two thirds the size.
const KY: f32 = 0.61;
/// How much smaller than the older reference text and icons are drawn.
const SZ: f32 = 0.66;
/// Where the columns of the older reference (x) land on Resolve's at 1080p (x, in its pixels):
/// the bins, the media pool, the source viewer and the timeline viewer. The three in the middle
/// move with the dividers.
const COLUMNS: [(f32, f32); 5] = [
    (3.0, 0.0),
    (233.0, 147.0),
    (688.0, 457.0),
    (1348.0, 1186.0),
    (2000.0, 1915.0),
];

/// An x of the older reference as points from the left of the window, with the columns where
/// `columns` puts them.
fn map_x(x: f32, columns: &[(f32, f32); 5]) -> f32 {
    let r = crate::chrome::R;
    if x <= columns[0].0 {
        return (columns[0].1 + (x - columns[0].0) * 0.64) * r;
    }
    for pair in columns.windows(2) {
        let ((a0, b0), (a1, b1)) = (pair[0], pair[1]);
        if x <= a1 {
            return (b0 + (x - a0) / (a1 - a0) * (b1 - b0)) * r;
        }
    }
    let (a, b) = columns[columns.len() - 1];
    (b + (x - a) * 1.12) * r
}

const BG: Color32 = Color32::from_rgb(40, 40, 45);
const TOOLBAR2: Color32 = Color32::from_rgb(30, 30, 34);
const LINE: Color32 = Color32::from_rgb(18, 18, 21);
const TEXT: Color32 = Color32::from_rgb(214, 214, 218);
const GREY: Color32 = Color32::from_rgb(150, 150, 154);
const ICON: Color32 = Color32::from_rgb(118, 118, 122);
const DIM: Color32 = Color32::from_rgb(70, 70, 76);
const RED: Color32 = Color32::from_rgb(222, 64, 52);

/// Reference rows: the end of the second toolbar, the parts that stretch, the page bar.
const TOOLBAR2_END: f32 = 140.0;
const VIEW_TOP: f32 = 186.0;
const VIEW_BOTTOM: f32 = 497.0;
const TRACKS_TOP: f32 = 735.0;
const PAGE_BAR: f32 = 1080.0;

/// A drawing surface that takes reference pixels.
struct Ed<'a> {
    p: &'a Painter,
    s: &'a Screen,
    /// Heights in points of the viewers and of the tracks.
    viewers: f32,
    tracks: f32,
    /// `COLUMNS` with the dividers where the user dragged them.
    columns: [(f32, f32); 5],
}

impl<'a> Ed<'a> {
    fn new(p: &'a Painter, s: &'a Screen, panels: &Panels) -> Self {
        let top = 92.0 + (TOOLBAR2_END - 100.0) * KY;
        let fixed = ((VIEW_TOP - TOOLBAR2_END) + (TRACKS_TOP - VIEW_BOTTOM)) * KY;
        let flex = (s.h - 46.8 - top - fixed).max(160.0);
        let viewers = flex * f32::from(panels.edit_viewers) / 1000.0;
        let mut columns = COLUMNS;
        for (column, x) in columns[1..4].iter_mut().zip(panels.edit_columns) {
            column.1 = f32::from(x);
        }
        Ed {
            p,
            s,
            viewers,
            tracks: flex - viewers,
            columns,
        }
    }
    fn y(&self, r: f32) -> f32 {
        let top = 92.0 + (TOOLBAR2_END - 100.0) * KY;
        let view_top = top + (VIEW_TOP - TOOLBAR2_END) * KY;
        let tracks_top = view_top + self.viewers + (TRACKS_TOP - VIEW_BOTTOM) * KY;
        if r <= TOOLBAR2_END {
            92.0 + (r - 100.0) * KY
        } else if r <= VIEW_TOP {
            top + (r - TOOLBAR2_END) * KY
        } else if r <= VIEW_BOTTOM {
            view_top + (r - VIEW_TOP) / (VIEW_BOTTOM - VIEW_TOP) * self.viewers
        } else if r <= TRACKS_TOP {
            view_top + self.viewers + (r - VIEW_BOTTOM) * KY
        } else {
            tracks_top + (r - TRACKS_TOP) / (PAGE_BAR - TRACKS_TOP) * self.tracks
        }
    }
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.s.p(map_x(x, &self.columns), self.y(y))
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.at(x0, y0), self.at(x1, y1))
    }
    fn band(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.p.rect_filled(self.rect(x0, y0, x1, y1), 0.0, c);
    }
    /// Text at a fixed size in points.
    fn label(&self, x: f32, y: f32, align: Align2, text: &str, size: f32, c: Color32, bold: bool) {
        let size = size * SZ;
        let font = if bold {
            theme::semibold(size)
        } else {
            theme::regular(size)
        };
        crate::i18n::paint_text(&self.p,self.at(x, y), align, text, font, c);
    }
    /// Text sized to fill `width` reference pixels.
    /// Text `width` reference pixels wide, smaller when a divider made its column narrower than
    /// usual, so it stays in its panel.
    fn fitted(&self, x: f32, y: f32, text: &str, width: f32, c: Color32) {
        let (x0, x1) = (x - width / 2.0, x + width / 2.0);
        let now = map_x(x1, &self.columns) - map_x(x0, &self.columns);
        let usual = map_x(x1, &COLUMNS) - map_x(x0, &COLUMNS);
        let shrink = (now / usual.max(1.0)).min(1.0);
        crate::i18n::paint_text(&self.p,
            self.at(x, y),
            Align2::CENTER_CENTER,
            text,
            fit(self.p, text, width * KY * shrink, false),
            c,
        );
    }
    fn line(&self, a: (f32, f32), b: (f32, f32), w: f32, c: Color32) {
        self.p
            .line_segment([self.at(a.0, a.1), self.at(b.0, b.1)], Stroke::new(w, c));
    }
    fn vline(&self, x: f32, y0: f32, y1: f32) {
        self.line((x, y0), (x, y1), 1.0, LINE);
    }
    fn icon(&self, x: f32, y: f32, icon: Icon, c: Color32, size: f32) {
        crate::icons::paint_scaled(self.p, self.at(x, y), icon, c, size * SZ);
    }
    fn chevron(&self, x: f32, y: f32, c: Color32) {
        let c0 = self.at(x, y);
        let u = SZ;
        self.p.add(Shape::line(
            vec![
                c0 + vec2(-6.0 * u, -3.5 * u),
                c0 + vec2(0.0, 3.0 * u),
                c0 + vec2(6.0 * u, -3.5 * u),
            ],
            Stroke::new(1.3, c),
        ));
    }
    fn dots(&self, x: f32, y: f32, c: Color32) {
        let c0 = self.at(x, y);
        for dx in [-10.0, 0.0, 10.0] {
            self.p.circle_filled(c0 + vec2(dx * SZ, 0.0), 2.8 * SZ, c);
        }
    }
    /// A red playhead flag over a scrub line.
    fn flag(&self, x: f32, y: f32) {
        let c = self.at(x, y);
        self.p.add(Shape::convex_polygon(
            vec![
                c + vec2(-5.0, -6.0),
                c + vec2(5.0, -6.0),
                c + vec2(5.0, 2.0),
                c + vec2(0.0, 7.0),
                c + vec2(-5.0, 2.0),
            ],
            RED,
            Stroke::NONE,
        ));
        self.p.line_segment(
            [c + vec2(0.0, 6.0), c + vec2(0.0, 14.0)],
            Stroke::new(1.5, RED),
        );
    }
}

/// A click area over an icon; returns whether it was clicked.
fn hit(ui: &mut Ui, e: &Ed, id: &str, x: f32, y: f32, half: f32, hint: &str) -> bool {
    ui.interact(
        e.rect(x - half * 1.5, y - 18.0, x + half * 1.5, y + 18.0),
        ui.id().with((id, x as i32, y as i32)),
        Sense::click(),
    )
    .on_hover_text(hint)
    .clicked()
}

impl StudioApp {
    /// The first toolbar of the Edit page.
    pub(crate) fn edit_toolbar1(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen, y: f32) {
        // (x in reference pixels, icon, hint, size)
        let tools: [(f32, Icon, &str, f32); 11] = [
            (45.0, Icon::MonitorUp, "Source viewer", 0.9),
            (130.0, Icon::PhotoMusic, "Media Pool", 1.0),
            (211.0, Icon::Wand, "Effects", 0.95),
            (289.0, Icon::ListBullets, "Edit Index", 0.9),
            (370.0, Icon::MusicLines, "Sound Library", 0.95),
            (449.0, Icon::Diamonds, "Keyframes", 0.95),
            (1641.0, Icon::Export, "Quick Export", 0.9),
            (1721.0, Icon::Mixer, "Mixer", 0.95),
            (1801.0, Icon::LabelTag, "Metadata", 0.95),
            (1880.0, Icon::Tools, "Inspector", 0.82),
            (1960.0, Icon::ChevronBox, "Toolbar", 0.85),
        ];
        let mut planned: Option<&str> = None;
        for (x, icon, hint, size) in tools {
            let color = match icon {
                Icon::Tools if self.inspector_open => theme::WHITE,
                Icon::PhotoMusic => Color32::from_rgb(200, 200, 204),
                Icon::Diamonds | Icon::ChevronBox => Color32::from_rgb(70, 70, 74),
                _ => theme::ICON,
            };
            let tool = Tool::new(hint).size(size).color(color);
            if self.tool(ui, s.p(x * K, y), icon, tool).clicked() {
                match icon {
                    Icon::PhotoMusic => self.import_dialog(ctx),
                    // The clip's effects are in the Inspector (its Effects part): this is where they
                    // are added and ordered, so the button opens it rather than leaving for the
                    // Fusion page.
                    Icon::Wand => self.inspector_open = true,
                    Icon::Export => self.page = crate::app::Page::Deliver,
                    Icon::Mixer => self.page = crate::app::Page::Audio,
                    Icon::Tools => self.inspector_open = !self.inspector_open,
                    Icon::Diamonds => self.inspector_open = true,
                    _ => planned = Some(hint),
                }
            }
        }
        if let Some(what) = planned {
            self.planned(what);
        }
    }

    pub(crate) fn edit_page(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        let painter = ui.painter().clone();
        let e = Ed::new(&painter, s, &self.extra.prefs.panels);
        e.band(3.0, 100.0, 2000.0, PAGE_BAR, BG);

        self.edit_toolbar2(ui, &e);
        self.edit_bins(ui, &e);
        self.edit_pool(ui, ctx, &e);
        self.edit_source_viewer(ui, ctx, &e);
        if self.inspector_open {
            e.band(1349.0, TOOLBAR2_END + 1.0, 2000.0, 590.0, BG);
            ui.scope_builder(
                UiBuilder::new().max_rect(e.rect(1362.0, 150.0, 1990.0, 586.0)),
                |ui| {
                    // Its own id: the media pool's list is drawn from a sibling scope of this page and
                    // got the same one, so dragging this scroll bar moved the pool's list (and the wheel
                    // did not reach this one).
                    egui::ScrollArea::vertical()
                        .id_salt("edit_inspector_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.inspector(ui, ctx));
                },
            );
        } else {
            self.edit_timeline_viewer(ui, ctx, &e);
        }
        e.vline(1348.0, TOOLBAR2_END, 590.0);
        e.vline(1349.0, TOOLBAR2_END, 590.0);
        self.edit_tool_row(ui, ctx, &e);

        // The timeline: the ruler, the tracks and the scroll bar.
        e.line((689.0, 672.0), (2000.0, 672.0), 1.0, LINE);
        ui.scope_builder(
            UiBuilder::new().max_rect(e.rect(690.0, 672.0, 2000.0, PAGE_BAR)),
            |ui| self.timeline_ui(ui, ctx, Mode::Edit),
        );
        e.vline(688.0, TOOLBAR2_END, PAGE_BAR);
        e.vline(689.0, TOOLBAR2_END, PAGE_BAR);
        self.edit_dividers(ui, &e);
    }

    /// The dividers between the bins, the media pool, the source viewer and the timeline viewer,
    /// and the one between the viewers and the timeline. They move on the next frame.
    fn edit_dividers(&mut self, ui: &mut Ui, e: &Ed) {
        let panels = &mut self.extra.prefs.panels;
        let mut save = false;
        let columns = [
            ("edit bins", e.rect(233.0, TOOLBAR2_END, 234.0, PAGE_BAR)),
            ("edit pool", e.rect(688.0, TOOLBAR2_END, 690.0, PAGE_BAR)),
            ("edit viewers", e.rect(1348.0, TOOLBAR2_END, 1350.0, 590.0)),
        ];
        for (i, (id, line)) in columns.into_iter().enumerate() {
            match divider(ui, id, line, false) {
                Divider::Moved(at) => {
                    panels.set_edit_column(i, (at.x - e.s.o.x) / crate::chrome::R)
                }
                Divider::Reset => {
                    panels.set_edit_column(i, f32::from(Panels::default().edit_columns[i]));
                    save = true;
                }
                Divider::Released => save = true,
                Divider::Still => {}
            }
        }
        match divider(
            ui,
            "edit timeline",
            e.rect(689.0, 589.0, 2000.0, 591.0),
            true,
        ) {
            Divider::Moved(at) => {
                // The divider sits a fixed height below the viewers.
                let viewers = at.y - (e.y(590.0) - e.viewers);
                panels.edit_viewers = Panels::share(viewers / (e.viewers + e.tracks));
            }
            Divider::Reset => {
                panels.edit_viewers = Panels::default().edit_viewers;
                save = true;
            }
            Divider::Released => save = true,
            Divider::Still => {}
        }
        if save {
            self.extra.prefs.save();
        }
    }

    fn edit_toolbar2(&mut self, ui: &mut Ui, e: &Ed) {
        e.band(3.0, 100.0, 2000.0, TOOLBAR2_END, TOOLBAR2);
        e.line((3.0, TOOLBAR2_END), (2000.0, TOOLBAR2_END), 1.5, LINE);
        let y = 120.0;
        e.icon(26.0, y, Icon::Sidebar, ICON, 0.9);
        e.chevron(62.0, y, GREY);
        e.chevron(106.0, y, DIM);
        e.chevron(140.0, y, DIM);
        e.icon(191.0, y, Icon::Unlink, ICON, 0.85);
        e.icon(248.0, y, Icon::ImportImage, ICON, 0.85);
        e.chevron(285.0, y, GREY);
        e.icon(336.0, y, Icon::Cloud, ICON, 0.9);
        e.label(397.0, y, Align2::CENTER_CENTER, "M...", 17.5, TEXT, true);
        e.icon(461.0, y, Icon::ThumbGrid, theme::WHITE, 0.95);
        e.chevron(492.0, y, GREY);
        e.icon(543.0, y, Icon::Search, GREY, 0.85);
        e.icon(601.0, y, Icon::Sort, GREY, 0.9);
        e.dots(659.0, y, GREY);
        for x in [85.0, 165.0, 215.0, 305.0, 360.0, 430.0, 515.0, 570.0, 628.0] {
            e.vline(x, 100.0, TOOLBAR2_END);
        }
        // The viewers' headers.
        e.icon(717.0, y, Icon::Photo, ICON, 0.85);
        e.icon(761.0, y, Icon::FilmView, ICON, 0.85);
        e.icon(806.0, y, Icon::StripView, ICON, 0.85);
        e.vline(832.0, 100.0, TOOLBAR2_END);
        let (source_zoom, timeline_zoom) = (self.edit_zoom(e, false), self.edit_zoom(e, true));
        e.label(
            878.0,
            y,
            Align2::CENTER_CENTER,
            &format!("{source_zoom}%"),
            17.5,
            TEXT,
            true,
        );
        e.chevron(921.0, y, GREY);
        e.vline(945.0, 100.0, TOOLBAR2_END);
        e.p.circle_stroke(e.at(961.0, y - 2.0), 2.5, Stroke::new(1.0, DIM));
        e.dots(1320.0, y, GREY);
        e.vline(1294.0, 100.0, TOOLBAR2_END);
        e.vline(1348.0, 100.0, TOOLBAR2_END);
        e.label(
            1388.0,
            y,
            Align2::CENTER_CENTER,
            &format!("{timeline_zoom}%"),
            17.5,
            TEXT,
            true,
        );
        e.chevron(1432.0, y, GREY);
        e.vline(1455.0, 100.0, TOOLBAR2_END);
        e.p.circle_stroke(e.at(1472.0, y - 2.0), 2.5, Stroke::new(1.0, DIM));
        e.icon(1635.0, y, Icon::ViewerFrame, ICON, 0.85);
        e.chevron(1670.0, y, GREY);
        e.icon(1715.0, y, Icon::Hq, ICON, 0.85);
        e.chevron(1749.0, y, GREY);
        e.icon(1795.0, y, Icon::SparkleFrame, ICON, 0.85);
        e.chevron(1828.0, y, GREY);
        e.icon(1878.0, y, Icon::ColorWipe, theme::WHITE, 0.9);
        e.icon(1930.0, y, Icon::ViewerFrame, theme::WHITE, 0.85);
        e.dots(1980.0, y, GREY);
        for x in [1685.0, 1770.0, 1850.0, 1905.0, 1955.0] {
            e.vline(x, 100.0, TOOLBAR2_END);
        }

        if hit(ui, e, "edit_tb2", 248.0, y, 20.0, "Import media") {
            self.import_dialog(&self.ctx.clone());
        }
        let planned: [(f32, &str); 16] = [
            (26.0, "Bins sidebar"),
            (106.0, "Previous bin"),
            (140.0, "Next bin"),
            (191.0, "Unlinked media"),
            (336.0, "Cloud media"),
            (397.0, "Media pool view"),
            (461.0, "Thumbnail view"),
            (543.0, "Search media"),
            (601.0, "Sort media"),
            (659.0, "Media pool options"),
            (717.0, "Source viewer mode"),
            (1320.0, "Source viewer options"),
            (1635.0, "Safe area"),
            (1715.0, "Playback quality"),
            (1795.0, "Transform overlay"),
            (1878.0, "Split screen wipe"),
        ];
        let ctx = self.ctx.clone();
        for (x, name) in planned {
            if hit(ui, e, "edit_tb2", x, y, 20.0, name) {
                match name {
                    "Safe area" => self.do_act(&ctx, Act::OverlaySafeArea),
                    "Playback quality" => self.set_proxy(if self.proxy > 1 { 1 } else { 2 }),
                    "Media pool view" | "Search media" | "Sort media" | "Thumbnail view" => {
                        self.pool_tool(name);
                    }
                    _ => self.planned(name),
                }
            }
        }
        if hit(ui, e, "edit_tb2", 1980.0, y, 20.0, "Viewer options") {
            self.do_act(&ctx, Act::OverlayAll);
        }
    }

    /// The zoom a viewer shows the picture at, in percent of its full size.
    fn edit_zoom(&self, e: &Ed, timeline: bool) -> u32 {
        let area = if timeline {
            e.rect(1350.0, VIEW_TOP, 2000.0, VIEW_BOTTOM)
        } else {
            e.rect(690.0, VIEW_TOP, 1348.0, VIEW_BOTTOM)
        };
        let (w, h) = (self.project().width as f32, self.project().height as f32);
        if w < 1.0 || h < 1.0 {
            return 100;
        }
        let scale = (area.width() / w).min(area.height() / h);
        (scale * 100.0).round().clamp(1.0, 999.0) as u32
    }

    /// The bins on the left: Master, then the smart bins.
    fn edit_bins(&mut self, ui: &mut Ui, e: &Ed) {
        e.band(3.0, TOOLBAR2_END + 1.0, 233.0, PAGE_BAR, BG);
        e.icon(26.0, 163.0, Icon::BinFolder, GREY, 0.85);
        e.label(50.0, 163.0, Align2::LEFT_CENTER, "Bins", 18.5, TEXT, false);
        e.label(211.0, 163.0, Align2::CENTER_CENTER, "+", 22.0, GREY, false);
        use crate::pool::BinView;
        let chosen = Color32::from_rgb(33, 33, 37);
        if self.pool.bin == BinView::Master {
            e.band(3.0, 186.0, 233.0, 222.0, chosen);
        }
        e.label(
            41.0,
            204.0,
            Align2::LEFT_CENTER,
            "Master",
            18.5,
            TEXT,
            false,
        );
        if hit(
            ui,
            e,
            "edit_bin_master",
            118.0,
            204.0,
            112.0,
            "The Master bin: all media",
        ) {
            self.pool.bin = BinView::Master;
        }
        // The bins, under Master.
        let bins: Vec<(u64, String)> = self
            .project()
            .bins
            .iter()
            .map(|b| (b.id, b.name.clone()))
            .collect();
        for (n, (id, name)) in bins.iter().enumerate().take(17) {
            let y = 240.0 + n as f32 * 36.0;
            if self.pool.bin == BinView::Bin(*id) {
                e.band(3.0, y - 18.0, 233.0, y + 18.0, chosen);
            }
            e.icon(54.0, y, Icon::BinFolder, GREY, 0.7);
            e.label(74.0, y, Align2::LEFT_CENTER, name, 17.0, TEXT, false);
            let r = ui.interact(
                e.rect(3.0, y - 18.0, 233.0, y + 18.0),
                ui.id().with(("edit_bin", *id)),
                Sense::click(),
            );
            if r.clicked() {
                self.pool.bin = BinView::Bin(*id);
            }
            r.context_menu(|ui| {
                if ui.button("Rename Bin").clicked() {
                    self.extra.rename_bin = Some((*id, name.clone()));
                    ui.close();
                }
                if ui.button("Delete Bin").clicked() {
                    if self.run(Command::DeleteBin { id: *id }).is_some() {
                        self.pool.bin = BinView::Master;
                    }
                    ui.close();
                }
            });
        }
        e.line((3.0, 883.0), (233.0, 883.0), 1.0, LINE);
        e.icon(26.0, 907.0, Icon::SmartBin, GREY, 0.85);
        e.label(
            50.0,
            907.0,
            Align2::LEFT_CENTER,
            "Smart Bins",
            18.5,
            TEXT,
            false,
        );
        e.label(211.0, 907.0, Align2::CENTER_CENTER, "+", 22.0, GREY, false);
        // The smart bins (Keywords and Collections stand in until there is one).
        let smart: Vec<(u64, String)> = self
            .project()
            .smart_bins
            .iter()
            .map(|b| (b.id, b.name.clone()))
            .collect();
        if smart.is_empty() {
            e.label(
                49.0,
                948.0,
                Align2::LEFT_CENTER,
                "Keywords",
                18.5,
                Color32::from_rgb(150, 150, 156),
                false,
            );
            e.label(
                20.0,
                982.0,
                Align2::CENTER_CENTER,
                ">",
                16.0,
                Color32::from_rgb(150, 150, 156),
                false,
            );
            e.label(
                49.0,
                982.0,
                Align2::LEFT_CENTER,
                "Collections",
                18.5,
                Color32::from_rgb(150, 150, 156),
                false,
            );
        }
        for (n, (id, name)) in smart.iter().enumerate().take(3) {
            let y = 948.0 + n as f32 * 34.0;
            if self.pool.bin == BinView::Smart(*id) {
                e.band(3.0, y - 17.0, 233.0, y + 17.0, chosen);
            }
            e.label(49.0, y, Align2::LEFT_CENTER, name, 17.0, TEXT, false);
            let r = ui.interact(
                e.rect(3.0, y - 17.0, 233.0, y + 17.0),
                ui.id().with(("edit_smart", *id)),
                Sense::click(),
            );
            if r.clicked() {
                self.pool.bin = BinView::Smart(*id);
            }
            r.context_menu(|ui| {
                if ui.button("Delete Smart Bin").clicked() {
                    if self.run(Command::DeleteSmartBin { id: *id }).is_some() {
                        self.pool.bin = BinView::Master;
                    }
                    ui.close();
                }
            });
        }
        e.vline(233.0, TOOLBAR2_END, PAGE_BAR);
        e.vline(234.0, TOOLBAR2_END, PAGE_BAR);
        if hit(ui, e, "edit_bins", 211.0, 163.0, 16.0, "New bin") {
            self.new_bin();
        }
        if hit(ui, e, "edit_bins", 211.0, 907.0, 16.0, "New smart bin") {
            self.extra.smart_bin = Some((String::new(), String::new(), 0));
        }
        if smart.is_empty() {
            for y in [948.0, 982.0] {
                if hit(ui, e, "edit_bins", 100.0, y, 90.0, "Smart bins") {
                    self.extra.smart_bin = Some((String::new(), String::new(), 0));
                }
            }
        }
    }

    /// The media pool: the clips of the Master bin, or a note when there are none.
    fn edit_pool(&mut self, ui: &mut Ui, ctx: &Context, e: &Ed) {
        e.band(235.0, TOOLBAR2_END + 1.0, 688.0, PAGE_BAR, BG);
        e.label(
            247.0,
            163.0,
            Align2::LEFT_CENTER,
            &self.pool_bin_title(),
            16.0,
            TEXT,
            false,
        );
        e.line((235.0, 186.0), (688.0, 186.0), 1.0, LINE);
        if self.project().media.is_empty() {
            e.fitted(
                452.0,
                612.0,
                "No clips in media pool",
                418.0,
                Color32::from_rgb(80, 80, 86),
            );
            e.fitted(
                460.0,
                690.0,
                "Add clips from Media Storage to get started",
                348.0,
                Color32::from_rgb(64, 64, 70),
            );
        } else {
            ui.scope_builder(
                UiBuilder::new().max_rect(e.rect(243.0, 196.0, 682.0, PAGE_BAR - 6.0)),
                |ui| self.pool_grid(ui, ctx),
            );
        }
    }

    /// The source viewer: the media chosen in the pool, its scrub line and its transport.
    fn edit_source_viewer(&mut self, ui: &mut Ui, ctx: &Context, e: &Ed) {
        self.tick_source(ctx);
        let fps = self.project().fps;
        let (length, at) = if self.source.media.is_some() {
            (self.source_length(), self.source.frame)
        } else {
            (0, 0)
        };
        let length_tc = fps.timecode(length);
        e.label(
            701.0,
            162.0,
            Align2::LEFT_CENTER,
            &length_tc,
            18.5,
            TEXT,
            true,
        );
        e.chevron(1047.0, 162.0, GREY);
        if let Some(name) = self
            .source
            .media
            .and_then(|m| self.project().media(m))
            .map(|m| m.name.clone())
        {
            // The name ends at its usual place and is cut, with "…", where it would reach the
            // length on its left.
            let tc_end = e.at(701.0, 162.0).x
                + e.p
                    .layout_no_wrap(length_tc, theme::semibold(18.5 * SZ), TEXT)
                    .size()
                    .x;
            let room = e.at(1000.0, 162.0).x - tc_end - 14.0 * SZ;
            let shown = crate::media::elided(e.p, &name, theme::regular(16.0 * SZ), room);
            e.label(
                1000.0,
                162.0,
                Align2::RIGHT_CENTER,
                &shown,
                16.0,
                GREY,
                false,
            );
        }
        e.label(
            1328.0,
            162.0,
            Align2::RIGHT_CENTER,
            &fps.timecode(at),
            18.5,
            TEXT,
            true,
        );
        // The picture: the selected media's thumbnail in a black frame.
        let area = e.rect(690.0, VIEW_TOP, 1348.0, VIEW_BOTTOM);
        let aspect = self.project().width as f32 / self.project().height.max(1) as f32;
        let fit_size = if area.width() / area.height() > aspect {
            vec2(area.height() * aspect, area.height())
        } else {
            vec2(area.width(), area.width() / aspect)
        };
        let frame = Rect::from_center_size(area.center(), fit_size);
        e.p.rect_filled(frame, 0.0, Color32::BLACK);
        let picture = self
            .source_texture(ctx)
            .or_else(|| self.selected_media.and_then(|id| self.media_thumb(ctx, id)));
        if let Some(texture) = picture {
            e.p.image(
                texture,
                frame,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        // The scrub line and the transport.
        e.band(690.0, VIEW_BOTTOM, 1348.0, 590.0, BG);
        e.line(
            (720.0, 517.0),
            (1315.0, 517.0),
            3.0,
            Color32::from_rgb(22, 22, 25),
        );
        // The scrub line: the flag at the source frame; a click or drag moves it.
        let along = if length > 1 {
            at as f32 / (length - 1) as f32
        } else {
            0.0
        };
        e.flag(720.0 + along * 595.0, 508.0);
        let scrub = ui.interact(
            e.rect(712.0, 500.0, 1323.0, 530.0),
            ui.id().with("source_scrub"),
            Sense::click_and_drag(),
        );
        if scrub.clicked() || scrub.dragged() {
            if let Some(p) = scrub.interact_pointer_pos() {
                let line = e.rect(720.0, 510.0, 1315.0, 520.0);
                let f = ((p.x - line.left()) / line.width().max(1.0)).clamp(0.0, 1.0);
                self.source_go_to((f * length.saturating_sub(1) as f32).round() as u64);
            }
        }
        let y = 566.0;
        e.icon(734.0, y, Icon::FilmView, ICON, 0.85);
        e.chevron(771.0, y, GREY);
        e.icon(818.0, y, Icon::InsertClip, ICON, 0.85);
        e.chevron(856.0, y, GREY);
        for (x, icon) in [
            (893.0, Icon::ToStart),
            (945.0, Icon::PlayReverse),
            (994.0, Icon::Stop),
            (1042.0, Icon::Play),
            (1095.0, Icon::ToEnd),
            (1155.0, Icon::Loop),
        ] {
            let lit = (icon == Icon::Play && self.source.playing)
                || (icon == Icon::Loop && self.source.looping);
            e.icon(x, y, icon, if lit { theme::WHITE } else { GREY }, 0.9);
            if hit(
                ui,
                e,
                "edit_source_transport",
                x,
                y,
                22.0,
                "Source playback",
            ) {
                match icon {
                    Icon::ToStart => self.source_go_to(0),
                    Icon::PlayReverse => self.source_go_to(self.source.frame.saturating_sub(1)),
                    Icon::Stop => self.source.playing = false,
                    Icon::Play => self.source_toggle_play(),
                    Icon::ToEnd => self.source_go_to(u64::MAX),
                    _ => self.source.looping = !self.source.looping,
                }
            }
        }
        e.icon(1290.0, y, Icon::FilmPlay, ICON, 0.85);
        if hit(
            ui,
            e,
            "edit_source",
            818.0,
            y,
            24.0,
            "Insert the selected media at the playhead",
        ) {
            self.insert_edit();
        }
        if hit(
            ui,
            e,
            "edit_source",
            1290.0,
            y,
            24.0,
            "Match frame: find this frame on the timeline",
        ) {
            self.match_frame_to_timeline();
        }
        if hit(ui, e, "edit_source", 734.0, y, 24.0, "Source clip view") {
            self.page = crate::app::Page::Media;
        }
    }

    /// The timeline viewer: the picture at the playhead, a scrub line over the whole timeline
    /// and the transport.
    fn edit_timeline_viewer(&mut self, ui: &mut Ui, ctx: &Context, e: &Ed) {
        let fps = self.project().fps;
        let duration = self.project().duration();
        e.label(
            1362.0,
            162.0,
            Align2::LEFT_CENTER,
            &fps.timecode(duration),
            18.5,
            TEXT,
            true,
        );
        let several = !self.project().timelines.is_empty();
        if duration > 0 || several {
            // The timeline being edited, in red, as in the reference.
            let name = self.project().timeline_name.clone();
            e.label(
                1672.0,
                162.0,
                Align2::CENTER_CENTER,
                &name,
                18.5,
                Color32::from_rgb(232, 76, 62),
                false,
            );
            e.chevron(1740.0, 162.0, GREY);
        } else {
            e.chevron(1708.0, 162.0, GREY);
        }
        // A click on the name lists the project's timelines.
        let picker = ui
            .interact(
                e.rect(1560.0, 140.0, 1760.0, 184.0),
                ui.id().with("edit_timeline_picker"),
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
        e.label(
            1988.0,
            162.0,
            Align2::RIGHT_CENTER,
            &self.tc(self.playhead as i64),
            18.5,
            TEXT,
            true,
        );
        let area = e.rect(1350.0, VIEW_TOP, 2000.0, VIEW_BOTTOM);
        let aspect = self.project().width as f32 / self.project().height.max(1) as f32;
        self.viewer
            .paint(ui, area, aspect, BG, duration > 0, self.overlays);

        e.band(1350.0, VIEW_BOTTOM, 2000.0, 590.0, BG);
        let bar = e.rect(1381.0, 515.0, 1975.0, 519.0);
        e.p.rect_filled(bar, 1.0, Color32::from_rgb(22, 22, 25));
        let frac = (self.playhead as f32 / duration.max(1) as f32).clamp(0.0, 1.0);
        let x = 1381.0 + (1975.0 - 1381.0) * frac;
        e.flag(x, 508.0);
        for hx in [1381.0, 1975.0] {
            e.p.rect_filled(
                Rect::from_center_size(e.at(hx, 532.0), vec2(6.0, 8.0)),
                1.0,
                Color32::from_rgb(150, 150, 154),
            );
        }
        let scrub = ui.interact(
            bar.expand2(vec2(6.0, 10.0)),
            ui.id().with("edit_scrub"),
            Sense::click_and_drag(),
        );
        if scrub.clicked() || scrub.dragged() {
            if let Some(p) = scrub.interact_pointer_pos() {
                let f = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                self.playing = false;
                self.go_to((f * duration as f32) as u64);
            }
        }
        let y = 566.0;
        e.icon(1395.0, y, Icon::ViewerFrame, ICON, 0.85);
        e.chevron(1435.0, y, GREY);
        e.icon(1488.0, y, Icon::Grid2, ICON, 0.85);
        let buttons: [(f32, Icon, Act); 6] = [
            (1540.0, Icon::ToStart, Act::GoFirst),
            (1592.0, Icon::PlayReverse, Act::PreviousFrame),
            (1641.0, Icon::Stop, Act::Stop),
            (1689.0, Icon::Play, Act::TogglePlay),
            (1742.0, Icon::ToEnd, Act::GoLast),
            (1802.0, Icon::Loop, Act::ToggleLoop),
        ];
        for (x, icon, act) in buttons {
            let lit = (icon == Icon::Play && self.playing) || (icon == Icon::Loop && self.looping);
            e.icon(x, y, icon, if lit { theme::WHITE } else { GREY }, 0.9);
            if hit(ui, e, "edit_transport", x, y, 22.0, "") {
                self.do_act(ctx, act);
            }
        }
        e.icon(1950.0, y, Icon::FilmPlay, ICON, 0.85);
        if hit(
            ui,
            e,
            "edit_tv",
            1950.0,
            y,
            24.0,
            "Match frame: show this frame's source (F)",
        ) {
            self.match_frame_to_source();
        }
        for (x, name) in [(1395.0, "Viewer framing"), (1488.0, "Multicam")] {
            if hit(ui, e, "edit_tv", x, y, 24.0, name) {
                if name == "Viewer framing" {
                    self.do_act(ctx, Act::OverlaySafeArea);
                } else {
                    self.do_act(ctx, Act::MulticamViewer);
                }
            }
        }
    }

    /// The timeline's tool row: edit modes, the three ways to put media on the timeline,
    /// snapping, flags and markers, and the zoom.
    fn edit_tool_row(&mut self, ui: &mut Ui, ctx: &Context, e: &Ed) {
        e.band(690.0, 590.0, 2000.0, 672.0, Color32::from_rgb(43, 43, 48));
        e.line((689.0, 590.0), (2000.0, 590.0), 1.5, LINE);
        let y = 631.0;
        let snap = self.timeline.snap();
        let icons: [(f32, Icon, Color32, f32); 15] = [
            (735.0, Icon::RailChecks, GREY, 0.85),
            (798.0, Icon::Diamonds, GREY, 0.9),
            (857.0, Icon::Mic, GREY, 0.85),
            (
                962.0,
                Icon::Pointer,
                if self.extra.trim_mode {
                    GREY
                } else {
                    theme::TOOL_RED
                },
                0.85,
            ),
            (1067.0, Icon::TrimTool, DIM, 0.85),
            (1146.0, Icon::InsertClip, GREY, 0.85),
            (1208.0, Icon::Overwrite, GREY, 0.85),
            (1270.0, Icon::Replace, GREY, 0.85),
            (
                1352.0,
                Icon::SnapHook,
                if snap { theme::WHITE } else { DIM },
                0.85,
            ),
            (1413.0, Icon::LinkChain, GREY, 0.85),
            (1476.0, Icon::PositionLock, GREY, 0.85),
            (1546.0, Icon::Bookmark, theme::ACCENT, 0.75),
            (1612.0, Icon::Marker, theme::ACCENT, 0.7),
            (1712.0, Icon::ZoomRuler, GREY, 0.85),
            (1975.0, Icon::Sidebar, DIM, 0.85),
        ];
        for (x, icon, color, size) in icons {
            e.icon(x, y, icon, color, size);
        }
        for x in [1001.0, 1575.0, 1642.0, 1746.0] {
            e.chevron(x, y, GREY);
        }
        for x in [898.0, 1027.0, 1105.0, 1312.0, 1513.0, 1668.0] {
            e.line((x, 605.0), (x, 657.0), 1.0, Color32::from_rgb(58, 58, 64));
        }
        // The zoom: minus, a slider and plus.
        e.line((1776.0, y), (1790.0, y), 1.5, GREY);
        e.line((1903.0, y), (1917.0, y), 1.5, GREY);
        e.line((1910.0, y - 7.0), (1910.0, y + 7.0), 1.5, GREY);
        e.line((1810.0, y), (1890.0, y), 1.5, DIM);
        let level = self.timeline.zoom_level();
        let knob = 1810.0 + 80.0 * level;
        e.p.circle_filled(e.at(knob, y), 6.0, Color32::from_rgb(200, 200, 204));
        let slider = ui
            .interact(
                e.rect(1802.0, y - 14.0, 1898.0, y + 14.0),
                ui.id().with("edit_zoom_slider"),
                Sense::click_and_drag(),
            )
            .on_hover_text("Zoom");
        if slider.clicked() || slider.dragged() {
            if let Some(p) = slider.interact_pointer_pos() {
                let x0 = e.at(1810.0, y).x;
                let x1 = e.at(1890.0, y).x;
                self.timeline.set_zoom_level((p.x - x0) / (x1 - x0));
            }
        }
        if hit(ui, e, "edit_zoom", 1783.0, y, 12.0, "Zoom out") {
            self.timeline.set_zoom_level(level - 0.06);
        }
        if hit(ui, e, "edit_zoom", 1910.0, y, 12.0, "Zoom in") {
            self.timeline.set_zoom_level(level + 0.06);
        }
        if hit(ui, e, "edit_zoom", 1712.0, y, 22.0, "Fit the timeline") {
            let width = e.at(2000.0, 0.0).x - e.at(1097.0, 0.0).x;
            let duration = self.project().duration();
            let fps = self.project().fps.nominal() as f32;
            self.timeline.fit(duration, width, fps);
        }

        if hit(ui, e, "edit_tools", 1146.0, y, 26.0, "Insert clip (F9)") {
            self.insert_edit();
        }
        if hit(ui, e, "edit_tools", 1208.0, y, 26.0, "Overwrite clip (F10)") {
            self.overwrite_edit();
        }
        if hit(ui, e, "edit_tools", 1270.0, y, 26.0, "Replace clip (F11)") {
            self.replace_edit();
        }
        if hit(ui, e, "edit_tools", 1352.0, y, 26.0, "Snapping (N)") {
            self.do_act(ctx, Act::ToggleSnap);
        }
        if hit(ui, e, "edit_tools", 1612.0, y, 26.0, "Add a marker (M)") {
            self.do_act(ctx, Act::AddMarker);
        }
        let planned: [(f32, &str); 9] = [
            (735.0, "Timeline view options"),
            (798.0, "Keyframes"),
            (857.0, "Voice over"),
            (962.0, "Selection mode"),
            (1067.0, "Trim mode"),
            (1413.0, "Linked selection"),
            (1476.0, "Position lock"),
            (1546.0, "Flags"),
            (1975.0, "Timeline panel"),
        ];
        for (x, name) in planned {
            if hit(ui, e, "edit_tools", x, y, 24.0, name) {
                match name {
                    "Linked selection" => self.do_act(ctx, Act::ToggleLinkedSelection),
                    "Position lock" => self.do_act(ctx, Act::PositionLock),
                    "Selection mode" => self.do_act(ctx, Act::SelectionMode),
                    "Keyframes" => self.do_act(ctx, Act::ShowInspector),
                    "Voice over" => self.voice_over(),
                    "Trim mode" => self.do_act(ctx, Act::TrimMode),
                    "Flags" | "Timeline view options" | "Timeline panel" => {
                        self.do_act(ctx, Act::ShowEditIndex)
                    }
                    _ => self.planned(name),
                }
            }
        }
    }

    fn inspector(&mut self, ui: &mut Ui, ctx: &Context) {
        heading(ui, "Inspector");
        let Some(id) = self.selection else {
            ui.label(RichText::new("Select a clip on the timeline.").color(theme::TEXT_DIM));
            return;
        };
        let Some((track, clip)) = self.project().clip(id) else {
            return;
        };
        let fps = self.project().fps;
        let media = self
            .project()
            .media(clip.media)
            .map(|m| m.name.clone())
            .unwrap_or_default();
        let (name, track_name, start, duration, graded) = (
            clip.name.clone(),
            track.name.clone(),
            clip.start,
            clip.duration,
            clip.is_graded(),
        );
        let (mut blend, speed) = (clip.blend, clip.speed);
        let (mut opacity, mut transform) = self.shown_transform(id);

        let (start_tc, end_tc) = (self.tc(start as i64), self.tc((start + duration) as i64));
        ui.label(RichText::new(name).strong().size(15.0));
        ui.label(RichText::new(format!("{track_name}  -  {media}")).color(theme::TEXT_DIM));
        ui.add_space(6.0);
        egui::Grid::new("clip_info").num_columns(2).show(ui, |ui| {
            ui.label("Start");
            ui.monospace(start_tc);
            ui.end_row();
            ui.label("Duration");
            ui.monospace(fps.timecode(duration));
            ui.end_row();
            ui.label("End");
            ui.monospace(end_tc);
            ui.end_row();
        });
        heading(ui, "Composite");
        let changed = ui
            .horizontal(|ui| {
                let r = ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("Opacity"));
                self.key_buttons(ui, id, picasso_core::KeyParam::Opacity);
                r.changed()
            })
            .inner;
        if changed && !self.set_animated(ctx, id, picasso_core::KeyParam::Opacity, opacity) {
            self.run_gesture(ctx, Command::SetOpacity { clip: id, opacity });
        }
        egui::ComboBox::from_label("Blend")
            .selected_text(blend.label())
            .show_ui(ui, |ui| {
                for mode in BlendMode::ALL {
                    ui.selectable_value(&mut blend, mode, mode.label());
                }
            });
        if blend != self.project().clip(id).map_or(blend, |(_, c)| c.blend) {
            self.run(Command::SetBlend { clip: id, blend });
        }
        self.title_controls(ui, ctx, id);
        self.speed_controls(ui, id, speed);
        self.sound_controls(ui, ctx, id);
        self.transform_controls(ui, ctx, id, &mut transform);
        self.effect_controls(ui, ctx, id);
        heading(ui, "Color");
        ui.horizontal(|ui| {
            ui.label(if graded { "Graded" } else { "Not graded" });
            if ui
                .add_enabled(graded, egui::Button::new("Reset grade"))
                .clicked()
            {
                self.run(Command::SetGrade {
                    clip: id,
                    grade: ColorGrade::default(),
                    node: 0,
                });
            }
        });
        if ui.button("Open in Color page").clicked() {
            self.page = crate::app::Page::Color;
        }
    }

    /// A title's words and look: what the text says, its size, colour, place and weight.
    fn title_controls(&mut self, ui: &mut Ui, ctx: &Context, id: picasso_core::ClipId) {
        use picasso_core::MediaSource;
        let Some((media, mut title)) = self
            .project()
            .clip(id)
            .and_then(|(_, c)| {
                self.project()
                    .media(c.media)
                    .map(|m| (m.id, m.source.clone()))
            })
            .and_then(|(m, source)| match source {
                MediaSource::Title(t) => Some((m, t)),
                _ => None,
            })
        else {
            return;
        };
        heading(ui, "Title");
        let mut text = match &self.title_draft {
            Some((m, draft)) if *m == media => draft.clone(),
            _ => title.text.clone(),
        };
        let field = ui.add(
            egui::TextEdit::multiline(&mut text)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if field.changed() {
            self.title_draft = Some((media, text.clone()));
        }
        // The words are kept when the field is left, as one undo step.
        if field.lost_focus() {
            if let Some((m, draft)) = self.title_draft.take() {
                if m == media && draft != title.text {
                    title.text = draft;
                    self.run(Command::SetTitle {
                        media,
                        title: title.clone(),
                    });
                }
            }
        }
        let before = title.clone();
        ui.add(egui::Slider::new(&mut title.size, 10..=500).text("Size"));
        ui.horizontal(|ui| {
            ui.label("Color");
            ui.color_edit_button_srgb(&mut title.color);
            ui.checkbox(&mut title.bold, "Bold");
        });
        ui.add(egui::Slider::new(&mut title.x, 0..=1000).text("Position X"));
        ui.add(egui::Slider::new(&mut title.y, 0..=1000).text("Position Y"));
        text_plus_controls(ui, &mut title.style);
        if title != before {
            self.run_gesture(ctx, Command::SetTitle { media, title });
        }
    }

    /// The clip's effects: each one on or off, its strength, its place in the order; and a
    /// list to add more.
    fn effect_controls(&mut self, ui: &mut Ui, ctx: &Context, id: picasso_core::ClipId) {
        use picasso_core::EffectKind;
        let Some(effects) = self.project().clip(id).map(|(_, c)| c.effects.clone()) else {
            return;
        };
        heading(ui, "Effects");
        let mut command = None;
        for (index, mut e) in effects.iter().copied().enumerate() {
            let before = e;
            ui.horizontal(|ui| {
                ui.checkbox(&mut e.enabled, e.kind.label());
                if ui
                    .add_enabled(index > 0, egui::Button::new("^").small())
                    .on_hover_text("Earlier")
                    .clicked()
                {
                    command = Some(vec![
                        Command::SetEffect {
                            clip: id,
                            index: index - 1,
                            effect: e,
                        },
                        Command::SetEffect {
                            clip: id,
                            index,
                            effect: effects[index - 1],
                        },
                    ]);
                }
                if ui.small_button("x").on_hover_text("Remove").clicked() {
                    command = Some(vec![Command::RemoveEffect { clip: id, index }]);
                }
            });
            ui.add(
                egui::Slider::new(&mut e.amount, 0.0..=100.0)
                    .text("Amount"),
            );

            // Show param sliders for effects that need extra parameters.
            match e.kind {
                EffectKind::RgbMixer => {
                    // The 3x3 matrix: 9 values, each 0..2 where 1.0 = passthrough.
                    // Layout: row-major [R→R, R→G, R→B, G→R, G→G, G→B, B→R, B→G, B→B].
                    ui.label(
                        egui::RichText::new("RGB Mixer Matrix").size(14.0),
                    );
                    ui.label(egui::RichText::new(
                        "Each cell controls how much of the input channel\ncontributes to the output. 1.0 = normal.",
                    ).size(11.0));
                    for (i, label) in [
                        "Red→Red", "Red→Green", "Red→Blue",
                        "Green→Red", "Green→Green", "Green→Blue",
                        "Blue→Red", "Blue→Green", "Blue→Blue",
                    ].iter().enumerate() {
                        ui.label(egui::RichText::new(*label).size(13.0));
                        ui.add(
                            egui::Slider::new(&mut e.params[i], 0.0..=2.0)
                                .text(label.to_string()),
                        );
                    }
                }
                EffectKind::ColorTemperature => {
                    ui.label(egui::RichText::new("Temperature").size(14.0));
                    ui.label(egui::RichText::new(
                        "Negative (cool/blue, 10K), 0 (neutral 6.6K),\npositive (warm/orange, 3.2K).",
                    ).size(11.0));
                    ui.add(
                        egui::Slider::new(&mut e.params[0], -100.0..=100.0)
                            .text("Shift"),
                    );
                }
                EffectKind::DirectionalBlur => {
                    ui.label(egui::RichText::new("Direction").size(14.0));
                    ui.label(egui::RichText::new(
                        "Blur direction in degrees. Amount stretches it.",
                    ).size(11.0));
                    ui.add(
                        egui::Slider::new(&mut e.params[0], 0.0..=360.0)
                            .text("Angle"),
                    );
                }
                EffectKind::Diffusion => {
                    ui.label(egui::RichText::new("Diffusion").size(14.0));
                    ui.label(egui::RichText::new(
                        "Blurred duplicate, screen-blended back\nfor a soft glow look.",
                    ).size(11.0));
                }
                _ => {}
            }
            if e != before && command.is_none() {
                self.run_gesture(
                    ctx,
                    Command::SetEffect {
                        clip: id,
                        index,
                        effect: e,
                    },
                );
            }
        }
        if let Some(commands) = command {
            if let Err(e) = self.session.dispatch_all(commands) {
                self.fail(e);
            }
        }
        let mut add = None;
        egui::ComboBox::from_id_salt(("add_effect", id.0))
            .selected_text("Add Effect...")
            .show_ui(ui, |ui| {
                for kind in EffectKind::ALL {
                    if ui.selectable_label(false, kind.label()).clicked() {
                        add = Some(kind);
                    }
                }
            });
        if let Some(kind) = add {
            self.run(Command::AddEffect { clip: id, kind });
        }
        // Effects presets.
        let effects = self.project().clip(id).map(|(_, c)| c.effects.clone());
        ui.separator();
        ui.label(egui::RichText::new("Presets").size(14.0));
        if ui.button("Save Current...").clicked() {
            self.show_effect_preset_name_dialog = true;
        }
        for name in crate::effect_presets::list_presets() {
            if ui.button(&name).clicked() {
                if let Some(preset) = crate::effect_presets::load_preset(&name) {
                    self.run_gesture(
                        ctx,
                        Command::SetEffects {
                            clip: id,
                            effects: preset.effects.clone(),
                        },
                    );
                    self.info(format!("Applied preset: {}", name));
                }
            }
        }

        // Save dialog popup.
        if self.show_effect_preset_name_dialog {
            egui::Window::new("Save Effects Preset")
                .id(egui::Id::new("save_preset_dialog"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Enter a name for this preset:");
                    ui.add(egui::TextEdit::singleline(&mut self.preset_name));
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            if !self.preset_name.is_empty() {
                                if let Some(effects) = &effects {
                                    if crate::effect_presets::save_preset(
                                        &self.preset_name,
                                        effects,
                                    ) {
                                        self.info(format!(
                                            "Saved preset: {}",
                                            self.preset_name
                                        ));
                                    }
                                }
                                self.show_effect_preset_name_dialog = false;
                                self.preset_name.clear();
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_effect_preset_name_dialog = false;
                            self.preset_name.clear();
                        }
                    });
                });
        }
    }

    /// The clip's gain and fades.
    fn sound_controls(&mut self, ui: &mut Ui, ctx: &Context, id: picasso_core::ClipId) {
        use picasso_core::{ClipAudio, MAX_GAIN_DB, MIN_GAIN_DB};
        let Some((_, clip)) = self.project().clip(id) else {
            return;
        };
        let (mut audio, duration) = (clip.audio, clip.duration);
        heading(ui, "Sound");
        ui.horizontal(|ui| {
            if ui.button("Track EQ...").clicked() {
                self.open_track_fx(crate::track_fx::FxTab::Eq);
            }
            if ui.button("Track Dynamics...").clicked() {
                self.open_track_fx(crate::track_fx::FxTab::Dynamics);
            }
        });
        let mut changed = false;
        let gain = ui.add(
            egui::Slider::new(&mut audio.gain_db, MIN_GAIN_DB..=MAX_GAIN_DB)
                .suffix(" dB")
                .step_by(0.1)
                .text("Gain"),
        );
        if gain.double_clicked() {
            audio.gain_db = 0.0;
            changed = true;
        }
        changed |= gain.changed();
        ui.horizontal(|ui| {
            ui.label("Fade in");
            changed |= ui
                .add(egui::DragValue::new(&mut audio.fade_in).range(0..=duration))
                .changed();
            ui.label("out");
            changed |= ui
                .add(egui::DragValue::new(&mut audio.fade_out).range(0..=duration))
                .changed();
        });
        if changed {
            self.run_gesture(ctx, Command::SetClipAudio { clip: id, audio });
        }
        if ui
            .add_enabled(
                audio != ClipAudio::default(),
                egui::Button::new("Reset sound"),
            )
            .clicked()
        {
            self.run(Command::SetClipAudio {
                clip: id,
                audio: ClipAudio::default(),
            });
        }
    }

    /// The speed of the clip, as a percentage; the clip gets shorter or longer to keep its range.
    fn speed_controls(&mut self, ui: &mut Ui, id: picasso_core::ClipId, speed: f32) {
        heading(ui, "Speed");
        let mut percent = speed * 100.0;
        let response = ui.add(
            egui::DragValue::new(&mut percent)
                .range(MIN_SPEED * 100.0..=MAX_SPEED * 100.0)
                .speed(1.0)
                .suffix(" %"),
        );
        // Applied when the value is let go or typed in, since it changes the clip's length.
        if response.drag_stopped() || (response.lost_focus() && response.changed()) {
            self.run(Command::SetClipSpeed {
                clip: id,
                speed: percent / 100.0,
            });
        }
        ui.horizontal(|ui| {
            for (label, value) in [("50%", 0.5), ("100%", 1.0), ("200%", 2.0)] {
                if ui
                    .add_enabled((speed - value).abs() > 1e-4, egui::Button::new(label))
                    .clicked()
                {
                    self.run(Command::SetClipSpeed {
                        clip: id,
                        speed: value,
                    });
                }
            }
        });
    }

    /// Position, scale, rotation and crop of the clip's picture.
    fn transform_controls(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        id: picasso_core::ClipId,
        t: &mut Transform,
    ) {
        use picasso_core::KeyParam;
        heading(ui, "Transform");
        let before = *t;
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut t.x, -1.0..=1.0).text("Position X"));
            self.key_buttons(ui, id, KeyParam::PositionX);
        });
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut t.y, -1.0..=1.0).text("Position Y"));
            self.key_buttons(ui, id, KeyParam::PositionY);
        });
        // One slider for both, so the picture keeps its shape; the numbers below set them apart.
        let mut scale = (t.scale_x + t.scale_y) * 0.5;
        let zoom = ui
            .horizontal(|ui| {
                let r = ui.add(
                    egui::Slider::new(&mut scale, 0.1..=4.0)
                        .logarithmic(true)
                        .text("Zoom"),
                );
                self.key_buttons(ui, id, KeyParam::Zoom);
                r.changed()
            })
            .inner;
        if zoom {
            t.scale_x = scale;
            t.scale_y = scale;
        }
        ui.horizontal(|ui| {
            ui.label("W");
            ui.add(
                egui::DragValue::new(&mut t.scale_x)
                    .range(0.01..=16.0)
                    .speed(0.01),
            );
            ui.label("H");
            ui.add(
                egui::DragValue::new(&mut t.scale_y)
                    .range(0.01..=16.0)
                    .speed(0.01),
            );
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut t.rotation, -180.0..=180.0)
                    .text("Rotation")
                    .suffix(" deg"),
            );
            self.key_buttons(ui, id, KeyParam::Rotation);
        });
        ui.label(RichText::new("Crop").color(theme::TEXT_DIM));
        ui.add(egui::Slider::new(&mut t.crop_left, 0.0..=0.9).text("Left"));
        ui.add(egui::Slider::new(&mut t.crop_right, 0.0..=0.9).text("Right"));
        ui.add(egui::Slider::new(&mut t.crop_top, 0.0..=0.9).text("Top"));
        ui.add(egui::Slider::new(&mut t.crop_bottom, 0.0..=0.9).text("Bottom"));
        if *t != before {
            // Animated values become keyframes; the rest are the clip's own.
            let Some(mut base) = self.project().clip(id).map(|(_, c)| c.transform) else {
                return;
            };
            let zoom_now = (t.scale_x + t.scale_y) * 0.5;
            if t.x != before.x && !self.set_animated(ctx, id, KeyParam::PositionX, t.x) {
                base.x = t.x;
            }
            if t.y != before.y && !self.set_animated(ctx, id, KeyParam::PositionY, t.y) {
                base.y = t.y;
            }
            if t.rotation != before.rotation
                && !self.set_animated(ctx, id, KeyParam::Rotation, t.rotation)
            {
                base.rotation = t.rotation;
            }
            if (t.scale_x != before.scale_x || t.scale_y != before.scale_y)
                && !(zoom && self.set_animated(ctx, id, KeyParam::Zoom, zoom_now))
            {
                base.scale_x = t.scale_x;
                base.scale_y = t.scale_y;
            }
            base.crop_left = t.crop_left;
            base.crop_right = t.crop_right;
            base.crop_top = t.crop_top;
            base.crop_bottom = t.crop_bottom;
            if Some(base) != self.project().clip(id).map(|(_, c)| c.transform) {
                self.run_gesture(
                    ctx,
                    Command::SetClipTransform {
                        clip: id,
                        transform: base,
                    },
                );
            }
        }
        let animated = self
            .project()
            .clip(id)
            .is_some_and(|(_, c)| !c.keyframes.is_empty());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!t.is_identity(), egui::Button::new("Reset transform"))
                .clicked()
            {
                self.run(Command::SetClipTransform {
                    clip: id,
                    transform: Transform::default(),
                });
            }
            if ui
                .add_enabled(animated, egui::Button::new("Clear keyframes"))
                .on_hover_text("Take away every keyframe of this clip")
                .clicked()
            {
                let commands = KeyParam::ALL
                    .iter()
                    .map(|&param| Command::ClearKeyframes { clip: id, param })
                    .collect();
                if let Err(e) = self.session.dispatch_all(commands) {
                    self.fail(e);
                }
            }
        });
    }
}

/// The Text+ look of a title: alignment, spacing, opacity, outline, shadow and background.
fn text_plus_controls(ui: &mut Ui, st: &mut picasso_core::TextStyle) {
    use picasso_core::TextAlign;
    egui::CollapsingHeader::new("Text+")
        .default_open(!st.is_plain())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Alignment");
                for (align, name) in [
                    (TextAlign::Left, "Left"),
                    (TextAlign::Center, "Center"),
                    (TextAlign::Right, "Right"),
                ] {
                    ui.selectable_value(&mut st.align, align, name);
                }
            });
            ui.add(egui::Slider::new(&mut st.tracking, -300..=1000).text("Tracking"));
            ui.add(
                egui::Slider::new(&mut st.line_spacing, 50..=400)
                    .text("Line Spacing")
                    .suffix(" %"),
            );
            ui.add(
                egui::Slider::new(&mut st.opacity, 0..=100)
                    .text("Opacity")
                    .suffix(" %"),
            );
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Outline");
                ui.color_edit_button_srgb(&mut st.outline_color);
            });
            ui.add(egui::Slider::new(&mut st.outline, 0..=100).text("Thickness"));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Drop Shadow");
                ui.color_edit_button_srgb(&mut st.shadow_color);
            });
            ui.add(egui::Slider::new(&mut st.shadow, 0..=200).text("Offset"));
            ui.add(egui::Slider::new(&mut st.shadow_blur, 0..=100).text("Softness"));
            ui.add(
                egui::Slider::new(&mut st.shadow_opacity, 0..=100)
                    .text("Shadow Opacity")
                    .suffix(" %"),
            );
            ui.separator();
            ui.horizontal(|ui| {
                ui.checkbox(&mut st.background, "Background");
                ui.color_edit_button_srgb(&mut st.background_color);
            });
            ui.add_enabled_ui(st.background, |ui| {
                ui.add(
                    egui::Slider::new(&mut st.background_opacity, 0..=100)
                        .text("Box Opacity")
                        .suffix(" %"),
                );
                ui.add(egui::Slider::new(&mut st.padding, 0..=300).text("Padding"));
            });
            if ui
                .add_enabled(!st.is_plain(), egui::Button::new("Reset Text+"))
                .clicked()
            {
                *st = picasso_core::TextStyle::default();
            }
        });
}
