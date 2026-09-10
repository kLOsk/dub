# Current state

> **Read this first.** `SHIPPED.md` says what is done; this says what is *in
> flight*, what is blocked, and what to pick up next. Keep it short — a page
> that grows stops being read, and an unread status page is how
> `docs/html/` drifted ten milestones before anyone noticed.
>
> Last updated: 2026-09-08.

## Where the branch is

`main` — the library's `NSTableView` migration (b949f0d…8d6335c) plus its
follow-up cleanup are committed locally and **unpushed**. Nothing in flight.
Pushing is a deliberate act: `.githooks/pre-push` runs fmt-check + clippy +
docs-check + the Rust suite + the Swift snapshot suite first, and that last
gate is local-only because GitHub CI has no macOS app job.

### The migration's cleanup pass — done

The hand-rolled `LazyVStack`-in-an-`NSHostingView` the table replaced is
gone: the scroll container and its coordinator, the document wrapper, the
AppKit selection layer, the arrow-key view, the whole SwiftUI column
resize / reorder / header stack, and `handleRowClick` / `selectRange`
(`NSTableView` does click, ⇧-range and ⌘-toggle natively). ~2,100 lines.

Three things the migration had quietly dropped, found while deleting and
now fixed:

- **The column picker had no entry point.** It was the old SwiftUI header's
  `.contextMenu`, and `LibraryTableHeaderView` had no menu — so there was no
  way to add or remove a column, or to switch key notation, at all. Now
  `LibraryColumnMenu`, an AppKit builder in the same shape as
  `LibraryRowMenu` (SwiftUI builds menu content eagerly, and this menu
  enumerates the whole registry behind "More columns").
- **Key notation relabelled the header and not the cells** — see the
  `contentRevision` entry in `LESSONS.md`.
- **Reveal selected but never scrolled.** `keyboardScrollTarget` lost its
  consumer; it is wired to `LibraryTable.Coordinator.scrollToTrack(id:)` now.

Test coverage went 131 → 166: `LibraryColumnMenuTests`, `LibraryRowMenuTests`
(whose file `LibraryRowMenu.swift` had claimed to exist since the migration
— it did not), and the row snapshots repointed off a test-only replica onto
the production `LibraryTintedRowView` + `LibraryHostingCellView` assembly,
with two new baselines covering selection-under-tint.

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

## The Prep UI overhaul — designed, queued

The Prep control surface was reviewed end to end and redesigned. Nothing is
built; this is the sequence and the decisions it waits on. Four design
artifacts exist (private, on claude.ai/code/artifact): the pad-surface
studies, the **Prep Rack** overhaul, the **Surface Map**, and **Map Mode**.

**What the review found.** The measurements are in the artifacts; the ones
that matter here:

- Unlit pad labels are `textTertiary` on `surface1` — **3.64:1**, under the
  4.5:1 floor. Key bindings, which are the actual performance input path,
  render in `textPlaceholder` at **2.20:1**. A caller-side `.opacity(0.5)`
  disabled pad lands near 1.84:1.
- `DubPadCell` carries one `lit: Bool`, so "cue is set", "loop is running"
  and "you are pressing it" all render identically.
- `prepPadGridIntrinsicWidth` still sums **three** columns (912) while
  `PrepPadGrid` renders two (648). `prepTuningColumn` (248) is referenced by
  nothing else, and `PerformanceLayoutTests` asserts against the inflated
  figure.
- The siren unit switch changes the pad row's length (GS1 has 8 presets,
  DS01E 4), so every mouse target and key-addressed pad relocates.
- `SirenPadRow` still uses a raw AppKit `Picker(.segmented)` rather than
  the house `DubSegmentedControl`.

**The sequence.** Each step is safe to land alone, and the order is a
dependency order, not a preference.

0. ~~Push the library cleanup~~ — done.
1. **Baseline the Prep surface** — done, `PrepPadSnapshotTests`. Required
   `SirenExpertPanel` to become value-driven, which is the unlock this file
   named. Everything below is now an image diff rather than a claim.
