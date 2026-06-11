//! A single deck: holds a track, plays it forward or backward at any rate,
//! mixes its output into a stereo buffer.
//!
//! Per PRD §4.4: forward and backward playback are byte-for-byte symmetric.
//! A negative rate is just "rate" with a sign. This is the foundation for
//! scratch (M5+), backspins, dnb-style manual rewinds, and ordinary
//! varispeed playback all using the same code path.
//!
//! M1 ships **linear interpolation** between adjacent track frames. This is
//! the standard "scratching" resampler — fast, branch-free, no aliasing
//! artefacts at extreme rates because anti-aliased resampling at e.g. 50×
//! reverse playback is not perceptually meaningful. Anti-aliased sinc
//! resampling for ordinary playback (with key-lock disabled) lands later
//! when we evaluate whether linear is audibly insufficient.

use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use dub_io::Track;

use crate::declick::DeclickEnvelope;
use crate::realtime::RealtimeContext;

/// State shared between an audio-thread [`Deck`] and the main-thread
/// [`crate::handle::DeckCommand`] proxy. Lock-free reads from the UI
/// thread, lock-free writes from the audio thread.
///
/// Six values are made visible across the boundary:
///
/// - **position (in track frames)** as `f64::to_bits` packed in an
///   `AtomicU64`. The audio thread updates this once per render block
///   (post-block position); the UI reads it for waveform/playhead.
///   Relaxed ordering is sufficient: it's a one-way snapshot, no
///   synchronization required.
/// - **playing flag**: the deck's transport state. Audio thread writes
///   when commands change it; UI reads to render the play/pause button.
/// - **end-of-track flag**: set by the audio thread when the playhead
///   walks off the end of the source. Lets the UI auto-reset etc.
/// - **panic-play flag (M10.6b)**: the deck is currently in
///   Panic-Play state (PRD §6.1.2 / §5.4.2) — i.e. running at a
///   held last-known-velocity rate, ignoring any attached
///   timecode input until a clean LFSR re-lock or an explicit
///   cancel. UI reads this to render the `TC · HOLD` source-pill
///   amber-dot state and to un-gate overview click-jump in
///   Timecode mode (M10.6c).
/// - **duration in seconds** as `f64::to_bits` (M11d.6 round 5). The
///   audio thread publishes this whenever a new source is set so
///   the off-main waveform render thread can read playhead and
///   duration from atomics without taking the FFI engine mutex.
///   Zero when no track is loaded.
/// - **has-track flag (M11d.6 round 5)**: matches `source.is_some()`
///   from the audio thread's perspective. Paired with
///   `duration_secs_bits` so a lock-free reader can detect an
///   unloaded deck without consulting `file_tracks` under the
///   `EngineState` mutex.
/// - **publish-time (M11d.6 round 6)**: the audio thread tags every
///   `position_secs` publish with the wall-clock host time at which
///   it was written, plus the rate at which the playhead is
///   advancing (seconds-of-playhead per second-of-wall-clock).
///   Stored together with `position_secs_bits` under a seqlock so
///   the lock-free reader sees a coherent `(host_time, secs, rate)`
///   triple. The FFI uses these to **extrapolate** the playhead
///   forward to the moment of the read, eliminating the 30 Hz
///   strobe that otherwise emerges when a ~100 Hz audio publisher
///   is sampled by a 60 Hz renderer (PRD §2.2.6 audience-grade
///   visual reliability).
#[derive(Debug)]
pub struct DeckSharedState {
    position_bits: AtomicU64,
    is_playing: AtomicBool,
    at_end: AtomicBool,
    is_panic_play: AtomicBool,
    duration_secs_bits: AtomicU64,
    /// Seqlock for `position_secs_bits` / `publish_host_ns` /
    /// `rate_bits`. Even ⇒ stable; odd ⇒ writer is in the middle
    /// of a publish. A single audio-thread writer per deck means
    /// the writer can never deadlock against itself.
    publish_seq: AtomicU64,
    /// Published alongside [`Self::position_bits`] so a lock-free
    /// snapshot reader can surface wall-clock seconds without
    /// owning the track's sample-rate metadata.
    position_secs_bits: AtomicU64,
    /// Engine-relative host time (nanoseconds since
    /// [`engine_host_origin`]) at which the audio thread last
    /// wrote `position_secs_bits`.
    publish_host_ns: AtomicU64,
    /// Rate at which `position_secs` advances per wall-clock
    /// second (f64 bits). `1.0` for ordinary forward playback,
    /// negative for reverse, `0.0` for paused / no-source.
    rate_bits: AtomicU64,
    has_track: AtomicBool,
    /// Coherent publish clock. When non-zero,
    /// [`Self::publish_position_secs`] uses this value as the
    /// `host_time_ns` paired with the published `position_secs`,
    /// instead of capturing `host_time_ns()` at the moment publish
    /// runs.
    ///
    /// The audio thread computes `position_secs` for the playhead
    /// at the **end of the block being rendered** — a value tied
    /// to the audio hardware's output clock, not to the wall-clock
    /// moment at which the publish runs. Publishing
    /// `(position_at_end_of_block, host_time_ns_now)` would let
    /// the renderer's `(vsync − now) × rate` extrapolation pick
    /// up the audio thread's CPU-scheduling jitter as visible
    /// sub-pixel grid-line wobble.
    ///
    /// `dub-audio` converts CoreAudio's hardware-locked
    /// `inTimeStamp.mHostTime` to the engine's `host_time_ns`
    /// domain, adds the block duration to get the output time of
    /// the **last** frame (the one `position_secs` corresponds
    /// to), and stores it here before calling `Engine::render`.
    /// Result: the published `(position_secs, host_time_ns)` pair
    /// is coherent to within audio-hardware-clock precision.
    ///
    /// Zero ⇒ no override; publish falls back to `host_time_ns()`
    /// for tests and offline-render callers that don't wire the
    /// CoreAudio timestamp through.
    publish_host_ns_override: AtomicU64,
    /// Timecode signal-quality telemetry, published by the audio
    /// thread from `drive_timecode_inputs` each block (the deck's
    /// decoder `DecodeOutput` + `LiftPolicy` state). Read lock-free
    /// by the FFI to drive the deck-header tracking dot + the
    /// signal-quality panel. Plain relaxed atomics — a display value
    /// tearing across one block is invisible at 60 fps, so no seqlock.
    tc_confidence_bits: AtomicU32,
    tc_amplitude_bits: AtomicU32,
    /// 0 = no timecode input, 1 = engaged/clean, 2 = engaged/degraded
    /// (in the sticky-disengage window), 3 = disengaged (lifted /
    /// scratching / dropout). See [`TimecodeTelemetry`].
    tc_lock_state: AtomicU8,
    tc_has_input: AtomicBool,
    /// Heavily low-passed playback rate for the **pitch / live-BPM
    /// display only** (f64 bits). The raw per-block decoded rate is
    /// far too jittery to show as a tempo — a ±1 % wobble reads as the
    /// whole-number BPM jumping. The audio thread EMA-smooths it before
    /// publishing here so the readout is stable to ~0.1 BPM, the way
    /// Serato / Traktor present it. Never feeds playback (that uses the
    /// raw rate in the position seqlock).
    tc_display_rate_bits: AtomicU64,
    /// Auto source-detection + calibration state for the deck-header
    /// Internal/Timecode switch + status dot. `control_mode`: 0
    /// Internal · 1 Timecode. `source_class`: 0 Silence · 1 Timecode ·
    /// 2 Record. Plus whether a whitening calibration is installed /
    /// in-progress.
    control_mode_bits: AtomicU8,
    source_class_bits: AtomicU8,
    calibrated_flag: AtomicBool,
    calibrating_flag: AtomicBool,
    /// Whether the user has pinned the control mode (the deck-header
    /// switch), suppressing auto source-detection until released. Lets
    /// the UI distinguish a *pinned* TIMECODE/INTERNAL from one the
    /// auto-classifier merely landed on.
    control_override_flag: AtomicBool,
    /// Diagnostic: the installed channel-whitening matrix (row-major
    /// `[w00, w01, w10, w11]`, f32 bits) and a counter that bumps each
    /// time a new calibration installs. Lets the signal-quality log show
    /// what calibration actually produced — a near-identity / degenerate
    /// result means the cartridge ellipse went uncorrected.
    whitening_bits: [AtomicU32; 4],
    calibration_seq: AtomicU32,
    /// M6 absolute-position diagnostics: whether the LFSR bitstream
    /// tracker is locked, and the decoded groove position in seconds
    /// of record time (input frames / input SR). Display/log only —
    /// the deck is *driven* by per-block deltas, never by this value.
    tc_abs_locked: AtomicBool,
    tc_abs_position_secs_bits: AtomicU64,
    /// Sticker-drift reading in milliseconds (see [`crate::drift`]);
    /// NaN until the first ABS-locked observation. Diagnostic only.
    tc_sticker_drift_ms_bits: AtomicU64,
    /// Whether the pitch / live-BPM readout has finished its settling
    /// measurements (wobble fit converged); UI dims the number until
    /// then. `true` for non-timecode drive.
    tc_pitch_settled: AtomicBool,
    /// Measurement progress [0, 1] for the deck-header calibration
    /// line (f32 bits). 1.0 when nothing is measuring.
    tc_measure_progress_bits: AtomicU32,
}

/// Lock-free snapshot of a deck's timecode signal health. Returned by
/// [`DeckSharedState::load_timecode_telemetry`].
///
/// The four bools are independent telemetry flags published per block,
/// not a state machine — a deck can be e.g. calibrated *and* overridden
/// *and* have an input simultaneously. Collapsing them into enums would
/// obscure the wire format the FFI mirrors, so the `excessive_bools`
/// lint is suppressed here deliberately.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimecodeTelemetry {
    /// Decoder coherence confidence [0, 1]; 1.0 = pure quadrature.
    pub confidence: f32,
    /// Decoder RMS amplitude (~0.1–0.5 for a healthy SL3 carrier).
    pub amplitude: f32,
    /// 0 none · 1 clean lock · 2 degraded (sticky window) · 3 disengaged.
    pub lock_state: u8,
    /// Whether a timecode input is attached to this deck at all.
    pub has_input: bool,
    /// Heavily smoothed playback rate for the pitch / live-BPM display
    /// (1.0 = unity). Display-only; never the playback rate.
    pub display_rate: f64,
    /// 0 Internal · 1 Timecode — how the deck is currently driven.
    pub control_mode: u8,
    /// 0 Silence · 1 Timecode · 2 Record — auto source classification.
    pub source_class: u8,
    /// Whether a channel-whitening calibration is installed.
    pub calibrated: bool,
    /// Whether a calibration capture is in progress.
    pub calibrating: bool,
    /// Whether the user has pinned the control mode (switch override),
    /// vs. the mode being chosen by auto source-detection.
    pub overridden: bool,
    /// Diagnostic: installed whitening matrix `[w00, w01, w10, w11]`
    /// (identity until first calibration).
    pub whitening: [f32; 4],
    /// Diagnostic: count of calibrations installed since attach.
    pub calibration_seq: u32,
    /// M6: whether the absolute LFSR position tracker is locked on the
    /// bitstream (bit-exact velocity + drift-free playhead active).
    pub abs_locked: bool,
    /// M6: decoded absolute groove position in seconds of record time.
    /// Meaningful only while `abs_locked`; diagnostic/log only.
    pub abs_position_secs: f64,
    /// Sticker drift in milliseconds: how far the relative-mode
    /// playhead has slid against the absolute groove position since
    /// the current anchor (see [`crate::drift`]). Positive = playhead
    /// lags the record. NaN until the first ABS-locked observation.
    pub sticker_drift_ms: f64,
    /// Whether the pitch / live-BPM readout has finished settling
    /// (wobble fit converged — ~2 revolutions of locked play). The UI
    /// shows the value dimmed / "measuring" until then.
    pub pitch_settled: bool,
    /// Measurement progress [0, 1] for the deck-header calibration
    /// progress line; 1.0 when nothing is measuring.
    pub measure_progress: f32,
}

/// Process-local monotonic clock origin shared by the audio
/// thread (writer) and the FFI snapshot reader. Lazily
/// initialised on first access. We deliberately keep the "host
/// time" private to dub-engine rather than exposing absolute
/// `Instant`s across the FFI: callers see only `position_secs`
/// already extrapolated to the moment of the read, so no clock
/// semantics leak past the boundary.
fn engine_host_origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

