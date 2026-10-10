use nih_plug::prelude::{Param, ParamSetter};
use nih_plug_egui::egui::{
    vec2, Align2, Color32, CornerRadius, Id, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
};

use super::theme::*;

// The knob travels 270 degrees, from bottom-left over the top to bottom-right
const KNOB_START_ANGLE: f32 = std::f32::consts::PI * 0.75;
const KNOB_SWEEP: f32 = std::f32::consts::PI * 1.5;

// Normalized parameter change per pixel of vertical drag. Shift slows it down.
const DRAG_SENSITIVITY: f32 = 0.005;
const FINE_DRAG_FACTOR: f32 = 0.1;

// Knobs at least this large get the larger label and value
const BIG_KNOB_RADIUS: f32 = 40.0;
const SMALL_KNOB_RADIUS: f32 = 24.0;

const LABEL_SPACING: f32 = 0.14;

// Diameter of a footswitch's washer
const FOOTSWITCH_SIZE: f32 = 74.0;
const SMALL_FOOTSWITCH_SIZE: f32 = 50.0;

fn polar(center: Pos2, radius: f32, angle: f32) -> Pos2 {
    Pos2::new(center.x + angle.cos() * radius, center.y + angle.sin() * radius)
}

/// Fluted amp knob with its printed scale. `scale` names the start, middle and end of the travel.
pub fn knob(painter: &Painter, center: Pos2, radius: f32, normalized: f32, scale: Option<[&str; 3]>) {
    // Scale printed on the enclosure
    for tick in 0..=10 {
        let angle = KNOB_START_ANGLE + KNOB_SWEEP * tick as f32 / 10.0;
        let major = tick % 5 == 0;
        let (length, width) = if major { (13.0, 2.2) } else { (10.0, 1.3) };
        painter.line_segment(
            [polar(center, radius + 6.0, angle), polar(center, radius + length, angle)],
            Stroke::new(width, SILK),
        );
    }
    if let Some(scale) = scale {
        for (i, text) in scale.iter().enumerate() {
            let angle = KNOB_START_ANGLE + KNOB_SWEEP * i as f32 / 2.0;
            painter.text(polar(center, radius + 24.0, angle), Align2::CENTER_CENTER, text, display(15.0), SILK);
        }
    }

    painter.circle_filled(center + vec2(1.5, 4.0), radius + 1.0, SHADOW);

    // Skirt: a ring of round flutes around a disc
    let flutes = (radius * 0.8).round().max(8.0) as usize;
    let flute_radius = std::f32::consts::PI * radius / flutes as f32 * 0.62;
    painter.circle_filled(center, radius - flute_radius * 0.5, KNOB_SKIRT);
    for flute in 0..flutes {
        let angle = std::f32::consts::TAU * flute as f32 / flutes as f32;
        painter.circle_filled(polar(center, radius - flute_radius, angle), flute_radius, KNOB_SKIRT);
    }

    painter.circle_filled(center, radius * 0.7, KNOB_CAP);
    painter.circle_stroke(center, radius * 0.7, Stroke::new(1.2, KNOB_CAP_EDGE));
    painter.circle_filled(center, radius * 0.52, KNOB_CAP_TOP);

    let angle = KNOB_START_ANGLE + KNOB_SWEEP * normalized.clamp(0.0, 1.0);
    painter.line_segment(
        [polar(center, radius * 0.12, angle), polar(center, radius * 0.94, angle)],
        Stroke::new((radius * 0.1).max(2.6), SILK),
    );
}

