//! Recognition for ripped vinyl (M26c).
//!
//! A ripped side arrives as PCM with no metadata beyond what the
//! operator typed. This crate turns that into names: an AcoustID lookup
//! on a TEST2 Chromaprint gives candidate recordings, MusicBrainz turns
//! those into releases and tracklists, and Discogs adds the pressing
//! detail a DJ actually cares about — label, catalogue number, year.
//!
//! **Recognition is enrichment and never a dependency.** A rip works
//! with the cable out; nothing in `dub-rip` calls into this crate on a
//! path that can fail the commit. Every error here means "keep the
//! filename metadata", not "the rip failed".
//!
//! **All network access is behind [`http::Http`].** `ureq` is reachable
//! from exactly one type in this crate and from nothing else in the
//! workspace, so the dependency is auditable and every other piece —
//! fingerprint encoding, response parsing, the release consensus that is
//! the real value here — is testable offline.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod acoustid;
pub mod discogs;
mod error;
mod fingerprint;
pub mod http;
pub mod musicbrainz;
mod side;

/// Sent on every request. MusicBrainz requires a descriptive agent with
/// contact details and will refuse anonymous traffic; the others accept
/// it and it is the polite thing regardless.
pub const USER_AGENT: &str = concat!(
    "Dub/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/kLOsk/dub )"
);

pub use acoustid::Candidate;
pub use discogs::DiscogsRelease;
pub use error::RecognizeError;
pub use fingerprint::{fingerprint, AcoustIdFingerprint, MIN_DURATION_SECS};
pub use musicbrainz::{Release, ReleaseRef, Track};
pub use side::{NamedTrack, Recognizer, SegmentAudio, SegmentMatch, SideRecognition};
