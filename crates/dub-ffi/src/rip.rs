//! M26a — vinyl-rip FFI surface.
//!
//! Wraps `dub_rip::RipSession` for the Apple shell: record a side off
//! the deck-0 Thru record tap, review it (live envelope + manual
//! splits + per-segment metadata), then commit (FLAC encode + tag +
//! library import) on a background worker. Everything here follows
//! the house polling model — no callback interfaces; Swift polls
//! [`DubRipSession::status`] / [`DubRipSession::generation`] /
//! [`DubRipSession::job_progress`] the same way it polls the deck
//! peaks and telemetry.
//!
//! The one engine-side subtlety lives in `lib.rs`: the record tap can
//! only be requested when the `ThruSource` is attached, so a rip
//! session must be started with [`DubEngine::start_thru_for_rip`]
//! (which parks the tap consumer on the running state) before
//! [`DubEngine::create_rip_session`] can claim it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dub_rip::{GapConfig, RipConfig, RipError, RipSession, RipState, StopReason, TrackMeta};

use crate::{
    deck_idx_to_usize, lock_state, peak_chunks_to_bytes, usize_from_u64, DubEngine, DubLibrary,
    EngineError, EngineState, PerfSource,
};

/// How long the FFI is willing to block joining the capture worker
/// after a stop request. The worker exits within one poll interval
/// (20 ms in production) plus the final ring drain + WAV finalize,
/// so ten seconds only trips on a genuinely wedged filesystem.
const STOP_JOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors surfaced from the rip FFI. Mirrors [`crate::EngineError`] /
/// [`crate::LibraryFfiError`] — `flat_error` because Swift consumers
/// mostly need a human-readable description; the variant name gives
/// the UI enough to branch on (e.g. `NotCapturing` → "start a rip
/// capture first" hint).
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum RipFfiError {
    /// `create_rip_session` was called without a pending record tap —
    /// either the engine is stopped, or the running Thru session was
    /// started with plain `start_thru` instead of
    /// [`DubEngine::start_thru_for_rip`], or the tap was already
    /// claimed by an earlier session.
    #[error("no record tap pending; start capture with start_thru_for_rip first")]
    NotCapturing,

    /// Bad session configuration (unresolvable destination dir,
    /// non-positive max duration, invalid deck index).
    #[error("invalid rip config: {0}")]
    InvalidConfig(String),

    /// The operation is not valid in the session's current lifecycle
    /// state (e.g. splits while recording, commit before stop,
    /// anything after cancel).
    #[error("invalid rip state: {0}")]
    InvalidState(String),

    /// A split edit was rejected — unknown id, duplicate boundary, or
    /// a boundary that violates the split plan (out of range /
    /// segment shorter than the 5 s minimum).
    #[error("invalid split: {0}")]
    InvalidSplit(String),

    /// A segment index was out of range for the current split plan.
    #[error("invalid segment: {0}")]
    InvalidSegment(String),

    /// The capture worker failed or could not be joined.
    #[error("rip capture failed: {0}")]
    CaptureFailed(String),

    /// Session-directory IO failed (spill, manifest, or cleanup).
    #[error("rip write failed: {0}")]
    WriteFailed(String),

    /// The encode + import worker could not be started.
    #[error("rip import failed: {0}")]
    ImportFailed(String),
}

/// Configuration for [`DubEngine::create_rip_session`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipSessionConfig {
    /// Session directory. `None` (or empty) resolves to
    /// `~/Music/Dub/Rips/<timestamp>` — user-visible data; the
    /// encoded tracks are imported in place.
    pub dest_dir: Option<String>,
    /// Hard recording cap in seconds. Must be > 0. Production
    /// callers pass 2400 (40 min — beyond any vinyl side; a
    /// forgotten needle in the runout groove must not fill the disk).
    pub max_duration_secs: f64,
    /// Start recording on the needle drop instead of waiting for
    /// [`DubRipSession::start`]. The second before the trigger is kept
    /// and written ahead of it, so the drop itself is in the rip
    /// (M26b).
    pub auto_start: bool,
    /// Stop by itself in the run-out groove instead of waiting for
    /// [`DubRipSession::stop`] (M26b). `stop_reason` reads `Silence`
    /// when it fires.
    pub auto_stop: bool,
}

/// Coarse lifecycle phase of a rip session as seen by the UI poll.
///
/// `Idle` → `Armed` → `Recording` → `Stopped` come straight from the
/// capture worker; `Encoding` / `Done` are FFI-level phases driven by
/// the [`DubRipSession::confirm_encode_and_import`] worker. `Failed`
/// covers both a capture failure and a failed commit — the
/// [`RipSessionStatus::error`] message says which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RipPhase {
    /// Constructed (or cancelled); no capture running.
    Idle,
    /// Capture worker draining and discarding — waiting for
    /// [`DubRipSession::start`].
    Armed,
    /// Writing the spill.
    Recording,
    /// Capture finished; splits and metadata may be edited and the
    /// session may be committed.
    Stopped,
    /// Capture or commit failed — see [`RipSessionStatus::error`].
    Failed,
    /// The commit worker is encoding + importing segments.
    Encoding,
    /// The commit worker finished and every segment imported.
    Done,
}

/// Why a recording stopped. `None` until the phase first reaches
/// `Stopped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum RipStopReason {
    /// Not stopped yet.
    #[default]
    None,
    /// The operator pressed stop.
    Manual,
    /// The configured hard duration cap was reached.
    MaxDuration,
    /// The record-tap producer went away (Thru session stopped or the
    /// audio device died). What was captured up to then is intact.
    InputLost,
    /// The side ran out — the input sat far enough under the music for
    /// long enough to be the run-out groove (M26b silence auto-stop).
    Silence,
    /// The session was reopened from disk after an interrupted rip; it
    /// never stopped in this process (M26b recovery).
    Recovered,
}

/// Polled snapshot of a rip session — the rip counterpart of
/// [`crate::PositionInfo`]. Cheap to copy; poll at UI cadence.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipSessionStatus {
    /// Lifecycle phase.
    pub phase: RipPhase,
    /// Why the recording stopped (`None` while it hasn't).
    pub stop_reason: RipStopReason,
    /// Recorded duration in seconds.
    pub elapsed_secs: f64,
    /// Frames written to the spill so far.
    pub recorded_frames: u64,
    /// Absolute peak of the most recent capture window — the level
    /// meter while recording. `0.0` once capture ends.
    pub level_peak: f32,
    /// Failure message when `phase == Failed`.
    pub error: Option<String>,
    /// Where the side starts, in seconds. The lead-in groove before
    /// it is discarded. `0.0` when nothing trimmed the head.
    pub side_start_secs: f64,
    /// Where the side ends, in seconds. The run-out groove after it is
    /// discarded. Equals the recorded length when nothing trimmed the
    /// tail.
    ///
    /// Both bounds are always real numbers rather than optionals, so a
    /// caller drawing the side never has to special-case an untrimmed
    /// rip.
    pub side_end_secs: f64,
}

/// An unfinished rip found on disk, offered back to the operator
/// (M26b). A session's spill is deleted only once every segment has
/// imported, so anything still holding one never finished.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipRecoverable {
    /// Absolute path of the session directory.
    pub session_dir: String,
    /// Recorded length in seconds, measured from the spill itself.
    pub recorded_secs: f64,
    /// True when the capture died mid-recording (the WAV header was
    /// never finalized) rather than being abandoned at review.
    pub was_interrupted: bool,
}

/// One split marker: a stable id (for SwiftUI diffing / drag
/// handles) plus its position in seconds.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipSplit {
    /// Stable id minted by [`DubRipSession::add_split`]. Never
    /// reused within a session.
    pub id: u32,
    /// Marker position in seconds from the start of the recording.
    pub secs: f64,
}

/// A committed rip, offered back for re-splitting (M26b, R-44).
///
/// The complement of [`RipRecoverable`]: commit deletes the spill only
/// once every segment has imported, so a session with no spill but a
/// `side.flac` is one that finished and can be split again.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipResplittable {
    /// Session directory — pass to [`DubEngine::resplit_rip_session`].
    pub session_dir: String,
    /// Directory name, which is the capture timestamp
    /// (`YYYYMMDD-HHMMSS`). The listing is newest-first on it.
    pub name: String,
    /// Length of the archived side in seconds.
    pub recorded_secs: f64,
    /// Tracks the last split produced.
    pub track_count: u32,
    /// How many times this side has been split; 1 is the first commit.
    pub split_generation: u32,
}

/// One derived segment of the recorded side: boundaries plus the
/// metadata that will be tagged into the encoded file.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipSegment {
    /// 0-based segment index (track number is `index + 1`).
    pub index: u32,
    /// Segment start in seconds.
    pub start_secs: f64,
    /// Segment end in seconds (exclusive).
    pub end_secs: f64,
    /// Track title, if set.
    pub title: Option<String>,
    /// Track artist, if set.
    pub artist: Option<String>,
    /// Release / album title, if set.
    pub album: Option<String>,
    /// Genre, if set (also seeds the analysis octave profile).
    pub genre: Option<String>,
    /// Release year, if set.
    pub year: Option<i32>,
}

/// Commit progress for one segment. M26a reports coarse progress —
/// every segment is `Pending` until the whole commit pass finishes,
/// then flips to its final state. Live per-segment progress lands
/// with M26b.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipSegmentJob {
    /// 0-based segment index.
    pub index: u32,
    /// Commit state of this segment.
    pub state: RipSegmentJobState,
    /// Failure or non-fatal analysis note for this segment.
    pub detail: Option<String>,
    /// Canonical library UUID once the segment imported.
    pub imported_track_id: Option<String>,
}

