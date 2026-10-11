//! Tremolo: the level goes down and up again in the shape of a sine, the same on both sides

use super::lfo::Lfo;
use super::{Dial, Fade};

pub const RATE_MIN_HZ: f32 = 0.5;
pub const RATE_MAX_HZ: f32 = 12.0;

// How much of the level is taken away at the bottom of the sine at Depth 10: down to
// -26 dB, not to silence
const DEEPEST: f32 = 0.95;

// Share, in dB, of the level the dips take out on average that is given back. All of it
// would put the tops 4 dB over the level without the pedal at Depth 10; half of it leaves
// the tremolo 1 dB quieter than off at Depth 5 with tops 1 dB over
const MAKEUP_SHARE: f32 = 0.5;

/// What the knobs say, read once per call. The tremolo smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TremoloSettings {
    pub on: bool,
    /// `RATE_MIN_HZ` to `RATE_MAX_HZ`
    pub rate_hz: f32,
    /// How far the level dips, 0.0 to 1.0
    pub depth: f32,
}

impl Default for TremoloSettings {
    fn default() -> Self {
        Self {
            on: false,
            rate_hz: 5.0,
            depth: 0.5,
        }
    }
}

pub struct Tremolo {
    lfo: Lfo,
    // Share of the tremolo in the output: 1.0 on, 0.0 off, which is the input itself
    mix: Fade,
    // Where the Depth dial is, on its way to the setting: the share of the level the dips
    // take away, and the gain that goes with it
    dip: Dial,
    makeup: Dial,
}

impl Tremolo {
    /// Allocates nothing: the tremolo has no buffers
    pub fn new() -> Self {
        let mut tremolo = Self {
            lfo: Lfo::new(),
            mix: Fade::new(),
            dip: Dial::new(),
            makeup: Dial::new(),
        };
        tremolo.set_sample_rate(48000.0);
        tremolo
    }

