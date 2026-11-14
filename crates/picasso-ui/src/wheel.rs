//! A colour wheel: the tint of a lift, gamma, gain or offset.
//!
//! The three channel values are shown as a point in the chroma plane (where the channels differ)
//! and a master (their mean). Dragging the puck changes the tint and keeps the master.

use std::f32::consts::TAU;

use egui::{vec2, Color32, Mesh, Pos2, Sense, Shape, Stroke, Ui, Vec2};

const SQRT3: f32 = 1.732_050_8;
/// How far the wheel is turned: red sits near the top, green at the lower left and blue at the
/// right, as on the reference wheels.
const TURN: f32 = 105.0 * std::f32::consts::PI / 180.0;

fn turn(v: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Splits channel values into (mean, x, y) with `x = r - (g + b) / 2` and `y = sqrt(3)/2 * (g - b)`.
fn to_plane(rgb: [f32; 3]) -> (f32, f32, f32) {
    (
        (rgb[0] + rgb[1] + rgb[2]) / 3.0,
        rgb[0] - 0.5 * (rgb[1] + rgb[2]),
        SQRT3 / 2.0 * (rgb[1] - rgb[2]),
    )
}

/// The inverse of [`to_plane`].
fn from_plane(mean: f32, x: f32, y: f32) -> [f32; 3] {
    [
        mean + 2.0 * x / 3.0,
        mean - x / 3.0 + y / SQRT3,
        mean - x / 3.0 - y / SQRT3,
    ]
}

fn clamp_len(v: Vec2, max: f32) -> Vec2 {
    let length = v.length();
    if length > max && length > 0.0 {
        v * (max / length)
    } else {
        v
    }
}

fn hue_color(angle: f32) -> Color32 {
    egui::ecolor::Hsva::new(angle.rem_euclid(TAU) / TAU, 0.92, 1.0, 1.0).into()
}

/// Draws one wheel around `center` with outer radius `radius` (the dark rim) and lets the user
/// drag its puck; returns whether `rgb` changed. `neutral` is the value of an untouched channel
/// (a double-click goes back to it), `range` how far the puck may move in the chroma plane.
#[allow(clippy::too_many_arguments)]
pub(crate) fn color_wheel(
    ui: &mut Ui,
    id: egui::Id,
    center: Pos2,
    radius: f32,
    rgb: &mut [f32; 3],
    neutral: f32,
    range: f32,
    arc_degrees: f32,
) -> bool {
    let painter = ui.painter().clone();
    // As Resolve draws it, from the outside in: a black rim carrying a light arc, a dark gap, a
    // thin ring of hues, then a dark field faintly tinted by the hue at each edge, with a
    // cross through the middle.
    painter.circle_filled(center, radius, Color32::from_rgb(10, 10, 11));
    let (outer, inner) = (radius * 0.81, radius * 0.745);
    painter.circle_filled(center, radius * 0.9, Color32::from_rgb(22, 22, 25));
    // The light arc on the rim: where it sits says the wheel's master value. It is centred on
    // `arc_degrees` (counter-clockwise from the right) and turns with the value.
    {
        let turned = (master(*rgb) - neutral) / (range * 2.0).max(1e-3) * 180.0;
        let middle = (arc_degrees + turned).to_radians();
        let half = 50f32.to_radians();
        let r = radius * 0.945;
        let steps = 40;
        for i in 0..steps {
            let (t0, t1) = (i as f32 / steps as f32, (i + 1) as f32 / steps as f32);
            let (a0, a1) = (
                middle - half + 2.0 * half * t0,
                middle - half + 2.0 * half * t1,
            );
            // Even along its length, tapering only at the ends.
            let edge = ((t0 + t1) / 2.0).min(1.0 - (t0 + t1) / 2.0);
            let alpha = ((edge / 0.12).min(1.0) * 225.0) as u8;
            painter.line_segment(
                [
                    Pos2::new(center.x + a0.cos() * r, center.y - a0.sin() * r),
                    Pos2::new(center.x + a1.cos() * r, center.y - a1.sin() * r),
                ],
                Stroke::new(
                    radius * 0.036,
                    Color32::from_rgba_unmultiplied(226, 226, 230, alpha),
                ),
            );
        }
    }
    // The field: grey in the middle, warming to a dark shade of each hue at the edge.
    {
        let mut mesh = Mesh::default();
        let steps = 120;
        mesh.colored_vertex(center, Color32::from_rgb(52, 52, 56));
        for i in 0..=steps {
            let angle = i as f32 / steps as f32 * TAU;
            let (s, c) = angle.sin_cos();
            let hue = hue_color(angle - TURN);
            let mix = |h: u8| (f32::from(h) * 0.16 + 38.0) as u8;
            mesh.colored_vertex(
                Pos2::new(center.x + c * inner, center.y - s * inner),
                Color32::from_rgb(mix(hue.r()), mix(hue.g()), mix(hue.b())),
            );
        }
        for i in 1..=steps as u32 {
            mesh.add_triangle(0, i, i + 1);
        }
        painter.add(Shape::mesh(mesh));
    }
    // The ring of hues.
    {
        let mut mesh = Mesh::default();
        let steps = 120;
        for i in 0..=steps {
            let angle = i as f32 / steps as f32 * TAU;
            let (s, c) = angle.sin_cos();
            let color = hue_color(angle - TURN);
            mesh.colored_vertex(Pos2::new(center.x + c * outer, center.y - s * outer), color);
            mesh.colored_vertex(Pos2::new(center.x + c * inner, center.y - s * inner), color);
        }
        for i in 0..steps as u32 {
            let k = i * 2;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
        painter.add(Shape::mesh(mesh));
    }
    let faint = Stroke::new(1.0, Color32::from_rgb(112, 112, 118));
    painter.line_segment(
        [center - vec2(inner, 0.0), center + vec2(inner, 0.0)],
        faint,
    );
    painter.line_segment(
        [center - vec2(0.0, inner), center + vec2(0.0, inner)],
        faint,
    );

    let (mean, x, y) = to_plane(*rgb);
    let scale = inner / range;
    let v = turn(vec2(x, y), TURN);
    let puck = center + clamp_len(vec2(v.x * scale, -v.y * scale), inner);
    // The puck is the same fraction of the wheel at any size (3.6 points at the usual one).
    painter.circle_filled(
        puck,
        (radius * 0.0777).max(2.2),
        Color32::from_rgb(222, 222, 224),
    );

    let rect = egui::Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
    let response = ui
        .interact(rect, id, Sense::click_and_drag())
        .on_hover_text("Drag to tint, double-click to reset");
    let mut changed = false;
    if response.double_clicked() {
        *rgb = from_plane(mean, 0.0, 0.0);
        if (mean - neutral).abs() < 1e-6 {
            *rgb = [neutral; 3];
        }
        changed = true;
    } else if response.dragged() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            let d = turn(
                clamp_len(
                    vec2((p.x - center.x) / scale, -(p.y - center.y) / scale),
                    range,
                ),
                -TURN,
            );
            *rgb = from_plane(mean, d.x, d.y);
            changed = true;
        }
    }
    changed
}

