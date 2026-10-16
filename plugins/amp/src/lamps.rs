//! Two lamps for playing live: whether the gate is open, and whether the output reaches the
//! safety clip, and two level meters, for the input and the output. The audio thread leaves
//! what it knows in atomics once per block, and the editor reads them when it draws. Neither waits for the other, and the sound is the same
//! whether anyone looks or not.

use std::sync::atomic::{AtomicU32, Ordering};
use suite_common::ui::{lit_level_lamps, LEVEL_MIN_DB};

// How long the clip lamp stays lit after the output was last over the knee: long enough to
// see a single peak
const CLIP_HOLD_S: f64 = 0.15;

// The gate counts as open while it lets through at least this much: from when it has
// opened, through the hold, until the release has taken 6 dB
const GATE_OPEN_GAIN: f32 = 0.5;

// A meter lets go of a level this fast, in dB a second: 18 dB in 300 ms. It takes a new peak
// at once
const METER_FALL_DB_S: f64 = 60.0;
// The meters are drawn anew when a lamp has changed, this often at most
const METER_REPAINT_S: f64 = 1.0 / 30.0;

// The lamps are looked at with every frame the window gets, and drawn anew when they have
// changed, this often at most
const REPAINT_S: f64 = 0.05;

/// What the chain tells the editor about itself
pub struct LampReading {
    // Bits of the gate's gain as f32, 0.0 closed to 1.0 open
    gate: AtomicU32,
    // How many blocks had a sample over the knee of the safety clip. Goes round
    clips: AtomicU32,
    // Bits of the largest absolute sample of the input and of the output since the editor
    // last took them, as f32. Non-negative floats order like their bits, so `fetch_max`
    // keeps the largest
    input_peak: AtomicU32,
    output_peak: AtomicU32,
}

impl LampReading {
    pub fn new() -> Self {
        Self {
            gate: AtomicU32::new(0.0f32.to_bits()),
            clips: AtomicU32::new(0),
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
        }
    }

    /// The gain the gate was at when the last block ended: 0.0 closed, 1.0 open, between
    /// the two while it opens or closes. 1.0 for a gate that is switched off
    pub fn gate_gain(&self) -> f32 {
        f32::from_bits(self.gate.load(Ordering::Relaxed))
    }

    /// Changes whenever a block went over the knee of the safety clip
    pub fn clips(&self) -> u32 {
        self.clips.load(Ordering::Relaxed)
    }

    pub fn set_gate_gain(&self, gain: f32) {
        self.gate.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// The largest absolute samples of a piece of the input and of the output. Only numbers
    /// that can be shown are kept
    pub fn note_peaks(&self, input: f32, output: f32) {
        for (atomic, peak) in [(&self.input_peak, input), (&self.output_peak, output)] {
            if peak.is_finite() && peak > 0.0 {
                atomic.fetch_max(peak.to_bits(), Ordering::Relaxed);
            }
        }
    }

    /// The largest absolute samples of the input and the output since the last time, and
    /// starts over
    pub fn take_peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.input_peak.swap(0, Ordering::Relaxed)),
            f32::from_bits(self.output_peak.swap(0, Ordering::Relaxed)),
        )
    }

    /// Only the audio thread counts, so reading and writing apart loses nothing
    pub fn count_clip(&self) {
        self.clips.store(self.clips.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed);
    }
}

/// Whether the Gate pedal's lamp is lit: while the gate is switched on and lets the guitar
/// through. A gate that is switched off does nothing, and its lamp is dark like its LED
pub fn gate_open(on: bool, gain: f32) -> bool {
    on && gain >= GATE_OPEN_GAIN
}

/// Keeps the clip lamp lit for a moment after each clip. Belongs to the editor: time is
/// what the window says it is, in seconds
pub struct ClipHold {
    seen: u32,
    lit_until: f64,
}

impl ClipHold {
    /// `clips` is the count as it stands: what was clipped before now does not light the lamp
    pub fn new(clips: u32) -> Self {
        Self { seen: clips, lit_until: f64::NEG_INFINITY }
    }

    /// Whether the lamp is lit at `now`, given the count as it stands
    pub fn lit(&mut self, clips: u32, now: f64) -> bool {
        if clips != self.seen {
            self.seen = clips;
            self.lit_until = now + CLIP_HOLD_S;
        }
        now < self.lit_until
    }
}

/// The level a meter shows: a new peak at once, and then it falls. Belongs to the editor
struct Ballistics {
    db: f32,
    at: f64,
}

impl Ballistics {
    fn new() -> Self {
        Self { db: LEVEL_MIN_DB, at: f64::NEG_INFINITY }
    }