2. ~~The contrast package~~ — **done.** Unlit labels onto `textSecondary`
   (3.64:1 → 6.27:1), a real `enabled` state *inside* `DubPadCell` replacing
   the caller-side `.opacity(0.5)`, a `DubKeycap` primitive for bindings
   (a punched `surface0` well, 2.20:1 → 6.27:1), `numericLarge` on the
   header stats, and the hardcoded `FX —` chip deleted. It landed on
   Performance in the same pass by construction — the primitives are
   shared — which 13 of the 20 moved baselines confirm.
3. ~~The binding registry~~ — **done.** `DubKeymap` replaces three inline
   `[UInt16: Int]` dictionaries in `KeyEventMonitorHost` *and* three
   hand-typed legend arrays. A binding carries a transport (key / MIDI /
   HID) from the start so profiles need no migration, and `A S D F` are
   modelled as **reserved** — the cap renders, the key falls through.
   Behaviour-neutral by construction: all 20 pad baselines still match
   after the legends started coming from the table.
4. ~~Cue names + colours through the FFI~~ — **done, FFI 67.** No schema
   migration was needed: `track_cues` has carried `name` and `color` since
   M11e because the importers write them, and only the *user* cue path was
   dropping them. `HotCue` now carries both, and `set_hot_cue_label` is
   separate from `set_hot_cue` so re-dropping a cue to nudge its position
   cannot silently erase the label.
5. **The surface overhaul — Prep done, Performance open.**
   `PrepRack` replaces `PrepPadGrid`: **CUE** as an unboxed list of named,
   coloured marks with timecodes; **LOOP** as a boxed instrument built
   around a numeral in a well, with ×2/÷2 steppers, one contiguous ladder
   and an ACTIVE lamp; **SAMPLES** as a drop shelf whose own border is
   dashed while empty. No shared module wrapper — a first pass gave all
   three the same frame, which re-flattened the hierarchy one level up
   from where it started.
   The FX left Prep entirely (echo, siren, Expert, Quick Scratch, rack),
   and sample *loading* moved in from Preferences.
   The sample shelf takes the remaining width and lays its bank out in
   adaptive columns, because no fourth section is coming.
   Since then: the header drops the deck label and PITCH in Prep (one
   deck, and no platter to pitch), the title/artist separator has equal
   air on both sides, CUE is **HOTCUE**, a hot cue on a *paused* deck
   previews while held and returns to the mark on release (both
   surfaces), a cue's colour now paints its waveform line, and the
   deck/library drag handle is gone — Prep sizes the deck to its content
   and hands the rest to the library, Performance keeps the decks
   dominant. `prepPadBarMinHeight` re-measured 280 → 192.
   **Performance is deliberately untouched** — its design is not settled,
   and `CuePadSection` still draws numbered pads there.
   `SirenPadRow` / `SirenExpertPanel` are kept but unmounted, staged for
   F-37's move to Performance; the file says so rather than letting them
   rot.
6. **Map mode.** Serato's mechanism: turn on MAP, click a control, press
   the key or move the knob. Not a Preferences screen — a mode over the
   live interface, so it is on both surfaces by construction. Keyboard lane
   at M18; the MIDI lane is the same mechanism pointed at
   `dub-controller` once that crate is real, and it is the missing half of
   Expert FX ("a real MIDI controller used instead of a second turntable").
   Shape is settled below: two lanes per control, latch-vs-momentary owned
   by the control, LED output in from the start, one shared map, and a
   binding that carries a transport (key / MIDI / HID) so profiles are
   possible later without a migration.

**Decisions — settled 2026-09-08.**

- **Vinyl rip's home.** Entered from a `VINYL RIP` entry in the library
  source tree, alongside the other five import sources — a rip *is* an
  import, and that is where a user looks for one. Selecting it opens the
  capture/review surface rather than leaving it squatting in the pad bar,
  where it currently evicts the pad rows during review, borrows the Track
  Overview band during capture, leaves the deck pane on its idle
  placeholder (R-41) and spends Prep's scarce vertical axis. The surface
  itself takes the **whole window** — capture and review need it, and the
  PRD's "no vocabulary leaking between modes" rule is the argument against
  leaving it inside Prep. The mode switch is a consequence of selecting the
  source, not a tab to remember; coming back lands in Prep with the new
  tracks selected.
