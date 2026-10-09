use std::f32::consts::{PI, TAU};

// Voices are considered finished below this level (-80 dB)
pub const SILENCE_THRESHOLD: f32 = 0.0001;

/// Per-sample multiplier for an exponential decay with the given time constant
pub fn decay_coeff(time_ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
}

pub fn semitones_to_ratio(semitones: f32) -> f32 {
    2.0f32.powf(semitones / 12.0)
}

pub fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

// Level where the output bus starts to round off peaks
pub const BUS_CLIP_KNEE: f32 = 0.7;

/// Limiter for the output bus: leaves everything below the knee untouched, then bends
/// smoothly towards 1.0
pub fn bus_clip(x: f32) -> f32 {
    let level = x.abs();
    if level <= BUS_CLIP_KNEE {
        x
    } else {
        let range = 1.0 - BUS_CLIP_KNEE;
        (BUS_CLIP_KNEE + range * ((level - BUS_CLIP_KNEE) / range).tanh()).copysign(x)
    }
}

/// White noise in -1.0..1.0 (xorshift). Each voice owns one, so no two hits are identical.
pub struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self {
            state: seed.wrapping_mul(0x9E37_79B9) | 1,
        }
    }

    pub fn next(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state as f32 / 2_147_483_648.0 - 1.0
    }
}

pub struct SvfOutput {
    pub band: f32,
    pub high: f32,
}

/// State-variable filter (trapezoidal), stable at any cutoff below Nyquist
pub struct Svf {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
}

impl Svf {
    pub fn new() -> Self {
        Self {
            k: 1.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1: 0.0,
            ic2: 0.0,
        }
    }

    pub fn set(&mut self, freq_hz: f32, q: f32, sample_rate: f32) {
        let g = (PI * freq_hz.min(sample_rate * 0.45) / sample_rate).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    pub fn process(&mut self, input: f32) -> SvfOutput {
        let v3 = input - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;

        SvfOutput {
            band: v1,
            high: input - self.k * v1 - v2,
        }
    }
}

/// One-pole lowpass
pub struct OnePole {
    coeff: f32,
    state: f32,
}

impl OnePole {
    pub fn new() -> Self {
        Self { coeff: 1.0, state: 0.0 }
    }

    pub fn set(&mut self, freq_hz: f32, sample_rate: f32) {
        self.coeff = 1.0 - (-TAU * freq_hz.min(sample_rate * 0.45) / sample_rate).exp();
    }

    pub fn reset(&mut self) {
        self.state = 0.0;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.state += self.coeff * (input - self.state);
        self.state
    }
}

/// The vibration modes of a drum head: decaying sines at inharmonic frequencies that
/// share one pitch bend
pub struct Modes<const N: usize> {
    phase: [f32; N],
    // Cycles per sample
    freq: [f32; N],
    amp: [f32; N],
    // Envelopes run from 1.0 towards 0.0
    env: [f32; N],
    coeff: [f32; N],
}

impl<const N: usize> Modes<N> {
    pub fn new() -> Self {
        Self {
            phase: [0.0; N],
            freq: [0.0; N],
            amp: [0.0; N],
            env: [0.0; N],
            coeff: [0.0; N],
        }
    }

    pub fn set(&mut self, index: usize, freq_hz: f32, amp: f32, decay_ms: f32, sample_rate: f32) {
        // Modes that would alias are left out
        let audible = freq_hz < sample_rate * 0.4;
        self.freq[index] = freq_hz / sample_rate;
        self.amp[index] = if audible { amp } else { 0.0 };
        self.coeff[index] = decay_coeff(decay_ms, sample_rate);
    }

    pub fn strike(&mut self) {
        self.phase = [0.0; N];
        self.env = [1.0; N];
    }

    pub fn reset(&mut self) {
        self.env = [0.0; N];
    }

    pub fn is_active(&self) -> bool {
        (0..N).any(|i| self.env[i] * self.amp[i] > SILENCE_THRESHOLD)
    }

    /// `bend` multiplies every mode frequency (1.0 = no bend)
    pub fn process(&mut self, bend: f32) -> f32 {
        let mut output = 0.0;
        for i in 0..N {
            output += (self.phase[i] * TAU).sin() * self.env[i] * self.amp[i];

            self.phase[i] += self.freq[i] * bend;
            if self.phase[i] >= 1.0 {
                self.phase[i] -= 1.0;
            }
            self.env[i] *= self.coeff[i];
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{peak, zero_crossing_freq};

    const SAMPLE_RATE: f32 = 44100.0;

    #[test]
    fn test_noise_is_bounded_and_centered() {
        let mut noise = Noise::new(1);
        let samples: Vec<f32> = (0..100_000).map(|_| noise.next()).collect();

        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!(samples.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!(mean.abs() < 0.01, "Noise mean: {}", mean);
    }

    #[test]
    fn test_bus_clip_is_clean_below_knee_and_bounded_above() {
        assert_eq!(bus_clip(0.5), 0.5);
        assert_eq!(bus_clip(-BUS_CLIP_KNEE), -BUS_CLIP_KNEE);

        let mut previous = 0.0;
        for i in 1..1000 {
            let output = bus_clip(i as f32 * 0.01);
            assert!(output >= previous && output <= 1.0, "Not monotonic or unbounded at {}", i);
            previous = output;
        }
        assert_eq!(bus_clip(-5.0), -bus_clip(5.0));
    }

    #[test]
    fn test_svf_highpass_removes_low_frequencies() {
        let mut filter = Svf::new();
        filter.set(2000.0, 0.7, SAMPLE_RATE);

        let low: Vec<f32> = (0..8192)
            .map(|i| filter.process((i as f32 * TAU * 100.0 / SAMPLE_RATE).sin()).high)
            .collect();
        filter.reset();
        let high: Vec<f32> = (0..8192)
            .map(|i| filter.process((i as f32 * TAU * 8000.0 / SAMPLE_RATE).sin()).high)
            .collect();

        assert!(peak(&low[4096..]) < 0.01);
        assert!(peak(&high[4096..]) > 0.9);
    }

    #[test]
    fn test_svf_stable_above_nyquist() {
        let mut filter = Svf::new();
        filter.set(30000.0, 0.7, 22050.0);
        let mut noise = Noise::new(3);

        for _ in 0..10_000 {
            let output = filter.process(noise.next());
            assert!(output.high.is_finite() && output.band.is_finite());
        }
    }

    #[test]
    fn test_modes_ring_at_their_frequency_and_decay() {
        let mut modes: Modes<1> = Modes::new();
        modes.set(0, 200.0, 1.0, 50.0, SAMPLE_RATE);
        assert!(!modes.is_active());

        modes.strike();
        let output: Vec<f32> = (0..SAMPLE_RATE as usize).map(|_| modes.process(1.0)).collect();

        let freq = zero_crossing_freq(&output[..4410], SAMPLE_RATE);
        assert!((freq - 200.0).abs() < 5.0, "Mode frequency: {}", freq);
        assert!(!modes.is_active());
    }
}
