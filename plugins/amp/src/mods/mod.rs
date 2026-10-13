//! The modulation pedals: chorus, flanger, phaser and tremolo, each with a Rate and a Depth.
//! A pedal runs in one of two places: in front of the amp, on one channel at the amp's
//! oversampled rate, or behind the cabinet, on two channels at the host's rate. It sounds
//! the same in both

pub mod chorus;
pub mod flanger;
pub mod lfo;
pub mod line;
pub mod phaser;
pub mod tremolo;

use crate::dsp::smoothing_coeff;

/// The highest rate a pedal is run at: the amp's oversampled rate in a host at 192 kHz
pub const MAX_RATE_HZ: f32 = 384_000.0;

// Fade when a pedal is switched
const FADE_MS: f32 = 10.0;
// How fast a level follows its dial
const DIAL_SMOOTH_MS: f32 = 30.0;
const DIAL_SETTLED: f64 = 1e-5;

// Behind the cabinet the right channel's sweep is a quarter turn ahead of the left one's
const TURNS_APART: [f32; 2] = [0.0, 0.25];

/// What is in the middle of the channels at one sample
fn middle<const N: usize>(channels: &[&mut [f32]; N], index: usize) -> f32 {
    channels.iter().map(|channel| channel[index]).sum::<f32>() / N as f32
}

/// What a switch lets through, 0.0 to 1.0: it moves in a straight line when the switch does
struct Fade {
    value: f32,
    step: f32,
}

impl Fade {
    fn new() -> Self {
        Self { value: 0.0, step: 1.0 }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.step = 1.0 / (FADE_MS * 0.001 * sample_rate);
    }

    /// Jumps to where the switch is, for when nothing is heard of it
    fn snap(&mut self, on: bool) {
        self.value = if on { 1.0 } else { 0.0 };
    }

    fn next(&mut self, on: bool) -> f32 {
        let target = if on { 1.0 } else { 0.0 };
        if self.value != target {
            self.value = (self.value + self.step.copysign(target - self.value)).clamp(0.0, 1.0);
        }
        self.value
    }

    fn is_out(&self) -> bool {
        self.value == 0.0
    }
}

/// A value on its way to what a dial says: a one-pole follower that arrives. In f64: at
/// the highest sample rates its steps are too small for f32
struct Dial {
    value: f64,
    coeff: f64,
}

impl Dial {
    fn new() -> Self {
        Self { value: 0.0, coeff: 1.0 }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.set_time(DIAL_SMOOTH_MS, sample_rate);
    }

    /// For a dial that is to be followed more slowly
    fn set_time(&mut self, time_ms: f32, sample_rate: f32) {
        self.coeff = smoothing_coeff(time_ms, sample_rate) as f64;
    }

    /// Jumps to the dial, for when nothing is heard of it
    fn snap(&mut self, target: f32) {
        self.value = target as f64;
    }

    fn next(&mut self, target: f32) -> f32 {
        let target = target as f64;
        if self.value != target {
            let next = self.value + self.coeff * (target - self.value);
            self.value = if (target - next).abs() < DIAL_SETTLED { target } else { next };
        }
        self.value as f32
    }

