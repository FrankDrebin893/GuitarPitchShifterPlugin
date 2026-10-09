use std::f32::consts::PI;

/// Circular buffer length. A power of two so positions wrap with a mask, and long enough for
/// the largest latency setting at 192 kHz, so it never has to be resized while audio runs.
const BUFFER_SIZE: usize = 1 << 16;
const BUFFER_MASK: usize = BUFFER_SIZE - 1;

/// Closest the read head may get to the write head. Cubic interpolation reads two samples
/// past the head, and a whole-sample delay reads the buffer back bit-exactly.
const MIN_DELAY: f64 = 2.0;

/// Shortest crossfade, in samples
const MIN_FADE: usize = 8;

/// Splice points are searched on a copy of the input decimated to roughly this rate
const ANALYSIS_RATE: f32 = 11025.0;

/// Shortest jump considered, about one period of the highest fretted note
const MIN_LAG_MS: f32 = 0.7;

/// Bounds on how much signal is compared when looking for a splice point
const MIN_WINDOW_MS: f32 = 5.0;
const MAX_WINDOW_MS: f32 = 20.0;

/// Span compared when lining the splice up to the exact sample
const REFINE_WINDOW_MS: f32 = 6.0;

/// The correlation has to fall below this before a peak counts, which rules out the
/// trivial match of the signal with itself at very short lags
const DIP_LEVEL: f32 = 0.5;

/// A peak this close to the best one is taken as the period (rather than a multiple of it)
const KEY_PEAK_RATIO: f32 = 0.9;

/// Match quality needed to splice before the latency limit forces one
const GOOD_MATCH: f32 = 0.85;

/// Below this the signal has no usable period and the jump only minimizes latency
const POOR_MATCH: f32 = 0.25;

/// How much further the head falls behind before a failed search is retried
const RETRY_MS: f32 = 1.0;

/// Input peak level below which splices are inaudible (-60 dBFS), and how long that level
/// is held after the last peak
const QUIET_LEVEL: f32 = 0.001;
const LEVEL_RELEASE_MS: f32 = 50.0;

/// Furthest the head may fall behind while the input is quiet
const QUIET_DELAY_MS: f32 = 3.0;

/// Capacity of the correlation scratch buffer
const MAX_SCORES: usize = 2048;

/// A pitch shifter using variable-rate playback with waveform-matched splices.
///
/// 1. One read head plays the input back at the shifted rate (cubic interpolation), so
///    between splices the output is a plain resampled copy of the input
/// 2. The head only jumps when it drifts too close to or too far from the write head
/// 3. Each jump lands where the waveform lines up with itself (normally a whole pitch
///    period away), bridged by a short constant-level crossfade
pub struct PitchShifter {
    // Circular buffers for input samples, one per channel
    buffers: [Vec<f32>; 2],

    // Mono mix of the input, used to find splice points
    mix: Vec<f32>,

    // Samples written so far (wraps with BUFFER_MASK)
    write_pos: usize,

    // Low-rate copy of the mix for the coarse splice search
    decimated: Vec<f32>,
    decimated_pos: usize,
    decimation: usize,
    decimation_sum: f32,
    decimation_count: usize,

    // Read head position, in samples behind the write head
    delay: f64,

    // Outgoing head while a splice is crossfading
    fade_delay: f64,
    fade_len: usize,
    fade_remaining: usize,

    // How well the two heads matched at the splice (0.0 to 1.0)
    fade_match: f32,

    // Playback rate (1.0 = normal, 2.0 = octave up, 0.5 = octave down)
    playback_rate: f64,

    // Parameters
    sample_rate: f32,
    latency_ms: f32,
    smoothness_ms: f32,

    // Furthest the head may fall behind the write head, in samples
    max_delay: f64,

    // The same limit while the input is quiet
    quiet_delay: f64,

    // Peak level of the recent input
    level: f32,
    level_decay: f32,

    // Crossfade length for the current rate, in samples
    crossfade_len: usize,

    // Shortest allowed jump at the current rate, in samples
    min_lag: usize,

    // When shifting up: jump once the head gets this close to the write head
    low_delay: f64,

    // When shifting up: where the head waits while the input is quiet
    hold_delay: f64,

    // When shifting down: look for a splice once the head is this far behind
    next_search_delay: f64,

    // Search sizes, in samples
    window: usize,
    refine_window: usize,
    retry_step: f64,

    // Scratch space for correlation values
    scores: Vec<f32>,
}

/// Result of a splice search
struct SpliceMatch {
    // How far to move the head, in samples
    lag: f64,
    // Normalized correlation at that lag
    quality: f32,
    // Shortest well-matched lag, i.e. the pitch period
    period: f64,
    // Whether the lag is a real correlation peak rather than just the best value in range
    is_peak: bool,
}

impl Default for PitchShifter {
    fn default() -> Self {
        Self::new()
    }
}

impl PitchShifter {
    pub fn new() -> Self {
        let mut shifter = Self {
            buffers: [vec![0.0; BUFFER_SIZE], vec![0.0; BUFFER_SIZE]],
            mix: vec![0.0; BUFFER_SIZE],
            write_pos: 0,
            decimated: vec![0.0; BUFFER_SIZE],
            decimated_pos: 0,
            decimation: 1,
            decimation_sum: 0.0,
            decimation_count: 0,
            delay: MIN_DELAY,
            fade_delay: MIN_DELAY,
            fade_len: 0,
            fade_remaining: 0,
            fade_match: 1.0,
            playback_rate: 1.0,
            sample_rate: 44100.0,
            latency_ms: 15.0,
            smoothness_ms: 2.0,
            max_delay: 0.0,
            quiet_delay: 0.0,
            level: 0.0,
            level_decay: 0.0,
            crossfade_len: 0,
            min_lag: 0,
            low_delay: MIN_DELAY,
            hold_delay: MIN_DELAY,
            next_search_delay: 0.0,
            window: 0,
            refine_window: 0,
            retry_step: 0.0,
            scores: vec![0.0; MAX_SCORES],
        };
        shifter.update_timing();
        shifter.reset();
        shifter
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.update_timing();
        self.reset();
    }

