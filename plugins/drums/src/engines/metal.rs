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
const HAT_TRIM_DB: f32 = -4.0;
const CYMBAL_TRIM_DB: f32 = -4.0;

// The room at the default Room setting (25 %), as a share of its full level
const ROOM_AT_DEFAULT: f32 = 0.16;
const ROOM_SCALE: f32 = ROOM_AT_DEFAULT / 0.25;

const BUS_DRIVE: f32 = 0.35;

// Lowest (floor) tom to highest: pitch, and position from -1.0 (left) to 1.0 (right)
const TOM_FREQS_HZ: [f32; TOM_COUNT] = [78.0, 92.0, 118.0, 138.0, 160.0, 185.0];
const TOM_PANS: [f32; TOM_COUNT] = [0.35, 0.25, 0.05, -0.1, -0.2, -0.3];

// Velocity to level
const VELOCITY_CURVE: f32 = 1.4;

#[derive(Default)]
struct Kick {
    on: bool,
    env: f32,
    pitch_env: f32,
    click_env: f32,
    phase: f32,
    click_phase: f32,
    tune: f32,
    decay: f32,
    pitch_decay: f32,
    click_decay: f32,
}

#[derive(Default)]
struct Snare {
    on: bool,
    body_env: f32,
    pitch_env: f32,
    crack_env: f32,
    wire_env: f32,
    phases: [f32; 2],
    tune: f32,
    body_decay: f32,
    pitch_decay: f32,
    crack_decay: f32,
    wire_decay: f32,
}

#[derive(Default)]
struct SideStick {
    env: f32,
    phases: [f32; 2],
    tune: f32,
    decay: f32,
}

#[derive(Default)]
struct Tom {
    on: bool,
    env: f32,
    pitch_env: f32,
    noise_env: f32,
    phases: [f32; 2],
    tune: f32,
    decay: f32,
    pitch_decay: f32,
    noise_decay: f32,
}

#[derive(Default)]
struct Ride {
    env: f32,
    ping: f32,
    bell: f32,
    bell_phases: [f32; 3],
    decay: f32,
    ping_decay: f32,
    bell_decay: f32,
}

/// Level of a cymbal: a short fade in, then a decay
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

    fn process(&mut self, attack_s: f32, sample_rate: f32) -> f32 {
        if self.attack < 1.0 {
            self.attack = (self.attack + 1.0 / (attack_s * sample_rate)).min(1.0);
        }
        let level = self.env * self.attack;
        self.env *= self.decay;
        level
    }
}

/// The Metal kit: tight and loud, with a clicking kick, a cracking snare and cymbals of
/// square waves and noise. One voice per piece: a new hit starts the piece again.
pub struct MetalKit {
    sample_rate: f32,
    active: bool,
    rest: Rest,
    noise: Noise,

    kick: Kick,
    kick_click: Svf,
    kick_hp: Svf,

    snare: Snare,
    snare_hp: Svf,
    snare_bp: Svf,
    snare_lp: Svf,
    side_stick: SideStick,
    side_stick_bp: Svf,

    toms: [Tom; TOM_COUNT],

    hat_env: f32,
    hat_decay: f32,
    hat_metal: MetalCluster,
    hat_hp: Svf,
    hat_bp: Svf,

    ride: Ride,
    ride_metal: MetalCluster,
    ride_hp: Svf,
    ride_bp: Svf,

    crashes: [Swell; 2],
    crash_metals: [MetalCluster; 2],
    crash_hps: [Svf; 2],
    crash_lps: [Svf; 2],
    splash: Swell,
    splash_hp: Svf,

    china: Swell,
    china_metal: MetalCluster,
    china_bp: Svf,
    china_hp: Svf,

    room: CombRoom,
    drive: BusDrive,
    out_hps: [Svf; 2],
}

