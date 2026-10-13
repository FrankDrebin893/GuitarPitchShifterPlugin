use std::f32::consts::PI;

use suite_common::dsp::{advance, note_hz, sine, Hall, HallShape, Noise, Svf};

pub const VOLUME_MIN_DB: f32 = -40.0;
pub const VOLUME_MAX_DB: f32 = 12.0;
pub const VOLUME_DEFAULT_DB: f32 = -6.0;
pub const ATTACK_MIN_S: f32 = 0.01;
pub const ATTACK_MAX_S: f32 = 6.0;
pub const RELEASE_MIN_S: f32 = 0.05;
pub const RELEASE_MAX_S: f32 = 10.0;
pub const CUTOFF_MIN_HZ: f32 = 150.0;
pub const CUTOFF_MAX_HZ: f32 = 8000.0;
pub const DETUNE_MAX_CENTS: f32 = 30.0;
pub const REVERB_DECAY_MIN_S: f32 = 1.0;
pub const REVERB_DECAY_MAX_S: f32 = 20.0;
pub const SHIMMER_MAX: f32 = 0.6;

/// Everything the pedal sets except Volume, as the plugin reads it once per block
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadSettings {
    /// Time to swell to full level
    pub attack_s: f32,
    /// Time to fall by 40 dB once a key is let go
    pub release_s: f32,
    /// Where the filter of a middle C stands once the note has swelled
    pub cutoff_hz: f32,
    /// How far the outer two saws are from the middle one
    pub detune_cents: f32,
    /// 0.0-1.0: noise in each voice
    pub breath: f32,
    /// 0.0-1.0: how far the filters wander
    pub motion: f32,
    /// 0.0-1.0: level of the pad itself
    pub dry: f32,
    pub chorus_on: bool,
    pub chorus: f32,
    pub reverb: f32,
    /// Time for the tail to fall by 60 dB
    pub reverb_decay_s: f32,
    pub shimmer_on: bool,
    /// 0.0-0.6: how much of the tail comes back an octave up
    pub shimmer: f32,
}

impl Default for PadSettings {
    fn default() -> Self {
        Self {
            attack_s: 1.5,
            release_s: 3.5,
            cutoff_hz: 1600.0,
            detune_cents: 9.0,
            breath: 0.18,
            motion: 0.35,
            dry: 0.45,
            chorus_on: true,
            chorus: 0.6,
            reverb: 0.6,
            reverb_decay_s: 7.0,
            shimmer_on: true,
            shimmer: 0.3,
        }
    }
}

const VOICES: usize = 12;
// A voice under this is finished and can be given away
const ENV_FLOOR: f32 = 0.00001;
const ENV_FREE: f32 = 0.0001;
// The filters are set this often, counted across the host's blocks
const CONTROL_SAMPLES: u32 = 32;
// How quickly a switch is followed
const SWITCH_MS: f32 = 20.0;
// Damping of the voice filters (1 / Q)
const FILTER_K: f32 = 1.25;

#[derive(Clone, Copy, Default)]
struct Voice {
    note: Option<u8>,
    held: bool,
    env: f32,
    freq_hz: f32,
    velocity: f32,
    // Three saws and a slow wobble of its own
    phases: [f32; 3],
    wobble: f32,
    // Filter coefficients, and its state for left and right
    a: [f32; 3],
    left: [f32; 2],
    right: [f32; 2],
}

