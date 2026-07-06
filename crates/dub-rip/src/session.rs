//! `RipSession` — the off-RT owner of one record-a-side workflow:
//! capture (via the worker in [`crate::capture`]), split plan,
//! manifest persistence, and commit orchestration.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ringbuf::HeapCons;

use crate::capture::{
    self, CaptureConfig, CaptureShared, CMD_START, CMD_STOP, REASON_INPUT_LOST, REASON_MANUAL,
    REASON_MAX_DURATION, STATE_ARMED, STATE_FAILED, STATE_IDLE, STATE_RECORDING, STATE_STOPPED,
};
use crate::commit;
use crate::manifest::{self, ManifestError, RipManifest, TrackEntry};
use crate::plan::{self, SplitError, TrackMeta, MIN_SEGMENT_SECS};

/// File name of the crash-safe capture spill inside the session dir.
pub const SPILL_FILE: &str = "side.raw.wav";

/// File name of the lossless side archive written at commit.
pub const ARCHIVE_FILE: &str = "side.flac";

/// Why a recording stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The operator pressed stop.
    Manual,
    /// The `max_duration_secs` hard cap was reached.
    MaxDuration,
    /// The record-tap producer went away (engine detached / audio
    /// device stopped). What was captured up to that point is intact.
    InputLost,
}

/// Session lifecycle as seen by pollers. M26a subset: the M26b
/// auto-start/auto-stop and the review/encode sub-states live in the
/// FFI layer's coarser phase model, not here.
#[derive(Debug, Clone, PartialEq)]
pub enum RipState {
    /// Constructed; no capture worker yet.
    Idle,
    /// Worker running, draining and discarding — waiting for
    /// [`RipSession::start`].
    Armed,
    /// Writing the spill.
    Recording,
    /// Capture finished; splits/metadata may be edited and
    /// [`RipSession::commit`] may run.
    Stopped(StopReason),
    /// Capture worker hit an unrecoverable error (message in
    /// [`RipStatus::failure`]).
    Failed,
}

/// Polled snapshot of the live session.
#[derive(Debug, Clone, PartialEq)]
pub struct RipStatus {
    /// Lifecycle state.
    pub state: RipState,
    /// Frames written to the spill so far.
    pub recorded_frames: u64,
    /// Recorded duration in seconds.
    pub elapsed_secs: f64,
    /// |peak| of the most recent capture window (level meter).
    pub window_peak: f32,
    /// Failure message when `state == Failed`.
    pub failure: Option<String>,
}

/// Session configuration.
#[derive(Debug, Clone)]
pub struct RipConfig {
    /// Engine/interface sample rate the record tap runs at.
    pub sample_rate: u32,
    /// Directory this session owns (created by [`RipSession::new`]).
    /// User-visible data — the encoded tracks are imported in place.
    pub session_dir: PathBuf,
    /// Hard recording cap. Defaults to 40 minutes — beyond any
    /// vinyl side; a forgotten needle in the runout groove must not
    /// fill the disk.
    pub max_duration_secs: f32,
    /// Capture worker poll cadence. 20 ms in production (matches
    /// `PeakStream`); tests shrink it to drain faster than realtime.
    pub poll_interval: Duration,
}

impl RipConfig {
    /// Config with production defaults.
    #[must_use]
    pub fn new(sample_rate: u32, session_dir: PathBuf) -> Self {
        Self {
            sample_rate,
            session_dir,
            max_duration_secs: 2_400.0,
            poll_interval: Duration::from_millis(20),
        }
    }
}

