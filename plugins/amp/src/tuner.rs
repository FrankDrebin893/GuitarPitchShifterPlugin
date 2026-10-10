//! Tuner: hears the plugin's input and tells the editor which note it is and how far off.
//!
//! The input is low-passed and thinned out to about 5.5 kHz into a ring. Every 30 ms a
//! reading is worked out from the ring, a little with each new sample so no block pays for
//! all of it:
//!
//! 1. The period, roughly: the difference function with cumulative-mean normalisation,
//!    taking the shortest lag that is nearly as good as the best one (a longer lag that fits
//!    as well is a multiple of the period)
//! 2. The harmonics at that pitch are weighed. Nothing at the pitch itself and all of it at
//!    its multiples means the lag was a multiple of the period after all, and the pitch is
//!    moved up. Nothing at the pitch but something between the multiples is a chord: no
//!    reading. Something halfway between the harmonics means the note is an octave lower
//!    and its own first harmonic is weak, as on the low strings
//! 3. The pitch, exactly: how far the phase of the first three harmonics moves between
//!    windows a known time apart, done twice so the windows hold whole periods. Of three
//!    harmonics the one in the middle is taken, so hum next to one of them does no harm;
//!    of fewer, the loud ones count most. A weak fundamental does no harm either, and the
//!    stiff string's sharp upper harmonics are not asked
//! 4. The second time there are three windows. A steady note moves as far from the first to
//!    the second as from the second to the third. A pluck or the next note inside them
//!    does not, and gives no reading
//!
//! Nothing here allocates or waits, and nothing here touches the audio.

use crate::dsp::filters::{Biquad, BiquadCoeffs};
use std::f64::consts::TAU;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// The A above middle C that the notes are counted from. Fixed for now
pub const REFERENCE_A_HZ: f32 = 440.0;

// The rate the detector works at, about: the input rate divided by a whole number
const TARGET_RATE_HZ: f32 = 5500.0;

// In front of the thinning out: eighth-order low-pass. It passes the highest note and the
// first harmonics of the low ones, and is 57 dB down where the folding reaches them
const LOWPASS_HZ: f32 = 1550.0;
const LOWPASS_Q: [f32; 4] = [0.5098, 0.6013, 0.9000, 2.5629];
// Behind it: keeps offsets and rumble out of the difference function
const HIGHPASS_HZ: f32 = 30.0;

// Notes the tuner reads: from a little under the low A of a drop tuning to the top of the neck
const MIN_HZ: f32 = 48.0;
const MAX_HZ: f32 = 1420.0;

// The ring holds the longest windows and what is written while a reading is worked out
const RING: usize = 2048;
const RING_MASK: usize = RING - 1;
// Longest lag searched: the period of `MIN_HZ` at the highest rate the detector can run at,
// which is 1.5 times `TARGET_RATE_HZ`
const MAX_LAG: usize = 176;

// A new reading is started this often
const HOP_MS: f32 = 30.0;

// Input below this level in dBFS RMS is silence
const LEVEL_MIN_DB: f32 = -70.0;

// The normalised difference at the period: 0.0 for a signal that repeats exactly, around
// 1.0 for noise. Above this there is no note
const CLARITY_MAX: f32 = 0.15;
// The shortest lag within this of the best one is the period. Half the period fits this
// well only when the odd harmonics hold less than 2 % of the energy
const OCTAVE_MARGIN: f32 = 0.04;

// Harmonics weighed for the check of the pitch, and measured for the exact pitch
const MAX_HARMONICS: usize = 6;
const FINE_HARMONICS: usize = 3;
// Only harmonics below this share of the detector's rate: the low-pass passes them
const TOP_RATIO: f64 = 0.3;
// A harmonic with less than this share of the strongest one's power is not there
const SIGNIFICANT: f64 = 3e-4;
// This share of it halfway between the harmonics, and the note is an octave lower. Asked
// of what lies at one and a half times the pitch, the lower note's third harmonic. At half
// the pitch there may as well be mains hum under an A string, so there it takes ten times
// as much, unless the third harmonic is out of reach
const UNDER_SIGNIFICANT: f64 = 3e-3;
const UNDER_ALONE: f64 = 3e-2;

// The windows of the exact measurement hold this many periods, and more for high notes so
// they last at least this long. The first and the last window are half a window apart
const MIN_PERIODS: f64 = 4.0;
const MIN_WINDOW_MS: f64 = 20.0;

// A harmonic with at least this share of the strongest one's power takes part when the
// middle one of three is taken. With fewer, they are averaged by power
const MEDIAN_SHARE: f64 = 1e-2;

// What makes a measurement a reading: the two passes agree, the pitch is the same in both
// halves of the time measured, the harmonics agree on it, and the oldest window is not
// mostly the silence before the note
const PASSES_AGREE_CENTS: f64 = 8.0;
const STEADY_CENTS: f64 = 6.0;
const HARMONICS_AGREE_CENTS: f64 = 12.0;
const OLDER_POWER_MIN: f64 = 0.3;

// Work done per sample of the ring: one step of a difference costs 1, one of a harmonic 4
const BUDGET: usize = 640;
const PROBE_COST: usize = 4;

// What is shown is the median of the last readings of a note. A reading this far from it
// is shown only once the next one agrees with it, and after this many failed readings in a
// row nothing is shown
const SHOWN: usize = 3;
const NOTE_CHANGE_CENTS: f32 = 70.0;
const MISSES_TO_CLEAR: u32 = 3;

/// What the tuner hears now, for the editor to read. One atomic: nothing to lock, nothing
/// that can be read half written
pub struct TunerReading {
    // Bits of the frequency in Hz as f32. 0.0 for no note
    hz: AtomicU32,
}

impl TunerReading {
    pub fn new() -> Self {
        Self {
            hz: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    /// The pitch in Hz, or nothing: silence, noise, a chord, or the tuner is off
    pub fn hz(&self) -> Option<f32> {
        let hz = f32::from_bits(self.hz.load(Ordering::Relaxed));
        (hz > 0.0).then_some(hz)
    }

    pub fn set(&self, hz: Option<f32>) {
        self.hz.store(hz.unwrap_or(0.0).to_bits(), Ordering::Relaxed);
    }
}

/// The nearest note to a pitch and how far the pitch is from it
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    /// `C`, `C#` and so on
    pub name: &'static str,
    /// Middle C is C4, the low E string E2
    pub octave: i32,
    /// Below zero flat, above sharp, 50 at most
    pub cents: f32,
}

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

impl Note {
    pub fn nearest(hz: f32) -> Self {
        let semitones = 12.0 * (hz / REFERENCE_A_HZ).log2();
        let nearest = semitones.round();
        // Counted as MIDI does: the reference A is 69, and C starts an octave
        let number = 69 + nearest as i32;
        Self {
            name: NOTE_NAMES[number.rem_euclid(12) as usize],
            octave: number.div_euclid(12) - 1,
            cents: (semitones - nearest) * 100.0,
        }
    }
}

fn cents_between(hz: f64, reference_hz: f64) -> f64 {
    1200.0 * (hz / reference_hz).log2()
}

/// Sum of a stretch of the ring times a windowed turning pointer: how much of one frequency
/// is in there, and at which phase. Run a piece at a time
#[derive(Clone, Copy)]
struct Probe {
    position: usize,
    left: usize,
    // Cosine and sine of the pointer and of the raised-cosine window, and of their steps
    pointer: (f64, f64),
    pointer_step: (f64, f64),
    window: (f64, f64),
    window_step: (f64, f64),
    sum: (f64, f64),
}

impl Probe {
    const IDLE: Self = Self {
        position: 0,
        left: 0,
        pointer: (1.0, 0.0),
        pointer_step: (1.0, 0.0),
        window: (1.0, 0.0),
        window_step: (1.0, 0.0),
        sum: (0.0, 0.0),
    };

