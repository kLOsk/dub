//! Serato 4 / 5 library import (2026-11-18).
//!
//! Serato DJ 4 moved its library out of `_Serato_/database V2` +
//! `Subcrates/*.crate` into SQLite at
//! `~/Library/Application Support/Serato/Library/master.sqlite`, and stopped
//! writing the old files. There is no public spec; the layout here was read
//! off a library Serato DJ Pro 4.0.7 wrote (see `LIBRARY-FORMATS.md`):
//!
//! - `asset`: one row per track. `portable_id` is the file path relative to
//!   its `location`'s root, with title / artist / album / genre / BPM / key.
//! - `location.path`: that root. Empty for the system drive (`/`).
//! - `space`: Serato's sources. The DJ's own library is the space named
//!   `Serato Library`; the others (`iTunes`, `Tidal`, …) are Serato's views
//!   of other places and are not imported here. Apple Music has its own
//!   source in Dub.
//! - `container`: crates, a tree by `parent_id`, in `list_order`. A
//!   container with a `smart_crate_rules` row is a smart crate and is
//!   skipped: its rules are Serato's to evaluate.
//! - `container_asset` → `location_container` → `container`: membership.
//!
//! Cues, loops and the beat grid are not in the database: Serato keeps them
//! in the audio file's GEOB tags as before, so each track goes through the
//! same [`crate::serato_import`] path the Serato 3 reader uses.
//!
//! The database is in WAL mode and Serato may have it open. Dub copies it
//! (with its `-wal`) to a temporary folder and reads the copy, so it never
//! opens, locks or checkpoints Serato's own file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

use crate::db::Library;
use crate::error::{LibraryError, Result};
use crate::importer::{ImportError, ImportSummary};
use crate::serato::database::SeratoEntry;
use crate::serato_import::{import_entry, SERATO};
use crate::volumes::discover_for_path;

/// The DJ's own library among Serato's spaces.
const SERATO_LIBRARY_SPACE: &str = "Serato Library";

/// Import a Serato 4 / 5 `master.sqlite`.
///
/// Idempotent, like the Serato 3 reader: a re-import refreshes tracks in
/// place and rebuilds the crate mirror.
///
/// # Errors
/// [`LibraryError`] when the database can't be copied, opened or read.
/// Per-track failures (a file on a drive that isn't plugged in) accumulate
/// in [`ImportSummary::errors`].
pub fn import_serato_sqlite(library: &mut Library, master: &Path) -> Result<ImportSummary> {
    let copy = SnapshotCopy::of(master)?;
    let conn = Connection::open_with_flags(
        copy.db(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| LibraryError::sqlite("open_serato_master", e))?;

    let Some(space_id) = conn
        .query_row(
            "SELECT id FROM space WHERE name = ?1 COLLATE NOCASE",
            params![SERATO_LIBRARY_SPACE],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| LibraryError::sqlite("serato_space", e))?
    else {
        // A Serato that has never had a track added: nothing to read.
        library.clear_imported_crates(SERATO)?;
        return Ok(ImportSummary::default());
    };

    let mut summary = ImportSummary::default();
    let mut resolved: HashMap<i64, String> = HashMap::new();
    for asset in read_assets(&conn, space_id)? {
        let abs = asset.root.join(&asset.entry_path);
        match import_asset(library, &abs, &asset.entry) {
            Ok((track_id, added)) => {
                if added {
                    summary.added += 1;
                } else {
                    summary.refreshed += 1;
                }
                resolved.insert(asset.id, track_id);
            }
            Err(reason) => {
                summary.skipped += 1;
                summary.errors.push(ImportError { path: abs, reason });
            }
        }
    }

    import_crates(library, &conn, space_id, &resolved)?;
    Ok(summary)
}

/// One `asset` row, as the Serato 3 pipeline wants it.
struct Asset {
    id: i64,
    root: PathBuf,
    entry_path: String,
    entry: SeratoEntry,
}

fn read_assets(conn: &Connection, space_id: i64) -> Result<Vec<Asset>> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, COALESCE(l.path, ''), a.portable_id, a.name, a.artist, a.album, \
                    a.genre, a.comments, a.composer, a.grouping, a.label, a.key, a.bpm \
             FROM asset a \
             JOIN space_asset sa ON sa.asset_id = a.id AND sa.space_id = ?1 \
             LEFT JOIN location l ON l.id = a.location_id \
             WHERE a.portable_id <> '' \
             ORDER BY a.id",
        )
        .map_err(|e| LibraryError::sqlite("prepare_serato_assets", e))?;
    let text = |s: Option<String>| s.filter(|v| !v.trim().is_empty());
    let rows = stmt
        .query_map(params![space_id], |r| {
            let root: String = r.get(1)?;
            let entry_path: String = r.get(2)?;
            Ok(Asset {
                id: r.get(0)?,
                root: location_root(&root),
                entry: SeratoEntry {
                    file_path: Some(entry_path.clone()),
                    title: text(r.get(3)?),
                    artist: text(r.get(4)?),
                    album: text(r.get(5)?),
                    genre: text(r.get(6)?),
                    comment: text(r.get(7)?),
                    composer: text(r.get(8)?),
                    grouping: text(r.get(9)?),
                    label: text(r.get(10)?),
                    key: text(r.get(11)?),
                    bpm: r.get::<_, Option<f64>>(12)?.filter(|b| *b > 0.0),
                    ..SeratoEntry::default()
                },
                entry_path,
            })
        })
        .map_err(|e| LibraryError::sqlite("query_serato_assets", e))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| LibraryError::sqlite("read_serato_assets", e))
}

