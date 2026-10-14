# Audio Plugin Suite

A suite of VST3/CLAP plugins built with Rust and the nih-plug framework. The brand is **HØJT** (Danish for "loud" and "high"). The vendor string is the ASCII form `Hojt Audio` and lives in one place (`crates/suite_common/src/lib.rs`).

## Layout

- `plugins/pitch_shifter/` - Real-time guitar pitch shifter (effect)
- `plugins/drums/` - Synthesized drum instrument (full kit, Rock/Jazz/Metal/Groove)
- `plugins/bass/` - Plucked string bass (instrument), picked or played with a finger
- `plugins/pad/` - Slow ambient pad with a chorus and a shimmering hall (instrument)
- `plugins/bells/` - Soft glassy bells with a ping-pong delay and a hall (instrument)
- `plugins/amp/` - Guitar amp simulator (effect). Its decisions, known problems and backlog are in `docs/amp-progress.md`
- `crates/suite_common/` - Shared code: `VENDOR` and `BRAND` constants, `ui` module (the pedal look, see Look below),
  `dsp` (filter, noise and the `Hall` reverb of the newer instruments), `measure` (what their tests measure with)
- `prototypes/jsfx/` - The REAPER scripts that Metal, Groove, bass, pad and bells were written as
  first: the reference for those ports, not built or shipped. Its `README.md` says where a port differs
- `crates/suite_common/assets/fonts/` - Embedded fonts (SIL OFL) with their licence files
- `xtask/` - nih-plug bundler
- `bundler.toml` - Bundle name per plugin package
- `site/` - The public page (plain HTML and CSS, no build step), published by GitHub Pages
- `site-sources/` - What is played in a sound example of the page when it is our own playing
  (DI recordings, MIDI drum parts); not published. See its `README.md`
- `scripts/package.sh` - Packs the bundles into the zips of a release
- `scripts/site-audio.sh` - Makes the page's sound examples from the render tests
- `.github/workflows/` - `ci.yml` (tests), `release.yml` (builds and publishes a release),
  `pages.yml` (publishes `site/`)

Each plugin keeps DSP in its own module(s), separate from `lib.rs` (plugin entry point, parameters, nih-plug integration) and `editor.rs` (egui GUI).

## Build Commands

```bash
# Build release VST3 and CLAP bundles (one plugin at a time)
cargo xtask bundle pitch_shifter --release
cargo xtask bundle drums --release
cargo xtask bundle amp --release
cargo xtask bundle bass --release
cargo xtask bundle pad --release
cargo xtask bundle bells --release

# Quick compile check
cargo check --workspace

# Run tests (always run before committing!)
cargo test --workspace

# Open a plugin's editor as a program, without a DAW (no audio: dummy backend)
cargo run -p pitch_shifter --features standalone -- --backend dummy
cargo run -p drums --features standalone -- --backend dummy
cargo run -p amp --features standalone -- --backend dummy
cargo run -p bass --features standalone -- --backend dummy
cargo run -p pad --features standalone -- --backend dummy
cargo run -p bells --features standalone -- --backend dummy
```

Output location: `target/bundled/<BundleName>.vst3`

## Install

Copy VST3 to system folder (requires admin):
```
xcopy /E /I /Y "target\bundled\HojtPitchShifter.vst3" "C:\Program Files\Common Files\VST3\HojtPitchShifter.vst3"
xcopy /E /I /Y "target\bundled\HojtDrumSynth.vst3" "C:\Program Files\Common Files\VST3\HojtDrumSynth.vst3"
xcopy /E /I /Y "target\bundled\HojtGuitarAmp.vst3" "C:\Program Files\Common Files\VST3\HojtGuitarAmp.vst3"
xcopy /E /I /Y "target\bundled\HojtBassSynth.vst3" "C:\Program Files\Common Files\VST3\HojtBassSynth.vst3"
xcopy /E /I /Y "target\bundled\HojtShimmerPad.vst3" "C:\Program Files\Common Files\VST3\HojtShimmerPad.vst3"
xcopy /E /I /Y "target\bundled\HojtGlassBells.vst3" "C:\Program Files\Common Files\VST3\HojtGlassBells.vst3"
```

