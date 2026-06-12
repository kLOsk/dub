//! Track library for Dub.
//!
//! Owns a SQLite database describing the user's tracks. Imports from
//! Serato, Traktor (NML), rekordbox (DB6 + XML), iTunes XML, and Lexicon
//! (via its rekordbox/Serato exports). See PRD §8 and the normative
//! schema reference at `docs/spec/LIBRARY-SCHEMA.md`.
//!
//! M11a (this commit) ships:
//!
//! * The full v1 SQLite schema with migration runner.
//! * Path-by-volume-UUID model via [`volumes::discover_for_path`]
//!   on macOS.
//! * [`Library`] handle with default-path open + volume upsert.
//! * Public source / event-type enums (used by callers building
//!   per-source metadata rows).
//!
//! M11b–M11f grow the surface to fingerprint cache, file
//! registration, metadata writes, crate management, browser
//! queries, and exporters (PRD §12).

// Volume discovery on macOS calls `getattrlist(2)` via a small unsafe
// FFI shim (see `volumes::macos`). The rest of the crate is
// safe-only; we'd rather grant `unsafe` here than smuggle a CFString
// dependency into every workspace consumer just to avoid one syscall.
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod analysis;
mod db;
mod dedupe;
mod error;
mod filename_parser;
mod history;
mod importer;
mod paths;
mod schema;
mod version_tokens;
mod volumes;

pub use analysis::{ActiveBeatgrid, AnalysisOutcome};
pub use db::{
    CrateRow, FileScanRow, Library, MissingTrack, SessionPlay, StoredFingerprint, TrackRow,
    TrackSortKey, TransitionStat,
};
pub use dedupe::{
    decide as decide_dedupe, DedupeDecision, DedupeInput, SiblingReason, DURATION_DELTA_MS,
    SIMILARITY_THRESHOLD,
};
pub use error::{LibraryError, Result};
pub use filename_parser::{is_junk_title, parse as parse_filename, ParsedFilename};
pub use history::{HistoryEventType, HistoryWrite, SessionTracker, MIN_TRANSITION_PLAY_MS};
pub use importer::{import_folder, ImportError, ImportSummary};
pub use paths::{default_library_db_path, default_waveforms_cache_dir, waveform_sidecar_path};
pub use schema::SCHEMA_VERSION;
pub use version_tokens::{parse as parse_version_tokens, VersionToken};
pub use volumes::{discover_for_path, DiscoveredVolume};

// Re-export the fingerprint primitives so callers don't have to add
// `dub-fingerprint` to their own Cargo.toml just to construct a
// `DedupeInput`.
pub use dub_fingerprint::{similarity as fingerprint_similarity, Fingerprint, FingerprintError};

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// External library sources we import from. These map onto the
/// `track_metadata_source.source`, `track_beatgrids.source`, and
/// `imported_crates.source` enum strings in the SQL schema. The
/// `as_str` method gives the canonical lowercase string used in
/// every `CHECK (source IN (...))` constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Serato (ID3 GEOB tags + crate files + database V2).
    Serato,
    /// Traktor `collection.nml` (XML).
    Traktor,
    /// rekordbox (`master.db` SQLite or XML export).
    Rekordbox,
    /// iTunes / Apple Music `Library.xml`.
    ITunes,
    /// Lexicon DJ exports (re-imported via Serato or rekordbox path).
    Lexicon,
}

impl Source {
    /// The canonical lowercase string used in the SQL schema. Lexicon
    /// has no direct SQL identity (it round-trips through Serato or
    /// rekordbox); callers map it before persisting.
    pub fn as_str(&self) -> Option<&'static str> {
        match self {
            Source::Serato => Some("serato"),
            Source::Traktor => Some("traktor"),
            Source::Rekordbox => Some("rekordbox"),
            Source::ITunes => Some("itunes"),
            Source::Lexicon => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn sources_are_distinct() {
        assert_ne!(Source::Serato, Source::Traktor);
        assert_ne!(Source::Rekordbox, Source::ITunes);
    }

    #[test]
    fn source_strings_match_schema_check_constraints() {
        assert_eq!(Source::Serato.as_str(), Some("serato"));
        assert_eq!(Source::Traktor.as_str(), Some("traktor"));
        assert_eq!(Source::Rekordbox.as_str(), Some("rekordbox"));
        assert_eq!(Source::ITunes.as_str(), Some("itunes"));
        assert_eq!(Source::Lexicon.as_str(), None);
    }
}
