# Current state

> **Read this first.** `SHIPPED.md` says what is done; this says what is *in
> flight*, what is blocked, and what to pick up next. Keep it short — a page
> that grows stops being read, and an unread status page is how
> `docs/html/` drifted ten milestones before anyone noticed.
>
> Last updated: 2026-11-19 (alpha.5 + main). Everything shipped up to alpha.2, and
> the rig rounds before it, moved to `SHIPPED.md` on 2026-11-15.

## Where the branch is

`main` — **1.0.0-alpha.5 released** (tag `v1.0.0-alpha.5`, a GitHub
pre-release; notes in `docs/releases/v1.0.0-alpha.5.md`). alpha.4 was tagged
but never released: replaced the same day to ship Serato's ⌘ keys. `make dist` builds
a universal Release `Dub.app`, ad-hoc signed and **not notarized** (no
Developer ID yet), zipped with `ditto` plus a SHA-256. Version lives in two
places, kept in step by hand: `[workspace.package] version` and
`CFBundleShortVersionString` in `apple/project.yml`. The rig checklist is
**not** signed off — both alphas went out with section 0 partly open.
Pushing is a deliberate act: `.githooks/pre-push` runs fmt-check + clippy +
docs-check + the Rust suite + the Swift snapshot suite first, and that last
gate is local-only because GitHub CI has no macOS app job.

**In alpha.3, since alpha.2** (none of it on the rig yet):

- **The signal panel speaks the DJ's language (FFI 85, 2026-11-15; canvas
  *Dub Signal Panel*).** It printed the decoder's own numbers (CONFIDENCE,
  AMPLITUDE); it now says one state — locked / weak / no lock / no signal /
  no input — with one line of what to check, and levels in dB
  (`SignalStatus`, pure, `SignalStatusTests`). A timecode deck's input is
  metered now, not only THRU and DUB FX, and `deck_scope` hands the panel
  the last block of raw input for its scope: the carrier's circle, squashed
  by a weak channel.
- **Each deck's output, metered (FFI 86, 2026-11-15; canvas *Dub Output
  Meters*).** The stage question "the fader is up, why is nothing coming
  out?" — most often ECHO OUT left on after its tail died, the record still
  playing under it. The engine meters each deck's output pair after
  everything (track, echo out, rack, the siren and samples routed to it,
  master gain): `DeckTelemetry.output_peak` / `output_routed`. Two slim
  meters stand in the gutter under the phase meter (`OutputMeter.swift`):
  24 × 2 dB from −48 dBFS, an echo tail in violet, and **ECHO OUT** /
  **NO OUTPUT** written up an empty meter when it is empty for a reason.
  No master meter (Dub never mixes) and no gain readout (auto gain sets
  it). The meters poll the engine on their own 30 Hz timer, not through
  the model — a level never sits still.
- **The top bar:** the clock and battery sit in its centre; an error stays
  up 10 s, not 5 (Daniel: gone before a DJ mid-mix looked up).
- **The DUB dial draws from a typed function** — the universal Release
  build timed out type-checking it inside its `Canvas` closure.
- **The stage rescue (P-33, P-34 folded in).** INT latches and TC picks
  the deck up where INT has it — both already true in the engine, now
  pinned by tests. New: ↻ Recalibrate on INT, so a new needle or record is
  calibrated while the set plays on, and a lamp in the TC segment while
  INT plays — amber measuring, green ready to hand back.
- **The DJ routes an interface Dub does not know (FFI 87; canvas *Dub
  Audio Routing*).** `devices.toml` knows two boxes and only the SL 3 is
  played on; any other fell to one deck on inputs 1+2 and a summed
  output. Preferences ▸ Audio now shows the interface and each deck's
  pair in and out — read-only for a known box (KNOWN · TESTED), pickers
  for any other, one deck or two, a meter and a word on each input
  (timecode / music / nothing), TEST for a quarter second of 1 kHz on a
  deck's pair, and a clash message when two decks share a pair (the
  last routing that worked stays). Saved per interface name
  (`AudioRoutingStore`), handed to the engine before every start
  (`set_routing_override`, which also wins in `performance_routing_for`).
  Applying reconnects the interface and empties the decks, so it is
  locked while a deck plays. An unknown interface nobody has routed gets
  one quiet notice pointing at the tab.
- **A sample pad says when its file is gone (2026-11-16).** A SAMPLES
  tile or Quick Scratch pad whose file is missing or unreadable wears a
  red **MISSING** / **UNREADABLE** badge where its shape would be (it
  read as still loading); Quick Scratch refuses it rather than parking
  the tune for silence. **Rename…** on either pad's right-click names
  the slot (`SampleSlot.name`, persisted; a new file drops the name).
- **C-30, the poll's Swift-side allocations, have a baseline**
  (`PollAllocationTests`, numbers on C-30 in `UI-BACKLOG.md`).
- **The DJ manual** (2026-11-16): `site/`, a Mintlify site published from
  `main` at https://dub-3ccc829c.mintlify.site. Writing rules in
  `site/AGENTS.md`; UI words are copied from `apple/Dub/`, so a renamed
  label means a manual edit in the same change.

**Since alpha.5, on `main`:**

- **Browser adjustments (2026-11-20, Daniel; uncommitted, built; he
  found the text size and its letter spacing right; library snapshot
  baselines NOT re-recorded).** The
  waveform zoom starts at 0.5× and is remembered
  (`dub.waveformZoomIndex`). The browser's text size is a setting,
  Preferences ▸ Library ▸ Text size, 12–24 pt, default 18 with 31 pt rows
  (was a fixed 14 / 28, "much smaller than in Traktor" — whose 14 and 21
  are not points). A click on a folder in another app's library lists
  every track under it (`imported_crate_tracks` walks down); only the
  arrow folds. No row-height setting and no shortcut for the size: the
  one setting is enough (Daniel, 2026-11-20).

