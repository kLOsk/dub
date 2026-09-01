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

**Narrowed (M11d-columns).** The configurable columns
(§8.5.3.1) sort through `LibraryRowComparator`, which holds
empty cells last in *both* directions — the FFI's contract. Only
the built-in columns still go through a bare
`KeyPathComparator`, so this item is now scoped to those; the
comparator to copy already exists.

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

## 5. FX / Dub siren (deferred)

_Added 2026-07-02 when M16 (dub siren + vintage-chip FX DSP) merged. The
instrument, its three units (GS1 / DS01E / SN76477), the shared PT2399 echo,
and the Prep-surface Expert panel shipped; the vintage-FX DSP chain
(spring / RE-201 / Big Knob / phaser) is in-tree but parked behind the deck-role
FX channel below. These are the pieces explicitly held back so they're not lost._

### P-39. Key lock is hard-bypassed while a loop is engaged — **done**

Shipped: the wrap and its seam crossfade moved into the stretcher's *feed*
(`kl_refill`), so the engaged path is handed an already-wrapped, already
seam-faded stream and needs no reset, no re-prime and no knowledge that a loop
exists. `loop_region.is_none()` is gone from `key_lock_engage_decision`. Pinned
by a test that measures the rendered fundamental: 480 Hz into a 12 000-frame
loop (exactly 120 periods, so the wrap is phase-seamless in the source) reads
480 Hz engaged, and read 509.8 Hz — the resampler shift — before the fix.

### P-40. The position extrapolator does not know about loops

**Symptom**: `PublishState::extrapolated_secs` is `position_secs + elapsed ×
rate` with no loop wrap, so between publishes the UI playhead reads past
`loop_out` — by about one block normally, more at a scratched platter rate, and
up to the 100 ms clamp if the audio thread stalls. The same unwrapped value is
what `set_reverse_loop` uses as the press playhead, so re-gripping a loop while
one is running can snap off a reading that is past the loop end. **Remediation**:
wrap the extrapolation into `[loop_in, loop_out)` when `loop_active` — the bounds
are already published in the same shared state. Small; the reason it is filed
rather than fixed is that the visible error is normally sub-frame.
**Location**: `crates/dub-engine/src/deck.rs` (`PublishState`),
`crates/dub-ffi/src/lib.rs` (`set_reverse_loop`).

### F-36. Siren-sound fine-tuning (polish phase)

The three units are voiced and level-matched (−14 LUFS), but the sounds
themselves are a first cut, not final. GS1's 8 shots (bomb / MG / lickshot were
flagged as the roughest), the DS01E 4 MODE tones, and the SN76477 preset bank
all want an ear-tuning pass against reference recordings.

**Fix**: during the polish phase, render fresh WAVs per unit (the `#[ignore]`
`dump_*_wavs` tests in `dub-dsp`), audition, and tune the preset specs +
per-unit output trims. No architectural change — data / coefficients only.

**Location**: `crates/dub-dsp/src/{hk628,siren,sn76477}.rs`.

### F-37. Expert panel on the Performance surface + deck B

The siren Expert panel (`SirenExpertPanel`) currently lives only on the **Prep**
surface for **deck A** — Prep is the mouse-config home, so it's the right first
home, but the panel isn't yet mirrored onto the Performance surface or deck B.
Doing so needs the `WaveformAppModel` threaded into `PerformancePadsView` at
both call sites and the panel bound to the correct `DeckSide`.

**Fix (later — "expert mode in the deck")**: thread the model + side into the
Performance pad bar and render `SirenExpertPanel` there for both decks. The
engine/FFI surface is already complete (per-deck `set_siren_*`), so this is
Swift-only.

**Location**: `apple/Dub/Performance/PerformancePadsView.swift`,
`apple/Dub/Performance/PerformanceView.swift`.

### F-38. FX rack as a deck-role FX channel (post-release)

The vintage-FX rack (HPF → phaser → RE-201 → spring) is **not** per-deck insert
FX. The decided model: it's a dedicated **FX channel** — an outboard-unit that
**replaces a deck** via the source switch (INT · TC · THRU · **FX**), takes a
**mic or mixer aux-send** as its input, and is **Expert-only** (no Simple /
Advanced). Proper sound-clash "FX on the mic" routing. Deferred post-release to
keep M16 focused on the siren.

Stage 1 (engine) **shipped dormant**: `ControlMode::Fx` exists (Thru-style
passthrough + rack processing hook), the four FX blocks + `rack_active` state
live in the engine, and the per-deck rack UI is hidden (`rackFxEnabled` defaults
false; the Preferences toggle was removed). Remaining:

* **Stage 2 (Swift)** — surface an `FX` position on the deck source switch; move
  the rack controls into the FX-deck pane; relabel the deck as an FX channel.
