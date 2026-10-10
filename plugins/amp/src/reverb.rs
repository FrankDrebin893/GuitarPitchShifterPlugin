//! Stereo reverb behind the delay: a smooth, dark tail that leaves the attack and the low
//! notes of the guitar alone

use crate::delay::{read_cubic, SILENCE};
use crate::dsp::filters::{Biquad, BiquadCoeffs, ANTI_DENORMAL};
use crate::dsp::smoothing_coeff;
use std::f64::consts::TAU;

pub const DECAY_MIN_S: f32 = 0.3;
pub const DECAY_MAX_S: f32 = 6.0;

const LINES: usize = 8;
// The delay lines of the network. Unrelated lengths, made mutually prime in samples, so
// their echoes never line up. The shortest is the pre-delay: nothing of the tail is heard
// before it, which keeps the attack of the dry signal clear. Long lines cost nothing and
// give the tail more resonances, closer together, which is what keeps it from ringing
const LINE_MS: [f32; LINES] = [15.1, 27.7, 41.3, 56.9, 73.1, 91.9, 111.7, 133.3];

// Every second line is read at a place that drifts slowly back and forth by this much, each
// at its own rate. The resonances of the network move with it, so none stands out as a tone
const MOD_DEPTH_MS: f32 = 0.3;
const MOD_HZ: [f32; LINES / 2] = [0.37, 0.53, 0.71, 0.89];
const MOD_START: [f32; LINES / 2] = [0.0, 0.25, 0.5, 0.75];

// Allpasses in front of the network: they smear a pick attack into a burst, so the lines
// are fed something dense and do not answer a palm mute with single echoes. The longest
// rings for 0.15 s by itself, which is why they are no longer: a short decay stays short
const DIFFUSER_MS: [f32; 6] = [1.3, 2.3, 3.7, 5.9, 8.3, 11.3];
const DIFFUSER_GAIN: f32 = 0.6;

// Only what is above this goes into the reverb: low notes stay tight
const LOW_CUT_HZ: f32 = 150.0;

// The decay time is the one at this frequency, in the middle of what a guitar has
const DECAY_HZ: f64 = 1000.0;
// The lows may last this much longer than that, and no more. With a short decay the
// low-pass below takes so much at `DECAY_HZ` that making up for all of it would leave the
// lows ringing on
const LOW_STRETCH: f64 = 1.12;
// At this frequency the tail lasts this share of the decay time, and less above
const DAMPING_HZ: f64 = 5000.0;
const DAMPING_RATIO: f64 = 0.5;

// Which way round the input goes into each line, and each line into the two outputs. The
// output patterns have nothing in common, so left and right are unrelated; neither is a
// pattern the mixing of the lines would gather into one line
const INPUT_SIGNS: [f32; LINES] = [1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0];
const LEFT_SIGNS: [f32; LINES] = [1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0];
const RIGHT_SIGNS: [f32; LINES] = [1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, -1.0];

// Level of the tail at mix 1.0, at a decay of `LEVEL_REFERENCE_S`: about as loud as the
// playing itself. A longer tail holds more energy; turning it down by the fourth root of
// the decay takes back half of that (in dB), so the Decay dial is not a volume dial as well
const WET_GAIN: f32 = 0.19;
const LEVEL_REFERENCE_S: f32 = 2.0;

// The decay is read once per this many samples and the gains move in straight lines between
const TICK: usize = 32;
const DECAY_SMOOTH_MS: f32 = 60.0;
// In seconds
const DECAY_SETTLED: f32 = 1e-3;
// How fast the mix follows its dial
const DIAL_SMOOTH_MS: f32 = 20.0;
const DIAL_SETTLED: f32 = 1e-5;
// Fade of the input to the network when the reverb is switched
const FADE_MS: f32 = 10.0;

/// What the knobs say, read once per block. The reverb smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbSettings {
    /// Off lets the tail ring out
    pub on: bool,
    /// Seconds for the tail to fall by 60 dB, `DECAY_MIN_S` to `DECAY_MAX_S`
    pub decay_s: f32,
    /// Level of the tail, 0.0 to 1.0. The dry signal stays as it is
    pub mix: f32,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            on: false,
            decay_s: 1.5,
            mix: 0.2,
        }
    }
}

/// Below -100 dBFS everything that is kept is kept as an exact zero: nothing is left to turn
/// into denormal numbers, and once every buffer is zeros the reverb is idle
fn audible(sample: f32) -> f32 {
    if sample.abs() >= SILENCE {
        sample
    } else {
        0.0
    }
}

struct Diffuser {
    buffer: Vec<f32>,
    position: usize,
}

impl Diffuser {
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.position];
        let output = delayed - DIFFUSER_GAIN * input;
        self.buffer[self.position] = audible(input + DIFFUSER_GAIN * output);
        self.position = if self.position + 1 == self.buffer.len() { 0 } else { self.position + 1 };
        output
    }
}

struct Line {
    buffer: Vec<f32>,
    write: usize,
    // In samples. A line that stays put is exactly this long
    delay: usize,
}

impl Line {
    fn push(&mut self, sample: f32) {
        self.buffer[self.write] = sample;
        self.write = if self.write + 1 == self.buffer.len() { 0 } else { self.write + 1 };
    }
}

/// What the lines do per pass at one decay time
#[derive(Clone, Copy, PartialEq)]
struct Loop {
    // What is left after one pass, with the mixing's own scale in it
    gains: [f32; LINES],
    // Coefficients of the low-passes that make the highs die faster
    damping: [f32; LINES],
    // Level of the tail in the output
    level: f32,
}

impl Loop {
    const REST: Loop = Loop {
        gains: [0.0; LINES],
        damping: [1.0; LINES],
        level: 0.0,
    };
}