    /// `turn` is the frequency in radians per sample. The phase counts from `origin`, so two
    /// windows find a tone at exactly that frequency at the same phase
    fn begin(&mut self, start: usize, len: usize, turn: f64, origin: usize) {
        let offset = start.wrapping_sub(origin) as isize as f64;
        let (sin, cos) = (turn * offset).sin_cos();
        let (step_sin, step_cos) = turn.sin_cos();
        let (window_sin, window_cos) = (TAU / len as f64).sin_cos();
        *self = Self {
            position: start,
            left: len,
            pointer: (cos, sin),
            pointer_step: (step_cos, step_sin),
            window: (1.0, 0.0),
            window_step: (window_cos, window_sin),
            sum: (0.0, 0.0),
        };
    }

    /// Takes up to `count` samples. Returns how many it took
    fn run(&mut self, ring: &[f32; RING], count: usize) -> usize {
        let count = count.min(self.left);
        let (mut cos, mut sin) = self.pointer;
        let (mut window_cos, mut window_sin) = self.window;
        let (step_cos, step_sin) = self.pointer_step;
        let (window_step_cos, window_step_sin) = self.window_step;
        let (mut real, mut imaginary) = self.sum;
        for index in 0..count {
            let sample = ring[(self.position.wrapping_add(index)) & RING_MASK] as f64 * (0.5 - 0.5 * window_cos);
            real += sample * cos;
            imaginary -= sample * sin;
            (cos, sin) = (cos * step_cos - sin * step_sin, sin * step_cos + cos * step_sin);
            (window_cos, window_sin) = (
                window_cos * window_step_cos - window_sin * window_step_sin,
                window_sin * window_step_cos + window_cos * window_step_sin,
            );
        }
        self.position = self.position.wrapping_add(count);
        self.left -= count;
        self.pointer = (cos, sin);
        self.window = (window_cos, window_sin);
        self.sum = (real, imaginary);
        count
    }

    fn done(&self) -> bool {
        self.left == 0
    }
}

fn power(sum: (f64, f64)) -> f64 {
    sum.0 * sum.0 + sum.1 * sum.1
}

/// The pitch from how far the phase of each of the first `count` harmonics has moved between
/// two windows `apart` samples from each other, both taken at `pitch`. With it, how far the
/// harmonics are from agreeing on it, in cents, and the power in the older window against
/// the newer one
fn measure(
    older: &[(f64, f64)],
    newer: &[(f64, f64)],
    count: usize,
    pitch: f64,
    rate: f64,
    apart: usize,
) -> Option<(f64, f64, f64)> {
    let mut pitches = [0.0; FINE_HARMONICS];
    let mut weights = [0.0; FINE_HARMONICS];
    let (mut older_power, mut newer_power) = (0.0, 0.0);
    for harmonic in 0..count {
        let (older, newer) = (older[harmonic], newer[harmonic]);
        // The newer one times the older one mirrored: its angle is the phase gained
        let real = newer.0 * older.0 + newer.1 * older.1;
        let imaginary = newer.1 * older.0 - newer.0 * older.1;
        let gained = imaginary.atan2(real);
        let number = (harmonic + 1) as f64;
        pitches[harmonic] = pitch + gained * rate / (TAU * apart as f64 * number);
        weights[harmonic] = (power(older) * power(newer)).sqrt();
        older_power += power(older);
        newer_power += power(newer);
    }

    let weight: f64 = weights.iter().sum();
    if weight <= 1e-30 || newer_power <= 0.0 {
        return None;
    }
    let mean = pitches.iter().zip(&weights).map(|(pitch, weight)| pitch * weight).sum::<f64>() / weight;
    if mean <= 0.0 {
        return None;
    }
    // Hum and whatever else lies close to one harmonic pushes that one about and leaves
    // the others alone. With three that are all really there the one in the middle is
    // taken: it is never the one that was pushed
    let strongest = weights.iter().fold(0.0, |most: f64, &weight| most.max(weight));
    let all_there = count == FINE_HARMONICS && weights.iter().all(|&weight| weight >= MEDIAN_SHARE * strongest);
    let pitch = if all_there {
        let [a, b, c] = pitches;
        a.max(b).min(a.min(b).max(c))
    } else {
        mean
    };
    // A harmonic that came out below zero is as far off as can be
    let spread = pitches[..count]
        .iter()
        .zip(&weights)
        .map(|(&each, weight)| if each > 0.0 { weight * cents_between(each, mean).powi(2) } else { weight * 1e6 })
        .sum::<f64>()
        / weight;
    Some((pitch, spread.sqrt(), older_power / newer_power))
}

/// Where a reading is in its making
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// Until the next reading is due
    Waiting,
    /// The difference function, lag by lag
    Lags,
    /// The harmonics at the rough pitch in the newest window
    Harmonics,
    /// What is halfway between them, at half and at one and a half times the pitch
    Under,
    /// The first harmonics in the window before it: with the ones above, a better pitch
    Older,
    /// Three windows at the better pitch, oldest first
    ExactOlder,
    ExactMiddle,
    ExactNewer,
}

pub struct Tuner {
    reading: Arc<TunerReading>,

    // Thinning out: every `every`th sample of the low-passed input goes into the ring
    every: usize,
    phase: usize,
    // The rate of the ring
    rate: f64,
    lowpass: [Biquad; 4],
    highpass: Biquad,
    ring: [f32; RING],
    // Samples put into the ring so far. Positions in the ring count on from this
    written: usize,

    min_lag: usize,
    max_lag: usize,
    // Samples the difference function compares
    span: usize,
    hop: usize,
    level_min: f32,

    stage: Stage,
    // When the next reading starts, and the sample behind the last one this reading sees
    due: usize,
    end: usize,
    lag: usize,
    difference: [f32; MAX_LAG + 2],
    // The pitch being measured, in Hz, and whether it was already moved up once
    pitch: f64,
    moved: bool,
    // The windows of the measurement: their length and how far apart they are
    window: usize,
    apart: usize,
    harmonics: usize,
    harmonic: usize,
    probe: Probe,
    newer: [(f64, f64); MAX_HARMONICS],
    under: [(f64, f64); 2],
    older: [(f64, f64); FINE_HARMONICS],
    middle: [(f64, f64); FINE_HARMONICS],
    // The pitch after the first pass
    first: f64,

