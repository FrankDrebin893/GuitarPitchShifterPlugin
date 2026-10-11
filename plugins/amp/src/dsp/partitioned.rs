//! The late part of a long impulse response, convolved in blocks by way of the Fourier
//! transform: many times less work than multiplying every tap with a sample, which is
//! what a response of 40 ms at 192 kHz costs most of.
//!
//! The response is cut into a head, which is convolved tap by tap as before and adds no
//! latency, and a tail in partitions of one block each. Every time a block of input is
//! complete, the last two blocks are transformed; each partition's transform (worked out
//! beforehand, off the audio thread: `partition`) is multiplied with the transform of the
//! input from as many blocks ago as the partition is late, the products are summed, and
//! the sum is transformed back. That is the tail's share of one block of output.
//!
//! The head is two blocks long. So the block of output that the input up to now is needed
//! for only starts one block from now, and the work for it is spread evenly over the
//! samples until then: every sample does its share, and no sample or host block carries a
//! transform by itself that the others do not. What comes out does not depend on how the
//! work is spread.
//!
//! One `Tail` serves both of a cabinet's slots: they hear the same input, so its
//! transforms are shared.

use super::fft::RealFft;

/// A partition is at least this many samples
const MIN_BLOCK: usize = 64;

/// The most partitions a tail has. With a block of `MIN_BLOCK` that covers the longest
/// response at 96 kHz, and above that the block is doubled. Measured in
/// `convolver_report`: half as many, in blocks twice as long, cost more at every rate,
/// because the head, which is two blocks, grows with them
pub const MAX_PARTITIONS: usize = 60;

/// The largest block: what the longest response the cabinet holds needs
const MAX_BLOCK: usize = 128;

/// Numbers in the transform of a partition: the real and the imaginary parts of the
/// frequencies from zero to half the sample rate
const fn spectrum_len(block: usize) -> usize {
    2 * (block + 1)
}

/// Room for the transforms of all partitions of the longest response
pub const MAX_SPECTRA_LEN: usize = MAX_PARTITIONS * spectrum_len(MAX_BLOCK);

/// What a transform costs, counted in multiplications of one partition: the share of the
/// work a sample does is counted in these, so the transforms weigh what they take
const TRANSFORM_COST: usize = 6;

/// The length of a block for responses of up to `longest` samples: the shortest that
/// covers them in `MAX_PARTITIONS` partitions behind the head
pub fn block_len(longest: usize) -> usize {
    let mut block = MIN_BLOCK;
    while block < MAX_BLOCK && longest > (MAX_PARTITIONS + 2) * block {
        block *= 2;
    }
    block
}

/// The samples of a response that are convolved tap by tap when it has a tail
pub fn head_len(block: usize) -> usize {
    2 * block
}

/// A response is given a tail from this many samples on. Below it the transforms cost
/// more than the taps they save
pub fn shortest_with_tail(block: usize) -> usize {
    4 * block + 1
}

/// The transforms of the partitions of a response's tail, one after the other in
/// `spectra`, for a `Tail` with this block length. Returns how many numbers that is:
/// none if the response is too short for a tail. Allocates: not for the audio thread
pub fn partition(taps: &[f32], block: usize, spectra: &mut [f32]) -> usize {
    if taps.len() < shortest_with_tail(block) {
        return 0;
    }
    let tail = &taps[head_len(block)..];
    let partitions = tail.len().div_ceil(block).min(MAX_PARTITIONS);
    let mut fft = RealFft::new(2 * block);
    let mut padded = vec![0.0; 2 * block];
    for (taps, spectrum) in tail.chunks(block).zip(spectra.chunks_exact_mut(spectrum_len(block))).take(partitions) {
        padded.fill(0.0);
        padded[..taps.len()].copy_from_slice(taps);
        let (re, im) = spectrum.split_at_mut(block + 1);
        fft.forward(&padded, re, im);
    }
    partitions * spectrum_len(block)
}

/// Where one cabinet's tail is on its way to being heard
#[derive(Clone, Copy, PartialEq)]
enum Progress {
    /// No tail: nothing is worked out for it
    Off,
    /// Waits for a block to end, and for as many blocks of input as it has partitions
    Waiting,
    /// Its first block of output is being worked out
    Starting,
    /// `now` holds its share of the output
    Playing,
}

