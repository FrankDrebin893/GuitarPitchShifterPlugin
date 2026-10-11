//! Two lamps for playing live: whether the gate is open, and whether the output reaches the
//! safety clip. The audio thread leaves what it knows in two atomics once per block, and the
//! editor reads them when it draws. Neither waits for the other, and the sound is the same
//! whether anyone looks or not.

use std::sync::atomic::{AtomicU32, Ordering};

// How long the clip lamp stays lit after the output was last over the knee: long enough to
// see a single peak
const CLIP_HOLD_S: f64 = 0.15;

// The gate counts as open while it lets through at least this much: from when it has
// opened, through the hold, until the release has taken 6 dB
const GATE_OPEN_GAIN: f32 = 0.5;

// The lamps are looked at with every frame the window gets, and drawn anew when they have
// changed, this often at most
const REPAINT_S: f64 = 0.05;

/// What the chain tells the editor about itself
pub struct LampReading {
    // Bits of the gate's gain as f32, 0.0 closed to 1.0 open
    gate: AtomicU32,
    // How many blocks had a sample over the knee of the safety clip. Goes round
    clips: AtomicU32,
}

impl LampReading {
    pub fn new() -> Self {
        Self {
            gate: AtomicU32::new(0.0f32.to_bits()),
            clips: AtomicU32::new(0),
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
        Self {
            seen: clips,
            lit_until: f64::NEG_INFINITY,
        }
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

/// The two lamps as the window shows them. Belongs to the editor
pub struct LampView {
    clip: ClipHold,
    // What is drawn: whether the gate's lamp is lit, and the clip lamp
    shown: (bool, bool),
    shown_at: f64,
}

impl LampView {
    /// For a window that opens now: dark until it has looked
    pub fn new(reading: &LampReading) -> Self {
        Self {
            clip: ClipHold::new(reading.clips()),
            shown: (false, false),
            shown_at: f64::NEG_INFINITY,
        }
    }

    /// Looks at the reading at `now`, in the window's seconds. True when the lamps are no
    /// longer as they were drawn and the window has to be painted again: 20 times a second
    /// at most, however fast they change
    pub fn look(&mut self, reading: &LampReading, gate_on: bool, now: f64) -> bool {
        let lit = self.clip.lit(reading.clips(), now);
        let seen = (gate_open(gate_on, reading.gate_gain()), lit);
        let changed = seen != self.shown && now - self.shown_at >= REPAINT_S;
        if changed {
            self.shown = seen;
            self.shown_at = now;
        }
        changed
    }

    /// Whether the gate's lamp and the clip lamp are lit, as of the last look that asked
    /// for a repaint
    pub fn shown(&self) -> (bool, bool) {
        self.shown
    }
}

#[cfg(test)]
mod tests {
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