## Look

Every plugin is drawn as a painted stompbox. All of it is vector, drawn with egui's painter in
`crates/suite_common/src/ui/`; there are no image files.

- `theme.rs` - Colours, the three embedded fonts (`install`), text helpers. One paint colour per plugin,
  one per mini pedal of a rig (`PEDAL_PAINTS`), and `shade` for darker tones of a paint
- `widgets.rs` - `param_knob` (fluted knob, name, value on label tape), `footswitch`, `led`,
  `small_footswitch`, `rig_led` and `param_switch` (the switch and LED of a rig, bound to an on/off
  parameter), `stepper` (label tape with a chrome button at each end, for picking one of many),
  `tuner_display` (note name and a row of lamps on one strip of label tape) and `cents_meter`
  (the lamps alone), `signal_lamp` (a small lamp with no switch, for what the sound is doing),
  `silk_frame`, `silk_label`, and `badge` (the HØJT logo: the Ø is a knob, its slash the pointer)
- `frame.rs` - `pedal`: enclosure, screws, jack captions, tilted name band, model number, logo.
  A plugin describes itself with a `PedalStyle`. For a window with several enclosures: `rig`
  (the bare bench), `head` (wide enclosure with everything a pedal has printed on it) and
  `mini_pedal` (small enclosure with only a title), each mini pedal described by a `MiniStyle`
- `art.rs` - `Motif`: what is painted on a mini pedal under its controls

Controls are placed at fixed positions relative to the window's top left corner. Knobs: drag up and
down, Shift for fine steps, double-click for the default.

### Style guide

Every plugin, existing and new, follows these. Copy `plugins/pitch_shifter/src/editor.rs` (few
controls, portrait) or `plugins/drums/src/editor.rs` (many controls, landscape) as the starting point.

- **One pedal per plugin.** The whole window is one enclosure from `ui::pedal`. No panels, tabs,
  menus or default egui widgets on top of it
- **Paint:** each plugin has its own saturated paint colour, a `PAINT_*` constant in `theme.rs`.
  Taken: orange (Pitch Shifter), teal (Drum Synth), oxblood red (Guitar Amp), cobalt (Bass Synth),
  plum (Shimmer Pad), pine green (Glass Bells). A new one must be clearly different from those
  and dark enough for cream print to read on it; it goes in `PLUGIN_PAINTS`, where a test holds it to both
- **Print on the paint is always `SILK` (cream).** Black is only for knobs, label tape and the logo plate.
  The one other thing on paint is the second coat of a mini pedal's art, in `shade`s of its own paint.
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
- **Model number:** two letters from the name and a number, like `PS-10`, `DS-2` and `BA-1`.
  It is print, not the version, and does not change with releases
- **Jack captions** say what really goes in and out (`In`, `MIDI In`, `Stereo Out`)
- **Window size:** fixed, sized to the controls. Portrait about 400 wide for up to four controls,
  landscape about 660 wide for more
- **Clean factory paint:** no wear, textures, gradients or images
- **New shared pieces go in `suite_common::ui`,** not in a plugin's editor, so every plugin gets them

- **Rigs:** a plugin made of several components (the Guitar Amp) is the one exception to one
  pedal per plugin. Its window is a `rig`: one `head` across the top, which carries the name band,
  model number and logo, and from left to right in signal order a row of `mini_pedal`s below it.
  The head has the plugin's paint; each mini pedal has a paint of its own. On a head and on mini pedals the switches are
  `small_footswitch` and the LEDs `rig_led` (an unlit `led` disappears on dark paint), beside or
  above the switch. A mini pedal holds two knobs of radius 24 side by side or three of radius 17
  in a triangle. Everything else in this guide applies
