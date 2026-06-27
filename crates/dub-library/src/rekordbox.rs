//! rekordbox `rekordbox.xml` (`DJ_PLAYLISTS`) parser (M12d).
//!
//! A pure, fuzzable `&[u8] -> ParsedLibrary` over rekordbox's exported XML
//! collection ("File → Export Collection in xml format"). Streaming
//! (quick-xml pull parser) so a large export stays flat in memory. Read-only;
//! the adapter ([`crate::rekordbox_import`]) maps the parsed output through the
//! shared `upsert_imported_*` writers. This module is *just* the parser — no
//! DB, no filesystem.
//!
//! We deliberately read the **XML export**, not the encrypted rekordbox 6/7
//! `master.db` (SQLCipher): the XML is Pioneer/AlphaTheta's documented
//! interchange format, GPL-clean, and reuses `quick-xml` (already a dep), where
//! decrypting `master.db` would mean a reverse-engineered key + C crypto + an
//! undocumented, version-fragile schema. See PRD §8 and AGENTS.md.
//!
//! ## Format notes (validated against a real rekordbox 7.2 export)
//! Pinned by the inline tests + the opt-in `validate_against_real_xml` probe
//! (`DUB_REKORDBOX_XML=…`). Validated directly against real data: the `TRACK`
//! attributes, `TEMPO` beat grids (constant + per-beat), and the `PLAYLISTS`
//! `NODE` tree. Built from rekordbox's documented schema (the validation
//! export carried none): `POSITION_MARK` cues/loops, `Tonality` keys, and
//! populated playlist membership — pinned by synthetic fixtures here, flagged
//! to re-confirm against an export that contains them.
//!
//! - `<TRACK>`: a self-closing tag when it has no grid/cues, or a container
//!   holding `<TEMPO>` / `<POSITION_MARK>` children. `TrackID` is the integer
//!   playlist-membership key (the default `KeyType="0"`). `TotalTime` is
//!   **integer seconds** (not ms — coarser than iTunes). `Location` is a
//!   percent-encoded `file://localhost/…` URL (same shape as iTunes).
//! - `<TEMPO Inizio Bpm Metro Battito>`: `Inizio` = grid anchor seconds, `Bpm`
//!   the tempo there, `Battito` (1–4) the beat-in-bar at the anchor. The first
//!   `TEMPO` is the grid anchor; extra ones are per-beat / variable-tempo
//!   markers we don't model (single anchor + bpm + bar phase).
//! - `<POSITION_MARK Name Type Start End Num Red Green Blue>`: `Type` 0 = cue,
//!   4 = loop (1/2 = fade, 3 = load — skipped). `Num` −1 = memory cue, 0–7 =
//!   hot-cue pad slot. `Start`/`End` in seconds; `Red`/`Green`/`Blue` 0–255.
//! - `<PLAYLISTS>`: nested `<NODE>`. `Type="0"` = folder (the top `Name="ROOT"`
//!   is transparent), `Type="1"` = playlist whose `<TRACK Key="…"/>` children
//!   reference collection tracks by `TrackID` (`KeyType="0"`).

use std::path::PathBuf;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// One cue parsed from a `<POSITION_MARK Type="0">` (hot or memory cue).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCue {
    /// Hot-cue pad slot (`Num` 0–7), or `None` for an unindexed memory cue
    /// (`Num = -1`).
    pub hotcue: Option<u8>,
    /// Position from the track start, in seconds (`Start`).
    pub position_secs: f64,
    /// Cue label, if any.
    pub name: Option<String>,
    /// `#RRGGBB` from the `Red`/`Green`/`Blue` attributes, if all present.
    pub color: Option<String>,
}

/// One loop parsed from a `<POSITION_MARK Type="4">`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLoop {
    /// Hot-cue pad slot the loop is stored on, or `None` (memory loop).
    pub hotcue: Option<u8>,
    /// Loop in-point, seconds (`Start`).
    pub start_secs: f64,
    /// Loop out-point, seconds (`End`).
    pub end_secs: f64,
    /// Loop label, if any.
    pub name: Option<String>,
    /// `#RRGGBB`, if present.
    pub color: Option<String>,
}

