# Dub — Architecture notes

> Companion to `docs/PRD.md`. The PRD describes *what* Dub does; this doc
> describes *how* it's structured.

## Overview

Dub is a Rust audio engine wrapped by a native macOS SwiftUI/AppKit shell.
The Rust core owns the audio thread end-to-end; Swift owns the UI thread
end-to-end. They communicate via lock-free state snapshots and SPSC ring
buffers, never callbacks across thread boundaries.

```
┌────────────────────────────────────────────────────────────────────┐
│                          macOS process                             │
│                                                                    │
│  ┌─────────────────┐         ┌──────────────────────────────────┐  │
│  │   SwiftUI/      │  UniFFI │           Rust core              │  │
│  │   AppKit shell  │◀───────▶│                                  │  │
│  │                 │  (lock- │  ┌────────────┐  ┌────────────┐  │  │
│  │  - Library UI   │   free  │  │  Engine    │  │ Library DB │  │  │
│  │  - Decks UI     │  msgs)  │  │  graph     │  │ (SQLite)   │  │  │
│  │  - Waveforms    │         │  │            │  └────────────┘  │  │
│  │    (Metal)      │         │  │  Decks     │  ┌────────────┐  │  │
│  │  - Preferences  │         │  │  FX        │  │ Track DBs  │  │  │
│  └─────────────────┘         │  │  Sampler   │  │ (in-RAM)   │  │  │
│                              │  └─────┬──────┘  └────────────┘  │  │
│                              │        │ render(rt, out)          │  │
│                              │        ▼                          │  │
│                              │  ┌─────────────────────────────┐  │  │
│                              │  │  CoreAudio AU IO proc       │  │  │
│                              │  │  (audio thread, RT)         │  │  │
│                              │  └─────────────────────────────┘  │  │
│                              └──────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────┘
```

## Crate dependency graph

```
                      ┌────────────────┐
                      │     dub-cli    │   (binary, smoke harness)
                      └───────┬────────┘
                              │
                              ▼
                      ┌────────────────┐
                      │     dub-ffi    │   (UniFFI surface)
                      └───────┬────────┘
                              │
                              ▼
                      ┌────────────────┐
                      │   dub-engine   │ ─┬─ ringbuf
                      └───────┬────────┘  │
              ┌───────────────┼──────────┐
              ▼               ▼          ▼
         dub-dsp         dub-stretch   dub-thru
         dub-io          dub-timecode  dub-fingerprint  dub-library
         dub-controller  (placeholders for v1+)
```

Only `dub-engine` is on the audio thread. Everything else is either
preparatory work, off-thread workers, or non-RT services.

## RT-safety enforcement

Three layers, in order of strength:

1. **Type system (compile-time):** `RealtimeContext<'_>` is the gating token.
   Any function reachable from `Engine::render` takes `&mut RealtimeContext<'_>`.
   The token is `!Send`, `!Sync`, and lifetime-bounded so it cannot leak.
2. **`assert_no_alloc` (runtime, dev/test):** the global allocator wraps an
   `AllocDisabler`. Tests that exercise the render path run inside
   `assert_no_alloc::assert_no_alloc(|| { ... })`; any allocation aborts.
3. **`assert_no_alloc` (runtime, release):** same allocator, configured to
   set a flag and emit a one-shot log entry rather than abort. Protects
   production users while making dev-time violations loud.

See PRD §2.2.3 and `crates/dub-engine/src/realtime.rs`.

## Audio I/O

- macOS only in v1.
- CoreAudio HAL via `coreaudio-rs`. Direct device-property listeners; opt-in
  hog mode for the lowest-latency path.
- `AVAudioEngine` is **not** used (too high-level, hides the IO proc).
- Per-deck input + output assignment in External Mixer mode (PRD §5.3).

### HAL input invariant — sample-rate match (M5.2)

CoreAudio HAL has a load-bearing footgun: if the AudioUnit's stream
format SR does **not** equal the device's hardware nominal SR, the IO
proc silently delivers zero callbacks. `AudioUnitStart` returns OK,
`coreaudiod` logs nothing, the green mic indicator never lights up.
You will think it's a TCC permission issue. It isn't.

`AudioInput::start_with_options` enforces the invariant by:

1. Reading `kAudioDevicePropertyNominalSampleRate` directly off the
   device (not via `AudioUnit::sample_rate()` — a fresh HALOutput AU
   reports its own internal default, 44.1 kHz, regardless of hardware).
