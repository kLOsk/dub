# AGENTS.md — project context for AI assistants

> **Always-loaded context for any AI agent working in this repo.**
> Source of truth for product spec: `docs/spec/PRD.md`. Read it first if uncertain.

---

## What is Dub?

Dub is a **timecode-vinyl DJ application** for scratch DJs and vinyl enthusiasts.
Mac-first, Rust-cored, GPLv3, pre-alpha.

The audience is the urban / sound-system / scratch DJ — hip hop, reggae, dnb,
dubstep — playing in front of audiences of hundreds to thousands. Reliability is
the **primary** feature.

We are **not** building a club-DJ all-in-one (no Pioneer/Engine territory) and
we are **not** building a controller-only DJ app (no Serato/rekordbox territory).

---

## Non-negotiable design principles

1. **No Mouse DJ.** A "Mouse DJ" performs the *whole set* on screen — pitch,
   crossfade, EQ, gain, mix-cueing all by mouse. Dub refuses to be that
   surface: those *continuous performance gestures* live on the turntable +
   external mixer + keyboard, never the mouse. The mouse is fine for everything
   else, including momentary **aux triggers** (loop / hot-cue / sampler pads),
   transport, library, and config — a DVS DJ clicking a loop button is *not* a
   Mouse DJ. See PRD §1.
2. **External mixer is the product.** No software mixer in v1/v2. We require a
   ≥4-in/4-out audio interface; the user's external mixer does the mixing.
3. **Real records are first-class citizens.** Thru mode passes a live record
   straight through; the DJ selects it explicitly via the per-deck
   INT · TC · THRU switch (auto-detection deferred — PRD §5.1.1). FX work on
   real records.
4. **Reliability over features.** A crash on stage ends a DJ's career night.
   We accept ~20–30 % slower velocity to never ship a show-stopper.
5. **Forward and backward playback are byte-for-byte symmetric.** Manual rewinds
   are first-class. Whole tracks are decoded into RAM on load.
6. **The audio thread is sacred.** No alloc, no lock, no syscall, no logging,
   no I/O, no `unwrap()`, no `dyn Trait` heap allocation. Enforced at compile
   time via `RealtimeContext` token + at runtime via `assert_no_alloc`.

---

## Repo layout

```
crates/
  dub-engine/        Audio graph, transport, RT-safety types, ThruSource (M7). Hot path.
  dub-audio/         CoreAudio HAL input + output, ringbuf-buffered handoff.
  dub-dsp/           Resamplers, filters, FX building blocks (placeholder for v1 FX).
  dub-stretch/       Rubber Band FFI wrapper (license-isolated, M14).
  dub-io/            symphonia-based decoders, in-memory track buffers.
  dub-timecode/      Serato CV02 + Traktor MK1 + Traktor MK2 decoders (clean-room).
  dub-thru/          Thru-mode source-detection classifier only (§5.1.1; placeholder).
                     The Thru *passthrough itself* (ThruSource) lives in dub-engine.
  dub-bpm/           M7.5 + M8 — BpmEstimator (DSP core), BpmTracker (estimator + hysteresis), BpmStream (per-deck off-RT analysis thread), analyze_bpm (offline). Pure-Rust spectral-flux + harmonic-summed autocorrelation. Aubio backend deferred to a future opt-in feature flag.
                     Aubio's LGPL boundary is confined to this leaf crate.
  dub-fingerprint/   Pure-Rust Chromaprint via rusty-chromaprint. Used for library dedupe (M11b, shipped) and parked for real-record recognition (v1.1).
  dub-library/       SQLite + import adapters (Serato/Traktor/rekordbox/iTunes/Lexicon).
  dub-controller/    HID/MIDI abstractions (placeholder; v1.x+).
  dub-ffi/           UniFFI Swift bindings (placeholder; M0.5).
  dub-cli/           `dub` binary — smoke / play / capture / levels /
                     timecode-deck / thru / scope / calibrate / analyze /
                     decode-timecode.

apple/               SwiftUI + AppKit shell (M0.5+).
tools/rt-audit/      RT-thread allocation auditor (binary tool).
docs/                README.md (routing guide — which doc to load for a task) + UI-BACKLOG.md.
  spec/              PRD.md (forward-looking spec), PRD-BEATS.md, ARCHITECTURE.md,
                     LIBRARY-SCHEMA.md, LIBRARY-FORMATS.md, LICENSE-DEPENDENCIES.md.
  history/           SHIPPED.md (one-line-per-milestone index; detail in git) +
                     LESSONS.md (pitfalls + load-bearing decisions — read before touching a subsystem).
  investigations/    BPM-DETECTOR-V2 + WAVEFORM-JITTER runbooks.
  html/              status dashboard (index / roadmap / backlog).
scripts/             Build, codesign, notarize helpers (M0.5 / M20).
.cursor/             Cursor rules + hooks for AI-assisted dev.
.claude/             Claude Code settings + hooks (mirrors .cursor/; see CLAUDE.md).
.github/workflows/   CI pipeline.
```