/// Per-segment commit state — see [`RipSegmentJob`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RipSegmentJobState {
    /// Not committed yet.
    Pending,
    /// Currently encoding / tagging / importing (M26b). Exactly one
    /// segment is in this state at a time — the commit pass is serial.
    Running,
    /// Encoded, tagged, and imported.
    Done,
    /// This segment failed; the others were still attempted
    /// (one broken segment must not strand the rest of the side).
    Failed,
}

/// Snapshot of the commit worker, polled via
/// [`DubRipSession::job_progress`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct RipJobProgress {
    /// `true` while the `dub-rip-commit` thread is running.
    pub running: bool,
    /// One entry per segment, in side order. Empty until a commit
    /// has been requested.
    pub per_segment: Vec<RipSegmentJob>,
}

/// FFI-side split-marker table: stable ids over boundary frames.
/// `dub_rip` only knows the sorted frame list; the id layer lives
/// here so SwiftUI list diffing and drag gestures have identity.
struct SplitIds {
    next_id: u32,
    /// `(id, boundary frame)` in insertion order; sorted on read.
    entries: Vec<(u32, u64)>,
}

/// Shared state of the commit worker. `recorded_frames` /
/// `elapsed_secs` / `stop_reason` are frozen at confirm time so
/// [`DubRipSession::status`] can answer during a commit without
/// touching the session mutex (which the worker holds while
/// encoding).
#[derive(Default)]
struct CommitJob {
    running: bool,
    finished: bool,
    complete: bool,
    error: Option<String>,
    per_segment: Vec<RipSegmentJob>,
    recorded_frames: u64,
    elapsed_secs: f64,
    stop_reason: RipStopReason,
}

/// One record-a-side rip session (M26a).
///
/// Created by [`DubEngine::create_rip_session`], already armed on the
/// engine's record tap — call [`Self::start`] to begin writing, then
/// [`Self::stop`], edit splits + metadata, and
/// [`Self::confirm_encode_and_import`]. Poll [`Self::status`] for
/// the lifecycle, [`Self::generation`] to detect plan mutations, and
/// [`Self::envelope_len`] / [`Self::envelope_extend`] for the live
/// waveform (same 12-byte packed wire format as
/// [`DubEngine::peaks_extend`]).
#[derive(uniffi::Object)]
pub struct DubRipSession {
    session: Arc<Mutex<RipSession>>,
    /// Capture sample rate — cached so seconds↔frames mapping never
    /// needs the session mutex.
    sample_rate: u32,
    /// Monotonic counter bumped on every splits / metadata /
    /// commit-progress mutation (the `peaks_generation` idiom): the
    /// Swift side polls it and refreshes its plan model on change.
    /// `Arc` so the commit worker can bump it when it finishes.
    generation: Arc<AtomicU64>,
    /// Whether the capture worker has been joined and the recorded
    /// length synced into the manifest (`RipSession::wait_stopped`).
    synced: AtomicBool,
    /// Set by [`Self::cancel`]; every later mutation refuses.
    cancelled: AtomicBool,
    splits: Mutex<SplitIds>,
    job: Arc<Mutex<CommitJob>>,
}

// Hand-rolled: `RipSession` owns a live worker thread and has no
// useful `Debug` of its own (same approach as `ThruTapHandles` in
// dub-engine). Needed so `Result<Arc<DubRipSession>, _>` is testable
// with `unwrap_err`.
impl std::fmt::Debug for DubRipSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DubRipSession")
            .field("sample_rate", &self.sample_rate)
            .field("generation", &self.generation.load(Ordering::Acquire))
            .field("cancelled", &self.cancelled.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

/// Lock a mutex, recovering from poisoning by taking the inner data —
/// same rationale as `lock_state` in `lib.rs`: panics never cross the
/// FFI boundary, so a poisoned lock is theoretical, but Swift must
/// not see one become a panic.
fn lock_mutex<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn map_rip_error(e: RipError) -> RipFfiError {
    let msg = e.to_string();
    match e {
        RipError::InvalidConfig(_) => RipFfiError::InvalidConfig(msg),
        RipError::InvalidState { .. } => RipFfiError::InvalidState(msg),
        RipError::Split(_) => RipFfiError::InvalidSplit(msg),
        RipError::TrackIndexOutOfRange { .. } => RipFfiError::InvalidSegment(msg),
        RipError::CaptureFailed(_) | RipError::StopTimeout => RipFfiError::CaptureFailed(msg),
        RipError::Io(_) | RipError::Manifest(_) | RipError::SpillUnreadable(_) => {
            RipFfiError::WriteFailed(msg)
        }
    }
}

fn phase_for(state: &RipState) -> RipPhase {
    match state {
        RipState::Idle => RipPhase::Idle,
        RipState::Armed => RipPhase::Armed,
        RipState::Recording => RipPhase::Recording,
        RipState::Stopped(_) => RipPhase::Stopped,
        RipState::Failed => RipPhase::Failed,
    }
}

fn stop_reason_for(state: &RipState) -> RipStopReason {
    match state {
        RipState::Stopped(reason) => map_stop_reason(*reason),
        _ => RipStopReason::None,
    }
}

fn map_stop_reason(reason: StopReason) -> RipStopReason {
    match reason {
        StopReason::Manual => RipStopReason::Manual,
        StopReason::MaxDuration => RipStopReason::MaxDuration,
        StopReason::InputLost => RipStopReason::InputLost,
        StopReason::Silence => RipStopReason::Silence,
        StopReason::Recovered => RipStopReason::Recovered,
    }
}

/// Seconds → frames at the session sample rate. Non-finite and
/// negative inputs clamp to frame 0 (the validator then rejects a
/// zero boundary with a clean error).
fn secs_to_frames(secs: f64, sample_rate: u32) -> u64 {
    if !secs.is_finite() || secs <= 0.0 {
        return 0;
    }
    // Truncation is fine after rounding: a boundary past u64 frames
    // is billions of years of audio.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (secs * f64::from(sample_rate)).round() as u64;
    frames
}

/// Frames → seconds at the session sample rate.
#[allow(clippy::cast_precision_loss)] // exact for any realistic side length
fn frames_to_secs(frames: u64, sample_rate: u32) -> f64 {
    if sample_rate == 0 {
        return 0.0;
    }
    frames as f64 / f64::from(sample_rate)
}

/// `YYYYMMDD-HHMMSS` (UTC) for the default session-dir name.
/// Hand-rolled civil-date conversion (Hinnant's algorithm) instead of
/// pulling a date crate into the FFI surface for one folder name.
fn timestamp_for(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Wrap is unreachable: u64 seconds / 86 400 fits i64 for ~10^14 years.
    #[allow(clippy::cast_possible_wrap)]
    let days = (secs / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    let tod = secs % 86_400;
    let (hh, mm, ss) = (tod / 3_600, (tod % 3_600) / 60, tod % 60);
    format!("{year:04}{month:02}{day:02}-{hh:02}{mm:02}{ss:02}")
}

/// Days since 1970-01-01 → civil `(year, month, day)`.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // month ∈ [1,12], day ∈ [1,31] by construction.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (year, month as u32, day as u32)
}

/// Resolve the default rip destination:
/// `~/Music/Dub/Rips/<timestamp>`, de-duplicated with a `-N` suffix
/// if two sessions land in the same second.
/// The directory rips live in, without minting a new session.
fn rips_root() -> Result<PathBuf, RipFfiError> {
    let music = dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .ok_or_else(|| {
            RipFfiError::InvalidConfig("cannot resolve the user's Music directory".into())
        })?;
    Ok(music.join("Dub").join("Rips"))
}

fn default_session_dir() -> Result<PathBuf, RipFfiError> {
    let music = dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .ok_or_else(|| {
            RipFfiError::InvalidConfig("cannot resolve the user's Music directory".into())
        })?;
    let base = music.join("Dub").join("Rips");
    let stamp = timestamp_for(SystemTime::now());
    let mut dir = base.join(&stamp);
    let mut n = 2_u32;
    while dir.exists() {
        dir = base.join(format!("{stamp}-{n}"));
        n = n.saturating_add(1);
    }
    Ok(dir)
}

/// Flip one segment's dot, ignoring an index the plan no longer has.
fn set_segment_state(job: &mut CommitJob, index: usize, state: RipSegmentJobState) {
    if let Some(entry) = job.per_segment.get_mut(index) {
        entry.state = state;
    }
}

/// Body of the `dub-rip-commit` worker thread. Locks the library for
/// the duration of the commit (the same single-connection model every
/// other library write uses), fills the job state from the
/// [`dub_rip::RipOutcome`], and bumps the generation so pollers
/// refresh.
fn run_commit_job(
    session: &Mutex<RipSession>,
    library: &DubLibrary,
    job: &Mutex<CommitJob>,
    generation: &AtomicU64,
) {
    let result = {
        let mut lib_guard = lock_mutex(&library.inner);
        match lib_guard.as_mut() {
            None => Err("library not open".to_string()),
            Some(lib) => {
                // Report each segment as it happens (M26b / R-39).
                // Locking `job` briefly here is safe: `status` scopes
                // its job guard and drops it before ever reaching for
                // the session, so there is no path that holds job and
                // then waits on session.
                let mut on_progress = |event: dub_rip::CommitProgress| {
                    {
                        let mut job = lock_mutex(job);
                        match event {
                            dub_rip::CommitProgress::Started { index } => {
                                set_segment_state(&mut job, index, RipSegmentJobState::Running);
                            }
                            dub_rip::CommitProgress::Finished { index, imported } => {
                                let state = if imported {
                                    RipSegmentJobState::Done
                                } else {
                                    RipSegmentJobState::Failed
                                };
                                set_segment_state(&mut job, index, state);
                            }
                            dub_rip::CommitProgress::ArchiveStarted => {}
                        }
                    }
                    // Poll-driven UI: the dots only move when the
                    // generation does.
                    generation.fetch_add(1, Ordering::Release);
                };
                lock_mutex(session)
                    .commit_with_progress(lib, &mut on_progress)
                    .map_err(|e| e.to_string())
            }
        }
    };

    let mut job = lock_mutex(job);
    job.running = false;
    job.finished = true;
    match result {
        Ok(outcome) => {
            job.per_segment = outcome
                .segments
                .iter()
                .map(|s| RipSegmentJob {
                    index: u32::try_from(s.index).unwrap_or(u32::MAX),
                    state: if s.library_uuid.is_some() {
                        RipSegmentJobState::Done
                    } else {
                        RipSegmentJobState::Failed
                    },
                    detail: s.error.clone(),
                    imported_track_id: s.library_uuid.clone(),
                })
                .collect();
            job.complete = outcome.is_complete();
            if !job.complete {
                let failed: Vec<String> = outcome
                    .segments
                    .iter()
                    .filter(|s| s.library_uuid.is_none())
                    .map(|s| format!("segment {}", s.index + 1))
                    .collect();
                job.error = Some(if failed.is_empty() {
                    "side archive not written".to_string()
                } else {
                    format!("{} failed to import", failed.join(", "))
                });
            }
        }
        Err(e) => {
            for seg in &mut job.per_segment {
                if seg.state == RipSegmentJobState::Pending {
                    seg.state = RipSegmentJobState::Failed;
                }
            }
            job.error = Some(e);
        }
    }
    drop(job);
    generation.fetch_add(1, Ordering::Release);
}

/// One raw (pre-classifier) HAL input device. Unlike
/// `AudioDeviceInfo` this list is NOT filtered to DJ-grade gear —
/// it includes the built-in microphone, Continuity mics, virtual
/// devices, everything the OS reports. It exists so DEBUG builds
/// can dogfood the vinyl-rip flow against the built-in soundcard
/// when the Developer mode override is engaged; production device
/// pickers must keep using `list_audio_devices`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RawInputDevice {
    /// Human-readable device name, resolvable by
    /// `DubEngine::start_thru_for_rip` (substring match).
    pub name: String,
    /// Channel count on the input bus. 1 for the built-in mic —
    /// the rip fallback records mono duplicated to both slots.
    pub channels: u32,
    /// Whether this is the current system-default input device.
    pub is_default: bool,
}

