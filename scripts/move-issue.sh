#!/usr/bin/env bash
# Moves a backlog issue to a state: its state:* label and its State on the board
# "HØJT backlog" (added to the board if it is not there). Never to approved: that is Rasmus's.
# Usage: scripts/move-issue.sh <issue> <idea|spec|building|review>
set -euo pipefail

issue=$1
state=$2
case "$state" in
    idea | spec | building | review) ;;
    *) echo "state must be idea, spec, building or review" >&2; exit 1 ;;
esac

owner=FrankDrebin893
repo=$owner/hojt-audio
board=2

old=$(gh issue view "$issue" --repo "$repo" --json labels --jq '[.labels[].name | select(startswith("state:"))] | join(",")')
gh issue edit "$issue" --repo "$repo" ${old:+--remove-label "$old"} --add-label "state:$state" > /dev/null

name=$(echo "${state:0:1}" | tr a-z A-Z)${state:1}
project=$(gh project view $board --owner $owner --format json --jq .id)
fields=$(gh project field-list $board --owner $owner --format json --jq '.fields[] | select(.name=="State") | .id, (.options[] | "\(.name) \(.id)")')
field_id=$(echo "$fields" | head -1)
option=$(echo "$fields" | sed -n "s/^$name //p")
item=$(gh project item-add $board --owner $owner --url "https://github.com/$repo/issues/$issue" --format json --jq .id)
gh project item-edit --id "$item" --project-id "$project" --field-id "$field_id" --single-select-option-id "$option" > /dev/null
echo "#$issue is $state"
