//! FFI bindings for the Apple frontend.
//!
//! UniFFI integration lands in M0.5 (see PRD §12). For now this crate just
//! re-exports a small handshake surface so the Swift app can verify it
//! linked the right artifact.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// FFI surface version. Bump when adding/removing exported symbols.
pub const FFI_VERSION: u32 = 1;

/// Returns a static greeting string. The Apple shell calls this on launch
/// to verify it linked the Rust core successfully.
pub fn greeting() -> &'static str {
    "Dub engine OK"
}

/// Returns the version of the underlying dub-engine crate.
pub fn engine_version() -> &'static str {
    dub_engine::VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_version_is_one() {
        assert_eq!(FFI_VERSION, 1);
    }

    #[test]
    fn greeting_matches() {
        assert_eq!(greeting(), "Dub engine OK");
    }

    #[test]
    fn engine_version_matches_crate() {
        assert_eq!(engine_version(), dub_engine::VERSION);
    }
}
