//! The slow oscillator that moves a pedal's sweep: a sine whose rate can change while it runs

use crate::dsp::smoothing_coeff;

// How fast the rate follows its dial
const RATE_SMOOTH_MS: f32 = 50.0;
// In Hz
const RATE_SETTLED: f64 = 1e-4;

pub struct Lfo {
    // Where in its turn the oscillator is, 0.0 to 1.0
    phase: f64,
    // Where the rate is, on its way to the setting, in Hz. In f64, as the place in the
    // turn: at the highest sample rates the steps are too small for f32
    rate: f64,
    rate_coeff: f64,
    // Turns per sample at 1 Hz
    turns_per_hz: f64,
}

impl Lfo {
    pub fn new() -> Self {
        Self { phase: 0.0, rate: 0.0, rate_coeff: 1.0, turns_per_hz: 0.0 }
    }

    /// Keeps the place in the turn
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.rate_coeff = smoothing_coeff(RATE_SMOOTH_MS, sample_rate) as f64;
        self.turns_per_hz = 1.0 / sample_rate as f64;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Takes a rate at once, for when nothing is heard of the oscillator
    pub fn snap(&mut self, rate_hz: f32) {
        self.rate = rate_hz as f64;
    }

    /// One sample on. The rate follows `rate_hz`; the turn goes on from where it is
    pub fn step(&mut self, rate_hz: f32) {
        let rate_hz = rate_hz as f64;
        if self.rate != rate_hz {
            let next = self.rate + self.rate_coeff * (rate_hz - self.rate);
            self.rate = if (rate_hz - next).abs() < RATE_SETTLED { rate_hz } else { next };
        }
        self.phase += self.rate * self.turns_per_hz;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
    }

    /// The sine, -1.0 to 1.0, `turns_ahead` of where the oscillator is (0.0 to 1.0)
    pub fn sine(&self, turns_ahead: f32) -> f32 {
        let turn = self.phase as f32 + turns_ahead;
        let turn = if turn >= 1.0 { turn - 1.0 } else { turn };
        // Half a turn back, which turns the sine upside down, then folded into the quarter
        // turn either side of zero, where a short polynomial is the sine to within 0.0002
        let mut x = turn - 0.5;
        if x > 0.25 {
            x = 0.5 - x;
        } else if x < -0.25 {
            x = -0.5 - x;
        }
        let t = -4.0 * x;
        let square = t * t;
        t * (1.570_796_3 + square * (-0.645_964_1 + square * (0.079_692_6 - 0.004_681_8 * square)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_lfo(sample_rate: f32, rate_hz: f32) -> Lfo {
        let mut lfo = Lfo::new();
        lfo.set_sample_rate(sample_rate);
        lfo.snap(rate_hz);
        lfo
    }

    #[test]
    fn test_sine_is_a_sine() {
        let mut lfo = new_lfo(1000.0, 1.0);
        for index in 0..3000 {
            for ahead in [0.0, 0.25, 0.7] {
                let expected =
                    (std::f64::consts::TAU * (index as f64 / 1000.0 + ahead as f64)).sin() as f32;
                assert!(
                    (lfo.sine(ahead) - expected).abs() < 3e-4,
                    "sample {}: {} against {}",
                    index,
                    lfo.sine(ahead),
                    expected
                );
                assert!(lfo.sine(ahead).abs() <= 1.0);
            }
            lfo.step(1.0);
        }
    }

    #[test]
    fn test_turns_at_the_set_rate_at_every_sample_rate() {
        for sample_rate in [44100.0, 384_000.0] {
            for rate_hz in [0.05, 0.8, 12.0] {
                let mut lfo = new_lfo(sample_rate, rate_hz);
                let (mut turns, mut last) = (0.0, 0.0);
                let seconds = 30.0;
                for _ in 0..(seconds * sample_rate as f64) as usize {
                    lfo.step(rate_hz);
                    if lfo.phase < last {
                        turns += 1.0;
                    }
                    last = lfo.phase;
                }
                let measured = (turns + lfo.phase) / seconds;
                assert!(
                    (measured / rate_hz as f64 - 1.0).abs() < 1e-4,
                    "{} Hz at {} Hz: {}",
                    rate_hz,
                    sample_rate,
                    measured
                );
            }
        }
    }

    #[test]
    fn test_a_new_rate_goes_on_from_the_same_place() {
        let mut lfo = new_lfo(48000.0, 0.1);
        for _ in 0..100_000 {
            lfo.step(0.1);
        }
        // The largest step of a sine at the faster rate
        let limit = 1.1 * std::f32::consts::TAU * 10.0 / 48000.0;
        let mut last = lfo.sine(0.0);
        for _ in 0..48000 {
            lfo.step(10.0);
            assert!((lfo.sine(0.0) - last).abs() < limit);
            last = lfo.sine(0.0);
        }
        assert_eq!(lfo.rate, 10.0);
    }
}