/// Where a `location` is mounted. Empty is the system drive. An absolute
/// path is taken as given; a bare name is read as a drive under `/Volumes`.
/// The library seen so far only had the system drive, so the last rule is
/// a guess, and a track it gets wrong is skipped as not found, never
/// misfiled.
fn location_root(path: &str) -> PathBuf {
    if path.is_empty() {
        PathBuf::from("/")
    } else if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        Path::new("/Volumes").join(path)
    }
}

/// Resolve the file to the volume it is really on (as every importer does,
/// so a Serato track and the same file found any other way are one track),
/// then hand it to the Serato 3 per-track path: metadata, key, and the grid
/// and cues from the file's tags.
fn import_asset(
    library: &Library,
    abs: &Path,
    entry: &SeratoEntry,
) -> std::result::Result<(String, bool), String> {
    let relocated = crate::relocate::relocate(abs);
    let abs = relocated.as_path();
    if !abs.is_file() {
        return Err(format!("file not found at {abs:?}"));
    }
    let volume = discover_for_path(abs).map_err(|e| format!("volume UUID unavailable: {e}"))?;
    library.upsert_volume(&volume).map_err(|e| e.to_string())?;
    let rel = volume
        .relative_to(abs)
        .ok_or_else(|| format!("path {abs:?} not under volume {:?}", volume.mount_point))?;
    let outcome = import_entry(library, &volume, &rel, entry)?;
    Ok((outcome.track_id, outcome.added))
}

