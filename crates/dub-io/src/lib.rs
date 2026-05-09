//! Audio file I/O for Dub.
//!
//! Per PRD §4.4, tracks are decoded fully into RAM on load to support
//! sample-accurate, bidirectional playback (forward and backward are
//! byte-for-byte symmetric in the engine). No per-block disk streaming.
//!
//! Implementation lands in M3 (see PRD §12).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!VERSION.is_empty());
    }
}
