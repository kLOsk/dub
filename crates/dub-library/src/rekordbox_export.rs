//! rekordbox XML **writer** — the inverse of [`crate::rekordbox`] (M11f).
//!
//! This is the load-bearing anti-lock-in commitment (PRD §8.6). The
//! rekordbox XML is the de-facto DJ interchange format — Serato,
//! Traktor, rekordbox and Lexicon all read it — so a competent exporter
//! makes a Dub library portable to anything. Making leaving easy is the
//! point: a DJ trusts a tool that does not try to trap them.
//!
//! **It writes [`ParsedLibrary`], the same type the reader produces.**
//! That is deliberate and is what makes the round-trip testable without
//! a database: build a library, write it, read it back, assert equality.
//! Every field the importer understands is a field the exporter emits;
//! if the two ever drift, the round-trip test fails rather than a user
//! silently losing their cues.
//!
//! **Precision.** Seconds are written to 3 decimals and BPM to 2, which
//! is rekordbox's own convention and what other apps expect to parse. A
//! cue is therefore preserved to the millisecond rather than to the
//! last bit of an `f64` — inaudible, but it means "lossless" here means
//! "lossless at the format's precision", not bit-exact. The
//! quantisation is pinned by a test so nobody discovers it by surprise.
//!
//! **Colour** round-trips at *token* fidelity. Import maps any hue onto
//! one of eight palette tokens, so the original hex is already gone by
//! the time we could export it; what is guaranteed is that a token
//! survives the trip unchanged (`color_label::rgb_from_token`).

use std::io::Write;

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::Writer;

use crate::color_label::rgb_from_token;
use crate::rekordbox::{ParsedCue, ParsedLibrary, ParsedLoop, ParsedPlaylist, ParsedTrack};

/// Why an export could not be written.
#[derive(Debug)]
pub struct ExportError(String);

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rekordbox export failed: {}", self.0)
    }
}

impl std::error::Error for ExportError {}

