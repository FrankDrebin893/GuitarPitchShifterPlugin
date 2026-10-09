//! Measurements shared by the tests

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
