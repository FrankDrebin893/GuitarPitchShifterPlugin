# Drum Synth

Loaded when working in `plugins/drums`. The suite-wide rules are in the root `CLAUDE.md`.

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

The hi-hat pedal (MIDI CC4, 127 shut, as electronic kits send it), on every kit:

- It sets how open a hit on 46 is: a sound between the closed hat (42) and the open one,
  decay times on a log scale (`CymbalPreset::between`, `hat_time` in the engines). Fully open
  is the open hat to the bit, and the hats are open until the pedal first moves. 42 and 44
  stay as they are
- Shutting it (under 10 % open) while a hat rings chokes it to the closed hat's decay
  (`Cymbal::choke`). It plays no chick: an electronic kit sends 44 for that itself
- The openness is kept when the kit changes; `reset` opens the hats again

Choke (`choke`, off by default): a note-off on a cymbal (49, 51, 52, 53, 55, 57, 59) damps it
at once, as a hand does, and a note-off on anything else does nothing. Hi-hats are the pedal's.

- A note-off within 120 ms of its note-on is ignored, so a drawn sixteenth does not cut a
  crash. With Choke off note-offs are ignored altogether, and the sound is as it was to the bit
- Rock and Jazz: `Cymbal::choke` with `CymbalPreset::damped` (every decay at most 25 ms);
  Metal and Groove: `choke_cymbal` pulls the envelopes of the ride, crashes, china and splash
  to a 25 ms time constant (Damping scales both)
- The kit tells time from the plugin (`set_time`, per event) and not by counting in `process`,
  where one more store per frame costs a ringing kit 8 %

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

Not there yet: see the `plugin:drums` issues.

Rock and Jazz are tuned in `src/kit.rs`, Metal and Groove in their engine's file; check `kit_report` and listen to `render_wavs`.

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
