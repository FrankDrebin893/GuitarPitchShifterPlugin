use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod bells;
mod editor;
use bells::{BellSettings, GlassBells};

pub struct GlassBellsPlugin {
    params: Arc<GlassBellsParams>,
    bells: GlassBells,
}

#[derive(Params)]
pub struct GlassBellsParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "volume"]
    pub volume: FloatParam,

    #[id = "brightness"]
    pub brightness: FloatParam,

    #[id = "decay"]
    pub decay: FloatParam,

    #[id = "attack"]
    pub attack: FloatParam,

    #[id = "dry"]
    pub dry: FloatParam,

    #[id = "delay_on"]
    pub delay_on: BoolParam,

    #[id = "delay_time"]
    pub delay_time: FloatParam,

    #[id = "delay_feedback"]
    pub delay_feedback: FloatParam,

    #[id = "delay_mix"]
    pub delay_mix: FloatParam,

    #[id = "reverb_on"]
    pub reverb_on: BoolParam,

    #[id = "reverb_decay"]
    pub reverb_decay: FloatParam,

    #[id = "reverb_mix"]
    pub reverb_mix: FloatParam,
}

impl Default for GlassBellsPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(GlassBellsParams::default()),
            bells: GlassBells::new(),
        }
    }
}

impl Default for GlassBellsParams {
    fn default() -> Self {
        // Every default is the one of the DSP
        let defaults = BellSettings::default();
        Self {
            editor_state: editor::default_state(),

            volume: FloatParam::new(
                "Volume",
                util::db_to_gain(bells::VOLUME_DEFAULT_DB),
                FloatRange::Skewed {
                    min: util::db_to_gain(bells::VOLUME_MIN_DB),
                    max: util::db_to_gain(bells::VOLUME_MAX_DB),
                    factor: FloatRange::gain_skew_factor(bells::VOLUME_MIN_DB, bells::VOLUME_MAX_DB),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            brightness: percent_param("Brightness", defaults.brightness, 1.0),

            decay: seconds_param("Decay", defaults.decay_s, bells::DECAY_MIN_S, bells::DECAY_MAX_S),

            attack: FloatParam::new(
                "Attack",
                defaults.attack_ms,
                FloatRange::Skewed {
                    min: bells::ATTACK_MIN_MS,
                    max: bells::ATTACK_MAX_MS,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_step_size(0.1)
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            dry: percent_param("Dry", defaults.dry, 1.0),

            delay_on: BoolParam::new("Delay", defaults.delay_on),

            delay_time: FloatParam::new(
                "Delay Time",
                defaults.delay_ms,
                FloatRange::Skewed {
                    min: bells::DELAY_MIN_MS,
                    max: bells::DELAY_MAX_MS,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(1.0)
            .with_unit(" ms"),

            delay_feedback: percent_param("Delay Feedback", defaults.delay_feedback, bells::FEEDBACK_MAX),
            delay_mix: percent_param("Delay Mix", defaults.delay_mix, 1.0),

            reverb_on: BoolParam::new("Reverb", defaults.reverb_on),

            reverb_decay: seconds_param(
                "Reverb Decay",
                defaults.reverb_decay_s,
                bells::REVERB_DECAY_MIN_S,
                bells::REVERB_DECAY_MAX_S,
            ),
            reverb_mix: percent_param("Reverb Mix", defaults.reverb_mix, 1.0),
        }
    }
}

/// 0.0 to `max` parameter shown as a percentage
fn percent_param(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

/// A time in seconds, with more of the knob for the short ones
fn seconds_param(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(-1.0),
        },
    )
    .with_step_size(0.1)
    .with_unit(" s")
    .with_value_to_string(formatters::v2s_f32_rounded(1))
}

impl GlassBellsParams {
    fn settings(&self) -> BellSettings {
        BellSettings {
            brightness: self.brightness.value(),
            decay_s: self.decay.value(),
            attack_ms: self.attack.value(),
            dry: self.dry.value(),
            delay_on: self.delay_on.value(),
            delay_ms: self.delay_time.value(),
            delay_feedback: self.delay_feedback.value(),
            delay_mix: self.delay_mix.value(),
            reverb_on: self.reverb_on.value(),
            reverb_decay_s: self.reverb_decay.value(),
            reverb_mix: self.reverb_mix.value(),
        }
    }
}

impl Plugin for GlassBellsPlugin {
    const NAME: &'static str = "Hojt Glass Bells";
    const VENDOR: &'static str = suite_common::VENDOR;
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // Controllers too: all sound off stops bells and tails. A bell rings out by itself, so
    // all notes off has nothing to let go of
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
        self.bells.set(&self.params.settings());
        self.bells.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.bells.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.bells.set(&self.params.settings());

        let mut next_event = context.next_event();

        for (sample_id, mut frame) in buffer.iter_samples().enumerate() {
            // Handle all events that land on this sample
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => self.bells.note_on(note, velocity),
                    NoteEvent::MidiCC { cc, .. } if cc == control_change::ALL_SOUND_OFF => self.bells.reset(),
                    _ => {}
                }

                next_event = context.next_event();
            }

            let gain = self.params.volume.smoothed.next();
            let (left, right) = self.bells.process();

            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = if channel == 0 { left } else { right } * gain;
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for GlassBellsPlugin {
    const CLAP_ID: &'static str = "net.hojte.glass-bells";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Soft glassy bells with a ping-pong delay and a hall");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for GlassBellsPlugin {
    // Frozen: DAW projects find the plugin by this ID. Never change it
    const VST3_CLASS_ID: [u8; 16] = *b"HojtGlassBells01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(GlassBellsPlugin);
nih_export_vst3!(GlassBellsPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_defaults_are_those_of_the_dsp() {
        assert_eq!(GlassBellsParams::default().settings(), BellSettings::default());
    }
}
