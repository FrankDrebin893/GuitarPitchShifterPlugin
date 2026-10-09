use std::f32::consts::{PI, TAU};

use super::Hit;
use crate::dsp::{decay_coeff, Noise, OnePole, Svf};

pub const MAX_MODES: usize = 64;

// A full-velocity stroke excites modes up to about this frequency, a soft one far less:
// the softer the stroke, the longer and duller the contact between stick and metal
const STRIKE_CUTOFF_MIN_HZ: f32 = 1500.0;
const STRIKE_CUTOFF_RANGE_HZ: f32 = 16000.0;
// Random level difference per mode from hit to hit
const MODE_VARIATION: f32 = 0.15;
// Mode frequencies thin out towards the top, where the noise wash takes over
const MODE_SPREAD_EXPONENT: f32 = 1.4;
const WASH_Q: f32 = 0.7;
// The voice is switched off after this many time constants (-60 dB)
const TAIL_TIME_CONSTANTS: f32 = 7.0;

pub struct CymbalPreset {
    /// Number of resonant modes, at most `MAX_MODES`. Few modes sound like a bell.
    pub modes: usize,
    pub low_hz: f32,
    pub high_hz: f32,
    /// Level of the highest mode relative to the lowest
    pub tilt: f32,
    pub decay_low_ms: f32,
    pub decay_high_ms: f32,
    pub mode_level: f32,
    /// Sustained noise that fills in between the modes
    pub wash_level: f32,
    pub wash_hp_hz: f32,
    /// Top of the noise at full velocity, soft hits are darker
    pub wash_lp_hz: f32,
    pub wash_decay_ms: f32,
    /// Short noise burst from the stick
    pub attack_level: f32,
    pub attack_ms: f32,
}

/// Synthesized cymbal: a bank of resonators ringing at inharmonic frequencies, plus a
/// band-limited noise wash.
///
/// Hitting it again adds energy to modes that are still ringing, like the real thing, so there
/// is nothing to cut off. Triggering with a preset that has a shorter decay damps the ringing,
/// which is how a closed hi-hat chokes an open one.
pub struct Cymbal {
    sample_rate: f32,
    noise: Noise,
    // Fixed per cymbal: where each mode sits within its slot of the frequency range, and how
    // strongly it responds
    jitter: [f32; MAX_MODES],
    weight: [f32; MAX_MODES],
    // Resonators: y[n] = a1 * y[n-1] - a2 * y[n-2]. A hit adds a sine of random phase to
    // the state, so no two hits are the same and repeated hits build up naturally.
    a1: [f32; MAX_MODES],
    a2: [f32; MAX_MODES],
    y1: [f32; MAX_MODES],
    y2: [f32; MAX_MODES],
    modes: usize,
    mode_scale: f32,
    wash_hp: Svf,
    wash_lp: OnePole,
    // Envelopes run towards 0.0
    wash_env: f32,
    attack_env: f32,
    wash_coeff: f32,
    attack_coeff: f32,
    wash_level: f32,
    attack_level: f32,
    // Samples until the tail is inaudible
    remaining: usize,
}

impl Cymbal {
    pub fn new(seed: u32) -> Self {
        let mut noise = Noise::new(seed);
        let mut jitter = [0.0; MAX_MODES];
        let mut weight = [0.0; MAX_MODES];
        for i in 0..MAX_MODES {
            jitter[i] = noise.next() * 0.5 + 0.5;
            weight[i] = 0.7 + 0.3 * noise.next();
        }

        Self {
            sample_rate: 44100.0,
            noise,
            jitter,
            weight,
            a1: [0.0; MAX_MODES],
            a2: [0.0; MAX_MODES],
            y1: [0.0; MAX_MODES],
            y2: [0.0; MAX_MODES],
            modes: 0,
            mode_scale: 0.0,
            wash_hp: Svf::new(),
            wash_lp: OnePole::new(),
            wash_env: 0.0,
            attack_env: 0.0,
            wash_coeff: 0.0,
            attack_coeff: 0.0,
            wash_level: 0.0,
            attack_level: 0.0,
            remaining: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.y1 = [0.0; MAX_MODES];
        self.y2 = [0.0; MAX_MODES];
        self.wash_hp.reset();
        self.wash_lp.reset();
        self.wash_env = 0.0;
        self.attack_env = 0.0;
        self.remaining = 0;
    }

    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.remaining > 0
    }

