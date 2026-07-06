//! Offline audio encoding for the M26 vinyl-rip pipeline.
//!
//! Scope (M26a): FLAC encoding of ripped tracks and the full-side archive,
//! plus Vorbis-comment / PICTURE tagging with Picard-convention MusicBrainz
//! fields so provenance rides inside the files themselves (portable to
//! Serato/rekordbox; no library schema extension needed).
//!
//! Deliberately permissive-only (flacenc Apache-2.0, metaflac MIT): the
//! MP3/LAME (LGPL) path was considered and deferred — see
//! `docs/spec/LICENSE-DEPENDENCIES.md`.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod encode;
mod error;
mod tag;

pub use encode::encode_flac_24bit;
pub use error::EncodeError;
pub use tag::{write_tags, TrackTags};
