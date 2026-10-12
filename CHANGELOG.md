# Changelog

Each release has a `## <version>` section here. The release workflow uses it as the text of the
GitHub release.

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
