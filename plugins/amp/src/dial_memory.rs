//! Each amp remembers where its six dials stood when the player last left it with the
//! selector on the head.
//!
//! The dials themselves are six parameters shared by the amps, and stay that: hosts and
//! automation see nothing of this. The memory belongs to the editor. When an amp switch is
//! clicked, the six values go into the memory under the amp that is left, and the six
//! parameters are set to what the memory holds for the amp that is entered. An amp that was
//! never left holds the defaults.
//!
//! What the editor does not do, it does not see:
//! - A host that changes Amp by itself (automation, its own presets, undo) leaves the dials
//!   where they are: they now belong to the amp it picked. The memory is not told, and does
//!   not need to be: the next click on the selector stores the dials under whatever amp is
//!   selected then
//! - A factory preset sets the amp and the dials. The amp it leaves is remembered as it
//!   stood, and the memory of the preset's amp becomes the preset's values
//! - A click on the amp that is already selected does nothing
//!
//! The audio thread never reads the memory. A host may restore a saved state from the audio
//! thread though, so the values are atomics: nothing waits for a lock there.

use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{Amp, GuitarAmpParams};

/// Gain, Bass, Mid, Treble, Presence, Master, as the parameters store them: 0.0 to 1.0
pub type AmpDials = [f32; 6];

pub struct DialMemory {
    // What an amp holds until it has been left once
    defaults: AmpDials,
    // Per amp in the order of `Amp::ALL`, the bits of each value
    stored: [[AtomicU32; 6]; Amp::ALL.len()],
}

impl DialMemory {
    pub fn new(defaults: AmpDials) -> Self {
        Self {
            defaults,
            stored: std::array::from_fn(|_| defaults.map(|dial| AtomicU32::new(dial.to_bits()))),
        }
    }

    /// The amp is switched away from with its dials at `dials`
    pub fn leave(&self, amp: Amp, dials: AmpDials) {
        for (stored, dial) in self.stored[amp.index()].iter().zip(dials) {
            stored.store(dial.to_bits(), Ordering::Relaxed);
        }
    }

    /// Where the dials go when the amp is switched to: where they stood when it was left.
    /// A value no dial can stand at (from a damaged state) gives that dial's default
    pub fn enter(&self, amp: Amp) -> AmpDials {
        let stored = &self.stored[amp.index()];
        std::array::from_fn(|index| {
            let dial = f32::from_bits(stored[index].load(Ordering::Relaxed));
            if (0.0..=1.0).contains(&dial) {
                dial
            } else {
                self.defaults[index]
            }
        })
    }
}

/// Saved with the project as a list of six values per amp, in the order of `Amp::ALL`. A
/// state from a build with fewer amps leaves the others at what they hold, and amps this
/// build does not have are dropped
impl PersistentField<'_, Vec<AmpDials>> for DialMemory {
    fn set(&self, new_value: Vec<AmpDials>) {
        for (amp, dials) in Amp::ALL.into_iter().zip(new_value) {
            self.leave(amp, dials);
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Vec<AmpDials>) -> R,
    {
        f(&Amp::ALL.map(|amp| self.enter(amp)).to_vec())
    }
}

/// The amp selector on the head was clicked: the amp that is left remembers its dials, and
/// the dials go to where the amp that is entered had them. Told to the host like any knob
/// movement: seven changes, the amp first. The chain fades the amp out before it switches
/// and starts the new one at its dials, so they are not heard moving
pub fn switch_amp(params: &GuitarAmpParams, setter: &ParamSetter, to: Amp) {
    let from = params.amp.value();
    if from == to {
        return;
    }
    let dials = params.amp_dials();
    params.dial_memory.leave(from, dials.map(|dial| dial.unmodulated_plain_value()));

    setter.begin_set_parameter(&params.amp);
    setter.set_parameter(&params.amp, to);
    setter.end_set_parameter(&params.amp);
    for (dial, value) in dials.into_iter().zip(params.dial_memory.enter(to)) {
        setter.begin_set_parameter(dial);
        setter.set_parameter(dial, value);
        setter.end_set_parameter(dial);
    }
}

