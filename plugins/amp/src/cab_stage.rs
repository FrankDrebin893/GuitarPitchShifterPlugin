//! How a cabinet gets from the thread that read it from a file to the audio thread.
//!
//! `CabStage` is one buffer, allocated once and as long as the longest impulse response,
//! with a state both sides agree on: empty, being written, ready, being taken. A loader
//! claims it (from empty, or from ready: a newer choice replaces one nobody took), fills
//! it and marks it ready. The audio thread, when its cabinet is free to change, claims a
//! ready one, copies the taps into the cabinet's unused slot and marks the stage empty.
//! Each side changes the state with one compare-and-swap, so the audio thread never
//! waits, and it never allocates or frees: the buffer stays where it is.
//!
//! `CabLoader` is everything around that which is not for the audio thread: which file
//! is chosen, reading it, and what the display says about it.

use crate::cab::MAX_USER_IR_LEN;
use crate::user_cab::{self, LoadError};
use std::cell::UnsafeCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;
const TAKING: u8 = 3;

struct Held {
    taps: Box<[f32]>,
    // No taps stands for the amp's own cabinet
    len: usize,
    // The rate the taps were made for
    sample_rate: f32,
}

/// The place where a loader leaves a cabinet for the audio thread
pub struct CabStage {
    state: AtomicU8,
    held: UnsafeCell<Held>,
    // One loader at a time. The audio thread never touches this
    loaders: Mutex<()>,
}

// SAFETY: `held` is only reached by the side that moved `state` to `WRITING` (one loader,
// holding `loaders`) or to `TAKING` (the audio thread), and each of the two states is left
// only by the side that entered it
unsafe impl Sync for CabStage {}

impl CabStage {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            held: UnsafeCell::new(Held {
                taps: vec![0.0; MAX_USER_IR_LEN].into_boxed_slice(),
                len: 0,
                sample_rate: 0.0,
            }),
            loaders: Mutex::new(()),
        }
    }

    /// Leaves a cabinet to be taken: an impulse response at `sample_rate`, or no taps at
    /// all for the amp's own cabinet. Replaces what was left before and not taken yet.
    /// Not for the audio thread: it may wait a moment for it
    pub fn offer(&self, taps: &[f32], sample_rate: f32) {
        let _loader = self.loaders.lock().unwrap_or_else(PoisonError::into_inner);
        let claim = |from: u8| self.state.compare_exchange(from, WRITING, Ordering::AcqRel, Ordering::Acquire).is_ok();
        // The audio thread holds the stage for as long as it takes to copy the taps
        while !claim(EMPTY) && !claim(READY) {
            std::thread::yield_now();
        }
        // SAFETY: the state is `WRITING` and this thread made it so
        let held = unsafe { &mut *self.held.get() };
        held.len = taps.len().min(held.taps.len());
        held.taps[..held.len].copy_from_slice(&taps[..held.len]);
        held.sample_rate = sample_rate;
        self.state.store(READY, Ordering::Release);
    }

    /// True when a cabinet waits to be taken
    pub fn is_ready(&self) -> bool {
        self.state.load(Ordering::Acquire) == READY
    }

    /// Gives the cabinet that waits, if one does, to `receive`: its taps (none for the
    /// amp's own cabinet) and the sample rate they are for. For the audio thread: it does
    /// not wait, allocate or free
    pub fn take(&self, receive: impl FnOnce(&[f32], f32)) -> bool {
        if self.state.compare_exchange(READY, TAKING, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        // SAFETY: the state is `TAKING` and this thread made it so
        let held = unsafe { &*self.held.get() };
        receive(&held.taps[..held.len], held.sample_rate);
        self.state.store(EMPTY, Ordering::Release);
        true
    }

    /// Address and capacity of the buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub fn buffer(&self) -> (usize, usize) {
        // SAFETY: only the address and the length are read, and neither ever changes
        let held = unsafe { &*self.held.get() };
        (held.taps.as_ptr() as usize, held.taps.len())
    }
}

/// What the display says about the chosen cabinet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CabStatus {
    /// The amp's own cabinet, or the chosen file plays, or it is being read
    Fine = 0,
    /// The chosen file is not there: the amp's own cabinet plays
    Missing = 1,
    /// The chosen file could not be used: the amp's own cabinet plays
    Bad = 2,
}

