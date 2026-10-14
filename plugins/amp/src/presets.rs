//! The built-in presets: plain data, loaded from the editor through the host like any other
//! knob movement. The audio thread knows nothing about them.
//!
//! A preset holds every parameter except four: Bypass, the Tuner switch, and Input and
//! Output, which are the player's gain staging for their guitar and their mix. They stay
//! where they are when a preset is loaded. It also holds the order the pedals stand in,
//! which is not a parameter.
//!
//! Values are written as the knobs show them: dials 0.0 to 10.0, dB, ms, s and percent.
//! `dial` and `percent` turn them into what the parameters store.

use nih_plug::prelude::*;

use crate::dial_memory;
use crate::order::ChainOrder;
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
    /// Switch, then Sustain, Level: 0.0 to 10.0
    pub comp: (bool, [f32; 2]),
    /// Chorus, phaser, flanger, tremolo: switch, rate in Hz, depth in percent
    pub mods: [(bool, f32, f32); 4],
    /// The pedals in the order the signal goes through them, with `amp` where the amp and
    /// its cabinet are: what `ChainOrder::from_ids` reads
    pub order: &'static [&'static str],
}

// The compressor and the modulation pedals switched off, with their dials where the
// parameters start
const COMP_OFF: (bool, [f32; 2]) = (OFF, [5.0, 5.0]);
const MODS_OFF: [(bool, f32, f32); 4] =
    [(OFF, 0.8, 50.0), (OFF, 0.5, 50.0), (OFF, 0.3, 50.0), (OFF, 5.0, 50.0)];
// The order the chain starts in: `ChainOrder::default`
const USUAL_ORDER: &[&str] =
    &["comp", "drive", "phaser", "amp", "chorus", "flanger", "tremolo", "delay", "reverb"];

