//! Traktor `collection.nml` import adapter (M12b).
//!
//! Bridges the pure [`crate::traktor`] parser to the SQLite schema. For each
//! collection entry it:
//!
//! 1. Resolves the reconstructed `<LOCATION>` path to a canonical track,
//!    idempotent by `(volume_uuid, relative_path)` — the *same* key the
//!    folder importer uses, so a track imported by folder-walk and then
//!    described by Traktor enriches one identity (the user loads the deck and
//!    gets Traktor's grid/key/cues, not a duplicate row).
//! 2. Writes a `source = 'traktor'` metadata row plus the imported beatgrid,
//!    key, cues, and loops (the §8.1 source-priority writers from M12b).
//! 3. Mirrors the `<PLAYLISTS>` folder/playlist tree into the read-only
//!    `imported_crates` tables (truncate-and-rewrite per source).
//!
//! Like the folder importer this is **lazy** (PRD §8.4): no decode, no
//! fingerprint at import time. A track first seen via NML lands with
//! `fingerprint_id = NULL`; the fingerprint + duration fill in on first
//! deck-load via `analyze_track`. Read-only on the NML and the audio files —
//! we only `stat` for size/mtime, and only for files that exist on this
//! machine (a dangling NML reference is reported in
//! [`ImportSummary::skipped`], never inserted).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use uuid::Uuid;

use crate::db::Library;
use crate::error::{LibraryError, Result};
use crate::importer::{detect_codec_from_extension, ImportError, ImportSummary};
use crate::traktor::{self, ParsedEntry, ParsedPlaylist};
use crate::volumes::discover_for_path;

/// The schema `source` tag for everything this adapter writes.
const TRAKTOR: &str = "traktor";

/// Convert optional source-seconds to a `duration_ms`, rejecting
/// non-finite / out-of-range values.
fn secs_to_ms(secs: Option<f64>) -> Option<u32> {
    let s = secs?;
    let ms = (s * 1000.0).round();
    (ms.is_finite() && (0.0..f64::from(u32::MAX)).contains(&ms)).then_some(ms as u32)
}

/// Import a Traktor `collection.nml` at `nml_path`.
///
/// Idempotent: a second call against the same file refreshes metadata /
/// grids / cues / loops in place and rebuilds the playlist mirror without
/// duplicating track identities.
///
/// # Errors
/// Returns [`LibraryError`] only on a hard failure (NML unreadable / not
/// valid XML, or a SQL error). Per-entry failures (a missing file, a path on
/// an untrackable volume) are accumulated in [`ImportSummary::errors`] and do
/// not abort the run.
pub fn import_traktor(library: &mut Library, nml_path: &Path) -> Result<ImportSummary> {
    let data = std::fs::read(nml_path).map_err(|e| LibraryError::io(nml_path, e))?;
    let collection = traktor::parse_nml(&data).map_err(|e| {
        LibraryError::io(
            nml_path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        )
    })?;

    let mut summary = ImportSummary::default();
    // Reconstructed path → canonical track id, for the playlist join. Every
    // successfully-resolved entry lands here keyed by the same path
    // `<PRIMARYKEY>` reconstructs to (validated identical in `traktor`).
    let mut resolved: HashMap<PathBuf, String> = HashMap::new();

    for entry in &collection.entries {
        let Some(path) = entry.path.clone() else {
            summary.skipped += 1;
            summary.errors.push(ImportError {
                path: PathBuf::new(),
                reason: "entry has no <LOCATION> path".to_string(),
            });
            continue;
        };
        match import_entry(library, &path, entry) {
            Ok(outcome) => {
                if outcome.added {
                    summary.added += 1;
                } else {
                    summary.refreshed += 1;
                }
                resolved.insert(path, outcome.track_id);
            }
            Err(reason) => {
                summary.skipped += 1;
                summary.errors.push(ImportError { path, reason });
            }
        }
    }

    import_playlists(library, &collection.playlists, &resolved)?;
    Ok(summary)
}

