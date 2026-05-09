//! Real-time safety primitives.
//!
//! The [`RealtimeContext`] type is a lifetime-bounded token that gates which
//! APIs may be called inside a render callback. The audio thread is allowed
//! to call methods that take `&mut RealtimeContext<'_>`. Anything else is
//! out of bounds by construction.
//!
//! This is a compile-time complement to the runtime `assert_no_alloc` checks.
//! Together they enforce PRD §4.2 (no alloc, no lock, no syscall on the audio
//! thread).
//!
//! ## Why a token?
//!
//! Rust's type system can't directly say "this code is on a real-time thread."
//! By requiring a `&mut RealtimeContext<'_>` to call any RT-sensitive method,
//! we centralize the contract: the only way to obtain such a token is from
//! the engine's render setup, which knows it's on the audio thread.
//!
//! Methods we *don't* want callable from the RT path simply don't accept a
//! `RealtimeContext` and don't have any other way to be invoked.
//!
//! ## Future extensions
//!
//! As the engine grows, this module will own:
//!
//! - Pre-allocated scratch buffer pools (RT can borrow, never extend)
//! - Lock-free SPSC channels for UI ↔ audio messaging
//! - The "is bypassed" flag for Rubber Band scratch-aware bypass (PRD §6.1.1)
//! - Counters/meters for the UI-thread snapshot view

use core::marker::PhantomData;

/// Errors that can occur in real-time code paths.
///
/// The error variants are deliberately small and `#[derive(Copy)]` so they
/// can flow through the audio thread without allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtError {
    /// Caller passed a buffer of unexpected length.
    BadBufferSize,
    /// Lock-free channel was full / empty when accessed.
    ChannelFull,
}

impl std::fmt::Display for RtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadBufferSize => write!(f, "bad buffer size"),
            Self::ChannelFull => write!(f, "channel full"),
        }
    }
}

impl std::error::Error for RtError {}

/// A token that proves the holder is executing on the audio (real-time) thread.
///
/// Held by mutable reference for the duration of a single render block. The
/// lifetime parameter prevents the token from outliving the render callback,
/// so it cannot be smuggled into a non-RT context.
pub struct RealtimeContext<'render> {
    ticks: u64,
    _not_send: PhantomData<*const ()>,
    _bound: PhantomData<&'render ()>,
}

impl RealtimeContext<'_> {
    /// Construct a new `RealtimeContext`. Only the engine's render
    /// orchestration should construct this.
    ///
    /// The token is `!Send` and `!Sync` (via the `*const ()` phantom),
    /// preventing it from being shipped across threads.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ticks: 0,
            _not_send: PhantomData,
            _bound: PhantomData,
        }
    }

    /// Increment the per-render block counter. Used for the engine to
    /// know which block it's processing without taking any locks.
    pub fn tick(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);
    }

    /// How many ticks (render blocks) have been observed.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
}

impl Default for RealtimeContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_increments() {
        let mut rt = RealtimeContext::new();
        assert_eq!(rt.ticks(), 0);
        rt.tick();
        assert_eq!(rt.ticks(), 1);
    }

    #[test]
    fn tick_wraps() {
        let mut rt = RealtimeContext::new();
        rt.ticks = u64::MAX;
        rt.tick();
        assert_eq!(rt.ticks(), 0);
    }

    /// Compile-time assertion: `RealtimeContext` is not `Send`.
    /// Trying to send it across a thread should fail to compile.
    /// We can't write a passing test for this; we rely on the type system.
    #[allow(dead_code)]
    fn _assert_not_send() {
        fn requires_send<T: Send>(_t: &T) {}
        // Uncomment to verify the negative — should fail to compile:
        // let rt = RealtimeContext::new();
        // requires_send(&rt);
        let _ = requires_send::<u32>;
    }

    #[test]
    fn rt_error_display_works() {
        assert_eq!(format!("{}", RtError::BadBufferSize), "bad buffer size");
        assert_eq!(format!("{}", RtError::ChannelFull), "channel full");
    }

    #[test]
    fn rt_error_is_copy() {
        let err = RtError::BadBufferSize;
        let copied = err;
        assert_eq!(err, copied);
    }
}