#[uniffi::export]
impl DubEngine {
    /// Enumerate every raw HAL input device — see [`RawInputDevice`]
    /// for why this deliberately bypasses the device classifier.
    /// Off-RT; returns an empty list if CoreAudio enumeration fails
    /// (same forgiving contract as `list_input_devices`).
    #[must_use]
    pub fn list_raw_input_devices(&self) -> Vec<RawInputDevice> {
        let default_name = dub_audio::query_default_input().ok().map(|d| d.name);
        dub_audio::list_input_devices()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|d| RawInputDevice {
                        is_default: Some(&d.name) == default_name.as_ref(),
                        name: d.name,
                        channels: d.channels,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// [`Self::start_thru`] flavour that additionally wires the M26
    /// full-quality stereo record tap on deck 0, so a
    /// [`DubRipSession`] can be created afterwards via
    /// [`Self::create_rip_session`].
    ///
    /// The tap must be requested here, at Thru-attach time: the input
    /// device consumer is take-once, and re-attaching a `ThruSource`
    /// later would displace the one that is already playing. Until a
    /// session claims the tap its ring simply overruns (the audio
    /// thread drops tap samples, never blocks) — the deck's audio
    /// path is unaffected.
    ///
    /// Parameters and audio behaviour are otherwise identical to
    /// [`Self::start_thru`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::start_thru`].
    pub fn start_thru_for_rip(
        &self,
        device_name: String,
        channels: Vec<u32>,
        output_device_uid: Option<String>,
    ) -> Result<(), EngineError> {
        let mut state = lock_state(&self.state);
        if matches!(*state, EngineState::Running(_)) {
            return Err(EngineError::AlreadyRunning);
        }

        if channels.len() != 2 || channels.contains(&0) {
            return Err(EngineError::InvalidChannels(channels));
        }

        let running = crate::start_thru_inner(
            &device_name,
            &channels,
            None,
            output_device_uid.as_deref(),
            &self.deck_shared,
            PerfSource::Thru,
            true,
        )?;
        *state = EngineState::Running(Box::new(running));
        self.bump_peak_generation(0);
        Ok(())
    }

    /// Unfinished rips sitting in the rips directory (M26b).
    ///
    /// Offered on entering Prep so a crashed or force-quit session can
    /// be finished rather than silently abandoned: the spill survives
    /// by design, and commit only deletes it once every segment has
    /// imported, so anything still holding one has work left. Cheap —
    /// it reads WAV headers, not audio.
    ///
    /// `rips_dir` overrides the default `~/Music/Dub/Rips` (tests).
    #[must_use]
    pub fn list_recoverable_rip_sessions(&self, rips_dir: Option<String>) -> Vec<RipRecoverable> {
        let root = match rips_dir {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => match rips_root() {
                Ok(dir) => dir,
                Err(_) => return Vec::new(),
            },
        };
        dub_rip::list_recoverable(&root)
            .into_iter()
            .map(|found| RipRecoverable {
                session_dir: found.session_dir.to_string_lossy().into_owned(),
                recorded_secs: found.secs(),
                was_interrupted: found.was_interrupted,
            })
            .collect()
    }

    /// Committed rips sitting in the rips directory, newest first
    /// (M26b, R-44).
    ///
    /// This is what a "past rips" surface lists, and the only way the
    /// app can produce a `session_dir` for
    /// [`Self::resplit_rip_session`]. Cheap — it reads each `rip.json`
    /// and stats the archive, and deliberately does *not* decode the
    /// FLACs to measure them.
    ///
    /// `rips_dir` overrides the default `~/Music/Dub/Rips` (tests).
    #[must_use]
    pub fn list_resplittable_rip_sessions(&self, rips_dir: Option<String>) -> Vec<RipResplittable> {
        let root = match rips_dir {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => match rips_root() {
                Ok(dir) => dir,
                Err(_) => return Vec::new(),
            },
        };
        dub_rip::list_resplittable(&root)
            .into_iter()
            .map(|found| RipResplittable {
                name: found
                    .session_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                session_dir: found.session_dir.to_string_lossy().into_owned(),
                recorded_secs: frames_to_secs(found.frames, found.sample_rate),
                track_count: found.track_count,
                split_generation: found.split_generation,
            })
            .collect()
    }

    /// Reopen one of [`Self::list_recoverable_rip_sessions`].
    ///
    /// The session comes back at the review stage — stopped, with its
    /// plan, its metadata, and an envelope rebuilt from the spill — and
    /// needs no engine, no record tap, and no Thru session, because
    /// nothing more will be recorded into it.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::WriteFailed`] if the manifest is missing or
    /// malformed; [`RipFfiError::CaptureFailed`] if the spill cannot be
    /// read back.
    pub fn resume_rip_session(
        &self,
        session_dir: String,
    ) -> Result<Arc<DubRipSession>, RipFfiError> {
        let session =
            RipSession::from_session_dir(PathBuf::from(session_dir)).map_err(map_rip_error)?;
        let sample_rate = session.manifest().sample_rate;
        let boundaries = session.manifest().boundaries_frames.clone();
        Ok(Arc::new(DubRipSession::resumed(
            session,
            sample_rate,
            &boundaries,
        )))
    }

    /// Reopen a committed rip to split it again from its lossless
    /// archive (M26b).
    ///
    /// Replace semantics: the new segments import first, and the
    /// tracks from the earlier split are removed from the library only
    /// once that succeeds — a failure part-way leaves the originals in
    /// place rather than leaving the DJ with neither. The archive
    /// survives, so a side can be split again as often as needed.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::WriteFailed`] if the manifest is missing or
    /// malformed; [`RipFfiError::CaptureFailed`] if `side.flac` cannot
    /// be decoded.
    pub fn resplit_rip_session(
        &self,
        session_dir: String,
    ) -> Result<Arc<DubRipSession>, RipFfiError> {
        let session =
            RipSession::resplit_from_archive(PathBuf::from(session_dir)).map_err(map_rip_error)?;
        let sample_rate = session.manifest().sample_rate;
        Ok(Arc::new(DubRipSession::resumed(session, sample_rate, &[])))
    }

    /// Create a rip session on `deck_idx`, claiming the record tap
    /// that [`Self::start_thru_for_rip`] parked. The session is
    /// created *armed* — its capture worker is already draining the
    /// tap (and discarding) — so recording starts at "now" the moment
    /// [`DubRipSession::start`] is called.
    ///
    /// The tap is claimed exactly once: a second call for the same
    /// Thru session returns [`RipFfiError::NotCapturing`].
    ///
    /// # Errors
    ///
    /// * [`RipFfiError::NotCapturing`] — engine stopped, Thru started
    ///   without the rip flavour, or the tap was already claimed.
    /// * [`RipFfiError::InvalidConfig`] — bad deck index, bad
    ///   `max_duration_secs`, or an unresolvable destination dir.
    /// * [`RipFfiError::WriteFailed`] — the session directory or the
    ///   initial manifest could not be created.
    pub fn create_rip_session(
        &self,
        deck_idx: u64,
        config: RipSessionConfig,
    ) -> Result<Arc<DubRipSession>, RipFfiError> {
        let idx = deck_idx_to_usize(deck_idx)
            .map_err(|_| RipFfiError::InvalidConfig(format!("invalid deck index {deck_idx}")))?;
        if !config.max_duration_secs.is_finite() || config.max_duration_secs <= 0.0 {
            return Err(RipFfiError::InvalidConfig(
                "max_duration_secs must be > 0".into(),
            ));
        }

        let mut state = lock_state(&self.state);
        let EngineState::Running(running) = &mut *state else {
            return Err(RipFfiError::NotCapturing);
        };
        if !matches!(running.record_taps.get(idx), Some(Some(_))) {
            return Err(RipFfiError::NotCapturing);
        }

        let session_dir = match config.dest_dir.as_deref() {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => default_session_dir()?,
        };
        let mut cfg = RipConfig::new(running.sample_rate, session_dir);
        // f64 → f32 loses nothing at duration magnitudes (seconds).
        #[allow(clippy::cast_possible_truncation)]
        {
            cfg.max_duration_secs = config.max_duration_secs as f32;
        }
        {
            let defaults = dub_rip::AutoCapture::default();
            cfg.auto = dub_rip::AutoCapture {
                start_threshold: config
                    .auto_start
                    .then_some(defaults.start_threshold.unwrap_or(0.01)),
                pre_roll_secs: defaults.pre_roll_secs,
                silence_stop_secs: config
                    .auto_stop
                    .then_some(defaults.silence_stop_secs.unwrap_or(20.0)),
                silence_drop_db: defaults.silence_drop_db,
            };
        }
        let mut session = RipSession::new(cfg).map_err(map_rip_error)?;

        let Some(record_rx) = running.record_taps.get_mut(idx).and_then(Option::take) else {
            // Unreachable: presence was checked above under the same
            // state lock. Kept as a defensive error, never a panic.
            return Err(RipFfiError::NotCapturing);
        };
        session.arm(record_rx).map_err(map_rip_error)?;

        Ok(Arc::new(DubRipSession::from_parts(
            session,
            running.sample_rate,
        )))
    }
}

impl DubRipSession {
    /// Internal constructor shared by [`DubEngine::create_rip_session`]
    /// and the unit tests (which arm against a synthetic ring instead
    /// of live audio hardware).
    pub(crate) fn from_parts(session: RipSession, sample_rate: u32) -> Self {
        Self {
            session: Arc::new(Mutex::new(session)),
            sample_rate,
            generation: Arc::new(AtomicU64::new(0)),
            synced: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            splits: Mutex::new(SplitIds {
                next_id: 1,
                entries: Vec::new(),
            }),
            job: Arc::new(Mutex::new(CommitJob::default())),
        }
    }

    /// A session reopened from disk: already stopped and synced (there
    /// is no worker to join), with stable ids minted for the split
    /// plan the manifest carried.
    pub(crate) fn resumed(session: RipSession, sample_rate: u32, boundaries: &[u64]) -> Self {
        let out = Self::from_parts(session, sample_rate);
        out.synced.store(true, Ordering::Release);
        {
            let mut splits = lock_mutex(&out.splits);
            for frame in boundaries {
                let id = splits.next_id;
                splits.next_id = splits.next_id.wrapping_add(1);
                splits.entries.push((id, *frame));
            }
        }
        out
    }

    fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn guard_not_cancelled(&self) -> Result<(), RipFfiError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(RipFfiError::InvalidState("session was cancelled".into()));
        }
        Ok(())
    }

