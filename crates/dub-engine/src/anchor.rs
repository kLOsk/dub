//! Canonical pitch-anchor map: the reason Traktor and Serato read a
//! perfect 0 / +8 / −8 on every deck a DJ ever plays.
//!
//! A DJ's mental model is positional: "the fader is at the detent, so
//! the pitch is 0" — an app showing +0.2 % there reads as broken, even
//! when +0.2 % is the turntable's true speed. And no single trim can
//! fix a real deck: the operator's deck A measures **+0.1–0.2 % at the
//! 0 detent but −0.15…−0.4 % at the ±8 stops** — opposite signs. The
//! commercial apps solve this by *pinning the canonical positions*: a
//! per-deck piecewise-linear warp that maps the rate the deck actually
//! produces at each canonical fader position (−8 %, 0, +8 %) to exactly
//! that canonical value, interpolating between anchors and extending
//! the end slopes beyond them.
//!
//! Properties that make this the right shape:
//! - **Continuous and monotonic** (slopes stay within a few % of 1.0):
//!   no dead zones, no snapping — a deliberate 0.1 % nudge off the
//!   detent registers immediately and proportionally, which is what
//!   keeps fine beatmatch trims usable.
//! - **Invisible**: anchors are learned silently (the zero anchor
//!   during the session-start calibration, the stops whenever the
//!   fader parks against them) — no buttons, no trim readouts.
//! - **Session-scoped**: DJs travel; the map dies with the attach and
//!   is relearned at the next session start.
//!
//! RT-safety: a handful of scalar fields, branch-light arithmetic, no
//! allocation.

/// Canonical anchor slots, as playback rates. Index 0 = −8 %, 1 = 0,
/// 2 = +8 %. Extending to ±16/±50 ranges later means widening these
/// arrays, nothing else.
pub(crate) const CANONICAL_RATES: [f64; 3] = [0.92, 1.0, 1.08];

/// Anchor slot indices, for readability at call sites.
pub(crate) const ANCHOR_MINUS_8: usize = 0;
pub(crate) const ANCHOR_ZERO: usize = 1;
pub(crate) const ANCHOR_PLUS_8: usize = 2;

/// Per-deck canonical anchor map. One per timecode input.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AnchorMap {
    /// Measured rate at each canonical slot; `NaN` = not learned.
    measured: [f64; 3],
}

impl AnchorMap {
    pub(crate) const fn new() -> Self {
        Self {
            measured: [f64::NAN; 3],
        }
    }

    /// Forget everything (re-calibration / fresh session).
    pub(crate) fn reset(&mut self) {
        self.measured = [f64::NAN; 3];
    }

    /// Adopt `measured_rate` as the deck's true rate at canonical
    /// slot `idx`. Rejected (no-op) if it would break the strict
    /// ordering against already-learned neighbours — a misdetected
    /// anchor must never make the warp non-monotonic.
    pub(crate) fn learn(&mut self, idx: usize, measured_rate: f64) {
        debug_assert!(idx < CANONICAL_RATES.len());
        for (i, &m) in self.measured.iter().enumerate() {
            if i == idx || m.is_nan() {
                continue;
            }
            let ordered = if i < idx {
                m < measured_rate
            } else {
                measured_rate < m
            };
            if !ordered {
                return;
            }
        }
        self.measured[idx] = measured_rate;
    }

    /// Whether slot `idx` has been learned. Test-only observer — the
    /// production path never branches on learned-ness (`apply` /
    /// `apply_playback` handle the NaN slots themselves).
    #[cfg(test)]
    pub(crate) fn learned(&self, idx: usize) -> bool {
        !self.measured[idx].is_nan()
    }

    /// Like [`learn`], but a re-adoption meets the prior value halfway
    /// instead of replacing it. Used by the stop anchors: each dwell's
    /// windowed mean still carries a little of the deck's own slow
    /// speed wander, so successive parks converge on the long-run stop
    /// rate rather than jumping to whichever window came last. (The
    /// zero anchor replaces outright — a fresh calibration is
    /// authoritative.)
    pub(crate) fn blend_learn(&mut self, idx: usize, measured_rate: f64) {
        let prior = self.measured[idx];
        let v = if prior.is_nan() {
            measured_rate
        } else {
            f64::midpoint(prior, measured_rate)
        };
        self.learn(idx, v);
    }

    /// Playback-grade correction: the **zero anchor only**, applied
    /// multiplicatively. This is what the audible rate, the policy,
    /// and the groove math consume — it is tiny (≤ the ±0.4 % guard),
    /// learned during the silent session-start hold, and can never
    /// produce an audible pitch step mid-play. The stop anchors are
    /// deliberately excluded here: they carry looser learning gates,
    /// and a misadopted stop must only ever be cosmetic (display
    /// scale), never a playback change.
    pub(crate) fn apply_playback(&self, m: f64) -> f64 {
        let zero = self.measured[ANCHOR_ZERO];
        if zero.is_nan() {
            m
        } else {
            m / zero
        }
    }

