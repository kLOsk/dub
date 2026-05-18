//! Error type for the library subsystem (M11a).
//!
//! Library failures travel out to the caller as typed errors per
//! `.cursor/rules/rust-general.mdc`. The migration runner, schema
//! creation, volume registration, and FTS5 trigger setup all funnel
//! through this enum. UI / CLI callers `?` against it.
//!
//! **No `unwrap()` outside test code.** Every `rusqlite::Error` and
//! `std::io::Error` becomes a typed variant here so callers see
//! "Dub library is on a schema version newer than this binary"
//! rather than "FOREIGN KEY constraint failed" at the bottom of a
//! panic.

use std::path::PathBuf;

use thiserror::Error;

/// All failures the `dub-library` crate can surface to its caller.
#[derive(Debug, Error)]
pub enum LibraryError {
    /// The library file (or one of its parents) could not be created
    /// or opened. Wraps the underlying `io::Error` for context.
    #[error("library i/o error at {path:?}: {source}")]
    Io {
        /// The path the I/O was attempted against.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A `rusqlite` call returned an error during schema creation,
    /// migration, or a runtime query. Wraps the underlying SQLite
    /// error with a short context string identifying the operation.
    #[error("sqlite error during {context}: {source}")]
    Sqlite {
        /// Short human-readable operation tag (`"open"`, `"migrate"`,
        /// `"register_volume"`, ...) so callers can tell which phase
        /// failed without re-parsing the SQLite message.
        context: &'static str,
        /// The underlying SQLite error.
        #[source]
        source: rusqlite::Error,
    },

    /// The database on disk is at a `schema_version` strictly greater
    /// than what this binary knows how to write. v1.0 ships a
    /// strict-write guard per `docs/LIBRARY-SCHEMA.md` — see the
    /// "What v1.0 does not commit to" section. The caller may
    /// degrade to read-only or refuse to operate.
    #[error(
        "library schema_version {found} is newer than this binary supports ({supported}); \
         refusing to write"
    )]
    SchemaTooNew {
        /// The `schema_version` value read from the database.
        found: u32,
        /// The highest `schema_version` this binary can apply.
        supported: u32,
    },

    /// The database on disk is at a `schema_version` strictly less
    /// than this binary supports *and* no migration path from that
    /// version to the supported version is registered. Indicates a
    /// developer error (a forgotten migration entry), not a user-
    /// facing condition; surfaces as a hard refusal so we never
    /// silently corrupt data.
    #[error(
        "library schema_version {found} has no migration path to {supported}; \
         missing migration entry"
    )]
    MigrationMissing {
        /// The current on-disk `schema_version`.
        found: u32,
        /// The target `schema_version` (the binary's supported version).
        supported: u32,
    },

    /// macOS volume discovery via `getattrlist(2)` failed for the
    /// given mount point. Common causes: a network filesystem that
    /// does not expose a stable UUID, or an unmounted path. The
    /// caller should surface this to the user as "this volume cannot
    /// be tracked; copy the files to a UUID-bearing volume first."
    #[error("could not resolve volume UUID for path {path:?}: {reason}")]
    VolumeUuidUnavailable {
        /// The path the resolution was attempted against.
        path: PathBuf,
        /// Short description of the failure mode for the user-facing
        /// error message.
        reason: &'static str,
    },

    /// `analyze_track` was called with a canonical UUID that is not
    /// present in `tracks`. Indicates a stale id on the caller side
    /// (UI raced a delete) rather than a library bug.
    #[error("no track with id {track_id} found")]
    TrackNotFound {
        /// The canonical UUID the caller asked for.
        track_id: String,
    },

    /// `analyze_track` was called for a track that has no
    /// `fingerprint_id`. Should not happen via M11c imports
    /// (the importer always fingerprints first), but checked
    /// explicitly because `analysis_cache` is keyed by fingerprint
    /// id and we'd rather refuse than write a NULL FK row.
    #[error("track {track_id} has no fingerprint; cannot run analysis")]
    TrackHasNoFingerprint {
        /// The canonical UUID we tried to analyse.
        track_id: String,
    },

    /// `analyze_track` was called for a track whose primary file
    /// is currently unreachable (volume unmounted, file moved).
    /// Caller surfaces this as "file missing; relocate first";
    /// after a successful Relocate run, re-analysis works.
    #[error("track {track_id} has no resolvable file on disk")]
    TrackHasNoFile {
        /// The canonical UUID we tried to analyse.
        track_id: String,
    },

    /// `analyze_track` failed because `dub-io` could not decode the
    /// underlying file (corrupt container, unsupported codec) or
    /// `dub-bpm` rejected the buffer (sample-rate / channel-count
    /// invariants violated by a degraded file). Surfaces a one-line
    /// reason so the user-facing error message can read "Decode
    /// failed: …" instead of swallowing the cause.
    #[error("decode or analysis failed for track {track_id} at {path:?}: {reason}")]
    DecodeFailed {
        /// The canonical UUID we tried to analyse.
        track_id: String,
        /// The file we tried to decode.
        path: PathBuf,
        /// Short reason string from the underlying decoder /
        /// analyser.
        reason: String,
    },
}

impl LibraryError {
    /// Convenience for wrapping a `rusqlite::Error` with a short
    /// context tag without losing the underlying error chain.
    pub(crate) fn sqlite(context: &'static str, source: rusqlite::Error) -> Self {
        LibraryError::Sqlite { context, source }
    }

    /// Convenience for wrapping an `io::Error` with the path being
    /// operated on.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        LibraryError::Io {
            path: path.into(),
            source,
        }
    }
}

/// Convenience alias for crate-internal `Result`s.
pub type Result<T> = std::result::Result<T, LibraryError>;
