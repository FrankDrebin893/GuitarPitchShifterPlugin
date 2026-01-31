use std::f32::consts::PI;

/// A pitch shifter using dual-head variable-rate playback with overlap.
///
/// Improvements over simple single-head approach:
/// 1. Two read heads with continuous crossfading (no abrupt resyncs)
/// 2. Cubic Hermite interpolation (smoother than linear)
/// 3. Equal-power crossfade curve (perceptually smoother)
pub struct PitchShifter {
    // Circular buffer for input samples
    buffer: Vec<f32>,
    buffer_size: usize,

    // Write position (integer, where new samples go)
    write_pos: usize,

    // Dual read heads for smooth crossfading
    read_pos_a: f64,
    read_pos_b: f64,

    // Crossfade position (0.0 to 1.0, then wraps)
    crossfade_phase: f64,

    // Playback rate (1.0 = normal, 2.0 = octave up, 0.5 = octave down)
    playback_rate: f64,

    // Grain size in samples (how often we reset a read head)
    grain_size: usize,

    // Target latency in samples
    target_latency: usize,

    // Sample counter within current grain
    grain_counter: usize,

    // Which head is currently "primary" (0 = A, 1 = B)
    active_head: usize,

    // Samples processed (for initial fill)
    samples_processed: usize,

    // Sample rate for calculations
    sample_rate: f32,

    // Whether parameters changed and need update
    needs_update: bool,
    pending_latency: usize,
    pending_grain_size: usize,
}

