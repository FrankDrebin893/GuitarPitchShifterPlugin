use nih_plug::prelude::Enum;

use crate::voices::cymbal::CymbalPreset;
use crate::voices::kick::KickPreset;
use crate::voices::snare::SnarePreset;
use crate::voices::tom::TomPreset;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kit {
    Rock,
    Jazz,
    Metal,
}

impl Kit {
    pub const ALL: [Kit; 3] = [Kit::Rock, Kit::Jazz, Kit::Metal];
}

pub struct CymbalSet {
    // The three hi-hat sounds share one cymbal, so they must have the same modes and range
    pub hat_closed: CymbalPreset,
    pub hat_open: CymbalPreset,
    pub hat_pedal: CymbalPreset,
    pub crash1: CymbalPreset,
    pub crash2: CymbalPreset,
    pub ride: CymbalPreset,
    pub bell: CymbalPreset,
    pub china: CymbalPreset,
    pub splash: CymbalPreset,
}

/// Everything that makes one kit sound different from another. All decay times are time
/// constants: the sound is 60 dB down after about seven of them.
pub struct KitPreset {
    pub kick: KickPreset,
    pub snare: SnarePreset,
    pub toms: TomPreset,
    pub cymbals: CymbalSet,
    /// Reverb time of the room (to -60 dB)
    pub room_decay_s: f32,
    /// Room level relative to the other kits at the same Room setting
    pub room_level: f32,
}

impl KitPreset {
    pub fn new(kit: Kit) -> Self {
        match kit {
            Kit::Rock => rock(),
            Kit::Jazz => jazz(),
            Kit::Metal => metal(),
        }
    }
}

/// Big and open: deep kick, fat snare, long toms, bright crashes
fn rock() -> KitPreset {
    KitPreset {
        kick: KickPreset {
            freq_hz: 50.0,
            sweep_semitones: 22.0,
            sweep_ms: 14.0,
            decay_ms: 95.0,
            shell_level: 0.18,
            click_level: 0.45,
            click_freq_hz: 3000.0,
            click_ms: 4.0,
            drive: 1.8,
        },
        snare: SnarePreset {
            freq_hz: 185.0,
            bend_semitones: 1.5,
            body_decay_ms: 75.0,
            body_level: 0.8,
            ring: 0.55,
            wire_level: 0.75,
            wire_decay_ms: 75.0,
            wire_hz: 3200.0,
            wire_lp_hz: 9000.0,
            attack_level: 0.5,
        },
        toms: TomPreset {
            freqs_hz: [72.0, 84.0, 100.0, 120.0, 145.0, 172.0],
            bend_semitones: 3.0,
            decay_ms: 220.0,
            ring: 0.5,
            attack_level: 0.5,
        },
        cymbals: cymbals(&CymbalCharacter {
            brightness: 1.0,
            sustain: 1.0,
            wash: 1.0,
            attack: 1.0,
        }),
        room_decay_s: 0.6,
        room_level: 1.0,
    }
}

/// Small and resonant: high open kick with a felt beater, ringing snare, singing toms,
/// dark washy cymbals, more room
fn jazz() -> KitPreset {
    KitPreset {
        kick: KickPreset {
            freq_hz: 76.0,
            sweep_semitones: 8.0,
            sweep_ms: 25.0,
            decay_ms: 170.0,
            shell_level: 0.35,
            click_level: 0.1,
            click_freq_hz: 1100.0,
            click_ms: 7.0,
            drive: 1.0,
        },
        snare: SnarePreset {
            freq_hz: 225.0,
            bend_semitones: 1.0,
            body_decay_ms: 110.0,
            body_level: 0.75,
            ring: 1.0,
            wire_level: 0.4,
            wire_decay_ms: 95.0,
            wire_hz: 3800.0,
            wire_lp_hz: 8000.0,
            attack_level: 0.35,
        },
        toms: TomPreset {
            freqs_hz: [96.0, 112.0, 132.0, 158.0, 188.0, 220.0],
            bend_semitones: 1.5,
            decay_ms: 280.0,
            ring: 0.7,
            attack_level: 0.3,
        },
        cymbals: cymbals(&CymbalCharacter {
            brightness: 0.6,
            sustain: 1.3,
            wash: 1.2,
            attack: 0.8,
        }),
        room_decay_s: 0.8,
        room_level: 1.2,
    }
}

/// Tight and cutting: short clicky kick, cracking snare, punchy toms, bright cymbals, dry
fn metal() -> KitPreset {
    KitPreset {
        kick: KickPreset {
            freq_hz: 56.0,
            sweep_semitones: 26.0,
            sweep_ms: 9.0,
            decay_ms: 60.0,
            shell_level: 0.08,
            click_level: 0.9,
            click_freq_hz: 4200.0,
            click_ms: 2.5,
            drive: 2.4,
        },
        snare: SnarePreset {
            freq_hz: 240.0,
            bend_semitones: 2.0,
            body_decay_ms: 50.0,
            body_level: 0.85,
            ring: 0.45,
            wire_level: 0.6,
            wire_decay_ms: 55.0,
            wire_hz: 4500.0,
            wire_lp_hz: 11000.0,
            attack_level: 0.8,
        },
        toms: TomPreset {
            freqs_hz: [64.0, 76.0, 90.0, 108.0, 130.0, 154.0],
            bend_semitones: 4.0,
            decay_ms: 120.0,
            ring: 0.4,
            attack_level: 0.6,
        },
        cymbals: cymbals(&CymbalCharacter {
            brightness: 1.5,
            sustain: 0.85,
            wash: 0.9,
            attack: 1.3,
        }),
        room_decay_s: 0.35,
        room_level: 0.7,
    }
}

