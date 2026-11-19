//! Traktor's data inside the file: the `TRAKTOR4` ID3 `PRIV` frame.
//!
//! Traktor writes each track's grid, key and cues into the file it plays,
//! as well as into `collection.nml`. A DJ who managed the music in Apple
//! Music and played it in Traktor on another Mac brings the files but not
//! the collection (Daniel, 2026-11-18: 2,167 of 2,171 MP3s carry the
//! frame, the collection on this Mac has 16 entries). Reading the frame
//! gives those files Traktor's grid and cues in Dub anyway.
//!
//! The frame is a tree of chunks, each a four-character tag stored
//! byte-reversed (`DMRT` = `TRMD`), a `u32` payload length and a `u32`
//! child count, all little-endian; a chunk with children holds them as its
//! payload. `TRMD/DATA` carries the fields, mirroring the collection:
//!
//! * `HBPM` `f32`: the collection's `TEMPO BPM`.
//! * `MKEY` `u32`: the collection's `MUSICAL_KEY VALUE` (0–23).
//! * `SYNC/LOCK` `u32`: the grid is locked (the collection's
//!   `<ENTRY LOCK="1">`).
//! * `CUEP`: `u32` count, then per cue `u32` version (1), `u32` name
//!   length in UTF-16 units + the name, `u32` display order, `i32` type,
//!   `f64` start ms, `f64` length ms, `i32` repeats, `i32` hot cue. The
//!   same record as `<CUE_V2>`.
//!
//! Confirmed against a real collection: for the one file present in both,
//! BPM, key value and AutoGrid start were identical to the digit. Every
//! `CUEP` in the 2,168 real frames parsed to its exact length.

use std::path::Path;

use id3::Content;
use rusqlite::{params, OptionalExtension};

use crate::db::Library;
use crate::error::{LibraryError, Result};
use crate::serato::geob;
use crate::traktor::{self, ParsedEntry};
use crate::traktor_import::write_traktor_analysis;

const OWNER: &str = "TRAKTOR4";
/// No real track has more; a count past this is a corrupt frame.
const MAX_CUES: u32 = 4096;

