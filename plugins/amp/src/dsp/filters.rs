use std::f64::consts::TAU;

/// Added inside every feedback path. Without it the states decay into denormal numbers
/// after the input goes silent, and those are many times slower to compute with
pub const ANTI_DENORMAL: f32 = 1e-20;

// Filter frequencies are kept below this fraction of the sample rate
const MAX_FREQ_RATIO: f64 = 0.45;

fn clamp_freq(freq_hz: f32, sample_rate: f32) -> f64 {
    (freq_hz as f64).clamp(1.0, sample_rate as f64 * MAX_FREQ_RATIO)
}

/// Second-order filter coefficients, normalised (a0 = 1). The designs are the usual audio
/// cookbook ones
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl BiquadCoeffs {
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalised(b: [f64; 3], a: [f64; 3]) -> Self {
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
        }
    }

    /// cos(w0) and alpha for a frequency and Q
    fn prototype(freq_hz: f32, q: f32, sample_rate: f32) -> (f64, f64) {
        let (sin, cos) = (TAU * clamp_freq(freq_hz, sample_rate) / sample_rate as f64).sin_cos();
        (cos, sin / (2.0 * (q as f64).max(0.05)))
    }

    pub fn lowpass(freq_hz: f32, q: f32, sample_rate: f32) -> Self {
        let (cos, alpha) = Self::prototype(freq_hz, q, sample_rate);
        Self::normalised(
            [(1.0 - cos) * 0.5, 1.0 - cos, (1.0 - cos) * 0.5],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        )
    }

    pub fn highpass(freq_hz: f32, q: f32, sample_rate: f32) -> Self {
        let (cos, alpha) = Self::prototype(freq_hz, q, sample_rate);
        Self::normalised(
            [(1.0 + cos) * 0.5, -(1.0 + cos), (1.0 + cos) * 0.5],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        )
    }

    /// Bell: `gain_db` at the centre, flat away from it
    pub fn peak(freq_hz: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (cos, alpha) = Self::prototype(freq_hz, q, sample_rate);
        let a = 10.0f64.powf(gain_db as f64 / 40.0);
        Self::normalised(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        )
    }

    /// `gain_db` below the corner, flat above it
    pub fn low_shelf(freq_hz: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (cos, alpha) = Self::prototype(freq_hz, q, sample_rate);
        let a = 10.0f64.powf(gain_db as f64 / 40.0);
        let slope = 2.0 * a.sqrt() * alpha;
        Self::normalised(
            [
                a * ((a + 1.0) - (a - 1.0) * cos + slope),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - slope),
            ],
            [
                (a + 1.0) + (a - 1.0) * cos + slope,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - slope,
            ],
        )
    }

    /// `gain_db` above the corner, flat below it
    pub fn high_shelf(freq_hz: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (cos, alpha) = Self::prototype(freq_hz, q, sample_rate);
        let a = 10.0f64.powf(gain_db as f64 / 40.0);
        let slope = 2.0 * a.sqrt() * alpha;
        Self::normalised(
            [
                a * ((a + 1.0) + (a - 1.0) * cos + slope),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - slope),
            ],
            [
                (a + 1.0) - (a - 1.0) * cos + slope,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - slope,
            ],
        )
    }

    /// The same filter with its output multiplied by `gain`
    pub fn scaled(self, gain: f32) -> Self {
        let gain = gain as f64;
        Self {
            b0: self.b0 * gain,
            b1: self.b1 * gain,
            b2: self.b2 * gain,
            ..self
        }
    }

    /// Gain of the filter at one frequency
    pub fn magnitude_db(&self, freq_hz: f32, sample_rate: f32) -> f32 {
        let (sin1, cos1) = (TAU * freq_hz as f64 / sample_rate as f64).sin_cos();
        let (sin2, cos2) = (2.0 * TAU * freq_hz as f64 / sample_rate as f64).sin_cos();
        let num_re = self.b0 + self.b1 * cos1 + self.b2 * cos2;
        let num_im = self.b1 * sin1 + self.b2 * sin2;
        let den_re = 1.0 + self.a1 * cos1 + self.a2 * cos2;
        let den_im = self.a1 * sin1 + self.a2 * sin2;
        let power = (num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im);
        (10.0 * power.max(1e-30).log10()) as f32
    }
}

