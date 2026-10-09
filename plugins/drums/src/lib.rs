use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod drum_kit;
mod dsp;
mod editor;
mod kit;
mod reverb;
#[cfg(test)]
mod test_util;
mod voices;
use drum_kit::DrumKit;
use kit::Kit;

const LEVEL_MIN_DB: f32 = -30.0;
const LEVEL_MAX_DB: f32 = 6.0;

pub struct DrumSynthPlugin {
    params: Arc<DrumSynthParams>,
    drums: DrumKit,
}

#[derive(Params)]
pub struct DrumSynthParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "kit"]
    pub kit: EnumParam<Kit>,

    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "room"]
    pub room: FloatParam,

    #[id = "tune"]
    pub tune: FloatParam,

    #[id = "damping"]
    pub damping: FloatParam,

    #[id = "kick"]
    pub kick: FloatParam,

    #[id = "snare"]
    pub snare: FloatParam,

    #[id = "toms"]
    pub toms: FloatParam,

    #[id = "hihat"]
    pub hihat: FloatParam,

    #[id = "cymbals"]
    pub cymbals: FloatParam,
}

impl Default for DrumSynthPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(DrumSynthParams::default()),
            drums: DrumKit::new(),
        }
    }
}

impl Default for DrumSynthParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),

            kit: EnumParam::new("Kit", Kit::Rock),

            gain: level_param("Gain", -6.0),

            room: percent_param("Room", 0.25).with_smoother(SmoothingStyle::Linear(20.0)),

            tune: FloatParam::new("Tune", 0.0, FloatRange::Linear { min: -6.0, max: 6.0 })
                .with_step_size(0.1)
                .with_unit(" st")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),

            damping: percent_param("Damping", 0.0),

            kick: level_param("Kick", 0.0),
            snare: level_param("Snare", 0.0),
            toms: level_param("Toms", 0.0),
            hihat: level_param("Hi-Hat", 0.0),
            cymbals: level_param("Cymbals", 0.0),
        }
    }
}

/// Gain parameter shown in dB
fn level_param(name: &str, default_db: f32) -> FloatParam {
    FloatParam::new(
        name,
        util::db_to_gain(default_db),
        FloatRange::Skewed {
            min: util::db_to_gain(LEVEL_MIN_DB),
            max: util::db_to_gain(LEVEL_MAX_DB),
            factor: FloatRange::gain_skew_factor(LEVEL_MIN_DB, LEVEL_MAX_DB),
        },
    )
    .with_smoother(SmoothingStyle::Logarithmic(20.0))
    .with_unit(" dB")
    .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
    .with_string_to_value(formatters::s2v_f32_gain_to_db())
}

/// 0.0-1.0 parameter shown as a percentage
fn percent_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Plugin for DrumSynthPlugin {
    const NAME: &'static str = "Drum Synth v2";
    const VENDOR: &'static str = suite_common::VENDOR;
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

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
        self.drums.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.drums.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // These only affect the next hits, so once per block is enough
        self.drums.set_kit(self.params.kit.value());
        self.drums.set_tune(self.params.tune.value());
        self.drums.set_damping(self.params.damping.value());

        let mut next_event = context.next_event();

        for (sample_id, mut frame) in buffer.iter_samples().enumerate() {
            // Handle all events that land on this sample
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                if let NoteEvent::NoteOn { note, velocity, .. } = event {
                    self.drums.note_on(note, velocity);
                }

                next_event = context.next_event();
            }

            let mix = &mut self.drums.mix;
            mix.kick = self.params.kick.smoothed.next();
            mix.snare = self.params.snare.smoothed.next();
            mix.toms = self.params.toms.smoothed.next();
            mix.hihat = self.params.hihat.smoothed.next();
            mix.cymbals = self.params.cymbals.smoothed.next();
            mix.room = self.params.room.smoothed.next();

            let gain = self.params.gain.smoothed.next();
            let (left, right) = self.drums.process();

            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = if channel == 0 { left } else { right } * gain;
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for DrumSynthPlugin {
    const CLAP_ID: &'static str = "com.guitarpitchshifter.drum-synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A self-contained synthesized drum kit");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Drum,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for DrumSynthPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"SuiteDrumSyn0002";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Drum,
    ];
}

nih_export_clap!(DrumSynthPlugin);
nih_export_vst3!(DrumSynthPlugin);
