use super::model::ToneModel;
use crate::dsp::db_to_gain;
use crate::dsp::filters::{BiquadCoeffs, GlidingBiquad};

// Gentle slopes, as from a network of a few resistors and capacitors
const SHELF_Q: f32 = 0.6;

/// The three filters the dials come to, as a passive tone network behaves: the mids sink
/// when Bass and Treble are both up, and the mid frequency follows the other two dials
pub struct ToneCurve {
    pub bass: BiquadCoeffs,
    pub mid: BiquadCoeffs,
    pub treble: BiquadCoeffs,
}

impl ToneCurve {
    /// Dials are 0.0 to 1.0
    pub fn new(model: &ToneModel, bass: f32, mid: f32, treble: f32, sample_rate: f32) -> Self {
        // -1.0 to 1.0 around the dial centre
        let (bass, mid, treble) = (bass * 2.0 - 1.0, mid * 2.0 - 1.0, treble * 2.0 - 1.0);

        let mid_db = model.mid_centre_db + model.mid_db * mid - model.scoop_db * 0.5 * (bass + treble);
        let mid_hz = model.mid_hz * 2.0f32.powf(model.mid_shift_octaves * 0.5 * (bass - treble));
        let level = db_to_gain(model.mid_level_db * mid);

        Self {
            bass: BiquadCoeffs::low_shelf(model.bass_hz, SHELF_Q, model.bass_db * bass, sample_rate).scaled(level),
            mid: BiquadCoeffs::peak(mid_hz, model.mid_q, mid_db, sample_rate),
            treble: BiquadCoeffs::high_shelf(model.treble_hz, SHELF_Q, model.treble_db * treble, sample_rate),
        }
    }

    #[cfg(test)]
    pub fn magnitude_db(&self, freq_hz: f32, sample_rate: f32) -> f32 {
        self.bass.magnitude_db(freq_hz, sample_rate)
            + self.mid.magnitude_db(freq_hz, sample_rate)
            + self.treble.magnitude_db(freq_hz, sample_rate)
    }
}

/// Bass, Mid and Treble, between the preamp and the power amp
pub struct ToneStack {
    bass: GlidingBiquad,
    mid: GlidingBiquad,
    treble: GlidingBiquad,
}

impl ToneStack {
    pub fn new() -> Self {
        Self {
            bass: GlidingBiquad::new(),
            mid: GlidingBiquad::new(),
            treble: GlidingBiquad::new(),
        }
    }

    /// Moves to a curve over the next `steps` samples; at once with no steps
    pub fn set(&mut self, curve: &ToneCurve, steps: u32) {
        self.bass.set(curve.bass, steps);
        self.mid.set(curve.mid, steps);
        self.treble.set(curve.treble, steps);
    }

    pub fn reset(&mut self) {
        self.bass.reset();
        self.mid.reset();
        self.treble.reset();
    }

    pub fn process(&mut self, block: &mut [f32]) {
        for sample in block.iter_mut() {
            let signal = self.bass.process(*sample as f64);
            let signal = self.mid.process(signal);
            *sample = self.treble.process(signal) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amp::model::Amp;
    use crate::test_util::{rms, sine, to_db};

    const SAMPLE_RATE: f32 = 192_000.0;
    const LOW_HZ: f32 = 80.0;
    const HIGH_HZ: f32 = 6000.0;

    fn response(amp: Amp, bass: f32, mid: f32, treble: f32, freq_hz: f32) -> f32 {
        let model = amp.model();
        ToneCurve::new(&model.tone, bass, mid, treble, SAMPLE_RATE).magnitude_db(freq_hz, SAMPLE_RATE)
    }

    #[test]
    fn test_each_dial_moves_its_own_band_most() {
        for amp in Amp::ALL {
            let mid_hz = amp.model().tone.mid_hz;
            let change = |low: [f32; 3], high: [f32; 3], freq: f32| {
                response(amp, high[0], high[1], high[2], freq) - response(amp, low[0], low[1], low[2], freq)
            };

            let bass = |freq| change([0.0, 0.5, 0.5], [1.0, 0.5, 0.5], freq);
            assert!(bass(LOW_HZ) > 8.0, "{:?} bass at {} Hz: {:.1} dB", amp, LOW_HZ, bass(LOW_HZ));
            assert!(bass(LOW_HZ) > bass(HIGH_HZ) + 8.0);

            let mid = |freq| change([0.5, 0.0, 0.5], [0.5, 1.0, 0.5], freq);
            assert!(mid(mid_hz) > 8.0, "{:?} mid at {} Hz: {:.1} dB", amp, mid_hz, mid(mid_hz));
            assert!(mid(mid_hz) > mid(LOW_HZ) + 4.0 && mid(mid_hz) > mid(HIGH_HZ) + 4.0);

            let treble = |freq| change([0.5, 0.5, 0.0], [0.5, 0.5, 1.0], freq);
            assert!(treble(HIGH_HZ) > 8.0, "{:?} treble at {} Hz: {:.1} dB", amp, HIGH_HZ, treble(HIGH_HZ));
            assert!(treble(HIGH_HZ) > treble(LOW_HZ) + 8.0);
        }
    }

    #[test]
    fn test_dials_interact_like_one_network() {
        for amp in Amp::ALL {
            let mid_hz = amp.model().tone.mid_hz;
            // The same Mid setting sits lower between a raised Bass and Treble: the mids
            // against the average of the two ends
            let mids = |bass: f32, treble: f32| {
                let ends = response(amp, bass, 0.5, treble, LOW_HZ) + response(amp, bass, 0.5, treble, HIGH_HZ);
                response(amp, bass, 0.5, treble, mid_hz) - 0.5 * ends
            };
            let (both_up, centre) = (mids(1.0, 1.0), mids(0.5, 0.5));
            assert!(both_up < centre - 8.0, "{:?}: {:.1} dB against {:.1} dB", amp, both_up, centre);

            // Mid also lifts the level outside its band a little
            assert!(response(amp, 0.5, 1.0, 0.5, 30.0) > response(amp, 0.5, 0.0, 0.5, 30.0));
        }
    }

    #[test]
    fn test_running_filters_match_the_curve() {
        let amp = Amp::ALL[0];
        let curve = ToneCurve::new(&amp.model().tone, 0.8, 0.3, 0.6, SAMPLE_RATE);
        for freq in [80.0, 650.0, 6000.0] {
            let mut stack = ToneStack::new();
            stack.set(&curve, 0);
            let mut block = sine(freq, 0.5, SAMPLE_RATE, 48_000);
            let input_rms = rms(&block[24_000..]);
            stack.process(&mut block);
            let measured = to_db(rms(&block[24_000..]) / input_rms);
            assert!((measured - curve.magnitude_db(freq, SAMPLE_RATE)).abs() < 0.2, "{} Hz", freq);
        }
    }
}
