//! The window's frame: the title and menu row, the first toolbar and the page bar.
//!
//! Everything is placed at fixed coordinates of a 1872-point-wide window (the interface is
//! scaled to that width, see `app.rs`). The rows are measured from the reference screen; the
//! height of the window only changes how tall the panels in the middle are.

use egui::{
    pos2, vec2, Align, Align2, Color32, Context, Layout, Pos2, Rect, Response, RichText, Sense, Ui,
    UiBuilder, ViewportCommand,
};

use crate::app::{Page, StudioApp};
use crate::icons::{paint, Icon};
use crate::menu::Act;
use crate::theme::{self, regular, semibold};

/// The width of the window in points; the interface is scaled so the window is always this wide.
pub(crate) const DESIGN_WIDTH: f32 = 1872.0;

/// Converts positions measured in the reference to the window.
#[derive(Clone, Copy)]
pub(crate) struct Screen {
    pub o: Pos2,
    pub h: f32,
    /// Where the page's dragged dividers move its points (`crate::panels`).
    pub warp: crate::panels::Warp,
}

impl Screen {
    pub fn p(&self, x: f32, y: f32) -> Pos2 {
        let (x, y) = self.warp.apply(x, y);
        pos2(self.o.x + x, self.o.y + y)
    }

    pub fn r(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        let middle = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (x0, y0) = self.warp.apply_within(x0, y0, middle);
        let (x1, y1) = self.warp.apply_within(x1, y1, middle);
        Rect::from_min_max(
            pos2(self.o.x + x0, self.o.y + y0),
            pos2(self.o.x + x1, self.o.y + y1),
        )
    }

    /// The ends of a line between two points in this screen's own points: whether a divider
    /// moves it is decided by its middle, as for `r`, so it is never moved at one end only.
    pub fn segment(&self, a: (f32, f32), b: (f32, f32)) -> [Pos2; 2] {
        let middle = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let (ax, ay) = self.warp.apply_within(a.0, a.1, middle);
        let (bx, by) = self.warp.apply_within(b.0, b.1, middle);
        [
            pos2(self.o.x + ax, self.o.y + ay),
            pos2(self.o.x + bx, self.o.y + by),
        ]
    }

    /// How much the dividers squeezed the page sideways at a point (1 where nothing is
    /// narrower than usual).
    pub fn squeeze(&self, x: f32, y: f32) -> f32 {
        let (a, _) = self.warp.apply(x - 0.5, y);
        let (b, _) = self.warp.apply(x + 0.5, y);
        (b - a).clamp(0.1, 1.0)
    }

    /// How much smaller text drawn at a point has to be: the dividers squeezed the page there
    /// sideways or up and down (1 where nothing is smaller than usual).
    pub fn shrink(&self, x: f32, y: f32) -> f32 {
        let (_, a) = self.warp.apply(x, y - 0.5);
        let (_, b) = self.warp.apply(x, y + 0.5);
        self.squeeze(x, y).min((b - a).clamp(0.1, 1.0))
    }

    /// `tool` drawn at `x`, `y` made smaller where a divider squeezed its panel, so the icons
    /// of a narrowed toolbar do not run into one another.
    pub fn squeezed(&self, mut tool: Tool, x: f32, y: f32) -> Tool {
        let k = self.squeeze(x, y);
        tool.scale *= k;
        tool.width *= k;
        tool.chevron = tool.chevron.map(|dx| dx * k);
        tool
    }

    /// The same screen with every divider at its usual place.
    pub fn usual(&self) -> Screen {
        Screen {
            warp: Default::default(),
            ..*self
        }
    }

