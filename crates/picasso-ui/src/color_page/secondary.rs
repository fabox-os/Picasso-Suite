//! Secondary correction on the Color page: the Qualifier palette (an HSL key), the Window
//! palette (power windows) and the windows drawn on the viewer, where they are moved, sized
//! and turned with the mouse.

use super::*;
use picasso_color::{PowerWindow, Qualifier, WindowShape, MAX_WINDOWS};
use picasso_core::{ClipId, MediaSource};

/// The palette shown under the clips, in place of the wheels.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Palette {
    #[default]
    Wheels,
    Qualifier,
    Window,
}

/// What a drag on the viewer changes in the chosen window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WindowDrag {
    Move,
    Width,
    Height,
    Rotate,
}

const ACCENT: Color32 = Color32::from_rgb(214, 128, 60);
const KEY_LINE: Color32 = Color32::from_rgb(236, 236, 240);

/// A small button with words, lit when `on`.
fn text_button(
    ui: &mut Ui,
    c: &Cv,
    id: &str,
    r: (f32, f32, f32, f32),
    text: &str,
    on: bool,
) -> bool {
    let rect = c.rect(r.0, r.1, r.2, r.3);
    let resp = ui.interact(rect, ui.id().with(id), Sense::click());
    let fill = if on {
        Color32::from_rgb(70, 70, 78)
    } else if resp.hovered() {
        Color32::from_rgb(52, 52, 58)
    } else {
        FIELD
    };
    c.p.rect_filled(rect, 3.0, fill);
    c.p.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, Color32::from_rgb(58, 58, 64)),
        StrokeKind::Inside,
    );
    c.label(
        (r.0 + r.2) / 2.0,
        (r.1 + r.3) / 2.0,
        Align2::CENTER_CENTER,
        text,
        15.0,
        if on { theme::WHITE } else { TEXT },
        false,
    );
    resp.clicked()
}

/// A horizontal bar of colours, `color(t)` for t from 0 to 1.
fn gradient_bar(c: &Cv, rect: Rect, color: impl Fn(f32) -> Color32) {
    let steps = 48;
    let mut mesh = egui::Mesh::default();
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = rect.left() + rect.width() * t;
        let col = color(t);
        mesh.colored_vertex(pos2(x, rect.top()), col);
        mesh.colored_vertex(pos2(x, rect.bottom()), col);
        if i > 0 {
            let n = (i * 2) as u32;
            mesh.add_triangle(n - 2, n - 1, n);
            mesh.add_triangle(n - 1, n, n + 1);
        }
    }
    c.p.add(egui::Shape::mesh(mesh));
}

