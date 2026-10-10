use super::model::PowerModel;
use crate::dsp::filters::{Biquad, BiquadCoeffs, DcBlocker, ANTI_DENORMAL};
use crate::dsp::shaper::PowerClipper;
use crate::dsp::{curve, db_to_gain, smoothing_coeff, Ramp};

const RESONANCE_Q: f32 = 1.2;
const PRESENCE_Q: f32 = 0.6;

/// The power stage: Master drives a symmetric clipper whose headroom sags under sustained
/// load. Presence and the low resonance sit in front of the clipper, as they do in a stage
/// with feedback: they shape the sound while it is clean and flatten out as it clips
pub struct PowerAmp {
    dc: DcBlocker,
    resonance: Biquad,
    presence: Biquad,
    drive: Ramp,
    volume: Ramp,
    clipper: PowerClipper,
    sag: f32,
    // Follows the output level, 0.0 to 1.0
    load: f32,
    attack: f32,
    release: f32,
}

impl PowerAmp {
    pub fn new() -> Self {
        Self {
            dc: DcBlocker::new(),
            resonance: Biquad::new(),
            presence: Biquad::new(),
            drive: Ramp::new(1.0),
            volume: Ramp::new(1.0),
            clipper: PowerClipper::new(),
            sag: 0.0,
            load: 0.0,
            attack: 1.0,
            release: 1.0,
        }
    }

    /// `sample_rate` is the rate the power amp runs at (the oversampled one)
    pub fn configure(&mut self, model: &PowerModel, sample_rate: f32) {
        self.dc.set_sample_rate(sample_rate);
        self.resonance.set(BiquadCoeffs::peak(
            model.resonance_hz,
            RESONANCE_Q,
            model.resonance_db,
            sample_rate,
        ));
        self.sag = model.sag;
        self.attack = smoothing_coeff(model.sag_attack_ms, sample_rate);
        self.release = smoothing_coeff(model.sag_release_ms, sample_rate);
    }

    pub fn reset(&mut self) {
        self.dc.reset();
        self.resonance.reset();
        self.presence.reset();
        self.clipper.reset();
        self.load = 0.0;
    }

    /// Moves to a Master dial position over the next `steps` samples. `makeup_db` is added
    /// to the level after the stage
    pub fn set_master(&mut self, model: &PowerModel, master: f32, makeup_db: f32, steps: u32) {
        self.drive.set_target(db_to_gain(curve(&model.drive_db, master)), steps);
        self.volume.set_target(db_to_gain(curve(&model.volume_db, master) + makeup_db), steps);
    }

    pub fn set_presence(&mut self, model: &PowerModel, presence: f32, sample_rate: f32) {
        self.presence.set(BiquadCoeffs::high_shelf(
            model.presence_hz,
            PRESENCE_Q,
            model.presence_db * presence,
            sample_rate,
        ));
    }

    /// Ends the move started by `set_master` at once
    pub fn snap(&mut self) {
        self.drive.snap();
        self.volume.snap();
    }

