// Below this input step the antialiased shapers fall back to the plain curve, because the
// difference quotient would divide rounding noise by almost nothing
const MIN_STEP: f64 = 1e-6;

// Where the power clipper reaches its ceiling of 1.0
const POWER_CLIP_CORNER: f64 = 1.5;

// Level where the output clip starts to round off peaks
pub const OUTPUT_CLIP_KNEE: f32 = 0.75;

/// Gain stage curve: unit slope through zero, then bends towards `positive` above and
/// `-negative` below. Smooth everywhere, also where the two halves meet
pub fn asym_clip(x: f32, positive: f32, negative: f32) -> f32 {
    asym_curve(x as f64, positive as f64, negative as f64) as f32
}

fn asym_curve(x: f64, positive: f64, negative: f64) -> f64 {
    let ratio = x / if x >= 0.0 { positive } else { negative };
    x / (1.0 + ratio * ratio).sqrt()
}

/// Integral of `asym_curve` from zero
fn asym_integral(x: f64, positive: f64, negative: f64) -> f64 {
    let limit = if x >= 0.0 { positive } else { negative };
    let ratio = x / limit;
    limit * limit * ((1.0 + ratio * ratio).sqrt() - 1.0)
}

/// Power stage curve: symmetric, nearly straight at low levels, then a firm ceiling at 1.0
#[cfg(test)]
pub fn power_clip(x: f32) -> f32 {
    power_curve(x as f64) as f32
}

fn power_curve(x: f64) -> f64 {
    if x.abs() < POWER_CLIP_CORNER {
        x - x * x * x * (4.0 / 27.0)
    } else {
        x.signum()
    }
}

/// Integral of `power_curve` from zero
fn power_integral(x: f64) -> f64 {
    if x.abs() < POWER_CLIP_CORNER {
        let square = x * x;
        square * 0.5 - square * square / 27.0
    } else {
        x.abs() - 0.5625
    }
}

/// Limiter for the output: leaves everything below the knee untouched, then bends smoothly
/// towards 1.0
pub fn output_clip(x: f32) -> f32 {
    let level = x.abs();
    if level <= OUTPUT_CLIP_KNEE {
        x
    } else {
        let range = 1.0 - OUTPUT_CLIP_KNEE;
        (OUTPUT_CLIP_KNEE + range * ((level - OUTPUT_CLIP_KNEE) / range).tanh()).copysign(x)
    }
}

/// Runs a curve antialiased: the output is the curve's average over the path from the
/// previous input to this one (its integral, differenced). The harmonics that would fold
/// back come out much weaker than by evaluating the curve at single points, for half a
/// sample of delay. Double precision, because the difference of two integrals loses digits
#[derive(Clone, Copy)]
struct Averager {
    last_input: f64,
    last_integral: f64,
}

impl Averager {
    fn new() -> Self {
        Self {
            last_input: 0.0,
            last_integral: 0.0,
        }
    }

    fn process(&mut self, input: f64, integral: f64, curve: impl Fn(f64) -> f64) -> f64 {
        let step = input - self.last_input;
        let output = if step.abs() > MIN_STEP {
            (integral - self.last_integral) / step
        } else {
            curve(0.5 * (input + self.last_input))
        };
        self.last_input = input;
        self.last_integral = integral;
        output
    }
}

/// `asym_clip`, antialiased
#[derive(Clone, Copy)]
pub struct AsymClipper {
    positive: f64,
    negative: f64,
    averager: Averager,
}

impl AsymClipper {
    pub fn new() -> Self {
        Self {
            positive: 1.0,
            negative: 1.0,
            averager: Averager::new(),
        }
    }

    pub fn set_limits(&mut self, positive: f32, negative: f32) {
        self.positive = positive as f64;
        self.negative = negative as f64;
    }

    /// `rest` is the input the clipper sits at in silence (the stage's operating point)
    pub fn reset(&mut self, rest: f32) {
        self.averager = Averager {
            last_input: rest as f64,
            last_integral: asym_integral(rest as f64, self.positive, self.negative),
        };
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let (positive, negative) = (self.positive, self.negative);
        let input = input as f64;
        self.averager
            .process(input, asym_integral(input, positive, negative), |x| asym_curve(x, positive, negative))
            as f32
    }
}

/// `power_clip`, antialiased
#[derive(Clone, Copy)]
pub struct PowerClipper {
    averager: Averager,
}

impl PowerClipper {
    pub fn new() -> Self {
        Self { averager: Averager::new() }
    }

