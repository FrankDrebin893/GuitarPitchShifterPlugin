//! Measurements for the tests and reports of the instruments

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

/// The fundamental of a tone with overtones: the lag at which it best repeats, between two
/// frequencies. Finer than a sample through a parabola over the peak.
pub fn pitch_hz(samples: &[f32], sample_rate: f32, low_hz: f32, high_hz: f32) -> f32 {
    let shortest = (sample_rate / high_hz) as usize;
    let longest = ((sample_rate / low_hz) as usize).min(samples.len() / 2);
    let match_at = |lag: usize| -> f64 {
        let (mut both, mut first, mut second) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..samples.len() - lag {
            let (a, b) = (samples[i] as f64, samples[i + lag] as f64);
            both += a * b;
            first += a * a;
            second += b * b;
        }
        both / (first * second).sqrt().max(1e-30)
    };

    let scores: Vec<f64> = (shortest..=longest).map(match_at).collect();
    let best = scores.iter().fold(f64::MIN, |best, &score| best.max(score));
    // The first peak that is nearly as good as the best: a later one is a multiple of the period
    let Some(index) = (1..scores.len().saturating_sub(1)).find(|&i| {
        scores[i] > best * 0.97 && scores[i] >= scores[i - 1] && scores[i] >= scores[i + 1]
    }) else {
        return 0.0;
    };
    let (before, at, after) = (scores[index - 1], scores[index], scores[index + 1]);
    let offset = 0.5 * (before - after) / (before - 2.0 * at + after).min(-1e-30);
    sample_rate / ((shortest + index) as f64 + offset) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::sine;

    #[test]
    fn test_pitch_of_a_tone_with_a_weak_fundamental() {
        let sample_rate = 48000.0;
        for freq_hz in [41.2f32, 110.0, 246.9] {
            let tone: Vec<f32> = (0..24000)
                .map(|i| {
                    let phase = i as f32 * freq_hz / sample_rate;
                    0.2 * sine(phase) + sine(2.0 * phase) + 0.6 * sine(3.0 * phase)
                })
                .collect();
            let found = pitch_hz(&tone, sample_rate, 30.0, 400.0);
            assert!((found / freq_hz - 1.0).abs() < 0.002, "{} Hz read as {}", freq_hz, found);
        }
    }

    #[test]
    fn test_decay_time_and_zero_crossings() {
        let sample_rate = 44100.0;
        let tone: Vec<f32> = (0..44100)
            .map(|i| {
                sine(i as f32 * 100.0 / sample_rate) * (-(i as f32) / (0.1 * sample_rate)).exp()
            })
            .collect();
        assert!((zero_crossing_freq(&tone, sample_rate) - 100.0).abs() < 0.5);
        // 40 dB is 4.6 time constants
        assert!((decay_time_s(&tone, sample_rate, -40.0) - 0.46).abs() < 0.02);
    }
}
