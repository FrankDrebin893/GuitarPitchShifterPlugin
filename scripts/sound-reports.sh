#!/usr/bin/env bash
# Runs the reports that describe how the plugins sound, without the lines that time them,
# into <out>/<plugin>_<report>.txt. Run from the root of a checkout. With --renders, the
# render tests of the plugins named after it write their WAV files to target/renders too.
# Usage: scripts/sound-reports.sh <out> [--renders <plugin>...]
set -uo pipefail

out=$1
shift
mkdir -p "$out"

reports='
amp fingerprint_report
amp loudness_report
amp preset_report
amp preset_gate_report
amp effects_report
amp comp_report
amp mods_report
drums kit_report
bass bass_report
pad pad_report
bells bells_report
pitch_shifter quality_report
'

while read -r plugin report; do
    [ -z "$plugin" ] && continue
    file="$out/${plugin}_${report}.txt"
    # Only what the report prints (stdout): cargo's build output and warnings go to stderr and
    # carry source line numbers, which move with any edit
    if cargo test -q -p "$plugin" --release --locked "$report" -- --ignored --nocapture > "$file.raw" 2>/dev/null; then
        # Timings differ from run to run and machine to machine: leave them out
        # (the cost tables of effects_report and the ns columns of comp_report)
        grep -vE 'of a core|ns per|us per|µs|finished in|^running [0-9]|^$| [0-9.]+ ns( |$)' "$file.raw"             | sed '/^Cost per block/,$d' > "$file"
    else
        echo "(this report failed or does not exist here)" > "$file"
    fi
    rm -f "$file.raw"
done <<< "$reports"

if [ "${1:-}" = "--renders" ]; then
    shift
    for plugin in "$@"; do
        cargo test -q -p "$plugin" --release --locked render_wavs -- --ignored > /dev/null 2>&1 || true
    done
fi
