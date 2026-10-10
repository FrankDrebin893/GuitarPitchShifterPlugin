use crate::amp::model::CabModel;
use crate::dsp::filters::{Biquad, BiquadCoeffs};

/// The longest impulse response the cabinet can hold: 20 ms at 192 kHz
pub const MAX_IR_LEN: usize = 3840;

// Long enough for the speaker resonance to ring out
const IR_MS: f32 = 20.0;

// Share of the impulse response, at its end, that is faded to zero
const FADE_SHARE: f32 = 0.5;

// The response is levelled so that its average over this band is 0 dB
const LEVEL_BAND_HZ: [f32; 2] = [100.0, 4000.0];
const LEVEL_PROBES: usize = 32;

// Two second-order sections with these Qs make a maximally flat 24 dB per octave slope
const ROLLOFF_QS: [f32; 2] = [0.541, 1.307];

const BREAKUP_Q: [f32; 2] = [6.0, 16.0];

// Crossfade when the impulse response is swapped while playing
const SWAP_MS: f32 = 10.0;

/// Number of samples of an impulse response at a sample rate
pub fn ir_len(sample_rate: f32) -> usize {
    ((IR_MS * 0.001 * sample_rate).round() as usize).clamp(16, MAX_IR_LEN)
}

/// Designs a cabinet's impulse response. The same model sounds the same at every sample
/// rate. Allocates and takes a while: not for the audio thread
pub fn design_ir(model: &CabModel, sample_rate: f32) -> Vec<f32> {
    let mut filters = vec![
        BiquadCoeffs::highpass(model.resonance_hz, model.resonance_q, sample_rate),
        BiquadCoeffs::peak(model.body[0], model.body[1], model.body[2], sample_rate),
        BiquadCoeffs::peak(model.dip[0], model.dip[1], model.dip[2], sample_rate),
        BiquadCoeffs::peak(model.bite[0], model.bite[1], model.bite[2], sample_rate),
        BiquadCoeffs::lowpass(model.rolloff_hz, ROLLOFF_QS[0], sample_rate),
        BiquadCoeffs::lowpass(model.rolloff_hz, ROLLOFF_QS[1], sample_rate),
        // A last, gentler slope above the roll-off, as a cone loses what little is left up there
        BiquadCoeffs::lowpass(model.rolloff_hz * model.air_ratio, 0.707, sample_rate),
    ];

    // Cone breakup: one narrow peak or notch per slice of the range, placed by the seed
    let mut random = Random::new(model.breakup_seed);
    let span = model.breakup_hz[1] / model.breakup_hz[0];
    for index in 0..model.breakup_count {
        let position = (index as f32 + random.next()) / model.breakup_count as f32;
        let freq = model.breakup_hz[0] * span.powf(position);
        let gain_db = model.breakup_db * (0.3 + 0.7 * random.next()) * if random.next() < 0.5 { -1.0 } else { 1.0 };
        let q = BREAKUP_Q[0] + (BREAKUP_Q[1] - BREAKUP_Q[0]) * random.next();
        filters.push(BiquadCoeffs::peak(freq, q, gain_db, sample_rate));
    }

    let mut sections: Vec<Biquad> = filters
        .iter()
        .map(|&coeffs| {
            let mut biquad = Biquad::new();
            biquad.set(coeffs);
            biquad
        })
        .collect();

    let len = ir_len(sample_rate);
    let fade_start = ((1.0 - FADE_SHARE) * len as f32) as usize;
    let mut ir: Vec<f32> = (0..len)
        .map(|index| {
            let impulse = if index == 0 { 1.0 } else { 0.0 };
            let sample = sections.iter_mut().fold(impulse, |signal, section| section.process(signal));
            let fade = if index < fade_start {
                1.0
            } else {
                let position = (index + 1 - fade_start) as f64 / (len - fade_start) as f64;
                0.5 + 0.5 * (std::f64::consts::PI * position).cos()
            };
            (sample * fade) as f32
        })
        .collect();

    let ratio = (LEVEL_BAND_HZ[1] / LEVEL_BAND_HZ[0]).powf(1.0 / (LEVEL_PROBES - 1) as f32);
    let power: f32 = (0..LEVEL_PROBES)
        .map(|probe| ir_magnitude(&ir, LEVEL_BAND_HZ[0] * ratio.powi(probe as i32), sample_rate).powi(2))
        .sum::<f32>()
        / LEVEL_PROBES as f32;
    let gain = 1.0 / power.sqrt().max(1e-9);
    for sample in &mut ir {
        *sample *= gain;
    }
    ir
}

