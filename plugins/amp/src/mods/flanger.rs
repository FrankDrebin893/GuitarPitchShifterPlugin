//! Flanger: a copy of the signal a few milliseconds and less behind it, fed back into itself,
//! so a comb of peaks runs up and down through the sound. Behind the cabinet there is one
//! comb on each side

use super::lfo::Lfo;
use super::line::ModLine;
use super::{middle, Dial, Fade, TURNS_APART};
use crate::delay::SILENCE;
use crate::dsp::filters::{OnePoleHp, OnePoleLp};

pub const RATE_MIN_HZ: f32 = 0.05;
pub const RATE_MAX_HZ: f32 = 5.0;

// The copy sweeps from this delay
const SHORTEST_MS: f32 = 0.5;
// to this much more at Depth 10: 0.5 to 6 ms. The sweep is the square of a sine, which
// stays longer at the short delays, where the comb is widest
const SWEEP_MS: f32 = 5.5;
// Up to this rate. Above it the sweep is shorter as the rate goes up, so the pitch of the
// copy wavers no more than at this rate
const FULL_SWEEP_TO_HZ: f32 = 1.0;

// Share of what comes out of a line that goes back into it. The peaks of the comb stand
// 6 dB over the copy and the line rings out in a tenth of a second; far from howling by
// itself at every setting and rate
const FEEDBACK: f32 = 0.5;

// Level of the copy against the dry signal at Depth 10. It follows the square root of
// the dial: 0.42 at Depth 5
const WET: f32 = 0.6;

// The copy has no lows: they would only be louder with it, below the first notch. Its
// top is soft, and softer with every turn through the line
const HIGHPASS_HZ: f32 = 150.0;
const LOWPASS_HZ: f32 = 8000.0;

// How fast the length of the sweep follows the dials. The pitch of the copy bends while it
// moves
const WIDTH_SMOOTH_MS: f32 = 250.0;

/// What the knobs say, read once per call. The flanger smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlangerSettings {
    /// Off lets what is in the lines ring out
    pub on: bool,
    /// `RATE_MIN_HZ` to `RATE_MAX_HZ`
    pub rate_hz: f32,
    /// Length of the sweep and level of the copy, 0.0 to 1.0. The dry signal stays as it is
    pub depth: f32,
}

impl Default for FlangerSettings {
    fn default() -> Self {
        Self {
            on: false,
            rate_hz: 0.3,
            depth: 0.5,
        }
    }
}

/// A delay line with feedback for each channel, both fed the middle of the channels. In
/// front of the amp only the first one runs
pub struct Flanger {
    lines: [ModLine; 2],
    highpass: OnePoleHp,
    lowpass: [OnePoleLp; 2],
    lfo: Lfo,
    // Share of the input that reaches the lines: 1.0 on, 0.0 off
    feed: Fade,

    // Where the dials are, on their way to the settings: the level of the copy, and the
    // share of the full sweep
    wet: Dial,
    width: Dial,

    // In samples
    shortest: f64,
    sweep: f64,
    // The furthest back a line is read
    reach: usize,

    // Samples in a row that only zeros went into the lines
    quiet_run: usize,
    // Nothing is left to hear and the filters are at rest
    idle: bool,
}

impl Flanger {
    /// Allocates every buffer the pedal will ever need, for rates up to `MAX_RATE_HZ`
    pub fn new() -> Self {
        let mut flanger = Self {
            lines: [ModLine::new(SHORTEST_MS + SWEEP_MS), ModLine::new(SHORTEST_MS + SWEEP_MS)],
            highpass: OnePoleHp::new(),
            lowpass: [OnePoleLp::new(); 2],
            lfo: Lfo::new(),
            feed: Fade::new(),
            wet: Dial::new(),
            width: Dial::new(),
            shortest: 0.0,
            sweep: 0.0,
            reach: 0,
            quiet_run: 0,
            idle: true,
        };
        flanger.set_sample_rate(48000.0);
        flanger
    }

