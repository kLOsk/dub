# Dub — Product Requirements Document

> macOS app bundle id: `com.klos.dub`.

**Version:** 0.1 (pre-alpha working spec)
**Author:** klos + Cursor
**Date:** 2026-05-20
**Status:** Working product spec. Implemented history lives in [`SHIPPED.md`](../history/SHIPPED.md); docs routing lives in [`docs/README.md`](../README.md).

---

## 1. Vision

**Dub** is a desktop DJ application for **scratch DJs and vinyl enthusiasts** built around a single, uncompromising commitment: **best-in-class control-vinyl latency and feel on macOS**, plus first-class support for playing real records *through* the software (Thru mode) with effects, auto-BPM, and — eventually — automatic waveform recognition.

It is the spiritual successor to Serato Scratch Live for the urban music scene — hip hop, reggae/dub, dnb, dubstep, jungle, scratch — DJs whose audience comes for the music, not the production. The product is opinionated: it does a small set of things extremely well, and it explicitly does not try to be a club-DJ all-in-one.

The ethos:

- **The software is a tool, not a stage.** Reduced UI. Fast workflows. No feature shall exist if it has no function.
- **No Mouse DJ for performance gestures.** A *"Mouse DJ"* performs the **whole set on screen** — presses play and **pitches, crossfades, EQs, rides gain, and cues the mix, all with the mouse**. *That* is the surface Dub refuses to be. It is **not** a DVS DJ on an external mixer who clicks a loop pad or a hot cue with the mouse — that is completely fine. The precise rule: a *continuous performance gesture* — pitch, scratch, crossfade, EQ, gain, cueing live to the audience — never goes through the mouse; those live on the turntable + external mixer + keyboard, always. Everything else **may** be mouse-driven (and often is): configuration in Preferences; library navigation and search; loading tracks onto decks; **track-position navigation and cue-point location** (single click on the overview to jump the playhead, vinyl-style drag-scratch on the zoomed waveform — see §6.1); the **transport / recovery controls** — Play, Pause, Restart (dirty-needle recovery §6.1.2, casual pre-performance playback §6.1.3); and the **auxiliary performance triggers** — loop pads, hot-cue pads, sampler / quick-scratch slots — which are momentary button presses, not the continuous gestures the rule guards. The forbidden list is short and precise: no software crossfader, no software EQ, no software cue/preview channel, no software pitch fader. **Mouse-driven vinyl-style scratching is allowed but only as a cue-locating affordance**, not as a routed-to-FOH performance gesture: the rate is driven by the cursor's velocity (mouse-still means silence, just like a stylus on a stationary platter), it never auto-plays from the click, and the audience never hears it because the DJ's external mixer is the only thing routed to the FOH. The DJ uses it to find the *exact* downbeat of an intro or the start of a kick before the platter takes over. Everything else is on the table. See §6 for the positive list, §5.3 / §6.6 / §15 for the negative list.
- **Real records are first-class citizens.** A scratch DJ playing alongside another DJ on real wax should get the same Dub features (FX, BPM detection, eventually waveform recognition) without recabling.
- **Reliability is the product.** This software runs in front of audiences of hundreds to thousands of paying people. A crash on stage is a career moment for the DJ. We treat reliability as the *primary* feature, ahead of every other capability. Test-driven development is not optional, not negotiable, and not deferable. See §2.2.

---

## 2. Target user

Primary persona: **scratch / urban / sound-system DJ** with the following profile:

- Plays on real turntables with control vinyl (Serato CV02 / Traktor MK2 timecode).
- Uses an **external hardware mixer** (Rane TTM/MP, DJM-S, Numark Scratch, vintage Vestax, etc.) for cueing, EQ, filters, and crossfading.
- Often plays a mix of real records and digital files.
- Wants software that gets out of the way and gives back what hardware can't: large library access, smart utility FX, quick sample-throws, loops.

Secondary persona: **vinyl enthusiast / home DJ** wanting to play their digital library with timecode vinyl on a small home setup.

### Non-goals (audience)

- Club/festival DJs running CDJs in sync. (rekordbox/Engine territory.)
- Controller-only DJs. (Serato/rekordbox territory.)
- Producers/remixers needing stems, AI separation, or arrangement tools.
- Streamers/influencers needing OBS integration, video sync, etc.

---

## 2.1 Foundational technical decisions & rationale

### Why Rust (not C++)

We chose Rust as the language of the engine. The reasoning is not marketing.

1. **Performance parity with C++.** Rust has no GC, no runtime, no ARC. The audio render callback compiles to the same kind of machine code C++ would produce. Both languages depend on the same OS audio APIs (CoreAudio, ASIO) for their latency floor.
2. **Memory safety without runtime cost.** The borrow checker eliminates entire classes of bugs (use-after-free, data races, iterator invalidation) at compile time. For a real-time audio codebase maintained by a small team, this is decisive.
3. **The audio thread can be statically guarded.** Rust's type system + the `assert_no_alloc` crate + a custom allocator allow us to *prove* that no allocation, lock, or syscall happens inside the render callback — something C++ requires discipline and code review to enforce.
4. **Better tooling.** `cargo` (build + dependency + test + bench in one tool) vs. CMake/Conan/Catch2/Google Bench. `clippy` is a real linter; C++ has no comparable mainstream equivalent.
5. **Better Apple FFI in 2026.** UniFFI generates safe Swift bindings from a Rust crate; `swift-bridge` and `cargo-xcode` integrate with Xcode. C++ ↔ Swift interop is younger and rougher.
6. **Production-grade audio ecosystem.** `coreaudio-rs`, `symphonia`, `rubato`, `ringbuf`, `assert_no_alloc`, `realfft`, `rustfft` — all mature, all maintained. (Aubio was the original M7.5 plan; pure-Rust took its place — see [`docs/SHIPPED.md`](../history/SHIPPED.md).)
7. **One developer can maintain it.** This is the load-bearing reason. Rust's compile-time guarantees compound; the project gets *easier* to evolve as it grows, which is the opposite of how C++ codebases age.

C++ would only be the right call if (a) we leveraged a large existing C++ codebase, (b) JUCE was a hard requirement, or (c) we hired from a senior C++ DSP talent pool. None apply.

We *do* link to C/C++ libraries (Rubber Band, Aubio, optionally Chromaprint) via FFI. This is fine; FFI is one-way and well-isolated.

### Performance philosophy

> **We optimize until the cost is no longer audibly justified, then we stop.**

Specifically:

- **Best-in-class at the same buffer size as Serato/Traktor.** Not 10× lower latency at 5× the CPU.
- **CPU headroom is a feature.** Users want to run a browser, Slack, OBS alongside Dub. We target < 25 % of one P-core under heavy use (2 decks + key lock + FX + sampler).
- **No micro-optimization theatre.** SIMD where it measurably helps, plain code elsewhere. Profile first.
- **Battery is a constraint.** A scratch DJ on tour using a MacBook Air should not see Dub drain the battery faster than a video call.
- **Marketing claims must hold under real conditions.** "Sub-5 ms latency" with no asterisks. If it requires hog mode + closing other apps + a specific interface, we say so.

---

## 2.2 Quality, testing & reliability — first principle

> **A crash on stage is a career moment for the DJ.** This software is used in front of audiences of hundreds to thousands of paying people. Reliability is our primary feature. Every other priority — features, performance, UI polish — is subordinate to "it works, every time, in front of a crowd."

This section is binding for every line of code in the project. We accept the cost knowingly, because the alternative is unacceptable.

### 2.2.0 Staged rigor — pragmatism before users, rigor before stable

**This is the load-bearing pragmatism of this section.** Until Dub has real users on real gigs, the most stringent reliability gates ("100 cumulative gig-hours zero-crash") are *theatre* — there is nobody to accumulate gig-hours from, and 100 % of pre-alpha-tester crashes are caught and fixed by the developer in seconds. Spending 20–30 % of velocity on those gates pre-users delays the day there *are* users.

We therefore stage the rigor in three phases. **All ground rules are in from M0** because they are cheap to set up and expensive to retrofit. **The release-blocking gates activate progressively** as the project earns the right to enforce them.

| Phase | Trigger | Rules in effect | Gates |
|---|---|---|---|
| **Phase A — Pre-Alpha** (M0 → M17) | No external users yet | TDD discipline (§2.2.1), test taxonomy (§2.2.2), RT-safety enforcement (§2.2.3), parser fuzzing (§2.2.5), CI green required to merge to main, branch protection, snapshot tests for UI | None for "release"; "release" means "the developer dogfoods the latest main daily" |
| **Phase B — Alpha** (M18) | Invite-only, 3–5 trusted DJs run on real gigs | Phase A + soak harness in nightly CI + manual rig checklist (§2.2.10) signed off before each alpha cut + 24h hotfix discipline for crashes reported by alpha testers | "Cut alpha" gated only by the manual rig checklist |
| **Phase C — Beta and Stable** (M19, M20+) | Public opt-in beta, then stable | Phase B + full §2.2.6 SLOs including 100 cumulative beta-gig-hours zero-crash + zero fuzz crashes in last 7 days + no benchmark regressions | Stable release gated by full §2.2.6 SLOs |

**Practical implication for Phase A:**

- Tests are written. CI is green. RT-safety is enforced. Fuzzing runs. **All the framework is operational.**
- We do **NOT** wait for a soak test to merge a feature.
- We do **NOT** require gig-hours to ship a Phase-A "release" — there's no release in this phase, just `main`.
- We **DO** enforce: every PR has tests; every PR is RT-safe; CI is green; no hand-merging around CI.
- The author dogfoods on their own setup daily. Bugs found in dogfooding go through the same fix-test-merge loop as future user-reported bugs.

**Crossing into Phase B happens at M17 / M18** (Polish + Alpha). At this point we activate the soak nightly + manual rig checklist + hotfix discipline.

**Crossing into Phase C happens at M19** (public Beta). At this point the gig-hour gate, the public-beta hotfix turnaround, and the full §2.2.6 SLOs become release-blocking for stable.

**Why this works:** the cost of TDD discipline + RT-safety + fuzzing is high in *culture* but low in *time* once it's set up. The cost of soak tests, manual rig checklists, gig-hour gates, and 24h hotfix discipline is high in *time*. We pay the culture cost from day one (cheap, shapes the codebase), and we defer the time cost until it's earned (expensive, but only meaningful with users).

This makes the engineering bar *higher* for v1.0 stable than for any other phase, and *appropriately matched* to the project's stage at every step before that.

### 2.2.1 Test-driven development (TDD) is the default

For all Rust code (engine, DSP, parsers, library, controllers, FFI surface):

1. **Write a failing test first.** Then write the minimum code to pass it. Then refactor. The standard TDD loop, applied uncompromisingly.
2. **Tests live next to source** (`#[cfg(test)] mod tests` blocks for unit tests; `tests/` directories for integration tests).
3. **Coverage target: ≥ 85 %** of branches in non-trivial modules (verified via `cargo-llvm-cov` in CI). UI code is exempt; see §2.2.4.
4. **No PR is mergeable without tests** for the changed behavior. Reviewer rejects PRs that change behavior without a corresponding test.

