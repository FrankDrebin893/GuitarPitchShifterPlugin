//! Surfaces that are lit pixel by pixel instead of filled with a flat colour.
//!
//! Vector shapes cannot show what makes a real pedal look real: a bevel that catches the light,
//! the flake in powder coat, knurled and brushed metal, a soft shadow. Each function here renders
//! such a surface once, with a small software renderer (a signed distance field gives the shape
//! and its edge normals, one fixed light gives the shading), and `cached` keeps the result as an
//! egui texture. The renders are plain functions of their inputs, so tests can look at the pixels.
//!
//! All sizes in the public functions are in points; `ppp` (pixels per point) makes the texture
//! as sharp as the screen.

use std::hash::Hash;

use nih_plug_egui::egui::{
    pos2, Color32, ColorImage, Context, Id, Painter, Pos2, Rect, TextureHandle, TextureOptions,
    Vec2,
};

use super::theme::{CHROME, INK, KNOB_CAP_EDGE, KNOB_CAP_TOP, KNOB_SKIRT};

/// Room around a box for its shadow, in points
pub const BOX_SHADOW_MARGIN: f32 = 6.0;
/// Room around a knob for its shadow, in points
pub const KNOB_MARGIN: f32 = 11.0;

/// Direction to the light, from the top left and in front. Unit length
const LIGHT: [f32; 3] = [-0.447, -0.646, 0.617];
// Halfway between the light and the viewer
const HALF: [f32; 3] = [-0.250, -0.359, 0.899];

pub(super) type Rgb = [f32; 3];

pub(super) fn rgb(color: Color32) -> Rgb {
    [color.r() as f32, color.g() as f32, color.b() as f32]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt().max(1e-6);
    [v[0] / length, v[1] / length, v[2] / length]
}

/// A surface normal tilted by `(x, y)`, which is the slope of the surface
pub(super) fn tilted(x: f32, y: f32) -> [f32; 3] {
    normalize([x, y, 1.0])
}

/// How much light the surface gets: 1 on a surface that faces the viewer, more or less elsewhere
pub(super) fn diffuse(normal: [f32; 3]) -> f32 {
    (dot(normal, LIGHT) / LIGHT[2]).max(0.0)
}

pub(super) fn specular(normal: [f32; 3], sharpness: f32) -> f32 {
    dot(normal, HALF).max(0.0).powf(sharpness)
}

pub(super) fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(super) fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA6B)
        ^ seed.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Smooth noise between 0 and 1 that changes over about one unit
pub(super) fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (smoothstep(0.0, 1.0, x - ix), smoothstep(0.0, 1.0, y - iy));
    let (ix, iy) = (ix as i32, iy as i32);
    let top = hash(ix, iy, seed) + (hash(ix + 1, iy, seed) - hash(ix, iy, seed)) * fx;
    let bottom =
        hash(ix, iy + 1, seed) + (hash(ix + 1, iy + 1, seed) - hash(ix, iy + 1, seed)) * fx;
    top + (bottom - top) * fy
}

/// Distance from `p` to the edge of a rounded rectangle centred on `c`: negative inside
pub(super) fn rounded_box(p: [f32; 2], c: [f32; 2], half: [f32; 2], radius: f32) -> f32 {
    let q = [(p[0] - c[0]).abs() - (half[0] - radius), (p[1] - c[1]).abs() - (half[1] - radius)];
    let outside = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt();
    outside + q[0].max(q[1]).min(0.0) - radius
}

/// A premultiplied pixel being built from layers
#[derive(Clone, Copy, Default)]
pub(super) struct Pixel {
    rgb: Rgb,
    alpha: f32,
}

impl Pixel {
    /// `color` laid over what is there, `coverage` of the way
    pub(super) fn over(&mut self, color: Rgb, coverage: f32) {
        let coverage = coverage.clamp(0.0, 1.0);
        for channel in 0..3 {
            self.rgb[channel] = color[channel] * coverage + self.rgb[channel] * (1.0 - coverage);
        }
        self.alpha = coverage + self.alpha * (1.0 - coverage);
    }

