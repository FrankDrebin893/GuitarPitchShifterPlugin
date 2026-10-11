//! Sample rate conversion of a recording as a whole: for impulse responses, which are made
//! ready once and off the audio thread. Allocates and takes its time

// The resampler: a sinc with this many zero crossings either side, under a Kaiser window
// that keeps what would fold back 90 dB down. Its corner sits this far below half the
// lower of the two rates, so that it has fallen that far there: flat to 0.41 of the rate
const ZERO_CROSSINGS: f64 = 32.0;
const KAISER_BETA: f64 = 9.0;
const CUTOFF_SHARE: f64 = 0.91;

pub fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let phase = std::f64::consts::PI * x;
        phase.sin() / phase
    }
}

/// Modified Bessel function of the first kind and order zero, for the Kaiser window
fn bessel_i0(x: f64) -> f64 {
    let quarter = x * x / 4.0;
    let (mut sum, mut term) = (1.0, 1.0);
    for step in 1..60 {
        term *= quarter / (step * step) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// A signal at another sample rate, as many seconds long. Band-limited to just under half
/// the lower of the two rates. The first sample stays the first: nothing is delayed
pub fn resample(input: &[f32], from_rate: f64, to_rate: f64) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    // Input samples per output sample, and the lower rate as a share of the input's
    let step = from_rate / to_rate;
    let low = (to_rate / from_rate).min(1.0);
    let cutoff = low * CUTOFF_SHARE;
    let half_width = ZERO_CROSSINGS / low;
    let window_scale = 1.0 / bessel_i0(KAISER_BETA);

    let len = (input.len() as f64 / step).ceil() as usize;
    (0..len)
        .map(|index| {
            let centre = index as f64 * step;
            let first = (centre - half_width).ceil().max(0.0) as usize;
            let last = ((centre + half_width).floor() as usize).min(input.len() - 1);
            let mut sum = 0.0;
            for (offset, &sample) in input[first..=last].iter().enumerate() {
                let distance = centre - (first + offset) as f64;
                let position = distance / half_width;
                let window = bessel_i0(KAISER_BETA * (1.0 - position * position).max(0.0).sqrt()) * window_scale;
                sum += sample as f64 * cutoff * sinc(cutoff * distance) * window;
            }
            sum as f32
        })
        .collect()
}
