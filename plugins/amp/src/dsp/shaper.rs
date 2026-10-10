// Below this input step the antialiased shapers fall back to the plain curve, because the
// difference quotient would divide rounding noise by almost nothing
const MIN_STEP: f64 = 1e-6;

// The same for the second-order shaper, which divides by the step twice
const MIN_STEP_2: f64 = 1e-5;

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

/// Integral of `asym_curve` from zero, and the integral of that
#[cfg(test)]
fn asym_integral(x: f64, positive: f64, negative: f64) -> f64 {
    AsymLimits::new(positive, negative).integral(x)
}

#[cfg(test)]
fn asym_second_integral(x: f64, positive: f64, negative: f64) -> f64 {
    AsymLimits::new(positive, negative).second_integral(x)
}

/// The two limits of `asym_curve` with what the integral needs of them worked out once: a
/// division per sample saved in every gain stage
#[derive(Clone, Copy)]
struct AsymLimits {
    positive: f64,
    negative: f64,
    // 1 / limit and limit squared, for the positive and the negative side
    inverse: [f64; 2],
    square: [f64; 2],
}

impl AsymLimits {
    fn new(positive: f64, negative: f64) -> Self {
        Self {
            positive,
            negative,
            inverse: [1.0 / positive, 1.0 / negative],
            square: [positive * positive, negative * negative],
        }
    }

    fn curve(&self, x: f64) -> f64 {
        asym_curve(x, self.positive, self.negative)
    }

    /// Integral of `asym_curve` from zero
    fn integral(&self, x: f64) -> f64 {
        let side = (x < 0.0) as usize;
        let ratio = x * self.inverse[side];
        self.square[side] * ((1.0 + ratio * ratio).sqrt() - 1.0)
    }

    /// Integral of `integral` from zero
    fn second_integral(&self, x: f64) -> f64 {
        let side = (x < 0.0) as usize;
        let limit = if side == 0 { self.positive } else { self.negative };
        let ratio = x * self.inverse[side];
        let root = (1.0 + ratio * ratio).sqrt();
        // The inverse hyperbolic sine of `ratio`, with the root that is already there
        let arsinh = (ratio.abs() + root).ln().copysign(ratio);
        self.square[side] * (0.5 * (x * root + limit * arsinh) - x)
    }
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

/// `asym_clip`, antialiased, to the first order like the `Averager` or to the second.
///
/// Second order: the output is the curve weighted over the path through the last three
/// inputs (a triangle instead of a box), worked out from the curve's second integral,
/// differenced twice. What folds back is weaker again by about as much as the first order
/// gained, for one sample of delay instead of half, and about four times the work
#[derive(Clone, Copy)]
pub struct AsymClipper {
    limits: AsymLimits,
    second_order: bool,
    // The input one and two samples ago
    last_input: f64,
    earlier_input: f64,
    // The integral at the last input: the first one, or the second in second order
    last_integral: f64,
    // Second order: the last difference quotient of the second integral
    last_quotient: f64,
}

impl AsymClipper {
    pub fn new() -> Self {
        Self {
            limits: AsymLimits::new(1.0, 1.0),
            second_order: false,
            last_input: 0.0,
            earlier_input: 0.0,
            last_integral: 0.0,
            last_quotient: 0.0,
        }
    }

    /// Call `reset` after either of these
    pub fn set_limits(&mut self, positive: f32, negative: f32) {
        self.limits = AsymLimits::new(positive as f64, negative as f64);
    }

    pub fn set_second_order(&mut self, second_order: bool) {
        self.second_order = second_order;
    }

    /// As if the input had stood where `other` saw it last: for taking over from it
    /// while it runs
    pub fn follow(&mut self, other: &FineClipper) {
        self.reset(other.clipper.last_input as f32);
    }

