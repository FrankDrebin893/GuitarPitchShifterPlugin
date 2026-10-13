use std::f32::consts::TAU;

use suite_common::dsp::{advance, note_hz, sine, Hall, HallShape, Svf};

pub const VOLUME_MIN_DB: f32 = -40.0;
pub const VOLUME_MAX_DB: f32 = 12.0;
pub const VOLUME_DEFAULT_DB: f32 = -8.0;
pub const DECAY_MIN_S: f32 = 0.3;
pub const DECAY_MAX_S: f32 = 10.0;
pub const ATTACK_MIN_MS: f32 = 0.5;
pub const ATTACK_MAX_MS: f32 = 50.0;
pub const DELAY_MIN_MS: f32 = 50.0;
pub const DELAY_MAX_MS: f32 = 1500.0;
pub const FEEDBACK_MAX: f32 = 0.9;
pub const REVERB_DECAY_MIN_S: f32 = 0.5;
pub const REVERB_DECAY_MAX_S: f32 = 20.0;

/// Everything the pedal sets except Volume, as the plugin reads it once per block
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BellSettings {
    /// 0.0-1.0: how much glass there is in the strike
    pub brightness: f32,
    /// Time for the A above middle C to fall by 40 dB; lower bells ring longer
    pub decay_s: f32,
    pub attack_ms: f32,
    /// 0.0-1.0: level of the bells themselves
    pub dry: f32,
    pub delay_on: bool,
    pub delay_ms: f32,
    /// Level of each echo against the one before
    pub delay_feedback: f32,
    pub delay_mix: f32,
    pub reverb_on: bool,
    /// Time for the tail to fall by 60 dB
    pub reverb_decay_s: f32,
    pub reverb_mix: f32,
}

impl Default for BellSettings {
    fn default() -> Self {
        Self {
            brightness: 0.45,
            decay_s: 3.0,
            attack_ms: 6.0,
            dry: 0.55,
            delay_on: true,
            delay_ms: 445.0,
            delay_feedback: 0.45,
            delay_mix: 0.35,
            reverb_on: true,
            reverb_decay_s: 6.0,
            reverb_mix: 0.5,
        }
    }
}

const VOICES: usize = 12;
// A bell under this is finished
const ENV_FLOOR: f32 = 0.000005;
// Time constant of the glass in a strike
const GLASS_DECAY_S: f32 = 0.35;
// How quickly a switch and the delay time are followed
const SWITCH_MS: f32 = 10.0;
const TIME_GLIDE_MS: f32 = 60.0;

#[derive(Clone, Copy, Default)]
struct Bell {
    freq_hz: f32,
    carrier: f32,
    modulator: f32,
    octave: f32,
    env: f32,
    // How far the modulator bends the carrier: the glass, which fades before the tone does
    glass: f32,
    attack: f32,
    velocity: f32,
    decay: f32,
    pan: f32,
}

/// Soft glassy bells: twelve voices of two-operator FM with an octave partial, into a
/// ping-pong delay and a large hall.
pub struct GlassBells {
    sample_rate: f32,
    settings: BellSettings,
    bells: [Bell; VOICES],
    next_bell: usize,
    attack_step: f32,
    glass_decay: f32,

    echo_left: Vec<f32>,
    echo_right: Vec<f32>,
    echo_mask: usize,
    echo_write: usize,
    echo_damped: [f32; 2],
    // In samples; glides to where the knob is
    echo_time: f32,
    echo_time_target: f32,
    echo_glide: f32,

    hall: Hall,
    // Where the switches are, as gains that fade
    delay_gain: f32,
    reverb_gain: f32,
    switch_step: f32,
    out_hps: [Svf; 2],
}

