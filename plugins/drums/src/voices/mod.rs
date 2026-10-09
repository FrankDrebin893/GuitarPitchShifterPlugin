use crate::dsp::{decay_coeff, SILENCE_THRESHOLD};

pub mod cymbal;
pub mod kick;
pub mod snare;
pub mod tom;

// How quickly the previous hit on a drum is damped by the next one
const RETRIGGER_FADE_MS: f32 = 3.0;

/// One strike on a kit piece
pub struct Hit {
    /// 0.0-1.0
    pub velocity: f32,
    /// Frequency ratio from the Tune parameter
    pub tune: f32,
    /// Decay time scale from the Damping parameter
    pub decay: f32,
}

impl Hit {
    /// Velocity to level. Steeper than linear, so soft notes are properly soft.
    pub fn amplitude(&self) -> f32 {
        self.velocity.powf(1.7)
    }
}

pub trait Voice {
    fn set_sample_rate(&mut self, sample_rate: f32);
    fn reset(&mut self);
    fn is_active(&self) -> bool;
    fn process_sample(&mut self) -> f32;
}

/// Two voices for one drum. A new hit takes the idle voice while the previous one fades out
/// quickly, so retriggering never cuts the waveform.
pub struct VoicePair<V> {
    voices: [V; 2],
    fades: [f32; 2],
    current: usize,
    fade_coeff: f32,
}

impl<V: Voice> VoicePair<V> {
    pub fn new(first: V, second: V) -> Self {
        Self {
            voices: [first, second],
            fades: [1.0; 2],
            current: 0,
            fade_coeff: decay_coeff(RETRIGGER_FADE_MS, 44100.0),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.fade_coeff = decay_coeff(RETRIGGER_FADE_MS, sample_rate);
        for voice in &mut self.voices {
            voice.set_sample_rate(sample_rate);
        }
    }

    pub fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
    }

    /// The voice to trigger for a new hit
    pub fn next_voice(&mut self) -> &mut V {
        self.current = 1 - self.current;
        self.fades[self.current] = 1.0;
        &mut self.voices[self.current]
    }

    pub fn process_sample(&mut self) -> f32 {
        let previous = 1 - self.current;
        let mut output = self.voices[self.current].process_sample();

        if self.voices[previous].is_active() {
            self.fades[previous] *= self.fade_coeff;
            if self.fades[previous] < SILENCE_THRESHOLD {
                self.voices[previous].reset();
            } else {
                output += self.voices[previous].process_sample() * self.fades[previous];
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outputs a constant while active, which makes any discontinuity obvious
    struct ConstantVoice {
        active: bool,
    }

    impl Voice for ConstantVoice {
        fn set_sample_rate(&mut self, _sample_rate: f32) {}

        fn reset(&mut self) {
            self.active = false;
        }

        fn is_active(&self) -> bool {
            self.active
        }

        fn process_sample(&mut self) -> f32 {
            if self.active {
                1.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn test_retrigger_fades_previous_voice_without_a_jump() {
        let mut pair = VoicePair::new(ConstantVoice { active: false }, ConstantVoice { active: false });
        pair.set_sample_rate(44100.0);

        pair.next_voice().active = true;
        assert_eq!(pair.process_sample(), 1.0);

        // Second hit: the first voice keeps sounding and fades, it is not cut
        pair.next_voice().active = true;
        let output: Vec<f32> = (0..4410).map(|_| pair.process_sample()).collect();

        assert!(output[0] > 1.9, "Previous voice was cut: {}", output[0]);
        let max_step = output.windows(2).fold(0.0f32, |max, w| max.max((w[1] - w[0]).abs()));
        assert!(max_step < 0.02, "Fade is not smooth, step: {}", max_step);
        assert_eq!(*output.last().unwrap(), 1.0);
    }

    #[test]
    fn test_velocity_curve() {
        let hit = |velocity| Hit { velocity, tune: 1.0, decay: 1.0 };

        assert_eq!(hit(1.0).amplitude(), 1.0);
        assert_eq!(hit(0.0).amplitude(), 0.0);
        assert!(hit(0.5).amplitude() < 0.5);
    }
}
