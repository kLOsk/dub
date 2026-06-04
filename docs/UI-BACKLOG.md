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

### ~~B-26. Waveform scrub lag and playback stutter~~ — fixed (M11d.5 follow-up)

Closed after dogfood validation (2026-05-20). Playing-deck scrub is responsive
again; idle playback scroll is smooth. Landed in `01291cd` via demand-driven
Metal rendering (`CVDisplayLink` on the main run loop), input yield during
press/drag, redundant peak-ingest skip when the playhead is stable, and GPU
catch-up when frames are deferred. No further action unless regression.

---

### ~~B-11. Auto BPM locks at 2× tempo on real hip-hop / rap~~ — largely addressed; residual cases handled by tap-to-grid

Closed as a blocking item. The octave decision was hardened across
M11c.3a–f (perceptual tempo prior, reggae skank double-time rejection,
genre-aware `OctaveProfile`, hip-hop double-time rejection, FourOnFloor /
Dub mid-band fix) and the M11d.7 / PRD-BEATS rounds 5–10 (universal
downbeat, `OctaveProfile::HipHop` lower-octave bias,
`OctaveProfile::DrumAndBass` upper-octave preference, integer-snap safety
net). The real-music regression corpus this item asked for now exists at
`crates/dub-bpm/tests/fixtures/*.tsv` (94 tracks, gated by
`real_music_corpus`); Classic sits at 11/94 failures concentrated in DnB
half-time / triplet false peaks and intrinsically ambiguous edges.

Why this is not "fixed to zero": `BPM-DETECTOR-V2-INVESTIGATION.md`
measured a from-scratch non-ML structural detector and found it cannot
beat the tuned Classic logic (overlapping decision classes have no global
threshold). The remaining ambiguous tracks are the user's call —
**tap-to-grid (U-19, shipped) is the per-track override** and the
`BpmRange` escape hatch constrains the search. Further automatic gains are
gated on a learned beat tracker (see the investigation doc); do not re-open
this as a heuristic-tuning task.

**Location:** `crates/dub-bpm/src/tempo.rs`, `crates/dub-bpm/src/octave_profile.rs`, `crates/dub-library/src/analysis.rs`.

---

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
that fires the FTS query *also* causes a full list refresh and
layout. On a 500 ms-long word the user types six FTS queries plus
six list rebuilds; we want one query plus one rebuild, ~250 ms
after the last keystroke.

**Fix**: wrap the `onChange` in a SwiftUI `.task(id: searchText)`
that sleeps 250 ms before firing `refreshTracks()`. Cancel-on-
new-input is automatic with that pattern (the previous task is
torn down when `id` changes).

---

### ~~B-10. Key column tap-to-toggle fires on every cell tap~~ — stale after browser column reset

The current performance browser no longer renders the Key column
by default (Artist / Title / BPM / Comment only), and the SwiftUI
`Table` selection model has been replaced by a custom
`ScrollView + LazyVStack` row model. If the Key column comes back
through customizable columns, re-open this as a header-toggle
affordance rather than a row-cell gesture.

Original fix still stands: the notation toggle belongs in the
column header, not on individual row cells.

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

### ~~U-18. Beat-grid overlay lines jitter slightly during playback~~ — fixed (waveform + grid jitter killed end to end)

Closed. The grid + envelope now share one playhead source extrapolated
from the audio clock (`EngineHostTimeMapping` / `PlayheadMarker`), and the
renderer draws off the main thread (`WaveformRenderThread`). The wobble is
gone in dogfooding. The `os_signpost` capture runbook in
`WAVEFORM-JITTER-CAPTURE.md` remains wired (`make trace-grid`) for the next
regression. Re-open only with a captured trace showing a bad frame.

---

### ~~U-19. Tap-to-grid not implemented (PRD §8.3.1)~~ — shipped (M11c.3b / M11d.7 / PRD-BEATS)

Closed. Tap-to-grid is the deck-header BPM affordance: 1–2 taps within 2 s
relatch the downbeat at the first tap (a pure `bar_phase` rotation, BPM
bit-identical); 3+ taps recompute tempo via `analyze_beat_grid_from_taps`
(tap median seeds the BPM range, ODF refines). Persists `user_tap` rows and
installs the grid in-place on the loaded deck without a full reanalysis. The
binding spec is `PRD-BEATS.md` (it supersedes PRD §8.3.1). This is the
per-track override for the residual B-11 cases.

---

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

### ~~U-13. LibraryView duplicates info that's already in the header~~ — stale after list rewrite

The current browser keeps the right-pane count in the footer. The
sidebar still shows the library total beside "All Tracks", which
is a different navigation cue and not this duplicate-header bug.

