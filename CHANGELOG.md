# Changelog

Each release has a `## <version>` section here. The release workflow uses it as the text of the
GitHub release.

## Unreleased

### Bass Synth, Shimmer Pad, Glass Bells

Three new instruments, each a pedal of its own:

- Bass Synth: a plucked string bass with two voices. Metal is picked, with clean lows under a
  distorted band of mids; Groove is played with a finger, warm, and slides between notes
  that overlap
- Shimmer Pad: a slow ambient pad of detuned saws and breath, with a chorus and a large hall
  whose tail comes back an octave up
- Glass Bells: soft glassy bells with a ping-pong delay and a hall

### Drum Synth

- The Metal kit is a new instrument: a clicking kick, a cracking snare, cymbals of square
  waves and noise, its own room and a saturated bus. It replaces the Metal kit there was;
  projects set to Metal play the new one
- A fourth kit, Groove: warm and soft, with a round kick, ghost notes that are darker and
  shorter than the backbeat, dark cymbals and a shaker (notes 69, 70 and 82)
- The hi-hat follows an electronic kit's pedal (MIDI CC4): a hit on 46 with the pedal half
  down is a half-open hat, and shutting the pedal chokes a hat that rings. On every kit; a
  part without the pedal sounds as before
- A Choke switch: with it on, a note-off on a cymbal that was held for a moment damps it at
  once, as a hand does. Off by default. The kit switches have moved to make room

- Cymbals and hi-hats are clear: the top is noise in six bands that die away one after the
  other, and the resonances only carry the body, so the bell-like ring and the chorus of
  beating tones are gone, and the low hum under the tail too. Rock and Jazz kits
- The snare has its crack: louder and brighter wires, a fuller stick attack, and as loud in the
  kit as the kick and the toms

### Guitar Amp

- The noise gate closes behind a note on a noisy guitar: hiss only has to sit 3 dB under the
  threshold, not 6. A note's tail is cut 3 dB sooner, and the four presets that had the
  gate at -54 dB are at -57 dB, so they sound as before
- The tuner no longer shows mains hum as a note. A note is shown from a pluck on, and let go
  when what rang does not fade like a string; a string that already rings when the tuner is
  switched on is read from its next pluck
- Level meters on the head: a column of lamps for the level the guitar comes in at, left of the
  dials, and one for the output level in front of the safety clip, by the clip lamp. They jump
  to a peak and fall over about a third of a second; the lowest lamp is lit from -45 dB and the
  top one at full scale. The window is only redrawn while the lamps move
- The clippers of the high-gain amp and the overdrive use less CPU. The sound is the same to the
  ear; only the last bits of the output move
- Delay and reverb at their maximum no longer push the output into its safety clip: what each
  gives out is rounded off under full scale, and left alone below 70 % of it. The delay's
  last 28 dB of tail (everything under -72 dBFS) is cut, so a 1000 ms delay at full feedback
  falls idle after about 37 seconds instead of 62, and a 350 ms one after 14 instead of 22
- A project with a large cabinet file on a slow disk no longer holds up the amp opening: the
  file is read in the background, and the amp plays through its own cabinet until it has arrived
- With a cabinet of your own, Mic and Resonance no longer follow the amp you picked: they work
  on the same frequencies for every amp (Mic a shelf up high, Resonance a bell at 110 Hz), and
  at their centre they leave your file as it is
- Switching amp, or moving a pedal, dips the sound for about 9 ms instead of 10: the fade is
  4.5 ms down and 4.5 ms up. It still does not click
- Your own cabinets can be 8-bit or 64-bit float WAV files, which showed BAD FILE before
- The amp is picked with a stepper (a name on label tape with a button at each end) under the
  dials, instead of three switches. Each amp still keeps its own dials
- Torden's palm mutes are tighter: about 6 dB less below 100 Hz against the mids than Brøl, up
  from about 4, and a low E keeps a little more of its fundamental than before
- Torden's Gain dial does more above 5: it is no longer squeezed flat by the middle of the dial,
  so 5 stays tighter and more dynamic and 5 to 10 grows into thicker, singing sustain. The
  dial is a little hotter at the top

### Guitar Amp, Shimmer Pad, Glass Bells

- VST3 hosts that stop processing a silent plugin no longer cut the delay and reverb tails:
  the plugins tell the host from the start that their tails ring on

## 0.1.0

First public release. VST3 and CLAP for Windows, macOS and Linux. The macOS and Linux builds
come from the build server and have not been tried in a DAW yet.

### Pitch Shifter

- Shifts a guitar by -12 to +12 semitones for live playing
- Delay of about one pitch period of the note being played: 12 ms on the low E string, under
  3 ms high up the neck
- Bit-exact passthrough at 0 semitones; the footswitch bypasses without a click

### Drum Synth

- A full drum kit with no samples: every sound is made in the plugin
- Three kits: Rock, Jazz, Metal
- General MIDI drum map; velocity changes the sound, not only the level
- Room reverb, Tune, Damping and a level per group

### Guitar Amp

- Three amps, each with its own cabinet: Klar (clean), Brøl (crunch), Torden (high gain)
- Ten pedals: gate, compressor, overdrive, phaser, chorus, flanger, tremolo, cabinet, delay,
  reverb. Drag a pedal by its title to move it in the chain
- Your own cabinet impulse responses (WAV) from `Documents/Hojt Audio/Cabinets`
- Factory presets and a built-in tuner
- 0.1 to 0.3 ms of latency
