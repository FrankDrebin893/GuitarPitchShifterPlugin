use crate::amp::model::CabModel;
use crate::dsp::db_to_gain;
use crate::dsp::filters::{Biquad, BiquadCoeffs, GlidingBiquad};

/// The longest impulse response a cabinet designed here can have: 20 ms at 192 kHz
pub const MAX_IR_LEN: usize = 3840;

/// A player's own impulse response is cut to this length (`user_cab_report` prints what
/// the lengths cost)
pub const USER_IR_MS: f32 = 40.0;
/// The longest impulse response the cabinet can hold: `USER_IR_MS` at 192 kHz
pub const MAX_USER_IR_LEN: usize = 7680;

// No impulse response is longer than this. The cabinets designed here are a quarter to
// less than half of it (`CabModel.ir_ms`)
const IR_MS: f32 = 20.0;

// Share of the impulse response, at its end, that is faded to zero
const FADE_SHARE: f32 = 0.25;

// The speaker's resonance rings many times longer than everything else a cabinet does: an
// impulse response that holds it has to be 20 ms long, without it 5 to 8 ms do. So the
// resonance is a filter behind the convolution. The cabinets were tuned as impulse responses
// of `IR_MS` that held it, faded out over their second half, which took a little off its
// ring. The filter is fitted to that response over this band, so the low end stays as tuned
const LONG_FADE_SHARE: f32 = 0.5;
const FIT_BAND_HZ: [f32; 2] = [60.0, 250.0];
const FIT_PROBES: usize = 16;
// How far the fit may move the resonance's frequency and Q, as a share of each, the number
// of values tried either side of the best so far, and the rounds, each five times finer
const FIT_SPAN: [f32; 2] = [0.1, 0.3];
const FIT_STEPS: i32 = 5;
const FIT_ROUNDS: usize = 3;

// The response is levelled so that its average over this band is 0 dB
const LEVEL_BAND_HZ: [f32; 2] = [100.0, 4000.0];
const LEVEL_PROBES: usize = 32;

// Two second-order sections with these Qs make a maximally flat 24 dB per octave slope
const ROLLOFF_QS: [f32; 2] = [0.541, 1.307];

const BREAKUP_Q: [f32; 2] = [6.0, 16.0];

// Crossfade when the impulse response is swapped while playing
const SWAP_MS: f32 = 10.0;

// Mic dial: a shelf above the cabinet's bite, this many dB at either end of the dial. Its
// corner is the bite frequency times these, at the dark and at the bright end
const MIC_DB: f32 = 7.0;
const MIC_CORNER_RATIO: [f32; 2] = [0.8, 1.1];
const MIC_Q: f32 = 0.6;
// And a little the other way below this, as a microphone at the edge of the cone hears it
const MIC_LOW_HZ: f32 = 350.0;
const MIC_LOW_DB: f32 = 1.5;
// Level in dB added at the dark end and taken away at the bright end, to keep the loudness
const MIC_TRIM_DB: f32 = 0.4;

// Resonance dial: a bell at the speaker's resonance, this many dB at either end
const THUMP_DB: f32 = 6.0;
const THUMP_Q: f32 = 1.4;

/// The most samples an impulse response has at a sample rate
pub fn ir_len(sample_rate: f32) -> usize {
    ((IR_MS * 0.001 * sample_rate).round() as usize).clamp(16, MAX_IR_LEN)
}

/// The most samples a player's own impulse response has at a sample rate
pub fn user_ir_len(sample_rate: f32) -> usize {
    ((USER_IR_MS * 0.001 * sample_rate).round() as usize).clamp(16, MAX_USER_IR_LEN)
}

/// The gain that brings a response to 0 dB average power over the level band. `magnitude`
/// is its gain at a frequency in Hz. Every cabinet, designed here or loaded from a file,
/// is levelled with this, so one is as loud as another
pub fn level_gain(magnitude: impl Fn(f32) -> f32) -> f32 {
    let ratio = (LEVEL_BAND_HZ[1] / LEVEL_BAND_HZ[0]).powf(1.0 / (LEVEL_PROBES - 1) as f32);
    let power: f32 = (0..LEVEL_PROBES)
        .map(|probe| magnitude(LEVEL_BAND_HZ[0] * ratio.powi(probe as i32)).powi(2))
        .sum::<f32>()
        / LEVEL_PROBES as f32;
    1.0 / power.sqrt().max(1e-9)
}

fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-9).log10()
}

/// A cabinet as it is run: a short impulse response, and the speaker's resonance as a
/// filter behind it
#[derive(Clone, Debug, PartialEq)]
pub struct CabIr {
    /// Everything but the resonance, first sample first
    pub taps: Vec<f32>,
    /// The resonance: a high-pass with a bump at its corner. `BiquadCoeffs::IDENTITY`
    /// leaves the impulse response by itself
    pub lows: BiquadCoeffs,
}

impl CabIr {
    /// An impulse response by itself
    #[cfg(test)]
    pub fn plain(taps: &[f32]) -> Self {
        Self {
            taps: taps.to_vec(),
            lows: BiquadCoeffs::IDENTITY,
        }
    }