- **Pedal art:** every mini pedal has its own `PAINT_*` and its own `Motif`, all on one theme
  per rig (the Guitar Amp: northern weather and nature). Two coats and a print: a large motif
  in darker tones of the pedal's paint (`theme::shade`) that may run under the controls, with
  no fine detail under a caption, and a few `SILK` lines (2 px) and dots only in the clear
  paint beside the controls. The art keeps 5 px of plain paint to the edge of the face. No
  letters, numbers or cream triangles in it, and nothing that moves. A new paint goes in
  `PEDAL_PAINTS`, where tests hold it to cream print reading on it and to being clearly apart
  from the others; a new motif is drawn in `art.rs` for the layout of its pedal
- **Slim pedals:** a pedal of a rig with two knobs can be a `slim_pedal`, 110 px wide: the
  LED where a trio's upper left knob is, the two radius-17 knobs upper right and lower left
  (`SLIM_LED`, `SLIM_KNOBS`), the switch where every pedal has it. Its title is size 21, so
  seven letters fit. In a row with wide pedals the LEDs and switches line up
- **A board that can be rearranged:** a pedal is moved by dragging it by its title
  (`title_grip`); while it is held it is drawn `lifted` and last, the others glide to make
  room, and it lands in the nearest place it may have. No arrows, menus or lists. A pedal
  that cannot be moved has no grip
- **The amp's place in the row** is the one thing drawn on the bench: `amp_marker`, two patch
  cables (`CABLE`) from the pedals on either side up to the head with a cream tag reading
  `AMP`, in a 40 px gap. Objects on the bench, not print: silk triangles still mean signal
  direction at the jacks only
- **An order is never a parameter.** Keep it in a `#[persist]` field, as ids, like a preset
  or a file choice
- **Signal lamps:** a light that follows the sound and not a switch (gate open, output clipping)
  is a `signal_lamp`, with a `silk_label` of size 13 centred 18 px under it, placed beside the
  control it belongs to. An LED above or beside a switch only ever says that the switch is on.
  Lamps are read from atomics the audio thread writes once per block; the editor asks for a
  repaint only when one changes, 20 times a second at most
- **Pick one of many (presets):** a `stepper` with a `silk_label` caption above it, top centre of
  the head on the jack-caption line, at least 12 px clear of the knob ticks. No dropdowns, lists
  or pop-ups. Step arrows are engraved in chrome buttons; silk triangles on the paint mean signal
  direction at the jacks only. Names have at most 14 characters and are our own words. A trailing
  `*` on the tape means a control was moved since loading
- **A preset choice is never a parameter.** The editor sets the real parameters through the
  `ParamSetter` and keeps only the index in a `#[persist]` field
- **Pick a file (cabinets):** files are never picked in a dialog. They live in one folder and are
  stepped through in name order with a `stepper` under a `silk_label` caption, the built-in choice
  first (`OWN`). On a mini pedal the stepper takes the place of the third knob of a trio: two
  radius-17 knobs with the LED between them, caption at y+174, tape 100 px wide at y+198. The
  tape shows at most 10 characters: no extension, capitals, long names as the first 5 and last 4
  with `~` between. `BAD FILE` for a file that cannot be used, `?` in front of the name of one
  that is not there
- **A file choice is never a parameter.** Keep the file name (not the path) in a `#[persist]` field
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

Fully synthesized drums: no sample files, nothing loaded from disk. MIDI in, stereo out, General MIDI drum map. Four kits: Rock, Jazz, Metal, Groove.

Rock and Jazz are two presets of one instrument, the modal voices. Metal and Groove are
instruments of their own (`src/engines/`), ported from `prototypes/jsfx`.

- `src/lib.rs` - Plugin entry point, parameters, MIDI events
- `src/drum_kit.rs` - The instrument: owns all voices, note map, stereo mix, room send, output limiter
- `src/kit.rs` - `Kit` enum and one `KitPreset` for Rock and for Jazz: all the constants that make those two differ
- `src/engines/` - `metal.rs` and `groove.rs`: the Metal and the Groove kit, each whole (voices,
  room, bus); `mod.rs` has what they share (`MetalCluster`, `CombRoom`, `BusDrive`, `Rest`)
