use crate::dsp::filters::ANTI_DENORMAL;
use crate::dsp::{db_to_gain, smoothing_coeff};

// From closed to fully open. Short enough to keep the pick attack, long enough not to click
const ATTACK_MS: f32 = 0.5;

// How long the gate stays open after the level has fallen below where it closes
const HOLD_MS: f32 = 40.0;

// The gate closes this far below the level that opens it, so a note that fades through the
// threshold does not open and close it over and over
const HYSTERESIS_DB: f32 = 6.0;

// How fast the measured level falls between the peaks of a waveform. Slow enough to bridge
// the half periods of the lowest notes
const DETECTOR_RELEASE_MS: f32 = 10.0;

// The release time is the time the gain takes to fall this far. From there it is zero
const FLOOR_DB: f32 = -80.0;

/// Noise gate. Listens to one signal (the plugin's input) and turns another one down (the
/// same input after the input gain), so the threshold means dBFS at the plugin's input.
/// No lookahead: it adds no latency
pub struct Gate {
    sample_rate: f32,
    on: bool,
    open_level: f32,
    close_level: f32,
    attack_step: f32,
    // The gain is multiplied by this per sample while the gate closes
    release_factor: f32,
    release_ms: f32,
    hold_len: u32,

    // Peak level of the detector signal
    level: f32,
    level_release: f32,
    open: bool,
    hold: u32,
    gain: f32,
}

impl Gate {
    pub fn new() -> Self {
        let mut gate = Self {
            sample_rate: 44100.0,
            on: true,
            open_level: 0.0,
            close_level: 0.0,
            attack_step: 1.0,
            release_factor: 0.0,
            release_ms: f32::NAN,
            hold_len: 0,
            level: 0.0,
            level_release: 1.0,
            open: false,
            hold: 0,
            gain: 0.0,
        };
        gate.set_sample_rate(44100.0);
        gate.set(true, -60.0, 100.0);
        gate
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.attack_step = 1.0 / (ATTACK_MS * 0.001 * sample_rate).max(1.0);
        self.hold_len = (HOLD_MS * 0.001 * sample_rate).round() as u32;
        self.level_release = smoothing_coeff(DETECTOR_RELEASE_MS, sample_rate);
        let release_ms = self.release_ms;
        self.release_ms = f32::NAN;
        self.set_release(release_ms);
        self.reset();
    }

    /// Closed, as after a long silence
    pub fn reset(&mut self) {
        self.level = 0.0;
        self.open = false;
        self.hold = 0;
        self.gain = 0.0;
    }

    /// `threshold_db` is the level in dBFS that opens the gate; `release_ms` the time it
    /// takes to close. Switched off, the gate opens and stays open
    pub fn set(&mut self, on: bool, threshold_db: f32, release_ms: f32) {
        self.on = on;
        self.open_level = db_to_gain(threshold_db);
        self.close_level = db_to_gain(threshold_db - HYSTERESIS_DB);
        self.set_release(release_ms);
    }

    fn set_release(&mut self, release_ms: f32) {
        if release_ms != self.release_ms && release_ms.is_finite() {
            self.release_ms = release_ms;
            let samples = (release_ms * 0.001 * self.sample_rate).max(1.0);
            self.release_factor = db_to_gain(FLOOR_DB / samples);
        }
    }

    /// Fully open at once if the gate is switched off: for the start of playing, when
    /// there is nothing to fade from
    pub fn snap(&mut self) {
        if !self.on {
            self.open = true;
            self.hold = self.hold_len;
            self.gain = 1.0;
        }
    }