/// Reads the chosen cabinet and leaves it on the stage. Shared by the plugin, its
/// background tasks and the editor; the audio thread only ever sees the stage
pub struct CabLoader {
    stage: Arc<CabStage>,
    // Bits of the host's sample rate as f32
    sample_rate: AtomicU32,
    status: AtomicU8,
    // One load at a time, start to finish: each reads the choice and the sample rate once
    // it has its turn, so the last one to run leaves what is wanted now
    working: Mutex<()>,
}

impl CabLoader {
    pub fn new(stage: Arc<CabStage>, sample_rate: f32) -> Self {
        Self {
            stage,
            sample_rate: AtomicU32::new(sample_rate.to_bits()),
            status: AtomicU8::new(CabStatus::Fine as u8),
            working: Mutex::new(()),
        }
    }

    /// The rate cabinets are made for from now on. Follow it with a `load`
    pub fn set_sample_rate(&self, sample_rate: f32) {
        self.sample_rate.store(sample_rate.to_bits(), Ordering::Release);
    }

    pub fn status(&self) -> CabStatus {
        match self.status.load(Ordering::Relaxed) {
            1 => CabStatus::Missing,
            2 => CabStatus::Bad,
            _ => CabStatus::Fine,
        }
    }

    /// Says that a new choice was made and its `load` is on its way
    pub fn chosen(&self) {
        self.status.store(CabStatus::Fine as u8, Ordering::Relaxed);
    }

    /// What the display shows for a choice, in at most `max_chars` letters
    pub fn display_name(&self, chosen: &str, max_chars: usize) -> String {
        if chosen.is_empty() {
            return user_cab::OWN_NAME.to_owned();
        }
        match self.status() {
            CabStatus::Fine => user_cab::display_name(chosen, max_chars),
            CabStatus::Bad => user_cab::BAD_NAME.to_owned(),
            CabStatus::Missing => {
                format!("{}{}", user_cab::MISSING_MARK, user_cab::display_name(chosen, max_chars.saturating_sub(1)))
            }
        }
    }

    /// Reads the chosen file from the cabinets folder and leaves it on the stage; the
    /// amp's own cabinet if nothing is chosen or the file cannot be used. Looks for the
    /// folder and reads the disk only when a file is chosen. Not for the audio thread
    pub fn load(&self, choice: &Mutex<String>) {
        self.load_with(choice, user_cab::cabinets_dir);
    }

    /// The same with the folder given
    #[cfg(test)]
    pub fn load_from(&self, choice: &Mutex<String>, dir: Option<&std::path::Path>) {
        self.load_with(choice, || dir.map(PathBuf::from));
    }

