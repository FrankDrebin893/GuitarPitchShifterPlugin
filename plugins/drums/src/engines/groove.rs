use super::{advance, coef, db, sine, BusDrive, CombRoom, MetalCluster, Rest, ENV_FLOOR};
use crate::drum_kit::{tom_index, Mix};
use crate::dsp::{Noise, Svf};
use crate::voices::tom::TOM_COUNT;
use crate::voices::Hit;

// Output level: kick and snare as loud as Rock's (by `kit_report`), and peaks that stay under
// the knee of the bus clipper, so the kit's own saturation is the only one it gets
const LEVEL: f32 = 0.4;

// Balance of the kit with the level controls at 0 dB
const TOM_TRIM_DB: f32 = -2.0;
const HAT_TRIM_DB: f32 = -2.0;
const SHAKER_TRIM_DB: f32 = -7.0;
const CYMBAL_TRIM_DB: f32 = -6.0;

// The room at the default Room setting (25 %), as a share of its full level
const ROOM_AT_DEFAULT: f32 = 0.22;
const ROOM_SCALE: f32 = ROOM_AT_DEFAULT / 0.25;

const TAPE_DRIVE: f32 = 0.2;
// 0.0 leaves the top open at 18 kHz, 1.0 closes it to 3.5 kHz
const WARMTH: f32 = 0.4;

// Lowest (floor) tom to highest: pitch, and position from -1.0 (left) to 1.0 (right)
const TOM_FREQS_HZ: [f32; TOM_COUNT] = [82.0, 98.0, 118.0, 136.0, 158.0, 182.0];
const TOM_PANS: [f32; TOM_COUNT] = [0.35, 0.25, 0.05, -0.1, -0.2, -0.3];

// Velocity to level
const VELOCITY_CURVE: f32 = 1.3;

#[derive(Default)]
struct Kick {
    on: bool,
    env: f32,
    pitch_env: f32,
    beater_env: f32,
    phase: f32,
    tune: f32,
    decay: f32,
    pitch_decay: f32,
    beater_decay: f32,
}

#[derive(Default)]
struct Snare {
    on: bool,
    // How hard it was hit: soft hits are darker
    level: f32,
    body_env: f32,
    pitch_env: f32,
    snap_env: f32,
    wire_env: f32,
    phases: [f32; 2],
    tune: f32,
    body_decay: f32,
    pitch_decay: f32,
    snap_decay: f32,
    wire_decay: f32,
}

#[derive(Default)]
struct SideStick {
    envs: [f32; 2],
    noise_env: f32,
    phases: [f32; 2],
    tune: f32,
    decays: [f32; 2],
    noise_decay: f32,
}

#[derive(Default)]
struct Tom {
    on: bool,
    env: f32,
    pitch_env: f32,
    noise_env: f32,
    phase: f32,
    tune: f32,
    decay: f32,
    pitch_decay: f32,
    noise_decay: f32,
}

#[derive(Default)]
struct Hat {
    env: f32,
    // How hard it was hit: soft hits have less top
    level: f32,
    decay: f32,
}

#[derive(Default)]
struct Ride {
    env: f32,
    ping: f32,
    bell: f32,
    bell_phases: [f32; 2],
    decay: f32,
    ping_decay: f32,
    bell_decay: f32,
}

/// Level of a cymbal or the shaker: a fade in, then a decay
#[derive(Default)]
struct Swell {
    env: f32,
    attack: f32,
    decay: f32,
}

impl Swell {
    fn trigger(&mut self, level: f32, tau_s: f32, sample_rate: f32) {
        self.env = level;
        self.attack = 0.0;
        self.decay = coef(tau_s, sample_rate);
    }

    fn fade_in(&mut self, attack_s: f32, sample_rate: f32) {
        if self.attack < 1.0 {
            self.attack = (self.attack + 1.0 / (attack_s * sample_rate)).min(1.0);
        }
    }
}

