//! Rev-locked wobble canceller for the pitch / live-BPM **readout**.
//!
//! The dominant residual in a cleanly decoded, de-spiked timecode rate
//! is **once-per-revolution wobble**: an off-center pressing or a warp
//! frequency-modulates the carrier at the platter's rotation rate,
//! swinging the rate by ±0.1–0.3 %. On a 175 BPM track that's the
//! readout flicking ±0.5 BPM even though the platter's *average* speed
//! is rock steady.
//!
//! Generic low-passes can't give both steadiness and feel: an EMA (or
//! a one-rev boxcar) heavy enough to hide 0.55 Hz wobble also makes a
//! fine pitch-fader move — dialing in the last 0.1 BPM of a beatmatch —
//! crawl onto the display over ~2 s. So instead of averaging the wobble
//! away, this filter **cancels** it:
//!
//! - The wobble is periodic in the *record's rotation angle*, and the
//!   decoder's position output is a perfect phase reference: one
//!   physical revolution always advances exactly [`REV_SECS`] of groove
//!   time, at any pitch, in either direction.
//! - An LMS quadrature tracker fits `a·cos φ + b·sin φ` (plus the 2nd
//!   harmonic for warp) to the rate against that phase and subtracts
//!   it. Adaptation converges within a few revolutions and then keeps
//!   tracking continuously.
//! - The cancelled residual only needs a **light** smoother
//!   ([`RESIDUAL_TAU`]) — so a deliberate pitch move of *any* size
//!   reaches the readout in a few hundred ms, and moves beyond
//!   [`SNAP_DEADBAND`] (nudges, scratches, re-locks) snap instantly.
//!
//! The first few revolutions after a fresh attach show the uncancelled
//! wobble while the tracker learns; it fades over ~5 s and stays
//! learned across needle lifts (the phase reference is groove position,
//! which maps to the same physical record angle on every pass).
//!
//! Display-only — playback consumes the [`dub_timecode::RateSmoother`]
//! output directly and is never delayed by this filter.
//!
//! **RT-safety**: a handful of scalar fields, O(1) per update, two
//! `sin_cos` calls per block, no allocation, no locks.

/// Groove time per physical revolution at 33⅓ RPM. (45 RPM pressings
/// also advance 1.8 record-seconds per turn only if cut at 33⅓ — the
/// §5.4.1 RPM work owns adjusting this alongside detection.)
const REV_SECS: f64 = 1.8;

/// Time constant of the light post-cancellation smoother. Small enough
/// that dialing in 0.1 BPM reads back within ~0.3 s.
const RESIDUAL_TAU: f64 = 0.15;

/// LMS adaptation time constant — a few revolutions to learn a
/// pressing's wobble, slow enough that noise doesn't shake the fit.
const LMS_TAU: f64 = 1.5;

/// Time constant of the slow mean tracker that anchors the LMS error.
/// Long vs a revolution (so the wobble stays in the error term for the
/// tracker to learn), short vs a set (so pitch drifts don't linger).
const BASELINE_TAU: f64 = 2.0;

/// Innovation (vs the smoothed display, normalized rate) beyond which
/// the platter is genuinely being moved: snap the display and pause
/// adaptation so nudges/scratches neither lag nor corrupt the fit.
/// 0.75 % sits well above eccentricity wobble (≤ ~0.3 %) and well below
/// any deliberate beatmatch nudge.
const SNAP_DEADBAND: f64 = 0.0075;

/// Rev-locked LMS wobble canceller + light smoother + snap gate.
/// One per deck.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DisplayRateFilter {
    /// Quadrature wobble fit: fundamental (`a1·cos φ + b1·sin φ`) and
    /// 2nd harmonic (`a2`, `b2`), φ = groove position × 2π / rev.
    a1: f64,
    b1: f64,
    a2: f64,
    b2: f64,
    /// Smoothed cancelled rate — the displayed value.
    smoothed: f64,
    /// Slow mean tracker ([`BASELINE_TAU`]) used as the LMS error
    /// reference. The fast display smoother can't serve that role: it
    /// *follows* most of the wobble, leaving only a sliver (phase-
    /// rotated, too) for the tracker to learn from, which stalls
    /// convergence ~10×. A slow baseline keeps nearly the full wobble
    /// amplitude in the error term.
    baseline: f64,
    /// `false` until the first update after construction / [`reset`] —
    /// the next update snaps instead of slewing. The wobble fit is
    /// *not* cleared by a reset: groove position maps to the same
    /// physical record angle on every pass, so the fit stays valid
    /// across lifts and needle drops.
    primed: bool,
    /// Total locked, adapting time the fit has accumulated. Drives
    /// [`Self::settled`] so the UI can mark the readout as still
    /// measuring during the first revolutions of a session instead of
    /// presenting the uncancelled wobble as truth.
    adapted_secs: f64,
}

