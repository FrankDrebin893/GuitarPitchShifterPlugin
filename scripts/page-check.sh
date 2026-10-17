#!/usr/bin/env bash
# The page (site/) against the work of a session: whether /page has looked at what changed,
# and whether the page's own links hold. Used by the /page skill and by the end-of-session
# hook of any agent tool (for Claude Code: .claude/settings.json).
#
#   scripts/page-check.sh            lists what changed since main that the page may describe;
#                                    exits 1 if /page has not looked at it yet, 0 if it has or
#                                    nothing changed
#   scripts/page-check.sh mark       records that /page has looked at what changed as it is now
#   scripts/page-check.sh links      every local link, image, sound and anchor of the page is
#                                    there, and every download is a zip a release makes
#   scripts/page-check.sh hook       the Claude Code Stop hook: asks for /page once if needed
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# What a player can notice, or what the page links to.
paths=(plugins crates CHANGELOG.md bundler.toml Cargo.toml .github/ISSUE_TEMPLATE NOTICE.md LICENSE site)

base() {
    local main=main
    git rev-parse -q --verify origin/main > /dev/null && main=origin/main
    git merge-base HEAD "$main" 2> /dev/null || git rev-parse HEAD
}

# Files changed since main (committed or not, and new files) under the paths above.
changed() {
    {
        git diff --name-only "$(base)" -- "${paths[@]}" 2> /dev/null
        git ls-files --others --exclude-standard -- "${paths[@]}"
    } | sort -u
}

# One hash for the changes as they are now, so a later edit asks for /page again.
fingerprint() {
    {
        git diff "$(base)" -- "${paths[@]}" 2> /dev/null
        git ls-files --others --exclude-standard -- "${paths[@]}" | while read -r f; do
            echo "$f"
            git hash-object "$f"
        done
    } | git hash-object --stdin
}

marker="$(git rev-parse --git-path page-checked)"

status() {
    local files
    files=$(changed)
    if [ -z "$files" ]; then
        echo "Nothing changed that the page describes."
        return 0
    fi
    if [ -f "$marker" ] && [ "$(cat "$marker")" = "$(fingerprint)" ]; then
        echo "/page has looked at these changes."
        return 0
    fi
    echo "Changed since main, not yet looked at by /page:"
    echo "$files" | sed 's/^/  /'
    return 1
}

links() {
    local page=site/index.html bad=0 ref file anchor zip
    for ref in $(grep -oE '(href|src)="[^"]*"' "$page" | sed -E 's/^(href|src)="//; s/"$//' | sort -u); do
        case "$ref" in
            http://* | https://* | mailto:*)
                case "$ref" in
                    */releases/latest/download/*)
                        zip=${ref##*/}
                        if ! grep -q "\"${zip%-*}\"" bundler.toml || ! [[ $zip =~ -(windows|macos|linux)\.zip$ ]]; then
                            echo "download $zip is not a zip a release makes (bundler.toml, scripts/package.sh)"
                            bad=1
                        fi
                        ;;
                esac
                ;;
            \#*)
                anchor=${ref#\#}
                if ! grep -q "id=\"$anchor\"" "$page"; then
                    echo "anchor #$anchor has no element with that id"
                    bad=1
                fi
                ;;
            *)
                file=site/${ref%%#*}
                if [ ! -e "$file" ]; then
                    echo "$ref is not in site/"
                    bad=1
                fi
                ;;
        esac
    done
    [ $bad = 0 ] && echo "Every link of the page holds."
    return $bad
}

case "${1:-}" in
    "") status ;;
    mark)
        fingerprint > "$marker"
        echo "Marked: /page has looked at the changes as they are now."
        ;;
    links) links ;;
    hook)
        input=$(cat)
        # Asked once already in this stop: let the session end, so a hook never loops.
        if echo "$input" | grep -qE '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
            exit 0
        fi
        if ! out=$(status); then
            reason="This session changed what the page (site/) may describe. Before ending, run the /page skill: bring site/index.html up to date with what is released, make sure CHANGELOG.md says what is not, and finish with scripts/page-check.sh mark. ${out//$'\n'/ }"
            reason=${reason//\\/\\\\}
            reason=${reason//\"/\\\"}
            printf '{"decision": "block", "reason": "%s"}\n' "$reason"
        fi
        exit 0
        ;;
    *)
        echo "usage: scripts/page-check.sh [mark|links|hook]" >&2
        exit 2
        ;;
esac
