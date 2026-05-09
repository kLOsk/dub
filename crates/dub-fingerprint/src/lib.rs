//! Audio fingerprinting for Dub. v1.1 feature.
//!
//! Recognizes real records played in Thru mode by computing a rolling
//! fingerprint over a 5-second window and matching against a local index
//! of records the user has previously played. See PRD §5.2.5.
//!
//! Implementation lands in M21 (post-v1.0).

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
