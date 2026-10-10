use std::sync::Arc;

use nih_plug_egui::egui::text::LayoutJob;
use nih_plug_egui::egui::{
    Align2, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Galley, Painter, Pos2, Rect, TextFormat,
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

pub const SHADOW: Color32 = Color32::from_black_alpha(80);

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
pub fn layout(painter: &Painter, text: &str, font: FontId, color: Color32, spacing: f32) -> Arc<Galley> {
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