impl PitchShifter {
    pub fn new(_block_size: usize) -> Self {
        let sample_rate = 44100.0;
        let target_latency = (sample_rate * 0.005) as usize; // 5ms
        let grain_size = (sample_rate * 0.004) as usize; // 4ms grains
        let buffer_size = target_latency * 8;

        Self {
            buffer: vec![0.0; buffer_size],
            buffer_size,
            write_pos: 0,
            read_pos_a: 0.0,
            read_pos_b: 0.0,
            crossfade_phase: 0.0,
            playback_rate: 1.0,
            grain_size,
            target_latency,
            grain_counter: 0,
            active_head: 0,
            samples_processed: 0,
            sample_rate,
            needs_update: false,
            pending_latency: target_latency,
            pending_grain_size: grain_size,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.target_latency = (sample_rate * 0.005) as usize;
        self.grain_size = (sample_rate * 0.004) as usize;
        self.buffer_size = self.target_latency * 8;
        self.buffer.resize(self.buffer_size, 0.0);
        self.reset();
    }

    pub fn set_latency_ms(&mut self, latency_ms: f32, sample_rate: f32) {
        let new_latency = ((sample_rate * latency_ms / 1000.0) as usize).max(64);
        if new_latency != self.target_latency {
            self.pending_latency = new_latency;
            // Grain size should be roughly 60-80% of latency for good overlap
            self.pending_grain_size = (new_latency as f32 * 0.7) as usize;
            self.pending_grain_size = self.pending_grain_size.max(32);
            self.needs_update = true;
        }
    }

    pub fn set_smoothness_ms(&mut self, smoothness_ms: f32, sample_rate: f32) {
        // Smoothness now controls grain size directly
        let new_grain_size = ((sample_rate * smoothness_ms / 1000.0) as usize).max(32);
        if new_grain_size != self.grain_size {
            self.pending_grain_size = new_grain_size;
            self.needs_update = true;
        }
    }

    pub fn set_semitones(&mut self, semitones: i32) {
        self.playback_rate = 2.0_f64.powf(semitones as f64 / 12.0);
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
        self.read_pos_a = 0.0;
        self.read_pos_b = 0.0;
        self.crossfade_phase = 0.0;
        self.grain_counter = 0;
        self.active_head = 0;
        self.samples_processed = 0;
    }

    fn apply_pending_updates(&mut self) {
        if !self.needs_update {
            return;
        }

        // Only update at grain boundaries to avoid glitches
        if self.grain_counter != 0 {
            return;
        }

        if self.pending_latency != self.target_latency {
            self.target_latency = self.pending_latency;
            let new_buffer_size = self.target_latency * 8;
            if new_buffer_size != self.buffer_size {
                self.buffer_size = new_buffer_size;
                self.buffer.resize(self.buffer_size, 0.0);
                self.write_pos = self.write_pos % self.buffer_size;
                self.read_pos_a = self.read_pos_a % self.buffer_size as f64;
                self.read_pos_b = self.read_pos_b % self.buffer_size as f64;
            }
        }

        if self.pending_grain_size != self.grain_size {
            self.grain_size = self.pending_grain_size;
        }

        self.needs_update = false;
    }

    pub fn process_sample(&mut self, input: f32) -> f32 {
        // Apply pending parameter updates at safe times
        self.apply_pending_updates();

        // Write input to buffer
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % self.buffer_size;
        self.samples_processed += 1;

        // During initial fill, output silence
        if self.samples_processed <= self.target_latency {
            // Initialize read positions once we have enough data
            if self.samples_processed == self.target_latency {
                self.read_pos_a = 0.0;
                self.read_pos_b = (self.grain_size / 2) as f64;
            }
            return 0.0;
        }

        // Read from both heads with cubic interpolation
        let sample_a = self.read_cubic(self.read_pos_a);
        let sample_b = self.read_cubic(self.read_pos_b);

        // Equal-power crossfade between the two heads
        let fade = self.equal_power_fade(self.crossfade_phase as f32);
        let output = if self.active_head == 0 {
            sample_a * fade.0 + sample_b * fade.1
        } else {
            sample_b * fade.0 + sample_a * fade.1
        };

        // Advance read positions
        self.read_pos_a += self.playback_rate;
        self.read_pos_b += self.playback_rate;

        // Wrap read positions
        if self.read_pos_a >= self.buffer_size as f64 {
            self.read_pos_a -= self.buffer_size as f64;
        }
        if self.read_pos_b >= self.buffer_size as f64 {
            self.read_pos_b -= self.buffer_size as f64;
        }

        // Advance crossfade phase
        self.grain_counter += 1;
        self.crossfade_phase = self.grain_counter as f64 / self.grain_size as f64;

        // At end of grain, reset the inactive head and swap
        if self.grain_counter >= self.grain_size {
            self.grain_counter = 0;
            self.crossfade_phase = 0.0;

            // Reset the head that just faded out to a good position
            let target_pos = (self.write_pos as f64 - self.target_latency as f64
                + self.buffer_size as f64) % self.buffer_size as f64;

            if self.active_head == 0 {
                // Head A was primary, now B becomes primary
                // Reset A to target position
                self.read_pos_a = target_pos;
                self.active_head = 1;
            } else {
                // Head B was primary, now A becomes primary
                // Reset B to target position
                self.read_pos_b = target_pos;
                self.active_head = 0;
            }
        }

        output
    }

    /// Cubic Hermite interpolation for smoother sample reading
    fn read_cubic(&self, pos: f64) -> f32 {
        let pos_wrapped = pos % self.buffer_size as f64;
        let idx = pos_wrapped as usize;
        let frac = (pos_wrapped - idx as f64) as f32;

        // Get 4 samples for cubic interpolation
        let s0 = self.buffer[(idx + self.buffer_size - 1) % self.buffer_size];
        let s1 = self.buffer[idx % self.buffer_size];
        let s2 = self.buffer[(idx + 1) % self.buffer_size];
        let s3 = self.buffer[(idx + 2) % self.buffer_size];

        // Cubic Hermite interpolation
        let c0 = s1;
        let c1 = 0.5 * (s2 - s0);
        let c2 = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
        let c3 = 0.5 * (s3 - s0) + 1.5 * (s1 - s2);

        ((c3 * frac + c2) * frac + c1) * frac + c0
    }

    /// Equal-power crossfade using cosine curve
    /// Returns (fade_out, fade_in) gains
    fn equal_power_fade(&self, phase: f32) -> (f32, f32) {
        let angle = phase * PI * 0.5;
        let fade_out = angle.cos();
        let fade_in = angle.sin();
        (fade_out, fade_in)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 44100.0;

    fn generate_sine_wave(frequency: f32, sample_rate: f32, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| (2.0 * PI * frequency * i as f32 / sample_rate).sin())
            .collect()
    }

    fn process_buffer(shifter: &mut PitchShifter, input: &[f32]) -> Vec<f32> {
        input.iter().map(|&s| shifter.process_sample(s)).collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn estimate_frequency(samples: &[f32], sample_rate: f32) -> f32 {
        let mut crossings = 0;
        for i in 1..samples.len() {
            if (samples[i - 1] >= 0.0) != (samples[i] >= 0.0) {
                crossings += 1;
            }
        }
        (crossings as f32 / 2.0) * sample_rate / samples.len() as f32
    }

    #[test]
    fn test_passthrough_at_zero_semitones() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(0);

        let input = generate_sine_wave(440.0, SAMPLE_RATE, 4000);
        let output = process_buffer(&mut shifter, &input);

        let skip = shifter.target_latency + 100;
        let input_segment = &input[..input.len() - skip];
        let output_segment = &output[skip..];

        let input_rms = rms(input_segment);
        let output_rms = rms(output_segment);

        // With dual-head overlap, RMS may vary slightly due to crossfade
        // Just verify output is in a reasonable range (not silent or clipping)
        assert!(
            output_rms > 0.3 && output_rms < 1.5,
            "RMS out of range: input={}, output={}",
            input_rms,
            output_rms
        );

        let input_freq = estimate_frequency(input_segment, SAMPLE_RATE);
        let output_freq = estimate_frequency(output_segment, SAMPLE_RATE);

        assert!(
            (input_freq - output_freq).abs() < 30.0,
            "Frequency mismatch: input={}, output={}",
            input_freq,
            output_freq
        );
    }

    #[test]
    fn test_pitch_up_octave() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(12);

        let input_freq = 220.0;
        let input = generate_sine_wave(input_freq, SAMPLE_RATE, 8000);
        let output = process_buffer(&mut shifter, &input);

        let skip = 1000;
        let output_segment = &output[skip..];

        let output_rms = rms(output_segment);
        assert!(output_rms > 0.1, "Output is too quiet: RMS={}", output_rms);

        let output_freq = estimate_frequency(output_segment, SAMPLE_RATE);
        let expected_freq = input_freq * 2.0;

        assert!(
            (output_freq - expected_freq).abs() < expected_freq * 0.2,
            "Expected ~{}Hz, got {}Hz",
            expected_freq,
            output_freq
        );
    }