* **Stage 3 (routing)** — input monitoring for the mic / aux-send into the FX
  deck, and the optional "send siren → rack" bridge (off by default) for the
  lush "siren through the Space Echo" sound.

**Location**: engine `crates/dub-engine/src/lib.rs` (`ControlMode::Fx`, dormant);
`crates/dub-ffi/src/lib.rs`; `apple/Dub/Performance/`,
`apple/Dub/Preferences/PreferencesSheet.swift`. Tracked as task #32.

---

## 6. Vinyl rip (M26a + M26b shipped; deliberate gaps)

### R-39. Live per-segment encode progress — **done (M26b)**

Shipped: `commit_session` takes a progress sink, `RipSegmentJobState::Running`
is a real state, and the review-panel dots move during the pass instead of
flipping together at the end.

### R-40. Session recovery banner — **done (M26b)**

Shipped: `RipSession::from_session_dir` + `list_recoverable_rip_sessions` /
`resume_rip_session`, and a quiet "Unfinished rip — Review / Later" row above
the rip bar on Prep entry. "Later" never deletes; the audio is irreplaceable
without setting the needle back down.

### R-44. Re-split has no app entry point (M26b) — **done**

Shipped: `DubEngine::list_resplittable_rip_sessions` (FFI 60, the complement of
the recovery listing) plus a live **Real Records** browser node — the reserved
placeholder, unlocked. Sessions render in the right-hand pane rather than the
track table (a rip has no fingerprint, grid or deck to load, so forcing it
through `LibraryTrack`'s column model would have meant forging rows that lie to
Space-load and the drag path). Picking one calls `resplitRip`, which mirrors
`resumeRip` but auditions from **`side.flac`** — a committed session's spill is
gone by design, so `ripSideAudioURL` now resolves spill-else-archive for all
three load sites.

No navigation was needed: `PerformanceView` stacks `waveformRegion` and
`LibraryView` in one VStack, so the review panel is already on screen above the
row that was clicked. Gated on Prep mode (that is where the panel mounts and
where auditioning takes deck A) with the reason stated in the pane rather than
switching modes — a mode switch restarts the audio engine, which is not
something a browser click should do. Re-split is two-step armed for the same
reason it clobbers whatever is on deck A.

### R-47. The end tracks absorb the lead-in and run-out grooves — **done**

Shipped: the detector bounds the side at both ends — the last *sustained* music
for the tail, the first *sustained peak onset* for the head (a needle drop is
rejected by a forward-looking hold) — and the manifest carries them as
`side_start_frame` / `side_end_frame`, so commit, re-split and the FFI segment
view all honour them. Safe because `side.flac` still archives the whole
capture: a re-split reaches back past the trims and clears them.

Note the first cut of this shipped **dead in the app**: the FFI's `auto_split`
reimplemented `RipSession::auto_split` rather than calling it, and only the
latter writes the trim. Fixed, and gated by a test that fails against the old
code.

### R-48. The discarded grooves are invisible in the rip UI — **done**

Shipped: both discarded regions shade out in the review overlay, and each bound
carries a draggable bracket — a different *shape* to a split marker, not just a
different hue, since the band already has an amber envelope and magenta cues.
Selection became an enum (`RipOverlaySelection`) rather than a sentinel marker
id, so ← / → nudge and ⌫ work on a bracket too: ⌫ *resets* that end to the whole
capture, because a trim always exists and can never be deleted. The context menu
offers the same as "Keep the lead-in" / "Keep the run-out". Split drags and
nudges clamp into the trimmed side; a double-click out in the shade is still
refused rather than relocated, because a discrete intent deserves an answer and
the shade has already explained it. The 5 s minimum-segment rule is deliberately
*not* duplicated in Swift — the FFI refuses and the overlay flashes.

The overlay's axis now spans the whole capture (its fallback used to be the
segment plan's end, which is the trimmed end and would have hidden the run-out
exactly while the deck-A decode was in flight), while the review header reports
the length that actually commits plus what is being dropped.

### R-45. Snapshot baselines are not tracked — **done**

**Symptom**: `apple/DubTests/__Snapshots__/` is in `.gitignore` (zero baselines
tracked), so the regression gate is a local artifact no review ever sees.

The *build* half of this is **fixed**: `project.yml` globbed `- path: DubTests`
bare, sweeping every baseline PNG into the DubTests resources phase, so deleting
one to re-record it broke the generated project. It now excludes
`__Snapshots__`, which is correct because SnapshotTesting resolves baselines
from `#filePath` on disk at run time, never from the test bundle. (The original
"a fresh clone cannot build" symptom was overstated — `apple/*.xcodeproj/` is
itself gitignored and regenerated by XcodeGen, so a fresh clone has no PNGs to
reference.)

