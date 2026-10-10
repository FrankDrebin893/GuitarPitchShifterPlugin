use crate::amp::model::Amp;
use crate::amp::poweramp::PowerAmp;
use crate::amp::preamp::Preamp;
use crate::amp::tonestack::{ToneCurve, ToneStack};
use crate::cab::{design_ir, Cabinet};
use crate::dsp::filters::DcBlocker;
use crate::dsp::oversample::{Oversampler, FACTOR};
use crate::dsp::shaper::output_clip;
use crate::dsp::{smoothing_coeff, Ramp};

// The chain works in pieces of at most this many samples, and reads the dials once per piece
const CHUNK: usize = 32;
const OVERSAMPLED_CHUNK: usize = CHUNK * FACTOR;

// How fast the chain follows a dial
const DIAL_SMOOTH_MS: f32 = 20.0;

// A dial this close to where it is going is taken as there, so the filters stop being redesigned
const DIAL_SETTLED: f32 = 1e-5;

// Crossfade between the amp and the untouched input when Bypass is switched
const BYPASS_FADE_MS: f32 = 10.0;

/// What the knobs say, read once per block. The chain smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmpSettings {
    pub bypass: bool,
    /// The amp dials, 0.0 to 1.0
    pub gain: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub presence: f32,
    pub master: f32,
    /// Linear gain
    pub out_level: f32,
}

impl Default for AmpSettings {
    fn default() -> Self {
        Self {
            bypass: false,
            gain: 0.5,
            bass: 0.5,
            mid: 0.5,
            treble: 0.5,
            presence: 0.5,
            master: 0.5,
            out_level: 1.0,
        }
    }
}

/// The dial values the chain is at, on their way to the settings
#[derive(Clone, Copy, PartialEq)]
struct Dials {
    gain: f32,
    bass: f32,
    mid: f32,
    treble: f32,
    presence: f32,
    master: f32,
    out_level: f32,
}

impl Dials {
    fn from_settings(settings: &AmpSettings) -> Self {
        Self {
            gain: settings.gain.clamp(0.0, 1.0),
            bass: settings.bass.clamp(0.0, 1.0),
            mid: settings.mid.clamp(0.0, 1.0),
            treble: settings.treble.clamp(0.0, 1.0),
            presence: settings.presence.clamp(0.0, 1.0),
            master: settings.master.clamp(0.0, 1.0),
            out_level: settings.out_level.max(0.0),
        }
    }

    fn approach(&mut self, target: &Dials, coeff: f32) {
        let step = |value: &mut f32, target: f32| {
            *value += coeff * (target - *value);
            if (target - *value).abs() < DIAL_SETTLED {
                *value = target;
            }
        };
        step(&mut self.gain, target.gain);
        step(&mut self.bass, target.bass);
        step(&mut self.mid, target.mid);
        step(&mut self.treble, target.treble);
        step(&mut self.presence, target.presence);
        step(&mut self.master, target.master);
        step(&mut self.out_level, target.out_level);
    }
}

/// The whole signal chain. Mono through the amp and cabinet
pub struct AmpChain {
    sample_rate: f32,
    amp: Amp,

    oversampler: Oversampler,
    preamp: Preamp,
    tone: ToneStack,
    power: PowerAmp,
    cabinet: Cabinet,
    // One impulse response per amp, designed when the sample rate is set
    cab_irs: Vec<Vec<f32>>,
    dc: DcBlocker,
    out_level: Ramp,

    dials: Dials,
    dial_coeff: f32,
    // The dial positions the tone filters were last designed for
    tone_applied: [f32; 3],
    presence_applied: f32,
    // False until the first block after a reset: the dials then start at the settings
    primed: bool,
    // Samples left until the dials are read again. Counted across blocks, so the output
    // does not depend on how the host cuts the stream into blocks
    until_tick: usize,

    // Share of the amp in the output: 1.0 playing, 0.0 bypassed
    wet: f32,
    wet_step: f32,
}

impl AmpChain {
    pub fn new() -> Self {
        let mut chain = Self {
            sample_rate: 44100.0,
            amp: Amp::Brol,
            oversampler: Oversampler::new(),
            preamp: Preamp::new(),
            tone: ToneStack::new(),
            power: PowerAmp::new(),
            cabinet: Cabinet::new(),
            cab_irs: Vec::new(),
            dc: DcBlocker::new(),
            out_level: Ramp::new(1.0),
            dials: Dials::from_settings(&AmpSettings::default()),
            dial_coeff: 1.0,
            tone_applied: [f32::NAN; 3],
            presence_applied: f32::NAN,
            primed: false,
            until_tick: 0,
            wet: 1.0,
            wet_step: 1.0,
        };
        chain.set_sample_rate(44100.0);
        chain
    }