    /// Furthest the output may lag behind the input. This is also the longest pitch period
    /// that can be spliced cleanly (low E is about 12 ms).
    pub fn set_latency_ms(&mut self, latency_ms: f32) {
        if latency_ms != self.latency_ms {
            self.latency_ms = latency_ms;
            self.update_timing();
        }
    }

    /// Crossfade length at each splice
    pub fn set_smoothness_ms(&mut self, smoothness_ms: f32) {
        if smoothness_ms != self.smoothness_ms {
            self.smoothness_ms = smoothness_ms;
            self.update_timing();
        }
    }

    pub fn set_semitones(&mut self, semitones: i32) {
        let playback_rate = 2.0_f64.powf(semitones as f64 / 12.0);
        if playback_rate != self.playback_rate {
            self.playback_rate = playback_rate;
            self.update_timing();
        }
    }

    pub fn reset(&mut self) {
        for buffer in &mut self.buffers {
            buffer.fill(0.0);
        }
        self.mix.fill(0.0);
        self.decimated.fill(0.0);
        self.write_pos = 0;
        self.decimated_pos = 0;
        self.decimation_sum = 0.0;
        self.decimation_count = 0;
        self.delay = MIN_DELAY;
        self.fade_delay = MIN_DELAY;
        self.fade_remaining = 0;
        self.next_search_delay = MIN_DELAY;
        self.level = 0.0;
    }

    /// Converts the parameters to sample counts for the current sample rate and shift
    fn update_timing(&mut self) {
        let samples = |ms: f32| (self.sample_rate * ms / 1000.0) as f64;
        let drift = (self.playback_rate - 1.0).abs();

        self.decimation = ((self.sample_rate / ANALYSIS_RATE).round() as usize).max(1);
        self.max_delay = samples(self.latency_ms).clamp(MIN_DELAY + 64.0, (BUFFER_SIZE / 4) as f64);
        self.window = samples(self.latency_ms.clamp(MIN_WINDOW_MS, MAX_WINDOW_MS)) as usize;
        self.refine_window = (samples(REFINE_WINDOW_MS) as usize).min(self.window);
        self.retry_step = samples(RETRY_MS);
        self.level_decay = (-1.0 / samples(LEVEL_RELEASE_MS)).exp() as f32;

        // While a crossfade runs the outgoing head keeps drifting, which uses up part of the
        // delay range. Shorten the fade if it would not leave room for a jump.
        let mut crossfade_len = samples(self.smoothness_ms) as usize;
        if drift > 0.0 {
            let room = ((self.max_delay - MIN_DELAY) / (5.0 * drift)) as usize;
            crossfade_len = crossfade_len.min(room);
        }
        self.crossfade_len = crossfade_len.max(MIN_FADE);

        // A jump has to buy at least two crossfade lengths of playing time
        let fade_drift = self.crossfade_len as f64 * drift;
        self.min_lag = (samples(MIN_LAG_MS) as usize).max((2.0 * fade_drift).ceil() as usize);
        self.low_delay = MIN_DELAY + fade_drift;

        // Far enough back that two of the longest periods go by before the head has caught up
        let hold_delay = self.low_delay + (2.0 * self.max_delay * drift).max(self.min_lag as f64);
        self.hold_delay = hold_delay.min(self.max_delay);
        self.quiet_delay =
            (MIN_DELAY + samples(QUIET_DELAY_MS).max(2.0 * self.min_lag as f64)).min(self.max_delay);
    }

    /// Processes one mono sample
    pub fn process_sample(&mut self, input: f32) -> f32 {
        self.process_frame([input, 0.0], 1)[0]
    }

