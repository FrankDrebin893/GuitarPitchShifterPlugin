use nih_plug::prelude::*;
use nih_plug_egui::egui::{vec2, Rect, Vec2};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use suite_common::ui::{
    self, param_knob, param_switch, rig_led, silk_label, small_footswitch, stepper, tuner_display, Ornament,
    PedalStyle, Step, BENCH_MARGIN,
};

use crate::cab_stage::{CabLoader, CabStatus};
use crate::presets::{self, PRESETS};
use crate::tuner::{Note, TunerReading};
use crate::user_cab;
use crate::{Amp, GuitarAmpParams, GuitarAmpPlugin, Task};

const WINDOW_WIDTH: u32 = 960;
const WINDOW_HEIGHT: u32 = 694;

// The head across the top, the pedalboard in the rest of the window below it
const HEAD_HEIGHT: f32 = 354.0;

const STYLE: PedalStyle = PedalStyle {
    name: "Guitar Amp",
    model: "GA-3",
    paint: ui::theme::PAINT_OXBLOOD,
    ornament: Ornament::Bolts,
    jacks: ["Stereo Out", "In"],
    band_y: 236.0,
};

// The preset display, top centre between the jack captions: its caption, and under it the
// tape with a button at each end. Wide enough for the longest name and the edited mark.
// While the tuner is on, its display is there instead, under its own caption
const PRESET_X: f32 = WINDOW_WIDTH as f32 / 2.0;
const PRESET_CAPTION_Y: f32 = 25.0;
const PRESET_Y: f32 = 47.0;
const PRESET_WIDTH: f32 = 156.0;
// Room for the note and seventeen lamps
const TUNER_WIDTH: f32 = 286.0;

// The amp's own six dials in an even row, and the output level set apart from them
const KNOB_Y: f32 = 114.0;
const KNOB_RADIUS: f32 = 30.0;
const DIAL_X: f32 = 150.0;
const DIAL_SPACING: f32 = 100.0;
const OUTPUT_X: f32 = 790.0;

// Small switches below the band. Left of each, its LED above its caption. The amp selector
// sits under the dials, the bypass under the output knob, and the tuner beside the bypass,
// further from the amps than they are from each other: it is not a fourth amp
const SWITCH_Y: f32 = 320.0;
const AMP_SWITCHES: [(Amp, &str, f32); 3] = [(Amp::Klar, "Klar", 255.0), (Amp::Brol, "Brøl", 385.0), (Amp::Torden, "Torden", 515.0)];
const BYPASS_X: f32 = 790.0;
const TUNER_X: f32 = 680.0;
const CAPTION_OFFSET: f32 = -58.0;
const LED_Y: f32 = 309.0;
const CAPTION_Y: f32 = 335.0;
const CAPTION_SIZE: f32 = 17.0;

// The pedalboard: five slots in signal order, left to right
const PEDAL_SLOTS: usize = 5;
const PEDAL_TOP: f32 = BENCH_MARGIN + HEAD_HEIGHT + BENCH_MARGIN;
const PEDAL_WIDTH: f32 = (WINDOW_WIDTH as f32 - BENCH_MARGIN) / PEDAL_SLOTS as f32 - BENCH_MARGIN;
const PEDAL_HEIGHT: f32 = WINDOW_HEIGHT as f32 - BENCH_MARGIN - PEDAL_TOP;

// On every pedal, from its top centre: the LED under the title, the knobs, the switch at the bottom.
// Two knobs sit side by side. Three do not fit in a row, so they are smaller and in a triangle,
// with the LED between the upper two
const PEDAL_LED: Vec2 = vec2(0.0, 71.0);
const PEDAL_SWITCH: Vec2 = vec2(0.0, 273.0);
const PAIR_RADIUS: f32 = 24.0;
const PAIR_KNOBS: [Vec2; 2] = [vec2(-45.0, 144.0), vec2(45.0, 144.0)];
const TRIO_RADIUS: f32 = 17.0;
const TRIO_KNOBS: [Vec2; 3] = [vec2(-45.0, 71.0), vec2(45.0, 71.0), vec2(0.0, 169.0)];