    pub(super) fn color(self) -> Color32 {
        let channel = |value: f32| value.clamp(0.0, 255.0).round() as u8;
        // `rgb` holds the premultiplied value already: `over` weighs it by coverage
        let alpha = (self.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_premultiplied(
            channel(self.rgb[0]).min(alpha),
            channel(self.rgb[1]).min(alpha),
            channel(self.rgb[2]).min(alpha),
            alpha,
        )
    }
}

pub(super) fn image(width: usize, height: usize, pixels: Vec<Pixel>) -> ColorImage {
    ColorImage { size: [width, height], pixels: pixels.into_iter().map(Pixel::color).collect() }
}

/// Number of device pixels for a length in points
pub(super) fn device(points: f32, ppp: f32) -> f32 {
    points * ppp
}

pub(super) fn shaded(base: Rgb, light: f32, glint: f32) -> Rgb {
    // Painted metal gets less contrast than the light would give it
    let level = 0.55 + 0.45 * light.clamp(0.0, 2.0);
    [
        base[0] * level + 255.0 * glint,
        base[1] * level + 255.0 * glint,
        base[2] * level + 255.0 * glint,
    ]
}

/// A pedal enclosure in powder coat: a bevelled top face, the darker lower edge of the box and
/// a soft shadow on the bench. Returns the image and the margin around the box in pixels.
pub fn box_image(
    size: Vec2,
    radius: f32,
    edge_height: f32,
    paint: Color32,
    ppp: f32,
) -> (ColorImage, usize) {
    let margin = device(BOX_SHADOW_MARGIN, ppp).round() as usize;
    let box_w = device(size.x, ppp).round().max(1.0);
    let box_h = device(size.y, ppp).round().max(1.0);
    let (width, height) = (box_w as usize + 2 * margin, box_h as usize + 2 * margin);
    let m = margin as f32;
    let radius = device(radius, ppp);
    let edge = device(edge_height, ppp);
    let bevel = device(4.5, ppp);
    let blur = device(2.6, ppp);
    let base = rgb(paint);
    let side_base = [base[0] * 0.70, base[1] * 0.70, base[2] * 0.70];

    let centre = [m + box_w / 2.0, m + box_h / 2.0];
    let half = [box_w / 2.0, box_h / 2.0];
    let top_centre = [centre[0], centre[1] - edge / 2.0];
    let top_half = [half[0], half[1] - edge / 2.0];
    let top_distance = |p: [f32; 2]| rounded_box(p, top_centre, top_half, radius);

    let mut pixels = vec![Pixel::default(); width * height];
    for y in 0..height {
        for x in 0..width {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let mut pixel = Pixel::default();

            // Shadow, a little lower than the box
            let shadow = rounded_box([p[0], p[1] - device(1.6, ppp)], centre, half, radius);
            pixel.over([0.0, 0.0, 0.0], 0.5 * (-(shadow.max(0.0) / blur).powi(2)).exp());

            // The box seen from the front: a lower edge that gets darker towards the bench
            let outline = rounded_box(p, centre, half, radius);
            let down = ((p[1] - (centre[1] + half[1] - edge)) / edge.max(1.0)).clamp(0.0, 1.0);
            let side = [
                side_base[0] * (1.0 - 0.18 * down),
                side_base[1] * (1.0 - 0.18 * down),
                side_base[2] * (1.0 - 0.18 * down),
            ];
            pixel.over(side, (0.5 - outline).clamp(0.0, 1.0));

            // The top face
            let d = top_distance(p);
            let coverage = (0.5 - d).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let inside = (-d).max(0.0);
                // The way outwards, from the slope of the distance
                let gx = top_distance([p[0] + 0.5, p[1]]) - top_distance([p[0] - 0.5, p[1]]);
                let gy = top_distance([p[0], p[1] + 0.5]) - top_distance([p[0], p[1] - 0.5]);
                // A rounded edge: steep at the rim, flat from `bevel` inwards
                let round = 1.0 - smoothstep(0.0, bevel, inside);
                let tilt = round * round * 1.8;
                let (mut nx, mut ny) = (gx * tilt, gy * tilt);
                // Powder coat: fine flake and a slow orange peel, both in the normal
                let flake = hash(x as i32, y as i32, 7) - 0.5;
                let peel_x =
                    value_noise(p[0] / device(3.0, ppp), p[1] / device(3.0, ppp), 11) - 0.5;
                let peel_y =
                    value_noise(p[0] / device(3.0, ppp), p[1] / device(3.0, ppp), 23) - 0.5;
                nx += peel_x * 0.07 + flake * 0.10;
                ny += peel_y * 0.07 + (hash(x as i32, y as i32, 13) - 0.5) * 0.10;
                let normal = tilted(nx, ny);
                // A soft sheen over the whole face, lighter at the top
                let sheen = 1.0 + 0.07 * (0.5 - (p[1] - m) / box_h) * 2.0;
                let glint = specular(normal, 38.0) * 0.45 * (0.35 + round);
                let mut color = shaded(base, diffuse(normal), glint);
                color = [color[0] * sheen, color[1] * sheen, color[2] * sheen];
                // A hairline of shadow where the top meets the lower edge
                let seam = smoothstep(0.0, device(1.5, ppp), inside);
                let dark = 0.80 + 0.20 * seam;
                color = [color[0] * dark, color[1] * dark, color[2] * dark];
                pixel.over(color, coverage);
            }
            pixels[y * width + x] = pixel;
        }
    }
    (image(width, height, pixels), margin)
}

