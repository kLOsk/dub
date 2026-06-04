//! Schema definition + migration runner for the Dub library DB.
//!
//! The schema is the public API surface per PRD §8.7 and is
//! normatively documented in `docs/spec/LIBRARY-SCHEMA.md`. The SQL strings
//! in this file must stay byte-for-byte identical with the doc's
//! `CREATE TABLE` blocks; the doc is the spec, this file is the
//! implementation.
//!
//! ## Migration policy
//!
//! `SCHEMA_VERSION` is the highest schema version this binary knows
//! how to apply. On open, the runner reads the current on-disk
//! version, applies every migration in order with `target > current`
//! inside a single transaction per migration, and bumps the
//! `schema_version` row.
//!
//! ### Backward compatibility contract (PRD §8.7)
//!
//! * **Additive** changes (new tables / columns / indexes) are
//!   version bumps but never break third-party readers that ignore
//!   unknown schema.
//! * **Renames / removals / type changes** require a documented
//!   migration; renamed columns retain `_legacy` aliases for at
//!   least one full minor-version cycle.
//! * A binary opening a DB at `schema_version > SCHEMA_VERSION` must
//!   refuse to write (read-only fallback). v1.0 surfaces this as
//!   `LibraryError::SchemaTooNew`.
//!
//! ### Idempotence
//!
//! Every migration's SQL uses `IF NOT EXISTS` / `INSERT OR IGNORE` /
//! similar guards so a partial application (e.g. crash mid-migration)
//! leaves a consistent state and the next open re-applies cleanly.

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::error::{LibraryError, Result};

/// The highest schema version this binary knows how to apply. Bump
/// in lockstep with adding an entry to [`MIGRATIONS`] and updating
/// `docs/spec/LIBRARY-SCHEMA.md`.
pub const SCHEMA_VERSION: u32 = 5;

/// One migration step. Applied inside a single SQLite transaction;
/// either every statement lands or none does.
struct Migration {
    /// The `schema_version` value this migration produces.
    target_version: u32,
    /// The SQL script (multi-statement) to execute. Statements are
    /// separated by `;` and executed via `Connection::execute_batch`.
    sql: &'static str,
}

/// All migrations in version order. Empty on a freshly created DB
/// means no work; `SCHEMA_VERSION` is the target.
static MIGRATIONS: &[Migration] = &[
    Migration {
        target_version: 1,
        sql: V1_SCHEMA,
    },
    Migration {
        target_version: 2,
        sql: V2_MIGRATION,
    },
    Migration {
        target_version: 3,
        sql: V3_MIGRATION,
    },
    Migration {
        target_version: 4,
        sql: V4_MIGRATION,
    },
    Migration {
        target_version: 5,
        sql: V5_MIGRATION,
    },
];

