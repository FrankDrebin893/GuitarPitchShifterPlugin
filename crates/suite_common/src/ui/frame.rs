use nih_plug_egui::egui::epaint::TextShape;
use nih_plug_egui::egui::{
    self, vec2, Align2, Color32, Context, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2,
};

use super::theme::*;
use super::widgets::badge;

// Gap between the window edge and the enclosure
const BENCH_MARGIN: f32 = 10.0;
const CORNER_RADIUS: f32 = 24.0;
// Distance from the enclosure edge to the printing
const PRINT_INSET: f32 = 30.0;

const BAND_HEIGHT: f32 = 46.0;
const BAND_ANGLE_DEG: f32 = -4.0;

/// Drawing printed at both ends of the name band
#[derive(Clone, Copy, PartialEq)]
pub enum Ornament {
    /// Chevrons pointing up on the left and down on the right
    Arrows,
    /// Drum head seen from above
    Rings,
}

/// What makes one plugin's pedal differ from the next
pub struct PedalStyle {
    /// Printed on the name band
    pub name: &'static str,
    /// Model number printed in the lower left corner
    pub model: &'static str,
    pub paint: Color32,
    pub ornament: Ornament,
    /// Captions of the jacks, top left and top right
    pub jacks: [&'static str; 2],
    /// Vertical centre of the name band, from the top of the window
    pub band_y: f32,
}

/// Draw the pedal and run `add_controls` on top of it.
///
/// The closure gets the window's top left corner. Controls are placed relative to it.
pub fn pedal(ctx: &Context, style: &PedalStyle, add_controls: impl FnOnce(&mut Ui, Pos2)) {
    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(BENCH))
        .show(ctx, |ui| {
            let window = ui.max_rect();
            let enclosure = window.shrink(BENCH_MARGIN);
            let painter = ui.painter();

            // The darker rectangle shows below the paint as the lower edge of the box
            painter.rect_filled(enclosure, CORNER_RADIUS, style.paint.gamma_multiply(0.72));
            let top = Rect::from_min_max(enclosure.min, enclosure.max - vec2(0.0, 5.0));
            painter.rect_filled(top, CORNER_RADIUS, style.paint);
            painter.rect_stroke(
                top.shrink(1.0),
                CORNER_RADIUS - 1.0,
                Stroke::new(1.5, Color32::from_white_alpha(45)),
                StrokeKind::Inside,
            );

            for (corner, slot_angle) in [
                (top.left_top() + vec2(17.0, 17.0), 35.0_f32),
                (top.right_top() + vec2(-17.0, 17.0), -50.0),
                (top.left_bottom() + vec2(17.0, -17.0), -50.0),
                (top.right_bottom() + vec2(-17.0, -17.0), 35.0),
            ] {
                screw(painter, corner, slot_angle.to_radians());
            }

            let jack_y = top.top() + 29.0;
            let [left_jack, right_jack] = style.jacks;
            jack_label(painter, Pos2::new(top.left() + PRINT_INSET, jack_y), left_jack, Align2::LEFT_CENTER);
            jack_label(painter, Pos2::new(top.right() - PRINT_INSET, jack_y), right_jack, Align2::RIGHT_CENTER);

            name_band(painter, top, window.top() + style.band_y, style);

            let print_bottom = top.bottom() - 24.0;
            spaced_text(
                painter,
                Pos2::new(top.left() + PRINT_INSET, print_bottom),
                Align2::LEFT_BOTTOM,
                style.model,
                display(15.0),
                SILK,
                0.16,
            );
            spaced_text(
                painter,
                Pos2::new(top.left() + PRINT_INSET, print_bottom - 16.0),
                Align2::LEFT_BOTTOM,
                "MODEL",
                display(12.0),
                SILK,
                0.16,
            );
            badge(painter, Pos2::new(top.right() - PRINT_INSET, print_bottom), 22.0);

            add_controls(ui, window.min);
        });
}

