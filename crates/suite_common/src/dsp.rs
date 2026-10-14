//! Building blocks the instruments share. Nothing here allocates after `set_sample_rate`.

use std::f32::consts::{PI, TAU};

pub fn db_to_gain(level_db: f32) -> f32 {
    10.0f32.powf(level_db / 20.0)
}

/// Frequency of a MIDI note, with a transposition in semitones
pub fn note_hz(note: f32) -> f32 {
    440.0 * 2.0f32.powf((note - 69.0) / 12.0)
}

/// Per-sample multiplier of an envelope with the time constant `tau_s`
pub fn decay_coeff(tau_s: f32, sample_rate: f32) -> f32 {
    (-1.0 / (tau_s.max(0.00001) * sample_rate)).exp()
}

pub fn sine(phase: f32) -> f32 {
    (TAU * phase).sin()
}

/// Moves a phase (0.0-1.0) on by `step` cycles
pub fn advance(phase: &mut f32, step: f32) {
    *phase += step;
    *phase -= phase.floor();
}

/// White noise in -1.0..1.0 (xorshift)
pub struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self { state: seed.wrapping_mul(0x9E37_79B9) | 1 }
    }

    pub fn next(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state as f32 / 2_147_483_648.0 - 1.0
    }

    /// 0.0..1.0
    pub fn next_unipolar(&mut self) -> f32 {
        self.next() * 0.5 + 0.5
    }
}

pub struct SvfOutput {
    pub low: f32,
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

impl Default for Svf {
    fn default() -> Self {
        Self::new()
    }
}

impl Svf {
    pub fn new() -> Self {
        Self { k: 1.0, a1: 0.0, a2: 0.0, a3: 0.0, ic1: 0.0, ic2: 0.0 }
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

        SvfOutput { low: v2, band: v1, high: input - self.k * v1 - v2 }
    }
}

struct Diffuser {
    buffer: Vec<f32>,
    position: usize,
    gain: f32,
}

impl Diffuser {
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.position];
        let output = delayed - self.gain * input;
        self.buffer[self.position] = input + self.gain * output;
        self.position += 1;
        if self.position >= self.buffer.len() {
            self.position = 0;
        }
        output
    }
}

// Lengths in samples at 44.1 kHz, and gains
const DIFFUSERS: [(f32, f32); 4] = [(142.0, 0.7), (379.0, 0.7), (107.0, 0.65), (277.0, 0.65)];
const HALL_LINES: usize = 8;

/// What makes one hall differ from another
pub struct HallShape {
    /// Lengths of the eight delay lines in samples at 44.1 kHz
    pub lines: [f32; HALL_LINES],
    /// How far the lines are swept, in samples at 44.1 kHz, and how fast
    pub sweep: f32,
    pub sweep_hz: f32,
    /// Where the tail starts to lose its top
    pub damp_hz: f32,
}

/// A large hall: four diffusers into eight delay lines that feed each other and are
/// slowly swept, so the tail never settles into a tone. One channel in, two out.
pub struct Hall {
    shape: HallShape,
    sample_rate: f32,
    diffusers: [Diffuser; 4],
    lines: [Vec<f32>; HALL_LINES],
    lengths: [usize; HALL_LINES],
    feedback: [f32; HALL_LINES],
    damped: [f32; HALL_LINES],
    mask: usize,
    write: usize,
    sweep: f32,
    sweep_phase: f32,
    damp: f32,
    decay_s: f32,
}

impl Hall {
    pub fn new(shape: HallShape) -> Self {
        let mut hall = Self {
            shape,
            sample_rate: 44100.0,
            diffusers: DIFFUSERS.map(|(_, gain)| Diffuser {
                buffer: vec![0.0; 1],
                position: 0,
                gain,
            }),
            lines: std::array::from_fn(|_| Vec::new()),
            lengths: [1; HALL_LINES],
            feedback: [0.0; HALL_LINES],
            damped: [0.0; HALL_LINES],
            mask: 0,
            write: 0,
            sweep: 0.0,
            sweep_phase: 0.0,
            damp: 1.0,
            decay_s: 1.0,
        };
        hall.set_sample_rate(44100.0);
        hall
    }

    /// Allocates the delay lines. Not for the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let rate = sample_rate / 44100.0;
        self.sample_rate = sample_rate;
        self.sweep = self.shape.sweep * rate;
        self.damp = 1.0 - (-TAU * self.shape.damp_hz / sample_rate).exp();

        for (diffuser, (length, _)) in self.diffusers.iter_mut().zip(DIFFUSERS) {
            diffuser.buffer = vec![0.0; ((length * rate) as usize).max(1)];
            diffuser.position = 0;
        }

