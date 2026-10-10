# Guitar Amp: progress

Working file for the amp simulator in `plugins/amp/`. Branch `amp-sim`, never merged to `main`
by the agent. A new session continues from this file and the branch alone: read it top to
bottom, run the commands under "How to resume", then carry on with "Next".

## How to resume

```bash
git checkout amp-sim
cargo test --workspace
cargo test -p amp --release amp_report -- --ignored --nocapture
cargo run -p amp --features standalone -- --backend dummy
```

Way of working (asked for by Rasmus on 2026-10-10): the session is the orchestrator. Per
milestone: commit the contract (parameters, public signatures), hand DSP and editor work to
subagents in worktrees, review, merge, `cargo test --workspace`, run `amp_report` before and
after DSP changes, look at the standalone window, update this file, commit, push. Do not stop
between milestones or after the last one; continue with the backlog until stopped. Only stop
for something destructive or something that truly needs Rasmus.

## Status

| Milestone | State |
|---|---|
| M1 One playable amp (Brøl) | done 2026-10-10 |
| M2 Three amps | done 2026-10-10 |
| M3 Gate, drive, pedalboard | done 2026-10-10 |
| M4 Delay and reverb | done 2026-10-10 |
| M5 Tuning and cost | done 2026-10-10 |
| Backlog 1: factory presets | done 2026-10-10 |
| Backlog 3: tuner | done 2026-10-10 |
| Backlog 2: user IR loader | done 2026-10-10 |

## Next

The planned milestones, the presets, the tuner and the IR loader are done. Continue with the
backlog at the end of this file. Next up: more amps and pedals (backlog 4). Adding an amp
means a new `Amp` variant with its own `#[id]` appended after `torden` (never reorder), a
model in `amp/model.rs`, a place for a fourth switch on the head (the bottom row is full:
this needs a layout decision, for example the amp choice as a `stepper`), presets for it.
Adding a pedal means a sixth slot on the board (the window is 960 wide with five pedals of
180: a wider window or a second row).

Before more DSP: nobody has listened to anything yet. If Rasmus has listened and left notes,
they come first.

Quick wins by the numbers:
- Torden with the drive at 10 / 10 / 10 at 44.1 kHz aliases at -67.6 dB (target -70)
- Gate thresholds in the metal presets (-46 to -50 dB) may chop quiet playing
- A 40 ms user cabinet at 192 kHz takes the plugin from 7.2 % to 15.7 % of a core. Capping
  taps instead of milliseconds would halve that but change the low end per rate

## What was decided with Rasmus

| Question | Answer |
|---|---|
| High gain target | Modern tight metal: tight low end, fast attack, clear in drop tunings, meant to be boosted by the drive |
| Clean target | Glassy, lots of headroom, slightly scooped |
| Crunch target | Mid-forward stack crunch, cleans up with picking dynamics |
| Cabinets | Our own impulse responses, generated in code at startup. No files. IR loader on the backlog |
| Chain | Fixed order, on/off per component, amp picked with a three-way selector |
| Presets | Good defaults now. Preset browser on the backlog |
| Look | Amp head over a pedalboard, one window |
| Paint | Oxblood red, one colour for head and pedals |

No real makers' names, trademarks, graphics or circuit names anywhere: code, comments, docs,
commit messages.

## Identity (frozen)

- Package `amp`, bundle `HojtGuitarAmp`, `NAME = "Hojt Guitar Amp"`
- `CLAP_ID = "com.guitarpitchshifter.guitar-amp"`, `VST3_CLASS_ID = *b"HojtGuitarAmp003"`
- Name band `Guitar Amp`, model `GA-3`
- Amps: **Klar** (clean, KL-30), **Brøl** (crunch, BR-50), **Torden** (high gain, TD-100)

Parameter ids. A parameter is added in the milestone that makes it do something. Its id comes
from this table and never changes.

| Group | ids | Added |
|---|---|---|
| Global | `bypass`, `out_level` | M1 |
| Global | `in_gain` | M3 |
| Global | `tuner` (not automatable, not in presets) | tuner |
| Gate | `gate_on`, `gate_thresh`, `gate_release` | M3 |
| Drive | `drive_on`, `drive_gain`, `drive_tone`, `drive_level` | M3 |
| Amp | `gain`, `bass`, `mid`, `treble`, `presence`, `master` | M1 |
| Amp | `amp` (enum, variant ids `klar`, `brol`, `torden`) | M2 |
| Cab | `cab_on`, `cab_mic`, `cab_res` | M3 |
| Delay | `delay_on`, `delay_time`, `delay_feedback`, `delay_mix` | M4 |
| Reverb | `reverb_on`, `reverb_decay`, `reverb_mix` | M4 |