/// Rebuild the Serato crate mirror from the `Serato Library` container
/// tree, skipping smart crates (and anything under one).
fn import_crates(
    library: &mut Library,
    conn: &Connection,
    space_id: i64,
    resolved: &HashMap<i64, String>,
) -> Result<()> {
    library.clear_imported_crates(SERATO)?;

    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.parent_id, c.name, \
                    EXISTS (SELECT 1 FROM smart_crate_rules r WHERE r.container_id = c.id) \
             FROM container c \
             WHERE c.space_id = ?1 \
             ORDER BY c.list_order, c.id",
        )
        .map_err(|e| LibraryError::sqlite("prepare_serato_crates", e))?;
    let containers: Vec<Container> = stmt
        .query_map(params![space_id], |r| {
            Ok(Container {
                id: r.get(0)?,
                parent: r.get(1)?,
                name: r.get(2)?,
                smart: r.get(3)?,
            })
        })
        .map_err(|e| LibraryError::sqlite("query_serato_crates", e))?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| LibraryError::sqlite("read_serato_crates", e))?;

    // The space's root container (its parent is Serato's global root, not
    // in this space) holds the crates; it is not one itself.
    let in_space: HashSet<i64> = containers.iter().map(|c| c.id).collect();
    let mut children: HashMap<Option<i64>, Vec<&Container>> = HashMap::new();
    for c in &containers {
        let parent = c.parent.filter(|p| in_space.contains(p));
        children.entry(parent).or_default().push(c);
    }

    let mut members = conn
        .prepare(
            "SELECT ca.asset_id FROM container_asset ca \
             JOIN location_container lc ON lc.id = ca.location_container_id \
             WHERE lc.container_id = ?1 \
             ORDER BY ca.list_order, ca.id",
        )
        .map_err(|e| LibraryError::sqlite("prepare_serato_members", e))?;

    // Walk down from the space roots in list order. A smart crate and all
    // under it are skipped; `seen` stops a parent loop Serato should never
    // write from spinning.
    let mut seen: HashSet<i64> = HashSet::new();
    let mut stack: Vec<(&Container, Option<i64>)> = Vec::new();
    for root in children.get(&None).into_iter().flatten().rev() {
        seen.insert(root.id);
        for c in children.get(&Some(root.id)).into_iter().flatten().rev() {
            stack.push((c, None));
        }
    }
    while let Some((c, parent_dub)) = stack.pop() {
        if c.smart || !seen.insert(c.id) {
            continue;
        }
        let crate_id = library.create_imported_crate(SERATO, &c.name, parent_dub)?;
        let assets: Vec<i64> = members
            .query_map(params![c.id], |r| r.get::<_, i64>(0))
            .map_err(|e| LibraryError::sqlite("query_serato_members", e))?
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| LibraryError::sqlite("read_serato_members", e))?;
        let tracks = assets.iter().filter_map(|a| resolved.get(a));
        for (ordinal, track_id) in tracks.enumerate() {
            library.add_track_to_imported_crate(crate_id, track_id, ordinal as i64)?;
        }
        for child in children.get(&Some(c.id)).into_iter().flatten().rev() {
            stack.push((child, Some(crate_id)));
        }
    }
    Ok(())
}

/// A `container` row in the DJ's Serato Library.
struct Container {
    id: i64,
    parent: Option<i64>,
    name: String,
    smart: bool,
}

/// A private copy of Serato's database (and its write-ahead log, which
/// holds whatever Serato wrote since its last checkpoint), removed when
/// the import is done.
struct SnapshotCopy {
    dir: PathBuf,
}

impl SnapshotCopy {
    fn of(master: &Path) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("dub-serato-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|e| LibraryError::io(&dir, e))?;
        let copy = Self { dir };
        std::fs::copy(master, copy.db()).map_err(|e| LibraryError::io(master, e))?;
        let wal = sidecar(master, "-wal");
        if wal.is_file() {
            std::fs::copy(&wal, sidecar(&copy.db(), "-wal"))
                .map_err(|e| LibraryError::io(&wal, e))?;
        }
        Ok(copy)
    }

    fn db(&self) -> PathBuf {
        self.dir.join("master.sqlite")
    }
}

