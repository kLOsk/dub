//! rekordbox `rekordbox.xml` import adapter (M12d).
//!
//! Bridges the pure [`crate::rekordbox`] parser to the SQLite schema. For each
//! collection track it:
//!
//! 1. Resolves the decoded `Location` path to a canonical track, idempotent by
//!    `(volume_uuid, relative_path)` — the *same* key the folder / Serato /
//!    Traktor / iTunes importers use, so a track described by several apps
//!    enriches one identity (no duplicate rows).
//! 2. Writes a `source = 'rekordbox'` metadata row plus the imported beatgrid,
//!    key, cues, and loops (the §8.1 source-priority writers).
//! 3. Mirrors the `<PLAYLISTS>` folder/playlist tree into the read-only
//!    `imported_crates` tables (truncate-and-rewrite per source), joining
//!    members by `TrackID` (rekordbox's `KeyType="0"`).
//!
//! Lazy (PRD §8.4) and read-only on the XML + audio files, exactly like the
//! Traktor adapter: no decode / fingerprint at import time, only a `stat` for
//! size/mtime, and only for files present on this machine (a dangling
//! reference is reported in [`ImportSummary::skipped`], never inserted).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use uuid::Uuid;

use crate::db::Library;
use crate::error::{LibraryError, Result};
use crate::importer::{detect_codec_from_extension, ImportError, ImportSummary};
use crate::rekordbox::{self, ParsedPlaylist, ParsedTrack};
use crate::volumes::discover_for_path;

/// The schema `source` tag for everything this adapter writes.
const REKORDBOX: &str = "rekordbox";

/// Convert optional source-seconds to a `duration_ms`, rejecting non-finite /
/// out-of-range values.
fn secs_to_ms(secs: Option<f64>) -> Option<u32> {
    let s = secs?;
    let ms = (s * 1000.0).round();
    (ms.is_finite() && (0.0..f64::from(u32::MAX)).contains(&ms)).then_some(ms as u32)
}

/// Import a rekordbox `rekordbox.xml` at `xml_path`.
///
/// Idempotent: a second call refreshes metadata / grids / cues / loops in place
/// and rebuilds the playlist mirror without duplicating track identities.
///
/// # Errors
/// Returns [`LibraryError`] only on a hard failure (XML unreadable / invalid, or
/// a SQL error). Per-track failures (missing file, untrackable volume) are
/// accumulated in [`ImportSummary::errors`] and do not abort the run.
pub fn import_rekordbox(library: &mut Library, xml_path: &Path) -> Result<ImportSummary> {
    let data = std::fs::read(xml_path).map_err(|e| LibraryError::io(xml_path, e))?;
    let lib = rekordbox::parse_xml(&data).map_err(|e| {
        LibraryError::io(
            xml_path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        )
    })?;

    let mut summary = ImportSummary::default();
    // rekordbox TrackID → canonical uuid, for the playlist join.
    let mut id_map: HashMap<i64, String> = HashMap::new();

    for track in &lib.tracks {
        let Some(path) = track.path.clone() else {
            summary.skipped += 1;
            summary.errors.push(ImportError {
                path: PathBuf::new(),
                reason: "track has no Location path".to_string(),
            });
            continue;
        };
        match import_track(library, &path, track) {
            Ok(outcome) => {
                if outcome.added {
                    summary.added += 1;
                } else {
                    summary.refreshed += 1;
                }
                id_map.insert(track.track_id, outcome.track_id);
            }
            Err(reason) => {
                summary.skipped += 1;
                summary.errors.push(ImportError { path, reason });
            }
        }
    }

    import_playlists(library, &lib.playlists, &id_map)?;
    Ok(summary)
}

/// What [`import_track`] resolved a single `<TRACK>` to.
struct TrackOutcome {
    track_id: String,
    /// `true` if a new `tracks` identity was minted, `false` on refresh.
    added: bool,
}

