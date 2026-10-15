use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod amp;
mod board;
mod cab;
mod cab_stage;
mod chain;
mod comp;
mod delay;
mod dial_memory;
mod drive;
mod dsp;
mod editor;
mod gate;
mod lamps;
mod mods;
mod order;
mod presets;
mod reverb;
#[cfg(test)]
mod test_util;
mod tuner;
mod user_cab;
pub use amp::model::Amp;
use cab_stage::CabLoader;
use chain::{AmpChain, AmpSettings, GATE_RELEASE_MS, GATE_THRESHOLD_DB, IN_GAIN_DB};
use comp::CompSettings;
use delay::{DelaySettings, FEEDBACK_MAX, TIME_MAX_MS, TIME_MIN_MS};
use dial_memory::DialMemory;
use mods::chorus::{self, ChorusSettings};
use mods::flanger::{self, FlangerSettings};
use mods::phaser::{self, PhaserSettings};
use mods::tremolo::{self, TremoloSettings};
use order::StoredOrder;
use reverb::{ReverbSettings, DECAY_MAX_S, DECAY_MIN_S};

// The id of the tuner switch, as in `GuitarAmpParams`
const TUNER_ID: &str = "tuner";

const LEVEL_MIN_DB: f32 = -30.0;
const LEVEL_MAX_DB: f32 = 6.0;

pub struct GuitarAmpPlugin {
    params: Arc<GuitarAmpParams>,
    chain: AmpChain,
    // Reads the player's own cabinet from its file and leaves it for the chain to take
    cab_loader: Arc<CabLoader>,
}

/// What is done away from the audio thread
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Read the chosen cabinet, for the sample rate the host runs at now
    LoadCabinet,
}

// Frozen: the ids below are what DAW projects store. Add parameters, never rename an id.
// The full table of ids is in `ids.txt`, held by the `ids` test
#[derive(Params)]
pub struct GuitarAmpParams {
    #[persist = "editor-state-rig3"]
    pub editor_state: Arc<EguiState>,

    /// Index into `presets::PRESETS` of the preset loaded last. Not a parameter: the editor
    /// sets the parameters themselves when a preset is loaded
    #[persist = "preset"]
    pub preset: std::sync::atomic::AtomicU32,

    /// File name, in the cabinets folder, of the player's own cabinet. Empty for the amp's
    /// own. Not a parameter: a file name cannot be one. The audio thread never reads it
    #[persist = "cabinet"]
    pub cabinet: std::sync::Mutex<String>,

    /// Where each amp's six dials stood when the player last switched away from it. Not
    /// parameters: the editor sets the dials themselves when an amp is picked on the head.
    /// The audio thread never reads it
    #[persist = "amp-dials"]
    pub dial_memory: DialMemory,

    /// The order the pedals stand in. Not a parameter: an arrangement cannot be automated.
    /// The editor changes it, and the audio thread reads it with the dials
    #[persist = "chain-order"]
    pub order: StoredOrder,

    #[id = "bypass"]
    pub bypass: BoolParam,

    // Input and gate
    #[id = "in_gain"]
    pub in_gain: FloatParam,

    #[id = "gate_on"]
    pub gate_on: BoolParam,

    #[id = "gate_thresh"]
    pub gate_thresh: FloatParam,

    #[id = "gate_release"]
    pub gate_release: FloatParam,

    // Compressor pedal
    #[id = "comp_on"]
    pub comp_on: BoolParam,

    #[id = "comp_sustain"]
    pub comp_sustain: FloatParam,

    #[id = "comp_level"]
    pub comp_level: FloatParam,

    // Drive pedal
    #[id = "drive_on"]
    pub drive_on: BoolParam,

    #[id = "drive_gain"]
    pub drive_gain: FloatParam,

    #[id = "drive_tone"]
    pub drive_tone: FloatParam,

    #[id = "drive_level"]
    pub drive_level: FloatParam,

