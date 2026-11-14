//! The EQ and Dynamics windows of a sound track, opened from the Audio page's mixer, the
//! Effects > Audio FX menu and the Inspector.

use egui::{pos2, Color32, Context, Rect, Sense, Stroke, Ui};
use picasso_core::{
    AudioEffect, AudioEffectGroup, AudioEffectKind, Command, TrackFx, TrackId, TrackKind,
    EQ_RANGE_DB, MAX_CHAIN,
};

use crate::app::StudioApp;

/// Which of the three the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FxTab {
    Eq,
    Dynamics,
    /// The sound effects: reverb, echo, voice changers...
    Effects,
}

/// The frequencies on the EQ graph, log spaced.
const LOW_HZ: f32 = 20.0;
const HIGH_HZ: f32 = 20_000.0;

/// Draws the EQ's curve in `rect` (20 Hz to 20 kHz across, +-20 dB up and down).
pub(crate) fn draw_eq_curve(
    painter: &egui::Painter,
    rect: Rect,
    eq: &picasso_core::Equalizer,
    color: Color32,
) {
    let points: Vec<egui::Pos2> = (0..=80)
        .map(|i| {
            let t = i as f32 / 80.0;
            let hz = LOW_HZ * (HIGH_HZ / LOW_HZ).powf(t);
            let db = picasso_audio::dsp::eq_response_db(eq, hz).clamp(-EQ_RANGE_DB, EQ_RANGE_DB);
            pos2(
                rect.left() + rect.width() * t,
                rect.center().y - db / EQ_RANGE_DB * rect.height() / 2.0,
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, color)));
}

impl StudioApp {
    /// Opens the window for the track of the clip being worked on, or the first sound track.
    pub(crate) fn open_track_fx(&mut self, tab: FxTab) {
        let project = self.project();
        let from_clip = self
            .target_clip()
            .and_then(|id| project.clip(id))
            .filter(|(t, _)| t.kind == TrackKind::Audio)
            .map(|(t, _)| t.id);
        let track = from_clip.or_else(|| {
            project
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Audio)
                .map(|t| t.id)
        });
        match track {
            Some(t) => self.extra.track_fx = Some((t, tab)),
            None => self.fail("there is no sound track"),
        }
    }

    pub(crate) fn track_fx_window(&mut self, ctx: &Context) {
        let Some((track, mut tab)) = self.extra.track_fx else {
            return;
        };
        let Some((name, mut fx)) = self
            .project()
            .track(track)
            .map(|t| (t.name.clone(), t.fx.clone()))
        else {
            self.extra.track_fx = None;
            return;
        };
        let before = fx.clone();
        let mut open = true;
        egui::Window::new(format!("{name} - Audio FX"))
            .id(egui::Id::new("track_fx_window"))
            .open(&mut open)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut tab, FxTab::Eq, "EQ");
                    ui.selectable_value(&mut tab, FxTab::Dynamics, "Dynamics");
                    ui.selectable_value(&mut tab, FxTab::Effects, "Effects");
                });
                ui.separator();
                match tab {
                    FxTab::Eq => eq_controls(ui, &mut fx),
                    FxTab::Dynamics => dynamics_controls(ui, &mut fx),
                    FxTab::Effects => chain_controls(ui, &mut fx),
                }
            });
        if fx != before {
            self.run_gesture(ctx, Command::SetTrackFx { track, fx });
        }
        self.extra.track_fx = open.then_some((track, tab));
    }

    /// The effects of track `track`, for the strip's curve.
    pub(crate) fn track_fx_of(&self, track: TrackId) -> TrackFx {
        self.project()
            .track(track)
            .map(|t| t.fx.clone())
            .unwrap_or_default()
    }

    /// Adds sound effect `kind` to the track of the clip being worked on (or the first sound
    /// track) and opens its Effects list.
    pub(crate) fn add_audio_effect(&mut self, kind: AudioEffectKind) {
        self.open_track_fx(FxTab::Effects);
        let Some((track, _)) = self.extra.track_fx else {
            return;
        };
        let mut fx = self.track_fx_of(track);
        if fx.chain.len() >= MAX_CHAIN {
            self.fail(format!("a track has at most {MAX_CHAIN} sound effects"));
            return;
        }
        fx.chain.push(AudioEffect::new(kind));
        if self.run(Command::SetTrackFx { track, fx }).is_some() {
            self.info(format!("Added {} to the track", kind.label()));
        }
    }
}

