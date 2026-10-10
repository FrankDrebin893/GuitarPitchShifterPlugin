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
            Amp::Klar => &KLAR,
            Amp::Brol => &BROL,
            Amp::Torden => &TORDEN,
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
    /// Gain in dB with the Gain dial at 0, 5 and 10
    pub gain_db: [f32; 3],
    /// Where the stage clips, upwards and downwards. The difference makes even harmonics
    pub headroom: [f32; 2],
    /// Resting operating point, as an offset into the curve
    pub bias: f32,
    /// How far hard playing pushes the operating point down. Soft playing leaves it alone
    pub bias_shift: f32,
    /// Low-pass after the stage
    pub lowpass_hz: f32,
    /// Antialiases the clipping to the second order instead of the first: for the stages
    /// that clip hardest in an amp with a lot of gain. Costs about four times as much
    pub second_order: bool,
    /// Clips at twice the rate while the drive pedal is on (`FineClipper`): for the stage
    /// that is handed square waves once the pedal pushes the stage before it into clipping.
    /// Costs twice the second order, and only while the pedal is on
    pub finer_when_driven: bool,
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
    /// The cone stops radiating above this, 24 dB per octave
    pub rolloff_hz: f32,
    /// A last, gentler slope starts this many times higher: close to 1.0 makes the
    /// roll-off steeper
    pub air_ratio: f32,
    /// Cone breakup: narrow peaks and notches between these frequencies
    pub breakup_hz: [f32; 2],
    pub breakup_count: usize,
    /// Largest peak or notch in dB
    pub breakup_db: f32,
    /// Fixes where the peaks and notches fall
    pub breakup_seed: u32,
    /// Length in milliseconds of the impulse response that holds all of this but the
    /// resonance: long enough for the narrowest of the peaks to ring out
    pub ir_ms: f32,
}

/// Everything that makes one amp differ from another
pub struct AmpModel {
    // Only the report prints it until the editor names the amps
    #[allow(dead_code)]
    pub name: &'static str,
    /// The model number printed on the amp
    #[allow(dead_code)]
    pub model_number: &'static str,
    /// High-pass at the input: keeps the low end tight when the gain is up
    pub tight_hz: f32,
    /// Bell in front of the first stage, so the clipping works on this band most:
    /// frequency, Q, dB
    pub focus: [f32; 3],
    /// Treble that bypasses the gain dial, strongest at low gain: corner and dB at Gain 0
    pub bright_hz: f32,
    pub bright_db: f32,
    pub stage_count: usize,
    pub stages: [StageModel; MAX_STAGES],
    /// Low-pass after the last stage, 12 dB per octave, that takes the fizz off a lot of
    /// gain. `None` leaves the top to the stages' own low-passes
    pub fizz_hz: Option<f32>,
    /// High-pass after the last stage, 12 dB per octave: takes away the lows that the
    /// clipping itself makes out of a chord. `None` leaves them in
    pub lowcut_hz: Option<f32>,
    /// Level after the preamp in dB across the Gain dial (0, 2.5, 5, 7.5, 10), set so the
    /// loudness stays about the same while the distortion changes
    pub level_db: [f32; 5],
    /// Level after the power stage in dB across the Gain dial (0, 2.5, 5, 7.5, 10). What is
    /// made up here instead of in `level_db` does not push the power stage, so the low end
    /// of the dial stays clean on hard pick attacks
    pub makeup_db: [f32; 5],
    pub tone: ToneModel,
    pub power: PowerModel,
    pub cab: CabModel,
}

const UNUSED_STAGE: StageModel = StageModel {
    coupling_hz: 20.0,
    gain_db: [0.0, 0.0, 0.0],
    headroom: [1.0, 1.0],
    bias: 0.0,
    bias_shift: 0.0,
    lowpass_hz: 20000.0,
    second_order: false,
    finer_when_driven: false,
};

