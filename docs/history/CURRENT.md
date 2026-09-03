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

**M18 — Polish + Alpha** (2–3 weeks). M17 was the last feature milestone;
what remains before trusted-DJ hands is polish, and it carries real
deferred work rather than only cosmetics:

- **Key remapping** — and with it the M17 keymaps. `⌘←→` (instant doubles)
  and `Q W E R` (Quick Scratch) are bound but fixed; the sampler's
  `A S D F` are **not bound at all** and its rack is driven from
  Preferences until they are.
- **The deferred M16 fine-tuning**: siren sound polish (GS1 shots / DS01E
  tones / SN76477 bank) and the deck-B siren Expert panel
  (`UI-BACKLOG.md` §5 F-36 / F-37). Note F-37's Performance half is moot:
  the siren is one rack in the global bar now, bound to the focused deck.
- Calibration UX, preferences, dark-mode polish, and the manual rig
  checklist.

### The layout pass — shipped

Both surfaces were misallocating space, and Performance was hiding a
shipped feature. The per-deck pad column needed ~478 pt of a ~330 pt
pane, spilled ~72 pt off each end (centre-aligned, nothing clipped), and
the bottom spill was painted over by the opaque `FXBarPlaceholder`
declared after it — so M17's sampler pads rendered sliced in half.

What changed:

- **`GlobalRackBar`** replaces the placeholder bar: one siren (bound to
  the focused deck, with a `→ A` / `→ B` pill), one Quick Scratch, one
  sampler. These were never per-deck — there is a single siren keymap
  and the trigger racks are four-slot tables whose slots carry their own
  target deck. The deck columns keep CUE / LOOP / ECHO OUT and come to
  218 pt.
- **Width flipped.** The waveform was a hard 200 pt while the pads were
  `maxWidth: .infinity`; now the column is a fixed 224 and the strip
  grows to a 280 cap.
- **Prep is a three-column grid** instead of one narrow gutter with 70 %
  of the window empty. The sections behind both surfaces are shared, so
  the `cueGroup()`/`CuePadRow` duplication (a live PRD §3.1 violation) is
  gone.
- **A draggable deck/library divider**, persisted per mode, deck-heavy in
  Performance per PRD §9.2. It is also the structural fix: both halves
  get explicit heights that sum to the space available, so nothing can
  overflow its slot and be drawn over. That meant deleting the
  `minHeight` on the waveform region *and* on `LibraryView`.

**Gap this closed in the test suite.** `pads-deck-a` rendered at a
hand-picked 560 × 360 with `sirenEnabled` defaulting to *false* — green
on a configuration nobody runs. More fundamentally a snapshot forces
`.frame(width:height:)`, so it can never catch an overflow; it just
renders the smaller thing correctly. `PerformanceLayoutTests` asserts fit
via `NSHostingView.fittingSize` instead, and baselines are now framed
from layout tokens so they move when the layout does.

Still open: `SirenExpertPanel`, `KeyLockControlView` and `PitchTestView`
take the model directly, so Prep's grid has no snapshot coverage. Making
them value-driven (the `PrepRipBarState` pattern) is what unlocks it.

**M12-lexicon** (0.5 day, docs only) is the other open row: document the
Lexicon → Serato / rekordbox / Traktor export paths in
`LIBRARY-FORMATS.md`.

## Recently shipped (detail in `SHIPPED.md`)

- **M17 — Sampler, Quick Scratch & Instant Doubles.** All three of §7's
  trigger mechanisms. **Instant Doubles** duplicates a deck's track onto
  the other at a sample-accurate playhead — done *on the audio thread* off
  the loaded `Arc<Track>`, which is what makes it sample-accurate and
  instant. **Quick Scratch** loads a bound sample through the library's own
  load path. **The sampler** is four additive one-shot voices summed onto
  the assigned deck bus after the FX, reading rate-converted buffers with
  an integer cursor because the conversion happens at bind time off-RT.
  Both racks bind from one shared sample bank. FFI 66.

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
