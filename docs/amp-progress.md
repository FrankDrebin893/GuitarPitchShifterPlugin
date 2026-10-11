# Guitar Amp: progress

Working file for the amp simulator in `plugins/amp/`. Branch `amp-sim`, never merged to `main`
by the agent. A new session continues from this file and the branch alone: read it top to
bottom, run the commands under "How to resume", then carry on with "Next".

Merged to `main` on 2026-10-11 at Rasmus's request, after he had listened ("it sounds
fantastic"). The rule below about never merging held until then; new work can go on `main`
or a new branch as he says.

## Round 2 (branch `amp-round2`, started 2026-10-11)

Clean level, compressor, four mods, a pedal order the player sets. Never merged to `main` by
the agent. Same way of working as below; worktrees for subagents start from `main`, so each
runs `git merge --ff-only amp-round2` first. To resume: `git checkout amp-round2`,
`cargo test --workspace`, then the first milestone below that is not done.

Decided with Rasmus on 2026-10-11:

| Question | Answer |
|---|---|
| Mods | Chorus, Tremolo, Phaser, Flanger: Rate and Depth each |
| Compressor | A pedal (Sustain, Level), off by default |
| Order | Two zones, in front of the amp and behind the cabinet, free order inside each. Gate first, amp and cabinet one fixed block. Drive only in front; Delay and Reverb only behind; Comp and the mods in either |
| Board | One wide row (about 1560 x 694): the five pedals as they are, new ones slim (about 110 px) |
| Moving a pedal | Dragged by its title |
| Presets | Hold the order and the new pedals |
| Level reference | No DI file from Rasmus: DI levels from -36 to -12 dBFS RMS are swept |

| Milestone | State |
|---|---|
| M0 Loudness measure, `loudness_report`, pinned reports | done 2026-10-11 |
| M1 Clean level | done 2026-10-11 |
| M3 Compressor (`order.rs` with a constant order, `comp.rs`, first slim pedal) | sound done 2026-10-11: `comp.rs` in the chain, `comp_on`, `comp_sustain`, `comp_level`. No pedal on the board yet: the editor waits for the mock |
| M2 Board mock in the standalone (feature `board_mock`); needs Rasmus's yes before M3's editor | built 2026-10-11, waits for Rasmus |
| M4 Mods (`mods/`, four slim pedals, `mods_report`) | sound done 2026-10-11: in the chain with their twelve parameters. No pedals on the board yet |
| M5 Order (`chain-order` persisted, re-route with a fade, dragging, order in presets) | sound done 2026-10-11: the chain follows the saved order. Nothing in the window changes it yet; presets do not hold it yet |
| M6 New presets, docs | |

Planned parameter ids: `comp_on`, `comp_sustain`, `comp_level`, `chorus_on`, `chorus_rate`,
`chorus_depth`, `phaser_on`, `phaser_rate`, `phaser_depth`, `flanger_on`, `flanger_rate`,
`flanger_depth`, `trem_on`, `trem_rate`, `trem_depth`. The order is not a parameter: persisted
as `chain-order`.

### M2: the mock

`cargo run -p amp --features board_mock -- --backend dummy`. Without the feature nothing is
different. Captures in `target/mock`. All of it is in `board_mock.rs` (order, rules, dragging,
the five pedals that do nothing yet) and `suite_common::ui` (`slim_pedal`, `title_grip`,
`lifted`, `amp_marker`, `value_knob`, `value_switch`).

- Window 1590 x 694: five pedals of 180, five slim ones of 110, and 40 px of bench for the
  amp's place in front of the Cab. The head's controls are as they were, 315 px further right
- The amp's place: two patch cables from the pedals on both sides up to the head, with a
  cream tag `AMP` on them, printed in ink like the name band
- A slim pedal cannot have the LED above two knobs one over the other at the heights of the
  wide pedals: that is 37 px too tall. So the LED has the place of a trio's upper left knob,
  the first knob the upper right one, and the second is below to the left. Titles are size 21
  (24 on the wide ones) so `TREMOLO` and `FLANGER` fit in 110
- A pedal held where it may not be keeps the nearest place it may have, and the others show
  it. Nothing else says no

### What M0 found

