use crate::dsp::filters::{OnePoleHp, ANTI_DENORMAL};
use crate::dsp::shaper::AsymClipper;
use crate::dsp::{curve, db_to_gain, lerp, Ramp};

// Only what is above this corner is amplified and clipped: the low end stays out of the
// clipping and passes underneath it, clean
const BODY_HZ: f32 = 720.0;

// Gain of the clipped path in dB with Drive at 0 and at 10
const DRIVE_DB: [f32; 2] = [12.0, 42.0];

// Where the clipped path tops out, upwards and downwards, against a full-scale input.
// The small difference adds a little second harmonic
const CLIP_LEVEL: [f32; 2] = [0.5, 0.56];

// Corner (-3 dB) of the low-pass after the clipping with Tone at 0 and at 10
const TONE_HZ: [f32; 2] = [1500.0, 7000.0];

// The low-pass is two equal one-pole sections. Each has its corner this many times higher,
// so that together they are 3 dB down at the corner asked for
const TONE_SECTION_RATIO: f32 = 1.554;

// Output gain in dB with Level at 0 and at 10
const LEVEL_DB: [f32; 2] = [-20.0, 20.0];

// Taken off the output in dB across Drive (0, 5, 10), so that Level at 5 is about as loud
// as the pedal switched off
const TRIM_DB: [f32; 3] = [-7.0, -11.5, -12.5];

// Crossfade to and from the untouched signal when the pedal is switched
const FADE_MS: f32 = 10.0;

/// Overdrive pedal in front of the amp: mid-focused, with the low end kept out of the
/// clipping. With little Drive and a lot of Level it tightens and pushes a high gain amp;
/// with more Drive it is an overdrive of its own. Runs at the oversampled rate
pub struct Drive {
    sample_rate: f32,
    body: OnePoleHp,
    gain: Ramp,
    clipper: AsymClipper,
    // Low-pass of two one-pole sections, written out so its coefficient can glide
    tone_coeff: Ramp,
    tone_state: [f32; 2],
    level: Ramp,
    // Share of the pedal in its output: 0.0 switched off, 1.0 on
    mix: Ramp,
    fade_steps: u32,
}

impl Drive {
    pub fn new() -> Self {
        // Antialiased to the second order: with Drive up, the pedal clips harder than any
        // one stage of an amp, and it only costs anything while it is switched on
        let mut clipper = AsymClipper::new();
        clipper.set_limits(CLIP_LEVEL[0], CLIP_LEVEL[1]);
        clipper.set_second_order(true);
        clipper.reset(0.0);
        Self {
            sample_rate: 176_400.0,
            body: OnePoleHp::new(),
            gain: Ramp::new(1.0),
            clipper,
            tone_coeff: Ramp::new(1.0),
            tone_state: [0.0; 2],
            level: Ramp::new(1.0),
            mix: Ramp::new(0.0),
            fade_steps: 1,
        }
    }

    /// `sample_rate` is the rate the pedal runs at (the oversampled one)
    pub fn configure(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.body.set(BODY_HZ, sample_rate);
        self.fade_steps = ((FADE_MS * 0.001 * sample_rate).round() as u32).max(1);
    }

    pub fn reset(&mut self) {
        self.body.reset();
        self.clipper.reset(0.0);
        self.tone_state = [0.0; 2];
    }

    /// Crossfades to the pedal or to the untouched signal. Nothing is reset: what the
    /// filters still hold from before has faded long before the pedal is heard
    pub fn set_on(&mut self, on: bool) {
        let target = if on { 1.0 } else { 0.0 };
        if target != self.mix.target() {
            self.mix.set_target(target, self.fade_steps);
        }
    }

    /// Moves to the dial positions (0.0 to 1.0) over the next `steps` samples
    pub fn set(&mut self, drive: f32, tone: f32, level: f32, steps: u32) {
        let tone_hz = TONE_SECTION_RATIO * TONE_HZ[0] * (TONE_HZ[1] / TONE_HZ[0]).powf(tone);
        let level_db = lerp(LEVEL_DB[0], LEVEL_DB[1], level) + curve(&TRIM_DB, drive);
        self.gain.set_target(db_to_gain(lerp(DRIVE_DB[0], DRIVE_DB[1], drive)), steps);
        self.tone_coeff.set_target(1.0 - (-std::f32::consts::TAU * tone_hz / self.sample_rate).exp(), steps);
        self.level.set_target(db_to_gain(level_db), steps);
        // Not heard, so there is nothing to glide from
        if self.is_idle() {
            self.snap_dials();
        }
    }

