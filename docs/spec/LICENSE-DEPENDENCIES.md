# Third-party dependency licenses

This document enumerates every external library Dub links against, with its license, role, and attribution requirement. It is the **source of truth** for the "Acknowledgments" / "Open Source Licenses" panel a shipped binary must surface.

This document covers the ~40 **direct** dependencies — what each one is for, why it was chosen, and what it costs us. That reasoning is worth a human writing and reading, so it is maintained by hand: if you add, remove or upgrade a direct dependency, update this document in the same commit.

Two machine-checked companions carry what a human should not maintain:

- **`deny.toml`** (`make deny`, and a CI job) is the *gate*. Every licence in the shipped graph must be on its allow-list, the copyleft FFIs Dub deliberately routed around are banned by name, and wildcard versions are refused. Dub is MIT OR Apache-2.0 only because no copyleft dependency was ever taken; this is what keeps that true rather than merely intended.
- **`about.toml` / `about.hbs`** (`make attribution`) generate the exhaustive licence-text bundle that ships in the app — roughly **390 crates** once transitives are counted, against the ~40 documented below. That is not a hand-maintainable list, and the hand-maintained version was already wrong: `assert_no_alloc` is BSD-1-Clause, not MIT, as recorded here for months.

The tables below count direct dependencies. The generated bundle counts everything distributed, which is the number the attribution obligation actually attaches to.

Last verified: M26b (workspace dependency-graph snapshot 2026-07; added `flacenc` + `metaflac` for the vinyl-rip encode stack, and promoted `serde` / `serde_json` from `dub-cli`-only to workspace-wide for the `rip.json` session manifest).

---

## Summary

| License family | Crates | Obligation |
|---|---|---|
| MIT / Apache-2.0 dual-licensed | 22 | Reproduce the license text + copyright notice in distributed binaries. |
| MIT | 9 | Reproduce the license text + copyright notice. |
| Apache-2.0 | 2 | Reproduce the license text + copyright notice; preserve any `NOTICE` file. |
| Unlicense / MIT | 1 | Reproduce the license text (either license satisfies). |
| MPL-2.0 | 2 | File-level copyleft only. The library binary may ship inside a proprietary application; if the library's source files themselves are modified, the modified files must remain MPL-2.0 and be made available on request. Dub does not modify any MPL-2.0 source files. |
| GPL-3.0-or-later | 0 | None, ever. The workspace `license` field used to *reserve* GPL-3.0-or-later in anticipation of a `rubberband` integration; M14 shipped pure-Rust WSOLA instead, the dependency never landed, and the workspace is now **MIT OR Apache-2.0**. See "Forward-looking license commitments" below. |
| LGPL-2.1 / LGPL-3.0 | 0 | Two LGPL FFIs (`chromaprint`, `aubio`) were explicitly routed around in favour of pure-Rust replacements (`rusty-chromaprint`, `dub-bpm`). See PRD §10.2 + `docs/SHIPPED.md` M7.5 / M11b. |

**Net effect.** Every external dependency wired into Dub today is permissive or file-level copyleft only. Nothing in the actual dependency graph contaminates a downstream binary with a viral copyleft obligation. This is what made the relicense to **MIT OR Apache-2.0** possible — no GPL dependency is in the graph or planned (M14 shipped pure-Rust WSOLA instead of `rubberband`), so nothing ever forced the old GPL declaration. **Keeping it possible is now a standing constraint, not a nice-to-have:** a new GPL or LGPL dependency would drag the whole distributed binary back to copyleft, so treat one as a decision to escalate rather than take. The M26 vinyl-rip encode stack was deliberately chosen pure-Rust / permissive (`flacenc` Apache-2.0, `metaflac` MIT); MP3 via LAME (LGPL-2.0-or-later) was evaluated and deferred — see "Forward-looking license commitments" — so this posture is **unchanged**.

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
* **License:** BSD-1-Clause
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
* **Role in Dub:** WAV writer. Used by `dub-cli` for offline-render output (`dub render --to-wav`), by `dub-rip` for the M26 crash-safe 32-bit-float capture spill (`side.raw.wav`), and by tests for synthetic WAV fixture generation. Not part of the engine.
* **Used by:** `dub-cli`, `dub-rip`, test-only in `dub-io`, `dub-library`, `dub-engine`

