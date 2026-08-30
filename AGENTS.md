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
   are first-class. Whole tracks are decoded into RAM on load (decode-ahead:
   playback starts once the head is resident; the tail streams in behind an
   atomic watermark at hundreds of times realtime — PRD §4.4 / §6.4).
6. **The audio thread is sacred.** No alloc, no lock, no syscall, no logging,
   no I/O, no `unwrap()`, no `dyn Trait` heap allocation. Enforced at compile
   time via `RealtimeContext` token + at runtime via `assert_no_alloc`.

---

## Repo layout

```
crates/
  dub-engine/        Audio graph, transport, RT-safety types, ThruSource (M7). Hot path.
  dub-audio/         CoreAudio HAL input + output, ringbuf-buffered handoff.
  dub-dsp/           Resamplers, filters, and the shipped FX: EchoOut (M15), the PT2399 dub echo,
                     the three siren units (GS1 / DS01E / SN76477) and the vintage chain
                     (spring / RE-201 / BigKnob / phaser) parked behind the deferred FX-deck role (M16).
  dub-stretch/       M14 — pure-Rust WSOLA time-stretch / key-lock engine. No unsafe, no C deps
                     (Rubber Band was benched and dropped; see the M14 row in PRD §12.0).
  dub-io/            symphonia-based decoders, in-memory track buffers.
  dub-encode/        M26 — offline FLAC encode (flacenc) + Vorbis-comment/PICTURE tagging (metaflac)
                     for ripped tracks. Deliberately permissive-only (MP3/LAME deferred).
  dub-timecode/      Serato CV02 + Traktor MK1 + Traktor MK2 decoders (clean-room).
  dub-thru/          Thru-mode source-detection classifier only (§5.1.1; placeholder).
                     The Thru *passthrough itself* (ThruSource) lives in dub-engine.
  dub-bpm/           M7.5 + M8 — BpmEstimator (DSP core), BpmTracker (estimator + hysteresis), BpmStream (per-deck off-RT analysis thread), analyze_bpm (offline). Pure-Rust spectral-flux + harmonic-summed autocorrelation. Aubio backend deferred to a future opt-in feature flag.
                     Aubio's LGPL boundary is confined to this leaf crate.
  dub-fingerprint/   Pure-Rust Chromaprint via rusty-chromaprint. Used for library dedupe (M11b, shipped) and parked for real-record recognition (v1.1).
  dub-library/       SQLite + import adapters (Serato/Traktor/rekordbox/iTunes/Lexicon).
  dub-rip/           M26 — vinyl-rip session engine: RipSession state machine, off-RT capture
                     worker (record-tap ring → crash-safe WAV spill + live envelope), split plan,
                     rip.json manifest, commit (encode + tag + import + side archive). M26b adds
                     adaptive gap detection, needle-drop auto-start + run-out auto-stop, spill
                     salvage / session recovery, and re-split from the lossless archive.
                     Fully offline.
  dub-controller/    HID/MIDI abstractions (placeholder; v1.x+).
  dub-ffi/           UniFFI Swift bindings — `DubEngine`, `DubLibrary` and `DubRipSession`.
                     `FFI_VERSION` is the contract number; bump it and README together (docs-check gates it).
  dub-cli/           `dub` binary — smoke / play / capture / levels /
                     timecode-deck / thru / scope / calibrate / analyze /
                     rip / rip-tune / rip-resplit / decode-timecode.

apple/               SwiftUI + AppKit shell (M0.5+).
tools/rt-audit/      RT-thread allocation auditor (binary tool).
docs/                README.md (routing guide — which doc to load for a task) + UI-BACKLOG.md.
  spec/              PRD.md (forward-looking spec), PRD-BEATS.md, ARCHITECTURE.md,
                     LIBRARY-SCHEMA.md, LIBRARY-FORMATS.md, LICENSE-DEPENDENCIES.md.
  history/           SHIPPED.md (one-line-per-milestone index; detail in git) +
                     LESSONS.md (pitfalls + load-bearing decisions — read before touching a subsystem).
  investigations/    BPM-DETECTOR-V2 + WAVEFORM-JITTER runbooks, and BEATMATCH-AID-STILLPOINT
                     (the binding sub-spec for the shipped Stillpoint aid; PRD §9.4 is the summary).
  html/              status dashboard (index / roadmap / backlog).
scripts/             Build, codesign, notarize helpers (M0.5 / M20).
.cursor/             Cursor rules + hooks for AI-assisted dev.
.claude/             Claude Code settings + hooks (mirrors .cursor/; see CLAUDE.md).
.githooks/pre-push   fmt-check + clippy + docs-check + tests — the gates that break main.
.github/workflows/   CI pipeline.
fuzz/                cargo-fuzz targets for the library parsers.
testdata/
  rip-baselines/     Three real record sides (24-bit FLAC, gitignored — 748 MB) + a tracked
                     README. Ground truth for every constant in dub-rip/src/gaps.rs; replay
                     with `dub rip-tune`. Never re-fit those gates against a single record.
Makefile             test / app / ci / sweep / docs-check / snapshot …
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
make sweep         # drop build artifacts unused for 14 days (see below)
make check-stale   # sweep only when target/debug/deps is over STALE_MAX (runs before test / app)
make docs-check    # FFI / schema / crate-count numbers in docs match the code
make app           # build the macOS app
make ci            # docs-check + fmt-check + clippy + test — exactly what CI runs
```

