use nih_plug::prelude::Enum;
use suite_common::dsp::{advance, decay_coeff, note_hz, sine, Noise, Svf};

/// The two basses. They share the string and differ in how it is plucked and in what
/// stands behind it.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// Played with a pick: clean lows with a distorted band of mids over them
    Metal,
    /// Played with a finger: a warm tone filter and a little tube drive
    // New voices go at the end: a project remembers its voice by its place here
    Groove,
}

impl Voice {
    pub const ALL: [Voice; 2] = [Voice::Metal, Voice::Groove];

    /// Where the knobs stand when the voice is picked on the pedal
    pub fn settings(self) -> BassSettings {
        match self {
            Voice::Metal => BassSettings {
                voice: self,
                volume_db: -2.0,
                drive: 0.5,
                brightness: 0.55,
                sub: 0.45,
                tone_hz: 3200.0,
                sustain_s: 3.5,
                slide_ms: 0.0,
                transpose: 0.0,
            },
            Voice::Groove => BassSettings {
                voice: self,
                volume_db: -3.0,
                drive: 0.15,
                brightness: 0.35,
                sub: 0.25,
                tone_hz: 1100.0,
                sustain_s: 3.0,
                slide_ms: 60.0,
                transpose: 0.0,
            },
        }
    }
}

pub const VOLUME_MIN_DB: f32 = -30.0;
pub const VOLUME_MAX_DB: f32 = 12.0;
pub const TONE_MIN_HZ: f32 = 300.0;
pub const TONE_MAX_HZ: f32 = 4000.0;
pub const SUSTAIN_MIN_S: f32 = 0.5;
pub const SUSTAIN_MAX_S: f32 = 8.0;
pub const SLIDE_MAX_MS: f32 = 300.0;
pub const TRANSPOSE_RANGE: f32 = 24.0;

/// Everything the pedal sets, as the plugin reads it once per block
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BassSettings {
    pub voice: Voice,
    pub volume_db: f32,
    /// 0.0-1.0
    pub drive: f32,
    /// 0.0-1.0: how hard the string is plucked and how long its top rings
    pub brightness: f32,
    /// 0.0-1.0: level of the sine under the string
    pub sub: f32,
    /// The low-pass behind the string (Groove) or behind the drive (Metal)
    pub tone_hz: f32,
    /// Time for a held note to fall by 60 dB
    pub sustain_s: f32,
    /// Time of a slide between overlapping notes. 0 plucks every note
    pub slide_ms: f32,
    /// Semitones
    pub transpose: f32,
}

impl Default for BassSettings {
    fn default() -> Self {
        Voice::Metal.settings()
    }
}

// Time for the string to fall by 60 dB once the note is let go
const RELEASE_METAL_S: f32 = 0.07;
const RELEASE_GROOVE_S: f32 = 0.12;

// The string's delay line at 48 kHz and below; longer in proportion above
const STRING_SAMPLES: usize = 16384;

/// One string, one note at a time: a delay line that feeds itself through a low-pass
/// (Karplus-Strong), a sine at the pitch of the string under it, and the voice's output
/// stage. Volume is left to the plugin, which smooths it.
pub struct Bass {
    sample_rate: f32,
    settings: BassSettings,
    noise: Noise,

    line: Vec<f32>,
    mask: usize,
    write: usize,
    // Scratch for shaping a pluck, as long as the line
    pluck: Vec<f32>,
    shaped: Vec<f32>,

    // Length of the loop in samples, and where a slide is taking it
    delay: f32,
    target_delay: f32,
    // What the loop's low-pass adds to its length
    filter_delay: f32,
    loop_lp: f32,
    loop_state: f32,
    feedback: f32,
    feedback_held: f32,
    feedback_released: f32,
    glide: f32,

    note: Option<u8>,
    held: bool,
    // The voice the ringing string was plucked with
    string_voice: Voice,

    sub_phase: f32,
    sub_env: f32,
    sub_attack: f32,
    // Per sample: the fade in, and the decay while the note is held and once it is let go
    sub_attack_step: f32,
    sub_decay_held: f32,
    sub_decay_released: f32,
    sub_level: f32,

