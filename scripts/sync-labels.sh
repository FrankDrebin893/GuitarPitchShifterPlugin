#!/usr/bin/env bash
# Creates or updates the backlog's labels on GitHub. Safe to run again.
# Usage: scripts/sync-labels.sh
set -euo pipefail

# name|colour|description
labels='
state:idea|ededed|Not yet specified or approved
state:spec|c5def5|A spec is on the issue; waits for Rasmus
state:approved|0e8a16|Rasmus said yes: ready to build
state:building|fbca04|Being built on a branch
state:review|5319e7|A pull request is open
from:user|f9d0c4|Came from someone outside the project
kind:feature|a2eeef|Something new
kind:bug|d73a4a|Something does not work
kind:sound|d4a017|How something sounds
kind:look|bfd4f2|How something looks
plugin:amp|7a1f1f|Guitar Amp
plugin:drums|1f6f6f|Drum Synth
plugin:pitch-shifter|d9822b|Pitch Shifter
plugin:bass|2a4fa0|Bass Synth
plugin:pad|5e2a5e|Shimmer Pad
plugin:bells|1f5a3a|Glass Bells
plugin:common|555555|suite_common: shared look and DSP
area:site|0075ca|The public page
area:infra|333333|CI, releases, tooling
'

while IFS='|' read -r name colour description; do
    [ -z "$name" ] && continue
    gh label create "$name" --color "$colour" --description "$description" --force
done <<< "$labels"
