# Guitar Amp

Loaded when working in `plugins/amp`. The suite-wide rules are in the root `CLAUDE.md`; the
reasons behind the amp's design are in `DESIGN.md` next to this file; open work is the
`plugin:amp` issues.

A guitar amp with a full signal chain, all our own designs: own names, no real makers' names,
trademarks or circuit names anywhere (code, comments, docs, commit messages). Three amps, each
with its own cabinet: Klar (clean, glassy), Brøl (mid-forward crunch), Torden (modern tight high
gain). They share the six dials.

- `src/lib.rs` - Plugin entry point, parameters. Reads them once per block into `AmpSettings`
- `src/chain.rs` - `AmpChain`: the whole chain, dial smoothing, bypass crossfade, amp switching
- `src/gate.rs`, `src/drive.rs` - Noise gate; overdrive pedal
- `src/amp/` - `model.rs` (`Amp` and one `AmpModel` of constants per amp), `preamp.rs`,
  `tonestack.rs`, `poweramp.rs`
- `src/cab.rs` - Cabinet: impulse response designed in code at 48 kHz and resampled for other
  rates, direct FIR; a player's response of 641 taps or more keeps its first two blocks as a
  direct FIR and runs the rest through the `Tail`; Mic and Resonance filters behind it
- `src/user_cab.rs` - The player's own cabinets: folder, WAV reading, resampler, trim, cut, level
- `src/cab_stage.rs` - `CabStage` (lock-free hand-off of a cabinet to the audio thread) and
  `CabLoader` (reads the chosen file on a background task)
- `src/delay.rs`, `src/reverb.rs` - Stereo delay; stereo reverb (8-line feedback delay network)
- `src/comp.rs` - Compressor pedal
- `src/mods/` - `chorus.rs`, `flanger.rs`, `phaser.rs`, `tremolo.rs`, with `lfo.rs` and `line.rs`
  (the modulated delay line of chorus and flanger)
- `src/order.rs` - `ChainOrder`: which pedals stand in front of the amp and which behind the
  cabinet, in what order; the rules, and `StoredOrder` (saved as `chain-order`)
- `src/dsp/` - `filters.rs` (f64 biquads), `oversample.rs` (minimum-phase IIR half-bands),
  `shaper.rs` (antialiased clippers), `resample.rs` (windowed-sinc resampler for cabinets),
  `pair.rs` (two samples' divisions and square roots as one SSE2 pair), `fft.rs` (radix-2 real
  FFT), `partitioned.rs` (`Tail`: the late part of a long impulse response in FFT partitions)
- `src/presets.rs` - Factory presets as plain data, and `preset_report`
- `src/dial_memory.rs` - Each amp's six dial positions, kept by the editor and saved with the
  project (`amp-dials`)
- `src/lamps.rs` - Gate-open and clip lamps: what the chain tells the editor, and how long the
  editor shows it
- `src/tuner.rs` - Tuner: pitch detector on the mono input, `TunerReading` (one atomic shared
  with the editor), `Note`
- `src/editor.rs` - egui GUI: oxblood head (1590 x 694 window) with the preset stepper or the tuner
  display top centre; draws each of the ten pedals, each in its own paint with its own art
- `src/board.rs` - The row of pedals under the head in the chain's order, the amp's marker,
  and dragging a pedal to another place

Signal flow:

1. Input (a stereo input is averaged to mono), input gain, gate. The gate's detector reads the
   input before the input gain; no lookahead. The tuner, when on, hears the same input in front
   of the input gain and the gate
2. One oversampled region: the pedals in front of the amp in their order, preamp stages, tone
   stack, power amp. 4x at 44.1 and 48 kHz, 2x from 88.2 kHz on. The clippers are antialiased
   (antiderivative method; second order where the gain is highest)
3. Cabinet: a short impulse response (5 to 10 ms) with the speaker resonance as a filter behind
   it, then the Mic and Resonance dials, DC blocker
4. Two channels from here: the pedals behind the cabinet in their order. Delay and reverb add
   wet to an untouched dry signal, smooth their own settings, ring out when switched off and
   cost nothing once idle
5. Output level, safety clip

How it behaves:

- Amps differ only by constants in `amp/model.rs`. Tune there, never with `match amp` in the DSP
- Dials are read every 32 samples counted across host blocks, so the output does not depend on
  the host's block size