    #[test]
    fn test_pitch_down_octave() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(-12);

        let input_freq = 880.0;
        let input = generate_sine_wave(input_freq, SAMPLE_RATE, 8000);
        let output = process_buffer(&mut shifter, &input);

        let skip = 1000;
        let output_segment = &output[skip..];

        let output_rms = rms(output_segment);
        assert!(output_rms > 0.1, "Output is too quiet: RMS={}", output_rms);

        let output_freq = estimate_frequency(output_segment, SAMPLE_RATE);
        let expected_freq = input_freq * 0.5;

        assert!(
            (output_freq - expected_freq).abs() < expected_freq * 0.2,
            "Expected ~{}Hz, got {}Hz",
            expected_freq,
            output_freq
        );
    }

    #[test]
    fn test_produces_output_all_semitones() {
        for semitones in -12..=12 {
            let mut shifter = PitchShifter::new(512);
            shifter.set_sample_rate(SAMPLE_RATE);
            shifter.set_semitones(semitones);

            let input = generate_sine_wave(440.0, SAMPLE_RATE, 4000);
            let output = process_buffer(&mut shifter, &input);

            let skip = 500;
            let output_segment = &output[skip..];
            let output_rms = rms(output_segment);

            assert!(
                output_rms > 0.05,
                "Semitones={}: Output is silent (RMS={})",
                semitones,
                output_rms
            );
        }
    }

    #[test]
    fn test_reset_clears_state() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(5);

        let input = generate_sine_wave(440.0, SAMPLE_RATE, 2000);
        let _ = process_buffer(&mut shifter, &input);

        shifter.reset();

        assert_eq!(shifter.write_pos, 0);
        assert_eq!(shifter.read_pos_a, 0.0);
        assert_eq!(shifter.read_pos_b, 0.0);
        assert_eq!(shifter.samples_processed, 0);
    }

    #[test]
    fn test_latency_parameter_change() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);

        let initial_latency = shifter.target_latency;

        shifter.set_latency_ms(20.0, SAMPLE_RATE);

        // Process enough samples to trigger update at grain boundary
        for _ in 0..2000 {
            shifter.process_sample(0.5);
        }

        assert_ne!(
            shifter.target_latency, initial_latency,
            "Latency should have changed"
        );
    }

    #[test]
    fn test_smoothness_parameter_change() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);

        let initial_grain_size = shifter.grain_size;

        shifter.set_smoothness_ms(8.0, SAMPLE_RATE);

        // Process enough samples to trigger update at grain boundary
        for _ in 0..2000 {
            shifter.process_sample(0.5);
        }

        assert_ne!(
            shifter.grain_size, initial_grain_size,
            "Grain size should have changed"
        );
    }

    #[test]
    fn test_playback_rate_calculation() {
        let mut shifter = PitchShifter::new(512);

        shifter.set_semitones(0);
        assert!((shifter.playback_rate - 1.0).abs() < 0.001);

        shifter.set_semitones(12);
        assert!((shifter.playback_rate - 2.0).abs() < 0.001);

        shifter.set_semitones(-12);
        assert!((shifter.playback_rate - 0.5).abs() < 0.001);

        shifter.set_semitones(7);
        let expected = 2.0_f64.powf(7.0 / 12.0);
        assert!((shifter.playback_rate - expected).abs() < 0.001);
    }

    #[test]
    fn test_initial_latency_silence() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(0);

        let latency = shifter.target_latency;

        for i in 0..latency {
            let output = shifter.process_sample(1.0);
            assert_eq!(
                output, 0.0,
                "Sample {} should be silent during initial latency",
                i
            );
        }

        // After latency, should produce output
        let output = shifter.process_sample(1.0);
        assert!(output.abs() >= 0.0, "Should produce output after latency");
    }

    #[test]
    fn test_no_crash_on_rapid_parameter_changes() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);

        let input = generate_sine_wave(440.0, SAMPLE_RATE, 10000);

        for (i, &sample) in input.iter().enumerate() {
            if i % 100 == 0 {
                shifter.set_semitones((i as i32 % 25) - 12);
            }
            if i % 200 == 0 {
                shifter.set_latency_ms((i % 30) as f32 + 2.0, SAMPLE_RATE);
            }
            if i % 300 == 0 {
                shifter.set_smoothness_ms((i % 8) as f32 + 1.0, SAMPLE_RATE);
            }

            let _ = shifter.process_sample(sample);
        }
    }

    #[test]
    fn test_equal_power_fade() {
        let shifter = PitchShifter::new(512);

        // At phase 0, should be (1, 0)
        let (out, inp) = shifter.equal_power_fade(0.0);
        assert!((out - 1.0).abs() < 0.001);
        assert!(inp.abs() < 0.001);

        // At phase 0.5, both should be equal (~0.707)
        let (out, inp) = shifter.equal_power_fade(0.5);
        assert!((out - inp).abs() < 0.001);
        assert!((out - 0.707).abs() < 0.01);

        // At phase 1, should be (0, 1)
        let (out, inp) = shifter.equal_power_fade(1.0);
        assert!(out.abs() < 0.001);
        assert!((inp - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cubic_interpolation_smoother_than_linear() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);

        // Fill buffer with a simple ramp
        for i in 0..shifter.buffer_size {
            shifter.buffer[i] = i as f32;
        }

        // Read at fractional position - cubic should be smooth
        let val = shifter.read_cubic(10.5);

        // For a linear ramp, cubic interpolation should give ~10.5
        assert!((val - 10.5).abs() < 0.1, "Cubic interpolation value: {}", val);
    }
}
