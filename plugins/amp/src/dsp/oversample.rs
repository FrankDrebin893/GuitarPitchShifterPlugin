use super::filters::ANTI_DENORMAL;

/// The most the sample rate is raised
pub const MAX_FACTOR: usize = 4;

// From this host rate on the rate is only doubled: there is as much room above the audio
// band at twice 96 kHz as at four times 48 kHz
const DOUBLE_FROM_HZ: f32 = 88_000.0;

/// How many times the sample rate is raised at a host sample rate: four times at the usual
/// rates, twice at the high ones
pub fn factor_for(sample_rate: f32) -> usize {
    if sample_rate >= DOUBLE_FROM_HZ {
        2
    } else {
        MAX_FACTOR
    }
}

// Half-band filters made of two parallel chains of allpass sections (polyphase IIR, minimum
// phase, so the delay is a few samples). Coefficients come from the standard elliptic design
// for this structure, given a number of sections and a transition width as a fraction of
// the filter's own (doubled) sample rate. Even entries are one chain, odd entries the other.
//
// First doubling: 5 sections, transition 0.0833, 80 dB rejection. Passband to 20 kHz and
// stopband from 28 kHz at a 48 kHz host rate. The only doubling at the high host rates:
// passband to 40 kHz and stopband from 56 kHz at 96 kHz
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

/// Raises the sample rate four times or twice and brings it back, so the clipping stages in
/// between have room for their harmonics
pub struct Oversampler {
    factor: usize,
    up_1: HalfBand<5>,
    up_2: HalfBand<2>,
    down_2: HalfBand<2>,
    down_1: HalfBand<5>,
}

impl Oversampler {
    pub fn new() -> Self {
        Self {
            factor: MAX_FACTOR,
            up_1: HalfBand::new(STAGE_1),
            up_2: HalfBand::new(STAGE_2),
            down_2: HalfBand::new(STAGE_2),
            down_1: HalfBand::new(STAGE_1),
        }
    }

    /// How many times the rate is raised: `MAX_FACTOR`, or 2, which leaves out the second
    /// doubling. Starts from rest
    pub fn set_factor(&mut self, factor: usize) {
        debug_assert!(factor == 2 || factor == MAX_FACTOR);
        self.factor = if factor == 2 { 2 } else { MAX_FACTOR };
        self.reset();
    }

    #[cfg(test)]
    pub fn factor(&self) -> usize {
        self.factor
    }

    pub fn reset(&mut self) {
        self.up_1.reset();
        self.up_2.reset();
        self.down_2.reset();
        self.down_1.reset();
    }

    /// `output` must hold `factor` samples per input sample
    pub fn upsample(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(output.len(), input.len() * self.factor);
        if self.factor == 2 {
            for (&sample, out) in input.iter().zip(output.chunks_exact_mut(2)) {
                (out[0], out[1]) = self.up_1.up(sample);
            }
        } else {
            for (&sample, out) in input.iter().zip(output.chunks_exact_mut(MAX_FACTOR)) {
                let (even, odd) = self.up_1.up(sample);
                (out[0], out[1]) = self.up_2.up(even);
                (out[2], out[3]) = self.up_2.up(odd);
            }
        }
    }

    /// `input` must hold `factor` samples per output sample
    pub fn downsample(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), output.len() * self.factor);
        if self.factor == 2 {
            for (frame, out) in input.chunks_exact(2).zip(output.iter_mut()) {
                *out = self.down_1.down(frame[0], frame[1]);
            }
        } else {
            for (frame, out) in input.chunks_exact(MAX_FACTOR).zip(output.iter_mut()) {
                let even = self.down_2.down(frame[0], frame[1]);
                let odd = self.down_2.down(frame[2], frame[3]);
                *out = self.down_1.down(even, odd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{rms, sine, to_db};

    const SAMPLE_RATE: f32 = 48000.0;
    const LEN: usize = 9600;

    fn round_trip_at(factor: usize, input: &[f32]) -> Vec<f32> {
        let mut oversampler = Oversampler::new();
        oversampler.set_factor(factor);
        let mut high = vec![0.0; input.len() * factor];
        let mut output = vec![0.0; input.len()];
        oversampler.upsample(input, &mut high);
        oversampler.downsample(&high, &mut output);
        output
    }

    fn round_trip(input: &[f32]) -> Vec<f32> {
        round_trip_at(MAX_FACTOR, input)
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
        let mut high = vec![0.0; LEN * MAX_FACTOR];
        oversampler.upsample(&input, &mut high);

        let settled = &high[LEN * 2..];
        assert!(to_db(rms(settled) / rms(&input)).abs() < 0.05);

        // What is left after taking the sine out are the images around 48, 96 and 144 kHz
        let (_, residual_db) =
            crate::test_util::fit_partials(settled, SAMPLE_RATE * MAX_FACTOR as f32, &[1000.0]);
        assert!(residual_db < -70.0, "Images at {:.1} dB", residual_db);
    }

    #[test]
    fn test_factor_follows_the_host_rate() {
        assert_eq!(factor_for(44100.0), 4);
        assert_eq!(factor_for(48000.0), 4);
        assert_eq!(factor_for(88200.0), 2);
        assert_eq!(factor_for(96000.0), 2);
        assert_eq!(factor_for(192000.0), 2);
    }

    #[test]
    fn test_doubling_alone_is_flat_to_20_khz_and_has_no_images() {
        // At a 96 kHz host rate
        let host_rate = 96000.0;
        for freq in [50.0, 1000.0, 10000.0, 20000.0] {
            let input = sine(freq, 0.5, host_rate, LEN);
            let output = round_trip_at(2, &input);
            let change = to_db(rms(&output[LEN / 2..]) / rms(&input[LEN / 2..]));
            assert!(change.abs() < 0.1, "{} Hz changed by {:.2} dB", freq, change);
        }

        let mut oversampler = Oversampler::new();
        oversampler.set_factor(2);
        assert_eq!(oversampler.factor(), 2);
        let input = sine(1000.0, 0.5, host_rate, LEN);
        let mut high = vec![0.0; LEN * 2];
        oversampler.upsample(&input, &mut high);
        let settled = &high[LEN..];
        assert!(to_db(rms(settled) / rms(&input)).abs() < 0.05);
        // What is left after taking the sine out is the image around 96 kHz
        let (_, residual_db) = crate::test_util::fit_partials(settled, host_rate * 2.0, &[1000.0]);
        assert!(residual_db < -70.0, "Image at {:.1} dB", residual_db);
    }

    #[test]
    fn test_doubling_alone_is_faster_through() {
        let delay = |factor: usize| {
            let mut input = vec![0.0; 256];
            input[0] = 1.0;
            let output = round_trip_at(factor, &input);
            let energy: f32 = output.iter().map(|s| s * s).sum();
            output.iter().enumerate().map(|(i, s)| i as f32 * s * s).sum::<f32>() / energy
        };
        assert!(delay(2) < delay(MAX_FACTOR), "{} against {}", delay(2), delay(MAX_FACTOR));
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
        let mut high = [0.0; 64 * MAX_FACTOR];
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
        let mut high = [0.0; 64 * MAX_FACTOR];
        let mut low = [0.0; 64];
        oversampler.upsample(&[1.0; 64], &mut high);
        oversampler.downsample(&high, &mut low);
        oversampler.reset();

        oversampler.upsample(&[0.0; 64], &mut high);
        oversampler.downsample(&high, &mut low);
        assert!(low.iter().all(|s| s.abs() < 1e-15));
    }
}