    fn value(&self) -> f32 {
        self.value as f32
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::chorus::{Chorus, ChorusSettings};
    use super::flanger::{Flanger, FlangerSettings};
    use super::phaser::{Phaser, PhaserSettings};
    use super::tremolo::{Tremolo, TremoloSettings};
    use super::*;
    use crate::test_util::*;
    use std::time::Instant;

    const SAMPLE_RATE: f32 = 48000.0;
    const BLOCK: usize = 64;

    // The two places a pedal runs in, at a rate each: behind the cabinet and in front of
    // the amp
    const PLACES: [(f32, bool); 2] = [(48000.0, true), (192_000.0, false)];

    /// What the tests ask of a pedal, the same for all four
    pub(crate) trait Pedal: Sized {
        const NAME: &'static str;
        /// Lowest and highest rate
        const RATE_HZ: (f32, f32);
        /// What `swept` is in
        const UNIT: &'static str;
        /// The right channel's sweep is a quarter turn ahead of the left one's
        const WIDE: bool;
        type Settings: Copy;

        fn at(sample_rate: f32) -> Self;
        fn retune(&mut self, sample_rate: f32);
        fn start_over(&mut self);
        fn idle(&self) -> bool;
        fn settings(on: bool, rate_hz: f32, depth: f32) -> Self::Settings;
        /// On, with the rate and depth a new pedal has
        fn defaults() -> Self::Settings;
        fn rate_of(settings: &Self::Settings) -> f32;
        fn defaults_rate() -> f32 {
            Self::rate_of(&Self::defaults())
        }
        fn mono(&mut self, settings: &Self::Settings, block: &mut [f32]);
        fn stereo(&mut self, settings: &Self::Settings, left: &mut [f32], right: &mut [f32]);
        /// Where a channel is in its sweep
        fn swept(&self, channel: usize) -> f32;
        fn buffers(&self) -> Vec<(usize, usize)>;
    }

    macro_rules! pedal {
        ($pedal:ident, $settings:ident, $module:ident, $name:literal, $unit:literal, wide: $wide:literal) => {
            impl Pedal for $pedal {
                const NAME: &'static str = $name;
                const RATE_HZ: (f32, f32) = (super::$module::RATE_MIN_HZ, super::$module::RATE_MAX_HZ);
                const UNIT: &'static str = $unit;
                const WIDE: bool = $wide;
                type Settings = $settings;

                fn at(sample_rate: f32) -> Self {
                    let mut pedal = $pedal::new();
                    pedal.set_sample_rate(sample_rate);
                    pedal
                }

                fn retune(&mut self, sample_rate: f32) {
                    self.set_sample_rate(sample_rate);
                }

                fn start_over(&mut self) {
                    self.reset();
                }

                fn idle(&self) -> bool {
                    self.is_idle()
                }

                fn settings(on: bool, rate_hz: f32, depth: f32) -> $settings {
                    $settings { on, rate_hz, depth }
                }

                fn defaults() -> $settings {
                    $settings { on: true, ..$settings::default() }
                }

                fn rate_of(settings: &$settings) -> f32 {
                    settings.rate_hz
                }

                fn mono(&mut self, settings: &$settings, block: &mut [f32]) {
                    self.process_mono(settings, block);
                }

                fn stereo(&mut self, settings: &$settings, left: &mut [f32], right: &mut [f32]) {
                    self.process_stereo(settings, left, right);
                }

                fn swept(&self, channel: usize) -> f32 {
                    $pedal::swept(self, channel)
                }

                fn buffers(&self) -> Vec<(usize, usize)> {
                    $pedal::buffers(self)
                }
            }
        };
    }

    pedal!(Chorus, ChorusSettings, chorus, "Chorus", "ms", wide: true);
    pedal!(Flanger, FlangerSettings, flanger, "Flanger", "ms", wide: true);
    pedal!(Phaser, PhaserSettings, phaser, "Phaser", "Hz", wide: true);
    pedal!(Tremolo, TremoloSettings, tremolo, "Tremolo", "x", wide: false);

    /// Runs a check on each of the four pedals
    macro_rules! every_pedal {
        ($check:ident) => {{
            $check::<Chorus>();
            $check::<Flanger>();
            $check::<Phaser>();
            $check::<Tremolo>();
        }};
    }

    /// Runs a signal through in blocks: on one channel, or the same on two. Returns the
    /// channels
    pub(crate) fn run<P: Pedal>(pedal: &mut P, settings: &P::Settings, input: &[f32], stereo: bool, block: usize) -> Vec<Vec<f32>> {
        let mut left = input.to_vec();
        if !stereo {
            left.chunks_mut(block).for_each(|piece| pedal.mono(settings, piece));
            return vec![left];
        }
        let mut right = input.to_vec();
        for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
            pedal.stereo(settings, l, r);
        }
        vec![left, right]
    }

    pub(crate) fn run_mono<P: Pedal>(pedal: &mut P, settings: &P::Settings, input: &[f32], block: usize) -> Vec<f32> {
        run(pedal, settings, input, false, block).remove(0)
    }

    pub(crate) fn run_stereo<P: Pedal>(pedal: &mut P, settings: &P::Settings, input: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        let mut channels = run(pedal, settings, input, true, block);
        let right = channels.remove(1);
        (channels.remove(0), right)
    }

    fn noise(level: f32, len: usize) -> Vec<f32> {
        let mut noise = Noise::new(3);
        (0..len).map(|_| level * noise.next()).collect()
    }

    fn seconds(time_s: f32, sample_rate: f32) -> usize {
        (time_s * sample_rate) as usize
    }

    // How often the sweep is looked at, in samples
    const LOOK: usize = 8;

    /// Where each channel is in its sweep, every `LOOK` samples of `time_s` seconds of
    /// playing
    fn sweep<P: Pedal>(sample_rate: f32, settings: &P::Settings, stereo: bool, time_s: f32) -> Vec<Vec<f32>> {
        let mut pedal = P::at(sample_rate);
        let mut swept = vec![Vec::new(); if stereo { 2 } else { 1 }];
        for piece in noise(0.1, seconds(time_s, sample_rate)).chunks(LOOK) {
            run(&mut pedal, settings, piece, stereo, LOOK);
            for (channel, swept) in swept.iter_mut().enumerate() {
                swept.push(pedal.swept(channel));
            }
        }
        swept
    }

    fn range(values: &[f32]) -> (f32, f32) {
        values.iter().fold((f32::MAX, f32::MIN), |(low, high), &value| (low.min(value), high.max(value)))
    }

