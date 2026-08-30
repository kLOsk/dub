//! Timecode-vinyl decoder for Dub.
//!
//! Supports **Serato CV02**, **Traktor MK1** and **Traktor MK2** in
//! relative-mode-only (PRD §5.4 / §6). All three share the same
//! decode algorithm — only the nominal carrier frequency differs
//! per format (1 / 2 / 2.5 kHz respectively). Absolute-mode decoding
//! is deferred to a future v1.x milestone.
//!
//! Pipeline:
//!
//! ```text
//!  AudioInput  ──► Decoder.process(stereo_block)  ──► DecodeOutput
//!  (M5.2)            (this crate, M5.1)                 │
//!                                                       ▼
//!                                                Engine deck transport
//!                                                  (rate, position)
//! ```
//!
//! [`signal::Generator`] produces synthetic timecode for tests and
//! offline diagnostics. [`decoder::Decoder`] consumes stereo audio
//! and emits per-block rate/position/amplitude/confidence — see the
//! algorithm note in `decoder.rs`.
//!
//! v1 design choice: relative mode only. We track *changes* in
//! position via the carrier phase, not absolute groove location. The
//! upside is that the decoder needs no AM-bitstream demodulation, no
//! 20-bit position lookup table, and no per-record calibration — a
//! drastically simpler v1 surface that nonetheless covers every
//! scratch DJ use case (PRD §5.4: "absolute mode is for digital
//! mixers we don't target in v1").
//!
//! License note: this is a **clean-room implementation** of the
//! published timecode-vinyl format documented by the xwax and Mixxx
//! projects. No xwax or Mixxx code is copied or derived — everything
//! here is re-implemented from the algorithm description. That is the
//! load-bearing fact: an algorithm is not copyrightable, its
//! expression is, so a clean-room rewrite leaves Dub free to pick its
//! own licence regardless of what those projects use. Do not paste
//! code in from either of them. See `format.rs` for the source list.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
// Vendor product names ("Serato", "Traktor", "CV02", "MK2") are not
// Rust symbols; clippy::doc_markdown is wrong to demand backticks.
#![allow(clippy::doc_markdown)]

mod absolute;
mod classifier;
mod decoder;
mod defs;
mod format;
pub mod signal;
mod smoothing;

pub use absolute::{
    deep_sweep, extract_bits_xwax, extract_cycles, sweep_conventions, sweep_xwax, AbsoluteTracker,
    ConventionResult, CycleObs, DeepResult, Observable, PositionLut, XwaxResult,
};
pub use classifier::{SourceClass, SourceClassifier};
pub use decoder::{
    compute_whitening, whitening_from_covariance, DecodeOutput, Decoder, IDENTITY_WHITENING,
};
pub use defs::{
    find_def, serato_candidates, TimecodeDef, SWITCH_PHASE, SWITCH_POLARITY, SWITCH_PRIMARY,
    TIMECODE_DEFS,
};
pub use format::Format;
pub use smoothing::RateSmoother;

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