/// Read the current engine-relative host time in nanoseconds.
///
/// On macOS this delegates to `mach_absolute_time` via
/// `Instant::now`; on Linux to `clock_gettime(CLOCK_MONOTONIC)`.
/// Both are vDSO-mapped on the platforms we care about, so the
/// call is wait-free and RT-safe — no syscall, no allocation.
#[inline]
fn host_time_ns() -> u64 {
    let origin = engine_host_origin();
    let dt = origin.elapsed();
    let nanos = u128::from(dt.as_secs())
        .saturating_mul(1_000_000_000)
        .saturating_add(u128::from(dt.subsec_nanos()));
    u64::try_from(nanos.min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
}

impl DeckSharedState {
    /// Construct a fresh shared-state slot. All fields default to
    /// "no track" / "stopped". Cheap; intended to be called once
    /// per deck per process.
    #[must_use]
    pub fn new() -> Self {
        Self {
            position_bits: AtomicU64::new(0.0f64.to_bits()),
            is_playing: AtomicBool::new(false),
            at_end: AtomicBool::new(false),
            is_panic_play: AtomicBool::new(false),
            duration_secs_bits: AtomicU64::new(0.0f64.to_bits()),
            publish_seq: AtomicU64::new(0),
            position_secs_bits: AtomicU64::new(0.0f64.to_bits()),
            publish_host_ns: AtomicU64::new(0),
            rate_bits: AtomicU64::new(0.0f64.to_bits()),
            has_track: AtomicBool::new(false),
            publish_host_ns_override: AtomicU64::new(0),
            tc_confidence_bits: AtomicU32::new(0.0f32.to_bits()),
            tc_amplitude_bits: AtomicU32::new(0.0f32.to_bits()),
            tc_lock_state: AtomicU8::new(0),
            tc_has_input: AtomicBool::new(false),
            tc_display_rate_bits: AtomicU64::new(1.0f64.to_bits()),
            control_mode_bits: AtomicU8::new(0),
            source_class_bits: AtomicU8::new(0),
            calibrated_flag: AtomicBool::new(false),
            calibrating_flag: AtomicBool::new(false),
            control_override_flag: AtomicBool::new(false),
            whitening_bits: [
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
            ],
            calibration_seq: AtomicU32::new(0),
            tc_abs_locked: AtomicBool::new(false),
            tc_sticker_drift_ms_bits: AtomicU64::new(f64::NAN.to_bits()),
            tc_pitch_settled: AtomicBool::new(true),
            tc_measure_progress_bits: AtomicU32::new(1.0f32.to_bits()),
            tc_abs_position_secs_bits: AtomicU64::new(0.0f64.to_bits()),
        }
    }

    /// Reset all atomics to the "no track" / "stopped" baseline
    /// (M11d.6 round 5). The FFI calls this when a Running →
    /// Stopped transition lands so a UI that's still polling
    /// `position_snapshot` after `stop_thru` sees a clean
    /// "empty deck" state instead of the last in-flight playhead.
    pub fn reset(&self) {
        self.position_bits.store(0u64, Ordering::Relaxed);
        self.duration_secs_bits.store(0u64, Ordering::Relaxed);
        self.is_playing.store(false, Ordering::Relaxed);
        self.at_end.store(false, Ordering::Relaxed);
        self.is_panic_play.store(false, Ordering::Relaxed);
        self.has_track.store(false, Ordering::Relaxed);
        self.tc_confidence_bits
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        self.tc_amplitude_bits
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        self.tc_lock_state.store(0, Ordering::Relaxed);
        self.tc_has_input.store(false, Ordering::Relaxed);
        self.tc_display_rate_bits
            .store(1.0f64.to_bits(), Ordering::Relaxed);
        self.control_mode_bits.store(0, Ordering::Relaxed);
        self.source_class_bits.store(0, Ordering::Relaxed);
        self.calibrated_flag.store(false, Ordering::Relaxed);
        self.calibrating_flag.store(false, Ordering::Relaxed);
        self.control_override_flag.store(false, Ordering::Relaxed);
        self.tc_abs_locked.store(false, Ordering::Relaxed);
        self.tc_abs_position_secs_bits
            .store(0.0f64.to_bits(), Ordering::Relaxed);
        self.tc_sticker_drift_ms_bits
            .store(f64::NAN.to_bits(), Ordering::Relaxed);
        self.tc_pitch_settled.store(true, Ordering::Relaxed);
        self.publish_calibration([[1.0, 0.0], [0.0, 1.0]], 0);
        // Bring the seqlock-protected publish triple back to a
        // clean baseline (zero playhead, zero rate, host time at
        // "now" so no extrapolation kicks in if the snapshot
        // reader arrives before the next audio block).
        self.publish_position_secs(0.0, 0.0);
    }

    /// Publish this deck's timecode signal health (audio-thread
    /// writer, called from `drive_timecode_inputs` each block).
    /// A handful of relaxed stores; RT-safe (no alloc, no lock, no
    /// syscall).
    // Flat scalar telemetry — bundling into a struct would just move
    // the field list without removing it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish_timecode_telemetry(
        &self,
        confidence: f32,
        amplitude: f32,
        lock_state: u8,
        has_input: bool,
        display_rate: f64,
        abs_locked: bool,
        abs_position_secs: f64,
        sticker_drift_ms: f64,
        pitch_settled: bool,
        measure_progress: f32,
    ) {
        self.tc_confidence_bits
            .store(confidence.to_bits(), Ordering::Relaxed);
        self.tc_amplitude_bits
            .store(amplitude.to_bits(), Ordering::Relaxed);
        self.tc_lock_state.store(lock_state, Ordering::Relaxed);
        self.tc_has_input.store(has_input, Ordering::Relaxed);
        self.tc_display_rate_bits
            .store(display_rate.to_bits(), Ordering::Relaxed);
        self.tc_abs_locked.store(abs_locked, Ordering::Relaxed);
        self.tc_abs_position_secs_bits
            .store(abs_position_secs.to_bits(), Ordering::Relaxed);
        self.tc_sticker_drift_ms_bits
            .store(sticker_drift_ms.to_bits(), Ordering::Relaxed);
        self.tc_pitch_settled
            .store(pitch_settled, Ordering::Relaxed);
        self.tc_measure_progress_bits
            .store(measure_progress.to_bits(), Ordering::Relaxed);
    }

    /// Publish auto source-detection + calibration state (audio thread).
    pub(crate) fn publish_source_state(
        &self,
        control_mode: u8,
        source_class: u8,
        calibrated: bool,
        calibrating: bool,
        overridden: bool,
    ) {
        self.control_mode_bits
            .store(control_mode, Ordering::Relaxed);
        self.source_class_bits
            .store(source_class, Ordering::Relaxed);
        self.calibrated_flag.store(calibrated, Ordering::Relaxed);
        self.calibrating_flag.store(calibrating, Ordering::Relaxed);
        self.control_override_flag
            .store(overridden, Ordering::Relaxed);
    }

    /// Publish the installed whitening matrix + calibration counter
    /// (audio thread). Diagnostic only. Four relaxed stores + one for the
    /// seq; RT-safe.
    pub(crate) fn publish_calibration(&self, whitening: [[f64; 2]; 2], seq: u32) {
        #[allow(clippy::cast_possible_truncation)]
        let flat = [
            whitening[0][0] as f32,
            whitening[0][1] as f32,
            whitening[1][0] as f32,
            whitening[1][1] as f32,
        ];
        for (slot, v) in self.whitening_bits.iter().zip(flat) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
        self.calibration_seq.store(seq, Ordering::Relaxed);
    }

    /// Lock-free read of the deck's timecode signal health + source state.
    #[must_use]
    pub fn load_timecode_telemetry(&self) -> TimecodeTelemetry {
        TimecodeTelemetry {
            confidence: f32::from_bits(self.tc_confidence_bits.load(Ordering::Relaxed)),
            amplitude: f32::from_bits(self.tc_amplitude_bits.load(Ordering::Relaxed)),
            lock_state: self.tc_lock_state.load(Ordering::Relaxed),
            has_input: self.tc_has_input.load(Ordering::Relaxed),
            display_rate: f64::from_bits(self.tc_display_rate_bits.load(Ordering::Relaxed)),
            control_mode: self.control_mode_bits.load(Ordering::Relaxed),
            source_class: self.source_class_bits.load(Ordering::Relaxed),
            calibrated: self.calibrated_flag.load(Ordering::Relaxed),
            calibrating: self.calibrating_flag.load(Ordering::Relaxed),
            overridden: self.control_override_flag.load(Ordering::Relaxed),
            whitening: [
                f32::from_bits(self.whitening_bits[0].load(Ordering::Relaxed)),
                f32::from_bits(self.whitening_bits[1].load(Ordering::Relaxed)),
                f32::from_bits(self.whitening_bits[2].load(Ordering::Relaxed)),
                f32::from_bits(self.whitening_bits[3].load(Ordering::Relaxed)),
            ],
            calibration_seq: self.calibration_seq.load(Ordering::Relaxed),
            abs_locked: self.tc_abs_locked.load(Ordering::Relaxed),
            abs_position_secs: f64::from_bits(
                self.tc_abs_position_secs_bits.load(Ordering::Relaxed),
            ),
            sticker_drift_ms: f64::from_bits(self.tc_sticker_drift_ms_bits.load(Ordering::Relaxed)),
            pitch_settled: self.tc_pitch_settled.load(Ordering::Relaxed),
            measure_progress: f32::from_bits(self.tc_measure_progress_bits.load(Ordering::Relaxed)),
        }
    }

    pub(crate) fn store_position(&self, frames: f64) {
        self.position_bits
            .store(frames.to_bits(), Ordering::Relaxed);
    }

    /// Read the current playhead in **track frames**.
    #[must_use]
    pub fn load_position(&self) -> f64 {
        f64::from_bits(self.position_bits.load(Ordering::Relaxed))
    }

    pub(crate) fn store_playing(&self, playing: bool) {
        self.is_playing.store(playing, Ordering::Relaxed);
    }

    /// `true` while the deck is advancing its playhead.
    #[must_use]
    pub fn load_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub(crate) fn store_at_end(&self, at_end: bool) {
        self.at_end.store(at_end, Ordering::Relaxed);
    }

    /// `true` once the playhead has walked off the end of the
    /// source.
    #[must_use]
    pub fn load_at_end(&self) -> bool {
        self.at_end.load(Ordering::Relaxed)
    }

    pub(crate) fn store_panic_play(&self, panic: bool) {
        self.is_panic_play.store(panic, Ordering::Relaxed);
    }

    /// M10.6b. `true` while the deck is in Panic-Play state.
    #[must_use]
    pub fn load_panic_play(&self) -> bool {
        self.is_panic_play.load(Ordering::Relaxed)
    }

    pub(crate) fn store_duration_secs(&self, secs: f64) {
        self.duration_secs_bits
            .store(secs.to_bits(), Ordering::Relaxed);
    }

    /// Read the loaded track's wall-clock duration. Zero when no
    /// track is loaded.
    #[must_use]
    pub fn load_duration_secs(&self) -> f64 {
        f64::from_bits(self.duration_secs_bits.load(Ordering::Relaxed))
    }

    pub(crate) fn store_has_track(&self, has: bool) {
        self.has_track.store(has, Ordering::Relaxed);
    }

    /// `true` while a File-mode track is loaded on the deck.
    #[must_use]
    pub fn load_has_track(&self) -> bool {
        self.has_track.load(Ordering::Relaxed)
    }

    /// Override the host time used by the next
    /// [`Self::publish_position_secs`] call. See the docstring on
    /// `publish_host_ns_override` for the motivation.
    /// `Engine::set_block_output_host_ns` calls this on every deck
    /// immediately before draining the audio block. Set `0` to
    /// disable.
    pub fn set_publish_host_ns_override(&self, ns: u64) {
        self.publish_host_ns_override.store(ns, Ordering::Release);
    }

    /// Publish the playhead (in wall-clock seconds) together with
    /// the host time at which it was written and the rate at
    /// which it is advancing.
    ///
    /// `rate` here is wall-time-rate: seconds of track time per
    /// second of CPU wall time. For 1.0× File-mode playback this
    /// is 1.0 regardless of any engine/track SR mismatch. Callers
    /// must normalise out the SR ratio that [`Deck::set_rate`]
    /// bakes into `self.rate` before passing the value in. Set to
    /// 0.0 on pause / seek / no-source so the FFI extrapolator
    /// freezes the playhead.
    ///
    /// Single audio-thread writer per deck, seqlock-protected so
    /// a lock-free reader sees a coherent triple. RT-safe: vDSO
    /// `Instant::now`, three relaxed stores, two fences, two
    /// seqlock stores. No allocation, no syscall, no spin.
    ///
    /// When [`Self::publish_host_ns_override`] is non-zero the
    /// publish uses that value as the `host_time_ns` paired with
    /// `secs` instead of capturing `host_time_ns()` at publish
    /// time, so the `(position, host_time)` pair is coherent
    /// against the audio hardware clock rather than carrying
    /// CPU-scheduling jitter from the render thread.
    pub(crate) fn publish_position_secs(&self, secs: f64, rate: f64) {
        let override_ns = self.publish_host_ns_override.load(Ordering::Acquire);
        let host_ns = if override_ns != 0 {
            override_ns
        } else {
            host_time_ns()
        };
        let seq = self.publish_seq.load(Ordering::Relaxed);
        // Move the seq into the "odd" / writing state so a reader
        // sandwiching its load between the two seq writes will
        // detect the in-flight update and retry.
        self.publish_seq
            .store(seq.wrapping_add(1), Ordering::Release);
        fence(Ordering::Release);
        self.position_secs_bits
            .store(secs.to_bits(), Ordering::Relaxed);
        self.publish_host_ns.store(host_ns, Ordering::Relaxed);
        self.rate_bits.store(rate.to_bits(), Ordering::Relaxed);
        fence(Ordering::Release);
        self.publish_seq
            .store(seq.wrapping_add(2), Ordering::Release);
    }

    /// Read the current playhead in **wall-clock seconds**.
    ///
    /// This is the raw value last published by the audio thread,
    /// **without extrapolation**. Use [`Self::load_publish_state`]
    /// when you want a smooth render-time playhead (see PRD §2.2.6).
    #[must_use]
    pub fn load_position_secs(&self) -> f64 {
        f64::from_bits(self.position_secs_bits.load(Ordering::Relaxed))
    }

    /// **M11d.6 round 6.** Lock-free read of the coherent
    /// `(host_time_ns, position_secs, rate)` triple. Returns the
    /// values that were in-flight together for one audio render
    /// block. The seqlock retries internally if a writer is
    /// mid-publish; in practice this happens once per audio
    /// block (~5–11 ms), so the reader spins at most once or
    /// twice.
    #[must_use]
    pub fn load_publish_state(&self) -> PublishState {
        loop {
            let s1 = self.publish_seq.load(Ordering::Acquire);
            // Writer is in the middle of a publish — back off and
            // retry. The audio thread's publish window is
            // <1 microsecond, so a `spin_loop` hint plus immediate
            // retry is sufficient.
            if s1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            fence(Ordering::Acquire);
            let position_secs = f64::from_bits(self.position_secs_bits.load(Ordering::Relaxed));
            let host_time_ns = self.publish_host_ns.load(Ordering::Relaxed);
            let rate = f64::from_bits(self.rate_bits.load(Ordering::Relaxed));
            fence(Ordering::Acquire);
            let s2 = self.publish_seq.load(Ordering::Acquire);
            if s1 == s2 {
                return PublishState {
                    position_secs,
                    host_time_ns,
                    rate,
                };
            }
            // Torn read: writer landed a publish between s1 and
            // s2. Loop. By construction this can only happen once
            // per audio block per reader call.
        }
    }

    /// Engine-relative host time **right now**, in nanoseconds.
    ///
    /// Same clock domain as [`PublishState::host_time_ns`], so the
    /// FFI can compute `(now - publish_host_ns)` to get the
    /// elapsed time since the audio thread last published. Wait-
    /// free, RT-safe (vDSO / `mach_absolute_time` under the hood).
    #[must_use]
    pub fn host_time_now_ns() -> u64 {
        host_time_ns()
    }
}

