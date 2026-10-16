# Guitar Amp: why it is the way it is

The reasons behind choices that the code alone does not explain. What it does is in
`AGENTS.md` (or `CLAUDE.md`) next to this file; what is open is the `plugin:amp` issues. Dates are 2026.

## Targets (set with Rasmus)

- Torden: modern tight metal, tight low end, fast attack, clear in drop tunings, meant to be
  boosted by the drive. Klar: glassy, lots of headroom, slightly scooped. Brøl: mid-forward
  stack crunch that cleans up with picking dynamics
- Presets: good defaults first, a browser later. Cabinets: our own at first, a loader later
- The window is one rig: a head over a row of pedals, each pedal with its own paint and art

## Signal and numerics

- Amp and cabinet are mono (a stereo input is averaged); delay and reverb make stereo
- Clippers are antialiased (first-order antiderivative, f64) on top of oversampling; plain 4x
  measured about -40 dB aliasing at full gain, this gives -87 dB or better. Second order only
  where it pays (Torden's second and third stages, the drive); everywhere cost twice as much
  for the same figures. With the drive on, a stage that would be hit by square waves clips at
  twice the rate instead (`finer_when_driven`)
- Oversampling is minimum-phase IIR half-bands: 4x at 44.1 and 48 kHz, 2x from 88.2 kHz
- Biquads are f64: at 4 x 192 kHz a 100 Hz filter sits too close to the unit circle for f32
- A 1e-20 constant in every feedback path keeps denormals away (tests run without
  flush-to-zero)
- A preamp stage takes its operating point from the previous sample, which breaks the chain
  of divisions and saves a quarter of the preamp's time. The preamp runs one stage over the
  whole block and then the next (same output, a third less time)
- Two samples at a time through the clippers as an SSE2 pair, bit-identical. Rejected: block
  passes for auto-vectorising (slower), plain pairs of numbers in release, an own logarithm
  (not bit-identical; issue #22)
- nih-plug does not smooth parameters; `AmpChain` does, per 32-sample tick counted across host
  blocks, so tests that drive the chain behave like the plugin and the output does not depend
  on block size
- Latency target under 1 ms, nothing reported to the host (0.1 to 0.3 ms)

## Amps and levels

- Amps are matched by loudness (LUFS) for a guitar at -24 dBFS RMS, not by RMS: a clean amp
  follows the guitar dB for dB and a clipping one hardly does, so they can only meet at one
  level. Brøl and Torden are themselves 3.3 LU apart at -30, so "within 3 LU from -30 to -18"
  cannot be met; held is 3 LU at the reference and 5 LU 6 dB either side
- Klar's power stage holds peaks with its own curve (`clean_to`: straight up to a knee, then
  bending), because the old curve bent from the start and gave -36 dB of distortion on a held
  note at the reference level (now -61)
- Brøl below Gain 5 is made up in front of its power stage so Gain 0 is clean on pick attacks
  and the safety clip is not what holds the peaks
- Switching amp fades out over 3.5 ms in front of the cabinet, switches at the silent sample,
  fades in; the cabinet crossfades over 10 ms. Two amps never run side by side (double the
  oversampled cost). The six dials are shared parameters; each amp's own positions are kept by
  the editor (`amp-dials`, atomics because nih-plug restores state on the audio thread)
- Cost figures are from a fast core; cores 2 to 9 of the development machine take 1.33 times
  as long, so the reports pin their thread

## Cabinet

- Designed at 48 kHz and resampled for other rates, so the sound matches across rates; the
  response is normalised to 0 dB average power from 100 Hz to 4 kHz
- The speaker resonance is a biquad behind a short IR (5 to 10 ms), fitted to the old 20 ms
  response within 0.4 dB from 60 Hz to 8 kHz. It is not applied to the player's own cabinets
- Mic is a shelf pair and Resonance a bell after the IR, skipped at 5.0 so the default is
  bit-identical
- No file dialog (the style guide forbids pop-ups): files in one folder, stepped through. The
  folder is found through the Windows known-folder call by a hand-written FFI declaration (it
  follows OneDrive), no crate. The choice is a persisted file name, not a parameter, not in
  presets, and stays when the amp changes
- Player cabinets are cut to 40 ms at every rate, trimmed to arrive within 0.2 ms, levelled
  like the built-in ones. Long ones run as a zero-latency two-stage convolver (head of two
  blocks direct, the rest in FFT partitions, spectra made off the audio thread); a tail at a
  decimated rate was measured and cannot beat it
- Taps reach the audio thread through one pre-allocated buffer with an atomic state; a swap to
  a long cabinet waits for its tail (the old one plays on)

## Pedals

- Gate: hard, 3 dB hysteresis, 0.5 ms opening ramp, 40 ms hold, no lookahead, its detector
  reads before the input gain. Preset gates: -54 dB is the highest threshold that leaves palm
  mutes at -30 dBFS alone; release, not threshold, sets how tight the rests are
- Drive: 720 Hz high-pass into a soft clipper with the dry signal added back, in the
  oversampled region before the preamp
- Delay and reverb are additive (the dry signal is never scaled, so mix 0 and off are
  bit-transparent). The delay's two lines feed each other so the left-right offset does not
  grow; its odd repeats lose 3 dB in mono by design (#36). Bypass cuts the tails in 10 ms
- The chain reports `KeepAlive` while a tail rings, since `Tail(n)` needs the length ahead of
  time. VST3 hosts read the tail once, so the amp, pad and bells set `VST3_INFINITE_TAIL` in
  our nih-plug fork: a host that suspends silent plugins never suspends these (about 2 % of a
  core on silence)
- A mono host layout gets the left effect channel only: the sum combs by up to 6.7 dB
- The order is saved as ids, never a parameter. A new order fades out in 3.5 ms, swaps at the
  silent sample, fades in; a pedal that changes sides starts from rest

## Tuner

- Muting is the behaviour: the amp's input and its output fade in 10 ms so tails are silent
  and a tuned string does not end up in the delay. `filter_state` drops `tuner` from a loaded
  state so a project never opens muted. Bypass wins while tuning (the dry signal is heard)
- Detector at about 5.5 kHz: normalised difference function for the rough period, a harmonic
  check against octave errors and chords, then the phase advance of the first three harmonics
  for the exact pitch (interpolating the difference function cannot reach a cent on high notes
  at that rate). The median of three readings is shown
- A note is shown only after a pluck (a 25 ms frame 12 dB over the quieter of the two before
  it; 6 dB to change notes). 25 ms is 1.5 periods of 60 Hz and 1.25 of 50 Hz: with 10 ms
  frames hum swung 5 dB from frame to frame. Hum that sets in at once must also fall 3 dB from
  0.2 s to 1 s, or the note is let go (#18; what remains is #44)

## Look

- The head is 320 px tall with 50 px small footswitches and the LED beside the switch (a full
  footswitch with an LED above does not fit under the band); the band on a head tilts -1.5
  degrees, pedals -4. The amp switches stand 40 px left so the Tuner switch does not read as a
  fourth amp
- The gate's `OPEN` lamp is its own small lamp: a half-lit pedal LED was built and rejected
  because at 30 % the lens is about the colour of the paint
- Rigs use `rig_led` (black socket, darker unlit lens): a plain unlit `led` disappears on dark paint

## Not yet exercised

Some of this was only run on synthetic signals or in the standalone window; #21 is the check in
a real DAW: mouse clicks on the stepper, saving and restoring the preset index and the cabinet
file in a project, the background task in VST3/CLAP hosts, macOS and Linux folders, a
OneDrive-redirected Documents folder, `non_automatable` and the tuner state filter, the
repaint rate of lamps and tuner, and host undo (#16).