## Architecture

Signal flow: input summed to mono, input gain, gate, then one oversampled region (drive,
preamp, tone stack, power amp), cabinet, stereo delay and reverb, output level, safety clip.

- `lib.rs` reads the parameters once per block into `chain::AmpSettings` and calls
  `AmpChain::process`. The chain smooths the settings itself
- `chain.rs` `AmpChain`: owns every stage, fixed chunks so scratch buffers are fixed arrays
- `dsp/` filters, oversampler (polyphase IIR half-bands, minimum phase), shapers
- `amp/model.rs` one `AmpModel` of constants per amp; `preamp.rs`, `tonestack.rs`, `poweramp.rs`
- `cab.rs` IR designed at `set_sample_rate` from a filter cascade plus fixed-seed resonances,
  run as a direct FIR (zero latency)
- Later: `gate.rs`, `drive.rs`, `delay.rs`, `reverb.rs`
- Editor: `suite_common::ui` gets `rig`, `head`, `mini_pedal`; window 960 x 340 in M1 and M2,
  960 x 660 from M3

## Decisions log

- 2026-10-10 The six amp dials are shared by the three amps and shown as 0.0 to 10.0. Simple
  parameters that automate; per-amp knob memory is on the backlog
- 2026-10-10 Amp and cabinet are mono (stereo input is averaged); delay and reverb make stereo
- 2026-10-10 Minimum-phase IIR oversampling and a direct FIR cabinet: latency target under
  1 ms, nothing reported to the host
- 2026-10-10 Parameters are not smoothed by nih-plug. `AmpChain` smooths per chunk, so tests
  that drive the chain directly behave like the plugin
- 2026-10-10 Drums DSP is left untouched; the amp has its own filters and reverb. Moving shared
  filters to `suite_common::dsp` is on the backlog

- 2026-10-10 M1: the clippers are antialiased (first-order antiderivative, in f64) on top of
  4x oversampling. Plain 4x measured about -40 dB aliasing at full gain; this gives -87 dB or better
- 2026-10-10 M1: biquads are f64. At 4 x 192 kHz a 100 Hz filter is too close to the unit
  circle for f32 coefficients
- 2026-10-10 M1: denormals are kept away by a 1e-20 constant in every feedback path, since
  tests do not run with flush-to-zero
- 2026-10-10 M1: the cabinet IR is normalised to 0 dB average power from 100 Hz to 4 kHz, so
  its gain does not depend on the sample rate
- 2026-10-10 M1: dials are read every 32 samples counted across host blocks, so the output
  does not depend on the host's block size
- 2026-10-10 M1: the head is 320 px tall and uses 50 px `small_footswitch`es with the LED
  beside them. A full-size footswitch with an LED above does not fit under the band
- 2026-10-10 M1: the band on a head tilts -1.5 degrees (pedals keep -4): over 940 px the
  pedal's tilt would rise more than the band is tall
- 2026-10-10 M2: amp switching fades the amp out over 5 ms in front of the cabinet, switches at
  the silent sample, fades in; the cabinet crossfades its IR over 10 ms from that point. Two
  amps never run side by side (it would double the oversampled cost)
- 2026-10-10 M2: `AmpModel` got `focus` (a bell in front of the clipping), `fizz_hz` and
  `lowcut_hz` (after the last stage) for Torden. Unused ones are exact pass-throughs
- 2026-10-10 M2: a preamp stage takes its operating point from the previous sample, which
  breaks the chain of divisions and saved about a quarter of the preamp's time
- 2026-10-10 M2: amp levels are matched by RMS at dials 5 (within 0.5 dB). Klar will be heard
  as quieter because it is not compressed
- 2026-10-10 M2: the selector and the On switch on the head have their LED above their caption,
  to the left of the switch
- 2026-10-10 M3: gate is a hard gate with 6 dB hysteresis, 0.5 ms opening ramp, 40 ms hold, no
  lookahead; its detector reads the input before the input gain. An expander was not needed
- 2026-10-10 M3: drive pedal sits in the oversampled region before the preamp: 720 Hz
  high-pass into a soft clipper with the dry signal added back underneath, Tone 1.5 to 7 kHz,
  Level -20 to +20 dB. Off costs nothing
- 2026-10-10 M3: second-order antialiasing on Torden's second and third stages and on the
  drive only (`StageModel.second_order`). First order gave -67 dB with the drive in front;
  second order everywhere cost twice as much for the same figures. 8x was not measured