/// The sound effects of the track: each with its switch, its knobs and buttons to move or remove
/// it, and a menu to add another.
fn chain_controls(ui: &mut Ui, fx: &mut TrackFx) {
    if fx.chain.is_empty() {
        ui.weak("No sound effects yet. Add one below, or from Effects > Audio FX.");
    }
    let count = fx.chain.len();
    let mut remove = None;
    let mut swap = None;
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        for (i, effect) in fx.chain.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut effect.enabled, "");
                    ui.label(egui::RichText::new(effect.kind.label()).strong());
                    ui.weak(effect.kind.group().label());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                        if ui.add_enabled(i + 1 < count, egui::Button::new("Down").small()).clicked() {
                            swap = Some((i, i + 1));
                        }
                        if ui.add_enabled(i > 0, egui::Button::new("Up").small()).clicked() {
                            swap = Some((i - 1, i));
                        }
                        if ui.small_button("Reset").clicked() {
                            *effect = AudioEffect {
                                enabled: effect.enabled,
                                ..AudioEffect::new(effect.kind)
                            };
                        }
                    });
                });
                ui.add_enabled_ui(effect.enabled, |ui| {
                    for (value, spec) in effect.params.iter_mut().zip(effect.kind.params()) {
                        ui.add(
                            egui::Slider::new(value, spec.min..=spec.max)
                                .logarithmic(spec.log)
                                .text(spec.label)
                                .suffix(spec.unit),
                        );
                    }
                });
                ui.separator();
            });
        }
    });
    if let Some(i) = remove {
        fx.chain.remove(i);
    }
    if let Some((a, b)) = swap {
        fx.chain.swap(a, b);
    }
    ui.add_enabled_ui(fx.chain.len() < MAX_CHAIN, |ui| {
        ui.menu_button("Add sound effect", |ui| {
            for group in AudioEffectGroup::ALL {
                ui.menu_button(group.label(), |ui| {
                    for kind in AudioEffectKind::ALL.iter().filter(|k| k.group() == group) {
                        if ui.button(kind.label()).clicked() {
                            fx.chain.push(AudioEffect::new(*kind));
                            ui.close();
                        }
                    }
                });
            }
        });
    });
}

