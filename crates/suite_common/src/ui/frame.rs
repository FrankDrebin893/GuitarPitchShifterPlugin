use nih_plug_egui::egui::epaint::TextShape;
use nih_plug_egui::egui::{
    self, vec2, Align2, Color32, Context, CursorIcon, Id, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, Ui, Vec2,
};

use super::art::{self, Motif};
use super::hardware;
use super::raster;
use super::theme::*;
use super::widgets::badge;

/// Gap between the window edge and an enclosure, and between two enclosures of a rig
pub const BENCH_MARGIN: f32 = 10.0;
const CORNER_RADIUS: f32 = 24.0;
// Distance from the enclosure edge to the printing
const PRINT_INSET: f32 = 30.0;
// The darker lower edge of the box that shows below the top face
const EDGE_HEIGHT: f32 = 5.0;
const SCREW_INSET: f32 = 17.0;

const BAND_HEIGHT: f32 = 46.0;
const BAND_ANGLE_DEG: f32 = -4.0;
// Over the width of a head the pedal's tilt would rise by more than the band is tall
const HEAD_BAND_ANGLE_DEG: f32 = -1.5;

// Mini pedals are small boxes: tighter corners, smaller screws
const MINI_CORNER_RADIUS: f32 = 14.0;
const MINI_SCREW_INSET: f32 = 13.0;
const MINI_SCREW_SCALE: f32 = 0.75;
// Centre of the title, from the top of the mini pedal
const MINI_TITLE_Y: f32 = 30.0;
const MINI_TITLE_SIZE: f32 = 24.0;
// The longest titles of slim pedals have seven letters
const SLIM_TITLE_SIZE: f32 = 21.0;
// The part of a mini pedal's top face that it is moved by: its title, between the screws
const GRIP_TOP: f32 = 12.0;
const GRIP_BOTTOM: f32 = 48.0;
const GRIP_INSET: f32 = 6.0;

/// Width of a `slim_pedal`
pub const SLIM_PEDAL_WIDTH: f32 = 110.0;
/// On a slim pedal, from the centre of its top edge: the LED, the two knobs and the switch.
/// The heights are those of a mini pedal with three knobs, so a row of both lines up. The
/// LED has the place of the upper left knob
pub const SLIM_LED: Vec2 = vec2(-33.0, 71.0);
pub const SLIM_KNOBS: [Vec2; 2] = [vec2(19.0, 71.0), vec2(-19.0, 172.0)];
pub const SLIM_KNOB_RADIUS: f32 = 17.0;
pub const SLIM_SWITCH: Vec2 = vec2(0.0, 273.0);

// The amp's place in a row of pedals: two patch cables up to the head, and a tag on them
const CABLE_INSET: f32 = 12.0;
const CABLE_WIDTH: f32 = 5.0;
// Height of the plugs in the pedals' sides and of the tag, from the top of the pedals
const PLUG_Y: f32 = 169.0;
const TAG_Y: f32 = 71.0;
const TAG_HEIGHT: f32 = 20.0;
// How far a pedal that is held is above the bench
const LIFT: f32 = 4.0;

/// Drawing printed at both ends of the name band
#[derive(Clone, Copy, PartialEq)]
pub enum Ornament {
    /// Chevrons pointing up on the left and down on the right
    Arrows,
    /// Drum head seen from above
    Rings,
    /// Lightning bolts, mirrored
    Bolts,
    /// The four strings of a bass, thick to thin
    Strings,
    /// Three slow waves
    Waves,
    /// A large and a small four-pointed glint, mirrored
    Glints,
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
    /// Vertical centre of the name band: from the top of the window for `pedal`,
    /// from the top of its rectangle for `head`
    pub band_y: f32,
    /// Picture printed on the top face under everything else: one of `artwork`'s, or `None`
    pub art: Option<&'static [u8]>,
}

/// What makes one mini pedal of a rig differ from the next
#[derive(Clone, Copy)]
pub struct MiniStyle {
    /// Printed at the top, in capitals
    pub title: &'static str,
    pub paint: Color32,
    /// Painted under the controls, in darker tones of the paint and a little cream
    pub motif: Motif,
}