/// The Groove kit: warm and soft, with a round kick under a felt beater, a snare whose
/// ghost notes are darker and shorter, dark cymbals and a shaker. One voice per piece.
pub struct GrooveKit {
    sample_rate: f32,
    active: bool,
    rest: Rest,
    noise: Noise,

    kick: Kick,
    kick_lp: Svf,

    snare: Snare,
    snare_bp: Svf,
    snare_lp: Svf,
    snare_snap: Svf,
    side_stick: SideStick,
    side_stick_bp: Svf,

    toms: [Tom; TOM_COUNT],

    hat: Hat,
    hat_metal: MetalCluster,
    hat_hp: Svf,
    hat_bp: Svf,

    ride: Ride,
    ride_metal: MetalCluster,
    ride_hp: Svf,
    ride_bp: Svf,

    crash: Swell,
    crash_metal: MetalCluster,
    crash_hp: Svf,
    crash_lp: Svf,

    shaker: Swell,
    shaker_bp: Svf,
    shaker_hp: Svf,

    room: CombRoom,
    drive: BusDrive,
    out_hps: [Svf; 2],
    tones: [Svf; 2],
}

impl GrooveKit {
    pub fn new(seed: u32) -> Self {
        let mut kit = Self {
            sample_rate: 44100.0,
            active: false,
            rest: Rest::new(),
            noise: Noise::new(seed),
            kick: Kick::default(),
            kick_lp: Svf::new(),
            snare: Snare::default(),
            snare_bp: Svf::new(),
            snare_lp: Svf::new(),
            snare_snap: Svf::new(),
            side_stick: SideStick::default(),
            side_stick_bp: Svf::new(),
            toms: std::array::from_fn(|_| Tom::default()),
            hat: Hat::default(),
            hat_metal: MetalCluster::new(380.0),
            hat_hp: Svf::new(),
            hat_bp: Svf::new(),
            ride: Ride::default(),
            ride_metal: MetalCluster::new(460.0),
            ride_hp: Svf::new(),
            ride_bp: Svf::new(),
            crash: Swell::default(),
            crash_metal: MetalCluster::new(330.0),
            crash_hp: Svf::new(),
            crash_lp: Svf::new(),
            shaker: Swell::default(),
            shaker_bp: Svf::new(),
            shaker_hp: Svf::new(),
            room: CombRoom::new(0.8, 0.78, 0.45),
            drive: BusDrive::new(0.5 + 2.5 * TAPE_DRIVE),
            out_hps: [Svf::new(), Svf::new()],
            tones: [Svf::new(), Svf::new()],
        };
        kit.set_filters();
        kit
    }