/// A fluted knob in glossy black plastic with a brushed dish on top, and its shadow.
/// The pointer is not in it: the caller draws that, since it turns.
pub fn knob_image(radius: f32, ppp: f32) -> ColorImage {
    let side = ((radius + KNOB_MARGIN) * 2.0 * ppp).ceil() as usize;
    let c = side as f32 / 2.0;
    let r = device(radius, ppp);
    let flutes = (radius * 0.8).round().max(8.0);
    let shadow_offset = [device(0.4, ppp), device(1.2, ppp)];
    let tight = device(1.0, ppp);
    let soft = device(3.5, ppp);
    let skirt = rgb(KNOB_SKIRT);
    let cap = rgb(KNOB_CAP_TOP);
    let rim = rgb(KNOB_CAP_EDGE);

    let mut pixels = vec![Pixel::default(); side * side];
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let dist = (dx * dx + dy * dy).sqrt();
            let mut pixel = Pixel::default();

            let (sx, sy) = (dx - shadow_offset[0], dy - shadow_offset[1]);
            let gap = ((sx * sx + sy * sy).sqrt() - r).max(0.0);
            // Where the knob meets the paint it is dark; a little further out only faintly so
            let contact = 0.55 * (-(gap / tight).powi(2)).exp();
            let ambient = 0.20 * (-(gap / soft).powi(2)).exp();
            pixel.over([0.0; 3], 1.0 - (1.0 - contact) * (1.0 - ambient));

            let coverage = (r - dist + 0.5).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let t = dist / r;
                let phi = dy.atan2(dx);
                let (ux, uy) = (phi.cos(), phi.sin());
                let color = if t > 0.7 {
                    // Skirt: flutes run up and down it, and it rounds off at the rim
                    let out = (t - 0.7) / 0.3;
                    let round = out.powi(3) * 1.1;
                    let flute = (flutes * phi).sin() * 0.55;
                    let normal = tilted(ux * round - uy * flute, uy * round + ux * flute);
                    let base = [skirt[0] * 1.6, skirt[1] * 1.6, skirt[2] * 1.6];
                    shaded(base, diffuse(normal), specular(normal, 26.0) * 0.5)
                } else {
                    // Cap: a shallow dome with a bevel at its rim and a brushed dish in the middle
                    let edge = smoothstep(0.62, 0.70, t);
                    let slope = t / 0.7 * 0.18 + edge * 1.2;
                    let normal = tilted(ux * slope, uy * slope);
                    let brush = value_noise(dist * 1.6 / ppp.max(1.0), 0.5, 5) - 0.5;
                    // Two soft bright bands across the turned face, as on turned metal
                    let band = (0.5 + 0.5 * (2.0 * (phi - 2.2)).cos()).powi(6);
                    let dish = 1.0 - smoothstep(0.50, 0.54, t);
                    let base = [
                        cap[0] + (rim[0] - cap[0]) * edge,
                        cap[1] + (rim[1] - cap[1]) * edge,
                        cap[2] + (rim[2] - cap[2]) * edge,
                    ];
                    let mut color = shaded(base, diffuse(normal), specular(normal, 30.0) * 0.4);
                    let shine = (band * 0.30 + brush * 0.10) * dish * 255.0 * (1.0 - t);
                    color = [color[0] + shine, color[1] + shine, color[2] + shine];
                    // Dark groove between the cap and the skirt
                    let groove = 1.0 - 0.55 * (1.0 - smoothstep(0.0, 0.025, (t - 0.70).abs()));
                    [color[0] * groove, color[1] * groove, color[2] * groove]
                };
                // The rim catches a little light all round
                let lip = smoothstep(0.93, 1.0, t) * 0.10 * 255.0;
                pixel.over([color[0] + lip, color[1] + lip, color[2] + lip], coverage);
            }
            pixels[y * side + x] = pixel;
        }
    }
    image(side, side, pixels)
}