/// Draw the pedal and run `add_controls` on top of it.
///
/// The closure gets the window's top left corner. Controls are placed relative to it.
pub fn pedal(ctx: &Context, style: &PedalStyle, add_controls: impl FnOnce(&mut Ui, Pos2)) {
    rig(ctx, |ui, origin| {
        let window = ui.max_rect();
        let painter = ui.painter();
        let top = enclosure(painter, window.shrink(BENCH_MARGIN), style.paint);
        art_under_print(painter, top, style);
        print(painter, top, window.top() + style.band_y, BAND_ANGLE_DEG, style);
        add_controls(ui, origin);
    });
}

/// The bare bench, for a window that holds several enclosures: a `head` and `mini_pedal`s.
///
/// The closure gets the window's top left corner. Enclosures and controls are placed relative to it.
pub fn rig(ctx: &Context, add_contents: impl FnOnce(&mut Ui, Pos2)) {
    egui::CentralPanel::default().frame(egui::Frame::default().fill(BENCH)).show(ctx, |ui| {
        let origin = ui.max_rect().min;
        add_contents(ui, origin);
    });
}

fn art_under_print(painter: &Painter, top: Rect, style: &PedalStyle) {
    if let Some(art) = style.art {
        // The screws are on top of it: they are drawn with the box, so draw the art first
        super::artwork::paint_art(painter, top, CORNER_RADIUS, art);
        screws(painter, top, SCREW_INSET, 1.0);
    }
}

/// Amp head: a wide enclosure filling `rect`, with everything a pedal has printed on it.
/// The name band is tilted less than on a pedal. Returns the top face.
pub fn head(ui: &Ui, rect: Rect, style: &PedalStyle) -> Rect {
    let painter = ui.painter();
    let top = enclosure(painter, rect, style.paint);
    art_under_print(painter, top, style);
    print(painter, top, rect.top() + style.band_y, HEAD_BAND_ANGLE_DEG, style);
    top
}

/// Small pedal of a rig, filling `rect`: an enclosure in its own paint with its art and
/// its title printed at the top. No band, model number or logo: the head carries those.
/// Returns the top face.
pub fn mini_pedal(ui: &Ui, rect: Rect, style: &MiniStyle) -> Rect {
    titled_box(ui.painter(), rect, style, MINI_TITLE_SIZE)
}

/// Mini pedal `SLIM_PEDAL_WIDTH` wide, for two knobs one above the other (`SLIM_KNOBS`).
/// Its title is a little smaller, so seven letters fit. Returns the top face.
pub fn slim_pedal(ui: &Ui, rect: Rect, style: &MiniStyle) -> Rect {
    titled_box(ui.painter(), rect, style, SLIM_TITLE_SIZE)
}

fn titled_box(painter: &Painter, rect: Rect, style: &MiniStyle, title_size: f32) -> Rect {
    let top = painted_box(painter, rect, style.paint, MINI_CORNER_RADIUS);
    // The art goes under the screws, the title and the controls
    art::motif(painter, top, style.paint, style.motif);
    screws(painter, top, MINI_SCREW_INSET, MINI_SCREW_SCALE);
    spaced_text(
        painter,
        Pos2::new(top.center().x, top.top() + MINI_TITLE_Y),
        Align2::CENTER_CENTER,
        &style.title.to_uppercase(),
        display(title_size),
        SILK,
        0.16,
    );
    top
}

/// The title of a mini pedal as the handle it is moved by. `top` is the pedal's top face.
/// The pointer is an open hand over it and a closed one while it is held. Returns the
/// drag response: the caller moves the pedal.
pub fn title_grip(ui: &mut Ui, top: Rect, id_source: &str) -> Response {
    let grip = Rect::from_min_max(
        Pos2::new(top.left() + GRIP_INSET, top.top() + GRIP_TOP),
        Pos2::new(top.right() - GRIP_INSET, top.top() + GRIP_BOTTOM),
    );
    let response = ui.interact(grip, Id::new(("title_grip", id_source)), Sense::drag());
    if response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    response
}

/// Where a mini pedal that is held by its title is drawn: `rect` a little higher, with its
/// shadow on what is under it. Call it before drawing the pedal in the rectangle it returns.
pub fn lifted(painter: &Painter, rect: Rect) -> Rect {
    painter.rect_filled(
        rect.translate(vec2(0.0, LIFT)).expand(3.0),
        MINI_CORNER_RADIUS + 3.0,
        SHADOW,
    );
    rect.translate(vec2(0.0, -LIFT))
}