impl From<quick_xml::Error> for ExportError {
    fn from(e: quick_xml::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

/// Serialise a library to rekordbox XML.
///
/// # Errors
///
/// [`ExportError`] if the underlying writer fails.
pub fn write_xml<W: Write>(library: &ParsedLibrary, sink: W) -> Result<(), ExportError> {
    let mut w = Writer::new_with_indent(sink, b' ', 2);
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

    let mut root = BytesStart::new("DJ_PLAYLISTS");
    root.push_attribute(("Version", "1.0.0"));
    w.write_event(Event::Start(root))?;

    // Identifying the producer is convention, and it tells anyone
    // debugging an import on the far side where the file came from.
    let mut product = BytesStart::new("PRODUCT");
    product.push_attribute(("Name", "Dub"));
    product.push_attribute(("Version", env!("CARGO_PKG_VERSION")));
    product.push_attribute(("Company", "Dub"));
    w.write_event(Event::Empty(product))?;

    write_collection(&mut w, &library.tracks)?;
    write_playlists(&mut w, &library.playlists)?;

    w.write_event(Event::End(BytesEnd::new("DJ_PLAYLISTS")))?;
    Ok(())
}

/// Serialise to a `String`.
///
/// # Errors
///
/// [`ExportError`] if serialisation fails or the result is not UTF-8.
pub fn to_string(library: &ParsedLibrary) -> Result<String, ExportError> {
    let mut buf: Vec<u8> = Vec::new();
    write_xml(library, &mut buf)?;
    String::from_utf8(buf).map_err(|e| ExportError(e.to_string()))
}

fn write_collection<W: Write>(
    w: &mut Writer<W>,
    tracks: &[ParsedTrack],
) -> Result<(), ExportError> {
    let mut collection = BytesStart::new("COLLECTION");
    collection.push_attribute(("Entries", tracks.len().to_string().as_str()));
    w.write_event(Event::Start(collection))?;
    for track in tracks {
        write_track(w, track)?;
    }
    w.write_event(Event::End(BytesEnd::new("COLLECTION")))?;
    Ok(())
}

fn write_track<W: Write>(w: &mut Writer<W>, t: &ParsedTrack) -> Result<(), ExportError> {
    let mut el = BytesStart::new("TRACK");
    el.push_attribute(("TrackID", t.track_id.to_string().as_str()));

    // Text fields. Absent means absent: writing an empty attribute
    // would come back as Some("") and turn "unknown artist" into a
    // track credited to nobody.
    for (key, value) in [
        ("Name", t.title.as_deref()),
        ("Artist", t.artist.as_deref()),
        ("Composer", t.composer.as_deref()),
        ("Album", t.album.as_deref()),
        ("Genre", t.genre.as_deref()),
        ("Comments", t.comment.as_deref()),
        ("Tonality", t.key.as_deref()),
    ] {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            el.push_attribute((key, v));
        }
    }

    if let Some(bpm) = t.bpm {
        el.push_attribute(("AverageBpm", fmt_bpm(bpm).as_str()));
    }
    if let Some(secs) = t.duration_secs {
        // TotalTime is integer seconds in this format — coarser than
        // iTunes' milliseconds, and not our choice.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = secs.max(0.0).round() as u64;
        el.push_attribute(("TotalTime", whole.to_string().as_str()));
    }
    if let Some(year) = t.year {
        el.push_attribute(("Year", year.to_string().as_str()));
    }
    if let Some(n) = t.track_number {
        el.push_attribute(("TrackNumber", n.to_string().as_str()));
    }
    if let Some(stars) = t.rating {
        // rekordbox stores 0-255 in steps of 51.
        el.push_attribute(("Rating", (stars.clamp(0, 5) * 51).to_string().as_str()));
    }
    if let Some((r, g, b)) = t.color.as_deref().and_then(rgb_from_token) {
        el.push_attribute(("Colour", format!("0x{r:02X}{g:02X}{b:02X}").as_str()));
    }
    if let Some(path) = t.path.as_deref() {
        el.push_attribute((
            "Location",
            encode_file_url(&path.to_string_lossy()).as_str(),
        ));
    }

    let has_children = t.grid_bpm.is_some() || !t.cues.is_empty() || !t.loops.is_empty();
    if !has_children {
        w.write_event(Event::Empty(el))?;
        return Ok(());
    }

    w.write_event(Event::Start(el))?;
    if let (Some(bpm), Some(inizio)) = (t.grid_bpm, t.grid_anchor_secs) {
        let mut tempo = BytesStart::new("TEMPO");
        tempo.push_attribute(("Inizio", fmt_secs(inizio).as_str()));
        tempo.push_attribute(("Bpm", fmt_bpm(bpm).as_str()));
        tempo.push_attribute(("Metro", "4/4"));
        // Battito is 1-based; our bar phase is 0-based.
        tempo.push_attribute((
            "Battito",
            (t.grid_bar_phase.min(3) + 1).to_string().as_str(),
        ));
        w.write_event(Event::Empty(tempo))?;
    }
    for cue in &t.cues {
        w.write_event(Event::Empty(cue_element(cue)))?;
    }
    for lp in &t.loops {
        w.write_event(Event::Empty(loop_element(lp)))?;
    }
    w.write_event(Event::End(BytesEnd::new("TRACK")))?;
    Ok(())
}

fn cue_element(cue: &ParsedCue) -> BytesStart<'static> {
    let mut el = BytesStart::new("POSITION_MARK");
    el.push_attribute(("Name", cue.name.as_deref().unwrap_or("")));
    el.push_attribute(("Type", "0"));
    el.push_attribute(("Start", fmt_secs(cue.position_secs).as_str()));
    push_num(&mut el, cue.hotcue);
    push_color(&mut el, cue.color.as_deref());
    el
}

fn loop_element(lp: &ParsedLoop) -> BytesStart<'static> {
    let mut el = BytesStart::new("POSITION_MARK");
    el.push_attribute(("Name", lp.name.as_deref().unwrap_or("")));
    el.push_attribute(("Type", "4"));
    el.push_attribute(("Start", fmt_secs(lp.start_secs).as_str()));
    el.push_attribute(("End", fmt_secs(lp.end_secs).as_str()));
    push_num(&mut el, lp.hotcue);
    push_color(&mut el, lp.color.as_deref());
    el
}

/// `Num` is the hot-cue pad slot; `-1` marks an unindexed memory cue.
fn push_num(el: &mut BytesStart<'static>, hotcue: Option<u8>) {
    let num = hotcue.map_or(-1_i32, i32::from);
    el.push_attribute(("Num", num.to_string().as_str()));
}

