use crate::dsp::OnePole;

const LINES: usize = 8;
// Mutually unrelated lengths so the echoes do not line up
const DELAYS_MS: [f32; LINES] = [11.3, 14.9, 19.1, 23.3, 28.7, 33.1, 38.9, 44.3];
// Input diffusers per channel: they smear a drum hit so the room does not answer with
// discrete echoes
const DIFFUSERS_MS: [[f32; 2]; 2] = [[4.7, 1.9], [5.3, 2.3]];
const DIFFUSER_GAIN: f32 = 0.6;
// High frequencies die faster than low ones, as in a real room
const DAMPING_HZ: f32 = 6000.0;
const WET_GAIN: f32 = 0.3;

struct Allpass {
    buffer: Vec<f32>,
    position: usize,
}

impl Allpass {
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.position];
        let output = delayed - DIFFUSER_GAIN * input;
        self.buffer[self.position] = input + DIFFUSER_GAIN * output;
        self.position = (self.position + 1) % self.buffer.len();
        output
    }
}

/// Small stereo room: a feedback delay network of eight lines.
///
/// The delay lines are allocated in `set_sample_rate` and never resized while processing.
pub struct Room {
    sample_rate: f32,
    decay_s: f32,
    lines: [Vec<f32>; LINES],
    positions: [usize; LINES],
    feedback: [f32; LINES],
    damping: [OnePole; LINES],
    diffusers: [[Allpass; 2]; 2],
}

impl Room {
    pub fn new() -> Self {
        let mut room = Self {
            sample_rate: 44100.0,
            decay_s: 0.5,
            lines: std::array::from_fn(|_| Vec::new()),
            positions: [0; LINES],
            feedback: [0.0; LINES],
            damping: std::array::from_fn(|_| OnePole::new()),
            diffusers: std::array::from_fn(|_| {
                std::array::from_fn(|_| Allpass {
                    buffer: Vec::new(),
                    position: 0,
                })
            }),
        };
        room.set_sample_rate(44100.0);
        room
    }

    /// Allocates the delay lines. Not for the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;

        for (line, delay_ms) in self.lines.iter_mut().zip(DELAYS_MS) {
            *line = vec![0.0; ms_to_samples(delay_ms, sample_rate)];
        }
        for (channel, delays_ms) in self.diffusers.iter_mut().zip(DIFFUSERS_MS) {
            for (diffuser, delay_ms) in channel.iter_mut().zip(delays_ms) {
                diffuser.buffer = vec![0.0; ms_to_samples(delay_ms, sample_rate)];
                diffuser.position = 0;
            }
        }
        for filter in &mut self.damping {
            filter.set(DAMPING_HZ, sample_rate);
            filter.reset();
        }
        self.positions = [0; LINES];
        self.set_decay(self.decay_s);
    }

    /// Reverb time to -60 dB
    pub fn set_decay(&mut self, decay_s: f32) {
        self.decay_s = decay_s;
        for (feedback, delay_ms) in self.feedback.iter_mut().zip(DELAYS_MS) {
            *feedback = 10.0f32.powf(-3.0 * delay_ms * 0.001 / decay_s);
        }
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        for diffuser in self.diffusers.iter_mut().flatten() {
            diffuser.buffer.fill(0.0);
        }
        for filter in &mut self.damping {
            filter.reset();
        }
    }

    /// Returns the wet signal only
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let mut inputs = [left, right];
        for (input, channel) in inputs.iter_mut().zip(self.diffusers.iter_mut()) {
            for diffuser in channel {
                *input = diffuser.process(*input);
            }
        }

        let mut outputs = [0.0; LINES];
        let mut fed_back = [0.0; LINES];
        let mut sum = 0.0;
        for i in 0..LINES {
            outputs[i] = self.lines[i][self.positions[i]];
            fed_back[i] = self.damping[i].process(outputs[i]) * self.feedback[i];
            sum += fed_back[i];
        }

        // Householder reflection: every line feeds every other line, without gaining energy
        let reflection = sum * (2.0 / LINES as f32);
        for i in 0..LINES {
            self.lines[i][self.positions[i]] = inputs[i % 2] + fed_back[i] - reflection;
            self.positions[i] = (self.positions[i] + 1) % self.lines[i].len();
        }

        // Even lines to the left, odd lines to the right, alternating signs to decorrelate
        let wet_left = outputs[0] - outputs[2] + outputs[4] - outputs[6];
        let wet_right = outputs[1] - outputs[3] + outputs[5] - outputs[7];
        (wet_left * WET_GAIN, wet_right * WET_GAIN)
    }
}

fn ms_to_samples(time_ms: f32, sample_rate: f32) -> usize {
    ((time_ms * 0.001 * sample_rate) as usize).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{decay_time_s, peak};

    const SAMPLE_RATE: f32 = 44100.0;

    fn impulse_response(room: &mut Room, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| {
                let input = if i == 0 { 1.0 } else { 0.0 };
                room.process(input, input).0
            })
            .collect()
    }

    #[test]
    fn test_silent_without_input() {
        let mut room = Room::new();

        for _ in 0..4096 {
            assert_eq!(room.process(0.0, 0.0), (0.0, 0.0));
        }
    }

    #[test]
    fn test_tail_follows_decay_time() {
        let mut short_room = Room::new();
        let mut long_room = Room::new();
        short_room.set_decay(0.3);
        long_room.set_decay(0.9);

        let short = impulse_response(&mut short_room, SAMPLE_RATE as usize * 3);
        let long = impulse_response(&mut long_room, SAMPLE_RATE as usize * 3);

        assert!(peak(&short) > 0.0 && peak(&short) <= 1.0);
        assert!(short.iter().all(|s| s.is_finite()));
        let short_time = decay_time_s(&short, SAMPLE_RATE, -40.0);
        let long_time = decay_time_s(&long, SAMPLE_RATE, -40.0);
        assert!(long_time > short_time * 1.5, "Decay times: {} s, {} s", short_time, long_time);
        assert!(peak(&long[SAMPLE_RATE as usize * 2..]) < 0.001);
    }

    #[test]
    fn test_reset_clears_tail() {
        let mut room = Room::new();
        impulse_response(&mut room, 1000);

        room.reset();

        assert_eq!(room.process(0.0, 0.0), (0.0, 0.0));
    }
}
