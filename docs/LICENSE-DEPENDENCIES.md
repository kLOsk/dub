# Third-party dependency licenses

This document enumerates every external library Dub links against, with its license, role, and attribution requirement. It is the **source of truth** for the "Acknowledgments" / "Open Source Licenses" panel a shipped binary must surface.

The list is kept in sync with the workspace `Cargo.toml` and per-crate `Cargo.toml` files by hand. If you add, remove, or upgrade an external dependency, update this document in the same commit.

Last verified: M11c (workspace dependency-graph snapshot 2026-05).

---

## Summary

| License family | Crates | Obligation |
|---|---|---|
| MIT / Apache-2.0 dual-licensed | 21 | Reproduce the license text + copyright notice in distributed binaries. |
| MIT | 6 | Reproduce the license text + copyright notice. |
| Apache-2.0 | 1 | Reproduce the license text + copyright notice; preserve any `NOTICE` file. |
| Unlicense / MIT | 1 | Reproduce the license text (either license satisfies). |
| MPL-2.0 | 2 | File-level copyleft only. The library binary may ship inside a proprietary application; if the library's source files themselves are modified, the modified files must remain MPL-2.0 and be made available on request. Dub does not modify any MPL-2.0 source files. |
| GPL-3.0-or-later | 0 (today) | None in the dep graph as of M11c. The workspace `license` field reserves GPL-3.0-or-later in anticipation of a future `rubberband` integration; see "Forward-looking license commitments" below. |
| LGPL-2.1 / LGPL-3.0 | 0 | Two LGPL FFIs (`chromaprint`, `aubio`) were explicitly routed around in favour of pure-Rust replacements (`rusty-chromaprint`, `dub-bpm`). See PRD §10.2 + `docs/SHIPPED.md` M7.5 / M11b. |

**Net effect.** Every external dependency wired into Dub today is permissive or file-level copyleft only. Nothing in the actual dependency graph contaminates a downstream binary with a viral copyleft obligation. The project is free to be relicensed at any point before the planned M14 `rubberband` integration lands.

---

## Real-time / audio path

### `coreaudio-rs`

