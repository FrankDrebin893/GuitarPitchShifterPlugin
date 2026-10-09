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

Shift guitar audio by semitones (-12 to +12) with minimal latency for live playing. Uses variable-rate playback with waveform-matched splices.

- `src/lib.rs` - Plugin entry point, parameters, nih-plug integration
- `src/pitch_shifter.rs` - DSP: circular buffer, variable-rate playback, waveform-matched splices
- `src/editor.rs` - egui GUI with dial controls

Algorithm:

1. Write input samples to a fixed-size circular buffer (one per channel, plus a mono mix)
2. One read head reads at variable rate (2^(semitones/12)) with cubic interpolation
3. The head's delay drifts by (1 - rate) per sample. It only jumps (splices) when it must:
   - shifting up: when it gets too close to the write head, jump back
   - shifting down: as soon as one pitch period fits, or at the max latency, jump forward
   - 0 semitones: settle on the minimum delay, then pass the input through bit-exactly
4. `find_splice_lag` picks the jump distance by normalized correlation of the signal leading
   up to the head against candidate positions: coarse search on a decimated copy, then
   sample-accurate refinement. `pick_peak` chooses the pitch period among the peaks
5. A short crossfade bridges the jump, with gains scaled by how well the two positions match
6. While the input is below -60 dBFS the head is parked so the next attack is clean and on time

Both stereo channels share one read head.

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Semitones | -12 to +12 | 0 | Pitch shift amount |
| Max Latency | 2-50 ms | 15 ms | Delay limit = longest period that splices cleanly |
| Smoothness | 0.5-10 ms | 2 ms | Crossfade length at each splice |

Actual delay is about one pitch period of the note being played, not the Max Latency value.
Single notes need Max Latency >= one period of the lowest note (12 ms for low E). Chords have a
longer combined period (~24 ms for a power chord on low E) and get cleaner at 25-30 ms.

Its CLAP ID, VST3 class ID scheme and bundle name predate the suite and are kept as they were.

### Drum Synth (`plugins/drums`)

Fully synthesized drums: no sample files, nothing loaded from disk. MIDI in, stereo out, General MIDI drum map.

- `src/lib.rs` - Plugin entry point, parameters, MIDI event handling
- `src/kick.rs` - DSP: sine oscillator with exponential pitch sweep and amp decay
- `src/editor.rs` - egui GUI with gain dial

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
4. Pick a unique `VST3_CLASS_ID` (16 bytes) and `CLAP_ID`; DAW projects reference plugins by these

## Versioning

When iterating on a plugin:
1. Increment version in `bundler.toml` (e.g. name = "GuitarPitchShifterPlugin_vX")
2. Update `NAME` constant in `plugins/<name>/src/lib.rs`
3. Update `VST3_CLASS_ID` in `plugins/<name>/src/lib.rs` (change last digit)

This allows testing multiple versions side-by-side in DAW.

## Testing

**Always run `cargo test --workspace` before committing changes.**

Pitch shifter tests are in `plugins/pitch_shifter/src/pitch_shifter.rs` and cover:
- Bit-exact passthrough at 0 semitones
- Artifact level of shifted sines, harmonic tones and chords (`artifact_ratio_db`: energy that
  is not at the expected shifted frequencies)
- Latency following the pitch period, and staying within the limit
- Stereo alignment, level, no clicks under parameter changes
- Buffer/state management

Two ignored tests are tools for working on the sound:

```bash
# Table of artifact levels and latency per note and shift. Compare before/after a DSP change.
cargo test -p pitch_shifter --release quality_report -- --ignored --nocapture

# Before/after WAV files in target/renders for listening.
# Optional env vars: PITCH_SHIFTER_INPUT_WAV=<recording>, PITCH_SHIFTER_LATENCY_MS=<ms>
cargo test -p pitch_shifter --release render_wavs -- --ignored
```

Drum tests are in `plugins/drums/src/kick.rs` and cover voice triggering, decay, velocity scaling and output bounds.

## Code Conventions

- Always run tests before committing
- Optimize for low latency over audio quality
- Never allocate or resize buffers on the audio thread (all buffers are fixed-size)
- Measure pitch shifter DSP changes with `quality_report` before judging them by ear
- Keep DSP code separate from plugin boilerplate
- nih-plug is pinned to one revision in the root `Cargo.toml` (`[workspace.dependencies]`); bump it there for all plugins at once
