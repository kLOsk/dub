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

1. **Loops under a real needle.** Acceptance §14 #8 is met on a synthetic
   Serato CV02 carrier. Not tried: scratching inside a loop, lifting the needle
   mid-loop, re-locking after a lift, key lock at a pitched platter.
2. **A rip end to end through the app.** Every gate is fitted against the three
   captures in `testdata/rip-baselines/` and replayed offline with
   `dub rip-tune`. The trim brackets, the Real Records node and re-split have
   never been driven on the rig with a record on the platter.

## Next milestone

**M26c — rip recognition.** The last M26 sub-milestone and the only thing
between a rip and a fully-tagged record. AcoustID (a TEST2 fingerprint computed
transiently — the stored dedupe blobs are TEST1) → MusicBrainz release
tracklist → Discogs enrichment, in a new `dub-recognize` leaf crate behind an
`Http` trait so rips keep working fully offline. The Picard-convention tag
fields are already written by `dub-encode`; they are simply never populated.

Two things to know before starting: it is **Dub's first network dependency**,
and **R-42 blocks the Discogs half** — the token needs Keychain storage and
there is no Keychain plumbing anywhere in the app yet. Start there.

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
