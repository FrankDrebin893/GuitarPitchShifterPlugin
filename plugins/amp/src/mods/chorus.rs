//! Chorus: a second voice a few milliseconds behind the dry signal that drifts nearer and
//! further, so its pitch wavers against the dry one. Behind the cabinet there is one voice
//! on each side

use super::lfo::Lfo;
use super::line::ModLine;
use super::{middle, Dial, Fade, TURNS_APART};
use crate::delay::SILENCE;
use crate::dsp::filters::{OnePoleHp, OnePoleLp};

pub const RATE_MIN_HZ: f32 = 0.1;
pub const RATE_MAX_HZ: f32 = 6.0;

// The voice sweeps around this delay,
const CENTRE_MS: f32 = 9.5;
// this far to either side at Depth 10: 7 to 12 ms
const SWEEP_MS: f32 = 2.5;
// Up to this rate. Above it the sweep narrows as the rate goes up, so the pitch of the
// voice never wavers by more than 27 cents
const FULL_SWEEP_TO_HZ: f32 = 1.0;

// Level of the voice against the dry signal at Depth 10. It follows the square root of
// the dial: 0.47 at Depth 5
const WET: f32 = 0.66;

// The voice has no lows, which would only beat against the dry ones, and a soft top
const HIGHPASS_HZ: f32 = 120.0;
const LOWPASS_HZ: f32 = 5000.0;

// How fast the width of the sweep follows the dials. The pitch of the voice bends while it
// moves: by 22 cents at most
const WIDTH_SMOOTH_MS: f32 = 200.0;

/// What the knobs say, read once per call. The chorus smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChorusSettings {
    /// Off lets what is in the line ring out
    pub on: bool,
    /// `RATE_MIN_HZ` to `RATE_MAX_HZ`
    pub rate_hz: f32,
    /// Width of the sweep and level of the voice, 0.0 to 1.0. The dry signal stays as it is
    pub depth: f32,
}

impl Default for ChorusSettings {
    fn default() -> Self {
        Self { on: false, rate_hz: 0.8, depth: 0.5 }
    }
}

/// One delay line, fed the middle of the channels and read once for each of them
pub struct Chorus {
    line: ModLine,
    highpass: OnePoleHp,
    lowpass: OnePoleLp,
    lfo: Lfo,
    // Share of the input that reaches the line: 1.0 on, 0.0 off
    feed: Fade,

    // Where the dials are, on their way to the settings: the level of the voice, and the
    // share of the full sweep
    wet: Dial,
    width: Dial,

    // In samples
    centre: f64,
    sweep: f64,
    // The furthest back the voice is read
    reach: usize,

    // Samples in a row that only zeros went into the line
    quiet_run: usize,
    // Nothing is left to hear and the filters are at rest
    idle: bool,
}

impl Chorus {
    /// Allocates every buffer the pedal will ever need, for rates up to `MAX_RATE_HZ`
    pub fn new() -> Self {
        let mut chorus = Self {
            line: ModLine::new(CENTRE_MS + SWEEP_MS),
            highpass: OnePoleHp::new(),
            lowpass: OnePoleLp::new(),
            lfo: Lfo::new(),
            feed: Fade::new(),
            wet: Dial::new(),
            width: Dial::new(),
            centre: 0.0,
            sweep: 0.0,
            reach: 0,
            quiet_run: 0,
            idle: true,
        };
        chorus.set_sample_rate(48000.0);
        chorus
    }

