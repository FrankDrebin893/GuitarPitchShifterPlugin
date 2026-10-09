use super::{Hit, Voice};
use crate::dsp::{decay_coeff, semitones_to_ratio, Modes, Noise, Svf, SILENCE_THRESHOLD};

pub const TOM_COUNT: usize = 6;

const MODES: usize = 4;
// Vibration modes of a circular membrane, relative to the fundamental
const MODE_RATIOS: [f32; MODES] = [1.0, 1.59, 2.14, 2.65];
const MODE_LEVELS: [f32; MODES] = [1.0, 0.5, 0.3, 0.2];

const BEND_MS: f32 = 35.0;
const ATTACK_MS: f32 = 5.0;
// Centre of the stick attack noise: higher for smaller toms
const ATTACK_BASE_HZ: f32 = 1200.0;
const ATTACK_FREQ_RATIO: f32 = 8.0;
const ATTACK_Q: f32 = 0.8;
// Makes up for the level the band-pass takes out of the noise
const ATTACK_GAIN: f32 = 3.0;

pub struct TomPreset {
    /// Lowest (floor tom) to highest
    pub freqs_hz: [f32; TOM_COUNT],
    /// Pitch drop after the hit, at full velocity
    pub bend_semitones: f32,
    /// Decay of the lowest tom, smaller toms are shorter
    pub decay_ms: f32,
    /// Level of the overtones
    pub ring: f32,
    /// Stick transient
    pub attack_level: f32,
}

/// Synthesized tom: membrane modes with a pitch bend and a stick transient
pub struct Tom {
    sample_rate: f32,
    noise: Noise,
    modes: Modes<MODES>,
    attack_filter: Svf,
    bend: f32,
    // Envelopes run from 1.0 towards 0.0
    pitch_env: f32,
    attack_env: f32,
    pitch_coeff: f32,
    attack_coeff: f32,
    level: f32,
    attack_level: f32,
}

impl Tom {
    pub fn new(seed: u32) -> Self {
        Self {
            sample_rate: 44100.0,
            noise: Noise::new(seed),
            modes: Modes::new(),
            attack_filter: Svf::new(),
            bend: 0.0,
            pitch_env: 0.0,
            attack_env: 0.0,
            pitch_coeff: 0.0,
            attack_coeff: 0.0,
            level: 0.0,
            attack_level: 0.0,
        }
    }

    /// `index` selects the tom, 0 is the lowest
    pub fn trigger(&mut self, preset: &TomPreset, index: usize, hit: &Hit) {
        let sample_rate = self.sample_rate;
        let base_freq = preset.freqs_hz[index];
        let freq = base_freq * hit.tune * (1.0 + 0.005 * self.noise.next());
        let decay_ms = preset.decay_ms * (preset.freqs_hz[0] / base_freq).sqrt() * hit.decay;

        for i in 0..MODES {
            let level = if i == 0 {
                1.0
            } else {
                MODE_LEVELS[i] * preset.ring * (0.5 + 0.5 * hit.velocity) * (1.0 + 0.2 * self.noise.next())
            };
            self.modes
                .set(i, freq * MODE_RATIOS[i], level, decay_ms / MODE_RATIOS[i], sample_rate);
        }

        self.bend = semitones_to_ratio(preset.bend_semitones * hit.velocity) - 1.0;
        self.pitch_coeff = decay_coeff(BEND_MS, sample_rate);
        self.attack_coeff = decay_coeff(ATTACK_MS, sample_rate);
        self.attack_filter
            .set(ATTACK_BASE_HZ + freq * ATTACK_FREQ_RATIO, ATTACK_Q, sample_rate);
        self.attack_filter.reset();

        self.level = hit.amplitude();
        self.attack_level = preset.attack_level * ATTACK_GAIN * (0.4 + 0.6 * hit.velocity);

        self.modes.strike();
        self.pitch_env = 1.0;
        self.attack_env = 1.0;
    }
}

impl Voice for Tom {
    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn reset(&mut self) {
        self.modes.reset();
        self.attack_filter.reset();
        self.attack_env = 0.0;
    }

    fn is_active(&self) -> bool {
        self.modes.is_active() || self.attack_env > SILENCE_THRESHOLD
    }

    fn process_sample(&mut self) -> f32 {
        if !self.is_active() {
            return 0.0;
        }

        let body = self.modes.process(1.0 + self.bend * self.pitch_env);
        let attack = self.attack_filter.process(self.noise.next()).band * self.attack_env * self.attack_level;

        self.pitch_env *= self.pitch_coeff;
        self.attack_env *= self.attack_coeff;

        (body + attack) * self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Kit, KitPreset};
    use crate::test_util::{decay_time_s, peak, zero_crossing_freq};

    const SAMPLE_RATE: f32 = 44100.0;

    fn hit(velocity: f32) -> Hit {
        Hit { velocity, tune: 1.0, decay: 1.0 }
    }

    fn render(tom: &mut Tom, num_samples: usize) -> Vec<f32> {
        (0..num_samples).map(|_| tom.process_sample()).collect()
    }

    #[test]
    fn test_sounds_and_decays_to_silence() {
        let preset = KitPreset::new(Kit::Rock).toms;
        let mut tom = Tom::new(1);
        assert_eq!(tom.process_sample(), 0.0);

        tom.trigger(&preset, 0, &hit(1.0));
        let output = render(&mut tom, SAMPLE_RATE as usize * 4);

        assert!(peak(&output) > 0.3, "Tom peak: {}", peak(&output));
        assert!(!tom.is_active());
    }

    #[test]
    fn test_toms_are_ordered_low_to_high() {
        let preset = KitPreset::new(Kit::Rock).toms;
        let mut tom = Tom::new(1);
        let mut previous_freq = 0.0;
        let mut previous_decay = f32::MAX;

        for index in 0..TOM_COUNT {
            tom.trigger(&preset, index, &hit(1.0));
            let output = render(&mut tom, SAMPLE_RATE as usize * 4);
            // After the bend has settled
            let freq = zero_crossing_freq(&output[8000..20000], SAMPLE_RATE);
            let decay = decay_time_s(&output, SAMPLE_RATE, -40.0);

            assert!(freq > previous_freq, "Tom {} is not higher: {} Hz", index, freq);
            assert!(decay < previous_decay, "Tom {} does not decay faster: {} s", index, decay);
            previous_freq = freq;
            previous_decay = decay;
        }
    }
}