No action unless the right-pane header count is reintroduced.

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

The Key column is currently not part of the default performance
browser. When it returns via customizable columns, the header must
make the notation mode visible; a hidden click target on `KEY`
will not be discoverable.

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

### ~~C-24. `FileBrowserView.swift` is dead code~~ — deleted

Deleted. The M10.5b file-browser sidebar had no call sites after
the M11d library superseded it (only its own `#Preview` and stale
doc-comment references in other files, which were rewritten). The
XcodeGen `sources: - path: Dub` directory glob drops it on the next
`bootstrap.sh`; no filelist edit was needed.

---

### ~~C-25. `LibraryPlaceholder` is dead code~~ — deleted

Deleted from `Placeholders.swift` (struct + its `#Preview` + header
comment). `FXBarPlaceholder` stays — it still marks the unshipped
M15/M16/M17 FX slots. `DubLayout.libraryMinHeight` is retained
because `LibraryView` still uses it.

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

### ~~C-29. `Table` selection model double-fires on programmatic set~~ — obsolete after Table removal

The current browser no longer uses SwiftUI `Table` selection.
Selection is owned by the row model and `LibraryArrowKeyView`, so
this exact double-fire path is gone.

Re-open only if a future NSTableView/SwiftUI Table paging rewrite
reintroduces a binding echo.

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

### C-31. Swift-UI snapshot tests are PRD-mandated but don't exist yet

