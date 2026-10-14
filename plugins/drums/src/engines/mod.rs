//! The Metal and Groove kits. Unlike Rock and Jazz they are not presets of the modal voices
//! but instruments of their own: one voice per piece, cymbals made of square waves and
//! noise through filters, a comb room and a saturating bus. `prototypes/jsfx` has the
//! scripts they were written as first; the numbers here are theirs.

use std::f32::consts::TAU;

use crate::dsp::decay_coeff;

pub mod groove;
pub mod metal;

/// Envelopes under this are finished (-90 dB)
const ENV_FLOOR: f32 = 0.00003;

// An engine whose output has been under this for this long is put to rest. The output is
// behind the level controls, which go no lower than -30 dB: what is cut is 90 dB down or more
const REST_LEVEL: f32 = 1e-6;
const REST_S: f32 = 0.1;

/// Per-sample multiplier of an envelope with the time constant `tau_s`
fn coef(tau_s: f32, sample_rate: f32) -> f32 {
    decay_coeff(tau_s.max(0.00001) * 1000.0, sample_rate)
}

fn db(level_db: f32) -> f32 {
    10.0f32.powf(level_db / 20.0)
}

fn sine(phase: f32) -> f32 {
    (TAU * phase).sin()
}

/// Moves a phase (0.0-1.0) on by one sample of `freq_hz`
fn advance(phase: &mut f32, freq_hz: f32, sample_rate: f32) {
    *phase += freq_hz / sample_rate;
    *phase -= phase.floor();
}

/// Six square waves at inharmonic ratios: the metal of hi-hats and cymbals
struct MetalCluster {
    freqs_hz: [f32; 6],
    phases: [f32; 6],
}

impl MetalCluster {
    const RATIOS: [f32; 6] = [1.0, 1.4827, 1.8003, 2.5460, 2.6303, 3.8967];

    fn new(base_hz: f32) -> Self {
        Self {
            freqs_hz: Self::RATIOS.map(|ratio| base_hz * ratio),
            phases: [0.0; 6],
        }
    }

    fn reset(&mut self) {
        self.phases = [0.0; 6];
    }

    fn process(&mut self, sample_rate: f32) -> f32 {
        let mut high = 0.0;
        for (phase, freq_hz) in self.phases.iter_mut().zip(self.freqs_hz) {
            advance(phase, freq_hz, sample_rate);
            if *phase < 0.5 {
                high += 1.0;
            }
        }
        (high - 3.0) / 3.0
    }
}

struct Comb {
    buffer: Vec<f32>,
    position: usize,
    filtered: f32,
}

impl Comb {
    fn new() -> Self {
        Self {
            buffer: vec![0.0; 1],
            position: 0,
            filtered: 0.0,
        }
    }

    fn resize(&mut self, length: usize) {
        self.buffer = vec![0.0; length.max(1)];
        self.position = 0;
        self.filtered = 0.0;
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.filtered = 0.0;
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let output = self.buffer[self.position];
        self.filtered = output * (1.0 - damp) + self.filtered * damp;
        self.buffer[self.position] = input + self.filtered * feedback;
        self.position += 1;
        if self.position >= self.buffer.len() {
            self.position = 0;
        }
        output
    }
}

struct AllPass {
    buffer: Vec<f32>,
    position: usize,
}

impl AllPass {
    fn new() -> Self {
        Self {
            buffer: vec![0.0; 1],
            position: 0,
        }
    }

    fn resize(&mut self, length: usize) {
        self.buffer = vec![0.0; length.max(1)];
        self.position = 0;
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
    }

    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.position];
        self.buffer[self.position] = input + delayed * 0.5;
        self.position += 1;
        if self.position >= self.buffer.len() {
            self.position = 0;
        }
        delayed - input
    }
}

// Delay lengths in samples at 44.1 kHz
const COMBS_LEFT: [f32; 4] = [1116.0, 1188.0, 1277.0, 1356.0];
const COMBS_RIGHT: [f32; 4] = [1139.0, 1211.0, 1300.0, 1379.0];
const ALL_PASSES_LEFT: [f32; 2] = [556.0, 441.0];
const ALL_PASSES_RIGHT: [f32; 2] = [579.0, 464.0];

/// A small room: four damped combs and two all-passes for each side, fed with one channel
struct CombRoom {
    // Scale of the comb lengths: the size of the room
    size: f32,
    feedback: f32,
    damp: f32,
    combs: [[Comb; 4]; 2],
    all_passes: [[AllPass; 2]; 2],
}

