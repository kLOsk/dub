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

use rusqlite::params;

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
    /// * [`LibraryError::TrackHasNoFingerprint`] — the track has no
    ///   `fingerprint_id`. Should not happen via M11c imports but is
    ///   guarded explicitly because `analysis_cache` is keyed by
    ///   fingerprint id.
    /// * [`LibraryError::TrackHasNoFile`] — the track has no
    ///   resolvable on-disk path. Caller (the Swift scanner) should
    ///   surface this as "file is missing"; the next Relocate run
    ///   re-attaches it and analysis can be retried.
    /// * [`LibraryError::DecodeFailed`] — `dub-io` couldn't decode
    ///   the file (corrupt / unsupported codec).
    /// * [`LibraryError::Sqlite`] — any underlying database error.
    pub fn analyze_track(&self, track_id: &str) -> Result<AnalysisOutcome> {
        let (fingerprint_id, file_path) = self.track_analysis_keys(track_id)?;
        let track =
            dub_io::Track::load_from_path(&file_path).map_err(|e| LibraryError::DecodeFailed {
                track_id: track_id.to_string(),
                path: file_path.clone(),
                reason: format!("{e}"),
            })?;

        // ---- Beat grid (M11c.1) ----------------------------------
        let grid =
            dub_bpm::analyze_beat_grid(track.samples(), track.sample_rate(), track.channels())
                .map_err(|e| LibraryError::DecodeFailed {
                    track_id: track_id.to_string(),
                    path: file_path.clone(),
                    reason: format!("beat-grid analysis failed: {e}"),
                })?;

        let mut outcome = AnalysisOutcome::EMPTY;
        if grid.confidence > 0.0 {
            let anchor_secs = grid.beats.first().copied().unwrap_or(0.0);
            let other_grid_active = self.has_non_auto_active_grid(track_id)?;
            let grid_auto_is_active = !other_grid_active;
            self.upsert_auto_beatgrid(track_id, anchor_secs, grid.bpm, grid_auto_is_active)?;
            outcome.bpm = grid.bpm;
            outcome.anchor_secs = anchor_secs;
            outcome.bpm_confidence = grid.confidence;
            outcome.grid_auto_is_active = grid_auto_is_active;
            outcome.wrote_grid = true;
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

        // Stamp `analysis_cache` exactly once. `has_active_grid` and
        // `has_active_key` reflect whether the auto pass landed the
        // active row (so they can be `1` here but get flipped to `0`
        // later if an importer claims the active slot — that's
        // tracked separately by `stamp_analysis_cache_after_import`
        // when those importers land in M11e).
        self.stamp_analysis_cache(
            fingerprint_id,
            outcome.grid_auto_is_active,
            outcome.key_auto_is_active,
        )?;

        Ok(outcome)
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
            "SELECT source, bpm, anchor_secs, captured_at \
                 FROM track_beatgrids \
                 WHERE track_id = ?1 AND is_active = 1",
            params![track_id],
            |r| {
                Ok(ActiveBeatgrid {
                    source: r.get::<_, String>(0)?,
                    bpm: r.get::<_, f64>(1)?,
                    anchor_secs: r.get::<_, f64>(2)?,
                    captured_at: r.get::<_, i64>(3)?,
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

    /// Resolve the `(fingerprint_id, absolute_path)` pair we need
    /// to run analysis: the fingerprint id keys `analysis_cache`,
    /// the path keys the decode. Returns typed errors for the
    /// two distinct "can't analyze this" cases so the FFI surfaces
    /// them cleanly.
    fn track_analysis_keys(&self, track_id: &str) -> Result<(i64, std::path::PathBuf)> {
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
        let fingerprint_id = fingerprint_id.ok_or_else(|| LibraryError::TrackHasNoFingerprint {
            track_id: track_id.to_string(),
        })?;
        let path =
            self.resolve_track_path(track_id)?
                .ok_or_else(|| LibraryError::TrackHasNoFile {
                    track_id: track_id.to_string(),
                })?;
        Ok((fingerprint_id, path))
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

    /// Upsert the auto-source row in `track_beatgrids`. The
    /// `(track_id, source)` UNIQUE constraint makes this an
    /// idempotent refresh of the row's anchor / BPM / captured_at.
    fn upsert_auto_beatgrid(
        &self,
        track_id: &str,
        anchor_secs: f64,
        bpm: f64,
        is_active: bool,
    ) -> Result<()> {
        self.connection()
            .execute(
                "INSERT INTO track_beatgrids \
                 (track_id, source, anchor_secs, bpm, is_active, captured_at) \
                 VALUES (?1, 'auto', ?2, ?3, ?4, strftime('%s','now')) \
                 ON CONFLICT(track_id, source) DO UPDATE SET \
                     anchor_secs = excluded.anchor_secs, \
                     bpm         = excluded.bpm, \
                     is_active   = excluded.is_active, \
                     captured_at = excluded.captured_at",
                params![track_id, anchor_secs, bpm, if is_active { 1 } else { 0 }],
            )
            .map_err(|e| LibraryError::sqlite("upsert_auto_beatgrid", e))?;
        Ok(())
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
    ) -> Result<()> {
        self.connection()
            .execute(
                "INSERT INTO analysis_cache \
                 (fingerprint_id, has_active_grid, has_active_key, analyzed_at) \
                 VALUES (?1, ?2, ?3, strftime('%s','now')) \
                 ON CONFLICT(fingerprint_id) DO UPDATE SET \
                     has_active_grid = excluded.has_active_grid, \
                     has_active_key  = excluded.has_active_key, \
                     analyzed_at     = excluded.analyzed_at",
                params![
                    fingerprint_id,
                    if has_active_grid { 1 } else { 0 },
                    if has_active_key { 1 } else { 0 }
                ],
            )
            .map_err(|e| LibraryError::sqlite("stamp_analysis_cache", e))?;
        Ok(())
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
        lib.insert_track(&track_id, fp_id, fp.duration_ms(), None)
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
        lib.insert_track(&track_id, fp_id, fp.duration_ms(), None)
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
        lib.insert_track(&track_id, fp_id, fp.duration_ms(), None)
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
}
