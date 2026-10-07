# Guitar Pitch Shifter Plugin

A real-time guitar pitch-shifting VST3/CLAP plugin built with Rust and the [nih-plug](https://github.com/robbert-vdh/nih-plug) framework.

## What It Does

Guitar Pitch Shifter shifts your guitar audio by semitones (-12 to +12) with minimal latency, making it suitable for live playing. Drop your tuning without retuning your guitar, or shift up for capo effects.

## How It Works

The pitch shifter uses **variable-rate playback with waveform-matched splices**:

1. **Circular buffer** - Incoming audio is written to a ring buffer
2. **Variable-rate reading** - Audio is read back at a different rate based on the pitch shift (rate = 2^(semitones/12)), with cubic interpolation between samples
3. **Splice only when needed** - The read position slowly drifts away from (or towards) the write position. It only jumps when it has drifted as far as it may
4. **Waveform-matched jumps** - Each jump lands where the waveform lines up with itself, normally exactly one pitch period away, and is bridged by a short crossfade. A sustained note therefore continues without a seam

Between splices the output is a plain resampled copy of the input, and at 0 semitones the signal passes through untouched.

The delay follows the note being played: about one pitch period at most (12 ms on the low E string, under 3 ms high up the neck), not a fixed amount.

## Parameters

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Semitones | -12 to +12 | 0 | Pitch shift in semitones |
| Max Latency | 2-50 ms | 15 ms | Upper limit for the delay, and the longest waveform period that can be spliced cleanly |
| Smoothness | 0.5-10 ms | 2 ms | Crossfade duration at each splice |

Tips:

- Single notes need Max Latency to cover one period of the lowest note: about 12 ms for low E, 14 ms for drop D. Lower settings still work but the low strings get rougher.
- Chords have a much longer combined period (about 24 ms for a power chord on the low E string). Raising Max Latency to 25-30 ms makes chords noticeably cleaner. Single notes keep their short delay either way.

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
xcopy /E /I /Y "target\bundled\GuitarPitchShifterPlugin_v10.vst3" "C:\Program Files\Common Files\VST3\GuitarPitchShifterPlugin_v10.vst3"
```

### macOS

```bash
cp -r target/bundled/GuitarPitchShifterPlugin_v10.vst3 ~/Library/Audio/Plug-Ins/VST3/
```

### Linux

```bash
cp -r target/bundled/GuitarPitchShifterPlugin_v10.vst3 ~/.vst3/
```

Then rescan plugins in your DAW.