2. If the caller asked for a different SR, calling
   `set_device_sample_rate` on the device first (synchronous; blocks
   until the HAL rate listener confirms).
3. Building the AudioUnit *uninitialized*, setting the stream format
   on `(Scope::Output, Element::Input)` to match the now-actual device
   SR, then calling `AudioUnitInitialize`.

Reverse order — initialize, then set format — appears to succeed but
sometimes leaves the IO proc unarmed. Set-then-init is the only
robust sequence.

`list_input_devices` and `query_default_input` likewise report the
device's hardware nominal SR, so the user-visible rate matches the
rate at which input will actually fire.

#### Cold-start capture overshoot — known issue, deferred

Empirically, the *first* `dub capture` against a freshly-opened SL3
records ~1–3 s more audio than wall time accounts for; subsequent
captures within the same process are exact (15.003 s wall ⇒ 14.997 s
audio observed). The decoder still locks at confidence 1.000 across
the entire capture and rate is correct, so the file is real audio, not
duplicated samples — the IO proc simply runs ahead of nominal for the
first second after `AudioUnitStart` on this driver. Levels mode never
sees this because it doesn't write a WAV (the file create was the
suspected trigger; the actual mechanism is undiagnosed). For M5.3 the
deck consumes samples directly off the input ringbuf and never
correlates input-sample-count with wall time, so the issue is invisible
to the live integration. Re-investigate when we add input-clock-vs-
output-clock drift compensation in M5.4+.

## Audio buffers

Per PRD §4.4:

- Tracks are decoded fully into RAM on load. No per-block disk streaming.
- Audio is `Arc<[f32]>`, planar stereo, 32-bit float.
- A 6-minute FLAC ≈ 140 MB at f32; two loaded decks = ~280 MB.
- Forward and backward playback are byte-for-byte symmetric.

## UI ↔ Engine messaging

Bidirectional, lock-free.

### UI → Engine (commands) — implemented in M2

`ringbuf::HeapRb<Command>` (SPSC, capacity 256). UI pushes, audio thread
pops at the start of each render block. Producer side lives in
`dub_engine::EngineHandle`; consumer side is owned by `Engine`.

- `Command` is a small enum, ≤ 64 bytes, no `Box`, no `dyn Trait`. Most
  variants are `Copy`-equivalent; `DeckLoad` carries an `Arc<Track>`
  by value. Variants today: `DeckPlay`, `DeckPause`, `DeckSeek`,
  `DeckSetRate`, `DeckSetGain`, `DeckLoad`. Adding a command is one
  variant + one match arm in `Engine::apply_command`.
- The drain is RT-safe: `try_pop` is a load + index, and every variant
  applies in-place to the deck array. Verified by `rt-audit` with 100k
  blocks, 10k pre-staged transport commands, and 20 hot-loads, all
  under `assert_no_alloc`.

### Trash channel (audio → UI for `Arc<Track>` disposal) — M3

`ringbuf::HeapRb<Arc<Track>>` (SPSC, capacity 32). The audio thread
NEVER drops `Arc<Track>` — `Arc::drop` decrements the strong count and
calls `dealloc()` if it hits zero. `dealloc` is a syscall, forbidden on
the RT thread.

When the engine applies `DeckLoad`, it `swap_source`s the new Arc onto
the deck and pushes the old Arc into the trash channel. The main thread
drains the channel via `EngineHandle::reclaim()` (called automatically
inside `DeckCommand::load` and on `EngineHandle::drop`).

If the trash channel ever overflows (UI not draining + storm of loads),
the audio thread `mem::forget`s the rejected Arc (leaking it) and
increments an atomic `trash_overflow_count`. Leaking is the lesser evil
versus a forbidden `dealloc` on the RT thread, and the counter surfaces
the contract violation to the UI for logging.

### De-click envelope on transport changes — M3.5

Any instantaneous transport mutation (track load, seek, play/pause)
would change the value the deck reads from one sample to the next.
A jump function in the time domain is, in the frequency domain, a
brief impulse with infinite-frequency content — the ear hears that
as a click.

`crates/dub-engine/src/declick.rs` precomputes a 2 ms equal-power
crossfade table at engine construction (one per engine, shared as
`Arc<DeclickEnvelope>` across decks). At 48 kHz that's 96 samples ×
4 bytes = 384 bytes — sits in L1 cache.

Each `Deck` carries:

