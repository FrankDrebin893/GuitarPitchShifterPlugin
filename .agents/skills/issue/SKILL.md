---
name: issue
description: File a backlog issue from chat, the same way the GitHub forms do. Use when filing a feature, bug, or sound report issue.
argument-hint: <what you want, in your own words>
---

# /issue

File one issue in FrankDrebin893/hojt-audio from what Rasmus said: $ARGUMENTS

1. **Kind.** Pick the form it fits: `feature` (something new), `bug` (something does not
   work) or `sound` (how something sounds). Pick the plugin: `amp`, `drums`,
   `pitch-shifter`, `bass`, `pad`, `bells`, `common` (shared look or DSP), or the area `site`
   or `infra`. If either is unclear, ask.
2. **Already there?** `gh issue list --state open --search "<key words>"`. If an open issue
   covers it, say which and ask whether to add a comment there instead.
3. **Title.** Short, what a player would say. Start with the plugin when it is not the amp,
   as the backlog does (`Drums: hi-hat openness from CC4`). No trailing full stop.
4. **Body.** The headings of the matching form in `.github/ISSUE_TEMPLATE/`, each as
   `### <label>` with the answer below it, in the order of the form. Only what Rasmus said or
   what is plainly true of the code; write `_No response_` for an optional heading he did
   not answer. Do not write a spec: that is `/spec`.
   - feature: Which plugin, What it should do, Why
   - bug: Which plugin, Version, system and host, What you did, What happened, and what you expected
   - sound: Which plugin, Settings, What you hear, and what you would like to hear, Recording or project (optional)
5. **File it** with labels `kind:<kind>` and `plugin:<plugin>` (or `area:<area>`):
   `gh issue create --title "..." --label kind:...,plugin:... --body-file <file>`, then
   `scripts/move-issue.sh <n> idea`, which sets `state:idea` and puts it on the board.
6. Reply with the link and one line on what it says. Do not start on it.
