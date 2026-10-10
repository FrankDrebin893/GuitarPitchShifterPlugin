# Audio Plugin Suite

A suite of VST3/CLAP plugins built with Rust and the nih-plug framework. The brand is **HØJT** (Danish for "loud" and "high"). The vendor string is the ASCII form `Hojt Audio` and lives in one place (`crates/suite_common/src/lib.rs`).

## Layout

- `plugins/pitch_shifter/` - Real-time guitar pitch shifter (effect)
- `plugins/drums/` - Synthesized drum instrument (full kit, Rock/Jazz/Metal)
- `plugins/amp/` - Guitar amp simulator (effect). In progress on branch `amp-sim`: see `docs/amp-progress.md`
- `crates/suite_common/` - Shared code: `VENDOR` and `BRAND` constants, `ui` module (the pedal look, see Look below)
- `crates/suite_common/assets/fonts/` - Embedded fonts (SIL OFL) with their licence files
- `xtask/` - nih-plug bundler
- `bundler.toml` - Bundle name per plugin package

Each plugin keeps DSP in its own module(s), separate from `lib.rs` (plugin entry point, parameters, nih-plug integration) and `editor.rs` (egui GUI).

## Build Commands

```bash
# Build release VST3 and CLAP bundles (one plugin at a time)
cargo xtask bundle pitch_shifter --release
cargo xtask bundle drums --release
cargo xtask bundle amp --release

# Quick compile check
cargo check --workspace

# Run tests (always run before committing!)
cargo test --workspace

# Open a plugin's editor as a program, without a DAW (no audio: dummy backend)
cargo run -p pitch_shifter --features standalone -- --backend dummy
cargo run -p drums --features standalone -- --backend dummy
cargo run -p amp --features standalone -- --backend dummy
```

Output location: `target/bundled/<BundleName>.vst3`

## Install

Copy VST3 to system folder (requires admin):
```
xcopy /E /I /Y "target\bundled\HojtPitchShifter.vst3" "C:\Program Files\Common Files\VST3\HojtPitchShifter.vst3"
xcopy /E /I /Y "target\bundled\HojtDrumSynth.vst3" "C:\Program Files\Common Files\VST3\HojtDrumSynth.vst3"
xcopy /E /I /Y "target\bundled\HojtGuitarAmp.vst3" "C:\Program Files\Common Files\VST3\HojtGuitarAmp.vst3"
```

## Look

Every plugin is drawn as a painted stompbox. All of it is vector, drawn with egui's painter in
`crates/suite_common/src/ui/`; there are no image files.

- `theme.rs` - Colours, the three embedded fonts (`install`), text helpers. One paint colour per plugin
- `widgets.rs` - `param_knob` (fluted knob, name, value on label tape), `footswitch`, `led`,
  `small_footswitch`, `rig_led` and `param_switch` (the switch and LED of a rig, bound to an on/off
  parameter), `stepper` (label tape with a chrome button at each end, for picking one of many),
  `tuner_display` (note name and a row of lamps on one strip of label tape) and `cents_meter`
  (the lamps alone),
  `silk_frame`, `silk_label`, and `badge` (the HØJT logo: the Ø is a knob, its slash the pointer)
- `frame.rs` - `pedal`: enclosure, screws, jack captions, tilted name band, model number, logo.
  A plugin describes itself with a `PedalStyle`. For a window with several enclosures: `rig`
  (the bare bench), `head` (wide enclosure with everything a pedal has printed on it) and
  `mini_pedal` (small enclosure with only a title)

Controls are placed at fixed positions relative to the window's top left corner. Knobs: drag up and
down, Shift for fine steps, double-click for the default.

### Style guide

Every plugin, existing and new, follows these. Copy `plugins/pitch_shifter/src/editor.rs` (few
controls, portrait) or `plugins/drums/src/editor.rs` (many controls, landscape) as the starting point.

- **One pedal per plugin.** The whole window is one enclosure from `ui::pedal`. No panels, tabs,
  menus or default egui widgets on top of it
- **Paint:** each plugin has its own saturated paint colour, a `PAINT_*` constant in `theme.rs`.
  Taken: orange (Pitch Shifter), teal (Drum Synth), oxblood red (Guitar Amp). A new one must be clearly different from those
  and dark enough for cream print to read on it
- **Print on the paint is always `SILK` (cream).** Black is only for knobs, label tape and the logo plate.
  `BRAND_ORANGE` is only the pointer in the logo
- **Fonts, by role:** `display` (Bebas Neue) for every caption, in capitals; `model` (Bowlby One SC)
  only for the name on the band; `mono` (IBM Plex Mono) only for values on label tape
- **Top to bottom:** jack captions, knobs, name band, footswitches. Model number bottom left and
  the logo badge bottom right come from `ui::pedal` and stay there
