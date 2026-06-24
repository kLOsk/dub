//! Traktor `collection.nml` parser (M12b).
//!
//! A pure, fuzzable `&[u8] -> ParsedCollection` over Traktor's monolithic
//! XML collection. Streaming (quick-xml pull parser) so a 100k-track file
//! stays flat in memory. Read-only; the adapter (`import_traktor`, a later
//! step) maps the parsed output through the shared `upsert_imported_*`
//! writers. This module is *just* the parser — no DB, no filesystem.
//!
//! ## Format notes (validated against a real `collection.nml`)
//! Checked against a real 2024 Traktor export; the inline tests pin the
//! parsing logic and the opt-in `validate_against_real_nml` probe
//! (`DUB_TRAKTOR_NML=…`) re-checks these end to end:
//! - `CUE_V2 START` / `LEN` are in **milliseconds** — ✓ confirmed (real
//!   AutoGrids at e.g. 2150.96 ms → 2.15 s). `÷1000` for seconds.
//! - `<LOCATION>` path: `VOLUME` + `DIR` (Traktor `/:` separator) + `FILE`
//!   — ✓ confirmed. `"Macintosh HD"` → `/`; others → `/Volumes/<name>`
//!   (see [`reconstruct_path`]).
//! - `MUSICAL_KEY VALUE` is 0–23, chromatic: 0–11 major C..B, 12–23 minor
//!   C..B (see [`musical_key_to_camelot`]). Range ✓ confirmed; the exact
//!   per-value→Camelot mapping is documented (the export carried no text
//!   key to cross-check), so double-check if a key reads wrong.
//! - `CUE_V2 TYPE`: 4 = grid/AutoGrid (downbeat anchor), 5 = loop, anything
//!   else = a cue. `HOTCUE` ≥ 0 is the pad slot; −1 = unindexed.
//!
//! Playlists (`<PLAYLISTS>` `<NODE>` tree) are parsed in a follow-up step;
//! `ENTRY` elements inside `<PLAYLISTS>` are deliberately NOT counted as
//! collection tracks (the `in_collection` gate below).

use std::path::PathBuf;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// One cue parsed from a `<CUE_V2>` (hot cue or memory cue).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCue {
    /// Hot-cue slot (0-based), or `None` for an unindexed memory cue.
    pub hotcue: Option<u8>,
    /// Position from the track start, in seconds.
    pub position_secs: f64,
    /// Cue label, if any (Traktor's placeholder `"n.n."` is dropped).
    pub name: Option<String>,
}

/// One loop parsed from a `<CUE_V2 TYPE="5">`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLoop {
    /// Hot-cue slot the loop is stored on, or `None`.
    pub hotcue: Option<u8>,
    /// Loop in-point, seconds.
    pub start_secs: f64,
    /// Loop out-point, seconds.
    pub end_secs: f64,
    /// Loop label, if any.
    pub name: Option<String>,
}

/// One collection track, as Traktor stores it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedEntry {
    /// Reconstructed absolute path to the audio file.
    pub path: Option<PathBuf>,
    /// Verbatim metadata from the source.
    pub artist: Option<String>,
    /// Track title.
    pub title: Option<String>,
    /// Album title.
    pub album: Option<String>,
    /// Genre.
    pub genre: Option<String>,
    /// Free-text comment.
    pub comment: Option<String>,
    /// Tempo, BPM.
    pub bpm: Option<f64>,
    /// Canonical Camelot key (from `MUSICAL_KEY VALUE`), if mappable.
    pub key_camelot: Option<String>,
    /// Verbatim source key (the raw numeric `VALUE`).
    pub key_original: Option<String>,
    /// Downbeat anchor (grid marker), seconds.
    pub grid_anchor_secs: Option<f64>,
    /// Track length in seconds, from `<INFO PLAYTIME_FLOAT>` (or the integer
    /// `PLAYTIME`). Lets the browser show a length before the track is
    /// decoded; analysis later refines `tracks.duration_ms`.
    pub duration_secs: Option<f64>,
    /// Hot/memory cues.
    pub cues: Vec<ParsedCue>,
    /// Stored loops.
    pub loops: Vec<ParsedLoop>,
}