- 2026-10-10 M3: Brøl's stage gains at Gain 0 were lowered and `AmpModel.makeup_db` makes the
  level up after the power stage, so Gain 0 is clean on pick attacks
- 2026-10-10 M3: Cab Mic is a shelf pair and Cab Res a bell after the IR, skipped at 5.0 so
  the default is bit-identical; cab off passes the amp at -1.4 dB
- 2026-10-10 M3: pedals are 180 x 310; three-knob pedals use radius 17 in a triangle, two-knob
  pedals radius 24; the LED sits under the title. `ui::param_switch` draws LED and switch
- 2026-10-10 M4: delay and reverb are additive: the dry signal is never scaled, so mix 0 and
  off are bit-transparent. The delay's two lines feed each other, so the left/right offset
  does not grow with each repeat. The reverb is an 8-line network with Hadamard mixing
- 2026-10-10 M4: effect settings are latched at the 32-sample tick, not smoothed in the chain;
  the modules smooth themselves
- 2026-10-10 M4: `KeepAlive` while `!chain.is_idle()`, else `Normal`. `Tail(n)` would need
  the tail length ahead of time, and gains nothing in CLAP
- 2026-10-10 M4: rigs use `ui::rig_led` (black socket, darker unlit lens); `ui::led` and the
  other two plugins are unchanged
- 2026-10-10 M5: oversampling is 4x at 44.1 and 48 kHz and 2x (the steep half-band alone)
  from 88.2 kHz on. Aliasing at 96 kHz equals 48 kHz; the sound matches across rates
- 2026-10-10 M5: the speaker resonance is a biquad behind the convolution, fitted to the old
  20 ms response; the rest fits in 5 ms (Klar), 10 ms (Brøl), 6 ms (Torden), within 0.4 dB
  from 60 Hz to 8 kHz
- 2026-10-10 M5: the preamp runs one stage over the whole block, then the next (same output
  to the bit, a third less time)
- 2026-10-10 M5: Klar is 0.8 to 1.4 dB quieter below Gain 10 for room under the safety clip;
  at default dials it is 1.1 dB under the loudest amp
- 2026-10-10 M5: delay and reverb go idle at -100 dBFS, and the delay as soon as its read
  positions have only zeros ahead
- 2026-10-10 M5: Tone, Presence and cabinet dial filters glide sample by sample (zipper
  -77 dB or less in a 50 ms sweep)
- 2026-10-10 M5: `lib.rs` takes defaults and ranges from the chain's settings; a test pins them
- 2026-10-10 Presets: a preset holds everything except Bypass, Input and Output (the player's
  gain staging). The choice is not a parameter: the editor sets the parameters and persists
  the index. Twelve presets, Danish names. The picker is a `stepper` top centre of the head;
  the head grew 34 px (window 960 x 694, persist key `editor-state-rig2`)
- 2026-10-10 Tuner: muting is the behaviour. The amp's input (after the tuner tap) and its
  output both fade in 10 ms, so tails are silent and tuned strings do not end up in the delay
- 2026-10-10 Tuner: `Plugin::filter_state` drops `tuner` from a loaded state, so a project
  saved while tuning does not open muted
- 2026-10-10 Tuner: the detector works at about 5.5 kHz: normalised difference function for
  the rough period, harmonic check against octave errors and chords, then the phase advance
  of the first three harmonics for the exact pitch (interpolating the difference function
  cannot reach a cent on high notes at that rate). Median of three readings is shown
- 2026-10-10 Tuner: the amp switches moved 40 px left (x 255 / 385 / 515) so the Tuner switch
  at x 680 does not read as a fourth amp
- 2026-10-10 Klar presets trimmed back to level with Master (Glas 5.0, Varm 4.8, Tåge 4.9)
- 2026-10-10 IR loader: no file dialog (the style guide forbids pop-ups, and it would be a
  new GUI dependency). WAV files in `Documents/Hojt Audio/Cabinets` are stepped through on
  the Cab pedal. The folder is created on the first step, never by loading the plugin.
  `HOJT_CABINETS_DIR` overrides the folder
- 2026-10-10 IR loader: `hound` moved from a test dependency to a runtime one. Documents is
  found through the Windows known-folder call by a hand-written FFI declaration (follows
  OneDrive), no crate
- 2026-10-10 IR loader: user IRs run in the same direct FIR (zero latency), cut to 40 ms at
  every rate so the sound is the same everywhere, trimmed so the sound arrives within 0.2 ms,
  levelled like the built-in ones. The amp's speaker-resonance filter is not applied to them