    // The last readings of the note that is shown, a reading that may start the next
    // note, and how many readings failed since
    shown: [f32; SHOWN],
    kept: usize,
    pending: Option<f32>,
    misses: u32,
}

impl Tuner {
    pub fn new() -> Self {
        let mut tuner = Self {
            reading: Arc::new(TunerReading::new()),
            every: 1,
            phase: 0,
            rate: TARGET_RATE_HZ as f64,
            lowpass: [Biquad::new(); 4],
            highpass: Biquad::new(),
            ring: [0.0; RING],
            written: 0,
            min_lag: 2,
            max_lag: MAX_LAG,
            span: MAX_LAG,
            hop: 1,
            level_min: 0.0,
            stage: Stage::Waiting,
            due: 0,
            end: 0,
            lag: 1,
            difference: [0.0; MAX_LAG + 2],
            pitch: 0.0,
            moved: false,
            window: 0,
            apart: 0,
            harmonics: 0,
            harmonic: 0,
            probe: Probe::IDLE,
            newer: [(0.0, 0.0); MAX_HARMONICS],
            under: [(0.0, 0.0); 2],
            older: [(0.0, 0.0); FINE_HARMONICS],
            middle: [(0.0, 0.0); FINE_HARMONICS],
            first: 0.0,
            shown: [0.0; SHOWN],
            kept: 0,
            pending: None,
            misses: 0,
        };
        tuner.set_sample_rate(44100.0);
        tuner
    }

    /// Where the editor finds what the tuner hears
    pub fn reading(&self) -> Arc<TunerReading> {
        self.reading.clone()
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.every = ((sample_rate / TARGET_RATE_HZ).round() as usize).max(1);
        let rate = sample_rate / self.every as f32;
        self.rate = rate as f64;
        for (filter, q) in self.lowpass.iter_mut().zip(LOWPASS_Q) {
            filter.set(BiquadCoeffs::lowpass(LOWPASS_HZ, q, sample_rate));
        }
        self.highpass.set(BiquadCoeffs::highpass(HIGHPASS_HZ, 0.7071, rate));

        self.min_lag = ((rate / MAX_HZ) as usize).max(2);
        self.max_lag = ((rate / MIN_HZ).ceil() as usize).min(MAX_LAG);
        self.span = self.max_lag + self.max_lag / 4;
        self.hop = ((HOP_MS * 0.001 * rate).round() as usize).max(1);
        self.level_min = 10.0f32.powf(LEVEL_MIN_DB / 20.0);
        self.start();
    }

    /// Forgets what was heard and starts listening. Does not allocate
    pub fn start(&mut self) {
        for filter in &mut self.lowpass {
            filter.reset();
        }
        self.highpass.reset();
        self.phase = 0;
        self.ring = [0.0; RING];
        self.written = 0;
        self.stage = Stage::Waiting;
        self.due = self.hop;
        self.kept = 0;
        self.pending = None;
        self.misses = 0;
        self.reading.set(None);
    }

    /// The tuner was switched off: there is nothing to show
    pub fn stop(&mut self) {
        self.reading.set(None);
    }

    /// Listens to a block. The work per sample is bounded
    pub fn process(&mut self, input: &[f32]) {
        for &sample in input {
            let mut filtered = sample as f64;
            for filter in &mut self.lowpass {
                filtered = filter.process(filtered);
            }
            self.phase += 1;
            if self.phase < self.every {
                continue;
            }
            self.phase = 0;
            self.ring[self.written & RING_MASK] = self.highpass.process(filtered) as f32;
            self.written = self.written.wrapping_add(1);
            self.work();
        }
    }

    fn at(&self, position: usize) -> f32 {
        self.ring[position & RING_MASK]
    }

    /// One sample's share of the reading that is being worked out
    fn work(&mut self) {
        let mut budget = BUDGET;
        if self.stage == Stage::Waiting {
            if self.written < self.due {
                return;
            }
            self.due = self.written + self.hop;
            self.end = self.written;
            budget = budget.saturating_sub(self.span);
            if !self.is_loud_enough() {
                self.miss();
                return;
            }
            self.lag = 1;
            self.stage = Stage::Lags;
        }

        while budget > 0 && self.stage != Stage::Waiting {
            let used = match self.stage {
                Stage::Lags => self.work_on_lags(budget),
                _ => self.work_on_probe(budget),
            };
            budget = budget.saturating_sub(used);
        }
    }

    /// Whether the newest stretch of the ring is above the level of silence
    fn is_loud_enough(&self) -> bool {
        let from = self.end.wrapping_sub(self.span);
        let energy: f32 = (0..self.span).map(|index| self.at(from.wrapping_add(index)).powi(2)).sum();
        energy >= self.level_min * self.level_min * self.span as f32
    }

    /// The difference between the newest stretch and the same stretch `lag` samples
    /// earlier, for as many lags as the budget allows and one at least
    fn work_on_lags(&mut self, budget: usize) -> usize {
        let mut used = 0;
        let from = self.end.wrapping_sub(self.span);
        while self.lag <= self.max_lag + 1 && (used == 0 || used + self.span <= budget) {
            let mut sum = 0.0f32;
            for index in 0..self.span {
                let position = from.wrapping_add(index);
                let difference = self.at(position) - self.at(position.wrapping_sub(self.lag));
                sum += difference * difference;
            }
            self.difference[self.lag] = sum;
            self.lag += 1;
            used += self.span;
        }

        if self.lag > self.max_lag + 1 {
            match self.pick_lag() {
                Some(lag) => {
                    self.pitch = self.rate / lag as f64;
                    self.moved = false;
                    self.begin_harmonics();
                }
                None => self.miss(),
            }
        }
        used
    }

    /// The period in samples from the difference function, or nothing if the signal does
    /// not repeat. Leaves the normalised function in `difference`
    fn pick_lag(&mut self) -> Option<f32> {
        // Each difference against the mean of those up to it: a dip is then small whatever
        // the level, and the short lags, where any signal differs little, are not dips
        let mut sum = 0.0f32;
        for lag in 1..=self.max_lag + 1 {
            sum += self.difference[lag];
            self.difference[lag] = if sum > 0.0 { self.difference[lag] * lag as f32 / sum } else { 1.0 };
        }
        self.difference[0] = 1.0;
        let normalised = &self.difference;

        let best = normalised[self.min_lag..=self.max_lag].iter().fold(f32::MAX, |best, &value| best.min(value));
        if best > CLARITY_MAX {
            return None;
        }
        for lag in self.min_lag..=self.max_lag {
            let (before, here, after) = (normalised[lag - 1], normalised[lag], normalised[lag + 1]);
            if here > before || here >= after {
                continue;
            }
            // The bottom of the parabola through the three: the dip lies between samples
            let bend = before - 2.0 * here + after;
            let offset = if bend > 0.0 { 0.5 * (before - after) / bend } else { 0.0 };
            let bottom = here - 0.25 * (before - after) * offset;
            if bottom <= best + OCTAVE_MARGIN {
                return Some(lag as f32 + offset);
            }
        }
        None
    }