/// Clean: glassy, with headroom to spare. Two stages with high ceilings that only bend at
/// the top of the Gain dial, a bright lift that is strongest at low gain, scooped mids
static KLAR: AmpModel = AmpModel {
    name: "Klar",
    model_number: "KL-30",
    tight_hz: 60.0,
    focus: [800.0, 0.7, 0.0],
    bright_hz: 1500.0,
    bright_db: 7.0,
    stage_count: 2,
    stages: [
        StageModel {
            coupling_hz: 20.0,
            gain_db: [-4.0, 3.5, 11.0],
            headroom: [4.0, 5.0],
            bias: 0.0,
            bias_shift: 0.3,
            lowpass_hz: 16000.0,
            second_order: false,
            finer_when_driven: false,
        },
        StageModel {
            coupling_hz: 40.0,
            gain_db: [-6.0, 5.5, 17.0],
            headroom: [3.0, 3.8],
            bias: 0.05,
            bias_shift: 0.3,
            lowpass_hz: 12000.0,
            second_order: false,
            finer_when_driven: false,
        },
        UNUSED_STAGE,
        UNUSED_STAGE,
    ],
    fizz_hz: None,
    lowcut_hz: None,
    level_db: [24.0, 17.0, 8.0, 0.0, -5.5],
    // Nothing squeezes the peaks of a clean amp, so it is turned down where it is cleanest:
    // chords then stay 1.2 to 1.4 dB under where the output starts to round them off
    makeup_db: [-1.3, -1.4, -0.8, -0.4, 0.0],
    tone: ToneModel {
        bass_hz: 120.0,
        bass_db: 10.0,
        mid_hz: 500.0,
        mid_q: 0.7,
        mid_db: 8.0,
        mid_centre_db: -3.5,
        scoop_db: 4.0,
        mid_shift_octaves: 0.4,
        treble_hz: 3200.0,
        treble_db: 10.0,
        mid_level_db: 2.0,
    },
    power: PowerModel {
        drive_db: [-30.0, -10.0, 10.0],
        volume_db: [-2.5, -2.5, -5.0],
        sag: 0.12,
        sag_attack_ms: 20.0,
        sag_release_ms: 100.0,
        resonance_hz: 85.0,
        resonance_db: 2.0,
        presence_hz: 5000.0,
        presence_db: 8.0,
    },
    cab: CabModel {
        resonance_hz: 78.0,
        resonance_q: 1.2,
        body: [130.0, 0.8, 1.5],
        dip: [400.0, 0.9, -2.5],
        bite: [3200.0, 0.6, 5.0],
        rolloff_hz: 6600.0,
        air_ratio: 1.5,
        breakup_hz: [1200.0, 7000.0],
        breakup_count: 24,
        breakup_db: 1.8,
        breakup_seed: 30,
        ir_ms: 5.0,
    },
};

/// Crunch: a mid-forward stack. Little low end goes into the clipping, so chords stay
/// defined; the first stage has headroom to spare, so picking softly cleans it up. At the
/// bottom of the Gain dial every stage is turned down and the level is made up after the
/// power stage, so that hard pick attacks stay clean there
static BROL: AmpModel = AmpModel {
    name: "Brøl",
    model_number: "BR-50",
    tight_hz: 110.0,
    focus: [800.0, 0.7, 0.0],
    bright_hz: 1800.0,
    bright_db: 6.0,
    stage_count: 3,
    stages: [
        StageModel {
            coupling_hz: 30.0,
            gain_db: [0.0, 8.0, 14.0],
            headroom: [2.0, 2.8],
            bias: 0.0,
            bias_shift: 0.5,
            lowpass_hz: 12000.0,
            second_order: false,
            finer_when_driven: false,
        },
        StageModel {
            coupling_hz: 180.0,
            gain_db: [-5.0, 10.5, 22.0],
            headroom: [1.0, 1.5],
            bias: 0.1,
            bias_shift: 0.4,
            lowpass_hz: 9000.0,
            second_order: false,
            finer_when_driven: false,
        },
        StageModel {
            coupling_hz: 70.0,
            gain_db: [-4.0, 6.5, 14.0],
            headroom: [1.2, 0.9],
            bias: -0.1,
            bias_shift: 0.3,
            lowpass_hz: 7500.0,
            second_order: false,
            finer_when_driven: false,
        },
        UNUSED_STAGE,
    ],
    fizz_hz: None,
    lowcut_hz: None,
    level_db: [6.0, 3.0, 0.0, -1.5, -2.5],
    makeup_db: [15.0, 6.5, 0.0, 0.0, 0.0],
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
        air_ratio: 1.7,
        breakup_hz: [1000.0, 6000.0],
        breakup_count: 28,
        breakup_db: 3.5,
        breakup_seed: 50,
        ir_ms: 10.0,
    },
};