/// Adaptation time after which the wobble fit is trustworthy — a bit
/// over two revolutions at 33⅓, matching the LMS convergence.
const SETTLE_SECS: f64 = 4.0;

impl DisplayRateFilter {
    pub(crate) const fn new() -> Self {
        Self {
            a1: 0.0,
            b1: 0.0,
            a2: 0.0,
            b2: 0.0,
            smoothed: 1.0,
            baseline: 1.0,
            primed: false,
            adapted_secs: 0.0,
        }
    }

    /// Whether the wobble fit has accumulated enough locked play to be
    /// trustworthy. Latches per session (the fit survives lifts).
    pub(crate) fn settled(&self) -> bool {
        self.adapted_secs >= SETTLE_SECS
    }

    /// Make the next update snap to the incoming rate. Called on lift /
    /// dropout, mirroring the freeze-then-snap display policy.
    pub(crate) fn reset(&mut self) {
        self.primed = false;
    }

    /// Fold one block's smoothed `rate` (spanning `dt` seconds, with the
    /// decoder's groove position at `position_secs`) into the filter and
    /// return the current display value.
    pub(crate) fn update(&mut self, rate: f64, position_secs: f64, dt: f64) -> f64 {
        let phase = std::f64::consts::TAU * (position_secs.rem_euclid(REV_SECS) / REV_SECS);
        let (s1, c1) = phase.sin_cos();
        let (s2, c2) = (2.0 * phase).sin_cos();
        let wobble = self.a1 * c1 + self.b1 * s1 + self.a2 * c2 + self.b2 * s2;
        let cancelled = rate - wobble;

        if !self.primed {
            self.smoothed = cancelled;
            self.baseline = cancelled;
            self.primed = true;
            return self.smoothed;
        }

        let innovation = cancelled - self.smoothed;
        if innovation.abs() > SNAP_DEADBAND {
            // A real move. Track it instantly; don't adapt the wobble
            // fit on transient error.
            self.smoothed = cancelled;
            self.baseline = cancelled;
            return self.smoothed;
        }

        // Steady platter: adapt the fit toward whatever rev-periodic
        // component remains (error vs the slow baseline, which holds
        // the wobble nearly in full), then ease the cancelled rate
        // into the display.
        let err = cancelled - self.baseline;
        self.adapted_secs += dt;
        let mu = dt / LMS_TAU;
        self.a1 += mu * err * c1;
        self.b1 += mu * err * s1;
        self.a2 += mu * err * c2;
        self.b2 += mu * err * s2;
        self.baseline += (dt / BASELINE_TAU) * err;
        self.smoothed += (dt / RESIDUAL_TAU) * innovation;
        self.smoothed
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
mod tests {
    use super::*;

    /// 64-frame blocks at 48 kHz — the production cadence.
    const DT: f64 = 64.0 / 48_000.0;

    /// Drive the filter for `secs` with the platter at `rate_at(t)`,
    /// adding once-per-rev wobble of amplitude `wob1` and 2nd-harmonic
    /// `wob2`, integrating groove position like the decoder does.
    /// Returns (last display value, ripple over the final `judge` secs).
    fn run(
        f: &mut DisplayRateFilter,
        secs: f64,
        judge: f64,
        wob1: f64,
        wob2: f64,
        rate_at: impl Fn(f64) -> f64,
    ) -> (f64, f64) {
        let blocks = (secs / DT) as usize;
        let mut pos = 0.0f64;
        let (mut last, mut min, mut max) = (1.0f64, f64::MAX, f64::MIN);
        for b in 0..blocks {
            let t = b as f64 * DT;
            let phase = std::f64::consts::TAU * (pos.rem_euclid(REV_SECS) / REV_SECS);
            let rate = rate_at(t) + wob1 * phase.sin() + wob2 * (2.0 * phase + 0.7).sin();
            last = f.update(rate, pos, DT);
            pos += rate * DT;
            if t > secs - judge {
                min = min.min(last);
                max = max.max(last);
            }
        }
        (last, max - min)
    }

    #[test]
    fn wobble_is_cancelled_after_adaptation() {
        // ±0.3 % once-per-rev FM — a visibly off-center pressing.
        let mut f = DisplayRateFilter::new();
        let (_, ripple) = run(&mut f, 15.0, 3.0, 0.003, 0.0, |_| 1.0);
        assert!(
            ripple < 2e-4,
            "once-per-rev wobble survived cancellation: ripple = {ripple}"
        );
    }

    #[test]
    fn warp_harmonic_is_cancelled_too() {
        let mut f = DisplayRateFilter::new();
        let (_, ripple) = run(&mut f, 15.0, 3.0, 0.002, 0.002, |_| 1.0);
        assert!(ripple < 3e-4, "harmonic wobble survived: ripple = {ripple}");
    }

    #[test]
    fn tenth_of_a_bpm_dial_in_reads_back_fast() {
        // The beatmatch endgame: +0.057 % (0.1 BPM on a 175 BPM tune),
        // far below the snap gate, on a pressing with real wobble. The
        // readout must reflect ≥ 80 % of the move within half a second
        // (the smoother alone reaches 96 %; the rest is cancelled-
        // wobble ripple at the sampling instant) — not crawl over a
        // whole revolution like a boxcar would (28 % in this window).
        let mut f = DisplayRateFilter::new();
        run(&mut f, 15.0, 3.0, 0.003, 0.0, |_| 1.0);
        let step = 0.00057;
        let (out, _) = run(&mut f, 0.5, 0.5, 0.003, 0.0, |_| 1.0 + step);
        assert!(
            out - 1.0 > 0.8 * step,
            "fine dial-in too sluggish: moved {} of {step}",
            out - 1.0
        );
    }

    #[test]
    fn big_move_snaps_immediately() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.002, 0.0, |_| 1.0);
        // A 5 % nudge — way past the deadband — must show on the very
        // next update.
        let out = f.update(1.05, 0.0, DT);
        assert!((out - 1.05).abs() < 3e-3, "nudge lagged: {out}");
    }