/// The lines mix into each other with equal weights and alternating signs, so each one
/// feeds all the others and the echoes multiply fast. Scaled by the square root of the
/// number of lines this keeps the energy as it is
fn mix_lines(lines: &mut [f32; LINES]) {
    let mut stride = 1;
    while stride < LINES {
        let mut start = 0;
        while start < LINES {
            for index in start..start + stride {
                let (a, b) = (lines[index], lines[index + stride]);
                lines[index] = a + b;
                lines[index + stride] = a - b;
            }
            start += 2 * stride;
        }
        stride *= 2;
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// The next length from `wanted` up that shares no factor with the ones taken
fn prime_to(wanted: usize, taken: &[usize]) -> usize {
    let mut len = wanted.max(2);
    while taken.iter().any(|&other| gcd(len, other) != 1) {
        len += 1;
    }
    len
}

/// A feedback delay network of eight lines behind four diffusers. The input is the average
/// of the channels; left and right of the tail are unrelated
pub struct Reverb {
    sample_rate: f32,
    low_cut: Biquad,
    diffusers: [Diffuser; DIFFUSER_MS.len()],
    lines: [Line; LINES],
    damping_state: [f32; LINES],
    // Where each moving line is in its drift, 0.0 to 1.0
    mod_phase: [f32; LINES / 2],
    mod_step: [f32; LINES / 2],
    mod_depth: f32,

    // The decay the lines are at, on its way to the setting
    decay_s: f32,
    current: Loop,
    target: Loop,
    step: Loop,
    ramping: bool,
    // False until the first tick after a rest: the decay then starts at the setting
    primed: bool,
    // Samples left until the decay is read again. Counted across blocks, so the output does
    // not depend on how the host cuts the stream into blocks
    until_tick: usize,
    decay_coeff: f32,

    mix: f32,
    dial_coeff: f32,
    // Share of the input that reaches the network: 1.0 on, 0.0 off
    feed: f32,
    feed_step: f32,

    // Samples in a row that only zeros went into the lines
    quiet_run: usize,
    // The longest buffer: after this many the diffusers and lines hold nothing but zeros
    idle_after: usize,
    // Everything is zeros and at rest
    idle: bool,
}

impl Reverb {
    pub fn new() -> Self {
        let mut reverb = Self {
            sample_rate: 44100.0,
            low_cut: Biquad::new(),
            diffusers: std::array::from_fn(|_| Diffuser {
                buffer: Vec::new(),
                position: 0,
            }),
            lines: std::array::from_fn(|_| Line {
                buffer: Vec::new(),
                write: 0,
                delay: 0,
            }),
            damping_state: [0.0; LINES],
            mod_phase: MOD_START,
            mod_step: [0.0; LINES / 2],
            mod_depth: 0.0,
            decay_s: DECAY_MIN_S,
            current: Loop::REST,
            target: Loop::REST,
            step: Loop::REST,
            ramping: false,
            primed: false,
            until_tick: 0,
            decay_coeff: 1.0,
            mix: 0.0,
            dial_coeff: 1.0,
            feed: 0.0,
            feed_step: 1.0,
            quiet_run: 0,
            idle_after: 0,
            idle: true,
        };
        reverb.set_sample_rate(44100.0);
        reverb
    }

    /// Allocates the delay lines. Not for the audio thread
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let samples = |time_ms: f32| (time_ms * 0.001 * sample_rate).round() as usize;

        let mut taken = Vec::new();
        for (diffuser, time_ms) in self.diffusers.iter_mut().zip(DIFFUSER_MS) {
            let len = prime_to(samples(time_ms), &taken);
            taken.push(len);
            diffuser.buffer = vec![0.0; len];
        }
        self.mod_depth = MOD_DEPTH_MS * 0.001 * sample_rate;
        for (index, (line, time_ms)) in self.lines.iter_mut().zip(LINE_MS).enumerate() {
            line.delay = prime_to(samples(time_ms), &taken);
            taken.push(line.delay);
            // A moving line needs room for its drift and for the interpolation
            let room = if index % 2 == 1 { self.mod_depth.ceil() as usize + 4 } else { 0 };
            line.buffer = vec![0.0; line.delay + room];
            // Written to once here, so the memory is there before the audio thread needs it
            line.buffer.fill(1.0);
            std::hint::black_box(&mut line.buffer[..]);
        }
        for (step, rate_hz) in self.mod_step.iter_mut().zip(MOD_HZ) {
            *step = rate_hz / sample_rate;
        }

        self.low_cut.set(BiquadCoeffs::highpass(LOW_CUT_HZ, std::f32::consts::FRAC_1_SQRT_2, sample_rate));
        self.decay_coeff = smoothing_coeff(DECAY_SMOOTH_MS, sample_rate / TICK as f32);
        self.dial_coeff = smoothing_coeff(DIAL_SMOOTH_MS, sample_rate);
        self.feed_step = 1.0 / (FADE_MS * 0.001 * sample_rate);
        // The diffusers are shorter than the lines, and go quiet before them
        self.idle_after = self.lines.iter().map(|line| line.buffer.len()).max().unwrap_or(0);
        self.reset();
    }

    /// Drops the tail. Does not allocate
    pub fn reset(&mut self) {
        for diffuser in &mut self.diffusers {
            diffuser.buffer.fill(0.0);
            diffuser.position = 0;
        }
        for line in &mut self.lines {
            line.buffer.fill(0.0);
            line.write = 0;
        }
        self.rest();
    }

    /// For when the buffers hold nothing but zeros
    fn rest(&mut self) {
        self.low_cut.reset();
        self.damping_state = [0.0; LINES];
        self.mod_phase = MOD_START;
        self.ramping = false;
        self.primed = false;
        self.until_tick = 0;
        self.quiet_run = 0;
        self.idle = true;
    }

    /// True when there is nothing left to hear of earlier input: the next block comes out as
    /// it goes in, unless the reverb is on and the block has sound in it
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// Address and capacity of every buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<(usize, usize)> {
        let lines = self.lines.iter().map(|line| &line.buffer);
        let diffusers = self.diffusers.iter().map(|diffuser| &diffuser.buffer);
        lines.chain(diffusers).map(|buffer| (buffer.as_ptr() as usize, buffer.capacity())).collect()
    }

    /// Gains, damping and level for a decay time
    fn design(&self, decay_s: f32) -> Loop {
        let mut design = Loop::REST;
        let cosine = (TAU * DAMPING_HZ.min(0.45 * self.sample_rate as f64) / self.sample_rate as f64).cos();
        let decay_cosine = (TAU * DECAY_HZ / self.sample_rate as f64).cos();
        for (index, line) in self.lines.iter().enumerate() {
            let pass_s = line.delay as f64 / self.sample_rate as f64;
            // What a pass leaves at `DECAY_HZ`, where the decay time is the setting
            let wanted = 10.0f64.powf(-3.0 * pass_s / decay_s as f64);
            // A one-pole low-pass makes the highs die faster: its pole is where its gain at
            // `DAMPING_HZ` is `high`. It takes a little away at `DECAY_HZ` as well, which the
            // gain of the line gives back, as far as the lows can take it; a few rounds
            // settle the two
            let most = wanted.powf(1.0 / LOW_STRETCH);
            let (mut gain, mut pole) = (wanted, 0.0);
            for _ in 0..4 {
                let high = (wanted.powf(1.0 / DAMPING_RATIO) / gain).min(1.0 - 1e-9);
                let (lost, lost_at_cosine) = (1.0 - high * high, 1.0 - high * high * cosine);
                pole = (lost_at_cosine - (lost_at_cosine * lost_at_cosine - lost * lost).max(0.0).sqrt()) / lost;
                let at_decay_hz = (1.0 - pole) / (1.0 - 2.0 * pole * decay_cosine + pole * pole).sqrt();
                gain = (wanted / at_decay_hz).min(most);
            }
            design.gains[index] = (gain / (LINES as f64).sqrt()) as f32;
            design.damping[index] = (1.0 - pole) as f32;
        }
        design.level = WET_GAIN * (LEVEL_REFERENCE_S / decay_s).powf(0.25);
        design
    }

    /// Moves the decay a step towards the setting, and the lines with it
    fn tick(&mut self, decay_s: f32) {
        if self.ramping {
            self.current = self.target;
            self.ramping = false;
        }
        if !self.primed {
            self.decay_s = decay_s;
            self.current = self.design(decay_s);
            self.target = self.current;
            self.primed = true;
        } else if self.decay_s != decay_s {
            self.decay_s += self.decay_coeff * (decay_s - self.decay_s);
            if (decay_s - self.decay_s).abs() < DECAY_SETTLED {
                self.decay_s = decay_s;
            }
            self.target = self.design(self.decay_s);
            let per_sample = 1.0 / TICK as f32;
            for index in 0..LINES {
                self.step.gains[index] = (self.target.gains[index] - self.current.gains[index]) * per_sample;
                self.step.damping[index] = (self.target.damping[index] - self.current.damping[index]) * per_sample;
            }
            self.step.level = (self.target.level - self.current.level) * per_sample;
            self.ramping = true;
        }
    }

    /// Adds the tail to a block in place. The network is fed the average of the channels
    pub fn process(&mut self, settings: &ReverbSettings, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        let len = left.len().min(right.len());
        let decay_s = settings.decay_s.clamp(DECAY_MIN_S, DECAY_MAX_S);
        let mix = settings.mix.clamp(0.0, 1.0);

        let mut start = 0;
        while start < len {
            if self.idle {
                // Nothing to add until the reverb is on and there is something to answer
                if !settings.on {
                    return;
                }
                let Some(first) = (start..len).find(|&index| (0.5 * (left[index] + right[index])).abs() >= SILENCE) else {
                    return;
                };
                // Nothing is heard yet, so there is nothing to smooth
                self.mix = mix;
                self.feed = 1.0;
                self.idle = false;
                start = first;
            }
            start = self.run(settings.on, decay_s, mix, &mut left[..len], &mut right[..len], start);
        }
    }

    /// Processes from `start` to the end of the block or to where the reverb falls idle.
    /// Returns where it stopped
    fn run(&mut self, on: bool, decay_s: f32, mix: f32, left: &mut [f32], right: &mut [f32], start: usize) -> usize {
        let feed_target = if on { 1.0 } else { 0.0 };

        for index in start..left.len() {
            if self.until_tick == 0 {
                self.tick(decay_s);
                self.until_tick = TICK;
            }
            self.until_tick -= 1;
            if self.ramping {
                for line in 0..LINES {
                    self.current.gains[line] += self.step.gains[line];
                    self.current.damping[line] += self.step.damping[line];
                }
                self.current.level += self.step.level;
            }
            if self.feed != feed_target {
                self.feed = (self.feed + self.feed_step.copysign(feed_target - self.feed)).clamp(0.0, 1.0);
            }
            if self.mix != mix {
                self.mix += self.dial_coeff * (mix - self.mix);
                if (mix - self.mix).abs() < DIAL_SETTLED {
                    self.mix = mix;
                }
            }

            let input = 0.5 * (left[index] + right[index]) * self.feed;
            let mut diffused = self.low_cut.process(input as f64) as f32;
            for diffuser in &mut self.diffusers {
                diffused = diffuser.process(diffused);
            }

            // What comes out of the lines: the even ones stay put, the odd ones drift
            let mut reads = [0.0f32; LINES];
            for pair in 0..LINES / 2 {
                let still = &self.lines[2 * pair];
                reads[2 * pair] = still.buffer[still.write];

                let phase = self.mod_phase[pair] + self.mod_step[pair];
                self.mod_phase[pair] = if phase >= 1.0 { phase - 1.0 } else { phase };
                // A parabola per half turn, close to a sine and as smooth where it turns
                let turn = 2.0 * self.mod_phase[pair] - 1.0;
                let drift = 4.0 * turn * (1.0 - turn.abs());
                let moving = &self.lines[2 * pair + 1];
                let delay = moving.delay as f32 + self.mod_depth * drift;
                let whole = delay as usize;
                reads[2 * pair + 1] = read_cubic(&moving.buffer, moving.write, whole, delay - whole as f32);
            }

            let (mut tail_left, mut tail_right) = (0.0, 0.0);
            let mut fed = [0.0f32; LINES];
            for line in 0..LINES {
                tail_left += LEFT_SIGNS[line] * reads[line];
                tail_right += RIGHT_SIGNS[line] * reads[line];
                let state = &mut self.damping_state[line];
                *state += self.current.damping[line] * (reads[line] - *state) + ANTI_DENORMAL;
                fed[line] = *state * self.current.gains[line];
            }
            mix_lines(&mut fed);

            let mut quiet = true;
            for line in 0..LINES {
                let sample = audible(fed[line] + INPUT_SIGNS[line] * diffused);
                quiet &= sample == 0.0;
                self.lines[line].push(sample);
            }

            let level = self.mix * self.current.level;
            left[index] += level * tail_left;
            right[index] += level * tail_right;

            self.quiet_run = if quiet { self.quiet_run + 1 } else { 0 };
            if self.quiet_run >= self.idle_after {
                self.rest();
                return index + 1;
            }
        }
        left.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delay::tests as delay_tests;
    use crate::delay::{Delay, DelaySettings, FEEDBACK_MAX, TIME_MAX_MS, TIME_MIN_MS};
    use crate::test_util::*;
    use std::time::Instant;

    const SAMPLE_RATE: f32 = 48000.0;
    const BLOCK: usize = 64;

    fn new_reverb(sample_rate: f32) -> Reverb {
        let mut reverb = Reverb::new();
        reverb.set_sample_rate(sample_rate);
        reverb
    }

    fn settings(decay_s: f32, mix: f32) -> ReverbSettings {
        ReverbSettings { on: true, decay_s, mix }
    }

    fn off() -> ReverbSettings {
        ReverbSettings {
            on: false,
            ..settings(0.5, 1.0)
        }
    }

    /// Runs a mono signal through in blocks. Returns left and right
    fn run_blocks(reverb: &mut Reverb, settings: &ReverbSettings, input: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
            reverb.process(settings, l, r);
        }
        (left, right)
    }

    fn run(settings: &ReverbSettings, input: &[f32], sample_rate: f32) -> (Vec<f32>, Vec<f32>) {
        run_blocks(&mut new_reverb(sample_rate), settings, input, BLOCK)
    }

    fn noise(level: f32, len: usize) -> Vec<f32> {
        let mut noise = Noise::new(5);
        (0..len).map(|_| level * noise.next()).collect()
    }

    fn seconds(time_s: f32, sample_rate: f32) -> usize {
        (time_s * sample_rate) as usize
    }

    fn without(output: &[f32], dry: &[f32]) -> Vec<f32> {
        output.iter().zip(dry).map(|(out, dry)| out - dry).collect()
    }

    /// The tail of a single full-scale sample at mix 1.0, left and right
    fn impulse_response(decay_s: f32, sample_rate: f32, len_s: f32) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![0.0; seconds(len_s, sample_rate)];
        input[0] = 1.0;
        let (left, right) = run(&settings(decay_s, 1.0), &input, sample_rate);
        (without(&left, &input), without(&right, &input))
    }

    /// Reverb time in the octave around a frequency
    fn decay_at(response: &[f32], sample_rate: f32, centre_hz: f32) -> f32 {
        decay_time_s(&octave_band(response, sample_rate, centre_hz), sample_rate, -5.0, -25.0)
    }

    #[test]
    fn test_line_lengths_share_no_factor() {
        for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            let reverb = new_reverb(sample_rate);
            let lengths: Vec<usize> = reverb.lines.iter().map(|line| line.delay).chain(reverb.diffusers.iter().map(|d| d.buffer.len())).collect();
            for (index, &a) in lengths.iter().enumerate() {
                for &b in &lengths[index + 1..] {
                    assert_eq!(gcd(a, b), 1, "{} and {} at {} Hz", a, b, sample_rate);
                }
            }
            for (line, time_ms) in reverb.lines.iter().zip(LINE_MS) {
                let wanted = time_ms * 0.001 * sample_rate;
                assert!((line.delay as f32 - wanted).abs() < 0.01 * wanted + 8.0);
            }
        }
    }

    #[test]
    fn test_mixing_keeps_the_energy() {
        let mut lines = [0.3, -0.2, 0.9, 0.1, -0.7, 0.4, 0.0, 0.5];
        let before: f32 = lines.iter().map(|x| x * x).sum();
        mix_lines(&mut lines);
        let after: f32 = lines.iter().map(|x| x * x).sum::<f32>() / LINES as f32;
        assert!((after - before).abs() < 1e-5);
    }

    #[test]
    fn test_decay_time_follows_the_setting() {
        for sample_rate in [44100.0, 96000.0] {
            for decay_s in [0.5, 2.0] {
                let (left, right) = impulse_response(decay_s, sample_rate, 0.75 * decay_s + 0.3);
                for channel in [&left, &right] {
                    let measured = decay_at(channel, sample_rate, 1000.0);
                    assert!((measured / decay_s - 1.0).abs() < 0.2, "{} Hz, set {} s: {} s", sample_rate, decay_s, measured);
                }
            }
        }
    }

    #[test]
    fn test_highs_die_faster() {
        let (left, _) = impulse_response(2.0, SAMPLE_RATE, 1.8);
        let (mid, high) = (decay_at(&left, SAMPLE_RATE, 1000.0), decay_at(&left, SAMPLE_RATE, 5000.0));
        assert!((0.35..0.65).contains(&(high / mid)), "{} s at 1 kHz, {} s at 5 kHz", mid, high);
    }

    #[test]
    fn test_tail_has_no_lows_and_starts_after_the_attack() {
        // An octave of noise, and how loud its tail is against it
        let tail_of = |centre_hz: f32| {
            let input = octave_band(&noise(0.5, seconds(2.0, SAMPLE_RATE)), SAMPLE_RATE, centre_hz);
            let (left, _) = run(&settings(1.0, 1.0), &input, SAMPLE_RATE);
            let from = seconds(1.0, SAMPLE_RATE);
            to_db(rms(&without(&left, &input)[from..]) / rms(&input[from..]))
        };
        let (low, mid) = (tail_of(62.5), tail_of(500.0));
        assert!(low < mid - 8.0, "{} dB around 62.5 Hz, {} dB around 500 Hz", low, mid);

        let (left, right) = impulse_response(1.0, SAMPLE_RATE, 0.2);
        let start = seconds(0.010, SAMPLE_RATE);
        assert!(peak(&left[..start]) == 0.0 && peak(&right[..start]) == 0.0);
        assert!(peak(&left[start..seconds(0.03, SAMPLE_RATE)]) > 0.0);
    }

    #[test]
    fn test_tail_is_dense_and_does_not_ring() {
        let (left, _) = impulse_response(2.0, SAMPLE_RATE, 1.2);
        let window = seconds(0.01, SAMPLE_RATE);
        // 3.0 is noise. Single echoes standing out push it up, a tail that rings on a few
        // tones pulls it down towards 1.5
        let mut values: Vec<f32> = left[seconds(0.15, SAMPLE_RATE)..].chunks_exact(window).map(kurtosis).collect();
        values.sort_by(|a, b| a.total_cmp(b));
        let (lowest, median, highest) = (values[0], values[values.len() / 2], values[values.len() - 1]);
        assert!((2.4..3.6).contains(&median), "median {}", median);
        assert!(lowest > 1.9 && highest < 8.0, "{} to {}", lowest, highest);
    }

    #[test]
    fn test_left_and_right_of_the_tail_differ() {
        let input = guitar_di(SAMPLE_RATE)[..seconds(3.0, SAMPLE_RATE)].to_vec();
        let (left, right) = run(&settings(2.0, 1.0), &input, SAMPLE_RATE);
        let (tail_left, tail_right) = (without(&left, &input), without(&right, &input));
        assert!(rms(&tail_left) > 0.01);
        let alike = correlation(&tail_left, &tail_right);
        assert!(alike.abs() < 0.3, "correlation {}", alike);
        let balance = to_db(rms(&tail_left) / rms(&tail_right));
        assert!(balance.abs() < 1.5, "left against right {} dB", balance);

        // The dry signal is the same on both sides: until the tail starts there is nothing else
        let first = input.iter().position(|sample| sample.abs() >= SILENCE).unwrap();
        let start = first + seconds(0.010, SAMPLE_RATE);
        assert!(first > seconds(0.1, SAMPLE_RATE));
        assert_eq!(left[..start], input[..start]);
        assert_eq!(right[..start], input[..start]);
    }

    #[test]
    fn test_mono_sum_keeps_the_dry_level() {
        let input = guitar_di(SAMPLE_RATE)[..seconds(3.0, SAMPLE_RATE)].to_vec();
        let (left, right) = run(&settings(2.0, 1.0), &input, SAMPLE_RATE);
        let mono: Vec<f32> = left.iter().zip(&right).map(|(l, r)| 0.5 * (l + r)).collect();
        let dry = dot(&mono, &input) / dot(&input, &input);
        // The tail of a held note is partly in step with the note itself
        assert!((dry - 1.0).abs() < 0.15, "dry signal in the mono sum: {}", dry);
        assert!(rms(&mono) >= rms(&input));
    }

    #[test]
    fn test_mix_one_is_wet_but_usable() {
        // The tail about as loud as the playing, at every decay
        let input = guitar_di(SAMPLE_RATE)[..seconds(6.0, SAMPLE_RATE)].to_vec();
        for decay_s in [DECAY_MIN_S, 2.0, DECAY_MAX_S] {
            let (left, _) = run(&settings(decay_s, 1.0), &input, SAMPLE_RATE);
            let level = to_db(rms(&without(&left, &input)) / rms(&input));
            assert!((-12.0..4.0).contains(&level), "decay {} s: tail at {} dB", decay_s, level);
        }
    }

    #[test]
    fn test_off_is_transparent_once_the_tail_has_died() {
        let mut reverb = new_reverb(SAMPLE_RATE);
        let input = noise(0.5, seconds(0.2, SAMPLE_RATE));
        run_blocks(&mut reverb, &settings(DECAY_MIN_S, 1.0), &input, BLOCK);
        assert!(!reverb.is_idle());

        // Switched off, the tail rings out: 120 dB at this decay is 0.6 s
        let (trail, _) = run_blocks(&mut reverb, &off(), &vec![0.0; seconds(0.1, SAMPLE_RATE)], BLOCK);
        assert!(peak(&trail[seconds(0.05, SAMPLE_RATE)..]) > 0.001);
        let (late, _) = run_blocks(&mut reverb, &off(), &vec![0.0; seconds(1.4, SAMPLE_RATE)], BLOCK);
        assert!(reverb.is_idle());
        assert!(late[seconds(1.3, SAMPLE_RATE)..].iter().all(|&sample| sample == 0.0));
        assert!(reverb.lines.iter().all(|line| line.buffer.iter().all(|&sample| sample == 0.0)));
        assert!(reverb.diffusers.iter().all(|diffuser| diffuser.buffer.iter().all(|&sample| sample == 0.0)));

        let (left, right) = run_blocks(&mut reverb, &off(), &input, BLOCK);
        assert_eq!(left, input);
        assert_eq!(right, input);
        assert!(reverb.is_idle());
    }

    #[test]
    fn test_off_from_the_start_leaves_both_channels_untouched() {
        let mut reverb = new_reverb(SAMPLE_RATE);
        let (mut left, mut right) = (noise(0.5, 4800), sine(330.0, 0.4, SAMPLE_RATE, 4800));
        let (left_in, right_in) = (left.clone(), right.clone());
        reverb.process(&off(), &mut left, &mut right);
        assert_eq!(left, left_in);
        assert_eq!(right, right_in);
    }

    #[test]
    fn test_mix_zero_is_transparent() {
        let input = guitar_di(SAMPLE_RATE)[..seconds(1.0, SAMPLE_RATE)].to_vec();
        let (left, right) = run(&settings(DECAY_MAX_S, 0.0), &input, SAMPLE_RATE);
        assert_eq!(left, input);
        assert_eq!(right, input);
    }

    #[test]
    fn test_tail_dies_to_exact_silence_and_is_not_slower() {
        // Denormal numbers in the tail would make its blocks many times slower
        let mut reverb = new_reverb(SAMPLE_RATE);
        let setting = settings(DECAY_MIN_S, 1.0);
        let playing = block_time_us(&mut reverb, &setting, &noise(0.5, seconds(0.25, SAMPLE_RATE)));
        // 100 dB at this decay is half a second: three of these end a little before
        let silence = vec![0.0; seconds(0.13, SAMPLE_RATE)];
        let mut slowest: f64 = 0.0;
        for _ in 0..3 {
            slowest = slowest.max(block_time_us(&mut reverb, &setting, &silence));
        }
        assert!(!reverb.is_idle());
        assert!(slowest < playing * 2.0, "{:.1} us per block playing, {:.1} us in the tail", playing, slowest);

        let (left, right) = run_blocks(&mut reverb, &setting, &vec![0.0; seconds(0.6, SAMPLE_RATE)], BLOCK);
        assert!(reverb.is_idle());
        let end = seconds(0.55, SAMPLE_RATE);
        assert!(left[end..].iter().chain(&right[end..]).all(|&sample| sample == 0.0));
    }

    #[test]
    fn test_decay_change_does_not_click() {
        let input = sine(440.0, 0.5, SAMPLE_RATE, seconds(3.0, SAMPLE_RATE));
        // The dry sine and the loudest a tail of it gets
        let limit = |output: &[f32]| 1.5 * peak(output) * std::f32::consts::TAU * 440.0 / SAMPLE_RATE;
        for (from_s, to_s) in [(DECAY_MIN_S, DECAY_MAX_S), (DECAY_MAX_S, DECAY_MIN_S), (1.0, 2.0)] {
            let mut reverb = new_reverb(SAMPLE_RATE);
            let half = input.len() / 2;
            let (mut output, _) = run_blocks(&mut reverb, &settings(from_s, 1.0), &input[..half], BLOCK);
            output.extend(run_blocks(&mut reverb, &settings(to_s, 1.0), &input[half..], BLOCK).0);
            let step = largest_step(&output[half - 100..]);
            assert!(step < limit(&output), "{} to {} s: step {} against {}", from_s, to_s, step, limit(&output));
            assert_eq!(reverb.decay_s, to_s);
            assert!(reverb.current == reverb.design(to_s));
        }
    }

    #[test]
    fn test_toggling_does_not_click_and_leaves_a_tail() {
        let input = sine(440.0, 0.5, SAMPLE_RATE, seconds(1.5, SAMPLE_RATE));
        let on = settings(1.5, 1.0);
        let off = ReverbSettings { on: false, ..on };
        let mut reverb = new_reverb(SAMPLE_RATE);
        let mut output = Vec::new();
        for (index, block) in input.chunks(BLOCK).enumerate() {
            let now = if (index * BLOCK / seconds(0.3, SAMPLE_RATE)) % 2 == 0 { &on } else { &off };
            output.extend(run_blocks(&mut reverb, now, block, BLOCK).0);
        }
        let limit = 1.5 * peak(&output) * std::f32::consts::TAU * 440.0 / SAMPLE_RATE;
        assert!(largest_step(&output) < limit, "step {} against {}", largest_step(&output), limit);

        let mut reverb = new_reverb(SAMPLE_RATE);
        run_blocks(&mut reverb, &on, &input, BLOCK);
        let (trail, _) = run_blocks(&mut reverb, &off, &vec![0.0; seconds(0.5, SAMPLE_RATE)], BLOCK);
        assert!(peak(&trail[seconds(0.3, SAMPLE_RATE)..]) > 0.01);
    }

    #[test]
    fn test_output_does_not_depend_on_block_size() {
        // Playing, a silence long enough to fall idle in, playing again, and a new decay
        let mut input = guitar_di(SAMPLE_RATE)[..seconds(0.6, SAMPLE_RATE)].to_vec();
        input.extend(vec![0.0; seconds(1.0, SAMPLE_RATE)]);
        input.extend(noise(0.3, seconds(0.4, SAMPLE_RATE)));
        let half = seconds(1.8, SAMPLE_RATE);
        let pass = |block: usize| {
            let mut reverb = new_reverb(SAMPLE_RATE);
            let (mut left, mut right) = run_blocks(&mut reverb, &settings(DECAY_MIN_S, 0.8), &input[..half], block);
            let (l, r) = run_blocks(&mut reverb, &settings(1.5, 0.5), &input[half..], block);
            left.extend(l);
            right.extend(r);
            assert!(!reverb.is_idle());
            (left, right)
        };
        let whole = pass(input.len());
        for block in [1, 7, 64, 500] {
            assert!(pass(block) == whole, "block of {}", block);
        }
    }

    #[test]
    fn test_output_is_bounded_at_all_sample_rates() {
        for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            for decay_s in [DECAY_MIN_S, DECAY_MAX_S] {
                let mut signal = noise(1.0, seconds(0.5, sample_rate));
                signal.extend((0..seconds(0.5, sample_rate)).map(|i| if (i / 100) % 2 == 0 { 1.0 } else { -1.0 }));
                let (left, right) = run(&settings(decay_s, 1.0), &signal, sample_rate);
                for channel in [&left, &right] {
                    assert!(channel.iter().all(|sample| sample.is_finite()));
                    assert!(peak(channel) < 8.0, "{} Hz, {} s: peak {}", sample_rate, decay_s, peak(channel));
                }
            }
        }
    }

    #[test]
    fn test_nothing_is_allocated_after_the_sample_rate_is_set() {
        let layout = |reverb: &Reverb| {
            let lines = reverb.lines.iter().map(|line| &line.buffer);
            let diffusers = reverb.diffusers.iter().map(|diffuser| &diffuser.buffer);
            lines.chain(diffusers).map(|buffer| (buffer.as_ptr() as usize, buffer.len(), buffer.capacity())).collect::<Vec<_>>()
        };
        let mut reverb = new_reverb(192000.0);
        let before = layout(&reverb);
        let input = noise(0.5, 48_000);
        for (index, block) in input.chunks(BLOCK).enumerate() {
            let setting = ReverbSettings {
                on: (index / 100) % 2 == 0,
                ..settings([DECAY_MIN_S, DECAY_MAX_S, 1.0][(index / 60) % 3], 1.0)
            };
            run_blocks(&mut reverb, &setting, block, BLOCK);
        }
        reverb.reset();
        assert_eq!(layout(&reverb), before);
    }

    #[test]
    fn test_reset_starts_over() {
        let mut reverb = new_reverb(SAMPLE_RATE);
        let input = guitar_di(SAMPLE_RATE)[..seconds(0.5, SAMPLE_RATE)].to_vec();
        let first = run_blocks(&mut reverb, &settings(2.0, 1.0), &input, BLOCK);
        reverb.reset();
        assert!(reverb.is_idle());
        assert!(run_blocks(&mut reverb, &settings(2.0, 1.0), &input, BLOCK) == first);
    }

    /// Median time to process one block, in microseconds
    fn block_time_us(reverb: &mut Reverb, settings: &ReverbSettings, input: &[f32]) -> f64 {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        let mut times: Vec<f64> = left
            .chunks_mut(BLOCK)
            .zip(right.chunks_mut(BLOCK))
            .map(|(l, r)| {
                let start = Instant::now();
                reverb.process(settings, l, r);
                start.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[times.len() / 2]
    }

    /// Delay, then reverb, as the chain runs them
    fn run_both(delay: &mut Delay, reverb: &mut Reverb, delay_settings: &DelaySettings, reverb_settings: &ReverbSettings, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            delay.process(delay_settings, l, r);
            reverb.process(reverb_settings, l, r);
        }
        (left, right)
    }

    fn new_delay(sample_rate: f32) -> Delay {
        let mut delay = Delay::new();
        delay.set_sample_rate(sample_rate);
        delay
    }

    /// Level at one frequency, by correlation with a sine and a cosine
    fn level_at_fast(signal: &[f32], sample_rate: f32, freq_hz: f64) -> f64 {
        let step = TAU * freq_hz / sample_rate as f64;
        let (mut sine, mut cosine) = (0.0, 0.0);
        for (index, sample) in signal.iter().enumerate() {
            let (s, c) = (index as f64 * step).sin_cos();
            sine += *sample as f64 * s;
            cosine += *sample as f64 * c;
        }
        2.0 * sine.hypot(cosine) / signal.len() as f64
    }

    /// Prints what the delay and the reverb do, and what they cost:
    ///   cargo test -p amp --release effects_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn effects_report() {
        let at = |time_ms: f32| seconds(time_ms * 0.001, SAMPLE_RATE);
        let di = guitar_di(SAMPLE_RATE);

        println!("\n== DELAY ==");
        println!("\nEcho time, single sample in, feedback 0.5, mix 1.0 (ms; the lines feed each other)");
        println!("{:>8} {:>19} {:>19} {:>19} {:>19}", "set", "1st left / right", "2nd left / right", "3rd left / right", "4th left / right");
        for time_ms in [TIME_MIN_MS, 125.0, 350.0, 500.0, TIME_MAX_MS] {
            let mut input = vec![0.0; at(time_ms * 4.6)];
            input[0] = 1.0;
            let (left, right) = delay_tests::run(&delay_tests::settings(time_ms, 0.5, 1.0), &input, SAMPLE_RATE);
            let mut row = format!("{:>8.1}", time_ms);
            for repeat in 1..=4 {
                let window = at(time_ms * (repeat as f32 - 0.5))..at(time_ms * (repeat as f32 + 0.5));
                let found = |channel: &[f32]| {
                    let slice = &channel[window.clone()];
                    let index = (0..slice.len()).max_by(|&a, &b| slice[a].abs().total_cmp(&slice[b].abs())).unwrap();
                    (window.start + index) as f32 * 1000.0 / SAMPLE_RATE
                };
                row += &format!(" {:>9.2} {:>9.2}", found(&left), found(&right));
            }
            println!("{}", row);
        }

        println!("\nLevel of each repeat of a 20 ms noise burst against the burst (dB), 300 ms, mix 1.0");
        println!("{:>9} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}   {}", "feedback", "1", "2", "3", "4", "5", "6", "per repeat, set");
        let mut burst = vec![0.0; at(2200.0)];
        burst[..at(20.0)].copy_from_slice(&noise(0.25, at(20.0)));
        let burst_level = rms(&burst[..at(20.0)]);
        let repeat_of = |channel: &[f32], number: usize| channel[at(300.0 * number as f32 - 8.0)..at(300.0 * number as f32 + 28.0)].to_vec();
        for feedback in [0.0, 0.3, 0.6, FEEDBACK_MAX] {
            let (left, _) = delay_tests::run(&delay_tests::settings(300.0, feedback, 1.0), &burst, SAMPLE_RATE);
            let mut row = format!("{:>9.2}", feedback);
            for number in 1..=6 {
                row += &format!(" {:>7.1}", to_db(rms(&repeat_of(&left, number)) * (36.0f32 / 20.0).sqrt() / burst_level));
            }
            println!("{}   {:.1} dB", row, to_db(feedback));
        }

        println!("\nSpectrum of each repeat at feedback 0.9 (dB against the burst's own level in the band)");
        println!("{:>8} {:>9} {:>9} {:>9} {:>9} {:>9}", "repeat", "<120 Hz", "120-500", "500-2k", "2k-5k", ">5 kHz");
        let (left, _) = delay_tests::run(&delay_tests::settings(300.0, FEEDBACK_MAX, 1.0), &burst, SAMPLE_RATE);
        let bands: [(Option<f32>, Option<f32>); 5] = [(None, Some(120.0)), (Some(120.0), Some(500.0)), (Some(500.0), Some(2000.0)), (Some(2000.0), Some(5000.0)), (Some(5000.0), None)];
        let in_band = |signal: &[f32], band: (Option<f32>, Option<f32>)| band_level_db(signal, SAMPLE_RATE, band.0, band.1) + to_db(rms(signal));
        // The burst with silence around it, as long as the pieces the repeats are cut into
        let mut dry_piece = vec![0.0; at(36.0)];
        dry_piece[at(8.0)..at(28.0)].copy_from_slice(&burst[..at(20.0)]);
        for number in 1..=6 {
            let piece = repeat_of(&left, number);
            let mut row = format!("{:>8}", number);
            for band in bands {
                row += &format!(" {:>9.1}", in_band(&piece, band) - in_band(&dry_piece, band));
            }
            println!("{}", row);
        }

        println!("\nLoudest output after 30 s of input, feedback 0.9, mix 1.0");
        let chords = power_chords(SAMPLE_RATE, 30.0);
        let loud_chords: Vec<f32> = chords.iter().map(|sample| (sample * 8.0).clamp(-1.0, 1.0)).collect();
        let full_noise = noise(1.0, seconds(30.0, SAMPLE_RATE));
        let square: Vec<f32> = (0..seconds(30.0, SAMPLE_RATE)).map(|i| if (i / at(20.0)) % 2 == 0 { 1.0 } else { -1.0 }).collect();
        for (name, signal, time_ms) in [
            ("power chords at -18 dBFS, 350 ms", &chords, 350.0),
            ("power chords clipped at full scale, 350 ms", &loud_chords, 350.0),
            ("full-scale noise, 350 ms", &full_noise, 350.0),
            ("full-scale square in tune with the loop, 20 ms", &square, 20.0),
        ] {
            let (left, right) = delay_tests::run(&delay_tests::settings(time_ms, FEEDBACK_MAX, 1.0), signal, SAMPLE_RATE);
            let last = seconds(25.0, SAMPLE_RATE);
            println!(
                "  {:<48} input peak {:.2}, output peak {:.2}, repeats alone {:.2}",
                name,
                peak(signal),
                peak(&left).max(peak(&right)),
                peak(&without(&left[last..], &signal[last..])).max(peak(&without(&right[last..], &signal[last..])))
            );
        }

        println!("\nWidth: guitar phrase, 350 ms, feedback 0.4, mix 1.0");
        let (left, right) = delay_tests::run(&delay_tests::settings(350.0, 0.4, 1.0), &di, SAMPLE_RATE);
        let (wet_left, wet_right) = (without(&left, &di), without(&right, &di));
        let mono: Vec<f32> = left.iter().zip(&right).map(|(l, r)| 0.5 * (l + r)).collect();
        let wet_mono: Vec<f32> = wet_left.iter().zip(&wet_right).map(|(l, r)| 0.5 * (l + r)).collect();
        println!("  correlation of the repeats, left against right: {:.2}", correlation(&wet_left, &wet_right));
        println!("  repeats against dry: {:.1} dB left, {:.1} dB right", to_db(rms(&wet_left) / rms(&di)), to_db(rms(&wet_right) / rms(&di)));
        println!("  repeats in the mono sum against one side: {:.1} dB", to_db(rms(&wet_mono) / rms(&wet_left)));
        println!("  dry signal in the mono sum: {:.3} of the input", dot(&mono, &di) / dot(&di, &di));
        println!("  mono sum against one side: {:.1} dB", to_db(rms(&mono) / rms(&left)));

        println!("\nTime dial moved while a 220 Hz sine plays (largest step between samples; steady {:.4})", {
            let (steady, _) = delay_tests::run(&delay_tests::settings(300.0, 0.5, 1.0), &sine(220.0, 0.5, SAMPLE_RATE, at(2000.0)), SAMPLE_RATE);
            largest_step(&steady[at(1500.0)..])
        });
        for (from_ms, to_ms) in [(300.0, 450.0), (450.0, 300.0), (TIME_MIN_MS, TIME_MAX_MS), (TIME_MAX_MS, TIME_MIN_MS)] {
            let input = sine(220.0, 0.5, SAMPLE_RATE, at(6000.0));
            let mut delay = new_delay(SAMPLE_RATE);
            delay_tests::run_blocks(&mut delay, &delay_tests::settings(from_ms, 0.5, 1.0), &input[..at(2000.0)], BLOCK);
            let (left, _) = delay_tests::run_blocks(&mut delay, &delay_tests::settings(to_ms, 0.5, 1.0), &input[at(2000.0)..], BLOCK);
            // Where the repeats are steady again: the step is back at what it ends up as
            let settled = largest_step(&left[left.len() - at(200.0)..]);
            let glide = (0..left.len() - at(50.0)).step_by(at(10.0)).rev().find(|&start| (largest_step(&left[start..start + at(50.0)]) / settled - 1.0).abs() > 0.05).unwrap_or(0);
            println!("  {:>6.0} to {:>6.0} ms: largest step {:.4}, glide over after about {:.2} s", from_ms, to_ms, largest_step(&left), glide as f32 / SAMPLE_RATE);
        }

        println!("\n== REVERB ==");
        println!("\nReverb time (s to fall 60 dB, from the fall between -5 and -25 dB), left channel");
        println!("{:>8} {:>8} | {:>8} {:>8} {:>8} {:>8} | {:>10} {:>10}", "rate", "set", "250 Hz", "1 kHz", "5 kHz", "5k / 1k", "1 kHz/set", "right 1k");
        for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            let decays: &[f32] = if sample_rate == 48000.0 { &[DECAY_MIN_S, 0.6, 1.0, 1.8, 3.0, DECAY_MAX_S] } else { &[1.8] };
            for &decay_s in decays {
                let (left, right) = impulse_response(decay_s, sample_rate, decay_s + 0.4);
                let (low, mid, high) = (decay_at(&left, sample_rate, 250.0), decay_at(&left, sample_rate, 1000.0), decay_at(&left, sample_rate, 5000.0));
                println!(
                    "{:>8.0} {:>8.2} | {:>8.2} {:>8.2} {:>8.2} {:>8.2} | {:>10.2} {:>10.2}",
                    sample_rate,
                    decay_s,
                    low,
                    mid,
                    high,
                    high / mid,
                    mid / decay_s,
                    decay_at(&right, sample_rate, 1000.0)
                );
            }
        }

        println!("\nDensity of the tail of one sample: kurtosis in 10 ms windows, left channel");
        println!("(3.0 is noise; higher is single echoes standing out; towards 1.5 is ringing on a few tones)");
        println!("{:>8} | {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} | {:>24}", "decay", "20 ms", "40 ms", "60 ms", "100 ms", "150 ms", "250 ms", "0.3 s on: low/median/high");
        let window = at(10.0);
        for decay_s in [DECAY_MIN_S, 1.8, DECAY_MAX_S] {
            let (left, _) = impulse_response(decay_s, SAMPLE_RATE, (decay_s * 0.6).max(0.9));
            let mut row = format!("{:>8.1} |", decay_s);
            for start_ms in [20.0, 40.0, 60.0, 100.0, 150.0, 250.0] {
                row += &format!(" {:>7.1}", kurtosis(&left[at(start_ms)..at(start_ms) + window]));
            }
            let end = seconds((decay_s * 0.6).max(0.9).min(decay_s * 1.5), SAMPLE_RATE);
            let mut late: Vec<f32> = left[at(300.0)..end.max(at(320.0))].chunks_exact(window).map(kurtosis).collect();
            late.sort_by(|a, b| a.total_cmp(b));
            println!("{} | {:>7.1} {:>7.1} {:>7.1}", row, late[0], late[late.len() / 2], late[late.len() - 1]);
        }

        println!("\nStart of the tail of one sample, decay 1.8 s");
        let (left, _) = impulse_response(1.8, SAMPLE_RATE, 0.5);
        let first = left.iter().position(|sample| sample.abs() > 0.0).unwrap();
        let smooth = at(5.0);
        let loudest = (0..left.len() - smooth).step_by(at(1.0)).max_by(|&a, &b| rms(&left[a..a + smooth]).total_cmp(&rms(&left[b..b + smooth]))).unwrap();
        let level_in = |from_ms: f32, to_ms: f32| to_db(rms(&left[at(from_ms)..at(to_ms)]) / rms(&left[loudest..loudest + smooth]));
        println!("  first sound after {:.1} ms; loudest 5 ms from {:.0} ms", first as f32 * 1000.0 / SAMPLE_RATE, loudest as f32 * 1000.0 / SAMPLE_RATE);
        println!(
            "  level against the loudest 5 ms (dB): 15-20 ms {:.1}, 20-30 ms {:.1}, 30-50 ms {:.1}, 50-100 ms {:.1}, 100-200 ms {:.1}",
            level_in(15.0, 20.0),
            level_in(20.0, 30.0),
            level_in(30.0, 50.0),
            level_in(50.0, 100.0),
            level_in(100.0, 200.0)
        );

        println!("\nRinging: the resonances from 400 to 1600 Hz in 1 Hz steps, in the tail from 0.5 s to 1.5 s:");
        println!("the strongest and the 99th percentile against the median one (dB)");
        let spread = |late: &[f32]| {
            let mut levels: Vec<f64> = (400..1600).map(|freq_hz| level_at_fast(late, SAMPLE_RATE, freq_hz as f64)).collect();
            levels.sort_by(|a, b| a.total_cmp(b));
            let against_median = |index: usize| 20.0 * (levels[index] / levels[levels.len() / 2]).log10();
            (against_median(levels.len() - 1), against_median(levels.len() * 99 / 100))
        };
        for decay_s in [1.8, DECAY_MAX_S] {
            let (left, _) = impulse_response(decay_s, SAMPLE_RATE, 1.5);
            let (strongest, most) = spread(&left[seconds(0.5, SAMPLE_RATE)..]);
            println!("  decay {:.1} s: {:.1} / {:.1}", decay_s, strongest, most);
            // What no reverb can beat: noise that dies away at the same rate
            let mut source = Noise::new(11);
            let ideal: Vec<f32> = (0..seconds(1.0, SAMPLE_RATE)).map(|index| source.next() * 10.0f32.powf(-3.0 * index as f32 / (decay_s * SAMPLE_RATE))).collect();
            let (strongest, most) = spread(&ideal);
            println!("  noise dying away at that rate: {:.1} / {:.1}", strongest, most);
        }

        println!("\nPalm mutes into the reverb (decay 1.8 s, mix 1.0): kurtosis of the tail in the gaps");
        let mutes = palm_mutes(SAMPLE_RATE, 2.4);
        let (left, right) = run(&settings(1.8, 1.0), &mutes, SAMPLE_RATE);
        let tail = without(&left, &mutes);
        let mut gaps: Vec<f32> = tail[at(300.0)..].chunks_exact(window).map(kurtosis).collect();
        gaps.sort_by(|a, b| a.total_cmp(b));
        println!("  low / median / high: {:.1} / {:.1} / {:.1}", gaps[0], gaps[gaps.len() / 2], gaps[gaps.len() - 1]);
        println!("  tail against dry: {:.1} dB, peak of the output {:.2} (dry {:.2})", to_db(rms(&tail) / rms(&mutes)), peak(&left).max(peak(&right)), peak(&mutes));

        println!("\nLevel and width: guitar phrase, mix 1.0");
        println!("{:>8} {:>14} {:>14} {:>13} {:>16} {:>14}", "decay", "tail left dB", "tail right dB", "correlation", "mono sum dB", "dry in mono");
        for decay_s in [DECAY_MIN_S, 1.0, 1.8, 3.0, DECAY_MAX_S] {
            let (left, right) = run(&settings(decay_s, 1.0), &di, SAMPLE_RATE);
            let (tail_left, tail_right) = (without(&left, &di), without(&right, &di));
            let mono: Vec<f32> = left.iter().zip(&right).map(|(l, r)| 0.5 * (l + r)).collect();
            println!(
                "{:>8.1} {:>14.1} {:>14.1} {:>13.2} {:>16.1} {:>14.3}",
                decay_s,
                to_db(rms(&tail_left) / rms(&di)),
                to_db(rms(&tail_right) / rms(&di)),
                correlation(&tail_left, &tail_right),
                to_db(rms(&mono) / rms(&left)),
                dot(&mono, &di) / dot(&di, &di)
            );
        }

        println!("\nTail of an octave of noise against that noise (dB), decay 1.8 s, mix 1.0: the low cut and the damping");
        let mut row = String::new();
        for centre_hz in [62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0] {
            let input = octave_band(&noise(0.25, seconds(6.0, SAMPLE_RATE)), SAMPLE_RATE, centre_hz);
            let (left, _) = run(&settings(1.8, 1.0), &input, SAMPLE_RATE);
            let from = seconds(2.0, SAMPLE_RATE);
            row += &format!("  {} Hz {:.1}", centre_hz, to_db(rms(&without(&left, &input)[from..]) / rms(&input[from..])));
        }
        println!("{}", row);
        let (left, right) = impulse_response(1.8, SAMPLE_RATE, 2.0);
        println!("Left against right of the tail of one sample: correlation {:.3}", correlation(&left, &right));

        println!("\nLoudest output, mix 1.0, decay 6 s, 10 s of input");
        for (name, signal) in [
            ("power chords clipped at full scale", &loud_chords[..seconds(10.0, SAMPLE_RATE)]),
            ("full-scale noise", &full_noise[..seconds(10.0, SAMPLE_RATE)]),
            ("full-scale 440 Hz sine", &sine(440.0, 1.0, SAMPLE_RATE, seconds(10.0, SAMPLE_RATE))[..]),
        ] {
            let (left, right) = run(&settings(DECAY_MAX_S, 1.0), signal, SAMPLE_RATE);
            println!("  {:<40} output peak {:.2}, tail alone {:.2}", name, peak(&left).max(peak(&right)), peak(&without(&left, signal)).max(peak(&without(&right, signal))));
        }

        println!("\nDecay dial moved while a 440 Hz sine plays (largest step; the dry sine alone makes {:.4})", 0.5 * std::f32::consts::TAU * 440.0 / SAMPLE_RATE);
        for (from_s, to_s) in [(DECAY_MIN_S, DECAY_MAX_S), (DECAY_MAX_S, DECAY_MIN_S)] {
            let input = sine(440.0, 0.5, SAMPLE_RATE, seconds(4.0, SAMPLE_RATE));
            let mut reverb = new_reverb(SAMPLE_RATE);
            let half = input.len() / 2;
            let (before, _) = run_blocks(&mut reverb, &settings(from_s, 1.0), &input[..half], BLOCK);
            let (after, _) = run_blocks(&mut reverb, &settings(to_s, 1.0), &input[half..], BLOCK);
            println!("  {:.1} to {:.1} s: before {:.4}, after {:.4}", from_s, to_s, largest_step(&before[half / 2..]), largest_step(&after));
        }

        println!("\n== TAILS AND COST ==");
        println!("\nSeconds from the end of the input until idle, at mix 1.0, and the peak of what was still");
        println!("coming out in the last 10 ms before that and in the 10 ms a second earlier, in dBFS");
        // Runs `process` on silence until `is_idle`. Returns the seconds and the two peaks
        fn ring_out(mut process: impl FnMut(&mut [f32], &mut [f32]) -> bool) -> (f32, f32, f32) {
            let mut heard = Vec::new();
            let mut idle = false;
            while !idle && heard.len() < 200 * SAMPLE_RATE as usize {
                let (mut left, mut right) = ([0.0; BLOCK], [0.0; BLOCK]);
                idle = process(&mut left, &mut right);
                heard.extend(left.iter().zip(&right).map(|(l, r)| l.abs().max(r.abs())));
            }
            let window = (0.01 * SAMPLE_RATE) as usize;
            let peak_before = |back: usize| {
                let end = heard.len().saturating_sub(back);
                to_db(peak(&heard[end.saturating_sub(window)..end]))
            };
            (heard.len() as f32 / SAMPLE_RATE, peak_before(0), peak_before(SAMPLE_RATE as usize))
        }
        for (time_ms, feedback) in [(TIME_MIN_MS, 0.35), (350.0, 0.35), (350.0, FEEDBACK_MAX), (TIME_MAX_MS, FEEDBACK_MAX)] {
            let mut delay = new_delay(SAMPLE_RATE);
            let setting = delay_tests::settings(time_ms, feedback, 1.0);
            delay_tests::run_blocks(&mut delay, &setting, &di[..seconds(2.0, SAMPLE_RATE)], BLOCK);
            let (time_s, last, earlier) = ring_out(|left, right| {
                delay.process(&setting, left, right);
                delay.is_idle()
            });
            println!("  delay {:>6.0} ms, feedback {:.2}: {:>5.1} s {:>8.1} {:>8.1}", time_ms, feedback, time_s, last, earlier);
        }
        for decay_s in [DECAY_MIN_S, 1.8, DECAY_MAX_S] {
            let mut reverb = new_reverb(SAMPLE_RATE);
            run_blocks(&mut reverb, &settings(decay_s, 1.0), &di[..seconds(2.0, SAMPLE_RATE)], BLOCK);
            let (time_s, last, earlier) = ring_out(|left, right| {
                reverb.process(&settings(decay_s, 1.0), left, right);
                reverb.is_idle()
            });
            println!("  reverb decay {:.1} s:               {:>5.1} s {:>8.1} {:>8.1}", decay_s, time_s, last, earlier);
        }

        println!("\nCost per block of {} samples (median, microseconds, and share of real time)", BLOCK);
        println!("{:>10} {:<34} {:>9} {:>9}", "rate", "", "us", "%");
        for sample_rate in [48000.0, 192000.0] {
            let block_us = BLOCK as f64 / sample_rate as f64 * 1e6;
            let playing = guitar_di(sample_rate);
            let silence = vec![0.0; seconds(1.0, sample_rate)];
            let delay_on = delay_tests::settings(350.0, 0.5, 0.5);
            let reverb_on = settings(1.8, 0.5);
            let print = |name: &str, time_us: f64| println!("{:>10.0} {:<34} {:>9.2} {:>9.3}", sample_rate, name, time_us, 100.0 * time_us / block_us);

            let mut delay = new_delay(sample_rate);
            print("delay playing", delay_tests::block_time_us(&mut delay, &delay_on, &playing));
            print("delay tail", delay_tests::block_time_us(&mut delay, &delay_on, &silence));
            delay.reset();
            print("delay on, idle", delay_tests::block_time_us(&mut delay, &delay_on, &silence));
            print("delay off, idle, input playing", delay_tests::block_time_us(&mut delay, &DelaySettings { on: false, ..delay_on }, &playing));

            let mut reverb = new_reverb(sample_rate);
            print("reverb playing", block_time_us(&mut reverb, &reverb_on, &playing));
            print("reverb tail", block_time_us(&mut reverb, &reverb_on, &silence));
            print("reverb, decay dial moving", {
                let mut times: Vec<f64> = playing
                    .chunks(BLOCK)
                    .enumerate()
                    .map(|(index, block)| {
                        let (mut l, mut r) = (block.to_vec(), block.to_vec());
                        let setting = settings(if (index / 200) % 2 == 0 { DECAY_MIN_S } else { DECAY_MAX_S }, 0.5);
                        let start = Instant::now();
                        reverb.process(&setting, &mut l, &mut r);
                        start.elapsed().as_secs_f64() * 1e6
                    })
                    .collect();
                times.sort_by(|a, b| a.total_cmp(b));
                times[times.len() / 2]
            });
            reverb.reset();
            print("reverb on, idle", block_time_us(&mut reverb, &reverb_on, &silence));
            print("reverb off, idle, input playing", block_time_us(&mut reverb, &ReverbSettings { on: false, ..reverb_on }, &playing));

            let both = |delay: &mut Delay, reverb: &mut Reverb, delay_settings: &DelaySettings, reverb_settings: &ReverbSettings, input: &[f32]| {
                let (mut left, mut right) = (input.to_vec(), input.to_vec());
                let mut times: Vec<f64> = left
                    .chunks_mut(BLOCK)
                    .zip(right.chunks_mut(BLOCK))
                    .map(|(l, r)| {
                        let start = Instant::now();
                        delay.process(delay_settings, l, r);
                        reverb.process(reverb_settings, l, r);
                        start.elapsed().as_secs_f64() * 1e6
                    })
                    .collect();
                times.sort_by(|a, b| a.total_cmp(b));
                (times[times.len() / 2], times[times.len() * 99 / 100], times[times.len() * 999 / 1000])
            };
            let (mut delay, mut reverb) = (new_delay(sample_rate), new_reverb(sample_rate));
            let (median, most, worst) = both(&mut delay, &mut reverb, &delay_on, &reverb_on, &playing);
            print("both playing", median);
            print("both playing, 99th percentile", most);
            print("both playing, 99.9th percentile", worst);
            let (median, _, _) = both(&mut delay, &mut reverb, &delay_on, &reverb_on, &silence);
            print("both, tail", median);
            let off = (DelaySettings { on: false, ..delay_on }, ReverbSettings { on: false, ..reverb_on });
            let (_, _, worst) = both(&mut delay, &mut reverb, &off.0, &off.1, &vec![0.0; seconds(30.0, sample_rate)]);
            assert!(delay.is_idle() && reverb.is_idle());
            print("both ringing out, 99.9th percentile", worst);
            let (median, _, _) = both(&mut delay, &mut reverb, &off.0, &off.1, &playing);
            print("both off, idle, input playing", median);
        }
    }

    /// Writes stereo WAV files to target/renders for listening:
    ///   cargo test -p amp --release render_effects -- --ignored
    #[test]
    #[ignore]
    fn render_effects() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();
        let rate = SAMPLE_RATE as u32;

        // The phrase with room after it for the tails
        let mut phrase = guitar_di(SAMPLE_RATE);
        phrase.extend(vec![0.0; seconds(5.0, SAMPLE_RATE)]);
        let mut mutes = palm_mutes(SAMPLE_RATE, 4.8);
        mutes.extend(vec![0.0; seconds(4.0, SAMPLE_RATE)]);
        let mut click = vec![0.0; seconds(7.0, SAMPLE_RATE)];
        click[0] = 0.9;
        write_wav(&dir.join("amp_fx_dry.wav"), &[&phrase, &phrase], rate);

        let delay_off = DelaySettings::default();
        let reverb_off = ReverbSettings::default();
        let delay = |time_ms: f32, feedback: f32, mix: f32| delay_tests::settings(time_ms, feedback, mix);
        let renders: [(&str, &[f32], DelaySettings, ReverbSettings); 12] = [
            ("delay_350ms", &phrase, delay(350.0, 0.4, 0.4), reverb_off),
            ("delay_slap_90ms", &phrase, delay(90.0, 0.15, 0.5), reverb_off),
            ("delay_long_feedback9", &phrase, delay(600.0, FEEDBACK_MAX, 0.5), reverb_off),
            ("reverb_short", &phrase, delay_off, settings(0.6, 0.4)),
            ("reverb_medium", &phrase, delay_off, settings(1.8, 0.35)),
            ("reverb_long", &phrase, delay_off, settings(DECAY_MAX_S, 0.4)),
            ("reverb_wet", &phrase, delay_off, settings(2.5, 1.0)),
            ("reverb_palm_mutes", &mutes, delay_off, settings(1.8, 0.5)),
            ("both", &phrase, delay(350.0, 0.4, 0.35), settings(1.8, 0.3)),
            ("impulse_delay", &click, delay(350.0, 0.6, 1.0), reverb_off),
            ("impulse_reverb_1s8", &click, delay_off, settings(1.8, 1.0)),
            ("impulse_reverb_6s", &click, delay_off, settings(DECAY_MAX_S, 1.0)),
        ];
        for (name, input, delay_settings, reverb_settings) in renders {
            let (left, right) = run_both(&mut new_delay(SAMPLE_RATE), &mut new_reverb(SAMPLE_RATE), &delay_settings, &reverb_settings, input);
            write_wav(&dir.join(format!("amp_fx_{}.wav", name)), &[&left, &right], rate);
        }

        // The dials moved and the effects switched while the playing goes on
        let (mut delay_unit, mut reverb_unit) = (new_delay(SAMPLE_RATE), new_reverb(SAMPLE_RATE));
        let (mut left, mut right) = (phrase.clone(), phrase.clone());
        for (index, (l, r)) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)).enumerate() {
            let turn = index * BLOCK / seconds(1.5, SAMPLE_RATE);
            let delay_settings = DelaySettings {
                on: turn % 4 != 3,
                ..delay([350.0, 500.0, 200.0][turn % 3], 0.5, 0.4)
            };
            let reverb_settings = ReverbSettings {
                on: turn % 5 != 4,
                ..settings([1.0, 4.0][turn % 2], 0.35)
            };
            delay_unit.process(&delay_settings, l, r);
            reverb_unit.process(&reverb_settings, l, r);
        }
        write_wav(&dir.join("amp_fx_dials_moving.wav"), &[&left, &right], rate);

        println!("Wrote renders to {}", dir.display());
    }
}
