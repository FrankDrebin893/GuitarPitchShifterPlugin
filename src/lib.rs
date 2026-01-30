use nih_plug::prelude::*;
use std::sync::Arc;

mod pitch_shifter;
use pitch_shifter::PitchShifter;

const BLOCK_SIZE: usize = 512;

pub struct TransposePlugin {
    params: Arc<TransposeParams>,
    pitch_shifter_l: PitchShifter,
    pitch_shifter_r: PitchShifter,
    sample_rate: f32,
}

#[derive(Params)]
struct TransposeParams {
    #[id = "semitones"]
    pub semitones: IntParam,
}

impl Default for TransposePlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(TransposeParams::default()),
            pitch_shifter_l: PitchShifter::new(BLOCK_SIZE),
            pitch_shifter_r: PitchShifter::new(BLOCK_SIZE),
            sample_rate: 44100.0,
        }
    }
}

impl Default for TransposeParams {
    fn default() -> Self {
        Self {
            semitones: IntParam::new(
                "Semitones",
                0,
                IntRange::Linear { min: -12, max: 12 },
            )
            .with_unit(" st"),
        }
    }
}

impl Plugin for TransposePlugin {
    const NAME: &'static str = "Transpose Plugin v2";
    const VENDOR: &'static str = "Transpose";
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

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.pitch_shifter_l = PitchShifter::new(BLOCK_SIZE);
        self.pitch_shifter_r = PitchShifter::new(BLOCK_SIZE);
        self.pitch_shifter_l.set_sample_rate(buffer_config.sample_rate);
        self.pitch_shifter_r.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.pitch_shifter_l.reset();
        self.pitch_shifter_r.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let semitones = self.params.semitones.value();

        self.pitch_shifter_l.set_semitones(semitones);
        self.pitch_shifter_r.set_semitones(semitones);

        let num_channels = buffer.channels();

        for mut frame in buffer.iter_samples() {
            if num_channels >= 1 {
                let sample = frame.get_mut(0).unwrap();
                *sample = self.pitch_shifter_l.process_sample(*sample);
            }
            if num_channels >= 2 {
                let sample = frame.get_mut(1).unwrap();
                *sample = self.pitch_shifter_r.process_sample(*sample);
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for TransposePlugin {
    const CLAP_ID: &'static str = "com.transpose.transpose-plugin";
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

impl Vst3Plugin for TransposePlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"TransposePlug002";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::PitchShift,
    ];
}

nih_export_clap!(TransposePlugin);
nih_export_vst3!(TransposePlugin);
