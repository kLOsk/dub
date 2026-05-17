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
}
