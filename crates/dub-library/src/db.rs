//! The `Library` struct: a typed handle to the Dub SQLite database
//! plus the small surface of M11a-shipping operations (open / migrate
//! / volume registration). M11b–M11f will grow this surface with the
//! fingerprint cache, file registration, metadata-source writes,
//! crate management, browser queries, and exporters.
//!
//! ## Threading
//!
//! `Library` owns a single `rusqlite::Connection`. It is **not**
//! `Sync`; the M11d browser will hold one connection on the UI
//! thread (read-only `SELECT`s) and a second connection on the
//! background importer thread. WAL mode (set in `schema::open_and_migrate`)
//! means the two connections don't block each other for normal use.
//!
//! Audio-thread access is forbidden — the database is on disk and
//! every operation can block on I/O. The engine never touches the
//! library directly; the Apple shell / CLI reads the library and
//! hands `Arc<Track>` snapshots to the engine via the existing
//! load path (PRD §6.4).

use std::path::{Path, PathBuf};

use dub_fingerprint::Fingerprint;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{LibraryError, Result};
use crate::paths::default_library_db_path;
use crate::schema::open_and_migrate;
use crate::volumes::DiscoveredVolume;

/// Normalise an optional metadata string: trim whitespace, return
/// `None` when the result is empty. Matches the schema's
/// "absent = NULL, not empty string" convention so a downstream
/// `COALESCE(artist, ...)` chain doesn't accidentally pick an
/// empty string as a real value.
fn nonempty(s: Option<&str>) -> Option<&str> {
    s.and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            Some(v.trim())
        }
    })
}

/// Map a `crates` write error to a typed [`LibraryError`]. A
/// `UNIQUE (parent_crate_id, name)` violation becomes
/// [`LibraryError::CrateNameConflict`] so the UI can show a friendly
/// message; everything else passes through as a generic `Sqlite`
/// error tagged with `context`.
fn map_crate_constraint(e: rusqlite::Error, name: &str, context: &'static str) -> LibraryError {
    if let rusqlite::Error::SqliteFailure(err, _) = &e {
        if err.code == rusqlite::ErrorCode::ConstraintViolation {
            return LibraryError::CrateNameConflict {
                name: name.to_string(),
            };
        }
    }
    LibraryError::sqlite(context, e)
}

/// One row returned by [`Library::list_files_for_scan`] for the
/// M11d.4 background missing-files scanner. The scanner reads the
/// rows, calls `access()` per absolute path, and feeds the result
/// back into [`Library::mark_file_state`].
#[derive(Debug, Clone, PartialEq)]
pub struct FileScanRow {
    /// Primary key of the `track_files` row. Re-used by
    /// `mark_file_state` to address the same row without
    /// re-resolving by `(volume_uuid, relative_path)`.
    pub file_id: i64,
    /// Canonical track this file belongs to.
    pub track_id: String,
    /// Volume the file lives on.
    pub volume_uuid: String,
    /// Volume-relative path.
    pub relative_path: String,
    /// Current `is_missing` flag. The scanner uses this to skip
    /// no-op writes when the verdict hasn't changed.
    pub was_missing: bool,
    /// `volumes.last_known_mount_point` joined-in for caller
    /// convenience. `None` means the volume itself is offline,
    /// in which case the scanner treats the row as missing
    /// without paying a `stat()` syscall.
    pub mount_point: Option<String>,
}

/// One row returned by [`Library::list_missing_tracks`] for the
/// M11d.4 Relocate panel. Carries the identity signals the matcher
/// uses to confirm a candidate file on the user-supplied directory
/// matches one of the missing tracks: fingerprint, duration, and
/// the original filename.
#[derive(Debug, Clone, PartialEq)]
pub struct MissingTrack {
    /// Canonical track UUID.
    pub track_id: String,
    /// `fingerprints.id` for [`Library::load_fingerprint`].
    pub fingerprint_id: i64,
    /// Original track duration in milliseconds. Matcher uses
    /// `|cand.duration - track.duration| < 200 ms` (mirrors
    /// PRD §8.1 dedupe threshold).
    pub duration_ms: u32,
    /// The relative path the track was last seen at, if any.
    /// Useful UX context for the Relocate panel.
    pub last_relative_path: Option<String>,
    /// Just the basename of `last_relative_path` for filename
    /// matching. Cached at SELECT-time so the Apple side doesn't
    /// parse paths twice.
    pub last_filename: Option<String>,
}

/// One row in the M11d browser's track list. The PRD §8.1
/// priority chain (`serato > rekordbox > traktor > id3 > filename`)
/// is baked in at the SELECT level via COALESCE so the browser
/// doesn't have to reimplement the chain. v1 only has the `id3`
/// and `filename` sources wired; the Serato / rekordbox / Traktor
/// importers slot in at M11e by extending the COALESCE chain.
///
/// **Per-column carve-outs.** `year` deviates and is filename-first
/// because the `[YYYY]` filename tail is more reliable than ID3
/// year in DJ-rip-naming conventions (file-format-stamped year,
/// not ripper-stamped). The deviation is documented per-field.
///
/// **Junk ID3 titles** (PRD §8.4: `Track 01`, `Unknown`, blogspot
/// noise, …) are not yet filtered at display time. When the user
/// has junk ID3 titles, the browser will show the junk title
/// instead of falling through to the filename source. A future
/// patch will register an `is_junk_title` SQL scalar function so
/// the COALESCE skips junk verbatim-preserved ID3 titles per
/// §8.1's "preserved verbatim" rule.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRow {
    /// Canonical UUID from `tracks.id`.
    pub id: String,
    /// Display title. PRD §8.1 priority: id3 source preferred over
    /// filename source. Falls through to filename when ID3 row has
    /// no title.
    pub title: Option<String>,
    /// Display artist. PRD §8.1 priority: id3 source preferred over
    /// filename source. Falls through to filename when ID3 row has
    /// no artist.
    pub artist: Option<String>,
    /// Album (id3 source).
    pub album: Option<String>,
    /// Genre (id3 source).
    pub genre: Option<String>,
    /// Year (filename source preferred over id3; the filename
    /// `[YYYY]` tail is the more reliable signal in DJ-rip-naming
    /// conventions).
    pub year: Option<i32>,
    /// BPM from the active beat grid (`track_beatgrids` where
    /// `is_active = 1`). Honours PRD §8.3 priority: an imported
    /// Serato / rekordbox / Traktor grid wins over the auto grid;
    /// the auto grid wins over nothing. `None` for tracks that
    /// have never been analyzed (M11c.1 onward). The browser
    /// renders `None` as an em-dash; ID3 BPM is **not** used as a
    /// fallback here because ID3 carries no anchor and is too
    /// unreliable to bind to the deck (PRD §8.3).
    pub bpm: Option<f64>,
    /// Musical key from the active row of `track_keys` (M11c.2).
    /// Always Camelot when present (e.g. `8B`); the per-source
    /// original notation is preserved in
    /// `track_keys.original_notation` for export round-trip but
    /// is **not** what the browser displays. `None` for tracks
    /// that have never been analysed (M11c.2 onward). The
    /// browser renders `None` as an em-dash; ID3 `TKEY` is **not**
    /// used as a fallback because it would break the
    /// "unanalysed = no key shown" visual contract.
    pub key: Option<String>,
    /// Duration in milliseconds, from `tracks.duration_ms`.
    ///
    /// `0` means "unknown yet". M11c.4 imports are metadata-only:
    /// fresh rows keep `tracks.duration_ms = NULL` until first
    /// deck-load analysis attaches the fingerprint and measured
    /// duration. The browser must still list those rows.
    pub duration_ms: u32,
    /// Comma-separated canonical version-tokens (filename source
    /// preferred). `None` when no tokens were detected.
    pub version_tokens: Option<String>,
    /// `Some(other_track_id)` when this row has a sibling-version
    /// link per §8.1 dedupe. Drives the M11d.3 potential-duplicate
    /// indicator.
    pub potential_duplicate_id: Option<String>,
    /// Free-form per-track comment from `track_metadata_source.
    /// comment` (id3 source today; M11e+ importers add Serato /
    /// Traktor / rekordbox comment fields under their own sources
    /// and we'll switch to COALESCE once the priority is settled).
    /// `None` when the file carries no comment frame, which is
    /// common for DJ rips. Surfaced in the M11d.5 LibraryView
    /// rebuild so DJs can read the per-track notes Mixed In Key
    /// / Lexicon / hand-written tooling stamps in there (cue
    /// timestamps, version hints, "energy 7", etc).
    pub comment: Option<String>,
    /// Composer (`TCOM` ID3 / equivalent). id3 source only in v1.
    pub composer: Option<String>,
    /// Track number (`TRCK` ID3 / equivalent). id3 source only in
    /// v1. `None` when the tag is absent.
    pub track_number: Option<i32>,
    /// Origin source — `"filesystem"` for M11c-imported rows;
    /// `"serato"` / `"traktor"` / `"rekordbox"` / `"itunes"` for
    /// future M11e+ importers. Synthesised from the per-source
    /// metadata-row `source` column.
    pub source: String,
    /// Volume UUID of the most-recently-confirmed file row for
    /// this track. `None` only for tracks with no `track_files`
    /// row (a state the importer never produces but a hand-
    /// inserted row could). The M11d.3 browser uses this to
    /// drive the missing-file glyph via a per-volume reachability
    /// cache — checking 5 volumes is cheap; checking 5 000 files
    /// per refresh would not be.
    pub primary_volume_uuid: Option<String>,
    /// Last-known mount point of the primary volume. Combined
    /// with `primary_relative_path` to reconstruct an absolute
    /// path on the Apple side without a per-row FFI round-trip.
    /// `None` when the volume is currently unmounted (the
    /// `volumes.last_known_mount_point` schema column is
    /// nullable for exactly this reason).
    pub primary_volume_mount_point: Option<String>,
    /// Volume-relative path of the primary file. Combined with
    /// `primary_volume_mount_point` to reconstruct the absolute
    /// path the browser drags / Space-loads.
    pub primary_relative_path: Option<String>,
    /// `true` once auto-analysis (M11c.1) has run against this
    /// track's fingerprint, regardless of whether a grid was
    /// found. Derived from `analysis_cache.analyzed_at IS NOT NULL`
    /// in the SELECT below. Drives the M11c.1 browser dimming:
    /// unanalyzed rows render at reduced opacity; analyzed rows
    /// render at full opacity even when the analyser declined to
    /// place a grid (silence stems, ambient pieces). Imported
    /// grids (M11e Serato/Traktor/rekordbox) also flip this flag
    /// because the importer writes `analysis_cache.has_active_grid
    /// = 1` for the fingerprint as it lands the imported grid.
    pub is_analyzed: bool,
    /// `true` when at least two `track_keys` rows for this track
    /// hold non-equivalent Camelot families per the relative-major-
    /// aware predicate in `dub_spectral::camelot_keys_disagree`
    /// (PRD §8.3.2). Two sources that round to the same Camelot
    /// *number* — e.g. C major (8B) vs A minor (8A) — are not
    /// flagged (legitimate K-K template ambiguity). Parallel
    /// disagreements — e.g. C major (8B) vs C minor (5A) — are.
    /// In M11c.2 only the `auto` source writes; this flag will
    /// always be `false` until M11e (Serato importer) lands a
    /// second source. The plumbing ships now so M11e is a pure
    /// data-load milestone with no UI changes required.
    pub key_disagreement: bool,
    /// M11d.7: locked grids skip auto re-analysis on reload.
    pub grid_locked: bool,
    /// M11d.7: LSQ drift slope (ms/min) for the ⚠ indicator.
    pub grid_drift_quality: Option<f32>,
}

/// Sort columns the M11d.2 browser table header can drive.
///
/// Constrained to an enum (rather than an open string) so user
/// input never reaches the SQL string — only the `sql_column`
/// helper below maps an enum variant to a column expression.
/// Every variant resolves to a column that exists in
/// `TRACK_ROW_SELECT` so SQLite's planner picks it up without a
/// re-prepare on each direction toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSortKey {
    /// `tracks.created_at` — the natural "import order" sort.
    /// Default in [`Library::list_tracks`].
    CreatedAt,
    /// Display title. id3 source preferred over filename per PRD
    /// §8.1; expression matches the COALESCE chain in
    /// `TRACK_ROW_SELECT` so SQLite reuses the prepared statement
    /// across direction flips.
    Title,
    /// Display artist. id3 source preferred over filename per PRD
    /// §8.1.
    Artist,
    /// Album name (id3 source).
    Album,
    /// BPM from the active beat grid (`track_beatgrids` where
    /// `is_active = 1`). `None` BPMs sort last in both directions.
    Bpm,
    /// Duration in milliseconds.
    Duration,
    /// Year (filename source preferred over id3).
    Year,
    /// Composer (id3 source).
    Composer,
    /// Track number (id3 source).
    TrackNumber,
}