    /// Gain of the cabinet at one frequency
    pub fn magnitude(&self, freq_hz: f32, sample_rate: f32) -> f32 {
        ir_magnitude(&self.taps, freq_hz, sample_rate) * db_to_gain(self.lows.magnitude_db(freq_hz, sample_rate))
    }
}

/// Designs a cabinet. The same model sounds the same at every sample rate. Allocates and
/// takes a while: not for the audio thread
pub fn design_ir(model: &CabModel, sample_rate: f32) -> CabIr {
    let rest = rest_filters(model, sample_rate);
    let len = ((model.ir_ms * 0.001 * sample_rate).round() as usize).clamp(16, ir_len(sample_rate));
    let taps = faded_response(&rest, len, FADE_SHARE);
    let lows = fit_lows(model, &long_response(model, sample_rate), &taps, sample_rate);
    let mut ir = CabIr { taps, lows };

    let gain = level_gain(|freq_hz| ir.magnitude(freq_hz, sample_rate));
    for tap in &mut ir.taps {
        *tap *= gain;
    }
    ir
}

fn resonance_filter(model: &CabModel, sample_rate: f32) -> BiquadCoeffs {
    BiquadCoeffs::highpass(model.resonance_hz, model.resonance_q, sample_rate)
}

/// The whole cabinet as one impulse response of `IR_MS`, not levelled: what the cabinet
/// was tuned as, and what the short one with its filter is held against
fn long_response(model: &CabModel, sample_rate: f32) -> Vec<f32> {
    let mut filters = vec![resonance_filter(model, sample_rate)];
    filters.extend(rest_filters(model, sample_rate));
    faded_response(&filters, ir_len(sample_rate), LONG_FADE_SHARE)
}

/// The resonance filter that, behind `taps`, comes closest to `long` at the low end: the
/// model's own, with its frequency and Q moved a little
fn fit_lows(model: &CabModel, long: &[f32], taps: &[f32], sample_rate: f32) -> BiquadCoeffs {
    let ratio = (FIT_BAND_HZ[1] / FIT_BAND_HZ[0]).powf(1.0 / (FIT_PROBES - 1) as f32);
    // What the filter has to do at each probe, in dB
    let wanted: Vec<(f32, f32)> = (0..FIT_PROBES)
        .map(|probe| {
            let freq_hz = FIT_BAND_HZ[0] * ratio.powi(probe as i32);
            let long_db = gain_to_db(ir_magnitude(long, freq_hz, sample_rate));
            (freq_hz, long_db - gain_to_db(ir_magnitude(taps, freq_hz, sample_rate)))
        })
        .collect();
    let design = |moved: [f32; 2]| {
        BiquadCoeffs::highpass(model.resonance_hz * moved[0], model.resonance_q * moved[1], sample_rate)
    };
    let error = |moved: [f32; 2]| {
        let filter = design(moved);
        wanted
            .iter()
            .map(|&(freq_hz, wanted_db)| (filter.magnitude_db(freq_hz, sample_rate) - wanted_db).abs())
            .fold(0.0, f32::max)
    };

    let mut best = [1.0, 1.0];
    let mut best_error = error(best);
    let mut span = FIT_SPAN;
    for _ in 0..FIT_ROUNDS {
        let centre = best;
        for freq_step in -FIT_STEPS..=FIT_STEPS {
            for q_step in -FIT_STEPS..=FIT_STEPS {
                let moved = [
                    centre[0] + span[0] * freq_step as f32 / FIT_STEPS as f32,
                    centre[1] + span[1] * q_step as f32 / FIT_STEPS as f32,
                ];
                let moved_error = error(moved);
                if moved_error < best_error {
                    (best, best_error) = (moved, moved_error);
                }
            }
        }
        span = [span[0] / FIT_STEPS as f32, span[1] / FIT_STEPS as f32];
    }
    design(best)
}

/// Everything a cabinet does but its resonance
fn rest_filters(model: &CabModel, sample_rate: f32) -> Vec<BiquadCoeffs> {
    let mut filters = vec![
        BiquadCoeffs::peak(model.body[0], model.body[1], model.body[2], sample_rate),
        BiquadCoeffs::peak(model.dip[0], model.dip[1], model.dip[2], sample_rate),
        BiquadCoeffs::peak(model.bite[0], model.bite[1], model.bite[2], sample_rate),
        BiquadCoeffs::lowpass(model.rolloff_hz, ROLLOFF_QS[0], sample_rate),
        BiquadCoeffs::lowpass(model.rolloff_hz, ROLLOFF_QS[1], sample_rate),
        // A last, gentler slope above the roll-off, as a cone loses what little is left up there
        BiquadCoeffs::lowpass(model.rolloff_hz * model.air_ratio, 0.707, sample_rate),
    ];

    // Cone breakup: one narrow peak or notch per slice of the range, placed by the seed
    let mut random = Random::new(model.breakup_seed);
    let span = model.breakup_hz[1] / model.breakup_hz[0];
    for index in 0..model.breakup_count {
        let position = (index as f32 + random.next()) / model.breakup_count as f32;
        let freq = model.breakup_hz[0] * span.powf(position);
        let gain_db = model.breakup_db * (0.3 + 0.7 * random.next()) * if random.next() < 0.5 { -1.0 } else { 1.0 };
        let q = BREAKUP_Q[0] + (BREAKUP_Q[1] - BREAKUP_Q[0]) * random.next();
        filters.push(BiquadCoeffs::peak(freq, q, gain_db, sample_rate));
    }
    filters
}

