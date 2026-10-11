//! Stereo delay behind the cabinet: warm repeats that thin and darken, a little wider than
//! the mono signal that goes in

use crate::dsp::filters::{OnePoleHp, OnePoleLp};
use crate::dsp::smoothing_coeff;

pub const TIME_MIN_MS: f32 = 20.0;
pub const TIME_MAX_MS: f32 = 1000.0;
pub const FEEDBACK_MAX: f32 = 0.9;

// The left line is this share of the time shorter and the right one as much longer, so the
// first repeat is wide. The lines feed each other, so the difference does not add up: every
// second repeat lands on both sides at once, and the repeats keep the tempo of the setting
const SPREAD: f64 = 0.012;
// At long times the two sides would be heard as two hits
const SPREAD_MAX_MS: f64 = 5.0;

// Every repeat loses lows and highs, as on tape
const HIGHPASS_HZ: f32 = 170.0;
const LOWPASS_HZ: f32 = 4000.0;

// What is fed back is rounded off towards this level, so repeats of a loud passage at full
// feedback pile up to a limit and not beyond
const SATURATION_LEVEL: f32 = 1.0;

// Fade of the input to the lines when the delay is switched
const FADE_MS: f32 = 10.0;
// How fast feedback and mix follow their dials
const DIAL_SMOOTH_MS: f32 = 20.0;
const DIAL_SETTLED: f32 = 1e-5;
// How fast the read position follows the time dial. The pitch of the repeats bends while it
// moves, as when a tape changes speed
const TIME_SMOOTH_MS: f32 = 120.0;
// Samples of delay per sample at most, which keeps the bend within an octave down and a
// fifth up
const GLIDE_MAX: f64 = 0.5;
// In samples
const TIME_SETTLED: f64 = 1e-3;

/// -100 dBFS. Below this the lines are fed exact zeros: nothing is left to turn into
/// denormal numbers, and once the read positions have only zeros ahead of them the delay is
/// idle. What is cut off there is under the noise of a 16-bit recording
pub(crate) const SILENCE: f32 = 1e-5;

// Samples a line is longer than the longest delay, for the interpolation
const LINE_MARGIN: usize = 8;

// The interpolation reads this many samples further back than the whole part of the delay
const READ_BEHIND: usize = 2;

/// What the knobs say, read once per block. The delay smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelaySettings {
    /// Off lets the repeats ring out
    pub on: bool,
    /// `TIME_MIN_MS` to `TIME_MAX_MS`
    pub time_ms: f32,
    /// Level of each repeat relative to the one before, 0.0 to `FEEDBACK_MAX`
    pub feedback: f32,
    /// Level of the repeats, 0.0 to 1.0. The dry signal stays as it is
    pub mix: f32,
}

impl Default for DelaySettings {
    fn default() -> Self {
        Self {
            on: false,
            time_ms: 350.0,
            feedback: 0.35,
            mix: 0.25,
        }
    }
}

/// Reads a delay line `whole + fraction` samples behind `write`, the place the next sample
/// goes. A cubic through the four samples around: its high end stays the same whatever the
/// fraction is. `whole` is at least 2 and at most the line's length less 3
pub(crate) fn read_cubic(line: &[f32], write: usize, whole: usize, fraction: f32) -> f32 {
    let len = line.len();
    let wrap = |index: usize| if index >= len { index - len } else { index };
    let first = wrap(write + len - whole - 2);
    let second = wrap(first + 1);
    let third = wrap(second + 1);
    let (xm1, x0, x1, x2) = (line[first], line[second], line[third], line[wrap(third + 1)]);

    let position = 1.0 - fraction;
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * position + c2) * position + c1) * position + x0
}

/// Rounds off towards `SATURATION_LEVEL`. Leaves quiet signals as they are, so the feedback
/// dial is the level of the repeats
fn saturate(input: f32) -> f32 {
    // A rational curve close to tanh that reaches 1 with no slope at 3
    let x = (input * (1.0 / SATURATION_LEVEL)).clamp(-3.0, 3.0);
    let square = x * x;
    SATURATION_LEVEL * x * (27.0 + square) / (27.0 + 9.0 * square)
}