    pub fn trigger(&mut self, preset: &CymbalPreset, hit: &Hit) {
        let sample_rate = self.sample_rate;
        let amplitude = hit.amplitude();
        let modes = preset.modes.min(MAX_MODES);

        if modes != self.modes {
            // Another kit's cymbal: the old ringing does not belong to these modes
            self.y1 = [0.0; MAX_MODES];
            self.y2 = [0.0; MAX_MODES];
            self.modes = modes;
        }
        self.mode_scale = preset.mode_level / (modes as f32).sqrt();

        let strike_cutoff = STRIKE_CUTOFF_MIN_HZ + STRIKE_CUTOFF_RANGE_HZ * hit.velocity * hit.velocity;

        for i in 0..modes {
            let position = (i as f32 + self.jitter[i]) / modes as f32;
            let freq = preset.low_hz + (preset.high_hz - preset.low_hz) * position.powf(MODE_SPREAD_EXPONENT);
            let decay_ms =
                preset.decay_low_ms * (preset.decay_high_ms / preset.decay_low_ms).powf(position) * hit.decay;
            let radius = decay_coeff(decay_ms, sample_rate);
            let omega = TAU * freq / sample_rate;

            self.a1[i] = 2.0 * radius * omega.cos();
            self.a2[i] = radius * radius;

            // Modes that would alias are left out
            if freq > sample_rate * 0.45 {
                continue;
            }
            let reach = 1.0 / (1.0 + (freq / strike_cutoff).powi(2)).sqrt();
            let level = amplitude
                * self.weight[i]
                * (1.0 + (preset.tilt - 1.0) * position)
                * reach
                * (1.0 + MODE_VARIATION * self.noise.next());
            let phase = self.noise.next() * PI;
            self.y1[i] += level * phase.sin();
            self.y2[i] += level * (phase - omega).sin();
        }

        self.wash_hp.set(preset.wash_hp_hz, WASH_Q, sample_rate);
        self.wash_lp.set(preset.wash_lp_hz * (0.4 + 0.6 * hit.velocity), sample_rate);
        self.wash_coeff = decay_coeff(preset.wash_decay_ms * hit.decay, sample_rate);
        self.attack_coeff = decay_coeff(preset.attack_ms, sample_rate);
        self.wash_level = preset.wash_level;
        self.attack_level = preset.attack_level;
        self.wash_env = self.wash_env.max(amplitude);
        self.attack_env = self.attack_env.max(amplitude * hit.velocity);

        let longest_ms = preset.decay_low_ms.max(preset.decay_high_ms).max(preset.wash_decay_ms) * hit.decay;
        self.remaining = (longest_ms * 0.001 * TAIL_TIME_CONSTANTS * sample_rate) as usize + 1;
    }

    pub fn process_sample(&mut self) -> f32 {
        if self.remaining == 0 {
            return 0.0;
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            self.reset();
            return 0.0;
        }

        let mut ringing = 0.0;
        for i in 0..self.modes {
            let y = self.a1[i] * self.y1[i] - self.a2[i] * self.y2[i];
            self.y2[i] = self.y1[i];
            self.y1[i] = y;
            ringing += y;
        }

        let hiss = self.wash_lp.process(self.wash_hp.process(self.noise.next()).high);
        let wash = hiss * (self.wash_env * self.wash_level + self.attack_env * self.attack_level);
        self.wash_env *= self.wash_coeff;
        self.attack_env *= self.attack_coeff;

        ringing * self.mode_scale + wash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Kit, KitPreset};
    use crate::test_util::{brightness, peak, rms};

    const SAMPLE_RATE: f32 = 44100.0;

    fn hit(velocity: f32) -> Hit {
        Hit { velocity, tune: 1.0, decay: 1.0 }
    }

    fn render(cymbal: &mut Cymbal, num_samples: usize) -> Vec<f32> {
        (0..num_samples).map(|_| cymbal.process_sample()).collect()
    }

    #[test]
    fn test_sounds_and_decays_to_silence() {
        let preset = KitPreset::new(Kit::Rock).cymbals.crash1;
        let mut cymbal = Cymbal::new(1);
        assert_eq!(cymbal.process_sample(), 0.0);

        cymbal.trigger(&preset, &hit(1.0));
        let output = render(&mut cymbal, SAMPLE_RATE as usize * 10);

        assert!(peak(&output) > 0.1, "Crash peak: {}", peak(&output));
        assert!(!cymbal.is_active());
        assert_eq!(cymbal.process_sample(), 0.0);
    }

    #[test]
    fn test_harder_hits_are_brighter() {
        let preset = KitPreset::new(Kit::Rock).cymbals.ride;
        let mut soft_cymbal = Cymbal::new(1);
        let mut hard_cymbal = Cymbal::new(1);

        soft_cymbal.trigger(&preset, &hit(0.3));
        hard_cymbal.trigger(&preset, &hit(1.0));
        let soft = render(&mut soft_cymbal, SAMPLE_RATE as usize);
        let hard = render(&mut hard_cymbal, SAMPLE_RATE as usize);

        assert!(rms(&hard) > rms(&soft) * 3.0);
        assert!(brightness(&hard) > brightness(&soft) * 1.2);
    }

    #[test]
    fn test_second_hit_adds_to_the_ringing() {
        let preset = KitPreset::new(Kit::Rock).cymbals.ride;
        let mut cymbal = Cymbal::new(1);

        cymbal.trigger(&preset, &hit(0.8));
        let first = render(&mut cymbal, 8820);
        cymbal.trigger(&preset, &hit(0.8));
        let second = render(&mut cymbal, 8820);

        assert!(rms(&second) > rms(&first) * 1.05, "{} vs {}", rms(&second), rms(&first));
    }

    #[test]
    fn test_level_is_independent_of_sample_rate() {
        let preset = KitPreset::new(Kit::Rock).cymbals.ride;
        let mut levels = Vec::new();

        for &sample_rate in &[44100.0, 96000.0] {
            let mut cymbal = Cymbal::new(1);
            cymbal.set_sample_rate(sample_rate);
            cymbal.trigger(&preset, &hit(1.0));
            levels.push(rms(&render(&mut cymbal, (sample_rate * 0.5) as usize)));
        }

        let ratio = levels[1] / levels[0];
        assert!((0.6..1.6).contains(&ratio), "Level changes with sample rate: {:?}", levels);
    }
}