    /// The level at `now` given the largest sample since the last time, in dBFS
    fn follow(&mut self, peak: f32, now: f64) -> f32 {
        let fallen =
            if self.at.is_finite() { METER_FALL_DB_S * (now - self.at).max(0.0) } else { 0.0 };
        let held = (self.db as f64 - fallen).max(LEVEL_MIN_DB as f64) as f32;
        let heard = if peak > 0.0 { (20.0 * peak.log10()).max(LEVEL_MIN_DB) } else { LEVEL_MIN_DB };
        self.db = held.max(heard);
        self.at = now;
        self.db
    }
}

/// The two lamps and the two meters as the window shows them. Belongs to the editor
pub struct LampView {
    clip: ClipHold,
    // What is drawn: whether the gate's lamp is lit, and the clip lamp
    shown: (bool, bool),
    shown_at: f64,
    // The input and the output meter: how they fall, and the level that is drawn
    levels: [Ballistics; 2],
    shown_levels: [f32; 2],
    levels_shown_at: f64,
}

impl LampView {
    /// For a window that opens now: dark until it has looked
    pub fn new(reading: &LampReading) -> Self {
        // What was played while the window was closed is not news
        reading.take_peaks();
        Self {
            clip: ClipHold::new(reading.clips()),
            shown: (false, false),
            shown_at: f64::NEG_INFINITY,
            levels: [Ballistics::new(), Ballistics::new()],
            shown_levels: [LEVEL_MIN_DB; 2],
            levels_shown_at: f64::NEG_INFINITY,
        }
    }

    /// Looks at the reading at `now`, in the window's seconds. True when the lamps are no
    /// longer as they were drawn and the window has to be painted again: 20 times a second
    /// at most, however fast they change
    pub fn look(&mut self, reading: &LampReading, gate_on: bool, now: f64) -> bool {
        let lit = self.clip.lit(reading.clips(), now);
        let seen = (gate_open(gate_on, reading.gate_gain()), lit);
        let mut changed = seen != self.shown && now - self.shown_at >= REPAINT_S;
        if changed {
            self.shown = seen;
            self.shown_at = now;
        }

        let (input, output) = reading.take_peaks();
        let levels = [self.levels[0].follow(input, now), self.levels[1].follow(output, now)];
        let moved = levels
            .iter()
            .zip(&self.shown_levels)
            .any(|(&new, &old)| lit_level_lamps(new) != lit_level_lamps(old));
        if moved && now - self.levels_shown_at >= METER_REPAINT_S {
            self.shown_levels = levels;
            self.levels_shown_at = now;
            changed = true;
        }
        changed
    }

    /// The levels of the input and the output in dBFS as of the last look that asked for
    /// a repaint
    pub fn levels(&self) -> [f32; 2] {
        self.shown_levels
    }