- **Knobs:** every continuous parameter is a `param_knob`. Radius about 48 for the one main control,
  about 30 for normal ones, about 19 for a row of secondary ones inside a `silk_frame` with a title.
  The value is always shown on label tape under the name, formatted by the parameter itself
- **Footswitches:** every on/off or pick-one-of-few parameter is a `footswitch` with an `led` above
  it (lit = on or selected) and, when there are several, a `silk_label` below
- **Name band:** the plugin's plain name (what it does, two short words), with an `Ornament` that
  hints at it. Add a new `Ornament` variant in `frame.rs` for a new plugin
- **Model number:** two letters from the name and a number, like `PS-10` and `DS-2`. It is print,
  not the version, and does not change with releases
- **Jack captions** say what really goes in and out (`In`, `MIDI In`, `Stereo Out`)
- **Window size:** fixed, sized to the controls. Portrait about 400 wide for up to four controls,
  landscape about 660 wide for more
- **Clean factory paint:** no wear, textures, gradients or images
- **New shared pieces go in `suite_common::ui`,** not in a plugin's editor, so every plugin gets them

- **Rigs:** a plugin made of several components (the Guitar Amp) is the one exception to one
  pedal per plugin. Its window is a `rig`: one `head` across the top, which carries the name band,
  model number and logo, and from left to right in signal order a row of `mini_pedal`s below it.
  All of them have the plugin's one paint colour. On a head and on mini pedals the switches are
  `small_footswitch` and the LEDs `rig_led` (an unlit `led` disappears on dark paint), beside or
  above the switch. A mini pedal holds two knobs of radius 24 side by side or three of radius 17
  in a triangle. Everything else in this guide applies
- **Pick one of many (presets):** a `stepper` with a `silk_label` caption above it, top centre of
  the head on the jack-caption line, at least 12 px clear of the knob ticks. No dropdowns, lists
  or pop-ups. Step arrows are engraved in chrome buttons; silk triangles on the paint mean signal
  direction at the jacks only. Names have at most 14 characters and are our own words. A trailing
  `*` on the tape means a control was moved since loading
- **A preset choice is never a parameter.** The editor sets the real parameters through the
  `ParamSetter` and keeps only the index in a `#[persist]` field
- **Tuner:** a `small_footswitch` with `rig_led` and a `silk_label` on the head, beside the On
  switch and further from the amp switches than they are from each other. While it is on, a
  `tuner_display` under its own caption takes the place of the preset `stepper`: note name in
  `mono` capitals at the left, 17 lamps, flat to the left and sharp to the right, the larger
  middle lamp lit alone within 2 cents. Lamps are `LED_ON` and `LED_OFF` on `TAPE`; no needles,
  numbers or other colours. The editor repaints continuously only while the tuner is on

After changing an editor, run it standalone (see Build Commands) and look at it before bundling.

## Plugins

### Pitch Shifter (`plugins/pitch_shifter`)

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

### Drum Synth (`plugins/drums`)

Fully synthesized drums: no sample files, nothing loaded from disk. MIDI in, stereo out, General MIDI drum map. Three kits: Rock, Jazz, Metal.

- `src/lib.rs` - Plugin entry point, parameters, MIDI events
- `src/drum_kit.rs` - The instrument: owns all voices, note map, stereo mix, room send, output limiter
- `src/kit.rs` - `Kit` enum and one `KitPreset` per kit: all the constants that make the kits differ
- `src/voices/` - `kick.rs`, `snare.rs`, `tom.rs`, `cymbal.rs`, and `mod.rs` (`Hit`, `Voice`, `VoicePair`)
- `src/dsp.rs` - Shared building blocks: noise, filters, membrane modes, clippers
- `src/reverb.rs` - Room: 8-line feedback delay network
- `src/editor.rs` - egui GUI: teal pedal, knobs, one footswitch per kit

How the voices work:

1. Kick: sine body with a pitch sweep, one shell mode, band-passed noise click for the beater, saturation
2. Snare and toms: inharmonic membrane modes (`dsp::Modes`) sharing a pitch bend, plus noise for the
   snare wires and the stick. The side stick is the snare voice with short, high modes and no wires
3. Cymbals: a bank of up to 64 two-pole resonators at fixed inharmonic frequencies, plus a band-limited
   noise wash. A hit adds a sine of random phase to each resonator, so repeated hits build up instead
   of restarting. One `Cymbal` serves closed, pedal and open hi-hat: the closed presets have a short
   decay, which is what chokes the open sound
