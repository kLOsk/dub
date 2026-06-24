//! ID3v2 `GEOB` frame extraction (M11e) — the I/O boundary for Serato's
//! in-file tags. The pure decoders (`beatgrid`, `markers2`, `autotags`) work
//! on the bytes this returns.
//!
//! Serato stores its per-track data as `GEOB` (General Encapsulated Object)
//! frames keyed by a description string (`Serato BeatGrid`, `Serato Markers2`,
//! `Serato Autotags`, …). We use the `id3` crate to read the tag and pull the
//! named object's raw bytes. MP3 / AIFF / WAV are supported (the dominant
//! scratch-DJ formats); MP4 and FLAC keep Serato data in different containers
//! and are deferred — those return `None` (a logged skip in the adapter), not
//! an error.

use std::path::Path;

/// GEOB description for the beat grid.
pub const BEATGRID: &str = "Serato BeatGrid";
/// GEOB description for the cues / loops (modern format).
pub const MARKERS2: &str = "Serato Markers2";
/// GEOB description for the BPM / gain auto-tags.
pub const AUTOTAGS: &str = "Serato Autotags";

/// Read every Serato GEOB blob from `path` in one tag pass, returned as
/// `(description, bytes)`. Empty when the file carries no ID3 tag, is an
/// unsupported container, or has no GEOB frames. Never panics.
pub fn read_serato_geobs(path: &Path) -> Vec<(String, Vec<u8>)> {
    let Some(tag) = read_tag(path) else {
        return Vec::new();
    };
    tag.encapsulated_objects()
        .filter(|obj| obj.description.starts_with("Serato"))
        .map(|obj| (obj.description.clone(), obj.data.clone()))
        .collect()
}

/// Pick one GEOB blob by description from an already-read list.
pub fn find<'a>(geobs: &'a [(String, Vec<u8>)], description: &str) -> Option<&'a [u8]> {
    geobs
        .iter()
        .find(|(d, _)| d == description)
        .map(|(_, data)| data.as_slice())
}

fn read_tag(path: &Path) -> Option<id3::Tag> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        // `read_from_path` auto-detects the MP3 / AIFF / WAV ID3 container.
        "mp3" | "aif" | "aiff" | "wav" => id3::Tag::read_from_path(path).ok(),
        // MP4 (`----:com.serato.dj:*` atoms) and FLAC (Vorbis comments)
        // hold Serato data in non-ID3 containers — deferred.
        _ => None,
    }
}
