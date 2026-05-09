//! Track library for Dub.
//!
//! Owns a SQLite database describing the user's tracks. Imports from
//! Serato, Traktor (NML), rekordbox (DB6 + XML), iTunes XML, and Lexicon
//! (via its rekordbox/Serato exports). See PRD §8.
//!
//! Implementation lands in M11–M12 (see PRD §12).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// External library sources we import from.
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
}