/// A slotted screw head in chrome: domed, with a slot at `slot_angle`, and its shadow
pub fn screw_image(scale: f32, slot_angle: f32, ppp: f32) -> ColorImage {
    let radius = 6.0 * scale;
    let side = ((radius + 3.0) * 2.0 * ppp).ceil() as usize;
    let c = side as f32 / 2.0;
    let r = device(radius, ppp);
    let chrome = rgb(CHROME);
    let ink = rgb(INK);
    let slot_dir = [slot_angle.cos(), slot_angle.sin()];
    let slot_len = device(4.0 * scale, ppp);
    let slot_half = device(0.8 * scale, ppp).max(0.6);

    let mut pixels = vec![Pixel::default(); side * side];
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let dist = (dx * dx + dy * dy).sqrt();
            let mut pixel = Pixel::default();

            let shadow = (dx * dx + (dy - device(1.0, ppp)).powi(2)).sqrt() - r;
            pixel.over([0.0; 3], 0.45 * (-(shadow.max(0.0) / device(1.4, ppp)).powi(2)).exp());

            let coverage = (r - dist + 0.5).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let t = dist / r;
                let slope = t * t * 0.9;
                let (ux, uy) = if dist > 0.0 { (dx / dist, dy / dist) } else { (0.0, 0.0) };
                let normal = tilted(ux * slope, uy * slope);
                let brush = (hash(x as i32, y as i32, 3) - 0.5) * 0.08;
                let mut color =
                    shaded(chrome, diffuse(normal) + brush, specular(normal, 20.0) * 0.55);
                // The slot: its distance from the line through the middle
                let along = dx * slot_dir[0] + dy * slot_dir[1];
                let across = (-dx * slot_dir[1] + dy * slot_dir[0]).abs();
                let slot = if along.abs() <= slot_len {
                    (slot_half + 0.5 - across).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                color = [
                    color[0] + (ink[0] - color[0]) * slot,
                    color[1] + (ink[1] - color[1]) * slot,
                    color[2] + (ink[2] - color[2]) * slot,
                ];
                // A thin dark ring where the head meets the paint
                let ring = smoothstep(0.88, 1.0, t) * 0.55;
                let color =
                    [color[0] * (1.0 - ring), color[1] * (1.0 - ring), color[2] * (1.0 - ring)];
                pixel.over(color, coverage);
            }
            pixels[y * side + x] = pixel;
        }
    }
    image(side, side, pixels)
}

