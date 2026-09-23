# Current state

> **Read this first.** `SHIPPED.md` says what is done; this says what is *in
> flight*, what is blocked, and what to pick up next. Keep it short — a page
> that grows stops being read, and an unread status page is how
> `docs/html/` drifted ten milestones before anyone noticed.
>
> Last updated: 2026-09-11.

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
   Serato CV02 carrier. **First rig session 2026-09-22: loops, scratching and
   pitching all behave** ("loops work awesome"). Still not tried: lifting the
   needle mid-loop, re-locking after a lift, and key lock at a pitched platter
   — the last of which was untestable until that session, because the key-lock
   control was mounted nowhere (below).
2. **A rip end to end through the app.** Every gate is fitted against the three
   captures in `testdata/rip-baselines/` and replayed offline with
   `dub rip-tune`. The trim brackets, the Real Records node and re-split have
   never been driven on the rig with a record on the platter.

## From the first rig session (2026-09-22)

The SL 3 on the real rig, both decks on timecode. What it turned up:

- **Key lock had no UI at all.** `KeyLockControlView` — an OFF/ON segmented
  control with the engine's state dot — existed in `SourceControlView.swift`
  and was referenced by nothing, so key lock sat at its default (off) with no
  way to reach it. The engine side was fine the whole time (FFI 40). It is a
  `LOCK` button in `DeckReadouts` now — the row the deck column draws, beside
  the pitch it holds — bindable in map mode (`keylock.a` / `keylock.b`). The
  old unmounted view is deleted. **It took two goes**: the button first went
  into `DeckHeader`, which only Prep renders, so it did not appear on the rig
  either — the same dead-UI trap, twice in one session. `PitchTestView` is
  still unmounted; it is the bench pitch rig for key-lock A/B and stays that
  way.
- **Three follow-ups the same evening, all found by using it:**
  - **The LOCK button was inert** — and not because of the button.
    `identityAndReadouts` carries the instant-double `.onDrag`, and the
    readouts row lives inside it; a drag source takes the press before any
    child gesture sees it. The drag is on the title and artist text now, which
    is what that block's own comment always claimed ("the readouts stay put
    under the pointer"). **A drag source is a hit-testing decision for
    everything under it** — check for one before suspecting a control.
  - **The waveform zoomed through the spin-up.** The ±50 % band alone did not
    gate `tempoPitchPercent`: a platter coming back after STOP, or settling
    after a cut, sweeps *through* the band on the way to the fader's value, and
    the axis followed every intermediate rate. The pitch now has to hold still
    (±0.2 % for ~⅓ s) before anything adopts it — a fader move qualifies, a
    spin-up does not.
  - **The filter floor came back down** to 120 pt. With the ceiling at 520 the
    boxes that need width have it; holding KEY and RATING at a wide floor just
    spent the bar on white space.
