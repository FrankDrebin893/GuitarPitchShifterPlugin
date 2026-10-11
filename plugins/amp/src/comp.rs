//! Compressor pedal: evens out the playing and lets notes ring longer. Two knobs, Sustain
//! and Level. Runs in front of the amp on one channel or behind the cabinet on two

use crate::dsp::filters::OnePoleHp;
use crate::dsp::{curve, db_to_gain, lerp, smoothing_coeff};

/// The Level dial turns the output down and up by this much around its middle, in dB
pub const LEVEL_DB: f32 = 12.0;

/// The most the pedal adds with Level at 5, in dB: what it gives silence and hiss with
/// Sustain at 10. A gate may sit in front of it, or not
pub const MAX_GAIN_DB: f32 = 20.0;

// Level of the side chain (peak, dBFS) above which the pedal turns down, with Sustain at 0
// and at 10
const THRESHOLD_DB: [f32; 2] = [-14.0, -36.0];

// Share of every dB above the threshold that is taken away, with Sustain at 0 and at 10:
// ratios of 1.4 and 6.7 to 1
const SLOPE: [f32; 2] = [0.3, 0.85];

// The bend from nothing to the full ratio is this wide, centred on the threshold
const KNEE_DB: f32 = 12.0;

// Added behind the curve across Sustain (0, 2.5, 5, 7.5, 10), so that Level at 5 is about as
// loud as the pedal switched off for a guitar at the reference level
const MAKEUP_DB: [f32; 5] = [1.1, 3.6, 7.5, 13.0, MAX_GAIN_DB];

// How fast the gain falls when a note is struck. The first milliseconds get through louder:
// the pick is still heard
const ATTACK_MS: f32 = 3.0;
// A note struck after a silence meets the gain at its highest. Only this much of it, in dB
// above where the gain is going, is left to the attack time. The rest is taken away at
// once, as the first swing of the string rises: gently for a moment, and no spike
const ATTACK_OVER_DB: f32 = 6.0;
// How fast the gain comes back as a note fades
const RELEASE_MS: f32 = 150.0;

// The level is the highest peak of the side chain in the window that is running and in the
// one before, so between 15 and 30 ms back. A held note has a peak in every window, also
// the lowest ones and those with one peak per period, so its level stands still and the gain
// does not follow its waveform. The release waits this long, which is not heard
const WINDOW_MS: f32 = 15.0;

// The side chain hears less of what is below this, so the weight of the low strings and
// the thump of a cabinet do not pull the gain down
const SIDE_HIGHPASS_HZ: f32 = 80.0;

// Levels are worked out in dB between these. The lower one is -120 dBFS
const LEVEL_FLOOR: f32 = 1e-6;
const LEVEL_CEILING: f32 = 1e6;

// Crossfade to and from the untouched signal when the pedal is switched
const FADE_MS: f32 = 10.0;
// How fast Sustain and Level follow their dials
const DIAL_SMOOTH_MS: f32 = 20.0;
const DIAL_SETTLED: f32 = 1e-5;

/// What the knobs say, read once per call. The pedal smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompSettings {
    pub on: bool,
    /// 0.0 to 1.0: how much the pedal evens out (threshold and ratio together)
    pub sustain: f32,
    /// 0.0 to 1.0: output level, 0.5 about as loud as the pedal switched off
    pub level: f32,
}

impl Default for CompSettings {
    fn default() -> Self {
        Self {
            on: false,
            sustain: 0.5,
            level: 0.5,
        }
    }
}

/// Feed-forward compressor with no lookahead: it adds no latency. It holds no buffers
pub struct Comp {
    // Only in front of the level measurement, left and right
    highpass: [OnePoleHp; 2],
    // Highest level of the side chain in the window that is running, and in the one before
    peak_now: f32,
    peak_before: f32,
    // Samples left of the window that is running
    window_left: u32,
    window_len: u32,

    // The level `wanted` was worked out for
    heard: f32,
    // The gain the level asks for, make-up included, and the gain on its way there
    wanted: f32,
    gain: f32,

    // Where the dials are, on their way to the settings. Level as a gain
    sustain: f32,
    level: f32,
    // The Level setting `level_target` was worked out for
    level_dial: f32,
    level_target: f32,
    // Share of the pedal in its output: 0.0 switched off, 1.0 on
    mix: f32,

    attack_over: f32,
    attack_coeff: f32,
    release_coeff: f32,
    dial_coeff: f32,
    fade_step: f32,

    // Off and faded out: the signal is left alone
    idle: bool,
}

impl Comp {
    pub fn new() -> Self {
        let mut comp = Self {
            highpass: [OnePoleHp::new(); 2],
            peak_now: 0.0,
            peak_before: 0.0,
            window_left: 1,
            window_len: 1,
            heard: -1.0,
            wanted: 1.0,
            gain: 1.0,
            sustain: 0.5,
            level: 1.0,
            level_dial: 0.5,
            level_target: 1.0,
            mix: 0.0,
            attack_over: db_to_gain(ATTACK_OVER_DB),
            attack_coeff: 1.0,
            release_coeff: 1.0,
            dial_coeff: 1.0,
            fade_step: 1.0,
            idle: true,
        };
        comp.set_sample_rate(44100.0);
        comp
    }

