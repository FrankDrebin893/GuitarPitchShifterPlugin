//! Signals and measurements shared by the tests

use crate::dsp::filters::{Biquad, BiquadCoeffs};
use std::f64::consts::TAU;
use std::path::Path;

pub fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / samples.len() as f64).sqrt() as f32
}

pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |max, s| max.max(s.abs()))
}

pub fn to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-10).log10()
}

/// Scales a signal to an RMS level in dBFS
pub fn set_rms_db(samples: &mut [f32], level_db: f32) {
    let gain = 10.0f32.powf(level_db / 20.0) / rms(samples).max(1e-10);
    for sample in samples.iter_mut() {
        *sample *= gain;
    }
}

pub fn sine(freq_hz: f32, level: f32, sample_rate: f32, len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| (level as f64 * (i as f64 * TAU * freq_hz as f64 / sample_rate as f64).sin()) as f32)
        .collect()
}

/// White noise in -1.0..1.0, the same on every run
pub struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    pub fn next(&mut self) -> f32 {
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.state >> 9) as f32 / (1 << 22) as f32 - 1.0
    }
}

/// The harmonics of a tone up to half the sample rate
pub fn harmonics_of(freq_hz: f32, sample_rate: f32) -> Vec<f64> {
    (1..)
        .map(|harmonic| freq_hz as f64 * harmonic as f64)
        .take_while(|&freq| freq < sample_rate as f64 * 0.499)
        .collect()
}

/// Fits a sine at each of the given frequencies by least squares and takes it out. Returns
/// the level of each one, and the energy of what is left relative to the total, in dB.
/// For a clipped sine and its harmonics, what is left is aliasing and noise. A constant
/// offset is taken out first and counts as neither
pub fn fit_partials(signal: &[f32], sample_rate: f32, partials: &[f64]) -> (Vec<f64>, f64) {
    let len = signal.len();
    let window: Vec<f64> = (0..len).map(|i| 0.5 - 0.5 * (TAU * i as f64 / len as f64).cos()).collect();
    let mean = signal.iter().zip(&window).map(|(&s, w)| s as f64 * w).sum::<f64>() / window.iter().sum::<f64>();
    let mut residual: Vec<f64> = signal.iter().map(|&s| s as f64 - mean).collect();
    let energy = |samples: &[f64]| -> f64 { samples.iter().zip(&window).map(|(s, w)| w * s * s).sum() };
    let total = energy(&residual);

    let mut fitted = vec![(0.0, 0.0); partials.len()];
    for _ in 0..2 {
        for (index, &frequency) in partials.iter().enumerate() {
            // A rotating phasor instead of a sine and cosine per sample
            let omega = TAU * frequency / sample_rate as f64;
            let (step_sin, step_cos) = omega.sin_cos();
            let (mut cc, mut ss, mut cs, mut yc, mut ys) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let (mut sin, mut cos) = (0.0, 1.0);
            for i in 0..len {
                cc += window[i] * cos * cos;
                ss += window[i] * sin * sin;
                cs += window[i] * cos * sin;
                yc += window[i] * residual[i] * cos;
                ys += window[i] * residual[i] * sin;
                (sin, cos) = (sin * step_cos + cos * step_sin, cos * step_cos - sin * step_sin);
            }
            let det = cc * ss - cs * cs;
            let a = (yc * ss - ys * cs) / det;
            let b = (ys * cc - yc * cs) / det;
            let (mut sin, mut cos) = (0.0, 1.0);
            for sample in residual.iter_mut() {
                *sample -= a * cos + b * sin;
                (sin, cos) = (sin * step_cos + cos * step_sin, cos * step_cos - sin * step_sin);
            }
            fitted[index].0 += a;
            fitted[index].1 += b;
        }
    }

    let levels = fitted.iter().map(|(a, b)| (a * a + b * b).sqrt()).collect();
    (levels, 10.0 * (energy(&residual) / total.max(1e-300)).max(1e-20).log10())
}