    /// The rate the pedal runs at, which starts it over. Never allocates: it may be called on the audio thread
    /// when the pedal is moved between its two places
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.highpass.set(HIGHPASS_HZ, sample_rate);
        self.lowpass.set(LOWPASS_HZ, sample_rate);
        self.lfo.set_sample_rate(sample_rate);
        self.feed.set_sample_rate(sample_rate);
        self.wet.set_sample_rate(sample_rate);
        self.width.set_time(WIDTH_SMOOTH_MS, sample_rate);
        self.centre = CENTRE_MS as f64 * 0.001 * sample_rate as f64;
        self.sweep = SWEEP_MS as f64 * 0.001 * sample_rate as f64;
        self.reach = (self.centre + self.sweep) as usize + 3;
        self.reset();
    }

    /// Drops what is in the line. Does not allocate
    pub fn reset(&mut self) {
        self.line.clear();
        self.lfo.reset();
        self.feed.snap(false);
        self.rest();
    }

    /// For when nothing is left to hear
    fn rest(&mut self) {
        self.highpass.reset();
        self.lowpass.reset();
        self.quiet_run = 0;
        self.idle = true;
    }

    /// True when nothing of earlier input is left in it (then processing costs next to nothing)
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// In front of the amp: one channel, at the amp's oversampled rate
    pub fn process_mono(&mut self, settings: &ChorusSettings, block: &mut [f32]) {
        self.process(settings, [block]);
    }

    /// Behind the cabinet: two channels at the host's rate
    pub fn process_stereo(
        &mut self,
        settings: &ChorusSettings,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        debug_assert_eq!(left.len(), right.len());
        let len = left.len().min(right.len());
        self.process(settings, [&mut left[..len], &mut right[..len]]);
    }

    /// How far behind the dry signal a channel's voice is now, in samples
    fn delay(&self, channel: usize) -> f64 {
        self.centre + self.sweep * (self.width.value() * self.lfo.sine(TURNS_APART[channel])) as f64
    }

    /// Adds the voices to a block in place
    fn process<const N: usize>(
        &mut self,
        settings: &ChorusSettings,
        mut channels: [&mut [f32]; N],
    ) {
        let rate = settings.rate_hz.clamp(RATE_MIN_HZ, RATE_MAX_HZ);
        let depth = settings.depth.clamp(0.0, 1.0);
        let wet = WET * depth.sqrt();
        let width = depth * (FULL_SWEEP_TO_HZ / rate).min(1.0);

        for index in 0..channels[0].len() {
            let input = middle(&channels, index);
            if self.idle {
                // Nothing to add until the chorus is on and there is something to play. When
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

            for (number, channel) in channels.iter_mut().enumerate() {
                channel[index] += level * self.line.read(self.delay(number));
            }

            let fed = self.lowpass.process(self.highpass.process(input * feed));
            if fed.abs() >= SILENCE {
                self.line.push(fed);
                self.quiet_run = 0;
                continue;
            }
            // Idle once everything the voices can get to is zeros and the filters have
            // rung out
            self.line.push(0.0);
            self.quiet_run += 1;
            if self.quiet_run >= self.reach
                && self.highpass.state().abs() < SILENCE
                && self.lowpass.state().abs() < SILENCE
            {
                self.line.keep_newest(self.quiet_run);
                self.feed.snap(settings.on);
                self.rest();
            }
        }
    }

    /// Where a channel's voice is in its sweep, in ms behind the dry signal
    #[cfg(test)]
    pub(crate) fn swept(&self, channel: usize) -> f32 {
        (self.delay(channel) / self.centre) as f32 * CENTRE_MS
    }

    /// Address and capacity of every buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub(crate) fn buffers(&self) -> Vec<(usize, usize)> {
        vec![self.line.buffer()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::tests::*;
    use crate::test_util::*;

    fn settings(rate_hz: f32, depth: f32) -> ChorusSettings {
        ChorusSettings { on: true, rate_hz, depth }
    }

    #[test]
    fn test_voice_sweeps_from_7_to_12_ms_at_every_rate() {
        for sample_rate in [48000.0, 192_000.0, 384_000.0] {
            for (depth, from_ms, to_ms) in [(1.0, 7.0, 12.0), (0.5, 8.25, 10.75), (0.0, 9.5, 9.5)] {
                for stereo in [false, true] {
                    let (low, high) =
                        swept_range::<Chorus>(sample_rate, &settings(1.0, depth), stereo);
                    assert!(
                        (low - from_ms).abs() < 0.01 && (high - to_ms).abs() < 0.01,
                        "{sample_rate} Hz, depth {depth}: {low} to {high} ms"
                    );
                }
            }
        }
    }

    #[test]
    fn test_voice_arrives_when_the_sweep_says() {
        // A click every 40 ms, and where the voice's copy of each one lands
        for sample_rate in [48000.0, 192_000.0, 384_000.0] {
            let every = (0.04 * sample_rate) as usize;
            let mut input = vec![0.0; 40 * every];
            input.iter_mut().step_by(every).for_each(|sample| *sample = 0.5);
            let mut chorus = Chorus::at(sample_rate);
            let output = run_mono(&mut chorus, &settings(1.0, 1.0), &input, 64);

            let delays_ms: Vec<f32> = output
                .chunks(every)
                .skip(1)
                .map(|piece| {
                    let wet = &piece[1..];
                    let at = (0..wet.len())
                        .max_by(|&a, &b| wet[a].abs().total_cmp(&wet[b].abs()))
                        .unwrap()
                        + 1;
                    at as f32 * 1000.0 / sample_rate
                })
                .collect();
            let (nearest, furthest) =
                delays_ms.iter().fold((f32::MAX, f32::MIN), |(low, high), &delay| {
                    (low.min(delay), high.max(delay))
                });
            // The filters on the voice smear a click over a few samples
            assert!(
                (6.9..7.5).contains(&nearest) && (11.6..12.3).contains(&furthest),
                "{sample_rate} Hz: {nearest} to {furthest} ms"
            );
        }
    }

    #[test]
    fn test_pitch_never_wavers_by_more_than_27_cents() {
        // The speed of the delay, in samples per sample, is the share the pitch is off by
        let cents = |change: f32| 1200.0 * (1.0 + change).log2();
        for rate_hz in [0.5, 1.0, 2.5, RATE_MAX_HZ] {
            let mut chorus = Chorus::at(48000.0);
            let setting = settings(rate_hz, 1.0);
            let mut steepest: f32 = 0.0;
            let mut last = None;
            for _ in 0..(48000.0 * 2.5 / rate_hz) as usize {
                chorus.process_mono(&setting, &mut [0.1]);
                let delay = chorus.delay(0);
                if let Some(last) = last {
                    steepest = steepest.max((delay - last) as f32);
                }
                last = Some(delay);
            }
            let expected = if rate_hz < 1.0 { 27.0 * rate_hz } else { 27.0 };
            assert!(
                (cents(steepest) - expected).abs() < 0.5,
                "{rate_hz} Hz: {} cents",
                cents(steepest)
            );
        }
    }

    #[test]
    fn test_depth_jump_bends_the_pitch_only_a_little() {
        let mut chorus = Chorus::at(48000.0);
        let mut steepest: f64 = 0.0;
        for depth in [0.0, 1.0, 0.0, 1.0] {
            for _ in 0..24000 {
                let before = chorus.delay(0);
                chorus.process_mono(&settings(0.1, depth), &mut [0.1]);
                if !chorus.is_idle() {
                    steepest = steepest.max((chorus.delay(0) - before).abs());
                }
            }
        }
        // 22 cents from the dial and 3 from the sweep itself
        assert!(steepest < 0.0145, "The voice moves {steepest} samples per sample");
    }

    #[test]
    fn test_voice_is_darker_than_the_dry_signal_and_has_no_lows() {
        let mut noise = Noise::new(5);
        let input: Vec<f32> = (0..96000).map(|_| 0.3 * noise.next()).collect();
        let output = run_mono(&mut Chorus::at(48000.0), &settings(0.8, 1.0), &input, 64);
        let wet: Vec<f32> = output.iter().zip(&input).map(|(out, dry)| out - dry).collect();
        let level = to_db(rms(&wet) / rms(&input));
        assert!((-12.0..-6.0).contains(&level), "Voice at {level} dB");
        let highs = band_level_db(&wet, 48000.0, Some(10000.0), None)
            - band_level_db(&input, 48000.0, Some(10000.0), None);
        let lows = band_level_db(&wet, 48000.0, None, Some(60.0))
            - band_level_db(&input, 48000.0, None, Some(60.0));
        assert!(
            highs < -3.0 && lows < -4.0,
            "Highs {highs} dB, lows {lows} dB against the voice as a whole"
        );
    }

    #[test]
    fn test_level_is_about_the_same_on_and_off() {
        for stereo in [false, true] {
            let default = level_change_lu::<Chorus>(&Chorus::defaults(), stereo);
            assert!((0.0..1.0).contains(&default), "Depth 5, stereo {stereo}: {default} LU");
            let deepest = level_change_lu::<Chorus>(&settings(0.8, 1.0), stereo);
            assert!((0.0..1.8).contains(&deepest), "Depth 10, stereo {stereo}: {deepest} LU");
        }
    }

    #[test]
    fn test_stereo_is_wide_and_sums_to_mono_without_deep_notches() {
        let setting = settings(0.5, 1.0);
        let width = stereo_correlation::<Chorus>(&setting);
        assert!((0.5..0.97).contains(&width), "Left against right {width}");
        let (one, _) = deepest_notch_db::<Chorus>(&setting, false);
        let (sum, at_hz) = deepest_notch_db::<Chorus>(&setting, true);
        // A voice at 0.66 against the dry signal takes 9.4 dB out at most
        assert!(one > -9.5, "One channel {one} dB");
        assert!(
            sum > -9.5 && sum > one - 0.5,
            "Mono sum {sum} dB at {at_hz} Hz, one channel {one} dB"
        );
    }
}