impl MetalKit {
    pub fn new(seed: u32) -> Self {
        let mut kit = Self {
            sample_rate: 44100.0,
            active: false,
            rest: Rest::new(),
            noise: Noise::new(seed),
            kick: Kick::default(),
            kick_click: Svf::new(),
            kick_hp: Svf::new(),
            snare: Snare::default(),
            snare_hp: Svf::new(),
            snare_bp: Svf::new(),
            snare_lp: Svf::new(),
            side_stick: SideStick::default(),
            side_stick_bp: Svf::new(),
            toms: std::array::from_fn(|_| Tom::default()),
            hat_env: 0.0,
            hat_decay: 0.0,
            hat_metal: MetalCluster::new(410.0),
            hat_hp: Svf::new(),
            hat_bp: Svf::new(),
            ride: Ride::default(),
            ride_metal: MetalCluster::new(530.0),
            ride_hp: Svf::new(),
            ride_bp: Svf::new(),
            crashes: [Swell::default(), Swell::default()],
            crash_metals: [MetalCluster::new(360.0), MetalCluster::new(395.0)],
            crash_hps: [Svf::new(), Svf::new()],
            crash_lps: [Svf::new(), Svf::new()],
            splash: Swell::default(),
            splash_hp: Svf::new(),
            china: Swell::default(),
            china_metal: MetalCluster::new(290.0),
            china_bp: Svf::new(),
            china_hp: Svf::new(),
            room: CombRoom::new(0.6, 0.74, 0.35),
            drive: BusDrive::new(0.6 + 2.4 * BUS_DRIVE),
            out_hps: [Svf::new(), Svf::new()],
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
        self.kick_hp.set(2500.0, 0.7, rate);
        self.kick_click.set(4200.0, 1.2, rate);
        self.snare_hp.set(1100.0, 0.7, rate);
        self.snare_bp.set(3600.0, 0.7, rate);
        self.snare_lp.set(7500.0, 0.7, rate);
        self.side_stick_bp.set(2200.0, 1.5, rate);
        self.hat_hp.set(7200.0, 0.7, rate);
        self.hat_bp.set(10500.0, 1.2, rate);
        self.ride_hp.set(3200.0, 0.7, rate);
        self.ride_bp.set(5600.0, 1.1, rate);
        self.crash_hps[0].set(2600.0, 0.7, rate);
        self.crash_lps[0].set(12500.0, 0.7, rate);
        self.crash_hps[1].set(3100.0, 0.7, rate);
        self.crash_lps[1].set(13500.0, 0.7, rate);
        self.splash_hp.set(4800.0, 0.7, rate);
        self.china_bp.set(2700.0, 0.9, rate);
        self.china_hp.set(700.0, 0.7, rate);
        for filter in &mut self.out_hps {
            filter.set(28.0, 0.7, rate);
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
        self.hat_env = 0.0;
        self.ride = Ride::default();
        self.crashes = [Swell::default(), Swell::default()];
        self.splash = Swell::default();
        self.china = Swell::default();

        for cluster in [&mut self.hat_metal, &mut self.ride_metal, &mut self.china_metal]
            .into_iter()
            .chain(&mut self.crash_metals)
        {
            cluster.reset();
        }
        for filter in [
            &mut self.kick_click,
            &mut self.kick_hp,
            &mut self.snare_hp,
            &mut self.snare_bp,
            &mut self.snare_lp,
            &mut self.side_stick_bp,
            &mut self.hat_hp,
            &mut self.hat_bp,
            &mut self.ride_hp,
            &mut self.ride_bp,
            &mut self.splash_hp,
            &mut self.china_bp,
            &mut self.china_hp,
        ]
        .into_iter()
        .chain(&mut self.crash_hps)
        .chain(&mut self.crash_lps)
        .chain(&mut self.out_hps)
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
                    click_env: level,
                    phase: 0.0,
                    click_phase: 0.0,
                    tune: hit.tune,
                    decay: decay(0.075),
                    pitch_decay: coef(0.022, rate),
                    click_decay: coef(0.003, rate),
                };
            }
            37 => {
                self.side_stick = SideStick {
                    env: level,
                    phases: [0.0; 2],
                    tune: hit.tune,
                    decay: decay(0.012),
                };
            }
            38 | 40 => {
                self.snare = Snare {
                    on: true,
                    body_env: level,
                    pitch_env: 1.0,
                    crack_env: level,
                    wire_env: level,
                    phases: [0.0; 2],
                    tune: hit.tune,
                    body_decay: decay(0.045),
                    pitch_decay: coef(0.012, rate),
                    crack_decay: coef(0.003, rate),
                    // Harder hits shake the wires for longer
                    wire_decay: decay(0.05 + 0.05 * level),
                };
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                self.toms[tom_index(note)] = Tom {
                    on: true,
                    env: level,
                    pitch_env: 1.0,
                    noise_env: level,
                    phases: [0.0; 2],
                    tune: hit.tune,
                    decay: decay(0.2),
                    pitch_decay: coef(0.035, rate),
                    noise_decay: coef(0.012, rate),
                };
            }
            // The closed and pedal sounds have a short decay, which chokes an open hi-hat
            42 => (self.hat_env, self.hat_decay) = (level, decay(0.028)),
            44 => (self.hat_env, self.hat_decay) = (level * 0.55, decay(0.018)),
            46 => (self.hat_env, self.hat_decay) = (level, decay(0.26)),
            51 | 59 => {
                self.ride.env = level;
                self.ride.ping = level;
                self.set_ride_decays(hit);
            }
            53 => {
                self.ride.env = level * 0.6;
                self.ride.ping = level * 0.5;
                self.ride.bell = level;
                self.set_ride_decays(hit);
            }
            49 => self.crashes[0].trigger(level, 0.55 * hit.decay, rate),
            57 => self.crashes[1].trigger(level, 0.6 * hit.decay, rate),
            55 => self.splash.trigger(level, 0.16 * hit.decay, rate),
            52 => self.china.trigger(level, 0.33 * hit.decay, rate),
            _ => return,
        }
        self.active = true;
        self.rest.wake();
    }

    fn set_ride_decays(&mut self, hit: &Hit) {
        self.ride.decay = coef(0.6 * hit.decay, self.sample_rate);
        self.ride.ping_decay = coef(0.035, self.sample_rate);
        self.ride.bell_decay = coef(0.4 * hit.decay, self.sample_rate);
    }

    /// One stereo frame
    pub fn process(&mut self, mix: &Mix) -> (f32, f32) {
        if !self.active {
            return (0.0, 0.0);
        }

        let rate = self.sample_rate;
        let tom_gain = mix.toms * db(TOM_TRIM_DB);
        let hat_gain = mix.hihat * db(HAT_TRIM_DB);
        let cymbal_gain = mix.cymbals * db(CYMBAL_TRIM_DB);

        // Every noise part of the kit hears the same noise, each through its own filters
        let noise = self.noise.next();
        let mut left = 0.0;
        let mut right = 0.0;
        let mut send = 0.0;

        // Kick: a sine that falls from 181 to 46 Hz, a click of a high sine and noise
        let mut kick = 0.0;
        if self.kick.on {
            let voice = &mut self.kick;
            advance(&mut voice.phase, (46.0 + 135.0 * voice.pitch_env) * voice.tune, rate);
            advance(&mut voice.click_phase, 3100.0, rate);
            let body = sine(voice.phase) * voice.env;
            let click = (sine(voice.click_phase) * 0.7
                + self.kick_click.process(noise).band * 1.6
                + self.kick_hp.process(noise).high * 0.3)
                * voice.click_env;
            kick = (body * 1.7 + click * 1.1).tanh() * mix.kick;
            voice.env *= voice.decay;
            voice.pitch_env *= voice.pitch_decay;
            voice.click_env *= voice.click_decay;
            if voice.env < ENV_FLOOR {
                voice.on = false;
            }
        }
        left += kick;
        right += kick;
        send += kick * 0.12;

        // Snare: two sines for the head, filtered noise for the wires and the crack
        let mut snare = 0.0;
        if self.snare.on {
            let voice = &mut self.snare;
            advance(&mut voice.phases[0], (182.0 + 45.0 * voice.pitch_env) * voice.tune, rate);
            advance(&mut voice.phases[1], (331.0 + 30.0 * voice.pitch_env) * voice.tune, rate);
            let body = (sine(voice.phases[0]) * 0.85 + sine(voice.phases[1]) * 0.45) * voice.body_env;
            let hiss = self.snare_hp.process(noise).high;
            let band = self.snare_bp.process(hiss).band;
            let wires = (self.snare_lp.process(hiss).low * 0.5 + band * 1.1) * voice.wire_env;
            let crack = band * voice.crack_env * 2.5;
            snare = ((body * 1.7 + wires * 1.1 + crack * 0.8) * 1.25).tanh() * mix.snare;
            voice.body_env *= voice.body_decay;
            voice.pitch_env *= voice.pitch_decay;
            voice.crack_env *= voice.crack_decay;
            voice.wire_env *= voice.wire_decay;
            if voice.wire_env < ENV_FLOOR && voice.body_env < ENV_FLOOR {
                voice.on = false;
            }
        }
        if self.side_stick.env > ENV_FLOOR {
            let voice = &mut self.side_stick;
            advance(&mut voice.phases[0], 1480.0 * voice.tune, rate);
            advance(&mut voice.phases[1], 2730.0 * voice.tune, rate);
            let click = sine(voice.phases[0]) * 0.6
                + sine(voice.phases[1]) * 0.35
                + self.side_stick_bp.process(noise).band * 0.8;
            snare += click * voice.env * mix.snare * 0.9;
            voice.env *= voice.decay;
        }
        left += snare;
        right += snare;
        send += snare * 0.45;

        // Toms: a sine with a fifth above it that fades with the pitch drop, and stick noise
        let mut toms = 0.0;
        for (index, voice) in self.toms.iter_mut().enumerate() {
            if !voice.on {
                continue;
            }
            let freq = TOM_FREQS_HZ[index] * (1.0 + 0.55 * voice.pitch_env) * voice.tune;
            advance(&mut voice.phases[0], freq, rate);
            advance(&mut voice.phases[1], freq * 1.51, rate);
            let tom = (sine(voice.phases[0]) + 0.3 * sine(voice.phases[1]) * (0.3 + 0.7 * voice.pitch_env))
                * voice.env
                + self.noise.next() * voice.noise_env * 0.2;
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
        send += toms * tom_gain * 0.35;

        // Hi-hat, on the left
        let mut hat = 0.0;
        if self.hat_env > ENV_FLOOR {
            let source = self.hat_metal.process(rate) * 0.75 + noise * 0.45;
            let high = self.hat_hp.process(source).high;
            hat = (high * 0.55 + self.hat_bp.process(high).band * 0.9) * self.hat_env * hat_gain;
            self.hat_env *= self.hat_decay;
        }
        left += hat * 0.8;
        right += hat * 0.35;
        send += hat * 0.1;

        // Ride, on the right: wash, the ping of the stick, and three sines for the bell
        let mut ride = 0.0;
        if self.ride.env > ENV_FLOOR || self.ride.bell > ENV_FLOOR {
            let voice = &mut self.ride;
            let source = self.ride_metal.process(rate) * 0.8 + noise * 0.25;
            let high = self.ride_hp.process(source).high;
            let band = self.ride_bp.process(high).band;
            advance(&mut voice.bell_phases[0], 772.0, rate);
            advance(&mut voice.bell_phases[1], 1163.0, rate);
            advance(&mut voice.bell_phases[2], 1741.0, rate);
            let bell = (sine(voice.bell_phases[0])
                + 0.7 * sine(voice.bell_phases[1])
                + 0.45 * sine(voice.bell_phases[2]))
                * voice.bell;
            ride = ((band * 0.7 + high * 0.25) * voice.env + high * voice.ping * 0.9 + bell * 0.3) * cymbal_gain;
            voice.env *= voice.decay;
            voice.ping *= voice.ping_decay;
            voice.bell *= voice.bell_decay;
        }
        left += ride * 0.35;
        right += ride * 0.85;
        send += ride * 0.12;

        // Crash 1 on the left, crash 2 on the right, splash between them
        let mut crashes = [0.0; 2];
        for (index, crash) in crashes.iter_mut().enumerate() {
            let source = self.crash_metals[index].process(rate) * 0.55 + noise * 0.6;
            let high = self.crash_hps[index].process(source).high;
            let low = self.crash_lps[index].process(high).low;
            *crash = low * self.crashes[index].process(0.0012, rate) * cymbal_gain;
        }
        // The splash borrows the hi-hat's metal, which runs twice as fast while both sound
        let source = noise * 0.7 + self.hat_metal.process(rate) * 0.3;
        let splash = self.splash_hp.process(source).high * self.splash.process(0.0012, rate) * cymbal_gain * 0.8;
        left += crashes[0] * 0.85 + crashes[1] * 0.3 + splash * 0.7;
        right += crashes[0] * 0.3 + crashes[1] * 0.85 + splash * 0.4;
        send += (crashes[0] + crashes[1]) * 0.15;

        // China, on the right: noise that the metal modulates, driven hard
        let mut china = 0.0;
        if self.china.env > ENV_FLOOR {
            let source = noise * (0.6 + self.china_metal.process(rate) * 0.8);
            let band = self.china_bp.process(source).band;
            let trash = (self.china_hp.process(band * 2.2 + source * 0.35).high * 1.8).tanh();
            china = trash * self.china.process(0.001, rate) * cymbal_gain * 0.9;
        }
        left += china * 0.3;
        right += china * 0.9;
        send += china * 0.12;

        let room = mix.room * ROOM_SCALE;
        if room > 0.0 {
            let (room_left, room_right) = self.room.process(send * 0.2);
            left += room_left * room;
            right += room_right * room;
        }

        let left = self.drive.process(self.out_hps[0].process(left).high) * LEVEL;
        let right = self.drive.process(self.out_hps[1].process(right).high) * LEVEL;

        if self.rest.is_due(left, right) {
            self.reset();
        }
        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{decay_time_s, peak, rms, zero_crossing_freq};

    const SAMPLE_RATE: f32 = 44100.0;

    fn hit(velocity: f32) -> Hit {
        Hit {
            velocity,
            tune: 1.0,
            decay: 1.0,
        }
    }

    fn dry() -> Mix {
        Mix {
            room: 0.0,
            ..Mix::default()
        }
    }

    fn kit() -> MetalKit {
        let mut kit = MetalKit::new(1);
        kit.set_sample_rate(SAMPLE_RATE);
        kit
    }

    fn render(kit: &mut MetalKit, mix: &Mix, seconds: f32) -> Vec<f32> {
        (0..(seconds * SAMPLE_RATE) as usize)
            .map(|_| {
                let (left, right) = kit.process(mix);
                (left + right) * 0.5
            })
            .collect()
    }

    #[test]
    fn test_rests_until_hit_and_again_after_the_sound() {
        let mut kit = kit();
        assert!(!kit.is_active());
        assert!(render(&mut kit, &Mix::default(), 0.1).iter().all(|&sample| sample == 0.0));

        kit.note_on(36, &hit(1.0));
        assert!(kit.is_active());
        assert!(peak(&render(&mut kit, &Mix::default(), 0.1)) > 0.3);

        render(&mut kit, &Mix::default(), 3.0);
        assert!(!kit.is_active());
        assert!(render(&mut kit, &Mix::default(), 0.1).iter().all(|&sample| sample == 0.0));
    }

    #[test]
    fn test_kick_settles_on_46_hz_and_is_short() {
        let mut kit = kit();
        kit.note_on(36, &hit(1.0));
        let output = render(&mut kit, &dry(), 1.0);

        let settled = &output[(0.12 * SAMPLE_RATE) as usize..(0.3 * SAMPLE_RATE) as usize];
        let freq = zero_crossing_freq(settled, SAMPLE_RATE);
        assert!((44.0..50.0).contains(&freq), "Kick at {} Hz", freq);
        // A time constant of 75 ms is 40 dB down after 0.35 s
        let decay = decay_time_s(&output, SAMPLE_RATE, -40.0);
        assert!((0.25..0.45).contains(&decay), "Kick decay: {} s", decay);
    }

    #[test]
    fn test_toms_ring_at_their_pitches() {
        for (note, expected) in [(41, 78.0), (45, 118.0), (50, 185.0)] {
            let mut kit = kit();
            kit.note_on(note, &hit(1.0));
            let output = render(&mut kit, &dry(), 0.6);

            let settled = &output[(0.2 * SAMPLE_RATE) as usize..(0.5 * SAMPLE_RATE) as usize];
            let freq = zero_crossing_freq(settled, SAMPLE_RATE);
            assert!((freq / expected - 1.0f32).abs() < 0.03, "Note {} at {} Hz", note, freq);
        }
    }

    #[test]
    fn test_tune_and_damping_apply_to_the_next_hit() {
        let tom = |tune: f32, decay: f32| {
            let mut kit = kit();
            kit.note_on(41, &Hit { velocity: 1.0, tune, decay });
            render(&mut kit, &dry(), 2.0)
        };
        let settled = |output: &[f32]| {
            zero_crossing_freq(&output[(0.2 * SAMPLE_RATE) as usize..(0.4 * SAMPLE_RATE) as usize], SAMPLE_RATE)
        };

        let (plain, high, damped) = (tom(1.0, 1.0), tom(1.5, 1.0), tom(1.0, 0.25));
        assert!((settled(&high) / settled(&plain) - 1.5).abs() < 0.05);
        let ratio = decay_time_s(&damped, SAMPLE_RATE, -40.0) / decay_time_s(&plain, SAMPLE_RATE, -40.0);
        assert!((0.2..0.35).contains(&ratio), "Decay ratio: {}", ratio);
    }

    #[test]
    fn test_room_at_the_default_setting_is_the_kits_own() {
        let snare = |room: f32| {
            let mut kit = kit();
            kit.note_on(38, &hit(0.5));
            render(&mut kit, &Mix { room, ..Mix::default() }, 0.5)
        };
        // Every render has the same noise, so what the room adds is the difference
        let room_alone = |room: f32| {
            let tail = (0.3 * SAMPLE_RATE) as usize;
            let added: Vec<f32> = snare(room)[tail..].iter().zip(&snare(0.0)[tail..]).map(|(wet, dry)| wet - dry).collect();
            rms(&added)
        };

        assert!(room_alone(0.25) > 1e-5);
        let ratio = room_alone(0.5) / room_alone(0.25);
        assert!((1.8..2.2).contains(&ratio), "Room at 50 % against 25 %: {}", ratio);
    }
}