### `flacenc`

* **Version:** 0.5, `default-features = false`
* **License:** Apache-2.0
* **Upstream:** https://github.com/yotarok/flacenc-rs
* **Role in Dub:** Pure-Rust FLAC encoder for the M26 vinyl-rip pipeline: per-track encodes and the lossless side archive (`side.flac`), both 24-bit. Chosen over an MP3/LAME route specifically so the encode stack stays permissive and the dependency-graph posture is unchanged — see "MP3 / LAME" under forward-looking commitments.
* **Used by:** `dub-encode`
* **Notes:** flacenc shrinks StreamInfo's `min_block_size` to the short final frame, which symphonia then rejects as a variable-blocksize stream. `dub-encode` forces `min_block_size == max_block_size` after encoding (as libFLAC does); the workaround is documented at the call site in `crates/dub-encode/src/encode.rs`.

### `metaflac`

* **Version:** 0.2
* **License:** MIT
* **Upstream:** https://github.com/jameshurst/rust-metaflac
* **Role in Dub:** Vorbis-comment + PICTURE (front-cover) tagging on the FLACs the M26 rip pipeline encodes. Fields follow the MusicBrainz Picard convention, including `MUSICBRAINZ_TRACKID`, `MUSICBRAINZ_ALBUMID`, and `DISCOGS_RELEASE_ID`, so M26c recognition writes provenance into the files themselves rather than into the library schema (see `LIBRARY-SCHEMA.md`).
* **Used by:** `dub-encode`

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
* **Role in Dub:** Pure-Rust port of Lukáš Lalinský's Chromaprint algorithm. Used by `dub-fingerprint` for library deduplication (PRD §8.1, TEST1 preset — see `LIBRARY-SCHEMA.md` "Fingerprint parameters") and reserved for real-record recognition (PRD §5.2.5, v1.1). AcoustID's database is built on TEST2, so the planned M26c AcoustID lookup computes a separate TEST2 fingerprint transiently; stored dedupe blobs are unchanged.
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

### `quick-xml`

* **Version:** 0.36
* **License:** MIT
* **Upstream:** https://github.com/tafia/quick-xml
* **Role in Dub:** Streaming XML pull-parser for the M12b Traktor `collection.nml` importer and the M12c iTunes `Library.xml` (plist) importer. We read attributes only — no DOM, no serde derive — so memory stays flat even on a 100k-track collection. MIT, GPL-compatible. (The future Serato/rekordbox XML importer will reuse it.)
* **Used by:** `dub-library`

---

### `id3`

* **Version:** 1.16
* **License:** MIT
* **Upstream:** https://github.com/polyfloyd/rust-id3
* **Role in Dub:** Reads ID3v2 `GEOB` (general-encapsulated-object) frames out of audio files for the M11e Serato importer. Serato keeps its beat grid / hot cues / loops / gain in `GEOB` frames keyed by description (`Serato BeatGrid`, `Serato Markers2`, `Serato Autotags`); symphonia only surfaces standard tag keys, so a dedicated ID3 reader is needed. MP3 / AIFF / WAV containers; MP4 / FLAC deferred. MIT, GPL-compatible.
* **Used by:** `dub-library`

---

### `base64`

