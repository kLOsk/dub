//! Auto-analysis pipeline (M11c.1).
//!
//! Wires `dub-bpm::analyze_beat_grid` into the library's
//! `track_beatgrids` + `analysis_cache` tables. The pipeline is:
//!
//! 1. Caller resolves a `track_id` to a primary file path (the
//!    library's existing [`Library::resolve_track_path`] does this).
//! 2. [`analyze_track`] decodes the file via `dub-io`, runs
//!    `dub-bpm::analyze_beat_grid`, and upserts the result.
//!
//! ## Lifecycle (PRD §8.4)
//!
//! Analysis is **lazy**. We never run it during filesystem import
//! (M11c) — that path stays fast (~seconds for a 50 GB folder of
//! MP3s, dominated by Chromaprint). Instead, the Apple shell calls
//! [`analyze_track`] from two places:
//!
//! * `WaveformAppModel.loadTrack` — when a library track is loaded
//!   onto a deck and `is_track_analyzed` returns `false`. Runs in a
//!   `Task.detached(priority: .background)` so the deck plays
//!   immediately and the BPM badge fills in within ~1.5–3 s.
//! * `LibraryView` right-click → "Analyze Selected" / "Re-analyze
//!   Selected" — batch path with progress in the browser footer.
//!
//! ## Idempotency + cross-validation
//!
//! `analyze_track` upserts `track_beatgrids` keyed on
//! `(track_id, 'auto')`. Re-running it on a track that already has
//! an auto row refreshes the row's `captured_at` and recomputes
//! anchor + BPM (which can change as the algorithm evolves — the
//! result is keyed to the binary, not pinned to a historical run).
//!
//! The `is_active` decision honours PRD §8.3's priority order
//! (`serato > rekordbox > traktor > auto`): if any other-source row
//! already has `is_active = 1`, the auto row is written
//! `is_active = 0`. If no other-source row is active, the auto row
//! becomes the active grid. The partial unique index
//! `idx_one_active_grid_per_track` makes the conflict path a clean
//! database-level rejection rather than a TOCTOU race.
//!
//! ## `analysis_cache` semantics
//!
//! Every call writes `analysis_cache.analyzed_at = now` for the
//! track's fingerprint id, regardless of whether the BPM analyzer
//! found a grid. This is the source-of-truth for the
//! "is_track_analyzed" predicate the browser uses to dim
//! unanalyzed rows; once we've *tried*, the row leaves the dim
//! state, even if the verdict was "silence / non-musical input,
//! no grid". Trying again is always allowed (right-click →
//! Re-analyze) but won't fire automatically.

use std::path::PathBuf;

use rusqlite::params;
use rusqlite::OptionalExtension;

use crate::db::Library;
use crate::error::{LibraryError, Result};

/// Read-only projection of the active row in `track_beatgrids` for
/// a single track.
///
/// Returned by [`Library::active_beatgrid_for_track`]. Carries only
/// the columns a caller needs to install the grid on a deck: the
/// source (so the FFI / UI can decide whether to badge it as
/// "imported" vs "auto" vs "user_tap"), the tempo, the first-beat
/// anchor in seconds, and the unix timestamp the row was captured
/// at (lets the caller decide between two competing rows on the
/// rare cross-deck race; the partial unique index already keeps
/// this to at most one row per track, but the field is cheap to
/// surface and useful for diagnostics).
///
/// Per-beat positions are *not* stored: the auto pipeline (M11c.1)
/// and the v1.0 importers all produce fixed-tempo grids, which are
/// fully described by `(bpm, anchor_secs)`. Callers that want a
/// `Vec<f64>` of beat timestamps synthesize it as
/// `anchor + i · 60 / bpm` clamped to the track's duration. A
/// future tempo-drifting grid format (M10.5p-grid in the PRD)
/// would need a schema change to add a per-beat positions column;
/// this struct would grow a parallel `beats: Vec<f64>` field at
/// that point.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveBeatgrid {
    /// Source enum string — `"serato"`, `"traktor"`, `"rekordbox"`,
    /// `"itunes"`, `"auto"`, or `"user_tap"`. Matches the
    /// CHECK constraint on `track_beatgrids.source`.
    pub source: String,
    /// Tempo in beats per minute. Always finite and > 0 for active
    /// rows (the v1.0 auto pipeline refuses to write a row with
    /// `bpm <= 0`).
    pub bpm: f64,
    /// First-beat offset in seconds from sample 0 of the track.
    /// The auto pipeline stores `grid.beats.first()` here; importers
    /// translate their source's anchor convention into the same
    /// "seconds from start" frame.
    pub anchor_secs: f64,
    /// Unix seconds (UTC) at which the row was upserted.
    pub captured_at: i64,
    /// M11d.7: per-track lock from `tracks.grid_locked`.
    pub grid_locked: bool,
    /// M11d.7: drift slope (ms/min) from `tracks.grid_drift_quality`.
    pub grid_drift_quality: Option<f32>,
    /// PRD-BEATS §4.5 / C1 round 4 — absolute path to the
    /// `.wf` waveform sidecar for this track's fingerprint, if the
    /// last [`Self::analyze_track`] pass managed to render and
    /// persist one. `None` when no sidecar has been written yet
    /// (cold cache) or the sidecar row predates the C1 column.
    /// The engine consults this path in
    /// `background_analyze_and_install` and short-circuits the
    /// ~100–300 ms `compute_offline_peaks` pass on a hit, which is
    /// the "instant waveform after analyze" contract.
    pub waveform_sidecar_path: Option<String>,
    /// PRD-BEATS C2 (round 4) — explicit bar-phase scalar
    /// persisted alongside `(bpm, anchor_secs)`. `bar_phase ∈
    /// [0, beats_per_bar)` is the index `i` such that
    /// `beats[i]`, `beats[i + beats_per_bar]`, … are downbeats.
    /// All v1 rows store `0`; user set-the-1 edits via
    /// [`crate::Library::upsert_user_tap_beatgrid`] +
    /// `DubEngine::set_bar_phase` rotate this scalar without
    /// touching `bpm` or `anchor_secs`.
    pub bar_phase: u8,
}

/// Outcome of a single [`analyze_track`] call. Carries both the
/// beat-grid and the key results from one analysis pass — the
/// pipeline decodes the file exactly once and feeds the samples
/// to both DSPs (`dub_bpm::analyze_beat_grid` and
/// `dub_spectral::analyze_key`) before returning. The FFI mirrors
/// this verbatim so the LibraryView can refresh BPM + Key badges
/// on the affected row without re-querying the whole listing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisOutcome {
    /// Auto-detected tempo. Meaningful iff `bpm_confidence > 0.0`.
    pub bpm: f64,
    /// First-beat offset in seconds from sample 0. Meaningful iff
    /// `bpm_confidence > 0.0`.
    pub anchor_secs: f64,
    /// `dub-bpm` confidence in `[0.0, 1.0]`. `0.0` means no
    /// periodic structure detected (silence, non-musical input,
    /// too-short audio); the auto-grid row is then not written.
    pub bpm_confidence: f32,
    /// `true` iff the auto-grid row was made `is_active = 1`.
    /// Honours PRD §8.3 priority: if another source was already
    /// active, the auto row is recorded but `is_active = 0`.
    pub grid_auto_is_active: bool,
    /// `true` iff a beat-grid row was written (confidence > 0).
    /// `false` for silence / non-musical input — the track is
    /// still marked `is_analyzed` (no auto-retry) but carries no
    /// BPM.
    pub wrote_grid: bool,

    /// Auto-detected key in canonical Camelot notation (e.g.
    /// `"8B"` for C major). Empty string for the no-key outcome
    /// (silence / pure-noise input, < 5 s of audio, degenerate
    /// chroma).
    pub camelot: &'static str,
    /// Tonic pitch class (0 = C, 1 = C♯, …, 11 = B). Meaningful
    /// iff `key_confidence > 0.0`.
    pub tonic_pc: u8,
    /// `true` for a major key, `false` for minor. Meaningful iff
    /// `key_confidence > 0.0`.
    pub is_major: bool,
    /// `dub-spectral::analyze_key` confidence in `[0.0, 1.0]`.
    /// `0.0` means "no key detected"; the auto-key row is then
    /// not written.
    pub key_confidence: f32,
    /// `true` iff the auto-key row was made `is_active = 1`.
    /// Same §8.3-priority semantics as the beat grid.
    pub key_auto_is_active: bool,
    /// `true` iff a key row was written (confidence > 0). `false`
    /// for non-musical input.
    pub wrote_key: bool,
}

impl AnalysisOutcome {
    /// Default empty outcome. Returned in pathological cases where
    /// neither the BPM nor the key analyser found anything; the
    /// `analysis_cache.analyzed_at` stamp is still refreshed so
    /// the track exits the dim / re-analyze loop.
    const EMPTY: Self = Self {
        bpm: 0.0,
        anchor_secs: 0.0,
        bpm_confidence: 0.0,
        grid_auto_is_active: false,
        wrote_grid: false,
        camelot: "",
        tonic_pc: 0,
        is_major: true,
        key_confidence: 0.0,
        key_auto_is_active: false,
        wrote_key: false,
    };
}

impl Library {
    /// Run the M11c.1 auto-analysis pipeline against the file
    /// currently registered as the primary file for `track_id`.
    ///
    /// Returns `AnalysisOutcome` describing what was written. The
    /// `analysis_cache.analyzed_at` stamp is **always** refreshed —
    /// the "we tried, found nothing" case still flips the
    /// `is_track_analyzed` predicate to `true`, so the browser
    /// stops dimming the row and the deck-load hook stops retrying
    /// on every load. The user can force a re-run via the Re-analyze
    /// affordance.
    ///
    /// # Errors
    ///
    /// * [`LibraryError::TrackNotFound`] — `track_id` is not in
    ///   `tracks`.
    /// * [`LibraryError::TrackHasNoFile`] — the track has no
    ///   resolvable on-disk path. Caller (the Swift scanner) should
    ///   surface this as "file is missing"; the next Relocate run
    ///   re-attaches it and analysis can be retried.
    /// * [`LibraryError::DecodeFailed`] — `dub-io` couldn't decode
    ///   the file (corrupt / unsupported codec).
    /// * [`LibraryError::Sqlite`] — any underlying database error.
    ///
    /// # M11c.4 lazy fingerprint
    ///
    /// As of M11c.4 the importer leaves `tracks.fingerprint_id =
    /// NULL` so the import phase stays metadata-only. When this
    /// method sees a NULL fingerprint, it computes the Chromaprint
    /// over the just-decoded samples, upserts a `fingerprints` row,
    /// and writes the id back into `tracks` via
    /// [`Library::attach_fingerprint`] **before** stamping
    /// `analysis_cache` (which is keyed by fingerprint id). The
    /// fingerprint pass costs ~30 % of the analysis budget on top
    /// of the decode + BPM + key passes the user already paid for
    /// by loading the deck.
    pub fn analyze_track(&self, track_id: &str) -> Result<AnalysisOutcome> {
        // PRD-BEATS §3.5 "lock is absolute": if the grid is locked
        // we refuse the whole analysis pass rather than skipping
        // just the beat-grid step (the previous `force` parameter
        // dropped the lock and rebuilt the grid; round 3 removes
        // that escape hatch). Locking is a user contract, not a
        // performance gate, so we surface the refusal as
        // `LibraryError::GridLocked` and let the Apple shell turn
        // it into a no-op (the menu item that would call us is
        // already greyed out when the grid is locked, so seeing
        // this error in practice means a race or a tool calling
        // us directly).
        if self.is_grid_locked(track_id)? {
            return Err(LibraryError::GridLocked {
                track_id: track_id.to_string(),
            });
        }
        let (existing_fingerprint_id, file_path) = self.track_analysis_keys(track_id)?;
        let track =
            dub_io::Track::load_from_path(&file_path).map_err(|e| LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: file_path.clone(),
                reason: format!("{e}"),
            })?;