- `src/voices/` - `kick.rs`, `snare.rs`, `tom.rs`, `cymbal.rs`, and `mod.rs` (`Hit`, `Voice`, `VoicePair`)
- `src/dsp.rs` - Shared building blocks: noise, filters, membrane modes, clippers
- `src/reverb.rs` - Room: 8-line feedback delay network
- `src/editor.rs` - egui GUI: teal pedal, knobs, one footswitch per kit

How the modal voices (Rock, Jazz) work:

1. Kick: sine body with a pitch sweep, one shell mode, band-passed noise click for the beater, saturation
2. Snare and toms: inharmonic membrane modes (`dsp::Modes`) sharing a pitch bend, plus noise for the
   snare wires and the stick. The snare's head and wires go through one saturation (`SnarePreset.drive`)
   in front of the hit's level, so a soft hit is as dense as a hard one. The side stick is the snare
   voice with short, high modes and no wires
3. Cymbals: a bank of up to 256 two-pole resonators at inharmonic frequencies, spread evenly
   from `low_hz` to `high_hz`, plus a wash of noise through six bands that decay separately
   (higher ones faster). The modes are the body and pitch and stop at a few kHz: a mode rings
   for a second at a bandwidth of a hertz, so it never fuses with its neighbours however
   close, and a comb of them up high is heard as a bell that beats (chorus). The sizzle above
   them is the wash. Modes under `body_hz` are turned down, or the tail is a low hum. A hit
   adds a sine of random phase to each resonator and places each mode anew in its slot, so
   repeated hits build up instead of restarting and never ring the same tones. One `Cymbal`
   serves closed, pedal and open hi-hat: the closed presets have a short decay, which is
   what chokes the open sound
4. Velocity changes the sound, not only the level: pitch bend depth, overtone level, noise brightness
5. Every hit varies slightly in pitch and overtone levels (each voice has its own noise generator)
6. Drums have two voices each (`VoicePair`): a new hit takes the idle one and the previous fades in 3 ms
7. A kit change, Tune and Damping apply to the next hits. Ringing voices keep their sound
8. `dsp::bus_clip` on the output leaves single hits untouched and rounds off the peaks of busy patterns

How Metal and Groove work:

- One voice per piece: a new hit starts the piece again. Kick, snare and toms are sines with a
  pitch drop and filtered noise; hi-hats and cymbals are six square waves at inharmonic ratios
  (`MetalCluster`) and noise through filters; every noise part of a kit hears the same noise
- Each has its own room (`CombRoom`: combs and all-passes) and a tanh on its bus. Metal's bus is
  driven at 35 %; Groove's is a tape at 20 % with a low-pass behind it (Warmth 40 %)
- They take the same controls as Rock and Jazz. Tune is on kick, snare, side stick and toms;
  Damping scales every decay time; both are taken at the hit. Room at 25 % is the kit's own
  default room (16 % of its full level for Metal, 22 % for Groove)
- The kit's balance is in its trims (`*_TRIM_DB`), its level against the other kits in `LEVEL`:
  kick and snare as loud as Rock's, and peaks that never reach `bus_clip`'s knee
- Groove has no china (52 plays its crash) and one crash for 49, 57 and, cut short, 55. Its
  shaker follows the Hi-Hat level
- An engine that has been silent for 0.1 s rests and costs nothing. One that is ringing when
  the kit changes rings on
- Metal's splash borrows the hi-hat's cluster, which then runs twice as fast while both
  sound. That is the script's sound; leave it

