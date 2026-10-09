# Audio Plugin Suite

A suite of VST3/CLAP plugins built with Rust and the nih-plug framework. The suite has no brand name yet; the vendor string lives in one place (`crates/suite_common/src/lib.rs`).

## Layout

- `plugins/pitch_shifter/` - Real-time guitar pitch shifter (effect)
- `plugins/drums/` - Synthesized drum instrument (skeleton: kick only)
- `crates/suite_common/` - Shared code: `VENDOR` constant, `ui` module (dial widget, theme colours)
- `xtask/` - nih-plug bundler
- `bundler.toml` - Bundle name per plugin package

Each plugin keeps DSP in its own module(s), separate from `lib.rs` (plugin entry point, parameters, nih-plug integration) and `editor.rs` (egui GUI).

## Build Commands

```bash
# Build release VST3 and CLAP bundles (one plugin at a time)
cargo xtask bundle pitch_shifter --release
cargo xtask bundle drums --release

# Quick compile check
cargo check --workspace

# Run tests (always run before committing!)
cargo test --workspace
```

Output location: `target/bundled/<BundleName>.vst3`

## Install

Copy VST3 to system folder (requires admin):
```
xcopy /E /I /Y "target\bundled\GuitarPitchShifterPlugin_vX.vst3" "C:\Program Files\Common Files\VST3\GuitarPitchShifterPlugin_vX.vst3"
```

## Plugins

### Pitch Shifter (`plugins/pitch_shifter`)

Shift guitar audio by semitones (-12 to +12) with minimal latency for live playing. Uses variable-rate playback with crossfade resync algorithm.

- `src/lib.rs` - Plugin entry point, parameters, nih-plug integration
- `src/pitch_shifter.rs` - DSP: circular buffer, variable-rate playback, crossfade resync

Algorithm:

1. Write input samples to circular buffer
2. Read at variable rate (2^(semitones/12)) with linear interpolation
3. When read/write distance exceeds bounds, crossfade to resync

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Semitones | -12 to +12 | 0 | Pitch shift amount |
| Latency | 2-50 ms | 5 ms | Lower = responsive, more artifacts |
| Smoothness | 0.5-10 ms | 2 ms | Crossfade length |

Its VST3 class ID, CLAP ID and bundle name predate the suite and are kept so existing DAW projects keep loading.

### Drum Synth (`plugins/drums`)

Fully synthesized drums: no sample files, nothing loaded from disk. MIDI in, stereo out, General MIDI drum map.

- `src/lib.rs` - Plugin entry point, parameters, MIDI event handling
- `src/kick.rs` - DSP: sine oscillator with exponential pitch sweep and amp decay

| MIDI note | Voice |
|-----------|-------|
| 36 (C1) | Kick |

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Gain | -30 to +6 dB | -6 dB | Output level |

## Adding a Plugin

1. Create `plugins/<name>/` with a `Cargo.toml` that uses the workspace dependencies (copy `plugins/drums/Cargo.toml`)
2. Add a `[<name>]` entry to `bundler.toml`
3. Use `suite_common::VENDOR` and the `suite_common::ui` widgets and colours
4. Pick a unique `VST3_CLASS_ID` (16 bytes) and `CLAP_ID`; these are permanent once the plugin is used in a DAW project

## Versioning

When iterating on a plugin:
1. Increment version in `bundler.toml` (e.g. name = "GuitarPitchShifterPlugin_vX")
2. Update `NAME` constant in `plugins/<name>/src/lib.rs`
3. Update `VST3_CLASS_ID` in `plugins/<name>/src/lib.rs` (change last digit)

This allows testing multiple versions side-by-side in DAW.

## Testing

**Always run `cargo test --workspace` before committing changes.**

Tests live next to the DSP code (`pitch_shifter.rs`, `kick.rs`) and cover:
- Passthrough behavior at 0 semitones
- Pitch shifting frequency accuracy
- Parameter changes
- Buffer/state management
- Stability under rapid parameter changes
- Drum voice triggering, decay, velocity and output bounds

## Code Conventions

- Always run tests before committing
- Optimize for low latency over audio quality
- Defer buffer resizes to avoid audio glitches
- Keep DSP code separate from plugin boilerplate
- No allocation in the audio path
- nih-plug is pinned to one revision in the root `Cargo.toml` (`[workspace.dependencies]`); bump it there for all plugins at once