/// Resolve + persist one collection track. Mirrors the other importers'
/// `(volume_uuid, relative_path)` idempotency so they converge on one identity.
fn import_track(
    library: &Library,
    path: &Path,
    track: &ParsedTrack,
) -> std::result::Result<TrackOutcome, String> {
    // `discover_for_path` queries the file via `getattrlist`, so a path absent
    // on this machine fails here — the dangling-reference case we report as
    // skipped rather than inserting a phantom track.
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

    // Provisional duration from `TotalTime` so the browser shows a length
    // before decode (fresh + re-scanned tracks; only fills a NULL, analysis
    // later wins). See `set_duration_if_absent`.
    if let Some(ms) = secs_to_ms(track.duration_secs) {
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

    write_rekordbox_metadata(library, &track_id, track).map_err(|e| e.to_string())?;
    Ok(TrackOutcome { track_id, added })
}

/// Write the `rekordbox` metadata row + imported grid / key / cues / loops.
fn write_rekordbox_metadata(library: &Library, track_id: &str, track: &ParsedTrack) -> Result<()> {
    library.upsert_metadata_source(
        track_id,
        REKORDBOX,
        track.artist.as_deref(),
        track.title.as_deref(),
        track.album.as_deref(),
        track.genre.as_deref(),
        track.comment.as_deref(),
        track.composer.as_deref(),
        track.year,
        track.track_number,
        track.bpm,
        track.key.as_deref(),
        None, // gain — rekordbox's per-track gain isn't in the XML
        None, // rating — not modelled
        None, // version_token — derived from filename/id3, not rekordbox
    )?;

    // First `<TEMPO>` → an imported beatgrid. Prefer the grid tempo; fall back
    // to AverageBpm. `bar_phase` from the anchor's `Battito`.
    if let (Some(anchor), Some(bpm)) = (track.grid_anchor_secs, track.grid_bpm.or(track.bpm)) {
        library.upsert_imported_beatgrid(track_id, REKORDBOX, anchor, bpm, track.grid_bar_phase)?;
    }

    if let Some(key) = track.key.as_deref() {
        library.upsert_imported_key(track_id, REKORDBOX, key, Some(key))?;
    }

    write_cues(library, track_id, track)?;
    write_loops(library, track_id, track)?;
    Ok(())
}

/// Rewrite the track's rekordbox cues (truncate-and-rewrite). A cue with a hot
/// pad slot (`Num` 0–7) keeps that slot as `cue_index` and is a `hot_cue`; a
/// memory cue (`Num = -1`) is a timeline marker assigned an index above the pad
/// range so the two never collide.
fn write_cues(library: &Library, track_id: &str, track: &ParsedTrack) -> Result<()> {
    library.clear_imported_cues(track_id, REKORDBOX)?;
    let mut next_memory = memory_base(track.cues.iter().filter_map(|c| c.hotcue));
    for cue in &track.cues {
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
            REKORDBOX,
            cue_index,
            cue.position_secs,
            cue.name.as_deref(),
            cue.color.as_deref(),
            kind,
        )?;
    }
    Ok(())
}

/// Rewrite the track's rekordbox loops (truncate-and-rewrite). Same slotted /
/// unslotted index rule as [`write_cues`].
fn write_loops(library: &Library, track_id: &str, track: &ParsedTrack) -> Result<()> {
    library.clear_imported_loops(track_id, REKORDBOX)?;
    let mut next_idx = memory_base(track.loops.iter().filter_map(|l| l.hotcue));
    for lp in &track.loops {
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
            REKORDBOX,
            loop_index,
            lp.start_secs,
            lp.end_secs,
            lp.name.as_deref(),
            lp.color.as_deref(),
        )?;
    }
    Ok(())
}

/// First index for unslotted (memory) cues/loops: just past the highest hot
/// slot, never inside the 0–7 pad range, so a memory marker can't alias a pad.
fn memory_base(slots: impl Iterator<Item = u8>) -> u8 {
    slots.max().map_or(0, |m| m.saturating_add(1)).max(8)
}