/// Open + migrate the library schema on the given connection. Idempotent.
///
/// The runner:
///
/// 1. Sets the connection-level PRAGMAs (`journal_mode=WAL`,
///    `foreign_keys=ON`, etc.) per `docs/spec/LIBRARY-SCHEMA.md`.
/// 2. Creates `schema_version` if missing; reads the current version.
/// 3. Refuses with `LibraryError::SchemaTooNew` if the on-disk
///    version exceeds [`SCHEMA_VERSION`].
/// 4. Applies every migration with `target > current` in order, each
///    inside its own transaction, bumping `schema_version` at the
///    tail of each.
pub fn open_and_migrate(conn: &mut Connection) -> Result<()> {
    apply_connection_pragmas(conn)?;
    ensure_schema_version_table(conn)?;
    let current = read_schema_version(conn)?;
    if current > SCHEMA_VERSION {
        return Err(LibraryError::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for migration in MIGRATIONS {
        if migration.target_version <= current {
            continue;
        }
        let tx = conn
            .transaction()
            .map_err(|e| LibraryError::sqlite("migration_begin", e))?;
        apply_migration(&tx, migration)?;
        tx.commit()
            .map_err(|e| LibraryError::sqlite("migration_commit", e))?;
    }
    Ok(())
}

/// Connection-level PRAGMAs documented in `docs/spec/LIBRARY-SCHEMA.md`
/// under "PRAGMAs". Applied on every open (PRAGMAs that are
/// per-connection rather than per-database, like `foreign_keys`,
/// must be set each time).
fn apply_connection_pragmas(conn: &Connection) -> Result<()> {
    // `journal_mode = WAL` is per-database (persistent) but cheap to
    // re-issue; doing it here means a freshly created DB lands in
    // WAL mode before any other write happens, so the very first
    // import never blocks readers.
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| LibraryError::sqlite("pragma_journal_mode", e))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| LibraryError::sqlite("pragma_synchronous", e))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| LibraryError::sqlite("pragma_foreign_keys", e))?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(|e| LibraryError::sqlite("pragma_temp_store", e))?;
    // 256 MB mmap window. The library is small (10s of MB even at
    // 100k tracks) so this comfortably covers the entire file in
    // mapped memory; SQLite reads are then page-faults into the
    // OS cache, not syscalls.
    conn.pragma_update(None, "mmap_size", 268_435_456_i64)
        .map_err(|e| LibraryError::sqlite("pragma_mmap_size", e))?;
    Ok(())
}

/// Creates the `schema_version` table if absent, inserting a row at
/// version 0 so the migration runner sees a deterministic "nothing
/// applied yet" state on a fresh DB. The version-0 row is updated
/// to `SCHEMA_VERSION` once the v1 migration completes.
fn ensure_schema_version_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version    INTEGER NOT NULL,
            applied_at INTEGER NOT NULL
        );",
    )
    .map_err(|e| LibraryError::sqlite("create_schema_version", e))?;
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
        .map_err(|e| LibraryError::sqlite("count_schema_version", e))?;
    if row_count == 0 {
        conn.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (0, strftime('%s','now'))",
            [],
        )
        .map_err(|e| LibraryError::sqlite("seed_schema_version", e))?;
    }
    Ok(())
}

/// Reads the single `schema_version.version` row. Returns 0 if the
/// table is empty (which `ensure_schema_version_table` guarantees
/// not to be, but we tolerate it for robustness).
fn read_schema_version(conn: &Connection) -> Result<u32> {
    let v: Option<i64> = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| LibraryError::sqlite("read_schema_version", e))?;
    Ok(v.unwrap_or(0) as u32)
}

/// Applies one migration inside the given transaction and bumps
/// `schema_version` at the end.
fn apply_migration(tx: &Transaction<'_>, migration: &Migration) -> Result<()> {
    tx.execute_batch(migration.sql).map_err(|e| {
        // The static `context` budget per error variant means we
        // pick the most useful summary string we can. The migration's
        // target_version is more useful than "execute_batch" because
        // the migration set is small and version-numbered.
        match migration.target_version {
            1 => LibraryError::sqlite("migration_v1", e),
            _ => LibraryError::sqlite("migration_apply", e),
        }
    })?;
    tx.execute(
        "UPDATE schema_version SET version = ?1, applied_at = strftime('%s','now')",
        params![migration.target_version],
    )
    .map_err(|e| LibraryError::sqlite("migration_bump_version", e))?;
    Ok(())
}

/// v1 schema. The canonical reference is `docs/spec/LIBRARY-SCHEMA.md`;
/// changes here must be reflected there in lockstep. Every statement
/// uses `IF NOT EXISTS` so re-running this script on an already-
/// migrated DB is a no-op (matters because the migration runner
/// commits each migration in its own transaction; if a transaction
/// commits then a later step fails, the next open re-attempts and
/// must not double-create).
const V1_SCHEMA: &str = r#"
-- See docs/spec/LIBRARY-SCHEMA.md for the normative reference.

