//! The hardware and print of a pedal, lit pixel by pixel like the rest of `raster`: the
//! footswitch, the indicator light, label tape and the paper of the name band.

use nih_plug_egui::egui::epaint::Vertex;
use nih_plug_egui::egui::{pos2, Color32, ColorImage, Mesh, Painter, Pos2, Rect, Shape, Vec2};

use super::raster::{
    blit, cached, device, diffuse, hash, image, rgb, rounded_box, shaded, smoothstep, specular,
    tilted, value_noise, Pixel, Rgb,
};
use super::theme::{CHROME, CHROME_DARK, CHROME_LIGHT, LED_ON, SILK, TAPE};

/// Chrome seen from above. `slope` is how steeply the surface falls away, 0 flat to 1 steep;
/// (`ux`, `uy`) points outwards. The result is a lit colour
fn chrome_at(base: Rgb, ux: f32, uy: f32, slope: f32, grain: f32, sharp: f32) -> Rgb {
    let normal = tilted(ux * slope, uy * slope);
    let color = shaded(base, diffuse(normal) + grain, specular(normal, sharp) * 0.6);
    // A reflected horizon: the top of the metal is lighter than its bottom
    let sky = 1.0 - 0.10 * (uy * slope).clamp(-1.0, 1.0);
    [color[0] * sky, color[1] * sky, color[2] * sky]
}

fn direction(dx: f32, dy: f32, dist: f32) -> (f32, f32) {
    if dist > 0.0 {
        (dx / dist, dy / dist)
    } else {
        (0.0, 0.0)
    }
}

/// A footswitch: a chrome washer, a nut and a domed cap, and its shadow. A pressed cap sits
/// lower, is darker and has no shine.
pub fn stomp_image(scale: f32, pressed: bool, ppp: f32) -> ColorImage {
    let side = (52.0 * 2.0 * scale * ppp).ceil() as usize;
    let c = side as f32 / 2.0;
    let u = scale * ppp;
    let (light, chrome, dark) = (rgb(CHROME_LIGHT), rgb(CHROME), rgb(CHROME_DARK));
    let cap_offset = if pressed { 1.5 * u } else { 0.0 };
    let mut pixels = vec![Pixel::default(); side * side];
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let dist = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = direction(dx, dy, dist);
            let mut pixel = Pixel::default();
            let gap = ((dx * dx + (dy - 1.5 * u).powi(2)).sqrt() - 37.0 * u).max(0.0);
            let contact = 0.55 * (-(gap / (1.2 * u)).powi(2)).exp();
            let ambient = 0.20 * (-(gap / (4.0 * u)).powi(2)).exp();
            pixel.over([0.0; 3], 1.0 - (1.0 - contact) * (1.0 - ambient));

            // Washer, with a rounded outer edge and concentric brushing
            let brush = (value_noise(dist / ppp.max(1.0) * 1.8, 0.5, 9) - 0.5) * 0.12;
            let washer = (37.0 * u - dist + 0.5).clamp(0.0, 1.0);
            if washer > 0.0 {
                let t = ((dist - 32.0 * u) / (5.0 * u)).clamp(0.0, 1.0);
                pixel.over(chrome_at(light, ux, uy, t.powi(3) * 1.2, brush, 18.0), washer);
            }
            // Nut: a bevelled ring with a dark seam to the washer
            let nut = (32.0 * u - dist + 0.5).clamp(0.0, 1.0);
            if nut > 0.0 {
                let t = ((dist - 27.0 * u) / (5.0 * u)).clamp(0.0, 1.0);
                let color = chrome_at(chrome, ux, uy, t * t, brush, 18.0);
                let seam = 1.0 - 0.45 * (1.0 - smoothstep(0.0, 1.5 * u, 32.0 * u - dist));
                pixel.over([color[0] * seam, color[1] * seam, color[2] * seam], nut);
            }
            // The cap
            let (cx, cy) = (dx, dy - cap_offset);
            let cap_dist = (cx * cx + cy * cy).sqrt();
            let cap = (26.0 * u - cap_dist + 0.5).clamp(0.0, 1.0);
            if cap > 0.0 {
                let t = cap_dist / (26.0 * u);
                let (cux, cuy) = direction(cx, cy, cap_dist);
                let slope = 0.25 * t * t + 0.9 * smoothstep(0.78, 1.0, t);
                let (base, sharp) = if pressed { (dark, 10.0) } else { (chrome, 28.0) };
                let color = chrome_at(base, cux, cuy, slope, brush * 0.5, sharp);
                // A dark ring where the cap meets the nut
                let ring = 1.0 - 0.5 * smoothstep(0.93, 1.0, t);
                pixel.over([color[0] * ring, color[1] * ring, color[2] * ring], cap);
            }
            pixels[y * side + x] = pixel;
        }
    }
    image(side, side, pixels)
}

