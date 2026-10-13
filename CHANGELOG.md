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

- Cymbals and hi-hats are crisp: four times as many resonances, so the top is a sizzle and no
  longer a swirl of separate tones, and the low hum under the tail is gone
- The snare has its crack: louder and brighter wires, a fuller stick attack, and as loud in the
  kit as the kick and the toms

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