/// One collection track, as rekordbox stores it in the XML.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedTrack {
    /// `TrackID` — the integer playlists reference by (`KeyType="0"`).
    pub track_id: i64,
    /// Absolute path, decoded from the `Location` `file://` URL.
    pub path: Option<PathBuf>,
    /// `Artist`.
    pub artist: Option<String>,
    /// `Name`.
    pub title: Option<String>,
    /// `Album`.
    pub album: Option<String>,
    /// `Genre`.
    pub genre: Option<String>,
    /// `Composer`.
    pub composer: Option<String>,
    /// `Comments`.
    pub comment: Option<String>,
    /// `Year` (0 dropped).
    pub year: Option<i32>,
    /// `TrackNumber` (0 dropped).
    pub track_number: Option<i32>,
    /// Tempo, BPM — `AverageBpm`, falling back to the first `TEMPO`'s `Bpm`.
    pub bpm: Option<f64>,
    /// Musical key, verbatim from `Tonality` (rekordbox's own notation —
    /// Camelot `8B`, Open-Key `5d`, or classical `Abm`; the schema stores it
    /// as-is, see `track_keys`). Empty dropped.
    pub key: Option<String>,
    /// Grid anchor, seconds — the first `<TEMPO>`'s `Inizio`.
    pub grid_anchor_secs: Option<f64>,
    /// Grid tempo — the first `<TEMPO>`'s `Bpm`.
    pub grid_bpm: Option<f64>,
    /// Beat-in-bar at the anchor, 0-based (`Battito − 1`, clamped 0–3).
    pub grid_bar_phase: u8,
    /// Track length in seconds (`TotalTime`, integer). Lets the browser show a
    /// length before decode; analysis later refines `tracks.duration_ms`.
    pub duration_secs: Option<f64>,
    /// Hot/memory cues.
    pub cues: Vec<ParsedCue>,
    /// Stored loops.
    pub loops: Vec<ParsedLoop>,
}

/// One node of rekordbox's `<PLAYLISTS>` tree — a folder or a playlist. Both
/// map onto a row in the read-only `imported_crates` mirror; a folder carries
/// no tracks. `parent` is an index back into [`ParsedLibrary::playlists`]
/// (document order guarantees a parent precedes its children), or `None` for a
/// top-level node. The transparent `ROOT` folder is not emitted.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedPlaylist {
    /// Display name (`NODE Name`).
    pub name: String,
    /// Index of the enclosing folder in [`ParsedLibrary::playlists`].
    pub parent: Option<usize>,
    /// Member `TrackID`s in playlist order (`<TRACK Key>` under `KeyType="0"`).
    /// Resolved to track ids by the import adapter.
    pub track_ids: Vec<i64>,
}

/// The parsed export: its tracks and its playlist/folder tree.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedLibrary {
    /// Collection tracks, in document order.
    pub tracks: Vec<ParsedTrack>,
    /// Playlist/folder tree, in document order (parents precede children).
    pub playlists: Vec<ParsedPlaylist>,
}

/// One open `<NODE>` while walking the `<PLAYLISTS>` tree. Tracks the
/// `imported_crates` row this node created (so children find their parent),
/// whether it is a playlist (so its `<TRACK>` children feed membership), and
/// whether its membership keys are `TrackID`s (`KeyType="0"`) — we only resolve
/// id-keyed playlists (location-keyed `KeyType="1"` are rare and import empty).
struct NodeFrame {
    crate_index: Option<usize>,
    is_playlist: bool,
    keys_are_ids: bool,
}

