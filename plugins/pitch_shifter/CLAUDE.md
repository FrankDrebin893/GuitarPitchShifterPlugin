# Pitch Shifter

Loaded when working in `plugins/pitch_shifter`. The suite-wide rules are in the root `CLAUDE.md`.

Shift guitar audio by semitones (-12 to +12) with minimal latency for live playing. Uses variable-rate playback with waveform-matched splices.

- `src/lib.rs` - Plugin entry point, parameters, nih-plug integration
- `src/pitch_shifter.rs` - DSP: circular buffer, variable-rate playback, waveform-matched splices
- `src/editor.rs` - egui GUI: orange pedal, three knobs, bypass footswitch

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
| Bypass | on / off | off | The footswitch. Bypass is a shift of 0 semitones, so it switches without a click |

Actual delay is about one pitch period of the note being played, not the Max Latency value.
Single notes need Max Latency >= one period of the lowest note (12 ms for low E). Chords have a
longer combined period (~24 ms for a power chord on low E) and get cleaner at 25-30 ms.

Its CLAP ID and VST3 class ID predate the suite and are kept as they were.

Measure DSP changes with `quality_report` before judging them by ear.

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