    /// `rest` is the input the clipper sits at in silence (the stage's operating point)
    pub fn reset(&mut self, rest: f32) {
        let rest = rest as f64;
        self.last_input = rest;
        self.earlier_input = rest;
        if self.second_order {
            self.last_integral = self.limits.second_integral(rest);
            self.last_quotient = self.limits.integral(rest);
        } else {
            self.last_integral = self.limits.integral(rest);
        }
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.step(input as f64) as f32
    }

    fn step(&mut self, input: f64) -> f64 {
        let output = if self.second_order { self.second_order(input) } else { self.first_order(input) };
        self.last_input = input;
        output
    }

    fn first_order(&mut self, input: f64) -> f64 {
        let integral = self.limits.integral(input);
        let step = input - self.last_input;
        let output = if step.abs() > MIN_STEP {
            (integral - self.last_integral) / step
        } else {
            self.limits.curve(0.5 * (input + self.last_input))
        };
        self.last_integral = integral;
        output
    }

    fn second_order(&mut self, input: f64) -> f64 {
        let limits = &self.limits;
        let second = limits.second_integral(input);

        let step = input - self.last_input;
        let quotient = if step.abs() > MIN_STEP_2 {
            (second - self.last_integral) / step
        } else {
            limits.integral(0.5 * (input + self.last_input))
        };

        let span = input - self.earlier_input;
        let output = if span.abs() > MIN_STEP_2 {
            2.0 * (quotient - self.last_quotient) / span
        } else {
            // The input turned around, or stands still: the same weighting, written for
            // the point halfway between this input and the one before last
            let middle = 0.5 * (input + self.earlier_input);
            let offset = middle - self.last_input;
            if offset.abs() > MIN_STEP_2 {
                2.0 / offset * (limits.integral(middle) + (self.last_integral - limits.second_integral(middle)) / offset)
            } else {
                limits.curve(0.5 * (middle + self.last_input))
            }
        };

        self.earlier_input = self.last_input;
        self.last_integral = second;
        self.last_quotient = quotient;
        output
    }
}

/// `asym_clip`, antialiased to the second order at twice the rate it is called at: for a
/// stage that is handed square waves, where the second order alone leaves aliasing at
/// about -70 dB whatever is done to the stages around it.
///
/// Each call takes two steps, to halfway between the last input and this one and then to
/// this one, which is the straight line the antialiasing takes the input to follow anyway.
/// The two results and the two before them are weighted 1, 3, 3, 1 and summed. Together
/// that averages the curve over a bell two and a half samples wide in place of the second
/// order's triangle of two, which is what lets through 15 dB less of what would fold
/// back. A quarter of a sample later than the second order, and twice the work
#[derive(Clone, Copy)]
pub struct FineClipper {
    clipper: AsymClipper,
    // The results of the last two half steps, the older one first
    history: [f64; 2],
}

impl FineClipper {
    pub fn new() -> Self {
        let mut clipper = AsymClipper::new();
        clipper.set_second_order(true);
        clipper.reset(0.0);
        Self {
            clipper,
            history: [0.0; 2],
        }
    }

    /// Call `reset` after this
    pub fn set_limits(&mut self, positive: f32, negative: f32) {
        self.clipper.set_limits(positive, negative);
    }

    /// `rest` is the input the clipper sits at in silence (the stage's operating point)
    pub fn reset(&mut self, rest: f32) {
        self.clipper.reset(rest);
        self.history = [self.clipper.limits.curve(rest as f64); 2];
    }

    /// As if the input had stood where `other` saw it last: for taking over from it
    /// while it runs
    pub fn follow(&mut self, other: &AsymClipper) {
        self.reset(other.last_input as f32);
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let input = input as f64;
        let halfway = self.clipper.step(0.5 * (self.clipper.last_input + input));
        let there = self.clipper.step(input);
        let output = 0.125 * (there + 3.0 * (halfway + self.history[1]) + self.history[0]);
        self.history = [halfway, there];
        output as f32
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
            let second_slope =
                (asym_second_integral(x + step, 1.2, 0.8) - asym_second_integral(x - step, 1.2, 0.8)) / (2.0 * step);
            assert!((second_slope - asym_integral(x, 1.2, 0.8)).abs() < 1e-6, "asym, second, at {}", x);
        }
        assert_eq!(asym_second_integral(0.0, 1.2, 0.8), 0.0);
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
        let mut second = second_order_clipper(1.0, 1.6, 0.0);
        let mut power = PowerClipper::new();