/// One node of Traktor's `<PLAYLISTS>` tree — a folder or a playlist. Both
/// map onto a row in the read-only `imported_crates` mirror; a folder simply
/// carries no tracks. `parent` is an index back into
/// [`ParsedCollection::playlists`] (document order guarantees a parent is
/// emitted before its children), or `None` for a top-level node. Smart
/// playlists (`SMARTLIST`) and the transparent `$ROOT` folder are not emitted.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedPlaylist {
    /// Display name (the `<NODE NAME>` attribute).
    pub name: String,
    /// Index of the enclosing folder in [`ParsedCollection::playlists`].
    pub parent: Option<usize>,
    /// Member track paths in playlist order, reconstructed from each
    /// `<PRIMARYKEY>`. Resolved to track ids by the import adapter.
    pub track_paths: Vec<PathBuf>,
}

/// The parsed collection: its tracks and its playlist/folder tree.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedCollection {
    /// Collection tracks, in document order.
    pub entries: Vec<ParsedEntry>,
    /// Playlist/folder tree, in document order (parents precede children).
    pub playlists: Vec<ParsedPlaylist>,
}

/// One open `<NODE>` while walking the `<PLAYLISTS>` tree. Tracks the
/// `imported_crates` row this node created (if any) so children can find
/// their parent, and whether it is a playlist (so its `<PRIMARYKEY>`s feed
/// track membership and its close clears the "current playlist").
struct NodeFrame {
    crate_index: Option<usize>,
    is_playlist: bool,
}

/// Parse failure. The parser never panics on malformed input (it is a
/// fuzz target — PRD §2.2.5); it returns this instead.
#[derive(Debug)]
pub struct ParseError(String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "traktor nml parse: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse a Traktor `collection.nml` byte buffer into its collection tracks.
///
/// # Errors
/// Returns [`ParseError`] on a hard XML error. Missing/garbled fields within
/// an otherwise-readable document are tolerated (that entry simply carries
/// `None` for the field) rather than failing the whole parse.
pub fn parse_nml(data: &[u8]) -> Result<ParsedCollection, ParseError> {
    let mut reader = Reader::from_reader(data);
    let mut buf = Vec::new();
    let mut out = ParsedCollection::default();
    // Only `<ENTRY>`s inside `<COLLECTION>` are tracks; `<PLAYLISTS>` reuses
    // the `ENTRY` tag for membership and must not be counted.
    let mut in_collection = false;
    let mut current: Option<ParsedEntry> = None;
    // `<PLAYLISTS>` walk: a stack of open `<NODE>`s plus the index of the
    // playlist currently collecting `<PRIMARYKEY>` members.
    let mut in_playlists = false;
    let mut node_stack: Vec<NodeFrame> = Vec::new();
    let mut current_playlist: Option<usize> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"COLLECTION" => in_collection = true,
                b"PLAYLISTS" => in_playlists = true,
                b"ENTRY" if in_collection => current = Some(parse_entry_open(&e)),
                b"NODE" if in_playlists => {
                    open_node(&e, &mut out, &mut node_stack, &mut current_playlist)
                }
                b"PRIMARYKEY" if in_playlists => add_primarykey(&e, &mut out, current_playlist),
                _ => {
                    if in_collection {
                        if let Some(cur) = current.as_mut() {
                            apply_child(cur, &e);
                        }
                    }
                }
            },
            Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"PRIMARYKEY" if in_playlists => add_primarykey(&e, &mut out, current_playlist),
                _ => {
                    if in_collection {
                        if let Some(cur) = current.as_mut() {
                            apply_child(cur, &e);
                        }
                    }
                }
            },
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"COLLECTION" => in_collection = false,
                b"PLAYLISTS" => in_playlists = false,
                b"ENTRY" if in_collection => {
                    if let Some(cur) = current.take() {
                        out.entries.push(cur);
                    }
                }
                b"NODE" if in_playlists => {
                    if let Some(frame) = node_stack.pop() {
                        if frame.is_playlist {
                            current_playlist = None;
                        }
                    }
                }
                _ => {}
            },
            Ok(_) => {}
            Err(e) => return Err(ParseError(e.to_string())),
        }
        buf.clear();
    }
    Ok(out)
}