impl GlassBells {
    pub fn new() -> Self {
        let mut bells = Self {
            sample_rate: 44100.0,
            settings: BellSettings::default(),
            bells: [Bell::default(); VOICES],
            next_bell: 0,
            attack_step: 1.0,
            glass_decay: 0.0,
            echo_left: Vec::new(),
            echo_right: Vec::new(),
            echo_mask: 0,
            echo_write: 0,
            echo_damped: [0.0; 2],
            echo_time: 0.0,
            echo_time_target: 0.0,
            echo_glide: 1.0,
            hall: Hall::new(HallShape {
                lines: [1777.0, 2053.0, 2399.0, 2687.0, 3001.0, 3329.0, 3697.0, 4001.0],
                sweep: 6.0,
                sweep_hz: 0.1,
                damp_hz: 6000.0,
            }),
            delay_gain: 1.0,
            reverb_gain: 1.0,
            switch_step: 1.0,
            out_hps: [Svf::new(), Svf::new()],
        };
        bells.set_sample_rate(44100.0);
        bells
    }

    /// Allocates the delay lines. Not for the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let size = ((DELAY_MAX_MS * 0.001 * sample_rate) as usize + 8).next_power_of_two();
        self.echo_left = vec![0.0; size];
        self.echo_right = vec![0.0; size];
        self.echo_mask = size - 1;
        self.echo_write = 0;
        self.echo_glide = 1.0 - (-1.0 / (TIME_GLIDE_MS * 0.001 * sample_rate)).exp();
        self.switch_step = 1.0 / (SWITCH_MS * 0.001 * sample_rate);
        self.glass_decay = (-1.0 / (GLASS_DECAY_S * sample_rate)).exp();
        self.hall.set_sample_rate(sample_rate);
        for filter in &mut self.out_hps {
            filter.set(80.0, 0.7, sample_rate);
        }
        self.apply_settings();
        self.echo_time = self.echo_time_target;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.bells = [Bell::default(); VOICES];
        self.echo_left.fill(0.0);
        self.echo_right.fill(0.0);
        self.echo_damped = [0.0; 2];
        self.hall.reset();
        for filter in &mut self.out_hps {
            filter.reset();
        }
        self.delay_gain = if self.settings.delay_on { 1.0 } else { 0.0 };
        self.reverb_gain = if self.settings.reverb_on { 1.0 } else { 0.0 };
    }

    /// Once per block
    pub fn set(&mut self, settings: &BellSettings) {
        if *settings != self.settings {
            self.settings = *settings;
            self.apply_settings();
        }
    }

    fn apply_settings(&mut self) {
        let rate = self.sample_rate;
        self.attack_step = 1.0 / (self.settings.attack_ms.max(ATTACK_MIN_MS) * 0.001 * rate);
        self.echo_time_target = (self.settings.delay_ms * 0.001 * rate).min((self.echo_left.len() - 4) as f32);
        self.hall.set_decay(self.settings.reverb_decay_s);
    }

    /// Velocity is 0.0-1.0. A bell rings out by itself: letting go of the key does nothing
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        if velocity <= 0.0 {
            return;
        }
        let velocity = velocity.min(1.0);
        let freq_hz = note_hz(note as f32);
        // Lower bells ring longer
        let decay_s = self.settings.decay_s / 4.6 * (440.0 / freq_hz).powf(0.35);

        self.bells[self.next_bell] = Bell {
            freq_hz,
            carrier: 0.0,
            modulator: 0.0,
            octave: 0.0,
            env: 0.00001,
            glass: (0.6 + 2.4 * self.settings.brightness) * (0.4 + 0.6 * velocity),
            attack: 0.0,
            velocity,
            decay: (-1.0 / (decay_s * self.sample_rate)).exp(),
            // Each note has its own place between the speakers
            pan: 0.5 + 0.35 * (note as f32 * 2.1).sin(),
        };
        self.next_bell = (self.next_bell + 1) % VOICES;
    }

    /// One stereo frame, before Volume
    pub fn process(&mut self) -> (f32, f32) {
        let rate = self.sample_rate;
        let settings = &self.settings;

        let mut left = 0.0;
        let mut right = 0.0;
        for bell in &mut self.bells {
            if bell.env <= ENV_FLOOR {
                continue;
            }
            if bell.attack < 1.0 {
                bell.attack = (bell.attack + self.attack_step).min(1.0);
            }
            let step = bell.freq_hz / rate;
            advance(&mut bell.carrier, step);
            advance(&mut bell.modulator, step * 3.5);
            advance(&mut bell.octave, step * 2.0);
            let tone = (TAU * bell.carrier + bell.glass * sine(bell.modulator)).sin()
                + 0.18 * sine(bell.octave) * bell.glass;
            let sample = tone * bell.env * bell.attack * bell.velocity;
            // Full level until the strike has faded in, then the decay
            if bell.attack >= 1.0 {
                bell.env *= bell.decay;
            } else {
                bell.env = 1.0;
            }
            bell.glass *= self.glass_decay;
            left += sample * (1.0 - bell.pan);
            right += sample * bell.pan;
        }
        left *= 0.35;
        right *= 0.35;

        let target = if settings.delay_on { 1.0 } else { 0.0 };
        self.delay_gain += (target - self.delay_gain).clamp(-self.switch_step, self.switch_step);
        let target = if settings.reverb_on { 1.0 } else { 0.0 };
        self.reverb_gain += (target - self.reverb_gain).clamp(-self.switch_step, self.switch_step);

        // Ping-pong delay: the bells go in on the left and bounce to the right and back
        self.echo_time += (self.echo_time_target - self.echo_time) * self.echo_glide;
        self.echo_write = (self.echo_write + 1) & self.echo_mask;
        let whole = self.echo_time.ceil();
        let fraction = whole - self.echo_time;
        let position = self.echo_write + self.echo_left.len() - whole as usize;
        let (first, second) = (position & self.echo_mask, (position + 1) & self.echo_mask);
        let delayed_left = self.echo_left[first] * (1.0 - fraction) + self.echo_left[second] * fraction;
        let delayed_right = self.echo_right[first] * (1.0 - fraction) + self.echo_right[second] * fraction;
        // Each bounce is a little darker
        self.echo_damped[0] += (delayed_left - self.echo_damped[0]) * 0.35;
        self.echo_damped[1] += (delayed_right - self.echo_damped[1]) * 0.35;
        self.echo_left[self.echo_write] = (left + right) * 0.5 + self.echo_damped[1] * settings.delay_feedback;
        self.echo_right[self.echo_write] = self.echo_damped[0] * settings.delay_feedback;
        let echo_left = self.echo_damped[0] * self.delay_gain;
        let echo_right = self.echo_damped[1] * self.delay_gain;

        let (hall_left, hall_right) = self.hall.process((left + right) * 0.3 + (echo_left + echo_right) * 0.25);

        let wet = settings.reverb_mix * self.reverb_gain;
        let left = left * settings.dry + echo_left * settings.delay_mix + hall_left * wet;
        let right = right * settings.dry + echo_right * settings.delay_mix + hall_right * wet;
        (self.out_hps[0].process(left).high, self.out_hps[1].process(right).high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suite_common::measure::{brightness, decay_time_s, peak, rms, zero_crossing_freq};

    const SAMPLE_RATE: f32 = 48000.0;

    /// The bells alone
    fn plain() -> BellSettings {
        BellSettings {
            delay_on: false,
            reverb_on: false,
            dry: 1.0,
            ..BellSettings::default()
        }
    }

    fn bells(settings: &BellSettings, sample_rate: f32) -> GlassBells {
        let mut bells = GlassBells::new();
        bells.set(settings);
        bells.set_sample_rate(sample_rate);
        bells
    }

    fn render(bells: &mut GlassBells, sample_rate: f32, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        (0..(seconds * sample_rate) as usize).map(|_| bells.process()).unzip()
    }

    fn strike(settings: &BellSettings, note: u8, velocity: f32, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut bells = bells(settings, SAMPLE_RATE);
        bells.note_on(note, velocity);
        render(&mut bells, SAMPLE_RATE, seconds)
    }

    #[test]
    fn test_silent_until_a_note() {
        let mut bells = bells(&BellSettings::default(), SAMPLE_RATE);
        let (left, right) = render(&mut bells, SAMPLE_RATE, 0.2);
        assert!(left.iter().chain(&right).all(|&sample| sample == 0.0));
    }

    #[test]
    fn test_bells_are_in_tune_at_every_rate() {
        for sample_rate in [44100.0, 96000.0] {
            for note in [57u8, 69, 81] {
                let settings = BellSettings { decay_s: 8.0, ..plain() };
                let mut bells = bells(&settings, sample_rate);
                bells.note_on(note, 0.8);
                let (left, _) = render(&mut bells, sample_rate, 3.0);
                // Once the glass has gone the carrier is what is left
                let found = zero_crossing_freq(&left[(2.5 * sample_rate) as usize..], sample_rate);
                assert!((found / note_hz(note as f32) - 1.0).abs() < 0.003, "Note {} at {} Hz", note, found);
            }
        }
    }

    #[test]
    fn test_decay_follows_the_knob_and_low_bells_ring_longer() {
        let decay = |decay_s: f32, note: u8| {
            let settings = BellSettings { decay_s, ..plain() };
            decay_time_s(&strike(&settings, note, 0.8, decay_s * 3.0).0, SAMPLE_RATE, -40.0)
        };
        // The knob is the time the A above middle C takes to fall by 40 dB
        assert!((decay(1.0, 69) - 1.0).abs() < 0.15, "{} s", decay(1.0, 69));
        assert!((decay(3.0, 69) - 3.0).abs() < 0.4, "{} s", decay(3.0, 69));
        assert!(decay(1.0, 45) > decay(1.0, 69) * 1.3);
    }

    #[test]
    fn test_velocity_and_brightness_add_glass() {
        let glass = |brightness: f32, velocity: f32| {
            let settings = BellSettings { brightness, ..plain() };
            self::brightness(&strike(&settings, 60, velocity, 0.2).0)
        };
        assert!(glass(1.0, 0.8) > glass(0.0, 0.8) * 2.0);
        assert!(glass(0.45, 1.0) > glass(0.45, 0.2) * 1.3);
    }

    #[test]
    fn test_twelve_bells_ring_together_and_the_thirteenth_takes_the_oldest() {
        let mut bells = bells(&plain(), SAMPLE_RATE);
        for note in 60..72 {
            bells.note_on(note, 0.8);
        }
        assert!(bells.bells.iter().all(|bell| bell.env > ENV_FLOOR));
        bells.note_on(84, 0.8);
        assert_eq!(bells.bells[0].freq_hz, note_hz(84.0));
        assert_eq!(bells.bells[1].freq_hz, note_hz(61.0));
    }

    #[test]
    fn test_delay_bounces_from_left_to_right() {
        let settings = BellSettings {
            reverb_on: false,
            dry: 0.0,
            delay_mix: 1.0,
            delay_ms: 400.0,
            decay_s: 0.3,
            ..BellSettings::default()
        };
        // A bell that sits in the middle
        let (left, right) = strike(&settings, 60, 0.8, 1.2);
        let window = |samples: &[f32], from: f32, to: f32| {
            rms(&samples[(from * SAMPLE_RATE) as usize..(to * SAMPLE_RATE) as usize])
        };

        // Nothing before the first echo, which is on the left; the second is on the right
        assert!(window(&left, 0.0, 0.35) < 1e-4 && window(&right, 0.0, 0.75) < 1e-4);
        assert!(window(&left, 0.4, 0.7) > 0.005);
        assert!(window(&right, 0.8, 1.1) > 0.002);
        assert!(window(&right, 0.8, 1.1) < window(&left, 0.4, 0.7));
    }

    #[test]
    fn test_switches_take_the_effects_away() {
        let tail = |settings: &BellSettings| {
            let (left, _) = strike(settings, 69, 0.8, 2.0);
            rms(&left[72000..])
        };
        let short = BellSettings {
            decay_s: 0.3,
            ..BellSettings::default()
        };
        let both_off = BellSettings {
            delay_on: false,
            reverb_on: false,
            ..short
        };
        assert!(tail(&short) > 0.001);
        assert!(tail(&both_off) < 1e-5, "{}", tail(&both_off));
    }

    #[test]
    fn test_reverb_decay_follows_the_knob() {
        let tail = |reverb_decay_s: f32| {
            let settings = BellSettings {
                delay_on: false,
                decay_s: 0.3,
                reverb_decay_s,
                ..BellSettings::default()
            };
            let (left, _) = strike(&settings, 69, 0.8, 4.0);
            rms(&left[144000..])
        };
        assert!(tail(8.0) > tail(1.0) * 20.0);
    }

    #[test]
    fn test_output_is_bounded_with_everything_up() {
        for sample_rate in [44100.0, 192000.0] {
            let settings = BellSettings {
                brightness: 1.0,
                decay_s: DECAY_MAX_S,
                attack_ms: ATTACK_MIN_MS,
                dry: 1.0,
                delay_on: true,
                delay_ms: DELAY_MIN_MS,
                delay_feedback: FEEDBACK_MAX,
                delay_mix: 1.0,
                reverb_on: true,
                reverb_decay_s: REVERB_DECAY_MAX_S,
                reverb_mix: 1.0,
            };
            let mut bells = bells(&settings, sample_rate);
            let mut output = Vec::new();
            for round in 0..4 {
                for note in 0..12 {
                    bells.note_on(48 + note * 3 + round, 1.0);
                }
                let (left, right) = render(&mut bells, sample_rate, 0.5);
                output.extend(left);
                output.extend(right);
            }
            assert!(output.iter().all(|sample| sample.is_finite()));
            assert!(peak(&output) < 12.0, "Peak {} at {} Hz", peak(&output), sample_rate);
        }
    }

    /// Level per note and the cost of twelve bells:
    ///   cargo test -p bells --release bells_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bells_report() {
        println!("{:<8}{:>8}{:>8}{:>12}{:>12}", "note", "peak", "rms", "decay (s)", "brightness");
        for note in [48u8, 60, 72, 84, 96] {
            let (left, right) = strike(&BellSettings::default(), note, 0.8, 12.0);
            let mono: Vec<f32> = left.iter().zip(&right).map(|(l, r)| (l + r) * 0.5).collect();
            println!(
                "{:<8}{:>8.3}{:>8.3}{:>12.2}{:>12.4}",
                note,
                peak(&mono),
                rms(&mono[..48000]),
                decay_time_s(&mono, SAMPLE_RATE, -40.0),
                brightness(&mono[..9600]),
            );
        }

        let mut bells = bells(&BellSettings::default(), SAMPLE_RATE);
        for note in 60..72 {
            bells.note_on(note, 0.8);
        }
        let frames = 96_000;
        let started = std::time::Instant::now();
        let mut sum = 0.0;
        for _ in 0..frames {
            sum += bells.process().0;
        }
        let per_frame_ns = started.elapsed().as_nanos() as f64 / frames as f64;
        println!("\nTwelve bells at 48 kHz: {:.0} ns per frame ({:.2} % of a core) ({})", per_frame_ns, per_frame_ns * 48000.0 / 1e7, sum.is_finite());
    }

    /// Writes target/renders/bells.wav for listening:
    ///   cargo test -p bells --release render_wavs -- --ignored
    #[test]
    #[ignore]
    fn render_wavs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();

        // A sparse line over two chords: (beat, note, velocity)
        let line: [(f32, u8, f32); 14] = [
            (0.0, 76, 0.7),
            (1.5, 83, 0.5),
            (3.0, 79, 0.6),
            (4.0, 88, 0.45),
            (6.0, 81, 0.6),
            (7.5, 76, 0.5),
            (8.0, 74, 0.7),
            (9.5, 81, 0.5),
            (11.0, 78, 0.6),
            (12.0, 86, 0.45),
            (14.0, 81, 0.55),
            (15.0, 69, 0.6),
            (16.0, 76, 0.7),
            (16.0, 64, 0.5),
        ];
        let samples_per_beat = SAMPLE_RATE * 60.0 / 72.0;
        let length = (16.0 * samples_per_beat) as usize + SAMPLE_RATE as usize * 10;
        let volume = suite_common::dsp::db_to_gain(VOLUME_DEFAULT_DB);

        let mut bells = bells(&BellSettings::default(), SAMPLE_RATE);
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SAMPLE_RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(dir.join("bells.wav"), spec).unwrap();
        let mut next = 0;
        for sample in 0..length {
            while next < line.len() && (line[next].0 * samples_per_beat) as usize <= sample {
                bells.note_on(line[next].1, line[next].2);
                next += 1;
            }
            let (left, right) = bells.process();
            for channel in [left, right] {
                writer.write_sample(((channel * volume).clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
            }
        }
        writer.finalize().unwrap();
        println!("Wrote renders to {}", dir.display());
    }
}
