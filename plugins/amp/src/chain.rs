use crate::amp::model::Amp;
use crate::amp::poweramp::PowerAmp;
use crate::amp::preamp::Preamp;
use crate::amp::tonestack::{ToneCurve, ToneStack};
use crate::cab::{design_ir, CabIr, CabVoicing, Cabinet};
use crate::cab_stage::CabStage;
use crate::delay::{Delay, DelaySettings};
use crate::drive::Drive;
use crate::dsp::filters::DcBlocker;
use crate::dsp::oversample::{factor_for, Oversampler, MAX_FACTOR};
use crate::dsp::shaper::output_clip;
use crate::dsp::{curve, db_to_gain, smoothing_coeff, Ramp};
use crate::gate::Gate;
use crate::reverb::{Reverb, ReverbSettings};
use crate::tuner::{Tuner, TunerReading};
use std::sync::Arc;

// The chain works in pieces of at most this many samples, and reads the dials once per piece
const CHUNK: usize = 32;
// Room for a piece at the highest oversampled rate
const OVERSAMPLED_CHUNK: usize = CHUNK * MAX_FACTOR;

// How fast the chain follows a dial
const DIAL_SMOOTH_MS: f32 = 20.0;

// A dial this close to where it is going is taken as there, so the filters stop being redesigned
const DIAL_SETTLED: f32 = 1e-5;

// Crossfade between the amp and the untouched input when Bypass is switched. The repeats and
// the reverb tail fade out with the amp, and are dropped once nothing is heard of them: what
// comes back is the amp from rest, not what was left in the effects when it was switched off
const BYPASS_FADE_MS: f32 = 10.0;

// When another amp is picked, the amp's output fades to silence in this time, the stages are
// set up as the new amp, and it fades back in as fast
const AMP_FADE_MS: f32 = 5.0;

// Crossfade between the cabinet and the amp's own signal when the cabinet is switched
const CAB_FADE_MS: f32 = 20.0;

// While the tuner is on the amp is given silence and its output is turned down, both in this
// time. The same when it is switched off again
const TUNER_FADE_MS: f32 = 10.0;

// Level of the amp's signal with the cabinet off, in dB: about as loud as with it on
const CAB_OFF_DB: f32 = -1.4;

// The limits of the settings that are not dials. The parameters in `lib.rs` take their
// ranges from these, and their defaults from `AmpSettings::default`
pub const IN_GAIN_DB: [f32; 2] = [-24.0, 24.0];
pub const GATE_THRESHOLD_DB: [f32; 2] = [-80.0, -20.0];
pub const GATE_RELEASE_MS: [f32; 2] = [20.0, 500.0];

/// What the knobs say, read once per block. The chain smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmpSettings {
    pub bypass: bool,
    /// Gain in dB in front of everything but the gate's detector
    pub in_gain_db: f32,

    pub gate_on: bool,
    /// Level in dBFS at the plugin's input that opens the gate
    pub gate_thresh_db: f32,
    /// Time the gate takes to close, in milliseconds
    pub gate_release_ms: f32,

    pub drive_on: bool,
    /// The drive pedal's dials, 0.0 to 1.0
    pub drive_gain: f32,
    pub drive_tone: f32,
    pub drive_level: f32,

    pub amp: Amp,
    /// The amp dials, 0.0 to 1.0
    pub gain: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub presence: f32,
    pub master: f32,

    /// Off leaves the amp's signal as it is, for a cabinet somewhere else
    pub cab_on: bool,
    /// The cabinet's dials, 0.0 to 1.0. Both leave the cabinet as designed at 0.5
    pub cab_mic: f32,
    pub cab_res: f32,

    /// The two effects behind the cabinet. They smooth their own settings
    pub delay: DelaySettings,
    pub reverb: ReverbSettings,

    /// Linear gain
    pub out_level: f32,

    /// The tuner listens to the input and the amp is silent meanwhile. Bypass still passes
    /// the input on
    pub tuner_on: bool,
}

/// What the plugin starts with. The parameters in `lib.rs` take their defaults from here
impl Default for AmpSettings {
    fn default() -> Self {
        Self {
            bypass: false,
            in_gain_db: 0.0,
            gate_on: true,
            gate_thresh_db: -60.0,
            gate_release_ms: 100.0,
            drive_on: false,
            drive_gain: 0.3,
            drive_tone: 0.5,
            drive_level: 0.5,
            amp: Amp::Brol,
            gain: 0.5,
            bass: 0.5,
            mid: 0.5,
            treble: 0.5,
            presence: 0.5,
            master: 0.5,
            cab_on: true,
            cab_mic: 0.5,
            cab_res: 0.5,
            delay: DelaySettings::default(),
            reverb: ReverbSettings::default(),
            out_level: 1.0,
            tuner_on: false,
        }
    }
}

/// The dial values the chain is at, on their way to the settings
#[derive(Clone, Copy, PartialEq)]
struct Dials {
    in_gain_db: f32,
    drive_gain: f32,
    drive_tone: f32,
    drive_level: f32,
    gain: f32,
    bass: f32,
    mid: f32,
    treble: f32,
    presence: f32,
    master: f32,
    cab_mic: f32,
    cab_res: f32,
    out_level: f32,
}

impl Dials {
    fn from_settings(settings: &AmpSettings) -> Self {
        Self {
            in_gain_db: settings.in_gain_db.clamp(IN_GAIN_DB[0], IN_GAIN_DB[1]),
            drive_gain: settings.drive_gain.clamp(0.0, 1.0),
            drive_tone: settings.drive_tone.clamp(0.0, 1.0),
            drive_level: settings.drive_level.clamp(0.0, 1.0),
            gain: settings.gain.clamp(0.0, 1.0),
            bass: settings.bass.clamp(0.0, 1.0),
            mid: settings.mid.clamp(0.0, 1.0),
            treble: settings.treble.clamp(0.0, 1.0),
            presence: settings.presence.clamp(0.0, 1.0),
            master: settings.master.clamp(0.0, 1.0),
            cab_mic: settings.cab_mic.clamp(0.0, 1.0),
            cab_res: settings.cab_res.clamp(0.0, 1.0),
            out_level: settings.out_level.max(0.0),
        }
    }

    fn approach(&mut self, target: &Dials, coeff: f32) {
        let step = |value: &mut f32, target: f32| {
            *value += coeff * (target - *value);
            if (target - *value).abs() < DIAL_SETTLED {
                *value = target;
            }
        };
        step(&mut self.in_gain_db, target.in_gain_db);
        step(&mut self.drive_gain, target.drive_gain);
        step(&mut self.drive_tone, target.drive_tone);
        step(&mut self.drive_level, target.drive_level);
        step(&mut self.gain, target.gain);
        step(&mut self.bass, target.bass);
        step(&mut self.mid, target.mid);
        step(&mut self.treble, target.treble);
        step(&mut self.presence, target.presence);
        step(&mut self.master, target.master);
        step(&mut self.cab_mic, target.cab_mic);
        step(&mut self.cab_res, target.cab_res);
        step(&mut self.out_level, target.out_level);
    }
}

/// The whole signal chain. Mono through the amp and cabinet, stereo from the delay on
pub struct AmpChain {
    sample_rate: f32,
    // How many times the drive and the amp run faster than the host: 4 at the usual sample
    // rates, 2 from 88.2 kHz on
    factor: usize,
    // The amp the stages are set up as, and the one the settings ask for
    amp: Amp,
    wanted_amp: Amp,

    in_gain: Ramp,
    gate: Gate,
    oversampler: Oversampler,
    drive: Drive,
    preamp: Preamp,
    tone: ToneStack,
    power: PowerAmp,
    cabinet: Cabinet,
    // One cabinet per amp, designed when the sample rate is set
    cab_irs: Vec<CabIr>,
    // Where a cabinet the player chose is left for the chain to take, and whether one of
    // the player's own is playing: it then stays whatever amp is picked
    cab_stage: Arc<CabStage>,
    user_cab: bool,
    cab_voicing: CabVoicing,
    // Share of the cabinet in what follows it: 1.0 on, 0.0 the amp's own signal
    cab_mix: Ramp,
    cab_fade_steps: u32,
    dc: DcBlocker,
    delay: Delay,
    reverb: Reverb,
    // What the effects are told. Read with the dials, so a switch takes effect on the same
    // sample whatever the block size
    delay_settings: DelaySettings,
    reverb_settings: ReverbSettings,
    out_level: Ramp,

    dials: Dials,
    dial_coeff: f32,
    // The dial positions the tone filters were last designed for
    tone_applied: [f32; 3],
    presence_applied: f32,
    // Mic and Resonance, likewise
    cab_applied: [f32; 2],
    // False until the first block after a reset: the dials then start at the settings
    primed: bool,
    // Samples left until the dials are read again. Counted across blocks, so the output
    // does not depend on how the host cuts the stream into blocks
    until_tick: usize,

    // Share of the amp in the output: 1.0 playing, 0.0 bypassed
    wet: f32,
    wet_step: f32,

    // Level of the amp's output in front of the cabinet, in steps: `amp_fade_len` playing,
    // down to 0 on the way to another amp and back up after
    amp_fade: u32,
    amp_fade_len: u32,
    // Samples until the cabinet has finished its crossfade and can start another
    cabinet_busy: usize,

    // Hears the input as it arrives, while `tuning`
    tuner: Tuner,
    tuning: bool,
    // Share of the input the amp is given and of its output that is heard: 1.0 playing,
    // 0.0 tuning
    tuner_mute: Ramp,
    tuner_fade_steps: u32,
}

impl AmpChain {
    pub fn new() -> Self {
        let mut chain = Self {
            sample_rate: 44100.0,
            factor: MAX_FACTOR,
            amp: Amp::Brol,
            wanted_amp: Amp::Brol,
            in_gain: Ramp::new(1.0),
            gate: Gate::new(),
            oversampler: Oversampler::new(),
            drive: Drive::new(),
            preamp: Preamp::new(),
            tone: ToneStack::new(),
            power: PowerAmp::new(),
            cabinet: Cabinet::new(),
            cab_irs: Vec::new(),
            cab_stage: Arc::new(CabStage::new()),
            user_cab: false,
            cab_voicing: CabVoicing::new(),
            cab_mix: Ramp::new(1.0),
            cab_fade_steps: 1,
            dc: DcBlocker::new(),
            delay: Delay::new(),
            reverb: Reverb::new(),
            delay_settings: DelaySettings::default(),
            reverb_settings: ReverbSettings::default(),
            out_level: Ramp::new(1.0),
            dials: Dials::from_settings(&AmpSettings::default()),
            dial_coeff: 1.0,
            tone_applied: [f32::NAN; 3],
            presence_applied: f32::NAN,
            cab_applied: [f32::NAN; 2],
            primed: false,
            until_tick: 0,
            wet: 1.0,
            wet_step: 1.0,
            amp_fade: 1,
            amp_fade_len: 1,
            cabinet_busy: 0,
            tuner: Tuner::new(),
            tuning: false,
            tuner_mute: Ramp::new(1.0),
            tuner_fade_steps: 1,
        };
        chain.set_sample_rate(44100.0);
        chain
    }

    /// Allocates and designs everything that depends on the sample rate. Not for the audio thread
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.dc.set_sample_rate(sample_rate);
        self.dial_coeff = smoothing_coeff(DIAL_SMOOTH_MS, sample_rate / CHUNK as f32);
        self.wet_step = 1.0 / (BYPASS_FADE_MS * 0.001 * sample_rate);
        self.amp_fade_len = ((AMP_FADE_MS * 0.001 * sample_rate).round() as u32).max(1);
        self.cab_fade_steps = ((CAB_FADE_MS * 0.001 * sample_rate).round() as u32).max(1);
        self.tuner_fade_steps = ((TUNER_FADE_MS * 0.001 * sample_rate).round() as u32).max(1);
        self.tuner.set_sample_rate(sample_rate);

        self.factor = factor_for(sample_rate);
        self.oversampler.set_factor(self.factor);
        self.gate.set_sample_rate(sample_rate);
        self.drive.configure(self.oversampled_rate());
        self.delay.set_sample_rate(sample_rate);
        self.reverb.set_sample_rate(sample_rate);

        self.cab_irs = Amp::ALL.iter().map(|amp| design_ir(&amp.model().cab, sample_rate)).collect();
        self.cabinet.set_sample_rate(sample_rate);
        // A cabinet of the player's own was made for the rate before: it has to be left
        // on the stage again for this one. Until then the amp plays through its own
        self.cabinet.set_ir(&self.cab_irs[self.amp.index()]);
        self.user_cab = false;

