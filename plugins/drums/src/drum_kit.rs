use std::f32::consts::FRAC_PI_4;

use crate::dsp::{bus_clip, semitones_to_ratio};
use crate::engines::groove::GrooveKit;
use crate::engines::metal::MetalKit;
use crate::kit::{Kit, KitPreset};
use crate::reverb::Room;
use crate::voices::cymbal::Cymbal;
use crate::voices::kick::Kick;
use crate::voices::snare::Snare;
use crate::voices::tom::{Tom, TOM_COUNT};
use crate::voices::{Hit, VoicePair};

// Cymbal slots
const HAT: usize = 0;
const CRASH1: usize = 1;
const CRASH2: usize = 2;
const RIDE: usize = 3;
const BELL: usize = 4;
const CHINA: usize = 5;
const SPLASH: usize = 6;
const CYMBAL_COUNT: usize = 7;

// Stereo positions from the drummer's seat, -1.0 (left) to 1.0 (right)
const CYMBAL_PANS: [f32; CYMBAL_COUNT] = [-0.45, -0.5, 0.5, 0.4, 0.4, 0.65, -0.15];
// Lowest (floor) tom to highest
const TOM_PANS: [f32; TOM_COUNT] = [0.5, 0.35, 0.15, -0.05, -0.2, -0.35];

// Balance between the pieces with all level controls at 0 dB
const KICK_GAIN: f32 = 0.5;
const SNARE_GAIN: f32 = 0.52;
const TOM_GAIN: f32 = 0.45;
const HAT_GAIN: f32 = 0.26;
const CYMBAL_GAIN: f32 = 0.26;

// How much of each piece goes to the room
const KICK_SEND: f32 = 0.15;
const SNARE_SEND: f32 = 0.5;
const TOM_SEND: f32 = 0.45;
const HAT_SEND: f32 = 0.25;
const CYMBAL_SEND: f32 = 0.35;

// Share of the decay time that full Damping takes away
const DAMPING_RANGE: f32 = 0.75;

/// Linear levels per group, and the room amount (0.0-1.0)
pub struct Mix {
    pub kick: f32,
    pub snare: f32,
    pub toms: f32,
    pub hihat: f32,
    pub cymbals: f32,
    pub room: f32,
}

impl Default for Mix {
    fn default() -> Self {
        Self { kick: 1.0, snare: 1.0, toms: 1.0, hihat: 1.0, cymbals: 1.0, room: 0.25 }
    }
}

#[derive(Default)]
struct Bus {
    left: f32,
    right: f32,
    send_left: f32,
    send_right: f32,
}

impl Bus {
    fn add(&mut self, sample: f32, pan: (f32, f32), send: f32) {
        let left = sample * pan.0;
        let right = sample * pan.1;
        self.left += left;
        self.right += right;
        self.send_left += left * send;
        self.send_right += right * send;
    }
}

/// The whole instrument: every voice, the General MIDI note map, the stereo mix and the room.
/// Rock and Jazz are played by the modal voices here, Metal and Groove by their own engines.
/// Nothing here allocates after `set_sample_rate`.
pub struct DrumKit {
    // None for the kits that have an engine of their own
    presets: [Option<KitPreset>; 4],
    kit: Kit,
    tune_semitones: f32,
    tune_ratio: f32,
    decay_scale: f32,
    pub mix: Mix,
    kick: VoicePair<Kick>,
    snare: VoicePair<Snare>,
    toms: [VoicePair<Tom>; TOM_COUNT],
    cymbals: [Cymbal; CYMBAL_COUNT],
    center_pan: (f32, f32),
    tom_pans: [(f32, f32); TOM_COUNT],
    cymbal_pans: [(f32, f32); CYMBAL_COUNT],
    room: Room,
    // Of the modal kit that was played last: its voices may still ring under another kit
    room_level: f32,
    metal: MetalKit,
    groove: GrooveKit,
}

impl DrumKit {
    pub fn new() -> Self {
        let presets = Kit::ALL.map(KitPreset::new);
        let rock = presets[Kit::Rock as usize].as_ref().expect("Rock is a modal kit");
        let room_level = rock.room_level;
        let mut room = Room::new();
        room.set_decay(rock.room_decay_s);

        // Every voice gets its own noise seed
        Self {
            presets,
            kit: Kit::Rock,
            tune_semitones: 0.0,
            tune_ratio: 1.0,
            decay_scale: 1.0,
            mix: Mix::default(),
            kick: VoicePair::new(Kick::new(1), Kick::new(2)),
            snare: VoicePair::new(Snare::new(3), Snare::new(4)),
            toms: std::array::from_fn(|i| {
                VoicePair::new(Tom::new(10 + 2 * i as u32), Tom::new(11 + 2 * i as u32))
            }),
            cymbals: std::array::from_fn(|i| Cymbal::new(30 + i as u32)),
            center_pan: pan_gains(0.0),
            tom_pans: TOM_PANS.map(pan_gains),
            cymbal_pans: CYMBAL_PANS.map(pan_gains),
            room,
            room_level,
            metal: MetalKit::new(50),
            groove: GrooveKit::new(51),
        }
    }