/// `#RRGGBB` back out to the separate 0-255 attributes.
fn push_color(el: &mut BytesStart<'static>, color: Option<&str>) {
    let Some((r, g, b)) = color.and_then(parse_hex_rgb) else {
        return;
    };
    el.push_attribute(("Red", r.to_string().as_str()));
    el.push_attribute(("Green", g.to_string().as_str()));
    el.push_attribute(("Blue", b.to_string().as_str()));
}

fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if h.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ))
}

/// The `<PLAYLISTS>` tree.
///
/// [`ParsedPlaylist`] is a flat list with `parent` indices, in document
/// order, so the tree is rebuilt by walking children of each node. The
/// transparent `ROOT` folder the reader discards is written back,
/// because rekordbox expects it.
fn write_playlists<W: Write>(
    w: &mut Writer<W>,
    playlists: &[ParsedPlaylist],
) -> Result<(), ExportError> {
    w.write_event(Event::Start(BytesStart::new("PLAYLISTS")))?;

    let roots: Vec<usize> = (0..playlists.len())
        .filter(|&i| playlists[i].parent.is_none())
        .collect();
    let mut root = BytesStart::new("NODE");
    root.push_attribute(("Type", "0"));
    root.push_attribute(("Name", "ROOT"));
    root.push_attribute(("Count", roots.len().to_string().as_str()));
    w.write_event(Event::Start(root))?;
    for i in roots {
        write_node(w, playlists, i)?;
    }
    w.write_event(Event::End(BytesEnd::new("NODE")))?;

    w.write_event(Event::End(BytesEnd::new("PLAYLISTS")))?;
    Ok(())
}

fn write_node<W: Write>(
    w: &mut Writer<W>,
    playlists: &[ParsedPlaylist],
    index: usize,
) -> Result<(), ExportError> {
    let node = &playlists[index];
    let children: Vec<usize> = (0..playlists.len())
        .filter(|&i| playlists[i].parent == Some(index))
        .collect();
    // A node with children is a folder; anything else is a playlist,
    // including an empty one — an empty *folder* and an empty playlist
    // are indistinguishable here, and a playlist is the safer reading
    // because it is what a user's crate actually is.
    let is_folder = !children.is_empty();

    let mut el = BytesStart::new("NODE");
    el.push_attribute(("Name", node.name.as_str()));
    if is_folder {
        el.push_attribute(("Type", "0"));
        el.push_attribute(("Count", children.len().to_string().as_str()));
        w.write_event(Event::Start(el))?;
        for child in children {
            write_node(w, playlists, child)?;
        }
    } else {
        el.push_attribute(("Type", "1"));
        el.push_attribute(("KeyType", "0"));
        el.push_attribute(("Entries", node.track_ids.len().to_string().as_str()));
        w.write_event(Event::Start(el))?;
        for id in &node.track_ids {
            let mut member = BytesStart::new("TRACK");
            member.push_attribute(("Key", id.to_string().as_str()));
            w.write_event(Event::Empty(member))?;
        }
    }
    w.write_event(Event::End(BytesEnd::new("NODE")))?;
    Ok(())
}