/// Harmonic distortion of a sine after processing: its harmonics relative to its
/// fundamental, in dB
pub fn thd_db(signal: &[f32], sample_rate: f32, freq_hz: f32) -> f32 {
    let (levels, _) = fit_partials(signal, sample_rate, &harmonics_of(freq_hz, sample_rate));
    let harmonics: f64 = levels[1..].iter().map(|level| level * level).sum();
    to_db((harmonics.sqrt() / levels[0].max(1e-12)) as f32)
}

/// Level of a signal at one frequency (the peak level of a sine there)
pub fn level_at(signal: &[f32], sample_rate: f32, freq_hz: f32) -> f32 {
    fit_partials(signal, sample_rate, &[freq_hz as f64]).0[0] as f32
}

/// Edges of the bands `band_levels_db` reports, in Hz
pub const BAND_EDGES_HZ: [f32; 8] = [62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

/// Level in each octave band between `BAND_EDGES_HZ`, in dB relative to the whole signal
pub fn band_levels_db(signal: &[f32], sample_rate: f32) -> Vec<f32> {
    BAND_EDGES_HZ
        .windows(2)
        .map(|edges| band_level_db(signal, sample_rate, Some(edges[0]), Some(edges[1])))
        .collect()
}

/// Level of what a signal has between two frequencies, in dB relative to the whole signal.
/// `None` leaves that side open
pub fn band_level_db(signal: &[f32], sample_rate: f32, from_hz: Option<f32>, to_hz: Option<f32>) -> f32 {
    let mut filters = [Biquad::new(); 4];
    if let Some(from_hz) = from_hz {
        filters[0].set(BiquadCoeffs::highpass(from_hz, 0.541, sample_rate));
        filters[1].set(BiquadCoeffs::highpass(from_hz, 1.307, sample_rate));
    }
    if let Some(to_hz) = to_hz {
        filters[2].set(BiquadCoeffs::lowpass(to_hz, 0.541, sample_rate));
        filters[3].set(BiquadCoeffs::lowpass(to_hz, 1.307, sample_rate));
    }
    let band: Vec<f32> = signal
        .iter()
        .map(|&sample| filters.iter_mut().fold(sample as f64, |signal, filter| filter.process(signal)) as f32)
        .collect();
    to_db(rms(&band) / rms(signal).max(1e-10))
}

/// Edges of the bands `tight_levels_db` reports, in Hz: sub lows, low mids (where mud
/// gathers), mids, upper mids, fizz
pub const TIGHT_EDGES_HZ: [f32; 4] = [100.0, 400.0, 1600.0, 6400.0];

/// Level below, between and above `TIGHT_EDGES_HZ`, in dB relative to the whole signal
pub fn tight_levels_db(signal: &[f32], sample_rate: f32) -> Vec<f32> {
    (0..=TIGHT_EDGES_HZ.len())
        .map(|band| {
            let from_hz = band.checked_sub(1).map(|index| TIGHT_EDGES_HZ[index]);
            band_level_db(signal, sample_rate, from_hz, TIGHT_EDGES_HZ.get(band).copied())
        })
        .collect()
}

const PICK_HZ: f32 = 2500.0;

// A passive pickup into a cable: a low-pass with a small resonant peak
const PICKUP_HZ: f32 = 3200.0;
const PICKUP_Q: f32 = 1.3;

/// How a string is struck
#[derive(Clone, Copy)]
pub struct Pluck {
    /// Seconds to fade by 60 dB
    pub decay_s: f32,
    /// 0.0 keeps the highs ringing, 0.5 loses them fast (a palm mute)
    pub damping: f32,
    /// Where the pick hits, as a share of the string length from the bridge
    pub pick_position: f32,
}

impl Pluck {
    pub const OPEN: Pluck = Pluck {
        decay_s: 4.0,
        damping: 0.18,
        pick_position: 0.13,
    };
    pub const MUTED: Pluck = Pluck {
        decay_s: 0.25,
        damping: 0.5,
        pick_position: 0.08,
    };
}

/// One plucked string as a pickup hears it: a delay line with losses, excited by a noise
/// burst that is combed for the pick position, through a low-pass with a resonance
pub fn pluck(freq_hz: f32, pluck: Pluck, seed: u32, sample_rate: f32, len: usize) -> Vec<f32> {
    let period = sample_rate / freq_hz;
    // The loop is the delay line, the loss filter (`damping` samples) and a tuning allpass
    let line_len = (period - pluck.damping - 0.1).floor() as usize;
    let fraction = period - pluck.damping - line_len as f32;
    let allpass = (1.0 - fraction) / (1.0 + fraction);
    let loss = 10.0f32.powf(-3.0 / (freq_hz * pluck.decay_s));

    let mut noise = Noise::new(seed.wrapping_mul(7919).wrapping_add(12345));
    // The harmonics of a plucked string fall off from the second one, and the width of
    // the pick takes away what is left at the top
    let tilt = 1.0 - (-std::f32::consts::TAU * 2.0 * freq_hz / sample_rate).exp();
    let softness = 1.0 - (-std::f32::consts::TAU * PICK_HZ / sample_rate).exp();
    let (mut tilted, mut smooth) = (0.0, 0.0);
    let burst: Vec<f32> = (0..line_len)
        .map(|_| {
            tilted += tilt * (noise.next() - tilted);
            smooth += softness * (tilted - smooth);
            smooth
        })
        .collect();
    let comb = ((pluck.pick_position * line_len as f32).round() as usize).max(1);
    let mut line: Vec<f32> = (0..line_len)
        .map(|i| burst[i] - burst[(i + line_len - comb) % line_len])
        .collect();

    let mut pickup = Biquad::new();
    pickup.set(BiquadCoeffs::lowpass(PICKUP_HZ, PICKUP_Q, sample_rate));

    let (mut position, mut last_read, mut last_filtered, mut last_tuned) = (0, 0.0, 0.0, 0.0);
    (0..len)
        .map(|_| {
            let read = line[position];
            let filtered = (1.0 - pluck.damping) * read + pluck.damping * last_read;
            let tuned = allpass * (filtered - last_tuned) + last_filtered;
            last_read = read;
            last_filtered = filtered;
            last_tuned = tuned;
            line[position] = loss * tuned;
            position = (position + 1) % line_len;
            pickup.process(read as f64) as f32
        })
        .collect()
}

/// Adds a note to a recording. It is cut off at `len_s` with a short fade, as when the
/// next note is fretted
fn add_note(recording: &mut [f32], sample_rate: f32, start_s: f32, len_s: f32, freq_hz: f32, how: Pluck, level: f32) {
    let start = (start_s * sample_rate) as usize;
    let len = ((len_s * sample_rate) as usize).min(recording.len().saturating_sub(start));
    let fade = (0.015 * sample_rate) as usize;
    let seed = (freq_hz * 10.0) as u32 + start as u32;
    for (i, sample) in pluck(freq_hz, how, seed, sample_rate, len).iter().enumerate() {
        let gain = ((len - i) as f32 / fade as f32).min(1.0);
        recording[start + i] += sample * gain * level;
    }
}

const DI_PEAK: f32 = 0.9;
const POWER_CHORD_HZ: [f32; 3] = [82.41, 123.47, 164.81];
const OPEN_CHORD_HZ: [f32; 6] = [82.41, 123.47, 164.81, 207.65, 246.94, 329.63];

/// A power chord on the low string, struck again every half second, at -18 dBFS RMS
pub fn power_chords(sample_rate: f32, seconds: f32) -> Vec<f32> {
    let mut recording = vec![0.0; (seconds * sample_rate) as usize];
    let mut start = 0.0;
    while start < seconds {
        for (string, &freq) in POWER_CHORD_HZ.iter().enumerate() {
            add_note(&mut recording, sample_rate, start + string as f32 * 0.004, 0.49, freq, Pluck::OPEN, 1.0);
        }
        start += 0.5;
    }
    set_rms_db(&mut recording, -18.0);
    recording
}

// Roots of the lowest power chords in two drop tunings
const DROP_ROOTS_HZ: [f32; 2] = [65.41, 73.42];
const PALM_MUTE_STEP_S: f32 = 0.15;

/// Palm-muted power chords in a drop tuning, a bar of eight on each of two low roots. At
/// -18 dBFS RMS, or lower if its peaks would not fit
pub fn palm_mutes(sample_rate: f32, seconds: f32) -> Vec<f32> {
    let mut recording = vec![0.0; (seconds * sample_rate) as usize];
    let mut beat = 0;
    while beat as f32 * PALM_MUTE_STEP_S < seconds {
        let root = DROP_ROOTS_HZ[(beat / 8) % DROP_ROOTS_HZ.len()];
        let start = beat as f32 * PALM_MUTE_STEP_S;
        for (string, ratio) in [1.0, 1.4983, 2.0].iter().enumerate() {
            let at = start + string as f32 * 0.003;
            add_note(&mut recording, sample_rate, at, PALM_MUTE_STEP_S - 0.01, root * ratio, Pluck::MUTED, 1.0);
        }
        beat += 1;
    }
    set_rms_db(&mut recording, -18.0);
    fit_peak(&mut recording);
    recording
}

/// What an overdrive pedal in front of the amp does to a signal, roughly: the lows cut, the
/// mids pushed forward, and all of it louder. Peaks are rounded off at full scale
pub fn boosted(signal: &[f32], sample_rate: f32) -> Vec<f32> {
    let mut filters = [Biquad::new(); 2];
    filters[0].set(BiquadCoeffs::highpass(300.0, 0.707, sample_rate));
    filters[1].set(BiquadCoeffs::peak(800.0, 0.7, 6.0, sample_rate));
    signal
        .iter()
        .map(|&sample| {
            let shaped = filters.iter_mut().fold(sample as f64, |signal, filter| filter.process(signal));
            (4.0 * shaped).tanh() as f32
        })
        .collect()
}

/// A direct guitar recording, made up: single notes on the low strings, palm-muted power
/// chords, an open chord that rings out, and a few high notes. At -18 dBFS RMS, or lower
/// if its peaks would not fit
pub fn guitar_di(sample_rate: f32) -> Vec<f32> {
    let mut recording = vec![0.0; (11.0 * sample_rate) as usize];
    let mut time = 0.2;

    for freq in [82.41, 98.0, 110.0, 82.41, 123.47, 146.83, 110.0, 82.41] {
        add_note(&mut recording, sample_rate, time, 0.3, freq, Pluck::OPEN, 1.0);
        time += 0.3;
    }

    time += 0.2;
    for beat in 0..12 {
        let how = if beat % 4 == 3 { Pluck::OPEN } else { Pluck::MUTED };
        for (string, &freq) in POWER_CHORD_HZ.iter().enumerate() {
            add_note(&mut recording, sample_rate, time + string as f32 * 0.003, 0.19, freq, how, 0.8);
        }
        time += 0.2;
    }

    time += 0.2;
    for (string, &freq) in OPEN_CHORD_HZ.iter().enumerate() {
        add_note(&mut recording, sample_rate, time + string as f32 * 0.018, 3.0, freq, Pluck::OPEN, 0.6);
    }
    time += 3.2;

    for freq in [659.26, 783.99, 880.0, 987.77, 880.0, 659.26] {
        add_note(&mut recording, sample_rate, time, 0.35, freq, Pluck::OPEN, 0.9);
        time += 0.35;
    }

    set_rms_db(&mut recording, -18.0);
    fit_peak(&mut recording);
    recording
}

/// Turns a recording down if its peaks are above `DI_PEAK`
fn fit_peak(recording: &mut [f32]) {
    let top = peak(recording);
    if top > DI_PEAK {
        for sample in recording.iter_mut() {
            *sample *= DI_PEAK / top;
        }
    }
}

/// Writes a 16-bit WAV file with one channel per slice
pub fn write_wav(path: &Path, channels: &[&[f32]], sample_rate: u32) {
    let spec = hound::WavSpec {
        channels: channels.len() as u16,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for frame in 0..channels[0].len() {
        for channel in channels {
            writer.write_sample((channel[frame].clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        }
    }
    writer.finalize().unwrap();
}

/// Reads a WAV file and mixes it down to mono
pub fn read_wav_mono(path: &str) -> (Vec<f32>, u32) {
    let mut reader = hound::WavReader::open(path).expect("Could not open the input WAV file");
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / scale).collect()
        }
    };
    let channels = spec.channels as usize;
    let mono = interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    (mono, spec.sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48000.0;

    #[test]
    fn test_fit_finds_levels_and_leaves_the_rest() {
        let mut signal = sine(1000.0, 0.5, SAMPLE_RATE, 24_000);
        for (sample, third) in signal.iter_mut().zip(sine(3000.0, 0.05, SAMPLE_RATE, 24_000)) {
            *sample += third;
        }
        let (levels, residual_db) = fit_partials(&signal, SAMPLE_RATE, &[1000.0, 3000.0]);
        assert!((levels[0] - 0.5).abs() < 1e-3 && (levels[1] - 0.05).abs() < 1e-3);
        assert!(residual_db < -100.0);
        assert!((thd_db(&signal, SAMPLE_RATE, 1000.0) + 20.0).abs() < 0.1);

        // A tone that is not a harmonic stays in the residual: -40 dB below the rest
        for (sample, stray) in signal.iter_mut().zip(sine(1730.0, 0.005, SAMPLE_RATE, 24_000)) {
            *sample += stray;
        }
        let (_, residual_db) = fit_partials(&signal, SAMPLE_RATE, &harmonics_of(1000.0, SAMPLE_RATE));
        assert!((residual_db + 40.0).abs() < 1.0, "Residual: {:.1} dB", residual_db);
    }

    #[test]
    fn test_band_levels_find_the_band() {
        let levels = band_levels_db(&sine(700.0, 0.5, SAMPLE_RATE, 24_000), SAMPLE_RATE);
        assert_eq!(levels.len(), 7);
        assert!(levels[3].abs() < 1.0, "Own band: {:.1} dB", levels[3]);
        assert!(levels[1] < -30.0 && levels[5] < -30.0);

        let tight = tight_levels_db(&sine(700.0, 0.5, SAMPLE_RATE, 24_000), SAMPLE_RATE);
        assert_eq!(tight.len(), 5);
        assert!(tight[2].abs() < 1.0, "Own band: {:.1} dB", tight[2]);
        assert!(tight[0] < -30.0 && tight[4] < -30.0);
    }

    #[test]
    fn test_pluck_is_in_tune_and_decays() {
        for freq in [82.41, 329.63, 987.77] {
            let note = pluck(freq, Pluck::OPEN, 1, SAMPLE_RATE, 48_000);
            let at_pitch = level_at(&note[4800..24_000], SAMPLE_RATE, freq);
            let flat = level_at(&note[4800..24_000], SAMPLE_RATE, freq * 0.94);
            let sharp = level_at(&note[4800..24_000], SAMPLE_RATE, freq * 1.06);
            assert!(at_pitch > 3.0 * flat && at_pitch > 3.0 * sharp, "{} Hz is out of tune", freq);
            assert!(rms(&note[40_000..]) < rms(&note[..8000]));
        }

        let muted = pluck(82.41, Pluck::MUTED, 1, SAMPLE_RATE, 48_000);
        assert!(rms(&muted[24_000..]) < 0.01 * rms(&muted[..4800]));
    }

    #[test]
    fn test_recordings_are_at_their_level() {
        let chords = power_chords(SAMPLE_RATE, 2.0);
        assert!((to_db(rms(&chords)) + 18.0).abs() < 0.1);
        assert!(peak(&chords) < 1.0, "Peak: {}", peak(&chords));

        let mutes = palm_mutes(SAMPLE_RATE, 2.4);
        assert!((-24.0..=-17.9).contains(&to_db(rms(&mutes))), "Level: {:.1} dBFS", to_db(rms(&mutes)));
        assert!(peak(&mutes) <= DI_PEAK * 1.001, "Peak: {}", peak(&mutes));
        // Low roots: more below 100 Hz than the open power chords have
        assert!(tight_levels_db(&mutes, SAMPLE_RATE)[0] > tight_levels_db(&chords, SAMPLE_RATE)[0]);

        let di = guitar_di(SAMPLE_RATE);
        assert!((-21.0..=-17.9).contains(&to_db(rms(&di))), "Level: {:.1} dBFS", to_db(rms(&di)));
        assert!(peak(&di) <= DI_PEAK * 1.001, "Peak: {}", peak(&di));
    }
}
