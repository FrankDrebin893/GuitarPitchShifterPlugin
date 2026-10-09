use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use suite_common::ui::{dial, ACCENT, BACKGROUND, DIAL_CENTER, LABEL_TEXT, TITLE_TEXT, VALUE_TEXT};

use crate::kit::Kit;
use crate::DrumSynthParams;

const WINDOW_WIDTH: u32 = 620;
const WINDOW_HEIGHT: u32 = 440;

const KIT_BUTTONS: [(Kit, &str); 3] = [(Kit::Rock, "ROCK"), (Kit::Jazz, "JAZZ"), (Kit::Metal, "METAL")];
const KIT_BUTTON_SIZE: egui::Vec2 = egui::vec2(110.0, 30.0);

// Normalized parameter change per pixel of vertical drag
const DRAG_SENSITIVITY: f32 = 0.005;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(params: Arc<DrumSynthParams>, editor_state: Arc<EguiState>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |_, _| {},
        move |egui_ctx, setter, _state| {
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(BACKGROUND))
                .show(egui_ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(15.0);

                        // Title
                        ui.label(
                            egui::RichText::new("DRUM SYNTH")
                                .size(24.0)
                                .color(TITLE_TEXT),
                        );

                        ui.add_space(15.0);

                        // Kit selector
                        let selector_width = KIT_BUTTON_SIZE.x * 3.0 + 40.0;
                        ui.allocate_ui(egui::vec2(selector_width, KIT_BUTTON_SIZE.y), |ui| {
                            ui.columns(3, |columns| {
                                for (column, (kit, label)) in columns.iter_mut().zip(KIT_BUTTONS) {
                                    column.vertical_centered(|ui| kit_button(ui, setter, &params.kit, kit, label));
                                }
                            });
                        });

                        ui.add_space(20.0);

                        // Overall sound
                        ui.columns(4, |columns| {
                            param_dial(&mut columns[0], setter, &params.gain, "Gain", 40.0, false);
                            param_dial(&mut columns[1], setter, &params.room, "Room", 40.0, false);
                            param_dial(&mut columns[2], setter, &params.tune, "Tune", 40.0, true);
                            param_dial(&mut columns[3], setter, &params.damping, "Damping", 40.0, false);
                        });

                        ui.add_space(20.0);

                        // Mixer
                        ui.columns(5, |columns| {
                            param_dial(&mut columns[0], setter, &params.kick, "Kick", 28.0, false);
                            param_dial(&mut columns[1], setter, &params.snare, "Snare", 28.0, false);
                            param_dial(&mut columns[2], setter, &params.toms, "Toms", 28.0, false);
                            param_dial(&mut columns[3], setter, &params.hihat, "Hi-Hat", 28.0, false);
                            param_dial(&mut columns[4], setter, &params.cymbals, "Cymbals", 28.0, false);
                        });
                    });
                });
        },
    )
}

fn kit_button(ui: &mut egui::Ui, setter: &ParamSetter, param: &EnumParam<Kit>, kit: Kit, label: &str) {
    let selected = param.value() == kit;
    let text = egui::RichText::new(label)
        .size(14.0)
        .color(if selected { BACKGROUND } else { VALUE_TEXT });
    let button = egui::Button::new(text)
        .fill(if selected { ACCENT } else { DIAL_CENTER })
        .min_size(KIT_BUTTON_SIZE);

    if ui.add(button).clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, kit);
        setter.end_set_parameter(param);
    }
}

/// Dial with its value and name below. Drag up and down to change the parameter.
fn param_dial(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &FloatParam,
    label: &str,
    radius: f32,
    is_bipolar: bool,
) {
    ui.vertical_centered(|ui| {
        let normalized = param.unmodulated_normalized_value();
        let response = if is_bipolar {
            dial(ui, radius, normalized - 0.5, -0.5, 0.5, true)
        } else {
            dial(ui, radius, normalized, 0.0, 1.0, false)
        };

        if response.dragged() {
            let new_value = (normalized - response.drag_delta().y * DRAG_SENSITIVITY).clamp(0.0, 1.0);
            setter.begin_set_parameter(param);
            setter.set_parameter_normalized(param, new_value);
            setter.end_set_parameter(param);
        }

        ui.add_space(3.0);
        ui.label(
            egui::RichText::new(param.normalized_value_to_string(normalized, true))
                .size(12.0)
                .color(VALUE_TEXT),
        );
        ui.label(
            egui::RichText::new(label)
                .size(10.0)
                .color(LABEL_TEXT),
        );
    });
}