/// Rebuild the `imported_crates` mirror for rekordbox: clear the source, then
/// re-create every folder/playlist node in document order (a parent always
/// precedes its children, so its row id is available when a child needs it).
/// Members join by `TrackID` via `id_map`.
fn import_playlists(
    library: &mut Library,
    playlists: &[ParsedPlaylist],
    id_map: &HashMap<i64, String>,
) -> Result<()> {
    library.clear_imported_crates(REKORDBOX)?;
    let mut crate_ids: Vec<i64> = Vec::with_capacity(playlists.len());
    for pl in playlists {
        let parent = pl.parent.and_then(|i| crate_ids.get(i).copied());
        let crate_id = library.create_imported_crate(REKORDBOX, &pl.name, parent)?;
        for (ordinal, track_id) in pl
            .track_ids
            .iter()
            .filter_map(|id| id_map.get(id))
            .enumerate()
        {
            library.add_track_to_imported_crate(crate_id, track_id, ordinal as i64)?;
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

    /// Synthetic mono i16 WAV so the adapter's `discover_for_path` / `stat`
    /// calls hit a real file (same trick as the folder-importer tests).
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

    /// Encode an absolute path as a rekordbox `Location` `file://localhost/…`
    /// URL (space → `%20`; enough for the temp paths these tests use).
    fn location(file: &Path) -> String {
        format!(
            "file://localhost{}",
            file.to_string_lossy().replace(' ', "%20")
        )
    }

    /// Build a one-playlist export referencing `files`, each track carrying a
    /// grid, a hot cue + a memory cue, and a hot loop. TrackID = index + 1.
    fn build_xml(files: &[PathBuf]) -> String {
        let mut tracks = String::new();
        for (i, f) in files.iter().enumerate() {
            tracks.push_str(&format!(
                r#"<TRACK TrackID="{id}" Name="T{id}" Artist="Artist" Genre="DnB"
                        TotalTime="174" AverageBpm="174.50"
                        Location="{loc}" Tonality="8A">
                     <TEMPO Inizio="0.100" Bpm="174.50" Metro="4/4" Battito="1"/>
                     <POSITION_MARK Name="Intro" Type="0" Start="0.5" Num="-1"/>
                     <POSITION_MARK Name="Drop" Type="0" Start="16.0" Num="1" Red="40" Green="226" Blue="20"/>
                     <POSITION_MARK Name="Roll" Type="4" Start="32.0" End="36.0" Num="0"/>
                   </TRACK>"#,
                id = i + 1,
                loc = location(f),
            ));
        }
        let members: String = (0..files.len())
            .map(|i| format!(r#"<TRACK Key="{}"/>"#, i + 1))
            .collect();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.14" Company="AlphaTheta"/>
  <COLLECTION Entries="{n}">{tracks}</COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Name="My Set" Type="1" KeyType="0" Entries="{n}">{members}</NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>"#,
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
    fn imports_tracks_grids_cues_loops_and_playlist() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let a = music.join("a.wav");
        let b = music.join("b.wav");
        write_wav(&a);
        write_wav(&b);
        let xml = tmp.path().join("rekordbox.xml");
        std::fs::write(&xml, build_xml(&[a.clone(), b.clone()])).unwrap();

        let mut lib = open_lib(tmp.path());
        let summary = import_rekordbox(&mut lib, &xml).expect("import");
        assert_eq!(summary.added, 2, "summary: {summary:?}");
        assert_eq!(summary.refreshed, 0);
        assert_eq!(summary.skipped, 0);
        assert!(summary.errors.is_empty(), "errors: {:?}", summary.errors);

        assert_eq!(count(&lib, "SELECT COUNT(*) FROM tracks"), 2);
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_metadata_source WHERE source='rekordbox'"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_beatgrids WHERE source='rekordbox' AND is_active=1"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_keys WHERE source='rekordbox'"
            ),
            2
        );
        // Per track: Intro (memory) + Drop (hot1). The loop is not a cue.
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='rekordbox' AND kind='hot_cue'"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='rekordbox' AND kind='memory'"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_loops WHERE source='rekordbox'"
            ),
            2
        );

        // Key 8A stored verbatim (Camelot, rekordbox's own notation).
        let key: String = lib
            .connection()
            .query_row(
                "SELECT key_notation FROM track_keys WHERE source='rekordbox' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(key, "8A");

        // Playlist mirror: ROOT transparent → one crate ("My Set"), both tracks.
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM imported_crates WHERE source='rekordbox'"
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
        let xml = tmp.path().join("rekordbox.xml");
        std::fs::write(&xml, build_xml(std::slice::from_ref(&a))).unwrap();

        let mut lib = open_lib(tmp.path());
        let first = import_rekordbox(&mut lib, &xml).expect("first");
        assert_eq!(first.added, 1);

        let second = import_rekordbox(&mut lib, &xml).expect("second");
        assert_eq!(second.added, 0, "no new identity on re-import");
        assert_eq!(second.refreshed, 1);

        assert_eq!(count(&lib, "SELECT COUNT(*) FROM tracks"), 1);
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='rekordbox'"
            ),
            2
        );
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM track_loops WHERE source='rekordbox'"
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
        let xml = tmp.path().join("rekordbox.xml");
        std::fs::write(&xml, build_xml(&[missing])).unwrap();

        let mut lib = open_lib(tmp.path());
        let summary = import_rekordbox(&mut lib, &xml).expect("import");
        assert_eq!(summary.added, 0);
        assert_eq!(summary.skipped, 1, "summary: {summary:?}");
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM tracks"), 0);
        // The playlist node is still mirrored even with no resolvable members.
        assert_eq!(
            count(
                &lib,
                "SELECT COUNT(*) FROM imported_crates WHERE source='rekordbox'"
            ),
            1
        );
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM imported_crate_tracks"), 0);
    }

    #[test]
    fn rejects_unreadable_xml() {
        let tmp = tempdir().unwrap();
        let mut lib = open_lib(tmp.path());
        let bogus = tmp.path().join("nope.xml");
        assert!(import_rekordbox(&mut lib, &bogus).is_err());
    }

    /// Opt-in end-to-end validation against a real export. `DUB_REKORDBOX_XML=…
    /// cargo test … validate_real_import -- --ignored --nocapture`. Imports
    /// into a throwaway temp library (never the real DB). Skips when unset.
    #[test]
    #[ignore = "set DUB_REKORDBOX_XML to a real rekordbox.xml"]
    fn validate_real_import() {
        let Ok(path) = std::env::var("DUB_REKORDBOX_XML") else {
            eprintln!("DUB_REKORDBOX_XML unset — skipping real-import validation");
            return;
        };
        let tmp = tempdir().unwrap();
        let mut lib = open_lib(tmp.path());
        let summary = import_rekordbox(&mut lib, Path::new(&path)).expect("import real xml");
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
            "tracks={} grids={} keys={} cues={} loops={} crates={} members={}",
            count(&lib, "SELECT COUNT(*) FROM tracks"),
            count(
                &lib,
                "SELECT COUNT(*) FROM track_beatgrids WHERE source='rekordbox'"
            ),
            count(
                &lib,
                "SELECT COUNT(*) FROM track_keys WHERE source='rekordbox'"
            ),
            count(
                &lib,
                "SELECT COUNT(*) FROM track_cues WHERE source='rekordbox'"
            ),
            count(
                &lib,
                "SELECT COUNT(*) FROM track_loops WHERE source='rekordbox'"
            ),
            count(
                &lib,
                "SELECT COUNT(*) FROM imported_crates WHERE source='rekordbox'"
            ),
            count(&lib, "SELECT COUNT(*) FROM imported_crate_tracks"),
        );
        let again = import_rekordbox(&mut lib, Path::new(&path)).expect("re-import");
        assert_eq!(again.added, 0, "re-import added new tracks: {again:?}");
    }
}
