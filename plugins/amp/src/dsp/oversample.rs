use super::filters::ANTI_DENORMAL;

/// How many times the sample rate is raised
pub const FACTOR: usize = 4;

// Half-band filters made of two parallel chains of allpass sections (polyphase IIR, minimum
// phase, so the delay is a few samples). Coefficients come from the standard elliptic design
// for this structure, given a number of sections and a transition width as a fraction of
// the filter's own (doubled) sample rate. Even entries are one chain, odd entries the other.
//
// First doubling: 5 sections, transition 0.0833, 80 dB rejection. Passband to 20 kHz and
// stopband from 28 kHz at a 48 kHz host rate
const STAGE_1: [f32; 5] = [
    0.061_106_325,
    0.221_113_243,
    0.430_533_776,
    0.651_029_500,
    0.876_685_896,
];

// Second doubling: 2 sections, transition 0.3, 72 dB rejection. It can be this gentle
// because at this rate the audio band (to 19 kHz) and its image (from 77 kHz) are far apart
const STAGE_2: [f32; 2] = [0.124_744_526, 0.562_584_953];

struct HalfBand<const N: usize> {
    coeffs: [f32; N],
    last_input: [f32; N],
    last_output: [f32; N],
}

impl<const N: usize> HalfBand<N> {
    fn new(coeffs: [f32; N]) -> Self {
        Self {
            coeffs,
            last_input: [0.0; N],
            last_output: [0.0; N],
        }
    }

    fn reset(&mut self) {
        self.last_input = [0.0; N];
        self.last_output = [0.0; N];
    }

    /// One chain: the sections `first`, `first + 2`, ... in series
    fn chain(&mut self, first: usize, input: f32) -> f32 {
        let mut signal = input;
        let mut index = first;
        while index < N {
            let output =
                self.coeffs[index] * (signal - self.last_output[index]) + self.last_input[index] + ANTI_DENORMAL;
            self.last_input[index] = signal;
            self.last_output[index] = output;
            signal = output;
            index += 2;
        }
        signal
    }

    /// One sample in, two out at twice the rate
    fn up(&mut self, input: f32) -> (f32, f32) {
        (self.chain(0, input), self.chain(1, input))
    }

    /// Two samples in, one out at half the rate
    fn down(&mut self, first: f32, second: f32) -> f32 {
        0.5 * (self.chain(0, second) + self.chain(1, first))
    }
}

/// Raises the sample rate four times and brings it back, so the clipping stages in between
/// have room for their harmonics
pub struct Oversampler {
    up_1: HalfBand<5>,
    up_2: HalfBand<2>,
    down_2: HalfBand<2>,
    down_1: HalfBand<5>,
}

impl Oversampler {
    pub fn new() -> Self {
        Self {
            up_1: HalfBand::new(STAGE_1),
            up_2: HalfBand::new(STAGE_2),
            down_2: HalfBand::new(STAGE_2),
            down_1: HalfBand::new(STAGE_1),
        }
    }

    pub fn reset(&mut self) {
        self.up_1.reset();
        self.up_2.reset();
        self.down_2.reset();
        self.down_1.reset();
    }

    /// `output` must hold `FACTOR` samples per input sample
    pub fn upsample(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(output.len(), input.len() * FACTOR);
        for (&sample, out) in input.iter().zip(output.chunks_exact_mut(FACTOR)) {
            let (even, odd) = self.up_1.up(sample);
            (out[0], out[1]) = self.up_2.up(even);
            (out[2], out[3]) = self.up_2.up(odd);
        }
    }