    /// The lowest and highest place of the sweep in a turn and a half, over all channels
    pub(crate) fn swept_range<P: Pedal>(sample_rate: f32, settings: &P::Settings, stereo: bool) -> (f32, f32) {
        range(&sweep::<P>(sample_rate, settings, stereo, 1.5 / P::rate_of(settings)).concat())
    }

    /// The rate the sweep turns at, in Hz, and how many turns the right channel is ahead
    /// of the left one (0.0 on one channel)
    pub(crate) fn measured_rate<P: Pedal>(sample_rate: f32, settings: &P::Settings, stereo: bool) -> (f32, f32) {
        let rate = P::rate_of(settings);
        let swept = sweep::<P>(sample_rate, settings, stereo, 3.6 / rate);
        // The moments a sweep goes up through its middle, in samples
        let crossings = |values: &[f32]| -> Vec<f64> {
            let (low, high) = range(values);
            let middle = 0.5 * (low + high);
            (1..values.len())
                .filter(|&index| values[index - 1] <= middle && values[index] > middle)
                .map(|index| (index as f64 - ((values[index] - middle) / (values[index] - values[index - 1])) as f64) * LOOK as f64)
                .collect()
        };
        let left = crossings(&swept[0]);
        assert!(left.len() >= 3, "{}: {} turns in the time of three", P::NAME, left.len());
        let turn = (left[left.len() - 1] - left[0]) / (left.len() - 1) as f64;
        let ahead = if stereo { ((left[0] - crossings(&swept[1])[0]) / turn).rem_euclid(1.0) } else { 0.0 };
        ((sample_rate as f64 / turn) as f32, ahead as f32)
    }

    /// How much louder a recording of clean playing is with the pedal than without, in LU
    pub(crate) fn level_change_lu<P: Pedal>(settings: &P::Settings, stereo: bool) -> f32 {
        let input = clean_di(SAMPLE_RATE);
        let output = run(&mut P::at(SAMPLE_RATE), settings, &input, stereo, BLOCK);
        let channels: Vec<&[f32]> = output.iter().map(|channel| &channel[..]).collect();
        let dry = vec![&input[..]; channels.len()];
        loudness_lufs(&channels, SAMPLE_RATE) - loudness_lufs(&dry, SAMPLE_RATE)
    }

    /// How alike left and right are behind the pedal, for clean playing in the middle
    pub(crate) fn stereo_correlation<P: Pedal>(settings: &P::Settings) -> f32 {
        let (left, right) = run_stereo(&mut P::at(SAMPLE_RATE), settings, &clean_di(SAMPLE_RATE), BLOCK);
        correlation(&left, &right)
    }

    /// How much louder clean playing is in the sum of the two channels behind the pedal
    /// than on one channel without it, in LU
    pub(crate) fn mono_sum_change_lu<P: Pedal>(settings: &P::Settings) -> f32 {
        let input = clean_di(SAMPLE_RATE);
        let (left, right) = run_stereo(&mut P::at(SAMPLE_RATE), settings, &input, BLOCK);
        let sum: Vec<f32> = left.iter().zip(&right).map(|(l, r)| 0.5 * (l + r)).collect();
        loudness_lufs(&[&sum], SAMPLE_RATE) - loudness_lufs(&[&input], SAMPLE_RATE)
    }

    /// The deepest notch the pedal ever makes, in dB against the level without it, and
    /// the frequency it was found at: steady tones from 100 Hz to 4 kHz, each through a
    /// whole turn of the sweep, and the lowest level each one gets to. In one channel, or
    /// in the sum of the two
    pub(crate) fn deepest_notch_db<P: Pedal>(settings: &P::Settings, mono_sum: bool) -> (f32, f32) {
        let turn_s = 1.0 / P::rate_of(settings);
        let mut deepest = (0.0, 0.0);
        for step in 0..24 {
            let freq_hz = 100.0 * 40f32.powf(step as f32 / 23.0);
            let input = sine(freq_hz, 0.25, SAMPLE_RATE, seconds(1.1 * turn_s + 0.1, SAMPLE_RATE));
            let output = run(&mut P::at(SAMPLE_RATE), settings, &input, mono_sum, BLOCK);
            let heard: Vec<f32> = (0..input.len()).map(|index| output.iter().map(|channel| channel[index]).sum::<f32>() / output.len() as f32).collect();

            // The level of the tone in pieces of whole periods, 8 ms or a little more
            let periods = (0.008 * freq_hz).ceil();
            let piece = (periods * SAMPLE_RATE / freq_hz).round() as usize;
            let turn = std::f32::consts::TAU * freq_hz / SAMPLE_RATE;
            for (number, heard) in heard.chunks_exact(piece).enumerate().skip(seconds(0.1, SAMPLE_RATE) / piece) {
                let (mut sin, mut cos) = (0.0, 0.0);
                for (index, sample) in heard.iter().enumerate() {
                    let angle = turn * (number * piece + index) as f32;
                    sin += sample * angle.sin();
                    cos += sample * angle.cos();
                }
                let level = to_db(2.0 * (sin * sin + cos * cos).sqrt() / piece as f32 / 0.25);
                if level < deepest.0 {
                    deepest = (level, freq_hz);
                }
            }
        }
        deepest
    }

