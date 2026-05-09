//! Dub audio engine — core types and RT-safety primitives.
//!
//! See PRD §4.2 for the design principles. The audio thread is sacred:
//! no allocation, no locks, no syscalls inside the render callback.
//!
//! M0 ships only:
//!
//! - [`RealtimeContext`] — a lifetime-bounded token type that gates which
//!   APIs may be called inside a render callback.
//! - A no-op [`Engine`] that exercises the type discipline end-to-end.
//! - RT-safety tests (under `#[cfg(test)]`) that prove the no-op render
//!   path is allocation-free, using the `assert_no_alloc` crate.
//!
//! Everything substantive (graph, transport, decks, FX) lands in subsequent
//! milestones. The shape of the public API is intentional: nothing leaks
//! out of [`RealtimeContext`] that could allocate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod realtime;

pub use realtime::{RealtimeContext, RtError};

/// Top-level engine. Sits between the platform audio I/O and the audio graph.
///
/// In M0 the engine just renders silence into whichever output buffer it is
/// asked to fill, while exercising the [`RealtimeContext`] discipline so the
/// type-system gate is real and tested.
#[derive(Debug, Default)]
pub struct Engine {
    sample_rate: f32,
    block_size: usize,
}

impl Engine {
    /// Create a new engine. Allocations may happen here. **This is not the
    /// audio thread.**
    #[must_use]
    pub fn new(sample_rate: f32, block_size: usize) -> Self {
        Self {
            sample_rate,
            block_size,
        }
    }

    /// Sample rate this engine was configured for.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Block size (frames per render call).
    #[must_use]
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Render one block of audio.
    ///
    /// Called from the audio thread. The [`RealtimeContext`] argument is the
    /// only way to invoke APIs that are RT-safe. Anything you can't reach
    /// through `rt` is forbidden in this scope by construction.
    ///
    /// `out` is interleaved stereo, length `2 * block_size`. M0 fills it
    /// with zeros (silence).
    pub fn render(&mut self, rt: &mut RealtimeContext<'_>, out: &mut [f32]) {
        debug_assert_eq!(out.len(), 2 * self.block_size, "buffer size mismatch");
        // Touch the rt token so its presence is enforced and so future
        // refactors that drop the parameter don't go unnoticed.
        rt.tick();
        for sample in out.iter_mut() {
            *sample = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::AllocDisabler;

    #[global_allocator]
    static A: AllocDisabler = AllocDisabler;

    #[test]
    fn engine_constructs() {
        let engine = Engine::new(48_000.0, 64);
        assert!((engine.sample_rate() - 48_000.0).abs() < f32::EPSILON);
        assert_eq!(engine.block_size(), 64);
    }

    #[test]
    fn render_silence_is_silent() {
        let mut engine = Engine::new(48_000.0, 64);
        let mut buffer = vec![1.0f32; 128]; // pre-allocated outside the RT scope
        let mut rt = RealtimeContext::new();

        // The render path must complete without an allocation.
        assert_no_alloc::assert_no_alloc(|| {
            engine.render(&mut rt, &mut buffer);
        });

        // Exact zero comparison is correct here: the silent renderer must
        // write literal `0.0` (positive or negative zero), nothing else.
        #[allow(clippy::float_cmp)]
        for sample in &buffer {
            assert_eq!(*sample, 0.0);
        }
    }

    #[test]
    fn render_advances_rt_tick() {
        let mut engine = Engine::new(48_000.0, 64);
        let mut buffer = vec![0.0f32; 128];
        let mut rt = RealtimeContext::new();
        let before = rt.ticks();

        assert_no_alloc::assert_no_alloc(|| {
            engine.render(&mut rt, &mut buffer);
        });

        assert_eq!(rt.ticks(), before + 1);
    }
}
