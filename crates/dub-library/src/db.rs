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
