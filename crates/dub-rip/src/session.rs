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
    REASON_MAX_DURATION, REASON_RECOVERED, REASON_SILENCE, STATE_ARMED, STATE_FAILED, STATE_IDLE,
    STATE_RECORDING, STATE_STOPPED,
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
    /// The side ran out: the input sat under the music by
    /// [`AutoCapture::silence_drop_db`] for
    /// [`AutoCapture::silence_stop_secs`] (M26b).
    Silence,
    /// Reopened from disk by [`RipSession::from_session_dir`] after an
    /// interrupted rip — the capture never stopped, the process did.
    Recovered,
}

/// Hands-off capture: start on the needle drop, stop in the run-out.
///
/// Both halves are opt-in and independent — [`AutoCapture::manual`]
/// (the [`RipConfig::new`] default) preserves the M26a behaviour where
/// the operator drives both ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoCapture {
    /// Peak level that starts the recording. `None` waits for
    /// [`RipSession::start`].
    pub start_threshold: Option<f32>,
    /// Seconds of pre-trigger audio kept while armed and written
    /// ahead of the trigger, so the needle drop that started the rip
    /// is *in* the rip. Ignored when `start_threshold` is `None`.
    pub pre_roll_secs: f32,
    /// Stop after this long below the quiet line. `None` waits for
    /// [`RipSession::stop`].
    pub silence_stop_secs: Option<f32>,
    /// How far under the running music level counts as quiet. Keeps
    /// the gate relative rather than absolute — see `SilenceGate`.
    pub silence_drop_db: f32,
}

impl AutoCapture {
    /// Operator drives both ends (M26a behaviour).
    #[must_use]
    pub fn manual() -> Self {
        Self {
            start_threshold: None,
            pre_roll_secs: 0.0,
            silence_stop_secs: None,
            silence_drop_db: 25.0,
        }
    }
}

impl Default for AutoCapture {
    /// Hands-off defaults: −40 dBFS trigger (a needle in the groove
    /// clears it; room noise through a phono stage does not), 1 s of
    /// pre-roll, and a 30 s run-out timeout at 18 dB under the side's
    /// loudest moment.
    ///
    /// Both stop numbers are measured across three real sides, and
    /// both are deliberately slack. The run-out on a worn soul
    /// sampler sits only 20 dB under its music, so a 25 dB drop never
    /// fired at all and 14 dB stopped that side mid-record, after two
    /// tracks — the window that works on every side on hand is 16–18,
    /// and 18 leaves the most room under the music. The timer is 30 s
    /// rather than 20 for the same reason: the longest mid-side quiet
    /// stretch measured is 11.8 s, and a false stop truncates a side
    /// while a late one costs only the disk it writes. Stopping late
    /// is nearly free now that the detector trims the run-out off the
    /// last track anyway.
    fn default() -> Self {
        Self {
            start_threshold: Some(0.01),
            pre_roll_secs: 1.0,
            silence_stop_secs: Some(30.0),
            silence_drop_db: 18.0,
        }
    }
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
    /// Hands-off start/stop (M26b). Defaults to
    /// [`AutoCapture::manual`].
    pub auto: AutoCapture,
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
            auto: AutoCapture::manual(),
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