    /// `input` must hold `FACTOR` samples per output sample
    pub fn downsample(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), output.len() * FACTOR);
        for (frame, out) in input.chunks_exact(FACTOR).zip(output.iter_mut()) {
            let even = self.down_2.down(frame[0], frame[1]);
            let odd = self.down_2.down(frame[2], frame[3]);
            *out = self.down_1.down(even, odd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{rms, sine, to_db};

    const SAMPLE_RATE: f32 = 48000.0;
    const LEN: usize = 9600;

    fn round_trip(input: &[f32]) -> Vec<f32> {
        let mut oversampler = Oversampler::new();
        let mut high = vec![0.0; input.len() * FACTOR];
        let mut output = vec![0.0; input.len()];
        oversampler.upsample(input, &mut high);
        oversampler.downsample(&high, &mut output);
        output
    }

    /// Level of a sine after one half-band decimation, relative to its level before
    fn decimated_db<const N: usize>(coeffs: [f32; N], freq_ratio: f32) -> f32 {
        let mut filter = HalfBand::new(coeffs);
        let input = sine(freq_ratio, 1.0, 1.0, LEN);
        let output: Vec<f32> = input.chunks_exact(2).map(|pair| filter.down(pair[0], pair[1])).collect();
        to_db(rms(&output[LEN / 4..]) / rms(&input))
    }

    #[test]
    fn test_passband_is_flat_to_16_khz() {
        for freq in [50.0, 1000.0, 5000.0, 10000.0, 16000.0] {
            let input = sine(freq, 0.5, SAMPLE_RATE, LEN);
            let output = round_trip(&input);
            let change = to_db(rms(&output[LEN / 2..]) / rms(&input[LEN / 2..]));
            assert!(change.abs() < 0.5, "{} Hz changed by {:.2} dB", freq, change);
        }
    }

    #[test]
    fn test_upsampled_sine_has_the_same_level_and_no_images() {
        let mut oversampler = Oversampler::new();
        let input = sine(1000.0, 0.5, SAMPLE_RATE, LEN);
        let mut high = vec![0.0; LEN * FACTOR];
        oversampler.upsample(&input, &mut high);

        let settled = &high[LEN * 2..];
        assert!(to_db(rms(settled) / rms(&input)).abs() < 0.05);

        // What is left after taking the sine out are the images around 48, 96 and 144 kHz
        let (_, residual_db) =
            crate::test_util::fit_partials(settled, SAMPLE_RATE * FACTOR as f32, &[1000.0]);
        assert!(residual_db < -70.0, "Images at {:.1} dB", residual_db);
    }

    #[test]
    fn test_first_stage_rejects_its_stopband_by_70_db() {
        // Fractions of the doubled rate: 28 kHz and up at a 48 kHz host rate
        for freq_ratio in [0.292, 0.33, 0.4, 0.47] {
            let level = decimated_db(STAGE_1, freq_ratio);
            assert!(level < -70.0, "{} of the rate: {:.1} dB", freq_ratio, level);
        }
        assert!(decimated_db(STAGE_1, 0.1).abs() < 0.01);
    }

    #[test]
    fn test_second_stage_rejects_its_stopband_by_60_db() {
        for freq_ratio in [0.4, 0.45, 0.49] {
            let level = decimated_db(STAGE_2, freq_ratio);
            assert!(level < -60.0, "{} of the rate: {:.1} dB", freq_ratio, level);
        }
        assert!(decimated_db(STAGE_2, 0.05).abs() < 0.01);
    }

    #[test]
    fn test_round_trip_delay_is_a_few_samples() {
        let mut input = vec![0.0; 256];
        input[0] = 1.0;
        let output = round_trip(&input);

        // Centre of energy of the impulse response
        let energy: f32 = output.iter().map(|s| s * s).sum();
        let centre: f32 = output.iter().enumerate().map(|(i, s)| i as f32 * s * s).sum::<f32>() / energy;
        assert!(centre < 6.0, "Delay: {:.2} samples", centre);
        assert!((energy - 1.0).abs() < 0.05, "Energy: {}", energy);
    }

    #[test]
    fn test_states_never_decay_into_denormal_numbers() {
        let mut oversampler = Oversampler::new();
        let mut high = [0.0; 64 * FACTOR];
        let mut low = [0.0; 64];
        oversampler.upsample(&[1.0; 64], &mut high);
        oversampler.downsample(&high, &mut low);

        for _ in 0..100 {
            oversampler.upsample(&[0.0; 64], &mut high);
            oversampler.downsample(&high, &mut low);
            let states = oversampler.up_1.last_output.iter().chain(&oversampler.down_1.last_output);
            let states = states.chain(&oversampler.up_2.last_output).chain(&oversampler.down_2.last_output);
            for state in states {
                assert!(*state == 0.0 || state.is_normal(), "State: {:e}", state);
            }
        }
    }

    #[test]
    fn test_reset_clears_the_filters() {
        let mut oversampler = Oversampler::new();
        let mut high = [0.0; 64 * FACTOR];
        let mut low = [0.0; 64];
        oversampler.upsample(&[1.0; 64], &mut high);
        oversampler.downsample(&high, &mut low);
        oversampler.reset();

        oversampler.upsample(&[0.0; 64], &mut high);
        oversampler.downsample(&high, &mut low);
        assert!(low.iter().all(|s| s.abs() < 1e-15));
    }
}