impl CombRoom {
    fn new(size: f32, feedback: f32, damp: f32) -> Self {
        Self {
            size,
            feedback,
            damp,
            combs: std::array::from_fn(|_| std::array::from_fn(|_| Comb::new())),
            all_passes: std::array::from_fn(|_| std::array::from_fn(|_| AllPass::new())),
        }
    }

    /// Allocates the delay lines. Not for the audio thread.
    fn set_sample_rate(&mut self, sample_rate: f32) {
        let rate = sample_rate / 44100.0;
        for (side, lengths) in [COMBS_LEFT, COMBS_RIGHT].into_iter().enumerate() {
            for (comb, length) in self.combs[side].iter_mut().zip(lengths) {
                comb.resize((length * rate * self.size) as usize);
            }
        }
        for (side, lengths) in [ALL_PASSES_LEFT, ALL_PASSES_RIGHT].into_iter().enumerate() {
            for (all_pass, length) in self.all_passes[side].iter_mut().zip(lengths) {
                all_pass.resize((length * rate) as usize);
            }
        }
    }

    fn reset(&mut self) {
        for comb in self.combs.iter_mut().flatten() {
            comb.reset();
        }
        for all_pass in self.all_passes.iter_mut().flatten() {
            all_pass.reset();
        }
    }

    fn process(&mut self, input: f32) -> (f32, f32) {
        let mut sides = [0.0; 2];
        for (side, output) in sides.iter_mut().enumerate() {
            let mut sum = 0.0;
            for comb in &mut self.combs[side] {
                sum += comb.process(input, self.feedback, self.damp);
            }
            for all_pass in &mut self.all_passes[side] {
                sum = all_pass.process(sum);
            }
            *output = sum;
        }
        (sides[0], sides[1])
    }
}

/// The saturation on the bus: tanh, scaled so that full level stays full level
struct BusDrive {
    gain: f32,
    makeup: f32,
}

impl BusDrive {
    fn new(gain: f32) -> Self {
        Self {
            gain,
            makeup: 1.0 / gain.tanh(),
        }
    }

    fn process(&self, input: f32) -> f32 {
        (input * self.gain).tanh() * self.makeup
    }
}

/// Counts how long an engine has been silent, so that it can stop working
struct Rest {
    quiet: u32,
    limit: u32,
}

impl Rest {
    fn new() -> Self {
        Self { quiet: 0, limit: u32::MAX }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.limit = (REST_S * sample_rate) as u32;
    }

    fn wake(&mut self) {
        self.quiet = 0;
    }

    /// True once the output has been silent for long enough
    fn is_due(&mut self, left: f32, right: f32) -> bool {
        if left.abs() + right.abs() < REST_LEVEL {
            self.quiet += 1;
        } else {
            self.quiet = 0;
        }
        self.quiet >= self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{decay_time_s, peak};

    #[test]
    fn test_metal_cluster_is_bounded_and_not_one_tone() {
        let mut cluster = MetalCluster::new(410.0);
        let output: Vec<f32> = (0..44100).map(|_| cluster.process(44100.0)).collect();

        assert!(output.iter().all(|sample| sample.abs() <= 1.0));
        // Seven levels, from all six squares low to all six high
        let mut levels: Vec<i32> = output.iter().map(|sample| (sample * 3.0).round() as i32).collect();
        levels.sort();
        levels.dedup();
        assert_eq!(levels, vec![-3, -2, -1, 0, 1, 2, 3]);
    }

    #[test]
    fn test_room_rings_and_dies_away() {
        let sample_rate = 44100.0;
        let mut room = CombRoom::new(0.6, 0.74, 0.35);
        room.set_sample_rate(sample_rate);

        let (left, right): (Vec<f32>, Vec<f32>) =
            (0..88200).map(|i| room.process(if i == 0 { 1.0 } else { 0.0 })).unzip();

        assert!(peak(&left) > 0.1);
        assert_ne!(left, right);
        let decay = decay_time_s(&left, sample_rate, -40.0);
        assert!((0.1..1.0).contains(&decay), "Decay: {} s", decay);
        assert!(peak(&left[66150..]) < 0.0001);
    }

    #[test]
    fn test_bus_drive_keeps_full_level() {
        let drive = BusDrive::new(1.44);
        assert!((drive.process(1.0) - 1.0).abs() < 1e-6);
        assert!(drive.process(0.1) > 0.1);
        assert!(drive.process(10.0) < 1.2);
    }
}