- Switching amp fades the amp out over 5 ms, reconfigures it at silence and fades it back in
- Bypass crossfades to the untouched input in 10 ms and drops the effects' tails
- With every pedal off and the cabinet dials at 5.0 the chain is bit-identical to the amp alone
- Latency is 0.1 to 0.3 ms and nothing is reported to the host
- A mono host layout gets the left channel of the effects
- `process` returns `KeepAlive` while a delay or reverb tail rings. VST3 hosts are told the tail
  is infinite from the start (`VST3_INFINITE_TAIL`), as are those of the pad and the bells
- Tuner on gives the amp silence and turns its output down in 10 ms (exactly silent, tails
  included); off brings it back as fast. Bypass still passes the input while tuning. The tuner
  costs nothing when off and under 3 us in its worst 64-sample block when on
- The tuner reads 48 Hz to 1420 Hz against A = 440 Hz (`REFERENCE_A_HZ`), a new reading every
  30 ms, none for silence, noise, hum or chords. A note is shown from a pluck on (a rise of
  12 dB within 50 ms; 6 dB to change notes or pluck again), and let go after a second if it
  does not fade like a string. A loaded
  project never opens with the tuner on
- Besides its own cabinet an amp can play through a WAV impulse response from
  `Documents/Hojt Audio/Cabinets` (or the folder in `HOJT_CABINETS_DIR`), picked on the Cab pedal.
  Mono or the left channel of stereo, 16/24/32-bit integer or 32-bit float, any rate; resampled,
  trimmed so the sound arrives within 0.2 ms, cut to 40 ms, levelled like the built-in ones.
  Latency stays the same
- The cabinet choice is a persisted file name, not a parameter and not part of a preset. It
  stays when another amp is picked. A missing or unusable file plays the amp's own cabinet
- Files are read on a background task (and in `initialize`), never on the audio thread; the taps
  reach it through `CabStage` without allocating, freeing or waiting
- The folder is created the first time the stepper is used, not by loading the plugin
- A 40 ms cabinet adds about 0.1 % of a core against the amp's own at any rate. Its late part is
  convolved in FFT partitions whose work is spread over the samples of a block: no added latency,
  the worst block as with the amp's own cabinet, output independent of the host's block size
- A swap to a long cabinet of the player's own starts its crossfade once its late part is there:
  about the length of the response after a short or built-in cabinet, two blocks after another
  long one. The cabinet before plays on meanwhile
- The clippers work on two samples at a time (`dsp/pair.rs`), the same to the bit as one by one.
  That path is only compiled in optimised x86-64 builds: run `cargo test -p amp --release` too
  when touching the clippers
- `hound` is the amp's one dependency besides nih-plug; it reads the WAV files
- Each amp remembers its own six dials: the selector on the head stores them under the amp that
  is left and sets them to what the amp that is picked had (defaults if never left). The dials
  are still six shared parameters. A host that changes Amp itself leaves the dials alone; a
  preset's dials become its amp's memory
- An amp that is switched to starts at the dials' target, at the silent sample of the switch
- The Gate pedal's `OPEN` lamp is lit while the gate is on and lets the guitar through; `CLIP`
  beside Output is lit for 150 ms after the output went over the safety clip's knee
- While the drive pedal is on, the second stage of Brøl and of Torden clips at twice the rate
  (`StageModel.finer_when_driven`) and Klar's power stage turns itself down under a heavy load
  (`PowerModel.driven_sag`), so no setting of the pedal aliases above -70 dB. With the pedal
  off the output is the same to the bit
- The amps are matched by loudness (BS.1770, `test_util::loudness_lufs`), not RMS, for a guitar
  recorded at -24 dBFS RMS (`REFERENCE_DI_DB`). A clean amp follows the guitar dB for dB and
  one that clips hardly does, so they can only meet at one level: 6 dB quieter the clean
  settings are 2 to 4 LU under the others, 6 dB hotter as much over. The Input knob is what
  moves a guitar to that level
- Klar's power stage holds its peaks (`PowerModel.clean_to`): straight up to a knee, so a note
  held at -24 dBFS RMS has no distortion (under -60 dB), and rounding off the attacks that go
  over. Peaks stay under the safety clip's knee up to a guitar at -12 dBFS RMS. Brøl below
  Gain 5 is made up in front of its power stage for the same reason