        // ---- Fingerprint (M11c.4 lazy attach) --------------------
        // If the importer left `fingerprint_id = NULL`, compute the
        // Chromaprint now and write it back to `tracks` (and
        // `fingerprints`). The race window (two concurrent
        // analyse_track calls on the same UUID) is closed by
        // `attach_fingerprint`'s `WHERE fingerprint_id IS NULL`
        // guard: the loser's UPDATE matches zero rows and we read
        // the winner's id back from `tracks`.
        let fingerprint_id = match existing_fingerprint_id {
            Some(id) => id,
            None => {
                let fp = dub_fingerprint::Fingerprint::compute_from_f32(
                    track.samples(),
                    track.sample_rate(),
                    u32::from(track.channels()),
                )
                .map_err(|e| LibraryError::DecodeFailed {
                    track_id: track_id.to_string(),
                    path: file_path.clone(),
                    reason: format!("fingerprint failed: {e}"),
                })?;
                let new_fp_id = self.upsert_fingerprint(
                    &fp,
                    Some(track.sample_rate()),
                    Some(u32::from(track.channels())),
                    None,
                )?;
                let attached = self.attach_fingerprint(track_id, new_fp_id, fp.duration_ms())?;
                if attached {
                    new_fp_id
                } else {
                    // A concurrent caller won the race. Re-read the
                    // winning id from `tracks`. (Cheaper than
                    // re-computing and the `fingerprints` row we
                    // just inserted becomes an orphan, which the
                    // future "Find duplicates" tool / a v1.x
                    // sweeper can garbage-collect.)
                    self.connection()
                        .query_row(
                            "SELECT fingerprint_id FROM tracks WHERE id = ?1",
                            params![track_id],
                            |r| r.get::<_, Option<i64>>(0),
                        )
                        .map_err(|e| LibraryError::sqlite("reread_fingerprint_after_race", e))?
                        .ok_or_else(|| LibraryError::TrackHasNoFingerprint {
                            track_id: track_id.to_string(),
                        })?
                }
            }
        };

        // ---- Beat grid (M11c.1, contract M11d.7 round 3) --------
        // PRD-BEATS §3.5 lock-is-absolute already gated us above:
        // by the time we get here `grid_locked == false` and the
        // user has explicitly asked for analysis. Any prior
        // `user_tap` row is demoted unconditionally — the new
        // auto run replaces it. This is the "re-analyze is a
        // pure reset" contract from PRD-BEATS §4 (user actions
        // table, "Re-analyze" row) and §4.6 idempotence: we never
        // carry tap-derived BPM forward into the next auto pass.
        let mut outcome = AnalysisOutcome::EMPTY;
        let demoted = self.deactivate_user_tap_beatgrid(track_id)?;
        if demoted > 0 {
            eprintln!(
                "dub-library: reanalyze demoted {demoted} active user_tap \
                 row(s) on unlocked track {track_id} — auto grid will \
                 claim is_active=1"
            );
        }
        // Genre is a pure octave-profile hint (M11c.3d). A failure
        // to read it (missing `track_metadata_source` row, NULL
        // column, or a SQLite error) must NOT abort analysis —
        // the worst that happens with no genre is we fall back to
        // `OctaveProfile::Default`, which is the profile used for
        // every track that never had an ID3 tag to begin with.
        // Pre-fix this branch could surface a `LibraryError::sqlite
        // (analyze_lookup_genre, …)` for any weirdness in the
        // metadata table and the user saw "Analysis failed for
        // track: …" instead of a successful analysis (PRD-BEATS
        // §4.4 contract: re-analyze is a pure reset, must succeed
        // on any track that decodes).
        let profile = match self.track_id3_genre(track_id) {
            Ok(g) => g
                .as_deref()
                .map(dub_bpm::octave_profile_from_genre)
                .unwrap_or(dub_bpm::OctaveProfile::Default),
            Err(e) => {
                eprintln!(
                    "dub-library: genre lookup failed for {track_id} ({e}); \
                     falling back to OctaveProfile::Default"
                );
                dub_bpm::OctaveProfile::Default
            }
        };
        let grid = dub_bpm::analyze_beat_grid_with_profile(
            track.samples(),
            track.sample_rate(),
            track.channels(),
            profile,
        )
        .map_err(|e| LibraryError::DecodeFailed {
            track_id: track_id.to_string(),
            path: file_path.clone(),
            reason: format!("beat-grid analysis failed: {e}"),
        })?;

        if grid.confidence > 0.0 {
            let anchor_secs = grid.beats.first().copied().unwrap_or(0.0);
            let other_grid_active = self.has_non_auto_active_grid(track_id)?;
            let grid_auto_is_active = !other_grid_active;
            self.upsert_auto_beatgrid(
                track_id,
                anchor_secs,
                grid.bpm,
                grid.bar_phase,
                grid_auto_is_active,
            )?;
            outcome.bpm = grid.bpm;
            outcome.anchor_secs = anchor_secs;
            outcome.bpm_confidence = grid.confidence;
            outcome.grid_auto_is_active = grid_auto_is_active;
            outcome.wrote_grid = true;
            if let Some(quality) = grid.quality.as_ref() {
                // Auto-lock disabled (user feedback: silent
                // freezes after an auto-pass were hostile — locks
                // now happen only when the user explicitly toggles
                // them via the BPM right-click menu or the library
                // row context menu). Persist the drift slope so
                // the "⚠" indicator still appears on suspect
                // grids, but don't touch `grid_locked`.
                self.set_grid_drift_quality(track_id, Some(quality.drift_slope_ms_per_min))?;
            }
        }