/// Second-order filter, transposed direct form II. Runs in double precision: at four times
/// 192 kHz a 100 Hz filter has its poles so close to 1 that single precision moves them
#[derive(Clone, Copy)]
pub struct Biquad {
    coeffs: BiquadCoeffs,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn new() -> Self {
        Self {
            coeffs: BiquadCoeffs::IDENTITY,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn set(&mut self, coeffs: BiquadCoeffs) {
        self.coeffs = coeffs;
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    pub fn process(&mut self, input: f64) -> f64 {
        let output = self.coeffs.b0 * input + self.z1;
        self.z1 = self.coeffs.b1 * input - self.coeffs.a1 * output + self.z2 + ANTI_DENORMAL as f64;
        self.z2 = self.coeffs.b2 * input - self.coeffs.a2 * output;
        output
    }
}

/// One-pole lowpass (6 dB per octave)
#[derive(Clone, Copy)]
pub struct OnePoleLp {
    coeff: f32,
    state: f32,
}

impl OnePoleLp {
    pub fn new() -> Self {
        Self { coeff: 1.0, state: 0.0 }
    }

    pub fn set(&mut self, freq_hz: f32, sample_rate: f32) {
        self.coeff = (1.0 - (-TAU * clamp_freq(freq_hz, sample_rate) / sample_rate as f64).exp()) as f32;
    }

    pub fn reset(&mut self) {
        self.state = 0.0;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.state += self.coeff * (input - self.state) + ANTI_DENORMAL;
        self.state
    }
}

/// One-pole highpass (6 dB per octave): the input minus its lowpass
#[derive(Clone, Copy)]
pub struct OnePoleHp {
    lowpass: OnePoleLp,
}

impl OnePoleHp {
    pub fn new() -> Self {
        Self { lowpass: OnePoleLp::new() }
    }

    pub fn set(&mut self, freq_hz: f32, sample_rate: f32) {
        self.lowpass.set(freq_hz, sample_rate);
    }

    pub fn reset(&mut self) {
        self.lowpass.reset();
    }

    pub fn process(&mut self, input: f32) -> f32 {
        input - self.lowpass.process(input)
    }
}

// Low enough to leave a guitar's lowest notes alone, also in drop tunings
const DC_BLOCK_HZ: f32 = 8.0;

/// Removes the constant offset that asymmetric clipping leaves behind
#[derive(Clone, Copy)]
pub struct DcBlocker {
    pole: f32,
    last_input: f32,
    last_output: f32,
}

impl DcBlocker {
    pub fn new() -> Self {
        Self {
            pole: 0.0,
            last_input: 0.0,
            last_output: 0.0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.pole = (-TAU * DC_BLOCK_HZ as f64 / sample_rate as f64).exp() as f32;
    }

    pub fn reset(&mut self) {
        self.last_input = 0.0;
        self.last_output = 0.0;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.last_output = input - self.last_input + self.pole * self.last_output + ANTI_DENORMAL;
        self.last_input = input;
        self.last_output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{rms, sine, to_db};

    const SAMPLE_RATE: f32 = 48000.0;

    /// Steady-state gain of a filter for a sine, in dB
    fn measured_db(mut filter: impl FnMut(f32) -> f32, freq_hz: f32, sample_rate: f32) -> f32 {
        let len = (sample_rate * 0.5) as usize;
        let output: Vec<f32> = sine(freq_hz, 1.0, sample_rate, len).iter().map(|&s| filter(s)).collect();
        to_db(rms(&output[len / 2..]) * std::f32::consts::SQRT_2)
    }

    fn biquad_db(coeffs: BiquadCoeffs, freq_hz: f32, sample_rate: f32) -> f32 {
        let mut biquad = Biquad::new();
        biquad.set(coeffs);
        measured_db(|s| biquad.process(s as f64) as f32, freq_hz, sample_rate)
    }

    #[test]
    fn test_lowpass_and_highpass_pass_and_stop() {
        let lowpass = BiquadCoeffs::lowpass(1000.0, 0.707, SAMPLE_RATE);
        assert!(biquad_db(lowpass, 100.0, SAMPLE_RATE).abs() < 0.1);
        assert!((biquad_db(lowpass, 1000.0, SAMPLE_RATE) + 3.0).abs() < 0.2);
        assert!(biquad_db(lowpass, 8000.0, SAMPLE_RATE) < -34.0);

        let highpass = BiquadCoeffs::highpass(1000.0, 0.707, SAMPLE_RATE);
        assert!(biquad_db(highpass, 8000.0, SAMPLE_RATE).abs() < 0.2);
        assert!((biquad_db(highpass, 1000.0, SAMPLE_RATE) + 3.0).abs() < 0.2);
        assert!(biquad_db(highpass, 100.0, SAMPLE_RATE) < -38.0);
    }

    #[test]
    fn test_peak_and_shelves_have_their_gain_where_they_should() {
        let peak = BiquadCoeffs::peak(1000.0, 1.0, 6.0, SAMPLE_RATE);
        assert!((biquad_db(peak, 1000.0, SAMPLE_RATE) - 6.0).abs() < 0.1);
        assert!(biquad_db(peak, 60.0, SAMPLE_RATE).abs() < 0.2);
        assert!(biquad_db(peak, 15000.0, SAMPLE_RATE).abs() < 0.2);

        let low_shelf = BiquadCoeffs::low_shelf(300.0, 0.707, -9.0, SAMPLE_RATE);
        assert!((biquad_db(low_shelf, 30.0, SAMPLE_RATE) + 9.0).abs() < 0.3);
        assert!(biquad_db(low_shelf, 6000.0, SAMPLE_RATE).abs() < 0.2);

        let high_shelf = BiquadCoeffs::high_shelf(3000.0, 0.707, 9.0, SAMPLE_RATE);
        assert!((biquad_db(high_shelf, 18000.0, SAMPLE_RATE) - 9.0).abs() < 0.3);
        assert!(biquad_db(high_shelf, 200.0, SAMPLE_RATE).abs() < 0.2);
    }

    #[test]
    fn test_magnitude_matches_the_running_filter() {
        let coeffs = BiquadCoeffs::peak(700.0, 2.0, -8.0, SAMPLE_RATE).scaled(2.0);
        for freq in [100.0, 500.0, 700.0, 1500.0, 9000.0] {
            let measured = biquad_db(coeffs, freq, SAMPLE_RATE);
            let computed = coeffs.magnitude_db(freq, SAMPLE_RATE);
            assert!((measured - computed).abs() < 0.1, "{} Hz: {} vs {}", freq, measured, computed);
        }
    }

    #[test]
    fn test_low_filters_keep_their_frequency_at_high_sample_rates() {
        // Four times 192 kHz, where single precision coefficients would be off
        let sample_rate = 768_000.0;
        let peak = BiquadCoeffs::peak(100.0, 2.0, 6.0, sample_rate);
        assert!((biquad_db(peak, 100.0, sample_rate) - 6.0).abs() < 0.2);
        assert!(biquad_db(peak, 400.0, sample_rate) < 1.0);
    }

    #[test]
    fn test_frequencies_above_nyquist_stay_stable() {
        let mut biquad = Biquad::new();
        biquad.set(BiquadCoeffs::lowpass(30000.0, 0.707, 44100.0));
        let mut lowpass = OnePoleLp::new();
        lowpass.set(90000.0, 44100.0);

        let mut seed = 1u32;
        for _ in 0..10_000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed >> 9) as f32 / (1 << 22) as f32 - 1.0;
            assert!(biquad.process(noise as f64).abs() < 10.0);
            assert!(lowpass.process(noise).abs() <= 1.0);
        }
    }

    #[test]
    fn test_one_pole_corners_are_3_db_down() {
        let mut lowpass = OnePoleLp::new();
        lowpass.set(500.0, SAMPLE_RATE);
        assert!((measured_db(|s| lowpass.process(s), 500.0, SAMPLE_RATE) + 3.0).abs() < 0.3);
        lowpass.reset();
        assert!(measured_db(|s| lowpass.process(s), 20.0, SAMPLE_RATE).abs() < 0.1);

        let mut highpass = OnePoleHp::new();
        highpass.set(500.0, SAMPLE_RATE);
        assert!((measured_db(|s| highpass.process(s), 500.0, SAMPLE_RATE) + 3.0).abs() < 0.3);
        highpass.reset();
        assert!(measured_db(|s| highpass.process(s), 50.0, SAMPLE_RATE) < -19.0);
    }

    #[test]
    fn test_states_never_decay_into_denormal_numbers() {
        // A burst, then silence for long enough to fall through the whole range of an f32
        // (and, for the fast biquad, of an f64)
        let input = |index: usize| if index < 100 { 1.0 } else { 0.0 };
        let healthy = |value: f64| value == 0.0 || value.is_normal();

        let mut lowpass = OnePoleLp::new();
        lowpass.set(2000.0, SAMPLE_RATE);
        let mut highpass = OnePoleHp::new();
        highpass.set(2000.0, SAMPLE_RATE);
        let mut blocker = DcBlocker::new();
        blocker.set_sample_rate(1000.0);
        let mut biquad = Biquad::new();
        biquad.set(BiquadCoeffs::lowpass(8000.0, 0.7, SAMPLE_RATE));

        for index in 0..20_000 {
            let sample = input(index);
            assert!(healthy(lowpass.process(sample) as f64) && healthy(lowpass.state as f64));
            highpass.process(sample);
            assert!(healthy(highpass.lowpass.state as f64));
            assert!(healthy(blocker.process(sample) as f64));
            biquad.process(sample as f64);
            assert!(healthy(biquad.z1) && healthy(biquad.z2), "Biquad at sample {}", index);
        }
        assert!(lowpass.state.abs() < 1e-15 && biquad.z1.abs() < 1e-15);
    }

    #[test]
    fn test_dc_blocker_removes_offset_and_keeps_low_e() {
        let mut blocker = DcBlocker::new();
        blocker.set_sample_rate(SAMPLE_RATE);
        let mut last = 1.0;
        for _ in 0..SAMPLE_RATE as usize {
            last = blocker.process(1.0);
        }
        assert!(last.abs() < 1e-4, "Offset left: {}", last);

        blocker.reset();
        assert!(measured_db(|s| blocker.process(s), 82.41, SAMPLE_RATE).abs() < 0.1);
    }
}