    fn load_with(&self, choice: &Mutex<String>, dir: impl FnOnce() -> Option<PathBuf>) {
        let _working = self.working.lock().unwrap_or_else(PoisonError::into_inner);
        let chosen = choice.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let sample_rate = f32::from_bits(self.sample_rate.load(Ordering::Acquire));

        let loaded = if chosen.is_empty() {
            Ok(Vec::new())
        } else {
            match dir() {
                // A name from a saved project is not followed out of the folder
                Some(dir) if user_cab::is_file_name(&chosen) => user_cab::load(&dir.join(&chosen), sample_rate),
                _ => Err(LoadError::Missing),
            }
        };
        let (taps, status) = match loaded {
            Ok(taps) => (taps, CabStatus::Fine),
            Err(LoadError::Missing) => (Vec::new(), CabStatus::Missing),
            Err(LoadError::Bad) => (Vec::new(), CabStatus::Bad),
        };
        self.stage.offer(&taps, sample_rate);
        self.status.store(status as u8, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amp::model::Amp;
    use crate::user_cab::tests::{test_dir, whole_response, write_ir_wav};

    fn taken(stage: &CabStage) -> Option<(Vec<f32>, f32)> {
        let mut received = None;
        stage.take(|taps, sample_rate| received = Some((taps.to_vec(), sample_rate)));
        received
    }

    #[test]
    fn test_stage_hands_over_once_and_the_newest() {
        let stage = CabStage::new();
        let buffer = stage.buffer();
        assert!(!stage.is_ready());
        assert_eq!(taken(&stage), None);

        stage.offer(&[0.5, 0.25], 48000.0);
        assert!(stage.is_ready());
        assert_eq!(taken(&stage), Some((vec![0.5, 0.25], 48000.0)));
        assert!(!stage.is_ready());
        assert_eq!(taken(&stage), None);

        // Two before anyone looked: the second is the one that is wanted
        stage.offer(&[1.0; 100], 48000.0);
        stage.offer(&[0.125], 96000.0);
        assert_eq!(taken(&stage), Some((vec![0.125], 96000.0)));
        assert_eq!(taken(&stage), None);

        // No taps is a choice too, and one too long is cut to what the cabinet holds
        stage.offer(&[], 44100.0);
        assert_eq!(taken(&stage), Some((Vec::new(), 44100.0)));
        stage.offer(&vec![0.1; MAX_USER_IR_LEN + 5], 192000.0);
        assert_eq!(taken(&stage).unwrap().0.len(), MAX_USER_IR_LEN);
        assert_eq!(stage.buffer(), buffer);
    }

    #[test]
    fn test_stage_never_hands_over_a_cabinet_half_written() {
        // A loader that keeps offering against a taker that keeps taking: what arrives is
        // always one offer as a whole, and the newest one arrives last
        let stage = Arc::new(CabStage::new());
        const OFFERS: usize = 2000;
        let loader = {
            let stage = stage.clone();
            std::thread::spawn(move || {
                for offer in 1..=OFFERS {
                    let len = 1 + offer % 4000;
                    stage.offer(&vec![offer as f32; len], len as f32);
                }
            })
        };

        let mut last = 0.0;
        let mut takes = 0;
        while last < OFFERS as f32 {
            stage.take(|taps, sample_rate| {
                assert_eq!(taps.len() as f32, sample_rate);
                assert!(taps.iter().all(|&tap| tap == taps[0]), "Taps of two offers");
                assert!(taps[0] > last, "Offer {} after {}", taps[0], last);
                last = taps[0];
                takes += 1;
            });
        }
        loader.join().unwrap();
        assert!(takes >= 1 && !stage.is_ready());
    }

    #[test]
    fn test_loader_leaves_the_chosen_file_or_the_amps_own() {
        let dir = test_dir("loader");
        write_ir_wav(&dir.join("good.wav"), &whole_response(Amp::Brol, 48000.0, 0.02, 0.5), 1, 16, false, 48000);
        std::fs::write(dir.join("bad.wav"), b"not a recording").unwrap();

        let stage = Arc::new(CabStage::new());
        let loader = CabLoader::new(stage.clone(), 44100.0);
        loader.set_sample_rate(48000.0);
        let choice = Mutex::new(String::new());
        let choose = |name: &str| {
            *choice.lock().unwrap() = name.to_owned();
            loader.chosen();
            loader.load_from(&choice, Some(&dir));
            taken(&stage).expect("Nothing was left on the stage")
        };

        // Nothing chosen: the amp's own, which is no taps
        assert_eq!(choose(""), (Vec::new(), 48000.0));
        assert_eq!((loader.status(), loader.display_name("", 10).as_str()), (CabStatus::Fine, "OWN"));

        let (taps, sample_rate) = choose("good.wav");
        assert!(taps.len() > 100 && sample_rate == 48000.0);
        assert_eq!((loader.status(), loader.display_name("good.wav", 10).as_str()), (CabStatus::Fine, "GOOD"));

        // A file that cannot be used and one that is not there: the amp's own, and the
        // display says which of the two it is
        assert_eq!(choose("bad.wav"), (Vec::new(), 48000.0));
        assert_eq!((loader.status(), loader.display_name("bad.wav", 10).as_str()), (CabStatus::Bad, "BAD FILE"));
        assert_eq!(choose("gone.wav"), (Vec::new(), 48000.0));
        assert_eq!((loader.status(), loader.display_name("gone.wav", 10).as_str()), (CabStatus::Missing, "?GONE"));
        assert_eq!(loader.display_name("a very long name.wav", 10), "?A VE~NAME");
        assert_eq!(loader.display_name("a very long name.wav", 10).chars().count(), 10);

        // A name that leads out of the folder is not followed, though the file is there
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        assert_eq!(choose("inner/../good.wav"), (Vec::new(), 48000.0));
        assert_eq!(loader.status(), CabStatus::Missing);
        // Nor is anything read when there is no folder to read from
        *choice.lock().unwrap() = "good.wav".to_owned();
        loader.load_from(&choice, None);
        assert_eq!((taken(&stage), loader.status()), (Some((Vec::new(), 48000.0)), CabStatus::Missing));

        // The same file at another sample rate comes out at that rate
        loader.set_sample_rate(96000.0);
        let (resampled, sample_rate) = choose("good.wav");
        assert_eq!(sample_rate, 96000.0);
        assert!(resampled.len() > taps.len() * 3 / 2);
    }
}