- The built-in cabinets are designed at 48 kHz and resampled for every other rate (within
  0.05 dB of each other from 60 Hz to 10 kHz). A rate is designed once and kept
- `lib.rs` takes every parameter default and range from `AmpSettings::default` and the limits in
  the DSP modules; a test pins the values
- A preset holds everything except Bypass, Input and Output, so every preset must fit at
  Output 0 dB (a test checks peak and level of each). It holds the pedal order too. New
  presets go at the end: a project remembers its preset by its number
- The pedal order (`order.rs`): Gate always first, amp and cabinet one block in the middle.
  Drive only in front of the amp; Delay and Reverb only behind the cabinet; Comp, Chorus,
  Phaser, Flanger and Tremolo on either side. Default: Comp, Drive, Phaser, amp, Chorus,
  Flanger, Tremolo, Delay, Reverb, which with the new pedals off is the chain as it was, to
  the bit
- In front of the amp a pedal runs on one channel at the oversampled rate, behind the cabinet
  on two at the host's rate (chorus, flanger and phaser a quarter turn apart left and right).
  Either way without latency
- The order is saved with the project (`chain-order`, a list of pedal ids with `amp` between
  the sides), not a parameter and not automatable. The chain reads it with the dials: the
  output fades to silence in 5 ms, the pedals change places, it fades back. A pedal that
  changes sides starts from rest. A list that is damaged or from another build is put right,
  never refused
- The compressor's Level at 5 is as loud as the pedal off (within 0.3 LU at the reference
  level); Sustain 10 adds at most 20 dB to what is quiet, hiss included
- Cost with everything on (Torden): about 2.0 % of a core at 48 kHz, 2.6 % at 96 kHz, 5.5 % at
  192 kHz, on a fast core. The cores of a machine differ (1.33 times on the development
  machine): the reports pin their thread (`test_util::pin_thread`)

| Parameter (id) | Range | Default | Description |
|----------------|-------|---------|-------------|
| Bypass (`bypass`) | on / off | off | The On switch on the head |
| Input (`in_gain`) | -24 to +24 dB | 0 dB | Gain in front of everything except the gate's detector |
| Gate (`gate_on`) | on / off | on | Noise gate on the input |
| Gate Threshold (`gate_thresh`) | -80 to -20 dB | -60 dB | Input level that opens the gate (closes 3 dB lower) |
| Gate Release (`gate_release`) | 20-500 ms | 100 ms | Time to close after a 40 ms hold |
| Comp (`comp_on`) | on / off | off | Compressor pedal |
| Comp Sustain (`comp_sustain`) | 0.0-10.0 | 5.0 | How much it evens out: threshold and ratio together |
| Comp Level (`comp_level`) | 0.0-10.0 | 5.0 | The pedal's output, -12 to +12 dB around as loud as off |
| Drive (`drive_on`) | on / off | off | Overdrive pedal in front of the amp |
| Drive Gain (`drive_gain`) | 0.0-10.0 | 3.0 | Amount of clipping in the pedal |
| Drive Tone (`drive_tone`) | 0.0-10.0 | 5.0 | The pedal's top end, 1.5 to 7 kHz |
| Drive Level (`drive_level`) | 0.0-10.0 | 5.0 | The pedal's output, -20 to +20 dB into the amp |
| Amp (`amp`) | Klar / Brøl / Torden | Brøl | Amp and its cabinet. Variant ids `klar`, `brol`, `torden` |
| Gain, Bass, Mid, Treble, Presence, Master (`gain`, `bass`, `mid`, `treble`, `presence`, `master`) | 0.0-10.0 | 5.0 | The amp's dials |
| Chorus, Phaser, Flanger, Tremolo (`chorus_on`, `phaser_on`, `flanger_on`, `trem_on`) | on / off | off | Modulation pedals |
| Chorus Rate (`chorus_rate`) | 0.1-6 Hz | 0.8 Hz | Speed of the sweep, 7 to 12 ms |
| Phaser Rate (`phaser_rate`) | 0.1-8 Hz | 0.5 Hz | Speed of the sweep, 200 Hz to 1.6 kHz |
| Flanger Rate (`flanger_rate`) | 0.05-5 Hz | 0.3 Hz | Speed of the sweep, 0.5 to 6 ms |
| Tremolo Rate (`trem_rate`) | 0.5-12 Hz | 5 Hz | Speed of the volume wobble |
| Depth (`chorus_depth`, `phaser_depth`, `flanger_depth`, `trem_depth`) | 0-100 % | 50 % | Width of the sweep; for chorus and flanger also the level of the voice |
| Cabinet (`cab_on`) | on / off | on | Off passes the amp at about the same level, for a cabinet elsewhere |
| Cab Mic (`cab_mic`) | 0.0-10.0 | 5.0 | Darker to brighter microphone position |
| Cab Resonance (`cab_res`) | 0.0-10.0 | 5.0 | Less or more low thump |
| Delay (`delay_on`) | on / off | off | Stereo delay behind the cabinet |
| Delay Time (`delay_time`) | 20-1000 ms | 350 ms | Time between repeats |
| Delay Feedback (`delay_feedback`) | 0-90 % | 35 % | Level of each repeat against the one before |
| Delay Mix (`delay_mix`) | 0-100 % | 25 % | Level of the repeats |
| Reverb (`reverb_on`) | on / off | off | Stereo reverb behind the delay |
| Reverb Decay (`reverb_decay`) | 0.3-6.0 s | 1.5 s | Time for the tail to fall by 60 dB |
| Reverb Mix (`reverb_mix`) | 0-100 % | 20 % | Level of the tail |
| Output (`out_level`) | -30 to +6 dB | 0 dB | Output level, in front of the safety clip |
| Tuner (`tuner`) | on / off | off | Silent tuning. Not automatable, not in presets, not changed when a project is loaded |