Every test signal is at -18 dBFS RMS, a hot guitar, and there the amps are level (Klar is 1 to
2 LU louder than Torden). A clean amp follows the guitar dB for dB and one that clips hardly
does: 12 dB less guitar is 11 LU less from Klar, 4.5 from Brøl at Gain 5, 0.6 from Torden at
Gain 5. So with a guitar recorded at -30 dBFS RMS the clean presets are 9 to 11 LU under the
Torden ones (Glas -24.0, Stram -15.1 LUFS), at -24 still 3 to 5, and at -36 about 15. That is
what Rasmus heard. Brøl below Gain 3 (Kobber) behaves like Klar. Klar's peaks are already in
the output's safety clip at -18 (-0.2 dBFS), so more gain alone is no answer.

### M1: what was done

- The amps are matched by loudness (LUFS) for a guitar at -24 dBFS RMS (`REFERENCE_DI_DB`),
  not by RMS at -18. Klar got 5.8 dB more gain at the centre of its dials, all of it in front
  of its power stage (`power.drive_db` -10 to +1.5 at Master 5, `volume_db` -2.5 to -9,
  `makeup_db` to 0)
- Klar's power stage holds the peaks with a curve of its own (`PowerModel.clean_to` 0.65,
  `shaper::kneed_curve`): exactly straight up to 0.65 of the ceiling, then bending over. A
  first try with the old curve, which bends from the start, gave a held note at -24 dBFS RMS
  -36 dB of distortion; this one gives -61 dB, and -70 dB at -30 (cleaner than before M1)
- Brøl below Gain 5: `level_db` 6 / 3 to 23 / 9.7 and `makeup_db` 15 / 6.5 to 1.5 / 1.0, so
  3.5 and 1.2 dB louder and with the peaks held by the power stage, not the safety clip.
  From Gain 5 up the constants are untouched
- `PowerModel.driven_sag` changed its meaning: with the pedal on, the stage divides what it
  is given by one plus `driven_sag` times the load over `driven_sag_from` (Klar 40, 0.45). It
  used to lower the headroom, which made a stage that now clips clip harder: -60 dB of
  aliasing with the pedal at 10 / 10 / 10. Now -95 / -91 dB at 48 kHz
- Presets: Master of Glas 5.0 to 4.9, Tåge 4.9 to 4.6, Kobber 4.6 to 5.3. Every preset but
  Init is within 2 LU of the middle one at the reference level (a test)
- Checked with `fingerprint_report` against the build before: Brøl at Gain 5, 7.5 and 10 and
  Torden everywhere are the same to the bit, plain, with the drive and with the effects, at
  48 and 96 kHz. Klar everywhere and Brøl at Gain 0 and 2.5 changed
- Tests that held the old level at -18 dBFS were moved to the reference level: the clean
  checks use a held note at -24 dBFS RMS (`HELD_NOTE`), the match is LUFS. Click limits for a
  switch to Klar are 1.35 times its own steepest step (it starts with its full headroom) and
  1.8 for a jump of the pedal's Level into Klar

Loudness after M1, riffs / clean playing, LUFS, Gain 5 (`loudness_report` has every Gain):

| DI dBFS RMS | -36 | -30 | -24 | -18 | -12 |
|---|---|---|---|---|---|
| Klar before | -30.8 / -31.5 | -24.8 / -25.6 | -19.2 / -19.8 | -14.1 / -14.5 | -10.2 / -10.1 |
| Klar | -25.0 / -25.7 | -19.5 / -20.0 | -15.5 / -15.4 | -13.0 / -12.4 | -11.6 / -10.7 |
| Brøl | -24.2 / -24.9 | -20.5 / -20.8 | -17.9 / -17.8 | -16.1 / -15.7 | -15.0 / -14.4 |
| Torden | -17.9 / -16.8 | -17.2 / -16.2 | -16.8 / -15.8 | -16.6 / -15.7 | -16.3 / -15.6 |

Klar's crest on the riffs: 19.8 / 17.5 / 14.8 / 12.8 / 11.9 dB (was 20.3 / 20.0 / 19.2 /
16.2 / 12.4): at the reference level its power stage takes 4.4 dB off the hardest attacks,
at -36 half a decibel. Peaks: -7.6 / -4.3 / -2.9 / -2.3 / -1.8 dBFS (were in the safety
clip from -18 on). A held note in Klar at Gain 5: -77 / -70 / -61 / -20 / -12 dB.

### M1: what it leaves

- Nobody has heard it. Whether Klar's rounded attacks still sound clean, and whether the
  presets feel equally loud when playing, is Rasmus's to say (`amp_presets_in_a_row.wav`,
  `amp_klar_clean_playing_gain5_di-30 / -24 / -18.wav` in `target/renders`)
