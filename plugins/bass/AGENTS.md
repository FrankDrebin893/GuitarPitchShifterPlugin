# Bass Synth

Loaded when working in `plugins/bass`. The suite-wide rules are in the root `AGENTS.md`.

A plucked string bass, one note at a time. MIDI in, the same signal on both outputs. Two
voices that share the string: Metal (picked; clean lows under a distorted band of mids) and
Groove (finger; a tone filter and a little tube drive). Ported from `prototypes/jsfx`.

- `src/lib.rs` - Plugin entry point, parameters. Reads them once per block into `BassSettings`
- `src/bass.rs` - `Bass`: the string (a delay line that feeds itself through a low-pass), the
  two plucks, the sine under the string, the two output stages; `Voice` and its settings
- `src/editor.rs` - egui GUI: cobalt pedal, eight knobs, one footswitch per voice

How it behaves:

- A voice footswitch also sets the knobs to that voice's own settings (`Voice::settings`),
  like a preset. A host that changes Voice itself leaves the knobs alone
- Slide at 0 ms plucks every note. Above 0, a note that starts while another is held slides
  to it without a new pluck, in the Slide time
- Tone is the low-pass behind the string on Groove, and behind the drive on Metal
- Letting go of the key damps the string (70 ms on Metal, 120 ms on Groove), and so does all
  notes off. All sound off stops it at once
- A note that is ringing keeps the string it was plucked with when the voice changes; what
  stands behind the string changes at once. Sustain reaches the ringing note
- Notes are within 1 cent from 44.1 to 192 kHz (`bass_report`)
- `lib.rs` takes every default from `BassSettings::default`, which is Metal's settings

| Parameter (id) | Range | Metal | Groove | Description |
|----------------|-------|-------|--------|-------------|
| Voice (`voice`) | Metal / Groove | | | Pluck and output stage |
| Volume (`volume`) | -30 to +12 dB | -2 dB | -3 dB | Output level |
| Drive (`drive`) | 0-100 % | 50 % | 15 % | Distortion of the mids (Metal), tube drive (Groove) |
| Brightness (`brightness`) | 0-100 % | 55 % | 35 % | How hard the string is plucked and how long its top rings |
| Sub (`sub`) | 0-100 % | 45 % | 25 % | Level of the sine under the string |
| Tone (`tone`) | 300-4000 Hz | 3200 Hz | 1100 Hz | Low-pass |
| Sustain (`sustain`) | 0.5-8 s | 3.5 s | 3 s | Time for a held note to fall by 60 dB |
| Slide (`slide`) | 0-300 ms | 0 ms | 60 ms | Slide between overlapping notes; 0 plucks every note |
| Transpose (`transpose`) | -24 to +24 st | 0 | 0 | |

Tests sit next to the code (`bass.rs`): tuning at several rates, envelopes, the two voices, bounded output with everything up. Two ignored tests are tools for working on the sound:

```bash
# Level, tuning and brightness per note, and the cost
cargo test -p bass --release bass_report -- --ignored --nocapture

# WAV files in target/renders: bass_metal.wav, bass_groove.wav
cargo test -p bass --release render_wavs -- --ignored
```