/// An indicator light: a chrome bezel, an optional black socket, a glossy lens and, when lit,
/// a glow around it. The measures are those of the full-size light times `scale`.
pub fn lamp_image(on: bool, off_color: Color32, socket: bool, scale: f32, ppp: f32) -> ColorImage {
    let side = (2.0 * 17.0 * scale * ppp).ceil() as usize;
    let c = side as f32 / 2.0;
    let u = scale * ppp;
    let (bezel, lens_on, lens_off, tape) =
        (rgb(CHROME_LIGHT), rgb(LED_ON), rgb(off_color), rgb(TAPE));
    let mut pixels = vec![Pixel::default(); side * side];
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let dist = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = direction(dx, dy, dist);
            let mut pixel = Pixel::default();
            if on {
                pixel.over(lens_on, (-(dist / (11.0 * u)).powi(2)).exp() * 0.45);
            }
            let outer = (8.5 * u - dist + 0.5).clamp(0.0, 1.0);
            if outer > 0.0 {
                let t = ((dist - 6.0 * u) / (2.5 * u)).clamp(0.0, 1.0);
                pixel.over(chrome_at(bezel, ux, uy, t * t * 1.1, 0.0, 18.0), outer);
            }
            if socket {
                pixel.over(tape, (6.8 * u - dist + 0.5).clamp(0.0, 1.0));
            }
            let lens = (5.5 * u - dist + 0.5).clamp(0.0, 1.0);
            if lens > 0.0 {
                let t = dist / (5.5 * u);
                let normal = tilted(ux * t * t * 0.9, uy * t * t * 0.9);
                let base = if on { lens_on } else { lens_off };
                let light = if on { 1.15 } else { 0.9 + 0.25 * (diffuse(normal) - 1.0) };
                let color = [base[0] * light, base[1] * light, base[2] * light];
                // The window's reflection in the lens: a sheen and a bright spot
                let spot = ((dx + 1.6 * u).powi(2) + (dy + 1.9 * u).powi(2)).sqrt();
                let gloss = (specular(normal, 14.0) * if on { 0.9 } else { 0.5 }
                    + (1.0 - smoothstep(0.0, 2.2 * u, spot)) * 0.55)
                    .min(1.0);
                pixel.over(
                    [
                        color[0] + (255.0 - color[0]) * gloss,
                        color[1] + (240.0 - color[1]) * gloss,
                        color[2] + (230.0 - color[2]) * gloss,
                    ],
                    lens,
                );
            }
            pixels[y * side + x] = pixel;
        }
    }
    image(side, side, pixels)
}

/// Room around label tape for its shadow, in points
const TAPE_MARGIN: f32 = 3.0;

/// Label tape: matte black with a lift towards its top, a hairline highlight on its top edge
/// and a fine grain, and its shadow
pub fn tape_image(size: Vec2, ppp: f32) -> ColorImage {
    let (w, h) = (device(size.x, ppp).round().max(1.0), device(size.y, ppp).round().max(1.0));
    let m = device(TAPE_MARGIN, ppp).round();
    let (width, height) = (w as usize + 2 * m as usize, h as usize + 2 * m as usize);
    let tape = rgb(TAPE);
    let radius = device(2.0, ppp);
    let (c, half) = ([m + w / 2.0, m + h / 2.0], [w / 2.0, h / 2.0]);
    let mut pixels = vec![Pixel::default(); width * height];
    for y in 0..height {
        for x in 0..width {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let mut pixel = Pixel::default();
            let shadow = rounded_box([p[0], p[1] - device(2.0, ppp)], c, half, radius);
            pixel.over([0.0; 3], 0.35 * (-(shadow.max(0.0) / device(1.5, ppp)).powi(2)).exp());
            let d = rounded_box(p, c, half, radius);
            let up = 1.0 - ((p[1] - m) / h).clamp(0.0, 1.0);
            let grain = (hash(x as i32, y as i32, 31) - 0.5) * 7.0;
            let top_edge = 1.0 - smoothstep(0.0, device(1.2, ppp), p[1] - m);
            let lift = 10.0 * up * up + grain + 22.0 * top_edge;
            pixel.over([tape[0] + lift, tape[1] + lift, tape[2] + lift], (0.5 - d).clamp(0.0, 1.0));
            pixels[y * width + x] = pixel;
        }
    }
    image(width, height, pixels)
}

