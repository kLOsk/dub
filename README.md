<div align="center">

# Dub

**A timecode-vinyl DJ application for scratch DJs and vinyl enthusiasts.**

*Mac-first. Rust-cored. GPLv3.*

[![CI](https://github.com/kLOsk/dub/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/kLOsk/dub/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange?logo=rust)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple)](#status)
[![Status](https://img.shields.io/badge/status-pre--alpha-red.svg)](#milestone-progress)

<img src="apple/Dub/Assets.xcassets/AboutSplash.imageset/AboutSplash.png" alt="Dub splash artwork" width="480">

</div>

> Dub is the spiritual successor to Serato Scratch Live for the urban music scene
> (hip hop, reggae, dnb, dubstep, scratch). Two decks, external mixer, real
> records **through** the software, smart utility FX, fast sample throws.

This is **pre-alpha software**. There is no release. There is `main`.

---

## What's interesting about this project

- **Rust on the audio thread.** The engine runs in a CoreAudio HAL IOProc with
  zero allocations, zero locks, and zero syscalls. Verified in CI by the
  [`rt-audit`](tools/rt-audit) binary, which renders 100k blocks under
  [`assert_no_alloc`](https://crates.io/crates/assert_no_alloc) before any merge.
- **Clean-room timecode.** [`dub-timecode`](crates/dub-timecode) decodes Serato
  CV02 in relative mode — analytic-signal demod, signed rate, confidence
  estimate, all alloc-free. ([Architecture notes.](docs/ARCHITECTURE.md))
- **Lift policy hardened on real hardware.** The `LiftPolicy` state machine
  combines a three-layer defense — RMS amplitude gate, two-edge confidence
  hysteresis, sticky-block window — driven by SL3 + Serato CV02 testing.
  Each pathology has a dedicated regression test.
- **TUI inspector.** [`dub scope`](crates/dub-cli/src/scope.rs) is a ratatui
  Lissajous + gauges + live thresholds, sharing the same `LiftPolicy` as
  playback so calibration transfers 1:1.
- **No mouse on the audio path, ever.** UI = external mixer + controllers + the
  user's hands on real records. The mouse is for browsing tracks, period.

## Quickstart

Mac with Rust stable, an audio interface, and (optionally) a Serato CV02
control vinyl.

```bash
# Build everything in release mode.
make ci

# Smoke-test the engine.
./target/release/dub smoke

# Play a file through CoreAudio at the device's native sample rate.
./target/release/dub play --realtime path/to/track.mp3

# Live timecode → deck (M5.3 / M6). One deck, Serato CV02 on SL3 ch 3+4.
# Auto-runs a fresh ~3.5 s calibration on startup (M5.4.3 / M5.4.6);
# audio output is live immediately, deck attaches the moment the carrier locks.
./target/release/dub timecode-deck path/to/track.mp3 --input-channels 3,4

# Two-deck timecode (M5.6 + M5.5.2). Single SL3 demuxed in the IOProc; each
# deck gets its own input ring, its own calibrator, its own output channel pair.
# The device-profile flag picks up the SL3's deck-A → ch 3+4, deck-B → ch 5+6
# routing automatically (also auto-detected if the device name matches).
./target/release/dub timecode-deck a.mp3 b.mp3 \
    --input-channels 3,4 --deck-b-input-channels 5,6 \
    --device-profile "SL 3"

# Traktor MK1 or MK2 instead of Serato (M6). Bare `traktor` is rejected as
# ambiguous — pick the generation, getting it wrong = silent 25 % speed error.
./target/release/dub timecode-deck a.mp3 --input-channels 3,4 --format traktor-mk2

# Thru mode — real (non-timecode) record routed through the engine (M7).
# Constant ~2.7 ms one-way latency, software-always-on so M8 BPM + M9 waveform
# + M15+ FX can hook in. One mode, no flags — there is no hardware-bypass mode.
./target/release/dub thru --input-channels 3,4 --device-profile "SL 3"

# TUI inspector for tuning your rig against the live timecode signal
# (M5.4.1). Same LiftPolicy as `timecode-deck`, so what you see here is what
# you'd hear during playback.
./target/release/dub scope --input-channels 3,4

# Manual one-shot per-rig calibration (M5.4.2 / M5.4.3 / M5.4.4).
# Default is single-phase carrier-only (~3.5 s). JSON is a diagnostic
# artifact only — `timecode-deck` doesn't read it back; every startup
# recalibrates fresh against whatever rig is in front of you (M5.4.6).
# Stored at ~/.dub/calibration/<device>_deck_<idx>_<format>.json.
./target/release/dub calibrate --input-channels 3,4 --deck 0

# Inspect output offline — `dub analyze` runs the M3.5 click detector
# over any WAV (peak / RMS / DC / clipping / max per-sample delta).
./target/release/dub analyze path/to/captured.wav

# Bootstrap the macOS app (M0.5 / M10). One-time: `brew install xcodegen`.
# Generates DubCore.xcframework + Swift UniFFI bindings + Dub.xcodeproj.
./scripts/bootstrap.sh
make app                              # or: open apple/Dub.xcodeproj and ⌘R
open apple/build/Build/Products/Debug/Dub.app
#                            Two-deck performance surface with library browser,
#                            Metal waveforms, launch splash, app icon, and
#                            About sheet (DUB wordmark or Dub → About Dub).
```

`dub scope` keys: `q`/`Esc` quit, `c` clear lissajous, `↑/↓` engage threshold,
`PgUp/PgDn` disengage, `←/→` amplitude, hold `Shift` for 10× steps.

## Milestone progress

Roadmap and forward-looking milestones live in [`docs/PRD.md`](docs/PRD.md);
detailed design history for everything shipped lives in [`docs/SHIPPED.md`](docs/SHIPPED.md).

| Milestone | Status | Headline |
|---|---|---|
| **M0–M2.1** | ✅ shipped | Scaffold, CI, RT-safety harness, first sound, lock-free transport command channel, soak / fuzz / nightly harness wired. |
| **M3 + M3.5** | ✅ shipped | Format coverage (mp3 / flac / m4a / aiff / aac / alac) + hot loading via `Arc<Track>` trash channel; declick envelope + tail-fade + `dub analyze` offline click detector. |
| **M4** | ✅ shipped | Two decks driveable end-to-end; debug internal mixer; `Command::SetMasterGain` for live master via the SPSC channel. |
| **M5.1 / M5.2 / M5.3** | ✅ shipped | Clean-room Serato CV02 decoder (relative mode, analytic-signal demod) → `AudioInput` HAL plumbing → live timecode-to-deck with 3-layer `LiftPolicy` (amplitude gate + confidence hysteresis + sticky window). **The point Dub becomes a DJ app.** |
| **M5.4 / M5.4.3 / M5.4.4** | ✅ shipped | `dub scope` TUI inspector (M5.4.1) + `dub calibrate` per-rig threshold derivation (M5.4.2) → single-phase carrier-only at industry-parity speed (M5.4.3, ≈ 3.5 s) → per-deck calibration (M5.4.4). |
| **M5.4.5** | ✅ shipped | Mid-stream `EngineHandle::attach_timecode_input` via second trash channel; parallel calibrator workers each owning their own ringbuf consumer; deck B's worker waits indefinitely for the takeover window. **Closes the DJ-takeover product gate.** |
| **M5.4.6** | ✅ shipped | Gutted the JSON-load + fingerprint-probe machinery. Touring DJs always recalibrate on startup; the file is a diagnostic artifact only. |
| **M5.5.1 / M5.5.2** | ✅ shipped | `Engine::render_routed` unifies internal- and external-mixer routing. CoreAudio 4-channel output with SL3 ✅-verified and Audio 6 ⚠️-unverified device profiles. |
| **M5.6** | ✅ shipped | Two-deck timecode through one CoreAudio input AU, IOProc-demuxed into per-deck SPSC rings. |
| **M6** | ✅ shipped | Traktor MK1 (2 kHz) + MK2 (2.5 kHz) through the same format-agnostic decoder. Bare `traktor` alias rejected as ambiguous. |
| **M7** | ✅ shipped | **Thru Mode** — per-deck `ThruSource` (single always-on software passthrough) integrated into `Engine::render_routed`; command-channel attach with third trash channel for `Box<ThruSource>`; new `dub thru` CLI sharing M5.5.2's routing. Constant ~2.7 ms one-way latency, independent of future FX state (Option A in-chain bypass). |
| **M7.5** | ✅ shipped | **BPM engine + offline analysis.** New `dub-bpm` crate (pure-Rust spectral-flux + harmonic-summed autocorrelation, fractional-step search). `BpmEstimator` streaming core + `analyze_bpm` offline driver + `Track::bpm` field on `dub-io::Track`. Synthetic clicks at 60–174 BPM detected within ±1 BPM. Aubio was the original plan; pivoted to pure-Rust after recon — see [`docs/SHIPPED.md#m75`](docs/SHIPPED.md#m75). |
| **M8** | ✅ shipped | **Auto-BPM on Thru — streaming driver.** `BpmTracker` (estimator + hysteresis state machine + throttled search) + `BpmStream` (per-deck off-RT analysis thread + lifecycle). Audio-thread mono-downmix tee on `ThruSource` (alloc-free). `EngineHandle::attach_thru_source_with_bpm_tracking` bundles tee + thread spawn. `dub thru` prints `searching → tentative → locked` transitions to stderr by default (`--no-bpm-track` to disable). See [`docs/SHIPPED.md#m8`](docs/SHIPPED.md#m8). |
| **M8.1** | ✅ shipped | **BPM octave fix — log-band ODF + windowed-energy picker.** Replaced single-band spectral flux with 8-band log-spaced flux, harmonic-sum with harmonic-mean over 4 multiples, parabolic-vertex peak height with windowed local-energy (5-bin sum, invariant to bin-split asymmetry), and added centroid sub-bin refinement. Fixes the M8 hip-hop 2× regression (100 BPM detected as 200 BPM). Locks reggae 65 / hip-hop 90/100 / rolling dnb 174 at the correct octave out of the box. New `BpmRange` API + `dub thru --bpm-range MIN,MAX` escape hatch for irreducibly-ambiguous genres (dubstep 140 / 70). See [`docs/SHIPPED.md#m81`](docs/SHIPPED.md#m81). |
| **M9** | ✅ shipped | **Live waveform capture (Thru).** New `dub-peaks` crate (off-RT decimator thread, shape mirrors M8's `dub-bpm`). `Decimator` (online min/max/rms aggregator), `PeakBuffer` (`AtomicUsize` len + `RwLock<Vec<PeakChunk>>`, with `extend_chunks` renderer fast path), `PeakStream` (joinable analysis thread). `ThruSource` refactored to share one mono-downmix between the BPM tee and a new peaks tap (one extra `push_slice`, verified alloc-free). New `EngineHandle::attach_thru_source_with_peaks_tracking` + `attach_thru_source_with_telemetry` (BPM + peaks combined). `dub thru` defaults to peaks-tracking on, periodic stats line shows captured chunk counts, `--dump-peaks PATH` writes a CSV envelope dump on shutdown for debugging before M10's UI lands. `PeakChunk` is `#[repr(C)]` 12-byte wire format — the M10 consumer contract. See [`docs/SHIPPED.md#m9`](docs/SHIPPED.md#m9). |
| **M0.5** | ✅ shipped | **Apple shell + smoke screen.** XcodeGen-generated `apple/Dub.xcodeproj` (AppKit `@main` + SwiftUI `SmokeScreenView` inside an `NSHostingController`). `crates/dub-ffi` upgraded to UniFFI 0.28 proc-macros + `staticlib`+`cdylib`+`uniffi-bindgen` binary. `scripts/build-xcframework.sh` builds universal (aarch64 + x86_64) `DubCore.xcframework` + Swift bindings via UniFFI's library mode. `scripts/bootstrap.sh` regenerates everything from a clean checkout. `DubShared/` Swift Package wraps the xcframework; the app window shows `"Dub engine OK · v0.0.1"` pulled live from Rust. Local "Sign to Run Locally" only — distribution signing is a separate post-M10.2 milestone. See [`docs/SHIPPED.md#m05`](docs/SHIPPED.md#m05). |
| **M9.5 (a + b)** | ✅ shipped | **`dub-spectral` extraction + 8-band peak capture.** M9.5a moved the shared FFT + log-band + magnitude-compression pipeline out of `dub-bpm/onset.rs` into a new `dub-spectral` crate (`SpectralFrameStream`); `OnsetDetector` is a thin shell over it, byte-identical ODF values on every M8.1 fixture. M9.5b extended `dub-peaks` with `BandPeakChunk { rms_per_band: [f32; 8] }` (`#[repr(C)]` 32-byte) + `BandDecimator` running on the existing mono tap (zero new audio-thread cost); `PeakStreamConfig::bands_enabled` defaults on; `dub thru --dump-band-peaks PATH` for verification before M10.1's renderer. See [`docs/SHIPPED.md#m95`](docs/SHIPPED.md#m95). |
| **M10 (A + B)** | ✅ shipped | **First waveform on screen.** M10-A: `dub-ffi` `DubEngine` UniFFI interface (`list_input_devices` / `start_thru` / `stop_thru` / `peaks_extend` / `peaks_len` / `peaks_chunk_duration_secs` + the matching `band_peaks_*` trio for M10.1) with a `flat_error` `EngineError`. M10-B: Apple shell shows a live, scrolling broadband waveform — Metal `MTKView` driven by a `@MainActor` renderer that owns a 2¹⁷-chunk ring buffer + triple-buffered uniforms, instanced quads per `PeakChunk`. `MainView` hosts a device picker, channels field, Start/Stop button, and the M0.5 greeting demoted to a debug overlay. `apple/project.yml` now surfaces CoreAudio/AudioToolbox/AudioUnit/Metal/MetalKit frameworks. `./scripts/bootstrap.sh && xcodebuild build -scheme Dub` produces a runnable universal `Dub.app`. See [`docs/SHIPPED.md#m10a`](docs/SHIPPED.md#m10a) and [`docs/SHIPPED.md#m10b`](docs/SHIPPED.md#m10b). |
| **M10.1** | ✅ shipped | **Multi-colour fragment shader.** Vertex shader reads the matching `BandPeakChunk` per broadband instance from a parallel `MTLBuffer` ring; fragment shader mixes 8 perceptual bands → RGB (`R` = bass, `G` = mids, `B` = highs) with per-channel loudness compensation and broadband-RMS luminance. Silence drops to neutral grey (honest dropouts). `DubEngine::sample_rate()` accessor added so the renderer can derive `samples_per_chunk` exactly; `FFI_VERSION` bumps to 3. See [`docs/SHIPPED.md#m101`](docs/SHIPPED.md#m101). |
| **M10.2 (first wave)** | ✅ shipped | **Polish.** Deck B wired via new `DubEngine::startThruTwoDeck(device, channelsA, channelsB)`; 4-channel input AU demuxed in the IOProc; `VSplitView` shows one waveform per deck. Three palette presets (Serato-faithful / high-contrast / monochrome) live in the toolbar. Honest silence (thin neutral hairline) and clipping (solid red bar) detected per-chunk in the vertex shader. `FFI_VERSION = 4`. See [`docs/SHIPPED.md#m102`](docs/SHIPPED.md#m102). |
| **M10.2 (remainder)** | ◻ planned | Independently shippable bullets: onset glow, beat-aware saturation, constant-Q bass split (9-band `dub-spectral`), mip pyramids. Each is its own PR. |

PRD §2.2.0 describes the reliability staging — pragmatism before users, rigor
before stable.

## Repo layout

```
dub/                                 repo root (workspace)
├── Cargo.toml                       Rust workspace
├── crates/
│   ├── dub-engine/                  audio graph, transport, RT-safety, LiftPolicy, ThruSource
│   ├── dub-audio/                   CoreAudio HAL input + output (M1.4, M5.2, M5.5.2, M5.6)
│   ├── dub-dsp/                     resamplers, filters, FX (placeholder for v1 FX work)
│   ├── dub-stretch/                 Rubber Band FFI wrapper (M14, placeholder)
│   ├── dub-io/                      symphonia-based decoders (everything in RAM)
│   ├── dub-timecode/                Serato CV02 + Traktor MK1/MK2 decoder (clean-room)
│   ├── dub-thru/                    Thru-mode source-detection classifier (§5.1.1, placeholder)
│   ├── dub-bpm/                     M7.5 + M8 + M8.1 — BpmEstimator, BpmTracker, BpmStream, log-band ODF (pure-Rust, shipped)
│   ├── dub-spectral/                M9.5a — SpectralFrameStream (shared STFT + log-bands + magnitude compression), pure-Rust
│   ├── dub-peaks/                   M9 + M9.5b — Decimator + BandDecimator, PeakBuffer (broadband + bands), PeakStream — live waveform capture
│   ├── dub-fingerprint/             Pure-Rust Chromaprint (M11b library dedupe; v1.1 recognition parked)
│   ├── dub-library/                 SQLite catalog + import adapters (M11, shipped)
│   ├── dub-controller/              HID/MIDI abstractions (v1.x+, placeholder)
│   ├── dub-ffi/                     UniFFI Swift bindings (M0.5 greeting + M10-A DubEngine / EngineError / peaks_extend / band_peaks_extend)
│   └── dub-cli/                     `dub` binary (smoke / play / capture /
│                                                 timecode-deck / thru / scope /
│                                                 calibrate / analyze / …)
├── apple/                           AppKit + SwiftUI shell (M0.5 + M10-B shipped — XcodeGen-managed)
│   ├── project.yml                  XcodeGen manifest (links CoreAudio + Metal SDK frameworks)
│   ├── Dub/                         AppKit @main + SwiftUI MainView + Waveform/{Shaders.metal,WaveformRenderer,WaveformView}
│   │   └── Assets.xcassets/         App icon + About splash artwork
│   └── DubShared/                   Swift Package wrapping DubCore.xcframework
├── tools/
│   └── rt-audit/                    RT-thread allocation auditor
├── docs/                            PRD.md, SHIPPED.md, ARCHITECTURE.md, LIBRARY-FORMATS.md
├── scripts/                         build-xcframework.sh, bootstrap.sh (M0.5)
├── .cursor/                         Cursor rules + hooks for AI-assisted dev
└── AGENTS.md                        always-loaded project context for AI
```

## Engineering tenets

These are anchored in [`docs/PRD.md` §2.2](docs/PRD.md) and enforced both
socially and in CI:

1. **No allocations on the audio thread.** Static buffers, lock-free SPSC
   ringbufs, `assert_no_alloc` in tests + a dedicated rt-audit binary.
2. **TDD on anything that touches a real audience.** Pre-alpha is permitted to
   move fast (PRD §2.2.0), but stable releases must demonstrate full coverage
   on every audio-path code path.
3. **The engine matches the device, never the other way around.** v1 does no
   boundary resampling; both input and output devices are forced to the engine's
   sample rate, or startup fails loudly. (See M5.3 SR-alignment notes in
   ARCHITECTURE.md.)
4. **DJs stand in front of audiences.** Stuttering, dropouts, sample-rate
   converter artifacts, and policy chatter are no-go bugs, not "polish later"
   bugs. The `dub analyze` and `dub scope` tools exist so we can verify
   correctness without subjective listening sessions.

## Common commands

```bash
make test          # cargo nextest run + clippy
make smoke         # run the CLI smoke test
make rt-audit      # run the RT-safety harness
make ci            # everything CI runs (fmt-check + clippy + test + rt-audit)
make clippy        # cargo clippy --workspace --all-targets -- -D warnings
make fmt           # cargo fmt
make app           # build Dub.app (Debug) — also regenerates xcodeproj if project.yml changed
make run-app       # build + launch Dub.app from apple/build/
```

See the [Makefile](Makefile) for more targets.

## Hardware tested

These are validated end-to-end on real hardware as each milestone lands.

| Hardware | Used for | Status |
|---|---|---|
| Serato SL 3 | 4-in / 6-out interface; deck A on input ch 3+4, deck B on 5+6, output to mixer on ch 3+4 / 5+6 | ✅ M5.2 → M7 (input + two-deck demux + 4-ch routing + Thru) |
| Serato Control CV02 vinyl | Timecode source (relative mode, 1 kHz carrier) | ✅ M5.1 → M5.4.5 |
| Traktor Scratch MK1 vinyl | Timecode source (relative mode, 2 kHz carrier) | ✅ M5.4.3 + M6 |
| Traktor Scratch MK2 vinyl | Timecode source (relative mode, 2.5 kHz carrier) | ⚠️ M6 awaiting empirical channel-polarity validation on real MK2 pressing |
| Native Instruments Audio 6 | 4-in / 4-out interface alternative | ⚠️ device profile in `KNOWN_DEVICES` is unverified best-effort; warns at startup |
| Rane mixers (any) | External mixer | ✅ M5.3 + M5.5.2 (line-in compatible) |
| Phase DJ | HID controller | ◻ planned for v1.x |

## License

GPLv3 — see [`LICENSE`](LICENSE).

This means: if you distribute a binary based on this code, you must release the
source under GPLv3 too. We chose GPL deliberately so that engine improvements
made by anyone in the community come back to the community.

## Contributing

This is currently a single-developer project. Contributions are welcome but
expect reviews to be opinionated about reliability and the No-Mouse-DJ-Ever
philosophy. Read [`docs/PRD.md`](docs/PRD.md) first; for engineering background,
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

Bugs and feature requests: open an issue. Patches: open a PR against `main`.
CI must be green; new audio-path code requires `assert_no_alloc` coverage and
ideally a `rt-audit` extension.
