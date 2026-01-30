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
}

impl PitchShifter {
    pub fn new(_block_size: usize) -> Self {
        // Target ~15ms latency at 44.1kHz = ~660 samples
        let target_latency = 660;

        // Buffer needs to be large enough for pitch shifting range
        // At 0.5x rate (octave down), we need 2x the latency window
        let buffer_size = target_latency * 6;

        // Crossfade length ~3ms for smooth transitions
        let crossfade_len = 128;

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
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        // Adjust latency based on sample rate (~15ms)
        self.target_latency = (sample_rate * 0.015) as usize;
        self.buffer_size = self.target_latency * 6;
        self.buffer.resize(self.buffer_size, 0.0);
        self.crossfade_len = (sample_rate * 0.003) as usize; // ~3ms crossfade
        self.crossfade_buffer.resize(self.crossfade_len, 0.0);
        self.reset();
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

    pub fn process_sample(&mut self, input: f32) -> f32 {
        // Write input to buffer
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % self.buffer_size;
        self.samples_processed += 1;

        // During initial fill, output silence to build up minimal latency buffer
        if self.samples_processed < self.target_latency {
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
