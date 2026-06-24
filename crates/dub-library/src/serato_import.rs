//! Serato library import adapter (M11e).
//!
//! Bridges the pure [`crate::serato`] parsers to the SQLite schema. Given a
//! `_Serato_` folder it:
//!
//! 1. Resolves the folder's volume once. Every `database V2` `pfil` is
//!    already a path **relative to that volume root**, i.e. the exact
//!    `track_files.relative_path` Dub stores — so a Serato track resolves to
//!    the *same* identity as the folder/Traktor importers (no duplicates),
//!    and its absolute path is `mount_point + pfil`.
//! 2. Writes a `source = 'serato'` metadata row, then reads the file's ID3
//!    `GEOB` frames and writes the imported beat grid (`Serato BeatGrid`),
//!    hot cues / loops (`Serato Markers2`), key (Camelot-converted), and gain
//!    (`Serato Autotags`).
//! 3. Mirrors `Subcrates/*.crate` into the read-only `imported_crates` tree
//!    (folder nesting from the `%%` filename convention).
//!
//! Lazy like the other importers (no decode / fingerprint at import). Files
//! that aren't present on disk are reported in [`ImportSummary::skipped`].
//! Read-only on the Serato data and the audio files (only `stat` + a tag
//! read).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use uuid::Uuid;

use crate::db::Library;
use crate::error::{LibraryError, Result};
use crate::importer::{detect_codec_from_extension, ImportError, ImportSummary};
use crate::serato::{autotags, beatgrid, database, geob, markers2};
use crate::volumes::{discover_for_path, DiscoveredVolume};

const SERATO: &str = "serato";

/// Import a `_Serato_` library folder (`~/Music/_Serato_` or a drive's
/// `/Volumes/<drive>/_Serato_`).
///
/// Idempotent: a re-import refreshes metadata / grids / cues / loops in place
/// and rebuilds the crate mirror without duplicating identities.
///
/// # Errors
/// Returns [`LibraryError`] only on a hard failure (the folder's volume can't
/// be resolved, or `database V2` can't be read). Per-track failures (a missing
/// file) accumulate in [`ImportSummary::errors`].
pub fn import_serato(library: &mut Library, serato_dir: &Path) -> Result<ImportSummary> {
    let db_path = serato_dir.join("database V2");
    let data = std::fs::read(&db_path).map_err(|e| LibraryError::io(&db_path, e))?;

    // The whole library is keyed to the volume the `_Serato_` folder sits on;
    // every `pfil` is relative to this volume's root.
    let volume = discover_for_path(serato_dir)?;
    library.upsert_volume(&volume)?;

    let mut summary = ImportSummary::default();
    // relative_path → track id, for the Subcrates membership join.
    let mut resolved: HashMap<String, String> = HashMap::new();

    for entry in database::parse_database_v2(&data) {
        let Some(rel) = entry.file_path.clone() else {
            summary.skipped += 1;
            summary.errors.push(ImportError {
                path: PathBuf::new(),
                reason: "database entry has no pfil path".to_string(),
            });
            continue;
        };
        match import_entry(library, &volume, &rel, &entry) {
            Ok(outcome) => {
                if outcome.added {
                    summary.added += 1;
                } else {
                    summary.refreshed += 1;
                }
                resolved.insert(rel, outcome.track_id);
            }
            Err(reason) => {
                summary.skipped += 1;
                summary.errors.push(ImportError {
                    path: volume.mount_point.join(&rel),
                    reason,
                });
            }
        }
    }

    import_subcrates(library, serato_dir, &resolved)?;
    Ok(summary)
}

struct EntryOutcome {
    track_id: String,
    added: bool,
}