/// How a kit's cymbals differ from the reference set below
struct CymbalCharacter {
    brightness: f32,
    sustain: f32,
    wash: f32,
    attack: f32,
}

fn cymbals(character: &CymbalCharacter) -> CymbalSet {
    let shape = |preset: CymbalPreset| CymbalPreset {
        tilt: preset.tilt * character.brightness,
        wash_hp_hz: preset.wash_hp_hz * character.brightness.sqrt(),
        wash_lp_hz: preset.wash_lp_hz * character.brightness,
        decay_low_ms: preset.decay_low_ms * character.sustain,
        decay_high_ms: preset.decay_high_ms * character.sustain,
        wash_decay_ms: preset.wash_decay_ms * character.sustain,
        wash_level: preset.wash_level * character.wash,
        attack_level: preset.attack_level * character.attack,
        ..preset
    };
    let hat = |decay_low_ms, decay_high_ms, mode_level, wash_level, wash_decay_ms, attack_level| CymbalPreset {
        modes: 40,
        low_hz: 900.0,
        high_hz: 13500.0,
        tilt: 2.0,
        decay_low_ms,
        decay_high_ms,
        mode_level,
        wash_level,
        wash_hp_hz: 7000.0,
        wash_lp_hz: 13000.0,
        wash_decay_ms,
        attack_level,
        attack_ms: 5.0,
    };

    CymbalSet {
        hat_closed: shape(hat(45.0, 30.0, 0.5, 0.5, 35.0, 0.6)),
        hat_open: shape(hat(500.0, 300.0, 0.5, 0.35, 300.0, 0.6)),
        hat_pedal: shape(hat(30.0, 20.0, 0.3, 0.2, 25.0, 0.2)),
        crash1: shape(CymbalPreset {
            modes: 56,
            low_hz: 250.0,
            high_hz: 12000.0,
            tilt: 1.2,
            decay_low_ms: 900.0,
            decay_high_ms: 350.0,
            mode_level: 0.6,
            wash_level: 0.5,
            wash_hp_hz: 5000.0,
            wash_lp_hz: 11000.0,
            wash_decay_ms: 450.0,
            attack_level: 0.8,
            attack_ms: 15.0,
        }),
        crash2: shape(CymbalPreset {
            modes: 56,
            low_hz: 310.0,
            high_hz: 13000.0,
            tilt: 1.3,
            decay_low_ms: 800.0,
            decay_high_ms: 300.0,
            mode_level: 0.6,
            wash_level: 0.5,
            wash_hp_hz: 5500.0,
            wash_lp_hz: 11500.0,
            wash_decay_ms: 400.0,
            attack_level: 0.8,
            attack_ms: 15.0,
        }),
        ride: shape(CymbalPreset {
            modes: 56,
            low_hz: 300.0,
            high_hz: 10000.0,
            tilt: 1.0,
            decay_low_ms: 1600.0,
            decay_high_ms: 500.0,
            mode_level: 0.5,
            wash_level: 0.12,
            wash_hp_hz: 6000.0,
            wash_lp_hz: 12000.0,
            wash_decay_ms: 300.0,
            attack_level: 0.5,
            attack_ms: 6.0,
        }),
        bell: shape(CymbalPreset {
            modes: 14,
            low_hz: 550.0,
            high_hz: 7000.0,
            tilt: 0.8,
            decay_low_ms: 1400.0,
            decay_high_ms: 600.0,
            mode_level: 0.8,
            wash_level: 0.05,
            wash_hp_hz: 6000.0,
            wash_lp_hz: 12000.0,
            wash_decay_ms: 200.0,
            attack_level: 0.3,
            attack_ms: 4.0,
        }),
        china: shape(CymbalPreset {
            modes: 48,
            low_hz: 350.0,
            high_hz: 11000.0,
            tilt: 1.2,
            decay_low_ms: 500.0,
            decay_high_ms: 200.0,
            mode_level: 0.6,
            wash_level: 0.7,
            wash_hp_hz: 3500.0,
            wash_lp_hz: 10000.0,
            wash_decay_ms: 300.0,
            attack_level: 0.9,
            attack_ms: 20.0,
        }),
        splash: shape(CymbalPreset {
            modes: 36,
            low_hz: 500.0,
            high_hz: 14000.0,
            tilt: 1.4,
            decay_low_ms: 300.0,
            decay_high_ms: 150.0,
            mode_level: 0.6,
            wash_level: 0.5,
            wash_hp_hz: 6000.0,
            wash_lp_hz: 12000.0,
            wash_decay_ms: 180.0,
            attack_level: 0.7,
            attack_ms: 8.0,
        }),
    }
}