        let longest = self.shape.lines.iter().fold(0.0f32, |longest, &line| longest.max(line));
        let size = ((longest * rate + 2.0 * self.sweep) as usize + 4).next_power_of_two();
        self.mask = size - 1;
        for (index, line) in self.lines.iter_mut().enumerate() {
            *line = vec![0.0; size];
            self.lengths[index] = (self.shape.lines[index] * rate) as usize;
        }
        self.write = 0;
        self.damped = [0.0; HALL_LINES];
        self.set_feedback();
    }

    /// Time for the tail to fall by 60 dB
    pub fn set_decay(&mut self, decay_s: f32) {
        if decay_s != self.decay_s {
            self.decay_s = decay_s;
            self.set_feedback();
        }
    }

    fn set_feedback(&mut self) {
        let decay_s = self.decay_s;
        for (feedback, &length) in self.feedback.iter_mut().zip(&self.lengths) {
            *feedback = 10.0f32.powf(-3.0 * length as f32 / (decay_s * self.sample_rate));
        }
    }

    pub fn reset(&mut self) {
        for diffuser in &mut self.diffusers {
            diffuser.buffer.fill(0.0);
        }
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.damped = [0.0; HALL_LINES];
    }

    pub fn process(&mut self, input: f32) -> (f32, f32) {
        let mut input = input;
        for diffuser in &mut self.diffusers {
            input = diffuser.process(input);
        }

        self.write = (self.write + 1) & self.mask;
        advance(&mut self.sweep_phase, self.shape.sweep_hz / self.sample_rate);

        let mut taps = [0.0; HALL_LINES];
        for (index, tap) in taps.iter_mut().enumerate() {
            // Each line is swept an eighth of a turn after the one before
            let delay = self.sweep * (1.0 + sine(self.sweep_phase + index as f32 * 0.125));
            let whole = delay.ceil();
            let fraction = whole - delay;
            let position = self.write + self.mask + 1 - self.lengths[index] - whole as usize;
            let line = &self.lines[index];
            let first = line[position & self.mask];
            let second = line[(position + 1) & self.mask];
            let delayed = first + (second - first) * fraction;

            self.damped[index] += (delayed - self.damped[index]) * self.damp;
            *tap = self.damped[index] * self.feedback[index];
        }

        let left = (taps[0] + taps[2] + taps[4] + taps[6]) * 0.5;
        let right = (taps[1] + taps[3] + taps[5] + taps[7]) * 0.5;

        // Every line feeds every other at the same level (a Hadamard matrix)
        let a = [
            taps[0] + taps[1],
            taps[0] - taps[1],
            taps[2] + taps[3],
            taps[2] - taps[3],
            taps[4] + taps[5],
            taps[4] - taps[5],
            taps[6] + taps[7],
            taps[6] - taps[7],
        ];
        let b = [
            a[0] + a[2],
            a[1] + a[3],
            a[0] - a[2],
            a[1] - a[3],
            a[4] + a[6],
            a[5] + a[7],
            a[4] - a[6],
            a[5] - a[7],
        ];
        for index in 0..4 {
            let sign = if index % 2 == 0 { input } else { -input };
            self.lines[index][self.write] = (b[index] + b[index + 4]) * 0.353553 + sign;
            self.lines[index + 4][self.write] = (b[index] - b[index + 4]) * 0.353553 + sign;
        }

        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 44100.0;

    fn shape() -> HallShape {
        HallShape {
            lines: [1777.0, 2053.0, 2399.0, 2687.0, 3001.0, 3329.0, 3697.0, 4001.0],
            sweep: 6.0,
            sweep_hz: 0.1,
            damp_hz: 6000.0,
        }
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn test_note_hz() {
        assert!((note_hz(69.0) - 440.0).abs() < 0.001);
        assert!((note_hz(57.0) - 220.0).abs() < 0.001);
    }

    #[test]
    fn test_noise_is_bounded_and_centred() {
        let mut noise = Noise::new(7);
        let samples: Vec<f32> = (0..100_000).map(|_| noise.next()).collect();
        assert!(samples.iter().all(|sample| (-1.0..=1.0).contains(sample)));
        assert!((samples.iter().sum::<f32>() / samples.len() as f32).abs() < 0.01);
    }

    #[test]
    fn test_svf_low_and_high_split_a_tone() {
        let tone = |freq_hz: f32| {
            let mut filter = Svf::new();
            filter.set(1000.0, 0.7, SAMPLE_RATE);
            let (low, high): (Vec<f32>, Vec<f32>) = (0..8192)
                .map(|i| {
                    let output = filter.process(sine(i as f32 * freq_hz / SAMPLE_RATE));
                    (output.low, output.high)
                })
                .unzip();
            (rms(&low[4096..]), rms(&high[4096..]))
        };

        let (low, high) = tone(100.0);
        assert!(low > 0.6 && high < 0.02);
        let (low, high) = tone(10000.0);
        assert!(low < 0.02 && high > 0.6);
    }

    #[test]
    fn test_hall_tail_follows_the_decay_time_and_is_stereo() {
        for decay_s in [1.0, 4.0] {
            let mut hall = Hall::new(shape());
            hall.set_sample_rate(SAMPLE_RATE);
            hall.set_decay(decay_s);

            let length = (decay_s * 1.5 * SAMPLE_RATE) as usize;
            let (left, right): (Vec<f32>, Vec<f32>) =
                (0..length).map(|i| hall.process(if i == 0 { 1.0 } else { 0.0 })).unzip();
            assert_ne!(left, right);

            // Level around a tenth of the decay time against the level around the whole of it:
            // 54 dB apart, less what the damping takes
            let window = (0.05 * SAMPLE_RATE) as usize;
            let around = |seconds: f32| {
                let start = (seconds * SAMPLE_RATE) as usize;
                rms(&left[start..start + window])
            };
            let fall_db = 20.0 * (around(decay_s * 0.1) / around(decay_s)).log10();
            assert!((48.0..75.0).contains(&fall_db), "Fell {} dB in 0.9 of {} s", fall_db, decay_s);
        }
    }

    #[test]
    fn test_hall_is_silent_without_input_and_after_reset() {
        let mut hall = Hall::new(shape());
        hall.set_sample_rate(48000.0);
        assert!((0..4096).all(|_| hall.process(0.0) == (0.0, 0.0)));

        hall.process(1.0);
        for _ in 0..10000 {
            hall.process(0.0);
        }
        hall.reset();
        assert!((0..4096).all(|_| hall.process(0.0) == (0.0, 0.0)));
    }
}
