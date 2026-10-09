use nih_plug_egui::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

pub const BACKGROUND: Color32 = Color32::from_rgb(30, 30, 35);
pub const TITLE_TEXT: Color32 = Color32::from_rgb(200, 200, 210);
pub const VALUE_TEXT: Color32 = Color32::from_rgb(180, 180, 190);
pub const LABEL_TEXT: Color32 = Color32::from_rgb(120, 120, 130);
pub const ACCENT: Color32 = Color32::from_rgb(100, 180, 255);
pub const ACCENT_NEGATIVE: Color32 = Color32::from_rgb(255, 140, 100);
pub const DIAL_TRACK: Color32 = Color32::from_rgb(60, 60, 70);
pub const DIAL_CENTER: Color32 = Color32::from_rgb(45, 45, 55);
pub const DIAL_CENTER_STROKE: Color32 = Color32::from_rgb(70, 70, 80);

/// Draw a dial/knob widget
pub fn dial(ui: &mut Ui, radius: f32, value: f32, min: f32, max: f32, is_bipolar: bool) -> Response {
    let desired_size = Vec2::splat(radius * 2.0 + 10.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::drag());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let center = rect.center();

        // Normalize value to 0-1 range
        let normalized = (value - min) / (max - min);

        // Arc angles (from bottom-left to bottom-right, ~270 degrees)
        let start_angle = std::f32::consts::PI * 0.75;
        let end_angle = std::f32::consts::PI * 2.25;
        let angle_range = end_angle - start_angle;

        // Background arc
        draw_arc(
            painter,
            center,
            radius,
            start_angle,
            end_angle,
            Stroke::new(4.0, DIAL_TRACK),
        );

        // Value arc
        let value_color = if is_bipolar {
            if value >= 0.0 {
                ACCENT
            } else {
                ACCENT_NEGATIVE
            }
        } else {
            ACCENT
        };

        if is_bipolar {
            // Bipolar: draw from center
            let center_angle = start_angle + angle_range * 0.5;
            let value_angle = start_angle + angle_range * normalized;
            let (arc_start, arc_end) = if value_angle > center_angle {
                (center_angle, value_angle)
            } else {
                (value_angle, center_angle)
            };
            draw_arc(
                painter,
                center,
                radius,
                arc_start,
                arc_end,
                Stroke::new(4.0, value_color),
            );
        } else {
            // Unipolar: draw from start
            let value_angle = start_angle + angle_range * normalized;
            draw_arc(
                painter,
                center,
                radius,
                start_angle,
                value_angle,
                Stroke::new(4.0, value_color),
            );
        }

        // Indicator dot
        let indicator_angle = start_angle + angle_range * normalized;
        let indicator_pos = Pos2::new(
            center.x + indicator_angle.cos() * (radius - 12.0),
            center.y + indicator_angle.sin() * (radius - 12.0),
        );
        painter.circle_filled(indicator_pos, 5.0, Color32::WHITE);

        // Center circle
        painter.circle_filled(center, radius * 0.5, DIAL_CENTER);
        painter.circle_stroke(center, radius * 0.5, Stroke::new(2.0, DIAL_CENTER_STROKE));
    }

    response
}

/// Draw an arc using line segments
pub fn draw_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    stroke: Stroke,
) {
    let segments = 32;
    let angle_step = (end_angle - start_angle) / segments as f32;

    for i in 0..segments {
        let a1 = start_angle + angle_step * i as f32;
        let a2 = start_angle + angle_step * (i + 1) as f32;

        let p1 = Pos2::new(center.x + a1.cos() * radius, center.y + a1.sin() * radius);
        let p2 = Pos2::new(center.x + a2.cos() * radius, center.y + a2.sin() * radius);

        painter.line_segment([p1, p2], stroke);
    }
}
