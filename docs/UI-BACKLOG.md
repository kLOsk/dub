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

## 2. UX polish (works, but feels rough)

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

## 3. Code health (no visible symptom yet, but architectural debt)

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
normal needle lift: both look like the carrier going away. Absolute-
position decoding **did** ship (M6 — `crates/dub-timecode/src/absolute.rs`
reads the LFSR groove position off the whitened carrier phasor); what is
still missing is the *classification* — deciding "the needle is in the
lead-out region" vs "the needle was lifted" — which is what gates the
auto-internal switch.

**Fix**: add lead-out-vs-lift discrimination on top of the shipped M6
absolute decode, then auto-engage internal play (Repeat, PRD §5.4.2)
*only* in the detected lead-out. Until then a dropout pauses and the
operator presses internal Play to continue past the end zone.

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

---

## Closed (archive)

_One line each; full write-ups are in git history. Kept so a reopened symptom is easy to cross-reference._

- B-26. Waveform scrub lag and playback stutter — fixed (M11d.5 follow-up)
- B-11. Auto BPM locks at 2× tempo on real hip-hop / rap — largely addressed; residual cases handled by tap-to-grid
- B-7. `scanMissingFilesBatch` has identical `if/else` branches — already fixed (M11d.8)
- B-8. `libraryTrackCount` refresh clobbers the user's selection — fixed (M11d.8)
- B-9. Search field has no debounce — fixed (M11d.8)
- B-10. Key column tap-to-toggle fires on every cell tap — stale after browser column reset
- B-25. BPM-grid polling on the deck never latches — fixed (M11d.5 round 4)
- U-18. Beat-grid overlay lines jitter slightly during playback — fixed (waveform + grid jitter killed end to end)
- U-19. Tap-to-grid not implemented (PRD §8.3.1) — shipped (M11c.3b / M11d.7 / PRD-BEATS)
- U-12. Dev-facing placeholder text leaks into Casual-Play — fixed (M11d.8)
- U-13. LibraryView duplicates info that's already in the header — stale after list rewrite
- U-16. arrow-key navigation fails silently at list edges — fixed (M11d.8)
- U-19. Idle-pane hint text gets truncated — fixed (M11d.8)
- U-20. PerformanceView idle pane shows redundant copy — fixed (M11d.8)
- U-22. Tooltip text is dev-leaky — fixed (M11d.8)
- U-23. Onboarding doesn't exist — fixed (M11d.8)
- C-24. `FileBrowserView.swift` is dead code — deleted
- C-25. `LibraryPlaceholder` is dead code — deleted
- C-26. BPM lookup differs between DeckHeader and LibraryView — fixed (M11d.5 round 4)
- C-29. `Table` selection model double-fires on programmatic set — obsolete after Table removal

## Triage notes

* Buckets are roughly ordered by priority within each section
  (top of list → fix first).
* Items B-7 through B-9 are now closed (M11d.8) — they were the
  gate before major new library UI features. B-7 was already fixed
  before the pass; B-8 (selection preserve) and B-9 (search debounce)
  landed in the pass. B-10, B-25, and B-26 are retained only as
  closed context.
* M11d.8 "Polish & First-Run" also closed U-12, U-16, U-19, U-20,
  U-22 (UX truthfulness sweep) and U-23 (first-run onboarding).
  Remaining open UX items (U-14, U-15, U-17, U-18, U-21) are
  non-blocking and can ride along with the next library PR.
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
