//! The render settings of the Deliver page: the preset, the file name and place, and the Video,
//! Audio and File groups. Every control changes what is rendered (see `picasso_export::Options`).

#[cfg(target_arch = "wasm32")]
use crate::{app::StudioApp, chrome::Screen};
#[cfg(target_arch = "wasm32")]
use egui::{Context, Ui};

#[cfg(target_arch = "wasm32")]
impl StudioApp {
    pub(crate) fn render_form(
        &mut self,
        ui: &mut Ui,
        _ctx: &Context,
        s: &Screen,
        top: f32,
        bottom: f32,
    ) {
        let area = s.r(
            crate::deliver::LEFT,
            top + crate::deliver::BAR,
            crate::deliver::LEFT_RIGHT,
            bottom,
        );
        crate::i18n::paint_text(&ui.painter(),
            area.center(),
            egui::Align2::CENTER_CENTER,
            "Rendering needs the desktop app",
            crate::theme::regular(22.0),
            crate::theme::EMPTY_TITLE,
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use egui::{
        pos2, vec2, Align2, Color32, Context, Painter, Rect, Response, Sense, Stroke, StrokeKind,
        Ui, UiBuilder,
    };
    use picasso_export::{
        AudioCodec, Hardware, Preset, Profile, Quality, Rate, Speed, DNXHR_PROFILES,
    };

    use crate::app::StudioApp;
    use crate::chrome::Screen;
    use crate::deliver::{Tab, BAR, LEFT, LEFT_RIGHT, RESOLUTIONS};
    use crate::icons::{paint, Icon};
    use crate::platform;
    use crate::theme::{self, regular, semibold};

    /// How wide the form has to be for labels beside their controls: the widest row (the
    /// location's field and its Browse button) ends 638 points from the form's left edge.
    pub(super) const SIDE_BY_SIDE: f32 = 650.0;
    /// The margin of the stacked form on each side.
    const MARGIN: f32 = 14.0;
    /// The height a label above its control takes in the stacked form.
    const LABEL_BAND: f32 = 26.0;

    /// Where the form puts its labels and controls. Wide enough, a label sits at the left of its
    /// control. Narrower, the form stacks: the label goes above the control, the controls take
    /// the width there is and what would not fit beside another goes under it, so that nothing
    /// runs into a neighbour or past the column however narrow a divider makes it.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) struct Geo {
        pub stacked: bool,
        pub label_right: f32,
        pub control_left: f32,
        /// The width of a wide control (a text field, a drop-down).
        pub wide: f32,
        /// The width the rows have.
        pub width: f32,
    }

    impl Geo {
        pub(super) fn for_width(width: f32) -> Self {
            if width >= SIDE_BY_SIDE {
                Geo {
                    stacked: false,
                    label_right: 205.0,
                    control_left: 220.0,
                    wide: 292.0,
                    width,
                }
            } else {
                let control_left = MARGIN + 8.0;
                Geo {
                    stacked: true,
                    label_right: 0.0,
                    control_left,
                    wide: (width - control_left - MARGIN).clamp(110.0, 292.0),
                    width,
                }
            }
        }

        /// The room a control starting at `x` points from the form's left edge has.
        fn room(&self, x: f32) -> f32 {
            (self.width - x - MARGIN).max(60.0)
        }
    }

    thread_local! {
        /// The geometry of the form being drawn (set by `render_form_in` for each frame).
        static GEO: std::cell::Cell<Geo> = const { std::cell::Cell::new(Geo {
            stacked: false,
            label_right: 205.0,
            control_left: 220.0,
            wide: 292.0,
            width: 685.0,
        }) };
    }

    fn geo() -> Geo {
        GEO.with(std::cell::Cell::get)
    }

    /// The extra height a row with a label needs when the label is above its control.
    fn stack() -> f32 {
        if geo().stacked {
            LABEL_BAND
        } else {
            0.0
        }
    }

    /// `text` cut short with an ellipsis so that it fits `room` points (an estimate: the form's
    /// 18 point text is about 9 points a letter).
    fn ellipsis(text: &str, room: f32) -> String {
        // In the language chosen, so that it is the translation that is cut short.
        let text = crate::i18n::translate_ui(crate::i18n::current(), text.trim()).unwrap_or(text);
        let fits = (room / 9.0).floor().max(1.0) as usize;
        if text.chars().count() <= fits {
            text.to_string()
        } else {
            let mut cut: String = text.chars().take(fits.saturating_sub(1)).collect();
            cut.push('…');
            cut
        }
    }

    /// `text` for a place `room` points wide to the row's right edge: whole in the usual form,
    /// cut short with an ellipsis in the stacked one.
    fn fitted(text: &str, room: f32) -> String {
        if geo().stacked {
            ellipsis(text, room - MARGIN)
        } else {
            text.to_string()
        }
    }

    const FIELD: Color32 = Color32::from_rgb(34, 33, 38);
    const FIELD_LINE: Color32 = Color32::from_rgb(12, 12, 15);
    const TEXT: Color32 = Color32::from_rgb(244, 240, 236);
    const DIM: Color32 = Color32::from_rgb(130, 130, 136);
    const OFF: Color32 = Color32::from_rgb(86, 86, 92);

    /// The formats and the presets (codecs) each holds.
    const FORMATS: [(&str, &[Preset]); 7] = [
        ("MP4", &[Preset::H264Web, Preset::H265, Preset::Av1]),
        ("QuickTime", &[Preset::ProRes422, Preset::DnxHr]),
        ("WebM", &[Preset::Vp9]),
        ("Matroska", &[Preset::Ffv1]),
        ("WAV", &[Preset::Wav]),
        ("MP3", &[Preset::Mp3]),
        ("Image Sequence", &[Preset::PngSequence]),
    ];

    fn codec_name(p: Preset) -> &'static str {
        match p {
            Preset::H264Web => "H.264",
            Preset::H265 => "H.265",
            Preset::Av1 => "AV1",
            Preset::Vp9 => "VP9",
            Preset::ProRes422 => "Apple ProRes",
            Preset::DnxHr => "DNxHR",
            Preset::Ffv1 => "FFV1",
            Preset::Wav => "Linear PCM",
            Preset::Mp3 => "MP3",
            Preset::PngSequence => "PNG",
        }
    }

    /// Whether a preset makes a video file (not sound only, not a folder of pictures).
    fn video_file(preset: Preset) -> bool {
        !matches!(preset, Preset::Wav | Preset::Mp3 | Preset::PngSequence)
    }

    fn enabled_text(on: bool) -> Color32 {
        if on {
            TEXT
        } else {
            OFF
        }
    }

    fn label(p: &Painter, row: Rect, text: &str, on: bool) {
        let g = geo();
        let color = if on {
            Color32::from_rgb(160, 160, 166)
        } else {
            OFF
        };
        if g.stacked {
            // Above its control, at the control's left edge.
            crate::i18n::paint_text(
                &p,
                pos2(row.left() + g.control_left, row.top() + 12.0),
                Align2::LEFT_CENTER,
                ellipsis(text, g.room(g.control_left)),
                regular(18.0),
                color,
            );
        } else {
            crate::i18n::paint_text(
                &p,
                pos2(row.left() + g.label_right, row.center().y),
                Align2::RIGHT_CENTER,
                text,
                regular(18.0),
                color,
            );
        }
    }

    /// The field of a row, `width` wide at most: in the stacked form what is left of the row.
    fn control(row: Rect, width: f32) -> Rect {
        let g = geo();
        Rect::from_min_size(
            pos2(row.left() + g.control_left, row.center().y - 15.0),
            vec2(width.min(g.room(g.control_left)), 30.0),
        )
    }