- The match holds at one guitar level. A guitar at -30 dBFS RMS still has the clean presets
  about 4 LU under the Torden ones (it was 9 to 11), one at -36 about 7 (was 13): the Input
  knob, or the compressor of M3, closes the rest. Nothing in the window says what level the
  guitar comes in at; an input lamp would
- A guitar hotter than -20 dBFS RMS makes Klar's held notes distort (-20 dB at -18 dBFS
  RMS): it has less headroom than before for hot pickups. Input down, or Master down, gives
  it back
- Klar is louder for 20 ms after it is switched to or the pedal's Level jumps (the sag and
  the giving way take that long)
- Tåge with its long repeats and tail reaches -2.1 dBFS on chords from a guitar at -18 dBFS
  RMS, 0.4 dB into the safety clip's knee
- The plan's target of amps within 3 LU from -30 to -18 dBFS RMS is not met and cannot be:
  Brøl and Torden themselves are 3.3 LU apart at -30. Held instead: within 3 LU at the
  reference level, within 5 LU 6 dB either side

### M3 so far

- `comp.rs` (by a subagent, merged): feed-forward, no lookahead, side chain high-passed at
  80 Hz, the level held over 15 to 30 ms so a held note does not ripple, soft knee, 3 ms
  attack with at most 6 dB of overshoot, 150 ms release, make-up tied to Sustain (within
  0.3 LU of the pedal off at the reference level), at most 20 dB of gain on silence. 5 ns
  per sample. `comp_report`, `render_comp`
- The chain walks `ChainOrder::front()` between the oversampler and the preamp and `back()`
  behind the DC blocker; the order is the default until M5. With the pedal off every render
  of `fingerprint_report` is the same as after M1
- Loading a preset switches the compressor off; no preset uses it yet
- A switch to Klar with compressor and drive on is up to 1.8 times as steep as Klar settled,
  for the 20 ms its power stage takes to give way (the test allows 2.0)
- Sustain 10 raises hiss by 20 dB when the gate is off; behind the cabinet the pedal is not
  calibrated (levels differ there): to measure once it can be moved

### M4 so far

- `mods/` (by a subagent, merged): chorus (one voice per side, 7 to 12 ms, 0.1 to 6 Hz),
  flanger (0.5 to 6 ms, feedback 0.5, 0.05 to 5 Hz), phaser (four all-pass stages, 200 Hz to
  1.6 kHz, feedback 0.25, 0.1 to 8 Hz), tremolo (sine, 0.5 to 12 Hz). Rate and Depth each.
  Behind the cabinet the right side runs a quarter turn ahead, except in the tremolo.
  `mods_report`, `render_mods`
- In the chain at their default places: phaser in front of the amp, chorus, flanger and
  tremolo behind the cabinet in front of the delay. Parameters `chorus_on`, `chorus_rate`,
  `chorus_depth` and likewise `phaser_`, `flanger_`, `trem_`. Off, every render of
  `fingerprint_report` is the same as after M1. Loading a preset switches them off
- Cost per host sample at 48 kHz: behind the cabinet 19 / 23 / 30 / 4 ns (chorus, flanger,
  phaser, tremolo); in front of the amp about 46 / 54 / 64 / 15 ns. All four in front would
  add about 0.9 % of a core to the chain's 2.0 %
- Not heard, and not measured through the amp: the phaser lifts the signal by up to 4.8 dB
  between its notches (make-up for the 50/50 mix), also into a high-gain amp; its mono sum
  is about 2 LU under one side; at Depth 0 it is 1.5 LU under off
- A reset pedal that is on fades in over 10 ms; `set_sample_rate` resets a pedal, which is
  what M5 will do to one that changes sides

### M5 so far

- `GuitarAmpParams.order` (`#[persist = "chain-order"]`, `order::StoredOrder`): saved as the
  list of pedal ids with `amp` between the two sides. A project without it gets the default
- The chain reads the order with the dials. A new order fades the output to silence in 5 ms
  (`ORDER_FADE_MS`), the pedals change places at the silent sample, and it fades back; a
  pedal that changes sides is set up for the rate of its new side from rest. Bypassed, the
  order changes at once
- Tests: six orders are heard, bounded, the same in every block size, at 44.1 and 96 kHz,
  with the latency of the default; changing to and from each while chords play is no
  steeper than 1.2 times the chords themselves and falls on the same sample in other blocks