/// Read an attribute's unescaped value as an owned `String`, or `None` if
/// the element has no such attribute (or it fails to decode).
fn attr(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Start a new entry from the `<ENTRY>` tag's own attributes.
fn parse_entry_open(e: &BytesStart) -> ParsedEntry {
    ParsedEntry {
        title: attr(e, b"TITLE"),
        artist: attr(e, b"ARTIST"),
        ..ParsedEntry::default()
    }
}

/// Fold one child element (`<LOCATION>`, `<INFO>`, `<TEMPO>`, …) into the
/// entry under construction.
fn apply_child(cur: &mut ParsedEntry, e: &BytesStart) {
    match e.name().as_ref() {
        b"LOCATION" => {
            cur.path = reconstruct_path(
                attr(e, b"VOLUME").as_deref(),
                attr(e, b"DIR").as_deref(),
                attr(e, b"FILE").as_deref(),
            );
        }
        b"ALBUM" if cur.album.is_none() => cur.album = attr(e, b"TITLE"),
        b"INFO" => {
            if cur.genre.is_none() {
                cur.genre = attr(e, b"GENRE");
            }
            if cur.comment.is_none() {
                cur.comment = attr(e, b"COMMENT");
            }
            if cur.duration_secs.is_none() {
                // PLAYTIME_FLOAT (fractional seconds) preferred; fall back to
                // the integer PLAYTIME.
                cur.duration_secs = attr(e, b"PLAYTIME_FLOAT")
                    .or_else(|| attr(e, b"PLAYTIME"))
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|d| *d > 0.0);
            }
        }
        b"TEMPO" => cur.bpm = attr(e, b"BPM").and_then(|s| s.parse::<f64>().ok()),
        b"MUSICAL_KEY" => {
            if let Some(v) = attr(e, b"VALUE") {
                if let Ok(n) = v.parse::<u8>() {
                    cur.key_camelot = musical_key_to_camelot(n).map(String::from);
                }
                cur.key_original = Some(v);
            }
        }
        b"CUE_V2" => apply_cue(cur, e),
        _ => {}
    }
}

/// Classify a `<CUE_V2>` into the entry's grid anchor / cue / loop.
fn apply_cue(cur: &mut ParsedEntry, e: &BytesStart) {
    let Some(start_ms) = attr(e, b"START").and_then(|s| s.parse::<f64>().ok()) else {
        return;
    };
    let start_secs = start_ms / 1000.0; // START is ms (validated on a real export).
    let cue_type = attr(e, b"TYPE").and_then(|s| s.parse::<i64>().ok());
    let hotcue = attr(e, b"HOTCUE")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&h| h >= 0)
        .and_then(|h| u8::try_from(h).ok());
    let name = attr(e, b"NAME").filter(|s| !s.is_empty() && s != "n.n.");

    match cue_type {
        Some(4) => {
            // AutoGrid: the first one is the downbeat anchor.
            if cur.grid_anchor_secs.is_none() {
                cur.grid_anchor_secs = Some(start_secs);
            }
        }
        Some(5) => {
            let len_ms = attr(e, b"LEN")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            cur.loops.push(ParsedLoop {
                hotcue,
                start_secs,
                end_secs: start_secs + len_ms / 1000.0,
                name,
            });
        }
        _ => cur.cues.push(ParsedCue {
            hotcue,
            position_secs: start_secs,
            name,
        }),
    }
}