impl Drop for SnapshotCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The Serato 4 / 5 database under `home`, if there is one.
pub(crate) fn master_path(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join("Serato")
        .join("Library")
        .join("master.sqlite")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables and columns the reader touches, as Serato DJ Pro 4.0.7
    /// declares them (cut down to those columns).
    const SCHEMA: &str = "
        CREATE TABLE location (id INTEGER PRIMARY KEY, path TEXT);
        CREATE TABLE space (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
        CREATE TABLE asset (
            id INTEGER PRIMARY KEY AUTOINCREMENT, location_id INTEGER NOT NULL,
            portable_id TEXT NOT NULL DEFAULT '', name TEXT NOT NULL DEFAULT '',
            artist TEXT NOT NULL DEFAULT '', album TEXT NOT NULL DEFAULT '',
            genre TEXT NOT NULL DEFAULT '', comments TEXT NOT NULL DEFAULT '',
            composer TEXT NOT NULL DEFAULT '', grouping TEXT NOT NULL DEFAULT '',
            label TEXT NOT NULL DEFAULT '', key TEXT NOT NULL DEFAULT '', bpm REAL);
        CREATE TABLE space_asset (id INTEGER PRIMARY KEY, asset_id INTEGER, space_id INTEGER);
        CREATE TABLE container (
            id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER, name TEXT NOT NULL,
            type INTEGER DEFAULT 1, list_order INTEGER NOT NULL, space_id INTEGER);
        CREATE TABLE location_container (id INTEGER PRIMARY KEY, container_id INTEGER, location_id INTEGER);
        CREATE TABLE container_asset (
            id INTEGER PRIMARY KEY AUTOINCREMENT, asset_id INTEGER, location_container_id INTEGER,
            space_asset_id INTEGER, list_order INTEGER NOT NULL);
        CREATE TABLE smart_crate_rules (container_id INTEGER PRIMARY KEY, version INTEGER, rules TEXT);
    ";

    /// A Serato 4 library over `files` (absolute), on the system drive:
    /// two crates, one subcrate, a smart crate, and an iTunes-space track
    /// that is Serato's, not the DJ's.
    fn write_master(path: &Path, files: &[PathBuf], itunes_file: &Path) {
        let c = Connection::open(path).unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c.execute_batch(
            "INSERT INTO location VALUES (1, '');
             INSERT INTO space VALUES (5, 'Serato Library'), (13, 'iTunes');
             INSERT INTO container VALUES (0, NULL, 'root', 0, 0, NULL);
             INSERT INTO container VALUES (5, 0, 'Serato Library root', 0, 0, 5);
             INSERT INTO container VALUES (40, 5, 'Reggae', 1, 1, 5);
             INSERT INTO container VALUES (41, 40, 'Roots', 1, 0, 5);
             INSERT INTO container VALUES (42, 5, 'Hip Hop', 1, 0, 5);
             INSERT INTO container VALUES (43, 5, 'Stems', 2, 2, 5);
             INSERT INTO smart_crate_rules VALUES (43, 1, '{}');
             INSERT INTO location_container VALUES (1, 40, 1), (2, 41, 1), (3, 42, 1), (4, 43, 1);",
        )
        .unwrap();
        let rel = |p: &Path| p.to_string_lossy().trim_start_matches('/').to_string();
        for (i, f) in files.iter().enumerate() {
            let id = i as i64 + 1;
            c.execute(
                "INSERT INTO asset (id, location_id, portable_id, name, artist, key, bpm) \
                 VALUES (?1, 1, ?2, ?3, 'Artist', 'Em', 91.0)",
                params![id, rel(f), format!("Track {id}")],
            )
            .unwrap();
            c.execute("INSERT INTO space_asset VALUES (?1, ?1, 5)", params![id])
                .unwrap();
        }
        c.execute(
            "INSERT INTO asset (id, location_id, portable_id, name) VALUES (99, 1, ?1, 'iTunes only')",
            params![rel(itunes_file)],
        )
        .unwrap();
        c.execute("INSERT INTO space_asset VALUES (99, 99, 13)", [])
            .unwrap();
        // Reggae: 2 then 1. Roots: 3. Hip Hop: 1. Stems (smart): 2.
        c.execute_batch(
            "INSERT INTO container_asset (asset_id, location_container_id, space_asset_id, list_order)
                 VALUES (2, 1, 2, 0), (1, 1, 1, 1), (3, 2, 3, 0), (1, 3, 1, 0), (2, 4, 2, 0);",
        )
        .unwrap();
    }

    fn crate_tree(lib: &Library) -> Vec<(String, Option<String>, Vec<String>)> {
        let mut stmt = lib
            .connection()
            .prepare(
                "SELECT c.id, c.name, p.name FROM imported_crates c \
                 LEFT JOIN imported_crates p ON p.id = c.parent_imported_crate_id \
                 WHERE c.source = 'serato' ORDER BY c.id",
            )
            .unwrap();
        let rows: Vec<(i64, String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // Each crate's own members, as stored: `imported_crate_tracks`
        // also lists what its subcrates hold, which is the browser's
        // view, not what the importer wrote.
        let own: std::collections::HashSet<(i64, String)> = lib
            .connection()
            .prepare("SELECT imported_crate_id, track_id FROM imported_crate_tracks")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.into_iter()
            .map(|(id, name, parent)| {
                let titles = lib
                    .imported_crate_tracks(id)
                    .unwrap()
                    .into_iter()
                    .filter(|t| own.contains(&(id, t.id.clone())))
                    .map(|t| t.title.unwrap_or_default())
                    .collect();
                (name, parent, titles)
            })
            .collect()
    }

    /// Tracks, crates in Serato's order, subcrates under their parent;
    /// smart crates and Serato's view of iTunes left out; a re-import adds
    /// nothing; a file that isn't there is skipped, not invented.
    #[test]
    fn reads_a_serato_4_library() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        let files: Vec<PathBuf> = (1..=3).map(|i| dir.join(format!("t{i}.mp3"))).collect();
        for f in &files {
            std::fs::write(f, b"not really audio").unwrap();
        }
        let itunes = dir.join("itunes.mp3");
        std::fs::write(&itunes, b"x").unwrap();
        let mut with_missing = files.clone();
        with_missing.push(dir.join("missing.mp3"));
        let master = dir.join("master.sqlite");
        write_master(&master, &with_missing, &itunes);

        let mut lib = Library::open_at(&dir.join("library.sqlite")).unwrap();
        let summary = import_serato_sqlite(&mut lib, &master).unwrap();
        assert_eq!((summary.added, summary.skipped), (3, 1), "{summary:?}");
        assert_eq!(lib.count_tracks_by_source(SERATO).unwrap(), 3);

        let tree = crate_tree(&lib);
        let names: Vec<_> = tree
            .iter()
            .map(|c| (c.0.as_str(), c.1.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("Hip Hop", None),
                ("Reggae", None),
                ("Roots", Some("Reggae"))
            ],
            "list order, the subcrate under its parent, no smart crate"
        );
        assert_eq!(
            tree[1].2,
            ["Track 2", "Track 1"],
            "Serato's order inside a crate"
        );
        assert_eq!(tree[2].2, ["Track 3"]);

        let again = import_serato_sqlite(&mut lib, &master).unwrap();
        assert_eq!(again.added, 0);
        assert_eq!(crate_tree(&lib).len(), 3, "rebuilt, not doubled");
        assert!(std::fs::read_dir(&dir).unwrap().count() > 0);
    }

    /// The reader never writes to Serato's file: the database it was given
    /// is byte-for-byte the same afterwards.
    #[test]
    fn leaves_seratos_file_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        let f = dir.join("t1.mp3");
        std::fs::write(&f, b"x").unwrap();
        let master = dir.join("master.sqlite");
        write_master(&master, &[f], &dir.join("none.mp3"));
        let before = std::fs::read(&master).unwrap();
        let mut lib = Library::open_at(&dir.join("library.sqlite")).unwrap();
        import_serato_sqlite(&mut lib, &master).unwrap();
        assert_eq!(std::fs::read(&master).unwrap(), before);
        assert!(!sidecar(&master, "-wal").exists());
    }

    #[test]
    fn location_roots() {
        assert_eq!(location_root(""), PathBuf::from("/"));
        assert_eq!(location_root("/Volumes/DJ"), PathBuf::from("/Volumes/DJ"));
        assert_eq!(location_root("DJ"), PathBuf::from("/Volumes/DJ"));
    }

    /// Opt-in run against a real Serato 4 / 5 library.
    /// `DUB_SERATO_MASTER=…/master.sqlite cargo nextest run -p dub-library
    /// serato_sqlite_real --run-ignored only --no-capture`.
    #[test]
    #[ignore = "set DUB_SERATO_MASTER to a real master.sqlite"]
    fn serato_sqlite_real() {
        let Ok(path) = std::env::var("DUB_SERATO_MASTER") else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open_at(&tmp.path().join("library.sqlite")).unwrap();
        let s = import_serato_sqlite(&mut lib, Path::new(&path)).expect("import");
        eprintln!(
            "added={} skipped={} errors={:?}",
            s.added,
            s.skipped,
            s.errors.iter().take(5).collect::<Vec<_>>()
        );
        let count =
            |sql: &str| -> i64 { lib.connection().query_row(sql, [], |r| r.get(0)).unwrap() };
        eprintln!(
            "grids={} keys={} cues={} crates={}",
            count("SELECT COUNT(*) FROM track_beatgrids WHERE source='serato'"),
            count("SELECT COUNT(*) FROM track_keys WHERE source='serato'"),
            count("SELECT COUNT(*) FROM track_cues WHERE source='serato'"),
            count("SELECT COUNT(*) FROM imported_crates WHERE source='serato'"),
        );
    }
}
