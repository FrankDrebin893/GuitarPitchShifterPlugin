//! A small fast Fourier transform of real signals, for the long cabinets: radix 2, single
//! precision, sizes that are powers of two. Made once per size (that allocates); the
//! transforms themselves do not allocate.
//!
//! A real signal of `size` samples has `size / 2 + 1` frequencies worth keeping, from zero
//! to half the sample rate. They are kept as two rows, the real parts and the imaginary
//! parts. The transform packs the even and the odd samples into one complex signal of
//! half the size, transforms that, and takes the two apart again.

use std::f64::consts::PI;

pub struct RealFft {
    // Half the number of samples: the size of the complex transform inside
    half: usize,
    // Where each sample of the complex signal goes before the passes
    order: Vec<u16>,
    // The turns of every pass, one pass after the other: the pass that joins groups of
    // `width` has `width` of them, from index `width - 1`
    turn_re: Vec<f32>,
    turn_im: Vec<f32>,
    // The turns that take the even and the odd samples apart: by half a step each
    split_re: Vec<f32>,
    split_im: Vec<f32>,
    // The complex signal
    work_re: Vec<f32>,
    work_im: Vec<f32>,
}

impl RealFft {
    /// `size` is the number of samples: a power of two, 4 at least
    pub fn new(size: usize) -> Self {
        assert!(size.is_power_of_two() && (4..=1 << 16).contains(&size));
        let half = size / 2;
        let bits = half.trailing_zeros();
        let order = (0..half).map(|index| (index.reverse_bits() >> (usize::BITS - bits)) as u16).collect();

        let (mut turn_re, mut turn_im) = (Vec::with_capacity(half), Vec::with_capacity(half));
        let mut width = 1;
        while width < half {
            for step in 0..width {
                let angle = -PI * step as f64 / width as f64;
                turn_re.push(angle.cos() as f32);
                turn_im.push(angle.sin() as f32);
            }
            width *= 2;
        }
        let split = |part: fn(f64) -> f64| (0..=half).map(|bin| part(-PI * bin as f64 / half as f64) as f32).collect();
        Self {
            half,
            order,
            turn_re,
            turn_im,
            split_re: split(f64::cos),
            split_im: split(f64::sin),
            work_re: vec![0.0; half],
            work_im: vec![0.0; half],
        }
    }

    /// The number of samples
    #[cfg(test)]
    pub fn size(&self) -> usize {
        2 * self.half
    }

    /// The number of frequencies: the length of each of the two rows
    #[cfg(test)]
    pub fn bins(&self) -> usize {
        self.half + 1
    }

    /// The complex transform of `work`, in place. The samples are in `order` already
    fn passes(&mut self) {
        let mut width = 1;
        while width < self.half {
            let turn_re = &self.turn_re[width - 1..2 * width - 1];
            let turn_im = &self.turn_im[width - 1..2 * width - 1];
            for (group_re, group_im) in self.work_re.chunks_exact_mut(2 * width).zip(self.work_im.chunks_exact_mut(2 * width)) {
                let (low_re, high_re) = group_re.split_at_mut(width);
                let (low_im, high_im) = group_im.split_at_mut(width);
                for step in 0..width {
                    let re = high_re[step] * turn_re[step] - high_im[step] * turn_im[step];
                    let im = high_re[step] * turn_im[step] + high_im[step] * turn_re[step];
                    high_re[step] = low_re[step] - re;
                    high_im[step] = low_im[step] - im;
                    low_re[step] += re;
                    low_im[step] += im;
                }
            }
            width *= 2;
        }
    }

    /// From `size` samples to `bins` frequencies
    pub fn forward(&mut self, samples: &[f32], re: &mut [f32], im: &mut [f32]) {
        let half = self.half;
        debug_assert!(samples.len() == 2 * half && re.len() == half + 1 && im.len() == half + 1);
        for (index, &to) in self.order.iter().enumerate() {
            self.work_re[to as usize] = samples[2 * index];
            self.work_im[to as usize] = samples[2 * index + 1];
        }
        self.passes();

        // `work` is the transform of the even samples plus i times that of the odd ones.
        // Both are transforms of real signals, which is what takes them apart
        for bin in 0..=half {
            let (this, mirror) = (bin % half, (half - bin) % half);
            let even_re = 0.5 * (self.work_re[this] + self.work_re[mirror]);
            let even_im = 0.5 * (self.work_im[this] - self.work_im[mirror]);
            let odd_re = 0.5 * (self.work_im[this] + self.work_im[mirror]);
            let odd_im = -0.5 * (self.work_re[this] - self.work_re[mirror]);
            re[bin] = even_re + odd_re * self.split_re[bin] - odd_im * self.split_im[bin];
            im[bin] = even_im + odd_re * self.split_im[bin] + odd_im * self.split_re[bin];
        }
    }