/// Knob bound to a parameter, with its name and value below.
///
/// Drag up and down to change the parameter, hold Shift for fine steps, double-click for the default.
pub fn param_knob<P: Param>(
    ui: &mut Ui,
    setter: &ParamSetter,
    param: &P,
    center: Pos2,
    radius: f32,
    label: &str,
    scale: Option<[&str; 3]>,
) -> Response {
    let id = Id::new(("param_knob", param.name()));
    let rect = Rect::from_center_size(center, Vec2::splat((radius + 6.0) * 2.0));
    let response = ui.interact(rect, id, Sense::click_and_drag());

    let normalized = param.unmodulated_normalized_value();

    // The drag position is kept apart from the parameter, so that stepped parameters
    // do not snap back to their current step on every frame
    if response.drag_started() {
        setter.begin_set_parameter(param);
        ui.data_mut(|data| data.insert_temp(id, normalized));
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let sensitivity = if fine { DRAG_SENSITIVITY * FINE_DRAG_FACTOR } else { DRAG_SENSITIVITY };
        let delta = -response.drag_delta().y * sensitivity;
        let position = ui.data_mut(|data| {
            let position = data.get_temp_mut_or(id, normalized);
            *position = (*position + delta).clamp(0.0, 1.0);
            *position
        });
        setter.set_parameter_normalized(param, position);
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if response.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(param, param.default_normalized_value());
        setter.end_set_parameter(param);
    }

    let painter = ui.painter();
    knob(painter, center, radius, normalized, scale);

    let (label_size, value_size) = if radius >= BIG_KNOB_RADIUS {
        (19.0, 14.0)
    } else if radius < SMALL_KNOB_RADIUS {
        (13.0, 10.0)
    } else {
        (15.0, 11.0)
    };
    let label_rect = spaced_text(
        painter,
        Pos2::new(center.x, center.y + radius + 18.0),
        Align2::CENTER_TOP,
        &label.to_uppercase(),
        display(label_size),
        SILK,
        LABEL_SPACING,
    );
    let value = param.normalized_value_to_string(normalized, true);
    label_tape(painter, Pos2::new(center.x, label_rect.bottom() + 2.0), &value, value_size);

    response
}

/// Value read-out: embossed label tape. `top_center` is the middle of its top edge.
pub fn label_tape(painter: &Painter, top_center: Pos2, text: &str, size: f32) {
    let galley = layout(painter, &text.to_uppercase(), mono(size), TAPE_TEXT, 0.1);
    let padding = vec2(size * 0.6, 2.0);
    let rect = Rect::from_min_size(
        Pos2::new(top_center.x - galley.size().x / 2.0 - padding.x, top_center.y),
        galley.size() + padding * 2.0,
    );
    painter.rect_filled(rect.translate(vec2(0.0, 2.0)), 2.0, SHADOW);
    painter.rect_filled(rect, 2.0, TAPE);
    painter.galley(rect.min + padding, galley, TAPE_TEXT);
}

/// Chrome stomp switch. Returns the click response.
pub fn footswitch(ui: &mut Ui, center: Pos2, id_source: &str) -> Response {
    stomp_switch(ui, center, id_source, FOOTSWITCH_SIZE)
}

/// The smaller stomp switch of a head or a mini pedal. Returns the click response.
pub fn small_footswitch(ui: &mut Ui, center: Pos2, id_source: &str) -> Response {
    stomp_switch(ui, center, id_source, SMALL_FOOTSWITCH_SIZE)
}

/// Stomp switch `size` across. The measures in here are those of the full-size one
fn stomp_switch(ui: &mut Ui, center: Pos2, id_source: &str, size: f32) -> Response {
    let scale = size / FOOTSWITCH_SIZE;
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    let response = ui.interact(rect, Id::new(("footswitch", id_source)), Sense::click());
    let pressed = response.is_pointer_button_down_on();

    let painter = ui.painter();
    let ring = Stroke::new(1.0, Color32::from_black_alpha(110));
    let disc = |center: Pos2, offset: Vec2, radius: f32, color: Color32| {
        painter.circle_filled(center + offset * scale, radius * scale, color);
    };
    disc(center, vec2(0.0, 8.0), 38.0, SHADOW);
    // Washer, nut, then the cap
    disc(center, Vec2::ZERO, 37.0, CHROME_LIGHT);
    painter.circle_stroke(center, 37.0 * scale, ring);
    disc(center, Vec2::ZERO, 32.0, CHROME);
    painter.circle_stroke(center, 32.0 * scale, ring);

    let cap = if pressed { center + vec2(0.0, 1.5) * scale } else { center };
    disc(cap, Vec2::ZERO, 26.0, CHROME_DARK);
    disc(cap, vec2(-2.0, -2.5), 22.0, CHROME);
    disc(cap, vec2(-4.5, -5.5), 15.0, CHROME_LIGHT);
    if !pressed {
        disc(cap, vec2(-7.0, -8.5), 7.0, CHROME_SHINE);
    }
    painter.circle_stroke(cap, 26.0 * scale, ring);

    response
}