    /// The rate the pedal runs at. Never allocates
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        for highpass in &mut self.highpass {
            highpass.set(SIDE_HIGHPASS_HZ, sample_rate);
        }
        self.window_len = ((WINDOW_MS * 0.001 * sample_rate).round() as u32).max(1);
        self.attack_coeff = smoothing_coeff(ATTACK_MS, sample_rate);
        self.release_coeff = smoothing_coeff(RELEASE_MS, sample_rate);
        self.dial_coeff = smoothing_coeff(DIAL_SMOOTH_MS, sample_rate);
        self.fade_step = 1.0 / (FADE_MS * 0.001 * sample_rate);
        self.reset();
    }

    /// As after a long time switched off. A pedal that is on fades in again
    pub fn reset(&mut self) {
        self.mix = 0.0;
        self.idle = true;
    }

    /// True while the pedal is off and has faded out: the next block comes out as it goes in
    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// In front of the amp: one channel, at the amp's oversampled rate
    pub fn process_mono(&mut self, settings: &CompSettings, block: &mut [f32]) {
        if !self.awake(settings) {
            return;
        }
        let sustain = settings.sustain.clamp(0.0, 1.0);
        for sample in block.iter_mut() {
            let dry = *sample;
            let side = self.highpass[0].process(dry).abs();
            let gain = self.next_gain(side, sustain);
            let mix = self.next_mix(settings.on);
            *sample = mixed(dry, gain, mix);
            if self.idle {
                return;
            }
        }
    }

    /// Behind the cabinet: two channels at the host's rate, one gain for both
    pub fn process_stereo(&mut self, settings: &CompSettings, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        if !self.awake(settings) {
            return;
        }
        let sustain = settings.sustain.clamp(0.0, 1.0);
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let (dry_left, dry_right) = (*left, *right);
            // The louder side sets the gain of both, so nothing moves between them
            let side = self.highpass[0].process(dry_left).abs().max(self.highpass[1].process(dry_right).abs());
            let gain = self.next_gain(side, sustain);
            let mix = self.next_mix(settings.on);
            *left = mixed(dry_left, gain, mix);
            *right = mixed(dry_right, gain, mix);
            if self.idle {
                return;
            }
        }
    }

    /// False if the block is to be left alone. Reads the Level dial, and wakes a pedal that
    /// was idle and is switched on
    fn awake(&mut self, settings: &CompSettings) -> bool {
        if self.idle && !settings.on {
            return false;
        }
        let level = settings.level.clamp(0.0, 1.0);
        if level != self.level_dial {
            self.level_dial = level;
            self.level_target = db_to_gain(lerp(-LEVEL_DB, LEVEL_DB, level));
        }
        if self.idle {
            // Nothing is heard yet, so there is nothing to smooth. The gain starts where it
            // changes nothing and not where silence would have left it, at its highest
            for highpass in &mut self.highpass {
                highpass.reset();
            }
            self.peak_now = 0.0;
            self.peak_before = 0.0;
            self.window_left = self.window_len;
            self.heard = -1.0;
            self.gain = 1.0;
            self.sustain = settings.sustain.clamp(0.0, 1.0);
            self.level = self.level_target;
            self.mix = 0.0;
            self.idle = false;
        }
        true
    }

    /// The gain for one sample, Level included. `side` is the level of the side chain
    #[inline]
    fn next_gain(&mut self, side: f32, sustain: f32) -> f32 {
        if self.sustain != sustain {
            self.sustain = follow(self.sustain, sustain, self.dial_coeff);
            self.heard = -1.0;
        }
        if self.level != self.level_target {
            self.level = follow(self.level, self.level_target, self.dial_coeff);
        }

        self.peak_now = self.peak_now.max(side);
        let level = self.peak_now.max(self.peak_before);
        self.window_left -= 1;
        if self.window_left == 0 {
            self.peak_before = self.peak_now;
            self.peak_now = 0.0;
            self.window_left = self.window_len;
        }

        // The level stands still most of the time: between the peaks of a note, and in silence
        if level != self.heard {
            self.heard = level;
            self.wanted = wanted_gain(level, self.sustain);
            self.gain = self.gain.min(self.wanted * self.attack_over);
        }
        let coeff = if self.wanted < self.gain { self.attack_coeff } else { self.release_coeff };
        self.gain += coeff * (self.wanted - self.gain);
        self.gain * self.level
    }

    /// The share of the pedal in the next sample
    #[inline]
    fn next_mix(&mut self, on: bool) -> f32 {
        if on {
            if self.mix < 1.0 {
                self.mix = (self.mix + self.fade_step).min(1.0);
            }
        } else {
            self.mix = (self.mix - self.fade_step).max(0.0);
            self.idle = self.mix == 0.0;
        }
        self.mix
    }

    #[cfg(test)]
    fn gain_db(&self) -> f32 {
        20.0 * self.gain.log10()
    }
}

/// The gain a level of the side chain asks for at a dial position, make-up included
fn wanted_gain(level: f32, sustain: f32) -> f32 {
    let level_db = 20.0 * level.clamp(LEVEL_FLOOR, LEVEL_CEILING).log10();
    let slope = lerp(SLOPE[0], SLOPE[1], sustain);
    // From where the knee begins
    let over = level_db - lerp(THRESHOLD_DB[0], THRESHOLD_DB[1], sustain) + 0.5 * KNEE_DB;
    let reduction_db = if over <= 0.0 {
        0.0
    } else if over < KNEE_DB {
        slope * over * over / (2.0 * KNEE_DB)
    } else {
        slope * (over - 0.5 * KNEE_DB)
    };
    db_to_gain(curve(&MAKEUP_DB, sustain) - reduction_db)
}

