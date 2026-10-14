use super::{Hit, Voice};
use crate::dsp::{decay_coeff, semitones_to_ratio, soft_clip, Modes, Noise, OnePole, Svf, SILENCE_THRESHOLD};

const MODES: usize = 6;
// Vibration modes of a circular membrane, relative to the fundamental
const MODE_RATIOS: [f32; MODES] = [1.0, 1.59, 2.14, 2.30, 2.65, 2.92];
const MODE_LEVELS: [f32; MODES] = [1.0, 0.6, 0.45, 0.35, 0.25, 0.2];

// Side stick: the stick on the rim, a short woody click
const STICK_FREQS_HZ: [f32; MODES] = [430.0, 1090.0, 1810.0, 2700.0, 0.0, 0.0];
const STICK_LEVELS: [f32; MODES] = [0.6, 1.0, 0.6, 0.3, 0.0, 0.0];
const STICK_DECAYS_MS: [f32; MODES] = [18.0, 12.0, 8.0, 5.0, 1.0, 1.0];

const BEND_MS: f32 = 12.0;
const ATTACK_MS: f32 = 6.0;
const SIDE_STICK_ATTACK_MS: f32 = 3.0;
// Soft hits have darker wires: share of the wires' top that the softest hit keeps
const WIRE_SOFT_TOP: f32 = 0.5;
const WIRE_Q: f32 = 0.6;
// Makes up for the level the band-pass takes out of the noise
const WIRE_GAIN: f32 = 2.5;

pub struct SnarePreset {
    pub freq_hz: f32,
    /// Pitch drop after the hit, at full velocity
    pub bend_semitones: f32,
    pub body_decay_ms: f32,
    pub body_level: f32,
    /// Level of the overtones: low is fat and damped, high rings
    pub ring: f32,
    pub wire_level: f32,
    pub wire_decay_ms: f32,
    /// Centre of the wire noise
    pub wire_hz: f32,
    /// Top of the wire noise at full velocity
    pub wire_lp_hz: f32,
    /// Stick transient
    pub attack_level: f32,
    /// Saturation of head and wires together, 1.0 is nearly clean
    pub drive: f32,
}

/// Synthesized snare drum: membrane modes plus snare wires (filtered noise) and a stick transient
pub struct Snare {
    sample_rate: f32,
    noise: Noise,
    modes: Modes<MODES>,
    wire_filter: Svf,
    wire_lp: OnePole,
    bend: f32,
    // Envelopes run from 1.0 towards 0.0
    pitch_env: f32,
    wire_env: f32,
    attack_env: f32,
    pitch_coeff: f32,
    wire_coeff: f32,
    attack_coeff: f32,
    level: f32,
    body_level: f32,
    wire_level: f32,
    attack_level: f32,
    drive: f32,
    drive_norm: f32,
    // The side stick is clean, and its click only the noise above the wire frequency
    side_stick: bool,
}

impl Snare {
    pub fn new(seed: u32) -> Self {
        Self {
            sample_rate: 44100.0,
            noise: Noise::new(seed),
            modes: Modes::new(),
            wire_filter: Svf::new(),
            wire_lp: OnePole::new(),
            bend: 0.0,
            pitch_env: 0.0,
            wire_env: 0.0,
            attack_env: 0.0,
            pitch_coeff: 0.0,
            wire_coeff: 0.0,
            attack_coeff: 0.0,
            level: 0.0,
            body_level: 0.0,
            wire_level: 0.0,
            attack_level: 0.0,
            drive: 1.0,
            drive_norm: 1.0,
            side_stick: false,
        }
    }

    pub fn trigger(&mut self, preset: &SnarePreset, hit: &Hit) {
        let sample_rate = self.sample_rate;
        let freq = preset.freq_hz * hit.tune * (1.0 + 0.006 * self.noise.next());

        for i in 0..MODES {
            // Harder hits excite the overtones more
            let level = if i == 0 {
                1.0
            } else {
                MODE_LEVELS[i] * preset.ring * (0.5 + 0.5 * hit.velocity) * (1.0 + 0.2 * self.noise.next())
            };
            let decay_ms = preset.body_decay_ms * hit.decay / MODE_RATIOS[i];
            self.modes.set(i, freq * MODE_RATIOS[i], level, decay_ms, sample_rate);
        }

        self.bend = semitones_to_ratio(preset.bend_semitones * hit.velocity) - 1.0;
        self.wire_filter.set(preset.wire_hz, WIRE_Q, sample_rate);
        // Soft hits are darker
        self.wire_lp
            .set(preset.wire_lp_hz * (WIRE_SOFT_TOP + (1.0 - WIRE_SOFT_TOP) * hit.velocity), sample_rate);
        self.wire_coeff = decay_coeff(preset.wire_decay_ms * hit.decay, sample_rate);

        self.body_level = preset.body_level;
        // Harder hits rattle the wires more against the head
        self.wire_level =
            preset.wire_level * WIRE_GAIN * (0.6 + 0.4 * hit.velocity) * (1.0 + 0.1 * self.noise.next());
        self.attack_level = preset.attack_level * (0.4 + 0.6 * hit.velocity);
        self.drive = preset.drive;
        self.drive_norm = 1.0 / preset.drive.tanh();
        self.side_stick = false;
        self.strike(hit, ATTACK_MS);
    }

