//! Vorbis-comment + PICTURE tagging tests for `write_tags`.
//!
//! Read-back uses metaflac directly so the assertions check the on-disk
//! metadata blocks, not our own writer's view of them.

use std::path::Path;

use dub_encode::{encode_flac_24bit, write_tags, EncodeError, TrackTags};
use metaflac::block::PictureType;
use metaflac::Tag;

// Truncated JPEG magic is enough: metaflac stores the bytes opaquely.
const JPEG_STUB: [u8; 6] = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

fn encode_fixture(path: &Path) {
    let input = vec![0.1_f32; 4_410];
    encode_flac_24bit(&input, 44_100, 1, path).expect("encode fixture");
}

fn full_tags() -> TrackTags {
    TrackTags {
        title: Some("Chase the Devil".into()),
        artist: Some("Max Romeo".into()),
        album: Some("War Ina Babylon".into()),
        album_artist: Some("Max Romeo & The Upsetters".into()),
        year: Some(1976),
        genre: Some("Reggae".into()),
        track_number: Some(3),
        track_total: Some(10),
        comment: Some("Ripped from vinyl with Dub".into()),
        cover_art_jpeg: Some(JPEG_STUB.to_vec()),
        musicbrainz_recording_id: Some("8b0d47f4-e2f5-4e34-a855-4f9c9f1c7e2a".into()),
        musicbrainz_release_id: Some("0b6b4e28-9d0a-3a8e-a53a-6c294b0f0c9d".into()),
        discogs_release_id: Some("438249".into()),
    }
}

fn vorbis_values(tag: &Tag, key: &str) -> Vec<String> {
    tag.get_vorbis(key)
        .map(|values| values.map(str::to_owned).collect())
        .unwrap_or_default()
}

fn assert_single(tag: &Tag, key: &str, expected: &str) {
    assert_eq!(
        vorbis_values(tag, key),
        vec![expected.to_owned()],
        "field {key}"
    );
}

#[test]
fn all_fields_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("tagged.flac");
    encode_fixture(&path);

    write_tags(&path, &full_tags()).expect("write tags");

    let tag = Tag::read_from_path(&path).expect("read tags back");
    assert_single(&tag, "TITLE", "Chase the Devil");
    assert_single(&tag, "ARTIST", "Max Romeo");
    assert_single(&tag, "ALBUM", "War Ina Babylon");
    assert_single(&tag, "ALBUMARTIST", "Max Romeo & The Upsetters");
    assert_single(&tag, "DATE", "1976");
    assert_single(&tag, "GENRE", "Reggae");
    assert_single(&tag, "TRACKNUMBER", "3");
    assert_single(&tag, "TRACKTOTAL", "10");
    assert_single(&tag, "COMMENT", "Ripped from vinyl with Dub");
    assert_single(
        &tag,
        "MUSICBRAINZ_TRACKID",
        "8b0d47f4-e2f5-4e34-a855-4f9c9f1c7e2a",
    );
    assert_single(
        &tag,
        "MUSICBRAINZ_ALBUMID",
        "0b6b4e28-9d0a-3a8e-a53a-6c294b0f0c9d",
    );
    assert_single(&tag, "DISCOGS_RELEASE_ID", "438249");

    let pictures: Vec<_> = tag.pictures().collect();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].picture_type, PictureType::CoverFront);
    assert_eq!(pictures[0].mime_type, "image/jpeg");
    assert_eq!(pictures[0].data, JPEG_STUB.to_vec());
}

#[test]
fn sparse_fields_leave_absent_keys_unwritten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sparse.flac");
    encode_fixture(&path);

    let tags = TrackTags {
        title: Some("Dubwise".into()),
        artist: Some("King Tubby".into()),
        ..TrackTags::default()
    };
    write_tags(&path, &tags).expect("write sparse tags");

    let tag = Tag::read_from_path(&path).expect("read tags back");
    assert_single(&tag, "TITLE", "Dubwise");
    assert_single(&tag, "ARTIST", "King Tubby");
    for key in [
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
    ] {
        assert!(
            tag.get_vorbis(key).is_none(),
            "key {key} must not be written for a None field"
        );
    }
    assert_eq!(tag.pictures().count(), 0);
}

#[test]
fn rewriting_replaces_instead_of_duplicating() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("rewrite.flac");
    encode_fixture(&path);

    write_tags(&path, &full_tags()).expect("first write");

    let mut updated = full_tags();
    updated.title = Some("Croaking Lizard".into());
    updated.track_number = Some(4);
    write_tags(&path, &updated).expect("second write");

    let tag = Tag::read_from_path(&path).expect("read tags back");
    assert_single(&tag, "TITLE", "Croaking Lizard");
    assert_single(&tag, "TRACKNUMBER", "4");
    assert_single(&tag, "ARTIST", "Max Romeo");
    assert_eq!(tag.pictures().count(), 1, "picture must not duplicate");
}

#[test]
fn missing_file_is_a_tag_error() {
    let err = write_tags(
        Path::new("/nonexistent-dub-encode-test/missing.flac"),
        &full_tags(),
    )
    .unwrap_err();
    assert!(matches!(err, EncodeError::Tag(_)), "got {err:?}");
}
