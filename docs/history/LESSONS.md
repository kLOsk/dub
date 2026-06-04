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

## FFI / UniFFI (Rust ↔ Swift)

- **Proc-macros, not UDL** — the Rust signature *is* the exposed surface, so
  there's no `.udl` to drift out of sync.
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

## Product invariants (don't relitigate without sign-off)

- **No software mixer / EQ / crossfader, ever** (v1 & v2). The hardware mixer is
  the product; Dub routes per-deck audio + FX only.
- **No device / channel picker.** Audio mode is hardware-derived (interface →
  Performance, none → Track Preparation; hot-plug switches live). The dev-only
  overrides are `#if DEBUG`. Surfaces that explain audio should *show* the
  auto-detected state, not offer a choice.
- **Whole tracks decode to RAM; forward/backward playback is byte-symmetric.**
  No per-block disk streaming. Instant rewind/backspin depends on this.
- **GPLv3 is deliberate** (anticipates the M14 Rubber Band FFI). Check every new
  dependency's license against it; prefer pure-Rust to avoid LGPL dynamic-link
  complications (why `dub-bpm` and `dub-fingerprint` are pure-Rust).

## Process

- **One regression test per pathology**, not one per feature. The lift state
  machine, the MK2 mis-routing, the octave cases, the SR-drift — each is a named
  test that would catch the specific class again.
- **Snapshot tests are PRD-mandated (§2.2.4) but still don't exist** (UI-BACKLOG
  C-31). The UI regressions each round (footer pill, locked-grid BPM colour,
  stale multi-select label) would all have been caught by a small
  `swift-snapshot-testing` suite around `LibraryView` footer, the row context
  menu, and `DeckHeader`. First UI PR that adds them earns its keep.
