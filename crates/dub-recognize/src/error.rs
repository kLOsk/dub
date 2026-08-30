//! Errors, kept coarse on purpose.
//!
//! Recognition is optional enrichment: every failure here means "the rip
//! keeps its filename metadata", never "the rip failed". Callers act on
//! the variant only to decide what to tell the operator.

use crate::http::HttpError;

/// Why recognition could not answer.
#[allow(missing_docs)] // the `#[error]` strings are the documentation
#[derive(Debug, thiserror::Error)]
pub enum RecognizeError {
    #[error("fingerprinting failed: {0}")]
    Fingerprint(String),

    /// Chromaprint needs a real span to be distinctive and AcoustID
    /// will not match a fragment.
    #[error("track is {secs} s; recognition needs at least {min} s")]
    TooShort { secs: u32, min: u32 },

    #[error(transparent)]
    Http(#[from] HttpError),

    /// A service answered, but not with what its contract promises.
    #[error("{service} returned something unexpected: {detail}")]
    Malformed {
        service: &'static str,
        detail: String,
    },

    /// The service said no in its own words — AcoustID puts a message
    /// in the body for a bad key or a malformed fingerprint.
    #[error("{service} refused: {message}")]
    Refused {
        service: &'static str,
        message: String,
    },

    /// No credential configured for a service that requires one.
    #[error("{service} needs a {what}; recognition skipped it")]
    MissingCredential {
        service: &'static str,
        what: &'static str,
    },
}