impl TrackSortKey {
    /// SQL column expression for this sort key. Every value is
    /// either a stable canonical column or the same COALESCE
    /// chain that `TRACK_ROW_SELECT` exposes — keeping the
    /// expressions identical means SQLite's expression cache can
    /// re-use the prepared statement across direction flips.
    fn sql_column(self) -> &'static str {
        match self {
            Self::CreatedAt => "t.created_at",
            Self::Title => "COALESCE(i3.title, fn.title)",
            Self::Artist => "COALESCE(i3.artist, fn.artist)",
            Self::Album => "i3.album",
            Self::Bpm => "ag.bpm",
            Self::Duration => "t.duration_ms",
            Self::Year => "COALESCE(fn.year, i3.year)",
            Self::Composer => "i3.composer",
            Self::TrackNumber => "i3.track_number",
        }
    }
}

/// Canonical SELECT for a [`TrackRow`]. The COALESCE chains
/// implement the §8.1 source-priority for the display fields. We
/// LEFT JOIN both metadata sources (filename + id3) so a track
/// with only one source still surfaces correctly. `MIN(source)`
/// across the per-source rows produces a deterministic "best"
/// source label until a real source-priority column lands at
/// M11e.
///
/// `pf.*` is a single-row subquery exposing the most-recently-
/// confirmed file for the track (ordered by `last_seen_at DESC`).
/// A track with zero file rows leaves these columns NULL, which
/// the browser renders as "missing"; a track with multiple file
/// rows resolves to the last one we touched. Volume mount point
/// is fetched via a correlated subquery against `volumes` so the
/// JOIN order doesn't reshuffle the per-row cost on huge
/// libraries.
const TRACK_ROW_SELECT: &str = "\
    SELECT t.id, \
           COALESCE(i3.title,    fn.title)    AS title, \
           COALESCE(i3.artist,   fn.artist)   AS artist, \
           i3.album                            AS album, \
           i3.genre                            AS genre, \
           COALESCE(fn.year,     i3.year)     AS year, \
           ag.bpm                              AS bpm, \
           ak.key_notation                     AS key, \
           t.duration_ms                       AS duration_ms, \
           COALESCE(fn.version_token, i3.version_token) AS version_tokens, \
           t.duplicate_link_track_id           AS potential_duplicate_id, \
           ( \
               SELECT MIN(source) FROM track_metadata_source ms \
               WHERE ms.track_id = t.id \
           )                                   AS source, \
           pf.volume_uuid                      AS primary_volume_uuid, \
           ( \
               SELECT v.last_known_mount_point \
               FROM volumes v \
               WHERE v.volume_uuid = pf.volume_uuid \
           )                                   AS primary_volume_mount_point, \
           pf.relative_path                    AS primary_relative_path, \
           CASE WHEN ac.analyzed_at IS NOT NULL THEN 1 ELSE 0 END AS is_analyzed, \
           ( \
               SELECT CASE WHEN COUNT(DISTINCT \
                   CASE substr(key_notation, 1, length(key_notation) - 1) \
                        WHEN '' THEN NULL ELSE \
                          substr(key_notation, 1, length(key_notation) - 1) \
                   END) > 1 THEN 1 ELSE 0 END \
               FROM track_keys tk WHERE tk.track_id = t.id \
           )                                   AS key_disagreement, \
           i3.comment                          AS comment, \
           i3.composer                         AS composer, \
           i3.track_number                     AS track_number, \
           t.grid_locked                       AS grid_locked, \
           t.grid_drift_quality                AS grid_drift_quality \
    FROM tracks t \
    LEFT JOIN track_metadata_source fn \
              ON fn.track_id = t.id AND fn.source = 'filename' \
    LEFT JOIN track_metadata_source i3 \
              ON i3.track_id = t.id AND i3.source = 'id3' \
    LEFT JOIN track_beatgrids ag \
              ON ag.track_id = t.id AND ag.is_active = 1 \
    LEFT JOIN track_keys ak \
              ON ak.track_id = t.id AND ak.is_active = 1 \
    LEFT JOIN analysis_cache ac \
              ON ac.fingerprint_id = t.fingerprint_id \
    LEFT JOIN ( \
        SELECT tf.track_id, tf.volume_uuid, tf.relative_path \
        FROM track_files tf \
        JOIN ( \
            SELECT track_id, MAX(id) AS max_id \
            FROM track_files \
            WHERE last_seen_at = ( \
                SELECT MAX(last_seen_at) FROM track_files tf2 \
                WHERE tf2.track_id = track_files.track_id \
            ) \
            GROUP BY track_id \
        ) latest \
          ON latest.track_id = tf.track_id \
          AND latest.max_id   = tf.id \
    ) pf ON pf.track_id = t.id \
    ";

/// Map a SELECT-shaped row to [`TrackRow`]. Used by every
/// `list_*` / `search_*` / `recently_*` method so column order
/// stays in lockstep with `TRACK_ROW_SELECT`.
fn track_row_from_columns(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackRow> {
    let duration_ms: Option<i64> = r.get(8)?;
    Ok(TrackRow {
        id: r.get(0)?,
        title: r.get(1)?,
        artist: r.get(2)?,
        album: r.get(3)?,
        genre: r.get(4)?,
        year: r.get(5)?,
        bpm: r.get(6)?,
        key: r.get(7)?,
        duration_ms: duration_ms.unwrap_or(0).max(0) as u32,
        version_tokens: r.get(9)?,
        potential_duplicate_id: r.get(10)?,
        source: r
            .get::<_, Option<String>>(11)?
            .unwrap_or_else(|| "unknown".to_string()),
        primary_volume_uuid: r.get(12)?,
        primary_volume_mount_point: r.get(13)?,
        primary_relative_path: r.get(14)?,
        is_analyzed: {
            let flag: i64 = r.get(15)?;
            flag != 0
        },
        key_disagreement: {
            let flag: i64 = r.get(16)?;
            flag != 0
        },
        comment: r.get(17)?,
        composer: r.get(18)?,
        track_number: r.get(19)?,
        grid_locked: {
            let flag: i64 = r.get(20)?;
            flag != 0
        },
        grid_drift_quality: r.get(21)?,
    })
}

/// Collect a row-mapped iterator into `Vec<TrackRow>`, propagating
/// the first SQL error with the calling context tag for `Library
/// Error::Sqlite`.
fn collect_track_rows<I>(rows: I, context: &'static str) -> Result<Vec<TrackRow>>
where
    I: Iterator<Item = rusqlite::Result<TrackRow>>,
{
    let mut out = Vec::new();
    for row in rows {
        let row = row.map_err(|e| LibraryError::sqlite(context, e))?;
        out.push(row);
    }
    Ok(out)
}

/// Build an FTS5 MATCH expression from a free-text query per PRD
/// §8.5.4. Whitespace-separated tokens are ANDed; each token is
/// suffix-matched (`workin*` hits `Workinonit`). Tokens shorter
/// than 2 ASCII chars are dropped to avoid noise on a 100k-track
/// library. Returns the empty string when the input yields no
/// usable tokens; callers treat that as "no search → no results".
fn build_fts_query(query: &str) -> String {
    let mut tokens = Vec::new();
    for raw in query.split_whitespace() {
        // FTS5's syntax is unhappy with bare quotes and dashes
        // glued to tokens; strip them before suffix-matching.
        let cleaned: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '\'' || *c == '-' || *c == '.')
            .collect();
        let trimmed = cleaned.trim_matches(|c: char| !c.is_alphanumeric());
        if trimmed.chars().count() < 2 {
            continue;
        }
        tokens.push(format!("\"{}\"*", trimmed.replace('"', "")));
    }
    tokens.join(" AND ")
}

/// A handle to an open Dub library database. Owns one SQLite
/// connection in WAL mode with PRAGMAs applied per
/// `docs/LIBRARY-SCHEMA.md`.
pub struct Library {
    conn: Connection,
    db_path: PathBuf,
    /// PRD-BEATS C1 (round 4) — per-Library override for the
    /// waveform-sidecar cache directory. `None` falls back to
    /// [`crate::paths::default_waveforms_cache_dir`] (i.e.
    /// `~/Library/Caches/Dub/waveforms/`). Tests that exercise
    /// `analyze_track` set this to a per-test tempdir so the
    /// suite stays hermetic against concurrent fp-id collisions.
    waveforms_cache_dir_override: Option<PathBuf>,
    /// Owned tempdir backing
    /// [`Self::waveforms_cache_dir_override`] when the override
    /// was minted by [`Self::open_in_memory`]. Stored on the
    /// struct so the dir is deleted when the Library is dropped
    /// (i.e. when the test ends). `None` for libraries opened
    /// via `open_default` / `open_at` (those use the platform
    /// cache and want it persisted across runs).
    _owned_waveforms_tempdir: Option<tempfile::TempDir>,
}

/// A fingerprint row read back from the `fingerprints` table.
/// Carries the deserialised [`Fingerprint`] alongside the row id and
/// the supplementary columns the M11b dedupe pipeline doesn't strictly
/// need but later analysis (M11c filesystem scanner, M11e Serato
/// importer) does.
#[derive(Debug, Clone)]
pub struct StoredFingerprint {
    /// `fingerprints.id` — the primary key. This is what
    /// `tracks.fingerprint_id` references.
    pub id: i64,
    /// The fingerprint itself, ready for [`dub_fingerprint::similarity`].
    pub fingerprint: Fingerprint,
    /// Sample rate of the source audio at fingerprint time. Optional
    /// because the importer may not yet know it (e.g. when computing
    /// fingerprints from a pre-decoded buffer without metadata).
    pub sample_rate: Option<u32>,
    /// Channel count of the source audio at fingerprint time.
    pub channel_count: Option<u32>,
    /// File size in bytes for the source file. Used as a fast first-
    /// pass dedupe filter (different sizes → almost certainly
    /// different recordings, and we can skip the Hamming compare).
    pub file_size: Option<u64>,
}

/// One Dub-crate node for the M11d-next "Dub Crates" source-tree
/// section (PRD §8.5.1). User-created, editable, nestable, and
/// persisted in the `crates` / `crate_tracks` tables. `track_count`
/// is the number of direct members (it does **not** roll up child
/// crates — the sidebar shows the count of tracks the user dragged
/// onto this node specifically).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateRow {
    /// `crates.id` primary key.
    pub id: i64,
    /// User-facing crate name. Unique among siblings sharing the
    /// same `parent_id`.
    pub name: String,
    /// Parent crate id for nesting, or `None` for a top-level crate.
    pub parent_id: Option<i64>,
    /// Number of tracks directly in this crate (not counting
    /// descendants).
    pub track_count: u64,
}

impl Library {
    /// Open (creating if missing) the library at the default platform
    /// path. On macOS this is `~/Library/Application Support/Dub/library.sqlite`.
    /// Runs every outstanding schema migration before returning.
    pub fn open_default() -> Result<Self> {
        let path = default_library_db_path()?;
        Self::open_at(&path)
    }

