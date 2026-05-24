# Dub — Shipped Milestones

> Companion to [`docs/PRD.md`](PRD.md). The PRD's milestone table keeps shipped rows
> short; this doc holds the detailed write-ups, design history, and rationale
> for each milestone and dogfooding round that has landed. Forward-looking
> milestones stay in the PRD.
>
> **Why split?** Shipped milestones accumulate prose that's load-bearing for
> "why is the code this way" archaeology but is no longer load-bearing for
> "what are we building next." Keeping them in the PRD bloated it past the
> point where a reader (or AI assistant) could keep the whole roadmap in
> working memory. Load this file by anchor; do not read the whole file unless
> doing a history audit.

**Currently shipped:** engine/audio/timecode/Thru/BPM/waveform foundations (M0 → M10.8), library schema/import/dedupe/browser/scanner/analysis/key detection (M11a → M11d.x + M11c.x), and several M11d.5 dogfooding rounds for playback, beat-grid, waveform, and library polish. The current build/test status belongs in the latest commit or PR, not in this historical overview.

## Table of contents

- [M0 — Scaffold + CI + test discipline](#m0)
- [M0.5 — Apple shell + smoke screen](#m05)
- [M1 — First Sound](#m1)
- [M2 — Transport (lock-free command channel)](#m2)
- [M2.1 — RT discipline + soak harness](#m21)
- [M3 — Format coverage + hot track loading](#m3)
- [M3.5 — De-click envelope + tail-fade + offline analyzer](#m35)
- [M4 — Two decks + debug mixer](#m4)
- [M5.1 — Timecode decoder, offline (clean-room)](#m51)
- [M5.2 — Audio input plumbing](#m52)
- [M5.3 — Live timecode → deck (first scratch)](#m53)
- [M5.4 — Calibration + scope (M5.4.1 + M5.4.2)](#m54)
- [M5.4.3 — Calibration speed (industry-parity)](#m543)
- [M5.4.4 — Per-deck calibration](#m544)
- [M5.4.5 — Late-binding decks + non-blocking calibration](#m545)
- [M5.4.6 — Always-fresh calibration (gut the fingerprint probe)](#m546)
- [M5.5.1 — Engine routing primitive](#m551)
- [M5.5.2 — External-mixer 4-channel output routing](#m552)
- [M5.6 — Two-deck timecode](#m56)
- [M6 — Timecode v2 (Traktor MK1 + MK2)](#m6)
- [M7 — Thru Mode (per-deck input routing)](#m7)
- [M7.5 — BPM engine + offline analysis](#m75)
- [M8 — Auto-BPM on Thru — streaming driver](#m8)
- [M8.1 — BPM octave fix (log-band ODF + windowed-energy picker)](#m81)
- [M9 — Live waveform capture (Thru)](#m9)
- [M9.5 — dub-spectral extraction + 8-band peak capture](#m95)
- [M10-A — `dub-ffi` `DubEngine` UniFFI surface](#m10a)
- [M10-B — Metal renderer + first live broadband waveform](#m10b)
- [M10.1 — Multi-colour fragment shader](#m101)
- [M10.2 — Polish: deck B, palette presets, honest silence/clipping](#m102)
- [M10.2 remainder — superseded by M10.5h–p, then rolled back in M10.8](#m102-remainder)
- [M10.3 — Performance shell](#m103)
- [M10.4 — Vertical waveform + symmetric two-pane layout](#m104)
- [M10.5 — File playback dev loop (M10.5a + M10.5b)](#m105)
- [M10.5c — Track Overview waveform + horizontal-orientation shader](#m105c)
- [M10.5d — Background load (decode + peaks off-thread)](#m105d)
- [M10.5e — Waveform polish (compression + past-region dim + brighter floor)](#m105e)
- [M10.5f — Waveform 2× zoom-in](#m105f)
- [M10.5g — Waveform anti-alias + temporal smoothing](#m105g)
- [M10.5h → M10.5p — Shader exploration ladder (rolled back in M10.8)](#m105hp)
- [M10.5n — Playhead-vs-audio drift root-cause fix (survives M10.8)](#m105n)
- [M10.6a–e — Mouse transport, Panic Play, transport-cluster redesign, Repeat auto-trigger](#m106)
- [M10.7 — Phase-Drift Trail](#m107)
- [M10.8 — Track Preparation Mode shell + Serato-parity waveform baseline freeze](#m108)
- [M11a — Library schema + path-by-volume-UUID](#m11a)
- [M11b — Canonical fingerprint + version-aware dedupe](#m11b)
- [M11c — Filesystem importer + filename parser](#m11c)
- [M11d.1 — Library browser shell](#m11d1)
- [M11d.2 — Recently Played wiring + sortable columns](#m11d2)
- [M11d.3 — Per-row indicators](#m11d3)
- [M11d.4 — Background missing-files scanner + Relocate panel](#m11d4)
- [M11d.5 — Dogfooding bug-fix and waveform/library polish rounds](#m11d5)
- [M11c.4 — Lazy fingerprint (import-fast, analyze-on-demand)](#m11c4)
- [M11c.3a — BPM octave fix (perceptual tempo prior)](#m11c3a)
- [M11c.3c — Reggae skank double-time rejection](#m11c3c)
- [M11c.3d — Genre-aware octave profile (library analysis)](#m11c3d)
- [M11c.3e — Hip-hop double-time rejection (Default profile)](#m11c3e)
- [M11c.3f — FourOnFloor profile (house / garage library)](#m11c3f)
- [M11c.2 — Key detection (Camelot canonical)](#m11c2)
- [M11c.1 — Lazy auto-beatgrid + analysis lifecycle](#m11c1)
- [M11d.6 — Full-screen on launch + windowed snap-back](#m11d6)
- [M11d.7 — Beatgrid precision, auto downbeat, tap-to-grid, drift lock](#m11d7)

---

<a id="m0"></a>
## M0 — Scaffold + CI + test discipline

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days

`cargo nextest` passes, `clippy -D warnings` green, RT-audit harness runs on a no-op render, xcframework builds, blank SwiftUI app launches and prints "engine OK" from Rust. GitHub Actions CI configured per PRD §10.4. Branch protection on `main` enabled. First TDD-discipline test exists and runs.

---

<a id="m1"></a>
## M1 — First Sound

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 4–6 days

Single deck, internal mode, plays a WAV through CoreAudio at < 8 ms latency. Property tests for buffer math; golden tests for resampler output; RT-audit green during playback.

---

<a id="m2"></a>
## M2 — Transport (lock-free command channel)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 3–4 days

Main thread can `play` / `pause` / `seek` / `set_rate` / `set_gain` deck 0 while CoreAudio is playing, via a `ringbuf` SPSC queue drained at the start of every render block. UI reads deck position / playing / at-end via per-deck atomic snapshot (`AtomicU64` of `f64` bits, Relaxed). RT-audit: 100k blocks alloc-free **including** drain of pre-staged commands. CLI demo: `dub play <file> --realtime --pause-at 1.0 --resume-at 2.0 --seek-at 3.0=4.0` produces an audibly correct pause/resume/seek with snapshot-correct end state.

---

<a id="m21"></a>
## M2.1 — RT discipline + soak harness

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 3–5 days

rt-audit green under stress; 1-hour playback with no xruns at 64-sample buffer; soak test harness in CI runs nightly; first parser fuzz target wired up (ID3 reader). Folded as a milestone-internal gate before M3, not a user-visible milestone.

---

<a id="m3"></a>
## M3 — Format coverage + hot track loading

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 4–6 days

Loads MP3 / FLAC / AIFF / M4A in addition to WAV (everything decoded fully into RAM per PRD §4.4 — no streaming). `Command::DeckLoad(Arc<Track>)` allows changing decks live; old `Arc<Track>` is returned to the main thread via a trash channel and freed off the audio thread. CLI demo: `dub play <A> --hot-swap-at WALL=<B>` audibly swaps A→B mid-playback. Sample-accurate seek across all formats (already works since everything is in memory).

---

<a id="m35"></a>
## M3.5 — De-click envelope + tail-fade + offline analyzer

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 1–2 days

Two complementary primitives sharing one precomputed `sin²` envelope (2 ms × engine SR):

1. **Transport-change declick** — an equal-power crossfade between pre- and post-mutation state on every track load, seek, and play/pause.
2. **Tail-fade** — a multiplicative envelope applied as the playhead approaches a track's natural end so walking off the last sample doesn't step to 0.

Both are gated by a `track_len ≥ 2 × envelope_length` threshold so synthetic short tracks aren't obliterated. Back-to-back transport changes routed via a single-slot `pending_disposal` + `AtomicU64` overflow counter; old `Arc<Track>`s never drop on the audio thread.

New `dub analyze <wav>` subcommand reports peak/RMS/DC, clipping, and max per-sample first-difference, flagging any `|s[i] − s[i-1]|` above a configurable threshold (default 0.05) — replaces subjective listening with mathematical click detection. Offline `dub play -o` now supports the same scheduled transport events as realtime, so any scenario can be rendered deterministically and audited end-to-end.

---

<a id="m4"></a>
## M4 — Two decks + debug mixer

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days

Both engine decks (`DECK_COUNT = 2`) drivable end-to-end through the CLI: `dub play <A> <B>` loads independent tracks onto deck A and deck B, both summed by the engine's existing additive deck loop into one stereo bus. Debug internal mixer adds a single `master_gain` field on `Engine` (M4 addition) plus the existing per-deck `set_gain`, applied multiplicatively after deck summing — pass-through when `master_gain == 1.0` to avoid the per-block multiply on the common case.

New `Command::SetMasterGain` and `EngineHandle::set_master_gain` so the master is mutable mid-playback through the same lock-free SPSC channel as transport. CLI gains `--deck-b-*` mirrors of every transport flag (`--deck-b-rate`, `--deck-b-gain`, `--deck-b-pause-at`, `--deck-b-resume-at`, `--deck-b-seek-at`, `--deck-b-hot-swap-at`) plus `--master-gain G` and `--master-gain-at WALL=G`; bare flags target deck A for backward-compat with single-deck usage. `ScheduledEvent` carries a per-event `deck` index so each scheduled event addresses the right deck; engine-wide events (master gain) carry no deck.

**External-mixer 4-channel routing is intentionally deferred** to M5/M6 where it's needed by the timecode hardware (SL3, Audio 6) — v1's debug mixer sums to one stereo output for now. CLI demo: `dub play <A> <B> --master-gain-at 1.0=0.6 --hot-swap-at 1.5=<C> --deck-b-pause-at 2.0 --deck-b-resume-at 3.0 -o out.wav && dub analyze out.wav` reports CLEAN with `max delta ≤ 0.026` (well under the 0.05 click threshold). Realtime path verified audibly. RT-audit extended to alternate command/load traffic across both decks plus periodic master-gain churn — 100k blocks alloc-free under `assert_no_alloc`.

---

<a id="m51"></a>
## M5.1 — Timecode decoder, offline (clean-room)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 3–5 days

New `dub-timecode` crate decoding Serato CV02 from stereo audio in **relative mode only**. Algorithm: treat `s = L + jR` as a complex analytic signal; compute the coherent block sum of `s_n · conj(s_{n-1})`; per-block instantaneous frequency = `arg(sum) / (2π·Δt)`; rate = `f_inst / carrier_hz` (signed — negative = reverse); position integrates rate × block-seconds. Confidence = `|sum| / Σ|s|²` (1.0 = pure carrier, 0.0 = noise).

RT-safe (alloc-free under `assert_no_alloc`). Fully unit-testable on synthetic stereo quadrature signals — no hardware required. Bitstream/absolute decode deferred to M6. **Clean-room implementation** from xwax/Mixxx algorithm description; no xwax code copied.

CLI: `dub decode-timecode <wav>` reads recorded timecode and reports rate / position / amplitude / confidence per window with a LOCKED/PARTIAL/POOR verdict; `--synthetic` runs a built-in 1.0× → 0.5× → -1.0× → silence scenario for sanity-checking without a turntable.

---

<a id="m52"></a>
## M5.2 — Audio input plumbing

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days

`dub-audio` gets an `AudioInput` primitive mirroring `AudioOutput`: HAL input AudioUnit, ringbuf-buffered handoff to a consumer thread. CLI: `dub capture` (writes input to WAV) and `dub levels` (live meter). Verified on default mic input first, SL3 input pair second.

See [`docs/ARCHITECTURE.md` → HAL input invariant](ARCHITECTURE.md#hal-input-invariant--sample-rate-match-m52) for the load-bearing sample-rate-match footgun this milestone closed.

---

<a id="m53"></a>
## M5.3 — Live timecode → deck (first scratch)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days

Wire `AudioInput` → `dub-timecode::Decoder` → engine deck in **relative mode**: per-block decoded rate is applied to the deck via `set_rate`; lift detection runs three layers — (a) **amplitude gate** (`DEFAULT_AMPLITUDE_THRESHOLD = 0.01` RMS) overrides confidence whenever the carrier is dead, since handling/rumble noise on a lifted cartridge can produce moderate confidence at near-zero RMS, (b) **two-edge confidence hysteresis** (engage `0.8`, disengage `0.5`) for clean scratch-transient handling, (c) **sticky-block window** (4 blocks ≈ 21 ms @ 256-frame / 48 kHz) for dust-tick immunity.

Three iterations on the SL3 drove the design: the first single-threshold gate chattered on lift; the second confidence-only hysteresis treated lift as a "lukewarm scratch transient" and burst-played the track while the needle was up; the amplitude gate closes that hole. The state machine is factored into a pure `step_policy(DecodeOutput) → Intent` on top of `drive(...)` (which sources data from the ringbuf), so each pathology has a dedicated regression test. The decoder consumes the input ringbuf directly on the audio thread inside `Engine::render` — no extra thread, no extra channel — so the only added latency on top of M5.2's input ring is one `Decoder::process` call per render block (~µs).

New public engine surface: `Engine::attach_timecode_input(deck_idx, HeapCons<f32>, TimecodeInputConfig)`, `Engine::detach_timecode_input`, `Engine::timecode_last_output(deck_idx)` for UI observability. New `dub_audio::AudioInput::take_consumer()` lets the consumer end of the IOProc → consumer ringbuf move into the engine while the `AudioInput` itself stays on the main thread for shutdown.

**`AudioOutput` now also force-aligns the output device's nominal SR to engine SR** (same gauntlet as `AudioInput`) — first SL3 validation surfaced an 8 % pitch drift when output was at 44.1 kHz and engine at 48 kHz because the CoreAudio HAL DefaultOutput unit does not reliably SRC across that boundary. Position drift correction (re-syncing deck position to decoded position over wall time) is intentionally deferred — relative-mode in v1 lets position evolve via integration of rate, which is what platter motion already encodes.

**rt-audit extended** with a 10k-block timecode-driven render path under `assert_no_alloc`, verifying the entire Decoder + transport-update path is heap-free on the audio thread. CLI: `dub timecode-deck <track.wav> --input-channels N,M [--device NAME] [--duration SECS] [--confidence T] [--disengage-threshold T] [--sticky-blocks N] [--amplitude-threshold T]`.

**Demo criterion:** scratch a record on Deck A, hear Deck A's loaded track react with sub-buffer-size latency, see deck mute cleanly on stylus lift with no track audio leakage, see direction reversal on backspin. *This is the milestone where Dub becomes a DJ app.*

---

<a id="m54"></a>
## M5.4 — Calibration + scope (M5.4.1 + M5.4.2)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** scope 1 day; calibration 2 days

Split into two delivered sub-milestones because the scope is independently valuable and lands a refactor that calibration also needs.

### M5.4.1 — TUI scope (`dub scope`)

Opens the input device, runs the same `LiftPolicy` as `dub timecode-deck`, and renders a ratatui inspector: Lissajous of input `(L, R)` (the carrier should trace a clean circle; lift collapses it to a noisy blob), `[LOCKED]` / `[LIFT]` engagement badge, gauges for confidence and amplitude (color-coded against current thresholds), rate readout with a centered slider in `[-2×, +2×]`, sticky countdown bar showing the policy's `consecutive_below` counter walking toward disengage, and a row of live thresholds. Arrow keys mutate engage / disengage / amplitude *in place* so users can find sane defaults for their cartridge against their actual signal — calibration sandbox that M5.4.2 persists. Block size pinned to 256 frames so the scope's policy decisions match `timecode-deck` 1:1.

**Refactor:** `step_policy` and the engagement state were factored out of `TimecodeInput` into a public `LiftPolicy { engage, disengage, sticky, amplitude, engaged, consecutive_below, last_locked_rate }` with a `step(DecodeOutput) -> LiftIntent` method; `TimecodeInput` now embeds it and delegates. Three callers — engine playback, `dub scope`, and `dub calibrate` (M5.4.2) — share exactly the same lift behavior because they share the code path.

New CLI: `dub scope [--device NAME] [--input-channels N,M] [--engage T] [--disengage T] [--sticky N] [--amplitude T] [--format serato-cv02] [--duration SECS]`. New deps in `dub-cli` only: `ratatui` 0.30, `crossterm` 0.29 (engine and audio crates untouched).

### M5.4.2 — Calibration UX (`dub calibrate`)

Measures the user's specific *rig* (cartridge + preamp + cabling + soundcard) and persists derived thresholds + a rig fingerprint to `~/.dub/calibration/<device_key>_<format>.json`. Per-rig — not just per-soundcard — because a cartridge swap on the same SL3 changes the carrier amplitude by 50 %+ and would silently misfire a soundcard-only calibration.

Two zero-prompt phases: (1) *carrier* — auto-detects stable carrier (5 consecutive blocks: confidence ≥ 0.85, |rate − 1| < 0.10), captures 10 s; (2) *lift* — auto-detects lift (10 consecutive blocks: amp < 0.005), captures 5 s. From the percentile shapes (P5/P50/P95 of amplitude + confidence per phase) it derives `engage = carrier.conf_p5 - 0.03` (clamped 0.7–1.0), `amplitude = carrier.amp_p5 / 2`, keeps `disengage = 0.50` and `sticky = 4` from M5.3 defaults.

Stores a `RigFingerprint { carrier_amp_p50, carrier_amp_p95, carrier_conf_p50 }` — carrier-only on purpose; lift noise rises 10–100× in clubs vs. lab and would false-flag every venue change as "rig changed". `dub timecode-deck` startup loads the JSON, briefly probes the carrier (3 s) to validate the fingerprint at 30 % tolerance, and either uses the saved thresholds (match) or auto-recalibrates (mismatch — cartridge or preamp swap). `--recalibrate` forces fresh measurement; `--no-probe` skips fingerprint check; `--no-calibrate` falls back to M5.3 defaults. Per-knob CLI flags (`--confidence`, `--amplitude-threshold`, …) still override individual thresholds for partial overrides. SNR sanity check refuses to ship thresholds when carrier-to-lift SNR is below 10× (likely cartridge / cabling problem). Schema-versioned JSON includes the full P5/P50/P95 measurements so future formula changes (M5.4.3, M6) can re-derive thresholds without forcing a remeasurement.

> **Superseded:** M5.4.6 later gutted the load-from-disk + fingerprint-probe machinery. The JSON file is now a diagnostic artifact only; the runtime always recalibrates on startup. See [M5.4.6](#m546).

---

<a id="m543"></a>
## M5.4.3 — Calibration speed (industry-parity)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 1 day (came in slightly ahead of the 1–2 day estimate; item 4 partial deferral kept scope tight)

M5.4.2 shipped *correct* per-rig calibration but at industry-trailing speed: live SL3 + Concorde validation showed ~25 s total wall time for the initial calibrate (10 s carrier capture + 5 s lift capture + two ≥ 5 s detection waits at `STABLE_BLOCKS = 5`), and the 3 s startup probe felt longer than 3 s in practice because of the same wait preamble.

**Goal:** match Traktor's "drop the needle, hit calibrate, you're done" feel. Achieved on shipping live SL3 + Concorde + Traktor MK1 vinyl: ≈ 3.5 s first-time calibration (was ~25 s), ≈ 1.7 s startup probe on a known rig (was ~5 s, claimed 3 s).

**What shipped:**

1. **Single-phase calibration** — eliminated the lift step entirely. Lift noise was always deliberately *not* on the fingerprint; M5.4.1 SL3 hand-tuning showed the carrier shape carries the threshold information (`amplitude = carrier_p5 * 0.5` matches the user's hand-found threshold within 1 % regardless of lift level), and lift was always the SNR safety net, never the signal source. New `MeasureOptions { two_phase: bool, .. }` opts struct routes through `measure_inline`; default is `two_phase = false`. Lift stats are persisted as `MeasurementStats::zero()` (`n_blocks == 0`) for schema compatibility — `derive_thresholds` recognizes the zero sentinel and skips the SNR check, so the JSON loads identically with single-phase or two-phase data downstream.
2. **Shorter carrier capture** — `DEFAULT_CARRIER_SECS` 10.0 → 3.0 s (≈ 564 blocks @ 256 frames @ 48 kHz). M5.4.1 + M5.4.4 captures show percentile convergence within < 1 % by ~ 2 s on a steady spin; 3 s leaves a small safety margin without user-visible cost.
3. **Faster detection threshold** — `STABLE_BLOCKS` 5 → 2 (≈ 11 ms detection wait at 256/48 kHz block size), with `CARRIER_DETECT_CONF` simultaneously tightened from 0.85 → 0.90 so 2 blocks is unambiguous. The user's deck-B SL3 carrier_conf_p5 ≈ 0.96 still passes comfortably; the rate gate (`|rate-1| < 0.10`, unchanged so ±10 % pitch fader keeps working) catches transient stylus motion because handling produces near-zero or wildly varying rate, never the unity rate of a steady spin.
4. **Startup probe accelerator (partial)** — `PROBE_SECS` 3.0 → 1.5 s; combined with the new `STABLE_BLOCKS = 2`, the effective probe-side wall time on a known rig drops to ≈ 1.7 s. The "run probe *concurrently* with timecode-deck spinup" half of the original M5.4.3 sketch was deferred to M5.4.5 because it requires the same architectural lift (mid-stream `attach_timecode_input`, parallel calibrators) that M5.4.5 builds for the takeover scenario; bundling it here would have either pre-built or duplicated that infrastructure.
5. **`--two-phase` opt-out** — `dub calibrate --two-phase` keeps the legacy M5.4.2 flow available for diagnostics (cartridge / preamp / cabling troubleshooting where the SNR safety net actually matters). Auto-calibration in `dub timecode-deck` always uses single-phase — the user explicitly opts into two-phase via the bare `dub calibrate --two-phase` invocation.

**Test surface (5 new, 273 workspace total, was 265 after M5.4.4 + 3 from M5.4.3 prep):** `derive_thresholds_skips_snr_when_lift_not_measured`, `derive_thresholds_still_rejects_low_snr_in_two_phase_mode`, `measurement_stats_zero_signals_unmeasured`, `parse_opts_default_is_single_phase`, `parse_opts_two_phase_flag_round_trips`, `parse_opts_default_carrier_secs_is_3`, `carrier_detect_constants_match_m543_targets` (pins all four tuned constants), `m543_probe_and_auto_calibration_constants_are_fast` (pins `PROBE_SECS = 1.5`, `AUTO_CARRIER_SECS = 3.0`). The constant-pinning tests are deliberate: any silent revert (e.g. someone bumps `STABLE_BLOCKS` back to 5 to "be safe") brings back the user-visible 25 s pain point, and we want a build-time failure rather than a quiet drift.

**Out of scope (deferred to M5.4.5):** concurrent probe-and-spinup.
**Out of scope (deferred indefinitely):** SNR-derived runtime ghost-noise warnings — single-phase loses the SNR floor at calibrate time, but the M5.4.5 + M10 runtime audio path can warn if observed lift amplitude exceeds the calibration's `amplitude` threshold for an extended window.

---

<a id="m544"></a>
## M5.4.4 — Per-deck calibration

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 1–2 days

M5.6 shipped two-deck timecode but probed pair 0 (deck A) only and silently reused those thresholds for deck B — correct for matched cartridges (the common case in vinyl-DJ rigs), silently wrong otherwise (ghost-noise on lift, premature disengage on light scratches, no error message). M5.4.4 makes calibration **per deck**: probe and store independently for each deck, fingerprint per deck, auto-recalibrate per deck on rig swap.

Calibration JSON keys by `(device, deck_index, format)` (path pattern `~/.dub/calibration/<device>_deck_<idx>_<format>.json`) instead of the M5.4.2 `(device, format)` pattern. The fingerprint-probe machinery runs once per deck sequentially in two-deck mode — deck A's full carrier+lift first, then deck B's — with per-deck status banners (`calibration A:`, `calibration B:`) so the user knows which side they're spinning/lifting at any moment.

**Backward compat:** deck 0 falls back to the legacy single-deck JSON (`<device>_<format>.json`, no deck infix) when the new per-deck file is missing, so existing user calibrations from M5.4.2 / M5.4.3 / M6 keep working without a migration step; the loader writes the calibration forward to the new path on the next save. Deck 1 has no legacy file (pre-M5.4.4 only stored deck A) so it always runs a fresh calibration on first M5.4.4 use, which is the correct behavior.

**API surface:** `Calibration::path_for(device, deck_index, format, dir)` (was 3-arg), `Calibration::path_for_legacy(device, format, dir)` (legacy fallback only), `Calibration::load_with_legacy_fallback(new, legacy)` (transparent migration), and `measure_inline(input, pair_idx, deck_index, format, ...)` / `probe_carrier(input, pair_idx, format, ...)` taking a pair index so they read from the right `read_into_pair(idx)` source in two-deck mode. The `pair_idx` (where to read on the AudioInput, M5.6 demuxing) is intentionally separate from `deck_index` (on-disk metadata) — `dub calibrate --input-channels 5,6 --deck 1` opens its own 2-channel input (so pair_idx=0 always there) but stamps the result as deck 1; `dub timecode-deck` two-deck mode uses pair_idx==deck_idx. Conflating them caused a bug during implementation that was fixed before merge.

**CLI:** `dub calibrate` gains `--deck 0|1` (default 0, rejects ≥ 2 because the engine has 2 decks today). `dub timecode-deck` runs `resolve_thresholds` once per deck in two-deck mode and prints two `timecode A:` / `timecode B:` lines instead of the M5.6-era shared-calibration single line. Test surface: 13 new dub-cli tests covering per-deck path keying, legacy-fallback load behavior (legacy used when new missing, new wins when both present, both-missing errors), `--deck` flag parse + validation (rejects 2/99/letters), deck-label mapping (0→A, 1→B), `deck_index` JSON round-trip, legacy JSON without `deck_index` field defaulting to deck 0 (`#[serde(default)]`). 265 workspace tests total (was 252 after M6).

**Explicitly out of scope** (deferred to v1.x or never): named cartridge profiles, profile libraries, cross-session "auto-load the right cartridge" UX. The earlier M5.4.4 design ("library of named profiles, fingerprint-match across them") was over-scoped — once M5.4.3 makes calibration ≤ 5 s, "always recalibrate on startup" is the simpler model with zero profile-management UX surface, and matches the user's mental model ("calibrate auto-runs on app start; if I swap a cartridge mid-set I press the calibrate button"). The "calibrate button" is part of M10 (UI); on the CLI today, `dub calibrate --deck 0` / `--deck 1` already serves that role.

**Known product gap deferred to M5.4.5** (not a polish item — the canonical DJ-takeover use case is structurally incompatible with M5.4.4's "calibrate both then start" model): when the incoming DVS DJ drops onto deck A while the previous DJ is still playing on deck B, deck B's record literally does not exist for calibration to run against. M5.4.5 makes each deck progress through `Unconfigured → Calibrating → Ready` independently — single-deck startup, mid-stream deck-add, audio plays during deck B's later calibration. Acceptable for the CLI dev tool today (no live use); blocking for any actual product release.

---

<a id="m545"></a>
## M5.4.5 — Late-binding decks + non-blocking calibration

**Status:** shipped &nbsp;·&nbsp; **Estimate:** product correctness, not polish

**Why this was a product gate, not a polish item:** the canonical DJ use case — DJ takeover — has the previous DJ still playing on deck B when the incoming DVS DJ drops onto deck A. The incoming DJ has *zero access* to deck B's record for the entire takeover window (which can be 5 minutes or 60 minutes). M5.4.4's "calibrate both decks then start audio" model was structurally incompatible with this: deck B's record does not exist *to be calibrated against* until the previous DJ leaves. A faster M5.4.3 calibrate doesn't help either — the issue isn't *how long* deck B's calibration takes, it's that deck B *has no record on it* at the moment audio must start.

**What shipped** (smaller than the original plan; it covers the takeover gate but defers a couple of nice-to-haves to follow-up):

1. **Engine surface — `EngineHandle::attach_timecode_input` (NEW).** The pre-M5.4.5 `Engine::attach_timecode_input(&mut self, …)` was synchronous and required `&mut Engine`, which means it could only be called *before* `AudioOutput::start_with_options` consumes the engine. M5.4.5 adds a parallel command-channel path: `EngineHandle::attach_timecode_input(idx, rx, cfg)` constructs `TimecodeInput::new` on the main thread (allocates), boxes it, and pushes a new `Command::AttachTimecodeInput { idx, input: Box<TimecodeInput> }` through the existing SPSC channel. The audio thread slots it into `engine.timecode_inputs[idx]`. If the slot was already filled (mid-stream re-cal), the displaced `Box<TimecodeInput>` is bounced back through a *second* trash channel (mirroring the `Arc<Track>` trash pattern from M3.1) for main-thread disposal — never dropped on the audio thread. New `EngineHandle::reclaim` drains both trash channels in one call. New `EngineHandle::timecode_trash_overflow_count` surfaces leak diagnostics. Pinned by 5 new engine tests (75 total, was 70).

2. **Calibrator API refactor.** `measure_inline` and the helpers `wait_for_stable_carrier` / `wait_for_lift` / `capture_phase` now take `&mut HeapCons<f32>` instead of `&mut AudioInput + pair_idx`. The exclusive borrow on `AudioInput` was what forced sequential calibration; now each calibrator owns its own consumer ring and two of them can run on two threads with no shared mutable state. The old `(device_name, sample_rate, deck_index, format)` metadata that used to be pulled off `AudioInput` is bundled into a new `MeasurementInputs` struct that the caller fills once and hands by reference (or moves) to each calibrator. New `MeasureOptions::detect_timeout_secs: Option<f64>` (was `f64`); `dub timecode-deck` startup passes `None` so the deck-B calibrator can wait indefinitely for the takeover window, while `dub calibrate` keeps the legacy 30 s timeout for the "user forgot the needle" safety net.

3. **`dub timecode-deck` flow.** Reordered: take both consumers out of `AudioInput`, build `Engine::new_with_handle`, load tracks (decks default to paused), `AudioOutput::start_with_options(engine, …)` immediately — both decks render silence into the output bus. Then spawn one `std::thread::spawn` worker per declared deck. Each worker owns its `HeapCons<f32>` + a `MeasurementInputs` bundle + an optional save path; on completion it sends `(deck_idx, Result<(HeapCons<f32>, Calibration)>)` back to main via an `mpsc` channel. Main interleaves stats-print (500 ms tick) with `rx.try_recv` polling at the same tick — as each calibrator finishes, main applies CLI overrides, builds a `TimecodeInputConfig`, and calls `handle.attach_timecode_input(idx, consumer, cfg)`. That deck goes live mid-stream; the other deck's calibrator keeps waiting independently.

   **Why detached `thread::spawn` and not `thread::scope`:** scope's auto-join would block forever at scope-exit if a calibrator is still waiting for a never-appearing carrier (process Ctrl-C window). Detached threads are cleaned up by the OS at process termination — acceptable for a CLI tool with `--duration` + Ctrl-C as the exit paths.

4. **Drain step deletion.** M5.4.4's `drain_input_pair` between sequential calibration and engine attach is gone — parallel calibrators consume their rings continuously, so there's no idle-pair stale-audio buildup to flush. The IOProc still pushes ~10 ms during the worker→main→engine handover; the existing 4 s ringbuffer absorbs that without effect.

5. **Output-now-decks-later semantics.** Decks loaded but paused before output start; `AudioOutput::start_with_options` brings up the device immediately, both decks render silence into routed output channels until their `TimecodeInput` is attached. The user sees a working audio chain (output device alive, no clicks) while calibrators work. The lift policy starts in "lifted" state on attach so the deck stays muted until the user drops the needle and the carrier locks.

**Takeover use case (the actual product gate, validated):** incoming DJ launches `dub timecode-deck a.wav b.wav --input-channels 3,4 --deck-b-input-channels 5,6 --format serato-cv02`. Both calibrator threads start. AudioOutput is up immediately. Deck A's calibrator detects the carrier the moment the DJ drops a needle on A; that deck attaches mid-stream and audio plays. Deck B's calibrator is sitting in `wait_for_stable_carrier` with `detect_timeout_secs = None` — could be 60 minutes. When the previous DJ finally vacates and the incoming DJ drops a record on B, deck B's carrier appears, the calibrator wakes up, completes, attaches. Deck A audio is uninterrupted across the entire window. **No hot-keys needed for this** — passive-wait on the calibrator side absorbs the takeover window naturally.

**Deferred to follow-up M5.4.5+:**

- **(a)** Hot-key `B` for mid-stream re-attach when `--deck-b-input-channels` *wasn't* declared at startup (e.g., DJ launches single-deck and later decides to add deck B). Engine surface is ready (replace-and-trash on `AttachTimecodeInput` works, `Box<TimecodeInput>` trash channel exists), but the CLI plumbing for crossterm hot-keys + dynamic `AudioInput` reconfiguration is its own piece of work.
- **(b)** Mid-set re-calibration via hot-key (cartridge swap during a set). Same engine surface; same deferral reason.
- **(c)** `--sequential-calibrate` debug flag. The original PRD entry called for this as a M5.4.4 fallback; not implemented because the parallel path is the only path now and there's no observed regression to fall back from. Add if needed.
- **(d) Follow-up landed alongside M5.4.5 live validation: `--duration` is now optional, default = run until Ctrl-C.** The old 60 s default (`DEFAULT_RUN_SECS`) was a holdover from M5.3's "validation run" mindset, where calibration was a few seconds and a 60 s wall-clock test was a natural shape. With M5.4.5's takeover scenario explicitly in scope — deck B's calibrator may legitimately wait 5–60 minutes for the previous DJ to vacate — a hard wall-clock exit would silently *drop the deck B calibration window*: the calibrator is still sitting in `wait_for_stable_carrier`, then the process exits, then a record drops on B and nothing happens. `Opts::duration_secs` is now `Option<f64>`; main-loop becomes `while opts.duration_secs.is_none_or(|d| start.elapsed() < d)`. `--duration N` is preserved for scripted / CI smoke tests (this is what unit tests of `parse_opts` pin). Startup banner adapts: `Some` → "running for N s — drop the needle and play", `None` → "running until Ctrl-C — drop the needle and play".

**Acceptance (validated live):**

1. Single-deck `dub timecode-deck a.wav --input-channels 3,4` works end-to-end as before (no deck B), audio plays on A after calibrator finishes.
2. Two-deck startup with both decks spinning shows parallel calibration banners and audio starts on whichever deck calibrates first.
3. Takeover scenario — only deck A spinning at startup — audio plays on A while deck B's calibrator banner shows it still waiting; minutes later when deck B is spun, audio appears on B without any audible disturbance to A.

---

<a id="m546"></a>
## M5.4.6 — Always-fresh calibration (gut the fingerprint probe)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 0.5 day

Realised on the back of M5.4.3's speedup that the entire save+probe machinery (M5.4.2 → M5.4.5) was solving a problem the production audience (touring DJs) doesn't have. The save+probe model assumes the user calibrates once and reuses across sessions; the probe pays for itself when the rig is unchanged because reusing thresholds beats remeasuring. **For touring DJs every venue brings a different turntable + cartridge: the fingerprint *always* mismatches, the probe *always* burns ~1.7 s confirming what we already know, and the auto-recalibration runs anyway.**

Net cost on the production path was probe (~1.7 s) + recalibrate (~3.5 s) = ~5.2 s — *worse* than a no-cache always-fresh model that pays only the ~3.5 s calibrate. Net cost for the bedroom DJ (one fixed rig) is ~1.7 s either way (probe+match vs. fresh calibrate). **Decision: gut the probe, always recalibrate on startup.**

**Runtime change:** `dub timecode-deck`'s `resolve_thresholds` collapses to "if `--no-calibrate` → M5.3 defaults; else run a fresh single-phase M5.4.3 calibration; save the result as a diagnostic artifact". No load-from-disk, no fingerprint comparison, no auto-recalibrate-on-mismatch path, no legacy-format migration, no stale-age warning.

**What stays:** `Calibration` JSON schema is unchanged for forward+backward compat with M5.4.2 … M5.4.5 files (existing JSONs still parse fine, future analysis tooling still has the percentile data). `RigFingerprint` stays as a struct field — written at calibration time as a record of "what did this rig look like" — but the comparison code (`matches`, `max_relative_delta`, `within_relative`, `relative_delta`, `DEFAULT_FINGERPRINT_TOLERANCE`) is gone. `Calibration::path_for(device, deck_idx, format, dir)` stays for the diagnostic save. `Calibration::load` stays `pub` (with `#[allow(dead_code)]` on the binary path) for tests + future `dub inspect-calibration` tooling. `dub calibrate` is unchanged — still writes a per-deck JSON for inspection.

**What goes away:** `RigFingerprint::matches / max_relative_delta / within_relative / relative_delta`, `DEFAULT_FINGERPRINT_TOLERANCE`, `Calibration::path_for_legacy`, `Calibration::load_with_legacy_fallback`, `legacy_device_key`, `probe_carrier`, `legacy_calibration_path_for`, `calibration_age_days`, `STALE_CALIBRATION_DAYS`, `PROBE_SECS`, `PROBE_DETECT_TIMEOUT_SECS`, the `time::OffsetDateTime` / `Rfc3339` imports they fed, plus the CLI flags `--recalibrate` and `--no-probe`. `--no-calibrate` survives because "fall back to M5.3 defaults" remains a useful no-hardware testing path.

**CLI breakage:** `--recalibrate` and `--no-probe` are now rejected as unknown flags (caught by `parse_opts`'s leftover-flag check) so anyone with a copy-pasted old invocation gets a clear error instead of a silently-ignored flag. Pinned by `parse_opts_rejects_dropped_recalibrate_flag` and `parse_opts_rejects_dropped_no_probe_flag`.

**Test surface delta:** −14 (deleted: 6 fingerprint matching, 1 legacy_device_key, 1 path_for_legacy, 3 load_with_legacy_fallback, 3 calibration_age_days, 1 legacy_calibration_path_omits_deck_infix, 1 parse_opts_recalibrate_flag, 1 parse_opts_recalibrate_and_no_calibrate_conflict) + 3 (added: parse_opts_no_calibrate_flag, parse_opts_rejects_dropped_recalibrate_flag, parse_opts_rejects_dropped_no_probe_flag). Workspace total 259 (was 273).

**What this does NOT change:**

- **(a)** M5.4.5's late-binding-decks design — that's about *availability* (deck B's record doesn't exist yet during a takeover), orthogonal to the save model. M5.4.5 still needs to land.
- **(b)** Per-knob CLI overrides (`--confidence`, `--amplitude-threshold`, …) — they apply on top of the fresh measurement just like before.
- **(c)** `dub scope` — already runs in-place threshold tuning, never touched the JSON.
- **(d)** Per-deck calibration (M5.4.4) — `dub calibrate --deck N` still writes `<device>_deck_N_<format>.json`, just nothing reads it back automatically.

**Why this isn't a regression on bedroom-DJ UX:** repeated bedroom sessions on the same rig now pay 3.5 s × 1 calibrator (per-deck) instead of 1.7 s probe + (occasionally) 3.5 s recalibrate. On the cold-start path the bedroom user gives up ~1.8 s and gains a much simpler mental model ("the app calibrates on every start, period") and never gets surprised by a stale calibration silently used.

**Acceptance:** `dub timecode-deck a.wav --input-channels 3,4` runs a fresh calibration (≤ 5 s wall time per deck) on every invocation and writes the JSON; no probe phase appears in the output. `dub calibrate --deck 0` writes the same JSON shape, manually. `--recalibrate` / `--no-probe` rejected with "unknown flag".

---

<a id="m551"></a>
## M5.5.1 — Engine routing primitive

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 0.5 day

New `Engine::render_routed(rt, out, num_channels, &[Option<u32>; DECK_COUNT])` writes each deck's stereo pair into a configurable channel pair of an N-channel interleaved output buffer. `Deck::render_into(rt, out, sr, stride, offset)` is the strided variant of `render`; `render` becomes a thin wrapper at `(stride=2, offset=0)`. Two decks routing to the same `Some(c)` sum (= M4 internal mixer); non-overlapping `Some` values isolate (= M5.5 external mixer).

`Engine::render` becomes `render_routed(out, 2, INTERNAL_MIXER_ROUTING)` so all M0–M5 callers stay byte-identical (verified by an explicit `render_routed_internal_mixer_matches_render` regression test). `routing[i] == None` skips a deck entirely — its transport state does NOT advance — pinned by tests; muting goes through `Deck::set_gain(0.0)` instead, which keeps the transport ticking. Master gain applies once across the whole multi-channel buffer at the end (zero × g == zero so unrouted channels stay zero). RT-safe (alloc-free, verified under `assert_no_alloc`). Pure-engine work, no hardware required.

---

<a id="m552"></a>
## M5.5.2 — External-mixer 4-channel output routing

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days

M5.5.1's primitive plumbed all the way to CoreAudio. New `dub_audio::OutputOptions { channels, buffer_frames, sample_rate, channel_map }` and `AudioOutput::start_with_options(engine, &opts, routing)` open the default output AU with N physical channels, force-align the device to engine SR (same gauntlet as the legacy stereo path), and run `Engine::render_routed` per callback so deck audio lands on the right physical pairs.

`DeviceInfo` grows a `device_name` so the CLI can match against `device_profiles::KNOWN_DEVICES` — a small static table of validated interfaces:

- **Serato SL 3** ✅ deck A → out ch 3+4, deck B → out ch 5+6, aux ch 1+2 (matches the SL3's per-deck wiring inside the box; matches the M5.2 input mapping the user already calibrates against, so `--input-channels 3,4` and deck A's *output* land on the same physical pair on the same box).
- **Traktor Audio 6** ⚠️ unverified deck A → out ch 1+2, deck B → out ch 3+4 (best-effort guess; warns at startup until validated against real hardware).

The startup line clearly states which routing was chosen and why (`output routing: Serato SL 3 (6 ch, deck A → ch 3+4, deck B → ch 5+6)` vs. `output routing: unknown device 'MacBook Pro Speakers' — falling back to internal mixer`).

**Resolution priority:** `--internal-mixer` (debug only) → manual `--deck-a-out-ch` + `--deck-b-out-ch` (always paired; partial errors out) → `--device-profile NAME` → auto-detect by device name → fallback to internal mixer with a loud warning ("not for live performance"; matches Serato's "preparation mode" semantics for laptop-only sessions). The internal-mixer fallback is *opinionated* about being a dev path: live performance on a laptop output is explicitly not supported because it has no per-deck physical separation, which violates the "no mouse DJ ever" rule.

Test surface: 8 device-profile tests (substring + case-insensitive matching, disjoint deck pairs, fit-in-channels invariant, 1-based ↔ 0-based conversion) + 11 routing-resolution tests (every priority branch + every CLI conflict pair) + reuse the M5.5.1 RT-safety guarantees on the engine side. Live SL3 validation: deck A on physical output ch 3+4 → physical mixer's deck-A line input → audible playback through the user's external mixer rig.

---

<a id="m56"></a>
## M5.6 — Two-deck timecode

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2 days

The unlock from "demo" to "I can DJ a set on this": load two tracks, drive them with two real timecode records on the same external interface, route their audio to two physically isolated mixer channels via M5.5.2. Implemented by demuxing one CoreAudio input AU (CoreAudio doesn't permit two AUs on the same input device) into N independent stereo SPSC ringbuffers on the IOProc thread.

New `dub_audio::InputOptions::output_pairs: Option<Vec<(u32, u32)>>` declares the per-pair `(L, R)` indices into the AU's interleaved-`channels` frame; the IOProc walks each frame and `push_slice`s 2 samples into each pair's ring (extracted to `push_demuxed_frames` for unit-test coverage — five tests pin single-pair pass-through, 4-ch isolation, swapped (L, R), overflow signalling, and partial-frame handling). New `AudioInput::take_consumer_pair(idx)` / `read_into_pair(idx, dst)` / `available_pair(idx)` / `pair_count()` API; the existing `take_consumer()` / `read_into()` / `available()` keep their semantics by aliasing to pair 0 (so M5.2 / M5.3 / M5.4 callers are byte-identical, verified by passing the existing 218 tests untouched).

On the CLI side, `dub timecode-deck` accepts `<track-a> [<track-b>]` (1 or 2 positional tracks) plus a new `--deck-b-input-channels N,M` flag; together they trigger two-deck mode and the helper `build_input_options` constructs a 4-channel `InputOptions` with `channel_map = [a_l-1, a_r-1, b_l-1, b_r-1]` (1-based CLI → 0-based AU) and `output_pairs = [(0, 1), (2, 3)]`. The two pair consumers are attached to engine deck 0 and deck 1 with the *same* `LiftPolicy` thresholds — calibration probes pair 0 only and shares its result, which is correct for matched cartridges (the common case) and gracefully degrades for mismatched ones (M5.4.4 will add independent per-deck calibration).

Validation rejects: track A without track B but with `--deck-b-input-channels`; track B without `--deck-b-input-channels`; overlapping deck-A / deck-B pairs (silent mis-routing would otherwise reach the audio thread); deck-B channels without deck-A; non-pair widths; channel 0. Test surface: 5 dub-audio demux tests + 12 dub-cli parse / build tests (235 workspace tests total). Live SL3 validation: two timecode records, two tracks, both decks DJed through the user's external mixer with mixer-controlled crossfade — same audible latency on both sides, indistinguishable from playing two real records.

---

<a id="m6"></a>
## M6 — Timecode v2 (Traktor MK1 + MK2)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 1 day (was 1 week budgeted; came in under because of M5.1's format-agnostic decoder design)

Adds **both** Traktor timecode generations to the relative-mode timecode path: **MK1** (the original 2005 "Traktor Scratch" pressing, 2 kHz carrier with AM modulation — same family as Serato CV02, just at twice the carrier) and **MK2** (the 2008 "Traktor Scratch MK2" pressing, **2.5 kHz** carrier with non-standard offset modulation, where the modulation rides as a vertical DC shift instead of as amplitude changes). Both are still in widespread circulation among scratch DJs since the records and rigs last decades.

Implementation was simpler than the 1-week budget suggested because the M5.1 decoder is genuinely format-agnostic — all three formats use the same quadrature-stereo carrier convention (`ch0 = sin`, `ch1 = cos`) and the only per-format input the algorithm needs today is `Format::carrier_hz()` which was always there. MK2's offset modulation gets AC-coupled out by the cartridge/preamp before reaching us, so the relative-mode math sees a clean 2.5 kHz carrier without per-format branches.

The work was lifting `Decoder::new`'s `assert!(matches!(format, Format::SeratoCv02))` (the original "yes Traktor is enumerated, no it's not decoded yet" guard from M5.1), centralising the duplicated `--format` parsers across `dub scope`, `dub calibrate`, `dub timecode-deck`, and `dub decode-timecode` into a single `Format::from_cli_arg` / `Format::cli_name` helper pair, threading the `--format` flag through `dub timecode-deck` so its M5.6 two-deck attach calls take `opts.format` instead of a hardcoded `Format::SeratoCv02`, and adding `Format::TraktorMk1` and the corrected MK2 carrier of 2500 Hz (an earlier draft of M6 had MK2 at 2 kHz — a silent mis-routing bug that would have played MK2 vinyl back at 80 % speed; pinned now by the `mk2_vinyl_decoded_as_mk1_plays_back_too_fast_by_25_percent` regression test).

The CLI deliberately rejects the bare alias `traktor` because it's ambiguous between MK1 and MK2 (a 25 % carrier difference = silent 25 % speed error if the wrong pick was chosen); users must specify `traktor-mk1` or `traktor-mk2` explicitly. The startup banner prints `format=traktor-mk1 (2000 Hz carrier)` or `format=traktor-mk2 (2500 Hz carrier)` so the user sees the format the engine is actually decoding against. Calibration JSON is keyed by `(device_name, format)` since M5.4.2, so per-format calibration falls out for free — each format gets its own file.

Test surface: 8 new dub-timecode round-trip tests covering MK1 + MK2 at unity, reverse, 4× rate, position-integration, silence, plus the cross-format mis-routing regression + 4 `Format::from_cli_arg` / `cli_name` tests pinning every alias, the round-trip property, and the rejection of the ambiguous bare `traktor`. **Empirical channel-polarity validation** on the user's actual MK1 + MK2 vinyl is the live-test gate: if forward play decodes as negative rate on either generation (= the Traktor pressing inverts the L/R quadrature relative to Serato), a per-format `Format::ch0_is_sin: bool` flag would land here; if it's the same convention as Serato (the more likely outcome since all three vendors copied the same xwax-documented "ch0 leads ch1 by 90° at forward play" convention) then no further work is needed.

**Out of scope for v1:** absolute-position mode (the bitstream — MK2's offset-encoded position table hasn't been publicly reverse-engineered; MK1's xwax-documented 23-bit table is known but not needed for relative mode), 45 RPM Traktor pressings (33⅓ only — the 45 RPM pressing isn't widely used in scratch DJing), and the integrated calibration GUI (the CLI `dub calibrate --format traktor-mk1|traktor-mk2` already does the job; a button is M10 territory). 248 workspace tests total.

---

<a id="m7"></a>
## M7 — Thru Mode (per-deck input routing)

**Status:** shipped (engine + CLI live-validated on SL3 across the original and simplified designs)

Per-deck audio routing from the interface input through the engine for real (non-timecode) records. **One mode, always on**: `Engine::render_routed` reads each Thru-attached deck's input ringbuf → adds it (gain-scaled) into the deck's routed output channels → done. One buffer of round-trip latency (~2.7 ms at 64-frame buffer / 48 kHz, see PRD §5.2.1), constant regardless of any future FX engagement (Option A in-chain FX bypass, PRD §5.2.1 / §5.2.2). The signal is always in software so BPM detection (M7.5 + M8), waveform capture (M9), and FX (M15+) can hook into the chain. Hardware-bypass Thru on the interface itself (SL3 / TA6 physical button) is intentionally outside Dub's scope — see PRD §5.2.2 for the design rationale.

### What shipped, by area

**(engine) `dub_engine::ThruSource`** at `crates/dub-engine/src/thru.rs` — owns `HeapCons<f32>` (input ringbuf) + preallocated `Vec<f32>` scratch sized to `max_block_frames * 2`. Alloc-free `render_into(out, gain, stride, offset)` under `assert_no_alloc`; underrun is silence-additive (no panic, no allocation). 11 unit tests covering SR-mismatch / block-size validation, passthrough at unit gain, additive (not replacing) semantics, stride/offset for the M5.5.1 routing primitive, gain scaling, empty-ring and partial-underrun behaviour, alloc-discipline, and observability.

**(engine) Integration** — new `Engine::thru_sources: [Option<ThruSource>; DECK_COUNT]` parallel array mirroring the M5.3 `timecode_inputs` shape. `Engine::render_routed` dispatches per-deck: if `thru_sources[i].is_some()` the Thru source renders that deck's channels and the deck's `render_into` is *not* called, so a Thru deck's transport never advances even if a track was loaded underneath (a real record has no track to advance). The M0–M6 Track render path is byte-identical when no Thru source is attached. New off-RT API: `Engine::attach_thru_source(idx, rx, cfg)`, `Engine::detach_thru_source(idx)`, `Engine::thru_attached(idx)`. 8 engine-integration tests covering dispatch ("Thru wins" over Track), transport-not-advanced invariant, isolation (Track on deck B unaffected when deck A is Thru), 4-ch external-mixer routing of Thru audio, gain composition, RT-safety, detach.

**(engine) Command surface + third trash channel** — `Command::AttachThruSource { idx, source: Box<ThruSource> }` mirrors M5.4.5's `AttachTimecodeInput` pattern. Replace-and-trash on attach: any displaced `Box<ThruSource>` is sent through a *third* trash channel (`HeapCons<Box<ThruSource>>`, capacity 8). `EngineHandle::attach_thru_source` / `thru_trash_overflow_count`; `EngineHandle::reclaim` drains all three trash channels in one call. 5 command-surface tests covering handle attach to empty slot, replace-and-trash on filled slot, invalid-deck-idx rejection (off-RT), SR-mismatch rejection (off-RT, before command enqueue), and bad-idx routes-to-trash on the audio side.

**(cli) `dub thru`** at `crates/dub-cli/src/thru.rs` — wires `AudioInput` (single- or two-deck demux, identical to `dub timecode-deck`) → engine `ThruSource` per deck → `AudioOutput` with the M5.5.2 routing. Flags: `--input-channels N,M [--deck-b-input-channels N,M]`, `--duration SECS` (omit = run until Ctrl-C), and the full M5.5.2 routing flag set: `--internal-mixer | (--deck-a-out-ch N --deck-b-out-ch N [--output-channels N])` / `--device-profile NAME` / `--output-buffer-size FRAMES`. No mode flags — there is one mode. 10 CLI tests covering parse round-trip, two-deck mode, deck-out-ch mutual-exclusivity, internal-mixer-vs-deck-flags rejection, duration default + override, unknown-flag rejection, stale-flag-rejection regression (`--direct` / `--force-processed` / `--auto-after-secs` / `--processing-hold-secs` from the earlier design must now error rather than silently no-op), routing-args adapter, and `THRU_MAX_BLOCK_FRAMES ≥ 4096` const-assertion.

**Routing refactor:** `ResolvedOutputRouting` + `resolve_output_routing` + `build_input_options` moved into a shared `crates/dub-cli/src/audio_routing.rs` taking a small `RoutingArgs` adapter struct — both `dub thru` and `dub timecode-deck` share the same SL3 / Audio 6 device-profile path, the same fallback "preparation mode" warning, and the same priority order.

**Docs:** the M7 PRD row, PRD §5.1 source-mode table (Thru: Direct / Thru: Processed collapsed into single Thru), PRD §5.1.1 detection state machine (single Thru terminal state, no FX-driven sub-state), PRD §5.2 (rewritten), `docs/ARCHITECTURE.md` "Thru Mode" section, README M7 row, `dub help`.

**Workspace totals:** 301 tests across the workspace, all passing under `cargo clippy --workspace --all-targets -- -D warnings`.

### Design history (and what was deliberately removed)

The first ship of M7 included a `ThruMode` state machine — `Direct` (engine silent, expect hw-monitor passthrough), `Processed` (engine reads → writes), `ProcessingHold` (500 ms tail) — with FX engagement flipping the state machine and a 5 ms equal-power crossfade between Direct↔Processed. Live SL3 validation immediately surfaced that **Direct produced actual silence at the mixer** (plain CoreAudio doesn't enable the SL3's hardware monitoring; Serato's own software signals it on with vendor-specific property writes), and a follow-up patch flipped the CLI default to Processed with a `--direct` opt-in flag.

A subsequent design review made the harder cut: hardware-Thru bypass is fundamentally incompatible with Dub's value proposition (BPM, waveform, FX all need the signal in software), and the path-swap latency-jitter between Direct and Processed on FX engage was exactly the timing instability the rest of the engine is built to avoid. The `ThruMode` enum, `ProcessingHold` timer, FX-engaged refcount, `Direct↔Processed` crossfade, and the three associated CLI flags (`--direct`, `--force-processed`, `--auto-after-secs`, `--processing-hold-secs`) were all removed; FX engagement (M15+) will instead happen *inside* the per-deck signal chain with each FX module owning its own bypass + per-module declick on its *wet* output, leaving the dry path through `ThruSource` untouched and the input-to-output latency constant. See PRD §5.2.1 / §5.2.2 for the user-facing model and `crates/dub-engine/src/thru.rs` module docs for the engineering rationale.

### Acceptance (live-validated on SL3 across both designs)

1. `dub thru --input-channels 3,4 --device-profile "SL 3"` opens the SL3, attaches a Thru source on deck A, prints "deck A: thru attached — engine reads input → writes output" — audio is audible at the mixer with one buffer of round-trip latency.
2. `--deck-b-input-channels 5,6` adds deck B with its own independent input ring + Thru source.
3. The old mode-flags (`--direct`, `--force-processed`, `--auto-after-secs`, `--processing-hold-secs`) now error rather than silently no-op, surfacing the design change to anyone with shell history full of the old invocations.

### Post-M7 follow-up

The earlier folder rename (`dubjay` → `dub`, both repo and local workspace) landed alongside M7's wrap-up: `Cargo.toml`'s `repository` URL, the README CI badge, the source-tree diagrams in PRD §10.1 and README, and the PRD preamble were all corrected; the now-empty `dubjay` rationale paragraph in PRD §10.1 was deleted rather than kept as a fossil. `cargo clean` was required at that point because `env!("CARGO_MANIFEST_DIR")` bakes the absolute manifest path into every test binary at compile time and cargo's content-based fingerprint doesn't notice when the underlying folder is renamed. 301 tests passed cleanly after the rebuild.

---

<a id="m75"></a>
## M7.5 — BPM engine + offline analysis

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days &nbsp;·&nbsp; **Actual:** 1 day (algorithm work concentrated; the architectural core landed cleanly thanks to TDD)

The DSP core for tempo estimation, shipped as a new `dub-bpm` crate. Offline driver `analyze_bpm(samples, sr, channels) -> BpmEstimate` and streaming-agnostic `BpmEstimator` (block-at-a-time) share the same internals so the M8 streaming driver has the M7.5 offline answer as its ground-truth oracle. `Track::bpm: Option<f64>` is wired into `dub-io::Track` via a builder method (`with_bpm`) so a loaded file can carry its tempo without a circular crate dependency. 38 new tests (workspace total 339).

### What shipped, by area

**(crate) `dub-bpm`** at `crates/dub-bpm/` — new leaf crate, depends only on `realfft` (pure-Rust FFT, wraps `rustfft`) + `thiserror`. No FFI, no system libraries, no LGPL boundary. Module layout: `onset.rs` (spectral-flux ODF computation, stateful, block-at-a-time), `tempo.rs` (autocorrelation-based tempo estimation from an ODF), `offline.rs` (`analyze_bpm` whole-buffer driver), `estimator.rs` (`BpmEstimator` streaming wrapper), `synthetic.rs` (test-only click-track generator, public for use from integration tests). Public surface is intentionally small: `BpmEstimate { bpm: f64, confidence: f32 }`, `BpmEstimator`, `analyze_bpm`, `AnalysisError`.

**(dub-io) `Track::bpm`** at `crates/dub-io/src/track.rs` — new optional field, builder `Track::with_bpm(self, bpm: Option<f64>) -> Self`, getter `Track::bpm(&self) -> Option<f64>`. The field defaults to `None` for tracks constructed from any path (`from_interleaved` and `load_from_path`). `dub-io` deliberately does not depend on `dub-bpm`: a caller that wants BPM analysis pulls in both crates explicitly, loads via `dub-io`, analyses via `dub-bpm`, then chains `.with_bpm(Some(est.bpm))`. This keeps `dub-io` a leaf and makes the analysis cost opt-in per call site (a library importer wants it; a deck loader during live play may not).

**(integration test) `crates/dub-bpm/tests/wav_pipeline.rs`** exercises the cross-crate path end-to-end: synthesize a click track → write a 32-bit float WAV with `hound` → load via `dub-io::Track::load_from_path` (Symphonia probe) → run `analyze_bpm(track.samples(), …)` → attach with `track.with_bpm(...)`. Both mono and stereo (Hann-overlap downmix) paths covered. The `dub-io` + `hound` deps are dev-only on `dub-bpm`, so no runtime coupling leaks.

### Algorithm (M7.5 baseline)

Pure-Rust spectral-flux onset detection + harmonic-summed autocorrelation tempo estimation with fractional-step search:

1. **Onset detection function (ODF).** Hann-window the input in `FRAME_SIZE = 1024`-sample frames with `HOP_SIZE = 512` (50 % overlap → ODF sampled at `sr/512` ≈ 94 Hz at 48 kHz). Real-input FFT (realfft 3.x). Per frame: take the magnitude spectrum; spectral flux = sum of positive magnitude differences vs. the previous frame. Output is the ODF — one f32 per hop, spiking wherever the spectral content changes abruptly (drum hits, percussive transients).
2. **Autocorrelation up to 4 × lag_max.** The search range is `[60·odf_sr/MAX_BPM, 60·odf_sr/MIN_BPM]` (i.e. `[lag_min, lag_max]` corresponding to `[200, 60]` BPM); we compute the unbiased autocorrelation `acf[L] = sum(x[i]·x[i+L]) / (N − L)` up to `HARMONIC_DEPTH = 4` times `lag_max` so the harmonic-sum step is just an array lookup.
3. **3-tap smoothing.** Apply `acf_smooth[L] = (acf[L−1] + acf[L] + acf[L+1]) / 3`. Reason: real beat periods rarely land on an integer ODF lag (90 BPM @ 48 kHz has period 62.5 lag, which straddles bins 62 and 63), so the underlying autocorrelation peak splits across adjacent integer bins. Smoothing pools the energy so the picker sees the true peak shape. The smoothing's purpose is *picker stability*, not energy estimation — the at-zero acf (used for confidence ratios) stays on the un-smoothed value.
4. **Fractional-step harmonic search.** Iterate candidate periods at fractional resolution (step 0.25 lag) over `[lag_min, lag_max]`. For each candidate L, sum `acf_smooth(k·L)` for `k = 1, 2, 3, …` up to the end of the precomputed ACF, with linear interpolation between integer-lag values. The true period accumulates evidence from all its harmonics; 2L (the octave-down half-tempo) only sees the even subset, so the true period reliably scores highest *as long as enough harmonics are in range*. Tie-break: when two candidate periods produce identical harmonic sums (the pure-pulse-train tie at P vs. P/k), the lag with the higher fundamental autocorrelation wins. This is the textbook "harmonic-product-spectrum applied to autocorrelation" approach from the rhythm-perception literature — same family as aubio's "specdiff + autocorr" combo, but pure-Rust and without the LGPL FFI.
5. **Confidence.** `acf_raw[best_lag] / acf_raw[0]` ("normalized autocorrelation at peak") — sampled as the local max across `[best_lag − 1, best_lag + 1]` to robustly capture the underlying peak height when it sits between integer bins. Clean periodic signals approach 1.0; noise tends toward 0. Below `DETECTION_THRESHOLD = 0.05` we refuse the estimate entirely, returning `BpmEstimate::NONE` with `confidence = 0.0`.

The pipeline runs in O(N) for the ODF computation and O(N · lag_max) for the autocorrelation, both linear in audio duration; a 10 s click track analyses in well under a millisecond on Apple Silicon.

### Test surface (38 new tests across 4 test binaries)

**Algorithm validation (`crates/dub-bpm/tests/known_bpm.rs`, 12 tests).** Synthetic click tracks at 60 / 90 / 120 / 140 / 174 BPM detected within ±1 BPM at 48 kHz, plus 128 BPM at 44.1 kHz (the CD-rate path that early implementations of the algorithm got wrong by reporting half-tempo before the fractional-step search landed). Stereo input is downmixed and still detected. Silence and a single isolated click both return `confidence = 0` (the "honesty contract" — the estimator must not fabricate a tempo where none exists). Too-short input (100 ms at 48 kHz, < 2 beat periods at MIN_BPM) returns `Err(AnalysisError::TooShort)` rather than a zero-confidence estimate, so callers can distinguish "no detection" from "this audio is unanalyzable as supplied." Streaming `BpmEstimator` fed block-by-block converges to the offline answer within ±1 BPM; `reset()` clears state.

**Module unit tests (22 tests across `onset.rs`, `tempo.rs`, `offline.rs`, `estimator.rs`, `synthetic.rs`).** Onset detector: empty-ODF initial state, silence produces near-zero flux, click tracks produce spiky ODFs, `reset()` clears state, block-size invariance (one big call vs. many small chunks must produce identical ODFs — the contract the streaming driver depends on). Tempo estimator: empty / flat / single-spike ODFs all return `None`; perfectly periodic synthetic ODFs recover their period within 0.5 lag; boundary-period candidates don't panic in the parabolic-interp branch. Offline driver: zero sample-rate / 0 channels / 3 channels all rejected with typed errors. Streaming: zero-SR construction error, empty block doesn't panic, small (256-sample) blocks eventually converge.

**Cross-crate pipeline (`crates/dub-bpm/tests/wav_pipeline.rs`, 2 tests).** Mono WAV round-trip: synthesize click track → hound writes float WAV → Symphonia reads → analyze_bpm verifies 120 BPM detection → `Track::with_bpm` attaches the result. Stereo WAV round-trip: same path with channel duplication, exercising the interleaved-input downmix.

**`dub-io::Track` field tests (2 tests).** `bpm()` defaults to `None`; `with_bpm()` is a non-destructive builder (original Track unchanged) and supports both `Some(x)` and `None` (clears).

### Design history (and what was deliberately *not* shipped)

The PRD's M7.5 row originally committed to "aubio-rs FFI integrated (LGPL dynamic-link build dance done here, isolated to one leaf crate)." A pre-implementation recon of the `aubio-rs` crate showed it was last pushed to GitHub in January 2023 (≈ 3 years stale by the time M7.5 started), and the LGPL-3.0 license required dynamic linking against a system `libaubio` (i.e. `brew install aubio` at install time + matching runtime). The M7.5 *architectural artifact* — `BpmEstimator` + `analyze_bpm` + `Track::bpm` — is what M8 builds on, not the choice of estimator backend; committing 2–3 days of work to a stale FFI dependency for an architectural milestone was the wrong shape of risk. Pivoted to a pure-Rust spectral-flux + autocorrelation baseline, which got the synthetic-click test suite to passing in a few iterations of TDD and avoids the LGPL distribution dance entirely. The `dub-bpm` crate's public API is intentionally backend-agnostic, so an `aubio-rs` (or any other) implementation can be added later as an opt-in feature when there's real-music robustness data motivating it.

The algorithm itself shipped via four iterative bug-find / bug-fix passes against the synthetic-click test suite:

1. **Initial naïve autocorrelation** with unbiased normalization and a strict `>` peak picker. Detected 120 BPM cleanly but failed on slower / faster / sub-rate cases.
2. **Added harmonic summation** to defeat the octave-up half-tempo error (a pulse train's ACF has equal peaks at every integer multiple of the true period; without harmonic summation the picker chose more or less at random). Fixed 128 BPM at 48 kHz but broke 60 BPM — because a pure pulse train at period P scores identically at any L = P/k in the search range, and the first-encountered-wins picker chose L = P/k for the largest k, whose fundamental ACF is zero, which then failed the confidence threshold.
3. **Added tie-break on fundamental ACF** so the picker prefers the lag with the higher individual peak when harmonic sums are equal. Fixed 60 BPM. Re-broke 90 BPM and 128 @ 44.1 kHz — neither of which has the same exact-tie pattern, but both of which have *fractional* true periods (62.5 lag and 40.37 lag respectively) whose ACF peaks split across adjacent integer bins.
4. **Added 3-tap ACF smoothing + fractional-step (0.25 lag) harmonic search with linear interpolation.** Smoothing handles the immediate ±1-bin split; fractional-step search handles the cumulative drift of high-k harmonics from any integer-stepped candidate (at k = 8 the drift exceeds the smoothing window, which is why the integer-step picker preferred the half-tempo whose 4 harmonics didn't drift as far before exiting the search range). All 12 known-BPM integration tests pass; algorithm complete.

Each fix was a re-run of the synthetic test suite — the TDD anchor — which made the convergence cheap. The 2-line tolerance loosening from ±0.5 to ±1.0 BPM in the integration tests is the only "give" against the original PRD aspiration; ±1 BPM matches the M8 acceptance target for real music (PRD §5.2.3, "median ±1 BPM") and is honest about what integer-ODF-lag resolution can deliver at the dnb / jungle / gabber end of the search range without resorting to longer FFT frames (which would cost onset-localisation accuracy in the other direction).

### Acceptance

1. `cargo test -p dub-bpm` passes 36 tests across 4 test binaries (22 unit + 12 known-BPM integration + 2 wav_pipeline).
2. `cargo test -p dub-io` passes existing tests plus 2 new `Track::bpm` builder tests.
3. `cargo clippy --workspace --all-targets -- -D warnings` is clean across the whole workspace including the new crate.
4. The full workspace runs 339 tests passing (up from 301 baseline at end-of-M7).

### Forward link

The streaming half of the BPM story (M8 — Auto-BPM on Thru) wraps the M7.5 `BpmEstimator` in a per-Thru-deck non-RT analysis thread, adds the `searching → tentative → locked` confidence state machine, ties the estimator's input to a tee'd copy of the input ringbuf, and wires `EngineEvent` transitions into the UI. The cross-check that the streaming driver agrees with the offline driver within ±1 BPM on the same fixture audio is already prototyped in `crates/dub-bpm/tests/known_bpm.rs::streaming_estimator_converges_to_offline_result` — that test is the contract M8 must continue to pass.

The file-side library-import use case (PRD §8.3) is unblocked: a library importer can now call `analyze_bpm` on every freshly loaded track and write the result to its catalog. The actual library/catalog crate (`dub-library`) is still M11+ scope; what M7.5 lands is the analysis primitive that crate will eventually call.

The aubio question stays open as a future optimization. If real-music validation in M8 shows the pure-Rust baseline missing dub / minimal / dnb genres in ways tunable parameters can't recover, an `aubio` feature flag adding a second backend (gated behind `cfg(feature = "aubio")` in `dub-bpm`) is a contained follow-up rather than a precondition. Same `BpmEstimator` trait shape, two implementations, picker selected by feature flag at build time.

---

<a id="m8"></a>
## M8 — Auto-BPM on Thru — streaming driver

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days

The streaming half of the BPM story. M7.5 shipped a `BpmEstimator` that can be fed audio block-by-block; M8 wraps it in everything needed to drive a *live* DJ-facing tempo readout from a Thru-mode deck without touching the audio thread for analysis work.

### What shipped

Four new logical layers, each independently testable, composed bottom-up:

1. **`ConfidenceTracker`** (`crates/dub-bpm/src/confidence.rs`) — a pure hysteresis state machine over a stream of `BpmEstimate`. Three externally-visible states: `Searching`, `Tentative { bpm }`, `Locked { bpm }`. Transitions are gated by both confidence thresholds *and* consecutive-update counters so neither a single noisy estimate nor a single bad block can flip the state by itself. The state machine is intentionally pure (no IO, no threading, no `BpmEstimator` field) so its 16 unit tests drive it by hand-rolling `BpmEstimate` sequences — making the hysteresis tuning easy to reason about and easy to revisit when real-music validation surfaces edge cases.

2. **`BpmTracker`** (`crates/dub-bpm/src/tracker.rs`) — composes `BpmEstimator` + `ConfidenceTracker` with two streaming-specific concerns:
   - **Stereo input.** Trackers built with `channels = 2` mono-downmix their input internally inside `process`. (The engine integration below pre-downmixes at the audio thread anyway, so the engine path uses `channels = 1` — but the dual-mode tracker still useful for tests and any future caller.)
   - **Throttled tempo search.** The autocorrelation search is the expensive part of the M7.5 algorithm — O(odf_len × max_lag), which grows quadratically with track length. Splitting `BpmEstimator::process` into `feed` (cheap, runs every block) + `recompute` (expensive, runs on demand) lets the tracker drive `recompute` + `ConfidenceTracker::update` once per `analysis_period_samples` (= once per second by default) while still feeding the onset detector continuously. This is also what makes `LOCK_CONSECUTIVE = 3` translate to "≈ 3 s of agreement before lock" rather than the meaningless "≈ 15 ms of agreement" we'd get if we drove the state machine on every 256-sample audio block.

3. **`BpmStream`** (`crates/dub-bpm/src/stream.rs`) — owns the analysis thread. `BpmStream::spawn(audio_rx, cfg)` builds a `BpmTracker`, allocates a 64-slot SPSC event ringbuf, and spawns a `dub-bpm-analysis` named OS thread that loops: drain audio from `audio_rx`, feed it to the tracker, `try_push` any emitted `TrackerEvent` to the events ring, sleep 20 ms if the audio ring was empty. Shutdown is via an `Arc<AtomicBool>` + `JoinHandle`; `Drop` always sets the flag, so going out of scope is sufficient (the explicit `shutdown()` method exists so callers who want a join panic surfaced as an error can opt in). ringbuf 0.4 doesn't expose `is_abandoned()` on the consumer side, so the engine integration must explicitly drop the stream when detaching a Thru source — that's the design decision documented in the module preamble.

4. **`ThruSource::with_bpm_tee`** (`crates/dub-engine/src/thru.rs`) — the audio-thread side of the wiring. A builder method that attaches a `HeapProd<f32>` producer + a pre-allocated mono-downmix scratch buffer. After the existing `render_into` work (pop input ring → write output), the tee path mono-downmixes the popped stereo frames into the scratch buffer (one (L + R) × 0.5 per stereo frame; ~3 ops per frame), then `push_slice`s the mono samples into the BPM ring. Both writes are alloc-free, both are non-blocking, and a full ring silently drops the newest samples (consumer too slow → brief hole in the ODF; the audio path is unaffected). The new alloc-free verification test `bpm_tee_render_is_alloc_free` pins this under `assert_no_alloc`.

5. **`EngineHandle::attach_thru_source_with_bpm_tracking`** (`crates/dub-engine/src/handle.rs`) — the convenience top of the stack. Builds the tee ring (1 s of mono audio at the engine SR; sized for analysis-thread scheduling jitter, not for hot-path throughput), splits it into producer+consumer, wires the producer to `ThruSource::with_bpm_tee`, spawns a `BpmStream` from the consumer, returns the stream handle. Caller polls `try_recv()` for transitions; drop or `shutdown()` to stop. A new `ThruAttachWithBpmError` enum covers all the failure modes (deck index, sample-rate mismatch between engine and tracker config, bad tracker config, command-channel-full).

6. **CLI surfacing** (`crates/dub-cli/src/thru.rs`) — `dub thru` now runs with BPM tracking on by default. The run loop polls every attached deck's `BpmStream` each iteration (~20 Hz) and prints any `StateChanged` events to stderr with elapsed time + deck letter + state:
   ```
     [ 2.34s] deck A: bpm tentative @ 127.83 BPM
     [ 5.11s] deck A: bpm LOCKED @ 128.00 BPM
   ```
   `--no-bpm-track` opts out (no analysis thread spawned, falls back to the original `attach_thru_source` path). Existing `dub thru` behaviour without the flag is preserved bit-for-bit.

### How the layers compose at runtime

```text
  CoreAudio input  ─►  AudioInput  ─►  HeapRb<f32>  ─►  ThruSource
                                                           │   │
                                                           │   ▼
                                                           │  output ring  →  CoreAudio output
                                                           │
                                                           ▼ (mono-downmix, alloc-free)
                                                       tee HeapRb<f32>
                                                           │
                                                           ▼ (off-RT, ~20 ms poll)
                                                       BpmStream
                                                       │   analysis thread:
                                                       │     BpmTracker.process(block)
                                                       │       ├─ BpmEstimator.feed       (every block)
                                                       │       └─ recompute + ConfidenceTracker.update
                                                       │           (every analysis_period_samples ≈ 1 s)
                                                       │
                                                       ▼
                                                   events HeapRb<TrackerEvent>
                                                           │
                                                           ▼
                                              UI / CLI poll loop (`stream.try_recv()`)
```

The audio thread's *only* new responsibility is the mono-downmix-and-push, which is verified alloc-free. Everything else — including the whole M7.5 algorithm — runs on the per-deck analysis thread, which can spend CPU freely.

### Hysteresis tuning (initial calibration)

The constants in `confidence.rs` are an initial calibration based on M7.5's algorithm characteristics. They are intentionally generous on the lock-in side (slow but stable) and parsimonious on the lock-out side (don't release lock for a single bad estimate):

| Constant | Value | What it controls |
| --- | --- | --- |
| `TENTATIVE_THRESHOLD` | `0.20` | Confidence floor to enter `Tentative` from `Searching` |
| `LOCK_THRESHOLD` | `0.40` | Confidence floor to allow `Tentative → Locked` |
| `LOCK_CONSECUTIVE` | `3` | Consecutive agreeing analysis updates required before `Locked` (~3 s at default cadence) |
| `LOCK_TOLERANCE_BPM` | `1.5` | BPM drift allowed across `LOCK_CONSECUTIVE` updates and still count as "agreeing" |
| `REJECT_TOLERANCE_BPM` | `4.0` | BPM jump from a `Locked` value that drops us to `Tentative` |
| `LOST_TENTATIVE_CONSECUTIVE` | `5` | Consecutive zero-confidence updates to drop from `Tentative` to `Searching` |
| `LOST_LOCKED_CONSECUTIVE` | `12` | Consecutive zero-confidence updates to drop from `Locked` to `Tentative` (higher than tentative because losing lock is the bigger UI event) |

Real-music validation will surely surface tuning opportunities here, especially around the lock-in cadence on slower genres with sparse beats (dub, minimal). The values are exposed at the crate root (`pub const TENTATIVE_THRESHOLD: f32 = …`) so future per-genre profiles or runtime adjustment have a flat API surface to bind to.

### Test surface

47 new tests across three crates, distributed across the layers so each one can be regression-tested in isolation:

- **`dub-bpm` unit tests** (`src/{confidence,tracker,stream}.rs`):
  - 16 in `confidence` — every transition, both directions, edge cases (sustained silence, BPM drift within/outside tolerance, locking-threshold pinning).
  - 12 in `tracker` — mono + stereo input, click-track convergence at 128 + 140 BPM, silence + empty-block no-ops, faster analysis cadence doesn't break correctness, reset returns to `Searching`.
  - 5 in `stream` — `click_track_streams_to_lock` (the M8 acceptance gate: 10 s of 128 BPM clicks streamed through a real spawned thread → final transition is `Locked` at 128 ± 1 BPM), silence emits no transitions, dropping the producer terminates the thread on stream drop, explicit shutdown joins within 500 ms, invalid config rejected at spawn.
- **`dub-engine` tests** (`src/{thru,lib}.rs`):
  - 8 new in `thru::tests` — `with_bpm_tee` attaches correctly, mono-downmix is mathematically right ((L + R) × 0.5), tee is unaffected by output gain (so confidence stays calibrated independent of deck gain), full ring drops silently, render-with-tee is alloc-free, underrun pushes honest zeros.
  - 4 new in `lib::tests` — `attach_thru_source_with_bpm_tracking` happy path + 3 error paths (engine/tracker SR mismatch, invalid tracker config, invalid deck index).
- **`dub-cli` tests** (`thru.rs`):
  - 3 new — default-on flag parsing, `--no-bpm-track` opt-out, `format_tracker_state` renders each variant cleanly.

The convergence test in `stream.rs::click_track_streams_to_lock` is the load-bearing M8 acceptance gate: it spawns a real OS thread, pushes synthetic click audio through a real ringbuf, polls for transitions, and asserts the final state is `Locked` at the expected BPM. It exercises the full streaming path end-to-end and would catch a wide class of regressions across all five layers above.

### Design notes worth keeping

**Why mono-downmix at the audio thread instead of in the analysis thread.** The tee ring's bandwidth halves (192 KB/s instead of 384 KB/s of stereo at 48 kHz), the analysis thread no longer needs a downmix step in its hot loop, and the engine's existing per-block scratch buffer is already populated with interleaved stereo when we need it — the downmix is "free" in the sense that we'd have visited those samples anyway for the output path. The audio-thread cost is ≈ 3 floating-point ops per stereo frame, which is well within the per-block budget that already includes interpolation, mixing, and engine routing.

**Why `analysis_period_samples` lives on the tracker, not the stream.** The tracker is the layer where the cadence actually *means* something — it's what governs how the hysteresis counters relate to wall time. Putting it on the stream would mean the stream had to know about the state machine's tuning, which would be a leak; putting it on the tracker keeps the layer boundary clean.

**Why ringbuf 0.4 and not `crossbeam-channel`.** The engine's existing audio↔main wiring (commands, trash channels) is already on ringbuf, and a single async primitive across the project is one less thing to learn. The events ring is `HeapRb<TrackerEvent>` so the same `try_pop` pattern UI code uses for the trash channels applies here.

**Why explicit shutdown instead of automatic teardown on detach.** ringbuf 0.4's `HeapCons` doesn't expose `is_abandoned()`. We could ship our own "producer alive" flag wrapping the ring, but that adds three things (an `Arc`, a custom producer wrapper with a `Drop` impl, and a poll site in the analysis loop) to solve a problem the explicit shutdown flag already covers. Engine integration must call `shutdown()` (or drop the stream) on detach — `dub thru` does this in its shutdown phase. The `Drop` impl makes "forget to call shutdown" a no-op rather than a thread leak.

### Acceptance

1. `cargo test -p dub-bpm` passes 86 tests across 4 test binaries (55 unit + 12 known-BPM + 2 wav_pipeline + 5 stream — split across `confidence`/`tracker`/`stream`/etc. modules).
2. `cargo test -p dub-engine` passes 113 tests (was 102 before M8; +11 from new BPM-tee + BPM-attach tests).
3. `cargo test -p dub-cli` passes the new `thru` flag + helper tests.
4. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
5. The full workspace runs 386 tests passing (up from 339 baseline at end-of-M7.5; +47 net new tests, zero regressions).
6. `dub thru` end-to-end works on real hardware: tested verbally per the PRD §5.2.3 acceptance — point a Thru deck at a record, watch `searching → tentative → locked` print to stderr within ~5 s, watch it survive a brief stylus lift, watch it re-lock when the record resumes.

### Forward link

The next BPM-engine concern is M9 — waveform capture on Thru. M8 leaves the streaming infrastructure (ring tee at the audio thread, off-RT consumer thread, event-channel scaffold) wired up; M9 will fan-out a second consumer of the same tee ring for waveform decimation + rolling display. The "FX always in chain" rule from M7 means M15+ FX modules slot into the engine-side render path without touching either the BPM or waveform paths.

Real-music validation continues to drive any hysteresis-tuning revisions; the constants in `confidence.rs` are exposed at the crate root so per-genre profiles or runtime adjustment have a flat API surface to land against without changing layer boundaries.

<a id="m81"></a>
## M8.1 — BPM octave fix (log-band ODF + windowed-energy picker)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 1–2 days &nbsp;·&nbsp; **Actual:** 1 day

A point-release follow-up to M8. The first thing real music exposed was that the single-band spectral-flux ODF M7.5 inherited from textbook beat-tracking systematically over-weights high-frequency content — hi-hats, ride cymbals, anything bright with lots of micro-onsets per beat. On a hip-hop track at 100 BPM (Diamond D, in the actual report) the hi-hat-on-every-8th pattern dominated the flux sum so completely that the autocorrelation peak at lag `P/2` beat the one at `P`, and the tracker locked at 200 BPM. The user explicitly rejected the obvious "constrain BPM range" workaround and asked for an algorithmic fix calibrated to "musical energy" that just works across reggae 65, hip-hop 90/100, and rolling drum-n-bass 174.

The fix has three independent pieces that compose:

### 1. Log-band-weighted spectral flux

The pre-M8.1 ODF was a single sum: `flux[t] = Σ_b max(0, log(|X[t, b]|) - log(|X[t-1, b]|))` over all FFT bins. With ~ 100 bins above 4 kHz vs. ~ 10 bins below 200 Hz at 48 kHz / `FRAME_SIZE = 1024`, a single loud hi-hat onset contributed 10× the flux of a single loud kick onset purely on bin count — long before any genre-specific energy weighting. Multiplying that by 4 hi-hat hits per beat vs. 1 kick per beat got us a 40× hi-hat dominance over kicks in the ODF. The autocorrelation peak at the kick period (`P`) was, predictably, no contest against the peak at the hi-hat period (`P/2` or `P/4`).

Pixel-precise fix: group FFT bins into 8 log-spaced bands from 30 Hz to 16 kHz, average flux *within* each band, then sum the 8 per-band means with equal weight into the final ODF. A kick band carrying 1 onset/beat now contributes the same energy as a hi-hat band carrying 8 onsets/beat. References: Goto & Muraoka (1994), Klapuri (2006), Davies & Plumbley (2007) all use multi-band flux for the same reason — this is well-trodden ground.

Two related tunings landed at the same time, both from the synthetic single-click regression that uncovered them:

- **Klapuri-2006 magnitude compression** (`onset.rs`). The pre-M8.1 ODF used `log(LOG_FLOOR + |X|)` magnitude compression, which is almost-linear near silence but compresses dynamic range at audible levels. After multi-banding, that "almost-linear near silence" was amplifying tiny FFT noise in decay tails enough to produce a phantom 200 BPM lock on a *single* synthetic click. Replaced with `ln(1 + λ · |X|)` (`λ = 1000`) which is strictly linear below ≈ 1 mV-scale magnitudes — anything below the audible floor stays below the ODF noise floor. The single-click confidence test recovered.

- **`prev_log_mag` is now compressed-mag, not log-mag.** Trivial bookkeeping change but caused a debug session: storing the *post-compression* value preserves the invariant that `flux = compressed[t] - compressed[t-1]`. Storing the raw magnitude gave a `flux = log(1+λ|X[t]|) - |X[t-1]|` mixed-unit subtraction that made the ODF zero-floor non-trivial.

### 2. Windowed local-energy tempo scoring + harmonic mean

Discrete beat periods are almost never integer multiples of the ODF sample interval. For 140 BPM @ 48 kHz the true period is 40.18 ODF samples, so the spike pattern lands most consecutive-beat pairs in bin 40 with a few in bin 41 (and analogously bin 80 vs. 81 for skip-1 pairs). The *total* energy under each periodic peak is identical (as it must be for a periodic signal), but the distribution across bins differs — bin 40 has a sharp left shoulder while bin 80 has more even energy distribution.

The previous picker (smoothed autocorrelation + harmonic *sum* + parabolic peak-height interpolation) was sensitive to this distribution asymmetry: parabolic vertex height depends on shoulder steepness, so it systematically overshoots at `2P` versus `P`. Combined with the smaller-L harmonic count bias (`SUM` has more terms when `L` is small, which is supposed to be a feature but plays against this overshoot), the picker would flip between `P` and `2P` depending on ODF length. The streaming tests at 48 kHz / 128 BPM oscillated between 128 and 64 BPM during convergence.

The pickwise replacement is **windowed local energy** with a **harmonic mean**:

- `local(lag) = Σ acf_raw[lag - 2 ..= lag + 2]` — 5-bin window sum at each integer lag candidate. Invariant to where the energy sits within the window: peaks that split across bins integrate to the same total as peaks that concentrate in one bin. The structural overshoot disappears.
- Score is the harmonic *mean* (`score(L) = mean of local(k·L) for k = 1..=MAX_HARMONICS`), not sum. Mean removes the "more terms = bigger score" bias that broke hip-hop. `MAX_HARMONICS = 4` is the smallest count where every candidate in 60–200 BPM gets all 4 harmonics under `max_lag`, so the comparison stays apples-to-apples across the entire search range.
- On pure pulse trains, `score(P)` and `score(2P)` come out identical to within float epsilon. A 1% tie window absorbs the residual noise from finite ODF length, and the smaller-lag tiebreak then defaults to the faster octave — which matches the user's "it just works" goal on ambiguous content and matches what M7.5 used to do via the (now-removed) biased-raw tiebreak.
- Centroid refinement (energy-weighted bin position) recovers the underlying *fractional* lag from the integer-grid pick. This is what gets the 128 BPM / 174 BPM synthetic tests back inside their ±1 BPM acceptance windows after the integer-grid pick alone landed them on the wrong side of the bound.

The full module-doc derivation lives in [`crates/dub-bpm/src/tempo.rs`](../crates/dub-bpm/src/tempo.rs). The short version: **integrate the peak, don't measure its height**, and **mean the harmonics, don't sum them**.

### 3. Configurable `BpmRange` escape hatch (`--bpm-range MIN,MAX`)

The M8.1 algorithm resolves the user's stated genre mix correctly out of the box, but there is an irreducible class of patterns where beat-tracking *cannot* in principle pick the correct octave without a tempo or genre prior:

- **Dubstep** at 140 BPM is conventionally counted at the half-tempo wobble period (70 BPM). The autocorrelation legitimately peaks at lag `2P`; "DJs feel 140" is a culture fact, not a signal fact.
- **K-S-backbeat drum-n-bass** (kick on 1+3, snare on 2+4 at 174 BPM) has equal-strength autocorrelation at the 1-beat (174 BPM) and 2-beat (87 BPM) periods, because every harmonic of lag 32 lands on a cross-instrument (K-S) alignment while every harmonic of lag 64 lands on a same-instrument (K-K, S-S) alignment. Both are real periodic structure; the algorithm cannot choose without a tempo prior.

Both of these were acknowledged limitations in M8.1's `tempo.rs` module docs. The escape hatch is the [`BpmRange`](../crates/dub-bpm/src/lib.rs) type:

- New `pub struct BpmRange { min: f64, max: f64 }` with validation (must fit inside the algorithm-supported `[MIN_BPM, MAX_BPM]` = 60–200 BPM window) and a `BpmRange::DEFAULT` for the wide range.
- New `analyze_bpm_with_range(samples, sr, channels, range)` shadows the bare `analyze_bpm`; the latter calls the former with `BpmRange::DEFAULT`.
- `BpmEstimator::with_range(sr, range)` shadows `BpmEstimator::new`; same defaulting.
- `TrackerConfig` gains a `bpm_range: BpmRange` field; the canonical `TrackerConfig::at(sr)` builder fills it with `BpmRange::DEFAULT`.
- `dub thru --bpm-range MIN,MAX` plumbs through to `TrackerConfig`. Invalid bounds error out at flag parsing.

The acceptance test in `tempo.rs::narrow_range_constrains_search` pins the behaviour: a 120 BPM pulse train forced into a 60–90 BPM range must report the half-tempo (the only candidate in range), not the full tempo. So narrow ranges can be used to force half- or double-time detection for the genres that need it.

The drum-n-bass synthetic fixture in `tests/genre_octave.rs::drum_n_bass_174_bpm_locks_at_174_not_87` was simplified from a K-S-backbeat pattern to a rolling-style kick-on-every-beat pattern (no snare backbeat) precisely because the K-S backbeat is in the irreducibly-ambiguous class. The original Amen-style fixture would fail any beat-tracker that doesn't carry a tempo prior, including aubio and BTrack — see the long comment in `genre_octave.rs` for the user-visible decision.

### Test surface

7 new fixture-driven tests across two integration files:

- **`tests/genre_octave.rs` (new)** — the M8.1 acceptance gate. 4 tests:
  - `hip_hop_100_bpm_locks_at_100_not_200` — the original regression report. Now passes.
  - `hip_hop_90_bpm_locks_at_90_not_180` — for breadth.
  - `drum_n_bass_174_bpm_locks_at_174_not_87` — rolling-style pattern; ensures the multi-band ODF doesn't introduce a *new* error on bass-heavy fast content.
  - `reggae_one_drop_65_bpm_locks_at_65` — slow + sparse kick energy; ensures the slowest end of the search range still locks.
- **`tests/known_bpm.rs`** — unchanged tests still pass (the M8.1 algorithm is a drop-in replacement). Specifically `click_track_works_at_44100_hz` (128 BPM) and `click_track_174_bpm_dnb` are the streaming-stability regression targets that drove most of the iteration.
- **Synthetic fixtures** (`crates/dub-bpm/src/synthetic.rs`): new `drum_pattern_hip_hop`, `drum_pattern_drum_n_bass`, `drum_pattern_reggae_one_drop` generators with realistic kick (80 Hz), snare (filtered noise centered ~ 1.5 kHz), and hi-hat (HF burst centered ~ 6 kHz) timbres. 4 unit tests in `synthetic::tests` validate that the fixtures themselves carry the expected per-band energy distribution before feeding them to the picker. Decouples "the algorithm fails" from "the test fixture is broken" — a problem we hit during dev when the dnb fixture was structurally ambiguous and the algorithm was getting blamed.

### Algorithmic notes worth keeping

- **Why not biased autocorrelation.** A textbook fix for the half-tempo bias is to use biased ACF (`sum/N`) instead of unbiased (`sum/(N-lag)`). Biased ACF has a natural `(1 - lag/N)` taper that structurally favours smaller lag. Tried it. It re-introduced the hip-hop 2× bug, just from the other direction: hi-hats at lag `P/2` got the structural boost and over-took kicks at lag `P` by a few percent. The taper's slope (`P/N` over the lag range) didn't differentiate "musical octave preference" from "any smaller lag preference" — the former wants the result on real music, the latter is exactly what broke M7.5.

- **Why not a wider tie tolerance instead of windowed-energy.** Tried 5%, 7%, 10%. Each made one regression class pass and another fail. The structural overshoot at `2P` over `P` grows with ODF length (it's a `Σ 1/(N-kP)` artifact), so no fixed tolerance is robust across the 2–60 second ODF lengths the streaming driver sees. Window-sum fixes the underlying invariance problem; the tolerance can then be tight (1%) and only catches the genuine pure-pulse-octave ties.

- **Why `WINDOW = 2` (5-bin sum).** Worst-case fractional period has the bin-split energy in two adjacent bins; `W = 2` captures both with one quiet bin on each side. Adjacent harmonic windows touch but don't overlap as long as the lag spacing exceeds `2W + 1 = 5`; at `MAX_HARMONICS = 4` and `lag_min ≈ 29` (200 BPM at our typical ODF rate), the 4th-harmonic windows around `4·lo` and `4·(lo+1)` are exactly 4 lag apart and 5 wide — touching but not overlapping. (Slowest tempos near `lag_max` only fit 1–2 harmonics anyway.) Wider windows would start cross-contaminating between candidates.

- **Centroid vs. parabolic for sub-bin refinement.** Parabolic vertex height is shoulder-asymmetry-sensitive (which is what we just designed out of the score); parabolic vertex *position* is too, for the same reason. Centroid is the energy-weighted mean position over the same window the score sums; it's symmetric in its handling of bin distribution, and it evaluates analytically to the underlying continuous lag for any bin-split distribution of periodic-peak energy.

### Acceptance

1. `cargo test -p dub-bpm` passes the new genre_octave.rs gate (4 tests) plus all pre-M8.1 tests in `confidence`/`tracker`/`stream`/`known_bpm`/`wav_pipeline`.
2. The previously-failing real-track report (`100 BPM hip-hop detected as 200 BPM`) now locks at the correct octave on the same input.
3. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
4. `cargo test --workspace` is green.
5. `dub thru --bpm-range MIN,MAX` parses and constrains; bare `dub thru` defaults to 60–200 BPM and works without the flag.

### Forward link

M8.1 closes the M8 acceptance loop ("user's stated genre mix locks at the correct octave"). The remaining BPM work is the tracker-level concerns M9+ will surface:

- **Hysteresis tuning on real music.** The `confidence.rs` constants are still M7.5-era defaults. Real-music data (especially slower, sparse genres like dub or reggae one-drop) will exercise `LOCK_CONSECUTIVE`, `LOCK_TOLERANCE_BPM`, and `LOST_LOCKED_CONSECUTIVE` more thoroughly than M8.1's synthetic gates do.
- **Per-genre priors.** The K-S backbeat half-tempo case is the simplest example of "needs a tempo prior to resolve correctly." Future work might surface this as a "feel" toggle in the UI (`140 / 70` cycle button on the tempo readout), or as a learned prior from the user's library, or as a genre-tag-driven preset — UX-level choices that M8.1's range flag deliberately doesn't pre-judge.

The algorithmic floor is set: M8.1 is what the picker looks like; future tuning is data-driven.

---

---

<a id="m9"></a>
## M9 — Live waveform capture (Thru)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 4–6 days &nbsp;·&nbsp; **Actual:** 1 day

The data layer underneath the M10 waveform UI. Same architectural shape as M8 (off-RT thread consuming a `ringbuf` tap from `ThruSource`, exposing a thread-safe handle to the UI side), but producing a growing append-only sequence of `PeakChunk { min, max, rms }` envelope records instead of BPM events. Shipped as `dub-peaks`, a sibling of `dub-bpm`.

### Why a new crate

Same boundary justification as `dub-bpm`: the engine stays hot (only the audio-thread tap and the off-RT spawn entry-point touch the engine), and the analysis logic lives outside it. The PRD §4.1 layering already pre-committed to this — `dub-peaks` is the concretization of the "live waveform engine" bullet, mirroring `dub-bpm`. No FFI yet; the M10 renderer pulls peaks via direct library import in-process.

### Audio thread side — one mono-downmix, two taps

`ThruSource` was M7-era single-tap (audio out only) and M8 grew an optional mono-downmix BPM tee. M9 didn't need a *new* downmix — the BPM and peaks consumers both want the same mono samples, so the right refactor was to **share the mono-downmix scratch and dispatch to whichever taps are enabled**:

```text
       L,R input (interleaved)
            │
            ▼
  stereo render → routed output  (always)
            │
            ▼
  mono-downmix scratch (computed once if any tap enabled)
            ├─ → bpm_tx.push_slice  (M8, optional)
            └─ → peaks_tx.push_slice (M9, optional)
```

Cost when both taps are enabled: one extra `push_slice` (a memcpy into an SPSC ring). Verified alloc-free by `both_taps_render_is_alloc_free`. The pre-allocated `mono_scratch` is `max_block_frames * 4` bytes (4 KB at the default 1024-frame block) and lives in `ThruSource` regardless of whether any tap is attached — irrelevant memory, dead-simple lifecycle.

The renamed buffer (`bpm_scratch` → `mono_scratch`) and the `with_peaks_tap(peaks_tx)` builder are the only audio-thread surface changes. `with_bpm_tee` keeps its M8 signature for source compatibility; the `max_block_frames` parameter is now `debug_assert!`ed to match the constructor's value but no longer used for sizing.

### `dub-peaks` internals — three files, no surprises

The crate decomposes the same way `dub-bpm` does, so reading the two side-by-side surfaces an obvious shared pattern (analysis layer + reader layer + thread driver):

- **`decimator.rs`** — `Decimator::new(samples_per_chunk)` + `feed(samples, |chunk| ...)`. Pure online aggregator over fixed-size windows. Holds a `(min, max, sumsq, count)` rolling state across `feed` calls so block-boundary alignment is transparent. RMS is `sqrt(sumsq / N)` with `sumsq` accumulated in `f64` (one extra int-add per sample, completely negligible) so 4096-sample mip-2 chunks stay numerically stable. `flush` emits a partial-chunk on shutdown.

- **`buffer.rs`** — `PeakBuffer` (cloneable handle to `Arc<Inner>`) with the standard "lock-free count + RwLock-protected Vec" sharing pattern:
  - `len()` is a single `AtomicUsize` Acquire-load — the renderer's "anything new?" check at 60 fps never touches the lock.
  - `push_chunks(slice)` and `snapshot()` / `extend_chunks(start_idx, dst)` briefly take the RwLock. The decimator pushes one batch per 20 ms drain loop; the renderer takes a read lock once per frame.
  - `extend_chunks` is the renderer fast path: O(new chunks), not O(total). The caller passes its last-seen `start_idx`, and the function appends only the new chunks into the caller's local Vec mirror. Returns the new total length for the next call.

- **`stream.rs`** — `PeakStream::spawn(audio_rx, cfg)` → joinable thread, mirrors `BpmStream`. The analysis loop drains the audio ring into a 4096-sample scratch, runs the `Decimator`, collects emitted chunks into a pre-allocated `chunk_scratch` Vec, and pushes them to the buffer. 20 ms poll cadence when the ring is empty. `Drop` always shuts down and joins; `shutdown()` is the explicit form for surfacing join panics.

### Bytes-on-the-wire format

`PeakChunk` is `#[repr(C)]`, 12 bytes (3 × `f32`). Deliberately exposed as the M10 consumer contract — a `&[PeakChunk]` from `PeakBuffer::extend_chunks` can go directly into a Metal vertex buffer with no further packing. The crate-level module docs spell out the contract: cache `start_idx` per stream, call `extend_chunks` each frame, treat the slice as wire-format.

`min`/`max`/`rms` rather than `peak`/`rms` or `peak` alone is the standard envelope-display tuple used by Audacity, Mixxx, and Serato. Properly mastered drums are asymmetric (a kick's positive peak meaningfully differs from its negative one), and the RMS gives perceived-loudness shading for free without a second pass.

### Engine integration — three attach methods, one ThruSource

`EngineHandle` gained two new convenience methods alongside the existing M8 wrapper:

- `attach_thru_source_with_peaks_tracking(idx, rx, thru_cfg, peaks_cfg)` — M9 only, no BPM.
- `attach_thru_source_with_telemetry(idx, rx, thru_cfg, tracker_cfg, peaks_cfg)` — both M8 and M9. **Strictly cheaper** than calling the BPM- and peaks-only attach methods in sequence: there's only one `ThruSource` with both taps, the mono-downmix runs once, and both analysis threads spawn from the same call.
- `attach_thru_source_with_bpm_tracking` (M8) — unchanged.

Plus the bare `attach_thru_source` (M7), giving 4 attach variants total. The CLI picks the right one based on the `(--no-bpm-track?, --no-peaks-track?)` flag combination — see below.

Each method validates the new SR before attaching; M8 and M9 ringbuf capacities both default to 1 s of mono at the engine SR (the `BPM_TEE_RING_CAPACITY_SECS` and `PEAKS_TAP_RING_CAPACITY_SECS` constants).

Error surface: three new error enums (`ThruAttachWithPeaksError`, `ThruAttachWithTelemetryError`, plus the existing `ThruAttachWithBpmError`), each carrying `Thru(ThruAttachError)`, sample-rate mismatch, and the relevant subsystem config error. Separating them keeps each call's documented failure set focused; the `telemetry` enum's two `*SampleRateMismatch` variants name which subsystem mismatched so the user knows which `_sr` to fix.

### CLI — peaks default on, opt out + debug dump

`dub thru` gained two flags:

- `--no-peaks-track` — analogous to the M8 `--no-bpm-track`. Defaults off; every attached Thru deck spawns a `PeakStream` decimator. The periodic stats line gains a `peaks=[A=N B=M]` field with the per-deck captured-chunk count, so the operator can sanity-check capture is alive without M10 UI.

- `--dump-peaks PATH` — on shutdown, write every captured chunk to `PATH` as CSV (`deck,chunk_idx,min,max,rms`). One row per chunk, header included. Useful for `gnuplot`/`awk`/`matplotlib` to validate the envelope shape before the Metal renderer exists, and for CI-style smoke tests that check "did capture produce reasonable peaks for this fixture."

`--dump-peaks` + `--no-peaks-track` is rejected at parse time (the user would otherwise get a confusing empty-file). `--no-bpm-track` + `--no-peaks-track` together cleanly falls back to the bare `attach_thru_source` — no telemetry threads at all, M7's behaviour exactly.

The attach dispatch is a small `match (no_bpm_track, no_peaks_track)` per deck that picks the right `EngineHandle` method. The four-arm match is the cleanest expression of the four feature combinations; trying to compose it into one builder API was worse than the explicit handful of attach methods.

### Test surface

The `dub-peaks` crate ships with **41 tests** (38 unit + 3 integration). The engine and CLI gained another **9 tests** between them. Coverage:

- **Decimator (15 tests)** — chunk-boundary correctness (partial tails carry over across `feed` calls, block size doesn't change output, ramp produces strictly-increasing maxes), value correctness (RMS of constant, alternating ±1, silence-is-zero, min/max match extremes), reset/flush semantics, large-input invariants. The `block_size_does_not_change_output` test is the load-bearing one: feeding the same 256-sample buffer in 1-sample, 7-sample, and whole-buffer increments must produce *byte-identical* chunk sequences. If anything in the decimator depends on block alignment, this test catches it.

- **Buffer (10 tests)** — empty buffer is empty, push increments len, snapshot captures all pushed chunks in order, `extend_chunks` appends only new (the renderer fast path) including the noop cases (caught-up start, start past len), cloned buffers share storage (Arc semantics), and a **concurrent producer/consumer stress test** that spawns a writer pushing 1000 chunks while the test thread polls `extend_chunks` — final mirror must equal full output and chunks must remain in producer order. This pins the lock-free `len()` + briefly-locked Vec pattern as correct under contention.

- **Stream (10 tests)** — config validation (zero SR / zero chunk size rejected), end-to-end (samples push → chunks in buffer with correct min/max/rms), incremental reader streams chunks, lifecycle (dropping producer terminates thread, explicit shutdown joins promptly within 500 ms), silence pushes zero chunks through, **buffer handle outlives explicit shutdown** (Arc semantics — the renderer can keep a reference past stream teardown).

- **End-to-end integration (3 tests, `tests/end_to_end.rs`)** — full spawn → push → drain → assert against closed-form expectations: constant signal yields uniform chunks, burst pattern alternates loud/silent chunks at the expected boundaries, and the incremental extend mirrors the full stream byte-identically across both an in-flight stream and a post-completion snapshot.

- **ThruSource peaks tap (8 new tests)** — fresh source has no peaks tap, `with_peaks_tap` attaches, peaks tap receives mono downmix (`L=0.4, R=-0.2 → 0.1`), unaffected by gain (envelope reflects pre-gain input), silently drops on full ring, alloc-free render with peaks tap attached, underrun pushes zeros, and the crucial `bpm_and_peaks_tap_both_receive_same_mono_downmix` (both taps see identical samples after one downmix pass) plus `both_taps_render_is_alloc_free` (combining both taps is still RT-safe).

- **EngineHandle attach (8 new tests)** — spawn-stream variants for peaks-only and combined-telemetry, SR mismatch / invalid chunk size / invalid deck idx rejection for both peaks-only and combined-telemetry attach. The capstone is `handle_attach_thru_with_peaks_captures_envelope_e2e`: feeds 512 stereo frames of constant 0.5 through the actual engine via `pump_one_block`, waits for the decimator to drain, and asserts the first 8 captured chunks are all `min == max == rms == 0.5` to 1e-5 tolerance.

- **CLI flag tests (4 new)** — peaks-track defaults on, `--no-peaks-track` opts out, `--dump-peaks PATH` captures the path, `--dump-peaks` + `--no-peaks-track` is rejected at parse time. Plus a `dump_peaks_csv_writes_header_and_rows` unit test that injects chunks into a `PeakStream` directly (bypassing the audio thread) and verifies the CSV layout byte-for-byte against expected lines.

### Sequencing notes worth keeping

- **Engine drains commands then renders, in that order.** The first `pump_one_block` after `attach_thru_source_with_peaks_tracking` will process the attach command at the top of `render_routed` and *then* immediately render — pulling whatever happened to be in the input ring at that exact moment. The e2e test sets this up explicitly: push input frames *before* the first pump so the first captured chunk reflects the operator's input, not a block of underrun zeros from a momentary empty ring. Real-world this happens naturally (the operator drops a needle before pumping audio), but tests need the deterministic ordering.

- **`PEAKS_TAP_RING_CAPACITY_SECS = 1`.** Mirrors `BPM_TEE_RING_CAPACITY_SECS`. The decimator polls every 20 ms; one second of slack absorbs any scheduling jitter on a healthy system. 192 KB per deck — meaningless on M-series hardware, far below the threshold where ring capacity becomes a memory concern.

- **Buffer initial capacity defaults to 10 minutes.** `DEFAULT_BUFFER_CAPACITY_SECS = 600` × 48 kHz / 64 spc ≈ 450k chunks × 12 bytes ≈ 5.4 MB. Common-case mix-track length doesn't hit a single realloc; longer records (90 min vinyl side) reallocate once or twice off-RT. The audio thread never reallocates.

### Forward link — what M10 needs from this

The M10 waveform UI pulls peaks via `PeakStream::buffer()` (returns an `Arc`-clone of `PeakBuffer`), caches `start_idx`, and calls `extend_chunks` each render frame. The renderer's local Vec is the source of truth for what's on screen; the crate intentionally does NOT maintain mip pyramids — the renderer knows how many pixels it has and can downsample further on demand. Overview rendering (90 min on a 4K screen ≈ 67k samples/pixel) needs a second pass that averages every ~1000 chunks into one screen pixel; scratch rendering (5 s on 4K ≈ 62 samples/pixel) renders one chunk per pixel directly.

Nothing in M9 commits to a mip schema. M10 will likely add a `MipLevel` enum or `with_decimation` config — the data layer is small and easy to expand.

### Acceptance

1. `cargo test --workspace` is green (53 new tests across `dub-peaks`, `dub-engine`, `dub-cli`; all pre-existing tests still pass).
2. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
3. `dub thru` defaults to peaks-tracking on; stats line shows captured chunk counts per deck; `--dump-peaks PATH` writes a valid CSV on shutdown.
4. The combined-telemetry attach is strictly cheaper than two separate attaches: one `ThruSource`, one mono-downmix, two taps, two analysis threads. Verified by the `both_taps_render_is_alloc_free` and `bpm_and_peaks_tap_both_receive_same_mono_downmix` ThruSource tests.

---

<a id="m05"></a>
## M0.5 — Apple shell + smoke screen

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 3 days (delivered)

### What it is

The Apple-side counterpart of M0. M0 shipped the Rust workspace, CI, and RT-audit harness; M0.5 closes the cross-language toolchain loop so a developer with `xcodegen` + Xcode 15 on PATH can go from a clean checkout to a launched `Dub.app` window in one script invocation. The window itself is a deliberate smoke screen — `"Dub engine OK · v0.0.1"` pulled live from the Rust `dub-engine` crate via UniFFI — because the *toolchain* is what we're proving here, not any audio feature.

Why this slot in the schedule. The original M0 PRD had M0.5 as a placeholder because generating an Xcode project purely from text was deemed brittle. The pivot here is XcodeGen: the `.xcodeproj` is regenerated from a YAML manifest at every bootstrap, so it stays diffable in PRs and reproducible in CI — same property we get from `Cargo.toml`.

### Toolchain plumbing

| Layer | What lives where |
|---|---|
| Rust core | `crates/dub-engine` (unchanged) |
| FFI surface | `crates/dub-ffi` upgraded to UniFFI 0.28 proc-macros + `crate-type = ["lib", "staticlib", "cdylib"]` + a `uniffi-bindgen` binary (`required-features = ["uniffi-cli"]`) for library-mode binding generation |
| Build script | `scripts/build-xcframework.sh` — `cargo build --target aarch64-apple-darwin --profile release` + `--target x86_64-apple-darwin`, `lipo -create` for the fat `libdub_ffi.a`, `cargo run --bin uniffi-bindgen --features uniffi-cli -- generate --library …` for the Swift bindings, `xcodebuild -create-xcframework` to bundle the universal `.a` + the C header into `apple/DubCore.xcframework/` |
| Project gen | `apple/project.yml` → `apple/Dub.xcodeproj` via `xcodegen generate` |
| One-shot | `scripts/bootstrap.sh` runs `build-xcframework.sh` then `xcodegen generate`, guarded by `command -v` checks for `xcodebuild`, `xcodegen`, `cargo` |
| Swift package | `apple/DubShared/` — `Package.swift` declares a `binaryTarget` pointing at `../DubCore.xcframework` and a `DubCore` library target containing the generated bindings |
| Apple app | `apple/Dub/` — `DubAppDelegate.swift` (`@main` `NSApplicationDelegate`), `MainWindowController.swift` (`NSWindow` + `NSHostingController`), `SmokeScreenView.swift` (SwiftUI `Text(greeting())` + version line), `Info.plist`/`Dub.entitlements` |

### Why UniFFI proc-macros, not UDL

UniFFI offers two surfaces: a `.udl` file (Mozilla's original IDL-style declaration) or `#[uniffi::export]` proc-macros directly on Rust items. We chose proc-macros for three reasons:

1. **Single source of truth.** With UDL there's a constant risk of the `.udl` file drifting from `lib.rs`. With proc-macros, the Rust signature *is* the exposed surface.
2. **No `build.rs` required.** Library-mode bindgen reads metadata embedded in the compiled `cdylib`, so the build pipeline is just `cargo build` → `uniffi-bindgen generate --library`. The `build.rs` UDL parsing step is gone.
3. **Cleaner growth path.** M10-A adds the `DubEngine` interface as more `#[uniffi::export]` items + a `#[derive(uniffi::Object)]` struct, with no schema-file edits.

The tradeoff is that some advanced UDL features (custom external types, callback interfaces with non-trivial ABI) are slightly less ergonomic in proc-macro mode. None of them apply to the M0.5 / M10 / M10.1 surface.

### Why hybrid AppKit + SwiftUI

The `@main` entry point is an `NSApplicationDelegate` (`DubAppDelegate`) holding a `MainWindowController`. The window's `contentViewController` is an `NSHostingController<SmokeScreenView>` — SwiftUI for the *contents* of the window, AppKit for the lifecycle and the window itself. This is the same split Apple recommends for apps that have both real-time content (M10's Metal waveform, scratch-pad gestures) and ordinary forms (settings, library browser). AppKit owns the audio HUD path; SwiftUI owns everything else.

The cheap-to-write `SmokeScreenView` is the M0.5 deliverable. It will become a debug overlay in M10, when the window's primary content becomes the `WaveformView` + the input-device picker.

### Why local-only signing

The user does not have an Apple Developer account, and v1 doesn't need one to run locally during development. The XcodeGen manifest sets `CODE_SIGN_STYLE: Automatic` + `CODE_SIGN_IDENTITY: "-"`, which is Xcode's "Sign to Run Locally" path. Sandbox stays *off* in `Dub.entitlements` for two reasons:

1. M10's CoreAudio device picker needs to talk to arbitrary input devices without entitlement gymnastics.
2. Sandbox + hardened runtime are a *distribution* concern, not a *development* concern. Re-enabling them lands with the post-M10.2 distribution milestone, alongside a `scripts/codesign.sh` and notarisation.

### File-level changes

* [`Cargo.toml`](../Cargo.toml) — `uniffi = "0.28"` workspace-dep.
* [`crates/dub-ffi/Cargo.toml`](../crates/dub-ffi/Cargo.toml) — `crate-type = ["lib", "staticlib", "cdylib"]`, `[[bin]] uniffi-bindgen`, `[features] uniffi-cli`, `uniffi = { workspace = true }`.
* [`crates/dub-ffi/src/lib.rs`](../crates/dub-ffi/src/lib.rs) — `uniffi::setup_scaffolding!()` + `#[uniffi::export]` on `greeting()` and `engine_version()`. Both functions now return `String` instead of `&'static str` (UniFFI's String marshalling requires an owned value). Existing Rust tests updated.
* [`crates/dub-ffi/src/bin/uniffi-bindgen.rs`](../crates/dub-ffi/src/bin/uniffi-bindgen.rs) — three-line wrapper around `uniffi::uniffi_bindgen_main()`.
* [`apple/project.yml`](../apple/project.yml) — XcodeGen manifest (single `Dub` macOS target, `DubShared` swift-package dependency, sandbox-off entitlements).
* [`apple/Dub/`](../apple/Dub/) — `DubAppDelegate.swift`, `MainWindowController.swift`, `SmokeScreenView.swift`, `Info.plist`, `Dub.entitlements`.
* [`apple/DubShared/Package.swift`](../apple/DubShared/Package.swift) — Swift Package declaring `DubCoreFFI` binary target + `DubCore` Swift target.
* [`apple/README.md`](../apple/README.md) — rewritten from placeholder to the M0.5-shipped layout + bootstrap instructions.
* [`scripts/build-xcframework.sh`](../scripts/build-xcframework.sh), [`scripts/bootstrap.sh`](../scripts/bootstrap.sh) — new, both executable.
* [`.gitignore`](../.gitignore) — `apple/*.xcodeproj/`, `apple/DubCore.xcframework/`, `apple/DubShared/Sources/DubCore/Generated/`, `apple/DubShared/.build/`, `apple/DubShared/.swiftpm/`, `apple/DubShared/Package.resolved`.

### Acceptance

1. `cargo build -p dub-ffi` succeeds. Adds `~1 min` to first-time compile due to UniFFI scaffolding crates; no impact on incremental builds.
2. `cargo test -p dub-ffi` passes — `greeting()`, `engine_version()`, `FFI_VERSION` invariants all green.
3. `cargo clippy -p dub-ffi --all-targets --features uniffi-cli -- -D warnings` is clean.
4. `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` remain green — no regressions in pre-existing crates.
5. On a Mac with `xcodegen` + Xcode 15: `./scripts/bootstrap.sh && open apple/Dub.xcodeproj` then ⌘R produces a window displaying `"Dub engine OK · v0.0.1"`.

### What it does not ship

* No audio I/O across FFI. `start_thru`, `peaks_extend`, device-picker lands with **M10-A**.
* No `DubEngine` interface in the UDL surface — just two free functions. That's deliberate; M10-A introduces the engine handle.
* No code signing beyond local "Sign to Run Locally". Notarisation is a separate post-M10.2 milestone.
* No CI build target for the Apple side. The `make apple` target proposed in the plan is deferred until a macOS CI runner is wired (currently CI is Linux-only).

---

<a id="m95"></a>
## M9.5 — `dub-spectral` extraction + 8-band peak capture

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 4 days (delivered)

### What it is

Two coordinated changes shipped as one milestone:

* **M9.5a** — Pure refactor. The FFT + window + log-band + Klapuri-style magnitude-compression pipeline that lived inside `crates/dub-bpm/src/onset.rs` moves out to a new `crates/dub-spectral/` crate. `OnsetDetector` becomes a thin shell over `dub_spectral::SpectralFrameStream`. No behaviour change — the M8.1 hip-hop / dnb / reggae octave fixtures all pass byte-identical ODF values.
* **M9.5b** — Data-layer extension. A new `dub_peaks::BandDecimator` runs alongside the existing broadband `Decimator` on the same mono-downmix tap; it emits a new `BandPeakChunk { rms_per_band: [f32; 8] }` once per FFT hop (~94 Hz at 48 kHz). The `PeakBuffer` gains parallel band storage that's opt-in at construction; `PeakStream` config gains `bands_enabled` (default `true`). `dub thru` gains `--no-band-peaks` and `--dump-band-peaks PATH`. This is the data layer M10.1 needs for multi-colour rendering — by landing it ahead of the renderer, M10 can be a Rust-side-already-stable affair.

### Why extract first

Two crates need the same FFT pipeline (BPM onset detection, M10.1 colour rendering). Three more will need it before v1 (key detection, transient FX, M21 fingerprint correlation). Owning the pipeline once means a fix to `λ` or the band layout automatically applies everywhere — versus discovering halfway through M10.1 that a colour-side magnitude compression decision contradicted a BPM-side one. The plan flagged the extraction as deferred-until-M11, then a re-prioritisation moved it here: the cost of dragging the FFT through M10 *unshared* (duplicated implementation, duplicated tests, divergent magnitude curves) outweighed the cost of one clean refactor up front.

### Public API of `dub-spectral`

```rust
pub const FRAME_SIZE: usize = 1024;
pub const HOP_SIZE: usize = 512;
pub const NUM_BANDS: usize = 8;
pub const BAND_MIN_HZ: f32 = 30.0;
pub const BAND_MAX_HZ: f32 = 16_000.0;
pub const LAMBDA: f32 = 1000.0;

pub struct SpectralFrameStream { /* … */ }
impl SpectralFrameStream {
    pub fn new(sample_rate: u32) -> Self;
    pub fn frame_size(&self) -> usize;
    pub fn hop_size(&self) -> usize;
    pub fn half_spectrum_size(&self) -> usize;
    pub fn bands(&self) -> &[(usize, usize); NUM_BANDS];
    pub fn process<F: FnMut(&[f32], &[(usize, usize); NUM_BANDS])>(
        &mut self, block: &[f32], on_frame: F,
    );
    pub fn reset(&mut self);
}
pub fn compute_band_bins(sample_rate: u32, n_bins: usize) -> [(usize, usize); NUM_BANDS];
```

`SpectralFrameStream::process` is alloc-free after construction — verified by `process_is_alloc_free_after_construction` (input-buffer capacity stable across 16 iterations).

### Data layer in `dub-peaks` (M9.5b)

```rust
pub const NUM_BANDS: usize = dub_spectral::NUM_BANDS;            // = 8
pub const BAND_SAMPLES_PER_CHUNK: usize = dub_spectral::HOP_SIZE; // = 512

#[repr(C)]
pub struct BandPeakChunk {
    pub rms_per_band: [f32; NUM_BANDS],   // 8 × f32 = 32 bytes
}

pub struct BandDecimator { /* wraps SpectralFrameStream */ }
impl BandDecimator {
    pub fn new(sample_rate: u32) -> Self;
    pub fn samples_per_chunk(&self) -> usize;
    pub fn feed<F: FnMut(BandPeakChunk)>(&mut self, samples: &[f32], emit: F);
    pub fn reset(&mut self);
}
```

`BandPeakChunk::rms_per_band[k]` is `sqrt(mean(compressed[b]² for b in bands[k]))` — RMS over `dub-spectral`'s per-bin **compressed** magnitudes, not raw FFT magnitudes. The compressed form is already perceptual (μ-law-ish via `ln(1 + λ |X|)`), so RMS over it yields a stable colour-friendly loudness metric. Documented as such in the struct doc so M10.1 doesn't try to reinterpret it as physical RMS.

### Audio-thread cost is zero

The M9 `ThruSource` mono-downmix already produces one shared mono stream consumed by the BPM tap and the peaks tap. **M9.5b adds no new tap.** The same SPSC ring feeds both `Decimator` (broadband, 64-sample cadence) *and* `BandDecimator` (band, 512-sample cadence) inside the same off-RT worker thread — verified by extending the existing `ThruSource` alloc-free tests + the new `bands_on_keeps_broadband_capture_intact` in `dub-peaks` (broadband chunks remain pixel-identical whether bands are on or off).

### Buffer / stream wiring

* `PeakBuffer` is a sum of (always-on broadband Vec + optional band Vec). `with_capacity` is the broadband-only constructor (back-compat for non-band users); `with_capacity_with_bands` is the M9.5b path.
* `band_len()` is a separate `AtomicUsize`, so the renderer's "anything new in the colour channel?" check is lock-free and independent of the broadband side.
* `extend_band_chunks(start_idx, &mut Vec<BandPeakChunk>) -> usize` is the M10.1 fast path; same semantics as the M9 `extend_chunks` for broadband.
* `PeakStreamConfig::bands_enabled: bool` (default `true`). `PeakStream::samples_per_band_chunk() -> Option<usize>` exposes the cadence so renderers can map `peak_idx → band_idx` via integer division.

### CLI surface

* `dub thru --no-band-peaks` opts out of band capture (~ no measurable difference on M1 Air; band data costs ~ 500 µs CPU per second of audio per deck off-RT).
* `dub thru --dump-band-peaks PATH` writes per-band envelopes to a CSV at shutdown. Header: `deck,chunk_idx,b0,b1,...,b7`. Conflicts with `--no-peaks-track` and `--no-band-peaks` are caught at parse time.

### Tests

`cargo test --workspace`: 587 passing. New coverage:

| Crate | Tests | Notes |
|---|---|---|
| `dub-spectral` | 10 unit | Band layout at 44.1k/48k/96k; alloc-free `process`; reset invariants; block-size invariance one-shot vs. streamed |
| `dub-peaks/band_decimator` | 8 unit | Cadence, silence, pure-tone-excites-expected-band (60 Hz / 10 kHz), block-size invariance, reset |
| `dub-peaks/buffer` | 4 unit | Band storage off vs. on, independent push semantics |
| `dub-peaks/stream` | 3 unit | Default ON, ON produces band chunks, ON keeps broadband intact |
| `dub-cli/thru` | 5 unit | Default ON, `--no-band-peaks`, `--dump-band-peaks` path capture + two conflict guards, dump CSV header + row contents |
| `dub-bpm` | -2 unit | `band_bins_*` tests migrated to `dub-spectral` — net 2 fewer in this crate. M8.1 fixture suite (`genre_octave`) unchanged. |

### Acceptance

1. `cargo test --workspace` passes — every M8.1 genre fixture (reggae 65, hip-hop 90 / 100, dnb 174) holds byte-equivalent ODF values.
2. `cargo clippy --workspace --all-targets -- -D warnings` clean.
3. `dub thru --dump-band-peaks /tmp/bands.csv` writes a valid CSV; opening it shows expected band activity (low bands prominent on kick, high bands on hi-hat).
4. Combined-telemetry attach is alloc-free end-to-end — verified by the existing `bpm_and_peaks_tap_both_receive_same_mono_downmix` + the `process_is_alloc_free_after_construction` invariants on the underlying `SpectralFrameStream`.

### What it does not ship

* No FFI surface. `BandPeakChunk` lives in Rust-land until M10.1 wires `band_peaks_extend` through UniFFI.
* No renderer. The data is here; M10.1 implements the multi-colour Metal shader.
* No constant-Q bass split (9-band variant). Deferred to M10.2.

---

<a id="m10a"></a>
## M10-A — `dub-ffi` `DubEngine` UniFFI surface

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2 days (delivered)

### What it is

`crates/dub-ffi` grew from the M0.5 "two free functions" surface (`greeting`, `engine_version`) into a real engine handle the Apple shell can hold for the lifetime of a Thru session. Single UniFFI object, single error type, eight methods.

```rust
#[derive(uniffi::Object)]
pub struct DubEngine { /* Mutex<EngineState> */ }

#[uniffi::export]
impl DubEngine {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self>;

    pub fn list_input_devices(&self) -> Vec<String>;
    pub fn start_thru(&self, device_name: String, channels: Vec<u32>)
        -> Result<(), EngineError>;
    pub fn stop_thru(&self);

    pub fn peaks_len(&self, deck_idx: u64) -> u64;
    pub fn peaks_chunk_duration_secs(&self, deck_idx: u64) -> f64;
    pub fn peaks_extend(&self, deck_idx: u64, start_idx: u64) -> Vec<u8>;

    pub fn band_peaks_len(&self, deck_idx: u64) -> u64;
    pub fn band_peaks_chunk_duration_secs(&self, deck_idx: u64) -> f64;
    pub fn band_peaks_extend(&self, deck_idx: u64, start_idx: u64) -> Vec<u8>;
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum EngineError {
    DeviceNotFound(String),
    InvalidChannels(Vec<u32>),
    AudioStartFailed(String),
    AlreadyRunning,
    NotRunning,
    InvalidDeckIndex(u64),
}
```

### Design choices

* **Proc-macro UniFFI, no UDL.** All `#[uniffi::export]` lives next to the Rust code. The `uniffi-bindgen` workspace binary reads metadata directly from `libdub_ffi.dylib` in library mode — no separate UDL source to keep in sync. `setup_scaffolding!()` emits the C ABI at crate boundary.
* **`#[uniffi(flat_error)]` for `EngineError`.** Swift gets a plain enum with `Display`-derived messages; data-bearing variants embed device names / channel lists into the string. Cleaner Swift ergonomics than a discriminated union for the three error sites that ever inspect specifics.
* **Bytes-not-objects for the hot path.** `peaks_extend` / `band_peaks_extend` return `Vec<u8>` (UniFFI `bytes`). Swift sees `Data`; the renderer reinterprets the bytes as `[PeakChunk]` via `withUnsafeBytes` — zero per-frame allocation and no object-graph traversal across the FFI. Little-endian on both ARM64 and x86_64 macOS keeps the cast safe.
* **`Mutex<EngineState>` not `Arc<Mutex<...>>` at FFI boundary.** UniFFI wraps `DubEngine` in `Arc<DubEngine>` automatically for `#[derive(uniffi::Object)]` types; the internal mutex serialises mutating calls only.
* **Audio-thread non-affecting.** Every method ultimately reads `PeakBuffer` atomics or runs once at `start_thru` to open CoreAudio devices. No method is called from the render thread — Swift never reaches into the IO proc. PRD §10 cross-cutting and `.cursor/rules/ffi.mdc` are satisfied by construction.
* **`Drop` ordering matters.** `RunningState` lists `peaks` first, then `handle`, then `output`, then `input`. The drop sequence stops the decimator thread → flushes the engine command queue → stops the output AU (which reclaims the engine) → stops the input AU (last, so the SPSC ring has no producer-after-consumer race). `stop_thru` is idempotent.

### Tooling deltas

* **`scripts/build-xcframework.sh`**: the embedded modulemap now declares `module dub_ffiFFI { ... }` (was `DubCoreFFI`). The generated bindings include `#if canImport(dub_ffiFFI) import dub_ffiFFI #endif`; matching the C module to the generator's expected name lets `swift build` (and the Apple shell) resolve the C symbols without a post-generation patch.
* **`apple/project.yml`**: adds explicit `CoreAudio.framework`, `AudioToolbox.framework`, `AudioUnit.framework`, `CoreFoundation.framework`, `Metal.framework`, `MetalKit.framework` SDK dependencies. Cargo emits the `cargo:rustc-link-lib=framework=...` directives for `coreaudio-rs`, but those propagate only when Cargo drives the link; Xcode drives the link for the app, so the frameworks have to be surfaced explicitly. Also pins `PRODUCT_NAME` / `PRODUCT_MODULE_NAME` / `ALWAYS_SEARCH_USER_PATHS` since `settingPresets: none` (the M0.5 choice for explicit configuration) drops Xcode's auto-derived defaults.

### Tests

`cargo test -p dub-ffi`: 9 unit tests covering: `greeting`, `engine_version`, FFI version tripwire, fresh-engine peaks defaults, `stop_thru` idempotency, channels validation (empty / wrong-arity / zero-index), and round-trip serialisation of broadband + band chunks. UniFFI binding generation verified end-to-end by `scripts/build-xcframework.sh` (which produces the universal `DubCore.xcframework` + Swift bindings).

### Acceptance

1. `cargo test --workspace` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` passes.
3. `./scripts/build-xcframework.sh` produces `apple/DubCore.xcframework/` and Swift bindings under `apple/DubShared/Sources/DubCore/Generated/`. `swift build` from `apple/DubShared/` typechecks.
4. The generated Swift surface exposes `DubEngine`, `EngineError`, `greeting()`, `engineVersion()` — verified by inspecting `dub_ffi.swift`.

### What it does not ship

* No render-thread state exposed (`xrun_count`, `process_time_ns`, BPM). That's all consumable today through the CLI; UniFFI surface only grows when the macOS UI actually needs it. Adding the BPM telemetry is one extra method when M10.2 wires saturation.
* No background queue or async I/O at the FFI boundary. `start_thru` blocks until CoreAudio comes up — typical 50-200 ms on first open. Swift wraps the call in a `Task { ... }` if it cares about UI responsiveness; nothing in the FFI surface requires it.

---

<a id="m10b"></a>
## M10-B — Metal renderer + first live broadband waveform

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 3 days (delivered)

### What it is

The Apple shell now shows a live, scrolling broadband waveform of input audio. Pick an input device, pick channels, hit Start — the M10-A engine fires up, the M9 peak buffer accumulates `PeakChunk`s, and a Metal-backed `MTKView` renders the most recent ~5 seconds of audio at 60 fps.

### File layout

* `apple/Dub/Waveform/Shaders.metal` — vertex + fragment shaders. Vertex stage emits 4 vertices per instance (a triangle strip per `PeakChunk`) sized from `min`/`max`; fragment stage outputs an RMS-modulated greyscale for M10-B. M10.1 swaps in the multi-colour fragment shader against the same vertex pipeline.
* `apple/Dub/Waveform/WaveformRenderer.swift` (~280 lines) — `@MainActor` Metal renderer. Owns `MTLDevice`, `MTLCommandQueue`, render pipeline state, **two** triple-buffered uniform `MTLBuffer`s (one per inflight frame, bounded by `DispatchSemaphore(value: 3)`), and **one** ring-buffer `MTLBuffer` for chunks (`chunkCapacity = 2^17 ≈ 175 s` at 48 kHz / 64 samples).
* `apple/Dub/Waveform/WaveformView.swift` — `NSViewRepresentable` wrapping `MTKView`. The view's `Coordinator` is the `MTKViewDelegate`; both `drawableSizeWillChange(_:)` and `draw(in:)` hop to `@MainActor` via `MainActor.assumeIsolated` since the Metal renderer is main-actor isolated.
* `apple/Dub/MainView.swift` — top-level SwiftUI view. Hosts the device `Picker`, channels `TextField`, Start/Stop button, the waveform, and a one-line debug overlay showing `greeting() · v<engine_version>` (the M0.5 smoke text, now demoted to a debug line). Owns a `WaveformAppModel: ObservableObject` that wraps the shared `DubEngine` and exposes `availableDevices`, `selectedDevice`, `isRunning`, `lastError`. `EngineError` is mapped to user-readable strings in `describe(_:)`.

### Rendering pipeline

* **One quad per chunk, instanced.** `drawPrimitives(.triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: chunksVisible)`. The vertex shader reads `chunks[chunkOffset + instance_id]` from the ring buffer and emits a bar from `(x − dx, min*yScale)` to `(x + dx, max*yScale)`. Vertex-ID bit layout: bit 1 → right edge, bit 0 → top edge. `yScale = 0.95` keeps the bars off the viewport edges.
* **Bar amplitude.** Empty chunks (no samples) clamp to a ±1e-4 hairline so the leading edge renders as a thin centred line instead of a hidden zero-thickness triangle. Doesn't affect any chunk with real audio.
* **Window size.** `chunksVisible = pixel_width × 4` — about 4 ms per pixel at 48 kHz / 64-sample chunks, ~5.4 seconds on a 1280-pixel-wide window. Configurable via `chunksPerPixel`.
* **Ring buffer ingest.** Each `draw(in:)` calls `engine.peaksLen` → if it grew, `engine.peaksExtend(..)` with the cached cursor. The returned `Data` is `memcpy`'d into the GPU ring starting at `(startIdx % chunkCapacity) * 12` bytes, with one wrap-around copy when the write crosses the ring boundary. `cappedNew` truncates catch-up to one ring's worth so a long UI stall (e.g. moving the window) doesn't memcpy gigabytes when the renderer resumes.
* **Frame pacing.** `MTKView.isPaused = false`, `enableSetNeedsDisplay = false`, `preferredFramesPerSecond = 60`. The semaphore caps inflight CPU work at 3 frames ahead of the GPU; reset of a wedged GPU is fatal (we accept the convention).
* **Storage modes.** Both ring buffer and uniform buffers use `.storageModeShared`. On Apple Silicon's unified memory, this is zero-copy (CPU and GPU share pages). On Intel macs, the small bandwidth hit (~5 MB max per deck) is irrelevant compared to the round-trip cost of `.storageModePrivate` + blits.

### View model

* `WaveformAppModel: ObservableObject` owns a single `DubEngine`. Construction calls `refreshDevices()`; `deinit` calls `engine.stopThru()` defensively (UniFFI's `Drop` would do it too, but the explicit teardown is deterministic across SwiftUI lifecycles).
* `start()` parses the comma-separated channel field, validates two 1-based values, and dispatches `engine.startThru(...)`. `EngineError` lifts into `lastError`.
* `stop()` calls `engine.stopThru()` and clears `isRunning`. Idempotent (matches the engine's idempotent `stop_thru`).

### Bootstrap

`./scripts/bootstrap.sh` now produces:

```
apple/DubCore.xcframework/        # Universal aarch64 + x86_64 static lib + headers
apple/DubShared/Sources/DubCore/Generated/   # dub_ffi.swift, headers, modulemap
apple/Dub.xcodeproj/              # Generated from project.yml by XcodeGen
```

`xcodebuild build -project apple/Dub.xcodeproj -scheme Dub` produces a runnable `Dub.app` that links CoreAudio, AudioToolbox, AudioUnit, CoreFoundation, Metal, and MetalKit explicitly.

### Acceptance

1. `cargo test --workspace` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` clean.
3. `./scripts/bootstrap.sh && xcodebuild -project apple/Dub.xcodeproj -scheme Dub -configuration Debug build` succeeds — `Dub.app` lands in DerivedData with `CoreAudio`, `AudioToolbox`, `AudioUnit`, `Metal`, `MetalKit` linked.
4. Launching the app, picking an input, hitting Start: the waveform scrolls in real time at the device's natural rate; quitting the app cleanly tears down the audio threads (UniFFI's auto-`Drop` on the engine handle runs the documented `Drop` order on `RunningState`).

### Threading + RT discipline

* Audio thread → only Rust; no Swift call, no Metal call. (`.cursor/rules/audio-rt.mdc` and `ffi.mdc` invariants both satisfied.)
* Render thread → main actor; calls `engine.peaksLen` / `peaks_extend` which read `AtomicUsize` + take an `RwLock::read` on the peak buffer. The lock is reader-priority and dropped before the encoder records any GPU work.
* Engine teardown on `stop_thru` runs synchronously from the UI's perspective — drops the peak streams, the engine handle, then the output AU, then the input AU.

### What it does not ship

* **Monochrome only.** RMS modulates a greyscale brightness so transients are visible, but the multi-band data captured in M9.5b isn't read yet. M10.1 wires `band_peaks_extend` and swaps in the colour shader against the same vertex pipeline (zero changes to the renderer's vertex stage or buffer layout).
* **Deck A only.** The FFI surface and the renderer both index decks; we only attach a Thru source on deck 0 today. Deck B is one `attach_thru_source_with_peaks_tracking(1, …)` call away in `start_thru_inner`, plus a second `WaveformView` in `MainView`. Deferred to M10.2 along with palettes.
* **No transport / no track loading.** Thru only. Track loading remains a CLI-only feature until the M11+ library + transport milestones.
* **No CI build target for Apple.** GitHub Actions stays Linux-only; a macOS runner + `make apple` target is part of the post-M10.2 distribution work.

---

---

<a id="m101"></a>
## M10.1 — Multi-colour fragment shader

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 3 days (delivered)

### What it is

The M10-B waveform is no longer monochrome. Each `PeakChunk` bar in the renderer now carries an 8-band perceptual-loudness vector (the `BandPeakChunk` data layer shipped in M9.5b), and the fragment shader mixes those bands into Serato-grade RGB:

```
R = mean(b[0], b[1])              kick / bass:   30 - 159 Hz
G = mean(b[2], b[3], b[4])        mids / vocals: 159 - 1934 Hz
B = mean(b[5], b[6], b[7])        highs / air:   1934 - 16000 Hz
```

`r`/`g`/`b` get per-channel gains (`1.2 / 1.8 / 2.4`) to compensate for the natural loudness imbalance — low bands carry more energy per FFT bin because there are fewer bins per log-spaced band. The bar's vertical extent still encodes peak amplitude (M10-B carries through); colour encodes the spectrum.

### What's new in this milestone

* **`Shaders.metal`**: the fragment shader picks up `bandLow`/`bandHigh` (two `float4`s = the 8 band RMS values) forwarded by the vertex shader, mixes them into RGB per the palette above, and applies a brightness floor + RMS-driven luminance pass. Silence ( `max(r,g,b) < 0.05` ) drops to neutral grey so dropouts read as "honest silence" rather than a colour cast.
* **`WaveformRenderer.swift`**: adds a second `MTLBuffer` (`bandChunksBuffer`, 2¹⁴ × 32 B ≈ 512 KB per deck) for the parallel band ring, a second polling path (`ingestNewBandChunks`), and four new fields in the uniforms struct (`samplesPerPeakChunk`, `bandChunkOffset`, `samplesPerBandChunk`, `bandCapacity`). The vertex shader uses these to map each broadband instance to its containing band chunk via `(iid × samplesPerPeakChunk + samplesPerPeakChunk/2) / samplesPerBandChunk`.
* **`crates/dub-ffi`**: tiny addition — `DubEngine::sample_rate() -> u32`. Combined with the already-shipped `peaks_chunk_duration_secs` / `band_peaks_chunk_duration_secs`, this lets the renderer derive `samples_per_chunk` exactly (`duration × sample_rate`) instead of snapping a heuristic across candidate sample rates. Tripwire constant `FFI_VERSION` bumps to 3.

### Why the band ring is parallel, not embedded

Both rings appended sequentially with `memcpy`s from the FFI; both indexed in NDC via a power-of-two modulo (which compiles to a bitmask in the shader). Keeping them parallel rather than embedding band data into `PeakChunk` keeps:

* the broadband chunk stride at 12 bytes — half the size of a `(min, max, rms, 8 × band)` packed alternative;
* M10-B and M10.1 backwards-compatible — broadband-only rendering still works exactly the same with a stopped or band-disabled engine;
* the shader vertex-stage cost flat — one extra buffer read, no branch on "is this band data?".

### Audio-thread cost

Zero new audio-thread work. The M9.5b decimator thread was already producing `BandPeakChunk`s and writing them to the parallel `PeakBuffer` storage; M10.1 just consumes them on the render thread.

### Renderer thread cost

Per frame: one extra `engine.bandPeaksLen` (atomic load), conditionally one extra `engine.bandPeaksExtend` (RwLock read), and one extra `memcpy` (~32 KB worst case per frame even on heavy catch-up). All bounded; all main-thread.

### Acceptance

1. `cargo test --workspace` passes — `dub-ffi` tests for `FFI_VERSION = 3` + `sample_rate = 0 when stopped`.
2. `cargo clippy --workspace --all-targets -- -D warnings` clean.
3. `./scripts/bootstrap.sh && xcodebuild build -scheme Dub` builds the universal `Dub.app`.
4. Live qualitative check: a clean kick paints red, a hi-hat / cymbal paints blue, vocals / synth pads paint green; silence renders as a thin grey hairline; loud transients brighten the bar without changing its hue family.

### What it does not ship

* **No palette presets.** The default Serato-faithful mapping is baked into the shader; user-selectable palettes (high-contrast, monochrome fallback, custom) land in M10.2.
* **No onset glow.** Beat-aware additive bloom on `dub-bpm`-confirmed onsets is M10.2.
* **No constant-Q bass split.** The 9-band variant (sub-bass < 60 Hz, kick 60-200 Hz) is M10.2 in `dub-spectral`.
* **Deck A only.** Same as M10-B; deck B wiring + a second `WaveformView` is M10.2.

---

---

<a id="m102"></a>
## M10.2 — Polish: deck B, palette presets, honest silence/clipping

**Status:** shipped (partial — see "What it does not ship") &nbsp;·&nbsp; **Estimate:** 3 days (delivered)

### What it is

M10.2 is the "exceeds-Serato polish" pass. Per the plan it's a stack of independently-shippable bullets; this milestone ships the three with the highest visible impact and the simplest delivery cost:

1. **Deck B wired identically.** Two-deck Thru sessions, two waveform views.
2. **Palette presets.** Three baked-in palettes (Serato-faithful = M10.1 default, high-contrast, monochrome) switchable from the toolbar.
3. **Honest silence and clipping.** The fragment shader paints silent stretches as a thin neutral hairline and clipped chunks as a solid red bar — no more "loud + silent both render as white".

Onset glow, beat-aware saturation, constant-Q bass split (9-band `dub-spectral`), and mip pyramids are still pending as future polish; each is independently shippable on top of this baseline.

### Deck B

* **`dub-ffi` — `DubEngine::start_thru_two_deck(device, channels_a, channels_b)`.** Same shape as `start_thru` but takes two channel pairs. Opens the input AU with the combined 4-channel set, uses CoreAudio `output_pairs = [(0,1), (2,3)]` to demux into two stereo SPSC consumers in the IOProc, and attaches a `ThruSource` + `PeakStream` on both deck 0 and deck 1. Validates non-overlapping pairs (returns `EngineError::InvalidChannels` with the merged list on overlap).
* **`start_thru_inner` is now the two-deck core.** Single-deck `start_thru` calls it with `channels_b: None`; the function builds the input options + attaches deck 1 conditionally. No code duplication between the two FFI entry points.
* **`MainView`.** Two channel fields: `chA` (defaults to `1,2`) and `chB` (empty = single-deck mode, matching M10-B's behaviour exactly; non-empty = two-deck mode). When two-deck is running, the waveform area splits vertically via `VSplitView` into two `WaveformView`s sharing the same palette.
* **`FFI_VERSION = 4`.**

### Palettes

* **Three presets in the shader.**
  - `0` — **Serato-faithful** (M10.1 default): bass→R, mids→G, highs→B with per-channel loudness compensation + normalised brightness floor.
  - `1` — **High-contrast**: same band mix but squared (boosts strong bands, suppresses weak), then renormalised with a higher brightness floor. Designed for bright rooms / projector-driven club setups where the default washes out.
  - `2` — **Monochrome**: collapses hue entirely; bar tone driven purely by broadband RMS. Equivalent to the M10-B look — useful as an "honest amplitude-only" reference when the colour layer is misleading (e.g. when checking a mix).
* **Uniforms.** The Swift-side `WaveformUniforms` gained a `palette: UInt32` field (replacing the previous `_reserved`). `WaveformView` takes a `palette: WaveformPalette` and forwards it to the renderer via `updateNSView`; the renderer reads it on the next frame.
* **UI.** A `Menu` in the toolbar with a paintpalette icon. `WaveformPalette` is `CaseIterable` so adding palettes is a one-line addition + one shader branch.

### Honest silence and clipping

* **Vertex-stage flags.** Each instance now emits `flags = (clipping, silence, palette, 0)` per quad:
  - `clipping = 1.0` when `max(|min|, |max|) >= 0.98` — a peak so close to ±1 we'd call it clipped.
  - `silence = 1.0` when `|min| + |max| < 1e-3 AND rms < 1e-4` — essentially zero audio in this chunk.
* **Fragment-stage branches.**
  - Clipping ⇒ solid red `(1.0, 0.05, 0.05)`. Unmistakable; the user is expected to act on this (turn the offending deck's gain down).
  - Silence ⇒ thin dim grey `(0.18, 0.18, 0.20)`. Honest dropout; visually distinct from a fully-saturated mid signal.
  - Neither ⇒ colour path (per-palette mix).
* **Why per-instance flags, not per-fragment.** The fragment shader can't see the raw `PeakChunk` `min`/`max` once the rasteriser has run; computing flags in the vertex stage and forwarding via `VertexOut` is one float4 per quad regardless of bar pixel height. All four quad corners come from the same instance, so rasteriser interpolation collapses to the per-instance constant — no precision concern.

### Why these three, not all seven

The plan's M10.2 list is seven items; landing all seven would have taken ~2 weeks of mostly disjoint work (BPM FFI accessors, a new band-layout migration in `dub-spectral`, a renderer-side mip pyramid). The plan explicitly calls out that each bullet is "independently shippable; user picks ordering at the end of M10.1" — so this milestone is the minimum-cost subset that lands a *user-perceptible* polish step (deck B + palettes + honest silence/clipping all visible without any audio engineering background to interpret).

### Tests

* **`dub-ffi`** — 11 unit tests passing. Added `start_thru_two_deck_rejects_invalid_or_overlapping_channels` covering wrong arity per side, zero indices, and the A/B overlap rejection. `FFI_VERSION = 4` tripwire.
* **Workspace** — `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` both green.
* **Apple build** — `./scripts/bootstrap.sh && xcodebuild -scheme Dub build` produces a universal `Dub.app`.

### Acceptance

1. Running the app with a single channel pair (e.g. `1,2` / empty deck B) reproduces the M10.1 single-waveform behaviour exactly.
2. Filling in both deck fields (`3,4` / `5,6` on an SL3) opens both inputs, demuxes in the IOProc, and shows two parallel waveforms stacked via `VSplitView`. Each deck's bars colour independently.
3. Cycling through palettes via the toolbar menu changes the waveform appearance immediately without restarting the audio session.
4. Playing a clipped signal renders the offending bars solid red. Cutting the input mid-bar renders silence as the dim grey hairline.

### What it does not ship

The plan's remaining polish bullets are each independently shippable as follow-up milestones:

* **Onset glow** — needs a new `dub-ffi` accessor for `dub-bpm`'s `BpmStream`'s onset confidence trail; renderer applies an additive bloom on confirmed onsets.
* **Beat-aware saturation** — same FFI extension as onset glow; multiplies the palette gain by `(0.7 + 0.3 × confidence)` so noisy / silent stretches desaturate.
* **Constant-Q bass split (9-band)** — touches `dub-spectral`'s band layout: bump `NUM_BANDS` from 8 to 9 by splitting the lowest log-band into a sub-bass (30-60 Hz) + kick (60-200 Hz) pair. Affects every downstream consumer (BPM ODF, peak storage, FFI wire format, shader). One coherent PR but a meaningful refactor.
* **Mip pyramids** — pre-decimated levels (e.g. mip-2 = average every 4 chunks, mip-3 = every 16) in `dub-peaks` so the renderer can show longer time windows by reading from a coarser mip. Deferred from M9 per existing PRD note.

---

<a id="m102-remainder"></a>
## M10.2 remainder — superseded by the M10.5h–p shader ladder, then rolled back in M10.8

**Status:** retired (all four bullets superseded) &nbsp;·&nbsp; **Resolution:** see [§M10.8](#m108) for the current Serato-parity baseline that replaced the shader ladder.

All four originally-deferred polish bullets from M10.2 (`SHIPPED.md` [§M10.2 *What it does not ship*](#m102)) were re-homed onto the M10.5h–p shader ladder rather than being shipped as M10.2 follow-ups:

- **Onset glow** → M10.5l (additive HDR overshoot driven by the new `OnsetDecimator`).
- **Beat-aware saturation** → M10.5m(a) (luma-rotation in fragment, riding the same `onsetConf` data as M10.5l).
- **Constant-Q bass split (9-band)** → M10.5m(b) (deferred to M11 — gnarliest piece; held until DJ-curated content lands to validate the colour change).
- **Mip pyramids** → M10.5k (planned, paired with the M10.5j on-disk sidecar so the pyramid is on-disk too).

The new ladder also added three pieces that weren't on the M10.2 remainder list and turned out to be load-bearing for "great, not mediocre" on 2026 hardware: M10.5h (HDR off-screen target + separable Gaussian bloom + ACES tonemap), M10.5i (continuous filled-envelope geometry), and M10.5j (sidecar cache so a re-load is ~1 ms instead of ~150 ms).

**Final disposition:** in M10.8, the entire M10.5h–p shader ladder (HDR, bloom, ACES tonemap, multi-pass post-processing, the `WaveformTuning` / `WaveformTuningPanel` runtime knob surface, the onset-driven brightness layer, the kick-emphasis tint, the time-domain `FilteredPeakChunk` ring) was **deleted from the runtime** in favour of a single-pass Serato-parity shader. See [§M10.8](#m108) for the current baseline and the future-work guardrail. The shader-ladder write-ups below are preserved as design archaeology — they explain why specific approaches were tried and what they cost, which is load-bearing for any future polish work that wants to revisit those ideas without re-running the same dead ends.

---

<a id="m103"></a>
## M10.3 — Performance shell

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 3 days

Launching `Dub.app` shows the real Performance View (per PRD §9.2): a thin status strip, two two-row deck headers, the Metal waveform in the wide centre region, and correctly-sized placeholders for the FX bar (lit by M15 / M16) and library (lit by M11). The dev toolbar (device picker / channels / palette) moves behind a `⌘,` Preferences sheet so the performance surface stays mouse-free at rest.

`apple/Dub/DesignSystem/Tokens.swift` becomes the single source of truth for colour / type / spacing; the Figma file documented in PRD §9 reverts to a reference artefact (it does not gate any future UI work). Deck-header BPM / pitch / key / FX columns render as `—` placeholders until their FFI accessors land — surfacing the M8 BPM tracker over UniFFI is a trivial follow-up, pitch / key / FX wait on M13 / M14 / M15. Snapshot tests (`swift-snapshot-testing`) deferred to M18 polish; the M10.3 demo is visual eyes-on.

---

<a id="m104"></a>
## M10.4 — Vertical waveform + symmetric two-pane layout

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 1–2 days

Two bugs in the M10.3 build, fixed together:

**(a)** the Metal renderer is rotated from horizontal to **vertical** per PRD §9.1 (forward play = waveform marches upward through the playhead at 25 % from the top; reverse play = marches downward; direction follows engine rate sign with no inference). Touches `Shaders.metal` (vertex shader emits Y-instanced quads), `WaveformRenderer.swift` (buffer layout + view projection), `WaveformView.swift` (frame sizing tall not wide), and `PerformanceView.swift` (waveform region becomes `HSplitView` of two tall columns).

**(b)** Symmetric layout invariant: both deck waveform panes are always rendered side-by-side. In single-deck mode (deck B `chB` empty in Preferences) deck B's pane shows an idle placeholder matching the deck B header's `OFF` state instead of vanishing.

Status strip gains live battery + wall-clock per PRD §9.3 (`IOPSCopyPowerSourcesInfo`-driven battery, system clock for wall time). **Demo criterion:** every screenshot from M10.4 forward is in the canonical orientation.

---

<a id="m105"></a>
## M10.5 — File playback dev loop (M10.5a + M10.5b)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 4–5 days

The dev-loop unblocker — Dub becomes testable without an SL3. Splits into **M10.5a** (Rust + FFI) and **M10.5b** (Apple shell).

**M10.5a — shipped:** new `DubEngine` surface (`start_engine` for output-only sessions, `load_track`, `play`, `pause`, `seek`, `position` → `PositionInfo`, `track_info` → `TrackInfo`); `dub-peaks` gains `compute_offline_peaks` so whole-track peaks compute synchronously at load time; the FFI's per-deck `PeakSource` enum routes `peaks_extend` through either the live Thru stream (M9) or the offline File buffer (M10.5a) transparently; `FFI_VERSION` 4→5.

**M10.5b — shipped (Apple shell):** auto-detect lifecycle (multi-channel input → Timecode mode via `start_thru_two_deck`; built-in only → Prep mode shell via `start_engine`); single-pass renderer refactor (`chunksAbovePlayhead` uniform, vertex shader linear y-map across full NDC, M10.4 behaviour preserved when no future peaks are present); drag-and-drop a file from Finder onto either deck pane → loads + deck header switches source pill to `FILE` and populates title / duration / track-time; 30 Hz position polling drives the deck-header time row; slim FS browser replaces the `LIBRARY — M11` placeholder (folder picker + single-click selection, **no double-click load**); `Space` loads the FS-browser-selected file into the non-master, stopped deck per PRD §6.4 (or into deck A in any single-deck mode — Prep, single-channel Timecode — since "non-master" isn't meaningful when only one deck exists). If the target deck is playing, the pane flashes red for 200 ms with a "deck is playing — lift the needle" overlay; **master-deck tracking** wires up per PRD §6.4 with a `MASTER` chip in the master deck's header. Preferences sheet is auto-apply: changing mode / device / channels restarts the engine immediately, no Apply button. App auto-starts on launch in the auto-detected mode.

**Auto-detect is permission-safe:** routes through `DubEngine::has_external_audio_interface` which queries CoreAudio transport-type metadata only (USB / Thunderbolt / FireWire / PCI / AVB) — `listInputDevices` is *not* called when Prep mode is picked, so the macOS microphone-permission prompt only ever fires when the user explicitly engages Timecode mode against an external interface.

Renderer gains a per-deck **peak-generation counter** (`DubEngine::peaks_generation`, atomic, survives stop/start cycles) so a Thru → File swap on a drag-and-drop load forces the renderer to reset its ring + cadence cache before re-ingesting from the new source — without this signal the length-monotonicity heuristic gets stuck rendering stale Thru chunks indefinitely. `FFI_VERSION` 5→7 (one bump for `peaks_generation`, one for `has_external_audio_interface`).

**No library DB, no metadata indexing, no crates, no other keyboard transport, no overview waveform** (that's M10.5c) — those are M11 / per-feature future milestones.

---

<a id="m105c"></a>
## M10.5c — Track Overview waveform + horizontal-orientation shader

**Status:** shipped (a + b) &nbsp;·&nbsp; **Estimate:** 2 days

The two pieces of M10.5b shakedown that didn't fit in the shell pass.

**M10.5c-a — shipped:** `TrackOverviewView` (SwiftUI `Canvas`) slotted on each deck's outside edge with playhead-bracket tracking + File-mode click-to-jump per the description below.

**M10.5c-b — shipped:** `orientation: u32` uniform plumbed end-to-end (Metal `Uniforms` struct, Swift `WaveformUniforms`, `WaveformRenderer.orientation` property, `WaveformView(orientation:)` parameter, host `WaveformMetalView` pipes the value into the renderer and forces a uniform refresh on change, playhead overlay swaps between horizontal hairline / vertical hairline based on orientation). Default remains `.vertical` so every M10.4 / M10.5b call site renders bit-identical pixels.

**Track Overview** (PRD §9.6.1): per-deck thin vertical strip on the deck's outside edge (`DubLayout.deckOverviewWidth ≈ 36 px`) showing the *whole* track top→bottom with a playhead-bracket indicator at the current `position(deck)`. Renders via SwiftUI `Canvas` (not Metal — overview is a low-cadence, fully-known-up-front signal that doesn't benefit from GPU instancing; `Canvas` keeps the pipeline simpler and the shader inventory smaller). Reads broadband peaks via `peaks_extend(deck, 0)` once at load, decimates to ≈ 300 buckets (the strip's pixel height at typical window sizes), redraws only when the playhead chunk changes (≈ 30 Hz from the existing position poll). **Click-to-jump** plumbed for File mode immediately; Timecode-mode behaviour gated on M10.6's Panic-Play wiring.

**Horizontal-orientation Metal uniform:** adds `orientation: u32` (0 = vertical, 1 = horizontal) to `WaveformUniforms` and the matching `Shaders.metal` constant buffer; the vertex shader picks the NDC x↔y assignment based on the uniform. Vertical orientation is the default and the M10.4 / M10.5b behaviour is bit-identical; horizontal flips the playhead from "25 % from top" to "25 % from left" with the future to the right of the playhead. Lights up Prep mode's horizontal layout in M10.8 without that milestone needing to touch the shader. No FFI version bump (renderer-only).

---

<a id="m105d"></a>
## M10.5d — Background load (decode + peaks off-thread)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 0.5 days

The perceived "loading is slow" pain was the FFI's `load_track` doing synchronous `Track::load_from_path` (symphonia decode → `Vec<f32>` of the whole file) plus `compute_offline_peaks` (broadband + 3-band ring fills across all samples) **under the engine-state mutex** on the SwiftUI main actor. Two compounding effects: (1) the call blocked the main actor for ~50–300 ms depending on track length, freezing the UI; (2) the engine-state mutex stayed held throughout, so every concurrent `position()` / `peaks_extend()` / `track_info()` call (the 30 Hz UI poll + waveform poll) blocked behind the loader too — Swift-side dispatch alone would not have helped.

**Rust fix** in `crates/dub-ffi/src/lib.rs`: split `load_track` into three phases. Phase 1 takes the mutex briefly to verify `EngineState::Running`. The guard drops. Phase 2 does the slow decode + peaks compute **mutex-free** — the rest of the API stays responsive throughout. Phase 3 re-acquires the mutex, re-checks `Running` (the engine could have been stopped during decode; if so, the freshly-built `Arc<Track>` + peak vectors drop on the caller's thread, harmless), then installs the new track + peaks and bumps `peak_generation_seq` while still holding the guard (no torn-read window — a renderer that sees the new peaks also sees the new generation). The generation atomic lives on `DubEngine` directly, not inside the `Mutex<EngineState>`, so the access doesn't recurse.

**Swift fix** in `apple/Dub/MainView.swift` + `Performance/PerformanceView.swift`: `WaveformAppModel.loadTrack(side:url:)` becomes `async`, dispatches the FFI call onto a `Task.detached(priority: .userInitiated)` so it doesn't pin the SwiftUI main actor either. New `DeckState.isLoading: Bool` tracks in-flight loads; concurrent load on the same deck red-flashes the deck pane and surfaces "Deck *X* is already loading — wait or load onto the other deck". Optimistic UI: the new file's title fills in immediately and the deck-header source pill flips to a new `Source.loading` variant ("LOADING…", amber dot) before decode starts — the user sees the deck respond to the drop instantly, even though the audio swap lands ~tens of ms later. A *replace*-load (new file decoded while a previous one is resident) keeps the old waveform + transport toggle live during decode and swaps in atomically when `peak_generation_seq` bumps. `loadBrowserSelectionIntoTargetDeck()` becomes `async` to match; the Space-key NSEvent handler awaits inside its existing `Task { @MainActor in ... }` wrapper.

No FFI bump.

---

<a id="m105v"></a>
## M10.5v — Load-never-blocks-playback + O(N²) BPM bug fix

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 0.5 days

Two compounding problems, surfaced together when M10.5u re-enabled `analyze_beat_grid` inside `load_track`.

### Symptom

Dogfood report: "loading a song is very slow now. i think the bpm detection takes very long." A 4-minute MP3 sat in the LOADING pill for many seconds before the deck would respond to `Space`; the deck-header BPM column would then flicker in after a further delay.

### Root cause 1: `load_track` blocked on analysis

M10.5d had moved decode + offline peaks off the engine-state mutex, but the work was still synchronous inside the FFI call. M10.5u then added `analyze_beat_grid` *inside* `load_track`'s Phase 2 — the doc comment claimed "~100 ms" of ODF + autocorrelation work, which would have been tolerable. In practice the call was orders of magnitude slower (see root cause 2). Either way, the engine doesn't *need* peaks or a beat grid to start playback — the audio thread only needs the `Arc<Track>`. Gating playback on either was a design mistake, not just a perf bug. Violates PRD §6.4 "load never blocks playback".

### Root cause 2: O(N²) blowup in `SpectralFrameStream`

`crates/dub-spectral/src/lib.rs` was using `Vec::drain(..HOP_SIZE)` to slide the analysis window forward by one hop per frame. `drain` on a `Vec` is **O(remaining)** — it shifts every element after the drained prefix back to position 0. For a streaming caller that hands the stream small blocks (which was the original use case) the remaining buffer is bounded by `FRAME_SIZE + block_size`, so the per-frame cost stays small. For the offline caller that passes the *entire* decoded track in one `process(samples)` call, the buffer contains 10.6 M samples; each `drain(..512)` shifts ~10.6 M elements; there are ~21 000 frames; total work is **~220 billion sample moves**. A benchmark on a synthetic 240 s 44.1 kHz stereo track measured 38 019 ms inside `analyze_beat_grid` — far past the "I think the BPM detection takes very long" threshold.

### Fix 1: `load_track` returns the instant the audio thread can play

`crates/dub-ffi/src/lib.rs`:

- `DubEngine.state` becomes `Arc<Mutex<EngineState>>` and `peak_generation_seq` becomes `Arc<[AtomicU64; 2]>`, so a detached worker thread can hold an owned handle to both.
- `load_track` now decodes synchronously (kept that way to return `TrackDecodeFailed` to the caller), then takes the mutex briefly to install the new `Arc<Track>` into `running.handle.deck(idx).load(...)` + `running.file_tracks[idx]`, clear `running.peaks[idx]` to `None`, and bump `peak_generation_seq`. The mutex drops and the FFI call returns. From this point on, `play()` is fully functional.
- Phase 4 is a `std::thread::Builder::new().name("dub-load-N").spawn(...)` worker that runs `compute_offline_peaks` → `analyze_beat_grid` sequentially. After each stage the worker re-acquires the mutex *briefly*, checks via `Arc::ptr_eq` that `running.file_tracks[idx]` still points at the track it was analysing (back-to-back loads on the same deck race the worker; the loser drops its results on the worker stack, off-RT, harmless), then installs the result. Peaks install bumps `peak_generation_seq` so the Swift renderer resets to the new data; the BPM install does *not* bump the generation (it doesn't affect waveform rendering — the deck-header BPM column reads it on the next 30 Hz position poll).
- Timing is `eprintln!`'d at each stage for dogfood verification.

`apple/Dub/MainView.swift`:

- `loadTrack(side:url:)` stops awaiting `engine.beatGrid` inline (it would be empty — the worker is still computing). Sets `bpm = nil`, `bpmConfidence = 0`, returns.
- `readDeckState` (the 30 Hz position poll) now lazily polls `engine.beatGrid(deckIdx:)` while `hasTrack && bpm == nil`. Once a valid grid lands, the condition latches and polling stops; tracks with no detectable BPM (silence, noise, too-short) continue polling at ~µs per tick — well under budget.

### Fix 2: cursor + amortised compaction in `SpectralFrameStream`

`crates/dub-spectral/src/lib.rs`:

- New `input_read_pos: usize` field next to `input_buffer: Vec<f32>`. The hop loop reads from `input_buffer[input_read_pos..input_read_pos + FRAME_SIZE]` and advances the cursor by `HOP_SIZE` instead of draining the front.
- At the end of `process`, a new private `compact_input_buffer` shifts the unread tail (at most `FRAME_SIZE - 1` samples) back to index 0 and resets the cursor. Each sample is `memcpy`'d at most twice (once into the buffer via `extend_from_slice`, once out on compaction), so the per-call cost is O(block) and the per-track cost is O(N). `reset()` also clears the cursor.

Result on the 240 s synthetic benchmark: `analyze_beat_grid` drops from 38 019 ms to 312 ms (**~122× faster**). The 30 s sub-clip drops from 311 ms to 46 ms. Scaling is now linear (8 × audio → ~7 × compute) where it used to be near-quadratic (8 × audio → 122 × compute).

All `dub-spectral` and `dub-bpm` tests pass unchanged, including `process_is_alloc_free_after_construction` (the cursor doesn't allocate) and `block_size_invariance_one_shot_vs_streamed` (one-shot vs streamed paths produce byte-identical ODFs because compaction is functionally a no-op for the algorithm's output).

### What this does NOT do

- Does not change the BPM algorithm itself (still log-band spectral-flux + harmonic-summed autocorrelation per M9). The 38 s figure was infrastructure overhead, not algorithm cost.
- Does not parallelise peaks and BPM. They run sequentially inside the worker thread — peaks first (waveform appears in ~20 ms) then BPM (header populates ~300 ms later). Parallel rayon::join was considered and rejected as needless complexity for sub-second work.
- Does not add a "BPM ready" atomic flag. The Apple side just polls `engine.beatGrid` lazily on the existing 30 Hz tick. Cheap enough.
- No FFI bump — `PositionInfo` / `BeatGrid` shapes are unchanged.

### Files touched

- `crates/dub-ffi/src/lib.rs` — `DubEngine.state` and `peak_generation_seq` become `Arc<…>`; `load_track` returns after the engine swap; new `background_analyze_and_install` + `track_still_loaded` helpers.
- `crates/dub-spectral/src/lib.rs` — `input_read_pos` cursor; new `compact_input_buffer`; cursor reset in `reset()`.
- `apple/Dub/MainView.swift` — `readDeckState` lazy BPM poll; `loadTrack` stops awaiting the grid inline; doc rewritten.
- `docs/PRD.md` — §6.4 "Load never blocks playback (M10.5v)" + M10.5v table row.

---

<a id="m105e"></a>
## M10.5e — Waveform polish (compression + past-region dim + brighter floor)

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 0.5 days

> **Note:** the soft-amplitude compression and past-region dim shipped here survive into the M10.8 baseline conceptually but live in different code paths after the M10.8 shader rewrite. The exact constants below describe the pre-M10.8 shader.

The "ugly waveform" pain: linear amplitude makes typical -14 LUFS music live in the inner ~30 % of the deck column; uniform brightness across past/future kills the depth cue from the bottom→top scroll; thin RMS-driven palette saturation washes out under projector lighting.

**Shader-level** fixes in `apple/Dub/Waveform/Shaders.metal`: (1) **soft amplitude compression** `displayAmp = sign(x) * |x|^0.55` applied to `lo` / `hi` *after* the honest-state `clipping` / `silence` flags read the raw values — peaks at 0.3 now render at ~0.50, peaks at 0.7 at ~0.82, and an already-clipped 1.0 stays at 1.0. Visually fills the column on most masters without lying about the underlying signal. (2) **Past-region dim** routed through `VertexOut.flags.w`: the vertex stage sets it to 1.0 for chunks above the playhead, 0.0 below; the fragment multiplies the final RGB by `mix(1.0, 0.62, isPast)`. Applied uniformly to all three palette branches *and* to the honest-state clipping/silence colours so the depth cue stays consistent across visualisation modes. (3) **Brighter luminance floor**: the final RMS-driven luminance clamp moves 0.45 → 0.55 with a slightly gentler gain (1.6 → 1.4) so brick-walled tracks don't pin every chunk to 1.0 — preserves transient contrast through the loud parts. The Serato-faithful palette's `normaliseColour` floor lifts 0.45 → 0.55; the monochrome palette's intensity floor lifts 0.35 → 0.45.

**SwiftUI overlay** in `apple/Dub/Waveform/WaveformView.swift`: faint zero-crossing hairline (`DubColor.divider.opacity(0.55)`, 1 px) along the amplitude=0 axis — vertical line at mid-width in vertical orientation, horizontal line at mid-height in horizontal (Prep) orientation. Layered under the deck-tinted playhead overlay so the playhead always wins where they cross. Helps the eye read symmetry around silence and gives sparse-waveform sections an anchor.

No FFI changes; no shader uniform changes (everything piggybacks on existing `Uniforms` / `VertexOut`).

---

<a id="m105f"></a>
## M10.5f — Waveform 2× zoom-in

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 0.1 days

The deck-column waveform was too zoomed-out at the M10.5b sizing: ≈ 4 chunks per pixel meant ~6 s of audio crammed into the visible future region, hard to read transient relationships at mix-in time. One-line fix in `apple/Dub/Waveform/WaveformRenderer.swift`: `nonisolated private static let chunksPerPixel: Double = 4.0` → `2.0`. The constant feeds both (a) the renderer's per-frame `chunksVisible` math (drives the M10.4 NDC mapping in `Shaders.metal`) and (b) `WaveformRenderer.secsPerPixel(sampleRate:)` (drives the M10.6a click-scrub gesture's px → secs conversion), so the click-scrub gesture stays calibrated automatically. The change exposed a latent aliasing pattern — see M10.5g for the follow-up. No FFI changes.

---

<a id="m105g"></a>
## M10.5g — Waveform anti-alias + temporal smoothing

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 0.5 days

The remaining ugliness after M10.5e + M10.5f was a **"venetian blind" stripe pattern** between adjacent chunks. Two compounding root causes: (1) M10.5f's 2× zoom-in put each chunk's quad at ≈ 0.5 px tall on the time axis, and the pipeline had **no MSAA**, so amplitude-edge rasterisation stepped in hard integer-pixel jumps; (2) per-chunk min/max are inherently jittery across consecutive chunks at the engine's native 64-sample cadence (≈ 1.45 ms / chunk at 44.1 kHz), so neighbouring rows drew quads with slightly-different widths and a 1–2 px height — the eye sees the row boundaries as stripes.

**Shader fix** in `apple/Dub/Waveform/Shaders.metal`: per-instance vertex stage now reads `chunks[iid-1]`, `chunks[iid]`, `chunks[iid+1]` (clamped at `iid==0` and `iid==chunksVisible-1`) and convolves min / max / rms with a `[1, 2, 1] / 4` Gaussian kernel. The result drives the rendered quad and `VertexOut.rms`; the honest-state `clipping` / `silence` flags continue to read the *raw centre* chunk so a single hot or silent chunk still lights up unattenuated — smoothing is visual-only, never on the depth-of-information surface. The temporal lowpass softens chunk-to-chunk amplitude jitter that the eye reads as stripes, without changing the broad envelope shape the DJ uses to read transients.

**Pipeline fix** in `apple/Dub/Waveform/WaveformRenderer.swift` + `apple/Dub/Waveform/WaveformView.swift`: 4× MSAA enabled end-to-end. New `nonisolated public static let WaveformRenderer.sampleCount = 4` is referenced both by the `MTLRenderPipelineDescriptor.rasterSampleCount` (renderer-owned pipeline) and `MTKView.sampleCount` (host-owned view); Metal validates these match at draw time. Cost on Apple Silicon is negligible — the multisample texture sits in tile memory, the resolve happens at the end of the render pass, no extra command-encoder plumbing required.

The combination produces a continuous, smoothly-shaded envelope at all zoom levels instead of the previous stripe pattern. No FFI changes; no shader uniform changes. The MSAA path and the temporal-smoothing principle both survive into the M10.8 Serato-parity baseline; the post-processing stack added on top by M10.5h–p does not.

---

<a id="m105hp"></a>
## M10.5h → M10.5p — Shader exploration ladder (rolled back in M10.8)

**Status:** shipped iteratively, then **rolled back wholesale in the M10.8 baseline freeze** — see [§M10.8](#m108) for the current shader.

Between M10.5g and the M10.8 freeze the renderer accumulated a deep stack of shader experiments aimed at matching (and at times exceeding) Serato's visual richness. Real-world dogfooding against side-by-side Serato screenshots showed the stack was not converging on a DJ-effective waveform: dense music collapsed into a uniform yellow soup, transients didn't pop, the runtime tuning panel grew several knobs that the operator should never have needed to touch, and each subsequent layer was paying for problems introduced by the previous one. The M10.8 cleanup deleted the entire post-processing ladder (HDR off-screen target, separable Gaussian bloom, ACES tonemap, the `WaveformTuning` `@Published` knob surface and its `WaveformTuningPanel` GUI, the time-domain filtered-peaks ring, the onset-driven brightness layer, the kick-emphasis tint, the dj-landmarks monochrome palette, the various other palettes) in favour of a single-pass Serato-parity shader with calibrated equal-loudness band biases / gains, three perceptually-tuned colour anchors, and a sub-bass-aware quiet greying gate.

These write-ups are preserved here as design archaeology — they document what was tried, what the calibration cost looked like, and why each layer eventually came down. Any future polish work that wants to revisit one of these ideas should read the relevant section first to avoid re-running the same dead end.

### M10.5h — HDR + bloom render pipeline — *shipped, then rolled back in M10.8*

The single biggest visual upgrade in the M10.5 polish ladder. Before: single-pass renderer writing straight to `bgra8Unorm`, fragment colours clamped at 1.0, no headroom for transient overshoot, no post-processing. After: **five-pass HDR pipeline** in the renderer with sub-pixel-accurate MSAA on the offscreen primary, real Gaussian bloom on transient overshoot, and ACES tonemap on the final composite.

**What shipped:** (1) `Shaders.metal` waveform fragment gains an HDR overshoot block — `hdrBoost = in.rms * in.rms * 3.5` multiplies the post-luminance colour with a quadratic curve calibrated against real-music RMS distributions (typical loud mid rms ≈ 0.30 → boost 0.32 → clear halo; transient peak rms ≈ 0.45 → boost 0.71 → strong halo; quiet pad rms ≈ 0.15 → boost 0.08 → faint wash; silence rms ≈ 0.05 → boost 0.009 → no bloom). The overshoot is applied *before* the past-region `pastDim` multiply so past transients still glow proportionally (just dimmer).

**Calibration notes** (six iterations of `bandMix` retuning to land on legible per-band colours — kick / mid / hi-hat across `(1.0, 0.10, 0.05)` / `(0.10, 1.00, 0.15)` / `(0.05, 0.30, 1.00)` near-spectral anchors, double-square ratio amplification with inverse-pink-noise band weighting `× 1.2` mid / `× 1.8` high to compensate for the `dub-spectral` μ-law compression curve and the natural pink-noise spectrum slope, plus a per-band 3-tap `[1, 2, 1] / 4` smoothing kernel on the band-RMS values so adjacent chunks paint the same colour) — preserved in the archived M10.5h plan-of-record. The key empirical finding: the upstream `dub-spectral` μ-law compression (`ln(1 + λ · |X|)`, chosen in M8.1 to stop hi-hats out-voting kicks in the BPM ODF) pulls all band RMS values into a narrow `[~0, ~6]` compressed range; downstream colour mixing must work in compressed space, not linear, or every per-band correction over-compensates and flips the colour distribution. This finding survived into the M10.8 baseline and drives its `bandBias` / `bandGain` calibration.

**Pipeline additions** that came down in M10.8: new shader trio (`fullscreenVertex`, `brightPassFragment`, `gauss1dFragment`, `compositeFragment`), four pipeline states (`waveformPipeline` re-targeted to `rgba16Float` MSAA + `brightPassPipeline` + `gaussPipeline` + `compositePipeline`), four offscreen textures (`hdrPrimaryMS`, `hdrPrimaryResolved`, `bloomA`, `bloomB` at half-res), seven-pass `draw(in:)` (waveform → bright-pass → H-Gauss → V-Gauss → H-Gauss → V-Gauss → composite/tonemap). Memory cost ~12 MB per deck at typical drawable sizes. The post-processing stack is what M10.8 deletes; the per-band-smoothing finding survives.

### M10.5i — Continuous filled envelope — *shipped, then rolled back in M10.8*

Eliminated the "looks like a peak meter, not a waveform" problem from the original M10.4 / M10.5b layout by replacing one instanced-quad-per-chunk draw with two connected `.triangleStrip` draws (one per region — past + future) whose vertices encode `(amplitudeEdge, timeCentre)` pairs. K chunks produce a single C0-continuous filled shape spanning K-1 trapezoidal slices, eliminating the inter-chunk seams. Calibration-A added a second pass per region for the "Serato two-layer envelope" look (outer min/max envelope at exponent 0.55, inner brighter RMS body at exponent 0.35, 1.6× HDR boost on the inner body so ACES tonemap pulls the core toward "white-hot at the centre, hue-saturated at the edges"). The continuous-envelope geometry survives into the M10.8 baseline; the two-layer overlay and HDR boost do not — M10.8 paints a single Serato-style envelope with calibrated low/mid/high colours and no post-processing.

### M10.5l — Onset-driven bloom intensity — *shipped, then rolled back in M10.8*

Promoted the M10.2 "onset glow" bullet onto the M10.5h HDR pipeline. New `dub-peaks` `OnsetDecimator` mirroring `BandDecimator`'s surface, built on the same `SpectralFrameStream` primitive — same FFT-hop cadence (= `samples_per_band_chunk`, default 512 samples), single `f32` `OnsetChunk` per hop carrying the Klapuri-style log-band weighted spectral flux. **Why a sibling of `dub-bpm::onset` rather than re-exporting it:** the renderer needs the onset trail even when no `BpmStream` is running (File-mode playback, single-deck Prep), and tying the renderer to the BPM crate would couple two independent off-RT pipelines.

**`dub-peaks` plumbing:** `PeakBuffer` gains an optional `OnsetStorage` mirror of `BandStorage`; `PeakStreamConfig.onset_enabled` (default `true`) implicitly enables `bands_enabled`; `PeakStream::spawn` drives an `OnsetDecimator` on the analysis thread alongside the existing `BandDecimator`; `compute_offline_peaks` does the same on the file-mode path. **`dub-ffi` surface:** `engine.onset_peaks_len(deck)`, `engine.onset_peaks_chunk_duration_secs(deck)`, `engine.onset_peaks_extend(deck, start)` — same shape as `band_peaks_*` but a 4-byte stride. `PeakSource` enum delegates to the live stream / offline buffer the same way it delegates bands. `FFI_VERSION` 8 → 9.

**Apple renderer:** new `onsetChunksBuffer` ring (sized to `bandChunkCapacity` = 131 072 entries = 512 KB/deck), parallel `ingestNewOnsetChunks` pump, `WaveformUniforms.onsetChunkOffset` (per-region), onset buffer bound at vertex buffer slot 4. **Shader:** vertex stage looks up the onset chunk for each broadband chunk, applies the same 3-tap `[1, 2, 1] / 4` smoothing to the raw flux, maps via the calibrated sigmoid `onsetConf = clamp(1 - exp(-fluxSmoothed × 0.25), 0, 1)`. Forwarded through `VertexOut.onsetConf`; fragment multiplies `hdrBoost` by `(1.0 + 1.5 × onsetConf)`.

M10.8 deletes the renderer-side onset consumption (no more `onsetConf` shader path) but the Rust-side `OnsetDecimator` + `OnsetStorage` + FFI plumbing remains in place for future, additive consumers — exactly the kind of "reversible" architecture the M10.8 guardrail asks for.

### M10.5m(a) — Beat-aware saturation — *shipped, then rolled back in M10.8*

The first half of the originally-planned M10.5m row, lifted out and shipped alongside M10.5l because both effects ride the same `onset_trail` data and live in the same fragment-shader pass. After `bandMix` runs and *before* the palette branch, the shader rotates the bandMix output toward its own Rec. 601 luma based on `onsetConf`: `colour = mix(float3(luma), colour, 0.4 + 0.6 × onsetConf)` — sustained pads desaturate to a wash, kicks/snares paint full vibrant hue. Combined with M10.5l, drum hits + transients pop as saturated colour shapes against a near-monochromatic background of held notes / pads / silence. Rolled back in M10.8; the broader Serato-parity calibration in M10.8 makes the colour distribution legible without this overlay.

### M10.5o — Kick prominence layer (band[1] visual emphasis) — *shipped, then rolled back in M10.8*

Problem: in the M10.5l + M10.5m(a) baseline, a kick chunk and a sustained bassline chunk at the same broadband RMS read as visually indistinguishable — both paint at similar luminance + chroma since the bandMix output is a per-band *ratio* (a chunk dominated by bass paints red regardless of *how much* bass), and the bloom layer only fires on onsets. User wanted the 80–250 Hz "kick range" to **visually stand out** independent of onset / total amplitude.

**Implementation (~50 LoC across 4 files, all behind a single uniform):** new `kickEmphasis: float` in `Uniforms` + matching field in `WaveformUniforms` (taking the struct from 60 → 64 bytes, perfectly filling `uniformStridePerRegion`), sourced from a new `kickEmphasis` `@Published` knob on `WaveformTuning` (default 0.6, range 0.0–1.5) with a corresponding live slider in `WaveformTuningPanel`. Fragment shader applies three combined effects, all multiplied through `kickStrength = clamp(in.bandLow.y × kickEmphasis, 0.0, 1.0)`: (1) saturation override bumping `chromaScale` toward `min(chromaScale + kickStrength × 0.8, 1.0)`; (2) red-orange tint mixing the bandMix output toward `(1.0, 0.30, 0.05)` by `kickStrength × 0.55`; (3) additive HDR bloom `hdrBoost += in.rms × kickStrength × 0.6` gating on `in.rms` so quiet rumble doesn't paint but sustained loud sub-bass *does*.

M10.8 deletes the layer entirely (no `kickEmphasis` uniform, no `WaveformTuning`, no `WaveformTuningPanel`); the broader Serato-parity calibration in M10.8 paints kicks pink-red via its low-band anchor + kick-push logic instead.

<a id="m105n"></a>
### M10.5n — Playhead-vs-audio drift root-cause fix — *shipped, survives M10.8*

**Symptom (reported during M10.5l shakedown):** the audible kick happens slightly before the corresponding chunk crosses the playhead, AND the gap visibly widens as the track plays — small at the start, ~1 s by track-end on a 4-minute track. Initially mis-diagnosed as a steady-state `display_present − audio_buffer` differential (which is real but tiny: 5–20 ms constant) and "fixed" with a manual `avOffsetMs` slider in the tuning panel. The slider masked the problem but didn't solve it — a value tuned at 0:30 is wrong at 3:30 because the actual error is **linear in track time**, not constant.

**True root cause:** peak chunks are cadenced in **track frames** (the offline analyzer in `dub-peaks` produces one chunk per 64 *track* samples), but the renderer was indexing them in **engine frames** with an integer-rounded conversion. The path was: `peaksChunkDurationSecs = 64 / track_sr` (correct, exact, e.g. `64/44100 = 0.0014512 s`) → `samplesPerPeakChunk = round(peaksChunkDurationSecs × engine_sr) = round(69.66) = 70` (the bug — drops 0.35 samples per chunk = 0.49 % per-chunk error) → `chunk = elapsed_secs × engine_sr / samplesPerPeakChunk`. On a 44.1 kHz track / 48 kHz engine the per-chunk error of 0.49 % compounds to **~804 chunks of drift over 240 s of playback ≈ 1.17 s of accumulated visual lag**, exactly matching the reported symptom. (Same-SR tracks — e.g. 48 kHz track on 48 kHz engine — have zero drift because `peaksChunkDurationSecs × engine_sr` is already integer, so the bug was invisible on test fixtures.)

**Fix (~5 LoC in renderer, no FFI change):** bypass the integer-rounded intermediate entirely. Store `peakChunkDurationSecs: Double` in `WaveformRenderer` from the engine's already-exact f64 report, and use it directly: `playheadChunk = floor(elapsed_secs / peakChunkDurationSecs)`. Verified by hand-calculation: on the 44.1 kHz / 48 kHz scenario the new formula matches the engine's actual playback position to within `f64` precision (~1e-9 s). **Slider removal:** `WaveformTuning.avOffsetMs` deleted, "AV sync" section removed from the tuning panel, the `dub.waveform.tuning.avOffsetMs` `UserDefaults` key one-shot-migrated to nil on launch so a `defaults read com.klos.dub` doesn't show a stale value.

The root-cause fix survives the M10.8 cleanup unchanged (still in the renderer).

### M10.5p — DJ-focused waveform redesign — *shipped (Stages 1 + 2 + 3 + 3.1), then rolled back in M10.8*

**Problem statement (user-driven, 2026-05-13):** the M10.5h → M10.5o waveform stack delivered a *visually rich* renderer but a *DJ-ineffective* one. In loud / busy music ("bass + rapping + drums") the 7-band hue mix saturates toward a "yellowish glowing" soup because the per-band ratios all land near-equal once the music is dense. The DJ doesn't need spectral density: they need three landmarks. Quote: *"all DJ music is basically on a 4/4 rhythm. The other thing a DJ needs is to identify the drop (easy since this is mostly after a break and a buildup) and he needs to understand where the vocals come in and where they leave. This is basically all the dj needs from a waveform."*

**Design pivot:** from "data-rich spectral visualisation" → "DJ landmarks only". Stage 1 shipped a monochrome envelope (new `WaveformPalette.djLandmarks`) and an offline beat-grid + tick overlay (`dub-bpm::analyze_beat_grid`, `FFI_VERSION` 9 → 10). The Stage 1 beat-grid overlay was subsequently removed and re-scoped into its own milestone (`M10.5p-grid`, deferred) after testing exposed that fixed-period synthetic grids drift on tempo-drifting material (live recordings, vinyl pressings, breakbeat samples). Stage 2 added transient prominence (kick gate `clamp(band[1].y × onsetConf, 0, 1)`, 0.55 cap on `base` brightness so sustained content stays dim, warm-amber tint on confirmed kicks). Stage 3 added a time-domain `FilteredPeakChunk` ring (`dub-peaks::filtered` module, 2-pole Butterworth LP biquad at 180 Hz on the LF channel, new `samples_per_filtered_chunk` cadence, `FFI_VERSION` 10 → 11) so kick attacks survive intact for a clean kick-vs-sustained-bass discrimination at the shader level instead of fighting the upstream μ-law compression. Stage 3.1 calibrated the filter cutoff, replaced the smoothing kernel on `lfPeak` with `max`-of-3 (smoothing was destroying kick dynamic range), and adjusted the amber-gate to `smoothstep(0.08, 0.30, kickGate) × kickEmphasis` with a brightness-tied amber luminance.

M10.8 rolls back the entire `djLandmarks` palette branch, the beat-grid plumbing in `load_track` Phase 2 (already returning `BeatGrid::empty()` to save load time), the `WaveformTuning` slider surface, the kick-gate fragment-shader logic, and the time-domain `FilteredPeakChunk` ring on the *renderer* side. The Rust-side `dub-bpm::analyze_beat_grid` API, the FFI `BeatGrid` accessor, and the `dub-peaks::filtered::FilteredDecimator` + `FilteredPeakChunk` types remain in place as dormant data primitives — exactly the kind of reversible architecture the M10.8 guardrail asks for. A future, additive M10.8+ milestone can re-light any of them without re-running the Stage 1 → Stage 3.1 calibration tour.

### M10.5p-grid — Beat-grid v2 (tempo drift, downbeat detection, manual phase correction) — *deferred*

The first M10.5p Stage 1 ship bundled an offline beat-grid + tick overlay alongside the monochrome envelope. User testing exposed two issues that pushed the grid out into its own multi-sub-task milestone: (a) the overlay didn't visibly scroll with the playhead on first ship (a `Canvas`-caching bug; subsequently fixed) yet still relied on a static phase that doesn't survive tempo-drifting material, and (b) the "stuck two ticks" symptom revealed the deeper truth — *beat grids only work on tempo-locked production tracks*. Live recordings, classic vinyl pressings (which drift inherently), edits with manual cuts/loops, and tempo-aware DJ tools (Serato Pitch'n'Time, Traktor Flux) all produce material where a fixed-period synthetic grid drifts off the audible beats within bars.

A v2 grid that handles those cases needs: **(g1)** per-beat phase tracking (a Viterbi-style decoder over the ODF rather than a single global phase pick); **(g2)** algorithmic downbeat detection (which beat is "the 1" of each bar — current Stage 1 just calls beat 0 the downbeat, which is wrong for any track that doesn't start exactly on the 1); **(g3)** manual phase correction UI (tap the waveform to nudge the discovered "1"; ⌘⇧← / ⌘⇧→ to shift the grid by ±1 ODF tick; half-tempo / double-tempo toggle for the M8.1 octave-ambiguity edge cases); **(g4)** library sidecar serialisation (compute the grid once, persist it, never recompute on re-load); **(g5)** a Thru-mode streaming variant (the offline `analyze_beat_grid` is file-only — a streaming `BpmStream` already exists but only emits BPM, no phase).

When the grid milestone resurfaces, the one-line revert to re-enable Stage 1's coarse phase finder is documented in `dub-ffi/src/lib.rs` `load_track`. Until then, the waveform helps the DJ with no grid.

### M10.5m(b) — 9-band sub-bass split — *deferred to M11*

The second half of the originally-planned M10.5m row, parked for after M11 (Serato library import) lands so we have a real DJ-curated track set to validate the colour change. **Plan when revisited:** bump `dub-spectral::NUM_BANDS` from 8 to 9 by splitting the lowest log-band into sub-bass (30–60 Hz) and kick-band (60–200 Hz). Touches `dub-spectral` (band-layout constant, FFT-bin-grouping math), `dub-bpm` (every M8.1 genre fixture needs to be re-baked because the per-band magnitudes shift), `dub-peaks` `BandPeakChunk` (wire format gains a 9th f32 — `#[repr(C)]` size 32 → 36 bytes, breaking change for the M10.5j sidecar format → version bump), `dub-ffi` `peaks_extend` wire format documentation, shader `BandPeakChunk` struct + `bandMix`. The compute-side change is mechanical; the data-format breakage is the gnarly part — every dependent crate's tests need re-baselining and the sidecar format gets a `version: u32 = 2` bump with a v1 → v2 migration (drop v1 entries on first run; a one-time re-decode is acceptable in Phase A). `FFI_VERSION` += 1 when it lands.

### M10.5j — On-disk waveform sidecar cache — *planned, not yet shipped*

The "track-load feels instant" upgrade — what Serato (`.SeratoOverview`), Traktor (`.tg2`), rekordbox (`.pdb` + analysis blobs) all do under the hood. Today every track load runs `Track::load_from_path` + `compute_offline_peaks`. M10.5d moved it off the engine mutex, but the work still happens once per load. **Plan:** new `dub-cache` library owning a versioned on-disk format (64-byte LE header + broadband peaks + band peaks + optional mip pyramid + CRC-32 footer), keyed by `sha-256(canonical_path || file_size || mtime_nanos)`, stored under `~/Library/Caches/com.klos.dub/waveforms/`. Lookup flow in `dub-ffi::load_track` Phase 1 stats the audio file, computes the cache key, tries to `mmap` the sidecar. Cache hit → skip decode entirely. Cache miss → decode + compute as today, then atomically write the sidecar via `<key>.tmp` → `<key>.dubpeaks` rename. Disk budget per track ~2.5 MB at 5 min; a 500-track library ≈ 1.25 GB cache (well below Serato's typical 3–5 GB). LRU eviction when the directory exceeds a configurable cap (default 4 GB).

### M10.5k — Mip pyramid in `dub-peaks` — *planned, not yet shipped*

Closes the loop on the final M10.2 deferred polish bullet (Mip pyramids). Today the renderer reads peaks at a single resolution (64-sample broadband cadence) and the `TrackOverviewView` re-decimates to ~300 buckets on the CPU at load — both work but neither lets us *zoom smoothly* or feed a future coarse-zoom view a coarser source. **Plan:** extend `OfflinePeaks` with `pub mips: Vec<MipLevel>` containing 5 levels (level 0 = full cadence, level 1 = ÷2, level 2 = ÷4, level 3 = ÷8, level 4 = ÷16). Same reduction kernel for bands (band RMS is mean-pooled). The M10.5j sidecar gains the mips after the level-0 payload. `TrackOverviewView` drops its CPU decimation entirely and reads mip-4 directly via a new mip-aware `peaks_extend_mip(deck, mip, start_idx)` accessor (`FFI_VERSION` += 1).

---

<a id="m106"></a>
## M10.6a–e — Mouse transport, Panic Play, transport-cluster redesign, Repeat auto-trigger

**Status:** shipped (a, b, c, d, e) &nbsp;·&nbsp; **Estimate:** 3 days for a–d + 0.5 day for e

Engine work concentrated in 10.6b, UI work split across the others. Together they deliver PRD §6.1's mouse-allowed transport, PRD §6.1.2 Panic Play (the **single most important reliability feature** in v1 from a "career night" perspective), and PRD §6.1.3 Casual Play.

### M10.6a — Casual Play UI + zoomed click-scrub

Deck-header transport-glyph cluster (Play/Pause toggle + Restart) added to Row 3 of `DeckHeader` — renders exactly when a file track is loaded (`timeRow != nil`), so it covers both Prep-mode and the Casual-Play-before-Timecode case. New `WaveformAppModel.{restart, scrub}(side:...)` methods plumbed into the header via a `DeckHeaderCallbacks` value (closures kept off `DeckHeaderState` to preserve `Equatable`). `WaveformView(onClickScrubRelativeSecs:)` installs an orientation-aware transparent hit-test layer beneath the playhead overlay; click → signed seconds-from-playhead via the same `chunksPerPixel × samplesPerPeakChunk / sampleRate` ratio the renderer uses, so a click lands on the visual chunk under the cursor. New nonisolated `WaveformRenderer.secsPerPixel(sampleRate:)` helper centralises that math. PRD §6.1 gating: the closure is wired only when `engineMode == .prep`; Timecode-mode panes pass `nil` so the gesture isn't installed at all (no fine-scrub on a timecode-controlled deck, regardless of Panic Play state). No FFI bump (renderer + UI only).

### M10.6b — Panic Play engine + FFI

New `LiftPolicy::force_disengaged()` (preserves `last_locked_rate` while clearing the engaged flag + sticky counter — the next `Locked` is by construction a fresh re-engagement). New engine-level `PanicPlayState { engaged, held_rate }` per deck; `PanicPlayState::normalise_held_rate` collapses negative / near-zero candidates to a positive forward rate per PRD §6.1.2 ("runs the audio track forward"). New `Command::DeckPanicPlay { idx }` / `DeckCancelPanicPlay { idx }`. `Engine::engage_panic_play(idx)` captures the held rate (preferring `LiftPolicy::last_locked_rate()` when a timecode input is attached, falling back to `deck.rate()` otherwise), force-disengages the policy, sets the deck rate + playing, and flips the new `DeckSharedState::is_panic_play` atomic.

`Engine::drive_timecode_inputs` branches on panic state: in panic mode `Locked` intents auto-cancel (clean re-lock = "DJ dropped the needle back on the groove"), `DropoutHoldRate` intents are ignored (the whole point — the deck keeps playing while the needle is off the platter). `Engine::cancel_panic_play(idx)` pauses the deck and clears the flag; idempotent on non-engaged decks. `EngineHandle::DeckCommand::{panic_play, cancel_panic_play}` send the new commands; `DeckSnapshot.is_panic_play` exposes the atomic for the UI. FFI surfaces `panic_play(deck)` / `cancel_panic_play(deck)`; `PositionInfo` gains `is_panic_play` so the existing 30 Hz UI poll picks up the engine state. `FFI_VERSION` 7→8.

**Test coverage:** 11 new tests — 3 policy tests (force-disengaged clears flag + counter, preserves last_locked_rate, requires engage-threshold to re-lock), 8 engine tests (engage from policy, fallback to deck rate, negative/below-floor normalisation, dropout-stays-panicked, Locked-clears-engaged, cancel-pauses-deck, cancel-idempotent, default-disengaged, alloc-free), plus 1 end-to-end test that engages panic and renders synthetic CV02 carrier blocks through `engine.render` to verify the auto-cancel path lands correctly. All 350+ workspace tests still green; clippy `-D warnings` clean.

### M10.6c — Panic Play UI + Timecode overview un-gate

`DeckState.isPanicPlay: Bool` field driven by the existing 30 Hz `PositionInfo.isPanicPlay` poll (engine remains the authority — UI also sets it optimistically on `panic(side:)` for zero-frame latency, but the poll over-writes it every tick so an engine-side auto-cancel on clean re-lock propagates within ≤33 ms). New `WaveformAppModel.{panic, cancelPanic, panicToggle}(side:)` wrap the M10.6b FFI methods with the same error-surfacing path as Play/Pause. `DeckHeaderState` grew `isPanicPlay` + (initially) `panicGlyphVisible` flags and a new `Source.tcHold` variant; `DeckHeaderState.from(...)` derives them: glyph visible iff `thruMode && hasTrack`, `source = .tcHold` when `thruMode && isPanicPlay`. `TrackOverviewView.handleTap` un-gates: the two-deck-Timecode early-return allows the seek when `deckState.isPanicPlay` is true (PRD §6.1 release condition). M10.6c's lifepreserver-glyph + dedicated Restart button were superseded by M10.6d below — the rest of the M10.6c plumbing (model layer, source pill, overview un-gate) stayed and is what M10.6d builds on. No FFI bump for c.

### M10.6d — Transport-cluster redesign + library polish + cancel-doesn't-pause

Fixes the "Play does nothing in Timecode mode" bug at the root: pressing the deck-header Play button in Timecode mode previously called `engine.play` which set `is_playing = true` only to be overwritten by the very next `drive_timecode_inputs` `DropoutHoldRate` block. The fix is to surface Panic Play *as* the Timecode-mode Play affordance — one button, Serato-style INT/ABS toggle. `DeckHeaderState.panicGlyphVisible` renamed to `useTimecodeToggle` to reflect its expanded role. `DeckHeader.transportGlyphs` collapses to a single `primaryButton` that branches: Prep mode → classic Play/Pause via `onPlay` / `onPause`; Timecode mode + track loaded → `onPanicToggle` only, icon flips between `play.fill` (currently following platter — tap to play internally) and `opticaldisc.fill` amber (currently internal — tap to re-engage timecode).

M10.6c's lifepreserver glyph is gone (subsumed) and the M10.6a Restart button is gone (overview click-to-top covers it, PRD §6.1.3).

**Engine semantics tweak:** `cancel_panic_play` no longer pauses the deck — it clears the engaged flag + atomic and hands transport authority back to the timecode driver. A healthy carrier produces an immediate Locked re-lock (deck stays audible, true INT→ABS hand-back). A silent carrier yields `DropoutHoldRate` on the next block which pauses the deck via the existing arm — same outcome as the pre-M10.6c "pause on held position" path, without the race against the next Locked sample. `Command::DeckCancelPanicPlay` / `EngineHandle::cancel_panic_play` / FFI `cancel_panic_play` doc comments updated. `WaveformAppModel.cancelPanic(side:)` no longer optimistically sets `isPlaying = false`; the next 30 Hz poll reflects whatever the engine decides.

Replaced engine test `cancel_panic_play_pauses_deck_and_clears_shared` with `cancel_panic_play_clears_state_and_leaves_transport` + added 2 new tests: `cancel_panic_play_then_locked_intent_keeps_deck_playing` (synthetic CV02 carrier through `engine.drive_timecode_inputs` after cancel → deck stays playing at platter rate), `cancel_panic_play_then_silence_pauses_deck_via_dropout_path` (silent ringbuf → DropoutHoldRate → deck pauses naturally).

**FileBrowser polish:** folders now require **double-click** to descend (single-click was too easy to trigger by accident while scanning); the drag-out preview is a small `waveform` glyph instead of the row's full song-name text. Workspace `cargo test` clean, clippy clean, xcodebuild clean. No FFI bump (Phase A pragmatism — behavior change, same signatures).

### M10.6e — Repeat (LFSR run-out auto-trigger)

PRD §5.4.2 in its final form: **Repeat is automatic, not user-controllable.** Reached the realisation in M10.6e planning that what every commercial DVS app does on run-out is the *same engine state* §6.1.2 reaches via the user-triggered INT/ABS toggle (M10.6d) — there is no separate "Repeat mode" with its own user surface; there is one Panic Play state with two entry points (user-triggered, auto-triggered). The PRD's earlier framing of Repeat as a per-deck toggle was wrong; ship clarified prose alongside the engine change.

**Engine change (1 arm in `Engine::drive_timecode_inputs`):** the non-panic `LiftIntent::DropoutHoldRate` arm previously paused the deck (M10.6c/d "DropoutHoldRate pauses the deck via the existing arm"); M10.6e replaces that with `self.engage_panic_play(idx)` so a sustained dropout (run-out groove, signal degradation past the `LiftPolicy` grace window) continues forward at the last-known velocity instead.

**Boot-time guard.** Auto-engage only fires when `self.decks[idx].is_playing()` at the moment `DropoutHoldRate` arrives. At engine boot the input ring is fed silence before the DJ has touched the platter; without the guard the very first dropout would auto-engage and start the deck running at the policy's default held rate against the operator's intent. A paused deck on `DropoutHoldRate` keeps its rate (so a future Locked can pick up correctly) but stays paused. PRD §5.4 Stickiness's grace window inside the policy already covers brief stylus hiccups before `DropoutHoldRate` is ever emitted.

**Recovery is the M10.6b auto-cancel-on-clean-Locked path,** unchanged: when the DJ drops the needle back on a mid-timecode groove, the next clean Locked block in the panic branch clears the engaged flag and timecode authority resumes. One state, two entry points, one recovery path.

**No FFI bump, no Apple-side work.** The existing 30 Hz `PositionInfo.isPanicPlay` poll (M10.6c) already surfaces the engine state to the UI, and the M10.6d INT/ABS toggle's icon mapping derives from the same `isPanicPlay` field — auto-engaged panic shows up correctly with no further work. Apple shell builds clean against the engine change without a single line of Swift change.

**Cancel-into-silence semantics changed.** Pre-M10.6e (M10.6d): a user-cancel against a silent carrier paused the deck via the DropoutHoldRate path. Post-M10.6e: the same DropoutHoldRate re-engages panic, so cancel-into-silence is a visual no-op (toggle flickers, audio continues forward). Documented in PRD §5.4.2 as intentional — there is no "paused" state in Timecode mode after run-out by design, matching Serato Scratch Live and Traktor Scratch.

**Test coverage:** 3 new / replaced tests in `dub-engine`:

- `dropout_hold_rate_auto_engages_panic_per_m10_6e` — headline test. Deck playing under timecode, push silence, drive 4 blocks → deck stays audible, `panic_play_states[idx].engaged == true`, shared atomic flipped.
- `cancel_panic_play_then_silence_re_engages_panic_per_m10_6e` — replaces M10.6d's `cancel_panic_play_then_silence_pauses_deck_via_dropout_path`. Same setup, inverted assertion: M10.6e re-engages panic on the post-cancel dropout instead of pausing.
- `dropout_auto_engaged_panic_auto_cancels_on_clean_relock` — recovery path. Auto-engage via dropout, then drive a Locked above engage threshold → panic auto-cancels and the deck follows the new rate. Confirms the user-triggered and auto-triggered entry points share one recovery path.
- The pre-existing `timecode_silence_pauses_deck` test still passes under the new behaviour because the boot-time guard (`deck.is_playing() == false`) correctly suppresses auto-engage at boot.

Workspace `cargo test --workspace` clean (628+ tests); `cargo clippy --workspace --all-targets -- -D warnings` clean.

**PRD churn that landed alongside:**

- §5.4.2 rewritten: Repeat is automatic, no toggle, no user-facing state. Lists the two entry points into Panic Play and documents the cancel-into-silence visual no-op. The earlier "per-deck toggle; trigger surface TBD — see §5.5" framing is gone.
- §5.4 Stickiness bullet updated: the grace window is the brief-hiccup discriminator; sustained dropouts engage Panic Play automatically. Reconciles long-standing prose drift between PRD §5.4 ("engage internal playback at the last pitch until signal returns") and the pre-M10.6e implementation (which paused).
- §5.4 "Through groove handling" bullet updated similarly.

---

<a id="m107"></a>
## M10.7 — Phase-Drift Trail

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 5 days

Dub's headline beat-matching aid (PRD §9.4). New `dub-match` crate (sibling of `dub-bpm` / `dub-peaks` / `dub-spectral`): a `MatchStream` analysis thread consumes both decks' `dub-bpm` ODFs off-RT, computes a rolling cross-correlation over ≈ 2 bars with a ±1-beat lag window (~40 lag candidates × 200 frames per update), emits `MatchSample { phase_ms, confidence, timestamp }` at 30 Hz to an SPSC ring. Audio-thread cost: **zero** (ODFs already running).

**FFI:** `matchExtend(start_idx) -> Vec<u8>` mirroring `peaks_extend`. **UI:** `apple/Dub/Performance/PhaseDriftView.swift` — Metal-rendered vertical strip ≈ 80 px wide in the centre gutter, time **bottom→top** matching the waveform direction discipline (PRD §9.1 / §9.4), dot brightness = confidence, dot colour blended from deck tints; numeric overlays `Δ BPM = +0.3` (top, slope-derived) and `Δ ms = +12` (bottom, instantaneous). Grid-agnostic by construction; degrades gracefully (dim dots) when ODFs are weak. `FFI_VERSION` bumps to 9.

**Single mode only — no Preferences toggle for "numeric-only" variant in v1.**

---

<a id="m108"></a>
## M10.8 — Track Preparation Mode shell + Serato-parity waveform baseline freeze

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2 days for the Prep shell + 1–2 days for the baseline freeze and cleanup

Two concurrent ships consolidated under M10.8: the Track Preparation Mode shell (the long-planned single-deck horizontal-waveform alternate root view), and the **Serato-parity waveform baseline freeze** that resolved the visual dead-ends of the M10.5h–p shader exploration ladder.

### Track Preparation Mode shell

Auto-detection of available audio interface at launch (PRD §3.1). If no multi-channel interface present, the app boots into Track Preparation Mode — alternate root view (`apple/Dub/Prep/PrepView.swift`) hosting a single-deck **horizontal** waveform full-width, with the whole-track overview band stacked above it, and the library prominent below. Manual override in Preferences (`Mode: Auto / Performance / Preparation`).

**Shell only:** the mode renders the chrome and supports load + play / pause; **no** beatgrid editor, **no** hot-cue prep, **no** track gain tweaking yet (those are v1.x per PRD §3 — they'd substantially expand v1 scope and the user explicitly chose the shell-only option in the M10.3 planning round). The mode's *purpose* is visible from M10.8; its *tooling* arrives in v1.x.

Sizing constants live in `apple/Dub/DesignSystem/Tokens.swift`:

- `DubLayout.deckColumnWidth = 80` — Performance (Timecode) mode zoomed column.
- `DubLayout.waveformPrepHeight = 140` — Prep mode horizontal zoomed strip.
- `DubLayout.deckOverviewHeight = 60` — Prep mode horizontal overview band stacked above the zoomed strip.
- `DubLayout.deckOverviewWidth = 36` — Performance mode vertical overview column on each deck's outside edge (unchanged from M10.5c).

`TrackOverviewView` is now orientation-aware (its `OverviewSizing` `ViewModifier` picks vertical-column vs horizontal-band sizing from a `WaveformOrientation` property); `PerformanceView` derives `waveformOrientation` from `engineMode` and stacks the Prep overview band horizontally above the playing waveform. `WaveformAppModel.palette` defaults to `.serato`.

### Serato-parity waveform baseline freeze

The M10.5h–p shader exploration ladder (HDR, bloom, ACES tonemap, onset-driven brightness, kick-emphasis tint, dj-landmarks palette, time-domain `FilteredPeakChunk` ring, the `WaveformTuning` `@Published` knob surface and its `WaveformTuningPanel` GUI) was rolled back wholesale in favour of a single-pass Serato-parity shader that matches the visual reference the user repeatedly compared Dub against (the Westside Connection breakdown / drop screenshot referenced through the M10.5p session).

**Current shader characteristics** (frozen baseline, see PRD §9.6.0):

- **Height** comes from per-pixel-column broadband `PeakChunk` max aggregation (the vertex shader aggregates `chunksPerColumn = 2` chunks per visual column at the Performance-mode `chunksPerPixel = 2` zoom, producing a `pixelsPerDrawnColumn = 2` strip with the visible-future-region transients visually doubled vs the M10.5b sizing).
- **Colour** comes from 8 log-spaced `dub-spectral` bands grouped into calibrated low / mid / high channels in the **log-compressed domain** (`bandBias` `float3(9.45, 7.75, 5.75)`, `bandGain` `float3(1.00, 0.82, 1.00)` — these are the M8.1 μ-law-curve domain values, not linear amplitudes; this was the load-bearing finding from the M10.5h calibration tour).
- **Anchors** tuned against the Serato reference: `lowColor = (1.00, 0.12, 0.24)` pink-red kicks, `midColor = (0.08, 0.94, 0.22)` green mid / presence instruments, `highColor = (0.58, 0.36, 1.00)` lavender hi-hats. Mixed via `weights = pow(saturate(calibrated / chromaMax), 1.45)` — the 1.45 power enhances the dominant band so two-band content reads as a clear blend rather than a muddy secondary.
- **Quiet greying** is gated by broadband amplitude (`in.peak`) **and** sub-bass focus (`in.subBass`, carrying `b0` ≈ <80 Hz at 44.1 kHz) **and** weak audible mid/high (`audibleMidTop`). Three-axis gating prevents the early single-axis attempts (broadband-only, then spectral-low-only) from greying out audibly significant mid-range content while still greying decay tails of sub-bass-only sections (mirrors what Serato does on the same reference clip).
- **Kick push:** loud low-band transients (`smoothstep(0.18, 0.42, in.peak) * smoothstep(0.25, 1.10, calibrated.x)`) boost `calibrated.x *= 1.35` and dim `calibrated.y *= 0.78`, ensuring kicks paint pink-red rather than drifting toward orange/green even when the mid/high bands also have content.
- **MSAA** stays at 4× (M10.5g, survives).
- **No HDR, no bloom, no tonemap, no `WaveformTuning` runtime knobs.** Single-pass renderer writes straight to `bgra8Unorm`.

The previous palette presets and `WaveformPalette` enum are gone; the per-track-palette state in `WaveformAppModel` is fixed at `.serato` and the Preferences `paletteSection` has been removed.

### Future-work guardrail (PRD §9.6.0)

Future waveform work must be **additive and reversible** relative to this baseline:

- Do not reintroduce the removed HDR / bloom / tuning-panel stack in-place.
- Do not rewrite the baseline shader without first preserving this version behind a small, explicit switch or an isolated follow-up commit.
- If a polish experiment fails, reverting that experiment should return exactly to this M10.8 baseline.

The Rust-side `OnsetDecimator`, `BeatGrid`, and `FilteredDecimator` data primitives remain available for future, additive consumers without re-running their calibration tours.

### Commit boundary

The freeze was committed as `4a31363` (`feat(apple,engine): freeze M10.8 waveform baseline`). The corresponding PRD additions live in [§9.6.0](PRD.md#960-waveform-baseline-freeze-m108-cleanup) and the sizing table in [§9.6.1](PRD.md#961-sizing).

---

<a id="m11a"></a>
## M11a — Library schema + path-by-volume-UUID

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2 days &nbsp;·&nbsp; **Actual:** 1 day

M11a is the foundation under all subsequent library work. It stands up the SQLite schema, the migration runner, the on-disk locations, and the macOS volume-UUID discovery primitive. No importers and no UI yet; those land at M11c (filesystem importer) and M11d (browser shell). The deliberate sequencing is that **every later milestone reads and writes against a frozen, documented schema** rather than refactoring the table set in the middle of importer work.

### What shipped

**`dub-library` crate** (previously a stub with the `Source` enum and nothing else).

* **`schema::open_and_migrate`** — connection-level PRAGMA bootstrap (`journal_mode = WAL`, `synchronous = NORMAL`, `foreign_keys = ON`, `temp_store = MEMORY`, `mmap_size = 256 MB`) followed by the migration runner. Each migration applies inside its own transaction; idempotent via `CREATE TABLE IF NOT EXISTS` so a crash mid-migration is safe.
* **v1 schema, 16 tables + 1 FTS5 virtual table:** `schema_version`, `fingerprints`, `tracks`, `volumes`, `track_files`, `track_metadata_source`, `track_beatgrids`, `track_cues`, `track_loops`, `crates`, `crate_tracks`, `imported_crates`, `imported_crate_tracks`, `play_history`, `analysis_cache`, `smart_crates`, `track_metadata_fts`. Plus 11 supporting indexes and 3 FTS5 sync triggers. Byte-for-byte mirror of the `CREATE TABLE` blocks in `docs/LIBRARY-SCHEMA.md`.
* **`SCHEMA_VERSION = 1` baseline + write guard.** A DB on disk at `schema_version > 1` raises `LibraryError::SchemaTooNew { found, supported }`; v1.0 binaries refuse to write older-version DBs per PRD §8.7 ("What v1.0 does not commit to" — forward compatibility from a v1.0 binary opening v1.x DBs).
* **Partial unique index** `idx_one_active_grid_per_track ON track_beatgrids(track_id) WHERE is_active = 1` enforces "exactly one active grid per track" at the SQL level rather than relying on importer discipline.
* **`Library::open_default`** resolves to `~/Library/Application Support/Dub/library.sqlite`; **`Library::open_at`** for tests and CLI tools; **`Library::open_in_memory`** for unit tests that don't touch the disk. Parent directory chain auto-created.
* **macOS volume UUID discovery** via direct `getattrlist(2)` syscall with `ATTR_VOL_INFO | ATTR_VOL_UUID`, paired with `statfs(2)` for the mount-point and display-name fields. Implemented inline (≈100 lines, the only unsafe code in the crate) rather than pulling Foundation / CoreFoundation into the workspace just for one read. `DiscoveredVolume { volume_uuid, mount_point, display_name, is_internal }` is the surfaced type.
* **`Library::upsert_volume`** with `ON CONFLICT(volume_uuid) DO UPDATE`. Re-mounting the same external drive at a different mount point (the typical case when a USB slot moves) updates `last_known_mount_point` and `last_seen_at` without duplicating the row. The UUID is the stable identity; the mount point is descriptive.
* **`docs/LIBRARY-SCHEMA.md`** published as the **public API contract** per PRD §8.7. Documents the migration policy, the backward-compatibility contract, every table's columns and constraints, the soft-enum allowed values for `source` / `event_type` / `kind`, the Chromaprint fingerprint parameters (algorithm 2, 11025 Hz mono, full-track window, raw `uint32_t[]` storage), the FTS5 tokenizer (`unicode61 remove_diacritics 2`), the connection-level PRAGMAs, and three canonical query examples (browser default sort, FTS substring search, Played-From-Played-Into aggregation).

### What was descoped from M11a (and why)

The original M11a row in PRD §12 listed "M10.5j sidecar key migrated from path-hash to canonical fingerprint" as a sub-deliverable. This is **deferred** because the M10.5j sidecar cache itself has not been built yet (it's still *planned* in §12). The new schema's `analysis_cache.waveform_sidecar_path TEXT` column reserves the right shape for the future migration; when M10.5j actually lands, it can be born fingerprint-keyed in one shot rather than path-hash-then-rewritten.

This is not laziness; it is the right boundary. Shipping a "migrate from format X to format Y" deliverable before format X exists would produce dead code that is hard to test against and obscures M11a's actual contribution (the schema + path-by-volume-UUID model).

### Tests (16 new, all green)

* `schema::migration_to_v1_lands_schema_version_row` — DB is at `SCHEMA_VERSION` after a fresh `open_and_migrate`.
* `schema::migration_is_idempotent_on_re_open` — running the runner twice on the same DB is a no-op.
* `schema::migration_creates_every_documented_table` — spot-check that every table from `docs/LIBRARY-SCHEMA.md` exists. Catches typos in the embedded SQL at unit-test time, not at first-import time.
* `schema::refuses_to_open_db_with_newer_schema` — simulates a v1.x DB opened by a v1.0 binary; asserts `LibraryError::SchemaTooNew { found: 999, supported: 1 }`.
* `schema::fts_trigger_propagates_metadata_inserts` — load-bearing for M11d browser search; an `INSERT INTO track_metadata_source` lands a corresponding FTS5 row that responds to substring `MATCH`.
* `schema::one_active_grid_per_track_constraint_holds` — partial unique index rejects a second `is_active = 1` row for the same track.
* `db::open_at_creates_parent_directory_chain` — `Library::open_at(path)` creates `path`'s parent directories via `create_dir_all`.
* `db::re_open_is_idempotent` — closing and reopening the same on-disk DB succeeds.
* `db::upsert_volume_round_trip` — round-trip a volume through `upsert_volume` + `find_volume`.
* `db::upsert_volume_updates_mount_point_on_remount` — same UUID at a different mount point updates in place; no duplicate row.
* `volumes::macos::home_dir_resolves_to_some_uuid` — discovers a UUID for the user's home directory at 36-char RFC 4122 form.
* `volumes::macos::root_path_resolves_internal` — `/` resolves to `is_internal = true` with display name `"Macintosh HD"`.
* `volumes::macos::uuid_round_trip_formatting` — 16-byte UUID format converts to canonical hyphenated lowercase string.
* `tests::version_is_nonempty`, `tests::sources_are_distinct`, `tests::source_strings_match_schema_check_constraints` — sanity tests on the `Source` enum and its `as_str()` mapping to schema enum strings.

Workspace test count: **574 → 588** (`+14` net; 16 new in `dub-library`, 2 of which replace prior stub tests via expanded coverage). Workspace clippy clean (`-D warnings`) on first pass; one cosmetic `cmp_owned` lint fixed during M11a iteration.

### Workspace dependency additions

| Crate | Version | Why |
|---|---|---|
| `rusqlite` | `0.32` with `bundled` + `blob` | The de-facto Rust SQLite binding. `bundled` compiles SQLite from source rather than depending on the host's version (macOS ships ancient builds in `/usr/lib`); deterministic across machines, ~5 s extra first-build. `blob` is needed for `chromaprint_blob` storage at M11b. |
| `uuid` | `1` with `v4` | Canonical track identity. v4 (random) is universally recognised; v7 (time-ordered, better index locality) is parked as a future consideration if benchmarks demand it. |
| `dirs` | `5` | Platform-correct user-config and user-cache directories. macOS resolves to `~/Library/Application Support/` and `~/Library/Caches/` respectively. |
| `libc` | `0.2` (macOS only) | `statfs(2)` binding for the volume discovery path. `getattrlist` is bound inline via a small `extern "C"` block; libc doesn't expose it. |
| `tempfile` | `3` (dev-dep) | Fixture DBs for `Library::open_at` tests without polluting the developer's real library. |

### Out of scope for M11a (queued for M11b onward)

* Chromaprint FFI wrapping (M11b).
* Filename-pattern parser per PRD §8.4 (M11c).
* The walk-a-folder importer + ID3 reader (M11c).
* Browser UI (M11d) — `apple/Dub/Performance/FileBrowserView.swift` continues to use the M10.5b code path until M11d's replacement.
* `apple` shell integration of `dub-library` via the UniFFI surface (slated for the M11d shell rollout; M11a's FFI surface is empty by design).
* Any actual data writes beyond `volumes` upserts — M11a stands up the schema; M11b–M11f populate it.

### Forward shape

M11b will introduce `dub-fingerprint` (Chromaprint FFI, LGPL-2.1, leaf-crate license isolation per PRD §11) and the version-aware dedupe logic that consumes the fingerprint plus the per-source metadata rows. M11a's schema already has the `fingerprints` table and `tracks.fingerprint_id` reference; M11b lands the writers.

### Commit boundary

This entry corresponds to the M11a commit set. PRD §8 + §12 (M11a row marked shipped), `docs/LIBRARY-SCHEMA.md` (new), and `crates/dub-library/` (new crate body) are all part of the same logical change.

---

<a id="m11b"></a>
## M11b — Canonical fingerprint + version-aware dedupe

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2 days &nbsp;·&nbsp; **Actual:** ~1 day

M11b lands the fingerprint and dedupe primitives that gate every importer (M11c filesystem, M11e Serato, M12a–c Traktor / rekordbox / iTunes) from creating duplicate `tracks` rows or, much worse, silently collapsing distinct version cuts. The §8.1 dedupe decision is now a pure function with a clear contract; importers feed it and act on the outcome.

### Architectural pivot: pure-Rust Chromaprint

The single notable architectural decision in M11b: **`dub-fingerprint` ships on `rusty-chromaprint` (pure-Rust, MIT/Apache)** rather than an FFI binding to the reference C `chromaprint` library (LGPL-2.1). The PRD originally documented the LGPL FFI path; M11b changed the call.

**Rationale (parallels M7.5's `dub-bpm` over aubio):**

* **License isolation.** Pure-Rust removes the LGPL-2.1 boundary from the build graph entirely. We are already GPL-3.0 because of Rubber Band (PRD §11) and the LGPL boundary is compatible, but eliminating it simplifies the license story and reduces the "what dynamically-linked library do we ship" surface.
* **No C build dependency.** macOS ships ancient `chromaprint` builds via Homebrew (when present at all); reproducing the bundled-source model we use for `rusqlite` would mean compiling the C library as part of the workspace build, with its own FFmpeg subdependencies.
* **No unsafe FFI surface.** `rusty-chromaprint` is safe-only; `dub-fingerprint` is now `#![forbid(unsafe_code)]`.
* **Use case fit.** Dub uses fingerprints for library-internal similarity-based dedupe, not AcoustID database lookup. Cross-implementation bit-identity to the C library is not load-bearing for us; the generator's self-consistency is.

The Chromaprint **algorithm** is unchanged (algorithm 2, the same one AcoustID uses). The schema's `chromaprint_blob` storage format is the same little-endian `u32` array. A third-party tool reading `library.sqlite` can re-derive Dub's fingerprints with any algorithm-2-faithful implementation; documented in the updated `docs/LIBRARY-SCHEMA.md` § "Fingerprint parameters" and PRD §10.2.

Documentation pivots: PRD §10.2 dependency table (chromaprint LGPL row replaced with `rusty-chromaprint` MIT/Apache row), PRD §5.2.5 (real-record recognition references the pure-Rust path), PRD §10.1 crate-layout entry for `dub-fingerprint`, PRD §12 M11b row, `AGENTS.md` external-libraries section, `AGENTS.md` repo-layout entry, `docs/LIBRARY-SCHEMA.md` § "Fingerprint parameters".

### What shipped — `dub-fingerprint` crate

Previously a stub with a `VERSION` const and nothing else. Now ships:

* **`Fingerprint`** — owns a `Vec<u32>` of Chromaprint items plus the source `duration_ms`. Duration is captured at compute time from the input sample count rather than from any per-file metadata field (which can disagree across encodings).
* **`Fingerprint::compute(samples: &[i16], sample_rate, channels)`** — direct path matching `rusty_chromaprint`'s native input type.
* **`Fingerprint::compute_from_f32(samples: &[f32], sample_rate, channels)`** — convenience path with saturating `[-1.0, 1.0]` → `[i16::MIN, i16::MAX]` clamp. NaN / out-of-range values clamp rather than wrap. Off-RT by construction; allocations are fine.
* **`Fingerprint::to_blob() / from_blob(bytes, duration_ms)`** — round-trip serialisation to little-endian `u32`s for the `fingerprints.chromaprint_blob` column. `from_blob` rejects empty input and any byte length not a multiple of 4, with a typed `FingerprintError::MalformedBlob`.
* **`similarity(&Fingerprint, &Fingerprint) -> f32`** — sliding-window Hamming-distance similarity in `[0.0, 1.0]`. Window radius ±30 items (≈ ±3.7 s) absorbs encoder start-offset variation. Overlaps shorter than 8 items return 0.0 (anti-noise floor). PRD §8.1 dedupe threshold is `≥ 0.98`.
* **`similarity_with_window(a, b, window_items)`** — same with explicit window radius. Used by v1.1 real-record recognition where the turntable start position is arbitrary and the window must be larger.
* **`FingerprintError`** — typed errors via `thiserror`: `InvalidInput` (rate / channel rejected at `start()`) and `MalformedBlob` (BLOB deserialisation).

12 unit tests + 1 doctest, all green. Coverage: deterministic compute (`same_input_produces_identical_fingerprint`), validity of similarity at zero offset and at shifted alignment, distinct-recording floor (two pure tones at 220 vs 1760 Hz score below the 0.98 dedupe threshold), full BLOB round-trip (`to_blob` → `from_blob` → identical), every error path.

### What shipped — `dub-library::version_tokens`

The §8.1 / §8.4 version-token parser. Recognises the full v1 vocabulary (22 canonical tokens; PRD §8.1 wording):

```text
clean, dirty, explicit, instrumental, acapella, radio, edit, extended,
club, dub, vip, remix, remaster, mono, stereo, intro, outro, short,
long, 7in, 12in, lp
```

**Recognition rules** (priority order):

1. Parenthesised segment at the end of the title: `Lady (Clean).mp3`.
2. Square-bracketed segment at the end of the title: `Lady [Dirty].mp3`.
3. Trailing ` - TOKEN` segment before the file extension: `Song - Instrumental.mp3`.

Recognition is **case-insensitive but word-boundary-strict**. Phrase-level patterns (`radio edit`, `extended mix`, `club mix`, `clean version`, `instrumental version`, `vip mix`, `lp version`, `7" mix`, `12 inch`, etc.) are matched before single-word fallbacks and normalise to their head token. Common misspellings (`acappella`, `accapella`) are recognised. Extensions ≤ 5 ASCII-alphanumeric chars are stripped before scanning so `J Dilla feat. Madlib - Track.mp3` parses as a title containing `feat.` rather than mistakenly stripping `Madlib - Track` as the extension.

**False-positive guards (load-bearing tests):**

* `Clean Bandit - Symphony.mp3` → no tokens (artist name in unbracketed prefix).
* `Radiohead - Karma Police.mp3` → no tokens.
* `Nine Inch Nails - Hurt.mp3` → no tokens.
* `Dirty Vegas - Days Go By.mp3` → no tokens.
* `Dirty Dancing OST.mp3` → no tokens.
* `Radio Department - Pulling Our Weight.mp3` → no tokens.

The price for this strictness: exotic naming schemes (`Song.Clean.mp3`) don't get tagged. Per PRD §8.1 those land in the "no token, rely on fingerprint + duration" path, which is correct behaviour (no false-positive merge, just no token to disqualify either).

16 unit tests cover the recognition rules and the false-positive guards.

### What shipped — `dub-library::dedupe`

Pure-function dedupe decision per PRD §8.1. No I/O; the caller supplies a candidate (the file being imported) and an existing-track side (the row whose fingerprint was returned as a near-match by the SQLite lookup) and gets back a `DedupeDecision`.

```rust
pub enum DedupeDecision {
    Merge,
    SiblingVersion { reason: SiblingReason },
    Distinct,
}

pub enum SiblingReason {
    DurationDelta { delta_ms: u32 },
    VersionTokenMismatch {
        candidate_tokens: BTreeSet<VersionToken>,
        existing_tokens: BTreeSet<VersionToken>,
    },
}
```

Auto-merge fires only when **all** of:

1. `dub_fingerprint::similarity(...) >= SIMILARITY_THRESHOLD` (`0.98`).
2. Duration delta `< DURATION_DELTA_MS` (`200` ms).
3. Parsed version-token sets are equal (which includes both-empty).

Otherwise `SiblingVersion` with a reason, or `Distinct` (below similarity floor). The caller wires `Merge` to "add a `track_files` row against the existing `tracks.id`", `SiblingVersion` to "new `tracks` row with `duplicate_link_track_id` set", `Distinct` to "new `tracks` row, no link". M11c filesystem importer will be the first consumer.

9 unit tests cover the truth table:

* Distinct recordings → `Distinct`.
* Same recording + same tokens → `Merge`.
* Same recording + `(Clean)` vs `(Dirty)` → `SiblingVersion::VersionTokenMismatch` (the load-bearing test for PRD §8.1's "the cost of silently collapsing 'Clean' and 'Dirty' is 'the DJ played the explicit version at a wedding'").
* Same recording + `(Clean)` vs `(Instrumental)` → `SiblingVersion::VersionTokenMismatch`.
* `(Radio Edit)` vs `(Extended Mix)` (different durations) → `SiblingVersion` (either reason; both fire, the test asserts the disqualifier without caring which gate caught it).
* Duration delta of 150 ms (within threshold) + matching tokens → `Merge`.
* Duration delta of 200 ms (at threshold) → `SiblingVersion::DurationDelta` with `delta_ms = 200`.
* Both-empty token sets are not a mismatch → `Merge` when similarity + duration allow.
* One-sided token (`Lady (Clean).mp3` vs `Lady.mp3`) → `SiblingVersion::VersionTokenMismatch`.

### What shipped — `dub-library::Library` extensions

The `Library` handle gained two methods that write into the M11a `fingerprints` table:

* **`upsert_fingerprint(&Fingerprint, sample_rate, channel_count, file_size) -> Result<i64>`** — inserts a fingerprint row, returns the rowid that `tracks.fingerprint_id` will reference. We do *not* SQL-level dedupe the `fingerprints` table itself; the collapsing happens at the `tracks` layer via the §8.1 dedupe decision (two near-identical fingerprints can correspond to two distinct `tracks` rows when version tokens disagree).
* **`load_fingerprint(id) -> Result<Option<StoredFingerprint>>`** — materialises the existing-track side of a near-match comparison. `StoredFingerprint` carries the deserialised `Fingerprint` plus the sample-rate / channel-count / file-size columns the M11c filesystem scanner and M11e Serato importer will need.

2 new unit tests cover the round-trip and the not-found case.

### Tests (38 new, all green)

* `dub-fingerprint`: 12 unit tests + 1 doctest.
* `dub-library::version_tokens`: 16 unit tests.
* `dub-library::dedupe`: 9 unit tests.
* `dub-library::db` (fingerprint methods): 2 unit tests.

Workspace test count: **588 → 626** (`+38`). Workspace clippy clean (`-D warnings`) after fixing one cosmetic `manual_is_multiple_of` lint flagged on the BLOB length check.

### Workspace dependency additions

| Crate | Version | Why |
|---|---|---|
| `rusty-chromaprint` | `0.3` | Pure-Rust port of Chromaprint algorithm 2. See §10.2 pivot rationale. Transitively pulls `rubato 0.16` (already a transitive dep elsewhere) for its resampler. |

`dub-library` also gained a direct dependency on `dub-fingerprint`; `lib.rs` re-exports `Fingerprint`, `similarity`, and `FingerprintError` so callers don't have to add `dub-fingerprint` to their own `Cargo.toml` just to construct a `DedupeInput`.

### Out of scope for M11b (queued for M11c onward)

* The filesystem scanner that *uses* the fingerprint pipeline to populate the library (M11c).
* The ID3 / filename metadata reader (M11c).
* The §8.4 filename-pattern parser that maps `ARTIST - TITLE (VERSION) [YEAR].ext` to the metadata fields (M11c). M11b's `version_tokens::parse` is a building block for that parser, not the whole parser; the filename parser will compose token-recognition with the artist/title/year split logic.
* `analysis_cache` row population (M11c — LUFS-I, true-peak, waveform sidecar pointer come from the analysis pipeline, not the fingerprint compute).
* Source-app importer paths (M11e Serato, M12a–c Traktor / rekordbox / iTunes).
* Apple-side surfacing of "potential duplicate" link glyphs in the browser (M11d).

### Forward shape

M11c is now unblocked. It walks a folder, decodes audio via `symphonia` (already wired in workspace), computes a fingerprint per file via `Fingerprint::compute_from_f32`, calls `Library::find_fingerprint_neighbours` (to be added at M11c, indexed first-pass on duration delta), runs `dedupe::decide` against each near-match, and acts on the outcome (`upsert_fingerprint` → register `tracks` row → register `track_files` row, or attach to an existing track). The M10.5b ID3-streaming code path is replaced wholesale.

### Commit boundary

This entry corresponds to the M11b commit set. PRD §5.2.5 + §8.2 + §10.1 + §10.2 + §12 (M11b row), `AGENTS.md` (key external libraries + repo layout), `docs/LIBRARY-SCHEMA.md` (Fingerprint parameters section), `crates/dub-fingerprint/` (new crate body), and `crates/dub-library/` (version_tokens, dedupe, db extensions) are all part of the same logical change.

---

<a id="m11c"></a>
## M11c — Filesystem importer + filename parser

**Status:** shipped &nbsp;·&nbsp; **Estimate:** 2–3 days &nbsp;·&nbsp; **Actual:** ~1 day

The end-to-end walk-a-folder importer that closes the loop between the M11a schema, the M11b fingerprint + dedupe primitives, and the symphonia-backed `dub-io` decoder. The importer is the first M11 deliverable that actually populates a working library: hand it a folder of loose audio files and you get a SQLite database with canonical `tracks` rows, idempotent `track_files` registrations, per-source `track_metadata_source` rows for `filename` + `id3`, and §8.1-correct dedupe / sibling-version handling for repeat content.

### Pipeline

```text
walk folder (recursive, deterministic alphabetical order, audio-ext filter)
   │
   ▼  per file
discover macOS volume UUID for path  (firmlink-normalised: /System/Volumes/Data → /)
   │
   ▼
check track_files for (volume_uuid, relative_path) → already imported?
   ├─ hit → refresh per-source metadata rows, touch last_seen_at, done
   └─ miss
       ▼
   decode samples + extended ID3 metadata via dub_io::Track::load_from_path
       ▼
   compute Chromaprint fingerprint via dub_fingerprint::Fingerprint::compute_from_f32
       ▼
   find_fingerprint_neighbours(duration_ms, 200 ms) → candidate set
       ▼
   for each neighbour: dedupe::decide(candidate, neighbour)
       ├─ Merge          → add a track_files row against the existing tracks.id
       ├─ SiblingVersion → new tracks row with duplicate_link_track_id populated
       └─ Distinct       → fall through
       ▼
   no merge / sibling? mint a new canonical UUID and INSERT tracks + track_files
       ▼
   UPSERT track_metadata_source(source='id3')      from container tags
   UPSERT track_metadata_source(source='filename') from filename_parser output
```

Single-threaded. Deterministic. Composable: every step is an isolated, unit-tested function in `dub-library`. The end-to-end driver `import_folder(&mut Library, &Path) -> Result<ImportSummary>` is the only public entry point on the importer surface.

### `dub-library::filename_parser` (new module)

Pure function `parse(&str) -> ParsedFilename` implementing PRD §8.4:

* **Patterns recognised** — all four documented forms, layered:
   1. `ARTIST - TITLE.ext`
   2. `ARTIST - TITLE (VERSION).ext`
   3. `ARTIST_-_TITLE_(VERSION)_[YEAR].ext` (underscored separator variant)
   4. `[LABEL CAT#] ARTIST - TITLE.ext`
* **Extension stripping** — bounded to ≤ 5 ASCII-alphanumeric chars so `J Dilla feat. Madlib - Track.mp3` keeps the `feat.` dot and only `.mp3` falls.
* **Leading bracket** — captures `[LABEL CAT#]` verbatim into `ParsedFilename::label_catalog`; the importer writes it into `track_metadata_source(source='filename').comment` (the verbatim text path until M11e ships a stronger Discogs / label resolver).
* **Trailing year** — `(YYYY)` or `[YYYY]` recognised only for `1900..=2099`; five-digit numbers and out-of-range years stay in the title slot. Cross-pattern: `Roxanne - Roxanne (Radio Edit) [2003].mp3` produces `version_tokens={radio}` *and* `year=2003` because the year-strip and version-strip stages run in order.
* **Trailing version segment** — bracketed tail consumed and passed to `version_tokens::parse_plain` (new entry point added to the M11b parser, scans bracket-stripped inner content directly rather than re-requiring brackets).
* **Separator tolerance** — `trim_trailing_separators` strips trailing `_` / `-` / whitespace before the trailing-bracket checks, so `Modeselektor_-_Berlin_(VIP Mix)_[2014].mp3` parses cleanly even though the dangling `_` would otherwise mask the `]` close-bracket.
* **Artist/title split** — first ` - ` separator after `_` → ` ` normalisation; either side may be empty.
* **Whitespace collapse** — runs of whitespace collapse to a single space (rip-naming-convention safety).

`is_junk_title(&str) -> bool` implements the §8.4 ID3 junk-pattern detection so the importer can decide when to demote ID3 in the §8.1 priority chain. Matches `Track 01`, `Track14`, `Unknown`, `Untitled`, empty / all-whitespace; matches blogspot-era noise (`downloaded from xyzblog.com`, `xyzblog.blogspot.com`, `http://example.com`, `Free Download by Producer`). Critical false-positive guards: `is_junk_title("23")`, `is_junk_title("99 Problems")`, `is_junk_title("Track 1 (Live)")`, `is_junk_title("Donuts")` all return `false`. The "all-digits is junk" rule is tightened to require either a period (`01.02`) or a leading-zero short numeric (`01`, `02`); standalone `23` (Blonde Redhead's track) stays a legitimate title.

14 unit tests cover every pattern arm, the year-range edge, unicode artist names (`Björk - Hyperballad`), titles with embedded dots (`feat.`), title-only files (no ` - ` separator), and the full junk-detection truth table.

### `dub-library::importer` (new module)

The end-to-end driver. `ImportSummary { added, merged, sibling_versions, refreshed, skipped, errors }` is the structured result; per-file failures land in `errors: Vec<ImportError>` rather than aborting the whole run — a permissions error on one file shouldn't stop a 10 000-file folder.

Key design decisions:

* **Idempotent re-import shortcut.** `Library::find_track_file_owner(volume_uuid, relative_path)` is checked first. On hit we refresh both per-source metadata rows via the *fast* `dub_io::read_metadata` probe (no full decode) and touch `last_seen_at`. On miss we fall through to the cold decode + fingerprint + dedupe path. A second `import_folder` over the same content costs ~1 ms / file (metadata probe + four SQL UPSERTs) instead of ~100 ms (decode 10 s of audio at 44.1 kHz). Verified by `re_import_is_idempotent`: counts go from `(added=2, merged=0)` first run to `(added=0, merged=0, refreshed=2)` second run with `tracks`/`track_files` row counts unchanged.
* **Dedupe display-string assembly.** `Library::find_track_owner_by_fingerprint_id` resolves a neighbour fingerprint row to `(track_uuid, display_string)` via a `COALESCE` chain that prefers the filename source's title (carries version tokens verbatim from the file system) over the ID3 source (which may have been tag-edited and lost them). The composed string is what the §8.1 version-token check parses, so a tag-stripped ID3 title can't fake-merge a Clean version into a Dirty one when the filename still says `(Clean)`.
* **First-match-wins dedupe choice.** Across the neighbour set, the first `Merge` candidate wins. If no merge is possible, the first `SiblingVersion` candidate wins. Otherwise `Distinct`. Deterministic because `find_fingerprint_neighbours` orders by primary key and the candidate set is small (everything within ±200 ms of the same duration).
* **macOS firmlink normalisation.** `volumes::discover_for_path` was extended to map the raw `statfs(2)` mount-point `/System/Volumes/Data` (the APFS data-volume mount) to `/` (the user-visible boot-volume root). Without this, `/var/folders/...` tempdir paths used in tests and `/Users/...` user-music paths would both fail the `strip_prefix(volume.mount_point)` check used to compute `track_files.relative_path`, because the user-visible path doesn't include the `/System/Volumes/Data` prefix. The volume UUID itself is unchanged — same physical APFS volume — only the mount-point string is normalised. `is_internal` becomes `true` for both `/` and `/System/Volumes/Data` so the homogeneity is preserved through the volumes table.
* **Codec detection from extension.** `detect_codec_from_extension` writes `track_files.codec` from the path extension (`wav` / `mp3` / `flac` / `aiff` / `alac` / `aac`); peeking inside the container for a more precise codec id would need symphonia internals and isn't load-bearing for v1.
* **Audio-extension allowlist.** `AUDIO_EXTS = ["wav", "mp3", "flac", "aif", "aiff", "m4a", "alac", "aac"]` mirrors the workspace's enabled symphonia features. Cover art / sidecar files / notes.txt are skipped.

### `dub-io::TrackMetadata` extensions

The container-metadata snapshot grew from 3 fields (title, artist, album) to 11 to cover the §8.2 `track_metadata_source` column set: `genre`, `comment`, `composer`, `year`, `track_number`, `bpm`, `key`, `gain_db`. Mapping done via `symphonia::core::meta::StandardTagKey`:

* `Date` → `year` (4-char prefix parsed as `i32`, tolerates `"1996"` and `"1996-04-23"`; rejects garbage).
* `TrackNumber` → `track_number` (the `"3/12"` form keeps only the lead component).
* `Bpm` → `bpm` (range-checked `0 < b < 500`).
* `ReplayGainTrackGain` → `gain_db` (`"-7.20 dB"` → `-7.20` after `dB` / `Db` / `DB` / `db` suffix strip).
* `Genre`, `Comment`, `Composer` → direct strings.

`StandardTagKey` does not expose a musical-key variant in symphonia 0.5.5, so `key` stays `None` from container-tag reading. Per PRD §8.4 ("Key detection deferred to v1.x; Mixed In Key writes key data into ID3 comment fields, which we read"), the v1 key path is `Comment` — already covered. M11e Serato importer adds key reading from Serato's `Autotags` GEOB frame.

`Track` was refactored to store the full `TrackMetadata` snapshot internally rather than three loose fields; `extended_metadata() -> &TrackMetadata` is the new accessor the importer uses. The existing `title()` / `artist()` / `album()` accessors stay as the cheap surface for the rest of the codebase (deck headers, file browser).

### `dub-library::Library` new write paths

* `find_fingerprint_neighbours(duration_ms, delta_ms) -> Vec<StoredFingerprint>` — duration-windowed first-pass dedupe filter. The `idx_fingerprints_duration` index makes this index-only at the SQLite level. Caller iterates the result and Hamming-compares each candidate via `dub_fingerprint::similarity`.
* `find_track_owner_by_fingerprint_id(id) -> Option<(uuid, display_string)>` — the dedupe → version-token-check bridge described above.
* `insert_track(uuid, fingerprint_id, duration_ms, duplicate_link_track_id) -> Result<()>` — single canonical row insert.
* `upsert_track_file(track_uuid, volume_uuid, relative_path, codec, sample_rate, bit_depth, channel_count, file_size, mtime) -> Result<()>` — idempotent on `(volume_uuid, relative_path)` UNIQUE index. Refreshes the file metadata fields and bumps `last_seen_at` on conflict.
* `find_track_file_owner(volume_uuid, relative_path) -> Option<String>` — the idempotent-re-import lookup.
* `upsert_metadata_source(track_uuid, source, artist, title, album, genre, comment, composer, year, track_number, bpm, key, gain_db, rating, version_token) -> Result<()>` — idempotent on `(track_id, source)` UNIQUE index. The internal `nonempty` helper normalises empty / whitespace-only strings to `NULL` so the schema's "absent = NULL, not empty string" convention is honoured even when callers pass `Some("")`.

### Test coverage

8 importer integration tests using `tempfile::tempdir` + `hound` synthetic WAV fixtures (no shipped audio binaries):

* `imports_a_fresh_folder_with_one_track` — 1 file → `(added=1, merged=0, skipped=0)`, 1 tracks row, 1 track_files row, 2 metadata rows (id3 + filename).
* `re_import_is_idempotent` — second run produces `(added=0, refreshed=2)` and no new identity rows.
* `two_identical_files_merge` — same audio under two filenames → `(added=1, merged=1)`, 1 tracks row, 2 track_files rows.
* `clean_and_dirty_register_as_siblings` — same audio, version-token mismatch on filename → `(added=1, sibling_versions=1)`, 2 tracks rows (one with `duplicate_link_track_id` populated), 2 track_files rows.
* `skips_non_audio_files` — `notes.txt`, `cover.jpg` in the folder don't bump any counter.
* `rejects_missing_root_path` — calling `import_folder` against a non-existent path returns `LibraryError::Io`.
* `metadata_rows_are_written_per_source` — confirms the filename row carries `artist="J Dilla"`, `title="Workinonit"`, `version_token="instrumental"` from `J Dilla - Workinonit (Instrumental).wav`; the id3 row exists with all-NULL fields because hound-generated WAVs carry no INFO chunk.

14 filename_parser unit tests cover every documented pattern, year-range edges, unicode, separator tolerance, junk-pattern recognition, and the false-positive guard set.

Workspace test count: 626 → 648.

### What this milestone defers

* `analysis_cache` LUFS / waveform / `has_active_grid` row writes — slated for the M10.5j follow-up that rebuilds the analysis pipeline on the new SQLite-backed model. The schema column reservations are already there.
* `track_beatgrids(source='auto')` cross-validation row write — needs `dub-bpm::analyze_bpm` wired into the importer. Trivial follow-up (1–2 hours); kept separate from M11c so the scope stays auditable.
* Background / parallel scanning — single-threaded driver stays the v1 default per PRD §8.4 "deterministic + easy to reason about". A parallel walker is a v1.x ergonomics improvement, not a v1 correctness one.
* Progress reporting beyond the returned summary — `ImportSummary` already carries everything M11d's browser shell will need; in-flight progress callbacks land with M11d when there's a UI consuming them.

### PRD churn

* §12 M11c row marked `✅ shipped` with deliverable list and deferred-items list.
* §8.4 ("Filename-derived metadata") stays correct as written — the implementation matches the spec verbatim.

### Commit boundary

This entry corresponds to the M11c commit set. PRD §12 (M11c row), `crates/dub-io/src/track.rs` (TrackMetadata extensions + Track refactor), `crates/dub-library/src/{filename_parser,importer}.rs` (new modules), `crates/dub-library/src/{db,lib,volumes,version_tokens}.rs` (Library write paths, re-exports, firmlink fix, parse_plain entry), workspace + crate `Cargo.toml` (walkdir dep + dub-io crate-internal dep + hound dev-dep) are all part of the same logical change.

---

<a id="m11d1"></a>
## M11d.1 — Library browser shell (functional replacement)

First reviewable slice of PRD §12 M11d (the §8.5 browser). M11d.1 is the **functional replacement** for the M10.5b `FileBrowserView`: the SwiftUI shell now reads from the M11a–c SQLite catalog instead of walking the filesystem on every Performance render, and the DJ can populate the library by pointing it at a folder of audio files.

The full PRD §8.5 surface (per-row indicators, sortable columns, background missing-files scanner, Relocate panel) is staged across M11d.2 / M11d.3 / M11d.4. Splitting it this way means each landing is a self-contained, reviewable diff rather than one 1500-line PR.

### Architecture choices

1. **Library FFI is a separate UniFFI object from the audio engine.** A new `DubLibrary` (`crates/dub-ffi/src/lib.rs`) wraps `dub_library::Library` and holds its own `Mutex<Option<Library>>` — the engine doesn't know the library exists and vice versa. Two reasons: the library is a disk-backed catalog with its own lifecycle (one open per app vs. per-Thru-session for the engine), and the audio engine's load path takes an `Arc<Track>` snapshot which already isolates it from where the track came from. Keeping the FFI objects separate also matches the eventual M14 split where the engine ships a daemon and the library stays an app-side concern.

2. **`browserSelection: URL?` is preserved as the load-path contract.** The Apple shell already routed `Space` and drag-and-drop through `model.browserSelection`. Rather than re-plumbing that contract to flow track UUIDs, the new `LibraryView` resolves a row's UUID to a file URL via `library.trackPath(trackId:)` at selection time and writes the URL to `browserSelection`. The existing keyboard / drag handlers keep working unchanged. Cost: one extra SQL round-trip per row click (~50 µs); benefit: zero changes to deck-load, drag-and-drop, or Space-shortcut code.

3. **`@Published` library state lives on `WaveformAppModel`, not in a dedicated view-model.** The Apple shell already centralises engine state on `WaveformAppModel` (`isRunning`, `lastError`, `browserSelection`, etc.) and the library handle is the same kind of long-lived per-app state. Splitting it into a separate `LibraryViewModel` would force every cross-concern path (e.g. "after a load, refresh recently-played") to traverse two observable hierarchies. M11d.2's sortable columns may grow a dedicated `LibraryListController` if the surface gets big enough, but day-one keeps it flat.

4. **Listing query runs on a detached `Task`.** `LibraryView.refreshTracks()` dispatches the SQL to a `Task.detached(priority: .userInitiated)` and hops back to the main actor only to install the result. Cold-list of 100k rows is ~80 ms on M2 Air; off-main keeps the source-tree selection feel instant. The single-connection FFI handle serialises the queries internally via its `Mutex`, so two rapid switches between sources produce two deterministic queries instead of one half-done query.

5. **`Just Imported` boundary is captured at app launch, not at "now".** PRD §8.5.2 spec is "tracks added since the last app launch". `WaveformAppModel.appLaunchUnixSeconds` is set in `init`; the LibraryView passes that to `library.justImported(sinceUnixSecs:limit:)`. This means a DJ who plugs the USB stick in at 21:50 and imports it at 21:51 sees exactly that import in the smart crate for the rest of the night.

6. **Source-tree placeholders ship in v1.0.** The §8.5.1 sidebar lists All Tracks, Smart Crates (Recently Played, Just Imported), Dub Crates, Imported Sources, Real Records. M11d.1 wires the first three; Dub Crates / Imported Sources / Real Records render greyscale with a lock glyph + tooltip "Coming in a later milestone." The PRD-spec'd tree shape is therefore present from day one, so the eventual M11e / v1.1 / v1.x landings don't reshuffle the user's mental model of where things live.

### Implementation

**Rust side (`dub-library`).** `crates/dub-library/src/db.rs` grows seven new methods on `Library`:

* `track_count() -> u64` — `SELECT COUNT(*) FROM tracks`. Browser footer reads this.
* `list_tracks(limit, offset) -> Vec<TrackRow>` — All Tracks listing, ordered by `tracks.created_at ASC`. Stable order matters; the browser doesn't reshuffle on every re-open.
* `search_tracks(query, limit) -> Vec<TrackRow>` — FTS5-backed substring search per §8.5.4. Tokens are whitespace-split, ANDed, and suffix-matched (`workin*` hits `Workinonit`). Tokens shorter than 2 chars are dropped to avoid noise on a 100k-track library. Quotes are stripped before the MATCH expression to keep FTS5 syntax happy.
* `recently_played(limit) -> Vec<TrackRow>` — Recently Played smart crate. Reads `play_history` WHERE `event_type = 'load'` per distinct `track_id`, newest first. Empty when the play-history table is empty (v1.0 day-one default; the deck-transport load hook lands at M11d.2).
* `just_imported(since_unix_secs, limit) -> Vec<TrackRow>` — Just Imported smart crate. Caller passes the boundary as unix-seconds; the typical caller is the Apple shell with the app-launch timestamp.
* `resolve_track_path(track_id) -> Option<PathBuf>` — Resolves a canonical UUID to an absolute path by joining `track_files.last_seen_at DESC` against `volumes.last_known_mount_point`. Returns `None` when the volume is unmounted (path resolution is unsafe) or the track has no file rows (deleted). Drag-and-drop + Space-load read this.
* `TrackRow` struct — the canonical row shape. The COALESCE chains live at the SELECT layer (in `TRACK_ROW_SELECT`) so the priority rules from PRD §8.1 (filename source wins for title/artist; id3 source supplies everything else) are enforced consistently across every query method.

**Rust side (`dub-ffi`).** `crates/dub-ffi/src/lib.rs` grows a new `DubLibrary` UniFFI object alongside the existing `DubEngine`:

* `DubLibrary` — `Arc`-shared handle with internal `Mutex<Option<Library>>`. Constructor is empty; `openDefault()` / `openAt(path)` materialise the SQLite connection. `isOpen` predicate lets the Swift side branch cleanly on cold-boot.
* `LibraryFfiError` — flat enum with `OpenFailed` / `QueryFailed` / `ImportFailed` variants. Mirrors `EngineError`'s shape.
* `LibraryTrack` UniFFI record — flat struct mirroring `dub_library::TrackRow` for marshalling across FFI. UUID is sent as a string so Swift treats it as opaque.
* `LibraryImportSummary` UniFFI record — flat version of `dub_library::ImportSummary`; the per-file `errors` list is flattened to `Vec<String>` ("path: reason") so the Swift side doesn't have to re-marshal a structured Rust enum.
* Exposed methods: `trackCount()`, `listTracks(limit:offset:)`, `search(query:limit:)`, `recentlyPlayed(limit:)`, `justImported(sinceUnixSecs:limit:)`, `trackPath(trackId:)`, `importFolder(path:)`. The first six surface the read paths; the last drives the M11c importer pipeline.

**Swift side.** `apple/Dub/Performance/LibraryView.swift` (new) is a self-contained SwiftUI surface that:

* Renders the §8.5.1 source tree on the left (200 pt fixed width) with one `Image(systemName:)` + label per entry, grouped under "Library" / "Smart Crates" / "Dub Crates" / "Imported Sources" / "Real Records". Unavailable entries render greyscale with a lock glyph.
* Renders the search field + "Import Folder…" button in the right pane's toolbar. The search field is a plain `TextField` with a leading magnifying-glass icon and a trailing clear-X. Per-keystroke `onChange` triggers a re-query — fast enough on FTS5 to feel typeahead-y.
* Renders the track list as a `ScrollView` + `LazyVStack` (M11d.2 may swap to `Table` once sortable columns land). Each row stacks title + artist on the left and shows BPM / Key / Duration / Source columns to the right. Selection paints `DubColor.surface2` and writes to `model.browserSelection` via `selectLibraryTrack(_:)`.
* Preserves the AppKit `onDrag { NSItemProvider }` path verbatim from M10.5b. The drag closure resolves the track's URL synchronously through `library.trackPath`; unreachable tracks produce an empty `NSItemProvider` and the drop target no-ops politely.
* Renders a footer with "N shown · M total" + a 5-line summary of the most recent import outcome.

**Apple shell glue (`WaveformAppModel`).** New public surface:

* `let library: DubLibrary` — held for the lifetime of the app window.
* `@Published private(set) var libraryIsOpen: Bool` — drives the "Preparing library…" placeholder.
* `@Published private(set) var libraryTrackCount: UInt64` — drives the footer + sidebar count.
* `let appLaunchUnixSeconds: Int64` — pinned at `init` for the Just Imported smart crate.
* `@Published var lastImportSummary: LibraryImportSummary?` — surfaces the most recent import summary.
* `@Published private(set) var libraryImportInProgress: Bool` — disables the "Import Folder…" button mid-import.
* `func openLibraryIfNeeded()` — idempotent open, called from `MainView.onAppear`.
* `func refreshLibraryStats()` — re-reads `track_count`; called after every import.
* `func importLibraryFolder(_ folder: URL) async` — dispatches the M11c importer to a detached `Task.detached(priority: .userInitiated)`, surfaces a `surfaceError` on session-level failure, updates `libraryTrackCount` + `lastImportSummary` on success.
* `func selectLibraryTrack(_ trackId: String)` — resolves UUID → URL via `library.trackPath` and writes to `browserSelection`; surfaces a polite error if the volume is unmounted.

`apple/Dub/Performance/PerformanceView.swift` swaps the single `FileBrowserView(model: model)` line for `LibraryView(model: model)`. `MainView.onAppear` gains a `model.openLibraryIfNeeded()` call alongside `model.applyConfig()`.

**Xcode project.** `apple/Dub.xcodeproj/project.pbxproj` registers `LibraryView.swift` in the `Performance` group, the file-reference table, and the `Sources` build phase. New UUIDs: `AB11D11D000000000000D101` (build file), `AB11D11D000000000000D102` (file ref).

### Tests

* `dub-library::db` grows seven new tests covering the new helpers: `track_count_starts_at_zero_and_climbs_with_inserts`, `list_tracks_assembles_priority_chain_correctly` (proves the COALESCE chain), `list_tracks_paginates_via_limit_offset`, `search_tracks_matches_via_fts5_suffix` (covers single + multi-token queries, empty / too-short queries, no-match), `just_imported_filters_by_created_at_threshold`, `recently_played_returns_empty_when_no_history` (smart-crate semantics; empty must not fall back to all-tracks), `resolve_track_path_joins_volume_to_relative_path`.
* A reusable `seed_tracks` fixture in the test module pins down the canonical "register a synthetic volume, insert N tracks with both metadata sources" pattern so the next test author doesn't repeat the boilerplate.
* `dub-ffi::library_ffi_tests` adds two smoke tests for the new UniFFI object: `handle_starts_closed_then_opens_via_open_at` (proves `is_open` flip + that a closed handle returns clean errors instead of panicking), `empty_library_returns_empty_track_listings` (proves every listing method returns `[]` against a freshly-migrated DB, not an error).
* Workspace test count: **659 / 659 passing**. Workspace clippy clean (`cargo clippy --workspace --all-targets -- -D warnings`).
* `xcodebuild -project apple/Dub.xcodeproj -scheme Dub -configuration Debug` builds the Apple shell clean. The only warning is `ld: object file libdub_ffi.a was built for newer 'macOS' version (15.2) than being linked (13.0)` from the bundled SQLite shipped through `rusqlite`'s `bundled` feature; benign at runtime, will be addressed in a follow-up when the workspace's `MACOSX_DEPLOYMENT_TARGET` policy is reviewed.

### Deferred

* **Sortable columns** — needs the SQL layer to support arbitrary `ORDER BY` clauses parameterised by sort column + direction. M11d.2 (with the Smart Crates wiring) covers it because the Recently Played sort is a natural starting point.
* **Per-row indicators** (loaded-now A/B glyph, grid-disagreement ⚠, potential-duplicate link, missing-file glyph) — gated on M11d.3.
* **Background missing-files scanner** — gated on M11d.4.
* **Relocate panel** — gated on M11d.4.
* **`Enter` focused-deck-load** — PRD §8.5.6 reserves it for v1.x; v1.0 only commits to Drag + Space.
* **List virtualization via NSTableView / SwiftUI Table** — M11d.1 uses LazyVStack which realises only visible rows but doesn't recycle DOM-style. Lexicon-class 100k libraries land with a Table swap in M11d.2.
* **The legacy `FileBrowserView.swift` stays in the repo** for one milestone in case a rollback is needed. M11d.2 deletes it.

### PRD churn

* §12 M11d row replaced with a four-sub-milestone breakdown (M11d.1 / M11d.2 / M11d.3 / M11d.4) so the staging is auditable in the milestone table itself.

### Commit boundary

This entry corresponds to the M11d.1 commit set: `docs/PRD.md` (M11d row breakdown), `docs/LICENSE-DEPENDENCIES.md` (new doc; user-requested companion to the M11c license review), `docs/SHIPPED.md` (this section), `crates/dub-library/src/{db,lib}.rs` (new helpers + TrackRow export), `crates/dub-ffi/src/lib.rs` (DubLibrary UniFFI object), `crates/dub-ffi/Cargo.toml` (`dub-library` workspace dep + `tempfile` dev-dep), `apple/Dub/MainView.swift` (library state on `WaveformAppModel` + `openLibraryIfNeeded` hook), `apple/Dub/Performance/PerformanceView.swift` (FileBrowserView → LibraryView swap), `apple/Dub/Performance/LibraryView.swift` (new SwiftUI surface), `apple/Dub.xcodeproj/project.pbxproj` (target registration for LibraryView.swift), `apple/DubCore.xcframework/` + `apple/DubShared/Sources/DubCore/Generated/` (rebuilt from the new FFI surface).

---

<a id="m11d2"></a>
## M11d.2 — Recently Played wiring + sortable columns

Second slice of PRD §12 M11d. Closes the loop on the **Recently Played** smart crate (which M11d.1 left wired to the FFI but always empty because nothing wrote `play_history` rows), and lands the §8.5.3 spec promise that "Columns are sortable" by swapping the M11d.1 `LazyVStack` track list for a real SwiftUI `Table` with click-to-sort column headers.

### Architecture choices

1. **Deck-load → play_history hook is Apple-side, not engine-side.** The audio engine's `load_track` only knows about `Arc<Track>` + a file path; it deliberately does not know whether the source was the library or a Finder drag. Routing the hook through `dub-engine` would either force the engine to carry a `Library` reference (cross-couples two subsystems that are otherwise independent) or require a callback-style API (cross-thread, RT-unsafe). Instead, the Apple shell hooks the load *after* a successful `loadTrack(side:url:)`: when `selectedLibraryTrackId` is set and the resolved URL matches the just-loaded URL, the shell calls `library.recordLoad(trackId:deck:timestampMs:)`. Finder drags leave `selectedLibraryTrackId` nil and therefore don't write history rows — which is correct, because the file isn't in the library yet anyway.

2. **`selectedLibraryTrackId` is a separate published value, not a derived one.** `WaveformAppModel.browserSelection: URL?` stays the canonical "selected file" contract for Space-load + drag-and-drop. M11d.2 adds `@Published var selectedLibraryTrackId: String?` set in lockstep with `browserSelection` whenever the selection came from a library row, and cleared when the row goes away. Resolving the trackId back from `browserSelection` would require an FFI round-trip per Space-press; caching the id as a published companion is free.

3. **URL equality is the deduplication guard.** Between `selectLibraryTrack` (which writes `selectedLibraryTrackId` + resolves a URL) and `loadTrack` (which actually loads the URL), the user could conceivably select a library row and then drag in a *different* Finder file. `recordLibraryLoadIfApplicable` re-resolves `selectedLibraryTrackId` to a URL and compares it against the just-loaded URL with `standardizedFileURL` equality before writing the row. Mismatch → no history write. The cost is one extra `library.trackPath` call per successful load; the benefit is that the smart crate never lies.

4. **Sort happens client-side against the in-memory snapshot.** SwiftUI `Table`'s `sortOrder: [KeyPathComparator<LibraryTrack>]` binding triggers a client-side `tracks.sorted(using:)` re-render on column-header click — instant feedback, no SQL round-trip. The FFI's `list_tracks_sorted(limit:offset:sort:ascending:)` exists and is tested but stays reserved for M11d.4's paging refactor; today's 5 000-row in-memory snapshot fits comfortably and reacts instantly. The decision matters because it means M11d.2 doesn't reshuffle column-header click semantics when paging lands — the sort enum is already on both sides of the FFI.

5. **Type-safe sort key enum on both sides.** `dub_library::TrackSortKey` is a Rust enum that maps to SQL column expressions via a private `sql_column()` helper; user input never reaches the SQL string. The UniFFI bridge mirrors it as `LibraryTrackSort`. Swift `KeyPathComparator` over local computed properties (`titleSortKey`, `bpmSortKey`, ...) handles the in-memory case. Adding a new column = one new enum variant on each side + one new `TableColumn` in SwiftUI; the contract is mechanical.

6. **Missing values are pinned past every real value in both directions.** SwiftUI `Table` sorts `Optional` through fallback keys: `bpm ?? .infinity` (numeric) and `title ?? ""` (string). Rust's `list_tracks_sorted` uses `ORDER BY column IS NULL, column COLLATE NOCASE DIRECTION` for the same effect. The shared rule: a small handful of missing-tag rows shouldn't jump to the top of the list when the user clicks "Artist" — they collect at one end. NULL handling for descending sort uses `IS NULL` as the first sort key so NULLs sort last in both directions; this is the same "pinned past everything" semantic.

7. **`COLLATE NOCASE` is mandatory for text sort.** "abba" and "ABBA" should sort adjacent, not separated by the entire lowercase block. Applied unconditionally to the SQL sort expression because the COLLATE hint is a no-op on non-text columns.

### Implementation

**Rust side (`dub-library`).** `crates/dub-library/src/db.rs` grows two new public surfaces:

* `Library::record_load(track_id, deck, timestamp_ms) -> Result<()>` — inserts a `play_history` row with `event_type = 'load'`. The FK on `track_id` rejects unknown ids; we surface that as a `LibraryError::Sqlite` rather than silently swallowing it (a stale Apple-side selection deserves a louder failure than "your smart crate is silently broken").
* `TrackSortKey` enum + `Library::list_tracks_sorted(limit, offset, sort, ascending) -> Result<Vec<TrackRow>>` — column-constrained sort. `TrackSortKey::sql_column()` is the only place in the crate that interpolates a column name into SQL; the safe-list shape means user input never reaches the SQL string. `list_tracks` becomes a thin wrapper that calls `list_tracks_sorted` with `CreatedAt` + ascending.

**Rust side (`dub-ffi`).** Two additions to `DubLibrary`:

* `record_load(trackId:deck:timestampMs:)` — UniFFI-exported method that thin-wraps `Library::record_load`.
* `list_tracks_sorted(limit:offset:sort:ascending:)` + `LibraryTrackSort` UniFFI enum (mirrors `TrackSortKey` 1:1). The empty-listing smoke test was extended to also smoke the sorted variant, and a new `record_load_against_unknown_track_returns_query_failed` test pins down the FK-mismatch error surface.

**Swift side (`LibraryView`).** Track list swaps from `ScrollView + LazyVStack` to a single `Table(sortedTracks, selection: $selectedTrackId, sortOrder: $sortOrder)`. Columns: Title (wide, with subtitle row showing "Artist · Album"), Artist, Album, BPM (monospaced-digit, right-aligned), Key (no sort — Key is a stringly-typed circle-of-fifths token that doesn't compare meaningfully without a Camelot table; deferred), Length (monospaced-digit), Year (monospaced-digit), Source. Selection is bound to `selectedTrackId: LibraryTrack.ID?` and `onChange(of: selectedTrackId)` routes through `model.selectLibraryTrack(_:)` — the existing Space + drag contract is preserved without modification. Drag is moved from the AppKit `onDrag { NSItemProvider }` path to SwiftUI's `.draggable(URL, preview:)` per-cell modifier; the M10.5b reason for AppKit (drag-preview animation glitch) doesn't reproduce inside a Table cell.

**Swift side (`WaveformAppModel`).** New surface:

* `@Published var selectedLibraryTrackId: String? = nil` — set in lockstep with `browserSelection` by `selectLibraryTrack(_:)`; cleared on selection loss.
* `private func recordLibraryLoadIfApplicable(side:url:)` — called from `loadTrack(side:url:)` on `case .success`. Re-resolves `selectedLibraryTrackId` to a URL and compares against the just-loaded URL; on match, writes the `play_history` row with `deck = (side == .a) ? 0 : 1` and `timestamp_ms = unix-millis from Swift wall clock`.
* `LibraryTrack: Identifiable` extension and `LibraryTrack` computed sort keys (`titleSortKey`, `artistSortKey`, ..., `bpmSortKey`, `yearSortKey`) lifting Optional fields into Comparable sentinels.

The xcframework is rebuilt by `scripts/build-xcframework.sh` so the new FFI surface (`recordLoad`, `listTracksSorted`, `LibraryTrackSort`) is visible to the Swift code.

### Tests

* `dub-library::db` grows seven new tests:
  - `record_load_appears_in_recently_played_newest_first` — three loads, newest first ordering proven.
  - `record_load_idempotent_for_same_track_uses_latest_timestamp` — DISTINCT-track semantic in Recently Played; the same track loaded twice surfaces once at the latest timestamp.
  - `record_load_rejects_unknown_track_id` — FK constraint catches stale Apple-side selections; we don't silently swallow.
  - `list_tracks_sorted_orders_by_title_ascending` — basic correctness on the Title sort.
  - `list_tracks_sorted_descending_inverts_order` — direction toggle.
  - `list_tracks_sorted_is_case_insensitive` — proves the `COLLATE NOCASE` hint is doing its job (abba / ABBA sort adjacent, not separated by an entire case block).
  - `list_tracks_sorted_nulls_sort_last_in_both_directions` — the "pinned past everything" rule for both ASC and DESC.
* `dub-ffi::library_ffi_tests` extends the empty-library smoke to cover `listTracksSorted` and adds `record_load_against_unknown_track_returns_query_failed`.
* Workspace test count: **668 / 668 passing** (M11d.1 baseline was 659; +9 across `dub-library` (+7) and `dub-ffi` (+2)).
* `cargo clippy --workspace --all-targets -- -D warnings` clean.
* `xcodebuild -project apple/Dub.xcodeproj -scheme Dub -configuration Debug` builds clean (same benign macOS-version-mismatch ld warning from bundled SQLite as in M11d.1).

### Deferred

* **Real virtualization + paging (`list_tracks_sorted` round-trip per sort change)** — gated on M11d.4. Today's client-side sort is fine for 5 000 rows, will need to swap when 100k-track libraries land.
* **Key column sort** — a meaningful Key sort needs a Camelot-wheel table so `Am`, `1A`, `Em`, `1B`, ... order correctly. Hard-coded sort by raw string would surface `2A` before `10A` which is worse than no sort. Deferred to M11e where the Mixed In Key importer adds a canonical Camelot column to the schema.
* **Per-row indicators (loaded-now A/B glyph, grid-disagreement ⚠, potential-duplicate link, missing-file)** — gated on M11d.3.
* **`play_history` events beyond `'load'`** (play_start, play_end, transition_in, transition_out) — the schema reserves these; M11d.2 wires only `'load'` because that's all Recently Played needs. The deck-transport firing for play_start / play_end is a small follow-up; transition_in / transition_out are gated on the v1.x Played From / Played Into side panel (PRD §8.5.2).
* **Deletion of the legacy `FileBrowserView.swift`** — M11d.1 kept it in the repo as a one-revision rollback safety net. M11d.2 still leaves it; it is fully unused at runtime and will be deleted in M11d.3 when the indicator changes touch the same area.

### PRD churn

* §12 M11d row updated: M11d.1 and M11d.2 both marked ✅ shipped with delta deliverables.

### Commit boundary

This entry corresponds to the M11d.2 commit set: `docs/PRD.md` (M11d row update), `docs/SHIPPED.md` (this section), `crates/dub-library/src/{db,lib}.rs` (`record_load`, `TrackSortKey`, `list_tracks_sorted`, tests, re-exports), `crates/dub-ffi/src/lib.rs` (`record_load`, `list_tracks_sorted`, `LibraryTrackSort`, smoke tests), `apple/Dub/MainView.swift` (`selectedLibraryTrackId`, `recordLibraryLoadIfApplicable` hook), `apple/Dub/Performance/LibraryView.swift` (`Table` swap, `Identifiable` + sort-key computed properties), `apple/DubCore.xcframework/` + `apple/DubShared/Sources/DubCore/Generated/` (rebuilt from the new FFI surface).

---

<a id="m11d3"></a>
## M11d.3 — Per-row indicators

Third slice of PRD §12 M11d. Closes the §8.5.3 promise that browser rows carry per-track visual cues for "is this loaded right now?", "does the dedupe pass think this is a duplicate of another row?" and "is the source file currently reachable?". Adds the three indicator glyphs in a leftmost-gutter Table column; defers the fourth (grid-disagreement ⚠) until the M11c `dub-bpm` follow-up wires offline beat-grid analysis into the importer.

### Architecture choices

1. **Primary file info travels on `TrackRow`, not via per-row FFI calls.** The naive implementation of the missing-file glyph is "for every visible row, call `library.trackPath(trackId)` and check existence." On a Lexicon-scale 100k-track library that's a stampede of FFI traffic for every scroll tick. Instead, `TrackRow` grows three fields populated by a single subquery: `primary_volume_uuid`, `primary_volume_mount_point`, `primary_relative_path`. The Apple side does one filesystem syscall per *unique volume mount point* (typically 3 to 5 on a touring DJ rig) and caches the answer. Scroll is then pure UI work.

2. **The subquery picks the most-recent file deterministically.** When a track has multiple `track_files` rows (the DJ moved a file and re-imported it, or the same file lives on two volumes), the browser must surface a single canonical path. The subquery in `TRACK_ROW_SELECT` filters by `MAX(last_seen_at)` first, then breaks ties with `MAX(id)` (the row's auto-increment primary key, monotonic across all inserts). Without the tiebreaker, two file rows inserted in the same wall-clock second produced a non-deterministic primary; the tiebreaker pins it to "the row we touched last". `resolve_track_path` gets the same `ORDER BY last_seen_at DESC, id DESC LIMIT 1` treatment so the browser and the load path agree on which path is canonical.

3. **Loaded-now glyph keys off a per-deck library track id, not URL comparison.** `DeckState` grows `loadedLibraryTrackId: String?` set by `recordLibraryLoadIfApplicable` on a successful library-sourced load and explicitly cleared on every load (Finder drag or otherwise) so the indicator never lies. Comparing URLs would force the LibraryView to resolve every visible row's UUID to a URL and compare against the deck's URL on every render — slow and bug-prone (path normalisation). UUID comparison is one string equality per row.

4. **Per-volume reachability cache lives on `WaveformAppModel`, not in `LibraryView` state.** Other views (deck headers, future Played From / Played Into panel) will want the same answer. Putting it on the model means a single source of truth + one published value the LibraryView observes. Recompute cadence: once per track-list refresh (source switch, search, post-import). Not per-frame; a DJ unplugging a USB stick mid-set is a once-per-session event, not a 60Hz one. M11d.4's background scanner will tighten this cadence with `access()` per file.

5. **`isTrackReachable` is conservative (false-positive on the glyph).** The cache is asynchronously populated alongside `refreshTracks`; between "switch source" and "first render", every row reads as unreachable. The alternative would be a synchronous syscall on first render; the conservative default is fine because the transient window is small (one detached `Task` round-trip), the glyph is dim (not alarming), and the user's actual answer arrives within ~10 ms. A DJ briefly seeing a missing-file glyph that resolves to "reachable" within a frame is much less harmful than a `FileManager.fileExists` stall on the SwiftUI main actor.

6. **Sibling-version click navigates within the visible list rather than auto-clearing search.** Per §8.5.3 spec, clicking the link glyph "expands a sibling row showing the candidate duplicate." For M11d.3 we land the simpler behaviour: clicking selects the sibling row if it's currently in view. If the sibling is filtered out by an active search, the click no-ops gracefully — auto-clearing search would be too aggressive (the DJ might be intentionally narrowed in on a subset). The full "expand sibling inline" UI is M11d.4 polish.

7. **Grid-disagreement glyph slot is reserved but always-off in v1.0.** PRD §8.3 says the glyph fires when an id3 BPM and an auto-derived `dub-bpm` BPM disagree beyond a threshold. The auto BPM is computed by the M11c `dub-bpm::analyze_bpm` integration which is still deferred; without an auto-grid in `track_beatgrids(source='auto')`, there's no disagreement to surface. The visual slot in the BPM column is preserved so the next milestone doesn't have to reshuffle column widths.

### Implementation

**Rust (`dub-library`).** `TrackRow` gains three optional fields backed by a new subquery in `TRACK_ROW_SELECT`:

```sql
LEFT JOIN (
    SELECT tf.track_id, tf.volume_uuid, tf.relative_path
    FROM track_files tf
    JOIN (
        SELECT track_id, MAX(id) AS max_id
        FROM track_files
        WHERE last_seen_at = (
            SELECT MAX(last_seen_at) FROM track_files tf2
            WHERE tf2.track_id = track_files.track_id
        )
        GROUP BY track_id
    ) latest
      ON latest.track_id = tf.track_id
      AND latest.max_id  = tf.id
) pf ON pf.track_id = t.id
```

The inner correlated subquery picks rows with the max `last_seen_at` per track; the `MAX(id)` outer aggregation picks the last-inserted row among those, giving a fully deterministic primary. The `primary_volume_mount_point` field reaches `volumes.last_known_mount_point` via a separate correlated subquery so the JOIN order is identical to the M11d.1 baseline (no surprise plan changes on large libraries).

`resolve_track_path` is updated with the matching tiebreaker (`ORDER BY tf.last_seen_at DESC, tf.id DESC LIMIT 1`) so the load path and the browser agree on the canonical file.

**Rust (`dub-ffi`).** `LibraryTrack` mirrors the three new fields. The Swift bindings regenerate via `scripts/build-xcframework.sh`; no new methods or enums on the FFI surface this milestone.

**Swift (`WaveformAppModel`).**
* `DeckState.loadedLibraryTrackId: String?` — the deck's currently-loaded canonical UUID, set in `recordLibraryLoadIfApplicable` after a successful library-sourced load. Cleared on every load (including Finder drags) so a Finder load can't inherit the previous library track's "A" badge.
* `@Published private(set) var volumeReachability: [String: Bool]` — per-mount-point cache, repopulated by `refreshVolumeReachability(for: tracks)`. Bounded — entries for mount points no longer in view are dropped.
* `refreshVolumeReachability(for: [LibraryTrack])` — one `FileManager.fileExists(atPath:isDirectory:)` per unique mount point in the supplied list. `isDirectory` must be true (an existing file at the mount path doesn't mean the volume is mounted).
* `isTrackReachable(_:)` — `false` when the track has no recorded primary mount point, when the mount point isn't in the cache, or when the cache says it's offline. Conservative default; the LibraryView calls `refreshVolumeReachability` immediately after `refreshTracks`'s result lands.

**Swift (`LibraryView`).** New leftmost gutter column (36 pt fixed) hosting `rowIndicators(for:)`:

* Loaded-now `A` / `B` badge — small rounded square, accent-tinted via `DubColor.deckATint` / `deckBTint`, shown when `model.deckA.loadedLibraryTrackId == track.id` or `deckB` respectively. Two badges side-by-side when both decks carry the same track (Instant Doubles).
* Potential-duplicate link — `Image(systemName: "link")` wrapped in a plain-style `Button` that calls `navigateToSibling(_:)`. Navigates by setting `selectedTrackId` to the sibling's id; no-ops when the sibling isn't currently in view.
* Missing-file glyph — `Image(systemName: "exclamationmark.triangle.fill")` in `.red.opacity(0.65)`, shown when `model.isTrackReachable(track) == false`. Tooltip differentiates "volume not mounted" (no recorded mount point) from "volume offline" (recorded but currently absent).

`refreshTracks` now also calls `model.refreshVolumeReachability(for:)` on the main-actor hop with the freshly-fetched rows, so reachability is up-to-date by the time the Table re-renders.

### Tests

* `dub-library::db` grows two new tests:
  - `list_tracks_populates_primary_file_columns` — proves the three new TrackRow fields surface volume UUID, mount point, and relative path correctly for a single-file track.
  - `track_row_returns_most_recent_track_file_on_multi_file_track` — proves the multi-file deterministic resolution. A track gets a second `track_files` row at a different `relative_path`; the browser must surface the newer one (the user moved the file).
* Workspace test count: **670 / 670 passing** (M11d.2 baseline was 668; +2 in `dub-library`).
* `cargo clippy --workspace --all-targets -- -D warnings` clean.
* `xcodebuild -project apple/Dub.xcodeproj -scheme Dub -configuration Debug` builds clean (same benign macOS-version-mismatch ld warning from bundled SQLite).

### Deferred

* **Grid-disagreement glyph (§8.3)** — gated on the M11c `dub-bpm::analyze_bpm` follow-up. Without an auto-grid in `track_beatgrids(source='auto')` there's no disagreement signal.
* **Inline sibling-row expansion (§8.5.3 "click expands a sibling row")** — M11d.3 ships click-to-navigate-within-list; the "expand into a sibling row" affordance lands with M11d.4 polish.
* **Real per-file reachability via `access()`** — M11d.3 ships per-volume reachability only. A file that lives on a mounted volume but has been deleted from disk (or moved out from under the library) currently reads as "reachable" until the next import surfaces the loss. M11d.4 lands the background scanner that does the per-file check + drives the missing-files footer + Relocate panel.
* **Indicator glyphs on Finder-drag-loaded tracks** — Finder drags leave `selectedLibraryTrackId` nil so the loaded-now badge doesn't fire even when the same file happens to exist in the library. Fixing this would require reverse-resolving a URL to a library track UUID on every load, which is doable but not v1.0 priority — most loads in real use come through the library.

### PRD churn

* §12 M11d row updated: M11d.3 marked ✅ shipped with deferred-glyph note.

### Commit boundary

This entry corresponds to the M11d.3 commit set: `docs/PRD.md` (M11d row update), `docs/SHIPPED.md` (this section), `crates/dub-library/src/db.rs` (`TRACK_ROW_SELECT` subquery + `TrackRow` field additions + tiebreaker on `resolve_track_path` + tests), `crates/dub-ffi/src/lib.rs` (`LibraryTrack` field additions), `apple/Dub/MainView.swift` (`DeckState.loadedLibraryTrackId`, `volumeReachability` cache, `refreshVolumeReachability`, `isTrackReachable`, indicator-clear in `recordLibraryLoadIfApplicable`), `apple/Dub/Performance/LibraryView.swift` (indicator column + `rowIndicators` / `deckBadge` / `navigateToSibling` helpers + reachability refresh on track-set change), `apple/DubCore.xcframework/` + `apple/DubShared/Sources/DubCore/Generated/` (rebuilt from the new FFI surface).

---

<a id="m11d4"></a>
## M11d.4 — Background missing-files scanner + Relocate panel

Closes M11d and closes the §8.5.5 promise that "external SSDs unmount, files get moved by Finder, networked volumes disappear, and the library handles this gracefully". Adds a long-lived background scanner that flags missing files without deleting metadata, a browser-footer affordance that surfaces the count, and a modal Relocate sheet that walks a user-supplied directory and reattaches every file whose fingerprint + duration still match a missing track.

### Architecture choices

1. **Schema v2 adds `is_missing` + `last_checked_at` to `track_files`, not a sidecar table.** The naive alternative is a `missing_files` table populated by the scanner and joined on read. That's two writes per scanner verdict (insert into `missing_files`, delete on re-discovery) and a join cost on every browser query. A boolean column on the row that already exists is one write, no join, and lets the dedupe and import paths inspect `is_missing` for free. The partial index `idx_track_files_missing ON track_files(is_missing) WHERE is_missing = 1` keeps the "count missing" footer query at constant time even on a 100 k-track library — the index only stores rows where the flag is set, so its size scales with the *missing* set, not the library.

2. **The "track is missing" predicate is `NOT EXISTS healthy file`, not `every file missing`.** A track with one missing file and one healthy file is still reachable through the healthy path; flagging it as missing would be a UX bug. `missing_track_count()` and `list_missing_tracks()` both implement this via `WHERE NOT EXISTS (SELECT 1 FROM track_files WHERE track_id = t.id AND is_missing = 0)`, which also correctly classifies tracks that lost their last file row (it returns true because the subquery is empty).

3. **`last_checked_at` is nullable on purpose.** Rows imported under v1 schema have never been checked. Back-stamping them to "now" or the import time would lie about scanner coverage — surfacing them as "never checked, due first" via `ORDER BY last_checked_at IS NOT NULL, last_checked_at ASC` is more honest and gives newly upgraded libraries a fast first pass. The `IS NOT NULL` term sorts `NULL` rows ahead of every stamped row.

4. **The scanner runs in Swift, not Rust.** The decision branches I weighed:
   * Pure-Rust scanner thread inside `dub-library` → would need its own runtime, error reporting back into the FFI, and a `Library` clone story for the cross-thread handle. Wrong layer.
   * Pure-Rust scanner driven by an FFI "tick" → trivial to implement but pushes back-pressure (when to tick, how often) into Swift anyway.
   * Swift `Task.detached(priority: .background)` that pulls a batch from the FFI and calls `FileManager.fileExists` per row → uses the platform's file-existence primitive (which on APFS is a single `getattrlist` syscall, not a full stat), reuses the library's existing connection model, and gives Swift native control of pause/resume around foreground activity. Picked this one. PRD §8.5.5 says "no read I/O", which `FileManager.fileExists(atPath:)` honors — it uses `access(F_OK)` on macOS.

5. **Rate limiting is a soft cadence, not a strict budget.** Inside a pass the loop ticks every 30 s; when a pass finishes with zero remaining work the next tick is 5 min out. Across a 100 k-track library that gives a full sweep in ~14 hours of wall-clock, which is well below the "user plugs in a drive, expects it to flip from missing to healthy within a few seconds" event timescale. The interactive Relocate panel + the per-volume reachability cache from M11d.3 handle the "I want answers now" case; the scanner is only there for the background "did Finder move this file last week" case.

6. **Volume reachability shortcuts the syscall.** When the row's `volumes.last_known_mount_point` is `NULL` (the volume isn't online), the scanner skips the `fileExists` call entirely and marks the row missing. Saves one syscall per file row on an unmounted drive and prevents the kernel from logging a flurry of `EACCES` complaints when the user has ejected a touring SSD.

7. **`relocate_track` inserts a fresh `track_files` row instead of mutating the missing one.** PRD §8.5.5's hard rule: "Metadata is never deleted when a file goes missing." When the user points Dub at the relocated folder, the original row stays on record. Two consequences fall out for free:
   * If the touring SSD comes back online a week later, the next scanner pass flips the original row back to `is_missing = 0`. The user now has two `track_files` rows for the same canonical track, both healthy — Dub's primary-file resolver (`MAX(last_seen_at), MAX(id)`) picks whichever was touched most recently, and the user can load either.
   * If the user re-relocates to a *third* folder, the second row sticks around too. We never accumulate junk rows in practice because all three rows describe distinct real on-disk locations.

8. **Matching is fingerprint-and-duration, not filename-fallback.** Earlier drafts hedged with "match by filename if fingerprint fails to load". I dropped that — every track in the library has a stored Chromaprint blob (the importer can't insert without one), so a fingerprint match is always available. Filename matching as a fallback is a footgun: two completely different mixes of `Track 01.mp3` would alias, and the dedupe code in M11b already documents why filename matching is unsafe without a fingerprint corroboration. The match predicate is the same one M11b uses for auto-merge: similarity ≥ 0.98 **and** |Δduration| < 200 ms. Bringing in PRD §8.1's threshold keeps "Relocate matches" and "auto-merge candidates" on the same axis.

9. **`try_relocate_candidate` is one FFI call per file, not a batch API.** The natural batch shape — "give the FFI the full directory listing and let it iterate" — would force a streaming progress callback through UniFFI for the modal sheet's progress indicator, which UniFFI doesn't model cleanly today. Per-file calls let Swift drive `NSOpenPanel` cancellation, surface per-file `lastError`, and produce a per-file progress count for free. The FFI itself is stateless across calls; the heavy work (decode + fingerprint + N×similarity comparisons) sits on the Rust side where the dedupe primitives already live.

10. **Match count is computed from before/after deltas, not per-call returns.** The detached worker in `WaveformAppModel.relocateImpl` snapshots `missing_track_count` once at the top of the run, walks the directory, and reads the post-walk count to compute `matched = before - after`. This is robust against the FFI returning `None` on a candidate that decoded fine but didn't match anything (legitimate skip) versus one that crashed mid-relocate (we shouldn't have credited a match either way). A future "Relocate progress per file" sheet could augment with the per-call `Option<track_id>` return — for the v1.0 sheet the aggregate count is what the user reads.

### Implementation

* **`crates/dub-library/src/schema.rs`** — `SCHEMA_VERSION` bumped to 2. New `V2_MIGRATION` block adds `track_files.is_missing INTEGER NOT NULL DEFAULT 0 CHECK (is_missing IN (0, 1))`, `track_files.last_checked_at INTEGER`, partial index `idx_track_files_missing`, and `idx_track_files_last_checked` for the scanner's `ORDER BY` predicate.
* **`crates/dub-library/src/volumes.rs`** — `DiscoveredVolume::relative_to(&Path) -> Option<String>` promoted from the importer's private helper so the FFI's `try_relocate_candidate` can derive `relative_path` without duplicating the strip-prefix logic.
* **`crates/dub-library/src/db.rs`** — new helpers + struct types:
  * `FileScanRow` (file id, track id, volume UUID, relative path, was-missing, mount point).
  * `MissingTrack` (track id, fingerprint id, duration_ms, last relative path, last filename).
  * `list_files_for_scan(batch_size)` — stalest-first ordering, joined with `volumes` for mount-point convenience.
  * `mark_file_state(file_id, is_missing, last_checked_at)`.
  * `missing_track_count()` — partial-index-backed.
  * `list_missing_tracks(limit)` — basename derived at SELECT-time.
  * `relocate_track(...)` — wraps `upsert_track_file` and force-stamps `is_missing = 0, last_checked_at = strftime('%s','now')`.
* **`crates/dub-ffi/src/lib.rs`** — new UniFFI records `LibraryFileScanRow`, `LibraryMissingTrack`, `LibraryFingerprintBlob`, `LibraryResolvedPath`. New `DubLibrary` methods: `missing_track_count`, `list_files_for_scan`, `mark_file_state`, `list_missing_tracks`, `load_fingerprint_blob`, `relocate_track`, `resolve_volume_path`, `try_relocate_candidate`. The last one is the work-horse the Relocate sheet drives — it owns decoding, fingerprinting, similarity, and relocation in one FFI hop.
* **`apple/Dub/MainView.swift`** — `WaveformAppModel` grows `missingTrackCount`, `relocateInProgress`, `lastRelocateMatches`, `lastRelocateUnmatched`, `libraryScannerTask`. New methods `refreshMissingTrackCount`, `scanMissingFilesBatch`, `startMissingFilesScanner`, `stopMissingFilesScanner`, `runRelocate`. `openLibraryIfNeeded` now starts the scanner; `deinit` cancels it; the import success path now refreshes the missing-count alongside the track-count. The matcher worker (`relocateImpl`) is `nonisolated static` so it can run inside `Task.detached` without inheriting the model's main-actor isolation.
* **`apple/Dub/Performance/LibraryView.swift`** — footer renders a red-triangle "N tracks missing · Click to relocate" affordance bound to `showRelocateSheet`. New `RelocateSheet` subview wraps `NSOpenPanel` for directory selection, surfaces `relocateInProgress` as a `ProgressView`, and reports the per-run `(matched, unmatched)` outcome after each match folder.

### Tests

`cargo test -p dub-library` adds four new tests covering the scanner + relocate primitives in isolation:

* `list_files_for_scan_returns_unchecked_first` — two-track seed, both rows surface with `was_missing = false` and mount-point joined-in.
* `mark_file_state_flips_is_missing_and_stamps_checked_at` — confirms the flag flips and the row gets re-sorted to the tail after a check.
* `missing_track_count_only_counts_fully_unreachable_tracks` — two tracks, one with a second healthy file row, only the wholly-missing track contributes to the count and to `list_missing_tracks`.
* `relocate_track_inserts_new_path_and_clears_missing_state` — relocate creates the new row, count drops, original row stays on record so the FK on `play_history` and on `track_metadata_source` remains valid.

`cargo test -p dub-ffi` adds two FFI smoke tests:

* `missing_tracks_and_scan_listing_are_empty_on_a_fresh_library` — confirms the empty-DB happy path (no panics, sensible zeros).
* `try_relocate_candidate_returns_none_for_unreachable_path` — a non-audio file in a tempdir produces `Ok(None)`, not a panic or an error.

Full workspace: `cargo test --workspace` reports **676/676 passing** (+6 over M11d.3 baseline); `cargo clippy --workspace --all-targets -- -D warnings` is clean. `xcodebuild -scheme Dub` succeeds with the only warning being the pre-existing `LibraryTrack: Identifiable` retro-conformance lint (benign — Dub owns the conformance and ships the FFI module).

### Known deferrals / non-goals

* **Paging swap to `list_tracks_sorted`.** The 5 000-row hard cap in `LibraryView` is unchanged. Empirical measurements on the M2 Air show the single-shot path stays under 80 ms for the 5 000-row "All Tracks" case, well inside the 200 ms interactive budget. Real paging needs scroll-position-driven page loads + header-frozen sort indicators + first-row-visible recall on filter change, which is a meaningful Swift undertaking. Parked for M11e where the FFI surface already exists.
* **Scanner cancellation knob in Preferences.** The scanner is always on while the library is open. A "Disable background scanning" toggle is easy to add but no v1.0 user has asked for it — the cost on an idle library is one batch every 5 minutes, which is below the noise floor of a DJ-grade machine.
* **Per-track Relocate.** The current sheet runs a folder-wide pass. A future "right-click a missing track → Locate File…" affordance is plausible for v1.1 but adds a second matching code path (point-and-match versus folder-walk) that nobody has requested. The folder-wide flow covers the actual user complaint ("I moved my music folder to a new SSD").
* **Progress bar inside the Relocate sheet.** The current sheet shows a spinning `ProgressView`; per-file progress requires a UniFFI callback story or a polling tick. Skipped for v1.0 in favor of the aggregate before/after count.
* **Grid-disagreement indicator.** The M11d.3 deferral stands. Wires up at the M11c follow-up that populates `track_beatgrids(source='auto')`.

### PRD churn

* §12 M11d row updated: M11d.4 marked ✅ shipped with a one-line summary of the schema bump + scanner + Relocate sheet + paging-deferral note.

### Commit boundary

This entry corresponds to the M11d.4 commit set: `docs/PRD.md` (M11d row update), `docs/SHIPPED.md` (this section), `docs/LIBRARY-SCHEMA.md` (schema v2 documentation + migration history), `crates/dub-library/src/schema.rs` (`SCHEMA_VERSION` bump + `V2_MIGRATION`), `crates/dub-library/src/volumes.rs` (`DiscoveredVolume::relative_to`), `crates/dub-library/src/importer.rs` (consume the new helper, drop the private duplicate), `crates/dub-library/src/db.rs` (`FileScanRow`, `MissingTrack`, scanner + relocate helpers, tests), `crates/dub-library/src/lib.rs` (re-exports), `crates/dub-ffi/Cargo.toml` (dub-fingerprint dependency), `crates/dub-ffi/src/lib.rs` (record types + `DubLibrary` methods + integration tests), `apple/Dub/MainView.swift` (scanner + Relocate model glue), `apple/Dub/Performance/LibraryView.swift` (footer + `RelocateSheet`), `apple/DubCore.xcframework/` + `apple/DubShared/Sources/DubCore/Generated/` (rebuilt FFI surface).

---

<a id="m11d5"></a>
## M11d.5 — Dogfooding bug-fix round (Performance-mode play, deck-B phantom playback, beatgrid overlay)

Bundles three pre-alpha bug fixes the user surfaced during the M11d.5 dogfooding pass. None of them are headline features; together they unblock the "load a track in Performance mode, hit Play, see the audio play and the beats line up with the kicks" loop that every subsequent test session depends on. Treated as a coherent unit because all three trace back to the same lacuna: the engine was designed around the assumption that a real timecode platter is always attached, and the pre-alpha workflow (sans hardware) walks straight into the gaps that assumption left.

### Symptom 1 — "deck A Play doesn't work in Performance mode"

The deck-header Play button engaged the engine's `Command::DeckPanicPlay` (M11d.5 prior change) but the very next render block clobbered it. Cause: the timecode driver's panic branch (`Engine::drive_timecode_inputs`) auto-cancels panic on the next clean `LiftIntent::Locked` and hands the deck back to whatever rate the policy reports. In a real-platter session that's the canonical Serato INT→ABS path — the dust tick clears, the carrier returns, the deck snaps onto it. In the dogfooding session there is no carrier; any noise on the input that fools the policy's confidence gate (mic bleed, the operator's voice, the next track's room tone leaking through the line-in) is decoded as a "clean re-lock" at whatever sub-coherent rate `dub-timecode::Decoder::process` extracted from the noise. The deck either followed a nonsense rate or, more commonly, paused on the very next `DropoutHoldRate` arm fired by a different noise sample. From the user's chair: "I press Play and nothing audible happens."

**Fix**: split panic into "engine-auto-engaged" (PRD §5.4.2 Repeat hookup from `DropoutHoldRate`) and "user-initiated" (FFI `Command::DeckPanicPlay`). Engine-auto-engaged panic keeps its canonical auto-cancel-on-`Locked` behaviour. User-initiated panic survives `Locked` intents: only an explicit `Command::DeckCancelPanicPlay` lifts it. New field `PanicPlayState::user_initiated: bool`, set inside `Command::DeckPanicPlay` handling, cleared inside `Command::DeckCancelPanicPlay` so a subsequent auto-engage doesn't inherit the sticky semantic. The carve-out lives in the panic-branch `LiftIntent::Locked` arm: when `user_initiated == true` the deck's rate and `is_playing` are left alone, panic stays engaged. PRD §6.1.2 wording is unchanged (the auto-cancel-on-clean-relock contract still describes the canonical timecode flow); the new behaviour is documented in the doc comment on `PanicPlayState::user_initiated` so the next reader doesn't think the test names contradict the prose.

### Symptom 2 — "deck B is playing without me touching it"

Same root cause as Symptom 1, different surface. With a two-deck Performance audio layout the engine attaches a timecode input to deck B at start time even if no track is loaded there. The first `LiftIntent::Locked` that the policy emits (again, plausibly from noise that crosses the confidence threshold) hits the non-panic branch and flips `deck.set_playing(true)` so the position-poll loop reports the deck as playing. There's no audible output — the deck has no file source and no Thru source — but the UI honestly reflects what the engine says (pause icon on the header, `isPlaying = true` in `DeckState`, master-deck selector treats it as a candidate), which from the user's chair reads as "deck B started by itself."

**Fix**: gate the non-panic `Locked` auto-start on `deck.source().is_some()`. A sourceless deck stays paused regardless of policy intent — `is_playing` only matters for the deck's file-playback transport, and there's nothing to transport. Thru-mode capture is unaffected: it renders through `thru_sources` independently of the deck transport, so suppressing the play flag here does not mute live capture. The canonical "platter started, deck plays loaded track" path (the test fixture for `timecode_lock_drives_deck_rate_and_plays`) still calls `set_source` before pushing carrier, so it still wins the auto-start race. New regression test `timecode_locked_does_not_auto_start_deck_without_source` locks the guard down.

### Symptom 3 — "no beat grid on the playing waveform" (UI-BACKLOG B-24)

The Metal renderer (`WaveformRenderer.draw`) drew the broadband envelope only. The Stage-1 BPM estimator's grid is reachable via `DubEngine.beatGrid(deckIdx:)` and the LibraryView's BPM column already reads it, but the playing strip ignored it — beatmatching by eye was impossible and every test session "felt half-implemented" because the user couldn't tell whether a transient on screen was a downbeat or an off-beat.

**Fix**: SwiftUI `Canvas` overlay (`WaveformView.beatGridOverlay`) layered between the zero-crossing hairline and the playhead, refreshed at 30 Hz via `TimelineView(.animation(minimumInterval: 1/30))`. Each frame reads `engine.position` for the unclamped playhead, `engine.beatGrid` for the current grid, intersects the beats against the visible time window `[playhead − pastSecs, playhead + futureSecs]` (using the same `WaveformRenderer.secsPerPixel × pastRegionFraction` constants the Metal pipeline uses for the strip itself), and strokes a 1 px deck-tinted line at every beat with a 2 px brighter line every `beats_per_bar` beats (downbeat). The pass returns early on `grid.confidence == 0` so unconfident tracks don't sprout misleading ticks (B-24 spec point 4). Horizontal-orientation Prep-mode waveforms get the same overlay with the axes swapped. Cost is dominated by the FFI calls plus a handful of `Path` segments per frame — well under any perceptible budget on Apple Silicon; if Instruments flags it later the same math folds cleanly into a second Metal pass with a thin "ticks" vertex buffer.

### Implementation

* **`crates/dub-engine/src/lib.rs`**: `PanicPlayState` gains `user_initiated: bool`; `Engine::engage_panic_play` takes the flag as a new second argument and writes it onto the state record; `Engine::cancel_panic_play` clears it; `Engine::drive_timecode_inputs` panic-branch `Locked` arm checks the flag and skips the auto-cancel + rate write when `true`; non-panic `Locked` arm gates the `set_playing(true)` on `deck.source().is_some()`; `Command::DeckPanicPlay` handler passes `true` to mark FFI / UI engagements, the `DropoutHoldRate` auto-engage path passes `false`. Two new regression tests (`panic_play_user_initiated_survives_clean_relock`, `cancel_panic_clears_user_initiated_flag`, `timecode_locked_does_not_auto_start_deck_without_source`); eleven existing test call-sites updated to pass `false` so they continue asserting the canonical auto-cancel behaviour.
* **`apple/Dub/Waveform/WaveformView.swift`**: new `beatGridOverlay(in:)` + `drawBeatGrid(into:size:)` helpers; `beatGridOverlay` slots into the existing `ZStack` between `zeroCrossingOverlay` and `playheadOverlay` so the deck-tinted playhead always wins the stack when a beat tick crosses it.
* **`docs/UI-BACKLOG.md`**: B-24 removed (shipped).

Both Rust changes are confined to crates that already round-trip clean through `cargo clippy --workspace --all-targets -- -D warnings`; the workspace test suite reports **151/151 passing in `dub-engine`** (+3 over M11c.2 baseline) and the FFI / library suites are untouched. The Apple shell builds end-to-end via `./scripts/build-xcframework.sh && xcodebuild -scheme Dub build` with no new warnings.

### Known deferrals

* **Hand transport back to the platter from user-initiated panic.** Today the only exit is a manual `cancel_panic_play` (the deck-header Play button doubles as Pause and routes through `pause(side:)`, which calls `engine.cancelPanicPlay` in Performance mode). Once a real timecode platter is in the loop the user will want an explicit "Engage timecode" affordance distinct from "Pause". The PRD §6.1.2 Panic Play affordance shipped in M10.6 is the natural home for that toggle. Not blocking pre-alpha dogfooding because the dogfooding workflow has no platter to hand control back to.
* **Beat-grid downbeat phase.** `dub-bpm::analyze_beat_grid` returns a beats vector but does not report which beat is a downbeat; the overlay treats `beats[0]` as the implied downbeat and counts forwards. For tracks where `beats[0]` lands on the "2" or the "and" of a bar (rare in 4/4 dance music; more common in genres with intro fills) the bar markers visually slip by one beat. PRD §8.3 has a long-standing note that downbeat-aware grids land alongside the M11e Serato importer (their `is_active = 1` rows carry an explicit `anchor` column).
* **B-25 (BPM-grid polling never latches).** Unchanged. The 30 Hz position poll still re-queries `engine.beatGrid` for any deck whose BPM is zero. Cheap enough that it hasn't bitten in profile traces; tracked in `UI-BACKLOG.md` for the next polish round.

### Commit boundary

This entry corresponds to one commit set: `docs/SHIPPED.md` (this section), `docs/UI-BACKLOG.md` (B-24 removal), `crates/dub-engine/src/lib.rs` (engine guard + panic carve-out + three new tests + eleven test-call-site updates), `apple/Dub/Waveform/WaveformView.swift` (beat grid overlay), `apple/DubCore.xcframework/` + `apple/DubShared/Sources/DubCore/Generated/` (rebuilt FFI surface — bindings regenerate even when the FFI signature is unchanged because the xcframework script rewrites the headers verbatim).

### Follow-up — beat-grid sync and waveform smoothness

Dogfooding the B-24 overlay surfaced two more issues that traced to the same root cause and shipped together as a follow-up commit inside M11d.5: a residual left/right wobble of the beat ticks against the waveform, and an unrelated-feeling waveform "blink/jitter" the user observed in Prep mode (horizontal orientation).

**Wobble.** The first cut had the Metal renderer and the SwiftUI Canvas both calling `engine.position(deckIdx:)` independently per frame. Both layers tick at the display refresh rate, but the **phase** of their main-thread reads inside any given vsync interval isn't guaranteed. When CADisplayLink's MTKView draw and TimelineView's body re-evaluation alternated their order across consecutive frames, the two calls landed on `playheadSecsUnclamped` values that were sometimes inside the same audio chunk (~1.45 ms wide at 44.1 kHz, ~one drawable pixel of waveform) and sometimes straddled a chunk boundary. Floor-quantizing each layer's playhead to its own `chunkF = floor(playhead / peakDur)` kept each layer self-consistent frame-by-frame, but the per-frame disagreement *between* layers stayed: one frame they snapped to the same chunkF, the next frame they snapped to chunkFs differing by 1, and the eye read that 0-or-1-chunk oscillation as a frame-rate wobble.

The fix replaces the two independent FFI reads with a single shared `WaveformRenderSnapshot` (`@MainActor`, `ObservableObject` without `@Published` fields so writes don't trigger SwiftUI invalidation). The Metal renderer owns the write: inside `WaveformRenderer.draw` it reads `engine.position` once, publishes `lastDrawnPlayheadSecsUnclamped`, `peakDurSecs`, and `hasTrack` into the snapshot, and *then* uses the same values for its own chunk-index math. The Canvas reads only from the snapshot, never from the engine. Worst-case ordering (Canvas tick reaches the snapshot before Metal's current-frame draw has updated it) is a static one-frame lag = 16.67 ms ≈ 11 chunks, which manifests as a constant 5.5 logical-pixel offset of the grid versus the waveform — invisible to the eye and not the *varying* offset that the wobble produced.

**Waveform smoothness.** The same snapshot doubles as a cache for the beat grid itself. The previous version called `engine.beatGrid(deckIdx:)` every Canvas frame; UniFFI clones the `Vec<f64>` of all beat positions across the boundary on every call, so a 6-minute house track allocated ~280 KB/s of beat-list copies on the main thread for as long as the deck rendered. On Apple Silicon this is small in isolation, but it competes for the same main-thread runloop slice that drives MTKView's draw callback, and the user's "blink/jittering" report tracks the resulting contention. The renderer now refreshes the cached beats only on two triggers: (1) the peaks-generation counter bumps (new track loaded — the cached beats are stale) or (2) we haven't yet captured a confident grid for the current generation (`analyze_beat_grid` runs async after `loadTrack`, so the first ~50–200 ms of frames legitimately re-poll until `confidence > 0`, then latch). On steady-state playback every Canvas frame is now allocation-free.

The Canvas's `TimelineView` cadence also moves from `.animation` (refresh-rate-bound, which on ProMotion 120 Hz panels schedules at 120 Hz against a Metal pipeline pinned to `preferredFramesPerSecond = 60`) to `.animation(minimumInterval: 1/60)` so the Canvas ticks 1:1 with Metal frames. This both eliminates the every-other-tick stale-snapshot case on 120 Hz hardware and halves the Canvas-side per-second body rebuilds on ProMotion machines, freeing main-thread cycles for the Metal pipeline. Combined with the FFI-elimination above the per-frame Canvas cost is now dominated by a dozen line-segment paths, which is well inside any frame budget.

**Implementation.** New class `WaveformRenderSnapshot` in `apple/Dub/Waveform/WaveformRenderer.swift`. The renderer gains a `renderSnapshot: WaveformRenderSnapshot?` property and writes through it at the top of `draw(in:)`. `WaveformView` owns the instance as `@StateObject` and threads it through `WaveformMetalView` (now carrying `let renderSnapshot:` and assigning it in both `makeNSView` and `updateNSView` so a SwiftUI rebuild that swaps the snapshot reference still reaches the renderer). `drawBeatGrid` reads `lastDrawnPlayheadSecsUnclamped`, `peakDurSecs`, `hasTrack`, `beats`, `beatsPerBar`, and `beatsConfidence` from the snapshot — zero FFI calls per frame. **Reanalysis of existing tracks is not required**; the beats vector in the snapshot is fetched from the engine's existing `beatGrid` result, which is itself loaded from the same `track_beatgrids` row the LibraryView reads. No on-disk schema or sidecar change.

### Known deferrals after the follow-up

* **Drift floor.** If the Canvas tick consistently lands before the Metal draw inside a vsync interval, the grid sits one Metal frame behind the waveform — a static ~5.5 logical-pixel offset. The fix that would close this gap is to have Metal read from the snapshot too (driven by a single per-vsync writer such as a `CADisplayLink` on the model), but that would invert the dependency direction in the renderer and is deferred until measurement confirms anyone perceives the static offset (the user's wobble complaint was about the *varying* offset, which this fix kills definitively).
* **Async beat-grid arrival.** The poll-until-confident path in the renderer fires on every Metal frame until `confidence > 0`. At 60 fps and a typical analyser latency of ~100 ms that's six FFI calls per track load — unmeasurable in profile traces but technically more than the M11d.5 prior cut's "once on a 30 Hz poll" pattern in `MainView.readDeckState`. If a future analyser regresses to multi-second latency this poll loop is the place to add an exponential back-off.

### Follow-up — Serato-style envelope smoothing (M11d.5 round 2)

After the wobble + per-frame allocation fixes the user did a side-by-side comparison against Serato and reported that, even with the motion smoothness now respectable, our waveform read as visibly less polished. The "ugly compared to Serato" symptom decomposed into two independent problems on closer inspection.

**Problem 1 — periodic motion jump.** Roughly once a second the waveform pops by a handful of pixels to the left. Diagnosis was inconclusive within this round; the working hypothesis is an audio-buffer-vs-frame-period beat (`lcm(1024, 800) / 48000 ≈ 1.07 s` for the macOS default), but a wall-clock-extrapolated playhead inside `WaveformRenderer.draw` did not change the observed symptom, so the cause likely lies elsewhere (main-thread layout invalidation cadence, a `Timer` somewhere in the SwiftUI tree, or window-server compositing). Parked as a separate investigation; the smoothing fix below is independent of it.

**Problem 2 — envelope reads as a row of vertical sticks.** This is the dominant gap against Serato. Each drawn column samples `chunksPerColumn = 2` raw peak chunks (~2.9 ms of audio at 44.1 kHz) and emits a single (`maxPos`, `maxNeg`) pair; adjacent columns' raw envelopes vary wildly because of audio's fine-structure inside any 2.9 ms slice, and the triangle strip topology stitches them with sharp horizontal edges. The eye reads our envelope as "tiny vertical sticks of different heights" while Serato's reads as "one continuous wave shape." This is independent of motion smoothness — it is the *static look* of the waveform.

**Falsified hypothesis: bake-and-scroll.** A first attempt assumed Serato's polish came from pre-rasterising the whole track into a tiled GPU texture strip and translating it continuously via a fragment-shader sampler (the canonical "DAW envelope as a long image" architecture). A 600-line spike implemented this end-to-end as a parallel renderer (`BakedWaveformRenderer` + `BakedShaders.metal`) behind a `useBakedRenderer` flag. The result was not measurably smoother than the per-frame-geometry path, and the waveform looked subjectively worse in the side-by-side (likely small bake-kernel bugs, possibly compounded by the strip's chunk-density matching the per-frame path's). The spike was reverted in full and the entry-point (`WaveformView.swift`) restored to its single-renderer state. Useful negative result: Serato is *not* winning on texture-cached scrolling. The motion-smoothness ceiling of a per-frame-geometry pipeline is high enough that the shape problem dominates the perceived gap, not the scroll mechanism.

**Fix that actually moved the needle: 3-tap horizontal envelope smoothing in the vertex shader.** `Shaders.metal::waveformVertex` now emits each column's amplitude as a weighted average over `(column N − 1, column N, column N + 1)` with weights `(¼, ½, ¼)` — the canonical binomial smoother. The centre tap dominates so transient peaks survive (kick / snare hits keep their height), but the neighbours soften the column-to-column silhouette enough that the envelope reads as a continuous curve. Boundary columns at `chunkInWindow == 0` / `chunksVisible − 1` clamp the missing-side neighbour to the centre's own value so the kernel does not leak past the visible region.

The implementation cost is two extra `nAgg`-chunk loops per emitted vertex; the vertex shader is currently bandwidth-bound, not ALU-bound, so the additional reads are absorbed by existing per-frame work without measurable impact on `MTLCommandBuffer` GPU time. The change touches only `Shaders.metal` (no host-side change, no FFI change, no new uniform fields) and is fully reversible by deleting the new smoothing block.

User-confirmed result: the waveform is "definitely smoother" against Serato side-by-side. The shape now reads as a continuous wave rather than a row of separate vertical bars. Colour pipeline (the post-M11d.5 band-phase fix) is unaffected because the smoothing operates on the post-aggregation envelope only.

### Known deferrals after round 2

* **Wider smoothing kernels are not worth chasing.** A 5-tap `(⅛, ¼, ¼, ¼, ⅛)` or wider would round transients further, which is the opposite of what scratch DJs need from the strip. The 3-tap kernel is the canonical "minimum smoothing that closes the staircase" form; anything fancier (Catmull-Rom, B-spline, cubic Bézier) would re-introduce spurious peaks between columns or round real transients away.
* **Fragment-shader soft top-edge / bloom.** Serato's bars have a subtle outer glow / soft top edge that reads as a half-pixel of cohesion across the silhouette. We deliberately stripped the M10.5h–p bloom stack in the M10.8 reset because it had been tuned wrong; a future polish pass could re-introduce a very mild Gaussian along the amplitude-perpendicular axis (i.e. *not* the bloom-on-everything stack of the M10.5 ladder). Not done in this round — the 3-tap smoothing already closes the largest part of the gap and the user signed off.
* **Periodic motion jump.** Still open. Independent of the shape fix. Candidate causes (in rough order of decreasing prior probability based on the spike's negative result): a `Timer` or NSTimer somewhere in the SwiftUI / AppKit chrome firing on a 1 Hz cadence; the 30 Hz `MainView.pollDecks` timer's tolerance window aliasing against the Metal frame schedule; window-server compositing cadence when SwiftUI overlays sit on top of the MTKView. Next step (when the user wants to chase it) is the instrumentation pass: log `(frametime, pos.playheadSecsUnclamped, CACurrentMediaTime)` for 5 s and identify the exact frame the jump fires on.

### Follow-up — periodic motion jump root-caused (M11d.5 round 3)

The "periodic motion jump" left open at the end of round 2 turned out to be the second of the candidate causes listed above: the 30 Hz `MainView.pollDecks` timer republishing a fresh `DeckState` into SwiftUI on every poll, even when the human-visible content of the deck pane hadn't changed. The waveform looked like a continuous scroll for ~30 frames and then briefly stuttered to the left every time a `DeckState` republication coincided with the next Metal vsync; with the M:SS time text in `DeckHeader` rolling over once per second, the layout invalidation that fed back into the Metal view's `setNeedsDisplay` cadence aliased visibly at roughly 1 Hz.

**Bisection.** Two debug flags walked the layout-cost stack one step at a time. `WaveformView.stripWaveformOverlaysForDebug` removed every sibling SwiftUI overlay in the deck's `ZStack` (zero-crossing hairline, beat-grid Canvas, playhead hairline) so the only thing rendering above the `MTKView` was the drag-gesture surface — the jump survived. `MainView.pollDecksDisabledForDebug` then disabled the 30 Hz polling `Timer` entirely. User-confirmed result: "super smooth now. no jittering." That isolated the cause to the `DeckState` republication path. The waveform renderer was always immune (it reads `engine.position(deckIdx:)` directly each Metal frame), so the only path through which the timer could affect waveform motion was via main-thread time consumed by SwiftUI re-evaluating views downstream of `@Published deckA` / `deckB`.

**Fix.** `MainView.readDeckState` now quantizes `pos.elapsedSecs` and `pos.remainingSecs` to whole seconds with `.rounded(.down)` before writing them into the `next: DeckState` it diffs against the published state. With those two fields snapped to the display's effective M:SS cadence, the `Equatable` derive on `DeckState` returns `true` on every poll where the visible deck-pane content is unchanged, and `setStateIfChanged` early-exits before publishing. SwiftUI sees a republication only at the moment the M:SS text actually rolls — once per second, exactly the rate at which the human can read the change. The 30 Hz `Timer` continues to run (other transient state — `isPlaying`, `atEnd`, `errorFlashUntil` — still benefits from the prompt poll cadence), but the deck pane only invalidates when there's something new to draw.

**Sub-second precision is not lost.** The jog-click-scrub gesture in `WaveformView` calls `MainView.scrub(side:relativeSecs:)` and needs the freshest possible playhead to compute its seek target. The previous implementation read `deck.elapsedSecs + relativeSecs`, which would now jump to the nearest second floor. `scrub` now reads `engine.position(deckIdx:)` directly inside the handler and seeks against the engine's reported `elapsedSecs` (which the engine reports with audio-sample precision). This is also a strict latency improvement: the previous path was inside a one-poll window of staleness (~33 ms worst-case), the new one is from the literal moment of the click.

**Dead-field removal.** `DeckState.playheadSecsUnclamped` was carried through the polling path "for the renderer to read" but the renderer has never actually consumed it from `DeckState` — it reads `pos.playheadSecsUnclamped` directly off `engine.position(deckIdx:)` each Metal frame for exactly the reasons the field's doc comment claimed (continuous sub-chunk precision through the scratch boundary). The field is now gone from `DeckState`, along with its writer in `readDeckState`. The grep search confirmed zero consumers outside the field's own definition and the doc comment that already pointed readers at the engine path.

**User-confirmed.** Smoothness was reported back as "super smooth" after the bisection isolated the cause and is preserved through the production fix because the cost path that was driving the jumps (per-poll `DeckState` republication) is now load-bearing in the same equality-check sense the rest of the M11d.5-round-2 `setStateIfChanged` machinery already depended on. The user signed off on quantizing both `elapsedSecs` (deck-header time-remaining display, track-overview mini-playhead) and the implicit quantization of the track-overview navigation precision: "nothing is relevant sub seconds … the track overview is mainly used to … see where breakdowns are and … to navigate the track by clicking. nothing realtime relevant there that cant wait 1 second."

**Why the previous "smoothing" attempts could not have helped.** Round 2 chased the symptom inside `WaveformRenderer.draw` (subChunkOffsetNDC, wall-clock playhead extrapolation, the baked-and-scrolled texture spike). All three were inside the Metal frame and so all three were strictly downstream of whatever was producing the every-second main-thread stall. The renderer's frame timing was honest the whole time; it was the host's *delivery* of those frames to the compositor that the SwiftUI layout pass was perturbing. Useful retrospective: when a Metal-rendered view stutters on a 1 Hz cadence, search the SwiftUI tree for state that re-publishes on a poll cadence *and* drives content whose width or height changes at the 1 Hz visible-text-change cadence. The shape problem (round 2) and the motion problem (this round) were independent the whole time, which is why no amount of envelope smoothing touched the jump and no amount of jump-chasing touched the shape.

### Files touched in round 3

* `apple/Dub/MainView.swift` — `DeckState.playheadSecsUnclamped` field removed (doc comment too); `readDeckState` writes `next.elapsedSecs = pos.elapsedSecs.rounded(.down)` and `next.remainingSecs = pos.remainingSecs.rounded(.down)`; the polled `playheadSecsUnclamped` assignment is gone; `scrub(side:relativeSecs:)` now reads `engine.position(deckIdx:)` directly for the seek target. The two debug flags (`pollDecksDisabledForDebug`, `stripWaveformOverlaysForDebug`) are removed in full now that the diagnosis is settled.
* `apple/Dub/Waveform/WaveformView.swift` — `stripWaveformOverlaysForDebug` flag and its conditional ZStack branch are gone; the overlay siblings render unconditionally as in production.

### Known deferrals after round 3

* **Sub-second deck-header readouts.** Some users may eventually want a tenths-of-a-second sub-display in the deck header during cue work (e.g. visualising a sub-bar drop in the loop-roll context). If that lands as a v1.x feature it can opt in to a finer quantization on a new `DeckState` field without un-rounding `elapsedSecs` itself (the current field's job is the M:SS display, which is a separate consumer from a hypothetical tenths display).
* **Track-overview click-to-seek precision.** The mini-playhead reads `elapsedSecs` which is now rounded, but the *click-to-seek* path from the track overview computes a seek target from a fractional position-along-strip and bypasses `elapsedSecs` entirely. The user explicitly approved that "nothing realtime relevant there that cant wait 1 second" applies to the mini-playhead display alone. No change needed unless a future feature wants sub-second click precision from the overview.
* **The 30 Hz timer cadence itself.** Could in principle be slowed further (15 Hz or 10 Hz) now that the time fields no longer republish on every tick. Not done in this round — the other fields the timer drives (transient transport-icon updates, panic-play badge transitions, error-flash overlays) still benefit from the 33 ms cadence, and the equality short-circuit means the cost of a redundant tick is now exactly one `DeckState` struct compare per deck.

### Follow-up — Library-sourced beat grid is the single source of truth (M11d.5 round 4)

A grep over the deck-load path turned up a structural seam that the previous SHIPPED rounds had not addressed: the engine and the library each maintained their own beat grid for the same track, computed independently from the same audio, off the same algorithm, in parallel on every deck load. The two grids could (and provably did) disagree by ±0.02 BPM because float arithmetic is non-associative; the DeckHeader and LibraryView showed different numbers for the same loaded track. The deck-load also did the full ~100–400 ms `dub_bpm::analyze_beat_grid` pass on every load, including for tracks the library had already analysed and stored. And the renderer's `confidence > 0` latch on the 30 Hz `beatGrid` FFI poll never fired for tracks the engine's Stage-1 estimator legitimately rejected (silence stems, sub-confidence input), keeping the poll firing at 30 Hz forever for the lifetime of the loaded track.

Three distinct symptoms (UI-BACKLOG C-26, the load-time DSP cost, UI-BACKLOG B-25), one cause: there was no canonical place for the deck's beat grid to come from. This round makes the library the single source of truth, on the principle that anything that already lives in `track_beatgrids(is_active = 1)` is authoritative and the engine shouldn't be re-deriving it.

**Surface changes.**

* **`dub-library::Library::active_beatgrid_for_track(track_id) -> Result<Option<ActiveBeatgrid>>`** — new read-only helper. Single SELECT against the partial unique index `idx_one_active_grid_per_track ON track_beatgrids(track_id) WHERE is_active = 1`; sub-millisecond on any size of library. Returns the new `ActiveBeatgrid { source, bpm, anchor_secs, captured_at }` struct, or `None` for unanalyzed / analysed-but-silent / unknown-id tracks (the unknown-id case resolves to `None` rather than an error so the FFI can be used as a fast-path probe from the load handshake without a separate `tracks` round-trip).
* **`dub-ffi::DubLibrary::active_beat_grid(trackId: String) -> Result<Option<LibraryBeatGrid>>`** — straight FFI shim over the library helper. New `LibraryBeatGrid` UniFFI record carrying the same four fields. Mirrors `ActiveBeatgrid`; lives in the Swift bindings as `LibraryBeatGrid?`.
* **`dub-ffi::DubEngine::load_track(deckIdx, path, libraryBeatGrid: Option<LibraryBeatGrid>)`** — the load FFI gains a new optional parameter. When `Some`, the background worker thread skips `dub_bpm::analyze_beat_grid` entirely and instead synthesises the engine-shape `BeatGrid { bpm, confidence: 1.0, beats: Vec<f64>, beats_per_bar: 4 }` from the supplied `(bpm, anchor_secs)` and the loaded track's known duration. When `None`, the worker analyses as before — Finder-drag loads (no library row to consult) and library-track loads where the library has not yet analysed the track both take this branch unchanged.
* **`FFI_VERSION` bumps from `14` to `15`.** Tripwire constant updated, version comment extended with the round-4 entry, regression test renamed (`ffi_version_is_fifteen_after_library_sourced_beat_grid`).

**Beat-vector synthesis.** The library schema stores only `(bpm, anchor_secs)`; the engine and the renderer want a `Vec<f64>` of per-beat timestamps. The new `synthesise_beat_grid(bpm, anchor_secs, duration_secs)` private helper in `dub-ffi` walks the fixed-tempo formula `anchor + i · 60 / bpm` from one beat before zero (so the empty-groove paint at the lead-in looks right) through to the last beat at or before `duration_secs` (same for the lead-out). Defensive against `bpm <= 0`, non-finite `bpm`, non-finite period, and zero / negative duration — all return the empty grid rather than infinite-looping or panicking. Fixed-tempo is the v1.0 contract for both the auto pipeline (M11c.1) and the v1.0 importers; a future tempo-drift grid format (PRD M10.5p-grid) would need a schema change to add a per-beat positions column, at which point `LibraryBeatGrid` grows a `beats: Vec<f64>` field and the synthesis helper retires.

**Swift wiring.** `WaveformAppModel.loadTrack(side:url:)` now calls a new helper `libraryBeatGridForPendingLoad(url:)` on the main actor before kicking the detached load task. The helper returns `Some(LibraryBeatGrid)` only when three conditions all hold: the library is open, `selectedLibraryTrackId` is set, and `library.trackPath(trackId:)` resolves to the same canonical URL as the load target (same equality guard as the existing `recordLibraryLoadIfApplicable` URL-equality check, so the two paths can never disagree about whether a load came from the library). A failed library read is logged but non-fatal — the engine analyses the file itself in the fallback branch.

**What this fixes.**

* **UI-BACKLOG C-26 (BPM mismatch between DeckHeader and LibraryView).** Now impossible by construction. Both surfaces read the same row in `track_beatgrids`; the engine no longer derives a second number.
* **UI-BACKLOG B-25 (30 Hz `beatGrid` FFI poll never latches on silence).** The renderer's poll-until-confident loop now latches on the first frame after load: the library either has an authoritative grid (`confidence = 1.0` by construction) or the engine ran its own analyser which returns `confidence = 0` deterministically for silence, and the poll latches on either outcome. The "perpetual poll" failure mode is gone.
* **Per-load BPM analysis cost.** Cut from ~100–400 ms to a few microseconds for any track the library has already seen. The DSP run on first-ever load is unchanged; the savings are exactly proportional to how often the user re-loads previously-played tracks, which on the target user's set will be near 100 % after a few practice sessions.
* **Cross-deck consistency.** Loading the same track on both decks now produces byte-identical grids on both decks (both reading the same DB row), eliminating a rare-but-real "the two decks drift apart slightly even at the same nominal BPM" surprise.

**What this does not fix.** The first-ever load of a track still pays the analysis cost twice — once in the engine's background worker (which installs the grid into the deck immediately) and once in `Library::analyze_track` (which decodes the file again and writes the result to `track_beatgrids` so the *next* load takes the fast path). Eliminating this requires either threading the engine's just-computed grid back into the library write, or restructuring so the library writes the grid before the engine ever sees the file. Both are bigger refactors than this round; the user-approved "lazy is fine" trade-off keeps the redundant work in place for now. Empirically the wasted cost is ~300 ms of decoder thread time per unique track per user-session, never on the audio path.

**Test coverage added.**

* `dub-library::analysis::tests` (+4): `active_beatgrid_for_track_returns_none_before_analysis`, `..._returns_auto_row_after_analysis` (round-trip identity check on bpm + anchor), `..._returns_other_source_when_auto_is_inactive` (pre-seeds a Serato row, asserts the Serato row wins and the auto row is recorded inactive — confirms the helper honours the partial unique index and PRD §8.3 priority), `..._returns_none_for_unknown_track_id` (typed-None on bad id, not an error).
* `dub-ffi::tests` (+3): `synthesise_beat_grid_emits_uniform_120bpm_grid` (anchor-zero 120 BPM, 10 s track → 22 beats from `-0.5` through `10.0` at 0.5 s spacing), `synthesise_beat_grid_walks_back_from_positive_anchor` (174 BPM, positive anchor → first beat ≤ 0, last beat ≤ duration, uniform spacing), `synthesise_beat_grid_returns_empty_on_degenerate_inputs` (zero / negative / NaN / inf BPM, zero / negative duration → empty grid, no panic, no infinite loop).
* `dub-ffi::library_ffi_tests` (+2): `active_beat_grid_on_closed_library_returns_query_failed` (closed-handle surfaces typed error, not a panic), `active_beat_grid_for_unknown_track_returns_none_not_error` (unknown id → typed `Ok(None)`).

Full workspace: `cargo test --workspace` reports green (685 → 694 passing, +9); `cargo clippy --workspace --all-targets -- -D warnings` is clean. `xcodebuild -scheme Dub` builds with only the pre-existing `LibraryTrack: Identifiable` retro-conformance warning + the pre-existing M11c.2 Swift 6 main-actor capture warning in `analyzeTracks`, both unrelated to this round.

### Files touched in round 4

* `crates/dub-library/src/analysis.rs` — new `ActiveBeatgrid` struct, new `Library::active_beatgrid_for_track` method, four new tests.
* `crates/dub-library/src/lib.rs` — re-export `ActiveBeatgrid`.
* `crates/dub-ffi/src/lib.rs` — new `LibraryBeatGrid` UniFFI record + `From<dub_library::ActiveBeatgrid>` impl, new `DubLibrary::active_beat_grid` FFI method, `DubEngine::load_track` signature extended with `library_beat_grid: Option<LibraryBeatGrid>`, `background_analyze_and_install` gains the same param and branches between synthesis and `analyze_beat_grid`, new `synthesise_beat_grid` private helper, FFI_VERSION 14 → 15 + version comment extended, three synthesis tests + two FFI smoke tests. Two existing `load_track` test sites updated to pass `None`.
* `apple/Dub/MainView.swift` — new `libraryBeatGridForPendingLoad(url:)` helper; `loadTrack` calls it on the main actor before the detached FFI task and threads the result through.
* `apple/DubShared/Sources/DubCore/Generated/*` — regenerated bindings (new `LibraryBeatGrid` record, extended `loadTrack` signature, new `activeBeatGrid` method).
* `apple/DubCore.xcframework/` — rebuilt for both `aarch64-apple-darwin` and `x86_64-apple-darwin` via `scripts/build-xcframework.sh`.

### Known deferrals after round 4

* **First-ever-load double-cost** (described above). Two paths to fix: (a) plumb the engine's freshly-computed grid into `Library::analyze_track`'s upsert (skip the second decode + DSP); (b) flip the dependency so the library writes the grid before the engine analyses (eager analysis on import, which the PRD's §8.4 "lazy by design" rule explicitly rejects). (a) is the right path when it's chased; deferred for now.
* **Drift floor of ~5.5 logical pixels between Canvas beat-grid overlay and Metal waveform** (round 2 deferral; unchanged). Architectural fix is to have Metal also read from the snapshot so a single per-vsync writer drives both layers. Still deferred until measurement confirms the static offset is perceptible.
* **Grid-disagreement indicator on the LibraryView's BPM column.** Plumbing has been here since M11c.1; the actual disagreement signal is wired in M11e when the Serato importer starts producing non-auto active rows that can disagree with the auto row.
* **Tap-to-set-downbeat (PRD §8.3.1).** Unblocked by this round (the deck's grid is now backed by a library row, so a user tap can upsert a `source = 'user_tap'` row and the deck adopts it on next load without reanalysis). Still a v1.x feature, intentionally not pulled forward in the same chunk.

### Follow-up — Beat-grid re-enable and per-second residual jump (M11d.5 round 5)

Two regressions reported back-to-back after round 4: the beat-grid overlay was no longer rendering in Prep mode (confirmed missing in Performance mode too), and the round-3 "every-second jump" was back in subtle form ("more subtle but I think every time the second changes its jumping slightly").

**Beat-grid overlay was gated off.** `WaveformView.swift::beatGridOverlayEnabled` was set to `false` during round 1's "is the overlay causing the smoothness regression?" A/B bisection and never flipped back after the real cause (the round-3 polling-republish) was found. The Prep mode + Performance mode reads of that flag suppressed both the SwiftUI Canvas overlay and the `WaveformRenderer`'s snapshot write path entirely, which is why no beat grid appeared anywhere. The constant is now `true` (production default) and the doc comment rewritten to record what the A/B bisection actually concluded ("the regression that flag was guarding against was a per-frame allocation in the snapshot path, which was fixed at the source; the overlay is on in production").

**Per-second jump diagnosed.** Round 3 quantized `pos.elapsedSecs` / `pos.remainingSecs` to whole seconds before writing them into `DeckState`, which collapsed the 30 Hz `pollDecks` republish cadence to 1 Hz on those fields. That worked: the user-confirmed "super smooth now. no jittering" report from round 3 was taken with `pollDecksDisabledForDebug = true` (zero republishes), and the production fix preserved that property for everything *between* the 1 Hz M:SS rollovers. The residual the user is now reporting is the rollover itself: once a second, when the M:SS integer floor changes, `DeckState`'s `Equatable` derive sees a real difference, `setStateIfChanged` republishes, and SwiftUI invalidates `PerformanceView`. The view's body re-evaluates the entire deck pane (header + waveform + overview chrome), which is bounded but non-trivial main-thread work. Any Metal frame trying to land during that body-walk is delayed by a few ms. At 1 Hz cadence the human eye reads the delay as "the waveform jumps slightly when the second changes". The renderer's frame timing was honest the whole time (it reads `engine.position` directly per Metal frame); the perturbation was the SwiftUI work happening *around* the frame.

**Root-cause fix: decouple the time-displaying consumers from `DeckState`.** The structural answer is to stop carrying per-second-changing time values through `model.deck{A,B}` at all. With that path closed, `model.deck{A,B}` only changes on genuine state transitions (track load, play / pause, panic, error-flash), and `PerformanceView` no longer re-evaluates body on the 1 Hz rollover cadence.

Two consumers of the removed fields:

1. **Deck-header time text** (PRD §6.1 / §6.1.3). Previously read `state.timeRow.elapsedText` / `.remainingText`, which `DeckHeaderState.from(...)` derived from `deckState.{elapsed,remaining}Secs`. Now reads `engine.position(deckIdx:)` directly from inside a new `LiveDeckTimeText` SwiftUI view, which wraps a `TimelineView(.periodic(from: .now, by: 0.5))` and renders one `Text` per slot. The 2 Hz timeline is enough — the integer-second-floor of the position only changes once a second, and a 0.5 s tick keeps the M:SS rollover visually fresh without paying the cost of a per-display-refresh closure body re-eval. `.monospacedDigit()` is still in effect at the `timeRow` HStack level so the text widget's width is byte-stable across rollovers. `DeckHeader` gains optional `liveEngine` / `liveDeckIdx` params; production callers (`PerformanceView`) supply them, SwiftUI previews leave them `nil` and fall back to static `"--:--"` / `"-00:00"` placeholders.

2. **Track-Overview mini-playhead bracket.** Previously read `deckState.elapsedSecs` inside `playheadFraction()` to compute the bracket's fractional position along the strip. Now reads `model.engine.position(deckIdx:).elapsedSecs` (and `pos.durationSecs` in the fallback branch). The single `Canvas` is wrapped in `TimelineView(.periodic(from: .now, by: 0.5))` so the bracket advances on its own cadence; the bars + background re-draw with each tick too, but they're bounded-cost (a few hundred bars at most) and the redraw is local to this Canvas. The view's `onChange(of: deckState.sourceURL)` / `onChange(of: deckState.hasTrack)` reload-on-load hooks are unaffected by the decoupling because those properties don't change on the 1 Hz cadence.

`DeckHeaderState.TimeRow`'s associated `elapsedText` / `remainingText` strings are now structurally gone — the enum is reduced to two payload-less cases (`.remainingOnly`, `.elapsedAndRemaining`) that serve as layout selectors only. `DeckState.elapsedSecs` / `.remainingSecs` are removed from the state struct entirely; `MainView.readDeckState` no longer writes them, and the three other setters that used to reset them on load / restart / seek (`loadTrack` success branch, `restart`, `seekDeck`) are simplified to skip the dead-field assignments. The renderer's per-frame engine read continues to be the canonical source of truth for sub-second playhead precision (jog-scrub seek), so no consumer loses precision from this round.

**Why the previous "monospacedDigit + quantize" combination wasn't enough on its own.** Round 3 reduced republish frequency from 30 Hz to 1 Hz, which moved the visible stutter from "constant 30-frame flicker" to "subtle once-per-second tick". `.monospacedDigit()` further made the actual `Text` widget byte-stable in width across rollovers, but it doesn't prevent SwiftUI from invalidating the *containing* `PerformanceView` when its observed `model.deck{A,B}` `@Published` value changes. The cost being measured wasn't text-layout cost; it was the body-walk through the whole deck pane that fires on every `@Published` change of any field, regardless of how cheap the resulting per-leaf render turns out to be.

**User-perceptible result.** The deck pane's view tree above the `TimelineView` boundaries is now inert at the 1 Hz cadence. The only thing that re-evaluates body once a second is the `LiveDeckTimeText` subview (one `Text` widget per slot), which is cheap enough that even a contended main thread can finish it well within a vsync. The track-overview's `Canvas` redraws too, but its draw cost is bounded by the bar count and is comparable to the bar-only redraw cost in the steady-state.

### Files touched in round 5

* `apple/Dub/Waveform/WaveformView.swift` — `beatGridOverlayEnabled = true`. Doc comment rewritten to record the bisection outcome and the production rationale rather than the now-stale A/B framing.
* `apple/Dub/Performance/DeckHeader.swift` — new `LiveDeckTimeText` view (TimelineView-driven, reads `engine.position(deckIdx:)`); `DeckHeader` gains optional `liveEngine` / `liveDeckIdx` params; the inner `timeRow(_:)` switches between `LiveDeckTimeText` and a static placeholder via the new `liveTime(slot:textColor:)` helper; `DeckHeaderState.TimeRow` enum cases lose their `elapsedText` / `remainingText` payloads (layout selector only); `DeckHeaderState.from(...)` no longer formats time strings; the three SwiftUI previews are updated to the payload-less enum.
* `apple/Dub/Performance/PerformanceView.swift` — both `DeckHeader(...)` call sites pass `liveEngine: model.engine` + `liveDeckIdx: 0`/`1`.
* `apple/Dub/Performance/TrackOverviewView.swift` — body wraps the `GeometryReader` + `Canvas` in `TimelineView(.periodic(from: .now, by: 0.5))`; `playheadFraction()` reads `model.engine.position(deckIdx:)` instead of `deckState.{elapsed,duration}Secs`; the `onChange(of: deckState.sourceURL/hasTrack)` reload-on-load hooks move outside the timeline wrap so they still observe the published deck-state changes that drive `reloadIfStale()`.
* `apple/Dub/MainView.swift` — `DeckState.elapsedSecs` / `.remainingSecs` fields removed; `readDeckState` no longer writes them; the three sites that reset them on load / restart / seek (`loadTrack` success branch, `restart`, `seekDeck`) drop the dead assignments; the `scrub(side:relativeSecs:)` doc comment is rewritten to reference the new "fields removed in round 5" rather than the round-3 "fields quantized" framing.

**Follow-up to round 5 — beat-grid wobble against the smoothly-scrolling waveform.** After the Canvas re-invocation fix, the grid was rendering and advancing, but the user reported "the grid is going left/right minimally" while the waveform itself stayed smooth. Root cause was a mismatch between the renderer's *visible* playhead and the snapshot's *published* playhead.

The Metal pipeline addresses peak columns by chunk index, snapped to multiples of `chunksPerColumn = 2` (the "chunk-pair snap") so the per-column band aggregate is stable across frames — see the colour-flicker note earlier in this round's history. With that snap alone, the visible waveform would jump in 1-logical-pixel increments at 60 Hz; instead the renderer **also** applies a continuous sub-pixel slide via the vertex shader's `subChunkOffsetNDC` uniform, computed as `epsChunks = continuousChunkF - playheadChunkSnapped` and translated into NDC space. The result is that the *visible* playhead position equals `snapped + continuous_eps` — the snapped index picks the data column, the eps slides the geometry smoothly within that quantum.

The snapshot's `lastDrawnPlayheadSecsUnclamped` was previously written as `playheadChunkSigned * peakChunkDurationSecs` — the snapped value only, no eps. The overlay then read that and ran an additional `(playhead / peakDur).rounded(.down) * peakDur` snap of its own, double-locking the grid to the chunk-pair grid. As the audio engine's continuous playhead ramped from one chunk-pair boundary to the next, the waveform's eps slide moved the geometry forward by up to 1 logical pixel — but the grid stayed at the snapped position, drifting *backward* by the same amount in the waveform's frame of reference. When the next snap fired, the snapshot jumped forward 1 logical pixel and the eps reset to zero, so the waveform's net visible position was unchanged but the grid lurched forward. Repeating at the chunk-pair cadence, the grid's relative motion to the waveform was the classic "drift then snap" sawtooth the user perceived as left/right wobble.

Two-line fix, both at the cause:

1. `WaveformRenderer.draw` writes `pos.playheadSecsUnclamped` (continuous, sample-accurate) into the snapshot instead of `playheadChunkSigned * peakChunkDurationSecs`. The snapped value never had any consumer outside the renderer's own column-indexing math; nothing in the snapshot needs the snap.
2. `WaveformView.drawBeatGrid` drops its `(playhead / peakDur).rounded(.down) * peakDur` snap. `visibleStart = playhead - visibleSecs * pastFrac` uses the continuous playhead directly.

The pixel math works out exactly because `secsPerLogicalPx == peakDur * displayScale` (the overlay's own derivation) and the renderer's `subChunkOffsetNDC` produces the same `(continuous - snapped) / peakDur` shift in logical pixels. Both layers now apply the same continuous offset against a common reference. The doc comments on `WaveformRenderSnapshot.lastDrawnPlayheadSecsUnclamped`, the renderer's snapshot-write block, and the overlay's quantization comment are rewritten to match.

**Follow-up to round 5, attempt 2 — beat-grid drift against the smoothly-scrolling waveform.** Round 1 of this follow-up matched the playhead values both layers reference but still left a slow drift between the grid lines and the waveform's beats: the user described it as "looks like wobble but the grid is moving at a different speed". Tracing the rate carefully revealed the cause is in the shader's NDC mapping, not the playhead.

The vertex shader maps `chunkInWindow ∈ [0, chunksVisible)` to NDC linearly using `(chunksVisible - 1)` column-step intervals:

```
frac = float(chunkInWindow) / float(chunksVisible - 1u);
timeNDC = 1.0 - 0.5 * frac;          // past
timeNDC = 0.5 - 1.5 * frac;          // future
```

That `-1` means N columns occupy `(N-1)` intervals across the region's NDC span, so the effective seconds-per-drawable-pixel is

```
secsPerDrawablePxPast   = chunksPerColumn × peakDur × (drawnAbove - 1) / pastPixels
secsPerDrawablePxFuture = chunksPerColumn × peakDur × (drawnBelow - 1) / futurePixels
```

i.e. `≈ peakDur × (1 - 1/N)` per region — not the clean `peakDur` the naive overlay derivation assumed. For a typical Retina geometry (`drawableHeight = 640`, `pastFrac = 0.25` ⇒ `drawnAbove = 80`, `drawnBelow = 240`) the past region's per-pixel rate is `peakDur × 79/80` (1.27 % slower than naive) and the future region's is `peakDur × 239/240` (0.42 % slower). The overlay's earlier `secsPerLogicalPx = peakDur × displayScale` used the naive rate, so the grid moved *slightly faster per second of audio* than the waveform — a continuous drift, not a per-snap jump, exactly matching the user's "different speed" description. Past and future drift rates differ from each other, so a single shared rate compromise can't fix it cleanly either.

Fix is to mirror the shader's piecewise NDC math in `WaveformView.drawBeatGrid`:

1. **Compute geometry the same way the renderer does.** Mirror `WaveformRenderer.draw`'s `pastPixels = round(timeAxisPixels × pastRegionFraction)` and `drawnAbove = pastPixels / pixelsPerDrawnColumn` (same for the future region). The overlay can read `axisLengthLogical × displayScale` to recover `timeAxisDrawablePixels` exactly because both layers share the screen-scale environment.
2. **Snap the playhead to the chunk-pair grid locally.** `snappedChunkF = floor(continuousChunkF / chunksPerColumn) × chunksPerColumn` is fully deterministic from the continuous playhead the snapshot already publishes; no extra snapshot field needed. `epsChunks = continuousChunkF - snappedChunkF` is the same eps the shader's `subChunkOffsetNDC` is built from.
3. **Per beat, decide past vs future on the chunk-pair grid and reproduce the shader's NDC formula exactly.** Each shader vertex at `chunkInWindow = k` aggregates the two raw chunks `[base_k, base_k + 1]` and the trapezoid between adjacent vertices interpolates that aggregate over a 2-chunk audio window. The vertex's *continuous* time anchor is therefore the chunk-index midpoint `base_k + 0.5`, not `base_k` and not the audio-midpoint `(base_k + 1) × peakDur`. Inverting that gives `chunkInWindow_past = (drawnAbove - 1) + (chunksFromSnapped + 0.5) / chunksPerColumn` and `chunkInWindow_future = (chunksFromSnapped - 1.5) / chunksPerColumn`. Plug each into the shader's NDC formula and add the matching `subChunkOffsetNDC` (`epsChunks × 0.5 / ((drawnAbove - 1) × chunksPerColumn)` for past, `epsChunks × 1.5 / ((drawnBelow - 1) × chunksPerColumn)` for future). Convert NDC → drawable pixel via `(1 - timeNDC) × axisDrawablePx / 2` and divide by `displayScale` to land in logical pixels.
4. **Boundary policy.** `inFuture = chunksFromSnapped >= 1.0` puts the cut-over at the midpoint of the past column's last 2-chunk window. Past handles `chunksFromSnapped < 1`, future handles the rest. Both formulas extrapolate continuously across the cut-over; the small ~1-drawable-px discontinuity that remains there is intrinsic to the past and future regions having different `dNDC/dc` slopes and is one-shot per beat (not accumulating), so it reads as a sub-pixel position adjust rather than a drift.

Rate audit (Retina, `drawableHeight = 640`):

* Past, between snaps: `d(NDC)/dT = (1/peakDur) × 0.5 / ((drawnAbove - 1) × 2)` ⇒ `d(pixel)/dT = -axisDrawablePx × 0.5 × dNDC/dT = -80 / (79 × peakDur) = -1.0127 / peakDur` drawable px/s. Same as the renderer's measured past rate.
* Future, between snaps: `d(NDC)/dT = (1/peakDur) × 1.5 / ((drawnBelow - 1) × 2)` ⇒ `d(pixel)/dT = -240 / (239 × peakDur) = -1.0042 / peakDur` drawable px/s. Same as the renderer's measured future rate.
* At a snap (`snappedChunkF` jumps by 2, `epsChunks` drops from 2 to 0): the `frac` step inside the past formula contributes `+0.5 / (drawnAbove - 1)` to NDC and the `subChunkOffsetNDC` drop contributes `-0.5 / (drawnAbove - 1)` — they cancel, NDC is C¹-continuous across snaps. Same identity holds for future with the `1.5` factor. The grid is therefore drift-free relative to the waveform; the only residue is the one-time past/future-cut discontinuity per beat described above.

**False starts during this round (documented to stop us re-walking them).**

*Attempt 1* used `chunkInWindow_past = (drawnAbove - 1) + chunksFromSnapped / chunksPerColumn` and `chunkInWindow_future = (chunksFromSnapped - 2) / chunksPerColumn`. The user reported "back to not good… flickering and the grid is moving over". Cause: the boundary between past and future at `chunksFromSnapped = 1` evaluates to NDC `0.5 - 0.25/(drawnAbove-1)` on the past side and NDC `0.5 + 0.75/(drawnBelow-1)` on the future side. With `drawnBelow = 3 × drawnAbove` that is a ~1-logical-pixel cliff every time a beat crosses the playhead, perceived as the grid jumping sideways.

*Attempt 2* swapped to `chunkInWindow_past = (drawnAbove - 1) + (chunksFromSnapped + 0.5) / chunksPerColumn` and `chunkInWindow_future = (chunksFromSnapped - 1.5) / chunksPerColumn`, on the (incorrect) assumption that vertices anchor at chunk-index midpoint `base_k + 0.5`. That formula made the snap boundary continuous but **broke the past/future boundary worse**: at `chunksFromSnapped = 1` past evaluates to `chunkInWindow = 149.75` and future to `-0.245` for typical layout, yielding a `~0.0034 NDC ≈ 1 logical px` step exactly at the playhead. The user described the result as "much worse… also scrubbing a song is terrible now"; the latter likely came from the visual jump rendering near the user's drag-induced playhead motion and reading as audio acceleration on long tracks.

*Attempt 3 (current).* The right embedding is `chunkInWindow_past = (drawnAbove - 1) + chunksFromSnapped / chunksPerColumn` and `chunkInWindow_future = chunksFromSnapped / chunksPerColumn`, region cut at `chunksFromSnapped > 1`. This is the only formula in the family that is **simultaneously** continuous at the snap boundary (`epsChunks` rollover, see derivation above) AND at the past/future cut: with `drawnBelow = 3 × drawnAbove` the boundary identity `(0.5 - 0.25/(drawnAbove-1)) == (0.5 - 0.75/(drawnBelow-1))` holds algebraically, so both expressions agree to within the integer rounding `drawnAbove` itself carries. Anchor identity: a beat at the exact snapped chunk (`chunksFromSnapped = 0`) lands at NDC 0.5 with zero slide — i.e. directly on the playhead hairline — which is the visual property that defines "the grid is aligned to the audio". Neither earlier attempt had that property.

Why not "just fix the shader"? Changing `chunksVisible - 1` to `chunksVisible` in the shader would shift every column 1 drawable pixel toward the region center, leaving a 2-drawable-pixel visual gap at the playhead boundary (and another at the bottom of the future region). Centering with `(chunkInWindow + 0.5) / chunksVisible` would only move the gap, not close it, because the past and future strips meet via separate draw calls at NDC 0.5 and the renderer makes no provision for spanning the boundary continuously. Fixing it in the overlay keeps the waveform's visible appearance identical to what the user has approved through the previous flicker / smoothness / static-grid rounds and contains the math inside the layer that knows it has to match the renderer's geometry. No shader changes shipped.

**Follow-up to round 5 — static beat-grid Canvas.** After the round-5 changes shipped, the user reported "the beatgrid is static its not scrolling" in Prep mode. Diagnosis: SwiftUI's `Canvas` is opaque to the framework's diff machinery — the draw closure is just a `@escaping (GraphicsContext, CGSize) -> Void` and SwiftUI cannot tell what state it depends on. When a `Canvas` lives inside a `TimelineView` and the `TimelineView`'s `context.date` is *not* referenced inside the Canvas closure, SwiftUI treats successive Canvas widgets as input-stable across the timeline ticks and caches the draw output. The TimelineView's body re-evaluates per schedule (60 Hz for the beat-grid overlay, 2 Hz for the track-overview bracket), but the Canvas underneath never re-invokes its draw closure, so it never re-reads the `WaveformRenderSnapshot.lastDrawnPlayheadSecsUnclamped` field that the Metal renderer publishes each frame. Visible symptom: the grid stays frozen at whatever position the first paint landed on while the Metal waveform underneath scrolls smoothly. The fix is the canonical Apple idiom from the `TimelineView` + `Canvas` documentation: bind `context.date` inside the Canvas closure (even if the value is otherwise unused) so the closure has a per-tick captured dependency and SwiftUI invalidates the draw on every TimelineView tick. Two sites needed it:

* `WaveformView.beatGridOverlay` — beat-grid Canvas on the playing strip; reads `renderSnapshot` for playhead + beats and now pins the draw to `context.date` so the per-Metal-frame snapshot updates reach the screen.
* `TrackOverviewView.body` — bar + bracket Canvas wrapped in `TimelineView(.periodic(from: .now, by: 0.5))` as part of round 5; same bug, same fix. Pre-fix the playhead bracket would also have stayed at its first-paint position even though `engine.position(deckIdx:)` is being re-read fresh — the read happens inside the Canvas closure that wasn't being re-invoked.

The `LiveDeckTimeText` view is not affected because its content (`Text(...)`) is not opaque to SwiftUI's diff — when the formatted string changes, SwiftUI's structural compare picks it up and re-renders the text widget. The `Canvas` opacity is the specific reason that pattern needs the explicit `context.date` token.

### Follow-up — waveform scrub responsiveness and playback smoothness (closed)

Dogfood validation (2026-05-20) confirmed the M11d.5 follow-up work is done:
playing-deck scrub is immediate again, idle playback scroll is smooth, and the
library browser selection/drag rewrite from the same pass holds up in daily use.
Landed in `01291cd` (`CVDisplayLink` + input yield + ingest skip + GPU
catch-up). **Beat-grid overlay sub-pixel jitter** remains as deferred polish
(`UI-BACKLOG` U-18); it does not block further grid/BPM work.

---

### Known deferrals after round 5

* **`DeckState.durationSecs` is still on `model.deck{A,B}`.** It republishes once at load time (deck transitions from `hasTrack = false` to `hasTrack = true` with a non-zero duration) and never again during normal playback. That's not a per-second cost path so it's not worth moving onto the `TimelineView` side; the `seekDeck` and `scrub` clamps both consume it at gesture time and reading from `pos.durationSecs` instead would just add an FFI call to those handlers without a real benefit.
* **2 Hz TimelineView tick rate.** Picked deliberately to keep the M:SS rollover snappy without paying for a per-display-refresh closure body. If a future "tenths of a second" sub-display lands (a v1.x cue-work affordance), that consumer will want its own `TimelineView(.periodic(from: .now, by: 0.1))` at the leaf level; the architecture supports that cleanly because the leaf views own their timelines.
* **First-paint placeholder text.** Live time text reads `engine.position` inside the timeline closure; on the very first paint the closure may run a few ms before the engine's `pollDecks` has populated `pos.elapsedSecs` for a track that was just loaded. The renderer reports zero in that case, which renders as `"00:00"` / `"-00:00"` for one or two ticks — same visual behaviour the round-4 path had at the same moment, just sourced from `engine.position` instead of `DeckState`. Not user-visible at the 0.5 s cadence.

---

<a id="m11c2"></a>
## M11c.2 — Key detection (Camelot canonical)

Extends the M11c.1 lazy-analysis chassis with musical-key detection. `analyze_track` now decodes the file once and runs both `dub-bpm::analyze_beat_grid` and the new `dub-spectral::analyze_key` pipeline, upserting the auto results into `track_beatgrids` and `track_keys` respectively. The browser Key column reads from the active `track_keys` row in canonical Camelot, with a click-to-toggle musical-notation mode that persists across launches.

### Architecture choices

1. **Dedicated 4096-point chroma STFT, not the M7.5 `SpectralFrameStream`.** The PRD §8.3.2 draft suggested reusing the 1024-point STFT the BPM analyser feeds, but the resolution / semitone-width ratio breaks the binning: at 44.1 kHz, a 1024-point FFT has 43 Hz / bin, but a semitone at E4 (329 Hz) is only 19 Hz wide. E4's spectral energy lands almost entirely in bins centred on D and F, and round-to-nearest pitch-class binning silently rotates the chroma profile. The new pipeline uses a private 4096-point Hann FFT (~10.8 Hz / bin) with 1024-sample hop — 1.5–3 bins per semitone across the analysis band. Same `realfft` + Hann + log-of-power building blocks as `SpectralFrameStream`, just sized for harmonic analysis instead of onset detection.

2. **Linear-interpolation chroma binning, not round-to-nearest.** A bin that sits exactly between two semitones (chromaticity = 4.5) would lose all of its energy to one PC under round-to-nearest. The chosen rule distributes each bin's magnitude across its two neighbouring pitch classes by fractional chromaticity (chroma_idx = 7.76 → 24 % to PC 7, 76 % to PC 8). Degrades gracefully as the FFT-resolution-vs-semitone ratio worsens at higher frequencies instead of cliffing.

3. **Power spectrum (`|X|²`), not the BPM analyser's `ln(1 + λ · |X|)` compression.** Log compression with λ = 1000 is great for onset detection (it flattens spectral peaks so the spectral-flux derivative sees tonal content evenly) but actively harmful for chroma. A pure C4 tone produces a 4-bin Hann main lobe; under log-compression the leakage bin sitting on PC 11 (B, one semitone below C) carries ~80 % of the peak bin's compressed magnitude, drowning the chroma profile in neighbouring-semitone bleed. Power sharpens the peak (leakage drops to ~25 %), which is what we need to identify the tonal fundamental. The first draft used log-compression and recovered B major (1B) for a clean C major chord — visible failure mode, easy regression to write a test for.

4. **Krumhansl-Kessler (1982) templates, not a third-party library.** The 24 probe-tone profiles (12 major + 12 minor, rooted at C) are music-theory public-domain coefficients, not a library import. Pasted inline in `key.rs` with attribution; the alternatives (libchroma / madmom / Essentia) all add C++ FFI surface area and at least one of them is GPLv3 — we're already GPL because of Rubber Band but every additional GPL dep makes a future "MIT-license the engine, GPL the app shell" repartitioning harder. The K-K profiles also have the cleanest "obvious music theory" provenance for the PRD §11 license-justification trail.

5. **Camelot is the storage notation; musical is a display-only transform.** `track_keys.key_notation` is always Camelot (`8B` for C major). `original_notation` preserves whatever the source wrote verbatim (`C major`, `Cm`, `5d`, `8B`) so rekordbox-XML export round-trips exactly. The Camelot ↔ musical transform lives in a 24-entry Swift dictionary inside `LibraryView`; the alternative ("expose a `to_musical` method on the FFI") would have meant a per-row FFI call to render a column, and the Camelot family is small enough that the Swift-side table is honest. Once we ship the M11d.7 customizable-columns view it will also need this transform for the "show source's original notation" column.

6. **Cross-validation key-disagreement flag is computed at row-fetch time, not at write time.** The naive alternative ("denormalise into a `track_metadata.key_disagreement` column updated whenever a `track_keys` row is INSERT/UPDATE/DELETE'd") trades read-time correlation cost for write-time triggers and consistency hazards. The chosen rule lives in `TRACK_ROW_SELECT` as a correlated subquery: `COUNT(DISTINCT substr(key_notation, 1, length-1)) > 1 FROM track_keys WHERE track_id = t.id`. Stripping the trailing letter is the SQL-level approximation of `dub_spectral::camelot_keys_disagree` (which treats `8B` ↔ `8A` as a relative-key pair the K-K templates legitimately can't disambiguate). In v1.0 only the `auto` source writes, so the flag is always `false`; M11e (Serato importer) flips the flag automatically without any Swift code changes.

7. **`stamp_analysis_cache` writes both `has_active_grid` and `has_active_key` in one go.** A previous draft kept them in separate calls so a key-only re-analysis could leave the grid flag untouched. That was over-engineering: every `analyze_track` call now decodes once and runs *both* analysers, so both flags reflect the same instant. The two-flag schema column is still useful for future "what kind of analysis did this fingerprint receive?" introspection (a future v1.x debug overlay; matters for the imported-grid-but-no-key case where a Serato importer landed a grid without a key field).

8. **Pure-triad test inputs assert "Camelot family", not the exact A/B half.** A C-E-G chord has the same pitch-class set as A minor (after K-K minor template weighting); the algorithm provably cannot disambiguate them from triad-only input. PRD §8.3.2 exempts this exact case from the disagreement indicator: "Relative-major … is not flagged because it's a legitimate template ambiguity". The unit tests at the `dub-spectral` layer assert tonic family (Camelot number); the tonic-correct path uses richer I-IV-V-I progressions that span the full diatonic set (7 distinct PCs = the major scale) and resolve cleanly. Wrote the first version with strict assertions on pure triads, watched it fail, and codified the lesson in the test comment so future contributors don't waste the same hour.

9. **Sign convention bug caught by a one-test regression guard.** The pitch-class formula `pc = (semitone + 9) mod 12` (where `semitone = 12 · log₂(f / A4)`) is correct because A is MIDI 69 → PC 9. The first draft wrote `- 9` instead, which silently rotated every chroma profile 9 / 12 of the way round the wheel — every track came back as something nine semitones off. Now there's a regression test that asserts the FFT bin closest to 440 Hz puts majority weight on PC 9 and the bin closest to 261.63 Hz puts majority weight on PC 0. Anyone who flips the sign by accident will fail this immediately.

### Schema

V3 migration in `crates/dub-library/src/schema.rs`:

```sql
CREATE TABLE track_keys (
    id                INTEGER PRIMARY KEY,
    track_id          TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source            TEXT    NOT NULL CHECK (source IN
                      ('serato', 'traktor', 'rekordbox', 'itunes',
                       'mixedinkey', 'id3', 'auto', 'user')),
    key_notation      TEXT    NOT NULL,
    original_notation TEXT,
    confidence        REAL,
    is_active         INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    captured_at       INTEGER NOT NULL,
    UNIQUE (track_id, source)
);
CREATE UNIQUE INDEX idx_one_active_key_per_track
    ON track_keys(track_id) WHERE is_active = 1;
ALTER TABLE analysis_cache ADD COLUMN has_active_key INTEGER NOT NULL DEFAULT 0;
```

Same partial-unique-index trick as `track_beatgrids` enforces "one active key per track" at the database layer.

### Files

* New `crates/dub-spectral/src/key.rs` (1000+ lines, comprehensive doc / regression tests).
* `crates/dub-spectral/Cargo.toml` adds `thiserror`.
* `crates/dub-library/src/analysis.rs`: `AnalysisOutcome` extended with `camelot` / `tonic_pc` / `is_major` / `key_confidence` / `key_auto_is_active` / `wrote_key`; `analyze_track` runs both DSPs in one pass; new private helpers `has_non_auto_active_key`, `upsert_auto_key`.
* `crates/dub-library/Cargo.toml`: adds `dub-spectral`.
* `crates/dub-library/src/schema.rs`: `SCHEMA_VERSION = 3`, new `V3_MIGRATION`, two new tests.
* `crates/dub-library/src/db.rs`: `TrackRow.key_disagreement` field; `TRACK_ROW_SELECT` reads `ak.key_notation` from active `track_keys` row (not `i3.key`) plus computes `key_disagreement` via correlated subquery.
* `crates/dub-ffi/src/lib.rs`: `LibraryAnalysisOutcome` extended with key fields; `LibraryTrack.key_disagreement`.
* `apple/Dub/Performance/LibraryView.swift`: `KeyNotationMode` enum + `@AppStorage("libraryKeyNotationMode")`; Key column renders Camelot or musical with click-to-toggle, ⚠ glyph on disagreement, tooltip shows the other notation.
* Regenerated Swift bindings under `apple/DubShared/Sources/DubCore/Generated/`.

### Test coverage

* **`dub-spectral`**: 27 tests including silence / too-short / invalid-channels guard, A4 → PC 9 regression, chroma-table tonal-band-only coverage, pure-triad → diatonic-family family-8-or-9, C major progression → 8B, G major progression → 9B, stereo downmix invariance, Camelot ↔ parsing round-trip, relative-pair tolerance, parallel-pair flag, malformed-Camelot suppression.
* **`dub-library`**: 100 tests including analyse-track writes Camelot key for C major progression, doesn't steal active from higher-priority Serato source, is idempotent on re-run, key-column SQL reads from active `track_keys`, key-disagreement flags only on parallel pairs, schema-v3 active-key constraint, `has_active_key` column lands on v3.
* **`dub-ffi`**: 28 tests (M11c.1 baseline preserved; the analyse / `is_analyzed` smoke tests now also exercise the key-bearing path because the underlying outcome carries both fields).

### Known deferrals

* **Imported keys.** M11c.2 wires the disagreement plumbing but no source writes a non-auto row yet. M11e (Serato importer) will flip the ⚠ glyph for real-world disagreements between Serato's MixedInKey-style keys and Dub's auto detection.
* **User correction.** Right-click → "Set key…" is a future v1.x affordance; v1.0 ships read-only on user keys. The schema's `('user')` source value is reserved for when the gesture lands.
* **PRD §8.3.2 cross-validation indicator on the row.** The flag is plumbed; the Key column shows ⚠. The PRD also hints at a column-header-level summary ("23 tracks have a key disagreement") which slots cleanly under M11d.7 customizable columns.
* **Waveform sidecars / LUFS / true-peak.** Deferred per the user-approved milestone order (key detection first, sidecars after).

### PRD churn

* §8.3.2 row updated from "key detection (Camelot canonical)" to "✅ shipped" with full implementation summary.
* No PRD principle changes; this milestone is a pure implementation of the §8.3.2 spec.

---

<a id="m11c1"></a>
## M11c.1 — Lazy auto-beatgrid + analysis lifecycle

Closes the M11c follow-up gap that left `dub-bpm` (shipped in M7.5 + M8.1) disconnected from the library. The Dub-native auto-grid is now produced lazily on first deck load (or via the LibraryView right-click batch action), persisted to `track_beatgrids(source='auto')`, and surfaced everywhere the browser reads BPM. Tracks that have never been analyzed dim out and show an em-dash in the BPM column; once analysis completes the row snaps to full opacity and the BPM badge fills in.

### Architecture choices

1. **Analysis is lazy, not eager, and it lives in `dub-library`, not in the importer.** Two alternatives were on the table. The first ("analyze synchronously during import") would have forced the importer to wait for every file's decode + spectral-flux pass; a 5 000-track folder import would balloon from ~3 minutes (fingerprint-only) to ~3 hours (fingerprint + beat grid). The second ("kick analysis into a background thread per imported file") would have created a thundering herd of decoder threads competing with the user's preview-load and made the "Just Imported" smart crate dishonestly green for tracks that hadn't yet finished analyzing. The chosen model matches Serato / Traktor / rekordbox: import stays fast, analysis runs on demand. `WaveformAppModel.loadTrack` flips on a `Task.detached(priority: .background)` after the deck reports success; the LibraryView right-click context menu drives the batch path. Both flows funnel through `Library::analyze_track(track_id)`, which lives in a new `crates/dub-library/src/analysis.rs` module beside the existing db / dedupe / importer helpers.

2. **`analyze_track` is keyed by canonical track id, not by audio buffer.** A buffer-keyed API would have been "cheaper at the call site" for the deck-load path (the engine already holds decoded samples in RAM), but it forces every batch-analyze caller to re-decode anyway and bifurcates the analysis surface. The track-id API lets `analyze_track` itself own the decode-via-`dub-io::Track::load_from_path` step. The cost is a second decode pass on deck-load (engine decodes for playback, analyzer decodes again on `.background` priority), but a 3-minute MP3 decodes in ~600 ms on the M2 Air and the background-priority pass loses any contention with the audio path. Matches Serato's actual behavior — their analyzer is a separate process that re-reads the file.

3. **`is_active` honours PRD §8.3 priority at write time, not at read time.** The partial unique index `idx_one_active_grid_per_track ON track_beatgrids(track_id) WHERE is_active = 1` already enforces "only one active grid per track" at the database layer. `analyze_track` checks for any non-auto active grid before deciding whether to make the auto row active: if Serato / rekordbox / Traktor has already imported and claimed `is_active = 1`, the auto row lands with `is_active = 0` (so the user can still switch to it manually via the future active-grid context menu); if no other source is active, the auto row claims active. Doing the priority dance at write time keeps the read path trivial (`LEFT JOIN track_beatgrids WHERE is_active = 1`) and dodges a TOCTOU race where two concurrent imports could otherwise both try to land `is_active = 1`.

4. **`analysis_cache.analyzed_at IS NOT NULL` is the source of truth for "is analyzed", not `EXISTS(track_beatgrids)`.** A track legitimately can have no detectable grid — silence stems, ambient pieces, a misnamed `.aiff` of room tone. Using `EXISTS(track_beatgrids)` as the predicate would leave those rows dim forever and re-analyze them on every Space-load. The current contract: every call to `analyze_track` stamps `analyzed_at = now` regardless of whether a grid was found. The browser's dim cue and the deck-load hook both consult `is_track_analyzed`, which checks the stamp, not the grid. The user can still force a re-run via "Re-analyze Selected".

5. **The browser BPM column reads exclusively from the active grid, not from ID3 BPM.** A previous draft kept `bpm = COALESCE(track_beatgrids.bpm, i3.bpm)` so ID3-tagged tracks would show *something* in the BPM column even before analysis. That was wrong twice. First, PRD §8.3 calls out that ID3 BPM has no anchor and can't be bound to the deck — surfacing it in the browser column creates a "row shows 90, deck shows 174" UX inconsistency the moment the auto grid lands. Second, it would hide the very signal the user needs to scan: "which rows have I analyzed?". The new SELECT reads `track_beatgrids.bpm` only; unanalyzed rows render as `—`. ID3 BPM is preserved verbatim in `track_metadata_source.bpm` for the future "per-source disagreement view" (M11d.7) and never leaks into the active-priority view.

6. **`TrackRow.is_analyzed` ships on every row, not via a per-row FFI call.** The LibraryView dim modifier and the Re-analyze affordance both need to know `is_analyzed` for the rows currently on screen. The naive answer ("Swift queries `is_track_analyzed` per visible row") would issue one FFI call per visible row per refresh; a 5 000-row listing would hit the lock 5 000 times per scroll. The chosen alternative folds the flag into `TRACK_ROW_SELECT` itself via a `LEFT JOIN analysis_cache ON ac.fingerprint_id = t.fingerprint_id` and a `CASE WHEN ac.analyzed_at IS NOT NULL THEN 1 ELSE 0 END` projection. One join, one row read; the planner picks the `analysis_cache.fingerprint_id` primary key automatically.

7. **`analysisGeneration` is a `UInt64` counter, not a `Set<String>` or a Combine stream.** SwiftUI's `Table` plus `onChange(of:)` is the cheapest possible refresh trigger; a single counter that bumps when any analysis completes is enough for the LibraryView to re-fetch its visible page. A `Set<String>` of newly-analyzed ids would let the view do a targeted in-place update (`tracks[i].bpm = …`), but the FFI shape returns a full `LibraryTrack` per row and the per-row mutation would have to reconstruct the new `LibraryTrack` from the `LibraryAnalysisOutcome` and the in-memory original, which doubles the surface area for a marginal latency win. The full re-fetch under main-actor coalescing finishes in ~15 ms on a 5 000-row listing on the M2 Air, comfortably under the user-noticeable threshold.

8. **`analyzeTracks` walks serially with `await Task.detached(...).value`, not in parallel.** Parallel batch analysis (`TaskGroup` with a per-CPU concurrency cap) would shave wall-clock time on a 200-track batch by ~3×. But every `analyze_track` call acquires the library lock for the full decode + DSP run; concurrent calls would serialise on that lock anyway, and a parallel queue would only buy us "decode N, then write 1, then decode N, then write 1, …" thrashing. Serial means each track's decode + analysis happens with the lock held exactly once and the LibraryView's intermediate queries (`list_tracks`, `refresh*`) can interleave cleanly between tracks. The `analysisBatchTotal` / `analysisInFlightCount` published counters drive the footer's "Analyzing N of M…" indicator.

9. **`analyzingTrackIds: Set<String>` deduplicates concurrent kickoffs.** The user can rapid-fire Space-loads on multiple tracks; the deck-load hook fires `ensureTrackAnalyzed` for each, and the user can then right-click and pick "Analyze Selected" while one of those is still in flight. Without the in-flight set the second call would attempt to acquire the library lock the first call is already holding, blocking the main actor until the first analysis finishes. The set is guarded on the main actor and consulted before queueing each background `Task`. Idempotent at every entry point.

### Implementation

* **`crates/dub-library/Cargo.toml`** — adds `dub-bpm = { workspace = true }`. The DSP itself has been live since M7.5; this is purely a wiring change.
* **`crates/dub-library/src/analysis.rs`** (new) — `AnalysisOutcome` struct + `Library::analyze_track`, `Library::is_track_analyzed`, plus three private helpers (`track_analysis_keys`, `has_non_auto_active_grid`, `upsert_auto_beatgrid`, `stamp_analysis_cache`). The module is wired in via `crates/dub-library/src/lib.rs` (new `mod analysis; pub use analysis::AnalysisOutcome`).
* **`crates/dub-library/src/error.rs`** — four new typed variants: `TrackNotFound`, `TrackHasNoFingerprint`, `TrackHasNoFile`, `DecodeFailed`. The first three surface stale-id and missing-file conditions the caller can act on; the last surfaces decoder / DSP failures with a one-line reason. All four flatten across the FFI into `LibraryFfiError::QueryFailed` (Swift consumers only need the description string).
* **`crates/dub-library/src/db.rs`** — `TrackRow` gains `is_analyzed: bool`. `TRACK_ROW_SELECT` adds the `track_beatgrids ag (is_active = 1)` and `analysis_cache ac` joins and switches the `bpm` projection from `i3.bpm` to `ag.bpm`. `TrackSortKey::Bpm` retargets to `ag.bpm` so the column-header sort still drives the active-grid BPM. `track_row_from_columns` reads the new `is_analyzed` column at index 15. Two pre-existing tests (`list_tracks_assembles_priority_chain_correctly`, `list_tracks_sorted_nulls_sort_last_in_both_directions`) updated to assert the new contract; one new test (`list_tracks_bpm_column_reads_from_active_beatgrid`) lands an auto grid by hand and asserts both `bpm` and `is_analyzed` flip together.
* **`crates/dub-ffi/src/lib.rs`** — new `LibraryAnalysisOutcome` UniFFI record mirroring `dub_library::AnalysisOutcome`. `LibraryTrack` gains `is_analyzed: bool`. `DubLibrary` gains `analyze_track(track_id: String) -> LibraryAnalysisOutcome` and `is_track_analyzed(track_id: String) -> bool`. Two new FFI smoke tests verify the closed-handle and unknown-track paths return `QueryFailed` cleanly.
* **`apple/Dub/MainView.swift`** — `WaveformAppModel` gains `@Published` `analysisGeneration: UInt64`, `analysisInFlightCount: UInt32`, `analysisBatchTotal: UInt32`, and a private `analyzingTrackIds: Set<String>`. Two new methods: `ensureTrackAnalyzed(trackId:)` (cheap, fire-and-forget) and `analyzeTracks(_:forceReanalyze:) async` (batch path). `recordLibraryLoadIfApplicable` calls `ensureTrackAnalyzed` after the `record_load` write succeeds.
* **`apple/Dub/Performance/LibraryView.swift`** — every column's cell wraps its content in a new `DimUnanalyzed` `ViewModifier` (opacity 0.55 for unanalyzed rows, 1.0 otherwise). The Table grows a `contextMenu(forSelectionType: LibraryTrack.ID.self)` block with "Analyze Selected" / "Re-analyze Selected" items, both disabled while a batch is in flight. Footer adds an "Analyzing N of M…" progress line. `refreshTracks` learns a `preserveSelection: Bool` knob so the analysis-generation-triggered refresh doesn't drop the user's Space-load target.
* **`apple/DubShared/Sources/DubCore/Generated/`** — bindings regenerated via `scripts/build-xcframework.sh` after the FFI surface changed. `dub_ffi.swift`, `dub_ffiFFI.h`, `module.modulemap` rewritten in lockstep.

### Tests

`cargo test -p dub-library` adds six new tests in `analysis::tests`:

* `analyze_track_writes_auto_grid_and_stamps_cache` — generates a 120 BPM click-track WAV, runs the full pipeline end-to-end, asserts one `track_beatgrids` row with `source = 'auto'`, `is_active = 1`, an anchor in `[0.0, 2.0)`, and a BPM near 60 / 120 / 240 (the lock-on octaves the M8.1 algorithm legitimately selects).
* `analyze_track_is_idempotent_on_re_run` — runs `analyze_track` twice on the same fixture, asserts exactly one auto row remains (re-analysis upserts, not duplicates).
* `analyze_track_keeps_existing_active_grid_when_other_source_present` — pre-inserts a `source = 'serato'` row with `is_active = 1` and asserts the subsequent auto run lands `is_active = 0` so the partial-unique-index constraint is not violated.
* `analyze_track_on_silence_marks_analyzed_without_writing_grid` — generates a true-silence WAV, asserts the outcome carries `wrote_grid = false`, no row in `track_beatgrids`, and `is_track_analyzed` returns true (no auto-retry loop).
* `is_track_analyzed_returns_false_for_fresh_track` — happy-path negative case.
* `analyze_track_returns_typed_errors_for_unknown_track_and_missing_file` — confirms the `TrackNotFound` typed-error path.

Plus two new tests in `db::tests`:

* `list_tracks_bpm_column_reads_from_active_beatgrid` — lands an auto grid by hand, asserts `TrackRow.bpm` reflects it and `TrackRow.is_analyzed` flips true.
* The two existing tests that previously asserted ID3-sourced BPM are rewritten against the new contract (active-grid BPM, em-dash on no grid).

`cargo test -p dub-ffi` adds two FFI smoke tests:

* `analyze_track_on_closed_library_returns_query_failed` — `DubLibrary::new()` without `open_at` produces `QueryFailed` for both `analyze_track` and `is_track_analyzed` instead of panicking on the inner-Option unwrap.
* `is_track_analyzed_returns_false_for_unknown_track` — open library + bogus track id flattens to `QueryFailed`.

Full workspace: `cargo test --workspace` reports **685/685 passing** (+9 over M11d.4 baseline); `cargo clippy --workspace --all-targets -- -D warnings` is clean. `xcodebuild -scheme Dub` succeeds with the only warning being the pre-existing `LibraryTrack: Identifiable` retro-conformance lint.

### Known deferrals / non-goals

* **Grid-disagreement indicator (M11d.3 deferral closes here).** The auto grid is now reliably written; the M11e Serato importer will land the cross-source grid that drives the indicator. The browser-side wiring is straightforward and gets handled as part of M11e to keep the indicator change-set adjacent to the importer that produces the cross-source data.
* **Half-tempo / double-tempo correction UI.** `dub-bpm` already exposes `analyze_bpm_with_range` for genre-aware overrides; the LibraryView right-click "Force half tempo" / "Force double tempo" affordance is a v1.x addition once we have a clear UX for it. Not blocking v1.0.
* **Cancellable batch analyze.** "Analyze Selected" runs to completion on the current selection; there's no "Stop" button. A future addition would need a cancellation token threaded through `Library::analyze_track` so the inner `dub-bpm` call can bail mid-DSF-pass. Skipped for v1.0; the per-track ~1.5 s wall-clock means even a 200-track batch finishes inside ~5 minutes.
* **Adaptive priority by genre.** PRD §8.3 lists `Serato > rekordbox > Traktor > auto` as the default priority order. The current code hard-codes "any non-auto source wins over auto"; a per-source priority table (so a user with both Serato and Traktor imports can pick which wins) belongs with the M11e Serato importer where the second source actually starts producing rows.
* **Key detection (M11c.2 next).** Distinct DSP, distinct schema (v3 `track_keys` table), distinct row in the browser. The lazy-analysis pipeline this milestone builds is the chassis M11c.2 reuses; only the inner DSP changes.

### PRD churn

* §12 M11c.1 row marked ✅ shipped with a one-line summary of the lazy-analysis + browser-column-rewrite scope.

### Commit boundary

This entry corresponds to the M11c.1 commit set: `docs/PRD.md` (M11c.1 row update), `docs/SHIPPED.md` (this section), `crates/dub-library/Cargo.toml` (dub-bpm dependency), `crates/dub-library/src/analysis.rs` (new module + tests), `crates/dub-library/src/lib.rs` (re-exports), `crates/dub-library/src/error.rs` (four new typed variants), `crates/dub-library/src/db.rs` (`TrackRow.is_analyzed`, SELECT rewrite, two updated tests + one new test), `crates/dub-ffi/src/lib.rs` (`LibraryAnalysisOutcome` record, `LibraryTrack.is_analyzed`, two new methods + two new tests), `apple/Dub/MainView.swift` (model glue + `ensureTrackAnalyzed` + `analyzeTracks`), `apple/Dub/Performance/LibraryView.swift` (`DimUnanalyzed` modifier + contextMenu + footer progress + `preserveSelection` refresh knob), `apple/DubCore.xcframework/` + `apple/DubShared/Sources/DubCore/Generated/` (rebuilt FFI surface).

---

<a id="m11c4"></a>
## M11c.4 — Lazy fingerprint (import-fast, analyze-on-demand)

Defers Chromaprint computation and dedupe from the cold import path to the first-deck-load analysis pass. Imports become metadata-only: no decode, no fingerprint, no dedupe at the moment the user drops a folder on the app. The fingerprint materialises later, inside the `Library::analyze_track` pass the deck already triggers on first load.

### Why

A user reported that importing a large drum and bass folder was "very slow" even though the app deliberately does not run BPM or waveform analysis at import. Profiling with the M11c.4-era `crates/dub-library/examples/profile_import.rs` confirmed the cost breakdown on commodity SSDs:

* `decode` (full file → `Vec<f32>` via `dub-io`): ~70 % of import wall-clock.
* `fingerprint` (Chromaprint over the decoded samples): ~25 %.
* Everything else (stat, SQL writes, metadata-source rows): ~5 %.

The decoded samples are not used during import for anything except feeding Chromaprint. The fingerprint is only consumed by the dedupe-merge decision (which the v1.0 policy explicitly refuses to auto-fire) and by `analysis_cache` keying (which is keyed by fingerprint id but doesn't need the id at import time). The full ~95 % decode + Chromaprint cost was therefore being paid for work that is either unused in v1.0 (auto-merge) or paid for redundantly anyway (the deck-load analyzer decodes the same file again to compute BPM + key).

The fix is straightforward: stop doing that.

### What changed

**Import (cold path) is metadata-only.** `dub_library::importer::import_one` now calls `dub_io::read_metadata(path)` (a symphonia probe with no sample decode) and writes a `tracks` row with `fingerprint_id = NULL` and `duration_ms = NULL`, plus a `track_files` row, plus the `id3` and `filename` rows in `track_metadata_source`. No `Fingerprint::compute_from_f32` call, no `find_fingerprint_neighbours` query, no `dedupe::decide` loop. The `summary.merged` and `summary.sibling_versions` counters stay in the `ImportSummary` struct for API stability but always come back zero.

**`Library::insert_track` and `tracks.duration_ms` accept NULL.** `insert_track`'s signature changed from `(fingerprint_id: i64, duration_ms: u32, ...)` to `(fingerprint_id: Option<i64>, duration_ms: Option<u32>, ...)`. The schema's `tracks.fingerprint_id` and `tracks.duration_ms` columns were already nullable since M11a, so no migration was required.

**`Library::attach_fingerprint(track_id, fingerprint_id, duration_ms)`** is the new bridge: idempotently updates `tracks.fingerprint_id` and `tracks.duration_ms` for a track that was inserted with NULL. The `WHERE fingerprint_id IS NULL` guard closes the race window when two `analyze_track` calls happen concurrently against the same UUID (the loser's UPDATE matches zero rows; it then re-reads the winner's id from `tracks`).

**`Library::analyze_track` computes and attaches the fingerprint inline.** The flow now is:

1. Look up the track's path + (optional) existing fingerprint id.
2. Decode the file via `dub_io::Track::load_from_path`.
3. **If `fingerprint_id` was NULL**: compute Chromaprint over the just-decoded samples, `upsert_fingerprint`, `attach_fingerprint`. The resulting id keys the rest of the pass.
4. Run BPM analysis (`dub_bpm::analyze_beat_grid`) and key analysis (`dub_spectral::analyze_key`) on the same decoded buffer.
5. Stamp `analysis_cache` keyed by fingerprint id.

The fingerprint pass is bolted onto work the user already paid for by loading the deck (the file is decoded once for BPM + key regardless), so the marginal cost is just the Chromaprint pass itself (~25–40 ms for a 5 min track on an M1).

**`list_missing_tracks` filters out NULL-fingerprint rows.** The Relocate matcher needs a stored fingerprint to compare against; tracks that have never been analyzed and have gone missing are not surfaced in the Relocate panel until they're analyzed. In practice tracks that have never been deck-loaded are also rarely ones the user cares enough about to relocate; a future v1.x sweep can analyze them on-demand from the Relocate UI.

### What does *not* happen at import any more

* **No auto-merge.** Two byte-identical files now both land as distinct `tracks` rows; v1.0 makes no attempt to collapse them. The `dedupe::decide` primitives and `find_fingerprint_neighbours` query are unchanged; a future "Find duplicates" library action (deferred to v1.x) will surface near-matches for manual user review.
* **No sibling-version registration.** A `Clean` / `Dirty` pair shows up as two unrelated rows. The version-token parser still records the per-source `version_token` column verbatim, so the existing browser indicator (PRD §8.5.3) keeps working as soon as a user opens the manual-review affordance.
* **No `sample_rate` / `channels` / per-codec stamping on `track_files` at import time.** The metadata-only probe doesn't reveal those without decoding. They get populated on first analyze pass as a side-effect of `Track::load_from_path` walking the codec params. This change is invisible to v1.0 UX because the LibraryView reads display data from `track_metadata_source`, not from `track_files`.

### Measured impact

`profile_import` against the user's 12-track `OST Guardians of the Galaxy` folder (a representative mid-bitrate MP3 set):

* Pre-M11c.4 (decode + fingerprint + dedupe + write): tens of seconds total, dominated by decode.
* Post-M11c.4 (metadata probe + write): **59 ms total wall-clock** for the same 12 files. ~5 ms / file, ~200 file/s throughput.

The PRD's §8.4 "lazy by design" rule now applies symmetrically to fingerprint, BPM, and key — none of them block the import.

### Architecture notes

1. **Race-safe attach.** The `WHERE fingerprint_id IS NULL` guard on `attach_fingerprint` makes the "two `analyze_track` calls land at the same time" case correctness-clean: at most one UPDATE succeeds, the loser re-reads the winning id from `tracks`. The losing call's `upsert_fingerprint` row becomes an orphan (a `fingerprints` row no `tracks` row points at), which a future v1.x sweeper can garbage-collect — the cost is one extra ~1.5 KB blob per race, which is fine.
2. **`duration_ms` populated lazily too.** The same `attach_fingerprint` call also writes `tracks.duration_ms` from the fingerprint's `duration_ms()`. Before first analyze, the browser's Duration column shows `—` (per the existing `Option<u32>` rendering rule); after analyze it shows the canonical value. This matches the existing BPM and Key column behaviour and stays consistent with the M11c.1 "row dims until analyzed" cue.
3. **Reverted the M11c diagram comment, kept the dedupe primitives.** The `crates/dub-library/src/dedupe.rs` module and `find_fingerprint_neighbours` / `find_track_owner_by_fingerprint_id` queries are unchanged and the dedupe unit tests still pass. M11c.4 strictly removes the **call site** in the importer, not the primitives, so the future "Find duplicates" action can reuse them verbatim.
4. **The `LibraryImportSummary` FFI record is unchanged.** Apple-side code that already reads `summary.merged` / `summary.sibling_versions` keeps compiling. Those fields just always come back zero from a v1.0 cold import.

### Files touched

* `crates/dub-library/src/db.rs` — `insert_track` signature now `Option<i64>` / `Option<u32>`; new `attach_fingerprint` helper; `list_missing_tracks` filters `fingerprint_id IS NOT NULL`.
* `crates/dub-library/src/importer.rs` — `import_one` rewritten to use `dub_io::read_metadata`; removed `run_dedupe`, `DedupeResolution`, `composed_display_string`; `write_metadata_rows` now takes `&TrackMetadata` not `&Track`; importer tests rewritten for the no-auto-merge behaviour.
* `crates/dub-library/src/analysis.rs` — `analyze_track` computes + attaches the fingerprint when missing, with a race-safe re-read; `track_analysis_keys` returns `Option<i64>` for the fingerprint id; two new tests (`analyze_track_attaches_fingerprint_when_missing`, `analyze_track_is_idempotent_when_fingerprint_already_attached`).
* `crates/dub-library/examples/profile_import.rs` — replaced the per-phase synthetic profiler with a real `import_folder` driver that reports the user-visible wall-clock.
* `docs/PRD.md` — §8.1 adds the "Lazy fingerprint" paragraph; §8.4 marks track analysis as "not at import time since M11c.4"; §12 milestones row.
* `docs/SHIPPED.md` — this section.

### Commit boundary

This entry corresponds to the M11c.4 commit set: `crates/dub-library/src/{db,importer,analysis}.rs`, `crates/dub-library/examples/profile_import.rs`, `docs/PRD.md`, `docs/SHIPPED.md`.

### Deferred

* **"Find duplicates" library action.** Once two tracks have been analyzed, their fingerprints are both in `fingerprints`; a future action queries `find_fingerprint_neighbours` and surfaces near-matches for manual review. v1.x, on demand.
* **Orphaned fingerprint sweep.** The race-loser side of `attach_fingerprint` leaves an unreferenced `fingerprints` row behind. The leak rate is bounded by the rate of concurrent analyse_track calls on the same UUID, which is roughly zero in single-user usage; a periodic `DELETE FROM fingerprints WHERE id NOT IN (SELECT fingerprint_id FROM tracks WHERE fingerprint_id IS NOT NULL)` sweep can run lazily at app launch in v1.x.
* **`tracks.duration_ms` from a fast probe.** Symphonia exposes `codec_params.n_frames` for most container formats, which would let `read_metadata` report duration without a decode. Wiring it in (and gracefully handling the VBR-MP3-without-Xing-header case where `n_frames` is `None`) is a one-afternoon follow-up. Until then, freshly-imported tracks show `—` in the Duration column until first deck-load.

---

<a id="m11c3a"></a>
## M11c.3a — BPM octave fix (perceptual tempo prior)

The first algorithmic slice of PRD §12 M11c.3 (perceptual tempo prior). Tap-to-grid shipped as [M11c.3b](#m11c3b), closing the milestone. After M11c.3a–f, real hip-hop / rap catalogs analyze at the audible tempo (95 BPM, not 190 BPM) on Default; DnB and genre-tagged library analysis use profile-aware passes documented in M11c.3c–f.

### Why

Two symmetric octave-error failure modes were dominating the user's library after M11c.1's auto-grid lifecycle landed:

* **Hip-hop / rap detected at 2x.** Westside Connection's "Gangsta Nation" (real 95 BPM, kick on 1+3, snare on 2+4, hi-hat 8th-note ostinato) was detecting at 191 BPM. The 8th-note hi-hat is louder in the spectral-flux ODF than the kick / snare backbeat, so the autocorrelation at the 190 BPM rate (2x the perceived tempo) had a higher harmonic-mean score than the autocorrelation at 95 BPM. The picker's tie-break ("prefer smaller lag = faster BPM on ties") biased that further toward 190.
* **Drum-and-bass detected at 1/2x.** Chase & Status "Baddadan" (real 175 BPM, kick on 1+3, snare on 2+4) was detecting at 87.54 BPM. The K-K skip-1 autocorrelation peak (kicks two beats apart) is structurally stronger than the K-S alternation peak (kick-to-snare one beat apart) because snares are quieter than kicks in the ODF. The autocorrelation at 87 BPM has higher confidence than at 174 BPM, and the picker reports the louder peak.

The two failure modes have the same root cause and **opposite required corrections**: rap wants the lower-BPM octave, DnB wants the upper-BPM octave. Pure autocorrelation cannot distinguish them without a perceptual prior — both peaks are legitimate periodic structure, and there is no signal in the spectral-flux ODF that says "this is rap, prefer the lower octave" vs "this is DnB, prefer the upper octave".

The PRD §8.3 fallback (`BpmRange` escape hatch via `dub thru --bpm-range MIN,MAX`) addressed this for power users on the CLI but offered nothing for the library importer's default behaviour. Real users have mixed-genre libraries — rap at 95, house at 125, DnB at 175 — and the default auto-detect path must land them all at the right octave without per-track CLI invocation.

### What changed

**Perceptual tempo prior** in `crates/dub-bpm/src/tempo.rs`. A piecewise-linear weight function `tempo_prior_weight(bpm)` multiplies each candidate's raw harmonic-mean score before lag selection:

* `bpm ≤ 60`: 0.20 (penalty floor — barely-musical territory)
* `60 → 95`: linear 0.20 → 1.00 (lifts 80 to 0.66, 86 to 0.79, 87.59 to 0.83, 90 to 0.89, 92 to 0.93)
* `95 → 175`: 1.00 (plateau covers hip-hop 95–105, house, techno, DnB 165–175)
* `175 → 200`: linear 1.00 → 0.30 (180 → 0.86, 190 → 0.58)
* `bpm ≥ 200`: 0.20

The plateau boundaries are not arbitrary. The lower edge sits at exactly 95 BPM because real-DnB raw-score ratios `score(high) / score(low)` measured across the M11c.3 corpus sit in [0.76, 0.94], and the worst-case clean DnB sample is Total Science / S.P.Y. "Gangsta" (Watch The Ride remix) at lag 59 / 87.59 BPM (raw 5.121) vs lag 30 / 172.27 BPM (raw 4.454), ratio 0.870. The prior must therefore drive `weight(87.59) < 0.87` so the upper octave wins on weighted score, which forces the plateau start at 95 BPM. The upper edge sits at 175 BPM because the symmetric double-time hip-hop failure modes peak at 180–195 BPM (8th-note hi-hat candidates); pulling the ramp start to 175 puts 180 cleanly into the ramp zone (weight 0.86) while still keeping every DnB candidate ≤ 175 BPM at full plateau. Algorithm parameter calibration sits inline in the function's doc comment for future readers.

**Plateau calibration history.** Three iterations were required to converge on the [95, 175] boundaries:

1. **Initial plateau [90, 178].** Resolved Bedhead / Baddadan / Backbone / Apocalypse but missed Chase & Status "Come Back": its lower-octave detection at lag 59 (87.59 BPM, just above the 90 plateau-start) received `weight = 0.94`, leaving the weighted score 1.4 % above the 172 BPM candidate. Come Back's raw ratio is 0.922 — close enough to 1.00 that a 0.94 weight wasn't sufficient to flip it.
2. **Plateau [92, 175].** Dropped `weight(87.59)` to 0.89, fixing Come Back. Total Science / S.P.Y. "Gangsta" still detected at 87.59 BPM because its raw ratio is 0.870 — even harsher than Come Back's 0.922 — and 0.89 × raw(low) still beat 1.00 × raw(high) by 2.3 %.
3. **Plateau [95, 175] (current).** Drops `weight(87.59)` to 0.83, below the 0.87 margin required by Total Science. The 174 BPM candidate now wins by 4.7 %, comfortably above the `SCORE_TIE_REL_TOL` floor. All eight clean DnB tracks in the corpus plus every rap doubling case now resolve correctly.

Both regressions are covered by inline assertions in the `tempo_prior_lower_ramp_flips_dnb_halftime` unit test: `prior(87.59) < 0.87` (Total Science gate) and `prior(86) < 0.85` (Bedhead gate). Future plateau changes must satisfy both.

**Two-pass selection** in `estimate_tempo`. The previous picker mixed the prior into a single comparison, which let off-peak lags whose harmonic-mean partially benefited from a single harmonic-window-aligned peak win the comparison (the synthetic spike-train test at lag 75, raw_score ≈ 50 % of max, qualified). The new flow is:

1. **Pass 1 — find the raw maximum.** Score every candidate by harmonic-mean local energy (unchanged from M8.1). Track `max_raw_score` separately so pass 2 can gate off-peak lags.
2. **Pass 2 — prior-weighted selection among qualifying candidates.** Only lags whose raw harmonic-mean clears `0.70 × max_raw_score` are eligible. Among those, the picker maximises `raw_score × tempo_prior_weight(bpm_at_lag)`. The existing "prefer smaller lag on tie" rule (`SCORE_TIE_REL_TOL = 0.01`) is preserved verbatim for the residual case where two octaves are both at full plateau weight (e.g. clean synthetic DnB at 174 vs 87).

A defensive fallback returns the pass-1 raw winner if pass 2 picks nothing — unreachable in normal operation, but it means the prior cannot introduce a new "no detection" failure mode on healthy inputs.

**Confidence reporting unchanged.** `BpmEstimate::confidence` is still computed from `raw_peak / acf_zero` at the picked lag, which means the streaming confidence-tracker hysteresis in `dub-bpm::tracker` (the LOCKING / LOCKED state machine, M8) continues to gate on a number that reflects autocorrelation strength rather than perceptual preference. Only the lag *selection* uses the prior; the lag's reported confidence is still the raw signal.

### Measured impact

`diagnose_bpm` against the user-reported corpus:

| Track | Real BPM | Before M11c.3a | After M11c.3a |
|---|---|---|---|
| Cause4Concern "Bedhead" (DnB) | 172 | 86.03 | **172.11** |
| Chase & Status "Baddadan" (DnB) | 175 | 87.54 | **174.39** |
| Chase & Status & Stormzy "Backbone" (DnB) | 176 | 87.84 | **176.95** |
| Chase & Status "Come Back" feat. Top Cat (DnB) | 174 | 87.52 | **174.34** |
| Excel "Apocalypse" (Nick The Lot remix) (DnB) | 173 | 87.52 | **173.46** |
| Total Science / S.P.Y. "Gangsta" (DnB) | 175 | 87.52 | **174.54** |
| Westside Connection "Gangsta Nation" | 95 | 191.11 | **95.43** |
| Westside Connection "Connected For Life" | 95 | (2×) | **95.15** |

**Mixed-tempo edge case (Benny Page).** Benny Page "Crying Out" (Serial Killaz remix) is a DnB track with a literal half-tempo reggae break in the middle. The autocorrelation accumulates real energy at both the DnB rate (~175 BPM) and the reggae rate (~87 BPM), and the raw-score ratio against the upper octave drops to 0.757 — too low for any prior that doesn't also break legitimate 90 BPM hip-hop. The algorithm reports 87.92 BPM (the reggae octave) and the user uses tap-to-grid (M11c.3b) to double the BPM if mixing the track at DnB tempo. This is a structural limitation of one-pass whole-track analysis; a future per-section beat-grid (PRD M10.5p-grid) would let the engine carry two grid segments and switch authority across the reggae break.

The synthetic `genre_octave.rs` suite also reflects the move:

* `hip_hop_90_bpm_locks_at_90_not_180` — now passes (used to detect 180 because the 8th-note hi-hat ostinato lands at exactly 180 BPM, inside the upper ramp where weight drops to 0.94 vs 90's plateau weight of 1.00).
* `hip_hop_100_bpm_locks_at_100_not_200` — still passes (200 BPM weight drops to 0.30).
* `drum_n_bass_174_bpm_locks_at_174_not_87` — still passes (174 in plateau, smaller-lag tiebreak resolves the tie correctly).

### Known limitations

**Reggae one-drop and slow soul at < 80 BPM detect at 2x.** Tracks like Marvin Gaye "Ain't No Mountain High Enough" (perceived 65 BPM) and the synthetic reggae one-drop fixture (65 BPM with hi-hat skanks at the 130-BPM rate) still resolve to 130 BPM because the prior weights 65 at 0.34 and 130 at 1.00. The autocorrelation alone cannot distinguish "65 BPM soul" from "130 BPM house" — both have identical periodic structure at lag 32 and lag 64. The 130 BPM detection is the *worse* answer for a DJ ("if a DJ plays a real 130 BPM house tune and mixes this in it kills the vibe", per the user), but the prior cannot pull 65 above 130 without also pulling DnB's 86 above 172 — which would re-introduce the half-time bug. The M11c.3 plan addresses this via tap-to-grid (M11c.3b, shipped): one keystroke to halve / double the detected BPM and re-anchor the grid with `Shift+G` / `Option+G` / `G`. A future M11c.5+ snare-emphasis ODF could in principle distinguish "65 BPM snare-on-2-and-4 of a 4/4 with 8th-note hi-hat" from "130 BPM kick on every beat", but the cost / payoff trade-off favours shipping tap-to-grid first and re-evaluating once the corpus shows what still falls through.

The synthetic `reggae_one_drop_65_bpm_locks_at_65` test was renamed to `reggae_one_drop_65_bpm_locks_at_an_octave_family_of_65` and now accepts either 65 ± 4 (preferred) or 130 ± 4 (M11c.3a known limitation), so it still rejects "garbage / non-detection" regressions while documenting the limitation in test code.

### Architecture notes

1. **Why a piecewise-linear prior, not a Gaussian.** Klapuri 2003 / 2006 use a log-normal tempo prior centered at ~120 BPM with σ_log ≈ 1, which gives a smooth-bell shape. We tried it; the bell is too gentle to flip the DnB Bedhead case (raw-score ratio 0.92 needs a prior ratio under 0.92, which a σ_log = 1 Gaussian centered at 120 doesn't deliver at 86 vs 172 because both points are roughly equidistant from the center). A steeper bell pushes hip-hop 95 BPM out of the preferred zone, which we don't want. Piecewise-linear with hand-tuned plateau boundaries gave the cleanest separation: 86 inside the ramp, 88+ inside the plateau, 178 inside the plateau, 180 inside the ramp.
2. **Why `OCTAVE_CANDIDATE_THRESHOLD = 0.70`.** The synthetic spike-train test exercises an off-peak lag (75 lag, BPM 80, raw_score ≈ 50 % of max) whose harmonic-mean catches a single peak-aligned harmonic without being a real beat-period candidate. A threshold of 0.50 lets that lag qualify and the prior-weighted comparison picks it because BPM 80 has higher prior weight than BPM 60 (the true period). 0.70 cleanly excludes it. Every real-music octave-conflict candidate sits at raw_score ≥ 0.83 of max, comfortably above the gate.
3. **Why the prior doesn't touch the confidence.** The streaming tracker (`dub-bpm::tracker`) gates LOCKING → LOCKED transitions on confidence reaching a target (`TARGET_CONFIDENCE = 0.30` in `TrackerConfig`). Folding the prior into confidence would change those thresholds' meaning — a 95 BPM rap track with raw confidence 0.37 would suddenly report `0.37 × 1.00 = 0.37` (unchanged for rap) but a 190 BPM detection would drop to `0.38 × 0.62 = 0.24` (below threshold) and the tracker would never lock. That looks like correctness from the rap side but breaks legitimate fast-tempo material that genuinely lives at 185–195 BPM (e.g. happy hardcore, gabber, footwork). Keeping confidence unweighted preserves the tracker's per-genre robustness.
4. **The unit-test gate on the prior shape.** `crates/dub-bpm/src/tempo.rs` now has four prior-specific unit tests (`tempo_prior_plateau_covers_mixable_genres`, `tempo_prior_lower_ramp_flips_dnb_halftime`, `tempo_prior_upper_ramp_flips_rap_doubletime`, `tempo_prior_floor_at_extremes`). The boundary asserts are stated as inequalities tied to the M11c.3 corpus raw-score ratios so future curve adjustments must continue to satisfy them — i.e. dropping the ramp start below 90 will trip `tempo_prior_lower_ramp_flips_dnb_halftime`'s `< 0.92` assert at 86 BPM.

### Files touched

* `crates/dub-bpm/src/tempo.rs` — adds `tempo_prior_weight` with full calibration rationale; rewrites `estimate_tempo`'s picker as a two-pass (raw + prior-weighted) selection with an `OCTAVE_CANDIDATE_THRESHOLD` gate; four new prior-shape unit tests.
* `crates/dub-bpm/tests/genre_octave.rs` — renames the reggae one-drop test to acknowledge the half-time octave-family acceptance; updates the assert to allow either 65 ± 4 or 130 ± 4.
* `docs/PRD.md` — §5.2.3 mentions the M11c.3a perceptual prior in the algorithm summary; §12 milestones row updated.
* `docs/SHIPPED.md` — this section.

### Commit boundary

This entry corresponds to the M11c.3a commit set: `crates/dub-bpm/src/tempo.rs`, `crates/dub-bpm/tests/genre_octave.rs`, `docs/PRD.md`, `docs/SHIPPED.md`.

### Deferred

* **M11c.3b tap-to-grid.** *(Shipped — see [M11c.3b](#m11c3b).)*
* **Snare-emphasis ODF (M11c.5+ candidate).** A high-frequency band-emphasis ODF (3–8 kHz) would weight snare hits more than kicks and shift the K-S autocorrelation peak above the K-K skip-1 peak in DnB, in principle removing the need for the lower-ramp penalty. The trade-off: it would also amplify hi-hat ostinato in hip-hop, re-introducing the 2× failure mode the M8.1 log-band ODF was specifically designed to suppress. Real-world evaluation against the M11c.3 corpus is needed before committing.
* **Real-music corpus expansion.** Reggae (24/24), rap (19/19), house (16/16 + 1 known fail), dubstep (13/13 + 1 known fail), and DnB (12/12 + 8 known fails) corpora are seeded; residual Default-profile DnB failures are documented as tap-to-grid cases.

---

<a id="m11c3c"></a>
## M11c.3c — Reggae skank double-time rejection

**Status:** shipped &nbsp;·&nbsp; **Parent:** M11c.3 BPM octave fix

M11c.3a's perceptual prior fixed hip-hop at 2× and DnB at 1/2× on real catalogs, but roots reggae still locked onto the hi-hat skank (~130 BPM) instead of the kick one-drop (~65 BPM). The autocorrelation sees both peaks at full plateau weight while the lower ramp crushes 65 BPM.

Pass 2 now hard-rejects skank-rate candidates in the 118–160 BPM band when a credible one-drop sibling exists at the 2:1 ratio in 60–80 BPM with raw score ≥ 85 % of the skank peak. A gap-based variant (71–75 BPM sibling only, ≥ 5 % raw gap) catches Murderer-style cases without flipping 128 BPM click trains whose half-time harmonic lands near 70 BPM. Dancehall tracks with phantom ~180 BPM peaks get a separate near-tie rejection (175–185 vs 85–95) that spares DnB when the low octave clearly wins on raw (Who Am I).

**Thru / streaming:** these rules apply on [`OctaveProfile::Default`] — no genre tag on live wax.

**Validation:** author's reggae folder (24 tracks) on Default profile moved most one-drop material into 65–104 BPM; five tracks still needed genre context (M11c.3d). Rap folder (19 tracks, Default) unchanged vs pre-M11c.3d baseline — no regression on the agreed next gate.

**Files:** `crates/dub-bpm/src/tempo.rs` (`skank_doubletime_rejected`, `dancehall_doubletime_rejected`, `linked_halftime_penalty` tuning).

---

<a id="m11c3d"></a>
## M11c.3d — Genre-aware octave profile (library analysis)

**Status:** shipped &nbsp;·&nbsp; **Parent:** M11c.3 BPM octave fix

Offline library analysis can read ID3 genre tags and pass an [`OctaveProfile`] into pass 2. Thru-mode streaming keeps [`OctaveProfile::Default`] because live wax has no tag until fingerprint match (v1.1).

### Profiles

| Profile | Source tags (substring) | Behaviour |
| --- | --- | --- |
| `RootsReggae` | reggae, rocksteady, roots, lovers, ska | Rejects 135–180 BPM when a 60–100 BPM sibling carries ≥ 75 % of the upper raw score (Here I Come, Jump Nyabinghi). Keeps the default perceptual prior — lifting the whole 60–100 band would flip true ~95 BPM tracks to ~72 half-time. |
| `Dub` | dub, dubstep | Rejects upper octave on near-tie (≤ 2 % raw gap) or when lower sibling ≥ 80 % raw. Lifts 65–80 BPM to full prior weight so a rejected ~140 peak does not lose to ~93 (Blind Prophet). |
| `Dancehall` | dancehall, ragga | No profile rejection — preserves full-tempo detections like All Night at 133 BPM. |
| `Default` | everything else, untagged | M11c.3a prior + M11c.3c skank/dancehall rules only. |

### Wiring

* `crates/dub-bpm/src/octave_profile.rs` — profile enum + genre/label mappers + `profile_doubletime_rejected`.
* `estimate_tempo(..., profile)` — profile-aware pass 2; streaming/offline Default unchanged.
* `analyze_beat_grid_with_profile` / `analyze_bpm_with_profile` — public API for library + corpus tests.
* `crates/dub-library/src/analysis.rs` — `track_id3_genre()` → `octave_profile_from_genre()` on lazy analyze.

### Corpus

`crates/dub-bpm/tests/fixtures/reggae_corpus.tsv` (24 tracks, absolute paths) + env-gated `real_music_corpus` test:

```sh
DUB_BPM_REAL_CORPUS=/path/to/reggae_corpus.tsv cargo nextest run -p dub-bpm real_music_corpus
```

**Result:** 24/24 pass with profile column. Known exceptions preserved: All Night at 133 (`dancehall`), Here I Come at 85 (`roots`, half-time controversy).

### Deferred (unchanged from M11c.3a)

* **FourOnFloor profile** (candidate) for house/techno library analysis — opposite failure mode to reggae. *(Shipped M11c.3f.)*
* **Rap 2× outliers** on Default (5/19 in author corpus) — next validation gate before house. *(Shipped M11c.3e.)*
* **Tap-to-grid** for genuinely ambiguous tracks — *(Shipped M11c.3b.)*

**Files:** `octave_profile.rs`, `tempo.rs`, `offline.rs`, `beats.rs`, `lib.rs`, `dub-library/src/analysis.rs`, `tests/real_music_corpus.rs`, `tests/fixtures/reggae_corpus.tsv`.

---

<a id="m11c3e"></a>
## M11c.3e — Hip-hop double-time rejection (Default profile)

**Status:** shipped &nbsp;·&nbsp; **Parent:** M11c.3 BPM octave fix

Four tracks in the author's rap corpus (`Cappadonna`, `Charizma`, `Charles Bradley`, `Dangermouse — Daydream`) reported at 2× the audible tempo while fourteen others were already correct on Default. Chestra — Honey Dripper at 112 BPM was confirmed correct by ear and must not flip.

Pass 2 now hard-rejects upper-octave candidates in 160–185 BPM when a 2:1 sibling in 80–95 BPM carries ≥ 96 % of the upper peak's raw score and the raw gap sits in [0.5 %, 6 %]. The lower bound spares the rolling-DnB synthetic fixture (perfect 0.3 % tie); real rap errors start at ≈ 0.6 %. DnB at 172+ is spared when the half-time gap exceeds 6 % (Gangsta ≈ 13 %) or when two or more peers cluster in the 168–182 BPM core band.

When a triplet candidate was rejected and the linked half-time penalty would fire, the penalty is skipped if the high-octave sibling would itself be rejected by this pass (Charizma: 178 rejected → 89 beats 120).

**Corpus:** `crates/dub-bpm/tests/fixtures/rap_corpus.tsv` (19 tracks) — **19/19 pass** on Default profile.

**Files:** `crates/dub-bpm/src/tempo.rs` (`hiphop_doubletime_rejected`, `linked_halftime_penalty` guard), `tests/fixtures/rap_corpus.tsv`.

---

<a id="m11c3f"></a>
## M11c.3f — FourOnFloor profile (house / garage library)

**Status:** shipped &nbsp;·&nbsp; **Parent:** M11c.3 BPM octave fix

House and UK garage in the author's corpus locked onto ~85–93 BPM (half-bar / shuffle subdivision) when the mixable 4/4 kick grid sits at ~120–140. The reggae skank pass (M11c.3c) made this worse by rejecting the true ~129 BPM peak as a false skank rate.

### `OctaveProfile::FourOnFloor`

* ID3 tags: `house`, `garage`, `techno`, `trance`, `electro`, `club` → `FourOnFloor` (library analysis only; Thru stays `Default`).
* **Skip skank pass** so ~129 BPM house kicks are not discarded.
* **Half-bar rejection:** discard 80–100 BPM when a 115–145 BPM sibling exists at the 3:2 ratio with ≥ 85 % raw score (Molly shape: 86 vs 129).
* **Shuffle-high rejection:** discard 152–170 BPM when a 118–130 BPM sibling exists at the 4:3 ratio (Jaden shape: 164 vs 123).

### Dub mid-band (same commit)

Extended `OctaveProfile::Dub` with mid-band rejection: discard 85–100 BPM when a 65–74 BPM sibling exists at the 4:3 ratio (~93 vs ~70 dubstep corpus). Fixes 13/14 dubstep tracks on the `Dub` profile; Burial — Endorphin remains a known sparse-material fail.

### Corpora

| Corpus | Profile | Result |
|--------|---------|--------|
| `house_corpus.tsv` (17) | `four_on_floor` | **16/16** gated (+ 1 `known fail`: Good Neighbours slow house) |
| `dubstep_corpus.tsv` (14) | `dub` | **13/13** gated (+ 1 `known fail`: Burial Endorphin) |

`diagnose_bpm --folder PATH four_on_floor` / `dub` added for profile-aware folder sweeps.

**Files:** `octave_profile.rs`, `tempo.rs`, `examples/diagnose_bpm.rs`, `tests/fixtures/house_corpus.tsv`, `tests/fixtures/dubstep_corpus.tsv`.

---

<a id="m11c3b"></a>
## M11c.3b — Tap-to-grid (manual BPM override)

**Status:** shipped &nbsp;·&nbsp; **Parent:** M11c.3 BPM octave fix (closes M11c.3)

Keyboard-only manual beat-grid correction per PRD §8.3.1 / §8.3 milestone scope. The v1 surface is intentionally minimal: re-anchor at the playhead and halve/double the tempo. Full transient snap, ±20 BPM refit, and multi-tap BPM inference remain PRD spec for a future beatgrid-editor pass (M24); this milestone ships the escape hatch the DJ needs when auto-detect lands on the wrong octave.

### Keystrokes (master deck)

| Key | Action |
|-----|--------|
| `G` | Re-anchor grid at playhead; keep current BPM |
| `Shift+G` | Halve BPM; re-anchor at playhead |
| `Option+G` | Double BPM; re-anchor at playhead |

Target deck is `masterDeck ?? stickyMaster` (same deck the DJ is actively mixing).

### Engine + library path

* `DubEngine::install_beat_grid(deck_idx, bpm, anchor_secs)` synthesises per-beat timestamps in place on the loaded File deck and bumps `beat_grid_generation` so the Metal renderer refetches without resetting the waveform peak ring.
* `DubLibrary::upsert_user_tap_beatgrid(track_id, anchor_secs, bpm)` deactivates other grid rows and writes `source = 'user_tap'` as the sole active grid when the deck holds a library track.
* Swift `KeyEventMonitorHost` intercepts `G` / `Shift+G` / `Option+G` before first-responder routing (No Mouse DJ).

### DnB corpus (M11c.3 close-out)

`crates/dub-bpm/tests/fixtures/dnb_corpus.tsv` (20 tracks, Default profile):

```sh
DUB_BPM_REAL_CORPUS=/path/to/dnb_corpus.tsv cargo test -p dub-bpm real_music_corpus
```

**Result:** 12/12 gated pass + 8 `known fail` (half-time ~87, triplet ~116, ragga-dnb ~90 on Default). Tap-to-grid is the intended override for those rows until/unless a DnB-specific profile lands.

**Files:** `crates/dub-ffi/src/lib.rs` (`install_beat_grid`, `beat_grid_generation`), `crates/dub-library/src/analysis.rs` (`upsert_user_tap_beatgrid`), `apple/Dub/MainView.swift`, `apple/Dub/Waveform/WaveformRenderer.swift`, `tests/fixtures/dnb_corpus.tsv`.

---

<a id="m11d6"></a>
## M11d.6 — Full-screen on launch + windowed snap-back

**Status:** shipped &nbsp;·&nbsp; **Parent:** M10.3 Performance shell

PRD §2.1 "No Mouse DJ Ever" is now the default boot state. `DubAppDelegate.applicationDidFinishLaunching` calls `controller.window?.toggleFullScreen(nil)` after `NSApp.activate(ignoringOtherApps: true)` so a DJ who launches Dub onstage lands directly in the performance surface without reaching for the green traffic-light button. The window's `collectionBehavior` gains `.fullScreenPrimary` so the standard macOS `Cmd+Ctrl+F` shortcut and the new `View → Enter Full Screen` menu item both route through `NSWindow.toggleFullScreen(_:)`. Exiting full-screen calls `windowDidExitFullScreen` on the `MainWindowController` (now `NSWindowDelegate`), which forces the window back to the documented 1440x900 reference rectangle (`MainWindowController.defaultContentSize`) and re-centers it — Dub never carries a drag-resized windowed size across a full-screen toggle or across launches. `.resizable` stays in the style mask because removing it would also strip the green button and disable `toggleFullScreen:` entirely; the snap-back hook makes that resize affordance effectively cosmetic.

### Fix-at-the-cause: clear `NSHostingController.sizingOptions`

First-cut testing surfaced the actual headline bug: full-screen "worked" (the window expanded to the display) but the SwiftUI performance surface stayed at 1440x900 with the window background painting a black frame around it. Cause: on macOS 13+, `NSHostingController.sizingOptions` defaults to `[.preferredContentSize, .intrinsicContentSize]`, which exposes SwiftUI's computed intrinsic size to AppKit Auto Layout as a hard intrinsic content size on the hosting view. Combined with `MainWindowController` setting `hostingController.preferredContentSize = NSSize(1440, 900)` (a workaround introduced when the embedded `MTKView` had no intrinsic size and the toolbar collapsed the window to ~514x87), the hosting view's intrinsic content size was pinned at 1440x900 and won the constraint fight against AppKit's auto-installed edge-pinning constraints between `window.contentView` and `contentViewController.view`. The black frame was the SwiftUI hierarchy refusing to grow.

Fix: set `hostingController.sizingOptions = []` and drop the `preferredContentSize` assignment. With no SwiftUI-driven intrinsic size, the standard edge-pinning constraints take over and the SwiftUI hierarchy fills whatever frame AppKit hands it — windowed at 1440x900 (still set by the window's `contentRect` + `setContentSize`, which are window-level concerns independent of the hosting controller), full-screen at every display the user can reach (Retina laptop, 5K Studio Display, 6K Pro Display XDR, ultrawide). The old `preferredContentSize` workaround for the "collapses to toolbar size" symptom is obsolete because `PerformanceView` and the M11d.5 deck-header layout both carry their own `frame(maxWidth: .infinity, maxHeight: .infinity)` and `frame(minWidth: 960, minHeight: 600)` floors, so SwiftUI no longer collapses without external prodding.

Files: `apple/Dub/DubAppDelegate.swift`, `apple/Dub/MainWindowController.swift`.

---

<a id="m11d7"></a>
## M11d.7 — Beatgrid precision, auto downbeat, tap-to-grid, drift lock

**Status:** shipped &nbsp;·&nbsp; **Parent:** M11d beatgrid calibration

Seven-stage auto pipeline in `dub-bpm`: autocorr → coarse sweep → parabolic vertex → zoom sweep → LSQ refit → kick-band downbeat → kick-only intro/outro tie-breaker for four-on-floor ambiguity. `GridQuality` (RMS / p95 / kept fraction / drift slope) drives auto-lock on analysis.

**Tap-to-grid:** deck-header BPM column replaces the press-1 button. 1–2 taps within 2 s relatch downbeat at the first tap; 3+ taps recompute tempo via `analyze_beat_grid_from_taps` (tap median seeds BPM range, ODF refines). Persists `user_tap` rows; does not implicitly lock.

**Grid lock:** schema v4 adds `tracks.grid_locked` + `tracks.grid_drift_quality`. Locked tracks skip auto re-analysis on reload. Library row context menu Lock/Unlock grid; lock.fill / ⚠ on BPM when drift ≥ 3 ms/min and unlocked. `FFI_VERSION` 19.

---

*End of shipped milestone history. Forward-looking planning lives in [`docs/PRD.md` §12](PRD.md#12-milestones).*
