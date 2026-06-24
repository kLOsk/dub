//! Serato `database V2` + `Subcrates/*.crate` binary parser (M11e).
//!
//! Both files share one container format: a flat sequence of tags, each
//!
//! ```text
//! [4 bytes ASCII tag type][4 bytes big-endian u32 length][length bytes payload]
//! ```
//!
//! `database V2` is `vrsn` followed by one `otrk` per track; each `otrk`
//! payload is itself a tag sequence (`pfil` path, `tsng`/`tart`/`talb`/… text).
//! A `.crate` is `vrsn` + sort/column tags + one `otrk` per member, each
//! carrying a single `ptrk` (track path). Text payloads are **UTF-16 big-
//! endian**; paths are **relative to the volume root** (no leading slash) — the
//! `_Serato_` folder's own volume mount point is prepended by the adapter.
//!
//! Pure + panic-free on any bytes (it is a fuzz target — PRD §2.2.5):
//! truncated / garbled input stops the scan rather than indexing out of
//! bounds, and an unreadable field is simply absent from the result.

/// One track row from `database V2`. Only the fields Dub maps into
/// `track_metadata_source('serato')` are pulled out; unknown tags are
/// skipped. `file_path` is relative to the volume root.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SeratoEntry {
    /// `pfil` — file path relative to the volume root (no leading slash).
    pub file_path: Option<String>,
    /// `ttyp` — file type / extension (e.g. `mp3`).
    pub file_type: Option<String>,
    /// `tsng` — title.
    pub title: Option<String>,
    /// `tart` — artist.
    pub artist: Option<String>,
    /// `talb` — album.
    pub album: Option<String>,
    /// `tgen` — genre.
    pub genre: Option<String>,
    /// `tcmt` — comment.
    pub comment: Option<String>,
    /// `tcom` — composer.
    pub composer: Option<String>,
    /// `tgrp` — grouping.
    pub grouping: Option<String>,
    /// `tlbl` — record label / publisher.
    pub label: Option<String>,
    /// `tkey` — musical key, in whatever notation Serato stored.
    pub key: Option<String>,
    /// `tbpm` — BPM parsed from its text payload (`None` if absent / 0 /
    /// unparseable).
    pub bpm: Option<f64>,
}

/// Walk a Serato tag container. `next` yields `(tag, payload)` until the
/// bytes run out; truncation ends the walk cleanly (never panics).
struct TagReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> TagReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn next_tag(&mut self) -> Option<([u8; 4], &'a [u8])> {
        // Header is 4-byte type + 4-byte BE length.
        if self.pos + 8 > self.data.len() {
            return None;
        }
        let tag = [
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ];
        let len = u32::from_be_bytes([
            self.data[self.pos + 4],
            self.data[self.pos + 5],
            self.data[self.pos + 6],
            self.data[self.pos + 7],
        ]) as usize;
        let start = self.pos + 8;
        let end = start.checked_add(len)?;
        if end > self.data.len() {
            // Truncated final tag — stop rather than read past the buffer.
            return None;
        }
        self.pos = end;
        Some((tag, &self.data[start..end]))
    }
}

/// Decode a UTF-16BE payload, trimming trailing NULs / whitespace; `None`
/// when the result is empty (so absent and blank fields read alike).
fn text(payload: &[u8]) -> Option<String> {
    let units: Vec<u16> = payload
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    let decoded = String::from_utf16_lossy(&units);
    let trimmed = decoded.trim_matches('\0').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse a `database V2` byte buffer into its track rows, in file order.
pub fn parse_database_v2(data: &[u8]) -> Vec<SeratoEntry> {
    let mut reader = TagReader::new(data);
    let mut entries = Vec::new();
    while let Some((tag, payload)) = reader.next_tag() {
        if &tag == b"otrk" {
            entries.push(parse_track(payload));
        }
    }
    entries
}

/// Fold one `otrk` payload's nested tags into a [`SeratoEntry`].
fn parse_track(payload: &[u8]) -> SeratoEntry {
    let mut e = SeratoEntry::default();
    let mut reader = TagReader::new(payload);
    while let Some((tag, val)) = reader.next_tag() {
        match &tag {
            b"pfil" => e.file_path = text(val),
            b"ttyp" => e.file_type = text(val),
            b"tsng" => e.title = text(val),
            b"tart" => e.artist = text(val),
            b"talb" => e.album = text(val),
            b"tgen" => e.genre = text(val),
            b"tcmt" => e.comment = text(val),
            b"tcom" => e.composer = text(val),
            b"tgrp" => e.grouping = text(val),
            b"tlbl" => e.label = text(val),
            b"tkey" => e.key = text(val),
            b"tbpm" => {
                e.bpm = text(val)
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|b| *b > 0.0)
            }
            _ => {}
        }
    }
    e
}