/// The tail of one of the cabinet's two slots
struct Line {
    // The transforms of its partitions
    spectra: Vec<f32>,
    partitions: usize,
    progress: Progress,
    // Its share of this block of output, and of the next as far as that is worked out
    now: Vec<f32>,
    next: Vec<f32>,
    // Whether the work under way includes it, with this many partitions, and whether
    // that work is still wanted: not if the slot was given another cabinet meanwhile.
    // The steps stay where they are, so the work is as evenly spread as it was
    working: bool,
    working_partitions: usize,
    stale: bool,
}

impl Line {
    fn new() -> Self {
        Self {
            spectra: vec![0.0; MAX_SPECTRA_LEN],
            partitions: 0,
            progress: Progress::Off,
            now: vec![0.0; MAX_BLOCK],
            next: vec![0.0; MAX_BLOCK],
            working: false,
            working_partitions: 0,
            stale: false,
        }
    }
}

/// The tails of the cabinet's two slots
pub struct Tail {
    block: usize,
    fft: RealFft,
    // The transforms of the input, two blocks each, one per block: a ring, `newest` last
    inputs: Vec<f32>,
    newest: usize,
    // How many of them hold what was played, counted from the newest back
    known: usize,
    lines: [Line; 2],
    // Samples into the block
    position: usize,
    // The two blocks the work under way started from, and the sum of the products
    samples: Vec<f32>,
    sum: Vec<f32>,
    // The work under way: the step that comes next, and what the samples so far have
    // paid towards it. A step is done once it is paid for in full, so the last sample
    // of the block pays for the last step
    step: usize,
    steps: usize,
    cost: usize,
    paid: usize,
}

impl Tail {
    pub fn new() -> Self {
        Self {
            block: MIN_BLOCK,
            fft: RealFft::new(2 * MIN_BLOCK),
            inputs: vec![0.0; MAX_SPECTRA_LEN],
            newest: 0,
            known: MAX_PARTITIONS,
            lines: [Line::new(), Line::new()],
            position: 0,
            samples: vec![0.0; 2 * MAX_BLOCK],
            sum: vec![0.0; spectrum_len(MAX_BLOCK)],
            step: 0,
            steps: 0,
            cost: 0,
            paid: 0,
        }
    }

    /// Sets the block length. Allocates: not for the audio thread
    pub fn set_block(&mut self, block: usize) {
        debug_assert!(block.is_power_of_two() && (MIN_BLOCK..=MAX_BLOCK).contains(&block));
        if block != self.block {
            self.block = block;
            self.fft = RealFft::new(2 * block);
        }
        for line in &mut self.lines {
            line.partitions = 0;
            line.progress = Progress::Off;
        }
        self.reset();
    }

    pub fn block(&self) -> usize {
        self.block
    }

    /// As if nothing had ever been played. A tail that is loaded plays from here on
    pub fn reset(&mut self) {
        self.inputs.fill(0.0);
        self.known = MAX_PARTITIONS;
        self.position = 0;
        self.stop_work();
        for line in &mut self.lines {
            line.now.fill(0.0);
            line.next.fill(0.0);
            if line.progress != Progress::Off {
                line.progress = Progress::Playing;
            }
        }
    }

    /// Drops the work under way
    fn stop_work(&mut self) {
        self.steps = 0;
        self.step = 0;
        self.cost = 0;
        self.paid = 0;
        for line in &mut self.lines {
            line.working = false;
        }
    }

    /// True while a tail is loaded: `advance` then has work to do
    pub fn is_running(&self) -> bool {
        self.lines.iter().any(|line| line.progress != Progress::Off)
    }

    /// Gives a slot a tail from what `partition` made, or none if `spectra` is empty.
    /// It is heard once `is_playing`. Does not allocate
    pub fn load(&mut self, slot: usize, spectra: &[f32]) {
        let was_running = self.is_running();
        let line = &mut self.lines[slot];
        // What is being worked out for the cabinet that was here is of no use any more
        line.stale = true;
        line.now.fill(0.0);
        line.next.fill(0.0);
        line.partitions = (spectra.len() / spectrum_len(self.block)).min(MAX_PARTITIONS);
        let len = line.partitions * spectrum_len(self.block);
        line.spectra[..len].copy_from_slice(&spectra[..len]);
        line.progress = if line.partitions == 0 { Progress::Off } else { Progress::Waiting };

        if !was_running && self.is_running() {
            // Blocks start here, and nothing is known yet of what was played
            self.position = 0;
            self.stop_work();
            self.known = 0;
        }
    }