- **The filter boxes are content-sized** (the DJ's ask): from the widest of
  the header and the value rows, replacing a flat 168 that truncated every
  Genre row while Colour sat half empty. Pure function, unit-tested
  (`LibraryFilterBoxWidthTests`). The clamps were **doubled to 240 … 520**
  after the first look: measuring the content alone put most boxes *under* the
  168 they replaced, which reads as a regression — this bar is scanned at
  arm's length, and a box sized to exactly its longest genre is technically
  right and too tight.
- **The waveform's time axis is in track seconds, not room seconds — fixed.**
  Two decks beatmatched at different grid BPMs draw their beats at different
  spacings (92/88 = 4.5 % on the rig), diverging away from the playhead, and
  the two strips do not scroll at the same pixel speed when matched — which is
  the premise PRD §9.4 rests the phase meter on. Second, independent one:
  `peaks_chunk_duration_secs` uses the *track's* sample rate, so a 48 kHz file
  draws 8.8 % wider than a 44.1 kHz one at the same BPM with no pitch at all.
  Both fell out of the same fix: `WaveformRenderer.effectiveTimeAxisZoom` folds
  the platter's rate *and* the chunk's own duration into the zoom, so a pixel
  is `referenceSecsPerPixel × zoom` of the **room** on every deck. It scales by
  the held pitch, not the instantaneous rate, or the picture would pump on
  every cut. 44.1 kHz at unity is the anchor and resolves to exactly what the
  renderer drew before, so nothing moves in the common case.
  - It needed a second change to be exact. `columnAggregation` and
    `effectivePixelsPerDrawnColumn` were independent functions of the zoom,
    each rounding on its own — fine for the integer rungs, but with a
    continuous rate folded in, the zoomed-out rungs (0.5× / 0.25×, where the
    column is pinned at one pixel and the aggregation carries the scale) came
    out up to 6 % off, which is the same mismatch reappearing where it is
    hardest to see. The aggregation is chosen first now and the pixel width
    divides, so the product is the requested scale at every rung and rate.
    `WaveformTimeAxisTests` pins the DJ-facing property directly: two decks at
    the same audible tempo, same beat spacing and same scroll speed, whatever
    their grids, pitches and sample rates.
- **BPM did not follow the pitch — fixed.** The scaling existed
  (`liveBpm`) on `DeckHeader`, the header **Prep** renders; Performance draws
  `DeckColumn`, which copied the raw grid BPM across. So the surface with the
  turntables on it was the one showing the unpitched number, and two header
  implementations had quietly drifted. `liveBpm` is on `DeckHeaderState` now —
  one computed property, both consumers.
  - It scales by `tempoPitchPercent`, **not** the live pitch: the smoothed rate
    runs past ±100 % under a hand, where 92.9 would read "−393.3" — nonsense,
    and wider than the BPM slot, so the row jumped on every cut. The model
    holds the last pitch inside ±50 % (a fader's range), so the BPM follows the
    fader and sits still through a scratch. PITCH still prints the truth,
    spikes and all; its slot reserves six places for exactly that.
- **`DubTests` crashed the GPU driver, twice.** Not the app: the test host
  segfaults inside `AppleIntelKBLGraphicsMTLDriver` during
  swift-snapshot-testing's *perceptual* image compare, which runs the diff
  through CoreImage → MPS on the integrated GPU (this is an Intel MacBook Pro;
  `perceptualPrecision: 0.98` is on every snapshot here). It is a flake, not a
  regression — the same suite passes on a re-run — but it means the pre-push
  hook can fail for reasons that have nothing to do with the change. If it
  becomes routine, the fix is to drop `perceptualPrecision` for exact
  comparison plus a small `precision`, which keeps the anti-aliasing tolerance
  without the GPU path.
- **The first rip on the rig turned up four things** (2026-09-22). Three fixed:
  - **The live overview was O(the whole recording), every second, on the main
    thread.** It kept every peak chunk and re-decimated the lot each tick; a
    chunk is 64 samples, so a side arrives at ~750 a second and ten minutes in
    each tick walked 450 000 of them. That is the laggy UI and the overview
    that "updated every 2 seconds". `RipEnvelopeTiler` folds incoming chunks
    into fixed 128-chunk tiles once, and only the tiles — a few thousand for a
    whole side — are re-bucketed per tick.
  - **The capture meter had no ballistics.** `level_peak` was the raw peak of
    whichever block was drained last, sampled at 10 Hz: a different transient
    every poll ("super jumpy"), and a clip flash that lasted one poll, which a
    DJ watching the record never sees. The worker now publishes a peak *hold*
    falling at 20 dB/s, a 300 ms RMS for the bar, and the frame position of the
    last full-scale sample so the UI can latch its clip warning (2 s). The bar
    is the RMS with the peak riding it, the way a recorder's meter reads.
    FFI 77.
  - **Review had no transport.** The segment rows could audition six seconds
    at a time and that was all; there was no way to simply listen to the side.
    A Play / Pause pill sits in the review header now (`ripTogglePlay`, which
    also cancels the audition's auto-pause so pressing Play inside one does not
    go quiet a moment later).
  - **Still open: no waveform while recording.** R-41 was closed on 2026-09-15
    by treating deck A as sourced during capture, and the condition is still
    there and correct (`hasSource` includes `ripCapturing`, `deckAEnabled` is
    just `isRunning`), so the strip should mount. Either the Thru-for-rip peaks
    tap is not feeding deck 0's stream or the mount is being starved — and the
    starvation theory is worth retesting first, because the overview fix above
    removed a large main-thread load that was running the whole time.
- **The second rip pass (2026-09-23)** — the review screen, used in anger:
  - **A segment can be dropped.** Placing a split creates track 2 and there
    was no way to discard track 1; a `dropped` flag on the manifest entry
    (serde-default, so the schema does not move) makes the commit skip it. The
    audio stays in `side.flac` and the indices do not shift, so nothing moves
    under the DJ and a re-split brings it back. Dropping and removing a
    boundary are different edits — the second *merges* — and both exist now.
  - **Space is play/pause in review**, where there is no browser selection to
    load and the hands are on the transport while placing splits.
  - **Split markers carry their track's name** in the overview, and are
    **mirrored into the playing strip** (they ride the cue-marker pass — to
    the renderer a split is the same thing, a full-height line at a track
    position). Judging a split against the overview band alone was the
    complaint.
  - **One transport per card**, and it is that track's. `▶ IN` / `▶ OUT`
    played six seconds of the head and the tail; the DJ reads the first as
    "play this track", so it plays the whole track and the glyph flips to
    pause while the playhead is inside it — at most one card shows pause, and
    Space does the same thing to whichever track the playhead is in. The
    panel-level Play added the day before is gone: a second Play only raised
    the question of what it played that the card's did not. The card's SAMPLES
    lookup went with the OUT button (`SampleLineage` still serves the library
    row menu).
  - **DROP is a bin**, and un-drop is an undo arrow. A glyph that needs no
    reading beats a word that does.
  - **The preview strip jumped — the main thread, not the renderer.** Two
    `TimelineView` clocks (the overview playhead, 4 Hz; the elapsed/remaining
    digits, 2 Hz) each forced a ~15 ms whole-window layout pass, and the strip
    dropped a frame waiting on `nextDrawable` every time. Both are Core
    Animation layers now (`LayerTickers.swift`); in review, window layout went
    574 → 9 samples in 8 s and the `nextDrawable` wait 1072 → 8. It lifts the
    same load off the Performance deck columns, which use the same two views.
    The per-frame colour-band re-copy that was left went the same day (delta
    ingest, under Known gaps): across the two changes the render thread's busy
    time in review fell 1 663 → 20 samples in 8 s.
  - **It came back on the external display** — two bugs that only exist
    there, on the DJ's LG DualUp above the MacBook. (1) The strip renders on
    the integrated GPU (the launch-freeze fix), but a MacBook Pro 16's external
    ports are wired to the discrete one, so every frame was copied across GPUs
    before it could be shown: 3 424 of 3 494 busy samples in `nextDrawable`.
    The coordinator now rebuilds the renderer on the GPU currently driving the
    window's display (`CGDirectDisplayCopyCurrentMetalDevice`) — free, since
    that display has already switched the discrete GPU on — and follows a GPU
    switch or a move between screens. (2) The `CVDisplayLink` was created fresh
    on every Play targeting the *main* display, and `setCurrentDisplay` was
    dropped while the link was stopped — i.e. whenever the deck was paused,
    which is when windows get moved. The render thread now remembers the
    display and applies it to every link it creates. On the LG afterwards: the
    strip on the AMD GPU, render thread busy 3 494 → 1 521, the cross-GPU
    presentation queue 294 → 0, completed frames roughly doubled.
  - **No BPM or key while ripping** (decided 2026-09-23). Tapping a tempo in
    review set a grid on the *spill* — the whole side on deck A, a temporary
    track that encode throws away — and ½ / 2× did nothing because the side is
    not in the library. Each segment gets its own BPM and key at **commit**
    (`commit_segment` analyses that segment's own PCM and writes its grid, key,
    `BPM` and `INITIALKEY`), so a number on the side described nothing a DJ
    keeps. `DeckHeader.hidesTempoAndKey` hides both columns in place during
    capture and review — the header keeps its fixed height, and hidden views
    take no clicks, so the tap and its ½ / 2× menu go with them — and
    `handleTapForGrid` / `applyTapToGrid` refuse during a rip so a bound tap
    key cannot reach the side either. The lever that matters before encoding
    is the card's **Genre**: it is written into the file and the analyser reads
    it to choose the octave (hip-hop 75–105, drum & bass 160–185, reggae/dub
    the one-drop range). A correction made after import lives in the library;
    nothing writes it back into the FLAC's `BPM` tag yet.
- **Second rig session, same day — three more, all fixed:**
  - **Key lock never engaged on a timecode deck.** Not the button: the engine
    required `!has_m6_advance`, and every timecode deck has one, so LOCK lit
    and the voices rode the fader. PRD §6.1.1 always specified it for
    timecode; M14 built it for internal play and refused the rest, because
    the stretcher reads through its own cursor and the M6 re-pin moved the
    playhead without it. The re-pin now moves the cursor by the same
    correction (bypass on anything > 256 frames). The first drift test passed
    with the fix deleted — a sine wobble averages out, and the tolerance sat
    just above the drift — so it now runs the rig's measured 0.3 % bias and a
    one-feed-hop tolerance, and fails without the correction.
  - **BPM snapped instead of following the fader.** The stillness gate added
    to stop the waveform zooming through a spin-up had been applied to the BPM
    readout too. The BPM follows the live pitch now, exactly like PITCH,
    holding its last value only past ±50 % (a hand on the record); the gate
    stays on the waveform axis alone. Rounded to the printed tenth in the
    column, or a raw per-poll value would rebuild the column and bring the
    window-layout jank back.
  - **Calibration was only visible in the SIGNAL panel**, and the deck is held
    until it finishes. `CalibrationBar` lays a bar and CALIBRATING / SETTLING
    PITCH over each deck's overview until it is done — on a layer, so its
    ten-a-second fill cannot re-lay-out the window while the other deck plays.
  - **An empty timecode deck showed no calibration.** The armed-but-empty
    header state never carried `pitchSettled` / `measureProgress`, so the bar
    only appeared once a track was loaded — after the calibration most DJs do
    first.
  - **"A little front and back jump every once in a while", both strips.**
    The engine playhead never went backwards (`make trace-grid-last`): a draw
    read the playhead for its vsync, then waited ~25 ms in `nextDrawable()`,
    so the frame lit up a vsync late — the step back — and the next one a
    double step forward. The cause was the main thread, not the strip:
    `sample` with both decks playing put **30 %** of it in whole-window layout
    (`_layoutViewTree`, 3 278 samples / 15 s), because `deckA` / `deckB`
    republished on every 30 Hz poll — the live pitch wobbles in its last
    digits and the input levels never sit still — and every observer of the
    model rebuilt (the overview re-drawing 480 bars alone was 365 samples).
    Fixed: PITCH and BPM are `LayerReadout`s reading the engine at 10 Hz; the
    stored pitch moves only on a 0.1 % step (`DeckState.heldPitch`); input
    levels are carried only on a DUB FX deck; phase meter / echo / siren read
    the exact pitch off the engine. That freed the main thread into the next
    wall: **57 %** asleep in RenderBox `wait_for_allocations`, from the phase
    meter's full-height `Canvas` in a 60 Hz `TimelineView` (≈ 72 × 2 400 px a
    frame on the Intel UHD 630). Now the gutter is drawn once and the marker
    is a layer (`PhaseMeterMarker`). After: main thread 88 % idle, layout 400
    samples, no RenderBox waits, **0 of 2 330 draws stalled** (was 192), frames
    skipped 0.5–1 % (display-link scheduling — no late frame, so no step back).
    Tried and reverted: re-targeting a late frame to a later vsync
    (`PresentTarget`) — in steady state every drawable arrived ~9 ms "late",
    right on the half-period threshold, and frames flip-flopped (272 repeats).
    Treat the pipeline stall, not the symptom.
    **Then still jumpy while the pitch moved** (fine once settled): the held
    0.1 % pitch still republished the model on every step of a fader move,
    each a ~90 ms whole-window freeze (watchdog: ~20 in 90 s). The model no
    longer carries the live pitch at all; every reader takes it off the
    engine, and the settled tempo moves in 0.1 % steps.
  - **The strip stretched late: "at the very end it jumps into a new zoom".**
    The time axis followed the settled pitch, which moves only once the
    platter is still. It now follows the platter every frame inside the
    fader's ±50 % band (`PlatterAxisFollower`, in the renderer — through the
    model it would rebuild the window per frame), holds outside it, and
    after a stop / spin-up / scratch waits 0.35 s in the band before gliding
    back, so a spin-up still does not zoom the picture. Serato's behaviour.
    First cut still zoomed on STOP and on scratches: a brake and a slow drag
    both pass *through* the band. The follower now also gates on speed — a
    fader moves at tens of %/s, a brake / start / scratch at hundreds, so
    anything over 40 %/s (measured over 100 ms) holds — and follows the
    pitch as it was 80 ms ago, longer than any of those takes to show its
    speed, so their first frames never reach the axis: the zoom is held
    *exactly*.
  - **Agreed next (Daniel, 2026-09-23): split the model.** Deck A, deck B,
    the rack and the library as separate observable objects, so a deck's
    change rebuilds that deck's views and not the window; plus a debug guard
    that logs a deck publishing more than a few times a second in steady
    play. Not a move off SwiftUI — static controls cost nothing; what cost
    was one model everyone observes, and per-frame drawing in SwiftUI.
