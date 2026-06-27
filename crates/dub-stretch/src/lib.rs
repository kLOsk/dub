//! Time-stretch / key-lock engine for Dub.
//!
//! [`WsolaStretcher`] — pure-Rust, FFT-free WSOLA (waveform-similarity
//! overlap-add), behind the [`TimeStretcher`] trait and dispatched by the
//! [`Stretcher`] enum (no `dyn` — the audio thread forbids heap-allocated trait
//! objects). No `unsafe`, no C build dependency, so the crate stays permissively
//! licensed — the posture `dub-bpm` and `dub-fingerprint` chose over their LGPL
//! C alternatives. (An opt-in Rubber Band backend was evaluated in M14.2–M14.4
//! and dropped: WSOLA sounded better — more faithful stereo — and keeps Dub
//! permissive.)
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
//! fixed; the deck drives the stretcher as a pure pitch corrector
//! (`time_ratio = 1`, `pitch_scale = 1 / rate`) so the insert is
//! frame-count-preserving and the transport playhead stays exact. See PRD
//! §6.1.1 and the M14 plan.
//!
//! [`set_time_ratio`]: TimeStretcher::set_time_ratio
//! [`set_pitch_scale`]: TimeStretcher::set_pitch_scale

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod wsola;

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
    /// pitch unchanged. The deck drives this to `1.0 / rate` for key lock.
    /// Cheap — folds into the per-frame hop math; no allocation.
    fn set_pitch_scale(&mut self, scale: f64);

    /// Set the time-stretch ratio (output length ÷ input length). `1.0`
    /// preserves duration. The deck holds this at `1.0` for key lock (its own
    /// resampler already moved tempo). Cheap; no allocation.
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

    /// Constant algorithmic latency in stereo frames at the current config.
    /// The engine discards this many primed frames so a bypass→engaged
    /// crossfade lines up with the dry playhead.
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
/// with rate); `DubOwn` = our WSOLA key lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchBackend {
    /// Identity — the deck's own resampler handles rate ([`Passthrough`]).
    ResamplerOnly,
    /// Dub's pure-Rust WSOLA ([`WsolaStretcher`]). The default key-lock engine.
    DubOwn,
}

/// One deck's selected stretcher, dispatched by `match` (no `dyn` on the audio
/// thread).
pub enum Stretcher {
    /// Identity / resampler-only.
    Passthrough(Passthrough),
    /// Pure-Rust WSOLA.
    DubOwn(WsolaStretcher),
}

impl TimeStretcher for Stretcher {
    fn set_pitch_scale(&mut self, scale: f64) {
        match self {
            Stretcher::Passthrough(s) => s.set_pitch_scale(scale),
            Stretcher::DubOwn(s) => s.set_pitch_scale(scale),
        }
    }

    fn set_time_ratio(&mut self, ratio: f64) {
        match self {
            Stretcher::Passthrough(s) => s.set_time_ratio(ratio),
            Stretcher::DubOwn(s) => s.set_time_ratio(ratio),
        }
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize) {
        match self {
            Stretcher::Passthrough(s) => s.process(input, output),
            Stretcher::DubOwn(s) => s.process(input, output),
        }
    }

    fn max_output_for(&self, in_frames: usize) -> usize {
        match self {
            Stretcher::Passthrough(s) => s.max_output_for(in_frames),
            Stretcher::DubOwn(s) => s.max_output_for(in_frames),
        }
    }

    fn latency_frames(&self) -> usize {
        match self {
            Stretcher::Passthrough(s) => s.latency_frames(),
            Stretcher::DubOwn(s) => s.latency_frames(),
        }
    }

    fn reset(&mut self) {
        match self {
            Stretcher::Passthrough(s) => s.reset(),
            Stretcher::DubOwn(s) => s.reset(),
        }
    }
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
