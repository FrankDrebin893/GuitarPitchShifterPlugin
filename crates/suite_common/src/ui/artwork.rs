//! Bespoke artwork: PNG pictures printed on the top face of an enclosure, under the controls.
//!
//! The pictures are files in `assets/art/`, made by `scripts/make-art.py` or by hand (painted,
//! drawn, generated: anything that ends up as an RGBA PNG). A picture is scaled to cover the
//! top face, cut to its rounded corners and kept as a texture. Nothing else knows how it was made.

use nih_plug_egui::egui::{Color32, ColorImage, Painter, Rect};

use super::raster::{blit, cached, rounded_box};

/// The artwork of the Pitch Shifter
pub const PITCH_SHIFTER: &[u8] = include_bytes!("../../assets/art/pitch_shifter.png");
/// The artwork of the Drum Synth
pub const DRUMS: &[u8] = include_bytes!("../../assets/art/drums.png");
/// The artwork of the Bass Synth
pub const BASS: &[u8] = include_bytes!("../../assets/art/bass.png");
/// The artwork of the Shimmer Pad
pub const PAD: &[u8] = include_bytes!("../../assets/art/pad.png");
/// The artwork of the Glass Bells
pub const BELLS: &[u8] = include_bytes!("../../assets/art/bells.png");
/// The artwork of the Guitar Amp's head
pub const AMP: &[u8] = include_bytes!("../../assets/art/amp.png");

struct Picture {
    width: usize,
    height: usize,
    /// Straight RGBA, 8 bits
    rgba: Vec<u8>,
}

fn decode(png_bytes: &[u8]) -> Option<Picture> {
    let mut decoder = png::Decoder::new(png_bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return None,
    };
    let (width, height) = (info.width as usize, info.height as usize);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in buffer[..width * height * channels].chunks_exact(channels) {
        rgba.extend_from_slice(&match channels {
            4 => [pixel[0], pixel[1], pixel[2], pixel[3]],
            3 => [pixel[0], pixel[1], pixel[2], 255],
            2 => [pixel[0], pixel[0], pixel[0], pixel[1]],
            _ => [pixel[0], pixel[0], pixel[0], 255],
        });
    }
    Some(Picture { width, height, rgba })
}

impl Picture {
    /// Bilinear sample at (`x`, `y`) in picture pixels, as premultiplied floats
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = (x - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (y - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| {
            let p = &self.rgba[(y * self.width + x) * 4..][..4];
            let a = p[3] as f32 / 255.0;
            [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, a * 255.0]
        };
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bottom = c[i] + (d[i] - c[i]) * fx;
            out[i] = top + (bottom - top) * fy;
        }
        out
    }
}

/// The picture scaled to cover `width` x `height` pixels, with its corners cut round
pub fn fit_image(png_bytes: &[u8], width: usize, height: usize, radius: f32) -> ColorImage {
    let mut pixels = vec![Color32::TRANSPARENT; width * height];
    let Some(picture) = decode(png_bytes) else {
        return ColorImage { size: [width, height], pixels };
    };
    // Cover: the picture's shorter side fills the face, the rest is cut off
    let scale = (width as f32 / picture.width as f32).max(height as f32 / picture.height as f32);
    let (offset_x, offset_y) = (
        (picture.width as f32 * scale - width as f32) / 2.0,
        (picture.height as f32 * scale - height as f32) / 2.0,
    );
    let (c, half) =
        ([width as f32 / 2.0, height as f32 / 2.0], [width as f32 / 2.0, height as f32 / 2.0]);
    for y in 0..height {
        for x in 0..width {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let cut = (0.5 - rounded_box(p, c, half, radius)).clamp(0.0, 1.0);
            if cut <= 0.0 {
                continue;
            }
            let s = picture.sample((p[0] + offset_x) / scale, (p[1] + offset_y) / scale);
            let channel = |v: f32| (v * cut).round().clamp(0.0, 255.0) as u8;
            let alpha = channel(s[3]);
            pixels[y * width + x] = Color32::from_rgba_premultiplied(
                channel(s[0]).min(alpha),
                channel(s[1]).min(alpha),
                channel(s[2]).min(alpha),
                alpha,
            );
        }
    }
    ColorImage { size: [width, height], pixels }
}