* **Version:** 0.22
* **License:** MIT/Apache-2.0
* **Upstream:** https://github.com/marshallpierce/rust-base64
* **Role in Dub (M26c):** Encodes the compressed Chromaprint fingerprint as base64url-unpadded, which is the form AcoustID's `fingerprint` field takes.
* **Role in Dub:** Decodes the base64 payloads inside Serato's `Serato Markers2` / `Serato Autotags` GEOB blobs (M11e). Already present transitively in the dependency graph; M11e makes it a direct dependency. We use a padding-indifferent STANDARD engine because Serato's payloads are inconsistently padded.
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
* **Role in Dub:** Structured output in `dub-cli` (e.g. `dub analyze --json`) and, since M26, the crash-safe `rip.json` session manifest in `dub-rip` — session state that must survive a crash mid-rip, which promoted `serde` from a `dub-cli`-only dependency to the workspace set. Still not used in the engine or the library's SQLite data path.
* **Used by:** `dub-cli`, `dub-rip`

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

## Network (M26c)

### `ureq` (the first and only network dependency)

* **License:** MIT OR Apache-2.0.
* **Upstream:** https://github.com/algesten/ureq
* **Role in Dub:** the HTTP client behind `dub-recognize`'s `Http` trait —
  AcoustID, MusicBrainz and Discogs lookups for ripped vinyl.
* **Why this one.** Blocking, so no async runtime is dragged in for what is a
  background batch job over a handful of tracks. TLS via **rustls**, not
  OpenSSL, so there is no C build dependency and the pure-Rust posture that
  `dub-bpm`, `dub-fingerprint`, `dub-stretch` and `dub-encode` all chose
  survives its first contact with the network.
* **Blast radius.** Reachable from exactly one type (`http::UreqHttp`) in one
  leaf crate. Nothing else in the workspace links it, and `dub-rip` never calls
  recognition on a path that can fail a commit — a rip works with the cable
  out. That containment is the point: it is what lets a network dependency into
  a project whose first principle is reliability on stage.
* **License effect:** none. Permissive, GPL-compatible, no copyleft obligation.

## Forward-looking license commitments

These are not in the dep graph today. They are documented here so future work knows the constraints they will impose.

### ~~`rubberband`~~ — evaluated at M14 and **dropped**

* **License:** GPL-3.0
* **Upstream:** https://breakfastquay.com/rubberband/ (Particular Programs Ltd / Chris Cannam)
* **Outcome: not linked, and not planned.** M14 benched it against a pure-Rust WSOLA written for `dub-stretch`: the WSOLA held pitch as well, beat it on transients and latency, and cost ~30 % more CPU (tunable via the search window). The GPL dependency never landed, and `crates/dub-stretch/` is a complete implementation rather than a placeholder.
* **License implications, had it landed:** every distributed Dub binary would have become GPL-3.0, subject to source-disclosure on request. Because it did not land, nothing in the graph ever forced the licence — and the workspace has since been relicensed to **MIT OR Apache-2.0** (PRD §11).
* **Alternatives evaluated at M14** (the last one won):
  - **zplane élastique** (commercial). The de-facto standard for DJ time-stretch (Serato / Traktor / rekordbox all use it). Per-product commercial licensing.
  - **Signalsmith Stretch** (MIT). Younger but maturing; pure DSP; permissive.
  - **SoundTouch** (LGPL-2.1). Older, lower quality; LGPL dynamic-linking complications for iOS.
  - **Custom phase-vocoder / WSOLA implementation in pure Rust.** Higher implementation cost; full control. ← **chosen**.
  - **No time-stretch in v1.** Scratch DJs ride the turntable pitch slider; time-stretch is a club-DJ feature first.
* **Commercial-license escape (moot):** Rubber Band is dual-licensed, and the commercial licence would have lifted the GPL obligation. Not needed — the dependency was never taken.

### `aubio` (deliberately not linked)

* **License:** LGPL-3.0
* **Status:** Replaced at M7.5 by the pure-Rust `dub-bpm` engine.
* **Reason:** License isolation, no C build dependency, no unsafe FFI surface. The pure-Rust spectral-flux + harmonic-summed autocorrelation engine in `dub-bpm` covers the v1 BPM-estimation requirement at sufficient quality for the target user (scratch DJ, urban music). Aubio is parked as a potential opt-in feature backend if real-music validation demands more accuracy in v1.x.