- **Prefs "output"**: a false alarm — the greyed picker is the DEBUG dev
  section's, and it genuinely does not apply in Performance (the master always
  returns on the interface). The real gap is that the Audio tab has *no* output
  row at all, so nothing tells the DJ where the master is going.

## Next milestone

**M18 — Polish + Alpha** (2–3 weeks). M17 was the last feature milestone;
what remains before trusted-DJ hands is polish, and it carries real
deferred work rather than only cosmetics:

- ~~**Key remapping**~~ — **the keyboard lane shipped 2026-09-15** (step 6
  below): `MAP` on the status strip, click a control, press the key. The
  sampler, Quick Scratch, the siren, ECHO OUT, the hot cues and the DUB FX
  rack's toggles + KICK are all bindable; the map persists
  (`dub.keymap.v1`) as overrides over the defaults — and **there is no
  default keymap for anything played** (Daniel, 2026-09-15): the number
  row, `G` and `⌘←→` went; only Space (load) and `⌘,` ship bound. **No
  MIDI yet** — the MIDI lane is the same mechanism pointed at
  `dub-controller` once that crate is real.
- **The deferred M16 fine-tuning**: siren sound polish (the five shots
  that survived the cut — Rifle Gun · Alarm · Sine · Laser · Siren) and
  the deck-B siren Expert panel (`UI-BACKLOG.md` §5 F-36 / F-37). Note
  F-37's Performance half is moot: the siren is one box in the global bar
  now, bound to the focused deck.
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

