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
| M2 Three amps | not started |
| M3 Gate, drive, pedalboard | not started |
| M4 Delay and reverb | not started |
| M5 Tuning and cost | not started |

## Next

M2: `amp` parameter (`EnumParam<Amp>`, variant ids `klar`, `brol`, `torden`; `Amp` already
derives `Enum`), `amp: Amp` in `AmpSettings`, Klar and Torden models and cabinets in
`amp/model.rs`, click-free switching (fade the amp out over about 5 ms, reconfigure and reset,
fade in, with `Cabinet::swap_ir` alongside), levels matched between amps at default settings.
Editor: three `small_footswitch` at y 286, x 295 / 425 / 555, each with an LED at (x - 58, 275)
above a `silk_label` size 17 at (x - 58, 301).

Layout notes for M3: window 960 x 660, five `mini_pedal`s of 180 x 300 at x = 10 + 190 i,
y 350 to 650, LED at pedal top + 198, `small_footswitch` at + 242. Two radius 19 knobs fit in
a row; three touch at the ticks, so Drive and Delay need a triangle layout or smaller knobs.

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
- 2026-10-10 Worktrees for subagents start from `main`, not from `amp-sim`. Tell each agent to
  run `git merge --ff-only amp-sim` first

## Known problems

- Nobody has listened yet. Everything is tuned by the report. The first listen to
  `target/renders/amp_*.wav` may ask for another voicing (cab bite, low end, gain at 5)
- Brøl at Gain 0 is not fully clean on hard pick attacks (the level table pushes transients
  into the power stage)
- CPU at 192 kHz is about 11 % of one core per instance: always 4x, 3840-tap FIR. Going to 2x
  at 96 kHz and above is the obvious saving (M5)
- Tone and presence filters are redesigned once per 32 samples while a dial moves; a fast
  automated sweep could zipper faintly
- `AmpChain::set_amp` crossfades the cabinet only; the amp itself switches at once (M2)
- An unlit LED on oxblood reads as an empty chrome ring: `LED_OFF` was tuned for orange and
  teal. Changing it changes the other two plugins, so it waits for a decision
- The no-denormal-slowdown test is a timing assertion (silent under 2x playing); stable so far
- The standalone window was looked at as a capture of the window; the plugin has not been
  loaded in a DAW

## Last report

After M1 (2026-10-10), 48 kHz, dials at 5 except Gain:

```
amp      gain  chords RMS   chords pk  sine RMS   sine pk   THD dB  alias 1245  alias 4186
Brøl      0.0       -19.5        -6.8     -20.5     -17.5    -37.4      -122.0      -123.9
Brøl      5.0       -16.6        -6.5     -14.9     -12.0    -15.5      -122.1      -120.5
Brøl     10.0       -15.6        -6.2     -14.3      -9.0     -8.2       -98.4       -86.6

Latency  48000 Hz    8 samples  0.167 ms
Time per 64-sample block: 48 kHz 21 us (1.6 %), 96 kHz 23 us (3.5 %), 192 kHz 37 us (11.2 %)
Silent tail costs the same as playing (no denormal slowdown)
```

## Backlog after M5 (top to bottom)

1. Factory preset browser
2. User IR loader
3. Tuner
4. More amps and pedals (compressor, fuzz, chorus)
5. Mic choices per cabinet
6. Lower CPU (shorter or partitioned IRs, SIMD)
7. Input and output meters
8. Shared `suite_common::dsp` for filters used by more than one plugin
9. Per-amp knob memory