### `chromaprint` (deliberately not linked)

* **License:** LGPL-2.1
* **Status:** Replaced at M11b by `rusty-chromaprint` (MIT/Apache-2.0).
* **Reason:** Same set of reasons as aubio. The pure-Rust port implements the Chromaprint presets at full fidelity; Dub's library-internal dedupe use case does not require cross-implementation bit-identity with the reference C library, so the FFI route was unnecessary.

### MP3 / LAME (evaluated and deferred, M26)

* **License:** LGPL-2.0-or-later (LAME itself and the Rust binding crates around it)
* **Status:** Deliberately not linked. The M26 vinyl-rip encode stack ships FLAC-only (`flacenc`, Apache-2.0 + `metaflac`, MIT).
* **Reason:** An MP3-320 rip export would have made LAME the first non-permissive library actually linked into Dub. FLAC is lossless, encodable in pure Rust under a permissive license, and symphonia already decodes it — the rip use case is fully covered at zero license cost. Revisit only if real users demand MP3 export for interop with other players.

**`ureq` is no longer forward-looking** — it landed with M26c and is
documented in the wired-dependency section above.

---

## Patent awareness — downbeat detection (informational)

Dub's downbeat refinement (`dub-bpm`, `refine_downbeat_backbeat`) uses the
standard metrical-emphasis heuristic — snare/clap on beats 2 & 4, bass drum
on beat 1 — which is long established in the published music-information-
retrieval literature, predating any relevant patent:

- M. Goto & Y. Muraoka, "A Real-time Beat Tracking System for Audio Signals,"
  ICMC 1995 (assumes bass-on-strong / snare-on-weak beats).
- M. Davies & M. Plumbley, "A Spectral Difference Approach to Downbeat
  Extraction in Musical Audio," EUSIPCO 2006.
- J. Hockman, M. Davies & I. Fujinaga, "One in the Jungle: Downbeat Detection
  in Hardcore, Jungle, and Drum and Bass," ISMIR 2012.

A vendor patent in this area exists (AlphaTheta / Pioneer US 11,176,915 B2,
"Song analysis device," filed 2017, in force). Dub does **not** practise its
claims: that patent recites discrete snare/bass *sounding-position detectors*
and selection of the "first sounding position from the start above a
threshold"; Dub instead sums band-attack energy into four bar-phase bins over
the whole track and compares aggregate contrast — the general published
technique above, not the patent's specific apparatus. Assessed low risk for a
pre-alpha project; a formal freedom-to-operate opinion is not required at this
stage and should be revisited before any commercial distribution. Note that
Apache-2.0 (one half of Dub's dual licence) carries an explicit patent grant
and retaliation clause, which is part of why it is offered alongside MIT.

---

## How to ship attribution

**This is done and automated.** `make attribution` runs `cargo-about`
(`about.toml` + `about.hbs`) over the real macOS build graph and writes
`apple/Dub/Resources/Acknowledgments.html` — every licence text, one section
per distinct copyright, headed by the crates it covers. `apple/project.yml`
bundles it into `Dub.app/Contents/Resources/`, and the About panel links to it
("Open source licenses"). The generated file is **tracked**, so a release build
never depends on `cargo-about` being installed.

That satisfies the obligations directly: full text and copyright notice for
every MIT / Apache / BSD / ISC / Zlib dependency, and the full MPL-2.0 text for
`symphonia` and `uniffi` (MPL requires source disclosure only for *modified*
MPL files, and Dub modifies none, so a notice suffices).

Two things still need a human:

1. **Regenerate after any dependency change** — `make attribution`, committed
   alongside. Nothing yet fails the build if it goes stale; a checked-in-diff
   CI step would close that.
2. **Apache-2.0 `NOTICE` files** must be preserved if an upstream starts
   shipping one. `hound` and `insta` do not today. `cargo-about` does not
   collect `NOTICE` files, so this remains a manual check on upgrade.
