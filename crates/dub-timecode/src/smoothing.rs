//! Outlier-rejecting smoother for the decoded timecode **rate**.
//!
//! The block phase-difference decoder ([`crate::Decoder`]) emits a rate
//! estimate every audio block. On a clean carrier most blocks are good,
//! but a worn stylus, a dust tick, a ground-hum beat, or a momentary
//! crosstalk glitch produces the occasional **impulsive outlier** — a
//! single block whose rate is wildly off. Fed straight to the deck
//! transport (which it is — playback uses the raw block rate) those
//! spikes are audible as pitch wow, and the steady-state estimator
//! noise shows up as a constant waver.
//!
//! A plain EMA can't fix this: it's a low-pass, not an outlier rejecter,
//! so it just bleeds a fraction of every spike through. This smoother is
//! two causal, RT-safe stages instead:
//!
//!   1. **Hampel spike rejection** over a short window. A lone block
//!      whose rate deviates from the window median by more than
//!      `k · MAD` (floored, so tiny noise isn't "a spike") is replaced
//!      by the median. *Sustained* motion — a scratch — is consistent
//!      across the window, so it sails through untouched; only true
//!      impulses are caught.
//!   2. **A velocity-adaptive one-pole.** When the platter is near
//!      steady (small innovation) the pole is slow → heavy smoothing,
//!      killing wow. When the rate is genuinely moving (large
//!      innovation) the pole opens toward unity → snaps, preserving
//!      scratch feel.
//!
//! Below [`MIN_CONFIDENCE`] (a stylus lift, a dropout, or the incoherent
//! turnaround of an aggressive scratch) the smoother **resets and passes
//! the raw rate through** — we neither trust nor smooth garbage, and the
//! lift policy ignores the rate there anyway. This also means hard
//! scratch reversals, which momentarily collapse carrier coherence,
//! bypass the spike stage entirely and stay tight.
//!
//! RT-safe: fixed stack arrays, in-place sort of ≤ `WINDOW` elements, no
//! allocation, no locks.

/// Hampel window length (blocks). At the design cadence
/// ([`DESIGN_DT_SECS`]) that's ~6.7 ms; a hard edge is accepted after
/// ⌈WINDOW/2⌉ blocks (~4 ms), which bounds the worst-case reversal lag.
const WINDOW: usize = 5;

/// Hampel cutoff: deviations beyond `HAMPEL_K · MAD` are outliers.
const HAMPEL_K: f64 = 3.0;

/// Floor on the spike threshold (normalized rate **per design block**).
/// Below this, deviation is treated as ordinary estimator noise for the
/// one-pole to absorb, not a spike to reject — so a flat trace isn't
/// chased by its own MAD. The applied floor scales linearly with the
/// real block duration: a *genuine* ramp moves `accel · dt` per block,
/// so at a 512-frame CoreAudio quantum (8× the design block) a
/// deceleration that is obviously real at 64 frames would read as a
/// "spike" against an unscaled floor — the Hampel then replayed the
/// stale pre-ramp rate for half a window, which was a measured ~+9 ms
/// of forward sticker drift per scratch decel (`tests/scratch_drift.rs`).
const SPIKE_FLOOR: f64 = 0.012;

/// The block cadence the constants above were tuned at: 64 frames at
/// 48 kHz. [`RateSmoother::smooth`] assumes it; the engine calls
/// [`RateSmoother::smooth_for`] with the real quantum so behavior is
/// block-size independent.
const DESIGN_DT_SECS: f64 = 64.0 / 48_000.0;

/// Steady-platter smoothing time constant (τ ≈ 67 ms — the design
/// `ALPHA_MIN = 0.02` per 64-frame block). Expressed in time so a
/// 512-frame quantum converges at the same *wall-clock* speed instead
/// of 8× slower: the per-block alpha left the smoothed rate ~0.011
/// short of the true rate through every steady stroke, and because a
/// scratch's backward draw lasts several times longer than its push,
/// that shortfall integrated into systematic forward sticker drift.
const TAU_MIN_SECS: f64 = 0.0667;
/// Innovation deadband (normalized rate). Below this the pole stays at
/// `ALPHA_MIN` — the key fix: steady-state noise must not open the
/// filter and defeat its own smoothing. ~1.5 % covers flutter + jitter
/// while staying well under a deliberate beatmatch nudge.
const MOTION_DEADBAND: f64 = 0.015;
/// Innovation past the deadband that drives the pole fully open. A
/// scratch is tracked at near-unity gain; a gentle nudge eases in.
const MOTION_SCALE: f64 = 0.12;

/// Below this decoder confidence the input is a lift / dropout / scratch
/// turnaround: reset and pass through rather than smooth noise.
const MIN_CONFIDENCE: f32 = 0.6;

/// Causal, RT-safe rate de-spiker + adaptive smoother. One per deck.
#[derive(Debug, Clone)]
pub struct RateSmoother {
    window: [f64; WINDOW],
    len: usize,
    pos: usize,
    smoothed: f64,
    primed: bool,
}

impl Default for RateSmoother {
    fn default() -> Self {
        Self::new()
    }
}

impl RateSmoother {
    /// A fresh smoother, unprimed, reporting unity until the first
    /// confident block arrives.
    #[must_use]
    pub fn new() -> Self {
        Self {
            window: [0.0; WINDOW],
            len: 0,
            pos: 0,
            smoothed: 1.0,
            primed: false,
        }
    }

    /// Drop all history so the next confident block snaps instead of
    /// slewing. Called on lift/dropout and on a fresh attach.
    pub fn reset(&mut self) {
        self.len = 0;
        self.pos = 0;
        self.primed = false;
    }