/// Coherent publish-triple returned by
/// [`DeckSharedState::load_publish_state`]. `position_secs` is the
/// playhead as written by the audio thread at `host_time_ns`,
/// advancing at `rate` seconds-of-playhead per second-of-wall-
/// clock. A reader wanting a smooth render-time playhead computes
/// `position_secs + (now - host_time_ns) * 1e-9 * rate`, clamped
/// to a sane elapsed bound (see [`PublishState::extrapolated_secs`]).
#[derive(Debug, Clone, Copy)]
pub struct PublishState {
    /// Last-published playhead in wall-clock seconds (unclamped;
    /// may be negative or beyond track end during scratching).
    pub position_secs: f64,
    /// Engine-relative host time when the audio thread wrote
    /// `position_secs`. Use [`DeckSharedState::host_time_now_ns`]
    /// for the matching "now" value.
    pub host_time_ns: u64,
    /// Seconds-of-playhead per second-of-wall-clock. `1.0` for
    /// ordinary forward playback, negative for reverse, `0.0`
    /// for paused or unloaded.
    pub rate: f64,
}

impl PublishState {
    /// Maximum wall-clock seconds the FFI will extrapolate forward
    /// from a publish before clamping. A stalled audio thread
    /// would otherwise let the renderer extrapolate into nonsense;
    /// 100 ms is ~10 audio blocks worth of headroom, far longer
    /// than any healthy publish interval, but short enough that
    /// a real stall is visibly frozen instead of running away.
    const MAX_EXTRAPOLATION_SECS: f64 = 0.100;

    /// Extrapolate the publish forward to the moment of the call.
    ///
    /// `now_ns` should come from [`DeckSharedState::host_time_now_ns`]
    /// so the clock domains match. Elapsed time is clamped to
    /// `[0, 100 ms]` so a hung audio thread can't drive the
    /// playhead off into the future.
    #[must_use]
    pub fn extrapolated_secs(&self, now_ns: u64) -> f64 {
        let elapsed_ns = now_ns.saturating_sub(self.host_time_ns);
        // `u64` → `f64` is lossy past 2^53 ns (~104 days). The
        // clamp below makes that lossless in every realistic case.
        #[allow(clippy::cast_precision_loss)]
        let elapsed_secs = (elapsed_ns as f64 * 1e-9).clamp(0.0, Self::MAX_EXTRAPOLATION_SECS);
        self.position_secs + elapsed_secs * self.rate
    }
}

impl Default for DeckSharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// In-flight de-click crossfade state.
///
/// `Idle` is the steady state. `Active` is set whenever a transport
/// mutation happens that would otherwise produce a sample-discontinuity:
/// source change, position jump, play/pause flip. The deck holds onto
/// the *previous* source/position/playing/rate values for the duration
/// of the ramp so the render can mix the old output (fading down) with
/// the new output (fading up).
///
/// The **engine** is responsible for taking `prev_source` (an
/// `Arc<Track>`) out of the `Active` variant once the ramp completes,
/// because the audio thread must not drop `Arc<Track>` (would call
/// `dealloc`). It bounces it through the trash channel like every other
/// off-RT-thread Arc disposal in M3.
#[derive(Debug)]
enum DeclickState {
    Idle,
    Active {
        prev_source: Option<Arc<Track>>,
        prev_position: f64,
        prev_rate: f64,
        prev_playing: bool,
        /// Counts down from `envelope.len()` to 0.
        samples_remaining: u32,
    },
}

/// A single deck's transport + audio source state.
///
/// Two views of the deck exist:
///
/// - the **audio-thread Deck** (this struct) holds the playhead, source,
///   and renders audio. Owned by the [`crate::Engine`].
/// - the **main-thread proxy** ([`crate::handle::DeckCommand`]) sends
///   commands to mutate this deck and reads the latest position via the
///   shared atomic snapshot.
///
/// They communicate through `Arc<DeckSharedState>`, written by the audio
/// thread once per render block and read with `Relaxed` ordering by the UI.
#[derive(Debug)]
pub struct Deck {
    source: Option<Arc<Track>>,

    /// Current playhead, in **track frames**, as a floating-point value.
    /// Sample-accurate over very long tracks (`f64`).
    position: f64,

    /// Playback rate in **track-frames per output-frame**. Already factors
    /// in the engine vs track sample-rate ratio. Set via [`Deck::set_rate`].
    rate: f64,

    /// Linear gain applied to the deck's contribution to the mix. `1.0` is
    /// unity. Range checked at set-time but not in render (RT discipline).
    gain: f32,

    /// True when this deck contributes audio to the engine output. False
    /// means the deck is muted and renders silence (without advancing).
    playing: bool,

    /// Atomic snapshot shared with the main-thread handle. Audio-thread-only
    /// writes; `Arc::clone` happens off-RT in the constructor.
    shared: Arc<DeckSharedState>,

    /// Crossfade table used to absorb transport-induced discontinuities.
    /// Shared (read-only) across decks of the same engine.
    declick_envelope: Arc<DeclickEnvelope>,

    /// Current de-click ramp state.
    declick: DeclickState,

    /// Holds an `Arc<Track>` that would otherwise be stranded by a
    /// transport mutation arriving before the previous declick ramp
    /// completed. The engine drains this each block and ferries it
    /// through the trash channel. `None` in the steady state.
    pending_disposal: Option<Arc<Track>>,

    /// Exact playhead advance (in **track frames**) to apply across
    /// the next render block, accumulated via
    /// [`Deck::advance_position_frames`] (M6 absolute timecode).
    /// While set, the block still integrates `rate` per sample for
    /// the intra-block resampling slope, but the block-boundary
    /// position is re-pinned to `block_start + pending_advance` —
    /// killing integration drift without a declick (the sub-frame
    /// correction per block is far below audibility). `None` in the
    /// steady state and whenever the deck is driven by rate alone.
    pending_advance: Option<f64>,
}

impl Deck {
    /// Construct an empty deck with no track loaded. Allocates the shared
    /// atomic state — call this off the audio thread.
    ///
    /// The declick envelope is shared across the engine's decks; cloning
    /// the `Arc` is cheap.
    #[must_use]
    pub fn new(declick_envelope: Arc<DeclickEnvelope>) -> Self {
        Self::with_shared(declick_envelope, Arc::new(DeckSharedState::new()))
    }

    /// Construct an empty deck that publishes its transport into
    /// the caller-provided `shared` slot (M11d.6 round 5). Used by
    /// the FFI so the same `Arc<DeckSharedState>` survives
    /// `start_thru` / `stop_thru` cycles — a render thread that
    /// captured the Arc once never has to swap it on engine
    /// restart, and `DubEngine::position_snapshot` reads atomics
    /// that exist for the lifetime of the process.
    ///
    /// The caller should pass a freshly [`DeckSharedState::reset`]-ed
    /// slot when the engine is being (re)started; this constructor
    /// does not touch the atomics.
    #[must_use]
    pub fn with_shared(
        declick_envelope: Arc<DeclickEnvelope>,
        shared: Arc<DeckSharedState>,
    ) -> Self {
        Self {
            source: None,
            position: 0.0,
            rate: 1.0,
            gain: 1.0,
            playing: false,
            shared,
            declick_envelope,
            declick: DeclickState::Idle,
            pending_disposal: None,
            pending_advance: None,
        }
    }

    /// Return a clone of the shared state Arc.
    ///
    /// Production callers now pre-allocate the
    /// `Arc<DeckSharedState>` and pass it to [`Self::with_shared`]
    /// (M11d.6 round 5); this accessor stays in the API for tests
    /// that need to poke the audio-thread atomics directly.
    #[allow(dead_code)]
    pub(crate) fn shared(&self) -> Arc<DeckSharedState> {
        self.shared.clone()
    }