/// Absolute path to the `file://localhost/…` URL the format uses.
///
/// Percent-encodes everything outside the unreserved set, leaving `/`
/// as the separator. The inverse of the reader's `decode_file_url`.
fn encode_file_url(path: &str) -> String {
    let mut out = String::from("file://localhost");
    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Seconds at the format's millisecond precision.
fn fmt_secs(v: f64) -> String {
    format!("{:.3}", if v.is_finite() { v } else { 0.0 })
}

/// BPM at the format's two decimals.
fn fmt_bpm(v: f64) -> String {
    format!("{:.2}", if v.is_finite() { v } else { 0.0 })
}

// --- adapter: the live library to the wire shape -----------------------

use crate::cues::{StoredCue, StoredLoop};
use crate::db::TrackRow;
use crate::error::Result as LibResult;
use crate::Library;

/// Export one Dub crate: its tracks, and a single playlist node holding
/// them in crate order.
///
/// # Errors
///
/// Propagates any library read failure.
pub fn crate_to_parsed(library: &Library, crate_id: i64, name: &str) -> LibResult<ParsedLibrary> {
    let rows = library.list_crate_tracks(crate_id)?;
    let tracks = rows_to_tracks(library, &rows)?;
    Ok(ParsedLibrary {
        playlists: vec![ParsedPlaylist {
            name: name.to_string(),
            parent: None,
            track_ids: tracks.iter().map(|t| t.track_id).collect(),
        }],
        tracks,
    })
}

/// Export the whole collection, with one playlist node per Dub crate.
///
/// # Errors
///
/// Propagates any library read failure.
pub fn collection_to_parsed(library: &Library) -> LibResult<ParsedLibrary> {
    // Not `list_tracks`: that is the *curated* collection and would
    // silently drop every browse-only row an external-source scan
    // minted, which is the one failure this feature exists to prevent.
    const MAX: u32 = 100_000;
    let rows = library.list_tracks_for_export(MAX, 0)?;
    let tracks = rows_to_tracks(library, &rows)?;

    // TrackID is positional, so map uuid -> id once for membership.
    let ids: std::collections::HashMap<&str, i64> = rows
        .iter()
        .zip(&tracks)
        .map(|(row, t)| (row.id.as_str(), t.track_id))
        .collect();

    let mut playlists = Vec::new();
    for c in library.list_crates()? {
        let members = library.list_crate_tracks(c.id)?;
        playlists.push(ParsedPlaylist {
            name: c.name.clone(),
            parent: None,
            track_ids: members
                .iter()
                .filter_map(|m| ids.get(m.id.as_str()).copied())
                .collect(),
        });
    }
    Ok(ParsedLibrary { tracks, playlists })
}

/// `TrackID` is assigned positionally, 1-based. rekordbox only needs it
/// to be unique within the document and stable enough for the playlist
/// nodes to reference, and Dub's own ids are UUIDs which the format
/// cannot carry.
fn rows_to_tracks(library: &Library, rows: &[TrackRow]) -> LibResult<Vec<ParsedTrack>> {
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let grid = library.active_beatgrid_for_track(&row.id)?;
        let cues = library.all_cues(&row.id)?;
        let loops = library.all_loops(&row.id)?;
        let source = preferred_source(&cues, &loops);

        out.push(ParsedTrack {
            track_id: i64::try_from(i + 1).unwrap_or(i64::MAX),
            path: library.track_path(&row.id)?,
            artist: row.artist.clone(),
            title: row.title.clone(),
            album: row.album.clone(),
            genre: row.genre.clone(),
            composer: None,
            comment: row.comment.clone(),
            year: row.year,
            track_number: None,
            rating: row.rating,
            color: row.color.clone(),
            bpm: row.bpm,
            // Camelot, which is what `track_keys` keeps active and what
            // rekordbox's Tonality accepts. The per-source original
            // notation is still in the schema if exact source fidelity
            // is ever wanted over a normalised key.
            key: row.key.clone(),
            grid_anchor_secs: grid.as_ref().map(|g| g.anchor_secs),
            grid_bpm: grid.as_ref().map(|g| g.bpm),
            grid_bar_phase: grid.as_ref().map_or(0, |g| g.bar_phase),
            // 0 means "never decoded", not "zero length".
            duration_secs: (row.duration_ms > 0).then(|| f64::from(row.duration_ms) / 1000.0),
            cues: cues
                .iter()
                .filter(|c| source.is_none_or(|s| c.source == s))
                .filter(|c| c.kind == "hot_cue" || c.kind == "memory")
                .map(to_parsed_cue)
                .collect(),
            loops: loops
                .iter()
                .filter(|l| source.is_none_or(|s| l.source == s))
                .map(to_parsed_loop)
                .collect(),
        });
    }
    Ok(out)
}

/// One source's cues, not a merge of several.
///
/// A track can carry a Serato set and a Traktor set at once, on
/// overlapping pad indices. Emitting both would put two markers on pad
/// 1 and the receiving app would keep whichever it read last. The DJ's
/// own cues win; failing that, the source with the most to say.
fn preferred_source<'a>(cues: &'a [StoredCue], loops: &'a [StoredLoop]) -> Option<&'a str> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for c in cues {
        *counts.entry(c.source.as_str()).or_default() += 1;
    }
    for l in loops {
        *counts.entry(l.source.as_str()).or_default() += 1;
    }
    if counts.contains_key("user") {
        return Some("user");
    }
    counts
        .into_iter()
        // Deterministic: most cues, ties broken by name so the same
        // library always exports the same file.
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(source, _)| source)
}

