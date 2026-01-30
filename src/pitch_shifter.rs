/// A pitch shifter using variable-rate playback with crossfade.
///
/// This uses a simple but effective approach:
/// 1. Write input samples to a circular buffer
/// 2. Read from the buffer at a different rate (faster = pitch up, slower = pitch down)
/// 3. When read position gets too far from write position, crossfade to resync
pub struct PitchShifter {
    // Circular buffer for input samples
    buffer: Vec<f32>,
    buffer_size: usize,

    // Write position (integer, where new samples go)
    write_pos: usize,

    // Read position (fractional, where we read from)
    read_pos: f64,

    // Playback rate (1.0 = normal, 2.0 = octave up, 0.5 = octave down)
    playback_rate: f64,

    // For crossfading during resync
    crossfade_buffer: Vec<f32>,
    crossfade_pos: usize,
    crossfade_len: usize,
    is_crossfading: bool,

    // Target latency in samples (distance between write and read)
    target_latency: usize,

    // Samples processed (for initial fill)
    samples_processed: usize,

    // Whether buffer needs resize (deferred to avoid audio glitches)
    needs_resize: bool,
    pending_buffer_size: usize,
    pending_crossfade_len: usize,
}

impl PitchShifter {
    pub fn new(_block_size: usize) -> Self {
        // Target ~5ms latency at 44.1kHz = ~220 samples
        let target_latency = 220;

        // Buffer needs to be large enough for pitch shifting range
        let buffer_size = target_latency * 8;

        // Crossfade length ~2ms for smooth transitions
        let crossfade_len = 88;

        Self {
            buffer: vec![0.0; buffer_size],
            buffer_size,
            write_pos: 0,
            read_pos: 0.0,
            playback_rate: 1.0,
            crossfade_buffer: vec![0.0; crossfade_len],
            crossfade_pos: 0,
            crossfade_len,
            is_crossfading: false,
            target_latency,
            samples_processed: 0,
            needs_resize: false,
            pending_buffer_size: buffer_size,
            pending_crossfade_len: crossfade_len,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        // Adjust latency based on sample rate (~5ms default)
        self.target_latency = (sample_rate * 0.005) as usize;
        self.buffer_size = self.target_latency * 8;
        self.buffer.resize(self.buffer_size, 0.0);
        self.crossfade_len = (sample_rate * 0.002) as usize; // ~2ms crossfade
        self.crossfade_buffer.resize(self.crossfade_len, 0.0);
        self.reset();
    }

    pub fn set_latency_ms(&mut self, latency_ms: f32, sample_rate: f32) {
        let new_latency = ((sample_rate * latency_ms / 1000.0) as usize).max(64);
        if new_latency != self.target_latency {
            self.target_latency = new_latency;
            self.pending_buffer_size = self.target_latency * 8;
            self.needs_resize = true;
        }
    }

    pub fn set_smoothness_ms(&mut self, smoothness_ms: f32, sample_rate: f32) {
        let new_crossfade_len = ((sample_rate * smoothness_ms / 1000.0) as usize).max(16);
        if new_crossfade_len != self.crossfade_len {
            self.pending_crossfade_len = new_crossfade_len;
            self.needs_resize = true;
        }
    }

    pub fn set_semitones(&mut self, semitones: i32) {
        // Convert semitones to playback rate
        // +12 semitones = 2x playback rate (octave up)
        // -12 semitones = 0.5x playback rate (octave down)
        self.playback_rate = 2.0_f64.powf(semitones as f64 / 12.0);
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
        self.read_pos = 0.0;
        self.crossfade_buffer.fill(0.0);
        self.crossfade_pos = 0;
        self.is_crossfading = false;
        self.samples_processed = 0;
    }

    fn apply_pending_resize(&mut self) {
        if !self.needs_resize {
            return;
        }

        // Only resize when not crossfading to avoid glitches
        if self.is_crossfading {
            return;
        }

        if self.pending_buffer_size != self.buffer_size {
            self.buffer_size = self.pending_buffer_size;
            self.buffer.resize(self.buffer_size, 0.0);
            // Reset positions to avoid out-of-bounds
            self.write_pos = self.write_pos % self.buffer_size;
            self.read_pos = self.read_pos % self.buffer_size as f64;
        }

        if self.pending_crossfade_len != self.crossfade_len {
            self.crossfade_len = self.pending_crossfade_len;
            self.crossfade_buffer.resize(self.crossfade_len, 0.0);
        }

        self.needs_resize = false;
    }

    pub fn process_sample(&mut self, input: f32) -> f32 {
        // Apply any pending buffer resizes
        self.apply_pending_resize();

        // Write input to buffer
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % self.buffer_size;
        self.samples_processed += 1;

        // During initial fill, output silence to build up minimal latency buffer
        if self.samples_processed <= self.target_latency {
            return 0.0;
        }

        // Read from buffer at variable rate with linear interpolation
        let output = self.read_interpolated(self.read_pos);

        // Advance read position
        self.read_pos += self.playback_rate;

        // Wrap read position
        if self.read_pos >= self.buffer_size as f64 {
            self.read_pos -= self.buffer_size as f64;
        }

        // Check if we need to resync (read too close to or too far from write)
        let distance = self.calculate_distance();
        let min_distance = self.crossfade_len as f64 + 64.0;
        let max_distance = (self.buffer_size - self.crossfade_len - 64) as f64;

        if !self.is_crossfading && (distance < min_distance || distance > max_distance) {
            self.start_crossfade();
        }

        // Apply crossfade if active
        if self.is_crossfading {
            return self.process_crossfade(output);
        }

        output
    }

    fn read_interpolated(&self, pos: f64) -> f32 {
        let idx = pos as usize % self.buffer_size;
        let frac = pos - pos.floor();

        let s0 = self.buffer[idx];
        let s1 = self.buffer[(idx + 1) % self.buffer_size];

        // Linear interpolation
        s0 + (s1 - s0) * frac as f32
    }

    fn calculate_distance(&self) -> f64 {
        // Calculate how far behind write position the read position is
        let write = self.write_pos as f64;
        let read = self.read_pos;

        if write >= read {
            write - read
        } else {
            (self.buffer_size as f64 - read) + write
        }
    }

    fn start_crossfade(&mut self) {
        // Save current output for crossfading
        for i in 0..self.crossfade_len {
            let pos = self.read_pos + i as f64 * self.playback_rate;
            self.crossfade_buffer[i] = self.read_interpolated(pos);
        }

        // Jump read position to target latency behind write
        self.read_pos = (self.write_pos as f64 - self.target_latency as f64 + self.buffer_size as f64)
            % self.buffer_size as f64;

        self.crossfade_pos = 0;
        self.is_crossfading = true;
    }

    fn process_crossfade(&mut self, new_sample: f32) -> f32 {
        let fade_in = self.crossfade_pos as f32 / self.crossfade_len as f32;
        let fade_out = 1.0 - fade_in;

        let old_sample = self.crossfade_buffer[self.crossfade_pos];
        let output = new_sample * fade_in + old_sample * fade_out;

        self.crossfade_pos += 1;
        if self.crossfade_pos >= self.crossfade_len {
            self.is_crossfading = false;
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

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
        // Count zero crossings to estimate frequency
        let mut crossings = 0;
        for i in 1..samples.len() {
            if (samples[i - 1] >= 0.0) != (samples[i] >= 0.0) {
                crossings += 1;
            }
        }
        // Each cycle has 2 zero crossings
        (crossings as f32 / 2.0) * sample_rate / samples.len() as f32
    }

    #[test]
    fn test_passthrough_at_zero_semitones() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(0);

        let input = generate_sine_wave(440.0, SAMPLE_RATE, 4000);
        let output = process_buffer(&mut shifter, &input);

        // Skip initial latency period
        let skip = shifter.target_latency + 100;
        let input_segment = &input[..input.len() - skip];
        let output_segment = &output[skip..];

        // Compare RMS levels (should be similar)
        let input_rms = rms(input_segment);
        let output_rms = rms(output_segment);

        assert!(
            (input_rms - output_rms).abs() < 0.1,
            "RMS mismatch: input={}, output={}",
            input_rms,
            output_rms
        );

        // Check frequency is preserved
        let input_freq = estimate_frequency(input_segment, SAMPLE_RATE);
        let output_freq = estimate_frequency(output_segment, SAMPLE_RATE);

        assert!(
            (input_freq - output_freq).abs() < 20.0,
            "Frequency mismatch: input={}, output={}",
            input_freq,
            output_freq
        );
    }

    #[test]
    fn test_pitch_up_octave() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(12); // +12 = octave up = 2x frequency

        let input_freq = 220.0;
        let input = generate_sine_wave(input_freq, SAMPLE_RATE, 8000);
        let output = process_buffer(&mut shifter, &input);

        // Skip initial latency and some settling time
        let skip = 1000;
        let output_segment = &output[skip..];

        // Should have signal (not silent)
        let output_rms = rms(output_segment);
        assert!(output_rms > 0.1, "Output is too quiet: RMS={}", output_rms);

        // Frequency should be approximately doubled
        let output_freq = estimate_frequency(output_segment, SAMPLE_RATE);
        let expected_freq = input_freq * 2.0;

        assert!(
            (output_freq - expected_freq).abs() < expected_freq * 0.15,
            "Expected ~{}Hz, got {}Hz",
            expected_freq,
            output_freq
        );
    }