- `declick_envelope: Arc<DeclickEnvelope>` (read-only),
- `declick: DeclickState` (`Idle` or `Active{ prev_source, prev_position,
  prev_rate, prev_playing, samples_remaining }`),
- `pending_disposal: Option<Arc<Track>>` for back-to-back swaps.

Mutators that change what the deck reads (`set_source`, `swap_source`,
`set_position_frames`, `set_playing` on transition, `clear_source`)
all call `start_declick`, which snapshots the *current* state into
`Active{prev_*}` before the caller mutates `self`. The render loop
then runs two phases per block:

1. **Fade phase** (while `samples_remaining > 0`): per sample, read
   `(old_l, old_r)` from `prev_source` at `prev_position` and
   `(new_l, new_r)` from `self.source` at `self.position`. Mix
   `out = old · (1 − fade_in[i]) + new · fade_in[i]` where
   `fade_in[i] = sin²(i · π/(2N))` is read from the envelope table.
2. **Steady phase**: normal additive interpolation, identical to the
   M2 render path.

The audio thread never drops `Arc<Track>`. After every render block
the engine sweeps each deck for finished ramps and `pending_disposal`
slots and ferries any orphaned `Arc<Track>` through the trash channel
(§Trash channel above). Back-to-back transport changes within a single
2 ms window stash one displaced Arc in `pending_disposal`; in the
≥4-deep edge case (physically impossible from human input) we
`mem::forget` and increment the same overflow counter the trash
channel uses.