/// Gain of an impulse response at one frequency
pub fn ir_magnitude(ir: &[f32], freq_hz: f32, sample_rate: f32) -> f32 {
    let omega = std::f64::consts::TAU * freq_hz as f64 / sample_rate as f64;
    let (re, im) = ir.iter().enumerate().fold((0.0, 0.0), |(re, im), (index, &sample)| {
        let (sin, cos) = (omega * index as f64).sin_cos();
        (re + sample as f64 * cos, im - sample as f64 * sin)
    });
    (re * re + im * im).sqrt() as f32
}

/// Numbers in 0.0..1.0 from a seed (xorshift), the same on every run
struct Random {
    state: u32,
}

impl Random {
    fn new(seed: u32) -> Self {
        Self {
            state: seed.wrapping_mul(0x9E37_79B9) | 1,
        }
    }

    fn next(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state >> 8) as f32 / (1 << 24) as f32
    }
}

/// Sum of products. Sixteen running sums instead of one, so the compiler can use vector
/// instructions (it may not reorder a single floating point sum by itself)
fn dot(a: &[f32], b: &[f32]) -> f32 {
    const LANES: usize = 16;
    let mut sums = [0.0f32; LANES];
    let mut a_chunks = a.chunks_exact(LANES);
    let mut b_chunks = b.chunks_exact(LANES);
    for (a_chunk, b_chunk) in (&mut a_chunks).zip(&mut b_chunks) {
        for lane in 0..LANES {
            sums[lane] += a_chunk[lane] * b_chunk[lane];
        }
    }
    let rest: f32 = a_chunks.remainder().iter().zip(b_chunks.remainder()).map(|(x, y)| x * y).sum();
    sums.iter().sum::<f32>() + rest
}

/// The cabinet: convolution with an impulse response, computed directly so it adds no
/// latency. Holds two responses so one can be swapped for another without a click
pub struct Cabinet {
    len: usize,
    // The last `len` input samples, as a ring
    history: Vec<f32>,
    position: usize,
    // Impulse responses stored backwards, so they line up with the history oldest first
    taps: [Vec<f32>; 2],
    active: usize,
    // 1.0 when only the active response plays, less while the previous one fades out
    fade: f32,
    fade_step: f32,
}

impl Cabinet {
    pub fn new() -> Self {
        let mut through = vec![0.0; MAX_IR_LEN];
        through[MAX_IR_LEN - 1] = 1.0;
        Self {
            len: MAX_IR_LEN,
            history: vec![0.0; MAX_IR_LEN],
            position: 0,
            taps: [through.clone(), through],
            active: 0,
            fade: 1.0,
            fade_step: 0.0,
        }
    }