/// The cream paper of the name band: a soft light along its top and fibres that run along it
pub fn paper_image(size: Vec2, ppp: f32) -> ColorImage {
    let w = device(size.x, ppp).round().max(1.0) as usize;
    let h = device(size.y, ppp).round().max(1.0) as usize;
    let silk = rgb(SILK);
    let mut pixels = vec![Pixel::default(); w * h];
    for y in 0..h {
        for x in 0..w {
            let fibre = value_noise(x as f32 / (14.0 * ppp), y as f32 / (0.9 * ppp), 41) - 0.5;
            let speck = hash(x as i32, y as i32, 43) - 0.5;
            let v = y as f32 / h as f32;
            let lift = 1.0 + 0.05 * (0.5 - v) + fibre * 0.03 + speck * 0.03;
            let mut pixel = Pixel::default();
            pixel.over([silk[0] * lift, silk[1] * lift, silk[2] * lift], 1.0);
            pixels[y * w + x] = pixel;
        }
    }
    image(w, h, pixels)
}

/// A footswitch centred on `center`, at `scale` times its full size
pub fn paint_stomp(painter: &Painter, center: Pos2, scale: f32, pressed: bool) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let key = ("stomp", scale.to_bits(), pressed, ppp.to_bits());
    let texture = cached(ctx, key, || stomp_image(scale, pressed, ppp));
    blit(painter, &texture, Rect::from_center_size(center, Vec2::splat(104.0 * scale)));
}

/// An indicator light centred on `center`
pub fn paint_lamp(
    painter: &Painter,
    center: Pos2,
    on: bool,
    off_color: Color32,
    socket: bool,
    scale: f32,
) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let key = ("lamp", on, off_color, socket, scale.to_bits(), ppp.to_bits());
    let texture = cached(ctx, key, || lamp_image(on, off_color, socket, scale, ppp));
    blit(painter, &texture, Rect::from_center_size(center, Vec2::splat(34.0 * scale)));
}

/// A strip of label tape filling `rect`
pub fn paint_tape(painter: &Painter, rect: Rect) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let key = ("tape", rect.width().to_bits(), rect.height().to_bits(), ppp.to_bits());
    let texture = cached(ctx, key, || tape_image(rect.size(), ppp));
    blit(painter, &texture, rect.expand(device(TAPE_MARGIN, ppp).round() / ppp));
}

/// Paper on the quad `corners`, clockwise from the top left; `size` is the paper's own size
pub fn paint_paper(painter: &Painter, corners: [Pos2; 4], size: Vec2) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let key = ("paper", size.x.to_bits(), size.y.to_bits(), ppp.to_bits());
    let texture = cached(ctx, key, || paper_image(size, ppp));
    let mut mesh = Mesh::with_texture(texture.id());
    let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    for (pos, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(Vertex { pos, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nih_plug_egui::egui::vec2;

    fn luminance(color: Color32) -> f32 {
        0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32
    }

    #[test]
    fn test_lit_lamp_is_redder_and_brighter_than_an_unlit_one() {
        let off = Color32::from_rgb(90, 26, 18);
        let (lit, dark) =
            (lamp_image(true, off, false, 1.0, 1.0), lamp_image(false, off, false, 1.0, 1.0));
        let middle = |image: &ColorImage| {
            image.pixels[image.size[1] / 2 * image.size[0] + image.size[0] / 2 + 1]
        };
        assert!(luminance(middle(&lit)) > luminance(middle(&dark)) + 40.0);
        // Only a lit lamp glows past its bezel
        let edge = |image: &ColorImage| image.pixels[image.size[1] / 2 * image.size[0] + 1].a();
        assert!(edge(&lit) > 0 && edge(&dark) == 0);
    }

    #[test]
    fn test_pressed_footswitch_is_darker_than_a_raised_one() {
        let (up, down) = (stomp_image(1.0, false, 1.0), stomp_image(1.0, true, 1.0));
        let sum = |image: &ColorImage| image.pixels.iter().map(|c| luminance(*c)).sum::<f32>();
        assert!(sum(&up) > sum(&down));
        assert_eq!(up.size, [104, 104]);
    }

    #[test]
    fn test_tape_is_dark_and_paper_is_cream() {
        let tape = tape_image(vec2(80.0, 20.0), 1.0);
        assert!(luminance(tape.pixels[tape.size[1] / 2 * tape.size[0] + tape.size[0] / 2]) < 60.0);
        let paper = paper_image(vec2(100.0, 40.0), 1.0);
        assert!(luminance(paper.pixels[paper.pixels.len() / 2]) > 180.0);
    }
}