/// Print `png_bytes` over `top`, the top face of an enclosure with corners of `radius`
pub fn paint_art(painter: &Painter, top: Rect, radius: f32, png_bytes: &'static [u8]) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let key = (
        "art",
        png_bytes.as_ptr() as usize,
        top.width().to_bits(),
        top.height().to_bits(),
        radius.to_bits(),
        ppp.to_bits(),
    );
    let texture = cached(ctx, key, || {
        let width = (top.width() * ppp).round().max(1.0) as usize;
        let height = (top.height() * ppp).round().max(1.0) as usize;
        fit_image(png_bytes, width, height, radius * ppp)
    });
    blit(painter, &texture, top);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_art_decodes_and_is_mostly_clear_in_the_middle() {
        for art in [PITCH_SHIFTER, AMP, DRUMS, BASS, PAD, BELLS] {
            let picture = decode(art).expect("the PNG decodes");
            assert!(picture.width >= 400 && picture.height >= 300);
            let alpha_at = |x: usize, y: usize| picture.rgba[(y * picture.width + x) * 4 + 3];
            let middle: u32 = (0..8)
                .map(|i| alpha_at(picture.width / 2 + i, picture.height / 2 + i) as u32)
                .sum();
            assert!(middle / 8 < 40, "the middle is for the controls");
        }
    }

    #[test]
    fn test_art_is_cut_to_the_rounded_corners() {
        let image = fit_image(AMP, 400, 150, 24.0);
        assert_eq!(image.pixels[0], Color32::TRANSPARENT);
        assert!(image.pixels.iter().any(|pixel| pixel.a() > 0));
    }

    #[test]
    fn test_a_picture_that_does_not_decode_leaves_the_face_bare() {
        let image = fit_image(b"not a png", 20, 10, 4.0);
        assert!(image.pixels.iter().all(|pixel| *pixel == Color32::TRANSPARENT));
    }

    /// Writes the lit box with its art as raw RGB (width, height, pixels) to `ART_PREVIEW`
    #[test]
    #[ignore]
    fn preview() {
        use crate::ui::raster::box_image;
        use nih_plug_egui::egui::vec2;
        let Ok(path) = std::env::var("ART_PREVIEW") else { return };
        let (ppp, size) = (1.0, vec2(960.0, 354.0));
        let (base, margin) = box_image(size, 24.0, 5.0, crate::ui::theme::PAINT_OXBLOOD, ppp);
        let art = fit_image(AMP, size.x as usize, size.y as usize - 5, 24.0);
        let (w, h) = (base.size[0], base.size[1]);
        let mut out = Vec::new();
        out.extend((w as u32).to_le_bytes());
        out.extend((h as u32).to_le_bytes());
        for y in 0..h {
            for x in 0..w {
                let b = base.pixels[y * w + x];
                let (ax, ay) = (x as isize - margin as isize, y as isize - margin as isize);
                let a = if ax >= 0
                    && ay >= 0
                    && (ax as usize) < art.size[0]
                    && (ay as usize) < art.size[1]
                {
                    art.pixels[ay as usize * art.size[0] + ax as usize]
                } else {
                    Color32::TRANSPARENT
                };
                let k = 1.0 - a.a() as f32 / 255.0;
                let ch = |b: u8, a: u8| {
                    (a as f32 + b as f32 * k + 18.0 * (1.0 - b as f32 / 255.0).max(0.0) * 0.0) as u8
                };
                out.extend([ch(b.r(), a.r()), ch(b.g(), a.g()), ch(b.b(), a.b())]);
            }
        }
        std::fs::write(path, out).unwrap();
    }
}
