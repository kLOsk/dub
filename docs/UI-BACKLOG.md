_<!-- This backlog was triaged on 2026-05-17 during a wide UI/UX
review of the M0.5+ SwiftUI shell. Items shipped in the same
session as the review (right-deck mirror, DeckHeader key surfacing,
PITCH column hide, analyze-progress counter, recency sort
preservation, Preferences shortcut conflict, per-row drag-URL FFI)
have been removed; what remains is the work explicitly deferred so
we don't forget. Cross-referenced against PRD.md and SHIPPED.md;
none of these items contradict a shipped milestone — they are
either bugs in surrounding plumbing or UX polish that the M11d.5+
flow will revisit. -->_

# UI / UX Backlog

Working notes from the 2026-05-17 SwiftUI review. The fixes that
shipped that same session are in `SHIPPED.md`; this file tracks
everything that was found and *not* immediately fixed, in priority
buckets. Each item lists the symptom, the location, and the
intended remediation so the next pass can land it without
re-discovering the issue.

---

## 1. Real bugs (correctness or visible user pain)

### B-7. `scanMissingFilesBatch` has identical `if/else` branches

`MainView.swift` ~L957–985 — the loop that reconciles the
`track_files` rows against on-disk presence has:

```
if isMissing != r.wasMissing {
    try library.markFileState(fileId:, isMissing:, timestampUnixSecs:)
} else {
    try library.markFileState(fileId:, isMissing:, timestampUnixSecs:) // identical args
}
```

Intent was almost certainly:

* When the missing-state flipped → write the new state.
* When it did *not* flip → bump `last_seen_at` only (a separate
  FFI, or a `markFileStateTouched(fileId:)` variant).

As-is, every row in the batch writes the same row to SQLite twice
the database hits would otherwise be on first-import days. The
behaviour is correct (we re-mark the same state), just wasteful.

**Fix**: either collapse the branches (write once, since both do
the same thing) or split the FFI so the unchanged path is a cheap
`UPDATE … SET last_seen_at = ?` without re-evaluating `is_missing`.

---

### B-8. `libraryTrackCount` refresh clobbers the user's selection

`LibraryView.swift` — `refreshTracks()` (called from the
`onChange(of: model.libraryTrackCount)`) defaults to
`preserveSelection: false`. The intended trigger is "another window
just imported tracks, refresh the listing" — but it also fires
during the user's own session if track-count bumps for any reason,
losing whatever row they had highlighted.

**Fix**: pass `preserveSelection: true` from this `onChange` so
the row the user is *looking at* survives the refresh. The
preserve path already exists; only the call site is wrong.

---

### B-9. Search field has no debounce

`LibraryView.swift` — the `onChange(of: searchText)` handler
re-queries the FTS5 index synchronously on every keystroke. On a
100 k row library this is fast enough today, but the keystroke
that fires the FTS query *also* causes a `Table` re-layout, which
is the bottleneck. On a 500 ms-long word the user types six FTS
queries plus six full table layouts; we want one query plus one
layout, ~250 ms after the last keystroke.

**Fix**: wrap the `onChange` in a SwiftUI `.task(id: searchText)`
that sleeps 250 ms before firing `refreshTracks()`. Cancel-on-
new-input is automatic with that pattern (the previous task is
torn down when `id` changes).

---

### B-10. Key column tap-to-toggle fires on every cell tap

`LibraryView.swift` — the Camelot/musical-notation toggle is
attached to the *cell* via `onTapGesture`, so any click on the KEY
column cell flips notation *and* races with `Table`'s row
selection model. Users trying to single-click a row to select it
sometimes flip notation instead, depending on which sub-region of
the cell catches the tap.

**Fix**: move the toggle to the column *header* via a
`TableColumn` header builder that wraps a `Button` with no
chrome. Headers don't compete with row selection, and "click the
column title to change the column's display" is a familiar
affordance on macOS (mail/finder both do versions of this).

---

### ~~B-25. BPM-grid polling on the deck never latches~~ — fixed (M11d.5 round 4)