/// Reconstruct an absolute path from a `<LOCATION>`'s `VOLUME`/`DIR`/`FILE`.
/// Traktor writes `DIR` with `/:` as the separator and a volume name;
/// `"Macintosh HD"` is the macOS boot volume (root), others mount under
/// `/Volumes/<name>`. VALIDATE against a real export — the volume→mount
/// mapping is the fragile part on multi-volume rigs.
fn reconstruct_path(
    volume: Option<&str>,
    dir: Option<&str>,
    file: Option<&str>,
) -> Option<PathBuf> {
    let file = file?;
    let dir = dir.unwrap_or("").replace("/:", "/");
    let mut path = String::new();
    if let Some(v) = volume {
        if !v.is_empty() && v != "Macintosh HD" {
            path.push_str("/Volumes/");
            path.push_str(v);
        }
    }
    path.push_str(&dir);
    if !path.ends_with('/') {
        path.push('/');
    }
    path.push_str(file);
    Some(PathBuf::from(path))
}

/// Open a `<PLAYLISTS>` `<NODE>`: append an `imported_crates`-bound entry for
/// a folder or playlist, push a frame so children can find their parent, and
/// mark a playlist as the current member-collecting node. The transparent
/// `$ROOT` folder and `SMARTLIST` nodes create no row (we can't resolve a
/// smart playlist's dynamic membership to fixed track ids).
fn open_node(
    e: &BytesStart,
    out: &mut ParsedCollection,
    node_stack: &mut Vec<NodeFrame>,
    current_playlist: &mut Option<usize>,
) {
    let node_type = attr(e, b"TYPE").unwrap_or_default();
    let name = attr(e, b"NAME").unwrap_or_default();
    let parent = node_stack.iter().rev().find_map(|f| f.crate_index);
    let mut frame = NodeFrame {
        crate_index: None,
        is_playlist: false,
    };
    match node_type.as_str() {
        "FOLDER" if name != "$ROOT" => {
            out.playlists.push(ParsedPlaylist {
                name,
                parent,
                track_paths: Vec::new(),
            });
            frame.crate_index = Some(out.playlists.len() - 1);
        }
        "PLAYLIST" => {
            out.playlists.push(ParsedPlaylist {
                name,
                parent,
                track_paths: Vec::new(),
            });
            let idx = out.playlists.len() - 1;
            frame.crate_index = Some(idx);
            frame.is_playlist = true;
            *current_playlist = Some(idx);
        }
        // `$ROOT` and `SMARTLIST` (and anything unknown) are transparent: no
        // row, so children attach to the nearest real ancestor.
        _ => {}
    }
    node_stack.push(frame);
}

/// Add one `<PRIMARYKEY>` track to the current playlist, if any. A key that
/// doesn't reconstruct to a path (malformed / non-TRACK) is silently dropped.
fn add_primarykey(e: &BytesStart, out: &mut ParsedCollection, current_playlist: Option<usize>) {
    let Some(idx) = current_playlist else { return };
    if let Some(key) = attr(e, b"KEY") {
        if let Some(path) = path_from_primarykey(&key) {
            out.playlists[idx].track_paths.push(path);
        }
    }
}

/// Reconstruct an absolute path from a `<PRIMARYKEY>` playlist-membership key.
/// Unlike `<LOCATION>`, the key is one string of the form
/// `VOLUME/:dir/:dir/:file` — the volume is the segment before the first
/// `/:`. Same boot-volume→root, other→`/Volumes/<name>` rule as
/// [`reconstruct_path`], so a playlist member's path matches its collection
/// entry's path exactly (the adapter joins on it). Returns `None` if the key
/// has no `/:` separator (not a real track key).
fn path_from_primarykey(key: &str) -> Option<PathBuf> {
    let (volume, rest) = key.split_once("/:")?;
    let rest = rest.replace("/:", "/");
    let mut path = String::new();
    if !volume.is_empty() && volume != "Macintosh HD" {
        path.push_str("/Volumes/");
        path.push_str(volume);
    }
    path.push('/');
    path.push_str(&rest);
    Some(PathBuf::from(path))
}