// The Cab pedal has the display for picking the cabinet where a third knob would be: the
// amp's own or one of the player's files. Its two knobs are the upper two of a trio. The
// tape is as wide as fits between the buttons, and holds this many letters
const CAB_SLOT: usize = 2;
const CAB_CAPTION: Vec2 = vec2(0.0, 174.0);
const CAB_STEPPER: Vec2 = vec2(0.0, 198.0);
const CAB_STEPPER_WIDTH: f32 = 100.0;
const CAB_NAME_CHARS: usize = 10;

/// One pedal of the board: its title, its switch and up to three knobs with their captions
type Pedal<'a> = (&'a str, &'a BoolParam, &'a [(&'a FloatParam, &'a str)]);

/// Moves the choice of cabinet one on in the round of the amp's own and the files in the
/// cabinets folder as they are now, and has what it arrives at read away from this thread
/// and the audio. The folder is made here, the first time someone looks for cabinets in
/// it, and not by opening the plugin
fn step_cabinet(
    params: &GuitarAmpParams,
    cab_loader: &CabLoader,
    async_executor: &AsyncExecutor<GuitarAmpPlugin>,
    forward: bool,
) {
    let Some(dir) = user_cab::cabinets_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let mut chosen = params.cabinet.lock().unwrap_or_else(PoisonError::into_inner);
    *chosen = user_cab::neighbour(&user_cab::list(&dir), &chosen, forward);
    drop(chosen);
    cab_loader.chosen();
    async_executor.execute_background(Task::LoadCabinet);
}

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(
    params: Arc<GuitarAmpParams>,
    editor_state: Arc<EguiState>,
    tuner: Arc<TunerReading>,
    cab_loader: Arc<CabLoader>,
    async_executor: AsyncExecutor<GuitarAmpPlugin>,
) -> Option<Box<dyn Editor>> {
    let (opened_loader, opened_executor) = (cab_loader.clone(), async_executor.clone());
    create_egui_editor(
        editor_state,
        (),
        move |egui_ctx, _| {
            ui::install(egui_ctx);
            // A cabinet file that was missing or unusable may have been put right since:
            // look again whenever the window opens
            if opened_loader.status() != CabStatus::Fine {
                opened_executor.execute_background(Task::LoadCabinet);
            }
        },
        move |egui_ctx, setter, _state| {
            ui::rig(egui_ctx, |ui, origin| {
                let head = Rect::from_min_size(
                    origin + vec2(BENCH_MARGIN, BENCH_MARGIN),
                    vec2(WINDOW_WIDTH as f32 - 2.0 * BENCH_MARGIN, HEAD_HEIGHT),
                );
                ui::head(ui, head, &STYLE);

                let tuning = params.tuner_on.value();
                if tuning {
                    // The note the tuner hears and how far off it is, where the preset was
                    let note = tuner.hz().map(Note::nearest);
                    let name = note.map(|note| format!("{}{}", note.name, note.octave));
                    silk_label(ui.painter(), origin + vec2(PRESET_X, PRESET_CAPTION_Y), "Tuner", 13.0);
                    tuner_display(
                        ui.painter(),
                        origin + vec2(PRESET_X, PRESET_Y),
                        TUNER_WIDTH,
                        name.as_deref(),
                        note.map(|note| note.cents),
                    );
                    // What it hears changes without anyone touching the window
                    ui.ctx().request_repaint();
                } else {
                    // The preset loaded last, marked once a knob was moved away from it. The
                    // buttons step through the list and load what they arrive at
                    let loaded = presets::index_or_first(params.preset.load(Ordering::Relaxed));
                    silk_label(ui.painter(), origin + vec2(PRESET_X, PRESET_CAPTION_Y), "Preset", 13.0);
                    let shown = PRESETS[loaded].display_name(&params);
                    if let Some(step) = stepper(ui, origin + vec2(PRESET_X, PRESET_Y), PRESET_WIDTH, &shown, "preset") {
                        let next = presets::neighbour(loaded, step == Step::Next);
                        PRESETS[next].load(&params, setter);
                        params.preset.store(next as u32, Ordering::Relaxed);
                    }
                }

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
                    rig_led(ui.painter(), origin + vec2(x + CAPTION_OFFSET, LED_Y), params.amp.value() == amp);
                    silk_label(ui.painter(), origin + vec2(x + CAPTION_OFFSET, CAPTION_Y), label, CAPTION_SIZE);
                    if small_footswitch(ui, origin + vec2(x, SWITCH_Y), label).clicked() {
                        setter.begin_set_parameter(&params.amp);
                        setter.set_parameter(&params.amp, amp);
                        setter.end_set_parameter(&params.amp);
                    }
                }

                // Tuner switch, lit while tuning: the amp is silent then
                rig_led(ui.painter(), origin + vec2(TUNER_X + CAPTION_OFFSET, LED_Y), tuning);
                silk_label(ui.painter(), origin + vec2(TUNER_X + CAPTION_OFFSET, CAPTION_Y), "Tuner", CAPTION_SIZE);
                if small_footswitch(ui, origin + vec2(TUNER_X, SWITCH_Y), "tuner").clicked() {
                    setter.begin_set_parameter(&params.tuner_on);
                    setter.set_parameter(&params.tuner_on, !tuning);
                    setter.end_set_parameter(&params.tuner_on);
                }

                // Bypass switch, lit while the amp is on
                let bypassed = params.bypass.value();
                rig_led(ui.painter(), origin + vec2(BYPASS_X + CAPTION_OFFSET, LED_Y), !bypassed);
                silk_label(ui.painter(), origin + vec2(BYPASS_X + CAPTION_OFFSET, CAPTION_Y), "On", CAPTION_SIZE);
                if small_footswitch(ui, origin + vec2(BYPASS_X, SWITCH_Y), "bypass").clicked() {
                    setter.begin_set_parameter(&params.bypass);
                    setter.set_parameter(&params.bypass, !bypassed);
                    setter.end_set_parameter(&params.bypass);
                }

                let pedals: [Pedal; PEDAL_SLOTS] = [
                    (
                        "Gate",
                        &params.gate_on,
                        &[(&params.in_gain, "Input"), (&params.gate_thresh, "Thresh"), (&params.gate_release, "Release")],
                    ),
                    (
                        "Drive",
                        &params.drive_on,
                        &[(&params.drive_gain, "Drive"), (&params.drive_tone, "Tone"), (&params.drive_level, "Level")],
                    ),
                    ("Cab", &params.cab_on, &[(&params.cab_mic, "Mic"), (&params.cab_res, "Res")]),
                    (
                        "Delay",
                        &params.delay_on,
                        &[(&params.delay_time, "Time"), (&params.delay_feedback, "Repeat"), (&params.delay_mix, "Mix")],
                    ),
                    ("Reverb", &params.reverb_on, &[(&params.reverb_decay, "Decay"), (&params.reverb_mix, "Mix")]),
                ];
                for (slot, (title, on, knobs)) in pedals.into_iter().enumerate() {
                    let left = BENCH_MARGIN + (PEDAL_WIDTH + BENCH_MARGIN) * slot as f32;
                    let rect = Rect::from_min_size(origin + vec2(left, PEDAL_TOP), vec2(PEDAL_WIDTH, PEDAL_HEIGHT));
                    let top = ui::mini_pedal(ui, rect, STYLE.paint, title).center_top();

                    let (radius, places) = if knobs.len() > PAIR_KNOBS.len() || slot == CAB_SLOT {
                        (TRIO_RADIUS, &TRIO_KNOBS[..])
                    } else {
                        (PAIR_RADIUS, &PAIR_KNOBS[..])
                    };
                    for (&(param, caption), &offset) in knobs.iter().zip(places) {
                        param_knob(ui, setter, param, top + offset, radius, caption, None);
                    }
                    param_switch(ui, setter, on, top + PEDAL_SWITCH, top + PEDAL_LED);

                    if slot == CAB_SLOT {
                        // The cabinet that plays: the amp's own, or a file from the cabinets
                        // folder. The buttons step through them
                        let chosen = params.cabinet.lock().unwrap_or_else(PoisonError::into_inner).clone();
                        let shown = cab_loader.display_name(&chosen, CAB_NAME_CHARS);
                        silk_label(ui.painter(), top + CAB_CAPTION, "Speaker", 13.0);
                        if let Some(step) = stepper(ui, top + CAB_STEPPER, CAB_STEPPER_WIDTH, &shown, "cabinet") {
                            step_cabinet(&params, &cab_loader, &async_executor, step == Step::Next);
                        }
                    }
                }
            });
        },
    )
}
