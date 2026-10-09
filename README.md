# Audio Plugin Suite

A suite of VST3/CLAP plugins built with Rust and the [nih-plug](https://github.com/robbert-vdh/nih-plug) framework.

| Plugin | Type | Location |
|--------|------|----------|
| Guitar Pitch Shifter | Effect | `plugins/pitch_shifter` |
| Drum Synth | Instrument (early skeleton) | `plugins/drums` |

Shared code (vendor string, dial widget, colour theme) lives in `crates/suite_common`.

## Guitar Pitch Shifter

Shifts your guitar audio by semitones (-12 to +12) with minimal latency, making it suitable for live playing. Drop your tuning without retuning your guitar, or shift up for capo effects.

### How It Works

The pitch shifter uses a **variable-rate playback** algorithm:

1. **Circular buffer** - Incoming audio is written to a ring buffer
2. **Variable-rate reading** - Audio is read back at a different rate based on the pitch shift (rate = 2^(semitones/12))
3. **Linear interpolation** - Smooths the output when reading between sample positions
4. **Crossfade resync** - When the read and write positions drift too far apart, the algorithm crossfades to a new position to avoid discontinuities

This approach prioritizes low latency over perfect audio quality, which is ideal for live performance.

### Parameters

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Semitones | -12 to +12 | 0 | Pitch shift in semitones |
| Latency | 2-50 ms | 5 ms | Trade-off: lower = more responsive but more artifacts |
| Smoothness | 0.5-10 ms | 2 ms | Crossfade duration for resync |

## Drum Synth

A fully synthesized drum instrument: every sound is generated in the plugin, so there are no sample files to install or locate. It takes MIDI in and follows the General MIDI drum map.

This is an early skeleton. Only the kick is implemented so far, on MIDI note 36 (C1), with a single Gain control (-30 to +6 dB).

## Building

Requires [Rust](https://rustup.rs/) to be installed.

```bash
# Build release VST3 and CLAP bundles
cargo xtask bundle pitch_shifter --release
cargo xtask bundle drums --release

# Run all tests
cargo test --workspace
```

Output will be in `target/bundled/`.

## Installing

Bundle names: `GuitarPitchShifterPlugin_v9.vst3`, `DrumSynth_v1.vst3`. The examples below use the pitch shifter; do the same for the others.

### Windows

Copy the VST3 bundle to your system VST3 folder (requires administrator privileges):

```cmd
xcopy /E /I /Y "target\bundled\GuitarPitchShifterPlugin_v9.vst3" "C:\Program Files\Common Files\VST3\GuitarPitchShifterPlugin_v9.vst3"
```

### macOS

```bash
cp -r target/bundled/GuitarPitchShifterPlugin_v9.vst3 ~/Library/Audio/Plug-Ins/VST3/
```

### Linux

```bash
cp -r target/bundled/GuitarPitchShifterPlugin_v9.vst3 ~/.vst3/
```

Then rescan plugins in your DAW.
