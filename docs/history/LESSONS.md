# LESSONS.md — pitfalls and hard-won decisions

> Distilled from the M0→M11d.8 build (the blow-by-blow lives in git history
> and the `SHIPPED.md` milestone index). This is the "don't repeat these
> mistakes" file: every entry cost us a debugging session or a dogfood night.
> Read it before touching the subsystem it names.

---

## Audio thread (RT-safety)

- **Never drop an `Arc<Track>` or `Box<TimecodeInput>` on the audio thread.**
  Transport changes and mid-stream re-cals bounce the displaced value back
  through a *trash channel* for main-thread disposal (`pending_disposal` +
  an `AtomicU64` overflow counter; `EngineHandle::reclaim` drains it). Dropping
  on the render thread is a hidden `free()` syscall.
- **Load must never block playback.** The audio thread only needs the
  `Arc<Track>` to start — *not* peaks, *not* a beat grid. M10.5v gated playback
  on in-`load_track` beat-grid analysis; an O(N²) blowup in `SpectralFrameStream`
  turned a "~100 ms" doc-comment into multi-second stalls. Decode + peaks +
  grid all run off the load path now. PRD §6.4.
- **"Load never blocks playback" also means the DECODE.** The synchronous
  whole-file decode in `load_track` scaled with track length — measured on a
  real 70-minute 320 kbps mixtape: **11.6 s before the deck could play**
  (release build, ~363× realtime; the "~50 ms for a 4-minute MP3" doc-comment
  was off by ~13×; *measure, don't trust comments*). The fix is decode-ahead:
  decode ~4 s of head synchronously, pre-allocate the full buffer from the
  container's declared frame count, stream the tail on a `dub-decode-{idx}`
  thread behind an atomic watermark (`dub-io::SampleStore`). `Track::frame`
  clamps to the watermark, so a playhead past the frontier renders silence and
  self-heals — the engine transport needed **zero** changes because
  out-of-range-reads-silence was already `frame()`'s contract. Same mixtape
  now playable in 1.1 s (buffer-allocation-bound); 4-minute tune in ~70 ms.
  Two invariants to keep: streaming vs one-shot decode is pinned
  **byte-identical** by test, and `#![forbid(unsafe_code)]` survives because
  the streaming buffer stores f32 *bit patterns in `AtomicU32`s* (Relaxed
  element loads are plain moves on every target we ship; the Release/Acquire
  watermark orders them) — don't "optimise" it into `UnsafeCell` + raw slices.
  Consequence for offline analysis: a streaming track can't borrow `&[f32]`
  (`samples()` returns empty); tap-to-grid copies the decoded prefix
  (`samples_for_offline_analysis`), the uncached worker paths park until the
  tail lands (`FullSamples`). Container without a declared frame count / rate
  / channels falls back to the old one-shot decode transparently.
- **Never hold the library mutex across an O(track-length) pass.**
  `dub-ffi::DubLibrary` serializes everything behind one `Mutex`;
  `analyze_track` used to hold it for the whole decode + fingerprint + BPM +
  key + peaks + loudness pass (seconds to tens of seconds, at `.background`
  QoS = E-cores, stretching it further). Every deck-load lookup on the main
  actor (`active_beat_grid`, `track_normalization_gain`, `track_path`, hot
  cues, history) queued behind it — loads froze for the remaining analysis
  time of the *previous* track, which read as "loading takes forever and
  scales with track length". Fix: three-phase split (`analyze_prepare` locked
  read → `analyze_compute` **lock-free** → `analyze_commit` locked write),
  commit re-checks the grid lock (lock-is-absolute survives the unlocked
  window), analysis runs at `.utility`. Pattern to keep: any Library call
  that decodes or scans audio must take the lock only around its reads and
  writes, never around the DSP.
- **Prove RT-equivalence with byte-identical regression tests.** When you
  refactor the render path (e.g. `render` → `render_routed`), pin it with a test
  that asserts the old callers are byte-for-byte unchanged, and run the whole
  thing under `assert_no_alloc` + `make rt-audit`. A `routing[i] == None` deck
  must *not* advance its transport — mute via `set_gain(0.0)` instead.

