pub mod filters;
pub mod oversample;
pub mod shaper;

pub fn db_to_gain(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

pub fn lerp(from: f32, to: f32, position: f32) -> f32 {
    from + (to - from) * position
}

/// Per-sample share of the distance a one-pole follower covers, for a time constant
pub fn smoothing_coeff(time_ms: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
}

/// A dial curve: straight lines through evenly spaced points, `position` from 0.0 to 1.0
pub fn curve(points: &[f32], position: f32) -> f32 {
    let scaled = position.clamp(0.0, 1.0) * (points.len() - 1) as f32;
    let index = (scaled as usize).min(points.len() - 2);
    lerp(points[index], points[index + 1], scaled - index as f32)
}

/// A value that moves to its target in a straight line, one step per sample, so a gain
/// that is set once per chunk does not leave a step in the signal
#[derive(Clone, Copy)]
pub struct Ramp {
    value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Ramp {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }

    pub fn set_target(&mut self, target: f32, steps: u32) {
        self.target = target;
        if steps == 0 || target == self.value {
            self.snap();
        } else {
            self.step = (target - self.value) / steps as f32;
            self.remaining = steps;
        }
    }

    /// Jumps to the target
    pub fn snap(&mut self) {
        self.value = self.target;
        self.remaining = 0;
    }

    pub fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.value = if self.remaining == 0 { self.target } else { self.value + self.step };
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_curve_passes_through_its_points() {
        let points = [1.0, 3.0, 2.0];
        assert_eq!(curve(&points, 0.0), 1.0);
        assert_eq!(curve(&points, 0.5), 3.0);
        assert_eq!(curve(&points, 1.0), 2.0);
        assert!((curve(&points, 0.25) - 2.0).abs() < 1e-6);
        assert_eq!(curve(&points, 2.0), 2.0);
    }

    #[test]
    fn test_ramp_reaches_target_exactly_and_stays() {
        let mut ramp = Ramp::new(0.0);
        ramp.set_target(1.0, 7);

        let values: Vec<f32> = (0..10).map(|_| ramp.next()).collect();
        assert!(values.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(values[5] < 1.0);
        assert_eq!(values[6], 1.0);
        assert_eq!(values[9], 1.0);
    }
}
