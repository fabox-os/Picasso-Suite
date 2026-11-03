//! The line under About that names the two people behind this build, shown only when the
//! version number is shift-clicked: GMN (India) and Fabox (Portugal), sent out to the world.
//!
//! The flags and the globe are drawn with shapes, like every icon in this program: no emoji
//! font is shipped, and the text drawing sets letters one at a time, so a flag emoji would come
//! out as two letters in a box.
//!
//! Nothing else in the program depends on this file: delete it when GMN stops working on the
//! project and the About window simply stops offering the line.

use egui::{pos2, vec2, Color32, Painter, Rect, Sense, Shape, Stroke, StrokeKind, Ui};

use crate::theme::{ICON, TEXT_DIM};

/// The effects author this line was added with.
pub(crate) const EFFECTS_AUTHOR: &str = "GMN";
/// The maintainer of the project.
pub(crate) const MAINTAINER: &str = "Fabox";
/// The line under the two names.
pub(crate) const THANKS: &str =
    "Thanks for using Picasso Suite - keep up the good work, you're doing great.";

/// The size a flag is drawn at, in points.
const FLAG: egui::Vec2 = vec2(16.0, 11.0);

/// The credits: both names with their flag, an arrow to the world, and the thanks line.
pub(crate) fn row(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(EFFECTS_AUTHOR);
        flag(ui, india);
        ui.label("·");
        ui.label(MAINTAINER);
        flag(ui, portugal);
        ui.label("→");
        globe(ui);
    });
    // One line: the label asks for the width it needs, which widens the About window rather
    // than wrapping the thanks note onto a second line.
    ui.add(
        egui::Label::new(egui::RichText::new(THANKS).color(TEXT_DIM))
            .wrap_mode(egui::TextWrapMode::Extend),
    );
}

/// Draws one flag in the row.
fn flag(ui: &mut Ui, draw: fn(&Painter, Rect)) {
    let (rect, _) = ui.allocate_exact_size(FLAG, Sense::hover());
    draw(ui.painter(), rect);
}

/// One of `count` equal horizontal bands of `rect`, the last ending exactly at the bottom.
fn band(rect: Rect, index: usize, count: usize) -> Rect {
    let height = rect.height() / count as f32;
    let top = rect.top() + height * index as f32;
    let bottom = if index + 1 == count {
        rect.bottom()
    } else {
        rect.top() + height * (index + 1) as f32
    };
    Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom))
}

/// A hairline around a flag, so its white band is not lost on the dark window.
fn border(painter: &Painter, rect: Rect) {
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(0.6, Color32::from_gray(80)),
        StrokeKind::Inside,
    );
}

/// India: saffron, white and green, with the navy chakra in the middle.
fn india(painter: &Painter, rect: Rect) {
    painter.rect_filled(band(rect, 0, 3), 0.0, Color32::from_rgb(0xFF, 0x99, 0x33));
    painter.rect_filled(band(rect, 1, 3), 0.0, Color32::WHITE);
    painter.rect_filled(band(rect, 2, 3), 0.0, Color32::from_rgb(0x13, 0x88, 0x08));
    // The chakra, as much of it as a 16-point flag holds: a navy ring in the white band.
    painter.circle_stroke(rect.center(), 1.7, Stroke::new(0.7, Color32::from_rgb(0, 0, 0x80)));
    border(painter, rect);
}

/// Portugal: green and red, with the armillary sphere reduced to a yellow ring.
fn portugal(painter: &Painter, rect: Rect) {
    let split = rect.left() + rect.width() * 0.4;
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(split, rect.bottom())),
        0.0,
        Color32::from_rgb(0x04, 0x6A, 0x38),
    );
    painter.rect_filled(
        Rect::from_min_max(pos2(split, rect.top()), rect.max),
        0.0,
        Color32::from_rgb(0xDA, 0x29, 0x1C),
    );
    painter.circle_stroke(
        pos2(split, rect.center().y),
        2.2,
        Stroke::new(0.9, Color32::from_rgb(0xFF, 0xE9, 0x00)),
    );
    border(painter, rect);
}

/// The world: a circle with one meridian and the equator.
fn globe(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let painter = ui.painter();
    let middle = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0 - 0.5;
    painter.circle_stroke(middle, radius, Stroke::new(1.0, ICON));
    painter.add(Shape::ellipse_stroke(
        middle,
        vec2(radius * 0.45, radius),
        Stroke::new(0.7, ICON),
    ));
    painter.line_segment(
        [
            pos2(middle.x - radius, middle.y),
            pos2(middle.x + radius, middle.y),
        ],
        Stroke::new(0.7, ICON),
    );
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use egui::{Context, Pos2, RawInput, Rect, Shape};

    use super::*;
    use crate::app::StudioApp;

    fn texts(shape: &Shape, out: &mut BTreeSet<String>) {
        match shape {
            Shape::Text(t) => {
                let text = t.galley.job.text.trim();
                if !text.is_empty() {
                    out.insert(text.to_string());
                }
            }
            Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }

    /// What About paints, with the credits line opened or not.
    fn about(credits_open: bool) -> BTreeSet<String> {
        let ctx = Context::default();
        let mut app = StudioApp::new(&ctx);
        app.load_demo();
        app.manager.open = false;
        app.show_about = true;
        app.show_credits = credits_open;
        let mut found = BTreeSet::new();
        for _ in 0..3 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.ui(ui));
            output.textures_delta.clear();
            for clipped in &output.shapes {
                texts(&clipped.shape, &mut found);
            }
        }
        found
    }

    #[test]
    fn the_two_names_and_the_thanks_line_are_painted_when_it_is_opened() {
        let painted = about(true);
        assert!(painted.contains(EFFECTS_AUTHOR), "{painted:?}");
        assert!(painted.contains(MAINTAINER), "{painted:?}");
        assert!(painted.contains(THANKS), "{painted:?}");
    }

    #[test]
    fn about_paints_none_of_them_until_the_version_is_shift_clicked() {
        let painted = about(false);
        assert!(!painted.contains(EFFECTS_AUTHOR), "{painted:?}");
        assert!(!painted.contains(MAINTAINER), "{painted:?}");
        assert!(!painted.contains(THANKS), "{painted:?}");
    }

    #[test]
    fn both_names_have_a_flag_and_the_thanks_line_is_one_line() {
        assert!(!EFFECTS_AUTHOR.trim().is_empty());
        assert!(!MAINTAINER.trim().is_empty());
        assert_ne!(EFFECTS_AUTHOR, MAINTAINER, "two different people");
        assert!(!THANKS.contains('\n'), "one line: the window widens rather than wrapping");
    }
}