* **Version:** 0.14
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/RustAudio/coreaudio-rs
* **Role in Dub:** macOS CoreAudio HAL bindings for both audio input (timecode capture, real-record passthrough) and audio output (deck output to the user's external mixer).
* **Used by:** `dub-audio`
* **Notes:** PRD §4.2 explicitly chose HAL via `coreaudio-rs` over `cpal` for lowest possible latency and direct device control. Pure-Rust wrapper on top of Apple's C frameworks; the frameworks themselves are part of macOS and not redistributed.

### `objc2-core-audio`, `objc2-core-audio-types`, `objc2-audio-toolbox`

* **Version:** 0.3 (all three)
* **License:** MIT
* **Upstream:** https://github.com/madsmtm/objc2
* **Role in Dub:** Direct FFI to the bits of CoreAudio that `coreaudio-rs` does not wrap (e.g. `kAudioDevicePropertyBufferFrameSize`). Versions pinned to whatever `coreaudio-rs` 0.14 transitively requires, to avoid duplicate symbol-version warnings.
* **Used by:** `dub-audio`

### `assert_no_alloc`

* **Version:** 1.1, `default-features = false`
* **License:** MIT
* **Upstream:** https://github.com/Windfisch/rust-assert-no-alloc
* **Role in Dub:** Compile- and runtime-time enforcement that the audio render thread never allocates. PRD §2.2.7 "The audio thread is sacred" is enforced through this crate.
* **Used by:** `dub-engine`, `dub-audio`, tools/rt-audit

### `ringbuf`

* **Version:** 0.4
* **License:** MIT
* **Upstream:** https://github.com/agerasev/ringbuf
* **Role in Dub:** Lock-free SPSC ring buffer for audio thread → analysis thread handoff (peak streams, onset streams, band peak streams) and for the CoreAudio HAL input thread → engine handoff.
* **Used by:** `dub-engine`, `dub-audio`, `dub-peaks`

---

## Decode / encode

### `symphonia`

* **Version:** 0.5
* **License:** MPL-2.0
* **Upstream:** https://github.com/pdeljanov/Symphonia
* **Features enabled:** `wav`, `pcm`, `mp3`, `flac`, `aiff`, `aac`, `alac`, `isomp4`
* **Role in Dub:** All audio file decoding. PRD §1.2 lists WAV, MP3, FLAC, AIFF, ALAC, AAC as the v1 format set; every one of those goes through Symphonia. Used both for full-file decode-to-RAM (PRD §4.4 "tracks fully decoded into RAM on load for bidirectional symmetry") and for fast metadata-only probe (M11c library importer).
* **Used by:** `dub-io`, `dub-cli`
* **MPL-2.0 implications:** File-level copyleft. The library binary may be linked into a proprietary application. Dub does not modify any Symphonia source file, so there is no source-disclosure obligation. Distributed binaries must reproduce the MPL-2.0 license text alongside the symphonia copyright notice.

### `hound`

* **Version:** 3
* **License:** Apache-2.0
* **Upstream:** https://github.com/ruuda/hound
* **Role in Dub:** WAV writer. Used by `dub-cli` for offline-render output (`dub render --to-wav`) and by tests for synthetic WAV fixture generation. Not part of the engine.
* **Used by:** `dub-cli`, test-only in `dub-io`, `dub-library`, `dub-engine`

---

## DSP

### `realfft`

* **Version:** 3
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/HEnquist/realfft
* **Role in Dub:** Real-input FFT (thin wrapper on `rustfft`) used for spectral-flux onset detection in the pure-Rust BPM engine (PRD §5.3, M7.5). ~2× faster than going through full-complex `rustfft` for the all-real-input use case.
* **Used by:** `dub-bpm`, `dub-spectral`
* **Transitively pulls:** `rustfft` (MIT/Apache-2.0).

### `rusty-chromaprint`

* **Version:** 0.3
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/0xcaff/rusty-chromaprint
* **Role in Dub:** Pure-Rust port of Lukáš Lalinský's Chromaprint algorithm (algorithm 2, the same one AcoustID uses). Used by `dub-fingerprint` for library deduplication (PRD §8.1) and reserved for real-record recognition (PRD §5.2.5, v1.1).
* **Used by:** `dub-fingerprint`
* **Transitively pulls:** `rubato` (MIT) for internal resampling to the 11025 Hz target rate. Not directly used by Dub.
* **Notes:** PRD §10.2 documents the M11b decision to use this crate instead of FFI-binding the reference C library (`chromaprint`, LGPL-2.1) for license isolation, no C build dependency, no unsafe FFI surface, and simpler distribution.

---

## Library / catalog (M11a–c)

### `rusqlite`

* **Version:** 0.32
* **License:** MIT
* **Upstream:** https://github.com/rusqlite/rusqlite
* **Features enabled:** `bundled` (ships SQLite source rather than linking the host's `libsqlite3`), `blob` (for the `fingerprints.chromaprint_blob` BLOB handle).
* **Role in Dub:** SQLite bindings for the v1 library catalog at `~/Library/Application Support/Dub/library.sqlite`. Schema is documented in `docs/LIBRARY-SCHEMA.md`.
* **Used by:** `dub-library`
* **Transitively pulls:** SQLite itself (public domain), `libsqlite3-sys` (MIT).
* **Notes:** `bundled` is chosen so we don't depend on the host's SQLite version. macOS ships ancient SQLite builds via `/usr/lib`; bundling gives deterministic schema behaviour across machines at a one-time ~5 s extra first-build cost.

### `uuid`

* **Version:** 1
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/uuid-rs/uuid
* **Features enabled:** `v4`
* **Role in Dub:** Canonical track identity. Every `tracks.id` is a UUIDv4. PRD §8.2 / `docs/LIBRARY-SCHEMA.md` document the choice; v7 (time-ordered) is a future consideration if index locality becomes a measured bottleneck.
* **Used by:** `dub-library`

### `dirs`

* **Version:** 5
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/dirs-dev/dirs-rs
* **Role in Dub:** Platform-correct path resolution. On macOS we resolve to `~/Library/Application Support/Dub/` for the library DB and `~/Library/Caches/Dub/waveforms/` for the analysis-cache sidecars.
* **Used by:** `dub-library`

### `libc`

* **Version:** 0.2
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/rust-lang/libc
* **Role in Dub:** macOS-specific FFI to `getattrlist(2)` (for `ATTR_VOL_UUID` volume-UUID discovery) and `statfs(2)` (for mount-point discovery). PRD §8.2 path-by-volume-UUID strategy.
* **Used by:** `dub-library` (macOS only, target-gated)

### `walkdir`

* **Version:** 2
* **License:** MIT/Unlicense (dual-licensed; choose either)
* **Upstream:** https://github.com/BurntSushi/walkdir
* **Role in Dub:** Recursive filesystem walker for the M11c folder importer. Deterministic alphabetical iteration order (depth-first, sorted within each directory) is load-bearing so re-imports replay in a reproducible order.
* **Used by:** `dub-library`

---

## Apple FFI

### `uniffi`

* **Version:** 0.28 (pinned to the 0.28.x line)
* **License:** MPL-2.0
* **Upstream:** https://github.com/mozilla/uniffi-rs
* **Role in Dub:** Generates the Swift bindings the Apple shell uses to call into `dub-ffi`. Library-mode (no `.udl` file); `#[uniffi::export]` proc-macros on Rust items are the single source of truth.
* **Used by:** `dub-ffi`
* **MPL-2.0 implications:** Same as Symphonia. File-level copyleft, binary linking into a proprietary application is permitted, source-disclosure obligation only on modified UniFFI source files (which Dub does not modify).

---

## Error / utility plumbing

### `thiserror`

* **Version:** 1
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/dtolnay/thiserror
* **Role in Dub:** Derive macro for typed library error enums. Every Rust crate in the workspace uses it for its public error surface.
* **Used by:** `dub-engine`, `dub-io`, `dub-timecode`, `dub-bpm`, `dub-library`, `dub-fingerprint`, `dub-audio`, others

### `anyhow`

* **Version:** 1
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/dtolnay/anyhow
* **Role in Dub:** Opaque error type for binary entry points (`dub-cli`) and non-library glue. Never appears in library APIs.
* **Used by:** `dub-cli`

### `serde`, `serde_json`

* **Version:** 1
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/serde-rs/serde, https://github.com/serde-rs/json
* **Role in Dub:** Structured output and configuration in `dub-cli` (e.g. `dub analyze --json`). Not used in the engine or library data paths.
* **Used by:** `dub-cli`

### `time`

* **Version:** 0.3
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/time-rs/time
* **Features enabled:** `formatting`, `parsing`, `serde`
* **Role in Dub:** Human-readable timestamp formatting in `dub-cli` output.
* **Used by:** `dub-cli`

---

## CLI / terminal UI

### `ratatui`

* **Version:** 0.30
* **License:** MIT
* **Upstream:** https://github.com/ratatui-org/ratatui
* **Role in Dub:** Terminal UI for `dub-cli` interactive subcommands (`dub scope`, `dub levels`, `dub timecode-deck`).
* **Used by:** `dub-cli`

### `crossterm`

* **Version:** 0.29
* **License:** MIT
* **Upstream:** https://github.com/crossterm-rs/crossterm
* **Role in Dub:** Terminal IO backend for `ratatui`. Cross-platform; only macOS path exercised in Dub.
* **Used by:** `dub-cli`

---

## Test-only

These are `dev-dependencies` only. They do **not** appear in any shipped binary. They are listed here for completeness; no attribution is required for tooling that does not enter the distribution.

### `proptest`

* **Version:** 1
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/proptest-rs/proptest
* **Role in Dub:** Property-based testing. Used across `dub-engine`, `dub-timecode`, `dub-bpm`, `dub-dsp` to flush out state-machine and audio-buffer-math edge cases.

### `insta`

* **Version:** 1
* **License:** Apache-2.0
* **Upstream:** https://github.com/mitsuhiko/insta
* **Role in Dub:** Snapshot / golden-file regression tests. Used in `dub-bpm` and `dub-dsp` for DSP output stability.

### `tempfile`

* **Version:** 3
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/Stebalien/tempfile
* **Role in Dub:** Temp directories for fixture-driven tests (M11a migration runner, M11c importer integration tests).

---

## Forward-looking license commitments

These are not in the dep graph today. They are documented here so future work knows the constraints they will impose.

### `rubberband` (planned, M14)

* **License:** GPL-3.0
* **Upstream:** https://breakfastquay.com/rubberband/ (Particular Programs Ltd / Chris Cannam)
* **Planned role in Dub:** Time-stretch and key-lock. `crates/dub-stretch/` is the reserved location; the crate exists as an empty placeholder.
* **License implications:** The workspace currently declares `license = "GPL-3.0-or-later"` in anticipation of this dependency landing. Once `rubberband` is wired in, every distributed Dub binary becomes GPL-3.0 and is subject to source-disclosure on request.
* **Alternatives evaluated for v1:**
  - **zplane élastique** (commercial). The de-facto standard for DJ time-stretch (Serato / Traktor / rekordbox all use it). Per-product commercial licensing.
  - **Signalsmith Stretch** (MIT). Younger but maturing; pure DSP; permissive.
  - **SoundTouch** (LGPL-2.1). Older, lower quality; LGPL dynamic-linking complications for iOS.
  - **Custom phase-vocoder / WSOLA implementation in pure Rust.** Higher implementation cost; full control.
  - **No time-stretch in v1.** Scratch DJs ride the turntable pitch slider; time-stretch is a club-DJ feature first.
* **Commercial-license escape:** Rubber Band is dual-licensed by its author. The commercial license lifts the GPL obligation entirely, allowing a closed-source distribution. Pricing is per-product; contact `breakfastquay.com` for current rates.

### `aubio` (deliberately not linked)

* **License:** LGPL-3.0
* **Status:** Replaced at M7.5 by the pure-Rust `dub-bpm` engine.
* **Reason:** License isolation, no C build dependency, no unsafe FFI surface. The pure-Rust spectral-flux + harmonic-summed autocorrelation engine in `dub-bpm` covers the v1 BPM-estimation requirement at sufficient quality for the target user (scratch DJ, urban music). Aubio is parked as a potential opt-in feature backend if real-music validation demands more accuracy in v1.x.

### `chromaprint` (deliberately not linked)

* **License:** LGPL-2.1
* **Status:** Replaced at M11b by `rusty-chromaprint` (MIT/Apache-2.0).
* **Reason:** Same set of reasons as aubio. The pure-Rust port implements algorithm 2 at full fidelity; Dub's library-internal dedupe use case does not require cross-implementation bit-identity with the reference C library, so the FFI route was unnecessary.

---

## How to ship attribution

When a Dub binary is distributed:

1. Bundle this document (or its rendered equivalent) inside the application bundle at `Contents/Resources/Acknowledgments.md` or similar.
2. Surface it in the macOS `About Dub` panel via a button or `NSAppKit` link.
3. For every MIT-licensed and Apache-licensed dependency listed above, include the full license text and the copyright notice. The canonical practice is one section per dependency, in the same order as this document.
4. Apache-2.0 dependencies require preservation of any `NOTICE` file the upstream ships. `hound` does not currently ship one; `insta` does not currently ship one. If an upgrade brings one in, copy it across.
5. MPL-2.0 dependencies (`symphonia`, `uniffi`) require the full MPL-2.0 license text to be bundled. The MPL only requires source disclosure if Dub modifies the library source itself — Dub does not, so a notice is sufficient.

Generating this list automatically from the cargo build graph is a worthwhile follow-up project (`cargo-about` or `cargo-deny` are the standard tools), gated on the first shipped binary milestone.
