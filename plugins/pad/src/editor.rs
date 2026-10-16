use nih_plug::prelude::*;
use nih_plug_egui::egui::{pos2, vec2, Rect};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use suite_common::ui::{
    self, footswitch, led, param_knob, silk_frame, silk_label, Ornament, PedalStyle,
};

use crate::ShimmerPadParams;

const WINDOW_WIDTH: u32 = 660;
const WINDOW_HEIGHT: u32 = 600;

const STYLE: PedalStyle = PedalStyle {
    name: "Shimmer Pad",
    model: "SP-1",
    paint: ui::theme::PAINT_PLUM,
    ornament: Ornament::Waves,
    jacks: ["Stereo Out", "MIDI In"],
    band_y: 394.0,
    art: Some(ui::artwork::PAD),
};

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(
    params: Arc<ShimmerPadParams>,
    editor_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |egui_ctx, _| ui::install(egui_ctx),
        move |egui_ctx, setter, _state| {
            ui::pedal(egui_ctx, &STYLE, |ui, origin| {
                // The pad
                let pad_y = 98.0;
                param_knob(
                    ui,
                    setter,
                    &params.volume,
                    origin + vec2(110.0, pad_y),
                    28.0,
                    "Volume",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.attack,
                    origin + vec2(220.0, pad_y),
                    28.0,
                    "Attack",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.release,
                    origin + vec2(330.0, pad_y),
                    28.0,
                    "Release",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.cutoff,
                    origin + vec2(440.0, pad_y),
                    28.0,
                    "Cutoff",
                    None,
                );
                param_knob(ui, setter, &params.dry, origin + vec2(550.0, pad_y), 28.0, "Dry", None);

                let frames_y = 246.0;
                let voice = Rect::from_min_max(pos2(40.0, 206.0), pos2(296.0, 328.0))
                    .translate(origin.to_vec2());
                silk_frame(ui.painter(), voice, "Voice", STYLE.paint);
                param_knob(
                    ui,
                    setter,
                    &params.detune,
                    origin + vec2(88.0, frames_y),
                    19.0,
                    "Detune",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.breath,
                    origin + vec2(168.0, frames_y),
                    19.0,
                    "Breath",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.motion,
                    origin + vec2(248.0, frames_y),
                    19.0,
                    "Motion",
                    None,
                );

                let space = Rect::from_min_max(pos2(312.0, 206.0), pos2(620.0, 328.0))
                    .translate(origin.to_vec2());
                silk_frame(ui.painter(), space, "Space", STYLE.paint);
                param_knob(
                    ui,
                    setter,
                    &params.chorus,
                    origin + vec2(352.0, frames_y),
                    19.0,
                    "Chorus",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.reverb,
                    origin + vec2(428.0, frames_y),
                    19.0,
                    "Reverb",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.reverb_decay,
                    origin + vec2(504.0, frames_y),
                    19.0,
                    "Decay",
                    None,
                );
                param_knob(
                    ui,
                    setter,
                    &params.shimmer,
                    origin + vec2(580.0, frames_y),
                    19.0,
                    "Shimmer",
                    None,
                );

                for (param, label, x) in
                    [(&params.chorus_on, "Chorus", 265.0), (&params.shimmer_on, "Shimmer", 395.0)]
                {
                    let on = param.value();
                    led(ui.painter(), origin + vec2(x, 452.0), on);
                    if footswitch(ui, origin + vec2(x, 506.0), label).clicked() {
                        setter.begin_set_parameter(param);
                        setter.set_parameter(param, !on);
                        setter.end_set_parameter(param);
                    }
                    silk_label(ui.painter(), origin + vec2(x, 558.0), label, 17.0);
                }
            });
        },
    )
}
