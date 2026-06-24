//! Serato library format (M11e).
//!
//! Pure, panic-free parsers for the on-disk + in-file Serato data:
//! * [`database`] — the `database V2` track list + `Subcrates/*.crate` tree
//!   (binary tag container).
//! * [`beatgrid`] / [`markers2`] / [`autotags`] — the per-track `GEOB` blobs
//!   (beat grid, hot cues / loops, BPM / gain).
//! * [`geob`] — the ID3 I/O boundary that pulls those blobs out of the audio
//!   files (the only module here that touches the filesystem).
//!
//! The import adapter that wires these into the library schema lives in
//! `crate::serato_import`.

pub mod autotags;
pub mod beatgrid;
pub mod database;
pub mod geob;
pub mod markers2;

/// Base64 engine for Serato's GEOB payloads. They are sometimes `=`-padded,
/// sometimes not, and often newline-wrapped — so we use the STANDARD
/// alphabet with **padding-indifferent** decoding (callers strip whitespace
/// + NULs first). A strict NO_PAD engine rejects the padded variants.
pub(crate) const B64: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    base64::engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
);
