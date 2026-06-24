//! iTunes / Apple Music `Library.xml` import adapter (M12c).
//!
//! Bridges [`crate::itunes`] to the schema: resolves each track's `Location`
//! path to the shared identity (idempotent by `(volume, relative_path)`,
//! same as the other importers), writes a `source = 'itunes'` metadata row,
//! and mirrors user playlists / folders into `imported_crates`. iTunes has no
//! beat grids or cues, so only metadata + playlists are imported.
//!
//! Built-in playlists (the `Library` master and the distinguished
//! Music / Films / Downloaded / … lists) are skipped — only user-created
//! playlists and folders become crate nodes. Lazy + read-only like the other
//! importers; missing files land in [`ImportSummary::skipped`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use uuid::Uuid;

use crate::db::Library;
use crate::error::{LibraryError, Result};
use crate::importer::{detect_codec_from_extension, ImportError, ImportSummary};
use crate::itunes::{self, ItunesPlaylist, ItunesTrack};
use crate::volumes::discover_for_path;

const ITUNES: &str = "itunes";

/// Import an iTunes / Apple Music `Library.xml`.
///
/// Idempotent. # Errors: [`LibraryError`] only on an unreadable / unparseable
/// file; per-track failures accumulate in [`ImportSummary::errors`].
pub fn import_itunes(library: &mut Library, xml_path: &Path) -> Result<ImportSummary> {
    let data = std::fs::read(xml_path).map_err(|e| LibraryError::io(xml_path, e))?;
    let lib = itunes::parse_library(&data).map_err(|e| {
        LibraryError::io(
            xml_path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        )
    })?;

    let mut summary = ImportSummary::default();
    // iTunes track id → our canonical uuid, for the playlist join.
    let mut id_map: HashMap<i64, String> = HashMap::new();

    for track in &lib.tracks {
        let Some(path) = track.path.clone() else {
            summary.skipped += 1;
            summary.errors.push(ImportError {
                path: PathBuf::new(),
                reason: "track has no file:// Location".to_string(),
            });
            continue;
        };
        match import_track(library, &path, track) {
            Ok((uuid, added)) => {
                if added {
                    summary.added += 1;
                } else {
                    summary.refreshed += 1;
                }
                id_map.insert(track.track_id, uuid);
            }
            Err(reason) => {
                summary.skipped += 1;
                summary.errors.push(ImportError {
                    path: PathBuf::from(path),
                    reason,
                });
            }
        }
    }

    import_playlists(library, &lib.playlists, &id_map)?;
    Ok(summary)
}

/// Resolve + persist one iTunes track, returning its canonical uuid and
/// whether a new identity was minted (`true`) vs an existing one reused.
fn import_track(
    library: &Library,
    path: &str,
    track: &ItunesTrack,
) -> std::result::Result<(String, bool), String> {
    let abs = Path::new(path);
    let volume = discover_for_path(abs).map_err(|e| format!("volume UUID unavailable: {e}"))?;
    library.upsert_volume(&volume).map_err(|e| e.to_string())?;
    let relative_path = volume
        .relative_to(abs)
        .ok_or_else(|| format!("path {abs:?} not under volume {:?}", volume.mount_point))?;

    let (file_size, mtime) = match std::fs::metadata(abs) {
        Ok(stats) => (
            Some(stats.len()),
            stats
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        ),
        Err(_) => (None, None),
    };

    let (track_id, added) = match library
        .find_track_file_owner(&volume.volume_uuid, &relative_path)
        .map_err(|e| e.to_string())?
    {
        Some(existing) => (existing, false),
        None => {
            let uuid = Uuid::new_v4().to_string();
            library
                .insert_track(&uuid, None, None, None)
                .map_err(|e| e.to_string())?;
            (uuid, true)
        }
    };

    // Provisional duration from iTunes' own `Total Time` so the browser shows
    // a length before the track is decoded (both fresh + re-scanned tracks;
    // only fills a NULL — analysis later overwrites with the decoded value).
    if let Some(ms) = track.total_time_ms.and_then(|ms| u32::try_from(ms).ok()) {
        library
            .set_duration_if_absent(&track_id, ms)
            .map_err(|e| e.to_string())?;
    }

    library
        .upsert_track_file(
            &track_id,
            &volume.volume_uuid,
            &relative_path,
            detect_codec_from_extension(abs),
            None,
            None,
            None,
            file_size,
            mtime,
        )
        .map_err(|e| e.to_string())?;

    library
        .upsert_metadata_source(
            &track_id,
            ITUNES,
            track.artist.as_deref(),
            track.name.as_deref(),
            track.album.as_deref(),
            track.genre.as_deref(),
            None, // comment
            track.composer.as_deref(),
            track.year,
            None, // track_number — not parsed yet
            track.bpm,
            None, // key — iTunes has none
            None, // gain
            None, // rating
            None, // version_token
        )
        .map_err(|e| e.to_string())?;

    Ok((track_id, added))
}

