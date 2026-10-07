# Guitar Pitch Shifter Plugin

Real-time guitar pitch-shifting VST3/CLAP plugin built with Rust and nih-plug framework.

## Purpose

Shift guitar audio by semitones (-12 to +12) with minimal latency for live playing. Uses variable-rate playback with waveform-matched splices.

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
- `src/pitch_shifter.rs` - DSP: circular buffer, variable-rate playback, waveform-matched splices
- `src/editor.rs` - egui GUI with dial controls

### Pitch Shifting Algorithm

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

## Parameters

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Semitones | -12 to +12 | 0 | Pitch shift amount |
| Max Latency | 2-50 ms | 15 ms | Delay limit = longest period that splices cleanly |
| Smoothness | 0.5-10 ms | 2 ms | Crossfade length at each splice |

Actual delay is about one pitch period of the note being played, not the Max Latency value.
Single notes need Max Latency >= one period of the lowest note (12 ms for low E). Chords have a
longer combined period (~24 ms for a power chord on low E) and get cleaner at 25-30 ms.

## Versioning

When iterating on the plugin:
1. Increment version in `bundler.toml` (name = "GuitarPitchShifterPlugin_vX")
2. Update `NAME` constant in `src/lib.rs`
3. Update `VST3_CLASS_ID` in `src/lib.rs` (change last digit)

This allows testing multiple versions side-by-side in DAW.

## Testing

**Always run `cargo test` before committing changes.**

Tests are in `src/pitch_shifter.rs` and cover:
- Bit-exact passthrough at 0 semitones
- Artifact level of shifted sines, harmonic tones and chords (`artifact_ratio_db`: energy that
  is not at the expected shifted frequencies)
- Latency following the pitch period, and staying within the limit
- Stereo alignment, level, no clicks under parameter changes
- Buffer/state management

Two ignored tests are tools for working on the sound:

```bash
# Table of artifact levels and latency per note and shift. Compare before/after a DSP change.
cargo test --release quality_report -- --ignored --nocapture

# Before/after WAV files in target/renders for listening.
# Optional env vars: PITCH_SHIFTER_INPUT_WAV=<recording>, PITCH_SHIFTER_LATENCY_MS=<ms>
cargo test --release render_wavs -- --ignored
```

## Code Conventions

- Always run tests before committing
- Optimize for low latency over audio quality
- Never allocate or resize buffers on the audio thread (all buffers are fixed-size)
- Measure DSP changes with `quality_report` before judging them by ear
- Keep DSP code in pitch_shifter.rs separate from plugin boilerplate