/// Parse failure. The parser never panics on malformed input (it is a fuzz
/// target — PRD §2.2.5); it returns this instead.
#[derive(Debug)]
pub struct ParseError(String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rekordbox xml parse: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse a rekordbox `rekordbox.xml` byte buffer into its tracks + playlists.
///
/// # Errors
/// Returns [`ParseError`] on a hard XML error. Missing/garbled fields within an
/// otherwise-readable document are tolerated (that field is simply `None`)
/// rather than failing the whole parse.
pub fn parse_xml(data: &[u8]) -> Result<ParsedLibrary, ParseError> {
    let mut reader = Reader::from_reader(data);
    let mut buf = Vec::new();
    let mut out = ParsedLibrary::default();
    // `<TRACK>` means a collection track inside `<COLLECTION>` but a membership
    // ref inside `<PLAYLISTS>`; gate the two the same way the Traktor parser
    // gates its reused `ENTRY` tag.
    let mut in_collection = false;
    let mut in_playlists = false;
    let mut current: Option<ParsedTrack> = None;
    let mut node_stack: Vec<NodeFrame> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"COLLECTION" => in_collection = true,
                b"PLAYLISTS" => in_playlists = true,
                b"TRACK" if in_collection => current = Some(parse_track_open(&e)),
                b"NODE" if in_playlists => node_stack.push(open_node(&e, &mut out, &node_stack)),
                _ => {
                    if in_collection {
                        if let Some(cur) = current.as_mut() {
                            apply_child(cur, &e);
                        }
                    }
                }
            },
            Ok(Event::Empty(e)) => match e.name().as_ref() {
                // Self-closing collection track (no grid/cues).
                b"TRACK" if in_collection && current.is_none() => {
                    out.tracks.push(parse_track_open(&e));
                }
                // Self-closing folder / empty playlist node.
                b"NODE" if in_playlists => {
                    let _ = open_node(&e, &mut out, &node_stack);
                }
                // Playlist membership ref (`<TRACK Key=…/>` inside PLAYLISTS).
                b"TRACK" if in_playlists => add_member(&e, &mut out, &node_stack),
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
                b"TRACK" if in_collection => {
                    if let Some(cur) = current.take() {
                        out.tracks.push(cur);
                    }
                }
                b"NODE" if in_playlists => {
                    node_stack.pop();
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

/// Read an attribute's unescaped value as an owned `String`, or `None` if the
/// element has no such attribute (or it fails to decode).
fn attr(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Non-empty attribute value, or `None` (rekordbox writes `""` for "unset").
fn attr_present(e: &BytesStart, key: &[u8]) -> Option<String> {
    attr(e, key).filter(|s| !s.is_empty())
}

/// Parse an integer attribute, dropping rekordbox's `0` "unset" sentinel when
/// `drop_zero` is set (Year / TrackNumber use 0 for absent).
fn attr_int(e: &BytesStart, key: &[u8], drop_zero: bool) -> Option<i32> {
    attr(e, key)
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|&n| !drop_zero || n != 0)
}

/// Build a `ParsedTrack` from a `<TRACK>` element's own attributes (the grid /
/// cues arrive as children for a non-self-closing track).
fn parse_track_open(e: &BytesStart) -> ParsedTrack {
    let bpm = attr(e, b"AverageBpm")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|b| *b > 0.0);
    // `TotalTime` is integer seconds.
    let duration_secs = attr(e, b"TotalTime")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| *d > 0.0);
    ParsedTrack {
        track_id: attr(e, b"TrackID")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0),
        path: attr(e, b"Location")
            .as_deref()
            .and_then(decode_file_url)
            .map(PathBuf::from),
        title: attr_present(e, b"Name"),
        artist: attr_present(e, b"Artist"),
        album: attr_present(e, b"Album"),
        genre: attr_present(e, b"Genre"),
        composer: attr_present(e, b"Composer"),
        comment: attr_present(e, b"Comments"),
        year: attr_int(e, b"Year", true),
        track_number: attr_int(e, b"TrackNumber", true),
        bpm,
        key: attr_present(e, b"Tonality"),
        duration_secs,
        ..ParsedTrack::default()
    }
}

/// Fold one child element (`<TEMPO>`, `<POSITION_MARK>`) into the track under
/// construction.
fn apply_child(cur: &mut ParsedTrack, e: &BytesStart) {
    match e.name().as_ref() {
        b"TEMPO" => apply_tempo(cur, e),
        b"POSITION_MARK" => apply_position_mark(cur, e),
        _ => {}
    }
}

/// The first `<TEMPO>` sets the grid anchor / bpm / bar phase; later ones are
/// per-beat or variable-tempo markers we don't model.
fn apply_tempo(cur: &mut ParsedTrack, e: &BytesStart) {
    if cur.grid_anchor_secs.is_some() {
        return;
    }
    let Some(inizio) = attr(e, b"Inizio").and_then(|s| s.parse::<f64>().ok()) else {
        return;
    };
    cur.grid_anchor_secs = Some(inizio);
    cur.grid_bpm = attr(e, b"Bpm")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|b| *b > 0.0);
    // Battito 1–4 (beat-in-bar) → 0-based bar phase.
    let battito = attr(e, b"Battito")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1);
    cur.grid_bar_phase = u8::try_from(battito.clamp(1, 4) - 1).unwrap_or(0);
    // If the track-level AverageBpm was absent, take the grid tempo for it.
    if cur.bpm.is_none() {
        cur.bpm = cur.grid_bpm;
    }
}

