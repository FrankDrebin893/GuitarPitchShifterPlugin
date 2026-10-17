---
name: page
description: Bring the public page (site/) up to date with the work of a session, or with a release. Use at the end of every session that changed something a player can notice, and from /release.
argument-hint: [release <version>]
---

# /page

Keep https://hojte.net/hojt-audio/ (`site/index.html`, published from `main` by `pages.yml`)
true to what a player can download. The rules are "Releasing" in AGENTS.md; the ones that
decide this skill:

- **The page describes the latest release.** It is published the moment a branch merges, and
  the downloads are `releases/latest`, so something not released yet does not go on the page:
  not a plugin, a control, a kit or a sound. It goes on the page in the commit of the release
  that ships it (`/page release`). Until then `CHANGELOG.md` under `Unreleased` carries it
- `site/audio/` and `site/img/` change only in the commit of a release. Sounds come from
  `scripts/site-audio.sh`, screenshots are taken by hand: never make either up
- The house voice: what is heard and seen, short plain sentences, no internals, no real makers'
  names or trademarks. Body text never sits on the paint

Without an argument, run the **end of session** steps. With `release <version>`, the
**release** steps. Both end with the **checks**.

## End of session

1. **What changed.** `scripts/page-check.sh` lists the files changed since `main` that the page
   may describe. If it says nothing changed, go to the checks. Otherwise read the diff of
   those files and `CHANGELOG.md` under `Unreleased`.
2. **Sort** every change a player can notice:
   - **True of the release now:** the page is wrong or missing something about what can be
     downloaded today (a fact, a range, a link, an install step, an issue template that was
     renamed, a fix to the page itself). Edit `site/index.html` now.
   - **Not released yet:** leave the page alone. Make sure `CHANGELOG.md` under `Unreleased`
     says it in the house voice and gives what the page will need at release: the names of new
     controls with their ranges, new kits, voices or MIDI notes, a new plugin's model number,
     paint and one-line description.
   - **Nothing a player notices** (tests, refactors, tools, the harness): nothing to do.
3. **Edit** with the page as it is: the same markup per plugin section, the same classes, the
   same order of headings. Keep the counts in the header, `<title>` and the meta and Open
   Graph descriptions right ("Three plugins") when a section is added or removed.
4. Go to the checks.

## Release

Called from `/release` after the changelog section `## <version>` is written and before the
gates. The page must describe exactly that version.

1. **Walk the section.** For every plugin in `## <version>` of `CHANGELOG.md`, and every plugin
   in `bundler.toml`, check the page against the code:
   - **A plugin not on the page** gets a section like the others: `section.plugin.<paint>` with
     the paint band, a caption (`<b>model number</b> / Effect or Instrument / what goes in and
     out`, from its `PedalStyle` and jack captions), What it is, Listen, Download (the three
     zips, named by `bundler.toml`), and a Controls table. A jump link in the header, in the
     order of the sections. Its paint as `--paint-<name>` and a section class in
     `site/style.css`, taken from its `PAINT_*` in `crates/suite_common/src/ui/theme.rs`
   - **Controls tables** match the plugin's parameters in `plugins/<name>/src/lib.rs`: every
     control a player sees, named as the window names it, with the range the parameter really
     has. MIDI tables match the note map in the plugin's `AGENTS.md`
   - **Points and counts** (kits, voices, pedals, presets, latency) match the code
   - **Listen** lists exactly the examples `scripts/site-audio.sh` makes for that plugin, with
     their files in `site/audio/`. A plugin with none yet has no Listen heading
2. **Screenshots.** A new plugin, or one whose look changed, needs a screenshot in `site/img/`
   (the standalone window at its own size, taken by hand). If it is missing, leave the figure
   out and list it for Rasmus; never draw or generate one.
3. Go to the checks.

## Checks

1. `scripts/page-check.sh links`: every local link, image, sound, anchor and download holds.
   Fix what it reports.
2. If the page changed, open `site/index.html` in a browser (Claude Code: the preview,
   `preview_start` with its `file://` URL) at desktop and phone width and look at what changed.
3. `scripts/page-check.sh mark`, last, so the end-of-session hook knows the page was looked at
   for the changes as they are now. A later edit asks for `/page` again.
4. Say in one or two lines what changed on the page, what waits in the changelog for the
   release, and any screenshot Rasmus has to take. Commit page edits with the session's work;
   on `main` itself, ask first.
