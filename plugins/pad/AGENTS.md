# Shimmer Pad

Loaded when working in `plugins/pad`. The suite-wide rules are in the root `AGENTS.md`.

A slow ambient pad. MIDI in, stereo out, twelve voices. Ported from `prototypes/jsfx`.

- `src/lib.rs` - Plugin entry point, parameters. Reads them once per block into `PadSettings`
- `src/pad.rs` - `ShimmerPad`: voices (three detuned saws and noise through a low-pass that
  opens with the swell), chorus, the hall (`suite_common::dsp::Hall`), the shimmer
- `src/editor.rs` - egui GUI: plum pedal, twelve knobs, Chorus and Shimmer footswitches

How it behaves:

- A note that is played again takes the voice it has; a thirteenth takes the quietest voice
  that is let go, or the quietest of all
- The filters are set every 32 samples counted across host blocks, so the output does not
  depend on the host's block size
- Shimmer reads the hall's tail back an octave up and feeds it into the hall again
- The two footswitches fade their effect in and out in 20 ms; off is the same as the knob at 0
- All notes off lets go of every key; all sound off stops voices and tails at once

| Parameter (id) | Range | Default | Description |
|----------------|-------|---------|-------------|
| Volume (`volume`) | -40 to +12 dB | -6 dB | Output level |
| Attack (`attack`) | 0.01-6 s | 1.5 s | Time to swell to full level |
| Release (`release`) | 0.05-10 s | 3.5 s | Time to fall by 40 dB once the key is let go |
| Cutoff (`cutoff`) | 150-8000 Hz | 1600 Hz | Where the filter of a middle C stands once swelled |
| Dry (`dry`) | 0-100 % | 45 % | Level of the pad itself |
| Detune (`detune`) | 0-30 ct | 9 ct | Distance of the outer saws from the middle one |
| Breath (`breath`) | 0-100 % | 18 % | Noise in each voice |
| Motion (`motion`) | 0-100 % | 35 % | How far the filters wander |
| Chorus (`chorus_on`), Chorus Depth (`chorus`) | on / off, 0-100 % | on, 60 % | |
| Reverb (`reverb`) | 0-100 % | 60 % | Level of the hall |
| Reverb Decay (`reverb_decay`) | 1-20 s | 7 s | Time for the tail to fall by 60 dB |
| Shimmer (`shimmer_on`), Shimmer Level (`shimmer`) | on / off, 0-60 % | on, 30 % | How much of the tail comes back an octave up |

Tests sit next to the code (`pad.rs`): tuning at several rates, envelopes, voice stealing, the effects and their switches, bounded output with everything up. Two ignored tests are tools for working on the sound:

```bash
# Level, tuning and brightness per note, and the cost
cargo test -p pad --release pad_report -- --ignored --nocapture

# WAV file in target/renders: pad.wav
cargo test -p pad --release render_wavs -- --ignored
```