        // First order is half a sample late, so compare with the curve at the midpoint.
        // Second order is a whole sample late, once it has seen even steps
        let input = sine(100.0, 3.0, 192_000.0, 4000);
        for (index, pair) in input.windows(2).enumerate() {
            let middle = 0.5 * (pair[0] + pair[1]);
            asym.process(pair[0]);
            second.process(pair[0]);
            power.process(pair[0]);
            let mut asym_next = asym;
            let mut second_next = second;
            let mut power_next = power;
            assert!((asym_next.process(pair[1]) - asym_clip(middle, 1.0, 1.6)).abs() < 1e-3);
            assert!(index < 2 || (second_next.process(pair[1]) - asym_clip(pair[0], 1.0, 1.6)).abs() < 1e-3);
            assert!((power_next.process(pair[1]) - power_clip(middle)).abs() < 1e-3);
        }
    }

    fn second_order_clipper(positive: f32, negative: f32, rest: f32) -> AsymClipper {
        let mut clipper = AsymClipper::new();
        clipper.set_limits(positive, negative);
        clipper.set_second_order(true);
        clipper.reset(rest);
        clipper
    }

    #[test]
    fn test_second_order_clipper_is_exact_on_quiet_and_on_still_signals() {
        // Where the steps are too small to divide by, and where they are just large enough
        for level in [1e-7, 1e-5, 1e-3, 0.1] {
            let mut asym = second_order_clipper(1.0, 1.6, 0.1);
            let input: Vec<f32> = sine(220.0, level, 192_000.0, 4000).iter().map(|s| s + 0.1).collect();
            // From the third sample on: until then the steps from rest are uneven
            for (index, pair) in input.windows(2).enumerate() {
                asym.process(pair[0]);
                if index < 2 {
                    continue;
                }
                let mut next = asym;
                let error = (next.process(pair[1]) - asym_clip(pair[0], 1.0, 1.6)).abs();
                assert!(error < 1e-6 + 1e-3 * level, "Level {}: off by {}", level, error);
            }
        }
        // A step and then nothing: the output settles on the curve
        let mut asym = second_order_clipper(1.0, 1.6, 0.0);
        let outputs: Vec<f32> = (0..4).map(|_| asym.process(2.0)).collect();
        assert!(outputs.iter().all(|s| s.is_finite()));
        assert_eq!(outputs[3], asym_clip(2.0, 1.0, 1.6));
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
        let mut second = second_order_clipper(1.0, 1.6, 0.0);
        let second: Vec<f32> = input.iter().map(|&s| second.process(s)).collect();

        let (plain_db, antialiased_db, second_db) = (folded_db(&plain), folded_db(&antialiased), folded_db(&second));
        assert!(
            antialiased_db < plain_db - 15.0 && second_db < antialiased_db - 10.0,
            "Plain {:.1} dB, antialiased {:.1} dB, to the second order {:.1} dB",
            plain_db,
            antialiased_db,
            second_db
        );
    }

    #[test]
    fn test_silence_gives_silence() {
        let mut asym = AsymClipper::new();
        let mut second = second_order_clipper(1.0, 1.6, 0.0);
        let mut fine = fine_clipper(1.0, 1.6, 0.0);
        let mut power = PowerClipper::new();
        for _ in 0..16 {
            assert_eq!(asym.process(0.0), 0.0);
            assert_eq!(second.process(0.0), 0.0);
            assert_eq!(fine.process(0.0), 0.0);
            assert_eq!(power.process(0.0), 0.0);
        }
    }

    fn fine_clipper(positive: f32, negative: f32, rest: f32) -> FineClipper {
        let mut clipper = FineClipper::new();
        clipper.set_limits(positive, negative);
        clipper.reset(rest);
        clipper
    }