/// Pad slots are 0-7; the importers park memory cues above them so they
/// cannot alias a pad, and that is the same line we read back out.
fn to_parsed_cue(c: &StoredCue) -> ParsedCue {
    ParsedCue {
        hotcue: (c.kind == "hot_cue" && (0..8).contains(&c.cue_index))
            .then(|| u8::try_from(c.cue_index).unwrap_or(0)),
        position_secs: c.position_secs,
        name: c.name.clone(),
        color: c.color.clone(),
    }
}

fn to_parsed_loop(l: &StoredLoop) -> ParsedLoop {
    ParsedLoop {
        hotcue: (0..8)
            .contains(&l.loop_index)
            .then(|| u8::try_from(l.loop_index).unwrap_or(0)),
        start_secs: l.in_secs,
        end_secs: l.out_secs,
        name: l.name.clone(),
        color: l.color.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rekordbox::parse_xml;
    use std::path::PathBuf;

    fn track(id: i64, title: &str) -> ParsedTrack {
        ParsedTrack {
            track_id: id,
            title: Some(title.into()),
            path: Some(PathBuf::from(format!("/Users/dj/{title}.flac"))),
            ..ParsedTrack::default()
        }
    }

    /// The acceptance criterion for M11f: what we write, we can read.
    fn round_trip(lib: &ParsedLibrary) -> ParsedLibrary {
        let xml = to_string(lib).expect("write");
        parse_xml(xml.as_bytes()).expect("read back")
    }

    #[test]
    fn a_bare_collection_round_trips() {
        let lib = ParsedLibrary {
            tracks: vec![track(1, "Real Rock"), track(2, "Version")],
            playlists: Vec::new(),
        };
        assert_eq!(round_trip(&lib), lib);
    }

    #[test]
    fn every_metadata_field_round_trips() {
        let lib = ParsedLibrary {
            tracks: vec![ParsedTrack {
                track_id: 7,
                path: Some(PathBuf::from("/Users/dj/Music/Studio One.flac")),
                artist: Some("Sound Dimension".into()),
                title: Some("Real Rock".into()),
                album: Some("Studio One Side A".into()),
                genre: Some("Reggae".into()),
                composer: Some("Jackie Mittoo".into()),
                comment: Some("ripped from vinyl".into()),
                year: Some(1967),
                track_number: Some(3),
                rating: Some(4),
                color: Some("orange".into()),
                bpm: Some(88.5),
                key: Some("8B".into()),
                duration_secs: Some(212.0),
                grid_anchor_secs: None,
                grid_bpm: None,
                grid_bar_phase: 0,
                cues: Vec::new(),
                loops: Vec::new(),
            }],
            playlists: Vec::new(),
        };
        assert_eq!(round_trip(&lib), lib);
    }

    /// The whole reason the schema keeps cues and loops from day one
    /// (PRD §8.6): a DJ who imports a Serato library and exports to
    /// rekordbox gets their hot cues back on the other side.
    #[test]
    fn grid_cues_and_loops_round_trip() {
        let mut t = track(1, "Chase the Devil");
        // A gridded track always carries AverageBpm too; see
        // `a_grid_without_average_bpm_is_not_a_fixpoint` for why this
        // is set rather than left to the reader's fallback.
        t.bpm = Some(74.5);
        t.grid_bpm = Some(74.5);
        t.grid_anchor_secs = Some(0.025);
        t.grid_bar_phase = 2;
        t.cues = vec![
            ParsedCue {
                hotcue: Some(0),
                position_secs: 12.5,
                name: Some("intro".into()),
                color: Some("#FF0000".into()),
            },
            ParsedCue {
                hotcue: None,
                position_secs: 60.25,
                name: None,
                color: None,
            },
        ];
        t.loops = vec![ParsedLoop {
            hotcue: Some(3),
            start_secs: 32.0,
            end_secs: 40.0,
            name: Some("dub".into()),
            color: Some("#00FF00".into()),
        }];
        let lib = ParsedLibrary {
            tracks: vec![t],
            playlists: Vec::new(),
        };
        assert_eq!(round_trip(&lib), lib);
    }

    /// The one place a round trip is deliberately *not* a fixpoint.
    ///
    /// The reader treats the first `<TEMPO>`'s Bpm as a fallback for a
    /// missing `AverageBpm`, so a track with a grid and no metadata
    /// tempo comes back carrying one. That is the reader being useful
    /// rather than the writer losing something — but it is a real
    /// asymmetry and belongs in a test rather than in a surprise.
    #[test]
    fn a_grid_without_average_bpm_is_not_a_fixpoint() {
        let mut t = track(1, "gridded");
        t.bpm = None;
        t.grid_bpm = Some(93.0);
        t.grid_anchor_secs = Some(1.0);
        let back = round_trip(&ParsedLibrary {
            tracks: vec![t],
            playlists: Vec::new(),
        });
        assert_eq!(
            back.tracks[0].bpm,
            Some(93.0),
            "the reader fills AverageBpm from the grid tempo"
        );
    }

    #[test]
    fn the_playlist_tree_round_trips() {
        let lib = ParsedLibrary {
            tracks: vec![track(1, "A"), track(2, "B")],
            playlists: vec![
                ParsedPlaylist {
                    name: "Reggae".into(),
                    parent: None,
                    track_ids: Vec::new(),
                },
                ParsedPlaylist {
                    name: "Roots".into(),
                    parent: Some(0),
                    track_ids: vec![1, 2],
                },
                ParsedPlaylist {
                    name: "Loose".into(),
                    parent: None,
                    track_ids: vec![2],
                },
            ],
        };
        assert_eq!(round_trip(&lib), lib);
    }

    /// Titles carrying XML metacharacters must not corrupt the document
    /// — the single most likely way an exporter breaks in the field.
    #[test]
    fn xml_metacharacters_in_metadata_survive() {
        let mut t = track(1, "quote");
        t.title = Some(r#"Rock & Roll <"Dub" Version>"#.into());
        t.artist = Some("A & B".into());
        t.path = Some(PathBuf::from("/Users/dj/a & b/c'd \"e\".flac"));
        let lib = ParsedLibrary {
            tracks: vec![t],
            playlists: Vec::new(),
        };
        assert_eq!(round_trip(&lib), lib);
    }

    /// Non-ASCII paths are ordinary in a record collection.
    #[test]
    fn unicode_paths_round_trip() {
        let mut t = track(1, "unicode");
        t.path = Some(PathBuf::from("/Users/dj/Café/ソウル･トレイン.flac"));
        t.title = Some("ソウル･トレインのテーマ".into());
        let lib = ParsedLibrary {
            tracks: vec![t],
            playlists: Vec::new(),
        };
        assert_eq!(round_trip(&lib), lib);
    }

    /// An empty title is not the same as no title, and writing one as
    /// an empty attribute would turn "unknown" into "named nothing".
    #[test]
    fn absent_fields_stay_absent() {
        let lib = ParsedLibrary {
            tracks: vec![ParsedTrack {
                track_id: 1,
                path: Some(PathBuf::from("/a.flac")),
                ..ParsedTrack::default()
            }],
            playlists: Vec::new(),
        };
        let back = round_trip(&lib);
        assert_eq!(back.tracks[0].title, None);
        assert_eq!(back.tracks[0].artist, None);
        assert_eq!(back.tracks[0].bpm, None);
        assert_eq!(back, lib);
    }

    /// Documents the format's precision rather than pretending it is
    /// bit-exact: seconds quantise to a millisecond, BPM to 1/100.
    #[test]
    fn precision_is_the_formats_not_the_f64s() {
        let mut t = track(1, "precision");
        t.bpm = Some(128.567_89);
        t.grid_bpm = Some(128.567_89);
        t.grid_anchor_secs = Some(0.123_456_7);
        t.cues = vec![ParsedCue {
            hotcue: Some(1),
            position_secs: 10.987_654_3,
            name: None,
            color: None,
        }];
        let back = round_trip(&ParsedLibrary {
            tracks: vec![t],
            playlists: Vec::new(),
        });
        assert!((back.tracks[0].bpm.unwrap() - 128.57).abs() < 1e-9);
        assert!((back.tracks[0].grid_anchor_secs.unwrap() - 0.123).abs() < 1e-9);
        assert!((back.tracks[0].cues[0].position_secs - 10.988).abs() < 1e-9);
    }

    #[test]
    fn the_document_declares_itself_as_dj_playlists() {
        let xml = to_string(&ParsedLibrary::default()).unwrap();
        assert!(
            xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
            "{xml}"
        );
        assert!(xml.contains(r#"<DJ_PLAYLISTS Version="1.0.0">"#), "{xml}");
        assert!(
            xml.contains(r#"Name="Dub""#),
            "the producer should be named: {xml}"
        );
    }
}