    /// Allocates the room's delay lines. Not for the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.rest.set_sample_rate(sample_rate);
        self.room.set_sample_rate(sample_rate);
        self.set_filters();
        self.reset();
    }

    fn set_filters(&mut self) {
        let rate = self.sample_rate;
        self.kick_lp.set(1600.0, 0.7, rate);
        self.snare_bp.set(2600.0, 0.6, rate);
        self.snare_lp.set(6000.0, 0.7, rate);
        self.snare_snap.set(4200.0, 0.9, rate);
        self.side_stick_bp.set(2400.0, 2.0, rate);
        self.hat_hp.set(5200.0, 0.7, rate);
        self.hat_bp.set(7200.0, 0.8, rate);
        self.ride_hp.set(2800.0, 0.7, rate);
        self.ride_bp.set(4800.0, 1.3, rate);
        self.crash_hp.set(1900.0, 0.7, rate);
        self.crash_lp.set(8500.0, 0.7, rate);
        self.shaker_bp.set(6800.0, 1.3, rate);
        self.shaker_hp.set(4200.0, 0.7, rate);
        let tone_hz = 18000.0 * (3500.0f32 / 18000.0).powf(WARMTH);
        for (out_hp, tone) in self.out_hps.iter_mut().zip(&mut self.tones) {
            out_hp.set(30.0, 0.7, rate);
            tone.set(tone_hz, 0.6, rate);
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.kick = Kick::default();
        self.snare = Snare::default();
        self.side_stick = SideStick::default();
        for tom in &mut self.toms {
            *tom = Tom::default();
        }
        self.hat = Hat::default();
        self.ride = Ride::default();
        self.crash = Swell::default();
        self.shaker = Swell::default();

        for cluster in [&mut self.hat_metal, &mut self.ride_metal, &mut self.crash_metal] {
            cluster.reset();
        }
        for filter in [
            &mut self.kick_lp,
            &mut self.snare_bp,
            &mut self.snare_lp,
            &mut self.snare_snap,
            &mut self.side_stick_bp,
            &mut self.hat_hp,
            &mut self.hat_bp,
            &mut self.ride_hp,
            &mut self.ride_bp,
            &mut self.crash_hp,
            &mut self.crash_lp,
            &mut self.shaker_bp,
            &mut self.shaker_hp,
        ]
        .into_iter()
        .chain(&mut self.out_hps)
        .chain(&mut self.tones)
        {
            filter.reset();
        }
        self.room.reset();
    }

    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Only called with notes of the kit's map
    pub fn note_on(&mut self, note: u8, hit: &Hit) {
        let rate = self.sample_rate;
        let level = hit.velocity.powf(VELOCITY_CURVE);
        // Time constant of a decay, with Damping
        let decay = |tau_s: f32| coef(tau_s * hit.decay, rate);

        match note {
            35 | 36 => {
                self.kick = Kick {
                    on: true,
                    env: level,
                    pitch_env: 1.0,
                    beater_env: level,
                    phase: 0.0,
                    tune: hit.tune,
                    decay: decay(0.17),
                    pitch_decay: coef(0.032, rate),
                    beater_decay: coef(0.0025, rate),
                };
            }
            37 => {
                self.side_stick = SideStick {
                    envs: [level; 2],
                    noise_env: level,
                    phases: [0.0; 2],
                    tune: hit.tune,
                    decays: [decay(0.028), decay(0.012)],
                    noise_decay: coef(0.006, rate),
                };
            }
            38 | 40 => {
                self.snare = Snare {
                    on: true,
                    level,
                    body_env: level,
                    pitch_env: 1.0,
                    snap_env: level,
                    wire_env: level,
                    phases: [0.0; 2],
                    tune: hit.tune,
                    body_decay: decay(0.07),
                    pitch_decay: coef(0.018, rate),
                    snap_decay: coef(0.004, rate),
                    // Harder hits shake the wires for longer
                    wire_decay: decay(0.06 + 0.12 * level),
                };
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                self.toms[tom_index(note)] = Tom {
                    on: true,
                    env: level,
                    pitch_env: 1.0,
                    noise_env: level,
                    phase: 0.0,
                    tune: hit.tune,
                    decay: decay(0.3),
                    pitch_decay: coef(0.05, rate),
                    noise_decay: coef(0.01, rate),
                };
            }
            // The closed and pedal sounds have a short decay, which chokes an open hi-hat
            42 => self.hat = Hat { env: level, level, decay: decay(0.022 + 0.025 * level) },
            44 => self.hat = Hat { env: level * 0.5, level: level * 0.6, decay: decay(0.016) },
            46 => self.hat = Hat { env: level, level, decay: decay(0.32) },
            51 | 59 => {
                self.ride.env = level * 0.7;
                self.ride.ping = level;
                self.set_ride_decays(hit);
            }
            53 => {
                self.ride.env = level * 0.4;
                self.ride.ping = level * 0.4;
                self.ride.bell = level;
                self.set_ride_decays(hit);
            }
            // One dark crash for every crash of the map; the kit has no china of its own
            49 | 52 | 57 => self.crash.trigger(level, 1.3 * hit.decay, rate),
            55 => self.crash.trigger(level, 0.35 * hit.decay, rate),
            69 | 70 | 82 => self.shaker.trigger(level, 0.045 * hit.decay, rate),
            _ => return,
        }
        self.active = true;
        self.rest.wake();
    }

    fn set_ride_decays(&mut self, hit: &Hit) {
        self.ride.decay = coef(0.9 * hit.decay, self.sample_rate);
        self.ride.ping_decay = coef(0.045, self.sample_rate);
        self.ride.bell_decay = coef(0.5 * hit.decay, self.sample_rate);
    }

    /// One stereo frame
    pub fn process(&mut self, mix: &Mix) -> (f32, f32) {
        if !self.active {
            return (0.0, 0.0);
        }

        let rate = self.sample_rate;
        let tom_gain = mix.toms * db(TOM_TRIM_DB);
        let hat_gain = mix.hihat * db(HAT_TRIM_DB);
        let shaker_gain = mix.hihat * db(SHAKER_TRIM_DB);
        let cymbal_gain = mix.cymbals * db(CYMBAL_TRIM_DB);

        // Every noise part of the kit hears the same noise, each through its own filters
        let noise = self.noise.next();
        let mut left = 0.0;
        let mut right = 0.0;
        let mut send = 0.0;

        // Kick: a round sine that falls from 114 to 52 Hz, with a soft felt beater
        let mut kick = 0.0;
        if self.kick.on {
            let voice = &mut self.kick;
            advance(&mut voice.phase, (52.0 + 62.0 * voice.pitch_env) * voice.tune, rate);
            let body = sine(voice.phase) * voice.env;
            let beater = self.kick_lp.process(noise).low * voice.beater_env * 0.9;
            kick = (body * 1.3 + beater).tanh() * mix.kick;
            voice.env *= voice.decay;
            voice.pitch_env *= voice.pitch_decay;
            voice.beater_env *= voice.beater_decay;
            if voice.env < ENV_FLOOR {
                voice.on = false;
            }
        }
        left += kick;
        right += kick;
        send += kick * 0.08;

        // Snare: warm body and soft wires
        let mut snare = 0.0;
        if self.snare.on {
            let voice = &mut self.snare;
            advance(&mut voice.phases[0], (178.0 + 30.0 * voice.pitch_env) * voice.tune, rate);
            advance(&mut voice.phases[1], (318.0 + 25.0 * voice.pitch_env) * voice.tune, rate);
            let body =
                (sine(voice.phases[0]) * 0.9 + sine(voice.phases[1]) * 0.35) * voice.body_env;
            let band = self.snare_bp.process(noise).band;
            let low = self.snare_lp.process(noise).low;
            let wires = (band * (0.35 + 0.65 * voice.level) + low * 0.45) * voice.wire_env;
            let snap = self.snare_snap.process(noise).band * voice.snap_env * 1.4;
            snare = ((body * 1.3 + wires * 1.1 + snap * voice.level) * 1.1).tanh() * mix.snare;
            voice.body_env *= voice.body_decay;
            voice.pitch_env *= voice.pitch_decay;
            voice.snap_env *= voice.snap_decay;
            voice.wire_env *= voice.wire_decay;
            if voice.wire_env < ENV_FLOOR && voice.body_env < ENV_FLOOR {
                voice.on = false;
            }
        }
        // Side stick: a woody click
        if self.side_stick.envs[0] > ENV_FLOOR || self.side_stick.noise_env > ENV_FLOOR {
            let voice = &mut self.side_stick;
            advance(&mut voice.phases[0], 510.0 * voice.tune, rate);
            advance(&mut voice.phases[1], 1630.0 * voice.tune, rate);
            let click = sine(voice.phases[0]) * 0.55 * voice.envs[0]
                + sine(voice.phases[1]) * 0.3 * voice.envs[1]
                + self.side_stick_bp.process(noise).band * 1.3 * voice.noise_env;
            snare += click * mix.snare * 0.95;
            voice.envs[0] *= voice.decays[0];
            voice.envs[1] *= voice.decays[1];
            voice.noise_env *= voice.noise_decay;
        }
        left += snare * 0.97;
        right += snare;
        send += snare * 0.5;

        // Toms: a sine with a small pitch drop and a little skin noise
        let mut toms = 0.0;
        for (index, voice) in self.toms.iter_mut().enumerate() {
            if !voice.on {
                continue;
            }
            advance(
                &mut voice.phase,
                TOM_FREQS_HZ[index] * (1.0 + 0.35 * voice.pitch_env) * voice.tune,
                rate,
            );
            let tom = sine(voice.phase) * voice.env + self.noise.next() * voice.noise_env * 0.08;
            voice.env *= voice.decay;
            voice.pitch_env *= voice.pitch_decay;
            voice.noise_env *= voice.noise_decay;
            if voice.env < ENV_FLOOR {
                voice.on = false;
            }
            left += tom * tom_gain * (0.5 - 0.5 * TOM_PANS[index]);
            right += tom * tom_gain * (0.5 + 0.5 * TOM_PANS[index]);
            toms += tom;
        }
        send += toms * tom_gain * 0.4;

        // Hi-hat, slightly left
        let mut hat = 0.0;
        if self.hat.env > ENV_FLOOR {
            let source = self.hat_metal.process(rate) * 0.5 + noise * 0.6;
            let high = self.hat_hp.process(source).high;
            let band = self.hat_bp.process(high).band;
            hat = (high * 0.35 * self.hat.level + band * 0.9) * self.hat.env * hat_gain;
            self.hat.env *= self.hat.decay;
        }
        left += hat * 0.75;
        right += hat * 0.45;
        send += hat * 0.12;

        // Ride, on the right
        let mut ride = 0.0;
        if self.ride.env > ENV_FLOOR || self.ride.ping > ENV_FLOOR || self.ride.bell > ENV_FLOOR {
            let voice = &mut self.ride;
            let source = self.ride_metal.process(rate) * 0.7 + noise * 0.3;
            let high = self.ride_hp.process(source).high;
            let band = self.ride_bp.process(high).band;
            advance(&mut voice.bell_phases[0], 740.0, rate);
            advance(&mut voice.bell_phases[1], 1119.0, rate);
            let bell = (sine(voice.bell_phases[0]) + 0.6 * sine(voice.bell_phases[1])) * voice.bell;
            ride = ((band * 0.6 + high * 0.15) * voice.env + band * voice.ping * 0.8 + bell * 0.25)
                * cymbal_gain;
            voice.env *= voice.decay;
            voice.ping *= voice.ping_decay;
            voice.bell *= voice.bell_decay;
        }
        left += ride * 0.4;
        right += ride * 0.8;
        send += ride * 0.15;

        // Crash: dark, left of centre
        let mut crash = 0.0;
        if self.crash.env > ENV_FLOOR {
            self.crash.fade_in(0.003, rate);
            let source = self.crash_metal.process(rate) * 0.45 + noise * 0.7;
            let low = self.crash_lp.process(self.crash_hp.process(source).high).low;
            crash = low * self.crash.env * self.crash.attack * cymbal_gain * 0.9;
            self.crash.env *= self.crash.decay;
        }
        left += crash * 0.8;
        right += crash * 0.5;
        send += crash * 0.2;

        // Shaker, on the right: it only starts to die away once it has swelled
        let mut shaker = 0.0;
        if self.shaker.env > ENV_FLOOR {
            self.shaker.fade_in(0.010, rate);
            let grains = self.shaker_bp.process(noise).band * 0.8
                + self.shaker_hp.process(noise).high * 0.25;
            shaker = grains * self.shaker.env * self.shaker.attack * shaker_gain;
            if self.shaker.attack >= 1.0 {
                self.shaker.env *= self.shaker.decay;
            }
        }
        left += shaker * 0.35;
        right += shaker * 0.75;
        send += shaker * 0.2;

        let room = mix.room * ROOM_SCALE;
        // Always run, so a tail keeps decaying while Room is at zero and does not resume
        let (room_left, room_right) = self.room.process(send * 0.18);
        left += room_left * room;
        right += room_right * room;

        // Tape: saturation, then the warmth filter takes the top off
        let left = self.drive.process(self.out_hps[0].process(left).high);
        let right = self.drive.process(self.out_hps[1].process(right).high);
        let left = self.tones[0].process(left).low * LEVEL;
        let right = self.tones[1].process(right).low * LEVEL;

        if self.rest.is_due(left, right) {
            self.reset();
        }
        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{band_rms, decay_time_s, peak, rms, zero_crossing_freq};

    const SAMPLE_RATE: f32 = 44100.0;

    fn hit(velocity: f32) -> Hit {
        Hit { velocity, tune: 1.0, decay: 1.0 }
    }

    fn dry() -> Mix {
        Mix { room: 0.0, ..Mix::default() }
    }

    fn kit() -> GrooveKit {
        let mut kit = GrooveKit::new(1);
        kit.set_sample_rate(SAMPLE_RATE);
        kit
    }

    fn render(kit: &mut GrooveKit, mix: &Mix, seconds: f32) -> Vec<f32> {
        (0..(seconds * SAMPLE_RATE) as usize)
            .map(|_| {
                let (left, right) = kit.process(mix);
                (left + right) * 0.5
            })
            .collect()
    }

    fn render_note(note: u8, velocity: f32, seconds: f32) -> Vec<f32> {
        let mut kit = kit();
        kit.note_on(note, &hit(velocity));
        render(&mut kit, &dry(), seconds)
    }

    #[test]
    fn test_rests_until_hit_and_again_after_the_sound() {
        let mut kit = kit();
        assert!(render(&mut kit, &Mix::default(), 0.1).iter().all(|&sample| sample == 0.0));

        kit.note_on(36, &hit(1.0));
        assert!(peak(&render(&mut kit, &Mix::default(), 0.1)) > 0.3);

        render(&mut kit, &Mix::default(), 4.0);
        assert!(!kit.is_active());
    }

    #[test]
    fn test_kick_settles_on_52_hz_and_rings_longer_than_a_tight_one() {
        let output = render_note(36, 1.0, 1.5);

        let settled = &output[(0.2 * SAMPLE_RATE) as usize..(0.5 * SAMPLE_RATE) as usize];
        let freq = zero_crossing_freq(settled, SAMPLE_RATE);
        assert!((50.0..56.0).contains(&freq), "Kick at {} Hz", freq);
        // A time constant of 170 ms is 40 dB down after 0.78 s
        let decay = decay_time_s(&output, SAMPLE_RATE, -40.0);
        assert!((0.6..1.0).contains(&decay), "Kick decay: {} s", decay);
    }

    #[test]
    fn test_ghost_notes_are_darker_than_backbeats() {
        let top_share = |velocity: f32| {
            let output = render_note(38, velocity, 0.2);
            band_rms(&output, SAMPLE_RATE, 3000.0, SAMPLE_RATE) / rms(&output)
        };
        assert!(
            top_share(1.0) > top_share(0.25) * 1.3,
            "{} vs {}",
            top_share(1.0),
            top_share(0.25)
        );
    }

    #[test]
    fn test_shaker_and_every_crash_note_sound() {
        for note in [49, 52, 55, 57, 69, 70, 82] {
            assert!(peak(&render_note(note, 1.0, 0.3)) > 0.02, "Note {}", note);
        }
        // The splash is the crash cut short
        let decay = |note| decay_time_s(&render_note(note, 1.0, 8.0), SAMPLE_RATE, -40.0);
        assert!(decay(55) < decay(49) * 0.5);
    }

    #[test]
    fn test_the_top_is_taken_off() {
        // The crash is noise up to 8.5 kHz; the warmth filter leaves little over 12 kHz
        let output = render_note(49, 1.0, 0.5);
        let top = band_rms(&output, SAMPLE_RATE, 12000.0, SAMPLE_RATE) / rms(&output);
        assert!(top < 0.1, "Share over 12 kHz: {}", top);
    }
}
