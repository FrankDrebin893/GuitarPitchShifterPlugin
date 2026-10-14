---
description: Prepare a release of the suite; stops before the tag
argument-hint: <version, e.g. 0.4.0>
---

Prepare release $ARGUMENTS by "Releasing" in CLAUDE.md. Work on `main`, up to date, with a
clean tree; if it is not, stop and say so.

1. **What is in it.** The last tag: `git describe --tags --abbrev=0`. The pull requests merged
   since: `gh pr list --state merged --base main --search "merged:>=<date of that tag>" --json number,title,labels`.
   Those with `kind:sound`, and those whose `sound.yml` comment shows a change, decide which
   plugins' sound changed
2. **Version.** Set `version` in `[workspace.package]` of `Cargo.toml` to $ARGUMENTS and run
   `cargo check --workspace` so `Cargo.lock` follows
3. **Changelog.** Turn `## Unreleased` into `## $ARGUMENTS`. Check it against the pull
   requests: everything a player will notice is there, in the house voice (what is heard and
   seen, no internals), grouped by plugin. Add what is missing; ask Rasmus if unsure
4. **Sound examples.** For each plugin whose sound changed, `scripts/site-audio.sh <plugin>`,
   then list which files in `site/audio/` changed for Rasmus to listen to. List the plugins
   whose look changed: their screenshot in `site/img/` is taken by hand
5. **Gates.** `cargo test --workspace`, `cargo test -p amp --release`, `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`
6. **Stop.** Show the changelog section, the changed sound examples and the commit you would
   make. Commit, tag `v$ARGUMENTS` and push only after Rasmus says yes in chat. Then the
   `Release` workflow publishes; give him its run link
