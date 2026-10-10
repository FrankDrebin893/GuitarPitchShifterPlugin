use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod amp;
mod cab;
mod chain;
mod dsp;
mod editor;
#[cfg(test)]
mod test_util;
pub use amp::model::Amp;
use chain::{AmpChain, AmpSettings};

const LEVEL_MIN_DB: f32 = -30.0;
const LEVEL_MAX_DB: f32 = 6.0;

pub struct GuitarAmpPlugin {
    params: Arc<GuitarAmpParams>,
    chain: AmpChain,
}

// Frozen: the ids below are what DAW projects store. Add parameters, never rename an id.
// The full table of ids, including the ones not added yet, is in docs/amp-progress.md
#[derive(Params)]
pub struct GuitarAmpParams {
    #[persist = "editor-state-rig"]
    pub editor_state: Arc<EguiState>,

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
}

impl Default for GuitarAmpPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(GuitarAmpParams::default()),
            chain: AmpChain::new(),
        }
    }
}

impl Default for GuitarAmpParams {
    fn default() -> Self {
        let defaults = AmpSettings::default();
        Self {
            editor_state: editor::default_state(),

            bypass: BoolParam::new("Bypass", false).make_bypass(),

            in_gain: FloatParam::new("Input", 0.0, FloatRange::Linear { min: -24.0, max: 24.0 })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            gate_on: BoolParam::new("Gate", true),
            gate_thresh: FloatParam::new("Gate Threshold", -60.0, FloatRange::Linear { min: -80.0, max: -20.0 })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            gate_release: FloatParam::new(
                "Gate Release",
                100.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 500.0,
                    factor: FloatRange::skew_factor(-1.0), // More resolution at low end
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            drive_on: BoolParam::new("Drive", false),
            drive_gain: dial_param("Drive Gain", 0.3),
            drive_tone: dial_param("Drive Tone", 0.5),
            drive_level: dial_param("Drive Level", 0.5),

            amp: EnumParam::new("Amp", defaults.amp),

            gain: dial_param("Gain", defaults.gain),
            bass: dial_param("Bass", defaults.bass),
            mid: dial_param("Mid", defaults.mid),
            treble: dial_param("Treble", defaults.treble),
            presence: dial_param("Presence", defaults.presence),
            master: dial_param("Master", defaults.master),

            cab_on: BoolParam::new("Cabinet", true),
            cab_mic: dial_param("Cab Mic", 0.5),
            cab_res: dial_param("Cab Resonance", 0.5),

            delay_on: BoolParam::new("Delay", false),
            delay_time: FloatParam::new(
                "Delay Time",
                350.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 1000.0,
                    factor: FloatRange::skew_factor(-1.0), // More resolution at low end
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            delay_feedback: percent_param("Delay Feedback", 0.35, 0.9),
            delay_mix: percent_param("Delay Mix", 0.25, 1.0),

            reverb_on: BoolParam::new("Reverb", false),
            reverb_decay: FloatParam::new(
                "Reverb Decay",
                1.5,
                FloatRange::Skewed {
                    min: 0.3,
                    max: 6.0,
                    factor: FloatRange::skew_factor(-1.0), // More resolution at low end
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            reverb_mix: percent_param("Reverb Mix", 0.2, 1.0),

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
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.params.editor_state.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.chain.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.chain.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let settings = AmpSettings {
            bypass: self.params.bypass.value(),
            amp: self.params.amp.value(),
            gain: self.params.gain.value(),
            bass: self.params.bass.value(),
            mid: self.params.mid.value(),
            treble: self.params.treble.value(),
            presence: self.params.presence.value(),
            master: self.params.master.value(),
            out_level: self.params.out_level.value(),
        };

        match buffer.as_slice() {
            [left, right] => self.chain.process(&settings, left, Some(&mut **right)),
            [mono] => self.chain.process(&settings, mono, None),
            _ => {}
        }

        ProcessStatus::Normal
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