- **Interfaces: a map set by hand can be sent in (2026-11-20, Daniel:
  "save the soundcards i connect so we can adapt them as preconfigured";
  uncommitted, built, not tried on an interface).** Dub already kept a
  hand-set routing per interface (FFI 87). New: Preferences ▸ Audio has
  **Copy map**, which puts the routing on the clipboard as a
  `devices.toml` entry; and an interface Dub knows only by an untested
  guess (the Traktor Audio 6) is no longer locked — the guess is where
  the pickers start. The Traktor Audio 6 went in as verified on
  2026-11-21 (decks on 3+4 and 5+6, read from the routing Daniel saved
  and played); still waiting on the DJM-250MK2. Same day: Preferences ▸
  Audio sets the TA6's own Direct Thru and Phono switches per deck
  (`TraktorAudioBox`; NI's control panel is 32-bit and no longer runs).
  Read path tried on Daniel's box; the write is his to try.

- **The sticker holds (2026-11-24, uncommitted, not on the rig).** Daniel:
  juggling, the samples should stay where they sit on the record, and a
  skip must not move the track — "the whole point of relative mode".
  Audited with `dub-engine/tests/sticker_drift.rs` (his deck A scratch
  capture and Traktor take through the engine, plus a synthetic juggle on
  both vinyls). Found and fixed: the LFSR slicer re-learned a level change
  over 330 cycles, so the lock never came during a scratch (0 locks in
  10 s; out 22 ms behind) and short juggle phrases stacked to −64 ms;
  acquisition was forward-only, so a spin back never locked; Traktor MK1
  had no absolute decode at all (+1.2 ms a pass, unbounded) — xwax's defs
  lock 96.7 / 99.2 % of his 12-minute takes, LUTs shared across decks
  (32 MB a side); the heal's 4 ms deadband left the sticker up to 4 ms off
  (now a 0.5 s exponential pull, still capped at 1 %); a skip inside a
  scratch was healed toward the skipped groove for minutes (now whole-turn
  jumps move the anchor); the heal read the playhead a block early. Now:
  the real scratch comes out 0.3 ms off (median), juggles land within a
  millisecond on a typical pass, 4 ms at worst. **Not done:** a juggling
  capture on the rig, both vinyls; the relative path still under-reads a
  fast spin's travel ~0.7 % (the rate smoother's lag), which an unlocked
  spin back carries to the release; a dust tick still slows the deck for
  a moment (~3 ms, healed within a second). Until the entry below, the app
  read Serato vinyl only (every path attached `TimecodeInputConfig::default()`,
  hardwired to `SeratoCv02`), so the Traktor half reached the app only with it.

- **Calibration names the record (2026-11-24, uncommitted, not on the rig).**
  Daniel: detect the vinyl at calibration, 33 and 45 both, no runtime
  switching, and show each deck's result. Each TC deck now builds a decoder
  per format at attach (LUTs shared, 64 MB for Traktor) and, while
  calibrating, names Serato / Traktor MK1 / MK2 at 33⅓ or 45 from a second of
  steady carrier plus which position code locked (`dub_timecode::detect`,
  `TimecodeInput::probe_vinyl`), then swaps that decoder in with its speed
  set. Whitening, the zero anchor and the wobble fit wait for the name. At 45
  the decoder reads 1.35× as unity (long-lag and band-pass where they were
  tuned); positions are track time, the skip rule's turn is 1.333 s, the
  wobble fit stays in groove time. The SIGNAL panel has a VINYL row (`SERATO ·
  33`, then *Position · the sticker holds* or *Speed only · no sticker hold*)
  and the calibration bar says `CALIBRATING · TRAKTOR MK1 · 45`. The PRD's
  Preferences RPM override is dropped (§5.4.1). FFI 96. Tests: all six records
  and speeds named and playing at unity through `drive()`, MK1 at 45 −6 %
  told from MK2 at 33⅓ by its code, same-record recalibration untouched,
  detection allocation-free (it wasn't: `detect` collected a `Vec`), an engine
  test of MK1 at 45 +2 % moving the track 1.02 s a second. **Not done:** the
  rig — Phase into the SL3 on line (as recorded, it should read `TRAKTOR MK2 ·
  33 · Speed only`), then Traktor vinyl at 33 and 45; a 33↔45 switch mid-set
  plays 35 % off until CALIBRATE, as agreed.

- **Phase measured against the record under it (2026-11-24).** Daniel
  remembered Phase on Traktor as "it worked but it was always off". The
  Phase remote on a control record, the needle on it into deck A, the
  receiver into deck B, recorded together (`tests/phase_drift.rs`; the
  takes are in `testdata/timecode/full/`, not in git). Neither mode sends a
  position code (Serato mode: a flat 1 kHz; Traktor mode: a flat 2.5 kHz).
  Phase is the record's motion **8 ms late** (Serato mode, correlation
  0.9998) or **10 ms** (Traktor mode, 0.998). **Serato mode reads 0.56 %
  fast**, 5.6 ms of drift a second, beyond the zero anchor's ±0.4 % guard;
  Traktor mode reads true. **Phase's own count**, its tone's cycles counted:
  in the clean Serato-mode take, its 0.56 % taken out, it came back within
  3 ms of the record after 229 s of juggling and backspins, while the
  engine adding up speed lost 66 ms more. So "always off" was the speed
  error plus the software adding up speed, not Phase's sensing.
  **Corrected the same day:** the Traktor-mode take broke 143 times in the
  input stream (a build ran beside it; the fault detector, now gated on a
  strong carrier, had also fired at scratch turnarounds), and each break
  re-zeroed the count against the record — its "±2 ms" and "1.2 ms" were
  the harness, not Phase; the harness now flags faults inside the measured
  stretch. **The clean Traktor-mode take (2026-11-25,**
  `phase-vs-traktor-1125-traktor-mode-4ch.wav`, nothing else running, faults
  only in the first 2 s): 11 ms late (0.993), speed true to 0.01 %, the
  count within 7 ms over the last 105 s of steady play, but two minutes of
  juggling moved it +80 ms (+91 at the end; the engine adding up speed
  ended −16). The gain sits where the record's code goes unread (stops,
  turnarounds, hand on the record); Phase never passed 5×, so the 2.5 kHz
  carrier didn't run out of room. Unsettled: Phase in Traktor mode, or the
  remote, loose on the label (no magnet yet) and knocked once in the take,
  shifting at hard stops. Daniel: the remote was as loose in the Serato-mode
  take, which held. **So Serato mode it is (2026-11-25, uncommitted):** a
  deck that names Phase on a Serato carrier reads it 0.564 % slower
  (`Decoder::set_carrier_scale`, `PHASE_SERATO_MODE_FAST`). On the Serato
  take the engine's count now ends −1.9 ms after 229 s of juggling (it was
  +510), and the panel asks Traktor-mode Phase for *Set Phase Manager to
  Serato*. The 1.00564 was fitted on that same take, so a second
  Serato-mode take, out of sample, is owed. **Built the same day (uncommitted, not on
  the rig): Preferences ▸ Beta ▸ Count Phase's signal** (a new Beta tab,
  setup steps beside the switch; `site/hardware.mdx#mwm-phase-beta`).
  Calibration names a flat, codeless carrier **PHASE** (level spread under
  0.3 %), and with the switch on the decoder counts its cycles on the raw
  input (counted after the 120 Hz high-pass it lost a third of a cycle at
  every pass through zero) and the engine drives the deck from the count's
  deltas. Synthetic juggle: within 0.2 ms of the sticker over 16 passes,
  against 12–15 ms adding up speed. Calibration names Phase in
  Serato mode PHASE with no code (`real_records_are_named_at_calibration`,
  an excerpt of the take's deck B), though a bare `Decoder::with_absolute`
  left running on that tone reports locks at confidence ≥ 0.9 now and
  then: not chased. **Counting real vinyl** (Daniel asked; measured, not
  built): against the record's own code, −3 / −21 ms over 310 s of steady
  play with fader moves (adding up speed +37 / +85) and +36 ms over the
  juggling take (+106). About three times better than speed for MK2, but
  it loses whole cycles to dust ticks (deck A's error moves in exact
  1.0 ms steps) and more at stops, so no sticker hold; a tick filter would
  need an MK2 recording, which we don't have. FFI 96 (`set_phase_counting`,
  `DeckTelemetry.vinyl_synthetic` / `vinyl_counted`). **Not done:** played
  in the app; a flat battery or lost radio link untested. The Traktor take first looked like the
  MK1 decoder relocking 0.35 s off; it was the input stream dropping audio
  while a build ran beside the capture (both decks' carriers broke at the
  same instants), and the harness now takes such faults out.
