use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use suite_common::ui::{dial, BACKGROUND, LABEL_TEXT, TITLE_TEXT, VALUE_TEXT};

use crate::DrumSynthParams;

const WINDOW_WIDTH: u32 = 300;
const WINDOW_HEIGHT: u32 = 260;

pub const GAIN_MIN_DB: f32 = -30.0;
pub const GAIN_MAX_DB: f32 = 6.0;

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

                        ui.add_space(20.0);

                        // Gain dial, edited in dB
                        let gain_db = util::gain_to_db(params.gain.value());
                        let response = dial(ui, 60.0, gain_db, GAIN_MIN_DB, GAIN_MAX_DB, false);

                        if response.dragged() {
                            let delta = -response.drag_delta().y * 0.15;
                            let new_db = (gain_db + delta).clamp(GAIN_MIN_DB, GAIN_MAX_DB);
                            setter.begin_set_parameter(&params.gain);
                            setter.set_parameter(&params.gain, util::db_to_gain(new_db));
                            setter.end_set_parameter(&params.gain);
                        }

                        ui.add_space(5.0);
                        ui.label(
                            egui::RichText::new(format!("{:.1} dB", gain_db))
                                .size(18.0)
                                .color(VALUE_TEXT),
                        );
                        ui.label(
                            egui::RichText::new("Gain")
                                .size(12.0)
                                .color(LABEL_TEXT),
                        );
                    });
                });
        },
    )
}
