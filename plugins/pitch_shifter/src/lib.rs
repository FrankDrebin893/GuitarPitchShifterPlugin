use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod editor;
mod pitch_shifter;
use pitch_shifter::PitchShifter;

pub struct GuitarPitchShifterPlugin {
    params: Arc<GuitarPitchShifterParams>,
    pitch_shifter: PitchShifter,
}

#[derive(Params)]
pub struct GuitarPitchShifterParams {
    // Key changed with the pedal layout, so that projects saved with the old window size open at the new one
    #[persist = "editor-state-pedal"]
    pub editor_state: Arc<EguiState>,

    #[id = "semitones"]
    pub semitones: IntParam,

    #[id = "latency"]
    pub latency_ms: FloatParam,

    #[id = "smoothness"]
    pub smoothness_ms: FloatParam,

    #[id = "bypass"]
    pub bypass: BoolParam,
}

impl Default for GuitarPitchShifterPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(GuitarPitchShifterParams::default()),
            pitch_shifter: PitchShifter::new(),
        }
    }
}

impl Default for GuitarPitchShifterParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),

            semitones: IntParam::new(
                "Semitones",
                0,
                IntRange::Linear { min: -12, max: 12 },
            )
            .with_unit(" st"),

            latency_ms: FloatParam::new(
                "Max Latency",
                15.0,
                FloatRange::Skewed {
                    min: 2.0,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-1.0), // More resolution at low end
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            smoothness_ms: FloatParam::new(
                "Smoothness",
                2.0,
                FloatRange::Skewed {
                    min: 0.5,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            bypass: BoolParam::new("Bypass", false).make_bypass(),
        }
    }
}

impl Plugin for GuitarPitchShifterPlugin {
    const NAME: &'static str = "Hojt Pitch Shifter";
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
        self.pitch_shifter.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.pitch_shifter.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Bypass is a shift of 0 semitones: the shifter settles on its minimum delay
        // and then passes the input through bit-exactly, without a click
        let semitones = if self.params.bypass.value() { 0 } else { self.params.semitones.value() };
        self.pitch_shifter.set_semitones(semitones);
        self.pitch_shifter.set_latency_ms(self.params.latency_ms.value());
        self.pitch_shifter.set_smoothness_ms(self.params.smoothness_ms.value());

        let num_channels = buffer.channels();

        for mut frame in buffer.iter_samples() {
            if num_channels >= 2 {
                let left = *frame.get_mut(0).unwrap();
                let right = *frame.get_mut(1).unwrap();
                let (left, right) = self.pitch_shifter.process_stereo(left, right);
                *frame.get_mut(0).unwrap() = left;
                *frame.get_mut(1).unwrap() = right;
            } else if num_channels == 1 {
                let sample = frame.get_mut(0).unwrap();
                *sample = self.pitch_shifter.process_sample(*sample);
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for GuitarPitchShifterPlugin {
    const CLAP_ID: &'static str = "com.guitarpitchshifter.guitar-pitch-shifter";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A low-latency pitch transposition plugin");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::PitchShifter,
        ClapFeature::Mono,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for GuitarPitchShifterPlugin {
    // Frozen: DAW projects find the plugin by this ID. Never change it
    const VST3_CLASS_ID: [u8; 16] = *b"GuitarPShift0010";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::PitchShift,
    ];
}

nih_export_clap!(GuitarPitchShifterPlugin);
nih_export_vst3!(GuitarPitchShifterPlugin);
