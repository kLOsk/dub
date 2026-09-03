# Current state

> **Read this first.** `SHIPPED.md` says what is done; this says what is *in
> flight*, what is blocked, and what to pick up next. Keep it short — a page
> that grows stops being read, and an unread status page is how
> `docs/html/` drifted ten milestones before anyone noticed.
>
> Last updated: 2026-09-03.

## Where the branch is

`main` — all work merged and pushed, nothing in flight, working tree clean.
Pushing is a deliberate act: `.githooks/pre-push` runs fmt-check + clippy +
docs-check + the Rust suite + the Swift snapshot suite first, and that last
gate is local-only because GitHub CI has no macOS app job.

## Owed before v1: validate on the rig

Two things are implemented, tested and green — **against synthetic signals
only**. `LESSONS.md` is explicit that timecode and vinyl behaviour are
confirmed on hardware, and neither has been.

0. ~~Timecode decode against real vinyl~~ — **done**: `testdata/timecode/`
   holds three SL 3 excerpts driving `real_vinyl_tests.rs`. Still uncovered
   there: a stalled platter, a mid-play lift, pitch extremes, and *both*
   Traktor formats. See that README.
1. **Loops under a real needle.** Acceptance §14 #8 is met on a synthetic
   Serato CV02 carrier. Not tried: scratching inside a loop, lifting the needle
   mid-loop, re-locking after a lift, key lock at a pitched platter.
2. **A rip end to end through the app.** Every gate is fitted against the three
   captures in `testdata/rip-baselines/` and replayed offline with
   `dub rip-tune`. The trim brackets, the Real Records node and re-split have
   never been driven on the rig with a record on the platter.

## Next milestone

**M17 — Sampler, Quick Scratch & Instant Doubles** (PRD §7, 4–6 days). The
last feature milestone before the polish ladder. Three mechanisms, very
unevenly priced:

1. **Instant Doubles** (§7.3) — `Cmd+→` / `Cmd+←` duplicate the loaded track
   to the other deck at a sample-accurate position. Both decks and a load
   path already exist; this is wiring plus a rebindable hotkey.
2. **Quick Scratch** (§7.2) — `Q W E R` load a bound sample to a target deck.
   The PRD is explicit that it shares the library drag-and-drop load path, so
   it is slot persistence, a binding UI and a keymap, not new audio work.
3. **Sampler** (§7.1) — the real engine work. Four one-shot voices
   (`A S D F`) mixed **additively** into the deck's output bus *post-FX*,
   each with gain and output assignment. A new RT-safe mixing path, and where
   the risk sits.

That order gets something dogfoodable on the rig first and leaves the piece
needing careful RT work with room.

## Recently shipped (detail in `SHIPPED.md`)

- **M26c — rip recognition. Complete.** AcoustID naming (no MusicBrainz on
  the common path), opt-in pressing identification, Discogs enrichment
  through MusicBrainz's curated link, `Identify` / `Use these` in the rip
  review panel, the Discogs token in the login Keychain (R-42), and the
  WhoSampled sample-lineage link-out (R-49).
- **M11f — export.** `dub export --rekordbox | --m3u8`, plus **File → Export
  Library As…** (⇧⌘E) and **Export As…** on a crate. PRD §8.6's anti-lock-in
  commitment reaching the app.
- **M11d-columns — browser column data plumbing.** A stable `LibraryColumnId`
  registry with the track SELECT assembled from the active set, so a
  switched-off column contributes no expression and no join. FFI 64.

**The key is not in the repo and must not be.** `dub recognize` reads
`$DUB_ACOUSTID_KEY` or takes `--key`; register a free one at
acoustid.org/new-application. Without it every lookup returns
`MissingCredential` deliberately, so the failure names the cause instead of
looking like an unknown record.

## Licence posture is now enforced, not just documented

Dub is **MIT OR Apache-2.0**. `make deny` (in `make ci`, and its own CI job)
fails the build on any dependency licence not on `deny.toml`'s allow-list, and
bans the four copyleft FFIs that were each deliberately routed around
(rubberband, aubio, chromaprint, mp3lame) by name and with a reason. Before the
relicense a copyleft dep cost nothing — the workspace already declared GPL —
so this guard is new risk cover, not tidying.

`make attribution` generates `apple/Dub/Resources/Acknowledgments.html` from
the real build graph and the About panel links to it, which retires the
"How to ship attribution" to-do. Regenerate it when dependencies change;
nothing fails yet if it goes stale.

RUSTSEC advisories are `make audit`, deliberately **not** in `ci`: they need
the network, and an upstream publication would turn an unrelated push red.

## Known gaps worth naming

- **P-40** — the FFI position extrapolator does not wrap into the loop region,
  so the UI playhead reads about a block past `loop_out`. Sub-frame normally.
- **R-41** — the deck pane shows its idle placeholder during a rip capture;
  only the overview band renders live signal.
- **R-43** — ripping is Prep-only by design; Performance-mode ripping is
  deliberately deferred past M26c.
- **Same-artist pressing tie-break** (M26c, closed as a limitation rather
  than a blocker). A foreign-language pressing credited to the same artist
  ties with the domestic one all the way down — record B's "TSOP" comes back
  with its Japanese title. The artist consensus cannot separate those; a
  script-coherence heuristic could, and that is a research task, not a
  milestone. Only bites `--album` / "identify the pressing"; naming is
  unaffected.
- The tests that sleep waiting on worker threads are now the slowest thing in
  the suite. `drain_then_stop` fixed the six that raced; others still sleep.

## Keeping this file honest

Update it in the same change that ships a milestone or opens a gap — the same
rule as `SHIPPED.md`. If a section here is longer than a screen, it belongs in
`UI-BACKLOG.md` or the PRD, and this file should link to it instead.