/// Rebuild the iTunes crate mirror from user playlists / folders, skipping the
/// `Library` master and the distinguished built-ins.
fn import_playlists(
    library: &mut Library,
    playlists: &[ItunesPlaylist],
    id_map: &HashMap<i64, String>,
) -> Result<()> {
    library.clear_imported_crates(ITUNES)?;

    // Importable playlists by persistent id (for parent lookup).
    let by_pid: HashMap<&str, &ItunesPlaylist> = playlists
        .iter()
        .filter(|p| importable(p))
        .filter_map(|p| p.persistent_id.as_deref().map(|pid| (pid, p)))
        .collect();

    // Memoised persistent id → created crate id (handles forward parent refs).
    let mut crate_ids: HashMap<String, i64> = HashMap::new();
    for pl in playlists.iter().filter(|p| importable(p)) {
        let crate_id = ensure_crate(library, pl, &by_pid, &mut crate_ids)?;
        if !pl.is_folder {
            for (ordinal, track_id) in pl
                .track_ids
                .iter()
                .filter_map(|id| id_map.get(id))
                .enumerate()
            {
                library.add_track_to_imported_crate(crate_id, track_id, ordinal as i64)?;
            }
        }
    }
    Ok(())
}

/// A playlist Dub mirrors: not the master library, not a distinguished
/// built-in, and named.
fn importable(p: &ItunesPlaylist) -> bool {
    !p.is_master && !p.distinguished && !p.name.is_empty()
}

/// Ensure a playlist's crate (and its parent chain) exists; return its id.
fn ensure_crate(
    library: &mut Library,
    pl: &ItunesPlaylist,
    by_pid: &HashMap<&str, &ItunesPlaylist>,
    crate_ids: &mut HashMap<String, i64>,
) -> Result<i64> {
    if let Some(pid) = pl.persistent_id.as_deref() {
        if let Some(&existing) = crate_ids.get(pid) {
            return Ok(existing);
        }
    }
    // Resolve the parent first, if it points at an importable playlist.
    let parent_id = match pl.parent_persistent_id.as_deref() {
        Some(parent_pid) => match by_pid.get(parent_pid) {
            Some(parent_pl) => Some(ensure_crate(library, parent_pl, by_pid, crate_ids)?),
            None => None,
        },
        None => None,
    };
    let id = library.create_imported_crate(ITUNES, &pl.name, parent_id)?;
    if let Some(pid) = pl.persistent_id.as_deref() {
        crate_ids.insert(pid.to_string(), id);
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in end-to-end import into a throwaway temp library (never the
    /// developer's real DB). `DUB_ITUNES_XML=… cargo test … itunes_import_real
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore = "set DUB_ITUNES_XML to a real iTunes Library.xml"]
    fn itunes_import_real() {
        let Ok(path) = std::env::var("DUB_ITUNES_XML") else {
            eprintln!("DUB_ITUNES_XML unset — skipping");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open_at(&tmp.path().join("library.sqlite")).unwrap();
        let summary = import_itunes(&mut lib, Path::new(&path)).expect("itunes import");
        eprintln!(
            "added={} refreshed={} skipped={} errors={}",
            summary.added,
            summary.refreshed,
            summary.skipped,
            summary.errors.len()
        );
        let count =
            |sql: &str| -> i64 { lib.connection().query_row(sql, [], |r| r.get(0)).unwrap() };
        eprintln!(
            "tracks={} itunes-meta={} crates={} members={}",
            count("SELECT COUNT(*) FROM tracks"),
            count("SELECT COUNT(*) FROM track_metadata_source WHERE source='itunes'"),
            count("SELECT COUNT(*) FROM imported_crates WHERE source='itunes'"),
            count("SELECT COUNT(*) FROM imported_crate_tracks"),
        );
    }
}