- Every render of `fingerprint_report` is still the same as after M1
- Left: nothing in the window sets the order (the mock keeps its own); presets do not hold
  it; a pedal that changes sides starts its sweep again; what a swap of delay and reverb
  does to their tails was only measured as steps, not heard

## How to resume (round 1)

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
| Backlog 9: per-amp dial memory | done 2026-10-10 |
| Backlog 7, small version: gate-open and clip lamps | done 2026-10-10 |

## Next

Waiting on Rasmus for one layout decision before backlog 4 (more amps and pedals): the head's
bottom row is full (three amp switches, Tuner, On), the board holds exactly five pedals at
960 wide, and the model print says GA-3. Recommended to him on 2026-10-10: the amp selector
becomes a `stepper` (any number of amps), and pedals are added by widening the window to six
slots. Do not start backlog 4 without his answer.

Nobody has listened to anything yet, and the plugin has not been in a DAW. That is now the
most useful next step by far, and it is Rasmus's. If he has listened and left notes, they
come first.

Work that needs neither:
- Backlog 5 (mic choices per cabinet) needs room on the Cab pedal, which is full: it goes
  with the layout decision above
- Backlog 8 (shared `suite_common::dsp`) is set aside: the drums use f32 state-variable and
  one-pole filters, the amp f64 biquads, so no filter is in use by both. Moving code would
  put the drums at risk for nothing. Take it up when a third plugin needs the amp's filters
- A logarithm of our own in the second-order clippers would save about 1.7 us per block but
  changes the last bits of the output: needs a decision
- The report tests do not pin their thread, so timings on this machine come out in two
  modes (fast and slow cores, 1.33 times apart). Pinning inside the reports would make
  before/after comparisons trustworthy

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
- 2026-10-10 Dial memory: editor-side state, persisted as `amp-dials` (atomics, since nih-plug
  restores state on the audio thread). The six dials stay shared parameters
- 2026-10-10 Dial memory: at the silent sample of an amp switch the six amp dials jump to their
  target, so the new amp starts at its own dials
- 2026-10-10 Lamps: the gate gets its own small `OPEN` lamp. A half-lit pedal LED was built
  first and rejected: at 30 % the lens is about the colour of the paint
- 2026-10-10 Aliasing: Torden with the drive at 10s was limited by square waves reaching the
  second stage, not by the order of antialiasing. `FineClipper` takes two second-order steps
  per sample there, only while the drive is on: -67.6 to -74.8 dB at 44.1 kHz for 0.3 % of
  a core at 48 kHz. Drive off is bit-identical
- 2026-10-10 Preset gates: -54 dB is the highest threshold that leaves palm mutes at -30 dBFS
  untouched; release, not threshold, sets how tight the rests are. Stram -56 dB / 50 ms,
  Granit -54 / 60, Dyb -54 / 40, Lyn -54 / 180. A test holds every preset to it
- 2026-10-11 Drive aliasing on every amp: Brøl's second stage clips at twice the rate while
  the pedal is on, as Torden's does; Klar's power stage gives way under a heavy load while
  the pedal is on, so sustained notes stay under the output's safety clip. Pedal off is
  bit-identical on every amp
- 2026-10-11 Built-in cabinets are designed at 48 kHz, the rate they were tuned at, and
  resampled for every other rate; 48 kHz is unchanged to the bit. The resampler moved to
  `dsp/resample.rs` and serves user cabinets too
- 2026-10-11 Cost: the 39.7 us figure was a slow core (cores 2 to 9 of this machine take 1.33
  times as long). On a fast core it was 30.8 us against 27.3 after M5: 2.6 us the finer
  clipping, 0.3 us a branch that had joined the power stage's feedback loop, nothing
  measurable from lamps, hand-off check, tuner or dial jump
- 2026-10-11 Clippers take two samples as one SSE2 pair, bit-identical (hash of 54 renders).
  Rejected: block passes for auto-vectorising (stages got slower: the filters no longer
  overlap the clipping), plain two-number pairs in release, an own logarithm (not bit-identical)