Shipped: the 35 baselines are tracked, together with the R-46 re-record pass —
committing the local set alone would have enshrined eight wrong images.

### R-46. Eight PerformanceSnapshotTests baselines are stale — **done**

**Symptom**: `test_deckHeader_*`, `test_performancePads_deckA` and
`test_sourceControl_allStates` fail against local baselines recorded around
`5d5b0f9` (hot-cues) — the views moved through the M11d / M14 / M15 / M16
rounds without a re-record. Verified pre-existing: they fail with all M26b work
stashed. M13 added a second reason for one of them — `test_performancePads_deckA`
now also disagrees because the LOOP row grew IN / OUT pads — but the failing set
is unchanged at the same eight. Shipped, and the diagnosis in the original symptom was half wrong. Measured
per baseline, the eight split into two causes:

* **Two were visually identical** — `idle` and `loading` differed by 310 bytes
  out of 1.25 MB (0.025 %, max delta 16/255). Anti-aliasing, not a moved view.
  Every suite now compares with `perceptualPrecision: 0.98`, so cross-machine
  render noise stops failing the build while a real layout change (which moves
  orders of magnitude more) still does.
* **Six had really changed** (0.24 % to 26 %), and re-recording view by view
  found two things worth fixing rather than blessing. `SourceControlView`
  gained its THRU position when auto-detection was deferred, which made
  `overridden` dead — documented in the type, but two tests still asserted a
  "· PINNED" variant that now renders identically to the unpinned one; they are
  replaced with states that actually differ. And `pads-deck-a` was framed at
  320 pt, chosen when the LOOP row was five pads; M13's IN / OUT pushed it past
  the edge, so the baseline had been clipping its own content — labels reading
  "UE" and "OOP", the exit pad off-screen.

### R-41. Deck pane shows the idle placeholder during capture

**Symptom**: while recording, only the overview band renders the live signal;
the main deck pane sits on its placeholder (the deck has no loaded track in
Thru-for-rip). **Remediation**: either render the live Metal Thru waveform
(the peaks stream is already attached) or design an intentional capture
backdrop. **Location**: `apple/Dub/Performance/PerformanceView.swift`
(`waveformRegion` prep branch), `apple/Dub/Waveform/`.

### R-42. Discogs token belongs in the Keychain (M26c)

**Symptom**: the Discogs token is a user credential and `UserDefaults`
stores it in plain text; there is still no Keychain plumbing anywhere.
**No longer an M26c blocker.** The AcoustID key turned out not to need the
Keychain at all — it is an *application* key that identifies Dub rather than
the user, and Dub's source is public, so it is public by construction;
recognition
therefore ships with its key in `UserDefaults` and works fully without
Discogs. The token field is empty by default and the Discogs half stays off
until someone types one, so nothing sensitive is stored unless the operator
opts in — a mitigation, not the fix. **Remediation**: minimal Keychain
helper, and move `dub.discogsToken` into it. **Location**:
`apple/Dub/Preferences/`.

### R-49. Sample lineage link-out (WhoSampled)

**Symptom**: recognition gives a rip its artist and title, and the question
this audience asks next — what does it sample, what samples it — has no
affordance anywhere. **Remediation**: a "Samples" item in the rip review
panel's track cards and the library row context menu that opens WhoSampled's
search for the track's artist + title. Link-out only: WhoSampled has no
public API and their terms forbid scraping, so an in-app data integration
needs a commercial agreement (PRD §5.2.5a). Cheap, breaks no terms, needs no
key. Optionally surface MusicBrainz's own `samples` / `is based on` relations
beside it — free, thinner coverage, one extra `inc=` on a call
`dub-recognize` already makes. **Location**: `apple/Dub/Performance/
RipReviewPanel.swift`, `apple/Dub/Performance/LibraryView.swift`.

### R-43. Performance-mode ripping + monitor-mute option

**Symptom**: rip is Prep-only by design (v1); with a ≥4-out interface in
Performance mode the same tap could record a Thru deck live. Edge setups may
also want the Mac-output monitor muted while ripping (the tap is pre-output,
so recording is unaffected). **Remediation**: revisit after M26c; the FFI is
mode-agnostic already. **Location**: `apple/Dub/Performance/WaveformAppModelRip.swift`,
`crates/dub-ffi/src/rip.rs`.

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
* The UX bucket can land alongside M11d-columns — most
  items are surface-level copy or layout tweaks.
* Code-health items are not blocking but should be ticked off
  during routine refactors rather than left to accrete.
* Section 4 (P-32 to P-35) tracks the Performance / timecode follow-ups
  deferred after the 2026-06-04 SL3 dogfood. P-32 (end-zone) is gated on
  M6 absolute-position decoding; P-33 + P-34 are a paired product/UX
  decision; P-35 is the deferred auto-source-detection plan phases.