**If the suite suddenly feels slow, count the build artifacts first.**
`target/debug/deps` accumulates, and process exec from a directory that size
gets dramatically slower — nextest runs one process per *test*, so the penalty
lands 1400 times per run. Measured here at 177,922 files: **562 ms per exec
against 7 ms**, which was ~780 s of a 789 s workspace suite; cleaning took it
to **26 s**. `make test` and `make app` now guard against it — over
`STALE_MAX` files they sweep artifacts unused for `STALE_DAYS` — but the
diagnostic is worth knowing:

```bash
ls target/debug/deps | wc -l    # 20k+ and the exec penalty is measurable
make sweep                      # safe any time; never touches the current build
```

Reach for `cargo clean` only when you want the space back — it forces a full
rebuild.

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
- **Run `make ci` before pushing** — `docs-check` + `fmt-check` + `clippy` +
  tests, the exact gates CI runs. The tracked `pre-push` hook
  (`make hooks`, or `scripts/bootstrap.sh`) runs all four for you and blocks
  the push if any is red; bypass with `git push --no-verify`. Every red `main`
  push in this repo's history failed on one of the first three. The suite used
  to be skipped there as too slow at ~11 minutes; nearly all of that was
  build-artifact overhead rather than tests (below), and it is ~26 s now — set
  `DUB_PREPUSH_TESTS=0` to skip it anyway.
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
- `rusty-chromaprint` (MIT/Apache) — pure-Rust port of the Chromaprint algorithm. Used in `dub-fingerprint` for library dedupe (M11b, shipped) with the **TEST1** preset (`Configuration::preset_test1()`); AcoustID's database is built on TEST2, so M26c recognition will compute a separate TEST2 fingerprint transiently (stored dedupe blobs unchanged). M11b chose pure-Rust over the LGPL-2.1 C library (`chromaprint`) for the same reasons `dub-bpm` chose pure-Rust over aubio: license isolation, no C build dep, no unsafe FFI surface, simpler distribution.
- `rusqlite` (MIT, feature `bundled`) — SQLite for the M11 library catalog
- `uuid` (MIT/Apache), `dirs` (MIT/Apache), `libc` (MIT/Apache), `walkdir` (MIT/Unlicense) — library plumbing
- `quick-xml` (MIT) — streaming XML pull-parser for the M12b Traktor `collection.nml`, M12c iTunes `Library.xml` (plist), and M12d rekordbox `rekordbox.xml` (`DJ_PLAYLISTS`) importers (`dub-library`). Attributes-only, no DOM; flat memory on huge collections. GPL-compatible. (The rekordbox importer reads the XML export, not the encrypted `master.db` — see `LIBRARY-FORMATS.md`.)
- `id3` (MIT) — reads ID3v2 `GEOB` frames from audio files for the M11e Serato importer (`dub-library`). Serato keeps its beat grid / hot cues / loops / gain in `GEOB` blobs (`Serato BeatGrid` / `Serato Markers2` / `Serato Autotags`); symphonia only exposes standard tag keys, so a dedicated ID3 reader is needed. MP3/AIFF/WAV; MP4/FLAC deferred. GPL-compatible.
- `base64` (MIT/Apache) — decodes the base64 payloads inside Serato's Markers2 / Autotags GEOB blobs (M11e, `dub-library`). Already transitive; M11e makes it a direct dep.
- `uniffi` (MPL-2.0) — Swift FFI surface generator
- `assert_no_alloc` (MIT) — RT-safety enforcement
- `ringbuf` (MIT) — lock-free SPSC
- `hound` (Apache-2.0) — WAV writer for offline render, test fixtures, and the M26 crash-safe rip spill (`dub-rip`)
- `flacenc` (Apache-2.0) — pure-Rust FLAC encoder for the M26 rip pipeline (ripped tracks + lossless side archive, 24-bit; `dub-encode`). Chosen over LAME/MP3 to keep the dep graph fully permissive — see `LICENSE-DEPENDENCIES.md`.
- `metaflac` (MIT) — Vorbis-comment + PICTURE tagging on ripped FLACs (M26, `dub-encode`), Picard-convention fields (`MUSICBRAINZ_TRACKID` / `MUSICBRAINZ_ALBUMID` / `DISCOGS_RELEASE_ID`) so M26c recognition can write provenance into the files themselves
- `serde` / `serde_json` (MIT/Apache) — was `dub-cli`-only; M26 promotes it to the workspace set for the crash-safe `rip.json` session manifest (`dub-rip`)
- `thiserror` / `anyhow` (MIT/Apache) — error plumbing
- `ratatui` / `crossterm` / `time` (MIT/Apache) — `dub-cli` only