/// Map Traktor `MUSICAL_KEY VALUE` (0–23) to canonical Camelot. The Camelot
/// math below is correct *given* Traktor's chromatic 0–23 ordering (0–11 =
/// major C..B, 12–23 = minor C..B); that ORDERING is the assumption to
/// confirm against a real export.
fn musical_key_to_camelot(value: u8) -> Option<&'static str> {
    // 0..11 major C,C#,D,D#,E,F,F#,G,G#,A,A#,B ; 12..23 minor C..B.
    const TABLE: [&str; 24] = [
        "8B", "3B", "10B", "5B", "12B", "7B", "2B", "9B", "4B", "11B", "6B", "1B", // major
        "5A", "12A", "7A", "2A", "9A", "4A", "11A", "6A", "1A", "8A", "3A", "10A", // minor
    ];
    TABLE.get(value as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_entry() {
        let nml = br#"<?xml version="1.0" encoding="UTF-8"?>
<NML VERSION="19">
  <COLLECTION ENTRIES="1">
    <ENTRY TITLE="Test Track" ARTIST="Test Artist">
      <LOCATION DIR="/:Users/:dj/:Music/:" FILE="track.mp3" VOLUME="Macintosh HD"/>
      <ALBUM TITLE="Test Album"/>
      <INFO GENRE="Techno" COMMENT="hello" PLAYTIME="167" PLAYTIME_FLOAT="166.523"/>
      <TEMPO BPM="128.000000"/>
      <MUSICAL_KEY VALUE="0"/>
      <CUE_V2 NAME="AutoGrid" TYPE="4" START="500.0" LEN="0" HOTCUE="-1"/>
      <CUE_V2 NAME="Drop" TYPE="0" START="32000.0" LEN="0" HOTCUE="1"/>
      <CUE_V2 NAME="n.n." TYPE="5" START="64000.0" LEN="4000.0" HOTCUE="2"/>
    </ENTRY>
  </COLLECTION>
  <PLAYLISTS>
    <NODE TYPE="PLAYLIST" NAME="ignored">
      <PLAYLIST><ENTRY><PRIMARYKEY TYPE="TRACK" KEY="x"/></ENTRY></PLAYLIST>
    </NODE>
  </PLAYLISTS>
</NML>"#;
        let c = parse_nml(nml).unwrap();
        // The ENTRY inside PLAYLISTS is NOT counted as a track.
        assert_eq!(c.entries.len(), 1);
        let e = &c.entries[0];
        assert_eq!(e.title.as_deref(), Some("Test Track"));
        assert_eq!(e.artist.as_deref(), Some("Test Artist"));
        assert_eq!(e.album.as_deref(), Some("Test Album"));
        assert_eq!(e.genre.as_deref(), Some("Techno"));
        assert_eq!(e.comment.as_deref(), Some("hello"));
        assert!((e.duration_secs.unwrap() - 166.523).abs() < 1e-6); // PLAYTIME_FLOAT
        assert!((e.bpm.unwrap() - 128.0).abs() < 1e-9);
        assert_eq!(e.key_camelot.as_deref(), Some("8B")); // VALUE 0 = C major
        assert_eq!(e.key_original.as_deref(), Some("0"));
        assert_eq!(
            e.path.as_deref(),
            Some(std::path::Path::new("/Users/dj/Music/track.mp3"))
        );
        assert!((e.grid_anchor_secs.unwrap() - 0.5).abs() < 1e-9); // 500 ms
        assert_eq!(e.cues.len(), 1);
        assert_eq!(e.cues[0].hotcue, Some(1));
        assert_eq!(e.cues[0].name.as_deref(), Some("Drop"));
        assert!((e.cues[0].position_secs - 32.0).abs() < 1e-9);
        assert_eq!(e.loops.len(), 1);
        assert_eq!(e.loops[0].name, None); // "n.n." dropped
        assert!((e.loops[0].start_secs - 64.0).abs() < 1e-9);
        assert!((e.loops[0].end_secs - 68.0).abs() < 1e-9);
    }

    #[test]
    fn non_boot_volume_mounts_under_volumes() {
        let nml = br#"<NML><COLLECTION>
          <ENTRY TITLE="t"><LOCATION DIR="/:Music/:" FILE="a.wav" VOLUME="USB Drive"/></ENTRY>
        </COLLECTION></NML>"#;
        let c = parse_nml(nml).unwrap();
        assert_eq!(
            c.entries[0].path.as_deref(),
            Some(std::path::Path::new("/Volumes/USB Drive/Music/a.wav"))
        );
    }

    #[test]
    fn playlists_entries_are_not_tracks() {
        let nml = br#"<NML><PLAYLISTS><NODE><PLAYLIST>
          <ENTRY><PRIMARYKEY TYPE="TRACK" KEY="x"/></ENTRY>
        </PLAYLIST></NODE></PLAYLISTS></NML>"#;
        assert_eq!(parse_nml(nml).unwrap().entries.len(), 0);
    }

    #[test]
    fn playlist_tree_nests_folders_skips_smartlists_and_collects_members() {
        let nml = br#"<NML>
  <COLLECTION>
    <ENTRY TITLE="A"><LOCATION DIR="/:Music/:" FILE="a.wav" VOLUME="Macintosh HD"/></ENTRY>
    <ENTRY TITLE="B"><LOCATION DIR="/:Music/:" FILE="b.wav" VOLUME="Macintosh HD"/></ENTRY>
  </COLLECTION>
  <PLAYLISTS>
    <NODE TYPE="FOLDER" NAME="$ROOT">
      <SUBNODES COUNT="2">
        <NODE TYPE="FOLDER" NAME="Sets">
          <SUBNODES COUNT="1">
            <NODE TYPE="PLAYLIST" NAME="Friday">
              <PLAYLIST ENTRIES="2" TYPE="LIST">
                <ENTRY><PRIMARYKEY TYPE="TRACK" KEY="Macintosh HD/:Music/:a.wav"/></ENTRY>
                <ENTRY><PRIMARYKEY TYPE="TRACK" KEY="Macintosh HD/:Music/:b.wav"/></ENTRY>
              </PLAYLIST>
            </NODE>
          </SUBNODES>
        </NODE>
        <NODE TYPE="SMARTLIST" NAME="Recent">
          <SMARTLIST><SEARCH_EXPRESSION VERSION="1" QUERY=""/></SMARTLIST>
        </NODE>
      </SUBNODES>
    </NODE>
  </PLAYLISTS>
</NML>"#;
        let c = parse_nml(nml).unwrap();
        // $ROOT is transparent and the SMARTLIST is skipped: only the real
        // folder + playlist survive.
        assert_eq!(c.playlists.len(), 2);

        let folder = &c.playlists[0];
        assert_eq!(folder.name, "Sets");
        assert_eq!(folder.parent, None);
        assert!(folder.track_paths.is_empty());

        let playlist = &c.playlists[1];
        assert_eq!(playlist.name, "Friday");
        assert_eq!(playlist.parent, Some(0)); // nested under "Sets"
        assert_eq!(
            playlist.track_paths,
            vec![
                std::path::PathBuf::from("/Music/a.wav"),
                std::path::PathBuf::from("/Music/b.wav"),
            ]
        );
        // Membership paths reconstruct identically to the collection entries
        // (the adapter joins playlists to tracks on this).
        assert_eq!(
            playlist.track_paths[0].as_path(),
            c.entries[0].path.as_deref().unwrap()
        );
    }

    #[test]
    fn primarykey_non_boot_volume_matches_location_reconstruction() {
        assert_eq!(
            path_from_primarykey("USB Drive/:Music/:a.wav"),
            Some(std::path::PathBuf::from("/Volumes/USB Drive/Music/a.wav"))
        );
        // No `/:` separator → not a real track key.
        assert_eq!(path_from_primarykey("garbage"), None);
    }

    #[test]
    fn minor_key_maps_to_a_side() {
        // VALUE 21 = A minor (12 + 9) → 8A.
        assert_eq!(musical_key_to_camelot(21), Some("8A"));
        assert_eq!(musical_key_to_camelot(24), None); // out of range
    }

    #[test]
    fn malformed_input_never_panics() {
        for bad in [
            &b""[..],
            b"not xml at all",
            b"<NML><COLLECTION><ENTRY TITLE=",
            b"<NML><COLLECTION><ENTRY></WRONG></NML>",
            &[0xff, 0xfe, 0x00, 0x01][..],
        ] {
            let _ = parse_nml(bad); // Ok or Err both fine — must not panic.
        }
    }

    /// Opt-in real-file validation. Set `DUB_TRAKTOR_NML` to a real
    /// `collection.nml` and run with `--ignored --nocapture`; skips cleanly
    /// when the env var is unset (CI-safe — never a hard dep on a personal
    /// file). Confirms the whole parse end to end and that grid anchors land
    /// in seconds (catches a ms/seconds regression in `CUE_V2 START`).
    #[test]
    #[ignore = "set DUB_TRAKTOR_NML to a real collection.nml"]
    fn validate_against_real_nml() {
        let Ok(path) = std::env::var("DUB_TRAKTOR_NML") else {
            eprintln!("DUB_TRAKTOR_NML unset — skipping real-file validation");
            return;
        };
        let data = std::fs::read(&path).expect("read DUB_TRAKTOR_NML");
        let c = parse_nml(&data).expect("real collection.nml must parse");
        eprintln!("parsed {} entries from {path}", c.entries.len());
        for e in c.entries.iter().take(5) {
            eprintln!(
                "  {:?} | {:?} BPM | grid {:?}s | key {:?} | {} cues, {} loops | {:?}",
                e.title,
                e.bpm,
                e.grid_anchor_secs,
                e.key_camelot,
                e.cues.len(),
                e.loops.len(),
                e.path
            );
        }
        assert!(
            !c.entries.is_empty(),
            "real collection parsed to zero tracks"
        );
        for e in &c.entries {
            assert!(
                e.path.is_some(),
                "entry {:?} has no reconstructed path",
                e.title
            );
            if let Some(g) = e.grid_anchor_secs {
                assert!(
                    (0.0..120.0).contains(&g),
                    "grid anchor {g}s out of range — CUE_V2 START unit wrong?"
                );
            }
        }

        eprintln!("parsed {} playlist/folder nodes", c.playlists.len());
        let entry_paths: std::collections::HashSet<_> =
            c.entries.iter().filter_map(|e| e.path.clone()).collect();
        for p in c.playlists.iter().take(10) {
            let parent = p.parent.and_then(|i| c.playlists.get(i)).map(|f| &f.name);
            eprintln!(
                "  {:?} (parent {:?}) — {} tracks",
                p.name,
                parent,
                p.track_paths.len()
            );
        }
        // Every playlist member must resolve to a path that *also* appears as
        // a collection entry — proves PRIMARYKEY and LOCATION reconstruct
        // identically, which is what lets the adapter join the two.
        let mut members = 0usize;
        let mut matched = 0usize;
        for p in &c.playlists {
            for path in &p.track_paths {
                members += 1;
                if entry_paths.contains(path) {
                    matched += 1;
                }
            }
        }
        eprintln!("{matched}/{members} playlist members resolve to a collection entry");
        if members > 0 {
            assert_eq!(
                matched, members,
                "some playlist members did not match any collection entry path"
            );
        }
    }
}