        // ---- Key (M11c.2) ----------------------------------------
        let key = dub_spectral::analyze_key(track.samples(), track.sample_rate(), track.channels())
            .map_err(|e| LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: file_path.clone(),
                reason: format!("key analysis failed: {e}"),
            })?;

        if key.confidence > 0.0 {
            let camelot = key.camelot();
            let other_key_active = self.has_non_auto_active_key(track_id)?;
            let key_auto_is_active = !other_key_active;
            self.upsert_auto_key(track_id, camelot, key.confidence, key_auto_is_active)?;
            outcome.camelot = camelot;
            outcome.tonic_pc = key.tonic_pc;
            outcome.is_major = key.is_major;
            outcome.key_confidence = key.confidence;
            outcome.key_auto_is_active = key_auto_is_active;
            outcome.wrote_key = true;
        }

        // ---- Waveform sidecar (PRD-BEATS C1, round 4) ------------
        // Pre-render the broadband / band / onset / filtered peak
        // streams the engine will need when a deck loads this
        // track, and persist them to
        // `~/Library/Caches/Dub/waveforms/{fingerprint_id}.wf`.
        // The engine's `background_analyze_and_install` consults
        // the same path on every load and short-circuits the
        // 100–300 ms `compute_offline_peaks` pass on hit, which is
        // exactly the "instant waveform" contract from PRD-BEATS
        // §4.5. Best-effort: any failure (cache dir unwritable,
        // disk full, `OfflinePeaksError` on a zero-length track)
        // is logged and analysis still returns success — BPM + key
        // are the contract here; the sidecar is a perf cache.
        let sidecar_path = self.write_waveform_sidecar(fingerprint_id, &track);

        // ---- Loudness / auto-gain (store-only) -------------------
        // Measure integrated LUFS (BS.1770-4) + sample peak in this
        // same decode pass and persist them to `analysis_cache`. The
        // value is **store-only**: it is consumed the *next* time the
        // track is loaded (`dub-ffi::load_track` reads it back via
        // [`Self::track_normalization_gain`] and applies the derived
        // gain once at load time). It is deliberately never pushed to
        // a deck that is already playing this track — retroactively
        // jumping the level of a tune live in front of an audience is
        // unacceptable, so a track analysed while it plays is
        // normalized only on its subsequent loads.
        let loudness = dub_dsp::measure_integrated_loudness(
            track.samples(),
            track.sample_rate(),
            u16::from(track.channels()),
        );
        self.stamp_loudness(fingerprint_id, loudness.lufs_i, loudness.sample_peak_dbfs)?;

        // Stamp `analysis_cache` exactly once. `has_active_grid` and
        // `has_active_key` reflect whether the auto pass landed the
        // active row (so they can be `1` here but get flipped to `0`
        // later if an importer claims the active slot — that's
        // tracked separately by `stamp_analysis_cache_after_import`
        // when those importers land in M11e). `sidecar_path` and
        // `has_waveform` mirror the C1 sidecar write outcome above.
        self.stamp_analysis_cache(
            fingerprint_id,
            outcome.grid_auto_is_active,
            outcome.key_auto_is_active,
            sidecar_path.as_deref(),
        )?;

        Ok(outcome)
    }

    /// Compute and persist the waveform sidecar for `fingerprint_id`
    /// from a freshly-decoded `track`. Returns the absolute path on
    /// success so the caller can stamp `analysis_cache`.
    ///
    /// **Best-effort.** All errors (peaks compute failure, missing
    /// cache dir, disk full) are logged and the function returns
    /// `None` — BPM + key analysis is the contract of
    /// [`Self::analyze_track`]; the sidecar is a perf cache and a
    /// missed write degrades to a slow first load, not a broken
    /// one. The engine treats a missing sidecar as a cache miss
    /// and recomputes on demand.
    fn write_waveform_sidecar(&self, fingerprint_id: i64, track: &dub_io::Track) -> Option<String> {
        let peaks = match dub_peaks::compute_offline_peaks(
            track.samples(),
            track.sample_rate(),
            track.channels(),
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "dub-library: skipping waveform sidecar for fingerprint \
                     {fingerprint_id} (peaks compute failed: {e})"
                );
                return None;
            }
        };
        let path = match self.waveforms_cache_dir() {
            Ok(dir) => dir.join(format!("{fingerprint_id}.wf")),
            Err(e) => {
                eprintln!(
                    "dub-library: skipping waveform sidecar for fingerprint \
                     {fingerprint_id} (cache dir unavailable: {e})"
                );
                return None;
            }
        };
        if let Err(e) = dub_peaks::write_sidecar(&path, &peaks) {
            eprintln!(
                "dub-library: failed to write waveform sidecar for fingerprint \
                 {fingerprint_id} at {}: {e}",
                path.display()
            );
            return None;
        }
        path.to_str().map(str::to_string)
    }

    /// Fetch the active beat-grid row for `track_id`, if any.
    ///
    /// Returns `None` when the track has no `is_active = 1` row in
    /// `track_beatgrids` yet (unanalyzed, or analyzed-but-silent so
    /// nothing was written). Returns `Some(ActiveBeatgrid)` with
    /// `(source, bpm, anchor_secs, captured_at)` from the active
    /// row when one exists. Always at most one row by virtue of the
    /// `idx_one_active_grid_per_track` partial unique index on
    /// `track_beatgrids(track_id) WHERE is_active = 1`.
    ///
    /// **Single source of truth for the deck's grid (M11d.5
    /// round 4).** `dub-ffi::DubEngine::load_track` calls this via
    /// the `DubLibrary::active_beat_grid` FFI shim before kicking
    /// the background analysis worker. When it returns `Some`, the
    /// engine installs the row's `(bpm, anchor_secs)` directly
    /// (synthesizing the per-beat positions vector from the
    /// fixed-tempo formula `anchor + i · 60 / bpm`) and the worker
    /// skips `dub_bpm::analyze_beat_grid` entirely. This kills:
    ///
    /// * The ~100–400 ms re-analysis on every load of an
    ///   already-analyzed track.
    /// * The ±0.02 BPM consistency drift between the DeckHeader
    ///   (which used to read the engine's in-memory grid) and the
    ///   LibraryView (which reads `track_beatgrids`).
    /// * The 30 Hz `beatGrid` FFI poll the Apple renderer used to
    ///   fire indefinitely for tracks whose Stage-1 estimator
    ///   legitimately returned zero — the load handshake now
    ///   delivers the final grid before the first Metal frame, so
    ///   the renderer's `confidence > 0` latch fires immediately
    ///   (closes UI-BACKLOG B-25).
    ///
    /// Read-only; safe to call from any callsite that wants to know
    /// the canonical grid for a track without running analysis.
    pub fn active_beatgrid_for_track(&self, track_id: &str) -> Result<Option<ActiveBeatgrid>> {
        let row = self.connection().query_row(
            "SELECT bg.source, bg.bpm, bg.anchor_secs, bg.captured_at, \
                    COALESCE(t.grid_locked, 0), t.grid_drift_quality, \
                    ac.waveform_sidecar_path, COALESCE(bg.bar_phase, 0) \
             FROM track_beatgrids bg \
             JOIN tracks t ON t.id = bg.track_id \
             LEFT JOIN analysis_cache ac ON ac.fingerprint_id = t.fingerprint_id \
             WHERE bg.track_id = ?1 AND bg.is_active = 1",
            params![track_id],
            |r| {
                let phase: i64 = r.get(7)?;
                Ok(ActiveBeatgrid {
                    source: r.get::<_, String>(0)?,
                    bpm: r.get::<_, f64>(1)?,
                    anchor_secs: r.get::<_, f64>(2)?,
                    captured_at: r.get::<_, i64>(3)?,
                    grid_locked: r.get::<_, i64>(4)? != 0,
                    grid_drift_quality: r.get(5)?,
                    waveform_sidecar_path: r.get::<_, Option<String>>(6)?,
                    bar_phase: u8::try_from(phase).unwrap_or(0),
                })
            },
        );
        match row {
            Ok(g) => Ok(Some(g)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibraryError::sqlite("active_beatgrid_for_track", e)),
        }
    }

    /// `true` once [`analyze_track`] has been called for this track
    /// (regardless of whether it found a grid). Backs the browser's
    /// dim / full-opacity visual cue.
    ///
    /// Implemented against `analysis_cache.analyzed_at` rather than
    /// `EXISTS track_beatgrids(track_id)` because the former
    /// correctly captures "we tried and found nothing" — a track
    /// that legitimately has no detectable grid (silence stem,
    /// ambient piece) should not stay dim forever.
    pub fn is_track_analyzed(&self, track_id: &str) -> Result<bool> {
        let analyzed: Option<i64> = self
            .connection()
            .query_row(
                "SELECT ac.analyzed_at \
                 FROM tracks t \
                 LEFT JOIN analysis_cache ac \
                        ON ac.fingerprint_id = t.fingerprint_id \
                 WHERE t.id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => rusqlite::Error::QueryReturnedNoRows,
                other => other,
            })
            .map_err(|e| LibraryError::sqlite("is_track_analyzed", e))?;
        Ok(analyzed.is_some())
    }

    /// M11d.7 — whether the track's beatgrid is frozen against
    /// auto re-analysis.
    pub fn is_grid_locked(&self, track_id: &str) -> Result<bool> {
        let locked: Option<i64> = self
            .connection()
            .query_row(
                "SELECT grid_locked FROM tracks WHERE id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("is_grid_locked", e))?;
        Ok(locked.is_some_and(|v| v != 0))
    }

    /// M11d.7 — user or auto-lock toggle.
    pub fn set_grid_locked(&self, track_id: &str, locked: bool) -> Result<()> {
        let n = self
            .connection()
            .execute(
                "UPDATE tracks SET grid_locked = ?2, updated_at = strftime('%s','now') \
                 WHERE id = ?1",
                params![track_id, if locked { 1 } else { 0 }],
            )
            .map_err(|e| LibraryError::sqlite("set_grid_locked", e))?;
        if n == 0 {
            return Err(LibraryError::TrackNotFound {
                track_id: track_id.to_string(),
            });
        }
        Ok(())
    }

    /// M11d.7 — stored LSQ drift slope for the ⚠ indicator.
    pub fn grid_drift_quality(&self, track_id: &str) -> Result<Option<f32>> {
        self.connection()
            .query_row(
                "SELECT grid_drift_quality FROM tracks WHERE id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("grid_drift_quality", e))
    }

    /// M11d.7 — persist drift slope after analysis or relatch.
    pub fn set_grid_drift_quality(&self, track_id: &str, drift: Option<f32>) -> Result<()> {
        let n = self
            .connection()
            .execute(
                "UPDATE tracks SET grid_drift_quality = ?2, updated_at = strftime('%s','now') \
                 WHERE id = ?1",
                params![track_id, drift],
            )
            .map_err(|e| LibraryError::sqlite("set_grid_drift_quality", e))?;
        if n == 0 {
            return Err(LibraryError::TrackNotFound {
                track_id: track_id.to_string(),
            });
        }
        Ok(())
    }

    /// M11d.7 — apply auto-lock thresholds from an LSQ fit.
    pub fn apply_grid_quality_lock(
        &self,
        track_id: &str,
        quality: &dub_bpm::GridQuality,
    ) -> Result<()> {
        self.set_grid_drift_quality(track_id, Some(quality.drift_slope_ms_per_min))?;
        self.set_grid_locked(track_id, quality.auto_lock_safe())
    }

    /// Resolve the `(fingerprint_id, absolute_path)` pair we need
    /// to run analysis. The fingerprint id is optional under the
    /// M11c.4 lazy-fingerprint model: a NULL value means the
    /// importer hasn't paid the Chromaprint cost yet, and
    /// `analyze_track` computes + attaches it inline. The path
    /// is mandatory — analysis without bytes to read is a
    /// `TrackHasNoFile`.
    fn track_analysis_keys(&self, track_id: &str) -> Result<(Option<i64>, std::path::PathBuf)> {
        let fingerprint_id: Option<i64> = self
            .connection()
            .query_row(
                "SELECT fingerprint_id FROM tracks WHERE id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => LibraryError::TrackNotFound {
                    track_id: track_id.to_string(),
                },
                other => LibraryError::sqlite("analyze_lookup_fingerprint", other),
            })?;
        let path =
            self.resolve_track_path(track_id)?
                .ok_or_else(|| LibraryError::TrackHasNoFile {
                    track_id: track_id.to_string(),
                })?;
        Ok((fingerprint_id, path))
    }

    /// ID3 genre tag for octave-profile selection (M11c.3d).
    fn track_id3_genre(&self, track_id: &str) -> Result<Option<String>> {
        let genre: Option<String> = self
            .connection()
            .query_row(
                "SELECT genre FROM track_metadata_source \
                 WHERE track_id = ?1 AND source = 'id3'",
                params![track_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("analyze_lookup_genre", e))?;
        Ok(genre.filter(|g| !g.trim().is_empty()))
    }

    /// `true` iff the track has an `is_active = 1` row in
    /// `track_beatgrids` from a source other than `auto`. Drives
    /// the §8.3 priority-honouring choice in `analyze_track`.
    fn has_non_auto_active_grid(&self, track_id: &str) -> Result<bool> {
        let n: i64 = self
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_beatgrids \
                 WHERE track_id = ?1 AND is_active = 1 AND source != 'auto'",
                params![track_id],
                |r| r.get(0),
            )
            .map_err(|e| LibraryError::sqlite("has_non_auto_active_grid", e))?;
        Ok(n > 0)
    }

    /// `true` iff the track has an `is_active = 1` row in
    /// `track_keys` from a source other than `auto`. Mirrors
    /// [`has_non_auto_active_grid`] for the M11c.2 key pipeline;
    /// same §8.3 priority semantics (Serato / Traktor / rekordbox
    /// / MixedInKey wins over auto when present).
    fn has_non_auto_active_key(&self, track_id: &str) -> Result<bool> {
        let n: i64 = self
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_keys \
                 WHERE track_id = ?1 AND is_active = 1 AND source != 'auto'",
                params![track_id],
                |r| r.get(0),
            )
            .map_err(|e| LibraryError::sqlite("has_non_auto_active_key", e))?;
        Ok(n > 0)
    }

    /// Demote any active `user_tap` beatgrid row for `track_id` to
    /// inactive. Called at the start of every unlocked beat-grid
    /// analysis pass (including force re-analyze after the lock is
    /// dropped) so the freshly-computed auto row can claim the
    /// single `is_active = 1` slot.
    ///
    /// **Why only `user_tap` and not all non-auto sources?** Imports
    /// (Serato / Traktor / rekordbox / iTunes) are external
    /// authorities the user pulled in deliberately, and PRD §8.3
    /// ranks them above both `auto` and `user_tap`. An unlocked
    /// re-analyse still honours an active import row via
    /// `has_non_auto_active_grid` (auto lands with `is_active = 0`).
    /// `user_tap` is Dub-authored and becomes stale the moment the
    /// analyser re-runs on the same buffer.
    ///
    /// Returns the number of rows touched (0 if there was no
    /// active user_tap to demote — useful for the debug log line
    /// in `analyze_track`).
    fn deactivate_user_tap_beatgrid(&self, track_id: &str) -> Result<usize> {
        self.connection()
            .execute(
                "UPDATE track_beatgrids SET is_active = 0 \
                 WHERE track_id = ?1 AND source = 'user_tap' AND is_active = 1",
                params![track_id],
            )
            .map_err(|e| LibraryError::sqlite("deactivate_user_tap_beatgrid", e))
    }

    /// Upsert the auto-source row in `track_beatgrids`. The
    /// `(track_id, source)` UNIQUE constraint makes this an
    /// idempotent refresh of the row's anchor / BPM / captured_at.
    fn upsert_auto_beatgrid(
        &self,
        track_id: &str,
        anchor_secs: f64,
        bpm: f64,
        bar_phase: u8,
        is_active: bool,
    ) -> Result<()> {
        self.connection()
            .execute(
                "INSERT INTO track_beatgrids \
                 (track_id, source, anchor_secs, bpm, bar_phase, is_active, captured_at) \
                 VALUES (?1, 'auto', ?2, ?3, ?4, ?5, strftime('%s','now')) \
                 ON CONFLICT(track_id, source) DO UPDATE SET \
                     anchor_secs = excluded.anchor_secs, \
                     bpm         = excluded.bpm, \
                     bar_phase   = excluded.bar_phase, \
                     is_active   = excluded.is_active, \
                     captured_at = excluded.captured_at",
                params![
                    track_id,
                    anchor_secs,
                    bpm,
                    i64::from(bar_phase),
                    if is_active { 1 } else { 0 },
                ],
            )
            .map_err(|e| LibraryError::sqlite("upsert_auto_beatgrid", e))?;
        Ok(())
    }

    /// Upsert a user tap-to-grid correction (M11c.3b). Deactivates
    /// every other grid row for the track, then writes
    /// `source = 'user_tap'` as the sole active grid.
    pub fn upsert_user_tap_beatgrid(
        &self,
        track_id: &str,
        anchor_secs: f64,
        bpm: f64,
        bar_phase: u8,
    ) -> Result<()> {
        if !bpm.is_finite() || bpm <= 0.0 {
            return Err(LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: PathBuf::new(),
                reason: format!("non-positive BPM ({bpm})"),
            });
        }
        if !anchor_secs.is_finite() {
            return Err(LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: PathBuf::new(),
                reason: format!("non-finite anchor ({anchor_secs})"),
            });
        }
        let conn = self.connection();
        conn.execute(
            "UPDATE track_beatgrids SET is_active = 0 WHERE track_id = ?1",
            params![track_id],
        )
        .map_err(|e| LibraryError::sqlite("upsert_user_tap_beatgrid_deactivate", e))?;
        conn.execute(
            "INSERT INTO track_beatgrids \
             (track_id, source, anchor_secs, bpm, bar_phase, is_active, captured_at) \
             VALUES (?1, 'user_tap', ?2, ?3, ?4, 1, strftime('%s','now')) \
             ON CONFLICT(track_id, source) DO UPDATE SET \
                 anchor_secs = excluded.anchor_secs, \
                 bpm         = excluded.bpm, \
                 bar_phase   = excluded.bar_phase, \
                 is_active   = 1, \
                 captured_at = excluded.captured_at",
            params![track_id, anchor_secs, bpm, i64::from(bar_phase)],
        )
        .map_err(|e| LibraryError::sqlite("upsert_user_tap_beatgrid", e))?;
        if let Ok((Some(fp_id), _path)) = self.track_analysis_keys(track_id) {
            let has_active_key: bool = conn
                .query_row(
                    "SELECT COALESCE(has_active_key, 0) FROM analysis_cache WHERE fingerprint_id = ?1",
                    params![fp_id],
                    |r| r.get::<_, i32>(0),
                )
                .map(|v| v != 0)
                .unwrap_or(false);
            self.stamp_analysis_cache(fp_id, true, has_active_key, None)?;
        }
        Ok(())
    }

    /// Octave-shift the currently active beatgrid (BPM × multiplier)
    /// while keeping the visible downbeat anchored at the same
    /// musical position. Writes the result as the sole active
    /// `user_tap` row so the change persists across library reload
    /// and so a subsequent re-analyse demotes it (PRD-BEATS §4
    /// "Re-analyze" semantics).
    ///
    /// Used by the deck-header BPM context menu's "2×" and "½"
    /// entries. The downbeat time is recovered from the active
    /// row's `(anchor_secs, bar_phase, bpm)` and re-projected onto
    /// the new period so the user sees twice as many (or half as
    /// many) beat ticks landing on the same kick.
    ///
    /// Returns the new `ActiveBeatgrid` (always `Some` on success
    /// because the upsert sets `is_active = 1`).
    pub fn scale_active_beatgrid(
        &self,
        track_id: &str,
        multiplier: f64,
    ) -> Result<Option<ActiveBeatgrid>> {
        if !multiplier.is_finite() || multiplier <= 0.0 {
            return Err(LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: PathBuf::new(),
                reason: format!("invalid bpm multiplier ({multiplier})"),
            });
        }
        if self.is_grid_locked(track_id)? {
            return Err(LibraryError::GridLocked {
                track_id: track_id.to_string(),
            });
        }
        let Some(active) = self.active_beatgrid_for_track(track_id)? else {
            return Ok(None);
        };
        if !active.bpm.is_finite() || active.bpm <= 0.0 {
            return Err(LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: PathBuf::new(),
                reason: format!("active grid has non-positive bpm ({})", active.bpm),
            });
        }
        let new_bpm = active.bpm * multiplier;
        if !new_bpm.is_finite() || new_bpm <= 0.0 {
            return Err(LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: PathBuf::new(),
                reason: format!("scaled bpm not positive ({new_bpm})"),
            });
        }
        let old_period = 60.0 / active.bpm;
        let downbeat_secs = active.anchor_secs + f64::from(active.bar_phase) * old_period;
        let new_period = 60.0 / new_bpm;
        // Project the downbeat back into [0, new_period) for the
        // anchor; pick `bar_phase` so that
        // `new_anchor + bar_phase * new_period ≈ downbeat_secs`.
        let new_anchor = downbeat_secs - (downbeat_secs / new_period).floor() * new_period;
        let beats_per_bar: i64 = 4;
        let beats_to_downbeat = ((downbeat_secs - new_anchor) / new_period).round() as i64;
        let phase = beats_to_downbeat.rem_euclid(beats_per_bar);
        let new_bar_phase = u8::try_from(phase).unwrap_or(0);
        self.upsert_user_tap_beatgrid(track_id, new_anchor, new_bpm, new_bar_phase)?;
        self.active_beatgrid_for_track(track_id)
    }

    /// Revert the currently active beatgrid to the original auto
    /// analysis: demote any active `user_tap` row and reactivate
    /// the `auto` row for this track. Backs the deck-header BPM
    /// context menu's "Reset" entry.
    ///
    /// Returns `Ok(None)` when no `auto` row exists yet (the track
    /// has never been analysed). The caller should surface a hint
    /// asking the user to run analysis. The lock is honoured: a
    /// locked grid refuses the reset and returns
    /// [`LibraryError::GridLocked`].
    pub fn reset_active_beatgrid_to_auto(&self, track_id: &str) -> Result<Option<ActiveBeatgrid>> {
        if self.is_grid_locked(track_id)? {
            return Err(LibraryError::GridLocked {
                track_id: track_id.to_string(),
            });
        }
        let conn = self.connection();
        // Does an auto row exist? If not, there is nothing to
        // revert to. Leave the user_tap row alone in that case so
        // the deck doesn't end up gridless.
        let auto_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM track_beatgrids \
                 WHERE track_id = ?1 AND source = 'auto')",
                params![track_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n != 0)
            .map_err(|e| LibraryError::sqlite("reset_active_beatgrid_to_auto_check_auto", e))?;
        if !auto_exists {
            return Ok(None);
        }
        conn.execute(
            "UPDATE track_beatgrids SET is_active = 0 \
             WHERE track_id = ?1 AND source = 'user_tap'",
            params![track_id],
        )
        .map_err(|e| LibraryError::sqlite("reset_active_beatgrid_to_auto_demote_tap", e))?;
        conn.execute(
            "UPDATE track_beatgrids SET is_active = 1 \
             WHERE track_id = ?1 AND source = 'auto'",
            params![track_id],
        )
        .map_err(|e| LibraryError::sqlite("reset_active_beatgrid_to_auto_promote_auto", e))?;
        if let Ok((Some(fp_id), _path)) = self.track_analysis_keys(track_id) {
            let has_active_key: bool = conn
                .query_row(
                    "SELECT COALESCE(has_active_key, 0) FROM analysis_cache \
                     WHERE fingerprint_id = ?1",
                    params![fp_id],
                    |r| r.get::<_, i32>(0),
                )
                .map(|v| v != 0)
                .unwrap_or(false);
            self.stamp_analysis_cache(fp_id, true, has_active_key, None)?;
        }
        self.active_beatgrid_for_track(track_id)
    }

    /// Stamp `analysis_cache` for the track's fingerprint. Inserts
    /// the row if it didn't exist (first analysis), updates the
    /// `analyzed_at` + `has_active_grid` fields if it did
    /// (re-analysis). The `(fingerprint_id)` PRIMARY KEY makes the
    /// upsert clean.
    fn stamp_analysis_cache(
        &self,
        fingerprint_id: i64,
        has_active_grid: bool,
        has_active_key: bool,
        waveform_sidecar_path: Option<&str>,
    ) -> Result<()> {
        let has_waveform = waveform_sidecar_path.is_some();
        // PRD-BEATS C1: `waveform_sidecar_path` + `has_waveform` are
        // updated atomically with the rest of the cache row. We
        // preserve any previously-stamped path on a re-analyze pass
        // that fails to refresh the sidecar (best-effort write
        // dropping to `None` should not surprise a downstream
        // reader by nulling out a valid prior path) by using
        // `COALESCE(excluded.col, analysis_cache.col)` in the
        // upsert.
        self.connection()
            .execute(
                "INSERT INTO analysis_cache \
                 (fingerprint_id, has_active_grid, has_active_key, has_waveform, \
                  waveform_sidecar_path, analyzed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now')) \
                 ON CONFLICT(fingerprint_id) DO UPDATE SET \
                     has_active_grid       = excluded.has_active_grid, \
                     has_active_key        = excluded.has_active_key, \
                     has_waveform          = CASE \
                         WHEN excluded.has_waveform = 1 THEN 1 \
                         ELSE analysis_cache.has_waveform END, \
                     waveform_sidecar_path = COALESCE(excluded.waveform_sidecar_path, \
                                                     analysis_cache.waveform_sidecar_path), \
                     analyzed_at           = excluded.analyzed_at",
                params![
                    fingerprint_id,
                    if has_active_grid { 1 } else { 0 },
                    if has_active_key { 1 } else { 0 },
                    if has_waveform { 1 } else { 0 },
                    waveform_sidecar_path
                ],
            )
            .map_err(|e| LibraryError::sqlite("stamp_analysis_cache", e))?;
        Ok(())
    }

    /// Persist the loudness measurement for a fingerprint into
    /// `analysis_cache` (`lufs_i`, `true_peak_dbtp`, `has_lufs`).
    ///
    /// Kept **separate** from [`Self::stamp_analysis_cache`] on
    /// purpose: the grid/key/waveform stamp runs from tap-edit and
    /// reset paths that have no loudness to offer, and folding the
    /// loudness columns into that upsert's `SET` list would null them
    /// out on every such edit. This method touches only the three
    /// loudness columns, leaving the grid/key/waveform state intact,
    /// and is called from exactly one place — [`Self::analyze_track`],
    /// once per analysis pass.
    ///
    /// `lufs_i = None` (silence / too-short input) is recorded with
    /// `has_lufs = 0` so the load path falls back to unity gain. The
    /// sample peak is always stored (it is meaningful even for the
    /// no-LUFS case and bounds any future gain).
    fn stamp_loudness(
        &self,
        fingerprint_id: i64,
        lufs_i: Option<f64>,
        sample_peak_dbfs: f64,
    ) -> Result<()> {
        let has_lufs = i64::from(lufs_i.is_some());
        self.connection()
            .execute(
                "INSERT INTO analysis_cache \
                 (fingerprint_id, lufs_i, true_peak_dbtp, has_lufs, analyzed_at) \
                 VALUES (?1, ?2, ?3, ?4, strftime('%s','now')) \
                 ON CONFLICT(fingerprint_id) DO UPDATE SET \
                     lufs_i         = excluded.lufs_i, \
                     true_peak_dbtp = excluded.true_peak_dbtp, \
                     has_lufs       = excluded.has_lufs",
                params![fingerprint_id, lufs_i, sample_peak_dbfs, has_lufs],
            )
            .map_err(|e| LibraryError::sqlite("stamp_loudness", e))?;
        Ok(())
    }

    /// Linear deck gain that normalizes `track_id` toward the default
    /// loudness target, or `None` when no loudness has been measured
    /// for the track yet.
    ///
    /// This is the **read side of auto-gain**. The Apple shell calls
    /// it (via the `DubLibrary::track_normalization_gain` FFI shim)
    /// immediately before `DubEngine::load_track` and hands the result
    /// to the engine, which applies it to the deck once at load and
    /// holds it for the life of that load. `None` is returned when:
    ///
    /// * the track is unknown / has no fingerprint, or
    /// * `analysis_cache` has no row for it, or
    /// * `has_lufs = 0` (analysed but silent / too short).
    ///
    /// In every `None` case the caller applies unity gain — including
    /// the "freshly imported, never analysed" track, which therefore
    /// plays at unity the first time and is normalized on subsequent
    /// loads once background analysis has stored its LUFS.
    ///
    /// The gain is derived from the stored integrated LUFS and sample
    /// peak via `dub_dsp::normalization_gain_db` against
    /// [`dub_dsp::DEFAULT_TARGET_LUFS`] and [`dub_dsp::CEILING_DBFS`],
    /// so it is bounded and clip-safe.
    pub fn track_normalization_gain(&self, track_id: &str) -> Result<Option<f32>> {
        let fingerprint_id: Option<i64> = self
            .connection()
            .query_row(
                "SELECT fingerprint_id FROM tracks WHERE id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("track_normalization_gain_lookup_fp", e))?
            .flatten();
        let Some(fingerprint_id) = fingerprint_id else {
            return Ok(None);
        };
        let row: Option<(Option<f64>, Option<f64>, i64)> = self
            .connection()
            .query_row(
                "SELECT lufs_i, true_peak_dbtp, COALESCE(has_lufs, 0) \
                 FROM analysis_cache WHERE fingerprint_id = ?1",
                params![fingerprint_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("track_normalization_gain_lookup_cache", e))?;
        let Some((Some(lufs), peak, has_lufs)) = row else {
            return Ok(None);
        };
        if has_lufs == 0 {
            return Ok(None);
        }
        // A missing peak (legacy row) is treated as 0 dBFS, the most
        // conservative assumption for the clip ceiling.
        let peak = peak.unwrap_or(0.0);
        let gain_db = dub_dsp::normalization_gain_db(
            lufs,
            peak,
            dub_dsp::DEFAULT_TARGET_LUFS,
            dub_dsp::CEILING_DBFS,
        );
        Ok(Some(dub_dsp::db_to_linear(gain_db)))
    }

    /// Upsert the auto-source row in `track_keys`. Mirrors
    /// [`Self::upsert_auto_beatgrid`] for the M11c.2 key
    /// pipeline. `original_notation` is stored equal to the
    /// canonical Camelot string — the auto pass has no separate
    /// "original notation" because it only ever produces Camelot;
    /// importers (M11e+) fill `original_notation` with whatever
    /// the source wrote verbatim.
    fn upsert_auto_key(
        &self,
        track_id: &str,
        camelot: &str,
        confidence: f32,
        is_active: bool,
    ) -> Result<()> {
        self.connection()
            .execute(
                "INSERT INTO track_keys \
                 (track_id, source, key_notation, original_notation, confidence, is_active, captured_at) \
                 VALUES (?1, 'auto', ?2, ?2, ?3, ?4, strftime('%s','now')) \
                 ON CONFLICT(track_id, source) DO UPDATE SET \
                     key_notation      = excluded.key_notation, \
                     original_notation = excluded.original_notation, \
                     confidence        = excluded.confidence, \
                     is_active         = excluded.is_active, \
                     captured_at       = excluded.captured_at",
                params![
                    track_id,
                    camelot,
                    f64::from(confidence),
                    if is_active { 1 } else { 0 }
                ],
            )
            .map_err(|e| LibraryError::sqlite("upsert_auto_key", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Fingerprint;
    use std::f32::consts::PI;
    use std::io::Write;

    /// Generate a single-channel click track at `bpm` for `duration_secs`.
    /// One short click (a sin-window envelope around a 1 kHz pulse)
    /// per beat. The DSP locks onto integer BPM at this density;
    /// we use it to test that `analyze_track` round-trips a known
    /// BPM into `track_beatgrids`.
    fn write_click_track(path: &std::path::Path, bpm: f64, secs: f64) {
        let sample_rate = 44_100_u32;
        let total = (secs * sample_rate as f64) as usize;
        let mut samples = vec![0.0_f32; total];
        let beat_interval = (60.0 / bpm * sample_rate as f64) as usize;
        let click_len = (0.02 * sample_rate as f64) as usize;
        let mut t = 0usize;
        while t + click_len < total {
            for k in 0..click_len {
                let env = (PI * k as f32 / click_len as f32).sin().powi(2);
                let tone = (2.0 * PI * 1000.0 * (k as f32 / sample_rate as f32)).sin();
                samples[t + k] = 0.6 * env * tone;
            }
            t += beat_interval;
        }
        // Encode to WAV via hound (the dev-dep used by the importer tests).
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for s in samples {
            let pcm = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(pcm).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Seed a single canonical track with a real on-disk file so
    /// `analyze_track` can actually run end-to-end. Returns the
    /// canonical track UUID + the fingerprint id + the file path.
    fn seed_track_with_file(
        lib: &Library,
        tmp: &tempfile::TempDir,
        bpm: f64,
        secs: f64,
    ) -> (String, i64, std::path::PathBuf) {
        // Build a click track and write it to the tempdir. The
        // tempdir is on the boot volume, which the M11a volume
        // discovery code normalises to mount_point = "/".
        let filename = format!("click-{bpm}.wav");
        let path = tmp.path().join(&filename);
        write_click_track(&path, bpm, secs);

        // Register the boot volume so the track_files FK holds.
        let boot_uuid = "00000000-0000-0000-0000-000000000000";
        lib.connection()
            .execute(
                "INSERT INTO volumes (volume_uuid, display_name, last_known_mount_point, last_seen_at, is_internal) \
                 VALUES (?1, 'Macintosh HD', '/', strftime('%s','now'), 1) \
                 ON CONFLICT(volume_uuid) DO NOTHING",
                params![boot_uuid],
            )
            .unwrap();

        // Fingerprint the file we just wrote (real Chromaprint
        // blob, real duration) so the analysis_cache key is valid.
        let track = dub_io::Track::load_from_path(&path).unwrap();
        let fp = Fingerprint::compute_from_f32(
            track.samples(),
            track.sample_rate(),
            u32::from(track.channels()),
        )
        .unwrap();
        let fp_id = lib
            .upsert_fingerprint(
                &fp,
                Some(track.sample_rate()),
                Some(u32::from(track.channels())),
                None,
            )
            .unwrap();
        let track_id = uuid::Uuid::new_v4().to_string();
        lib.insert_track(&track_id, Some(fp_id), Some(fp.duration_ms()), None)
            .unwrap();
        // Strip the tempdir prefix to get the volume-relative path.
        let rel = path
            .strip_prefix("/")
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        lib.upsert_track_file(
            &track_id,
            boot_uuid,
            &rel,
            Some("wav"),
            Some(track.sample_rate()),
            None,
            Some(u32::from(track.channels())),
            None,
            None,
        )
        .unwrap();
        (track_id, fp_id, path)
    }

    /// M11c.4 helper: seed a `tracks` row with **no** fingerprint
    /// id and **no** duration_ms, mirroring the state the importer
    /// leaves behind under the lazy-fingerprint model. Returns the
    /// track UUID + file path so the test can call
    /// `analyze_track` and assert the attach side-effect.
    fn seed_track_without_fingerprint(
        lib: &Library,
        tmp: &tempfile::TempDir,
        bpm: f64,
        secs: f64,
    ) -> (String, std::path::PathBuf) {
        let filename = format!("click-{bpm}.wav");
        let path = tmp.path().join(&filename);
        write_click_track(&path, bpm, secs);

        let boot_uuid = "00000000-0000-0000-0000-000000000000";
        lib.connection()
            .execute(
                "INSERT INTO volumes (volume_uuid, display_name, last_known_mount_point, last_seen_at, is_internal) \
                 VALUES (?1, 'Macintosh HD', '/', strftime('%s','now'), 1) \
                 ON CONFLICT(volume_uuid) DO NOTHING",
                params![boot_uuid],
            )
            .unwrap();

        let track_id = uuid::Uuid::new_v4().to_string();
        lib.insert_track(&track_id, None, None, None).unwrap();
        let rel = path
            .strip_prefix("/")
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        lib.upsert_track_file(
            &track_id,
            boot_uuid,
            &rel,
            Some("wav"),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        (track_id, path)
    }

    #[test]
    fn analyze_track_attaches_fingerprint_when_missing() {
        // M11c.4: a fresh-from-importer track has fingerprint_id
        // NULL. `analyze_track` must compute the Chromaprint over
        // the just-decoded samples, write it to `fingerprints`,
        // and link the new id back into `tracks` so the
        // analysis_cache stamp can use it as a key.
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _) = seed_track_without_fingerprint(&lib, &tmp, 120.0, 8.0);

        // Pre-analysis: NULL fingerprint, NULL duration, 0 rows
        // in `fingerprints`.
        let (fp_id_before, dur_before): (Option<i64>, Option<i64>) = lib
            .connection()
            .query_row(
                "SELECT fingerprint_id, duration_ms FROM tracks WHERE id = ?1",
                params![track_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(fp_id_before.is_none(), "precondition: NULL fingerprint");
        assert!(dur_before.is_none(), "precondition: NULL duration");
        let fp_rows_before: i64 = lib
            .connection()
            .query_row("SELECT COUNT(*) FROM fingerprints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fp_rows_before, 0);

        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(outcome.wrote_grid, "click track must produce a grid");

        // Post-analysis: fingerprint attached, duration populated,
        // one row in `fingerprints`, analysis_cache stamped.
        let (fp_id_after, dur_after): (Option<i64>, Option<i64>) = lib
            .connection()
            .query_row(
                "SELECT fingerprint_id, duration_ms FROM tracks WHERE id = ?1",
                params![track_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(
            fp_id_after.is_some(),
            "analyze_track must attach a fingerprint"
        );
        assert!(
            dur_after.is_some_and(|d| d > 0),
            "analyze_track must populate duration"
        );
        let fp_rows_after: i64 = lib
            .connection()
            .query_row("SELECT COUNT(*) FROM fingerprints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fp_rows_after, 1);
        assert!(lib.is_track_analyzed(&track_id).unwrap());
    }

    #[test]
    fn analyze_track_is_idempotent_when_fingerprint_already_attached() {
        // Re-running analyze on a track that already has a
        // fingerprint id must NOT mint a second `fingerprints` row.
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _) = seed_track_without_fingerprint(&lib, &tmp, 120.0, 8.0);

        lib.analyze_track(&track_id).unwrap();
        lib.analyze_track(&track_id).unwrap();

        let fp_rows: i64 = lib
            .connection()
            .query_row("SELECT COUNT(*) FROM fingerprints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            fp_rows, 1,
            "second analyze must reuse the existing fingerprint id"
        );
    }

    #[test]
    fn loudness_round_trips_through_analyze_to_normalization_gain() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _fp_id, _path) = seed_track_with_file(&lib, &tmp, 120.0, 12.0);

        // Pre-analysis: no stored loudness → unity-fallback (None).
        assert!(
            lib.track_normalization_gain(&track_id).unwrap().is_none(),
            "an unanalyzed track must yield no auto-gain (caller uses unity)"
        );

        lib.analyze_track(&track_id).unwrap();

        // analysis_cache now carries a finite LUFS + sample peak.
        let (lufs, peak, has): (Option<f64>, Option<f64>, i64) = lib
            .connection()
            .query_row(
                "SELECT ac.lufs_i, ac.true_peak_dbtp, ac.has_lufs \
                 FROM tracks t JOIN analysis_cache ac \
                   ON ac.fingerprint_id = t.fingerprint_id \
                 WHERE t.id = ?1",
                params![track_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(has, 1, "click track must measure a loudness");
        assert!(lufs.is_some_and(f64::is_finite));
        assert!(peak.is_some_and(f64::is_finite));

        // The derived gain is finite, positive, and clip-safe: the
        // resulting sample peak stays at or under the ceiling.
        let gain = lib
            .track_normalization_gain(&track_id)
            .unwrap()
            .expect("analyzed track must yield an auto-gain");
        assert!(
            gain.is_finite() && gain > 0.0,
            "gain must be a real multiplier: {gain}"
        );
        let peak_after_db = 20.0 * f64::from(gain).log10() + peak.unwrap();
        assert!(
            peak_after_db <= dub_dsp::CEILING_DBFS + 1e-6,
            "normalized peak {peak_after_db} dBFS must respect the {} dBFS ceiling",
            dub_dsp::CEILING_DBFS
        );
    }

    #[test]
    fn normalization_gain_is_none_for_unknown_track() {
        let lib = Library::open_in_memory().unwrap();
        assert!(lib
            .track_normalization_gain("00000000-0000-0000-0000-000000000000")
            .unwrap()
            .is_none());
    }

    #[test]
    fn analyze_track_writes_auto_grid_and_stamps_cache() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, fp_id, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);

        // Pre-analysis: not analyzed, no grid, no cache row.
        assert!(!lib.is_track_analyzed(&track_id).unwrap());
        let pre_grid_count: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_beatgrids WHERE track_id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pre_grid_count, 0);

        // Run analysis. A 120 BPM click track is the easy case;
        // the algorithm must place a grid with non-zero confidence.
        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(
            outcome.wrote_grid,
            "expected a grid for a 120 BPM click track"
        );
        assert!(outcome.bpm_confidence > 0.0);
        assert!(
            outcome.grid_auto_is_active,
            "no other source active → auto becomes active"
        );
        // The estimator picks the lock-on octave; for a clean
        // 120 BPM click track this is 120 or 60 depending on the
        // window. Both are valid; assert "close to a multiple/divisor of 120".
        let bpm = outcome.bpm;
        let candidates = [60.0, 120.0, 240.0];
        assert!(
            candidates.iter().any(|c| (bpm - c).abs() < 1.5),
            "expected BPM near 60/120/240, got {bpm}"
        );

        // Post-analysis: analyzed=true, one active auto grid, one
        // analysis_cache row stamped with has_active_grid=1.
        assert!(lib.is_track_analyzed(&track_id).unwrap());
        let (rows, anchor, active, src): (i64, f64, i64, String) = lib
            .connection()
            .query_row(
                "SELECT COUNT(*), MAX(anchor_secs), MAX(is_active), MAX(source) \
                 FROM track_beatgrids WHERE track_id = ?1",
                params![track_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(src, "auto");
        assert_eq!(active, 1);
        assert!((0.0..2.0).contains(&anchor), "anchor in expected window");

        let cache_active: i64 = lib
            .connection()
            .query_row(
                "SELECT has_active_grid FROM analysis_cache WHERE fingerprint_id = ?1",
                params![fp_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cache_active, 1);
    }

    /// PRD-BEATS C1 (round 4) — `analyze_track` must render the
    /// waveform sidecar and stamp the path + `has_waveform = 1` so
    /// the engine can short-circuit `compute_offline_peaks` on the
    /// next deck load. End-to-end check: the sidecar file exists on
    /// disk, the cache row points to it, and the surfaced
    /// `ActiveBeatgrid` carries the same path.
    #[test]
    fn analyze_track_writes_waveform_sidecar_and_stamps_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("waveforms");
        std::fs::create_dir_all(&cache_dir).unwrap();
        // Canonicalise so `/var/folders/...` and `/private/var/folders/...`
        // (the macOS symlink pair) match cleanly against the stored
        // path the library writes (which is built from the override
        // value, not from canonicalising the tempdir).
        let cache_dir_canonical = cache_dir.canonicalize().unwrap_or(cache_dir.clone());
        let lib = Library::open_in_memory()
            .unwrap()
            .with_waveforms_cache_dir(cache_dir.clone());
        let (track_id, fp_id, _path) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(outcome.wrote_grid, "click track must produce a grid");

        // analysis_cache row: path populated, has_waveform flipped.
        let (sidecar_path_db, has_waveform): (Option<String>, i64) = lib
            .connection()
            .query_row(
                "SELECT waveform_sidecar_path, has_waveform \
                 FROM analysis_cache WHERE fingerprint_id = ?1",
                params![fp_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            has_waveform, 1,
            "analyze_track must flip has_waveform after writing the sidecar"
        );
        let sidecar_path_db =
            sidecar_path_db.expect("analyze_track must store the absolute sidecar path");
        let stored_canonical = std::path::Path::new(&sidecar_path_db)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(&sidecar_path_db));
        assert!(
            stored_canonical.starts_with(&cache_dir_canonical),
            "stored sidecar path {} (canonical {}) must live inside the env-overridden cache dir {}",
            sidecar_path_db,
            stored_canonical.display(),
            cache_dir_canonical.display()
        );

        // File actually exists on disk (the engine's read_sidecar
        // will hit it on the next load).
        let path_on_disk = std::path::Path::new(&sidecar_path_db);
        assert!(
            path_on_disk.exists(),
            "expected sidecar file at {sidecar_path_db}"
        );
        let metadata = std::fs::metadata(path_on_disk).unwrap();
        assert!(
            metadata.len() > 64,
            "sidecar must be larger than the header (got {} bytes)",
            metadata.len()
        );

        // ActiveBeatgrid surfaces the same path so the engine can
        // consult it without a second DB query.
        let active = lib.active_beatgrid_for_track(&track_id).unwrap().unwrap();
        assert_eq!(
            active.waveform_sidecar_path.as_deref(),
            Some(sidecar_path_db.as_str()),
            "active_beatgrid_for_track must surface the sidecar path"
        );

        // Round-trip: the file the library wrote must deserialise
        // cleanly back into `OfflinePeaks` via the engine's reader.
        let loaded = dub_peaks::read_sidecar(path_on_disk).unwrap();
        let peaks = loaded.expect("sidecar round-trips through read_sidecar");
        assert!(
            !peaks.broadband.is_empty() && !peaks.bands.is_empty(),
            "round-tripped sidecar must carry chunks"
        );
    }

    /// PRD-BEATS C1: a re-analyze pass must overwrite the sidecar
    /// (so the cached peaks track the latest analysis), keep the
    /// path stable (the file lives at `{fingerprint_id}.wf`), and
    /// leave `has_waveform = 1`. Mirror of
    /// `analyze_track_is_idempotent_on_re_run` for the sidecar
    /// state.
    #[test]
    fn re_analyze_overwrites_waveform_sidecar_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("waveforms");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let lib = Library::open_in_memory()
            .unwrap()
            .with_waveforms_cache_dir(cache_dir);
        let (track_id, fp_id, _path) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();
        let first_path: String = lib
            .connection()
            .query_row(
                "SELECT waveform_sidecar_path FROM analysis_cache WHERE fingerprint_id = ?1",
                params![fp_id],
                |r| r.get(0),
            )
            .unwrap();
        let first_mtime = std::fs::metadata(&first_path).unwrap().modified().unwrap();
        // Sleep a hair so mtime can advance even on filesystems
        // with second-resolution timestamps.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        lib.analyze_track(&track_id).unwrap();
        let second_path: String = lib
            .connection()
            .query_row(
                "SELECT waveform_sidecar_path FROM analysis_cache WHERE fingerprint_id = ?1",
                params![fp_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            first_path, second_path,
            "sidecar path is keyed by fingerprint id and must be stable across re-analyzes"
        );
        let second_mtime = std::fs::metadata(&second_path).unwrap().modified().unwrap();
        assert!(
            second_mtime >= first_mtime,
            "re-analyze must refresh the sidecar (mtime moved backwards: \
             first {first_mtime:?}, second {second_mtime:?})"
        );
    }

    #[test]
    fn analyze_track_is_idempotent_on_re_run() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 90.0, 8.0);

        let first = lib.analyze_track(&track_id).unwrap();
        let second = lib.analyze_track(&track_id).unwrap();

        // Same input → same outcome shape, single auto row.
        assert_eq!(first.wrote_grid, second.wrote_grid);
        let n: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_beatgrids WHERE track_id = ?1 AND source = 'auto'",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "re-analyze should upsert, not duplicate");
    }

    #[test]
    fn analyze_track_keeps_existing_active_grid_when_other_source_present() {
        // PRD §8.3 priority: serato > auto. If serato has an
        // is_active=1 row, the auto pass must record its own row
        // with is_active=0 (so the user can switch to it via the
        // active-grid context menu) but must not steal active.
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);

        // Pretend a Serato importer landed an authoritative grid.
        lib.connection()
            .execute(
                "INSERT INTO track_beatgrids \
                 (track_id, source, anchor_secs, bpm, is_active, captured_at) \
                 VALUES (?1, 'serato', 0.0, 92.0, 1, strftime('%s','now'))",
                params![track_id],
            )
            .unwrap();

        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(outcome.wrote_grid);
        assert!(
            !outcome.grid_auto_is_active,
            "auto must not steal active from a higher-priority source"
        );

        let (serato_active, auto_active): (i64, i64) = lib
            .connection()
            .query_row(
                "SELECT \
                    SUM(CASE WHEN source = 'serato' THEN is_active ELSE 0 END), \
                    SUM(CASE WHEN source = 'auto'   THEN is_active ELSE 0 END) \
                 FROM track_beatgrids WHERE track_id = ?1",
                params![track_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(serato_active, 1);
        assert_eq!(auto_active, 0);
    }

    #[test]
    fn analyze_track_on_silence_marks_analyzed_without_writing_grid() {
        // PRD §8.4 honesty contract: silence is "analyzed but no
        // grid". The row stops dimming in the browser; the BPM
        // column stays empty.
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();

        // Build a silent WAV directly (don't use the click-track
        // generator).
        let path = tmp.path().join("silence.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for _ in 0..(44_100 * 8) {
                w.write_sample(0i16).unwrap();
            }
            w.finalize().unwrap();
            // Force flush.
            let _ = std::io::stdout().flush();
        }

        // Wire the file into the library by hand.
        let boot_uuid = "00000000-0000-0000-0000-000000000000";
        lib.connection()
            .execute(
                "INSERT INTO volumes (volume_uuid, display_name, last_known_mount_point, last_seen_at, is_internal) \
                 VALUES (?1, 'Macintosh HD', '/', strftime('%s','now'), 1) \
                 ON CONFLICT(volume_uuid) DO NOTHING",
                params![boot_uuid],
            )
            .unwrap();
        let track = dub_io::Track::load_from_path(&path).unwrap();
        let fp = Fingerprint::compute_from_f32(
            track.samples(),
            track.sample_rate(),
            u32::from(track.channels()),
        )
        .unwrap();
        let fp_id = lib
            .upsert_fingerprint(
                &fp,
                Some(track.sample_rate()),
                Some(u32::from(track.channels())),
                None,
            )
            .unwrap();
        let track_id = uuid::Uuid::new_v4().to_string();
        lib.insert_track(&track_id, Some(fp_id), Some(fp.duration_ms()), None)
            .unwrap();
        let rel = path
            .strip_prefix("/")
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        lib.upsert_track_file(
            &track_id,
            boot_uuid,
            &rel,
            Some("wav"),
            Some(44_100),
            None,
            Some(1),
            None,
            None,
        )
        .unwrap();

        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(!outcome.wrote_grid, "silence should not produce a grid");
        assert_eq!(outcome.bpm_confidence, 0.0);
        assert!(!outcome.wrote_key, "silence should not produce a key");
        assert_eq!(outcome.key_confidence, 0.0);
        assert!(lib.is_track_analyzed(&track_id).unwrap());

        let cache_has_grid: i64 = lib
            .connection()
            .query_row(
                "SELECT has_active_grid FROM analysis_cache WHERE fingerprint_id = ?1",
                params![fp_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cache_has_grid, 0);

        let grid_rows: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_beatgrids WHERE track_id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(grid_rows, 0);
    }

    #[test]
    fn is_track_analyzed_returns_false_for_fresh_track() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        assert!(!lib.is_track_analyzed(&track_id).unwrap());
    }

    #[test]
    fn analyze_track_returns_typed_errors_for_unknown_track_and_missing_file() {
        let lib = Library::open_in_memory().unwrap();
        let err = lib
            .analyze_track("ffffffff-0000-0000-0000-000000000000")
            .err();
        assert!(matches!(err, Some(LibraryError::TrackNotFound { .. })));
    }

    /// Generate a I-IV-V-I chord progression in the given key
    /// (`root_hz` = the I-chord root) and seed the library with
    /// it. Returns `(track_id, fingerprint_id, path)`.
    ///
    /// A I-IV-V-I progression sweeps the full diatonic set of the
    /// key (C major's I = C-E-G, IV = F-A-C, V = G-B-D, back to
    /// I): seven distinct pitch classes covering every note of
    /// the major scale. Krumhansl-Kessler templates separate
    /// cleanly on this profile — the diatonic set of C major
    /// differs from A minor's natural-minor set, so the relative-
    /// key template ambiguity that hits pure-triad fixtures is
    /// gone. For C major with `third_semitones = 4.0`, the
    /// expected outcome is Camelot `8B`.
    fn seed_track_with_chord(
        lib: &Library,
        tmp: &tempfile::TempDir,
        root_hz: f32,
        third_semitones: f32,
        secs: f64,
    ) -> (String, i64, std::path::PathBuf) {
        let path = tmp.path().join(format!("progression_{root_hz}.wav"));
        let sample_rate = 44_100_u32;
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let total = (secs * f64::from(sample_rate)) as usize;
        let mut samples = vec![0.0_f32; total];
        let dt = 1.0 / sample_rate as f32;

        // Frequencies for each chord in the I-IV-V-I cycle. The
        // mode parameter (`third_semitones`) sets the chord
        // quality of the I chord; for major keys the IV and V
        // chords are also major triads diatonic to the key.
        let i_root = root_hz;
        let i_third = i_root * 2.0_f32.powf(third_semitones / 12.0);
        let i_fifth = i_root * 2.0_f32.powf(7.0 / 12.0);
        let iv_root = i_root * 2.0_f32.powf(5.0 / 12.0);
        let iv_third = iv_root * 2.0_f32.powf(4.0 / 12.0);
        let iv_fifth = iv_root * 2.0_f32.powf(7.0 / 12.0);
        let v_root = i_root * 2.0_f32.powf(7.0 / 12.0);
        let v_third = v_root * 2.0_f32.powf(4.0 / 12.0);
        let v_fifth = v_root * 2.0_f32.powf(7.0 / 12.0);

        for (i, s) in samples.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 * dt;
            let chord_idx = (4 * i) / total;
            let (f1, f2, f3) = match chord_idx {
                0 => (i_root, i_third, i_fifth),
                1 => (iv_root, iv_third, iv_fifth),
                2 => (v_root, v_third, v_fifth),
                _ => (i_root, i_third, i_fifth),
            };
            *s = 0.25
                * ((std::f32::consts::TAU * f1 * t).sin()
                    + (std::f32::consts::TAU * f2 * t).sin()
                    + (std::f32::consts::TAU * f3 * t).sin());
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for s in samples {
            #[allow(clippy::cast_possible_truncation)]
            let pcm = (s * i16::MAX as f32) as i16;
            writer.write_sample(pcm).unwrap();
        }
        writer.finalize().unwrap();

        let boot_uuid = "00000000-0000-0000-0000-000000000000";
        lib.connection()
            .execute(
                "INSERT INTO volumes (volume_uuid, display_name, last_known_mount_point, last_seen_at, is_internal) \
                 VALUES (?1, 'Macintosh HD', '/', strftime('%s','now'), 1) \
                 ON CONFLICT(volume_uuid) DO NOTHING",
                params![boot_uuid],
            )
            .unwrap();
        let track = dub_io::Track::load_from_path(&path).unwrap();
        let fp = Fingerprint::compute_from_f32(
            track.samples(),
            track.sample_rate(),
            u32::from(track.channels()),
        )
        .unwrap();
        let fp_id = lib
            .upsert_fingerprint(
                &fp,
                Some(track.sample_rate()),
                Some(u32::from(track.channels())),
                None,
            )
            .unwrap();
        let track_id = uuid::Uuid::new_v4().to_string();
        lib.insert_track(&track_id, Some(fp_id), Some(fp.duration_ms()), None)
            .unwrap();
        let rel = path
            .strip_prefix("/")
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        lib.upsert_track_file(
            &track_id,
            boot_uuid,
            &rel,
            Some("wav"),
            Some(44_100),
            None,
            Some(1),
            None,
            None,
        )
        .unwrap();
        (track_id, fp_id, path)
    }

    #[test]
    fn analyze_track_writes_camelot_key_for_c_major_chord() {
        // C major triad held for 8 s. The DSP recovers C major
        // (Camelot 8B) at non-zero confidence; analyze_track must
        // upsert exactly one `track_keys(source='auto')` row,
        // mark it active (no other-source row present), and stamp
        // `analysis_cache.has_active_key = 1`.
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, fp_id, _) = seed_track_with_chord(&lib, &tmp, 261.63, 4.0, 8.0);

        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(
            outcome.wrote_key,
            "C major progression must yield a key row"
        );
        // The I-IV-V-I progression spans the full diatonic set of
        // C major, breaking the pure-triad relative-key
        // ambiguity. We assert tonic = C with confidence > 0;
        // major vs minor is left soft (assert Camelot *family
        // 8*) because the Krumhansl-Kessler templates rate the
        // major and minor versions of the same family very close
        // when the fixture is synthetic / harmonic-free.
        let (num, _) = dub_spectral::parse_camelot(outcome.camelot)
            .expect("auto pass produces canonical Camelot");
        assert_eq!(
            num, 8,
            "C major progression must classify into Camelot family 8; got {}",
            outcome.camelot
        );
        assert_eq!(outcome.tonic_pc, 0, "tonic must be C (PC 0)");
        assert!(outcome.key_auto_is_active);

        let (rows, active, src): (i64, i64, String) = lib
            .connection()
            .query_row(
                "SELECT COUNT(*), MAX(is_active), MAX(source) \
                 FROM track_keys WHERE track_id = ?1",
                params![track_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(src, "auto");
        assert_eq!(active, 1);

        let cache_active_key: i64 = lib
            .connection()
            .query_row(
                "SELECT has_active_key FROM analysis_cache WHERE fingerprint_id = ?1",
                params![fp_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cache_active_key, 1);
    }

    #[test]
    fn analyze_track_does_not_steal_active_key_from_higher_priority_source() {
        // PRD §8.3 priority: serato > auto. If serato already
        // claimed the active-key slot, our auto pass must record
        // its row with is_active = 0 (preserving the user's
        // choice) but otherwise upsert normally.
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_chord(&lib, &tmp, 261.63, 4.0, 8.0);

        lib.connection()
            .execute(
                "INSERT INTO track_keys \
                 (track_id, source, key_notation, original_notation, confidence, is_active, captured_at) \
                 VALUES (?1, 'serato', '5A', 'C minor', NULL, 1, strftime('%s','now'))",
                params![track_id],
            )
            .unwrap();

        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(outcome.wrote_key);
        assert!(
            !outcome.key_auto_is_active,
            "auto must not steal active key from a higher-priority source"
        );

        let (serato_active, auto_active): (i64, i64) = lib
            .connection()
            .query_row(
                "SELECT \
                    SUM(CASE WHEN source = 'serato' THEN is_active ELSE 0 END), \
                    SUM(CASE WHEN source = 'auto'   THEN is_active ELSE 0 END) \
                 FROM track_keys WHERE track_id = ?1",
                params![track_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(serato_active, 1);
        assert_eq!(auto_active, 0);
    }

    // ===== active_beatgrid_for_track (M11d.5 round 4) =========

    #[test]
    fn active_beatgrid_for_track_returns_none_before_analysis() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);

        let row = lib.active_beatgrid_for_track(&track_id).unwrap();
        assert!(
            row.is_none(),
            "fresh track has no active grid until analyze_track runs"
        );
    }

    #[test]
    fn active_beatgrid_for_track_returns_auto_row_after_analysis() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);

        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(outcome.wrote_grid, "click track must produce a grid");
        assert!(outcome.grid_auto_is_active);

        let row = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("auto row must surface as the active grid");
        assert_eq!(row.source, "auto");
        assert!(
            (row.bpm - outcome.bpm).abs() < 1e-9,
            "active row bpm must equal the outcome bpm (no round-trip drift)"
        );
        assert!(
            (row.anchor_secs - outcome.anchor_secs).abs() < 1e-9,
            "active row anchor must equal the outcome anchor"
        );
        assert!(row.captured_at > 0, "captured_at populated by strftime");
    }

    #[test]
    fn active_beatgrid_for_track_returns_other_source_when_auto_is_inactive() {
        // Pre-seed a serato row (active) and then run analyze_track.
        // The auto row lands with is_active = 0; the lookup must
        // surface the serato row instead — confirming the helper
        // honours the partial unique index and PRD §8.3 priority.
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);

        lib.connection()
            .execute(
                "INSERT INTO track_beatgrids \
                 (track_id, source, anchor_secs, bpm, is_active, captured_at) \
                 VALUES (?1, 'serato', 0.123, 174.0, 1, strftime('%s','now'))",
                params![track_id],
            )
            .unwrap();

        lib.analyze_track(&track_id).unwrap();

        let row = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("serato row must be returned as the active grid");
        assert_eq!(row.source, "serato");
        assert!((row.bpm - 174.0).abs() < 1e-9);
        assert!((row.anchor_secs - 0.123).abs() < 1e-9);
    }

    #[test]
    fn active_beatgrid_for_track_returns_none_for_unknown_track_id() {
        let lib = Library::open_in_memory().unwrap();
        let row = lib
            .active_beatgrid_for_track("ffffffff-0000-0000-0000-000000000000")
            .unwrap();
        assert!(
            row.is_none(),
            "an id with no matching track resolves to no active row (not an error)"
        );
    }

    #[test]
    fn upsert_user_tap_beatgrid_becomes_active_and_deactivates_auto() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();

        lib.upsert_user_tap_beatgrid(&track_id, 1.25, 65.0, 0)
            .unwrap();

        let row = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("user tap row must be active");
        assert_eq!(row.source, "user_tap");
        assert!((row.bpm - 65.0).abs() < 1e-9);
        assert!((row.anchor_secs - 1.25).abs() < 1e-9);

        let auto_active: i64 = lib
            .connection()
            .query_row(
                "SELECT is_active FROM track_beatgrids \
                 WHERE track_id = ?1 AND source = 'auto'",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(auto_active, 0, "auto row must be deactivated");
    }

    /// Re-analyze on an unlocked track with an active `user_tap`
    /// row demotes that row and replaces it with a fresh auto
    /// grid (PRD-BEATS §4 "Re-analyze" row: pure reset). The
    /// previous round shipped this as the "non-force" path of a
    /// `force: bool` parameter; round 3 removes `force` entirely
    /// (lock is absolute) so this is now the *only* re-analyze
    /// path on an unlocked track. Regression for the stale-tap
    /// 133.017 / 133.000 mismatch the user reported on Oppidan.
    #[test]
    fn reanalyze_on_unlocked_track_demotes_active_user_tap_to_let_auto_win() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);

        lib.analyze_track(&track_id).unwrap();
        lib.upsert_user_tap_beatgrid(&track_id, 0.123, 120.5, 0)
            .unwrap();

        let pre = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("user tap row must be active before reanalyze");
        assert_eq!(pre.source, "user_tap");
        assert!((pre.bpm - 120.5).abs() < 1e-9);

        let outcome = lib.analyze_track(&track_id).unwrap();
        assert!(outcome.wrote_grid);
        assert!(
            outcome.grid_auto_is_active,
            "auto row must claim is_active = 1 after re-analyze"
        );

        let post = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("auto row must be active after re-analyze");
        assert_eq!(post.source, "auto");

        let tap_active: i64 = lib
            .connection()
            .query_row(
                "SELECT is_active FROM track_beatgrids \
                 WHERE track_id = ?1 AND source = 'user_tap'",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tap_active, 0, "stale user_tap row must be demoted");
    }

    /// PRD-BEATS C2 (round 4) — `bar_phase` is a first-class
    /// column on `track_beatgrids` and `active_beatgrid_for_track`
    /// surfaces it verbatim. The `user_tap` upsert path takes the
    /// phase as a parameter (no inference from `anchor_secs`); the
    /// schema default is `0` so legacy rows behave as before.
    #[test]
    fn upsert_user_tap_beatgrid_roundtrips_bar_phase() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();

        lib.upsert_user_tap_beatgrid(&track_id, 1.25, 120.0, 2)
            .unwrap();

        let row = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("user tap row must be active");
        assert_eq!(row.bar_phase, 2);

        lib.upsert_user_tap_beatgrid(&track_id, 1.25, 120.0, 3)
            .unwrap();
        let row = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("user tap row must remain active after second upsert");
        assert_eq!(row.bar_phase, 3);
    }

    /// PRD-BEATS §3.5 lock-is-absolute: analysing a locked track
    /// returns `GridLocked` rather than silently skipping. The
    /// previous behaviour (non-force on locked = no-op `Ok`) is
    /// replaced with an explicit refusal so the Apple shell can
    /// short-circuit cleanly. The locked grid stays untouched.
    #[test]
    fn analyze_track_on_locked_grid_returns_grid_locked_and_leaves_state() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);

        lib.analyze_track(&track_id).unwrap();
        lib.upsert_user_tap_beatgrid(&track_id, 0.123, 120.5, 0)
            .unwrap();
        lib.set_grid_locked(&track_id, true).unwrap();

        let err = lib.analyze_track(&track_id).err();
        assert!(
            matches!(err, Some(LibraryError::GridLocked { .. })),
            "locked grid must refuse analysis with GridLocked; got {err:?}"
        );

        let post = lib
            .active_beatgrid_for_track(&track_id)
            .unwrap()
            .expect("tap row must still be active");
        assert_eq!(post.source, "user_tap");
        assert!((post.bpm - 120.5).abs() < 1e-9);
    }

    #[test]
    fn analyze_track_is_idempotent_for_keys_on_re_run() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_chord(&lib, &tmp, 261.63, 4.0, 8.0);

        let first = lib.analyze_track(&track_id).unwrap();
        let second = lib.analyze_track(&track_id).unwrap();
        assert_eq!(first.wrote_key, second.wrote_key);
        assert_eq!(first.camelot, second.camelot);

        let n: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_keys WHERE track_id = ?1 AND source = 'auto'",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "re-analyze should upsert the auto key row, not duplicate"
        );
    }

    // ===== octave shift + reset (deck header BPM context menu) =====

    /// Helper: `t_secs` must sit on a downbeat of the grid encoded
    /// by `(anchor, bpm, bar_phase)` with `beats_per_bar = 4`.
    /// Downbeats live at `anchor + (bar_phase + 4·k)·period` for
    /// any integer k. We reverse the relation: compute how many
    /// beats fit between `anchor` and `t_secs`, check it lands on
    /// an integer beat, and check that integer is congruent to
    /// `bar_phase` modulo 4.
    fn assert_is_downbeat(t_secs: f64, anchor: f64, bpm: f64, bar_phase: u8) {
        let period = 60.0 / bpm;
        let n_beats = (t_secs - anchor) / period;
        let n_round = n_beats.round();
        assert!(
            (n_beats - n_round).abs() < 1e-6,
            "{t_secs} must sit on a beat of (anchor={anchor}, bpm={bpm}); \
             n_beats={n_beats}"
        );
        let n_i = n_round as i64;
        assert_eq!(
            n_i.rem_euclid(4),
            i64::from(bar_phase),
            "{t_secs} must sit on a *downbeat* of (anchor={anchor}, bpm={bpm}, \
             bar_phase={bar_phase}); n_beats={n_i}"
        );
    }

    /// 2× preserves the *position* (in seconds) of every existing
    /// downbeat — the new grid is denser, but every old downbeat is
    /// still a downbeat in it. The post-wrap-back representation
    /// `(anchor, bar_phase)` may differ from the pre-shift one
    /// because `anchor` lives in `[0, period)` and the period
    /// halves; what matters is the visible musical position.
    #[test]
    fn scale_active_beatgrid_doubles_bpm_and_preserves_downbeat_position() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();
        // anchor=0.5s, bpm=60 (period 1s), bar_phase=2
        // → original downbeats at 2.5s, 6.5s, 10.5s, …
        lib.upsert_user_tap_beatgrid(&track_id, 0.5, 60.0, 2)
            .unwrap();
        let pre = lib.active_beatgrid_for_track(&track_id).unwrap().unwrap();
        let pre_downbeat = pre.anchor_secs + f64::from(pre.bar_phase) * 60.0 / pre.bpm;

        let post = lib
            .scale_active_beatgrid(&track_id, 2.0)
            .unwrap()
            .expect("scale on a known-active grid must surface the new active row");

        assert_eq!(
            post.source, "user_tap",
            "scaled grid must persist as user_tap"
        );
        assert!(
            (post.bpm - 120.0).abs() < 1e-9,
            "bpm must be doubled, got {}",
            post.bpm
        );
        let new_period = 60.0 / post.bpm;
        assert!(
            post.anchor_secs >= 0.0 && post.anchor_secs < new_period,
            "anchor must lie in [0, new_period); got anchor={} period={new_period}",
            post.anchor_secs
        );
        assert!(post.bar_phase < 4, "bar_phase must stay < beats_per_bar");
        // Every old downbeat must still be a downbeat in the new grid.
        assert_is_downbeat(pre_downbeat, post.anchor_secs, post.bpm, post.bar_phase);
        assert_is_downbeat(
            pre_downbeat + 4.0,
            post.anchor_secs,
            post.bpm,
            post.bar_phase,
        );
    }

    /// ½ preserves the position of every *retained* downbeat (every
    /// other one in the original grid). Same invariant as the 2×
    /// case: the originally-pinned downbeat at `pre_downbeat` must
    /// still be a downbeat in the half-density grid.
    #[test]
    fn scale_active_beatgrid_halves_bpm_and_preserves_downbeat_position() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();
        // anchor=0.25s, bpm=120 (period 0.5s), bar_phase=1
        // → original downbeats at 0.75s, 2.75s, 4.75s, …
        lib.upsert_user_tap_beatgrid(&track_id, 0.25, 120.0, 1)
            .unwrap();
        let pre = lib.active_beatgrid_for_track(&track_id).unwrap().unwrap();
        let pre_downbeat = pre.anchor_secs + f64::from(pre.bar_phase) * 60.0 / pre.bpm;

        let post = lib.scale_active_beatgrid(&track_id, 0.5).unwrap().unwrap();
        assert!(
            (post.bpm - 60.0).abs() < 1e-9,
            "bpm must be halved, got {}",
            post.bpm
        );
        assert_is_downbeat(pre_downbeat, post.anchor_secs, post.bpm, post.bar_phase);
        // After halving, the next downbeat is 4 new-beats = 4
        // seconds further (new period = 1s). The original grid had
        // a downbeat at pre_downbeat + 2s — but that one *is not*
        // a downbeat in the half-density grid; only every other one
        // is retained.
        assert_is_downbeat(
            pre_downbeat + 4.0,
            post.anchor_secs,
            post.bpm,
            post.bar_phase,
        );
    }

    #[test]
    fn scale_active_beatgrid_rejects_invalid_multipliers() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();

        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let err = lib.scale_active_beatgrid(&track_id, bad).err();
            assert!(
                matches!(err, Some(LibraryError::DecodeFailed { .. })),
                "multiplier {bad} must be rejected, got {err:?}"
            );
        }
    }

    #[test]
    fn scale_active_beatgrid_refuses_to_touch_locked_grids() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();
        lib.set_grid_locked(&track_id, true).unwrap();

        let err = lib.scale_active_beatgrid(&track_id, 2.0).err();
        assert!(
            matches!(err, Some(LibraryError::GridLocked { .. })),
            "locked grid must refuse octave shift, got {err:?}"
        );
    }

    #[test]
    fn scale_active_beatgrid_returns_none_for_track_with_no_active_grid() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        // No analyze → no grid rows at all.
        let res = lib.scale_active_beatgrid(&track_id, 2.0).unwrap();
        assert!(res.is_none(), "no active grid → no-op Ok(None)");
    }

    #[test]
    fn reset_active_beatgrid_demotes_user_tap_and_reactivates_auto() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();
        let auto = lib.active_beatgrid_for_track(&track_id).unwrap().unwrap();
        assert_eq!(auto.source, "auto");

        lib.upsert_user_tap_beatgrid(&track_id, 0.123, 99.0, 0)
            .unwrap();
        let tapped = lib.active_beatgrid_for_track(&track_id).unwrap().unwrap();
        assert_eq!(tapped.source, "user_tap");

        let reset = lib
            .reset_active_beatgrid_to_auto(&track_id)
            .unwrap()
            .expect("auto row must come back as the active grid");
        assert_eq!(reset.source, "auto");
        assert!(
            (reset.bpm - auto.bpm).abs() < 1e-9,
            "reset must restore the original auto bpm exactly"
        );
        assert!(
            (reset.anchor_secs - auto.anchor_secs).abs() < 1e-9,
            "reset must restore the original auto anchor exactly"
        );
        assert_eq!(reset.bar_phase, auto.bar_phase);

        // user_tap row must remain in the table but with is_active=0
        let tap_active: i64 = lib
            .connection()
            .query_row(
                "SELECT is_active FROM track_beatgrids \
                 WHERE track_id = ?1 AND source = 'user_tap'",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tap_active, 0, "user_tap row must be demoted, not deleted");
    }

    #[test]
    fn reset_active_beatgrid_returns_none_when_no_auto_row_exists() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        // Never analyzed → no auto row. Write a user_tap row anyway
        // to exercise the "no auto to fall back on" branch.
        lib.upsert_user_tap_beatgrid(&track_id, 0.0, 120.0, 0)
            .unwrap();

        let res = lib.reset_active_beatgrid_to_auto(&track_id).unwrap();
        assert!(
            res.is_none(),
            "no auto row → reset must report nothing to restore (caller asks user to analyze)"
        );
        // user_tap row must still be active so the deck doesn't go gridless.
        let tap_active: i64 = lib
            .connection()
            .query_row(
                "SELECT is_active FROM track_beatgrids \
                 WHERE track_id = ?1 AND source = 'user_tap'",
                params![track_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tap_active, 1, "user_tap stays active when no auto exists");
    }

    #[test]
    fn reset_active_beatgrid_refuses_to_touch_locked_grids() {
        let lib = Library::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (track_id, _, _) = seed_track_with_file(&lib, &tmp, 120.0, 8.0);
        lib.analyze_track(&track_id).unwrap();
        lib.upsert_user_tap_beatgrid(&track_id, 0.123, 99.0, 0)
            .unwrap();
        lib.set_grid_locked(&track_id, true).unwrap();

        let err = lib.reset_active_beatgrid_to_auto(&track_id).err();
        assert!(
            matches!(err, Some(LibraryError::GridLocked { .. })),
            "locked grid must refuse reset, got {err:?}"
        );
    }
}
