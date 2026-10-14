use std::f32::consts::{PI, TAU};

use super::Hit;
use crate::dsp::{decay_coeff, Noise, Svf};

// A multiple of `BATCH`
pub const MAX_MODES: usize = 256;
// Modes are computed this many at a time, so the loop compiles to vector instructions
const BATCH: usize = 8;

// A full-velocity stroke excites modes up to about this frequency, a soft one far less:
// the softer the stroke, the longer and duller the contact between stick and metal
const STRIKE_CUTOFF_MIN_HZ: f32 = 4000.0;
const STRIKE_CUTOFF_RANGE_HZ: f32 = 16000.0;
// Random level difference per mode from hit to hit
const MODE_VARIATION: f32 = 0.15;
// Soft hits have a darker wash: share of the wash's top that the softest hit keeps
const WASH_SOFT_TOP: f32 = 0.2;
// The wash is noise through this many bands, spread evenly in pitch from `wash_hp_hz` to
// `wash_lp_hz`. Each band has its own decay, so the sizzle darkens as it dies away
const WASH_BANDS: usize = 6;
const WASH_BAND_Q: f32 = 1.0;
// How much shorter the top band rings than the bottom one
const WASH_TOP_DECAY: f32 = 0.35;
// Higher bands are a little louder per octave: the bands are as wide as they are high
const WASH_TILT: f32 = 0.5;
// Brings the sum of the bands to the level the wash had as one band
const WASH_NORM: f32 = 0.25;
// The voice is switched off after this many time constants (-60 dB)
const TAIL_TIME_CONSTANTS: f32 = 7.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CymbalPreset {
    /// Number of resonant modes, at most `MAX_MODES`, spread evenly from `low_hz` to
    /// `high_hz`: the body and the pitch of the cymbal. A mode rings for a second or so, a
    /// bandwidth of a hertz, so modes never fuse into noise however close they are: up high
    /// they are heard as separate tones that beat (a bell, a chorus). The sizzle above
    /// `high_hz` is the wash, which is noise.
    pub modes: usize,
    pub low_hz: f32,
    pub high_hz: f32,
    /// Modes below this are quieter the lower they are: the hum under the cymbal
    pub body_hz: f32,
    /// Level of the highest mode relative to the lowest
    pub tilt: f32,
    pub decay_low_ms: f32,
    pub decay_high_ms: f32,
    pub mode_level: f32,
    /// Sustained noise: the sizzle that carries everything above the modes
    pub wash_level: f32,
    /// Bottom of the noise
    pub wash_hp_hz: f32,
    /// Top of the noise at full velocity, soft hits are darker
    pub wash_lp_hz: f32,
    /// Decay of the lowest band; higher bands die faster
    pub wash_decay_ms: f32,
    /// Short noise burst from the stick
    pub attack_level: f32,
    pub attack_ms: f32,
}

