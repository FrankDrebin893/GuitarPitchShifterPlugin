use std::f32::consts::TAU;

const START_FREQ_HZ: f32 = 180.0;
const END_FREQ_HZ: f32 = 45.0;
const PITCH_DECAY_MS: f32 = 30.0;
const AMP_DECAY_MS: f32 = 250.0;
// Voice is considered finished below this level (-80 dB)
const SILENCE_THRESHOLD: f32 = 0.0001;

/// Synthesized kick drum: sine oscillator with an exponential pitch sweep and amp decay
pub struct Kick {
    sample_rate: f32,
    phase: f32,
    // Envelopes run from 1.0 towards 0.0
    pitch_env: f32,
    amp_env: f32,
    pitch_coeff: f32,
    amp_coeff: f32,
    level: f32,
    active: bool,
}

impl Kick {
    pub fn new() -> Self {
        let mut kick = Self {
            sample_rate: 44100.0,
            phase: 0.0,
            pitch_env: 0.0,
            amp_env: 0.0,
            pitch_coeff: 0.0,
            amp_coeff: 0.0,
            level: 0.0,
            active: false,
        };
        kick.set_sample_rate(44100.0);
        kick
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.pitch_coeff = decay_coeff(PITCH_DECAY_MS, sample_rate);
        self.amp_coeff = decay_coeff(AMP_DECAY_MS, sample_rate);
    }

    /// Start the voice. Velocity is 0.0-1.0. Retriggers restart phase so every hit sounds the same.
    pub fn trigger(&mut self, velocity: f32) {
        self.level = velocity.clamp(0.0, 1.0);
        self.phase = 0.0;
        self.pitch_env = 1.0;
        self.amp_env = 1.0;
        self.active = true;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.pitch_env = 0.0;
        self.amp_env = 0.0;
        self.active = false;
    }

    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn process_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        let output = (self.phase * TAU).sin() * self.amp_env * self.level;

        let freq = END_FREQ_HZ + (START_FREQ_HZ - END_FREQ_HZ) * self.pitch_env;
        self.phase += freq / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        self.pitch_env *= self.pitch_coeff;
        self.amp_env *= self.amp_coeff;
        if self.amp_env < SILENCE_THRESHOLD {
            self.active = false;
        }

        output
    }
}

/// Per-sample multiplier for an exponential decay with the given time constant
fn decay_coeff(time_ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 44100.0;

    fn render(kick: &mut Kick, num_samples: usize) -> Vec<f32> {
        (0..num_samples).map(|_| kick.process_sample()).collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |max, s| max.max(s.abs()))
    }

    #[test]
    fn test_silent_before_trigger() {
        let mut kick = Kick::new();
        kick.set_sample_rate(SAMPLE_RATE);

        let output = render(&mut kick, 1024);

        assert!(!kick.is_active());
        assert!(output.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_produces_output_after_trigger() {
        let mut kick = Kick::new();
        kick.set_sample_rate(SAMPLE_RATE);

        kick.trigger(1.0);
        let output = render(&mut kick, 1024);

        assert!(kick.is_active());
        assert!(peak(&output) > 0.5, "Kick should be clearly audible, peak: {}", peak(&output));
    }

    #[test]
    fn test_decays_to_silence() {
        let mut kick = Kick::new();
        kick.set_sample_rate(SAMPLE_RATE);

        kick.trigger(1.0);
        // 3 seconds is far longer than the amp decay
        render(&mut kick, SAMPLE_RATE as usize * 3);

        assert!(!kick.is_active());
        assert_eq!(kick.process_sample(), 0.0);
    }

    #[test]
    fn test_output_bounded_and_finite() {
        for &sample_rate in &[22050.0, 44100.0, 48000.0, 96000.0, 192000.0] {
            let mut kick = Kick::new();
            kick.set_sample_rate(sample_rate);

            kick.trigger(1.0);
            let output = render(&mut kick, sample_rate as usize);

            assert!(output.iter().all(|s| s.is_finite()));
            assert!(peak(&output) <= 1.0, "Peak {} exceeds 1.0 at {} Hz", peak(&output), sample_rate);
        }
    }

    #[test]
    fn test_velocity_scales_level() {
        let mut loud = Kick::new();
        let mut quiet = Kick::new();

        loud.trigger(1.0);
        quiet.trigger(0.25);
        let loud_peak = peak(&render(&mut loud, 4096));
        let quiet_peak = peak(&render(&mut quiet, 4096));

        assert!((quiet_peak / loud_peak - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_retrigger_restarts_voice() {
        let mut kick = Kick::new();
        kick.set_sample_rate(SAMPLE_RATE);

        kick.trigger(1.0);
        let first = render(&mut kick, 2048);
        kick.trigger(1.0);
        let second = render(&mut kick, 2048);

        assert_eq!(first, second);
    }

    #[test]
    fn test_reset_silences_voice() {
        let mut kick = Kick::new();
        kick.trigger(1.0);
        render(&mut kick, 100);

        kick.reset();

        assert!(!kick.is_active());
        assert_eq!(kick.process_sample(), 0.0);
    }
}