/// A preset for `amp` with its dials at `dials` is about to be loaded: the amp that is left
/// for it remembers its dials, and the preset's amp remembers the preset's
pub fn preset_loaded(params: &GuitarAmpParams, amp: Amp, dials: AmpDials) {
    let from = params.amp.value();
    if from != amp {
        params.dial_memory.leave(from, params.amp_dials().map(|dial| dial.unmodulated_plain_value()));
    }
    params.dial_memory.leave(amp, dials);
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: AmpDials = [0.5; 6];
    const KLAR: AmpDials = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
    const TORDEN: AmpDials = [0.9, 0.35, 0.7, 0.65, 0.8, 0.45];

    #[test]
    fn test_amp_never_left_holds_the_defaults() {
        let memory = DialMemory::new([0.5, 0.4, 0.3, 0.2, 0.1, 0.0]);
        for amp in Amp::ALL {
            assert_eq!(memory.enter(amp), [0.5, 0.4, 0.3, 0.2, 0.1, 0.0]);
        }
    }

    #[test]
    fn test_each_amp_gets_back_what_it_was_left_with() {
        let memory = DialMemory::new(DEFAULTS);
        // Klar is set up and left for Torden, Torden is set up and left for Klar
        memory.leave(Amp::Klar, KLAR);
        assert_eq!(memory.enter(Amp::Torden), DEFAULTS);
        memory.leave(Amp::Torden, TORDEN);
        assert_eq!(memory.enter(Amp::Klar), KLAR);

        // Changed and left again: the newer positions are the ones kept, to the bit
        let changed = [0.15, 0.2, 0.3, 0.4, 0.5, 0.6000001];
        memory.leave(Amp::Klar, changed);
        assert_eq!(memory.enter(Amp::Torden), TORDEN);
        assert_eq!(memory.enter(Amp::Klar), changed);
        assert_eq!(memory.enter(Amp::Brol), DEFAULTS);
    }

    #[test]
    fn test_entering_does_not_change_the_memory() {
        let memory = DialMemory::new(DEFAULTS);
        memory.leave(Amp::Brol, KLAR);
        for _ in 0..3 {
            assert_eq!(memory.enter(Amp::Brol), KLAR);
        }
    }

    #[test]
    fn test_values_no_dial_can_stand_at_give_the_default() {
        let memory = DialMemory::new(DEFAULTS);
        memory.leave(Amp::Klar, [f32::NAN, -0.1, 1.5, f32::INFINITY, 0.0, 1.0]);
        assert_eq!(memory.enter(Amp::Klar), [0.5, 0.5, 0.5, 0.5, 0.0, 1.0]);
    }

    #[test]
    fn test_saved_and_restored_it_is_the_same() {
        let memory = DialMemory::new(DEFAULTS);
        memory.leave(Amp::Klar, KLAR);
        memory.leave(Amp::Torden, TORDEN);
        let saved = memory.map(|stored| stored.clone());
        assert_eq!(saved, vec![KLAR, DEFAULTS, TORDEN]);

        let restored = DialMemory::new(DEFAULTS);
        restored.set(saved);
        for amp in Amp::ALL {
            assert_eq!(restored.enter(amp), memory.enter(amp));
        }
    }

    #[test]
    fn test_state_with_fewer_or_more_amps_is_taken_as_far_as_it_goes() {
        let memory = DialMemory::new(DEFAULTS);
        memory.leave(Amp::Torden, TORDEN);
        memory.set(vec![KLAR]);
        assert_eq!(memory.enter(Amp::Klar), KLAR);
        assert_eq!(memory.enter(Amp::Brol), DEFAULTS);
        assert_eq!(memory.enter(Amp::Torden), TORDEN);

        memory.set(vec![DEFAULTS, KLAR, DEFAULTS, TORDEN, TORDEN]);
        assert_eq!(memory.enter(Amp::Klar), DEFAULTS);
        assert_eq!(memory.enter(Amp::Brol), KLAR);
        assert_eq!(memory.enter(Amp::Torden), DEFAULTS);
    }

    #[test]
    fn test_the_parameters_memory_starts_at_the_dials_defaults() {
        let params = GuitarAmpParams::default();
        let defaults = params.amp_dials().map(|dial| dial.default_plain_value());
        for amp in Amp::ALL {
            assert_eq!(params.dial_memory.enter(amp), defaults);
        }
    }

    #[test]
    fn test_loading_a_preset_remembers_the_amp_left_and_the_presets_dials() {
        // The parameters stand at their defaults, on Brøl. A Torden preset comes in
        let params = GuitarAmpParams::default();
        params.dial_memory.leave(Amp::Brol, KLAR);
        preset_loaded(&params, Amp::Torden, TORDEN);
        assert_eq!(params.dial_memory.enter(Amp::Torden), TORDEN);
        // Brøl is remembered as it stood (the defaults), not as it was left some time before
        assert_eq!(params.dial_memory.enter(Amp::Brol), DEFAULTS);

        // A preset for the amp that is selected replaces its memory
        preset_loaded(&params, Amp::Brol, KLAR);
        assert_eq!(params.dial_memory.enter(Amp::Brol), KLAR);
        assert_eq!(params.dial_memory.enter(Amp::Torden), TORDEN);
    }
}
