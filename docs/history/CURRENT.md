# Current state

> **Read this first.** `SHIPPED.md` says what is done; this says what is *in
> flight*, what is blocked, and what to pick up next. Keep it short — a page
> that grows stops being read, and an unread status page is how
> `docs/html/` drifted ten milestones before anyone noticed.
>
> Last updated: 2026-11-17 (alpha.4). Everything shipped up to alpha.2, and
> the rig rounds before it, moved to `SHIPPED.md` on 2026-11-15.

## Where the branch is

`main` — **1.0.0-alpha.4 released** (tag `v1.0.0-alpha.4`, a GitHub
pre-release; notes in `docs/releases/v1.0.0-alpha.4.md`). `make dist` builds
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

**Since alpha.4, on `main`:**

- **Serato's ⌘ keys (2026-11-17, Daniel, from Serato's shortcut page).**
  Serato's "control" is ⌘ on a Mac, so instant doubles are `⌘⇧←`/`⌘⇧→`
  (were `⌃⇧`, which also fought macOS's Spaces), and `⌘⇧` + a cue key
  clears it as `⇧` does. Folders moved to `⌘←`/`⌘→`; plain `←`/`→` stay
  free (Daniel maps echo out there). The left-hand / right-hand letter
  keys stay ("I prefer the left right mode"); no default for key lock or
  INT play.

**In alpha.4, since alpha.3** (not on the rig yet):

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
- **Serato 4/5's library is not read.** Newer Serato keeps it in SQLite
  under `~/Library/Application Support/Serato/Library`, not in
  `_Serato_/database V2`, and Lexicon's Serato 4/5 sync writes there. The
  Serato importer reads only the old files, so that library is invisible
  (found while documenting M12-lexicon, 2026-11-16). Workaround: the
  rekordbox XML route. A reader is its own job — no public spec found yet;
  start by confirming which Serato version wrote `master.sqlite` here.
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