    /// How much bigger than usual the dividers made a panel, by its height and by its width.
    pub fn grown(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> (f32, f32) {
        let (now, usual) = (self.r(x0, y0, x1, y1), self.usual().r(x0, y0, x1, y1));
        (
            now.height() / usual.height().max(1.0),
            now.width() / usual.width().max(1.0),
        )
    }

    /// A rectangle between two points given in this screen's own points (before the dividers
    /// move them), as `r` places it.
    pub fn between(&self, a: (f32, f32), b: (f32, f32)) -> Rect {
        self.r(a.0, a.1, b.0, b.1)
    }
}

/// How an icon button looks and what it says when hovered.
#[derive(Clone, Copy)]
pub(crate) struct Tool {
    pub hint: &'static str,
    pub dim: bool,
    /// Draw a small drop-down arrow this many points right of the icon.
    pub chevron: Option<f32>,
    pub color: Option<Color32>,
    pub width: f32,
    /// Makes the icon this much bigger or smaller than its usual size.
    pub scale: f32,
}

impl Tool {
    pub fn new(hint: &'static str) -> Self {
        Self {
            hint,
            dim: false,
            chevron: None,
            color: None,
            width: 46.0,
            scale: 1.0,
        }
    }
    pub fn dim(mut self) -> Self {
        self.dim = true;
        self
    }
    pub fn chev(mut self, dx: f32) -> Self {
        self.chevron = Some(dx);
        self
    }
    pub fn color(mut self, c: Color32) -> Self {
        self.color = Some(c);
        self
    }
    pub fn size(mut self, k: f32) -> Self {
        self.scale = k;
        self
    }
    pub fn wide(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
}

/// The page tabs, left to right: icon, name, and the page if it exists yet.
const TABS: [(Icon, &str, Option<Page>); 8] = [
    (Icon::PageMedia, "Media", Some(Page::Media)),
    (Icon::PageCapture, "Photo", Some(Page::Capture)),
    (Icon::PageCut, "Cut", Some(Page::Cut)),
    (Icon::PageEdit, "Edit", Some(Page::Edit)),
    (Icon::PageEffects, "Fusion", Some(Page::Effects)),
    (Icon::PageColor, "Color", Some(Page::Color)),
    (Icon::PageAudio, "Fairlight", Some(Page::Audio)),
    (Icon::PageDeliver, "Deliver", Some(Page::Deliver)),
];

/// The page a name stands for, as the tabs are called: `photo` is the Capture page (its tab
/// says Photo), `fusion` the Effects page and `fairlight` the Audio one, beside the plain names.
pub(crate) fn page_from_name(name: &str) -> Option<Page> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "media" => Page::Media,
        "photo" | "capture" => Page::Capture,
        "cut" => Page::Cut,
        "edit" => Page::Edit,
        "fusion" | "effects" => Page::Effects,
        "color" => Page::Color,
        "fairlight" | "audio" => Page::Audio,
        "deliver" => Page::Deliver,
        _ => return None,
    })
}

impl StudioApp {
    /// An icon button at `c`. Returns the response; a dim button never reports a click.
    pub(crate) fn tool(&self, ui: &mut Ui, c: Pos2, icon: Icon, tool: Tool) -> Response {
        let reach = tool.chevron.map_or(0.0, |dx| dx / 2.0);
        let rect = Rect::from_center_size(
            pos2(c.x + reach, c.y),
            vec2(
                tool.width + reach.abs() * 2.0,
                40.0 * tool.scale.clamp(0.7, 1.0),
            ),
        );
        let sense = if tool.dim {
            Sense::hover()
        } else {
            Sense::click()
        };
        let response = ui.interact(
            rect,
            ui.id().with(("tool", icon as u8, c.x as i32, c.y as i32)),
            sense,
        );
        let hovered = response.hovered() && !tool.dim;
        let painter = ui.painter();
        if hovered {
            painter.rect_filled(
                rect.shrink2(vec2(2.0, 3.0)),
                3.0,
                Color32::from_white_alpha(10),
            );
        }
        let color = if tool.dim {
            theme::ICON_DIM
        } else if hovered {
            theme::ICON_HOVER
        } else {
            tool.color.unwrap_or(theme::ICON)
        };
        crate::icons::paint_scaled(painter, c, icon, color, tool.scale);
        if let Some(dx) = tool.chevron {
            crate::icons::paint_scaled(
                painter,
                pos2(c.x + dx, c.y),
                Icon::Chevron,
                color,
                tool.scale,
            );
        }
        if !tool.hint.is_empty() && !tool.dim {
            return response.on_hover_text(tool.hint);
        }
        response
    }

    /// Says that a control is not built yet, in the status line.
    pub(crate) fn planned(&mut self, what: &str) {
        self.info(format!("{what} is planned and does not work yet"));
    }

    // ----- title and menu row -----