    /// Sets the length the impulse responses have at this rate. Load one with `set_ir` after
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.len = ir_len(sample_rate);
        self.fade_step = 1.0 / (SWAP_MS * 0.001 * sample_rate);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.position = 0;
        self.fade = 1.0;
    }

    /// Loads an impulse response (first sample first) and uses it at once. Longer ones are
    /// cut to the cabinet's length. Does not allocate
    pub fn set_ir(&mut self, ir: &[f32]) {
        self.load(self.active, ir);
        self.fade = 1.0;
    }

    /// Loads an impulse response and crossfades to it. Only while `is_swapping` is false:
    /// there are two slots, and during a crossfade both are heard
    pub fn swap_ir(&mut self, ir: &[f32]) {
        self.active = 1 - self.active;
        self.load(self.active, ir);
        self.fade = 0.0;
    }

    /// True while the crossfade started by `swap_ir` is running
    pub fn is_swapping(&self) -> bool {
        self.fade < 1.0
    }

    /// Samples after which a crossfade is sure to be over
    pub fn swap_len(&self) -> usize {
        (1.0 / self.fade_step).ceil() as usize + 1
    }

    /// Address and capacity of every buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub fn buffers(&self) -> [(usize, usize); 3] {
        let describe = |buffer: &Vec<f32>| (buffer.as_ptr() as usize, buffer.capacity());
        [describe(&self.history), describe(&self.taps[0]), describe(&self.taps[1])]
    }

    fn load(&mut self, slot: usize, ir: &[f32]) {
        let taps = &mut self.taps[slot][..self.len];
        taps.fill(0.0);
        for (tap, &sample) in taps.iter_mut().rev().zip(ir) {
            *tap = sample;
        }
    }

    fn convolve(&self, slot: usize) -> f32 {
        // The oldest sample sits right after the newest one in the ring: two straight runs
        let taps = &self.taps[slot][..self.len];
        let split = self.len - 1 - self.position;
        dot(&self.history[self.position + 1..self.len], &taps[..split])
            + dot(&self.history[..=self.position], &taps[split..])
    }

    pub fn process(&mut self, block: &mut [f32]) {
        for sample in block.iter_mut() {
            self.history[self.position] = *sample;

            let mut output = self.convolve(self.active);
            if self.fade < 1.0 {
                output = self.fade * output + (1.0 - self.fade) * self.convolve(1 - self.active);
                self.fade = (self.fade + self.fade_step).min(1.0);
            }
            *sample = output;

            self.position += 1;
            if self.position == self.len {
                self.position = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amp::model::Amp;
    use crate::test_util::{rms, sine, to_db, Noise};

    const PROBES_HZ: [f32; 7] = [100.0, 250.0, 500.0, 1000.0, 2000.0, 3000.0, 4000.0];

    fn design(amp: Amp, sample_rate: f32) -> Vec<f32> {
        design_ir(&amp.model().cab, sample_rate)
    }

    #[test]
    fn test_ir_is_finite_and_levelled() {
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                let ir = design(amp, sample_rate);
                assert_eq!(ir.len(), ir_len(sample_rate));
                assert!(ir.iter().all(|s| s.is_finite()));
                assert_eq!(*ir.last().unwrap(), 0.0);

                let ratio = (LEVEL_BAND_HZ[1] / LEVEL_BAND_HZ[0]).powf(1.0 / 99.0);
                let power: f32 = (0..100)
                    .map(|i| ir_magnitude(&ir, LEVEL_BAND_HZ[0] * ratio.powi(i), sample_rate).powi(2))
                    .sum::<f32>()
                    / 100.0;
                assert!(to_db(power.sqrt()).abs() < 0.5, "{:?} at {}: {:.2} dB", amp, sample_rate, to_db(power.sqrt()));
            }
        }
    }

    #[test]
    fn test_ir_length_follows_the_sample_rate_up_to_the_limit() {
        assert_eq!(ir_len(48000.0), 960);
        assert_eq!(ir_len(192000.0), MAX_IR_LEN);
        assert_eq!(ir_len(384000.0), MAX_IR_LEN);
    }

    #[test]
    fn test_ir_energy_is_at_the_start() {
        for amp in Amp::ALL {
            let sample_rate = 48000.0;
            let ir = design(amp, sample_rate);
            let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
            let early = energy(&ir[..(0.005 * sample_rate) as usize]) / energy(&ir);
            assert!(early > 0.9, "{:?}: {:.2} of the energy in the first 5 ms", amp, early);
        }
    }

    #[test]
    fn test_ir_sounds_the_same_at_every_sample_rate() {
        for amp in Amp::ALL {
            let reference = design(amp, 48000.0);
            for sample_rate in [44100.0, 96000.0, 192000.0] {
                let ir = design(amp, sample_rate);
                for freq in PROBES_HZ {
                    let difference =
                        to_db(ir_magnitude(&ir, freq, sample_rate) / ir_magnitude(&reference, freq, 48000.0));
                    assert!(
                        difference.abs() < 1.0,
                        "{:?} at {} Hz and {} Hz rate: {:.2} dB",
                        amp,
                        freq,
                        sample_rate,
                        difference
                    );
                }
            }
        }
    }

    #[test]
    fn test_ir_has_the_shape_of_a_guitar_speaker() {
        for amp in Amp::ALL {
            let ir = design(amp, 48000.0);
            let level = |freq: f32| to_db(ir_magnitude(&ir, freq, 48000.0));
            let mids = level(1000.0);
            assert!(level(30.0) < mids - 15.0, "{:?} keeps sub bass", amp);
            assert!(level(10000.0) < mids - 20.0, "{:?} keeps fizz", amp);
            assert!(level(amp.model().cab.resonance_hz) > mids - 8.0, "{:?} has no low end", amp);
        }
    }

    #[test]
    fn test_convolution_plays_back_the_impulse_response() {
        let sample_rate = 48000.0;
        let ir = design(Amp::ALL[0], sample_rate);
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&ir);

        let mut block = vec![0.0; ir.len() + 100];
        block[0] = 1.0;
        cabinet.process(&mut block);
        for (index, &expected) in ir.iter().enumerate() {
            assert!((block[index] - expected).abs() < 1e-6, "Tap {}", index);
        }
        assert!(block[ir.len()..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn test_convolution_matches_the_plain_sum() {
        let sample_rate = 44100.0;
        let ir = design(Amp::ALL[0], sample_rate);
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&ir);

        let mut noise = Noise::new(7);
        let input: Vec<f32> = (0..3000).map(|_| noise.next()).collect();
        let mut output = input.clone();
        // Uneven blocks, so the ring wraps at different places within them
        for block in output.chunks_mut(37) {
            cabinet.process(block);
        }

        for index in (0..input.len()).step_by(97) {
            let expected: f64 = (0..=index.min(ir.len() - 1))
                .map(|tap| ir[tap] as f64 * input[index - tap] as f64)
                .sum();
            assert!((output[index] as f64 - expected).abs() < 1e-4, "Sample {}", index);
        }
    }

    #[test]
    fn test_swap_crossfades_without_a_click() {
        let sample_rate = 48000.0;
        let ir = design(Amp::ALL[0], sample_rate);
        let quieter: Vec<f32> = ir.iter().map(|s| s * 0.25).collect();
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&ir);

        let mut block = sine(500.0, 0.5, sample_rate, 9600);
        cabinet.process(&mut block[..4800]);
        cabinet.swap_ir(&quieter);
        cabinet.process(&mut block[4800..]);

        let before = rms(&block[2400..4800]);
        let after = rms(&block[7200..]);
        assert!((after / before - 0.25).abs() < 0.01, "Level ratio: {}", after / before);

        let largest_step = block[2400..].windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max);
        let own_step = block[2400..4800].windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max);
        assert!(largest_step < own_step * 1.05, "Step of {} against {}", largest_step, own_step);
    }

    #[test]
    fn test_short_and_long_responses_fit() {
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(48000.0);

        cabinet.set_ir(&[0.5]);
        let mut block = [1.0, 0.0, 0.0];
        cabinet.process(&mut block);
        assert_eq!(block, [0.5, 0.0, 0.0]);

        cabinet.reset();
        cabinet.set_ir(&vec![0.1; MAX_IR_LEN * 2]);
        let mut long = vec![1.0; 2000];
        cabinet.process(&mut long);
        assert!((long[1999] - 0.1 * 960.0).abs() < 1e-2);
    }
}