    /// Gate for plan mutations (splits / metadata): refused after
    /// cancel, while a commit runs, and once a commit fully
    /// succeeded (a failed commit stays editable so the plan can be
    /// fixed and retried).
    fn guard_mutable(&self) -> Result<(), RipFfiError> {
        self.guard_not_cancelled()?;
        let job = lock_mutex(&self.job);
        if job.running {
            return Err(RipFfiError::InvalidState("a commit is running".into()));
        }
        if job.finished && job.complete {
            return Err(RipFfiError::InvalidState(
                "the session is already committed".into(),
            ));
        }
        Ok(())
    }

    /// If the capture has reached a terminal state, join the worker
    /// and sync the recorded length into the manifest. Non-blocking
    /// while the capture is still live (this is what the 30 Hz
    /// status poll rides on); a short bounded join once terminal.
    fn sync_if_terminal(&self) {
        if self.synced.load(Ordering::Acquire) {
            return;
        }
        let mut session = lock_mutex(&self.session);
        let state = session.status().state;
        if matches!(state, RipState::Stopped(_) | RipState::Failed) {
            // Failure is surfaced by status() from the session state;
            // here we only care that the worker is joined.
            let _ = session.wait_stopped(STOP_JOIN_TIMEOUT);
            self.synced.store(true, Ordering::Release);
        }
    }

    /// Blocking flavour of [`Self::sync_if_terminal`] used by `stop`:
    /// waits for the worker to land, surfaces capture failures.
    fn sync_stopped(&self) -> Result<(), RipFfiError> {
        if self.synced.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut session = lock_mutex(&self.session);
        match session.wait_stopped(STOP_JOIN_TIMEOUT) {
            Ok(_) => {
                self.synced.store(true, Ordering::Release);
                Ok(())
            }
            Err(e @ RipError::CaptureFailed(_)) => {
                // The worker is joined; the session is terminal.
                self.synced.store(true, Ordering::Release);
                Err(map_rip_error(e))
            }
            Err(e) => Err(map_rip_error(e)),
        }
    }

    /// Push the current id table (with `candidate` boundary frames,
    /// already sorted) into the session's split plan. The caller
    /// mutates its id table only after this validates.
    fn apply_splits(&self, candidate: &[u64]) -> Result<(), RipFfiError> {
        lock_mutex(&self.session)
            .set_splits(candidate.to_vec())
            .map_err(map_rip_error)
    }

    fn set_side_bounds(&self, start_secs: f64, end_secs: f64) -> Result<(), RipFfiError> {
        let start = secs_to_frames(start_secs.max(0.0), self.sample_rate);
        let end = secs_to_frames(end_secs.max(0.0), self.sample_rate);
        lock_mutex(&self.session)
            .set_side_bounds(start, end)
            .map_err(map_rip_error)?;
        self.bump_generation();
        Ok(())
    }

    /// The side's bounds in seconds, read off the manifest.
    fn side_bounds(&self) -> (f64, f64) {
        let session = lock_mutex(&self.session);
        let manifest = session.manifest();
        (
            frames_to_secs(manifest.side_start(), self.sample_rate),
            frames_to_secs(manifest.side_end(), self.sample_rate),
        )
    }
}

#[uniffi::export]
impl DubRipSession {
    /// Begin writing the spill. Arming already happened at
    /// [`DubEngine::create_rip_session`], so this is the "needle
    /// down, hit record" moment — capture starts at "now", not at
    /// session creation.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidState`] unless the session is `Armed`
    /// (or already `Recording`, where it is a no-op).
    pub fn start(&self) -> Result<(), RipFfiError> {
        self.guard_not_cancelled()?;
        lock_mutex(&self.session).start().map_err(map_rip_error)
    }

    /// Stop the recording, wait for the capture worker to finalize
    /// the spill, and sync the recorded length. Idempotent: calling
    /// it after an auto-stop (duration cap / input lost) just joins
    /// the worker.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::CaptureFailed`] if the worker failed or cannot
    /// be joined.
    pub fn stop(&self) -> Result<(), RipFfiError> {
        self.guard_not_cancelled()?;
        {
            let session = lock_mutex(&self.session);
            if matches!(
                session.status().state,
                RipState::Armed | RipState::Recording
            ) {
                session.stop().map_err(map_rip_error)?;
            }
        }
        self.sync_stopped()?;
        self.bump_generation();
        Ok(())
    }

    /// Abandon the session: stop the capture if it is live and delete
    /// the session directory (spill, manifest, anything encoded so
    /// far). Idempotent. The object stays alive but refuses every
    /// further mutation.
    ///
    /// # Errors
    ///
    /// * [`RipFfiError::InvalidState`] while a commit is running.
    /// * [`RipFfiError::WriteFailed`] if the directory removal fails.
    pub fn cancel(&self) -> Result<(), RipFfiError> {
        {
            let job = lock_mutex(&self.job);
            if job.running {
                return Err(RipFfiError::InvalidState(
                    "cannot cancel while a commit is running".into(),
                ));
            }
        }
        {
            // Imported segments live inside the session dir and the
            // library references them by path — deleting the dir
            // would orphan those tracks. Retry the failed segments
            // instead (commit is idempotent).
            let session = lock_mutex(&self.session);
            if session
                .manifest()
                .tracks
                .iter()
                .any(|t| t.library_uuid.is_some())
            {
                return Err(RipFfiError::InvalidState(
                    "session already has tracks imported into the library; \
                     retry the failed segments instead of discarding"
                        .into(),
                ));
            }
        }
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let dir = {
            let mut session = lock_mutex(&self.session);
            match session.status().state {
                RipState::Armed | RipState::Recording => {
                    let _ = session.stop();
                    let _ = session.wait_stopped(STOP_JOIN_TIMEOUT);
                }
                RipState::Stopped(_) | RipState::Failed => {
                    if !self.synced.load(Ordering::Acquire) {
                        let _ = session.wait_stopped(STOP_JOIN_TIMEOUT);
                    }
                }
                RipState::Idle => {}
            }
            session.session_dir().to_path_buf()
        };
        self.synced.store(true, Ordering::Release);
        std::fs::remove_dir_all(&dir)
            .map_err(|e| RipFfiError::WriteFailed(format!("removing {}: {e}", dir.display())))?;
        self.bump_generation();
        Ok(())
    }

