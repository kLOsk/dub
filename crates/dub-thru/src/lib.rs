//! Thru-mode source-detection types for Dub (placeholder).
//!
//! PRD §5.1.1 specified a per-deck auto-detection classifier (timecode vs
//! real music vs silence). That auto-detection is **deferred** — the current
//! build has no automatic source switching (the user picks the source on the
//! deck-header INT·TC·THRU switch), and the telemetry-only classifier that
//! does run lives in `dub-engine` (`SourceClassifier`). This crate currently
//! holds only the shared `DetectedMode` enum; the Thru *passthrough* itself
//! is `ThruSource` in `dub-engine`. The Direct/Processed switch once planned
//! for §5.2.2 was removed.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Detected source mode for a deck input.
///
/// State machine described in PRD §5.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedMode {
    /// No signal above noise floor.
    Silent,
    /// Classifier still gathering evidence (250–500 ms window).
    Detecting,
    /// LFSR lock acquired; audio is timecode.
    Timecode,
    /// Music detected; route through Thru pipeline.
    Music,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn modes_are_distinct() {
        assert_ne!(DetectedMode::Silent, DetectedMode::Timecode);
        assert_ne!(DetectedMode::Music, DetectedMode::Timecode);
    }
}