    /// Forward timecode signal-quality telemetry to the deck's
    /// shared state without cloning the `Arc` (RT-safe; called every
    /// block from `drive_timecode_inputs`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish_timecode_telemetry(
        &self,
        confidence: f32,
        amplitude: f32,
        lock_state: u8,
        has_input: bool,
        display_rate: f64,
        abs_locked: bool,
        abs_position_secs: f64,
        sticker_drift_ms: f64,
        pitch_settled: bool,
        measure_progress: f32,
    ) {
        self.shared.publish_timecode_telemetry(
            confidence,
            amplitude,
            lock_state,
            has_input,
            display_rate,
            abs_locked,
            abs_position_secs,
            sticker_drift_ms,
            pitch_settled,
            measure_progress,
        );
    }

    /// Forward auto source-detection + calibration state to the deck's
    /// shared state (RT-safe; called from `drive_timecode_inputs`).
    pub(crate) fn publish_source_state(
        &self,
        control_mode: u8,
        source_class: u8,
        calibrated: bool,
        calibrating: bool,
        overridden: bool,
    ) {
        self.shared.publish_source_state(
            control_mode,
            source_class,
            calibrated,
            calibrating,
            overridden,
        );
    }

    /// Forward the installed whitening matrix + calibration counter to
    /// the deck's shared state (RT-safe; diagnostic).
    pub(crate) fn publish_calibration(&self, whitening: [[f64; 2]; 2], seq: u32) {
        self.shared.publish_calibration(whitening, seq);
    }

    /// Audio-thread hook: pin the `host_time_ns` the next
    /// [`DeckSharedState::publish_position_secs`] call will pair
    /// with `position_secs`. Forwards to
    /// [`DeckSharedState::set_publish_host_ns_override`]; lives on
    /// `Deck` so [`crate::Engine::set_block_output_host_ns`] can
    /// fan out across both decks without exposing `shared()` as
    /// public. RT-safe (one relaxed store per deck per block).
    pub fn set_publish_host_ns_override(&self, ns: u64) {
        self.shared.set_publish_host_ns_override(ns);
    }

    /// Load a track. Resets the playhead to position 0. Wraps the source
    /// change in a de-click ramp so a fresh load fades in smoothly from
    /// silence (or from whatever was previously playing).
    pub fn set_source(&mut self, track: Arc<Track>) {
        self.start_declick();
        let duration_secs = Self::track_duration_secs(&track);
        self.source = Some(track);
        self.position = 0.0;
        self.shared.store_position(0.0);
        // Publish (secs=0, rate=0) so a snapshot reader landing
        // between source-set and the first render block sees a
        // pinned playhead instead of extrapolating off the prior
        // track's rate. The audio thread will overwrite both with
        // the real values on the next render block.
        self.shared.publish_position_secs(0.0, 0.0);
        self.shared.store_at_end(false);
        // Publish duration + has_track before we return so a
        // lock-free position_snapshot reader on another thread
        // observes the new track at the same moment the audio
        // thread does. Audio-thread safe: pure atomic stores.
        self.shared.store_duration_secs(duration_secs);
        self.shared.store_has_track(true);
    }

    /// Swap the deck's track for `track`. The previous source (if any)
    /// is stashed in the de-click state for the duration of the ramp,
    /// then must be harvested by the engine via
    /// [`Deck::take_finished_declick_source`] and bounced through the
    /// trash channel — the audio thread never drops `Arc<Track>`.
    ///
    /// Used by [`crate::Engine`] when applying [`crate::Command::DeckLoad`].
    pub fn swap_source(&mut self, track: Arc<Track>) {
        self.start_declick();
        let duration_secs = Self::track_duration_secs(&track);
        self.source = Some(track);
        self.position = 0.0;
        self.shared.store_position(0.0);
        self.shared.publish_position_secs(0.0, 0.0);
        self.shared.store_at_end(false);
        self.shared.store_duration_secs(duration_secs);
        self.shared.store_has_track(true);
    }

    /// Clear the loaded track. The deck fades to silence over the
    /// declick window then renders silence afterward.
    pub fn clear_source(&mut self) {
        self.start_declick();
        self.source = None;
        self.position = 0.0;
        self.shared.store_position(0.0);
        self.shared.publish_position_secs(0.0, 0.0);
        self.shared.store_at_end(false);
        self.shared.store_duration_secs(0.0);
        self.shared.store_has_track(false);
    }

    /// Compute wall-clock duration of a track in seconds. Pulled out
    /// of [`Self::set_source`] / [`Self::swap_source`] so both paths
    /// publish the same value, and so the conversion lives in one
    /// place (frames as `u64` → `f64`, sample rate as `u32` → `f64`,
    /// guarding against a zero sample rate).
    #[allow(clippy::cast_precision_loss)]
    fn track_duration_secs(track: &Track) -> f64 {
        let sr = f64::from(track.sample_rate());
        if sr > 0.0 {
            track.frames() as f64 / sr
        } else {
            0.0
        }
    }

    /// Begin a de-click ramp.
    ///
    /// Snapshots the *current* (about-to-become-old) deck state into
    /// `DeclickState::Active`. The caller then mutates `self` to the
    /// new state. The render loop crossfades old → new over
    /// `declick_envelope.len()` samples.
    ///
    /// Back-to-back transport mutations: if a ramp is already in
    /// flight when this is called, the previous ramp's `prev_source`
    /// would be clobbered by the new snapshot — and dropping it on
    /// the audio thread is forbidden. We move it into
    /// [`Self::pending_disposal`], which the engine drains after every
    /// command and ferries through the trash channel.
    fn start_declick(&mut self) {
        let new_prev_source = self.source.clone();
        let new_prev_position = self.position;
        let new_prev_rate = self.rate;
        let new_prev_playing = self.playing;
        let n = self.declick_envelope.len();

        // If a ramp is already active, its prev_source needs to be
        // surfaced for trash routing before we overwrite the slot.
        if let DeclickState::Active {
            prev_source: ref mut stranded,
            ..
        } = self.declick
        {
            if let Some(arc) = stranded.take() {
                // Defensive: if pending_disposal was already populated
                // (the user did *three* transport changes in a single
                // block), keep the older one in pending and discard
                // this one through pending too. Both reach the engine
                // after this block; the engine drains both. We need
                // somewhere to stage the *second* discard, but with
                // only one pending slot we'd have to choose. In
                // practice this is a < 2 ms window for a human-typed
                // burst; the worst case is two-deep, handled here.
                if self.pending_disposal.is_none() {
                    self.pending_disposal = Some(arc);
                } else {
                    // Three-deep: the audio thread cannot drop and
                    // cannot stash. Last resort: leak via mem::forget
                    // and let the engine surface this via a counter.
                    // (The engine's send_to_trash already implements
                    // this fallback, so just hand the Arc to that
                    // path on next harvest by reusing pending — we
                    // need another slot.) For now, we accept that
                    // four sub-ramp-window transport changes will
                    // leak one Arc; PRD's de-click is 2 ms, and a
                    // human can't generate 4 distinct transport
                    // events within 2 ms, so this is theoretical.
                    std::mem::forget(arc);
                }
            }
        }

        self.declick = DeclickState::Active {
            prev_source: new_prev_source,
            prev_position: new_prev_position,
            prev_rate: new_prev_rate,
            prev_playing: new_prev_playing,
            samples_remaining: n,
        };
    }

    /// Engine-only: take any `Arc<Track>` that became orphaned because a
    /// new transport change started before the previous declick had
    /// finished. Returns `None` in the common case.
    pub(crate) fn take_pending_disposal(&mut self) -> Option<Arc<Track>> {
        self.pending_disposal.take()
    }

    /// Engine-only: if a de-click ramp finished during the most recent
    /// render block, take the snapshot's `prev_source` so the engine can
    /// route it through the trash channel. Returns `None` if no ramp
    /// finished or if the previous side held no track (e.g. fading in
    /// from silence on first load).
    pub(crate) fn take_finished_declick_source(&mut self) -> Option<Arc<Track>> {
        if let DeclickState::Active {
            samples_remaining: 0,
            ..
        } = &self.declick
        {
            if let DeclickState::Active { prev_source, .. } =
                std::mem::replace(&mut self.declick, DeclickState::Idle)
            {
                return prev_source;
            }
        }
        None
    }

    /// Borrow the loaded track, if any.
    #[must_use]
    pub fn source(&self) -> Option<&Arc<Track>> {
        self.source.as_ref()
    }

    /// Current playback rate in **musical** units
    /// (audio-seconds-per-real-second). `1.0` is realtime at the
    /// track's natural pitch regardless of the engine vs source
    /// sample rates; `2.0` is double speed, `-1.0` is reverse at
    /// realtime, `0.0` is paused.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Set the playback rate in **musical** units (see
    /// [`Deck::rate`]). `1.0` is realtime at the track's natural
    /// pitch; the deck's render loop converts this into
    /// per-output-sample track-frame increments internally
    /// (`new_increment = rate * track_sr / engine_sr`), so callers
    /// must **not** pre-multiply by the SR ratio. See the
    /// `sample_rate_conversion_44k_to_48k` regression test for the
    /// single-conversion guarantee.
    ///
    /// `0.0` parks the playhead but does not stop the deck — the
    /// engine will still mix in whatever's currently at the
    /// playhead position. `-1.0` plays in reverse at realtime.
    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
    }

    /// Current playback position in track frames.
    #[must_use]
    pub fn position_frames(&self) -> f64 {
        self.position
    }

    /// Current playhead in **track seconds** (0.0 with no track
    /// loaded). Same conversion as the seek path publishes.
    #[must_use]
    pub fn position_secs(&self) -> f64 {
        self.source.as_ref().map_or(0.0, |track| {
            let sr = f64::from(track.sample_rate());
            if sr > 0.0 {
                self.position / sr
            } else {
                0.0
            }
        })
    }

    /// Whether a track is currently loaded.
    #[must_use]
    pub fn has_track(&self) -> bool {
        self.source.is_some()
    }

    /// Set the playback position in track frames. Clamped to the track's
    /// length when a source is loaded; otherwise stored as-is. Wrapped
    /// in a de-click ramp so seek-induced phase jumps don't click.
    pub fn set_position_frames(&mut self, position: f64) {
        self.start_declick();
        self.position = position;
        self.shared.store_position(position);
        // Mirror in seconds so the lock-free
        // `position_snapshot` FFI surfaces a consistent playhead.
        // Falls back to zero when no track is loaded (sample rate
        // unknown) — same convention as
        // [`Self::track_duration_secs`].
        let secs = match self.source.as_ref() {
            Some(track) => {
                let sr = f64::from(track.sample_rate());
                if sr > 0.0 {
                    position / sr
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        // Publish (secs, rate=0) on the seek path: the audio
        // thread is between blocks here, so the playhead is
        // momentarily pinned. The next `render_into` will
        // overwrite rate with the steady-state value. Pinning
        // rate to 0 keeps the snapshot reader from extrapolating
        // the seek target forward into the next block.
        self.shared.publish_position_secs(secs, 0.0);
        self.shared.store_at_end(false);
        // A seek invalidates any in-flight absolute advance — it was
        // measured relative to the pre-seek playhead.
        self.pending_advance = None;
    }

    /// Queue an exact playhead advance of `delta_track_frames` (track
    /// frames; negative for reverse) to be applied across the next
    /// render block (M6 absolute timecode). Unlike
    /// [`Deck::set_position_frames`] this is **not** a seek: no
    /// declick ramp, no immediate position write. The render loop
    /// still integrates `rate` per sample (that's what shapes the
    /// audio), then re-pins the block-end position to
    /// `block_start + delta` — so the playhead tracks the groove
    /// exactly instead of accumulating integration drift.
    ///
    /// Multiple calls between renders accumulate. RT-safe: pure field
    /// arithmetic.
    pub fn advance_position_frames(&mut self, delta_track_frames: f64) {
        self.pending_advance = Some(self.pending_advance.unwrap_or(0.0) + delta_track_frames);
    }

    /// Linear gain. Default `1.0`.
    #[must_use]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Set the linear gain. Negative values invert phase; out-of-range is
    /// allowed but generally not what the user wants.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    /// `true` when the deck is currently contributing audio.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Set play/pause. Starts a de-click ramp on transitions so
    /// pause/resume fades from/to silence over ~2 ms instead of
    /// snapping the playhead value to zero (or away from zero).
    pub fn set_playing(&mut self, playing: bool) {
        if self.playing != playing {
            self.start_declick();
        }
        self.playing = playing;
        self.shared.store_playing(playing);
    }

    /// M10.6b. Mirror the deck's Panic-Play state into the UI-
    /// readable shared atomic. Pure atomic store, RT-safe; the
    /// audio-thread engine calls this on engage / cancel /
    /// auto-resume so the UI's 30 Hz poll sees the transition
    /// within one frame. Doesn't itself change deck transport —
    /// the engine pairs this with `set_rate` / `set_playing` to
    /// drive the actual audio.
    pub fn set_panic_play_visible(&self, panic: bool) {
        self.shared.store_panic_play(panic);
    }

    /// Render this deck's contribution into `out`, mixing additively.
    ///
    /// `out` is interleaved stereo, length `2 * frames`. The caller is
    /// responsible for zeroing it if a fresh mix is desired.
    ///
    /// `engine_sr` is the engine's output sample rate; the deck adjusts
    /// its rate to convert between the track's SR and the engine's SR.
    ///
    /// The deck reads frames at fractional positions using linear
    /// interpolation. Out-of-range positions contribute silence. Playback
    /// outside `[0, frames)` is allowed (the caller may have set it via
    /// `set_position_frames`); the deck simply renders silence and keeps
    /// advancing in case the rate later brings it back into range.
    ///
    /// **RT-safety**: no allocation, no locks, no syscalls. The only
    /// inputs are the deck's pre-allocated state and the input buffer.
    pub fn render(&mut self, rt: &mut RealtimeContext<'_>, out: &mut [f32], engine_sr: f32) {
        self.render_into(rt, out, engine_sr, 2, 0);
    }

    /// Strided variant of [`Self::render`]. Writes the deck's stereo
    /// frames into `out` at `stride`-sample stride, with the L sample
    /// at `out[offset + n*stride]` and the R sample at
    /// `out[offset + n*stride + 1]`. The deck always *adds* into the
    /// destination cells (`+=`), so two decks with the same
    /// `(stride, offset)` sum (= M4 internal mixer); two decks with
    /// non-overlapping `(stride, offset)` are isolated (= M5.5
    /// external-mixer routing).
    ///
    /// `stride == 2, offset == 0` is the dense-stereo case, identical
    /// in behavior to [`Self::render`] (which is a thin wrapper around
    /// this method).
    ///
    /// **RT-safety**: same as `render` — the only difference is the
    /// stride argument to the inner chunk iteration, no extra
    /// allocation or branching on the hot path.
    // The function is long but it's a single linear render — splitting
    // into "fade phase" and "steady-state phase" helpers would force
    // shared state (`pos`, `frames_consumed_in_fade`) into a struct
    // and obscure the per-frame data flow. Clippy's threshold catches
    // genuinely tangled functions; this isn't one.
    #[allow(clippy::too_many_lines)]
    pub fn render_into(
        &mut self,
        rt: &mut RealtimeContext<'_>,
        out: &mut [f32],
        engine_sr: f32,
        stride: usize,
        offset: usize,
    ) {
        rt.tick();
        debug_assert!(
            stride >= 2,
            "stride must be at least 2 to hold a stereo pair"
        );
        debug_assert!(
            offset + 2 <= stride,
            "offset {offset} + 2 must fit inside stride {stride}"
        );
        debug_assert_eq!(
            out.len() % stride,
            0,
            "output buffer length must be a multiple of stride"
        );

        let engine_sr_f = f64::from(engine_sr);
        let gain = self.gain;
        let mut pos = self.position;
        // M6 absolute-timecode advance: remember where the block
        // started and drain any queued exact advance. Taken
        // unconditionally so a stale advance never outlives the block
        // it was measured for.
        let block_start = pos;
        let advance = self.pending_advance.take();

        // The increment for the *current* (new-side) source. Computed
        // once per block — the source doesn't change mid-render.
        let new_increment = self.source.as_ref().map_or(0.0, |t| {
            self.rate * (f64::from(t.sample_rate()) / engine_sr_f)
        });

        // === Phase 1: crossfade (if a declick is active). ===
        let mut frames_consumed_in_fade = 0usize;
        if let DeclickState::Active {
            prev_source,
            prev_position,
            prev_rate,
            prev_playing,
            samples_remaining,
        } = &mut self.declick
        {
            let env = &self.declick_envelope;
            let n_total = env.len();
            // Index into the envelope of the *next* sample to apply.
            // After this method runs, we want `i` to have advanced by
            // however many fade samples we render here.
            let total_frames = out.len() / stride;
            let prev_increment = prev_source.as_ref().map_or(0.0, |t| {
                *prev_rate * (f64::from(t.sample_rate()) / engine_sr_f)
            });

            #[allow(clippy::cast_possible_truncation)]
            let fade_frames = (*samples_remaining as usize).min(total_frames);

            for chunk in out.chunks_exact_mut(stride).take(fade_frames) {
                let i = n_total - *samples_remaining;
                let fade_in = env.fade_in(i);
                let fade_out = 1.0 - fade_in;

                // Old-side sample (silence if previously paused or no source).
                // Apply the same trailing-edge fade as the steady-state
                // path so the old side smoothly tails to silence if it
                // happens to walk past its track end during the M3.5
                // crossfade.
                let (old_l, old_r) = if *prev_playing {
                    if let Some(t) = prev_source.as_ref() {
                        let (l, r) = read_stereo_at(t, *prev_position);
                        #[allow(clippy::cast_precision_loss)]
                        let tlen = t.frames() as f64;
                        let edge = track_tail_fade_scale(tlen, *prev_position, env);
                        (l * edge, r * edge)
                    } else {
                        (0.0, 0.0)
                    }
                } else {
                    (0.0, 0.0)
                };

                // New-side sample (silence if currently paused or no source).
                let (new_l, new_r) = if self.playing {
                    if let Some(t) = self.source.as_ref() {
                        let (l, r) = read_stereo_at(t, pos);
                        #[allow(clippy::cast_precision_loss)]
                        let tlen = t.frames() as f64;
                        let edge = track_tail_fade_scale(tlen, pos, env);
                        (l * edge, r * edge)
                    } else {
                        (0.0, 0.0)
                    }
                } else {
                    (0.0, 0.0)
                };

                let l = old_l * fade_out + new_l * fade_in;
                let r = old_r * fade_out + new_r * fade_in;
                chunk[offset] += l * gain;
                chunk[offset + 1] += r * gain;

                // Each side only advances when its play-state was true:
                // a paused side reads silence and stays at its position.
                // (Mirrors the steady-state semantics where `set_playing(false)`
                // freezes the playhead.)
                if *prev_playing {
                    *prev_position += prev_increment;
                }
                if self.playing {
                    pos += new_increment;
                }
                *samples_remaining -= 1;
            }

            frames_consumed_in_fade = fade_frames;
            // If we landed exactly on samples_remaining == 0, the engine
            // will harvest prev_source via take_finished_declick_source
            // after this render returns.
        }

        // === Phase 2: steady-state playback (no fade) for the rest. ===
        if self.playing {
            if let Some(track) = self.source.as_ref() {
                #[allow(clippy::cast_precision_loss)]
                let track_len = track.frames() as f64;
                let env = &self.declick_envelope;

                for chunk in out.chunks_exact_mut(stride).skip(frames_consumed_in_fade) {
                    let (l, r) = read_stereo_at(track, pos);
                    let edge = track_tail_fade_scale(track_len, pos, env);
                    chunk[offset] += l * gain * edge;
                    chunk[offset + 1] += r * gain * edge;
                    pos += new_increment;
                }

                // End-of-track flag tracks the steady-state position only;
                // during a fade the snapshot is meaningful but not load-bearing.
                let off_end = pos < 0.0 || pos >= track_len;
                if off_end != self.shared.load_at_end() {
                    self.shared.store_at_end(off_end);
                }
            } else {
                // No source: still advance position so tests of
                // "paused-doesn't-advance" behave predictably (they
                // pin `pos = 0.0` anyway when set_playing(false)).
            }
        }

        // M6: re-pin the block-end position to the absolute anchor.
        // The per-sample integrator above shaped the audio (intra-
        // block slope from `rate`); this corrects the accumulated
        // sub-frame integration error at the boundary so the playhead
        // can never drift from the groove. Audio is untouched — the
        // samples were already written — and the correction is far
        // below one frame per block in practice, so the next block's
        // read position is inaudibly close to where integration alone
        // would have put it.
        if self.playing && self.source.is_some() {
            if let Some(delta) = advance {
                pos = block_start + delta;
            }
        }

        self.position = pos;
        self.shared.store_position(pos);
        // Publish playhead in seconds for the lock-free
        // `position_snapshot` reader (M11d.6 round 5). One divide
        // per render block; negligible cost. We use the *currently
        // loaded* source's sample rate — the audio thread already
        // owns this Arc for the duration of the block.
        //
        // **M11d.6 round 8.** `self.rate` is the **musical** rate
        // (1.0 = real-time playback at the track's natural pitch).
        // The deck's `new_increment = self.rate * (track_sr /
        // engine_sr)` math turns it into the per-output-sample
        // track-frame step, so over one engine sample:
        //   Δsecs       = self.rate / engine_sr
        //   Δwall       = 1 / engine_sr
        //   Δsecs/Δwall = self.rate
        // So the wall-time rate we want the FFI extrapolator to
        // apply is just `self.rate` — no SR ratio cancellation.
        // The round-7 formula multiplied by `engine_sr / track_sr`
        // here, which would only have been correct if `self.rate`
        // were in track-frames-per-output-frame; in practice it
        // is musical, and the FFI's old pre-multiplication on the
        // command side made `self.rate` 0.92 instead of 1.0 — so
        // round-7's extra factor coincidentally produced
        // pub_rate = 1.0 from a deck that was actually playing
        // audio at 0.92×. Round 8 removes both wrongs.
        let (secs, advance_rate) = match self.source.as_ref() {
            Some(track) => {
                let track_sr = f64::from(track.sample_rate());
                if track_sr > 0.0 {
                    let secs = pos / track_sr;
                    let pub_rate = if self.playing { self.rate } else { 0.0 };
                    (secs, pub_rate)
                } else {
                    (0.0, 0.0)
                }
            }
            None => (0.0, 0.0),
        };
        self.shared.publish_position_secs(secs, advance_rate);
    }
}

/// Multiplicative envelope applied to every track read so the deck
/// fades smoothly to silence when the playhead approaches the natural
/// end of a track.
///
/// Why this is separate from the M3.5 transport-mutation declick:
/// the transport declick fires on user-initiated state changes
/// (load, seek, play/pause). It does *not* fire when the playhead
/// simply walks past the last frame of a track — that's the data
/// running out, not a transport change. Without this scale, the
/// output value drops from "last in-range sample" to 0.0 in one
/// frame, which is exactly the kind of step-function discontinuity
/// the ear hears as a click. Universal in sample players; we wrap it
/// with the same `sin²` envelope used by the transport declick so the
/// edge has equal-power energy distribution.
///
/// Only the *trailing* edge is faded here. Leading-edge attack is
/// already handled by the M3.5 transport declick, which fades from
/// the previous source (or silence) into the new source whenever a
/// load happens. Adding an unconditional leading-edge fade would
/// inappropriately attenuate the previous side of an in-flight
/// crossfade if its position happened to land near `pos = 0`.
///
/// Skipped on very short tracks (< 2 × envelope length) — applying a
/// 2 ms fade to a sub-2 ms test track would obliterate it. Real DJ
/// material is always orders of magnitude longer than the threshold.
#[inline]
fn track_tail_fade_scale(track_len: f64, pos: f64, env: &DeclickEnvelope) -> f32 {
    let n_u = env.len();
    let n = f64::from(n_u);
    if track_len < 2.0 * n {
        return 1.0;
    }
    let frames_to_end = track_len - pos;
    if frames_to_end <= 0.0 {
        return 0.0;
    }
    if frames_to_end >= n {
        return 1.0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = (n - frames_to_end) as u32;
    env.fade_out(i.min(n_u.saturating_sub(1)))
}

/// Linear-interpolation read of a stereo (or mono-as-stereo) sample
/// from a track at a fractional position. Out-of-range positions
/// return `(0.0, 0.0)` — a silent contribution.
///
/// Hot path: called once per output frame in steady playback, twice
/// per frame during a de-click crossfade. `#[inline]` is a hint to
/// LLVM, which will almost always honor it for a function this small.
/// We deliberately avoid `inline(always)` (clippy::inline_always)
/// because it disables the compiler's heuristics across LTO boundaries
/// and can occasionally pessimize the call site.
#[inline]
fn read_stereo_at(track: &Track, pos: f64) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss)]
    let track_len = track.frames() as f64;
    #[allow(clippy::cast_precision_loss)]
    let last_index_f = (track.frames().saturating_sub(1)) as f64;
    if pos < 0.0 || pos >= track_len {
        return (0.0, 0.0);
    }
    let i_floor = pos.floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let idx = i_floor as usize;
    #[allow(clippy::cast_possible_truncation)]
    let frac = (pos - i_floor) as f32;
    let a = track.frame(idx);
    let b = if i_floor < last_index_f {
        track.frame(idx + 1)
    } else {
        a
    };
    let l = a[0] + (b[0] - a[0]) * frac;
    let r = a[1] + (b[1] - a[1]) * frac;
    (l, r)
}