    /// The gain the gate is at, 0.0 closed to 1.0 open
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Follows `detector` and turns `signal` down. Both the same length
    pub fn process(&mut self, detector: &[f32], signal: &mut [f32]) {
        debug_assert_eq!(detector.len(), signal.len());
        if !self.on && self.gain == 1.0 {
            // Nothing to do, and switching it on finds it open with the whole hold ahead
            self.open = true;
            self.hold = self.hold_len;
            self.level = 0.0;
            return;
        }

        let floor = db_to_gain(FLOOR_DB);
        for (&heard, sample) in detector.iter().zip(signal.iter_mut()) {
            let heard = heard.abs();
            if heard > self.level {
                self.level = heard;
            } else {
                self.level += self.level_release * (heard - self.level) + ANTI_DENORMAL;
            }

            if !self.on || self.level >= self.open_level || (self.open && self.level >= self.close_level) {
                self.open = true;
                self.hold = self.hold_len;
            } else if self.hold > 0 {
                self.hold -= 1;
            } else {
                self.open = false;
            }

            if self.open {
                self.gain = (self.gain + self.attack_step).min(1.0);
            } else if self.gain > 0.0 {
                self.gain *= self.release_factor;
                if self.gain < floor {
                    self.gain = 0.0;
                }
            }
            *sample *= self.gain;
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::test_util::{peak, pluck, set_rms_db, sine, Noise, Pluck};

    const SAMPLE_RATE: f32 = 48000.0;

    fn new_gate(threshold_db: f32, release_ms: f32) -> Gate {
        let mut gate = Gate::new();
        gate.set_sample_rate(SAMPLE_RATE);
        gate.set(true, threshold_db, release_ms);
        gate
    }

    /// The gate's gain for every sample of `input`
    pub fn gain_trace(gate: &mut Gate, input: &[f32]) -> Vec<f32> {
        input
            .iter()
            .map(|&sample| {
                let mut one = [1.0];
                gate.process(&[sample], &mut one);
                one[0]
            })
            .collect()
    }

    /// How often the gate went from fully open to closing, or back to fully open
    pub fn transitions(trace: &[f32]) -> usize {
        let mut open = false;
        let mut count = 0;
        for &gain in trace {
            if open && gain < 1.0 {
                open = false;
                count += 1;
            } else if !open && gain == 1.0 {
                open = true;
                count += 1;
            }
        }
        count
    }

    /// One low string left to ring, peaking at -12 dBFS
    pub fn decaying_note(sample_rate: f32, seconds: f32) -> Vec<f32> {
        let mut note = pluck(82.41, Pluck::OPEN, 5, sample_rate, (seconds * sample_rate) as usize);
        let top = peak(&note);
        for sample in &mut note {
            *sample *= 0.25 / top;
        }
        note
    }

    /// White noise at a level in dBFS RMS
    pub fn hiss(level_db: f32, len: usize) -> Vec<f32> {
        let mut noise = Noise::new(21);
        let mut samples: Vec<f32> = (0..len).map(|_| noise.next()).collect();
        set_rms_db(&mut samples, level_db);
        samples
    }

    #[test]
    fn test_opens_fast_on_a_note_and_closes_after_it() {
        let mut gate = new_gate(-50.0, 100.0);
        let mut input = vec![0.0; 4800];
        input.extend(sine(110.0, 0.1, SAMPLE_RATE, 24_000));
        input.extend(vec![0.0; 24_000]);
        let trace = gain_trace(&mut gate, &input);

        assert!(trace[..4800].iter().all(|&gain| gain == 0.0));
        // Open within a millisecond of the level crossing the threshold
        let crossing = input.iter().position(|s| s.abs() >= db_to_gain(-50.0)).unwrap();
        let opened = trace.iter().position(|&gain| gain == 1.0).unwrap();
        assert!(opened - crossing <= (0.001 * SAMPLE_RATE) as usize, "Opened after {} samples", opened - crossing);
        assert!(trace[opened..28_800].iter().all(|&gain| gain == 1.0));

        // Held, then closed all the way within the hold, the detector's fall and the release
        let end = 28_800;
        assert_eq!(trace[end + (0.03 * SAMPLE_RATE) as usize], 1.0);
        let closed = trace[end..].iter().position(|&gain| gain == 0.0).unwrap();
        let limit = ((HOLD_MS + 100.0 + 8.0 * DETECTOR_RELEASE_MS) * 0.001 * SAMPLE_RATE) as usize;
        assert!(closed <= limit, "Closed after {} samples", closed);
        assert!(trace[end + closed..].iter().all(|&gain| gain == 0.0));
        assert_eq!(transitions(&trace), 2);
    }

    #[test]
    fn test_release_time_is_the_time_to_close() {
        let closing_ms = |release_ms: f32| {
            let mut gate = new_gate(-50.0, release_ms);
            let mut input = sine(110.0, 0.1, SAMPLE_RATE, 9600);
            input.extend(vec![0.0; 48_000]);
            let trace = gain_trace(&mut gate, &input);
            let start = trace[9600..].iter().position(|&gain| gain < 1.0).unwrap();
            let end = trace[9600..].iter().position(|&gain| gain == 0.0).unwrap();
            (end - start) as f32 / SAMPLE_RATE * 1000.0
        };
        for release_ms in [20.0, 100.0, 500.0] {
            let measured = closing_ms(release_ms);
            assert!((measured - release_ms).abs() < 0.03 * release_ms + 0.1, "{} ms: {} ms", release_ms, measured);
        }
    }

    #[test]
    fn test_does_not_chatter_on_a_decaying_note() {
        let note = decaying_note(SAMPLE_RATE, 6.0);
        for threshold_db in [-60.0, -50.0, -40.0, -30.0] {
            for release_ms in [20.0, 100.0, 500.0] {
                let trace = gain_trace(&mut new_gate(threshold_db, release_ms), &note);
                assert!(
                    transitions(&trace) <= 2,
                    "{} dB, {} ms: {} transitions",
                    threshold_db,
                    release_ms,
                    transitions(&trace)
                );
                assert_eq!(trace[2400], 1.0);
            }
        }
    }

    #[test]
    fn test_closes_below_where_it_opens() {
        // A level between the two: not enough to open the gate, enough to keep it open
        let between = db_to_gain(-50.0 - 0.5 * HYSTERESIS_DB);
        let quiet = sine(440.0, between, SAMPLE_RATE, 24_000);
        let trace = gain_trace(&mut new_gate(-50.0, 100.0), &quiet);
        assert!(trace.iter().all(|&gain| gain == 0.0));

        let mut gate = new_gate(-50.0, 100.0);
        gain_trace(&mut gate, &sine(440.0, 0.1, SAMPLE_RATE, 4800));
        let trace = gain_trace(&mut gate, &quiet);
        assert!(trace.iter().all(|&gain| gain == 1.0));
    }

    #[test]
    fn test_hiss_below_the_threshold_stays_out() {
        let noise = hiss(-70.0, 48_000);
        let trace = gain_trace(&mut new_gate(-60.0, 100.0), &noise);
        assert!(trace.iter().all(|&gain| gain == 0.0));
    }

    #[test]
    fn test_switched_off_it_is_open_and_exact() {
        let mut gate = new_gate(-20.0, 100.0);
        gate.set(false, -20.0, 100.0);
        gate.snap();
        let input = hiss(-70.0, 4800);
        let mut output = input.clone();
        gate.process(&input, &mut output);
        assert_eq!(input, output);

        // Switched on under the threshold it closes smoothly, and off again it opens fast
        gate.set(true, -20.0, 20.0);
        let trace = gain_trace(&mut gate, &input);
        assert_eq!(trace[0], 1.0);
        assert!(trace.windows(2).all(|pair| pair[1] <= pair[0] && pair[0] - pair[1] < 0.02));
        assert_eq!(*trace.last().unwrap(), 0.0);

        gate.set(false, -20.0, 20.0);
        let trace = gain_trace(&mut gate, &input);
        assert!(trace.windows(2).all(|pair| pair[1] - pair[0] < 0.05));
        assert_eq!(trace[(0.001 * SAMPLE_RATE) as usize], 1.0);
    }

    #[test]
    fn test_level_does_not_decay_into_denormals() {
        let mut gate = new_gate(-60.0, 100.0);
        gain_trace(&mut gate, &sine(110.0, 0.5, SAMPLE_RATE, 4800));
        gain_trace(&mut gate, &vec![0.0; 480_000]);
        assert!(gate.level == 0.0 || gate.level.is_normal(), "Level: {:e}", gate.level);
        assert_eq!(gate.gain(), 0.0);
    }
}