    /// Polled lifecycle snapshot. While the commit worker runs (or
    /// after it finished) the capture stats are the values frozen at
    /// confirm time — the recording is over by then, so nothing is
    /// lost — and the phase reports `Encoding` / `Done` / `Failed`.
    /// A cancelled session reports `Idle`.
    #[must_use]
    pub fn status(&self) -> RipSessionStatus {
        if self.cancelled.load(Ordering::Acquire) {
            return RipSessionStatus {
                phase: RipPhase::Idle,
                stop_reason: RipStopReason::None,
                elapsed_secs: 0.0,
                recorded_frames: 0,
                level_peak: 0.0,
                error: None,
                side_start_secs: 0.0,
                side_end_secs: 0.0,
            };
        }
        // Snapshot the job and release its lock before taking the
        // session's — the trim lives in the manifest, and holding both
        // would be the only place in this file that nests them.
        let commit = {
            let job = lock_mutex(&self.job);
            (job.running || job.finished).then(|| {
                let phase = if job.running {
                    RipPhase::Encoding
                } else if job.complete && job.error.is_none() {
                    RipPhase::Done
                } else {
                    RipPhase::Failed
                };
                (
                    phase,
                    job.stop_reason,
                    job.elapsed_secs,
                    job.recorded_frames,
                    job.error.clone(),
                )
            })
        };
        if let Some((phase, stop_reason, elapsed_secs, recorded_frames, error)) = commit {
            let (side_start_secs, side_end_secs) = self.side_bounds();
            return RipSessionStatus {
                phase,
                stop_reason,
                elapsed_secs,
                recorded_frames,
                level_peak: 0.0,
                error,
                side_start_secs,
                side_end_secs,
            };
        }
        self.sync_if_terminal();
        let (s, side_start_secs, side_end_secs) = {
            let session = lock_mutex(&self.session);
            let manifest = session.manifest();
            (
                session.status(),
                frames_to_secs(manifest.side_start(), self.sample_rate),
                frames_to_secs(manifest.side_end(), self.sample_rate),
            )
        };
        RipSessionStatus {
            phase: phase_for(&s.state),
            stop_reason: stop_reason_for(&s.state),
            elapsed_secs: s.elapsed_secs,
            recorded_frames: s.recorded_frames,
            level_peak: s.window_peak,
            error: s.failure,
            side_start_secs,
            side_end_secs,
        }
    }

    /// Monotonic mutation counter (splits / metadata / commit
    /// progress). Poll it and re-fetch [`Self::split_markers`] /
    /// [`Self::segments`] / [`Self::job_progress`] on change — the
    /// same idiom as [`DubEngine::peaks_generation`].
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Number of envelope chunks accumulated so far. Grows while
    /// recording; fixed once stopped.
    #[must_use]
    pub fn envelope_len(&self) -> u64 {
        lock_mutex(&self.session).envelope_len() as u64
    }

    /// Fetch envelope chunks from `start_idx` on, packed as 12-byte
    /// little-endian `(f32 min, f32 max, f32 rms)` triples — the
    /// exact wire format of [`DubEngine::peaks_extend`], so the Swift
    /// renderer reuses its existing decoder. Chunk `i` covers WAV
    /// frames `i × 64 ..`. Returns empty when `start_idx` is past the
    /// end.
    #[must_use]
    pub fn envelope_extend(&self, start_idx: u64) -> Vec<u8> {
        let chunks = lock_mutex(&self.session).envelope_from(usize_from_u64(start_idx));
        peak_chunks_to_bytes(&chunks)
    }

    /// The split markers, sorted by position. Ids are stable across
    /// edits (never reused), so SwiftUI diffing and drag handles can
    /// key on them.
    #[must_use]
    pub fn split_markers(&self) -> Vec<RipSplit> {
        let splits = lock_mutex(&self.splits);
        let mut entries = splits.entries.clone();
        entries.sort_by_key(|&(_, frame)| frame);
        entries
            .into_iter()
            .map(|(id, frame)| RipSplit {
                id,
                secs: frames_to_secs(frame, self.sample_rate),
            })
            .collect()
    }

    /// Replace the plan with splits detected from the capture
    /// envelope and return how many were installed (segments =
    /// splits + 1). Existing markers are discarded; the proposals are
    /// ordinary markers afterwards, so the operator drags or removes
    /// them like any other. A side with no detectable gaps stays one
    /// segment and returns 0 — not an error.
    ///
    /// Also trims the side: the detector reports where the music
    /// begins and ends, and the lead-in and run-out grooves come off
    /// the first and last tracks.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidState`] while recording / after commit;
    /// [`RipFfiError::InvalidSplit`] if the detected plan somehow
    /// fails validation (it cannot, by construction — the detector
    /// enforces a far larger minimum track length).
    pub fn auto_split(&self) -> Result<u32, RipFfiError> {
        self.guard_mutable()?;
        self.sync_if_terminal();
        let mut splits = lock_mutex(&self.splits);
        // Delegate to `RipSession::auto_split` rather than running
        // `detect_gaps` + `set_splits` here. It is the only writer of
        // the side trim, and an earlier version of this method
        // reimplemented its body — which silently dropped the trim on
        // every rip the *app* drove, while the CLI (which calls the
        // real thing) was fine.
        let boundaries: Vec<u64> = {
            let mut session = lock_mutex(&self.session);
            session
                .auto_split(&GapConfig::default())
                .map_err(map_rip_error)?;
            session.manifest().boundaries_frames.clone()
        };
        splits.entries.clear();
        for frame in &boundaries {
            let id = splits.next_id;
            splits.next_id = splits.next_id.wrapping_add(1);
            splits.entries.push((id, *frame));
        }
        drop(splits);
        self.bump_generation();
        Ok(u32::try_from(boundaries.len()).unwrap_or(u32::MAX))
    }

    /// Move where the side starts — the end of the lead-in groove.
    /// Everything before it is discarded at commit.
    ///
    /// Pass `0.0` to keep the whole head. Nothing here is
    /// irreversible: `side.flac` archives the entire capture and a
    /// re-split reaches back past any trim.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidState`] while recording / after commit;
    /// [`RipFfiError::InvalidSplit`] when the trim would swallow a
    /// split marker or leave a segment under the minimum.
    pub fn set_side_start(&self, secs: f64) -> Result<(), RipFfiError> {
        self.guard_mutable()?;
        self.sync_if_terminal();
        let (_, end) = self.side_bounds();
        self.set_side_bounds(secs, end)
    }

    /// Move where the side ends — the start of the run-out groove.
    /// Everything after it is discarded at commit. Pass the recorded
    /// length to keep the whole tail.
    ///
    /// # Errors
    ///
    /// As [`Self::set_side_start`].
    pub fn set_side_end(&self, secs: f64) -> Result<(), RipFfiError> {
        self.guard_mutable()?;
        self.sync_if_terminal();
        let (start, _) = self.side_bounds();
        self.set_side_bounds(start, secs)
    }

    /// Add a split marker at `secs` and return its stable id. Only
    /// valid once stopped. The whole plan re-validates on every edit
    /// (boundaries strictly increasing, every segment ≥ 5 s); a
    /// rejected edit leaves the plan untouched.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidSplit`] for an invalid boundary,
    /// [`RipFfiError::InvalidState`] while recording / after commit.
    pub fn add_split(&self, secs: f64) -> Result<u32, RipFfiError> {
        self.guard_mutable()?;
        self.sync_if_terminal();
        let frame = secs_to_frames(secs, self.sample_rate);
        let mut splits = lock_mutex(&self.splits);
        if splits.entries.iter().any(|&(_, f)| f == frame) {
            return Err(RipFfiError::InvalidSplit(format!(
                "a split already exists at {secs:.2} s"
            )));
        }
        let mut candidate: Vec<u64> = splits.entries.iter().map(|&(_, f)| f).collect();
        candidate.push(frame);
        candidate.sort_unstable();
        self.apply_splits(&candidate)?;
        let id = splits.next_id;
        splits.next_id = splits.next_id.wrapping_add(1);
        splits.entries.push((id, frame));
        drop(splits);
        self.bump_generation();
        Ok(id)
    }

