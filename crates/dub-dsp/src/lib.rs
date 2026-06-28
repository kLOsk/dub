//! DSP building blocks for Dub.
//!
//! Resamplers, filters, FX primitives. Strict real-time safety:
//! no allocation, no locks, no syscalls inside the inner loops.
//!
//! Implementation lands per milestone (see PRD §12). M0 ships only this
//! placeholder so the workspace builds end-to-end.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod echo;
pub mod loudness;
pub mod siren;

pub use echo::{
    one_pole_coeff, EchoOut, EchoState, DEFAULT_FEEDBACK, DEFAULT_LPF_HZ, MAX_FEEDBACK,
};
pub use loudness::{
    db_to_linear, measure_integrated_loudness, normalization_gain_db, LoudnessMeasurement,
    CEILING_DBFS, DEFAULT_TARGET_LUFS,
};
pub use siren::{
    siren_preset_name, siren_preset_patch, SirenPatch, SirenState, SirenVoice, SirenWave,
    SIREN_PRESET_COUNT,
};

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
