//! Timecode-vinyl decoder for Dub.
//!
//! Supports Serato CV02 and Traktor MK2 control vinyl. Derived in spirit
//! from xwax (BSD-licensed). Relative-mode-only in v1 (see PRD §5.4).
//!
//! Implementation lands in M5–M6 (see PRD §12).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Supported timecode formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Serato Control Vinyl CV02
    SeratoCv02,
    /// Traktor MK2 Timecode
    TraktorMk2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn formats_are_distinct() {
        assert_ne!(Format::SeratoCv02, Format::TraktorMk2);
    }
}