/// Errors from [`RipSession`] operations.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum RipError {
    #[error("invalid rip config: {0}")]
    InvalidConfig(String),

    #[error("session io failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("cannot {op} while session is {state}")]
    InvalidState { op: &'static str, state: String },

    #[error(transparent)]
    Split(#[from] SplitError),

    #[error(transparent)]
    Manifest(#[from] ManifestError),

    #[error("capture failed: {0}")]
    CaptureFailed(String),

    #[error("timed out waiting for capture to stop")]
    StopTimeout,

    #[error("track index {index} out of range ({count} segments)")]
    TrackIndexOutOfRange { index: usize, count: usize },

    #[error("side spill unreadable: {0}")]
    SpillUnreadable(String),
}

/// One record-a-side session. See the crate docs for the workflow.
pub struct RipSession {
    cfg: RipConfig,
    shared: Arc<CaptureShared>,
    worker: Option<JoinHandle<()>>,
    manifest: RipManifest,
}

impl RipSession {
    /// Create a session: validates the config, creates the session
    /// directory, writes the initial manifest.
    pub fn new(cfg: RipConfig) -> Result<Self, RipError> {
        if cfg.sample_rate == 0 {
            return Err(RipError::InvalidConfig("sample_rate must be > 0".into()));
        }
        if cfg.max_duration_secs <= 0.0 {
            return Err(RipError::InvalidConfig(
                "max_duration_secs must be > 0".into(),
            ));
        }
        std::fs::create_dir_all(&cfg.session_dir)?;
        let manifest = RipManifest::new(cfg.sample_rate, 2);
        manifest::save(&cfg.session_dir, &manifest)?;
        Ok(Self {
            cfg,
            shared: Arc::new(CaptureShared::new()),
            worker: None,
            manifest,
        })
    }

    /// Path of the capture spill WAV.
    #[must_use]
    pub fn spill_path(&self) -> PathBuf {
        self.cfg.session_dir.join(SPILL_FILE)
    }

    /// The session directory.
    #[must_use]
    pub fn session_dir(&self) -> &Path {
        &self.cfg.session_dir
    }

    /// Current manifest (splits, metadata, commit progress).
    #[must_use]
    pub fn manifest(&self) -> &RipManifest {
        &self.manifest
    }

    /// Spawn the capture worker on the record-tap consumer. The
    /// session moves to `Armed`: draining but discarding until
    /// [`Self::start`].
    pub fn arm(&mut self, record_rx: HeapCons<f32>) -> Result<(), RipError> {
        if self.worker.is_some() {
            return Err(self.invalid_state("arm"));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_frames =
            (f64::from(self.cfg.sample_rate) * f64::from(self.cfg.max_duration_secs)) as u64;
        let worker = capture::spawn(
            record_rx,
            Arc::clone(&self.shared),
            CaptureConfig {
                wav_path: self.spill_path(),
                sample_rate: self.cfg.sample_rate,
                max_frames,
                poll_interval: self.cfg.poll_interval,
            },
        )?;
        // Armed is visible the moment `arm` returns — a `start()`
        // racing the worker's thread startup must not bounce off
        // `Idle`. (The worker re-stores Armed harmlessly; a spill
        // creation failure overwrites with Failed.)
        self.shared.state.store(STATE_ARMED, Ordering::Release);
        self.worker = Some(worker);
        Ok(())
    }

    /// Begin writing the spill (Armed → Recording).
    pub fn start(&self) -> Result<(), RipError> {
        match self.shared.state.load(Ordering::Acquire) {
            STATE_ARMED | STATE_RECORDING => {
                self.shared.command.store(CMD_START, Ordering::Release);
                Ok(())
            }
            _ => Err(self.invalid_state("start")),
        }
    }

    /// Request a stop. The worker drains the ring tail, finalizes
    /// the WAV, and lands in `Stopped`; poll [`Self::status`] or use
    /// [`Self::wait_stopped`].
    pub fn stop(&self) -> Result<(), RipError> {
        match self.shared.state.load(Ordering::Acquire) {
            STATE_ARMED | STATE_RECORDING => {
                self.shared.command.store(CMD_STOP, Ordering::Release);
                Ok(())
            }
            _ => Err(self.invalid_state("stop")),
        }
    }

    /// Polled status snapshot.
    #[must_use]
    pub fn status(&self) -> RipStatus {
        let recorded_frames = self.shared.recorded_frames.load(Ordering::Acquire);
        let state = match self.shared.state.load(Ordering::Acquire) {
            STATE_IDLE => RipState::Idle,
            STATE_ARMED => RipState::Armed,
            STATE_RECORDING => RipState::Recording,
            STATE_STOPPED => RipState::Stopped(self.stop_reason()),
            _ => RipState::Failed,
        };
        RipStatus {
            state,
            recorded_frames,
            elapsed_secs: recorded_frames as f64 / f64::from(self.cfg.sample_rate),
            window_peak: f32::from_bits(self.shared.window_peak_bits.load(Ordering::Acquire)),
            failure: self.shared.failure_guard().clone(),
        }
    }

    /// Block until the worker reaches a terminal state (after a
    /// [`Self::stop`], input loss, or the duration cap), join it, and
    /// sync the recorded length into the manifest. 2 ms poll — this
    /// is an off-RT convenience for the CLI and tests; the UI polls
    /// [`Self::status`] instead and calls this once stopped.
    pub fn wait_stopped(&mut self, timeout: Duration) -> Result<RipStatus, RipError> {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self.shared.state.load(Ordering::Acquire);
            if state == STATE_STOPPED || state == STATE_FAILED {
                break;
            }
            if Instant::now() >= deadline {
                return Err(RipError::StopTimeout);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if let Some(worker) = self.worker.take() {
            // A worker that panicked has already poisoned nothing we
            // rely on (fail() writes state first); surface it as a
            // capture failure rather than propagating the panic.
            if worker.join().is_err() {
                return Err(RipError::CaptureFailed("capture worker panicked".into()));
            }
        }
        let status = self.status();
        if status.state == RipState::Failed {
            return Err(RipError::CaptureFailed(
                status.failure.clone().unwrap_or_else(|| "unknown".into()),
            ));
        }
        self.manifest.recorded_frames = status.recorded_frames;
        manifest::save(&self.cfg.session_dir, &self.manifest)?;
        Ok(status)
    }

    /// Number of envelope chunks accumulated so far.
    #[must_use]
    pub fn envelope_len(&self) -> usize {
        self.shared.envelope_guard().len()
    }

    /// Copy of the envelope from `from_chunk` on — the incremental
    /// fetch pattern the live waveform uses.
    #[must_use]
    pub fn envelope_from(&self, from_chunk: usize) -> Vec<dub_peaks::PeakChunk> {
        let guard = self.shared.envelope_guard();
        guard.get(from_chunk..).map_or_else(Vec::new, <[_]>::to_vec)
    }

    /// Set the split boundaries (frames where the next track starts).
    /// Only valid once stopped. Re-splitting preserves per-segment
    /// metadata by index. Persists the manifest.
    pub fn set_splits(&mut self, boundaries_frames: Vec<u64>) -> Result<(), RipError> {
        self.require_stopped("set splits")?;
        plan::validate_boundaries(
            &boundaries_frames,
            self.manifest.recorded_frames,
            self.cfg.sample_rate,
            MIN_SEGMENT_SECS,
        )?;
        self.manifest.boundaries_frames = boundaries_frames;
        self.manifest.tracks.resize_with(
            self.manifest.boundaries_frames.len() + 1,
            TrackEntry::default,
        );
        manifest::save(&self.cfg.session_dir, &self.manifest)?;
        Ok(())
    }

    /// Set one segment's metadata. Persists the manifest.
    pub fn set_track_meta(&mut self, index: usize, meta: TrackMeta) -> Result<(), RipError> {
        self.require_stopped("set track metadata")?;
        let count = self.manifest.tracks.len();
        let entry = self
            .manifest
            .tracks
            .get_mut(index)
            .ok_or(RipError::TrackIndexOutOfRange { index, count })?;
        entry.meta = meta;
        manifest::save(&self.cfg.session_dir, &self.manifest)?;
        Ok(())
    }

    /// Encode + tag + import every segment, write the side archive,
    /// and delete the spill once everything succeeded. Idempotent:
    /// segments that already carry a `library_uuid` are skipped, so
    /// a partial failure is retried by calling `commit` again.
    ///
    /// Per-segment failures are collected in the returned
    /// [`crate::RipOutcome`], not raised — one broken segment must
    /// not strand the rest of the side.
    pub fn commit(
        &mut self,
        library: &mut dub_library::Library,
    ) -> Result<crate::RipOutcome, RipError> {
        self.require_stopped("commit")?;
        if self.manifest.tracks.is_empty() {
            // No splits set: the whole side is one track.
            self.manifest.tracks.push(TrackEntry::default());
        }
        commit::commit_session(
            &self.cfg.session_dir,
            &self.spill_path(),
            &mut self.manifest,
            library,
        )
    }

    fn require_stopped(&self, op: &'static str) -> Result<(), RipError> {
        if self.worker.is_some() || self.shared.state.load(Ordering::Acquire) != STATE_STOPPED {
            return Err(self.invalid_state_op(op));
        }
        Ok(())
    }

    fn stop_reason(&self) -> StopReason {
        match self.shared.stop_reason.load(Ordering::Acquire) {
            REASON_MAX_DURATION => StopReason::MaxDuration,
            REASON_INPUT_LOST => StopReason::InputLost,
            // REASON_MANUAL and (defensively) anything unexpected.
            _ => StopReason::Manual,
        }
    }

    fn invalid_state(&self, op: &'static str) -> RipError {
        self.invalid_state_op(op)
    }

    fn invalid_state_op(&self, op: &'static str) -> RipError {
        RipError::InvalidState {
            op,
            state: format!("{:?}", self.status().state),
        }
    }
}

// Never leave a live capture thread behind: request a stop and join.
// The worker exits its loop on CMD_STOP within one poll interval.
impl Drop for RipSession {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.shared.command.store(CMD_STOP, Ordering::Release);
            let _ = worker.join();
        }
    }
}

// REASON_MANUAL is only read through `stop_reason`'s catch-all arm;
// keep the symbol referenced so the constant table stays complete.
const _: u8 = REASON_MANUAL;
