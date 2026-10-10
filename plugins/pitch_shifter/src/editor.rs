use nih_plug::prelude::*;
use nih_plug_egui::egui::vec2;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use suite_common::ui::{self, footswitch, led, param_knob, Ornament, PedalStyle};

use crate::GuitarPitchShifterParams;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 560;

const STYLE: PedalStyle = PedalStyle {
    name: "Pitch Shifter",
    model: "PS-10",
    paint: ui::theme::PAINT_ORANGE,
    ornament: Ornament::Arrows,
    jacks: ["Out", "In"],
    band_y: 367.0,
};

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(params: Arc<GuitarPitchShifterParams>, editor_state: Arc<EguiState>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |egui_ctx, _| ui::install(egui_ctx),
        move |egui_ctx, setter, _state| {
            ui::pedal(egui_ctx, &STYLE, |ui, origin| {
                // Three knobs in a triangle
                param_knob(ui, setter, &params.latency_ms, origin + vec2(83.0, 103.0), 27.0, "Max Latency", None);
                param_knob(ui, setter, &params.smoothness_ms, origin + vec2(317.0, 103.0), 27.0, "Smoothness", None);
                param_knob(
                    ui,
                    setter,
                    &params.semitones,
                    origin + vec2(200.0, 205.0),
                    48.0,
                    "Semitones",
                    Some(["-12", "0", "+12"]),
                );

                // Bypass switch, lit while the effect is on
                let bypassed = params.bypass.value();
                led(ui.painter(), origin + vec2(200.0, 418.0), !bypassed);
                if footswitch(ui, origin + vec2(200.0, 478.0), "bypass").clicked() {
                    setter.begin_set_parameter(&params.bypass);
                    setter.set_parameter(&params.bypass, !bypassed);
                    setter.end_set_parameter(&params.bypass);
                }
            });
        },
    )
}
