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
| M1 One playable amp (Brøl) | in progress: scaffold and contract committed |
| M2 Three amps | not started |
| M3 Gate, drive, pedalboard | not started |
| M4 Delay and reverb | not started |
| M5 Tuning and cost | not started |

## Next

M1: oversampler, filters, shaper, preamp, tone stack, power amp, Brøl cabinet, `amp_report`,
`render_wavs`, tests. Shared UI: `rig`, `head`, `PAINT_OXBLOOD`, `Ornament::Bolts`; editor is
the head alone.

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

## Known problems

None yet.

## Last report

Not run yet.

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
