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

/// One row in the M11d browser's track list. The PRD §8.1
/// priority chain ("filename source preferred over id3 source for
/// title/artist; id3 preferred for everything else") is baked in at
/// the SELECT level via COALESCE so the browser doesn't have to
/// reimplement the chain.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRow {
    /// Canonical UUID from `tracks.id`.
    pub id: String,
    /// Display title (filename source preferred over id3 per §8.1).
    pub title: Option<String>,
    /// Display artist (filename source preferred over id3 per §8.1).
    pub artist: Option<String>,
    /// Album (id3 source).
    pub album: Option<String>,
    /// Genre (id3 source).
    pub genre: Option<String>,
    /// Year (filename source preferred over id3; the filename
    /// `[YYYY]` tail is the more reliable signal in DJ-rip-naming
    /// conventions).
    pub year: Option<i32>,
    /// BPM from the id3 source (auto-grid lands as part of the
    /// M11c follow-up that wires `dub-bpm` into the importer).
    pub bpm: Option<f64>,
    /// Musical key from the id3 source (or Mixed In Key's comment
    /// field via M11e Serato importer at a later milestone).
    pub key: Option<String>,
    /// Duration in milliseconds, from `tracks.duration_ms`.
    pub duration_ms: u32,
    /// Comma-separated canonical version-tokens (filename source
    /// preferred). `None` when no tokens were detected.
    pub version_tokens: Option<String>,
    /// `Some(other_track_id)` when this row has a sibling-version
    /// link per §8.1 dedupe. Drives the M11d.3 potential-duplicate
    /// indicator.
    pub potential_duplicate_id: Option<String>,
    /// Origin source — `"filesystem"` for M11c-imported rows;
    /// `"serato"` / `"traktor"` / `"rekordbox"` / `"itunes"` for
    /// future M11e+ importers. Synthesised from the per-source
    /// metadata-row `source` column.
    pub source: String,
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
    /// Display title (filename source preferred over id3).
    Title,
    /// Display artist (filename source preferred over id3).
    Artist,
    /// Album name (id3 source).
    Album,
    /// BPM (id3 source). `None` BPMs sort last in both directions.
    Bpm,
    /// Duration in milliseconds.
    Duration,
    /// Year (filename source preferred over id3).
    Year,
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
            Self::Title => "COALESCE(fn.title, i3.title)",
            Self::Artist => "COALESCE(fn.artist, i3.artist)",
            Self::Album => "i3.album",
            Self::Bpm => "i3.bpm",
            Self::Duration => "t.duration_ms",
            Self::Year => "COALESCE(fn.year, i3.year)",
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
const TRACK_ROW_SELECT: &str = "\
    SELECT t.id, \
           COALESCE(fn.title,    i3.title)    AS title, \
           COALESCE(fn.artist,   i3.artist)   AS artist, \
           i3.album                            AS album, \
           i3.genre                            AS genre, \
           COALESCE(fn.year,     i3.year)     AS year, \
           i3.bpm                              AS bpm, \
           i3.key                              AS key, \
           t.duration_ms                       AS duration_ms, \
           COALESCE(fn.version_token, i3.version_token) AS version_tokens, \
           t.duplicate_link_track_id           AS potential_duplicate_id, \
           ( \
               SELECT MIN(source) FROM track_metadata_source ms \
               WHERE ms.track_id = t.id \
           )                                   AS source \
    FROM tracks t \
    LEFT JOIN track_metadata_source fn \
              ON fn.track_id = t.id AND fn.source = 'filename' \
    LEFT JOIN track_metadata_source i3 \
              ON i3.track_id = t.id AND i3.source = 'id3' \
    ";

/// Map a SELECT-shaped row to [`TrackRow`]. Used by every
/// `list_*` / `search_*` / `recently_*` method so column order
/// stays in lockstep with `TRACK_ROW_SELECT`.
fn track_row_from_columns(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackRow> {
    let duration_ms: i64 = r.get(8)?;
    Ok(TrackRow {
        id: r.get(0)?,
        title: r.get(1)?,
        artist: r.get(2)?,
        album: r.get(3)?,
        genre: r.get(4)?,
        year: r.get(5)?,
        bpm: r.get(6)?,
        key: r.get(7)?,
        duration_ms: duration_ms.max(0) as u32,
        version_tokens: r.get(9)?,
        potential_duplicate_id: r.get(10)?,
        source: r
            .get::<_, Option<String>>(11)?
            .unwrap_or_else(|| "unknown".to_string()),
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
        })
    }

    /// Open an in-memory database (`:memory:`). Migrations applied;
    /// used by tests that never touch the disk.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn =
            Connection::open_in_memory().map_err(|e| LibraryError::sqlite("open_in_memory", e))?;
        open_and_migrate(&mut conn)?;
        Ok(Self {
            conn,
            db_path: PathBuf::from(":memory:"),
        })
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
    pub fn insert_track(
        &self,
        track_uuid: &str,
        fingerprint_id: i64,
        duration_ms: u32,
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
                    duration_ms as i64,
                    duplicate_link_track_id,
                ],
            )
            .map_err(|e| LibraryError::sqlite("insert_track", e))?;
        Ok(())
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
                 ORDER BY tf.last_seen_at DESC LIMIT 1",
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
            lib.insert_track(&id, fp_id, 10_000, None).unwrap();
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
            // exercise both sources independently.
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
        // Title comes from the filename source (preferred over id3).
        assert_eq!(rows[0].title.as_deref(), Some("First"));
        // Artist same: filename source wins.
        assert_eq!(rows[0].artist.as_deref(), Some("Test Artist"));
        // Album from id3.
        assert_eq!(rows[0].album.as_deref(), Some("Test Album"));
        // BPM from id3.
        assert!((rows[0].bpm.unwrap() - 123.45).abs() < 1e-6);
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
        let lib = Library::open_in_memory().unwrap();
        let ids = seed_tracks(&lib, &["With Bpm", "Also With Bpm"]);
        // Seed inserts BPM=123.45 for every row via the id3
        // metadata. Strip it from one row to test NULL handling.
        lib.connection()
            .execute(
                "UPDATE track_metadata_source SET bpm = NULL \
                 WHERE track_id = ?1 AND source = 'id3'",
                params![ids[0]],
            )
            .unwrap();
        let asc = lib
            .list_tracks_sorted(10, 0, TrackSortKey::Bpm, true)
            .unwrap();
        // NULL row must be last in ASC.
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
}