    /// Sets the windows for the pitch as it is known now: a whole number of periods, so each
    /// harmonic is blind to the others
    fn set_windows(&mut self) -> bool {
        let period = self.rate / self.pitch;
        // An even number, so half the pitch has whole periods in it as well
        let periods = MIN_PERIODS.max(2.0 * (MIN_WINDOW_MS * 0.001 * self.pitch / 2.0).ceil());
        self.window = (periods * period).round() as usize;
        // Even: the middle window is halfway
        self.apart = self.window / 4 * 2;
        self.harmonics = ((TOP_RATIO * self.rate / self.pitch) as usize).clamp(1, MAX_HARMONICS);
        // What the ring still holds of it when the reading is done
        self.apart >= 2 && self.window + self.apart + RING / 4 <= RING
    }

    /// Starts a probe at the `harmonic`th harmonic (counted from 0) in the window that
    /// ends `back` samples before the newest one
    fn begin_probe(&mut self, stage: Stage, harmonic: usize, back: usize) {
        self.begin_probe_at(stage, harmonic, back, (harmonic + 1) as f64);
    }

    /// The same at any multiple of the pitch. `index` is where the result goes
    fn begin_probe_at(&mut self, stage: Stage, index: usize, back: usize, multiple: f64) {
        self.stage = stage;
        self.harmonic = index;
        let start = self.end.wrapping_sub(self.window + back);
        self.probe.begin(start, self.window, TAU * self.pitch * multiple / self.rate, self.end);
    }

    fn begin_harmonics(&mut self) {
        if !self.set_windows() {
            self.miss();
            return;
        }
        self.begin_probe(Stage::Harmonics, 0, 0);
    }

    /// How far apart the two windows of the first pass are: two periods at most, so the
    /// phase cannot go round unseen while the pitch is only known roughly
    fn first_apart(&self) -> usize {
        self.apart.min((2.0 * self.rate / self.pitch).round() as usize).max(1)
    }

    fn work_on_probe(&mut self, budget: usize) -> usize {
        let taken = self.probe.run(&self.ring, (budget / PROBE_COST).max(1));
        if !self.probe.done() {
            return taken * PROBE_COST;
        }

        let (harmonic, sum) = (self.harmonic, self.probe.sum);
        let fine = self.harmonics.min(FINE_HARMONICS);
        match self.stage {
            Stage::Harmonics => {
                self.newer[harmonic] = sum;
                if harmonic + 1 < self.harmonics {
                    self.begin_probe(Stage::Harmonics, harmonic + 1, 0);
                } else {
                    self.judge_harmonics();
                }
            }
            Stage::Under => {
                self.under[harmonic] = sum;
                if harmonic == 0 {
                    self.begin_probe_at(Stage::Under, 1, 0, 1.5);
                } else {
                    self.judge_under();
                }
            }
            Stage::Older => {
                self.older[harmonic] = sum;
                if harmonic + 1 < fine {
                    self.begin_probe(Stage::Older, harmonic + 1, self.first_apart());
                } else {
                    match measure(&self.older, &self.newer, fine, self.pitch, self.rate, self.first_apart()) {
                        Some((pitch, _, _)) => {
                            self.first = pitch;
                            self.pitch = pitch;
                            if self.set_windows() {
                                self.begin_probe(Stage::ExactOlder, 0, self.apart);
                            } else {
                                self.miss();
                            }
                        }
                        _ => self.miss(),
                    }
                }
            }
            Stage::ExactOlder => {
                self.older[harmonic] = sum;
                if harmonic + 1 < fine {
                    self.begin_probe(Stage::ExactOlder, harmonic + 1, self.apart);
                } else {
                    self.begin_probe(Stage::ExactMiddle, 0, self.apart / 2);
                }
            }
            Stage::ExactMiddle => {
                self.middle[harmonic] = sum;
                if harmonic + 1 < fine {
                    self.begin_probe(Stage::ExactMiddle, harmonic + 1, self.apart / 2);
                } else {
                    self.begin_probe(Stage::ExactNewer, 0, 0);
                }
            }
            Stage::ExactNewer => {
                self.newer[harmonic] = sum;
                if harmonic + 1 < fine {
                    self.begin_probe(Stage::ExactNewer, harmonic + 1, 0);
                } else {
                    self.finish();
                }
            }
            // No probe runs in these
            Stage::Waiting | Stage::Lags => self.miss(),
        }
        taken * PROBE_COST
    }

    /// Checks the rough pitch against the harmonics that are there
    fn judge_harmonics(&mut self) {
        let powers = self.newer.map(power);
        let powers = &powers[..self.harmonics];
        let strongest = powers.iter().fold(0.0, |most: f64, &power| most.max(power));
        let is_there = |power: f64| power >= SIGNIFICANT * strongest && power > 0.0;
        let Some(lowest) = powers.iter().position(|&power| is_there(power)) else {
            self.miss();
            return;
        };

        if lowest == 0 {
            // The window holds an even number of periods, so what lies halfway between
            // the harmonics can be told from them. Not looked for under the lowest note
            if !self.moved && self.pitch * 0.5 >= MIN_HZ as f64 {
                self.begin_probe_at(Stage::Under, 0, 0, 0.5);
            } else {
                self.begin_probe(Stage::Older, 0, self.first_apart());
            }
            return;
        }
        // Nothing at the pitch itself. If all there is sits at the multiples of one harmonic,
        // that one is the note. If not, more than one note is sounding
        let multiple = lowest + 1;
        let all_multiples = powers.iter().enumerate().all(|(index, &power)| !is_there(power) || (index + 1) % multiple == 0);
        let pitch = self.pitch * multiple as f64;
        if all_multiples && !self.moved && pitch <= MAX_HZ as f64 * 1.05 {
            self.pitch = pitch;
            self.moved = true;
            self.begin_harmonics();
        } else {
            self.miss();
        }
    }

    /// Checks for a note an octave under the pitch, whose first harmonic is weak
    fn judge_under(&mut self) {
        let strongest = self.newer[..self.harmonics].iter().fold(0.0, |most: f64, &sum| most.max(power(sum)));
        let [half, third] = self.under.map(power);
        let third_in_reach = self.pitch * 1.5 <= TOP_RATIO * self.rate;
        let found = if third_in_reach {
            third >= UNDER_SIGNIFICANT * strongest || half >= UNDER_ALONE * strongest
        } else {
            half >= UNDER_SIGNIFICANT * strongest
        };
        if found {
            self.pitch *= 0.5;
            self.moved = true;
            self.begin_harmonics();
        } else {
            self.begin_probe(Stage::Older, 0, self.first_apart());
        }
    }

