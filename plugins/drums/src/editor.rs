use nih_plug::prelude::*;
use nih_plug_egui::egui::{pos2, vec2, Rect};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use suite_common::ui::{self, footswitch, led, param_knob, silk_frame, silk_label, Ornament, PedalStyle};

use crate::kit::Kit;
use crate::DrumSynthParams;

const WINDOW_WIDTH: u32 = 660;
const WINDOW_HEIGHT: u32 = 600;

const STYLE: PedalStyle = PedalStyle {
    name: "Drum Synth",
    model: "DS-2",
    paint: ui::theme::PAINT_TEAL,
    ornament: Ornament::Rings,
    jacks: ["Stereo Out", "MIDI In"],
    band_y: 394.0,
};

// One footswitch per kit, with its horizontal position
const KIT_SWITCHES: [(Kit, &str, f32); 3] = [(Kit::Rock, "Rock", 200.0), (Kit::Jazz, "Jazz", 330.0), (Kit::Metal, "Metal", 460.0)];

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(params: Arc<DrumSynthParams>, editor_state: Arc<EguiState>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |egui_ctx, _| ui::install(egui_ctx),
        move |egui_ctx, setter, _state| {
            ui::pedal(egui_ctx, &STYLE, |ui, origin| {
                // Overall sound
                let sound_y = 98.0;
                param_knob(ui, setter, &params.gain, origin + vec2(130.0, sound_y), 31.0, "Gain", None);
                param_knob(ui, setter, &params.room, origin + vec2(263.0, sound_y), 31.0, "Room", None);
                param_knob(ui, setter, &params.tune, origin + vec2(397.0, sound_y), 31.0, "Tune", None);
                param_knob(ui, setter, &params.damping, origin + vec2(530.0, sound_y), 31.0, "Damping", None);

                // Mixer
                let frame = Rect::from_min_max(pos2(40.0, 206.0), pos2(620.0, 328.0)).translate(origin.to_vec2());
                silk_frame(ui.painter(), frame, "Mixer", STYLE.paint);
                let mixer_y = 246.0;
                param_knob(ui, setter, &params.kick, origin + vec2(100.0, mixer_y), 19.0, "Kick", None);
                param_knob(ui, setter, &params.snare, origin + vec2(215.0, mixer_y), 19.0, "Snare", None);
                param_knob(ui, setter, &params.toms, origin + vec2(330.0, mixer_y), 19.0, "Toms", None);
                param_knob(ui, setter, &params.hihat, origin + vec2(445.0, mixer_y), 19.0, "Hi-Hat", None);
                param_knob(ui, setter, &params.cymbals, origin + vec2(560.0, mixer_y), 19.0, "Cymbals", None);

                // Kit selector
                for (kit, label, x) in KIT_SWITCHES {
                    led(ui.painter(), origin + vec2(x, 452.0), params.kit.value() == kit);
                    if footswitch(ui, origin + vec2(x, 506.0), label).clicked() {
                        setter.begin_set_parameter(&params.kit);
                        setter.set_parameter(&params.kit, kit);
                        setter.end_set_parameter(&params.kit);
                    }
                    silk_label(ui.painter(), origin + vec2(x, 558.0), label, 17.0);
                }
            });
        },
    )
}
