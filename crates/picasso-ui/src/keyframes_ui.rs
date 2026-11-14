//! Keyframes in the Inspector: a diamond beside each property that can be animated (filled when
//! there is a keyframe at the playhead), arrows to the keyframes before and after, and the
//! property's value written as a keyframe once it is animated.

use egui::{Color32, Context, Sense, Stroke, Ui};
use picasso_core::{ClipId, Command, KeyParam, Transform};

use crate::app::StudioApp;

const KEY_ON: Color32 = Color32::from_rgb(232, 120, 60);
const KEY_OFF: Color32 = Color32::from_rgb(130, 130, 136);

/// A diamond button, filled or not; returns whether it was clicked.
fn diamond(ui: &mut Ui, filled: bool, lit: bool, hint: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::click());
    let c = rect.center();
    let r = 5.5;
    let points = vec![
        c + egui::vec2(0.0, -r),
        c + egui::vec2(r, 0.0),
        c + egui::vec2(0.0, r),
        c + egui::vec2(-r, 0.0),
    ];
    let color = if lit || response.hovered() {
        KEY_ON
    } else {
        KEY_OFF
    };
    let fill = if filled { color } else { Color32::TRANSPARENT };
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        fill,
        Stroke::new(1.3, color),
    ));
    response.on_hover_text(hint).clicked()
}

impl StudioApp {
    /// The playhead as a frame of clip `id`, when it is inside the clip.
    pub(crate) fn clip_frame(&self, id: ClipId) -> Option<u64> {
        let (_, c) = self.project().clip(id)?;
        c.contains(self.playhead).then(|| self.playhead - c.start)
    }

    /// The diamond and arrows of `param` on clip `id`.
    pub(crate) fn key_buttons(&mut self, ui: &mut Ui, id: ClipId, param: KeyParam) {
        let Some((_, clip)) = self.project().clip(id) else {
            return;
        };
        let keys: Vec<u64> = clip
            .keys(param)
            .map(|k| k.iter().map(|k| k.at).collect())
            .unwrap_or_default();
        let (start, value) = (clip.start, clip.value_at(param, self.playhead));
        let here = self.clip_frame(id);
        // The curve of the keyframe at the playhead, if there is one.
        let curve_here = here.and_then(|f| {
            clip.keys(param)?
                .iter()
                .find(|k| k.at == f)
                .map(|k| k.curve)
        });
        let on_key = here.is_some_and(|f| keys.contains(&f));
        let animated = !keys.is_empty();
        let before = here.and_then(|f| keys.iter().rev().find(|k| **k < f).copied());
        let after = here.and_then(|f| keys.iter().find(|k| **k > f).copied());
        ui.add_enabled_ui(before.is_some(), |ui| {
            if ui
                .small_button("<")
                .on_hover_text("The keyframe before")
                .clicked()
            {
                if let Some(k) = before {
                    self.go_to(start + k);
                }
            }
        });
        let hint = if on_key {
            "Take the keyframe away"
        } else {
            "Add a keyframe at the playhead"
        };
        if diamond(ui, on_key, animated, hint) {
            match here {
                Some(at) => {
                    self.run(Command::SetKeyframe {
                        clip: id,
                        param,
                        at,
                        value: (!on_key).then_some(value),
                        ease: false,
                    });
                }
                None => self.fail("put the playhead inside the clip to add a keyframe"),
            }
        }
        // On a keyframe: the curve the value takes to the next one.
        if let (Some(current), Some(at)) = (curve_here, here) {
            let name = picasso_core::easing::preset_of(current).map_or("Custom", |p| p.label);
            let mut chosen = None;
            ui.menu_button("~", |ui| {
                for family in picasso_core::easing::FAMILIES {
                    ui.menu_button(family, |ui| {
                        for p in picasso_core::easing::PRESETS.iter().filter(|p| p.family == family) {
                            if ui.selectable_label(current == p.curve, p.label).clicked() {
                                chosen = Some(p.curve);
                                ui.close();
                            }
                        }
                    });
                }
            })
            .response
            .on_hover_text(format!("Curve to the next keyframe: {name}"));
            if let Some(curve) = chosen {
                self.run(Command::SetKeyCurve {
                    clip: id,
                    param,
                    at,
                    curve,
                });
            }
        }
        ui.add_enabled_ui(after.is_some(), |ui| {
            if ui
                .small_button(">")
                .on_hover_text("The keyframe after")
                .clicked()
            {
                if let Some(k) = after {
                    self.go_to(start + k);
                }
            }
        });
    }

    /// Writes a changed value of `param`: a keyframe at the playhead when the property is
    /// animated (a drag is one undo step), else the clip's own value. Returns whether it was
    /// written as a keyframe.
    pub(crate) fn set_animated(
        &mut self,
        ctx: &Context,
        id: ClipId,
        param: KeyParam,
        value: f32,
    ) -> bool {
        let animated = self
            .project()
            .clip(id)
            .is_some_and(|(_, c)| c.keys(param).is_some());
        if !animated {
            return false;
        }
        match self.clip_frame(id) {
            Some(at) => self.run_gesture(
                ctx,
                Command::SetKeyframe {
                    clip: id,
                    param,
                    at,
                    value: Some(value),
                    ease: false,
                },
            ),
            None => self.fail("put the playhead inside the clip to change an animated value"),
        }
        true
    }

    /// The transform shown in the Inspector: the clip's, with its animation at the playhead.
    pub(crate) fn shown_transform(&self, id: ClipId) -> (f32, Transform) {
        self.project()
            .clip(id)
            .map_or((1.0, Transform::default()), |(_, c)| {
                c.animated(self.playhead.clamp(c.start, c.end().saturating_sub(1)))
            })
    }
}