- **The SIGNAL drawer opens for calibration (Daniel, 2026-11-25,
  uncommitted, not on the rig).** A TC deck's first calibration of the
  session opens its drawer by itself; once calibrated and LOCKED for 1.5 s
  it slides away to its tab. It stays while something wants fixing (no
  input, a weak side, Phase uncounted or in Traktor mode; MK2's speed only
  asks nothing), never opens by itself again that session, and the DJ's
  click or × ends all of it (`SignalDrawer`, held by the model so Prep
  doesn't reset it). The bar that covered the track overview while a deck
  measured is gone; the panel's calibration line has the bar and the
  percent beside **CALIBRATE**, a refusal fills it red, and a closed tab
  fills as the deck measures. The drawer now slides as one piece on an
  offset (it was a width animated from zero, re-laid out every frame) and
  its content leaves the tree once it has slid away. Prep's header line
  stays: there is no drawer there. First try in the app (2026-11-26) found
  two layout bugs, both fixed with a layout test each: shut, the tab slid
  off the screen (the panel's width hung on a `Group`, which went with its
  content); and open, the panel needed 490 pt of a 438 pt deck pane, so it
  stretched its deck row and pushed the other deck down. The drawer is an
  `.overlay` on the pane now, and the scope gives way when the pane is
  short. Swift suite green (426 + 2).

- **The deck header, mirrored and draggable (Daniel, 2026-11-26,
  uncommitted).** The readout row is one mirror: deck A's labels hang right
  and its key lock sits *before* the key, deck B's hang left with the lock
  after, so the key is as far from its lock on both (snapshot
  `readouts-both-decks`). The whole header, switch row to title, is the
  instant-double drag handle now, not the title and artist text alone, and
  it carries the library's `♪ Artist — Title` chip: the drag lies *behind*
  the header, so the switch, BPM's tap pad, STEADY and the key lock keep
  their presses (a drag source on the block made the lock inert,
  2026-09-22). Not tried with a mouse yet.

- **Phase on TC (2026-11-24, probe only).** Daniel's MWM Phase Essential,
  Phase Manager set to Serato, into the SL3 on line: both decks give a clean
  2.5 kHz quadrature sine (−9.4 dBFS, constant amplitude, so no position
  code). `decode-timecode --format traktor-mk2` reads +1.0002 at confidence
  1.0; as Serato it reads 2.5×. Captures:
  `testdata/timecode/full/phase-serato-1124-deck{A-line,B-probe}.wav`. On
  phono the receiver clips flat. Also found and fixed in `dub levels` /
  `dub capture`: `--channels 6` read only inputs 1–2, three times over, and
  kept a third of the frames — unmapped, the input unit fills the device's
  first stream only, so more than a pair now gets the identity channel map
  (not yet re-run on the SL 3; `--input-channels N,M` always worked). And a
  capture run in the background stopped at 0 frames, its empty stdin taken
  for Enter; end of input no longer stops it.

- **Turntable pitch read true (2026-11-21, uncommitted).** Daniel beatmatches
  by lining up the numbers, and found Traktor's steadier and truer at ±8 %.
  Measured on four captures of his rig against counted carrier cycles, then
  fixed in the decoder: the long-lag estimator gets the ellipse correction
  and the roundness gate is gone, and near playing speed the carrier is read
  through a band-pass that follows it (LESSONS, timecode). Serato vinyl and
  Traktor MK1 now both within 0.06 % across the fader on both decks (Serato
  was 0.15 %, Traktor 1.2 % with 3 % jumps). A value that rounds to zero prints `+0.0` (`PitchText`):
  the sign flipped on a resting platter. **Nudges, measured on the same captures:** a 0.1–0.35 % nudge was
  half shown 0.9 s after the platter. The display filter's noise band and
  confirm time are halved and its steady pole slowed to 2.0 s
  (`display_rate.rs`): half shown at 0.45 s, within 0.05 % at 0.2 s (was
  1.05 s), for the same digit flicker on a still fader. **Not done:** the left deck reading about 0.05 % high
  at the zero click on Serato vinyl, and Daniel's
  own check on the rig. **`dub capture` (2026-11-22):** it opened N
  channels and read the first pair, so `--input-channels 3,4,5,6` gave a
  four-channel header over stereo data and `--channels 6` could not be
  closed; it and `dub levels` now read every channel, through a 10 s
  ring. The four captures above were therefore made by two `dub capture`
  processes at once, and the **Serato take is faulty**: 722.08 s of
  audio in 717.83 s (+0.59 %, no overflow counted), with the carrier
  jumping phase in 512-frame steps for about a second every 38 s on both
  decks. The Traktor take beside it is 718.933 s in 718.927 s. Capture
  now warns when the two disagree by 0.1 %. **Fresh Serato take, one process,
  both decks (2026-11-22, `sl3-serato-1122-0844-*`):** 324.299 s in
  324.297 s. Bare decoder +0.004…+0.043 % on both decks; shown pitch
  with the first 3 s skipped −0.015…+0.055 % left, ±0.023 % right. The
  same 512-frame phase steps are there for 0.1 s at stream start and
  again at 1.05–1.7 s, on both decks, with audio repeated 16384 frames
  later — below our code (SL 3 driver or HAL). Replayed from the first sample the zero anchor
  averaged that glitch in and the left deck read 0.08 % low all take.
  **Fixed:** the zero capture is judged in 0.1 s slices and starts over
  when two differ by 0.2 % (`ZERO_ANCHOR_STEADY`), giving up after 15 s;
  same replay now −0.03…+0.04 % left, ±0.025 % right. Not tried in the
  app. Left at −5…−8 % reads 0.05 % above the rest: not chased.

- **Re-analyze gives Dub's grid (2026-11-22, uncommitted, FFI 94, not
  tried in the app).** *My Boo* (Ghost Town DJs): Traktor's 130.09
  slides 49 ms over the track; Daniel unlocked and re-analysed, Dub
  measured 130.07, stored it and kept showing Traktor's, because an
  imported grid outranked `auto` even unlocked. Re-analyze and Reset
  grid now go through `reanalyze_track`, whose grid takes the slot on
  an unlocked track; an unchanged import no longer takes it back
  (PRD-BEATS §4 table). Not done: his track still has the tap grid at
  130.09 active until he re-analyses it; a change of app order
  re-picks unlocked tracks.

