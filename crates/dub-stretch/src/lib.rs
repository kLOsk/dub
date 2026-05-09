//! Rubber Band time-stretch wrapper for Dub.
//!
//! Isolated in its own crate for license clarity (Rubber Band is GPL).
//!
//! Implementation lands in M14 (see PRD §12). The scratch-aware auto-bypass
//! described in PRD §6.1.1 lives here and in `dub-engine`.

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