    /// Takes a slot's tail away
    pub fn unload(&mut self, slot: usize) {
        self.lines[slot].progress = Progress::Off;
        self.lines[slot].stale = true;
    }

    /// True if the slot has a tail, heard or not yet
    pub fn has(&self, slot: usize) -> bool {
        self.lines[slot].progress != Progress::Off
    }

    /// True once a slot's tail gives its share of the output
    pub fn is_playing(&self, slot: usize) -> bool {
        self.lines[slot].progress == Progress::Playing
    }

    /// Samples until a tail that was just loaded is heard, at most
    pub fn wait_len(&self, slot: usize) -> usize {
        let line = &self.lines[slot];
        match line.progress {
            Progress::Waiting => {
                // It joins the work at the end of the block that brings the number of
                // known transforms to its number of partitions, and plays a block later
                let blocks_to_join = line.partitions.saturating_sub(self.known).max(1);
                (blocks_to_join + 1) * self.block - self.position
            }
            Progress::Starting => 2 * self.block - self.position,
            _ => 0,
        }
    }

    /// The tail's share of this sample of a slot's output
    pub fn output(&self, slot: usize) -> f32 {
        self.lines[slot].now[self.position]
    }

    /// Moves on by one sample: does that sample's share of the work, and at the end of a
    /// block starts the work for the block after the next. `latest` ends with the two
    /// blocks of input up to and with this sample. Does not allocate
    pub fn advance(&mut self, latest: &[f32]) {
        self.paid += self.cost;
        while self.step < self.steps {
            let due = self.step_cost(self.step) * self.block;
            if self.paid < due {
                break;
            }
            self.paid -= due;
            self.work(self.step);
            self.step += 1;
        }

        self.position += 1;
        if self.position == self.block {
            self.position = 0;
            self.start_block(&latest[latest.len() - 2 * self.block..]);
        }
    }

    /// The steps of the work for a block: the transform of the input, then for each line
    /// that is worked on one multiplication per partition and the transform back
    fn step_cost(&self, step: usize) -> usize {
        let mut first = 1;
        if step < first {
            return TRANSFORM_COST;
        }
        for line in &self.lines {
            if line.working {
                if step < first + line.working_partitions {
                    return 1;
                }
                if step == first + line.working_partitions {
                    return TRANSFORM_COST;
                }
                first += line.working_partitions + 1;
            }
        }
        0
    }

    fn work(&mut self, step: usize) {
        let bins = self.block + 1;
        let spectrum = spectrum_len(self.block);
        if step == 0 {
            self.newest = (self.newest + 1) % MAX_PARTITIONS;
            let (re, im) = self.inputs[self.newest * spectrum..(self.newest + 1) * spectrum].split_at_mut(bins);
            self.fft.forward(&self.samples[..2 * self.block], re, im);
            self.known = (self.known + 1).min(MAX_PARTITIONS);
            return;
        }

        let mut first = 1;
        for line in &mut self.lines {
            if !line.working {
                continue;
            }
            let partitions = line.working_partitions;
            if line.stale && step <= first + partitions {
                return;
            }
            if step < first + partitions {
                // One partition times the input from as many blocks ago as it is late
                let partition = step - first;
                let from = (self.newest + MAX_PARTITIONS - partition) % MAX_PARTITIONS * spectrum;
                let (input_re, input_im) = self.inputs[from..from + spectrum].split_at(bins);
                let (tap_re, tap_im) = line.spectra[partition * spectrum..(partition + 1) * spectrum].split_at(bins);
                let (sum_re, sum_im) = self.sum[..spectrum].split_at_mut(bins);
                if partition == 0 {
                    sum_re.fill(0.0);
                    sum_im.fill(0.0);
                }
                for bin in 0..bins {
                    sum_re[bin] += input_re[bin] * tap_re[bin] - input_im[bin] * tap_im[bin];
                    sum_im[bin] += input_re[bin] * tap_im[bin] + input_im[bin] * tap_re[bin];
                }
                return;
            }
            if step == first + partitions {
                // Of the two blocks that come back, the second is the one that the
                // partitions' taps reached from within the two blocks of input
                let (sum_re, sum_im) = self.sum[..spectrum].split_at(bins);
                self.fft.inverse(sum_re, sum_im, &mut self.samples[..2 * self.block]);
                line.next[..self.block].copy_from_slice(&self.samples[self.block..2 * self.block]);
                return;
            }
            first += partitions + 1;
        }
    }

