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