    fn finish(&mut self) {
        let fine = self.harmonics.min(FINE_HARMONICS);
        let half = self.apart / 2;
        let whole = measure(&self.older, &self.newer, fine, self.pitch, self.rate, self.apart);
        let earlier = measure(&self.older, &self.middle, fine, self.pitch, self.rate, half);
        let later = measure(&self.middle, &self.newer, fine, self.pitch, self.rate, half);
        let (Some((pitch, spread, older_share)), Some((earlier, _, _)), Some((later, _, _))) = (whole, earlier, later) else {
            self.miss();
            return;
        };
        let in_range = pitch >= MIN_HZ as f64 && pitch <= MAX_HZ as f64;
        let passes_agree = cents_between(pitch, self.first).abs() <= PASSES_AGREE_CENTS;
        let steady = cents_between(later, earlier).abs() <= STEADY_CENTS;
        if in_range && passes_agree && steady && spread <= HARMONICS_AGREE_CENTS && older_share >= OLDER_POWER_MIN {
            self.accept(pitch as f32);
        } else {
            self.miss();
        }
    }

    /// A reading: shown as the median of the last few of its note
    fn accept(&mut self, hz: f32) {
        self.stage = Stage::Waiting;
        self.misses = 0;
        if self.kept > 0 && cents_between(hz as f64, self.median() as f64).abs() > NOTE_CHANGE_CENTS as f64 {
            // One stray reading, or the first of the next note: the reading after it tells
            match self.pending.take() {
                Some(pending) if cents_between(hz as f64, pending as f64).abs() <= NOTE_CHANGE_CENTS as f64 => {
                    self.shown[0] = pending;
                    self.shown[1] = hz;
                    self.kept = 2;
                }
                _ => {
                    self.pending = Some(hz);
                    return;
                }
            }
        } else {
            self.pending = None;
            if self.kept == SHOWN {
                self.shown.rotate_left(1);
                self.kept -= 1;
            }
            self.shown[self.kept] = hz;
            self.kept += 1;
        }
        self.reading.set(Some(self.median()));
    }

    /// No reading this time. After a few in a row there is nothing to show
    fn miss(&mut self) {
        self.stage = Stage::Waiting;
        self.misses += 1;
        if self.misses >= MISSES_TO_CLEAR {
            self.kept = 0;
            self.pending = None;
            self.reading.set(None);
        }
    }

