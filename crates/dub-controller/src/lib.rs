//! HID/MIDI controller support for Dub.
//!
//! Out of scope for v1.0 (PRD §5.6) — scratch DJs use external mixers, not
//! controllers. Crate exists so the abstraction lands without rework when
//! controller support arrives in v1.x or v2.

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
