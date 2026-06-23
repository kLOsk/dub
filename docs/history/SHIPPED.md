# Dub — Shipped Milestones (index)

> Companion to [`PRD.md`](../spec/PRD.md) (what's next) and [`LESSONS.md`](LESSONS.md)
> (the hard-won gotchas worth keeping). This file is now a **one-line index** of
> what has shipped, in build order. The detailed per-milestone write-ups that
> used to live here are preserved in git history — `git log` the relevant
> crate, or read the commit that landed the milestone. Durable "why is the code
> this way" rationale that's still load-bearing was lifted into `LESSONS.md`.

**Currently shipped:** engine / audio / timecode / Thru / BPM / waveform
foundations (M0 → M10.8), the Apple shell, and the full library stack
(schema / import / dedupe / browser / scanner / analysis / key detection /
beat grid / crates) through M11d.8. Build/test status lives in the latest
commit or PR, not here.

---

## Engine + audio foundation

- **M0** — Scaffold + CI + test discipline.
- **M1** — First sound (CoreAudio output).
- **M2** — Transport (lock-free command channel).
- **M2.1** — RT discipline + soak harness.
- **M3** — Format coverage (symphonia) + hot track loading into RAM.
- **M3.5** — De-click envelope + tail-fade + offline analyzer.
- **M4** — Two decks + debug mixer.

## Timecode / control vinyl

- **M5.1** — Timecode decoder, offline (clean-room Serato CV02).
- **M5.2** — Audio input plumbing (HAL input + sample-rate-match invariant).
- **M5.3** — Live timecode → deck (first scratch).
- **M5.4** — Calibration + scope (M5.4.1 fingerprint, M5.4.2 dropout handling).
- **M5.4.3** — Calibration speed (industry-parity).
- **M5.4.4** — Per-deck calibration.
- **M5.4.5** — Late-binding decks + non-blocking calibration (command-channel attach).
- **M5.4.6** — Always-fresh calibration (gut the fingerprint probe; calibrate every start).
- **M5.5.1** — Engine routing primitive (`render_routed`).
- **M5.5.2** — External-mixer 4-channel output routing.
- **M5.6** — Two-deck timecode.
- **M6** — Timecode v2 (Traktor MK1 + MK2; carrier-frequency validation).

## Thru mode + BPM

- **M7** — Thru mode (per-deck input routing; real records passthrough).
- **M7.5** — BPM engine + offline analysis (pure-Rust spectral-flux + autocorrelation).
- **M8** — Auto-BPM on Thru (streaming driver + lock hysteresis).
- **M8.1** — BPM octave fix (log-band ODF + windowed-energy picker).

## Waveform + Apple shell

- **M9** — Live waveform capture (Thru).
- **M0.5** — Apple shell + smoke screen (SwiftUI/AppKit, UniFFI, signing).
- **M9.5** — `dub-spectral` extraction + 8-band peak capture.
- **M10-A** — `dub-ffi` `DubEngine` UniFFI surface.
- **M10-B** — Metal renderer + first live broadband waveform.
- **M10.1** — Multi-colour fragment shader.
- **M10.2** — Polish: deck B, palette presets, honest silence/clipping.
- **M10.3** — Performance shell.
- **M10.4** — Vertical waveform + symmetric two-pane layout.
- **M10.5 / M10.5c** — File playback dev loop; Track Overview + horizontal shader.
- **M10.5d** — Background load (decode + peaks off-thread).
- **M10.5v** — Load-never-blocks-playback + O(N²) BPM bug fix.
- **M10.5e–g** — Waveform polish: compression, past-region dim, anti-alias, temporal smoothing.
- **M10.5h–p** — Shader exploration ladder (deliberately rolled back in M10.8).
- **M10.6a–e** — Mouse transport, Panic Play, transport-cluster redesign, Repeat auto-trigger.
- **M10.7** — Phase-Drift Trail (beatmatch aid; later redesigned → Stillpoint, PRD §9.4).
- **M10.8** — Track Preparation Mode shell + Serato-parity waveform baseline freeze.

## Library + browser + beat grid

- **M11a** — Library schema + path-by-volume-UUID.
- **M11b** — Canonical fingerprint + version-aware dedupe (Chromaprint).
- **M11c** — Filesystem importer + filename parser.
- **M11c.1** — Lazy auto-beatgrid + analysis lifecycle.
- **M11c.2** — Key detection (Camelot canonical).
- **M11c.3a** — BPM octave fix (perceptual tempo prior).
- **M11c.3b** — Tap-to-grid (manual BPM override).
- **M11c.3c** — Reggae skank double-time rejection.
- **M11c.3d** — Genre-aware octave profile (library analysis).
- **M11c.3e** — Hip-hop double-time rejection (Default profile).
- **M11c.3f** — FourOnFloor profile (house / garage).
- **M11c.4** — Lazy fingerprint (import-fast, analyze-on-demand).
- **M11d.1** — Library browser shell (functional replacement).
- **M11d.2** — Recently Played wiring + sortable columns.
- **M11d.3** — Per-row indicators (loaded-deck / duplicate / missing).
- **M11d.4** — Background missing-files scanner + Relocate panel.
- **M11d.5** — Dogfooding round: Performance-mode play, deck-B phantom playback, beatgrid overlay; library-sourced grid becomes the single source of truth.
- **M11d.6** — Full-screen on launch + windowed snap-back; waveform rendering moved off the main thread.
- **M11d.7** — Beatgrid precision, auto downbeat, tap-to-grid, drift lock (schema v4).
- **PRD-BEATS** — Beat-grid hardening: uniform Traktor-style grid, explicit `bar_phase` (schema v5), set-the-1, robustness rounds 5–10, `dub diagnose` CLI, waveform + grid jitter killed end to end. Spec: [`PRD-BEATS.md`](../spec/PRD-BEATS.md).
- **M11d-next** — Manual crates / playlists (`crates` + `crate_tracks`, drag-reorder, FFI 29).
- **M11d.8** — Polish & First-Run: search debounce + selection-preserve, dev-text sweep, edge-of-list beep, idle-hint layout, first-run onboarding sheet; dogfood fixes for the reachability false-positive and Prep-mode Space-load auto-play.
- **M11d-history** — Played From / Played Into, v1.0 stage: handover-inferred transitions (`SessionTracker`, min-play gate, duplicate suppression), full `play_history` capture with per-run `session_id`, deck-header "↝ usually" hint (click reveals the track in the browser), Session History smart crate (FFI 37).
- **Timecode-display** — Killed the pitch/BPM divergence: the deck-header rate now equals the *audible* rate (xwax/Mixxx parity); the ±8-canonical anchor-warp display subsystem deleted (~176 LOC). Phasor-based absolute decode (whitened complex carrier) lifts vinyl abs-lock uptime (97.8 → 100 % on a steady capture).
- **PRD-BEATS round 11** — Visual beat-grid: snap to the kick LEADING EDGE the waveform draws (auto `shift_grid_to_kick_edge`, set-the-1 relatch, 3+ tap) — ~2 ms of the hand-set grid, retiring the forward-only amplitude-peak shift (−516 LOC). Downbeat = "the 1 is the first measurable beat" (the backbeat snare/bass rule demoted to the reggae/vocal-intro fallback). Set-the-1 re-anchors the grid instead of rotating `bar_phase`. Tap-tempo integer-snap policy (`IntegerSnapPolicy::{AUTO,TAP}`) lands clean integers on poor-onset tracks. DnB genre lifts the hip-hop double-time veto.
- **Hot cues** — performance cues, *not* a CDJ-style cue button (PRD §6.2.1, correcting the old §6.6 "v2" deferral): four CUE pads set / recall / clear via keyboard 1–4 (or a pad controller), persisted per track (`track_cues`, `source='user'`), drawn as waveform markers; live in both Performance and Prep. FFI 38.
- **Reverse loops** — "repeat the bars just heard": grid-snapped backward loop (`dub-engine/src/looping.rs`), ½/1/2/4-bar beat-length select, internal-play / Prep only (timecode-correct looping still owed — acceptance §14 #8). PRD §6.2. FFI 39.

---

*Forward-looking planning lives in [`PRD.md` §12](../spec/PRD.md#12-milestones). Pitfalls
and load-bearing rationale live in [`LESSONS.md`](LESSONS.md).*