    /// Processes one stereo frame. Both channels share the same read head, so they stay
    /// aligned with each other.
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        let output = self.process_frame([left, right], 2);
        (output[0], output[1])
    }

    fn process_frame(&mut self, input: [f32; 2], channels: usize) -> [f32; 2] {
        // Write input to the buffers
        let write_idx = self.write_pos & BUFFER_MASK;
        let mut mix = 0.0;
        let mut peak = 0.0_f32;
        for channel in 0..channels {
            self.buffers[channel][write_idx] = input[channel];
            mix += input[channel];
            peak = peak.max(input[channel].abs());
        }
        mix /= channels as f32;
        self.mix[write_idx] = mix;
        self.write_pos = self.write_pos.wrapping_add(1);
        self.level = peak.max(self.level * self.level_decay);

        self.decimation_sum += mix;
        self.decimation_count += 1;
        if self.decimation_count == self.decimation {
            self.decimated[self.decimated_pos & BUFFER_MASK] =
                self.decimation_sum / self.decimation as f32;
            self.decimated_pos = self.decimated_pos.wrapping_add(1);
            self.decimation_sum = 0.0;
            self.decimation_count = 0;
        }

        if self.fade_remaining == 0 {
            self.update_splice();
        }

        let mut output = [0.0; 2];
        if self.fade_remaining > 0 {
            let (fade_out, fade_in) = self.fade_gains();
            for channel in 0..channels {
                output[channel] = self.read_cubic(channel, self.fade_delay) * fade_out
                    + self.read_cubic(channel, self.delay) * fade_in;
            }
            self.fade_delay = (self.fade_delay + 1.0 - self.playback_rate).max(MIN_DELAY);
            self.fade_remaining -= 1;
        } else {
            for channel in 0..channels {
                output[channel] = self.read_cubic(channel, self.delay);
            }
        }

        // The write head moves one sample per sample, the read head moves at the playback rate
        self.delay = (self.delay + 1.0 - self.playback_rate).max(MIN_DELAY);

        output
    }

    /// Starts a splice when the read head has drifted as far as it may
    fn update_splice(&mut self) {
        if self.playback_rate > 1.0 {
            // Shifting up: the head catches up with the write head. Jump back while the
            // outgoing head still has room to finish its fade.
            if self.level < QUIET_LEVEL {
                // Nothing audible is playing, so wait further back. The next note then gets
                // a couple of periods in before its first splice, instead of one that lands
                // in the middle of the attack.
                if self.delay <= self.hold_delay - self.min_lag as f64 {
                    self.begin_fade(self.hold_delay, 0.0);
                }
            } else if self.delay <= self.low_delay {
                let max_lag = ((self.max_delay - self.delay).max(0.0) as usize).max(self.min_lag);
                let found = self.find_splice_lag(self.min_lag, max_lag, true);
                self.begin_fade(self.delay + found.lag, found.quality);
            }
        } else if self.playback_rate < 1.0 {
            // Shifting down: the head falls behind. Jump forward as soon as a whole period
            // fits, or when the latency limit is reached. While the input is quiet a rough
            // splice cannot be heard, so stay close and keep the next attack on time.
            let limit = if self.level < QUIET_LEVEL { self.quiet_delay } else { self.max_delay };
            let forced = self.delay >= limit;
            if !forced && self.delay < self.next_search_delay {
                return;
            }

            let max_lag = (self.delay - MIN_DELAY) as usize;
            if max_lag <= self.min_lag && !forced {
                self.next_search_delay = MIN_DELAY + self.min_lag as f64 + self.retry_step;
                return;
            }

            let found = self.find_splice_lag(self.min_lag.min(max_lag), max_lag, false);
            let good = found.is_peak && found.quality >= GOOD_MATCH;
            if good {
                self.begin_fade(self.delay - found.lag, found.quality);
                // Search again once the next period has built up, with room to see its peak
                let reach = found.period * 1.05 + (2 * self.decimation) as f64;
                self.next_search_delay = MIN_DELAY + reach.max(self.min_lag as f64 + 1.0);
            } else if forced {
                // Nothing lines up well, so at least get back to the lowest latency
                let lag = if found.quality < POOR_MATCH { max_lag as f64 } else { found.lag };
                self.begin_fade(self.delay - lag, found.quality);
                self.next_search_delay = self.delay + self.retry_step;
            } else {
                self.next_search_delay = self.delay + self.retry_step;
            }
        } else if self.delay != MIN_DELAY {
            // No shift: settle on a whole-sample delay so the input passes through untouched
            let head = self.delay.round() as usize;
            let quality = correlation(
                &self.mix,
                self.write_pos.wrapping_sub(1),
                head,
                MIN_DELAY as usize,
                self.refine_window,
            );
            self.begin_fade(MIN_DELAY, quality);
        }
    }

    fn begin_fade(&mut self, new_delay: f64, quality: f32) {
        self.fade_delay = self.delay;
        self.delay = new_delay.max(MIN_DELAY);
        self.fade_len = self.crossfade_len;
        self.fade_remaining = self.crossfade_len;
        self.fade_match = quality.clamp(0.0, 1.0);
    }

    /// Finds how far the read head should jump so the waveform continues seamlessly.
    ///
    /// Compares the signal leading up to the head with the signal leading up to each
    /// candidate position, `min_lag` to `max_lag` samples away. Only past input is used, so
    /// the search adds no latency.
    fn find_splice_lag(&mut self, min_lag: usize, max_lag: usize, toward_past: bool) -> SpliceMatch {
        let head = self.delay.round() as usize;
        let step = self.decimation;

        // Coarse search on the decimated mix, starting from lag 1 so the peak picker can
        // see where the signal stops matching itself
        let newest = self.decimated_pos.wrapping_sub(1);
        let head_coarse = (head + step - 1).saturating_sub(self.decimation_count) / step;
        let window = (self.window / step).max(4);
        let first_lag = ((min_lag + step - 1) / step).max(1);
        let last_lag = (max_lag / step).clamp(first_lag.min(MAX_SCORES), MAX_SCORES);
        for lag in 1..=last_lag {
            let other = if toward_past {
                head_coarse + lag
            } else {
                head_coarse.saturating_sub(lag)
            };
            self.scores[lag - 1] = correlation(&self.decimated, newest, head_coarse, other, window);
        }
        let (chosen, first, is_peak) =
            pick_peak(&self.scores[..last_lag], first_lag - 1, toward_past);
        let coarse_lag = (chosen + 1) * step;
        let coarse_period = (first + 1) * step;

        // Fine search at full rate around the coarse result, with one extra lag on each
        // side for interpolating the peak position
        let newest = self.write_pos.wrapping_sub(1);
        let lo = coarse_lag.saturating_sub(step).clamp(min_lag, max_lag);
        let hi = (coarse_lag + step).clamp(lo, max_lag);
        let from = lo.saturating_sub(1).max(1);
        let to = if toward_past { hi + 1 } else { (hi + 1).min(head) };
        for lag in from..=to {
            let other = if toward_past { head + lag } else { head - lag };
            self.scores[lag - from] = correlation(&self.mix, newest, head, other, self.refine_window);
        }
        let mut best = lo;
        for lag in lo..=hi {
            if self.scores[lag - from] > self.scores[best - from] {
                best = lag;
            }
        }

        let mut lag = best as f64;
        if best > from && best < to {
            let before = self.scores[best - 1 - from];
            let peak = self.scores[best - from];
            let after = self.scores[best + 1 - from];
            let curvature = before - 2.0 * peak + after;
            if curvature < 0.0 {
                lag += (0.5 * (before - after) / curvature).clamp(-0.5, 0.5) as f64;
            }
        }

        // Judge the match over the long window. The short one above only says how well the
        // last few milliseconds happen to line up, which is not enough for a chord.
        let other = if toward_past { head + best } else { head - best };
        let quality = correlation(&self.mix, newest, head, other, self.window);

        SpliceMatch {
            lag,
            quality,
            period: if chosen == first { lag } else { coarse_period as f64 },
            is_peak,
        }
    }

    /// Gains for the outgoing and incoming head during a splice.
    /// Returns (fade_out, fade_in)
    fn fade_gains(&self) -> (f32, f32) {
        let progress = (self.fade_len - self.fade_remaining + 1) as f32 / (self.fade_len + 1) as f32;
        let fade_in = 0.5 - 0.5 * (PI * progress).cos();
        let fade_out = 1.0 - fade_in;

        // Matched signals add in amplitude, unrelated ones add in power. Scale for the
        // measured match so the level stays constant through the fade either way.
        let level = fade_in * fade_in + fade_out * fade_out + 2.0 * self.fade_match * fade_in * fade_out;
        let scale = level.sqrt().recip();
        (fade_out * scale, fade_in * scale)
    }

    /// Cubic Hermite interpolation, reading `delay` samples behind the newest input sample
    fn read_cubic(&self, channel: usize, delay: f64) -> f32 {
        let whole = delay.ceil();
        let frac = (whole - delay) as f32;
        let idx = self.write_pos.wrapping_sub(1 + whole as usize);
        let buffer = &self.buffers[channel];

        // Get 4 samples for cubic interpolation
        let s0 = buffer[idx.wrapping_sub(1) & BUFFER_MASK];
        let s1 = buffer[idx & BUFFER_MASK];
        let s2 = buffer[idx.wrapping_add(1) & BUFFER_MASK];
        let s3 = buffer[idx.wrapping_add(2) & BUFFER_MASK];

        // Cubic Hermite interpolation
        let c0 = s1;
        let c1 = 0.5 * (s2 - s0);
        let c2 = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
        let c3 = 0.5 * (s3 - s0) + 1.5 * (s1 - s2);

        ((c3 * frac + c2) * frac + c1) * frac + c0
    }
}

