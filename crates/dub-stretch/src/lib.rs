//! Time-stretch / key-lock engine for Dub.
//!
//! [`SpliceStretcher`] is the one the deck runs: it copies the record sample
//! for sample and joins it right before drum hits and where the record
//! repeats itself, bass and the rest separately. [`WsolaStretcher`] is the
//! stretcher it replaced on 2026-11-19 (waveform-similarity overlap-add), kept
//! so `dub stretch-bench` can still set the two side by side. Both sit behind
//! the [`TimeStretcher`] trait; the audio thread uses the concrete type, never
//! `dyn`. No `unsafe`, no C build dependency, so the crate stays permissively
//! licensed — the posture `dub-bpm` and `dub-fingerprint` chose over their LGPL
//! C alternatives. (An opt-in Rubber Band backend was evaluated in M14.2–M14.4
//! and dropped: a time-domain stretcher sounded better — more faithful stereo —
//! and keeps Dub permissive.)
//!
//! All audio is interleaved stereo `f32` (`L, R, L, R, …`); buffer lengths are
//! `frames * 2`. Frame counts (`consumed` / `produced` / latency) are stereo
//! frames. Every [`TimeStretcher`] method is real-time safe — no allocation,
//! lock, or syscall — with all buffers sized in the constructor. `dub-stretch`
//! deliberately does **not** depend on `dub-engine`, so the `RealtimeContext`
//! token can't gate these methods directly; the engine calls them from inside
//! its own RT-gated `render_into`.
//!
//! ## Key-lock controls
//!
//! A stretcher exposes two orthogonal controls: [`set_time_ratio`] (output
//! length ÷ input length) and [`set_pitch_scale`] (output pitch × input
//! pitch). Dub's master-tempo "key lock" keeps tempo on the platter and pitch
//! fixed; the deck feeds the record in at its own speed and drives the
//! stretcher as a pure time-stretch (`time_ratio = 1 / rate`,
//! `pitch_scale = 1`), so a record at the engine's sample rate is never
//! interpolated. Until 2026-11-19 the deck played the record fast or slow by
//! straight-line interpolation first and had the stretcher pitch it back
//! (`pitch_scale = 1 / rate`), which cost 5 dB at 12–16 kHz. See PRD §6.1.1.
//!
//! [`set_time_ratio`]: TimeStretcher::set_time_ratio
//! [`set_pitch_scale`]: TimeStretcher::set_pitch_scale

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod splice;
mod wsola;

pub use splice::{Joins, SpliceStretcher};
pub use wsola::WsolaStretcher;

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A block-based time/pitch processor driven from the deck's render pull loop.
///
/// Audio is interleaved stereo `f32`; slice lengths are `frames * 2`. All
/// methods are real-time safe: no allocation, lock, or syscall. Every buffer
/// is sized once in the constructor; [`reset`](Self::reset) only zeroes
/// preallocated state.
pub trait TimeStretcher {
    /// Set the pitch multiplier (output pitch ÷ input pitch). `1.0` leaves
    /// pitch unchanged, and is where the deck holds it for key lock.
    /// Cheap — folds into the per-frame hop math; no allocation.
    fn set_pitch_scale(&mut self, scale: f64);

    /// Set the time-stretch ratio (output length ÷ input length). `1.0`
    /// preserves duration. The deck drives this to `1.0 / rate` for key
    /// lock. Cheap; no allocation.
    fn set_time_ratio(&mut self, ratio: f64);

    /// Push `input` (interleaved stereo) and write whatever output is ready
    /// into `output`. Returns `(frames_consumed, frames_produced)`: consumes up
    /// to the smaller of the input supplied and the internal buffer's free
    /// space, produces up to `output.len() / 2` frames. Never allocates.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize);

    /// Upper bound on frames produced for `in_frames` of input at the current
    /// ratio — lets the engine size its output ring off the RT thread. A pure
    /// function of configuration and current ratio.
    fn max_output_for(&self, in_frames: usize) -> usize;

    /// Frames by which the output trails the input: frame `n` of the input
    /// comes out as frame `n + latency` at unity. The engine discards this
    /// many primed frames so a bypass→engaged crossfade lines up with the dry
    /// playhead. Record a stretcher only needs *ahead* of where it plays is
    /// not latency — report it here and a locked deck plays early.
    fn latency_frames(&self) -> usize;

    /// Flush all internal state to silence. RT-safe (zeroes preallocated
    /// buffers only). Called on seek / load / backend switch / re-engage.
    fn reset(&mut self);
}

/// Identity backend: copies input to output frame-for-frame, ignoring the
/// ratio. This is the "resampler-only" path — when key lock is bypassed the
/// deck's own variable-rate resampler does all the work and the stretcher is a
/// no-op. Zero latency.
#[derive(Debug, Default, Clone, Copy)]
pub struct Passthrough;

impl Passthrough {
    /// Construct the identity backend.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl TimeStretcher for Passthrough {
    fn set_pitch_scale(&mut self, _scale: f64) {}
    fn set_time_ratio(&mut self, _ratio: f64) {}

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize) {
        let n = (input.len() / 2).min(output.len() / 2);
        let n2 = n * 2;
        output[..n2].copy_from_slice(&input[..n2]);
        (n, n)
    }

    fn max_output_for(&self, in_frames: usize) -> usize {
        in_frames
    }

    fn latency_frames(&self) -> usize {
        0
    }

    fn reset(&mut self) {}
}

/// Which time-stretch engine a deck is using. `Copy` + FFI-friendly so the UI
/// can toggle key lock per deck. `ResamplerOnly` = key lock off (pitch shifts
/// with rate); `DubOwn` = our own key lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchBackend {
    /// Identity — the deck's own resampler handles rate ([`Passthrough`]).
    ResamplerOnly,
    /// Dub's own stretcher ([`SpliceStretcher`]). The key-lock engine.
    DubOwn,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_nonempty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn passthrough_is_identity() {
        let mut s = Passthrough::new();
        let input: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let mut output = vec![0.0f32; 16];
        let (consumed, produced) = s.process(&input, &mut output);
        assert_eq!((consumed, produced), (8, 8));
        assert_eq!(output, input);
        assert_eq!(s.latency_frames(), 0);
        assert_eq!(s.max_output_for(100), 100);
    }

    #[test]
    fn passthrough_clamps_to_smaller_buffer() {
        let mut s = Passthrough::new();
        let input = vec![1.0f32; 12]; // 6 frames
        let mut output = vec![0.0f32; 8]; // 4 frames
        let (consumed, produced) = s.process(&input, &mut output);
        assert_eq!((consumed, produced), (4, 4));
        assert!(output.iter().all(|&x| x == 1.0));
    }
}
