# Glass Bells

Loaded when working in `plugins/bells`. The suite-wide rules are in the root `AGENTS.md`.

Soft glassy bells for sparse lines far behind a mix. MIDI in, stereo out, twelve voices; a
bell rings out by itself, so note-off does nothing. Ported from `prototypes/jsfx`.

- `src/lib.rs` - Plugin entry point, parameters. Reads them once per block into `BellSettings`
- `src/bells.rs` - `GlassBells`: voices (a sine bent by another at 3.5 times its pitch, and an
  octave partial), ping-pong delay, the hall (`suite_common::dsp::Hall`)
- `src/editor.rs` - egui GUI: pine green pedal, ten knobs, Delay and Reverb footswitches

How it behaves:

- The glass of a strike (how far the modulator bends the tone) fades in 0.35 s, faster than
  the tone; Brightness and velocity set how much there is
- Decay is the time the A above middle C takes to fall by 40 dB; lower bells ring longer
- Each note has its own place between the speakers. The delay takes the bells in on the left
  and bounces them right and back, each bounce a little darker
- A new bell takes a voice that has rung out, or else the one struck longest ago
- All sound off stops bells and tails at once
- The delay time glides when its knob is turned. The footswitches fade in 10 ms; with Delay
  off the echoes are out of the hall as well

| Parameter (id) | Range | Default | Description |
|-----------|-------|---------|-------------|
| Volume (`volume`) | -40 to +12 dB | -8 dB | Output level |
| Brightness (`brightness`) | 0-100 % | 45 % | Glass in the strike |
| Decay (`decay`) | 0.3-10 s | 3 s | How long a bell rings |
| Attack (`attack`) | 0.5-50 ms | 6 ms | Fade in of the strike |
| Dry (`dry`) | 0-100 % | 55 % | Level of the bells themselves |
| Delay (`delay_on`) | on / off | on | |
| Delay Time, Feedback, Mix (`delay_time`, `delay_feedback`, `delay_mix`) | 50-1500 ms, 0-90 %, 0-100 % | 445 ms, 45 %, 35 % | |
| Reverb (`reverb_on`) | on / off | on | |
| Reverb Decay, Mix (`reverb_decay`, `reverb_mix`) | 0.5-20 s, 0-100 % | 6 s, 50 % | |

Tests sit next to the code (`bells.rs`): tuning at several rates, envelopes, voice stealing, the effects and their switches, bounded output with everything up. Two ignored tests are tools for working on the sound:

```bash
# Level, tuning and brightness per note, and the cost
cargo test -p bells --release bells_report -- --ignored --nocapture

# WAV file in target/renders: bells.wav
cargo test -p bells --release render_wavs -- --ignored
```