/// Indicator light in a chrome bezel
pub fn led(painter: &Painter, center: Pos2, on: bool) {
    if on {
        for (radius, alpha) in [(15.0, 22), (11.5, 40), (8.5, 70)] {
            painter.circle_filled(center, radius, LED_ON.gamma_multiply(alpha as f32 / 255.0));
        }
    }
    painter.circle_filled(center, 8.5, CHROME_LIGHT);
    painter.circle_stroke(center, 8.5, Stroke::new(1.0, Color32::from_black_alpha(110)));
    painter.circle_filled(center, 5.5, if on { LED_ON } else { LED_OFF });
    if on {
        painter.circle_filled(center + vec2(-1.2, -1.5), 2.0, Color32::from_rgb(255, 208, 196));
    }
}

/// Screen-printed caption, centred on `center`
pub fn silk_label(painter: &Painter, center: Pos2, text: &str, size: f32) -> Rect {
    spaced_text(painter, center, Align2::CENTER_CENTER, &text.to_uppercase(), display(size), SILK, 0.16)
}

/// Screen-printed frame around a group of controls, with its title set into the top line
pub fn silk_frame(painter: &Painter, rect: Rect, title: &str, paint: Color32) {
    painter.rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, SILK), StrokeKind::Middle);

    let galley = layout(painter, &title.to_uppercase(), display(14.0), SILK, 0.2);
    let title_rect = Rect::from_min_size(
        Pos2::new(rect.left() + 30.0, rect.top() - galley.size().y / 2.0),
        galley.size(),
    );
    painter.rect_filled(title_rect.expand2(vec2(8.0, 0.0)), 0.0, paint);
    painter.galley(title_rect.min, galley, SILK);
}

/// The HØJT logo on its black plate. The Ø is a knob and its slash the pointer.
/// `right_bottom` is the plate's lower right corner.
pub fn badge(painter: &Painter, right_bottom: Pos2, size: f32) {
    let spacing = 0.1;
    let h = layout(painter, "H", display(size), SILK, spacing);
    let jt = layout(painter, "JT", display(size), SILK, spacing);
    let o_width = size * 0.62;
    let gap = size * 0.13;

    let text_width = h.size().x + gap + o_width + gap + jt.size().x;
    let padding = vec2(size * 0.42, size * 0.2);
    let plate = Rect::from_min_max(
        Pos2::new(right_bottom.x - text_width - padding.x * 2.0, right_bottom.y - size - padding.y * 2.0),
        right_bottom,
    );
    painter.rect_filled(plate.translate(vec2(0.0, 3.0)), 6.0, SHADOW);
    painter.rect_filled(plate, 6.0, PANEL);
    painter.rect_stroke(plate.shrink(2.0), 4.0, Stroke::new(1.5, SILK), StrokeKind::Inside);

    // Bebas Neue's capitals sit a little above the middle of the line
    let mid_y = plate.center().y;
    let text_y = mid_y - h.size().y / 2.0 + size * 0.07;
    let mut x = plate.left() + padding.x;
    let h_width = h.size().x;
    painter.galley(Pos2::new(x, text_y), h, SILK);
    x += h_width + gap;

    let o_center = Pos2::new(x + o_width / 2.0, mid_y);
    painter.circle_stroke(o_center, size * 0.3, Stroke::new(size * 0.105, SILK));
    let slash_angle = -55.0_f32.to_radians();
    painter.line_segment(
        [polar(o_center, size * 0.5, slash_angle + std::f32::consts::PI), polar(o_center, size * 0.5, slash_angle)],
        Stroke::new(size * 0.085, BRAND_ORANGE),
    );
    x += o_width + gap;

    painter.galley(Pos2::new(x, text_y), jt, SILK);
}