fn mixed(dry: f32, gain: f32, mix: f32) -> f32 {
    if mix >= 1.0 {
        dry * gain
    } else if mix <= 0.0 {
        dry
    } else {
        dry + mix * (dry * gain - dry)
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
mod tests {
    use super::*;
    use crate::test_util::*;
    use std::time::Instant;

    const SAMPLE_RATE: f32 = 48000.0;
    const BLOCK: usize = 32;

    fn new_comp(sample_rate: f32) -> Comp {
        let mut comp = Comp::new();
        comp.set_sample_rate(sample_rate);
        comp
    }

    fn settings(sustain: f32, level: f32) -> CompSettings {
        CompSettings { on: true, sustain, level }
    }

    fn off() -> CompSettings {
        CompSettings {
            on: false,
            ..settings(1.0, 1.0)
        }
    }

    fn run_mono(comp: &mut Comp, settings: &CompSettings, input: &[f32], block: usize) -> Vec<f32> {
        let mut output = input.to_vec();
        for piece in output.chunks_mut(block) {
            comp.process_mono(settings, piece);
        }
        output
    }

    fn run_stereo(comp: &mut Comp, settings: &CompSettings, left: &[f32], right: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (left.to_vec(), right.to_vec());
        for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
            comp.process_stereo(settings, l, r);
        }
        (left, right)
    }

    /// A new pedal at 48 kHz over a whole recording, one channel
    fn run(settings: &CompSettings, input: &[f32]) -> Vec<f32> {
        run_mono(&mut new_comp(SAMPLE_RATE), settings, input, BLOCK)
    }

    fn noise(level: f32, len: usize) -> Vec<f32> {
        let mut noise = Noise::new(5);
        (0..len).map(|_| level * noise.next()).collect()
    }

    fn ms(time_ms: f32, sample_rate: f32) -> usize {
        (time_ms * 0.001 * sample_rate) as usize
    }

    fn crest_db(samples: &[f32]) -> f32 {
        to_db(peak(samples) / rms(samples))
    }

    /// The two made-up recordings at the reference level
    fn reference_recordings(sample_rate: f32) -> [(&'static str, Vec<f32>); 2] {
        [
            ("riffs", at_rms_db(&guitar_di(sample_rate), REFERENCE_DI_DB)),
            ("clean", at_rms_db(&clean_di(sample_rate), REFERENCE_DI_DB)),
        ]
    }

    /// Single notes picked hard, each left to ring for most of a second
    fn picked_notes(sample_rate: f32) -> Vec<f32> {
        let note_len = ms(800.0, sample_rate);
        let mut recording = Vec::new();
        for (index, freq) in [110.0, 196.0, 146.83, 246.94, 329.63, 164.81].into_iter().enumerate() {
            recording.extend(pluck(freq, Pluck::OPEN, index as u32 + 1, sample_rate, note_len));
            recording.extend(vec![0.0; ms(50.0, sample_rate)]);
        }
        at_rms_db(&recording, REFERENCE_DI_DB)
    }

    /// A tone that is soft, then loud, then soft again, 400 ms each: the same sound at
    /// every sample rate
    fn steps(sample_rate: f32) -> Vec<f32> {
        let piece = ms(400.0, sample_rate);
        let mut tone = sine(1000.0, 1.0, sample_rate, 3 * piece);
        for (index, sample) in tone.iter_mut().enumerate() {
            *sample *= if index / piece == 1 { 0.3 } else { 0.01 };
        }
        tone
    }

    /// Gain in dB over every 10 ms of a recording
    fn gain_trace_db(input: &[f32], output: &[f32], sample_rate: f32) -> Vec<f32> {
        let piece = ms(10.0, sample_rate);
        input.chunks_exact(piece).zip(output.chunks_exact(piece)).map(|(dry, wet)| to_db(rms(wet) / rms(dry))).collect()
    }

    #[test]
    fn test_settings_default_to_off_with_the_dials_in_the_middle() {
        assert_eq!(CompSettings::default(), CompSettings { on: false, sustain: 0.5, level: 0.5 });
        assert_eq!(*MAKEUP_DB.last().unwrap(), MAX_GAIN_DB);
        assert!(MAKEUP_DB.iter().all(|&makeup| makeup <= MAX_GAIN_DB));
    }

    #[test]
    fn test_off_leaves_the_signal_alone() {
        let (left_in, right_in) = (noise(0.5, 4800), sine(330.0, 0.4, SAMPLE_RATE, 4800));
        let mut comp = new_comp(SAMPLE_RATE);
        let (left, right) = run_stereo(&mut comp, &off(), &left_in, &right_in, BLOCK);
        assert!(left == left_in && right == right_in);
        assert!(run_mono(&mut comp, &off(), &left_in, BLOCK) == left_in);
        assert!(comp.is_idle());
    }

    #[test]
    fn test_off_is_bit_transparent_once_it_has_faded() {
        let input = at_rms_db(&guitar_di(SAMPLE_RATE)[..ms(1500.0, SAMPLE_RATE)], REFERENCE_DI_DB);
        let split = ms(700.0, SAMPLE_RATE) / BLOCK * BLOCK;
        let fade = ms(FADE_MS, SAMPLE_RATE) + 1;

        let mut comp = new_comp(SAMPLE_RATE);
        let before = run_mono(&mut comp, &settings(0.8, 0.7), &input[..split], BLOCK);
        assert!(before != input[..split]);
        assert!(!comp.is_idle());
        let after = run_mono(&mut comp, &off(), &input[split..], BLOCK);
        assert!(after[..fade - 2] != input[split..split + fade - 2]);
        assert!(after[fade..] == input[split + fade..]);
        assert!(comp.is_idle());

        let mut comp = new_comp(SAMPLE_RATE);
        run_stereo(&mut comp, &settings(0.8, 0.7), &input[..split], &input[..split], BLOCK);
        let (left, right) = run_stereo(&mut comp, &off(), &input[split..], &input[split..], BLOCK);
        assert!(left[fade..] == input[split + fade..] && right[fade..] == input[split + fade..]);
        assert!(comp.is_idle());
    }

    #[test]
    fn test_switching_does_not_click() {
        let input = sine(220.0, 0.3, SAMPLE_RATE, ms(1500.0, SAMPLE_RATE));
        for sustain in [0.0, 0.5, 1.0] {
            let on = settings(sustain, 0.5);
            let mut comp = new_comp(SAMPLE_RATE);
            let mut output = Vec::new();
            for (index, block) in input.chunks(BLOCK).enumerate() {
                // Off and on again a few times while the sine goes on
                let now = if (index * BLOCK / ms(250.0, SAMPLE_RATE)) % 2 == 0 { on } else { CompSettings { on: false, ..on } };
                output.extend(run_mono(&mut comp, &now, block, BLOCK));
            }
            // The steepest the sine is by itself, and through the pedal once it has settled
            let settled = run(&on, &input);
            let own_step = largest_step(&input).max(largest_step(&settled[settled.len() / 2..]));
            assert!(largest_step(&output) < own_step * 1.1, "Sustain {}: step {} against {}", sustain * 10.0, largest_step(&output), own_step);
            assert!(rms(&output) > 0.1);
        }
    }

    #[test]
    fn test_dial_jumps_do_not_click() {
        let input = sine(220.0, 0.3, SAMPLE_RATE, ms(2000.0, SAMPLE_RATE));
        let half = input.len() / 2;
        let jumps = [
            (settings(0.0, 0.5), settings(1.0, 0.5)),
            (settings(1.0, 0.5), settings(0.0, 0.5)),
            (settings(0.5, 0.0), settings(0.5, 1.0)),
            (settings(0.5, 1.0), settings(0.5, 0.0)),
            (settings(0.0, 0.0), settings(1.0, 1.0)),
        ];
        for (from, to) in jumps {
            let mut comp = new_comp(SAMPLE_RATE);
            let mut output = run_mono(&mut comp, &from, &input[..half], BLOCK);
            output.extend(run_mono(&mut comp, &to, &input[half..], BLOCK));
            // A click would be a step far above the steepest the sine gets at either setting
            let steepest = |setting: &CompSettings| largest_step(&run(setting, &input)[half..]);
            let own_step = steepest(&from).max(steepest(&to));
            let step = largest_step(&output[half - 1..]);
            assert!(step < own_step * 1.15, "{:?} to {:?}: step {} against {}", from, to, step, own_step);
            // And it arrives where the new setting is by itself
            let end = input.len() - ms(100.0, SAMPLE_RATE);
            let apart = to_db(rms(&output[end..]) / rms(&run(&to, &input)[end..]));
            assert!(apart.abs() < 0.1, "{:?} to {:?}: {:.2} dB from the new setting", from, to, apart);
        }
    }

    #[test]
    fn test_output_does_not_depend_on_block_size() {
        // Playing, switched off for long enough to fall idle, on again at other settings
        let input = at_rms_db(&guitar_di(SAMPLE_RATE)[..ms(2400.0, SAMPLE_RATE)], REFERENCE_DI_DB);
        let third = input.len() / 3;
        let parts = [
            (settings(0.6, 0.5), &input[..third]),
            (off(), &input[third..2 * third]),
            (settings(0.9, 0.3), &input[2 * third..]),
        ];
        let mono = |block: usize| {
            let mut comp = new_comp(SAMPLE_RATE);
            parts.iter().flat_map(|(setting, part)| run_mono(&mut comp, setting, part, block)).collect::<Vec<f32>>()
        };
        let stereo = |block: usize| {
            let mut comp = new_comp(SAMPLE_RATE);
            let mut both = (Vec::new(), Vec::new());
            for (setting, part) in &parts {
                // The right channel is the left one late and quieter
                let right: Vec<f32> = (0..part.len()).map(|index| 0.7 * part[index.saturating_sub(40)]).collect();
                let (left, right) = run_stereo(&mut comp, setting, part, &right, block);
                both.0.extend(left);
                both.1.extend(right);
            }
            both
        };
        let (whole_mono, whole_stereo) = (mono(input.len()), stereo(input.len()));
        assert!(whole_mono.iter().all(|sample| sample.is_finite()));
        assert!(whole_mono != input);
        for block in [1, 7, 32, 64, 128] {
            assert!(mono(block) == whole_mono, "one channel in blocks of {}", block);
            assert!(stereo(block) == whole_stereo, "two channels in blocks of {}", block);
        }
    }

    #[test]
    fn test_times_are_the_same_at_every_sample_rate() {
        // The gain over every 10 ms of a tone that steps up and down
        let trace = |sample_rate: f32, block: usize| {
            let input = steps(sample_rate);
            let output = run_mono(&mut new_comp(sample_rate), &settings(0.7, 0.5), &input, block);
            gain_trace_db(&input, &output, sample_rate)
        };
        let reference = trace(48000.0, 32);
        // It moves: turned down in the loud part, and back up after it
        assert!(reference[35] - reference[75] > 10.0, "{:.1} and {:.1} dB", reference[35], reference[75]);
        assert!(reference[115] - reference[75] > 10.0);
        for (sample_rate, block) in [(44100.0, 32), (96000.0, 32), (192000.0, 128), (384000.0, 128)] {
            let other = trace(sample_rate, block);
            let apart = reference.iter().zip(&other).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(apart < 0.3, "{} Hz: {:.2} dB from 48 kHz", sample_rate, apart);
        }

        // And on two channels
        let input = steps(192000.0);
        let (left, _) = run_stereo(&mut new_comp(192000.0), &settings(0.7, 0.5), &input, &input, 32);
        let other = gain_trace_db(&input, &left, 192000.0);
        let apart = reference.iter().zip(&other).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(apart < 0.3, "Two channels at 192 kHz: {:.2} dB from 48 kHz", apart);
    }

    #[test]
    fn test_level_five_is_as_loud_as_off_across_sustain() {
        for (name, input) in reference_recordings(SAMPLE_RATE) {
            let dry = loudness_lufs(&[&input], SAMPLE_RATE);
            for step in 0..=10 {
                let sustain = step as f32 / 10.0;
                let wet = loudness_lufs(&[&run(&settings(sustain, 0.5), &input)], SAMPLE_RATE);
                assert!((wet - dry).abs() < 1.5, "{}, Sustain {}: {:.1} LU against off", name, step, wet - dry);
            }
        }
    }

    #[test]
    fn test_level_dial_spans_its_range_around_the_middle() {
        let input = at_rms_db(&guitar_di(SAMPLE_RATE)[..ms(3000.0, SAMPLE_RATE)], REFERENCE_DI_DB);
        let level = |level: f32| to_db(rms(&run(&settings(0.5, level), &input)));
        // All but the 10 ms fade from the untouched signal at the start
        assert!((level(1.0) - level(0.5) - LEVEL_DB).abs() < 0.1, "{:.2} dB up", level(1.0) - level(0.5));
        assert!((level(0.5) - level(0.0) - LEVEL_DB).abs() < 0.1, "{:.2} dB down", level(0.5) - level(0.0));
    }

    /// How far apart in level the pedal leaves the same playing at -36 and at -18 dBFS RMS
    fn soft_to_hard_db(sustain: f32) -> f32 {
        let di = guitar_di(SAMPLE_RATE);
        let out = |level_db: f32| to_db(rms(&run(&settings(sustain, 0.5), &at_rms_db(&di, level_db))));
        out(-18.0) - out(-36.0)
    }

    #[test]
    fn test_sustain_evens_out_soft_and_hard_playing() {
        // 18 dB apart going in
        let apart: Vec<f32> = [0.0, 0.25, 0.5, 0.75, 1.0].into_iter().map(soft_to_hard_db).collect();
        assert!(apart.windows(2).all(|pair| pair[1] < pair[0] - 1.0), "{:?}", apart);
        assert!(apart[0] > 15.0, "Sustain 0 does more than a little: {:.1} dB", apart[0]);
        assert!((8.0..13.0).contains(&apart[2]), "Sustain 5: {:.1} dB", apart[2]);
        assert!(apart[4] < 5.0, "Sustain 10: {:.1} dB", apart[4]);
    }

    /// Level of a ringing string a second and a half after it was picked, against 100 ms
    /// after, in dB
    fn ring_db(sustain: Option<f32>) -> f32 {
        let mut note = pluck(196.0, Pluck::OPEN, 3, SAMPLE_RATE, ms(2000.0, SAMPLE_RATE));
        set_rms_db(&mut note, REFERENCE_DI_DB);
        let output = match sustain {
            Some(sustain) => run(&settings(sustain, 0.5), &note),
            None => note,
        };
        let at = |time_ms: f32| rms(&output[ms(time_ms, SAMPLE_RATE)..ms(time_ms + 100.0, SAMPLE_RATE)]);
        to_db(at(1500.0) / at(100.0))
    }

    #[test]
    fn test_notes_ring_longer_with_more_sustain() {
        let dry = ring_db(None);
        let (low, centre, full) = (ring_db(Some(0.0)), ring_db(Some(0.5)), ring_db(Some(1.0)));
        assert!(dry < -15.0, "The string by itself: {:.1} dB", dry);
        assert!(low - dry < 4.0, "Sustain 0: {:.1} dB against {:.1}", low, dry);
        assert!(centre > dry + 6.0, "Sustain 5: {:.1} dB against {:.1}", centre, dry);
        assert!(full > centre + 4.0 && full > -6.0, "Sustain 10: {:.1} dB", full);
    }

    #[test]
    fn test_pick_attack_is_kept() {
        // At Sustain 5 the notes come out with as much peak over their average as they
        // went in with: the body is turned down, the first milliseconds are not
        let input = picked_notes(SAMPLE_RATE);
        let output = run(&settings(0.5, 0.5), &input);
        let (crest_in, crest_out) = (crest_db(&input), crest_db(&output));
        assert!(crest_out > crest_in - 1.0, "Crest {:.1} dB in, {:.1} dB out", crest_in, crest_out);
        assert!(crest_out < crest_in + 6.0, "Crest {:.1} dB in, {:.1} dB out", crest_in, crest_out);

        // The first 5 ms of a note against the 100 ms from 50 ms on
        let note = ms(850.0, SAMPLE_RATE);
        let pick = |signal: &[f32]| -> f32 {
            let over: Vec<f32> = signal
                .chunks_exact(note)
                .map(|note| to_db(peak(&note[..ms(5.0, SAMPLE_RATE)]) / rms(&note[ms(50.0, SAMPLE_RATE)..ms(150.0, SAMPLE_RATE)])))
                .collect();
            over.iter().sum::<f32>() / over.len() as f32
        };
        // The second note on: the first one meets a pedal that is still fading in
        let (pick_in, pick_out) = (pick(&input[note..]), pick(&output[note..]));
        assert!(pick_out > pick_in, "Pick over body: {:.1} dB in, {:.1} dB out", pick_in, pick_out);
        assert!(pick_out < pick_in + 9.0, "Pick over body: {:.1} dB in, {:.1} dB out", pick_in, pick_out);
    }

    #[test]
    fn test_gain_in_silence_is_bounded() {
        // Hiss at -100 dBFS, and nothing at all
        let hiss = noise(1e-5, ms(2000.0, SAMPLE_RATE));
        let tail = ms(1000.0, SAMPLE_RATE);
        for step in 0..=10 {
            let sustain = step as f32 / 10.0;
            let mut comp = new_comp(SAMPLE_RATE);
            let output = run_mono(&mut comp, &settings(sustain, 0.5), &hiss, BLOCK);
            let gain = to_db(rms(&output[tail..]) / rms(&hiss[tail..]));
            assert!(gain <= MAX_GAIN_DB + 0.01, "Sustain {}: {:.2} dB added to hiss", step, gain);
            assert!(gain >= -0.01);

            let silence = run_mono(&mut comp, &settings(sustain, 0.5), &vec![0.0; tail], BLOCK);
            assert!(silence.iter().all(|&sample| sample == 0.0));
            assert!(comp.gain_db() <= MAX_GAIN_DB + 0.01);
        }
        // With Level all the way up as well
        let output = run(&settings(1.0, 1.0), &hiss);
        let gain = to_db(rms(&output[tail..]) / rms(&hiss[tail..]));
        assert!((gain - MAX_GAIN_DB - LEVEL_DB).abs() < 0.1, "{:.2} dB", gain);
    }

    #[test]
    fn test_silence_is_not_slower_than_playing() {
        // Denormal numbers would make the silence after a note many times slower
        let mut comp = new_comp(SAMPLE_RATE);
        let setting = settings(1.0, 0.5);
        let time_us = |comp: &mut Comp, input: &[f32]| {
            let mut times: Vec<f64> = input
                .chunks(BLOCK)
                .map(|block| {
                    let mut block = block.to_vec();
                    let start = Instant::now();
                    comp.process_mono(&setting, &mut block);
                    start.elapsed().as_secs_f64() * 1e6
                })
                .collect();
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            times[times.len() / 2]
        };
        let playing = time_us(&mut comp, &noise(0.5, ms(500.0, SAMPLE_RATE)));
        let mut slowest: f64 = 0.0;
        for _ in 0..6 {
            slowest = slowest.max(time_us(&mut comp, &vec![0.0; ms(500.0, SAMPLE_RATE)]));
        }
        assert!(slowest < playing * SLOWED_DOWN, "{:.2} us per block playing, {:.2} us in silence", playing, slowest);
    }

    /// Distortion of a held note through the pedal once it has settled, in dB
    fn held_note_thd_db(sustain: f32, freq_hz: f32, level: f32, sample_rate: f32) -> f32 {
        let input = sine(freq_hz, level, sample_rate, ms(1500.0, sample_rate));
        let output = run_mono(&mut new_comp(sample_rate), &settings(sustain, 0.5), &input, BLOCK);
        thd_db(&output[ms(500.0, sample_rate)..], sample_rate, freq_hz)
    }

    #[test]
    fn test_held_low_note_is_not_rippled() {
        // The gain must not follow the waveform of the lowest string. Down to the lowest
        // note of a drop tuning, soft and hard
        for sustain in [0.5, 1.0] {
            for freq_hz in [65.41, 82.41, 220.0] {
                for level in [HELD_NOTE, 0.5] {
                    let thd = held_note_thd_db(sustain, freq_hz, level, SAMPLE_RATE);
                    assert!(thd < -70.0, "Sustain {}, {} Hz at {}: {:.1} dB", sustain * 10.0, freq_hz, level, thd);
                }
            }
        }
        assert!(held_note_thd_db(0.5, 82.41, HELD_NOTE, 192000.0) < -70.0);

        // It is turned down all the same
        let input = sine(82.41, 0.5, SAMPLE_RATE, ms(1000.0, SAMPLE_RATE));
        let output = run(&settings(1.0, 0.5), &input);
        let half = input.len() / 2;
        assert!(to_db(rms(&output[half..]) / rms(&input[half..])) < MAX_GAIN_DB - 20.0);
    }

    #[test]
    fn test_low_strings_are_turned_down_less_than_high_ones() {
        let gain = |freq_hz: f32| {
            let input = sine(freq_hz, 0.3, SAMPLE_RATE, ms(1000.0, SAMPLE_RATE));
            let output = run(&settings(0.7, 0.5), &input);
            to_db(rms(&output[input.len() / 2..]) / rms(&input[input.len() / 2..]))
        };
        let (low, high) = (gain(82.41), gain(660.0));
        assert!(low > high + 1.0 && low < high + 4.0, "{:.1} dB at 82 Hz, {:.1} dB at 660 Hz", low, high);
    }

    #[test]
    fn test_both_channels_get_the_same_gain() {
        let left_in = at_rms_db(&guitar_di(SAMPLE_RATE)[..ms(3000.0, SAMPLE_RATE)], REFERENCE_DI_DB);
        // Something else on the right, quieter, then louder than the left
        let right_in: Vec<f32> = sine(330.0, 1.0, SAMPLE_RATE, left_in.len())
            .iter()
            .enumerate()
            .map(|(index, sample)| sample * if index < left_in.len() / 2 { 0.01 } else { 0.4 })
            .collect();
        let setting = settings(0.8, 0.5);
        let (left, right) = run_stereo(&mut new_comp(SAMPLE_RATE), &setting, &left_in, &right_in, BLOCK);

        // Past the fade, each side is its input times the same gain
        let from = ms(FADE_MS, SAMPLE_RATE) + 1;
        let mut compared = 0;
        for index in from..left.len() {
            if left_in[index].abs() > 1e-3 && right_in[index].abs() > 1e-3 {
                let (gain_left, gain_right) = (left[index] / left_in[index], right[index] / right_in[index]);
                assert!((gain_left / gain_right - 1.0).abs() < 1e-5, "{} against {} at sample {}", gain_left, gain_right, index);
                compared += 1;
            }
        }
        assert!(compared > left.len() / 4);

        // Whichever side is louder sets the gain: the sides swapped come out swapped
        let (swapped_left, swapped_right) = run_stereo(&mut new_comp(SAMPLE_RATE), &setting, &right_in, &left_in, BLOCK);
        assert!(swapped_left == right && swapped_right == left);
        // A loud right side turns the left one down
        let alone = run(&setting, &left_in);
        let half = left.len() / 2 + ms(100.0, SAMPLE_RATE);
        assert!(rms(&left[half..]) < 0.8 * rms(&alone[half..]));

        // The same on both sides is what one channel gives
        let (left, right) = run_stereo(&mut new_comp(SAMPLE_RATE), &setting, &left_in, &left_in, BLOCK);
        assert!(left == alone && right == alone);
    }

    #[test]
    fn test_output_is_bounded_and_finite_with_extreme_input() {
        let most = db_to_gain(MAX_GAIN_DB + LEVEL_DB);
        for sample_rate in [44100.0, 48000.0, 192000.0, 384000.0] {
            let len = ms(600.0, sample_rate);
            let period = ms(4.0, sample_rate);
            // Far over full scale, silence, a square, and next to nothing
            let mut input = noise(100.0, len);
            input.extend(vec![0.0; len]);
            input.extend((0..len).map(|index| if (index / period) % 2 == 0 { 1e4 } else { -1e4 }));
            input.extend(noise(1e-30, len));
            input.extend(noise(1.0, len));
            for (sustain, level) in [(0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0), (0.5, 0.5)] {
                let setting = settings(sustain, level);
                let mono = run_mono(&mut new_comp(sample_rate), &setting, &input, 128);
                let (left, right) = run_stereo(&mut new_comp(sample_rate), &setting, &input, &mono, 32);
                for output in [&mono, &left, &right] {
                    assert!(output.iter().all(|sample| sample.is_finite()));
                }
                for (sample, dry) in mono.iter().zip(&input) {
                    assert!(sample.abs() <= dry.abs() * most * 1.001, "{} from {} at {} Hz", sample, dry, sample_rate);
                }
            }
        }
    }

    #[test]
    fn test_reset_starts_over() {
        let input = at_rms_db(&guitar_di(SAMPLE_RATE)[..ms(800.0, SAMPLE_RATE)], REFERENCE_DI_DB);
        let setting = settings(0.7, 0.6);
        let mut comp = new_comp(SAMPLE_RATE);
        let first = run_mono(&mut comp, &setting, &input, BLOCK);
        assert!(!comp.is_idle());
        comp.reset();
        assert!(comp.is_idle());
        assert!(run_mono(&mut comp, &setting, &input, BLOCK) == first);
        // Another rate and back, as when the pedal is moved to the other side of the amp
        comp.set_sample_rate(192000.0);
        comp.set_sample_rate(SAMPLE_RATE);
        assert!(run_mono(&mut comp, &setting, &input, BLOCK) == first);
    }

    #[test]
    fn test_it_holds_no_buffers() {
        // Nothing to free, so nothing was allocated: the rate can be set on the audio thread
        assert!(!std::mem::needs_drop::<Comp>());
    }

    /// Milliseconds the gain takes to cover a share of its way after the tone of `steps`
    /// gets loud (the attack) and after it gets soft again (the release)
    fn step_times_ms(sustain: f32, share: f32, sample_rate: f32) -> (f32, f32) {
        let input = steps(sample_rate);
        let piece = input.len() / 3;
        let mut comp = new_comp(sample_rate);
        let setting = settings(sustain, 0.5);
        let gains: Vec<f32> = input
            .iter()
            .map(|&sample| {
                comp.process_mono(&setting, &mut [sample]);
                comp.gain_db()
            })
            .collect();
        let (soft, loud) = (gains[piece - 1], gains[2 * piece - 1]);
        let passed = soft + share * (loud - soft);
        let attack = gains[piece..].iter().position(|&gain| gain <= passed).unwrap();
        let release = gains[2 * piece..].iter().position(|&gain| gain >= loud + share * (soft - loud)).unwrap_or(piece);
        (attack as f32 * 1000.0 / sample_rate, release as f32 * 1000.0 / sample_rate)
    }

    #[test]
    fn test_attack_takes_milliseconds_and_release_a_tenth_of_a_second() {
        for sustain in [0.25, 0.5, 1.0] {
            // With Sustain up, most of the way down is taken at once and the rest in time
            let (attack, release) = step_times_ms(sustain, 0.63, SAMPLE_RATE);
            let (attack_end, _) = step_times_ms(sustain, 0.9, SAMPLE_RATE);
            assert!(attack < 6.0, "Sustain {}: attack {:.1} ms", sustain * 10.0, attack);
            assert!((2.0..12.0).contains(&attack_end), "Sustain {}: attack ends after {:.1} ms", sustain * 10.0, attack_end);
            assert!((60.0..250.0).contains(&release), "Sustain {}: release {:.1} ms", sustain * 10.0, release);
        }
    }

    #[test]
    fn test_a_note_after_silence_does_not_spike() {
        // The gain is at its highest when the note comes. It may come out a little louder
        // than it settles at, for the pick, and no more than that. (The high-pass of the
        // side chain hears the very first swing a little low, which is the 1.5 dB.) With no
        // bound the riffs had 36 dB of peak over their average at Sustain 10, and 19 with it
        let mut input = vec![0.0; ms(500.0, SAMPLE_RATE)];
        input.extend(sine(330.0, 0.3, SAMPLE_RATE, ms(500.0, SAMPLE_RATE)));
        for sustain in [0.5, 0.75, 1.0] {
            let output = run(&settings(sustain, 0.5), &input);
            let settled = peak(&output[ms(900.0, SAMPLE_RATE)..]);
            let first = to_db(peak(&output[..ms(520.0, SAMPLE_RATE)]) / settled);
            assert!(first > 2.0, "Sustain {}: no attack left, {:.1} dB", sustain * 10.0, first);
            assert!(first < ATTACK_OVER_DB + 1.5, "Sustain {}: {:.1} dB over where it settles", sustain * 10.0, first);
        }
    }

    /// Fastest of seven runs over a recording, in ns per sample
    fn cost_ns(sample_rate: f32, block: usize, stereo: bool, setting: &CompSettings, input: &[f32]) -> f64 {
        let mut comp = new_comp(sample_rate);
        (0..7)
            .map(|_| {
                let (mut left, mut right) = (input.to_vec(), input.to_vec());
                let start = Instant::now();
                for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
                    if stereo {
                        comp.process_stereo(setting, l, r);
                    } else {
                        comp.process_mono(setting, l);
                    }
                }
                std::hint::black_box((&left, &right));
                start.elapsed().as_secs_f64() * 1e9 / input.len() as f64
            })
            .fold(f64::MAX, f64::min)
    }

    /// Prints what the pedal does per Sustain setting. Run before and after a change:
    ///   cargo test -p amp --release comp_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn comp_report() {
        crate::test_util::pin_thread();
        println!("Compressor pedal at 48 kHz, Level 5. Reduction: how far the pedal turns the riffs down at");
        println!("-36 (soft) and -18 dBFS RMS (hard), make-up left out. Apart: how far the two are apart coming");
        println!("out (18 dB going in). Loudness: against the pedal off, at the reference level of {} dBFS RMS.", REFERENCE_DI_DB);
        println!("Ring: level of a string 1.5 s after it was picked against 100 ms after ({:.1} dB by itself).", ring_db(None));
        println!();
        println!("Sustain  make-up  reduction dB   apart   loudness LU    ring");
        println!("            dB    soft   hard      dB   riffs  clean      dB");
        let di = guitar_di(SAMPLE_RATE);
        for sustain in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let makeup = curve(&MAKEUP_DB, sustain);
            let setting = settings(sustain, 0.5);
            let reduction = |level_db: f32| {
                let input = at_rms_db(&di, level_db);
                makeup - to_db(rms(&run(&setting, &input)) / rms(&input))
            };
            let loudness: Vec<f32> = reference_recordings(SAMPLE_RATE)
                .iter()
                .map(|(_, input)| loudness_lufs(&[&run(&setting, input)], SAMPLE_RATE) - loudness_lufs(&[input], SAMPLE_RATE))
                .collect();
            println!(
                "{:>6.1}{:>9.1}{:>8.1}{:>7.1}{:>8.1}{:>8.1}{:>7.1}{:>8.1}",
                sustain * 10.0,
                makeup,
                reduction(-36.0),
                reduction(-18.0),
                soft_to_hard_db(sustain),
                loudness[0],
                loudness[1],
                ring_db(Some(sustain)),
            );
        }

        println!();
        println!("Crest: peak over RMS of the recordings at the reference level, and of single picked notes.");
        println!("Held: distortion of a held note at -24 dBFS RMS. Attack and release: time for 63 % and 90 %");
        println!("of the way in dB after a 1 kHz tone steps from -40 to -10.5 dBFS and back");
        println!();
        println!("Sustain     crest dB (in: riffs {:.1}, clean {:.1}, picked {:.1})     held dB       attack ms     release ms",
            crest_db(&reference_recordings(SAMPLE_RATE)[0].1), crest_db(&reference_recordings(SAMPLE_RATE)[1].1), crest_db(&picked_notes(SAMPLE_RATE)));
        println!("            riffs   clean  picked                              82 Hz  220 Hz     63 %   90 %    63 %   90 %");
        for sustain in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let setting = settings(sustain, 0.5);
            let crests: Vec<f32> = reference_recordings(SAMPLE_RATE).iter().map(|(_, input)| crest_db(&run(&setting, input))).collect();
            let picked = crest_db(&run(&setting, &picked_notes(SAMPLE_RATE)));
            let (attack, release) = step_times_ms(sustain, 0.63, SAMPLE_RATE);
            let (attack_90, release_90) = step_times_ms(sustain, 0.9, SAMPLE_RATE);
            println!(
                "{:>6.1}{:>12.1}{:>8.1}{:>8.1}{:>35.1}{:>8.1}{:>9.1}{:>7.1}{:>8.0}{:>7.0}",
                sustain * 10.0,
                crests[0],
                crests[1],
                picked,
                held_note_thd_db(sustain, 82.41, HELD_NOTE, SAMPLE_RATE),
                held_note_thd_db(sustain, 220.0, HELD_NOTE, SAMPLE_RATE),
                attack,
                attack_90,
                release,
                release_90,
            );
        }

        println!();
        println!("Cost at Sustain 5 in ns per sample, the fastest of seven runs over the riffs (and over noise,");
        println!("where the level changes most often), and as a share of real time");
        let on = settings(0.5, 0.5);
        let cases = [
            ("two channels, 48 kHz, blocks of 32", 48000.0, 32, true),
            ("one channel, 192 kHz, blocks of 128", 192000.0, 128, false),
            ("one channel, 384 kHz, blocks of 128", 384000.0, 128, false),
        ];
        for (name, sample_rate, block, stereo) in cases {
            let riffs = at_rms_db(&guitar_di(sample_rate), REFERENCE_DI_DB);
            let hiss = noise(0.1, riffs.len());
            let playing = cost_ns(sample_rate, block, stereo, &on, &riffs);
            let worst = cost_ns(sample_rate, block, stereo, &on, &hiss);
            let idle = cost_ns(sample_rate, block, stereo, &CompSettings::default(), &riffs);
            let share = |ns: f64| ns * 1e-9 * sample_rate as f64 * 100.0;
            println!(
                "{:<38}{:>6.2} ns{:>6.3} %   noise {:>6.2} ns{:>6.3} %   off {:>5.2} ns",
                name,
                playing,
                share(playing),
                worst,
                share(worst),
                idle
            );
        }
    }

    /// Writes WAV files to target/renders for listening: clean playing at the reference
    /// level, dry and through the pedal at Sustain 3, 5 and 8 with Level at 5
    ///   cargo test -p amp --release render_comp -- --ignored
    #[test]
    #[ignore]
    fn render_comp() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();
        let input = at_rms_db(&clean_di(SAMPLE_RATE), REFERENCE_DI_DB);
        write_wav(&dir.join("amp_comp_dry.wav"), &[&input], SAMPLE_RATE as u32);
        for sustain in [3, 5, 8] {
            let output = run(&settings(sustain as f32 / 10.0, 0.5), &input);
            write_wav(&dir.join(format!("amp_comp_sustain_{}.wav", sustain)), &[&output], SAMPLE_RATE as u32);
        }
        println!("Wrote renders to {}", dir.display());
    }
}