---

## Build / test commands

Always prefer the Makefile; falls back to cargo when needed.

```bash
make test          # cargo nextest run --workspace + clippy -D warnings
make smoke         # run the dub-cli binary; should print "engine OK"
make rt-audit      # run the RT-safety test harness
make fmt           # cargo fmt
make clippy        # cargo clippy --all-targets -- -D warnings
make cov           # cargo llvm-cov (requires cargo-llvm-cov installed)
make fuzz-quick    # run all fuzz targets for 60s each (placeholder until parsers exist)
make soak          # run the offline render soak harness for 1 hour
```

For a single crate:
```bash
cargo nextest run -p dub-engine
cargo clippy -p dub-engine -- -D warnings
```

---

## Tooling conventions for AI agents

- **Use `Read`, never `cat`/`head`/`tail`.**
- **Use `StrReplace`/`Write`, never `sed`/`awk`/heredoc-redirect.**
- **Use `SemanticSearch` for concept queries; `Grep` for exact symbol lookups.**
- **Use `cargo nextest run`, not `cargo test`** — faster, parallel, better output.
- **Run `cargo clippy --all-targets -- -D warnings`** after non-trivial Rust edits.
- **Run `cargo fmt`** before opening a PR; the pre-commit hook should do this.
- **No comments that narrate what the code does.** Comments explain *why*, not
  *what*. Code that needs `// Increment counter` should just say `counter += 1`.

---

## Testing discipline

We run TDD on Rust code. See PRD §2.2 for full philosophy. Quick rules:

- **Write a failing test first.** Then make it pass. Then refactor.
- **Property tests** (`proptest`) for state machines, audio buffer math, parsers,
  timecode decoder.
- **Golden tests** (`insta`) for DSP regression — record a reference output, hash,
  compare. Snapshot updates require explicit acceptance.
- **Integration tests** in `tests/` for full engine pipelines.
- **RT-safety tests** are non-negotiable. Any test exercising the audio render
  path must run under `assert_no_alloc::AllocDisabler`. CI fails on RT alloc.
- **No flaky tests.** Fix flakes; never `#[ignore]` to dodge them.
- **Coverage target ≥ 85 %** for non-trivial modules. UI/glue code is exempt.

---

## Branching & commits

- Branch names: `feat/`, `fix/`, `chore/`, `refactor/`, `docs/`, `test/`.
- One concern per PR.
- Conventional commits style:
  - `feat(engine): add bidirectional resampler`
  - `fix(timecode): handle CV02 LFSR drop-out without click`
  - `chore(ci): bump nextest to 0.9.x`
- Linear history on `main` (rebase merges only). No merge commits.
- PR description must include a "Test plan" section.

---

## Where things live (cheat sheet)

| If you need to... | Look here |
|---|---|
| Define an audio thread API | `crates/dub-engine/src/realtime.rs` (RealtimeContext) |
| Add a DSP block | `crates/dub-dsp/src/` |
| Read a music file | `crates/dub-io/src/` |
| Decode timecode | `crates/dub-timecode/src/` |
| Parse a library file | `crates/dub-library/src/<source>.rs` |
| Expose to Swift | `crates/dub-ffi/src/lib.rs` (UniFFI) |
| Test the full engine offline | `crates/dub-cli/` and `crates/dub-engine/tests/` |
| Add a fuzzer | `fuzz/fuzz_targets/` |

---

## Key external libraries (with license notes)

Currently wired (in the actual `Cargo.toml` dependency graph):