    #[test]
    fn test_fine_clipper_follows_the_curve_a_sample_and_a_quarter_late() {
        // A slow signal, so the curve is close to straight between the samples
        let mut fine = fine_clipper(1.0, 1.6, 0.0);
        let input = sine(100.0, 3.0, 192_000.0, 4000);
        let output: Vec<f32> = input.iter().map(|&s| fine.process(s)).collect();
        for index in 4..input.len() {
            let late = 0.75 * input[index - 1] + 0.25 * input[index - 2];
            assert!((output[index] - asym_clip(late, 1.0, 1.6)).abs() < 1e-3, "At {index}");
        }

        // At rest on a stage's operating point it gives the curve there, from the start,
        // and after a step it settles on the curve at the new value
        let mut fine = fine_clipper(1.2, 0.9, -0.1);
        assert_eq!(fine.process(-0.1), asym_clip(-0.1, 1.2, 0.9));
        let outputs: Vec<f32> = (0..5).map(|_| fine.process(2.0)).collect();
        assert!(outputs.iter().all(|s| s.is_finite()));
        assert_eq!(outputs[4], asym_clip(2.0, 1.2, 0.9));
    }

    #[test]
    fn test_fine_clipper_aliases_less_than_the_second_order_on_a_square_wave() {
        // A sine so far over the limits that it crosses them within a fifth of a sample:
        // what comes out is a square wave, every edge of it one step
        let sample_rate = 176_400.0;
        let freq = 4186.0;
        let input = sine(freq, 80.0, sample_rate, 35_280);
        let harmonics = harmonics_of(freq, sample_rate);
        // Where harmonics 42 to 47 land once folded: all of them can be heard
        let folded: Vec<f64> = (42..=47).map(|n| (n as f64 * freq as f64 - sample_rate as f64).abs()).collect();
        let partials = [harmonics.clone(), folded].concat();
        let folded_db = |output: &[f32]| {
            let (levels, _) = fit_partials(&output[4410..], sample_rate, &partials);
            let energy: f64 = levels[harmonics.len()..].iter().map(|level| level * level).sum();
            10.0 * (energy / (levels[0] * levels[0])).log10()
        };

        let mut second = second_order_clipper(1.0, 1.4, 0.0);
        let second: Vec<f32> = input.iter().map(|&s| second.process(s)).collect();
        let mut fine = fine_clipper(1.0, 1.4, 0.0);
        let fine: Vec<f32> = input.iter().map(|&s| fine.process(s)).collect();
        let (second_db, fine_db) = (folded_db(&second), folded_db(&fine));
        assert!(fine_db < second_db - 10.0, "Second order {second_db:.1} dB, at twice the rate {fine_db:.1} dB");
    }

    #[test]
    fn test_clippers_take_over_from_each_other_where_the_input_stands() {
        // The one that starts while the other runs gives the curve at the last input
        // until it has seen the input move
        let mut second = second_order_clipper(1.0, 1.4, 0.0);
        let mut fine = fine_clipper(1.0, 1.4, 0.0);
        for &sample in &sine(1000.0, 0.8, 176_400.0, 100) {
            second.process(sample);
        }
        let stands_at = second.last_input as f32;
        fine.follow(&second);
        assert_eq!(fine.process(stands_at), asym_clip(stands_at, 1.0, 1.4));

        // The second order knows all it needs after two inputs: from there it is as if
        // it had run all along
        let rest = sine(1000.0, 0.8, 176_400.0, 200);
        let mut alone = second_order_clipper(1.0, 1.4, 0.0);
        let mut taking_over = second_order_clipper(1.0, 1.4, 0.0);
        for (index, &sample) in rest.iter().enumerate() {
            fine.process(sample);
            let expected = alone.process(sample);
            if index == 100 {
                taking_over.follow(&fine);
            }
            if index > 102 {
                assert_eq!(taking_over.process(sample), expected);
            } else if index > 100 {
                taking_over.process(sample);
            }
        }
    }
}