    // Metal's output stage
    split_low: Svf,
    drive_hp: Svf,
    drive_lp: Svf,
    mid_peak: Svf,
    // Groove's
    tone: Svf,
    dc: Svf,
}

impl Bass {
    pub fn new() -> Self {
        let mut bass = Self {
            sample_rate: 44100.0,
            settings: BassSettings::default(),
            noise: Noise::new(1),
            line: Vec::new(),
            mask: 0,
            write: 0,
            pluck: Vec::new(),
            shaped: Vec::new(),
            delay: 800.0,
            target_delay: 800.0,
            filter_delay: 0.0,
            loop_lp: 0.4,
            loop_state: 0.0,
            feedback: 0.0,
            feedback_held: 0.0,
            feedback_released: 0.0,
            glide: 1.0,
            note: None,
            held: false,
            string_voice: Voice::Metal,
            sub_phase: 0.0,
            sub_env: 0.0,
            sub_attack: 1.0,
            sub_attack_step: 1.0,
            sub_decay_held: 0.0,
            sub_decay_released: 0.0,
            sub_level: 0.0,
            split_low: Svf::new(),
            drive_hp: Svf::new(),
            drive_lp: Svf::new(),
            mid_peak: Svf::new(),
            tone: Svf::new(),
            dc: Svf::new(),
        };
        bass.set_sample_rate(44100.0);
        bass
    }

    /// Allocates the string. Not for the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let size = STRING_SAMPLES * ((sample_rate / 48000.0).ceil() as usize).next_power_of_two();
        self.line = vec![0.0; size];
        self.pluck = vec![0.0; size];
        self.shaped = vec![0.0; size];
        self.mask = size - 1;
        self.write = 0;