- `coreaudio-rs` (MIT/Apache) — CoreAudio I/O
- `objc2-core-audio` / `objc2-core-audio-types` / `objc2-audio-toolbox` (MIT) — CoreAudio FFI for the bits `coreaudio-rs` doesn't wrap
- `symphonia` (MPL-2.0) — audio decoding (features: wav, pcm, mp3, flac, aiff, aac, alac, isomp4)
- `realfft` (MIT/Apache, thin wrapper on `rustfft`) — pure-Rust FFT used by `dub-bpm` for spectral-flux onset detection
- `rusty-chromaprint` (MIT/Apache) — pure-Rust port of the Chromaprint algorithm (algorithm 2). Used in `dub-fingerprint` for library dedupe (M11b, shipped) and parked for real-record recognition (v1.1). M11b chose pure-Rust over the LGPL-2.1 C library (`chromaprint`) for the same reasons `dub-bpm` chose pure-Rust over aubio: license isolation, no C build dep, no unsafe FFI surface, simpler distribution.
- `rusqlite` (MIT, feature `bundled`) — SQLite for the M11 library catalog
- `uuid` (MIT/Apache), `dirs` (MIT/Apache), `libc` (MIT/Apache), `walkdir` (MIT/Unlicense) — library plumbing
- `quick-xml` (MIT) — streaming XML pull-parser for the M12b Traktor `collection.nml` importer + the M12c iTunes `Library.xml` (plist) importer (`dub-library`). Attributes-only, no DOM; flat memory on huge collections. GPL-compatible. The forthcoming Serato/rekordbox XML importer will reuse it.
- `id3` (MIT) — reads ID3v2 `GEOB` frames from audio files for the M11e Serato importer (`dub-library`). Serato keeps its beat grid / hot cues / loops / gain in `GEOB` blobs (`Serato BeatGrid` / `Serato Markers2` / `Serato Autotags`); symphonia only exposes standard tag keys, so a dedicated ID3 reader is needed. MP3/AIFF/WAV; MP4/FLAC deferred. GPL-compatible.
- `base64` (MIT/Apache) — decodes the base64 payloads inside Serato's Markers2 / Autotags GEOB blobs (M11e, `dub-library`). Already transitive; M11e makes it a direct dep.
- `uniffi` (MPL-2.0) — Swift FFI surface generator
- `assert_no_alloc` (MIT) — RT-safety enforcement
- `ringbuf` (MIT) — lock-free SPSC
- `hound` (Apache-2.0) — WAV writer for offline render + test fixtures
- `thiserror` / `anyhow` (MIT/Apache) — error plumbing
- `ratatui` / `crossterm` / `serde` / `serde_json` / `time` (MIT/Apache) — `dub-cli` only

Planned but **not** in the dep graph yet (placeholder crates exist):

- `rubberband` (FFI, **GPL-3.0**) — time-stretch. Slated for M14 in `crates/dub-stretch/` (currently empty). The workspace `license = "GPL-3.0-or-later"` reservation anticipates this dependency landing; until it does, the actual dep graph is fully permissive (MIT / Apache / MPL-2.0 only). Commercial-license alternatives exist if a closed-source distribution model is chosen — see PRD §11.
- `aubio` (FFI, LGPL-3.0) — *deliberately not linked.* M7.5 shipped a pure-Rust BPM engine in `dub-bpm`; aubio is parked as a future opt-in feature backend if real-music validation demands more accuracy.
- `chromaprint` (FFI, LGPL-2.1) — *deliberately not linked.* Replaced at M11b by `rusty-chromaprint` (pure-Rust, MIT/Apache) for license isolation + no C build dep.

We are GPLv3 because of the planned Rubber Band integration. This is a deliberate choice. See PRD §11. The license posture stays flexible until M14 ships Rubber Band: today nothing in the dep graph forces GPL.

---

## Things to never do

- Allocate on the audio thread.
- Add a software mixer / EQ / crossfader to the UI.
- Add a "preview" feature to the library browser.
- Add features that aren't justified by the target user (scratch / urban DJ).
- Skip writing tests for non-trivial logic.
- Use `unwrap()` outside test code.
- Commit secrets.
- Add a dependency without checking its license against our GPL stance.