/// The first `len` samples of what a row of filters answers to an impulse, the last
/// `fade_share` of them faded to zero
fn faded_response(filters: &[BiquadCoeffs], len: usize, fade_share: f32) -> Vec<f32> {
    let mut sections: Vec<Biquad> = filters
        .iter()
        .map(|&coeffs| {
            let mut biquad = Biquad::new();
            biquad.set(coeffs);
            biquad
        })
        .collect();

    let fade_start = ((1.0 - fade_share) * len as f32) as usize;
    (0..len)
        .map(|index| {
            let impulse = if index == 0 { 1.0 } else { 0.0 };
            let sample = sections.iter_mut().fold(impulse, |signal, section| section.process(signal));
            let fade = if index < fade_start {
                1.0
            } else {
                let position = (index + 1 - fade_start) as f64 / (len - fade_start) as f64;
                0.5 + 0.5 * (std::f64::consts::PI * position).cos()
            };
            (sample * fade) as f32
        })
        .collect()
}

/// Gain of an impulse response at one frequency
pub fn ir_magnitude(ir: &[f32], freq_hz: f32, sample_rate: f32) -> f32 {
    let omega = std::f64::consts::TAU * freq_hz as f64 / sample_rate as f64;
    let (re, im) = ir.iter().enumerate().fold((0.0, 0.0), |(re, im), (index, &sample)| {
        let (sin, cos) = (omega * index as f64).sin_cos();
        (re + sample as f64 * cos, im - sample as f64 * sin)
    });
    (re * re + im * im).sqrt() as f32
}

/// Numbers in 0.0..1.0 from a seed (xorshift), the same on every run
struct Random {
    state: u32,
}

impl Random {
    fn new(seed: u32) -> Self {
        Self {
            state: seed.wrapping_mul(0x9E37_79B9) | 1,
        }
    }

    fn next(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state >> 8) as f32 / (1 << 24) as f32
    }
}

/// Sum of products. Sixteen running sums instead of one, so the compiler can use vector
/// instructions (it may not reorder a single floating point sum by itself)
fn dot(a: &[f32], b: &[f32]) -> f32 {
    const LANES: usize = 16;
    let mut sums = [0.0f32; LANES];
    let mut a_chunks = a.chunks_exact(LANES);
    let mut b_chunks = b.chunks_exact(LANES);
    for (a_chunk, b_chunk) in (&mut a_chunks).zip(&mut b_chunks) {
        for lane in 0..LANES {
            sums[lane] += a_chunk[lane] * b_chunk[lane];
        }
    }
    let rest: f32 = a_chunks.remainder().iter().zip(b_chunks.remainder()).map(|(x, y)| x * y).sum();
    sums.iter().sum::<f32>() + rest
}

/// One cabinet, loaded
struct Slot {
    // The impulse response stored backwards, so it lines up with the history oldest first
    taps: Vec<f32>,
    len: usize,
    lows: Biquad,
    // False leaves the filter out: the impulse response by itself, to the bit
    filtered: bool,
}

impl Slot {
    fn new() -> Self {
        let mut taps = vec![0.0; MAX_USER_IR_LEN];
        taps[0] = 1.0;
        Self {
            taps,
            len: 1,
            lows: Biquad::new(),
            filtered: false,
        }
    }
}

/// The cabinet: convolution with an impulse response, computed directly so it adds no
/// latency, and the speaker's resonance as a filter behind it. Holds two cabinets so one
/// can be swapped for another without a click
pub struct Cabinet {
    // The most samples an impulse response has at this sample rate
    ring: usize,
    // The last `ring` input samples as a ring, each written twice, `ring` apart: the latest
    // ones are then always one straight run, whatever the length of the impulse response
    history: Vec<f32>,
    position: usize,
    slots: [Slot; 2],
    active: usize,
    // 1.0 when only the active cabinet plays, less while the previous one fades out
    fade: f32,
    fade_step: f32,
}

impl Cabinet {
    pub fn new() -> Self {
        Self {
            ring: MAX_USER_IR_LEN,
            history: vec![0.0; 2 * MAX_USER_IR_LEN],
            position: 0,
            slots: [Slot::new(), Slot::new()],
            active: 0,
            fade: 1.0,
            fade_step: 0.0,
        }
    }

