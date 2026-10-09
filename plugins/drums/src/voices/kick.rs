use std::f32::consts::TAU;

use super::{Hit, Voice};
use crate::dsp::{decay_coeff, semitones_to_ratio, soft_clip, Noise, Svf, SILENCE_THRESHOLD};

// Second shell mode relative to the fundamental
const SHELL_RATIO: f32 = 1.59;
const CLICK_Q: f32 = 1.2;
// Makes up for the level the band-pass takes out of the noise
const CLICK_GAIN: f32 = 3.0;

pub struct KickPreset {
    /// Pitch the drum settles on
    pub freq_hz: f32,
    /// How far above that the hit starts, at full velocity
    pub sweep_semitones: f32,
    pub sweep_ms: f32,
    pub decay_ms: f32,
    pub shell_level: f32,
    /// Beater attack: band-passed noise burst
    pub click_level: f32,
    pub click_freq_hz: f32,
    pub click_ms: f32,
    /// Saturation of the body, 1.0 is nearly clean
    pub drive: f32,
}

/// Synthesized kick drum: sine body with a pitch sweep, a shell mode and a beater click
pub struct Kick {
    sample_rate: f32,
    noise: Noise,
    click_filter: Svf,
    phase: f32,
    shell_phase: f32,
    freq: f32,
    sweep: f32,
    shell_freq: f32,
    // Envelopes run from 1.0 towards 0.0
    pitch_env: f32,
    amp_env: f32,
    shell_env: f32,
    click_env: f32,
    pitch_coeff: f32,
    amp_coeff: f32,
    shell_coeff: f32,
    click_coeff: f32,
    level: f32,
    shell_level: f32,
    click_level: f32,
    drive: f32,
    drive_norm: f32,
    active: bool,
}

impl Kick {
    pub fn new(seed: u32) -> Self {
        Self {
            sample_rate: 44100.0,
            noise: Noise::new(seed),
            click_filter: Svf::new(),
            phase: 0.0,
            shell_phase: 0.0,
            freq: 0.0,
            sweep: 0.0,
            shell_freq: 0.0,
            pitch_env: 0.0,
            amp_env: 0.0,
            shell_env: 0.0,
            click_env: 0.0,
            pitch_coeff: 0.0,
            amp_coeff: 0.0,
            shell_coeff: 0.0,
            click_coeff: 0.0,
            level: 0.0,
            shell_level: 0.0,
            click_level: 0.0,
            drive: 1.0,
            drive_norm: 1.0,
            active: false,
        }
    }

    pub fn trigger(&mut self, preset: &KickPreset, hit: &Hit) {
        let sample_rate = self.sample_rate;
        let amplitude = hit.amplitude();

        // No drummer hits the same spot twice
        self.freq = preset.freq_hz * hit.tune * (1.0 + 0.004 * self.noise.next());
        // Harder hits stretch the head further, so they start higher
        let start_freq = self.freq * semitones_to_ratio(preset.sweep_semitones * (0.6 + 0.4 * hit.velocity));
        self.sweep = start_freq - self.freq;
        self.shell_freq = self.freq * SHELL_RATIO;

        self.pitch_coeff = decay_coeff(preset.sweep_ms, sample_rate);
        self.amp_coeff = decay_coeff(preset.decay_ms * hit.decay, sample_rate);
        self.shell_coeff = decay_coeff(preset.decay_ms * hit.decay * 0.5, sample_rate);
        self.click_coeff = decay_coeff(preset.click_ms, sample_rate);
        self.click_filter.set(preset.click_freq_hz, CLICK_Q, sample_rate);
        self.click_filter.reset();

        self.level = amplitude;
        self.shell_level = preset.shell_level;
        self.click_level =
            preset.click_level * CLICK_GAIN * amplitude * (0.5 + 0.5 * hit.velocity) * (1.0 + 0.15 * self.noise.next());
        self.drive = preset.drive;
        self.drive_norm = 1.0 / preset.drive.tanh();

        self.phase = 0.0;
        self.shell_phase = 0.0;
        self.pitch_env = 1.0;
        self.amp_env = 1.0;
        self.shell_env = 1.0;
        self.click_env = 1.0;
        self.active = true;
    }
}

impl Voice for Kick {
    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn reset(&mut self) {
        self.amp_env = 0.0;
        self.shell_env = 0.0;
        self.click_env = 0.0;
        self.click_filter.reset();
        self.active = false;
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn process_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        let body = (self.phase * TAU).sin() * self.amp_env
            + (self.shell_phase * TAU).sin() * self.shell_env * self.shell_level;
        let click = self.click_filter.process(self.noise.next()).band * self.click_env * self.click_level;
        let output = soft_clip(body * self.drive * self.level) * self.drive_norm + click;

        let freq = self.freq + self.sweep * self.pitch_env;
        self.phase += freq / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.shell_phase += self.shell_freq / self.sample_rate;
        if self.shell_phase >= 1.0 {
            self.shell_phase -= 1.0;
        }

        self.pitch_env *= self.pitch_coeff;
        self.amp_env *= self.amp_coeff;
        self.shell_env *= self.shell_coeff;
        self.click_env *= self.click_coeff;
        if self.amp_env < SILENCE_THRESHOLD && self.click_env < SILENCE_THRESHOLD {
            self.active = false;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Kit, KitPreset};
    use crate::test_util::{peak, zero_crossing_freq};

    const SAMPLE_RATE: f32 = 44100.0;

    fn hit(velocity: f32) -> Hit {
        Hit { velocity, tune: 1.0, decay: 1.0 }
    }

    fn render(kick: &mut Kick, num_samples: usize) -> Vec<f32> {
        (0..num_samples).map(|_| kick.process_sample()).collect()
    }

    #[test]
    fn test_silent_before_trigger() {
        let mut kick = Kick::new(1);

        let output = render(&mut kick, 1024);

        assert!(!kick.is_active());
        assert!(output.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_decays_to_silence() {
        let preset = KitPreset::new(Kit::Rock).kick;
        let mut kick = Kick::new(1);

        kick.trigger(&preset, &hit(1.0));
        let output = render(&mut kick, SAMPLE_RATE as usize * 3);

        assert!(peak(&output) > 0.5, "Kick should be clearly audible, peak: {}", peak(&output));
        assert!(!kick.is_active());
        assert_eq!(kick.process_sample(), 0.0);
    }

    #[test]
    fn test_settles_on_preset_pitch() {
        let preset = KitPreset::new(Kit::Jazz).kick;
        let mut kick = Kick::new(1);

        kick.trigger(&preset, &hit(1.0));
        let output = render(&mut kick, SAMPLE_RATE as usize);
        // After the sweep and the click, before the tail gets too quiet
        let freq = zero_crossing_freq(&output[6000..16000], SAMPLE_RATE);

        assert!(
            (freq - preset.freq_hz).abs() < preset.freq_hz * 0.1,
            "Expected about {} Hz, got {}",
            preset.freq_hz,
            freq
        );
    }

    #[test]
    fn test_hits_are_not_identical() {
        let preset = KitPreset::new(Kit::Rock).kick;
        let mut kick = Kick::new(1);

        kick.trigger(&preset, &hit(1.0));
        let first = render(&mut kick, 2048);
        kick.trigger(&preset, &hit(1.0));
        let second = render(&mut kick, 2048);

        assert_ne!(first, second);
    }
}
