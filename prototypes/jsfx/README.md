# The scripts the newer instruments were written as first

Six REAPER JSFX instruments. They are the reference for their ports in `plugins/`: the
numbers in the ports are the numbers here. They are not built, tested or shipped, and a port
is free to move on from its script.

| Script | Port |
|--------|------|
| `claude_metal_drums.jsfx` | Drum Synth, Metal kit (`plugins/drums/src/engines/metal.rs`) |
| `claude_groove_kit.jsfx` | Drum Synth, Groove kit (`plugins/drums/src/engines/groove.rs`) |
| `claude_metal_bass.jsfx` | Bass Synth, Metal voice (`plugins/bass/src/bass.rs`) |
| `claude_groove_bass.jsfx` | Bass Synth, Groove voice |
| `claude_ethereal_pad.jsfx` | Shimmer Pad (`plugins/pad/src/pad.rs`) |
| `claude_glass_bells.jsfx` | Glass Bells (`plugins/bells/src/bells.rs`) |

To hear one, copy it to REAPER's `Effects` folder and put it on a track with MIDI.

Where a port differs from its script:

- The drum kits take Drum Synth's controls. Tune and Damping are new to them; the scripts'
  Drive, Warmth and Tape drive are fixed at their defaults, and so are the level trims, which
  are the kit's balance at 0 dB. Room at 25 % is the script's default room. The output is
  4.9 dB lower, to sit with Rock and Jazz
- The kits keep the suite's note map: note 39 does not play Metal's snare, and Groove plays
  its crash for the china (52)
- The two basses are one plugin. Slide at 0 ms plucks every note, as the metal script did;
  Tone on the Metal voice is the low-pass behind its drive, which the script fixed at 3200 Hz
- Pad and bells have two switches each that the scripts did not, and the bells' delay time
  glides when its knob is turned
- The pad sets its filters every 32 samples, not once per block of the host