fn import_entry(
    library: &Library,
    volume: &DiscoveredVolume,
    relative_path: &str,
    entry: &database::SeratoEntry,
) -> std::result::Result<EntryOutcome, String> {
    let abs_path = volume.mount_point.join(relative_path);
    if !abs_path.exists() {
        return Err(format!("file not found at {abs_path:?}"));
    }

    let (file_size, mtime) = match std::fs::metadata(&abs_path) {
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
        .find_track_file_owner(&volume.volume_uuid, relative_path)
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

    library
        .upsert_track_file(
            &track_id,
            &volume.volume_uuid,
            relative_path,
            detect_codec_from_extension(&abs_path),
            None,
            None,
            None,
            file_size,
            mtime,
        )
        .map_err(|e| e.to_string())?;

    write_metadata(library, &track_id, entry, &abs_path).map_err(|e| e.to_string())?;
    Ok(EntryOutcome { track_id, added })
}

fn write_metadata(
    library: &Library,
    track_id: &str,
    entry: &database::SeratoEntry,
    abs_path: &Path,
) -> Result<()> {
    // GEOB tags carry the gain (and a redundant BPM); read them once.
    let geobs = geob::read_serato_geobs(abs_path);
    let tags = geob::find(&geobs, geob::AUTOTAGS).map(autotags::parse);
    let gain_db = tags.and_then(|t| t.gain_db);

    library.upsert_metadata_source(
        track_id,
        SERATO,
        entry.artist.as_deref(),
        entry.title.as_deref(),
        entry.album.as_deref(),
        entry.genre.as_deref(),
        entry.comment.as_deref(),
        entry.composer.as_deref(),
        None, // year — not in database V2
        None, // track_number
        entry.bpm,
        entry.key.as_deref(),
        gain_db,
        None, // rating
        None, // version_token
    )?;

    // Beat grid (Serato BeatGrid GEOB).
    if let Some(grid) = geob::find(&geobs, geob::BEATGRID).and_then(beatgrid::parse) {
        library.upsert_imported_beatgrid(track_id, SERATO, grid.anchor_secs, grid.bpm, 0)?;
    }

    // Key → Camelot (Serato stores musical notation, e.g. "Em"); preserve
    // the original.
    if let Some(raw) = entry.key.as_deref() {
        if let Some(camelot) = musical_key_to_camelot(raw) {
            library.upsert_imported_key(track_id, SERATO, &camelot, Some(raw))?;
        } else {
            library.upsert_imported_key(track_id, SERATO, raw, Some(raw))?;
        }
    }

    // Cues + loops (Serato Markers2 GEOB) — truncate-and-rewrite.
    library.clear_imported_cues(track_id, SERATO)?;
    library.clear_imported_loops(track_id, SERATO)?;
    if let Some(markers) = geob::find(&geobs, geob::MARKERS2).map(markers2::parse) {
        for cue in &markers.cues {
            library.upsert_imported_cue(
                track_id,
                SERATO,
                cue.index,
                f64::from(cue.position_ms) / 1000.0,
                cue.name.as_deref(),
                None,
                "hot_cue",
            )?;
        }
        for lp in &markers.loops {
            library.upsert_imported_loop(
                track_id,
                SERATO,
                lp.index,
                f64::from(lp.start_ms) / 1000.0,
                f64::from(lp.end_ms) / 1000.0,
                lp.name.as_deref(),
                None,
            )?;
        }
    }
    Ok(())
}

/// Rebuild the Serato crate mirror from `Subcrates/*.crate`. Folder nesting
/// comes from the `%%` filename convention; intermediate folders are created
/// on demand and de-duplicated by their full name path.
fn import_subcrates(
    library: &mut Library,
    serato_dir: &Path,
    resolved: &HashMap<String, String>,
) -> Result<()> {
    library.clear_imported_crates(SERATO)?;

    let subcrates_dir = serato_dir.join("Subcrates");
    let Ok(read) = std::fs::read_dir(&subcrates_dir) else {
        return Ok(()); // no Subcrates folder → nothing to mirror
    };
    // Deterministic order so re-imports replay identically.
    let mut crate_files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("crate"))
        .collect();
    crate_files.sort();

    // Full name-path ("Hip Hop" / "Hip Hop"→"90s") → imported_crates id.
    let mut crate_ids: HashMap<Vec<String>, i64> = HashMap::new();

    for file in &crate_files {
        let Some(stem) = file.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let components = database::crate_name_components(stem);
        if components.is_empty() {
            continue;
        }
        let leaf_id = ensure_crate_chain(library, &components, &mut crate_ids)?;

        let data = std::fs::read(file).map_err(|e| LibraryError::io(file, e))?;
        for (ordinal, rel) in database::parse_crate(&data).iter().enumerate() {
            if let Some(track_id) = resolved.get(rel) {
                library.add_track_to_imported_crate(leaf_id, track_id, ordinal as i64)?;
            }
        }
    }
    Ok(())
}

/// Ensure every prefix of `components` exists as an imported crate (creating
/// missing folders), returning the leaf crate's id.
fn ensure_crate_chain(
    library: &mut Library,
    components: &[String],
    crate_ids: &mut HashMap<Vec<String>, i64>,
) -> Result<i64> {
    let mut parent: Option<i64> = None;
    let mut path: Vec<String> = Vec::new();
    for name in components {
        path.push(name.clone());
        let id = if let Some(&existing) = crate_ids.get(&path) {
            existing
        } else {
            let id = library.create_imported_crate(SERATO, name, parent)?;
            crate_ids.insert(path.clone(), id);
            id
        };
        parent = Some(id);
    }
    Ok(parent.expect("components is non-empty"))
}