- **The DJ's word on following, and an app's library hidden
  (2026-11-22, uncommitted, FFI 94, schema v13, built, not tried in the
  app).** *Beat grid ▸ Automatic / Follow the band / Even grid* on the
  library row's right-click and the deck BPM's, while the following grid
  is on and the grid unlocked (`tracks.grid_follow`,
  `follow_grid_always`, `set_deck_grid_follow`; PRD-BEATS). The drift
  mark now reads "The grid's tempo is slightly off … Re-analyze to
  correct it, or Lock grid to accept" (*My Boo* read as a case for
  following and was a tempo 0.018 BPM out). Right-click an app in the
  sidebar ▸ Hide … Library: its grids, keys and cues stay in use, its
  node and playlists go, and search leaves out tracks reachable only
  through it; Preferences ▸ Library ▸ Show in sidebar undoes it
  (Daniel: "i use itunes and want traktor beatgrids"). Not done: the
  FOLLOW / STEADY labels and icons on the deck are as they were; Swift
  tests and snapshots not run (the drift sentence and both BPM menus
  changed).

- **Search finds the record wherever it is (2026-11-22, uncommitted,
  FFI 94, built, not looked at in the app).** Daniel: "i think i want
  everywhere not just the current playlist". The search still narrows
  the list on screen, in its order; under a divider come the matches
  from the rest of the library (`search_anywhere`), and a line under
  the search field counts both ("3 in this list · 12 elsewhere").
  Traktor does this with Enter; here it needs no key. A column sort
  sorts within each group; crate drag-reorder is off while searching.
  No snapshot of the divider yet.

- **Loudness for tracks analysed before there was one (2026-11-22,
  uncommitted, FFI 94, not tried in the app).** Analysis runs once per
  track and measured no loudness before 2026-06-10: 31 tracks in
  Daniel's library were analysed, had none, and played at unity on
  every load; 14 carry a set-the-1 grid and 5 a lock, so Re-analyze was
  no cure. A deck load now measures the loudness alone for such a track
  (`loudness_prepare` / `measure_loudness` / `loudness_commit`,
  `backfill_track_loudness`), grid and lock untouched; the gain applies
  from the next load. "Night Shift - Bonanza" (2026-06-22, no loudness,
  no waveform) is unexplained and will be measured the same way.

- **The following grid on the band's pulse, and a STEADY switch
  (2026-11-20, Daniel: "tempo tracker i need to judge. please wire it in.
  i would want to try the transition way"; uncommitted, built, FFI 93,
  none of it heard or seen in the app yet).** Both are behind
  Preferences ▸ Alpha ▸ Following beat grid. The followed grid's lines
  are the pulse (`band_pulse`), with the hit chain deciding whether a
  track is followed and which side of the beat the lines are on; unlocked
  tracks are followed afresh at every load. **STEADY**, beside LOCK on a
  deck whose grid follows, holds the record to the band's own middle
  tempo while it is on (`dub_engine::steady`, `set_deck_steady`): the
  transition switch. Details and what is open in PRD-BEATS, "Wired in"
  and "Steadying". For Daniel to judge: whether the lines sit right
  (known weak: whole sections half a beat off on some tracks), and
  whether steadying sounds right and helps a mix. Not done: a key for
  STEADY in the key map; snapshot baselines for the deck row.

- **A click on a playlist: 670 ms → 150 ms (2026-11-20, Daniel:
  "Traktor clicking of the library is instant … can we get there?";
  uncommitted, built, measured with `LibraryBench` in a Release build on
  his library; library snapshot baselines NOT re-recorded and they will
  differ).** Median from the pick to the main thread at rest, over lists
  that fill the screen; the 2,183-track Apple Music node went 900 → 245.
  The library itself answered in 10–30 ms throughout, so all of it was
  the app:
  - `reloadData()` took every row view, a SwiftUI host per column, off
    the table and back: half a second. The table refills the rows it
    holds (`Coordinator.refillRows`).
  - The **folded filter bar** was kept current out of sight — five boxes
    given new rows and widths for every list: 290 ms. Folded, the boxes
    hold nothing (`boxFacets`); opening the bar fills them. Open, the bar
    costs about 70 ms a click.
  - The **sidebar's rows** were rebuilt and compared on every change to
    `LibraryView`'s state: 90 ms. Each is now a `SidebarNodeRow`,
    `Equatable`, skipped unless its own values moved.
  - The **cells**: 170 ms for some 340 SwiftUI hosts. A column that is a
    line of text is drawn by AppKit (`LibraryTextCellView` from
    `LibraryCellText`); BPM, rating, colour and the gutter stay SwiftUI
    and are skipped when they already show the value.
  - The **sort** went through `KeyPathComparator`: 175 ms for 2,183
    titles, 22 comparing the strings (`sortedByText`).
  Each list logs its phases (`category == "list"`), now from the pick
  and with the redraw split by phase. `open Dub.app --args
  -dubLibraryBench 1` clicks through nine lists by itself. What is left
  of the 150: ~35 ms before the rows land (the pick's own pass through
  the window), ~17 fetch, ~100 for SwiftUI's update, layout and drawing.
  Not done: the pick still fires on mouse-up (`onTapGesture`), which is
  a finger's 60–100 ms that Traktor's mouse-down does not spend; the
  filter bar when open; the 105 ms fetch of a 2,183-row list.

- **Four things from the 2026-11-19 session (Daniel; uncommitted, built,
  none tried in the app, Swift tests compiled but not run).** The right
  deck's **LOCK** lit in its deck tint, teal, beside amber controls — it
  takes `DubColor.controlAccent` like every other control. **Quick
  Scratch** now drops an engaged echo-out, on its deck and on the deck the
  tune doubles to. The **phase meter** read every grid as one tempo and one
  anchor, so on a followed track the marker wandered with the band; it
  now reads the grid's own beats (`PhaseMeterInputs.followedBeats`,
  fetched once per grid generation). **Echo out keeps the master's time**
  when the other deck is the master and playing
  (`WaveformAppModel.echoOutTempo`).