    pub(crate) fn menu_row(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        let painter = ui.painter().clone();
        painter.rect_filled(s.r(0.0, 0.0, DESIGN_WIDTH, 38.0), 0.0, theme::MENU_BG);
        painter.rect_filled(
            s.r(0.0, 38.0, DESIGN_WIDTH, 39.0),
            0.0,
            theme::SEPARATOR_DARK,
        );
        painter.rect_filled(
            s.r(0.0, 39.0, DESIGN_WIDTH, 40.0),
            0.0,
            theme::SEPARATOR_SOFT,
        );

        // Moving the window by its title row, and maximising by a double click.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let drag = ui.interact(
                s.r(3.0, 0.0, 1700.0, 38.0),
                ui.id().with("title_drag"),
                Sense::click_and_drag(),
            );
            if drag.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
        }

        paint_logo(
            ctx,
            &painter,
            s.p(27.0, 19.0),
            s.p(0.0, 0.0).distance(s.p(28.0, 0.0)),
        );
        crate::i18n::paint_text(&painter,
            s.p(57.0, 16.5),
            Align2::LEFT_CENTER,
            "Picasso Suite",
            regular(18.0),
            theme::MENU_TEXT,
        );

        for (n, (label, x, entries)) in crate::menu::MENUS.into_iter().enumerate() {
            // The slot is the English name's; another language's name is a short form that fits
            // it (`crate::i18n`).
            let rect = s.r(x - 11.0, -4.0, x + label.len() as f32 * 10.5 + 11.0, 34.0);
            let shown = crate::i18n::menu_name(label);
            let size = if shown == label {
                18.0
            } else {
                let room = crate::menu::MENUS
                    .get(n + 1)
                    .map_or(120.0, |next| next.1 - x - 8.0);
                let room = s.r(0.0, 0.0, room, 0.0).width();
                menu_name_size(ui, shown, room)
            };
            // In a layer above the pages: the Media and Deliver pages are layers of their own, and
            // a layer over the title row would take the clicks the menus are meant to get.
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(rect)
                    .layer_id(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new(("menu_row", n)),
                    ))
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    let style = ui.style_mut();
                    for w in [
                        &mut style.visuals.widgets.inactive,
                        &mut style.visuals.widgets.hovered,
                        &mut style.visuals.widgets.active,
                        &mut style.visuals.widgets.open,
                    ] {
                        w.weak_bg_fill = Color32::TRANSPARENT;
                        w.bg_stroke = egui::Stroke::NONE;
                    }
                    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_white_alpha(12);
                    style.visuals.widgets.open.weak_bg_fill = Color32::from_white_alpha(18);
                    style.spacing.button_padding = vec2(11.0, 6.0);
                    ui.set_min_height(38.0);
                    ui.menu_button(
                        RichText::new(shown).size(size).color(theme::MENU_TEXT),
                        |ui| {
                            ui.set_min_width(300.0);
                            self.draw_entries(ui, ctx, entries);
                        },
                    );
                },
            );
        }

        // What just happened.
        if !self.status.text.is_empty() {
            let color = if self.status.error {
                theme::ERROR
            } else {
                theme::TEXT_DIM
            };
            crate::i18n::paint_text(&painter,
                s.p(1700.0, 19.0),
                Align2::RIGHT_CENTER,
                &self.status.text,
                regular(15.0),
                color,
            );
        }

        #[cfg(not(target_arch = "wasm32"))]
        self.window_buttons(ui, ctx, s);
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn window_buttons(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let buttons = [
            (1741.0, "Minimize", Icon::Minimize),
            (
                1795.0,
                if maximized { "Restore" } else { "Maximize" },
                Icon::Restore,
            ),
            (1847.0, "Close", Icon::Close),
        ];
        for (n, (x, hint, icon)) in buttons.into_iter().enumerate() {
            let rect = s.r(x - 27.0, 0.0, x + 27.0, 38.0);
            let r = ui
                .interact(rect, ui.id().with(("window_button", n)), Sense::click())
                .on_hover_text(hint);
            if r.hovered() {
                let fill = if n == 2 {
                    Color32::from_rgb(196, 43, 28)
                } else {
                    Color32::from_white_alpha(16)
                };
                ui.painter().rect_filled(rect, 0.0, fill);
            }
            let color = if r.hovered() && n == 2 {
                Color32::WHITE
            } else {
                Color32::from_rgb(114, 116, 117)
            };
            paint(ui.painter(), s.p(x, 17.0), icon, color);
            if r.clicked() {
                match n {
                    0 => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                    1 => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                    _ => ctx.send_viewport_cmd(ViewportCommand::Close),
                }
            }
        }
    }

    /// Thin zones along the window's edges that start a resize (the window has no frame).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn resize_edges(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        use egui::viewport::ResizeDirection as D;
        use egui::CursorIcon;
        if ctx.input(|i| {
            i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
        }) {
            return;
        }
        let (w, h, e) = (DESIGN_WIDTH, s.h, 4.0);
        let zones = [
            (s.r(e, 0.0, w - e, e), D::North, CursorIcon::ResizeVertical),
            (
                s.r(e, h - e, w - e, h),
                D::South,
                CursorIcon::ResizeVertical,
            ),
            (s.r(0.0, e, e, h - e), D::West, CursorIcon::ResizeHorizontal),
            (
                s.r(w - e, e, w, h - e),
                D::East,
                CursorIcon::ResizeHorizontal,
            ),
            (s.r(0.0, 0.0, e, e), D::NorthWest, CursorIcon::ResizeNwSe),
            (s.r(w - e, 0.0, w, e), D::NorthEast, CursorIcon::ResizeNeSw),
            (s.r(0.0, h - e, e, h), D::SouthWest, CursorIcon::ResizeNeSw),
            (
                s.r(w - e, h - e, w, h),
                D::SouthEast,
                CursorIcon::ResizeNwSe,
            ),
        ];
        for (n, (rect, dir, cursor)) in zones.into_iter().enumerate() {
            let r = ui.interact(
                rect,
                ui.id().with(("resize_edge", n)),
                Sense::click_and_drag(),
            );
            if r.hovered() || r.dragged() {
                ctx.set_cursor_icon(cursor);
            }
            if r.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            }
        }
    }

    // ----- the first toolbar -----

    /// The project's name in the middle of the first toolbar, with "Edited" after it in yellow
    /// once something has changed.
    fn project_title(&self, painter: &egui::Painter, s: &Screen, y: f32) {
        let name = &self.project().name;
        // The Capture page's reference has a smaller title.
        let font = if self.page == Page::Capture {
            crate::capture::fit(painter, "New Project 1", 126.4, true)
        } else if self.page == Page::Audio {
            crate::capture::fit(painter, "New Project 1", 144.1, true)
        } else if self.page == Page::Effects {
            crate::capture::fit(painter, "New Project 1", 139.5, true)
        } else if self.page == Page::Color {
            crate::capture::fit(painter, "New Project 1", 142.0, true)
        } else if self.page == Page::Edit {
            crate::capture::fit(painter, "New Project 1", 144.0, true)
        } else {
            semibold(21.0)
        };
        if !self.session.is_dirty() {
            crate::i18n::paint_text(&painter,
                s.p(937.0, y - 1.0),
                Align2::CENTER_CENTER,
                name,
                font,
                theme::WHITE,
            );
            return;
        }
        let width = painter
            .layout_no_wrap(name.clone(), font.clone(), theme::WHITE)
            .size()
            .x;
        let left = 925.2 - width / 2.0;
        crate::i18n::paint_text(&painter,
            s.p(left, y + 0.7),
            Align2::LEFT_CENTER,
            name,
            font.clone(),
            theme::WHITE,
        );
        let right = left + width;
        crate::i18n::paint_text(&painter,
            s.p(right + 11.7, y),
            Align2::CENTER_CENTER,
            "|",
            theme::regular(19.0),
            Color32::from_rgb(150, 150, 152),
        );
        crate::i18n::paint_text(&painter,
            s.p(right + 49.6, y + 0.5),
            Align2::LEFT_CENTER,
            "Edited",
            crate::capture::fit(painter, "Edited", 53.3, false),
            Color32::from_rgb(196, 196, 62),
        );
    }

    pub(crate) fn toolbar1(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        // Pages laid out on the 1080p Resolve screenshots draw their own, labelled toolbar.
        self.page_toolbar1(ui, ctx, s);
        if true {
            return;
        }
        let painter = ui.painter().clone();
        painter.rect_filled(s.r(0.0, 40.0, DESIGN_WIDTH, 90.0), 0.0, theme::TOOLBAR1_BG);
        painter.rect_filled(
            s.r(0.0, 90.0, DESIGN_WIDTH, 91.0),
            0.0,
            Color32::from_rgb(17, 19, 20),
        );
        painter.rect_filled(
            s.r(0.0, 91.0, DESIGN_WIDTH, 92.0),
            0.0,
            Color32::from_rgb(12, 13, 15),
        );
        let dividers: &[f32] = if self.page == Page::Capture {
            &[74.0, 145.0, 1724.0, 1798.0]
        } else if self.page == Page::Audio {
            &[79.0, 154.0, 229.0, 304.0, 379.0, 1648.0, 1723.0, 1798.0]
        } else if self.page == Page::Effects {
            &[83.3, 159.1, 234.0, 308.9, 1575.0, 1650.0, 1725.0, 1800.0]
        } else if self.page == Page::Color {
            &[84.2, 159.1, 232.1, 1578.1, 1652.0, 1726.9, 1801.8]
        } else if self.page == Page::Edit {
            &[
                84.2, 159.1, 234.0, 308.9, 383.8, 1572.5, 1647.4, 1722.2, 1797.1,
            ]
        } else {
            &[80.0, 154.5, 229.0, 303.5, 378.5, 1649.0, 1724.0, 1798.0]
        };
        for x in dividers.iter().copied() {
            painter.line_segment(
                [s.p(x, 45.0), s.p(x, 86.0)],
                egui::Stroke::new(1.0, Color32::from_rgb(58, 58, 60)),
            );
        }
        let y = 64.0;
        if self.page == Page::Capture {
            self.capture_toolbar1(ui, ctx, s, y);
            self.project_title(&painter, s, y);
            return;
        }
        if self.page == Page::Audio {
            self.fairlight_toolbar1(ui, ctx, s, 66.0);
            self.project_title(&painter, s, 66.0);
            return;
        }
        if self.page == Page::Effects {
            self.effects_toolbar1(ui, ctx, s, 66.0);
            self.project_title(&painter, s, 66.0);
            return;
        }
        if self.page == Page::Color {
            self.color_toolbar1(ui, ctx, s, 66.0);
            self.project_title(&painter, s, 66.0);
            return;
        }
        if self.page == Page::Edit {
            self.edit_toolbar1(ui, s, 66.0);
            self.project_title(&painter, s, 66.0);
            return;
        }
        if self
            .tool(
                ui,
                s.p(43.0, y),
                Icon::MediaImport,
                Tool::new("Import media"),
            )
            .clicked()
        {
            self.import_dialog(ctx);
        }
        if self
            .tool(ui, s.p(117.0, y), Icon::SyncBin, Tool::new("Sync bin"))
            .clicked()
        {
            self.planned("Sync bin");
        }
        if self
            .tool(ui, s.p(192.0, y), Icon::Mail, Tool::new("Transitions"))
            .clicked()
        {
            self.do_act(ctx, Act::AddCrossDissolve);
        }
        if self
            .tool(ui, s.p(267.0, y), Icon::TitleT, Tool::new("Titles"))
            .clicked()
        {
            self.do_act(ctx, Act::AddTitle);
        }
        if self
            .tool(ui, s.p(342.0, y), Icon::Wand, Tool::new("Effects"))
            .clicked()
        {
            self.page = Page::Effects;
        }
        if self
            .tool(
                ui,
                s.p(415.0, y),
                Icon::Diamonds,
                Tool::new("Speed effects"),
            )
            .clicked()
        {
            self.do_act(ctx, Act::ChangeClipSpeed);
        }
        self.project_title(&painter, s, y);
        if self
            .tool(ui, s.p(1612.0, y), Icon::Export, Tool::new("Export"))
            .clicked()
        {
            self.page = Page::Deliver;
        }
        if self
            .tool(
                ui,
                s.p(1687.0, y),
                Icon::Fullscreen,
                Tool::new("Full screen"),
            )
            .clicked()
        {
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
        }
        if self
            .tool(ui, s.p(1761.0, y), Icon::Mixer, Tool::new("Mixer"))
            .clicked()
        {
            self.page = Page::Audio;
        }
        if self
            .tool(
                ui,
                s.p(1835.0, y),
                Icon::Tools,
                Tool::new("Project settings"),
            )
            .clicked()
        {
            self.show_settings = true;
        }
    }

    // ----- the page bar -----

    pub(crate) fn page_bar(&mut self, ui: &mut Ui, s: &Screen) {
        // Measured on Resolve at 1920 x 1080 (`docs/reference/resolve-1080p`), in its pixels
        // scaled by `R`: 48 high with a dark line on top, the pages 102 apart from 603, each an
        // icon over its name, the open one on black with a red line under it.
        const R: f32 = DESIGN_WIDTH / 1919.0;
        let ctx = ui.ctx().clone();
        let top = s.h - 48.0 * R;
        let at = |x: f32, from_bottom: f32| s.p(x * R, s.h - from_bottom * R);
        let painter = ui.painter().clone();
        painter.rect_filled(s.r(0.0, top, DESIGN_WIDTH, s.h), 0.0, theme::PAGEBAR_BG);
        painter.rect_filled(
            s.r(0.0, top, DESIGN_WIDTH, top + R),
            0.0,
            Color32::from_rgb(9, 9, 11),
        );
        paint_logo(&ctx, &painter, at(55.0, 23.5), 17.0 * R);
        crate::i18n::paint_text(&painter,
            at(73.0, 23.5),
            Align2::LEFT_CENTER,
            format!("Picasso Suite {}", env!("CARGO_PKG_VERSION")),
            semibold(13.0 * R),
            theme::WHITE,
        );

        for (n, (icon, name, page)) in TABS.into_iter().enumerate() {
            let cx = 603.0 + 102.0 * n as f32;
            let rect = Rect::from_min_max(
                pos2(s.o.x + (cx - 50.5) * R, top + R),
                pos2(s.o.x + (cx + 50.5) * R, s.o.y + s.h),
            );
            let selected = page == Some(self.page);
            let r = ui
                .interact(rect, ui.id().with(("page_tab", n)), Sense::click())
                .on_hover_text(if page.is_some() {
                    name.to_string()
                } else {
                    format!("{name} (planned)")
                });
            if selected {
                painter.rect_filled(rect, 0.0, Color32::BLACK);
                // The line under the page that is showing.
                painter.rect_filled(
                    Rect::from_min_max(pos2(rect.left(), s.o.y + s.h - 2.0 * R), rect.max),
                    0.0,
                    Color32::from_rgb(217, 71, 62),
                );
            }
            let color = if selected {
                Color32::from_rgb(154, 180, 209)
            } else if r.hovered() {
                theme::ICON_HOVER
            } else {
                Color32::from_rgb(132, 131, 134)
            };
            // Each page's icon shows its colours while the page is open.
            let icon = match icon {
                _ if !selected => icon,
                Icon::PageMedia => Icon::PageMediaOn,
                Icon::PageCapture => Icon::PageCaptureOn,
                Icon::PageCut => Icon::PageCutOn,
                Icon::PageEdit => Icon::PageEditOn,
                Icon::PageEffects => Icon::PageEffectsOn,
                Icon::PageColor => Icon::PageColorOn,
                Icon::PageAudio => Icon::PageAudioOn,
                Icon::PageDeliver => Icon::PageDeliverOn,
                other => other,
            };
            crate::icons::paint_scaled(&painter, at(cx, 31.5), icon, color, 0.6 * R);
            crate::i18n::paint_text(&painter,
                at(cx, 12.5),
                Align2::CENTER_CENTER,
                crate::i18n::tr_ui(name),
                if selected {
                    semibold(12.5 * R)
                } else {
                    theme::regular(12.5 * R)
                },
                if selected {
                    theme::WHITE
                } else if r.hovered() {
                    theme::ICON_HOVER
                } else {
                    Color32::from_rgb(140, 140, 144)
                },
            );
            if r.clicked() {
                match page {
                    Some(page) => self.page = page,
                    None => self.planned(name),
                }
            }
        }
        let home = at(1835.0, 16.0);
        crate::icons::paint_scaled(
            &painter,
            home,
            Icon::Home,
            Color32::from_rgb(135, 137, 138),
            0.6 * R,
        );
        if ui
            .interact(
                Rect::from_center_size(home, vec2(30.0 * R, 30.0 * R)),
                ui.id().with("page_bar_home"),
                Sense::click(),
            )
            .on_hover_text("Projects")
            .clicked()
        {
            self.show_project_manager();
        }
        let gear = at(1884.0, 16.0);
        crate::icons::paint_scaled(
            &painter,
            gear,
            Icon::Settings,
            Color32::from_rgb(135, 137, 138),
            0.6 * R,
        );
        if ui
            .interact(
                Rect::from_center_size(gear, vec2(30.0 * R, 30.0 * R)),
                ui.id().with("page_bar_settings"),
                Sense::click(),
            )
            .on_hover_text("Project settings")
            .clicked()
        {
            self.show_settings = !self.show_settings;
        }
    }

    // ----- small windows -----

    pub(crate) fn windows(&mut self, ctx: &Context) {
        let mut projects = self.show_projects;
        if projects {
            egui::Window::new("Projects")
                .open(&mut projects)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.set_min_width(260.0);
                    if ui.button("New project").clicked() {
                        self.new_project();
                        self.show_projects = false;
                    }
                    if ui.button("Open project...").clicked() {
                        self.open_dialog(ctx);
                        self.show_projects = false;
                    }
                    if ui.button("Save project").clicked() {
                        self.save(ctx, false);
                    }
                    if ui.button("Load demo project").clicked() {
                        self.load_demo();
                        self.show_projects = false;
                    }
                });
            self.show_projects &= projects;
        }
        let mut settings = self.show_settings;
        if settings {
            egui::Window::new("Project settings")
                .open(&mut settings)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    self.format_settings(ui, ctx);
                });
            self.show_settings &= settings;
        }
        self.extra_windows(ctx);
    }
}