/// Convert a musical key notation (Serato's `Em`, `Ab`, `F#m`, `Ebm`, …) to
/// Camelot. `None` for anything unrecognised. Enharmonic spellings collapse
/// (`Ebm` == `D#m`).
fn musical_key_to_camelot(raw: &str) -> Option<String> {
    let s = raw.trim();
    // Minor keys end in a lowercase 'm' (e.g. "Em"); strip it for the root.
    let (root, minor) = if let Some(stripped) = s.strip_suffix('m') {
        (stripped, true)
    } else {
        (s, false)
    };
    // Pitch class 0..11 from the root spelling.
    let pc = pitch_class(root)?;
    // Camelot number per pitch class, separately for major (B) / minor (A).
    // Major: C=8B,G=9B,D=10B,A=11B,E=12B,B=1B,F#=2B,C#=3B,G#=4B,D#=5B,A#=6B,F=7B.
    const MAJOR: [u8; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1]; // index = pitch class C..B
                                                                     // Minor: Am=8A,Em=9A,Bm=10A,F#m=11A,C#m=12A,G#m=1A,D#m=2A,A#m=3A,Fm=4A,Cm=5A,Gm=6A,Dm=7A.
    const MINOR: [u8; 12] = [5, 12, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10]; // index = pitch class C..B
    let number = if minor {
        MINOR[pc as usize]
    } else {
        MAJOR[pc as usize]
    };
    let side = if minor { 'A' } else { 'B' };
    Some(format!("{number}{side}"))
}

/// Map a note-name root to a pitch class 0..11 (C=0 … B=11), handling sharps
/// and flats.
fn pitch_class(root: &str) -> Option<u8> {
    let mut chars = root.chars();
    let letter = chars.next()?;
    let base: i32 = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let accidental = match chars.next() {
        Some('#') | Some('s') => 1,
        Some('b') | Some('♭') => -1,
        None => 0,
        _ => return None,
    };
    Some(((base + accidental).rem_euclid(12)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_conversion_minor_and_major() {
        assert_eq!(musical_key_to_camelot("Em").as_deref(), Some("9A"));
        assert_eq!(musical_key_to_camelot("Bm").as_deref(), Some("10A"));
        assert_eq!(musical_key_to_camelot("Ebm").as_deref(), Some("2A")); // D#m
        assert_eq!(musical_key_to_camelot("Am").as_deref(), Some("8A"));
        assert_eq!(musical_key_to_camelot("C").as_deref(), Some("8B"));
        assert_eq!(musical_key_to_camelot("F#").as_deref(), Some("2B"));
        assert_eq!(musical_key_to_camelot("Db").as_deref(), Some("3B")); // C#
        assert_eq!(musical_key_to_camelot("xyz"), None);
    }

    /// Opt-in end-to-end import against a real `_Serato_` folder, into a
    /// throwaway temp library (never the developer's real DB).
    /// `DUB_SERATO_DIR=~/Music/_Serato_ cargo test … serato_import_real --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "set DUB_SERATO_DIR to a real _Serato_ folder"]
    fn serato_import_real() {
        let Ok(dir) = std::env::var("DUB_SERATO_DIR") else {
            eprintln!("DUB_SERATO_DIR unset — skipping");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = Library::open_at(&tmp.path().join("library.sqlite")).unwrap();
        let summary = import_serato(&mut lib, Path::new(&dir)).expect("serato import");
        eprintln!(
            "added={} refreshed={} skipped={} errors={}",
            summary.added,
            summary.refreshed,
            summary.skipped,
            summary.errors.len()
        );
        for e in summary.errors.iter().take(10) {
            eprintln!("  skip: {} — {}", e.path.display(), e.reason);
        }
        let count =
            |sql: &str| -> i64 { lib.connection().query_row(sql, [], |r| r.get(0)).unwrap() };
        eprintln!(
            "tracks={} grids={} keys={} cues={} loops={} crates={} members={}",
            count("SELECT COUNT(*) FROM tracks"),
            count("SELECT COUNT(*) FROM track_beatgrids WHERE source='serato'"),
            count("SELECT COUNT(*) FROM track_keys WHERE source='serato'"),
            count("SELECT COUNT(*) FROM track_cues WHERE source='serato'"),
            count("SELECT COUNT(*) FROM track_loops WHERE source='serato'"),
            count("SELECT COUNT(*) FROM imported_crates WHERE source='serato'"),
            count("SELECT COUNT(*) FROM imported_crate_tracks"),
        );
        // Dump the first track's grid + cues so GEOB offsets can be eyeballed.
        let rows: Vec<(String, f64, f64)> = {
            let mut stmt = lib
                .connection()
                .prepare(
                    "SELECT track_id, anchor_secs, bpm FROM track_beatgrids \
                     WHERE source='serato' LIMIT 3",
                )
                .unwrap();
            let r = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap();
            r.filter_map(std::result::Result::ok).collect()
        };
        for (id, anchor, bpm) in rows {
            eprintln!("  grid {id}: anchor={anchor:.3}s bpm={bpm:.2}");
        }

        let again = import_serato(&mut lib, Path::new(&dir)).expect("re-import");
        assert_eq!(again.added, 0, "re-import minted new tracks: {again:?}");
    }
}