    /// Reopen an interrupted session from its directory (M26b).
    ///
    /// The session comes back **stopped**, with the plan and metadata
    /// the manifest last recorded and an envelope rebuilt by scanning
    /// the spill — everything the review UI needs, minus a capture
    /// worker (that process is gone). Splits, metadata and commit all
    /// work exactly as they did before the interruption.
    ///
    /// The recorded length comes from the *file*, not the manifest:
    /// `recorded_frames` is only synced at [`Self::wait_stopped`], so
    /// a rip that died mid-side has a manifest claiming zero frames
    /// over a spill holding twenty minutes of music.
    ///
    /// # Errors
    ///
    /// [`RipError::Manifest`] if `rip.json` is missing or malformed,
    /// [`RipError::SpillUnreadable`] if the spill is gone or is not a
    /// capture WAV.
    pub fn from_session_dir(session_dir: PathBuf) -> Result<Self, RipError> {
        let mut manifest = manifest::load(&session_dir)?;
        let spill = session_dir.join(SPILL_FILE);
        let (envelope, info) = crate::salvage::rebuild_envelope(&spill)?;

        manifest.recorded_frames = info.frames;
        if manifest.sample_rate == 0 {
            manifest.sample_rate = info.sample_rate;
        }
        // A plan saved before the crash may not survive the recovered
        // length (the side is shorter than the operator thought).
        // Drop what no longer validates rather than refusing to open —
        // the trims go with it, since they were measured against the
        // length the manifest used to claim.
        if plan::validate_boundaries(
            &manifest.boundaries_frames,
            manifest.side_start(),
            manifest.side_end(),
            manifest.sample_rate,
            MIN_SEGMENT_SECS,
        )
        .is_err()
        {
            manifest.boundaries_frames.clear();
            manifest.tracks.truncate(1);
            manifest.side_start_frame = None;
            manifest.side_end_frame = None;
        }
        manifest::save(&session_dir, &manifest)?;

        let shared = Arc::new(CaptureShared::new());
        shared.state.store(STATE_STOPPED, Ordering::Release);
        shared
            .stop_reason
            .store(REASON_RECOVERED, Ordering::Release);
        shared
            .recorded_frames
            .store(manifest.recorded_frames, Ordering::Release);
        *shared.envelope_guard() = envelope;

        let mut cfg = RipConfig::new(manifest.sample_rate, session_dir);
        // Nothing will record on this session; keep the cap honest
        // anyway so a re-armed future never inherits a zero.
        cfg.max_duration_secs = cfg.max_duration_secs.max(
            #[allow(clippy::cast_precision_loss)]
            {
                manifest.recorded_frames as f32 / manifest.sample_rate.max(1) as f32
            },
        );
        Ok(Self {
            cfg,
            shared,
            worker: None,
            manifest,
        })
    }