    /// The rate the pedal runs at, which starts it over. Never allocates: it may be called on the audio thread
    /// when the pedal is moved between its two places
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.highpass.set(HIGHPASS_HZ, sample_rate);
        for lowpass in &mut self.lowpass {
            lowpass.set(LOWPASS_HZ, sample_rate);
        }
        self.lfo.set_sample_rate(sample_rate);
        self.feed.set_sample_rate(sample_rate);
        self.wet.set_sample_rate(sample_rate);
        self.width.set_time(WIDTH_SMOOTH_MS, sample_rate);
        self.shortest = SHORTEST_MS as f64 * 0.001 * sample_rate as f64;
        self.sweep = SWEEP_MS as f64 * 0.001 * sample_rate as f64;
        self.reach = (self.shortest + self.sweep) as usize + 3;
        self.reset();
    }

    /// Drops what is in the lines. Does not allocate
    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        self.lfo.reset();
        self.feed.snap(false);
        self.rest();
    }

    /// For when nothing is left to hear
    fn rest(&mut self) {
        self.highpass.reset();
        for lowpass in &mut self.lowpass {
            lowpass.reset();
        }
        self.quiet_run = 0;
        self.idle = true;
    }

    /// True when nothing of earlier input is left in it (then processing costs next to nothing)
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// In front of the amp: one channel, at the amp's oversampled rate
    pub fn process_mono(&mut self, settings: &FlangerSettings, block: &mut [f32]) {
        self.process(settings, [block]);
    }

    /// Behind the cabinet: two channels at the host's rate
    pub fn process_stereo(&mut self, settings: &FlangerSettings, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        let len = left.len().min(right.len());
        self.process(settings, [&mut left[..len], &mut right[..len]]);
    }

    /// How far behind the dry signal a channel's copy is now, in samples
    fn delay(&self, channel: usize) -> f64 {
        let position = 0.5 + 0.5 * self.lfo.sine(TURNS_APART[channel]);
        self.shortest + self.sweep * (self.width.value() * position * position) as f64
    }

    /// True when the filters would put out nothing more by themselves
    fn filters_at_rest(&self) -> bool {
        self.highpass.state().abs() < SILENCE && self.lowpass.iter().all(|filter| filter.state().abs() < SILENCE)
    }

    /// Adds the copies to a block in place
    fn process<const N: usize>(&mut self, settings: &FlangerSettings, mut channels: [&mut [f32]; N]) {
        let rate = settings.rate_hz.clamp(RATE_MIN_HZ, RATE_MAX_HZ);
        let depth = settings.depth.clamp(0.0, 1.0);
        let wet = WET * depth.sqrt();
        let width = depth * (FULL_SWEEP_TO_HZ / rate).min(1.0);

        for index in 0..channels[0].len() {
            let input = middle(&channels, index);
            if self.idle {
                // Nothing to add until the flanger is on and there is something to play. When
                // it is switched on later, in the middle of a note, it fades in
                if !settings.on {
                    self.feed.snap(false);
                    return;
                }
                if input.abs() < SILENCE {
                    continue;
                }
                // Nothing is heard yet, so there is nothing to smooth
                self.wet.snap(wet);
                self.width.snap(width);
                self.lfo.snap(rate);
                self.idle = false;
            }

            let feed = self.feed.next(settings.on);
            let level = self.wet.next(wet);
            self.width.next(width);
            self.lfo.step(rate);

            let input = self.highpass.process(input * feed);
            let mut quiet = true;
            for (number, channel) in channels.iter_mut().enumerate() {
                let copy = self.lines[number].read(self.delay(number));
                channel[index] += level * copy;

                let fed = self.lowpass[number].process(input + FEEDBACK * copy);
                if fed.abs() >= SILENCE {
                    self.lines[number].push(fed);
                    quiet = false;
                } else {
                    self.lines[number].push(0.0);
                }
            }

            if !quiet {
                self.quiet_run = 0;
                continue;
            }
            // Idle once everything that can be read is zeros and the filters have rung out
            self.quiet_run += 1;
            if self.quiet_run >= self.reach && self.filters_at_rest() {
                for line in &mut self.lines {
                    line.keep_newest(self.quiet_run);
                }
                self.feed.snap(settings.on);
                self.rest();
            }
        }
    }

    /// Where a channel's copy is in its sweep, in ms behind the dry signal
    #[cfg(test)]
    pub(crate) fn swept(&self, channel: usize) -> f32 {
        (self.delay(channel) / self.shortest) as f32 * SHORTEST_MS
    }

    /// Address and capacity of every buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<(usize, usize)> {
        self.lines.iter().map(|line| line.buffer()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::tests::*;
    use crate::test_util::*;

    fn settings(rate_hz: f32, depth: f32) -> FlangerSettings {
        FlangerSettings { on: true, rate_hz, depth }
    }

    #[test]
    fn test_copy_sweeps_from_half_a_ms_to_6_ms_at_every_rate() {
        for sample_rate in [48000.0, 192_000.0, 384_000.0] {
            for (depth, to_ms) in [(1.0, 6.0), (0.5, 3.25), (0.0, 0.5)] {
                for stereo in [false, true] {
                    let (low, high) = swept_range::<Flanger>(sample_rate, &settings(1.0, depth), stereo);
                    assert!((low - 0.5).abs() < 0.01 && (high - to_ms).abs() < 0.01, "{sample_rate} Hz, depth {depth}: {low} to {high} ms");
                }
            }
        }
    }

    #[test]
    fn test_sweep_is_shorter_at_fast_rates() {
        let (low, high) = swept_range::<Flanger>(48000.0, &settings(RATE_MAX_HZ, 1.0), false);
        assert!((low - 0.5).abs() < 0.01 && (high - 1.6).abs() < 0.01, "{low} to {high} ms");
    }

    #[test]
    fn test_copy_arrives_when_the_sweep_says() {
        // A click every 40 ms, and where the copy of each one lands: the first and loudest
        // turn through the line
        for sample_rate in [48000.0, 192_000.0, 384_000.0] {
            let every = (0.04 * sample_rate) as usize;
            let mut input = vec![0.0; 60 * every];
            input.iter_mut().step_by(every).for_each(|sample| *sample = 0.5);
            let mut flanger = Flanger::at(sample_rate);
            let output = run_mono(&mut flanger, &settings(1.0, 1.0), &input, 64);

            let delays_ms: Vec<f32> = output
                .chunks(every)
                .skip(1)
                .map(|piece| {
                    let wet = &piece[1..];
                    let at = (0..wet.len()).max_by(|&a, &b| wet[a].abs().total_cmp(&wet[b].abs())).unwrap() + 1;
                    at as f32 * 1000.0 / sample_rate
                })
                .collect();
            let (nearest, furthest) = delays_ms.iter().fold((f32::MAX, f32::MIN), |(low, high), &delay| (low.min(delay), high.max(delay)));
            assert!((0.5..0.8).contains(&nearest) && (5.7..6.2).contains(&furthest), "{sample_rate} Hz: {nearest} to {furthest} ms");
        }
    }

    #[test]
    fn test_comb_has_peaks_of_the_set_height_and_shallow_notches() {
        // Next to no depth is next to no sweep: a copy that stays 0.5 ms behind
        for sample_rate in [48000.0, 192_000.0] {
            let depth = 0.001;
            let gain = |freq_hz: f32| {
                let input = sine(freq_hz, 0.25, sample_rate, (0.2 * sample_rate) as usize);
                let output = run_mono(&mut Flanger::at(sample_rate), &settings(RATE_MIN_HZ, depth), &input, 64);
                let from = output.len() / 2;
                (rms(&output[from..]) / rms(&input[from..]) - 1.0) / (WET * depth.sqrt())
            };
            // At 2 kHz the copy and every turn of it are in step with the dry signal, but
            // for what the filters in the line turn them by. At 1 kHz every other turn is
            // upside down
            let (peak, notch) = (gain(2000.0), gain(1000.0));
            assert!(peak > 1.5 && peak < 1.0 / (1.0 - FEEDBACK), "{sample_rate} Hz: peak {peak}");
            assert!((notch + 1.0 / (1.0 + FEEDBACK)).abs() < 0.1, "{sample_rate} Hz: notch {notch}");
        }
    }

    #[test]
    fn test_line_rings_out_and_never_howls() {
        // A burst at every setting, and what is left a quarter of a second later
        for sample_rate in [44100.0, 384_000.0] {
            for rate_hz in [RATE_MIN_HZ, RATE_MAX_HZ] {
                for depth in [0.05, 1.0] {
                    let mut noise = Noise::new(9);
                    let mut input: Vec<f32> = (0..(0.1 * sample_rate) as usize).map(|_| noise.next()).collect();
                    input.resize((0.4 * sample_rate) as usize, 0.0);
                    let mut flanger = Flanger::at(sample_rate);
                    let output = run_mono(&mut flanger, &settings(rate_hz, depth), &input, 128);
                    let late = peak(&output[(0.35 * sample_rate) as usize..]);
                    assert!(late < 1e-4, "{sample_rate} Hz, {rate_hz} Hz, depth {depth}: {late} left");
                }
            }
        }
    }

    #[test]
    fn test_depth_jump_bends_the_pitch_only_a_little() {
        let mut flanger = Flanger::at(48000.0);
        let mut steepest: f64 = 0.0;
        for depth in [0.0, 1.0, 0.0, 1.0] {
            for _ in 0..36000 {
                let before = flanger.delay(0);
                flanger.process_mono(&settings(0.05, depth), &mut [0.1]);
                if !flanger.is_idle() {
                    steepest = steepest.max((flanger.delay(0) - before).abs());
                }
            }
        }
        // 38 cents
        assert!(steepest < 0.0225, "The copy moves {steepest} samples per sample");
    }

    #[test]
    fn test_level_is_about_the_same_on_and_off() {
        for stereo in [false, true] {
            let default = level_change_lu::<Flanger>(&Flanger::defaults(), stereo);
            assert!((0.0..1.0).contains(&default), "Depth 5, stereo {stereo}: {default} LU");
            let deepest = level_change_lu::<Flanger>(&settings(0.3, 1.0), stereo);
            assert!((0.0..1.8).contains(&deepest), "Depth 10, stereo {stereo}: {deepest} LU");
        }
    }

    #[test]
    fn test_stereo_is_wide_and_sums_to_mono_without_deep_notches() {
        let setting = settings(0.5, 1.0);
        let width = stereo_correlation::<Flanger>(&setting);
        assert!((0.5..0.97).contains(&width), "Left against right {width}");
        let (one, _) = deepest_notch_db::<Flanger>(&setting, false);
        let (sum, at_hz) = deepest_notch_db::<Flanger>(&setting, true);
        // A copy at 0.6 with every other turn upside down takes 4.4 dB out while it stands
        // still. While it moves the turns are not in step, and at the top of the guitar's
        // range it gets to 6 dB
        assert!(one > -8.0, "One channel {one} dB");
        assert!(sum > -8.0 && sum > one - 0.5, "Mono sum {sum} dB at {at_hz} Hz, one channel {one} dB");
    }
}
