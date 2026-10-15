# Performances for the page's sound examples

What is played in a sound example on the page, when it is not the playing a render test makes
up: a guitar recorded straight in, a drum part as MIDI. The sound is never kept here. It is
made from these files by the plugins as they are, with `scripts/site-audio.sh`, so an example
follows the plugins and the playing stays the same.

This folder is not part of the page: only `site/` is published.

## What a file must be

| For | File | |
|-----|------|---|
| Guitar Amp | `guitar-<what>.wav` | Mono, straight from the guitar, no effects. 48 kHz (the presets are rendered at that rate only). Recorded at about -24 dBFS RMS, the level the amps and presets are matched for |
| Pitch Shifter | `guitar-<what>.wav` | Mono, straight from the guitar. Any rate |
| Drum Synth | `drums-<what>.mid` | Notes of the General MIDI drum map (the table in `AGENTS.md`). One tempo: a part whose tempo changes is refused. Note lengths do not matter |

## Using one

1. Put the file here
2. In the table of `scripts/site-audio.sh`, write its name in the last column of the example
   it is for. For the pitch shifter and the drums the render's name changes too
   (`input_<shift>.wav`, `drums_<kit>_input.wav`); the comment above the table says how
3. Run `scripts/site-audio.sh <plugin>` and listen to the MP3 in `site/audio/`
4. A new example also needs its line in `site/index.html`

## Files

None yet: every example on the page is played by its render test.