fn hue_color(hue: f32) -> Color32 {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Darkens what lies outside `low..high` (fractions of the bar) and marks the range.
fn range_marks(c: &Cv, rect: Rect, low: f32, high: f32, soft: f32) {
    let shade = Color32::from_black_alpha(170);
    let x = |t: f32| rect.left() + rect.width() * t.clamp(0.0, 1.0);
    let dark = |a: f32, b: f32| {
        if b > a {
            c.p.rect_filled(
                Rect::from_x_y_ranges(x(a)..=x(b), rect.y_range()),
                0.0,
                shade,
            );
        }
    };
    if low <= high {
        dark(0.0, low);
        dark(high, 1.0);
    } else {
        // A hue range that wraps around red.
        dark(high, low);
    }
    for t in [low, high] {
        c.p.line_segment(
            [
                pos2(x(t), rect.top() - 3.0),
                pos2(x(t), rect.bottom() + 3.0),
            ],
            Stroke::new(2.0, KEY_LINE),
        );
    }
    for t in [low - soft, high + soft] {
        let t = t.rem_euclid(1.0001);
        c.p.line_segment(
            [pos2(x(t), rect.top()), pos2(x(t), rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(150)),
        );
    }
}

impl StudioApp {
    /// A mask from the Effects page: a power window on the clip's current node, shown in the
    /// Color page's Window palette.
    pub(crate) fn add_window_from_effects(&mut self, shape: WindowShape) {
        let Some((id, node, mut grade)) = self.edited_node() else {
            self.fail("there is no clip at the playhead");
            return;
        };
        if grade.windows.len() >= MAX_WINDOWS {
            self.fail("a node has at most eight windows");
            return;
        }
        grade.windows.push(PowerWindow::new(shape));
        let n = grade.windows.len() - 1;
        if self
            .run(Command::SetGrade {
                clip: id,
                grade,
                node,
            })
            .is_some()
        {
            self.color.window = n;
            self.color.palette = Palette::Window;
            self.page = crate::app::Page::Color;
        }
    }

    /// The clip and node the Color page edits, with that node's grade.
    fn edited_node(&self) -> Option<(ClipId, usize, ColorGrade)> {
        let id = self.current_clip()?;
        let node = self.node_for(id);
        let grade = self.project().clip(id)?.1.node(node)?.clone();
        Some((id, node, grade))
    }

    /// Keeps `grade` as the node's when it differs from `before` (a drag is one undo step).
    fn commit_node(
        &mut self,
        ctx: &Context,
        id: ClipId,
        node: usize,
        before: &ColorGrade,
        grade: ColorGrade,
    ) {
        let grade = grade.sanitized();
        if grade != *before {
            self.run_gesture(
                ctx,
                Command::SetGrade {
                    clip: id,
                    grade,
                    node,
                },
            );
        }
    }

    /// The palette header: its name, and on the right the Highlight and reset buttons.
    fn palette_header(&mut self, ui: &mut Ui, c: &Cv, title: &str) -> bool {
        c.band(3.0, 595.0, 1053.0, 633.0, BAR);
        c.label(
            18.0,
            614.0,
            Align2::LEFT_CENTER,
            title,
            17.5,
            theme::WHITE,
            true,
        );
        c.vline(1053.0, 595.0, PAGE_BAR);
        c.line((3.0, 633.0), (1053.0, 633.0), 1.0, LINE);
        if text_button(
            ui,
            c,
            "color_highlight",
            (850.0, 601.0, 980.0, 627.0),
            "Highlight",
            self.color.highlight,
        ) {
            self.color.highlight = !self.color.highlight;
        }
        c.vline(990.0, 595.0, 633.0);
        c.icon(1022.0, 614.0, Icon::Reset, GREY, 0.8);
        ui.interact(
            c.rect(1002.0, 597.0, 1042.0, 631.0),
            ui.id().with(("palette_reset", title)),
            Sense::click(),
        )
        .on_hover_text("Reset this palette")
        .clicked()
    }

    /// The Qualifier palette: the hue, saturation and luminance ranges as bars that can be
    /// dragged, their values, and the matte finesse.
    pub(super) fn color_qualifier(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        c.band(3.0, 633.0, 1053.0, PAGE_BAR, BG);
        let reset = self.palette_header(ui, c, "Qualifier - HSL");
        let Some((id, node, before)) = self.edited_node() else {
            c.text(
                528.0,
                820.0,
                Align2::CENTER_CENTER,
                "No clip to grade",
                130.0,
                GREY,
                false,
            );
            return;
        };
        let mut grade = before.clone();
        let mut q = grade.qualifier.clone().unwrap_or_default();
        if reset {
            q = Qualifier::default();
        }

        // The picker and Invert.
        let pick_on = self.color.picker == Some(Picker::Qualify);
        c.icon(
            700.0,
            614.0,
            Icon::Picker,
            if pick_on { theme::WHITE } else { GREY },
            0.85,
        );
        if ui
            .interact(
                c.rect(680.0, 597.0, 720.0, 631.0),
                ui.id().with("qualifier_pick"),
                Sense::click(),
            )
            .on_hover_text("Pick the colour to key in the viewer")
            .clicked()
        {
            self.color.picker = if pick_on { None } else { Some(Picker::Qualify) };
        }
        if text_button(
            ui,
            c,
            "qualifier_invert",
            (735.0, 601.0, 835.0, 627.0),
            "Invert",
            q.invert,
        ) {
            q.invert = !q.invert;
        }

        let rows = [("Hue", 685.0), ("Sat", 780.0), ("Lum", 875.0)];
        let (bx0, bx1) = (110.0, 600.0);
        for (n, (name, y)) in rows.into_iter().enumerate() {
            c.label(28.0, y, Align2::LEFT_CENTER, name, 17.0, TEXT, true);
            let rect = c.rect(bx0, y - 16.0, bx1, y + 16.0);
            let (low, high, soft) = match n {
                0 => {
                    gradient_bar(c, rect, |t| hue_color(t * 360.0));
                    if q.hue_width >= 360.0 {
                        (0.0, 1.0, 0.0)
                    } else {
                        let lo = (q.hue_center - q.hue_width / 2.0).rem_euclid(360.0) / 360.0;
                        let hi = (q.hue_center + q.hue_width / 2.0).rem_euclid(360.0) / 360.0;
                        (lo, hi, q.hue_soft / 360.0)
                    }
                }
                1 => {
                    let tint = hue_color(q.hue_center);
                    gradient_bar(c, rect, |t| {
                        let g = Color32::from_gray(128);
                        g.lerp_to_gamma(tint, t)
                    });
                    (q.sat_low, q.sat_high, q.sat_soft)
                }
                _ => {
                    gradient_bar(c, rect, |t| Color32::from_gray((t * 255.0) as u8));
                    (q.lum_low, q.lum_high, q.lum_soft)
                }
            };
            range_marks(c, rect, low, high, soft);
            c.p.rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0, Color32::from_rgb(58, 58, 64)),
                StrokeKind::Outside,
            );
            // A drag moves the range; a click on the hue bar centres it there.
            let resp = ui
                .interact(
                    rect,
                    ui.id().with(("qualifier_bar", n)),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text("Drag to move the range");
            let dt = resp.drag_delta().x / rect.width().max(1.0);
            if n == 0 {
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        q.hue_center = (p.x - rect.left()) / rect.width().max(1.0) * 360.0;
                        if q.hue_width >= 360.0 {
                            q.hue_width = 40.0;
                        }
                    }
                } else if dt != 0.0 {
                    q.hue_center = (q.hue_center + dt * 360.0).rem_euclid(360.0);
                }
            } else if dt != 0.0 {
                let (lo, hi) = if n == 1 {
                    (&mut q.sat_low, &mut q.sat_high)
                } else {
                    (&mut q.lum_low, &mut q.lum_high)
                };
                let span = *hi - *lo;
                let new_lo = (*lo + dt).clamp(0.0, 1.0 - span);
                (*lo, *hi) = (new_lo, new_lo + span);
            }

            // The values: centre, width and soft for the hue; low, high and soft for the rest.
            let fields: [(&str, f32, f32, f32); 3] = match n {
                0 => [
                    ("Center", q.hue_center, 0.5, 0.0),
                    ("Width", q.hue_width, 0.5, 360.0),
                    ("Soft", q.hue_soft, 0.2, 10.0),
                ],
                1 => [
                    ("Low", q.sat_low * 100.0, 0.25, 0.0),
                    ("High", q.sat_high * 100.0, 0.25, 100.0),
                    ("Soft", q.sat_soft * 100.0, 0.25, 5.0),
                ],
                _ => [
                    ("Low", q.lum_low * 100.0, 0.25, 0.0),
                    ("High", q.lum_high * 100.0, 0.25, 100.0),
                    ("Soft", q.lum_soft * 100.0, 0.25, 5.0),
                ],
            };
            for (f, (label, value, per, neutral)) in fields.into_iter().enumerate() {
                let x0 = 640.0 + f as f32 * 135.0;
                c.label(
                    x0 + 55.0,
                    y - 30.0,
                    Align2::CENTER_CENTER,
                    label,
                    14.0,
                    LABEL,
                    false,
                );
                if let Some(v) = value_box(
                    ui,
                    c,
                    ("qualifier_value", n * 3 + f),
                    (x0, y - 16.0, x0 + 110.0, y + 16.0),
                    value,
                    &format!("{value:.1}"),
                    per,
                    neutral,
                ) {
                    match (n, f) {
                        (0, 0) => q.hue_center = v.rem_euclid(360.0),
                        (0, 1) => q.hue_width = v.clamp(0.0, 360.0),
                        (0, _) => q.hue_soft = v.clamp(0.0, 180.0),
                        (1, 0) => q.sat_low = (v / 100.0).clamp(0.0, q.sat_high),
                        (1, 1) => q.sat_high = (v / 100.0).clamp(q.sat_low, 1.0),
                        (1, _) => q.sat_soft = (v / 100.0).clamp(0.0, 1.0),
                        (_, 0) => q.lum_low = (v / 100.0).clamp(0.0, q.lum_high),
                        (_, 1) => q.lum_high = (v / 100.0).clamp(q.lum_low, 1.0),
                        _ => q.lum_soft = (v / 100.0).clamp(0.0, 1.0),
                    }
                }
            }
        }

        // Matte finesse.
        c.line((3.0, 935.0), (1053.0, 935.0), 1.0, LINE);
        c.label(
            28.0,
            985.0,
            Align2::LEFT_CENTER,
            "Matte Finesse",
            16.0,
            TEXT,
            true,
        );
        let finesse: [(&str, f32, f32); 3] = [
            ("Clean Black", q.clean_black * 100.0, 0.0),
            ("Clean White", q.clean_white * 100.0, 0.0),
            ("Blur Radius", q.blur, 0.0),
        ];
        for (f, (label, value, neutral)) in finesse.into_iter().enumerate() {
            let x0 = 250.0 + f as f32 * 200.0;
            c.label(
                x0 + 70.0,
                962.0,
                Align2::CENTER_CENTER,
                label,
                14.0,
                LABEL,
                false,
            );
            if let Some(v) = value_box(
                ui,
                c,
                ("qualifier_finesse", f),
                (x0, 975.0, x0 + 140.0, 1007.0),
                value,
                &format!("{value:.1}"),
                0.25,
                neutral,
            ) {
                match f {
                    0 => q.clean_black = (v / 100.0).clamp(0.0, 0.99),
                    1 => q.clean_white = (v / 100.0).clamp(0.0, 0.99),
                    _ => q.blur = v.clamp(0.0, 100.0),
                }
            }
        }

        grade.qualifier = if q == Qualifier::default() {
            None
        } else {
            Some(q)
        };
        self.commit_node(ctx, id, node, &before, grade);
    }

    /// The Window palette: buttons that add a window of each shape, the node's windows (on or
    /// off, inverted, deleted), and the chosen window's transform.
    pub(super) fn color_window(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        c.band(3.0, 633.0, 1053.0, PAGE_BAR, BG);
        let reset = self.palette_header(ui, c, "Window");
        let Some((id, node, before)) = self.edited_node() else {
            c.text(
                528.0,
                820.0,
                Align2::CENTER_CENTER,
                "No clip to grade",
                130.0,
                GREY,
                false,
            );
            return;
        };
        let mut grade = before.clone();
        if reset {
            grade.windows.clear();
        }
        // What the window tracking reads: the clip's file and how many frames it has.
        let media_path = self
            .project()
            .clip(id)
            .and_then(|(_, clip)| self.project().media(clip.media))
            .and_then(|m| match &m.source {
                MediaSource::Video { path } | MediaSource::Image { path } => Some(path.clone()),
                _ => None,
            })
            .unwrap_or_default();

        // Adding windows.
        let shapes = [
            (WindowShape::Linear, Icon::FxRectMask, 40.0),
            (WindowShape::Circle, Icon::FxEllipseMask, 130.0),
            (WindowShape::Gradient, Icon::FxCurves, 220.0),
        ];
        for (shape, icon, x) in shapes {
            let rect = c.rect(x - 38.0, 645.0, x + 38.0, 700.0);
            let resp = ui
                .interact(
                    rect,
                    ui.id().with(("window_add", shape.label())),
                    Sense::click(),
                )
                .on_hover_text(format!("Add a {} window", shape.label().to_lowercase()));
            c.p.rect_filled(
                rect,
                3.0,
                if resp.hovered() {
                    Color32::from_rgb(52, 52, 58)
                } else {
                    FIELD
                },
            );
            c.icon(x, 663.0, icon, TEXT, 0.8);
            c.label(
                x,
                688.0,
                Align2::CENTER_CENTER,
                shape.label(),
                13.0,
                LABEL,
                false,
            );
            if resp.clicked() {
                if grade.windows.len() >= MAX_WINDOWS {
                    self.fail("a node has at most eight windows");
                } else {
                    grade.windows.push(PowerWindow::new(shape));
                    self.color.window = grade.windows.len() - 1;
                }
            }
        }

        // The list of windows.
        c.line((3.0, 712.0), (330.0, 712.0), 1.0, LINE);
        c.vline(330.0, 633.0, PAGE_BAR);
        if grade.windows.is_empty() {
            c.label(
                166.0,
                760.0,
                Align2::CENTER_CENTER,
                "No windows",
                15.0,
                GREY,
                false,
            );
        }
        let mut remove = None;
        for (i, w) in grade.windows.iter_mut().enumerate() {
            let y = 735.0 + i as f32 * 38.0;
            let row = c.rect(10.0, y - 16.0, 322.0, y + 16.0);
            let chosen = i == self.color.window;
            if chosen {
                c.p.rect_filled(row, 2.0, Color32::from_rgb(56, 56, 62));
            }
            if ui
                .interact(row, ui.id().with(("window_row", i)), Sense::click())
                .clicked()
            {
                self.color.window = i;
            }
            // On or off.
            let on = c.rect(18.0, y - 9.0, 36.0, y + 9.0);
            c.p.rect_stroke(on, 2.0, Stroke::new(1.0, GREY), StrokeKind::Inside);
            if w.enabled {
                c.p.rect_filled(on.shrink(3.0), 1.0, ACCENT);
            }
            if ui
                .interact(on, ui.id().with(("window_on", i)), Sense::click())
                .on_hover_text("Switch the window on or off")
                .clicked()
            {
                w.enabled = !w.enabled;
            }
            c.label(
                50.0,
                y,
                Align2::LEFT_CENTER,
                &format!("{} {}", w.shape.label(), i + 1),
                15.0,
                if chosen { theme::WHITE } else { TEXT },
                false,
            );
            if text_button(
                ui,
                c,
                &format!("window_inv{i}"),
                (190.0, y - 13.0, 262.0, y + 13.0),
                "Invert",
                w.invert,
            ) {
                w.invert = !w.invert;
            }
            if text_button(
                ui,
                c,
                &format!("window_del{i}"),
                (272.0, y - 13.0, 312.0, y + 13.0),
                "×",
                false,
            ) {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            grade.windows.remove(i);
            self.color.window = self.color.window.min(grade.windows.len().saturating_sub(1));
        }

        // The transform of the chosen window.
        let chosen = self.color.window;
        if let Some(w) = grade.windows.get_mut(chosen) {
            ui.push_id(("window_transform", chosen), |ui| {
                c.label(
                    360.0,
                    660.0,
                    Align2::LEFT_CENTER,
                    "Transform",
                    16.0,
                    TEXT,
                    true,
                );
                let tracking = self.window_tracking();
                if text_button(
                    ui,
                    c,
                    "window_track",
                    (860.0, 648.0, 1000.0, 676.0),
                    if tracking { "Tracking..." } else { "Track" },
                    tracking,
                ) && !tracking
                {
                    self.start_window_tracking(
                        ctx,
                        id,
                        node,
                        chosen,
                        media_path.clone(),
                        w.clone(),
                    );
                }
                let fields: [(&str, f32, f32, f32); 6] = [
                    ("Size X", w.size[0] * 100.0, 0.25, 40.0),
                    ("Size Y", w.size[1] * 100.0, 0.25, 50.0),
                    ("Pan", w.center[0] * 100.0, 0.25, 50.0),
                    ("Tilt", w.center[1] * 100.0, 0.25, 50.0),
                    ("Rotate", w.rotation, 0.5, 0.0),
                    ("Softness", w.softness * 100.0, 0.25, 20.0),
                ];
                for (f, (label, value, per, neutral)) in fields.into_iter().enumerate() {
                    let x0 = 380.0 + (f % 3) as f32 * 220.0;
                    let y = 730.0 + (f / 3) as f32 * 95.0;
                    c.label(
                        x0 + 75.0,
                        y - 30.0,
                        Align2::CENTER_CENTER,
                        label,
                        14.0,
                        LABEL,
                        false,
                    );
                    if let Some(v) = value_box(
                        ui,
                        c,
                        ("window_value", f),
                        (x0, y - 16.0, x0 + 150.0, y + 16.0),
                        value,
                        &format!("{value:.1}"),
                        per,
                        neutral,
                    ) {
                        match f {
                            0 => w.size[0] = (v / 100.0).clamp(0.005, 4.0),
                            1 => w.size[1] = (v / 100.0).clamp(0.005, 4.0),
                            2 => w.center[0] = (v / 100.0).clamp(-1.0, 2.0),
                            3 => w.center[1] = (v / 100.0).clamp(-1.0, 2.0),
                            4 => w.rotation = v.clamp(-360.0, 360.0),
                            _ => w.softness = (v / 100.0).clamp(0.0, 1.0),
                        }
                    }
                }
                c.label(
                    380.0,
                    935.0,
                    Align2::LEFT_CENTER,
                    "Drag the window in the viewer to move it, its handles to size and turn it",
                    14.0,
                    LABEL,
                    false,
                );
            });
        }
        self.commit_node(ctx, id, node, &before, grade);
    }

    /// Draws the node's windows over the picture at `shown`; with the Window palette open,
    /// a drag moves, sizes or turns the chosen one.
    pub(super) fn window_overlay(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv, shown: Rect) {
        let Some((id, node, before)) = self.edited_node() else {
            return;
        };
        if before.windows.is_empty() {
            return;
        }
        let editing = self.color.palette == Palette::Window && self.color.picker.is_none();
        let chosen = self.color.window.min(before.windows.len() - 1);
        let painter = c.p.with_clip_rect(shown);
        let (w, h) = (shown.width(), shown.height());
        // From a window's own frame (pixels of the shown picture) to the screen.
        let to_screen = |win: &PowerWindow, u: f32, v: f32| {
            let (s, co) = win.rotation.to_radians().sin_cos();
            pos2(
                shown.left() + win.center[0] * w + u * co - v * s,
                shown.top() + win.center[1] * h + u * s + v * co,
            )
        };
        for (i, win) in before.windows.iter().enumerate() {
            let lit = editing && i == chosen;
            let color = if !win.enabled {
                Color32::from_gray(110)
            } else if lit {
                KEY_LINE
            } else {
                Color32::from_gray(190)
            };
            let (rx, ry) = (win.size[0] * w / 2.0, win.size[1] * h / 2.0);
            let outline = |scale: f32| -> Vec<Pos2> {
                match win.shape {
                    WindowShape::Circle => (0..64)
                        .map(|k| {
                            let t = k as f32 / 64.0 * std::f32::consts::TAU;
                            to_screen(win, rx * scale * t.cos(), ry * scale * t.sin())
                        })
                        .collect(),
                    _ => [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                        .map(|(a, b)| to_screen(win, a * rx * scale, b * ry * scale))
                        .to_vec(),
                }
            };
            match win.shape {
                WindowShape::Gradient => {
                    let far = (w + h) * 2.0;
                    painter.line_segment(
                        [to_screen(win, -far, 0.0), to_screen(win, far, 0.0)],
                        Stroke::new(1.5, color),
                    );
                    for v in [-ry, ry] {
                        painter.add(egui::Shape::dashed_line(
                            &[to_screen(win, -far, v), to_screen(win, far, v)],
                            Stroke::new(1.0, color.gamma_multiply(0.6)),
                            6.0,
                            5.0,
                        ));
                    }
                    // An arrow towards what is graded.
                    painter.arrow(
                        to_screen(win, 0.0, 0.0),
                        to_screen(win, 0.0, -ry.max(20.0)) - to_screen(win, 0.0, 0.0),
                        Stroke::new(1.5, color),
                    );
                }
                _ => {
                    painter.add(egui::Shape::closed_line(
                        outline(1.0),
                        Stroke::new(1.5, color),
                    ));
                    let soft = win.softness / 2.0;
                    if soft > 0.0 {
                        for scale in [1.0 - soft, 1.0 + soft] {
                            let mut points = outline(scale.max(0.01));
                            if let Some(first) = points.first().copied() {
                                points.push(first);
                            }
                            painter.add(egui::Shape::dashed_line(
                                &points,
                                Stroke::new(1.0, color.gamma_multiply(0.6)),
                                6.0,
                                5.0,
                            ));
                        }
                    }
                }
            }
            if lit {
                painter.circle_filled(to_screen(win, 0.0, 0.0), 3.5, color);
                for (u, v) in [(rx, 0.0), (0.0, ry), (0.0, -ry - 18.0)] {
                    painter.circle_stroke(to_screen(win, u, v), 5.0, Stroke::new(1.5, color));
                }
            }
        }
        if !editing {
            return;
        }
        let Some(win) = before.windows.get(chosen).cloned() else {
            return;
        };
        let (rx, ry) = (win.size[0] * w / 2.0, win.size[1] * h / 2.0);
        let resp = ui
            .interact(
                shown,
                ui.id().with("window_overlay"),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Move);
        if resp.drag_started() {
            let at = resp.interact_pointer_pos().unwrap_or(shown.center());
            let near = |u: f32, v: f32| to_screen(&win, u, v).distance(at) < 12.0;
            self.color.window_drag = Some(if near(0.0, -ry - 18.0) {
                WindowDrag::Rotate
            } else if near(rx, 0.0) {
                WindowDrag::Width
            } else if near(0.0, ry) {
                WindowDrag::Height
            } else {
                WindowDrag::Move
            });
        }
        if !resp.dragged() {
            if !ctx.input(|i| i.pointer.any_down()) {
                self.color.window_drag = None;
            }
            return;
        }
        let (Some(kind), Some(at)) = (self.color.window_drag, resp.interact_pointer_pos()) else {
            return;
        };
        let mut grade = before.clone();
        let Some(edit) = grade.windows.get_mut(chosen) else {
            return;
        };
        let centre = to_screen(&win, 0.0, 0.0);
        let d = at - centre;
        let (s, co) = win.rotation.to_radians().sin_cos();
        // The pointer in the window's own frame.
        let (u, v) = (d.x * co + d.y * s, -d.x * s + d.y * co);
        match kind {
            WindowDrag::Move => {
                let delta = resp.drag_delta();
                edit.center[0] += delta.x / w.max(1.0);
                edit.center[1] += delta.y / h.max(1.0);
            }
            WindowDrag::Width => edit.size[0] = (2.0 * u.abs() / w.max(1.0)).max(0.005),
            WindowDrag::Height => edit.size[1] = (2.0 * v.abs() / h.max(1.0)).max(0.005),
            WindowDrag::Rotate => edit.rotation = d.y.atan2(d.x).to_degrees() + 90.0,
        }
        self.commit_node(ctx, id, node, &before, grade);
    }
}