/// Classify a `<POSITION_MARK>` into the track's cues / loops. `Type` 0 = cue,
/// 4 = loop; 1/2 (fade) and 3 (load) are skipped. `Num` −1 = memory, 0–7 = hot.
fn apply_position_mark(cur: &mut ParsedTrack, e: &BytesStart) {
    let Some(start) = attr(e, b"Start").and_then(|s| s.parse::<f64>().ok()) else {
        return;
    };
    let mark_type = attr(e, b"Type").and_then(|s| s.parse::<i64>().ok());
    let hotcue = attr(e, b"Num")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&n| n >= 0)
        .and_then(|n| u8::try_from(n).ok());
    let name = attr_present(e, b"Name");
    let color = parse_color(e);

    match mark_type {
        Some(4) => {
            let end = attr(e, b"End")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(start);
            cur.loops.push(ParsedLoop {
                hotcue,
                start_secs: start,
                end_secs: end,
                name,
                color,
            });
        }
        // 0 = cue; treat anything that isn't a loop/fade/load as a cue too.
        Some(0) | None => cur.cues.push(ParsedCue {
            hotcue,
            position_secs: start,
            name,
            color,
        }),
        // 1 = fade-in, 2 = fade-out, 3 = load — not surfaced as hot cues.
        Some(_) => {}
    }
}

/// `#RRGGBB` from `Red`/`Green`/`Blue` (0–255), or `None` if any is missing.
fn parse_color(e: &BytesStart) -> Option<String> {
    let r = attr(e, b"Red")?.parse::<u8>().ok()?;
    let g = attr(e, b"Green")?.parse::<u8>().ok()?;
    let b = attr(e, b"Blue")?.parse::<u8>().ok()?;
    Some(format!("#{r:02X}{g:02X}{b:02X}"))
}

/// Open a `<PLAYLISTS>` `<NODE>`: append an `imported_crates`-bound row for a
/// folder (`Type="0"`, except the transparent `ROOT`) or a playlist
/// (`Type="1"`), and return a frame so children find their parent. The frame is
/// pushed by the caller for a container node and dropped for a self-closing one.
fn open_node(e: &BytesStart, out: &mut ParsedLibrary, node_stack: &[NodeFrame]) -> NodeFrame {
    let node_type = attr(e, b"Type").unwrap_or_default();
    let name = attr(e, b"Name").unwrap_or_default();
    let parent = node_stack.iter().rev().find_map(|f| f.crate_index);
    let mut frame = NodeFrame {
        crate_index: None,
        is_playlist: false,
        keys_are_ids: false,
    };
    match node_type.as_str() {
        // Folder. The top-level `ROOT` is transparent (no row); children attach
        // to the nearest real ancestor.
        "0" if name != "ROOT" => {
            out.playlists.push(ParsedPlaylist {
                name,
                parent,
                track_ids: Vec::new(),
            });
            frame.crate_index = Some(out.playlists.len() - 1);
        }
        "1" => {
            out.playlists.push(ParsedPlaylist {
                name,
                parent,
                track_ids: Vec::new(),
            });
            frame.crate_index = Some(out.playlists.len() - 1);
            frame.is_playlist = true;
            // `KeyType="0"` (default) ⇒ membership keys are TrackIDs; "1" ⇒
            // they're file paths, which we don't resolve (import empty).
            frame.keys_are_ids = attr(e, b"KeyType").is_none_or(|k| k == "0");
        }
        _ => {}
    }
    frame
}

/// Add one `<TRACK Key=…/>` membership ref to the current playlist, if any and
/// if it is id-keyed. A non-numeric / location key is silently dropped.
fn add_member(e: &BytesStart, out: &mut ParsedLibrary, node_stack: &[NodeFrame]) {
    let Some(frame) = node_stack.last() else {
        return;
    };
    if !frame.is_playlist || !frame.keys_are_ids {
        return;
    }
    let Some(idx) = frame.crate_index else { return };
    if let Some(id) = attr(e, b"Key").and_then(|k| k.parse::<i64>().ok()) {
        out.playlists[idx].track_ids.push(id);
    }
}

/// Decode a rekordbox `Location` (`file://localhost/Users/…`, percent-encoded)
/// into an absolute filesystem path. `None` for non-`file://` locations.
fn decode_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest);
    decoded.starts_with('/').then_some(decoded)
}