/// The amp's place in a row of mini pedals, on the bench between two of them: a patch cable
/// from the pedal on the left up to the head and one from the head down to the pedal on the
/// right, with a tag on them. `gap` is the bench between the two pedals, over their whole
/// height; `head_bottom` is the lower edge of the head. Give it 40 px of width.
pub fn amp_marker(painter: &Painter, gap: Rect, head_bottom: f32) {
    let plug_y = gap.top() + PLUG_Y;
    let ring = Stroke::new(1.0, Color32::from_black_alpha(110));
    for (edge, side) in [(gap.left(), 1.0), (gap.right(), -1.0)] {
        let x = edge + side * CABLE_INSET;
        painter.line_segment(
            [Pos2::new(x, head_bottom), Pos2::new(x, plug_y)],
            Stroke::new(CABLE_WIDTH, CABLE),
        );
        // The sleeve where the cable goes in under the head
        let sleeve = Rect::from_center_size(Pos2::new(x, head_bottom + 5.0), vec2(9.0, 10.0));
        painter.rect_filled(sleeve, 1.5, CHROME_DARK);
        // An angled plug in the side of the pedal: its barrel, and the housing the cable leaves upwards
        let barrel = Rect::from_min_max(
            Pos2::new(edge.min(x), plug_y - 3.5),
            Pos2::new(edge.max(x), plug_y + 3.5),
        );
        painter.rect_filled(barrel, 0.0, CHROME_LIGHT);
        let housing = Rect::from_center_size(Pos2::new(x, plug_y), vec2(13.0, 17.0));
        painter.rect_filled(housing.translate(vec2(0.0, 2.0)), 3.0, SHADOW);
        painter.rect_filled(housing, 3.0, CHROME);
        painter.rect_stroke(housing, 3.0, ring, StrokeKind::Inside);
    }

    // Printed like the name band: ink on cream
    let tag = Rect::from_center_size(
        Pos2::new(gap.center().x, gap.top() + TAG_Y),
        vec2(gap.width() - 2.0 * CABLE_INSET + CABLE_WIDTH + 10.0, TAG_HEIGHT),
    );
    painter.rect_filled(tag.translate(vec2(0.0, 2.0)), 2.0, SHADOW);
    painter.rect_filled(tag, 2.0, SILK);
    let galley = layout(painter, "AMP", display(14.0), INK, 0.16);
    // The letter spacing also follows the last letter: leave it out of the centring
    let text_width = galley.size().x - 14.0 * 0.16;
    painter.galley(
        Pos2::new(tag.center().x - text_width / 2.0, tag.center().y - galley.size().y / 2.0 + 1.0),
        galley,
        INK,
    );
}

/// Painted box with a screw in each corner, filling `rect`. Returns the top face,
/// which is `rect` without the lower edge of the box.
pub fn enclosure(painter: &Painter, rect: Rect, paint: Color32) -> Rect {
    shell(painter, rect, paint, CORNER_RADIUS, SCREW_INSET, 1.0)
}

fn shell(
    painter: &Painter,
    rect: Rect,
    paint: Color32,
    radius: f32,
    screw_inset: f32,
    screw_scale: f32,
) -> Rect {
    let top = painted_box(painter, rect, paint, radius);
    screws(painter, top, screw_inset, screw_scale);
    top
}

/// The box without its screws. Returns the top face
fn painted_box(painter: &Painter, rect: Rect, paint: Color32, radius: f32) -> Rect {
    raster::paint_box(painter, rect, radius, EDGE_HEIGHT, paint);
    Rect::from_min_max(rect.min, rect.max - vec2(0.0, EDGE_HEIGHT))
}

/// A screw in each corner of the top face `top`
fn screws(painter: &Painter, top: Rect, screw_inset: f32, screw_scale: f32) {
    for (corner, slot_angle) in [
        (top.left_top() + vec2(screw_inset, screw_inset), 35.0_f32),
        (top.right_top() + vec2(-screw_inset, screw_inset), -50.0),
        (top.left_bottom() + vec2(screw_inset, -screw_inset), -50.0),
        (top.right_bottom() + vec2(-screw_inset, -screw_inset), 35.0),
    ] {
        screw(painter, corner, slot_angle.to_radians(), screw_scale);
    }
}