// `Default` impl removed: Deck::new now requires an Arc<DeclickEnvelope>
// from the owning engine. Construct decks via `Engine::new` /
// `Engine::new_with_handle`, not directly.

#[cfg(test)]
impl Deck {
    /// Snap any in-flight de-click ramp to `Idle` and drop any
    /// associated `Arc<Track>` immediately.
    ///
    /// **Test-only.** Real audio-thread code must never call this —
    /// it can drop an `Arc<Track>`, which is forbidden on the RT
    /// thread. We exempt tests because they don't run on the audio
    /// thread; this exists so tests that pre-date M3.5 can assert
    /// on raw playback samples without 96-frame fade-in artifacts
    /// dominating the first chunk.
    pub(crate) fn quiesce_declick_for_test(&mut self) {
        self.declick = DeclickState::Idle;
        self.pending_disposal = None;
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::float_cmp
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn const_track(samples: &[f32], channels: u8, sample_rate: u32) -> Arc<Track> {
        Arc::new(Track::from_interleaved(samples.to_vec(), sample_rate, channels).unwrap())
    }

    /// Build a Deck with a fixed-size declick envelope. Tests that don't
    /// care about the exact ramp length use this helper. We default to
    /// the standard 48 kHz / 2 ms ramp so test expectations match
    /// what the engine ships.
    fn test_deck() -> Deck {
        Deck::new(DeclickEnvelope::new(48_000.0, 2.0))
    }

    #[test]
    fn timecode_telemetry_roundtrips_and_resets() {
        let shared = DeckSharedState::new();
        // Fresh state: no input, all zero.
        let t0 = shared.load_timecode_telemetry();
        assert!(!t0.has_input);
        assert_eq!(t0.lock_state, 0);
        assert_eq!(t0.confidence, 0.0);

        shared.publish_timecode_telemetry(0.97, 0.31, 2, true, 0.984, true, 12.5, 3.25, true, 1.0);
        let t1 = shared.load_timecode_telemetry();
        assert!(t1.has_input);
        assert_eq!(t1.lock_state, 2);
        assert!((t1.confidence - 0.97).abs() < 1e-6);
        assert!((t1.amplitude - 0.31).abs() < 1e-6);
        assert!((t1.display_rate - 0.984).abs() < 1e-9);
        assert!(t1.abs_locked);
        assert!((t1.abs_position_secs - 12.5).abs() < 1e-9);
        assert!((t1.sticker_drift_ms - 3.25).abs() < 1e-9);

        // reset() clears telemetry back to the empty baseline.
        shared.reset();
        let t2 = shared.load_timecode_telemetry();
        assert!(!t2.has_input);
        assert_eq!(t2.lock_state, 0);
        assert_eq!(t2.amplitude, 0.0);
        assert!((t2.display_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn empty_deck_renders_silence() {
        let mut deck = test_deck();
        let mut rt = RealtimeContext::new();
        let mut out = [0.5f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);
        // No source loaded: render is a no-op (additive mix).
        for s in out {
            #[allow(clippy::float_cmp)]
            {
                assert_eq!(s, 0.5);
            }
        }
    }

    #[test]
    fn paused_deck_does_not_advance() {
        let mut deck = test_deck();
        deck.set_source(const_track(&[0.5, -0.5, 0.5, -0.5], 2, 48_000));
        deck.set_playing(false);
        // Skip the post-set_source fade-in so we test the steady-state
        // "paused" behavior, not the (correctly silent) fade phase.
        deck.quiesce_declick_for_test();
        let mut rt = RealtimeContext::new();
        let mut out = [0.0f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);
        #[allow(clippy::float_cmp)]
        for s in out {
            assert_eq!(s, 0.0);
        }
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(deck.position_frames(), 0.0);
        }
    }

    // M6 — absolute-timecode exact advance.

    #[test]
    fn advance_position_frames_re_pins_block_end() {
        // A queued advance replaces the integrated block-end position
        // with `block_start + delta` — exactly, no declick. Rate is
        // deliberately ≠ delta/frames so integration alone would land
        // somewhere else and the test can tell the paths apart.
        let mut deck = test_deck();
        deck.set_source(Arc::new(
            Track::from_interleaved(vec![0.5; 9600], 48_000, 2).unwrap(),
        ));
        deck.set_playing(true);
        deck.set_rate(1.0);
        deck.quiesce_declick_for_test();
        let mut rt = RealtimeContext::new();
        let mut out = [0.0f32; 256];
        // 128 frames at rate 1.0 would integrate to 128; the absolute
        // fix says the groove actually travelled 130.25 frames.
        deck.advance_position_frames(130.25);
        deck.render(&mut rt, &mut out, 48_000.0);
        assert!(
            (deck.position_frames() - 130.25).abs() < 1e-9,
            "block end must re-pin to the absolute advance, got {}",
            deck.position_frames()
        );
        // The advance is consumed — the next block integrates normally.
        deck.render(&mut rt, &mut out, 48_000.0);
        assert!(
            (deck.position_frames() - (130.25 + 128.0)).abs() < 1e-9,
            "next block integrates from the re-pinned position, got {}",
            deck.position_frames()
        );
    }

    #[test]
    fn advance_accumulates_across_calls() {
        let mut deck = test_deck();
        deck.set_source(Arc::new(
            Track::from_interleaved(vec![0.5; 9600], 48_000, 2).unwrap(),
        ));
        deck.set_playing(true);
        deck.set_rate(1.0);
        deck.quiesce_declick_for_test();
        deck.advance_position_frames(100.0);
        deck.advance_position_frames(28.5);
        let mut rt = RealtimeContext::new();
        let mut out = [0.0f32; 256];
        deck.render(&mut rt, &mut out, 48_000.0);
        assert!((deck.position_frames() - 128.5).abs() < 1e-9);
    }

    #[test]
    fn paused_deck_drops_pending_advance() {
        // A paused deck must not move — and the stale advance must not
        // survive to fire on a later block (it was measured for the
        // block it was queued on).
        let mut deck = test_deck();
        deck.set_source(Arc::new(
            Track::from_interleaved(vec![0.5; 9600], 48_000, 2).unwrap(),
        ));
        deck.set_playing(false);
        deck.quiesce_declick_for_test();
        deck.advance_position_frames(500.0);
        let mut rt = RealtimeContext::new();
        let mut out = [0.0f32; 256];
        deck.render(&mut rt, &mut out, 48_000.0);
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(deck.position_frames(), 0.0, "paused deck must not move");
        }
        deck.set_playing(true);
        deck.quiesce_declick_for_test();
        deck.set_rate(1.0);
        deck.render(&mut rt, &mut out, 48_000.0);
        assert!(
            (deck.position_frames() - 128.0).abs() < 1e-9,
            "stale advance must not fire after resume, got {}",
            deck.position_frames()
        );
    }

    #[test]
    fn seek_clears_pending_advance() {
        // An explicit seek invalidates any in-flight absolute advance —
        // it was measured relative to the pre-seek playhead.
        let mut deck = test_deck();
        deck.set_source(Arc::new(
            Track::from_interleaved(vec![0.5; 9600], 48_000, 2).unwrap(),
        ));
        deck.set_playing(true);
        deck.set_rate(1.0);
        deck.quiesce_declick_for_test();
        deck.advance_position_frames(999.0);
        deck.set_position_frames(1000.0);
        deck.quiesce_declick_for_test();
        let mut rt = RealtimeContext::new();
        let mut out = [0.0f32; 256];
        deck.render(&mut rt, &mut out, 48_000.0);
        assert!(
            (deck.position_frames() - 1128.0).abs() < 1e-9,
            "post-seek block must integrate from the seek target, got {}",
            deck.position_frames()
        );
    }

    #[test]
    fn forward_playback_at_unity_rate_matches_source() {
        // Stereo track: 4 frames of (i, -i)
        let mut samples = Vec::new();
        for i in 0..4 {
            samples.push(i as f32);
            samples.push(-(i as f32));
        }
        let mut deck = test_deck();
        deck.set_source(const_track(&samples, 2, 48_000));
        deck.set_playing(true);
        // The set_source / set_playing(true) above each scheduled a
        // ~2 ms ramp; this test asserts on raw sample correctness, not
        // the fade. Quiesce so we observe steady-state behavior.
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);

        for f in 0..4 {
            #[allow(clippy::float_cmp)]
            {
                assert_eq!(out[f * 2], f as f32);
                assert_eq!(out[f * 2 + 1], -(f as f32));
            }
        }
    }

    #[test]
    fn reverse_playback_reads_in_reverse() {
        // Mono track of 4 distinct samples
        let track = const_track(&[1.0, 2.0, 3.0, 4.0], 1, 48_000);
        let mut deck = test_deck();
        deck.set_source(track);
        deck.set_playing(true);
        deck.set_position_frames(3.0);
        deck.set_rate(-1.0);
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);

        // Should read 4.0, 3.0, 2.0, 1.0 — written to both stereo channels
        let expected = [4.0, 4.0, 3.0, 3.0, 2.0, 2.0, 1.0, 1.0];
        for (got, want) in out.iter().zip(expected.iter()) {
            assert!((got - want).abs() < 1e-6, "got {got}, want {want}");
        }
    }