    // Amp
    #[id = "amp"]
    pub amp: EnumParam<Amp>,

    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "bass"]
    pub bass: FloatParam,

    #[id = "mid"]
    pub mid: FloatParam,

    #[id = "treble"]
    pub treble: FloatParam,

    #[id = "presence"]
    pub presence: FloatParam,

    #[id = "master"]
    pub master: FloatParam,

    // Modulation pedals
    #[id = "chorus_on"]
    pub chorus_on: BoolParam,

    #[id = "chorus_rate"]
    pub chorus_rate: FloatParam,

    #[id = "chorus_depth"]
    pub chorus_depth: FloatParam,

    #[id = "phaser_on"]
    pub phaser_on: BoolParam,

    #[id = "phaser_rate"]
    pub phaser_rate: FloatParam,

    #[id = "phaser_depth"]
    pub phaser_depth: FloatParam,

    #[id = "flanger_on"]
    pub flanger_on: BoolParam,

    #[id = "flanger_rate"]
    pub flanger_rate: FloatParam,

    #[id = "flanger_depth"]
    pub flanger_depth: FloatParam,

    #[id = "trem_on"]
    pub trem_on: BoolParam,

    #[id = "trem_rate"]
    pub trem_rate: FloatParam,

    #[id = "trem_depth"]
    pub trem_depth: FloatParam,

    // Cabinet
    #[id = "cab_on"]
    pub cab_on: BoolParam,

    #[id = "cab_mic"]
    pub cab_mic: FloatParam,

    #[id = "cab_res"]
    pub cab_res: FloatParam,

    // Delay
    #[id = "delay_on"]
    pub delay_on: BoolParam,

    #[id = "delay_time"]
    pub delay_time: FloatParam,

    #[id = "delay_feedback"]
    pub delay_feedback: FloatParam,

    #[id = "delay_mix"]
    pub delay_mix: FloatParam,

    // Reverb
    #[id = "reverb_on"]
    pub reverb_on: BoolParam,

    #[id = "reverb_decay"]
    pub reverb_decay: FloatParam,

    #[id = "reverb_mix"]
    pub reverb_mix: FloatParam,

    // Output
    #[id = "out_level"]
    pub out_level: FloatParam,

    // Tuner: a switch on the head, not part of the sound. Presets and saved projects leave it alone
    #[id = "tuner"]
    pub tuner_on: BoolParam,
}

impl Default for GuitarAmpPlugin {
    fn default() -> Self {
        let chain = AmpChain::new();
        let cab_loader = Arc::new(CabLoader::new(chain.cab_stage(), chain.sample_rate()));
        Self { params: Arc::new(GuitarAmpParams::default()), chain, cab_loader }
    }
}