    /// Reopen a *committed* session to split it again from the
    /// lossless side archive (M26b).
    ///
    /// For the case the review screen cannot catch: the split looked
    /// right, the tracks imported, and only later — listening — does a
    /// boundary turn out to be wrong. `side.flac` is the whole side,
    /// so this needs no record and no turntable.
    ///
    /// The plan is cleared and the previously imported UUIDs move to
    /// [`RipManifest::replaced_uuids`]: the next successful commit
    /// imports the new segments and *then* removes the old tracks, so
    /// a failure part-way leaves the library with the originals rather
    /// than with nothing.
    ///
    /// # Errors
    ///
    /// [`RipError::Manifest`] if the manifest is missing or malformed;
    /// [`RipError::SpillUnreadable`] if the archive cannot be decoded.
    pub fn resplit_from_archive(session_dir: PathBuf) -> Result<Self, RipError> {
        let mut manifest = manifest::load(&session_dir)?;
        let archive = session_dir.join(ARCHIVE_FILE);
        if !archive.is_file() {
            return Err(RipError::SpillUnreadable(format!(
                "no side archive at {}",
                archive.display()
            )));
        }
        let track = dub_io::Track::load_from_path(&archive)
            .map_err(|e| RipError::SpillUnreadable(format!("{e}")))?;
        let frames = track.frames() as u64;
        let sample_rate = track.sample_rate();

        // Carry the old tracks forward for removal, not deletion now:
        // nothing is destroyed until the replacement is safely in.
        let mut replaced: Vec<String> = std::mem::take(&mut manifest.replaced_uuids);
        replaced.extend(
            manifest
                .tracks
                .iter()
                .filter_map(|t| t.library_uuid.clone()),
        );
        replaced.sort_unstable();
        replaced.dedup();

        manifest.replaced_uuids = replaced;
        manifest.split_generation = manifest.split_generation.max(1).saturating_add(1);
        manifest.recorded_frames = frames;
        manifest.sample_rate = sample_rate;
        manifest.boundaries_frames.clear();
        // The archive is the whole capture, lead-in and run-out
        // included, so the previous split's trims no longer bind: a
        // fresh auto-split measures its own, and a manual one gets the
        // side entire.
        manifest.side_end_frame = None;
        manifest.side_start_frame = None;
        manifest.tracks = vec![TrackEntry::default()];
        manifest::save(&session_dir, &manifest)?;

        let envelope = crate::salvage::envelope_from_samples(track.samples(), track.channels());
        let shared = Arc::new(CaptureShared::new());
        shared.state.store(STATE_STOPPED, Ordering::Release);
        shared
            .stop_reason
            .store(REASON_RECOVERED, Ordering::Release);
        shared.recorded_frames.store(frames, Ordering::Release);
        *shared.envelope_guard() = envelope;

        Ok(Self {
            cfg: RipConfig::new(sample_rate, session_dir),
            shared,
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
                auto: self.cfg.auto,
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

    /// Propose track boundaries from the captured envelope without
    /// installing them — the review UI draws these before the
    /// operator accepts. See [`crate::detect_gaps`].
    #[must_use]
    pub fn detect_gaps(&self, cfg: &crate::GapConfig) -> Vec<crate::Gap> {
        let envelope = self.shared.envelope_guard();
        crate::detect_gaps(
            &envelope,
            dub_peaks::DEFAULT_SAMPLES_PER_CHUNK,
            self.cfg.sample_rate,
            self.manifest.recorded_frames,
            cfg,
        )
    }

    /// Install the detected gaps as the split plan and persist it.
    /// Returns the resulting segment count.
    ///
    /// Stopped-only, like every plan mutation: the boundaries are
    /// validated against `recorded_frames`, which is only final once
    /// [`Self::wait_stopped`] has synced it into the manifest. A side
    /// with no detectable gaps stays one segment — that is a valid
    /// plan, not an error.
    pub fn auto_split(&mut self, cfg: &crate::GapConfig) -> Result<usize, RipError> {
        self.require_stopped("auto split")?;
        // Trim the side before splitting it: the boundaries validate
        // against the side end, and the last track has to stop at the
        // last music rather than carrying the run-out groove.
        let end = {
            let envelope = self.shared.envelope_guard();
            crate::analyze_gaps(
                &envelope,
                dub_peaks::DEFAULT_SAMPLES_PER_CHUNK,
                self.cfg.sample_rate,
                cfg,
            )
        };
        let previous = (self.manifest.side_start_frame, self.manifest.side_end_frame);
        if end.usable && end.music_end_frame > 0 {
            self.manifest.side_end_frame =
                Some(end.music_end_frame.min(self.manifest.recorded_frames));
            self.manifest.side_start_frame =
                Some(end.side_start_frame.min(self.manifest.recorded_frames));
        }
        let boundaries = self
            .detect_gaps(cfg)
            .iter()
            .map(|gap| gap.boundary_frame)
            .collect();
        if let Err(e) = self.set_splits(boundaries) {
            (self.manifest.side_start_frame, self.manifest.side_end_frame) = previous;
            return Err(e);
        }
        Ok(self.manifest.tracks.len())
    }

    /// Move the side's bounds — where the lead-in ends and the
    /// run-out begins. Everything outside is discarded at commit;
    /// `side.flac` still archives the whole capture, so nothing here
    /// is irreversible.
    ///
    /// Stopped-only, and validated against the existing plan: a trim
    /// that would swallow a split marker, or leave a segment under the
    /// minimum, is refused rather than silently moving markers.
    ///
    /// # Errors
    ///
    /// [`RipError::InvalidState`] unless stopped; [`RipError::Split`]
    /// when the plan cannot survive the new bounds.
    pub fn set_side_bounds(&mut self, start_frame: u64, end_frame: u64) -> Result<(), RipError> {
        self.require_stopped("set side bounds")?;
        let recorded = self.manifest.recorded_frames;
        let end = end_frame.min(recorded);
        plan::validate_boundaries(
            &self.manifest.boundaries_frames,
            start_frame,
            end,
            self.cfg.sample_rate,
            MIN_SEGMENT_SECS,
        )?;
        // Store only a real trim, so an untrimmed side round-trips
        // through the manifest as `None` rather than as its own length.
        self.manifest.side_start_frame = (start_frame > 0).then_some(start_frame);
        self.manifest.side_end_frame = (end < recorded).then_some(end);
        manifest::save(&self.cfg.session_dir, &self.manifest)?;
        Ok(())
    }

    /// Set the split boundaries (frames where the next track starts).
    /// Only valid once stopped. Re-splitting preserves per-segment
    /// metadata by index. Persists the manifest.
    ///
    /// # Errors
    ///
    /// [`RipError::InvalidState`] unless stopped; [`RipError::Split`]
    /// when the boundaries are not a valid plan for this side.
    pub fn set_splits(&mut self, boundaries_frames: Vec<u64>) -> Result<(), RipError> {
        self.require_stopped("set splits")?;
        plan::validate_boundaries(
            &boundaries_frames,
            self.manifest.side_start(),
            self.manifest.side_end(),
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
        self.commit_with_progress(library, &mut |_| {})
    }

    /// [`Self::commit`], reporting each segment as it starts and
    /// finishes so a caller can show honest progress instead of one
    /// long stall.
    ///
    /// # Errors
    ///
    /// Same as [`Self::commit`].
    pub fn commit_with_progress(
        &mut self,
        library: &mut dub_library::Library,
        progress: &mut dyn FnMut(crate::CommitProgress),
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
            progress,
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
            REASON_SILENCE => StopReason::Silence,
            REASON_RECOVERED => StopReason::Recovered,
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