    /// Sets the most samples an impulse response has at this rate. Load one with `set_ir` after
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.ring = user_ir_len(sample_rate);
        self.fade_step = 1.0 / (SWAP_MS * 0.001 * sample_rate);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.position = 0;
        self.fade = 1.0;
        for slot in &mut self.slots {
            slot.lows.reset();
        }
    }

    /// Loads a cabinet and uses it at once. A longer impulse response is cut to the most
    /// the cabinet holds at this sample rate. Does not allocate
    pub fn set_ir(&mut self, ir: &CabIr) {
        self.load(self.active, &ir.taps, ir.lows);
        self.fade = 1.0;
    }

    /// The same for an impulse response by itself, as a player's own cabinet is
    pub fn set_taps(&mut self, taps: &[f32]) {
        self.load(self.active, taps, BiquadCoeffs::IDENTITY);
        self.fade = 1.0;
    }

    /// Loads a cabinet and crossfades to it. Only while `is_swapping` is false: there are
    /// two slots, and during a crossfade both are heard
    pub fn swap_ir(&mut self, ir: &CabIr) {
        self.active = 1 - self.active;
        self.load(self.active, &ir.taps, ir.lows);
        self.fade = 0.0;
    }

    /// The same for an impulse response by itself. Does not allocate either
    pub fn swap_taps(&mut self, taps: &[f32]) {
        self.active = 1 - self.active;
        self.load(self.active, taps, BiquadCoeffs::IDENTITY);
        self.fade = 0.0;
    }

    /// True while the crossfade started by `swap_ir` is running
    pub fn is_swapping(&self) -> bool {
        self.fade < 1.0
    }

    /// Samples after which a crossfade is sure to be over
    pub fn swap_len(&self) -> usize {
        (1.0 / self.fade_step).ceil() as usize + 1
    }

    /// Address and capacity of every buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub fn buffers(&self) -> [(usize, usize); 3] {
        let describe = |buffer: &Vec<f32>| (buffer.as_ptr() as usize, buffer.capacity());
        [describe(&self.history), describe(&self.slots[0].taps), describe(&self.slots[1].taps)]
    }

    /// The filter starts from rest: it is another one than the slot held before
    fn load(&mut self, slot: usize, taps: &[f32], lows: BiquadCoeffs) {
        let slot = &mut self.slots[slot];
        slot.len = taps.len().clamp(1, self.ring);
        let reversed = &mut slot.taps[..slot.len];
        reversed.fill(0.0);
        for (tap, &sample) in reversed.iter_mut().rev().zip(taps) {
            *tap = sample;
        }
        slot.lows.set(lows);
        slot.lows.reset();
        slot.filtered = lows != BiquadCoeffs::IDENTITY;
    }

    /// One sample of one of the cabinets. The input has been written to the history
    fn run(&mut self, slot: usize) -> f32 {
        let slot = &mut self.slots[slot];
        // The second copy of the newest sample ends the run of the latest ones
        let end = self.position + self.ring + 1;
        let output = dot(&self.history[end - slot.len..end], &slot.taps[..slot.len]);
        if slot.filtered {
            slot.lows.process(output as f64) as f32
        } else {
            output
        }
    }

    pub fn process(&mut self, block: &mut [f32]) {
        for sample in block.iter_mut() {
            self.history[self.position] = *sample;
            self.history[self.position + self.ring] = *sample;

            let mut output = self.run(self.active);
            if self.fade < 1.0 {
                output = self.fade * output + (1.0 - self.fade) * self.run(1 - self.active);
                self.fade = (self.fade + self.fade_step).min(1.0);
            }
            *sample = output;

            self.position += 1;
            if self.position == self.ring {
                self.position = 0;
            }
        }
    }
}

/// The Mic and Resonance dials: filters after the convolution. With a dial at its centre
/// its filters are skipped, and the cabinet is the impulse response as designed
pub struct CabVoicing {
    mic_high: GlidingBiquad,
    mic_low: GlidingBiquad,
    thump: GlidingBiquad,
    // The dial is at its centre, or on its last steps there
    mic_centred: bool,
    thump_centred: bool,
}

impl CabVoicing {
    pub fn new() -> Self {
        Self {
            mic_high: GlidingBiquad::new(),
            mic_low: GlidingBiquad::new(),
            thump: GlidingBiquad::new(),
            mic_centred: true,
            thump_centred: true,
        }
    }

    /// The filters of a dial run unless it has arrived at its centre
    fn mic_runs(&self) -> bool {
        !self.mic_centred || self.mic_high.is_gliding()
    }

    fn thump_runs(&self) -> bool {
        !self.thump_centred || self.thump.is_gliding()
    }

    pub fn reset(&mut self) {
        self.mic_high.reset();
        self.mic_low.reset();
        self.thump.reset();
    }

