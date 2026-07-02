//! DSP building blocks for Dub.
//!
//! Resamplers, filters, FX primitives. Strict real-time safety:
//! no allocation, no locks, no syscalls inside the inner loops.
//!
//! Implementation lands per milestone (see PRD §12). M0 ships only this
//! placeholder so the workspace builds end-to-end.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bigknob;
pub mod echo;
pub mod hk628;
pub mod loudness;
pub mod phaser;
pub mod pt2399;
pub mod re201;
pub mod siren;
pub mod sn76477;
pub mod spring;
pub mod um3561;

pub use bigknob::{BigKnobHpf, BIG_KNOB_STEPS, BIG_KNOB_STEP_COUNT};
pub use echo::{
    one_pole_coeff, EchoOut, EchoState, DEFAULT_FEEDBACK, DEFAULT_LPF_HZ, MAX_FEEDBACK,
};
pub use hk628::{
    hk623_program, hk623_program_name, hk628_program, hk628_program_name, Hk628, Hk628Program,
    Hk628State, HK623_PROGRAM_COUNT, HK628_PROGRAM_COUNT,
};
pub use loudness::{
    db_to_linear, measure_integrated_loudness, normalization_gain_db, LoudnessMeasurement,
    CEILING_DBFS, DEFAULT_TARGET_LUFS,
};
pub use phaser::Phaser;
pub use pt2399::Pt2399;
pub use re201::{Re201, Re201Mode};
pub use siren::{
    benidub_preset_name, benidub_preset_patch, siren_preset_name, siren_preset_patch,
    siren_preset_route, SirenPatch, SirenRoute, SirenState, SirenVoice, SirenWave,
    BENIDUB_PRESET_COUNT, SIREN_PRESET_COUNT,
};
pub use sn76477::{
    sn76477_preset, sn76477_preset_name, Sn76477, Sn76477Patch, Sn76477State, SN76477_PRESET_COUNT,
};
pub use spring::SpringReverb;
pub use um3561::{um3561_program, um3561_program_name, UM3561_PROGRAM_COUNT};

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
