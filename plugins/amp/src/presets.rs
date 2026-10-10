//! The built-in presets: plain data, loaded from the editor through the host like any other
//! knob movement. The audio thread knows nothing about them.
//!
//! A preset holds every parameter except four: Bypass, the Tuner switch, and Input and
//! Output, which are the player's gain staging for their guitar and their mix. They stay
//! where they are when a preset is loaded.
//!
//! Values are written as the knobs show them: dials 0.0 to 10.0, dB, ms, s and percent.
//! `dial` and `percent` turn them into what the parameters store.

use nih_plug::prelude::*;

use crate::dial_memory;
use crate::{Amp, GuitarAmpParams};

/// Shown behind the name once a knob no longer stands where the preset put it
pub const EDITED_MARK: char = '*';

// A knob counts as moved when it is this far from the preset's value, as a share of its
// travel. Well under the smallest step a knob shows, and under one pixel of a fine drag
const MATCH_TOLERANCE: f32 = 0.0004;

const ON: bool = true;
const OFF: bool = false;

pub struct Preset {
    pub name: &'static str,
    pub amp: Amp,
    /// Gain, Bass, Mid, Treble, Presence, Master: 0.0 to 10.0
    pub dials: [f32; 6],
    /// Switch, threshold in dB, release in ms
    pub gate: (bool, f32, f32),
    /// Switch, then Drive, Tone, Level: 0.0 to 10.0
    pub drive: (bool, [f32; 3]),
    /// Switch, then Mic, Resonance: 0.0 to 10.0
    pub cab: (bool, [f32; 2]),
    /// Switch, time in ms, feedback in percent, mix in percent
    pub delay: (bool, f32, f32, f32),
    /// Switch, decay in s, mix in percent
    pub reverb: (bool, f32, f32),
}