- 2026-10-11 Long user cabinets: zero-latency two-stage convolver. The head of two blocks
  stays a direct FIR, the rest runs in uniform FFT partitions (block 64 up to 96 kHz, 128
  above), own radix-2 FFT, work spread per sample, spectra made off the audio thread in
  `CabStage::offer`. Response identical to the direct FIR; no tail under 641 taps. A tail at
  a decimated rate was not built: from the measured parts it cannot beat this
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
- Mic and Res use the selected amp's frequencies also under a user cabinet

- Dial memory: a switch is 7 host undo steps, and host undo does not undo the memory;
  automation that writes the dials while the player switches amps fights it; a host that
  changes Amp itself leaves the dials alone (by decision)
- Lamps: the cost of repainting could not be measured: the standalone's UI thread already
  uses 0.4 to 0.7 of a core on this machine with frames arriving in bursts about 250 ms
  apart. The unchanged Drum Synth standalone does the same, so it predates the amp. Worth a
  look in a real host: if frames really come that slowly, lamps and the tuner lag as much
- Lamps: the 150 ms clip hold was unit-tested, not watched on screen
- Finer clipping with the drive on made a few 1245 Hz aliasing figures slightly worse while
  the 4186 Hz ones improved (largest: Torden 10, drive 3/5/8, 44.1 kHz, -87.4 to -83.2 dB)
- Gates: mutes quieter than -36 dBFS are still chopped at -54 dB; a guitar noisier than
  -62 dBFS RMS holds the preset gates open. All measured on synthetic plucks

- The agent doing the drive aliasing and the cabinets was cut off twice and left no report.
  Its two commits are verified by the tests and `amp_report`; its account of what it tried
  and rejected is missing. Its third piece (long user cabinets at 192 kHz) was not started
- Klar's power stage now gives way under a heavy load while the drive pedal is on. That is a
  change of sound with the pedal at high Level into Klar, chosen by the numbers and unheard
- With the drive at 10 / 10 / 10 the closest figures to the -70 dB target are Torden Gain 10
  at 44.1 kHz (-74.8 dB) and Brøl Gain 10 (-75.6 dB)

Figures of 2026-10-11 (48 kHz unless said): aliasing with the drive at 10 / 10 / 10, Gain 10,
1245 / 4186 Hz tone: Klar -89.9 / -81.7, Brøl -78.6 / -78.5, Torden -78.3 / -76.5 dB; at
44.1 kHz -88.4 / -79.9, -76.5 / -75.6, -75.5 / -74.8. Cabinets across rates: within 0.044 dB
from 60 Hz to 10 kHz. Everything on, stereo, both effects (Torden Gain 10): 39.7 us per
64-sample block (3.0 %), 96 kHz 25.2 us (3.8 %), 192 kHz 26.1 us (7.8 %). The drive at
10 / 10 / 10 adds 0.4 % (Klar), 0.8 % (Brøl), 0.6 % (Torden) of a core at 48 kHz. Designing
the cabinets for a new rate takes 4 to 12 ms once; setting a known rate 0.1 ms.

- A swap to a long user cabinet now waits for its tail: up to 40 ms after a built-in or short
  cabinet (the old one plays on), two blocks after another long one. It was immediate before
- With 16-sample host blocks the cabinet's worst block is up to 1.9 times its median (each
  FFT is done in one piece); far below what the chain takes
- When Bypass has faded while a long cabinet is in use, the reset clears about 60 KB of tail
  buffers once, on the audio thread
- The SSE2 pair path of the clippers is compiled only in optimised x86-64 builds, so
  `cargo test` in debug does not run it; `cargo test -p amp --release` does and passes.
  The inlining attributes around it were chosen by timing the test binary, not the plugin
- Debug-mode amp tests take 24 to 29 s now

Figures of 2026-10-11 after the cost work, on a fast core, Torden Gain 10, everything on,
stereo, both effects: 27.1 us per 64-sample block (2.0 %) at 48 kHz, 17.1 us (2.6 %) at
96 kHz, 18.5 us (5.5 %) at 192 kHz; drive off 22.4 us (1.7 %) at 48 kHz. With a 40 ms user
cabinet: 28.2 us (2.1 %), 18.2 us (2.7 %), 18.4 us (5.5 %); it was 15.4 % at 192 kHz. The
earlier "3.0 % at 48 kHz" was measured on a slow core; on a fast core it was 2.3 %.

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
6. Lower CPU (shorter or partitioned IRs, SIMD) (done)
7. Input and output meters (small version done: gate-open and clip lamps)
8. Shared `suite_common::dsp` for filters used by more than one plugin
9. Per-amp knob memory (done)