    #[test]
    fn test_pitch_down_octave() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(-12); // -12 = octave down = 0.5x frequency

        let input_freq = 880.0;
        let input = generate_sine_wave(input_freq, SAMPLE_RATE, 8000);
        let output = process_buffer(&mut shifter, &input);

        // Skip initial latency
        let skip = 1000;
        let output_segment = &output[skip..];

        // Should have signal
        let output_rms = rms(output_segment);
        assert!(output_rms > 0.1, "Output is too quiet: RMS={}", output_rms);

        // Frequency should be approximately halved
        let output_freq = estimate_frequency(output_segment, SAMPLE_RATE);
        let expected_freq = input_freq * 0.5;

        assert!(
            (output_freq - expected_freq).abs() < expected_freq * 0.15,
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

            // Skip latency
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

        // Process some audio
        let input = generate_sine_wave(440.0, SAMPLE_RATE, 2000);
        let _ = process_buffer(&mut shifter, &input);

        // Reset
        shifter.reset();

        // Check state is cleared
        assert_eq!(shifter.write_pos, 0);
        assert_eq!(shifter.read_pos, 0.0);
        assert_eq!(shifter.samples_processed, 0);
        assert!(!shifter.is_crossfading);
    }

    #[test]
    fn test_latency_parameter_change() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);

        let initial_latency = shifter.target_latency;

        // Change latency
        shifter.set_latency_ms(20.0, SAMPLE_RATE);

        // Process some samples to apply the change
        for _ in 0..1000 {
            shifter.process_sample(0.5);
        }

        assert_ne!(
            shifter.target_latency, initial_latency,
            "Latency should have changed"
        );

        let expected_latency = (SAMPLE_RATE * 0.020) as usize;
        assert_eq!(shifter.target_latency, expected_latency);
    }

    #[test]
    fn test_smoothness_parameter_change() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);

        let initial_crossfade = shifter.crossfade_len;

        // Change smoothness
        shifter.set_smoothness_ms(5.0, SAMPLE_RATE);

        // Process some samples to apply the change
        for _ in 0..1000 {
            shifter.process_sample(0.5);
        }

        assert_ne!(
            shifter.crossfade_len, initial_crossfade,
            "Crossfade length should have changed"
        );

        let expected_crossfade = (SAMPLE_RATE * 0.005) as usize;
        assert_eq!(shifter.crossfade_len, expected_crossfade);
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

        shifter.set_semitones(7); // Perfect fifth
        let expected = 2.0_f64.powf(7.0 / 12.0); // ~1.498
        assert!((shifter.playback_rate - expected).abs() < 0.001);
    }

    #[test]
    fn test_initial_latency_silence() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(0);

        let latency = shifter.target_latency;

        // First `latency` samples should be silent
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
        assert!(output.abs() > 0.0, "Should produce output after latency");
    }

    #[test]
    fn test_no_crash_on_rapid_parameter_changes() {
        let mut shifter = PitchShifter::new(512);
        shifter.set_sample_rate(SAMPLE_RATE);

        let input = generate_sine_wave(440.0, SAMPLE_RATE, 10000);

        for (i, &sample) in input.iter().enumerate() {
            // Rapidly change parameters
            if i % 100 == 0 {
                shifter.set_semitones((i as i32 % 25) - 12);
            }
            if i % 200 == 0 {
                shifter.set_latency_ms((i % 30) as f32 + 2.0, SAMPLE_RATE);
            }
            if i % 300 == 0 {
                shifter.set_smoothness_ms((i % 8) as f32 + 1.0, SAMPLE_RATE);
            }

            // Should not panic
            let _ = shifter.process_sample(sample);
        }
    }
}