/// Two delay lines that feed each other. The same input goes into both; only the repeats
/// differ between left and right
pub struct Delay {
    sample_rate: f32,
    // Left and right, allocated in `set_sample_rate`
    lines: [Vec<f32>; 2],
    write: usize,
    highpass: [OnePoleHp; 2],
    lowpass: [OnePoleLp; 2],

    // Where the dials are, on their way to the settings. The time is in samples
    time: f64,
    feedback: f32,
    mix: f32,
    // Share of the input that reaches the lines: 1.0 on, 0.0 off
    feed: f32,

    time_coeff: f64,
    dial_coeff: f32,
    feed_step: f32,
    spread_max: f64,

    // Samples in a row that only zeros went into the lines
    quiet_run: usize,
    // Samples behind the write position that count. The delay falls idle as soon as the
    // read positions have only zeros ahead of them, which leaves older repeats in the lines
    // further back. They are never heard: what is further back than this reads as silence
    valid: usize,
    // Nothing is left to hear and the filters are at rest
    idle: bool,
}

impl Delay {
    pub fn new() -> Self {
        let mut delay = Self {
            sample_rate: 44100.0,
            lines: [Vec::new(), Vec::new()],
            write: 0,
            highpass: [OnePoleHp::new(); 2],
            lowpass: [OnePoleLp::new(); 2],
            time: 0.0,
            feedback: 0.0,
            mix: 0.0,
            feed: 0.0,
            time_coeff: 1.0,
            dial_coeff: 1.0,
            feed_step: 1.0,
            spread_max: 0.0,
            quiet_run: 0,
            valid: 0,
            idle: true,
        };
        delay.set_sample_rate(44100.0);
        delay
    }