PRD §2.2.4 says "every PR that changes a view must include
updated snapshots" via `swift-snapshot-testing`. We have zero
Swift snapshot tests. Every UI regression this round (footer
progress pill missing on Re-analyze, multi-select context menu
label stuck on a single track, BPM dimmed when locked, search
field couldn't deselect, click delay on row select) would have
been caught by a small snapshot suite around three views.

Scope of the first cut:

* `LibraryView` footer in three states (idle, batch in flight,
  missing-files banner). Catches the C-1 / C-2 class of
  flash-prone progress pills.
* Library row context menu in single, multi-unlocked,
  multi-mixed (some locked), and all-locked selections.
  Catches the multi-select-label staleness class.
* `DeckHeader` in idle, playing, locked-grid, and rolling-tap
  preview states. Catches "BPM color wrong when locked" and
  "tap rolling number missing" classes.

Implementation notes:

* Add `pointfreeco/swift-snapshot-testing` as a Swift-package
  dependency on the test target only (not the app target).
* Snapshots committed to `apple/DubTests/__Snapshots__/`.
* Reviewer protocol: snapshot diff is part of PR review,
  reviewer must click through each new image. CI runs the
  tests but does NOT auto-accept changed snapshots; a changed
  snapshot is a test failure until the developer re-records
  with `record: true` and commits the new image.
* Start with `assertSnapshot(matching: view, as: .image)` for
  three sizes (compact / standard / wide) per view.

Counts as the highest-leverage SwiftUI investment. The
boundary contracts that the snapshot suite would test against
are now documented inline at each `NSViewRepresentable` header
(see commit landing alongside this entry).

---

## 4. Performance / timecode mode (deferred)

_Added 2026-06-04 after dogfooding the timecode playback wiring on the
SL3. The load/lift/hand-back behaviour shipped (see notes below); these
are the follow-ups that were explicitly deferred so we don't lose them._

What shipped in this round (for context, not action):

* Loading a track in Performance mode no longer auto-plays. The
  drag-to-play idiom is now Prep-only (`apple/Dub/Performance/PerformanceView.swift`,
  `DeckDropTarget`).
* A needle lift pauses the deck and holds position instead of
  auto-engaging internal play. The PRD §5.4.2 "Repeat" auto-Panic-on-
  dropout was removed (`crates/dub-engine/src/lib.rs`,
  `drive_timecode_inputs` `DropoutHoldRate` arm).
* Internal (Panic) Play hands control back to timecode once the
  carrier is solidly re-locked for a short debounce window
  (`PANIC_RELOCK_BLOCKS_TO_HANDBACK`), so dropping the control record
  back on resumes timecode control.

### P-32. End-zone auto-internal needs absolute-position decoding (M6)

The intended behaviour is: when the timecode runs into the control
record's end zone (the lead-out / indefinite loop area), the deck
auto-switches to internal play so the track keeps going; the operator
can then lift and reposition the needle to re-enter timecode. We cannot
do this today because the engine cannot tell the lead-out apart from a
normal needle lift: both look like the carrier going away. The decoder
only integrates *relative* position from carrier phase
(`crates/dub-timecode/src/decoder.rs`, `position_secs`); it does not
read the absolute-position bitstream that rides on the control tone.

**Fix**: implement M6 absolute-position decoding in `dub-timecode` so
the deck knows when the needle is in the lead-out region, then
auto-engage internal play only there (replacing the current "a dropout
always pauses" behaviour at the end zone). Until then a dropout pauses
and the operator presses internal Play to continue past the end zone.

**Location**: `crates/dub-timecode/src/decoder.rs`,
`crates/dub-engine/src/lib.rs` (`drive_timecode_inputs`).

---

### P-33. Cannot switch to internal play while timecode is actively running

While the control vinyl is driving the deck, the deck Play button does
not let the operator switch to internal play. Today they have to lift
the needle first (which now pauses the deck), then press Play. The
intended flow: while timecode plays, pressing Play switches to internal
playback at the current rate; the deck then keeps playing internally
even when the needle goes back down.

Open product decision (undecided, deferred deliberately): once the
operator is in internal play and drops the needle back on, should the
deck (a) stay internal until they explicitly re-engage timecode, or
(b) auto-return to timecode control as soon as a clean carrier is
present. Option (b) is what the engine hand-back already does today;
option (a) needs an explicit "internal latch" that the carrier cannot
override. Decide this together with the pill/UX work in P-34.

**Location**: `apple/Dub/Performance/DeckHeader.swift` (Play button
enablement/labeling in timecode mode), `apple/Dub/MainView.swift`
(`play(side:)` timecode branch), `crates/dub-engine/src/lib.rs`
(panic hand-back debounce).

---

### P-34. Performance source pill is not self-explanatory

The deck-header source pill currently reads "FILE", and the dropout
state surfaces as "TC HOLD"; neither communicates the actual deck state
to a DJ (timecode-driven vs internal play vs paused vs thru). This is
the UI half of P-33: the operator needs to see at a glance whether the
deck is following the platter, running internally, or paused, plus the
green/amber/red timecode tracking dot from PRD §5.4.

**Fix**: redesign the source pill + tracking dot to show Timecode /
Internal / Paused (and Thru once P-35 lands), driven by truthful engine
state. Pairs with the Phase 4 UI-truthfulness work in P-35.

**Location**: `apple/Dub/Performance/DeckHeader.swift`.

---

### P-35. Automatic per-deck source detection (Thru ↔ Timecode)

PRD §5.1.1 calls for each deck to auto-detect whether the input is
control vinyl (drive a loaded file) or a real record (Thru passthrough)
and switch transparently. The timecode-playback wiring shipped via the
engine's existing tested path; the auto-detection half (Phases 2 to 5
of the `timecode_playback_+_auto_source_detect` plan) was deferred to
avoid rewriting the audio thread's render path without hardware
validation.

**Fix**: the deferred plan phases — a per-deck `SourceMux` (single input
read per block, atomic mode flip), an off-RT `SourceDetector`
(silence / Goertzel spectral / Serato-lock state machine publishing
`desired_mode` + confidence), a 5 ms equal-power crossfade with
stickiness + freeze-during-scratch, FFI `deck_source_mode` /
`timecode_signal`, and the waveform-source-follows-mode UI. All
render-path code must stay allocation/lock/syscall free
(`assert_no_alloc` + `make rt-audit`).

**Location**: `crates/dub-engine/src/{thru,timecode}.rs`,
`crates/dub-engine/src/lib.rs`, `crates/dub-thru/src/lib.rs`,
`crates/dub-ffi/src/lib.rs`, `apple/Dub/Performance/`.

---

## Triage notes

* Buckets are roughly ordered by priority within each section
  (top of list → fix first).
* Items B-7 through B-9 should land before major new library UI
  features, since they affect existing functionality. B-10, B-25,
  and B-26 are retained only as closed context.
* The beat-grid cluster (B-11 octave, U-18 grid jitter, U-19
  tap-to-grid) has shipped / closed. Further automatic octave gains
  are gated on a learned beat tracker — see
  `BPM-DETECTOR-V2-INVESTIGATION.md`, not a heuristic-tuning task here.
* C-24 / C-25 (`FileBrowserView` + `LibraryPlaceholder` dead code) are
  deleted. The remaining open code-health items (C-27, C-28, C-30, C-31)
  are non-blocking and should ride along with the next library PR.
* The UX bucket can land alongside M11e (Library polish) — most
  items are surface-level copy or layout tweaks.
* Code-health items are not blocking but should be ticked off
  during routine refactors rather than left to accrete.
* Section 4 (P-32 to P-35) tracks the Performance / timecode follow-ups
  deferred after the 2026-06-04 SL3 dogfood. P-32 (end-zone) is gated on
  M6 absolute-position decoding; P-33 + P-34 are a paired product/UX
  decision; P-35 is the deferred auto-source-detection plan phases.
