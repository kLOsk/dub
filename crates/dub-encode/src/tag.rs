//! Vorbis-comment + PICTURE tagging via `metaflac`, using Picard's field
//! conventions so rips are portable to Serato / rekordbox / Picard itself.

use std::path::Path;

use metaflac::block::PictureType;
use metaflac::Tag;

use crate::error::EncodeError;

/// Tags written into the encoded FLAC as Vorbis comments (Picard conventions).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackTags {
    /// `TITLE`.
    pub title: Option<String>,
    /// `ARTIST`.
    pub artist: Option<String>,
    /// `ALBUM`.
    pub album: Option<String>,
    /// `ALBUMARTIST`.
    pub album_artist: Option<String>,
    /// `DATE` — release year.
    pub year: Option<i32>,
    /// `GENRE`.
    pub genre: Option<String>,
    /// `TRACKNUMBER`.
    pub track_number: Option<u32>,
    /// `TRACKTOTAL`.
    pub track_total: Option<u32>,
    /// `COMMENT`.
    pub comment: Option<String>,
    /// Front-cover art, embedded as a PICTURE block (`CoverFront`,
    /// `image/jpeg`).
    pub cover_art_jpeg: Option<Vec<u8>>,
    /// `MUSICBRAINZ_TRACKID` — Picard stores the *recording* MBID under this
    /// key (the historical name predates the track/recording split).
    pub musicbrainz_recording_id: Option<String>,
    /// `MUSICBRAINZ_ALBUMID` — the release MBID.
    pub musicbrainz_release_id: Option<String>,
    /// `DISCOGS_RELEASE_ID`.
    pub discogs_release_id: Option<String>,
    /// `BPM` — detected tempo. Dub's own catalog already holds this;
    /// the tag is what survives the file being moved to Serato,
    /// Traktor or rekordbox.
    pub bpm: Option<f64>,
    /// `INITIALKEY` — detected key in **Camelot** notation ("8B").
    ///
    /// Camelot rather than musical notation because that is what Dub
    /// itself stores and compares (`dub_spectral::camelot_keys_disagree`)
    /// and what a DJ reads when matching keys. `INITIALKEY` is the
    /// Vorbis mirror of ID3's `TKEY`, and every DJ tool that reads the
    /// field accepts a Camelot string in it.
    pub initial_key: Option<String>,
}

/// Every Vorbis key this crate manages. All of them are cleared before each
/// write so `write_tags` has replace semantics: re-tagging never duplicates
/// values and never leaves stale fields behind from an earlier rip pass.
const MANAGED_KEYS: [&str; 14] = [
    "TITLE",
    "ARTIST",
    "ALBUM",
    "ALBUMARTIST",
    "DATE",
    "GENRE",
    "TRACKNUMBER",
    "TRACKTOTAL",
    "COMMENT",
    "MUSICBRAINZ_TRACKID",
    "MUSICBRAINZ_ALBUMID",
    "DISCOGS_RELEASE_ID",
    "BPM",
    "INITIALKEY",
];

/// Render a tempo for the `BPM` tag.
///
/// Two decimals, with trailing zeros trimmed, so a detected 128.0 writes
/// as `128` rather than `128.00`: readers accept both, but a whole
/// number is what a DJ expects to see in another app's browser.
fn format_bpm(bpm: f64) -> String {
    let s = format!("{bpm:.2}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Write (or replace) Vorbis comments and the front-cover PICTURE block on an
/// existing FLAC file.
///
/// Only fields that are `Some` are written; `None` fields are removed if a
/// previous call wrote them, so calling twice replaces rather than
/// accumulates. Audio frames are untouched.
///
/// # Errors
///
/// [`EncodeError::Tag`] if the file cannot be read as FLAC or the rewritten
/// metadata cannot be saved.
pub fn write_tags(path: &Path, tags: &TrackTags) -> Result<(), EncodeError> {
    let mut tag = Tag::read_from_path(path).map_err(tag_err)?;

    for key in MANAGED_KEYS {
        tag.remove_vorbis(key);
    }

    set_opt(&mut tag, "TITLE", tags.title.as_deref());
    set_opt(&mut tag, "ARTIST", tags.artist.as_deref());
    set_opt(&mut tag, "ALBUM", tags.album.as_deref());
    set_opt(&mut tag, "ALBUMARTIST", tags.album_artist.as_deref());
    set_opt(
        &mut tag,
        "DATE",
        tags.year.map(|y| y.to_string()).as_deref(),
    );
    set_opt(&mut tag, "GENRE", tags.genre.as_deref());
    set_opt(
        &mut tag,
        "TRACKNUMBER",
        tags.track_number.map(|n| n.to_string()).as_deref(),
    );
    set_opt(
        &mut tag,
        "TRACKTOTAL",
        tags.track_total.map(|n| n.to_string()).as_deref(),
    );
    set_opt(&mut tag, "COMMENT", tags.comment.as_deref());
    set_opt(
        &mut tag,
        "MUSICBRAINZ_TRACKID",
        tags.musicbrainz_recording_id.as_deref(),
    );
    set_opt(
        &mut tag,
        "MUSICBRAINZ_ALBUMID",
        tags.musicbrainz_release_id.as_deref(),
    );
    set_opt(
        &mut tag,
        "DISCOGS_RELEASE_ID",
        tags.discogs_release_id.as_deref(),
    );

    set_opt(&mut tag, "BPM", tags.bpm.map(format_bpm).as_deref());
    set_opt(&mut tag, "INITIALKEY", tags.initial_key.as_deref());

    tag.remove_picture_type(PictureType::CoverFront);
    if let Some(jpeg) = &tags.cover_art_jpeg {
        tag.add_picture("image/jpeg", PictureType::CoverFront, jpeg.clone());
    }

    tag.save().map_err(tag_err)
}

fn tag_err(e: metaflac::Error) -> EncodeError {
    EncodeError::Tag(e.to_string())
}

fn set_opt(tag: &mut Tag, key: &str, value: Option<&str>) {
    if let Some(v) = value {
        tag.set_vorbis(key, vec![v]);
    }
}