    fn median(&self) -> f32 {
        let [a, b, c] = self.shown;
        match self.kept {
            1 => a,
            2 => 0.5 * (a + b),
            _ => a.max(b).min(a.min(b).max(c)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_util::*;
    use std::time::Instant;

    const BLOCK: usize = 64;
    const RATES: [f32; 6] = [44100.0, 48000.0, 88200.0, 96000.0, 176400.0, 192000.0];

    // Open strings by MIDI number: standard tuning, then what drop D, drop C, a drop A and a
    // seven-string add to it
    const STANDARD: [i32; 6] = [40, 45, 50, 55, 59, 64];
    const LOWERED: [i32; 8] = [38, 36, 43, 48, 53, 57, 33, 35];
    // The top of the neck on the first string
    const HIGHEST: i32 = 88;

    pub(crate) fn note_hz(number: i32) -> f32 {
        (REFERENCE_A_HZ as f64 * 2.0f64.powf((number - 69) as f64 / 12.0)) as f32
    }

    fn name_of(number: i32) -> String {
        let note = Note::nearest(note_hz(number));
        format!("{}{}", note.name, note.octave)
    }

    /// Every open string of the tunings above, each also at the twelfth fret, and the
    /// highest note, low to high
    fn all_notes() -> Vec<i32> {
        let mut notes: Vec<i32> = STANDARD.iter().chain(&LOWERED).flat_map(|&open| [open, open + 12]).collect();
        notes.push(HIGHEST);
        notes.sort();
        notes.dedup();
        notes
    }

    /// The lowest and the highest note and a few between them, for the rates that are
    /// not looked at note by note
    fn some_notes() -> [i32; 6] {
        [33, 35, 40, 57, 76, HIGHEST]
    }

    fn cents(hz: f32, expected_hz: f32) -> f32 {
        cents_between(hz as f64, expected_hz as f64) as f32
    }

    /// A tone whose second harmonic is ten times its fundamental, as a low string gives
    fn weak_fundamental(freq_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        let levels = [0.1, 1.0, 0.6, 0.45, 0.3, 0.2, 0.12, 0.08];
        let mut tone = vec![0.0f32; len];
        for (index, level) in levels.iter().enumerate() {
            let partial_hz = freq_hz * (index + 1) as f32;
            if partial_hz < 0.45 * sample_rate {
                for (sample, partial) in tone.iter_mut().zip(sine(partial_hz, 0.15 * level, sample_rate, len)) {
                    *sample += partial;
                }
            }
        }
        tone
    }

    /// How stiff a string of this pitch is, about: the wound low ones most
    fn stiffness(freq_hz: f32) -> f32 {
        if freq_hz < 130.0 {
            1.0e-4
        } else if freq_hz < 300.0 {
            6.0e-5
        } else {
            3.0e-5
        }
    }

    pub(crate) fn string(freq_hz: f32, sample_rate: f32, seconds: f32) -> Vec<f32> {
        stiff_string(freq_hz, stiffness(freq_hz), Pluck::OPEN, sample_rate, (seconds * sample_rate) as usize)
    }

    fn new_tuner(sample_rate: f32) -> Tuner {
        let mut tuner = Tuner::new();
        tuner.set_sample_rate(sample_rate);
        tuner
    }

    /// What the tuner shows after each block of `block` samples
    fn readings_in_blocks(signal: &[f32], sample_rate: f32, block: usize) -> Vec<Option<f32>> {
        let mut tuner = new_tuner(sample_rate);
        let shown = tuner.reading();
        signal
            .chunks(block)
            .map(|piece| {
                tuner.process(piece);
                shown.hz()
            })
            .collect()
    }

    fn readings(signal: &[f32], sample_rate: f32) -> Vec<Option<f32>> {
        readings_in_blocks(signal, sample_rate, BLOCK)
    }

    /// What it shows at the end of the signal
    fn settled(signal: &[f32], sample_rate: f32) -> Option<f32> {
        *readings(signal, sample_rate).last().unwrap()
    }

    /// Seconds until the first reading
    fn first_reading_s(signal: &[f32], sample_rate: f32) -> Option<f32> {
        let first = readings(signal, sample_rate).iter().position(|reading| reading.is_some())?;
        Some(((first + 1) * BLOCK) as f32 / sample_rate)
    }

    /// The reading furthest from `expected_hz` among those from `from_s` on, in cents, and
    /// how many of the blocks from there on had one
    fn worst_from(signal: &[f32], sample_rate: f32, expected_hz: f32, from_s: f32) -> (f32, usize, usize) {
        let all = readings(signal, sample_rate);
        let from = (from_s * sample_rate) as usize / BLOCK;
        let heard: Vec<f32> = all[from..].iter().flatten().map(|&hz| cents(hz, expected_hz)).collect();
        let worst = heard.iter().fold(0.0f32, |worst, &off| if off.abs() > worst.abs() { off } else { worst });
        (worst, heard.len(), all.len() - from)
    }

    #[test]
    fn test_notes_have_their_names() {
        let named = |hz: f32| {
            let note = Note::nearest(hz);
            (note.name, note.octave)
        };
        assert_eq!(named(440.0), ("A", 4));
        assert_eq!(named(82.41), ("E", 2));
        assert_eq!(named(55.0), ("A", 1));
        assert_eq!(named(61.74), ("B", 1));
        assert_eq!(named(261.63), ("C", 4));
        assert_eq!(named(246.94), ("B", 3));
        assert_eq!(named(185.0), ("F#", 3));
        assert_eq!(named(1318.51), ("E", 6));

        assert!(Note::nearest(440.0).cents.abs() < 1e-3);
        let flat = Note::nearest(440.0 * 2.0f32.powf(-20.0 / 1200.0));
        assert_eq!(flat.name, "A");
        assert!((flat.cents + 20.0).abs() < 0.01, "{}", flat.cents);
        let sharp = Note::nearest(note_hz(40) * 2.0f32.powf(35.0 / 1200.0));
        assert_eq!((sharp.name, sharp.octave), ("E", 2));
        assert!((sharp.cents - 35.0).abs() < 0.01, "{}", sharp.cents);
        // More than half a semitone off is the next note, flat
        let next = Note::nearest(440.0 * 2.0f32.powf(60.0 / 1200.0));
        assert_eq!(next.name, "A#");
        assert!((next.cents + 40.0).abs() < 0.01);
    }

    #[test]
    fn test_reading_is_shared_and_empty_at_first() {
        let mut tuner = new_tuner(48000.0);
        let shown = tuner.reading();
        assert_eq!(shown.hz(), None);
        tuner.process(&sine(220.0, 0.2, 48000.0, 24_000));
        assert!(shown.hz().is_some());
        tuner.stop();
        assert_eq!(shown.hz(), None);
    }

    #[test]
    fn test_steady_tones_read_within_a_cent_at_every_rate() {
        for sample_rate in RATES {
            let notes = if sample_rate == 48000.0 { all_notes() } else { some_notes().to_vec() };
            let len = (0.4 * sample_rate) as usize;
            for number in notes {
                let freq = note_hz(number);
                let tones = [("sine", sine(freq, 0.2, sample_rate, len)), ("weak fundamental", weak_fundamental(freq, sample_rate, len))];
                for (kind, tone) in tones {
                    let heard = settled(&tone, sample_rate);
                    let off = heard.map(|hz| cents(hz, freq));
                    assert!(
                        off.is_some_and(|off| off.abs() < 1.0),
                        "{} {kind} at {sample_rate} Hz: {heard:?} for {freq} Hz, {off:?} cents",
                        name_of(number)
                    );
                }
            }
        }
    }

    #[test]
    fn test_steady_tones_between_the_notes_read_within_a_cent() {
        // A string on its way to pitch: 50 cents flat to 50 cents sharp, 5 cents at a time
        let sample_rate = 44100.0;
        let len = (0.4 * sample_rate) as usize;
        for number in [33, 40, 69, HIGHEST] {
            for step in -10i32..=10 {
                let detune = step as f32 * 5.0;
                let freq = note_hz(number) * 2.0f32.powf(detune / 1200.0);
                let heard = settled(&weak_fundamental(freq, sample_rate, len), sample_rate);
                let off = heard.map(|hz| cents(hz, freq));
                assert!(off.is_some_and(|off| off.abs() < 1.0), "{} {detune:+} cents: {heard:?}", name_of(number));
                if step.abs() < 10 {
                    let note = Note::nearest(heard.unwrap());
                    assert_eq!(format!("{}{}", note.name, note.octave), name_of(number));
                    assert!((note.cents - detune).abs() < 1.0, "{} {detune:+} cents shows {}", name_of(number), note.cents);
                }
            }
        }
    }

    #[test]
    fn test_plucked_strings_read_within_three_cents_as_they_fade() {
        for sample_rate in RATES {
            let notes = if sample_rate == 48000.0 { all_notes() } else { some_notes().to_vec() };
            for number in notes {
                let freq = note_hz(number);
                // Stiff strings, whose upper partials are sharp, and the delay-line string the
                // amp is tested with. The latter cannot reach the highest notes
                let mut plucks = vec![("stiff string", string(freq, sample_rate, 1.2))];
                if freq < 700.0 {
                    plucks.push(("delay line", pluck(freq, Pluck::OPEN, 3, sample_rate, (1.2 * sample_rate) as usize)));
                }
                for (kind, note) in plucks {
                    let (worst, heard, blocks) = worst_from(&note, sample_rate, freq, 0.25);
                    assert_eq!(heard, blocks, "{} {kind} at {sample_rate} Hz: lost for {} blocks", name_of(number), blocks - heard);
                    assert!(worst.abs() < 3.0, "{} {kind} at {sample_rate} Hz: {worst} cents", name_of(number));
                }
            }
        }
    }

    #[test]
    fn test_a_note_that_fades_away_is_never_read_wrong() {
        // A muted note is gone in a quarter of a second, an open one fades for seconds. Every
        // reading there is has to be right, down to where the tuner lets go
        let sample_rate = 48000.0;
        for (how, seconds) in [(Pluck::MUTED, 1.0), (Pluck::OPEN, 6.0)] {
            for number in [33, 40, 59] {
                let freq = note_hz(number);
                let note = stiff_string(freq, stiffness(freq), how, sample_rate, (seconds * sample_rate) as usize);
                let (worst, heard, _) = worst_from(&note, sample_rate, freq, 0.0);
                assert!(heard > 0, "{} was never read", name_of(number));
                assert!(worst.abs() < 3.0, "{}: {worst} cents", name_of(number));
            }
        }
        // And silence after a note clears the display soon
        let mut note = string(note_hz(40), sample_rate, 0.5);
        note.resize((1.0 * sample_rate) as usize, 0.0);
        let all = readings(&note, sample_rate);
        let cleared = all.iter().rposition(|reading| reading.is_some()).unwrap() + 1;
        let after_s = (cleared * BLOCK) as f32 / sample_rate - 0.5;
        assert!(all[all.len() / 2 - 2].is_some());
        assert!((0.0..0.2).contains(&after_s), "Shown for {after_s} s after the note");
    }

    #[test]
    fn test_low_e_is_read_within_150_ms_of_the_pluck() {
        for sample_rate in RATES {
            let freq = note_hz(40);
            let len = (0.5 * sample_rate) as usize;
            for (kind, note) in [("stiff string", string(freq, sample_rate, 0.5)), ("delay line", pluck(freq, Pluck::OPEN, 3, sample_rate, len))] {
                // The pluck comes after a moment of silence, as it does when tuning
                let mut signal = vec![0.0; (0.1 * sample_rate) as usize];
                signal.extend(note);
                let first = first_reading_s(&signal, sample_rate).map(|first| first - 0.1);
                assert!(first.is_some_and(|first| first < 0.15), "{kind} at {sample_rate} Hz: {first:?} s");
            }
        }
    }

    #[test]
    fn test_silence_noise_and_chords_give_no_reading() {
        let sample_rate = 48000.0;
        let len = (2.0 * sample_rate) as usize;
        let never = |name: &str, signal: &[f32]| {
            let heard: Vec<f32> = readings(signal, sample_rate).into_iter().flatten().collect();
            assert!(heard.is_empty(), "{name}: read as {:?} Hz in {} blocks", heard[0], heard.len());
        };

        never("silence", &vec![0.0; len]);
        for level_db in [-12.0, -40.0, -60.0] {
            let mut noise = Noise::new(5);
            let mut hiss: Vec<f32> = (0..len).map(|_| noise.next()).collect();
            set_rms_db(&mut hiss, level_db);
            never("noise", &hiss);
        }
        // A tone under the level of silence
        never("a tone too quiet to be a string", &sine(110.0, 1e-4, sample_rate, len));

        // Power chords: root, fifth and octave. Together they repeat an octave under the
        // root, which for the higher ones is a note a guitar has
        never("power chord on low E", &power_chords(sample_rate, 2.0));
        for root in [40, 45, 52] {
            let mut chord = vec![0.0f32; len];
            for interval in [0, 7, 12] {
                let freq = note_hz(root + interval);
                for (sample, string) in chord.iter_mut().zip(stiff_string(freq, stiffness(freq), Pluck::OPEN, sample_rate, len)) {
                    *sample += 0.5 * string;
                }
            }
            never(&format!("power chord on {}", name_of(root)), &chord);
        }
    }

    #[test]
    fn test_hiss_and_hum_under_a_string_do_not_move_it() {
        // What a guitar brings along: hiss at -63 dBFS and mains hum at -57 dBFS, under
        // strings that fade from -20 to -35 dBFS
        let sample_rate = 48000.0;
        let len = (1.0 * sample_rate) as usize;
        for hum_hz in [50.0, 60.0] {
            let hum = sine(hum_hz, 0.002, sample_rate, len);
            for number in [33, 35, 40, 45, 55, 64] {
                let freq = note_hz(number);
                let mut noise = Noise::new(9);
                let mut note = string(freq, sample_rate, 1.0);
                for (sample, hum) in note.iter_mut().zip(&hum) {
                    *sample += hum + 0.0012 * noise.next();
                }
                let (worst, heard, blocks) = worst_from(&note, sample_rate, freq, 0.25);
                assert_eq!(heard, blocks, "{} over {hum_hz} Hz hum: lost for {} blocks", name_of(number), blocks - heard);
                assert!(worst.abs() < 3.0, "{} over {hum_hz} Hz hum: {worst} cents", name_of(number));
            }
        }
    }

    #[test]
    fn test_reading_follows_the_next_string() {
        let sample_rate = 48000.0;
        let mut signal = string(note_hz(40), sample_rate, 0.6);
        signal.extend(string(note_hz(45), sample_rate, 0.6));
        let all = readings(&signal, sample_rate);
        let block_at = |seconds: f32| (seconds * sample_rate) as usize / BLOCK;

        assert!(cents(all[block_at(0.55)].unwrap(), note_hz(40)).abs() < 3.0);
        // Never anything but the one note or the other
        for hz in all.iter().flatten() {
            let off = cents(*hz, note_hz(40)).abs().min(cents(*hz, note_hz(45)).abs());
            assert!(off < 3.0, "Read {hz} Hz between the two");
        }
        let changed = all.iter().position(|reading| reading.is_some_and(|hz| cents(hz, note_hz(45)).abs() < 3.0)).unwrap();
        let after_s = (changed * BLOCK) as f32 / sample_rate - 0.6;
        assert!((0.0..0.2).contains(&after_s), "The next string was read after {after_s} s");
    }

    #[test]
    fn test_readings_do_not_depend_on_block_size() {
        // The work is counted in samples, not in blocks
        let sample_rate = 44100.0;
        let signal = string(note_hz(40), sample_rate, 0.5);
        let reference = readings_in_blocks(&signal, sample_rate, 1);
        for block in [7, 64, 1000] {
            let seen = readings_in_blocks(&signal, sample_rate, block);
            for (index, reading) in seen.iter().enumerate() {
                let at = ((index + 1) * block).min(signal.len()) - 1;
                assert_eq!(reading.map(f32::to_bits), reference[at].map(f32::to_bits), "Blocks of {block}, sample {at}");
            }
        }
    }

    #[test]
    fn test_start_forgets_the_last_note() {
        let sample_rate = 48000.0;
        let mut tuner = new_tuner(sample_rate);
        let shown = tuner.reading();
        tuner.process(&string(note_hz(40), sample_rate, 0.5));
        assert!(shown.hz().is_some());
        tuner.start();
        assert_eq!(shown.hz(), None);

        // And hears the next one as a tuner that was never used
        let next = string(note_hz(57), sample_rate, 0.5);
        tuner.process(&next);
        assert_eq!(shown.hz().map(f32::to_bits), settled(&next, sample_rate).map(f32::to_bits));
    }

    #[test]
    fn test_odd_sample_rates_fit_the_fixed_buffers() {
        for sample_rate in [8000.0, 11025.0, 22050.0, 32000.0, 384000.0] {
            let tuner = new_tuner(sample_rate);
            assert!(tuner.max_lag <= MAX_LAG && tuner.min_lag >= 2, "{sample_rate} Hz");
            assert!(tuner.rate <= 1.5 * TARGET_RATE_HZ as f64 + 1.0, "{sample_rate} Hz runs at {}", tuner.rate);
            let freq = note_hz(40);
            let heard = settled(&sine(freq, 0.2, sample_rate, (0.4 * sample_rate) as usize), sample_rate);
            assert!(heard.is_some_and(|hz| cents(hz, freq).abs() < 1.0), "{sample_rate} Hz: {heard:?}");
        }
    }

    /// Time of each 64-sample block of a signal in nanoseconds, the fastest of a few runs
    /// for each block: the work a block does is the same every time, what the machine adds
    /// to it is not
    fn block_times_ns(signal: &[f32], sample_rate: f32) -> Vec<f64> {
        let mut times = vec![f64::MAX; signal.len() / BLOCK];
        for _ in 0..7 {
            let mut tuner = new_tuner(sample_rate);
            for (time, block) in times.iter_mut().zip(signal.chunks_exact(BLOCK)) {
                let start = Instant::now();
                tuner.process(std::hint::black_box(block));
                *time = time.min(start.elapsed().as_secs_f64() * 1e9);
            }
        }
        times
    }

    /// Accuracy, speed and cost of the tuner. Run before and after changing it:
    /// cargo test -p amp --release tuner_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn tuner_report() {
        let off_of = |heard: Option<f32>, freq: f32| heard.map_or("     none".to_owned(), |hz| format!("{:>+9.2}", cents(hz, freq)));
        let delay_line = |freq: f32, sample_rate: f32, seconds: f32| {
            (freq < 700.0).then(|| pluck(freq, Pluck::OPEN, 3, sample_rate, (seconds * sample_rate) as usize))
        };

        println!("\nError in cents at 48 kHz. Tones after 0.4 s; strings: the worst reading from 0.25 to 1.2 s");
        println!("{:<5} {:>8} {:>9} {:>9} {:>9} {:>9} {:>9}", "note", "Hz", "sine", "weak 1st", "stiff", "delay ln", "first ms");
        let sample_rate = 48000.0;
        for number in all_notes() {
            let freq = note_hz(number);
            let len = (0.4 * sample_rate) as usize;
            let stiff = string(freq, sample_rate, 1.2);
            let worst = |note: &[f32]| {
                let (worst, heard, blocks) = worst_from(note, sample_rate, freq, 0.25);
                if heard == blocks { format!("{worst:>+9.2}") } else { format!("{:>9}", format!("lost {}", blocks - heard)) }
            };
            println!(
                "{:<5} {:>8.2} {} {} {} {} {:>9}",
                name_of(number),
                freq,
                off_of(settled(&sine(freq, 0.2, sample_rate, len), sample_rate), freq),
                off_of(settled(&weak_fundamental(freq, sample_rate, len), sample_rate), freq),
                worst(&stiff),
                delay_line(freq, sample_rate, 1.2).map_or(format!("{:>9}", "-"), |note| worst(&note)),
                first_reading_s(&stiff, sample_rate).map_or("none".to_owned(), |first| format!("{:.0}", first * 1000.0)),
            );
        }

        println!("\nWorst error in cents over all notes, per sample rate");
        println!("{:<8} {:>9} {:>9} {:>9} {:>9} {:>14}", "rate", "sine", "weak 1st", "stiff", "delay ln", "first ms (E2)");
        for sample_rate in RATES {
            let len = (0.4 * sample_rate) as usize;
            let mut worst = [0.0f32; 4];
            let mut keep = |slot: usize, off: f32| {
                if off.abs() > worst[slot].abs() {
                    worst[slot] = off;
                }
            };
            for number in all_notes() {
                let freq = note_hz(number);
                let lost = 999.0;
                keep(0, settled(&sine(freq, 0.2, sample_rate, len), sample_rate).map_or(lost, |hz| cents(hz, freq)));
                keep(1, settled(&weak_fundamental(freq, sample_rate, len), sample_rate).map_or(lost, |hz| cents(hz, freq)));
                let (off, heard, blocks) = worst_from(&string(freq, sample_rate, 1.2), sample_rate, freq, 0.25);
                keep(2, if heard == blocks { off } else { lost });
                if let Some(note) = delay_line(freq, sample_rate, 1.2) {
                    let (off, heard, blocks) = worst_from(&note, sample_rate, freq, 0.25);
                    keep(3, if heard == blocks { off } else { lost });
                }
            }
            let first = first_reading_s(&string(note_hz(40), sample_rate, 0.5), sample_rate);
            println!(
                "{:<8} {:>+9.2} {:>+9.2} {:>+9.2} {:>+9.2} {:>14}",
                sample_rate,
                worst[0],
                worst[1],
                worst[2],
                worst[3],
                first.map_or("none".to_owned(), |first| format!("{:.0}", first * 1000.0))
            );
        }

        println!("\nA string between the notes: weak fundamental at 44.1 kHz, error in cents of the reading");
        print!("{:<5}", "note");
        for step in -10..=10 {
            print!(" {:>+5}", step * 5);
        }
        println!();
        for number in [33, 40, 69, HIGHEST] {
            print!("{:<5}", name_of(number));
            for step in -10i32..=10 {
                let freq = note_hz(number) * 2.0f32.powf(step as f32 * 5.0 / 1200.0);
                let heard = settled(&weak_fundamental(freq, 44100.0, 17_640), 44100.0);
                print!(" {}", heard.map_or(" none".to_owned(), |hz| format!("{:>+5.2}", cents(hz, freq))));
            }
            println!();
        }

        println!("\nOpen strings fading at 48 kHz: error in cents at each time, and the string's level in dBFS RMS");
        let times = [0.1, 0.15, 0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0];
        print!("{:<5}", "note");
        for time in times {
            print!(" {:>7}", format!("{time} s"));
        }
        println!();
        for number in [33, 40, 55, 64] {
            let freq = note_hz(number);
            let note = string(freq, 48000.0, 8.1);
            let all = readings(&note, 48000.0);
            print!("{:<5}", name_of(number));
            for time in times {
                let block = (time * 48000.0) as usize / BLOCK - 1;
                print!(" {}", all[block].map_or("   none".to_owned(), |hz| format!("{:>+7.2}", cents(hz, freq))));
            }
            println!();
            print!("{:<5}", "");
            for time in times {
                let at = (time * 48000.0) as usize;
                print!(" {:>7.1}", to_db(rms(&note[at - 2400..at])));
            }
            println!();
        }

        println!("\nTime to the first reading in ms after the pluck, stiff string and delay line");
        println!("{:<5} {:>12} {:>12}", "note", "stiff", "delay line");
        for number in [33, 35, 36, 38, 40, 45, 50, 55, 59, 64, 76] {
            let freq = note_hz(number);
            let ms = |note: &[f32]| first_reading_s(note, 48000.0).map_or("none".to_owned(), |first| format!("{:.0}", first * 1000.0));
            println!(
                "{:<5} {:>12} {:>12}",
                name_of(number),
                ms(&string(freq, 48000.0, 0.5)),
                delay_line(freq, 48000.0, 0.5).map_or("-".to_owned(), |note| ms(&note))
            );
        }

        println!("\nTime per 64-sample block of the tuner alone, in microseconds (share of the block's own time)");
        println!("{:<8} {:<16} {:>16} {:>16} {:>16}", "rate", "input", "median", "worst", "mean");
        for sample_rate in [48000.0f32, 192000.0] {
            let len = (4.0 * sample_rate) as usize;
            let low = string(note_hz(33), sample_rate, 4.0);
            let high = sine(note_hz(HIGHEST), 0.2, sample_rate, len);
            let silence = vec![0.0f32; len];
            for (name, signal) in [("low A string", &low), ("highest note", &high), ("silence", &silence)] {
                let mut times = block_times_ns(signal, sample_rate);
                times.sort_by(f64::total_cmp);
                let block_ns = BLOCK as f64 / sample_rate as f64 * 1e9;
                let share = |time: f64| format!("{:.2} ({:.2} %)", time / 1000.0, 100.0 * time / block_ns);
                println!(
                    "{:<8} {:<16} {:>16} {:>16} {:>16}",
                    sample_rate,
                    name,
                    share(times[times.len() / 2]),
                    share(times[times.len() - 1]),
                    share(times.iter().sum::<f64>() / times.len() as f64)
                );
            }
        }
        println!("Switched off the tuner is not called: no time at all");
    }
}
