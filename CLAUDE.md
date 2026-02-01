# Guitar Pitch Shifter Plugin

Real-time guitar pitch-shifting VST3/CLAP plugin built with Rust and nih-plug framework.

## Purpose

Shift guitar audio by semitones (-12 to +12) with minimal latency for live playing. Uses variable-rate playback with crossfade resync algorithm.

## Build Commands

```bash
# Build release VST3 and CLAP bundles
cargo xtask bundle guitar_pitch_shifter_plugin --release

# Quick compile check
cargo check

# Run tests (always run before committing!)
cargo test
```

Output location: `target/bundled/GuitarPitchShifterPlugin_vX.vst3`

## Install

Copy VST3 to system folder (requires admin):
```
xcopy /E /I /Y "target\bundled\GuitarPitchShifterPlugin_vX.vst3" "C:\Program Files\Common Files\VST3\GuitarPitchShifterPlugin_vX.vst3"
```

## Architecture

- `src/lib.rs` - Plugin entry point, parameters, nih-plug integration
- `src/pitch_shifter.rs` - DSP: circular buffer, variable-rate playback, crossfade resync

### Pitch Shifting Algorithm

1. Write input samples to circular buffer
2. Read at variable rate (2^(semitones/12)) with linear interpolation
3. When read/write distance exceeds bounds, crossfade to resync

## Parameters

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Semitones | -12 to +12 | 0 | Pitch shift amount |
| Latency | 2-50 ms | 5 ms | Lower = responsive, more artifacts |
| Smoothness | 0.5-10 ms | 2 ms | Crossfade length |

## Versioning

When iterating on the plugin:
1. Increment version in `bundler.toml` (name = "GuitarPitchShifterPlugin_vX")
2. Update `NAME` constant in `src/lib.rs`
3. Update `VST3_CLASS_ID` in `src/lib.rs` (change last digit)

This allows testing multiple versions side-by-side in DAW.

## Testing

**Always run `cargo test` before committing changes.**

Tests are in `src/pitch_shifter.rs` and cover:
- Passthrough behavior at 0 semitones
- Pitch shifting frequency accuracy
- Parameter changes
- Buffer/state management
- Stability under rapid parameter changes

## Code Conventions

- Always run tests before committing
- Optimize for low latency over audio quality
- Defer buffer resizes to avoid audio glitches
- Keep DSP code in pitch_shifter.rs separate from plugin boilerplate
