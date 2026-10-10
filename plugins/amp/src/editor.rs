use nih_plug::prelude::*;
use nih_plug_egui::egui::vec2;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;
use suite_common::ui::{self, footswitch, led, param_knob, Ornament, PedalStyle};

use crate::GuitarAmpParams;

const WINDOW_WIDTH: u32 = 960;
const WINDOW_HEIGHT: u32 = 340;

// Stub: the head, its paint and its ornament come with milestone 1
const STYLE: PedalStyle = PedalStyle {
    name: "Guitar Amp",
    model: "GA-3",
    paint: ui::theme::PAINT_ORANGE,
    ornament: Ornament::Arrows,
    jacks: ["Stereo Out", "In"],
    band_y: 210.0,
};

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(params: Arc<GuitarAmpParams>, editor_state: Arc<EguiState>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |egui_ctx, _| ui::install(egui_ctx),
        move |egui_ctx, setter, _state| {
            ui::pedal(egui_ctx, &STYLE, |ui, origin| {
                let knobs = [
                    (&params.gain, "Gain"),
                    (&params.bass, "Bass"),
                    (&params.mid, "Mid"),
                    (&params.treble, "Treble"),
                    (&params.presence, "Presence"),
                    (&params.master, "Master"),
                    (&params.out_level, "Output"),
                ];
                for (index, (param, label)) in knobs.into_iter().enumerate() {
                    let x = 120.0 + 120.0 * index as f32;
                    param_knob(ui, setter, param, origin + vec2(x, 100.0), 30.0, label, None);
                }

                let bypassed = params.bypass.value();
                led(ui.painter(), origin + vec2(380.0, 285.0), !bypassed);
                if footswitch(ui, origin + vec2(480.0, 285.0), "bypass").clicked() {
                    setter.begin_set_parameter(&params.bypass);
                    setter.set_parameter(&params.bypass, !bypassed);
                    setter.end_set_parameter(&params.bypass);
                }
            });
        },
    )
}
