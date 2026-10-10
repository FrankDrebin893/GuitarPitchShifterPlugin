/// What the knobs say, read once per block. The chain smooths the values itself
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmpSettings {
    pub bypass: bool,
    /// The amp dials, 0.0 to 1.0
    pub gain: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub presence: f32,
    pub master: f32,
    /// Linear gain
    pub out_level: f32,
}

impl Default for AmpSettings {
    fn default() -> Self {
        Self {
            bypass: false,
            gain: 0.5,
            bass: 0.5,
            mid: 0.5,
            treble: 0.5,
            presence: 0.5,
            master: 0.5,
            out_level: 1.0,
        }
    }
}

/// The whole signal chain. Mono through the amp and cabinet
pub struct AmpChain {
    sample_rate: f32,
}

impl AmpChain {
    pub fn new() -> Self {
        Self { sample_rate: 44100.0 }
    }

    /// Allocates and designs everything that depends on the sample rate. Not for the audio thread
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.reset();
    }

    pub fn reset(&mut self) {}

    /// Processes a block in place. With two channels the input is their average and the
    /// output goes to both
    pub fn process(&mut self, settings: &AmpSettings, left: &mut [f32], right: Option<&mut [f32]>) {
        // Stub: the stages come with milestone 1
        let gain = if settings.bypass { 1.0 } else { settings.out_level };
        match right {
            Some(right) => {
                for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                    let mono = 0.5 * (*l + *r) * gain;
                    *l = mono;
                    *r = mono;
                }
            }
            None => {
                for sample in left.iter_mut() {
                    *sample *= gain;
                }
            }
        }
    }
}
