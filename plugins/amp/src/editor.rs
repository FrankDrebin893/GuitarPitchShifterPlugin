use nih_plug::prelude::*;
use nih_plug_egui::egui::{vec2, Rect};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;
use suite_common::ui::{self, led, param_knob, silk_label, small_footswitch, Ornament, PedalStyle, BENCH_MARGIN};

use crate::{Amp, GuitarAmpParams};

const WINDOW_WIDTH: u32 = 960;
const WINDOW_HEIGHT: u32 = 340;

// The head keeps this height when the window grows for the pedals below it
const HEAD_HEIGHT: f32 = 320.0;

const STYLE: PedalStyle = PedalStyle {
    name: "Guitar Amp",
    model: "GA-3",
    paint: ui::theme::PAINT_OXBLOOD,
    ornament: Ornament::Bolts,
    jacks: ["Stereo Out", "In"],
    band_y: 202.0,
};

// The amp's own six dials in an even row, and the output level set apart from them
const KNOB_Y: f32 = 80.0;
const KNOB_RADIUS: f32 = 30.0;
const DIAL_X: f32 = 150.0;
const DIAL_SPACING: f32 = 100.0;
const OUTPUT_X: f32 = 790.0;

// Small switches below the band. Left of each, its LED above its caption. The amp selector
// sits under the dials, the bypass under the output knob
const SWITCH_Y: f32 = 286.0;
const AMP_SWITCHES: [(Amp, &str, f32); 3] = [(Amp::Klar, "Klar", 295.0), (Amp::Brol, "Brøl", 425.0), (Amp::Torden, "Torden", 555.0)];
const BYPASS_X: f32 = 790.0;
const CAPTION_OFFSET: f32 = -58.0;
const LED_Y: f32 = 275.0;
const CAPTION_Y: f32 = 301.0;
const CAPTION_SIZE: f32 = 17.0;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(params: Arc<GuitarAmpParams>, editor_state: Arc<EguiState>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |egui_ctx, _| ui::install(egui_ctx),
        move |egui_ctx, setter, _state| {
            ui::rig(egui_ctx, |ui, origin| {
                let head = Rect::from_min_size(
                    origin + vec2(BENCH_MARGIN, BENCH_MARGIN),
                    vec2(WINDOW_WIDTH as f32 - 2.0 * BENCH_MARGIN, HEAD_HEIGHT),
                );
                ui::head(ui, head, &STYLE);

                let dials = [
                    (&params.gain, "Gain"),
                    (&params.bass, "Bass"),
                    (&params.mid, "Mid"),
                    (&params.treble, "Treble"),
                    (&params.presence, "Presence"),
                    (&params.master, "Master"),
                ];
                for (index, (param, label)) in dials.into_iter().enumerate() {
                    let x = DIAL_X + DIAL_SPACING * index as f32;
                    param_knob(ui, setter, param, origin + vec2(x, KNOB_Y), KNOB_RADIUS, label, None);
                }
                param_knob(ui, setter, &params.out_level, origin + vec2(OUTPUT_X, KNOB_Y), KNOB_RADIUS, "Output", None);

                for (amp, label, x) in AMP_SWITCHES {
                    led(ui.painter(), origin + vec2(x + CAPTION_OFFSET, LED_Y), params.amp.value() == amp);
                    silk_label(ui.painter(), origin + vec2(x + CAPTION_OFFSET, CAPTION_Y), label, CAPTION_SIZE);
                    if small_footswitch(ui, origin + vec2(x, SWITCH_Y), label).clicked() {
                        setter.begin_set_parameter(&params.amp);
                        setter.set_parameter(&params.amp, amp);
                        setter.end_set_parameter(&params.amp);
                    }
                }

                // Bypass switch, lit while the amp is on
                let bypassed = params.bypass.value();
                led(ui.painter(), origin + vec2(BYPASS_X + CAPTION_OFFSET, LED_Y), !bypassed);
                silk_label(ui.painter(), origin + vec2(BYPASS_X + CAPTION_OFFSET, CAPTION_Y), "On", CAPTION_SIZE);
                if small_footswitch(ui, origin + vec2(BYPASS_X, SWITCH_Y), "bypass").clicked() {
                    setter.begin_set_parameter(&params.bypass);
                    setter.set_parameter(&params.bypass, !bypassed);
                    setter.end_set_parameter(&params.bypass);
                }
            });
        },
    )
}
