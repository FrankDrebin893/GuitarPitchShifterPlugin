//! A short delay line that is read at a distance that moves: what the chorus and the flanger
//! are made of

use super::MAX_RATE_HZ;
use crate::delay::read_cubic;

// Samples the line is longer than the longest delay, for the interpolation
const LINE_MARGIN: usize = 8;
// The interpolation reads this many samples further back than the whole part of the delay
const READ_BEHIND: usize = 2;

pub struct ModLine {
    line: Vec<f32>,
    write: usize,
    // Samples behind the write position that count: what is further back reads as silence
    valid: usize,
}

impl ModLine {
    /// Allocates for delays up to `longest_ms` at every rate up to `MAX_RATE_HZ`
    pub fn new(longest_ms: f32) -> Self {
        let mut line = vec![0.0; (longest_ms * 0.001 * MAX_RATE_HZ).ceil() as usize + LINE_MARGIN];
        // Memory that has only been asked for is handed over when it is first written to,
        // which would be on the audio thread. Writing to all of it here takes it now
        line.fill(1.0);
        std::hint::black_box(&mut line[..]);
        line.fill(0.0);
        let valid = line.len();
        Self { line, write: 0, valid }
    }

    /// Does not allocate
    pub fn clear(&mut self) {
        self.line.fill(0.0);
        self.write = 0;
        self.valid = self.line.len();
    }

    /// What went in `delay` samples ago: a cubic through the four samples around. At least
    /// 2 samples, and no more than the line was made for
    pub fn read(&self, delay: f64) -> f32 {
        let whole = delay as usize;
        if whole + READ_BEHIND > self.valid {
            return 0.0;
        }
        read_cubic(&self.line, self.write, whole, (delay - whole as f64) as f32)
    }

    pub fn push(&mut self, sample: f32) {
        self.line[self.write] = sample;
        self.write = if self.write + 1 == self.line.len() { 0 } else { self.write + 1 };
        self.valid = (self.valid + 1).min(self.line.len());
    }

    /// From here on only the newest `samples` count. For a pedal that falls idle with
    /// zeros as far back as it reads and older sound behind them: that is never heard,
    /// whatever is read later
    pub fn keep_newest(&mut self, samples: usize) {
        self.valid = samples.min(self.line.len());
    }

    /// Address and capacity of the buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub(crate) fn buffer(&self) -> (usize, usize) {
        (self.line.as_ptr() as usize, self.line.capacity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;

    #[test]
    fn test_reads_what_went_in_that_long_ago() {
        // The signal is longer than the line, so the read wraps around its end
        let mut line = ModLine::new(1.0);
        let input = sine(440.0, 0.5, 48000.0, 3000);
        for delay in [2.0, 17.25, 100.5, 383.9] {
            line.clear();
            for (index, &sample) in input.iter().enumerate() {
                let expected =
                    0.5 * (std::f64::consts::TAU * 440.0 * (index as f64 - delay) / 48000.0).sin();
                if index > 400 {
                    assert!(
                        (line.read(delay) as f64 - expected).abs() < 1e-4,
                        "{} samples back at {}",
                        delay,
                        index
                    );
                }
                line.push(sample);
            }
        }
    }

    #[test]
    fn test_is_long_enough_at_the_highest_rate() {
        let mut line = ModLine::new(12.0);
        for index in 0..10_000 {
            line.push(index as f32);
        }
        let delay = 12.0 * 0.001 * MAX_RATE_HZ as f64;
        assert_eq!(line.read(delay), 10_000.0 - delay as f32);
    }

    #[test]
    fn test_what_is_older_than_was_kept_reads_as_silence() {
        let mut line = ModLine::new(1.0);
        for _ in 0..500 {
            line.push(1.0);
        }
        line.keep_newest(10);
        assert_eq!(line.read(5.0), 1.0);
        assert_eq!(line.read(9.5), 0.0);
        assert_eq!(line.read(200.0), 0.0);
        line.push(1.0);
        line.push(1.0);
        assert_eq!(line.read(9.5), 1.0);
    }
}
