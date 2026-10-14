use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod bass;
mod editor;
use bass::{Bass, BassSettings, Voice};

pub struct BassSynthPlugin {
    params: Arc<BassSynthParams>,
    bass: Bass,
}

#[derive(Params)]
pub struct BassSynthParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "voice"]
    pub voice: EnumParam<Voice>,

    #[id = "volume"]
    pub volume: FloatParam,

    #[id = "drive"]
    pub drive: FloatParam,

    #[id = "brightness"]
    pub brightness: FloatParam,

    #[id = "sub"]
    pub sub: FloatParam,

    #[id = "tone"]
    pub tone: FloatParam,

    #[id = "sustain"]
    pub sustain: FloatParam,

    #[id = "slide"]
    pub slide: FloatParam,

    #[id = "transpose"]
    pub transpose: FloatParam,
}

impl Default for BassSynthPlugin {
    fn default() -> Self {
        Self { params: Arc::new(BassSynthParams::default()), bass: Bass::new() }
    }
}

impl Default for BassSynthParams {
    fn default() -> Self {
        // Every default is the one of the DSP
        let defaults = BassSettings::default();
        Self {
            editor_state: editor::default_state(),

            voice: EnumParam::new("Voice", defaults.voice),

            volume: FloatParam::new(
                "Volume",
                util::db_to_gain(defaults.volume_db),
                FloatRange::Skewed {
                    min: util::db_to_gain(bass::VOLUME_MIN_DB),
                    max: util::db_to_gain(bass::VOLUME_MAX_DB),
                    factor: FloatRange::gain_skew_factor(bass::VOLUME_MIN_DB, bass::VOLUME_MAX_DB),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            drive: percent_param("Drive", defaults.drive),
            brightness: percent_param("Brightness", defaults.brightness),
            sub: percent_param("Sub", defaults.sub),

            tone: FloatParam::new(
                "Tone",
                defaults.tone_hz,
                FloatRange::Skewed {
                    min: bass::TONE_MIN_HZ,
                    max: bass::TONE_MAX_HZ,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(1.0)
            .with_unit(" Hz"),

            sustain: FloatParam::new(
                "Sustain",
                defaults.sustain_s,
                FloatRange::Linear { min: bass::SUSTAIN_MIN_S, max: bass::SUSTAIN_MAX_S },
            )
            .with_step_size(0.1)
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            slide: FloatParam::new(
                "Slide",
                defaults.slide_ms,
                FloatRange::Linear { min: 0.0, max: bass::SLIDE_MAX_MS },
            )
            .with_step_size(1.0)
            .with_unit(" ms"),

            transpose: FloatParam::new(
                "Transpose",
                defaults.transpose,
                FloatRange::Linear { min: -bass::TRANSPOSE_RANGE, max: bass::TRANSPOSE_RANGE },
            )
            .with_step_size(1.0)
            .with_unit(" st"),
        }
    }
}

/// 0.0-1.0 parameter shown as a percentage
fn percent_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl BassSynthParams {
    fn settings(&self) -> BassSettings {
        BassSettings {
            voice: self.voice.value(),
            volume_db: util::gain_to_db(self.volume.value()),
            drive: self.drive.value(),
            brightness: self.brightness.value(),
            sub: self.sub.value(),
            tone_hz: self.tone.value(),
            sustain_s: self.sustain.value(),
            slide_ms: self.slide.value(),
            transpose: self.transpose.value(),
        }
    }
}

impl Plugin for BassSynthPlugin {
    const NAME: &'static str = "Hojt Bass Synth";
    const VENDOR: &'static str = suite_common::VENDOR;
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // Controllers too: all notes off lets go of the string, all sound off stops it
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
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
        self.bass.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.bass.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.bass.set(&self.params.settings());

        let mut next_event = context.next_event();

        for (sample_id, mut frame) in buffer.iter_samples().enumerate() {
            // Handle all events that land on this sample
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => self.bass.note_on(note, velocity),
                    NoteEvent::NoteOff { note, .. } => self.bass.note_off(note),
                    NoteEvent::MidiCC { cc, .. } if cc == control_change::ALL_NOTES_OFF => {
                        self.bass.release()
                    }
                    NoteEvent::MidiCC { cc, .. } if cc == control_change::ALL_SOUND_OFF => {
                        self.bass.reset()
                    }
                    _ => {}
                }

                next_event = context.next_event();
            }

            let sample = self.bass.process() * self.params.volume.smoothed.next();
            for channel in frame.iter_mut() {
                *channel = sample;
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for BassSynthPlugin {
    const CLAP_ID: &'static str = "net.hojte.bass-synth";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A plucked string bass, picked or played with a finger");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo];
}

impl Vst3Plugin for BassSynthPlugin {
    // Frozen: DAW projects find the plugin by this ID. Never change it
    const VST3_CLASS_ID: [u8; 16] = *b"HojtBassSynth001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(BassSynthPlugin);
nih_export_vst3!(BassSynthPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_defaults_are_those_of_the_dsp() {
        let (params, defaults) = (BassSynthParams::default().settings(), BassSettings::default());
        assert!((params.volume_db - defaults.volume_db).abs() < 0.001);
        assert_eq!(BassSettings { volume_db: defaults.volume_db, ..params }, defaults);
    }

    #[test]
    fn test_the_settings_of_every_voice_are_within_the_knobs() {
        let params = BassSynthParams::default();
        for voice in Voice::ALL {
            let settings = voice.settings();
            let fits = |param: &FloatParam, value: f32| {
                (param.preview_plain(param.preview_normalized(value)) - value).abs() < 0.001
            };
            assert!(fits(&params.tone, settings.tone_hz), "{:?} tone", voice);
            assert!(fits(&params.sustain, settings.sustain_s), "{:?} sustain", voice);
            assert!(fits(&params.slide, settings.slide_ms), "{:?} slide", voice);
            assert!(fits(&params.drive, settings.drive), "{:?} drive", voice);
        }
    }
}