    /// Move split `id` to `secs`. Same validation semantics as
    /// [`Self::add_split`].
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidSplit`] for an unknown id or an invalid
    /// target position; [`RipFfiError::InvalidState`] while recording
    /// / after commit.
    pub fn move_split(&self, id: u32, secs: f64) -> Result<(), RipFfiError> {
        self.guard_mutable()?;
        self.sync_if_terminal();
        let frame = secs_to_frames(secs, self.sample_rate);
        let mut splits = lock_mutex(&self.splits);
        let Some(pos) = splits.entries.iter().position(|&(sid, _)| sid == id) else {
            return Err(RipFfiError::InvalidSplit(format!("unknown split id {id}")));
        };
        if splits
            .entries
            .iter()
            .any(|&(sid, f)| sid != id && f == frame)
        {
            return Err(RipFfiError::InvalidSplit(format!(
                "a split already exists at {secs:.2} s"
            )));
        }
        let mut candidate: Vec<u64> = splits
            .entries
            .iter()
            .map(|&(sid, f)| if sid == id { frame } else { f })
            .collect();
        candidate.sort_unstable();
        self.apply_splits(&candidate)?;
        splits.entries[pos].1 = frame;
        drop(splits);
        self.bump_generation();
        Ok(())
    }

    /// Remove split `id`, merging its two neighbouring segments.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidSplit`] for an unknown id;
    /// [`RipFfiError::InvalidState`] while recording / after commit.
    pub fn remove_split(&self, id: u32) -> Result<(), RipFfiError> {
        self.guard_mutable()?;
        self.sync_if_terminal();
        let mut splits = lock_mutex(&self.splits);
        let Some(pos) = splits.entries.iter().position(|&(sid, _)| sid == id) else {
            return Err(RipFfiError::InvalidSplit(format!("unknown split id {id}")));
        };
        let mut candidate: Vec<u64> = splits
            .entries
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != pos)
            .map(|(_, &(_, f))| f)
            .collect();
        candidate.sort_unstable();
        self.apply_splits(&candidate)?;
        splits.entries.remove(pos);
        drop(splits);
        self.bump_generation();
        Ok(())
    }

    /// The derived segments: split boundaries + recorded length +
    /// per-segment metadata from the manifest. Empty until the
    /// recording stopped (segment boundaries need the final length).
    #[must_use]
    pub fn segments(&self) -> Vec<RipSegment> {
        self.sync_if_terminal();
        let session = lock_mutex(&self.session);
        let manifest = session.manifest();
        if manifest.recorded_frames == 0 {
            return Vec::new();
        }
        let ranges = dub_rip::segments(
            &manifest.boundaries_frames,
            manifest.side_start(),
            manifest.side_end(),
        );
        ranges
            .iter()
            .enumerate()
            .map(|(i, range)| {
                let meta = manifest
                    .tracks
                    .get(i)
                    .map(|t| t.meta.clone())
                    .unwrap_or_default();
                RipSegment {
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                    start_secs: frames_to_secs(range.start, self.sample_rate),
                    end_secs: frames_to_secs(range.end, self.sample_rate),
                    title: meta.title,
                    artist: meta.artist,
                    album: meta.album,
                    genre: meta.genre,
                    year: meta.year,
                }
            })
            .collect()
    }

    /// Set one segment's metadata (tagged into the encoded FLAC and
    /// carried into the library import). All fields optional — an
    /// untagged rip commits fine. Only valid once stopped.
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidSegment`] for an out-of-range index;
    /// [`RipFfiError::InvalidState`] while recording / after commit.
    pub fn set_segment_metadata(
        &self,
        index: u32,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        genre: Option<String>,
        year: Option<i32>,
    ) -> Result<(), RipFfiError> {
        self.guard_mutable()?;
        self.sync_if_terminal();
        let meta = TrackMeta {
            title,
            artist,
            album,
            year,
            genre,
        };
        {
            let mut session = lock_mutex(&self.session);
            // An unsplit side has no track entries until the first
            // set_splits; materialise the single whole-side segment
            // so its metadata is editable too.
            if session.manifest().tracks.is_empty() {
                session.set_splits(Vec::new()).map_err(map_rip_error)?;
            }
            session
                .set_track_meta(usize_from_u64(u64::from(index)), meta)
                .map_err(map_rip_error)?;
        }
        self.bump_generation();
        Ok(())
    }

    /// Encode + tag + import every segment on a background worker
    /// (`dub-rip-commit`): FLAC-encode each segment, write the
    /// lossless side archive, import into `library` pre-analyzed, and
    /// delete the spill once everything succeeded.
    ///
    /// Returns immediately; [`Self::status`] reports `Encoding` while
    /// the worker runs and `Done` / `Failed` after, and
    /// [`Self::job_progress`] carries the per-segment outcome. M26a
    /// progress is coarse (per-segment states land when the whole
    /// pass finishes); live per-segment progress lands with M26b. A
    /// failed commit may be retried by calling this again — already
    /// imported segments are skipped (idempotent).
    ///
    /// # Errors
    ///
    /// [`RipFfiError::InvalidState`] unless the capture is stopped
    /// and no commit is currently running;
    /// [`RipFfiError::ImportFailed`] if the worker thread cannot be
    /// spawned.
    pub fn confirm_encode_and_import(&self, library: Arc<DubLibrary>) -> Result<(), RipFfiError> {
        self.guard_not_cancelled()?;
        self.sync_if_terminal();

        let (recorded_frames, elapsed_secs, stop_reason, segment_count) = {
            let session = lock_mutex(&self.session);
            let status = session.status();
            let RipState::Stopped(reason) = status.state else {
                return Err(RipFfiError::InvalidState(format!(
                    "cannot commit while capture is {:?}",
                    status.state
                )));
            };
            let count = session.manifest().boundaries_frames.len() + 1;
            (
                status.recorded_frames,
                status.elapsed_secs,
                map_stop_reason(reason),
                count,
            )
        };

        {
            let mut job = lock_mutex(&self.job);
            if job.running {
                return Err(RipFfiError::InvalidState(
                    "a commit is already running".into(),
                ));
            }
            job.running = true;
            job.finished = false;
            job.complete = false;
            job.error = None;
            job.recorded_frames = recorded_frames;
            job.elapsed_secs = elapsed_secs;
            job.stop_reason = stop_reason;
            job.per_segment = (0..segment_count)
                .map(|i| RipSegmentJob {
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                    state: RipSegmentJobState::Pending,
                    detail: None,
                    imported_track_id: None,
                })
                .collect();
        }

        let session = Arc::clone(&self.session);
        let job = Arc::clone(&self.job);
        let generation = Arc::clone(&self.generation);
        let spawned = std::thread::Builder::new()
            .name("dub-rip-commit".into())
            .spawn(move || run_commit_job(&session, &library, &job, &generation));
        if let Err(e) = spawned {
            let mut job = lock_mutex(&self.job);
            job.running = false;
            job.finished = false;
            return Err(RipFfiError::ImportFailed(format!(
                "spawning commit worker: {e}"
            )));
        }
        self.bump_generation();
        Ok(())
    }

    /// Snapshot of the commit worker: whether it is running plus the
    /// per-segment outcome once finished. Empty `per_segment` until a
    /// commit has been requested.
    #[must_use]
    pub fn job_progress(&self) -> RipJobProgress {
        let job = lock_mutex(&self.job);
        RipJobProgress {
            running: job.running,
            per_segment: job.per_segment.clone(),
        }
    }

    /// The session directory (spill, manifest, encoded tracks, side
    /// archive). The Apple shell uses it for "Reveal in Finder".
    #[must_use]
    pub fn session_dir(&self) -> String {
        lock_mutex(&self.session)
            .session_dir()
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::{Producer, Split};
    use ringbuf::HeapRb;
    use std::time::Instant;

    const SR: u32 = 44_100;

    /// Armed FFI session over a synthetic ring — the no-hardware
    /// analogue of `create_rip_session` (mirrors
    /// `dub-rip/tests/full_pipeline.rs`).
    fn armed_session(dir: &std::path::Path) -> (DubRipSession, ringbuf::HeapProd<f32>) {
        let mut cfg = RipConfig::new(SR, dir.join("session"));
        cfg.poll_interval = Duration::from_millis(1);
        let mut session = RipSession::new(cfg).unwrap();
        let (tx, rx) = HeapRb::<f32>::new(1 << 22).split();
        session.arm(rx).unwrap();
        (DubRipSession::from_parts(session, SR), tx)
    }

    /// Record `secs` seconds of a stereo tone and stop.
    fn recorded_session(dir: &std::path::Path, secs: u64) -> DubRipSession {
        let (ffi, mut tx) = armed_session(dir);
        ffi.start().unwrap();
        // Wait for the worker to consume CMD_START before pushing: a
        // stop() racing into the same single command slot would
        // overwrite it and the whole push would be discarded as
        // pre-record backlog. Real callers poll status() the same way.
        let deadline = Instant::now() + Duration::from_secs(5);
        while ffi.status().phase != RipPhase::Recording {
            assert!(Instant::now() < deadline, "worker never started recording");
            std::thread::sleep(Duration::from_millis(1));
        }
        let frames = usize::try_from(secs * u64::from(SR)).unwrap();
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / SR as f32;
            let s = 0.5 * (std::f32::consts::TAU * 220.0 * t).sin();
            samples.push(s);
            samples.push(s);
        }
        let mut pushed = 0;
        while pushed < samples.len() {
            pushed += tx.push_slice(&samples[pushed..]);
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(5));
        ffi.stop().unwrap();
        ffi
    }

    /// M26b: a side with real inter-track silence, recorded through
    /// the same worker the app uses. Track lengths clear the
    /// detector's production 30 s minimum, so no tuning is involved.
    #[test]
    fn auto_split_installs_detected_boundaries() {
        const TRACK_SECS: u64 = 40;
        const GAP_SECS: f64 = 2.0;

        let dir = tempfile::tempdir().unwrap();
        let (ffi, mut tx) = armed_session(dir.path());
        assert!(
            ffi.auto_split().is_err(),
            "auto split must be refused before the capture stops"
        );
        ffi.start().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while ffi.status().phase != RipPhase::Recording {
            assert!(Instant::now() < deadline, "worker never started recording");
            std::thread::sleep(Duration::from_millis(1));
        }

        let mut side: Vec<f32> = Vec::new();
        for _ in 0..2 {
            if !side.is_empty() {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let gap_frames = (GAP_SECS * f64::from(SR)) as u64;
                for i in 0..gap_frames {
                    // Groove noise 54 dB down, deterministic.
                    let s = if i % 2 == 0 { 0.002 } else { -0.002 };
                    side.push(s);
                    side.push(s);
                }
            }
            for i in 0..TRACK_SECS * u64::from(SR) {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / SR as f32;
                let s = 0.5 * (std::f32::consts::TAU * 220.0 * t).sin();
                side.push(s);
                side.push(s);
            }
        }
        let mut pushed = 0;
        while pushed < side.len() {
            pushed += tx.push_slice(&side[pushed..]);
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(5));
        ffi.stop().unwrap();

        let before = ffi.generation();
        assert_eq!(ffi.auto_split().unwrap(), 1);
        assert!(ffi.generation() > before, "plan mutation must bump");

        let markers = ffi.split_markers();
        assert_eq!(markers.len(), 1);
        #[allow(clippy::cast_precision_loss)]
        let expected = TRACK_SECS as f64 + GAP_SECS - 0.3;
        assert!(
            (markers[0].secs - expected).abs() < 0.3,
            "split at {} s, expected ~{expected} s",
            markers[0].secs
        );

        let segments = ffi.segments();
        assert_eq!(segments.len(), 2);
        assert!((segments[0].end_secs - markers[0].secs).abs() < f64::EPSILON);

        // Proposals are ordinary markers: removing one merges again.
        ffi.remove_split(markers[0].id).unwrap();
        assert_eq!(ffi.segments().len(), 1);
    }

    /// The run-out trim has to survive the trip through the FFI.
    ///
    /// It did not: this method used to run `detect_gaps` + `set_splits`
    /// itself instead of calling [`RipSession::auto_split`], which is
    /// the only writer of the side trim — so every rip driven from the
    /// app kept its run-out while the CLI dropped it, and nothing here
    /// noticed because no test looked past the boundary count.
    #[test]
    fn auto_split_trims_the_run_out_off_the_last_segment() {
        const TRACK_SECS: u64 = 40;
        const GAP_SECS: f64 = 2.0;
        const RUN_OUT_SECS: u64 = 40;

        let dir = tempfile::tempdir().unwrap();
        let (ffi, mut tx) = armed_session(dir.path());
        ffi.start().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while ffi.status().phase != RipPhase::Recording {
            assert!(Instant::now() < deadline, "worker never started recording");
            std::thread::sleep(Duration::from_millis(1));
        }

        let mut side: Vec<f32> = Vec::new();
        let push_tone = |side: &mut Vec<f32>| {
            for i in 0..TRACK_SECS * u64::from(SR) {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / SR as f32;
                let s = 0.5 * (std::f32::consts::TAU * 220.0 * t).sin();
                side.push(s);
                side.push(s);
            }
        };
        let push_groove = |side: &mut Vec<f32>, secs: f64| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let frames = (secs * f64::from(SR)) as u64;
            for i in 0..frames {
                let s = if i % 2 == 0 { 0.002 } else { -0.002 };
                side.push(s);
                side.push(s);
            }
        };
        push_tone(&mut side);
        push_groove(&mut side, GAP_SECS);
        push_tone(&mut side);
        #[allow(clippy::cast_precision_loss)]
        push_groove(&mut side, RUN_OUT_SECS as f64);

        let mut pushed = 0;
        while pushed < side.len() {
            pushed += tx.push_slice(&side[pushed..]);
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(5));
        ffi.stop().unwrap();

        assert_eq!(ffi.auto_split().unwrap(), 1, "expected the one gap");

        let recorded_secs = frames_to_secs(ffi.status().recorded_frames, SR);
        let segments = ffi.segments();
        let side_end = segments.last().unwrap().end_secs;
        #[allow(clippy::cast_precision_loss)]
        let music_end = 2.0 * TRACK_SECS as f64 + GAP_SECS;

        assert!(
            side_end < recorded_secs - 20.0,
            "run-out kept: side ends at {side_end:.1} s of {recorded_secs:.1} s"
        );
        assert!(
            (side_end - music_end).abs() < 5.0,
            "side ends at {side_end:.1} s, music ends at {music_end:.1} s"
        );
    }

    /// The operator can move both trims, and a trim that would
    /// swallow a split marker is refused rather than shoving it.
    #[test]
    fn side_bounds_move_and_are_refused_when_they_eat_a_split() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 90);

        let full = ffi.status();
        assert!((full.side_start_secs - 0.0).abs() < f64::EPSILON);
        assert!(
            (full.side_end_secs - 90.0).abs() < 0.5,
            "an untrimmed side reports its whole length, got {}",
            full.side_end_secs
        );

        ffi.add_split(45.0).unwrap();
        ffi.set_side_start(10.0).unwrap();
        ffi.set_side_end(80.0).unwrap();

        let trimmed = ffi.status();
        assert!((trimmed.side_start_secs - 10.0).abs() < 0.1);
        assert!((trimmed.side_end_secs - 80.0).abs() < 0.1);

        let segments = ffi.segments();
        assert_eq!(segments.len(), 2);
        assert!(
            (segments[0].start_secs - 10.0).abs() < 0.1,
            "first track starts at the side start"
        );
        assert!(
            (segments[1].end_secs - 80.0).abs() < 0.1,
            "last track ends at the side end"
        );

        // Past the marker: refused, and nothing moves.
        assert!(ffi.set_side_start(50.0).is_err());
        assert!((ffi.status().side_start_secs - 10.0).abs() < 0.1);
        assert!(ffi.set_side_end(40.0).is_err());
        assert!((ffi.status().side_end_secs - 80.0).abs() < 0.1);

        // And back to the whole side.
        ffi.set_side_start(0.0).unwrap();
        assert!((ffi.status().side_start_secs - 0.0).abs() < f64::EPSILON);
    }

    /// M26b: an interrupted rip is discoverable and reopens straight
    /// into review, with its plan and envelope intact.
    #[test]
    fn recoverable_session_is_listed_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let rips_root = dir.path().join("Rips");
        std::fs::create_dir_all(&rips_root).unwrap();

        // Record a side and leave it at review (spill still present).
        // `recorded_session` puts the session in `<dir>/session`, so
        // pass the root directly — `list_recoverable` scans immediate
        // children for a manifest.
        let ffi = recorded_session(&rips_root, 12);
        let session_dir = ffi.session_dir();
        drop(ffi);

        let engine = DubEngine::new();
        let found =
            engine.list_recoverable_rip_sessions(Some(rips_root.to_string_lossy().into_owned()));
        assert_eq!(found.len(), 1, "unfinished rip must be listed: {found:?}");
        assert!(found[0].recorded_secs > 11.0);

        let resumed = engine
            .resume_rip_session(found[0].session_dir.clone())
            .expect("resume");
        let status = resumed.status();
        assert_eq!(status.phase, RipPhase::Stopped);
        assert_eq!(status.stop_reason, RipStopReason::Recovered);
        assert!(status.recorded_frames > 0);
        assert!(resumed.envelope_len() > 0, "envelope rebuilt from spill");
        assert_eq!(resumed.session_dir(), session_dir);
        // Editing works on a resumed session like any other.
        let id = resumed.add_split(6.0).expect("split on resumed session");
        assert_eq!(resumed.segments().len(), 2);
        resumed.remove_split(id).unwrap();
    }

    #[test]
    fn listing_recoverable_sessions_ignores_empty_and_missing_dirs() {
        let engine = DubEngine::new();
        let dir = tempfile::tempdir().unwrap();
        assert!(engine
            .list_recoverable_rip_sessions(Some(dir.path().to_string_lossy().into_owned()))
            .is_empty());
        assert!(engine
            .list_recoverable_rip_sessions(Some("/nope/not/here".into()))
            .is_empty());
    }

    #[test]
    fn auto_split_on_a_gapless_side_leaves_one_segment() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 12);
        assert_eq!(ffi.auto_split().unwrap(), 0);
        assert!(ffi.split_markers().is_empty());
        assert_eq!(ffi.segments().len(), 1);
    }

    #[test]
    fn phase_and_stop_reason_map_every_session_state() {
        assert_eq!(phase_for(&RipState::Idle), RipPhase::Idle);
        assert_eq!(phase_for(&RipState::Armed), RipPhase::Armed);
        assert_eq!(phase_for(&RipState::Recording), RipPhase::Recording);
        assert_eq!(
            phase_for(&RipState::Stopped(StopReason::Manual)),
            RipPhase::Stopped
        );
        assert_eq!(phase_for(&RipState::Failed), RipPhase::Failed);

        assert_eq!(stop_reason_for(&RipState::Recording), RipStopReason::None);
        assert_eq!(
            stop_reason_for(&RipState::Stopped(StopReason::Manual)),
            RipStopReason::Manual
        );
        assert_eq!(
            stop_reason_for(&RipState::Stopped(StopReason::MaxDuration)),
            RipStopReason::MaxDuration
        );
        assert_eq!(
            stop_reason_for(&RipState::Stopped(StopReason::InputLost)),
            RipStopReason::InputLost
        );
    }

    #[test]
    fn secs_frames_mapping_round_trips() {
        for &frames in &[1_u64, 64, 44_100, 44_101, 10 * 44_100 + 7, 2_646_000] {
            let secs = frames_to_secs(frames, SR);
            assert_eq!(secs_to_frames(secs, SR), frames, "frames {frames}");
        }
        assert_eq!(secs_to_frames(0.0, SR), 0);
        assert_eq!(secs_to_frames(-1.0, SR), 0);
        assert_eq!(secs_to_frames(f64::NAN, SR), 0);
        assert_eq!(frames_to_secs(123, 0), 0.0);
    }

    #[test]
    fn timestamp_formats_known_civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2024-01-01 = 19 723 days after the epoch (13 leap years in
        // 1970..2024); 2024-02-29 exercises the leap branch.
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_723 + 31 + 28), (2024, 2, 29));
        let stamp = timestamp_for(UNIX_EPOCH + Duration::from_secs(19_723 * 86_400 + 3_723));
        assert_eq!(stamp, "20240101-010203");
    }

    #[test]
    fn create_rip_session_on_stopped_engine_is_not_capturing() {
        let engine = DubEngine::new();
        let err = engine
            .create_rip_session(
                0,
                RipSessionConfig {
                    dest_dir: None,
                    max_duration_secs: 2_400.0,
                    auto_start: false,
                    auto_stop: false,
                },
            )
            .unwrap_err();
        assert!(matches!(err, RipFfiError::NotCapturing), "got {err:?}");

        // Config validation fires before the engine-state check.
        let err = engine
            .create_rip_session(
                0,
                RipSessionConfig {
                    dest_dir: None,
                    max_duration_secs: 0.0,
                    auto_start: false,
                    auto_stop: false,
                },
            )
            .unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidConfig(_)), "got {err:?}");
        let err = engine
            .create_rip_session(
                99,
                RipSessionConfig {
                    dest_dir: None,
                    max_duration_secs: 2_400.0,
                    auto_start: false,
                    auto_stop: false,
                },
            )
            .unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidConfig(_)), "got {err:?}");
    }

    #[test]
    fn status_reports_manual_stop_with_recorded_length() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 6);
        let status = ffi.status();
        assert_eq!(status.phase, RipPhase::Stopped);
        assert_eq!(status.stop_reason, RipStopReason::Manual);
        assert_eq!(status.recorded_frames, 6 * u64::from(SR));
        assert!((status.elapsed_secs - 6.0).abs() < 1e-9);
        assert!(status.error.is_none());
    }

    #[test]
    fn envelope_extend_packs_12_byte_le_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 6);

        let len = ffi.envelope_len();
        assert_eq!(len, 6 * u64::from(SR) / 64);
        let bytes = ffi.envelope_extend(0);
        assert_eq!(bytes.len(), usize::try_from(len).unwrap() * 12);

        // First chunk round-trips through the packing.
        let reference = lock_mutex(&ffi.session).envelope_from(0);
        let min = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let max = f32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let rms = f32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(min, reference[0].min);
        assert_eq!(max, reference[0].max);
        assert_eq!(rms, reference[0].rms);

        // Incremental fetch: past-the-end start is empty, tail fetch
        // returns exactly the remainder.
        assert!(ffi.envelope_extend(len).is_empty());
        assert_eq!(ffi.envelope_extend(len - 2).len(), 2 * 12);
    }

    #[test]
    fn split_crud_keeps_ids_stable_and_boundaries_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 18);
        let gen0 = ffi.generation();

        // Out-of-order inserts come back sorted by position.
        let id_late = ffi.add_split(12.0).unwrap();
        let id_early = ffi.add_split(6.0).unwrap();
        assert_ne!(id_late, id_early);
        let markers = ffi.split_markers();
        assert_eq!(
            markers.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![id_early, id_late]
        );
        assert!((markers[0].secs - 6.0).abs() < 1e-9);
        assert!(ffi.generation() > gen0);

        // A 1 s head segment violates the 5 s minimum → rejected, plan
        // untouched.
        let err = ffi.add_split(1.0).unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidSplit(_)), "got {err:?}");
        assert_eq!(ffi.split_markers().len(), 2);

        // Duplicate position → rejected.
        let err = ffi.add_split(6.0).unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidSplit(_)));

        // Move keeps the id; invalid move rolls back.
        ffi.move_split(id_early, 7.0).unwrap();
        let markers = ffi.split_markers();
        assert_eq!(markers[0].id, id_early);
        assert!((markers[0].secs - 7.0).abs() < 1e-9);
        let err = ffi.move_split(id_early, 11.0).unwrap_err(); // 11→12 = 1 s segment
        assert!(matches!(err, RipFfiError::InvalidSplit(_)));
        assert!((ffi.split_markers()[0].secs - 7.0).abs() < 1e-9);
        let err = ffi.move_split(9_999, 8.0).unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidSplit(_)));

        // Remove merges segments; the other id survives.
        ffi.remove_split(id_late).unwrap();
        let markers = ffi.split_markers();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].id, id_early);
        let err = ffi.remove_split(id_late).unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidSplit(_)));

        // Segments derive from the surviving boundary.
        let segments = ffi.segments();
        assert_eq!(segments.len(), 2);
        assert!((segments[0].start_secs - 0.0).abs() < 1e-9);
        assert!((segments[0].end_secs - 7.0).abs() < 1e-9);
        assert!((segments[1].end_secs - 18.0).abs() < 1e-9);
    }

    #[test]
    fn splits_are_rejected_while_recording() {
        let dir = tempfile::tempdir().unwrap();
        let (ffi, _tx) = armed_session(dir.path());
        ffi.start().unwrap();
        let err = ffi.add_split(6.0).unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidState(_)), "got {err:?}");
    }

    #[test]
    fn whole_side_metadata_is_editable_without_splits() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 6);
        ffi.set_segment_metadata(
            0,
            Some("Real Rock".into()),
            Some("Sound Dimension".into()),
            None,
            Some("Reggae".into()),
            Some(1967),
        )
        .unwrap();
        let segments = ffi.segments();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].title.as_deref(), Some("Real Rock"));
        assert_eq!(segments[0].year, Some(1967));

        let err = ffi
            .set_segment_metadata(5, None, None, None, None, None)
            .unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidSegment(_)), "got {err:?}");
    }

    #[test]
    fn commit_imports_segments_and_reports_done() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 12);
        ffi.add_split(6.0).unwrap();
        ffi.set_segment_metadata(
            0,
            Some("Side A".into()),
            Some("Tester".into()),
            None,
            None,
            None,
        )
        .unwrap();

        let library = DubLibrary::new();
        let lib_path = dir.path().join("library.sqlite");
        library
            .open_at(lib_path.to_string_lossy().into_owned())
            .unwrap();

        // Commit requires a stopped capture and no running job.
        ffi.confirm_encode_and_import(Arc::clone(&library)).unwrap();

        let deadline = Instant::now() + Duration::from_secs(120);
        let final_status = loop {
            let status = ffi.status();
            if status.phase != RipPhase::Encoding {
                break status;
            }
            assert!(Instant::now() < deadline, "commit worker timed out");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(
            final_status.phase,
            RipPhase::Done,
            "commit failed: {:?}",
            final_status.error
        );
        assert_eq!(final_status.recorded_frames, 12 * u64::from(SR));

        let progress = ffi.job_progress();
        assert!(!progress.running);
        assert_eq!(progress.per_segment.len(), 2);
        for seg in &progress.per_segment {
            assert_eq!(seg.state, RipSegmentJobState::Done, "segment {}", seg.index);
            assert!(seg.imported_track_id.is_some());
        }
        assert_eq!(library.track_count().unwrap(), 2);

        // A committed session refuses further plan edits.
        let err = ffi.add_split(8.0).unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidState(_)), "got {err:?}");

        // Cancel must refuse too: the imported FLACs live inside the
        // session dir and the library references them by path —
        // deleting the dir would orphan the just-imported tracks.
        let session_dir = PathBuf::from(ffi.session_dir());
        let err = ffi.cancel().unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidState(_)), "got {err:?}");
        assert!(
            session_dir.exists(),
            "cancel after import must not delete the session dir"
        );
        assert_eq!(library.track_count().unwrap(), 2);
    }

    #[test]
    fn commit_fails_cleanly_on_a_closed_library() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 6);
        let library = DubLibrary::new(); // never opened

        ffi.confirm_encode_and_import(library).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let final_status = loop {
            let status = ffi.status();
            if status.phase != RipPhase::Encoding {
                break status;
            }
            assert!(Instant::now() < deadline, "commit worker timed out");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(final_status.phase, RipPhase::Failed);
        assert!(final_status.error.unwrap().contains("library not open"));
        let progress = ffi.job_progress();
        assert!(progress
            .per_segment
            .iter()
            .all(|s| s.state == RipSegmentJobState::Failed));
    }

    #[test]
    fn commit_is_rejected_before_stop() {
        let dir = tempfile::tempdir().unwrap();
        let (ffi, _tx) = armed_session(dir.path());
        let err = ffi
            .confirm_encode_and_import(DubLibrary::new())
            .unwrap_err();
        assert!(matches!(err, RipFfiError::InvalidState(_)), "got {err:?}");
    }

    #[test]
    fn cancel_stops_capture_and_removes_the_session_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ffi = recorded_session(dir.path(), 6);
        let session_dir = PathBuf::from(ffi.session_dir());
        assert!(session_dir.exists());

        ffi.cancel().unwrap();
        assert!(!session_dir.exists(), "session dir must be deleted");
        assert_eq!(ffi.status().phase, RipPhase::Idle);

        // Idempotent; everything else refuses.
        ffi.cancel().unwrap();
        assert!(matches!(
            ffi.start().unwrap_err(),
            RipFfiError::InvalidState(_)
        ));
        assert!(matches!(
            ffi.add_split(3.0).unwrap_err(),
            RipFfiError::InvalidState(_)
        ));
    }
}