    /// Allocates the rooms' delay lines. Not for the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.kick.set_sample_rate(sample_rate);
        self.snare.set_sample_rate(sample_rate);
        for tom in &mut self.toms {
            tom.set_sample_rate(sample_rate);
        }
        for cymbal in &mut self.cymbals {
            cymbal.set_sample_rate(sample_rate);
        }
        self.room.set_sample_rate(sample_rate);
        self.metal.set_sample_rate(sample_rate);
        self.groove.set_sample_rate(sample_rate);
    }

    pub fn reset(&mut self) {
        self.kick.reset();
        self.snare.reset();
        for tom in &mut self.toms {
            tom.reset();
        }
        for cymbal in &mut self.cymbals {
            cymbal.reset();
        }
        self.room.reset();
        self.metal.reset();
        self.groove.reset();
    }

    /// Applies to the next hits. Voices that are ringing keep the sound they were hit with.
    pub fn set_kit(&mut self, kit: Kit) {
        if kit != self.kit {
            self.kit = kit;
            if let Some(preset) = &self.presets[kit as usize] {
                self.room.set_decay(preset.room_decay_s);
                self.room_level = preset.room_level;
            }
        }
    }

    /// Tuning of the drums (not the cymbals) for the next hits
    pub fn set_tune(&mut self, semitones: f32) {
        if semitones != self.tune_semitones {
            self.tune_semitones = semitones;
            self.tune_ratio = semitones_to_ratio(semitones);
        }
    }

    /// 0.0 lets everything ring, 1.0 is heavily damped. Applies to the next hits.
    pub fn set_damping(&mut self, damping: f32) {
        self.decay_scale = 1.0 - DAMPING_RANGE * damping.clamp(0.0, 1.0);
    }

    /// General MIDI drum map. Velocity is 0.0-1.0.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        if velocity <= 0.0 {
            return;
        }

        let hit =
            Hit { velocity: velocity.min(1.0), tune: self.tune_ratio, decay: self.decay_scale };
        let preset = match (self.kit, &self.presets[self.kit as usize]) {
            (_, Some(preset)) => preset,
            (Kit::Groove, None) => return self.groove.note_on(note, &hit),
            (_, None) => return self.metal.note_on(note, &hit),
        };
        let cymbals = &preset.cymbals;

        match note {
            35 | 36 => self.kick.next_voice().trigger(&preset.kick, &hit),
            37 => self.snare.next_voice().trigger_side_stick(&hit),
            38 | 40 => self.snare.next_voice().trigger(&preset.snare, &hit),
            41 | 43 | 45 | 47 | 48 | 50 => {
                let index = tom_index(note);
                self.toms[index].next_voice().trigger(&preset.toms, index, &hit);
            }
            // The closed and pedal sounds have a short decay, which chokes an open hi-hat
            42 => self.cymbals[HAT].trigger(&cymbals.hat_closed, &hit),
            44 => self.cymbals[HAT].trigger(&cymbals.hat_pedal, &hit),
            46 => self.cymbals[HAT].trigger(&cymbals.hat_open, &hit),
            49 => self.cymbals[CRASH1].trigger(&cymbals.crash1, &hit),
            57 => self.cymbals[CRASH2].trigger(&cymbals.crash2, &hit),
            51 | 59 => self.cymbals[RIDE].trigger(&cymbals.ride, &hit),
            53 => self.cymbals[BELL].trigger(&cymbals.bell, &hit),
            52 => self.cymbals[CHINA].trigger(&cymbals.china, &hit),
            55 => self.cymbals[SPLASH].trigger(&cymbals.splash, &hit),
            _ => {}
        }
    }

    /// One stereo frame
    pub fn process(&mut self) -> (f32, f32) {
        let mix = &self.mix;
        let mut bus = Bus::default();

        bus.add(self.kick.process_sample() * mix.kick * KICK_GAIN, self.center_pan, KICK_SEND);
        bus.add(self.snare.process_sample() * mix.snare * SNARE_GAIN, self.center_pan, SNARE_SEND);
        for (tom, pan) in self.toms.iter_mut().zip(self.tom_pans) {
            bus.add(tom.process_sample() * mix.toms * TOM_GAIN, pan, TOM_SEND);
        }
        for (slot, (cymbal, pan)) in self.cymbals.iter_mut().zip(self.cymbal_pans).enumerate() {
            let (gain, send) = if slot == HAT {
                (mix.hihat * HAT_GAIN, HAT_SEND)
            } else {
                (mix.cymbals * CYMBAL_GAIN, CYMBAL_SEND)
            };
            bus.add(cymbal.process_sample() * gain, pan, send);
        }

        let (wet_left, wet_right) = self.room.process(bus.send_left, bus.send_right);
        let room_gain = mix.room * self.room_level;

        // An engine that is not ringing costs nothing
        let (metal_left, metal_right) = self.metal.process(mix);
        let (groove_left, groove_right) = self.groove.process(mix);

        // Single hits pass untouched. Peaks of a busy pattern are rounded off instead of
        // clipping.
        (
            bus_clip(bus.left + wet_left * room_gain + metal_left + groove_left),
            bus_clip(bus.right + wet_right * room_gain + metal_right + groove_right),
        )
    }
}

/// 0 is the lowest tom. Only called with tom notes.
pub(crate) fn tom_index(note: u8) -> usize {
    match note {
        41 => 0,
        43 => 1,
        45 => 2,
        47 => 3,
        48 => 4,
        _ => 5,
    }
}

