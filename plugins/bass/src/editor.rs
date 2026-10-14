use nih_plug::prelude::*;
use nih_plug_egui::egui::{pos2, vec2, Rect};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use suite_common::ui::{
    self, footswitch, led, param_knob, silk_frame, silk_label, Ornament, PedalStyle,
};

use crate::bass::Voice;
use crate::BassSynthParams;

const WINDOW_WIDTH: u32 = 660;
const WINDOW_HEIGHT: u32 = 600;

const STYLE: PedalStyle = PedalStyle {
    name: "Bass Synth",
    model: "BA-1",
    paint: ui::theme::PAINT_COBALT,
    ornament: Ornament::Strings,
    jacks: ["Out", "MIDI In"],
    band_y: 394.0,
};

// One footswitch per voice, with its horizontal position
const VOICE_SWITCHES: [(Voice, &str, f32); 2] =
    [(Voice::Metal, "Metal", 265.0), (Voice::Groove, "Groove", 395.0)];

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

fn set<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

/// Picks a voice and puts the knobs where that bass has them, like a preset
fn pick_voice(setter: &ParamSetter, params: &BassSynthParams, voice: Voice) {
    let settings = voice.settings();
    set(setter, &params.voice, voice);
    set(setter, &params.volume, util::db_to_gain(settings.volume_db));
    set(setter, &params.drive, settings.drive);
    set(setter, &params.brightness, settings.brightness);
    set(setter, &params.sub, settings.sub);
    set(setter, &params.tone, settings.tone_hz);
    set(setter, &params.sustain, settings.sustain_s);
    set(setter, &params.slide, settings.slide_ms);
    set(setter, &params.transpose, settings.transpose);
}

pub fn create(
    params: Arc<BassSynthParams>,
    editor_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |egui_ctx, _| ui::install(egui_ctx),
        move |egui_ctx, setter, _state| {
            ui::pedal(egui_ctx, &STYLE, |ui, origin| {
                // Overall sound
                let sound_y = 98.0;
                param_knob(
                    ui,
                    setter,
                    &params.volume,
                    origin + vec2(130.0, sound_y),
                    31.0,
                    "Volume",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.drive,
                    origin + vec2(263.0, sound_y),
                    31.0,
                    "Drive",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.brightness,
                    origin + vec2(397.0, sound_y),
                    31.0,
                    "Brightness",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.sub,
                    origin + vec2(530.0, sound_y),
                    31.0,
                    "Sub",
                    None,
                );

                // The string
                let frame = Rect::from_min_max(pos2(40.0, 206.0), pos2(620.0, 328.0))
                    .translate(origin.to_vec2());
                silk_frame(ui.painter(), frame, "String", STYLE.paint);
                let string_y = 246.0;
                param_knob(
                    ui,
                    setter,
                    &params.tone,
                    origin + vec2(115.0, string_y),
                    19.0,
                    "Tone",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.sustain,
                    origin + vec2(258.0, string_y),
                    19.0,
                    "Sustain",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.slide,
                    origin + vec2(402.0, string_y),
                    19.0,
                    "Slide",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.transpose,
                    origin + vec2(545.0, string_y),
                    19.0,
                    "Transpose",
                    None,
                );

                // Voice selector
                for (voice, label, x) in VOICE_SWITCHES {
                    led(ui.painter(), origin + vec2(x, 452.0), params.voice.value() == voice);
                    if footswitch(ui, origin + vec2(x, 506.0), label).clicked() {
                        pick_voice(setter, &params, voice);
                    }
                    silk_label(ui.painter(), origin + vec2(x, 558.0), label, 17.0);
                }
            });
        },
    )
}
