use std::sync::Arc;

use nih_plug_egui::egui::text::LayoutJob;
use nih_plug_egui::egui::{
    Align2, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Galley, Painter, Pos2,
    Rect, TextFormat,
};

/// Window background around the enclosure
pub const BENCH: Color32 = Color32::from_rgb(18, 17, 16);
/// Screen print on the paint, knob pointers, the name band
pub const SILK: Color32 = Color32::from_rgb(234, 223, 200);
/// Print on the name band
pub const INK: Color32 = Color32::from_rgb(42, 38, 34);
/// Logo plate
pub const PANEL: Color32 = Color32::from_rgb(28, 26, 24);
/// Pointer of the knob in the logo. The same on every plugin
pub const BRAND_ORANGE: Color32 = Color32::from_rgb(226, 88, 30);

/// Enclosure paint, one per plugin
pub const PAINT_ORANGE: Color32 = Color32::from_rgb(222, 86, 30);
pub const PAINT_TEAL: Color32 = Color32::from_rgb(29, 130, 130);
pub const PAINT_OXBLOOD: Color32 = Color32::from_rgb(124, 28, 32);
pub const PAINT_COBALT: Color32 = Color32::from_rgb(30, 64, 156);
pub const PAINT_PLUM: Color32 = Color32::from_rgb(96, 44, 120);
pub const PAINT_PINE: Color32 = Color32::from_rgb(28, 104, 60);

/// The paint of every plugin
pub const PLUGIN_PAINTS: [Color32; 6] =
    [PAINT_ORANGE, PAINT_TEAL, PAINT_OXBLOOD, PAINT_COBALT, PAINT_PLUM, PAINT_PINE];

/// Paint of the mini pedals of a rig, one each. The head keeps the plugin's paint
pub const PAINT_GRANITE: Color32 = Color32::from_rgb(92, 94, 96);
pub const PAINT_MOSS: Color32 = Color32::from_rgb(104, 120, 20);
pub const PAINT_OCHRE: Color32 = Color32::from_rgb(176, 112, 16);
pub const PAINT_VIOLET: Color32 = Color32::from_rgb(84, 48, 140);
pub const PAINT_BARK: Color32 = Color32::from_rgb(116, 80, 44);
pub const PAINT_SEA: Color32 = Color32::from_rgb(32, 72, 172);
pub const PAINT_AURORA: Color32 = Color32::from_rgb(24, 128, 64);
pub const PAINT_HEATHER: Color32 = Color32::from_rgb(148, 48, 104);
pub const PAINT_PETROL: Color32 = Color32::from_rgb(14, 100, 108);
pub const PAINT_FJORD: Color32 = Color32::from_rgb(40, 56, 84);
/// Every mini pedal paint. A new one must pass the tests below
pub const PEDAL_PAINTS: [Color32; 10] = [
    PAINT_GRANITE,
    PAINT_MOSS,
    PAINT_OCHRE,
    PAINT_VIOLET,
    PAINT_BARK,
    PAINT_SEA,
    PAINT_AURORA,
    PAINT_HEATHER,
    PAINT_PETROL,
    PAINT_FJORD,
];

pub const KNOB_SKIRT: Color32 = Color32::from_rgb(21, 19, 17);
pub const KNOB_CAP: Color32 = Color32::from_rgb(38, 34, 30);
pub const KNOB_CAP_TOP: Color32 = Color32::from_rgb(46, 42, 37);
pub const KNOB_CAP_EDGE: Color32 = Color32::from_rgb(74, 68, 61);

pub const TAPE: Color32 = Color32::from_rgb(20, 18, 16);
pub const TAPE_TEXT: Color32 = Color32::from_rgb(244, 239, 228);

pub const CHROME_LIGHT: Color32 = Color32::from_rgb(207, 203, 194);
pub const CHROME: Color32 = Color32::from_rgb(170, 165, 155);
pub const CHROME_DARK: Color32 = Color32::from_rgb(120, 115, 107);
pub const CHROME_SHINE: Color32 = Color32::from_rgb(246, 245, 241);

pub const LED_ON: Color32 = Color32::from_rgb(255, 59, 36);
pub const LED_OFF: Color32 = Color32::from_rgb(90, 26, 18);
// Unlit lens on the dark paint of a rig
pub const LED_OFF_DARK: Color32 = Color32::from_rgb(58, 12, 10);

