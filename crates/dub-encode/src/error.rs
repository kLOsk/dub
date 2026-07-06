//! Error type shared by the encode and tag halves of the crate.

/// Errors from FLAC encoding and tagging.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// The caller handed us audio we refuse up front: zero or more-than-two
    /// channels, zero sample rate, an empty buffer, or a buffer that isn't
    /// frame-aligned to the channel count.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The FLAC encoder rejected the configuration or the sample stream.
    #[error("FLAC encode failed: {0}")]
    Encode(String),

    /// Filesystem error while writing the encoded stream.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Reading or writing the FLAC metadata blocks failed.
    #[error("FLAC tagging failed: {0}")]
    Tag(String),
}
