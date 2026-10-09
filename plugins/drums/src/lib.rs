use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod editor;
mod kick;
use kick::Kick;

// General MIDI drum map
const NOTE_KICK: u8 = 36;

pub struct DrumSynthPlugin {
    params: Arc<DrumSynthParams>,
    kick: Kick,
}

#[derive(Params)]
pub struct DrumSynthParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "gain"]
    pub gain: FloatParam,
}

impl Default for DrumSynthPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(DrumSynthParams::default()),
            kick: Kick::new(),
        }
    }
}

impl Default for DrumSynthParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),

            gain: FloatParam::new(
                "Gain",
                util::db_to_gain(-6.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(editor::GAIN_MIN_DB),
                    max: util::db_to_gain(editor::GAIN_MAX_DB),
                    factor: FloatRange::gain_skew_factor(editor::GAIN_MIN_DB, editor::GAIN_MAX_DB),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl Plugin for DrumSynthPlugin {
    const NAME: &'static str = "Drum Synth v1";
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
        self.kick.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.kick.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();

        for (sample_id, mut frame) in buffer.iter_samples().enumerate() {
            // Handle all events that land on this sample
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                if let NoteEvent::NoteOn { note, velocity, .. } = event {
                    if note == NOTE_KICK {
                        self.kick.trigger(velocity);
                    }
                }

                next_event = context.next_event();
            }

            let gain = self.params.gain.smoothed.next();
            let output = self.kick.process_sample() * gain;

            for sample in frame.iter_mut() {
                *sample = output;
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
    const VST3_CLASS_ID: [u8; 16] = *b"SuiteDrumSyn0001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Drum,
    ];
}

nih_export_clap!(DrumSynthPlugin);
nih_export_vst3!(DrumSynthPlugin);
