use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use suite_common::ui::{dial, BACKGROUND, LABEL_TEXT, TITLE_TEXT, VALUE_TEXT};

use crate::GuitarPitchShifterParams;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 300;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(params: Arc<GuitarPitchShifterParams>, editor_state: Arc<EguiState>) -> Option<Box<dyn Editor>> {
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
                            egui::RichText::new("PITCH SHIFTER")
                                .size(24.0)
                                .color(TITLE_TEXT),
                        );

                        ui.add_space(20.0);

                        // Main semitones dial (large)
                        ui.vertical_centered(|ui| {
                            let semitones = params.semitones.value();
                            let response = dial(ui, 80.0, semitones as f32, -12.0, 12.0, true);

                            if response.dragged() {
                                let delta = -response.drag_delta().y * 0.1;
                                let new_value = (semitones as f32 + delta).clamp(-12.0, 12.0);
                                setter.begin_set_parameter(&params.semitones);
                                setter.set_parameter(&params.semitones, new_value.round() as i32);
                                setter.end_set_parameter(&params.semitones);
                            }

                            ui.add_space(5.0);
                            ui.label(
                                egui::RichText::new(format!("{:+} st", semitones))
                                    .size(18.0)
                                    .color(VALUE_TEXT),
                            );
                            ui.label(
                                egui::RichText::new("Semitones")
                                    .size(12.0)
                                    .color(LABEL_TEXT),
                            );
                        });

                        ui.add_space(20.0);

                        // Smaller dials for max latency and smoothness
                        ui.columns(2, |columns| {
                            // Max latency dial (left column)
                            columns[0].vertical_centered(|ui| {
                                let latency = params.latency_ms.value();
                                let response = dial(ui, 40.0, latency, 2.0, 50.0, false);

                                if response.dragged() {
                                    let delta = -response.drag_delta().y * 0.2;
                                    let new_value = (latency + delta).clamp(2.0, 50.0);
                                    setter.begin_set_parameter(&params.latency_ms);
                                    setter.set_parameter(&params.latency_ms, new_value);
                                    setter.end_set_parameter(&params.latency_ms);
                                }

                                ui.add_space(3.0);
                                ui.label(
                                    egui::RichText::new(format!("{:.1} ms", latency))
                                        .size(12.0)
                                        .color(VALUE_TEXT),
                                );
                                ui.label(
                                    egui::RichText::new("Max Latency")
                                        .size(10.0)
                                        .color(LABEL_TEXT),
                                );
                            });

                            // Smoothness dial (right column)
                            columns[1].vertical_centered(|ui| {
                                let smoothness = params.smoothness_ms.value();
                                let response = dial(ui, 40.0, smoothness, 0.5, 10.0, false);

                                if response.dragged() {
                                    let delta = -response.drag_delta().y * 0.05;
                                    let new_value = (smoothness + delta).clamp(0.5, 10.0);
                                    setter.begin_set_parameter(&params.smoothness_ms);
                                    setter.set_parameter(&params.smoothness_ms, new_value);
                                    setter.end_set_parameter(&params.smoothness_ms);
                                }

                                ui.add_space(3.0);
                                ui.label(
                                    egui::RichText::new(format!("{:.1} ms", smoothness))
                                        .size(12.0)
                                        .color(VALUE_TEXT),
                                );
                                ui.label(
                                    egui::RichText::new("Smoothness")
                                        .size(10.0)
                                        .color(LABEL_TEXT),
                                );
                            });
                        });
                    });
                });
        },
    )
}