/// The font size of a menu name of the title row: 18, or smaller when the name (in a language
/// whose short form is still a wide one) would run into the next menu. English never shrinks.
fn menu_name_size(ui: &Ui, name: &str, room: f32) -> f32 {
    let wide = ui
        .painter()
        .layout_no_wrap(name.to_string(), regular(18.0), theme::MENU_TEXT)
        .size()
        .x;
    if wide <= room || wide <= 0.0 || room <= 0.0 {
        return 18.0;
    }
    (18.0 * room / wide).max(12.0)
}

/// The app's picture (96 x 96, raw RGBA), drawn `size` points wide and centred on `c`.
pub(crate) fn paint_logo(ctx: &Context, painter: &egui::Painter, c: egui::Pos2, size: f32) {
    const SIDE: usize = 96;
    let id = egui::Id::new("picasso_logo");
    let texture = ctx.data_mut(|d| d.get_temp::<egui::TextureHandle>(id));
    let texture = texture.unwrap_or_else(|| {
        let bytes: &[u8] = include_bytes!("../assets/logo-96.rgba");
        let image = egui::ColorImage::from_rgba_unmultiplied([SIDE, SIDE], bytes);
        let handle = ctx.load_texture("picasso_logo", image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
        handle
    });
    painter.image(
        texture.id(),
        egui::Rect::from_center_size(c, egui::vec2(size, size)),
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// Pixels of the 1920 x 1080 Resolve screenshots (`docs/reference/resolve-1080p`) in points:
/// the pages measured on them are drawn at Resolve's own density on any window.
pub(crate) const R: f32 = DESIGN_WIDTH / 1919.0;

/// One button of the top toolbar: an icon with its name beside it, lit when its panel shows.
#[derive(Clone, Copy)]
pub(crate) struct ToolbarItem {
    pub icon: Icon,
    pub label: &'static str,
    pub lit: bool,
    /// Followed by a drop-down arrow.
    pub drop: bool,
}

/// Which toolbar button was clicked: `false` for the left group, `true` for the right one.
pub(crate) type ToolbarClick = Option<(bool, usize)>;

impl StudioApp {
    /// The top toolbar as Resolve draws it at 1080p: 39 pixels high, buttons with their names
    /// from the left and from the right, thin lines between them, and the project's name in the
    /// middle. Returns the button clicked.
    pub(crate) fn labeled_toolbar(
        &mut self,
        ui: &mut Ui,
        s: &Screen,
        left: &[ToolbarItem],
        right: &[ToolbarItem],
    ) -> ToolbarClick {
        let painter = ui.painter().clone();
        let top = 40.0;
        let y = top + 19.5 * R;
        painter.rect_filled(
            s.r(0.0, top, DESIGN_WIDTH, top + 39.0 * R),
            0.0,
            theme::PAGEBAR_BG,
        );
        painter.rect_filled(
            s.r(0.0, top + 39.0 * R, DESIGN_WIDTH, top + 40.0 * R),
            0.0,
            Color32::from_rgb(8, 10, 11),
        );
        let font = theme::regular(12.8 * R);
        let width = |text: &str| {
            painter
                .layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        };
        let divider = |x: f32| {
            painter.line_segment(
                [s.p(x, top + 7.0 * R), s.p(x, top + 32.0 * R)],
                egui::Stroke::new(1.0, Color32::from_rgb(66, 68, 70)),
            );
        };
        let mut clicked = None;
        let mut item = |ui: &mut Ui, icon_x: f32, it: &ToolbarItem, n: (bool, usize)| -> f32 {
            let label_x = icon_x + 22.0 * R;
            let end = label_x + width(it.label);
            let rect = s.r(icon_x - 14.0 * R, top + 2.0, end + 6.0 * R, top + 37.0 * R);
            let r = ui.interact(rect, ui.id().with(("toolbar", n.0, n.1)), Sense::click());
            let color = if it.lit {
                Color32::from_rgb(244, 246, 248)
            } else if r.hovered() {
                theme::ICON_HOVER
            } else {
                Color32::from_rgb(142, 146, 150)
            };
            crate::icons::paint_scaled(&painter, s.p(icon_x, y), it.icon, color, 0.62 * R);
            crate::i18n::paint_text(&painter,
                s.p(label_x, y),
                Align2::LEFT_CENTER,
                it.label,
                font.clone(),
                color,
            );
            if r.clicked() {
                clicked = Some(n);
            }
            if it.drop {
                crate::icons::paint_scaled(
                    &painter,
                    s.p(end + 10.0 * R, y + R),
                    Icon::Chevron,
                    color,
                    0.62 * R,
                );
                return end + 16.0 * R;
            }
            end
        };
        let mut x = 31.0 * R;
        for (n, it) in left.iter().enumerate() {
            if n == 0 && left[0].icon == Icon::Sidebar {
                x = 20.0 * R;
            }
            let end = item(ui, x, it, (false, n));
            divider(end + 13.0 * R);
            x = end + 36.0 * R;
        }
        // From the right edge leftwards: the last name ends 19 pixels from the edge.
        let mut end = DESIGN_WIDTH - 19.0 * R;
        for (n, it) in right.iter().enumerate().rev() {
            let icon_x = end - width(it.label) - 22.0 * R;
            item(ui, icon_x, it, (true, n));
            let line = icon_x - 13.0 * R;
            if n > 0 {
                divider(line);
            }
            end = line - 13.0 * R;
        }
        crate::i18n::paint_text(&painter,
            s.p(958.0 * R, y),
            Align2::CENTER_CENTER,
            &self.project().name,
            semibold(15.0 * R),
            theme::WHITE,
        );
        clicked
    }
}

impl StudioApp {
    /// The top toolbar of every page, as Resolve draws it at 1080p: the panels' buttons with
    /// their names on the left, the other panels on the right.
    pub(crate) fn page_toolbar1(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        let it = |icon, label, lit, drop| ToolbarItem {
            icon,
            label,
            lit,
            drop,
        };
        let (left, right): (Vec<ToolbarItem>, Vec<ToolbarItem>) = match self.page {
            Page::Cut => return self.cut_toolbar1(ui, ctx, s),
            Page::Media => (
                vec![
                    it(Icon::MonitorUp, "Media Storage", true, false),
                    it(Icon::ClipFrame, "Clone Tool", false, false),
                ],
                vec![
                    it(Icon::MusicLines, "Audio", true, false),
                    it(Icon::LabelTag, "Metadata", true, false),
                    it(Icon::Tools, "Inspector", false, false),
                    it(Icon::PageCapture, "Capture", false, false),
                ],
            ),
            Page::Capture => (
                vec![
                    it(Icon::PhotoMusic, "Media Pool", true, false),
                    it(Icon::Wand, "Effects", false, false),
                    it(Icon::Photo, "Photo Album", true, true),
                ],
                vec![
                    it(Icon::Export, "Quick Export", false, false),
                    it(Icon::LabelTag, "Metadata", false, false),
                    it(Icon::Tools, "Inspector", true, false),
                ],
            ),
            Page::Edit => (
                vec![
                    it(
                        Icon::PhotoMusic,
                        "Media Pool",
                        !self.effects_library_open,
                        false,
                    ),
                    it(Icon::Wand, "Effects", self.effects_library_open, false),
                    it(Icon::ListBullets, "Index", false, false),
                    it(Icon::MusicLines, "Sound Library", false, false),
                    it(Icon::Diamonds, "Keyframes", false, false),
                ],
                vec![
                    it(Icon::Export, "Quick Export", false, false),
                    it(Icon::Mixer, "Mixer", false, false),
                    it(Icon::LabelTag, "Metadata", false, false),
                    it(Icon::Tools, "Inspector", self.inspector_open, false),
                ],
            ),
            Page::Effects => (
                vec![
                    it(Icon::PhotoMusic, "Media Pool", false, false),
                    it(Icon::Wand, "Effects", false, false),
                    it(Icon::ClipFrame, "Clips", false, false),
                    it(Icon::NodeTree, "Nodes", true, false),
                ],
                vec![
                    it(Icon::Spline, "Spline", false, false),
                    it(Icon::Diamonds3, "Keyframes", false, false),
                    it(Icon::LabelTag, "Metadata", false, false),
                    it(Icon::Tools, "Inspector", true, false),
                ],
            ),
            Page::Color => (
                vec![
                    it(Icon::Gallery, "Gallery", true, false),
                    it(Icon::LutGrid, "LUTs", false, false),
                    it(Icon::PhotoMusic, "Media Pool", false, false),
                    it(Icon::ClipFrame, "Clips", false, true),
                ],
                vec![
                    it(Icon::Export, "Quick Export", false, false),
                    it(Icon::PageEdit, "Timeline", true, false),
                    it(Icon::NodeTree, "Nodes", true, false),
                    it(Icon::FxBox, "Effects", false, false),
                    it(Icon::Lightbox, "Lightbox", false, false),
                ],
            ),
            Page::Audio => (
                vec![
                    it(Icon::PhotoMusic, "Media Pool", false, false),
                    it(Icon::Wand, "Effects", false, false),
                    it(Icon::ListBullets, "Index", false, false),
                    it(Icon::Grid3, "Groups", false, false),
                    it(Icon::MusicLines, "Sound Library", false, false),
                    it(Icon::Mic, "ADR", false, false),
                ],
                vec![
                    it(Icon::Mixer, "Mixer", true, false),
                    it(Icon::Bars, "Meters", true, false),
                    it(Icon::LabelTag, "Metadata", false, false),
                    it(Icon::Tools, "Inspector", false, false),
                ],
            ),
            Page::Deliver => (
                vec![
                    it(Icon::ClipFrame, "Render Settings", true, false),
                    it(Icon::FilmView, "Tape", false, false),
                    it(Icon::ClipFrame, "Clips", false, true),
                ],
                vec![it(Icon::Export, "Render Queue", true, false)],
            ),
        };
        let page = self.page;
        let Some((is_right, n)) = self.labeled_toolbar(ui, s, &left, &right) else {
            return;
        };
        let name = if is_right {
            right[n].label
        } else {
            left[n].label
        };
        use crate::app::Page as P;
        match (page, name) {
            (P::Edit, "Media Pool") => {
                self.effects_library_open = false;
                ctx.request_repaint();
            }
            (P::Edit, "Effects") => {
                self.effects_library_open = !self.effects_library_open;
                ctx.request_repaint();
            }
            (_, "Media Pool") | (_, "Media Storage") => self.import_dialog(ctx),
            (_, "Effects") if page != P::Color => self.page = P::Effects,
            (_, "Effects") => self.page = P::Effects,
            (_, "Quick Export") | (P::Deliver, _) if name != "Clips" && name != "Tape" => {
                self.page = P::Deliver
            }
            (_, "Mixer") => self.page = P::Audio,
            (_, "Inspector") => {
                if page == P::Edit {
                    self.inspector_open = !self.inspector_open;
                } else {
                    self.do_act(ctx, Act::ShowInspector);
                }
            }
            (_, "Keyframes") if page == P::Edit => self.inspector_open = true,
            (_, "LUTs") => self.lut_dialog(ctx),
            (_, "Timeline") => self.page = P::Edit,
            (_, "Capture") => self.page = P::Capture,
            (_, other) => self.planned(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_has_a_name_of_its_own_and_opens_its_page() {
        let mut names: Vec<&str> = TABS.iter().map(|(_, name, _)| *name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "two page tabs have the same name");
        for (_, name, page) in TABS {
            assert_eq!(
                page_from_name(name),
                page,
                "the {name} tab does not open its page"
            );
        }
    }

    #[test]
    fn the_pages_are_named_as_in_resolve_and_by_their_own_names() {
        assert_eq!(page_from_name("photo"), Some(Page::Capture));
        assert_eq!(page_from_name("Photo"), Some(Page::Capture));
        assert_eq!(page_from_name("capture"), Some(Page::Capture));
        assert_eq!(page_from_name("fusion"), Some(Page::Effects));
        assert_eq!(page_from_name("effects"), Some(Page::Effects));
        assert_eq!(page_from_name("fairlight"), Some(Page::Audio));
        assert_eq!(page_from_name("audio"), Some(Page::Audio));
        assert_eq!(page_from_name(" color "), Some(Page::Color));
        assert_eq!(page_from_name("nonsense"), None);
    }
}
