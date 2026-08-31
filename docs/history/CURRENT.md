# Current state

> **Read this first.** `SHIPPED.md` says what is done; this says what is *in
> flight*, what is blocked, and what to pick up next. Keep it short — a page
> that grows stops being read, and an unread status page is how
> `docs/html/` drifted ten milestones before anyone noticed.
>
> Last updated: 2026-08-30.

## Where the branch is

`main` — all work merged, nothing in flight, working tree clean. Local commits
are **not pushed**; pushing is a deliberate act (`.githooks/pre-push` runs
fmt-check + clippy + docs-check + the suite first).

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

**M26c — rip recognition. In progress.** The last M26 sub-milestone and the
only thing between a rip and a fully-tagged record: AcoustID (a TEST2
fingerprint computed transiently — the stored dedupe blobs are TEST1) →
MusicBrainz release tracklist → Discogs enrichment. It is **Dub's first network
dependency**, confined to the `Http` trait in the `dub-recognize` leaf crate so
rips keep working fully offline.

Done: the crate — TEST2 fingerprint in AcoustID's wire format, both clients,
and the side-level release vote — plus `dub recognize <session-dir | side.wav>`,
validated end to end against the live services and against all three baseline
records. **Naming is the primary path and needs no MusicBrainz**; identifying
the pressing is opt-in (`--album`). Committed rips also carry `BPM` and
`INITIALKEY` in their tags now, not just in the catalog.

**The key is not in the repo and must not be.** `dub recognize` reads
`$DUB_ACOUSTID_KEY` or takes `--key`; register a free one at
acoustid.org/new-application. Without it every lookup returns
`MissingCredential` deliberately, so the failure names the cause instead of
looking like an unknown record.

Done since: **Discogs enrichment** (through MusicBrainz's curated link, never
a fuzzy search) and the **FFI + Swift wiring** — `Identify` / `Use these` in
the rip review panel, backed by a background worker polled like the commit
job, with the key and options in Preferences.

Remaining:
- **R-42 — the Discogs token in the Keychain.** No longer a blocker: the
  AcoustID key is an *application* key and needs no Keychain, so recognition
  ships without one and Discogs stays optional. The token sits in
  `UserDefaults` until a helper exists.
- **R-49 — sample lineage link-out** (WhoSampled). New; see PRD §5.2.5a.
- **A better tie-break for same-artist pressings.** A foreign-language
  pressing credited to the same artist ties with the domestic one all the way
  down (record B's "TSOP" comes back with its Japanese title). The artist
  consensus cannot separate those; a script-coherence heuristic could.

**M11f (export) is shipped** — `dub export --rekordbox | --m3u8`, the
anti-lock-in commitment. Not yet surfaced in the app: PRD §8.6 calls for a
one-click `File → Export Crate As...` with the format as a dropdown, and today
it is CLI-only.

Alternatives on the roadmap: **M11d-columns** (browser column data plumbing,
2–3 days), **M17** (sampler / quick scratch / instant doubles, 4–6 days).
See `../spec/PRD.md` §12.1.

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
- The tests that sleep waiting on worker threads are now the slowest thing in
  the suite. `drain_then_stop` fixed the six that raced; others still sleep.

## Keeping this file honest

Update it in the same change that ships a milestone or opens a gap — the same
rule as `SHIPPED.md`. If a section here is longer than a screen, it belongs in
`UI-BACKLOG.md` or the PRD, and this file should link to it instead.