    pub fn process(&mut self, block: &mut [f32]) {
        for sample in block.iter_mut() {
            let input = self.dc.process(*sample) as f64;
            let shaped = self.presence.process(self.resonance.process(input)) as f32 * self.drive.next();

            let headroom = 1.0 - self.sag * self.load;
            let output = headroom * self.clipper.process(shaped / headroom);

            let level = output.abs();
            let coeff = if level > self.load { self.attack } else { self.release };
            self.load += coeff * (level - self.load) + ANTI_DENORMAL;

            *sample = output * self.volume.next();
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

    fn run(amp: Amp, master: f32, presence: f32, input: &[f32]) -> Vec<f32> {
        let model = &amp.model().power;
        let mut power = PowerAmp::new();
        power.configure(model, SAMPLE_RATE);
        power.set_master(model, master, 0.0, 0);
        power.set_presence(model, presence, SAMPLE_RATE);
        let mut output = input.to_vec();
        power.process(&mut output);
        output
    }

    fn harmonics_db(output: &[f32], freq: f32) -> f32 {
        let partials: Vec<f64> = harmonics_of(freq, SAMPLE_RATE).into_iter().take(9).collect();
        let (levels, _) = fit_partials(&output[LEN / 2..], SAMPLE_RATE, &partials);
        let harmonics: f64 = levels[1..].iter().map(|level| level * level).sum();
        to_db((harmonics.sqrt() / levels[0]) as f32)
    }

    #[test]
    fn test_master_at_centre_is_mostly_clean_and_full_up_clips() {
        for amp in Amp::ALL {
            // About what the preamp delivers
            let input = sine(440.0, 0.7, SAMPLE_RATE, LEN);
            let centre = harmonics_db(&run(amp, 0.5, 0.5, &input), 440.0);
            let full = harmonics_db(&run(amp, 1.0, 0.5, &input), 440.0);
            assert!(centre < -30.0, "{:?} at master 5: {:.1} dB", amp, centre);
            assert!(full > -20.0, "{:?} at master 10: {:.1} dB", amp, full);
        }
    }

    #[test]
    fn test_master_sets_the_level() {
        for amp in Amp::ALL {
            let input = sine(440.0, 0.7, SAMPLE_RATE, LEN);
            let levels: Vec<f32> = [0.0, 0.5, 1.0]
                .iter()
                .map(|&master| to_db(rms(&run(amp, master, 0.5, &input)[LEN / 2..])))
                .collect();
            assert!(levels[1] > levels[0] + 10.0 && levels[2] > levels[1], "{:?}: {:?}", amp, levels);
        }
    }

    #[test]
    fn test_presence_lifts_the_highs_and_resonance_the_lows() {
        for amp in Amp::ALL {
            let model = &amp.model().power;
            let level = |presence: f32, freq: f32| {
                let input = sine(freq, 0.05, SAMPLE_RATE, LEN);
                to_db(rms(&run(amp, 0.5, presence, &input)[LEN / 2..]))
            };
            assert!(level(1.0, 8000.0) > level(0.0, 8000.0) + 0.6 * model.presence_db);
            assert!((level(1.0, 300.0) - level(0.0, 300.0)).abs() < 1.0);
            assert!(level(0.5, model.resonance_hz) > level(0.5, 500.0) + 0.6 * model.resonance_db);
        }
    }

    #[test]
    fn test_sag_lowers_sustained_peaks_and_recovers() {
        for amp in Amp::ALL {
            let model = &amp.model().power;
            let mut power = PowerAmp::new();
            power.configure(model, SAMPLE_RATE);
            power.set_master(model, 1.0, 0.0, 0);
            power.set_presence(model, 0.0, SAMPLE_RATE);

            let mut burst = sine(440.0, 2.0, SAMPLE_RATE, LEN);
            power.process(&mut burst);
            let early = peak(&burst[..960]);
            let late = peak(&burst[LEN / 2..]);
            assert!(late < early * (1.0 - 0.5 * model.sag), "{:?}: {} then {}", amp, early, late);

            let mut pause = vec![0.0; LEN];
            power.process(&mut pause);
            assert!(power.load < 0.01, "{:?} load after a pause: {}", amp, power.load);
        }
    }

    #[test]
    fn test_offset_is_blocked_and_output_bounded() {
        for amp in Amp::ALL {
            let model = &amp.model().power;
            let input: Vec<f32> = sine(440.0, 5.0, SAMPLE_RATE, LEN).iter().map(|s| s + 3.0).collect();
            let output = run(amp, 1.0, 1.0, &input);
            let limit = db_to_gain(model.volume_db[2]);
            assert!(output.iter().all(|s| s.abs() <= limit * 1.001));

            let settled = &output[LEN / 2..];
            let mean = settled.iter().sum::<f32>() / settled.len() as f32;
            assert!(mean.abs() < 0.02 * limit, "{:?} offset: {}", amp, mean);
        }
    }
}