    fn rule(p: &Painter, row: Rect) {
        p.hline(
            row.left() + 14.0..=row.right() - 6.0,
            row.center().y,
            Stroke::new(1.0, Color32::from_rgb(20, 20, 24)),
        );
    }

    /// A drop-down: the label, the current choice, and a menu of `options` (name, usable).
    /// Returns the choice made.
    #[allow(clippy::too_many_arguments)]
    fn combo(
        ui: &mut Ui,
        row: Rect,
        id: &str,
        name: &str,
        current: &str,
        options: &[(String, bool)],
        width: f32,
        on: bool,
    ) -> Option<usize> {
        let p = ui.painter().clone();
        label(&p, row, name, on);
        let rect = control(row, width);
        let response = ui.interact(
            rect,
            ui.id().with(("combo", id)),
            if on { Sense::click() } else { Sense::hover() },
        );
        let fill = if response.hovered() && on {
            Color32::from_rgb(44, 43, 49)
        } else {
            FIELD
        };
        p.rect_filled(rect, 2.0, fill);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, FIELD_LINE), StrokeKind::Inside);
        crate::i18n::paint_text(&p,
            rect.left_center() + vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            if geo().stacked {
                ellipsis(current, rect.width() - 44.0)
            } else {
                current.to_string()
            },
            regular(18.0),
            enabled_text(on),
        );
        paint(
            &p,
            rect.right_center() - vec2(14.0, 0.0),
            Icon::Chevron,
            if on { theme::ICON } else { OFF },
        );
        let mut picked = None;
        if on {
            egui::Popup::menu(&response).show(|ui| {
                ui.set_min_width(width.max(160.0));
                for (i, (text, usable)) in options.iter().enumerate() {
                    ui.add_enabled_ui(*usable, |ui| {
                        if ui
                            .selectable_label(text == current, text.as_str())
                            .clicked()
                        {
                            picked = Some(i);
                        }
                    });
                }
            });
        }
        picked
    }

    fn box_at(ui: &mut Ui, id: &str, rect: Rect, on: bool, checked: bool) -> Response {
        let p = ui.painter().clone();
        let r = ui.interact(
            rect,
            ui.id().with(("check", id)),
            if on { Sense::click() } else { Sense::hover() },
        );
        p.rect_filled(
            rect,
            3.0,
            if on {
                FIELD
            } else {
                Color32::from_rgb(30, 29, 34)
            },
        );
        p.rect_stroke(rect, 3.0, Stroke::new(1.0, FIELD_LINE), StrokeKind::Inside);
        if checked {
            let c = rect.center();
            let color = if on { theme::WHITE } else { OFF };
            p.line_segment(
                [c + vec2(-5.0, 0.0), c + vec2(-1.5, 4.0)],
                Stroke::new(2.0, color),
            );
            p.line_segment(
                [c + vec2(-1.5, 4.0), c + vec2(6.0, -5.0)],
                Stroke::new(2.0, color),
            );
        }
        r
    }

    /// A check box with its text; returns the new value when it was clicked.
    fn check(
        ui: &mut Ui,
        row: Rect,
        id: &str,
        x: f32,
        text: &str,
        value: bool,
        on: bool,
    ) -> Option<bool> {
        let rect = Rect::from_center_size(
            pos2(row.left() + x + 12.0, row.center().y),
            vec2(24.0, 24.0),
        );
        let r = box_at(ui, id, rect, on, value);
        let shown = fitted(text, row.right() - (rect.right() + 14.0));
        crate::i18n::paint_text(&ui.painter(),
            pos2(rect.right() + 14.0, row.center().y),
            Align2::LEFT_CENTER,
            shown,
            regular(18.0),
            enabled_text(on),
        );
        let hit = Rect::from_min_max(rect.min, pos2(rect.right() + 14.0 + 220.0, rect.bottom()));
        let click = ui.interact(
            hit,
            ui.id().with(("checkhit", id)),
            if on { Sense::click() } else { Sense::hover() },
        );
        (on && (r.clicked() || click.clicked())).then_some(!value)
    }

    /// A radio button with its text; true when it was clicked.
    fn radio(ui: &mut Ui, row: Rect, id: &str, x: f32, text: &str, value: bool, on: bool) -> bool {
        let p = ui.painter().clone();
        let c = pos2(row.left() + x + 12.0, row.center().y);
        let hit = Rect::from_min_max(
            pos2(c.x - 14.0, row.top() + 2.0),
            pos2(c.x + 28.0 + 7.0 * text.len() as f32, row.bottom() - 2.0),
        );
        let r = ui.interact(
            hit,
            ui.id().with(("radio", id)),
            if on { Sense::click() } else { Sense::hover() },
        );
        p.circle_filled(
            c,
            11.0,
            if on {
                FIELD
            } else {
                Color32::from_rgb(30, 29, 34)
            },
        );
        p.circle_stroke(c, 11.0, Stroke::new(1.0, FIELD_LINE));
        if value {
            p.circle_filled(c, 4.5, if on { theme::WHITE } else { OFF });
        }
        crate::i18n::paint_text(&p,
            pos2(c.x + 22.0, c.y),
            Align2::LEFT_CENTER,
            fitted(text, row.right() - (c.x + 22.0)),
            regular(18.0),
            enabled_text(on),
        );
        on && r.clicked()
    }

    fn number(
        ui: &mut Ui,
        rect: Rect,
        value: &mut u32,
        range: std::ops::RangeInclusive<u32>,
        on: bool,
    ) -> bool {
        let p = ui.painter().clone();
        p.rect_filled(rect, 2.0, FIELD);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, FIELD_LINE), StrokeKind::Inside);
        ui.add_enabled_ui(on, |ui| {
            ui.put(
                rect.shrink2(vec2(2.0, 3.0)),
                egui::DragValue::new(value).range(range).speed(1.0),
            )
            .changed()
        })
        .inner
    }

    fn text_field(ui: &mut Ui, rect: Rect, text: &mut String, hint: &str) {
        ui.put(
            rect,
            egui::TextEdit::singleline(text)
                .font(regular(18.0))
                .hint_text(hint)
                .margin(vec2(10.0, 6.0))
                .desired_width(rect.width()),
        );
    }

    impl StudioApp {
        pub(crate) fn render_form(
            &mut self,
            ui: &mut Ui,
            ctx: &Context,
            s: &Screen,
            top: f32,
            bottom: f32,
        ) {
            // However narrow a divider makes its column, the form fits it: see `Geo`.
            let pane = s.r(LEFT, top + BAR, LEFT_RIGHT - 1.0, bottom);
            self.render_form_in(ui, ctx, pane);
        }

        /// The form in `pane`.
        fn render_form_in(&mut self, ui: &mut Ui, ctx: &Context, pane: Rect) {
            let painter = ui.painter().clone();
            // The rows of the settings are 14 points narrower than the pane (the scroll bar).
            let g = Geo::for_width(pane.width() - 14.0);
            GEO.with(|cell| cell.set(g));

            // The preset, always in view.
            let preset_row = Rect::from_min_size(pane.min, vec2(pane.width(), 96.0));
            let box_rect = if g.stacked {
                crate::i18n::paint_text(
                    &painter,
                    pos2(preset_row.left() + MARGIN, preset_row.top() + 16.0),
                    Align2::LEFT_CENTER,
                    "Preset",
                    regular(19.0),
                    Color32::from_rgb(160, 160, 166),
                );
                Rect::from_min_size(
                    pos2(preset_row.left() + MARGIN, preset_row.top() + 34.0),
                    vec2((pane.width() - 2.0 * MARGIN).max(120.0), 54.0),
                )
            } else {
                crate::i18n::paint_text(
                    &painter,
                    pos2(preset_row.left() + 88.0, preset_row.center().y),
                    Align2::RIGHT_CENTER,
                    "Preset",
                    regular(19.0),
                    Color32::from_rgb(160, 160, 166),
                );
                Rect::from_min_size(
                    pos2(preset_row.left() + 102.0, preset_row.center().y - 27.0),
                    vec2(392.0, 54.0),
                )
            };
            let r = ui.interact(box_rect, ui.id().with("preset_box"), Sense::click());
            painter.rect_filled(
                box_rect,
                4.0,
                if r.hovered() {
                    Color32::from_rgb(40, 39, 45)
                } else {
                    FIELD
                },
            );
            painter.rect_stroke(
                box_rect,
                4.0,
                Stroke::new(1.0, FIELD_LINE),
                StrokeKind::Inside,
            );
            crate::render_presets::paint_badge(
                &painter,
                box_rect.left_center() + vec2(32.0, 0.0),
                self.preset_badge(),
            );
            crate::i18n::paint_text(&painter,
                box_rect.left_center() + vec2(72.0, 0.0),
                Align2::LEFT_CENTER,
                if g.stacked {
                    ellipsis(&self.deliver_ui.preset_name, box_rect.width() - 72.0 - 40.0)
                } else {
                    self.deliver_ui.preset_name.clone()
                },
                semibold(20.0),
                TEXT,
            );
            paint(
                &painter,
                box_rect.right_center() - vec2(16.0, 0.0),
                Icon::Chevron,
                theme::ICON,
            );
            egui::Popup::menu(&r)
                .width(box_rect.width())
                .show(|ui| self.preset_menu(ui));
            painter.hline(
                pane.x_range(),
                preset_row.bottom(),
                Stroke::new(1.0, Color32::from_rgb(14, 14, 18)),
            );

            // The footer: the size and the button. Narrow, the size is above the button.
            let footer_height = if g.stacked { 122.0 } else { 74.0 };
            let footer =
                Rect::from_min_max(pos2(pane.left(), pane.bottom() - footer_height), pane.max);
            painter.rect_filled(footer, 0.0, theme::POOL_HEADER_BG);
            painter.hline(
                pane.x_range(),
                footer.top(),
                Stroke::new(1.0, Color32::from_rgb(14, 14, 18)),
            );
            let size_y = if g.stacked {
                footer.top() + 30.0
            } else {
                footer.center().y
            };
            crate::i18n::paint_text(&painter,
                pos2(footer.left() + if g.stacked { MARGIN } else { 40.0 }, size_y),
                Align2::LEFT_CENTER,
                "Estimated File Size",
                regular(18.0),
                Color32::from_rgb(160, 160, 166),
            );
            let mb = self.estimated_mb();
            let (size_at, size_align) = if g.stacked {
                (pos2(footer.right() - MARGIN, size_y), Align2::RIGHT_CENTER)
            } else {
                (pos2(footer.left() + 232.0, size_y), Align2::LEFT_CENTER)
            };
            crate::i18n::paint_text(&painter,
                size_at,
                size_align,
                if mb >= 1024.0 {
                    format!("{:.2} GB", mb / 1024.0)
                } else {
                    format!("{mb:.2} MB")
                },
                regular(19.0),
                theme::WHITE,
            );
            let can_add = self.project().duration() > 0;
            let button = if g.stacked {
                Rect::from_min_size(
                    pos2(footer.left() + MARGIN, footer.top() + 66.0),
                    vec2((footer.width() - 2.0 * MARGIN).max(100.0), 36.0),
                )
            } else {
                Rect::from_min_size(
                    pos2(footer.left() + 363.0, footer.center().y - 18.0),
                    vec2(262.0, 36.0),
                )
            };
            let r = ui.interact(
                button,
                ui.id().with("add_to_queue"),
                if can_add {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            painter.rect_filled(
                button,
                18.0,
                if can_add && r.hovered() {
                    Color32::from_rgb(52, 51, 58)
                } else {
                    Color32::from_rgb(38, 37, 43)
                },
            );
            painter.rect_stroke(
                button,
                18.0,
                Stroke::new(1.0, Color32::from_rgb(58, 57, 63)),
                StrokeKind::Inside,
            );
            crate::i18n::paint_text(&painter,
                button.center(),
                Align2::CENTER_CENTER,
                "Add to Render Queue",
                regular(18.0),
                if can_add { TEXT } else { OFF },
            );
            if can_add
                && r.on_hover_text("Add a render of these settings to the queue")
                    .clicked()
            {
                self.add_to_queue(ctx);
            }

            // The settings, which scroll.
            let scroll = Rect::from_min_max(
                pos2(pane.left(), preset_row.bottom() + 1.0),
                pos2(pane.right(), footer.top()),
            );
            ui.scope_builder(UiBuilder::new().max_rect(scroll), |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                egui::ScrollArea::vertical()
                    .id_salt("render_settings")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = scroll.width() - 14.0;
                        // The rows follow one another from here, whatever widgets placed at
                        // a rectangle (a text field, a number) did to the layout's cursor.
                        let origin = ui.cursor().min;
                        let mut used = 0.0;
                        let mut row = |ui: &mut Ui, height: f32| {
                            let rect = Rect::from_min_size(
                                origin + vec2(0.0, used),
                                vec2(width, height),
                            );
                            used += height;
                            ui.expand_to_include_rect(rect);
                            rect
                        };
                        self.form_top(ui, ctx, &mut row);
                        match self.deliver_ui.tab {
                            Tab::Video => self.form_video(ui, &mut row),
                            Tab::Audio => self.form_audio(ui, &mut row),
                            Tab::File => self.form_file(ui, &mut row),
                        }
                        row(ui, 16.0);
                    });
            });
        }

        /// The file name and place, how it is rendered, and the three tabs.
        fn form_top(
            &mut self,
            ui: &mut Ui,
            ctx: &Context,
            row: &mut dyn FnMut(&mut Ui, f32) -> Rect,
        ) {
            let p = ui.painter().clone();
            row(ui, 10.0);
            // File name.
            let r = row(ui, 46.0 + stack());
            label(&p, r, "File Name", true);
            let field = control(r, geo().wide);
            let mut name = std::mem::take(&mut self.deliver_ui.file_name);
            text_field(
                ui,
                Rect::from_min_max(field.min, pos2(field.right() - 30.0, field.bottom())),
                &mut name,
                "Untitled",
            );
            self.deliver_ui.file_name = name;
            let plus =
                Rect::from_min_size(pos2(field.right() - 30.0, field.top()), vec2(30.0, 30.0));
            let pr = ui.interact(plus, ui.id().with("name_plus"), Sense::click());
            let c = if pr.hovered() {
                theme::ICON_HOVER
            } else {
                theme::ICON
            };
            p.line_segment(
                [
                    plus.center() - vec2(6.0, 0.0),
                    plus.center() + vec2(6.0, 0.0),
                ],
                Stroke::new(1.5, c),
            );
            p.line_segment(
                [
                    plus.center() - vec2(0.0, 6.0),
                    plus.center() + vec2(0.0, 6.0),
                ],
                Stroke::new(1.5, c),
            );
            let project_name = self.project().name.clone();
            let mut insert = None;
            egui::Popup::menu(&pr).show(|ui| {
                if ui.button("Project name").clicked() {
                    insert = Some(project_name.clone());
                }
                if ui.button("Preset name").clicked() {
                    insert = Some("Custom Export".to_string());
                }
            });
            if let Some(text) = insert {
                self.deliver_ui.file_name.push_str(&text);
            }
            // Location.
            // Narrow, the Browse button goes under the field.
            let below = geo().stacked;
            let r = row(ui, 46.0 + stack() + if below { 44.0 } else { 0.0 });
            label(&p, r, "Location", true);
            let field = control(r, geo().wide);
            let mut location = std::mem::take(&mut self.deliver_ui.location);
            text_field(ui, field, &mut location, "Choose a folder");
            self.deliver_ui.location = location;
            let browse = if below {
                Rect::from_min_size(pos2(field.left(), field.bottom() + 10.0), vec2(110.0, 32.0))
            } else {
                Rect::from_min_size(
                    pos2(field.right() + 16.0, field.top() - 1.0),
                    vec2(110.0, 32.0),
                )
            };
            let br = ui.interact(browse, ui.id().with("browse"), Sense::click());
            p.rect_filled(
                browse,
                16.0,
                if br.hovered() {
                    Color32::from_rgb(52, 51, 58)
                } else {
                    Color32::from_rgb(38, 37, 43)
                },
            );
            p.rect_stroke(
                browse,
                16.0,
                Stroke::new(1.0, Color32::from_rgb(58, 57, 63)),
                StrokeKind::Inside,
            );
            crate::i18n::paint_text(&p,
                browse.center(),
                Align2::CENTER_CENTER,
                "Browse",
                regular(18.0),
                TEXT,
            );
            if br.clicked() {
                self.deliver_ui.browsing = true;
                platform::pick_folder(self.inbox.clone(), ctx.clone());
            }
            let r = row(ui, 12.0);
            rule(&p, r);
            // Render: one file, or one for each clip.
            // Narrow, the second choice goes under the first.
            let wrap = geo().stacked && geo().width < 360.0;
            let r = row(ui, 62.0 + stack() + if wrap { 32.0 } else { 0.0 });
            label(&p, r, "Render", true);
            let individual = self.deliver_ui.individual;
            let (first, second) = if wrap {
                (r.translate(vec2(0.0, -16.0)), r.translate(vec2(0.0, 16.0)))
            } else {
                (r, r)
            };
            if radio(
                ui,
                first,
                "single",
                geo().control_left - 8.0,
                "Single clip",
                !individual,
                true,
            ) {
                self.deliver_ui.individual = false;
            }
            if radio(
                ui,
                second,
                "individual",
                geo().control_left + if wrap { -8.0 } else { 140.0 },
                "Individual clips",
                individual,
                true,
            ) {
                self.deliver_ui.individual = true;
            }
            let r = row(ui, 6.0);
            rule(&p, r);
            // The tabs.
            let r = row(ui, 56.0);
            let tabs = if geo().stacked {
                Rect::from_min_size(
                    pos2(r.left() + MARGIN, r.center().y - 17.0),
                    vec2((r.width() - 2.0 * MARGIN).max(90.0), 34.0),
                )
            } else {
                Rect::from_min_size(
                    pos2(r.left() + 86.0, r.center().y - 17.0),
                    vec2(462.0, 34.0),
                )
            };
            let seg_width = tabs.width() / 3.0;
            p.rect_filled(tabs, 2.0, FIELD);
            p.rect_stroke(tabs, 2.0, Stroke::new(1.0, FIELD_LINE), StrokeKind::Inside);
            for (n, (tab, name)) in [
                (Tab::Video, "Video"),
                (Tab::Audio, "Audio"),
                (Tab::File, "File"),
            ]
            .into_iter()
            .enumerate()
            {
                let seg = Rect::from_min_size(
                    tabs.min + vec2(n as f32 * seg_width, 0.0),
                    vec2(seg_width, 34.0),
                );
                let tr = ui.interact(seg, ui.id().with(("tab", name)), Sense::click());
                if tr.clicked() {
                    self.deliver_ui.tab = tab;
                }
                let on = self.deliver_ui.tab == tab;
                if on {
                    p.rect_filled(seg, 2.0, Color32::from_rgb(20, 20, 23));
                }
                crate::i18n::paint_text(&p,
                    seg.center(),
                    Align2::CENTER_CENTER,
                    name,
                    regular(19.0),
                    if on {
                        theme::WHITE
                    } else {
                        Color32::from_rgb(170, 170, 176)
                    },
                );
            }
        }

        fn form_video(&mut self, ui: &mut Ui, row: &mut dyn FnMut(&mut Ui, f32) -> Rect) {
            let p = ui.painter().clone();
            let preset = self.deliver.preset;
            let is_video_preset = !preset.is_audio_only();
            let is_file = video_file(preset);

            let r = row(ui, 52.0);
            if let Some(v) = check(
                ui,
                r,
                "export_video",
                14.0,
                "Export Video",
                self.deliver.options.video && is_video_preset,
                is_video_preset && is_file,
            ) {
                self.deliver.options.video = v;
            }
            let r = row(ui, 12.0);
            rule(&p, r);

            // Format and codec.
            let format_now = FORMATS
                .iter()
                .position(|(_, ps)| ps.contains(&preset))
                .unwrap_or(0);
            let r = row(ui, 44.0 + stack());
            let formats: Vec<(String, bool)> = FORMATS
                .iter()
                .map(|(n, ps)| (n.to_string(), ps.iter().any(|p| p.available())))
                .collect();
            if let Some(i) = combo(
                ui,
                r,
                "format",
                "Format",
                FORMATS[format_now].0,
                &formats,
                geo().wide,
                true,
            ) {
                if let Some(first) = FORMATS[i].1.iter().find(|p| p.available()) {
                    self.deliver.preset = *first;
                }
            }
            let r = row(ui, 44.0 + stack());
            let codecs: Vec<(String, bool)> = FORMATS[format_now]
                .1
                .iter()
                .map(|p| (codec_name(*p).to_string(), p.available()))
                .collect();
            if let Some(i) = combo(
                ui,
                r,
                "codec",
                "Codec",
                codec_name(preset),
                &codecs,
                geo().wide,
                true,
            ) {
                if let Some(p) = FORMATS[format_now].1.get(i) {
                    self.deliver.preset = *p;
                }
            }
            // The processor's encoder or a graphics card's: the ones this FFmpeg has, those that did
            // not open on this computer greyed out.
            let r = row(ui, 44.0 + stack());
            let choices = preset.hardware_choices();
            // A codec without a card's encoder (ProRes ...) shows Native but keeps the choice
            // for when an H.264, H.265 or AV1 codec is chosen again.
            let hardware = if choices.contains(&self.deliver.options.hardware) {
                self.deliver.options.hardware
            } else {
                if choices.len() > 1 {
                    self.deliver.options.hardware = Hardware::Native;
                }
                Hardware::Native
            };
            let list: Vec<(String, bool)> = choices
                .iter()
                .map(|h| {
                    let works = picasso_export::hardware_works(preset, *h) != Some(false);
                    (h.label().to_string(), works)
                })
                .collect();
            if let Some(i) = combo(
                ui,
                r,
                "encoder",
                "Encoder",
                hardware.label(),
                &list,
                geo().wide,
                is_file && choices.len() > 1,
            ) {
                if let Some(h) = choices.get(i) {
                    self.deliver.options.hardware = *h;
                    // The cards make no 10-bit H.264.
                    if *h != Hardware::Native && preset == Preset::H264Web {
                        self.deliver.options.ten_bit = false;
                    }
                }
            }
            let r = row(ui, 44.0);
            let mp4 = matches!(preset.extension(), "mp4" | "mov");
            if let Some(v) = check(
                ui,
                r,
                "network",
                geo().control_left - 8.0,
                "Network Optimization",
                self.deliver.options.faststart && mp4,
                is_file && mp4,
            ) {
                self.deliver.options.faststart = v;
            }
            let r = row(ui, 14.0);
            rule(&p, r);

            // Resolution and frame rate.
            let r = row(ui, 44.0 + stack());
            let mut names: Vec<(String, bool)> = vec![("Timeline Resolution".to_string(), true)];
            names.extend(RESOLUTIONS.iter().map(|(n, _, _)| (n.to_string(), true)));
            names.push(("Custom".to_string(), true));
            let mode = self.deliver_ui.res_mode.min(names.len() - 1);
            if let Some(i) = combo(
                ui,
                r,
                "resolution",
                "Resolution",
                &names[mode].0,
                &names,
                geo().wide,
                is_video_preset,
            ) {
                self.deliver_ui.res_mode = i;
                if i == names.len() - 1 {
                    let (w, h) = (self.project().width, self.project().height);
                    self.deliver_ui.custom = (w, h);
                }
            }
            let custom = self.deliver_ui.res_mode == names.len() - 1;
            let (w, h) = match self.deliver_ui.res_mode {
                0 => (self.project().width, self.project().height),
                n if n <= RESOLUTIONS.len() => (RESOLUTIONS[n - 1].1, RESOLUTIONS[n - 1].2),
                _ => self.deliver_ui.custom,
            };
            let r = row(ui, 44.0);
            let (mut cw, mut ch) = (w, h);
            let left = pos2(r.left() + geo().control_left, r.center().y - 15.0);
            // The two numbers are narrower where the column is.
            let nw = if geo().stacked { 84.0 } else { 100.0 };
            let changed_w = number(
                ui,
                Rect::from_min_size(left, vec2(nw, 30.0)),
                &mut cw,
                16..=16_384,
                custom && !self.deliver_ui.vertical,
            );
            crate::i18n::paint_text(&p,
                pos2(left.x + nw + 26.0, r.center().y),
                Align2::CENTER_CENTER,
                "x",
                regular(18.0),
                DIM,
            );
            let changed_h = number(
                ui,
                Rect::from_min_size(left + vec2(nw + 52.0, 0.0), vec2(nw, 30.0)),
                &mut ch,
                16..=16_384,
                custom,
            );
            if custom && (changed_w || changed_h) {
                self.deliver_ui.custom = (cw, ch);
            }
            let r = row(ui, 44.0);
            if let Some(v) = check(
                ui,
                r,
                "vertical",
                geo().control_left - 8.0,
                "Use vertical resolution",
                self.deliver_ui.vertical,
                custom,
            ) {
                self.deliver_ui.vertical = v;
            }
            let r = row(ui, 44.0 + stack());
            let mut rates: Vec<(String, bool)> = vec![("Timeline Frame Rate".to_string(), true)];
            rates.extend(
                picasso_core::Fps::PRESETS
                    .iter()
                    .map(|f| (fps_label(*f), true)),
            );
            let fps_now = self.deliver_ui.fps_choice.min(rates.len() - 1);
            if let Some(i) = combo(
                ui,
                r,
                "fps",
                "Frame rate",
                &rates[fps_now].0,
                &rates,
                geo().wide,
                is_video_preset,
            ) {
                self.deliver_ui.fps_choice = i;
            }
            let r = row(ui, 40.0);
            let shown = match fps_now {
                0 => self.project().fps,
                n => picasso_core::Fps::PRESETS
                    .get(n - 1)
                    .copied()
                    .unwrap_or(self.project().fps),
            };
            crate::i18n::paint_text(&p,
                pos2(r.left() + geo().control_left + 4.0, r.center().y),
                Align2::LEFT_CENTER,
                format!("{} frames per second", fps_label(shown)),
                regular(18.0),
                Color32::from_rgb(160, 160, 166),
            );
            let r = row(ui, 40.0);
            if let Some(v) = check(
                ui,
                r,
                "chapters",
                geo().control_left - 8.0,
                "Chapters from Markers",
                self.deliver.options.chapters,
                is_file,
            ) {
                self.deliver.options.chapters = v;
            }
            let r = row(ui, 14.0);
            rule(&p, r);

            // The encoder's settings.
            let encodes = matches!(
                preset,
                Preset::H264Web | Preset::H265 | Preset::Av1 | Preset::Vp9
            );
            let on_card = self.deliver.options.hardware != Hardware::Native;
            let ten = self.deliver.options.ten_bit;
            let r = row(ui, 44.0 + stack());
            // The profiles of the codec; for H.264, H.265, AV1 and VP9 the last is the 10-bit one.
            let profiles: Vec<(String, bool)> = match preset {
                Preset::ProRes422 => [
                    "ProRes 422 Proxy",
                    "ProRes 422 LT",
                    "ProRes 422",
                    "ProRes 422 HQ",
                ]
                .iter()
                .map(|n| (n.to_string(), true))
                .collect(),
                Preset::DnxHr => DNXHR_PROFILES
                    .iter()
                    .map(|(n, _)| (n.to_string(), true))
                    .collect(),
                Preset::H264Web => Profile::ALL
                    .iter()
                    .map(|p| (p.label().to_string(), true))
                    .chain([("High 10".to_string(), !on_card)])
                    .collect(),
                Preset::H265 => vec![("Main".into(), true), ("Main 10".into(), true)],
                Preset::Av1 => vec![("Main, 8-bit".into(), true), ("Main, 10-bit".into(), true)],
                Preset::Vp9 => vec![
                    ("Profile 0, 8-bit".into(), true),
                    ("Profile 2, 10-bit".into(), true),
                ],
                Preset::Ffv1 => vec![("Level 3, 10-bit 4:4:4".into(), true)],
                _ => vec![("Auto".into(), true)],
            };
            let current = match preset {
                Preset::ProRes422 => usize::from(self.deliver.options.prores),
                Preset::DnxHr => usize::from(self.deliver.options.dnxhr),
                Preset::H264Web if ten => Profile::ALL.len(),
                Preset::H264Web => Profile::ALL
                    .iter()
                    .position(|p| *p == self.deliver.options.profile)
                    .unwrap_or(0),
                Preset::H265 | Preset::Av1 | Preset::Vp9 => usize::from(ten),
                _ => 0,
            }
            .min(profiles.len().saturating_sub(1));
            let shown = profiles.get(current).map_or("Auto", |(n, _)| n.as_str());
            if let Some(i) = combo(
                ui,
                r,
                "profile",
                "Encoding Profile",
                shown,
                &profiles,
                240.0,
                is_file && profiles.len() > 1,
            ) {
                let o = &mut self.deliver.options;
                match preset {
                    Preset::ProRes422 => o.prores = i as u8,
                    Preset::DnxHr => o.dnxhr = i as u8,
                    Preset::H264Web => match Profile::ALL.get(i) {
                        Some(pr) => {
                            o.profile = *pr;
                            o.ten_bit = false;
                        }
                        None => {
                            o.profile = Profile::Auto;
                            o.ten_bit = true;
                        }
                    },
                    Preset::H265 | Preset::Av1 | Preset::Vp9 => o.ten_bit = i == 1,
                    _ => {}
                }
            }
            // Key frames.
            let r = row(ui, 34.0 + stack());
            label(&p, r, "Key Frames", encodes);
            let auto = self.deliver_ui.key_auto;
            if radio(
                ui,
                r,
                "key_auto",
                geo().control_left - 8.0,
                "Automatic",
                auto,
                encodes,
            ) {
                self.deliver_ui.key_auto = true;
            }
            let r = row(ui, 34.0);
            if radio(
                ui,
                r,
                "key_every",
                geo().control_left - 8.0,
                "Every",
                !auto,
                encodes,
            ) {
                self.deliver_ui.key_auto = false;
            }
            let mut every = self.deliver_ui.every;
            let field = Rect::from_min_size(
                pos2(r.left() + geo().control_left + 86.0, r.center().y - 14.0),
                vec2(80.0, 28.0),
            );
            if number(ui, field, &mut every, 1..=600, encodes && !auto) {
                self.deliver_ui.every = every;
            }
            crate::i18n::paint_text(&p,
                pos2(field.right() + 14.0, r.center().y),
                Align2::LEFT_CENTER,
                "frames",
                regular(18.0),
                enabled_text(encodes && !auto),
            );
            let r = row(ui, 40.0);
            if let Some(v) = check(
                ui,
                r,
                "reorder",
                geo().control_left - 8.0,
                "Frame reordering",
                self.deliver.options.frame_reordering,
                encodes,
            ) {
                self.deliver.options.frame_reordering = v;
            }
            row(ui, 14.0);
            // Speed, rate control and quality.
            let r = row(ui, 44.0 + stack());
            let speeds = [
                ("Faster", Speed::Faster),
                ("Balanced", Speed::Balanced),
                ("Best", Speed::Best),
            ];
            let current = speeds
                .iter()
                .find(|(_, s)| *s == self.deliver.options.speed)
                .map_or("Balanced", |(n, _)| *n);
            let list: Vec<(String, bool)> =
                speeds.iter().map(|(n, _)| (n.to_string(), true)).collect();
            if let Some(i) = combo(ui, r, "speed", "Preset", current, &list, 188.0, encodes) {
                self.deliver.options.speed = speeds[i].1;
            }
            let r = row(ui, 44.0 + stack());
            let by_quality = matches!(self.deliver.options.rate, Rate::Quality(_));
            let modes = [
                ("Quality".to_string(), true),
                ("Restrict to".to_string(), true),
            ];
            let current = if by_quality { "Quality" } else { "Restrict to" };
            if let Some(i) = combo(
                ui,
                r,
                "rate",
                "Rate Control",
                current,
                &modes,
                218.0,
                encodes,
            ) {
                self.deliver.options.rate = if i == 0 {
                    Rate::Quality(Quality::Best)
                } else {
                    Rate::Bitrate(8000)
                };
            }
            let r = row(ui, 44.0 + stack());
            match self.deliver.options.rate {
                Rate::Quality(q) => {
                    let list: Vec<(String, bool)> = Quality::ALL
                        .iter()
                        .map(|q| (q.label().to_string(), true))
                        .collect();
                    if let Some(i) = combo(
                        ui,
                        r,
                        "quality",
                        "Quality",
                        q.label(),
                        &list,
                        188.0,
                        encodes,
                    ) {
                        self.deliver.options.rate = Rate::Quality(Quality::ALL[i]);
                    }
                }
                Rate::Bitrate(kbps) => {
                    label(&p, r, "Bitrate", encodes);
                    let mut value = kbps;
                    let field = Rect::from_min_size(
                        pos2(r.left() + geo().control_left, r.center().y - 15.0),
                        vec2(120.0, 30.0),
                    );
                    if number(ui, field, &mut value, 100..=500_000, encodes) {
                        self.deliver.options.rate = Rate::Bitrate(value);
                    }
                    crate::i18n::paint_text(&p,
                        pos2(field.right() + 14.0, r.center().y),
                        Align2::LEFT_CENTER,
                        "Kb/s",
                        regular(18.0),
                        enabled_text(encodes),
                    );
                }
            }
            let r = row(ui, 14.0);
            rule(&p, r);
            // Advanced and subtitles.
            let r = row(ui, 44.0);
            self.section(ui, r, "Advanced Settings", true);
            if self.deliver_ui.advanced_open {
                self.form_advanced(ui, row);
            }
            let r = row(ui, 44.0);
            self.section(ui, r, "Subtitle Settings", false);
            if self.deliver_ui.subtitle_open {
                self.form_subtitles(ui, row);
            }
        }

        /// The Advanced Settings. The shape of a pixel, the levels, the colour and gamma tags, the
        /// burn-in and the script change the file; the ones that stand for media or passes this
        /// version does not have are kept and change nothing.
        fn form_advanced(&mut self, ui: &mut Ui, row: &mut dyn FnMut(&mut Ui, f32) -> Rect) {
            let p = ui.painter().clone();
            let x = geo().control_left - 8.0;
            let video = !self.deliver.preset.is_audio_only();
            let is_file = video_file(self.deliver.preset);
            let on = video && is_file;
            row(ui, 6.0);
            // The shape of a pixel.
            let r = row(ui, 38.0 + stack());
            label(&p, r, "Pixel aspect ratio", on);
            let cine = self.deliver_ui.cinemascope;
            if radio(ui, r, "pa_square", x, "Square", !cine, on) {
                self.deliver_ui.cinemascope = false;
            }
            let r = row(ui, 38.0);
            if radio(ui, r, "pa_cine", x, "Cinemascope", cine, on) {
                self.deliver_ui.cinemascope = true;
            }
            // The levels.
            let levels = self.deliver_ui.levels;
            for (n, name) in ["Auto", "Video", "Full"].into_iter().enumerate() {
                let r = row(ui, 38.0 + stack());
                if n == 0 {
                    label(&p, r, "Data Levels", on);
                }
                if radio(ui, r, name, x, name, levels == n, on) {
                    self.deliver_ui.levels = n;
                }
            }
            let r = row(ui, 44.0);
            if let Some(v) = check(
                ui,
                r,
                "retain",
                x,
                "Retain sub-black and super-white data",
                self.deliver_ui.retain_blacks,
                on,
            ) {
                self.deliver_ui.retain_blacks = v;
            }
            // The tags.
            let r = row(ui, 46.0 + stack());
            let list: Vec<(String, bool)> = crate::deliver::COLOR_TAGS
                .iter()
                .map(|(n, _)| (n.to_string(), true))
                .collect();
            let tag = self.deliver_ui.color_tag.min(list.len() - 1);
            if let Some(i) = combo(
                ui,
                r,
                "color_tag",
                "Color Space Tag",
                &list[tag].0,
                &list,
                geo().wide,
                on,
            ) {
                self.deliver_ui.color_tag = i;
            }
            let r = row(ui, 46.0 + stack());
            let list: Vec<(String, bool)> = crate::deliver::GAMMA_TAGS
                .iter()
                .map(|(n, _)| (n.to_string(), true))
                .collect();
            let tag = self.deliver_ui.gamma_tag.min(list.len() - 1);
            if let Some(i) = combo(
                ui,
                r,
                "gamma_tag",
                "Gamma Tag",
                &list[tag].0,
                &list,
                geo().wide,
                on,
            ) {
                self.deliver_ui.gamma_tag = i;
            }
            let r = row(ui, 14.0);
            rule(&p, r);
            // Burn-in.
            let r = row(ui, 46.0 + stack());
            let list: Vec<(String, bool)> = crate::deliver::BURN_INS
                .iter()
                .map(|n| (n.to_string(), true))
                .collect();
            let burn = self.deliver_ui.burn.min(list.len() - 1);
            if let Some(i) = combo(
                ui,
                r,
                "burn",
                "Data burn-in",
                &list[burn].0,
                &list,
                220.0,
                on,
            ) {
                self.deliver_ui.burn = i;
            }
            let r = row(ui, 14.0);
            rule(&p, r);
            // What is used.
            let boxes: [(&str, &str); 5] = [
                ("bypass", "Bypass re-encode when possible"),
                ("proxy", "Use proxy media"),
                ("cached", "Use render cached images"),
                ("sizing", "Force sizing to highest quality"),
                ("debayer", "Force debayer to highest quality"),
            ];
            for (id, text) in boxes {
                let r = row(ui, 52.0);
                let value = match id {
                    "bypass" => self.deliver_ui.bypass_reencode,
                    "proxy" => self.deliver_ui.proxy_media,
                    "cached" => self.deliver_ui.cached_images,
                    "sizing" => self.deliver_ui.force_sizing,
                    _ => self.deliver_ui.force_debayer,
                };
                if let Some(v) = check(ui, r, id, x, text, value, true) {
                    match id {
                        "bypass" => self.deliver_ui.bypass_reencode = v,
                        "proxy" => self.deliver_ui.proxy_media = v,
                        "cached" => self.deliver_ui.cached_images = v,
                        "sizing" => self.deliver_ui.force_sizing = v,
                        _ => self.deliver_ui.force_debayer = v,
                    }
                }
            }
            let r = row(ui, 14.0);
            rule(&p, r);
            // Every video track is always rendered.
            let r = row(ui, 52.0);
            let _ = check(
                ui,
                r,
                "all_tracks",
                x,
                "Render All Video Tracks",
                true,
                false,
            );
            let r = row(ui, 14.0);
            rule(&p, r);
            // Passes.
            let off_on = [("Off".to_string(), true), ("On".to_string(), true)];
            let r = row(ui, 46.0 + stack());
            let now = usize::from(self.deliver_ui.flat_pass);
            if let Some(i) = combo(
                ui,
                r,
                "flat",
                "Enable Flat Pass",
                &off_on[now].0,
                &off_on,
                220.0,
                true,
            ) {
                self.deliver_ui.flat_pass = i == 1;
            }
            let r = row(ui, 46.0 + stack());
            let now = usize::from(self.deliver_ui.vision_bypass);
            if let Some(i) = combo(
                ui,
                r,
                "vision",
                "visionOS Bypass",
                &off_on[now].0,
                &off_on,
                geo().wide,
                true,
            ) {
                self.deliver_ui.vision_bypass = i == 1;
            }
            let r = row(ui, 44.0);
            if let Some(v) = check(
                ui,
                r,
                "no_sizing",
                x,
                "Disable sizing and blanking output",
                self.deliver_ui.disable_sizing,
                true,
            ) {
                self.deliver_ui.disable_sizing = v;
            }
            let r = row(ui, 14.0);
            rule(&p, r);
            // A program at the start or end of the job.
            // Narrow, the Start/End choice goes under the check box.
            let below = geo().stacked;
            let r = row(ui, if below { 96.0 } else { 56.0 });
            let (boxed, beside) = if below {
                (r.translate(vec2(0.0, -20.0)), r.translate(vec2(0.0, 20.0)))
            } else {
                (r, r)
            };
            if let Some(v) = check(
                ui,
                boxed,
                "script",
                x,
                "Trigger script at",
                self.deliver_ui.script_on,
                true,
            ) {
                self.deliver_ui.script_on = v;
            }
            let at = Rect::from_min_size(
                pos2(
                    beside.left() + geo().control_left + if below { 0.0 } else { 218.0 },
                    beside.center().y - 15.0,
                ),
                vec2(if below { 96.0 } else { 130.0 }, 30.0),
            );
            let list = [("Start".to_string(), true), ("End".to_string(), true)];
            let now = usize::from(self.deliver_ui.script_at_end);
            let rr = ui.interact(
                at,
                ui.id().with("script_at"),
                if self.deliver_ui.script_on {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            p.rect_filled(at, 2.0, FIELD);
            p.rect_stroke(at, 2.0, Stroke::new(1.0, FIELD_LINE), StrokeKind::Inside);
            crate::i18n::paint_text(&p,
                at.left_center() + vec2(12.0, 0.0),
                Align2::LEFT_CENTER,
                &list[now].0,
                regular(18.0),
                enabled_text(self.deliver_ui.script_on),
            );
            paint(
                &p,
                at.right_center() - vec2(14.0, 0.0),
                Icon::Chevron,
                if self.deliver_ui.script_on {
                    theme::ICON
                } else {
                    OFF
                },
            );
            let mut picked = None;
            if self.deliver_ui.script_on {
                egui::Popup::menu(&rr).show(|ui| {
                    for (i, (text, _)) in list.iter().enumerate() {
                        if ui.selectable_label(i == now, text.as_str()).clicked() {
                            picked = Some(i);
                        }
                    }
                });
            }
            if let Some(i) = picked {
                self.deliver_ui.script_at_end = i == 1;
            }
            crate::i18n::paint_text(&p,
                pos2(at.right() + if below { 10.0 } else { 14.0 }, at.center().y),
                Align2::LEFT_CENTER,
                "of render job",
                regular(18.0),
                enabled_text(self.deliver_ui.script_on),
            );
            if self.deliver_ui.script_on {
                let r = row(ui, 46.0 + stack());
                label(&p, r, "Program", true);
                let field = control(r, geo().wide + 130.0);
                let mut path = std::mem::take(&mut self.deliver_ui.script_path);
                text_field(ui, field, &mut path, "Path of a program or script");
                self.deliver_ui.script_path = path;
            }
            row(ui, 10.0);
        }

        /// The Subtitle Settings: the titles of the timeline as subtitles, in a file of their
        /// own next to the render, or in the picture (where titles already are).
        fn form_subtitles(&mut self, ui: &mut Ui, row: &mut dyn FnMut(&mut Ui, f32) -> Rect) {
            use crate::subtitles::SubtitleKind;
            const FORMATS: [&str; 3] = [
                "As a separate file",
                "Burn into video",
                "As embedded captions",
            ];
            let p = ui.painter().clone();
            let r = row(ui, 52.0);
            if let Some(v) = check(
                ui,
                r,
                "export_subtitle",
                14.0,
                "Export Subtitle",
                self.deliver_ui.export_subtitle,
                true,
            ) {
                self.deliver_ui.export_subtitle = v;
            }
            let r = row(ui, 12.0);
            rule(&p, r);
            let on = self.deliver_ui.export_subtitle;
            row(ui, 8.0);
            let r = row(ui, 44.0 + stack());
            let format = self.deliver_ui.subtitle_format.min(FORMATS.len() - 1);
            // Embedded captions need a container that carries them; this version writes none.
            let list: Vec<(String, bool)> = FORMATS
                .iter()
                .enumerate()
                .map(|(i, f)| (f.to_string(), i < 2))
                .collect();
            if let Some(i) = combo(
                ui,
                r,
                "subtitle_format",
                "Format",
                FORMATS[format],
                &list,
                geo().wide,
                on,
            ) {
                self.deliver_ui.subtitle_format = i;
            }
            let r = row(ui, 44.0 + stack());
            let kind = self.deliver_ui.subtitle_kind;
            let separate = format == 0;
            let list: Vec<(String, bool)> = SubtitleKind::ALL
                .iter()
                .map(|k| (k.label().to_string(), true))
                .collect();
            if let Some(i) = combo(
                ui,
                r,
                "subtitle_kind",
                "Export As",
                kind.label(),
                &list,
                geo().wide,
                on && separate,
            ) {
                if let Some(k) = SubtitleKind::ALL.get(i) {
                    self.deliver_ui.subtitle_kind = *k;
                }
            }
            row(ui, 10.0);
        }

        /// A heading with a line after it that opens and closes.
        fn section(&mut self, ui: &mut Ui, r: Rect, name: &str, advanced: bool) {
            let p = ui.painter().clone();
            let open = if advanced {
                self.deliver_ui.advanced_open
            } else {
                self.deliver_ui.subtitle_open
            };
            let hit = Rect::from_min_size(r.min, vec2(r.width(), r.height()));
            let resp = ui.interact(hit, ui.id().with(("section", name)), Sense::click());
            paint(
                &p,
                pos2(r.left() + 22.0, r.center().y),
                if open { Icon::Chevron } else { Icon::ChevRight },
                theme::ICON,
            );
            let galley = p.layout_no_wrap(
                name.to_string(),
                regular(18.0),
                if resp.hovered() {
                    theme::WHITE
                } else {
                    Color32::from_rgb(160, 160, 166)
                },
            );
            let width = galley.size().x;
            p.galley(
                pos2(r.left() + 44.0, r.center().y - galley.size().y / 2.0),
                galley,
                TEXT,
            );
            p.hline(
                r.left() + 56.0 + width..=r.right() - 6.0,
                r.center().y,
                Stroke::new(1.0, Color32::from_rgb(40, 40, 46)),
            );
            if resp.clicked() {
                if advanced {
                    self.deliver_ui.advanced_open = !open;
                } else {
                    self.deliver_ui.subtitle_open = !open;
                }
            }
        }

        fn form_audio(&mut self, ui: &mut Ui, row: &mut dyn FnMut(&mut Ui, f32) -> Rect) {
            let p = ui.painter().clone();
            let preset = self.deliver.preset;
            let has_sound = !matches!(preset, Preset::PngSequence);
            let r = row(ui, 52.0);
            if let Some(v) = check(
                ui,
                r,
                "export_audio",
                14.0,
                "Export Audio",
                self.deliver.options.audio && has_sound,
                has_sound,
            ) {
                self.deliver.options.audio = v;
            }
            let r = row(ui, 12.0);
            rule(&p, r);
            let on = has_sound && self.deliver.options.audio;
            // The codec: each file type has its own, and a QuickTime file AAC or linear PCM.
            let r = row(ui, 44.0 + stack());
            let mov = matches!(preset, Preset::ProRes422 | Preset::DnxHr);
            let pcm_now = mov && self.deliver.options.audio_codec != AudioCodec::Aac;
            let shown = match preset {
                Preset::Wav => "Linear PCM",
                Preset::Mp3 => "MP3",
                Preset::Vp9 => "Opus",
                Preset::Ffv1 => "FLAC",
                _ if pcm_now => "Linear PCM",
                _ => "AAC",
            };
            let list: Vec<(String, bool)> = if mov {
                vec![("AAC".to_string(), true), ("Linear PCM".to_string(), true)]
            } else {
                vec![(shown.to_string(), true)]
            };
            if let Some(i) = combo(ui, r, "acodec", "Codec", shown, &list, geo().wide, on && mov) {
                self.deliver.options.audio_codec = if i == 0 {
                    AudioCodec::Aac
                } else {
                    AudioCodec::Pcm
                };
            }
            // The data rate of the codecs that have one.
            let r = row(ui, 44.0 + stack());
            let rates = [128u32, 192, 256, 320];
            let list: Vec<(String, bool)> =
                rates.iter().map(|k| (format!("{k} kbps"), true)).collect();
            let current = format!("{} kbps", self.deliver.options.audio_kbps);
            let has_rate = on && matches!(shown, "AAC" | "Opus" | "MP3");
            if let Some(i) = combo(ui, r, "arate", "Data Rate", &current, &list, geo().wide, has_rate) {
                if let Some(k) = rates.get(i) {
                    self.deliver.options.audio_kbps = *k;
                }
            }
            let r = row(ui, 44.0 + stack());
            label(&p, r, "Output Track 1", on);
            crate::i18n::paint_text(&p,
                pos2(r.left() + geo().control_left + 4.0, r.center().y),
                Align2::LEFT_CENTER,
                "Main, stereo, 48 kHz",
                regular(18.0),
                enabled_text(on),
            );
        }

        fn form_file(&mut self, ui: &mut Ui, row: &mut dyn FnMut(&mut Ui, f32) -> Rect) {
            let p = ui.painter().clone();
            let r = row(ui, 52.0);
            if let Some(v) = check(
                ui,
                r,
                "unique",
                14.0,
                "Use unique filename",
                self.deliver_ui.unique,
                true,
            ) {
                self.deliver_ui.unique = v;
            }
            let r = row(ui, 40.0);
            crate::i18n::paint_text(&p,
                pos2(r.left() + 52.0, r.center().y),
                Align2::LEFT_CENTER,
                "Adds (1), (2) ... to the name instead of writing over a file.",
                regular(17.0),
                Color32::from_rgb(160, 160, 166),
            );
            let r = row(ui, 12.0);
            rule(&p, r);
            let r = row(ui, 44.0 + stack());
            label(&p, r, "File type", true);
            let ext = match self.deliver.preset.extension() {
                "" => "folder of PNG pictures".to_string(),
                e => format!(".{e}"),
            };
            crate::i18n::paint_text(&p,
                pos2(r.left() + geo().control_left + 4.0, r.center().y),
                Align2::LEFT_CENTER,
                ext,
                regular(18.0),
                TEXT,
            );
            let r = row(ui, 44.0 + stack());
            label(&p, r, "Range", true);
            let (marks, whole) = (
                self.project().mark_in.is_some() || self.project().mark_out.is_some(),
                self.range_is_whole(),
            );
            let r2 = r;
            if radio(
                ui,
                r2,
                "range_whole",
                geo().control_left - 8.0,
                "Whole timeline",
                whole,
                true,
            ) {
                self.set_range_whole(true);
            }
            let r = row(ui, 44.0);
            if radio(
                ui,
                r,
                "range_marks",
                geo().control_left - 8.0,
                "In to out marks",
                !whole,
                marks,
            ) {
                self.set_range_whole(false);
            }
        }
    }

    fn fps_label(f: picasso_core::Fps) -> String {
        if f == picasso_core::Fps::FPS_2997 {
            "29.97".to_string()
        } else {
            format!("{}", f.as_f64())
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod geometry_tests {
    use super::native::{Geo, SIDE_BY_SIDE};

    #[test]
    fn a_wide_form_keeps_its_labels_beside_the_controls() {
        let g = Geo::for_width(685.0);
        assert!(!g.stacked);
        assert_eq!((g.label_right, g.control_left, g.wide), (205.0, 220.0, 292.0));
        assert!(!Geo::for_width(SIDE_BY_SIDE).stacked);
    }

    #[test]
    fn a_narrow_form_stacks_and_its_controls_stay_inside_it() {
        // From just under the wide layout down to the narrowest a divider allows (0.4 of 685).
        for width in [649.0, 500.0, 360.0, 274.0, 150.0] {
            let g = Geo::for_width(width);
            assert!(g.stacked, "{width}");
            assert!(
                g.control_left + g.wide <= width.max(g.control_left + 110.0),
                "{width}: a wide control must not pass the right edge"
            );
            assert!(g.wide >= 110.0);
        }
    }
}