- **Search looks in the list on screen (FFI 92, 2026-11-19, Daniel: with
  the Apple Music node selected, "searching for any song gives me 0
  results"; uncommitted, not checked on the rig).** Search went to the
  collection whatever was selected, and 2,540 of his 2,654 tracks are in
  other apps' libraries only. Now All Tracks searches the collection as
  before and every other list is narrowed to its rows the search matches
  (`DubLibrary::search_track_ids`), keeping the list's own order.
- **One analysis result no longer re-sorts the library list (2026-11-19,
  Daniel: "a lot of beachballs" while skipping through tracks;
  uncommitted, not checked on the rig).** The stall log had the main
  thread in `sortedRows` and `LibraryTrack.==` for 4.3 s and 3.7 s: each
  result re-sorted the whole view and compared it with the old one, and a
  load fires several. The changed row is now overwritten in place, or
  moved by a binary search when what it is sorted or filtered by changed
  (`LibraryView.repositionRow`). The Swift snapshot suite was not run.

- **The tempo a band keeps can be read (2026-11-19, Daniel: quantize the
  tracks whose tempo "is simply drifting", leave megamixes and real tempo
  changes in follow mode; uncommitted, a library function only — the app
  does not call it yet).** The followed beats could not carry that
  decision: on tracks locked in Traktor *and* at Dub's tempo they read a
  four-bar tempo spread of 2.6 %, the same as on unlocked ones. `band_pulse`
  (`dub-bpm/src/pulse.rs`) reads the tempo bar by bar instead, all bars
  chosen together. `tests/follow_corpus.rs` is its benchmark: locked
  Traktor tracks as they are, and bent ±1 % by the new stretcher. Outside
  the first and last 20 s, 91 % of steady tracks never read more than
  0.5 % off (follow's chain: 31 %) and 87 % of drifting ones (27 %).
  Over the library (2,547 tracks, body of each): Traktor-locked tracks
  swing 0.19 % at the median, unlocked 1.94 %. **Not done:** which of beat
  and off-beat the pulse sits on is a guess (more onset wins); a rule for
  "one tempo that wanders" against "changes tempo" (bands move 1–2 %
  within a few bars, so slowness is not the test — a held step is);
  follow mode still draws the old chain; nothing is stretched.

- **Key lock has a new stretcher (2026-11-19, Daniel: steadying a band's
  tempo "first needs a good key lock"; uncommitted, not on the rig, not
  judged by ear yet).** Measuring the M14 WSOLA found four faults: sub notes
  under 48 Hz followed the platter (133 cents sharp at +8 %), drum hits
  wandered up to 11 ms with some doubled, the record lost 5 dB at 12–16 kHz
  because the deck interpolated it twice, and a locked deck played ~10 ms
  ahead of its own playhead. `SpliceStretcher` replaces it in the deck: the
  record copied untouched, joined before hits and where it repeats, bass and
  the rest apart (PRD §6.1.1 has the table). No FFI change. `WsolaStretcher`
  stays for `dub stretch-bench` only — delete it once the new one has been
  heard on the rig. Listening pairs were rendered for Daniel (reggae,
  dubstep, dnb, funk at ±6 %); he heard less bass in the new one, most on
  the dnb track, and it measured 0.3–1.7 dB short below 170 Hz — fixed the
  same day with a steep bass split at 150 Hz (now within 0.2 dB on those
  four), not yet re-heard. Next on this path: straightening a
  variable-tempo track to its grid, which should render offline through
  this same stretcher, one ratio per beat, so the result scratches and
  reverses like any other track.

- **Tempo is read inside one octave; the rule pile is gone (FFI 91,
  2026-11-19, Daniel: "the quality of analysis is pretty poor").** Scored
  against his 1,758 locked Traktor grids the detector got 62.4 % of tempos:
  a reggae rule threw the right tempo away on every untagged
  four-on-the-floor song (128 → 85.3), dubstep was told to prefer 70, and
  a prior penalised anything under 95. All of it is removed. A song is now
  read in its genre family's octave (`OctaveProfile::tempo_octave`: reggae
  and dub 60–120, hip-hop 68–136, dancehall 75–150, house / garage / grime
  / dubstep 88–176, drum & bass 100–200), else in the DJ's **tempo range**
  (Preferences ▸ Library ▸ Tempo, 78–156 to start; `library_settings`, no
  schema change). **96.3 %** on the same songs; the 94-song fixture set
  reads 4 known misses, its expectations corrected from Traktor (eight
  were the old detector's own four-thirds). `tests/traktor_grid_corpus.rs`
  is the benchmark, path-gated on a manifest written from the library.
  Songs already analysed keep their grid until re-analysed. Account:
  `BPM-DETECTOR-V2-INVESTIGATION.md` §8; contract: PRD-BEATS §6.0.
  Thru decks don't follow the tempo range yet.
- **Grids on the off-beat are moved onto the beat (2026-11-19, Daniel:
  "the grid sits on the actual beat and not half a beat off").**
  `move_off_the_off_beat`: when the song's first sound falls half a beat
  from the lines and the off-beat is the louder, the grid moves half a
  beat. Against the locked Traktor grids: 105 → 58 half a beat out, lines
  within 25 ms 84.7 % → 87.0 %. Whole-song cues (kick band, several bands
  at once, kick edges, kick/snare alternation) were measured first and
  separate nothing. About 30 of the 58 left are songs where Traktor's own
  lines sit off the low end.
- **A tap on a playing deck picks the beat; the song places the lines
  (FFI 91, 2026-11-19, Daniel).** `setBarPhase(playing:)`. Playing: the
  tap says which beat is the 1 and which run of hits is the beat, and
  `fit_grid_at_playing_tap` fits the whole song — the same lines however
  early, late or far along the tap fell, and the way to put an off-beat
  grid right. Stopped: unchanged, the exact kick under the playhead. It
  used to hang the grid on the loudest hit within 60 ms of a tap made by
  ear. **To check on the rig:** tap the 1 on a playing deck, a few times,
  on a locked-quality house track and on a reggae one. **Corrected the
  same day** (Daniel, on *Hey Ya*: "it always jumps to the next half
  beat"): it took the strongest run of onsets in reach and landed 215 ms
  from the tap, on the strums. It now takes the hit the tap is *by*, and
  a hit is where the waveform's level jumps, not what the onset function
  counts; his two logged taps replay to 20 and 26 ms from the tap
  (`tests/hey_ya_playing_tap.rs`).
- **The drift warning starts at 8 ms a minute (2026-11-19, Daniel on
  *Hey Ya*: "it says may drift over a long mix but i find the grid
  perfect").** It fired at 3, on a reading too scattered for that: 28 %
  of the grids he locked in Traktor would have carried it. The drift is
  now read as a median over pairs of hits and marked from 8
  (`GridQuality::drifts`; PRD-BEATS §5.3): 7.0 % of locked grids, 92.3 %
  of a 12 ms/min drift. Stored readings are the old kind until a song is
  re-analysed or its 1 is set again; *Hey Ya*'s stored 5.6 is under the
  new limit either way. **Not run:** the Swift snapshot suite — the
  fixtures that draw the mark moved from 5 to 12 so the pictures should
  not change.
- **Tap tempo: the taps are a hint (FFI 91, 2026-11-19, Daniel: "the dj
  always taps wrong, he's never as accurate as the transients").** 3+ taps
  on a playing deck now tell Dub roughly how fast and roughly where; the
  track's own tempo is found within ±12 % of the tapped one and the whole
  track places the lines (`analyze_beat_grid_from_taps`;
  `installBeatGridFromTaps`). His logged 118–121 on a 129 song committed
  121; it now gives 129. Simulated on the 1,767 locked songs (tempo 8 %
  out, tap 35 ms late): 94.3 % come back within 0.05 BPM of Traktor,
  85.9 % with their lines within 25 ms. The tempo is read from the taps'
  playhead positions, so the pitch no longer comes into it — the
  wall-clock rate it used was the sounding tempo, right only at zero
  pitch. A track with no tempo of its own keeps the tapped one.
  **To check on the rig:** tap a track Dub read wrong, roughly, with the
  pitch well off zero.

- **Window dots in full screen (2026-11-18, Daniel).** With the menu bar
  kept away, macOS's window buttons go too, so `WindowDots` puts three at
  the left of the status strip, Serato-style, in full screen only
  (`FullScreenState`, set by the window delegate): left → the quit page,
  middle (minimise) disabled, right → window mode. Dub greys at rest,
  macOS red / green with glyphs on hover; the disabled one stays grey.

- **⌘Q asks first (2026-11-18, Daniel).** `QuitGuard`: ⌘Q, the Dock's Quit
  and the window's close button cover the surface with a full-window
  "Quit Dub?" page (after Serato's); only a click on Quit quits (no key,
  Daniel: a stray ⏎ must not end a set), Esc goes back to the decks, and every other key is swallowed so nothing fires behind it.
  Logout / restart / shutdown (a quit event with `kAEQuitReason`) and the
  XCTest host are never held up.

- **Full screen keeps the menu bar away (2026-11-18, Daniel).** Like
  Serato's: `window(_:willUseFullScreenPresentationOptions:)` returns
  `.hideMenuBar` + `.hideDock` instead of the auto-hide pair, so the
  pointer at the screen edge reveals nothing. Preferences ▸ Decks ▸
  Screen, on by default (`dub.fullScreenHidesMenuBar`); toggling it while
  full screen applies at once via `NSApp.presentationOptions`.

- **Serato 4 / 5's library (2026-11-18, Daniel).** New `serato_sqlite.rs`
  reads a copy of `~/Library/Application Support/Serato/Library/master.sqlite`
  (never Serato's own file): tracks of the `Serato Library` space through the
  Serato 3 per-track path (so grids, cues and loops still come from GEOB),
  and the crate tree in Serato's order, smart crates skipped. `import_serato`
  takes the file or a folder holding it; discovery picks whichever of
  `database V2` / `master.sqlite` Serato wrote last. No FFI or schema change.

- **Apple Music, choosable and reloadable (2026-11-18, Daniel).** Preferences ▸
  Library ▸ Apple Music has **Choose…** / **Default** (`dub.itunesLibraryPath`;
  nil = the default discovery), and the sidebar's Import ▸ iTunes
  Library.xml… picks the followed library too while the source is on.
  Read at launch and on the Apple Music node's right-click **Reload Apple
  Music Library** only; a background file watch was built and taken out
  again (Daniel: less pressure on the app). The import now retires
  `itunes` metadata rows for tracks the XML no longer lists
  (`imported_at = 0`; the per-source list and count skip them), so the
  node shows the current library, not every library ever read. A track
  on an offline drive is never retired; no schema bump, no FFI change.
- **Libraries from another Mac (2026-11-18, Daniel).** An XML from an old
  account names `/Users/dj/…`; Daniel's 2,186 tracks were 0 found as
  written. `dub-library/src/relocate.rs`: a missing `/Users/<name>/rest`
  is retried as `$HOME/rest` and taken only if that file exists. Every
  absolute-path importer (iTunes, Traktor, rekordbox, Serato 4) and
  Serato 3's volume-relative `pfil` go through it. His real XML: 2,184
  found, 2 genuinely absent.
- **Apple Music tracks: genre, BPM, comment (2026-11-18, Daniel).** SOSA UK
  "The Wave" (Apple Music: House, 132) analysed at 88: the octave profile
  read the genre from the `id3` row only, and a track read from Apple
  Music has none, so it fell to Default. `track_id3_genre` now takes the
  file's tag first, else any library's (`dub diagnose --profile house`:
  132 at 0.84 vs 88 at 0.73). The BPM column shows a DJ library's BPM
  (`sr`/`rb`/`tr`/`it`, never `i3`) until there is an active grid, and
  the iTunes reader now keeps `Comments` and `Track Number`. Tracks
  already analysed wrongly need Re-analyze; nothing reruns on its own.
  The deck header's key also fills in when the first analysis lands on a
  track already loaded (`showKeyOnLoadedDecks`), not only on the next load.
- **Traktor's grid and cues from inside the file (schema v12, 2026-11-18,
  Daniel).** Traktor writes a `TRAKTOR4` ID3 frame into every file it
  plays; 2,167 of Daniel's 2,171 MP3s carry one while the collection
  itself stayed on another Mac. `dub-library/src/traktor_tag.rs` reads it
  (format in LIBRARY-FORMATS.md) on Apple Music and folder imports, once
  per file version (`track_files.tags_read_mtime`): 2,156 grids and 287
  cues from his XML, ~20 s the first time, ~4 s a reload. Found on the
  way: Traktor's MP3 timeline is one frame (26 ms) later than Dub's on
  files with an encoder header frame, and the NML import had the same
  offset; `mp3_frames.rs` now corrects both (median vs his taps +27.4 ms
  → +1.4 ms). No FFI change. Open: a fuzz target for the frame parser.
  Serato's GEOBs ride the same pass (1,642 of his files): Serato's MP3
  timeline is the header frame *minus* the 529-sample decoder delay, which
  the Serato library import had never corrected either; corrected grids
  agree with Traktor's to −3.4 ms median over 1,456 files (was +11.4 /
  −16.9). A Serato library already imported moves on its next import.
  Grid locks come along (Traktor `LOCK` / `SYNC/LOCK`, Serato `BPMLOCK`;
  PRD-BEATS §4.4): Dub locks when that grid arrives or changes and is the
  active one, and a lock over an imported grid no longer blocks the first
  analysis. Serato is written before Traktor so Traktor's lock can't pin
  Serato's grid. His library: 390 tracks arrive locked (Traktor locked
  ~1,775, but Serato's unlocked grid outranks it on most).
- **Which app comes first (FFI 90, schema v12; 2026-11-18, Daniel).**
  Preferences ▸ Library orders Serato / rekordbox / Traktor
  (`Library::import_order`, kept in `library_settings`); the order ranks
  grids, keys and pad cues and the rekordbox export's cue set (was the
  fixed `CUE_SOURCE_ORDER`, now gone). Setting it re-picks every track
  whose active grid / key is an app's, skipping locked grids; a grid its
  app locked (`track_beatgrids.source_locked`) brings the lock when it
  takes over. user_tap / user keys / mixedinkey still outrank every app.
- **The ⚠ compares the grid in use with Dub's analysis (2026-11-18,
  Daniel: "a lot of imported and analysed grid disagree").** It compared
  every pair of grids, so with Serato and Traktor both read from the
  files 509 tracks lit up where the two apps disagreed with each other —
  a fraction of a beat on old hip-hop / breaks, either app the wrong one
  (16 sampled against `dub diagnose`). Now: active app grid vs `auto`
  only, never on a set-the-1 grid (PRD §8.3's wording). Live library:
  9 of 14 analysed tracks flagged, all genuine (Dub's pre-genre-fix 88s
  and two ¼-beat splits).
- **An app switched off counts for nothing (FFI 90; 2026-11-18, Daniel:
  "Serato disabled, the node still shows").** Preferences ▸ Library's
  switches now reach the library (`set_disabled_sources`, kept in
  `library_settings`): the node hides, the app's grids / keys / pad cues
  never take the slot (`SourcePrefs::grid_rank` → `None`), every track is
  re-picked (a locked one too when its active grid was that app's; the
  lock then follows the new grid's `source_locked`), and its metadata
  drops out of the row joins. Data stays stored: on again is instant.

- **The Matching smart crate (2026-11-17, Daniel).** Lists what mixes
  with the playing track (master deck, else any playing deck; kept when
  the decks stop): Camelot distance ≤ N steps (relative major/minor = 1)
  and BPM within ±X, nearest key then nearest tempo first. `CamelotKey`
  reads Camelot, musical and Open Key spellings. Window in Preferences ▸
  Library ▸ Matching (`MatchWindow`, defaults 1 step / ±3 BPM; Daniel
  asked for both to be prefs), plus two switches, both off by default:
  half / double time, and match the pitched tempo (track BPM × the
  deck's pitch, held until it moves 0.5 BPM so platter wobble doesn't
  rescan the library every poll; a lift keeps the last value). Scans the whole library off the main
  thread, only when the reference or window changes.
- **The loop row is mappable.** Each ladder length (`loop.size.<deck>.<n>`,
  pressing the running length exits, as the button does) and ÷2 / ×2
  (`loop.halve/double.<deck>`), per deck, no default keys.

**In alpha.5** (alpha.4 never went out; not on the rig yet):

- **Serato's ⌘ keys (2026-11-17, Daniel, from Serato's shortcut page).**
  Serato's "control" is ⌘ on a Mac, so instant doubles are `⌘⇧←`/`⌘⇧→`
  (were `⌃⇧`, which also fought macOS's Spaces), and `⌘⇧` + a cue key
  clears it as `⇧` does. Folders moved to `⌘←`/`⌘→`; plain `←`/`→` stay
  free (Daniel maps echo out there). The left-hand / right-hand letter
  keys stay ("I prefer the left right mode"); no default for key lock or
  INT play.

**Also in alpha.5, since alpha.3** (not on the rig yet):

- **A default keymap (2026-11-17, Daniel).** Replaces "no default keymap"
  (2026-09-15). Left hand deck A, right hand deck B: cues `1–5`/`6–0`
  (new per-deck `deckCue`; the master-deck `hotCue` stays for maps made
  before), tap `Q`/`P`, loop off `W`/`O`, echo `T`/`Y`, Quick Scratch
  `ASDF`/`HJKL`, samples `ZXCVBNM,`, load `⇧←`/`⇧→`, double onto a deck
  `⌃⇧←`/`⌃⇧→`, tracks `↑`/`↓`, `⌘F` search, sidebar `⌘⇧↑↓` and folders
  `⌘⇧←→` (new `sidebar` / `folder` actions). Space loads into the
  *empty* deck. A key now carries ⇧ ⌃ ⌥ as well as ⌘; an unclaimed ⇧/⌥
  falls back to the plain key as an argument. Tap the grid merged into
  Tap the BPM (Daniel): ⇧Q / ⌥Q halve / double deck A's track tempo, the
  BPM menu's ½ / 2×; the `grid.tap.*` ids parse to nothing. A key the DJ bound beats a
  default; `⌘,` can't be taken. To check on the rig: ⌃⇧ arrows vs
  macOS's Spaces shortcuts.

- **Welcome guide reworked (2026-11-17, Daniel).** Three stale pages
  (it said audio needs no setup; it offered only a folder import) became
  a setup checklist over the live rig: your rig (interface → Performance,
  none → Prep; one line of principles) · audio (the microphone permission
  before macOS asks; routing ready for a known box, `AudioRoutingSection`
  embedded for an unknown one) · signal check (a light per deck, green
  once timecode arrives — `DeckSignalCheck`) · music (Traktor / Serato /
  rekordbox / Apple Music found on this Mac, read-only; or a folder) ·
  keys (MAP and Preferences ▸ Keys; *bound now* read from the keymap, so
  default keys added later show up by themselves) · ready (INT · TC ·
  THRU, calibrate, the manual). Audio and signal appear in Performance
  only. A new interface does not reopen it (Daniel).

- **Keys for actions with no button (2026-11-17, Daniel).** Preferences ▸
  Keys maps what map mode cannot click (`DubKeymap.buttonless`): load
  into A / B / the deck not playing, instant doubles, Tap the grid **per
  deck** (was the master's — "per deck", Daniel; refused on a locked
  grid; merged into Tap the BPM later that day), loop off per deck, jump to search, next / previous track. Click
  the cap, press the key — the same store and capture as MAP. A browse
  key that types nothing (arrow, F-key) still fires from the search
  field and leaves it, so search → ↓ → Space needs no mouse. The tab now
  explains ⇧ / ⌥ as modifiers and ⌘ as part of the key. Closes the
  "unreachable actions" item. The old `grid.tap` id parses to nothing;
  nothing could bind it.

- **Tap the BPM, on every deck (2026-11-16, Daniel).** The Performance
  column's track BPM is the Prep header's tap pad now — 1–2 taps set the
  1, 3+ re-take the tempo, a locked grid takes none and wears a padlock,
  and its right-click has 2× / ½ / Reset to auto / Lock grid. On THRU the
  first tap of a run is the record's 1: the renderer snaps it to the
  nearest followed beat (`PredictedBeats.snap`), rolls it forward look by
  look (`roll`, so the follower's drift never adds up) and draws a solid
  bar line every fourth beat. A THRU tempo takes three taps now, not
  two — two would turn a double-tap on the 1 into a tempo; the Space
  Echo's TAP still takes two. One mappable action for both,
  `.tapTempo(side)` ("Tap the BPM A"), id unchanged so old maps keep
  working.
- **"↝ usually:" is back where it was designed to be (2026-11-16,
  Daniel).** The M11d-history hint lived in the Prep header's
  remaining-only time row — a layout Prep never uses — so after the
  Performance column replaced the header band it showed nowhere. It now
  sits in the deck column's time row, between elapsed and remaining
  (title · artist, truncating first; click reveals it in the library),
  and is gone from Prep. **Preferences ▸ Decks ▸ Show what you usually
  play next**, on by default (`dub.historyHintEnabled`).
- **The loop row stays put under a press (2026-11-16, Daniel: "the
  clicked loop button moves to the right").** It re-centred on the
  engaged length, so pressing 1 slid the row to 1/4 · 1/2 · 1 · 2. Now a
  visible length engages where it is; only ÷2 / ×2 past an end moves the
  window, by one, and it stays there (`LoopEngine.window`).
- **The library sidebar folds (2026-11-16, Daniel).** Every section
  heading folds its section; imported sources fold as before; folders
  inside a source (Apple Music playlist folders, Traktor folders) fold
  too and **start folded**; an empty folder's row folds on a click.
  All of it persists (`SidebarFolding`, `librarySidebarFolding`), folders
  keyed by source + name path so a rescan keeps them.
- **One ⚠ in the BPM column, never two (2026-11-16, Daniel).** A grid
  disagreement and a drift warning drew two near-identical triangles side
  by side. Now one mark, amber for a disagreement and orange for drift
  alone, its tooltip listing each (`LibraryCellFormat.bpmIssues`); the
  padlock stays its own glyph.
- **Imported hot cues on the pads (FFI 88, schema v10; 2026-11-16,
  Daniel).** Serato / rekordbox / Traktor cues were imported but the pads
  showed user cues only. Now the pads show the DJ's own cues, else one
  import's set in the beat-grid order (Serato > rekordbox > Traktor —
  `Library::pad_cues`), and the HOTCUE heading says **FROM TRAKTOR**.
  The first set / clear / label on such a track copies that set to
  `user` rows and records it in `track_cue_takeover`, so edits land on
  what the DJ sees and a re-import never overrides them. The rekordbox
  export now picks its cue source by the same order (it took the source
  with the most cues). **Preferences ▸ Library** explains the order.
  To check on the rig: Traktor cue positions against Dub's decode on
  MP3s (encoder-delay handling can differ between apps).
- **The following grid is saved (FFI 89, schema v11; 2026-11-16,
  Daniel).** It was recomputed on every load and never kept, so the
  library could not show it and a lock could not keep it. Now the deck
  saves the band-followed beats to `track_tempo_maps` with the even grid
  they came from; a load reuses them while that grid stands and always on
  a locked track. A moving grid's BPM is `DubColor.gridMoving` (cyan) in
  the library and on the deck, and its padlock too once locked — moving,
  accepted. A grid edit on a following deck follows the band again
  instead of flattening it (`DubEngine::refollow`), and tap / nudge saves
  write the even grid's anchor and phase (`BeatGrid.even_*`), not the
  band's first beat. `kind` leaves room for time-stretched tracks, which
  will get their own colour.

## Waiting on the rig

**Confirmed by Daniel on the rig, 2026-11-15:** the SL3 plug-in freeze is
gone; the following beat grid works; the output meters look right; THRU
appears to work.

Built and tested, not yet tried on hardware:

- **Audio routing:** the SL 3 shows read-only with TEST on both decks;
  an unknown interface (borrow one) routes from the tab and starts two
  decks on the pairs picked.
- **The stage rescue:** INT, swap or fix the needle, ↻ under INT, wait
  for the green lamp, TC — the beat must not move.
- **The signal panel** (FFI 85), and whether a spent ECHO OUT reads
  **ECHO OUT** on the meter while the record plays on.
- **The siren's DSP fixes**, and two calls left for Daniel's ear: HK628
  release 10 → 25 ms, some harmonics on the pure-sine Sine.
- **The model split** (`ModelStores.swift`) and the **rip-review fixes**
  of 2026-09-24/25 (load refusal, the footer, the sleeve tracklist).

## Owed before v1: validate on the rig

`LESSONS.md` is explicit that timecode and vinyl behaviour are confirmed on
hardware.

0. ~~Timecode decode against real vinyl~~ — **done**: `testdata/timecode/`
   holds three SL 3 excerpts driving `real_vinyl_tests.rs`. Still uncovered
   there: a stalled platter, a mid-play lift, pitch extremes, and *both*
   Traktor formats. See that README.
1. **Loops under a real needle.** Loops, scratching and pitching behave on
   the rig (2026-09-22). Not tried: lifting the needle mid-loop, re-locking
   after a lift, key lock at a pitched platter.
2. **A rip end to end through the app.** Every gate is fitted against the
   three captures in `testdata/rip-baselines/` and replayed offline with
   `dub rip-tune`. The trim brackets, the Real Records node and re-split
   have never been driven on the rig with a record on the platter.

## Open

- **No waveform while recording a rip.** `hasSource` includes
  `ripCapturing` and `deckAEnabled` is just `isRunning`, so the strip should
  mount. Either the Thru-for-rip peaks tap is not feeding deck 0's stream or
  the mount is starved; retest starvation first — the `RipEnvelopeTiler` fix
  removed a large main-thread load that was running the whole time.
- **Space Echo HOLD / SYNC / TAP on the rig:** the aux send is patched into
  5/6 (deck B's pair) but nothing arrives there. Looks like the mixer side.
- **dnb on THRU reads 116.6 (⅔ of ~175)**, live and offline — the next
  tempo-detector job.
- **The control record's end zone (P-32):** INT when the needle runs into
  it. The end zone is a locked groove, so the absolute position repeats —
  jumping back one revolution (~1.8 s at 33⅓) each turn while the platter
  runs at speed — which a lift never does. Needs a capture of the SL 3
  running into the end zone first.
- **Serato 4/5 library: unverified edges.** Read since 2026-11-18
  (`serato_sqlite.rs`, see the entry above and `LIBRARY-FORMATS.md`), checked
  against Serato DJ Pro 4.0.7 on the dev Mac (3 tracks, grids + keys from
  GEOB, 1 crate). Still to see: a Serato 5 library, a library on an
  external drive (`location.path` non-empty), and Lexicon's Serato 5 grid
  tag. A Serato 4/5 smart crate is skipped, not evaluated.
- **Following grid: a half-beat jump across a breakdown (parked,
  2026-11-21).** The tracker picks which half of the beat is the kick once
  for the whole track, so a record that comes out of a break half a beat
  shifted (*Bam Bam*, Dj Turk Disco Remix) has its lines right on one side
  only. The fix is to choose again after a beatless stretch, and only
  there; it belongs with the tracker handling beatless stretches, and has
  to be checked against the Traktor-locked corpus because a second choice
  is a second chance to land on a reggae offbeat. STEADY is not affected.
- **MIDI.** Map mode's keyboard lane shipped; the MIDI lane is the same
  mechanism pointed at `dub-controller` once that crate is real — the
  missing half of Expert FX.
- **DUB FX (parked behind Alpha):** building the rack pane costs ~120 ms;
  if the switch still bites, keep the built rack alive when switching back.
  Next ideas there: a low-pass side on the Big Knob.

## Next milestone

**M18 — Polish + Alpha.** M17 was the last feature milestone; what remains
before trusted-DJ hands is polish, carrying real deferred work:

- **The deferred M16 fine-tuning**: siren sound polish (F-36). The siren
  Expert panel (F-37) is delayed until it has a full design.
- Calibration UX, preferences, dark-mode polish, and the manual rig
  checklist.

Post-1.0: **M27 — UI themes** (PRD §12.2). Until then, keep every colour
in `DubColor`.

## Known gaps worth naming

- **P-40** — the FFI position extrapolator does not wrap into the loop region,
  so the UI playhead reads about a block past `loop_out`. Sub-frame normally.
- **R-41** — the deck pane shows its idle placeholder during a rip capture;
  only the overview band renders live signal.
- **R-43** — ripping is Prep-only by design; Performance-mode ripping is
  deliberately deferred.
- **Same-artist pressing tie-break** (M26c, a limitation). A foreign-language
  pressing credited to the same artist ties with the domestic one; only bites
  "identify the pressing", naming is unaffected. A research task.
- **The snapshot suite can crash the Intel GPU driver** (twice so far):
  swift-snapshot-testing's perceptual compare runs through CoreImage → MPS.
  A flake — a re-run passes — but it can fail the pre-push hook. If it
  becomes routine, drop `perceptualPrecision` for an exact compare plus a
  small `precision`.
- **The AcoustID key is not in the repo and must not be.** `dub recognize`
  reads `$DUB_ACOUSTID_KEY` or `--key`; without it every lookup returns
  `MissingCredential` on purpose.

## Keeping this file honest

Update it in the same change that ships a milestone or opens a gap — the same
rule as `SHIPPED.md`. If a section here is longer than a screen, it belongs in
`UI-BACKLOG.md` or the PRD, and this file should link to it instead.