### The siren box — shipped (2026-09-12, FFI 74)

The bank was cut from nineteen sounds across three switchable units to
**five shots in one flat bank** — Rifle Gun · Alarm (HK628), Sine (DS01E),
Laser · Siren (SN76477) — each naming its chip in
`dub-engine::siren_bank`, so the unit selector and everything that hung
off it (`SirenUnit`, `set_siren_unit`, `siren_unit_preset_names`, the
per-deck unit) is gone — and so, later the same day at Daniel's request,
are the siren's key bindings altogether: `Z X C V B` fall through, the
caps print no legend, and the siren waits for map mode like the sampler
(`DubAction.sirenPreset` stays for the profile to bind). The DSP chip tables stay whole:
they are the chip recreations, and the cut is a product decision, so it
lives in the engine's bank rather than in `dub-dsp`.

The block was rebuilt as the box a dub DJ owns (study: the *Dub Siren
Faceplate* artifact): a framed faceplate the height of the shelf's two
rows on the deck's `surface1`, with a window, five `DubKey` caps that
sink when pressed, and a `DubKnob` dial DRY → DUB whose readout states
the delay and feedback (`siren_dub_macro_controls` exposes the engine's
own curve). It is the only framed, only metered, only dialled thing on
the surface — the differentiation the pad-surface studies asked for.
The DUB value persists (`dub.sirenDub`) and is pushed into every
freshly started engine; it used to start dry every launch, and 0 is no
echo at all.