    /// Allocates the lines for the longest time at this rate. Not for the audio thread
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.spread_max = SPREAD_MAX_MS * 0.001 * sample_rate as f64;
        let longest = TIME_MAX_MS as f64 * 0.001 * sample_rate as f64 + self.spread_max;
        for line in &mut self.lines {
            *line = vec![0.0; longest.ceil() as usize + LINE_MARGIN];
            // Memory that has only been asked for is handed over when it is first written
            // to, which would be on the audio thread. Writing to all of it here takes it now
            line.fill(1.0);
            std::hint::black_box(&mut line[..]);
            line.fill(0.0);
        }
        for (highpass, lowpass) in self.highpass.iter_mut().zip(self.lowpass.iter_mut()) {
            highpass.set(HIGHPASS_HZ, sample_rate);
            lowpass.set(LOWPASS_HZ, sample_rate);
        }
        self.time_coeff = smoothing_coeff(TIME_SMOOTH_MS, sample_rate) as f64;
        self.dial_coeff = smoothing_coeff(DIAL_SMOOTH_MS, sample_rate);
        self.feed_step = 1.0 / (FADE_MS * 0.001 * sample_rate);
        self.reset();
    }

    /// Drops the repeats. Does not allocate
    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.write = 0;
        self.valid = self.lines[0].len();
        self.rest();
    }

    /// For when nothing is left to hear
    fn rest(&mut self) {
        for (highpass, lowpass) in self.highpass.iter_mut().zip(self.lowpass.iter_mut()) {
            highpass.reset();
            lowpass.reset();
        }
        self.quiet_run = 0;
        self.idle = true;
    }

    /// True when there is nothing left to hear of earlier input: the next block comes out as
    /// it goes in, unless the delay is on and the block has sound in it
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// How far behind the write position the read positions get, now and on their way to
    /// a delay of `time` samples
    fn reach(&self, time: f64) -> usize {
        let longest = self.time.max(time);
        (longest + (longest * SPREAD).min(self.spread_max)) as usize + READ_BEHIND + 1
    }

    /// True when the filters would put out nothing more by themselves
    fn filters_at_rest(&self) -> bool {
        let quiet = |state: f32| state.abs() < SILENCE;
        self.highpass.iter().all(|filter| quiet(filter.state())) && self.lowpass.iter().all(|filter| quiet(filter.state()))
    }

    /// Address and capacity of every buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<(usize, usize)> {
        self.lines.iter().map(|line| (line.as_ptr() as usize, line.capacity())).collect()
    }

    /// Adds the repeats to a block in place. The lines are fed the average of the channels
    pub fn process(&mut self, settings: &DelaySettings, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        let len = left.len().min(right.len());
        let time = (settings.time_ms.clamp(TIME_MIN_MS, TIME_MAX_MS) * 0.001 * self.sample_rate) as f64;
        let feedback = settings.feedback.clamp(0.0, FEEDBACK_MAX);
        let mix = settings.mix.clamp(0.0, 1.0);

        let mut start = 0;
        while start < len {
            if self.idle {
                // Nothing to add until the delay is on and there is something to repeat
                if !settings.on {
                    return;
                }
                let Some(first) = (start..len).find(|&index| (0.5 * (left[index] + right[index])).abs() >= SILENCE) else {
                    return;
                };
                // Nothing is heard yet, so there is nothing to smooth
                self.time = time;
                self.feedback = feedback;
                self.mix = mix;
                self.feed = 1.0;
                self.idle = false;
                start = first;
            }
            start = self.run(settings.on, time, feedback, mix, &mut left[..len], &mut right[..len], start);
        }
    }

    /// Processes from `start` to the end of the block or to where the delay falls idle.
    /// Returns where it stopped
    #[allow(clippy::too_many_arguments)]
    fn run(&mut self, on: bool, time: f64, feedback: f32, mix: f32, left: &mut [f32], right: &mut [f32], start: usize) -> usize {
        let feed_target = if on { 1.0 } else { 0.0 };
        let line_len = self.lines[0].len();

        for index in start..left.len() {
            if self.feed != feed_target {
                self.feed = (self.feed + self.feed_step.copysign(feed_target - self.feed)).clamp(0.0, 1.0);
            }
            if self.feedback != feedback {
                self.feedback = follow(self.feedback, feedback, self.dial_coeff);
            }
            if self.mix != mix {
                self.mix = follow(self.mix, mix, self.dial_coeff);
            }
            if self.time != time {
                let distance = time - self.time;
                self.time = if distance.abs() < TIME_SETTLED {
                    time
                } else {
                    self.time + (self.time_coeff * distance).clamp(-GLIDE_MAX, GLIDE_MAX)
                };
            }

            let spread = (self.time * SPREAD).min(self.spread_max);
            let mut repeats = [0.0f32; 2];
            for (channel, delay) in [self.time - spread, self.time + spread].into_iter().enumerate() {
                let whole = delay as usize;
                let read = if whole + READ_BEHIND <= self.valid {
                    read_cubic(&self.lines[channel], self.write, whole, (delay - whole as f64) as f32)
                } else {
                    0.0
                };
                repeats[channel] = self.lowpass[channel].process(self.highpass[channel].process(read));
            }

            let input = 0.5 * (left[index] + right[index]) * self.feed;
            let mut quiet = true;
            for channel in 0..2 {
                // Each line is fed the other one's repeats
                let fed = input + saturate(self.feedback * repeats[1 - channel]);
                self.lines[channel][self.write] = if fed.abs() >= SILENCE {
                    quiet = false;
                    fed
                } else {
                    0.0
                };
            }
            self.write = if self.write + 1 == line_len { 0 } else { self.write + 1 };
            self.valid = (self.valid + 1).min(line_len);

            left[index] += self.mix * repeats[0];
            right[index] += self.mix * repeats[1];

            if !quiet {
                self.quiet_run = 0;
                continue;
            }
            // Idle once everything the read positions can get to is zeros, also if the time
            // dial is still on its way, and the filters have rung out
            self.quiet_run += 1;
            if self.quiet_run >= self.reach(time) && self.filters_at_rest() {
                self.valid = self.quiet_run.min(line_len);
                self.rest();
                return index + 1;
            }
        }
        left.len()
    }
}