/// Minimal percent-decoder (`%XX` → byte), UTF-8 lossy.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full export with both a self-closing track (no grid) and a container
    /// track (grid + cues + loop), plus a nested folder/playlist tree.
    const SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.14" Company="AlphaTheta"/>
  <COLLECTION Entries="2">
    <TRACK TrackID="100" Name="Oneshot" Artist="" Genre="" Kind="WAV File"
           TotalTime="5" AverageBpm="0.00"
           Location="file://localhost/Users/dj/Music/one%20shot.wav" Tonality=""/>
    <TRACK TrackID="200" Name="Banger" Artist="DJ Test" Album="LP" Genre="House"
           Composer="Writer" Comments="hi" Year="2020" TrackNumber="3"
           TotalTime="180" AverageBpm="128.00"
           Location="file://localhost/Users/dj/Music/banger.mp3" Tonality="8B">
      <TEMPO Inizio="0.025" Bpm="128.00" Metro="4/4" Battito="3"/>
      <TEMPO Inizio="48.026" Bpm="128.00" Metro="4/4" Battito="1"/>
      <POSITION_MARK Name="Intro" Type="0" Start="0.025" Num="-1"/>
      <POSITION_MARK Name="Drop" Type="0" Start="32.0" Num="1" Red="40" Green="226" Blue="20"/>
      <POSITION_MARK Name="Roll" Type="4" Start="64.0" End="68.0" Num="0"/>
      <POSITION_MARK Name="FadeOut" Type="2" Start="170.0" Num="-1"/>
    </TRACK>
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Type="0" Name="Sets" Count="1">
        <NODE Name="Friday" Type="1" KeyType="0" Entries="2">
          <TRACK Key="100"/>
          <TRACK Key="200"/>
        </NODE>
      </NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>"#;

    #[test]
    fn parses_tracks_grid_cues_loops() {
        let lib = parse_xml(SAMPLE).unwrap();
        assert_eq!(lib.tracks.len(), 2);

        let one = &lib.tracks[0];
        assert_eq!(one.track_id, 100);
        assert_eq!(one.title.as_deref(), Some("Oneshot"));
        assert_eq!(one.artist, None); // empty Artist dropped
        assert_eq!(one.bpm, None); // AverageBpm 0.00 dropped
        assert_eq!(one.key, None); // empty Tonality dropped
        assert!((one.duration_secs.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(
            one.path.as_deref(),
            Some(std::path::Path::new("/Users/dj/Music/one shot.wav"))
        );
        assert!(one.grid_anchor_secs.is_none());
        assert!(one.cues.is_empty() && one.loops.is_empty());

        let two = &lib.tracks[1];
        assert_eq!(two.track_id, 200);
        assert_eq!(two.artist.as_deref(), Some("DJ Test"));
        assert_eq!(two.album.as_deref(), Some("LP"));
        assert_eq!(two.genre.as_deref(), Some("House"));
        assert_eq!(two.composer.as_deref(), Some("Writer"));
        assert_eq!(two.comment.as_deref(), Some("hi"));
        assert_eq!(two.year, Some(2020));
        assert_eq!(two.track_number, Some(3));
        assert!((two.bpm.unwrap() - 128.0).abs() < 1e-9);
        assert_eq!(two.key.as_deref(), Some("8B"));
        assert!((two.duration_secs.unwrap() - 180.0).abs() < 1e-9);
        // First TEMPO is the anchor; Battito 3 → bar phase 2.
        assert!((two.grid_anchor_secs.unwrap() - 0.025).abs() < 1e-9);
        assert!((two.grid_bpm.unwrap() - 128.0).abs() < 1e-9);
        assert_eq!(two.grid_bar_phase, 2);

        // Two cues (Intro memory + Drop hot1); the loop and the fade are not cues.
        assert_eq!(two.cues.len(), 2);
        assert_eq!(two.cues[0].hotcue, None);
        assert_eq!(two.cues[0].name.as_deref(), Some("Intro"));
        assert_eq!(two.cues[1].hotcue, Some(1));
        assert_eq!(two.cues[1].color.as_deref(), Some("#28E214"));
        assert_eq!(two.loops.len(), 1);
        assert_eq!(two.loops[0].hotcue, Some(0));
        assert!((two.loops[0].start_secs - 64.0).abs() < 1e-9);
        assert!((two.loops[0].end_secs - 68.0).abs() < 1e-9);
    }

    #[test]
    fn playlist_tree_nests_folders_and_collects_ids() {
        let lib = parse_xml(SAMPLE).unwrap();
        // ROOT is transparent: a folder ("Sets") + a playlist ("Friday").
        assert_eq!(lib.playlists.len(), 2);

        let folder = &lib.playlists[0];
        assert_eq!(folder.name, "Sets");
        assert_eq!(folder.parent, None);
        assert!(folder.track_ids.is_empty());

        let pl = &lib.playlists[1];
        assert_eq!(pl.name, "Friday");
        assert_eq!(pl.parent, Some(0));
        assert_eq!(pl.track_ids, vec![100, 200]);
    }

    #[test]
    fn empty_playlist_node_makes_a_crate_with_no_members() {
        let xml = br#"<DJ_PLAYLISTS><PLAYLISTS>
            <NODE Type="0" Name="ROOT" Count="1">
              <NODE Name="CUE Analysis Playlist" Type="1" KeyType="0" Entries="0"/>
            </NODE>
        </PLAYLISTS></DJ_PLAYLISTS>"#;
        let lib = parse_xml(xml).unwrap();
        assert_eq!(lib.playlists.len(), 1);
        assert_eq!(lib.playlists[0].name, "CUE Analysis Playlist");
        assert!(lib.playlists[0].track_ids.is_empty());
    }

    #[test]
    fn location_keyed_playlist_imports_empty() {
        // KeyType="1" members are file paths, not TrackIDs — not resolved.
        let xml = br#"<DJ_PLAYLISTS><PLAYLISTS>
            <NODE Type="0" Name="ROOT" Count="1">
              <NODE Name="ByPath" Type="1" KeyType="1" Entries="1">
                <TRACK Key="file://localhost/Users/dj/x.mp3"/>
              </NODE>
            </NODE>
        </PLAYLISTS></DJ_PLAYLISTS>"#;
        let lib = parse_xml(xml).unwrap();
        assert_eq!(lib.playlists.len(), 1);
        assert!(lib.playlists[0].track_ids.is_empty());
    }

    #[test]
    fn tracks_in_playlists_are_not_collection_tracks() {
        let xml = br#"<DJ_PLAYLISTS><PLAYLISTS>
            <NODE Type="0" Name="ROOT" Count="1">
              <NODE Name="P" Type="1" KeyType="0" Entries="1"><TRACK Key="1"/></NODE>
            </NODE>
        </PLAYLISTS></DJ_PLAYLISTS>"#;
        assert_eq!(parse_xml(xml).unwrap().tracks.len(), 0);
    }

    #[test]
    fn malformed_input_never_panics() {
        for bad in [
            &b""[..],
            b"not xml at all",
            b"<DJ_PLAYLISTS><COLLECTION><TRACK TrackID=",
            b"<DJ_PLAYLISTS><COLLECTION><TRACK></WRONG></DJ_PLAYLISTS>",
            &[0xff, 0xfe, 0x00, 0x01][..],
        ] {
            let _ = parse_xml(bad); // Ok or Err both fine — must not panic.
        }
    }

    /// Opt-in real-file validation. `DUB_REKORDBOX_XML=… cargo test …
    /// validate_against_real_xml -- --ignored --nocapture`; skips cleanly when
    /// unset (CI-safe — never a hard dep on a personal file).
    #[test]
    #[ignore = "set DUB_REKORDBOX_XML to a real rekordbox.xml"]
    fn validate_against_real_xml() {
        let Ok(path) = std::env::var("DUB_REKORDBOX_XML") else {
            eprintln!("DUB_REKORDBOX_XML unset — skipping real-file validation");
            return;
        };
        let data = std::fs::read(&path).expect("read DUB_REKORDBOX_XML");
        let lib = parse_xml(&data).expect("real rekordbox.xml must parse");
        eprintln!("parsed {} tracks from {path}", lib.tracks.len());
        for t in lib.tracks.iter().take(5) {
            eprintln!(
                "  id={} {:?} | {:?} bpm | grid {:?}s phase {} | key {:?} | {} cues, {} loops | {:?}",
                t.track_id, t.title, t.bpm, t.grid_anchor_secs, t.grid_bar_phase,
                t.key, t.cues.len(), t.loops.len(), t.path
            );
        }
        assert!(!lib.tracks.is_empty(), "real export parsed to zero tracks");
        for t in &lib.tracks {
            assert!(t.path.is_some(), "track {:?} has no path", t.title);
            if let Some(g) = t.grid_anchor_secs {
                assert!(g >= 0.0, "negative grid anchor {g}s");
            }
        }
        eprintln!("parsed {} playlist/folder nodes", lib.playlists.len());
        for p in lib.playlists.iter().take(10) {
            let parent = p.parent.and_then(|i| lib.playlists.get(i)).map(|f| &f.name);
            eprintln!(
                "  {:?} (parent {parent:?}) — {} tracks",
                p.name,
                p.track_ids.len()
            );
        }
    }
}
