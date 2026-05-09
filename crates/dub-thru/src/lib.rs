//! Thru-mode pipeline for Dub.
//!
//! Routes real-record audio from the audio interface input through the
//! engine. Implements the auto-detection classifier described in PRD §5.1.1
//! (timecode vs real music vs silence) and the Direct/Processed switch in
//! §5.2.2.
//!
//! Implementation lands in M7–M9 (see PRD §12).

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
