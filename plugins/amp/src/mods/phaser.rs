//! Phaser: four all-pass stages in a row, mixed half and half with the dry signal. Where
//! the stages turn the signal upside down the two cancel: two notches that sweep up and
//! down together. Behind the cabinet the two sides sweep a quarter turn apart

use super::lfo::Lfo;
use super::{Dial, Fade, TURNS_APART};
use crate::delay::SILENCE;
use crate::dsp::db_to_gain;

pub const RATE_MIN_HZ: f32 = 0.1;
pub const RATE_MAX_HZ: f32 = 8.0;

const STAGES: usize = 4;

// The stages turn the signal by a quarter turn each at this frequency: the middle of the
// sweep, between 200 Hz and 1.6 kHz. The notches are at 0.41 and 2.41 times it
const CENTRE_HZ: f32 = 565.7;
// Octaves to either side at Depth 10. The sweep is a sine in octaves, not in Hz
const SWEEP_OCTAVES: f32 = 1.5;

// Share of what comes out of the stages that goes back into them. It lifts what is between
// the notches by 1.3 dB and leaves the notches 20 dB deep
const FEEDBACK: f32 = 0.25;

// The notches take this much loudness out of clean playing, and the output is turned up
// by it: what is between the notches is that much louder than without the pedal
const MAKEUP_DB: f32 = 3.5;

// How fast the width of the sweep follows the Depth dial
const WIDTH_SMOOTH_MS: f32 = 100.0;

/// What the knobs say, read once per call. The phaser smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhaserSettings {
    pub on: bool,
    /// `RATE_MIN_HZ` to `RATE_MAX_HZ`
    pub rate_hz: f32,
    /// Width of the sweep, 0.0 to 1.0. At 0.0 the notches stand still
    pub depth: f32,
}

impl Default for PhaserSettings {
    fn default() -> Self {
        Self {
            on: false,
            rate_hz: 0.5,
            depth: 0.5,
        }
    }
}

/// Four stages for each channel. In front of the amp only the first four run
pub struct Phaser {
    // What each stage holds of earlier input
    stages: [[f32; STAGES]; 2],
    lfo: Lfo,
    // Share of the phased signal in the output: 1.0 on, 0.0 off, which is the input itself
    mix: Fade,
    // Where the Depth dial is, on its way to the setting
    width: Dial,

    makeup: f32,
    // Half a turn per sample at 1 Hz, in radians
    half_turn: f32,

    // Nothing of earlier input is in the stages, and either the phaser is off or the
    // input is silent
    idle: bool,
}

/// 2 to the power of `octaves`, within 0.01 % for an octave and a half either way
fn octaves_up(octaves: f32) -> f32 {
    // The square of the series for half the exponent
    let x = 0.5 * std::f32::consts::LN_2 * octaves;
    let root = 1.0 + x * (1.0 + x * (0.5 + x * (1.0 / 6.0 + x * (1.0 / 24.0 + x * (1.0 / 120.0)))));
    root * root
}

/// The tangent of a small angle: within a millionth up to 0.2
fn tangent(angle: f32) -> f32 {
    let square = angle * angle;
    angle * (1.0 + square * (1.0 / 3.0 + square * (2.0 / 15.0)))
}

impl Phaser {
    /// Allocates nothing: the phaser has no buffers
    pub fn new() -> Self {
        let mut phaser = Self {
            stages: [[0.0; STAGES]; 2],
            lfo: Lfo::new(),
            mix: Fade::new(),
            width: Dial::new(),
            makeup: db_to_gain(MAKEUP_DB),
            half_turn: 0.0,
            idle: true,
        };
        phaser.set_sample_rate(48000.0);
        phaser
    }