| MIDI note | Voice | MIDI note | Voice |
|-----------|-------|-----------|-------|
| 35, 36 | Kick | 42 / 44 / 46 | Hi-hat closed / pedal / open |
| 38, 40 | Snare | 49, 57 | Crash 1, Crash 2 |
| 37 | Side stick | 51, 59 / 53 | Ride / Ride bell |
| 41, 43, 45, 47, 48, 50 | Toms, low to high | 52, 55 | China, Splash |
| | | 69, 70, 82 | Shaker (Groove only) |

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| Kit | Rock / Jazz / Metal / Groove | Rock | Sound of every piece and of the room. New kits go at the end: a project remembers its kit by its number |
| Gain | -30 to +6 dB | -6 dB | Output level |
| Room | 0-100 % | 25 % | Room reverb amount |
| Tune | -6 to +6 st | 0 | Pitch of kick, snare and toms |
| Damping | 0-100 % | 0 % | Shortens the decay of drums and cymbals |
| Kick, Snare, Toms, Hi-Hat, Cymbals | -30 to +6 dB | 0 dB | Level per group |

Stereo positions are from the drummer's seat (hi-hat left, floor tom right).

Not there yet: separate outputs per drum, hi-hat openness from CC4, cymbal choke on note-off.

### Bass Synth (`plugins/bass`)

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

### Shimmer Pad (`plugins/pad`)

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

### Glass Bells (`plugins/bells`)

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
|----------------|-------|---------|-------------|
| Volume (`volume`) | -40 to +12 dB | -8 dB | Output level |
| Brightness (`brightness`) | 0-100 % | 45 % | Glass in the strike |
| Decay (`decay`) | 0.3-10 s | 3 s | How long a bell rings |
| Attack (`attack`) | 0.5-50 ms | 6 ms | Fade in of the strike |
| Dry (`dry`) | 0-100 % | 55 % | Level of the bells themselves |
| Delay (`delay_on`) | on / off | on | |
| Delay Time, Feedback, Mix (`delay_time`, `delay_feedback`, `delay_mix`) | 50-1500 ms, 0-90 %, 0-100 % | 445 ms, 45 %, 35 % | |
| Reverb (`reverb_on`) | on / off | on | |
| Reverb Decay, Mix (`reverb_decay`, `reverb_mix`) | 0.5-20 s, 0-100 % | 6 s, 50 % | |

### Guitar Amp (`plugins/amp`)

`docs/amp-progress.md` has the plan, the decisions, the known
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
- `process` returns `KeepAlive` while a delay or reverb tail rings
- Tuner on gives the amp silence and turns its output down in 10 ms (exactly silent, tails
  included); off brings it back as fast. Bypass still passes the input while tuning. The tuner
  costs nothing when off and under 3 us in its worst 64-sample block when on
- The tuner reads 48 Hz to 1420 Hz against A = 440 Hz (`REFERENCE_A_HZ`), a new reading every
  30 ms, none for silence, noise or chords. A loaded project never opens with the tuner on
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
| Gate Threshold (`gate_thresh`) | -80 to -20 dB | -60 dB | Input level that opens the gate (closes 6 dB lower) |
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

Each plugin's `ids` test holds these, with the keys of its `#[persist]` fields, against its
`ids.txt` (`suite_common::ids`). A plugin that gains a parameter: run
`UPDATE_IDS=1 cargo test -p <name> ids` and commit `ids.txt`. Never do that for an id that changed.

The version number lives in `Cargo.toml` (`[workspace.package]`) and is what hosts show.

Only to compare two builds side by side in the DAW: temporarily give one of them another bundle
name, `NAME`, `VST3_CLASS_ID` and `CLAP_ID`, and do not commit that.

## Releasing

The page is https://hojte.net/hojt-audio/ and the downloads are GitHub releases.
The code is GPL-3.0-or-later, which the VST3 bindings require of a VST3 binary that is handed
out; the HØJT name and logo are kept out of it (`NOTICE.md`).

1. Set the version in `Cargo.toml` (`[workspace.package]`) and run `cargo check --workspace` so
   `Cargo.lock` follows