    /// Median time to process a sample, in nanoseconds, in the blocks the place gives the
    /// pedal. Timed over 16 blocks at a time
    fn time_ns<P: Pedal>(pedal: &mut P, settings: &P::Settings, input: &[f32], stereo: bool) -> f64 {
        let block = if stereo { 32 } else { 128 };
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        let mut times: Vec<f64> = left
            .chunks_mut(16 * block)
            .zip(right.chunks_mut(16 * block))
            .map(|(left, right)| {
                let start = Instant::now();
                for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
                    if stereo {
                        pedal.stereo(settings, l, r);
                    } else {
                        pedal.mono(settings, l);
                    }
                }
                start.elapsed().as_secs_f64() * 1e9 / left.len() as f64
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[times.len() / 2]
    }

    #[test]
    fn test_dial_and_fade_arrive_at_the_highest_rate() {
        let mut dial = Dial::new();
        dial.set_time(250.0, MAX_RATE_HZ);
        let mut fade = Fade::new();
        fade.set_sample_rate(MAX_RATE_HZ);
        let mut faded_at = None;
        for index in 0..seconds(4.0, MAX_RATE_HZ) {
            dial.next(0.7);
            if fade.next(true) == 1.0 && faded_at.is_none() {
                faded_at = Some(index);
            }
        }
        assert_eq!(dial.value(), 0.7);
        assert!(faded_at.unwrap().abs_diff(seconds(0.01, MAX_RATE_HZ)) <= 2, "Faded in after {faded_at:?} samples");
        // After 250 ms it is 1/e away
        dial.snap(0.0);
        for _ in 0..seconds(0.25, MAX_RATE_HZ) {
            dial.next(1.0);
        }
        assert!((dial.value() - 0.632).abs() < 0.001, "{}", dial.value());
    }

    fn off_from_the_start_leaves_the_signal_untouched<P: Pedal>() {
        let off = P::settings(false, P::RATE_HZ.1, 1.0);
        let mut pedal = P::at(SAMPLE_RATE);
        let (mut left, mut right) = (noise(0.5, 4800), sine(330.0, 0.4, SAMPLE_RATE, 4800));
        let (left_in, right_in) = (left.clone(), right.clone());
        pedal.stereo(&off, &mut left, &mut right);
        assert!(left == left_in && right == right_in, "{}", P::NAME);
        pedal.mono(&off, &mut left);
        assert!(left == left_in, "{}", P::NAME);
        assert!(pedal.idle());
    }

    #[test]
    fn test_off_from_the_start_leaves_the_signal_untouched() {
        every_pedal!(off_from_the_start_leaves_the_signal_untouched);
    }

    fn off_is_transparent_once_it_has_faded<P: Pedal>() {
        let (on, off) = (P::settings(true, P::RATE_HZ.1, 1.0), P::settings(false, P::RATE_HZ.1, 1.0));
        for (sample_rate, stereo) in PLACES {
            let input = noise(0.5, seconds(0.3, sample_rate));
            let mut pedal = P::at(sample_rate);
            let played = run(&mut pedal, &on, &input, stereo, BLOCK);
            assert!(played[0] != input, "{}", P::NAME);

            // Switched off it fades, and what is still in it rings out
            let fading = run(&mut pedal, &off, &input[..seconds(0.2, sample_rate)], stereo, BLOCK);
            assert!(fading[0] != input[..fading[0].len()], "{}", P::NAME);
            assert!(pedal.idle(), "{}", P::NAME);

            for channel in run(&mut pedal, &off, &input, stereo, BLOCK) {
                assert!(channel == input, "{}", P::NAME);
            }
        }
    }

    #[test]
    fn test_off_is_transparent_once_it_has_faded() {
        every_pedal!(off_is_transparent_once_it_has_faded);
    }

    #[test]
    fn test_depth_zero_leaves_the_dry_signal_untouched() {
        // The chorus and the flanger add to the dry signal. With nothing to add, it is all
        // there is, on from the start or with the dial turned down while playing
        fn check<P: Pedal>() {
            for (sample_rate, stereo) in PLACES {
                let input = noise(0.5, seconds(1.0, sample_rate));
                let mut pedal = P::at(sample_rate);
                for channel in run(&mut pedal, &P::settings(true, 1.0, 0.0), &input, stereo, BLOCK) {
                    assert!(channel == input, "{}", P::NAME);
                }
                run(&mut pedal, &P::settings(true, 1.0, 1.0), &input, stereo, BLOCK);
                run(&mut pedal, &P::settings(true, 1.0, 0.0), &input, stereo, BLOCK);
                for channel in run(&mut pedal, &P::settings(true, 1.0, 0.0), &input, stereo, BLOCK) {
                    assert!(channel == input, "{}", P::NAME);
                }
            }
        }
        check::<Chorus>();
        check::<Flanger>();
    }

    // A click would be a step many times the largest one a steady sine makes. This many
    // times covers the loudest a pedal makes the sine: the flanger at the top of its comb
    const LOUDEST: f32 = 3.0;

    fn switching_does_not_click<P: Pedal>() {
        for (sample_rate, stereo) in PLACES {
            let input = sine(220.0, 0.5, sample_rate, seconds(1.5, sample_rate));
            let mut pedal = P::at(sample_rate);
            // It has been played through and left on in a silence before
            let on = P::settings(true, P::defaults_rate(), 1.0);
            run(&mut pedal, &on, &input[..seconds(0.05, sample_rate)], stereo, BLOCK);
            run(&mut pedal, &on, &vec![0.0; seconds(0.3, sample_rate)], stereo, BLOCK);

            let mut output = vec![Vec::new(); 2];
            for (index, piece) in input.chunks(BLOCK).enumerate() {
                // Off at first, then on and off again every 150 ms while the sine goes on
                let on = (index * BLOCK / seconds(0.15, sample_rate)) % 2 == 1;
                let channels = run(&mut pedal, &P::settings(on, P::defaults_rate(), 1.0), piece, stereo, BLOCK);
                for (output, channel) in output.iter_mut().zip(channels) {
                    output.extend(channel);
                }
            }
            let limit = LOUDEST * 0.5 * std::f32::consts::TAU * 220.0 / sample_rate;
            for channel in &output {
                assert!(largest_step(channel) < limit, "{} at {} Hz: step {} against {}", P::NAME, sample_rate, largest_step(channel), limit);
            }
            assert!(output[0] != input, "{}", P::NAME);
        }
    }

    #[test]
    fn test_switching_does_not_click() {
        every_pedal!(switching_does_not_click);
    }

    fn dial_jumps_do_not_click<P: Pedal>() {
        let (slowest, fastest) = P::RATE_HZ;
        let jumps = [(slowest, 0.0), (fastest, 1.0), (P::defaults_rate(), 0.5), (fastest, 0.0), (slowest, 1.0)];
        for (sample_rate, stereo) in PLACES {
            let input = sine(220.0, 0.5, sample_rate, seconds(2.0, sample_rate));
            let mut pedal = P::at(sample_rate);
            let mut output = vec![Vec::new(); 2];
            for (index, piece) in input.chunks(BLOCK).enumerate() {
                let (rate, depth) = jumps[(index * BLOCK / seconds(0.1, sample_rate)) % jumps.len()];
                let channels = run(&mut pedal, &P::settings(true, rate, depth), piece, stereo, BLOCK);
                for (output, channel) in output.iter_mut().zip(channels) {
                    output.extend(channel);
                }
            }
            let limit = LOUDEST * 0.5 * std::f32::consts::TAU * 220.0 / sample_rate;
            for channel in &output {
                assert!(largest_step(channel) < limit, "{} at {} Hz: step {} against {}", P::NAME, sample_rate, largest_step(channel), limit);
            }
        }
    }

    #[test]
    fn test_dial_jumps_do_not_click() {
        every_pedal!(dial_jumps_do_not_click);
    }

    fn rate_jump_goes_on_from_the_same_place<P: Pedal>() {
        let (slowest, fastest) = P::RATE_HZ;
        let (low, high) = swept_range::<P>(SAMPLE_RATE, &P::settings(true, 1.0, 1.0), false);
        let mut pedal = P::at(SAMPLE_RATE);
        let mut swept = Vec::new();
        for (index, sample) in noise(0.1, 96000).into_iter().enumerate() {
            let rate = if index < 48000 { slowest } else { fastest };
            pedal.mono(&P::settings(true, rate, 1.0), &mut [sample]);
            swept.push(pedal.swept(0));
        }
        // A sweep that started anew would jump by a good part of its range. A few times
        // the steepest a sine of that range is at the fast rate is far less
        let limit = 4.0 * (high - low) * std::f32::consts::PI * fastest / SAMPLE_RATE;
        assert!(largest_step(&swept) < limit, "{}: step of {} {} against {}", P::NAME, largest_step(&swept), P::UNIT, limit);
        let (_, late) = swept.split_at(72000);
        assert!(range(late).1 - range(late).0 > 0.1 * (high - low), "{}", P::NAME);
    }

    #[test]
    fn test_rate_jump_goes_on_from_the_same_place() {
        every_pedal!(rate_jump_goes_on_from_the_same_place);
    }

    fn output_does_not_depend_on_block_size<P: Pedal>() {
        // Playing, a silence long enough to fall idle in, playing again, and the pedal
        // switched off in the middle of that
        let mut input = guitar_di(SAMPLE_RATE)[..48000].to_vec();
        input.extend(vec![0.0; 14400]);
        input.extend(noise(0.3, 19200));
        let (played, rest) = input.split_at(input.len() - 9600);
        let (on, off) = (P::settings(true, P::defaults_rate(), 1.0), P::settings(false, P::defaults_rate(), 1.0));
        let both = |block: usize, stereo: bool| {
            let mut pedal = P::at(SAMPLE_RATE);
            let mut output = run(&mut pedal, &on, played, stereo, block);
            output.extend(run(&mut pedal, &off, rest, stereo, block));
            output
        };

        for stereo in [false, true] {
            let whole = both(input.len(), stereo);
            assert!(whole[0][..played.len()] != *played, "{}", P::NAME);
            for block in [1, 7, 32, 64, 128] {
                assert!(both(block, stereo) == whole, "{}: block of {}, stereo {}", P::NAME, block, stereo);
            }
        }
    }

    #[test]
    fn test_output_does_not_depend_on_block_size() {
        every_pedal!(output_does_not_depend_on_block_size);
    }

    fn sweep_turns_at_the_set_rate<P: Pedal>() {
        for sample_rate in [48000.0, 192_000.0, MAX_RATE_HZ] {
            for rate in [2.0, P::RATE_HZ.1] {
                let (measured, _) = measured_rate::<P>(sample_rate, &P::settings(true, rate, 1.0), false);
                assert!((measured / rate - 1.0).abs() < 0.002, "{} at {} Hz: {} Hz for {}", P::NAME, sample_rate, measured, rate);
            }
        }
        // Behind the cabinet the right side is a quarter turn ahead, but for the tremolo
        let (measured, ahead) = measured_rate::<P>(SAMPLE_RATE, &P::settings(true, 2.0, 1.0), true);
        assert!((measured / 2.0 - 1.0).abs() < 0.002, "{}: {} Hz", P::NAME, measured);
        let expected = if P::WIDE { 0.25 } else { 0.0 };
        assert!((ahead - expected + 0.5).rem_euclid(1.0) - 0.5 < 0.005, "{}: right is {} turns ahead", P::NAME, ahead);
        assert!((ahead - expected + 0.5).rem_euclid(1.0) - 0.5 > -0.005, "{}: right is {} turns ahead", P::NAME, ahead);
    }

    #[test]
    fn test_sweep_turns_at_the_set_rate_at_every_sample_rate() {
        every_pedal!(sweep_turns_at_the_set_rate);
    }

    #[test]
    fn test_sounds_the_same_at_every_sample_rate() {
        // The same chords in front of the amp at three rates: as loud, and as much of
        // them in the lows, the mids and the highs
        let rates = [48000.0, 176_400.0, 192_000.0];
        let inputs = rates.map(|rate| same_chords(rate, 2.0));
        fn check<P: Pedal>(rates: &[f32; 3], inputs: &[Vec<f32>; 3]) {
            let measured: Vec<[f32; 4]> = (0..3)
                .map(|index| {
                    let output = run_mono(&mut P::at(rates[index]), &P::settings(true, 2.0, 1.0), &inputs[index], 128);
                    let band = |from_hz: f32, to_hz: f32| spectrum_level_db(&output, rates[index], from_hz, to_hz);
                    [loudness_lufs(&[&output], rates[index]), band(60.0, 300.0), band(300.0, 1500.0), band(1500.0, 6000.0)]
                })
                .collect();
            for other in &measured[1..] {
                for (here, there) in measured[0].iter().zip(other) {
                    assert!((here - there).abs() < 0.3, "{}: {:?} against {:?}", P::NAME, measured[0], other);
                }
            }
        }
        check::<Chorus>(&rates, &inputs);
        check::<Flanger>(&rates, &inputs);
        check::<Phaser>(&rates, &inputs);
        check::<Tremolo>(&rates, &inputs);
    }

    fn output_is_bounded_whatever_goes_in<P: Pedal>() {
        for (sample_rate, stereo) in [(44100.0, true), (48000.0, false), (MAX_RATE_HZ, false)] {
            // Full scale, and squares in tune with the shortest and the longest delay
            let mut signal = noise(1.0, seconds(0.2, sample_rate));
            for period_ms in [1.0, 12.0] {
                let half = seconds(0.0005 * period_ms, sample_rate);
                signal.extend((0..seconds(0.3, sample_rate)).map(|index| if (index / half) % 2 == 0 { 1.0 } else { -1.0 }));
            }
            for (rate, depth) in [(P::RATE_HZ.0, 1.0), (P::RATE_HZ.1, 1.0), (P::RATE_HZ.1 * 100.0, 5.0), (0.0, -1.0)] {
                for channel in run(&mut P::at(sample_rate), &P::settings(true, rate, depth), &signal, stereo, BLOCK) {
                    assert!(channel.iter().all(|sample| sample.is_finite()), "{}", P::NAME);
                    assert!(peak(&channel) < 4.0, "{} at {} Hz, rate {}: peak {}", P::NAME, sample_rate, rate, peak(&channel));
                }
            }
        }
    }

    #[test]
    fn test_output_is_bounded_whatever_goes_in() {
        every_pedal!(output_is_bounded_whatever_goes_in);
    }

    fn adds_no_offset<P: Pedal>() {
        // Whole turns of the sweep and whole periods of the tone
        let input = sine(110.0, 0.3, SAMPLE_RATE, 3 * 48000);
        for stereo in [false, true] {
            for channel in run(&mut P::at(SAMPLE_RATE), &P::settings(true, 2.0, 1.0), &input, stereo, BLOCK) {
                let offset = channel[48000..].iter().sum::<f32>() / 96000.0;
                assert!(offset.abs() < 1e-4, "{}: offset of {}", P::NAME, offset);
            }
        }
    }

    #[test]
    fn test_adds_no_offset() {
        every_pedal!(adds_no_offset);
    }

    fn rings_out_to_silence_and_is_not_slower_there<P: Pedal>() {
        // Denormal numbers in what rings out would make it many times slower
        let on = P::settings(true, P::defaults_rate(), 1.0);
        for (sample_rate, stereo) in PLACES {
            let silence = vec![0.0; seconds(0.3, sample_rate)];
            // Denormal numbers slow every measurement down, a machine that is busy with something
            // else (a build server running the other tests) only now and then: one that fails is
            // taken again
            let mut attempts = 0;
            let mut pedal = loop {
                let mut pedal = P::at(sample_rate);
                let playing = time_ns(&mut pedal, &on, &noise(0.5, seconds(0.25, sample_rate)), stereo);
                let mut slowest: f64 = 0.0;
                for _ in 0..3 {
                    slowest = slowest.max(time_ns(&mut pedal, &on, &silence, stereo));
                }
                if slowest < playing * SLOWED_DOWN {
                    break pedal;
                }
                attempts += 1;
                assert!(attempts < 3, "{}: {:.1} ns per sample playing, {:.1} ns in silence", P::NAME, playing, slowest);
            };

            assert!(pedal.idle(), "{}", P::NAME);
            for channel in run(&mut pedal, &on, &silence, stereo, BLOCK) {
                assert!(channel.iter().all(|&sample| sample == 0.0), "{}", P::NAME);
            }
            // Playing again after the silence wakes it
            let again = run(&mut pedal, &on, &noise(0.5, 4800), stereo, BLOCK);
            assert!(again[0] != noise(0.5, 4800), "{}", P::NAME);
        }
    }

    #[test]
    fn test_rings_out_to_silence_and_is_not_slower_there() {
        every_pedal!(rings_out_to_silence_and_is_not_slower_there);
    }

    fn off_and_rung_out_costs_next_to_nothing<P: Pedal>() {
        let (on, off) = (P::settings(true, P::defaults_rate(), 1.0), P::settings(false, P::defaults_rate(), 1.0));
        let input = noise(0.5, 48000);
        let mut pedal = P::at(SAMPLE_RATE);
        let playing = time_ns(&mut pedal, &on, &input, true);
        run(&mut pedal, &off, &input[..14400], true, BLOCK);
        assert!(pedal.idle(), "{}", P::NAME);
        let resting = time_ns(&mut pedal, &off, &input, true);
        assert!(resting < 0.5 * playing, "{}: {:.1} ns per sample on, {:.1} ns off", P::NAME, playing, resting);
    }

    #[test]
    fn test_off_and_rung_out_costs_next_to_nothing() {
        every_pedal!(off_and_rung_out_costs_next_to_nothing);
    }

    fn nothing_is_allocated_after_new<P: Pedal>() {
        let mut pedal = P::at(SAMPLE_RATE);
        let before = pedal.buffers();
        for (sample_rate, stereo) in [(44100.0, true), (MAX_RATE_HZ, false), (192_000.0, true)] {
            pedal.retune(sample_rate);
            for (index, piece) in noise(0.5, seconds(0.5, sample_rate)).chunks(BLOCK).enumerate() {
                let rate = [P::RATE_HZ.0, P::RATE_HZ.1][(index / 60) % 2];
                run(&mut pedal, &P::settings((index / 100) % 2 == 0, rate, 1.0), piece, stereo, BLOCK);
            }
            pedal.start_over();
            assert_eq!(pedal.buffers(), before, "{}", P::NAME);
        }
    }

    #[test]
    fn test_nothing_is_allocated_after_new() {
        every_pedal!(nothing_is_allocated_after_new);
    }

    fn reset_starts_over<P: Pedal>() {
        let input = guitar_di(SAMPLE_RATE)[..38400].to_vec();
        let setting = P::settings(true, P::RATE_HZ.1, 1.0);
        for stereo in [false, true] {
            let mut pedal = P::at(SAMPLE_RATE);
            let first = run(&mut pedal, &setting, &input, stereo, BLOCK);
            pedal.start_over();
            assert!(pedal.idle(), "{}", P::NAME);
            assert!(run(&mut pedal, &setting, &input, stereo, BLOCK) == first, "{}", P::NAME);
            // And a new sample rate starts over as well
            pedal.retune(96000.0);
            pedal.retune(SAMPLE_RATE);
            assert!(run(&mut pedal, &setting, &input, stereo, BLOCK) == first, "{}", P::NAME);
        }
    }

    #[test]
    fn test_reset_starts_over() {
        every_pedal!(reset_starts_over);
    }

    fn report<P: Pedal>() {
        let default = P::defaults();
        let rate = P::rate_of(&default);
        let deepest = P::settings(true, rate, 1.0);
        println!("{}", P::NAME);

        let rates: Vec<String> = [48000.0, 192_000.0, MAX_RATE_HZ].iter().map(|&sample_rate| format!("{:.4}", measured_rate::<P>(sample_rate, &default, false).0)).collect();
        let (_, ahead) = measured_rate::<P>(SAMPLE_RATE, &default, true);
        println!("  rate       set {rate} Hz, measured {} Hz at 48 / 192 / 384 kHz; right {ahead:.3} turns ahead", rates.join(" / "));
        for (name, setting) in [("depth 5", &default), ("depth 10", &deepest)] {
            let ranges: Vec<String> = [48000.0, 192_000.0, MAX_RATE_HZ]
                .iter()
                .map(|&sample_rate| {
                    let (low, high) = swept_range::<P>(sample_rate, setting, false);
                    format!("{low:.3} to {high:.3}")
                })
                .collect();
            println!("  sweep      {name}: {} {} at 48 / 192 / 384 kHz", ranges.join(" / "), P::UNIT);
        }
        for (name, setting) in [("depth 5", &default), ("depth 10", &deepest)] {
            println!(
                "  level      {name}: {:+.2} LU on one channel, {:+.2} LU on two, {:+.2} LU in their mono sum, against off",
                level_change_lu::<P>(setting, false),
                level_change_lu::<P>(setting, true),
                mono_sum_change_lu::<P>(setting)
            );
        }
        for (name, setting) in [("depth 5", &default), ("depth 10", &deepest)] {
            println!("  stereo     {name}: left against right {:.2}", stereo_correlation::<P>(setting));
            // The tremolo has no notches, and its dips are its sweep
            if P::WIDE {
                let (one, one_hz) = deepest_notch_db::<P>(setting, false);
                let (sum, sum_hz) = deepest_notch_db::<P>(setting, true);
                println!("  notch      {name}: {one:.1} dB at {one_hz:.0} Hz in one channel, {sum:.1} dB at {sum_hz:.0} Hz in the mono sum");
            }
        }
        let costs: Vec<String> = [(48000.0, true), (192_000.0, false), (MAX_RATE_HZ, false)]
            .iter()
            .map(|&(sample_rate, stereo)| {
                let mut pedal = P::at(sample_rate);
                let input = noise(0.1, seconds(4.0, sample_rate));
                format!("{:.1}", time_ns(&mut pedal, &deepest, &input, stereo))
            })
            .collect();
        println!("  cost       {} ns per sample: two channels at 48 kHz / one at 192 kHz / one at 384 kHz", costs.join(" / "));
    }

    /// Prints what each pedal does, measured: the rate and range of its sweep, how much
    /// louder it makes clean playing, how wide it is and how deep its notches get, and
    /// what it costs. Run with:
    /// cargo test -p amp --release mods_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn mods_report() {
        pin_thread();
        every_pedal!(report);
    }

    /// Writes stereo WAV files to target/renders for listening: clean playing through each
    /// pedal as it is when new and at Depth 10 (amp_mod_*.wav). Run with:
    /// cargo test -p amp --release render_mods -- --ignored
    #[test]
    #[ignore]
    fn render_mods() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();
        let input = clean_di(SAMPLE_RATE);
        write_wav(&dir.join("amp_mod_dry.wav"), &[&input, &input], SAMPLE_RATE as u32);

        fn render<P: Pedal>(dir: &std::path::Path, input: &[f32]) {
            let default = P::defaults();
            let renders = [("default", default), ("depth10", P::settings(true, P::rate_of(&default), 1.0))];
            for (name, setting) in renders {
                let (left, right) = run_stereo(&mut P::at(SAMPLE_RATE), &setting, input, 32);
                let file = format!("amp_mod_{}_{}.wav", P::NAME.to_lowercase(), name);
                write_wav(&dir.join(file), &[&left, &right], SAMPLE_RATE as u32);
            }
        }
        render::<Chorus>(&dir, &input);
        render::<Flanger>(&dir, &input);
        render::<Phaser>(&dir, &input);
        render::<Tremolo>(&dir, &input);
        println!("Wrote renders to {}", dir.display());
    }
}