/// The texture for `key`: rendered by `render` the first time and kept by the context after that
pub(super) fn cached(
    ctx: &Context,
    key: impl Hash,
    render: impl FnOnce() -> ColorImage,
) -> TextureHandle {
    let id = Id::new(("raster", key));
    if let Some(texture) = ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
        return texture;
    }
    let texture = ctx.load_texture("raster", render(), TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

pub(super) fn blit(painter: &Painter, texture: &TextureHandle, rect: Rect) {
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// Paint `rect` as a bevelled box in powder coat, with its lower edge `edge_height` tall and a
/// shadow around it
pub fn paint_box(painter: &Painter, rect: Rect, radius: f32, edge_height: f32, paint: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let key = (
        "box",
        rect.width().to_bits(),
        rect.height().to_bits(),
        radius.to_bits(),
        edge_height.to_bits(),
        paint,
        ppp.to_bits(),
    );
    let mut margin = 0;
    let texture = cached(ctx, key, || {
        let (image, m) = box_image(rect.size(), radius, edge_height, paint, ppp);
        margin = m;
        image
    });
    // On a cache hit the margin was not rendered: it follows from the sizes
    let margin = if margin == 0 { device(BOX_SHADOW_MARGIN, ppp).round() as usize } else { margin };
    blit(painter, &texture, rect.expand(margin as f32 / ppp));
}

/// A knob's body and shadow centred on `center`
pub fn paint_knob(painter: &Painter, center: Pos2, radius: f32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let texture =
        cached(ctx, ("knob", radius.to_bits(), ppp.to_bits()), || knob_image(radius, ppp));
    blit(
        painter,
        &texture,
        Rect::from_center_size(center, Vec2::splat((radius + KNOB_MARGIN) * 2.0)),
    );
}

/// A screw head and its shadow centred on `center`
pub fn paint_screw(painter: &Painter, center: Pos2, slot_angle: f32, scale: f32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let key = ("screw", scale.to_bits(), (slot_angle * 100.0).round() as i32, ppp.to_bits());
    let texture = cached(ctx, key, || screw_image(scale, slot_angle, ppp));
    blit(painter, &texture, Rect::from_center_size(center, Vec2::splat((6.0 * scale + 3.0) * 2.0)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::PAINT_TEAL;
    use nih_plug_egui::egui::vec2;

    fn pixel(image: &ColorImage, x: usize, y: usize) -> Color32 {
        image.pixels[y * image.size[0] + x]
    }

    fn luminance(color: Color32) -> f32 {
        0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32
    }

    #[test]
    fn test_box_is_paint_in_the_middle_and_clear_outside_its_shadow() {
        let (image, margin) = box_image(vec2(200.0, 300.0), 24.0, 5.0, PAINT_TEAL, 1.0);
        assert_eq!(image.size, [200 + 2 * margin, 300 + 2 * margin]);
        let middle = pixel(&image, image.size[0] / 2, image.size[1] / 2);
        // Within the flake and sheen of the paint
        assert!((middle.g() as i32 - PAINT_TEAL.g() as i32).abs() < 40, "{middle:?}");
        assert!(middle.a() == 255);
        assert_eq!(pixel(&image, 0, 0), Color32::TRANSPARENT);
    }

    #[test]
    fn test_box_edge_facing_the_light_is_lighter_than_the_one_facing_away() {
        let (image, margin) = box_image(vec2(200.0, 300.0), 24.0, 5.0, PAINT_TEAL, 1.0);
        let row = image.size[1] / 2;
        let mean =
            |x: usize| (0..9).map(|dy| luminance(pixel(&image, x, row + dy))).sum::<f32>() / 9.0;
        let left = mean(margin + 1);
        let right = mean(image.size[0] - margin - 2);
        assert!(left > right + 5.0, "left {left}, right {right}");
    }

    #[test]
    fn test_knob_is_round_with_its_shadow_below_and_to_the_right() {
        let image = knob_image(30.0, 1.0);
        let side = image.size[0];
        assert_eq!(image.size[0], image.size[1]);
        assert_eq!(pixel(&image, 0, 0), Color32::TRANSPARENT);
        assert_eq!(pixel(&image, side / 2, side / 2).a(), 255);
        let below = pixel(&image, side / 2 + 2, side / 2 + 34).a();
        let above = pixel(&image, side / 2 + 2, side / 2 - 34).a();
        assert!(below > above, "below {below}, above {above}");
    }

    #[test]
    fn test_screw_has_a_dark_slot() {
        let image = screw_image(1.0, 0.0, 2.0);
        let side = image.size[0];
        let on_slot = luminance(pixel(&image, side / 2 + 4, side / 2));
        let off_slot = luminance(pixel(&image, side / 2 + 4, side / 2 + 6));
        assert!(on_slot < off_slot - 20.0, "{on_slot} vs {off_slot}");
    }

    #[test]
    fn test_pixels_are_valid_premultiplied() {
        let (image, _) = box_image(vec2(80.0, 80.0), 14.0, 5.0, PAINT_TEAL, 2.0);
        for color in image.pixels {
            assert!(color.r() <= color.a() && color.g() <= color.a() && color.b() <= color.a());
        }
    }

    /// Writes a sheet of the surfaces as raw RGB (width, height, then pixels) to the file named
    /// by `RASTER_PREVIEW`, for looking at: `cargo test -p suite_common preview -- --ignored`
    #[test]
    #[ignore]
    fn preview() {
        let Ok(path) = std::env::var("RASTER_PREVIEW") else { return };
        let ppp = 2.0;
        let (w, h) = (880usize, 560usize);
        let mut sheet = vec![[18.0_f32, 17.0, 16.0]; w * h];
        let mut put = |image: &ColorImage, ox: usize, oy: usize| {
            for y in 0..image.size[1] {
                for x in 0..image.size[0] {
                    let c = image.pixels[y * image.size[0] + x];
                    let (px, py) = (ox + x, oy + y);
                    if px < w && py < h {
                        let a = c.a() as f32 / 255.0;
                        let dst = &mut sheet[py * w + px];
                        dst[0] = c.r() as f32 + dst[0] * (1.0 - a);
                        dst[1] = c.g() as f32 + dst[1] * (1.0 - a);
                        dst[2] = c.b() as f32 + dst[2] * (1.0 - a);
                    }
                }
            }
        };
        let (b, _) = box_image(vec2(200.0, 270.0), 24.0, 5.0, PAINT_TEAL, ppp);
        put(&b, 10, 10);
        let (b, _) = box_image(vec2(200.0, 270.0), 24.0, 5.0, crate::ui::theme::PAINT_OXBLOOD, ppp);
        put(&b, 450, 10);
        put(&knob_image(48.0, ppp), 20, 300);
        put(&knob_image(30.0, ppp), 260, 330);
        put(&knob_image(19.0, ppp), 400, 350);
        put(&screw_image(1.0, 0.6, ppp), 500, 350);
        put(&screw_image(1.0, -0.9, ppp), 560, 350);
        use crate::ui::hardware::*;
        put(&stomp_image(1.0, false, ppp), 600, 300);
        put(&stomp_image(1.0, true, ppp), 740, 300);
        let off = Color32::from_rgb(90, 26, 18);
        put(&lamp_image(true, off, false, 1.0, ppp), 640, 160);
        put(&lamp_image(false, off, false, 1.0, ppp), 700, 160);
        put(&lamp_image(false, Color32::from_rgb(58, 12, 10), true, 1.0, ppp), 760, 160);
        put(&tape_image(vec2(90.0, 22.0), ppp), 620, 230);
        put(&paper_image(vec2(120.0, 46.0), ppp), 600, 100);
        let mut out = Vec::new();
        out.extend((w as u32).to_le_bytes());
        out.extend((h as u32).to_le_bytes());
        for p in sheet {
            out.extend(p.map(|v| v.clamp(0.0, 255.0) as u8));
        }
        std::fs::write(path, out).unwrap();
    }
}