4. Velocity changes the sound, not only the level: pitch bend depth, overtone level, noise brightness
5. Every hit varies slightly in pitch and overtone levels (each voice has its own noise generator)
6. Drums have two voices each (`VoicePair`): a new hit takes the idle one and the previous fades in 3 ms
7. A kit change, Tune and Damping apply to the next hits. Ringing voices keep their sound
8. `dsp::bus_clip` on the output leaves single hits untouched and rounds off the peaks of busy patterns

| MIDI note | Voice | MIDI note | Voice |
|-----------|-------|-----------|-------|
| 35, 36 | Kick | 42 / 44 / 46 | Hi-hat closed / pedal / open |
| 38, 40 | Snare | 49, 57 | Crash 1, Crash 2 |
| 37 | Side stick | 51, 59 / 53 | Ride / Ride bell |
| 41, 43, 45, 47, 48, 50 | Toms, low to high | 52, 55 | China, Splash |

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Kit | Rock / Jazz / Metal | Rock | Sound of every piece and of the room |
| Gain | -30 to +6 dB | -6 dB | Output level |
| Room | 0-100 % | 25 % | Room reverb amount |
| Tune | -6 to +6 st | 0 | Pitch of kick, snare and toms |
| Damping | 0-100 % | 0 % | Shortens the decay of drums and cymbals |
| Kick, Snare, Toms, Hi-Hat, Cymbals | -30 to +6 dB | 0 dB | Level per group |

Stereo positions are from the drummer's seat (hi-hat left, floor tom right).

Not there yet: separate outputs per drum, hi-hat openness from CC4, cymbal choke on note-off.

### Guitar Amp (`plugins/amp`)

In progress on branch `amp-sim`. `docs/amp-progress.md` has the plan, the decisions, the known
problems and the backlog; read it before working on the plugin.

A guitar amp with a full signal chain, all our own designs: own names, no real makers' names,
trademarks or circuit names anywhere (code, comments, docs, commit messages). Three amps, each
with its own cabinet: Klar (clean, glassy), Brøl (mid-forward crunch), Torden (modern tight high
gain). They share the six dials.

- `src/lib.rs` - Plugin entry point, parameters. Reads them once per block into `AmpSettings`
- `src/chain.rs` - `AmpChain`: the whole chain, dial smoothing, bypass crossfade, amp switching
- `src/gate.rs`, `src/drive.rs` - Noise gate; overdrive pedal
- `src/amp/` - `model.rs` (`Amp` and one `AmpModel` of constants per amp), `preamp.rs`,
  `tonestack.rs`, `poweramp.rs`
- `src/cab.rs` - Cabinet: impulse response designed in code per sample rate, direct FIR, Mic and
  Resonance filters behind it
- `src/delay.rs`, `src/reverb.rs` - Stereo delay; stereo reverb (8-line feedback delay network)
- `src/dsp/` - `filters.rs` (f64 biquads), `oversample.rs` (minimum-phase IIR half-bands),
  `shaper.rs` (antialiased clippers)
- `src/presets.rs` - Factory presets as plain data, and `preset_report`
- `src/tuner.rs` - Tuner: pitch detector on the mono input, `TunerReading` (one atomic shared
  with the editor), `Note`
- `src/editor.rs` - egui GUI: oxblood head (960 x 694 window) with the preset stepper or the tuner display
  top centre, over five pedals

Signal flow:

1. Input (a stereo input is averaged to mono), input gain, gate. The gate's detector reads the
   input before the input gain; no lookahead. The tuner, when on, hears the same input in front
   of the input gain and the gate
2. One oversampled region: drive pedal, preamp stages, tone stack, power amp. 4x at 44.1 and
   48 kHz, 2x from 88.2 kHz on. The clippers are antialiased (antiderivative method; second
   order where the gain is highest)
3. Cabinet: a short impulse response (5 to 10 ms) with the speaker resonance as a filter behind
   it, then the Mic and Resonance dials, DC blocker
4. Stereo delay, then stereo reverb. They add wet to an untouched dry signal, smooth their own
   settings, ring out when switched off and cost nothing once idle
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
- `process` returns `KeepAlive` while a delay or reverb tail rings
- Tuner on gives the amp silence and turns its output down in 10 ms (exactly silent, tails
  included); off brings it back as fast. Bypass still passes the input while tuning. The tuner
  costs nothing when off and under 3 us in its worst 64-sample block when on
- The tuner reads 48 Hz to 1420 Hz against A = 440 Hz (`REFERENCE_A_HZ`), a new reading every
  30 ms, none for silence, noise or chords. A loaded project never opens with the tuner on
- `lib.rs` takes every parameter default and range from `AmpSettings::default` and the limits in
  the DSP modules; a test pins the values
- A preset holds everything except Bypass, Input and Output, so every preset must fit at
  Output 0 dB (a test checks peak and level of each)
- Cost with everything on (Torden): about 2 % of a core at 48 kHz, 2.6 % at 96 kHz, 5.6 % at 192 kHz