/// Normalized correlation between two `len`-sample spans of a circular buffer. Each span is
/// given by how far its newest sample lies behind `newest`.
fn correlation(buffer: &[f32], newest: usize, delay_a: usize, delay_b: usize, len: usize) -> f32 {
    let (mut ab, mut aa, mut bb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for i in 0..len {
        let a = buffer[newest.wrapping_sub(delay_a + i) & BUFFER_MASK];
        let b = buffer[newest.wrapping_sub(delay_b + i) & BUFFER_MASK];
        ab += a * b;
        aa += a * a;
        bb += b * b;
    }

    let energy = aa * bb;
    if energy > 1e-18 {
        ab / energy.sqrt()
    } else {
        0.0
    }
}

/// Picks the lag to splice at from correlation scores (`scores[i]` belongs to lag `i + 1`).
/// Only indices from `min_index` on may be chosen.
///
/// Returns (chosen index, index of the shortest good peak, whether a real peak was found).
/// `prefer_short` chooses the shortest good peak, otherwise the longest.
fn pick_peak(scores: &[f32], min_index: usize, prefer_short: bool) -> (usize, usize, bool) {
    let len = scores.len();
    let is_peak = |i: usize| scores[i] >= scores[i - 1] && scores[i] > scores[i + 1];

    // Peaks only count after the signal has stopped matching itself
    let dip = scores.iter().position(|&score| score < DIP_LEVEL).unwrap_or(len);
    let start = dip.max(min_index).max(1);
    let end = len.saturating_sub(1);

    let mut best = 0.0_f32;
    for i in start..end {
        if is_peak(i) && scores[i] > best {
            best = scores[i];
        }
    }

    if best > 0.0 {
        // With a clear period its multiples match about equally well, so the direction
        // decides. Without one, only the single best peak is worth anything.
        let threshold = if best >= GOOD_MATCH { best * KEY_PEAK_RATIO } else { best };
        let mut first = None;
        let mut last = start;
        for i in start..end {
            if is_peak(i) && scores[i] >= threshold {
                first.get_or_insert(i);
                last = i;
            }
        }
        let first = first.unwrap_or(last);
        return (if prefer_short { first } else { last }, first, true);
    }

    // No peak in range: take the best value there is
    let mut chosen = min_index.min(end);
    for i in chosen + 1..len {
        if scores[i] > scores[chosen] || (!prefer_short && scores[i] == scores[chosen]) {
            chosen = i;
        }
    }
    (chosen, chosen, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI as PI64;

    const SAMPLE_RATE: f32 = 44100.0;

    // Test signals are one second long. Measurements skip the first third, so the shifter
    // has settled on the note.
    const NUM_SAMPLES: usize = 44100;
    const SETTLE: usize = 14700;

    // Just intonation, so partials of different notes either coincide or are far apart
    const POWER_CHORD: [f64; 3] = [82.41, 123.615, 164.82];
    const MAJOR_CHORD: [f64; 6] = [82.41, 123.615, 164.82, 206.025, 247.23, 329.64];

    fn new_shifter(semitones: i32) -> PitchShifter {
        let mut shifter = PitchShifter::new();
        shifter.set_sample_rate(SAMPLE_RATE);
        shifter.set_semitones(semitones);
        shifter
    }

    fn shift_ratio(semitones: i32) -> f64 {
        2.0_f64.powf(semitones as f64 / 12.0)
    }

    fn generate_sine_wave(frequency: f64, sample_rate: f32, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| (2.0 * PI64 * frequency * i as f64 / sample_rate as f64).sin() as f32)
            .collect()
    }

    /// Steady tone with harmonics falling off as 1/n
    fn generate_harmonic_tone(frequency: f64, harmonics: usize, num_samples: usize) -> Vec<f32> {
        let scale: f64 = (1..=harmonics).map(|h| 1.0 / h as f64).sum();
        (0..num_samples)
            .map(|i| {
                let phase = 2.0 * PI64 * frequency * i as f64 / SAMPLE_RATE as f64;
                let sum: f64 = (1..=harmonics).map(|h| (phase * h as f64).sin() / h as f64).sum();
                (sum / scale) as f32
            })
            .collect()
    }

    fn generate_chord(notes: &[f64], harmonics: usize, num_samples: usize) -> Vec<f32> {
        let mut chord = vec![0.0; num_samples];
        for &note in notes {
            for (sum, sample) in chord.iter_mut().zip(generate_harmonic_tone(note, harmonics, num_samples)) {
                *sum += sample / notes.len() as f32;
            }
        }
        chord
    }

    /// Plucked string (Karplus-Strong), for a guitar-like attack and decay
    fn generate_pluck(frequency: f64, sample_rate: f32, num_samples: usize) -> Vec<f32> {
        let period = (sample_rate as f64 / frequency - 0.5).round() as usize;
        let mut seed = 12345_u32;
        let mut line: Vec<f32> = (0..period)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 9) as f32 / (1 << 22) as f32 - 1.0
            })
            .collect();

        let mut pos = 0;
        (0..num_samples)
            .map(|_| {
                let next = (pos + 1) % period;
                let sample = line[pos];
                line[pos] = 0.998 * 0.5 * (sample + line[next]);
                pos = next;
                sample * 0.5
            })
            .collect()
    }

    fn generate_noise(num_samples: usize) -> Vec<f32> {
        let mut seed = 987654321_u32;
        (0..num_samples)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 9) as f32 / (1 << 22) as f32 - 1.0
            })
            .collect()
    }

    fn process_buffer(shifter: &mut PitchShifter, input: &[f32]) -> Vec<f32> {
        input.iter().map(|&s| shifter.process_sample(s)).collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// The partials a shifted tone should consist of
    fn shifted_partials(notes: &[f64], harmonics: usize, semitones: i32, sample_rate: f32) -> Vec<f64> {
        let mut partials: Vec<f64> = notes
            .iter()
            .flat_map(|&note| (1..=harmonics).map(move |h| note * h as f64))
            .map(|frequency| frequency * shift_ratio(semitones))
            .filter(|&frequency| frequency < sample_rate as f64 * 0.45)
            .collect();
        partials.sort_by(|a, b| a.partial_cmp(b).unwrap());
        partials.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        partials
    }

    /// Energy that is not at any of the expected frequencies, relative to the total, in dB.
    /// Lower is cleaner. Each expected partial is fitted by least squares and removed; what
    /// remains is the sidebands, clicks and noise the shifter added.
    fn artifact_ratio_db(signal: &[f32], sample_rate: f32, partials: &[f64]) -> f64 {
        let len = signal.len();
        let window: Vec<f64> = (0..len)
            .map(|i| 0.5 - 0.5 * (2.0 * PI64 * i as f64 / len as f64).cos())
            .collect();
        let mut residual: Vec<f64> = signal.iter().map(|&s| s as f64).collect();
        let energy = |samples: &[f64]| -> f64 {
            samples.iter().zip(&window).map(|(s, w)| w * s * s).sum()
        };
        let total = energy(&residual);

        for _ in 0..2 {
            for &frequency in partials {
                let omega = 2.0 * PI64 * frequency / sample_rate as f64;
                let (mut cc, mut ss, mut cs, mut yc, mut ys) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for i in 0..len {
                    let (sin, cos) = (omega * i as f64).sin_cos();
                    cc += window[i] * cos * cos;
                    ss += window[i] * sin * sin;
                    cs += window[i] * cos * sin;
                    yc += window[i] * residual[i] * cos;
                    ys += window[i] * residual[i] * sin;
                }
                let det = cc * ss - cs * cs;
                let a = (yc * ss - ys * cs) / det;
                let b = (ys * cc - yc * cs) / det;
                for i in 0..len {
                    let (sin, cos) = (omega * i as f64).sin_cos();
                    residual[i] -= a * cos + b * sin;
                }
            }
        }

        10.0 * (energy(&residual) / total).max(1e-20).log10()
    }

    /// Shifts a steady tone and measures how much of the output is not the shifted tone
    fn measure_artifacts(notes: &[f64], harmonics: usize, semitones: i32, latency_ms: f32) -> f64 {
        let mut shifter = new_shifter(semitones);
        shifter.set_latency_ms(latency_ms);
        let output = process_buffer(&mut shifter, &generate_chord(notes, harmonics, NUM_SAMPLES));
        let partials = shifted_partials(notes, harmonics, semitones, SAMPLE_RATE);
        artifact_ratio_db(&output[SETTLE..], SAMPLE_RATE, &partials)
    }

    #[test]
    fn test_passthrough_is_exact_at_zero_semitones() {
        let mut shifter = new_shifter(0);

        let input = generate_harmonic_tone(110.0, 10, 4000);
        let output = process_buffer(&mut shifter, &input);

        let delay = MIN_DELAY as usize;
        for i in delay..input.len() {
            assert_eq!(output[i], input[i - delay], "Sample {} was altered", i);
        }
    }

    #[test]
    fn test_returns_to_passthrough_after_shifting() {
        let mut shifter = new_shifter(-3);

        let input = generate_harmonic_tone(110.0, 10, 30000);
        let mut output = process_buffer(&mut shifter, &input[..20000]);
        shifter.set_semitones(0);
        output.extend(process_buffer(&mut shifter, &input[20000..]));

        // One crossfade back to the minimum delay, then untouched again
        let delay = MIN_DELAY as usize;
        for i in 20000 + shifter.crossfade_len + 1..input.len() {
            assert_eq!(output[i], input[i - delay], "Sample {} was altered", i);
        }
    }

    #[test]
    fn test_sine_stays_clean_at_all_semitones() {
        for semitones in -12..=12 {
            for note in [82.41, 196.0, 659.26] {
                let artifacts = measure_artifacts(&[note], 1, semitones, 15.0);
                assert!(
                    artifacts < -45.0,
                    "Semitones={}, {} Hz: artifacts at {:.1} dB",
                    semitones,
                    note,
                    artifacts
                );
            }
        }
    }

    #[test]
    fn test_harmonic_tone_stays_clean() {
        for semitones in [-12, -7, -5, -2, -1, 1, 2, 5, 7, 12] {
            let artifacts = measure_artifacts(&[110.0], 10, semitones, 15.0);
            assert!(
                artifacts < -40.0,
                "Semitones={}: artifacts at {:.1} dB",
                semitones,
                artifacts
            );
        }
    }

    #[test]
    fn test_chord_stays_clean_when_its_period_fits() {
        // The notes of a power chord repeat together every 24.3 ms
        for semitones in [-12, -5, -2, 2, 7, 12] {
            let artifacts = measure_artifacts(&POWER_CHORD, 6, semitones, 30.0);
            assert!(
                artifacts < -45.0,
                "Semitones={}: artifacts at {:.1} dB",
                semitones,
                artifacts
            );
        }
    }

    #[test]
    fn test_stays_clean_at_other_sample_rates() {
        for sample_rate in [48000.0_f32, 96000.0] {
            for semitones in [-2, 7] {
                let mut shifter = PitchShifter::new();
                shifter.set_sample_rate(sample_rate);
                shifter.set_semitones(semitones);

                let num_samples = sample_rate as usize;
                let input = generate_sine_wave(110.0, sample_rate, num_samples);
                let output = process_buffer(&mut shifter, &input);

                let partials = shifted_partials(&[110.0], 1, semitones, sample_rate);
                let artifacts = artifact_ratio_db(&output[num_samples / 3..], sample_rate, &partials);
                assert!(
                    artifacts < -45.0,
                    "{} Hz, semitones={}: artifacts at {:.1} dB",
                    sample_rate,
                    semitones,
                    artifacts
                );
            }
        }
    }

    #[test]
    fn test_level_is_preserved() {
        for semitones in [-12, -2, 2, 12] {
            let mut shifter = new_shifter(semitones);

            let input = generate_sine_wave(196.0, SAMPLE_RATE, NUM_SAMPLES);
            let output = process_buffer(&mut shifter, &input);

            let ratio = rms(&output[SETTLE..]) / rms(&input[SETTLE..]);
            assert!(
                (ratio - 1.0).abs() < 0.02,
                "Semitones={}: level changed by a factor of {}",
                semitones,
                ratio
            );
        }
    }

    #[test]
    fn test_latency_follows_pitch_period() {
        // 329.63 Hz has a period of 134 samples, far below the 15 ms (661 sample) limit
        for semitones in [-2, 2] {
            let mut shifter = new_shifter(semitones);

            let input = generate_sine_wave(329.63, SAMPLE_RATE, NUM_SAMPLES);
            let mut longest: f64 = 0.0;
            for (i, &sample) in input.iter().enumerate() {
                shifter.process_sample(sample);
                if i > SETTLE {
                    longest = longest.max(shifter.delay);
                }
            }

            assert!(
                longest < 2.0 * 134.0,
                "Semitones={}: delay reached {} samples",
                semitones,
                longest
            );
        }
    }

    #[test]
    fn test_delay_stays_within_limits() {
        // Noise has no period to lock on to, so every splice is a forced one
        let input = generate_noise(20000);

        for semitones in -12..=12 {
            let mut shifter = new_shifter(semitones);

            for &sample in &input {
                let output = shifter.process_sample(sample);
                assert!(output.is_finite());
                assert!(
                    shifter.delay >= MIN_DELAY && shifter.delay <= shifter.max_delay + 1.0,
                    "Semitones={}: delay {} outside {}..{}",
                    semitones,
                    shifter.delay,
                    MIN_DELAY,
                    shifter.max_delay
                );
            }
        }
    }

    #[test]
    fn test_quiet_input_keeps_latency_low() {
        let mut shifter = new_shifter(-5);

        for _ in 0..20000 {
            shifter.process_sample(0.0);
            assert!(
                shifter.delay <= shifter.quiet_delay + 1.0,
                "Delay grew to {} samples during silence",
                shifter.delay
            );
        }
    }

    #[test]
    fn test_quiet_input_holds_head_back_when_shifting_up() {
        let mut shifter = new_shifter(2);

        for i in 0..20000 {
            shifter.process_sample(0.0);
            if i > 1000 {
                assert!(
                    shifter.delay > shifter.hold_delay - shifter.min_lag as f64 - 1.0
                        && shifter.delay <= shifter.hold_delay,
                    "Delay {} strayed from the hold position {}",
                    shifter.delay,
                    shifter.hold_delay
                );
            }
        }
    }

    #[test]
    fn test_stereo_channels_share_one_read_head() {
        let mut shifter = new_shifter(5);

        let input = generate_harmonic_tone(110.0, 10, 10000);
        for &sample in &input {
            let (left, right) = shifter.process_stereo(sample, 0.5 * sample);
            assert!(
                (right - 0.5 * left).abs() < 1e-6,
                "Channels drifted apart: left={}, right={}",
                left,
                right
            );
        }
    }

    #[test]
    fn test_no_clicks_on_parameter_changes() {
        let mut shifter = new_shifter(0);

        let input = generate_sine_wave(220.0, SAMPLE_RATE, 60000);
        let mut previous = 0.0;

        for (i, &sample) in input.iter().enumerate() {
            if i % 1500 == 0 {
                shifter.set_semitones(((i / 1500 * 7) % 25) as i32 - 12);
            }
            if i % 2300 == 0 {
                shifter.set_latency_ms(((i / 2300 * 11) % 48) as f32 + 2.0);
            }
            if i % 3100 == 0 {
                shifter.set_smoothness_ms(((i / 3100 * 3) % 9) as f32 + 1.0);
            }

            let output = shifter.process_sample(sample);
            assert!(output.is_finite() && output.abs() < 1.5, "Sample {}: output {}", i, output);

            // A clean 440 Hz sine moves at most 0.063 per sample; a click is several times that
            let step = (output - previous).abs();
            assert!(step < 0.25, "Sample {}: jumped by {}", i, step);
            previous = output;
        }
    }

    #[test]
    fn test_reset_clears_state() {
        let mut shifter = new_shifter(5);

        let input = generate_sine_wave(440.0, SAMPLE_RATE, 2000);
        let _ = process_buffer(&mut shifter, &input);

        shifter.reset();

        assert_eq!(shifter.write_pos, 0);
        assert_eq!(shifter.delay, MIN_DELAY);
        assert_eq!(shifter.fade_remaining, 0);
        for _ in 0..2000 {
            assert_eq!(shifter.process_sample(0.0), 0.0);
        }
    }

    #[test]
    fn test_playback_rate_calculation() {
        let mut shifter = PitchShifter::new();

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
    fn test_fade_gains_keep_level_constant() {
        let mut shifter = PitchShifter::new();
        shifter.fade_len = 100;

        for remaining in 1..=100 {
            shifter.fade_remaining = remaining;

            // Matched heads add in amplitude
            shifter.fade_match = 1.0;
            let (fade_out, fade_in) = shifter.fade_gains();
            assert!((fade_out + fade_in - 1.0).abs() < 0.001);

            // Unrelated heads add in power
            shifter.fade_match = 0.0;
            let (fade_out, fade_in) = shifter.fade_gains();
            assert!((fade_out * fade_out + fade_in * fade_in - 1.0).abs() < 0.001);
        }

        // The fade ends on the incoming head
        shifter.fade_remaining = 1;
        let (fade_out, fade_in) = shifter.fade_gains();
        assert!(fade_out < 0.001 && fade_in > 0.999);
    }

    #[test]
    fn test_cubic_interpolation() {
        let mut shifter = new_shifter(0);

        // Fill the buffer with a simple ramp; the newest sample is 99
        for i in 0..100 {
            shifter.process_sample(i as f32);
        }

        assert_eq!(shifter.read_cubic(0, 10.0), 89.0);

        // For a linear ramp, cubic interpolation should land halfway
        let val = shifter.read_cubic(0, 10.5);
        assert!((val - 88.5).abs() < 0.01, "Cubic interpolation value: {}", val);
    }

    #[test]
    fn test_pick_peak_finds_the_period() {
        // Correlation of a signal with a period of 20 lags: peaks at lag 20, 40 and 60
        let scores: Vec<f32> = (1..=70)
            .map(|lag| (2.0 * PI * lag as f32 / 20.0).cos())
            .collect();

        // Shifting up takes the shortest jump, shifting down the longest
        assert_eq!(pick_peak(&scores, 0, true), (19, 19, true));
        assert_eq!(pick_peak(&scores, 0, false), (59, 19, true));

        // Lags below the minimum are not eligible
        assert_eq!(pick_peak(&scores, 25, true), (39, 39, true));

        // The falling slope next to lag zero is not a match
        let slope: Vec<f32> = (1..=10).map(|lag| 1.0 - 0.01 * lag as f32).collect();
        let (_, _, is_peak) = pick_peak(&slope, 0, true);
        assert!(!is_peak);
    }

    /// Prints artifact levels and latency for a range of notes and shifts. Use it to compare
    /// before and after changing the algorithm or its constants:
    ///   cargo test --release quality_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn quality_report() {
        let shifts = [-12, -7, -5, -2, -1, 0, 1, 2, 5, 7, 12];
        let notes = [82.41, 110.0, 196.0, 329.63, 659.26];

        println!("Artifacts in dB (lower is cleaner), sines and a 110 Hz harmonic tone");
        println!("{:>4}{:>8}{:>8}{:>8}{:>8}{:>8}{:>10}", "st", 82, 110, 196, 330, 659, "harmonic");
        for semitones in shifts {
            print!("{:>4}", semitones);
            for note in notes {
                print!("{:>8.1}", measure_artifacts(&[note], 1, semitones, 15.0));
            }
            println!("{:>10.1}", measure_artifacts(&[110.0], 10, semitones, 15.0));
        }

        println!("Artifacts in dB, power chord and major chord at 15 and 30 ms max latency");
        println!("{:>4}{:>10}{:>10}{:>10}{:>10}", "st", "power/15", "major/15", "power/30", "major/30");
        for semitones in shifts {
            print!("{:>4}", semitones);
            for latency_ms in [15.0, 30.0] {
                print!("{:>10.1}", measure_artifacts(&POWER_CHORD, 6, semitones, latency_ms));
                print!("{:>10.1}", measure_artifacts(&MAJOR_CHORD, 6, semitones, latency_ms));
            }
            println!();
        }

        println!("Longest delay in ms once settled");
        for semitones in shifts {
            print!("{:>4}", semitones);
            for note in notes {
                let mut shifter = new_shifter(semitones);
                let mut longest: f64 = 0.0;
                let input = generate_sine_wave(note, SAMPLE_RATE, NUM_SAMPLES);
                for (i, &sample) in input.iter().enumerate() {
                    shifter.process_sample(sample);
                    if i > SETTLE {
                        longest = longest.max(shifter.delay);
                    }
                }
                print!("{:>8.2}", longest / SAMPLE_RATE as f64 * 1000.0);
            }
            println!();
        }
    }

    /// Writes before/after WAV files to target/renders for listening:
    ///   cargo test --release render_wavs -- --ignored
    ///
    /// Set PITCH_SHIFTER_INPUT_WAV to the path of a recording to render that as well, and
    /// PITCH_SHIFTER_LATENCY_MS to try another max latency.
    #[test]
    #[ignore]
    fn render_wavs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/renders");
        std::fs::create_dir_all(&dir).unwrap();

        let latency_ms: Option<f32> = std::env::var("PITCH_SHIFTER_LATENCY_MS")
            .ok()
            .map(|value| value.parse().expect("PITCH_SHIFTER_LATENCY_MS must be a number"));

        // A phrase of single notes across the strings, then a strummed E major chord
        let note_len = (SAMPLE_RATE * 0.5) as usize;
        let mut phrase = vec![0.0; note_len * 7];
        for (i, note) in [82.41, 110.0, 146.83, 196.0, 246.94, 329.63].iter().enumerate() {
            for (j, sample) in generate_pluck(*note, SAMPLE_RATE, note_len).iter().enumerate() {
                phrase[i * note_len + j] += sample;
            }
        }
        let strum_gap = (SAMPLE_RATE * 0.015) as usize;
        let mut chord = vec![0.0; note_len * 5];
        for (i, note) in [82.41, 123.47, 164.81, 207.65, 246.94, 329.63].iter().enumerate() {
            let start = i * strum_gap;
            let pluck = generate_pluck(*note, SAMPLE_RATE, chord.len() - start);
            for (j, sample) in pluck.iter().enumerate() {
                chord[start + j] += sample / 3.0;
            }
        }

        let mut sources = vec![
            ("notes".to_string(), phrase, SAMPLE_RATE as u32),
            ("chord".to_string(), chord, SAMPLE_RATE as u32),
        ];
        if let Ok(path) = std::env::var("PITCH_SHIFTER_INPUT_WAV") {
            let (samples, sample_rate) = read_wav_mono(&path);
            sources.push(("input".to_string(), samples, sample_rate));
        }

        for (name, input, sample_rate) in &sources {
            write_wav(&dir.join(format!("{}_original.wav", name)), input, *sample_rate);
            for semitones in [-12, -5, -2, -1, 0, 2, 7, 12] {
                let mut shifter = PitchShifter::new();
                shifter.set_sample_rate(*sample_rate as f32);
                shifter.set_semitones(semitones);
                if let Some(latency_ms) = latency_ms {
                    shifter.set_latency_ms(latency_ms);
                }
                let output = process_buffer(&mut shifter, input);
                write_wav(&dir.join(format!("{}_{:+}.wav", name, semitones)), &output, *sample_rate);
            }
        }
        println!("Wrote renders to {}", dir.display());
    }

    fn write_wav(path: &std::path::Path, samples: &[f32], sample_rate: u32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &sample in samples {
            writer.write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Reads a WAV file and mixes it down to mono
    fn read_wav_mono(path: &str) -> (Vec<f32>, u32) {
        let mut reader = hound::WavReader::open(path).expect("Could not open PITCH_SHIFTER_INPUT_WAV");
        let spec = reader.spec();
        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
            hound::SampleFormat::Int => {
                let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
                reader.samples::<i32>().map(|s| s.unwrap() as f32 / scale).collect()
            }
        };
        let channels = spec.channels as usize;
        let mono = interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();
        (mono, spec.sample_rate)
    }
}