    /// Allocates and designs everything that depends on the sample rate. Not for the audio thread
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.dc.set_sample_rate(sample_rate);
        self.dial_coeff = smoothing_coeff(DIAL_SMOOTH_MS, sample_rate / CHUNK as f32);
        self.wet_step = 1.0 / (BYPASS_FADE_MS * 0.001 * sample_rate);

        self.cab_irs = Amp::ALL.iter().map(|amp| design_ir(&amp.model().cab, sample_rate)).collect();
        self.cabinet.set_sample_rate(sample_rate);
        self.cabinet.set_ir(&self.cab_irs[self.amp.index()]);

        self.configure_amp();
        self.reset();
    }

    /// Changes to another amp. Does not allocate; the cabinet crossfades, the amp itself
    /// switches at once
    // Not called by the plugin until it has a second amp
    #[allow(dead_code)]
    pub fn set_amp(&mut self, amp: Amp) {
        if amp != self.amp {
            self.amp = amp;
            self.cabinet.swap_ir(&self.cab_irs[amp.index()]);
            self.configure_amp();
        }
    }

    fn configure_amp(&mut self) {
        let model = self.amp.model();
        let oversampled_rate = self.sample_rate * FACTOR as f32;
        self.preamp.configure(model, oversampled_rate);
        self.power.configure(&model.power, oversampled_rate);
        self.tone_applied = [f32::NAN; 3];
        self.presence_applied = f32::NAN;
    }

    pub fn reset(&mut self) {
        self.reset_stages();
        self.primed = false;
        self.until_tick = 0;
    }

    fn reset_stages(&mut self) {
        self.oversampler.reset();
        self.preamp.reset();
        self.tone.reset();
        self.power.reset();
        self.cabinet.reset();
        self.dc.reset();
    }

    /// Processes a block in place. With two channels the input is their average and the
    /// output goes to both
    pub fn process(&mut self, settings: &AmpSettings, left: &mut [f32], mut right: Option<&mut [f32]>) {
        let wet_target = if settings.bypass { 0.0 } else { 1.0 };
        if !self.primed {
            self.wet = wet_target;
        }

        let mut start = 0;
        while start < left.len() {
            if self.until_tick == 0 {
                self.read_dials(settings);
                self.until_tick = CHUNK;
            }
            let len = (left.len() - start).min(self.until_tick);
            let end = start + len;
            let right_chunk = right.as_deref_mut().map(|right| &mut right[start..end]);
            self.process_chunk(wet_target, &mut left[start..end], right_chunk);
            self.until_tick -= len;
            start = end;
        }
    }

    /// Moves the dials a step towards the settings and passes them on to the stages
    fn read_dials(&mut self, settings: &AmpSettings) {
        let model = self.amp.model();
        let target = Dials::from_settings(settings);
        // Nothing is heard of the amp while bypassed, so there is nothing to smooth
        let jump = !self.primed || self.wet == 0.0;
        if jump {
            self.dials = target;
        } else {
            self.dials.approach(&target, self.dial_coeff);
        }

        self.preamp.set_gain(model, self.dials.gain, OVERSAMPLED_CHUNK as u32);
        self.power.set_master(&model.power, self.dials.master, OVERSAMPLED_CHUNK as u32);
        self.out_level.set_target(self.dials.out_level, CHUNK as u32);
        if jump {
            self.preamp.snap();
            self.power.snap();
            self.out_level.snap();
        }

        let oversampled_rate = self.sample_rate * FACTOR as f32;
        let tone = [self.dials.bass, self.dials.mid, self.dials.treble];
        if tone != self.tone_applied {
            self.tone.set(&ToneCurve::new(&model.tone, tone[0], tone[1], tone[2], oversampled_rate));
            self.tone_applied = tone;
        }
        if self.dials.presence != self.presence_applied {
            self.power.set_presence(&model.power, self.dials.presence, oversampled_rate);
            self.presence_applied = self.dials.presence;
        }
        self.primed = true;
    }

    /// At most `CHUNK` samples
    fn process_chunk(&mut self, wet_target: f32, left: &mut [f32], mut right: Option<&mut [f32]>) {
        let bypassed = |chain: &Self| chain.wet == 0.0 && wet_target == 0.0;
        if bypassed(self) {
            return;
        }
        let len = left.len();

        let mut signal = [0.0f32; CHUNK];
        match &right {
            Some(right) => {
                for ((mono, l), r) in signal.iter_mut().zip(left.iter()).zip(right.iter()) {
                    *mono = 0.5 * (l + r);
                }
            }
            None => signal[..len].copy_from_slice(left),
        }

        let mut oversampled = [0.0f32; OVERSAMPLED_CHUNK];
        let high = &mut oversampled[..len * FACTOR];
        self.oversampler.upsample(&signal[..len], high);
        self.preamp.process(high);
        self.tone.process(high);
        self.power.process(high);
        self.oversampler.downsample(high, &mut signal[..len]);

        self.cabinet.process(&mut signal[..len]);

        for index in 0..len {
            let output = output_clip(self.dc.process(signal[index]) * self.out_level.next());

            if self.wet != wet_target {
                self.wet = (self.wet + self.wet_step.copysign(wet_target - self.wet)).clamp(0.0, 1.0);
            }
            if self.wet >= 1.0 {
                left[index] = output;
                if let Some(right) = right.as_deref_mut() {
                    right[index] = output;
                }
            } else {
                left[index] += self.wet * (output - left[index]);
                if let Some(right) = right.as_deref_mut() {
                    right[index] += self.wet * (output - right[index]);
                }
            }
        }

        // Start from silence when the amp comes back, not from where it was left
        if bypassed(self) {
            self.reset_stages();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cab::ir_magnitude;
    use crate::dsp::shaper::{asym_clip, AsymClipper};
    use crate::test_util::*;
    use std::time::Instant;

    const SAMPLE_RATE: f32 = 48000.0;
    const BLOCK: usize = 64;

    // Tones for the aliasing measurements: their harmonics fall between each other's
    // aliases at the usual sample rates
    const ALIAS_TONES_HZ: [f32; 2] = [1245.0, 4186.0];

    fn new_chain(sample_rate: f32) -> AmpChain {
        let mut chain = AmpChain::new();
        chain.set_sample_rate(sample_rate);
        chain
    }

    fn with_gain(gain: f32) -> AmpSettings {
        AmpSettings {
            gain,
            ..AmpSettings::default()
        }
    }

    fn with_all_dials(value: f32, out_level: f32) -> AmpSettings {
        AmpSettings {
            bypass: false,
            gain: value,
            bass: value,
            mid: value,
            treble: value,
            presence: value,
            master: value,
            out_level,
        }
    }

    /// Mono, in blocks of `block` samples
    fn run_blocks(chain: &mut AmpChain, settings: &AmpSettings, input: &[f32], block: usize) -> Vec<f32> {
        let mut output = input.to_vec();
        for chunk in output.chunks_mut(block) {
            chain.process(settings, chunk, None);
        }
        output
    }

    fn run(settings: &AmpSettings, input: &[f32], sample_rate: f32) -> Vec<f32> {
        run_blocks(&mut new_chain(sample_rate), settings, input, BLOCK)
    }

    fn largest_step(samples: &[f32]) -> f32 {
        samples.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max)
    }

    /// Samples from an impulse until the output first reaches half of its peak
    fn latency_samples(amp: Amp, sample_rate: f32) -> usize {
        let mut chain = new_chain(sample_rate);
        chain.set_amp(amp);
        let mut impulse = vec![0.0; 2048];
        impulse[0] = 0.05;
        let output = run_blocks(&mut chain, &with_gain(0.0), &impulse, BLOCK);
        let top = peak(&output);
        output.iter().position(|s| s.abs() >= 0.5 * top).unwrap()
    }

    /// Energy of a clipped sine that is not at its harmonics, relative to the total, in dB
    fn aliasing_db(amp: Amp, gain: f32, freq_hz: f32) -> f64 {
        let mut chain = new_chain(SAMPLE_RATE);
        chain.set_amp(amp);
        let input = sine(freq_hz, 0.178, SAMPLE_RATE, 36_000);
        let output = run_blocks(&mut chain, &with_gain(gain), &input, BLOCK);
        fit_partials(&output[12_000..], SAMPLE_RATE, &harmonics_of(freq_hz, SAMPLE_RATE)).1
    }

    #[test]
    fn test_silence_in_gives_silence_out() {
        let output = run(&with_all_dials(1.0, 1.0), &vec![0.0; 24_000], SAMPLE_RATE);
        assert!(peak(&output) < 1e-5, "Peak: {}", peak(&output));

        // Also once a loud note has rung out, and without an offset left behind
        let mut input = sine(110.0, 0.8, SAMPLE_RATE, 12_000);
        input.extend(vec![0.0; 36_000]);
        let output = run(&with_gain(1.0), &input, SAMPLE_RATE);
        let tail = &output[36_000..];
        assert!(peak(tail) < 1e-5, "Tail peak: {}", peak(tail));
        let playing = &output[2400..12_000];
        let offset = playing.iter().sum::<f32>() / playing.len() as f32;
        assert!(offset.abs() < 0.01 * rms(playing), "Offset: {}", offset);
    }

    #[test]
    fn test_output_is_bounded_at_all_sample_rates() {
        for sample_rate in [44100.0, 48000.0, 88200.0, 96000.0, 192000.0] {
            let len = (sample_rate * 0.05) as usize;
            let mut noise = Noise::new(3);
            let input: Vec<f32> = sine(97.0, 1.0, sample_rate, len).iter().map(|s| s + noise.next()).collect();

            for settings in [with_all_dials(0.0, 0.0), with_all_dials(0.0, 2.0), with_all_dials(1.0, 2.0)] {
                let output = run(&settings, &input, sample_rate);
                assert!(output.iter().all(|s| s.is_finite()), "Not finite at {} Hz", sample_rate);
                assert!(peak(&output) <= 1.0, "Peak {} at {} Hz", peak(&output), sample_rate);
            }
            let loud = run(&with_all_dials(1.0, 2.0), &input, sample_rate);
            assert!(rms(&loud) > 0.05, "No output at {} Hz", sample_rate);
        }
    }

    #[test]
    fn test_output_does_not_depend_on_block_size() {
        let input = power_chords(SAMPLE_RATE, 0.2);
        let settings = with_gain(0.7);
        let reference = run_blocks(&mut new_chain(SAMPLE_RATE), &settings, &input, input.len());

        for block in [1, 7, 32, 64, 1000] {
            let output = run_blocks(&mut new_chain(SAMPLE_RATE), &settings, &input, block);
            let difference = reference.iter().zip(&output).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(difference < 1e-5, "Blocks of {}: off by {}", block, difference);
        }
    }

    #[test]
    fn test_gain_dial_adds_distortion() {
        let input = sine(220.0, 0.178, SAMPLE_RATE, 19_200);
        let distortion = |gain: f32| thd_db(&run(&with_gain(gain), &input, SAMPLE_RATE)[9600..], SAMPLE_RATE, 220.0);
        let (clean, crunch, full) = (distortion(0.0), distortion(0.5), distortion(1.0));

        assert!(clean < -26.0, "Gain 0 is not clean: {:.1} dB", clean);
        assert!(crunch > clean + 10.0, "Gain 5 at {:.1} dB, gain 0 at {:.1} dB", crunch, clean);
        assert!(full > crunch + 1.0, "Gain 10 at {:.1} dB, gain 5 at {:.1} dB", full, crunch);
    }

    #[test]
    fn test_level_is_consistent_across_the_gain_dial() {
        let input = power_chords(SAMPLE_RATE, 0.5);
        let levels: Vec<f32> = [0.0, 0.5, 1.0]
            .iter()
            .map(|&gain| to_db(rms(&run(&with_gain(gain), &input, SAMPLE_RATE))))
            .collect();
        let loudest = levels.iter().cloned().fold(f32::MIN, f32::max);
        let quietest = levels.iter().cloned().fold(f32::MAX, f32::min);
        assert!(loudest - quietest < 6.0, "Levels: {:?}", levels);
        assert!((-19.0..=-13.0).contains(&levels[1]), "Level at the defaults: {:.1} dBFS", levels[1]);
    }

    #[test]
    fn test_tone_dials_move_their_bands() {
        // Quiet and with little gain, so the amp is close to linear
        let level = |settings: &AmpSettings, freq_hz: f32| {
            let input = sine(freq_hz, 0.01, SAMPLE_RATE, 9600);
            to_db(rms(&run(settings, &input, SAMPLE_RATE)[4800..]))
        };
        let base = with_gain(0.2);

        let bass = |value| level(&AmpSettings { bass: value, ..base }, 100.0);
        assert!(bass(1.0) > bass(0.0) + 6.0, "Bass: {:.1} to {:.1} dB", bass(0.0), bass(1.0));

        let mid = |value| level(&AmpSettings { mid: value, ..base }, 650.0);
        assert!(mid(1.0) > mid(0.0) + 6.0, "Mid: {:.1} to {:.1} dB", mid(0.0), mid(1.0));

        let treble = |value| level(&AmpSettings { treble: value, ..base }, 4000.0);
        assert!(treble(1.0) > treble(0.0) + 6.0, "Treble: {:.1} to {:.1} dB", treble(0.0), treble(1.0));

        let presence = |value| level(&AmpSettings { presence: value, ..base }, 5000.0);
        assert!(presence(1.0) > presence(0.0) + 3.0, "Presence: {:.1} to {:.1} dB", presence(0.0), presence(1.0));

        // And each leaves the far end of the spectrum mostly alone
        let far = |low: AmpSettings, high: AmpSettings, freq_hz| (level(&high, freq_hz) - level(&low, freq_hz)).abs();
        assert!(far(AmpSettings { bass: 0.0, ..base }, AmpSettings { bass: 1.0, ..base }, 4000.0) < 3.0);
        assert!(far(AmpSettings { treble: 0.0, ..base }, AmpSettings { treble: 1.0, ..base }, 100.0) < 3.0);
        assert!(far(AmpSettings { presence: 0.0, ..base }, AmpSettings { presence: 1.0, ..base }, 100.0) < 1.0);
    }

    #[test]
    fn test_aliasing_stays_low_at_full_gain() {
        // Limits are a few dB above what `amp_report` measures
        for amp in Amp::ALL {
            let low = aliasing_db(amp, 1.0, ALIAS_TONES_HZ[0]);
            let high = aliasing_db(amp, 1.0, ALIAS_TONES_HZ[1]);
            assert!(low < -90.0, "{:?}: {:.1} dB at {} Hz", amp, low, ALIAS_TONES_HZ[0]);
            assert!(high < -78.0, "{:?}: {:.1} dB at {} Hz", amp, high, ALIAS_TONES_HZ[1]);
        }
    }

    #[test]
    fn test_latency_is_under_a_millisecond() {
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0] {
                let latency_ms = latency_samples(amp, sample_rate) as f32 / sample_rate * 1000.0;
                assert!(latency_ms < 1.0, "{:?} at {} Hz: {:.2} ms", amp, sample_rate, latency_ms);
            }
        }
    }

    #[test]
    fn test_bypass_leaves_both_channels_untouched() {
        let mut chain = new_chain(SAMPLE_RATE);
        let mut noise = Noise::new(11);
        let left_in: Vec<f32> = (0..4800).map(|_| noise.next()).collect();
        let right_in: Vec<f32> = (0..4800).map(|_| noise.next() * 0.3).collect();
        let (mut left, mut right) = (left_in.clone(), right_in.clone());

        let settings = AmpSettings {
            bypass: true,
            ..AmpSettings::default()
        };
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            chain.process(&settings, l, Some(r));
        }
        assert_eq!(left, left_in);
        assert_eq!(right, right_in);
    }

    #[test]
    fn test_bypass_switches_without_a_click() {
        let mut chain = new_chain(SAMPLE_RATE);
        let input = sine(220.0, 0.3, SAMPLE_RATE, 28_800);
        let (mut left, mut right) = (input.clone(), input.clone());
        let playing = AmpSettings::default();
        let bypassed = AmpSettings {
            bypass: true,
            ..playing
        };

        // Playing, bypassed, playing again
        for (index, (l, r)) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)).enumerate() {
            let settings = if (150..300).contains(&index) { &bypassed } else { &playing };
            chain.process(settings, l, Some(r));
        }
        assert_eq!(left, right);

        let (off, on) = (150 * BLOCK, 300 * BLOCK);
        let fade = (BYPASS_FADE_MS * 0.001 * SAMPLE_RATE) as usize + 1;
        assert_eq!(left[off + fade..on], input[off + fade..on]);
        assert!(rms(&left[on + 4800..]) > 0.01);

        let own_step = largest_step(&left[4800..off]).max(largest_step(&input));
        let step_off = largest_step(&left[off - 1..off + fade + 1]);
        let step_on = largest_step(&left[on - 1..on + 2400]);
        assert!(step_off < own_step * 1.2, "Step of {} into bypass, {} while playing", step_off, own_step);
        assert!(step_on < own_step * 1.2, "Step of {} out of bypass, {} while playing", step_on, own_step);
    }

    #[test]
    fn test_dial_jumps_do_not_click() {
        let input = sine(220.0, 0.178, SAMPLE_RATE, 24_000);
        let low = AmpSettings {
            out_level: 0.25,
            ..with_all_dials(0.2, 0.25)
        };
        let jumps = [
            AmpSettings { gain: 1.0, ..low },
            AmpSettings { bass: 1.0, ..low },
            AmpSettings { mid: 1.0, ..low },
            AmpSettings { treble: 1.0, ..low },
            AmpSettings { presence: 1.0, ..low },
            AmpSettings { master: 1.0, ..low },
            AmpSettings { out_level: 1.0, ..low },
            with_all_dials(1.0, 1.0),
        ];

        for high in jumps {
            let mut chain = new_chain(SAMPLE_RATE);
            let mut output = input.clone();
            for (index, block) in output.chunks_mut(BLOCK).enumerate() {
                chain.process(if index < 150 { &low } else { &high }, block, None);
            }

            // The waveform's own steepest slope, before the jump and once it has settled
            let jump = 150 * BLOCK;
            let own_step = largest_step(&output[4800..jump]).max(largest_step(&output[jump + 9600..]));
            let step = largest_step(&output[jump - 1..jump + 9600]);
            assert!(step < own_step * 1.3, "Step of {} against {} for {:?}", step, own_step, high);
        }
    }

    #[test]
    fn test_stereo_outputs_are_equal_and_match_mono() {
        let left_in = power_chords(SAMPLE_RATE, 0.2);
        let right_in: Vec<f32> = sine(330.0, 0.1, SAMPLE_RATE, left_in.len());
        let mono_in: Vec<f32> = left_in.iter().zip(&right_in).map(|(l, r)| 0.5 * (l + r)).collect();
        let settings = AmpSettings::default();

        let (mut left, mut right) = (left_in.clone(), right_in.clone());
        let mut chain = new_chain(SAMPLE_RATE);
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            chain.process(&settings, l, Some(r));
        }
        let mono = run(&settings, &mono_in, SAMPLE_RATE);

        assert_eq!(left, right);
        assert_eq!(left, mono);
        assert!(rms(&mono) > 0.01);
    }

    #[test]
    fn test_reset_starts_over() {
        let input = power_chords(SAMPLE_RATE, 0.1);
        let mut chain = new_chain(SAMPLE_RATE);
        let first = run_blocks(&mut chain, &with_gain(0.8), &input, BLOCK);
        chain.reset();
        let second = run_blocks(&mut chain, &with_gain(0.8), &input, BLOCK);
        assert_eq!(first, second);
    }

    /// Median time to process one block, in microseconds
    fn block_time_us(chain: &mut AmpChain, settings: &AmpSettings, input: &[f32]) -> f64 {
        let mut scratch = input.to_vec();
        let mut times: Vec<f64> = scratch
            .chunks_mut(BLOCK)
            .map(|block| {
                let start = Instant::now();
                chain.process(settings, block, None);
                start.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[times.len() / 2]
    }

    #[test]
    fn test_silence_after_playing_is_not_slower() {
        // States that decay into denormal numbers would make the silent blocks many times
        // slower. Short time constants reach that range within this test; `amp_report`
        // measures a long tail
        let mut chain = new_chain(SAMPLE_RATE);
        let settings = with_gain(1.0);
        let playing = block_time_us(&mut chain, &settings, &power_chords(SAMPLE_RATE, 0.25));
        run_blocks(&mut chain, &settings, &vec![0.0; 24_000], BLOCK);
        let silent = block_time_us(&mut chain, &settings, &vec![0.0; 12_000]);
        assert!(silent < playing * 2.0, "{:.1} us per block playing, {:.1} us silent", playing, silent);
    }

    /// Prints levels, distortion, aliasing, latency and cost for every amp. Use it to compare
    /// before and after changing the DSP or a model's constants:
    ///   cargo test -p amp --release amp_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn amp_report() {
        let chords = power_chords(SAMPLE_RATE, 4.0);
        let tone = sine(220.0, 0.178, SAMPLE_RATE, 48_000);
        let gains = [0.0, 0.5, 1.0];

        println!("Input: power chords and a 220 Hz sine, both at -18 dBFS RMS. Dials at 5 except Gain. 48 kHz");
        println!("Levels in dBFS. THD of the sine. Aliasing: energy not at harmonics, dB below the total");
        println!(
            "{:<8}{:>5}{:>12}{:>12}{:>10}{:>10}{:>9}{:>12}{:>12}",
            "amp", "gain", "chords RMS", "chords pk", "sine RMS", "sine pk", "THD dB", "alias 1245", "alias 4186"
        );
        for amp in Amp::ALL {
            for gain in gains {
                let settings = with_gain(gain);
                let mut chain = new_chain(SAMPLE_RATE);
                chain.set_amp(amp);
                let chords_out = run_blocks(&mut chain, &settings, &chords, BLOCK);
                chain.reset();
                let tone_out = run_blocks(&mut chain, &settings, &tone, BLOCK);
                let settled = &tone_out[24_000..];
                println!(
                    "{:<8}{:>5.1}{:>12.1}{:>12.1}{:>10.1}{:>10.1}{:>9.1}{:>12.1}{:>12.1}",
                    amp.model().name,
                    gain * 10.0,
                    to_db(rms(&chords_out)),
                    to_db(peak(&chords_out)),
                    to_db(rms(settled)),
                    to_db(peak(settled)),
                    thd_db(settled, SAMPLE_RATE, 220.0),
                    aliasing_db(amp, gain, ALIAS_TONES_HZ[0]),
                    aliasing_db(amp, gain, ALIAS_TONES_HZ[1]),
                );
            }
        }

        println!();
        println!("Chords RMS in dBFS across Master (Gain 5) and across Gain (Master 5)");
        for amp in Amp::ALL {
            print!("{:<8}master", amp.model().name);
            for step in 0..=4 {
                let settings = AmpSettings {
                    master: step as f32 * 0.25,
                    ..AmpSettings::default()
                };
                let mut chain = new_chain(SAMPLE_RATE);
                chain.set_amp(amp);
                print!("{:>8.1}", to_db(rms(&run_blocks(&mut chain, &settings, &chords[..96_000], BLOCK))));
            }
            println!();
            print!("{:<8}gain  ", amp.model().name);
            for step in 0..=4 {
                let mut chain = new_chain(SAMPLE_RATE);
                chain.set_amp(amp);
                let output = run_blocks(&mut chain, &with_gain(step as f32 * 0.25), &chords[..96_000], BLOCK);
                print!("{:>8.1}", to_db(rms(&output)));
            }
            println!();
        }

        println!();
        println!("Spectrum of the chords in octave bands, dB relative to the whole signal");
        print!("{:<14}", "Hz from");
        for edge in &BAND_EDGES_HZ[..BAND_EDGES_HZ.len() - 1] {
            print!("{:>8}", edge);
        }
        println!();
        print!("{:<14}", "dry");
        for level in band_levels_db(&chords, SAMPLE_RATE) {
            print!("{:>8.1}", level);
        }
        println!();
        for amp in Amp::ALL {
            for gain in gains {
                let mut chain = new_chain(SAMPLE_RATE);
                chain.set_amp(amp);
                let output = run_blocks(&mut chain, &with_gain(gain), &chords, BLOCK);
                print!("{:<8}{:>4.1}  ", amp.model().name, gain * 10.0);
                for level in band_levels_db(&output, SAMPLE_RATE) {
                    print!("{:>8.1}", level);
                }
                println!();
            }
        }

        println!();
        println!("Level after the preamp alone in dBFS RMS (chords), across Gain: tunes `level_db`");
        for amp in Amp::ALL {
            print!("{:<8}", amp.model().name);
            for step in 0..=4 {
                let oversampled_rate = SAMPLE_RATE * FACTOR as f32;
                let mut oversampler = Oversampler::new();
                let mut preamp = Preamp::new();
                preamp.configure(amp.model(), oversampled_rate);
                preamp.set_gain(amp.model(), step as f32 * 0.25, 0);
                let mut high = vec![0.0; 96_000 * FACTOR];
                oversampler.upsample(&chords[..96_000], &mut high);
                preamp.process(&mut high);
                print!("{:>8.1}", to_db(rms(&high)));
            }
            println!();
        }

        println!();
        println!("Cabinet response in dB");
        let probes = [
            [63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0],
            [1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 16000.0],
        ];
        for row in probes {
            print!("{:<8}", "Hz");
            for probe in row {
                print!("{:>7}", probe);
            }
            println!();
            for amp in Amp::ALL {
                let ir = design_ir(&amp.model().cab, SAMPLE_RATE);
                print!("{:<8}", amp.model().name);
                for probe in row {
                    print!("{:>7.1}", to_db(ir_magnitude(&ir, probe, SAMPLE_RATE)));
                }
                println!();
            }
        }

        println!();
        println!("Latency (impulse to the first output sample at half the peak level)");
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                let samples = latency_samples(amp, sample_rate);
                println!(
                    "{:<8}{:>8} Hz{:>5} samples{:>7.3} ms",
                    amp.model().name,
                    sample_rate,
                    samples,
                    samples as f32 / sample_rate * 1000.0
                );
            }
        }

        println!();
        println!("Time per {}-sample block, median, and as a share of real time", BLOCK);
        for amp in Amp::ALL {
            for sample_rate in [48000.0, 96000.0, 192000.0] {
                let block_us = BLOCK as f64 / sample_rate as f64 * 1e6;
                let mut chain = new_chain(sample_rate);
                chain.set_amp(amp);
                let settings = with_gain(1.0);
                let chords = power_chords(sample_rate, 2.0);
                run_blocks(&mut chain, &settings, &chords, BLOCK);
                // Long enough for the slowest state in the chain to have decayed all the way
                run_blocks(&mut chain, &settings, &vec![0.0; (sample_rate * 20.0) as usize], BLOCK);
                let silent = block_time_us(&mut chain, &settings, &vec![0.0; (sample_rate * 2.0) as usize]);
                // Measured right after each other, so the processor is in the same mood for both
                let playing = block_time_us(&mut chain, &settings, &chords);
                println!(
                    "{:<8}{:>8} Hz  playing{:>7.1} us{:>6.1} %   silent tail{:>7.1} us{:>6.1} %",
                    amp.model().name,
                    sample_rate,
                    playing,
                    playing / block_us * 100.0,
                    silent,
                    silent / block_us * 100.0
                );
            }
        }

        println!();
        println!("Clipping curves, ns per sample");
        let ramp: Vec<f32> = (0..1_000_000).map(|i| ((i % 2000) as f32 - 1000.0) * 0.004).collect();
        let time_ns = |name: &str, shape: &mut dyn FnMut(f32) -> f32| {
            let start = Instant::now();
            let sum: f32 = ramp.iter().map(|&x| shape(x)).sum();
            std::hint::black_box(sum);
            println!("{:<22}{:>6.1}", name, start.elapsed().as_secs_f64() * 1e9 / ramp.len() as f64);
        };
        let mut clipper = AsymClipper::new();
        time_ns("asym_clip", &mut |x| asym_clip(x, 1.0, 1.5));
        time_ns("AsymClipper", &mut |x| clipper.process(x));
        time_ns("tanh", &mut |x| x.tanh());
    }

    /// Writes WAV files to target/renders for listening: a direct guitar signal, dry and
    /// through every amp at a few settings:
    ///   cargo test -p amp --release render_wavs -- --ignored
    ///
    /// Set AMP_INPUT_WAV to the path of a recording to use that instead of the made-up one.
    /// It is processed at its own sample rate.
    #[test]
    #[ignore]
    fn render_wavs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();

        let (input, sample_rate) = match std::env::var("AMP_INPUT_WAV") {
            Ok(path) => read_wav_mono(&path),
            Err(_) => (guitar_di(SAMPLE_RATE), SAMPLE_RATE as u32),
        };
        write_wav(&dir.join("amp_dry.wav"), &[&input], sample_rate);

        let settings = [
            ("gain0", with_gain(0.0)),
            ("gain3", with_gain(0.3)),
            ("gain5", with_gain(0.5)),
            ("gain7", with_gain(0.7)),
            ("gain10", with_gain(1.0)),
            (
                "gain5_master10",
                AmpSettings {
                    master: 1.0,
                    out_level: 0.5,
                    ..AmpSettings::default()
                },
            ),
            (
                "gain7_scooped",
                AmpSettings {
                    gain: 0.7,
                    bass: 0.8,
                    mid: 0.2,
                    treble: 0.8,
                    ..AmpSettings::default()
                },
            ),
        ];
        for amp in Amp::ALL {
            for (name, settings) in &settings {
                let mut chain = new_chain(sample_rate as f32);
                chain.set_amp(amp);
                let output = run_blocks(&mut chain, settings, &input, BLOCK);
                let file = format!("amp_{}_{}.wav", format!("{:?}", amp).to_lowercase(), name);
                write_wav(&dir.join(file), &[&output, &output], sample_rate);
            }
        }
        println!("Wrote renders to {}", dir.display());
    }
}