The window is a **Sifam-style VU meter** (`SirenDisplay` +
`SirenMeterFace`, a `Canvas`): cream face, red +3 zone, the last shot
printed on the dial, the echo line under it, and a needle that rests on
its stop, throws into the red on a press and falls back through the
repeats. **Eye candy by decision** — Daniel picked it from *The Siren
Window* study over the honest jewel lamp ("this is just eye candy so its
fine"). The siren bus has no level tap; the ballistics are synthesised
from `siren_state`, the press count and the knob's delay/feedback, and
the file says so. The `TimelineView` ticks at 30 Hz only while a shot or
its tail is live. The DUB knob is the one from the photo Daniel sent
after rejecting all four of *The Siren Knob* study: the MXR / Davies-1900
pattern — black phenolic body with eight flutes that turn with the value,
a spun-aluminium diamond-cut cap whose sheen stays with the light, a white
index line down the lobe under the pointer, the scale ticked on the plate
(`DubKnob`, a `Canvas`). No readout under it — the echo line on the meter
and the tooltip carry the numbers.

**Instant double by drag (2026-09-14).** The deck column's identity
block (artist + title) drags as `dubdeck:a` / `dubdeck:b` — the crate
drag's in-process string pattern — and the other deck's `DeckDropTarget`
doubles onto itself (`instantDouble`, the same path as `⌘←` / `⌘→`).
The target moved from `dropDestination(for: URL.self)` to `onDrop(of:
[.fileURL, .plainText])` so one target takes both a file and a deck;
a drop on the deck the tune is already on is ignored. Verified on the
internal mixer: B's tune doubled onto A at the same playhead, cues and
grid along.

**Quick Scratch double — the doubled deck's waveform sat frozen ~1 s
(found and fixed, 2026-09-14).** Daniel's rig log, with the Debug
`MainThreadWatchdog` in: `overview reloadIfStale took 333 ms`, `took
300 ms`, then `main thread stalled 862 ms` on the press; three reloads
and an 1142 ms stall on the release. `TrackOverviewView.reloadIfStale`
pulled the *entire* track's peak chunks across the FFI (2–3 MB) and
reduced them in unoptimised Swift on the main thread, once per deck
whose overview changed — and a Quick Scratch changes two or three. The
render link for the doubled deck is started by a view update, so it
waited behind those reloads; the audio and render threads never need
the main thread, which is why deck A kept scrolling and the sound was
fine. Fix: `DubEngine::peaks_overview(deck, bucket_count)` decimates in
Rust (release-built) and returns 480 `(peak, rms)` pairs — 4 KB, about
a millisecond — and the shell reads that. `OverviewDecimator` stays for
the rip's live overview, which reduces a growing buffer it already
holds. The `stallTimed` fence around the reload and the watchdog stay
in Debug builds so a regression prints itself:
`/usr/bin/log show --predicate 'subsystem == "com.dub.app" AND category == "stall"' --last 5m --style compact`.

**Then the watchdog grew a stack capture** (in-process: suspend the
main thread, walk its frame pointers, resume, symbolicate —
`/usr/bin/sample` was tried first and always attached after the stall
had ended). Full stacks in `~/Library/Logs/Dub/stall-<ms>.txt`, the
top frames in the log. It named two more things straight away:

- **The GPU mux (fixed).** The 2.6 s stall at engine start and the 8.6 s
  one at a cold launch were `CAMetalLayer.setDevice` →
  `layer_private_mux_acquire` → `IOServiceOpen`: this MacBook Pro has an
  Intel UHD 630 and a Radeon Pro 5500M, `MTLCreateSystemDefaultDevice()`
  hands over the Radeon, and the first layer bound to it makes macOS
  switch GPUs, once per waveform view. `NSSupportsAutomaticGraphicsSwitching`
  is in Info.plist now and `WaveformMetalView` takes the low-power device
  (`MTLCopyAllDevices().first { $0.isLowPower }`), shared across views.
  Measured on the internal mixer: 2571 ms → no stall.
- **The Quick Scratch press is ~200–270 ms now** (was ~1 s): what remains
  is SwiftUI / AppKit relaying out the window on a deck-state change —
  `_NSViewUpdateConstraints` under the attribute graph, nothing of ours in
  the stack — at Debug-build speed. The lever is narrowing what a
  `deckA` / `deckB` publish invalidates (`PerformanceView` observes the
  whole model, so both panes, the bar and the library host re-evaluate);
  an architectural pass, not done here. A Release build would show the
  real-world number first.
- **Library load on the main thread (fixed).** At launch the rows'
  landing did the sort (~1.0 s — an ICU collation per comparison through
  `KeyPathComparator`), the facet tallies and the volume `stat`s on the
  main thread, in the same hop that shipped the rows to the table.
  `refreshTracks` now does all three on its fetch task — `sortedRows`,
  `computedFacets`, `WaveformAppModel.probeVolumeReachability` — and the
  main-thread landing is assignments plus one reload; if the sort order
  or filter moved during the fetch it recomputes on main as before.
  **The trap, caught by the watchdog on the first attempt:** `View` is a
  `@MainActor` protocol in the current SDK, so a `static func` on a
  SwiftUI view is main-actor-isolated by inference and a call from
  `Task.detached` hops straight back to the main thread; the helpers are
  `nonisolated` for that reason. Launch stalls went 994 / 1072 ms →
  233 ms (CoreAudio `AudioDeviceCreateIOProcID` starting the device) and
  ~630 ms of SwiftUI's first window layout — no app code in either
  sample; the latter is the Debug-build cost of the view tree.

**Deck column header, rows swapped (2026-09-14).** The artist now
shares the row with BPM · KEY · PITCH, on the numbers' baseline, and the
title has the next row to itself at the column's full width — it is the
one string in the header whose length is not ours to choose, and beside
the readouts it wrapped at the numbers while the artist ran under them
with room to spare. Two title lines stay reserved; five deck-column
baselines re-recorded; the 1440 × 900 fit test still holds.

**The rack folds (2026-09-14).** A chevron column at the bar's leading
edge — the library's `› FILTER` gesture — folds the whole bar to a
22 pt strip (`▸ SAMPLES · DUB SIREN` since the 2026-09-16 swap), and the 112 pt it gives up go to
the library: `DeckLibrarySplit` now takes a `deckChromeBudget` (the open
bar) and subtracts what is missing from the deck side, so the waveform
region keeps its height and nothing above the fold moves
(`test_foldingTheRackGivesTheLibraryTheSpace`). Persisted as
`dub.rackFolded`.

**Levels and idle, fenced.** Laser and Siren measured −3.9 / −2.1 LUFS
through the deck bus against the chip shots' −12.6 / −16.6; all five are
trimmed to **−14.0 LUFS** now (`siren_bank_shots_are_level_matched`, ±1).
And the Sine voice reported "sounding" for two silent seconds after its
release — its dry DS01E patch still recirculated an unheard slap-back and
the idle check waited for it — which the meter drew as a needle that
would not fall. The voice now judges the *heard* tail
(`wet * delay_mix`); `siren_bank_shots_report_idle_as_soon_as_they_are_quiet`
holds every shot to ≤ 0.3 s.

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
- ~~The siren unit switch changes the pad row's length, so every mouse
  target and key-addressed pad relocates~~ — gone with the switch: the
  bank is five shots and the keys never move (the siren box, below).
- ~~`SirenPadRow` still uses a raw AppKit `Picker(.segmented)`~~ —
  `SirenPadRow` is deleted.

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
   **Performance's CUE / LOOP are deliberately untouched** — that design
   is not settled, and `CuePadSection` still draws numbered pads there.
   **The sampler did land on Performance (2026-09-10, FFI 70):** the
   global rack bar's four `TriggerPadGroup` pads are gone and the same
   `SampleShelf` Prep loads now fires there — eight slots, one table
   (`SampleBank` is positional now; the list-backed bank slid pads left
   on every unload), the Preferences "Samples" / "Sampler" sections
   deleted. Press fires on the **master deck** like the siren (`→ A` pill;
   the deck is resolved at the press, so a master switch mid-horn does
   not hop the sound); press again restarts; **right-click stops while
   sounding and offers Unload when quiet** (`onSecondaryClick`, an AppKit
   catcher — `.contextMenu` cannot decide at click time). **Auto-gain at
   load**, the track's −14 LUFS target, with a whole-clip fallback for
   stabs under one BS.1770 block (`measure_clip_loudness`). The tile
   lights and sweeps off `sampler_telemetry`, polled at 30 Hz only while
   something sounds. The rack bar grew 92 → 134 to hold the two-row
   shelf; `rackBarHeight` is derived from it now.
   `SirenPadRow` / `SirenExpertPanel` are kept but unmounted, staged for
   F-37's move to Performance; the file says so rather than letting them
   rot.
6. **Map mode — keyboard lane shipped 2026-09-15.** Serato's mechanism:
   turn on MAP, click a control, press the key. Not a Preferences screen —
   a mode over the live interface, so it is on both surfaces by
   construction. Built as an environment value (`DubMapping`) that
   `.mappable(action)` controls read: off, nothing changes; on, the
   control's own press becomes "arm me", it draws its cap and a dashed
   ring, the armed one pulses and says PRESS A KEY, and
   `KeyEventMonitorHost` hands the next key to `DubKeymapStore` (Escape
   cancels, ⌫ unbinds). One key, one action — binding steals the key from
   whatever held it, default or not; `⌘,` can be neither taken nor
   rebound. The store holds overrides only, so the default map stays what
   a profile is diffed against. The MIDI lane is the same mechanism
   pointed at `dub-controller` once that crate is real — the missing half
   of Expert FX ("a real MIDI controller used instead of a second
   turntable"); a binding already carries its transport.

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
  rack is a deck role, and the switch is how you select it. **The pane's
  design and its routing were settled 2026-09-15 and stage 2 shipped the
  same day (FFI 75)** — the mixer's send is the input, the siren reaches
  the rack through its own `→ FX` output pill, the bar re-orders rather
  than being taken over; the full description, what shipped and the design
  canvas are on F-38 in `UI-BACKLOG.md`. Stage 3 — the MIC sum and a
  measured input meter — shipped the same day (FFI 76).
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

**The vintage FX rack has its home** — the `DUB FX` deck role, F-38 stage
2, shipped 2026-09-15. Off by default (Preferences ▸ FX).

**Placement rule, for future features.** A capability belongs where the DJ
is in that state of mind. Prep is couch work with no rig; Performance is a
record running in front of people. Corrected against that: pitch and key
lock are Performance-only (Prep is always `+0.0 %`); every FX is
Performance-only; loading a track is drag-and-drop or a key, never a
button; crates, imports, analysis and keymap remapping are reachable from
**both**; and sample *loading* is Prep work even though sample *firing* is
not. That group is the surprise — Prep is not "the library mode". The two
surfaces differ almost entirely in the deck.

**The beatmatch aid is the phase meter now (2026-09-16).** Stillpoint
(round 3) and the dead round-2 candidates (`PhaseClockView`,
`BeatmatchViz`) are deleted; `PhaseMeter.swift` is Traktor's one-beat
meter standing in the gutter, late above the line, the incoming deck's
beat against the master's. PRD §9.4 rewritten; the Stillpoint sub-spec
stays in `docs/investigations/` as history. `PerformanceLayoutTests`
follow the renamed `phaseMeterGutterWidth`. Same day, on the DJ's ask:
the gutter is the meter's own width now (88 → 36 pt; the zoom control
stands vertical in it), each playing strip is a fifth narrower
(70/106/150 for min/ideal/cap) and both cuts went to the deck columns;
and the rack bar reads sampler · siren everywhere, the order the FX
channel on deck B already used (`GlobalRackBar.sirenLeads` — only an FX
channel on deck A swaps them back).

**The debug watchdog could freeze the app (fixed 2026-09-16).** Its stall
capture suspended the main thread and then allocated; when the suspend
landed inside `malloc` the whole process deadlocked on the zone lock at
0 % CPU — the "not responding" after a DUB FX session, five minutes into
idle. `MainThreadHandle.backtrace` now allocates before the suspend and
builds its result after the resume; the rule is in LESSONS (build +
test hygiene). Debug builds only — the watchdog is `#if DEBUG`. An
Address-Sanitizer variant of the app builds into `apple/build-asan`
(gitignored) with `xcodebuild … -derivedDataPath apple/build-asan
-enableAddressSanitizer YES`; it was the wrong tool for this one but
is the right one for a real double free.

**Post-1.0, on the roadmap (2026-09-16): M27 — UI themes.** Light for
daytime DJing and Kingston (greenish olive military tones), beside the
dark default, chosen in Preferences. PRD §12.2. Nothing to do before
v1.0 except keep every colour in `DubColor` — the FX rack's faces are
the one place hex literals live in a view, and they are the hardware's
own paint, not the theme's.

## Recently shipped (detail in `SHIPPED.md`)

- **M17 — Sampler, Quick Scratch & Instant Doubles.** All three of §7's
  trigger mechanisms. **Instant Doubles** duplicates a deck's track onto
  the other at a sample-accurate playhead — done *on the audio thread* off
  the loaded `Arc<Track>`, which is what makes it sample-accurate and
  instant. **Quick Scratch** loads a bound sample through the library's own
  load path. **The sampler** is additive one-shot voices summed onto the
  deck bus after the FX, reading rate-converted buffers with an integer
  cursor because the conversion happens at bind time off-RT. Shipped as
  four voices bound in Preferences (FFI 66); now eight, the Prep shelf
  itself, auto-gained, on the master deck (FFI 70 — see the Prep overhaul
  step 5 above). **Quick Scratch** was a hotkey fast-load with no way
  back; it parks and returns now (FFI 71, same place).

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
- **Waveform renderer, second-order CPU** (noted 2026-09-10). Two of the
  three leftovers are done (2026-09-23), measured on this Intel MacBook Pro
  in rip review with the side playing, 8 s per build:
  - ~~(1) every frame re-ingests the whole 8 192-chunk window~~ — **done**.
    Each GPU buffer tracks which chunks it holds (`PeakRingCoverage`) and a
    frame fetches only what is missing: a sliver in steady play, the window
    once after a seek, nothing on a rewind. Render thread busy
    805 → **20** samples; the colour-band copy, 79 % of what was left,
    639 → **0**. Frames committed and presented at the same rate throughout.
  - ~~(3) `NSEvent.pressedMouseButtons` read per frame~~ — **gone** with the
    old dedupe gate it belonged to; exact coverage makes it unnecessary.
  - (2) Two array literals in `drawBeatGrid` (loop-edge pair, per-cue
    halo/stem pair) still allocate on the render thread per frame. Free to
    fix, not measurable any more.

## Keeping this file honest

Update it in the same change that ships a milestone or opens a gap — the same
rule as `SHIPPED.md`. If a section here is longer than a screen, it belongs in
`UI-BACKLOG.md` or the PRD, and this file should link to it instead.
