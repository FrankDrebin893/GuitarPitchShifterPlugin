use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::Arc;

mod editor;
mod pad;
use pad::{PadSettings, ShimmerPad};

pub struct ShimmerPadPlugin {
    params: Arc<ShimmerPadParams>,
    pad: ShimmerPad,
}

#[derive(Params)]
pub struct ShimmerPadParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "volume"]
    pub volume: FloatParam,

    #[id = "attack"]
    pub attack: FloatParam,

    #[id = "release"]
    pub release: FloatParam,

    #[id = "cutoff"]
    pub cutoff: FloatParam,

    #[id = "detune"]
    pub detune: FloatParam,

    #[id = "breath"]
    pub breath: FloatParam,

    #[id = "motion"]
    pub motion: FloatParam,

    #[id = "dry"]
    pub dry: FloatParam,

    #[id = "chorus_on"]
    pub chorus_on: BoolParam,

    #[id = "chorus"]
    pub chorus: FloatParam,

    #[id = "reverb"]
    pub reverb: FloatParam,

    #[id = "reverb_decay"]
    pub reverb_decay: FloatParam,

    #[id = "shimmer_on"]
    pub shimmer_on: BoolParam,

    #[id = "shimmer"]
    pub shimmer: FloatParam,
}

impl Default for ShimmerPadPlugin {
    fn default() -> Self {
        Self { params: Arc::new(ShimmerPadParams::default()), pad: ShimmerPad::new() }
    }
}

impl Default for ShimmerPadParams {
    fn default() -> Self {
        // Every default is the one of the DSP
        let defaults = PadSettings::default();
        Self {
            editor_state: editor::default_state(),

            volume: FloatParam::new(
                "Volume",
                util::db_to_gain(pad::VOLUME_DEFAULT_DB),
                FloatRange::Skewed {
                    min: util::db_to_gain(pad::VOLUME_MIN_DB),
                    max: util::db_to_gain(pad::VOLUME_MAX_DB),
                    factor: FloatRange::gain_skew_factor(pad::VOLUME_MIN_DB, pad::VOLUME_MAX_DB),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),

            attack: seconds_param(
                "Attack",
                defaults.attack_s,
                pad::ATTACK_MIN_S,
                pad::ATTACK_MAX_S,
            ),
            release: seconds_param(
                "Release",
                defaults.release_s,
                pad::RELEASE_MIN_S,
                pad::RELEASE_MAX_S,
            ),

            cutoff: FloatParam::new(
                "Cutoff",
                defaults.cutoff_hz,
                FloatRange::Skewed {
                    min: pad::CUTOFF_MIN_HZ,
                    max: pad::CUTOFF_MAX_HZ,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_step_size(1.0)
            .with_unit(" Hz"),

            detune: FloatParam::new(
                "Detune",
                defaults.detune_cents,
                FloatRange::Linear { min: 0.0, max: pad::DETUNE_MAX_CENTS },
            )
            .with_step_size(0.1)
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            breath: percent_param("Breath", defaults.breath, 1.0),
            motion: percent_param("Motion", defaults.motion, 1.0),
            dry: percent_param("Dry", defaults.dry, 1.0),

            chorus_on: BoolParam::new("Chorus", defaults.chorus_on),
            chorus: percent_param("Chorus Depth", defaults.chorus, 1.0),

            reverb: percent_param("Reverb", defaults.reverb, 1.0),
            reverb_decay: seconds_param(
                "Reverb Decay",
                defaults.reverb_decay_s,
                pad::REVERB_DECAY_MIN_S,
                pad::REVERB_DECAY_MAX_S,
            ),

            shimmer_on: BoolParam::new("Shimmer", defaults.shimmer_on),
            shimmer: percent_param("Shimmer Level", defaults.shimmer, pad::SHIMMER_MAX),
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
        FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.0) },
    )
    .with_step_size(0.01)
    .with_unit(" s")
    .with_value_to_string(formatters::v2s_f32_rounded(2))
}

impl ShimmerPadParams {
    fn settings(&self) -> PadSettings {
        PadSettings {
            attack_s: self.attack.value(),
            release_s: self.release.value(),
            cutoff_hz: self.cutoff.value(),
            detune_cents: self.detune.value(),
            breath: self.breath.value(),
            motion: self.motion.value(),
            dry: self.dry.value(),
            chorus_on: self.chorus_on.value(),
            chorus: self.chorus.value(),
            reverb: self.reverb.value(),
            reverb_decay_s: self.reverb_decay.value(),
            shimmer_on: self.shimmer_on.value(),
            shimmer: self.shimmer.value(),
        }
    }
}

impl Plugin for ShimmerPadPlugin {
    const NAME: &'static str = "Hojt Shimmer Pad";
    const VENDOR: &'static str = suite_common::VENDOR;
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // Controllers too: all notes off lets go of every key, all sound off stops voices and tails
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
        self.pad.set(&self.params.settings());
        self.pad.set_sample_rate(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.pad.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.pad.set(&self.params.settings());

        let mut next_event = context.next_event();

        for (sample_id, mut frame) in buffer.iter_samples().enumerate() {
            // Handle all events that land on this sample
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => self.pad.note_on(note, velocity),
                    NoteEvent::NoteOff { note, .. } => self.pad.note_off(note),
                    NoteEvent::MidiCC { cc, .. } if cc == control_change::ALL_NOTES_OFF => {
                        self.pad.release()
                    }
                    NoteEvent::MidiCC { cc, .. } if cc == control_change::ALL_SOUND_OFF => {
                        self.pad.reset()
                    }
                    _ => {}
                }

                next_event = context.next_event();
            }

            let gain = self.params.volume.smoothed.next();
            let (left, right) = self.pad.process();

            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = if channel == 0 { left } else { right } * gain;
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for ShimmerPadPlugin {
    const CLAP_ID: &'static str = "net.hojte.shimmer-pad";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A slow ambient pad with a chorus and a shimmering hall");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo];
}

impl Vst3Plugin for ShimmerPadPlugin {
    // Frozen: DAW projects find the plugin by this ID. Never change it
    const VST3_CLASS_ID: [u8; 16] = *b"HojtShimmerPad01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(ShimmerPadPlugin);
nih_export_vst3!(ShimmerPadPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_defaults_are_those_of_the_dsp() {
        assert_eq!(ShimmerPadParams::default().settings(), PadSettings::default());
    }
}
