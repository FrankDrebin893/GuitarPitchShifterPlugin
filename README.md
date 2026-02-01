# Guitar Pitch Shifter Plugin

A real-time guitar pitch-shifting VST3/CLAP plugin built with Rust and the [nih-plug](https://github.com/robbert-vdh/nih-plug) framework.

## What It Does

Guitar Pitch Shifter shifts your guitar audio by semitones (-12 to +12) with minimal latency, making it suitable for live playing. Drop your tuning without retuning your guitar, or shift up for capo effects.

## How It Works

The pitch shifter uses a **variable-rate playback** algorithm:

1. **Circular buffer** - Incoming audio is written to a ring buffer
2. **Variable-rate reading** - Audio is read back at a different rate based on the pitch shift (rate = 2^(semitones/12))
3. **Linear interpolation** - Smooths the output when reading between sample positions
4. **Crossfade resync** - When the read and write positions drift too far apart, the algorithm crossfades to a new position to avoid discontinuities

This approach prioritizes low latency over perfect audio quality, which is ideal for live performance.

## Parameters

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Semitones | -12 to +12 | 0 | Pitch shift in semitones |
| Latency | 2-50 ms | 5 ms | Trade-off: lower = more responsive but more artifacts |
| Smoothness | 0.5-10 ms | 2 ms | Crossfade duration for resync |

## Building

Requires [Rust](https://rustup.rs/) to be installed.

```bash
# Build release VST3 and CLAP bundles
cargo xtask bundle guitar_pitch_shifter_plugin --release
```

Output will be in `target/bundled/`.

## Installing

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