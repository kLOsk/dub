//! Vinyl-rip session engine (M26).
//!
//! A `RipSession` owns the off-RT side of ripping a record side: it drains
//! the engine's stereo record tap into a crash-safe WAV spill while
//! accumulating a live peak envelope, holds the split plan (manual in M26a,
//! auto gap detection in M26b), persists everything mutation-by-mutation to
//! a `rip.json` manifest, and on confirm commits each segment through
//! analyze → FLAC encode+tag → library import.
//!
//! This crate never touches the audio thread (the record tap is attached by
//! `dub-engine`; we only consume the ring), and it has no network
//! dependency — recognition (`dub-recognize`, M26c) is bridged by the
//! caller so rips work fully offline.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod capture;
mod commit;
mod gaps;
mod manifest;
mod plan;
mod salvage;
mod session;

pub use capture::{simulate as simulate_auto_capture, AutoCaptureSim};
pub use commit::{CommitProgress, RipOutcome, SegmentOutcome};
pub use gaps::{analyze as analyze_gaps, cell_levels_db, detect_gaps, Gap, GapAnalysis, GapConfig};
pub use manifest::{
    load as load_manifest, save as save_manifest, ManifestError, RipManifest, TrackEntry,
    MANIFEST_FILE, MANIFEST_VERSION,
};
pub use plan::{segments, validate_boundaries, SplitError, TrackMeta, MIN_SEGMENT_SECS};
pub use salvage::{
    envelope_from_samples, list_recoverable, probe as probe_spill, read_all as read_spill_all,
    rebuild_envelope, RecoverableRip, SpillInfo,
};
pub use session::{
    AutoCapture, RipConfig, RipError, RipSession, RipState, RipStatus, StopReason, ARCHIVE_FILE,
    SPILL_FILE,
};