| Parameter (id) | Range | Default | Description |
|----------------|-------|---------|-------------|
| Bypass (`bypass`) | on / off | off | The On switch on the head |
| Input (`in_gain`) | -24 to +24 dB | 0 dB | Gain in front of everything except the gate's detector |
| Gate (`gate_on`) | on / off | on | Noise gate on the input |
| Gate Threshold (`gate_thresh`) | -80 to -20 dB | -60 dB | Input level that opens the gate (closes 6 dB lower) |
| Gate Release (`gate_release`) | 20-500 ms | 100 ms | Time to close after a 40 ms hold |
| Drive (`drive_on`) | on / off | off | Overdrive pedal in front of the amp |
| Drive Gain (`drive_gain`) | 0.0-10.0 | 3.0 | Amount of clipping in the pedal |
| Drive Tone (`drive_tone`) | 0.0-10.0 | 5.0 | The pedal's top end, 1.5 to 7 kHz |
| Drive Level (`drive_level`) | 0.0-10.0 | 5.0 | The pedal's output, -20 to +20 dB into the amp |
| Amp (`amp`) | Klar / Brøl / Torden | Brøl | Amp and its cabinet. Variant ids `klar`, `brol`, `torden` |
| Gain, Bass, Mid, Treble, Presence, Master (`gain`, `bass`, `mid`, `treble`, `presence`, `master`) | 0.0-10.0 | 5.0 | The amp's dials |
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

# Delay and reverb by themselves: echo times, decay times, density, width, cost.
cargo test -p amp --release effects_report -- --ignored --nocapture

# Time per stage of the chain, and level, peak and tightness of every factory preset.
cargo test -p amp --release stage_report -- --ignored --nocapture
cargo test -p amp --release preset_report -- --ignored --nocapture

# Tuner: error in cents per note and signal, time to the first reading, drift, cost.
cargo test -p amp --release tuner_report -- --ignored --nocapture

# A DI guitar through each amp and setting, as WAV files in target/renders (amp_*.wav).
# Optional: AMP_INPUT_WAV=<recording>. render_effects writes the effects alone (amp_fx_*.wav)
cargo test -p amp --release render_wavs -- --ignored
cargo test -p amp --release render_effects -- --ignored
```

## Adding a Plugin

1. Create `plugins/<name>/` with a `Cargo.toml` that uses the workspace dependencies (copy `plugins/drums/Cargo.toml`)
2. Add a `[<name>]` entry to `bundler.toml`
3. Use `suite_common::VENDOR`. In the editor, call `ui::install` in the build closure and draw with
   `ui::pedal` and a `PedalStyle` (own paint colour in `ui/theme.rs`, model number, ornament)
4. Pick a unique `VST3_CLASS_ID` (16 bytes) and `CLAP_ID`; DAW projects reference plugins by these

## Versioning

A new build replaces the old one in place: install it over the old bundle and every DAW project
picks it up, with its settings. For that to hold, these never change once a plugin is in use:

- the bundle name in `bundler.toml`
- `NAME`, `VST3_CLASS_ID` and `CLAP_ID` in `plugins/<name>/src/lib.rs`
- the `#[id = "..."]` of every existing parameter (adding parameters is fine; removing or renaming
  an id loses that setting in saved projects)

The version number lives in `Cargo.toml` (`[workspace.package]`) and is what hosts show.

Only to compare two builds side by side in the DAW: temporarily give one of them another bundle
name, `NAME`, `VST3_CLASS_ID` and `CLAP_ID`, and do not commit that.

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

Drum tests sit next to the code in `plugins/drums/src`. The ones for the whole instrument are in
`drum_kit.rs` and cover the note map, decay to silence, output bounds at all sample rates, velocity
changing level and brightness, hi-hat choke, the kits differing measurably, Tune, Damping, stereo
placement and the room.

Two ignored tests are tools for working on the drum sound:

```bash
# Table of peak, level, decay time and brightness per piece and kit. Compare before/after a change.
cargo test -p drums --release kit_report -- --ignored --nocapture

# WAV files in target/renders for listening: per kit, every piece at three velocities and a groove.
cargo test -p drums --release render_wavs -- --ignored
```

## Code Conventions

- Always run tests before committing
- Optimize for low latency over audio quality
- Never allocate or resize buffers on the audio thread (all buffers are fixed-size)
- Measure pitch shifter DSP changes with `quality_report` before judging them by ear
- Drum sounds are tuned in `plugins/drums/src/kit.rs`; check `kit_report` and listen to `render_wavs`
- Keep DSP code separate from plugin boilerplate
- nih-plug is pinned to one revision in the root `Cargo.toml` (`[workspace.dependencies]`); bump it there for all plugins at once