/// Constant-power pan: (left, right) gains for a position from -1.0 to 1.0
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan + 1.0) * FRAC_PI_4;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::BUS_CLIP_KNEE;
    use crate::test_util::{
        band_rms, brightness, decay_time_s, peak, rms, spectrum_fill, spectrum_fill_by,
        zero_crossing_freq,
    };

    const SAMPLE_RATE: f32 = 44100.0;

    const KICK: u8 = 36;
    const SNARE: u8 = 38;
    const FLOOR_TOM: u8 = 41;
    const HAT_CLOSED: u8 = 42;
    const HAT_OPEN: u8 = 46;
    const CRASH: u8 = 49;
    const ALL_NOTES: [u8; 21] =
        [35, 36, 37, 38, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 55, 57, 59];
    const PIECES: [(&str, u8); 18] = [
        ("kick", 36),
        ("side stick", 37),
        ("snare", 38),
        ("tom 1 (floor)", 41),
        ("tom 2", 43),
        ("tom 3", 45),
        ("tom 4", 47),
        ("tom 5", 48),
        ("tom 6 (high)", 50),
        ("hat closed", 42),
        ("hat pedal", 44),
        ("hat open", 46),
        ("crash 1", 49),
        ("crash 2", 57),
        ("ride", 51),
        ("ride bell", 53),
        ("china", 52),
        ("splash", 55),
    ];
    // Index into `PIECES` of the first cymbal
    const FIRST_CYMBAL_PIECE: usize = 9;

    /// Dry kit, so measurements are of the voices and not the room
    fn dry_kit(kit: Kit, sample_rate: f32) -> DrumKit {
        let mut drums = DrumKit::new();
        drums.set_sample_rate(sample_rate);
        drums.set_kit(kit);
        drums.mix.room = 0.0;
        drums
    }

    fn render(drums: &mut DrumKit, num_samples: usize) -> (Vec<f32>, Vec<f32>) {
        (0..num_samples).map(|_| drums.process()).unzip()
    }

    fn render_mono(drums: &mut DrumKit, seconds: f32) -> Vec<f32> {
        let (left, right) = render(drums, (seconds * SAMPLE_RATE) as usize);
        left.iter().zip(&right).map(|(l, r)| (l + r) * 0.5).collect()
    }

    fn render_note(kit: Kit, note: u8, velocity: f32, seconds: f32) -> Vec<f32> {
        let mut drums = dry_kit(kit, SAMPLE_RATE);
        drums.note_on(note, velocity);
        render_mono(&mut drums, seconds)
    }

    fn seconds(samples: &[f32], from: f32, to: f32) -> &[f32] {
        &samples[(from * SAMPLE_RATE) as usize..(to * SAMPLE_RATE) as usize]
    }

    #[test]
    fn test_silent_before_any_note() {
        let mut drums = DrumKit::new();
        drums.set_sample_rate(SAMPLE_RATE);

        let (left, right) = render(&mut drums, 2048);

        assert!(left.iter().chain(&right).all(|&s| s == 0.0));
    }

    #[test]
    fn test_every_mapped_note_sounds_in_every_kit() {
        for kit in Kit::ALL {
            for note in ALL_NOTES {
                // The quietest is Groove's pedal hi-hat, a soft chick
                let output = render_note(kit, note, 1.0, 0.2);
                assert!(peak(&output) > 0.02, "{:?} note {} peak: {}", kit, note, peak(&output));
            }
        }
    }

    #[test]
    fn test_unmapped_notes_and_zero_velocity_are_ignored() {
        let mut drums = dry_kit(Kit::Rock, SAMPLE_RATE);

        for note in [0, 34, 39, 54, 60, 127] {
            drums.note_on(note, 1.0);
        }
        drums.note_on(KICK, 0.0);
        let output = render_mono(&mut drums, 0.1);

        assert!(output.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_groove_has_a_shaker_and_the_other_kits_do_not() {
        for kit in Kit::ALL {
            for note in [69, 70, 82] {
                let sounds = peak(&render_note(kit, note, 1.0, 0.2)) > 0.01;
                assert_eq!(sounds, kit == Kit::Groove, "{:?} note {}", kit, note);
            }
        }
    }

    #[test]
    fn test_default_tune_and_damping_leave_the_engines_alone() {
        // A hit is the same whether the two controls were never touched or set to nothing
        for kit in [Kit::Metal, Kit::Groove] {
            let plain = render_note(kit, SNARE, 0.9, 0.3);
            let mut drums = dry_kit(kit, SAMPLE_RATE);
            drums.set_tune(0.0);
            drums.set_damping(0.0);
            drums.note_on(SNARE, 0.9);
            assert_eq!(plain, render_mono(&mut drums, 0.3), "{:?}", kit);
        }
    }

    #[test]
    fn test_engine_kits_follow_tune_damping_and_levels() {
        for kit in [Kit::Metal, Kit::Groove] {
            let tom = |semitones: f32, damping: f32, level: f32| {
                let mut drums = dry_kit(kit, SAMPLE_RATE);
                drums.set_tune(semitones);
                drums.set_damping(damping);
                drums.mix.toms = level;
                drums.note_on(FLOOR_TOM, 1.0);
                render_mono(&mut drums, 3.0)
            };
            let pitch = |output: &[f32]| zero_crossing_freq(seconds(output, 0.3, 0.6), SAMPLE_RATE);
            let (plain, high, damped, quiet) =
                (tom(0.0, 0.0, 1.0), tom(6.0, 0.0, 1.0), tom(0.0, 1.0, 1.0), tom(0.0, 0.0, 0.5));

            let ratio = pitch(&high) / pitch(&plain);
            assert!(
                (ratio / semitones_to_ratio(6.0) - 1.0).abs() < 0.05,
                "{:?} tune ratio {}",
                kit,
                ratio
            );
            let decay = |output: &[f32]| decay_time_s(output, SAMPLE_RATE, -40.0);
            assert!(
                decay(&damped) < decay(&plain) * 0.5,
                "{:?}: {} vs {}",
                kit,
                decay(&damped),
                decay(&plain)
            );
            assert!(peak(&quiet) < peak(&plain) * 0.7, "{:?}", kit);
        }
    }

    #[test]
    fn test_engines_ring_on_under_another_kit_and_then_rest() {
        let mut drums = dry_kit(Kit::Metal, SAMPLE_RATE);
        drums.note_on(CRASH, 1.0);
        let before = render_mono(&mut drums, 0.1);

        drums.set_kit(Kit::Rock);
        let after = render_mono(&mut drums, 0.1);
        let ratio = rms(&after) / rms(&before);
        assert!((0.3..1.0).contains(&ratio), "Level ratio across kit change: {}", ratio);

        render_mono(&mut drums, 12.0);
        assert!(render_mono(&mut drums, 0.1).iter().all(|&sample| sample == 0.0));
    }

    #[test]
    fn test_full_kit_decays_to_silence() {
        // Half the usual sample rate keeps this long render quick
        let sample_rate = 22050.0;
        for kit in Kit::ALL {
            let mut drums = DrumKit::new();
            drums.set_sample_rate(sample_rate);
            drums.set_kit(kit);
            drums.mix.room = 1.0;

            for note in ALL_NOTES {
                drums.note_on(note, 1.0);
            }
            let (left, _) = render(&mut drums, sample_rate as usize * 20);

            let tail = &left[left.len() - sample_rate as usize / 2..];
            assert!(peak(tail) < 0.0001, "{:?} is still ringing: {}", kit, peak(tail));
        }
    }

    #[test]
    fn test_output_bounded_and_finite() {
        for &sample_rate in &[22050.0, 44100.0, 48000.0, 96000.0, 192000.0] {
            for kit in Kit::ALL {
                let mut drums = DrumKit::new();
                drums.set_sample_rate(sample_rate);
                drums.set_kit(kit);
                drums.mix.room = 1.0;

                // Everything at once, twice, is far more than any real pattern
                let mut output = Vec::new();
                for _ in 0..2 {
                    for note in ALL_NOTES {
                        drums.note_on(note, 1.0);
                    }
                    let (left, right) = render(&mut drums, (sample_rate * 0.15) as usize);
                    output.extend(left);
                    output.extend(right);
                }

                assert!(output.iter().all(|s| s.is_finite()));
                assert!(peak(&output) <= 1.0, "Peak {} at {} Hz", peak(&output), sample_rate);
            }
        }
    }

    #[test]
    fn test_single_hits_stay_below_the_bus_clipper() {
        for kit in Kit::ALL {
            for (name, note) in PIECES {
                let level = peak(&render_note(kit, note, 1.0, 0.5));
                assert!(level < BUS_CLIP_KNEE, "{:?} {} peaks at {}", kit, name, level);
            }
        }
    }

    #[test]
    fn test_velocity_raises_level_and_brightness() {
        for note in [KICK, SNARE, CRASH] {
            let soft = render_note(Kit::Rock, note, 0.3, 0.5);
            let hard = render_note(Kit::Rock, note, 1.0, 0.5);

            assert!(peak(&hard) > peak(&soft) * 2.0, "Note {} level", note);
            assert!(brightness(&hard) > brightness(&soft) * 1.2, "Note {} brightness", note);
        }
    }

    #[test]
    fn test_consecutive_hits_differ() {
        let mut drums = dry_kit(Kit::Rock, SAMPLE_RATE);

        drums.note_on(SNARE, 0.8);
        let first = render_mono(&mut drums, 2.0);
        drums.note_on(SNARE, 0.8);
        let second = render_mono(&mut drums, 2.0);

        assert_ne!(first, second);
        // Different, but still the same drum
        let level_ratio = rms(&second) / rms(&first);
        assert!((0.7..1.4).contains(&level_ratio), "Level ratio: {}", level_ratio);
    }

    #[test]
    fn test_closed_hat_chokes_open_hat() {
        for kit in Kit::ALL {
            let open = render_note(kit, HAT_OPEN, 1.0, 0.5);

            let mut drums = dry_kit(kit, SAMPLE_RATE);
            drums.note_on(HAT_OPEN, 1.0);
            let mut choked = render_mono(&mut drums, 0.1);
            drums.note_on(HAT_CLOSED, 0.5);
            choked.extend(render_mono(&mut drums, 0.4));

            let ringing = rms(seconds(&open, 0.3, 0.5));
            let after_choke = rms(seconds(&choked, 0.3, 0.5));
            assert!(ringing > 0.001, "{:?} open hat does not ring: {}", kit, ringing);
            assert!(after_choke < ringing * 0.1, "{:?}: {} vs {}", kit, after_choke, ringing);
        }
    }

    #[test]
    fn test_kits_differ() {
        let kick_freq = |kit| {
            zero_crossing_freq(seconds(&render_note(kit, KICK, 1.0, 0.5), 0.1, 0.3), SAMPLE_RATE)
        };
        let decay = |kit, note| decay_time_s(&render_note(kit, note, 1.0, 4.0), SAMPLE_RATE, -40.0);
        let crash_brightness = |kit| brightness(&render_note(kit, CRASH, 1.0, 1.0));

        // Jazz kick is tuned high and open, metal kick is short
        assert!(kick_freq(Kit::Jazz) > kick_freq(Kit::Rock) * 1.2);
        assert!(decay(Kit::Metal, KICK) < decay(Kit::Rock, KICK));
        assert!(decay(Kit::Rock, KICK) < decay(Kit::Jazz, KICK));
        // Metal snare and toms are tight, jazz ones ring
        assert!(decay(Kit::Metal, SNARE) < decay(Kit::Jazz, SNARE));
        assert!(decay(Kit::Metal, FLOOR_TOM) < decay(Kit::Jazz, FLOOR_TOM));
        // Jazz cymbals are dark, and Groove's darker still
        assert!(crash_brightness(Kit::Rock) > crash_brightness(Kit::Jazz) * 1.1);
        assert!(crash_brightness(Kit::Jazz) > crash_brightness(Kit::Groove) * 2.0);
        // Groove is round and open where Metal is tight
        assert!(decay(Kit::Metal, KICK) < decay(Kit::Groove, KICK) * 0.6);
        assert!(decay(Kit::Metal, FLOOR_TOM) < decay(Kit::Groove, FLOOR_TOM));
    }

    #[test]
    fn test_tune_shifts_drum_pitch() {
        let kick_freq = |semitones| {
            let mut drums = dry_kit(Kit::Jazz, SAMPLE_RATE);
            drums.set_tune(semitones);
            drums.note_on(KICK, 1.0);
            zero_crossing_freq(seconds(&render_mono(&mut drums, 0.6), 0.15, 0.55), SAMPLE_RATE)
        };

        let ratio = kick_freq(6.0) / kick_freq(0.0);
        let expected = semitones_to_ratio(6.0);
        assert!((ratio / expected - 1.0).abs() < 0.05, "Ratio {}, expected {}", ratio, expected);
    }

    #[test]
    fn test_damping_shortens_decay() {
        for note in [FLOOR_TOM, CRASH] {
            let decay = |damping| {
                let mut drums = dry_kit(Kit::Rock, SAMPLE_RATE);
                drums.set_damping(damping);
                drums.note_on(note, 1.0);
                decay_time_s(&render_mono(&mut drums, 5.0), SAMPLE_RATE, -40.0)
            };

            assert!(
                decay(1.0) < decay(0.0) * 0.5,
                "Note {}: {} s vs {} s",
                note,
                decay(1.0),
                decay(0.0)
            );
        }
    }

    #[test]
    fn test_stereo_placement() {
        let levels = |note| {
            let mut drums = dry_kit(Kit::Rock, SAMPLE_RATE);
            drums.note_on(note, 1.0);
            let (left, right) = render(&mut drums, 8820);
            (rms(&left), rms(&right))
        };

        // From the drummer's seat: hi-hat on the left, floor tom on the right
        let (left, right) = levels(HAT_CLOSED);
        assert!(left > right * 1.5);
        let (left, right) = levels(FLOOR_TOM);
        assert!(right > left * 1.5);
        let (left, right) = levels(KICK);
        assert!((left / right - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_room_adds_a_stereo_tail() {
        let snare_hit = |room| {
            let mut drums = dry_kit(Kit::Rock, SAMPLE_RATE);
            drums.mix.room = room;
            drums.note_on(SNARE, 1.0);
            render(&mut drums, SAMPLE_RATE as usize)
        };

        // Every kit starts from the same noise seeds, so the difference is the room alone
        let (dry_left, dry_right) = snare_hit(0.0);
        let (wet_left, wet_right) = snare_hit(1.0);
        let room_left: Vec<f32> =
            wet_left.iter().zip(&dry_left).map(|(wet, dry)| wet - dry).collect();
        let room_right: Vec<f32> =
            wet_right.iter().zip(&dry_right).map(|(wet, dry)| wet - dry).collect();

        let ratio = rms(&room_left) / rms(&dry_left);
        assert!((0.2..1.5).contains(&ratio), "Room level relative to the dry snare: {}", ratio);
        assert!(decay_time_s(&room_left, SAMPLE_RATE, -40.0) > 0.2);
        // The snare is dead centre, so any difference between the channels is the room
        assert_ne!(room_left, room_right);
    }

    #[test]
    fn test_kit_change_does_not_cut_ringing_voices() {
        let mut drums = dry_kit(Kit::Rock, SAMPLE_RATE);
        drums.note_on(CRASH, 1.0);
        let before = render_mono(&mut drums, 0.1);

        drums.set_kit(Kit::Metal);
        let after = render_mono(&mut drums, 0.1);

        let ratio = rms(&after) / rms(&before);
        assert!((0.3..1.0).contains(&ratio), "Level ratio across kit change: {}", ratio);
    }

    /// Prints level, decay and brightness of every piece in every kit:
    ///   cargo test -p drums --release kit_report -- --ignored --nocapture
    ///
    /// Compare before and after changing a preset or a voice.
    #[test]
    #[ignore]
    fn kit_report() {
        // rms and brightness are of the first 200 ms, crack is the level of the first 50 ms,
        // >4k the level above 4 kHz in the first 200 ms. For cymbals: low tail is the share
        // of the energy under 1 kHz from half a second on, fill how evenly 4 to 12 kHz is
        // filled (1 is noise, 0 a few sines), fine the same in 15 Hz steps from 5 to 7 kHz, which
        // tells noise from a bank of resonators
        for velocity in [0.6, 1.0] {
            for kit in Kit::ALL {
                println!("\n{:?}, velocity {}", kit, velocity);
                println!(
                    "{:<16}{:>8}{:>8}{:>8}{:>8}{:>11}{:>12}{:>10}{:>7}{:>7}",
                    "piece",
                    "peak",
                    "rms",
                    "crack",
                    ">4k",
                    "decay (s)",
                    "brightness",
                    "low tail",
                    "fill",
                    "fine"
                );
                for (index, (name, note)) in PIECES.into_iter().enumerate() {
                    let output = render_note(kit, note, velocity, 12.0);
                    let start = seconds(&output, 0.0, 0.2);
                    print!(
                        "{:<16}{:>8.3}{:>8.3}{:>8.3}{:>8.3}{:>11.2}{:>12.3}",
                        name,
                        peak(&output),
                        rms(start),
                        rms(seconds(&output, 0.0, 0.05)),
                        band_rms(start, SAMPLE_RATE, 4000.0, SAMPLE_RATE),
                        decay_time_s(&output, SAMPLE_RATE, -40.0),
                        brightness(start),
                    );
                    if index >= FIRST_CYMBAL_PIECE {
                        let tail = seconds(&output, 0.5, 1.5);
                        let low = band_rms(tail, SAMPLE_RATE, 20.0, 1000.0) / rms(tail).max(1e-9);
                        let fill =
                            spectrum_fill(seconds(&output, 0.1, 0.3), SAMPLE_RATE, 4000.0, 12000.0);
                        let fine = spectrum_fill_by(
                            seconds(&output, 0.1, 0.4),
                            SAMPLE_RATE,
                            5000.0,
                            7000.0,
                            15.0,
                        );
                        print!("{:>10.3}{:>7.2}{:>7.2}", low * low, fill, fine);
                    }
                    println!();
                }
            }
        }

        // Cost of the cymbals, the only voices with more than a handful of oscillators
        let mut drums = dry_kit(Kit::Rock, 48000.0);
        let ring_all = |drums: &mut DrumKit| {
            for note in [46, 49, 57, 51, 53, 52, 55] {
                drums.note_on(note, 1.0);
            }
        };
        ring_all(&mut drums);
        let frames = 48000;
        let started = std::time::Instant::now();
        let mut sum = 0.0;
        for _ in 0..frames {
            sum += drums.process().0;
        }
        let per_frame_ns = started.elapsed().as_nanos() as f64 / frames as f64;
        let hits = 1000;
        let started = std::time::Instant::now();
        for _ in 0..hits {
            drums.note_on(CRASH, 0.8);
        }
        let per_hit_us = started.elapsed().as_nanos() as f64 / hits as f64 / 1000.0;
        println!(
            "\nEvery cymbal ringing at 48 kHz: {:.0} ns per frame ({:.1} % of a core), one crash hit {:.1} us ({})",
            per_frame_ns,
            per_frame_ns * 48000.0 / 1e7,
            per_hit_us,
            sum.is_finite()
        );
    }

    /// Writes WAV files to target/renders for listening:
    ///   cargo test -p drums --release render_wavs -- --ignored
    ///
    /// Per kit: every piece at three velocities, and a short groove in the style.
    ///
    /// Set DRUMS_INPUT_MID to the path of a MIDI file to render that drum part as well,
    /// through every kit (drums_<kit>_input.wav).
    #[test]
    #[ignore]
    fn render_wavs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();
        let part = std::env::var("DRUMS_INPUT_MID").ok().map(|path| {
            let bytes = std::fs::read(&path).expect("Could not open DRUMS_INPUT_MID");
            midi_pattern(&bytes).unwrap_or_else(|problem| panic!("DRUMS_INPUT_MID: {problem}"))
        });

        for kit in Kit::ALL {
            let name = format!("{:?}", kit).to_lowercase();

            let mut pieces = Vec::new();
            for (i, (_, note)) in PIECES.iter().enumerate() {
                for (j, velocity) in [0.4, 0.7, 1.0].into_iter().enumerate() {
                    pieces.push(((i * 3 + j) as f32, *note, velocity));
                }
            }
            write_wav(
                &dir.join(format!("drums_{}_pieces.wav", name)),
                &render_pattern(kit, &pieces, 100.0),
            );

            let (groove, bpm) = groove(kit);
            write_wav(
                &dir.join(format!("drums_{}_groove.wav", name)),
                &render_pattern(kit, &groove, bpm),
            );

            if let Some((part, bpm)) = &part {
                write_wav(
                    &dir.join(format!("drums_{}_input.wav", name)),
                    &render_pattern(kit, part, *bpm),
                );
            }
        }
        println!("Wrote renders to {}", dir.display());
    }

    /// A pattern is a list of (beat, note, velocity)
    type Pattern = Vec<(f32, u8, f32)>;

    /// The drum part of a MIDI file as a pattern and its tempo: the note-ons of every track
    /// and channel. A pattern has one tempo, so a file whose tempo changes is refused; one
    /// that names no tempo is at 120, as the format says.
    fn midi_pattern(bytes: &[u8]) -> Result<(Pattern, f32), String> {
        use midly::{MetaMessage, MidiMessage, Timing, TrackEventKind};

        let file =
            midly::Smf::parse(bytes).map_err(|error| format!("not a MIDI file ({error})"))?;
        let ticks_per_beat = match file.header.timing {
            Timing::Metrical(ticks) => ticks.as_int() as f64,
            Timing::Timecode(..) => return Err("timed in frames, not in beats".to_string()),
        };

        let mut pattern = Pattern::new();
        let mut beat_us: Option<u32> = None;
        for track in &file.tracks {
            let mut tick = 0u64;
            for event in track {
                tick += event.delta.as_int() as u64;
                match event.kind {
                    TrackEventKind::Midi { message: MidiMessage::NoteOn { key, vel }, .. }
                        if vel.as_int() > 0 =>
                    {
                        let beat = (tick as f64 / ticks_per_beat) as f32;
                        pattern.push((beat, key.as_int(), vel.as_int() as f32 / 127.0));
                    }
                    TrackEventKind::Meta(MetaMessage::Tempo(tempo)) => {
                        if beat_us.is_some_and(|first| first != tempo.as_int()) {
                            return Err(
                                "the tempo changes: a part is rendered at one tempo".to_string()
                            );
                        }
                        beat_us = Some(tempo.as_int());
                    }
                    _ => {}
                }
            }
        }
        if pattern.is_empty() {
            return Err("no notes".to_string());
        }
        Ok((pattern, 60e6 / beat_us.unwrap_or(500_000) as f32))
    }

    /// A MIDI file of one track, 480 ticks to the beat, from (ticks since the event before, event)
    fn midi_file(events: &[(u32, midly::TrackEventKind<'static>)]) -> Vec<u8> {
        let header =
            midly::Header::new(midly::Format::SingleTrack, midly::Timing::Metrical(480.into()));
        let mut file = midly::Smf::new(header);
        let mut track: Vec<midly::TrackEvent> = events
            .iter()
            .map(|&(delta, kind)| midly::TrackEvent { delta: delta.into(), kind })
            .collect();
        track.push(midly::TrackEvent {
            delta: 0.into(),
            kind: midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack),
        });
        file.tracks.push(track);
        let mut bytes = Vec::new();
        file.write_std(&mut bytes).unwrap();
        bytes
    }

    fn midi_tempo(bpm: f32) -> midly::TrackEventKind<'static> {
        midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(((60e6 / bpm) as u32).into()))
    }

    fn midi_note(on: bool, note: u8, velocity: u8) -> midly::TrackEventKind<'static> {
        let (key, vel) = (note.into(), velocity.into());
        let message = if on {
            midly::MidiMessage::NoteOn { key, vel }
        } else {
            midly::MidiMessage::NoteOff { key, vel }
        };
        midly::TrackEventKind::Midi { channel: 9.into(), message }
    }

    #[test]
    fn test_midi_part_keeps_beats_notes_velocities_and_tempo() {
        let bytes = midi_file(&[
            (0, midi_tempo(100.0)),
            (0, midi_note(true, 36, 127)),
            (0, midi_note(true, 42, 64)),
            (240, midi_note(false, 36, 0)),
            // A note-on of velocity 0 is a note-off
            (0, midi_note(true, 42, 0)),
            (240, midi_note(true, 38, 100)),
            (720, midi_note(true, 36, 1)),
        ]);
        let (pattern, bpm) = midi_pattern(&bytes).unwrap();
        assert_eq!(bpm, 100.0);
        assert_eq!(
            pattern,
            vec![
                (0.0, 36, 1.0),
                (0.0, 42, 64.0 / 127.0),
                (1.0, 38, 100.0 / 127.0),
                (2.5, 36, 1.0 / 127.0)
            ]
        );
    }

    #[test]
    fn test_midi_part_without_tempo_is_at_120() {
        let (_, bpm) = midi_pattern(&midi_file(&[(0, midi_note(true, 36, 100))])).unwrap();
        assert_eq!(bpm, 120.0);
    }

    #[test]
    fn test_midi_part_that_cannot_be_a_pattern_is_refused() {
        let changing = midi_file(&[
            (0, midi_tempo(100.0)),
            (0, midi_note(true, 36, 100)),
            (480, midi_tempo(140.0)),
        ]);
        assert!(midi_pattern(&changing).unwrap_err().contains("tempo"));
        // The same tempo written again is no change
        let repeated = midi_file(&[
            (0, midi_tempo(100.0)),
            (0, midi_note(true, 36, 100)),
            (480, midi_tempo(100.0)),
        ]);
        assert!(midi_pattern(&repeated).is_ok());

        assert!(midi_pattern(&midi_file(&[(0, midi_tempo(100.0))])).is_err());
        assert!(midi_pattern(b"RIFF").is_err());
    }

    /// Four bars in the style of the kit, and the tempo
    fn groove(kit: Kit) -> (Pattern, f32) {
        let mut pattern = Pattern::new();
        match kit {
            Kit::Rock => {
                for bar in 0..4 {
                    let start = bar as f32 * 4.0;
                    pattern.push((start, if bar == 0 { 49 } else { 42 }, 0.9));
                    for eighth in 1..8 {
                        let velocity = if eighth % 2 == 0 { 0.75 } else { 0.5 };
                        pattern.push((start + eighth as f32 * 0.5, 42, velocity));
                    }
                    pattern.extend([
                        (start, 36, 0.95),
                        (start + 2.0, 36, 0.9),
                        (start + 2.5, 36, 0.8),
                    ]);
                    pattern.push((start + 1.0, 38, 0.95));
                    if bar < 3 {
                        pattern.push((start + 3.0, 38, 0.95));
                    }
                }
                // Fill down the toms, then land on a crash
                for (i, note) in [50, 50, 48, 47, 45, 45, 43, 41].into_iter().enumerate() {
                    pattern.push((15.0 + i as f32 * 0.125, note, 0.8 + 0.02 * i as f32));
                }
                pattern.extend([(16.0, 36, 1.0), (16.0, 57, 1.0)]);
                (pattern, 110.0)
            }
            Kit::Jazz => {
                for bar in 0..4 {
                    let start = bar as f32 * 4.0;
                    // Swing ride pattern with the hi-hat foot on two and four
                    for (beat, velocity) in
                        [(0.0, 0.7), (1.0, 0.8), (1.667, 0.5), (2.0, 0.7), (3.0, 0.8), (3.667, 0.5)]
                    {
                        pattern.push((start + beat, 51, velocity));
                    }
                    pattern.extend([(start + 1.0, 44, 0.7), (start + 3.0, 44, 0.7)]);
                    // Feathered kick and snare comping
                    pattern.push((start, 36, 0.35));
                    pattern.push((start + 1.667, 38, 0.25));
                    if bar % 2 == 1 {
                        pattern.extend([(start + 2.667, 38, 0.6), (start + 3.667, 36, 0.55)]);
                    }
                }
                pattern.extend([(14.667, 47, 0.5), (15.0, 45, 0.55), (15.667, 43, 0.6)]);
                pattern.extend([(16.0, 36, 0.6), (16.0, 49, 0.7), (17.0, 53, 0.6)]);
                (pattern, 140.0)
            }
            Kit::Metal => {
                for bar in 0..4 {
                    let start = bar as f32 * 4.0;
                    // Double bass sixteenths under china quarters
                    for sixteenth in 0..16 {
                        if bar < 3 || sixteenth < 12 {
                            pattern.push((start + sixteenth as f32 * 0.25, 36, 0.95));
                        }
                    }
                    for beat in 0..4 {
                        pattern.push((
                            start + beat as f32,
                            if bar == 0 && beat == 0 { 49 } else { 52 },
                            0.9,
                        ));
                    }
                    pattern.push((start + 1.0, 38, 1.0));
                    if bar < 3 {
                        pattern.push((start + 3.0, 38, 1.0));
                    }
                }
                for (i, note) in
                    [38, 38, 50, 50, 48, 48, 45, 45, 43, 43, 41, 41].into_iter().enumerate()
                {
                    pattern.push((15.0 + i as f32 / 12.0, note, 0.95));
                }
                pattern.extend([(16.0, 36, 1.0), (16.0, 49, 1.0), (16.0, 57, 1.0)]);
                (pattern, 180.0)
            }
            Kit::Groove => {
                for bar in 0..4 {
                    let start = bar as f32 * 4.0;
                    // Sixteenth hi-hats, loud on the beat and soft between, a shaker on the offbeats
                    for sixteenth in 0..16 {
                        let velocity = [0.75, 0.3, 0.5, 0.3][sixteenth % 4];
                        let open = bar % 2 == 1 && sixteenth == 14;
                        pattern.push((
                            start + sixteenth as f32 * 0.25,
                            if open { 46 } else { 42 },
                            velocity,
                        ));
                    }
                    for beat in 0..4 {
                        pattern.push((start + beat as f32 + 0.5, 70, 0.6));
                    }
                    // A laid back kick, the snare on two and four with ghost notes around it
                    pattern.extend([
                        (start, 36, 0.9),
                        (start + 1.75, 36, 0.6),
                        (start + 2.5, 36, 0.8),
                    ]);
                    pattern.extend([(start + 1.0, 38, 0.85), (start + 3.0, 38, 0.9)]);
                    pattern.extend([
                        (start + 1.5, 38, 0.2),
                        (start + 2.25, 38, 0.25),
                        (start + 3.75, 38, 0.2),
                    ]);
                    if bar == 2 {
                        pattern.push((start + 3.5, 37, 0.7));
                    }
                }
                pattern.extend([(15.5, 47, 0.6), (15.75, 43, 0.65)]);
                pattern.extend([
                    (16.0, 36, 0.9),
                    (16.0, 49, 0.8),
                    (17.0, 53, 0.6),
                    (18.0, 51, 0.6),
                ]);
                (pattern, 88.0)
            }
        }
    }

    /// Renders a pattern with the default mix, plus a tail. Returns interleaved stereo.
    fn render_pattern(kit: Kit, pattern: &Pattern, bpm: f32) -> Vec<f32> {
        let mut drums = DrumKit::new();
        drums.set_sample_rate(SAMPLE_RATE);
        drums.set_kit(kit);

        let samples_per_beat = SAMPLE_RATE * 60.0 / bpm;
        let mut events: Vec<(usize, u8, f32)> = pattern
            .iter()
            .map(|&(beat, note, velocity)| ((beat * samples_per_beat) as usize, note, velocity))
            .collect();
        events.sort_by_key(|event| event.0);
        let length = events.last().map_or(0, |event| event.0) + SAMPLE_RATE as usize * 3;

        let mut output = Vec::with_capacity(length * 2);
        let mut next = 0;
        for sample in 0..length {
            while next < events.len() && events[next].0 <= sample {
                drums.note_on(events[next].1, events[next].2);
                next += 1;
            }
            let (left, right) = drums.process();
            output.push(left);
            output.push(right);
        }
        output
    }

    fn write_wav(path: &std::path::Path, interleaved: &[f32]) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SAMPLE_RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &sample in interleaved {
            writer.write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }
}