/// Parse a `Subcrates/*.crate` byte buffer into its member track paths
/// (each relative to the volume root), in crate order.
pub fn parse_crate(data: &[u8]) -> Vec<String> {
    let mut reader = TagReader::new(data);
    let mut paths = Vec::new();
    while let Some((tag, payload)) = reader.next_tag() {
        if &tag == b"otrk" {
            let mut inner = TagReader::new(payload);
            while let Some((t, v)) = inner.next_tag() {
                if &t == b"ptrk" {
                    if let Some(p) = text(v) {
                        paths.push(p);
                    }
                }
            }
        }
    }
    paths
}

/// Split a `.crate` filename stem into its nested name components. Serato
/// encodes folder nesting in the filename with `%%` (e.g.
/// `Hip Hop%%90s` → `["Hip Hop", "90s"]`). A stem with no separator is a
/// single top-level crate.
pub fn crate_name_components(stem: &str) -> Vec<String> {
    stem.split("%%")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a Serato tag: 4-byte type + 4-byte BE length + payload.
    fn tag(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(kind);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// UTF-16BE encode (Serato's text payload encoding).
    fn u16be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn parses_a_database_track() {
        let mut otrk = Vec::new();
        otrk.extend(tag(b"ttyp", &u16be("mp3")));
        otrk.extend(tag(b"pfil", &u16be("Users/dj/Music/track.mp3")));
        otrk.extend(tag(b"tsng", &u16be("Potential Victims")));
        otrk.extend(tag(b"tart", &u16be("Westside Connection")));
        otrk.extend(tag(b"talb", &u16be("The Best Of")));
        otrk.extend(tag(b"tgen", &u16be("Hip-Hop")));
        otrk.extend(tag(b"tbpm", &u16be("93.5")));
        otrk.extend(tag(b"tkey", &u16be("Am")));

        let mut db = Vec::new();
        db.extend(tag(b"vrsn", &u16be("2.0/Serato Scratch LIVE Database")));
        db.extend(tag(b"otrk", &otrk));

        let entries = parse_database_v2(&db);
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.file_type.as_deref(), Some("mp3"));
        assert_eq!(e.file_path.as_deref(), Some("Users/dj/Music/track.mp3"));
        assert_eq!(e.title.as_deref(), Some("Potential Victims"));
        assert_eq!(e.artist.as_deref(), Some("Westside Connection"));
        assert_eq!(e.album.as_deref(), Some("The Best Of"));
        assert_eq!(e.genre.as_deref(), Some("Hip-Hop"));
        assert!((e.bpm.unwrap() - 93.5).abs() < 1e-9);
        assert_eq!(e.key.as_deref(), Some("Am"));
    }

    #[test]
    fn parses_crate_members() {
        let mut data = Vec::new();
        data.extend(tag(b"vrsn", &u16be("1.0/Serato ScratchLive Crate")));
        for p in ["Users/dj/a.mp3", "Users/dj/b.mp3"] {
            data.extend(tag(b"otrk", &tag(b"ptrk", &u16be(p))));
        }
        let paths = parse_crate(&data);
        assert_eq!(paths, vec!["Users/dj/a.mp3", "Users/dj/b.mp3"]);
    }

    #[test]
    fn crate_nesting_from_filename() {
        assert_eq!(crate_name_components("Hip Hop"), vec!["Hip Hop"]);
        assert_eq!(
            crate_name_components("Hip Hop%%90s%%East"),
            vec!["Hip Hop", "90s", "East"]
        );
    }

    #[test]
    fn malformed_never_panics() {
        for bad in [
            &b""[..],
            b"otrk",
            b"otrk\x00\x00\xff\xff",   // length overruns buffer
            b"otrk\x00\x00\x00\x04ab", // truncated payload
            &[0xff, 0xfe, 0x00, 0x01][..],
        ] {
            let _ = parse_database_v2(bad);
            let _ = parse_crate(bad);
        }
    }

    /// Opt-in real-file validation. `DUB_SERATO_DIR=~/Music/_Serato_
    /// cargo test … serato_database_real -- --ignored --nocapture`.
    #[test]
    #[ignore = "set DUB_SERATO_DIR to a real _Serato_ folder"]
    fn serato_database_real() {
        let Ok(dir) = std::env::var("DUB_SERATO_DIR") else {
            eprintln!("DUB_SERATO_DIR unset — skipping");
            return;
        };
        let path = std::path::Path::new(&dir).join("database V2");
        let data = std::fs::read(&path).expect("read database V2");
        let entries = parse_database_v2(&data);
        eprintln!("parsed {} serato tracks from {path:?}", entries.len());
        for e in entries.iter().take(10) {
            eprintln!(
                "  {:?} — {:?} / {:?} | {:?} BPM | key {:?} | {:?}",
                e.file_path, e.artist, e.title, e.bpm, e.key, e.genre
            );
        }
        assert!(!entries.is_empty(), "real database parsed to zero tracks");
        for e in &entries {
            assert!(e.file_path.is_some(), "track with no pfil path");
        }
    }
}