    /// The rate the pedal runs at, which starts it over. Never allocates: it may be called on the audio thread
    /// when the pedal is moved between its two places
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.lfo.set_sample_rate(sample_rate);
        self.mix.set_sample_rate(sample_rate);
        self.width.set_time(WIDTH_SMOOTH_MS, sample_rate);
        self.half_turn = std::f32::consts::PI / sample_rate;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.lfo.reset();
        self.mix.snap(false);
        self.rest();
    }

    /// For when nothing is left to hear
    fn rest(&mut self) {
        self.stages = [[0.0; STAGES]; 2];
        self.idle = true;
    }

    /// True when nothing of earlier input is left in it (then processing costs next to nothing)
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// In front of the amp: one channel, at the amp's oversampled rate
    pub fn process_mono(&mut self, settings: &PhaserSettings, block: &mut [f32]) {
        self.process(settings, [block]);
    }

    /// Behind the cabinet: two channels at the host's rate
    pub fn process_stereo(&mut self, settings: &PhaserSettings, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        let len = left.len().min(right.len());
        self.process(settings, [&mut left[..len], &mut right[..len]]);
    }

    /// Where a channel's stages are in their sweep now, in Hz
    fn stage_hz(&self, channel: usize) -> f32 {
        CENTRE_HZ * octaves_up(SWEEP_OCTAVES * self.width.value() * self.lfo.sine(TURNS_APART[channel]))
    }

    /// Phases a block in place, each channel by itself
    fn process<const N: usize>(&mut self, settings: &PhaserSettings, mut channels: [&mut [f32]; N]) {
        let rate = settings.rate_hz.clamp(RATE_MIN_HZ, RATE_MAX_HZ);
        let width = settings.depth.clamp(0.0, 1.0);

        for index in 0..channels[0].len() {
            if self.idle {
                // The input is the output until the phaser is on and there is something
                // to play. When it is switched on later, in the middle of a note, it fades in
                if !settings.on {
                    self.mix.snap(false);
                    return;
                }
                if channels.iter().all(|channel| channel[index].abs() < SILENCE) {
                    continue;
                }
                // Nothing is heard yet, so there is nothing to smooth
                self.width.snap(width);
                self.lfo.snap(rate);
                self.idle = false;
            }

            let mix = self.mix.next(settings.on);
            self.width.next(width);
            self.lfo.step(rate);

            let mut quiet = true;
            for (number, channel) in channels.iter_mut().enumerate() {
                let input = channel[index];
                let tangent = tangent(self.half_turn * self.stage_hz(number));
                let coeff = (tangent - 1.0) / (tangent + 1.0);
                let held = &mut self.stages[number];

                // What comes out of the last stage is the input times `through` plus what
                // the stages hold. With that the feedback is solved for this very sample,
                // so the notches are where they should be at every sample rate
                let square = coeff * coeff;
                let through = square * square;
                let ringing = (held[0] * coeff + held[1]) * square + (held[2] * coeff + held[3]);
                let phased = (through * input + ringing) / (1.0 - FEEDBACK * through);

                let mut signal = input + FEEDBACK * phased;
                for held in held.iter_mut() {
                    let output = coeff * signal + *held;
                    *held = signal - coeff * output;
                    signal = output;
                }

                channel[index] = input + mix * (self.makeup * 0.5 * (input + signal) - input);
                quiet &= input.abs() < SILENCE && held.iter().all(|held| held.abs() < SILENCE);
            }

            // Before what the stages hold gets small enough to slow the processor down
            if quiet || (!settings.on && mix == 0.0) {
                self.mix.snap(settings.on);
                self.rest();
            }
        }
    }

    /// Where a channel's stages are in their sweep, in Hz
    #[cfg(test)]
    pub(crate) fn swept(&self, channel: usize) -> f32 {
        self.stage_hz(channel)
    }

    /// Address and capacity of every buffer: none
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<(usize, usize)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::tests::*;
    use crate::test_util::*;

    fn settings(rate_hz: f32, depth: f32) -> PhaserSettings {
        PhaserSettings { on: true, rate_hz, depth }
    }

    #[test]
    fn test_shortcuts_are_close_enough() {
        for step in -150..=150 {
            let octaves = step as f32 * 0.01;
            assert!((octaves_up(octaves) / octaves.exp2() - 1.0).abs() < 1e-4, "{octaves} octaves");
            let angle = step as f32 * 0.0013;
            assert!((tangent(angle) - angle.tan()).abs() < 1e-6, "Tangent of {angle}");
        }
    }

    #[test]
    fn test_stages_sweep_from_200_to_1600_hz_at_every_rate() {
        for sample_rate in [48000.0, 192_000.0, 384_000.0] {
            for (depth, from_hz, to_hz) in [(1.0, 200.0, 1600.0), (0.5, 336.4, 951.4), (0.0, 565.7, 565.7)] {
                for stereo in [false, true] {
                    let (low, high) = swept_range::<Phaser>(sample_rate, &settings(2.0, depth), stereo);
                    assert!((low / from_hz - 1.0).abs() < 0.002 && (high / to_hz - 1.0).abs() < 0.002, "{sample_rate} Hz, depth {depth}: {low} to {high} Hz");
                }
            }
        }
    }

    /// Level of a sine through the phaser with its notches standing still, against the sine
    fn gain_db(freq_hz: f32, sample_rate: f32) -> f32 {
        let input = sine(freq_hz, 0.25, sample_rate, (0.3 * sample_rate) as usize);
        let output = run_mono(&mut Phaser::at(sample_rate), &settings(1.0, 0.0), &input, 64);
        let from = output.len() / 2;
        to_db(rms(&output[from..]) / rms(&input[from..])) - MAKEUP_DB
    }

    #[test]
    fn test_notches_are_where_the_stages_put_them_at_every_rate() {
        // 22.5 and 67.5 degrees in each stage: half a turn and one and a half in all
        let notches_hz = [CENTRE_HZ * 0.41421, CENTRE_HZ * 2.41421];
        for sample_rate in [44100.0, 48000.0, 192_000.0, 384_000.0] {
            for notch_hz in notches_hz {
                let depth = gain_db(notch_hz, sample_rate);
                assert!((depth + 20.0).abs() < 0.5, "{sample_rate} Hz: {depth} dB at {notch_hz} Hz");
                // And nowhere near as deep less than an octave to either side
                for beside_hz in [notch_hz * 0.6, notch_hz / 0.6] {
                    assert!(gain_db(beside_hz, sample_rate) > -9.0, "{sample_rate} Hz: {} dB at {beside_hz} Hz", gain_db(beside_hz, sample_rate));
                }
            }
            // Between the notches the feedback lifts the signal by 1.3 dB, and below and
            // above them by a little less
            let lift = gain_db(CENTRE_HZ, sample_rate);
            assert!((1.2..1.4).contains(&lift), "{sample_rate} Hz: {lift} dB between the notches");
            for outside_hz in [30.0, 12000.0] {
                let lift = gain_db(outside_hz, sample_rate);
                assert!((0.5..1.4).contains(&lift), "{sample_rate} Hz: {lift} dB at {outside_hz} Hz");
            }
        }
    }

    #[test]
    fn test_level_is_about_the_same_on_and_off() {
        for stereo in [false, true] {
            for setting in [Phaser::defaults(), settings(0.5, 1.0)] {
                let change = level_change_lu::<Phaser>(&setting, stereo);
                assert!(change.abs() < 1.0, "Depth {}, stereo {stereo}: {change} LU", setting.depth);
            }
            // With the notches standing still in the middle of the guitar it is quieter
            let still = level_change_lu::<Phaser>(&settings(0.5, 0.0), stereo);
            assert!((-2.0..0.0).contains(&still), "Depth 0, stereo {stereo}: {still} LU");
        }
    }

    #[test]
    fn test_stereo_is_wide_and_its_mono_sum_is_no_deeper_than_one_channel() {
        let setting = settings(0.5, 1.0);
        let width = stereo_correlation::<Phaser>(&setting);
        assert!((0.3..0.97).contains(&width), "Left against right {width}");
        // Twice in every turn the two sides are at the same place in their sweeps, and
        // the sum has the notch of one of them
        let (one, _) = deepest_notch_db::<Phaser>(&setting, false);
        let (sum, at_hz) = deepest_notch_db::<Phaser>(&setting, true);
        assert!(one > -20.5 + MAKEUP_DB, "One channel {one} dB");
        assert!(sum > one - 0.5, "Mono sum {sum} dB at {at_hz} Hz, one channel {one} dB");
    }

    #[test]
    fn test_both_channels_are_phased_by_themselves() {
        // What an effect in front of the phaser put on one side only stays there
        let input = guitar_di(48000.0)[..24000].to_vec();
        let (mut left, mut right) = (input.clone(), vec![0.0; input.len()]);
        Phaser::at(48000.0).process_stereo(&settings(0.5, 0.5), &mut left, &mut right);
        assert!(rms(&left) > 0.5 * rms(&input));
        assert!(right.iter().all(|&sample| sample == 0.0));
    }
}