Carve-outs (where TDD doesn't apply):

- **SwiftUI views** — use snapshot tests (§2.2.4).
- **CoreAudio I/O proc setup** — physical hardware required; covered by manual checklist.
- **Throwaway exploratory spikes** — explicitly marked `experimental/` and never merged to main.

### 2.2.2 Test taxonomy

Every commit pushes through this stack:

| Type | What it tests | Scope | Run when |
|---|---|---|---|
| **Unit** | Pure functions, small modules | All Rust modules | Per commit (every push, every PR) |
| **Property** (`proptest`) | Invariants over generated input | State machines, DSP buffer math, parsers, timecode decoder | Per commit |
| **Golden** | DSP regression — hash a reference output, compare | All DSP stages, Rubber Band integration, resampler, FX | Per commit |
| **Integration** | Multi-crate orchestration via offline render | Full engine pipelines (load track → render N seconds with synthetic input → assert output) | Per commit |
| **RT-safety** | `assert_no_alloc` engaged during render call | Audio thread code path | Per commit (**hard gate**) |
| **Fuzz** (`cargo-fuzz`) | Malformed input does not crash | All file-format parsers (NML, GEOB, DB6, ID3, ALAC, FLAC, MP3 frame headers) | Continuous (dedicated fuzzer host or CI nightly) |
| **Soak** | Long-running stability | 1+ hour offline playback with synthetic timecode and FX rotation | CI nightly |
| **Performance** | Latency / CPU regression | Microbenchmarks (`criterion`) and macro RT-render benchmarks | Per commit (warn on regression > 5 %, fail on > 15 %) |
| **Snapshot** | UI hasn't changed unexpectedly | SwiftUI views via Swift snapshot library | Per commit on Apple side |
| **Manual rig checklist** | Real hardware behavior | Full release readiness on test rig | Pre-release only |

### 2.2.3 RT-safety is the hardest gate

The audio thread is special. A single allocation, mutex, or syscall on it can cause an audible glitch. The CI pipeline enforces this:

1. **Compile-time hint:** the engine code path inside the render callback only takes a `&mut RealtimeContext<'_>` token. Methods that allocate, lock, or perform I/O are not implemented for `RealtimeContext`. This catches many issues at compile time.
2. **Dev-build runtime check:** `assert_no_alloc` wraps the render closure. If anything allocates during render, the test process aborts. Tests run with this engaged.
3. **Release-build runtime check:** the same wrapper exists in release builds, but on alloc it sets a flag and emits a one-shot log entry post-render rather than aborting. This protects production users while making dev-time violations loud.
4. **CI failure on any RT alloc:** any test that triggers an RT-thread alloc fails the build. No exceptions, no `#[allow]`-style escape hatches.

### 2.2.4 UI testing

SwiftUI views are tested via:

- **Snapshot tests** (`swift-snapshot-testing` library) — every PR that changes a view must include updated snapshots. Reviewer visually confirms the diff.
- **Logic-layer tests** — view models / observable state are pure Swift code, fully unit-tested.
- **No UI flow is untested** — accept lower coverage on raw view code, but the state machines that drive views are fully tested.

**Phase-A status (current):** the snapshot-testing infrastructure
does not exist yet. The recent UI-bug cluster (footer progress
pill, multi-select context menu label, BPM color when locked,
search-field focus dismiss, click-delay on row select) would all
have been caught by a small suite around three views. Tracked
in `docs/UI-BACKLOG.md` C-31 ("Swift-UI snapshot tests are PRD-
mandated but don't exist yet") with concrete first-cut scope
(LibraryView footer, library-row context menu, DeckHeader). The
PRD policy stays in force; the backlog item carries the work.

### 2.2.5 Fuzzing parsers — special priority

This is the highest-leverage investment for our use case. Imagine: DJ at a gig, imports a friend's library on a USB stick mid-set, file is subtly corrupted. **We must not crash.**

- Every parser (`dub-library/src/serato.rs`, `traktor.rs`, `rekordbox.rs`, `itunes.rs`, ID3 readers, audio frame parsers) has a dedicated fuzz target.
- Fuzz corpus seeded with real-world examples and known-malformed samples.
- Run continuously on a dedicated machine or CI nightly job for ≥ 30 minutes per parser.
- Any crash discovered = blocking bug, fix before any further feature work.

### 2.2.6 Reliability SLOs (Phase C — Stable releases only)

These gates apply to **stable** releases only (v1.0 stable and beyond). They do not apply to Phase A (pre-alpha development) or Phase B (alpha cuts). See §2.2.0 for the staging rationale.

Before any **stable** release, all of the following hold:

1. **Zero crashes** in the last 100 cumulative hours of beta-tester gig-time.
2. **Zero xruns** in a 60-minute soak test at 64-sample buffer on the reference rig (M2 Air + SL3 or Audio 6).
3. **Zero RT-thread allocations** detected in soak test.
4. **Zero parser fuzz-discovered crashes** in the last 7 days of fuzzing.
5. **No regression** in latency or CPU benchmarks vs. previous stable.
6. **Manual rig checklist** signed off by at least one human on real hardware (see §2.2.10).
7. **All CI tests green on `main`** for ≥ 24 h before tag.

**Phase A and Phase B equivalents** (much weaker, intentionally):

- Phase A: CI green to merge. No release exists; `main` is the rolling target.
- Phase B (alpha cuts): manual rig checklist signed off + soak test green. No gig-hour requirement (alpha *generates* gig-hours).

### 2.2.7 Production observability (without telemetry-creep)

DJs hate phoning home. We respect that.

- **Local crash dumps**: stored in `~/Library/Logs/Dub/crashes/` automatically. Never uploaded automatically.
- **Local verbose log**: `~/Library/Logs/Dub/session.log` with a configurable retention. Includes audio-engine events (xruns, source mode changes, FX engagements, errors) but no PII.
- **Optional opt-in crash reporting** (Sentry or similar): off by default. Explicit toggle in preferences. When enabled, redacts file paths and library content.
- **Performance Mode** (preference): when enabled, Dub disables its own non-essential background work *and* asks the OS to enable Do Not Disturb (via the macOS Focus API). Mid-set notifications are disabled; Spotlight scope can be reduced via a one-click button (best effort).

### 2.2.8 Release process (staged)

Mapped to the rigor phases in §2.2.0:

| Stage | Phase | Channel | Audience | Gates |
|---|---|---|---|---|
| **Internal** | A | author's machine | author only | CI green |
| **Dev** | A | optional `dev` GitHub Releases channel | author + ad-hoc collaborators | CI green |
| **Alpha** | B | private GitHub Releases | ~3–5 invited DJs running on real gigs | CI green + soak nightly + manual rig checklist (§2.2.10) signed off |
| **Beta** | C | public opt-in GitHub Releases (marked beta) | community | All Phase B gates + feature freeze + accumulating gig-hours toward §2.2.6 |
| **Stable** | C | public GitHub Releases | everyone | All §2.2.6 SLOs met |

**Hotfix discipline (Phase B onward):** any crash bug reported against alpha, beta, or stable triggers a hotfix branch within **24 hours**. No exceptions. We may temporarily yank an unstable release rather than let it linger broken. **Phase A has no hotfix obligation** — there is no release to fix.

### 2.2.9 What this is NOT

Honest about the limits:

- **Not a "zero bugs ever" promise.** That's impossible. We promise: zero **show-stopping** bugs in stable releases (crash, freeze, audio dropout > 1 second, library corruption, data loss).
- **Not 100 % test coverage.** Coverage is a proxy; the real goal is meaningful tests for meaningful behavior. UI rendering code can hover at 30–40 % coverage without concern as long as the state machines underneath are fully tested.
- **Not a substitute for real-world testing.** CI tests prove the code works *in the simulator*. The manual rig checklist (§2.2.8) and gig-time soak (§2.2.6) prove it works in the world.

### 2.2.10 Items in the Manual Rig Checklist

Every release runs through this on real hardware. All must pass.

1. Cold launch with no library configured → first-run experience appears, no crash.
2. Import a 50k-track Serato library → completes within 60 s, no crash, no missing metadata.
3. Import a Traktor library, a rekordbox library, an iTunes XML, all sequentially → no conflicts.
4. Plug in SL3, route control vinyl to inputs 1/2 + 3/4, both decks under timecode → no calibration glitches.
5. Same with Audio 6 + Traktor MK2 vinyl.
6. Play a track for 30 minutes with timecode active, key lock on, occasional scratching → zero xruns, no audible glitches.
7. Engage Echo-Out 50 times in a row → no degradation, tail decays cleanly each time.
8. Switch a deck to Thru, drop a real record → audio passes through the engine (~2.7 ms one-way), waveform builds live, auto-BPM locks.
9. On the same Thru deck, engage Echo-Out → FX layers on top of the dry record; disengage → echo tail decays naturally over the next bar.
10. Auto-BPM on Thru locks within 15 s on a 4/4 hip-hop record, a reggae record, and a dnb record.
11. Unplug the audio interface mid-playback → engine stops cleanly, UI shows "interface lost", reconnect → playback resumes.
12. Run 60 minutes continuous use (any combination of features), close the app cleanly → no crash, no orphan processes.
13. Open the macOS sleep/wake cycle while Dub is running → audio resumes without artifacts on wake.
14. Run with a deliberately corrupted Serato library file → graceful error, app does not crash.

---

## 3. Platforms & roadmap

| Version | Platforms | Headline additions |
|---------|-----------|-------------------|
| **v1.0** | macOS (Apple Silicon + Intel) | Timecode vinyl, 2-deck, sampler, smart FX, library import, Stillpoint beat-match aid, Track Preparation Mode (shell) |
| **v1.x** | macOS | Polishing, controller/mapping support if requested by community, Track Preparation Mode prep tooling (beatgrid editor, hot cues if pulled forward) |
| **v2.0** | macOS + Windows | **Phase support**, hot cues, recording, Windows port (ASIO/WASAPI) |
| **v3.0** | macOS + Windows + iPadOS | iOS/iPadOS port (USB-C iPads), cloud library sync |

**v1 is macOS only.** No iOS, no Windows, no Phase. Time-to-first-release is the constraint.

### 3.1 Runtime modes

Dub has **two top-level runtime modes**, auto-selected at launch based on which audio interface is present. The user can override in Preferences.

| Mode | Triggered when | UI | Purpose |
|---|---|---|---|
| **Performance Mode** | A pro audio interface (≥ 4 in / 4 out) is detected | Two decks side-by-side, **vertical waveforms** scrolling bottom→top (PRD §9), Stillpoint in the centre gutter, FX bar, library | The live-DJ surface. The whole rest of this document, unless stated otherwise, describes Performance Mode. |
| **Track Preparation Mode** | Only the built-in soundcard is detected (no multi-channel interface) | Single deck, **horizontal** waveform full-width, library prominent | Auditioning tracks, fixing beatgrids, prepping cues — work the DJ does in advance of a gig, on the couch with no rig attached. **v1.0 ships the shell only** (load + play + horizontal waveform); the actual prep tooling (beatgrid editor, hot-cue prep) is v1.x — see [§12 M10.8 row](#12-milestones) and [SHIPPED §M10.8](../history/SHIPPED.md). |

Both modes share the same engine, the same library, the same file format support, and the same tokens / colour palette — they differ only in the surface they present. Switching modes is a window-level re-mount (not an in-place reflow); the user perceives them as "two apps in one binary" rather than as a layout switch. This is intentional — neither mode should leak vocabulary into the other.

**Future polish guardrail:** keep Prep and Performance as parameterized variants of the same deck surface, not parallel implementations. If the mode-specific branches grow past simple layout and policy choices, extract a small mode configuration object (deck count, waveform orientation, overview placement, header time style, load policy, transport policy) and keep `LibraryView`, `WaveformView`, `TrackOverviewView`, `DeckHeader`, `DeckState`, and the `DubEngine` load / seek / poll paths shared. Prep may change zoom, orientation, placement, and disabled affordances; it must not reimplement library import, waveform rendering, overview seek, deck headers, or track loading.

---

## 4. Audio architecture & performance targets

### 4.1 Performance targets (hard requirements)

| Metric | Target | Test rig |
|---|---|---|
| Round-trip latency | **< 5 ms** at 48 kHz / 64-sample buffer | M-series Mac, class-compliant USB interface (Rane TWELVE / NI Audio 10) |
| Round-trip latency | **< 8 ms** at 48 kHz / 128-sample buffer | Same |
| xrun rate | **0** in 60-min scratch session, 64-sample buffer | M2 Air with browser/email open |
| Timecode-to-audio response | **< 10 ms** total (input → DSP → output) | Same |
| CPU @ idle (1 deck playing, no FX) | **< 5 %** of one P-core | M2 Air |
| CPU @ stress (2 decks, key lock, echo-out, sampler firing) | **< 25 %** of one P-core | M2 Air |
| Cold start to ready-to-play | **< 2 s** | 50k-track library |

### 4.2 Audio engine principles

- **Internal sample format:** 32-bit float, interleaved or planar per stage.
- **Internal sample rate:** track device native; never silently resample. Resample at file→engine boundary only when track SR ≠ device SR (using `rubato` SincFixedOut). Note that **bitrate (e.g. MP3 320 kbps) is unrelated to sample rate** — bitrate is the compressed file's bandwidth; sample rate is in the file's PCM header. Most DJ MP3s are 44.1 kHz (CD-ancestry); a growing minority are 48 kHz (DAW/streaming exports). The engine doesn't pick a sample rate; it follows the device.
- **Sample-rate UI policy (deferred to v1.x):** We let the user run any device rate the OS allows in v1, including 96 kHz and 192 kHz. **Open question for the audio settings UI**: should we soft-warn or hide rates above 96 kHz? At 192 kHz the engine does 4× the work for no audible benefit on music playback, and several mid-range DACs exhibit IM distortion above 96 kHz. Defer the decision until we have a settings UI; this note is the reminder.
- **Audio thread is sacred.** No allocations, no locks, no syscalls, no logging, no file I/O, no Mutex, no Box, no Vec growth. Enforced by:
  - `assert_no_alloc` crate in dev/test
  - Custom allocator that aborts on RT-thread alloc in CI builds
  - A `RealtimeContext` lifetime token type that gates which APIs are callable in the render callback
- **Pre-allocation everywhere:** all buffers (file ring, decoder scratch, FX scratch, sampler voices) sized at session start.
- **Lock-free communication:** UI ↔ audio uses SPSC ring buffers (`ringbuf`) for command/event passing and atomic snapshots for read-only state (transport position, peak meters).
- **Sample-accurate scheduling:** all transport changes timestamped in samples; no millisecond rounding.

### 4.3 Audio I/O strategy

- **v1:** CoreAudio HAL via `coreaudio-rs`, not `cpal`. We need direct device-property listeners and hog mode opt-in for ultra-low-latency mode.
- **AVAudioEngine** is **not** used (too high level, hides the IO proc).
- We will support:
  - Default output device (built-in)
  - External multi-channel USB interfaces (the primary case)
- **Aggregate devices not officially supported in v1.** They'll work if the OS configures them, but we don't test against them and we don't expose UI for assembling them. Real DJs plug one interface in.
- We require the user to assign **per-deck output pairs** in External Mixer mode (see §5.3).

### 4.4 Track loading & in-memory audio

> **All audio for loaded tracks lives in RAM.** No per-block disk streaming. This is the simplest design that supports the full bidirectional, sample-accurate, scratchable, rewindable, instant-seek behavior our target users demand.

- File formats supported: MP3, WAV, AIFF, FLAC, ALAC, M4A (AAC).
- Decoders: `symphonia` for everything.
- **On load**: decode the entire track into a `Arc<[f32]>` (32-bit float, planar stereo). A 6-minute FLAC at 48 kHz stereo = ~140 MB at f32 (or we can store as f32 throughout the engine and accept the size). Two loaded decks = ~280 MB. Acceptable on any modern Mac.
- **Rationale**: scratch DJs and reggae/dnb DJs perform manual rewinds (DJ holds the pitch slider full negative, sometimes spinning the platter back by hand at high speed for 30+ seconds — common gesture for the "rewind!" moment in dnb and dub). Disk streaming with bidirectional ring buffers can keep up *most* of the time but introduces edge cases where the worker thread can't refill fast enough on a large backwards seek. In-memory eliminates the entire class of problem and makes forward and backward playback **fundamentally identical** at the engine level.
- **Forward and backward playback are byte-for-byte symmetric.** The audio engine reads a `f32` slice with a sample-accurate floating-point playhead and direction-agnostic resampler. There is no "forward path" and "backward path" — there is only "read sample at offset X with rate R," where R can be any real number including negative.
- **Memory budget**: a hard ceiling of 1 GB total audio cache across decks + sampler. Tracks loaded but not on a deck are LRU-evicted. We never load 96 kHz/24-bit material at full resolution if it would breach the budget — we downsample at load to engine SR.
- **Pre-render waveform during load**: as the decoder fills the buffer, we compute multi-resolution peak data for the waveform display. Track is "ready to play" before decode finishes (we fade in playback availability when 5 seconds of head are decoded), but full bidirectional access requires full decode (≤ 1 s for typical track on Apple Silicon).
- **Sampler / Quick Scratch slot audio** loaded the same way, persistently held in RAM (samples are small).

---

## 5. Input & control architecture

### 5.1 Per-deck source modes

Each deck has a **source** that drives its audio output. The user selects it **explicitly** via a three-way switch on the deck header (§5.1.1). There is no automatic source detection in the current build — see the deferred note below for why.

| Mode | Behavior | v1 |
|---|---|---|
| **Internal** | The loaded file plays on its own clock. **The INT switch position is the play control — there is no separate Play button.** Selecting Internal *from* Timecode keeps the platter's last pitch so the music continues without a stall or jump; *from* Thru (or a fresh deck) it plays at unity. | **Yes** |
| **Timecode** | Position driven by Serato or Traktor control vinyl. File from library is the audio source. **Relative mode only in v1** — needle drop is ignored, only motion is tracked. When the carrier stops, the deck **holds its position** — it never auto-falls-back to internal play. | **Yes — primary v1 mode** |
| **Thru** | Audio interface input on this deck's input pair routed *through* the engine to its output pair. **Always software-path**, never hardware-bypass: BPM detection, waveform capture, and FX all need the signal in software. One buffer of round-trip latency (~2.7 ms at 64-frame buffer / 48 kHz) — constant regardless of FX state. Hardware-Thru on the interface (SL3 / TA6 physical button) is outside Dub's scope because it routes audio around the software entirely. | **Yes** |
| **Phase** | Position driven by Phase wireless (file from library is audio source). | v2 |

**Implementation note:** all three modes share a single per-deck input consumer. The timecode decoder is always attached and always drains the deck's input ring; the selected mode decides what happens to those samples each block — Timecode feeds them to the LFSR decoder (drives the file), Thru passes them straight to the output (the live record), Internal ignores them (the file plays on its own clock). One consumer, no second ring, switch is instant and click-free.

#### 5.1.1 Explicit source selection (three-way switch)

The deck header carries a three-position switch — **INT · TC · THRU** — that the user clicks (mouse, pre-alpha) to choose the deck's source. Exactly one is active per deck. The switch is sticky: a mode only changes when the operator selects another, never on its own.

The switch is the transport — there is **no separate Play/Pause button** on the performance surface. Each position fully determines what the deck does:

- **INT** — the loaded file plays internally. Selecting it *is* pressing play.
- **TC** — the deck follows the platter: it plays when the control record moves and holds when it stops. Nothing to press.
- **THRU** — the live record passes through.

Transitions are designed for continuity:

- **Timecode → Internal**: internal play starts (or continues) at the platter's last pitch — no stall, no jump.
- **Internal → Timecode**: the platter takes over on the next block; the only speed change is the genuine difference between the internal pitch and the platter's actual speed.
- **Thru → Internal**: the passthrough stops and the loaded file plays internally at unity. An empty deck simply plays silence until a file is loaded.
- **→ Thru**: the live record owns the output; the loaded file (if any) stops advancing.

The small ↻ next to the TC position recalibrates the needle (channel-whitening capture).

> **Deferred — automatic source detection.** Earlier drafts of this spec had Dub *detect* the source and switch modes on its own (the algorithm below). During pre-alpha dogfooding this proved to be the wrong default: a stopped timecode record was misread as a "real record" and yanked the deck into internal playback mid-set, and the hidden state machine made the deck's behavior hard to predict. The explicit switch above replaces it. The classifier still runs off the audio thread for **telemetry and auto-calibration only** — it informs the status dot and triggers needle calibration, but it never moves the switch. Automatic detection may return later as an opt-in once it can reliably tell a stopped control record from a real one (the absolute LFSR decoding it needs shipped in M6 — see §5.4 — so the remaining work is the detection policy itself). The algorithm is retained here for that future work.

**Algorithm (deferred — not wired to mode switching):**

A short-window classifier (running on a worker thread, NOT the audio thread) examines a 250 ms sliding window of the deck's input audio:

- **Spectral test**: timecode signal has a dominant tone at 1 kHz with harmonics at 2 kHz (Serato CV02) or 2 kHz fundamental (Traktor). Compute `power_at_1k / total_power` and `power_at_2k / total_power`. Timecode: ratio > 0.6. Music: ratio < 0.1.
- **LFSR/phase test**: timecode has a deterministic phase relationship between L and R channels (the LFSR-modulated absolute position). We attempt to lock onto the LFSR. Lock acquired = high confidence timecode. Lock failed for 500 ms = high confidence non-timecode.
- **Silence test**: input below noise floor (-60 dBFS RMS for 250 ms) → "no signal."

**State machine per deck:**

```
              ┌──────────────────────────────────────┐
              │             SILENT                   │ ← all decks start here
              │  (no input above noise floor)        │
              └──────────────────────────────────────┘
                  │ signal detected
                  ▼
              ┌──────────────────────────────────────┐
              │           DETECTING                  │
              │  (250–500 ms classification window)  │
              └──────────────────────────────────────┘
                  │ timecode      │ music
                  ▼               ▼
              ┌─────────┐     ┌─────────────────┐
              │TIMECODE │     │      THRU       │
              └─────────┘     └─────────────────┘
                              (FX modules engage/disengage
                               inside the chain; no
                               source-mode change)
```

**Switch rules:**

- Music → Timecode requires 500 ms of clean LFSR lock.
- Timecode → Music requires 500 ms of LFSR lock failure AND clear non-timecode spectral signature.
- During active scratching (timecode lock plus motion), **detection is frozen** — we never switch out of Timecode mid-scratch. Even if signal degrades briefly (dust, rough handling), Stickiness (§5.4) holds the mode.
- Silence (needle lifted) does not trigger a switch. Mode is held until next signal arrives, then re-evaluated.

**User-facing behavior (if auto-detection returns):** the detected mode would be shown in the deck's status indicator and animate in/out. In the current build this is replaced by the explicit INT · TC · THRU switch (above); the status dot still reflects the *classification* (silence / timecode / record) as a health cue but does not drive the switch.

**Confidence and edge cases:**

- Very low-volume music (e.g., a quiet intro under -40 dBFS) might fail spectral classification. Behavior: stay in last-known mode. Won't engage Thru-mode FX until the music is loud enough to classify, but won't accidentally trigger Timecode either.
- A record cut with a 1 kHz tone in the music (rare but exists, e.g., test tones, some experimental records) might false-positive timecode. Mitigation: LFSR lock is the second gate; pure 1 kHz won't lock the LFSR. Both gates together have very low false-positive rate.
- Manual override always available in preferences for users who want explicit control.

**Implementation note:** the classifier runs **off the audio thread** (worker thread, ~250 ms cadence). It informs a "desired mode" for the audio thread to honor on the next block boundary, with crossfade. RT-thread is never blocked by classification.

**Source selection:** the per-deck INT · TC · THRU switch on the deck header is the source control (§5.1.1). There is no `Auto` position in the current build; if automatic detection returns it will be an opt-in alongside the explicit positions, not the default.

### 5.2 Thru Mode (real records through the software)

Thru Mode is a **headline feature** of v1 and a key differentiator. The user plays a real (non-timecode) record on their turntable, the audio flows through the audio interface into Dub, and Dub treats it like any other source: BPM tracking, waveform capture, and FX all apply.

#### 5.2.1 Routing

- The user's real turntable plugs into the audio interface's input pair (e.g. SL3 inputs A or B, Audio 6 inputs 1/2 or 3/4).
- **Signal path:** input pair → engine bus → (FX chain, when modules are engaged) → output pair. The engine treats the live audio identically to file-decoded audio for everything downstream of the source. Constant one-buffer round-trip latency (~2.7 ms at 64 frames / 48 kHz) — independent of FX state.
- **No mode flip on FX engage/disengage.** FX modules sit inside the per-deck signal chain and own their own bypass semantics (each with its own per-module declick on engage/disengage of the FX's *wet* output; the *dry* path through the Thru source is never paused, never crossfaded, never re-timed). This gives the DJ a hardware-pedal hand-feel: the dry record is always present underneath, FX layer on top.

#### 5.2.2 Why we don't expose a "hardware Thru" mode

Some audio interfaces (SL3, TA6, …) ship a physical Thru button that routes the preamp output directly to the analog output, bypassing USB and the host entirely — zero latency, no software involvement. We do not integrate with that switch, and we do not try to expose a "Direct" software equivalent (engine-silent + driver-level monitoring), because both approaches are **incompatible with what Thru Mode is for in Dub**:

- BPM detection (§5.2.3) needs the signal in software.
- Live waveform capture (§5.2.4) needs the signal in software.
- FX (M15+) needs the signal in software.

If a DJ wants hardware-Thru zero latency on a specific record, they can press the interface's physical Thru button. Dub will see no input on that pair for the duration; the waveform stops growing, BPM goes "searching", FX have no signal to operate on. That's the correct behaviour given the trade-off the operator just made — it's not Dub's job to mirror it. The earlier design that had Dub auto-flip between "engine silent" and "engine processing" based on FX engagement is removed: the silent state was producing actual silence in practice (no host-side hw-monitor control exists on plain CoreAudio), and the path-swap latency-jitter between modes was exactly the timing instability the rest of the engine is built to avoid (latency that *changes* under the operator's hands is worse than latency that's constant and slightly higher).

#### 5.2.3 Auto-BPM on live audio

A Thru deck runs continuous tempo tracking on its input:

- Algorithm: pure-Rust **log-band spectral-flux ODF** (8 log-spaced bands from 30 Hz – 16 kHz, each summed equally into the final ODF) + **windowed local-energy autocorrelation** (5-bin sum at each integer-lag candidate) with **harmonic-mean** scoring over the first 4 multiples, **perceptual tempo prior** (M11c.3a; piecewise-linear weight peaking at 90–178 BPM that biases the picker toward the "mixable band"), **reggae skank double-time rejection** (M11c.3c; hard-rejects 118–160 BPM skank peaks when a credible one-drop sibling exists at 2×), **genre-aware octave profiles** (M11c.3d–f; library analysis maps ID3 genre → `OctaveProfile` for roots/dub/dancehall/FourOnFloor disambiguation; Thru streaming stays on `Default`), **hip-hop double-time rejection** (M11c.3e; rap corpus 19/19 on Default), plus centroid sub-bin refinement. Shipped in the `dub-bpm` crate; algorithm walk-through in [`docs/SHIPPED.md`](../history/SHIPPED.md) (M7.5 baseline), [`docs/SHIPPED.md`](../history/SHIPPED.md) (M8.1 multi-band + windowed-energy overhaul), [`docs/SHIPPED.md`](../history/SHIPPED.md) (perceptual prior), [`docs/SHIPPED.md`](../history/SHIPPED.md) (reggae skank rejection), [`docs/SHIPPED.md`](../history/SHIPPED.md) (genre profiles), [`docs/SHIPPED.md`](../history/SHIPPED.md) (rap 2× fix), and [`docs/SHIPPED.md`](../history/SHIPPED.md) (FourOnFloor + Dub mid-band). **Manual override:** M11c.3b tap-to-grid (`G` re-anchor at playhead, `Shift+G` halve, `Option+G` double) writes `track_beatgrids(source='user_tap')` and hot-swaps the deck grid via `DubEngine::install_beat_grid` — see [`docs/SHIPPED.md`](../history/SHIPPED.md). Dogfooded real-music corpora (reggae 24/24, rap 19/19, house 16/16 + 1 known fail, dubstep 13/13 + 1 known fail, DnB 12/12 + 8 known fails on Default) gate octave regressions via env-gated `real_music_corpus`. Residual ambiguous cases (slow soul at 65 vs 130, sparse dubstep, DnB half-time/triplet) are expected to land on tap-to-grid rather than further picker tuning in v1. An `aubio-rs` backend remains a future opt-in feature flag. The LGPL FFI is not on any near-term critical path. The file-side fallback in §8.3 and the streaming wrapper below both build on the same `BpmEstimator` core. The `BpmRange` escape hatch (`--bpm-range MIN,MAX` on `dub thru`, `analyze_bpm_with_range` for the offline driver) constrains search when the user knows the window.
- Streaming wrapper (M8, shipped): `dub_bpm::BpmStream` spawns a non-RT analysis thread per Thru deck. The audio thread runs only a mono-downmix + SPSC push into a tee ring (alloc-free; verified by `assert_no_alloc` on the `render_into` path). The analysis thread reads from that ring, runs `BpmTracker` (= `BpmEstimator` + `ConfidenceTracker` hysteresis state machine + ~1 s throttle on the expensive autocorrelation search), and emits `TrackerEvent::StateChanged` transitions through a second SPSC ring the UI polls. The Thru `ThruSource` itself stays a pure passthrough (PRD §5.2.1). Engine ↔ tracker glue lives in `EngineHandle::attach_thru_source_with_bpm_tracking`. Full design + tuning rationale in [`docs/SHIPPED.md`](../history/SHIPPED.md). The streaming-stability regressions M8 exposed at 128 BPM / 44.1 kHz and 140 BPM / 48 kHz are part of what motivated the M8.1 algorithm overhaul.
- Stabilization: ~10–15 s of music for a confident reading.
- UI: BPM display with a **confidence indicator** (3-state: searching / tentative / locked). Tentative readings shown italicized.
- The detected BPM feeds the FX (echo-out divisions, loop length references) so the user can apply tempo-synced FX to a real record.

#### 5.2.4 Live waveform capture (v1)

While a Thru deck is active, the engine accumulates a **peak waveform** of the input audio in real time, rendered live as the record plays. When the record finishes (or the user disengages Thru), the captured waveform is held in memory and optionally persisted (see §5.2.5).

**Shipped in M9** as the `dub-peaks` crate, the data-layer companion to M8's `dub-bpm`:

- **Audio thread** — the same mono-downmix scratch that feeds the M8 BPM tee also feeds a second `peaks_tap` SPSC ringbuf via one extra `push_slice`. Alloc-free; the ThruSource computes the downmix once and dispatches to whichever taps are enabled.
- **Off-RT decimator thread** — drains the peaks tap at 20 ms cadence into a `Decimator(samples_per_chunk=64)` that emits a `PeakChunk { min, max, rms }` per 64 input samples (≈ 1.33 ms / chunk at 48 kHz). Same architectural shape and lifecycle as `BpmStream`.
- **Shared buffer** — `PeakBuffer { len: AtomicUsize, chunks: RwLock<Vec<PeakChunk>> }`. The renderer's "anything new?" check is a lock-free Acquire-load on `len`; only when there's new data does it take a brief read lock to `extend_chunks(start_idx, &mut local_mirror)`. O(new chunks) per 60 fps frame, not O(total). Initial capacity defaults to 10 min of audio; growth beyond that reallocates off-RT (the audio thread never reallocates).
- **`PeakChunk` is `#[repr(C)]`** (12 bytes) — the M10 consumer contract is to take a `&[PeakChunk]` from `extend_chunks` and shove it directly into a Metal vertex buffer with no further packing.
- **Multi-resolution** — the M9 crate ships a single base mip level. Mip-up for the overview view (~67k samples/pixel at 90 min on a 4K screen) is one averaging pass in the renderer; the crate deliberately doesn't pre-build a mip pyramid because the renderer knows how many pixels it has and can downsample on demand.
- **CLI surface** — `dub thru` defaults to peaks-tracking on; the periodic stats line shows per-deck captured chunk counts. `--no-peaks-track` opts out; `--dump-peaks PATH` writes the captured buffer to a CSV file on shutdown (one row per chunk: `deck,chunk_idx,min,max,rms`) so the operator can plot the envelope before the M10 UI lands.

The architectural pre-commitment in §4.1 ("**Live waveform engine** — runs alongside Thru, accumulates multi-resolution peak data, rendered by Metal") is concretized exactly: `dub-peaks` is what M10's Metal renderer consumes. The M10 consumer contract is documented in `crates/dub-peaks/src/lib.rs` module docs.

See [`docs/SHIPPED.md`](../history/SHIPPED.md) for the full design history and test surface.

#### 5.2.5 Audio fingerprint recognition (v1.1 — *not v1.0*)

The differentiating feature, planned for v1.1.

- As a Thru deck plays, the engine continuously computes a rolling **audio fingerprint** over a 5-second window. Algorithm: Chromaprint (algorithm 2), via the pure-Rust `rusty-chromaprint` crate. M11b chose pure-Rust over the LGPL-2.1 C library for the reasons documented in §10.2 (license isolation, no C build dep, no unsafe FFI surface).
- Fingerprints are matched against a local database of records the user has played before.
- **First play** of a record: no match. Engine creates a new entry, captures the waveform, captures the auto-BPM, captures the fingerprint, persists everything to `library.sqlite` keyed by fingerprint hash. The user can optionally tag the entry with title/artist (or it stays anonymous).
- **Subsequent plays**: fingerprint matches within 5–10 s of needle-drop. Engine loads the saved waveform, beatgrid, and BPM. UI animates: waveform "fades in" as recognition completes, BPM stops searching and locks, beatgrid overlays appear. Effects that need beat-sync become available.
- **Robustness considerations**: pitch variation (turntable ±8 %), surface noise, mixer EQ, room sound. Chromaprint is designed for exactly this and handles ±10 % pitch reliably. Our fingerprint hashing is pitch-tolerant (use the Shazam-style constellation approach over Chromaprint's chroma if Chromaprint proves insufficient). Implementation: `rusty-chromaprint` (pure-Rust, MIT/Apache) since M11b — see §10.2.

**v1.0 ships:** Thru routing (always software-path, FX in-chain), auto-BPM, **live waveform capture and rendering** (in-memory only — not persisted, no recognition). v1.0 already shows the user "this record I'm playing has BPM 92 and here's its waveform as it plays." That alone is unique.

**v1.1 adds:** persistence, fingerprinting, recognition, beatgrid storage. This is the magic.

#### 5.2.6 Constraints

- Thru Mode adds **one buffer of round-trip latency** (input + output), e.g. ~2.7 ms at 64 samples / 48 kHz. This is unavoidable physics. It is *constant* with respect to FX state (engaging FX does not change the input-to-output delay; FX modules add to the dry path, they don't replace it), so the DJ's hand→ear muscle memory stays calibrated across the whole set.
- The user must drop the needle near the start of a record for waveform capture to be meaningful. We do not "stitch" partial captures across plays in v1; that's a v1.x consideration.
- Auto-BPM cannot detect tempo on solo a-cappella or beat-less ambient sections. UI must communicate "no beat detected" honestly, not lie with a fake number.

### 5.3 External mixer mode (only mode in v1)

- **Required:** audio interface with ≥ 4 outputs (2 stereo pairs) AND ≥ 4 inputs if Thru mode is used.
- Per-deck output assignment: Deck A → Out 1/2, Deck B → Out 3/4 (configurable).
- Per-deck input assignment (for Thru and timecode): Deck A → In 1/2, Deck B → In 3/4 (configurable).
- **No software cue/preview channel.** Cueing is the hardware mixer's job.
- **No software crossfader.** External mixer's crossfader is the only crossfader.
- **No software EQ.** External mixer's EQ is the only EQ.
- **No mouse-driven mixing or performance gestures.** Per the §1 mouse rule — mouse never drives pitch / scratch / crossfade / EQ / gain / cue. Mouse-driven transport (Panic Play, Casual Play, position navigation) is allowed and lives in §6.1.
- **Smart FX (echo-out, dub siren) are inserted into the per-deck output bus pre-output.**

The internal debug mixer (§5.6) is **not** the same as a user-facing internal mixer mode. v1 ships only External Mixer mode in the UI; the debug mixer is dev-only.

### 5.4 Timecode subsystem

**Supported control records:**
- Serato Control Vinyl CV02 (1 kHz reference + LFSR position)
- Traktor MK2 Timecode

**Both supported in v1.** User selects which in preferences; auto-detect attempted on input.

**Required behaviors:**
- 33⅓ and 45 RPM detection (auto, with a manual Preferences override — see §5.4.1).
- **Relative behavior only in v1.** Needle drops are ignored; only motion is tracked. Absolute *positioning* (needle-drop seeks the track) is deferred — almost no scratch DJ uses it in practice, and skipping it cuts calibration UI complexity significantly. **However, since M6 the decoder does decode the absolute LFSR position internally** (Serato CV02; Traktor MK1 planned, MK2 has no public bitstream and stays carrier-phase-only): while the bitstream is locked, the deck's velocity and playhead advance come from per-block *deltas* of the decoded groove position — bit-exact pitch (a steady +8.0 reads +8.0, like Traktor) and a playhead that cannot drift from the groove ("sticker drift"). A lift + re-drop re-anchors the reference and never jumps the playhead; during acquisition (~20 ms) and degraded signal the carrier-phase relative path drives the deck exactly as before. No user-facing mode, no calibration UI added.
- **Pitch range** as wide as the user's turntable (typically ±8 / ±16 / ±50 %)
- Slow-down to stop: tracks pitch through zero cleanly without click/glitch
- Backspin: tracks negative pitch with no audible artifact up to the resampler's limits
- **Drop-out detection (Stickiness)**: if signal quality degrades (dust, scratch, end of run-out groove), hold the last known velocity for a grace window (default 250 ms). If the signal does not recover within the window, the engine **pauses the deck on the held position**. The grace window discriminates brief stylus hiccups (held, no state change) from sustained dropouts (pause + hold). *(Auto-continue-forward on run-out — "Repeat" — is deferred and will be scoped to the detected lead-out only; see §5.4.2.)*
- **Through groove handling**: when the needle reaches the end of the encoded area, Stickiness's grace window expires and the deck pauses on the held position. The operator presses internal Play (Panic Play, §6.1.2) to continue forward; recovery to timecode is automatic on the next clean Locked sample (DJ drops the needle back on a mid-timecode groove). Auto-continue in the lead-out (Repeat) is deferred — see §5.4.2.
- **Calibration UI**: show signal scope, S/N ratio, RPM detection, pitch readout. Live calibration with vinyl spinning. **No A/B side toggle, no abs/rel mode selector** — relative mode is universal.
- **Session-start calibration hold (Traktor-style).** A Timecode deck does not auto-start playback until **all** of the needle's measurements complete — whitening calibration installed *and* the pitch stabilized (wobble fit settled): once the track plays, the DJ treats the deck as ready and starts pitching, so nothing may still be stabilizing underneath (calibration may take ~5–10 s of the record spinning; that's fine — the DJ sets up minutes before the first tune, and this matches Traktor's "calibrating" startup behavior). The non-negotiable companion is **feedback**: the deck header draws a left→right progress line while measuring, so the DJ sees calibration advancing and the exact moment the deck goes live (the readouts un-dim at the same moment). Stopping the record pauses the measurements — the line makes that visible too. The deck's Play button (Panic Play) bypasses the hold instantly for stage emergencies, and the hold never pauses an already-playing deck (mid-set recalibration runs live).
- **Tracking quality indicator (UI)**: a per-deck signal-health glyph in the deck header source pill (PRD §9) — green dot = clean lock, amber = degraded signal, red = no lock. Read off the M5.4.6 `LiftPolicy` confidence the engine already publishes. Note: it is **expected** for tracking to be red while cueing or scratching, identical to Serato's behaviour — the dot reports signal quality, not user intent.

**Algorithm:** port the xwax decoder (well-understood, ~2k lines C, BSD-licensed). Our port lives in `crates/dub-timecode/`. Both Serato and Traktor LFSR tables included.

**Absolute *positioning* (needle-drop) is deferred to v1.x or later** if user demand emerges. Most likely, never. This is a UI/behavior decision, not a decoder limitation — the absolute position is already decoded internally (M6, see above); we deliberately use only its deltas.

#### 5.4.1 RPM Preferences override

The auto-detect path handles 33⅓ and 45 RPM transparently in 99 % of cases. Edge cases (unusual pressings, calibration of an under-spec'd turntable) need a manual override. This lives in the Preferences sheet under "Timecode" — a per-deck `Auto / 33⅓ / 45` selector. The override is *not* a performance gesture; the DJ sets it once during sound-check and forgets it. It must not appear on the performance surface.

#### 5.4.2 Repeat (timecode run-out) — deferred, rescoped to the lead-out

The intended end-state: when the needle reaches the end of the timecode-encoded area (the lead-out / run-out groove), the deck **auto-continues** the audio track forward at the last-known velocity — audio decouples from the now-absent timecode so a track longer than the control record's encoded area keeps playing. Every commercial DVS app does this, and it's what the target user expects.

**Status — deferred.** The original M10.6e build auto-engaged this on *any* sustained dropout. That was removed: the engine cannot yet tell the lead-out apart from an ordinary needle lift (both look like the carrier going away), and auto-running-forward on a lift is wrong. So **today a sustained dropout pauses the deck on the held position** (§5.4 Stickiness); the operator presses internal Play (Panic Play, §6.1.2) to continue past the end zone.

**Path to shipping it.** Repeat returns scoped to *only the detected lead-out region*, gated on **lead-out discrimination** (UI-BACKLOG P-32). The decoder already reads absolute LFSR position internally (§5.4, since M6), but does not yet classify "needle is in the lead-out" vs "needle was lifted." Once that classification lands, the engine auto-engages Panic Play **only** in the lead-out, leaving the lift case paused. Until then run-out is a manual Panic-Play action — so there *is* now a held/paused state in Timecode mode after run-out (previously there was not).

#### 5.4.3 Groove-continuity healing (sticker lock)

Relative mode integrates decoded motion, and integration errors are permanent: every scratch turnaround or slow draw where the carrier momentarily collapses (a cartridge is a velocity sensor — slow stylus motion is inherently quiet) freezes the deck for a few ms while the record keeps moving. Accumulated over a scratch session this is **sticker drift** — the kick slides off the cue sticker (measured on-rig: ~1.5 s lost over one minute of hold-heavy scratching).

Since M6 the decoder reads the absolute LFSR groove position. That makes the drift *measurable* (the `groove − playhead` offset must stay constant for an engagement; the engine publishes its deviation as the Sticker-Drift telemetry in the deck Signal panel) — and *repairable*: **as long as the needle stayed in the groove, the record's own continuity is ground truth.** When the absolute tracker re-locks after a relative-only gap, the engine bleeds the measured drift back into the playhead as a **bounded slew** — at most ~1 % of real time per block (an inaudible rate trim), re-measured every locked block until the residual sits inside a ~4 ms deadband. Never a step: a step correction was audible as the track "jumping slightly forward" at the first re-lock after a scratch.

This is **not** needle-drop positioning (still deferred, see above): healing only ever enforces continuity of the mapping the DJ already established. The guards:

- **Needle lifts re-anchor instead of healing.** A lift is detected as sustained (≥ 1.5 s) carrier silence; an offset jump after a lift is a deliberate re-drop (re-anchor if it moved > 250 ms; a re-drop back on the sticker keeps the running measurement).
- **Deliberate playhead moves re-anchor instead of healing.** Seeks/cues, track loads, internal Play, control-mode switches, and Panic-Play exits flag the drift monitor, so the heal never fights an intentional jump.
- **Sub-4 ms drift is left alone** — steady play is never micro-nudged.
- **Travel gestures never late-correct.** A backspin/spin-forward physically skids the needle across grooves; a large offset jump after large groove travel re-anchors silently instead of "healing" (= audibly jumping) the song.
- A > 5 s offset jump with no lift detected is treated as a programmatic remap (safety fallback), not drift.
- **A held deck has no mapping.** While playback is gated (session-start calibration hold) or otherwise stopped, the playhead is pinned where the track was loaded — no groove advance, no healing. The groove↔playhead mapping is established at the moment playback engages.

#### 5.4.4 Reverse Input Control

A keyboard-only command (no UI button on the performance surface) that **swaps deck A's and deck B's timecode input pairs**. Two motivations:

1. **Booth wiring discovered late.** In a dark booth, the DJ can't always see which turntable is on which channel pair on their interface. They arrive, drop the needle, and the wrong virtual deck moves. Rather than re-cabling under stage lights, they hit one key and the mapping flips.
2. **One-turntable emergency.** A needle skips, a deck breaks, a cartridge dies mid-set. The DJ wants to continue with the surviving turntable controlling whichever virtual deck the next track is queued on. Reverse Input transfers control of the working turntable to the silent virtual deck without re-routing audio.

Implementation: the M5.4.5 late-binding-decks + per-deck input attach work already supports this; the command is a single re-attach pair on the trash channel. **Audio is never re-routed** — only the *control* (timecode → virtual deck) flips. Deck A still plays out of Out 1/2, Deck B still plays out of Out 3/4. This is critical: if we swapped audio routing too, the external mixer's channel assignments would become wrong mid-set.

Trigger surface TBD (see §5.5). Visual feedback: a brief "INPUT SWAPPED" toast in the Status Strip (PRD §9). No persistent indicator — the swap is the new default state until reversed.

#### 5.4.5 Canonical zero (detent) anchor

A DJ's pitch model is **positional**: fader at the detent → the readout should say 0. Real turntables don't run true (the reference rig's deck A measures +0.1–0.2 % at the detent), and an app that shows +0.2 % at the detent reads as broken even when that is the platter's true speed.

Dub pins **only the zero/detent anchor**: a multiplicative trim measured during the session-start calibration hold (§5.4 — the record is spinning at the detent anyway), taken as a multi-second mean, guarded to ±0.4 %, and installed while the deck is still silent. It is applied to **playback**, and the displayed rate equals the audible rate (xwax / Mixxx parity — the readout shows exactly what plays). The zero anchor is **session-scoped** (DJs travel, decks differ venue to venue, a stale trim is worse than none) and relearned silently at each session start inside the calibration hold.

**The ±8 stop anchors were removed.** Earlier revisions also pinned the ±8 % stops to canonical via a per-deck piecewise-linear warp plus an opportunistic stop-learning state machine. That warp drove the *displayed* rate off the *played* rate — two decks shown the same BPM ran at different speeds — so it was deleted (~176 LOC; see `crates/dub-engine/src/anchor.rs` and `LESSONS.md`). Only the zero anchor survives; the ±8 readout now simply shows the platter's true rate at the stops rather than a forced canonical ±8.

Keyboard is first-class for **non-performance** tasks (load, navigate, settings). Performance gestures — pitch, scratch, crossfade, EQ, gain, cue — live on the turntable and the user's external mixer, per the §1 mouse rule extended to the keyboard (the keyboard is not a substitute for a turntable).

v1.0 ships an **intentionally minimal** keymap. Performance keys (Quick Scratch, Sampler, Loops, Key Lock, Echo-Out, Zoom, etc.) get bindings *with* their feature milestone (PRD §12) — speculating an exhaustive keymap up front churns more than it helps.

**v1.0 confirmed bindings:**

| Key | Action | Milestone |
|---|---|---|
| `⌘,` | Open Preferences | M10.3 |
| `Space` | Load the **library selection** (highlighted row in the file / library browser, §8.5) into the **stopped, non-master deck**. If the non-master deck is currently playing, the deck pane flashes red with a "deck is playing — lift the needle" overlay; the user lifts the needle (or stops Casual Play) and tries again. See §6.4 Master deck. | M10.5 |

Every other key — performance and otherwise — is **TBD** and will be added to this table as its feature ships. The PRD does not commit to a binding before the feature exists, because we've learned that DJ keyboard muscle memory is heavily anchored to Serato / Traktor conventions and we want to choose deliberately, not preemptively.

User can rebind any action. Key binding profiles saved per-user.

### 5.6 HID / MIDI controllers

**Out of scope for v1.** Scratch DJs use external mixers, not controllers. The codebase will include the abstraction (`crates/dub-controller/`) so this is additive in v1.x without rework.

When controllers do land (v1.x / v2), they map to the **same** external-mixer mental model: a controller represents a turntable + its mixer channel. The controller's mixer section (faders, EQ) controls the user's *external* mixer if there is one, or — only if the user explicitly enables it — a software mixer. Software mixer is **not** a v1/v2 commitment; it's a v3 question.

### 5.7 Debug internal mixer

For testing forward/backward play, scratch sample feel, FX behavior without a turntable rig. Behind a **Debug menu** (hidden in release builds unless `--dev` flag).

Includes: per-deck play/pause/scrub-bar, master gain, channel gain, primitive crossfader, master output. **No EQ, no filter, no FX UI parity** — just enough to verify the engine.

---

## 6. Feature set — v1

### 6.1 Core transport (per deck)

- Timecode-driven play/scrub/scratch
- Slip mode (always on for timecode mode; configurable for internal)
- **Key lock** (master tempo) toggle, via Rubber Band — see §6.1.1 for scratch-aware auto-bypass
- **Pitch range** display (informational; pitch is set by the turntable, not software)
- **Track time display** — shown in the deck header. Live read of the engine's deck-rate-aware playhead, formatted `MM:SS`. **Performance / Timecode mode** shows *only* the remaining time as `-MM:SS` — the two-deck split is space-tight in the header and "how long until I have to mix" is the only number the DJ touches mid-set (PRD §1 "every screen pixel earns its keep"). **Prep / Track-Preparation mode** shows both elapsed (`MM:SS`) and remaining (`-MM:SS`) because the single-deck rehearsal surface has the real estate and elapsed time is useful for hot-cue placement. Total length is omitted in both modes — duration + the displayed value gives total trivially.
- Auto **gain trim** based on track loudness (LUFS-I or peak normalization, user choice)
- **Mouse-allowed transport surfaces** (per the §1 mouse rule and §6.1.2 / §6.1.3 below):
  - **Click-to-jump on the overview waveform** seeks the deck to the absolute track position under the cursor. Single click only — the overview is a "where am I in the whole track" map, not a fine-positioning tool, so it intentionally does not support drag-scrub. Transport is left alone: a paused deck stays paused at the new position, a playing deck keeps playing from the new position. Works in both Performance and Prep mode. In two-deck Timecode the seek lands instantly and the timecode driver re-locks on the next confident sample.
  - **Click + drag on the zoomed waveform** for **vinyl-style scratch-cueing**. The cursor's *velocity* drives the deck's playback rate every block. Drag direction per orientation: in **vertical** mode the future region of the waveform sits below the playhead (§9.6), and dragging *down* plays the deck forward at the cursor's speed — this matches the "drag toward the future" intuition that DJs already use on vertical-waveform software (Serato, rekordbox, Engine). In **horizontal** mode the convention is the mirror of the visual scroll instead: forward playback scrolls the waveform leftward through the playhead, so dragging *left* plays the deck forward, like grabbing the platter and pushing it forward. Drag faster and the deck plays sped-up; drag the opposite way and it plays in reverse; hold the cursor still and the playhead freezes silently, exactly like a stylus on a stationary record. There is no auto-play on click — pressing without moving produces silence. In Timecode mode the shell engages Panic Play for the duration of the drag so the timecode driver doesn't fight the cursor-driven rate; on release Panic Play cancels and the pre-scratch transport (play / pause) is restored. The DJ uses this surface to find the *exact* downbeat of an intro or the leading edge of a kick before passing the deck to the platter. Works in both Performance and Prep mode.
  - The §1 carve-out is mechanical: this *is* a mouse-driven scratch (continuous rate modulation), explicitly allowed for cueing because the audience never hears it — the DJ's external mixer is the only routing to FOH, and a cueing scratch lives on the cue channel / headphones. The DJ would never use the mouse to scratch during a live mix; the platter is on the deck for that. The PRD's earlier ban on mouse-DJ-as-performance still holds — what's new is the recognition that *off-air* mouse scratching is the right cueing tool for a wide-track waveform.
  - Click the deck header's **primary transport button** to drive Panic Play (Timecode mode, §6.1.2) or Play/Pause (Prep mode, §6.1.3). One button per deck — its role changes with engine mode. Keyboard equivalents in §5.5.

#### 6.1.2 Panic Play (dirty-needle recovery)

The most common stage failure for a Timecode-mode DJ is needle contamination — dust, a tiny scratch, accidental thumb on the cartridge — which interrupts the LFSR signal mid-song. Stickiness (§5.4) holds the playhead for 250 ms, but that's not enough time to clean a needle.

**Panic Play** is the user-driven extension, surfaced as the **deck-header primary transport button** in Timecode mode. The button is a Serato-style INT/ABS toggle:

- **Currently following the platter** → button shows ▶ `play.fill`. Tapping engages Panic Play (the deck disengages from timecode and runs internally at last-known velocity). The source pill flips to `TC · HOLD` with an amber dot, the button morphs to a vinyl glyph (`opticaldisc.fill`, amber tint).
- **Currently in Panic Play / internal mode** → button shows the vinyl glyph. Tapping cancels Panic Play (hands transport authority back to the timecode driver).

The transport state captures whatever rate the turntable was running just before, read from the most recent confident `LiftPolicy` velocity sample. While engaged the deck ignores `Locked` / `DropoutHoldRate` intents from the timecode driver — the cartridge can be lifted, cleaned, recalibrated and dropped back without the deck pausing.

The button also subsumes **Casual Play in Timecode mode**: when a track is loaded but neither timecode nor panic is active (the platter is silent, the needle isn't on yet), tapping ▶ engages Panic Play, which starts internal playback at unity (`PanicPlayState::normalise_held_rate` floors zero / negative held rates to `+1.0`). This is what "Play actually starts a Timecode-mode deck" maps to — bypassing the `DropoutHoldRate` arm that previously slammed `set_playing(false)` against any direct `engine.play` call.

Two ways out of Panic Play:

1. **Auto-resume.** When the engine sees a clean LFSR lock returning (carrier alive + confidence above the engage threshold; an M6 absolute-position lock counts identically — it's the strongest possible lock signal), `Engine::drive_timecode_inputs` clears the panic flag and applies the new platter rate on the same block. The held playhead position is the new "zero" reference for the LFSR's relative motion (§5.4). The audience hears no interruption beyond a tiny `LiftPolicy` crossfade.
2. **Manual cancel** (the user tapping the vinyl button). The engine clears the engaged flag and **leaves deck transport alone**; the timecode driver decides what happens on the next block:
   - Healthy carrier present → driver re-locks, deck keeps playing at the platter rate (this is the INT→ABS hand-back).
   - Carrier silent / below threshold → driver's existing `DropoutHoldRate` arm pauses the deck on the held position (this matches the pre-M10.6c "engine pauses on held position" outcome, now produced by the natural dropout path instead of a manual `set_playing(false)` that would race the next Locked sample).

Panic Play is the **single most important reliability feature** in v1 from a "career night" perspective. PRD §2 reliability commitment is fulfilled here.

#### 6.1.3 Casual Play (pre-performance file-mode playback)

Before the actual set starts — sound-check, soundcrew dinner, opening DJ playing — the DJ may want to play music through the rig without engaging timecode at all. Maybe they want to hear how their gear sounds in the room; maybe they want to play a curated mixtape so the venue isn't silent.

**Casual Play** is the file-mode transport: drag a file onto a deck (or load via `Space` from the library, §6.4), tap the deck-header transport button (▶ `play.fill` ↔ ⏸ `pause.fill`), the track plays from the start at 1.0× rate. The deck header source pill shows `FILE`. **No pitch fader is exposed** — the user explicitly accepted "no mouse-driven pitch" in §1, and pitch in File mode is not a performance gesture. The deck plays at 1.0× the entire time; if the DJ wants pitch control they need to engage timecode. Restart / jump-to-zero lives on the Track Overview strip (§9.6.1): a click at the top of the overview seeks to 0:00 — one affordance, one place. No dedicated Restart glyph.

Casual Play is **not** sync-mixable from the keyboard — there is no "load and beat-match" workflow, no auto-crossfade, no countdown. A real mix requires the turntables. This mode exists for the *pre-set* use case only, and is intentionally limited so it doesn't grow into a controller-DJ surface.

When a Casual-Play track ends, the deck simply stops. No autoplay, no next-track logic — keeping the surface area small.

#### 6.1.4 Master deck (single-master semantics)

At any moment exactly one deck is the **master**. The master is the deck whose movement is currently authoritative for the rest of the surface — keyboard-load (§5.5) targets the *non*-master, the Stillpoint aid (§9.4) is anchored to the master (which *is* the lock line), the Status Strip shows the master's BPM, and future sync/quantise logic (v1.x) snaps to the master's beat phase.

**Derivation (engine, not user-controlled):**

1. If exactly one deck is playing, that deck is the master.
2. If both decks are playing, the master is the deck whose **transport last advanced** in the most recent UI frame. In Timecode mode "advanced" means the needle moved at a non-zero rate; in File mode (Casual Play) advancement is constant while the deck is playing, so a re-`play()` or `seek()` re-promotes that deck to master. The intent matches Traktor's deck-focus convention: whichever deck the DJ just touched is the one their next keyboard action targets.
3. If neither deck is playing, master is **sticky** — whichever was master last remains master. A freshly-launched session has no master until the first play.

The master is **not** chosen by mouse or by a focus ring. There is no `Tab` to cycle, no "click deck pane to focus." The platter (or in Casual Play, the deck's own play state) is the only authoring surface. This is a deliberate continuation of the §1 mouse rule — the deck the DJ is currently performing on tells the app what's the master; the DJ shouldn't have to tell the app twice.

**UI surface**: a single small **MASTER** chip in the master deck's header (top-right of the deck header), with the deck's BPM next to it. The non-master deck shows its BPM without the chip. No flashing, no animation — just presence vs absence.

**Load-into-non-master rule (M10.5):** pressing `Space` with a library row selected loads that file into the **non-master, stopped** deck. If the non-master deck is currently playing (rare, but possible if both decks were just playing and the DJ touched the other), the deck pane flashes red for 200 ms with a "deck is playing — lift the needle" overlay. The user lifts the needle (or pauses Casual Play), and the next `Space` succeeds. We do not auto-stop the deck — silently dropping a track on a deck that's mid-air is the kind of bug-prone helpfulness §2 rejects.

**Opt-out for the load-into-playing guard (M10.5r):** Preferences exposes an `Allow loading onto a playing deck (Performance mode)` toggle, default off. When on, drops / Space-loads onto a playing deck succeed silently — the new track replaces the old one mid-play. The toggle exists for the rehearsal-style workflow where a single DJ is bouncing between two decks and wants to drop the next track without lifting the needle first. The default stays "refuse + red flash" because that matches the on-stage muscle memory of every other DVS app and prevents accidental cue-track loss. **Prep mode always allows the load** regardless of the toggle — Prep is a single-deck shell where the "currently playing in front of an audience" concern doesn't apply.

**Load never blocks playback (M10.5v).** `load_track` returns to the caller the moment the audio thread has the new `Arc<Track>` (~50 ms decode + a sub-millisecond engine-state swap). Offline peaks and `analyze_beat_grid` move to a detached `std::thread::spawn` inside `dub-ffi`, which installs its results back through `Arc<Mutex<EngineState>>` and bumps the per-deck `peak_generation_seq`. The Apple shell calls `loadTrack` from a `Task.detached`, then drops back into the 30 Hz position poll; the waveform appears within ~30 ms of swap, the deck-header BPM within ~300 ms. The user can press `Space` immediately after the load completes — playback is *never* gated on analysis. Cancellation against a back-to-back load on the same deck is by `Arc::ptr_eq`: if a newer track has displaced the in-flight `Arc<Track>` in `running.file_tracks[idx]` by the time the worker thread finishes, the stale peaks / grid are dropped on the worker's stack. **Performance budget**: a 4-minute track must reach a fully populated BPM column in ≤ 1 s on M-series silicon end-to-end; before M10.5v the `Vec::drain(..HOP_SIZE)` in `dub-spectral::SpectralFrameStream::process` made this 38 s for a clean synthetic track (O(N²) collapse on the offline path), which M10.5v fixed with a read cursor + amortised compaction (now O(N)).

#### 6.1.1 Key Lock with scratch-aware auto-bypass

Rubber Band cannot handle the rate excursions of scratching (rapid back/forward, very high `|rate|`, sub-millisecond rate changes). When Key Lock is enabled, the engine **automatically bypasses** the time-stretcher during scratching and re-engages it when the playhead settles, transparently to the user.

**Decision logic** (runs every audio block):

- Compute current playback rate `r` (samples-per-output-sample) and rate-of-change `dr/dt`.
- **Bypass** Rubber Band when ANY of:
  - `|r|` > 1.5× (scratching at speed)
  - `|dr/dt|` > threshold (rapid rate change, e.g. needle just hit)
  - `r` < 0.05 or `r` < 0 (near-stop or reverse)
- **Re-engage** Rubber Band when ALL of:
  - `|r - r_user|` < 0.1 (rate has settled near user's set tempo, where `r_user` is the turntable's current pitch slider position as inferred from timecode)
  - This condition has held for ≥ 200 ms

**Crossfade**: bypass → engaged transition uses a 20–30 ms equal-power crossfade between the resampler-only signal and the Rubber Band signal to avoid clicks. Engaged → bypass is instantaneous (drop the Rubber Band stage; resampler picks up the same input pointer).

**UI**: a "Key Lock" indicator with two states:
- **Green / on** — Rubber Band currently active (deck is playing in tempo).
- **Dim green / standby** — Rubber Band bypassed for now (user is scratching), will re-engage automatically.

User does not see or configure thresholds. It just works.

### 6.2 Looping

Looping in Dub is built for the turntablist's "rewind the bit I just played" reflex, not the controller-DJ's forward loop-roll. The model is a **reverse loop**: press a loop length and the engine grabs the passage *just heard* — it snaps the press to the nearest beat, takes that many beats backwards, and jumps the playhead into the region so the loop wraps seamlessly (`crates/dub-engine/src/looping.rs`).

- **Beat-length buttons** select the loop length. v1 ships ½ / 1 / 2 / 4 bars; a polish pass may relabel these in *beats* and add a few more levels — the change is cosmetic, the engine already loops an arbitrary beat count.
- **Halve / double** is *selecting an adjacent length button*, not a separate control: tap a shorter / longer length and the loop re-grabs at the new size.
- **Reloop / exit** — re-arm the last loop, or drop out and continue.
- **No manual Loop In / Loop Out, and no loop relocation.** Deliberately out of scope — the beat-length + reverse-grab model is the whole loop UX. Per-edge in/out points and "move the loop while it's running" are controller-DJ idioms we are not building.
- **Loops must work correctly under timecode** (acceptance §14 #8). The loop is in-engine (not driven by needle position), so under timecode the engine has to own the looped region without fighting the platter. **Current state:** looping ships as an *internal-play* feature (Prep / Panic-Play transport); timecode-correct looping is a v1 ship gate still to be met.

**Saved loop slots** (8 numbered, recallable from keyboard) — **deferred to v1.x**. The Serato workflow of "save loops to the track, recall during performance" is real but not load-bearing for v1; v1 ships ephemeral loops only. Library schema includes an empty `track_loops` table from M11 onward so v1.x can land the feature without a migration.

### 6.2.1 Hot cues (performance cues)

**Hot cues are a v1 performance feature** — fixed trigger points the DJ drops onto a track and fires live for beat-juggling, finger-drumming, and re-triggering a phrase or a drop. They live on the four CUE pads, fired from the **number-row keys (1–4)**, a small pad controller next to the turntables, or by **clicking the pad** (Shift / ⇧-click clears). A hot-cue press is a momentary trigger, not a continuous performance gesture, so the mouse is fine for it (§1); the keys and controller are there for DJs who keep their hands off the trackpad mid-set.

This is a **different feature from a CDJ-style "cue" button** (a navigation affordance: set a temp cue point, jump back to audition). A turntablist does not need that — **he cues with the needle.** Earlier PRD drafts conflated the two and wrongly deferred all "cues" to v2; hot cues (performance) are v1, the CDJ cue/preview button is not built (see §6.6).

Hot cues persist per track (`track_cues`, `source='user'`; see [`LIBRARY-SCHEMA.md`](LIBRARY-SCHEMA.md)) so they survive reloads and round-trip through library export. Available in both Performance and Prep mode.

### 6.3 Smart FX (per deck, mutually compatible)

**Echo-Out**
- Hold-to-engage button (or keyboard tap with sustain)
- Captures the last N beats of the deck's output into a delay line, freezes the deck's main signal, plays the captured loop with feedback decay.
- Parameters: divisions (1/4, 1/2, 1, 2, 4 beats), feedback (default 60 %), filter (low-pass, default 8 kHz)
- One-button workflow: tap and hold → echo-out engages; release → tail decays naturally; deck's actual playback continues where it would have been (slip-aware).

**Dub Siren**
- Classic dub-siren synth: oscillator (sine/saw/square), envelope, slap-back delay, optional spring reverb modeling
- Trigger via keyboard or on-screen button
- Pitch-bend mod wheel via mouse drag or trackpad
- Routed to a configurable output (default: Deck A output, but should support a dedicated "FX bus" output for users with mixer aux returns — **decision deferred to v1.1** unless trivial)

### 6.4 Sampler / Quick Scratch (see §7 for detail)

### 6.5 Library

See §8 for detail.

### 6.6 Out of scope for v1 (deferred)

- **CDJ-style cue / preview button** (set a temp cue point, jump back to audition) → **not built.** A turntablist cues with the needle, not a software cue button — this navigation affordance isn't needed for the target user. *Note: this is distinct from **hot cues** (performance trigger points for beat-juggling / finger-drumming), which **are** a v1 feature — see §6.2.1. Earlier drafts of this PRD conflated the two and wrongly deferred all cues to v2.*
- **Saved loop slots** (8 numbered, recallable) → **v1.x.** v1 ships ephemeral loops only. M11 includes the empty `track_loops` table so v1.x lands without a schema migration.
- **Sampler expansion (4 → 6 slots, à la Serato SP-6)** → **v1.x** *if real-world use demands it.* v1 ships 4 slots, symmetric with the 4 Quick Scratch slots.
- **Track Preparation Mode tooling** (beatgrid editor, gain tweak UI) → **v1.x.** M10.8 ships the *mode shell* — load + play + horizontal waveform — but no editing surface. The mode is *visible* in v1; its *tools* arrive in v1.x. (Hot-cue authoring shipped early, in both Performance and Prep — see §6.2.1.)
- **Stillpoint "numeric-only" variant** (Preferences toggle to hide the band and keep just the Δ BPM / Δ ms readouts) → **v1.x** *if real use suggests it.* v1 ships the single design and learns from how DJs actually use it.
- **Filesystem browser → full library** transition: v1.0's slim FS browser (M10.5) is intentionally minimal — folder navigation only, no metadata indexing, no crates. M11 lands the SQLite-backed library that replaces it.
- Recording → **v2**
- Streaming services (Tidal, Beatport, SoundCloud) → **v2+ or never**
- Phase → **v2**
- HID controllers → **v1.x or v2**
- Audio fingerprint recognition + persistent waveform learning → **v1.1** (post-launch)
- Apple Developer ID / notarization / auto-update → **v1.1**
- Stems / AI separation → **never**
- Video / OBS → **never**
- Software mixer / internal mixing mode (user-facing) → **never in v1/v2** (philosophy: external mixer is the product). v3 may reconsider for controller-only users.
- Mouse-driven **performance gestures** (pitch, scratch, crossfade, EQ, gain, cue) → **never** — per §1. Mouse-driven *transport* (panic-play, casual play, position navigation) → **explicitly in v1** (M10.6) and not in conflict with the philosophy.
- Cloud sync → **v3+**

---

## 7. Sampler, Quick Scratch & Instant Doubles

Dub has **three distinct sample/track-throw mechanisms** — each solves a different problem.

### 7.1 Sampler (one-shot, additive)

Classic DJ sampler. v1: **4 slots** (`A S D F`). The PRD considered matching Serato's SP-6 (6 slots), but rejected it: 4 keeps the keymap symmetric with the 4 Quick Scratch slots (`Q W E R`), and 4 has historically been enough for the target user's drop / siren / horn / vocal-stab workflows. Expansion to 6 stays on the table for v1.x if real-world use suggests it.

- One-shot trigger (key press → sample plays through and ends).
- Per-slot: gain, output assignment (default: master out / Deck A's output bus, configurable).
- Loadable via drag-and-drop from finder or library, or right-click "Assign to slot".
- Output is **additive** — sample plays *over* whatever Deck A/B are currently playing. Mixed into the deck's output bus, post-FX.
- Use case: air horns, vocal stabs, dub-siren one-shots, "rewind!" FX, drops.

### 7.2 Quick Scratch (hotkey-bound fast load)

Hotkey-triggered fast load of a sample to a deck. Semantically identical to dragging a track from the library — just instant.

- **4 slots** in v1 (`Q W E R` by default).
- Each slot is bound to a sample file (drag-and-drop to assign, or right-click).
- Each slot has a **target deck** (default: Deck A; configurable per slot).
- **Behavior**: pressing the hotkey **loads the sample to the target deck** as if the user had loaded it from the library. The deck reset to position 0 of the new sample, plays from the start, fully under timecode control. The user can scratch the sample using their needle.
- **Returning to a track**: the user loads a track normally afterward (drag, search, or another hotkey). There is no automatic "restore previous track" feature — that proved more complicated than valuable.
- **Workflow**:
  > Deck B plays. User wants to scratch a sample over Deck B. User presses `Q`. Deck A now has the assigned sample loaded at position 0; user scratches it with their needle. When done, user drags the next track to Deck A (or presses another Quick Scratch hotkey).

This is exactly the same load operation as the library's "load to deck", just keyboard-instant. Internally it shares the same code path as a library drag-and-drop. Quick Scratch slots are persisted across sessions per user.

### 7.3 Instant Doubles

Press a hotkey → the track currently loaded on one deck is duplicated to the other deck at the current play position. Used for juggling.

- **Hotkeys:** `Cmd+→` (Deck A → Deck B), `Cmd+←` (Deck B → Deck A). User-rebindable.
- Position alignment: sample-accurate.
- Both decks remain independently controlled afterward.
- If the destination deck has a track loaded, it is replaced (no confirmation; this is a performance feature).

### 7.4 Sample bundling

v1 ships with **no bundled samples**. UI prompts user to load samples on first run with a "Browse..." button. We may publish a curated CC0/royalty-free starter pack as a separate optional download from the GitHub releases page once we've vetted samples that don't sound like a free pack. **Decision deferred until late in v1 development.**

---

## 8. Library

The library is the user's set of tracks (filesystem files + metadata) and the relationships between them (crates, beatgrids, cues, mix history). Dub reads from external libraries, never writes to them, owns its own SQLite database for Dub-originated user data, and exports to standard interchange formats so the user is never trapped.

Three principles govern the whole subsystem:

1. **Local-first.** The library lives entirely on the user's machine. Zero telemetry on library content, no metadata fetched over the network, no phone-home. The user's library is never an asset Dub syncs, sells, or transmits. The target audience has been burned by Serato Cloud, rekordbox Cloud, and Beatport LINK; "your library never leaves your machine" is both a discipline and a positioning commitment.
2. **Source libraries are sacred.** Dub never modifies source library files (Serato GEOB tags inside audio files, Traktor `collection.nml`, rekordbox `master.db`, iTunes `Library.xml`) and never writes a sidecar file into a source library directory. Source files are opened read-only.
3. **No lock-in.** Everything the user does inside Dub (Dub crates, tap-to-grid corrections, future hot cues, future saved loops, custom tags) is exportable to a documented, standard format (§8.6). The Dub SQLite schema is itself documented and treated as a public API surface (§8.7).

### 8.1 Imports

Dub reads (does not own) external libraries. **One-shot import + manual re-scan**, no continuous live sync.

| Source | Format | What we read |
|---|---|---|
| Serato | ID3 GEOB tags, `_Serato_/database V2`, crate files | Tracks, BPM, beatgrids, hot cues (stored for v2 export round-trip), cue points, loops, file paths, custom tags |
| Traktor | `collection.nml` (XML) | Tracks, BPM, beatgrids, cues, key, comments, gain |
| rekordbox | XML export (v1.0); `master.db` (SQLite, encrypted, v1.1) | Tracks, BPM, beatgrids, cues, key |
| iTunes / Apple Music | `Library.xml` | Tracks, BPM (often missing), playlists, ratings |
| Lexicon DJ | Indirect, reads its rekordbox / Serato exports | As above |

**rekordbox `master.db` decryption:** the DB6 key is community-known. We will use a clean-room implementation when we tackle it. **v1.0 ships XML-export-only**; DB6 lands in v1.1 if the format is still stable then. Pioneer can change the format at any time; the XML path is the durable contract.

**Read-only access discipline.** Source files and library databases are opened with `O_RDONLY` semantics. We never advisory-lock a source library file; the user can have Serato / Traktor / rekordbox running while Dub imports.

**Per-source metadata preserved verbatim.** Different sources hold different opinions about the same track (Serato says "Dilla, J", rekordbox says "J Dilla", the ID3 frame in the file says "James Yancey"). Dub does not collapse these on import; each source's opinion is stored as a separate row in `track_metadata_source` keyed against the canonical track (§8.2). The browser picks a displayed value via a documented per-column priority chain (`serato > rekordbox > traktor > id3 > filename`) but every source's value is preserved and available for "what does each app think this is called?" UI in v1.x.

**Idempotent re-import.** Re-running an importer against the same source matches existing tracks by canonical identity (§8.2), refreshes the per-source metadata row, and preserves Dub-only data (Dub crates, play history, tap-to-grid corrections, prepared flag).

**Version-aware dedupe.** Hip-hop, reggae, dnb, and dubstep libraries characteristically contain multiple distinct files of the same recording: `(Clean)`, `(Dirty)`, `(Instrumental)`, `(Acapella)`, `(Radio Edit)`, `(Extended Mix)`, `(12" Mix)` etc. These have near-identical fingerprints and often near-identical durations, but the DJ must be able to pick between them on stage. Dedupe runs **lazily** — see "Lazy fingerprint" below — and never auto-merges in v1; the criteria below describe the decision the upcoming "Find duplicates" library action applies once both candidates have been fingerprinted. The merge candidate must satisfy **all** of:

- Chromaprint similarity ≥ 0.98
- Duration delta < 200 ms
- No version token differs between the two filenames or ID3 titles. The version-token list is `clean, dirty, explicit, instrumental, acapella, radio, edit, extended, club, dub, vip, remix, remaster, mono, stereo, intro, outro, short, long, 7", 12", lp`.

Otherwise the second file is registered as a separate `track` row with a "potential duplicate" link to the first, surfaced in the browser as a small link glyph the user can expand. A manual merge UI is v1.x. v1 is content to show two rows; that is strictly better than the wrong merge. The cost of silently collapsing "Clean" and "Dirty" into one row is "the DJ played the explicit version at a wedding"; we will not pay that cost.

**Lazy fingerprint (M11c.4).** Chromaprint computation requires a full audio decode, which on a large library dominates import wall-clock (~95 % of a cold import on commodity SSDs, per `crates/dub-library/examples/profile_import.rs`). The importer therefore writes `tracks` and `track_metadata_source` rows from a metadata-only probe (`dub_io::read_metadata`) and leaves `tracks.fingerprint_id = NULL` and `tracks.duration_ms = NULL`. `Library::analyze_track` — already invoked on first deck-load to populate beat-grid and key — computes the Chromaprint over the just-decoded samples, writes a `fingerprints` row, and attaches the id back to the track in the same pass. Dedupe is therefore also lazy: the user-facing "Find duplicates" action (surfacing near-duplicate fingerprints for manual review) is deferred to v1.x; v1.0 keeps every imported file as a distinct track row, on the principle that a false split is recoverable but a false merge is a stage hazard.

**Format-specific gotchas documented in `docs/LIBRARY-FORMATS.md`,** filled in during M11 / M12 as the importers land.

### 8.2 Data model

The Dub library is a SQLite database at `~/Library/Application Support/Dub/library.sqlite`. The schema separates three independent concerns: canonical track identity, per-source metadata (verbatim from external libraries), and Dub-originated user data.

**Canonical track identity** is a stable UUID assigned to a recording, not to a file. A canonical track may correspond to multiple on-disk files (different encodings, different drives) and multiple per-source metadata rows. The canonical identity survives file moves, drive renames, and re-encodes. Dedupe (§8.1) is what populates and merges this layer.

**Path-by-volume-UUID.** Each `track_files` row stores `(volume_uuid, relative_path_from_volume_root)` rather than an absolute path. On track load, Dub resolves files via the volume UUID first (handles the "the SSD mounted at `/Volumes/Touring` today but `/Volumes/Touring 1` tomorrow" case), falls back to last-known mount + relative path, then a basename + fingerprint search across known volumes, then prompts the user. Working DJs run libraries off external SSDs; absolute paths are fragile by construction. Engineering cost is one afternoon; trust earned is significant.

**Tables (v1.0 shape, with empty tables present for forward-compat).**

| Table | Purpose | v1 use |
|---|---|---|
| `tracks` | Canonical track row: UUID, created/updated timestamps, fingerprint reference, optional explicit-merge marker. | Populated. |
| `track_files` | One canonical track to many on-disk files. Stores `(volume_uuid, relative_path, codec, sample_rate, bit_depth, file_size, mtime, last_seen_at)`. | Populated. |
| `track_metadata_source` | Per-source verbatim metadata snapshots: one row per `(track, source)` with artist / title / album / comment / bpm / key / gain / version_token as that source reports them. | Populated. |
| `track_beatgrids` | One-to-many per track: `(source, anchor_seconds, bpm, is_active, captured_at)`. Multiple grids preserved (imported + auto-detected + tap-corrected); user can switch the active grid per track. | Populated. |
| `track_keys` | One-to-many per track: `(source, key_notation, original_notation, confidence, is_active, captured_at)`. `key_notation` is canonical Camelot (e.g. `8B`); `original_notation` preserves whatever the source wrote verbatim (`C major`, `Cm`, `5d`, …) so notation choices round-trip on export. Multiple keys preserved (imported + auto-detected + user-corrected); active key overridable per track. Schema v3 (M11c.2). | Populated. |
| `track_loops` | Saved loop slots per track. | Empty in v1; populated in v1.x without migration. |
| `track_cues` | Hot cues per track. | Empty in v1 *as far as the UI is concerned*; **imported Serato / Traktor / rekordbox cues are written here from v1 day one** so they round-trip on export (§8.6), they are simply not surfaced in the v1 UI per §6.6. |
| `crates` | User-created Dub crates: `(id, name, parent_crate_id, created_at)`. | Populated. |
| `crate_tracks` | Many-to-many membership Dub crates ↔ canonical tracks, with ordering. | Populated. |
| `imported_crates` | Read-only mirror of source-library crates (Serato `_Serato_/Subcrates/*.crate`, Traktor playlists, rekordbox playlists). Re-import rewrites this table; no user edits. | Populated. |
| `imported_crate_tracks` | Mirror membership. | Populated. |
| `fingerprints` | Chromaprint hash (algorithm 2, via pure-Rust `rusty-chromaprint`) + duration + size + bitrate signature. Keyed against `tracks` for dedupe and analysis-cache lookup. | Populated (M11b). |
| `volumes` | Known external volumes for path resolution: `(volume_uuid, last_known_mount_point, display_name, last_seen_at)`. | Populated. |
| `play_history` | Every load, play-start, play-end, deck-to-deck transition: `(track_id, deck, event_type, timestamp, duration_played_ms, from_track_id, to_track_id, session_id)`. | Populated from v1.0 day one (data capture; transitions inferred at handover, see `LIBRARY-SCHEMA.md`). Surfaces in v1.0 as "Last Played" sort, Recently Played + Session History smart crates, and the deck-header "↝ usually" hint (M11d-history); the full Played From / Played Into side panel lands in v1.x. |
| `analysis_cache` | LUFS-I, true-peak, prepared-flag inputs, waveform-sidecar pointer. Keyed by canonical fingerprint so the cache survives file moves and dedupe merges. | Populated. |
| `smart_crates` | Future user-defined smart crates: `(name, sql_predicate)`. | Empty in v1; v1 ships two hardcoded smart crates as code, not data (§8.5.2). |

The schema is documented in `docs/LIBRARY-SCHEMA.md` (published with M11a) and is part of Dub's public API surface (§8.7).

**Mix-history capture.** From v1.0 day one, Dub writes a `play_history` event on every load, play-start, play-end, and deck-to-deck transition. Because the mixer is external (§1), transitions are *inferred from deck transport*: a handover is recorded when a deck stops while the other deck keeps playing a different track, gated by a minimum-accumulated-play guard so timecode cueing (needle lifts, scratch holds) never writes false edges — the full heuristic is documented in `LIBRARY-SCHEMA.md`. The data drives "Recently Played", "Last Played" sort, and the M11d-history surfaces (Session History smart crate + deck-header "↝ usually" hint) immediately; it drives the full "Played From / Played Into" side panel in v1.x. Capture is local; nothing leaves the machine. The user can disable mix-history capture in Preferences (the table is still present, just not written to); the default is on (Preferences toggle lands with M18's preferences pass).

### 8.3 Beatgrids

- **Prefer imported.** When importing from Serato / Traktor / rekordbox, we use their grid as authoritative for the row's displayed BPM and the active grid in `track_beatgrids`.
- **Fall back to auto-detect** when no grid exists. Algorithm: the M7.5 `dub-bpm::analyze_bpm` offline driver (shipped, pure-Rust spectral-flux + autocorrelation; see [`docs/SHIPPED.md`](../history/SHIPPED.md)) feeds a grid placement step (anchor point + BPM, with downbeat detection from low-frequency emphasis). Same `BpmEstimator` core as the Thru streaming driver in §5.2.3, one DSP implementation, two front-ends.
- **Cross-validate every imported grid against `dub-bpm`.** Even on tracks that arrive with a Serato / Traktor / rekordbox grid, the analyzer runs once and the result is stored alongside the imported grid in `track_beatgrids`. When the imported and auto-detected grids disagree by more than 5 % BPM or 50 ms anchor over the first 32 bars, the row shows a small ⚠ "grid disagreement" indicator. The DJ would rather know "Serato says 92 BPM but the audio looks like 184 BPM" before they queue the track. The analysis cost is one-time per fingerprint and cached in `analysis_cache`, so re-imports are free.
- **Per-genre priority is user-configurable.** Default priority order is `Serato > rekordbox > Traktor > auto`, matching the typical urban-DJ workflow (Serato dominates pre-analyzed hip-hop / reggae catalogs). The priority is exposed in Preferences for users coming from rekordbox-first (techno / house: Pioneer's analyzer is tuned for 4/4 quantized grids) or Traktor-first (dnb / breakbeat: NI's analyzer handles syncopation and half-time better) workflows.
- **Active grid is overridable per-track** from the row's context menu. One database column flip, one menu item.
- **Manual correction = tap-to-grid only in v1.** That's the entire user-facing grid-editing tooling.

#### 8.3.1 Tap-to-grid (the only manual editing)

> **Sub-spec:** the precise contract for tap-to-grid, set-the-1,
> tempo, beat grids, and the waveform overlay lives in
> [`PRD-BEATS.md`](PRD-BEATS.md). This section is a
> one-paragraph summary; PRD-BEATS.md is binding.

The user plays a track. The deck-header BPM column accepts taps. **One or two taps** within a 2 s window is "set the 1" — it **re-anchors** the grid onto the tapped kick's leading edge (BPM preserved), moving the grid onto the transient the DJ pointed at rather than merely re-labelling the nearest existing beat. **Three or more taps** within a 2 s window run a **constrained re-analysis** in a tight **±3 % LSQ window** around the tap median: the full estimator runs again with the search range pinned by the user's hint, the best-fitting real peak in that window is the BPM (snapped to integer if residuals don't get worse), the first tap snaps to the nearest transient as the anchor, and `bar_phase` is the value that best fits the user's tap times across the four candidates. The tap is a search hint, never the BPM — a 2 s window of 3–8 taps cannot beat a full-track spectral-flux estimator on precision.

The tap result is written to `track_beatgrids` as `source=user_tap` and becomes the active grid; previous grids (imported, auto-detected) are preserved on the same row so the user can revert. **`grid_locked = true` is absolute**: no analyze, re-analyze, tap-tempo, or set-the-1 mutates a locked grid. The user explicitly toggles Lock grid off before any edit. The right-click menu shows a single Analyze / Re-analyze entry whose label switches on whether the track has been analyzed before, and whose enabled state follows `grid_locked`. Library row BPM and deck-header BPM are always the same value (single source of truth: the active `track_beatgrids` row); the waveform sidecar at `~/Library/Caches/Dub/waveforms/{fingerprint}.wf` is written by analyze passes and read synchronously on deck load so the waveform paints on the first frame.

Per PRD-BEATS.md, beat-grid edits never trim beats and never force the user's tap to be the "first beat" of the track — set-the-1 re-anchors onto the tapped kick, but the grid still extends both directions and is continuous across the whole track at all times.

#### 8.3.2 Key detection (Camelot canonical)

Same lazy-analysis lifecycle as the beatgrid (analyze on first deck load, cache forever, batch-analyze on right-click; see §8.4), implemented as a sibling DSP block in `dub-spectral` rather than a new top-level crate so the FFT pipeline is shared with the BPM analyzer.

- **Algorithm.** STFT (reusing the `SpectralFrameStream` pipeline that already feeds the BPM onset detector) → chroma vector (12 pitch classes via equal-temperament binning of log-magnitude, weighted by frame energy to discount silence) → time-averaged chroma across the analyzable section of the track → Pearson correlation against 24 Krumhansl-Schmuckler template vectors (12 major + 12 minor profiles, well-established music-theory templates with no licensing encumbrance) → argmax with confidence = correlation gap between first and second best. Pure-Rust; no FFI; lives in `dub-spectral`.
- **Camelot is the canonical storage notation.** `track_keys.key_notation` is always written as Camelot (e.g. `8B` for C major, `5A` for C minor). The Camelot wheel is the dominant harmonic-mixing convention in scratch DJ tooling (MixedInKey popularised it, Serato / Traktor / rekordbox all support it as a display option). Storing canonical avoids the Cm-vs-C-minor-vs-Am-relative-vs-5d translation soup at query time.
- **Original notation preserved verbatim.** `track_keys.original_notation` carries whatever the source wrote (`C major`, `Cm`, `5d`, `8B`). Two consequences: rekordbox-XML export round-trips exactly, and a future "Track Info" inspector can show all sources' opinions side by side without re-translating.
- **Cross-validation against imported sources.** Same model as beatgrid (§8.3 bullet 3). We always run `dub-spectral` key detection, even when the imported library (Serato / Traktor / rekordbox / MixedInKey via ID3) carries a key. When our auto-detected key disagrees with the imported key by more than a relative-major / parallel-minor shift, the row shows a small ⚠ "key disagreement" indicator. Relative-major (e.g. C major ↔ A minor — Camelot 8B ↔ 8A) is not flagged because it's a legitimate template ambiguity; absolute disagreements are.
- **ID3 TKEY frame.** Already parsed verbatim into `track_metadata_source(source='id3', key=…)` by M11c. We never overwrite the audio file. Tools that wrote Camelot into TKEY (Mixed In Key, most modern Serato installs) and tools that wrote music-notation (Traktor's `Cm` style) both round-trip on export untouched.
- **User correction.** A future v1.x "right-click → Set key…" affordance writes `track_keys(source='user', is_active=true)`. v1 ships read-only on user keys; correction lands when there's user demand and we know what the gesture should be (free-text? menu? roll-the-wheel? deferred to user research).

#### 8.3.3 Drift on non-quantized recordings

Honest disclosure: tracks that drift (vintage soul cuts, live-played reggae bands, breakbeat samples cut from drummer-played records) **will not stay grid-locked over a long mix**. A grid that's perfect at the intro will be 50–100 ms off by minute 5.

- **Indicator**: when Dub auto-detects that a track's transients deviate > 5 % from the fitted grid over its length, the deck shows a small ⚠ "May drift" indicator.
- **The mitigation is the DJ's hand on the pitch slider** — exactly as it has always been. We don't pretend otherwise.
- Multi-anchor flex grid (à la Ableton's "warp markers") is a v2 consideration, gated by user demand.

### 8.4 Track analysis

On-demand (first deck-load) or background; **not at import time** since M11c.4. Analysis is keyed by canonical fingerprint so the work runs once per recording, ever, regardless of how many file copies the user has or how many sources reference it. The first analyze pass for a freshly-imported track also computes and attaches the Chromaprint fingerprint (M11c.4 lazy-fingerprint contract; see §8.1).

- **Waveform** (multi-resolution overview + zoom), pre-rendered, cached on disk via the M10.5j sidecar at `~/Library/Caches/Dub/waveforms/{fingerprint}.wf`. The sidecar key migrates from the M10.5b path-based hash to the canonical fingerprint at M11a so the cache survives file moves and dedupe merges. The renderer reads through `analysis_cache.waveform_sidecar_path`.
- **Loudness (LUFS-I) and true-peak** measured and stored in `analysis_cache`, **and applied as a load-time normalization gain** — on by default, with a per-app opt-out. A LUFS column in the browser still lets the DJ scan for outliers before loading. The applied gain lines decks up at a consistent loudness on load so the DJ isn't re-trimming on every track change; it is a single multiply resolved once at load (`DubLibrary::track_normalization_gain` → `DubEngine::load_track(auto_gain)`), held for the life of the load, never revised by analysis that finishes later. **Auto-gain measures, it does not master:** it only sets the starting level — the hardware mixer trim still sits on top and has the last word, so it does not create the "fighting the app" failure mode (§2.1) the way an always-on AGC would. A DJ who wants to own level entirely turns it off in Preferences (`Loudness → Auto-match loudness on load`, persisted under `dub.loudnessAutoGainEnabled`); with it off, tracks load at unity and measurement is unaffected. The toggle gates *application* only — LUFS is always measured, cached, and surfaced regardless. *(The earlier v1 stance was "measured, not applied"; auto-gain was promoted into v1 as a default-on, opt-out convenience once it was clear an opt-out toggle resolves the "external mixer is the product" objection — the mixer still wins, this just picks a sane start.)*
- **Beatgrid** if not imported (§8.3). Cross-validation runs even when imported (§8.3 bullet).
- **Filename-derived metadata.** When ID3 tags are absent or matched against a junk pattern (`Track 01`, `Unknown`, the bare filename without extension, `downloaded from xyzblog.com`-class garbage), Dub parses the filename for common DJ patterns:
  - `ARTIST - TITLE.ext`
  - `ARTIST - TITLE (VERSION).ext`
  - `ARTIST_-_TITLE_(VERSION)_[YEAR].ext`
  - `[LABEL CAT#] ARTIST - TITLE.ext`

  The parsed result is stored as the `filename` source row in `track_metadata_source`. The browser falls back through the priority chain (§8.1). This is load-bearing for the bootleg, mixtape, and blogspot-era files that the target audience has in bulk; ID3 on these is absent or actively wrong.
- **Prepared / unprepared flag.** Derived: a track is `prepared` when it has all of (verified active beatgrid, LUFS-I cached, waveform cached). Surfaced as a browser column and a filter chip so the DJ can see what's ready for tonight and what needs attention. The flag is meaningful in both Performance and Prep modes; full Prep tooling (beatgrid editor, hot-cue prep, gain UI) is v1.x (§3, M10.8).
- **Key detection** per §8.3.2. Pure-Rust on top of `dub-spectral`; runs in the same lazy first-load / batch-analyze pipeline as the BPM analyzer; result stored in `track_keys` in canonical Camelot notation; cross-validated against imported keys with a relative-major-aware ⚠ indicator on disagreement.

### 8.5 Browser UI

This is the §6.4 keyboard-load target and the M11 replacement for the M10.5b slim filesystem browser (`apple/Dub/Performance/FileBrowserView.swift`).

**Layout.** Source tree on the left (collapsible), virtualized track list on the right. **No in-browser preview.** Cueing happens *in the deck*, on the user's hardware mixer headphones, exactly as a real DJ pulls a record out of a crate, drops it on the deck, and cues with their headphone monitor. The browser's job is finding tracks, not previewing them.

Performance: list virtualization required. Lexicon-class libraries hit 100k+ tracks; we only realize visible rows.

#### 8.5.1 Source tree

Top to bottom:

- **All Tracks** (every canonical track, regardless of source).
- **Smart Crates** (v1.0 hardcoded list, §8.5.2).
- **Dub Crates** (user-created, full-color icon, editable). Drag tracks in from any source. Nestable. Persisted in `crates` / `crate_tracks`.
- **Imported Sources**, one node per configured source (Serato, Traktor, rekordbox, iTunes), each containing a **read-only mirror** of that source's crates / playlists. Visually distinguished from Dub Crates (greyscale icon + lock glyph). Re-import rewrites this subtree without touching anything else.
- **Real Records** (v1.1, fingerprint-recognized records the user has played in Thru mode; see §5.2.2, M21).

The split between Dub Crates (editable, owned) and Imported Sources (read-only, mirrored) is non-negotiable. The user already has 200 Serato crates that took them eight years to organize; we are not the system of record for that. If we let the user "edit" an imported crate, the next re-import clobbers the edit and the DJ loses trust forever. Imported crates are sacred mirrors of the source app's truth; Dub Crates are the user's free space.

#### 8.5.2 Smart crates

v1.0 ships exactly three hardcoded smart crates, no user-defined rule builder:

- **Recently Played**: last 200 tracks across all sessions, newest first. Backed by `play_history`. The single most-used sort column in pro Serato workflows.
- **Session History** (M11d-history): this app run's set list — every track with a `play_start` in the current session, in play order (newest first). Rows mixed into from the other deck carry a "← from \<track\>" annotation from the session's recorded transitions (§8.2). The post-gig "what did I actually play" surface, available before the DJ has even left the booth.
- **Just Imported**: tracks added since the last app launch. Catches the "USB stick dropped on the machine 10 minutes before the gig" workflow.

**Played From / Played Into** ships in two stages. **v1.0 (M11d-history)** records handover-inferred transitions (§8.2) and surfaces them in two places: the deck header's "↝ usually: \<track\>" hint (§9.5 row 3) — the most-common mix-out target for the just-loaded track — and the Session History annotations above. **v1.x** grows the full context-bound side panel on the browser: when a track is loaded on Deck A, the panel shows the N most-common tracks the DJ has previously mixed *into* it from Deck B, and the N most-common tracks they have mixed *from* it. Backed by the same `play_history` table. **No commercial DJ app surfaces this**; it is a genuine differentiator for Dub and a load-bearing feature for the working scratch DJ who plays the same tracks across many sets and wants to remember which transitions worked.

A user-defined smart-crate rule builder is parked until v1.x at the earliest. The `smart_crates` table exists empty in v1.0 so v1.x lands without a migration. Every DJ app that shipped a rule builder in v1 (Serato, Traktor, Lexicon) ate a quarter of product roadmap on it and ended up with users who didn't know how to use it; we wait for real demand.

#### 8.5.3 Track list

- **Default columns.** Title, Artist, BPM, Key, Length, Comment. Sortable; user-reorderable via header drag (SwiftUI Table on macOS 14+).
- **Customizable column set** per §8.5.3.1. The user opens the column picker by right-clicking any column header; choices persist across launches.
- **Loaded-now badges.** Tracks currently loaded on Deck A or Deck B carry a small accent-colored `A` / `B` glyph in the leftmost gutter. Prevents the "I just loaded the track that was already playing" mistake and visually confirms an Instant Doubles call (§7.3).
- **Grid-disagreement indicator** (§8.3) as a small ⚠ in the BPM column.
- **Key-disagreement indicator** (§8.3.2) as a small ⚠ in the Key column.
- **Potential-duplicate indicator** (§8.1 dedupe) as a small link glyph on the row; click expands a sibling row showing the candidate duplicate.
- **Missing-file indicator** (§8.5.5) as a small dim red glyph on the row.
- **Virtualized rendering** is a hard requirement; only visible rows are realized.

##### 8.5.3.1 Customizable columns

Right-click any column header → context menu, grouped by category, checkable items per available column. Choices persist in `~/Library/Application Support/Dub/preferences.json` as **one global column set** across all sidebar sources (v1.0 simplification; per-source layouts deferred to v1.x if real use warrants the persistence-key + crate-rename plumbing). Column order persists via the same mechanism; widths via SwiftUI's built-in `TableColumn.width`.

The available columns are exposed across the FFI as a stable Rust enum `LibraryColumnId` with serde-stable string names (so preferences round-trip across schema updates without churn), grouped as:

| Group | Columns |
|---|---|
| Identity / library | `date_added`, `source`, `in_crates`, `duplicates`, `missing` |
| Active-priority metadata (the §8.1 priority chain) | `title`, `artist`, `album`, `genre`, `comment`, `composer`, `track_number`, `year`, `version_token` |
| Per-source metadata (verbatim from `track_metadata_source`) | For each source ∈ `{id3, filename, serato, traktor, rekordbox, mixedinkey}`: `{source}_title`, `{source}_artist`, `{source}_album`, `{source}_comment`, `{source}_bpm`, `{source}_key`, `{source}_year`. (Some sources don't carry all fields — e.g. `filename` only has title / artist / version / year; the unsupported entries are simply absent from the registry.) |
| Analysis | `bpm_active`, `bpm_auto` (Dub's analyzer, M11c.1), `key_active`, `key_auto` (Dub's analyzer, M11c.2), `length`, `lufs_i`, `true_peak`, `prepared` |
| Audio file (from `track_files`) | `codec`, `bit_rate`, `sample_rate`, `bit_depth`, `channel_count`, `file_size`, `file_path`, `file_modified` |
| Mix history (from `play_history`, aggregated) | `play_count`, `last_played`, `last_loaded`, `played_last_7d` |

The per-source metadata group is the **load-bearing differentiator** versus single-source DJ apps: a user migrating from Serato can enable `serato_bpm` alongside `bpm_auto` to scan for tracks where Serato's grid disagrees with Dub's, click to sort, fix outliers in bulk. The ⚠ disagreement indicator (§8.3, §8.3.2) is the at-a-glance summary; the explicit per-source columns are the spreadsheet view.

**Implementation note.** The track-list SELECT is generated dynamically from the active column set so disabled columns cost zero query time. Each enabled per-source group adds one `LEFT JOIN` of `track_metadata_source` aliased by source (`i3`, `sr`, `tr`, `rb`, `mk`); the index on `(track_id, source)` makes each join log-N. Benchmarks on the 5 000-row "All Tracks" view at v1.0 cap stay under 200 ms even with all six source-groups enabled.

#### 8.5.4 Search

SQLite FTS5 over `(artist, title, album, filename, comment)`.

- **Default**: case-insensitive substring, `AND` across whitespace-separated tokens. No Levenshtein, no "did you mean", no fancy fuzziness. Pro DJs type fast on stage and reward predictability; false positives during a set are worse than zero hits.
- **Operators** (power-user, additive to substring), available in v1.0:
  - `bpm:90-100`
  - `key:Am`, `key:~Am` (key-compatible neighbours)
  - `source:serato` / `source:traktor` / `source:rekordbox` / `source:itunes` / `source:dub` / `source:filesystem`
  - `played:<7d` / `played:never`
  - `prepared:no` / `prepared:yes`
  - `version:clean` / `version:dirty` / `version:instrumental` / `version:acapella` etc.

  Documented in the README and accessible via a `?` glyph next to the search field.

Search target latency: < 50 ms on a 100k-track library on M2 Air. SQLite FTS5 handles this easily; the engineering risk is zero.

#### 8.5.5 Missing files

External SSDs unmount. Files get moved by Finder. Networked volumes disappear. The library handles this gracefully:

- On app startup and at low-priority intervals, a background task confirms file existence for `track_files` rows via `access()` (no read I/O). Rate-limited so it does not trash SSD lifetime.
- A small footer in the browser: `247 tracks missing. Click to relocate.`
- The Relocate panel accepts a directory and tries to match missing files by `(fingerprint hash, filename, duration)`. Matches update the `track_files` volume + relative-path columns; mismatches stay missing.
- **Metadata is never deleted when a file goes missing.** The user will plug the drive back in. Losing a year of `last_played` history and tap-to-grid corrections because the touring SSD got ejected is unacceptable.

#### 8.5.6 Load + drag

- **Drag** a row onto a deck pane to load (existing M10.5b drag path).
- **`Space`** loads the focused row into the stopped, non-master deck per §6.4 (see also §5.5).
- **`Enter`** focused-deck-load semantics are reserved for v1.x; v1.0 only commits to Drag and Space.

### 8.6 Export and interop

Dub never writes to source libraries (§8.1). To prevent lock-in of Dub-originated user data (Dub crates, tap-to-grid corrections, future hot cues, future saved loops, custom tags), Dub ships first-class exporters. The user owns their work and can take it elsewhere with one click.

| Format | What it carries | Read by | Milestone |
|---|---|---|---|
| **rekordbox XML** (`.xml`) | Tracks, BPM, key, beatgrid (BPM + first-beat anchor), hot cues, loops, nested playlists | Serato, Traktor, rekordbox, Lexicon | M11f |
| **M3U / M3U8** | File paths only (lossy on cues / grid / metadata) | Universal | M11f |
| **Dub JSON** | Dub-specific data: mix history, play history, grid-disagreement notes, prepared flag, custom tags. Schema documented publicly. | Dub only; future tools by anyone willing to read the schema | v1.x |

The rekordbox XML format is the de facto DJ-library interchange standard (documented schema, accepted as an import source by Serato, Traktor, rekordbox, and Lexicon's own primary output). Shipping a competent rekordbox-XML exporter at M11f makes approximately 90 % of Dub-originated data portable to any other DJ app: the user exports a Dub crate, drops the XML into their alternative app, and the cues / loops / grid corrections come with them. **This is the load-bearing anti-lock-in commitment.**

The remaining 10 % (mix history, play history, prepared flag) is data that exists in Dub because no competing product models it. Losing it on migration costs the user nothing they had before Dub; we still export it as JSON so the data is not trapped.

Export is a one-click `File → Export Crate As...` UI surface with the target format as a dropdown. Making leaving easy is the load-bearing behaviour: the DJ trusts a tool that doesn't try to trap them. Bury this and the trust goes.

**Round-trip discipline.** Imported hot cues / loops are stored in `track_cues` / `track_loops` from v1.0 day one (§8.2) so an export at M11f preserves them losslessly, even though the v1 UI does not surface or edit them. A user who imports a Serato library, builds a Dub crate, and exports to rekordbox XML gets their Serato hot cues back on the other side.

### 8.7 Schema as public API

The Dub SQLite schema is documented in [`docs/LIBRARY-SCHEMA.md`](LIBRARY-SCHEMA.md) (published with M11a) and is treated as a public API surface: we will not break it without a documented migration path and a stable version bump. Third-party tools may read `library.sqlite` directly. The Chromaprint parameters Dub uses are similarly documented so a third party can re-derive Dub's fingerprints.

This is the ultimate anti-lock-in commitment: even if Dub disappears, the user's data is in an open, documented file format they can hand to any SQLite tool. No competing DJ app makes this commitment (Apple Music's schema is opaque, Serato's GEOB blobs are undocumented and version-coupled, Traktor's NML evolves silently, rekordbox actively encrypts to prevent it). It costs us nothing engineering-wise and is a genuine differentiator for the target audience, which has been burned by closed-format DJ tools for two decades.

---

## 9. UI principles

### 9.1 Design ethos

> **Design means usability.** Every pixel justifies itself. If a control isn't used in a typical scratch session, it's not on the main view.

- **Modern, dark, calm.** Not Las-Vegas neon, not skeuomorphic decks. Something closer to Logic Pro / Ableton 12 than to rekordbox 7.
- **Two decks, equal weight, side-by-side**, with library below or in a togglable panel.
- **Waveforms front and center, vertical, side-by-side.** Two parallel vertical waveform columns — Deck A on the left, Deck B on the right — with time running **bottom → top**. This matches Serato Scratch Live's vertical-waveform mode and, more importantly, mirrors the platter's rotation: when the DJ pushes the record *forward*, the groove segment about to play moves *upward* under the needle, and the waveform must move the same way under the playhead so the on-screen motion never contradicts the hand. Horizontal-time waveforms (Traktor, default rekordbox, default Serato DJ) are explicitly not the model.
- **Playhead at 25 % from the top.** The user sees 25 % of what just played *above* the playhead and **75 % of what's coming up *below*** it. The waveform scrolls **upward** through the stationary playhead during forward playback — future rises from the bottom of the screen, passes through the playhead (where it is "now playing"), and continues upward into the played-region above, eventually sliding off the top. Reverse playback (manual rewind, backspin) inverts this: the waveform marches downward, future falls back into the playhead from above, freshly-recovered past pushes downward off the bottom. What's coming is more important than what's gone — most DJ apps (and even Serato's own vertical mode) put the playhead at center; this is wrong for the audience-facing DJ who needs to see *into the future* of the track. **Direction discipline:** the audio playhead's on-screen motion must match the hand's motion on the platter — forward platter rotation = waveform marches upward through the field; reverse (rewind) = waveform marches downward. The deviation from convention is the playhead's *position* (25 % top, not centre), not its *direction*.
- **Type-driven** — readable type at performance distance (1–2 m from the screen).
- **Color = function, not decoration.** Deck A and Deck B each have a single accent color; everything else is neutral.
- **No skeuomorphism, no jog wheel graphics, no fake CDJ overlays.** This is software, not a stage prop.

### 9.2 Layout (v1)

```
┌─ STATUS STRIP ──────────────────────────────────────────────────────┐
│ DUB · 48.0 kHz · LIVE · INPUT · TIMECODE  ····  CLOCK 21:47 · 🔋87% │
├─────────────────────────────────────────────────────────────────────┤
│ A · TIMECODE · The Test                     │ B · THRU · Live Capture│
│ A · 122.4 BPM · ±0.0 % · F♯m · FX —         │ B · 122.7 BPM · — · — │
│ A · 02:14.7 elapsed / 05:23.1 remaining     │ B ·  —                │
├──────┬──────────────────┬─────────┬─────────┬──────────────────┬────┤
│ ░░░░ │ ─past 25%────    │ │ phase │         │    ────past 25%─ │░░░░│
│ ░░░░ │ ▌▌▌▌▌▌▌ playhead │ │ drift │         │ playhead ▌▌▌▌▌▌▌ │░░░░│
│ ░░░░ │ ── ── ── ── ──   │ │trail │         │   ── ── ── ── ── │░░░░│
│ ░░░░ │ ── ── ── ── ──   │ │ ·  · │         │   ── ── ── ── ── │░░░░│
│ ░░░░ │ ─future 75%───   │ │ ·  · │         │   ─future 75%─── │░░░░│
│ ░░░░ │ ── ── ── ── ──   │ │  ·  ·│         │   ── ── ── ── ── │░░░░│
│ ░░░░ │ ── ── ── ── ──   │ │  ·  ·│         │   ── ── ── ── ── │░░░░│
│ over │   DECK A zoom    │ │ Δ BPM│         │   DECK B zoom    │over│
│ view │   (~4 bars)      │ │+0.3  │         │   (~4 bars)      │view│
│ A    │                  │ │ Δ ms │         │                  │ B  │
│ (thin│ forward play =   │ │ +12  │         │ forward play =   │thin│
│  full│ marches upward   │ │      │         │ marches upward   │full│
│ track│                  │ │ §9.4 │         │                  │track│
│      │                  │ │      │         │                  │    │
├──────┴──────────────────┴─┴──────┴─────────┴──────────────────┴────┤
│  FX bar (Deck A):   Echo-Out · Dub Siren · Quick Scratch · Sampler  │
│  FX bar (Deck B):   Echo-Out · Dub Siren · Quick Scratch · Sampler  │
├─────────────────────────────────────────────────────────────────────┤
│  Library / Browser                                                  │
│  [Source tree]  |  [Track list]                                     │
└─────────────────────────────────────────────────────────────────────┘
```

**Top:** the **Status Strip** (§9.3) — DUB wordmark, sample-rate, engine state, current source mode per deck (collapsed into the strip when no deck-specific info needs to be shown), wall clock, battery indicator. Persistent during the entire session.

**Below that:** **Deck headers** — three logical rows (identity / live stats / track time), two columns (deck A / deck B). Source pill, track title + artist, format, BPM, pitch, key, FX state, elapsed / remaining time. See §9.5 for the live-state column responsibilities.

**Centre:** **Two decks face each other symmetrically** — A on the left, B on the right. Each deck has a **thin overview column on its outside edge** (full track at a glance, click-jumpable per §6.1) and a **wide zoomed column on its inside edge** (~4 bars visible). Beatgrid is overlaid as faint horizontal lines on the zoomed column.

**Centre gutter:** **Stillpoint** (§9.4) — Dub's beatmatching aid. A narrow vertical strip co-located between the two waveforms (where the DJ's eyes naturally focus during a mix). Replaces both Serato's Tempo Matching Display and Traktor's phase meter with a single motion-nulling visualization. This is the only thing that lives in the centre gutter; it never houses controls.

The `▌` marks the playhead, fixed at 25 % from the **top** of each zoomed column. **Upcoming audio fills the lower 75 % of the column;** during forward play it rises through the playhead and slides off the top of the played-region above. Reverse playback (manual rewind, backspin) inverts this: the waveform marches downward, exactly mirroring the hand on the platter. This is the load-bearing UX commitment of the layout — on-screen motion direction must equal hand-on-platter motion direction at all times, because any contradiction between them costs the DJ a frame of cognitive translation that turntablist muscle memory cannot afford during scratch work.

The decks dominate vertical real estate intentionally. Scratch DJs spend the majority of a set staring at the waveform; everything else (FX, library) is subordinate.

### 9.3 Status Strip

A thin (≈ 28 px tall) row across the top of the window. Always visible, never interactive (clickable elements never live here — the status strip is *read-only by contract*; interaction lives in deck headers, the waveform region, and the Preferences sheet).

Left → right:

| Element | Source | Why |
|---|---|---|
| **DUB wordmark** | static | Identity. |
| **Engine state dot + sample-rate** | `DubEngine.sampleRate()` | Quick "is the engine running, at what rate" check during sound-check. Green dot + `48.0 kHz` when running; grey dot + `IDLE` when stopped. |
| **Source-mode summary** | per-deck source pill (PRD §5.1) | Optional condensed echo of the deck headers' source pills when the user needs an at-a-distance read. Hidden when both decks' source pills are already visible at their natural size. |
| **Spacer** | — | Pushes the right cluster to the edge. |
| **Wall clock** | system clock | Set-timing utility. "27 minutes until I need to be off stage." Format `HH:MM` (24-hour); user can switch to 12-hour in Preferences. No seconds — the DJ glancing at this should not be distracted by ticking digits. |
| **Battery indicator** | `IOPSCopyPowerSourcesInfo` | Critical safety: when running on battery, show `🔋 87 %` in dim amber; when battery drops below 20 %, the glyph turns red and pulses for ~3 s on transition. When plugged in, the glyph shows a power-plug variant and never alarms. **Critically: low battery never blocks or pauses playback — it warns only.** A DJ on stage cannot have the app refuse to play because the laptop is low; that's the audience's career-night moment, not a safe-default trigger. |

The Status Strip is intentionally bare. Hardware connection status, USB dropout indicator, CPU meter, and per-deck level meters are all **out of scope for v1** (level meters: PRD §9, decided in M10.3 round; CPU + USB dropout: M18 polish if at all).

### 9.4 Stillpoint (beatmatching aid)

The PRD's single most opinionated visual design decision. Replaces Serato's "Tempo Matching Display" (a row of peaks) + Traktor's phase meter (a needle/dial against a beatgrid) with one **asymmetric, motion-nulling** display: **Stillpoint**.

> **Binding sub-spec:** the full Stillpoint design lives in [`BEATMATCH-AID-STILLPOINT.md`](../investigations/BEATMATCH-AID-STILLPOINT.md) and is the source of truth for the centre-gutter aid (the shipped `StillpointModel` / `StillpointView`). This section is the PRD-level summary; the earlier "Phase-Drift Trail" dot-trail design it replaced is preserved only in git history.

#### 9.4.1 Problem statement

Beatmatching by ear is the bedrock skill of the target user. The visual aids most DJ apps ship have two persistent failure modes:

1. **Serato's peak rows** are a low-resolution position indicator: a 0.1 BPM mismatch takes many bars to visibly desync. By the time the DJ sees it, the tracks have already audibly drifted. The numeric BPM readout (e.g. `120.1` vs `120.0`) is consistently more useful in practice. The visual aid is *less* precise than the text it sits next to.

2. **Traktor's phase meter** trusts the inferred beatgrid. When the grid is even slightly wrong — and on real-world music with micro-timing, swing, or pre-analysis errors, it usually is — the meter lies. It says "in phase" while the kicks audibly clash. It's most likely to fail on the genres the target user plays the most: dub, reggae, dnb with shuffled snares, hip-hop with off-grid loops.

Both failure modes share a root cause: **trust in quantised abstractions (BPM numbers, inferred grid positions) instead of the actual audio.**

#### 9.4.2 Design (summary — full spec in the sub-spec)

Stillpoint is **asymmetric**, matching how a DJ actually thinks during a blend: the *master* deck owns the room and is never drawn as a contestant — **the master is the lock line** (a hairline collinear with the waveform playheads). The only object the DJ acts on is one **incoming-tinted band** on that line. Governing law: **nothing in the gutter moves at beat rate — only at error rate.**

- **Tempo as drift, not position.** Tempo error renders as motion (a scrolling belt pre-drop; band drift post-drop) — matched = the world *freezes*, the same motion-null read a turntablist already trusts from a Technics strobe. Climbing = your record is slow (pitch `+`); sinking = fast.
- **Phase as displacement.** After the drop the band sits above the line (you're late) or below (early); you nudge it into the ±10 ms pocket on the line.
- **Lock as a growing green line.** Certified lock ignites the line green and grows it outward (~2 px/beat); lock-loss **shatters from the edges** as a one-shot event, never a slow fade. A false green has no render path.
- **Ride-stage coach.** A repeated same-direction nudge surfaces a small `± n.n %` pitch-trim hint — the app quoting the DJ's own hands back. No shipping product does this.
- **Honesty.** Low onset confidence renders as stillness / ghosting, never a confident-looking wrong position.

The display runs at **60 Hz** (the round-2 dot-trail's 30 Hz was superseded). Surface: a 120 px gutter at full waveform height, lock line at y ≈ 25 %.

#### 9.4.3 Grid coupling — a known v1 limitation

The shipped Stillpoint derives beat phase from each deck's **inferred beatgrid** (`StillpointModel`: `φ = frac((playhead − anchor) × bpm/60)`), *not* from a grid-agnostic ODF cross-correlation. So — ironically, given §9.4.1 point 2 faults Traktor's phase meter for exactly this — **a wrong beatgrid currently does degrade the aid.** This is an accepted v1 trade-off: it is "good enough" on the genres tested, and the grid-agnostic ODF rewrite (raw log-band spectral-flux cross-correlation — the design's original promise that "a wrong grid does not affect the display") is **parked until variable beatgrids land**, at which point the φ/Δ producers swap to the ODF path behind the same UI. The Rust `dub-match` crate the earlier design sketched was **not built**; Stillpoint is implemented in Swift (`StillpointModel` + `StillpointView`), consuming the engine's existing per-deck rate/grid publish.

#### 9.4.4 Trade-offs we accept

1. **Novel display, first-time interpretation needed.** DJs trained on Serato/Traktor won't read it at first glance; a `?` legend mitigates ("make it stop moving, then sit it on the line").
2. **Centre-gutter real estate** (120 px × full height) is reserved for this and nothing else — the DJ's eyes already converge there during a mix, so anywhere else costs an eye-saccade per check.
3. **Grid coupling** (§9.4.3) until variable grids land.

A **numeric-only variant** (drop the band, keep just `Δ BPM` + `Δ ms`) is a future possible Preferences option but **not v1** (§13.4).

### 9.5 Deck header (per deck)

Each deck header (top of each deck's column) is a three-row, two-column-aware strip:

**Row 1 — Identity.**
- Deck label (`DECK A` / `DECK B`) in the deck's accent tint.
- Source pill (`TIMECODE` / `TIMECODE · HOLD` / `THRU · LIVE` / `FILE` / `OFF`) with a dot whose colour encodes the tracking-quality indicator from §5.4: green = clean lock, amber = degraded, red = no lock / scratching / cueing (per Serato convention, red is *normal* during scratching).
- Track title + artist (live from file metadata or library, em-dash placeholder when none).
- Format chip (`FLAC · 44.1 kHz` etc.) — chip is hidden in Thru mode.

**Row 2 — Live stats.**
- `PITCH ±X.X %` — deck-rate as set by the turntable (from M5 timecode pipeline) or `±0.0 %` in File mode.
- `BPM XX.X` — track BPM, from `dub-bpm` tracker. Tentative readings shown italicized.
- `KEY` — `—` until v1.x key-detection ships (or M11-imported library metadata, whichever lands first).
- `FX` — active Smart FX state (Echo-Out / Dub Siren / off) — populated from M15 / M16 onward.

**Row 3 — Track time.**
- **Performance / Timecode mode:** `-MM:SS` remaining only. Right-aligned. Audience-facing "30 seconds to mix" cue (§6.1). The otherwise-empty middle region carries the M11d-history hint `↝ usually: <track>` (tertiary text, tail-truncated, lowest layout priority so the time never compresses): the track the DJ has most often mixed into from the loaded one, from `play_history` transitions (§8.5.2). Omitted when the track has no recorded transitions or didn't come from the library — the row's height and the time's position never change. **Clicking the hint reveals the suggested track in the library browser** (selects + scrolls, switching to All Tracks and clearing the search when the track isn't in the current listing); `Space` then loads the selection through the existing §6.4 flow. The click is a shortcut, not a requirement — keyboard search remains the no-mouse path (§1).
- **Prep mode:** `MM:SS` elapsed (left) · `-MM:SS` remaining (right). Both shown because the single-deck rehearsal surface has the screen room and elapsed time is the natural anchor for hot-cue placement.
- **Thru mode (no track loaded):** the row is omitted entirely — no canonical playhead concept when timecode is driving the rate.

Header height is fixed at ≈ 92 px regardless of which rows have content. Empty rows preserve their height to avoid layout reflow when source mode changes mid-set.

### 9.6 Waveform rendering

- GPU-rendered via Metal (`MTKView`).
- Two views per deck: **overview** (whole track, thin vertical strip on the deck's outside edge) + **zoomed** (≈ 4 bars, wide vertical strip on the deck's inside edge).
- **Playhead is fixed at 25 % from the top of the zoomed column**; during forward playback the waveform scrolls *upward* through the playhead at `1 / (engine_rate)` pixels per sample (future rises from the bottom, plays into the playhead, slides upward into the past above). During reverse playback the waveform scrolls *downward* — direction follows engine rate sign, never inferred or corrected. The user sees the future below the playhead, the past above. The overview's playhead marker tracks the same position as a horizontal hairline.
- **Empty-groove rendering at track edges.** Both the past region (above the playhead) and the future region (below the playhead) always draw their full pixel extent, even when the playhead is at the very start or very end of the track. Slots in the renderer's peak-chunk ring buffer beyond the loaded `peaksLen` range are guaranteed zero — the ring is zero-initialised at construction and on every `reset()`, and the bounded `ingestNewChunks` writer only ever fills `[0, peaksLen)` — so the shader reads "silence" peaks for any column that addresses a global chunk index `< 0` or `≥ peaksLen` and the trapezoid collapses to zero height. The visual result is the **lead-in / lead-out groove of a real record**: a flat dark band of "empty room" above the track at `t = 0`, and a flat dark band below the track at `t = duration`, with the bars themselves remaining at their natural pixel-per-chunk cadence. The dark band carries a 1-px white-at-22 %-opacity zero-crossing line through its centre (the amplitude=0 axis) so it reads as "needle is on the platter, just no audio" rather than pure black; the same line runs through the bars region too, but is largely hidden by the bar geometry which is centred on it. Without this the regions used to *collapse* as the playhead approached an edge (e.g. at `t = 0` the past region shrunk to zero width and the bars "warped" against the strip edge), which felt wrong against the rest of the renderer's fixed-axis design. The empty-groove path is gated on the ring having enough zero-init headroom past `peaksLen` to keep both regions clear of loaded data; tracks long enough to violate that (≈ > 25 min at the broadband 64-sample chunk cadence) fall back to the pre-existing collapse-at-edges behaviour rather than risk a column wrapping into real chunks.
- **Playhead can be pushed past the track edges.** The renderer drives its `playheadChunk` arithmetic off `PositionInfo.playhead_secs_unclamped` (FFI v14, M10.5t) rather than `elapsed_secs`. The unclamped field carries the raw `position_frames / sample_rate` from the audio thread, which the deck already lets walk past `[0, frames)` while emitting silence (matching a real platter being scratched off the end of its run-out groove). The renderer's ring-offset math is signed-Int64 with Euclidean modulo, so a negative or post-`peaksLen` global chunk index wraps cleanly into the zero-padded tail of the ring and the past / future regions render silence in the right places. Time-display consumers (deck header, track overview bracket) keep using the *clamped* `elapsed_secs` so a hard scratch past the end doesn't make the running counter run negative on screen.
- **Click-to-jump on the overview column** (per §6.1) seeks the deck to that absolute position. Single click only — the overview does not support drag-scrub or scrub-with-audio. Transport state is left alone (paused stays paused at the new position; playing keeps playing from it). Works in both Performance and Prep mode. Both the click-to-fraction maths and the playhead bracket position are computed on the **same chunk grid the bars are laid out on** (`peaksLen × chunkDurationSecs` seconds) rather than through the deck's reported `durationSecs`. This is the same M10.5n principle applied to the overview: peaks are cadenced in track frames, all seek and playhead maths must stay on that grid, otherwise a sub-millisecond mismatch between `track.frames() / track_sr` and `peaksLen × chunkDurationSecs` accumulates into a visible "which bar represents the current audio" misalignment by the end of the file. The overview also reserves a small dark padding region (8 pt) at each end of its time axis so the first and last bars don't kiss the strip edges — without it the bars read as a solid block and feel "warped" against the strip corners. Click positions inside the padding clamp to the nearest bar.
- **Drag-to-scratch on the zoomed column** (per §6.1) is the cue-locating surface — a vinyl-style rate-driven scratch. The mouse cursor's *velocity* (pixels per real-second, converted to audio-seconds per real-second) drives the deck's playback rate every block; cursor-still means rate-zero means silence, exactly like a stylus on a stationary platter. There is no auto-play on click and no implicit seek-to-cursor: the cursor maps to motion, not to position. In Timecode mode the shell engages Panic Play for the duration of the drag so the timecode driver doesn't fight the cursor-driven rate; on release Panic Play cancels and the pre-scratch transport (play / pause) is restored. Allowed in both modes — see §1 for the cueing-affordance carve-out. **Rate derivation is per-event, not polled** (M10.5t rework): each `onChanged` from the SwiftUI gesture overlay computes `instantRate = Δoffset / Δrealtime`, low-passes it with an EMA (α = 0.35), and writes the result through `setDeckRate`. A 60 Hz watchdog timer runs alongside *only* to ramp the deck rate toward zero when no event has fired for ≥ 25 ms (cursor held still). The earlier polled implementation snapshotted the running offset on a fixed 60 Hz clock, which beat against the 60–120 Hz cursor-event stream and surfaced as audible "jumping" on a steady drag — that is the bug the per-event path fixes.
- Beatgrid overlay drawn as **horizontal** lines across the zoomed column (white = downbeat, gray = beat).
- 60 fps minimum, 120 fps where supported (ProMotion).
- Timecode signal scope is also Metal-rendered (cheap — small circular buffer), shown as a small overlay on the deck (not a separate panel).
- During scratching, waveform tracks position with no visible lag relative to needle (i.e. ≤ 1 frame).
- `+` / `-` keyboard shortcuts zoom the focused deck's zoomed column in / out (Serato parity).

#### 9.6.0 Waveform baseline freeze (M10.8 cleanup)

The current Metal waveform shader is the **frozen Serato-parity baseline** for M10.8 dogfooding. It deliberately keeps the renderer simple and inspectable:

- height comes from per-pixel-column broadband `PeakChunk` max aggregation;
- colour comes from 8 log-spaced `dub-spectral` bands grouped into calibrated low / mid / high channels;
- low / mid / high anchors are tuned to the tested Serato references: pink-red kicks, green mid/presence instruments, lavender high hats, and dark neutral quiet sub-bass details;
- quiet greying is gated by broadband amplitude **and** sub-bass focus (`b0`, roughly <80 Hz at 44.1 kHz), not by "quiet" alone.

Future waveform work must be **additive and reversible** relative to this baseline. Do not reintroduce the removed HDR / bloom / tuning-panel stack or rewrite the baseline shader in-place without first preserving this version behind a small, explicit switch or an isolated follow-up commit. If a polish experiment fails, reverting that experiment should return exactly to this M10.8 baseline.

The full archaeology of the M10.5h–p shader ladder that was rolled back to produce this baseline lives in [`docs/SHIPPED.md` §M10.5h → §M10.5p](../history/SHIPPED.md) and [§M10.8](../history/SHIPPED.md). The Rust-side `OnsetDecimator`, `BeatGrid`, and `FilteredDecimator` data primitives remain in place as dormant building blocks for future additive consumers.

#### 9.6.1 Sizing

The zoomed column is **deliberately slim**. Scratch DJs need vertical *time-history* much more than they need horizontal *peak-detail* — a clean, narrow strip reads faster at performance distance than a wide one and leaves room for the overview, the deck-header chips, and the M10.7 Stillpoint aid in the centre gutter. Concretely, in the SwiftUI implementation:

| Surface | Dimension | Constant | Notes |
|---|---|---|---|
| **Zoomed column, Performance (Timecode) mode** | ≈ 80 px wide | `DubLayout.deckColumnWidth` | Slim Serato-parity strip after M10.8 waveform dogfooding; keeps kick transients readable while leaving room for overview, centre gutter, and info chips. |
| **Zoomed strip, Prep mode** | ≈ 140 px tall, full-width horizontal | `DubLayout.waveformPrepHeight` | Prep mode is single-deck and uses a horizontal scrolling playing waveform for screenshot/A-B judgement and track prep. |
| **Overview band, Prep mode** | ≈ 60 px tall, full-width horizontal | `DubLayout.deckOverviewHeight` | Whole-track waveform stacked above the zoomed Prep waveform. Same click-to-jump semantics as the vertical overview. |
| **Overview column** (M10.5c) | ≈ 36 px wide, full track top→bottom | `DubLayout.deckOverviewWidth` | Thin strip on the deck's outside edge. Shows the whole track at a glance with a playhead-bracket indicator at the current position. Click-to-jump per §6.1. |
| **Centre gutter** (M10.7) | ≈ 120 px wide | reserved | Stillpoint and nothing else. |

The **remaining horizontal space inside each deck pane** (window-half-width minus the zoomed column minus the overview column minus the centre-gutter share) is reserved for per-deck info chips that don't fit in the deck header — RPM toggle (33 / 45), key-lock indicator, beatgrid-offset readout, time-elapsed-vs-remaining secondary readout. Those are M10.x polish work and not specified individually here; the column-width discipline reserves the canvas they'll be drawn onto.

> **Why not just stretch the waveform to fill the column?**
> Two reasons. First, the waveform strip's *information density per pixel* peaks at the Serato-equivalent ≈ 140 px; past that, each additional horizontal pixel duplicates information already shown in the previous one (the peak data is one-dimensional in time — the *width* of a vertical strip is purely a visual amplitude axis). Second, a fat waveform crowds out everything else in the deck pane, leaving no room for the overview or the info chips; the slim discipline is what makes the rest of the layout possible.

#### 9.6.2 Future additive waveform layers — parked

The M10.8 Serato-parity baseline (§9.6.0) is the line below which all future waveform work must remain *additive and reversible*. Three concrete ideas have been scoped against the target user (scratch / urban DJ on timecode, no programmed hot cues, performance-distance glance only) and explicitly **parked for later**. None are committed to a milestone; each must be re-evaluated when picked up.

1. **Phrase landmarks (lead candidate).** Off-line analyzer (new `dub-segment` crate or `dub-bpm` feature) detects structural boundaries — drop, breakdown, verse-in, outro — from discontinuities in the spectral-flux ODF that `dub-bpm` already produces. Result: small chevrons painted *in the existing reserved gutter beside the waveform*, green for energy-up, red for energy-down. Conservative confidence threshold so the failure mode is "no chevron" rather than "wrong chevron." Cached in the M10.5j sidecar. Differentiator: **no commercial DJ app surfaces phrase boundaries on the playing waveform**, and this fills the cue-point role that scratch DJs on timecode otherwise can't program. Cost: ~3–4 days incl. corpus validation against hip-hop / reggae tracks before exposing the feature.

2. **Ghost waveform of the other deck (cheapest win).** A second low-opacity (≈ 0.15) monochrome render of the *other* deck's `PeakBuffer` painted *behind* the current deck's coloured waveform, aligned by playhead. Makes inter-deck transient alignment visually obvious — the qualitative complement to M10.7's Stillpoint aid. Zero new analysis (reuses existing peak data). One additional Metal draw call per deck; default-off Preferences toggle. Cost: ~0.5–1 day. The natural pairing with phrase landmarks.

3. **Vocal-presence overlay (parked pending §15 ruling).** Heuristic over the existing `dub-spectral` 8-band data (strong stable mid-band harmonics + low onset density inside that band) flags vocal-heavy sections; rendered as a dotted-line texture or local desaturation across the waveform's mid stripe. Useful for hip-hop / reggae blending discipline ("don't put two rappers over each other"). **Boundary case with §15 stems / AI separation** — the proposal is a visual annotation derived from spectral statistics with no separated audio buffer ever produced, but explicit sign-off needed before any code lands. Cost: ~2.5 days if approved.

Guardrail: each of these is a *layer*, not a rewrite. They land behind a Preferences toggle (default off until corpus-validated), they don't touch the M10.8 baseline shader's per-column colour mapping, and they ship one at a time so any regression bisects cleanly back to the M10.8 commit.

### 9.7 Accessibility

- Full keyboard control (any feature reachable without a mouse — see §1 for the mouse-policy nuance).
- VoiceOver labels on all controls (best-effort in v1, full in v1.x).
- High-contrast mode (v1.x).

---

## 10. Tech stack

### 10.1 Workspace

```
dub/                                 # repo / workspace name
├── Cargo.toml                       # Rust workspace
├── crates/
│   ├── dub-engine/                  # Audio graph, transport, mixer, ThruSource, no_std-ish hot path
│   ├── dub-audio/                   # CoreAudio HAL input + output, ringbuf-buffered handoff
│   ├── dub-dsp/                     # rubato, biquads, dub-siren synth, echo-out
│   ├── dub-stretch/                 # Rubber Band FFI wrapper (separate crate for license clarity)
│   ├── dub-io/                      # symphonia-based decoders (everything in RAM, see §4.4)
│   ├── dub-timecode/                # Serato CV02 + Traktor MK1/MK2 decoder (clean-room)
│   ├── dub-thru/                    # Thru-mode source-detection classifier (§5.1.1)
│   ├── dub-bpm/                     # M7.5 — BpmEstimator (offline + streaming drivers, pure-Rust)
│   ├── dub-peaks/                   # M9 — off-RT decimator producing PeakChunk + BandPeakChunk for the renderer (M10)
│   ├── dub-spectral/                # M9.5 — shared FFT + log-band magnitude pipeline (consumed by dub-bpm + dub-peaks)
│   ├── dub-fingerprint/             # Library dedupe (M11b, shipped) + real-record recognition (v1.1). Pure-Rust Chromaprint via rusty-chromaprint.
│   ├── dub-library/                 # SQLite + import adapters
│   ├── dub-controller/              # HID/MIDI abstractions (placeholder in v1)
│   ├── dub-ffi/                     # UniFFI-generated bindings to Swift
│   └── dub-cli/                     # `dub` binary (smoke / play / capture / timecode-deck / scope / calibrate / thru)
├── apple/                           # M0.5 shipped — AppKit + SwiftUI shell
│   ├── project.yml                  # XcodeGen manifest (source of truth)
│   ├── Dub.xcodeproj                # generated, gitignored
│   ├── DubCore.xcframework/         # generated, gitignored — universal Rust static lib
│   ├── Dub/                         # @main AppKit lifecycle + SwiftUI views (bundle id: com.klos.dub)
│   │   ├── DubAppDelegate.swift     # NSApplicationDelegate lifecycle
│   │   ├── MainWindowController.swift # NSWindow holding an NSHostingController
│   │   ├── MainView.swift           # Top-level shell — hosts PerformanceView + Preferences sheet (M10.3)
│   │   ├── DesignSystem/Tokens.swift # Colour / type / spacing tokens — single source of truth (M10.3)
│   │   ├── Performance/             # PerformanceView, DeckHeader, StatusStrip, PhaseDriftView (M10.7), placeholders
│   │   ├── Preferences/             # PreferencesSheet (⌘,)
│   │   └── Waveform/                # Metal renderer + MTKView host (M10-B → M10.4 vertical rotation)
│   └── DubShared/                   # Swift Package wrapping DubCore.xcframework + bindings
├── scripts/                         # M0.5 shipped — Apple toolchain orchestration
│   ├── build-xcframework.sh         # cargo build (aarch64+x86_64) + lipo + xcodebuild -create-xcframework + UniFFI bindgen
│   ├── bootstrap.sh                 # one-shot: build-xcframework + xcodegen generate
│   ├── codesign.sh                  # v1.1 (placeholder)
│   └── notarize.sh                  # v1.1
├── tools/
│   └── rt-audit/                    # Static + runtime check: no alloc on audio thread
├── docs/
│   ├── PRD.md                       # ← this file
│   ├── README.md                    # Routing guide: which doc to load for which task
│   ├── SHIPPED.md                   # Full shipped design history; load by anchor
│   ├── ARCHITECTURE.md              # How the crates fit together
│   ├── LIBRARY-SCHEMA.md            # Public SQLite schema contract
│   ├── LICENSE-DEPENDENCIES.md      # Dependency license + attribution inventory
│   ├── UI-BACKLOG.md                # Deferred SwiftUI/AppKit bugs and polish
│   └── LIBRARY-FORMATS.md           # Field notes on Serato / Traktor / rekordbox / iTunes / Lexicon parsing
└── README.md
```

**Notes on the tree:**

- **`dub-bpm`** (shipped in M7.5) hosts the BPM estimator. The current implementation is pure-Rust (spectral-flux ODF + harmonic-summed autocorrelation, see [`docs/SHIPPED.md`](../history/SHIPPED.md)) so the crate has no system or FFI dependencies. If a future `aubio-rs` backend is added it will live behind a feature flag on this same crate, keeping any LGPL dynamic-link concern contained. Both the M7.5 offline driver (file-side fallback when imported metadata has no BPM, §8.3) and the M8 streaming driver (Thru-side live tracking, §5.2.3) build on the same `BpmEstimator` core.
- **`dub-thru`** is reserved for the **source-detection classifier** (§5.1.1's per-deck state machine that decides Timecode vs. Thru from a sliding window of input audio), not the Thru passthrough itself — that lives in `dub-engine` as `ThruSource`. The split exists because Thru *playback* is on the audio thread (and shipped in M7 in `dub-engine`), but Thru *detection* runs on a worker thread off-RT.
- **`apple/`** shipped with M0.5 — XcodeGen-managed `Dub.xcodeproj`, UniFFI-based `dub-ffi` (proc-macros, no UDL), and the AppKit-+-SwiftUI smoke screen. `scripts/build-xcframework.sh` + `scripts/bootstrap.sh` are the only entry points; everything else is gitignored. Distribution signing (a notarization-ready `codesign.sh`) is a separate post-M10.2 milestone.
- **ADRs** are not currently used. Significant design decisions live as commentary in `SHIPPED.md` and `ARCHITECTURE.md` instead; if ADRs prove valuable later they'll land as `docs/adr/`.

### 10.2 Key dependencies

| Crate | Purpose | License | Notes |
|---|---|---|---|
| `coreaudio-rs` | macOS audio I/O | MIT/Apache | Direct HAL access |
| `symphonia` | Decoding | MPL-2.0 | All formats incl. ALAC |
| `rubato` | Resampling | MIT | Sinc-based, FixedOut variant |
| `rubberband` (FFI) | Time-stretch / key lock | **GPLv3** | Forces whole project to GPL — accepted. |
| `aubio` (FFI) | Beat detection (fallback) + live tempo tracking on Thru — *not currently used* | **LGPL-3.0** | M7.5 shipped a pure-Rust baseline (see [`docs/SHIPPED.md`](../history/SHIPPED.md)). Aubio is parked as a future opt-in feature backend on `dub-bpm`; if added it would be dynamically linked and confined to that single crate. |
| `rusty-chromaprint` | Audio fingerprinting (library dedupe M11b, real-record recognition v1.1) | MIT/Apache | Pure-Rust port of Lukáš Lalinský's Chromaprint algorithm (algorithm 2). M11b chose this over an FFI wrapper around the reference C library (`chromaprint`, LGPL-2.1) for the same reasons `dub-bpm` chose pure-Rust over aubio at M7.5: license isolation, no C build dep, no unsafe FFI surface, simpler distribution. Our use case is library-internal similarity-based dedupe, not AcoustID database lookup — cross-implementation bit-identity is not a requirement. The Chromaprint algorithm parameters Dub uses are documented in `docs/LIBRARY-SCHEMA.md` so the schema stays implementation-portable. |
| `ringbuf` | Lock-free SPSC | MIT | RT-safe |
| `crossbeam` | Concurrency primitives | MIT/Apache | Off-RT only |
| `assert_no_alloc` | RT-safety check (dev + release) | MIT | Aborts if alloc on RT thread |
| `rusqlite` | Library DB | MIT | Bundled SQLite |
| `serde` + `quick-xml` | NML / iTunes XML / rekordbox XML | MIT | |
| `id3` + `metaflac` | Tag reading + Serato GEOB | MIT/MPL | |
| `hidapi-rs` | HID (placeholder, v1.x+) | BSD | |
| `midir` | MIDI (placeholder) | MIT | |

**Testing-only dependencies** (per §2.2):

| Crate | Purpose | License |
|---|---|---|
| `proptest` | Property-based testing | MIT/Apache |
| `insta` | Golden / snapshot testing | Apache |
| `criterion` | Microbenchmarks with regression tracking | MIT/Apache |
| `cargo-fuzz` | Coverage-guided fuzzing | MIT/Apache |
| `cargo-llvm-cov` | Coverage reporting | MIT/Apache |
| `cargo-nextest` | Faster test runner | MIT/Apache |
| `mockall` | Mocks for hardware abstractions | MIT/Apache |

Apple side: `swift-snapshot-testing` for SwiftUI snapshot tests.

### 10.3 Apple frontend

- **SwiftUI** for the chrome (toolbar, library, preferences).
- **AppKit** + **MTKView** for the performance surface (waveforms, scopes).
- **UniFFI**-generated Swift bindings into the Rust core for state queries and command dispatch. The audio render callback is set up entirely on the Rust side; Swift doesn't touch it.
- **Combine** for binding observable engine state (transport position, peak meters) to SwiftUI views, polled at 60 fps from the lock-free state snapshots.

### 10.4 Build & CI

The CI pipeline encodes the §2.2 quality bar. Every PR runs through the following GitHub Actions workflow:

**Per-PR (blocking):**
1. `cargo fmt --check` — formatting
2. `cargo clippy --all-targets -- -D warnings` — lints, treated as errors
3. `cargo nextest run` — full test suite (unit, property, golden, integration, RT-safety)
4. `cargo llvm-cov` — coverage report; PR fails if non-trivial modules drop below 85 %
5. `cargo bench --no-run` — benchmark compile check
6. RT-audit assertion: any test that triggers RT-thread allocation fails the build
7. Build matrix: `aarch64-apple-darwin` + `x86_64-apple-darwin`
8. Apple side: `xcodebuild test` for snapshot tests

**Per-merge to main (blocking the release pipeline):**
9. xcframework artifact build (universal: aarch64 + x86_64)
10. Apple `Dub.app` build & test
11. Tag-based release artifact assembly

**Nightly (non-blocking but tracked):**
12. Soak tests: 1+ hour offline render with synthetic timecode and FX rotation
13. Fuzz runs: ≥ 30 minutes per parser fuzz target
14. Benchmark history pushed to a tracking dashboard

**Branch protection on `main`:**
- No direct pushes
- Required: green CI, ≥ 1 review (when team grows beyond 1; until then, author self-review with a 24h cool-off for non-trivial changes)
- Required: linear history (rebase merges, no merge commits)

Notarization via `notarytool` arrives in **v1.1** (M22) once Developer ID is acquired.

---

## 11. Distribution & licensing

- **License:** GPLv3, top-level `LICENSE` file.
- **Distribution:** GitHub Releases. Notarized DMG. Apple Silicon + Intel universal binary.
- **No Mac App Store** in v1 (sandboxing breaks USB HID access for v1.x controller plans, and the Phase RF/SDK story for v2 is hostile to MAS).
- **Source:** public on GitHub from day one.
- **Funding model:** TBD (not part of v1 scope). Could be Patreon / GitHub Sponsors / paid commercial license dual-tier later. Listed as a v2+ open question.

---

## 12. Milestones

This section is for planning. Detailed shipped implementation history lives in
[`SHIPPED.md`](../history/SHIPPED.md); load that file by anchor only. Each planned
milestone keeps a **demo criterion**: one sentence describing what the user can
observably do at the end.

### 12.0 Shipped index

| Range | Scope | History |
| --- | --- | --- |
| **M0 → M8.1** | Workspace, audio engine, transport, timecode, external routing, Thru, BPM. | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M9 → M10.8** | Live peaks, spectral extraction, Metal waveform, Apple shell, Performance/Prep mode, Panic Play, Stillpoint (beatmatch aid), waveform baseline reset. | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11a → M11d.4** | Library schema, fingerprint dedupe, filesystem importer, browser shell, smart crates, indicators, scanner, Relocate. | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.1 → M11c.2** | Lazy auto beat-grid analysis and Camelot key detection. | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.3a** | BPM octave fix (perceptual tempo prior — rap at 95, DnB at 172). | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.3c** | Reggae skank double-time rejection (Default profile pass 2). | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.3d** | Genre-aware octave profile for library analysis (ID3 → `OctaveProfile`). | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.3e** | Hip-hop double-time rejection (rap corpus 19/19 on Default). | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.3f** | FourOnFloor profile + Dub mid-band fix (house/dubstep library). | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.3b** | Tap-to-grid (`G` / `Shift+G` / `Option+G`) + in-place deck grid install. | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11c.4** | Lazy fingerprint (deferred from import to first deck-load). | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11d dogfooding rounds** | Performance-mode play fixes, library-sourced beat grids, waveform/beat-grid smoothness, library UI reliability. | [`SHIPPED.md`](../history/SHIPPED.md) |
| **M11d.6 → M11d.7** | Full-screen launch + windowed snap-back; off-main-thread waveform rendering; beatgrid precision + auto downbeat + drift lock (schema v4 `grid_locked`). | [`SHIPPED.md`](../history/SHIPPED.md) |
| **PRD-BEATS hardening** | Uniform Traktor-style grid + calibration; tap-to-grid + explicit `bar_phase` (schema v5) + relatch "set the 1"; robustness rounds 5–10 (`OctaveProfile::HipHop`/`DrumAndBass`, integer-snap safety net) + `dub diagnose`; waveform/beat-grid jitter killed end to end. | [`PRD-BEATS.md`](PRD-BEATS.md) |
| **M11d-next** | Manual crates (playlists, §8.5.1): create / inline-rename / delete (cascade) / drag-add / remove / drag-to-reorder + context-menu reorder; `crates` + `crate_tracks` CRUD + ordering, FFI 29, editable "Dub Crates" sidebar. A `#` manual-order column (`crate_ordinal` on the row) is the crate's default sort and the only state where reorder is enabled; other column sorts render a read-only view, leaving manual order composable into future multi-column sorting. Nested crates deferred. | §8.5.1 |
| **M11d-history** | Played From / Played Into, v1.0 stage (§8.5.2): `SessionTracker` handover-inferred transitions (min-play gate + duplicate suppression, `LIBRARY-SCHEMA.md`), full `play_history` event capture (load / play_start / play_end / transition pair, `session_id` per app run), `played_into` + `session_history` queries, FFI 37, deck-header "↝ usually" hint (§9.5 row 3), Session History smart crate with "← from" annotations. | §8.5.2 |

### 12.1 Planned path to v1.0

| # | Name | Demo criterion | Estimate |
| --- | --- | --- | --- |
| **M11e** | **Serato importer** | Read Serato DB/crates/GEOB tags, populate source metadata, beatgrids, keys, imported crates, cues, and loops. | 4–5 days |
| **M11d-columns** | **Customizable browser columns + per-source disagreement view** | User can choose visible columns, inspect source-specific metadata, and see analysis/source disagreements without paying query cost for disabled columns. | 3–4 days |
| **M11f** | **Export: rekordbox XML + M3U / M3U8** | Export a Dub crate and round-trip it through a fresh import with canonical identity, cues, loops, and grids intact. | 3 days |
| **M12a** | **rekordbox XML importer** | Read user-exported rekordbox XML; DB6 `master.db` remains v1.1 scope. | 2 days |
| **M12b** | **Traktor NML importer** | Read `collection.nml` and populate metadata, beatgrids, and imported playlists. | 2 days |
| **M12c** | **iTunes XML importer** | Read `Library.xml` for playlists and ratings; BPM is expected to be sparse. | 1–2 days |
| **M12d** | **Lexicon path documented** | No code: document Lexicon → Serato / rekordbox / Traktor export paths in `LIBRARY-FORMATS.md`. | 0.5 day |
| **M13** | **Looping** | Manual and auto-loop, halve/double, behave correctly under timecode. | 4–6 days |
| **M14** | **Key Lock + auto-bypass** | Rubber Band integrated per deck, with scratch-aware auto-bypass. | 1 week |
| **M15** | **Smart FX: Echo-Out** | Tap-and-hold echo-out works on both decks, including Thru decks. | 4–5 days |
| **M16** | **Smart FX: Dub Siren** | Dub siren synth + delay + reverb are keyboard-controllable. | 3–4 days |
| **M17** | **Sampler + Quick Scratch + Instant Doubles** | All three trigger systems work per §7. | 4–6 days |
| **M18** | **Polish + Alpha** | Calibration UX, preferences, key remapping, dark-mode polish, and manual rig checklist are ready for 3–5 trusted DJs. | 2–3 weeks |
| **M19** | **Beta** | Public opt-in beta on GitHub Releases; feature-frozen for v1.0 with hotfix discipline active. | 2–4 weeks, gated by gig time |
| **M20** | **v1.0 Stable Release** | §2.2.6 SLOs met, DMG published, README/docs/demo ready. | 3–5 days once SLOs pass |

**Aggregate:** still approximately 20–26 weeks of focused work for v1.0,
including beta-gated promotion. The Beta → Stable gap is deliberately variable:
we ship Stable when the SLOs are met, not on a calendar.

### 12.2 v1.1 (post-launch follow-up, ~6–8 weeks after v1.0 stable)

| # | Name | Demo criterion |
|---|---|---|
| **M21** | **Fingerprint recognition** | Chromaprint integrated; first play of a real record captures fingerprint; second play recognizes within 5–10 s and loads saved waveform. |
| **M22** | **Persistent waveform learning** | Captured Thru waveforms persist to library DB; rendered immediately on recognized records. |
| **M23** | **Code signing + notarization** | Apple Developer ID acquired; notarized DMG; auto-update mechanism. |
| **M24** | **Beatgrid editor** | Full grid editing UX (drag downbeat, nudge BPM, halve/double, taps). |
| **M25** | **Opt-in crash reporting** | Sentry (or similar) integration with explicit user toggle, redaction of file paths, per §2.2.7. |

---

## 13. Risks & open questions

### 13.1 Technical risks

| Risk | Severity | Mitigation |
|---|---|---|
| Timecode quality on cheap interfaces | High | Test matrix from day one (SL3, Audio 6, generic class-compliant); document supported interfaces. We have both reference rigs in-house. |
| Rubber Band CPU at 2 decks + key lock + active playback | Medium | Profile early (M14). Have a lower-quality fallback flag (`R3` engine off, use `Faster` engine). Scratch-aware auto-bypass (§6.1.1) reduces total Rubber Band load substantially during real DJ use. |
| Auto-BPM accuracy on dub / minimal genres (sparse beats, half-time feels) | Medium → Low | **First-line:** M7.5's offline driver lets us evaluate the BPM engine against a fixture corpus of target genres on the bench (`cargo test`) before risking it on live audio. M8.1's log-band ODF + windowed-energy picker resolved the synthetic-fixture half-tempo / double-tempo cases (reggae 65, hip-hop 90/100, rolling dnb 174); see [`docs/SHIPPED.md`](../history/SHIPPED.md). M11c.3a's perceptual tempo prior extends the resolution to real catalogs, fixing the symmetric hip-hop at 2× (95 → 190) and DnB at 1/2× (172 → 86) failure modes that the synthetic-only M8.1 calibration missed; see [`docs/SHIPPED.md`](../history/SHIPPED.md). **Second-line (already shipped):** `BpmRange` escape hatch (`dub thru --bpm-range MIN,MAX`, `analyze_bpm_with_range(samples, sr, ch, range)`) constrains the search to a user-chosen window for the irreducibly-ambiguous genres (dubstep 140 / 70, reggae one-drop 65 / 130, slow soul) the algorithm cannot resolve without a prior. **Third-line (shipped):** tap-to-grid (M11c.3b) lets the DJ override the auto-detected BPM on a per-track basis with one keystroke (`G` / `Shift+G` / `Option+G`); see [`docs/SHIPPED.md`](../history/SHIPPED.md). **Fourth-line (future):** real-music validation can still motivate an `aubio-rs` feature backend on `dub-bpm` if a class of tracks falls outside both the algorithmic gate and the manual escape hatch — but the M8.1 + M11c.3 architecture has reduced this risk from the "blocking" level we started at. |
| Chromaprint robustness to turntable pitch drift / mixer EQ (v1.1) | Medium | Validate during v1.1 with real-world test corpus. Fall back to Shazam-style constellation hashing if Chromaprint underperforms. |
| Thru latency perceived as "feel different" by sensitive scratch DJs | Low–Medium | Hold latency below the ~5 ms scratch-imperceptibility threshold (PRD §6.1) with a 64-frame buffer / 48 kHz path. Keep it *constant* across FX state (Option A in-chain FX bypass, §5.2.1 / §5.2.2) so the DJ internalises one timing relationship for the whole set instead of one per FX engage. Document the trade-off; if hardware Thru is required, the operator uses the interface's physical button (which trades away BPM/waveform/FX for zero latency). |
| rekordbox DB6 format changes | Medium | Always offer XML-export path as fallback. |
| CoreAudio aggregate device weirdness | Medium | Document recommended interface configs. SL3 and Audio 6 both don't need aggregation. |
| Notarization / code-signing setup | Low | Defer to v1.1 (M22). v1.0 ships unsigned-with-instructions per current decision. |
| GPL incompatibility with future commercial plans | Medium | Explicit decision: GPL for now, revisit at v2. Rubber Band commercial license = ~£600 one-time when/if needed. |
| SL3 discontinued by Serato | Low | Class-compliant on macOS, works fine. We test against it but recommend the Audio 6 (or successors) as the reference modern interface in our docs. |

### 13.2 Open questions (to resolve during development)

1. **Sample bundling decision** — defer until UX is testable.
2. **Dub-siren dedicated FX-bus output** — cheap to add; decide during M16.
3. **Auto-gain default**: LUFS-I (musically correct) or peak (predictable for scratch DJs)? — A/B test with users in beta.
4. **Beatgrid editor UX** — minimal in v1, full pass in v1.1 (M24).
5. **Funding model for sustainability post-v1** — out of scope for PRD; revisit before v1 release.
6. **Brand identity** (logo, marks) — not in v1 PRD scope.
7. **Alpha tester recruitment** — need 3–5 trusted DJs willing to run pre-release on real gigs. Author's network presumably covers this; revisit at M17.
8. **CI runner for nightly soak/fuzz** — GitHub Actions has limits; may need a self-hosted runner (e.g., a spare Mac mini) once nightly soak exceeds free CI minutes. Defer the decision until soak is actually running into the limit (Phase B, M18 onward); through Phase A the per-PR pipeline fits comfortably.

### 13.3 Items explicitly deferred to v2

- Phase support (full subsystem, including SDK access and integration)
- Recording (master out + per-deck)
- Windows port
- HID controller ecosystem
- Stems (probably never)

### 13.4 Items explicitly deferred to v1.x

- Saved loop slots (M11 schema includes empty `track_loops` table for forward-compat)
- Sampler expansion 4 → 6 slots (if real use demands)
- Stillpoint numeric-only Preferences variant (§9.4)
- Track Preparation Mode prep tooling (beatgrid editor, gain UI) — *hot-cue authoring shipped early (§6.2.1); beatgrid editor + gain UI remain v1.x*
- Additive waveform layers parked in [§9.6.2](#962-future-additive-waveform-layers-parked) — phrase landmarks, ghost waveform of the other deck, vocal-presence overlay. None scheduled; each must be re-evaluated against the M10.8 baseline guardrail (§9.6.0) when picked up.

---

## 14. Acceptance criteria for v1.0

Dub v1.0 ships when **all** of the following hold on a DMG installed on a clean macOS 14+ machine:

1. Scratch DJ can plug a class-compliant 4-in/4-out USB interface (test rig: SL3 or Audio 6), route timecode inputs to In 1/2 and 3/4, route outputs to their hardware mixer, and have both decks under timecode control with **< 5 ms latency at 64-sample buffer, < 10 ms total timecode-to-audio response.**
2. Both Serato CV02 and Traktor MK2 vinyl supported.
3. Either deck can be set to **Thru** (real record routes through the engine, ~2.7 ms one-way latency, FX-capable). Hardware bypass on the interface itself is outside Dub's scope — see §5.2.2.
4. **Auto-BPM detects tempo of a real record played in Thru mode within 15 seconds, with a confidence indicator that distinguishes "tentative" from "locked".**
5. **Live waveform of a Thru-mode real record is rendered as the record plays, at 60 fps, with no glitches.** (Persistence and recognition land in v1.1.)
6. Echo-Out and Dub Siren can be applied to a Thru deck (i.e. FX work on real records); engaging FX does not change the deck's input-to-output latency.
7. User can import their existing Serato / Traktor / rekordbox / iTunes / Lexicon library and play tracks with imported beatgrids. Auto-detect grids fall back when source has none.
8. Looping (reverse-loop with beat-length select + halve/double, per §6.2) works correctly under timecode. *(Ships internal-play today; timecode-correct looping is the open part of this gate.)*
9. **Key Lock works on both decks; engages and disengages automatically based on playback rate per §6.1.1; user hears no glitches during scratching with Key Lock on.**
10. Echo-Out, Dub Siren, Sampler (4 slots), Quick Scratch (4 slots, hotkey fast-load), Instant Doubles all work per §6 / §7.
11. UI is keyboard-navigable end-to-end. **No performance gesture** (pitch / scratch / crossfade / EQ / gain / cue) requires the mouse — per §1's refined mouse rule. Mouse-driven *transport* (Panic Play, Casual Play, position navigation per §6.1) is in v1 and *not* in conflict with the philosophy.
12. **Panic Play (§6.1.2)** recovers from a needle dirt event without audible interruption: keystroke transitions the deck from timecode-driven to last-known-velocity playback, audience hears no glitch, automatic resume on clean LFSR return verified in a manual rig test.
13. **Stillpoint (§9.4)** renders in the centre gutter at 60 Hz with ≤ 1 frame of stutter, drifts when tempos differ and freezes / seats the band on the line when matched, and certifies lock honestly (no false green). Verified against its Swift test suite (`apple/DubTests/`).
14. **Track Preparation Mode shell** (M10.8) auto-boots when no multi-channel interface is connected; can load + play a file from the library at horizontal-waveform resolution.
15. Zero xruns in a 60-minute scratch session at 64-sample buffer on M2 Air.
16. README + first-run experience documents how to set up a typical rig (turntables → interface → mixer → speakers) and a Thru-mode rig (real record → interface → engine → mixer).
17. **All §2.2.6 reliability SLOs met**: zero crashes in 100 cumulative beta-gig-hours; zero xruns in soak; zero RT-thread allocations; zero fuzz crashes in last 7 days; no benchmark regressions; manual rig checklist (§2.2.10) signed off.

---

## 15. Out of scope for v1 (reaffirmed)

- Internal mixer mode (user-facing)
- Mouse-driven **performance gestures** (pitch / scratch / crossfade / EQ / gain / cue). Mouse-driven transport (Panic Play, Casual Play, position navigation) is **in scope** — see §1 for the rule, §6.1 for the surface.
- Hot cues (entirely deferred to v2 — no v1 "lite" version)
- Saved loop slots (deferred to v1.x — v1 ships ephemeral loops only)
- Sampler beyond 4 slots (v1 is 4; expansion to Serato-parity 6 deferred to v1.x if real use demands it)
- Track Preparation Mode editing tooling — beatgrid editor, hot-cue prep, gain tweak (v1 ships the *shell*; tools land in v1.x)
- Stillpoint numeric-only Preferences variant (single design in v1; alternative ships in v1.x if needed)
- Recording
- Streaming services
- Phase
- HID controllers
- Audio fingerprint recognition / persistent waveform learning (planned v1.1)
- Code signing & notarization (planned v1.1; v1.0 ships as unsigned dev DMG)
- Auto-update mechanism (planned v1.1)
- Stems / AI
- Video / visuals
- Cloud
- Mobile
- Windows
- Mac App Store
- Localizations beyond English

---

*End of document. v0.1 pre-alpha working spec. For shipped implementation history, see [`SHIPPED.md`](../history/SHIPPED.md); for docs routing, see [`docs/README.md`](../README.md).*