    /// From `bins` frequencies back to `size` samples
    pub fn inverse(&mut self, re: &[f32], im: &[f32], samples: &mut [f32]) {
        let half = self.half;
        debug_assert!(samples.len() == 2 * half && re.len() == half + 1 && im.len() == half + 1);
        // The transform backwards is the transform forwards of the mirrored frequencies,
        // so the even and odd samples are put together as that right away
        let scale = 1.0 / half as f32;
        for bin in 0..half {
            let mirror = half - bin;
            let even_re = 0.5 * (re[bin] + re[mirror]);
            let even_im = 0.5 * (im[bin] - im[mirror]);
            let turned_re = 0.5 * (re[bin] - re[mirror]);
            let turned_im = 0.5 * (im[bin] + im[mirror]);
            // Turned back by the half step it was turned by
            let odd_re = turned_re * self.split_re[bin] + turned_im * self.split_im[bin];
            let odd_im = turned_im * self.split_re[bin] - turned_re * self.split_im[bin];
            // Even plus i times odd, mirrored: its conjugate goes through the passes
            let to = self.order[bin] as usize;
            self.work_re[to] = scale * (even_re - odd_im);
            self.work_im[to] = -scale * (even_im + odd_re);
        }
        self.passes();
        for (index, pair) in samples.chunks_exact_mut(2).enumerate() {
            pair[0] = self.work_re[index];
            pair[1] = -self.work_im[index];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Noise;

    fn noise(len: usize, seed: u32) -> Vec<f32> {
        let mut noise = Noise::new(seed);
        (0..len).map(|_| noise.next()).collect()
    }

    #[test]
    fn test_forward_is_the_sum_it_stands_for() {
        for size in [4, 8, 64, 256] {
            let samples = noise(size, 3);
            let mut fft = RealFft::new(size);
            let (mut re, mut im) = (vec![0.0; fft.bins()], vec![0.0; fft.bins()]);
            fft.forward(&samples, &mut re, &mut im);
            for bin in 0..fft.bins() {
                let (mut sum_re, mut sum_im) = (0.0f64, 0.0f64);
                for (index, &sample) in samples.iter().enumerate() {
                    let angle = -2.0 * PI * (index * bin) as f64 / size as f64;
                    sum_re += sample as f64 * angle.cos();
                    sum_im += sample as f64 * angle.sin();
                }
                let off = (re[bin] as f64 - sum_re).abs().max((im[bin] as f64 - sum_im).abs());
                assert!(off < 1e-5 * size as f64, "Size {size}, bin {bin}: off by {off}");
            }
        }
    }

    #[test]
    fn test_there_and_back_gives_the_samples() {
        for size in [4, 16, 128, 512, 4096] {
            let samples = noise(size, 5);
            let mut fft = RealFft::new(size);
            assert_eq!(fft.size(), size);
            let (mut re, mut im) = (vec![0.0; fft.bins()], vec![0.0; fft.bins()]);
            fft.forward(&samples, &mut re, &mut im);
            let mut back = vec![0.0; size];
            fft.inverse(&re, &im, &mut back);
            let off = samples.iter().zip(&back).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(off < 2e-6, "Size {size}: off by {off}");
        }
    }

    #[test]
    fn test_product_of_transforms_is_the_convolution_around_the_circle() {
        let size = 64;
        let (a, b) = (noise(size, 7), noise(size, 9));
        let mut fft = RealFft::new(size);
        let bins = fft.bins();
        let (mut a_re, mut a_im, mut b_re, mut b_im) = (vec![0.0; bins], vec![0.0; bins], vec![0.0; bins], vec![0.0; bins]);
        fft.forward(&a, &mut a_re, &mut a_im);
        fft.forward(&b, &mut b_re, &mut b_im);
        let re: Vec<f32> = (0..bins).map(|bin| a_re[bin] * b_re[bin] - a_im[bin] * b_im[bin]).collect();
        let im: Vec<f32> = (0..bins).map(|bin| a_re[bin] * b_im[bin] + a_im[bin] * b_re[bin]).collect();
        let mut product = vec![0.0; size];
        fft.inverse(&re, &im, &mut product);
        for index in 0..size {
            let expected: f64 = (0..size).map(|tap| a[tap] as f64 * b[(index + size - tap) % size] as f64).sum();
            assert!((product[index] as f64 - expected).abs() < 1e-4, "Sample {index}");
        }
    }
}