        self.split_low.set(260.0, 0.6, sample_rate);
        self.drive_hp.set(120.0, 0.7, sample_rate);
        self.mid_peak.set(900.0, 1.0, sample_rate);
        self.set_voice_filters();
        self.reset();
    }

    pub fn reset(&mut self) {
        self.line.fill(0.0);
        self.loop_state = 0.0;
        self.note = None;
        self.held = false;
        self.sub_env = 0.0;
        for filter in [
            &mut self.split_low,
            &mut self.drive_hp,
            &mut self.drive_lp,
            &mut self.mid_peak,
            &mut self.tone,
            &mut self.dc,
        ] {
            filter.reset();
        }
    }

    /// Once per block. A ringing note keeps the string it was plucked with: another voice
    /// changes what stands behind the string at once, and the string at the next pluck.
    pub fn set(&mut self, settings: &BassSettings) {
        if *settings == self.settings {
            return;
        }
        let voice_changed = settings.voice != self.settings.voice;
        let decay_changed = voice_changed || settings.sustain_s != self.settings.sustain_s;
        self.settings = *settings;
        self.set_voice_filters();
        if voice_changed {
            // The other voice's filters start from rest
            for filter in [&mut self.drive_lp, &mut self.tone, &mut self.dc] {
                filter.reset();
            }
        }
        if decay_changed && self.note.is_some() {
            // The note that is ringing follows Sustain too, held or let go
            let held = self.held;
            self.set_feedback(self.sample_rate / (self.target_delay + self.filter_delay));
            if !held {
                self.feedback = self.feedback_released;
            }
        }
    }

    fn set_voice_filters(&mut self) {
        let rate = self.sample_rate;
        let tone_hz = self.settings.tone_hz;
        let (sub_attack_s, sub_held_s, sub_released_s, sub_level) = match self.settings.voice {
            Voice::Metal => (0.004, 1.4, 0.035, 0.9),
            Voice::Groove => (0.006, 1.2, 0.03, 0.8),
        };
        self.sub_attack_step = 1.0 / (sub_attack_s * rate);
        self.sub_decay_held = decay_coeff(sub_held_s, rate);
        self.sub_decay_released = decay_coeff(sub_released_s, rate);
        self.sub_level = sub_level;
        match self.settings.voice {
            Voice::Metal => {
                self.drive_lp.set(tone_hz, 0.8, rate);
                self.dc.set(25.0, 0.7, rate);
                // A picked string's top follows Brightness while it rings
                if self.string_voice == Voice::Metal {
                    self.loop_lp = 0.3 + 0.62 * self.settings.brightness;
                }
            }
            Voice::Groove => {
                self.tone.set(tone_hz, 0.75, rate);
                self.dc.set(28.0, 0.7, rate);
            }
        }
        // A third of the slide time as the time constant: there after the whole of it
        self.glide = if self.settings.slide_ms > 0.0 {
            1.0 - (-1.0 / (self.settings.slide_ms * 0.001 / 3.0 * rate)).exp()
        } else {
            1.0
        };
    }

    /// Velocity is 0.0-1.0
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        if velocity <= 0.0 {
            return self.note_off(note);
        }
        let sliding = self.settings.slide_ms > 0.0 && self.held && self.note.is_some_and(|held| held != note);
        if sliding {
            self.slide_to(note);
        } else {
            self.pluck(note, velocity.min(1.0));
        }
    }

    pub fn note_off(&mut self, note: u8) {
        if self.note == Some(note) {
            self.release();
        }
    }

    /// Lets go of whatever note is held
    pub fn release(&mut self) {
        self.held = false;
        self.feedback = self.feedback_released;
    }

    fn freq_hz(&self, note: u8) -> f32 {
        note_hz(note as f32 + self.settings.transpose)
    }

    /// Length of the loop for a note, less what its low-pass adds
    fn loop_delay(&self, freq_hz: f32) -> f32 {
        let longest = (self.line.len() - 4) as f32;
        (self.sample_rate / freq_hz - self.filter_delay).clamp(2.0, longest)
    }

    fn set_feedback(&mut self, freq_hz: f32) {
        let release_s = match self.settings.voice {
            Voice::Metal => RELEASE_METAL_S,
            Voice::Groove => RELEASE_GROOVE_S,
        };
        self.feedback_held = 10.0f32.powf(-3.0 / (self.settings.sustain_s * freq_hz));
        self.feedback_released = 10.0f32.powf(-3.0 / (release_s * freq_hz));
        self.feedback = self.feedback_held;
    }

    fn slide_to(&mut self, note: u8) {
        let freq_hz = self.freq_hz(note);
        self.target_delay = self.loop_delay(freq_hz);
        self.set_feedback(freq_hz);
        self.note = Some(note);
        self.held = true;
    }

    fn pluck(&mut self, note: u8, velocity: f32) {
        let freq_hz = self.freq_hz(note);
        let brightness = self.settings.brightness;
        let period = (self.sample_rate / freq_hz).min((self.line.len() - 8) as f32);

        let level = match self.settings.voice {
            Voice::Metal => {
                let level = velocity.powf(1.2);
                self.loop_lp = 0.3 + 0.62 * brightness;
                // A burst of low-passed noise, less itself a little later: where the pick is
                let length = period.ceil() as usize + 2;
                let mut smooth = 0.0;
                for sample in &mut self.pluck[..length] {
                    smooth += (self.noise.next() - smooth) * (0.18 + 0.55 * brightness);
                    *sample = smooth;
                }
                let pick = ((period * 0.18) as usize).max(1);
                for index in 0..length {
                    let comb = self.pluck[index] - if index >= pick { self.pluck[index - pick] } else { 0.0 };
                    let position = (self.write + self.line.len() - 1 - index) & self.mask;
                    self.line[position] = comb * level * 1.6;
                }
                level
            }
            Voice::Groove => {
                // A softer touch is darker
                self.loop_lp = 0.18 + 0.6 * brightness * (0.55 + 0.45 * velocity);
                // The string pulled into a triangle a fifth of the way along, with a touch of noise
                let length = period.ceil() as usize;
                let mut mean = 0.0;
                for (index, sample) in self.pluck[..length].iter_mut().enumerate() {
                    let along = index as f32 / length as f32;
                    let shape = if along < 0.2 { along / 0.2 } else { (1.0 - along) / 0.8 };
                    *sample = shape + self.noise.next() * 0.1;
                    mean += *sample;
                }
                mean /= length as f32;
                // Smoothed twice round the string, so that its ends meet
                let smoothing = 0.12 + 0.55 * velocity * (0.4 + 0.6 * brightness);
                let mut smooth = 0.0;
                for _ in 0..2 {
                    for index in 0..length {
                        smooth += ((self.pluck[index] - mean) - smooth) * smoothing;
                        self.shaped[index] = smooth;
                    }
                }
                for index in 0..length {
                    let position = (self.write + self.line.len() - length + index) & self.mask;
                    self.line[position] = self.shaped[index] * velocity * 1.5;
                }
                velocity
            }
        };

        self.filter_delay = (1.0 - self.loop_lp) / self.loop_lp;
        self.delay = self.loop_delay(freq_hz);
        self.target_delay = self.delay;
        self.set_feedback(freq_hz);
        self.string_voice = self.settings.voice;
        self.loop_state = 0.0;
        self.sub_env = level;
        self.sub_attack = 0.0;
        self.note = Some(note);
        self.held = true;
    }

    /// One sample, the same for both channels, before Volume
    pub fn process(&mut self) -> f32 {
        let rate = self.sample_rate;
        let settings = &self.settings;

        // Only moves during a slide
        self.delay += (self.target_delay - self.delay) * self.glide;
        let freq_hz = rate / (self.delay + self.filter_delay);

        // The string
        let whole = self.delay.ceil();
        let fraction = whole - self.delay;
        let position = self.write + self.line.len() - whole as usize;
        let first = self.line[position & self.mask];
        let second = self.line[(position + 1) & self.mask];
        let string = first * (1.0 - fraction) + second * fraction;
        self.loop_state += (string - self.loop_state) * self.loop_lp;
        self.line[self.write] = self.loop_state * self.feedback;
        self.write = (self.write + 1) & self.mask;

        // The sine under it
        advance(&mut self.sub_phase, freq_hz / rate);
        if self.sub_attack < 1.0 {
            self.sub_attack = (self.sub_attack + self.sub_attack_step).min(1.0);
        }
        let sub = sine(self.sub_phase) * self.sub_env * self.sub_attack;
        self.sub_env *= if self.held { self.sub_decay_held } else { self.sub_decay_released };

        let raw = string + sub * settings.sub * self.sub_level;

        match settings.voice {
            Voice::Metal => {
                // The lows stay clean; what is over them is distorted and laid on top
                let low = self.split_low.process(raw).low;
                let high = self.drive_hp.process(raw).high;
                let driven = (high * (1.0 + settings.drive * 14.0)).tanh();
                let driven = self.drive_lp.process(driven).low;
                let driven = driven + self.mid_peak.process(driven).band * 0.6;
                let mixed = low * 1.1 + driven * (0.25 + settings.drive * 0.9) * 0.5;
                (self.dc.process(mixed).high * 1.2).tanh() * 0.8
            }
            Voice::Groove => {
                let gain = 1.0 + settings.drive * 3.0;
                let warm = self.tone.process(raw).low;
                let driven = (warm * gain).tanh() / gain * (1.0 + settings.drive);
                self.dc.process(driven).high * 0.9
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suite_common::measure::{brightness, peak, pitch_hz, rms};

    const SAMPLE_RATE: f32 = 48000.0;

    fn bass(voice: Voice, sample_rate: f32) -> Bass {
        let mut bass = Bass::new();
        bass.set_sample_rate(sample_rate);
        bass.set(&voice.settings());
        bass
    }

    fn render(bass: &mut Bass, sample_rate: f32, seconds: f32) -> Vec<f32> {
        (0..(seconds * sample_rate) as usize).map(|_| bass.process()).collect()
    }

    fn note(voice: Voice, note: u8, velocity: f32, seconds: f32) -> Vec<f32> {
        let mut bass = bass(voice, SAMPLE_RATE);
        bass.note_on(note, velocity);
        render(&mut bass, SAMPLE_RATE, seconds)
    }

    fn pitch(samples: &[f32], sample_rate: f32) -> f32 {
        pitch_hz(samples, sample_rate, 25.0, 500.0)
    }

    #[test]
    fn test_silent_until_a_note() {
        for voice in Voice::ALL {
            let mut bass = bass(voice, SAMPLE_RATE);
            assert!(render(&mut bass, SAMPLE_RATE, 0.1).iter().all(|&sample| sample == 0.0));
        }
    }

    #[test]
    fn test_notes_are_in_tune_at_every_rate() {
        for voice in Voice::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                // Low E, A, and the G an octave and a third above it
                for midi in [28u8, 33, 55] {
                    let mut bass = bass(voice, sample_rate);
                    bass.note_on(midi, 0.9);
                    let output = render(&mut bass, sample_rate, 0.5);
                    let found = pitch(&output[(0.2 * sample_rate) as usize..], sample_rate);
                    let cents = 1200.0 * (found / note_hz(midi as f32)).log2();
                    assert!(cents.abs() < 6.0, "{:?} note {} at {} Hz: {} cents off", voice, midi, sample_rate, cents);
                }
            }
        }
    }

    #[test]
    fn test_transpose_moves_the_pitch() {
        let mut bass = bass(Voice::Groove, SAMPLE_RATE);
        bass.set(&BassSettings {
            transpose: -12.0,
            ..Voice::Groove.settings()
        });
        bass.note_on(45, 0.9);
        let output = render(&mut bass, SAMPLE_RATE, 0.8);
        let found = pitch(&output[9600..], SAMPLE_RATE);
        assert!((found / note_hz(33.0) - 1.0).abs() < 0.005, "{} Hz", found);
    }

    #[test]
    fn test_a_held_note_rings_and_a_released_one_stops() {
        for voice in Voice::ALL {
            let held = note(voice, 33, 0.9, 1.0);
            assert!(rms(&held[38400..]) > 0.02, "{:?} held: {}", voice, rms(&held[38400..]));

            let mut bass = bass(voice, SAMPLE_RATE);
            bass.note_on(33, 0.9);
            render(&mut bass, SAMPLE_RATE, 0.3);
            bass.note_off(33);
            let released = render(&mut bass, SAMPLE_RATE, 0.7);
            assert!(rms(&released[24000..]) < 0.002, "{:?} released: {}", voice, rms(&released[24000..]));
        }
    }

    #[test]
    fn test_sustain_reaches_the_note_that_is_ringing() {
        // Without the sine under the string, which has its own decay
        let string_alone = BassSettings {
            sub: 0.0,
            ..Voice::Groove.settings()
        };
        let level_after = |sustain_s: f32| {
            let mut bass = bass(Voice::Groove, SAMPLE_RATE);
            bass.set(&string_alone);
            bass.note_on(33, 0.9);
            render(&mut bass, SAMPLE_RATE, 0.2);
            bass.set(&BassSettings { sustain_s, ..string_alone });
            rms(&render(&mut bass, SAMPLE_RATE, 1.0)[38400..])
        };
        assert!(level_after(SUSTAIN_MIN_S) < level_after(SUSTAIN_MAX_S) * 0.3);

        // And one that was let go stays let go
        let mut bass = bass(Voice::Groove, SAMPLE_RATE);
        bass.note_on(33, 0.9);
        render(&mut bass, SAMPLE_RATE, 0.2);
        bass.note_off(33);
        bass.set(&BassSettings {
            sustain_s: SUSTAIN_MAX_S,
            ..Voice::Groove.settings()
        });
        assert!(rms(&render(&mut bass, SAMPLE_RATE, 0.7)[24000..]) < 0.002);
    }

    #[test]
    fn test_a_ringing_note_keeps_its_string_and_its_pitch_when_the_voice_changes() {
        for (from, to) in [(Voice::Metal, Voice::Groove), (Voice::Groove, Voice::Metal)] {
            let mut bass = bass(from, SAMPLE_RATE);
            bass.note_on(33, 0.9);
            render(&mut bass, SAMPLE_RATE, 0.2);
            let loop_lp = bass.loop_lp;
            bass.set(&to.settings());
            assert_eq!(bass.loop_lp, loop_lp, "{:?} to {:?}", from, to);

            let found = pitch(&render(&mut bass, SAMPLE_RATE, 0.5)[4800..], SAMPLE_RATE);
            let cents = 1200.0 * (found / note_hz(33.0)).log2();
            assert!(cents.abs() < 6.0, "{:?} to {:?}: {} cents off", from, to, cents);

            // The next pluck is the new voice's
            bass.note_off(33);
            bass.note_on(33, 0.9);
            assert_ne!(bass.loop_lp, loop_lp);
        }
    }

    #[test]
    fn test_letting_go_of_another_note_changes_nothing() {
        let mut bass = bass(Voice::Metal, SAMPLE_RATE);
        bass.note_on(33, 0.9);
        render(&mut bass, SAMPLE_RATE, 0.1);
        bass.note_off(40);
        assert!(rms(&render(&mut bass, SAMPLE_RATE, 0.5)[19200..]) > 0.02);
    }

    #[test]
    fn test_overlapping_notes_slide_on_groove_and_are_plucked_on_metal() {
        // The level right after the second note: a pluck is a new attack, a slide is not
        let second_note = |voice: Voice| {
            let mut bass = bass(voice, SAMPLE_RATE);
            bass.note_on(33, 0.9);
            render(&mut bass, SAMPLE_RATE, 1.5);
            let before = peak(&render(&mut bass, SAMPLE_RATE, 0.05));
            bass.note_on(38, 0.9);
            let output = render(&mut bass, SAMPLE_RATE, 0.8);
            (peak(&output[..2400]) / before, pitch(&output[14400..], SAMPLE_RATE))
        };

        let (attack, found) = second_note(Voice::Groove);
        assert!(attack < 1.3, "Groove attack: {}", attack);
        assert!((found / note_hz(38.0) - 1.0).abs() < 0.01, "Groove slid to {} Hz", found);

        let (attack, found) = second_note(Voice::Metal);
        assert!(attack > 1.5, "Metal attack: {}", attack);
        assert!((found / note_hz(38.0) - 1.0).abs() < 0.01, "Metal plucked {} Hz", found);
    }

    #[test]
    fn test_velocity_and_brightness_open_the_sound() {
        for voice in Voice::ALL {
            let soft = note(voice, 33, 0.3, 0.3);
            let hard = note(voice, 33, 1.0, 0.3);
            // Less than the velocity says where the drive evens the level out
            assert!(peak(&hard) > peak(&soft) * 1.2, "{:?}: {} vs {}", voice, peak(&hard), peak(&soft));

            let with = |brightness: f32| {
                let mut bass = bass(voice, SAMPLE_RATE);
                bass.set(&BassSettings {
                    brightness,
                    ..voice.settings()
                });
                bass.note_on(33, 0.9);
                self::brightness(&render(&mut bass, SAMPLE_RATE, 0.3))
            };
            // Groove's tone filter keeps most of it back
            assert!(with(1.0) > with(0.0) * 1.1, "{:?}: {} vs {}", voice, with(1.0), with(0.0));
        }
    }

    #[test]
    fn test_the_voices_differ_and_metal_is_the_brighter() {
        let metal = note(Voice::Metal, 33, 0.9, 0.5);
        let groove = note(Voice::Groove, 33, 0.9, 0.5);
        assert!(brightness(&metal) > brightness(&groove) * 2.0);
    }

    #[test]
    fn test_output_is_bounded_at_every_setting() {
        for voice in Voice::ALL {
            for sample_rate in [44100.0, 192000.0] {
                let mut bass = bass(voice, sample_rate);
                bass.set(&BassSettings {
                    voice,
                    volume_db: 0.0,
                    drive: 1.0,
                    brightness: 1.0,
                    sub: 1.0,
                    tone_hz: TONE_MAX_HZ,
                    sustain_s: SUSTAIN_MAX_S,
                    slide_ms: 5.0,
                    transpose: -TRANSPOSE_RANGE,
                });
                let mut output = Vec::new();
                for midi in [0u8, 28, 127, 40, 28] {
                    bass.note_on(midi, 1.0);
                    output.extend(render(&mut bass, sample_rate, 0.2));
                }
                assert!(output.iter().all(|sample| sample.is_finite()));
                assert!(peak(&output) <= 2.0, "{:?} at {} Hz peaks at {}", voice, sample_rate, peak(&output));
            }
        }
    }

    /// Level, pitch error and brightness per voice and note:
    ///   cargo test -p bass --release bass_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bass_report() {
        for voice in Voice::ALL {
            println!("\n{:?}", voice);
            println!("{:<8}{:>8}{:>8}{:>10}{:>12}", "note", "peak", "rms", "cents", "brightness");
            for midi in [28u8, 33, 38, 43, 50, 55] {
                let output = note(voice, midi, 0.9, 1.0);
                let found = pitch(&output[9600..], SAMPLE_RATE);
                println!(
                    "{:<8}{:>8.3}{:>8.3}{:>10.1}{:>12.4}",
                    midi,
                    peak(&output),
                    rms(&output),
                    1200.0 * (found / note_hz(midi as f32)).log2(),
                    brightness(&output),
                );
            }
        }

        let mut bass = bass(Voice::Metal, SAMPLE_RATE);
        bass.note_on(28, 1.0);
        let frames = 480_000;
        let started = std::time::Instant::now();
        let mut sum = 0.0;
        for _ in 0..frames {
            sum += bass.process();
        }
        let per_frame_ns = started.elapsed().as_nanos() as f64 / frames as f64;
        println!("\nOne note at 48 kHz: {:.0} ns per frame ({:.2} % of a core) ({})", per_frame_ns, per_frame_ns * 48000.0 / 1e7, sum.is_finite());
    }

    /// Writes WAV files to target/renders for listening (bass_<voice>.wav):
    ///   cargo test -p bass --release render_wavs -- --ignored
    #[test]
    #[ignore]
    fn render_wavs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();

        for voice in Voice::ALL {
            // (beat, length in beats, note, velocity)
            let (line, bpm): (Vec<(f32, f32, u8, f32)>, f32) = match voice {
                Voice::Metal => {
                    let mut line = Vec::new();
                    for bar in 0..4 {
                        let start = bar as f32 * 4.0;
                        let root = [28, 28, 31, 26][bar];
                        for sixteenth in 0..16 {
                            let accent = if sixteenth % 4 == 0 { 1.0 } else { 0.8 };
                            let midi = if sixteenth == 14 { root + 12 } else { root };
                            line.push((start + sixteenth as f32 * 0.25, 0.2, midi, accent));
                        }
                    }
                    line.push((16.0, 4.0, 28, 1.0));
                    (line, 150.0)
                }
                Voice::Groove => {
                    let mut line = Vec::new();
                    for bar in 0..4 {
                        let start = bar as f32 * 4.0;
                        let root = [33, 33, 38, 36][bar];
                        line.push((start, 1.4, root, 0.85));
                        // A dead note, then a slide up to the fifth and back
                        line.push((start + 1.5, 0.1, root, 0.3));
                        line.push((start + 2.0, 0.8, root + 5, 0.8));
                        line.push((start + 2.5, 0.75, root + 7, 0.8));
                        line.push((start + 3.25, 0.5, root + 12, 0.7));
                    }
                    line.push((16.0, 4.0, 33, 0.85));
                    (line, 88.0)
                }
            };

            let samples_per_beat = SAMPLE_RATE * 60.0 / bpm;
            let mut events: Vec<(usize, u8, f32)> = Vec::new();
            for &(beat, length, midi, velocity) in &line {
                events.push(((beat * samples_per_beat) as usize, midi, velocity));
                events.push((((beat + length) * samples_per_beat) as usize, midi, 0.0));
            }
            events.sort_by_key(|event| event.0);
            let length = events.last().unwrap().0 + SAMPLE_RATE as usize;

            let mut bass = bass(voice, SAMPLE_RATE);
            let mut next = 0;
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE as u32,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let name = format!("bass_{}.wav", format!("{:?}", voice).to_lowercase());
            let mut writer = hound::WavWriter::create(dir.join(name), spec).unwrap();
            for sample in 0..length {
                while next < events.len() && events[next].0 <= sample {
                    let (_, midi, velocity) = events[next];
                    if velocity > 0.0 {
                        bass.note_on(midi, velocity);
                    } else {
                        bass.note_off(midi);
                    }
                    next += 1;
                }
                writer.write_sample((bass.process().clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        println!("Wrote renders to {}", dir.display());
    }
}