    /// Whether the gate's lamp and the clip lamp are lit, as of the last look that asked
    /// for a repaint
    pub fn shown(&self) -> (bool, bool) {
        self.shown
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_meters_take_a_peak_at_once_fall_and_then_leave_the_window_alone() {
        let reading = LampReading::new();
        let mut view = LampView::new(&reading);
        for frame in 0..600 {
            assert!(!view.look(&reading, true, frame as f64 / 60.0));
        }
        assert_eq!(view.levels(), [LEVEL_MIN_DB; 2]);

        // Half scale at the input and a tenth at the output, with a quieter piece after
        reading.note_peaks(0.5, 0.1);
        reading.note_peaks(0.25, 0.01);
        assert!(view.look(&reading, true, 10.0));
        let [input, output] = view.levels();
        assert!((input + 6.02).abs() < 0.01 && (output + 20.0).abs() < 0.01, "{input} {output}");

        // It falls 60 dB a second: 18 dB in 300 ms
        assert!(view.look(&reading, true, 10.3));
        let [input, _] = view.levels();
        assert!((input - (-6.02 - 18.0)).abs() < 0.01, "{input}");
        // Until it is silent, and then nothing needs painting
        assert!(view.look(&reading, true, 12.0));
        assert_eq!(view.levels(), [LEVEL_MIN_DB; 2]);
        assert!(!view.look(&reading, true, 12.5));
    }

    #[test]
    fn test_peaks_keep_the_largest_and_ignore_what_cannot_be_shown() {
        let reading = LampReading::new();
        reading.note_peaks(0.2, 0.9);
        reading.note_peaks(0.7, f32::NAN);
        reading.note_peaks(f32::INFINITY, 0.3);
        assert_eq!(reading.take_peaks(), (0.7, 0.9));
        assert_eq!(reading.take_peaks(), (0.0, 0.0));
    }

    use super::*;

    #[test]
    fn test_window_repaints_when_a_lamp_changes_and_not_otherwise() {
        let reading = LampReading::new();
        let mut view = LampView::new(&reading);
        // Frame after frame of nothing new: nothing to paint
        for frame in 0..600 {
            assert!(!view.look(&reading, true, frame as f64 / 60.0));
        }
        assert_eq!(view.shown(), (false, false));

        // The gate opens and the output clips: one repaint for both
        reading.set_gate_gain(1.0);
        reading.count_clip();
        assert!(view.look(&reading, true, 10.0));
        assert_eq!(view.shown(), (true, true));
        assert!(!view.look(&reading, true, 10.01));

        // The clip lamp goes out by itself, which is a repaint as well
        assert!(!view.look(&reading, true, 10.0 + 0.9 * CLIP_HOLD_S));
        assert!(view.look(&reading, true, 10.0 + 1.1 * CLIP_HOLD_S));
        assert_eq!(view.shown(), (true, false));

        // Switching the gate off darkens its lamp, though an idle gate lets everything through
        assert!(view.look(&reading, false, 11.0));
        assert_eq!(view.shown(), (false, false));
    }

    #[test]
    fn test_lamps_that_flicker_repaint_twenty_times_a_second_at_most() {
        let reading = LampReading::new();
        let mut view = LampView::new(&reading);
        // The gate opens and closes with every frame at 60 frames a second, for a second
        let mut repaints = 0;
        for frame in 0..60 {
            reading.set_gate_gain(((frame + 1) % 2) as f32);
            repaints += view.look(&reading, true, frame as f64 / 60.0) as u32;
        }
        assert!((10..=20).contains(&repaints), "{repaints} repaints");

        // What is shown catches up once it stops, within the interval
        reading.set_gate_gain(1.0);
        view.look(&reading, true, 1.0);
        view.look(&reading, true, 1.0 + REPAINT_S);
        assert_eq!(view.shown(), (true, false));
    }

    #[test]
    fn test_window_opened_after_clipping_starts_dark() {
        let reading = LampReading::new();
        reading.count_clip();
        let mut view = LampView::new(&reading);
        assert!(!view.look(&reading, true, 0.0));
        assert_eq!(view.shown(), (false, false));
    }

    #[test]
    fn test_reading_starts_closed_and_unclipped_and_gives_back_what_it_is_told() {
        let reading = LampReading::new();
        assert_eq!((reading.gate_gain(), reading.clips()), (0.0, 0));
        reading.set_gate_gain(0.25);
        reading.count_clip();
        reading.count_clip();
        assert_eq!((reading.gate_gain(), reading.clips()), (0.25, 2));
    }

    #[test]
    fn test_clip_count_goes_round() {
        let reading = LampReading::new();
        reading.clips.store(u32::MAX, Ordering::Relaxed);
        reading.count_clip();
        assert_eq!(reading.clips(), 0);

        // The lamp only asks whether the count changed
        let mut hold = ClipHold::new(u32::MAX);
        assert!(hold.lit(0, 1.0));
    }

    #[test]
    fn test_gate_lamp_is_lit_while_a_gate_that_is_on_lets_the_guitar_through() {
        assert!(gate_open(true, 1.0));
        assert!(!gate_open(true, 0.0));
        // Through the first 6 dB of the release, then dark
        assert!(gate_open(true, 0.5));
        assert!(!gate_open(true, 0.49));
        // Switched off the gate is fully open and its lamp is dark
        assert!(!gate_open(false, 1.0));
        assert!(!gate_open(false, 0.0));
    }

    #[test]
    fn test_clip_lamp_is_held_after_a_clip_and_goes_out() {
        let mut hold = ClipHold::new(0);
        assert!(!hold.lit(0, 0.0));
        assert!(!hold.lit(0, 10.0));

        // One clipped block: lit at once and for the hold, then out
        assert!(hold.lit(1, 10.0));
        assert!(hold.lit(1, 10.0 + 0.9 * CLIP_HOLD_S));
        assert!(!hold.lit(1, 10.0 + 1.1 * CLIP_HOLD_S));
        assert!(!hold.lit(1, 20.0));

        // Clipping that goes on keeps it lit, and the hold counts from the last of it
        assert!(hold.lit(2, 20.0));
        assert!(hold.lit(5, 20.1));
        assert!(hold.lit(5, 20.1 + 0.9 * CLIP_HOLD_S));
        assert!(!hold.lit(5, 20.1 + 1.1 * CLIP_HOLD_S));
    }

    #[test]
    fn test_clips_from_before_the_window_opened_do_not_light_the_lamp() {
        let mut hold = ClipHold::new(41);
        assert!(!hold.lit(41, 0.0));
        assert!(hold.lit(42, 0.5));
    }
}