// Chosen from the amp models' constants and measured with `preset_report` below: nobody
// has listened to them yet. Master is where their levels were matched: the report's chords
// come out at -18 to -14 dBFS RMS (cleans lowest, leads highest) with the peaks under the
// safety clip's knee. The first one is every parameter at its default
pub static PRESETS: [Preset; 12] = [
    Preset {
        name: "Init",
        amp: Amp::Brol,
        dials: [5.0, 5.0, 5.0, 5.0, 5.0, 5.0],
        gate: (ON, -60.0, 100.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [5.0, 5.0]),
        delay: (OFF, 350.0, 35.0, 25.0),
        reverb: (OFF, 1.5, 20.0),
    },
    // Klar. Glassy clean: below the point where the stages bend, where the bright lift is
    // strongest, the top end up and a little off the amp's full low end. A short room
    Preset {
        name: "Glas",
        amp: Amp::Klar,
        dials: [3.5, 4.5, 4.5, 6.0, 6.5, 5.0],
        gate: (ON, -66.0, 250.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [6.0, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 1.2, 12.0),
    },
    // Klar. Warm clean for the neck pickup: mids filled in against the amp's scoop, treble
    // and presence down, a darker microphone
    Preset {
        name: "Varm",
        amp: Amp::Klar,
        dials: [4.0, 5.0, 6.5, 3.5, 3.0, 4.8],
        gate: (ON, -66.0, 250.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [3.5, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 1.4, 14.0),
    },
    // Klar. Ambient: clean, long repeats into a long tail. The gate stays on so the effects
    // can fall idle, but low and slow so it does not cut a note that fades
    Preset {
        name: "Tåge",
        amp: Amp::Klar,
        dials: [3.0, 4.5, 5.0, 5.5, 5.5, 4.9],
        gate: (ON, -72.0, 500.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [4.5, 5.0]),
        delay: (ON, 480.0, 45.0, 35.0),
        reverb: (ON, 4.0, 40.0),
    },
    // Klar. Edge of breakup: the amp where its stages start to bend, and the drive pedal as
    // the overdrive in front, so picking harder is what distorts
    Preset {
        name: "Glød",
        amp: Amp::Klar,
        dials: [6.5, 4.5, 6.0, 5.5, 5.0, 4.0],
        gate: (ON, -62.0, 150.0),
        drive: (ON, [5.5, 5.0, 5.5]),
        cab: (ON, [5.0, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 1.5, 15.0),
    },
    // Brøl. Cleaned up: the bottom of the Gain dial, where every stage is turned down, with
    // Master at the centre so pick attacks do not push the power stage
    Preset {
        name: "Kobber",
        amp: Amp::Brol,
        dials: [1.5, 5.5, 5.5, 6.0, 6.0, 4.6],
        gate: (ON, -64.0, 200.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [5.5, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 1.4, 15.0),
    },
    // Brøl. Rhythm crunch: a little past the centre of the Gain dial, mids forward, dry
    Preset {
        name: "Grus",
        amp: Amp::Brol,
        dials: [6.0, 5.0, 6.5, 5.5, 5.5, 5.0],
        gate: (ON, -58.0, 120.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [5.0, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 15.0),
    },
    // Brøl. Lead: high on the Gain dial with the pedal pushing it, less bass into it, mids
    // up to carry single notes, a darker microphone and one delay behind
    Preset {
        name: "Flamme",
        amp: Amp::Brol,
        dials: [8.0, 4.5, 7.5, 5.5, 5.0, 4.7],
        gate: (ON, -56.0, 200.0),
        drive: (ON, [2.0, 5.5, 7.0]),
        cab: (ON, [4.5, 5.0]),
        delay: (ON, 380.0, 30.0, 22.0),
        reverb: (OFF, 1.8, 12.0),
    },
    // Torden. Tight modern rhythm: the pedal as a boost (Drive low, Level high), which cuts
    // the lows going into the amp. Gain no higher than it needs: the amp is fully squeezed
    // from the centre of the dial. A fast gate
    Preset {
        name: "Stram",
        amp: Amp::Torden,
        dials: [5.5, 4.5, 5.0, 6.0, 6.0, 5.0],
        gate: (ON, -50.0, 60.0),
        drive: (ON, [1.0, 6.0, 8.0]),
        cab: (ON, [5.5, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 10.0),
    },
    // Torden. Scooped and heavier: mids down, bass and treble up (which deepens the scoop),
    // more gain, more thump from the cabinet
    Preset {
        name: "Granit",
        amp: Amp::Torden,
        dials: [7.5, 6.5, 3.0, 6.5, 6.5, 5.5],
        gate: (ON, -48.0, 60.0),
        drive: (ON, [1.5, 5.5, 7.0]),
        cab: (ON, [5.0, 6.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 10.0),
    },
    // Torden. Palm mutes in a drop tuning: the hardest boost and the brightest pedal, bass
    // down, the cabinet's resonance well down so the low roots do not smear, the fastest gate
    Preset {
        name: "Dyb",
        amp: Amp::Torden,
        dials: [6.0, 4.0, 5.5, 6.0, 6.5, 5.0],
        gate: (ON, -46.0, 40.0),
        drive: (ON, [0.5, 6.5, 8.5]),
        cab: (ON, [5.5, 2.5]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 10.0),
    },
    // Torden. Lead: mids back up, a darker microphone, a slower gate that lets notes ring,
    // a delay and a little reverb
    Preset {
        name: "Lyn",
        amp: Amp::Torden,
        dials: [7.0, 5.0, 6.5, 5.5, 5.5, 5.0],
        gate: (ON, -52.0, 180.0),
        drive: (ON, [2.5, 5.0, 7.0]),
        cab: (ON, [4.5, 5.0]),
        delay: (ON, 420.0, 30.0, 20.0),
        reverb: (ON, 2.0, 10.0),
    },
];

/// A dial as printed, 0.0 to 10.0, as its parameter stores it
fn dial(shown: f32) -> f32 {
    shown / 10.0
}

/// A percentage as its parameter stores it
fn percent(shown: f32) -> f32 {
    shown / 100.0
}

/// The preset the stored index stands for. An index from a build with more presets than
/// this one falls back to the first
pub fn index_or_first(stored: u32) -> usize {
    let index = stored as usize;
    if index < PRESETS.len() {
        index
    } else {
        0
    }
}

/// The preset before or after `index`, going round at both ends
pub fn neighbour(index: usize, forward: bool) -> usize {
    let count = PRESETS.len();
    (index + if forward { 1 } else { count - 1 }) % count
}

impl Preset {
    /// Every continuous parameter the preset holds, with the value it gets
    fn knobs<'a>(&self, params: &'a GuitarAmpParams) -> [(&'a FloatParam, f32); 18] {
        let [gain, bass, mid, treble, presence, master] = self.dials;
        let (_, gate_thresh_db, gate_release_ms) = self.gate;
        let (_, [drive_gain, drive_tone, drive_level]) = self.drive;
        let (_, [cab_mic, cab_res]) = self.cab;
        let (_, delay_time_ms, delay_feedback, delay_mix) = self.delay;
        let (_, reverb_decay_s, reverb_mix) = self.reverb;
        [
            (&params.gate_thresh, gate_thresh_db),
            (&params.gate_release, gate_release_ms),
            (&params.drive_gain, dial(drive_gain)),
            (&params.drive_tone, dial(drive_tone)),
            (&params.drive_level, dial(drive_level)),
            (&params.gain, dial(gain)),
            (&params.bass, dial(bass)),
            (&params.mid, dial(mid)),
            (&params.treble, dial(treble)),
            (&params.presence, dial(presence)),
            (&params.master, dial(master)),
            (&params.cab_mic, dial(cab_mic)),
            (&params.cab_res, dial(cab_res)),
            (&params.delay_time, delay_time_ms),
            (&params.delay_feedback, percent(delay_feedback)),
            (&params.delay_mix, percent(delay_mix)),
            (&params.reverb_decay, reverb_decay_s),
            (&params.reverb_mix, percent(reverb_mix)),
        ]
    }

    /// Every switch the preset holds, with the position it gets
    fn switches<'a>(&self, params: &'a GuitarAmpParams) -> [(&'a BoolParam, bool); 5] {
        [
            (&params.gate_on, self.gate.0),
            (&params.drive_on, self.drive.0),
            (&params.cab_on, self.cab.0),
            (&params.delay_on, self.delay.0),
            (&params.reverb_on, self.reverb.0),
        ]
    }

    /// Set every parameter the preset holds, as the host would see the knobs being turned:
    /// it records the changes and can undo them.
    ///
    /// All of them are set, also those that seem to stand right already: a host may apply
    /// a change some time after it was asked for, so what a parameter reads here can be
    /// older than the last preset loaded
    ///
    /// The amp that is left for the preset's remembers its dials, and the preset's amp
    /// remembers the preset's: see `dial_memory`
    pub fn load(&self, params: &GuitarAmpParams, setter: &ParamSetter) {
        dial_memory::preset_loaded(params, self.amp, self.dials.map(dial));
        set(setter, &params.amp, self.amp);
        for (param, on) in self.switches(params) {
            set(setter, param, on);
        }
        for (param, value) in self.knobs(params) {
            set(setter, param, value);
        }
    }

    /// Whether every parameter the preset holds still stands where the preset puts it
    pub fn matches(&self, params: &GuitarAmpParams) -> bool {
        params.amp.value() == self.amp
            && self.switches(params).into_iter().all(|(param, on)| param.value() == on)
            && self.knobs(params).into_iter().all(|(param, value)| {
                (param.unmodulated_normalized_value() - param.preview_normalized(value)).abs() <= MATCH_TOLERANCE
            })
    }

    /// What the head's display shows: the name, marked once the sound was edited
    pub fn display_name(&self, params: &GuitarAmpParams) -> String {
        if self.matches(params) {
            self.name.to_owned()
        } else {
            format!("{}{EDITED_MARK}", self.name)
        }
    }
}

fn set<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{AmpChain, AmpSettings};
    use crate::delay::DelaySettings;
    use crate::dsp::shaper::OUTPUT_CLIP_KNEE;
    use crate::reverb::ReverbSettings;
    use crate::test_util::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    // Longest preset name: what fits on the head's display with the edited mark behind it
    const NAME_MAX_CHARS: usize = 14;

    impl Preset {
        /// What the chain is given once the preset is loaded, with Input and Output untouched
        fn settings(&self) -> AmpSettings {
            let [gain, bass, mid, treble, presence, master] = self.dials.map(dial);
            let [drive_gain, drive_tone, drive_level] = self.drive.1.map(dial);
            let [cab_mic, cab_res] = self.cab.1.map(dial);
            AmpSettings {
                gate_on: self.gate.0,
                gate_thresh_db: self.gate.1,
                gate_release_ms: self.gate.2,
                drive_on: self.drive.0,
                drive_gain,
                drive_tone,
                drive_level,
                amp: self.amp,
                gain,
                bass,
                mid,
                treble,
                presence,
                master,
                cab_on: self.cab.0,
                cab_mic,
                cab_res,
                delay: DelaySettings {
                    on: self.delay.0,
                    time_ms: self.delay.1,
                    feedback: percent(self.delay.2),
                    mix: percent(self.delay.3),
                },
                reverb: ReverbSettings {
                    on: self.reverb.0,
                    decay_s: self.reverb.1,
                    mix: percent(self.reverb.2),
                },
                ..AmpSettings::default()
            }
        }
    }

    fn run(settings: &AmpSettings, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut chain = AmpChain::new();
        chain.set_sample_rate(SAMPLE_RATE);
        let mut left = input.to_vec();
        let mut right = input.to_vec();
        for (left, right) in left.chunks_mut(64).zip(right.chunks_mut(64)) {
            chain.process(settings, left, Some(right));
        }
        (left, right)
    }

    #[test]
    fn test_names_are_unique_and_fit_the_display() {
        for (index, preset) in PRESETS.iter().enumerate() {
            let length = preset.name.chars().count();
            assert!((1..=NAME_MAX_CHARS).contains(&length), "{} has {length} characters", preset.name);
            assert_eq!(preset.name, preset.name.trim());
            assert!(!preset.name.contains(EDITED_MARK), "{} looks edited", preset.name);
            for other in &PRESETS[..index] {
                assert_ne!(preset.name.to_lowercase(), other.name.to_lowercase());
            }
        }
    }

    #[test]
    fn test_every_value_is_inside_its_parameters_range() {
        let params = GuitarAmpParams::default();
        for preset in &PRESETS {
            for (param, value) in preset.knobs(&params) {
                // Outside the range the parameter would clamp, and not come back to the value
                let stored = param.preview_plain(param.preview_normalized(value));
                assert!(
                    (stored - value).abs() <= 1e-4 * value.abs().max(1.0),
                    "{}: {} = {value} is outside its range",
                    preset.name,
                    param.name()
                );
            }
        }
    }

    #[test]
    fn test_printed_values_are_on_their_scales() {
        // Dials and percentages are written as the knobs show them, not as they are stored
        for preset in &PRESETS {
            let dials = preset.dials.iter().chain(&preset.drive.1).chain(&preset.cab.1);
            for &shown in dials {
                assert!((0.0..=10.0).contains(&shown), "{}: dial at {shown}", preset.name);
            }
            assert!((0.0..=90.0).contains(&preset.delay.2), "{}: feedback", preset.name);
            for shown in [preset.delay.3, preset.reverb.2] {
                assert!((0.0..=100.0).contains(&shown), "{}: mix at {shown}", preset.name);
            }
        }
    }

    #[test]
    fn test_first_preset_is_every_default() {
        let params = GuitarAmpParams::default();
        assert_eq!(PRESETS[0].name, "Init");
        assert!(PRESETS[0].matches(&params));
        assert_eq!(PRESETS[0].display_name(&params), "Init");
        assert_eq!(PRESETS[0].settings(), AmpSettings::default());
    }

    #[test]
    fn test_a_changed_parameter_marks_the_preset_as_edited() {
        // The defaults stand where Init puts them, and nowhere else
        let params = GuitarAmpParams::default();
        for preset in &PRESETS[1..] {
            assert!(!preset.matches(&params), "{} equals the defaults", preset.name);
            assert_eq!(preset.display_name(&params), format!("{}*", preset.name));
        }
    }

    #[test]
    fn test_presets_leave_the_tuner_alone() {
        // Like Bypass, Input and Output, the tuner switch is not part of a sound
        let params = GuitarAmpParams::default();
        for preset in &PRESETS {
            let switches = preset.switches(&params);
            assert!(switches.iter().all(|(param, _)| !std::ptr::eq(*param, &params.tuner_on)), "{}", preset.name);
            assert!(!preset.settings().tuner_on);
        }
    }

    #[test]
    fn test_presets_differ_from_each_other() {
        for (index, preset) in PRESETS.iter().enumerate() {
            for other in &PRESETS[..index] {
                assert_ne!(preset.settings(), other.settings(), "{} and {}", preset.name, other.name);
            }
        }
    }

    #[test]
    fn test_every_amp_has_at_least_three_presets() {
        for amp in Amp::ALL {
            let count = PRESETS[1..].iter().filter(|preset| preset.amp == amp).count();
            assert!(count >= 3, "{amp:?} has {count}");
        }
    }

    #[test]
    fn test_stepping_goes_round_and_survives_a_stale_index() {
        let last = PRESETS.len() - 1;
        assert_eq!(neighbour(0, true), 1);
        assert_eq!(neighbour(last, true), 0);
        assert_eq!(neighbour(0, false), last);
        assert_eq!(neighbour(1, false), 0);
        assert_eq!(index_or_first(last as u32), last);
        assert_eq!(index_or_first(PRESETS.len() as u32), 0);
        assert_eq!(index_or_first(u32::MAX), 0);
    }

    #[test]
    fn test_presets_leave_the_safety_clip_alone() {
        // Input and Output are not part of a preset, so a preset has to fit at 0 dB
        let chords = power_chords(SAMPLE_RATE, 2.0);
        for preset in &PRESETS {
            let (left, right) = run(&preset.settings(), &chords);
            let level = to_db(peak(&left).max(peak(&right)));
            assert!(level < to_db(OUTPUT_CLIP_KNEE), "{} peaks at {level:.1} dBFS", preset.name);
            let loudness = to_db(rms(&left));
            assert!((-20.0..-12.0).contains(&loudness), "{} is at {loudness:.1} dBFS RMS", preset.name);
        }
    }

    /// Level and balance of every preset. Run after changing a preset or the amps:
    /// cargo test -p amp --release preset_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn preset_report() {
        let chords = power_chords(SAMPLE_RATE, 4.0);
        let mutes = palm_mutes(SAMPLE_RATE, 2.4);
        println!("\nChords: level in dBFS. Palm mutes: dB relative to the whole signal");
        println!(
            "{:<8} {:<8} {:>7} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "preset", "amp", "RMS", "peak", "to 100", "100-400", "400-1k6", "1k6-6k4", "6k4 up"
        );
        for preset in &PRESETS {
            let settings = preset.settings();
            let (left, right) = run(&settings, &chords);
            let (mutes_left, _) = run(&settings, &mutes);
            let bands = tight_levels_db(&mutes_left, SAMPLE_RATE);
            print!(
                "{:<8} {:<8} {:>7.1} {:>7.1}",
                preset.name,
                format!("{:?}", preset.amp),
                to_db(rms(&left)),
                to_db(peak(&left).max(peak(&right)))
            );
            for band in bands {
                print!(" {band:>8.1}");
            }
            println!();
        }
    }
}
