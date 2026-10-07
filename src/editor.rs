use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

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
                .frame(egui::Frame::default().fill(Color32::from_rgb(30, 30, 35)))
                .show(egui_ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(15.0);

                        // Title
                        ui.label(
                            egui::RichText::new("PITCH SHIFTER")
                                .size(24.0)
                                .color(Color32::from_rgb(200, 200, 210)),
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
                                    .color(Color32::from_rgb(180, 180, 190)),
                            );
                            ui.label(
                                egui::RichText::new("Semitones")
                                    .size(12.0)
                                    .color(Color32::from_rgb(120, 120, 130)),
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
                                        .color(Color32::from_rgb(180, 180, 190)),
                                );
                                ui.label(
                                    egui::RichText::new("Max Latency")
                                        .size(10.0)
                                        .color(Color32::from_rgb(120, 120, 130)),
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
                                        .color(Color32::from_rgb(180, 180, 190)),
                                );
                                ui.label(
                                    egui::RichText::new("Smoothness")
                                        .size(10.0)
                                        .color(Color32::from_rgb(120, 120, 130)),
                                );
                            });
                        });
                    });
                });
        },
    )
}

/// Draw a dial/knob widget
fn dial(ui: &mut Ui, radius: f32, value: f32, min: f32, max: f32, is_bipolar: bool) -> Response {
    let desired_size = Vec2::splat(radius * 2.0 + 10.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::drag());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let center = rect.center();

        // Normalize value to 0-1 range
        let normalized = (value - min) / (max - min);

        // Arc angles (from bottom-left to bottom-right, ~270 degrees)
        let start_angle = std::f32::consts::PI * 0.75;
        let end_angle = std::f32::consts::PI * 2.25;
        let angle_range = end_angle - start_angle;

        // Background arc
        draw_arc(
            painter,
            center,
            radius,
            start_angle,
            end_angle,
            Stroke::new(4.0, Color32::from_rgb(60, 60, 70)),
        );

        // Value arc
        let value_color = if is_bipolar {
            if value >= 0.0 {
                Color32::from_rgb(100, 180, 255)
            } else {
                Color32::from_rgb(255, 140, 100)
            }
        } else {
            Color32::from_rgb(100, 180, 255)
        };

        if is_bipolar {
            // Bipolar: draw from center
            let center_angle = start_angle + angle_range * 0.5;
            let value_angle = start_angle + angle_range * normalized;
            let (arc_start, arc_end) = if value_angle > center_angle {
                (center_angle, value_angle)
            } else {
                (value_angle, center_angle)
            };
            draw_arc(
                painter,
                center,
                radius,
                arc_start,
                arc_end,
                Stroke::new(4.0, value_color),
            );
        } else {
            // Unipolar: draw from start
            let value_angle = start_angle + angle_range * normalized;
            draw_arc(
                painter,
                center,
                radius,
                start_angle,
                value_angle,
                Stroke::new(4.0, value_color),
            );
        }

        // Indicator dot
        let indicator_angle = start_angle + angle_range * normalized;
        let indicator_pos = Pos2::new(
            center.x + indicator_angle.cos() * (radius - 12.0),
            center.y + indicator_angle.sin() * (radius - 12.0),
        );
        painter.circle_filled(indicator_pos, 5.0, Color32::WHITE);

        // Center circle
        painter.circle_filled(center, radius * 0.5, Color32::from_rgb(45, 45, 55));
        painter.circle_stroke(center, radius * 0.5, Stroke::new(2.0, Color32::from_rgb(70, 70, 80)));
    }

    response
}

/// Draw an arc using line segments
fn draw_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    stroke: Stroke,
) {
    let segments = 32;
    let angle_step = (end_angle - start_angle) / segments as f32;

    for i in 0..segments {
        let a1 = start_angle + angle_step * i as f32;
        let a2 = start_angle + angle_step * (i + 1) as f32;

        let p1 = Pos2::new(center.x + a1.cos() * radius, center.y + a1.sin() * radius);
        let p2 = Pos2::new(center.x + a2.cos() * radius, center.y + a2.sin() * radius);

        painter.line_segment([p1, p2], stroke);
    }
}