2. Add a `## <version>` section to `CHANGELOG.md`: it becomes the text of the release
3. For every plugin whose sound changed in that section, run `scripts/site-audio.sh <plugin>`
   (`drums`, `amp`, `pitch_shifter`, `bass`, `pad`, `bells`) and listen to what changed in `site/audio/`. Redo a
   screenshot in `site/img/` if the plugin's look changed
4. Commit, then tag `v<version>` and push the tag. `release.yml` tests, builds for Windows,
   macOS (Intel and Apple Silicon in one binary, signed ad hoc) and Linux, and publishes the zips

- The zips are named `<BundleName>-<windows|macos|linux>.zip`, without a version: the page
  links to `releases/latest/download/<name>`, so these names never change
- To see the zips without releasing, run the Release workflow by hand (Actions tab) and take
  them from the run's artifacts
- `Cargo.lock` is committed so a release is built from the crates that were tested
- The page uses the plugins' colours (`ui/theme.rs`) and fonts (copies in `site/fonts/`). Body
  text never sits on the paint: cream on orange or teal is too weak at that size
- The page is published from `main` and the downloads are the latest release, so
  `site/audio/` and `site/img/` only change in the commit of a release: a branch that changes
  a sound leaves them alone, or the page plays what nobody can download yet
- Sound examples in `site/audio/` are MP3s (`ffmpeg -codec:a libmp3lame -q:a 4`) of files from
  the `render_*` tests, never made by hand. The table in `scripts/site-audio.sh` says which
  render each one is. An example that has not changed comes out the same to the byte
- An example can be our own playing through the plugin: the recording or MIDI file goes in
  `site-sources/` and its name in that table. The sound is still made by the script
- Screenshots in `site/img/` are the standalone windows at their own size, taken by hand

## Testing

**Always run `cargo test --workspace` before committing changes.** CI also runs
`cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`; run them too.
Lints that clash with DSP code are allowed in the root `Cargo.toml`, each with its reason.

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
placement and the room. The tests of an engine sit in its file and pin it to the numbers of its
script (pitches, decay times, the room at the default setting).

Two ignored tests are tools for working on the drum sound:

```bash
# Table per piece, kit and velocity: peak, level, level of the first 50 ms, level over 4 kHz, decay
# time, brightness; for cymbals the low hum in the tail and how evenly 4 to 12 kHz is filled
# (1 is noise, 0 a few tones). Last line: cost of every cymbal ringing. Compare before/after a change.
cargo test -p drums --release kit_report -- --ignored --nocapture

# WAV files in target/renders for listening: per kit, every piece at three velocities and a groove.
# Optional: DRUMS_INPUT_MID=<MIDI file> renders that drum part through every kit as well
cargo test -p drums --release render_wavs -- --ignored
```

Bass, pad and bells have their tests next to the code too (`bass.rs`, `pad.rs`, `bells.rs`):
tuning at several rates, envelopes, voice stealing, the effects and their switches, bounded
output with everything up. Each has the same two tools:

```bash
# Level, tuning and brightness per note, and the cost
cargo test -p bass --release bass_report -- --ignored --nocapture
cargo test -p pad --release pad_report -- --ignored --nocapture
cargo test -p bells --release bells_report -- --ignored --nocapture

# WAV files in target/renders: bass_metal.wav, bass_groove.wav, pad.wav, bells.wav
cargo test -p bass --release render_wavs -- --ignored
cargo test -p pad --release render_wavs -- --ignored
cargo test -p bells --release render_wavs -- --ignored
```

## Code Conventions

- Always run tests before committing
- Optimize for low latency over audio quality
- Never allocate or resize buffers on the audio thread (all buffers are fixed-size)
- Measure pitch shifter DSP changes with `quality_report` before judging them by ear
- Rock and Jazz are tuned in `plugins/drums/src/kit.rs`, Metal and Groove in their engine's file;
  check `kit_report` and listen to `render_wavs`
- Keep DSP code separate from plugin boilerplate
- nih-plug is pinned to one revision in the root `Cargo.toml` (`[workspace.dependencies]`); bump it there for all plugins at once