    /// The rate the pedal runs at, which starts it over. Never allocates: it may be called on the audio thread
    /// when the pedal is moved between its two places
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.lfo.set_sample_rate(sample_rate);
        self.mix.set_sample_rate(sample_rate);
        self.dip.set_sample_rate(sample_rate);
        self.makeup.set_sample_rate(sample_rate);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.lfo.reset();
        self.mix.snap(false);
    }

    /// True when nothing of earlier input is left in it: always, a tremolo keeps nothing
    /// of its input. Off and faded out it costs next to nothing
    pub fn is_idle(&self) -> bool {
        true
    }

    /// In front of the amp: one channel, at the amp's oversampled rate
    pub fn process_mono(&mut self, settings: &TremoloSettings, block: &mut [f32]) {
        self.process(settings, [block]);
    }

    /// Behind the cabinet: two channels at the host's rate
    pub fn process_stereo(&mut self, settings: &TremoloSettings, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        let len = left.len().min(right.len());
        self.process(settings, [&mut left[..len], &mut right[..len]]);
    }

    /// What the signal is multiplied by now
    fn gain(&self) -> f32 {
        self.makeup.value() * (1.0 - self.dip.value() * (0.5 - 0.5 * self.lfo.sine(0.0)))
    }

    fn process<const N: usize>(&mut self, settings: &TremoloSettings, mut channels: [&mut [f32]; N]) {
        let rate = settings.rate_hz.clamp(RATE_MIN_HZ, RATE_MAX_HZ);
        let dip = DEEPEST * settings.depth.clamp(0.0, 1.0);
        // The power the dips leave on average, over a whole turn of the sine
        let power = 1.0 - dip + 0.375 * dip * dip;
        let makeup = power.powf(-0.5 * MAKEUP_SHARE);

        for index in 0..channels[0].len() {
            if self.mix.is_out() {
                // The input is the output until the tremolo is on
                if !settings.on {
                    return;
                }
                // Nothing is heard yet, so there is nothing to smooth
                self.dip.snap(dip);
                self.makeup.snap(makeup);
                self.lfo.snap(rate);
            }

            let mix = self.mix.next(settings.on);
            self.dip.next(dip);
            self.makeup.next(makeup);
            self.lfo.step(rate);

            let gain = 1.0 + mix * (self.gain() - 1.0);
            for channel in channels.iter_mut() {
                channel[index] *= gain;
            }
        }
    }

    /// What the signal is multiplied by, on either channel
    #[cfg(test)]
    pub(crate) fn swept(&self, _channel: usize) -> f32 {
        self.gain()
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

    fn settings(rate_hz: f32, depth: f32) -> TremoloSettings {
        TremoloSettings { on: true, rate_hz, depth }
    }

    #[test]
    fn test_level_dips_as_far_as_the_depth_says_at_every_rate() {
        for sample_rate in [48000.0, 192_000.0, 384_000.0] {
            for (depth, dip_db) in [(1.0, -26.0), (0.5, -5.6), (0.0, 0.0)] {
                for stereo in [false, true] {
                    let (low, high) = swept_range::<Tremolo>(sample_rate, &settings(5.0, depth), stereo);
                    let measured = to_db(low / high);
                    assert!((measured - dip_db).abs() < 0.1, "{sample_rate} Hz, depth {depth}: {measured} dB");
                }
            }
        }
    }

    #[test]
    fn test_level_follows_a_sine() {
        // A steady tone, and its level every millisecond
        let input = sine(1000.0, 0.5, 48000.0, 48000);
        let output = run_mono(&mut Tremolo::at(48000.0), &settings(5.0, 1.0), &input, 64);
        let levels: Vec<f32> = output[4800..43200].chunks(48).map(|piece| rms(piece) / rms(&input[..48])).collect();
        let (low, high) = levels.iter().fold((f32::MAX, f32::MIN), |(low, high), &level| (low.min(level), high.max(level)));
        let middle = 0.5 * (low + high);
        // Four turns: as long over the middle as under it, and a top in each
        let over = levels.iter().filter(|&&level| level > middle).count() as f32 / levels.len() as f32;
        assert!((over - 0.5).abs() < 0.03, "{over} of the time over the middle");
        let tops = levels.windows(2).filter(|pair| pair[0] <= middle && pair[1] > middle).count();
        assert!((3..=5).contains(&tops), "{tops} tops in 800 ms");
        assert!((to_db(low / high) + 26.0).abs() < 0.5, "Dips by {} dB", to_db(low / high));
    }

    #[test]
    fn test_depth_zero_is_transparent() {
        let input = guitar_di(48000.0)[..48000].to_vec();
        let mut tremolo = Tremolo::at(48000.0);
        assert_eq!(run_mono(&mut tremolo, &settings(5.0, 0.0), &input, 64), input);
    }

    #[test]
    fn test_both_channels_get_the_same_gain() {
        let input = clean_di(48000.0)[..96000].to_vec();
        let (left, right) = run_stereo(&mut Tremolo::at(48000.0), &settings(5.0, 1.0), &input, 32);
        assert_eq!(left, right);
        // And a side that is silent stays silent
        let (mut left, mut right) = (input.clone(), vec![0.0; input.len()]);
        Tremolo::at(48000.0).process_stereo(&settings(5.0, 1.0), &mut left, &mut right);
        assert!(rms(&left) > 0.2 * rms(&input));
        assert!(right.iter().all(|&sample| sample == 0.0));
    }

    #[test]
    fn test_level_stays_near_the_level_without_it() {
        for stereo in [false, true] {
            let default = level_change_lu::<Tremolo>(&Tremolo::defaults(), stereo);
            assert!((-1.5..0.0).contains(&default), "Depth 5, stereo {stereo}: {default} LU");
            let deepest = level_change_lu::<Tremolo>(&settings(5.0, 1.0), stereo);
            assert!((-3.0..0.0).contains(&deepest), "Depth 10, stereo {stereo}: {deepest} LU");
        }
        // The tops are over the level without the pedal by as much as the average is under
        for (depth, top_db) in [(0.5, 1.07), (1.0, 2.05)] {
            let (_, high) = swept_range::<Tremolo>(48000.0, &settings(5.0, depth), false);
            assert!((to_db(high) - top_db).abs() < 0.05, "Depth {depth}: tops at {} dB", to_db(high));
        }
    }
}