    /// Open (creating if missing) the library at an explicit path.
    /// Used by tests (`tempfile`-backed DBs) and CLI tools that
    /// pass `--library /custom/path.sqlite`.
    ///
    /// Returns `LibraryError::SchemaTooNew` if the on-disk
    /// `schema_version` is higher than this binary supports.
    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| LibraryError::io(parent, e))?;
            }
        }
        let mut conn = Connection::open(path).map_err(|e| LibraryError::sqlite("open", e))?;
        open_and_migrate(&mut conn)?;
        Ok(Self {
            conn,
            db_path: path.to_path_buf(),
            waveforms_cache_dir_override: None,
            _owned_waveforms_tempdir: None,
        })
    }

    /// Open an in-memory database (`:memory:`). Migrations applied;
    /// used by tests that never touch the disk.
    ///
    /// PRD-BEATS C1 (round 4): in-memory libraries also mint their
    /// own per-Library tempdir for waveform-sidecar writes. Without
    /// this, every test that calls `analyze_track` would write
    /// into the developer's `~/Library/Caches/Dub/waveforms/` and
    /// races between parallel tests sharing the same
    /// `fingerprint_id` (in-memory DBs reset the autoincrement
    /// counter) would cause flaky cache content.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn =
            Connection::open_in_memory().map_err(|e| LibraryError::sqlite("open_in_memory", e))?;
        open_and_migrate(&mut conn)?;
        let tempdir =
            tempfile::tempdir().map_err(|e| LibraryError::io(Path::new("<tempdir>"), e))?;
        let dir_path = tempdir.path().to_path_buf();
        Ok(Self {
            conn,
            db_path: PathBuf::from(":memory:"),
            waveforms_cache_dir_override: Some(dir_path),
            _owned_waveforms_tempdir: Some(tempdir),
        })
    }

    /// Redirect waveform-sidecar writes to `dir` instead of the
    /// platform default. Returns a new Library with the override
    /// applied. Used by the dub-library test suite (so
    /// concurrent tests don't collide on `~/Library/Caches/...`)
    /// and reserved for a future Preferences "Cache location"
    /// setting.
    #[must_use]
    pub fn with_waveforms_cache_dir(mut self, dir: PathBuf) -> Self {
        self.waveforms_cache_dir_override = Some(dir);
        self._owned_waveforms_tempdir = None;
        self
    }

    /// Resolve the waveform-sidecar cache directory, honouring
    /// any per-Library override set via
    /// [`Self::with_waveforms_cache_dir`].
    pub(crate) fn waveforms_cache_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = &self.waveforms_cache_dir_override {
            std::fs::create_dir_all(dir).map_err(|e| LibraryError::io(dir, e))?;
            return Ok(dir.clone());
        }
        crate::paths::default_waveforms_cache_dir()
    }

    /// Read-only access to the on-disk path the library was opened
    /// from. Used by callers that want to surface the location to
    /// the user (Preferences → "Library location").
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Read-only access to the underlying connection. Exposed for
    /// integration tests and for the migration runner; M11b will
    /// replace direct connection access with typed accessor methods
    /// per repository.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Mutable access to the underlying connection. Used by the
    /// importer and by tests. M11b will likewise narrow this.
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Register (or refresh) a volume in the `volumes` table.
    ///
    /// Idempotent: re-registering a known UUID updates the
    /// mount-point and `last_seen_at` without touching the rest of
    /// the row, which means the M11c filesystem scanner can call
    /// this on every track-file registration without worrying about
    /// row count.
    pub fn upsert_volume(&self, volume: &DiscoveredVolume) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO volumes \
                 (volume_uuid, display_name, last_known_mount_point, last_seen_at, is_internal) \
                 VALUES (?1, ?2, ?3, strftime('%s','now'), ?4) \
                 ON CONFLICT(volume_uuid) DO UPDATE SET \
                     display_name           = excluded.display_name, \
                     last_known_mount_point = excluded.last_known_mount_point, \
                     last_seen_at           = excluded.last_seen_at, \
                     is_internal            = excluded.is_internal",
                params![
                    volume.volume_uuid,
                    volume.display_name,
                    volume.mount_point.to_string_lossy(),
                    volume.is_internal as i32,
                ],
            )
            .map_err(|e| LibraryError::sqlite("upsert_volume", e))?;
        Ok(())
    }

    /// Insert a freshly computed fingerprint into the `fingerprints`
    /// table and return the row's id. M11b's dedupe pipeline calls
    /// this when registering a new canonical recording; the returned
    /// id is what `tracks.fingerprint_id` points at.
    ///
    /// We do not deduplicate the `fingerprints` table at the SQL
    /// level — two different canonical tracks may produce two
    /// fingerprint rows even though their Hamming distance is tiny.
    /// The collapsing happens at the `tracks` layer via the §8.1
    /// dedupe decision.
    pub fn upsert_fingerprint(
        &self,
        fingerprint: &Fingerprint,
        sample_rate: Option<u32>,
        channel_count: Option<u32>,
        file_size: Option<u64>,
    ) -> Result<i64> {
        let blob = fingerprint.to_blob();
        self.conn
            .execute(
                "INSERT INTO fingerprints \
                 (chromaprint_blob, duration_ms, file_size, sample_rate, channel_count, computed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'))",
                params![
                    blob,
                    fingerprint.duration_ms() as i64,
                    file_size.map(|v| v as i64),
                    sample_rate.map(|v| v as i64),
                    channel_count.map(|v| v as i64),
                ],
            )
            .map_err(|e| LibraryError::sqlite("insert_fingerprint", e))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Return every stored fingerprint whose `duration_ms` falls
    /// within `[duration_ms - delta_ms, duration_ms + delta_ms]`.
    /// This is the M11c importer's fast first-pass dedupe filter:
    /// the duration window is cheap to query (the
    /// `idx_fingerprints_duration` index makes it index-only) and
    /// dramatically reduces the number of candidates we Hamming-
    /// compare. The §8.1 dedupe-merge threshold is 200 ms; the
    /// caller typically passes that, but loosening the window for
    /// "potential duplicate" detection is supported.
    pub fn find_fingerprint_neighbours(
        &self,
        duration_ms: u32,
        delta_ms: u32,
    ) -> Result<Vec<StoredFingerprint>> {
        let lo = duration_ms.saturating_sub(delta_ms) as i64;
        let hi = duration_ms.saturating_add(delta_ms) as i64;
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT id, chromaprint_blob, duration_ms, sample_rate, channel_count, file_size \
                 FROM fingerprints \
                 WHERE duration_ms BETWEEN ?1 AND ?2",
            )
            .map_err(|e| LibraryError::sqlite("prepare_find_fingerprint_neighbours", e))?;
        let rows = stmt
            .query_map(params![lo, hi], |r| {
                let id: i64 = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                let duration_ms: i64 = r.get(2)?;
                let sample_rate: Option<i64> = r.get(3)?;
                let channel_count: Option<i64> = r.get(4)?;
                let file_size: Option<i64> = r.get(5)?;
                Ok((id, blob, duration_ms, sample_rate, channel_count, file_size))
            })
            .map_err(|e| LibraryError::sqlite("query_find_fingerprint_neighbours", e))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, blob, duration_ms, sample_rate, channel_count, file_size) =
                row.map_err(|e| LibraryError::sqlite("row_find_fingerprint_neighbours", e))?;
            let fp = Fingerprint::from_blob(&blob, duration_ms as u32).map_err(|e| {
                LibraryError::Sqlite {
                    context: "find_fingerprint_neighbours_blob",
                    source: rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                }
            })?;
            out.push(StoredFingerprint {
                id,
                fingerprint: fp,
                sample_rate: sample_rate.map(|v| v as u32),
                channel_count: channel_count.map(|v| v as u32),
                file_size: file_size.map(|v| v as u64),
            });
        }
        Ok(out)
    }

    /// Insert a canonical `tracks` row. The caller supplies a
    /// freshly minted UUID (typically `uuid::Uuid::new_v4()`); we
    /// don't generate it here so the caller can record the UUID
    /// in their in-memory work queue before any SQL fires.
    /// `duplicate_link_track_id` is `Some(other_uuid)` for sibling-
    /// version registration; `None` otherwise.
    ///
    /// Both `fingerprint_id` and `duration_ms` are nullable. The
    /// M11c.4 lazy-fingerprint model uses `None` for both at import
    /// time and fills them in on the first deck-load via
    /// [`Library::attach_fingerprint`]. Importers that *do* have a
    /// fingerprint at registration time (e.g. the M11d.4 Relocate
    /// path) pass `Some(...)` as before.
    pub fn insert_track(
        &self,
        track_uuid: &str,
        fingerprint_id: Option<i64>,
        duration_ms: Option<u32>,
        duplicate_link_track_id: Option<&str>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO tracks \
                 (id, fingerprint_id, duration_ms, duplicate_link_track_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, strftime('%s','now'), strftime('%s','now'))",
                params![
                    track_uuid,
                    fingerprint_id,
                    duration_ms.map(|v| v as i64),
                    duplicate_link_track_id,
                ],
            )
            .map_err(|e| LibraryError::sqlite("insert_track", e))?;
        Ok(())
    }

    /// Attach a freshly-computed fingerprint to a track that was
    /// imported with `fingerprint_id = NULL` under the M11c.4 lazy
    /// model. Idempotent: if the track already has a fingerprint
    /// (e.g. a concurrent analyse won the race) the existing row
    /// is left in place and `Ok(false)` is returned. Otherwise the
    /// fingerprint id and duration are written and `Ok(true)`.
    ///
    /// The `duration_ms` parameter is the authoritative duration
    /// measured during decode — the same value used to populate
    /// `fingerprints.duration_ms`. We update `tracks.duration_ms`
    /// in the same statement so the browser's duration column
    /// stops showing "—" the instant analysis completes.
    pub fn attach_fingerprint(
        &self,
        track_uuid: &str,
        fingerprint_id: i64,
        duration_ms: u32,
    ) -> Result<bool> {
        let updated = self
            .conn
            .execute(
                "UPDATE tracks \
                    SET fingerprint_id = ?1, \
                        duration_ms    = ?2, \
                        updated_at     = strftime('%s','now') \
                  WHERE id = ?3 \
                    AND fingerprint_id IS NULL",
                params![fingerprint_id, duration_ms as i64, track_uuid],
            )
            .map_err(|e| LibraryError::sqlite("attach_fingerprint", e))?;
        Ok(updated > 0)
    }

    /// Upsert a `track_files` row for the given canonical track and
    /// `(volume_uuid, relative_path)`. The UNIQUE index on
    /// `(volume_uuid, relative_path)` enforces single-file identity;
    /// re-import refreshes the codec / sample_rate / mtime fields
    /// without duplicating rows.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_track_file(
        &self,
        track_uuid: &str,
        volume_uuid: &str,
        relative_path: &str,
        codec: Option<&str>,
        sample_rate: Option<u32>,
        bit_depth: Option<u32>,
        channel_count: Option<u32>,
        file_size: Option<u64>,
        mtime: Option<i64>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO track_files \
                 (track_id, volume_uuid, relative_path, codec, sample_rate, bit_depth, \
                  channel_count, file_size, mtime, last_seen_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%s','now')) \
                 ON CONFLICT(volume_uuid, relative_path) DO UPDATE SET \
                     track_id      = excluded.track_id, \
                     codec         = excluded.codec, \
                     sample_rate   = excluded.sample_rate, \
                     bit_depth     = excluded.bit_depth, \
                     channel_count = excluded.channel_count, \
                     file_size     = excluded.file_size, \
                     mtime         = excluded.mtime, \
                     last_seen_at  = strftime('%s','now')",
                params![
                    track_uuid,
                    volume_uuid,
                    relative_path,
                    codec,
                    sample_rate.map(|v| v as i64),
                    bit_depth.map(|v| v as i64),
                    channel_count.map(|v| v as i64),
                    file_size.map(|v| v as i64),
                    mtime,
                ],
            )
            .map_err(|e| LibraryError::sqlite("upsert_track_file", e))?;
        Ok(())
    }

    /// Find the `track_id` that owns the given
    /// `(volume_uuid, relative_path)`. Used by idempotent re-import:
    /// when a previously-seen file is re-encountered, the
    /// `track_files` lookup tells us the canonical track without
    /// needing to re-decode and re-fingerprint.
    pub fn find_track_file_owner(
        &self,
        volume_uuid: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT track_id FROM track_files \
                 WHERE volume_uuid = ?1 AND relative_path = ?2",
                params![volume_uuid, relative_path],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("find_track_file_owner", e))
    }

    /// Upsert a per-source metadata row. The UNIQUE index on
    /// `(track_id, source)` means re-import overwrites the existing
    /// row (refreshes the metadata) rather than duplicating it.
    /// Empty strings in any field are stored as `NULL` to match the
    /// schema's "absent = NULL" convention.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_metadata_source(
        &self,
        track_uuid: &str,
        source: &str,
        artist: Option<&str>,
        title: Option<&str>,
        album: Option<&str>,
        genre: Option<&str>,
        comment: Option<&str>,
        composer: Option<&str>,
        year: Option<i32>,
        track_number: Option<i32>,
        bpm: Option<f64>,
        key: Option<&str>,
        gain_db: Option<f64>,
        rating: Option<i32>,
        version_token: Option<&str>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO track_metadata_source \
                 (track_id, source, artist, title, album, genre, comment, composer, year, \
                  track_number, bpm, key, gain_db, rating, version_token, imported_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                         strftime('%s','now')) \
                 ON CONFLICT(track_id, source) DO UPDATE SET \
                     artist        = excluded.artist, \
                     title         = excluded.title, \
                     album         = excluded.album, \
                     genre         = excluded.genre, \
                     comment       = excluded.comment, \
                     composer      = excluded.composer, \
                     year          = excluded.year, \
                     track_number  = excluded.track_number, \
                     bpm           = excluded.bpm, \
                     key           = excluded.key, \
                     gain_db       = excluded.gain_db, \
                     rating        = excluded.rating, \
                     version_token = excluded.version_token, \
                     imported_at   = strftime('%s','now')",
                params![
                    track_uuid,
                    source,
                    nonempty(artist),
                    nonempty(title),
                    nonempty(album),
                    nonempty(genre),
                    nonempty(comment),
                    nonempty(composer),
                    year,
                    track_number,
                    bpm,
                    nonempty(key),
                    gain_db,
                    rating,
                    nonempty(version_token),
                ],
            )
            .map_err(|e| LibraryError::sqlite("upsert_metadata_source", e))?;
        Ok(())
    }

    /// Record a deck-load event in `play_history`. Backs the
    /// "Recently Played" smart crate (§8.5.2) and the v1.x
    /// Played From / Played Into side panel. `deck` is 0 (= A)
    /// or 1 (= B); `timestamp_ms` is unix-millis (the caller is
    /// responsible for capturing the wall clock — usually
    /// `Date().timeIntervalSince1970 * 1000`). No-op when
    /// `track_id` doesn't match any canonical row (the FK
    /// constraint rejects the insert; we surface that as a
    /// query error so a stale Apple-side selection doesn't
    /// silently swallow plays).
    pub fn record_load(&self, track_id: &str, deck: u32, timestamp_ms: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO play_history (track_id, deck, event_type, timestamp_ms) \
                 VALUES (?1, ?2, 'load', ?3)",
                params![track_id, deck as i64, timestamp_ms],
            )
            .map_err(|e| LibraryError::sqlite("record_load", e))?;
        Ok(())
    }

    // === M11d.4 missing-files scanner =====================================

    /// List `track_files` rows for the background scanner per
    /// PRD §8.5.5. Returns a batch ordered by "stalest first" —
    /// `last_checked_at ASC NULLS FIRST` so rows that were never
    /// checked (just imported) get the earliest priority. The
    /// Apple shell calls this with a small `batch_size` (typically
    /// 100) on a low-priority interval, then for each result
    /// runs `access(absolute_path)` and calls `mark_file_state`.
    /// Rate-limiting is the caller's responsibility — the SQL is
    /// stateless w.r.t. how often you ask.
    pub fn list_files_for_scan(&self, batch_size: u32) -> Result<Vec<FileScanRow>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT tf.id, tf.track_id, tf.volume_uuid, tf.relative_path, \
                        tf.is_missing, v.last_known_mount_point \
                 FROM track_files tf \
                 LEFT JOIN volumes v ON v.volume_uuid = tf.volume_uuid \
                 ORDER BY tf.last_checked_at IS NOT NULL, tf.last_checked_at ASC \
                 LIMIT ?1",
            )
            .map_err(|e| LibraryError::sqlite("prepare_list_files_for_scan", e))?;
        let rows = stmt
            .query_map(params![batch_size as i64], |r| {
                let id: i64 = r.get(0)?;
                let was_missing: i64 = r.get(4)?;
                Ok(FileScanRow {
                    file_id: id,
                    track_id: r.get(1)?,
                    volume_uuid: r.get(2)?,
                    relative_path: r.get(3)?,
                    was_missing: was_missing != 0,
                    mount_point: r.get(5)?,
                })
            })
            .map_err(|e| LibraryError::sqlite("query_list_files_for_scan", e))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| LibraryError::sqlite("collect_list_files_for_scan", e))?);
        }
        Ok(out)
    }

    /// Stamp a `track_files` row with the scanner's verdict.
    /// `is_missing = true` when `access()` failed; `false` when
    /// the file is present. `last_checked_at` is always updated
    /// to the supplied unix-seconds value so the rate-limiter
    /// can skip recently-checked rows.
    pub fn mark_file_state(
        &self,
        file_id: i64,
        is_missing: bool,
        last_checked_at: i64,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE track_files \
                 SET is_missing = ?1, last_checked_at = ?2 \
                 WHERE id = ?3",
                params![if is_missing { 1 } else { 0 }, last_checked_at, file_id],
            )
            .map_err(|e| LibraryError::sqlite("mark_file_state", e))?;
        Ok(())
    }

    /// Count of tracks the browser should flag as missing —
    /// canonical tracks where *every* `track_files` row is
    /// `is_missing = 1` (or the track has no files at all). A
    /// track with one missing and one healthy file is *not*
    /// missing because the user can still reach it through the
    /// healthy path. Drives the M11d browser footer per §8.5.5.
    pub fn missing_track_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tracks t \
                 WHERE NOT EXISTS ( \
                    SELECT 1 FROM track_files tf \
                    WHERE tf.track_id = t.id AND tf.is_missing = 0 \
                 )",
                [],
                |r| r.get(0),
            )
            .map_err(|e| LibraryError::sqlite("missing_track_count", e))?;
        Ok(n.max(0) as u64)
    }

    /// List missing canonical tracks for the M11d.4 Relocate
    /// panel. Returns up to `limit` rows, each carrying enough
    /// signal for the matcher: original filename (last path
    /// component of the most-recent `track_files.relative_path`),
    /// duration in milliseconds, and `fingerprint_id` so the
    /// matcher can pull the stored Chromaprint blob via
    /// [`Library::load_fingerprint`] without re-decoding.
    pub fn list_missing_tracks(&self, limit: u32) -> Result<Vec<MissingTrack>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT t.id, t.fingerprint_id, t.duration_ms, \
                        ( \
                            SELECT tf.relative_path FROM track_files tf \
                            WHERE tf.track_id = t.id \
                            ORDER BY tf.last_seen_at DESC, tf.id DESC LIMIT 1 \
                        ) AS last_relative_path \
                 FROM tracks t \
                 WHERE NOT EXISTS ( \
                    SELECT 1 FROM track_files tf \
                    WHERE tf.track_id = t.id AND tf.is_missing = 0 \
                 ) \
                   AND t.fingerprint_id IS NOT NULL \
                 ORDER BY t.created_at ASC LIMIT ?1",
            )
            .map_err(|e| LibraryError::sqlite("prepare_list_missing_tracks", e))?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                let dur: i64 = r.get(2)?;
                let path: Option<String> = r.get(3)?;
                let filename = path.as_deref().and_then(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .and_then(|f| f.to_str())
                        .map(str::to_string)
                });
                Ok(MissingTrack {
                    track_id: r.get(0)?,
                    fingerprint_id: r.get(1)?,
                    duration_ms: dur.max(0) as u32,
                    last_relative_path: path,
                    last_filename: filename,
                })
            })
            .map_err(|e| LibraryError::sqlite("query_list_missing_tracks", e))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| LibraryError::sqlite("collect_list_missing_tracks", e))?);
        }
        Ok(out)
    }

    /// Register a new on-disk location for an existing canonical
    /// track, used by the M11d.4 Relocate panel after the matcher
    /// has confirmed the file's identity. Inserts a fresh
    /// `track_files` row (rather than mutating an existing one)
    /// so the original location stays on record — when the
    /// touring SSD comes back online, the previous file row is
    /// already there, the scanner flips it back to
    /// `is_missing = 0`, and the user has both locations
    /// available. PRD §8.5.5: "Metadata is never deleted when a
    /// file goes missing."
    #[allow(clippy::too_many_arguments)]
    pub fn relocate_track(
        &self,
        track_id: &str,
        volume_uuid: &str,
        relative_path: &str,
        codec: Option<&str>,
        sample_rate: Option<u32>,
        channel_count: Option<u32>,
        file_size: Option<u64>,
        mtime: Option<i64>,
    ) -> Result<()> {
        // upsert_track_file already handles the
        // (volume_uuid, relative_path) UNIQUE conflict by
        // refreshing the existing row; relocate uses the same
        // path so a re-relocate to a path Dub already knew about
        // becomes a no-op refresh. last_checked_at is stamped to
        // "now" + is_missing reset to 0 so the row leaves
        // "missing" status immediately.
        self.upsert_track_file(
            track_id,
            volume_uuid,
            relative_path,
            codec,
            sample_rate,
            None,
            channel_count,
            file_size,
            mtime,
        )?;
        self.conn
            .execute(
                "UPDATE track_files \
                 SET is_missing = 0, last_checked_at = strftime('%s','now') \
                 WHERE track_id = ?1 AND volume_uuid = ?2 AND relative_path = ?3",
                params![track_id, volume_uuid, relative_path],
            )
            .map_err(|e| LibraryError::sqlite("relocate_track_clear_missing", e))?;
        Ok(())
    }

    /// Total canonical-track count. Backs the M11d browser footer
    /// and the §8.5 source-tree "All Tracks" badge.
    pub fn track_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
            .map_err(|e| LibraryError::sqlite("track_count", e))?;
        Ok(n.max(0) as u64)
    }

    /// List canonical tracks for the M11d browser "All Tracks"
    /// surface. Returns the assembled [`TrackRow`] (filename →
    /// id3 priority chain per §8.1) sliced by `limit` / `offset`.
    /// Ordering is stable by `tracks.created_at` ascending so the
    /// browser doesn't reshuffle on every re-open.
    ///
    /// Convenience wrapper over [`list_tracks_sorted`] for callers
    /// who don't care about column sort.
    pub fn list_tracks(&self, limit: u32, offset: u32) -> Result<Vec<TrackRow>> {
        self.list_tracks_sorted(limit, offset, TrackSortKey::CreatedAt, true)
    }

    /// Sortable variant of [`list_tracks`] for the M11d.2 browser
    /// table header. The sort key is picked from the safe-list
    /// enum so user input never reaches the SQL string; this is
    /// the only place in the crate where a sort column is
    /// interpolated. NULL handling is deterministic — NULLs sort
    /// last in both directions so a few missing-tag rows don't
    /// jump to the top when the user clicks "Artist". A stable
    /// secondary key (`t.created_at ASC`) keeps the order
    /// reproducible across re-queries.
    pub fn list_tracks_sorted(
        &self,
        limit: u32,
        offset: u32,
        sort: TrackSortKey,
        ascending: bool,
    ) -> Result<Vec<TrackRow>> {
        let direction = if ascending { "ASC" } else { "DESC" };
        let column = sort.sql_column();
        // COLLATE NOCASE on text columns so "abba" sorts next to
        // "ABBA", not after the entire lowercase block. Numeric
        // columns ignore the collate hint, so adding it
        // unconditionally is harmless and keeps the SQL shape
        // identical across sort keys.
        let sql = format!(
            "{TRACK_ROW_SELECT} \
             ORDER BY {column} IS NULL, {column} COLLATE NOCASE {direction}, \
                      t.created_at ASC \
             LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| LibraryError::sqlite("prepare_list_tracks_sorted", e))?;
        let rows = stmt
            .query_map(params![limit as i64, offset as i64], track_row_from_columns)
            .map_err(|e| LibraryError::sqlite("query_list_tracks_sorted", e))?;
        collect_track_rows(rows, "list_tracks_sorted")
    }

    /// FTS5-backed substring search per PRD §8.5.4. Whitespace-
    /// separated tokens are ANDed; tokens are wrapped with `*`
    /// suffix-match so a partial query (`workin`) hits `Workinonit`.
    /// Tokens shorter than 2 chars are dropped (single-letter
    /// suffix-matches would produce noise on a 100k-track library).
    /// Quotes are stripped to keep the FTS5 syntax happy.
    pub fn search_tracks(&self, query: &str, limit: u32) -> Result<Vec<TrackRow>> {
        let fts_query = build_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "{TRACK_ROW_SELECT} \
             WHERE t.id IN (\
                SELECT DISTINCT track_id FROM track_metadata_fts \
                WHERE track_metadata_fts MATCH ?1\
             ) \
             ORDER BY t.created_at ASC LIMIT ?2"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| LibraryError::sqlite("prepare_search_tracks", e))?;
        let rows = stmt
            .query_map(params![fts_query, limit as i64], track_row_from_columns)
            .map_err(|e| LibraryError::sqlite("query_search_tracks", e))?;
        collect_track_rows(rows, "search_tracks")
    }

    /// Recently Played smart crate per §8.5.2. Reads `play_history`
    /// for the last `limit` distinct tracks, newest first; only
    /// `event_type = 'load'` rows count (we don't want a single
    /// 5-second play_start to overshadow earlier real loads).
    /// Empty when `play_history` carries no rows (v1.0 day-one
    /// default state until the deck transport actually fires the
    /// history-write hook in a follow-up sub-milestone).
    pub fn recently_played(&self, limit: u32) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{TRACK_ROW_SELECT} \
             JOIN (\
                SELECT track_id, MAX(timestamp_ms) AS last_loaded \
                FROM play_history \
                WHERE event_type = 'load' \
                GROUP BY track_id\
             ) ph ON ph.track_id = t.id \
             ORDER BY ph.last_loaded DESC LIMIT ?1"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| LibraryError::sqlite("prepare_recently_played", e))?;
        let rows = stmt
            .query_map(params![limit as i64], track_row_from_columns)
            .map_err(|e| LibraryError::sqlite("query_recently_played", e))?;
        collect_track_rows(rows, "recently_played")
    }

    /// Just Imported smart crate per §8.5.2. Tracks whose
    /// `tracks.created_at` is >= the given unix-seconds boundary.
    /// Caller chooses the boundary (typically: app-launch time).
    pub fn just_imported(&self, since_unix_secs: i64, limit: u32) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{TRACK_ROW_SELECT} \
             WHERE t.created_at >= ?1 \
             ORDER BY t.created_at DESC LIMIT ?2"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| LibraryError::sqlite("prepare_just_imported", e))?;
        let rows = stmt
            .query_map(
                params![since_unix_secs, limit as i64],
                track_row_from_columns,
            )
            .map_err(|e| LibraryError::sqlite("query_just_imported", e))?;
        collect_track_rows(rows, "just_imported")
    }

    // === Dub crates (M11d-next) ==========================================
    //
    // User-created, editable, nestable crates backed by `crates` /
    // `crate_tracks` (PRD §8.5.1). The split between these (owned,
    // editable) and the read-only `imported_crates` mirror is
    // non-negotiable — see the PRD section. Every mutator bumps
    // `crates.updated_at` so a future "sort by recently-edited"
    // sidebar order has the data it needs. Ordinals are dense
    // (0..n) and rewritten wholesale on reorder; `add_track_to_crate`
    // appends at `MAX(ordinal) + 1`.

    /// List every Dub crate with its direct-member track count, for
    /// the sidebar's "Dub Crates" section. Ordered case-insensitively
    /// by name so the tree is stable across re-opens. Returns the
    /// flat set; the caller reconstructs nesting from `parent_id`.
    pub fn list_crates(&self) -> Result<Vec<CrateRow>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT c.id, c.name, c.parent_crate_id, \
                        COUNT(ct.track_id) AS track_count \
                 FROM crates c \
                 LEFT JOIN crate_tracks ct ON ct.crate_id = c.id \
                 GROUP BY c.id \
                 ORDER BY c.name COLLATE NOCASE ASC",
            )
            .map_err(|e| LibraryError::sqlite("prepare_list_crates", e))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CrateRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    parent_id: r.get(2)?,
                    track_count: r.get::<_, i64>(3)?.max(0) as u64,
                })
            })
            .map_err(|e| LibraryError::sqlite("query_list_crates", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| LibraryError::sqlite("collect_list_crates", e))?);
        }
        Ok(out)
    }

    /// Create a Dub crate and return its new id. `parent_id` nests it
    /// under another crate (or `None` for a top-level crate). The
    /// `UNIQUE (parent_crate_id, name)` constraint maps to
    /// [`LibraryError::CrateNameConflict`] so the UI can show a
    /// friendly "name already exists" message instead of a raw
    /// SQLite string.
    pub fn create_crate(&self, name: &str, parent_id: Option<i64>) -> Result<i64> {
        // SQLite's `UNIQUE (parent_crate_id, name)` does NOT fire for
        // top-level crates: two rows with `parent_crate_id IS NULL`
        // and the same name are "distinct" because NULL != NULL in SQL
        // uniqueness. So we pre-check siblings explicitly (the `IS`
        // operator compares NULLs as equal); the table constraint stays
        // as a backstop for non-NULL parents.
        if self.crate_name_taken(name, parent_id, None)? {
            return Err(LibraryError::CrateNameConflict {
                name: name.to_string(),
            });
        }
        self.conn
            .execute(
                "INSERT INTO crates (name, parent_crate_id, created_at, updated_at) \
                 VALUES (?1, ?2, strftime('%s','now'), strftime('%s','now'))",
                params![name, parent_id],
            )
            .map_err(|e| map_crate_constraint(e, name, "create_crate"))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Rename a Dub crate. Errors with [`LibraryError::CrateNotFound`]
    /// when no crate carries the id, and
    /// [`LibraryError::CrateNameConflict`] on a sibling-name clash.
    pub fn rename_crate(&self, crate_id: i64, new_name: &str) -> Result<()> {
        // Resolve the crate's parent so we can scope the sibling-name
        // pre-check (see `create_crate` for why the table UNIQUE alone
        // isn't enough for top-level crates). `None` parent on a missing
        // id is caught by the row-count check below.
        let parent_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT parent_crate_id FROM crates WHERE id = ?1",
                params![crate_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("rename_crate_lookup_parent", e))?
            .ok_or(LibraryError::CrateNotFound { crate_id })?;
        if self.crate_name_taken(new_name, parent_id, Some(crate_id))? {
            return Err(LibraryError::CrateNameConflict {
                name: new_name.to_string(),
            });
        }
        let changed = self
            .conn
            .execute(
                "UPDATE crates SET name = ?1, updated_at = strftime('%s','now') \
                 WHERE id = ?2",
                params![new_name, crate_id],
            )
            .map_err(|e| map_crate_constraint(e, new_name, "rename_crate"))?;
        if changed == 0 {
            return Err(LibraryError::CrateNotFound { crate_id });
        }
        Ok(())
    }

    /// `true` when a sibling crate (same `parent_id`, NULLs compared
    /// equal via SQL `IS`) already carries `name`, optionally excluding
    /// a crate id (used by rename so a no-op rename onto the same name
    /// doesn't self-conflict).
    fn crate_name_taken(
        &self,
        name: &str,
        parent_id: Option<i64>,
        exclude_id: Option<i64>,
    ) -> Result<bool> {
        let taken: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(\
                    SELECT 1 FROM crates \
                    WHERE name = ?1 \
                      AND parent_crate_id IS ?2 \
                      AND (?3 IS NULL OR id != ?3)\
                 )",
                params![name, parent_id, exclude_id],
                |r| r.get(0),
            )
            .map_err(|e| LibraryError::sqlite("crate_name_taken", e))?;
        Ok(taken)
    }

    /// Delete a Dub crate. `crate_tracks` rows and any child crates
    /// cascade via the schema's `ON DELETE CASCADE`. Errors with
    /// [`LibraryError::CrateNotFound`] when the id is unknown so the
    /// UI can distinguish a no-op from a real delete.
    pub fn delete_crate(&self, crate_id: i64) -> Result<()> {
        let changed = self
            .conn
            .execute("DELETE FROM crates WHERE id = ?1", params![crate_id])
            .map_err(|e| LibraryError::sqlite("delete_crate", e))?;
        if changed == 0 {
            return Err(LibraryError::CrateNotFound { crate_id });
        }
        Ok(())
    }

    /// Append a track to a crate at the next ordinal. Idempotent:
    /// re-adding a track already in the crate is a no-op and returns
    /// `Ok(false)`; a fresh insert returns `Ok(true)`. Validates that
    /// both the crate and the track exist first so the caller gets a
    /// typed [`LibraryError::CrateNotFound`] / [`LibraryError::TrackNotFound`]
    /// instead of a raw foreign-key failure.
    pub fn add_track_to_crate(&self, crate_id: i64, track_id: &str) -> Result<bool> {
        self.ensure_crate_exists(crate_id)?;
        self.ensure_track_exists(track_id)?;
        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO crate_tracks (crate_id, track_id, ordinal, added_at) \
                 VALUES (\
                    ?1, ?2, \
                    (SELECT COALESCE(MAX(ordinal), -1) + 1 \
                       FROM crate_tracks WHERE crate_id = ?1), \
                    strftime('%s','now')\
                 )",
                params![crate_id, track_id],
            )
            .map_err(|e| LibraryError::sqlite("add_track_to_crate", e))?;
        if inserted > 0 {
            self.touch_crate(crate_id)?;
        }
        Ok(inserted > 0)
    }

    /// Remove a track from a crate. Idempotent — removing a track that
    /// isn't a member is a successful no-op. Leaves a gap in the
    /// ordinal sequence; ordinals are only used for relative ordering,
    /// not as dense keys, and the next reorder rewrites them anyway.
    pub fn remove_track_from_crate(&self, crate_id: i64, track_id: &str) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM crate_tracks WHERE crate_id = ?1 AND track_id = ?2",
                params![crate_id, track_id],
            )
            .map_err(|e| LibraryError::sqlite("remove_track_from_crate", e))?;
        if changed > 0 {
            self.touch_crate(crate_id)?;
        }
        Ok(())
    }

    /// Rewrite the member ordering of a crate to match `ordered_track_ids`
    /// (0-based, in the given order). Ids not currently in the crate are
    /// ignored; members absent from the list keep their old ordinal and
    /// therefore sort after the reordered block (the UI always sends the
    /// full member list, so this is a defensive corner). Runs in a single
    /// transaction so a mid-write failure can't leave a half-reordered
    /// crate.
    pub fn set_crate_track_order(&self, crate_id: i64, ordered_track_ids: &[&str]) -> Result<()> {
        self.ensure_crate_exists(crate_id)?;
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| LibraryError::sqlite("begin_set_crate_track_order", e))?;
        {
            let mut stmt = tx
                .prepare_cached(
                    "UPDATE crate_tracks SET ordinal = ?1 \
                     WHERE crate_id = ?2 AND track_id = ?3",
                )
                .map_err(|e| LibraryError::sqlite("prepare_set_crate_track_order", e))?;
            for (ordinal, track_id) in ordered_track_ids.iter().enumerate() {
                stmt.execute(params![ordinal as i64, crate_id, track_id])
                    .map_err(|e| LibraryError::sqlite("set_crate_track_order", e))?;
            }
        }
        tx.execute(
            "UPDATE crates SET updated_at = strftime('%s','now') WHERE id = ?1",
            params![crate_id],
        )
        .map_err(|e| LibraryError::sqlite("touch_set_crate_track_order", e))?;
        tx.commit()
            .map_err(|e| LibraryError::sqlite("commit_set_crate_track_order", e))?;
        Ok(())
    }

    /// List a crate's member tracks in ordinal order, assembled into
    /// the same [`TrackRow`] shape the browser uses everywhere else.
    /// Empty for an empty (or unknown) crate.
    pub fn list_crate_tracks(&self, crate_id: i64) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{TRACK_ROW_SELECT} \
             JOIN crate_tracks ct ON ct.track_id = t.id \
             WHERE ct.crate_id = ?1 \
             ORDER BY ct.ordinal ASC"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| LibraryError::sqlite("prepare_list_crate_tracks", e))?;
        let rows = stmt
            .query_map(params![crate_id], track_row_from_columns)
            .map_err(|e| LibraryError::sqlite("query_list_crate_tracks", e))?;
        collect_track_rows(rows, "list_crate_tracks")
    }

    /// Bump a crate's `updated_at`. Called after every membership
    /// mutation. Silently does nothing for an unknown id (the
    /// mutators that call this have already validated the crate).
    fn touch_crate(&self, crate_id: i64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE crates SET updated_at = strftime('%s','now') WHERE id = ?1",
                params![crate_id],
            )
            .map_err(|e| LibraryError::sqlite("touch_crate", e))?;
        Ok(())
    }

    /// Return [`LibraryError::CrateNotFound`] unless the crate exists.
    fn ensure_crate_exists(&self, crate_id: i64) -> Result<()> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM crates WHERE id = ?1)",
                params![crate_id],
                |r| r.get(0),
            )
            .map_err(|e| LibraryError::sqlite("ensure_crate_exists", e))?;
        if exists {
            Ok(())
        } else {
            Err(LibraryError::CrateNotFound { crate_id })
        }
    }

    /// Return [`LibraryError::TrackNotFound`] unless the track exists.
    fn ensure_track_exists(&self, track_id: &str) -> Result<()> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tracks WHERE id = ?1)",
                params![track_id],
                |r| r.get(0),
            )
            .map_err(|e| LibraryError::sqlite("ensure_track_exists", e))?;
        if exists {
            Ok(())
        } else {
            Err(LibraryError::TrackNotFound {
                track_id: track_id.to_string(),
            })
        }
    }

    /// Resolve a canonical `track_id` to one of its on-disk paths.
    /// Returns the *first* `track_files` row by `last_seen_at`
    /// descending (so the most-recently-confirmed path wins),
    /// joined against the `volumes` table to reconstruct the
    /// absolute path. Returns `None` when the track has no file
    /// rows (deleted from disk) or the volume isn't mounted.
    /// Used by the M11d browser to back drag-and-drop and Space-
    /// load with a real file URL.
    pub fn resolve_track_path(&self, track_id: &str) -> Result<Option<std::path::PathBuf>> {
        let row = self
            .conn
            .query_row(
                "SELECT v.last_known_mount_point, tf.relative_path \
                 FROM track_files tf \
                 JOIN volumes v ON v.volume_uuid = tf.volume_uuid \
                 WHERE tf.track_id = ?1 \
                 ORDER BY tf.last_seen_at DESC, tf.id DESC LIMIT 1",
                params![track_id],
                |r| {
                    let mount: Option<String> = r.get(0)?;
                    let rel: String = r.get(1)?;
                    Ok((mount, rel))
                },
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("resolve_track_path", e))?;
        // `last_known_mount_point` is nullable in the schema — a
        // volume that's currently unmounted has no path to attach.
        // We treat that as "track is unreachable right now" and
        // return None; the M11d.4 missing-files panel will flag it.
        Ok(row.and_then(|(mount, rel)| {
            mount.map(|m| {
                let mut p = std::path::PathBuf::from(m);
                p.push(rel);
                p
            })
        }))
    }

    /// Reverse of [`Self::resolve_track_path`]: given an absolute
    /// on-disk path, return the canonical `tracks.id` when the file
    /// is registered in `track_files`. Used by the Apple shell to
    /// stamp `loadedLibraryTrackId` and fire lazy analysis when a
    /// deck load came from a library drag that didn't update the
    /// browser selection first.
    pub fn track_id_for_absolute_path(&self, path: &Path) -> Result<Option<String>> {
        let volume = crate::volumes::discover_for_path(path)?;
        let relative_path = volume.relative_to(path).ok_or_else(|| LibraryError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path is not under discovered volume mount point",
            ),
        })?;
        self.find_track_file_owner(&volume.volume_uuid, &relative_path)
    }

    /// Resolve a fingerprint row id to `(track_uuid, display_string)`
    /// for one of the tracks pointing at it. Used by the M11c
    /// importer's dedupe step: a `find_fingerprint_neighbours` hit
    /// gives us the candidate fingerprint but not the canonical
    /// track UUID; this method bridges that gap and assembles a
    /// title-or-filename string from the per-source metadata rows
    /// for the version-token check.
    ///
    /// Returns `None` for orphan fingerprints (rows whose owning
    /// track has been deleted but whose fingerprint row survives
    /// — possible during the brief window of a v1.x merge UI).
    /// When multiple tracks reference the same fingerprint id
    /// (only possible via the duplicate_link path), the first by
    /// `tracks.created_at` wins; in normal operation only one row
    /// per fingerprint exists.
    pub fn find_track_owner_by_fingerprint_id(
        &self,
        fingerprint_id: i64,
    ) -> Result<Option<(String, String)>> {
        // Display-string assembly: prefer the filename source's
        // title (carries version tokens verbatim) over id3 (which
        // may have been written by a tag editor that stripped
        // them). The COALESCE chain picks the first non-NULL.
        self.conn
            .query_row(
                "SELECT t.id, \
                        COALESCE(\
                            CASE WHEN fn.artist IS NOT NULL AND fn.title IS NOT NULL \
                                 THEN fn.artist || ' - ' || fn.title \
                                 ELSE fn.title END, \
                            CASE WHEN i3.artist IS NOT NULL AND i3.title IS NOT NULL \
                                 THEN i3.artist || ' - ' || i3.title \
                                 ELSE i3.title END, \
                            '') AS display \
                 FROM tracks t \
                 LEFT JOIN track_metadata_source fn \
                          ON fn.track_id = t.id AND fn.source = 'filename' \
                 LEFT JOIN track_metadata_source i3 \
                          ON i3.track_id = t.id AND i3.source = 'id3' \
                 WHERE t.fingerprint_id = ?1 \
                 ORDER BY t.created_at ASC \
                 LIMIT 1",
                params![fingerprint_id],
                |r| {
                    let uuid: String = r.get(0)?;
                    let display: String = r.get(1)?;
                    Ok((uuid, display))
                },
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("find_track_owner_by_fingerprint_id", e))
    }

    /// Look up a stored fingerprint by its primary key. Used by the
    /// dedupe pipeline to materialise the existing-track side of a
    /// near-match comparison.
    pub fn load_fingerprint(&self, id: i64) -> Result<Option<StoredFingerprint>> {
        self.conn
            .query_row(
                "SELECT chromaprint_blob, duration_ms, sample_rate, channel_count, file_size \
                 FROM fingerprints WHERE id = ?1",
                params![id],
                |r| {
                    let blob: Vec<u8> = r.get(0)?;
                    let duration_ms: i64 = r.get(1)?;
                    let sample_rate: Option<i64> = r.get(2)?;
                    let channel_count: Option<i64> = r.get(3)?;
                    let file_size: Option<i64> = r.get(4)?;
                    Ok((blob, duration_ms, sample_rate, channel_count, file_size))
                },
            )
            .optional()
            .map_err(|e| LibraryError::sqlite("load_fingerprint", e))?
            .map(
                |(blob, duration_ms, sample_rate, channel_count, file_size)| {
                    let fp = Fingerprint::from_blob(&blob, duration_ms as u32).map_err(|e| {
                        // Surface as Sqlite-context error rather than a
                        // separate variant; the BLOB came from the DB
                        // so a malformed value is a corruption-class
                        // condition rather than a typed library error.
                        LibraryError::Sqlite {
                            context: "load_fingerprint_blob",
                            source: rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                        }
                    })?;
                    Ok(StoredFingerprint {
                        id,
                        fingerprint: fp,
                        sample_rate: sample_rate.map(|v| v as u32),
                        channel_count: channel_count.map(|v| v as u32),
                        file_size: file_size.map(|v| v as u64),
                    })
                },
            )
            .transpose()
    }

    /// Look up a volume row by UUID. Returns `None` if the volume
    /// is not registered.
    pub fn find_volume(&self, volume_uuid: &str) -> Result<Option<DiscoveredVolume>> {
        let row = self
            .conn
            .query_row(
                "SELECT volume_uuid, display_name, last_known_mount_point, is_internal \
                 FROM volumes WHERE volume_uuid = ?1",
                params![volume_uuid],
                |r| {
                    let uuid: String = r.get(0)?;
                    let display: String = r.get(1)?;
                    let mount: Option<String> = r.get(2)?;
                    let is_internal: i64 = r.get(3)?;
                    Ok((uuid, display, mount, is_internal))
                },
            )
            .ok();
        Ok(
            row.map(|(uuid, display, mount, is_internal)| DiscoveredVolume {
                volume_uuid: uuid,
                mount_point: mount.map(PathBuf::from).unwrap_or_default(),
                display_name: display,
                is_internal: is_internal != 0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_at_creates_parent_directory_chain() {
        let tmp = tempdir().unwrap();
        // Nest two levels deep to verify `create_dir_all` semantics.
        let path = tmp.path().join("a/b/library.sqlite");
        let lib = Library::open_at(&path).expect("open succeeds and creates parents");
        assert!(path.exists(), "library file must exist after open");
        assert_eq!(lib.db_path(), path);
    }

    #[test]
    fn re_open_is_idempotent() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("library.sqlite");
        drop(Library::open_at(&path).unwrap());
        // Re-open the same DB. Should succeed without re-applying
        // migrations (the runner is itself idempotent but the
        // observable behaviour is "open works twice").
        drop(Library::open_at(&path).unwrap());
    }

    #[test]
    fn upsert_volume_round_trip() {
        let lib = Library::open_in_memory().unwrap();
        let v = DiscoveredVolume {
            volume_uuid: "00112233-4455-6677-8899-aabbccddeeff".to_string(),
            mount_point: PathBuf::from("/Volumes/Touring SSD"),
            display_name: "Touring SSD".to_string(),
            is_internal: false,
        };
        lib.upsert_volume(&v).unwrap();
        let got = lib
            .find_volume(&v.volume_uuid)
            .unwrap()
            .expect("registered");
        assert_eq!(got.volume_uuid, v.volume_uuid);
        assert_eq!(got.display_name, v.display_name);
        assert_eq!(got.mount_point, v.mount_point);
        assert!(!got.is_internal);
    }

    /// Build a small but real fingerprint for round-trip tests.
    /// Mirrors the `tone` helper in `dub-fingerprint::tests`; we
    /// can't import a test-only helper from another crate so the
    /// minimal copy lives here.
    fn fingerprint_for(freq: f32, secs: f32) -> Fingerprint {
        let n = (11025_f32 * secs) as usize;
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / 11025_f32;
            samples.push(0.5 * (2.0 * std::f32::consts::PI * freq * t).sin());
        }
        Fingerprint::compute_from_f32(&samples, 11025, 1).expect("compute")
    }

    #[test]
    fn fingerprint_round_trip_via_sqlite() {
        let lib = Library::open_in_memory().unwrap();
        let fp = fingerprint_for(440.0, 10.0);
        let id = lib
            .upsert_fingerprint(&fp, Some(44_100), Some(2), Some(8_000_000))
            .expect("insert fingerprint");
        let stored = lib
            .load_fingerprint(id)
            .expect("query fingerprint")
            .expect("row exists");
        assert_eq!(stored.id, id);
        assert_eq!(stored.fingerprint, fp);
        assert_eq!(stored.sample_rate, Some(44_100));
        assert_eq!(stored.channel_count, Some(2));
        assert_eq!(stored.file_size, Some(8_000_000));
    }

    #[test]
    fn load_fingerprint_returns_none_for_missing_id() {
        let lib = Library::open_in_memory().unwrap();
        assert!(lib.load_fingerprint(999).unwrap().is_none());
    }

    #[test]
    fn upsert_volume_updates_mount_point_on_remount() {
        // Touring SSD re-plugged into a different USB slot mounts
        // at a different path. The UUID is invariant; the mount
        // point updates. The upsert path must reflect that without
        // creating a duplicate row.
        let lib = Library::open_in_memory().unwrap();
        let uuid = "deadbeef-0000-0000-0000-000000000000";
        lib.upsert_volume(&DiscoveredVolume {
            volume_uuid: uuid.to_string(),
            mount_point: PathBuf::from("/Volumes/Touring SSD"),
            display_name: "Touring SSD".to_string(),
            is_internal: false,
        })
        .unwrap();
        lib.upsert_volume(&DiscoveredVolume {
            volume_uuid: uuid.to_string(),
            mount_point: PathBuf::from("/Volumes/Touring SSD 1"),
            display_name: "Touring SSD".to_string(),
            is_internal: false,
        })
        .unwrap();
        let count: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM volumes WHERE volume_uuid = ?1",
                params![uuid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "upsert must not duplicate rows");
        let got = lib.find_volume(uuid).unwrap().unwrap();
        assert_eq!(got.mount_point, PathBuf::from("/Volumes/Touring SSD 1"));
    }

    /// Seed a library with N synthetic tracks (filename source only;
    /// fingerprint, track_file row, both metadata rows). Returns the
    /// minted UUIDs in insertion order so tests can assert on them.
    /// Used by every M11d.1 browser-query test below.
    fn seed_tracks(lib: &Library, titles: &[&str]) -> Vec<String> {
        // Register a synthetic volume so the (volume_uuid, relative_path)
        // FK on track_files holds.
        let volume = DiscoveredVolume {
            volume_uuid: "11111111-1111-1111-1111-111111111111".into(),
            mount_point: PathBuf::from("/"),
            display_name: "Macintosh HD".into(),
            is_internal: true,
        };
        lib.upsert_volume(&volume).unwrap();

        // Each track needs a unique fingerprint blob so the
        // `fingerprints` table doesn't reject duplicates via the
        // future fingerprints-uniqueness index; today's schema does
        // not enforce that but the rows are still semantically
        // distinct.
        let mut uuids = Vec::new();
        for (i, title) in titles.iter().enumerate() {
            let id = uuid::Uuid::new_v4().to_string();
            let fp_blob: Vec<u8> = (0..32_u8).map(|b| b.wrapping_add(i as u8)).collect();
            let fp = dub_fingerprint::Fingerprint::from_blob(&fp_blob, 10_000).unwrap();
            let fp_id = lib
                .upsert_fingerprint(&fp, Some(44_100), Some(1), Some(123_456))
                .unwrap();
            lib.insert_track(&id, Some(fp_id), Some(10_000), None)
                .unwrap();
            lib.upsert_track_file(
                &id,
                &volume.volume_uuid,
                &format!("test/{title}.wav"),
                Some("wav"),
                Some(44_100),
                None,
                Some(1),
                Some(123_456),
                Some(1_700_000_000 + i as i64),
            )
            .unwrap();
            lib.upsert_metadata_source(
                &id,
                "filename",
                Some("Test Artist"),
                Some(title),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            // ID3 source has only album + bpm so the COALESCE chains
            // exercise both sources independently. id3.title is
            // left NULL on purpose so the `list_tracks_*` sort /
            // search / pagination tests can keep asserting against
            // the raw `title` input string — the COALESCE falls
            // through to the filename row in that case regardless
            // of which source the priority chain prefers. The
            // priority-chain tests below override id3.title
            // explicitly to exercise PRD §8.1's `id3 > filename`
            // contract.
            lib.upsert_metadata_source(
                &id,
                "id3",
                None,
                None,
                Some("Test Album"),
                Some("Test Genre"),
                None,
                None,
                None,
                None,
                Some(123.45),
                None,
                None,
                None,
                None,
            )
            .unwrap();
            uuids.push(id);
        }
        uuids
    }

    #[test]
    fn track_count_starts_at_zero_and_climbs_with_inserts() {
        let lib = Library::open_in_memory().unwrap();
        assert_eq!(lib.track_count().unwrap(), 0);
        seed_tracks(&lib, &["A", "B", "C"]);
        assert_eq!(lib.track_count().unwrap(), 3);
    }

    #[test]
    fn list_tracks_assembles_priority_chain_correctly() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["First", "Second"]);
        let rows = lib.list_tracks(10, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, ids[0]);
        // `seed_tracks` leaves id3.title NULL → COALESCE falls
        // through to filename per PRD §8.4 fallback. The priority
        // direction itself (id3 > filename when both present) is
        // exercised by `list_tracks_prefers_id3_title_over_filename`
        // below.
        assert_eq!(rows[0].title.as_deref(), Some("First"));
        assert_eq!(rows[0].artist.as_deref(), Some("Test Artist"));
        // Album from id3.
        assert_eq!(rows[0].album.as_deref(), Some("Test Album"));
        // BPM now comes from the active beatgrid (M11c.1) — a
        // seeded track with no `track_beatgrids` row reports
        // `None`. ID3 BPM stays preserved verbatim in
        // `track_metadata_source` but never bleeds into the
        // browser column (PRD §8.3: ID3 BPM has no anchor and
        // can't be bound to the deck).
        assert!(rows[0].bpm.is_none());
        // No `analysis_cache` row yet → row reads as unanalyzed.
        assert!(!rows[0].is_analyzed);
    }

    #[test]
    fn list_tracks_tolerates_unknown_import_duration() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["Fresh Import", "Known Duration"]);
        lib.connection()
            .execute(
                "UPDATE tracks SET duration_ms = NULL WHERE id = ?1",
                params![ids[0]],
            )
            .unwrap();

        let rows = lib.list_tracks(10, 0).unwrap();
        let fresh = rows.iter().find(|r| r.id == ids[0]).unwrap();
        let known = rows.iter().find(|r| r.id == ids[1]).unwrap();

        assert_eq!(fresh.title.as_deref(), Some("Fresh Import"));
        assert_eq!(fresh.duration_ms, 0);
        assert_eq!(known.duration_ms, 10_000);
    }

    /// Regression for the M11d.6 fix: PRD §8.1 priority chain has
    /// `id3 > filename` for title and artist. The previous SQL
    /// (`COALESCE(fn.title, i3.title)`) inverted this and surfaced
    /// the parsed filename even when the file carried a perfectly
    /// good ID3 title — the user-visible "library shows filename
    /// instead of ID3 title" report. This test overrides the id3
    /// row's title/artist (the default `seed_tracks` leaves them
    /// NULL) and asserts the id3 row's value wins over the
    /// filename row.
    #[test]
    fn list_tracks_prefers_id3_title_over_filename() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["Filename Title"]);
        // Stamp distinct id3.title / id3.artist. The filename
        // source already carries "Filename Title" / "Test Artist"
        // via `seed_tracks`. With the §8.1 chain (id3 > filename)
        // the browser must surface the id3 values.
        lib.connection()
            .execute(
                "UPDATE track_metadata_source \
                 SET title = 'ID3 Title', artist = 'ID3 Artist' \
                 WHERE track_id = ?1 AND source = 'id3'",
                params![ids[0]],
            )
            .unwrap();
        let row = lib
            .list_tracks(10, 0)
            .unwrap()
            .into_iter()
            .find(|r| r.id == ids[0])
            .unwrap();
        assert_eq!(row.title.as_deref(), Some("ID3 Title"));
        assert_eq!(row.artist.as_deref(), Some("ID3 Artist"));
    }

    /// Sort companion to the priority-chain regression: the sort
    /// column expression in `TrackSortKey::sql_column` must use
    /// the same COALESCE order as the SELECT, otherwise rows can
    /// sort by one source's value but display another's. Seed
    /// three tracks with id3 titles that intentionally invert the
    /// filename alphabetical order and assert the rows come back
    /// sorted by id3 (the displayed value), not by filename.
    #[test]
    fn list_tracks_sort_by_title_uses_id3_when_present() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["AAA filename", "BBB filename", "CCC filename"]);
        // Filename order ascending is AAA, BBB, CCC. Stamp id3
        // titles that reverse the order so an id3-first sort
        // surfaces ZZZ, YYY, XXX while a filename-first sort
        // would keep AAA, BBB, CCC.
        for (id, t) in ids.iter().zip(["ZZZ id3", "YYY id3", "XXX id3"]) {
            lib.connection()
                .execute(
                    "UPDATE track_metadata_source SET title = ?2 \
                     WHERE track_id = ?1 AND source = 'id3'",
                    params![id, t],
                )
                .unwrap();
        }
        let rows = lib
            .list_tracks_sorted(10, 0, TrackSortKey::Title, true)
            .unwrap();
        assert_eq!(rows[0].title.as_deref(), Some("XXX id3"));
        assert_eq!(rows[1].title.as_deref(), Some("YYY id3"));
        assert_eq!(rows[2].title.as_deref(), Some("ZZZ id3"));
    }

    #[test]
    fn list_tracks_bpm_column_reads_from_active_beatgrid() {
        // M11c.1 contract: TrackRow.bpm comes from
        // `track_beatgrids WHERE is_active = 1`, not from
        // id3.bpm. Land an auto grid by hand (mirrors what the
        // analysis module would do) and assert the row reflects
        // it. Also assert `is_analyzed` flips once `analysis_cache`
        // is stamped.
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["First"]);
        let now = 1_700_000_000_i64;
        lib.connection()
            .execute(
                "INSERT INTO track_beatgrids \
                 (track_id, source, anchor_secs, bpm, is_active, captured_at) \
                 VALUES (?1, 'auto', 0.0, 92.5, 1, ?2)",
                params![ids[0], now],
            )
            .unwrap();
        let fp_id: i64 = lib
            .connection()
            .query_row(
                "SELECT fingerprint_id FROM tracks WHERE id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        lib.connection()
            .execute(
                "INSERT INTO analysis_cache \
                 (fingerprint_id, has_active_grid, analyzed_at) \
                 VALUES (?1, 1, ?2)",
                params![fp_id, now],
            )
            .unwrap();

        let rows = lib.list_tracks(10, 0).unwrap();
        let row = rows.iter().find(|r| r.id == ids[0]).unwrap();
        assert!((row.bpm.unwrap() - 92.5).abs() < 1e-6);
        assert!(row.is_analyzed);
    }

    #[test]
    fn list_tracks_key_column_reads_from_active_track_keys() {
        // M11c.2 contract: TrackRow.key comes from
        // `track_keys WHERE is_active = 1`, *always* Camelot.
        // ID3 TKEY is never read into TrackRow.key (it would
        // break the unanalysed = no key visual contract).
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["KeyTrack"]);
        let now = 1_700_000_000_i64;
        lib.connection()
            .execute(
                "INSERT INTO track_keys \
                 (track_id, source, key_notation, original_notation, confidence, is_active, captured_at) \
                 VALUES (?1, 'auto', '8B', '8B', 0.42, 1, ?2)",
                params![ids[0], now],
            )
            .unwrap();
        let rows = lib.list_tracks(10, 0).unwrap();
        let row = rows.iter().find(|r| r.id == ids[0]).unwrap();
        assert_eq!(row.key.as_deref(), Some("8B"));
        assert!(
            !row.key_disagreement,
            "single source -> never flagged disagreement"
        );
    }

    #[test]
    fn list_tracks_flags_key_disagreement_only_on_parallel_pairs() {
        // PRD §8.3.2 relative-major-aware rule: same Camelot
        // *number* across sources is never flagged (legitimate
        // K-K template ambiguity, e.g. 8B vs 8A). Different
        // Camelot numbers (e.g. 8B vs 5A — C major vs C minor)
        // do flag.
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["Relative", "Parallel"]);
        let now = 1_700_000_000_i64;

        // Track 0: 8B (auto) + 8A (serato) — same family,
        // should NOT flag.
        lib.connection()
            .execute(
                "INSERT INTO track_keys (track_id, source, key_notation, original_notation, confidence, is_active, captured_at) \
                 VALUES (?1, 'auto', '8B', '8B', 0.5, 1, ?2), \
                        (?1, 'serato', '8A', '8A', NULL, 0, ?2)",
                params![ids[0], now],
            )
            .unwrap();

        // Track 1: 8B (auto) + 5A (serato) — different families,
        // SHOULD flag.
        lib.connection()
            .execute(
                "INSERT INTO track_keys (track_id, source, key_notation, original_notation, confidence, is_active, captured_at) \
                 VALUES (?1, 'auto', '8B', '8B', 0.5, 1, ?2), \
                        (?1, 'serato', '5A', 'C minor', NULL, 0, ?2)",
                params![ids[1], now],
            )
            .unwrap();

        let rows = lib.list_tracks(10, 0).unwrap();
        let relative = rows.iter().find(|r| r.id == ids[0]).unwrap();
        let parallel = rows.iter().find(|r| r.id == ids[1]).unwrap();
        assert!(
            !relative.key_disagreement,
            "8B vs 8A is a relative-key pair; must not flag"
        );
        assert!(
            parallel.key_disagreement,
            "8B vs 5A is a parallel-key pair; must flag"
        );
    }

    #[test]
    fn list_tracks_paginates_via_limit_offset() {
        let lib = Library::open_in_memory().unwrap();
        seed_tracks(&lib, &["A", "B", "C", "D", "E"]);
        let page1 = lib.list_tracks(2, 0).unwrap();
        let page2 = lib.list_tracks(2, 2).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        assert_ne!(page1[0].id, page2[0].id);
    }

    #[test]
    fn search_tracks_matches_via_fts5_suffix() {
        let lib = Library::open_in_memory().unwrap();
        seed_tracks(&lib, &["Workinonit", "Stakes Is High", "Donuts"]);
        let hits = lib.search_tracks("workin", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title.as_deref(), Some("Workinonit"));

        // Multi-token query ANDs the tokens.
        let hits = lib.search_tracks("stakes high", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title.as_deref(), Some("Stakes Is High"));

        // Empty / too-short queries return empty.
        let hits = lib.search_tracks("", 10).unwrap();
        assert!(hits.is_empty());
        let hits = lib.search_tracks("a", 10).unwrap();
        assert!(hits.is_empty());

        // No-match query returns empty.
        let hits = lib.search_tracks("nonexistent", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn just_imported_filters_by_created_at_threshold() {
        let lib = Library::open_in_memory().unwrap();
        seed_tracks(&lib, &["A", "B"]);
        // strftime('%s','now') is the seed time → all rows are
        // "since the dawn of time"; from-the-future threshold
        // returns empty.
        let none = lib.just_imported(i64::MAX - 1, 10).unwrap();
        assert!(none.is_empty());
        let all = lib.just_imported(0, 10).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn recently_played_returns_empty_when_no_history() {
        let lib = Library::open_in_memory().unwrap();
        seed_tracks(&lib, &["A", "B"]);
        let rows = lib.recently_played(10).unwrap();
        // No play_history rows seeded → empty result, *not* a fallback
        // to all-tracks. Keeps the smart-crate semantics honest.
        assert!(rows.is_empty());
    }

    #[test]
    fn list_files_for_scan_returns_unchecked_first() {
        // PRD §8.5.5: the scanner prioritises rows that have
        // never been checked. Two seeded tracks → two file rows
        // → first batch returns both with `was_missing = false`
        // and `last_checked_at = NULL`-ordered first.
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["A", "B"]);
        let rows = lib.list_files_for_scan(10).unwrap();
        assert_eq!(rows.len(), 2);
        for r in &rows {
            assert!(!r.was_missing);
            assert!(ids.contains(&r.track_id));
            assert_eq!(r.mount_point.as_deref(), Some("/"));
        }
    }

    #[test]
    fn mark_file_state_flips_is_missing_and_stamps_checked_at() {
        let lib = Library::open_in_memory().unwrap();
        let _ids = seed_tracks(&lib, &["A"]);
        let row = lib.list_files_for_scan(1).unwrap().pop().unwrap();
        assert!(!row.was_missing);
        lib.mark_file_state(row.file_id, true, 1_700_000_000)
            .unwrap();
        // Re-list: same row should now have was_missing=true and
        // sort *last* because it's been checked once.
        let next = lib.list_files_for_scan(1).unwrap().pop().unwrap();
        assert!(next.was_missing);
        assert_eq!(next.file_id, row.file_id);
    }

    #[test]
    fn missing_track_count_only_counts_fully_unreachable_tracks() {
        // Two tracks; one gets a second healthy file row before
        // its first is marked missing, the other only ever had
        // one file row.
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["StillReachable", "Lost"]);
        // Second healthy file for the first track.
        lib.upsert_track_file(
            &ids[0],
            "11111111-1111-1111-1111-111111111111",
            "test/StillReachable-copy.wav",
            Some("wav"),
            Some(44_100),
            None,
            Some(1),
            Some(123_456),
            Some(1_700_000_000),
        )
        .unwrap();
        // Mark every file of the first track *except* the new one
        // missing; mark the only file of the second track missing.
        let all = lib.list_files_for_scan(10).unwrap();
        for r in all {
            if r.relative_path != "test/StillReachable-copy.wav" {
                lib.mark_file_state(r.file_id, true, 1_700_000_001).unwrap();
            }
        }
        // The first track has at least one healthy file → not
        // missing. The second has zero → missing.
        let n = lib.missing_track_count().unwrap();
        assert_eq!(n, 1);
        let rows = lib.list_missing_tracks(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].track_id, ids[1]);
        // Filename derived from the most-recent relative_path.
        assert_eq!(rows[0].last_filename.as_deref(), Some("Lost.wav"));
    }

    #[test]
    fn relocate_track_inserts_new_path_and_clears_missing_state() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["Workinonit"]);
        // Mark the original file missing.
        let orig = lib.list_files_for_scan(1).unwrap().pop().unwrap();
        lib.mark_file_state(orig.file_id, true, 1_700_000_000)
            .unwrap();
        assert_eq!(lib.missing_track_count().unwrap(), 1);

        // Relocate to a new path. PRD §8.5.5: the original row
        // stays on record (no deletion); a new track_files row
        // appears and the track count drops back to 0 missing.
        lib.relocate_track(
            &ids[0],
            "11111111-1111-1111-1111-111111111111",
            "relocated/Workinonit.wav",
            Some("wav"),
            Some(44_100),
            Some(1),
            Some(234_567),
            Some(1_700_000_100),
        )
        .unwrap();
        assert_eq!(lib.missing_track_count().unwrap(), 0);
        // Both file rows still exist (original + relocated).
        let count: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_files WHERE track_id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn list_tracks_populates_primary_file_columns() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["First"]);
        let rows = lib.list_tracks(10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, ids[0]);
        assert_eq!(
            rows[0].primary_volume_uuid.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        // seed_tracks registers the volume with mount_point = "/".
        assert_eq!(rows[0].primary_volume_mount_point.as_deref(), Some("/"));
        assert_eq!(
            rows[0].primary_relative_path.as_deref(),
            Some("test/First.wav")
        );
    }

    #[test]
    fn track_row_returns_most_recent_track_file_on_multi_file_track() {
        // A track gets re-imported from a different path (e.g. the
        // DJ moved it). track_files now has two rows; the browser
        // must surface the *newer* one because that's the one the
        // user can still reach.
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["Workinonit"]);
        // Second file row, newer `last_seen_at`.
        lib.upsert_track_file(
            &ids[0],
            "11111111-1111-1111-1111-111111111111",
            "moved/Workinonit.wav",
            Some("wav"),
            Some(44_100),
            None,
            Some(1),
            Some(123_456),
            Some(2_000_000_000),
        )
        .unwrap();
        let rows = lib.list_tracks(10, 0).unwrap();
        assert_eq!(
            rows[0].primary_relative_path.as_deref(),
            Some("moved/Workinonit.wav")
        );
    }

    #[test]
    fn record_load_appears_in_recently_played_newest_first() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["First", "Second", "Third"]);
        // Three loads, oldest → newest. Recently Played should
        // return newest first (last load wins).
        lib.record_load(&ids[0], 0, 1_700_000_000_000).unwrap();
        lib.record_load(&ids[1], 1, 1_700_000_001_000).unwrap();
        lib.record_load(&ids[2], 0, 1_700_000_002_000).unwrap();

        let rows = lib.recently_played(10).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, ids[2]);
        assert_eq!(rows[1].id, ids[1]);
        assert_eq!(rows[2].id, ids[0]);
    }

    #[test]
    fn record_load_idempotent_for_same_track_uses_latest_timestamp() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["A", "B"]);
        // Two loads of the same track; "Recently Played" must
        // show it once (DISTINCT track_id) at the *latest*
        // timestamp.
        lib.record_load(&ids[0], 0, 1_700_000_000_000).unwrap();
        lib.record_load(&ids[1], 0, 1_700_000_001_000).unwrap();
        lib.record_load(&ids[0], 1, 1_700_000_002_000).unwrap();
        let rows = lib.recently_played(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, ids[0]);
        assert_eq!(rows[1].id, ids[1]);
    }

    #[test]
    fn record_load_rejects_unknown_track_id() {
        let lib = Library::open_in_memory().unwrap();
        seed_tracks(&lib, &["A"]);
        // FK constraint catches a stale Apple-side selection
        // pointing at a track that was deleted between selection
        // and load.
        let err = lib.record_load("ffffffff-0000-0000-0000-000000000000", 0, 1_700_000_000_000);
        assert!(err.is_err());
    }

    #[test]
    fn list_tracks_sorted_orders_by_title_ascending() {
        let lib = Library::open_in_memory().unwrap();
        seed_tracks(&lib, &["Cherry", "Apple", "Banana"]);
        let rows = lib
            .list_tracks_sorted(10, 0, TrackSortKey::Title, true)
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].title.as_deref(), Some("Apple"));
        assert_eq!(rows[1].title.as_deref(), Some("Banana"));
        assert_eq!(rows[2].title.as_deref(), Some("Cherry"));
    }

    #[test]
    fn list_tracks_sorted_descending_inverts_order() {
        let lib = Library::open_in_memory().unwrap();
        seed_tracks(&lib, &["Cherry", "Apple", "Banana"]);
        let rows = lib
            .list_tracks_sorted(10, 0, TrackSortKey::Title, false)
            .unwrap();
        assert_eq!(rows[0].title.as_deref(), Some("Cherry"));
        assert_eq!(rows[2].title.as_deref(), Some("Apple"));
    }

    #[test]
    fn list_tracks_sorted_is_case_insensitive() {
        let lib = Library::open_in_memory().unwrap();
        // "abba" and "ABBA" should sort adjacent under NOCASE
        // collation; without it, the entire uppercase block would
        // precede the lowercase block.
        seed_tracks(&lib, &["abba", "Zoso", "ABBA"]);
        let rows = lib
            .list_tracks_sorted(10, 0, TrackSortKey::Title, true)
            .unwrap();
        // abba / ABBA in some order, Zoso last.
        assert!(matches!(rows[0].title.as_deref(), Some("abba" | "ABBA")));
        assert!(matches!(rows[1].title.as_deref(), Some("abba" | "ABBA")));
        assert_eq!(rows[2].title.as_deref(), Some("Zoso"));
    }

    #[test]
    fn list_tracks_sorted_nulls_sort_last_in_both_directions() {
        // M11c.1 changed TrackRow.bpm to read from
        // `track_beatgrids` (active row), not from
        // `track_metadata_source.bpm`. Seed two tracks, give one
        // of them an active auto grid; the row without a grid is
        // the NULL-BPM case and must sort last in both directions.
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["With Bpm", "Also With Bpm"]);
        let now = 1_700_000_000_i64;
        lib.connection()
            .execute(
                "INSERT INTO track_beatgrids \
                 (track_id, source, anchor_secs, bpm, is_active, captured_at) \
                 VALUES (?1, 'auto', 0.0, 140.0, 1, ?2)",
                params![ids[1], now],
            )
            .unwrap();
        let asc = lib
            .list_tracks_sorted(10, 0, TrackSortKey::Bpm, true)
            .unwrap();
        // NULL row (ids[0] — no grid) must be last in ASC.
        assert_eq!(asc[1].id, ids[0]);
        let desc = lib
            .list_tracks_sorted(10, 0, TrackSortKey::Bpm, false)
            .unwrap();
        // NULL row stays last in DESC too.
        assert_eq!(desc[1].id, ids[0]);
    }

    #[test]
    fn resolve_track_path_joins_volume_to_relative_path() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["Workinonit"]);
        let path = lib.resolve_track_path(&ids[0]).unwrap().unwrap();
        assert_eq!(path, PathBuf::from("/").join("test/Workinonit.wav"));
        // Bogus id → None, not error.
        let missing = lib
            .resolve_track_path("00000000-0000-0000-0000-000000000000")
            .unwrap();
        assert!(missing.is_none());
    }

    // === Dub crates (M11d-next) ==========================================

    #[test]
    fn create_crate_returns_id_and_lists_with_zero_tracks() {
        let lib = Library::open_in_memory().unwrap();
        let id = lib.create_crate("Reggae 45s", None).unwrap();
        assert!(id > 0);
        let crates = lib.list_crates().unwrap();
        assert_eq!(crates.len(), 1);
        assert_eq!(crates[0].id, id);
        assert_eq!(crates[0].name, "Reggae 45s");
        assert_eq!(crates[0].parent_id, None);
        assert_eq!(crates[0].track_count, 0);
    }

    #[test]
    fn create_crate_rejects_duplicate_sibling_name() {
        let lib = Library::open_in_memory().unwrap();
        lib.create_crate("Dubplates", None).unwrap();
        let err = lib.create_crate("Dubplates", None).unwrap_err();
        assert!(
            matches!(err, LibraryError::CrateNameConflict { ref name } if name == "Dubplates"),
            "expected CrateNameConflict, got {err:?}"
        );
    }

    #[test]
    fn nested_crate_can_reuse_sibling_name_under_different_parent() {
        let lib = Library::open_in_memory().unwrap();
        let parent_a = lib.create_crate("Sets", None).unwrap();
        let parent_b = lib.create_crate("Archive", None).unwrap();
        // "Friday" under two different parents is allowed; the UNIQUE
        // constraint is scoped to (parent_crate_id, name).
        lib.create_crate("Friday", Some(parent_a)).unwrap();
        lib.create_crate("Friday", Some(parent_b)).unwrap();
        let crates = lib.list_crates().unwrap();
        assert_eq!(crates.len(), 4);
    }

    #[test]
    fn rename_crate_changes_name_and_flags_conflicts() {
        let lib = Library::open_in_memory().unwrap();
        let id = lib.create_crate("Workhouse", None).unwrap();
        lib.create_crate("Taken", None).unwrap();
        lib.rename_crate(id, "Workshop").unwrap();
        assert_eq!(lib.list_crates().unwrap()[0].name, "Taken");
        // Renaming onto a sibling's name conflicts.
        let conflict = lib.rename_crate(id, "Taken").unwrap_err();
        assert!(matches!(conflict, LibraryError::CrateNameConflict { .. }));
        // Renaming a non-existent crate is CrateNotFound.
        let missing = lib.rename_crate(9999, "Whatever").unwrap_err();
        assert!(matches!(
            missing,
            LibraryError::CrateNotFound { crate_id: 9999 }
        ));
    }

    #[test]
    fn delete_crate_cascades_membership_and_children() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["A", "B"]);
        let parent = lib.create_crate("Parent", None).unwrap();
        let child = lib.create_crate("Child", Some(parent)).unwrap();
        lib.add_track_to_crate(parent, &ids[0]).unwrap();
        lib.add_track_to_crate(child, &ids[1]).unwrap();
        lib.delete_crate(parent).unwrap();
        // Parent + cascaded child are both gone.
        assert!(lib.list_crates().unwrap().is_empty());
        // crate_tracks rows for both crates cascaded away; the tracks
        // themselves survive.
        assert_eq!(lib.track_count().unwrap(), 2);
        // Deleting a missing crate is CrateNotFound.
        let missing = lib.delete_crate(parent).unwrap_err();
        assert!(matches!(missing, LibraryError::CrateNotFound { .. }));
    }

    #[test]
    fn add_track_is_idempotent_and_counts_members() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["One", "Two"]);
        let id = lib.create_crate("Bag", None).unwrap();
        assert!(lib.add_track_to_crate(id, &ids[0]).unwrap());
        // Re-adding the same track is a no-op.
        assert!(!lib.add_track_to_crate(id, &ids[0]).unwrap());
        assert!(lib.add_track_to_crate(id, &ids[1]).unwrap());
        assert_eq!(lib.list_crates().unwrap()[0].track_count, 2);
    }

    #[test]
    fn add_track_validates_crate_and_track_existence() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["Real"]);
        let id = lib.create_crate("Bag", None).unwrap();
        // Unknown crate.
        let no_crate = lib.add_track_to_crate(4242, &ids[0]).unwrap_err();
        assert!(matches!(
            no_crate,
            LibraryError::CrateNotFound { crate_id: 4242 }
        ));
        // Unknown track.
        let no_track = lib
            .add_track_to_crate(id, "00000000-0000-0000-0000-000000000000")
            .unwrap_err();
        assert!(matches!(no_track, LibraryError::TrackNotFound { .. }));
    }

    #[test]
    fn list_crate_tracks_returns_members_in_insertion_order() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["First", "Second", "Third"]);
        let id = lib.create_crate("Order", None).unwrap();
        lib.add_track_to_crate(id, &ids[0]).unwrap();
        lib.add_track_to_crate(id, &ids[1]).unwrap();
        lib.add_track_to_crate(id, &ids[2]).unwrap();
        let rows = lib.list_crate_tracks(id).unwrap();
        let got: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(got, vec![&ids[0], &ids[1], &ids[2]]);
    }

    #[test]
    fn remove_track_is_idempotent() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["X"]);
        let id = lib.create_crate("C", None).unwrap();
        lib.add_track_to_crate(id, &ids[0]).unwrap();
        lib.remove_track_from_crate(id, &ids[0]).unwrap();
        assert!(lib.list_crate_tracks(id).unwrap().is_empty());
        // Removing again is a successful no-op.
        lib.remove_track_from_crate(id, &ids[0]).unwrap();
    }

    #[test]
    fn set_crate_track_order_rewrites_ordinals() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["A", "B", "C"]);
        let id = lib.create_crate("Reorder", None).unwrap();
        for t in &ids {
            lib.add_track_to_crate(id, t).unwrap();
        }
        // Reverse the order.
        let reversed: Vec<&str> = vec![ids[2].as_str(), ids[1].as_str(), ids[0].as_str()];
        lib.set_crate_track_order(id, &reversed).unwrap();
        let got: Vec<String> = lib
            .list_crate_tracks(id)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, vec![ids[2].clone(), ids[1].clone(), ids[0].clone()]);
    }

    #[test]
    fn add_track_appends_after_reorder() {
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["A", "B", "C"]);
        let id = lib.create_crate("Append", None).unwrap();
        lib.add_track_to_crate(id, &ids[0]).unwrap();
        lib.add_track_to_crate(id, &ids[1]).unwrap();
        // Reorder so B is first, then add C — C should land last.
        lib.set_crate_track_order(id, &[ids[1].as_str(), ids[0].as_str()])
            .unwrap();
        lib.add_track_to_crate(id, &ids[2]).unwrap();
        let got: Vec<String> = lib
            .list_crate_tracks(id)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, vec![ids[1].clone(), ids[0].clone(), ids[2].clone()]);
    }
}