    /// The end of a block: what was worked out becomes the share of the block that
    /// starts now, and the work for the block after it is set up from `latest`, the two
    /// blocks of input up to here
    fn start_block(&mut self, latest: &[f32]) {
        debug_assert!(self.step == self.steps);
        self.steps = 1;
        self.cost = TRANSFORM_COST;
        for line in &mut self.lines {
            if line.working && !line.stale {
                std::mem::swap(&mut line.now, &mut line.next);
            }
            line.stale = false;
            match line.progress {
                Progress::Starting => line.progress = Progress::Playing,
                // With the transform of this block there are enough for all its partitions
                Progress::Waiting if self.known + 1 >= line.partitions => line.progress = Progress::Starting,
                _ => {}
            }
            line.working = matches!(line.progress, Progress::Starting | Progress::Playing);
            if line.working {
                line.working_partitions = line.partitions;
                self.steps += line.partitions + 1;
                self.cost += line.partitions + TRANSFORM_COST;
            }
        }
        self.step = 0;
        self.paid = 0;
        self.samples[..2 * self.block].copy_from_slice(latest);
    }

    /// Address and capacity of every buffer, for checking that nothing is allocated anew
    #[cfg(test)]
    pub fn buffers(&self) -> Vec<(usize, usize)> {
        let describe = |buffer: &Vec<f32>| (buffer.as_ptr() as usize, buffer.capacity());
        let mut buffers = vec![describe(&self.inputs), describe(&self.samples), describe(&self.sum)];
        for line in &self.lines {
            buffers.extend([describe(&line.spectra), describe(&line.now), describe(&line.next)]);
        }
        buffers
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

    /// A response that dies away, as long as asked for
    fn response(len: usize, seed: u32) -> Vec<f32> {
        noise(len, seed).iter().enumerate().map(|(index, tap)| 0.05 * tap * (-4.0 * index as f32 / len as f32).exp()).collect()
    }

    /// The tail's share of the output for `input`, with the line loaded before anything
    /// is played. The input is all the tail is told of: as the cabinet does it
    fn run_tail(tail: &mut Tail, taps: &[f32], input: &[f32]) -> Vec<f32> {
        let block = tail.block();
        let mut spectra = vec![0.0; MAX_SPECTRA_LEN];
        let len = partition(taps, block, &mut spectra);
        tail.load(0, &spectra[..len]);
        tail.reset();
        let mut history = vec![0.0; 2 * block];
        let mut output = Vec::new();
        for &sample in input {
            history.push(sample);
            output.push(if tail.is_playing(0) { tail.output(0) } else { f32::NAN });
            if tail.is_running() {
                tail.advance(&history);
            }
        }
        output
    }

    /// What the taps behind the head give by the plain sum
    fn expected_tail(taps: &[f32], block: usize, input: &[f32], index: usize) -> f64 {
        (head_len(block)..taps.len().min(index + 1)).map(|tap| taps[tap] as f64 * input[index - tap] as f64).sum()
    }

    #[test]
    fn test_block_follows_the_longest_response() {
        assert_eq!(block_len(1764), 64);
        assert_eq!(block_len(1920), 64);
        assert_eq!(block_len(3840), 64);
        assert_eq!(block_len(3969), 128);
        assert_eq!(block_len(7680), 128);
        for (longest, block) in [(1920, 64), (3840, 64), (7056, 128), (7680, 128)] {
            assert!((longest - head_len(block)).div_ceil(block) <= MAX_PARTITIONS);
        }
    }

    #[test]
    fn test_short_responses_get_no_tail() {
        let mut spectra = vec![0.0; MAX_SPECTRA_LEN];
        assert_eq!(partition(&response(256, 1), 64, &mut spectra), 0);
        assert_eq!(partition(&response(257, 1), 64, &mut spectra), 3 * spectrum_len(64));
        assert_eq!(partition(&response(1920, 1), 64, &mut spectra), 28 * spectrum_len(64));
        assert_eq!(partition(&response(7680, 1), 128, &mut spectra), 58 * spectrum_len(128));
        // Longer than the cabinet holds: what fits
        assert_eq!(partition(&response(9000, 1), 128, &mut spectra), MAX_SPECTRA_LEN);
    }

    #[test]
    fn test_tail_is_the_plain_sum_of_the_late_taps() {
        for (block, len) in [(64, 1920), (64, 300), (128, 7680), (64, 3840)] {
            let taps = response(len, 3);
            let input = noise(4 * len, 5);
            let mut tail = Tail::new();
            tail.set_block(block);
            // Loaded before anything is played, it plays from the first sample
            let output = run_tail(&mut tail, &taps, &input);
            for index in (0..input.len()).step_by(61) {
                let expected = expected_tail(&taps, block, &input, index);
                assert!((output[index] as f64 - expected).abs() < 2e-5, "Block {block}, {len} taps, sample {index}");
            }
        }
    }

    #[test]
    fn test_tail_loaded_while_playing_waits_until_it_knows_enough_of_the_past() {
        let block = 64;
        let taps = response(1500, 7);
        let input = noise(12_000, 9);
        let mut tail = Tail::new();
        tail.set_block(block);
        // Played into without a tail first: none of that was transformed
        let at = 3000;
        let mut history = vec![0.0; 2 * block];
        history.extend(&input[..at]);
        assert!(!tail.is_running());
        let mut spectra = vec![0.0; MAX_SPECTRA_LEN];
        let len = partition(&taps, block, &mut spectra);
        tail.load(0, &spectra[..len]);
        let wait = tail.wait_len(0);
        let partitions = (1500 - 128usize).div_ceil(64);
        assert_eq!(wait, (partitions + 1) * block);

        let mut heard_from = None;
        for index in at..input.len() {
            history.push(input[index]);
            if tail.is_playing(0) {
                heard_from.get_or_insert(index);
                // As if it had been there all along: the past is in it
                let expected = expected_tail(&taps, block, &input, index);
                assert!((tail.output(0) as f64 - expected).abs() < 2e-5, "Sample {index}");
            }
            tail.advance(&history);
        }
        assert_eq!(heard_from, Some(at + wait));

        // A second one, loaded while the first plays, needs two blocks at most
        let other = response(1900, 11);
        let len = partition(&other, block, &mut spectra);
        tail.load(1, &spectra[..len]);
        assert!(tail.wait_len(1) <= 2 * block && tail.wait_len(1) > block);
        let wait = tail.wait_len(1);
        let more = noise(2000, 13);
        let all: Vec<f32> = input.iter().chain(&more).copied().collect();
        for (offset, &sample) in more.iter().enumerate() {
            let index = input.len() + offset;
            history.push(sample);
            assert_eq!(tail.is_playing(1), offset >= wait);
            if tail.is_playing(1) {
                let expected = expected_tail(&other, block, &all, index);
                assert!((tail.output(1) as f64 - expected).abs() < 2e-5, "Second, sample {index}");
            }
            assert!((tail.output(0) as f64 - expected_tail(&taps, block, &all, index)).abs() < 2e-5);
            tail.advance(&history);
        }

        // Taken away, nothing is worked on, and the buffers are where they were
        let buffers = tail.buffers();
        tail.unload(0);
        tail.unload(1);
        assert!(!tail.is_running());
        tail.load(0, &spectra[..len]);
        tail.reset();
        assert!(tail.is_playing(0));
        assert_eq!(tail.buffers(), buffers);
    }

    #[test]
    fn test_work_is_spread_over_the_block() {
        // Every sample pays the same, and no step is done before it is paid for: the
        // steps done by any sample of a block are within one transform of its share
        let block = 64;
        let taps = response(1920, 3);
        let input = noise(4000, 5);
        let mut tail = Tail::new();
        tail.set_block(block);
        let mut spectra = vec![0.0; MAX_SPECTRA_LEN];
        let len = partition(&taps, block, &mut spectra);
        tail.load(0, &spectra[..len]);
        let mut history = vec![0.0; 2 * block];
        for (index, &sample) in input.iter().enumerate() {
            history.push(sample);
            let before = tail.step;
            tail.advance(&history);
            if index >= block && tail.position != 0 {
                let done: usize = (0..tail.step).map(|step| tail.step_cost(step)).sum();
                let share = tail.cost * tail.position / block;
                assert!(done <= share && share - done < TRANSFORM_COST, "Sample {index}: {done} of {share}");
                assert!(tail.step - before <= 2);
            }
        }
    }
}
