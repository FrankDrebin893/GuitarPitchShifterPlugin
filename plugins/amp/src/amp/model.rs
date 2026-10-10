use nih_plug::prelude::Enum;

/// The most gain stages a preamp can have
pub const MAX_STAGES: usize = 4;

// Frozen: the variant ids are what DAW projects store
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Amp {
    #[id = "klar"]
    #[name = "Klar"]
    Klar,
    #[id = "brol"]
    #[name = "Brøl"]
    Brol,
    #[id = "torden"]
    #[name = "Torden"]
    Torden,
}

impl Amp {
    pub const ALL: [Amp; 3] = [Amp::Klar, Amp::Brol, Amp::Torden];

    pub fn model(self) -> &'static AmpModel {
        match self {
            // Stub: Klar and Torden get their own models in milestone 2
            Amp::Klar | Amp::Brol | Amp::Torden => &BROL,
        }
    }

    /// Position in `ALL`
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One gain stage of the preamp
pub struct StageModel {
    /// High-pass in front of the stage: how much low end reaches the clipping
    pub coupling_hz: f32,
    /// Gain in dB with the Gain dial at 0 and at 10
    pub gain_db: [f32; 2],
    /// Where the stage clips, upwards and downwards. The difference makes even harmonics
    pub headroom: [f32; 2],
    /// Resting operating point, as an offset into the curve
    pub bias: f32,
    /// How far hard playing pushes the operating point down. Soft playing leaves it alone
    pub bias_shift: f32,
    /// Low-pass after the stage
    pub lowpass_hz: f32,
}

/// Bass, Mid and Treble. They share one network, so each dial moves the others' ranges
pub struct ToneModel {
    pub bass_hz: f32,
    /// Shelf gain in dB from dial centre to either end
    pub bass_db: f32,
    pub mid_hz: f32,
    pub mid_q: f32,
    pub mid_db: f32,
    /// Mid level with every dial at centre: above zero is mid-forward, below is scooped
    pub mid_centre_db: f32,
    /// How much deeper the mids sit with Bass and Treble both at 10 (and shallower at 0)
    pub scoop_db: f32,
    /// Octaves the mid frequency moves, up with Bass and down with Treble
    pub mid_shift_octaves: f32,
    pub treble_hz: f32,
    pub treble_db: f32,
    /// Overall level change in dB that comes with the Mid dial, centre to end
    pub mid_level_db: f32,
}

pub struct PowerModel {
    /// Drive into the power stage in dB with Master at 0, 5 and 10
    pub drive_db: [f32; 3],
    /// Level after the power stage in dB with Master at 0, 5 and 10
    pub volume_db: [f32; 3],
    /// Share of the headroom lost at full sustained output
    pub sag: f32,
    pub sag_attack_ms: f32,
    pub sag_release_ms: f32,
    /// Low peak from the speaker's resonance pushing back on the power stage
    pub resonance_hz: f32,
    pub resonance_db: f32,
    pub presence_hz: f32,
    /// High shelf gain in dB with Presence at 10
    pub presence_db: f32,
}

/// The cabinet: speaker and box, as heard by a microphone close to the cone
pub struct CabModel {
    /// Speaker resonance: a high-pass with a bump at its corner
    pub resonance_hz: f32,
    pub resonance_q: f32,
    /// Low-mid body: frequency, Q, dB
    pub body: [f32; 3],
    /// Dip between body and bite: frequency, Q, dB
    pub dip: [f32; 3],
    /// Upper-mid bite: frequency, Q, dB
    pub bite: [f32; 3],
    /// The cone stops radiating above this, 24 dB per octave and then some
    pub rolloff_hz: f32,
    /// Cone breakup: narrow peaks and notches between these frequencies
    pub breakup_hz: [f32; 2],
    pub breakup_count: usize,
    /// Largest peak or notch in dB
    pub breakup_db: f32,
    /// Fixes where the peaks and notches fall
    pub breakup_seed: u32,
}