    /// `mic` and `resonance` are dial positions, 0.0 to 1.0. Below 0.5 the microphone moves
    /// to the edge of the cone (darker, a little fuller), above it to the centre (brighter,
    /// a little leaner). `resonance` is how much the cabinet thumps at its own resonance.
    /// The filters move there over the next `steps` samples; at once with no steps
    pub fn set(&mut self, model: &CabModel, mic: f32, resonance: f32, sample_rate: f32, steps: u32) {
        let (mic_coeffs, mic_centred) = Self::mic_coeffs(model, mic, sample_rate);
        // Coming from the centre the filters start from rest. What they hold there passes
        // the signal as it is, so they start from where the skipped signal was
        if !self.mic_runs() && !mic_centred {
            self.mic_high.reset();
            self.mic_low.reset();
        }
        self.mic_high.set(mic_coeffs[0], steps);
        self.mic_low.set(mic_coeffs[1], steps);
        self.mic_centred = mic_centred;

        let (thump_coeffs, thump_centred) = Self::thump_coeffs(model, resonance, sample_rate);
        if !self.thump_runs() && !thump_centred {
            self.thump.reset();
        }
        self.thump.set(thump_coeffs, steps);
        self.thump_centred = thump_centred;
    }

    fn mic_coeffs(model: &CabModel, mic: f32, sample_rate: f32) -> ([BiquadCoeffs; 2], bool) {
        let mic = mic.clamp(0.0, 1.0);
        let position = (mic - 0.5) * 2.0;
        let corner_hz = model.bite[0] * (MIC_CORNER_RATIO[0] + (MIC_CORNER_RATIO[1] - MIC_CORNER_RATIO[0]) * mic);
        let trim = 10.0f32.powf(-MIC_TRIM_DB * position / 20.0);
        (
            [
                BiquadCoeffs::high_shelf(corner_hz, MIC_Q, MIC_DB * position, sample_rate),
                BiquadCoeffs::low_shelf(MIC_LOW_HZ, 0.707, -MIC_LOW_DB * position, sample_rate).scaled(trim),
            ],
            position == 0.0,
        )
    }

    fn thump_coeffs(model: &CabModel, resonance: f32, sample_rate: f32) -> (BiquadCoeffs, bool) {
        let position = (resonance.clamp(0.0, 1.0) - 0.5) * 2.0;
        (BiquadCoeffs::peak(model.resonance_hz, THUMP_Q, THUMP_DB * position, sample_rate), position == 0.0)
    }

    pub fn process(&mut self, block: &mut [f32]) {
        for sample in block.iter_mut() {
            if self.mic_runs() {
                *sample = self.mic_low.process(self.mic_high.process(*sample as f64)) as f32;
            }
            if self.thump_runs() {
                *sample = self.thump.process(*sample as f64) as f32;
            }
        }
    }

