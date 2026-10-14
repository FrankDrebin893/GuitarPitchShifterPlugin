#!/usr/bin/env bash
# Makes the page's sound examples (site/audio) with the plugins as they are in this checkout:
# runs the render tests and turns their WAV files into MP3s. Part of releasing, so the page
# sounds like the download. Needs ffmpeg.
#
#   scripts/site-audio.sh [drums] [amp] [pitch_shifter] [bass] [bells] [pad]     none means all
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
renders="$root/target/renders"
audio="$root/site/audio"
sources="$root/site-sources"

# Every example on the page: what it is made from.
#
#   file in site/audio | plugin | render test | file in target/renders | performance
#
# The performance is what is played: - for the one the test makes up, or a file in
# site-sources (a DI recording for amp and pitch_shifter, a MIDI file for drums). With a
# performance of your own the pitch shifter writes input_<shift>.wav and the drums
# drums_<kit>_input.wav; the amp keeps its names. A * in a render's name stands for the part
# that is not ASCII.
examples='
drums-rock.mp3|drums|render_wavs|drums_rock_groove.wav|-
drums-jazz.mp3|drums|render_wavs|drums_jazz_groove.wav|-
drums-metal.mp3|drums|render_wavs|drums_metal_groove.wav|-
drums-groove.mp3|drums|render_wavs|drums_groove_groove.wav|-
amp-dry.mp3|amp|render_wavs|amp_dry.wav|-
amp-klar.mp3|amp|render_wavs|amp_klar_gain5.wav|-
amp-brol.mp3|amp|render_wavs|amp_brol_gain7.wav|-
amp-torden.mp3|amp|render_wavs|amp_torden_gain7.wav|-
amp-torden-lead.mp3|amp|render_wavs|amp_torden_gain7_lead_delay.wav|-
amp-preset-taage.mp3|amp|render_presets|amp_preset_03_*.wav|-
pitch-notes-original.mp3|pitch_shifter|render_wavs|notes_original.wav|-
pitch-notes-down2.mp3|pitch_shifter|render_wavs|notes_-2.wav|-
pitch-notes-up7.mp3|pitch_shifter|render_wavs|notes_+7.wav|-
pitch-chord-original.mp3|pitch_shifter|render_wavs|chord_original.wav|-
pitch-chord-down2.mp3|pitch_shifter|render_wavs|chord_-2.wav|-
bass-metal.mp3|bass|render_wavs|bass_metal.wav|-
bass-groove.mp3|bass|render_wavs|bass_groove.wav|-
bells.mp3|bells|render_wavs|bells.wav|-
pad.mp3|pad|render_wavs|pad.wav|-
'

# The variable a plugin's render tests read their performance from. Bass, bells and pad
# only play what their tests make up: theirs is a name nothing reads
input_variable() {
  case "$1" in
    drums) echo DRUMS_INPUT_MID ;;
    amp) echo AMP_INPUT_WAV ;;
    pitch_shifter) echo PITCH_SHIFTER_INPUT_WAV ;;
    bass | bells | pad) echo HOJT_NO_PERFORMANCE ;;
  esac
}

if [ $# -eq 0 ]; then
  set -- drums amp pitch_shifter bass bells pad
fi
for plugin in "$@"; do
  if [ -z "$(input_variable "$plugin")" ]; then
    echo "unknown plugin: $plugin (drums, amp, pitch_shifter, bass, bells or pad)" >&2
    exit 1
  fi
done

if ! command -v ffmpeg > /dev/null; then
  echo "ffmpeg is not on the path" >&2
  exit 1
fi

# One run of a render test serves every example with the same plugin, test and performance
runs=$(
  for plugin in "$@"; do
    echo "$examples" | awk -F'|' -v plugin="$plugin" '$2 == plugin { print $2 "|" $3 "|" $5 }'
  done | sort -u
)

for run in $runs; do
  IFS='|' read -r plugin test performance <<< "$run"
  rows=$(echo "$examples" | awk -F'|' -v plugin="$plugin" -v test="$test" -v performance="$performance" \
    '$2 == plugin && $3 == test && $5 == performance { print $1 "|" $4 }')

  # Out with what an earlier run left, so that a render the test no longer writes is missed
  for row in $rows; do
    rm -f "$renders"/${row#*|}
  done

  variable="$(input_variable "$plugin")"
  if [ "$performance" = "-" ]; then
    echo "$plugin $test"
    (unset "$variable" && cd "$root" && cargo test -q -p "$plugin" --release "$test" -- --ignored > /dev/null)
  else
    input="$sources/$performance"
    if [ ! -f "$input" ]; then
      echo "missing $input" >&2
      exit 1
    fi
    if command -v cygpath > /dev/null; then
      input="$(cygpath -m "$input")"
    fi
    echo "$plugin $test, played by $performance"
    (export "$variable=$input" && cd "$root" && cargo test -q -p "$plugin" --release "$test" -- --ignored > /dev/null)
  fi

  for row in $rows; do
    name="${row%%|*}"
    wav=$(ls "$renders"/${row#*|} 2> /dev/null || true)
    if [ ! -f "$wav" ]; then
      echo "$test of $plugin did not write ${row#*|} for $name" >&2
      exit 1
    fi
    ffmpeg -v error -y -i "$wav" -codec:a libmp3lame -q:a 4 "$audio/$name"
    echo "  $name"
  done
done