    #[test]
    fn wobble_fit_survives_a_lift() {
        // Learn the wobble, lift (reset), re-drop: the fit must still
        // cancel immediately — no fresh multi-second re-adaptation.
        let mut f = DisplayRateFilter::new();
        run(&mut f, 15.0, 3.0, 0.003, 0.0, |_| 1.0);
        f.reset();
        let (_, ripple) = run(&mut f, 2.0, 1.5, 0.003, 0.0, |_| 1.0);
        assert!(
            ripple < 3e-4,
            "wobble fit lost across a lift: ripple = {ripple}"
        );
    }

    #[test]
    fn reset_makes_next_update_snap() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.0, 0.0, |_| 1.0);
        f.reset();
        let out = f.update(0.92, 0.0, DT);
        assert!((out - 0.92).abs() < 1e-12, "re-lock did not snap: {out}");
    }

    #[test]
    fn settles_after_two_revolutions_and_latches_across_lifts() {
        let mut f = DisplayRateFilter::new();
        assert!(!f.settled());
        run(&mut f, 2.0, 1.0, 0.003, 0.0, |_| 1.0);
        assert!(!f.settled(), "settled too early");
        run(&mut f, 3.0, 1.0, 0.003, 0.0, |_| 1.0);
        assert!(f.settled(), "should be settled after ~5 s of lock");
        f.reset(); // lift — the fit (and its maturity) persist
        assert!(f.settled(), "lift must not reset the settled state");
    }

    #[test]
    fn unprimed_first_update_returns_input() {
        let mut f = DisplayRateFilter::new();
        let out = f.update(1.35, 0.0, DT);
        assert!((out - 1.35).abs() < 1e-12);
    }
}