/// Patch cable on the bench
pub const CABLE: Color32 = Color32::from_rgb(74, 68, 61);

pub const SHADOW: Color32 = Color32::from_black_alpha(80);

/// `from` mixed with `to`: 0 is all `from`, 1 all `to`
pub fn blend(from: Color32, to: Color32, share: f32) -> Color32 {
    let channel = |from: u8, to: u8| {
        (from as f32 + (to as f32 - from as f32) * share.clamp(0.0, 1.0)).round() as u8
    };
    Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}

/// A darker tone of `paint` for the second coat of a pedal's art: 0 is the paint, 1 black
pub fn shade(paint: Color32, depth: f32) -> Color32 {
    blend(paint, Color32::BLACK, depth)
}

const DISPLAY: &str = "display";
const MODEL: &str = "model";
const MONO: &str = "mono";

/// Register the suite's fonts. Call once from the build closure of `create_egui_editor`.
pub fn install(ctx: &Context) {
    let faces: [(&str, &'static [u8]); 3] = [
        (DISPLAY, include_bytes!("../../assets/fonts/BebasNeue-Regular.ttf")),
        (MODEL, include_bytes!("../../assets/fonts/BowlbyOneSC-Regular.ttf")),
        (MONO, include_bytes!("../../assets/fonts/IBMPlexMono-Medium.ttf")),
    ];

    let mut fonts = FontDefinitions::default();
    for (name, bytes) in faces {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        fonts.families.insert(FontFamily::Name(name.into()), vec![name.to_owned()]);
    }
    ctx.set_fonts(fonts);
}

/// Condensed capitals: labels and the logo
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY.into()))
}

/// Heavy letters of the name band
pub fn model(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MODEL.into()))
}

/// Parameter values
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MONO.into()))
}

/// Single line of text with letter spacing given as a fraction of the font size
pub fn layout(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    spacing: f32,
) -> Arc<Galley> {
    let format = TextFormat {
        extra_letter_spacing: font.size * spacing,
        font_id: font,
        color,
        ..Default::default()
    };
    painter.layout_job(LayoutJob::single_section(text.to_owned(), format))
}

/// Paint a line of spaced text anchored at `pos`. Returns the rectangle it covers.
pub fn spaced_text(
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    spacing: f32,
) -> Rect {
    let galley = layout(painter, text, font, color, spacing);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance of a colour, as contrast ratios are counted
    fn luminance(color: Color32) -> f32 {
        let linear = |channel: u8| {
            let value = channel as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    fn silk_contrast(paint: Color32) -> f32 {
        (luminance(SILK) + 0.05) / (luminance(paint) + 0.05)
    }

    fn distance(a: Color32, b: Color32) -> f32 {
        let square = |a: u8, b: u8| (a as f32 - b as f32).powi(2);
        (square(a.r(), b.r()) + square(a.g(), b.g()) + square(a.b(), b.b())).sqrt()
    }

    #[test]
    fn test_cream_print_reads_on_every_pedal_paint() {
        // The orange of the Pitch Shifter is the lightest paint print is known to read on
        let least = silk_contrast(PAINT_ORANGE);
        for paint in PEDAL_PAINTS {
            assert!(
                silk_contrast(paint) >= least,
                "{paint:?}: {} under {least}",
                silk_contrast(paint)
            );
        }
    }

    #[test]
    fn test_plugin_paints_take_cream_print_and_are_clearly_apart() {
        let least = silk_contrast(PAINT_ORANGE);
        for (i, &a) in PLUGIN_PAINTS.iter().enumerate() {
            assert!(silk_contrast(a) >= least, "{a:?}: {} under {least}", silk_contrast(a));
            for &b in &PLUGIN_PAINTS[i + 1..] {
                assert!(distance(a, b) >= 45.0, "{a:?} and {b:?} are {} apart", distance(a, b));
            }
        }
    }

    #[test]
    fn test_pedal_paints_are_clearly_apart() {
        let mut paints = PEDAL_PAINTS.to_vec();
        paints.push(PAINT_OXBLOOD);
        for (i, &a) in paints.iter().enumerate() {
            for &b in &paints[i + 1..] {
                assert!(distance(a, b) >= 45.0, "{a:?} and {b:?} are {} apart", distance(a, b));
            }
        }
    }
}