impl Default for GuitarAmpParams {
    fn default() -> Self {
        // Every default comes from the settings the chain itself starts with, and the
        // ranges from the limits the chain and the effects keep to
        let defaults = AmpSettings::default();
        Self {
            editor_state: editor::default_state(),
            preset: std::sync::atomic::AtomicU32::new(0),
            cabinet: std::sync::Mutex::new(String::new()),
            order: StoredOrder::new(),
            dial_memory: DialMemory::new([
                defaults.gain,
                defaults.bass,
                defaults.mid,
                defaults.treble,
                defaults.presence,
                defaults.master,
            ]),

            bypass: BoolParam::new("Bypass", defaults.bypass).make_bypass(),

            in_gain: FloatParam::new(
                "Input",
                defaults.in_gain_db,
                FloatRange::Linear { min: IN_GAIN_DB[0], max: IN_GAIN_DB[1] },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            gate_on: BoolParam::new("Gate", defaults.gate_on),
            gate_thresh: FloatParam::new(
                "Gate Threshold",
                defaults.gate_thresh_db,
                FloatRange::Linear { min: GATE_THRESHOLD_DB[0], max: GATE_THRESHOLD_DB[1] },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            gate_release: FloatParam::new(
                "Gate Release",
                defaults.gate_release_ms,
                FloatRange::Skewed {
                    min: GATE_RELEASE_MS[0],
                    max: GATE_RELEASE_MS[1],
                    factor: FloatRange::skew_factor(-1.0), // More resolution at low end
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            comp_on: BoolParam::new("Comp", defaults.comp.on),
            comp_sustain: dial_param("Comp Sustain", defaults.comp.sustain),
            comp_level: dial_param("Comp Level", defaults.comp.level),

            drive_on: BoolParam::new("Drive", defaults.drive_on),
            drive_gain: dial_param("Drive Gain", defaults.drive_gain),
            drive_tone: dial_param("Drive Tone", defaults.drive_tone),
            drive_level: dial_param("Drive Level", defaults.drive_level),

            amp: EnumParam::new("Amp", defaults.amp),

            gain: dial_param("Gain", defaults.gain),
            bass: dial_param("Bass", defaults.bass),
            mid: dial_param("Mid", defaults.mid),
            treble: dial_param("Treble", defaults.treble),
            presence: dial_param("Presence", defaults.presence),
            master: dial_param("Master", defaults.master),

            chorus_on: BoolParam::new("Chorus", defaults.chorus.on),
            chorus_rate: rate_param(
                "Chorus Rate",
                defaults.chorus.rate_hz,
                chorus::RATE_MIN_HZ,
                chorus::RATE_MAX_HZ,
            ),
            chorus_depth: percent_param("Chorus Depth", defaults.chorus.depth, 1.0),
            phaser_on: BoolParam::new("Phaser", defaults.phaser.on),
            phaser_rate: rate_param(
                "Phaser Rate",
                defaults.phaser.rate_hz,
                phaser::RATE_MIN_HZ,
                phaser::RATE_MAX_HZ,
            ),
            phaser_depth: percent_param("Phaser Depth", defaults.phaser.depth, 1.0),
            flanger_on: BoolParam::new("Flanger", defaults.flanger.on),
            flanger_rate: rate_param(
                "Flanger Rate",
                defaults.flanger.rate_hz,
                flanger::RATE_MIN_HZ,
                flanger::RATE_MAX_HZ,
            ),
            flanger_depth: percent_param("Flanger Depth", defaults.flanger.depth, 1.0),
            trem_on: BoolParam::new("Tremolo", defaults.tremolo.on),
            trem_rate: rate_param(
                "Tremolo Rate",
                defaults.tremolo.rate_hz,
                tremolo::RATE_MIN_HZ,
                tremolo::RATE_MAX_HZ,
            ),
            trem_depth: percent_param("Tremolo Depth", defaults.tremolo.depth, 1.0),

            cab_on: BoolParam::new("Cabinet", defaults.cab_on),
            cab_mic: dial_param("Cab Mic", defaults.cab_mic),
            cab_res: dial_param("Cab Resonance", defaults.cab_res),

            delay_on: BoolParam::new("Delay", defaults.delay.on),
            delay_time: FloatParam::new(
                "Delay Time",
                defaults.delay.time_ms,
                FloatRange::Skewed {
                    min: TIME_MIN_MS,
                    max: TIME_MAX_MS,
                    factor: FloatRange::skew_factor(-1.0), // More resolution at low end
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            delay_feedback: percent_param("Delay Feedback", defaults.delay.feedback, FEEDBACK_MAX),
            delay_mix: percent_param("Delay Mix", defaults.delay.mix, 1.0),

            reverb_on: BoolParam::new("Reverb", defaults.reverb.on),
            reverb_decay: FloatParam::new(
                "Reverb Decay",
                defaults.reverb.decay_s,
                FloatRange::Skewed {
                    min: DECAY_MIN_S,
                    max: DECAY_MAX_S,
                    factor: FloatRange::skew_factor(-1.0), // More resolution at low end
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            reverb_mix: percent_param("Reverb Mix", defaults.reverb.mix, 1.0),

            out_level: FloatParam::new(
                "Output",
                defaults.out_level,
                FloatRange::Skewed {
                    min: util::db_to_gain(LEVEL_MIN_DB),
                    max: util::db_to_gain(LEVEL_MAX_DB),
                    factor: FloatRange::gain_skew_factor(LEVEL_MIN_DB, LEVEL_MAX_DB),
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            tuner_on: BoolParam::new("Tuner", defaults.tuner_on).non_automatable(),
        }
    }
}

impl GuitarAmpParams {
    /// The amp's own six dials, in the order they stand on the head
    pub fn amp_dials(&self) -> [&FloatParam; 6] {
        [&self.gain, &self.bass, &self.mid, &self.treble, &self.presence, &self.master]
    }

    /// What the knobs say now
    fn settings(&self) -> AmpSettings {
        AmpSettings {
            bypass: self.bypass.value(),
            in_gain_db: self.in_gain.value(),
            gate_on: self.gate_on.value(),
            gate_thresh_db: self.gate_thresh.value(),
            gate_release_ms: self.gate_release.value(),
            comp: CompSettings {
                on: self.comp_on.value(),
                sustain: self.comp_sustain.value(),
                level: self.comp_level.value(),
            },
            drive_on: self.drive_on.value(),
            drive_gain: self.drive_gain.value(),
            drive_tone: self.drive_tone.value(),
            drive_level: self.drive_level.value(),
            amp: self.amp.value(),
            gain: self.gain.value(),
            bass: self.bass.value(),
            mid: self.mid.value(),
            treble: self.treble.value(),
            presence: self.presence.value(),
            master: self.master.value(),
            chorus: ChorusSettings {
                on: self.chorus_on.value(),
                rate_hz: self.chorus_rate.value(),
                depth: self.chorus_depth.value(),
            },
            phaser: PhaserSettings {
                on: self.phaser_on.value(),
                rate_hz: self.phaser_rate.value(),
                depth: self.phaser_depth.value(),
            },
            flanger: FlangerSettings {
                on: self.flanger_on.value(),
                rate_hz: self.flanger_rate.value(),
                depth: self.flanger_depth.value(),
            },
            tremolo: TremoloSettings {
                on: self.trem_on.value(),
                rate_hz: self.trem_rate.value(),
                depth: self.trem_depth.value(),
            },
            cab_on: self.cab_on.value(),
            cab_mic: self.cab_mic.value(),
            cab_res: self.cab_res.value(),
            delay: DelaySettings {
                on: self.delay_on.value(),
                time_ms: self.delay_time.value(),
                feedback: self.delay_feedback.value(),
                mix: self.delay_mix.value(),
            },
            reverb: ReverbSettings {
                on: self.reverb_on.value(),
                decay_s: self.reverb_decay.value(),
                mix: self.reverb_mix.value(),
            },
            out_level: self.out_level.value(),
            tuner_on: self.tuner_on.value(),
            order: self.order.get(),
        }
    }
}

/// 0.0-1.0 parameter shown as an amp dial, 0.0 to 10.0
fn dial_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(Arc::new(|value| format!("{:.1}", value * 10.0)))
        .with_string_to_value(Arc::new(|text| {
            text.trim().parse::<f32>().ok().map(|dial| dial / 10.0)
        }))
}

/// The rate of a modulation pedal, in Hz
fn rate_param(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(-1.0), // More resolution at low end
        },
    )
    .with_unit(" Hz")
    .with_value_to_string(formatters::v2s_f32_rounded(2))
}

/// 0.0 to `max` parameter shown as a percentage
fn percent_param(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Plugin for GuitarAmpPlugin {
    const NAME: &'static str = "Hojt Guitar Amp";
    const VENDOR: &'static str = suite_common::VENDOR;
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    // The chain smooths its settings itself, once per chunk
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = Task;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = self.params.clone();
        let cab_loader = self.cab_loader.clone();
        Box::new(move |task| match task {
            Task::LoadCabinet => cab_loader.load(&params.cabinet),
        })
    }

    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.params.editor_state.clone(),
            self.chain.tuner_reading(),
            self.chain.lamp_reading(),
            self.cab_loader.clone(),
            async_executor,
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.chain.set_sample_rate(buffer_config.sample_rate);
        // The player's own cabinet is made for one sample rate, and a project that was
        // just loaded may name another file: read it again. Done before this returns, so
        // the first block already plays through it. With no file chosen no disk is read
        self.cab_loader.set_sample_rate(buffer_config.sample_rate);
        context.execute(Task::LoadCabinet);
        true
    }

    fn reset(&mut self) {
        self.chain.reset();
    }

    /// The tuner is a parameter, so hosts store it, but a project does not open muted
    /// because it was saved while tuning: the switch stays as it is when a state is loaded,
    /// which is off in a plugin that was just created
    fn filter_state(state: &mut PluginState) {
        state.params.remove(TUNER_ID);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let settings = self.params.settings();

        match buffer.as_slice() {
            [left, right] => self.chain.process(&settings, left, Some(&mut **right)),
            [mono] => self.chain.process(&settings, mono, None),
            _ => {}
        }

        // The repeats and the reverb tail go on after the input has stopped. The chain
        // knows when the last of them has died away, to the sample; the host may stop
        // calling from there on. `Tail` would need their length ahead of time, which with
        // feedback is a guess, and gains nothing: CLAP hosts are told to go on either way
        if self.chain.is_idle() {
            ProcessStatus::Normal
        } else {
            ProcessStatus::KeepAlive
        }
    }
}

impl ClapPlugin for GuitarAmpPlugin {
    // Frozen: DAW projects find the plugin by this ID. Never change it
    const CLAP_ID: &'static str = "com.guitarpitchshifter.guitar-amp";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A low-latency guitar amp with pedals, cabinet, delay and reverb");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Distortion,
        ClapFeature::Mono,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for GuitarAmpPlugin {
    // Frozen: DAW projects find the plugin by this ID. Never change it
    const VST3_CLASS_ID: [u8; 16] = *b"HojtGuitarAmp003";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
    // The delay and reverb ring on after the input stops, for as long as their settings say. A host that asks only once, before the first block, would hear 0
    const VST3_INFINITE_TAIL: bool = true;
}

nih_export_clap!(GuitarAmpPlugin);
nih_export_vst3!(GuitarAmpPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameters_start_at_the_settings_the_chain_starts_with() {
        let params = GuitarAmpParams::default();
        assert_eq!(params.settings(), AmpSettings::default());

        // The values DAW projects were saved against. Changing a default changes how a
        // project sounds that never touched that knob
        let settings = params.settings();
        assert!(!settings.bypass && settings.gate_on && !settings.drive_on && settings.cab_on);
        assert_eq!(
            (settings.comp.on, settings.comp.sustain, settings.comp.level),
            (false, 0.5, 0.5)
        );
        assert_eq!(
            (settings.chorus.on, settings.chorus.rate_hz, settings.chorus.depth),
            (false, 0.8, 0.5)
        );
        assert_eq!(
            (settings.phaser.on, settings.phaser.rate_hz, settings.phaser.depth),
            (false, 0.5, 0.5)
        );
        assert_eq!(
            (settings.flanger.on, settings.flanger.rate_hz, settings.flanger.depth),
            (false, 0.3, 0.5)
        );
        assert_eq!(
            (settings.tremolo.on, settings.tremolo.rate_hz, settings.tremolo.depth),
            (false, 5.0, 0.5)
        );
        assert!(!settings.delay.on && !settings.reverb.on && !settings.tuner_on);
        assert_eq!(settings.amp, Amp::Brol);
        assert_eq!(
            (settings.in_gain_db, settings.gate_thresh_db, settings.gate_release_ms),
            (0.0, -60.0, 100.0)
        );
        assert_eq!(
            (settings.drive_gain, settings.drive_tone, settings.drive_level),
            (0.3, 0.5, 0.5)
        );
        assert_eq!(
            [
                settings.gain,
                settings.bass,
                settings.mid,
                settings.treble,
                settings.presence,
                settings.master
            ],
            [0.5; 6]
        );
        assert_eq!((settings.cab_mic, settings.cab_res), (0.5, 0.5));
        assert_eq!(
            (settings.delay.time_ms, settings.delay.feedback, settings.delay.mix),
            (350.0, 0.35, 0.25)
        );
        assert_eq!((settings.reverb.decay_s, settings.reverb.mix), (1.5, 0.2));
        assert_eq!(settings.out_level, 1.0);
    }

    #[test]
    fn test_parameter_ranges_are_the_limits_of_the_chain() {
        let params = GuitarAmpParams::default();
        let range = |param: &FloatParam| (param.preview_plain(0.0), param.preview_plain(1.0));
        assert_eq!(range(&params.in_gain), (-24.0, 24.0));
        assert_eq!(range(&params.gate_thresh), (-80.0, -20.0));
        assert_eq!(range(&params.gate_release), (20.0, 500.0));
        assert_eq!(range(&params.delay_time), (20.0, 1000.0));
        assert_eq!(range(&params.delay_feedback), (0.0, 0.9));
        assert_eq!(range(&params.delay_mix), (0.0, 1.0));
        assert_eq!(range(&params.reverb_decay), (0.3, 6.0));
        assert_eq!(range(&params.reverb_mix), (0.0, 1.0));
        assert_eq!(range(&params.chorus_rate), (0.1, 6.0));
        assert_eq!(range(&params.phaser_rate), (0.1, 8.0));
        assert_eq!(range(&params.flanger_rate), (0.05, 5.0));
        assert_eq!(range(&params.trem_rate), (0.5, 12.0));
        for depth in
            [&params.chorus_depth, &params.phaser_depth, &params.flanger_depth, &params.trem_depth]
        {
            assert_eq!(range(depth), (0.0, 1.0));
        }
        for dial in [
            &params.gain,
            &params.drive_gain,
            &params.cab_mic,
            &params.comp_sustain,
            &params.comp_level,
        ] {
            assert_eq!(range(dial), (0.0, 1.0));
        }
    }
}

#[cfg(test)]
mod ids {
    /// Bundle name, plugin ids and every parameter id stay as saved projects know them
    #[test]
    fn ids() {
        suite_common::ids::check::<super::GuitarAmpPlugin>(
            env!("CARGO_MANIFEST_DIR"),
            include_str!("../../../bundler.toml"),
            "amp",
        );
    }

    #[test]
    fn vst3_tail_is_infinite() {
        use nih_plug::prelude::Vst3Plugin;
        let infinite = super::GuitarAmpPlugin::VST3_INFINITE_TAIL;
        assert!(infinite);
    }
}
