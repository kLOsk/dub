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

Remaining:

- **Discogs enrichment — blocked on R-42**, the Keychain token. There is no
  Keychain plumbing anywhere in the app yet, so that is where it starts. The
  same store should take over the AcoustID key once it exists; the env var is
  the CLI's answer, not the app's.
- **Wiring the result into the rip review panel** (FFI + Swift). The
  Picard-convention tag fields are already written by `dub-encode`; they are
  simply never populated.
- **A better tie-break for same-artist pressings.** A foreign-language
  pressing credited to the same artist ties with the domestic one all the way
  down (record B's "TSOP" comes back with its Japanese title). The artist
  consensus cannot separate those; a script-coherence heuristic could.

Alternatives on the roadmap if M26c is not the appetite: **M11d-columns**
(browser column data plumbing, 2–3 days), **M11f** (export: rekordbox XML +
M3U8, 3 days), **M17** (sampler / quick scratch / instant doubles, 4–6 days).
See `../spec/PRD.md` §12.1.

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