/// The master value of a wheel: the mean of its channels.
pub(crate) fn master(rgb: [f32; 3]) -> f32 {
    to_plane(rgb).0
}

/// Moves all three channels so that their mean becomes `value`, keeping the tint.
pub(crate) fn set_master(rgb: &mut [f32; 3], value: f32) {
    let d = value - master(*rgb);
    rgb.iter_mut().for_each(|c| *c += d);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_round_trip() {
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.3, 0.1, 0.7],
            [1.2, 0.8, 0.9],
            [-0.1, 0.2, 0.05],
        ] {
            let (m, x, y) = to_plane(rgb);
            let back = from_plane(m, x, y);
            for c in 0..3 {
                assert!((back[c] - rgb[c]).abs() < 1e-5, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn grey_has_no_tint_and_red_points_right() {
        let (m, x, y) = to_plane([0.5; 3]);
        assert_eq!((m, x, y), (0.5, 0.0, 0.0));
        let (_, x, y) = to_plane([1.0, 0.0, 0.0]);
        assert!(x > 0.9 && y.abs() < 1e-6);
        let (_, _, y) = to_plane([0.0, 1.0, 0.0]);
        assert!(y > 0.0, "green is up");
    }

    #[test]
    fn the_turned_wheel_puts_red_near_the_top() {
        // Pure red in the plane, turned for the screen, points up and a little left.
        let (_, x, y) = to_plane([1.0, 0.0, 0.0]);
        let v = turn(vec2(x, y), TURN);
        assert!(v.y > 0.9 && v.x < 0.0);
        // Turning back gives the plane again.
        let back = turn(v, -TURN);
        assert!((back.x - x).abs() < 1e-5 && (back.y - y).abs() < 1e-5);
    }
}
