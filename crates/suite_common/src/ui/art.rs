//! The art of a mini pedal: two coats and a print.
//!
//! Over the pedal's paint goes a second coat, a large motif in darker tones of the same
//! paint that may run under the controls, and on that a few cream lines in the clear paint
//! beside them. All of it keeps a margin to the edge of the face, like a screen print.
//! Nothing here moves or depends on anything but the face, so it costs no repaints.
//!
//! Places are in pixels from the top left corner of the face. A motif is drawn for one
//! layout of controls: 180 px wide with three knobs, with two, with the cabinet's stepper,
//! or `SLIM_PEDAL_WIDTH` wide.

use nih_plug_egui::egui::{vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

use super::theme::{shade, SILK};

// Plain paint between the art and the edge of the face. Also keeps it off the round corners
const MARGIN: f32 = 5.0;
// How much darker than the paint the second coat is
const COAT: f32 = 0.3;
// Width of the cream lines
const LINE: f32 = 2.0;

/// What is painted on a mini pedal under its controls
#[derive(Clone, Copy, PartialEq)]
pub enum Motif {
    /// Bare paint
    Plain,
    /// Lock gate: one leaf shut, one lowered, still water (wide, three knobs)
    Sluice,
    /// Rings of pressure on a weather chart (slim)
    Isobars,
    /// Midsummer fire with its sparks (wide, three knobs)
    Bonfire,
    /// The phases of the moon (slim)
    Moons,
    /// Spruces either side of the switch (wide, cabinet)
    Timber,
    /// Sea swell (slim)
    Swell,
    /// Northern lights: a sweep with hanging rays (slim)
    Aurora,
    /// Bands of rain (slim)
    Rain,
    /// Mountain ridges one behind the other (wide, three knobs)
    Ridges,
    /// Steep walls of a fjord at night (wide, two knobs)
    Fjord,
}

/// Paints `motif` on the top face `face` of a pedal painted `paint`. Call it over the paint
/// and under the screws, the title and the controls
pub(super) fn motif(painter: &Painter, face: Rect, paint: Color32, motif: Motif) {
    let print = Print {
        painter: painter.with_clip_rect(face.shrink(MARGIN)),
        face,
        paint,
    };
    match motif {
        Motif::Plain => {}
        Motif::Sluice => sluice(&print),
        Motif::Isobars => isobars(&print),
        Motif::Bonfire => bonfire(&print),
        Motif::Moons => moons(&print),
        Motif::Timber => timber(&print),
        Motif::Swell => swell(&print),
        Motif::Aurora => aurora(&print),
        Motif::Rain => rain(&print),
        Motif::Ridges => ridges(&print),
        Motif::Fjord => fjord(&print),
    }
}

/// One face being printed on. Filled shapes of one colour may overlap but never only touch:
/// the soft edges of two that touch leave a seam
struct Print {
    painter: Painter,
    face: Rect,
    paint: Color32,
}

impl Print {
    fn at(&self, (x, y): (f32, f32)) -> Pos2 {
        self.face.left_top() + vec2(x, y)
    }

    fn width(&self) -> f32 {
        self.face.width()
    }

    fn height(&self) -> f32 {
        self.face.height()
    }

    fn coat(&self) -> Color32 {
        shade(self.paint, COAT)
    }

    /// Convex shape. Corners clockwise
    fn fill(&self, corners: &[(f32, f32)], color: Color32) {
        let points = corners.iter().map(|&corner| self.at(corner)).collect();
        self.painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }

    /// `fill`, and the same again mirrored on the right half of the face
    fn fill_both(&self, corners: &[(f32, f32)], color: Color32) {
        self.fill(corners, color);
        let mirrored: Vec<_> = corners.iter().rev().map(|&(x, y)| (self.width() - x, y)).collect();
        self.fill(&mirrored, color);
    }

    fn rect(&self, min: (f32, f32), max: (f32, f32), color: Color32) {
        self.painter.rect_filled(Rect::from_min_max(self.at(min), self.at(max)), 0.0, color);
    }

    /// Everything from `top` down, across the face
    fn ground(&self, top: f32, color: Color32) {
        self.rect((0.0, top), (self.width(), self.height()), color);
    }

    fn path(&self, points: &[(f32, f32)], width: f32, color: Color32) {
        let points = points.iter().map(|&point| self.at(point)).collect();
        self.painter.add(Shape::line(points, Stroke::new(width, color)));
    }

    /// Cream line
    fn line(&self, from: (f32, f32), to: (f32, f32)) {
        self.painter.line_segment([self.at(from), self.at(to)], Stroke::new(LINE, SILK));
    }

    fn disc(&self, center: (f32, f32), radius: f32, color: Color32) {
        self.painter.circle_filled(self.at(center), radius, color);
    }

    fn ring(&self, center: (f32, f32), radius: f32, width: f32, color: Color32) {
        self.painter.circle_stroke(self.at(center), radius, Stroke::new(width, color));
    }

    /// A row of peaks: `profile` goes valley, peak, valley and so on from left to right.
    /// Each peak is a triangle down to the bottom of the face, so they overlap under the valleys
    fn peaks(&self, profile: &[(f32, f32)], color: Color32) {
        let bottom = self.height();
        let foot = |peak: (f32, f32), valley: (f32, f32)| {
            let run = (valley.0 - peak.0) / (valley.1 - peak.1);
            (peak.0 + run * (bottom - peak.1), bottom)
        };
        for at in (1..profile.len() - 1).step_by(2) {
            let peak = profile[at];
            self.fill(&[peak, foot(peak, profile[at + 1]), foot(peak, profile[at - 1])], color);
        }
    }
}

/// Sine line from `from` to the right as far as `to_x`: (x, y) of where it starts, the
/// height of a crest, the length of a wave and how far into a wave it starts, 0 to 1
fn wave(from: (f32, f32), to_x: f32, crest: f32, length: f32, start: f32) -> Vec<(f32, f32)> {
    let steps = ((to_x - from.0) / 2.0).ceil() as usize;
    (0..=steps)
        .map(|step| {
            let x = from.0 + step as f32 * 2.0;
            let turn = ((x - from.0) / length + start) * std::f32::consts::TAU;
            (x, from.1 - crest * turn.sin())
        })
        .collect()
}

fn sluice(print: &Print) {
    let coat = print.coat();
    print.ground(232.0, coat);
    // The shut leaf, with its brace, and the lowered one under the lamp
    for x in [8.0, 22.0, 36.0] {
        print.rect((x, 150.0), (x + 11.0, 234.0), coat);
    }
    print.path(&[(10.0, 228.0), (45.0, 156.0)], 5.0, coat);
    for x in [133.0, 147.0, 161.0] {
        print.rect((x, 200.0), (x + 11.0, 234.0), coat);
    }
    // Still water
    for (y, from, to) in [(252.0, 14.0, 42.0), (264.0, 24.0, 50.0), (276.0, 14.0, 34.0)] {
        print.line((from, y), (to, y));
        print.line((print.width() - to, y), (print.width() - from, y));
    }
}

fn isobars(print: &Print) {
    let center = (92.0, 198.0);
    for radius in [18.0, 38.0, 58.0, 78.0] {
        print.ring(center, radius, 6.0, print.coat());
    }
    print.line((center.0 - 5.0, center.1), (center.0 + 5.0, center.1));
    print.line((center.0, center.1 - 5.0), (center.0, center.1 + 5.0));
}

fn bonfire(print: &Print) {
    let coat = print.coat();
    print.ground(272.0, coat);
    // Two tongues a side, each steep on one flank and full on the other
    print.fill_both(&[(14.0, 158.0), (38.0, 236.0), (44.0, 300.0), (-6.0, 300.0)], coat);
    print.fill_both(&[(50.0, 186.0), (54.0, 300.0), (22.0, 300.0), (30.0, 236.0)], coat);
    for (x, y, size) in [(28.0, 156.0, 3.5), (48.0, 176.0, 2.5), (152.0, 152.0, 3.5), (132.0, 174.0, 2.5), (166.0, 180.0, 2.5)]
    {
        print.fill(&[(x, y - size), (x + size, y), (x, y + size), (x - size, y)], SILK);
    }
}

fn moons(print: &Print) {
    print.disc((110.0, 335.0), 85.0, print.coat());
    // Full, waning, a sliver, new: the paint covers what is dark
    let x = 88.0;
    for (y, dark) in [(158.0, None), (182.0, Some(11.0)), (206.0, Some(5.0))] {
        print.disc((x, y), 8.0, SILK);
        if let Some(offset) = dark {
            print.disc((x + offset, y), 8.5, print.paint);
        }
    }
    print.ring((x, 230.0), 7.0, 1.6, SILK);
    for star in [(14.0, 100.0), (30.0, 114.0), (18.0, 130.0)] {
        print.disc(star, 1.6, SILK);
    }
}

fn timber(print: &Print) {
    let coat = print.coat();
    print.ground(288.0, coat);
    // A spruce is a stack of tiers: (top, half width at the foot, foot)
    let spruce = |x: f32, tiers: &[(f32, f32, f32)]| {
        for &(top, half, foot) in tiers {
            print.fill_both(&[(x, top), (x + half, foot), (x - half, foot)], coat);
        }
    };
    spruce(24.0, &[(214.0, 12.0, 246.0), (234.0, 16.0, 270.0), (256.0, 20.0, 292.0)]);
    spruce(50.0, &[(238.0, 10.0, 264.0), (256.0, 13.0, 292.0)]);
}

fn swell(print: &Print) {
    let coat = print.coat();
    let width = print.width();
    // The sea: a thick wave is its surface
    print.ground(262.0, coat);
    print.path(&wave((-4.0, 258.0), width + 4.0, 5.0, 27.0, 0.0), 8.0, coat);
    // Deeper water under it
    let deep = shade(print.paint, COAT * 1.5);
    print.ground(288.0, deep);
    print.path(&wave((-4.0, 284.0), width + 4.0, 5.0, 27.0, 0.5), 8.0, deep);
    // Three voices, out of step
    for (y, start) in [(158.0, 0.0), (172.0, 0.33), (186.0, 0.66)] {
        print.path(&wave((70.0, y), width, 4.0, 20.0, start), LINE, SILK);
    }
}

fn aurora(print: &Print) {
    // One sweep from the lower left up to the right
    let (from, pull_from, pull_to, to) = ((-10.0, 262.0), (40.0, 250.0), (60.0, 150.0), (120.0, 116.0));
    let sweep: Vec<_> = (0..=24)
        .map(|step| {
            let t = step as f32 / 24.0;
            let weights = [(1.0 - t).powi(3), 3.0 * (1.0 - t).powi(2) * t, 3.0 * (1.0 - t) * t * t, t.powi(3)];
            let along = |pick: fn((f32, f32)) -> f32| {
                weights[0] * pick(from) + weights[1] * pick(pull_from) + weights[2] * pick(pull_to) + weights[3] * pick(to)
            };
            (along(|point| point.0), along(|point| point.1))
        })
        .collect();
    print.path(&sweep, 28.0, print.coat());
    // Rays hanging like the teeth of a comb, shorter as they go
    for (x, top, bottom) in [
        (72.0, 150.0, 192.0),
        (77.0, 156.0, 194.0),
        (82.0, 164.0, 198.0),
        (87.0, 174.0, 204.0),
        (92.0, 186.0, 212.0),
        (97.0, 200.0, 222.0),
        (102.0, 216.0, 234.0),
        (10.0, 96.0, 122.0),
        (16.0, 100.0, 118.0),
        (22.0, 106.0, 116.0),
    ] {
        print.line((x, top), (x, bottom));
    }
}

fn rain(print: &Print) {
    let right = print.width() + 10.0;
    for top in [196.0, 252.0, 308.0] {
        print.fill(&[(-10.0, top), (right, top - 46.0), (right, top - 24.0), (-10.0, top + 22.0)], print.coat());
    }
    // Drops, long and short in turn
    for (row, y) in [150.0, 176.0, 202.0, 228.0].into_iter().enumerate() {
        for (column, x) in [78.0, 90.0, 102.0].into_iter().enumerate() {
            let (lean, length) = if (row + column) % 2 == 0 { (4.0, 14.0) } else { (2.0, 6.0) };
            print.line((x, y), (x - lean, y + length));
        }
    }
}

fn ridges(print: &Print) {
    // The same land three times, each nearer and darker
    let far = [(0.0, 230.0), (30.0, 170.0), (62.0, 236.0), (96.0, 190.0), (132.0, 246.0), (160.0, 206.0), (180.0, 232.0)];
    let middle = [(0.0, 262.0), (40.0, 214.0), (76.0, 268.0), (112.0, 232.0), (150.0, 276.0), (186.0, 246.0), (200.0, 280.0)];
    let near = [(0.0, 284.0), (50.0, 252.0), (92.0, 290.0), (128.0, 266.0), (168.0, 294.0), (190.0, 280.0), (200.0, 300.0)];
    for (profile, depth) in [(far, 0.5), (middle, 1.0), (near, 1.5)] {
        print.peaks(&profile, shade(print.paint, COAT * depth));
    }
    // A call and what comes back of it
    let call = (152.0, 168.0);
    print.disc(call, 4.0, SILK);
    print.ring(call, 9.0, 1.8, SILK);
    print.ring(call, 15.0, 1.0, SILK);
}

fn fjord(print: &Print) {
    let coat = print.coat();
    // A wall down each side, in slabs from the edge to its outline
    let wall = [(12.0, 0.0), (22.0, 60.0), (34.0, 120.0), (52.0, 150.0), (60.0, 230.0), (76.0, 300.0)];
    for slab in wall.windows(2) {
        let (upper, lower) = (slab[0], slab[1]);
        // A pixel into the slab below, so that the two overlap
        print.fill_both(&[(-5.0, upper.1), upper, lower, (lower.0, lower.1 + 1.0), (-5.0, lower.1 + 1.0)], coat);
    }
    for star in [(62.0, 56.0), (72.0, 92.0), (118.0, 60.0), (108.0, 98.0)] {
        print.disc(star, 1.6, SILK);
    }
    print.line((70.0, 232.0), (110.0, 232.0));
    print.line((78.0, 240.0), (102.0, 240.0));
}