/// Bring the DJ data a file carries in its own tags into the library —
/// Traktor's `TRAKTOR4` frame and Serato's GEOBs (grid, cues, loops), each
/// under its own source so the usual Serato > Traktor > auto order picks —
/// at most once per version of the file: skipped while
/// `track_files.tags_read_mtime` matches the file's `mtime`. A file with
/// neither is marked read too, so a library that never saw either app pays
/// for the read once. One tag read serves both.
pub(crate) fn sync_file_tags(
    library: &Library,
    track_id: &str,
    volume_uuid: &str,
    relative_path: &str,
    path: &Path,
) -> Result<()> {
    let state: Option<(Option<i64>, Option<i64>)> = library
        .connection()
        .query_row(
            "SELECT mtime, tags_read_mtime FROM track_files \
             WHERE volume_uuid = ?1 AND relative_path = ?2",
            params![volume_uuid, relative_path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| LibraryError::sqlite("file_tag_state", e))?;
    let Some((Some(mtime), read_at)) = state else {
        return Ok(());
    };
    if read_at == Some(mtime) {
        return Ok(());
    }
    if let Some(tag) = geob::read_tag(path) {
        // Serato first, in the grids' own rank: a lock follows a grid only
        // while it is the active one, so Traktor's must meet Serato's
        // already in place, not be pinned before Serato's takes over.
        let serato = geob::serato_geobs(&tag);
        let analysed = [geob::BEATGRID, geob::MARKERS2]
            .iter()
            .any(|d| geob::find(&serato, d).is_some());
        if analysed {
            crate::serato_import::write_serato_analysis(library, track_id, &serato, path)?;
        }
        if let Some(entry) = from_tag(&tag) {
            write_traktor_analysis(library, track_id, &entry, path)?;
        }
    }
    library
        .connection()
        .execute(
            "UPDATE track_files SET tags_read_mtime = ?3 \
             WHERE volume_uuid = ?1 AND relative_path = ?2",
            params![volume_uuid, relative_path, mtime],
        )
        .map_err(|e| LibraryError::sqlite("mark_file_tags_read", e))?;
    Ok(())
}

/// The file's Traktor data, if it has any worth keeping.
#[cfg(test)]
fn read_file(path: &Path) -> Option<ParsedEntry> {
    from_tag(&geob::read_tag(path)?)
}

fn from_tag(tag: &id3::Tag) -> Option<ParsedEntry> {
    let frame = tag.frames().find_map(|f| match f.content() {
        Content::Private(p) if p.owner_identifier == OWNER => Some(p.private_data.as_slice()),
        _ => None,
    })?;
    parse(frame)
}

/// Parse a `TRAKTOR4` frame body. `None` when it holds no grid, key or cue.
pub fn parse(frame: &[u8]) -> Option<ParsedEntry> {
    let (_, root, _) = chunks(frame).find(|(tag, _, n)| tag == b"TRMD" && *n > 0)?;
    let (_, data, _) = chunks(root).find(|(tag, _, n)| tag == b"DATA" && *n > 0)?;

    let mut entry = ParsedEntry::default();
    for (tag, body, children) in chunks(data) {
        if children > 0 {
            if &tag == b"SYNC" {
                entry.grid_locked = chunks(body)
                    .find(|(t, _, n)| t == b"LOCK" && *n == 0)
                    .and_then(|(_, b, _)| u32_at(b, 0))
                    .is_some_and(|v| v != 0);
            }
            continue;
        }
        match &tag {
            b"HBPM" => {
                entry.bpm = f32_at(body, 0)
                    .map(f64::from)
                    .filter(|b| b.is_finite() && *b > 0.0);
            }
            b"MKEY" => {
                entry.key_camelot = u32_at(body, 0)
                    .and_then(|v| u8::try_from(v).ok())
                    .and_then(traktor::musical_key_to_camelot)
                    .map(String::from);
            }
            b"CUEP" => read_cues(body, &mut entry),
            _ => {}
        }
    }
    let worth_keeping = entry.bpm.is_some()
        || entry.key_camelot.is_some()
        || !entry.cues.is_empty()
        || !entry.loops.is_empty();
    worth_keeping.then_some(entry)
}

/// A cue list that doesn't parse to the end is dropped whole: half a list
/// read from a misaligned offset would put cues in the wrong places.
fn read_cues(body: &[u8], entry: &mut ParsedEntry) {
    let mut parsed = ParsedEntry::default();
    if parse_cues(body, &mut parsed).is_some() {
        entry.grid_anchor_secs = parsed.grid_anchor_secs;
        entry.cues = parsed.cues;
        entry.loops = parsed.loops;
    }
}

fn parse_cues(body: &[u8], into: &mut ParsedEntry) -> Option<()> {
    let count = u32_at(body, 0)?;
    if count > MAX_CUES {
        return None;
    }
    let mut at = 4;
    for _ in 0..count {
        let version = u32_at(body, at)?;
        if version != 1 {
            return None;
        }
        let name_units = usize::try_from(u32_at(body, at + 4)?).ok()?;
        let name_start = at + 8;
        let name_end = name_start.checked_add(name_units.checked_mul(2)?)?;
        let name_bytes = body.get(name_start..name_end)?;
        let units: Vec<u16> = name_bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let name = String::from_utf16(&units).ok();
        let fields = name_end + 4; // display order, unused
        let cue_type = i32_at(body, fields)?;
        let start_ms = f64_at(body, fields + 4)?;
        let len_ms = f64_at(body, fields + 12)?;
        let hotcue = i32_at(body, fields + 24)?;
        traktor::add_cue(
            into,
            Some(i64::from(cue_type)),
            Some(i64::from(hotcue)),
            start_ms,
            Some(len_ms),
            name,
        );
        at = fields + 28;
    }
    (at == body.len()).then_some(())
}

/// The chunks directly inside `buf`: `(tag, payload, child count)`. Stops at
/// the first chunk that runs past the end.
fn chunks(buf: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8], u32)> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let head = buf.get(at..at.checked_add(12)?)?;
        let mut tag = [head[3], head[2], head[1], head[0]];
        if tag[0] == b' ' {
            tag.rotate_left(1); // " HDR" is HDR padded at the front
        }
        let len = usize::try_from(u32::from_le_bytes([head[4], head[5], head[6], head[7]])).ok()?;
        let children = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
        let start = at + 12;
        let payload = buf.get(start..start.checked_add(len)?)?;
        at = start + len;
        Some((tag, payload, children))
    })
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn f32_at(b: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn f64_at(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(tag: &[u8; 4], children: u32, payload: &[u8]) -> Vec<u8> {
        let mut out: Vec<u8> = tag.iter().rev().copied().collect();
        out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&children.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn cue(name: &str, kind: i32, start_ms: f64, len_ms: f64, hotcue: i32) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut out = 1u32.to_le_bytes().to_vec();
        out.extend_from_slice(&u32::try_from(units.len()).unwrap().to_le_bytes());
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&start_ms.to_le_bytes());
        out.extend_from_slice(&len_ms.to_le_bytes());
        out.extend_from_slice(&(-1i32).to_le_bytes());
        out.extend_from_slice(&hotcue.to_le_bytes());
        out
    }

    /// The shape of SOSA UK "The Wave"'s real frame, cut down: a header,
    /// then BPM, key and a cue list with the grid, a hot cue and a loop.
    fn frame(cues: &[Vec<u8>]) -> Vec<u8> {
        let mut list = u32::try_from(cues.len()).unwrap().to_le_bytes().to_vec();
        for c in cues {
            list.extend_from_slice(c);
        }
        let hdr = chunk(b" HDR", 1, &chunk(b"VRSN", 0, &7u32.to_le_bytes()));
        let mut data = chunk(b"HBPM", 0, &132.0f32.to_le_bytes());
        data.extend(chunk(b"MKEY", 0, &21u32.to_le_bytes()));
        data.extend(chunk(b"CUEP", 0, &list));
        data.extend(chunk(b"TIT2", 0, b"ignored"));
        data.extend(chunk(b"SYNC", 1, &chunk(b"LOCK", 0, &1u32.to_le_bytes())));
        let mut root = hdr;
        root.extend(chunk(b"DATA", 5, &data));
        let mut out = chunk(b"TRMD", 2, &root);
        out.extend_from_slice(&[0u8; 16]); // Traktor pads the frame
        out
    }

    #[test]
    fn reads_bpm_key_grid_cues_and_loops() {
        let f = frame(&[
            cue("AutoGrid", 4, 267.149, 0.0, -1),
            cue("Drop", 0, 64_000.0, 0.0, 2),
            cue("n.n.", 5, 96_000.0, 3_636.4, 3),
        ]);
        let e = parse(&f).expect("a frame with data");
        assert_eq!(e.bpm, Some(132.0));
        assert!(e.grid_locked, "SYNC/LOCK");
        assert_eq!(e.key_camelot.as_deref(), Some("8A"), "21 = A minor");
        assert!((e.grid_anchor_secs.unwrap() - 0.267_149).abs() < 1e-9);
        assert_eq!(e.cues.len(), 1);
        assert_eq!(e.cues[0].hotcue, Some(2));
        assert_eq!(e.cues[0].name.as_deref(), Some("Drop"));
        assert!((e.cues[0].position_secs - 64.0).abs() < 1e-9);
        assert_eq!(e.loops.len(), 1);
        assert!((e.loops[0].end_secs - 99.6364).abs() < 1e-9);
        assert_eq!(e.loops[0].name, None, "Traktor's n.n. means no name");
    }

    #[test]
    fn a_cue_list_that_does_not_add_up_is_dropped_whole() {
        let mut f = frame(&[cue("AutoGrid", 4, 500.0, 0.0, -1)]);
        // Claim two cues while holding one: the list overruns.
        let at = f.windows(4).position(|w| w == b"PEUC").unwrap() + 12;
        f[at..at + 4].copy_from_slice(&2u32.to_le_bytes());
        let e = parse(&f).expect("BPM and key still count");
        assert_eq!(e.grid_anchor_secs, None);
        assert!(e.cues.is_empty());
        assert_eq!(e.bpm, Some(132.0));
    }

    #[test]
    fn garbage_never_panics() {
        assert!(parse(&[]).is_none());
        assert!(parse(b"DMRT").is_none());
        let f = frame(&[cue("AutoGrid", 4, 500.0, 0.0, -1)]);
        for cut in 0..f.len() {
            let _ = parse(&f[..cut]);
        }
        let mut huge = f.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&huge).is_none());
    }

    #[test]
    fn a_tagged_file_brings_its_grid_and_is_read_once_per_version() {
        use id3::TagLike;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("The Wave.mp3");
        std::fs::write(&path, [0u8; 64]).unwrap();
        let mut tag = id3::Tag::new();
        tag.add_frame(id3::Frame::with_content(
            "PRIV",
            Content::Private(id3::frame::Private {
                owner_identifier: OWNER.to_string(),
                private_data: frame(&[cue("AutoGrid", 4, 267.149, 0.0, -1)]),
            }),
        ));
        tag.write_to_path(&path, id3::Version::Id3v24).unwrap();

        let lib = Library::open_in_memory().unwrap();
        let volume = crate::volumes::discover_for_path(&path).unwrap();
        lib.upsert_volume(&volume).unwrap();
        let rel = volume.relative_to(&path).unwrap();
        let id = "22222222-2222-2222-2222-222222222222";
        lib.insert_track(id, None, None, None).unwrap();
        let file = |mtime: i64| {
            lib.upsert_track_file(
                id,
                &volume.volume_uuid,
                &rel,
                None,
                None,
                None,
                None,
                None,
                Some(mtime),
            )
            .unwrap();
        };
        let traktor_grid = |lib: &Library| -> Option<f64> {
            lib.connection()
                .query_row(
                    "SELECT bpm FROM track_beatgrids WHERE track_id = ?1 AND source = 'traktor'",
                    params![id],
                    |r| r.get(0),
                )
                .optional()
                .unwrap()
        };

        file(100);
        sync_file_tags(&lib, id, &volume.volume_uuid, &rel, &path).unwrap();
        assert_eq!(traktor_grid(&lib), Some(132.0));

        // Unchanged file: not read again, so a grid removed since stays removed.
        lib.connection()
            .execute("DELETE FROM track_beatgrids", [])
            .unwrap();
        sync_file_tags(&lib, id, &volume.volume_uuid, &rel, &path).unwrap();
        assert_eq!(traktor_grid(&lib), None);

        // The file changed: read again.
        file(200);
        sync_file_tags(&lib, id, &volume.volume_uuid, &rel, &path).unwrap();
        assert_eq!(traktor_grid(&lib), Some(132.0));
    }

    #[test]
    fn serato_and_traktor_in_one_file_each_land_under_their_own_source() {
        use id3::TagLike;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Both.mp3");
        std::fs::write(&path, [0u8; 64]).unwrap();
        let mut grid = vec![0x01, 0x00];
        grid.extend_from_slice(&1u32.to_be_bytes());
        grid.extend_from_slice(&0.5f32.to_be_bytes());
        grid.extend_from_slice(&128.0f32.to_be_bytes());
        grid.push(0);
        let mut tag = id3::Tag::new();
        tag.add_frame(id3::Frame::with_content(
            "PRIV",
            Content::Private(id3::frame::Private {
                owner_identifier: OWNER.to_string(),
                private_data: frame(&[cue("AutoGrid", 4, 267.149, 0.0, -1)]),
            }),
        ));
        tag.add_frame(id3::Frame::with_content(
            "GEOB",
            Content::EncapsulatedObject(id3::frame::EncapsulatedObject {
                mime_type: "application/octet-stream".to_string(),
                filename: String::new(),
                description: geob::BEATGRID.to_string(),
                data: grid,
            }),
        ));
        tag.write_to_path(&path, id3::Version::Id3v24).unwrap();

        let lib = Library::open_in_memory().unwrap();
        let volume = crate::volumes::discover_for_path(&path).unwrap();
        lib.upsert_volume(&volume).unwrap();
        let rel = volume.relative_to(&path).unwrap();
        let id = "33333333-3333-3333-3333-333333333333";
        lib.insert_track(id, None, None, None).unwrap();
        lib.upsert_track_file(
            id,
            &volume.volume_uuid,
            &rel,
            None,
            None,
            None,
            None,
            None,
            Some(1),
        )
        .unwrap();
        sync_file_tags(&lib, id, &volume.volume_uuid, &rel, &path).unwrap();

        let grids: Vec<(String, f64, bool)> = lib
            .connection()
            .prepare("SELECT source, bpm, is_active FROM track_beatgrids WHERE track_id = ?1 ORDER BY source")
            .unwrap()
            .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            grids,
            vec![
                ("serato".to_string(), 128.0, true),
                ("traktor".to_string(), 132.0, false),
            ],
            "both kept; Serato ranks above Traktor"
        );
        let rows: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_metadata_source WHERE track_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "neither app's node gains the file");
        assert!(
            !lib.is_grid_locked(id).unwrap(),
            "Traktor locked its grid, but Serato's unlocked one is in use"
        );
    }

    /// Opt-in: `DUB_TRAKTOR_FILE=/path/to/track.mp3 cargo nextest run -p
    /// dub-library traktor_tag_real --run-ignored only --no-capture`.
    #[test]
    #[ignore = "set DUB_TRAKTOR_FILE to an MP3 Traktor has played"]
    fn traktor_tag_real() {
        let Ok(path) = std::env::var("DUB_TRAKTOR_FILE") else {
            return;
        };
        let e = read_file(Path::new(&path)).expect("a TRAKTOR4 frame");
        eprintln!(
            "bpm={:?} key={:?} grid={:?} cues={} loops={}",
            e.bpm,
            e.key_camelot,
            e.grid_anchor_secs,
            e.cues.len(),
            e.loops.len()
        );
    }
}