    /// Ends the moves started by `set` and `set_on` at once
    pub fn snap(&mut self) {
        self.snap_dials();
        self.mix.snap();
    }

    fn snap_dials(&mut self) {
        self.gain.snap();
        self.tone_coeff.snap();
        self.level.snap();
    }

    /// True while the pedal is off and has faded out: `process` then leaves the signal alone
    pub fn is_idle(&self) -> bool {
        self.mix.value() == 0.0 && self.mix.target() == 0.0
    }

    pub fn process(&mut self, block: &mut [f32]) {
        if self.is_idle() {
            return;
        }
        for sample in block.iter_mut() {
            let dry = *sample;
            let clipped = self.clipper.process(self.body.process(dry) * self.gain.next());
            let coeff = self.tone_coeff.next();
            self.tone_state[0] += coeff * (dry + clipped - self.tone_state[0]) + ANTI_DENORMAL;
            self.tone_state[1] += coeff * (self.tone_state[0] - self.tone_state[1]) + ANTI_DENORMAL;
            let wet = self.tone_state[1] * self.level.next();

            let mix = self.mix.next();
            *sample = if mix >= 1.0 { wet } else { dry + mix * (wet - dry) };
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::test_util::{band_level_db, peak, power_chords, rms, sine, thd_db, to_db};

    const SAMPLE_RATE: f32 = 192_000.0;
    const LEN: usize = 96_000;

    pub fn new_drive(sample_rate: f32, drive: f32, tone: f32, level: f32) -> Drive {
        let mut pedal = Drive::new();
        pedal.configure(sample_rate);
        pedal.set_on(true);
        pedal.set(drive, tone, level, 0);
        pedal.snap();
        pedal
    }

    fn run(drive: f32, tone: f32, level: f32, input: &[f32]) -> Vec<f32> {
        let mut output = input.to_vec();
        new_drive(SAMPLE_RATE, drive, tone, level).process(&mut output);
        output
    }

    fn largest_step(samples: &[f32]) -> f32 {
        samples.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn test_switched_off_it_leaves_the_signal_alone() {
        let input = sine(220.0, 0.3, SAMPLE_RATE, 4800);
        let mut pedal = Drive::new();
        pedal.configure(SAMPLE_RATE);
        pedal.set(1.0, 1.0, 1.0, 128);
        let mut output = input.clone();
        pedal.process(&mut output);
        assert!(pedal.is_idle());
        assert_eq!(input, output);
    }

    #[test]
    fn test_drive_dial_adds_distortion() {
        let input = sine(440.0, 0.178, SAMPLE_RATE, LEN);
        let distortion = |drive: f32| thd_db(&run(drive, 0.5, 0.5, &input)[LEN / 2..], SAMPLE_RATE, 440.0);
        let (low, centre, full) = (distortion(0.0), distortion(0.5), distortion(1.0));
        assert!(low < -26.0, "Drive 0 is not nearly clean: {:.1} dB", low);
        assert!(centre > low + 8.0, "Drive 5 at {:.1} dB, drive 0 at {:.1} dB", centre, low);
        assert!(full > centre + 1.0, "Drive 10 at {:.1} dB, drive 5 at {:.1} dB", full, centre);
        assert!(full > -16.0, "Drive 10: {:.1} dB", full);
    }

    #[test]
    fn test_lows_are_cut_against_the_mids_and_stay_clean() {
        // Quiet enough to be linear
        let gain = |drive: f32, freq_hz: f32| {
            let input = sine(freq_hz, 0.001, SAMPLE_RATE, LEN);
            to_db(rms(&run(drive, 0.5, 0.5, &input)[LEN / 2..]) / rms(&input))
        };
        for drive in [0.0, 0.2, 0.5, 1.0] {
            let (lows, mids) = (gain(drive, 80.0), gain(drive, 1000.0));
            assert!(lows < mids - 8.0, "Drive {}: {:.1} dB at 80 Hz, {:.1} dB at 1 kHz", drive * 10.0, lows, mids);
        }

        // A low note is clipped less than one in the middle of the neck
        let low_note = sine(82.0, 0.178, SAMPLE_RATE, LEN);
        let mid_note = sine(660.0, 0.178, SAMPLE_RATE, LEN);
        let low = thd_db(&run(0.3, 1.0, 0.5, &low_note)[LEN / 2..], SAMPLE_RATE, 82.0);
        let mid = thd_db(&run(0.3, 1.0, 0.5, &mid_note)[LEN / 2..], SAMPLE_RATE, 660.0);
        assert!(low < mid - 6.0, "82 Hz at {:.1} dB, 660 Hz at {:.1} dB", low, mid);
    }

    #[test]
    fn test_tone_dial_opens_the_top() {
        let input = power_chords(SAMPLE_RATE, 0.5);
        let top = |tone: f32| {
            let output = run(0.5, tone, 0.5, &input);
            band_level_db(&output, SAMPLE_RATE, Some(3000.0), None) + to_db(rms(&output))
        };
        let (dark, centre, bright) = (top(0.0), top(0.5), top(1.0));
        assert!(centre > dark + 2.0 && bright > centre + 2.0, "Above 3 kHz: {:.1}, {:.1}, {:.1} dB", dark, centre, bright);

        // And leaves the low end alone
        let low = |tone: f32| to_db(rms(&run(0.0, tone, 0.5, &sine(100.0, 0.001, SAMPLE_RATE, LEN))[LEN / 2..]));
        assert!((low(0.0) - low(1.0)).abs() < 0.5);
    }

    #[test]
    fn test_level_dial_spans_forty_decibels_with_unity_near_the_middle() {
        let input = power_chords(SAMPLE_RATE, 1.0);
        let level = |drive: f32, level: f32| to_db(rms(&run(drive, 0.5, level, &input)) / rms(&input));
        assert!((level(0.3, 1.0) - level(0.3, 0.0) - 40.0).abs() < 0.5);
        for drive in [0.0, 0.3, 0.5] {
            let centre = level(drive, 0.5);
            assert!(centre.abs() < 2.0, "Drive {}: {:.1} dB against bypassed at Level 5", drive * 10.0, centre);
        }
        assert!(level(1.0, 0.5).abs() < 4.0, "Drive 10: {:.1} dB", level(1.0, 0.5));
    }

    #[test]
    fn test_switching_does_not_click_or_reset() {
        let input = sine(220.0, 0.3, SAMPLE_RATE, LEN);
        let mut pedal = new_drive(SAMPLE_RATE, 0.7, 0.5, 0.5);
        let mut output = input.clone();
        for (index, block) in output.chunks_mut(128).enumerate() {
            pedal.set_on(!(250..500).contains(&index));
            pedal.process(block);
        }
        let (off, on) = (250 * 128, 500 * 128);
        let fade = (FADE_MS * 0.001 * SAMPLE_RATE) as usize + 1;
        assert_eq!(output[off + fade..on], input[off + fade..on]);
        assert!(rms(&output[on + fade..]) > 0.05);

        let own_step = largest_step(&output[..off]).max(largest_step(&input));
        assert!(largest_step(&output[off - 1..off + fade + 1]) < own_step * 1.1);
        assert!(largest_step(&output[on - 1..on + fade + 1]) < own_step * 1.1);
    }

    #[test]
    fn test_output_is_bounded_and_finite() {
        for sample_rate in [176_400.0, 192_000.0, 384_000.0, 768_000.0] {
            let input = sine(300.0, 4.0, sample_rate, 19_200);
            for (drive, tone, level) in [(0.0, 0.0, 0.0), (1.0, 1.0, 1.0), (1.0, 0.0, 1.0), (0.0, 1.0, 1.0)] {
                let mut output = input.clone();
                new_drive(sample_rate, drive, tone, level).process(&mut output);
                assert!(output.iter().all(|s| s.is_finite()));
                // The clean path is not limited: at most the input and the clipped path, times Level
                let limit = (4.0 + CLIP_LEVEL[1]) * db_to_gain(LEVEL_DB[1]);
                assert!(peak(&output) <= limit, "Peak {} at {} Hz", peak(&output), sample_rate);
            }
        }
    }
}