impl CymbalPreset {
    /// A cymbal between `self` (at 0.0) and `other` (at 1.0), which must have the same modes
    /// and range, as the three sounds of a hi-hat do: times on a log scale, levels straight.
    /// The ends are `self` and `other` exactly
    pub fn between(&self, other: &Self, share: f32) -> Self {
        if share <= 0.0 {
            return *self;
        }
        if share >= 1.0 {
            return *other;
        }
        let time = |from: f32, to: f32| from * (to / from).powf(share);
        let level = |from: f32, to: f32| from + (to - from) * share;
        Self {
            decay_low_ms: time(self.decay_low_ms, other.decay_low_ms),
            decay_high_ms: time(self.decay_high_ms, other.decay_high_ms),
            wash_decay_ms: time(self.wash_decay_ms, other.wash_decay_ms),
            attack_ms: time(self.attack_ms, other.attack_ms),
            mode_level: level(self.mode_level, other.mode_level),
            wash_level: level(self.wash_level, other.wash_level),
            attack_level: level(self.attack_level, other.attack_level),
            tilt: level(self.tilt, other.tilt),
            body_hz: time(self.body_hz, other.body_hz),
            wash_hp_hz: time(self.wash_hp_hz, other.wash_hp_hz),
            wash_lp_hz: time(self.wash_lp_hz, other.wash_lp_hz),
            ..*self
        }
    }
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
    // The last two outputs of every mode. `newest` says which of the two is y[n-1]: each
    // sample overwrites the older one, so nothing is copied.
    state: [[f32; MAX_MODES]; 2],
    newest: usize,
    modes: usize,
    mode_scale: f32,
    wash_bands: [Svf; WASH_BANDS],
    wash_gain: [f32; WASH_BANDS],
    // Envelopes run towards 0.0
    wash_env: [f32; WASH_BANDS],
    wash_coeff: [f32; WASH_BANDS],
    attack_env: f32,
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
            state: [[0.0; MAX_MODES]; 2],
            newest: 0,
            modes: 0,
            mode_scale: 0.0,
            wash_bands: std::array::from_fn(|_| Svf::new()),
            wash_gain: [0.0; WASH_BANDS],
            wash_env: [0.0; WASH_BANDS],
            wash_coeff: [0.0; WASH_BANDS],
            attack_env: 0.0,
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
        self.state = [[0.0; MAX_MODES]; 2];
        for band in &mut self.wash_bands {
            band.reset();
        }
        self.wash_env = [0.0; WASH_BANDS];
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
            self.state = [[0.0; MAX_MODES]; 2];
            self.modes = modes;
        }
        self.mode_scale = preset.mode_level / (modes as f32).sqrt();

        let strike_cutoff =
            STRIKE_CUTOFF_MIN_HZ + STRIKE_CUTOFF_RANGE_HZ * hit.velocity * hit.velocity;

        for i in 0..modes {
            // Where in its slot a mode sits is new for every hit on a cymbal at rest, so no two
            // strokes ring the same tones. A hit on one that still rings keeps its tuning, or
            // the tail left from before would be retuned in the middle of its ring
            if self.remaining == 0 {
                self.jitter[i] = self.noise.next() * 0.5 + 0.5;
            }
            let (position, freq, omega) = self.tune_mode(i, preset, hit.decay);

            // Modes that would alias are left out
            if freq > sample_rate * 0.45 {
                continue;
            }
            let reach = 1.0 / (1.0 + (freq / strike_cutoff).powi(2)).sqrt();
            let body = freq / (freq * freq + preset.body_hz * preset.body_hz).sqrt();
            let level = amplitude
                * self.weight[i]
                * (1.0 + (preset.tilt - 1.0) * position)
                * reach
                * body
                * (1.0 + MODE_VARIATION * self.noise.next());
            let phase = self.noise.next() * PI;
            self.state[self.newest][i] += level * phase.sin();
            self.state[1 - self.newest][i] += level * (phase - omega).sin();
        }

        self.set_wash_decays(preset, hit.decay);
        let wash_top = preset.wash_lp_hz
            * (WASH_SOFT_TOP + (1.0 - WASH_SOFT_TOP) * hit.velocity * hit.velocity);
        let span = preset.wash_lp_hz / preset.wash_hp_hz;
        for b in 0..WASH_BANDS {
            let position = (b as f32 + 0.5) / WASH_BANDS as f32;
            let centre = preset.wash_hp_hz * span.powf(position);
            self.wash_bands[b].set(centre, WASH_BAND_Q, sample_rate);
            // Bands that would alias are left out; the rest are rounded off above the top
            self.wash_gain[b] = if centre > sample_rate * 0.45 {
                0.0
            } else {
                let reach = 1.0 / (1.0 + (centre / wash_top).powi(2)).sqrt();
                WASH_NORM * reach * (centre / preset.wash_hp_hz).powf(WASH_TILT) / WASH_BAND_Q
            };
            self.wash_env[b] = self.wash_env[b].max(amplitude);
        }
        self.wash_level = preset.wash_level;
        self.attack_level = preset.attack_level;
        self.attack_env = self.attack_env.max(amplitude * hit.velocity);
        self.remaining = self.tail_samples(preset, hit.decay);
    }

    /// Damps what rings to the decays of `preset`, without a new hit: the hi-hat's pedal
    /// closing on a cymbal that rings open. `preset` has the modes and range it was hit with
    pub fn choke(&mut self, preset: &CymbalPreset, decay: f32) {
        if self.remaining == 0 || preset.modes.min(MAX_MODES) != self.modes {
            return;
        }
        for i in 0..self.modes {
            self.tune_mode(i, preset, decay);
        }
        self.set_wash_decays(preset, decay);
        self.remaining = self.remaining.min(self.tail_samples(preset, decay));
    }

    /// Frequency and decay of one mode, from its place in the range. Returns the place, the
    /// frequency and the frequency in radians per sample
    fn tune_mode(&mut self, i: usize, preset: &CymbalPreset, decay: f32) -> (f32, f32, f32) {
        let sample_rate = self.sample_rate;
        let position = (i as f32 + self.jitter[i]) / self.modes as f32;
        let freq = preset.low_hz + (preset.high_hz - preset.low_hz) * position;
        let decay_ms = preset.decay_low_ms
            * (preset.decay_high_ms / preset.decay_low_ms).powf(position)
            * decay;
        let radius = decay_coeff(decay_ms, sample_rate);
        let omega = TAU * freq / sample_rate;

        self.a1[i] = 2.0 * radius * omega.cos();
        self.a2[i] = radius * radius;
        (position, freq, omega)
    }

    fn set_wash_decays(&mut self, preset: &CymbalPreset, decay: f32) {
        for b in 0..WASH_BANDS {
            let decay_ms = preset.wash_decay_ms
                * WASH_TOP_DECAY.powf(b as f32 / (WASH_BANDS - 1) as f32)
                * decay;
            self.wash_coeff[b] = decay_coeff(decay_ms, self.sample_rate);
        }
        self.attack_coeff = decay_coeff(preset.attack_ms, self.sample_rate);
    }

    fn tail_samples(&self, preset: &CymbalPreset, decay: f32) -> usize {
        let longest_ms =
            preset.decay_low_ms.max(preset.decay_high_ms).max(preset.wash_decay_ms) * decay;
        (longest_ms * 0.001 * TAIL_TIME_CONSTANTS * self.sample_rate) as usize + 1
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

        // Modes past `self.modes` in the last batch are at rest and stay there
        let count = self.modes.next_multiple_of(BATCH);
        let [first, second] = &mut self.state;
        let (previous, next) = if self.newest == 0 { (&*first, second) } else { (&*second, first) };
        let mut sums = [0.0; BATCH];
        let batches = next[..count]
            .chunks_exact_mut(BATCH)
            .zip(previous[..count].chunks_exact(BATCH))
            .zip(self.a1[..count].chunks_exact(BATCH).zip(self.a2[..count].chunks_exact(BATCH)));
        for ((next, previous), (a1, a2)) in batches {
            for k in 0..BATCH {
                next[k] = a1[k] * previous[k] - a2[k] * next[k];
                sums[k] += next[k];
            }
        }
        self.newest = 1 - self.newest;
        let ringing: f32 = sums.iter().sum();

        let noise = self.noise.next();
        let mut air = 0.0;
        let mut wash = 0.0;
        for b in 0..WASH_BANDS {
            let band = self.wash_bands[b].process(noise).band * self.wash_gain[b];
            air += band;
            wash += band * self.wash_env[b];
            self.wash_env[b] *= self.wash_coeff[b];
        }
        // The stick's click is all the bands at once
        let wash = wash * self.wash_level + air * self.attack_env * self.attack_level;
        self.attack_env *= self.attack_coeff;

        ringing * self.mode_scale + wash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Kit, KitPreset};
    use crate::test_util::{brightness, peak, rms, spectrum_fill_by};

    const SAMPLE_RATE: f32 = 44100.0;

    fn hit(velocity: f32) -> Hit {
        Hit { velocity, tune: 1.0, decay: 1.0 }
    }

    fn render(cymbal: &mut Cymbal, num_samples: usize) -> Vec<f32> {
        (0..num_samples).map(|_| cymbal.process_sample()).collect()
    }

    #[test]
    fn test_sounds_and_decays_to_silence() {
        let preset = KitPreset::new(Kit::Rock).unwrap().cymbals.crash1;
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
        let preset = KitPreset::new(Kit::Rock).unwrap().cymbals.ride;
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
    fn test_top_is_noise_and_not_a_comb_of_tones() {
        // Resonators ring for a second at a bandwidth of a hertz: tens of them in a few kHz are
        // heard as a bell that beats. Looked at 15 Hz at a time, noise fills nearly every slot
        // and a comb of tones (0.03 to 0.2 before the wash carried the top) leaves most empty
        let set = KitPreset::new(Kit::Rock).unwrap().cymbals;
        for (name, preset) in [
            ("crash", set.crash1),
            ("ride", set.ride),
            ("open hat", set.hat_open),
            ("china", set.china),
        ] {
            let mut cymbal = Cymbal::new(1);
            cymbal.trigger(&preset, &hit(1.0));
            let output = render(&mut cymbal, SAMPLE_RATE as usize / 2);
            let fill = spectrum_fill_by(&output[4410..17640], SAMPLE_RATE, 5000.0, 7000.0, 15.0);
            assert!(fill > 0.55, "{} fill {}", name, fill);
        }
    }

    #[test]
    fn test_modes_move_from_hit_to_hit() {
        let preset = KitPreset::new(Kit::Rock).unwrap().cymbals.crash1;
        let mut cymbal = Cymbal::new(1);
        cymbal.trigger(&preset, &hit(1.0));
        let first = cymbal.a1;
        cymbal.reset();
        cymbal.trigger(&preset, &hit(1.0));
        assert!(first[..preset.modes] != cymbal.a1[..preset.modes]);
    }

    #[test]
    fn test_second_hit_adds_to_the_ringing() {
        let preset = KitPreset::new(Kit::Rock).unwrap().cymbals.ride;
        let mut cymbal = Cymbal::new(1);

        cymbal.trigger(&preset, &hit(0.8));
        let first = render(&mut cymbal, 8820);
        cymbal.trigger(&preset, &hit(0.8));
        let second = render(&mut cymbal, 8820);

        assert!(rms(&second) > rms(&first) * 1.05, "{} vs {}", rms(&second), rms(&first));
    }

    #[test]
    fn test_level_is_independent_of_sample_rate() {
        let preset = KitPreset::new(Kit::Rock).unwrap().cymbals.ride;
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