**Tail-fade**: complementary primitive sharing the same envelope. The
transport declick fires on user-initiated state changes; it does not
fire when the playhead simply walks past the last sample of a track
(that's the data running out, not a transport mutation). Without a
tail-fade, the deck reads "last in-range value, then zero" in one
sample — a step function the ear hears as a click. The `track_tail_fade_scale`
helper applies `cos²` over the last `N` frames of every track read,
on both the steady-state path and inside the M3.5 crossfade's old/new
sides. Gated by a `track_len ≥ 2 × envelope_length` threshold so
sub-millisecond test tracks aren't obliterated.

Verification: 7 declick + tail-fade unit tests cover fade-in monotonicity,
fade-out to silence on pause, A→B crossfade smoothness, no-jump bound
on per-sample deltas, back-to-back-swap Arc accounting, end-of-track
smoothness, and the short-track skip threshold. `rt-audit` exercises
100k blocks with 20 hot-loads each producing a 2 ms fade, all under
`assert_no_alloc`, with zero overflows.

**End-to-end audit**: subjective listening is a poor debug loop for
clicks, so M3.5 also ships a `dub analyze <wav>` subcommand that
reads any 32-bit-float (or 16-bit PCM) WAV and reports peak, RMS,
DC offset, clipping count, and the maximum per-sample first-difference
per channel, flagging samples where `|s[i] − s[i-1]|` exceeds a
configurable threshold (default 0.05). The offline `dub play -o`
path supports the same scheduled transport events as realtime, so a
hot-swap scenario can be rendered deterministically and audited
mathematically — current measured worst-case delta on the M3.5 demo
suite is 0.0187, against a click step of order 0.5+.

### Timecode decoder, relative-mode-only — M5.1

Lives in `dub-timecode`. Pure DSP, no I/O, no allocations on the hot
path — designed to drop straight onto the audio thread when M5.3 wires
it up to live audio input.

**Signal model.** Both stereo channels carry the same nominal sinusoid
at the format's carrier (1 kHz for Serato CV02), offset by 90° between
ch0 and ch1. The convention — verified empirically against a real
Serato Control CV02 cartridge through an SL3 — is `ch0 ≈ A·sin(φ)`,
`ch1 ≈ A·cos(φ)`, with ch0 *leading* ch1 by 90° at forward play.
Treating each frame as a complex sample `s = ch1 + j·ch0` makes the
input a single complex exponential `s(t) = A · exp(j·2π·f·t)` whose
frequency is positive when the record turns forward and negative when
reversed. Magnitude `|s|² = ch0² + ch1²` is constant across rotation,
which is what makes amplitude AGC unnecessary for the *phase* tracking
(it'll matter later for AM-bitstream decoding in M6).

The synthetic generator in `dub-timecode::signal` emits the same
quadrature convention so round-trip tests, the `dub decode-timecode`
`--synthetic` mode, and live SL3 captures all share one sign convention:
**forward stylus motion ⇒ +rate, reverse ⇒ −rate**. Getting this wrong
in M5.1 would have looked perfectly reasonable on synthetic data
(generator and decoder would have been internally consistent); only the
first capture from real hardware exposed the channel ordering, which is
why we delayed picking the convention until empirical data was in.

**Per-block algorithm.**

```text
  for each stereo frame n:
    s_n = ch1_n + j·ch0_n                            # Serato CV02 quadrature
    accum  += s_n * conj(s_{n-1})
    amp_acc += |s_n|²
  Δφ_block = arg(accum)                              # coherent phase diff
  f_inst   = Δφ_block / (2π · Δt_per_sample)         # signed Hz
  rate     = f_inst / carrier_hz                      # ±1.0 = ±unity
  position += rate * block_seconds                   # seconds at unity
  confidence = |accum| / amp_acc                      # 1.0 = pure carrier
```

The coherent sum is the key to robustness: noise (uncorrelated across
samples) suppresses by `√N`, signal adds linearly. With a 64-sample
block at 48 kHz that's a ~9 dB noise gain — easily good enough to lock
onto a real cartridge, and orders of magnitude better than per-sample
phase tracking (which is what naive PLLs do).

Direction falls out for free: forward rotation → `f_inst > 0`, reverse
→ `f_inst < 0`. No state machine, no quadrature flag, no zero-crossing
parity tracking. The L/R quadrature relationship of the printed signal
is the only direction encoding we need.

**Limits.** Per-sample phase advance saturates at ±π, which puts a
`Nyquist / carrier = 24×` ceiling on trackable rates at 48 kHz / 1 kHz.
Real DJ scratching tops out at ~8×, well clear. Below that limit the
estimator is bias-free and limited only by sample-rate quantization
(~50 µs at 48 kHz, equivalent to ~0.005 of unity rate).

**What's *not* here yet.** Absolute position (M6 — needs bitstream
demod and the format's 20-bit code table), stickiness policy (M5.4 —
"confidence dropped below threshold for N ms → freeze deck" lives in
the integration layer, not in the DSP), and AGC + cartridge
calibration (M6 — real-world amplitude variation). The decoder
exposes `confidence` and `amplitude` so the integration layer can
implement those policies without modifying the DSP.

**License + provenance.** Clean-room implementation from the
xwax/Mixxx algorithm description; no xwax code copied (xwax is BSD;
dub is GPL-3.0 — the *direction* of compatibility allows BSD → GPL,
but we want attribution to remain unambiguous, hence the rewrite from
spec).

### Live timecode → deck — M5.3

This is where the offline decoder (M5.1) and the input plumbing
(M5.2) meet the engine. The integration is intentionally narrow:
one new module (`dub_engine::timecode`), one new method
(`Engine::attach_timecode_input`), one new render-loop step
(`Engine::drive_timecode_inputs`). No new threads, no new channels,
no extra IPC.

**Wiring.**

```text
  CoreAudio input IOProc                       AudioOutput callback
  (e.g. SL3 ch3+4, 48 kHz)                     (default device, 48 kHz)
           │                                            │
           ▼                                            ▼
  HeapRb<f32> (1 s capacity)                    Engine::render
           │  (consumer moved into engine               │
           │   via AudioInput::take_consumer)           │
           ▼                                            │
  TimecodeInput { rx, decoder, scratch }                │
           │                                            │
           └────────► drive_timecode_inputs ────────────┤
                          │ (per render block)          │
                          │ pop_slice → Decoder::process│
                          │ → DecodeOutput              │
                          │                             │
                          ▼                             │
                  Intent::Locked { rate }   ──┐         │
                  Intent::DropoutHoldRate    ──┤        │
                                               ▼        │
                                Deck.set_rate / set_playing
                                               │        │
                                               └───►Deck.render
```

The `AudioInput` keeps the AudioUnit alive on the main thread (drop
= stop input). The consumer end of its IOProc → consumer ringbuf
moves into the engine via `AudioInput::take_consumer`, after which
`AudioInput::read_into` returns 0 forever (only one reader on an
SPSC ring).

**Lift policy: amplitude gate + two-edge confidence hysteresis +
sticky window.**

Three iterations on real SL3 hardware drove the design here, each
exposing a class of bug the previous policy missed:

1. *Single-threshold gate.* Confidence wobbles around 0.8 as the
   carrier dies on lift → rapid play/pause toggles → audible
   chatter from repeated 2 ms declick fades.
2. *Two-edge confidence hysteresis (no amplitude gate).* The
   lukewarm `[0.5, 0.8)` band is correct for *scratch* transients
   (cartridge firmly on groove, brief direction reversals) but
   *wrong* for lift: the cartridge picks up handling/rumble noise
   that the decoder finds *some* coherent rotation in (moderate
   confidence) while the RMS is near-zero. The deck stayed
   engaged at `last_locked_rate`, burst-playing track audio for
   as long as the needle was held aloft.
3. *Amplitude gate over confidence hysteresis (current).*
   Amplitude is the truthful "is the cartridge on the groove?"
   signal; confidence alone is not. The gate overrides the
   confidence bands.

```text
  amplitude < amplitude_threshold ─────────── "carrier dead"
      Treated as below-floor regardless of confidence.
      Engaged: counts toward sticky disengage.
      Disengaged: stays disengaged.

  amplitude ≥ amplitude_threshold AND ...
    ┌── conf ≥ engage_threshold ───────────── "fully locked"
    │       set rate = decoded_rate; engaged = true; reset countdown.
    │
    │── disengage_threshold ≤ conf < engage_threshold ── "lukewarm"
    │       if engaged: hold last_locked_rate, stay engaged, reset
    │                    countdown (mid-scratch transients).
    │       if disengaged: stay disengaged (noise floor).
    │
    └── conf < disengage_threshold ─────────── "below floor"
            engaged: increment countdown; disengage when it hits
                     sticky_blocks_to_disengage (deck mutes via
                     M3.5 declick).
            disengaged: stays disengaged.
```

Defaults: engage 0.8, disengage 0.5, sticky 4 blocks (~21 ms @
256-frame / 48 kHz), amplitude 0.01 RMS. CV02 carriers through SL3
sit at 0.1–0.5 RMS; lifted needles drop to <0.005, so the gate has
a wide margin. All four are tunable per attach via
`TimecodeInputConfig`; the CLI exposes `--confidence` (engage),
`--disengage-threshold`, `--sticky-blocks`, and
`--amplitude-threshold`. Setting `amplitude_threshold = 0.0`
disables the gate (confidence-only fallback) — diagnostic only,
pinned by a regression test so we can't lose it.

The factoring deliberately separates `step_policy(DecodeOutput)`
from `drive(...)` (which sources data from the ringbuf) so the
state machine is unit-testable without ringbufs or decoders. The
test suite covers each pathology this policy was tightened to
fix — including the lukewarm-but-quiet lift bug from the second
SL3 validation. M5.4 layers calibration UX (per-cartridge stored
thresholds) and a TUI scope on top; the lift gate itself is
complete.

**RT-safety.** `drive_timecode_inputs` is allocation-free and
finite-time:

- `pop_slice` on the SPSC consumer is a memcpy.
- `Decoder::process` is `assert_no_alloc`-clean (M5.1 verified).
- The scratch buffer is pre-allocated at attach time
  (`max_block_frames × 2` interleaved samples) and never resized.
- `Deck::set_rate` / `set_playing` are field writes plus relaxed
  atomic stores; the M3.5 declick start is alloc-free (verified in
  M3.5).

`rt-audit` carries a 10k-block timecode-driven render path under
`assert_no_alloc` so any future regression on this hot path fails
CI rather than reaching audio threads in the wild.

**SR alignment.** v1 requires `input_sample_rate == engine_sample_rate`
to within 0.5 Hz; mismatch is rejected at attach time
(`AttachError::SampleRateMismatch`). Sample-rate conversion between
input and engine isn't in scope. The output device is *also* aligned
to engine SR — `AudioOutput::start_with_buffer_size` queries the
device's nominal rate and forces it via
`kAudioDevicePropertyNominalSampleRate` if it differs (same gauntlet
as `AudioInput`). The first SL3 run shipped with output at 44.1 kHz
and engine at 48 kHz, which the CoreAudio HAL DefaultOutput unit
sometimes resamples and sometimes plays literally at the device
clock — driver-dependent and silent either way. Forcing alignment
removes the resampler from the path; if the device can't honor the
engine SR, output start-up fails with a clear error rather than
shipping audible 8% pitch drift. `dub play --realtime` already
built the engine at the device's reported SR so it sees a no-op
here; only the timecode-deck case (which pins engine to *input* SR)
exercises the new alignment.

**What this is *not*.**

- Position drift correction. Relative-mode in v1 lets deck position
  evolve via integration of rate, which is what the platter
  encodes. M5.4+ may add explicit re-sync if accumulated drift
  becomes audible over long sessions.
- Stickiness on stylus lift (M5.4).
- External-mixer multi-channel output routing (M5.5). Output today
  is a single summed stereo bus; per-deck routing waits until
  hardware actually demands it.
- Multi-deck timecode. Engine has slots for `[Option<TimecodeInput>;
  DECK_COUNT]` so M5.5 just attaches a second one — but until then
  CLI's `dub timecode-deck` wires only deck 0.

### Two decks + debug internal mixer — M4

The engine has always declared `DECK_COUNT = 2`; M4 makes the second
deck driveable end-to-end and adds a master gain to the debug internal
mixer. The mixer is intentionally minimal: each deck has its own
linear `gain`, both decks render additively into one summed stereo
bus, and `Engine::master_gain` (default 1.0) multiplies the bus once
after the deck loop. The multiply is skipped when master is unity
(`(g - 1.0).abs() <= f32::EPSILON`) so the common case has zero
arithmetic cost.

```text
                   ┌────────────────────────────┐
  Deck 0 ──gain──► │                            │
                   │   Σ   ──── master_gain ──► │ ──► CoreAudio (one stereo bus)
  Deck 1 ──gain──► │                            │
                   └────────────────────────────┘
```

Master gain is mutable through the lock-free command channel via
`Command::SetMasterGain` (engine-wide; carries no deck index). The
public surface on `EngineHandle` is `set_master_gain(g)`; per-deck
gain stays on `DeckCommand::set_gain`. Both compose multiplicatively
inside the render loop — no separate "channel strip" abstraction —
because v1's debug mixer doesn't need EQ/filters/sends and a flat
implementation keeps the audio thread's data dependency graph tiny.

External-mixer 4-channel routing (deck 0 → output channels 1+2,
deck 1 → output channels 3+4) is **deliberately deferred** to M5/M6.
That's the milestone where the timecode hardware (SL3, Audio 6) makes
multi-channel routing actually testable. v1's debug mixer covers
single-stereo-output development and is what every existing CLI
analyze workflow runs against.

### Engine → UI (state snapshot) — implemented in M2

Per-deck `Arc<DeckSharedState>` carrying:

- `position_bits: AtomicU64` (`f64::to_bits` of current track frame),
- `is_playing: AtomicBool`,
- `at_end: AtomicBool`.

Audio thread writes (Relaxed) once per render block. UI reads (Relaxed)
at whatever rate it likes — typically 60 fps for waveforms. There is no
synchronization guarantee across fields; tearing during a transport
change is invisible at 60 fps and we deliberately avoid the cost of
`SeqCst` here.

### Engine → UI (events) — pending M5+

`ringbuf::HeapRb<EngineEvent>` for discrete events (xrun detected, source
mode changed, end-of-track reached, etc.). Not yet wired; the snapshot
covers everything we need through M4.

## Build / link / ship

- Rust core builds to a static library + cdylib.
- UniFFI generates Swift bindings from `dub-ffi`'s UDL.
- `scripts/build-xcframework.sh` (M0.5) orchestrates: cargo build for both
  arches, lipo, xcodebuild -create-xcframework, UniFFI bindgen.
- Apple app links the `DubCore.xcframework`.
- Distribution: GitHub Releases, unsigned in v1.0, notarized in v1.1.

## Tests

- Unit + property tests live next to source.
- Integration tests in `crates/<name>/tests/`.
- Soak harness lives in `crates/dub-cli/` (offline render with synthetic input).
- Fuzz targets in `fuzz/fuzz_targets/` (added per parser as they land).
- Snapshot tests for SwiftUI views via `swift-snapshot-testing`.

## Open architecture questions

(These are tracked here, not as commitments — answers emerge during implementation.)

- Should the audio worker (decoder + waveform pre-render) be a single thread
  with cooperative work-stealing, or one thread per deck? **Decision: M3.**
- Engine state snapshot: one big atomic struct, or many small atomics? Trade-off
  is cache-line traffic vs. update granularity. **Decision: M4.**
- UniFFI vs `swift-bridge` for the FFI surface — UniFFI is more polished,
  `swift-bridge` allows tighter integration. **Decision: M0.5.**

## See also

- `docs/PRD.md` — product spec (source of truth)
- `docs/LIBRARY-FORMATS.md` — Serato / Traktor / rekordbox / iTunes / Lexicon
- `docs/adr/` — architecture decision records (not yet populated)