- **The sampler is a Prep citizen too.** Firing stays a Performance move,
  but **loading and binding samples belongs in Prep** — that is work a DJ
  does in advance, and routing it through Preferences only was wrong.
- **The vintage FX rack gets a real home.** A Preferences toggle enables
  it; once enabled the deck source switch grows a fourth position named
  **`DUB FX`** (INT · TC · THRU · DUB FX). That settles F-38 stage 2: the
  rack is a deck role, and the switch is how you select it.
- **Map mode, all five.** Two binding lanes per control (key **and** MIDI,
  so the laptop keymap and the controller can coexist). Momentary-vs-
  latching is a property of the **control**, not the binding — fewer ways
  to get it wrong. **LED output back to the controller is in**, designed
  from the start rather than retrofitted. **One shared map** across
  surfaces. **Profiles come later, but the model must allow for them
  now** — and they have to span keyboard, MIDI *and* HID, so a binding
  carries a transport dimension from day one even while only the keyboard
  lane ships.

**No beatgrid editor — decided 2026-09-08.** The surface map flagged its
absence as Prep's biggest gap, on the strength of PRD §3.1 naming a
beatgrid editor as Prep's reason to exist. Ruled out: **setting the 1 plus
Analyze is enough.** The deck-header BPM tap re-anchors the grid to the
visible kick, and Analyze produces the grid in the first place; the six
FFI calls behind a full editor (`nudge_beat_grid_phase` / `_bpm`,
`scale_active_beat_grid`, `reset_active_beat_grid_to_auto`,
`install_beat_grid_from_taps`, `set_bar_phase`) stay unmounted rather than
becoming a surface nobody asked for. `PrepRack` is three sections and does
not reserve room for a fourth.

**One gap left that is bigger than a step.** The **vintage FX rack**
cannot live anywhere until the deck source switch grows its `DUB FX`
position (F-38).

**Placement rule, for future features.** A capability belongs where the DJ
is in that state of mind. Prep is couch work with no rig; Performance is a
record running in front of people. Corrected against that: pitch and key
lock are Performance-only (Prep is always `+0.0 %`); every FX is
Performance-only; loading a track is drag-and-drop or a key, never a
button; crates, imports, analysis and keymap remapping are reachable from
**both**; and sample *loading* is Prep work even though sample *firing* is
not. That group is the surprise — Prep is not "the library mode". The two
surfaces differ almost entirely in the deck.

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
- **Waveform renderer, second-order CPU (noted 2026-09-10, not visible today).**
  A read-through after the library-sidebar drag work found the big items all
  done (off-main `CVDisplayLink` thread, vsync-extrapolated playhead, grid +
  cues + loop in one draw call, paused deck at zero cost) and three leftovers:
  (1) while playing, every frame re-ingests the whole 8 192-chunk window
  around the playhead — ~350 KB per deck across the FFI, three copies deep —
  when only ~12 chunks are new; a delta ingest (full window only on seek /
  generation bump) would cut it ~99 %. (2) Two array literals in
  `drawBeatGrid` (loop-edge pair, per-cue halo/stem pair) allocate on the
  render thread per frame. (3) `NSEvent.pressedMouseButtons` is read twice
  per frame from the render thread. Measure first — `make trace-grid` with
  two decks playing — and do (1) only if it shows; (2) is free.
  This MacBook Pro is Intel, and the renderer's comments benchmark against
  Apple Silicon, so the trace is worth taking here.

## Keeping this file honest

Update it in the same change that ships a milestone or opens a gap — the same
rule as `SHIPPED.md`. If a section here is longer than a screen, it belongs in
`UI-BACKLOG.md` or the PRD, and this file should link to it instead.