Planned but **not** in the dep graph yet (placeholder crates exist):

- `rubberband` (FFI, **GPL-3.0**) — *evaluated at M14 and dropped.* The pure-Rust WSOLA stretcher in `dub-stretch` matched it on pitch and beat it on transients + latency, so the GPL dep never landed. The workspace still declares `license = "GPL-3.0-or-later"`, but the actual dep graph is fully permissive (MIT / Apache / MPL-2.0 only) — see PRD §11.
- `aubio` (FFI, LGPL-3.0) — *deliberately not linked.* M7.5 shipped a pure-Rust BPM engine in `dub-bpm`; aubio is parked as a future opt-in feature backend if real-music validation demands more accuracy.
- `chromaprint` (FFI, LGPL-2.1) — *deliberately not linked.* Replaced at M11b by `rusty-chromaprint` (pure-Rust, MIT/Apache) for license isolation + no C build dep.
- LAME / `mp3lame` bindings (LGPL) — *considered and deferred.* MP3-320 rip export (M26) would have been the first non-permissive library actually linked; FLAC via `flacenc` covers the rip use case at zero license cost. Revisit only if users demand MP3 export.
- `ureq` (MIT/Apache, rustls chain) — planned for M26c recognition (`dub-recognize`: AcoustID + MusicBrainz + Discogs). First network dependency; confined behind an `Http` trait in a leaf crate so rips keep working fully offline.

We are GPLv3 by declaration (the reservation originally anticipated Rubber Band; M14 dropped it). See PRD §11. Nothing in the dep graph forces GPL — the posture stays flexible, and M26 deliberately kept it that way (FLAC over MP3/LAME).

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