/// High gain: tight and modern. The low end is cut before the clipping and put back after
/// it, four stages clip a signal with its upper mids pushed forward, and the fizz is
/// filtered off after every stage. A stiff power stage with a strong resonance and presence.
/// The second and third stages do most of the clipping, and are antialiased to match. With
/// the drive pedal's Level up the first stage clips as well and hands the second one square
/// waves: that one then clips at twice the rate
static TORDEN: AmpModel = AmpModel {
    name: "Torden",
    model_number: "TD-100",
    tight_hz: 160.0,
    focus: [1000.0, 0.5, 4.0],
    bright_hz: 2000.0,
    bright_db: 3.0,
    stage_count: 4,
    stages: [
        StageModel {
            coupling_hz: 30.0,
            gain_db: [10.0, 14.0, 18.0],
            headroom: [2.5, 3.0],
            bias: 0.0,
            bias_shift: 0.2,
            lowpass_hz: 12000.0,
            second_order: false,
            finer_when_driven: false,
        },
        StageModel {
            coupling_hz: 250.0,
            gain_db: [8.0, 14.0, 20.0],
            headroom: [1.0, 1.4],
            bias: 0.1,
            bias_shift: 0.2,
            lowpass_hz: 8000.0,
            second_order: true,
            finer_when_driven: true,
        },
        StageModel {
            coupling_hz: 150.0,
            gain_db: [6.0, 12.0, 18.0],
            headroom: [1.2, 0.9],
            bias: -0.1,
            bias_shift: 0.15,
            lowpass_hz: 7000.0,
            second_order: true,
            finer_when_driven: false,
        },
        StageModel {
            coupling_hz: 140.0,
            gain_db: [6.0, 10.0, 14.0],
            headroom: [1.0, 1.3],
            bias: 0.05,
            bias_shift: 0.1,
            lowpass_hz: 6000.0,
            second_order: false,
            finer_when_driven: false,
        },
    ],
    fizz_hz: Some(7500.0),
    lowcut_hz: Some(120.0),
    level_db: [2.0, 0.5, 0.0, 0.0, 0.0],
    makeup_db: [0.0; 5],
    tone: ToneModel {
        bass_hz: 110.0,
        bass_db: 10.0,
        mid_hz: 700.0,
        mid_q: 0.6,
        mid_db: 12.0,
        mid_centre_db: -2.0,
        scoop_db: 3.0,
        mid_shift_octaves: 0.3,
        treble_hz: 3000.0,
        treble_db: 10.0,
        mid_level_db: 2.0,
    },
    power: PowerModel {
        drive_db: [-26.0, -8.0, 10.0],
        volume_db: [-8.0, -7.0, -7.5],
        sag: 0.04,
        sag_attack_ms: 10.0,
        sag_release_ms: 60.0,
        resonance_hz: 125.0,
        resonance_db: 7.0,
        presence_hz: 3800.0,
        presence_db: 10.0,
    },
    cab: CabModel {
        resonance_hz: 125.0,
        resonance_q: 1.15,
        body: [220.0, 0.7, 4.0],
        dip: [550.0, 1.2, -4.0],
        bite: [2800.0, 0.9, 6.5],
        rolloff_hz: 4700.0,
        air_ratio: 1.2,
        breakup_hz: [1000.0, 5500.0],
        breakup_count: 26,
        breakup_db: 3.0,
        breakup_seed: 100,
        ir_ms: 6.0,
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
            assert!(!model.name.is_empty() && !model.model_number.is_empty());
            assert!(model.focus[0] > 0.0 && model.focus[1] > 0.0);
            assert!(model.cab.air_ratio >= 1.0);
            assert!((1..=MAX_STAGES).contains(&model.stage_count));
            for stage in &model.stages[..model.stage_count] {
                assert!(stage.headroom[0] > 0.0 && stage.headroom[1] > 0.0);
                assert!(stage.gain_db[1] >= stage.gain_db[0] && stage.gain_db[2] >= stage.gain_db[1]);
                assert!(stage.bias_shift >= 0.0);
            }
            assert!(model.power.sag >= 0.0 && model.power.sag < 0.5);
            assert!(model.cab.breakup_hz[0] < model.cab.breakup_hz[1]);
        }
    }
}
