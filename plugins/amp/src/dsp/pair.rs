//! Two double precision numbers worked on as one: what the clippers cost is their
//! divisions and square roots, and the processor does two of either in the time of one
//! when they are handed to it together. Every operation gives each half exactly what it
//! gives the number alone, so code written with pairs is the same to the bit as code
//! written sample by sample.
//!
//! On x86-64 a pair is one SSE2 register (every x86-64 processor has those). Anywhere else,
//! and in builds that are not optimised, where every one of those instructions would be a
//! function call, it is two numbers side by side.

use std::ops::{Add, Div, Mul, Sub};

#[cfg(all(target_arch = "x86_64", not(debug_assertions)))]
mod wide {
    // SAFETY, for every block in here: the functions called need SSE2 and nothing else,
    // and SSE2 is part of x86-64. They take and give values, no pointers
    use std::arch::x86_64::*;

    pub type Inner = __m128d;

    #[inline(always)]
    pub fn new(first: f64, second: f64) -> Inner {
        unsafe { _mm_set_pd(second, first) }
    }

    #[inline(always)]
    pub fn first(pair: Inner) -> f64 {
        unsafe { _mm_cvtsd_f64(pair) }
    }

    #[inline(always)]
    pub fn second(pair: Inner) -> f64 {
        unsafe { _mm_cvtsd_f64(_mm_unpackhi_pd(pair, pair)) }
    }

    #[inline(always)]
    pub fn add(a: Inner, b: Inner) -> Inner {
        unsafe { _mm_add_pd(a, b) }
    }

    #[inline(always)]
    pub fn sub(a: Inner, b: Inner) -> Inner {
        unsafe { _mm_sub_pd(a, b) }
    }

    #[inline(always)]
    pub fn mul(a: Inner, b: Inner) -> Inner {
        unsafe { _mm_mul_pd(a, b) }
    }

    #[inline(always)]
    pub fn div(a: Inner, b: Inner) -> Inner {
        unsafe { _mm_div_pd(a, b) }
    }

    #[inline(always)]
    pub fn sqrt(pair: Inner) -> Inner {
        unsafe { _mm_sqrt_pd(pair) }
    }

    #[inline(always)]
    pub fn abs(pair: Inner) -> Inner {
        unsafe { _mm_andnot_pd(_mm_set1_pd(-0.0), pair) }
    }

    /// The second half of `a` with the first half of `b` behind it
    #[inline(always)]
    pub fn follow(a: Inner, b: Inner) -> Inner {
        unsafe { _mm_shuffle_pd::<1>(a, b) }
    }
}

#[cfg(not(all(target_arch = "x86_64", not(debug_assertions))))]
mod wide {
    pub type Inner = [f64; 2];

    #[inline(always)]
    pub fn new(first: f64, second: f64) -> Inner {
        [first, second]
    }

    #[inline(always)]
    pub fn first(pair: Inner) -> f64 {
        pair[0]
    }

    #[inline(always)]
    pub fn second(pair: Inner) -> f64 {
        pair[1]
    }

    #[inline(always)]
    pub fn add(a: Inner, b: Inner) -> Inner {
        [a[0] + b[0], a[1] + b[1]]
    }

    #[inline(always)]
    pub fn sub(a: Inner, b: Inner) -> Inner {
        [a[0] - b[0], a[1] - b[1]]
    }

    #[inline(always)]
    pub fn mul(a: Inner, b: Inner) -> Inner {
        [a[0] * b[0], a[1] * b[1]]
    }

    #[inline(always)]
    pub fn div(a: Inner, b: Inner) -> Inner {
        [a[0] / b[0], a[1] / b[1]]
    }

    #[inline(always)]
    pub fn sqrt(pair: Inner) -> Inner {
        [pair[0].sqrt(), pair[1].sqrt()]
    }

    #[inline(always)]
    pub fn abs(pair: Inner) -> Inner {
        [pair[0].abs(), pair[1].abs()]
    }

    #[inline(always)]
    pub fn follow(a: Inner, b: Inner) -> Inner {
        [a[1], b[0]]
    }
}

/// Two numbers, a first and a second
#[derive(Clone, Copy)]
pub struct Pair(wide::Inner);

impl Pair {
    #[inline(always)]
    pub fn new(first: f64, second: f64) -> Self {
        Self(wide::new(first, second))
    }

    /// The same number twice
    #[inline(always)]
    pub fn both(value: f64) -> Self {
        Self::new(value, value)
    }

    #[inline(always)]
    pub fn first(self) -> f64 {
        wide::first(self.0)
    }

    #[inline(always)]
    pub fn second(self) -> f64 {
        wide::second(self.0)
    }

    #[inline(always)]
    pub fn sqrt(self) -> Self {
        Self(wide::sqrt(self.0))
    }

    #[inline(always)]
    pub fn abs(self) -> Self {
        Self(wide::abs(self.0))
    }

    /// The pair one place earlier in a row of numbers that ends with this pair and has
    /// `before` in front of it: the second of `before`, then the first of this one
    #[inline(always)]
    pub fn after(self, before: Self) -> Self {
        Self(wide::follow(before.0, self.0))
    }
}

impl Add for Pair {
    type Output = Self;

    #[inline(always)]
    fn add(self, other: Self) -> Self {
        Self(wide::add(self.0, other.0))
    }
}

impl Sub for Pair {
    type Output = Self;

    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        Self(wide::sub(self.0, other.0))
    }
}

impl Mul for Pair {
    type Output = Self;

    #[inline(always)]
    fn mul(self, other: Self) -> Self {
        Self(wide::mul(self.0, other.0))
    }
}

impl Div for Pair {
    type Output = Self;

    #[inline(always)]
    fn div(self, other: Self) -> Self {
        Self(wide::div(self.0, other.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_each_half_gets_what_the_number_alone_gets() {
        let values = [0.0, -0.0, 1.0, -1.5, 3.25e-7, 1e300, -7.123456789e-12, 0.1, 2.0 / 3.0];
        for &a in &values {
            for &b in &values {
                let (pair, other) = (Pair::new(a, b), Pair::new(b, a));
                let same = |pair: Pair, first: f64, second: f64| {
                    pair.first().to_bits() == first.to_bits() && pair.second().to_bits() == second.to_bits()
                };
                assert!(same(pair + other, a + b, b + a));
                assert!(same(pair - other, a - b, b - a));
                assert!(same(pair * other, a * b, b * a));
                assert!(same(pair.abs(), a.abs(), b.abs()));
                assert!(same(pair.after(other), a, a));
                assert!(same(Pair::both(a), a, a));
                if a != 0.0 && b != 0.0 {
                    assert!(same(pair / other, a / b, b / a));
                }
                if a >= 0.0 && b >= 0.0 {
                    assert!(same(pair.sqrt(), a.sqrt(), b.sqrt()));
                }
            }
        }
    }
}
