//! Per-deck detent (zero) anchor: the one calibration that makes the
//! fader's 0 detent play exactly unity.
//!
//! A DJ's mental model is positional: "the fader is at the detent, so
//! the pitch is 0" — an app showing +0.2 % there reads as broken, even
//! when +0.2 % is the turntable's true speed. A single divisor, measured
//! silently during the session-start calibration (fader at the detent),
//! removes that common-mode offset from BOTH the displayed rate and
//! playback — so the detent reads 0 and a matched BPM equals the played
//! speed.
//!
//! Earlier revisions also pinned the ±8 stops to canonical via a
//! per-deck piecewise warp (Traktor/Serato style). That warp drove the
//! *displayed* rate off the *played* rate — two decks shown the same BPM
//! ran at different speeds — so it was removed (with its stop-learning
//! state machine). xwax (`pl->pitch`) and Mixxx (`m_pRateRatio`) likewise
//! show the one measured rate that plays. Only the zero anchor survives,
//! applied identically to display and playback.
//!
//! RT-safety: one `Option<f64>`, one divide, no allocation.

/// Per-deck zero (detent) anchor. One per timecode input.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AnchorMap {
    /// Measured rate at the fader's 0 detent; `None` until learned.
    zero: Option<f64>,
}

impl AnchorMap {
    pub(crate) const fn new() -> Self {
        Self { zero: None }
    }

    /// Forget the anchor (re-calibration / fresh session).
    pub(crate) fn reset(&mut self) {
        self.zero = None;
    }

    /// Adopt `measured_rate` as the deck's true rate at the 0 detent.
    /// A fresh calibration is authoritative — it replaces outright.
    pub(crate) fn learn_zero(&mut self, measured_rate: f64) {
        self.zero = Some(measured_rate);
    }

    /// Whether the zero anchor has been learned. Test-only observer —
    /// the production path handles the unlearned case inside
    /// [`Self::apply_playback`].
    #[cfg(test)]
    pub(crate) fn zero_learned(&self) -> bool {
        self.zero.is_some()
    }

    /// Divide out the measured detent rate so the fader's 0 plays
    /// exactly unity; identity until learned. The ONLY rate correction
    /// on both the audible and the displayed path (see the module doc on
    /// the removed ±8 warp) — continuous everywhere, scratch and reverse
    /// rates included.
    pub(crate) fn apply_playback(&self, m: f64) -> f64 {
        match self.zero {
            Some(z) => m / z,
            None => m,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlearned_is_identity() {
        let a = AnchorMap::new();
        for m in [-1.0, 0.0, 0.92, 1.0, 1.08, 2.5] {
            assert!((a.apply_playback(m) - m).abs() < 1e-12);
        }
    }

    #[test]
    fn zero_anchor_is_multiplicative_everywhere() {
        let mut a = AnchorMap::new();
        a.learn_zero(1.002); // deck detent runs +0.2 %
        assert!(
            (a.apply_playback(1.002) - 1.0).abs() < 1e-12,
            "detent must read 0"
        );
        // The whole scale corrects proportionally — scratch and reverse
        // rates included (continuity everywhere).
        assert!((a.apply_playback(2.004) - 2.0).abs() < 1e-12);
        assert!((a.apply_playback(-1.002) - -1.0).abs() < 1e-12);
    }

    #[test]
    fn relearning_replaces_the_anchor() {
        let mut a = AnchorMap::new();
        a.learn_zero(1.002);
        a.learn_zero(1.001); // temperature drift, re-parked
        assert!((a.apply_playback(1.001) - 1.0).abs() < 1e-12);
        assert!(a.zero_learned());
    }

    #[test]
    fn reset_forgets_the_anchor() {
        let mut a = AnchorMap::new();
        a.learn_zero(1.01);
        a.reset();
        assert!(!a.zero_learned());
        assert!((a.apply_playback(1.01) - 1.01).abs() < 1e-12);
    }
}
