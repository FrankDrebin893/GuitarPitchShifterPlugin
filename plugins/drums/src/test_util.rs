//! Measurements shared by the tests

use std::f32::consts::TAU;

use crate::dsp::Svf;

pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |max, s| max.max(s.abs()))
}

pub fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// Energy of the sample-to-sample difference relative to the signal energy.
/// 0 for DC, 4 at Nyquist: higher means more high-frequency content.
pub fn brightness(samples: &[f32]) -> f32 {
    let energy: f32 = samples.iter().map(|s| s * s).sum();
    let diff_energy: f32 = samples.windows(2).map(|w| (w[1] - w[0]) * (w[1] - w[0])).sum();
    diff_energy / energy.max(1e-12)
}

/// The part of the signal between two frequencies, 24 dB per octave on each side.
/// A `high_hz` at or above Nyquist leaves the top open; one just under it is cut at 0.45
/// of the sample rate, the highest the filter goes.
pub fn band(samples: &[f32], sample_rate: f32, low_hz: f32, high_hz: f32) -> Vec<f32> {
    const Q: f32 = 0.707;
    let mut output = samples.to_vec();
    for _ in 0..2 {
        let mut highpass = Svf::new();
        highpass.set(low_hz, Q, sample_rate);
        for sample in &mut output {
            *sample = highpass.process(*sample).high;
        }
        if high_hz < sample_rate * 0.5 {
            let mut lowpass = Svf::new();
            lowpass.set(high_hz, Q, sample_rate);
            for sample in &mut output {
                let split = lowpass.process(*sample);
                *sample -= split.band / Q + split.high;
            }
        }
    }
    output
}

pub fn band_rms(samples: &[f32], sample_rate: f32, low_hz: f32, high_hz: f32) -> f32 {
    rms(&band(samples, sample_rate, low_hz, high_hz))
}

/// How evenly the energy between two frequencies is spread, looked at 100 Hz at a time:
/// near 1 for noise, near 0 for a few sines with nothing between them.
pub fn spectrum_fill(samples: &[f32], sample_rate: f32, low_hz: f32, high_hz: f32) -> f32 {
    spectrum_fill_by(samples, sample_rate, low_hz, high_hz, 100.0)
}

/// The same, looked at `slot_hz` at a time. Tones that sit closer together than 100 Hz fill
/// the coarse measure, so a finer one tells a bank of resonators from noise.
pub fn spectrum_fill_by(samples: &[f32], sample_rate: f32, low_hz: f32, high_hz: f32, slot_hz: f32) -> f32 {
    let len = samples.len();
    let windowed: Vec<f64> = samples
        .iter()
        .enumerate()
        .map(|(i, &s)| s as f64 * (0.5 - 0.5 * (TAU as f64 * i as f64 / len as f64).cos()))
        .collect();
    // One sine's power at one frequency (Goertzel)
    let power_at = |freq_hz: f32| {
        let coeff = 2.0 * (TAU as f64 * freq_hz as f64 / sample_rate as f64).cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in &windowed {
            let s0 = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        s1 * s1 + s2 * s2 - coeff * s1 * s2
    };

    let step_hz = sample_rate / len as f32;
    let slots = ((high_hz - low_hz) / slot_hz) as usize;
    let powers: Vec<f64> = (0..slots)
        .map(|slot| {
            let start = low_hz + slot as f32 * slot_hz;
            let points = (slot_hz / step_hz).max(1.0) as usize;
            (0..points).map(|point| power_at(start + point as f32 * step_hz)).sum::<f64>() / points as f64
        })
        .collect();

    let mean = powers.iter().sum::<f64>() / slots as f64;
    let log_mean = powers.iter().map(|p| p.max(1e-30).ln()).sum::<f64>() / slots as f64;
    (log_mean.exp() / mean.max(1e-30)) as f32
}

/// Time of the last sample that is within `db` (negative) of the peak
pub fn decay_time_s(samples: &[f32], sample_rate: f32, db: f32) -> f32 {
    let threshold = peak(samples) * 10.0f32.powf(db / 20.0);
    let last = samples.iter().rposition(|s| s.abs() > threshold).unwrap_or(0);
    last as f32 / sample_rate
}

/// Frequency estimate from the time between the first and last upward zero crossing
pub fn zero_crossing_freq(samples: &[f32], sample_rate: f32) -> f32 {
    let crossings: Vec<f32> = samples
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
        .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
        .collect();
    if crossings.len() < 2 {
        return 0.0;
    }
    (crossings.len() - 1) as f32 * sample_rate / (crossings[crossings.len() - 1] - crossings[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::Noise;

    #[test]
    fn test_band_cuts_the_top_unless_it_is_left_open() {
        let sample_rate = 44100.0;
        let mut noise = Noise::new(1);
        let samples: Vec<f32> = (0..44100).map(|_| noise.next()).collect();

        let open = band_rms(&samples, sample_rate, 100.0, sample_rate);
        let nearly_open = band_rms(&samples, sample_rate, 100.0, 21000.0);
        let half = band_rms(&samples, sample_rate, 100.0, 5000.0);

        assert!(nearly_open < open * 0.99, "{} vs {}", nearly_open, open);
        assert!(half < open * 0.6 && half > open * 0.35, "{} vs {}", half, open);
    }
}