/// What [`import_entry`] resolved a single `<ENTRY>` to.
struct EntryOutcome {
    track_id: String,
    /// `true` if a new `tracks` identity was minted, `false` if an existing
    /// `(volume, path)` was reused (refresh).
    added: bool,
}

/// Resolve + persist one collection entry. Mirrors the folder importer's
/// `(volume_uuid, relative_path)` idempotency so the two importers converge
/// on one track identity per file.
fn import_entry(
    library: &Library,
    path: &Path,
    entry: &ParsedEntry,
) -> std::result::Result<EntryOutcome, String> {
    // `discover_for_path` queries the file via `getattrlist`, so a path that
    // doesn't exist on this machine fails here — exactly the dangling-NML
    // case we want to report as skipped rather than insert a phantom track.
    let volume = discover_for_path(path).map_err(|e| format!("volume UUID unavailable: {e}"))?;
    library.upsert_volume(&volume).map_err(|e| e.to_string())?;
    let relative_path = volume
        .relative_to(path)
        .ok_or_else(|| format!("path {path:?} is not under volume {:?}", volume.mount_point))?;

    let (file_size, mtime) = match std::fs::metadata(path) {
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

    // Provisional duration from the NML's `PLAYTIME` so the browser shows a
    // length before the track is decoded (works for both freshly-inserted and
    // re-scanned tracks; only fills a NULL, so analysis later wins). See
    // `set_duration_if_absent`.
    if let Some(ms) = secs_to_ms(entry.duration_secs) {
        library
            .set_duration_if_absent(&track_id, ms)
            .map_err(|e| e.to_string())?;
    }

    library
        .upsert_track_file(
            &track_id,
            &volume.volume_uuid,
            &relative_path,
            detect_codec_from_extension(path),
            None,
            None,
            None,
            file_size,
            mtime,
        )
        .map_err(|e| e.to_string())?;

    write_traktor_metadata(library, &track_id, entry).map_err(|e| e.to_string())?;
    Ok(EntryOutcome { track_id, added })
}

/// Write the `traktor` metadata row + imported grid / key / cues / loops for
/// one resolved track.
fn write_traktor_metadata(library: &Library, track_id: &str, entry: &ParsedEntry) -> Result<()> {
    library.upsert_metadata_source(
        track_id,
        TRAKTOR,
        entry.artist.as_deref(),
        entry.title.as_deref(),
        entry.album.as_deref(),
        entry.genre.as_deref(),
        entry.comment.as_deref(),
        None, // composer — Traktor keeps it in INFO/REMIXER; not modelled yet
        None, // year
        None, // track_number
        entry.bpm,
        entry.key_camelot.as_deref(),
        None, // gain — Traktor's per-track gain is dB-relative; deferred
        None, // rating
        None, // version_token — derived from filename/id3, not Traktor
    )?;

    // AutoGrid downbeat → an imported beatgrid. `bar_phase = 0`: a Traktor
    // grid anchor *is* a downbeat (beat 1).
    if let (Some(anchor), Some(bpm)) = (entry.grid_anchor_secs, entry.bpm) {
        library.upsert_imported_beatgrid(track_id, TRAKTOR, anchor, bpm, 0)?;
    }

    if let Some(key) = entry.key_camelot.as_deref() {
        library.upsert_imported_key(track_id, TRAKTOR, key, entry.key_original.as_deref())?;
    }

    write_cues(library, track_id, entry)?;
    write_loops(library, track_id, entry)?;
    Ok(())
}

/// Rewrite the track's Traktor cues (truncate-and-rewrite). A cue carrying a
/// Traktor hotcue slot keeps that slot as `cue_index` and is a `hot_cue`
/// (performance pad); an unslotted cue is a timeline `memory` marker assigned
/// an index above the pad range so the two never collide.
fn write_cues(library: &Library, track_id: &str, entry: &ParsedEntry) -> Result<()> {
    library.clear_imported_cues(track_id, TRAKTOR)?;
    let mut next_memory = memory_base(entry.cues.iter().filter_map(|c| c.hotcue));
    for cue in &entry.cues {
        let (cue_index, kind) = match cue.hotcue {
            Some(slot) => (slot, "hot_cue"),
            None => {
                let idx = next_memory;
                next_memory = next_memory.saturating_add(1);
                (idx, "memory")
            }
        };
        library.upsert_imported_cue(
            track_id,
            TRAKTOR,
            cue_index,
            cue.position_secs,
            cue.name.as_deref(),
            None,
            kind,
        )?;
    }
    Ok(())
}

/// Rewrite the track's Traktor loops (truncate-and-rewrite). Same slotted /
/// unslotted index rule as [`write_cues`].
fn write_loops(library: &Library, track_id: &str, entry: &ParsedEntry) -> Result<()> {
    library.clear_imported_loops(track_id, TRAKTOR)?;
    let mut next_idx = memory_base(entry.loops.iter().filter_map(|l| l.hotcue));
    for lp in &entry.loops {
        let loop_index = match lp.hotcue {
            Some(slot) => slot,
            None => {
                let idx = next_idx;
                next_idx = next_idx.saturating_add(1);
                idx
            }
        };
        library.upsert_imported_loop(
            track_id,
            TRAKTOR,
            loop_index,
            lp.start_secs,
            lp.end_secs,
            lp.name.as_deref(),
            None,
        )?;
    }
    Ok(())
}

/// First index to use for unslotted (memory) cues/loops: just past the
/// highest hotcue slot, but never inside the 0–7 pad range, so an unslotted
/// marker can never alias a real hotcue pad.
fn memory_base(slots: impl Iterator<Item = u8>) -> u8 {
    slots.max().map_or(0, |m| m.saturating_add(1)).max(8)
}

/// Rebuild the `imported_crates` mirror for Traktor: clear the source, then
/// re-create every folder/playlist node in document order (a parent always
/// precedes its children, so its row id is available when a child needs it).
fn import_playlists(
    library: &mut Library,
    playlists: &[ParsedPlaylist],
    resolved: &HashMap<PathBuf, String>,
) -> Result<()> {
    library.clear_imported_crates(TRAKTOR)?;
    let mut crate_ids: Vec<i64> = Vec::with_capacity(playlists.len());
    for pl in playlists {
        let parent = pl.parent.and_then(|i| crate_ids.get(i).copied());
        let crate_id = library.create_imported_crate(TRAKTOR, &pl.name, parent)?;
        for (ordinal, path) in pl.track_paths.iter().enumerate() {
            if let Some(track_id) = resolved.get(path) {
                library.add_track_to_imported_crate(crate_id, track_id, ordinal as i64)?;
            }
        }
        crate_ids.push(crate_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use tempfile::tempdir;

    /// Synthetic mono i16 WAV so the adapter's `discover_for_path` /
    /// `stat` calls hit a real file (same trick as the folder-importer
    /// tests — no binary fixtures in the repo).
    fn write_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..4_410 {
            let t = i as f32 / 44_100.0;
            let s = 0.4 * (2.0 * std::f32::consts::PI * 220.0 * t).sin();
            writer
                .write_sample((s * f32::from(i16::MAX)) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Encode an absolute file path back into a Traktor `<LOCATION>`'s
    /// `(DIR, FILE)` pair — `/` → `/:`, trailing `/:` on the dir. Mirror of
    /// the parser's `reconstruct_path`, so the round-trip is exact.
    fn location_attrs(file: &Path) -> (String, String) {
        let parent = file.parent().unwrap().to_string_lossy();
        let dir = format!("{}/:", parent.replace('/', "/:"));
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        (dir, name)
    }

    /// Encode an absolute file path as a `<PRIMARYKEY>` membership key on the
    /// boot volume (`VOLUME/:dir/:file`). Reconstructs to the same path as
    /// `location_attrs`, which is what lets playlists join to tracks.
    fn primary_key(file: &Path) -> String {
        format!("Macintosh HD{}", file.to_string_lossy().replace('/', "/:"))
    }

    /// Build a one-playlist NML referencing `files`, each entry carrying a
    /// grid anchor, a key, a slotted hotcue + an unslotted memory cue, and a
    /// slotted loop.
    fn build_nml(files: &[PathBuf]) -> String {
        let mut entries = String::new();
        for f in files {
            let (dir, name) = location_attrs(f);
            entries.push_str(&format!(
                r#"<ENTRY TITLE="{name}" ARTIST="Artist">
                     <LOCATION DIR="{dir}" FILE="{name}" VOLUME="Macintosh HD"/>
                     <INFO GENRE="DnB" COMMENT="hello"/>
                     <TEMPO BPM="174.500000"/>
                     <MUSICAL_KEY VALUE="21"/>
                     <CUE_V2 NAME="AutoGrid" TYPE="4" START="0.0" LEN="0" HOTCUE="-1"/>
                     <CUE_V2 NAME="Drop" TYPE="0" START="16000.0" LEN="0" HOTCUE="1"/>
                     <CUE_V2 NAME="Intro" TYPE="0" START="500.0" LEN="0" HOTCUE="-1"/>
                     <CUE_V2 NAME="Roll" TYPE="5" START="32000.0" LEN="4000.0" HOTCUE="0"/>
                   </ENTRY>"#,
            ));
        }
        let mut members = String::new();
        for f in files {
            members.push_str(&format!(
                r#"<ENTRY><PRIMARYKEY TYPE="TRACK" KEY="{}"/></ENTRY>"#,
                primary_key(f)
            ));
        }
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><NML VERSION="19">
                <COLLECTION ENTRIES="{n}">{entries}</COLLECTION>
                <PLAYLISTS><NODE TYPE="FOLDER" NAME="$ROOT"><SUBNODES COUNT="1">
                  <NODE TYPE="PLAYLIST" NAME="My Set">
                    <PLAYLIST ENTRIES="{n}" TYPE="LIST">{members}</PLAYLIST>
                  </NODE>
                </SUBNODES></NODE></PLAYLISTS>
               </NML>"#,
            n = files.len(),
        )
    }

    fn open_lib(dir: &Path) -> Library {
        Library::open_at(&dir.join("library.sqlite")).unwrap()
    }

    fn count(lib: &Library, sql: &str) -> i64 {
        lib.connection().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn imports_entries_grids_cues_loops_and_playlist() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let a = music.join("a.wav");
        let b = music.join("b.wav");
        write_wav(&a);
        write_wav(&b);
        let nml = tmp.path().join("collection.nml");
        std::fs::write(&nml, build_nml(&[a.clone(), b.clone()])).unwrap();

        let mut lib = open_lib(tmp.path());
        let summary = import_traktor(&mut lib, &nml).expect("import");
        assert_eq!(summary.added, 2, "summary: {summary:?}");
        assert_eq!(summary.refreshed, 0);
        assert_eq!(summary.skipped, 0);
        assert!(summary.errors.is_empty(), "errors: {:?}", summary.errors);

        assert_eq!(count(&lib, "SELECT COUNT(*) FROM tracks"), 2);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM track_files"), 2);
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_metadata_source WHERE source='traktor'"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_beatgrids WHERE source='traktor' AND is_active=1"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_keys WHERE source='traktor'"
            ),
            2
        );
        // Per entry: Drop (hot_cue idx1) + Intro (memory idx8). The AutoGrid
        // and the loop are NOT cues.
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='traktor'"
            ),
            4
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='traktor' AND kind='hot_cue'"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='traktor' AND kind='memory'"
            ),
            2
        );
        // One loop per entry (the TYPE 5 region).
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_loops WHERE source='traktor'"
            ),
            2
        );

        // Key VALUE 21 → 8A (A minor); original retained.
        let (key, orig): (String, Option<String>) = lib
            .connection()
            .query_row(
                "SELECT key_notation, original_notation FROM track_keys \
                 WHERE source='traktor' LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(key, "8A");
        assert_eq!(orig.as_deref(), Some("21"));

        // Playlist mirror: "$ROOT" is transparent → one crate ("My Set")
        // with both tracks as members.
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM imported_crates WHERE source='traktor'"
            ),
            1
        );
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM imported_crate_tracks"), 2);
    }

    #[test]
    fn reimport_is_idempotent() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let a = music.join("a.wav");
        write_wav(&a);
        let nml = tmp.path().join("collection.nml");
        std::fs::write(&nml, build_nml(std::slice::from_ref(&a))).unwrap();

        let mut lib = open_lib(tmp.path());
        let first = import_traktor(&mut lib, &nml).expect("first");
        assert_eq!(first.added, 1);

        let second = import_traktor(&mut lib, &nml).expect("second");
        assert_eq!(second.added, 0, "no new identity on re-import");
        assert_eq!(second.refreshed, 1);

        // Nothing doubled: one track, one cue set, one loop, one playlist.
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM tracks"), 1);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM track_files"), 1);
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='traktor'"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_loops WHERE source='traktor'"
            ),
            1
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM imported_crates WHERE source='traktor'"
            ),
            1
        );
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM imported_crate_tracks"), 1);
    }

    #[test]
    fn dangling_reference_is_skipped_not_inserted() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let missing = music.join("not-here.wav"); // never written
        let nml = tmp.path().join("collection.nml");
        std::fs::write(&nml, build_nml(&[missing])).unwrap();

        let mut lib = open_lib(tmp.path());
        let summary = import_traktor(&mut lib, &nml).expect("import");
        assert_eq!(summary.added, 0);
        assert_eq!(summary.skipped, 1, "summary: {summary:?}");
        assert_eq!(summary.errors.len(), 1);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM tracks"), 0);
        // The playlist node is still mirrored even with no resolvable members.
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM imported_crates WHERE source='traktor'"
            ),
            1
        );
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM imported_crate_tracks"), 0);
    }

    #[test]
    fn rejects_unreadable_nml() {
        let tmp = tempdir().unwrap();
        let mut lib = open_lib(tmp.path());
        let bogus = tmp.path().join("nope.nml");
        assert!(import_traktor(&mut lib, &bogus).is_err());
    }

    /// Opt-in end-to-end validation against a real export. Set
    /// `DUB_TRAKTOR_NML` to a real `collection.nml` whose files exist on
    /// this machine and run with `--ignored --nocapture`. Imports into a
    /// throwaway temp library (never the developer's real DB) and reports
    /// what landed. Skips cleanly when the env var is unset (CI-safe).
    #[test]
    #[ignore = "set DUB_TRAKTOR_NML to a real collection.nml"]
    fn validate_real_import() {
        let Ok(path) = std::env::var("DUB_TRAKTOR_NML") else {
            eprintln!("DUB_TRAKTOR_NML unset — skipping real-import validation");
            return;
        };
        let tmp = tempdir().unwrap();
        let mut lib = open_lib(tmp.path());
        let summary = import_traktor(&mut lib, Path::new(&path)).expect("import real nml");
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
        eprintln!(
            "tracks={} traktor-grids={} traktor-keys={} traktor-cues={} traktor-loops={} crates={} members={}",
            count(&lib, "SELECT COUNT(*) FROM tracks"),
            count(&lib, "SELECT COUNT(*) FROM track_beatgrids WHERE source='traktor'"),
            count(&lib, "SELECT COUNT(*) FROM track_keys WHERE source='traktor'"),
            count(&lib, "SELECT COUNT(*) FROM track_cues WHERE source='traktor'"),
            count(&lib, "SELECT COUNT(*) FROM track_loops WHERE source='traktor'"),
            count(&lib, "SELECT COUNT(*) FROM imported_crates WHERE source='traktor'"),
            count(&lib, "SELECT COUNT(*) FROM imported_crate_tracks"),
        );
        // A re-import must not mint new identities.
        let again = import_traktor(&mut lib, Path::new(&path)).expect("re-import");
        assert_eq!(again.added, 0, "re-import added new tracks: {again:?}");
    }
}
