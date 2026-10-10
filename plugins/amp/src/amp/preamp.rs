use super::model::{AmpModel, StageModel, MAX_STAGES};
use crate::dsp::filters::{OnePoleHp, OnePoleLp, ANTI_DENORMAL};
use crate::dsp::shaper::{asym_clip, AsymClipper};
use crate::dsp::{curve, db_to_gain, lerp, smoothing_coeff, Ramp};

// How fast hard playing moves a stage's operating point, and how slowly it comes back
const BIAS_ATTACK_MS: f32 = 4.0;
const BIAS_RELEASE_MS: f32 = 120.0;

/// One gain stage: coupling high-pass, gain, asymmetric clip around a moving operating
/// point, low-pass
#[derive(Clone, Copy)]
struct Stage {
    coupling: OnePoleHp,
    gain: Ramp,
    clipper: AsymClipper,
    lowpass: OnePoleLp,
    headroom: [f32; 2],
    bias: f32,
    bias_shift: f32,
    // Follows how far the signal is driven past the top of the curve
    overdrive: f32,
    attack: f32,
    release: f32,
}

impl Stage {
    fn new() -> Self {
        Self {
            coupling: OnePoleHp::new(),
            gain: Ramp::new(1.0),
            clipper: AsymClipper::new(),
            lowpass: OnePoleLp::new(),
            headroom: [1.0, 1.0],
            bias: 0.0,
            bias_shift: 0.0,
            overdrive: 0.0,
            attack: 1.0,
            release: 1.0,
        }
    }

    fn configure(&mut self, model: &StageModel, sample_rate: f32) {
        self.coupling.set(model.coupling_hz, sample_rate);
        self.lowpass.set(model.lowpass_hz, sample_rate);
        self.clipper.set_limits(model.headroom[0], model.headroom[1]);
        self.headroom = model.headroom;
        self.bias = model.bias;
        self.bias_shift = model.bias_shift;
        self.attack = smoothing_coeff(BIAS_ATTACK_MS, sample_rate);
        self.release = smoothing_coeff(BIAS_RELEASE_MS, sample_rate);
    }

    fn reset(&mut self) {
        self.coupling.reset();
        self.clipper.reset(self.bias);
        self.lowpass.reset();
        self.overdrive = 0.0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let driven = self.coupling.process(input) * self.gain.next();

        // Peaks past the top of the curve charge the coupling capacitor, which pulls the
        // operating point down until it has drained again
        let excess = (driven - self.headroom[0]).max(0.0);
        let coeff = if excess > self.overdrive { self.attack } else { self.release };
        self.overdrive += coeff * (excess - self.overdrive) + ANTI_DENORMAL;
        let bias = self.bias - self.bias_shift * self.overdrive / (1.0 + self.overdrive);

        let clipped = self.clipper.process(driven + bias) - asym_clip(bias, self.headroom[0], self.headroom[1]);
        // A gain stage inverts, so the next one clips the other half harder
        -self.lowpass.process(clipped)
    }
}

/// The gain stages of an amp in series
pub struct Preamp {
    tight: OnePoleHp,
    bright: OnePoleHp,
    bright_gain: Ramp,
    stages: [Stage; MAX_STAGES],
    stage_count: usize,
    level: Ramp,
}

impl Preamp {
    pub fn new() -> Self {
        Self {
            tight: OnePoleHp::new(),
            bright: OnePoleHp::new(),
            bright_gain: Ramp::new(0.0),
            stages: [Stage::new(); MAX_STAGES],
            stage_count: 1,
            level: Ramp::new(1.0),
        }
    }

    /// `sample_rate` is the rate the preamp runs at (the oversampled one)
    pub fn configure(&mut self, model: &AmpModel, sample_rate: f32) {
        self.tight.set(model.tight_hz, sample_rate);
        self.bright.set(model.bright_hz, sample_rate);
        self.stage_count = model.stage_count.clamp(1, MAX_STAGES);
        for (stage, stage_model) in self.stages.iter_mut().zip(&model.stages) {
            stage.configure(stage_model, sample_rate);
        }
    }

    pub fn reset(&mut self) {
        self.tight.reset();
        self.bright.reset();
        for stage in &mut self.stages {
            stage.reset();
        }
    }

    /// Moves to a Gain dial position over the next `steps` samples
    pub fn set_gain(&mut self, model: &AmpModel, gain: f32, steps: u32) {
        for (stage, stage_model) in self.stages.iter_mut().zip(&model.stages) {
            let gain_db = lerp(stage_model.gain_db[0], stage_model.gain_db[1], gain);
            stage.gain.set_target(db_to_gain(gain_db), steps);
        }

        // Odd numbers of inverting stages are turned back, so the amp keeps the polarity
        // of its input
        let polarity = if self.stage_count % 2 == 1 { -1.0 } else { 1.0 };
        self.level.set_target(polarity * db_to_gain(curve(&model.level_db, gain)), steps);

        let bright = (1.0 - gain) * (1.0 - gain);
        self.bright_gain.set_target((db_to_gain(model.bright_db) - 1.0) * bright, steps);
    }