    /// Smooth one block's `rate`, gated by the decoder's `confidence`,
    /// assuming the design 64-frame @ 48 kHz cadence. Tests and
    /// diagnostic tools use this; the engine calls
    /// [`Self::smooth_for`] with the real block duration.
    #[must_use]
    pub fn smooth(&mut self, rate: f64, confidence: f32) -> f64 {
        self.smooth_for(rate, confidence, DESIGN_DT_SECS)
    }

    /// Smooth one block of `dt_secs` duration. Time-based: the spike
    /// floor and the one-pole coefficient both derive from `dt_secs`,
    /// so a 512-frame CoreAudio quantum tracks at the same wall-clock
    /// speed as the 64-frame design cadence.
    #[must_use]
    pub fn smooth_for(&mut self, rate: f64, confidence: f32, dt_secs: f64) -> f64 {
        if confidence < MIN_CONFIDENCE {
            self.reset();
            return rate;
        }

        self.window[self.pos] = rate;
        self.pos = (self.pos + 1) % WINDOW;
        if self.len < WINDOW {
            self.len += 1;
        }

        let med = self.median();
        let floor = SPIKE_FLOOR * (dt_secs / DESIGN_DT_SECS).max(1.0);
        let threshold = (HAMPEL_K * self.mad(med)).max(floor);
        let corrected = if (rate - med).abs() > threshold {
            med
        } else {
            rate
        };

        if self.primed {
            let innovation = corrected - self.smoothed;
            let excess = (innovation.abs() - MOTION_DEADBAND).max(0.0);
            let alpha_min = 1.0 - (-dt_secs / TAU_MIN_SECS).exp();
            let alpha = (alpha_min + (1.0 - alpha_min) * (excess / MOTION_SCALE).min(1.0))
                .clamp(alpha_min, 1.0);
            self.smoothed += alpha * innovation;
        } else {
            self.smoothed = corrected;
            self.primed = true;
        }
        self.smoothed
    }

    fn median(&self) -> f64 {
        let mut buf = [0.0f64; WINDOW];
        buf[..self.len].copy_from_slice(&self.window[..self.len]);
        let s = &mut buf[..self.len];
        s.sort_unstable_by(f64::total_cmp);
        s[self.len / 2]
    }

    fn mad(&self, med: f64) -> f64 {
        let mut buf = [0.0f64; WINDOW];
        for (b, &w) in buf[..self.len].iter_mut().zip(&self.window[..self.len]) {
            *b = (w - med).abs();
        }
        let s = &mut buf[..self.len];
        s.sort_unstable_by(f64::total_cmp);
        s[self.len / 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: f32 = 1.0;

    fn feed(s: &mut RateSmoother, rates: &[f64]) -> f64 {
        let mut last = 0.0;
        for &r in rates {
            last = s.smooth(r, CONF);
        }
        last
    }

    #[test]
    fn lone_spike_is_rejected() {
        let mut s = RateSmoother::new();
        feed(&mut s, &[1.0; WINDOW]); // settle at unity
        let out = s.smooth(1.6, CONF); // a +60 % impulse
        assert!((out - 1.0).abs() < 0.02, "spike leaked through: {out}");
    }

    #[test]
    fn double_spike_is_rejected() {
        let mut s = RateSmoother::new();
        feed(&mut s, &[1.0; WINDOW]);
        let _ = s.smooth(0.4, CONF);
        let out = s.smooth(0.4, CONF); // two bad blocks, < WINDOW/2
        assert!((out - 1.0).abs() < 0.05, "double spike leaked: {out}");
    }

    #[test]
    fn steady_noise_is_attenuated() {
        let mut s = RateSmoother::new();
        // Deterministic ±0.01 dither around unity.
        let noise: Vec<f64> = (0..200)
            .map(|i| 1.0 + if i % 2 == 0 { 0.01 } else { -0.01 })
            .collect();
        let mut max_dev = 0.0f64;
        for (i, &r) in noise.iter().enumerate() {
            let out = s.smooth(r, CONF);
            // Skip the settle (heavy steady-state pole, τ ≈ 50 blocks);
            // measure the residual ripple once converged.
            if i > 120 {
                max_dev = max_dev.max((out - 1.0).abs());
            }
        }
        assert!(max_dev < 0.004, "noise barely attenuated: {max_dev}");
    }

    #[test]
    fn sustained_ramp_is_tracked() {
        let mut s = RateSmoother::new();
        let mut out = 0.0;
        for i in 0..60 {
            out = s.smooth(1.0 + 0.01 * f64::from(i), CONF);
        }
        // Target is ~1.59; heavy steady-state smoothing lags a gentle
        // ramp, but it must clearly track, not stick near unity.
        assert!(out > 1.45, "ramp not tracked: {out}");
    }

    #[test]
    fn hard_reversal_snaps_quickly() {
        let mut s = RateSmoother::new();
        feed(&mut s, &[1.0; 10]);
        let out = feed(&mut s, &[-1.0; WINDOW]); // full WINDOW of reverse
        assert!(out < -0.9, "reversal did not snap: {out}");
    }

    #[test]
    fn low_confidence_passes_through_raw() {
        let mut s = RateSmoother::new();
        feed(&mut s, &[1.0; WINDOW]);
        let out = s.smooth(0.3, 0.1); // lift: garbage rate, near-zero conf
        assert!(
            (out - 0.3).abs() < 1e-9,
            "low-conf block was altered: {out}"
        );
    }
}