Dials are stored 0.0-1.0 and shown as 0.0-10.0.

Amp tests sit next to the code. The ones for the whole chain are in `chain.rs`. These ignored
tests are the tools for working on the sound:

```bash
# Levels, distortion, aliasing, tightness, gate, drive, cabinet, effects, latency and CPU time.
# Run before and after a DSP change and compare.
cargo test -p amp --release amp_report -- --ignored --nocapture

# How loud each amp is heard (LUFS), its peaks and crest per Gain, for guitars recorded from
# -36 to -12 dBFS RMS, and the distortion of a held note at those levels.
cargo test -p amp --release loudness_report -- --ignored --nocapture

# One number per amp, Gain and pedal that changes with any bit of the output. Save the lines
# before and after a change that must leave a sound as it is, and compare them.
cargo test -p amp --release fingerprint_report -- --ignored --nocapture

# Delay and reverb by themselves: echo times, decay times, density, width, cost.
cargo test -p amp --release effects_report -- --ignored --nocapture

# Time per stage of the chain, and level, peak and tightness of every factory preset.
cargo test -p amp --release stage_report -- --ignored --nocapture
cargo test -p amp --release preset_report -- --ignored --nocapture

# How every preset's gate treats ringing notes, quiet playing, palm mutes and hiss.
cargo test -p amp --release preset_gate_report -- --ignored --nocapture

# Compressor and modulation pedals by themselves: what they do per setting, and their cost.
cargo test -p amp --release comp_report -- --ignored --nocapture
cargo test -p amp --release mods_report -- --ignored --nocapture

# Tuner: error in cents per note and signal, time to the first reading, drift, cost.
cargo test -p amp --release tuner_report -- --ignored --nocapture

# The player's cabinets: cost per length and rate, latency, what loading does to a response.
cargo test -p amp --release user_cab_report -- --ignored --nocapture

# Long cabinets: direct FIR against the tail, per length and rate; error, evenness, swaps.
cargo test -p amp --release convolver_report -- --ignored --nocapture

# The three built-in cabinets as 48 kHz WAV files in target/renders/cabinets.
cargo test -p amp --release export_cabinets -- --ignored --nocapture

# A DI guitar through each amp and setting, as WAV files in target/renders (amp_*.wav).
# Optional: AMP_INPUT_WAV=<recording>. render_effects writes the effects alone (amp_fx_*.wav),
# render_presets every preset (amp_preset_*.wav) and all of them in a row, render_comp and
# render_mods those pedals alone (amp_comp_*.wav, amp_mod_*.wav)
cargo test -p amp --release render_wavs -- --ignored
cargo test -p amp --release render_effects -- --ignored
cargo test -p amp --release render_presets -- --ignored
cargo test -p amp --release render_comp -- --ignored
cargo test -p amp --release render_mods -- --ignored
```