CREATE TABLE IF NOT EXISTS fingerprints (
    id                INTEGER PRIMARY KEY,
    chromaprint_blob  BLOB    NOT NULL,
    duration_ms       INTEGER NOT NULL,
    file_size         INTEGER,
    sample_rate       INTEGER,
    channel_count     INTEGER,
    computed_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_fingerprints_duration ON fingerprints(duration_ms);

CREATE TABLE IF NOT EXISTS tracks (
    id                          TEXT    PRIMARY KEY NOT NULL,
    fingerprint_id              INTEGER REFERENCES fingerprints(id) ON DELETE SET NULL,
    duration_ms                 INTEGER,
    duplicate_link_track_id     TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    explicit_merge_target_id    TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tracks_fingerprint    ON tracks(fingerprint_id);
CREATE INDEX IF NOT EXISTS idx_tracks_duplicate_link ON tracks(duplicate_link_track_id);
CREATE INDEX IF NOT EXISTS idx_tracks_merge_target   ON tracks(explicit_merge_target_id);

CREATE TABLE IF NOT EXISTS volumes (
    volume_uuid             TEXT    PRIMARY KEY NOT NULL,
    display_name            TEXT    NOT NULL,
    last_known_mount_point  TEXT,
    last_seen_at            INTEGER NOT NULL,
    is_internal             INTEGER NOT NULL DEFAULT 0 CHECK (is_internal IN (0, 1))
);

CREATE TABLE IF NOT EXISTS track_files (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    volume_uuid     TEXT    NOT NULL REFERENCES volumes(volume_uuid) ON DELETE CASCADE,
    relative_path   TEXT    NOT NULL,
    codec           TEXT,
    sample_rate     INTEGER,
    bit_depth       INTEGER,
    channel_count   INTEGER,
    file_size       INTEGER,
    mtime           INTEGER,
    last_seen_at    INTEGER NOT NULL,
    UNIQUE (volume_uuid, relative_path)
);
CREATE INDEX IF NOT EXISTS idx_track_files_track  ON track_files(track_id);
CREATE INDEX IF NOT EXISTS idx_track_files_volume ON track_files(volume_uuid);

CREATE TABLE IF NOT EXISTS track_metadata_source (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source          TEXT    NOT NULL CHECK (source IN
                    ('serato', 'traktor', 'rekordbox', 'itunes', 'id3', 'filename')),
    artist          TEXT,
    title           TEXT,
    album           TEXT,
    genre           TEXT,
    comment         TEXT,
    composer        TEXT,
    year            INTEGER,
    track_number    INTEGER,
    bpm             REAL,
    key             TEXT,
    gain_db         REAL,
    rating          INTEGER,
    version_token   TEXT,
    imported_at     INTEGER NOT NULL,
    UNIQUE (track_id, source)
);
CREATE INDEX IF NOT EXISTS idx_metadata_source ON track_metadata_source(source);

CREATE TABLE IF NOT EXISTS track_beatgrids (
    id            INTEGER PRIMARY KEY,
    track_id      TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source        TEXT    NOT NULL CHECK (source IN
                  ('serato', 'traktor', 'rekordbox', 'itunes', 'auto', 'user_tap')),
    anchor_secs   REAL    NOT NULL,
    bpm           REAL    NOT NULL,
    is_active     INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    captured_at   INTEGER NOT NULL,
    UNIQUE (track_id, source)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_one_active_grid_per_track
    ON track_beatgrids(track_id) WHERE is_active = 1;

CREATE TABLE IF NOT EXISTS track_cues (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source          TEXT    NOT NULL CHECK (source IN
                    ('serato', 'traktor', 'rekordbox', 'itunes', 'user')),
    cue_index       INTEGER NOT NULL,
    position_secs   REAL    NOT NULL,
    name            TEXT,
    color           TEXT,
    kind            TEXT    NOT NULL DEFAULT 'hot_cue' CHECK (kind IN
                    ('hot_cue', 'memory', 'load', 'loop_in', 'loop_out')),
    imported_at     INTEGER NOT NULL,
    UNIQUE (track_id, source, cue_index)
);
CREATE INDEX IF NOT EXISTS idx_track_cues_track ON track_cues(track_id);

CREATE TABLE IF NOT EXISTS track_loops (
    id              INTEGER PRIMARY KEY,
    track_id        TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source          TEXT    NOT NULL CHECK (source IN
                    ('serato', 'traktor', 'rekordbox', 'itunes', 'user')),
    loop_index      INTEGER NOT NULL,
    in_secs         REAL    NOT NULL,
    out_secs        REAL    NOT NULL,
    name            TEXT,
    color           TEXT,
    is_locked       INTEGER NOT NULL DEFAULT 0 CHECK (is_locked IN (0, 1)),
    imported_at     INTEGER NOT NULL,
    UNIQUE (track_id, source, loop_index)
);
CREATE INDEX IF NOT EXISTS idx_track_loops_track ON track_loops(track_id);

CREATE TABLE IF NOT EXISTS crates (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,
    parent_crate_id INTEGER REFERENCES crates(id) ON DELETE CASCADE,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (parent_crate_id, name)
);

CREATE TABLE IF NOT EXISTS crate_tracks (
    crate_id    INTEGER NOT NULL REFERENCES crates(id) ON DELETE CASCADE,
    track_id    TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    ordinal     INTEGER NOT NULL,
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (crate_id, track_id)
);
CREATE INDEX IF NOT EXISTS idx_crate_tracks_ord ON crate_tracks(crate_id, ordinal);

CREATE TABLE IF NOT EXISTS imported_crates (
    id                          INTEGER PRIMARY KEY,
    source                      TEXT    NOT NULL CHECK (source IN
                                ('serato', 'traktor', 'rekordbox', 'itunes')),
    name                        TEXT    NOT NULL,
    parent_imported_crate_id    INTEGER REFERENCES imported_crates(id) ON DELETE CASCADE,
    imported_at                 INTEGER NOT NULL,
    UNIQUE (source, parent_imported_crate_id, name)
);

CREATE TABLE IF NOT EXISTS imported_crate_tracks (
    imported_crate_id   INTEGER NOT NULL REFERENCES imported_crates(id) ON DELETE CASCADE,
    track_id            TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    ordinal             INTEGER NOT NULL,
    PRIMARY KEY (imported_crate_id, track_id)
);
CREATE INDEX IF NOT EXISTS idx_imported_crate_tracks_ord
    ON imported_crate_tracks(imported_crate_id, ordinal);

CREATE TABLE IF NOT EXISTS play_history (
    id                  INTEGER PRIMARY KEY,
    track_id            TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    deck                INTEGER NOT NULL CHECK (deck IN (0, 1)),
    event_type          TEXT    NOT NULL CHECK (event_type IN
                        ('load', 'play_start', 'play_end',
                         'transition_in', 'transition_out')),
    timestamp_ms        INTEGER NOT NULL,
    duration_played_ms  INTEGER,
    from_track_id       TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    to_track_id         TEXT    REFERENCES tracks(id) ON DELETE SET NULL,
    session_id          TEXT
);
CREATE INDEX IF NOT EXISTS idx_play_history_track
    ON play_history(track_id, timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_play_history_timestamp
    ON play_history(timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_play_history_session
    ON play_history(session_id, timestamp_ms);

CREATE TABLE IF NOT EXISTS analysis_cache (
    fingerprint_id          INTEGER PRIMARY KEY
                            REFERENCES fingerprints(id) ON DELETE CASCADE,
    lufs_i                  REAL,
    true_peak_dbtp          REAL,
    waveform_sidecar_path   TEXT,
    has_lufs                INTEGER NOT NULL DEFAULT 0 CHECK (has_lufs IN (0, 1)),
    has_waveform            INTEGER NOT NULL DEFAULT 0 CHECK (has_waveform IN (0, 1)),
    has_active_grid         INTEGER NOT NULL DEFAULT 0 CHECK (has_active_grid IN (0, 1)),
    analyzed_at             INTEGER
);

CREATE TABLE IF NOT EXISTS smart_crates (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL UNIQUE,
    sql_predicate   TEXT    NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- FTS5 virtual table for substring search per PRD §8.5.4. Indexes
-- every per-source metadata row; search returns DISTINCT track_id.
-- Synced via triggers below.
CREATE VIRTUAL TABLE IF NOT EXISTS track_metadata_fts USING fts5(
    artist,
    title,
    album,
    comment,
    track_metadata_source_id UNINDEXED,
    track_id UNINDEXED,
    source UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS trg_metadata_fts_insert
AFTER INSERT ON track_metadata_source BEGIN
    INSERT INTO track_metadata_fts (
        artist, title, album, comment,
        track_metadata_source_id, track_id, source
    ) VALUES (
        new.artist, new.title, new.album, new.comment,
        new.id, new.track_id, new.source
    );
END;

CREATE TRIGGER IF NOT EXISTS trg_metadata_fts_delete
AFTER DELETE ON track_metadata_source BEGIN
    DELETE FROM track_metadata_fts
        WHERE track_metadata_source_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_metadata_fts_update
AFTER UPDATE ON track_metadata_source BEGIN
    UPDATE track_metadata_fts SET
        artist  = new.artist,
        title   = new.title,
        album   = new.album,
        comment = new.comment,
        source  = new.source
    WHERE track_metadata_source_id = old.id;
END;
"#;

/// Schema v2 migration — M11d.4 missing-file tracking (PRD §8.5.5).
///
/// Adds `track_files.is_missing` so the background scanner can flag
/// individual file rows without dropping them (PRD §8.5.5: "Metadata
/// is never deleted when a file goes missing"). Also stamps
/// `last_checked_at` so the rate-limiter knows when each row was
/// last probed, and a partial index for the "list missing files"
/// query pattern.
///
/// `last_checked_at` is nullable on purpose — rows imported under v1
/// schema have never been checked, and surfacing that as
/// "unknown / due for next scan" is more honest than back-stamping
/// the import time.
const V2_MIGRATION: &str = r#"
ALTER TABLE track_files ADD COLUMN is_missing INTEGER NOT NULL DEFAULT 0
    CHECK (is_missing IN (0, 1));
ALTER TABLE track_files ADD COLUMN last_checked_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_track_files_missing
    ON track_files(is_missing) WHERE is_missing = 1;

CREATE INDEX IF NOT EXISTS idx_track_files_last_checked
    ON track_files(last_checked_at);
"#;

/// V3 migration (M11c.2) — adds the `track_keys` table parallel to
/// `track_beatgrids` and a partial unique index that enforces "one
/// active key per track". Also extends `analysis_cache` with a
/// `has_active_key` flag so the prepared-flag predicate
/// (§8.3 / §8.5) can be computed without a join.
///
/// Why parallel to `track_beatgrids` rather than a column on
/// `tracks`: the same per-source/`is_active` shape applies (Serato
/// / Traktor / rekordbox can each claim a key; auto from M11c.2
/// runs alongside; future user override has its own source string).
/// One row per `(track_id, source)` keeps cross-source preservation
/// trivial and matches the structure the LibraryView's
/// disagreement indicator already understands from `track_beatgrids`.
///
/// `key_notation` is canonical Camelot (e.g. `8B`). `original_notation`
/// preserves whatever the source wrote verbatim (`C major`, `Cm`,
/// `5d`, `8B`) so rekordbox-XML export round-trips exactly.
/// `confidence` is `[0.0, 1.0]` for auto-detected rows; NULL for
/// imported / user rows (which by definition carry no algorithmic
/// confidence).
const V3_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS track_keys (
    id                INTEGER PRIMARY KEY,
    track_id          TEXT    NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    source            TEXT    NOT NULL CHECK (source IN
                      ('serato', 'traktor', 'rekordbox', 'itunes',
                       'mixedinkey', 'id3', 'auto', 'user')),
    key_notation      TEXT    NOT NULL,
    original_notation TEXT,
    confidence        REAL,
    is_active         INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    captured_at       INTEGER NOT NULL,
    UNIQUE (track_id, source)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_one_active_key_per_track
    ON track_keys(track_id) WHERE is_active = 1;
CREATE INDEX IF NOT EXISTS idx_track_keys_track_id
    ON track_keys(track_id);

ALTER TABLE analysis_cache ADD COLUMN has_active_key INTEGER NOT NULL DEFAULT 0
    CHECK (has_active_key IN (0, 1));
"#;

/// M11d.7 — per-track beatgrid lock + drift-quality indicator.
const V4_MIGRATION: &str = r#"
ALTER TABLE tracks ADD COLUMN grid_locked INTEGER NOT NULL DEFAULT 0
    CHECK (grid_locked IN (0, 1));
ALTER TABLE tracks ADD COLUMN grid_drift_quality REAL;
"#;

/// PRD-BEATS C2 (round 4) — beat-grid `bar_phase` becomes a
/// first-class scalar on every row in `track_beatgrids`.
///
/// `bar_phase ∈ [0, beats_per_bar)` is the index `i` such that
/// `beats[i]`, `beats[i + beats_per_bar]`, … are the downbeats
/// (bar position 1). Prior to v5 the column did not exist; phase
/// was implicit ("`beats[0]` is the downbeat") and "set the 1"
/// rebuilt the whole grid by re-anchoring. With v5, "set the 1"
/// becomes a pure rotation (bpm + anchor unchanged) and the
/// renderer reads phase explicitly via `(idx mod beats_per_bar)
/// == bar_phase`.
///
/// Default `0` preserves the v4 behaviour for any rows analyzed
/// before this migration: those rows already had `beats[0]` as
/// the downbeat (auto path shifted anchor; tap path used first
/// tap as anchor). The CHECK constraint enforces the 4/4
/// assumption that the rest of the codebase already encodes.
///
/// PRD-BEATS round 4 ships in dev only; per the user's
/// "we are not in production yet" go-ahead the migration is a
/// simple `ALTER TABLE` with a safe default rather than a
/// re-analysis-required schema bump.
const V5_MIGRATION: &str = r#"
ALTER TABLE track_beatgrids ADD COLUMN bar_phase INTEGER NOT NULL DEFAULT 0
    CHECK (bar_phase >= 0 AND bar_phase < 16);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Helper: open an in-memory DB and run the migration. Used by
    /// the migration / schema-version tests.
    fn fresh_db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory DB");
        open_and_migrate(&mut conn).expect("migration must succeed on fresh DB");
        conn
    }

    #[test]
    fn migration_v4_adds_grid_lock_columns() {
        let conn = fresh_db();
        let mut stmt = conn.prepare("PRAGMA table_info(tracks)").unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap();
        assert!(
            rows.iter().any(|c| c == "grid_locked"),
            "v4 migration must add grid_locked to tracks; saw {rows:?}"
        );
        assert!(
            rows.iter().any(|c| c == "grid_drift_quality"),
            "v4 migration must add grid_drift_quality to tracks; saw {rows:?}"
        );
    }

    #[test]
    fn migration_to_v1_lands_schema_version_row() {
        let conn = fresh_db();
        let v: u32 = conn
            .query_row("SELECT version FROM schema_version", [], |r| {
                r.get::<_, i64>(0).map(|i| i as u32)
            })
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn migration_is_idempotent_on_re_open() {
        // Run the migration twice. The second pass should be a no-op
        // (every CREATE uses IF NOT EXISTS) and leave the version
        // exactly at SCHEMA_VERSION.
        let mut conn = Connection::open_in_memory().unwrap();
        open_and_migrate(&mut conn).unwrap();
        open_and_migrate(&mut conn).unwrap();
        let v: u32 = conn
            .query_row("SELECT version FROM schema_version", [], |r| {
                r.get::<_, i64>(0).map(|i| i as u32)
            })
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn migration_creates_every_documented_table() {
        // Spot-check every table from docs/spec/LIBRARY-SCHEMA.md is
        // present after migration. Catches a typo in V1_SCHEMA at
        // build time rather than at "first import" time.
        let conn = fresh_db();
        for table in [
            "schema_version",
            "fingerprints",
            "tracks",
            "volumes",
            "track_files",
            "track_metadata_source",
            "track_beatgrids",
            "track_keys",
            "track_cues",
            "track_loops",
            "crates",
            "crate_tracks",
            "imported_crates",
            "imported_crate_tracks",
            "play_history",
            "analysis_cache",
            "smart_crates",
            "track_metadata_fts",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE name = ?1 AND type IN ('table', 'view')",
                    params![table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table {table} missing after migration");
        }
    }

    #[test]
    fn refuses_to_open_db_with_newer_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Stand up the schema_version table at a version higher
        // than this binary supports (simulating a v1.x DB opened
        // by a v1.0 binary per PRD §8.7).
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at INTEGER NOT NULL); \
             INSERT INTO schema_version (version, applied_at) VALUES (999, 0);",
        )
        .unwrap();
        match open_and_migrate(&mut conn) {
            Err(LibraryError::SchemaTooNew { found, supported }) => {
                assert_eq!(found, 999);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected SchemaTooNew, got {other:?}"),
        }
    }

    #[test]
    fn fts_trigger_propagates_metadata_inserts() {
        // Sanity-check the FTS sync triggers — insert a track + a
        // metadata row, then assert the FTS table sees it. This is
        // a load-bearing test for M11d browser search; we don't
        // want to discover a busted trigger when the importer is
        // already mid-flight.
        let conn = fresh_db();
        // Need a volume + a track row first to satisfy FK.
        let now = 1_700_000_000_i64;
        conn.execute(
            "INSERT INTO volumes (volume_uuid, display_name, last_seen_at) \
             VALUES ('TEST-VOLUME', 'Macintosh HD', ?1)",
            params![now],
        )
        .unwrap();
        let track_uuid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tracks (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO track_metadata_source \
             (track_id, source, artist, title, imported_at) \
             VALUES (?1, 'id3', 'J Dilla', 'Donuts', ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        // Substring query over FTS5.
        let hits: Vec<String> = conn
            .prepare(
                "SELECT track_id FROM track_metadata_fts \
                 WHERE track_metadata_fts MATCH 'donuts'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(hits, vec![track_uuid]);
    }

    #[test]
    fn one_active_key_per_track_constraint_holds() {
        // V3 mirror of the active-grid constraint. The partial
        // unique index must reject a second `is_active = 1` row
        // for the same track — two active keys would be a data-
        // correctness bug that every browser query would step on.
        let conn = fresh_db();
        let now = 1_700_000_000_i64;
        conn.execute(
            "INSERT INTO volumes (volume_uuid, display_name, last_seen_at) \
             VALUES ('TEST-VOLUME', 'Macintosh HD', ?1)",
            params![now],
        )
        .unwrap();
        let track_uuid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tracks (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO track_keys \
             (track_id, source, key_notation, original_notation, confidence, is_active, captured_at) \
             VALUES (?1, 'serato', '8B', 'C major', NULL, 1, ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO track_keys \
             (track_id, source, key_notation, original_notation, confidence, is_active, captured_at) \
             VALUES (?1, 'auto', '5A', '5A', 0.7, 1, ?2)",
            params![track_uuid, now],
        );
        assert!(
            result.is_err(),
            "two active keys on one track must be rejected"
        );
    }

    #[test]
    fn track_beatgrids_has_bar_phase_column_lands_on_v5() {
        // PRD-BEATS C2 (round 4): the V5 migration must add a
        // non-null `bar_phase` column with a default of 0 to
        // `track_beatgrids`. The default is what lets the
        // migration land on an existing dev DB without forcing a
        // re-analyze of every track (anything analyzed before v5
        // already had `beats[0]` as the downbeat → bar_phase 0).
        let conn = fresh_db();
        let mut stmt = conn.prepare("PRAGMA table_info(track_beatgrids)").unwrap();
        let cols = stmt
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i32>(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<(String, i32)>>>()
            .unwrap();
        let bar_phase = cols.iter().find(|(name, _)| name == "bar_phase");
        let (_, notnull) = bar_phase.unwrap_or_else(|| {
            panic!("v5 migration must add bar_phase to track_beatgrids; saw {cols:?}")
        });
        assert_eq!(
            *notnull, 1,
            "bar_phase must be NOT NULL so renderer code never sees a sentinel"
        );

        // The default must be 0 so a re-open of an existing v4 DB
        // doesn't suddenly drop every row's downbeat phase to NULL.
        // Insert a row without supplying bar_phase; expect 0.
        let now = 1_700_000_000_i64;
        conn.execute(
            "INSERT INTO volumes (volume_uuid, display_name, last_seen_at) \
             VALUES ('TEST-VOLUME', 'Macintosh HD', ?1)",
            params![now],
        )
        .unwrap();
        let track_uuid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tracks (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO track_beatgrids \
             (track_id, source, anchor_secs, bpm, is_active, captured_at) \
             VALUES (?1, 'auto', 0.0, 120.0, 1, ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        let stored: i64 = conn
            .query_row(
                "SELECT bar_phase FROM track_beatgrids WHERE track_id = ?1",
                params![track_uuid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, 0, "v5 default for bar_phase must be 0");
    }

    #[test]
    fn analysis_cache_has_active_key_column_lands_on_v3() {
        // Smoke test: the V3 migration must add `has_active_key` to
        // `analysis_cache`. A direct `SELECT has_active_key` would
        // be a compile-time TODO; we check column existence via
        // `PRAGMA table_info` so the test breaks loudly if a future
        // migration drops the column.
        let conn = fresh_db();
        let mut stmt = conn.prepare("PRAGMA table_info(analysis_cache)").unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap();
        assert!(
            rows.iter().any(|c| c == "has_active_key"),
            "v3 migration must add has_active_key to analysis_cache; saw {rows:?}"
        );
    }

    #[test]
    fn one_active_grid_per_track_constraint_holds() {
        // The partial unique index `idx_one_active_grid_per_track`
        // must reject a second `is_active = 1` row for the same
        // track. Two active grids would be a data-correctness bug
        // every browser query would step on.
        let conn = fresh_db();
        let now = 1_700_000_000_i64;
        conn.execute(
            "INSERT INTO volumes (volume_uuid, display_name, last_seen_at) \
             VALUES ('TEST-VOLUME', 'Macintosh HD', ?1)",
            params![now],
        )
        .unwrap();
        let track_uuid = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tracks (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO track_beatgrids \
             (track_id, source, anchor_secs, bpm, is_active, captured_at) \
             VALUES (?1, 'serato', 0.0, 92.0, 1, ?2)",
            params![track_uuid, now],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO track_beatgrids \
             (track_id, source, anchor_secs, bpm, is_active, captured_at) \
             VALUES (?1, 'auto', 0.0, 184.0, 1, ?2)",
            params![track_uuid, now],
        );
        assert!(
            result.is_err(),
            "two active grids on one track must be rejected"
        );
    }
}