    /// What the dials do to the level at one frequency, in dB
    #[cfg(test)]
    pub fn response_db(model: &CabModel, mic: f32, resonance: f32, freq_hz: f32, sample_rate: f32) -> f32 {
        if mic == 0.5 && resonance == 0.5 {
            return 0.0;
        }
        let (mic_coeffs, _) = Self::mic_coeffs(model, mic, sample_rate);
        let (thump_coeffs, _) = Self::thump_coeffs(model, resonance, sample_rate);
        mic_coeffs[0].magnitude_db(freq_hz, sample_rate)
            + mic_coeffs[1].magnitude_db(freq_hz, sample_rate)
            + thump_coeffs.magnitude_db(freq_hz, sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amp::model::Amp;
    use crate::test_util::{rms, sine, to_db, Noise};

    const PROBES_HZ: [f32; 7] = [100.0, 250.0, 500.0, 1000.0, 2000.0, 3000.0, 4000.0];

    fn design(amp: Amp, sample_rate: f32) -> CabIr {
        design_ir(&amp.model().cab, sample_rate)
    }

    /// What a cabinet answers to an impulse, filter and all
    fn impulse_response(ir: &CabIr, sample_rate: f32, len: usize) -> Vec<f32> {
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(ir);
        let mut block = vec![0.0; len];
        block[0] = 1.0;
        cabinet.process(&mut block);
        block
    }

    /// Largest difference in dB, from 60 Hz to 8 kHz, between a cabinet and the one long
    /// impulse response it was tuned as, both levelled alike. And the frequency it is at
    fn off_the_long_response(amp: Amp, sample_rate: f32, probes: usize) -> (f32, f32) {
        let ir = design(amp, sample_rate);
        let long = long_response(&amp.model().cab, sample_rate);
        let ratio = (LEVEL_BAND_HZ[1] / LEVEL_BAND_HZ[0]).powf(1.0 / (LEVEL_PROBES - 1) as f32);
        let long_level: f32 = (0..LEVEL_PROBES)
            .map(|probe| ir_magnitude(&long, LEVEL_BAND_HZ[0] * ratio.powi(probe as i32), sample_rate).powi(2))
            .sum::<f32>()
            / LEVEL_PROBES as f32;
        let long_level_db = 0.5 * gain_to_db(long_level);

        let mut worst = (0.0f32, 0.0);
        for probe in 0..probes {
            let freq_hz = 60.0 * (8000.0f32 / 60.0).powf(probe as f32 / (probes - 1) as f32);
            let off = gain_to_db(ir.magnitude(freq_hz, sample_rate))
                - (gain_to_db(ir_magnitude(&long, freq_hz, sample_rate)) - long_level_db);
            if off.abs() > worst.0.abs() {
                worst = (off, freq_hz);
            }
        }
        worst
    }

    /// Time the cabinet takes per sample, in ns: the fastest of five runs over a second of noise
    fn convolution_ns(ir: &CabIr) -> f64 {
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(192000.0);
        cabinet.set_ir(ir);
        let mut noise = Noise::new(11);
        let input: Vec<f32> = (0..48_000).map(|_| 0.1 * noise.next()).collect();
        (0..5)
            .map(|_| {
                let mut block = input.clone();
                let start = std::time::Instant::now();
                block.chunks_mut(32).for_each(|chunk| cabinet.process(chunk));
                std::hint::black_box(&block);
                start.elapsed().as_secs_f64() * 1e9 / block.len() as f64
            })
            .fold(f64::MAX, f64::min)
    }

    /// Prints how far each cabinet is from the long impulse response it was tuned as, at
    /// every sample rate, how long its own impulse response is, and what the convolution
    /// costs per sample and per tap:
    ///   cargo test -p amp --release cab_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn cab_report() {
        println!("Time of the convolution per sample, filter included, and per tap");
        for amp in Amp::ALL {
            print!("{:<8}", amp.model().name);
            for sample_rate in [48000.0, 192000.0] {
                let ir = design(amp, sample_rate);
                let time = convolution_ns(&ir);
                print!("{:>8} taps{:>7.1} ns{:>7.3} ns/tap", ir.taps.len(), time, time / ir.taps.len() as f64);
            }
            println!();
        }
        for taps in [240, 960, 3840] {
            let time = convolution_ns(&CabIr::plain(&vec![0.01; taps]));
            println!("{:<8}{:>8} taps{:>7.1} ns{:>7.3} ns/tap", "plain", taps, time, time / taps as f64);
        }
        println!();

        println!("Largest difference from the 20 ms impulse response, 60 Hz to 8 kHz, in dB");
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 88200.0, 96000.0, 192000.0] {
                let (off, freq_hz) = off_the_long_response(amp, sample_rate, 600);
                let ir = design(amp, sample_rate);
                println!(
                    "{:<8}{:>8} Hz{:>6} taps (of {}){:>7.2} dB at {:>5.0} Hz",
                    amp.model().name,
                    sample_rate,
                    ir.taps.len(),
                    ir_len(sample_rate),
                    off,
                    freq_hz
                );
            }
        }
    }

    #[test]
    fn test_cabinet_stays_within_half_a_db_of_the_long_impulse_response() {
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0] {
                let (off, freq_hz) = off_the_long_response(amp, sample_rate, 150);
                assert!(off.abs() < 0.5, "{:?} at {} Hz: {:.2} dB at {:.0} Hz", amp, sample_rate, off, freq_hz);
            }
        }
    }

    #[test]
    fn test_impulse_response_is_at_most_half_the_longest() {
        for amp in Amp::ALL {
            for sample_rate in [48000.0, 192000.0] {
                let ir = design(amp, sample_rate);
                assert!(ir.taps.len() * 2 <= ir_len(sample_rate), "{:?}: {} taps", amp, ir.taps.len());
                assert!(ir.lows != BiquadCoeffs::IDENTITY);
            }
        }
    }

    #[test]
    fn test_cabinet_plays_what_its_design_says() {
        // The filter behind the convolution included
        let sample_rate = 48000.0;
        for amp in Amp::ALL {
            let ir = design(amp, sample_rate);
            let response = impulse_response(&ir, sample_rate, 9600);
            for freq in [60.0, 80.0, 100.0, 150.0, 1000.0, 3000.0] {
                let played = to_db(ir_magnitude(&response, freq, sample_rate));
                let designed = to_db(ir.magnitude(freq, sample_rate));
                assert!((played - designed).abs() < 0.05, "{:?} at {} Hz: {:.2} dB, {:.2} dB", amp, freq, played, designed);
            }
        }
    }

    #[test]
    fn test_ir_is_finite_and_levelled() {
        for amp in Amp::ALL {
            for sample_rate in [44100.0, 48000.0, 96000.0, 192000.0] {
                let ir = design(amp, sample_rate);
                assert!(ir.taps.len() <= ir_len(sample_rate));
                assert!(ir.taps.iter().all(|s| s.is_finite()));
                assert_eq!(*ir.taps.last().unwrap(), 0.0);

                let ratio = (LEVEL_BAND_HZ[1] / LEVEL_BAND_HZ[0]).powf(1.0 / 99.0);
                let power: f32 = (0..100)
                    .map(|i| ir.magnitude(LEVEL_BAND_HZ[0] * ratio.powi(i), sample_rate).powi(2))
                    .sum::<f32>()
                    / 100.0;
                assert!(to_db(power.sqrt()).abs() < 0.5, "{:?} at {}: {:.2} dB", amp, sample_rate, to_db(power.sqrt()));
            }
        }
    }

    #[test]
    fn test_ir_length_follows_the_sample_rate_up_to_the_limit() {
        assert_eq!(ir_len(48000.0), 960);
        assert_eq!(ir_len(192000.0), MAX_IR_LEN);
        assert_eq!(ir_len(384000.0), MAX_IR_LEN);
        assert_eq!(user_ir_len(48000.0), 1920);
        assert_eq!(user_ir_len(192000.0), MAX_USER_IR_LEN);
        assert_eq!(user_ir_len(384000.0), MAX_USER_IR_LEN);
    }

    #[test]
    fn test_ir_energy_is_at_the_start() {
        for amp in Amp::ALL {
            let sample_rate = 48000.0;
            let ir = impulse_response(&design(amp, sample_rate), sample_rate, 4800);
            let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
            let early = energy(&ir[..(0.005 * sample_rate) as usize]) / energy(&ir);
            assert!(early > 0.9, "{:?}: {:.2} of the energy in the first 5 ms", amp, early);
        }
    }

    #[test]
    fn test_ir_sounds_the_same_at_every_sample_rate() {
        for amp in Amp::ALL {
            let reference = design(amp, 48000.0);
            for sample_rate in [44100.0, 96000.0, 192000.0] {
                let ir = design(amp, sample_rate);
                for freq in PROBES_HZ {
                    let difference =
                        to_db(ir.magnitude(freq, sample_rate) / reference.magnitude(freq, 48000.0));
                    assert!(
                        difference.abs() < 1.0,
                        "{:?} at {} Hz and {} Hz rate: {:.2} dB",
                        amp,
                        freq,
                        sample_rate,
                        difference
                    );
                }
            }
        }
    }

    #[test]
    fn test_ir_has_the_shape_of_a_guitar_speaker() {
        for amp in Amp::ALL {
            let ir = design(amp, 48000.0);
            let level = |freq: f32| to_db(ir.magnitude(freq, 48000.0));
            let mids = level(1000.0);
            assert!(level(30.0) < mids - 15.0, "{:?} keeps sub bass", amp);
            assert!(level(10000.0) < mids - 20.0, "{:?} keeps fizz", amp);
            assert!(level(amp.model().cab.resonance_hz) > mids - 8.0, "{:?} has no low end", amp);
        }
    }

    #[test]
    fn test_convolution_plays_back_the_impulse_response() {
        let sample_rate = 48000.0;
        let ir = design(Amp::ALL[0], sample_rate).taps;
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&CabIr::plain(&ir));

        let mut block = vec![0.0; ir.len() + 100];
        block[0] = 1.0;
        cabinet.process(&mut block);
        for (index, &expected) in ir.iter().enumerate() {
            assert!((block[index] - expected).abs() < 1e-6, "Tap {}", index);
        }
        assert!(block[ir.len()..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn test_convolution_matches_the_plain_sum() {
        let sample_rate = 44100.0;
        let ir = design(Amp::ALL[0], sample_rate).taps;
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&CabIr::plain(&ir));

        let mut noise = Noise::new(7);
        let input: Vec<f32> = (0..3000).map(|_| noise.next()).collect();
        let mut output = input.clone();
        // Uneven blocks, so the ring wraps at different places within them
        for block in output.chunks_mut(37) {
            cabinet.process(block);
        }

        for index in (0..input.len()).step_by(97) {
            let expected: f64 = (0..=index.min(ir.len() - 1))
                .map(|tap| ir[tap] as f64 * input[index - tap] as f64)
                .sum();
            assert!((output[index] as f64 - expected).abs() < 1e-4, "Sample {}", index);
        }
    }

    #[test]
    fn test_swap_crossfades_without_a_click() {
        let sample_rate = 48000.0;
        let ir = design(Amp::ALL[0], sample_rate);
        let quieter = CabIr {
            taps: ir.taps.iter().map(|s| s * 0.25).collect(),
            lows: ir.lows,
        };
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(sample_rate);
        cabinet.set_ir(&ir);

        let mut block = sine(500.0, 0.5, sample_rate, 9600);
        cabinet.process(&mut block[..4800]);
        cabinet.swap_ir(&quieter);
        cabinet.process(&mut block[4800..]);

        let before = rms(&block[2400..4800]);
        let after = rms(&block[7200..]);
        assert!((after / before - 0.25).abs() < 0.01, "Level ratio: {}", after / before);

        let largest_step = block[2400..].windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max);
        let own_step = block[2400..4800].windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max);
        assert!(largest_step < own_step * 1.05, "Step of {} against {}", largest_step, own_step);
    }

    #[test]
    fn test_short_and_long_responses_fit() {
        let mut cabinet = Cabinet::new();
        cabinet.set_sample_rate(48000.0);

        cabinet.set_ir(&CabIr::plain(&[0.5]));
        let mut block = [1.0, 0.0, 0.0];
        cabinet.process(&mut block);
        assert_eq!(block, [0.5, 0.0, 0.0]);

        cabinet.reset();
        cabinet.set_ir(&CabIr::plain(&vec![0.1; MAX_USER_IR_LEN * 2]));
        let mut long = vec![1.0; 4000];
        cabinet.process(&mut long);
        assert!((long[3999] - 0.1 * user_ir_len(48000.0) as f32).abs() < 1e-2);
    }

    fn voiced(amp: Amp, mic: f32, resonance: f32, input: &[f32]) -> Vec<f32> {
        let mut voicing = CabVoicing::new();
        voicing.set(&amp.model().cab, mic, resonance, 48000.0, 0);
        let mut output = input.to_vec();
        voicing.process(&mut output);
        output
    }

    #[test]
    fn test_voicing_at_the_centre_changes_nothing() {
        let mut noise = Noise::new(5);
        let input: Vec<f32> = (0..4800).map(|_| noise.next()).collect();
        for amp in Amp::ALL {
            assert_eq!(voiced(amp, 0.5, 0.5, &input), input);
        }

        // Also after a dial has been somewhere else
        let mut voicing = CabVoicing::new();
        voicing.set(&Amp::Brol.model().cab, 0.9, 0.1, 48000.0, 0);
        voicing.process(&mut input.clone());
        voicing.set(&Amp::Brol.model().cab, 0.5, 0.5, 48000.0, 0);
        let mut output = input.clone();
        voicing.process(&mut output);
        assert_eq!(output, input);

        // And when the dials glide back: the same as the input once they have arrived, and
        // no step on the way there or on the way out again
        let largest_step = |samples: &[f32]| samples.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max);
        let tone = sine(300.0, 0.5, 48000.0, 4800);
        voicing.set(&Amp::Brol.model().cab, 0.9, 0.1, 48000.0, 0);
        let mut output = tone.clone();
        voicing.process(&mut output[..2400]);
        voicing.set(&Amp::Brol.model().cab, 0.5, 0.5, 48000.0, 32);
        voicing.process(&mut output[2400..3600]);
        assert_eq!(output[2432..3600], tone[2432..3600]);
        voicing.set(&Amp::Brol.model().cab, 0.6, 0.6, 48000.0, 32);
        voicing.process(&mut output[3600..]);
        assert!(largest_step(&output[1200..]) < 1.3 * largest_step(&output[1200..2400]));
        assert!(output[3700..] != tone[3700..]);
    }

    #[test]
    fn test_mic_dial_moves_the_top_and_a_little_of_the_low_end() {
        for amp in Amp::ALL {
            let cab = &amp.model().cab;
            let change = |mic: f32, freq: f32| CabVoicing::response_db(cab, mic, 0.5, freq, 48000.0);
            assert!(change(0.0, 4000.0) < -4.0, "{:?} dark at 4 kHz: {:.1} dB", amp, change(0.0, 4000.0));
            assert!(change(1.0, 5000.0) > 4.0, "{:?} bright at 5 kHz: {:.1} dB", amp, change(1.0, 5000.0));
            assert!((0.3..2.5).contains(&change(0.0, 200.0)), "{:?} dark at 200 Hz: {:.1} dB", amp, change(0.0, 200.0));
            assert!((-2.5..-0.3).contains(&change(1.0, 100.0)), "{:?} bright at 100 Hz: {:.1} dB", amp, change(1.0, 100.0));
            // Steady across the dial
            let steps: Vec<f32> = (0..=10).map(|step| change(step as f32 * 0.1, 4000.0)).collect();
            assert!(steps.windows(2).all(|pair| pair[1] > pair[0]), "{:?}: {:?}", amp, steps);
        }
    }

    #[test]
    fn test_resonance_dial_moves_the_thump_only() {
        for amp in Amp::ALL {
            let cab = &amp.model().cab;
            let change = |resonance: f32, freq: f32| CabVoicing::response_db(cab, 0.5, resonance, freq, 48000.0);
            assert!((change(1.0, cab.resonance_hz) - THUMP_DB).abs() < 0.1);
            assert!((change(0.0, cab.resonance_hz) + THUMP_DB).abs() < 0.1);
            assert!(change(1.0, 1000.0).abs() < 0.3 && change(0.0, 1000.0).abs() < 0.3);
        }
    }

    #[test]
    fn test_voicing_filters_do_what_their_response_says() {
        let cab = &Amp::Brol.model().cab;
        for (mic, resonance, freq) in [(0.0, 0.5, 4000.0), (1.0, 0.5, 4000.0), (0.5, 1.0, 95.0), (0.2, 0.1, 300.0)] {
            let input = sine(freq, 0.5, 48000.0, 24_000);
            let output = voiced(Amp::Brol, mic, resonance, &input);
            let measured = to_db(rms(&output[12_000..]) / rms(&input[12_000..]));
            let expected = CabVoicing::response_db(cab, mic, resonance, freq, 48000.0);
            assert!((measured - expected).abs() < 0.1, "{} Hz: {:.2} dB, expected {:.2} dB", freq, measured, expected);
        }
    }
}