/// Everything that makes one amp differ from another
pub struct AmpModel {
    // Only the report prints it until the editor names the amps
    #[allow(dead_code)]
    pub name: &'static str,
    /// High-pass at the input: keeps the low end tight when the gain is up
    pub tight_hz: f32,
    /// Treble that bypasses the gain dial, strongest at low gain: corner and dB at Gain 0
    pub bright_hz: f32,
    pub bright_db: f32,
    pub stage_count: usize,
    pub stages: [StageModel; MAX_STAGES],
    /// Level after the preamp in dB across the Gain dial (0, 2.5, 5, 7.5, 10), set so the
    /// loudness stays about the same while the distortion changes
    pub level_db: [f32; 5],
    pub tone: ToneModel,
    pub power: PowerModel,
    pub cab: CabModel,
}

const UNUSED_STAGE: StageModel = StageModel {
    coupling_hz: 20.0,
    gain_db: [0.0, 0.0],
    headroom: [1.0, 1.0],
    bias: 0.0,
    bias_shift: 0.0,
    lowpass_hz: 20000.0,
};

/// Crunch: a mid-forward stack. Little low end goes into the clipping, so chords stay
/// defined; the first stage has headroom to spare, so picking softly cleans it up
static BROL: AmpModel = AmpModel {
    name: "Brøl",
    tight_hz: 110.0,
    bright_hz: 1800.0,
    bright_db: 6.0,
    stage_count: 3,
    stages: [
        StageModel {
            coupling_hz: 30.0,
            gain_db: [2.0, 14.0],
            headroom: [2.0, 2.8],
            bias: 0.0,
            bias_shift: 0.5,
            lowpass_hz: 12000.0,
        },
        StageModel {
            coupling_hz: 180.0,
            gain_db: [-1.0, 22.0],
            headroom: [1.0, 1.5],
            bias: 0.1,
            bias_shift: 0.4,
            lowpass_hz: 9000.0,
        },
        StageModel {
            coupling_hz: 70.0,
            gain_db: [-1.0, 14.0],
            headroom: [1.2, 0.9],
            bias: -0.1,
            bias_shift: 0.3,
            lowpass_hz: 7500.0,
        },
        UNUSED_STAGE,
    ],
    level_db: [13.0, 6.5, 0.0, -1.5, -2.5],
    tone: ToneModel {
        bass_hz: 140.0,
        bass_db: 10.0,
        mid_hz: 650.0,
        mid_q: 0.7,
        mid_db: 8.0,
        mid_centre_db: 2.5,
        scoop_db: 4.0,
        mid_shift_octaves: 0.4,
        treble_hz: 2800.0,
        treble_db: 10.0,
        mid_level_db: 2.0,
    },
    power: PowerModel {
        drive_db: [-26.0, -6.0, 14.0],
        volume_db: [-8.0, -7.0, -7.5],
        sag: 0.15,
        sag_attack_ms: 15.0,
        sag_release_ms: 100.0,
        resonance_hz: 100.0,
        resonance_db: 4.0,
        presence_hz: 4000.0,
        presence_db: 8.0,
    },
    cab: CabModel {
        resonance_hz: 95.0,
        resonance_q: 1.3,
        body: [240.0, 0.8, 2.0],
        dip: [450.0, 1.0, -4.0],
        bite: [3000.0, 1.0, 7.0],
        rolloff_hz: 5200.0,
        breakup_hz: [1000.0, 6000.0],
        breakup_count: 28,
        breakup_db: 3.5,
        breakup_seed: 50,
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_amp_has_a_usable_model() {
        for (index, amp) in Amp::ALL.iter().enumerate() {
            let model = amp.model();
            assert_eq!(amp.index(), index);
            assert!(!model.name.is_empty());
            assert!((1..=MAX_STAGES).contains(&model.stage_count));
            for stage in &model.stages[..model.stage_count] {
                assert!(stage.headroom[0] > 0.0 && stage.headroom[1] > 0.0);
                assert!(stage.gain_db[1] >= stage.gain_db[0]);
                assert!(stage.bias_shift >= 0.0);
            }
            assert!(model.power.sag >= 0.0 && model.power.sag < 0.5);
            assert!(model.cab.breakup_hz[0] < model.cab.breakup_hz[1]);
        }
    }
}
