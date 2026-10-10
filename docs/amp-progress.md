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
| M5 Tuning and cost | not started |

## Next

M5, tuning and cost, in this order:

1. 2x oversampling at 88.2 kHz and above (192 kHz is 16 to 17.6 % of a core, almost all in
   the oversampled region). `FACTOR` becomes a runtime value; re-check aliasing at 96 kHz
   (Torden Gain 10 with the drive is the row to watch)
2. Shorter cabinet IR at high rates (3840 taps at 192 kHz): measure how many taps hold the
   response within 0.5 dB, or cap the IR
3. Klar headroom: Gain 0 with default effects peaks 0.1 dB under the safety clip's knee;
   1 to 1.5 dB less makeup, keeping the 2 dB level match between amps
4. Faster idle: the delay waits a full line (1 s) of silence and both run down to -120 dBFS
   (77 s at 1000 ms / 90 %)
5. Bundle VST3 and CLAP (`cargo xtask bundle amp --release`) and check the bundle is there
6. Then the backlog at the end of this file

CLAUDE.md has the plugin's structure, signal flow and parameter table.

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
- M3: `AmpSettings::default()` mirrors the parameter defaults in `lib.rs` by hand
- M3: Torden costs 2.9 % at 48 kHz and 14.5 % at 192 kHz (16.1 % with every pedal on)
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
- M4: everything on in stereo, Torden: 3.6 % / 7.8 % / 17.6 % of a core at 48 / 96 / 192 kHz.
  The effects cost about 5 us per 64-sample block (delay 1.3, reverb 3.7)
- M4: latency is unchanged by the effects (Klar 8, Brøl 10, Torden 14 samples at 48 kHz with
  every pedal on)

## Last report

After M2 (2026-10-10), 48 kHz, dials at 5 except Gain. The full report has more tables
(tightness, dynamics, cabinet responses, switching steps).

```
amp      gain  chords RMS   chords pk  sine RMS   sine pk   THD dB  alias 1245  alias 4186
Klar      0.0       -17.6        -3.0     -19.7     -16.7    -53.4      -125.2      -120.0
Klar      5.0       -17.0        -3.6     -17.1     -14.1    -44.7      -125.6      -125.1
Klar     10.0       -16.2        -4.5     -14.9     -12.3    -21.5      -124.8      -123.2
Brøl      0.0       -19.5        -6.8     -20.5     -17.5    -37.4      -122.0      -123.9
Brøl      5.0       -16.6        -6.5     -14.9     -12.0    -15.5      -121.3      -121.7
Brøl     10.0       -15.6        -6.2     -14.3      -9.0     -8.2       -98.4       -86.6
Torden    0.0       -17.4        -6.9     -13.5     -10.1    -24.5      -123.2      -122.6
Torden    5.0       -16.9        -6.5     -13.0      -8.2    -18.0      -116.6       -99.8
Torden   10.0       -16.3        -5.6     -12.3      -6.1    -14.3       -76.8       -73.8

Tightness (palm mutes on 65 and 73 Hz roots), dB relative to the whole signal
Hz                       to 100  100-400  400-1k6  1k6-6k4   6k4 up
Brøl 5.0                  -21.6     -6.3     -2.7     -6.5    -28.8
Torden 5.0                -25.1     -7.2     -3.7     -4.3    -27.4
Torden 5.0 boosted        -26.3     -9.7     -3.0     -4.1    -26.5

Dynamics at Gain 5, 24 dB between soft and hard playing: squeezed by Klar 0.8, Brøl 11.0, Torden 23.2 dB
Latency at 48 kHz: Klar 7, Brøl 8, Torden 12 samples (0.15 / 0.17 / 0.25 ms)
Time per 64-sample block at 48 kHz: Klar 19 us (1.4 %), Brøl 24 us (1.8 %), Torden 29 us (2.2 %)
At 192 kHz: 8.7 % / 10.2 % / 11.8 %
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
