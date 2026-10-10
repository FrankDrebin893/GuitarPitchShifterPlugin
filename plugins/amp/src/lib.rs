use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod amp;
mod cab;
mod cab_stage;
mod chain;
mod delay;
mod drive;
mod dsp;
mod editor;
mod gate;
mod presets;
mod reverb;
#[cfg(test)]
mod test_util;
mod tuner;
mod user_cab;
pub use amp::model::Amp;
use cab_stage::CabLoader;
use chain::{AmpChain, AmpSettings, GATE_RELEASE_MS, GATE_THRESHOLD_DB, IN_GAIN_DB};
use delay::{DelaySettings, FEEDBACK_MAX, TIME_MAX_MS, TIME_MIN_MS};
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
// The full table of ids, including the ones not added yet, is in docs/amp-progress.md
#[derive(Params)]
pub struct GuitarAmpParams {
    #[persist = "editor-state-rig2"]
    pub editor_state: Arc<EguiState>,

    /// Index into `presets::PRESETS` of the preset loaded last. Not a parameter: the editor
    /// sets the parameters themselves when a preset is loaded
    #[persist = "preset"]
    pub preset: std::sync::atomic::AtomicU32,

    /// File name, in the cabinets folder, of the player's own cabinet. Empty for the amp's
    /// own. Not a parameter: a file name cannot be one. The audio thread never reads it
    #[persist = "cabinet"]
    pub cabinet: std::sync::Mutex<String>,

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
        Self {
            params: Arc::new(GuitarAmpParams::default()),
            chain,
            cab_loader,
        }
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

            bypass: BoolParam::new("Bypass", defaults.bypass).make_bypass(),

            in_gain: FloatParam::new(
                "Input",
                defaults.in_gain_db,
                FloatRange::Linear {
                    min: IN_GAIN_DB[0],
                    max: IN_GAIN_DB[1],
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            gate_on: BoolParam::new("Gate", defaults.gate_on),
            gate_thresh: FloatParam::new(
                "Gate Threshold",
                defaults.gate_thresh_db,
                FloatRange::Linear {
                    min: GATE_THRESHOLD_DB[0],
                    max: GATE_THRESHOLD_DB[1],
                },
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
    /// What the knobs say now
    fn settings(&self) -> AmpSettings {
        AmpSettings {
            bypass: self.bypass.value(),
            in_gain_db: self.in_gain.value(),
            gate_on: self.gate_on.value(),
            gate_thresh_db: self.gate_thresh.value(),
            gate_release_ms: self.gate_release.value(),
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
        }
    }
}

/// 0.0-1.0 parameter shown as an amp dial, 0.0 to 10.0
fn dial_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(Arc::new(|value| format!("{:.1}", value * 10.0)))
        .with_string_to_value(Arc::new(|text| text.trim().parse::<f32>().ok().map(|dial| dial / 10.0)))
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
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A low-latency guitar amp with pedals, cabinet, delay and reverb");
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
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Distortion,
    ];
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
        assert!(!settings.delay.on && !settings.reverb.on && !settings.tuner_on);
        assert_eq!(settings.amp, Amp::Brol);
        assert_eq!((settings.in_gain_db, settings.gate_thresh_db, settings.gate_release_ms), (0.0, -60.0, 100.0));
        assert_eq!((settings.drive_gain, settings.drive_tone, settings.drive_level), (0.3, 0.5, 0.5));
        assert_eq!([settings.gain, settings.bass, settings.mid, settings.treble, settings.presence, settings.master], [0.5; 6]);
        assert_eq!((settings.cab_mic, settings.cab_res), (0.5, 0.5));
        assert_eq!((settings.delay.time_ms, settings.delay.feedback, settings.delay.mix), (350.0, 0.35, 0.25));
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
        for dial in [&params.gain, &params.drive_gain, &params.cab_mic] {
            assert_eq!(range(dial), (0.0, 1.0));
        }
    }
}