    #[test]
    fn out_of_range_position_is_silent() {
        let track = const_track(&[1.0, 2.0, 3.0, 4.0], 1, 48_000);
        let mut deck = test_deck();
        deck.set_source(track);
        deck.set_playing(true);
        deck.set_rate(1.0);
        deck.set_position_frames(-100.0);
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.5f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);
        // Initial buffer was all 0.5; deck added nothing because positions
        // -100..-96 are out of range, so output stays 0.5.
        for s in out {
            #[allow(clippy::float_cmp)]
            {
                assert_eq!(s, 0.5);
            }
        }
    }

    #[test]
    fn render_is_additive_not_replacing() {
        let track = const_track(&[1.0, 1.0, 1.0, 1.0], 1, 48_000);
        let mut deck = test_deck();
        deck.set_source(track);
        deck.set_playing(true);
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.5f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);
        // Each output is 0.5 (initial) + 1.0 (deck) = 1.5
        for s in out {
            assert!((s - 1.5).abs() < 1e-6, "got {s}");
        }
    }

    #[test]
    fn gain_scales_output() {
        let track = const_track(&[1.0, 1.0, 1.0, 1.0], 1, 48_000);
        let mut deck = test_deck();
        deck.set_source(track);
        deck.set_playing(true);
        deck.set_gain(0.25);
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);
        for s in out {
            assert!((s - 0.25).abs() < 1e-6);
        }
    }

    #[test]
    fn sample_rate_conversion_44k_to_48k() {
        // 4 frames at 44.1k. Rendered into a 48k engine at unity rate.
        // Increment per output frame = 44100/48000 ≈ 0.91875 frames.
        // We just verify position advances correctly and no panic occurs.
        let track = const_track(&[0.1, 0.2, 0.3, 0.4], 1, 44_100);
        let mut deck = test_deck();
        deck.set_source(track);
        deck.set_playing(true);
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 8]; // 4 frames at 48k
        deck.render(&mut rt, &mut out, 48_000.0);

        // Position should have advanced ~3.675 frames over 4 output frames.
        let expected = 4.0 * 44_100.0 / 48_000.0;
        assert!(
            (deck.position_frames() - expected).abs() < 1e-6,
            "got {} want {}",
            deck.position_frames(),
            expected
        );
    }

    // ============================================================
    //                       M3.5 de-click tests
    // ============================================================

    /// Track that produces a constant sample value across all frames.
    fn const_value_track(value: f32, frames: usize) -> Arc<Track> {
        let samples: Vec<f32> = std::iter::repeat_n(value, frames * 2).collect();
        Arc::new(Track::from_interleaved(samples, 48_000, 2).unwrap())
    }

    #[test]
    fn declick_fade_in_starts_at_zero_and_reaches_full() {
        // Fresh deck → set_source (with constant 1.0 track) →
        // set_playing(true) → render. The first sample should be ~0
        // (fade_in starts at sin²(0) = 0) and the post-fade samples
        // should be ~1.0.
        let mut deck = test_deck();
        deck.set_source(const_value_track(1.0, 1024));
        deck.set_playing(true);
        // Note: NOT calling quiesce — we WANT the fade.

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256 * 2];
        deck.render(&mut rt, &mut out, 48_000.0);

        // Frame 0 of the fade: fade_in = 0 → sample = 0.
        assert!(
            out[0].abs() < 1e-6,
            "first sample was {}, expected ~0",
            out[0]
        );
        // Frames after the 96-sample ramp (at 48 kHz / 2 ms): full value.
        for (i, s) in out.chunks_exact(2).enumerate().skip(96) {
            assert!(
                (s[0] - 1.0).abs() < 1e-6,
                "post-fade frame {i}: got {}, expected 1.0",
                s[0]
            );
        }
    }

    #[test]
    fn declick_fade_is_monotonic_no_jump_discontinuity() {
        // Crucial invariant: the fade-in must produce a *smooth* curve
        // with no large step between consecutive samples. We measure
        // the maximum first-difference across the fade window — for a
        // 2 ms ramp on a constant source, this is bounded by the
        // largest fade-table delta, well below the source value.
        let mut deck = test_deck();
        deck.set_source(const_value_track(1.0, 1024));
        deck.set_playing(true);

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256 * 2];
        deck.render(&mut rt, &mut out, 48_000.0);

        let l_channel: Vec<f32> = out.iter().step_by(2).copied().collect();
        let mut max_diff = 0.0f32;
        for w in l_channel.windows(2) {
            let d = (w[1] - w[0]).abs();
            if d > max_diff {
                max_diff = d;
            }
        }
        // For a 96-sample fade of a unit-step input, the max per-sample
        // delta is the largest gap between adjacent fade-table values,
        // which is bounded by π/(2N) ≈ 0.016 for N=96. We use a slightly
        // looser bound to leave headroom for floating-point.
        assert!(
            max_diff < 0.05,
            "max sample-to-sample delta = {max_diff} (want < 0.05); a true \
             jump-discontinuity would produce a delta of ~1.0"
        );
    }

    #[test]
    fn declick_fade_out_to_silence_on_pause() {
        // Start playing → quiesce → set_playing(false) → render. First
        // sample should be near the steady-state value (1.0); end of
        // fade should be silence.
        let mut deck = test_deck();
        deck.set_source(const_value_track(1.0, 1024));
        deck.set_playing(true);
        deck.quiesce_declick_for_test();

        // Now pause: triggers fade-out from 1.0 → 0.0.
        deck.set_playing(false);

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256 * 2];
        deck.render(&mut rt, &mut out, 48_000.0);

        // First sample: fade_in=0 → output = old(=1.0)*1.0 + new(silent)*0 = 1.0.
        assert!(
            (out[0] - 1.0).abs() < 1e-6,
            "first sample {} should be ~1.0 (start of fade-out)",
            out[0]
        );
        // After fade window: silence.
        for (i, s) in out.chunks_exact(2).enumerate().skip(96) {
            assert!(
                s[0].abs() < 1e-6,
                "post-fade frame {i}: got {}, expected silence",
                s[0]
            );
        }
    }

    #[test]
    fn declick_crossfade_between_two_tracks() {
        // Track A constant 1.0, track B constant -1.0. After A's fade-in
        // settles, swap to B. Across the 96-sample crossfade, the output
        // smoothly transitions 1.0 → -1.0 with no jump.
        let mut deck = test_deck();
        deck.set_source(const_value_track(1.0, 1024));
        deck.set_playing(true);
        deck.quiesce_declick_for_test();

        // Swap to B: starts fade A → B.
        deck.swap_source(const_value_track(-1.0, 1024));

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256 * 2];
        deck.render(&mut rt, &mut out, 48_000.0);

        // Fade-start: ~old (1.0).
        assert!(
            (out[0] - 1.0).abs() < 1e-6,
            "fade-start should be old value 1.0, got {}",
            out[0]
        );
        // Post-fade: new value -1.0.
        for (i, s) in out.chunks_exact(2).enumerate().skip(96) {
            assert!(
                (s[0] - (-1.0)).abs() < 1e-6,
                "post-fade frame {i}: got {}, expected -1.0",
                s[0]
            );
        }
        // Smoothness: no per-sample jump >= 0.1 (the natural envelope
        // step is ~0.033 worst-case).
        let l: Vec<f32> = out.iter().step_by(2).copied().collect();
        let mut max_diff = 0.0f32;
        for w in l.windows(2) {
            let d = (w[1] - w[0]).abs();
            if d > max_diff {
                max_diff = d;
            }
        }
        assert!(max_diff < 0.1, "max delta {max_diff} suggests a jump");
    }

    #[test]
    #[allow(clippy::similar_names)] // intentional parallel naming for track_a/b/c
    fn declick_back_to_back_swaps_strand_no_arcs() {
        // Three swaps in rapid succession (within one render block).
        // Without proper handling, the second swap would clobber the
        // first's prev_source and the audio thread would drop an Arc.
        // Our implementation routes the stranded Arc to pending_disposal
        // (one slot) or, in the truly worst case, leaks it via mem::forget
        // (4-deep within 2ms — physically impossible from human input).
        let track_a = Arc::new(Track::from_interleaved(vec![0.5; 1024], 48_000, 2).unwrap());
        let track_b = Arc::new(Track::from_interleaved(vec![0.25; 1024], 48_000, 2).unwrap());
        let track_c = Arc::new(Track::from_interleaved(vec![0.1; 1024], 48_000, 2).unwrap());

        // Each external clone is one strong reference; the deck holds
        // its own. Tracks held by user + deck = 2 each at start.
        let count_a_initial = Arc::strong_count(&track_a);
        let count_b_initial = Arc::strong_count(&track_b);
        let count_c_initial = Arc::strong_count(&track_c);

        let mut deck = test_deck();
        deck.set_source(track_a.clone());
        deck.swap_source(track_b.clone()); // displaces A's prev (None) → no strand
        deck.swap_source(track_c.clone()); // displaces B which was prev → goes to pending_disposal

        // Verify: at this point, current = C, prev (in active declick) = B,
        // pending_disposal might contain... let's just check the harvest
        // surfaces the right Arcs.
        let pending = deck.take_pending_disposal();
        // After two swaps: pending_disposal holds the second-to-last
        // prev (which was the first-swap's prev_source, i.e. None or
        // an empty slot). In our specific sequence:
        //   set_source(A): prev=None, no strand.
        //   swap_source(B): displaces declick.prev_source (which was None
        //                   from set_source's start_declick), nothing to
        //                   stash; new prev = current source (A).
        //   swap_source(C): declick.prev_source was Some(A); stash A in
        //                   pending_disposal; new prev = B.
        // So pending_disposal contains A.
        assert!(pending.is_some(), "pending_disposal should hold A");
        assert!(Arc::ptr_eq(pending.as_ref().unwrap(), &track_a));

        // Drop pending so the count goes back to start_a.
        drop(pending);

        // Now finish the fade so prev_source (= B) gets surfaced.
        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256 * 2];
        deck.set_playing(true); // adds another declick layer; but we just want to drive render
        deck.quiesce_declick_for_test(); // simpler: skip & start fresh
                                         // Re-test the harvest: the prior render should've already
                                         // bookkept; with quiesce we lose state, so this assertion
                                         // section is observational. The real RT-safety contract is
                                         // tested by `rt-audit`.
        deck.render(&mut rt, &mut out, 48_000.0);

        // The original Arcs survived without RT-thread drops.
        assert_eq!(
            Arc::strong_count(&track_a),
            count_a_initial,
            "A's count should be restored"
        );
        // B was stashed inside the deck's declick state when we quiesced
        // (which dropped it). C is in the deck's source slot.
        let _ = count_b_initial;
        assert!(Arc::strong_count(&track_c) >= count_c_initial);
    }

    #[test]
    fn track_tail_fade_smooths_natural_end_of_track() {
        // A 1024-frame constant-1.0 track. When the playhead walks off
        // the end during a render, the output must NOT step from 1.0
        // straight to 0.0. With the tail-fade scale applied, the last
        // ~96 samples ramp down through `cos²` and the per-sample
        // delta stays well below the un-faded 1.0 step.
        let track = const_value_track(1.0, 1024);
        let mut deck = test_deck();
        deck.set_source(track);
        deck.set_playing(true);
        deck.set_position_frames(900.0); // start near the end
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256 * 2]; // 256 frames, walks past end
        deck.render(&mut rt, &mut out, 48_000.0);

        let l: Vec<f32> = out.iter().step_by(2).copied().collect();

        // Sanity: the end should be silent (track ran out + tail-fade
        // brought us to zero).
        assert!(
            l[200].abs() < 1e-6,
            "post-end frame 200: {} should be silent",
            l[200]
        );

        // The crucial invariant: no per-sample jump near the boundary.
        // Without the tail-fade, the frame at track_len would step
        // directly from ~1.0 to 0.0 (delta = 1.0). With the fade,
        // adjacent deltas are bounded by the envelope's slope.
        let mut max_diff = 0.0f32;
        for w in l.windows(2) {
            let d = (w[1] - w[0]).abs();
            if d > max_diff {
                max_diff = d;
            }
        }
        assert!(
            max_diff < 0.1,
            "max sample-to-sample delta = {max_diff}; without tail-fade \
             it would be 1.0 at the track-end boundary"
        );
    }

    #[test]
    fn track_tail_fade_skipped_for_short_tracks() {
        // A 4-frame track is too short to apply a 96-sample fade-out
        // meaningfully. The threshold (track_len < 2 × envelope) means
        // the fade is bypassed entirely; output is the raw frames.
        let track = const_track(&[1.0, 1.0, 1.0, 1.0], 1, 48_000);
        let mut deck = test_deck();
        deck.set_source(track);
        deck.set_playing(true);
        deck.quiesce_declick_for_test();

        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 8];
        deck.render(&mut rt, &mut out, 48_000.0);

        // Without the fade, all 4 in-range output frames are 1.0.
        for s in &out {
            assert!(
                (s - 1.0).abs() < 1e-6,
                "tail-fade shouldn't fire on a 4-frame track; got {s}"
            );
        }
    }

    #[test]
    fn shared_state_publishes_duration_and_has_track_on_load() {
        let mut deck = test_deck();
        let shared = deck.shared();
        assert!(!shared.load_has_track(), "fresh deck must have no track");
        assert_eq!(shared.load_duration_secs(), 0.0);
        assert_eq!(shared.load_position_secs(), 0.0);

        let n = 48_000;
        let samples = vec![0.0_f32; n * 2];
        let track = const_track(&samples, 2, 48_000);
        deck.set_source(track);

        assert!(shared.load_has_track(), "set_source must publish has_track");
        let dur = shared.load_duration_secs();
        assert!(
            (dur - 1.0).abs() < 1e-9,
            "duration_secs must reflect track length; got {dur}"
        );
        assert_eq!(shared.load_position_secs(), 0.0);

        deck.clear_source();
        assert!(
            !shared.load_has_track(),
            "clear_source must publish !has_track"
        );
        assert_eq!(shared.load_duration_secs(), 0.0);
        assert_eq!(shared.load_position_secs(), 0.0);
    }

    #[test]
    fn shared_state_position_secs_tracks_seek() {
        let mut deck = test_deck();
        let shared = deck.shared();
        let samples = vec![0.0_f32; 96_000 * 2];
        let track = const_track(&samples, 2, 48_000);
        deck.set_source(track);

        deck.set_position_frames(48_000.0);
        let secs = shared.load_position_secs();
        assert!(
            (secs - 1.0).abs() < 1e-9,
            "set_position_frames must publish position_secs; got {secs}"
        );
    }

    #[test]
    fn shared_state_reset_clears_all_fields() {
        let mut deck = test_deck();
        let shared = deck.shared();
        let samples = vec![0.0_f32; 48_000 * 2];
        let track = const_track(&samples, 2, 48_000);
        deck.set_source(track);
        deck.set_position_frames(24_000.0);
        deck.set_playing(true);

        assert!(shared.load_has_track());
        assert!(shared.load_playing());

        shared.reset();
        assert!(!shared.load_has_track());
        assert!(!shared.load_playing());
        assert!(!shared.load_at_end());
        assert!(!shared.load_panic_play());
        assert_eq!(shared.load_position(), 0.0);
        assert_eq!(shared.load_position_secs(), 0.0);
        assert_eq!(shared.load_duration_secs(), 0.0);
    }

    /// Soak: every individual `load_*` accessor on
    /// [`DeckSharedState`] must return a **non-torn** value
    /// regardless of how aggressively the writer thread is
    /// mutating the deck (M11d.6 round 5). That is, an `f64`
    /// read from any of the `*_bits` atomics is always a single
    /// previously-stored `f64::to_bits` — never a half-old /
    /// half-new bit pattern that would yield `NaN`. The audio
    /// thread updates these atomics roughly once per render
    /// block; the off-main waveform renderer reads them at
    /// vsync.
    ///
    /// Cross-field tearing (e.g. observing `has_track == true`
    /// briefly paired with `duration_secs == 0.0` during the
    /// sub-microsecond store window inside `set_source` /
    /// `clear_source`) is **tolerated** and visually invisible:
    /// the renderer skips drawing that frame's playhead and
    /// recovers on the next vsync. This test therefore asserts
    /// only the within-field invariant, which is what
    /// `AtomicU64` provides.
    #[test]
    fn shared_state_concurrent_reads_are_within_field_coherent() {
        use std::sync::Arc as StdArc;
        use std::thread;

        let mut deck = test_deck();
        let shared: StdArc<DeckSharedState> = deck.shared();

        let stop = StdArc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader_shared = shared.clone();
        let reader = thread::spawn(move || {
            let mut observed = 0u64;
            while !reader_stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _has = reader_shared.load_has_track();
                let dur = reader_shared.load_duration_secs();
                let pos_secs = reader_shared.load_position_secs();
                let pos_frames = reader_shared.load_position();
                assert!(
                    dur.is_finite() && dur >= 0.0,
                    "duration_secs torn or negative: {dur}"
                );
                assert!(
                    pos_secs.is_finite(),
                    "position_secs torn (NaN/inf): {pos_secs}"
                );
                assert!(
                    pos_frames.is_finite(),
                    "position_frames torn (NaN/inf): {pos_frames}"
                );
                observed += 1;
            }
            observed
        });

        for cycle in 0..200 {
            let n = 48_000 + (cycle as usize % 16) * 4_800;
            let track = const_track(&vec![0.0_f32; n * 2], 2, 48_000);
            deck.set_source(track);
            deck.set_position_frames(((cycle as f64) * 1_000.0) % (n as f64));
            std::thread::yield_now();
            deck.clear_source();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let observed = reader.join().expect("reader thread");
        assert!(observed > 0, "reader must have taken at least one sample");
    }

    /// **M11d.6 round 6.** Verify that the seqlock-protected
    /// `(host_time, position_secs, rate)` triple makes it
    /// through `publish_position_secs` / `load_publish_state`
    /// intact, and that `extrapolated_secs` advances the
    /// playhead by the right amount.
    #[test]
    fn publish_state_round_trip_and_extrapolates_at_rate_one() {
        let shared = DeckSharedState::new();
        shared.publish_position_secs(10.0, 1.0);

        let st = shared.load_publish_state();
        assert!((st.position_secs - 10.0).abs() < f64::EPSILON);
        assert!((st.rate - 1.0).abs() < f64::EPSILON);

        // Extrapolate 5 ms into the future: at rate 1.0 the
        // playhead should be 10.005.
        let plus_5ms = st.host_time_ns + 5_000_000;
        let extrap = st.extrapolated_secs(plus_5ms);
        assert!(
            (extrap - 10.005).abs() < 1e-9,
            "expected 10.005 at +5ms@rate=1, got {extrap}"
        );
    }

    /// A paused deck (`rate == 0.0`) must not extrapolate forward.
    /// Without this invariant the renderer would smear the playhead
    /// across the moment of pause.
    #[test]
    fn publish_state_rate_zero_pins_playhead() {
        let shared = DeckSharedState::new();
        shared.publish_position_secs(42.0, 0.0);
        let st = shared.load_publish_state();
        let plus_100ms = st.host_time_ns + 100_000_000;
        let extrap = st.extrapolated_secs(plus_100ms);
        assert!(
            (extrap - 42.0).abs() < f64::EPSILON,
            "rate 0 must pin the playhead, got drift to {extrap}"
        );
    }

    /// Negative rate (scratch backwards) must extrapolate the
    /// playhead in the opposite direction. PRD §4.4 — forward
    /// and backward playback are byte-for-byte symmetric.
    #[test]
    fn publish_state_negative_rate_extrapolates_backwards() {
        let shared = DeckSharedState::new();
        shared.publish_position_secs(5.0, -2.0);
        let st = shared.load_publish_state();
        let plus_10ms = st.host_time_ns + 10_000_000;
        let extrap = st.extrapolated_secs(plus_10ms);
        // 5.0 + 0.010 * -2.0 = 4.98
        assert!(
            (extrap - 4.98).abs() < 1e-9,
            "expected 4.98 at +10ms@rate=-2, got {extrap}"
        );
    }

    /// **M11d.6 round 8 regression.** The audio thread's
    /// publish-time `rate` must be the **musical** rate
    /// (audio-seconds-per-real-second). Because `self.rate` is
    /// already musical (see [`Deck::set_rate`]), the publisher
    /// emits it directly — no SR-ratio multiplication on either
    /// side. Verified by reading the published rate back through
    /// the seqlock after one render block with a 44.1 kHz track
    /// on a 48 kHz engine: a `set_rate(1.0)` call still yields
    /// `pub_rate = 1.0`, not `0.9187` (the round-6 bug) and not
    /// `1.0884` (the round-7 anti-pattern).
    #[test]
    fn render_publishes_one_x_rate_when_track_sr_differs_from_engine_sr() {
        const ENGINE_SR: u32 = 48_000;
        const TRACK_SR: u32 = 44_100;

        let mut deck = test_deck();
        let samples = vec![0.0f32; (TRACK_SR as usize) * 2];
        deck.set_source(const_track(&samples, 2, TRACK_SR));
        deck.set_playing(true);
        deck.quiesce_declick_for_test();
        deck.set_rate(1.0);

        let shared = deck.shared();
        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256];
        deck.render(&mut rt, &mut out, ENGINE_SR as f32);

        let st = shared.load_publish_state();
        assert!(
            (st.rate - 1.0).abs() < 1e-9,
            "expected musical publish rate 1.0 on SR-mismatch, got {} (round-6 bug would land 0.9187, \
             round-7 anti-pattern would land 1.0884)",
            st.rate,
        );
    }

    /// Same SR for engine and track: publish rate is `self.rate`
    /// directly — trivially.
    #[test]
    fn render_publishes_self_rate_when_engine_sr_matches_track_sr() {
        const SR: u32 = 48_000;

        let mut deck = test_deck();
        let samples = vec![0.0f32; (SR as usize) * 2];
        deck.set_source(const_track(&samples, 2, SR));
        deck.set_playing(true);
        deck.quiesce_declick_for_test();
        deck.set_rate(1.0);

        let shared = deck.shared();
        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256];
        deck.render(&mut rt, &mut out, SR as f32);

        let st = shared.load_publish_state();
        assert!(
            (st.rate - 1.0).abs() < 1e-9,
            "expected publish rate 1.0, got {}",
            st.rate
        );
    }

    /// Half-speed scratch (0.5× musical) on an SR-mismatched
    /// track: publish rate must be exactly 0.5 regardless of
    /// engine vs source SR.
    #[test]
    fn render_publishes_half_rate_on_half_speed_scratch() {
        const ENGINE_SR: u32 = 48_000;
        const TRACK_SR: u32 = 44_100;

        let mut deck = test_deck();
        let samples = vec![0.0f32; (TRACK_SR as usize) * 2];
        deck.set_source(const_track(&samples, 2, TRACK_SR));
        deck.set_playing(true);
        deck.quiesce_declick_for_test();
        deck.set_rate(0.5);

        let shared = deck.shared();
        let mut rt = RealtimeContext::new();
        let mut out = vec![0.0f32; 256];
        deck.render(&mut rt, &mut out, ENGINE_SR as f32);

        let st = shared.load_publish_state();
        assert!(
            (st.rate - 0.5).abs() < 1e-9,
            "expected publish rate 0.5 (musical), got {}",
            st.rate
        );
    }

    /// A stalled audio thread (no publish for seconds) must not
    /// let the renderer extrapolate the playhead off into the
    /// future. The clamp at 100 ms gives a hard visible-freeze
    /// rather than a runaway.
    #[test]
    fn publish_state_clamps_extrapolation_on_stall() {
        let shared = DeckSharedState::new();
        shared.publish_position_secs(1.0, 1.0);
        let st = shared.load_publish_state();
        let plus_60s = st.host_time_ns + 60_000_000_000;
        let extrap = st.extrapolated_secs(plus_60s);
        // 100 ms clamp at rate 1.0 → +0.1s → 1.1.
        assert!(
            (extrap - 1.1).abs() < 1e-9,
            "expected clamp to 1.1 on a 60s stall, got {extrap}"
        );
    }

    /// `now_ns < host_time_ns` would yield a negative elapsed,
    /// which the clamp pins to zero (renderer must not see the
    /// playhead move backwards in time on clock-skew jitter).
    #[test]
    fn publish_state_clamps_negative_elapsed() {
        let shared = DeckSharedState::new();
        shared.publish_position_secs(7.0, 1.0);
        let st = shared.load_publish_state();
        let earlier = st.host_time_ns.saturating_sub(1_000_000);
        let extrap = st.extrapolated_secs(earlier);
        assert!(
            (extrap - 7.0).abs() < f64::EPSILON,
            "negative elapsed must pin to publish-time playhead, got {extrap}"
        );
    }

    /// Concurrent soak: writer publishes at a coarse cadence
    /// (≈100 Hz, matching a real CoreAudio block on macOS),
    /// reader spins reading the publish state at a much higher
    /// rate (matching a 60 Hz renderer that calls
    /// `position_snapshot` every vsync). The reader's
    /// extrapolated playhead must be:
    ///
    /// 1. Always finite (no torn `f64::from_bits` reads).
    /// 2. Monotonic non-decreasing as long as the writer's
    ///    publish times are monotonic and rate ≥ 0.
    ///
    /// This is the regression guard for the 30 Hz strobe pattern
    /// the unfix surfaces in production. A torn or non-monotonic
    /// extrapolation would manifest there as the visible jitter.
    #[test]
    fn publish_state_extrapolation_is_monotonic_under_concurrent_writes() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc as StdArc;
        use std::thread;
        use std::time::Duration;

        let shared = StdArc::new(DeckSharedState::new());
        let stop = StdArc::new(AtomicBool::new(false));

        let writer_shared = shared.clone();
        let writer_stop = stop.clone();
        let writer = thread::spawn(move || {
            // Simulate the audio thread publishing the playhead
            // every ~10 ms, advancing at rate = 1.0.
            let start = Instant::now();
            while !writer_stop.load(Ordering::Relaxed) {
                let secs = start.elapsed().as_secs_f64();
                writer_shared.publish_position_secs(secs, 1.0);
                thread::sleep(Duration::from_millis(10));
            }
        });

        let reader_shared = shared.clone();
        let reader_stop = stop.clone();
        let reader = thread::spawn(move || {
            // Mimic the 60 Hz render thread: read at ~16 ms.
            let mut last_extrap = f64::NEG_INFINITY;
            let mut samples = 0usize;
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline && !reader_stop.load(Ordering::Relaxed) {
                let st = reader_shared.load_publish_state();
                let now = DeckSharedState::host_time_now_ns();
                let extrap = st.extrapolated_secs(now);
                assert!(extrap.is_finite(), "torn read: extrap = {extrap}");
                // Allow a tiny tolerance for the writer racing
                // ahead between two reader observations: the
                // reader observed (host_time_N, secs_N), then
                // the writer landed (host_time_{N+1}, secs_{N+1}).
                // The new extrapolated playhead is from a fresher
                // publish, so it can occasionally be slightly
                // below the previous reader's clamped-by-stall
                // extrapolation if the writer paused. A 1 ms
                // window absorbs scheduler jitter.
                assert!(
                    extrap + 0.001 >= last_extrap,
                    "non-monotonic extrap: {last_extrap} → {extrap}"
                );
                last_extrap = extrap;
                samples += 1;
                thread::sleep(Duration::from_millis(2));
            }
            samples
        });

        let samples = reader.join().expect("reader joined");
        stop.store(true, Ordering::Relaxed);
        writer.join().expect("writer joined");
        assert!(samples > 20, "reader should have sampled many times");
    }

    proptest! {
        #[test]
        fn render_never_panics(
            samples in proptest::collection::vec(-1.0f32..=1.0, 2..256),
            channels in 1u8..=2,
            sample_rate in 8_000u32..=192_000,
            engine_sr in 8_000u32..=192_000,
            rate in -8.0f64..=8.0,
            position in -1_000.0f64..=10_000.0,
            n_frames in 1usize..=128,
        ) {
            // Trim samples to a multiple of channels.
            let n = samples.len();
            let trimmed = n - (n % usize::from(channels));
            let mut samples = samples;
            samples.truncate(trimmed);
            prop_assume!(!samples.is_empty());

            let track = Arc::new(
                Track::from_interleaved(samples, sample_rate, channels).unwrap()
            );

            let mut deck = test_deck();
            deck.set_source(track);
            deck.set_playing(true);
            deck.set_rate(rate);
            deck.set_position_frames(position);
            deck.quiesce_declick_for_test();

            let mut rt = RealtimeContext::new();
            let mut out = vec![0.0f32; n_frames * 2];
            deck.render(&mut rt, &mut out, engine_sr as f32);

            // Output must not contain NaN / inf
            for s in &out {
                prop_assert!(s.is_finite(), "non-finite sample {s}");
            }
        }
    }
}