- 2026-10-10 IR loader: taps reach the audio thread through one pre-allocated buffer with an
  atomic state (empty, writing, ready, taking); the audio thread copies them into the
  cabinet's free slot when no crossfade runs
- 2026-10-10 IR loader: the choice is a persisted file name, not a parameter, not in presets,
  and stays when the amp is switched
- 2026-10-10 Worktrees for subagents start from `main`, not from `amp-sim`. Tell each agent to
  run `git merge --ff-only amp-sim` first

## Known problems

- Nobody has listened yet. Everything is tuned by the report. The first listen to
  `target/renders/amp_*.wav` may ask for another voicing (cab bite, low end, gain at 5)
- Brøl at Gain 0 is not fully clean on hard pick attacks (the level table pushes transients
  into the power stage)
- Klar: chord peaks reach -3.0 dBFS at Gain 0, just under the output clip's knee; its low end
  is the fullest of the three and may boom with a neck pickup; its sparkle is EQ only
- Torden: aliasing at Gain 10 is -74 dB, 4 dB inside the -70 dB target; the drive pedal in
  front will eat into that. Next step is a second-order antiderivative in `AsymClipper`
  (about +0.4 % CPU), then 8x
- Torden: Gain 5 to 10 changes little (already fully squeezed at 5); the upper half of the
  dial could change character more
- Torden is only 2.5 dB tighter than Brøl below 100 Hz relative to the mids on palm mutes
- Amp switching leaves a dip of about 10 ms; not listened to (`amp_switching.wav`)
- The no-denormal-slowdown test is a timing assertion (silent under 2x playing); stable so far
- The standalone window was looked at as a capture of the window; the plugin has not been
  loaded in a DAW

- M3: the 5 dB tightness target (Torden with the drive against Brøl) is met by only 0.6 and
  1.1 dB, and Torden leaves little fundamental on single low notes (low E -37.5 dB of the
  whole signal, -39.0 with the drive)
- M3: hiss between the gate's closing and opening levels keeps it open after a note; the
  threshold has to sit 6 dB above the noise peaks
- M3: the output safety clip runs at base rate. Klar with Drive 10 and Level 10 hits it and
  aliases at -49 dB
- M3: Brøl at Gain 0 now peaks at -3.1 dBFS on chords, just under the output clip's knee
- Delay and reverb (not merged yet): the reverb blooms late (loudest around 95 ms); the
  delay's odd repeats lose 2.8 dB in mono; neither limits its own output; the delay at
  1000 ms and feedback 0.9 takes 76 s to go idle

M3 figures (2026-10-10, 48 kHz): Torden Gain 10 aliasing -91.7 / -90.4 dB, with the drive
(Drive 3, Level 8) -91.5 / -79.5 dB; Brøl Gain 0 THD -45.0 dB; closed gate is exactly silent;
worst latency 0.317 ms (Torden, 44.1 kHz, everything on). Run `amp_report` for the full tables.

- M4: bypass cuts the delay and reverb tails (10 ms fade) and resets them
- M4: a mono host layout gets the left effect channel only (the sum combs: up to -6.7 dB in
  one octave)
- M4: effects at their maximum push 2 to 11 dB over the safety clip's knee; the clip is not
  antialiased (an antialiased one costs half a sample and rolls off the top even when idle)
- M4: with the gate off and a hissy amp the effects never go idle, so the status stays KeepAlive
- M4: VST3 hosts that read the tail length only once at load see 0
- M4: latency is unchanged by the effects (Klar 8, Brøl 10, Torden 14 samples at 48 kHz with
  every pedal on)

- M5: the agent doing it was cut off before its final report. Its six commits were complete
  and are verified by the tests and the reports, but its own account of what it left undone
  is missing
- Presets: nobody has heard them. Values come from the model constants and `preset_report`
- Presets: loading one is 24 separate parameter changes, so a host may need 24 undo steps;
  host undo does not restore the preset index (the display then shows the newer name with `*`)
- Presets: mouse clicks on the stepper, host undo, and saving and restoring the preset index
  in a DAW project were not exercised (stepping, wrap and the `*` were, through a temporary
  debug path)
- Presets: no direct pick from a list; reaching the far side is up to six clicks
- Projects saved before the preset picker open at the default window size (new persist key)

- Tuner: only run on synthetic signals. No guitar, no DAW; the path from detector to display
  was exercised in halves (atomic hand-off by tests, drawing by captures with fixed readings)
- Tuner: mains hum alone above -70 dBFS reads as a note; strong hum near a low string's
  fundamental can pull the reading. Octaves and root-plus-octave read as the root
