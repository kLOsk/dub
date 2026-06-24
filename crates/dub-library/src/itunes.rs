//! iTunes / Apple Music `Library.xml` parser (M12c).
//!
//! The file is an Apple **plist** (XML flavour): a top `<dict>` whose `Tracks`
//! key holds a dict of `trackID → <dict>` and whose `Playlists` key holds an
//! array of playlist `<dict>`s. We stream it with `quick-xml` (the Tracks dict
//! is processed entry-by-entry, never held whole) and pull only the fields Dub
//! maps. iTunes has **no beat grids or cues** — just metadata + playlists.
//!
//! Plist values pair as `<key>NAME</key>` followed by a value element
//! (`<string>`, `<integer>`, `<date>`, `<true/>`, `<dict>`, `<array>`, …).
//! Pure + panic-free: a framing error ends the parse with whatever was read.

use quick_xml::events::Event;
use quick_xml::Reader;

/// One track from the `Tracks` dict.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ItunesTrack {
    /// iTunes numeric `Track ID` (referenced by playlist items).
    pub track_id: i64,
    /// `Name`.
    pub name: Option<String>,
    /// `Artist`.
    pub artist: Option<String>,
    /// `Album`.
    pub album: Option<String>,
    /// `Composer`.
    pub composer: Option<String>,
    /// `Genre`.
    pub genre: Option<String>,
    /// `BPM` (iTunes stores an integer).
    pub bpm: Option<f64>,
    /// `Year`.
    pub year: Option<i32>,
    /// `Total Time` in milliseconds.
    pub total_time_ms: Option<i64>,
    /// Absolute filesystem path, decoded from the `Location` `file://` URL.
    pub path: Option<String>,
}

/// One playlist from the `Playlists` array.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ItunesPlaylist {
    /// `Name`.
    pub name: String,
    /// `Playlist Persistent ID`.
    pub persistent_id: Option<String>,
    /// `Parent Persistent ID` (set for playlists inside a folder).
    pub parent_persistent_id: Option<String>,
    /// `true` when this node is a folder (`Folder` = true).
    pub is_folder: bool,
    /// `true` for the special "Library" master playlist.
    pub is_master: bool,
    /// `true` for iTunes' built-in distinguished playlists (Music, Films,
    /// TV Programmes, Downloaded, Audiobooks, …) — not user-created.
    pub distinguished: bool,
    /// Member `Track ID`s, in playlist order.
    pub track_ids: Vec<i64>,
}

/// The parsed library.
#[derive(Debug, Default, PartialEq)]
pub struct ItunesLibrary {
    /// Tracks, in document order.
    pub tracks: Vec<ItunesTrack>,
    /// Playlists / folders, in document order.
    pub playlists: Vec<ItunesPlaylist>,
}