/// A step of a one-pole follower that arrives
fn follow(value: f32, target: f32, coeff: f32) -> f32 {
    let next = value + coeff * (target - value);
    if (target - next).abs() < DIAL_SETTLED {
        target
    } else {
        next
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_util::*;
    use std::time::Instant;

    const SAMPLE_RATE: f32 = 48000.0;
    const BLOCK: usize = 64;

    fn new_delay(sample_rate: f32) -> Delay {
        let mut delay = Delay::new();
        delay.set_sample_rate(sample_rate);
        delay
    }

    pub(crate) fn settings(time_ms: f32, feedback: f32, mix: f32) -> DelaySettings {
        DelaySettings {
            on: true,
            time_ms,
            feedback,
            mix,
        }
    }

    fn off() -> DelaySettings {
        DelaySettings {
            on: false,
            ..settings(100.0, 0.5, 1.0)
        }
    }

    /// Runs a mono signal through in blocks. Returns left and right
    pub(crate) fn run_blocks(delay: &mut Delay, settings: &DelaySettings, input: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
            delay.process(settings, l, r);
        }
        (left, right)
    }

    pub(crate) fn run(settings: &DelaySettings, input: &[f32], sample_rate: f32) -> (Vec<f32>, Vec<f32>) {
        run_blocks(&mut new_delay(sample_rate), settings, input, BLOCK)
    }

    fn impulse(len: usize) -> Vec<f32> {
        let mut signal = vec![0.0; len];
        signal[0] = 1.0;
        signal
    }

    fn noise(level: f32, len: usize) -> Vec<f32> {
        let mut noise = Noise::new(3);
        (0..len).map(|_| level * noise.next()).collect()
    }

    fn peak_index(samples: &[f32]) -> usize {
        (0..samples.len()).max_by(|&a, &b| samples[a].abs().total_cmp(&samples[b].abs())).unwrap()
    }

    fn ms(time_ms: f32) -> usize {
        (time_ms * 0.001 * SAMPLE_RATE) as usize
    }

    #[test]
    fn test_echo_arrives_at_the_set_time() {
        for time_ms in [20.0, 250.0, 1000.0] {
            let (left, right) = run(&settings(time_ms, 0.0, 1.0), &impulse(ms(time_ms) * 2), SAMPLE_RATE);
            let spread_ms = (time_ms * SPREAD as f32).min(SPREAD_MAX_MS as f32);
            let expected = |side: f32| (time_ms + side * spread_ms) * 0.001 * SAMPLE_RATE;
            let (at_left, at_right) = (peak_index(&left[1..]) + 1, peak_index(&right[1..]) + 1);
            assert!((at_left as f32 - expected(-1.0)).abs() <= 1.5, "{} ms: left echo at sample {}", time_ms, at_left);
            assert!((at_right as f32 - expected(1.0)).abs() <= 1.5, "{} ms: right echo at sample {}", time_ms, at_right);
            // A single sample loses most of its height to the low-pass
            assert!(left[at_left] > 0.2 && right[at_right] > 0.2);
        }
    }

    #[test]
    fn test_feedback_zero_gives_one_repeat() {
        let (left, right) = run(&settings(100.0, 0.0, 1.0), &impulse(ms(600.0)), SAMPLE_RATE);
        for channel in [&left, &right] {
            assert!(peak(&channel[ms(90.0)..ms(110.0)]) > 0.2);
            assert!(peak(&channel[ms(150.0)..]) < 1e-5, "more than one repeat: {}", peak(&channel[ms(150.0)..]));
        }
    }

    #[test]
    fn test_repeats_get_quieter_and_darker() {
        // A short burst of noise, and what is left of it in each repeat
        let mut input = vec![0.0; ms(1800.0)];
        input[..ms(20.0)].copy_from_slice(&noise(0.5, ms(20.0)));
        let (left, _) = run(&settings(300.0, 0.6, 1.0), &input, SAMPLE_RATE);
        let repeat = |number: usize| &left[ms(300.0 * number as f32 - 10.0)..ms(300.0 * number as f32 + 60.0)];

        let levels: Vec<f32> = (1..=5).map(|number| rms(repeat(number))).collect();
        assert!(levels.windows(2).all(|pair| pair[1] < pair[0] * 0.7), "{:?}", levels);
        let highs: Vec<f32> = (1..=5).map(|number| band_level_db(repeat(number), SAMPLE_RATE, Some(4000.0), None)).collect();
        assert!(highs.windows(2).all(|pair| pair[1] < pair[0] - 1.0), "highs per repeat: {:?}", highs);
        let lows: Vec<f32> = (1..=5).map(|number| band_level_db(repeat(number), SAMPLE_RATE, None, Some(120.0))).collect();
        assert!(lows[4] < lows[0] - 3.0, "lows per repeat: {:?}", lows);
    }

    #[test]
    fn test_off_is_transparent_once_the_tail_has_died() {
        let mut delay = new_delay(SAMPLE_RATE);
        let input = noise(0.5, ms(300.0));
        run_blocks(&mut delay, &settings(50.0, 0.3, 1.0), &input, BLOCK);
        assert!(!delay.is_idle());

        // Switched off, the repeats ring out
        let (trail, _) = run_blocks(&mut delay, &off(), &vec![0.0; ms(100.0)], BLOCK);
        assert!(peak(&trail) > 0.01);
        run_blocks(&mut delay, &off(), &vec![0.0; ms(2500.0)], BLOCK);
        assert!(delay.is_idle());

        let (left, right) = run_blocks(&mut delay, &off(), &input, BLOCK);
        assert_eq!(left, input);
        assert_eq!(right, input);
        assert!(delay.is_idle());
    }

    #[test]
    fn test_off_from_the_start_leaves_both_channels_untouched() {
        let mut delay = new_delay(SAMPLE_RATE);
        let (mut left, mut right) = (noise(0.5, 4800), sine(330.0, 0.4, SAMPLE_RATE, 4800));
        let (left_in, right_in) = (left.clone(), right.clone());
        delay.process(&off(), &mut left, &mut right);
        assert_eq!(left, left_in);
        assert_eq!(right, right_in);
    }

    #[test]
    fn test_mix_zero_is_transparent() {
        let input = guitar_di(SAMPLE_RATE)[..ms(1500.0)].to_vec();
        let (left, right) = run(&settings(120.0, 0.9, 0.0), &input, SAMPLE_RATE);
        assert_eq!(left, input);
        assert_eq!(right, input);
    }

    #[test]
    fn test_only_the_repeats_differ_between_the_channels() {
        let input = guitar_di(SAMPLE_RATE)[..ms(3000.0)].to_vec();
        let (left, right) = run(&settings(250.0, 0.5, 1.0), &input, SAMPLE_RATE);

        // Until the first repeat there is only the dry signal, the same on both sides
        let first = ms(250.0 - 5.0);
        assert_eq!(left[..first], input[..first]);
        assert_eq!(right[..first], input[..first]);

        let wet = |channel: &[f32]| -> Vec<f32> { channel.iter().zip(&input).map(|(out, dry)| out - dry).collect() };
        let (wet_left, wet_right) = (wet(&left), wet(&right));
        assert!(rms(&wet_left) > 0.01);
        assert!(correlation(&wet_left, &wet_right) < 0.9, "wet correlation {}", correlation(&wet_left, &wet_right));
        let balance = to_db(rms(&wet_left) / rms(&wet_right));
        assert!(balance.abs() < 1.0, "left against right {} dB", balance);
    }

    #[test]
    fn test_mono_sum_keeps_the_dry_level() {
        let input = guitar_di(SAMPLE_RATE)[..ms(3000.0)].to_vec();
        let (left, right) = run(&settings(250.0, 0.5, 1.0), &input, SAMPLE_RATE);
        let mono: Vec<f32> = left.iter().zip(&right).map(|(l, r)| 0.5 * (l + r)).collect();
        // The share of the dry signal in the sum
        let dry = dot(&mono, &input) / dot(&input, &input);
        assert!((dry - 1.0).abs() < 0.1, "dry signal in the mono sum: {}", dry);
        assert!(rms(&mono) >= rms(&input));
    }

    #[test]
    fn test_time_change_does_not_click() {
        // A click would be a step many times the largest one the sine and its repeats make
        let input = sine(220.0, 0.5, SAMPLE_RATE, ms(4000.0));
        for (from_ms, to_ms) in [(300.0, 450.0), (450.0, 300.0), (20.0, 1000.0), (1000.0, 20.0)] {
            let mut delay = new_delay(SAMPLE_RATE);
            let (mut left, mut right) = run_blocks(&mut delay, &settings(from_ms, 0.5, 1.0), &input[..ms(2000.0)], BLOCK);
            let after = run_blocks(&mut delay, &settings(to_ms, 0.5, 1.0), &input[ms(2000.0)..], BLOCK);
            left.extend(after.0);
            right.extend(after.1);
            // The dry sine, and repeats of twice its level a fifth up
            let limit = 0.5 * (1.0 + 2.0 * 1.6) * std::f32::consts::TAU * 220.0 / SAMPLE_RATE;
            for channel in [&left, &right] {
                let step = largest_step(&channel[ms(1999.0)..]);
                assert!(step < limit, "{} to {} ms: step {} against {}", from_ms, to_ms, step, limit);
            }
        }
    }

    #[test]
    fn test_time_change_ends_at_the_new_time() {
        let mut delay = new_delay(SAMPLE_RATE);
        run_blocks(&mut delay, &settings(400.0, 0.0, 1.0), &noise(0.3, ms(500.0)), BLOCK);
        run_blocks(&mut delay, &settings(100.0, 0.0, 1.0), &noise(0.3, ms(3000.0)), BLOCK);
        assert_eq!(delay.time, 0.1 * SAMPLE_RATE as f64);
    }

    #[test]
    fn test_toggling_does_not_click_and_leaves_trails() {
        let input = sine(220.0, 0.5, SAMPLE_RATE, ms(1500.0));
        let on = settings(200.0, 0.5, 1.0);
        let off = DelaySettings { on: false, ..on };
        let mut delay = new_delay(SAMPLE_RATE);

        let mut output = Vec::new();
        for (index, block) in input.chunks(BLOCK).enumerate() {
            // Off and on again a few times while the sine goes on
            let now = if (index * BLOCK / ms(300.0)) % 2 == 0 { &on } else { &off };
            output.extend(run_blocks(&mut delay, now, block, BLOCK).0);
        }
        // Dry and repeats in phase would be the loudest a steady state gets
        let limit = 0.5 * 3.0 * std::f32::consts::TAU * 220.0 / SAMPLE_RATE;
        assert!(largest_step(&output) < limit, "step {} against {}", largest_step(&output), limit);

        // The repeats of what was played go on after the switch
        let mut delay = new_delay(SAMPLE_RATE);
        run_blocks(&mut delay, &on, &input, BLOCK);
        let (trail, _) = run_blocks(&mut delay, &off, &vec![0.0; ms(700.0)], BLOCK);
        assert!(peak(&trail[ms(100.0)..ms(200.0)]) > 0.2);
        assert!(peak(&trail[ms(500.0)..]) > 0.02);
    }

    #[test]
    fn test_output_does_not_depend_on_block_size() {
        // Playing, a silence long enough to fall idle in, and playing again
        let mut input = guitar_di(SAMPLE_RATE)[..ms(1000.0)].to_vec();
        input.extend(vec![0.0; ms(1400.0)]);
        input.extend(noise(0.3, ms(300.0)));
        let setting = settings(35.0, 0.3, 0.8);

        let whole = run_blocks(&mut new_delay(SAMPLE_RATE), &setting, &input, input.len());
        for block in [1, 7, 64, 500] {
            let mut delay = new_delay(SAMPLE_RATE);
            assert!(run_blocks(&mut delay, &setting, &input, block) == whole, "block of {}", block);
            assert!(!delay.is_idle());
        }
    }

    #[test]
    fn test_output_is_bounded_at_all_sample_rates() {
        for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            for time_ms in [TIME_MIN_MS, TIME_MAX_MS] {
                // Full scale, and a square at a frequency the short loop is in tune with
                let mut signal = noise(1.0, (sample_rate * 1.5) as usize);
                let period = (time_ms * 0.001 * sample_rate) as usize;
                signal.extend((0..(sample_rate * 2.5) as usize).map(|i| if (i / period) % 2 == 0 { 1.0 } else { -1.0 }));
                let (left, right) = run(&settings(time_ms, 1.0, 1.0), &signal, sample_rate);
                for channel in [&left, &right] {
                    assert!(channel.iter().all(|sample| sample.is_finite()));
                    assert!(peak(channel) < 4.0, "{} Hz, {} ms: peak {}", sample_rate, time_ms, peak(channel));
                }
            }
        }
    }

    #[test]
    fn test_nothing_is_allocated_after_the_sample_rate_is_set() {
        for sample_rate in [44100.0, 192000.0] {
            let mut delay = new_delay(sample_rate);
            assert!(delay.lines[0].len() as f32 > 1.004 * sample_rate);
            let layout = |delay: &Delay| delay.lines.iter().map(|line| (line.as_ptr() as usize, line.len(), line.capacity())).collect::<Vec<_>>();
            let before = layout(&delay);
            let input = noise(0.5, 48_000);
            for (index, block) in input.chunks(BLOCK).enumerate() {
                let setting = DelaySettings {
                    on: (index / 100) % 2 == 0,
                    ..settings([20.0, 1000.0, 333.0][(index / 60) % 3], 0.9, 1.0)
                };
                run_blocks(&mut delay, &setting, block, BLOCK);
            }
            delay.reset();
            assert_eq!(layout(&delay), before);
        }
    }

    #[test]
    fn test_reset_starts_over() {
        let mut delay = new_delay(SAMPLE_RATE);
        let input = guitar_di(SAMPLE_RATE)[..ms(800.0)].to_vec();
        let setting = settings(120.0, 0.7, 1.0);
        let first = run_blocks(&mut delay, &setting, &input, BLOCK);
        delay.reset();
        assert!(delay.is_idle());
        assert!(run_blocks(&mut delay, &setting, &input, BLOCK) == first);
    }

    /// Median time to process one block, in microseconds
    pub(crate) fn block_time_us(delay: &mut Delay, settings: &DelaySettings, input: &[f32]) -> f64 {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        let mut times: Vec<f64> = left
            .chunks_mut(BLOCK)
            .zip(right.chunks_mut(BLOCK))
            .map(|(l, r)| {
                let start = Instant::now();
                delay.process(settings, l, r);
                start.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[times.len() / 2]
    }

    #[test]
    fn test_tail_dies_to_exact_silence_and_is_not_slower() {
        // Denormal numbers in the tail would make its blocks many times slower
        let mut delay = new_delay(SAMPLE_RATE);
        let setting = settings(20.0, 0.5, 1.0);
        let playing = block_time_us(&mut delay, &setting, &noise(0.5, ms(250.0)));
        let silence = vec![0.0; ms(300.0)];
        // 60 dB down every 200 ms: in the last of these the tail is far below any sound
        let mut slowest: f64 = 0.0;
        for _ in 0..3 {
            slowest = slowest.max(block_time_us(&mut delay, &setting, &silence));
        }
        assert!(slowest < playing * SLOWED_DOWN, "{:.1} us per block playing, {:.1} us in the tail", playing, slowest);

        let (left, right) = run_blocks(&mut delay, &setting, &vec![0.0; ms(1200.0)], BLOCK);
        assert!(delay.is_idle());
        assert!(left[ms(1100.0)..].iter().chain(&right[ms(1100.0)..]).all(|&sample| sample == 0.0));
    }

    /// Samples from the start of `silence` until the delay is idle, and what it put out
    fn ring_out(delay: &mut Delay, setting: &DelaySettings, silence: usize) -> (usize, Vec<f32>) {
        let (mut left, mut right) = (vec![0.0; silence], vec![0.0; silence]);
        for index in 0..silence {
            delay.process(setting, &mut left[index..index + 1], &mut right[index..index + 1]);
            if delay.is_idle() {
                return (index + 1, left);
            }
        }
        panic!("Not idle after {} samples", silence);
    }

    #[test]
    fn test_idle_as_soon_as_the_last_repeat_is_over() {
        // No feedback: one repeat. It is heard in full, and the delay is idle right after
        // it, not a whole line later
        for time_ms in [20.0, 250.0, 1000.0] {
            let mut delay = new_delay(SAMPLE_RATE);
            let setting = settings(time_ms, 0.0, 1.0);
            let burst = noise(0.5, ms(15.0));
            run_blocks(&mut delay, &setting, &burst, BLOCK);
            assert!(!delay.is_idle());

            let (idle_at, left) = ring_out(&mut delay, &setting, ms(2000.0));
            let repeat = &left[ms(time_ms - 15.0) - 16..ms(time_ms) + 16];
            assert!(rms(repeat) > 0.1, "{} ms: the repeat at {}", time_ms, rms(repeat));
            assert!(idle_at >= ms(time_ms), "{} ms: idle after {} samples", time_ms, idle_at);
            assert!(idle_at < ms(time_ms + 40.0), "{} ms: idle after {} samples", time_ms, idle_at);
            // What was still coming out when it stopped is nothing
            assert!(peak(&left[idle_at - 48..idle_at]) < 2.0 * SILENCE, "{} ms: cut at {}", time_ms, peak(&left[idle_at - 48..idle_at]));
        }
    }

    #[test]
    fn test_idle_waits_for_a_time_dial_that_is_on_its_way_up() {
        // Idle only when the longer delay the dial is moving to has nothing left either
        let mut delay = new_delay(SAMPLE_RATE);
        let burst = noise(0.5, ms(15.0));
        run_blocks(&mut delay, &settings(20.0, 0.0, 1.0), &burst, BLOCK);
        let longer = settings(200.0, 0.0, 1.0);
        let (idle_at, _) = ring_out(&mut delay, &longer, ms(2000.0));
        assert!(idle_at >= ms(200.0), "Idle after {} samples", idle_at);
    }

    #[test]
    fn test_what_is_left_in_the_lines_at_idle_is_never_heard() {
        // A short delay falls idle with older repeats still far back in the lines. A long
        // time setting afterwards reads back there: it must find silence, as a new delay does
        let mut delay = new_delay(SAMPLE_RATE);
        let short = settings(20.0, 0.6, 1.0);
        run_blocks(&mut delay, &short, &noise(0.5, ms(600.0)), BLOCK);
        ring_out(&mut delay, &short, ms(3000.0));
        assert!(delay.lines.iter().flatten().any(|&sample| sample != 0.0));

        let mut input = noise(0.3, ms(50.0));
        input.resize(ms(2500.0), 0.0);
        for time_ms in [1000.0, 300.0] {
            let long = settings(time_ms, 0.5, 1.0);
            let expected = run_blocks(&mut new_delay(SAMPLE_RATE), &long, &input, BLOCK);
            assert!(run_blocks(&mut delay, &long, &input, BLOCK) == expected, "{} ms", time_ms);
            ring_out(&mut delay, &long, ms(20_000.0));
        }

        // And a time dial that moves while the delay plays gets no further back than what
        // was played since
        let moving = run_blocks(&mut delay, &short, &input[..ms(100.0)], BLOCK);
        let mut fresh = new_delay(SAMPLE_RATE);
        assert!(moving == run_blocks(&mut fresh, &short, &input[..ms(100.0)], BLOCK));
        let long = settings(1000.0, 0.5, 1.0);
        let silence = vec![0.0; ms(3000.0)];
        assert!(run_blocks(&mut delay, &long, &silence, BLOCK) == run_blocks(&mut fresh, &long, &silence, BLOCK));
    }
}