/// The step that takes the corner off a saw at its jump (PolyBLEP)
fn blep(phase: f32, step: f32) -> f32 {
    if phase < step {
        let t = phase / step;
        t + t - t * t - 1.0
    } else if phase > 1.0 - step {
        let t = (phase - 1.0) / step;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

fn saw(phase: &mut f32, step: f32) -> f32 {
    advance(phase, step);
    2.0 * *phase - 1.0 - blep(*phase, step)
}

/// A slow ambient pad: twelve voices of three detuned saws and breath through filters that
/// open as the note swells, a chorus, and a large hall whose tail feeds back an octave up.
pub struct ShimmerPad {
    sample_rate: f32,
    settings: PadSettings,
    noise: Noise,
    voices: [Voice; VOICES],
    control: u32,
    attack: f32,
    release: f32,
    detune: f32,
    // One slow wobble for every filter
    drift: f32,

    chorus_left: Vec<f32>,
    chorus_right: Vec<f32>,
    chorus_mask: usize,
    chorus_write: usize,
    chorus_phases: [f32; 2],

    hall: Hall,

    // The tail, read back at twice the speed through two windows that take turns
    shimmer_line: Vec<f32>,
    shimmer_mask: usize,
    shimmer_write: usize,
    shimmer_window: usize,
    shimmer_delay: usize,
    shimmer_hp: Svf,
    shimmer_lp: Svf,
    shimmer_feedback: f32,

    // Where the switches are, as gains that fade
    chorus_gain: f32,
    shimmer_gain: f32,
    switch_step: f32,
    out_hps: [Svf; 2],
}

impl ShimmerPad {
    pub fn new() -> Self {
        let mut pad = Self {
            sample_rate: 44100.0,
            settings: PadSettings::default(),
            noise: Noise::new(1),
            voices: [Voice::default(); VOICES],
            control: 0,
            attack: 1.0,
            release: 0.0,
            detune: 1.0,
            drift: 0.0,
            chorus_left: Vec::new(),
            chorus_right: Vec::new(),
            chorus_mask: 0,
            chorus_write: 0,
            chorus_phases: [0.0, 0.37],
            hall: Hall::new(HallShape {
                lines: [2171.0, 2539.0, 2927.0, 3331.0, 3719.0, 4177.0, 4621.0, 5059.0],
                sweep: 7.0,
                sweep_hz: 0.12,
                damp_hz: 5200.0,
            }),
            shimmer_line: Vec::new(),
            shimmer_mask: 0,
            shimmer_write: 0,
            shimmer_window: 2,
            shimmer_delay: 0,
            shimmer_hp: Svf::new(),
            shimmer_lp: Svf::new(),
            shimmer_feedback: 0.0,
            chorus_gain: 1.0,
            shimmer_gain: 1.0,
            switch_step: 1.0,
            out_hps: [Svf::new(), Svf::new()],
        };
        pad.set_sample_rate(44100.0);
        pad
    }

    /// Allocates the delay lines. Not for the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;

        // The chorus reaches back 17.5 ms at most
        let size = ((0.02 * sample_rate) as usize + 4).next_power_of_two();
        self.chorus_left = vec![0.0; size];
        self.chorus_right = vec![0.0; size];
        self.chorus_mask = size - 1;
        self.chorus_write = 0;

        self.shimmer_window = 2 * ((0.09 * sample_rate) as usize / 2);
        let size = (self.shimmer_window + 4).next_power_of_two();
        self.shimmer_line = vec![0.0; size];
        self.shimmer_mask = size - 1;
        self.shimmer_write = 0;
        self.shimmer_delay = 0;
        self.shimmer_hp.set(450.0, 0.7, sample_rate);
        self.shimmer_lp.set(6500.0, 0.7, sample_rate);

        self.hall.set_sample_rate(sample_rate);
        self.switch_step = 1.0 / (SWITCH_MS * 0.001 * sample_rate);
        for filter in &mut self.out_hps {
            filter.set(35.0, 0.7, sample_rate);
        }
        self.apply_settings();
        self.reset();
    }

    pub fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.control = 0;
        self.chorus_left.fill(0.0);
        self.chorus_right.fill(0.0);
        self.shimmer_line.fill(0.0);
        self.shimmer_feedback = 0.0;
        self.hall.reset();
        for filter in [&mut self.shimmer_hp, &mut self.shimmer_lp].into_iter().chain(&mut self.out_hps) {
            filter.reset();
        }
        self.chorus_gain = if self.settings.chorus_on { 1.0 } else { 0.0 };
        self.shimmer_gain = if self.settings.shimmer_on { 1.0 } else { 0.0 };
    }

    /// Once per block
    pub fn set(&mut self, settings: &PadSettings) {
        if *settings != self.settings {
            self.settings = *settings;
            self.apply_settings();
        }
    }

    fn apply_settings(&mut self) {
        let rate = self.sample_rate;
        let settings = &self.settings;
        // The swell is there after three of its time constants, the release 40 dB down after 4.6
        self.attack = 1.0 - (-1.0 / (settings.attack_s.max(ATTACK_MIN_S) / 3.0 * rate)).exp();
        self.release = (-1.0 / (settings.release_s.max(RELEASE_MIN_S) / 4.6 * rate)).exp();
        self.detune = 2.0f32.powf(settings.detune_cents / 1200.0);
        self.hall.set_decay(settings.reverb_decay_s);
    }

    /// Velocity is 0.0-1.0
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        if velocity <= 0.0 {
            return self.note_off(note);
        }
        let voices = &self.voices;
        let sounding = |voice: &Voice| voice.held || voice.env > ENV_FREE;
        // The voice that already has the note; else a free one; else the quietest that is
        // let go; else the quietest of all
        let quietest = |candidates: &dyn Fn(&Voice) -> bool| {
            (0..VOICES)
                .filter(|&index| candidates(&voices[index]))
                .min_by(|&a, &b| voices[a].env.total_cmp(&voices[b].env))
        };
        let index = (0..VOICES)
            .rev()
            .find(|&index| voices[index].note == Some(note) && sounding(&voices[index]))
            .or_else(|| (0..VOICES).find(|&index| !sounding(&voices[index])))
            .or_else(|| quietest(&|voice| !voice.held))
            .or_else(|| quietest(&|_| true))
            .unwrap_or(0);

        let voice = &mut self.voices[index];
        if voice.env < ENV_FREE {
            // A voice that starts from silence starts somewhere of its own
            voice.phases = std::array::from_fn(|_| self.noise.next_unipolar());
            voice.wobble = self.noise.next_unipolar();
            voice.left = [0.0; 2];
            voice.right = [0.0; 2];
            voice.a = [0.0; 3];
        }
        voice.note = Some(note);
        voice.held = true;
        voice.velocity = 0.4 + 0.6 * velocity.min(1.0);
        voice.freq_hz = note_hz(note as f32);
    }

    pub fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.note == Some(note) {
                voice.held = false;
            }
        }
    }

    /// Lets go of every note
    pub fn release(&mut self) {
        for voice in &mut self.voices {
            voice.held = false;
        }
    }

    /// Where every voice's filter stands, a few hundred times a second
    fn set_filters(&mut self) {
        let rate = self.sample_rate;
        let settings = &self.settings;
        let elapsed = CONTROL_SAMPLES as f32 / rate;
        advance(&mut self.drift, 0.06 * elapsed);
        let drift = sine(self.drift);

        for (index, voice) in self.voices.iter_mut().enumerate() {
            if !voice.held && voice.env <= ENV_FLOOR {
                continue;
            }
            advance(&mut voice.wobble, 0.13 * (1.0 + 0.4 * index as f32 / VOICES as f32) * elapsed);
            // Higher notes, harder notes and notes that have swelled are more open
            let cutoff_hz = settings.cutoff_hz
                * (voice.freq_hz / 261.6).powf(0.3)
                * (0.65 + 0.35 * voice.velocity)
                * (0.45 + 0.55 * voice.env)
                * (1.0 + settings.motion * (0.4 * drift + 0.3 * sine(voice.wobble)));
            let g = (PI * cutoff_hz.clamp(60.0, rate * 0.4) / rate).tan();
            let a1 = 1.0 / (1.0 + g * (g + FILTER_K));
            voice.a = [a1, g * a1, g * g * a1];
        }
    }

    /// One stereo frame, before Volume
    pub fn process(&mut self) -> (f32, f32) {
        if self.control == 0 {
            self.set_filters();
        }
        self.control = (self.control + 1) % CONTROL_SAMPLES;

        let rate = self.sample_rate;
        let settings = self.settings;

        let mut left = 0.0;
        let mut right = 0.0;
        for voice in &mut self.voices {
            if !voice.held && voice.env <= ENV_FLOOR {
                continue;
            }
            voice.env = if voice.held {
                voice.env + (1.0 - voice.env) * self.attack
            } else {
                voice.env * self.release
            };

            // The wobble also bends the pitch a little
            let step = voice.freq_hz * (1.0 + 0.0012 * sine(voice.wobble)) / rate;
            let low = saw(&mut voice.phases[0], step / self.detune);
            let middle = saw(&mut voice.phases[1], step);
            let high = saw(&mut voice.phases[2], step * self.detune);
            let breath = self.noise.next() * settings.breath * 1.5;
            let level = voice.env * voice.velocity;
            // The low saw leans left and the high one right
            let inputs = [
                (low * 0.8 + middle * 0.5 + high * 0.25 + breath) * level,
                (high * 0.8 + middle * 0.5 + low * 0.25 + breath) * level,
            ];

            let [a1, a2, a3] = voice.a;
            for (input, state, output) in
                [(inputs[0], &mut voice.left, &mut left), (inputs[1], &mut voice.right, &mut right)]
            {
                let v3 = input - state[1];
                let v1 = a1 * state[0] + a2 * v3;
                let v2 = state[1] + a2 * state[0] + a3 * v3;
                state[0] = 2.0 * v1 - state[0];
                state[1] = 2.0 * v2 - state[1];
                *output += v2;
            }
        }
        left *= 0.2;
        right *= 0.2;

        let target = if settings.chorus_on { 1.0 } else { 0.0 };
        self.chorus_gain += (target - self.chorus_gain).clamp(-self.switch_step, self.switch_step);
        let target = if settings.shimmer_on { 1.0 } else { 0.0 };
        self.shimmer_gain += (target - self.shimmer_gain).clamp(-self.switch_step, self.switch_step);

        // Chorus: each side hears the other, delayed by a time that slowly sways
        self.chorus_write = (self.chorus_write + 1) & self.chorus_mask;
        self.chorus_left[self.chorus_write] = left;
        self.chorus_right[self.chorus_write] = right;
        advance(&mut self.chorus_phases[0], 0.21 / rate);
        advance(&mut self.chorus_phases[1], 0.29 / rate);
        let tap = |line: &[f32], delay_s: f32, phase: f32| {
            let delay = (delay_s + 0.0035 * sine(phase)) * rate;
            let whole = delay.ceil();
            let fraction = whole - delay;
            let position = self.chorus_write + line.len() - whole as usize;
            line[position & self.chorus_mask] * (1.0 - fraction) + line[(position + 1) & self.chorus_mask] * fraction
        };
        let swayed_left = tap(&self.chorus_left, 0.011, self.chorus_phases[0]);
        let swayed_right = tap(&self.chorus_right, 0.014, self.chorus_phases[1]);
        let chorus = settings.chorus * self.chorus_gain;
        let pad_left = left * (1.0 - 0.4 * chorus) + swayed_right * chorus * 0.8;
        let pad_right = right * (1.0 - 0.4 * chorus) + swayed_left * chorus * 0.8;

        let (hall_left, hall_right) = self.hall.process((pad_left + pad_right) * 0.35 + self.shimmer_feedback);

        // Shimmer: the delay shrinks by a sample each sample, which reads the tail an octave
        // up; a second reader half a window away covers the first one's jump back
        self.shimmer_write = (self.shimmer_write + 1) & self.shimmer_mask;
        self.shimmer_line[self.shimmer_write] = (hall_left + hall_right) * 0.5;
        let window = self.shimmer_window;
        self.shimmer_delay = if self.shimmer_delay == 0 { window - 1 } else { self.shimmer_delay - 1 };
        let other_delay = (self.shimmer_delay + window / 2) % window;
        let read = |delay: usize| {
            let fade = (PI * delay as f32 / window as f32).sin();
            self.shimmer_line[(self.shimmer_write + self.shimmer_line.len() - delay) & self.shimmer_mask] * fade * fade
        };
        let octave_up = read(self.shimmer_delay) + read(other_delay);
        let octave_up = self.shimmer_lp.process(self.shimmer_hp.process(octave_up).high).low;
        self.shimmer_feedback = (octave_up * settings.shimmer * self.shimmer_gain * 1.2).tanh() * 0.5;

        let left = pad_left * settings.dry + hall_left * settings.reverb * 0.9;
        let right = pad_right * settings.dry + hall_right * settings.reverb * 0.9;
        (self.out_hps[0].process(left).high, self.out_hps[1].process(right).high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suite_common::measure::{brightness, peak, pitch_hz, rms};

    const SAMPLE_RATE: f32 = 48000.0;

    /// The voices alone, without their slow motion
    fn plain() -> PadSettings {
        PadSettings {
            attack_s: 0.05,
            release_s: 0.2,
            breath: 0.0,
            motion: 0.0,
            dry: 1.0,
            chorus_on: false,
            reverb: 0.0,
            shimmer_on: false,
            ..PadSettings::default()
        }
    }

    fn pad(settings: &PadSettings, sample_rate: f32) -> ShimmerPad {
        let mut pad = ShimmerPad::new();
        pad.set(settings);
        pad.set_sample_rate(sample_rate);
        pad
    }

    fn render(pad: &mut ShimmerPad, sample_rate: f32, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        (0..(seconds * sample_rate) as usize).map(|_| pad.process()).unzip()
    }

    fn hold(settings: &PadSettings, notes: &[u8], seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut pad = pad(settings, SAMPLE_RATE);
        for &note in notes {
            pad.note_on(note, 0.8);
        }
        render(&mut pad, SAMPLE_RATE, seconds)
    }

    #[test]
    fn test_silent_until_a_note() {
        let mut pad = pad(&PadSettings::default(), SAMPLE_RATE);
        let (left, right) = render(&mut pad, SAMPLE_RATE, 0.2);
        assert!(left.iter().chain(&right).all(|&sample| sample == 0.0));
    }

    #[test]
    fn test_notes_are_in_tune_at_every_rate() {
        for sample_rate in [44100.0, 96000.0] {
            for note in [45u8, 57, 69] {
                let settings = PadSettings {
                    detune_cents: 0.0,
                    ..plain()
                };
                let mut pad = pad(&settings, sample_rate);
                pad.note_on(note, 0.8);
                let (left, _) = render(&mut pad, sample_rate, 0.6);
                let found = pitch_hz(&left[(0.3 * sample_rate) as usize..], sample_rate, 60.0, 1000.0);
                assert!((found / note_hz(note as f32) - 1.0).abs() < 0.003, "Note {} at {} Hz", note, found);
            }
        }
    }

    #[test]
    fn test_swells_with_the_attack_and_fades_with_the_release() {
        let settings = PadSettings {
            attack_s: 1.0,
            release_s: 1.0,
            ..plain()
        };
        let mut pad = pad(&settings, SAMPLE_RATE);
        pad.note_on(57, 0.8);
        let (swell, _) = render(&mut pad, SAMPLE_RATE, 2.0);
        let window = |samples: &[f32], at: f32| {
            let start = (at * SAMPLE_RATE) as usize;
            rms(&samples[start..start + 4800])
        };
        // There after the attack time, and far from there after a tenth of it
        assert!(window(&swell, 0.02) < window(&swell, 1.8) * 0.4);
        assert!(window(&swell, 1.0) > window(&swell, 1.8) * 0.85);

        pad.note_off(57);
        let (fade, _) = render(&mut pad, SAMPLE_RATE, 2.0);
        let fall_db = 20.0 * (window(&fade, 1.0) / window(&swell, 1.8)).log10();
        assert!((-55.0..-32.0).contains(&fall_db), "Fell {} dB in the release time", fall_db);
        assert!(window(&fade, 1.8) < 0.0005);
    }

    #[test]
    fn test_detuned_saws_beat_and_the_sides_differ() {
        // How much the level moves over two seconds, 50 ms at a time
        let movement = |detune_cents: f32| {
            let settings = PadSettings { detune_cents, ..plain() };
            let (left, _) = hold(&settings, &[57], 3.0);
            let levels: Vec<f32> = left[48000..].chunks(2400).map(rms).collect();
            let loudest = levels.iter().fold(0.0f32, |loudest, &level| loudest.max(level));
            let quietest = levels.iter().fold(f32::MAX, |quietest, &level| quietest.min(level));
            loudest / quietest
        };
        assert!(movement(0.0) < 1.1, "Three saws in tune move by {}", movement(0.0));
        assert!(movement(20.0) > 1.3, "Detuned saws move by {}", movement(20.0));

        let (left, right) = hold(&plain(), &[57], 2.0);
        let side: Vec<f32> = left.iter().zip(&right).map(|(l, r)| l - r).collect();
        assert!(rms(&side[48000..]) > rms(&left[48000..]) * 0.2);
    }

    #[test]
    fn test_cutoff_and_velocity_open_the_filter() {
        let top = |cutoff_hz: f32, velocity: f32| {
            let mut pad = pad(&PadSettings { cutoff_hz, ..plain() }, SAMPLE_RATE);
            pad.note_on(45, velocity);
            brightness(&render(&mut pad, SAMPLE_RATE, 1.0).0[24000..])
        };
        assert!(top(6000.0, 0.8) > top(300.0, 0.8) * 5.0);
        assert!(top(1600.0, 1.0) > top(1600.0, 0.1) * 1.2);
    }

    #[test]
    fn test_thirteenth_note_takes_a_voice_and_a_repeated_note_keeps_its_own() {
        let mut pad = pad(&plain(), SAMPLE_RATE);
        for note in 48..60 {
            pad.note_on(note, 0.8);
        }
        render(&mut pad, SAMPLE_RATE, 0.3);
        assert!(pad.voices.iter().all(|voice| voice.held));

        pad.note_on(50, 0.8);
        assert_eq!(pad.voices.iter().filter(|voice| voice.note == Some(50)).count(), 1);

        // One that is let go is given away before one that is held
        pad.note_off(53);
        render(&mut pad, SAMPLE_RATE, 0.05);
        pad.note_on(72, 0.8);
        assert_eq!(pad.voices[5].note, Some(72));
        assert!(pad.voices.iter().all(|voice| voice.held));
    }

    #[test]
    fn test_release_lets_go_of_every_note() {
        let mut pad = pad(&plain(), SAMPLE_RATE);
        for note in [48, 55, 60] {
            pad.note_on(note, 0.8);
        }
        render(&mut pad, SAMPLE_RATE, 0.3);
        pad.release();
        let (left, _) = render(&mut pad, SAMPLE_RATE, 1.5);
        assert!(peak(&left[60000..]) < 0.0005);
    }

    #[test]
    fn test_reverb_rings_on_and_shimmer_adds_an_octave() {
        let tail = |shimmer_on: bool| {
            let settings = PadSettings {
                attack_s: 0.05,
                release_s: 0.1,
                breath: 0.0,
                motion: 0.0,
                chorus_on: false,
                shimmer_on,
                shimmer: SHIMMER_MAX,
                cutoff_hz: 500.0,
                ..PadSettings::default()
            };
            let mut pad = pad(&settings, SAMPLE_RATE);
            pad.note_on(45, 0.8);
            render(&mut pad, SAMPLE_RATE, 2.0);
            pad.note_off(45);
            render(&mut pad, SAMPLE_RATE, 2.0).0
        };
        let (without, with) = (tail(false), tail(true));
        // The voice is long gone: this is the hall
        assert!(rms(&without[48000..]) > 0.0005);
        assert!(brightness(&with[48000..]) > brightness(&without[48000..]) * 1.5);
    }

    #[test]
    fn test_chorus_switch_changes_the_sound() {
        let on = hold(&PadSettings { chorus_on: true, ..plain() }, &[57], 1.0).0;
        let off = hold(&plain(), &[57], 1.0).0;
        let difference: Vec<f32> = on.iter().zip(&off).map(|(a, b)| a - b).collect();
        assert!(rms(&difference[24000..]) > rms(&off[24000..]) * 0.2);
    }

    #[test]
    fn test_output_does_not_depend_on_how_it_is_asked_for() {
        // The filters are set by a count of samples, not by the host's blocks, so two pads
        // given the same notes at the same samples play the same
        let play = |settings_every: usize| {
            let mut pad = pad(&PadSettings::default(), SAMPLE_RATE);
            let mut output = Vec::new();
            for sample in 0..24000 {
                if sample % settings_every == 0 {
                    pad.set(&PadSettings::default());
                }
                if sample == 100 {
                    pad.note_on(57, 0.8);
                }
                output.push(pad.process().0);
            }
            output
        };
        assert_eq!(play(64), play(1000));
    }

    #[test]
    fn test_output_is_bounded_with_everything_up() {
        for sample_rate in [44100.0, 192000.0] {
            let settings = PadSettings {
                attack_s: ATTACK_MIN_S,
                release_s: RELEASE_MAX_S,
                cutoff_hz: CUTOFF_MAX_HZ,
                detune_cents: DETUNE_MAX_CENTS,
                breath: 1.0,
                motion: 1.0,
                dry: 1.0,
                chorus_on: true,
                chorus: 1.0,
                reverb: 1.0,
                reverb_decay_s: REVERB_DECAY_MAX_S,
                shimmer_on: true,
                shimmer: SHIMMER_MAX,
            };
            let mut pad = pad(&settings, sample_rate);
            for note in 0..12 {
                pad.note_on(36 + note * 5, 1.0);
            }
            let (left, right) = render(&mut pad, sample_rate, 6.0);
            assert!(left.iter().chain(&right).all(|sample| sample.is_finite()));
            let level = peak(&left).max(peak(&right));
            assert!(level < 40.0, "Peak {} at {} Hz", level, sample_rate);
        }
    }

    /// Level of a chord over time and the cost of twelve voices:
    ///   cargo test -p pad --release pad_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn pad_report() {
        let mut pad = pad(&PadSettings::default(), SAMPLE_RATE);
        for note in [45, 52, 57, 60, 64] {
            pad.note_on(note, 0.8);
        }
        println!("{:<10}{:>8}{:>8}{:>12}", "second", "peak", "rms", "brightness");
        for second in 0..12 {
            if second == 6 {
                pad.release();
            }
            let (left, right) = render(&mut pad, SAMPLE_RATE, 1.0);
            let mono: Vec<f32> = left.iter().zip(&right).map(|(l, r)| (l + r) * 0.5).collect();
            println!("{:<10}{:>8.3}{:>8.3}{:>12.4}", second, peak(&mono), rms(&mono), brightness(&mono));
        }

        let mut pad = self::pad(&PadSettings::default(), SAMPLE_RATE);
        for note in 48..60 {
            pad.note_on(note, 0.8);
        }
        let frames = 96_000;
        let started = std::time::Instant::now();
        let mut sum = 0.0;
        for _ in 0..frames {
            sum += pad.process().0;
        }
        let per_frame_ns = started.elapsed().as_nanos() as f64 / frames as f64;
        println!("\nTwelve voices at 48 kHz: {:.0} ns per frame ({:.2} % of a core) ({})", per_frame_ns, per_frame_ns * 48000.0 / 1e7, sum.is_finite());
    }

    /// Writes target/renders/pad.wav for listening:
    ///   cargo test -p pad --release render_wavs -- --ignored
    #[test]
    #[ignore]
    fn render_wavs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();

        // Three slow chords that overlap, each held for six seconds
        let chords: [(f32, [u8; 4]); 3] = [(0.0, [45, 52, 60, 64]), (5.0, [41, 53, 57, 60]), (10.0, [43, 50, 59, 62])];
        let length = SAMPLE_RATE as usize * 26;
        let volume = suite_common::dsp::db_to_gain(VOLUME_DEFAULT_DB);

        let mut pad = pad(&PadSettings::default(), SAMPLE_RATE);
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SAMPLE_RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(dir.join("pad.wav"), spec).unwrap();
        for sample in 0..length {
            for (start_s, notes) in chords {
                for note in notes {
                    if sample == (start_s * SAMPLE_RATE) as usize {
                        pad.note_on(note, 0.7);
                    }
                    if sample == ((start_s + 6.0) * SAMPLE_RATE) as usize {
                        pad.note_off(note);
                    }
                }
            }
            let (left, right) = pad.process();
            for channel in [left, right] {
                writer.write_sample(((channel * volume).clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
            }
        }
        writer.finalize().unwrap();
        println!("Wrote renders to {}", dir.display());
    }
}