    /// Ends the move started by `set_gain` at once
    pub fn snap(&mut self) {
        for stage in &mut self.stages {
            stage.gain.snap();
        }
        self.level.snap();
        self.bright_gain.snap();
    }

    pub fn process(&mut self, block: &mut [f32]) {
        for sample in block.iter_mut() {
            let tight = self.tight.process(*sample);
            let mut signal = tight + self.bright.process(tight) * self.bright_gain.next();
            for stage in &mut self.stages[..self.stage_count] {
                signal = stage.process(signal);
            }
            *sample = signal * self.level.next();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amp::model::Amp;
    use crate::test_util::{fit_partials, harmonics_of, peak, rms, sine, to_db};

    const SAMPLE_RATE: f32 = 192_000.0;
    const LEN: usize = 96_000;

    fn run(amp: Amp, gain: f32, input: &[f32]) -> Vec<f32> {
        let mut preamp = Preamp::new();
        preamp.configure(amp.model(), SAMPLE_RATE);
        preamp.set_gain(amp.model(), gain, 0);
        let mut output = input.to_vec();
        preamp.process(&mut output);
        output
    }

    /// Level of the harmonics relative to the fundamental, in dB
    fn harmonics_db(output: &[f32], freq: f32) -> f32 {
        let partials: Vec<f64> = harmonics_of(freq, SAMPLE_RATE).into_iter().take(12).collect();
        let (levels, _) = fit_partials(&output[LEN / 2..], SAMPLE_RATE, &partials);
        let harmonics: f64 = levels[1..].iter().map(|level| level * level).sum();
        to_db((harmonics.sqrt() / levels[0]) as f32)
    }

    #[test]
    fn test_gain_dial_adds_distortion() {
        for amp in Amp::ALL {
            let input = sine(440.0, 0.15, SAMPLE_RATE, LEN);
            let clean = harmonics_db(&run(amp, 0.0, &input), 440.0);
            let crunch = harmonics_db(&run(amp, 0.5, &input), 440.0);
            let full = harmonics_db(&run(amp, 1.0, &input), 440.0);
            assert!(clean < -30.0, "{:?} at gain 0: {:.1} dB", amp, clean);
            assert!(crunch > clean + 10.0, "{:?} at gain 5: {:.1} dB", amp, crunch);
            assert!(full > crunch, "{:?} at gain 10: {:.1} dB", amp, full);
        }
    }

    #[test]
    fn test_soft_playing_is_cleaner_than_hard_playing() {
        for amp in Amp::ALL {
            let soft = harmonics_db(&run(amp, 0.5, &sine(440.0, 0.02, SAMPLE_RATE, LEN)), 440.0);
            let hard = harmonics_db(&run(amp, 0.5, &sine(440.0, 0.3, SAMPLE_RATE, LEN)), 440.0);
            assert!(soft < hard - 12.0, "{:?}: soft {:.1} dB, hard {:.1} dB", amp, soft, hard);
        }
    }

    #[test]
    fn test_output_is_bounded_and_keeps_polarity() {
        for amp in Amp::ALL {
            let loud = run(amp, 1.0, &sine(200.0, 4.0, SAMPLE_RATE, LEN));
            assert!(loud.iter().all(|s| s.is_finite()));
            assert!(peak(&loud) < 4.0, "{:?} peak: {}", amp, peak(&loud));

            // A slow, small sine comes out the same way up
            let input = sine(1000.0, 0.01, SAMPLE_RATE, LEN);
            let output = run(amp, 0.3, &input);
            let dot: f32 = input[LEN / 2..].iter().zip(&output[LEN / 2..]).map(|(a, b)| a * b).sum();
            assert!(dot > 0.0, "{:?} inverts", amp);
        }
    }

    #[test]
    fn test_loudness_stays_within_reach_across_the_gain_dial() {
        for amp in Amp::ALL {
            let input = sine(440.0, 0.15, SAMPLE_RATE, LEN);
            let levels: Vec<f32> = [0.0, 0.25, 0.5, 0.75, 1.0]
                .iter()
                .map(|&gain| to_db(rms(&run(amp, gain, &input)[LEN / 2..])))
                .collect();
            let loudest = levels.iter().cloned().fold(f32::MIN, f32::max);
            let quietest = levels.iter().cloned().fold(f32::MAX, f32::min);
            assert!(loudest - quietest < 9.0, "{:?} levels: {:?}", amp, levels);
        }
    }
}