fn eq_controls(ui: &mut Ui, fx: &mut TrackFx) {
    let eq = &mut fx.eq;
    ui.checkbox(&mut eq.enabled, "EQ on");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(340.0, 120.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(20, 20, 23));
    for db in [-12.0f32, 0.0, 12.0] {
        let y = rect.center().y - db / EQ_RANGE_DB * rect.height() / 2.0;
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(if db == 0.0 { 70 } else { 45 })),
        );
    }
    for hz in [100.0f32, 1000.0, 10_000.0] {
        let x = rect.left() + rect.width() * (hz / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln();
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(45)),
        );
        crate::i18n::paint_text(&painter,
            pos2(x + 3.0, rect.bottom() - 3.0),
            egui::Align2::LEFT_BOTTOM,
            if hz >= 1000.0 {
                format!("{}k", hz / 1000.0)
            } else {
                format!("{hz}")
            },
            egui::FontId::proportional(10.0),
            Color32::from_gray(110),
        );
    }
    draw_eq_curve(
        &painter,
        rect,
        eq,
        if eq.enabled {
            Color32::from_rgb(90, 200, 120)
        } else {
            Color32::from_gray(90)
        },
    );
    ui.add_enabled_ui(eq.enabled, |ui| {
        egui::Grid::new("eq_bands").num_columns(3).show(ui, |ui| {
            ui.strong("Low");
            ui.add(
                egui::Slider::new(&mut eq.low_freq, 20.0..=1000.0)
                    .logarithmic(true)
                    .suffix(" Hz"),
            );
            ui.add(egui::Slider::new(&mut eq.low_gain, -EQ_RANGE_DB..=EQ_RANGE_DB).suffix(" dB"));
            ui.end_row();
            ui.strong("Mid");
            ui.add(
                egui::Slider::new(&mut eq.mid_freq, 100.0..=10_000.0)
                    .logarithmic(true)
                    .suffix(" Hz"),
            );
            ui.add(egui::Slider::new(&mut eq.mid_gain, -EQ_RANGE_DB..=EQ_RANGE_DB).suffix(" dB"));
            ui.end_row();
            ui.label("");
            ui.add(egui::Slider::new(&mut eq.mid_q, 0.3..=8.0).text("Q"));
            ui.label("");
            ui.end_row();
            ui.strong("High");
            ui.add(
                egui::Slider::new(&mut eq.high_freq, 1000.0..=20_000.0)
                    .logarithmic(true)
                    .suffix(" Hz"),
            );
            ui.add(egui::Slider::new(&mut eq.high_gain, -EQ_RANGE_DB..=EQ_RANGE_DB).suffix(" dB"));
            ui.end_row();
        });
        if ui.button("Flat").clicked() {
            *eq = picasso_core::Equalizer {
                enabled: true,
                ..Default::default()
            };
        }
    });
}

fn dynamics_controls(ui: &mut Ui, fx: &mut TrackFx) {
    let c = &mut fx.dynamics;
    ui.checkbox(&mut c.enabled, "Compressor on");
    // The transfer curve: input level across, output level up, -60 to 0 dB.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 160.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(20, 20, 23));
    let at = |db_in: f32, db_out: f32| {
        pos2(
            rect.left() + (db_in + 60.0) / 60.0 * rect.width(),
            rect.bottom() - (db_out + 60.0) / 60.0 * rect.height(),
        )
    };
    painter.line_segment(
        [at(-60.0, -60.0), at(0.0, 0.0)],
        Stroke::new(1.0, Color32::from_gray(55)),
    );
    let curve: Vec<egui::Pos2> = (0..=60)
        .map(|i| {
            let x = -60.0 + i as f32;
            let y = if c.enabled {
                x + c.reduction_db(x) + c.makeup
            } else {
                x
            };
            at(x, y.clamp(-60.0, 0.0))
        })
        .collect();
    painter.add(egui::Shape::line(
        curve,
        Stroke::new(
            1.5,
            if c.enabled {
                Color32::from_rgb(230, 170, 60)
            } else {
                Color32::from_gray(90)
            },
        ),
    ));
    ui.add_enabled_ui(c.enabled, |ui| {
        ui.add(
            egui::Slider::new(&mut c.threshold, -60.0..=0.0)
                .text("Threshold")
                .suffix(" dB"),
        );
        ui.add(
            egui::Slider::new(&mut c.ratio, 1.0..=20.0)
                .text("Ratio")
                .suffix(":1"),
        );
        ui.add(
            egui::Slider::new(&mut c.attack_ms, 0.1..=200.0)
                .logarithmic(true)
                .text("Attack")
                .suffix(" ms"),
        );
        ui.add(
            egui::Slider::new(&mut c.release_ms, 5.0..=2000.0)
                .logarithmic(true)
                .text("Release")
                .suffix(" ms"),
        );
        ui.add(
            egui::Slider::new(&mut c.makeup, 0.0..=24.0)
                .text("Make Up")
                .suffix(" dB"),
        );
    });
}