## Timecode / control vinyl (validate on real hardware)

- **Lift detection took three SL3 iterations.** A single-threshold gate
  chatters on lift; confidence-only hysteresis reads a lift as a "lukewarm
  scratch transient" and burst-plays the track with the needle up. The working
  design is three layers: an **amplitude gate** (RMS kill when the carrier is
  dead), **two-edge confidence hysteresis** (engage 0.8 / disengage 0.5), and a
  **sticky-block window** for dust-tick immunity. Factor the policy into a pure
  `step_policy(DecodeOutput) → Intent` so each pathology gets its own test.
- **Carrier frequencies are silent-failure landmines.** An early M6 draft had
  Traktor MK2 at 2 kHz instead of 2500 Hz — vinyl would have played back at 80 %
  speed with no error. Validate every carrier on the actual pressing and pin a
  `..._plays_back_too_fast_by_25_percent`-style regression test. Same caution
  for channel polarity (the xwax "ch0 leads ch1 by 90° forward" convention).
- **Calibrate on every start.** A simpler, predictable mental model ("the app
  calibrates each launch, period") beats reusing a stale calibration that
  silently misbehaves — worth ~1.8 s on cold start.
- **In Timecode mode, never auto-play on load.** Calling `play()` there engages
  *user-initiated Panic-Play* (internal playback that ignores the timecode until
  paused) — the "load auto-starts internal play and the record does nothing"
  bug. Auto-play (drag-to-play / Space-load) is **Prep-mode only**.
- **Tune the decode chain in *time*, never in *blocks* — and decode at the
  design cadence regardless of the render quantum.** The Hampel window, smoother
  poles, and sticky window were tuned at 64-frame blocks; fed whole 512-frame
  CoreAudio quanta they ran 8× off the design point, and the errors were
  *directional*: the Hampel rejected genuine hand decelerations as spikes
  (replaying the stale fast push rate at every scratch turnaround) and slow
  draws under-converged — net **forward sticker drift, ~30 ms per back-and-forth
  scratch** (half a bar in 30 strokes, on stage). `TimecodeInput::drive` now
  slices every quantum into 64-frame sub-blocks, and the smoother/policy clocks
  take real `dt`. The whole pathology is pinned by an offline physics harness
  (`dub-engine/tests/scratch_drift.rs`: velocity-scaled carrier amplitude,
  zero-net-displacement stroke cycles, drift bounds per profile) — extend that
  harness first when touching the policy, smoother, or decoder gates.
- **Lag-1 phase-difference shrinks under *correlated* noise — measure the
  phase advance over a long lag instead.** The coherent `Σ s·conj(s_prev)`
  estimator is unbiased under white noise, but vinyl surface noise has lag-1
  autocorrelation ~0.8, and shrink/incoherence ≈ ρ/(1−ρ): coherence 0.999 still
  meant **−0.31 % pitch at a true 0** on a real rig (zero-crossing-counted the
  capture to prove the carrier sat at nominal — always check whether the bias
  is yours before "calibrating it away"; a measured zero-reference would have
  broken Traktor's calibrate-at-any-pitch contract). The long-lag estimator
  (16 samples for CV02, format-scaled for ambiguity) collapses ρ^L and reads
  +0.0x %; it engages only inside |rate| < 1.2 and only when the *smoothed*
  residual-ellipse roundness is high (the lag-1 ellipse-bias law and its
  `atan(tan/R)` correction don't transfer to long lags — and gating on the
  instantaneous roundness made the two paths' bias difference toggle per block,
  which read as display jitter).
- **A cartridge is a velocity sensor — gate carrier presence on
  filter-compensated amplitude.** A slow draw is doubly quiet (low velocity ×
  input high-pass attenuation at its low carrier frequency); gating on the raw
  filtered RMS paused the deck through every slow backward draw (+700 ms/cycle
  of forward drift in the harness's slow-draw profile). The decoder divides the
  measured amplitude by |H(carrier_est)| — capped, and only when |rate| is
  meaningfully nonzero so silence and stopped platters never get boosted into
  "carrier present".
- **Never judge a self-calibration's adoption gate on the scale that adoption
  itself warps.** The ±8 stop anchors originally checked "is the rate near a
  canonical stop?" on the anchor-corrected scale; each slightly-off adoption
  (e.g. a steady beatmatch hold at +7 % inside a generous ±1.5 % band) shifted
  the window for the next one — an unbounded ratchet that walked a healthy
  deck's stops to +12/−14 displayed, and once the true stop fell outside the
  shifted window the session could never self-correct. The fixed point: judge
  the gate on a reference the adoption cannot move (here the zero-corrected
  scale, whose own anchor carries an independent ±0.4 % guard), keep the band
  tight, and reject dwells whose tracker slid (a ride) or whose head hadn't
  settled. Found because the replay harness on the deck-B capture did *not*
  reproduce the field report — the discrepancy between capture and rig is what
  pointed at usage-dependent state (anchor learning), not decode.
- **The displayed rate MUST equal the played rate.** Two tracks pitched to the
  same BPM ran at visibly different speeds because the deck-header BPM went
  through the ±8-canonical anchor *warp* (`AnchorMap::apply`, a piecewise-linear
  display map) while the audio used the zero-anchor only (`apply_playback`).
  Same input, two outputs — the header lied. Fix: the header reads the audible
  rate (xwax/Mixxx parity), and the entire ±8 anchor-warp subsystem was deleted
  (~176 LOC) — it was a display-only transform with no audio counterpart, so it
  could only ever diverge. A "stabilised" readout that isn't the thing you hear
  is a bug, not a feature.

## BPM / beat grid

- **The octave ceiling is structural, not a tuning bug.** Double/half-tempo
  errors come from genuinely overlapping decision classes; no global threshold
  separates them. We fixed what we could with a windowed-energy picker (the
  `2P > P` overshoot is a `Σ 1/(N−kP)` artifact — no fixed tie-tolerance is
  robust across 2–60 s ODF lengths) and genre-aware `OctaveProfile`s. The
  Classic detector sits at ~11/94 on the real-music corpus and **further gains
  need a learned beat tracker (RNN/TCN + DBN), not more heuristics** — see
  `BPM-DETECTOR-V2-INVESTIGATION.md`. Do not reopen this as heuristic tuning;
  the per-track escape hatch is tap-to-grid + `BpmRange`.
- **Honesty contract.** The estimator returns `confidence = 0` for silence or a
  single click and `Err(TooShort)` for sub-2-beat input — callers must be able
  to tell "no detection" from "unanalyzable." Never fabricate a tempo.
- **Watch the CD rate.** 128 BPM @ 44.1 kHz reported half-tempo until the
  fractional-step autocorrelation search landed; beat periods are almost never
  integer multiples of the ODF interval. 44.1 kHz tracks on a 48 kHz engine are
  the fixture that catches sample-rate-dependent bugs (same-SR fixtures hide
  them — see waveform drift below).
- **The library's active grid is the single source of truth.** On load the
  engine *adopts* `track_beatgrids(is_active=1)` via the `LibraryBeatGrid`
  param instead of re-running `analyze_beat_grid`. This killed ±0.02 BPM
  cross-deck drift and a ~100–400 ms per-load analysis cost. Two readers
  computing "the same" grid independently will drift.
- **Snap the grid to the feature the user SEES, not the spectral-flux ODF.**
  DJs set grids by eye against the rendered waveform: the kick is the burst
  whose amplitude shoots up, and the "1" belongs on that rising edge. So the
  grid snap (`kick_leading_edge_secs` / `broadband_amp_envelope`) targets the
  **broadband-amplitude leading edge** — exactly what's drawn — not the
  onset-detector peak (mid-attack, jitters ±17–44 ms between sub-peaks) nor the
  amplitude *peak* (the old forward-only shift sat tens of ms late on slow
  sub-bass kicks). Validated: a crisp kick locks within ~2 ms of the hand-set
  grid, invariant to where in the bar you tap. Soft/ramping kicks with no clean
  edge fall back to verbatim (the user's eye wins). Used by all three paths:
  auto (`shift_grid_to_kick_edge`), set-the-1, and 3+ tap.
- **"Set the 1" RE-ANCHORS the whole grid, it does not rotate `bar_phase`.**
  A 1–2 tap on the deck-header BPM re-phases every beat onto the tapped kick
  (`relatch_grid_at_downbeat_tap`), keeping BPM bit-exact. Pure rotation
  (`bar_phase_from_tap`, "nearest analysed beat") can only pick an existing beat
  up to ½ a beat away, so it can NEVER correct a sub-beat-off auto grid — and it
  silently regressed once when `set_bar_phase` was reverted to rotate-only while
  a test only exercised the dub-bpm fn, not the FFI. Stopped/prep taps land
  verbatim; playing taps already read the audible timeline — the playhead is paired with CoreAudio's output `mHostTime` (M11d.6), so there is no separate upstream latency correction (the measured-latency query was removed).
- **The "1" is the first measurable beat (dance music).** For ~95 % of dance
  music the downbeat is simply the first audible beat at the track start (every
  hand-set grid is `bar_phase 0`). `apply_downbeat_refinement` picks the first
  grid beat clearing 10 % of the track-max amplitude; the backbeat (snare/bass) refinement
  rule is only the FALLBACK for the 5 % (reggae roll-ups, vocal/talk intros).
  backbeat-as-primary moved the 1 a beat late on borderline tracks (Oppidan,
  conf 0.067). Use a PLAIN amplitude probe here, not the sharp
  `kick_leading_edge_secs` — the edge detector skips a soft opening hit and
  locks onto the first *loud* kick a beat later (wrong bar position).
- **Tap-tempo integer-snap trusts the human on poor-onset tracks.**
  `IntegerSnapPolicy::TAP` widens the tolerance to 0.25 BPM and accepts the
  nearest integer *unconditionally* when `kept_fraction < 0.6` — a sparse fit
  can't resolve sub-integer BPM and a ≤0.25 snap can't be an octave error, so
  the tapped integer wins (Apocalypse: 174.8/174.9/175.10 → 175.0 regardless of
  where you tap). `AUTO` stays strict so a clean fit at a genuine fractional
  tempo (Chase & Status 174.98) is preserved. Don't loosen AUTO.

## Waveform rendering (Metal)

- **Sample-rate mismatch is the recurring footgun.** Peak chunks are cadenced
  in **track** frames; the renderer once indexed them in **engine** frames with
  an integer-rounded `samplesPerPeakChunk` → 0.49 %/chunk drift compounding to
  ~1.17 s of visual lag over 240 s. Invisible on same-SR test fixtures. Keep the
  conversion exact and test cross-SR. (See also the M5.2 HAL sample-rate-match
  invariant in `ARCHITECTURE.md`.)
- **Render off the main thread.** `MTKView` hops every `draw(_:)` back to the
  main runloop, so any SwiftUI body re-eval or library FFI stalls the next
  vsync ("tap a folder, the waveform jumps"). The fix: a `CAMetalLayer`-backed
  `NSView` + a dedicated per-deck `CVDisplayLink` render thread, appearance
  state behind an `OSAllocatedUnfairLock` snapshot, and a lock-free playhead
  read (`position_snapshot` atomics, no `Mutex<EngineState>`). Grid + envelope
  must share one playhead source or they wobble apart;
  `WAVEFORM-JITTER-CAPTURE.md` has the `os_signpost` runbook (`make trace-grid`).
- **"Venetian blind" stripes = no MSAA + sub-pixel quads + per-chunk min/max
  jitter.** Smooth with a `[1,2,1]/4` temporal kernel in the vertex stage — but
  keep the honest `clipping`/`silence` flags reading the *raw* centre chunk, so
  smoothing never touches the depth-of-information surface.
- **Don't chase richness with a post-processing ladder.** M10.8 deleted an
  entire stack (HDR target, separable bloom, ACES tonemap, a runtime
  `WaveformTuning` knob panel, onset-brightness, kick-tint, …) that was paying
  for problems it created and collapsing dense music into yellow soup. A single
  Serato-parity pass with calibrated band biases won. If the operator needs a
  knob to make the waveform readable, the default is wrong.
- **`NSHostingController.sizingOptions` defaults pin SwiftUI's intrinsic size**
  onto AppKit Auto Layout — the cause of the full-screen "black frame around a
  1440×900 island." Set `sizingOptions = []` and drop `preferredContentSize`.
- **A windowed renderer must never call an "extend-to-end" FFI.** The scrolling
  waveform ingests a bounded playhead-centred window per frame, but
  `peaks_extend(start)` returned `[start, peaks_len)` — the *whole tail*. On a
  90-minute mix that's ~3.7 M chunks (~44 MB) allocated + serialised + marshalled
  *every frame* near the start (the dedupe gate passes each frame during
  playback), shrinking to nothing near the end → "jittery at the start, smooth at
  the end." The computed `count` budget existed but was silently dropped on the
  copy, and the single-wrap ring memcpy *overran* once the tail exceeded
  `chunkCapacity` (~25 min). Fix: thread `max_chunks` through `peaks_extend` /
  `band_peaks_extend` (`0` = to-end for the one-shot overview strip; the renderer
  passes its budget). Lesson: if a symptom scales with *remaining* track length,
  suspect per-frame work proportional to "to the end," not the grid math.

## DSP / FX (dub siren, echo-out, vintage chips)

- **Macros resolve to coefficients OFF the audio thread — never in
  `apply_command`.** A "super-knob" (siren DUB macro, Big Knob HPF, spring
  damping, PT2399 filter) that maps one value to many filter coefficients tempts
  a `powf` / `sin` / `exp` in the setter — but `apply_command` runs on the audio
  thread, so a transcendental there is a hard RT violation. The pattern: bake a
  **LUT indexed by the knob** at `new()` (e.g. `macro_g: [f32; 65]`,
  `filter_lut`, `macro_damp`) and have the RT-side `set_macro(m)` do a table
  lookup + lerp. ms→samples is a plain multiply; keep it that way. Enforced by
  the `*_render_is_alloc_free` tests under `assert_no_alloc`.
- **The dub siren is an INSTRUMENT, not a rack effect — render it LAST.** It's a
  self-contained box (voice + its *own* PT2399 echo + volume) summed onto the
  bus *after* the deck echo-out and the FX rack, so it survives regardless of
  their state (echo-out mutes the deck dry to 100 % wet; a siren rendered before
  it would get swallowed). It's purely **additive** (`out[..] += s`), unlike
  `EchoOut` which rewrites in place — pinned by an "additive contract" test that
  pre-fills the buffer and asserts the original survives. This render-order flip
  was the one behavioural gotcha when wiring it.
- **Chip pitch and timing are coupled — model the clock, not the knob.** The GS1
  (`Hk628`) `set_speed` scales pitch *and* step duration together, chip-clock
  style: slower = lower + longer. When the DUB macro rises it therefore *deepens*
  the siren (pitch couples **down**), which is what fixed the "macros too
  high-pitched" dogfood note — don't decouple them back into an independent
  "rate" knob.
- **A square reads far louder than a sine at equal amplitude** (RMS, not peak).
  Level-match by ear / integrated LUFS per waveform, not by peak — the Benidub
  "Square" preset needed its volume dropped to ~0.22 to sit with the sine tones,
  and every siren/FX output is trimmed to the −14 LUFS track target
  (`hk628::OUTPUT_GAIN`).
- **Emulate the real unit's control surface, don't invent knobs.** Expert mode
  mirrors the physical box (DS01E = MODE / PITCH / RATE / TRIGGER + PT2399
  TIME / FEEDBACK / ECHO VOLUME / FILTER / ECHO CUT). Confirm the real layout
  from the manufacturer's manual before shipping the UI — the in-app controls
  are a contract with the muscle memory of someone who owns the hardware.

## FFI / UniFFI (Rust ↔ Swift)

- **Proc-macros, not UDL** — the Rust signature *is* the exposed surface, so
  there's no `.udl` to drift out of sync.
- **A new Swift file that references an FFI type needs `import DubCore`.** The
  siren build broke with "cannot find type 'SirenUnit' in scope" because
  `PerformancePadsView.swift` imported only AppKit / SwiftUI. Generated enums /
  records live in the `DubCore` module — import it in every file that names one.
- **Regenerate bindings whenever the FFI surface changes.** A new file under
  `apple/Dub/` needs `xcodegen generate` (the `project.pbxproj` only
  regenerates when `project.yml` changes). A changed `lib.rs` needs the
  xcframework rebuilt (`make app` triggers it when Rust sources are newer);
  stale generated bindings misalign record fields and decode garbage/nil.
- **The FFI is audio-thread-non-affecting by construction** — every method
  reads `PeakBuffer` atomics or runs once at attach. Swift never reaches into
  the IO proc. Keep it that way.

## Library / SwiftUI

- **Be optimistic about reachability — don't cry wolf.** `isTrackReachable`
  flags a row only when a volume probe *positively* returned `false`. The
  reachability cache is populated one runloop tick *after* rows render, so a
  pessimistic "unknown → unreachable" default flashed the red ⚠ on every healthy
  track (M11d.8 fix). Genuinely unmounted volumes still flag.
- **Glyphs key off track id, not URL comparison.** Path normalisation is
  bug-prone and resolving every visible row's URL per render is slow. The
  loaded-deck badge and history use `loadedLibraryTrackId` / a published
  `selectedLibraryTrackId` companion to `browserSelection` (avoids an FFI
  round-trip per Space-press). Clear them on *every* load so they never lie.
- **Caches that multiple views want live on the model**, not in a `View`'s
  `@State` — one published source of truth (`volumeReachability` on
  `WaveformAppModel`). Recompute per list-refresh, never per-frame.
- **Column sort is a safe-listed enum.** `TrackSortKey::sql_column()` is the
  *only* place a column name reaches SQL — user input never does. Adding a
  column = one enum variant each side.
- **Never delete metadata when a file goes missing** (PRD §8.5.5). `relocate`
  inserts a fresh `track_files` row; the old one stays on record.
  `last_checked_at` is nullable on purpose so a never-scanned row sorts
  "due first" honestly instead of being back-stamped to a lie.
- **Match on fingerprint + duration, never filename-fallback.** Filename
  matching aliases two different mixes of `Track 01.mp3`. Predicate: Chromaprint
  similarity ≥ 0.98 **and** |Δduration| < 200 ms (same axis as M11b auto-merge).
- **Two sheets can't present at once.** Re-opening onboarding from Preferences
  dismisses Preferences, then presents onboarding on the next runloop tick.

## Vinyl rip (M26)

- **The record tap must be wired at Thru-attach time.** The `AudioInput`
  consumer is take-once and re-attaching a `ThruSource` displaces the one
  that's playing — so `start_thru_for_rip` requests the tap up front and
  `create_rip_session` claims it later. Don't try to bolt a tap onto a
  running Thru source.
- **flacenc 0.5.1 writes files symphonia rejects** unless you force
  `set_block_sizes(bs, bs)` after encoding: it shrinks StreamInfo
  `min_block_size` to the short final frame, which makes decoders classify
  the stream as variable-blocksize (libFLAC keeps min == max; the spec
  exempts the last frame). Worked around in `dub-encode`; re-check on any
  flacenc upgrade.
- **The device classifier deliberately hides the built-in mic** (and every
  non-DJ input). Anything that must record outside DJ gear needs the raw
  HAL list (`DubEngine::list_raw_input_devices`) — added for the
  DEBUG-only rip dogfood fallback; production pickers stay
  classifier-filtered. A mono mic records duplicated to both slots via a
  `[0, 0]` channel map (CoreAudio allows duplicate map entries).
- **Capture spills to disk, not RAM.** A crash 24 minutes into a one-take
  rip must not lose audio (the WAV survives for salvage), and a 25-min
  side would pin ~576 MB while the engine may hold a loaded deck. The
  envelope is fed from the *recorded* samples so chunk ↔ frame alignment
  is exact — the deck's live `PeakStream` starts at Thru attach and can't
  give that.
- **Cancel must refuse once anything imported.** The encoded FLACs live
  inside the session dir and the library references them by path;
  `remove_dir_all` on discard would orphan the imported tracks. Commit is
  idempotent — retry is the recovery path.

## Product invariants (don't relitigate without sign-off)

- **No software mixer / EQ / crossfader, ever** (v1 & v2). The hardware mixer is
  the product; Dub routes per-deck audio + FX only.
- **No device / channel picker.** Audio mode is hardware-derived (interface →
  Performance, none → Track Preparation; hot-plug switches live). The dev-only
  overrides are `#if DEBUG`. Surfaces that explain audio should *show* the
  auto-detected state, not offer a choice. **One signed-off exception (M26):**
  with "Vinyl recording" enabled and a DJ interface present, a PREP / PERF
  switch overrides auto-detect so the DJ rig can digitize records in Prep
  mode (PRD §5.2.7); the override clears when the interface unplugs.
- **Whole tracks decode to RAM; forward/backward playback is byte-symmetric.**
  No per-block disk streaming. Instant rewind/backspin depends on this.
- **The dep graph is fully permissive — keep it that way.** The GPLv3 workspace
  reservation anticipated Rubber Band; M14 shipped pure-Rust WSOLA instead and
  M26 chose FLAC (flacenc, Apache-2.0) over MP3/LAME (LGPL) for the same
  reason. Check every new dependency's license; prefer pure-Rust (why
  `dub-bpm`, `dub-fingerprint`, `dub-stretch`, and `dub-encode` are pure-Rust).

## Process

- **One regression test per pathology**, not one per feature. The lift state
  machine, the MK2 mis-routing, the octave cases, the SR-drift — each is a named
  test that would catch the specific class again.
- **Snapshot tests exist now — keep new views value-struct-driven.** The
  `snap(_:width:height:named:)` harness (`StillpointSnapshotTests`,
  `PerformanceSnapshotTests`, `RipSnapshotTests`) closed the old "they
  don't exist" gap (UI-BACKLOG C-31): views that take plain state structs
  (no FFI) snapshot for free. The UI regressions each round (footer pill,
  locked-grid BPM colour, stale multi-select label) are exactly the class
  they catch.
- **Never assert on how much wall-clock work a thread got.** The publish-state
  soak (`deck.rs`) read for 300 ms and then asserted it had managed >20 samples.
  That measures the scheduler, not the code: on a loaded CI runner it starved
  below the floor and failed a rip PR that touched nothing near it. Bound
  concurrency soaks by **iteration count**; keep wall-clock deadlines for
  wait-until-condition loops, where they are an upper bound (`dub-ffi/src/rip.rs`
  does it right).
- **The Rust toolchain is pinned to an exact version, on purpose.** CI runs
  clippy with `-D warnings`, so a floating `stable` makes every Rust release a
  potential red build on untouched code — and a dev on an older stable can't
  reproduce it locally. That is exactly how M26a's PR went red: CI moved to
  1.98 while local sat on 1.95, and three new lints fired in `dub-spectral`.
  The version lives in `rust-toolchain.toml` **and** `.github/workflows/ci.yml`
  (`dtolnay/rust-toolchain@master` with an explicit `toolchain:`); bumping it is
  its own PR, where fixing the new lint set is the whole point of the diff.