    /// Warp a measured rate onto the canonical **display** scale (all
    /// learned anchors).
    ///
    /// - No anchors: identity.
    /// - One anchor: multiplicative correction (`m · c/mᵢ`) — continuous
    ///   everywhere including reverse/scratch rates.
    /// - Two+ anchors: piecewise-linear through the learned points,
    ///   end segments extended with their own slope.
    pub(crate) fn apply(&self, m: f64) -> f64 {
        // Collect learned (measured, canonical) pairs in slot order —
        // `learn` guarantees measured values are strictly increasing.
        let mut pts = [(0.0f64, 0.0f64); 3];
        let mut n = 0;
        for (i, &meas) in self.measured.iter().enumerate() {
            if !meas.is_nan() {
                pts[n] = (meas, CANONICAL_RATES[i]);
                n += 1;
            }
        }
        match n {
            0 => m,
            1 => m * (pts[0].1 / pts[0].0),
            _ => {
                // Find the segment; clamp to the end segments beyond
                // the outermost anchors (their slope extends).
                let mut k = 0;
                while k + 2 < n && m > pts[k + 1].0 {
                    k += 1;
                }
                if n > 2 && m > pts[1].0 {
                    k = n - 2;
                }
                let (m0, c0) = pts[k];
                let (m1, c1) = pts[k + 1];
                c0 + (m - m0) * (c1 - c0) / (m1 - m0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_anchors_is_identity() {
        let a = AnchorMap::new();
        for m in [-1.0, 0.0, 0.92, 1.0, 1.08, 2.5] {
            assert!((a.apply(m) - m).abs() < 1e-12);
        }
    }

    #[test]
    fn zero_anchor_alone_is_multiplicative() {
        let mut a = AnchorMap::new();
        a.learn(ANCHOR_ZERO, 1.002); // deck detent runs +0.2 %
        assert!((a.apply(1.002) - 1.0).abs() < 1e-12, "detent must read 0");
        // The whole scale corrects proportionally, including scratch
        // and reverse rates (continuity everywhere).
        assert!((a.apply(2.004) - 2.0).abs() < 1e-12);
        assert!((a.apply(-1.002) - -1.0).abs() < 1e-12);
    }

    #[test]
    fn three_anchors_pin_all_canonical_positions() {
        // The operator's real deck shape: detent high, stops low.
        let mut a = AnchorMap::new();
        a.learn(ANCHOR_ZERO, 1.001);
        a.learn(ANCHOR_PLUS_8, 1.0785); // +8 stop actually +7.85 %
        a.learn(ANCHOR_MINUS_8, 0.9158); // −8 stop actually −8.42 %
        assert!((a.apply(1.001) - 1.0).abs() < 1e-12);
        assert!((a.apply(1.0785) - 1.08).abs() < 1e-12);
        assert!((a.apply(0.9158) - 0.92).abs() < 1e-12);
        // Mid-range interpolates smoothly: +3 % true ≈ +3.x % canonical,
        // and a fine 0.1 % move stays a ≈0.1 % move (slope ≈ 1).
        let three = a.apply(1.031);
        assert!(
            (three - 1.031).abs() < 0.002,
            "mid-range distorted: {three}"
        );
        let slope = (a.apply(1.0035) - a.apply(1.0025)) / 0.001;
        assert!((slope - 1.0).abs() < 0.05, "fine-move slope off: {slope}");
    }

    #[test]
    fn end_slopes_extend_beyond_the_stops() {
        let mut a = AnchorMap::new();
        a.learn(ANCHOR_ZERO, 1.0);
        a.learn(ANCHOR_PLUS_8, 1.075);
        // Beyond the +8 stop the last segment's slope continues —
        // continuous, monotonic, no kink back to identity.
        let just_below = a.apply(1.0749);
        let at = a.apply(1.075);
        let above = a.apply(1.08);
        assert!(just_below < at && at < above);
        assert!((at - 1.08).abs() < 1e-12);
    }

    #[test]
    fn ordering_violations_are_rejected() {
        let mut a = AnchorMap::new();
        a.learn(ANCHOR_ZERO, 1.001);
        a.learn(ANCHOR_PLUS_8, 0.999); // nonsense: +8 below the detent
        assert!(!a.learned(ANCHOR_PLUS_8), "non-monotonic anchor accepted");
        // And the map still behaves as zero-only.
        assert!((a.apply(1.001) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn blend_learn_converges_instead_of_jumping() {
        let mut a = AnchorMap::new();
        a.blend_learn(ANCHOR_PLUS_8, 1.078); // first park window, a touch low
        a.blend_learn(ANCHOR_PLUS_8, 1.082); // refresh window, a touch high
        // Halfway between, not the last window.
        assert!((a.apply(1.080) - 1.08).abs() < 1e-12);
    }

    #[test]
    fn relearning_updates_an_anchor() {
        let mut a = AnchorMap::new();
        a.learn(ANCHOR_ZERO, 1.002);
        a.learn(ANCHOR_ZERO, 1.001); // temperature drift, re-parked
        assert!((a.apply(1.001) - 1.0).abs() < 1e-12);
    }
}