- Tuner: low limit 48 Hz; readings stop below -70 dBFS; first reading on low A after 133 ms
- Tuner and Bypass both on: the dry signal is heard while tuning (bypass wins, by decision)
- Tuner: `non_automatable` and the state filter that keeps a project from opening muted were
  not checked in a real host

- User cabinets: nobody has heard one, and only the exported built-in cabinets and synthetic
  responses were loaded, no third-party IR files
- User cabinets: not exercised: mouse clicks on the stepper, saving and restoring the choice
  in a DAW project, the background executor in VST3/CLAP hosts, macOS/Linux folders, a real
  OneDrive-redirected Documents folder
- User cabinets: the tape holds 10 characters (`BRØL ~INET`); `BAD FILE` hides which file;
  nothing in the window says where the folder is; an empty folder just stays on `OWN`
- User cabinets: only the file name is stored, so a project on another machine needs the same
  file there; the host is not told the state changed, and host undo does not cover it
- User cabinets: stereo files use the left channel; 8-bit and 64-bit float are rejected;
  `initialize` reads the file synchronously (a slow disk delays activation)
- User cabinets: the built-in designs differ between sample rates by up to 3 dB at 8 kHz (in
  the roll-off, where the filters warp). Found while testing the resampler; below 4 kHz they
  agree within 0.16 dB
- Mic and Res use the selected amp's frequencies also under a user cabinet

## Last report

After M5 and the presets (2026-10-10). Run the reports for the full tables.

```
48 kHz, dials at 5 except Gain
amp      gain  chords RMS   chords pk  sine RMS   sine pk   THD dB  alias 1245  alias 4186
Klar      0.0       -18.8        -4.3     -20.8     -17.8    -53.6      -125.9      -122.8
Klar      5.0       -17.7        -4.4     -17.7     -14.7    -44.9      -125.4      -123.0
Klar     10.0       -16.1        -4.4     -14.7     -12.0    -21.7      -124.8      -122.5
Brøl      0.0       -19.7        -3.1     -21.4     -18.4    -44.9      -123.7      -124.7
Brøl      5.0       -16.6        -6.5     -15.0     -12.0    -15.5      -122.5      -121.8
Brøl     10.0       -15.6        -6.2     -14.4      -9.1     -8.2       -98.6       -86.7
Torden    0.0       -17.5        -7.0     -13.7     -10.0    -24.5      -125.1      -122.1
Torden    5.0       -17.0        -6.7     -13.2      -8.3    -18.0      -119.4      -111.7
Torden   10.0       -16.3        -5.7     -12.5      -6.0    -14.4       -91.5       -90.4

Aliasing per sample rate at Gain 10 (1245 Hz / 4186 Hz tone)
                                   44100             48000             96000            192000
Brøl 10.0                  -93.3 / -83.8     -98.6 / -86.7     -98.7 / -86.7   -118.5 / -117.6
Torden 10.0                -87.3 / -90.3     -91.5 / -90.4     -91.5 / -90.5   -117.3 / -118.4
Torden 10.0 drive          -87.4 / -77.8     -91.2 / -79.5     -91.3 / -79.5   -114.3 / -110.1
Torden 10.0 drive 10s      -75.8 / -67.6     -76.9 / -70.1     -77.0 / -70.1     -98.0 / -91.8

Latency with every pedal on, samples at 44.1 / 48 / 96 / 192 kHz
Klar 8 / 8 / 13 / 23, Brøl 10 / 10 / 17 / 31, Torden 14 / 14 / 25 / 48 (0.32 ms at most)

Time per 64-sample block, stereo, everything on (Torden, Gain 10), both effects
48 kHz 27.3 us (2.0 %), 96 kHz 17.2 us (2.6 %), 192 kHz 18.6 us (5.6 %)
Before M5: 48.5 us (3.6 %), 51.8 us (7.8 %), 58.6 us (17.6 %)

Time per stage at 48 kHz, ns per sample: preamp 206, power 65, drive 51, reverb 42,
cabinet 21, delay 16, tone 12, the rest under 6 each

The same chords at 44.1, 48, 96 and 192 kHz differ by at most 0.17 dB and 0.03 dB THD
Idle after the last note: 3.5 s at default effects, 63 s at delay 1000 ms / 90 %
```

## Backlog after M5 (top to bottom)

1. Factory preset browser (done)
2. User IR loader (done)
3. Tuner (done)
4. More amps and pedals (compressor, fuzz, chorus)
5. Mic choices per cabinet
6. Lower CPU (shorter or partitioned IRs, SIMD)
7. Input and output meters
8. Shared `suite_common::dsp` for filters used by more than one plugin
9. Per-amp knob memory