    pub fn trigger_side_stick(&mut self, hit: &Hit) {
        let sample_rate = self.sample_rate;
        let detune = 1.0 + 0.01 * self.noise.next();

        for i in 0..MODES {
            self.modes
                .set(i, STICK_FREQS_HZ[i] * detune, STICK_LEVELS[i], STICK_DECAYS_MS[i], sample_rate);
        }

        self.bend = 0.0;
        self.wire_filter.set(2500.0, WIRE_Q, sample_rate);
        self.wire_lp.set(9000.0, sample_rate);
        self.wire_coeff = 0.0;

        self.body_level = 0.4;
        self.wire_level = 0.0;
        self.attack_level = 0.4;
        self.side_stick = true;
        self.strike(hit, SIDE_STICK_ATTACK_MS);
    }

    fn strike(&mut self, hit: &Hit, attack_ms: f32) {
        self.pitch_coeff = decay_coeff(BEND_MS, self.sample_rate);
        self.attack_coeff = decay_coeff(attack_ms, self.sample_rate);
        self.level = hit.amplitude();

        self.modes.strike();
        self.pitch_env = 1.0;
        self.wire_env = 1.0;
        self.attack_env = 1.0;
    }
}

impl Voice for Snare {
    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn reset(&mut self) {
        self.modes.reset();
        self.wire_filter.reset();
        self.wire_lp.reset();
        self.wire_env = 0.0;
        self.attack_env = 0.0;
    }

    fn is_active(&self) -> bool {
        self.modes.is_active()
            || self.wire_env * self.wire_level > SILENCE_THRESHOLD
            || self.attack_env > SILENCE_THRESHOLD
    }

    fn process_sample(&mut self) -> f32 {
        if !self.is_active() {
            return 0.0;
        }

        let body = self.modes.process(1.0 + self.bend * self.pitch_env);
        // The band around the wire frequency is the wires, the noise as it is the stick
        let noise = self.noise.next();
        let split = self.wire_filter.process(noise);
        let wires = self.wire_lp.process(split.band) * self.wire_env * self.wire_level;
        let stick = if self.side_stick { split.high } else { noise };
        let attack = stick * self.attack_env * self.attack_level;

        self.pitch_env *= self.pitch_coeff;
        self.wire_env *= self.wire_coeff;
        self.attack_env *= self.attack_coeff;

        // Head and wires through one saturation, so the wires belong to the drum. It comes
        // before the level: a soft hit is as dense as a hard one, only quieter.
        let drum = body * self.body_level + wires;
        if self.side_stick {
            return (drum + attack) * self.level;
        }
        (soft_clip(drum * self.drive) * self.drive_norm + attack) * self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Kit, KitPreset};
    use crate::test_util::{brightness, decay_time_s, peak};

    const SAMPLE_RATE: f32 = 44100.0;

    fn hit(velocity: f32) -> Hit {
        Hit { velocity, tune: 1.0, decay: 1.0 }
    }

    fn render(snare: &mut Snare, num_samples: usize) -> Vec<f32> {
        (0..num_samples).map(|_| snare.process_sample()).collect()
    }

    #[test]
    fn test_sounds_and_decays_to_silence() {
        let preset = KitPreset::new(Kit::Rock).unwrap().snare;
        let mut snare = Snare::new(1);
        assert_eq!(snare.process_sample(), 0.0);

        snare.trigger(&preset, &hit(1.0));
        let output = render(&mut snare, SAMPLE_RATE as usize * 3);

        assert!(peak(&output) > 0.3, "Snare peak: {}", peak(&output));
        assert!(!snare.is_active());
        assert_eq!(snare.process_sample(), 0.0);
    }

    #[test]
    fn test_harder_hits_are_brighter() {
        let preset = KitPreset::new(Kit::Rock).unwrap().snare;
        let mut snare = Snare::new(1);

        snare.trigger(&preset, &hit(0.3));
        let soft = render(&mut snare, SAMPLE_RATE as usize);
        snare.trigger(&preset, &hit(1.0));
        let hard = render(&mut snare, SAMPLE_RATE as usize);

        assert!(peak(&hard) > peak(&soft) * 3.0);
        assert!(brightness(&hard) > brightness(&soft) * 1.2);
    }

    #[test]
    fn test_side_stick_is_short() {
        let preset = KitPreset::new(Kit::Rock).unwrap().snare;
        let mut snare = Snare::new(1);

        snare.trigger(&preset, &hit(1.0));
        let full = render(&mut snare, SAMPLE_RATE as usize);
        snare.trigger_side_stick(&hit(1.0));
        let stick = render(&mut snare, SAMPLE_RATE as usize);

        assert!(peak(&stick) > 0.1);
        assert!(decay_time_s(&stick, SAMPLE_RATE, -40.0) < decay_time_s(&full, SAMPLE_RATE, -40.0) * 0.5);
    }
}