// Chosen from the amp models' constants and measured with `preset_report` below: nobody
// has listened to them yet. Master is where their levels were matched, by how loud they
// are heard (`loudness_lufs`) with a guitar recorded at `REFERENCE_DI_DB`: within 2 LU of
// each other, leads highest, with the peaks of a guitar recorded 6 dB hotter under the
// safety clip's knee. A clean preset follows the level of the guitar and one that clips
// does not, so with a quieter guitar the clean ones come out quieter than the rest, and
// with a hotter one louder. The first one is every parameter at its default.
//
// The gates are set by `preset_gate_report`. A threshold of -54 dB is the highest that
// leaves palm mutes alone when they are played quietly (peaks at -30 dBFS): from -52 dB up
// the gate starts to close on the tail of every mute. How fast a gate shuts in a rest is
// its release, not its threshold: 8 dB of threshold are 9 ms
pub static PRESETS: [Preset; 17] = [
    Preset {
        name: "Init",
        amp: Amp::Brol,
        dials: [5.0, 5.0, 5.0, 5.0, 5.0, 5.0],
        gate: (ON, -60.0, 100.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [5.0, 5.0]),
        delay: (OFF, 350.0, 35.0, 25.0),
        reverb: (OFF, 1.5, 20.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // Klar. Glassy clean: below the point where the stages bend, where the bright lift is
    // strongest, the top end up and a little off the amp's full low end. A short room
    Preset {
        name: "Glas",
        amp: Amp::Klar,
        dials: [3.5, 4.5, 4.5, 6.0, 6.5, 4.9],
        gate: (ON, -66.0, 250.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [6.0, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 1.2, 12.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
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
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // Klar. Ambient: clean, long repeats into a long tail. The gate stays on so the effects
    // can fall idle, but low and slow so it does not cut a note that fades
    Preset {
        name: "Tåge",
        amp: Amp::Klar,
        dials: [3.0, 4.5, 5.0, 5.5, 5.5, 4.6],
        gate: (ON, -72.0, 500.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [4.5, 5.0]),
        delay: (ON, 480.0, 45.0, 35.0),
        reverb: (ON, 4.0, 40.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
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
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // Brøl. Cleaned up: the bottom of the Gain dial, where every stage is turned down, with
    // Master at the centre so pick attacks do not push the power stage
    Preset {
        name: "Kobber",
        amp: Amp::Brol,
        dials: [1.5, 5.5, 5.5, 6.0, 6.0, 5.3],
        gate: (ON, -64.0, 200.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [5.5, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 1.4, 15.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
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
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
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
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // Torden. Tight modern rhythm: the pedal as a boost (Drive low, Level high), which cuts
    // the lows going into the amp. Gain no higher than it needs: the amp is fully squeezed
    // from the centre of the dial. A fast gate
    Preset {
        name: "Stram",
        amp: Amp::Torden,
        dials: [5.5, 4.5, 5.0, 6.0, 6.0, 5.0],
        gate: (ON, -56.0, 50.0),
        drive: (ON, [1.0, 6.0, 8.0]),
        cab: (ON, [5.5, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 10.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // Torden. Scooped and heavier: mids down, bass and treble up (which deepens the scoop),
    // more gain, more thump from the cabinet
    Preset {
        name: "Granit",
        amp: Amp::Torden,
        dials: [7.5, 6.5, 3.0, 6.5, 6.5, 5.5],
        gate: (ON, -54.0, 60.0),
        drive: (ON, [1.5, 5.5, 7.0]),
        cab: (ON, [5.0, 6.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 10.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // Torden. Palm mutes in a drop tuning: the hardest boost and the brightest pedal, bass
    // down, the cabinet's resonance well down so the low roots do not smear, the fastest gate
    Preset {
        name: "Dyb",
        amp: Amp::Torden,
        dials: [6.0, 4.0, 5.5, 6.0, 6.5, 5.0],
        gate: (ON, -54.0, 40.0),
        drive: (ON, [0.5, 6.5, 8.5]),
        cab: (ON, [5.5, 2.5]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 10.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // Torden. Lead: mids back up, a darker microphone, a slower gate that lets notes ring,
    // a delay and a little reverb
    Preset {
        name: "Lyn",
        amp: Amp::Torden,
        dials: [7.0, 5.0, 6.5, 5.5, 5.5, 5.0],
        gate: (ON, -54.0, 180.0),
        drive: (ON, [2.5, 5.0, 7.0]),
        cab: (ON, [4.5, 5.0]),
        delay: (ON, 420.0, 30.0, 20.0),
        reverb: (ON, 2.0, 10.0),
        comp: COMP_OFF,
        mods: MODS_OFF,
        order: USUAL_ORDER,
    },
    // The presets from here on use the pedals that came with the board that can be
    // rearranged. New ones go at the end: a project remembers its preset by its number.
    //
    // Klar. Clean with the compressor in front and a slow chorus behind the cabinet, where
    // it is wide
    Preset {
        name: "Rav",
        amp: Amp::Klar,
        dials: [3.5, 4.5, 5.0, 6.0, 6.0, 4.5],
        gate: (ON, -66.0, 250.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [5.5, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 1.4, 14.0),
        comp: (ON, [5.0, 5.0]),
        mods: [(ON, 0.8, 55.0), (OFF, 0.5, 50.0), (OFF, 0.3, 50.0), (OFF, 5.0, 50.0)],
        order: USUAL_ORDER,
    },
    // Klar. Warm clean with a tremolo and a longer room behind it
    Preset {
        name: "Puls",
        amp: Amp::Klar,
        dials: [4.5, 5.0, 5.5, 5.0, 4.5, 4.9],
        gate: (ON, -66.0, 250.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [4.5, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (ON, 2.0, 18.0),
        comp: COMP_OFF,
        mods: [(OFF, 0.8, 50.0), (OFF, 0.5, 50.0), (OFF, 0.3, 50.0), (ON, 5.5, 60.0)],
        order: USUAL_ORDER,
    },
    // Brøl. Crunch with the phaser in front of the amp, where its sweep is clipped with
    // the guitar and moves the grit
    Preset {
        name: "Hvirvel",
        amp: Amp::Brol,
        dials: [5.5, 5.0, 6.0, 5.5, 5.5, 5.0],
        gate: (ON, -58.0, 150.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [5.0, 5.0]),
        delay: (OFF, 350.0, 30.0, 20.0),
        reverb: (OFF, 1.5, 15.0),
        comp: COMP_OFF,
        mods: [(OFF, 0.8, 50.0), (ON, 0.5, 60.0), (OFF, 0.3, 50.0), (OFF, 5.0, 50.0)],
        order: USUAL_ORDER,
    },
    // Torden. Lead with a slow flanger behind the cabinet and one delay
    Preset {
        name: "Sus",
        amp: Amp::Torden,
        dials: [6.5, 5.0, 5.5, 5.5, 5.5, 5.0],
        gate: (ON, -54.0, 120.0),
        drive: (ON, [1.5, 5.5, 7.0]),
        cab: (ON, [5.0, 5.0]),
        delay: (ON, 380.0, 25.0, 18.0),
        reverb: (OFF, 1.5, 10.0),
        comp: COMP_OFF,
        mods: [(OFF, 0.8, 50.0), (OFF, 0.5, 50.0), (ON, 0.25, 50.0), (OFF, 5.0, 50.0)],
        order: USUAL_ORDER,
    },
    // Klar. Haze: the chorus in front of the amp, and the reverb in front of the delay, so
    // every repeat is the whole room again
    Preset {
        name: "Dis",
        amp: Amp::Klar,
        dials: [3.0, 4.5, 5.0, 5.5, 5.5, 4.4],
        gate: (ON, -72.0, 500.0),
        drive: (OFF, [3.0, 5.0, 5.0]),
        cab: (ON, [4.5, 5.0]),
        delay: (ON, 450.0, 40.0, 30.0),
        reverb: (ON, 3.5, 35.0),
        comp: (ON, [4.0, 5.0]),
        mods: [(ON, 0.6, 50.0), (OFF, 0.5, 50.0), (OFF, 0.3, 50.0), (OFF, 5.0, 50.0)],
        order: &[
            "comp", "chorus", "drive", "phaser", "amp", "flanger", "tremolo", "reverb", "delay",
        ],
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
    fn knobs<'a>(&self, params: &'a GuitarAmpParams) -> [(&'a FloatParam, f32); 28] {
        let [gain, bass, mid, treble, presence, master] = self.dials;
        let (_, gate_thresh_db, gate_release_ms) = self.gate;
        let (_, [drive_gain, drive_tone, drive_level]) = self.drive;
        let (_, [cab_mic, cab_res]) = self.cab;
        let (_, delay_time_ms, delay_feedback, delay_mix) = self.delay;
        let (_, reverb_decay_s, reverb_mix) = self.reverb;
        let (_, [comp_sustain, comp_level]) = self.comp;
        let [chorus, phaser, flanger, tremolo] = self.mods;
        [
            (&params.comp_sustain, dial(comp_sustain)),
            (&params.comp_level, dial(comp_level)),
            (&params.chorus_rate, chorus.1),
            (&params.chorus_depth, percent(chorus.2)),
            (&params.phaser_rate, phaser.1),
            (&params.phaser_depth, percent(phaser.2)),
            (&params.flanger_rate, flanger.1),
            (&params.flanger_depth, percent(flanger.2)),
            (&params.trem_rate, tremolo.1),
            (&params.trem_depth, percent(tremolo.2)),
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
    fn switches<'a>(&self, params: &'a GuitarAmpParams) -> [(&'a BoolParam, bool); 10] {
        [
            (&params.gate_on, self.gate.0),
            (&params.comp_on, self.comp.0),
            (&params.chorus_on, self.mods[0].0),
            (&params.phaser_on, self.mods[1].0),
            (&params.flanger_on, self.mods[2].0),
            (&params.trem_on, self.mods[3].0),
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
    /// remembers the preset's: see `dial_memory`. The pedals go into the preset's order,
    /// which the host is not told: it is no parameter
    pub fn load(&self, params: &GuitarAmpParams, setter: &ParamSetter) {
        dial_memory::preset_loaded(params, self.amp, self.dials.map(dial));
        params.order.put(self.chain_order());
        set(setter, &params.amp, self.amp);
        for (param, on) in self.switches(params) {
            set(setter, param, on);
        }
        for (param, value) in self.knobs(params) {
            set(setter, param, value);
        }
    }

    /// The order the preset puts the pedals in
    pub fn chain_order(&self) -> ChainOrder {
        ChainOrder::from_ids(self.order.iter().copied())
    }

    /// Whether every parameter the preset holds still stands where the preset puts it, and
    /// the pedals in its order
    pub fn matches(&self, params: &GuitarAmpParams) -> bool {
        params.amp.value() == self.amp
            && params.order.get() == self.chain_order()
            && self.switches(params).into_iter().all(|(param, on)| param.value() == on)
            && self.knobs(params).into_iter().all(|(param, value)| {
                (param.unmodulated_normalized_value() - param.preview_normalized(value)).abs()
                    <= MATCH_TOLERANCE
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
    use crate::comp::CompSettings;
    use crate::delay::DelaySettings;
    use crate::dsp::shaper::OUTPUT_CLIP_KNEE;
    use crate::gate::tests::{decaying_note, gain_trace, hiss, transitions as gate_changes};
    use crate::gate::Gate;
    use crate::mods::chorus::ChorusSettings;
    use crate::mods::flanger::FlangerSettings;
    use crate::mods::phaser::PhaserSettings;
    use crate::mods::tremolo::TremoloSettings;
    use crate::order::Pedal;
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
            let [comp_sustain, comp_level] = self.comp.1.map(dial);
            let [chorus, phaser, flanger, tremolo] = self.mods;
            AmpSettings {
                comp: CompSettings { on: self.comp.0, sustain: comp_sustain, level: comp_level },
                chorus: ChorusSettings {
                    on: chorus.0,
                    rate_hz: chorus.1,
                    depth: percent(chorus.2),
                },
                phaser: PhaserSettings {
                    on: phaser.0,
                    rate_hz: phaser.1,
                    depth: percent(phaser.2),
                },
                flanger: FlangerSettings {
                    on: flanger.0,
                    rate_hz: flanger.1,
                    depth: percent(flanger.2),
                },
                tremolo: TremoloSettings {
                    on: tremolo.0,
                    rate_hz: tremolo.1,
                    depth: percent(tremolo.2),
                },
                order: self.chain_order(),
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

    // The levels of a direct recording the presets are measured at: a guitar recorded
    // quietly, one in the middle, and a hot one
    const REPORT_DI_LEVELS_DB: [f32; 3] = [-30.0, -24.0, -18.0];

    /// How loud a preset is heard with `recording` played into it at `level_db` dBFS RMS,
    /// in LUFS of the left channel
    fn loudness_at(preset: &Preset, recording: &[f32], level_db: f32) -> f32 {
        let (left, _) = run(&preset.settings(), &at_rms_db(recording, level_db));
        loudness_lufs(&[&left], SAMPLE_RATE)
    }

    #[test]
    fn test_names_are_unique_and_fit_the_display() {
        for (index, preset) in PRESETS.iter().enumerate() {
            let length = preset.name.chars().count();
            assert!(
                (1..=NAME_MAX_CHARS).contains(&length),
                "{} has {length} characters",
                preset.name
            );
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
            let dials = preset
                .dials
                .iter()
                .chain(&preset.drive.1)
                .chain(&preset.cab.1)
                .chain(&preset.comp.1);
            for &shown in dials {
                assert!((0.0..=10.0).contains(&shown), "{}: dial at {shown}", preset.name);
            }
            assert!((0.0..=90.0).contains(&preset.delay.2), "{}: feedback", preset.name);
            for shown in [preset.delay.3, preset.reverb.2] {
                assert!((0.0..=100.0).contains(&shown), "{}: mix at {shown}", preset.name);
            }
            for (_, _, depth) in preset.mods {
                assert!((0.0..=100.0).contains(&depth), "{}: depth at {depth}", preset.name);
            }
        }
    }

    #[test]
    fn test_orders_are_written_out_in_full_and_legal() {
        assert_eq!(USUAL_ORDER, ChainOrder::default().ids());
        for preset in &PRESETS {
            // Every pedal named once, on a side it can stand on: read back, the list is
            // what was written, with nothing put right
            assert_eq!(preset.chain_order().ids(), preset.order, "{}", preset.name);
            assert_eq!(preset.order.len(), Pedal::ALL.len() + 1, "{}", preset.name);
        }
        // The presets from before the pedals could be moved keep the order of then
        for preset in &PRESETS[..12] {
            assert_eq!(preset.chain_order(), ChainOrder::default(), "{}", preset.name);
            assert_eq!((preset.comp, preset.mods), (COMP_OFF, MODS_OFF), "{}", preset.name);
        }
    }

    #[test]
    fn test_a_moved_pedal_marks_the_preset_as_edited() {
        let params = GuitarAmpParams::default();
        assert!(PRESETS[0].matches(&params));
        params.order.put(ChainOrder::default().moved(Pedal::Reverb, crate::order::Zone::Back, 0));
        assert_eq!(PRESETS[0].display_name(&params), "Init*");
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
            assert!(
                switches.iter().all(|(param, _)| !std::ptr::eq(*param, &params.tuner_on)),
                "{}",
                preset.name
            );
            assert!(!preset.settings().tuner_on);
        }
    }

    #[test]
    fn test_presets_differ_from_each_other() {
        for (index, preset) in PRESETS.iter().enumerate() {
            for other in &PRESETS[..index] {
                assert_ne!(
                    preset.settings(),
                    other.settings(),
                    "{} and {}",
                    preset.name,
                    other.name
                );
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
            assert!(
                (-20.0..-12.0).contains(&loudness),
                "{} is at {loudness:.1} dBFS RMS",
                preset.name
            );
        }
    }

    #[test]
    fn test_presets_are_about_as_loud_as_each_other() {
        // Stepping from a distorted preset to a clean one keeps the volume: for a guitar
        // recorded at the reference level every preset is heard within 2 LU of the middle
        // one. The first is not levelled: it is the parameters' defaults
        let recording = guitar_di(SAMPLE_RATE);
        let heard: Vec<f32> = PRESETS[1..]
            .iter()
            .map(|preset| loudness_at(preset, &recording, REFERENCE_DI_DB))
            .collect();
        let mut sorted = heard.clone();
        sorted.sort_by(f32::total_cmp);
        let middle = sorted[sorted.len() / 2];
        for (preset, loudness) in PRESETS[1..].iter().zip(&heard) {
            assert!(
                (loudness - middle).abs() < 2.0,
                "{} is at {loudness:.1} LUFS, the middle one at {middle:.1}",
                preset.name
            );
        }
    }

    /// What a preset's gate does to `input`: the gate's gain for every sample
    fn gate_trace(preset: &Preset, input: &[f32]) -> Vec<f32> {
        let mut gate = Gate::new();
        gate.set_sample_rate(SAMPLE_RATE);
        gate.set(preset.gate.0, preset.gate.1, preset.gate.2);
        gain_trace(&mut gate, input)
    }

    fn at_peak_db(mut signal: Vec<f32>, peak_db: f32) -> Vec<f32> {
        let scale = 10f32.powf(peak_db / 20.0) / peak(&signal);
        signal.iter_mut().for_each(|sample| *sample *= scale);
        signal
    }

    /// A power chord on the low string, struck once and left to ring
    fn held_chord(seconds: f32) -> Vec<f32> {
        let len = (seconds * SAMPLE_RATE) as usize;
        let mut chord = vec![0.0; len];
        for (string, freq_hz) in [82.41, 123.47, 164.81].into_iter().enumerate() {
            let offset = string * (0.004 * SAMPLE_RATE) as usize;
            let note = pluck(freq_hz, Pluck::OPEN, 7 + string as u32, SAMPLE_RATE, len - offset);
            for (sample, note) in chord[offset..].iter_mut().zip(note) {
                *sample += note;
            }
        }
        chord
    }

    /// Two bars of palm mutes, a rest of half a second after each bar
    fn mutes_with_rests() -> Vec<f32> {
        let bar = palm_mutes(SAMPLE_RATE, 1.2);
        let rest = vec![0.0; (0.5 * SAMPLE_RATE) as usize];
        [&bar[..], &rest[..], &bar[..], &rest[..]].concat()
    }

    /// How far a ringing sound had fallen from its loudest, in dB, when the gate started
    /// to close on it, and how long after its start that was. Nothing if the gate never
    /// opened. Levels are peaks over 30 ms, as a listener and the gate's detector hear them
    fn cut(trace: &[f32], input: &[f32]) -> Option<(f32, f32)> {
        let opened = trace.iter().position(|&gain| gain == 1.0)?;
        let closing = opened
            + trace[opened..].iter().position(|&gain| gain < 1.0).unwrap_or(trace.len() - opened);
        let window = (0.03 * SAMPLE_RATE) as usize;
        let level_at = |at: usize| {
            peak(&input[at.saturating_sub(window).min(input.len() - window)..][..window])
        };
        let fallen = to_db(peak(input) / level_at(closing.min(input.len())).max(1e-9));
        Some((fallen, closing as f32 / SAMPLE_RATE))
    }

    #[test]
    fn test_gates_leave_quiet_playing_and_ringing_chords_alone() {
        let chord = at_peak_db(held_chord(6.0), -12.0);
        let quiet_mutes = at_peak_db(mutes_with_rests(), -30.0);
        let bar = (1.2 * SAMPLE_RATE) as usize;
        let lead = (0.01 * SAMPLE_RATE) as usize;
        for preset in &PRESETS {
            // A chord that is held has fallen 40 dB before the gate starts to close on it
            let (fallen, _) = cut(&gate_trace(preset, &chord), &chord).unwrap();
            assert!(fallen >= 40.0, "{} cuts a held chord {fallen:.0} dB down", preset.name);

            // Palm mutes played quietly open the gate and it stays open through the bar
            let trace = gate_trace(preset, &quiet_mutes);
            assert!(
                trace[lead..bar].iter().all(|&gain| gain == 1.0),
                "{} chops quiet palm mutes",
                preset.name
            );

            // And in the rest behind the bar it shuts: the rhythm presets of the high gain
            // amp within an eighth of a second of the last mute
            let shut = trace[bar - lead..].iter().position(|&gain| gain == 0.0);
            if preset.amp == Amp::Torden && preset.gate.2 <= 60.0 {
                let limit = (0.125 * SAMPLE_RATE) as usize;
                assert!(
                    shut.is_some_and(|samples| samples <= limit),
                    "{} shuts after {shut:?} samples",
                    preset.name
                );
            }
        }
    }

    /// How the gate of every preset treats notes that ring out, quiet playing and palm
    /// mutes. Run after changing a preset's gate:
    /// cargo test -p amp --release preset_gate_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn preset_gate_report() {
        let note = decaying_note(SAMPLE_RATE, 8.0);
        let chord = held_chord(8.0);
        let mutes = mutes_with_rests();
        println!("\nA low E and a power chord left to ring, peaking at -12 and at -30 dBFS: how far each has");
        println!("fallen when the gate starts to close on it, in dB, and after how many seconds (`shut`: never");
        println!("opened). Palm mutes with a rest after each bar, peaking at -12 and at -30 dBFS: `gaps` is how");
        println!("often the gate started to close between two mutes of a bar (0 is right), `rest` the time from");
        println!("the last mute of a bar to silence in ms. `hiss`: the gate on noise at -70 and -76 dBFS RMS");
        println!(
            "{:<8} {:>6} {:>5} {:>13} {:>13} {:>13} {:>13} {:>10} {:>10} {:>13}",
            "preset",
            "thresh",
            "rel",
            "note -12",
            "note -30",
            "chord -12",
            "chord -30",
            "mutes -12",
            "mutes -30",
            "hiss -70/-76"
        );
        for preset in &PRESETS {
            print!("{:<8} {:>6.0} {:>5.0}", preset.name, preset.gate.1, preset.gate.2);
            for sound in [&note, &chord] {
                for peak_db in [-12.0, -30.0] {
                    let input = at_peak_db(sound.clone(), peak_db);
                    match cut(&gate_trace(preset, &input), &input) {
                        Some((fallen, at_s)) => {
                            print!(" {:>13}", format!("{fallen:.0} dB {at_s:.2} s"))
                        }
                        None => print!(" {:>13}", "shut"),
                    }
                }
            }
            for peak_db in [-12.0, -30.0] {
                let input = at_peak_db(mutes.clone(), peak_db);
                let trace = gate_trace(preset, &input);
                // The bars are 1.2 s long with half a second of rest behind each
                let bar = (1.2 * SAMPLE_RATE) as usize;
                let period = (1.7 * SAMPLE_RATE) as usize;
                let lead = (0.01 * SAMPLE_RATE) as usize;
                let gaps: usize = (0..2)
                    .map(|index| {
                        gate_changes(&trace[index * period + lead..index * period + bar]) / 2
                    })
                    .sum();
                let opened = trace[..bar].iter().any(|&gain| gain == 1.0);
                // The last mute is cut off 10 ms before the bar ends
                let last_ends = bar - (0.01 * SAMPLE_RATE) as usize;
                let silent = trace[last_ends..period].iter().position(|&gain| gain == 0.0);
                let rest = match (opened, silent) {
                    (false, _) => "shut".to_string(),
                    (true, Some(samples)) => {
                        format!("{gaps} {:.0} ms", samples as f32 / SAMPLE_RATE * 1000.0)
                    }
                    (true, None) => format!("{gaps} open"),
                };
                print!(" {rest:>10}");
            }
            let on_hiss = [-70.0, -76.0].map(|level_db| {
                let trace = gate_trace(preset, &hiss(level_db, 96_000));
                if trace.iter().all(|&gain| gain == 0.0) {
                    "shut"
                } else {
                    "open"
                }
            });
            println!(" {:>13}", on_hiss.join(" / "));
        }
    }

    /// Every preset as a WAV file in target/renders (amp_preset_*.wav), played with the
    /// made-up guitar at the reference level, and `amp_presets_in_a_row.wav`: the first
    /// four seconds of each, one after the other, for hearing whether they are equally loud.
    /// Set AMP_INPUT_WAV to the path of a recording to use that instead, at its own level:
    ///   cargo test -p amp --release render_presets -- --ignored
    #[test]
    #[ignore]
    fn render_presets() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();
        let input = match std::env::var("AMP_INPUT_WAV") {
            Ok(path) => {
                let (recording, sample_rate) = read_wav_mono(&path);
                assert_eq!(
                    sample_rate as f32, SAMPLE_RATE,
                    "The presets are rendered at {SAMPLE_RATE} Hz"
                );
                recording
            }
            Err(_) => at_rms_db(&guitar_di(SAMPLE_RATE), REFERENCE_DI_DB),
        };
        let part = input.len().min(4 * SAMPLE_RATE as usize);
        let (mut row_left, mut row_right) = (Vec::new(), Vec::new());
        for (index, preset) in PRESETS.iter().enumerate() {
            let (left, right) = run(&preset.settings(), &input);
            let name = format!("amp_preset_{:02}_{}.wav", index, preset.name.to_lowercase());
            write_wav(&dir.join(name), &[&left, &right], SAMPLE_RATE as u32);
            row_left.extend_from_slice(&left[..part]);
            row_right.extend_from_slice(&right[..part]);
        }
        write_wav(
            &dir.join("amp_presets_in_a_row.wav"),
            &[&row_left, &row_right],
            SAMPLE_RATE as u32,
        );
    }

    /// Level and balance of every preset. Run after changing a preset or the amps:
    /// cargo test -p amp --release preset_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn preset_report() {
        let chords = power_chords(SAMPLE_RATE, 4.0);
        let mutes = palm_mutes(SAMPLE_RATE, 2.4);
        let recordings = [guitar_di(SAMPLE_RATE), clean_di(SAMPLE_RATE)];
        println!("\nChords: level in dBFS. Palm mutes: dB relative to the whole signal. Loudness: LUFS of the");
        println!(
            "left channel, for riffs and for clean playing recorded at -30, -24 and -18 dBFS RMS"
        );
        println!(
            "{:<8} {:<8} {:>7} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>20} {:>20}",
            "preset",
            "amp",
            "RMS",
            "peak",
            "to 100",
            "100-400",
            "400-1k6",
            "1k6-6k4",
            "6k4 up",
            "riffs",
            "clean"
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
            for recording in &recordings {
                let heard = REPORT_DI_LEVELS_DB
                    .map(|level| format!("{:.1}", loudness_at(preset, recording, level)));
                print!(" {:>20}", heard.join(" / "));
            }
            println!();
        }
    }
}