/// Everything printed on the top face `top`: jack captions, name band, model number, logo
fn print(
    painter: &Painter,
    top: Rect,
    band_center_y: f32,
    band_angle_deg: f32,
    style: &PedalStyle,
) {
    let jack_y = top.top() + 29.0;
    let [left_jack, right_jack] = style.jacks;
    jack_label(
        painter,
        Pos2::new(top.left() + PRINT_INSET, jack_y),
        left_jack,
        Align2::LEFT_CENTER,
    );
    jack_label(
        painter,
        Pos2::new(top.right() - PRINT_INSET, jack_y),
        right_jack,
        Align2::RIGHT_CENTER,
    );

    name_band(painter, top, band_center_y, band_angle_deg, style);

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

/// Slotted screw head. `scale` is 1 on a pedal
fn screw(painter: &Painter, center: Pos2, slot_angle: f32, scale: f32) {
    raster::paint_screw(painter, center, slot_angle, scale);
}

/// Tilted cream band across the enclosure, with the plugin name in heavy letters
fn name_band(
    painter: &Painter,
    enclosure: Rect,
    center_y: f32,
    angle_deg: f32,
    style: &PedalStyle,
) {
    let angle = angle_deg.to_radians();
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
    let corners = [top_left, top_right, bottom_right, bottom_left];
    painter.add(Shape::convex_polygon(
        corners.iter().map(|corner| *corner + vec2(0.0, 2.0)).collect(),
        SHADOW,
        Stroke::NONE,
    ));
    hardware::paint_paper(painter, corners, vec2(enclosure.width(), BAND_HEIGHT));
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
        Ornament::Bolts => {
            bolt(painter, left, 1.0);
            bolt(painter, right, -1.0);
        }
        Ornament::Strings => {
            strings(painter, left);
            strings(painter, right);
        }
        Ornament::Waves => {
            waves(painter, left);
            waves(painter, right);
        }
        Ornament::Glints => {
            glints(painter, left, 1.0);
            glints(painter, right, -1.0);
        }
    }
}

/// Four strings, the thickest at the top
fn strings(painter: &Painter, center: Pos2) {
    for (row, width) in [(-10.5, 4.0), (-3.5, 3.2), (3.5, 2.4), (10.5, 1.6)] {
        let at = center + vec2(0.0, row);
        painter.line_segment([at - vec2(13.0, 0.0), at + vec2(13.0, 0.0)], Stroke::new(width, INK));
    }
}

/// Three waves of one and a half turns each
fn waves(painter: &Painter, center: Pos2) {
    for row in [-9.0, 0.0, 9.0] {
        let points = (0..=24)
            .map(|step| {
                let along = step as f32 / 24.0;
                center
                    + vec2(
                        (along - 0.5) * 28.0,
                        row - (along * 3.0 * std::f32::consts::PI).sin() * 3.2,
                    )
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(2.8, INK)));
    }
}

/// A four-pointed star with hollow sides
fn glint(painter: &Painter, center: Pos2, radius: f32) {
    let waist = radius * 0.22;
    // Four kites around the centre: each is convex, the star as a whole is not
    for quarter in 0..4 {
        let turn = |x: f32, y: f32| match quarter {
            0 => center + vec2(x, y),
            1 => center + vec2(-y, x),
            2 => center + vec2(-x, -y),
            _ => center + vec2(y, -x),
        };
        painter.add(Shape::convex_polygon(
            vec![turn(0.0, 0.0), turn(waist, -waist), turn(radius, 0.0), turn(waist, waist)],
            INK,
            Stroke::NONE,
        ));
    }
}

/// `lean` is 1 with the small glint to the upper right, -1 mirrored
fn glints(painter: &Painter, center: Pos2, lean: f32) {
    glint(painter, center + vec2(-3.0 * lean, 3.0), 13.0);
    glint(painter, center + vec2(9.0 * lean, -10.0), 6.0);
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

/// Lightning bolt striking downwards. `lean` is 1 with the top to the right, -1 mirrored
fn bolt(painter: &Painter, center: Pos2, lean: f32) {
    let at = |x: f32, y: f32| center + vec2(x * lean, y);
    // Two slanted halves that share the short edge between the inner corners
    let (upper_inner, lower_inner) = (at(3.0, -3.5), at(-3.0, 3.5));
    for half in [
        [at(6.5, -16.0), at(-10.5, 3.5), lower_inner, upper_inner],
        [at(-6.5, 16.0), at(10.5, -3.5), upper_inner, lower_inner],
    ] {
        painter.add(Shape::convex_polygon(half.to_vec(), INK, Stroke::NONE));
    }
    // Covers the seam left by the soft edges of the two halves
    painter.line_segment([upper_inner, lower_inner], Stroke::new(1.5, INK));
}