/// Parse failure (a hard XML error). Missing fields are tolerated.
#[derive(Debug)]
pub struct ParseError(String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "itunes plist parse: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse an iTunes `Library.xml` byte buffer.
///
/// # Errors
/// Returns [`ParseError`] only on an unrecoverable XML error.
pub fn parse_library(data: &[u8]) -> Result<ItunesLibrary, ParseError> {
    let mut reader = Reader::from_reader(data);
    let mut buf = Vec::new();
    let mut out = ItunesLibrary::default();

    // Walk to the top-level <dict> (inside <plist>).
    loop {
        match reader.read_event_into(&mut buf).map_err(err)? {
            Event::Eof => return Ok(out),
            Event::Start(e) if e.name().as_ref() == b"dict" => break,
            _ => {}
        }
        buf.clear();
    }

    // Iterate the top dict's key/value pairs.
    while let Some(key) = read_key(&mut reader, &mut buf)? {
        match key.as_str() {
            "Tracks" => read_tracks(&mut reader, &mut buf, &mut out)?,
            "Playlists" => read_playlists(&mut reader, &mut buf, &mut out)?,
            _ => skip_value(&mut reader, &mut buf)?,
        }
    }
    Ok(out)
}

fn err(e: quick_xml::Error) -> ParseError {
    ParseError(e.to_string())
}

/// Read the next `<key>…</key>` text within a dict, or `None` at the dict's
/// closing `</dict>`.
fn read_key(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<Option<String>, ParseError> {
    loop {
        match reader.read_event_into(buf).map_err(err)? {
            Event::Eof => return Ok(None),
            Event::End(e) if e.name().as_ref() == b"dict" => return Ok(None),
            Event::Start(e) if e.name().as_ref() == b"key" => {
                let text = read_text(reader, buf, b"key")?;
                return Ok(Some(text));
            }
            _ => {}
        }
    }
}

/// Read text content up to `</end_tag>`.
fn read_text(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    end_tag: &[u8],
) -> Result<String, ParseError> {
    let mut text = String::new();
    loop {
        match reader.read_event_into(buf).map_err(err)? {
            Event::Eof => break,
            Event::Text(t) => text.push_str(&t.unescape().map_err(err)?),
            Event::End(e) if e.name().as_ref() == end_tag => break,
            _ => {}
        }
    }
    Ok(text)
}

/// The next value element after a `<key>`: a scalar's text, a container
/// marker, a boolean, or nothing. Containers are then consumed by the
/// dedicated readers / `skip_container`.
enum Value {
    Scalar(String),
    Dict,
    Array,
    /// `<true/>` / `<false/>`.
    Bool(bool),
    None,
}

fn read_value(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<Value, ParseError> {
    loop {
        match reader.read_event_into(buf).map_err(err)? {
            Event::Eof => return Ok(Value::None),
            Event::Empty(e) => {
                return Ok(match e.name().as_ref() {
                    b"true" => Value::Bool(true),
                    b"false" => Value::Bool(false),
                    _ => Value::Scalar(String::new()),
                });
            }
            Event::Start(e) => {
                let name = e.name().as_ref().to_vec();
                return match name.as_slice() {
                    b"dict" => Ok(Value::Dict),
                    b"array" => Ok(Value::Array),
                    _ => {
                        let text = read_text(reader, buf, &name)?;
                        Ok(Value::Scalar(text))
                    }
                };
            }
            // Skip stray whitespace text between key and value.
            _ => {}
        }
    }
}

/// Consume one value (scalar or container) fully and discard it.
fn skip_value(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<(), ParseError> {
    match read_value(reader, buf)? {
        Value::Dict => skip_container(reader, buf, b"dict"),
        Value::Array => skip_container(reader, buf, b"array"),
        _ => Ok(()),
    }
}

/// Consume a container element to its matching close, handling nesting.
fn skip_container(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    tag: &[u8],
) -> Result<(), ParseError> {
    let mut depth = 1usize;
    loop {
        match reader.read_event_into(buf).map_err(err)? {
            Event::Eof => return Ok(()),
            Event::Start(e) if e.name().as_ref() == tag => depth += 1,
            Event::End(e) if e.name().as_ref() == tag => {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

/// Parse the `Tracks` value: a dict of `trackID → <dict>`. Each entry's key
/// is the numeric id (also present inside as `Track ID`); we parse the value
/// dict.
fn read_tracks(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    out: &mut ItunesLibrary,
) -> Result<(), ParseError> {
    // The value after the "Tracks" key must be a <dict>.
    if !matches!(read_value(reader, buf)?, Value::Dict) {
        return Ok(());
    }
    while read_key(reader, buf)?.is_some() {
        // Value is the track dict.
        match read_value(reader, buf)? {
            Value::Dict => {
                let track = parse_track_dict(reader, buf)?;
                if track.track_id != 0 || track.path.is_some() {
                    out.tracks.push(track);
                }
            }
            Value::Array => skip_container(reader, buf, b"array")?,
            _ => {}
        }
    }
    Ok(())
}

fn parse_track_dict(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
) -> Result<ItunesTrack, ParseError> {
    let mut t = ItunesTrack::default();
    while let Some(key) = read_key(reader, buf)? {
        let value = read_value(reader, buf)?;
        match value {
            Value::Scalar(text) => match key.as_str() {
                "Track ID" => t.track_id = text.parse().unwrap_or(0),
                "Name" => t.name = nonempty(text),
                "Artist" => t.artist = nonempty(text),
                "Album" => t.album = nonempty(text),
                "Composer" => t.composer = nonempty(text),
                "Genre" => t.genre = nonempty(text),
                "BPM" => t.bpm = text.parse::<f64>().ok().filter(|b| *b > 0.0),
                "Year" => t.year = text.parse().ok(),
                "Total Time" => t.total_time_ms = text.parse().ok(),
                "Location" => t.path = decode_file_url(&text),
                _ => {}
            },
            Value::Dict => skip_container(reader, buf, b"dict")?,
            Value::Array => skip_container(reader, buf, b"array")?,
            _ => {}
        }
    }
    Ok(t)
}

/// Parse the `Playlists` value: an array of playlist dicts.
fn read_playlists(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    out: &mut ItunesLibrary,
) -> Result<(), ParseError> {
    if !matches!(read_value(reader, buf)?, Value::Array) {
        return Ok(());
    }
    loop {
        match reader.read_event_into(buf).map_err(err)? {
            Event::Eof => break,
            Event::End(e) if e.name().as_ref() == b"array" => break,
            Event::Start(e) if e.name().as_ref() == b"dict" => {
                let pl = parse_playlist_dict(reader, buf)?;
                if !pl.name.is_empty() {
                    out.playlists.push(pl);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_playlist_dict(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
) -> Result<ItunesPlaylist, ParseError> {
    let mut pl = ItunesPlaylist::default();
    while let Some(key) = read_key(reader, buf)? {
        match key.as_str() {
            "Playlist Items" => {
                if matches!(read_value(reader, buf)?, Value::Array) {
                    pl.track_ids = read_playlist_items(reader, buf)?;
                }
            }
            _ => match read_value(reader, buf)? {
                Value::Scalar(text) => match key.as_str() {
                    "Name" => pl.name = text,
                    "Playlist Persistent ID" => pl.persistent_id = nonempty(text),
                    "Parent Persistent ID" => pl.parent_persistent_id = nonempty(text),
                    // Presence of "Distinguished Kind" marks a built-in
                    // (Music / Films / Downloaded / …), not a user playlist.
                    "Distinguished Kind" => pl.distinguished = true,
                    _ => {}
                },
                Value::Bool(b) => match key.as_str() {
                    "Folder" => pl.is_folder = b,
                    "Master" => pl.is_master = b,
                    _ => {}
                },
                Value::Dict => skip_container(reader, buf, b"dict")?,
                Value::Array => skip_container(reader, buf, b"array")?,
                Value::None => {}
            },
        }
    }
    Ok(pl)
}

/// Read a `Playlist Items` array: dicts each carrying a `Track ID` integer.
fn read_playlist_items(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
) -> Result<Vec<i64>, ParseError> {
    let mut ids = Vec::new();
    loop {
        match reader.read_event_into(buf).map_err(err)? {
            Event::Eof => break,
            Event::End(e) if e.name().as_ref() == b"array" => break,
            Event::Start(e) if e.name().as_ref() == b"dict" => {
                // Each item dict is { "Track ID": <integer> }.
                while let Some(k) = read_key(reader, buf)? {
                    let v = read_value(reader, buf)?;
                    if let Value::Scalar(text) = v {
                        if k == "Track ID" {
                            if let Ok(id) = text.parse::<i64>() {
                                ids.push(id);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ids)
}

fn nonempty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Decode an iTunes `Location` (`file://localhost/Users/...` or
/// `file:///Users/...`, percent-encoded) into an absolute filesystem path.
/// `None` for non-`file://` locations (remote / streaming tracks).
fn decode_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    // Drop an optional `localhost` authority; what's left starts with `/`.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest);
    if decoded.starts_with('/') {
        Some(decoded)
    } else {
        None
    }
}

/// Minimal percent-decoder (`%XX` → byte), UTF-8 lossy. Avoids pulling a URL
/// crate for one field.
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

    const SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Major Version</key><integer>1</integer>
  <key>Tracks</key>
  <dict>
    <key>2971</key>
    <dict>
      <key>Track ID</key><integer>2971</integer>
      <key>Name</key><string>Sweet Assed Child o Mine</string>
      <key>Artist</key><string>DJ Donna Summer</string>
      <key>Genre</key><string>Mashup</string>
      <key>BPM</key><integer>135</integer>
      <key>Year</key><integer>2008</integer>
      <key>Total Time</key><integer>261459</integer>
      <key>Location</key><string>file:///Users/dj/Music/Sweet%20Child.mp3</string>
    </dict>
  </dict>
  <key>Playlists</key>
  <array>
    <dict>
      <key>Name</key><string>Bangers</string>
      <key>Playlist Persistent ID</key><string>ABC123</string>
      <key>Playlist Items</key>
      <array>
        <dict><key>Track ID</key><integer>2971</integer></dict>
      </array>
    </dict>
    <dict>
      <key>Name</key><string>Folder</string>
      <key>Folder</key><true/>
    </dict>
  </array>
</dict>
</plist>"#;

    #[test]
    fn parses_tracks_and_playlists() {
        let lib = parse_library(SAMPLE).unwrap();
        assert_eq!(lib.tracks.len(), 1);
        let t = &lib.tracks[0];
        assert_eq!(t.track_id, 2971);
        assert_eq!(t.name.as_deref(), Some("Sweet Assed Child o Mine"));
        assert_eq!(t.artist.as_deref(), Some("DJ Donna Summer"));
        assert_eq!(t.genre.as_deref(), Some("Mashup"));
        assert!((t.bpm.unwrap() - 135.0).abs() < 1e-9);
        assert_eq!(t.year, Some(2008));
        assert_eq!(t.total_time_ms, Some(261_459));
        assert_eq!(t.path.as_deref(), Some("/Users/dj/Music/Sweet Child.mp3"));

        assert_eq!(lib.playlists.len(), 2);
        assert_eq!(lib.playlists[0].name, "Bangers");
        assert_eq!(lib.playlists[0].track_ids, vec![2971]);
        assert!(!lib.playlists[0].is_folder);
        assert_eq!(lib.playlists[1].name, "Folder");
        assert!(lib.playlists[1].is_folder);
    }

    #[test]
    fn file_url_decoding() {
        assert_eq!(
            decode_file_url("file:///Users/dj/a%20b.mp3").as_deref(),
            Some("/Users/dj/a b.mp3")
        );
        assert_eq!(
            decode_file_url("file://localhost/Users/dj/x.mp3").as_deref(),
            Some("/Users/dj/x.mp3")
        );
        assert_eq!(decode_file_url("http://example.com/x.mp3"), None);
    }

    #[test]
    fn malformed_never_panics() {
        for bad in [
            &b""[..],
            b"not xml",
            b"<plist><dict><key>Tracks</key><dict>",
            b"<plist><dict><key>x",
        ] {
            let _ = parse_library(bad);
        }
    }

    /// Opt-in real-file validation. `DUB_ITUNES_XML=~/Music/iTunes/iTunes\
    /// Library.xml cargo test … itunes_real -- --ignored --nocapture`.
    #[test]
    #[ignore = "set DUB_ITUNES_XML to a real iTunes Library.xml"]
    fn itunes_real() {
        let Ok(path) = std::env::var("DUB_ITUNES_XML") else {
            eprintln!("DUB_ITUNES_XML unset — skipping");
            return;
        };
        let data = std::fs::read(&path).expect("read DUB_ITUNES_XML");
        let lib = parse_library(&data).expect("parse");
        eprintln!(
            "parsed {} tracks, {} playlists",
            lib.tracks.len(),
            lib.playlists.len()
        );
        for t in lib.tracks.iter().take(5) {
            eprintln!(
                "  #{} {:?} — {:?} | {:?} BPM | {:?}",
                t.track_id, t.artist, t.name, t.bpm, t.path
            );
        }
        for p in lib.playlists.iter().take(8) {
            eprintln!(
                "  playlist {:?} (folder={}) — {} items",
                p.name,
                p.is_folder,
                p.track_ids.len()
            );
        }
        assert!(!lib.tracks.is_empty(), "real library parsed to zero tracks");
    }
}
