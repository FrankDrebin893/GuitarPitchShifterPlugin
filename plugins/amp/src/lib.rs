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
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "bypass"]
    pub bypass: BoolParam,

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

            amp: EnumParam::new("Amp", defaults.amp),

            gain: dial_param("Gain", defaults.gain),
            bass: dial_param("Bass", defaults.bass),
            mid: dial_param("Mid", defaults.mid),
            treble: dial_param("Treble", defaults.treble),
            presence: dial_param("Presence", defaults.presence),
            master: dial_param("Master", defaults.master),

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