/// Jack caption with an arrow for the signal direction, which runs right to left as on a pedal.
/// The arrow sits on the outer side: `anchor` is `LEFT_CENTER` or `RIGHT_CENTER`.
fn jack_label(painter: &Painter, pos: Pos2, text: &str, anchor: Align2) {
    let arrow_width = 8.0;
    let gap = 6.0;
    let (arrow_x, text_pos) = if anchor == Align2::LEFT_CENTER {
        (pos.x, pos + vec2(arrow_width + gap, 0.0))
    } else {
        (pos.x - arrow_width, pos - vec2(arrow_width + gap, 0.0))
    };
    let tip = Pos2::new(arrow_x, pos.y - 1.0);
    painter.add(Shape::convex_polygon(
        vec![tip, tip + vec2(arrow_width, -4.5), tip + vec2(arrow_width, 4.5)],
        SILK,
        Stroke::NONE,
    ));
    spaced_text(painter, text_pos, anchor, &text.to_uppercase(), display(13.0), SILK, 0.16);
}

fn screw(painter: &Painter, center: Pos2, slot_angle: f32) {
    painter.circle_filled(center + vec2(0.0, 1.0), 6.5, SHADOW);
    painter.circle_filled(center, 6.0, CHROME);
    painter.circle_stroke(center, 6.0, Stroke::new(1.0, Color32::from_black_alpha(110)));
    let slot = Vec2::angled(slot_angle) * 4.0;
    painter.line_segment([center - slot, center + slot], Stroke::new(1.5, INK));
}

/// Tilted cream band across the enclosure, with the plugin name in heavy letters
fn name_band(painter: &Painter, enclosure: Rect, center_y: f32, style: &PedalStyle) {
    let angle = BAND_ANGLE_DEG.to_radians();
    let center = Pos2::new(enclosure.center().x, center_y);
    // Height of a line through the band's centre at horizontal position x, shifted by `offset`
    let edge = |offset: f32| {
        let rise = angle.tan() * enclosure.width() / 2.0;
        [
            Pos2::new(enclosure.left(), center.y - rise + offset),
            Pos2::new(enclosure.right(), center.y + rise + offset),
        ]
    };

    let half = BAND_HEIGHT / 2.0;
    let [top_left, top_right] = edge(-half);
    let [bottom_left, bottom_right] = edge(half);
    painter.add(Shape::convex_polygon(
        vec![top_left, top_right, bottom_right, bottom_left],
        SILK,
        Stroke::NONE,
    ));
    // Ink line along each edge, then a thin cream line outside it
    for side in [-1.0, 1.0] {
        painter.line_segment(edge(side * (half - 1.5)), Stroke::new(3.0, INK));
        painter.line_segment(edge(side * (half + 6.5)), Stroke::new(3.0, SILK));
    }

    // Name, with a second print in the paint colour offset behind it
    let name = style.name.to_uppercase();
    let galley = layout(painter, &name, model(32.0), INK, 0.03);
    let along = Vec2::angled(angle);
    let down = vec2(-along.y, along.x);
    let text_size = galley.size();
    let origin = center - along * (text_size.x / 2.0) - down * (text_size.y / 2.0 - 1.0);
    painter.add(
        TextShape::new(origin + vec2(2.5, 2.5), galley.clone(), style.paint)
            .with_override_text_color(style.paint)
            .with_angle(angle),
    );
    painter.add(TextShape::new(origin, galley, INK).with_angle(angle));

    let ornament_distance = text_size.x / 2.0 + 30.0;
    let left = center - along * ornament_distance;
    let right = center + along * ornament_distance;
    match style.ornament {
        Ornament::Arrows => {
            chevrons(painter, left, -1.0);
            chevrons(painter, right, 1.0);
        }
        Ornament::Rings => {
            rings(painter, left);
            rings(painter, right);
        }
    }
}

/// Three stacked chevrons. `direction` is -1 for up, 1 for down
fn chevrons(painter: &Painter, center: Pos2, direction: f32) {
    let stroke = Stroke::new(3.2, INK);
    for row in [-7.0, 0.0, 7.0] {
        let tip = center + vec2(0.0, row + direction * 4.0);
        let left = center + vec2(-9.0, row - direction * 4.0);
        let right = center + vec2(9.0, row - direction * 4.0);
        painter.add(Shape::line(vec![left, tip, right], stroke));
    }
}

fn rings(painter: &Painter, center: Pos2) {
    painter.circle_stroke(center, 12.5, Stroke::new(2.6, INK));
    painter.circle_stroke(center, 7.0, Stroke::new(2.6, INK));
    painter.circle_filled(center, 2.4, INK);
}