Closed by the library-sourced beat-grid work documented in
`docs/SHIPPED.md` under "Follow-up — Library-sourced beat grid
is the single source of truth (M11d.5 round 4)". The deck-load
handshake now delivers the final grid before the first Metal
frame after load (when the library has an active row) or runs
the deterministic engine analyser which returns
`confidence = 0` for silence (when it doesn't). Either way the
renderer's `confidence > 0` latch fires on the first poll and
the perpetual-FFI failure mode is gone. The fix landed at the
*source* of the per-frame poll (the load handshake) rather than
the symptom (a per-deck "analysis finished" flag), which also
removed the ±0.02 BPM cross-deck drift and the redundant
~100–400 ms BPM analysis on every load.

---

## 2. UX polish (works, but feels rough)

### U-12. Dev-facing placeholder text leaks into Casual-Play

`Placeholders.swift` — strings like "Phase-Drift Trail lands at
M10.7" and "Filter ladder ships with M14" appear verbatim in the
UI. These are meaningful to us but confusing to a DJ who just
opened the app.

**Fix**: replace with user-facing copy that names the *feature*
rather than the *milestone* ("Phase drift visualisation coming
soon"), or hide the placeholder entirely in Casual-Play builds
behind a debug flag.

---

### U-13. LibraryView duplicates info that's already in the header

The header line at the top of the right pane reads
`5 000 shown · 12 743 total` while the footer also reads
`5 000 shown · 12 743 total`. One of them is enough; the footer
is the more conventional spot (Finder, Mail) and aligns with
the analyze-progress + missing-files lines that already live
there.

**Fix**: drop the header copy. Keep the footer.

---

### U-14. No feedback while a single-deck-load analysis is running

`ensureTrackAnalyzed` fires when a track loads but doesn't surface
its progress anywhere — the BPM column shows "—" until the worker
thread completes, at which point it just appears. Users have no
way to tell if analysis is in flight or stuck.

**Fix**: bump `analysisInFlightCount` for single loads too (it
already is), and have the LibraryView footer render a quiet
"Analyzing 1 track…" line whenever `analysisInFlightCount > 0
&& analysisBatchTotal == 0`. Different copy from the batch line
so users can tell the modes apart; same spinner.

---

### U-15. Error toasts are too aggressive

`surfaceError` writes to `lastError` which renders as a banner
across the top of the window. Any FFI hiccup (transient unmounted
volume, a stale `track_path` lookup, an analysis failure on a 5
second track) lights up the banner for several seconds. Users in
the middle of a mix should not have anything full-width-stealing
their attention.

**Fix**: split the error path into "user-actionable" (banner) and
"informational" (status-strip glyph or quiet log line). Analysis
failures, single-row resolve failures, missing-file scans go to
the log. Engine start failures, library open failures, Preferences-
modal errors go to the banner.

---

### U-16. `navigateToSibling` fails silently

The arrow-key sibling navigation in LibraryView is a no-op when
there's no next sibling (top/bottom of the listing). No glyph
flash, no system beep. Users press Down repeatedly at the end of
the list and nothing tells them they're at the end.

**Fix**: `NSSound.beep()` on hard-stop, matching the rest of macOS
arrow-key navigation in tables.

---

### U-17. The notation-toggle affordance is invisible

KEY column header reads `KEY` whether you're in Camelot or
musical notation. Users have no way to know that clicking it
flips between the two notations.

**Fix**: render the header as `KEY (Camelot)` / `KEY (Musical)`
or a small toggle pill. Tooltip already explains it; the visible
chrome doesn't.

---

### U-18. Sort semantics differ between FFI and client sort

The FFI's `list_tracks_sorted` puts NULLs last in both directions
("missing tag rows don't jump to the top when you click Artist").
The client-side `sortedTracks` sort uses `KeyPathComparator` over
optional sort keys, which Apple's framework happens to put NULLs
last for ASC and first for DESC. Result: clicking Artist twice
gives different NULL placement than the initial open.

**Fix**: build a `KeyPathComparator` that always sorts NULLs
last, or push the sort back through the FFI so there's a single
source of truth. The FFI path is preferable once M11d.4 paging
lands, since client-side sort doesn't scale to a paginated
listing anyway.

---

### U-19. Idle-pane hint text gets truncated

`Placeholders.idleHint` renders a multi-line cue inside the
deckPane's `GeometryReader`. When the window is narrower than
~840 px, the second line truncates mid-word ("…drop a track to
load i").

**Fix**: replace the explicit `.lineLimit(2)` with
`.lineLimit(nil)` + `.minimumScaleFactor(0.8)`. The hint is allowed
to wrap to three lines on narrow windows; better truncated text
ruins the cue.

---

### U-20. PerformanceView idle pane shows redundant copy

Both decks render the same idle hint ("Drop a track here · Space
to load the browser selection · ⌘O to open the library") when no
track is loaded. On a two-deck layout this reads twice.

**Fix**: render the hint only on the left (or master) deck. The
right deck idle pane stays blank — the divider already implies
symmetry.

---

### U-21. StatusStrip mixes engine + library state

The status strip currently shows engine-running, master-deck, and
library-imported counts in a single horizontal row. Engine state
is a "what's playing right now" cue; library counts are a "how
big is my collection" cue. Mixing them means glance-distance
parsing has to disambiguate which number is which.

**Fix**: move library counts to the LibraryView footer (where
they're already echoed), keep the status strip for live engine
state only.

---

### U-22. Tooltip text is dev-leaky

Several tooltips embed milestone names ("M11c.2 Camelot key
detection") that are meaningless to a DJ. Tooltips should describe
the *feature*, not the *milestone*.

**Fix**: sweep `.help("M…")` strings and rewrite them in user
language.

---

### U-23. Onboarding doesn't exist

The first-run experience is "you open the app, see an empty
library, and figure it out". For a DJ tool with a strict hardware
prerequisite (≥4 in / 4 out interface) this is hostile.

**Fix**: first-run sheet that walks the user through device
selection, channel routing, and "drop a folder here to import".
Skippable. Lives behind the Preferences sheet so power users can
re-open it.

---

## 3. Code health (no visible symptom yet, but architectural debt)

### C-24. `FileBrowserView.swift` is dead code

The M10.5b file-browser sidebar was superseded by the M11d
library. The file remains in the target with no call sites.
Removing it would drop a few hundred lines of code and prevent
future contributors from extending the wrong UI surface.

**Fix**: delete the file; verify no references in the Xcode
project's filelist.

---

### C-25. `LibraryPlaceholder` is dead code

Same story for the empty-state placeholder we used pre-M11d.2;
the new LibraryView's empty states cover all paths.

**Fix**: delete.

---

### ~~C-26. BPM lookup differs between DeckHeader and LibraryView~~ — fixed (M11d.5 round 4)

Closed by the library-sourced beat-grid work documented in
`docs/SHIPPED.md` under "Follow-up — Library-sourced beat grid
is the single source of truth (M11d.5 round 4)". The fix lands
exactly as the backlog item suggested: the engine now adopts the
library's `track_beatgrids(is_active = 1)` row via a new optional
`LibraryBeatGrid` parameter on `DubEngine.loadTrack`, instead of
re-running `dub_bpm::analyze_beat_grid` from scratch. Both the
DeckHeader and the LibraryView now read the same SQLite row by
construction; the ±0.02 BPM drift is impossible to reproduce. The
side-benefit also materialised — the ~100–400 ms per-load
estimator pass is cut to a few microseconds for any track the
library has already seen (the first-ever load still pays the
analysis cost; see the deferral note in the SHIPPED entry).

---

### C-27. `selectLibraryTrack` calls `trackPath` twice

The selection path:

1. `LibraryView.onChange(selectedTrackId)` calls
   `model.selectLibraryTrack(_:snapshot:)` which calls
   `library.trackPath(trackId:)` to set `browserSelection`.
2. The user presses Space (or drags), which lands in
   `recordLibraryLoadIfApplicable` which *also* calls
   `library.trackPath(trackId:)` to verify the URL hasn't
   changed since selection.

Two FFI lookups for a single user action. The verification round-
trip exists for a real edge case (the selection could go stale if
the volume unmounts between click and Space), but it's the same
SELECT executed twice within a few ms.

**Fix**: cache the resolved URL on `selectedLibraryTrack` (the
snapshot already published as of B-1's fix) and have
`recordLibraryLoadIfApplicable` compare against the cached value
instead of re-querying. The unmounted-mid-action edge case still
gets caught by the `loadTrack` engine error.

---

### C-28. `analysisInFlightCount` naming is now misleading

Post-fix for B-3/B-4 the counter represents "any analysis in
flight, batch or not" but the documentation comment also says
"divides batch total for progress fraction" (true pre-fix, false
post-fix). The right value for batch progress is now
`analysisBatchCompleted`.

**Fix**: this file already documents the rule, but the renamed
counter would be clearer as `analysisInFlight: Bool` (we never
actually use the count > 1 case, since analyses are serial).
Trivial follow-up; can land with the next library-related PR.

---

### C-29. `Table` selection model double-fires on programmatic set

When `selectedLibraryTrackId` is set programmatically (e.g. after
an import lands and we auto-select the new row) the `Table`'s
`selection` binding fires `onChange(of: selectedTrackId)` which
then re-calls `model.selectLibraryTrack`, which re-publishes
`selectedLibraryTrackId`, which fires the `Table` again. The
cycle settles in two ticks but wastes work and risks priority-
inversion on the main actor for large listings.

**Fix**: gate the `onChange` body with a "did this id actually
change?" check, or drive the binding through a single
`@Published` field so SwiftUI's de-dup catches the self-echo.

---

### C-30. RT-thread audit doesn't cover the new SwiftUI render path

The RT-thread auditor walks the engine's audio callback. The
30 Hz UI poll lives on the main actor and is therefore RT-safe by
construction, but the *path* between an audio-thread BPM update
and a deck-header refresh routes through several FFI hops that
are auditable for allocations only on the Rust side. Swift-side
allocations during the poll are uncharacterised.

**Fix**: add a Swift-side `MainActor` profile pass that runs the
30 Hz poll for 60 s under Instruments' Allocations template and
records the allocation rate. Not a hard "must be zero" gate; we
want a baseline so regressions are visible.

---

## Triage notes

* Buckets are roughly ordered by priority within each section
  (top of list → fix first).
* Items B-7 through B-10 + B-25 should land before any new
  user-facing feature, since they affect existing functionality.
* The UX bucket can land alongside M11e (Library polish) — most
  items are surface-level copy or layout tweaks.
* Code-health items are not blocking but should be ticked off
  during routine refactors rather than left to accrete.