    pub fn reset(&mut self) {
        self.averager = Averager::new();
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let input = input as f64;
        self.averager.process(input, power_integral(input), power_curve) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{fit_partials, harmonics_of, sine};

    #[test]
    fn test_asym_clip_is_bounded_monotonic_and_asymmetric() {
        let mut previous = -10.0;
        for i in -5000..=5000 {
            let output = asym_clip(i as f32 * 0.01, 1.0, 1.6);
            assert!(output > previous, "Not monotonic at {}", i);
            assert!(output < 1.0 && output > -1.6);
            previous = output;
        }
        assert!(asym_clip(50.0, 1.0, 1.6) > 0.99);
        assert!(asym_clip(-50.0, 1.0, 1.6) < -1.59);
        assert_eq!(asym_clip(0.0, 1.0, 1.6), 0.0);
    }

    #[test]
    fn test_curves_are_smooth() {
        // The slope may not jump anywhere: a kink is what aliases most
        let step = 1e-3;
        let slope_jump = |curve: &dyn Fn(f64) -> f64| {
            (-4000..4000)
                .map(|i| {
                    let x = i as f64 * step;
                    ((curve(x + step) - curve(x)) - (curve(x) - curve(x - step))).abs() / step
                })
                .fold(0.0, f64::max)
        };
        assert!(slope_jump(&|x| asym_curve(x, 1.0, 1.6)) < 2e-3);
        assert!(slope_jump(&power_curve) < 2e-3);

        assert!((asym_curve(1e-4, 1.0, 1.6) / 1e-4 - 1.0).abs() < 1e-6);
        assert!((asym_curve(-1e-4, 1.0, 1.6) / -1e-4 - 1.0).abs() < 1e-6);
        assert!((power_curve(1e-4) / 1e-4 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_integrals_match_their_curves() {
        let step = 1e-5;
        for i in -40..=40 {
            let x = i as f64 * 0.1 + 0.013;
            let asym_slope = (asym_integral(x + step, 1.2, 0.8) - asym_integral(x - step, 1.2, 0.8)) / (2.0 * step);
            assert!((asym_slope - asym_curve(x, 1.2, 0.8)).abs() < 1e-6, "asym at {}", x);
            let power_slope = (power_integral(x + step) - power_integral(x - step)) / (2.0 * step);
            assert!((power_slope - power_curve(x)).abs() < 1e-6, "power at {}", x);
        }
    }

    #[test]
    fn test_power_clip_is_nearly_clean_at_half_level_and_bounded() {
        assert!((power_clip(0.5) - 0.5).abs() < 0.02);
        assert_eq!(power_clip(3.0), 1.0);
        assert_eq!(power_clip(-3.0), -1.0);
        assert!((power_clip(1.499) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn test_output_clip_is_clean_below_knee_and_bounded_above() {
        assert_eq!(output_clip(0.5), 0.5);
        assert_eq!(output_clip(-OUTPUT_CLIP_KNEE), -OUTPUT_CLIP_KNEE);

        let mut previous = 0.0;
        for i in 1..1000 {
            let output = output_clip(i as f32 * 0.01);
            assert!(output >= previous && output <= 1.0, "Not monotonic or unbounded at {}", i);
            previous = output;
        }
        assert_eq!(output_clip(-5.0), -output_clip(5.0));
    }

    #[test]
    fn test_antialiased_clippers_follow_their_curves_on_slow_signals() {
        let mut asym = AsymClipper::new();
        asym.set_limits(1.0, 1.6);
        let mut power = PowerClipper::new();

        // Half a sample late, so compare with the curve at the midpoint
        let input = sine(100.0, 3.0, 192_000.0, 4000);
        for pair in input.windows(2) {
            let middle = 0.5 * (pair[0] + pair[1]);
            asym.process(pair[0]);
            power.process(pair[0]);
            let mut asym_next = asym;
            let mut power_next = power;
            assert!((asym_next.process(pair[1]) - asym_clip(middle, 1.0, 1.6)).abs() < 1e-3);
            assert!((power_next.process(pair[1]) - power_clip(middle)).abs() < 1e-3);
        }
    }

    #[test]
    fn test_antialiased_clipper_aliases_less_than_the_plain_curve() {
        let sample_rate = 192_000.0;
        let input = sine(4186.0, 30.0, sample_rate, 38_400);
        let harmonics = harmonics_of(4186.0, sample_rate);
        // Where harmonics 42 to 47 land once folded: all in the audible range
        let folded: Vec<f64> = (42..=47).map(|n| (n as f64 * 4186.0 - sample_rate as f64).abs()).collect();
        let partials = [harmonics.clone(), folded].concat();

        let plain: Vec<f32> = input.iter().map(|&s| asym_clip(s, 1.0, 1.6)).collect();
        let mut clipper = AsymClipper::new();
        clipper.set_limits(1.0, 1.6);
        let antialiased: Vec<f32> = input.iter().map(|&s| clipper.process(s)).collect();

        let folded_db = |output: &[f32]| {
            let (levels, _) = fit_partials(&output[4800..], sample_rate, &partials);
            let energy: f64 = levels[harmonics.len()..].iter().map(|level| level * level).sum();
            10.0 * (energy / (levels[0] * levels[0])).log10()
        };
        let (plain_db, antialiased_db) = (folded_db(&plain), folded_db(&antialiased));
        assert!(
            antialiased_db < plain_db - 15.0,
            "Plain {:.1} dB, antialiased {:.1} dB",
            plain_db,
            antialiased_db
        );
    }

    #[test]
    fn test_silence_gives_silence() {
        let mut asym = AsymClipper::new();
        let mut power = PowerClipper::new();
        for _ in 0..16 {
            assert_eq!(asym.process(0.0), 0.0);
            assert_eq!(power.process(0.0), 0.0);
        }
    }
}