        self.configure_amp();
        self.reset();
    }

    /// The rate the drive and the amp run at
    fn oversampled_rate(&self) -> f32 {
        self.sample_rate * self.factor as f32
    }

    /// Samples at that rate in a piece of `CHUNK`: the time the stages take to follow a dial
    fn oversampled_chunk(&self) -> u32 {
        (CHUNK * self.factor) as u32
    }

    /// Moves towards the amp the settings ask for. Does not allocate. While the amp is heard
    /// its output first fades to silence; there the stages are set up as the new amp and
    /// start from rest, and the cabinet crossfades to the new amp's own, unless the player's
    /// own cabinet is playing: that one stays
    fn follow_amp(&mut self) {
        if self.wanted_amp == self.amp {
            return;
        }
        // Before the first block and while bypassed nothing is heard and every stage is at rest
        let unheard = !self.primed || self.wet == 0.0;
        let silent = self.amp_fade == 0 && self.cabinet_busy == 0;
        if !unheard && !silent {
            return;
        }

        self.amp = self.wanted_amp;
        if unheard {
            if !self.user_cab {
                self.cabinet.set_ir(&self.cab_irs[self.amp.index()]);
            }
            self.amp_fade = self.amp_fade_len;
        } else if !self.user_cab {
            debug_assert!(!self.cabinet.is_swapping());
            self.cabinet.swap_ir(&self.cab_irs[self.amp.index()]);
            self.cabinet_busy = self.cabinet.swap_len();
        }
        self.configure_amp();
        self.reset_amp_stages();
        self.apply_dials(true);
    }

    /// Takes the cabinet that was left on the stage, if there is one and the cabinet is
    /// free to change: no crossfade is running and no other amp is on its way. Until then
    /// it waits there, and a newer one may replace it. Crossfades to it while the amp is
    /// heard. Does not allocate, free or wait
    fn follow_cabinet(&mut self) {
        if !self.cab_stage.is_ready() || self.cabinet_busy > 0 || self.wanted_amp != self.amp {
            return;
        }
        let unheard = !self.primed || self.wet == 0.0;
        let Self {
            cab_stage,
            cabinet,
            cab_irs,
            user_cab,
            cabinet_busy,
            amp,
            sample_rate,
            ..
        } = self;
        cab_stage.take(|taps, made_for| {
            // One made for another sample rate is of no use: its successor is on its way.
            // And the amp's own cabinet is not swapped for itself
            if made_for != *sample_rate || (taps.is_empty() && !*user_cab) {
                return;
            }
            *user_cab = !taps.is_empty();
            let own = &cab_irs[amp.index()];
            match (unheard, *user_cab) {
                (true, true) => cabinet.set_taps(taps),
                (true, false) => cabinet.set_ir(own),
                (false, true) => cabinet.swap_taps(taps),
                (false, false) => cabinet.swap_ir(own),
            }
            if !unheard {
                *cabinet_busy = cabinet.swap_len();
            }
        });
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Where to leave a cabinet for the chain: see `CabStage`
    pub fn cab_stage(&self) -> Arc<CabStage> {
        self.cab_stage.clone()
    }

    fn configure_amp(&mut self) {
        let model = self.amp.model();
        let oversampled_rate = self.oversampled_rate();
        self.preamp.configure(model, oversampled_rate);
        self.power.configure(&model.power, oversampled_rate);
        self.tone_applied = [f32::NAN; 3];
        self.presence_applied = f32::NAN;
        self.cab_applied = [f32::NAN; 2];
    }

    pub fn reset(&mut self) {
        self.reset_stages();
        self.primed = false;
        self.until_tick = 0;
        // Starts again with the first block if it is still switched on
        self.tuning = false;
        self.tuner.stop();
    }

    /// What the tuner hears, for the editor
    pub fn tuner_reading(&self) -> Arc<TunerReading> {
        self.tuner.reading()
    }

    fn reset_stages(&mut self) {
        self.gate.reset();
        self.reset_amp_stages();
        self.cabinet.reset();
        self.cab_voicing.reset();
        self.dc.reset();
        // An idle effect has nothing left to be heard already. Clearing its lines would
        // only cost time, and this is also called on the audio thread, when Bypass has faded
        if !self.delay.is_idle() {
            self.delay.reset();
        }
        if !self.reverb.is_idle() {
            self.reverb.reset();
        }
        self.amp_fade = self.amp_fade_len;
        self.cabinet_busy = 0;
    }

    /// True when nothing is left to hear of earlier input: no repeats and no reverb tail are
    /// ringing. The amp and the cabinet themselves are silent within milliseconds
    pub fn is_idle(&self) -> bool {
        self.delay.is_idle() && self.reverb.is_idle()
    }

    /// Everything between the gate and the cabinet. The effects are not touched: their
    /// repeats and tail ring on while another amp is set up
    fn reset_amp_stages(&mut self) {
        self.oversampler.reset();
        self.drive.reset();
        self.preamp.reset();
        self.tone.reset();
        self.power.reset();
    }

    /// Processes a block in place. With two channels the input is their average and the
    /// output is stereo: the amp in the middle, the repeats and the reverb tail around it.
    /// With one channel the output is the left one of those two. Their sum would keep the amp
    /// as it is as well, but the tail and the repeats come out 3 dB lower in it, and the
    /// repeats hollow: left and right repeat a few milliseconds apart, which is a comb
    pub fn process(&mut self, settings: &AmpSettings, left: &mut [f32], mut right: Option<&mut [f32]>) {
        self.wanted_amp = settings.amp;
        let wet_target = if settings.bypass { 0.0 } else { 1.0 };
        if !self.primed {
            self.wet = wet_target;
        }

        let mut start = 0;
        while start < left.len() {
            self.follow_amp();
            self.follow_cabinet();
            if self.until_tick == 0 {
                self.read_dials(settings);
                self.until_tick = CHUNK;
            }
            let mut len = (left.len() - start).min(self.until_tick);
            if self.wanted_amp != self.amp {
                // The piece ends where the amp has faded to silence, or where the cabinet is
                // ready for the next amp, so the switch falls on that sample
                let wait = if self.amp_fade > 0 { self.amp_fade as usize } else { self.cabinet_busy };
                if wait > 0 {
                    len = len.min(wait);
                }
            }
            let end = start + len;
            let right_chunk = right.as_deref_mut().map(|right| &mut right[start..end]);
            self.process_chunk(wet_target, &mut left[start..end], right_chunk);
            self.cabinet_busy = self.cabinet_busy.saturating_sub(len);
            self.until_tick -= len;
            start = end;
        }
    }

    /// Moves the dials a step towards the settings and passes them on to the stages
    fn read_dials(&mut self, settings: &AmpSettings) {
        let target = Dials::from_settings(settings);
        // Nothing is heard of the amp while bypassed, so there is nothing to smooth
        let jump = !self.primed || self.wet == 0.0;
        if jump {
            self.dials = target;
        } else {
            self.dials.approach(&target, self.dial_coeff);
        }

        self.in_gain.set_target(db_to_gain(self.dials.in_gain_db), CHUNK as u32);
        self.out_level.set_target(self.dials.out_level, CHUNK as u32);

        // The switches and the gate's settings are read here as well, not once per block,
        // so that they too take effect on the same sample whatever the block size
        self.gate.set(
            settings.gate_on,
            settings.gate_thresh_db.clamp(GATE_THRESHOLD_DB[0], GATE_THRESHOLD_DB[1]),
            settings.gate_release_ms.clamp(GATE_RELEASE_MS[0], GATE_RELEASE_MS[1]),
        );
        self.drive.set_on(settings.drive_on);
        self.drive.set(
            self.dials.drive_gain,
            self.dials.drive_tone,
            self.dials.drive_level,
            self.oversampled_chunk(),
        );
        let cab_target = if settings.cab_on { 1.0 } else { 0.0 };
        if cab_target != self.cab_mix.target() {
            self.cab_mix.set_target(cab_target, self.cab_fade_steps);
        }
        self.delay_settings = settings.delay;
        self.reverb_settings = settings.reverb;

        if settings.tuner_on != self.tuning {
            self.tuning = settings.tuner_on;
            if self.tuning {
                self.tuner.start();
            } else {
                self.tuner.stop();
            }
        }
        let mute_target = if self.tuning { 0.0 } else { 1.0 };
        if mute_target != self.tuner_mute.target() {
            self.tuner_mute.set_target(mute_target, self.tuner_fade_steps);
        }

        if jump {
            self.tuner_mute.snap();
            self.in_gain.snap();
            self.out_level.snap();
            self.gate.snap();
            self.drive.snap();
            self.cab_mix.snap();
        }
        self.apply_dials(jump);
        self.primed = true;
    }

    /// Passes the amp's own dials on to its stages and its cabinet, as the amp they are set
    /// up as reads them
    fn apply_dials(&mut self, snap: bool) {
        let model = self.amp.model();
        self.preamp.set_gain(model, self.dials.gain, self.oversampled_chunk());
        self.power.set_master(
            &model.power,
            self.dials.master,
            curve(&model.makeup_db, self.dials.gain),
            self.oversampled_chunk(),
        );
        if snap {
            self.preamp.snap();
            self.power.snap();
        }

        // The filters of the dials glide to where the dial is now over the piece that
        // follows, so that reading the dials once per piece is not heard as steps
        let oversampled_rate = self.oversampled_rate();
        let steps = if snap { 0 } else { self.oversampled_chunk() };
        let tone = [self.dials.bass, self.dials.mid, self.dials.treble];
        if tone != self.tone_applied {
            self.tone.set(&ToneCurve::new(&model.tone, tone[0], tone[1], tone[2], oversampled_rate), steps);
            self.tone_applied = tone;
        }
        if self.dials.presence != self.presence_applied {
            self.power.set_presence(&model.power, self.dials.presence, oversampled_rate, steps);
            self.presence_applied = self.dials.presence;
        }
        let cab = [self.dials.cab_mic, self.dials.cab_res];
        if cab != self.cab_applied {
            let steps = if snap { 0 } else { CHUNK as u32 };
            self.cab_voicing.set(&model.cab, cab[0], cab[1], self.sample_rate, steps);
            self.cab_applied = cab;
        }
    }

    /// Fades the amp's output down while another amp is waiting, and back up after
    fn fade_amp(&mut self, block: &mut [f32]) {
        let down = self.wanted_amp != self.amp;
        if !down && self.amp_fade == self.amp_fade_len {
            return;
        }
        for sample in block.iter_mut() {
            if down {
                self.amp_fade = self.amp_fade.saturating_sub(1);
            } else if self.amp_fade < self.amp_fade_len {
                self.amp_fade += 1;
            }
            // Eased at both ends
            let position = self.amp_fade as f32 / self.amp_fade_len as f32;
            *sample *= position * position * (3.0 - 2.0 * position);
        }
    }

    /// The cabinet with its dials, or the amp's own signal, or on the way between the two.
    /// The cabinet always runs, so it has its history when it is switched back on
    fn process_cabinet(&mut self, block: &mut [f32]) {
        let all_cabinet = self.cab_mix.value() == 1.0 && self.cab_mix.target() == 1.0;
        let mut direct = [0.0f32; CHUNK];
        if !all_cabinet {
            direct[..block.len()].copy_from_slice(block);
        }

        self.cabinet.process(block);
        self.cab_voicing.process(block);

        if !all_cabinet {
            let direct_gain = db_to_gain(CAB_OFF_DB);
            for (sample, &direct) in block.iter_mut().zip(&direct) {
                let direct = direct * direct_gain;
                *sample = direct + self.cab_mix.next() * (*sample - direct);
            }
        }
    }

    /// At most `CHUNK` samples
    fn process_chunk(&mut self, wet_target: f32, left: &mut [f32], mut right: Option<&mut [f32]>) {
        let bypassed = |chain: &Self| chain.wet == 0.0 && wet_target == 0.0;
        if bypassed(self) && !self.tuning {
            return;
        }
        let len = left.len();

        let mut signal = [0.0f32; CHUNK];
        match &right {
            Some(right) => {
                for ((mono, l), r) in signal.iter_mut().zip(left.iter()).zip(right.iter()) {
                    *mono = 0.5 * (l + r);
                }
            }
            None => signal[..len].copy_from_slice(left),
        }

        // The tuner hears the input as it arrives, whatever the amp does with it
        if self.tuning {
            self.tuner.process(&signal[..len]);
            if bypassed(self) {
                return;
            }
        }

        // While tuning the amp gets silence, so nothing of it is left in the repeats and
        // the tail afterwards, and what still rings in them is turned down behind them
        let muting = self.tuner_mute.value() != 1.0 || self.tuner_mute.target() != 1.0;
        let mut heard_share = [1.0f32; CHUNK];
        if muting {
            for share in &mut heard_share[..len] {
                *share = self.tuner_mute.next();
            }
        }

        // The gate listens to the input as it arrives and acts on it after the input gain
        let heard = signal;
        for sample in &mut signal[..len] {
            *sample *= self.in_gain.next();
        }
        if muting {
            for (sample, share) in signal[..len].iter_mut().zip(&heard_share) {
                *sample *= share;
            }
        }
        self.gate.process(&heard[..len], &mut signal[..len]);

        let mut oversampled = [0.0f32; OVERSAMPLED_CHUNK];
        let high = &mut oversampled[..len * self.factor];
        self.oversampler.upsample(&signal[..len], high);
        self.drive.process(high);
        self.preamp.process(high);
        self.tone.process(high);
        self.power.process(high);
        self.oversampler.downsample(high, &mut signal[..len]);

        self.fade_amp(&mut signal[..len]);
        self.process_cabinet(&mut signal[..len]);

        // Two channels from here on. The effects add to what they are given and leave the
        // amp's own signal as it is, on time. Both are called whether they are on or not:
        // switched off they let their repeats and tail ring out, and idle they do nothing
        let mut wide = [[0.0f32; CHUNK]; 2];
        for index in 0..len {
            let sample = self.dc.process(signal[index]);
            wide[0][index] = sample;
            wide[1][index] = sample;
        }
        let [wide_left, wide_right] = &mut wide;
        self.delay.process(&self.delay_settings, &mut wide_left[..len], &mut wide_right[..len]);
        self.reverb.process(&self.reverb_settings, &mut wide_left[..len], &mut wide_right[..len]);

        // The effects do not limit themselves, so the safety clip comes last
        for index in 0..len {
            let level = self.out_level.next();
            if self.wet != wet_target {
                self.wet = (self.wet + self.wet_step.copysign(wet_target - self.wet)).clamp(0.0, 1.0);
            }

            let mut output = output_clip(wide_left[index] * level);
            if muting {
                output *= heard_share[index];
            }
            if self.wet >= 1.0 {
                left[index] = output;
            } else {
                left[index] += self.wet * (output - left[index]);
            }
            if let Some(right) = right.as_deref_mut() {
                let mut output = output_clip(wide_right[index] * level);
                if muting {
                    output *= heard_share[index];
                }
                if self.wet >= 1.0 {
                    right[index] = output;
                } else {
                    right[index] += self.wet * (output - right[index]);
                }
            }
        }

        // Start from silence when the amp comes back, not from where it was left, and
        // without the repeats and the tail of what was played before
        if bypassed(self) {
            self.reset_stages();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cab::{user_ir_len, MAX_USER_IR_LEN, USER_IR_MS};
    use crate::cab_stage::{CabLoader, CabStatus};
    use crate::drive::tests::new_drive;
    use crate::dsp::shaper::{asym_clip, AsymClipper, OUTPUT_CLIP_KNEE};
    use crate::gate::tests::{decaying_note, gain_trace, hiss, transitions as gate_changes};
    use crate::test_util::*;
    use crate::tuner::tests::{note_hz as tuner_note_hz, string as tuner_string};
    use crate::user_cab::tests::{test_dir, whole_response, write_ir_wav};
    use crate::user_cab::{self, Recording};
    use std::sync::Mutex;
    use std::time::Instant;

    const SAMPLE_RATE: f32 = 48000.0;
    const BLOCK: usize = 64;

    // Tones for the aliasing measurements: their harmonics fall between each other's
    // aliases at the usual sample rates
    const ALIAS_TONES_HZ: [f32; 2] = [1245.0, 4186.0];
    // Per amp, at each of the tones
    const ALIAS_LIMITS_DB: [[f64; 2]; 3] = [[-90.0, -78.0], [-90.0, -78.0], [-84.0, -84.0]];

    fn new_chain(sample_rate: f32) -> AmpChain {
        let mut chain = AmpChain::new();
        chain.set_sample_rate(sample_rate);
        chain
    }

    fn with_gain(gain: f32) -> AmpSettings {
        AmpSettings {
            gain,
            ..AmpSettings::default()
        }
    }

    fn with_amp(amp: Amp, gain: f32) -> AmpSettings {
        AmpSettings {
            amp,
            gain,
            ..AmpSettings::default()
        }
    }

    fn with_all_dials(amp: Amp, value: f32, out_level: f32) -> AmpSettings {
        AmpSettings {
            amp,
            gain: value,
            bass: value,
            mid: value,
            treble: value,
            presence: value,
            master: value,
            out_level,
            ..AmpSettings::default()
        }
    }

    /// Mono, in blocks of `block` samples
    fn run_blocks(chain: &mut AmpChain, settings: &AmpSettings, input: &[f32], block: usize) -> Vec<f32> {
        let mut output = input.to_vec();
        for chunk in output.chunks_mut(block) {
            chain.process(settings, chunk, None);
        }
        output
    }

    fn run(settings: &AmpSettings, input: &[f32], sample_rate: f32) -> Vec<f32> {
        run_blocks(&mut new_chain(sample_rate), settings, input, BLOCK)
    }

    fn largest_step(samples: &[f32]) -> f32 {
        samples.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max)
    }

    /// Samples from an impulse until the output first reaches half of its peak
    fn latency_samples(amp: Amp, sample_rate: f32) -> usize {
        let mut impulse = vec![0.0; 2048];
        impulse[0] = 0.05;
        let output = run(&with_amp(amp, 0.0), &impulse, sample_rate);
        let top = peak(&output);
        output.iter().position(|s| s.abs() >= 0.5 * top).unwrap()
    }

    /// The same for any settings, also with the gate on: a quiet tone opens the gate, and
    /// the impulse comes while it is still held open. What the impulse adds to the output
    /// is the output with it minus the output without
    fn latency_samples_with(settings: &AmpSettings, sample_rate: f32) -> usize {
        let lead = (0.05 * sample_rate) as usize;
        let at = lead + (0.025 * sample_rate) as usize;
        let mut quiet = sine(220.0, 0.05, sample_rate, lead);
        quiet.resize(at + 2048, 0.0);
        let mut struck = quiet.clone();
        struck[at] += 0.05;
        let (quiet, struck) = (run(settings, &quiet, sample_rate), run(settings, &struck, sample_rate));
        let added: Vec<f32> = struck[at..].iter().zip(&quiet[at..]).map(|(a, b)| a - b).collect();
        let top = peak(&added);
        added.iter().position(|s| s.abs() >= 0.5 * top).unwrap()
    }

    /// Every pedal on and every new dial off its centre
    fn everything_on(amp: Amp, gain: f32) -> AmpSettings {
        AmpSettings {
            gate_on: true,
            drive_on: true,
            cab_mic: 0.7,
            cab_res: 0.7,
            ..with_amp(amp, gain)
        }
    }

    fn with_drive(amp: Amp, gain: f32, drive: f32, tone: f32, level: f32) -> AmpSettings {
        AmpSettings {
            drive_on: true,
            drive_gain: drive,
            drive_tone: tone,
            drive_level: level,
            ..with_amp(amp, gain)
        }
    }

    fn delay_on(time_ms: f32, feedback: f32, mix: f32) -> DelaySettings {
        DelaySettings {
            on: true,
            time_ms,
            feedback,
            mix,
        }
    }

    fn reverb_on(decay_s: f32, mix: f32) -> ReverbSettings {
        ReverbSettings {
            on: true,
            decay_s,
            mix,
        }
    }

    /// Delay and reverb on, both short: heard within a fraction of a second and soon over
    fn with_effects(base: AmpSettings) -> AmpSettings {
        AmpSettings {
            delay: delay_on(60.0, 0.4, 0.5),
            reverb: reverb_on(0.4, 0.5),
            ..base
        }
    }

    /// Both effects as loud and as long as they go
    fn with_effects_at_most(base: AmpSettings) -> AmpSettings {
        AmpSettings {
            delay: delay_on(350.0, 0.9, 1.0),
            reverb: reverb_on(6.0, 1.0),
            ..base
        }
    }

    /// Both effects switched on and otherwise as the plugin starts
    fn with_default_effects(base: AmpSettings) -> AmpSettings {
        AmpSettings {
            delay: DelaySettings {
                on: true,
                ..DelaySettings::default()
            },
            reverb: ReverbSettings {
                on: true,
                ..ReverbSettings::default()
            },
            ..base
        }
    }

    /// The stereo layout with the same input on both channels, in blocks of `block` samples
    fn run_stereo_blocks(chain: &mut AmpChain, settings: &AmpSettings, input: &[f32], block: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
            chain.process(settings, l, Some(r));
        }
        (left, right)
    }

    fn run_stereo(settings: &AmpSettings, input: &[f32], sample_rate: f32) -> (Vec<f32>, Vec<f32>) {
        run_stereo_blocks(&mut new_chain(sample_rate), settings, input, BLOCK)
    }

    fn difference(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b).map(|(a, b)| a - b).collect()
    }

    fn largest_difference(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
    }

    /// Address and capacity of every buffer the chain points to
    fn buffer_layout(chain: &AmpChain) -> Vec<(usize, usize)> {
        let mut layout = vec![(chain.cab_irs.as_ptr() as usize, chain.cab_irs.capacity())];
        layout.extend(chain.cab_irs.iter().map(|ir| (ir.taps.as_ptr() as usize, ir.taps.capacity())));
        layout.extend(chain.cabinet.buffers());
        layout.push(chain.cab_stage.buffer());
        layout.extend(chain.delay.buffers());
        layout.extend(chain.reverb.buffers());
        layout
    }

    /// Energy of a clipped sine that is not at its harmonics, relative to the total, in dB
    fn aliasing_db(amp: Amp, gain: f32, freq_hz: f32) -> f64 {
        aliasing_db_with(&with_amp(amp, gain), freq_hz)
    }

    fn aliasing_db_with(settings: &AmpSettings, freq_hz: f32) -> f64 {
        aliasing_db_at(settings, freq_hz, SAMPLE_RATE)
    }

    /// Three quarters of a second of the tone, the last half second measured
    fn aliasing_db_at(settings: &AmpSettings, freq_hz: f32, sample_rate: f32) -> f64 {
        let len = (0.75 * sample_rate) as usize;
        let input = sine(freq_hz, 0.178, sample_rate, len);
        let output = run(settings, &input, sample_rate);
        fit_partials(&output[len / 3..], sample_rate, &harmonics_of(freq_hz, sample_rate)).1
    }

    /// The six dials that are filters, with a tone each one moves
    const SWEPT_DIALS: [(&str, f32); 6] = [("bass", 110.0), ("mid", 550.0), ("treble", 5000.0), ("presence", 6000.0), ("cab_mic", 5000.0), ("cab_res", 80.0)];

    /// Zipper noise of a dial swept from 0 to 10 in `sweep_s` seconds while a sine plays
    /// through Klar at Gain 0: the dials are read once per `CHUNK` samples, and what that
    /// leaves in the sound is beside the sine, at that rate above and below it. The louder
    /// of the two against the sine, in dB, in the middle of the sweep
    fn zipper_db(dial: &str, freq_hz: f32, sweep_s: f32, sample_rate: f32) -> f32 {
        let len = (sweep_s * sample_rate) as usize;
        let mut output = sine(freq_hz, 0.178, sample_rate, len);
        let mut chain = new_chain(sample_rate);
        for (index, block) in output.chunks_mut(CHUNK).enumerate() {
            let position = (index * CHUNK) as f32 / len as f32;
            let mut settings = with_amp(Amp::Klar, 0.0);
            match dial {
                "bass" => settings.bass = position,
                "mid" => settings.mid = position,
                "treble" => settings.treble = position,
                "cab_mic" => settings.cab_mic = position,
                "cab_res" => settings.cab_res = position,
                _ => settings.presence = position,
            }
            chain.process(&settings, block, None);
        }
        let tick_hz = (sample_rate / CHUNK as f32) as f64;
        let freq = freq_hz as f64;
        let partials = [freq, freq - tick_hz, freq + tick_hz];
        let (levels, _) = fit_partials(&output[len / 4..3 * len / 4], sample_rate, &partials);
        to_db((levels[1].max(levels[2]) / levels[0]) as f32)
    }

    const STAGE_NAMES: [&str; 11] = [
        "gate", "up", "drive", "preamp", "tone", "power", "down", "cabinet", "delay", "reverb", "output",
    ];

    /// Time each stage of the chain takes by itself over `input`, in the order of
    /// `STAGE_NAMES`, in ns per sample at the host's rate. The stages are the chain's own,
    /// set up by playing through it first, and run in pieces of `CHUNK` samples as there
    fn stage_times_ns(settings: &AmpSettings, input: &[f32], sample_rate: f32) -> Vec<f64> {
        let mut chain = new_chain(sample_rate);
        run_stereo_blocks(&mut chain, settings, &input[..input.len() / 4], BLOCK);
        let factor = chain.factor;
        let len = input.len();
        let mut times = Vec::new();
        let mut timed = |work: &mut dyn FnMut()| {
            let start = Instant::now();
            work();
            times.push(start.elapsed().as_secs_f64() * 1e9 / len as f64);
        };

        let mut signal = input.to_vec();
        timed(&mut || {
            for (heard, chunk) in input.chunks(CHUNK).zip(signal.chunks_mut(CHUNK)) {
                for sample in chunk.iter_mut() {
                    *sample *= chain.in_gain.next();
                }
                chain.gate.process(heard, chunk);
            }
        });
        let mut high = vec![0.0; len * factor];
        timed(&mut || {
            for (chunk, high_chunk) in signal.chunks(CHUNK).zip(high.chunks_mut(CHUNK * factor)) {
                chain.oversampler.upsample(chunk, high_chunk);
            }
        });
        timed(&mut || high.chunks_mut(CHUNK * factor).for_each(|chunk| chain.drive.process(chunk)));
        timed(&mut || high.chunks_mut(CHUNK * factor).for_each(|chunk| chain.preamp.process(chunk)));
        timed(&mut || high.chunks_mut(CHUNK * factor).for_each(|chunk| chain.tone.process(chunk)));
        timed(&mut || high.chunks_mut(CHUNK * factor).for_each(|chunk| chain.power.process(chunk)));
        timed(&mut || {
            for (high_chunk, chunk) in high.chunks(CHUNK * factor).zip(signal.chunks_mut(CHUNK)) {
                chain.oversampler.downsample(high_chunk, chunk);
            }
        });
        timed(&mut || {
            for chunk in signal.chunks_mut(CHUNK) {
                chain.cabinet.process(chunk);
                chain.cab_voicing.process(chunk);
            }
        });

        // The output stage is in two parts, around the effects: the DC blocker and the
        // split into two channels, then the output level and the safety clip
        let (mut left, mut right) = (vec![0.0; len], vec![0.0; len]);
        let start = Instant::now();
        for ((sample, l), r) in signal.iter().zip(left.iter_mut()).zip(right.iter_mut()) {
            *l = chain.dc.process(*sample);
            *r = *l;
        }
        let split_ns = start.elapsed().as_secs_f64() * 1e9 / len as f64;
        timed(&mut || {
            for (l, r) in left.chunks_mut(CHUNK).zip(right.chunks_mut(CHUNK)) {
                chain.delay.process(&settings.delay, l, r);
            }
        });
        timed(&mut || {
            for (l, r) in left.chunks_mut(CHUNK).zip(right.chunks_mut(CHUNK)) {
                chain.reverb.process(&settings.reverb, l, r);
            }
        });
        timed(&mut || {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let level = chain.out_level.next();
                *l = output_clip(*l * level);
                *r = output_clip(*r * level);
            }
        });
        std::hint::black_box((&left, &right));
        *times.last_mut().unwrap() += split_ns;
        times
    }

    /// The amp and its cabinet put together from their parts, with nothing else around
    /// them: what the chain was before it had pedals and effects
    fn amp_alone(settings: &AmpSettings, input: &[f32], sample_rate: f32) -> Vec<f32> {
        let model = settings.amp.model();
        let factor = factor_for(sample_rate);
        let oversampled_rate = sample_rate * factor as f32;
        let mut oversampler = Oversampler::new();
        oversampler.set_factor(factor);
        let mut preamp = Preamp::new();
        preamp.configure(model, oversampled_rate);
        preamp.reset();
        preamp.set_gain(model, settings.gain, 0);
        let mut tone = ToneStack::new();
        tone.set(&ToneCurve::new(&model.tone, settings.bass, settings.mid, settings.treble, oversampled_rate), 0);
        let mut power = PowerAmp::new();
        power.configure(&model.power, oversampled_rate);
        power.reset();
        power.set_master(&model.power, settings.master, curve(&model.makeup_db, settings.gain), 0);
        power.set_presence(&model.power, settings.presence, oversampled_rate, 0);
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&design_ir(&model.cab, sample_rate));
        let mut dc = DcBlocker::new();
        dc.set_sample_rate(sample_rate);

        let mut high = vec![0.0; input.len() * factor];
        oversampler.upsample(input, &mut high);
        preamp.process(&mut high);
        tone.process(&mut high);
        power.process(&mut high);
        let mut output = vec![0.0; input.len()];
        oversampler.downsample(&high, &mut output);
        cabinet.process(&mut output);
        output.iter().map(|&sample| output_clip(dc.process(sample) * settings.out_level)).collect()
    }

    #[test]
    fn test_silence_in_gives_silence_out() {
        for amp in Amp::ALL {
            let output = run(&with_all_dials(amp, 1.0, 1.0), &vec![0.0; 24_000], SAMPLE_RATE);
            assert!(peak(&output) < 1e-5, "{:?} peak: {}", amp, peak(&output));

            // Also once a loud note has rung out, and without an offset left behind
            let mut input = sine(110.0, 0.8, SAMPLE_RATE, 12_000);
            input.extend(vec![0.0; 36_000]);
            let output = run(&with_amp(amp, 1.0), &input, SAMPLE_RATE);
            let tail = &output[36_000..];
            assert!(peak(tail) < 1e-5, "{:?} tail peak: {}", amp, peak(tail));
            let playing = &output[2400..12_000];
            let offset = playing.iter().sum::<f32>() / playing.len() as f32;
            assert!(offset.abs() < 0.01 * rms(playing), "{:?} offset: {}", amp, offset);
        }
    }

    #[test]
    fn test_output_is_bounded_at_all_sample_rates() {
        for sample_rate in [44100.0, 48000.0, 88200.0, 96000.0, 192000.0] {
            let len = (sample_rate * 0.05) as usize;
            let mut noise = Noise::new(3);
            let input: Vec<f32> = sine(97.0, 1.0, sample_rate, len).iter().map(|s| s + noise.next()).collect();

            for amp in Amp::ALL {
                for (value, out_level) in [(0.0, 0.0), (0.0, 2.0), (1.0, 2.0)] {
                    let output = run(&with_all_dials(amp, value, out_level), &input, sample_rate);
                    assert!(output.iter().all(|s| s.is_finite()), "{:?} not finite at {} Hz", amp, sample_rate);
                    assert!(peak(&output) <= 1.0, "{:?} peak {} at {} Hz", amp, peak(&output), sample_rate);
                }
                let loud = run(&with_all_dials(amp, 1.0, 2.0), &input, sample_rate);
                assert!(rms(&loud) > 0.05, "{:?} gives no output at {} Hz", amp, sample_rate);

                // And with every pedal on and every dial at either end
                for (value, cab_on) in [(0.0, true), (1.0, true), (1.0, false), (0.0, false)] {
                    let settings = AmpSettings {
                        in_gain_db: -24.0 + 48.0 * value,
                        gate_on: true,
                        gate_thresh_db: -80.0 + 60.0 * (1.0 - value),
                        gate_release_ms: 20.0 + 480.0 * value,
                        drive_on: true,
                        drive_gain: value,
                        drive_tone: value,
                        drive_level: value,
                        cab_on,
                        cab_mic: value,
                        cab_res: value,
                        ..with_all_dials(amp, value, 2.0)
                    };
                    let output = run(&settings, &input, sample_rate);
                    assert!(output.iter().all(|s| s.is_finite()), "{:?} not finite at {} Hz", amp, sample_rate);
                    assert!(peak(&output) <= 1.0, "{:?} peak {} at {} Hz", amp, peak(&output), sample_rate);
                }
            }
        }
    }

    #[test]
    fn test_output_does_not_depend_on_block_size() {
        let input = power_chords(SAMPLE_RATE, 0.2);
        for amp in Amp::ALL {
            let pedals = AmpSettings {
                in_gain_db: 3.0,
                gate_thresh_db: -30.0,
                gate_release_ms: 20.0,
                ..everything_on(amp, 0.7)
            };
            let no_cabinet = AmpSettings {
                cab_on: false,
                ..pedals
            };
            for settings in [with_amp(amp, 0.7), pedals, no_cabinet] {
                let reference = run_blocks(&mut new_chain(SAMPLE_RATE), &settings, &input, input.len());

                for block in [1, 7, 32, 64, 1000] {
                    let output = run_blocks(&mut new_chain(SAMPLE_RATE), &settings, &input, block);
                    let difference = reference.iter().zip(&output).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
                    assert!(difference < 1e-5, "{:?} in blocks of {}: off by {}", amp, block, difference);
                }
            }
        }
    }

    #[test]
    fn test_gain_dial_adds_distortion() {
        let input = sine(220.0, 0.178, SAMPLE_RATE, 19_200);
        let distortion = |gain: f32| thd_db(&run(&with_gain(gain), &input, SAMPLE_RATE)[9600..], SAMPLE_RATE, 220.0);
        let (clean, crunch, full) = (distortion(0.0), distortion(0.5), distortion(1.0));

        assert!(clean < -26.0, "Gain 0 is not clean: {:.1} dB", clean);
        assert!(crunch > clean + 10.0, "Gain 5 at {:.1} dB, gain 0 at {:.1} dB", crunch, clean);
        assert!(full > crunch + 1.0, "Gain 10 at {:.1} dB, gain 5 at {:.1} dB", full, crunch);
    }

    #[test]
    fn test_level_is_consistent_across_the_gain_dial() {
        let input = power_chords(SAMPLE_RATE, 0.5);
        for amp in Amp::ALL {
            let levels: Vec<f32> = [0.0, 0.25, 0.5, 0.75, 1.0]
                .iter()
                .map(|&gain| to_db(rms(&run(&with_amp(amp, gain), &input, SAMPLE_RATE))))
                .collect();
            let loudest = levels.iter().cloned().fold(f32::MIN, f32::max);
            let quietest = levels.iter().cloned().fold(f32::MAX, f32::min);
            assert!(loudest - quietest < 6.0, "{:?} levels: {:?}", amp, levels);
            assert!((-19.0..=-13.0).contains(&levels[2]), "{:?} at the defaults: {:.1} dBFS", amp, levels[2]);
        }
    }

    #[test]
    fn test_klar_leaves_room_under_the_safety_clip() {
        // The clean amp has the highest peaks for its level. With the delay and the reverb
        // on as the plugin starts, chords stay a decibel under the knee of the output clip
        let chords = power_chords(SAMPLE_RATE, 2.0);
        for gain in [0.0, 0.25, 0.5] {
            let settings = with_default_effects(with_amp(Amp::Klar, gain));
            let (left, right) = run_stereo(&settings, &chords, SAMPLE_RATE);
            let room = to_db(OUTPUT_CLIP_KNEE) - to_db(peak(&left).max(peak(&right)));
            assert!(room > 1.0, "Gain {}: {:.2} dB under the knee", gain * 10.0, room);
        }
    }

    #[test]
    fn test_amps_are_equally_loud_at_the_defaults() {
        let input = power_chords(SAMPLE_RATE, 2.0);
        let levels: Vec<f32> =
            Amp::ALL.iter().map(|&amp| to_db(rms(&run(&with_amp(amp, 0.5), &input, SAMPLE_RATE)))).collect();
        let loudest = levels.iter().cloned().fold(f32::MIN, f32::max);
        let quietest = levels.iter().cloned().fold(f32::MAX, f32::min);
        assert!(loudest - quietest < 2.0, "Levels: {:?}", levels);
    }

    #[test]
    fn test_amps_differ_in_distortion_and_spectrum() {
        let chords = power_chords(SAMPLE_RATE, 1.0);
        let distortion = |gain: f32, level: f32| -> Vec<f32> {
            let tone = sine(220.0, level, SAMPLE_RATE, 19_200);
            Amp::ALL
                .iter()
                .map(|&amp| thd_db(&run(&with_amp(amp, gain), &tone, SAMPLE_RATE)[9600..], SAMPLE_RATE, 220.0))
                .collect()
        };
        // A sine at -18 dBFS RMS with the gain low, and one at -36 dBFS RMS with the gain at
        // 5. Louder or with more gain, Brøl and Torden both clip all the way, and which of
        // the two then measures more is a matter of their tone, not of how much they distort
        for (gain, level) in [(0.0, 0.178), (0.25, 0.178), (0.5, 0.0224)] {
            let amps = distortion(gain, level);
            assert!(amps[0] < amps[1] - 6.0, "Gain {}: Klar and Brøl {:?}", gain * 10.0, amps);
            assert!(amps[1] < amps[2] - 3.0, "Gain {}: Brøl and Torden {:?}", gain * 10.0, amps);
        }
        // Klar is the cleanest wherever the dial is
        for gain in [0.5, 1.0] {
            let amps = distortion(gain, 0.178);
            assert!(amps[0] < amps[1] - 6.0 && amps[0] < amps[2] - 6.0, "Gain {}: {:?}", gain * 10.0, amps);
        }

        let spectra: Vec<Vec<f32>> = Amp::ALL
            .iter()
            .map(|&amp| band_levels_db(&run(&with_amp(amp, 0.5), &chords, SAMPLE_RATE), SAMPLE_RATE))
            .collect();
        for (first, second) in [(0, 1), (1, 2), (0, 2)] {
            let apart = spectra[first].iter().zip(&spectra[second]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(apart > 2.0, "{:?} and {:?} are {:.1} dB apart", Amp::ALL[first], Amp::ALL[second], apart);
        }
    }

    #[test]
    fn test_klar_stays_clean_until_gain_is_high() {
        let tone = sine(220.0, 0.178, SAMPLE_RATE, 19_200);
        let distortion =
            |gain: f32| thd_db(&run(&with_amp(Amp::Klar, gain), &tone, SAMPLE_RATE)[9600..], SAMPLE_RATE, 220.0);
        for gain in [0.0, 0.25, 0.5] {
            assert!(distortion(gain) < -40.0, "Gain {}: {:.1} dB", gain * 10.0, distortion(gain));
        }
        // Breaks up at the top, gently
        let full = distortion(1.0);
        assert!((-32.0..-14.0).contains(&full), "Gain 10: {:.1} dB", full);
    }

    #[test]
    fn test_soft_playing_cleans_up_klar_and_brol() {
        // -36 and -12 dBFS RMS
        let (soft, hard) = (sine(220.0, 0.0224, SAMPLE_RATE, 19_200), sine(220.0, 0.355, SAMPLE_RATE, 19_200));
        for amp in [Amp::Klar, Amp::Brol] {
            let distortion =
                |input: &[f32]| thd_db(&run(&with_amp(amp, 0.5), input, SAMPLE_RATE)[9600..], SAMPLE_RATE, 220.0);
            assert!(distortion(&soft) < -30.0, "{:?} played softly: {:.1} dB", amp, distortion(&soft));
            assert!(distortion(&soft) < distortion(&hard) - 12.0);
        }

        // And how much of the 24 dB between the two each amp takes away: more with more gain
        let squeezed: Vec<f32> = Amp::ALL
            .iter()
            .map(|&amp| {
                let level = |input: &[f32]| to_db(rms(&run(&with_amp(amp, 0.5), input, SAMPLE_RATE)[9600..]));
                24.0 - (level(&hard) - level(&soft))
            })
            .collect();
        assert!(squeezed[0] < 3.0, "Klar squeezes by {:.1} dB", squeezed[0]);
        assert!(squeezed[1] > squeezed[0] + 6.0 && squeezed[2] > squeezed[1] + 6.0, "Squeezed: {:?}", squeezed);
    }

    /// Level below 100 Hz against the level from 400 to 1600 Hz, in dB
    fn lows_against_mids_db(signal: &[f32]) -> f32 {
        band_level_db(signal, SAMPLE_RATE, None, Some(TIGHT_EDGES_HZ[0]))
            - band_level_db(signal, SAMPLE_RATE, Some(TIGHT_EDGES_HZ[1]), Some(TIGHT_EDGES_HZ[2]))
    }

    /// Level of a note's fundamental against the whole signal, in dB
    fn fundamental_db(signal: &[f32], freq_hz: f32) -> f32 {
        to_db(level_at(signal, SAMPLE_RATE, freq_hz) / std::f32::consts::SQRT_2 / rms(signal))
    }

    #[test]
    fn test_torden_keeps_palm_mutes_tight() {
        let input = palm_mutes(SAMPLE_RATE, 2.4);
        for gain in [0.5, 1.0] {
            let brol = lows_against_mids_db(&run(&with_amp(Amp::Brol, gain), &input, SAMPLE_RATE));
            let torden = lows_against_mids_db(&run(&with_amp(Amp::Torden, gain), &input, SAMPLE_RATE));
            assert!(torden < brol - 1.0, "Gain {}: Torden {:.1} dB, Brøl {:.1} dB", gain * 10.0, torden, brol);
            assert!(torden < -15.0, "Gain {}: lows at {:.1} dB against the mids", gain * 10.0, torden);
        }
    }

    #[test]
    fn test_tone_dials_move_their_bands() {
        // Quiet and with little gain, so the amp is close to linear
        let level = |settings: &AmpSettings, freq_hz: f32| {
            let input = sine(freq_hz, 0.01, SAMPLE_RATE, 9600);
            to_db(rms(&run(settings, &input, SAMPLE_RATE)[4800..]))
        };
        let base = with_gain(0.2);

        let bass = |value| level(&AmpSettings { bass: value, ..base }, 100.0);
        assert!(bass(1.0) > bass(0.0) + 6.0, "Bass: {:.1} to {:.1} dB", bass(0.0), bass(1.0));

        let mid = |value| level(&AmpSettings { mid: value, ..base }, 650.0);
        assert!(mid(1.0) > mid(0.0) + 6.0, "Mid: {:.1} to {:.1} dB", mid(0.0), mid(1.0));

        let treble = |value| level(&AmpSettings { treble: value, ..base }, 4000.0);
        assert!(treble(1.0) > treble(0.0) + 6.0, "Treble: {:.1} to {:.1} dB", treble(0.0), treble(1.0));

        let presence = |value| level(&AmpSettings { presence: value, ..base }, 5000.0);
        assert!(presence(1.0) > presence(0.0) + 3.0, "Presence: {:.1} to {:.1} dB", presence(0.0), presence(1.0));

        // And each leaves the far end of the spectrum mostly alone
        let far = |low: AmpSettings, high: AmpSettings, freq_hz| (level(&high, freq_hz) - level(&low, freq_hz)).abs();
        assert!(far(AmpSettings { bass: 0.0, ..base }, AmpSettings { bass: 1.0, ..base }, 4000.0) < 3.0);
        assert!(far(AmpSettings { treble: 0.0, ..base }, AmpSettings { treble: 1.0, ..base }, 100.0) < 3.0);
        assert!(far(AmpSettings { presence: 0.0, ..base }, AmpSettings { presence: 1.0, ..base }, 100.0) < 1.0);
    }

    #[test]
    fn test_aliasing_stays_low_at_full_gain() {
        // Limits are a few dB above what `amp_report` measures
        for (amp, limits) in Amp::ALL.into_iter().zip(ALIAS_LIMITS_DB) {
            let low = aliasing_db(amp, 1.0, ALIAS_TONES_HZ[0]);
            let high = aliasing_db(amp, 1.0, ALIAS_TONES_HZ[1]);
            assert!(low < limits[0], "{:?}: {:.1} dB at {} Hz", amp, low, ALIAS_TONES_HZ[0]);
            assert!(high < limits[1], "{:?}: {:.1} dB at {} Hz", amp, high, ALIAS_TONES_HZ[1]);
        }
    }

    #[test]
    fn test_twice_oversampled_at_96_khz_sounds_like_48_khz_and_aliases_as_little() {
        // From 88.2 kHz on the amp runs at twice the host's rate, not four times
        assert_eq!(new_chain(48000.0).factor, 4);
        assert_eq!(new_chain(88200.0).factor, 2);
        assert_eq!(new_chain(96000.0).factor, 2);

        let settings = with_drive(Amp::Torden, 1.0, 0.3, 0.5, 0.8);
        let measure = |sample_rate: f32| {
            let output = run(&settings, &same_chords(sample_rate, 0.5), sample_rate);
            let mut levels = vec![to_db(rms(&output))];
            levels.extend(band_levels_db(&output, sample_rate));
            levels
        };
        let (low, high) = (measure(SAMPLE_RATE), measure(96000.0));
        for (index, (a, b)) in low.iter().zip(&high).enumerate() {
            assert!((a - b).abs() < 0.5, "Level {}: {:.2} dB at 48 kHz, {:.2} dB at 96 kHz", index, a, b);
        }

        let aliasing = aliasing_db_at(&settings, ALIAS_TONES_HZ[1], 96000.0);
        assert!(aliasing < -70.0, "Aliasing at 96 kHz: {:.1} dB", aliasing);
    }

    #[test]
    fn test_latency_is_under_half_a_millisecond() {
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0] {
                let latency_ms = latency_samples(amp, sample_rate) as f32 / sample_rate * 1000.0;
                assert!(latency_ms < 0.5, "{:?} at {} Hz: {:.2} ms", amp, sample_rate, latency_ms);

                // The pedals add next to nothing, the gate nothing at all
                for tone in [0.0, 0.5, 1.0] {
                    let settings = AmpSettings {
                        drive_tone: tone,
                        ..everything_on(amp, 0.0)
                    };
                    let with_pedals = latency_samples_with(&settings, sample_rate) as f32 / sample_rate * 1000.0;
                    assert!(with_pedals < 0.5, "{:?} at {} Hz with pedals: {:.2} ms", amp, sample_rate, with_pedals);
                }
            }
        }
    }

    #[test]
    fn test_bypass_leaves_both_channels_untouched() {
        let mut chain = new_chain(SAMPLE_RATE);
        let mut noise = Noise::new(11);
        let left_in: Vec<f32> = (0..4800).map(|_| noise.next()).collect();
        let right_in: Vec<f32> = (0..4800).map(|_| noise.next() * 0.3).collect();
        let (mut left, mut right) = (left_in.clone(), right_in.clone());

        let settings = AmpSettings {
            bypass: true,
            ..AmpSettings::default()
        };
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            chain.process(&settings, l, Some(r));
        }
        assert_eq!(left, left_in);
        assert_eq!(right, right_in);
    }

    #[test]
    fn test_bypass_switches_without_a_click() {
        let mut chain = new_chain(SAMPLE_RATE);
        let input = sine(220.0, 0.3, SAMPLE_RATE, 28_800);
        let (mut left, mut right) = (input.clone(), input.clone());
        let playing = AmpSettings::default();
        let bypassed = AmpSettings {
            bypass: true,
            ..playing
        };

        // Playing, bypassed, playing again
        for (index, (l, r)) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)).enumerate() {
            let settings = if (150..300).contains(&index) { &bypassed } else { &playing };
            chain.process(settings, l, Some(r));
        }
        assert_eq!(left, right);

        let (off, on) = (150 * BLOCK, 300 * BLOCK);
        let fade = (BYPASS_FADE_MS * 0.001 * SAMPLE_RATE) as usize + 1;
        assert_eq!(left[off + fade..on], input[off + fade..on]);
        assert!(rms(&left[on + 4800..]) > 0.01);

        let own_step = largest_step(&left[4800..off]).max(largest_step(&input));
        let step_off = largest_step(&left[off - 1..off + fade + 1]);
        let step_on = largest_step(&left[on - 1..on + 2400]);
        assert!(step_off < own_step * 1.2, "Step of {} into bypass, {} while playing", step_off, own_step);
        assert!(step_on < own_step * 1.2, "Step of {} out of bypass, {} while playing", step_on, own_step);
    }

    #[test]
    fn test_dial_jumps_do_not_click() {
        let input = sine(220.0, 0.178, SAMPLE_RATE, 24_000);
        let low = AmpSettings {
            out_level: 0.25,
            ..with_all_dials(Amp::Brol, 0.2, 0.25)
        };
        let jumps = [
            AmpSettings { gain: 1.0, ..low },
            AmpSettings { bass: 1.0, ..low },
            AmpSettings { mid: 1.0, ..low },
            AmpSettings { treble: 1.0, ..low },
            AmpSettings { presence: 1.0, ..low },
            AmpSettings { master: 1.0, ..low },
            AmpSettings { out_level: 1.0, ..low },
            with_all_dials(Amp::Brol, 1.0, 1.0),
            AmpSettings { in_gain_db: 12.0, ..low },
            AmpSettings { cab_mic: 1.0, ..low },
            AmpSettings { cab_mic: 0.0, ..low },
            AmpSettings { cab_res: 1.0, ..low },
            AmpSettings { cab_res: 0.0, ..low },
        ];

        for high in jumps {
            let mut chain = new_chain(SAMPLE_RATE);
            let mut output = input.clone();
            for (index, block) in output.chunks_mut(BLOCK).enumerate() {
                chain.process(if index < 150 { &low } else { &high }, block, None);
            }

            // The waveform's own steepest slope, before the jump and once it has settled
            let jump = 150 * BLOCK;
            let own_step = largest_step(&output[4800..jump]).max(largest_step(&output[jump + 9600..]));
            let step = largest_step(&output[jump - 1..jump + 9600]);
            assert!(step < own_step * 1.3, "Step of {} against {} for {:?}", step, own_step, high);
        }
    }

    #[test]
    fn test_fast_dial_sweeps_leave_no_zipper_noise() {
        // The filters of the dials glide between the positions the dials are read at
        for (dial, freq_hz) in SWEPT_DIALS {
            let zipper = zipper_db(dial, freq_hz, 0.2, SAMPLE_RATE);
            assert!(zipper < -90.0, "{} swept in 200 ms: {:.1} dB beside the sine", dial, zipper);
        }
    }

    #[test]
    fn test_stereo_outputs_are_equal_and_match_mono() {
        let left_in = power_chords(SAMPLE_RATE, 0.2);
        let right_in: Vec<f32> = sine(330.0, 0.1, SAMPLE_RATE, left_in.len());
        let mono_in: Vec<f32> = left_in.iter().zip(&right_in).map(|(l, r)| 0.5 * (l + r)).collect();
        let settings = AmpSettings::default();

        let (mut left, mut right) = (left_in.clone(), right_in.clone());
        let mut chain = new_chain(SAMPLE_RATE);
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            chain.process(&settings, l, Some(r));
        }
        let mono = run(&settings, &mono_in, SAMPLE_RATE);

        assert_eq!(left, right);
        assert_eq!(left, mono);
        assert!(rms(&mono) > 0.01);
    }

    #[test]
    fn test_reset_starts_over() {
        let input = power_chords(SAMPLE_RATE, 0.1);
        let mut chain = new_chain(SAMPLE_RATE);
        let first = run_blocks(&mut chain, &with_gain(0.8), &input, BLOCK);
        chain.reset();
        let second = run_blocks(&mut chain, &with_gain(0.8), &input, BLOCK);
        assert_eq!(first, second);
    }

    /// Plays `input` and changes from one amp to the other at sample `at`
    fn run_switch(chain: &mut AmpChain, from: Amp, to: Amp, gain: f32, input: &[f32], at: usize) -> Vec<f32> {
        let mut output = input.to_vec();
        let (before, after) = output.split_at_mut(at);
        for block in before.chunks_mut(BLOCK) {
            chain.process(&with_amp(from, gain), block, None);
        }
        for block in after.chunks_mut(BLOCK) {
            chain.process(&with_amp(to, gain), block, None);
        }
        output
    }

    fn transitions() -> Vec<(Amp, Amp)> {
        let mut pairs = Vec::new();
        for from in Amp::ALL {
            for to in Amp::ALL {
                if from != to {
                    pairs.push((from, to));
                }
            }
        }
        pairs
    }

    #[test]
    fn test_switching_amps_does_not_click() {
        let at = 300 * BLOCK;
        let settle = 9600;
        for input in [sine(220.0, 0.178, SAMPLE_RATE, 48_000), power_chords(SAMPLE_RATE, 1.0)] {
            for gain in [0.3, 1.0] {
                for (from, to) in transitions() {
                    let output = run_switch(&mut new_chain(SAMPLE_RATE), from, to, gain, &input, at);

                    // The steepest slope either amp makes by itself, against the steepest
                    // one around the switch
                    let own_step = largest_step(&output[4800..at]).max(largest_step(&output[at + settle..]));
                    let step = largest_step(&output[at - 1..at + settle]);
                    assert!(
                        step < own_step * 1.2,
                        "{:?} to {:?} at gain {}: step of {} against {}",
                        from,
                        to,
                        gain,
                        step,
                        own_step
                    );

                    // And the amp is silent in between: the output dips, it does not jump
                    let silent = at + (AMP_FADE_MS * 0.001 * SAMPLE_RATE) as usize;
                    assert!(peak(&output[silent - 2..silent + 2]) < 0.5 * peak(&output[4800..at]));
                }
            }
        }
    }

    #[test]
    fn test_switching_amps_with_the_pedals_on_does_not_click() {
        // The drive keeps running through the switch, and the cabinet's dials move to the
        // new cabinet's frequencies while the old one still rings. The chords go on long
        // enough for the new amp to be heard on pick attacks of its own after the switch
        let at = 300 * BLOCK;
        for input in [sine(220.0, 0.178, SAMPLE_RATE, 48_000), power_chords(SAMPLE_RATE, 2.0)] {
            for (from, to) in transitions() {
                let pedals = |amp: Amp| AmpSettings {
                    cab_mic: 1.0,
                    cab_res: 1.0,
                    ..everything_on(amp, 0.5)
                };
                let output = run_change(&pedals(from), &pedals(to), &input, at);
                let ratio = step_ratio(&output, at);
                assert!(ratio < 1.2, "{:?} to {:?}: step {} times its own", from, to, ratio);
            }
        }
    }

    #[test]
    fn test_switching_ends_up_as_the_amp_itself() {
        let input = power_chords(SAMPLE_RATE, 2.0);
        let at = 300 * BLOCK;
        for (from, to) in transitions() {
            let switched = run_switch(&mut new_chain(SAMPLE_RATE), from, to, 0.5, &input, at);
            let reference = run(&with_amp(to, 0.5), &input, SAMPLE_RATE);
            let settled = input.len() - 24_000;
            let difference = switched[settled..]
                .iter()
                .zip(&reference[settled..])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            let level = rms(&reference[settled..]);
            assert!(difference < 0.01 * level, "{:?} to {:?}: off by {} at a level of {}", from, to, difference, level);
        }
    }

    #[test]
    fn test_switching_back_and_forth_quickly_does_not_click() {
        // A new amp every 3 ms: faster than the fades and the cabinet's crossfade
        let input = sine(220.0, 0.178, SAMPLE_RATE, 48_000);
        let mut chain = new_chain(SAMPLE_RATE);
        let mut output = input.clone();
        for (index, block) in output.chunks_mut(BLOCK).enumerate() {
            let amp = if (150..450).contains(&index) { Amp::ALL[(index / 2) % 3] } else { Amp::Torden };
            chain.process(&with_amp(amp, 0.7), block, None);
        }
        let own_step = largest_step(&output[4800..150 * BLOCK]);
        let step = largest_step(&output[150 * BLOCK - 1..]);
        assert!(step < own_step * 1.2, "Step of {} against {}", step, own_step);

        let reference = run(&with_amp(Amp::Torden, 0.7), &input, SAMPLE_RATE);
        let difference = output[43_200..].iter().zip(&reference[43_200..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(difference < 0.01 * rms(&reference[43_200..]), "Off by {}", difference);
    }

    #[test]
    fn test_switch_while_bypassed_or_before_playing_is_at_once() {
        let input = power_chords(SAMPLE_RATE, 0.5);
        let bypassed = |amp: Amp| AmpSettings {
            bypass: true,
            ..with_amp(amp, 0.5)
        };

        for (from, to) in transitions() {
            // Bypassed on one amp, then on the other, then playing
            let mut chain = new_chain(SAMPLE_RATE);
            let mut switched = input.clone();
            let mut reference = input.clone();
            let mut reference_chain = new_chain(SAMPLE_RATE);
            for (index, (block, reference_block)) in
                switched.chunks_mut(BLOCK).zip(reference.chunks_mut(BLOCK)).enumerate()
            {
                let settings = match index {
                    0..=49 => bypassed(from),
                    50..=99 => bypassed(to),
                    _ => with_amp(to, 0.5),
                };
                chain.process(&settings, block, None);
                let reference_settings = if index < 100 { bypassed(to) } else { settings };
                reference_chain.process(&reference_settings, reference_block, None);
            }
            assert_eq!(switched, reference, "{:?} to {:?}", from, to);
            assert!(rms(&switched[100 * BLOCK + 4800..]) > 0.01);

            // Set up as one amp, never played, then playing as the other: no fade at the start
            let mut chain = new_chain(SAMPLE_RATE);
            chain.process(&with_amp(from, 0.5), &mut [], None);
            let late = run_blocks(&mut chain, &with_amp(to, 0.5), &input, BLOCK);
            assert_eq!(late, run(&with_amp(to, 0.5), &input, SAMPLE_RATE), "{:?} to {:?}", from, to);
        }
    }

    #[test]
    fn test_switching_does_not_allocate() {
        // What can be seen from here: every buffer stays where it is, with the size it had
        let layout = buffer_layout;
        for sample_rate in [44100.0, 192000.0] {
            let mut chain = new_chain(sample_rate);
            let before = layout(&chain);
            let mut block = sine(220.0, 0.2, sample_rate, 96_000);
            for (index, piece) in block.chunks_mut(BLOCK).enumerate() {
                chain.process(&with_amp(Amp::ALL[(index / 40) % 3], 0.5), piece, None);
            }
            assert_eq!(layout(&chain), before);
            assert_eq!(chain.cab_irs.len(), Amp::ALL.len());
        }
    }

    /// Plays `input` with one settings up to sample `at` and with another from there
    fn run_change(before: &AmpSettings, after: &AmpSettings, input: &[f32], at: usize) -> Vec<f32> {
        let mut chain = new_chain(SAMPLE_RATE);
        let mut output = input.to_vec();
        let (first, second) = output.split_at_mut(at);
        for block in first.chunks_mut(BLOCK) {
            chain.process(before, block, None);
        }
        for block in second.chunks_mut(BLOCK) {
            chain.process(after, block, None);
        }
        output
    }

    /// The largest step around a change of settings against the largest the sound makes by
    /// itself before the change and once it has settled
    fn step_ratio(output: &[f32], at: usize) -> f32 {
        let settle = 9600;
        let own_step = largest_step(&output[4800..at]).max(largest_step(&output[at + settle..]));
        largest_step(&output[at - 1..at + settle]) / own_step
    }

    #[test]
    fn test_pedals_off_and_dials_centred_is_the_amp_alone() {
        let input = power_chords(SAMPLE_RATE, 0.5);
        for amp in Amp::ALL {
            for gain in [0.0, 0.7] {
                // Bit for bit: no pedal and no effect leaves a trace, wherever its dials are
                let settings = AmpSettings {
                    gate_on: false,
                    gate_thresh_db: -20.0,
                    drive_on: false,
                    drive_gain: 1.0,
                    drive_tone: 0.0,
                    drive_level: 1.0,
                    delay: DelaySettings {
                        on: false,
                        ..delay_on(20.0, 0.9, 1.0)
                    },
                    reverb: ReverbSettings {
                        on: false,
                        ..reverb_on(6.0, 1.0)
                    },
                    ..with_amp(amp, gain)
                };
                let reference = amp_alone(&settings, &input, SAMPLE_RATE);
                assert_eq!(run(&settings, &input, SAMPLE_RATE), reference, "{:?} at gain {}", amp, gain * 10.0);
                let (left, right) = run_stereo(&settings, &input, SAMPLE_RATE);
                assert!(left == reference && right == reference, "{:?} at gain {} in stereo", amp, gain * 10.0);

                // The gate, open, changes nothing but the half millisecond in which it opens
                let gated = run(&with_amp(amp, gain), &input, SAMPLE_RATE);
                let difference =
                    gated[12_000..].iter().zip(&reference[12_000..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
                assert!(difference < 1e-5, "{:?} at gain {}: off by {} with the gate on", amp, gain * 10.0, difference);
            }
        }
    }

    #[test]
    fn test_pedals_switched_off_again_leave_the_amp_alone() {
        // Once the fades are over and what the amp remembers of the pedals has died away
        let input = power_chords(SAMPLE_RATE, 3.0);
        let at = 300 * BLOCK;
        for amp in Amp::ALL {
            let plain = AmpSettings {
                gate_on: false,
                ..with_amp(amp, 0.5)
            };
            let pedals = AmpSettings {
                in_gain_db: 6.0,
                cab_on: false,
                ..everything_on(amp, 0.5)
            };
            let output = run_change(&pedals, &plain, &input, at);
            let reference = run(&plain, &input, SAMPLE_RATE);
            let settled = input.len() - 24_000;
            let difference =
                output[settled..].iter().zip(&reference[settled..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(difference < 0.01 * rms(&reference[settled..]), "{:?}: off by {}", amp, difference);
        }
    }

    #[test]
    fn test_input_gain_scales_the_input() {
        // Quiet and with little gain, so the amp is close to linear
        let input = sine(440.0, 0.002, SAMPLE_RATE, 19_200);
        let level = |in_gain_db: f32| {
            let settings = AmpSettings {
                in_gain_db,
                gate_on: false,
                ..with_amp(Amp::Klar, 0.2)
            };
            to_db(rms(&run(&settings, &input, SAMPLE_RATE)[9600..]))
        };
        let unity = level(0.0);
        for in_gain_db in [-24.0, -6.0, 12.0, 24.0] {
            let change = level(in_gain_db) - unity;
            assert!((change - in_gain_db).abs() < 0.5, "{} dB at the input gives {:.2} dB", in_gain_db, change);
        }
    }

    #[test]
    fn test_gate_opens_on_a_note_and_closes_after_it() {
        let mut input = hiss(-75.0, 96_000);
        for (sample, note) in input[24_000..48_000].iter_mut().zip(sine(110.0, 0.2, SAMPLE_RATE, 24_000)) {
            *sample += note;
        }
        for amp in Amp::ALL {
            let gated = run(&with_amp(amp, 1.0), &input, SAMPLE_RATE);
            let open = run(
                &AmpSettings {
                    gate_on: false,
                    ..with_amp(amp, 1.0)
                },
                &input,
                SAMPLE_RATE,
            );
            // Silent before the note, the note as without the gate, silent again after
            assert!(peak(&gated[..24_000]) < 1e-9, "{:?} before the note: {}", amp, peak(&gated[..24_000]));
            assert!(peak(&open[12_000..24_000]) > 1e-4, "{:?} has no hiss to gate", amp);
            let playing = 28_800..48_000;
            let difference = gated[playing.clone()]
                .iter()
                .zip(&open[playing.clone()])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(difference < 0.02 * rms(&open[playing]), "{:?}: the note differs by {}", amp, difference);
            assert!(peak(&gated[60_000..]) < 1e-6, "{:?} after the note: {}", amp, peak(&gated[60_000..]));
        }
    }

    #[test]
    fn test_gate_threshold_is_the_level_at_the_plugin_input() {
        // The input gain may not open the gate: 24 dB more of a hiss under the threshold
        let input = hiss(-75.0, 24_000);
        let settings = AmpSettings {
            in_gain_db: 24.0,
            ..with_amp(Amp::Torden, 1.0)
        };
        let output = run(&settings, &input, SAMPLE_RATE);
        assert!(peak(&output) < 1e-9, "Peak: {}", peak(&output));

        // And a note over the threshold opens it however far the input gain is turned down
        let note = sine(220.0, 0.1, SAMPLE_RATE, 24_000);
        let settings = AmpSettings {
            in_gain_db: -24.0,
            ..with_amp(Amp::Torden, 1.0)
        };
        assert!(rms(&run(&settings, &note, SAMPLE_RATE)) > 0.01);
    }

    #[test]
    fn test_gate_does_not_chatter_or_click_on_a_decaying_note() {
        let note = decaying_note(SAMPLE_RATE, 6.0);
        for (amp, gain) in [(Amp::Klar, 0.5), (Amp::Torden, 1.0)] {
            for release_ms in [20.0, 100.0] {
                let settings = AmpSettings {
                    gate_thresh_db: -40.0,
                    gate_release_ms: release_ms,
                    ..with_amp(amp, gain)
                };
                let output = run(&settings, &note, SAMPLE_RATE);
                // Heard from the start, quiet once, and quiet from there on
                let heard: Vec<bool> = output.chunks(480).map(|piece| peak(piece) > 1e-6).collect();
                let changes = heard.windows(2).filter(|pair| pair[0] != pair[1]).count();
                assert!(heard[0] && !heard[heard.len() - 1], "{:?} at {} ms release", amp, release_ms);
                assert_eq!(changes, 1, "{:?} at {} ms release", amp, release_ms);
                // And closing makes no step larger than the note makes while it rings
                let last = output.iter().rposition(|s| s.abs() > 1e-6).unwrap();
                let own_step = largest_step(&output[4800..48_000]);
                let closing = largest_step(&output[last - 24_000..]);
                assert!(closing <= own_step, "{:?}: step of {} while closing, {} while ringing", amp, closing, own_step);
            }
        }
    }

    #[test]
    fn test_gate_switches_without_a_click() {
        // A tone under the threshold: the gate is all that decides whether it is heard
        let input = sine(220.0, 0.005, SAMPLE_RATE, 48_000);
        let closed = AmpSettings {
            gate_thresh_db: -30.0,
            ..with_amp(Amp::Brol, 0.5)
        };
        let off = AmpSettings {
            gate_on: false,
            ..closed
        };
        let at = 300 * BLOCK;
        let opened = run_change(&closed, &off, &input, at);
        assert!(peak(&opened[..at]) < 1e-9);
        let own_step = largest_step(&opened[at + 9600..]);
        assert!(largest_step(&opened[at - 1..]) < own_step * 1.2);

        let shut = run_change(&off, &closed, &input, at);
        let own_step = largest_step(&shut[4800..at]);
        assert!(largest_step(&shut[at - 1..]) < own_step * 1.2);
        assert!(peak(&shut[at + 24_000..]) < 1e-9);
    }

    #[test]
    fn test_drive_adds_distortion_and_cuts_lows() {
        let tone = sine(220.0, 0.178, SAMPLE_RATE, 19_200);
        let distortion = |settings: &AmpSettings| thd_db(&run(settings, &tone, SAMPLE_RATE)[9600..], SAMPLE_RATE, 220.0);
        let clean = distortion(&with_amp(Amp::Klar, 0.3));
        let low = distortion(&with_drive(Amp::Klar, 0.3, 0.0, 0.5, 0.5));
        let centre = distortion(&with_drive(Amp::Klar, 0.3, 0.5, 0.5, 0.5));
        let full = distortion(&with_drive(Amp::Klar, 0.3, 1.0, 0.5, 0.5));
        assert!(clean < -40.0 && low < -20.0, "Klar alone {:.1} dB, Drive 0 {:.1} dB", clean, low);
        assert!(centre > clean + 15.0 && centre > low + 4.0, "Drive 5: {:.1} dB", centre);
        assert!(full > centre, "Drive 10 at {:.1} dB, Drive 5 at {:.1} dB", full, centre);

        // Less low end against the mids, on a clean amp and on a high gain one
        let mutes = palm_mutes(SAMPLE_RATE, 2.4);
        for (amp, gain) in [(Amp::Klar, 0.3), (Amp::Torden, 0.5)] {
            let plain = lows_against_mids_db(&run(&with_amp(amp, gain), &mutes, SAMPLE_RATE));
            let driven = lows_against_mids_db(&run(&with_drive(amp, gain, 0.2, 0.5, 0.8), &mutes, SAMPLE_RATE));
            assert!(driven < plain - 2.0, "{:?}: lows at {:.1} dB without, {:.1} dB with the drive", amp, plain, driven);
        }
    }

    #[test]
    fn test_drive_tightens_torden_beyond_brol() {
        let mutes = palm_mutes(SAMPLE_RATE, 2.4);
        for gain in [0.5, 1.0] {
            let brol = lows_against_mids_db(&run(&with_amp(Amp::Brol, gain), &mutes, SAMPLE_RATE));
            let torden =
                lows_against_mids_db(&run(&with_drive(Amp::Torden, gain, 0.2, 0.5, 0.8), &mutes, SAMPLE_RATE));
            assert!(torden < brol - 5.0, "Gain {}: Torden with drive {:.1} dB, Brøl {:.1} dB", gain * 10.0, torden, brol);
        }

        // A single low E still has its fundamental: the drive takes little of what the amp leaves
        let note = pluck(82.41, Pluck::OPEN, 3, SAMPLE_RATE, 48_000);
        let fundamental = |settings: &AmpSettings| fundamental_db(&run(settings, &note, SAMPLE_RATE)[4800..28_800], 82.41);
        let (plain, driven) = (
            fundamental(&with_amp(Amp::Torden, 0.5)),
            fundamental(&with_drive(Amp::Torden, 0.5, 0.2, 0.5, 0.8)),
        );
        assert!(
            driven > plain - 6.0 && driven > -40.0,
            "Fundamental at {:.1} dB, {:.1} dB without the drive",
            driven,
            plain
        );
    }

    #[test]
    fn test_drive_tone_and_level_move_the_right_way() {
        let chords = power_chords(SAMPLE_RATE, 1.0);
        let top = |tone: f32| {
            let output = run(&with_drive(Amp::Klar, 0.3, 0.5, tone, 0.5), &chords, SAMPLE_RATE);
            band_level_db(&output, SAMPLE_RATE, Some(3000.0), None)
        };
        let (dark, centre, bright) = (top(0.0), top(0.5), top(1.0));
        assert!(centre > dark + 1.5 && bright > centre + 1.5, "Above 3 kHz: {:.1}, {:.1}, {:.1} dB", dark, centre, bright);

        // Klar turned down so far that it follows its input
        let quiet: Vec<f32> = chords.iter().map(|s| s * 0.05).collect();
        let level = |settings: &AmpSettings| to_db(rms(&run(settings, &quiet, SAMPLE_RATE)));
        let pedal = |level_dial: f32| AmpSettings {
            gate_on: false,
            master: 0.2,
            ..with_drive(Amp::Klar, 0.2, 0.0, 0.5, level_dial)
        };
        let (low, centre, high) = (level(&pedal(0.0)), level(&pedal(0.5)), level(&pedal(1.0)));
        assert!(
            (centre - low - 20.0).abs() < 1.5 && (high - centre - 20.0).abs() < 1.5,
            "Level 0, 5 and 10: {:.1}, {:.1}, {:.1} dB",
            low,
            centre,
            high
        );
    }

    #[test]
    fn test_drive_switches_without_a_click() {
        let at = 300 * BLOCK;
        for input in [sine(220.0, 0.178, SAMPLE_RATE, 48_000), power_chords(SAMPLE_RATE, 1.0)] {
            for (amp, gain) in [(Amp::Klar, 0.5), (Amp::Torden, 0.7)] {
                for (drive, level) in [(0.3, 0.5), (1.0, 0.8)] {
                    let on = with_drive(amp, gain, drive, 0.5, level);
                    let off = AmpSettings { drive_on: false, ..on };
                    for (before, after) in [(&off, &on), (&on, &off)] {
                        let ratio = step_ratio(&run_change(before, after, &input, at), at);
                        assert!(ratio < 1.2, "{:?}, drive on {}: step {} times its own", amp, after.drive_on, ratio);
                    }
                }
            }
        }
    }

    #[test]
    fn test_drive_dial_jumps_do_not_click() {
        let input = sine(220.0, 0.178, SAMPLE_RATE, 24_000);
        let low = with_drive(Amp::Klar, 0.3, 0.2, 0.2, 0.3);
        let jumps = [
            AmpSettings { drive_gain: 1.0, ..low },
            AmpSettings { drive_tone: 1.0, ..low },
            AmpSettings { drive_level: 0.8, ..low },
        ];
        for high in jumps {
            let output = run_change(&low, &high, &input, 150 * BLOCK);
            let ratio = step_ratio(&output[..], 150 * BLOCK);
            assert!(ratio < 1.3, "Step {} times its own for {:?}", ratio, high);
        }
    }

    #[test]
    fn test_cabinet_switches_without_a_click_and_keeps_its_level() {
        let at = 300 * BLOCK;
        for input in [sine(220.0, 0.178, SAMPLE_RATE, 48_000), power_chords(SAMPLE_RATE, 1.0)] {
            for amp in Amp::ALL {
                let on = with_amp(amp, 0.5);
                let off = AmpSettings { cab_on: false, ..on };
                for (before, after) in [(&off, &on), (&on, &off)] {
                    let output = run_change(before, after, &input, at);
                    let ratio = step_ratio(&output, at);
                    assert!(ratio < 1.3, "{:?}, cabinet on {}: step {} times its own", amp, after.cab_on, ratio);
                    // Ends up as if it had been that way all along: the cabinet kept its history
                    let reference = run(after, &input, SAMPLE_RATE);
                    let settled = at + 4800;
                    let difference = output[settled..]
                        .iter()
                        .zip(&reference[settled..])
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f32::max);
                    assert!(difference < 1e-4, "{:?}, cabinet on {}: off by {}", amp, after.cab_on, difference);
                }
            }
        }

        // Without the cabinet the top is open, and it is about as loud
        let chords = power_chords(SAMPLE_RATE, 2.0);
        for amp in Amp::ALL {
            let with = run(&with_amp(amp, 0.5), &chords, SAMPLE_RATE);
            let without = run(
                &AmpSettings {
                    cab_on: false,
                    ..with_amp(amp, 0.5)
                },
                &chords,
                SAMPLE_RATE,
            );
            let fizz = |output: &[f32]| band_level_db(output, SAMPLE_RATE, Some(8000.0), None);
            if amp != Amp::Klar {
                let (with, without) = (fizz(&with), fizz(&without));
                assert!(without > with + 6.0, "{:?}: {:.1} dB and {:.1} dB above 8 kHz", amp, with, without);
            }
            let difference = to_db(rms(&without) / rms(&with));
            assert!(difference.abs() < 3.0, "{:?}: {:.1} dB louder without the cabinet", amp, difference);
        }
    }

    #[test]
    fn test_cabinet_dials_move_their_bands() {
        // Quiet and with little gain, so the amp is close to linear
        let level = |settings: &AmpSettings, freq_hz: f32| {
            let input = sine(freq_hz, 0.01, SAMPLE_RATE, 9600);
            to_db(rms(&run(settings, &input, SAMPLE_RATE)[4800..]))
        };
        for amp in Amp::ALL {
            let base = AmpSettings {
                gate_on: false,
                ..with_amp(amp, 0.2)
            };
            let resonance_hz = amp.model().cab.resonance_hz;
            // What the dial does at a frequency, at its dark and at its bright end
            let mic = |freq_hz: f32| {
                [0.0, 1.0].map(|value| level(&AmpSettings { cab_mic: value, ..base }, freq_hz) - level(&base, freq_hz))
            };
            let res = |freq_hz: f32| {
                [0.0, 1.0].map(|value| level(&AmpSettings { cab_res: value, ..base }, freq_hz) - level(&base, freq_hz))
            };

            let top = mic(4000.0);
            assert!(top[0] < -3.0 && top[1] > 3.0, "{:?} mic at 4 kHz: {:.1}, {:.1} dB", amp, top[0], top[1]);
            let lows = mic(150.0);
            assert!(lows[0] > 0.3 && lows[1] < -0.3, "{:?} mic at 150 Hz: {:.1}, {:.1} dB", amp, lows[0], lows[1]);
            let mids = mic(800.0);
            assert!(mids[0].abs() < 1.5 && mids[1].abs() < 1.5);

            let thump = res(resonance_hz);
            assert!(thump[0] < -4.5 && thump[1] > 4.5, "{:?} resonance: {:.1}, {:.1} dB", amp, thump[0], thump[1]);
            let mids = res(1000.0);
            assert!(mids[0].abs() < 0.5 && mids[1].abs() < 0.5);
        }
    }

    #[test]
    fn test_cabinet_dials_stay_about_as_loud() {
        let chords = power_chords(SAMPLE_RATE, 2.0);
        for amp in Amp::ALL {
            let level = |settings: &AmpSettings| to_db(rms(&run(settings, &chords, SAMPLE_RATE)));
            let base = with_amp(amp, 0.5);
            for mic in [0.0, 1.0] {
                let change = level(&AmpSettings { cab_mic: mic, ..base }) - level(&base);
                assert!(change.abs() < 1.5, "{:?} with Mic at {}: {:.1} dB", amp, mic * 10.0, change);
            }
            for res in [0.0, 1.0] {
                let change = level(&AmpSettings { cab_res: res, ..base }) - level(&base);
                assert!(change.abs() < 2.5, "{:?} with Resonance at {}: {:.1} dB", amp, res * 10.0, change);
            }
        }
    }

    #[test]
    fn test_aliasing_stays_low_with_the_drive_in_front_of_torden() {
        let settings = with_drive(Amp::Torden, 1.0, 0.3, 0.5, 0.8);
        for freq_hz in ALIAS_TONES_HZ {
            let aliasing = aliasing_db_with(&settings, freq_hz);
            assert!(aliasing < -74.0, "{:.1} dB at {} Hz", aliasing, freq_hz);
        }
    }

    #[test]
    fn test_pedals_and_effects_do_not_allocate() {
        // The pedals and the tuner own no buffers at all: a chain is as large as its fields,
        // and the only memory it points to is the cabinet's and the lines of the delay and
        // the reverb (and what the tuner shows, which it shares with the editor)
        for sample_rate in [44100.0, 192000.0] {
            let mut chain = new_chain(sample_rate);
            let before = buffer_layout(&chain);
            let mut block = sine(220.0, 0.2, sample_rate, 96_000);
            for (index, piece) in block.chunks_mut(BLOCK).enumerate() {
                let turn = index / 40;
                let settings = AmpSettings {
                    in_gain_db: (turn % 5) as f32 * 6.0 - 12.0,
                    gate_on: turn % 2 == 0,
                    gate_thresh_db: -20.0 - (turn % 3) as f32 * 20.0,
                    drive_on: turn % 3 == 0,
                    drive_gain: (turn % 4) as f32 / 3.0,
                    cab_on: turn % 5 != 0,
                    cab_mic: (turn % 3) as f32 * 0.5,
                    cab_res: (turn % 4) as f32 / 3.0,
                    bypass: turn % 7 == 6,
                    tuner_on: turn % 4 == 3,
                    delay: DelaySettings {
                        on: turn % 2 == 1,
                        ..delay_on([20.0, 1000.0, 350.0][turn % 3], 0.9, 1.0)
                    },
                    reverb: ReverbSettings {
                        on: turn % 4 < 2,
                        ..reverb_on([0.3, 6.0][turn % 2], 1.0)
                    },
                    ..with_amp(Amp::ALL[turn % 3], 0.5)
                };
                chain.process(&settings, piece, None);
            }
            assert_eq!(buffer_layout(&chain), before);
            chain.reset();
            assert_eq!(buffer_layout(&chain), before);
        }
    }

    #[test]
    fn test_closed_gate_and_silence_are_not_slower() {
        // Everything on, played, then left alone: the gate closes, and nothing behind it
        // may decay into denormal numbers
        let mut chain = new_chain(SAMPLE_RATE);
        let settings = everything_on(Amp::Torden, 1.0);
        let playing = block_time_us(&mut chain, &settings, &power_chords(SAMPLE_RATE, 0.25));
        let quiet = hiss(-75.0, 48_000);
        let tail = run_blocks(&mut chain, &settings, &quiet, BLOCK);
        assert!(peak(&tail[24_000..]) < 1e-6, "The gate is not closed: {}", peak(&tail[24_000..]));
        let closed = block_time_us(&mut chain, &settings, &hiss(-75.0, 12_000));
        let silent = block_time_us(&mut chain, &settings, &vec![0.0; 12_000]);
        assert!(closed < playing * SLOWED_DOWN, "{:.1} us per block playing, {:.1} us with the gate closed", playing, closed);
        assert!(silent < playing * SLOWED_DOWN, "{:.1} us per block playing, {:.1} us silent", playing, silent);
    }

    /// Median time to process one block, in microseconds
    fn block_time_us(chain: &mut AmpChain, settings: &AmpSettings, input: &[f32]) -> f64 {
        let mut scratch = input.to_vec();
        let mut times: Vec<f64> = scratch
            .chunks_mut(BLOCK)
            .map(|block| {
                let start = Instant::now();
                chain.process(settings, block, None);
                start.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[times.len() / 2]
    }

    #[test]
    fn test_silence_after_playing_is_not_slower() {
        // States that decay into denormal numbers would make the silent blocks many times
        // slower. Short time constants reach that range within this test; `amp_report`
        // measures a long tail
        let mut chain = new_chain(SAMPLE_RATE);
        let settings = with_gain(1.0);
        let playing = block_time_us(&mut chain, &settings, &power_chords(SAMPLE_RATE, 0.25));
        run_blocks(&mut chain, &settings, &vec![0.0; 24_000], BLOCK);
        let silent = block_time_us(&mut chain, &settings, &vec![0.0; 12_000]);
        assert!(silent < playing * SLOWED_DOWN, "{:.1} us per block playing, {:.1} us silent", playing, silent);
    }

    #[test]
    fn test_effects_switched_off_leave_no_trace() {
        // Bit for bit, wherever their dials are, on both channels and in the mono layout.
        // That the chain without them is the amp alone is tested above
        let input = power_chords(SAMPLE_RATE, 0.3);
        for amp in Amp::ALL {
            let plain = everything_on(amp, 0.6);
            let off = AmpSettings {
                delay: DelaySettings {
                    on: false,
                    ..delay_on(1000.0, 0.9, 1.0)
                },
                reverb: ReverbSettings {
                    on: false,
                    ..reverb_on(6.0, 1.0)
                },
                ..plain
            };
            let reference = run(&plain, &input, SAMPLE_RATE);
            let mut chain = new_chain(SAMPLE_RATE);
            let (left, right) = run_stereo_blocks(&mut chain, &off, &input, BLOCK);
            assert!(left == reference && right == reference, "{:?}", amp);
            assert!(run(&off, &input, SAMPLE_RATE) == reference, "{:?} in mono", amp);
            assert!(chain.is_idle());
        }
    }

    #[test]
    fn test_delay_makes_left_and_right_differ_around_the_same_amp() {
        // Turned down, so the safety clip stays out of it
        let input = power_chords(SAMPLE_RATE, 0.5);
        let plain = AmpSettings {
            out_level: 0.25,
            ..with_amp(Amp::Klar, 0.5)
        };
        let delayed = AmpSettings {
            delay: delay_on(100.0, 0.4, 0.5),
            ..plain
        };
        let dry = run(&plain, &input, SAMPLE_RATE);
        let (left, right) = run_stereo(&delayed, &input, SAMPLE_RATE);

        // Until the first repeat there is only the amp, as it is without the delay
        let first = (0.09 * SAMPLE_RATE) as usize;
        assert!(left[..first] == dry[..first] && right[..first] == dry[..first]);

        // From there on the repeats are added to it, and they are not the same on both sides
        let (repeats_left, repeats_right) = (difference(&left, &dry), difference(&right, &dry));
        for repeats in [&repeats_left, &repeats_right] {
            assert!(rms(&repeats[first..]) > 0.1 * rms(&dry), "Repeats at {}, amp at {}", rms(repeats), rms(&dry));
        }
        let alike = correlation(&repeats_left[first..], &repeats_right[first..]);
        assert!(alike < 0.9, "Left and right repeats correlate by {}", alike);
        assert!(rms(&difference(&left, &right)) > 0.05 * rms(&dry));
    }

    #[test]
    fn test_effects_add_no_latency() {
        // The amp's own signal comes through as without them, to the sample: bit for bit
        // until the first of the tail and the first repeat arrive
        let input = power_chords(SAMPLE_RATE, 0.1);
        let plain = everything_on(Amp::Brol, 0.5);
        let effects = AmpSettings {
            delay: delay_on(20.0, 0.9, 1.0),
            reverb: reverb_on(6.0, 1.0),
            ..plain
        };
        let dry = run(&plain, &input, SAMPLE_RATE);
        let (left, right) = run_stereo(&effects, &input, SAMPLE_RATE);
        let before = (0.014 * SAMPLE_RATE) as usize;
        assert!(left[..before] == dry[..before] && right[..before] == dry[..before]);
        assert!(left[before..] != dry[before..]);

        for (amp, sample_rate) in [(Amp::Klar, 48000.0), (Amp::Brol, 48000.0), (Amp::Torden, 48000.0), (Amp::Torden, 44100.0)] {
            let plain = everything_on(amp, 0.0);
            let effects = AmpSettings {
                delay: delay_on(20.0, 0.9, 1.0),
                reverb: reverb_on(6.0, 1.0),
                ..plain
            };
            let (without, with) = (latency_samples_with(&plain, sample_rate), latency_samples_with(&effects, sample_rate));
            assert_eq!(with, without, "{:?} at {} Hz", amp, sample_rate);
        }
    }

    /// Plays `input` block by block and tells after which blocks the chain was idle
    fn run_watching(chain: &mut AmpChain, settings: &AmpSettings, input: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<bool>) {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        let mut idle = Vec::new();
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            chain.process(settings, l, Some(r));
            idle.push(chain.is_idle());
        }
        (left, right, idle)
    }

    #[test]
    fn test_reverb_tail_rings_on_after_the_input_and_ends() {
        let playing = 192 * BLOCK;
        let mut input = power_chords(SAMPLE_RATE, 0.3)[..playing].to_vec();
        input.resize(playing + 72_000, 0.0);
        let plain = AmpSettings {
            out_level: 0.25,
            ..with_amp(Amp::Klar, 0.5)
        };
        let settings = AmpSettings {
            reverb: reverb_on(0.3, 0.5),
            ..plain
        };
        let dry = run(&plain, &input, SAMPLE_RATE);
        let mut chain = new_chain(SAMPLE_RATE);
        assert!(chain.is_idle());
        let (left, right, idle) = run_watching(&mut chain, &settings, &input);

        // A tenth of a second after the last note: nothing of the amp, a tail on both sides
        let after = playing + 4800..playing + 9600;
        assert!(peak(&dry[after.clone()]) < 1e-6, "The amp alone is still at {}", peak(&dry[after.clone()]));
        assert!(rms(&left[after.clone()]) > 1e-4 && rms(&right[after.clone()]) > 1e-4, "No tail: {}", rms(&left[after.clone()]));
        assert!(left[after.clone()] != right[after.clone()]);
        assert!(!idle[playing / BLOCK - 1] && !idle[(playing + 4800) / BLOCK]);

        // And it ends: idle within a second and a half, and from the block after the one it
        // ended in there is nothing but the amp, which is silent
        let ended = idle.iter().rposition(|idle| !idle).map_or(0, |last| (last + 2) * BLOCK);
        assert!(ended < playing + 72_000, "Never idle");
        assert!(ended > playing && ended < playing + (1.5 * SAMPLE_RATE) as usize, "Idle after {} samples", ended - playing);
        assert!(left[ended..] == dry[ended..] && right[ended..] == dry[ended..]);
        assert!(peak(&dry[ended..]) < 1e-6);
    }

    #[test]
    fn test_chain_is_idle_without_effects_and_once_the_repeats_have_ended() {
        let playing = 96 * BLOCK;
        let mut input = power_chords(SAMPLE_RATE, 0.2)[..playing].to_vec();
        input.resize(playing + 84_000, 0.0);

        // Without effects there is never a tail to wait for
        let plain = with_amp(Amp::Klar, 0.5);
        let (_, _, idle) = run_watching(&mut new_chain(SAMPLE_RATE), &plain, &input[..2 * playing]);
        assert!(idle.iter().all(|&idle| idle));

        // The delay is idle as soon as its last repeats have fallen silent and its read
        // positions, 20 ms back, have nothing but silence ahead of them: within half a
        // second here. From the block after that there is nothing but the amp
        let settings = AmpSettings {
            delay: delay_on(20.0, 0.3, 0.5),
            ..plain
        };
        let dry = run(&plain, &input, SAMPLE_RATE);
        let (left, right, idle) = run_watching(&mut new_chain(SAMPLE_RATE), &settings, &input);
        assert!(idle[..(playing + 2400) / BLOCK].iter().all(|&idle| !idle));
        let ended = idle.iter().rposition(|idle| !idle).map_or(0, |last| (last + 2) * BLOCK);
        assert!(ended < playing + 84_000, "Never idle");
        assert!(ended < playing + (0.5 * SAMPLE_RATE) as usize, "Idle after {} samples", ended - playing);
        assert!(left[ended..] == dry[ended..] && right[ended..] == dry[ended..]);
        assert!(peak(&dry[ended..]) < 1e-6);
    }

    #[test]
    fn test_switching_amps_does_not_cut_the_tail() {
        // The last chord has stopped and its repeats and tail ring on when another amp is
        // picked. The amps themselves are silent by then, so the tail is all there is
        let playing = 128 * BLOCK;
        let at = playing + 40 * BLOCK;
        let mut input = power_chords(SAMPLE_RATE, 0.2)[..playing].to_vec();
        input.resize(playing + 24_000, 0.0);
        let effects = |amp: Amp| AmpSettings {
            delay: delay_on(100.0, 0.5, 0.5),
            reverb: reverb_on(1.5, 0.5),
            out_level: 0.25,
            ..with_amp(amp, 0.5)
        };

        for (from, to) in [(Amp::Torden, Amp::Klar), (Amp::Klar, Amp::Brol)] {
            let (stayed, _) = run_stereo(&effects(from), &input, SAMPLE_RATE);
            let mut chain = new_chain(SAMPLE_RATE);
            let (mut left, mut right) = (input.clone(), input.clone());
            for (index, (l, r)) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)).enumerate() {
                chain.process(&effects(if index * BLOCK < at { from } else { to }), l, Some(r));
            }
            let tail = rms(&stayed[at..]);
            assert!(tail > 1e-3, "{:?}: no tail to keep, {}", from, tail);
            let off = largest_difference(&left[at..], &stayed[at..]);
            assert!(off < 0.01 * tail, "{:?} to {:?}: tail off by {} at a level of {}", from, to, off, tail);
            assert!(rms(&right[at..]) > 0.5 * tail);
        }
    }

    #[test]
    fn test_switching_amps_with_effects_on_does_not_click() {
        let at = 300 * BLOCK;
        let input = sine(220.0, 0.178, SAMPLE_RATE, 36_000);
        for (from, to) in [(Amp::Klar, Amp::Torden), (Amp::Torden, Amp::Brol)] {
            let effects = |amp: Amp| with_effects(with_amp(amp, 0.5));
            let mut chain = new_chain(SAMPLE_RATE);
            let (mut left, mut right) = (input.clone(), input.clone());
            for (index, (l, r)) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)).enumerate() {
                chain.process(&effects(if index * BLOCK < at { from } else { to }), l, Some(r));
            }
            for channel in [&left, &right] {
                let ratio = step_ratio(channel, at);
                assert!(ratio < 1.2, "{:?} to {:?}: step {} times its own", from, to, ratio);
            }
        }
    }

    #[test]
    fn test_bypass_with_effects_does_not_click_and_comes_back_clean() {
        let input = sine(220.0, 0.3, SAMPLE_RATE, 28_800);
        let playing = with_effects(with_amp(Amp::Klar, 0.5));
        let bypassed = AmpSettings {
            bypass: true,
            ..playing
        };
        let (off, on) = (150 * BLOCK, 300 * BLOCK);
        let fade = (BYPASS_FADE_MS * 0.001 * SAMPLE_RATE) as usize + 1;

        // Playing, bypassed, playing again
        let mut chain = new_chain(SAMPLE_RATE);
        let (mut left, mut right) = (input.clone(), input.clone());
        for (index, (l, r)) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)).enumerate() {
            let settings = if (150..300).contains(&index) { &bypassed } else { &playing };
            chain.process(settings, l, Some(r));
            // Once the fade is over nothing is kept ringing behind the bypass
            assert!(!(160..300).contains(&index) || chain.is_idle());
        }
        assert!(left != right);
        // What comes back is a chain that starts there: nothing of what the effects held
        let (fresh_left, fresh_right) = run_stereo(&playing, &input[on..], SAMPLE_RATE);

        for (channel, fresh) in [(&left, &fresh_left), (&right, &fresh_right)] {
            assert!(channel[off + fade..on] == input[off + fade..on]);
            let own_step = largest_step(&channel[4800..off]).max(largest_step(&input));
            let step_off = largest_step(&channel[off - 1..off + fade + 1]);
            let step_on = largest_step(&channel[on - 1..on + 2400]);
            assert!(step_off < own_step * 1.2, "Step of {} into bypass, {} while playing", step_off, own_step);
            assert!(step_on < own_step * 1.2, "Step of {} out of bypass, {} while playing", step_on, own_step);
            let stale = largest_difference(&channel[on + fade..], &fresh[fade..]);
            assert!(stale < 1e-6, "Off by {} from a chain that starts at the switch", stale);
        }

        // And with nothing played after a short bypass, nothing is heard: the long tail and
        // the repeats of the chords before it are gone
        let long = with_effects_at_most(with_amp(Amp::Klar, 0.5));
        let mut chain = new_chain(SAMPLE_RATE);
        run_stereo_blocks(&mut chain, &long, &power_chords(SAMPLE_RATE, 0.2), BLOCK);
        assert!(!chain.is_idle());
        let bypassed = AmpSettings { bypass: true, ..long };
        run_stereo_blocks(&mut chain, &bypassed, &vec![0.0; 16 * BLOCK], BLOCK);
        assert!(chain.is_idle());
        let (left, right) = run_stereo_blocks(&mut chain, &long, &vec![0.0; 9600], BLOCK);
        assert!(peak(&left) < 1e-9 && peak(&right) < 1e-9, "Left over: {} and {}", peak(&left), peak(&right));
        assert!(chain.is_idle());
    }

    #[test]
    fn test_mono_layout_is_the_left_channel() {
        let input = power_chords(SAMPLE_RATE, 0.3);
        for amp in [Amp::Klar, Amp::Torden] {
            let settings = with_effects(everything_on(amp, 0.6));
            let mono = run(&settings, &input, SAMPLE_RATE);
            let (left, right) = run_stereo(&settings, &input, SAMPLE_RATE);
            assert!(mono == left, "{:?}", amp);
            assert!(left != right, "{:?}", amp);
        }
    }

    #[test]
    fn test_output_is_bounded_with_effects_at_extremes_at_all_sample_rates() {
        // Full-scale input, every dial up, the shortest delay at full feedback so that as
        // many repeats as can be pile up, the longest reverb, twice the output level
        for sample_rate in [44100.0, 48000.0, 88200.0, 96000.0, 192000.0] {
            let len = (sample_rate * 0.2) as usize;
            let mut noise = Noise::new(3);
            let input: Vec<f32> = sine(97.0, 1.0, sample_rate, len).iter().map(|s| s + noise.next()).collect();
            for (amp, time_ms) in [(Amp::Klar, 20.0), (Amp::Torden, 1000.0)] {
                let settings = AmpSettings {
                    in_gain_db: 24.0,
                    drive_on: true,
                    drive_gain: 1.0,
                    drive_level: 1.0,
                    delay: delay_on(time_ms, 0.9, 1.0),
                    reverb: reverb_on(6.0, 1.0),
                    ..with_all_dials(amp, 1.0, 2.0)
                };
                let (left, right) = run_stereo(&settings, &input, sample_rate);
                for channel in [&left, &right] {
                    assert!(channel.iter().all(|s| s.is_finite()), "{:?} not finite at {} Hz", amp, sample_rate);
                    assert!(peak(channel) <= 1.0, "{:?} peak {} at {} Hz", amp, peak(channel), sample_rate);
                    assert!(rms(channel) > 0.05);
                }
                let mono = run(&settings, &input, sample_rate);
                assert!(mono.iter().all(|s| s.is_finite()) && peak(&mono) <= 1.0);
            }
        }
    }

    #[test]
    fn test_output_with_effects_does_not_depend_on_block_size() {
        let input = power_chords(SAMPLE_RATE, 0.2);
        let settings = with_effects(AmpSettings {
            gate_thresh_db: -30.0,
            gate_release_ms: 20.0,
            ..everything_on(Amp::Brol, 0.7)
        });
        let reference = run_stereo_blocks(&mut new_chain(SAMPLE_RATE), &settings, &input, input.len());
        assert!(reference.0 != reference.1);
        for block in [1, 7, 32, 64, 1000] {
            let output = run_stereo_blocks(&mut new_chain(SAMPLE_RATE), &settings, &input, block);
            let off = largest_difference(&reference.0, &output.0).max(largest_difference(&reference.1, &output.1));
            assert!(off < 1e-5, "In blocks of {}: off by {}", block, off);
            let mono = run_blocks(&mut new_chain(SAMPLE_RATE), &settings, &input, block);
            assert!(largest_difference(&reference.0, &mono) < 1e-5, "Mono in blocks of {}", block);
        }
    }

    #[test]
    fn test_effects_tails_are_not_slower_than_playing() {
        // The repeats and the tail fall all the way to nothing in this time. Numbers that
        // small must not slow the processor down on the way
        let mut chain = new_chain(SAMPLE_RATE);
        let settings = AmpSettings {
            delay: delay_on(20.0, 0.5, 1.0),
            reverb: reverb_on(0.3, 1.0),
            ..with_amp(Amp::Klar, 0.5)
        };
        let playing = block_time_us(&mut chain, &settings, &power_chords(SAMPLE_RATE, 0.25));
        assert!(!chain.is_idle());
        // A third of a second in which both still ring, then the rest of the way down
        let ringing = block_time_us(&mut chain, &settings, &vec![0.0; 14_400]);
        assert!(!chain.is_idle());
        run_blocks(&mut chain, &settings, &vec![0.0; 48_000], BLOCK);
        assert!(chain.is_idle());
        let silent = block_time_us(&mut chain, &settings, &vec![0.0; 12_000]);
        assert!(ringing < playing * SLOWED_DOWN, "{:.1} us per block playing, {:.1} us while the tail ends", playing, ringing);
        assert!(silent < playing * SLOWED_DOWN, "{:.1} us per block playing, {:.1} us silent", playing, silent);
    }

    fn tuning(base: AmpSettings) -> AmpSettings {
        AmpSettings {
            tuner_on: true,
            ..base
        }
    }

    /// A low E string, quieter than a guitar gives it
    fn quiet_low_e(seconds: f32) -> Vec<f32> {
        let mut note = tuner_string(tuner_note_hz(40), SAMPLE_RATE, seconds);
        note.iter_mut().for_each(|sample| *sample *= 0.04);
        note
    }

    fn cents_off(hz: f32, expected_hz: f32) -> f32 {
        1200.0 * (hz / expected_hz).log2()
    }

    #[test]
    fn test_tuner_mutes_without_a_click_and_gives_the_amp_back() {
        let input = sine(220.0, 0.3, SAMPLE_RATE, 28_800);
        for playing in [AmpSettings::default(), with_default_effects(with_amp(Amp::Torden, 0.8))] {
            let mut chain = new_chain(SAMPLE_RATE);
            let (mut left, mut right) = (input.clone(), input.clone());

            // Playing, tuning, playing again
            for (index, (l, r)) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)).enumerate() {
                let settings = if (150..300).contains(&index) { tuning(playing) } else { playing };
                chain.process(&settings, l, Some(r));
            }

            let (on, off) = (150 * BLOCK, 300 * BLOCK);
            let fade = (TUNER_FADE_MS * 0.001 * SAMPLE_RATE) as usize + 1;
            for channel in [&left, &right] {
                // Not quiet: silent, whatever still rings in the delay and the reverb
                assert!(channel[on + fade..off].iter().all(|sample| *sample == 0.0), "{:?} is heard while tuning", playing.amp);
                assert!(rms(&channel[off + 4800..]) > 0.01);

                let own_step = largest_step(&channel[4800..on]);
                let step_on = largest_step(&channel[on - 1..on + fade + 1]);
                let step_off = largest_step(&channel[off - 1..off + 2400]);
                assert!(step_on < own_step * 1.2, "{:?}: step of {step_on} into tuning, {own_step} while playing", playing.amp);
                assert!(step_off < own_step * 1.2, "{:?}: step of {step_off} out of tuning, {own_step} while playing", playing.amp);
            }
        }
    }

    #[test]
    fn test_what_is_played_while_tuning_is_not_in_the_effects_afterwards() {
        // The amp gets silence while the tuner is on: the strings that were tuned are not
        // in the repeats and the tail when it is switched off
        let settings = with_effects_at_most(AmpSettings::default());
        let mut chain = new_chain(SAMPLE_RATE);
        let mut input = power_chords(SAMPLE_RATE, 1.0);
        input.resize(96_000, 0.0);
        let mut output = input.clone();
        let (tuned, after) = output.split_at_mut(48_000);
        for block in tuned.chunks_mut(BLOCK) {
            chain.process(&tuning(settings), block, None);
        }
        for block in after.chunks_mut(BLOCK) {
            chain.process(&settings, block, None);
        }
        assert!(peak(&output) < 1e-9, "Peak of {}", peak(&output));
        assert!(chain.is_idle());
    }

    #[test]
    fn test_tuner_hears_the_input_whatever_the_amp_does() {
        let note = quiet_low_e(0.5);
        let expected = tuner_note_hz(40);
        let far_down = AmpSettings {
            in_gain_db: -24.0,
            gate_thresh_db: -20.0,
            out_level: 0.0,
            ..AmpSettings::default()
        };
        let bypassed = AmpSettings {
            bypass: true,
            ..AmpSettings::default()
        };
        for settings in [AmpSettings::default(), far_down, with_amp(Amp::Torden, 1.0), bypassed] {
            let mut chain = new_chain(SAMPLE_RATE);
            let reading = chain.tuner_reading();
            let (mut left, mut right) = (note.clone(), note.clone());
            for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
                chain.process(&tuning(settings), l, Some(r));
            }
            let heard = reading.hz();
            assert!(heard.is_some_and(|hz| cents_off(hz, expected).abs() < 3.0), "{heard:?} with {settings:?}");

            // Bypass passes the input on while the tuner listens. Anything else is silent
            if settings.bypass {
                assert_eq!(left, note);
                assert_eq!(right, note);
            } else {
                assert_eq!(peak(&left).max(peak(&right)), 0.0);
            }

            // Switched off there is nothing to show, and nothing is listened to
            let mut more = note.clone();
            for block in more.chunks_mut(BLOCK) {
                chain.process(&settings, block, None);
            }
            assert_eq!(reading.hz(), None);
        }

        // Never switched on: no reading
        let mut chain = new_chain(SAMPLE_RATE);
        run_blocks(&mut chain, &AmpSettings::default(), &note, BLOCK);
        assert_eq!(chain.tuner_reading().hz(), None);
    }

    #[test]
    fn test_tuner_starts_over_after_a_reset() {
        let mut chain = new_chain(SAMPLE_RATE);
        let reading = chain.tuner_reading();
        let settings = tuning(AmpSettings::default());
        run_blocks(&mut chain, &settings, &quiet_low_e(0.5), BLOCK);
        assert!(reading.hz().is_some());
        chain.reset();
        assert_eq!(reading.hz(), None);
        run_blocks(&mut chain, &settings, &vec![0.0; 4800], BLOCK);
        assert_eq!(reading.hz(), None);
        run_blocks(&mut chain, &settings, &quiet_low_e(0.5), BLOCK);
        assert!(reading.hz().is_some_and(|hz| cents_off(hz, tuner_note_hz(40)).abs() < 3.0));
    }

    #[test]
    fn test_tuning_does_not_depend_on_block_size() {
        // The switch is read with the dials, and the tuner counts its work in samples: the
        // same output and the same readings however the host cuts the stream
        let input = quiet_low_e(0.6);
        let playing = with_default_effects(with_amp(Amp::Klar, 0.5));
        let (on, off) = (4800, 24_000);
        let run = |block: usize| {
            let mut chain = new_chain(SAMPLE_RATE);
            let reading = chain.tuner_reading();
            let mut output = input.clone();
            let mut heard = Vec::new();
            for (range, settings) in [(0..on, playing), (on..off, tuning(playing)), (off..input.len(), playing)] {
                for piece in output[range].chunks_mut(block) {
                    chain.process(&settings, piece, None);
                }
                heard.push(reading.hz().map(f32::to_bits));
            }
            (output, heard)
        };

        let (reference, heard) = run(input.len());
        assert!(heard[1].is_some() && heard[0].is_none() && heard[2].is_none());
        for block in [1, 7, 32, 64, 1000] {
            let (output, heard_in_blocks) = run(block);
            assert!(largest_difference(&reference, &output) < 1e-5, "Blocks of {block}");
            assert_eq!(heard_in_blocks, heard, "Blocks of {block}");
        }
    }

    /// A built-in cabinet as a file of it comes out when it is loaded at `sample_rate`
    fn file_taps(amp: Amp, sample_rate: f32) -> Vec<f32> {
        let recording = Recording {
            samples: whole_response(amp, 48000.0, 0.1, 0.5),
            sample_rate: 48000,
        };
        user_cab::prepare(&recording, sample_rate).unwrap()
    }

    /// A chain that finds a cabinet on its stage before its first block: an impulse
    /// response of the player's own, or no taps for the amp's own
    fn chain_with_cab(taps: &[f32], sample_rate: f32) -> AmpChain {
        let chain = new_chain(sample_rate);
        chain.cab_stage().offer(taps, sample_rate);
        chain
    }

    /// Plays `input` and leaves a cabinet on the stage in front of the block at sample `at`
    fn run_cab_change(chain: &mut AmpChain, settings: &AmpSettings, taps: &[f32], input: &[f32], at: usize) -> Vec<f32> {
        let mut output = input.to_vec();
        let (before, after) = output.split_at_mut(at);
        for block in before.chunks_mut(BLOCK) {
            chain.process(settings, block, None);
        }
        chain.cab_stage().offer(taps, chain.sample_rate());
        for block in after.chunks_mut(BLOCK) {
            chain.process(settings, block, None);
        }
        output
    }

    /// Samples from an impulse until the output first reaches half of its peak, through
    /// a cabinet of the player's own
    fn latency_samples_through(taps: &[f32], amp: Amp, sample_rate: f32) -> usize {
        let mut impulse = vec![0.0; 2048];
        impulse[0] = 0.05;
        let output = run_blocks(&mut chain_with_cab(taps, sample_rate), &with_amp(amp, 0.0), &impulse, BLOCK);
        let top = peak(&output);
        output.iter().position(|s| s.abs() >= 0.5 * top).unwrap()
    }

    #[test]
    fn test_own_cabinet_left_on_the_stage_changes_nothing() {
        // Bit for bit: before the first block, while playing, and with a cabinet that was
        // made for another sample rate, which is never played
        let input = power_chords(SAMPLE_RATE, 0.5);
        for amp in Amp::ALL {
            let settings = with_effects(everything_on(amp, 0.6));
            let reference = run(&settings, &input, SAMPLE_RATE);
            let mut chain = chain_with_cab(&[], SAMPLE_RATE);
            let mut output = input.clone();
            for (index, block) in output.chunks_mut(BLOCK).enumerate() {
                match index {
                    100 => chain.cab_stage().offer(&[], SAMPLE_RATE),
                    200 => chain.cab_stage().offer(&[0.5, 0.5], 44100.0),
                    _ => {}
                }
                chain.process(&settings, block, None);
            }
            assert!(output == reference, "{:?}", amp);
            assert!(!chain.user_cab && !chain.cab_stage.is_ready());
        }
    }

    #[test]
    fn test_missing_and_unusable_files_leave_the_amps_own_cabinet() {
        let dir = test_dir("chain");
        std::fs::write(dir.join("bad.wav"), b"not a recording").unwrap();
        write_ir_wav(&dir.join("good.wav"), &whole_response(Amp::Klar, 48000.0, 0.1, 0.5), 2, 24, false, 48000);

        let input = power_chords(SAMPLE_RATE, 0.25);
        let settings = with_amp(Amp::Torden, 0.5);
        let reference = run(&settings, &input, SAMPLE_RATE);
        // The same played a second time, as below
        let mut chain = new_chain(SAMPLE_RATE);
        run_blocks(&mut chain, &settings, &input, BLOCK);
        let again = run_blocks(&mut chain, &settings, &input, BLOCK);
        for (name, status) in [("gone.wav", CabStatus::Missing), ("bad.wav", CabStatus::Bad), ("", CabStatus::Fine)] {
            let mut chain = new_chain(SAMPLE_RATE);
            let loader = CabLoader::new(chain.cab_stage(), SAMPLE_RATE);
            loader.load_from(&Mutex::new(name.to_owned()), Some(&dir));
            assert!(run_blocks(&mut chain, &settings, &input, BLOCK) == reference, "{:?}", name);
            assert_eq!(loader.status(), status);

            // Also when it comes while one of the player's own plays: back to the amp's own
            let mut chain = chain_with_cab(&file_taps(Amp::Brol, SAMPLE_RATE), SAMPLE_RATE);
            run_blocks(&mut chain, &settings, &input, BLOCK);
            assert!(chain.user_cab);
            loader_for(&chain).load_from(&Mutex::new(name.to_owned()), Some(&dir));
            let output = run_blocks(&mut chain, &settings, &input, BLOCK);
            assert!(!chain.user_cab, "{:?}", name);
            assert!(largest_difference(&output[4800..], &again[4800..]) < 1e-4, "{:?}", name);
        }

        // And a file that is there plays: Torden through the cabinet of Klar
        let mut chain = new_chain(SAMPLE_RATE);
        loader_for(&chain).load_from(&Mutex::new("good.wav".to_owned()), Some(&dir));
        let output = run_blocks(&mut chain, &settings, &input, BLOCK);
        assert!(chain.user_cab);
        let through_file = run_blocks(&mut chain_with_cab(&file_taps(Amp::Klar, SAMPLE_RATE), SAMPLE_RATE), &settings, &input, BLOCK);
        assert!(largest_difference(&output, &through_file) < 1e-3 * peak(&output));
        assert!(largest_difference(&output, &reference) > 0.05 * peak(&output));
    }

    fn loader_for(chain: &AmpChain) -> CabLoader {
        CabLoader::new(chain.cab_stage(), chain.sample_rate())
    }

    #[test]
    fn test_a_cabinet_of_one_sample_is_the_cabinet_switched_off() {
        let input = power_chords(SAMPLE_RATE, 0.5);
        for amp in Amp::ALL {
            // Quiet enough for the safety clip to leave both alone
            let on = AmpSettings {
                out_level: 0.2,
                ..with_amp(amp, 0.6)
            };
            let off = AmpSettings { cab_on: false, ..on };
            let through = run_blocks(&mut chain_with_cab(&[1.0], SAMPLE_RATE), &on, &input, BLOCK);
            let without = run(&off, &input, SAMPLE_RATE);
            assert!(peak(&through) < OUTPUT_CLIP_KNEE && peak(&through) > 0.01);
            // The same but for the level the switched-off cabinet is turned down by
            let trim = db_to_gain(-CAB_OFF_DB);
            let difference = through.iter().zip(&without).map(|(a, b)| (a - b * trim).abs()).fold(0.0, f32::max);
            assert!(difference < 1e-5 * peak(&through), "{:?}: off by {}", amp, difference);

            // Switched off, the player's cabinet is as much out of the way as the amp's own
            let user_off = run_blocks(&mut chain_with_cab(&file_taps(amp, SAMPLE_RATE), SAMPLE_RATE), &off, &input, BLOCK);
            assert!(user_off == without, "{:?}", amp);
        }
    }

    #[test]
    fn test_changing_cabinets_does_not_click() {
        let at = 300 * BLOCK;
        // A tone through every amp, and chords through one
        let tone = sine(220.0, 0.178, SAMPLE_RATE, 36_000);
        let chords = power_chords(SAMPLE_RATE, 0.75);
        for (input, amps) in [(&tone, &Amp::ALL[..]), (&chords, &[Amp::Brol][..])] {
            for &amp in amps {
                let settings = with_amp(amp, 0.5);
                let other = file_taps(Amp::ALL[(amp.index() + 1) % 3], SAMPLE_RATE);
                let third = file_taps(Amp::ALL[(amp.index() + 2) % 3], SAMPLE_RATE);
                // From the amp's own to a file, from file to file, and back to the amp's own
                for (before, after) in [(&[][..], &other[..]), (&other, &third), (&third, &[])] {
                    let mut chain = chain_with_cab(before, SAMPLE_RATE);
                    let output = run_cab_change(&mut chain, &settings, after, input, at);
                    let ratio = step_ratio(&output, at);
                    assert!(ratio < 1.3, "{:?}, {} to {} taps: step {} times its own", amp, before.len(), after.len(), ratio);
                    // Ends up as if it had been that cabinet all along
                    let reference = run_blocks(&mut chain_with_cab(after, SAMPLE_RATE), &settings, input, BLOCK);
                    let settled = at + 4800;
                    let difference = largest_difference(&output[settled..], &reference[settled..]);
                    assert!(difference < 1e-4, "{:?}, {} to {} taps: off by {}", amp, before.len(), after.len(), difference);
                }
            }
        }
    }

    #[test]
    fn test_cabinets_arriving_during_a_crossfade_wait_and_the_last_one_stays() {
        let input = power_chords(SAMPLE_RATE, 1.0);
        let settings = with_amp(Amp::Brol, 0.5);
        let [first, second, third] = Amp::ALL.map(|amp| file_taps(amp, SAMPLE_RATE));
        let mut chain = new_chain(SAMPLE_RATE);
        let stage = chain.cab_stage();
        let before = buffer_layout(&chain);

        let piece = 16;
        let mut output = input.clone();
        for (index, block) in output.chunks_mut(piece).enumerate() {
            match index {
                600 => stage.offer(&first, SAMPLE_RATE),
                // The crossfade to the first takes 10 ms. The second comes 1 ms into it and
                // waits; the third comes 1 ms later and takes its place before it ever played
                603 => {
                    assert!(chain.cabinet.is_swapping() && !stage.is_ready());
                    stage.offer(&second, SAMPLE_RATE);
                }
                606 => {
                    assert!(chain.cabinet.is_swapping() && stage.is_ready());
                    stage.offer(&third, SAMPLE_RATE);
                }
                _ => {}
            }
            chain.process(&settings, block, None);
            // Left alone until the crossfade is over, then taken at once
            let fading = (600 * piece..600 * piece + chain.cabinet.swap_len()).contains(&((index + 1) * piece));
            if index >= 603 && fading {
                assert!(stage.is_ready(), "Taken during the crossfade, in piece {}", index);
            }
        }
        assert!(!stage.is_ready() && chain.user_cab);
        assert_eq!(buffer_layout(&chain), before);

        let at = 600 * piece;
        let ratio = step_ratio(&output, at);
        assert!(ratio < 1.3, "Step {} times its own", ratio);
        let reference = run_blocks(&mut chain_with_cab(&third, SAMPLE_RATE), &settings, &input, piece);
        let settled = at + 9600;
        let difference = largest_difference(&output[settled..], &reference[settled..]);
        assert!(difference < 1e-4, "Off by {}", difference);
    }

    #[test]
    fn test_players_cabinet_stays_when_another_amp_is_picked() {
        let input = power_chords(SAMPLE_RATE, 1.5);
        let taps = file_taps(Amp::Klar, SAMPLE_RATE);
        let at = 300 * BLOCK;
        let settled = input.len() - 24_000;
        for (from, to) in [(Amp::Klar, Amp::Torden), (Amp::Torden, Amp::Brol)] {
            let mut chain = chain_with_cab(&taps, SAMPLE_RATE);
            let switched = run_switch(&mut chain, from, to, 0.5, &input, at);
            assert!(chain.user_cab);
            assert!(step_ratio(&switched, at) < 1.2, "{:?} to {:?}: step {}", from, to, step_ratio(&switched, at));

            // The new amp through the player's cabinet, not through its own
            let through = run_blocks(&mut chain_with_cab(&taps, SAMPLE_RATE), &with_amp(to, 0.5), &input, BLOCK);
            let own = run(&with_amp(to, 0.5), &input, SAMPLE_RATE);
            let level = rms(&through[settled..]);
            let difference = largest_difference(&switched[settled..], &through[settled..]);
            assert!(difference < 0.01 * level, "{:?} to {:?}: off by {} at a level of {}", from, to, difference, level);
            assert!(largest_difference(&switched[settled..], &own[settled..]) > 0.1 * level);

            // The same while bypassed, where nothing is faded
            let mut chain = chain_with_cab(&taps, SAMPLE_RATE);
            let bypassed = |amp: Amp| AmpSettings {
                bypass: true,
                ..with_amp(amp, 0.5)
            };
            run_blocks(&mut chain, &with_amp(from, 0.5), &input[..4800], BLOCK);
            run_blocks(&mut chain, &bypassed(from), &input[..4800], BLOCK);
            run_blocks(&mut chain, &bypassed(to), &input[..4800], BLOCK);
            assert!(chain.user_cab && chain.amp == to);
            let back = run_blocks(&mut chain, &with_amp(to, 0.5), &input, BLOCK);
            let difference = largest_difference(&back[settled..], &through[settled..]);
            assert!(difference < 0.01 * level, "{:?} to {:?} bypassed: off by {}", from, to, difference);
        }
    }

    #[test]
    fn test_no_setting_changes_the_choice_of_cabinet() {
        // A preset is a set of settings, and the cabinet is not among them
        let input = power_chords(SAMPLE_RATE, 0.1);
        let mut chain = chain_with_cab(&file_taps(Amp::Torden, SAMPLE_RATE), SAMPLE_RATE);
        let mut settings = vec![AmpSettings::default()];
        for amp in Amp::ALL {
            settings.push(with_effects(everything_on(amp, 0.8)));
            settings.push(AmpSettings {
                cab_on: false,
                cab_mic: 0.0,
                cab_res: 1.0,
                tuner_on: true,
                ..with_all_dials(amp, 1.0, 0.5)
            });
        }
        for settings in &settings {
            run_blocks(&mut chain, settings, &input, BLOCK);
            assert!(chain.user_cab);
        }
        chain.reset();
        run_blocks(&mut chain, &settings[0], &input, BLOCK);
        assert!(chain.user_cab);
    }

    #[test]
    fn test_cabinet_dials_work_on_the_players_cabinet() {
        // Through a cabinet of one sample what is heard of the dials is the dials alone:
        // they sit behind the amp, so they change its sound by what their filters do
        for amp in Amp::ALL {
            let cab = &amp.model().cab;
            let level = |mic: f32, res: f32, freq_hz: f32| {
                let settings = AmpSettings {
                    cab_mic: mic,
                    cab_res: res,
                    gate_on: false,
                    ..with_amp(amp, 0.0)
                };
                let input = sine(freq_hz, 0.02, SAMPLE_RATE, 24_000);
                let output = run_blocks(&mut chain_with_cab(&[1.0], SAMPLE_RATE), &settings, &input, BLOCK);
                to_db(level_at(&output[12_000..], SAMPLE_RATE, freq_hz))
            };
            let bright = level(1.0, 0.5, 5000.0) - level(0.5, 0.5, 5000.0);
            let dark = level(0.0, 0.5, 4000.0) - level(0.5, 0.5, 4000.0);
            let thump = level(0.5, 1.0, cab.resonance_hz) - level(0.5, 0.5, cab.resonance_hz);
            assert!(bright > 4.0, "{:?}: Mic 10 adds {:.1} dB at 5 kHz", amp, bright);
            assert!(dark < -4.0, "{:?}: Mic 0 adds {:.1} dB at 4 kHz", amp, dark);
            assert!((thump - 6.0).abs() < 0.5, "{:?}: Resonance 10 adds {:.1} dB at {} Hz", amp, thump, cab.resonance_hz);
        }
    }

    #[test]
    fn test_another_sample_rate_needs_the_cabinet_again() {
        let settings = with_amp(Amp::Brol, 0.5);
        let mut chain = chain_with_cab(&file_taps(Amp::Klar, 48000.0), 48000.0);
        run_blocks(&mut chain, &settings, &power_chords(48000.0, 0.05), BLOCK);
        assert!(chain.user_cab);

        // What was made for 48 kHz is not played at 96 kHz, whether it was taken or still waits
        chain.cab_stage().offer(&file_taps(Amp::Torden, 48000.0), 48000.0);
        chain.set_sample_rate(96000.0);
        let input = power_chords(96000.0, 0.25);
        let output = run_blocks(&mut chain, &settings, &input, BLOCK);
        assert!(!chain.user_cab && !chain.cab_stage.is_ready());
        assert!(output == run(&settings, &input, 96000.0));

        // Left again for the new rate, as the plugin does when the rate is set, it plays
        // from the first block on
        let taps = file_taps(Amp::Klar, 96000.0);
        chain.set_sample_rate(96000.0);
        chain.cab_stage().offer(&taps, 96000.0);
        let output = run_blocks(&mut chain, &settings, &input, BLOCK);
        assert!(chain.user_cab);
        assert!(output == run_blocks(&mut chain_with_cab(&taps, 96000.0), &settings, &input, BLOCK));
    }

    #[test]
    fn test_taking_cabinets_does_not_allocate() {
        for sample_rate in [44100.0, 192000.0] {
            let mut chain = new_chain(sample_rate);
            let stage = chain.cab_stage();
            let before = buffer_layout(&chain);
            // Short, as long as the cabinet holds, longer than that, the amp's own, a file
            let cabs = [
                vec![1.0],
                vec![0.001; user_ir_len(sample_rate)],
                Vec::new(),
                vec![0.0005; MAX_USER_IR_LEN + 100],
                file_taps(Amp::Klar, sample_rate),
            ];
            let mut block = sine(220.0, 0.2, sample_rate, 4800);
            for (index, piece) in block.chunks_mut(BLOCK).enumerate() {
                // Some arrive during a crossfade, with other amps and Bypass in between
                if index % 4 == 0 {
                    stage.offer(&cabs[(index / 4) % cabs.len()], sample_rate);
                }
                let turn = index / 10;
                let settings = AmpSettings {
                    bypass: turn % 5 == 4,
                    cab_on: turn % 3 != 0,
                    ..with_amp(Amp::ALL[turn % 3], 0.5)
                };
                chain.process(&settings, piece, None);
                assert!(piece.iter().all(|sample| sample.is_finite() && sample.abs() <= 1.0));
            }
            assert_eq!(buffer_layout(&chain), before);
        }
    }

    #[test]
    fn test_players_cabinet_adds_no_latency() {
        for sample_rate in [44100.0, 48000.0, 96000.0] {
            for amp in Amp::ALL {
                let own = latency_samples(amp, sample_rate);
                // Nothing in the way of the amp: no later than through the amp's own cabinet
                let direct = latency_samples_through(&[1.0], amp, sample_rate);
                assert!(direct <= own, "{:?} at {} Hz: {} samples, {} through its own", amp, sample_rate, direct, own);
                // The amp's own cabinet from a file with 30 ms of silence in front
                let mut late = vec![0.0; 1440];
                late.extend(whole_response(amp, 48000.0, 0.1, 0.5));
                let recording = Recording {
                    samples: late,
                    sample_rate: 48000,
                };
                let through = latency_samples_through(&user_cab::prepare(&recording, sample_rate).unwrap(), amp, sample_rate);
                assert!(through <= own + 1, "{:?} at {} Hz: {} samples, {} through its own", amp, sample_rate, through, own);
            }
        }
    }

    /// Prints what a cabinet of the player's own costs at each length and sample rate, and
    /// the latency through one:
    ///   cargo test -p amp --release user_cab_report -- --ignored --nocapture
    /// which also runs `user_cab_report_response` in `user_cab.rs`: what loading does to a response
    #[test]
    #[ignore]
    fn user_cab_report_cost() {
        println!("Time per {}-sample block, stereo, everything on (Torden, Gain 10), both effects, and its", BLOCK);
        println!("share of real time: through the amp's own cabinet, and through one of the player's own");
        println!("of each length. The longest a file is cut to is {} ms", USER_IR_MS);
        println!("{:>11}{:>19}{:>19}{:>19}{:>19}{:>19}", "", "own", "10 ms", "20 ms", "30 ms", "40 ms");
        for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            let block_us = BLOCK as f64 / sample_rate as f64 * 1e6;
            let settings = with_default_effects(everything_on(Amp::Torden, 1.0));
            let chords = power_chords(sample_rate, 2.0);
            print!("{:>8} Hz", sample_rate);
            for ms in [0.0, 10.0, 20.0, 30.0, 40.0] {
                // Noise that dies away, levelled like a file: as loud as the amp's own
                let mut noise = Noise::new(5);
                let len = (ms * 0.001 * sample_rate) as usize;
                let response: Vec<f32> =
                    (0..len).map(|index| if index == 0 { 1.0 } else { noise.next() * (-6.0 * index as f32 / len as f32).exp() }).collect();
                let taps = if len == 0 { Vec::new() } else { user_cab::trim_and_level(&response, sample_rate).unwrap() };
                let mut chain = chain_with_cab(&taps, sample_rate);
                run_stereo_blocks(&mut chain, &settings, &chords, BLOCK);
                assert_eq!(chain.user_cab, len > 0);
                let time = (0..5).map(|_| block_time_stereo_us(&mut chain, &settings, &chords)).fold(f64::MAX, f64::min);
                print!("{:>6} {:>5.1} us{:>4.1}%", taps.len(), time, time / block_us * 100.0);
            }
            println!();
        }

        println!();
        println!("Latency in samples at Gain 0: through the amp's own cabinet, through the same cabinet");
        println!("from a 48 kHz file with 30 ms of silence in front, and through a cabinet of one sample");
        println!("{:<8}{:>20}{:>20}{:>20}{:>20}", "", 44100, 48000, 96000, 192000);
        for amp in Amp::ALL {
            print!("{:<8}", amp.model().name);
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                let mut late = vec![0.0; 1440];
                late.extend(whole_response(amp, 48000.0, 0.1, 0.5));
                let recording = Recording {
                    samples: late,
                    sample_rate: 48000,
                };
                let taps = user_cab::prepare(&recording, sample_rate).unwrap();
                print!(
                    "{:>10} /{:>3} /{:>3}",
                    latency_samples(amp, sample_rate),
                    latency_samples_through(&taps, amp, sample_rate),
                    latency_samples_through(&[1.0], amp, sample_rate)
                );
            }
            println!();
        }
    }

    /// Prints levels, distortion, aliasing, tightness, dynamics, latency and cost for every
    /// amp, and what the gate, the drive pedal, the cabinet's dials, the delay, the reverb and
    /// the safety clip do. Use it to compare
    /// before and after changing the DSP or a model's constants:
    ///   cargo test -p amp --release amp_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn amp_report() {
        let chords = power_chords(SAMPLE_RATE, 4.0);
        let tone = sine(220.0, 0.178, SAMPLE_RATE, 48_000);
        let gains = [0.0, 0.5, 1.0];

        println!("Input: power chords and a 220 Hz sine, both at -18 dBFS RMS. Dials at 5 except Gain. 48 kHz");
        println!("Levels in dBFS. THD of the sine. Aliasing: energy not at harmonics, dB below the total");
        println!(
            "{:<8}{:>5}{:>12}{:>12}{:>10}{:>10}{:>9}{:>12}{:>12}",
            "amp", "gain", "chords RMS", "chords pk", "sine RMS", "sine pk", "THD dB", "alias 1245", "alias 4186"
        );
        for amp in Amp::ALL {
            for gain in gains {
                let settings = with_amp(amp, gain);
                let chords_out = run(&settings, &chords, SAMPLE_RATE);
                let tone_out = run(&settings, &tone, SAMPLE_RATE);
                let settled = &tone_out[24_000..];
                println!(
                    "{:<8}{:>5.1}{:>12.1}{:>12.1}{:>10.1}{:>10.1}{:>9.1}{:>12.1}{:>12.1}",
                    amp.model().name,
                    gain * 10.0,
                    to_db(rms(&chords_out)),
                    to_db(peak(&chords_out)),
                    to_db(rms(settled)),
                    to_db(peak(settled)),
                    thd_db(settled, SAMPLE_RATE, 220.0),
                    aliasing_db(amp, gain, ALIAS_TONES_HZ[0]),
                    aliasing_db(amp, gain, ALIAS_TONES_HZ[1]),
                );
            }
        }

        println!();
        println!("Chords RMS in dBFS across Master (Gain 5) and across Gain (Master 5)");
        for amp in Amp::ALL {
            print!("{:<8}master", amp.model().name);
            for step in 0..=4 {
                let settings = AmpSettings {
                    master: step as f32 * 0.25,
                    ..with_amp(amp, 0.5)
                };
                print!("{:>8.1}", to_db(rms(&run(&settings, &chords[..96_000], SAMPLE_RATE))));
            }
            println!();
            print!("{:<8}gain  ", amp.model().name);
            for step in 0..=4 {
                let output = run(&with_amp(amp, step as f32 * 0.25), &chords[..96_000], SAMPLE_RATE);
                print!("{:>8.1}", to_db(rms(&output)));
            }
            println!();
        }

        println!();
        println!("Spectrum of the chords in octave bands, dB relative to the whole signal");
        print!("{:<14}", "Hz from");
        for edge in &BAND_EDGES_HZ[..BAND_EDGES_HZ.len() - 1] {
            print!("{:>8}", edge);
        }
        println!();
        print!("{:<14}", "dry");
        for level in band_levels_db(&chords, SAMPLE_RATE) {
            print!("{:>8.1}", level);
        }
        println!();
        for amp in Amp::ALL {
            for gain in gains {
                let output = run(&with_amp(amp, gain), &chords, SAMPLE_RATE);
                print!("{:<8}{:>4.1}  ", amp.model().name, gain * 10.0);
                for level in band_levels_db(&output, SAMPLE_RATE) {
                    print!("{:>8.1}", level);
                }
                println!();
            }
        }

        println!();
        println!("Tightness: palm-muted power chords on 65 and 73 Hz roots, level per band in dB relative");
        println!("to the whole signal. `boosted` is the same playing with the lows cut, the mids pushed");
        println!("and 12 dB more level, as from an overdrive pedal in front");
        let mutes = palm_mutes(SAMPLE_RATE, 4.8);
        let print_tight = |name: &str, signal: &[f32]| {
            print!("{:<22}", name);
            for level in tight_levels_db(signal, SAMPLE_RATE) {
                print!("{:>9.1}", level);
            }
            println!("{:>9.1}{:>9.1}", to_db(rms(signal)), to_db(peak(signal)));
        };
        println!(
            "{:<22}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}",
            "Hz", "to 100", "100-400", "400-1k6", "1k6-6k4", "6k4 up", "RMS", "peak"
        );
        print_tight("dry", &mutes);
        print_tight("dry boosted", &boosted(&mutes, SAMPLE_RATE));
        for amp in Amp::ALL {
            for gain in [0.5, 1.0] {
                let name = format!("{} {:.1}", amp.model().name, gain * 10.0);
                print_tight(&name, &run(&with_amp(amp, gain), &mutes, SAMPLE_RATE));
            }
        }
        for gain in [0.5, 1.0] {
            let name = format!("{} {:.1} boosted", Amp::Torden.model().name, gain * 10.0);
            print_tight(&name, &run(&with_amp(Amp::Torden, gain), &boosted(&mutes, SAMPLE_RATE), SAMPLE_RATE));
        }

        println!();
        println!("Dynamics at Gain 5: a 220 Hz sine played softly (-36 dBFS RMS) and hard (-12 dBFS RMS).");
        println!("24 dB apart going in; `squeezed` is how much of that the amp takes away");
        println!(
            "{:<8}{:>10}{:>10}{:>10}{:>10}{:>10}",
            "amp", "soft RMS", "hard RMS", "squeezed", "soft THD", "hard THD"
        );
        for amp in Amp::ALL {
            let play = |level_db: f32| {
                let input = sine(220.0, db_to_gain(level_db) * std::f32::consts::SQRT_2, SAMPLE_RATE, 48_000);
                let output = run(&with_amp(amp, 0.5), &input, SAMPLE_RATE);
                (to_db(rms(&output[24_000..])), thd_db(&output[24_000..], SAMPLE_RATE, 220.0))
            };
            let (soft, hard) = (play(-36.0), play(-12.0));
            println!(
                "{:<8}{:>10.1}{:>10.1}{:>10.1}{:>10.1}{:>10.1}",
                amp.model().name,
                soft.0,
                hard.0,
                24.0 - (hard.0 - soft.0),
                soft.1,
                hard.1
            );
        }

        println!();
        println!("Level after the preamp alone in dBFS RMS (chords), across Gain: tunes `level_db`");
        for amp in Amp::ALL {
            print!("{:<8}", amp.model().name);
            for step in 0..=4 {
                let oversampled_rate = SAMPLE_RATE * MAX_FACTOR as f32;
                let mut oversampler = Oversampler::new();
                let mut preamp = Preamp::new();
                preamp.configure(amp.model(), oversampled_rate);
                preamp.reset();
                preamp.set_gain(amp.model(), step as f32 * 0.25, 0);
                let mut high = vec![0.0; 96_000 * MAX_FACTOR];
                oversampler.upsample(&chords[..96_000], &mut high);
                preamp.process(&mut high);
                print!("{:>8.1}", to_db(rms(&high)));
            }
            println!();
        }

        println!();
        println!("Cabinet response in dB");
        let probes = [
            [63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0],
            [1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 16000.0],
        ];
        for row in probes {
            print!("{:<8}", "Hz");
            for probe in row {
                print!("{:>7}", probe);
            }
            println!();
            for amp in Amp::ALL {
                let ir = design_ir(&amp.model().cab, SAMPLE_RATE);
                print!("{:<8}", amp.model().name);
                for probe in row {
                    print!("{:>7.1}", to_db(ir.magnitude(probe, SAMPLE_RATE)));
                }
                println!();
            }
        }

        println!();
        println!("Switching amps while a 220 Hz sine plays (Gain 5): the largest step between two samples");
        println!("around the switch, against the largest either amp makes by itself");
        for (from, to) in transitions() {
            let at = 300 * BLOCK;
            let output = run_switch(&mut new_chain(SAMPLE_RATE), from, to, 0.5, &tone, at);
            let own_step = largest_step(&output[4800..at]).max(largest_step(&output[at + 9600..]));
            println!(
                "{:<8}to {:<8}{:>8.4} against{:>8.4}",
                from.model().name,
                to.model().name,
                largest_step(&output[at - 1..at + 9600]),
                own_step
            );
        }

        println!();
        println!("Latency (impulse to the first output sample at half the peak level)");
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                let samples = latency_samples(amp, sample_rate);
                println!(
                    "{:<8}{:>8} Hz{:>5} samples{:>7.3} ms",
                    amp.model().name,
                    sample_rate,
                    samples,
                    samples as f32 / sample_rate * 1000.0
                );
            }
        }

        println!();
        println!("Time per {}-sample block, median, and as a share of real time", BLOCK);
        for amp in Amp::ALL {
            for sample_rate in [48000.0, 96000.0, 192000.0] {
                let block_us = BLOCK as f64 / sample_rate as f64 * 1e6;
                let mut chain = new_chain(sample_rate);
                let settings = with_amp(amp, 1.0);
                let chords = power_chords(sample_rate, 2.0);
                run_blocks(&mut chain, &settings, &chords, BLOCK);
                // Long enough for the slowest state in the chain to have decayed all the way
                run_blocks(&mut chain, &settings, &vec![0.0; (sample_rate * 20.0) as usize], BLOCK);
                let silent = block_time_us(&mut chain, &settings, &vec![0.0; (sample_rate * 2.0) as usize]);
                // Measured right after each other, so the processor is in the same mood for both
                let playing = block_time_us(&mut chain, &settings, &chords);
                println!(
                    "{:<8}{:>8} Hz  playing{:>7.1} us{:>6.1} %   silent tail{:>7.1} us{:>6.1} %",
                    amp.model().name,
                    sample_rate,
                    playing,
                    playing / block_us * 100.0,
                    silent,
                    silent / block_us * 100.0
                );
            }
        }

        println!();
        println!("Clean headroom at Gain 0: THD in dB of a 220 Hz sine at three levels in dBFS RMS");
        println!("{:<8}{:>8}{:>8}{:>8}", "amp", "-18", "-12", "-6");
        for amp in Amp::ALL {
            print!("{:<8}", amp.model().name);
            for level_db in [-18.0, -12.0, -6.0] {
                let input = sine(220.0, db_to_gain(level_db) * std::f32::consts::SQRT_2, SAMPLE_RATE, 48_000);
                let output = run(&with_amp(amp, 0.0), &input, SAMPLE_RATE);
                print!("{:>8.1}", thd_db(&output[24_000..], SAMPLE_RATE, 220.0));
            }
            println!();
        }

        println!();
        println!("Gate. A low E that rings out (peak -12 dBFS), the same note cut off after half a second,");
        println!("and hiss at -70 dBFS RMS. `opens`: from the first sample of the note to fully open.");
        println!("`cut closed`: from the cut to silence. `ringing closed`: when the ringing note is shut out.");
        println!("`changes`: times the gate opened or started to close during the ringing note (2 is once each)");
        println!(
            "{:>10}{:>10}{:>10}{:>12}{:>16}{:>9}{:>10}",
            "threshold", "release", "opens ms", "cut closed", "ringing closed", "changes", "hiss"
        );
        let ringing = decaying_note(SAMPLE_RATE, 8.0);
        let mut cut = ringing[..24_000].to_vec();
        cut.resize(96_000, 0.0);
        let noise = hiss(-70.0, 96_000);
        for threshold_db in [-70.0, -60.0, -50.0, -40.0] {
            for release_ms in [20.0, 100.0, 500.0] {
                let trace = |input: &[f32]| {
                    let mut gate = Gate::new();
                    gate.set_sample_rate(SAMPLE_RATE);
                    gate.set(true, threshold_db, release_ms);
                    gain_trace(&mut gate, input)
                };
                let to_ms = |samples: usize| samples as f32 / SAMPLE_RATE * 1000.0;
                let ringing_trace = trace(&ringing);
                let cut_trace = trace(&cut);
                let hiss_trace = trace(&noise);
                let opens = ringing_trace.iter().position(|&gain| gain == 1.0).unwrap();
                let cut_closed = cut_trace[24_000..].iter().position(|&gain| gain == 0.0).unwrap();
                let ringing_closed = match ringing_trace.iter().rposition(|&gain| gain > 0.0) {
                    Some(last) if last + 1 < ringing_trace.len() => format!("{:.2} s", (last + 1) as f32 / SAMPLE_RATE),
                    _ => "still open".to_string(),
                };
                let hiss_gain = rms(&hiss_trace[48_000..]);
                let hiss_state = match hiss_gain {
                    gain if gain == 0.0 => "silent".to_string(),
                    gain if gain == 1.0 => "open".to_string(),
                    gain => format!("{:.1} dB", to_db(gain)),
                };
                println!(
                    "{:>7} dB{:>7} ms{:>10.2}{:>9.0} ms{:>16}{:>9}{:>10}",
                    threshold_db,
                    release_ms,
                    to_ms(opens),
                    to_ms(cut_closed),
                    ringing_closed,
                    gate_changes(&ringing_trace),
                    hiss_state
                );
            }
        }
        println!("Hiss at -70 dBFS RMS through each amp at Gain 10, output in dBFS RMS");
        for amp in Amp::ALL {
            let level = |gate_on: bool| {
                let settings = AmpSettings {
                    gate_on,
                    ..with_amp(amp, 1.0)
                };
                let output = run(&settings, &noise, SAMPLE_RATE);
                let level = rms(&output[48_000..]);
                if level < 1e-9 { "silent".to_string() } else { format!("{:.1}", to_db(level)) }
            };
            println!("{:<8}gate off {:>8}   gate on at -60 dB {:>8}", amp.model().name, level(false), level(true));
        }

        println!();
        println!("Drive pedal alone (no amp), 4x oversampled as in the chain. THD of the 220 Hz sine, level of");
        println!("the chords against the pedal switched off, and the chords per band relative to the whole");
        let pedal_alone = |drive: f32, tone: f32, level: f32, input: &[f32]| {
            let mut oversampler = Oversampler::new();
            let mut pedal = new_drive(SAMPLE_RATE * MAX_FACTOR as f32, drive, tone, level);
            let mut high = vec![0.0; input.len() * MAX_FACTOR];
            oversampler.upsample(input, &mut high);
            pedal.process(&mut high);
            let mut output = vec![0.0; input.len()];
            oversampler.downsample(&high, &mut output);
            output
        };
        println!(
            "{:<22}{:>8}{:>8}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}",
            "drive tone level", "THD dB", "level", "to 100", "100-400", "400-1k6", "1k6-6k4", "6k4 up", "over 3k"
        );
        print!("{:<22}{:>8}{:>8}", "off", "", "");
        for level in tight_levels_db(&chords, SAMPLE_RATE) {
            print!("{:>9.1}", level);
        }
        println!("{:>9.1}", band_level_db(&chords, SAMPLE_RATE, Some(3000.0), None));
        let pedal_settings = [
            (0.0, 0.5, 0.5),
            (0.2, 0.5, 0.5),
            (0.3, 0.5, 0.5),
            (0.5, 0.5, 0.5),
            (1.0, 0.5, 0.5),
            (0.5, 0.0, 0.5),
            (0.5, 1.0, 0.5),
            (0.5, 0.5, 0.0),
            (0.5, 0.5, 1.0),
            (0.2, 0.5, 0.8),
        ];
        for (drive, tone_dial, level) in pedal_settings {
            let sine_out = pedal_alone(drive, tone_dial, level, &tone);
            let chords_out = pedal_alone(drive, tone_dial, level, &chords);
            print!(
                "{:<22}{:>8.1}{:>8.1}",
                format!("{:>4.1}{:>6.1}{:>6.1}", drive * 10.0, tone_dial * 10.0, level * 10.0),
                thd_db(&sine_out[24_000..], SAMPLE_RATE, 220.0),
                to_db(rms(&chords_out) / rms(&chords))
            );
            for level in tight_levels_db(&chords_out, SAMPLE_RATE) {
                print!("{:>9.1}", level);
            }
            println!("{:>9.1}", band_level_db(&chords_out, SAMPLE_RATE, Some(3000.0), None));
        }

        println!();
        println!("Drive pedal into an amp. Chords RMS and peak in dBFS, THD of the sine, aliasing as above");
        println!(
            "{:<12}{:<20}{:>12}{:>12}{:>9}{:>12}{:>12}",
            "amp gain", "drive tone level", "chords RMS", "chords pk", "THD dB", "alias 1245", "alias 4186"
        );
        let pedal_rows = [
            (Amp::Klar, 0.3, None),
            (Amp::Klar, 0.3, Some((0.0, 0.5, 0.5))),
            (Amp::Klar, 0.3, Some((0.5, 0.5, 0.5))),
            (Amp::Klar, 0.3, Some((1.0, 0.5, 0.5))),
            (Amp::Klar, 0.3, Some((1.0, 1.0, 1.0))),
            (Amp::Brol, 0.5, Some((0.3, 0.5, 0.7))),
            (Amp::Torden, 0.5, None),
            (Amp::Torden, 0.5, Some((0.2, 0.5, 0.8))),
            (Amp::Torden, 1.0, None),
            (Amp::Torden, 1.0, Some((0.0, 0.5, 0.5))),
            (Amp::Torden, 1.0, Some((0.3, 0.5, 0.8))),
            (Amp::Torden, 1.0, Some((0.3, 1.0, 1.0))),
            (Amp::Torden, 1.0, Some((1.0, 1.0, 1.0))),
        ];
        for (amp, gain, pedal) in pedal_rows {
            let (settings, pedal_name) = match pedal {
                Some((drive, tone_dial, level)) => (
                    with_drive(amp, gain, drive, tone_dial, level),
                    format!("{:>4.1}{:>6.1}{:>6.1}", drive * 10.0, tone_dial * 10.0, level * 10.0),
                ),
                None => (with_amp(amp, gain), "off".to_string()),
            };
            let chords_out = run(&settings, &chords, SAMPLE_RATE);
            let tone_out = run(&settings, &tone, SAMPLE_RATE);
            println!(
                "{:<12}{:<20}{:>12.1}{:>12.1}{:>9.1}{:>12.1}{:>12.1}",
                format!("{} {:.1}", amp.model().name, gain * 10.0),
                pedal_name,
                to_db(rms(&chords_out)),
                to_db(peak(&chords_out)),
                thd_db(&tone_out[24_000..], SAMPLE_RATE, 220.0),
                aliasing_db_with(&settings, ALIAS_TONES_HZ[0]),
                aliasing_db_with(&settings, ALIAS_TONES_HZ[1]),
            );
        }

        println!();
        println!("Tightness with the pedal itself (Drive 2, Tone 5, Level 8 unless named): the palm mutes as");
        println!("above, and `lows` is the level below 100 Hz against 400 to 1600 Hz. The last columns are the");
        println!("fundamental of a single low C (65 Hz) and low E (82 Hz) against the whole signal, in dB");
        println!(
            "{:<26}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}",
            "Hz", "to 100", "100-400", "400-1k6", "1k6-6k4", "6k4 up", "RMS", "lows", "65 Hz", "82 Hz"
        );
        let low_notes = [65.41, 82.41].map(|freq_hz| (freq_hz, pluck(freq_hz, Pluck::OPEN, 3, SAMPLE_RATE, 48_000)));
        print!("{:<26}", "dry");
        for level in tight_levels_db(&mutes, SAMPLE_RATE) {
            print!("{:>9.1}", level);
        }
        print!("{:>9.1}{:>9.1}", to_db(rms(&mutes)), lows_against_mids_db(&mutes));
        for (freq_hz, note) in &low_notes {
            print!("{:>9.1}", fundamental_db(&note[4800..28_800], *freq_hz));
        }
        println!();
        let tight_rows = [
            ("Brøl 5.0".to_string(), with_amp(Amp::Brol, 0.5)),
            ("Brøl 10.0".to_string(), with_amp(Amp::Brol, 1.0)),
            ("Torden 5.0".to_string(), with_amp(Amp::Torden, 0.5)),
            ("Torden 10.0".to_string(), with_amp(Amp::Torden, 1.0)),
            ("Torden 5.0 drive".to_string(), with_drive(Amp::Torden, 0.5, 0.2, 0.5, 0.8)),
            ("Torden 10.0 drive".to_string(), with_drive(Amp::Torden, 1.0, 0.2, 0.5, 0.8)),
            ("Torden 5.0 drive 0/5/10".to_string(), with_drive(Amp::Torden, 0.5, 0.0, 0.5, 1.0)),
            ("Torden 5.0 drive 5/5/5".to_string(), with_drive(Amp::Torden, 0.5, 0.5, 0.5, 0.5)),
            ("Brøl 5.0 drive".to_string(), with_drive(Amp::Brol, 0.5, 0.2, 0.5, 0.8)),
            ("Klar 5.0".to_string(), with_amp(Amp::Klar, 0.5)),
            ("Klar 5.0 drive 5/5/5".to_string(), with_drive(Amp::Klar, 0.5, 0.5, 0.5, 0.5)),
        ];
        for (name, settings) in &tight_rows {
            let output = run(settings, &mutes, SAMPLE_RATE);
            print!("{:<26}", name);
            for level in tight_levels_db(&output, SAMPLE_RATE) {
                print!("{:>9.1}", level);
            }
            print!("{:>9.1}{:>9.1}", to_db(rms(&output)), lows_against_mids_db(&output));
            for (freq_hz, note) in &low_notes {
                print!("{:>9.1}", fundamental_db(&run(settings, note, SAMPLE_RATE)[4800..28_800], *freq_hz));
            }
            println!();
        }

        println!();
        println!("Cabinet dials: change of the response in dB with Mic and Resonance at either end");
        let cab_probes = [63.0, 100.0, 125.0, 200.0, 400.0, 800.0, 1600.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0];
        print!("{:<18}", "Hz");
        for probe in cab_probes {
            print!("{:>7}", probe);
        }
        println!();
        for amp in Amp::ALL {
            for (name, mic, res) in [("mic 0", 0.0, 0.5), ("mic 10", 1.0, 0.5), ("res 0", 0.5, 0.0), ("res 10", 0.5, 1.0)] {
                print!("{:<18}", format!("{} {}", amp.model().name, name));
                for probe in cab_probes {
                    print!("{:>7.1}", CabVoicing::response_db(&amp.model().cab, mic, res, probe, SAMPLE_RATE));
                }
                println!();
            }
        }
        println!("Chords RMS at Gain 5 against the cabinet as designed, in dB");
        println!("{:<8}{:>8}{:>8}{:>8}{:>8}{:>9}", "amp", "mic 0", "mic 10", "res 0", "res 10", "cab off");
        for amp in Amp::ALL {
            let base = with_amp(amp, 0.5);
            let level = |settings: &AmpSettings| to_db(rms(&run(settings, &chords[..96_000], SAMPLE_RATE)));
            let designed = level(&base);
            println!(
                "{:<8}{:>8.1}{:>8.1}{:>8.1}{:>8.1}{:>9.1}",
                amp.model().name,
                level(&AmpSettings { cab_mic: 0.0, ..base }) - designed,
                level(&AmpSettings { cab_mic: 1.0, ..base }) - designed,
                level(&AmpSettings { cab_res: 0.0, ..base }) - designed,
                level(&AmpSettings { cab_res: 1.0, ..base }) - designed,
                level(&AmpSettings { cab_on: false, ..base }) - designed,
            );
        }

        println!();
        println!("Latency with gate, drive and cabinet dials on (Drive 3, Tone 5 and Tone 0, Level 5)");
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                let samples = latency_samples_with(&everything_on(amp, 0.0), sample_rate);
                let dark = AmpSettings {
                    drive_tone: 0.0,
                    ..everything_on(amp, 0.0)
                };
                let dark_samples = latency_samples_with(&dark, sample_rate);
                println!(
                    "{:<8}{:>8} Hz{:>5} samples{:>7.3} ms   Tone 0{:>5} samples{:>7.3} ms",
                    amp.model().name,
                    sample_rate,
                    samples,
                    samples as f32 / sample_rate * 1000.0,
                    dark_samples,
                    dark_samples as f32 / sample_rate * 1000.0
                );
            }
        }

        println!();
        println!("Time per {}-sample block with everything on (Torden, Gain 10, gate, drive, cabinet dials),", BLOCK);
        println!("and with the gate closed on hiss");
        for sample_rate in [48000.0, 96000.0, 192000.0] {
            let block_us = BLOCK as f64 / sample_rate as f64 * 1e6;
            let mut chain = new_chain(sample_rate);
            let settings = everything_on(Amp::Torden, 1.0);
            let chords = power_chords(sample_rate, 2.0);
            let quiet = hiss(-75.0, (sample_rate * 2.0) as usize);
            run_blocks(&mut chain, &settings, &chords, BLOCK);
            run_blocks(&mut chain, &settings, &vec![0.0; (sample_rate * 20.0) as usize], BLOCK);
            let closed = block_time_us(&mut chain, &settings, &quiet);
            let playing = block_time_us(&mut chain, &settings, &chords);
            let mut plain_chain = new_chain(sample_rate);
            let plain = block_time_us(&mut plain_chain, &with_amp(Amp::Torden, 1.0), &chords);
            println!(
                "{:>8} Hz  playing{:>7.1} us{:>6.1} %   gate closed{:>7.1} us{:>6.1} %   pedals off{:>7.1} us{:>6.1} %",
                sample_rate,
                playing,
                playing / block_us * 100.0,
                closed,
                closed / block_us * 100.0,
                plain,
                plain / block_us * 100.0
            );
        }

        println!();
        println!("Delay and reverb. `default`: both on as the plugin starts (350 ms, 35 %, 25 % and 1.5 s, 20 %).");
        println!("`most`: 350 ms, feedback 90 %, mix 100 % and 6 s, mix 100 %");
        println!();
        println!("Time per {}-sample block, stereo, everything on (Torden, Gain 10, gate, drive, cabinet dials)", BLOCK);
        println!(
            "{:>11}{:>19}{:>19}{:>19}{:>19}{:>19}",
            "", "effects off", "delay", "reverb", "both", "both, tails only"
        );
        for sample_rate in [48000.0, 96000.0, 192000.0] {
            let block_us = BLOCK as f64 / sample_rate as f64 * 1e6;
            let base = everything_on(Amp::Torden, 1.0);
            let both = with_default_effects(base);
            let chords = power_chords(sample_rate, 2.0);
            print!("{:>8} Hz", sample_rate);
            for settings in [base, AmpSettings { reverb: base.reverb, ..both }, AmpSettings { delay: base.delay, ..both }, both] {
                let mut chain = new_chain(sample_rate);
                run_stereo_blocks(&mut chain, &settings, &chords, BLOCK);
                let time = block_time_stereo_us(&mut chain, &settings, &chords);
                print!("{:>9.1} us{:>5.1} %", time, time / block_us * 100.0);
            }
            // Nothing played, the gate closed, the repeats and a long tail ringing
            let long = AmpSettings {
                reverb: reverb_on(6.0, 0.2),
                delay: delay_on(350.0, 0.9, 0.25),
                ..base
            };
            let mut chain = new_chain(sample_rate);
            run_stereo_blocks(&mut chain, &long, &chords, BLOCK);
            run_stereo_blocks(&mut chain, &long, &vec![0.0; sample_rate as usize], BLOCK);
            let time = block_time_stereo_us(&mut chain, &long, &vec![0.0; sample_rate as usize]);
            assert!(!chain.is_idle());
            println!("{:>9.1} us{:>5.1} %", time, time / block_us * 100.0);
        }

        println!();
        println!("Output peak on the chords in dBFS, the louder of left and right: in front of the safety");
        println!("clip and behind it, and the share of samples over its knee ({:.1} dBFS)", to_db(OUTPUT_CLIP_KNEE));
        println!("{:<22}{:>21}{:>24}{:>24}", "", "effects off", "default", "most");
        println!(
            "{:<22}{:>7}{:>7}{:>7}{:>10}{:>7}{:>7}{:>10}{:>7}{:>7}",
            "amp", "front", "out", "over", "front", "out", "over", "front", "out", "over"
        );
        let stereo_peak = |settings: &AmpSettings, input: &[f32]| {
            let (left, right) = run_stereo(settings, input, SAMPLE_RATE);
            let over = left.iter().chain(&right).filter(|s| s.abs() > OUTPUT_CLIP_KNEE).count();
            (peak(&left).max(peak(&right)), over as f32 / (2 * left.len()) as f32 * 100.0)
        };
        let loud: Vec<f32> = chords.iter().map(|s| (s * 2.0).clamp(-1.0, 1.0)).collect();
        let peak_rows = [
            ("Klar 0.0", with_amp(Amp::Klar, 0.0), &chords),
            ("Klar 5.0", with_amp(Amp::Klar, 0.5), &chords),
            ("Brøl 0.0", with_amp(Amp::Brol, 0.0), &chords),
            ("Brøl 5.0", with_amp(Amp::Brol, 0.5), &chords),
            ("Torden 5.0", with_amp(Amp::Torden, 0.5), &chords),
            ("Torden 10.0", with_amp(Amp::Torden, 1.0), &chords),
            ("Klar 5.0 +6 dB in", with_amp(Amp::Klar, 0.5), &loud),
            ("Brøl 5.0 +6 dB in", with_amp(Amp::Brol, 0.5), &loud),
            ("Torden 5.0 +6 dB in", with_amp(Amp::Torden, 0.5), &loud),
            ("Klar 5.0 Master 10", AmpSettings { master: 1.0, ..with_amp(Amp::Klar, 0.5) }, &chords),
            ("Torden 5.0 Master 10", AmpSettings { master: 1.0, ..with_amp(Amp::Torden, 0.5) }, &chords),
        ];
        for (name, base, input) in peak_rows {
            let mut row = format!("{:<22}", name);
            for settings in [base, with_default_effects(base), with_effects_at_most(base)] {
                // The output level sits between the effects and the clip. Turned far down,
                // nothing reaches the clip, and the peak is the one in front of it
                let quiet = AmpSettings { out_level: 0.01, ..settings };
                let front = stereo_peak(&quiet, input).0 * 100.0;
                let (out, over) = stereo_peak(&settings, input);
                row += &format!("{:>7.1}{:>7.1}{:>6.1}%   ", to_db(front), to_db(out), over);
            }
            println!("{}", row.trim_end());
        }

        println!();
        println!("Safety clip. Klar at Gain 3 with the drive at 10, 10, 10 in front, the loudest setting there");
        println!("is, across the Output dial: chords peak in front of the clip and behind it, and aliasing");
        println!("{:<14}{:>8}{:>8}{:>12}{:>12}", "Output dB", "front", "out", "alias 1245", "alias 4186");
        for out_db in [0.0, -3.0, -6.0, -9.0] {
            let settings = AmpSettings {
                out_level: db_to_gain(out_db),
                ..with_drive(Amp::Klar, 0.3, 1.0, 1.0, 1.0)
            };
            let quiet = AmpSettings { out_level: 0.01, ..settings };
            println!(
                "{:<14}{:>8.1}{:>8.1}{:>12.1}{:>12.1}",
                out_db,
                to_db(peak(&run(&quiet, &chords, SAMPLE_RATE)) * 100.0 * settings.out_level),
                to_db(peak(&run(&settings, &chords, SAMPLE_RATE))),
                aliasing_db_with(&settings, ALIAS_TONES_HZ[0]),
                aliasing_db_with(&settings, ALIAS_TONES_HZ[1]),
            );
        }

        println!();
        println!("Stereo (Brøl, Gain 5, chords, in front of the safety clip). `L/R`: how alike left and right");
        println!("of the whole output are, 1.0 is mono. `wet L/R`: the same for what the effects add. `wet`:");
        println!("its level against the amp's own signal. Mono host layout: the plugin puts out the left");
        println!("channel (`layout`: its largest difference from the stereo layout's left). `sum`: what the");
        println!("average of left and right would do to the level of what the effects add, over all and in");
        println!("the octave where it loses most");
        println!(
            "{:<26}{:>8}{:>9}{:>8}{:>10}{:>8}{:>14}",
            "", "L/R", "wet L/R", "wet dB", "layout", "sum dB", "worst octave"
        );
        let base = AmpSettings {
            out_level: 0.01,
            ..with_amp(Amp::Brol, 0.5)
        };
        let defaults = with_default_effects(base);
        let stereo_rows = [
            ("delay default", AmpSettings { reverb: base.reverb, ..defaults }),
            ("reverb default", AmpSettings { delay: base.delay, ..defaults }),
            ("both default", defaults),
            ("delay 100 ms", AmpSettings { delay: delay_on(100.0, 0.35, 0.25), ..base }),
            ("delay 1000 ms", AmpSettings { delay: delay_on(1000.0, 0.35, 0.25), ..base }),
            ("reverb 0.3 s", AmpSettings { reverb: reverb_on(0.3, 0.2), ..base }),
            ("reverb 6 s", AmpSettings { reverb: reverb_on(6.0, 0.2), ..base }),
            ("both most", with_effects_at_most(base)),
        ];
        let dry = run(&base, &chords, SAMPLE_RATE);
        for (name, settings) in stereo_rows {
            let (left, right) = run_stereo(&settings, &chords, SAMPLE_RATE);
            let mono = run(&settings, &chords, SAMPLE_RATE);
            let (wet_left, wet_right) = (difference(&left, &dry), difference(&right, &dry));
            let wet_sum: Vec<f32> = wet_left.iter().zip(&wet_right).map(|(l, r)| 0.5 * (l + r)).collect();
            let worst = [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0]
                .iter()
                .map(|&centre_hz| {
                    let band = |signal: &[f32]| rms(&octave_band(signal, SAMPLE_RATE, centre_hz));
                    to_db(band(&wet_sum) / band(&wet_left))
                })
                .fold(f32::MAX, f32::min);
            println!(
                "{:<26}{:>8.3}{:>9.3}{:>8.1}{:>10.1e}{:>8.1}{:>14.1}",
                name,
                correlation(&left, &right),
                correlation(&wet_left, &wet_right),
                to_db(rms(&wet_left) / rms(&dry)),
                largest_difference(&mono, &left),
                to_db(rms(&wet_sum) / rms(&wet_left)),
                worst
            );
        }

        println!();
        println!("Latency with everything on, without the effects and with them (delay 20 ms, 90 %, 100 %,");
        println!("reverb 6 s, 100 %), in samples");
        println!("{:<8}{:>10}{:>10}{:>10}{:>10}", "", "44100", "48000", "96000", "192000");
        for amp in Amp::ALL {
            print!("{:<8}", amp.model().name);
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                let plain = everything_on(amp, 0.0);
                let effects = AmpSettings {
                    delay: delay_on(20.0, 0.9, 1.0),
                    reverb: reverb_on(6.0, 1.0),
                    ..plain
                };
                print!(
                    "{:>10}",
                    format!("{} / {}", latency_samples_with(&plain, sample_rate), latency_samples_with(&effects, sample_rate))
                );
            }
            println!();
        }

        println!();
        println!("Time from the last note until the chain is idle (Brøl, Gain 5, gate on), in seconds, and the");
        println!("peak of the output in the last 10 ms before that, in dBFS");
        let last_note = power_chords(SAMPLE_RATE, 2.0);
        let idle_rows = [
            ("effects off", with_amp(Amp::Brol, 0.5)),
            ("delay default", AmpSettings { reverb: ReverbSettings::default(), ..with_default_effects(with_amp(Amp::Brol, 0.5)) }),
            ("reverb default", AmpSettings { delay: DelaySettings::default(), ..with_default_effects(with_amp(Amp::Brol, 0.5)) }),
            ("both default", with_default_effects(with_amp(Amp::Brol, 0.5))),
            ("reverb 6 s", AmpSettings { reverb: reverb_on(6.0, 0.2), ..with_amp(Amp::Brol, 0.5) }),
            ("delay 1000 ms, 90 %", AmpSettings { delay: delay_on(1000.0, 0.9, 0.25), ..with_amp(Amp::Brol, 0.5) }),
        ];
        for (name, settings) in idle_rows {
            let mut chain = new_chain(SAMPLE_RATE);
            run_stereo_blocks(&mut chain, &settings, &last_note, BLOCK);
            let mut blocks = 0;
            let mut heard = Vec::new();
            while !chain.is_idle() && blocks < 200 * SAMPLE_RATE as usize / BLOCK {
                let (mut left, mut right) = ([0.0; BLOCK], [0.0; BLOCK]);
                chain.process(&settings, &mut left, Some(&mut right));
                heard.extend(left.iter().zip(&right).map(|(l, r)| l.abs().max(r.abs())));
                blocks += 1;
            }
            let last = &heard[heard.len().saturating_sub(480)..];
            let level = if last.is_empty() { String::new() } else { format!("{:>9.1}", to_db(peak(last))) };
            println!("{:<22}{:>8.2}{}", name, (blocks * BLOCK) as f32 / SAMPLE_RATE, level);
        }

        println!();
        println!("Clipping curves, ns per sample");
        let ramp: Vec<f32> = (0..1_000_000).map(|i| ((i % 2000) as f32 - 1000.0) * 0.004).collect();
        let time_ns = |name: &str, shape: &mut dyn FnMut(f32) -> f32| {
            let start = Instant::now();
            let sum: f32 = ramp.iter().map(|&x| shape(x)).sum();
            std::hint::black_box(sum);
            println!("{:<22}{:>6.1}", name, start.elapsed().as_secs_f64() * 1e9 / ramp.len() as f64);
        };
        let mut clipper = AsymClipper::new();
        let mut second = AsymClipper::new();
        second.set_second_order(true);
        time_ns("asym_clip", &mut |x| asym_clip(x, 1.0, 1.5));
        time_ns("AsymClipper", &mut |x| clipper.process(x));
        time_ns("AsymClipper, 2nd order", &mut |x| second.process(x));
        time_ns("tanh", &mut |x| x.tanh());


        println!();
        println!("Aliasing per sample rate at Gain 10 (and Torden with the drive at 3, 5, 8 in front): the");
        println!("1245 Hz and the 4186 Hz tone, energy not at harmonics up to half the rate, dB below the total");
        let rates = [44100.0, 48000.0, 96000.0, 192000.0];
        print!("{:<22}", "");
        for sample_rate in rates {
            print!("{:>18}", sample_rate);
        }
        println!();
        let alias_rows = [
            ("Klar 10.0".to_string(), with_amp(Amp::Klar, 1.0)),
            ("Brøl 10.0".to_string(), with_amp(Amp::Brol, 1.0)),
            ("Torden 10.0".to_string(), with_amp(Amp::Torden, 1.0)),
            ("Torden 10.0 drive".to_string(), with_drive(Amp::Torden, 1.0, 0.3, 0.5, 0.8)),
            ("Torden 10.0 drive 10s".to_string(), with_drive(Amp::Torden, 1.0, 1.0, 1.0, 1.0)),
        ];
        for (name, settings) in &alias_rows {
            print!("{:<22}", name);
            for sample_rate in rates {
                let levels = ALIAS_TONES_HZ.map(|freq_hz| aliasing_db_at(settings, freq_hz, sample_rate));
                print!("{:>18}", format!("{:.1} / {:.1}", levels[0], levels[1]));
            }
            println!();
        }

        println!();
        println!("The same sound at every sample rate: chords put together from sines (RMS in dBFS and octave");
        println!("bands in dB relative to the whole signal) and the 220 Hz sine (THD in dB). `off` is the");
        println!("largest difference from the 48 kHz row in the levels and in the THD");
        print!("{:<20}{:>8}{:>8}{:>8}", "", "Hz", "RMS", "THD");
        for edge in &BAND_EDGES_HZ[..BAND_EDGES_HZ.len() - 1] {
            print!("{:>7}", edge);
        }
        println!("{:>14}", "off");
        let rate_rows = [
            ("Klar 5.0".to_string(), with_amp(Amp::Klar, 0.5)),
            ("Brøl 5.0".to_string(), with_amp(Amp::Brol, 0.5)),
            ("Torden 5.0".to_string(), with_amp(Amp::Torden, 0.5)),
            ("Torden 10.0 drive".to_string(), with_drive(Amp::Torden, 1.0, 0.3, 0.5, 0.8)),
        ];
        for (name, settings) in &rate_rows {
            let measure = |sample_rate: f32| {
                let chords_out = run(settings, &same_chords(sample_rate, 3.0), sample_rate);
                let half = (0.5 * sample_rate) as usize;
                let tone_out = run(settings, &sine(220.0, 0.178, sample_rate, 2 * half), sample_rate);
                let mut levels = vec![to_db(rms(&chords_out))];
                levels.extend(band_levels_db(&chords_out, sample_rate));
                (levels, thd_db(&tone_out[half..], sample_rate, 220.0))
            };
            let reference = measure(SAMPLE_RATE);
            for sample_rate in rates {
                let (levels, thd) = measure(sample_rate);
                print!("{:<20}{:>8}{:>8.1}{:>8.1}", name, sample_rate, levels[0], thd);
                for level in &levels[1..] {
                    print!("{:>7.1}", level);
                }
                let off = levels.iter().zip(&reference.0).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
                println!("{:>14}", format!("{:.2} / {:.2}", off, (thd - reference.1).abs()));
            }
        }

        println!();
        println!("Zipper noise: a dial swept from 0 to 10 while a sine plays through Klar at Gain 0. Level");
        println!("beside the sine, at the rate the dials are read ({} Hz), against the sine, in dB", SAMPLE_RATE / CHUNK as f32);
        println!("{:<10}{:>8}{:>12}{:>12}{:>12}", "dial", "sine Hz", "in 50 ms", "in 200 ms", "in 1 s");
        for (dial, freq_hz) in SWEPT_DIALS {
            print!("{:<10}{:>8}", dial, freq_hz);
            for sweep_s in [0.05, 0.2, 1.0] {
                print!("{:>12.1}", zipper_db(dial, freq_hz, sweep_s, SAMPLE_RATE));
            }
            println!();
        }

        println!();
        print_stage_times();
    }

    /// Median time to process one stereo block, in microseconds
    fn block_time_stereo_us(chain: &mut AmpChain, settings: &AmpSettings, input: &[f32]) -> f64 {
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        let mut times: Vec<f64> = left
            .chunks_mut(BLOCK)
            .zip(right.chunks_mut(BLOCK))
            .map(|(l, r)| {
                let start = Instant::now();
                chain.process(settings, l, Some(r));
                start.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[times.len() / 2]
    }

    /// The last table of `amp_report` by itself, for working on the cost:
    ///   cargo test -p amp --release stage_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn stage_report() {
        print_stage_times();
    }

    fn print_stage_times() {
        println!("Time per stage: Torden, Gain 10, everything on, default delay and reverb, stereo. Each stage");
        println!("is run by itself over two seconds of chords in pieces of {} samples; the fastest of seven", CHUNK);
        println!("runs, in ns per sample at the host's rate and as a share of real time. `whole` is the chain");
        println!("itself (median block)");
        let profile_settings = with_default_effects(everything_on(Amp::Torden, 1.0));
        let mut profile = Vec::new();
        for sample_rate in [48000.0, 96000.0, 192000.0] {
            let chords = power_chords(sample_rate, 2.0);
            // The fastest run is the one the rest of the machine disturbed least
            let runs: Vec<Vec<f64>> = (0..7).map(|_| stage_times_ns(&profile_settings, &chords, sample_rate)).collect();
            let medians: Vec<f64> = (0..STAGE_NAMES.len())
                .map(|stage| runs.iter().map(|run| run[stage]).fold(f64::MAX, f64::min))
                .collect();
            let mut chain = new_chain(sample_rate);
            run_stereo_blocks(&mut chain, &profile_settings, &chords, BLOCK);
            let whole = block_time_stereo_us(&mut chain, &profile_settings, &chords) * 1000.0 / BLOCK as f64;
            profile.push((sample_rate, medians, whole));
        }
        print!("{:<10}", "");
        for (sample_rate, _, _) in &profile {
            print!("{:>16} Hz", sample_rate);
        }
        println!();
        let share = |ns: f64, sample_rate: f32| ns * 1e-9 * sample_rate as f64 * 100.0;
        for (stage, name) in STAGE_NAMES.iter().enumerate() {
            print!("{:<10}", name);
            for (sample_rate, medians, _) in &profile {
                print!("{:>8.1} ns{:>6.2} %", medians[stage], share(medians[stage], *sample_rate));
            }
            println!();
        }
        for (name, pick) in [("sum", 0), ("whole", 1)] {
            print!("{:<10}", name);
            for (sample_rate, medians, whole) in &profile {
                let ns = if pick == 0 { medians.iter().sum::<f64>() } else { *whole };
                print!("{:>8.1} ns{:>6.2} %", ns, share(ns, *sample_rate));
            }
            println!();
        }
    }

    /// Writes WAV files to target/renders for listening: a direct guitar signal, dry and
    /// through every amp at a few settings, palm mutes through every amp, the drive pedal in
    /// front of Torden and of Klar, the cabinet's dials and the cabinet switched off, the gate
    /// on a noisy input, one file that changes amp every two seconds, and each amp in stereo
    /// with delay and reverb:
    ///   cargo test -p amp --release render_wavs -- --ignored
    ///
    /// Set AMP_INPUT_WAV to the path of a recording to use that instead of the made-up one.
    /// It is processed at its own sample rate.
    #[test]
    #[ignore]
    fn render_wavs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();

        let (input, sample_rate) = match std::env::var("AMP_INPUT_WAV") {
            Ok(path) => read_wav_mono(&path),
            Err(_) => (guitar_di(SAMPLE_RATE), SAMPLE_RATE as u32),
        };
        write_wav(&dir.join("amp_dry.wav"), &[&input], sample_rate);

        let settings = [
            ("gain0", with_gain(0.0)),
            ("gain3", with_gain(0.3)),
            ("gain5", with_gain(0.5)),
            ("gain7", with_gain(0.7)),
            ("gain10", with_gain(1.0)),
            (
                "gain5_master10",
                AmpSettings {
                    master: 1.0,
                    out_level: 0.5,
                    ..AmpSettings::default()
                },
            ),
            (
                "gain7_scooped",
                AmpSettings {
                    gain: 0.7,
                    bass: 0.8,
                    mid: 0.2,
                    treble: 0.8,
                    ..AmpSettings::default()
                },
            ),
        ];
        let file_name = |amp: Amp, name: &str| format!("amp_{}_{}.wav", format!("{:?}", amp).to_lowercase(), name);
        let mutes = palm_mutes(sample_rate as f32, 4.8);
        for amp in Amp::ALL {
            for (name, settings) in &settings {
                let settings = AmpSettings { amp, ..*settings };
                let output = run(&settings, &input, sample_rate as f32);
                write_wav(&dir.join(file_name(amp, name)), &[&output, &output], sample_rate);
            }
            let output = run(&with_amp(amp, 0.6), &mutes, sample_rate as f32);
            write_wav(&dir.join(file_name(amp, "palm_mutes_gain6")), &[&output, &output], sample_rate);
        }
        let output = run(&with_amp(Amp::Torden, 0.5), &boosted(&mutes, sample_rate as f32), sample_rate as f32);
        write_wav(&dir.join(file_name(Amp::Torden, "palm_mutes_boosted_gain5")), &[&output, &output], sample_rate);

        // The pedals. Torden with and without the drive in front, on the recording and on
        // palm mutes; Klar with the drive as an overdrive of its own
        let rate = sample_rate as f32;
        let render = |name: &str, settings: &AmpSettings, input: &[f32]| {
            let output = run(settings, input, rate);
            write_wav(&dir.join(format!("amp_{}.wav", name)), &[&output, &output], sample_rate);
        };
        render("torden_gain6_drive_off", &with_amp(Amp::Torden, 0.6), &input);
        render("torden_gain6_drive_on", &with_drive(Amp::Torden, 0.6, 0.2, 0.5, 0.8), &input);
        render("torden_palm_mutes_gain6_drive_on", &with_drive(Amp::Torden, 0.6, 0.2, 0.5, 0.8), &mutes);
        render("klar_gain3_drive_off", &with_amp(Amp::Klar, 0.3), &input);
        render("klar_gain3_drive3", &with_drive(Amp::Klar, 0.3, 0.3, 0.5, 0.5), &input);
        render("klar_gain3_drive7", &with_drive(Amp::Klar, 0.3, 0.7, 0.5, 0.5), &input);
        render("klar_gain3_drive10_tone8", &with_drive(Amp::Klar, 0.3, 1.0, 0.8, 0.5), &input);

        // The cabinet: Mic at 0, 5 and 10, Resonance at 0 and 10, and switched off
        for (name, mic, res, cab_on) in [
            ("mic0", 0.0, 0.5, true),
            ("mic5", 0.5, 0.5, true),
            ("mic10", 1.0, 0.5, true),
            ("res0", 0.5, 0.0, true),
            ("res10", 0.5, 1.0, true),
            ("off", 0.5, 0.5, false),
        ] {
            let settings = AmpSettings {
                cab_on,
                cab_mic: mic,
                cab_res: res,
                ..with_amp(Amp::Brol, 0.6)
            };
            render(&format!("brol_gain6_cab_{}", name), &settings, &input);
        }

        // The gate: the recording with hum and hiss under it, twice through Torden. The
        // first time with the gate off, the second time with it on
        let mut noisy = input.clone();
        let mut noise = Noise::new(9);
        for (index, sample) in noisy.iter_mut().enumerate() {
            let time = index as f32 / rate;
            let hum = (std::f32::consts::TAU * 50.0 * time).sin() + 0.5 * (std::f32::consts::TAU * 150.0 * time).sin();
            *sample += db_to_gain(-66.0) * hum + db_to_gain(-70.0) * noise.next();
        }
        let mut chain = new_chain(rate);
        let gate_off = AmpSettings {
            gate_on: false,
            ..with_amp(Amp::Torden, 0.7)
        };
        let gate_on = AmpSettings {
            gate_thresh_db: -55.0,
            ..with_amp(Amp::Torden, 0.7)
        };
        let mut output = run_blocks(&mut chain, &gate_off, &noisy, BLOCK);
        output.extend(run_blocks(&mut chain, &gate_on, &noisy, BLOCK));
        write_wav(&dir.join("amp_torden_gate_off_then_on.wav"), &[&output, &output], sample_rate);

        // A new amp every two seconds while the playing goes on
        let mut chain = new_chain(sample_rate as f32);
        let mut output = input.clone();
        for (index, block) in output.chunks_mut(BLOCK).enumerate() {
            let turn = index * BLOCK / (2 * sample_rate as usize);
            chain.process(&with_amp(Amp::ALL[turn % Amp::ALL.len()], 0.5), block, None);
        }
        write_wav(&dir.join("amp_switching.wav"), &[&output, &output], sample_rate);

        // The effects, in stereo, with three seconds after the playing for what rings on:
        // Klar with delay and reverb, Brøl in a small room, Torden as a lead with delay
        let mut ringing_out = input.clone();
        ringing_out.resize(input.len() + 3 * sample_rate as usize, 0.0);
        let effect_renders = [
            (
                "klar_gain4_delay_reverb",
                AmpSettings {
                    delay: delay_on(380.0, 0.4, 0.3),
                    reverb: reverb_on(2.2, 0.3),
                    ..with_amp(Amp::Klar, 0.4)
                },
            ),
            (
                "brol_gain6_room",
                AmpSettings {
                    reverb: reverb_on(0.5, 0.3),
                    ..with_amp(Amp::Brol, 0.6)
                },
            ),
            (
                "torden_gain7_lead_delay",
                AmpSettings {
                    delay: delay_on(430.0, 0.45, 0.3),
                    reverb: reverb_on(1.2, 0.1),
                    mid: 0.65,
                    ..with_drive(Amp::Torden, 0.7, 0.2, 0.5, 0.8)
                },
            ),
            ("brol_gain5_default_effects", with_default_effects(with_amp(Amp::Brol, 0.5))),
        ];
        for (name, settings) in effect_renders {
            let (left, right) = run_stereo(&settings, &ringing_out, rate);
            write_wav(&dir.join(format!("amp_{}.wav", name)), &[&left, &right], sample_rate);
        }

        println!("Wrote renders to {}", dir.display());
    }
}
