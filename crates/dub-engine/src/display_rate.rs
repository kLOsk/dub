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
//! - The cancelled value then passes a **median-of-[`MED_WIN`]
//!   prefilter** (decode spikes from a dirty needle — 1–2 quanta, any
//!   amplitude — never reach the readout) and a **persistence-scaled
//!   pole**: slow ([`TAU_STEADY`]) while the value holds, opening
//!   toward snappy ([`TAU_FAST`]) as a sign-consistent change sustains
//!   [`MOVE_CONFIRM_SECS`] — a hand on the fader sustains, noise
//!   doesn't. The opening is **continuous in the response-rate domain
//!   (∝ urgency²)**, never a regime flip: a τ-domain cliff made the
//!   digits stall through the confirmation window and then lunge.
//!   Amplitude-based snap gates are deliberately gone: they passed
//!   exactly the spikes they were meant to ignore.
//! - A **trend detector** short-circuits that confirmation clock for
//!   big deliberate moves (jumping the pitch to a target BPM): a
//!   fast-minus-slow EMA pair over the median output measures *net
//!   sustained displacement*. Real-capture data killed the obvious
//!   slope detector: a dirty deck emits multi-quantum decode bursts
//!   (0.5–1.5 % over 50–150 ms — too wide for the median, monotone,
//!   steeper than a fader move's early phase), so the confirm
//!   threshold sits **above** the burst scale ([`TREND_CONFIRM_DISP`],
//!   measured 1.8× the worst steady-state excursion) — bursts revert
//!   before accumulating that much, a hand on the fader doesn't.
//!   Hard-confirm drops the pole to [`TAU_MOVE`] while the trend and
//!   the catch-up gap last. A decaying peak-hold of the innovation
//!   ([`CALM_MAX_INNOVATION`]) keeps the fast path exclusive to moves
//!   that *start from steady tracking*: scratching saturates it, so
//!   post-scratch settles glide in on the ordinary clock instead of
//!   slamming onto every turntable transient.
//! - After a lift/re-lock the display holds its frozen value until the
//!   median window refills (~50 ms), then reseeds from the median —
//!   never from a single block, whose first re-locked value is often
//!   turnaround garbage.
//!
//! The first few revolutions after a fresh attach show the uncancelled
//! wobble while the tracker learns; it fades over ~5 s (the published
//! `settled` state dims the readout until then) and stays learned
//! across needle lifts (the phase reference is groove position, which
//! maps to the same physical record angle on every pass).
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

/// LMS adaptation time constant — a few revolutions to learn a
/// pressing's wobble, slow enough that noise doesn't shake the fit.
const LMS_TAU: f64 = 1.5;

/// Time constant of the slow mean tracker that anchors the LMS error.
/// Long vs a revolution (so the wobble stays in the error term for the
/// tracker to learn), short vs a set (so pitch drifts don't linger).
const BASELINE_TAU: f64 = 2.0;

/// Innovation beyond which the wobble fit pauses adapting (transient
/// error must not corrupt the learned ellipse).
const ADAPT_GUARD: f64 = 0.0075;

/// Display median prefilter length (render quanta, ~75 ms at 512
/// frames). Decode spikes from a dirty needle last 1–3 quanta; the
/// median removes them entirely, at any amplitude, before the pole
/// ever sees them — amplitude-based snap gates let exactly those
/// spikes through (the on-rig "jumps by a whole BPM and quickly
/// back").
const MED_WIN: usize = 9;

/// Innovation below this is presence noise; above it, *sustained*, is
/// a real fader move. ~0.1 % ≈ 0.17 BPM at 175 — moves smaller than
/// that ease in on the steady pole (jitter immunity wins, per the
/// operator: "smoothen this much harder").
const MOVE_NOISE_BAND: f64 = 0.002;

/// Sign-consistent out-of-band time required to confirm a real move.
/// Spikes last a few quanta and alternate; a hand on the fader
/// sustains.
const MOVE_CONFIRM_SECS: f64 = 0.6;

/// Pole while steady — slow, per the operator's spec: "when the
/// pitch fader is steady it can be very slow to move". Was 3.0 when
/// this pole carried the anti-jitter load alone; with the wobble
/// canceller + median in front, 1.5 keeps the readout calm while a
/// beatmatch dial-in (sub-noise-band by definition) lands in ~3 s
/// instead of ~8 s ("slow to update", on-rig).
const TAU_STEADY: f64 = 1.5;

/// Pole once a move is confirmed — "fast and snappy".
const TAU_FAST: f64 = 0.08;

/// Pole while a *hard-confirmed* move is in progress (trend detector
/// fired and the display is still catching up) — tighter than
/// [`TAU_FAST`] so a deliberate pitch jump reads back almost as fast
/// as the decoder reports it.
const TAU_MOVE: f64 = 0.03;

/// Fast EMA of the trend pair — short enough to ride a fader move,
/// long enough that the (already median-filtered) value can't whip it
/// around quantum-to-quantum.
const TREND_EMA_FAST_TAU: f64 = 0.03;

/// Slow EMA of the trend pair — the recent-baseline reference. For a
/// sustained ramp of slope `m` the pair separates by
/// `m × (slow − fast)` ≈ 0.22 s × m, so it measures *net sustained
/// displacement* and forgets transients on its own.
const TREND_EMA_SLOW_TAU: f64 = 0.25;

/// Fast−slow separation that hard-confirms a move. Tuned on the real
/// deck-A capture: dirty-needle decode bursts (multi-quantum, survive
/// the median) separate the pair by ≤ 0.9 %, the hand-landing wobble
/// before a move by ≤ 0.8 %, while genuine fader moves reach 2.5–6 %.
/// 1.6 % keeps ≥ 1.8× margin against every non-move event on the
/// capture.
const TREND_CONFIRM_DISP: f64 = 0.016;

/// The trend condition must hold contiguously this long before it
/// confirms — a one-quantum graze of the threshold is not a move.
const TREND_SUSTAIN_SECS: f64 = 0.03;

/// Hard tracking also requires the display to actually be behind by
/// this much. Below it the ordinary pole has the move covered, and a
/// completed move releases the fast pole instead of riding the trend
/// pair's decay tail across the next few platter transients.
const TREND_MIN_INNOVATION: f64 = 0.005;

/// Decaying peak-hold of |innovation| above which the deck is *not*
/// steady (scratching, post-scratch settle, fresh re-lock churn). A
/// trend run only qualifies for the hard-confirm path when it starts
/// below this — fader moves depart from steady tracking (innovation
/// ≪ 1 %), scratch swings leave tens of % behind for a second or two.
const CALM_MAX_INNOVATION: f64 = 0.04;

/// Decay time of that peak-hold — roughly how long after a scratch
/// the fast path stays locked out.
const CALM_DECAY_TAU: f64 = 1.0;

/// Rev-locked LMS wobble canceller + median prefilter + a
/// persistence-scaled pole (slow when steady, fast once a move is
/// confirmed). One per deck.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DisplayRateFilter {
    /// Quadrature wobble fit: fundamental + harmonics 2–4 (a sharp
    /// once-per-rev feature — a pressing seam, a warp crest — is far
    /// from sinusoidal, and two harmonics left a rev-locked residual
    /// of ~±0.2 % on a real capture), φ = groove position × 2π / rev.
    a1: f64,
    b1: f64,
    a2: f64,
    b2: f64,
    a3: f64,
    b3: f64,
    a4: f64,
    b4: f64,
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
    /// Median prefilter ring over the wobble-cancelled rate.
    med_win: [f64; MED_WIN],
    med_len: usize,
    med_pos: usize,
    /// Sign-consistent time spent outside [`MOVE_NOISE_BAND`] — the
    /// move-confirmation clock.
    off_secs: f64,
    /// Sign of the current out-of-band run (+1.0 / −1.0).
    off_sign: f64,
    /// Fast / slow EMA pair over the median output — the trend
    /// detector's displacement measure.
    trend_fast: f64,
    trend_slow: f64,
    /// Contiguous time the trend condition (separation ≥
    /// [`TREND_CONFIRM_DISP`], innovation aligned and ≥
    /// [`TREND_MIN_INNOVATION`]) has held — only accumulated for runs
    /// that start from steady tracking ([`Self::run_calm`]).
    trend_secs: f64,
    /// Steadiness snapshot taken when the current *out-of-band
    /// innovation run* began. The trend condition matures several
    /// quanta into a move, by which time a fast fader slam's own
    /// innovation has already saturated the peak-hold — the move must
    /// be judged by the calm that preceded it, not by itself.
    run_calm: bool,
    /// Whether the previous update's innovation was inside the noise
    /// band — detects out-of-band run onsets.
    in_band_prev: bool,
    /// Decaying peak-hold of |innovation| — the steadiness measure
    /// gating the hard-confirm path. Survives [`Self::reset`]: a lift
    /// or scratch gap is exactly when the fast path must stay locked
    /// out.
    chaos: f64,
    /// Number of update quanta spent in hard-confirmed (trend) mode —
    /// instrumentation for the unit tests and the replay harness.
    #[cfg(test)]
    hard_quanta: u32,
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
            a3: 0.0,
            b3: 0.0,
            a4: 0.0,
            b4: 0.0,
            smoothed: 1.0,
            baseline: 1.0,
            primed: false,
            adapted_secs: 0.0,
            med_win: [0.0; MED_WIN],
            med_len: 0,
            med_pos: 0,
            off_secs: 0.0,
            off_sign: 0.0,
            trend_fast: 1.0,
            trend_slow: 1.0,
            trend_secs: 0.0,
            run_calm: true,
            in_band_prev: true,
            chaos: 0.0,
            #[cfg(test)]
            hard_quanta: 0,
        }
    }

    /// Whether the wobble fit has accumulated enough locked play to be
    /// trustworthy. Latches per session (the fit survives lifts).
    pub(crate) fn settled(&self) -> bool {
        self.adapted_secs >= SETTLE_SECS
    }

    /// Measurement progress in [0, 1] — drives the deck-header
    /// calibration progress line so the DJ sees the session-start
    /// hold advancing instead of guessing whether it hung.
    pub(crate) fn settle_progress(&self) -> f64 {
        (self.adapted_secs / SETTLE_SECS).min(1.0)
    }

    /// Make the next update snap to the incoming rate. Called on lift /
    /// dropout, mirroring the freeze-then-snap display policy.
    pub(crate) fn reset(&mut self) {
        self.primed = false;
        self.med_len = 0;
        self.med_pos = 0;
        self.off_secs = 0.0;
        self.off_sign = 0.0;
        self.trend_secs = 0.0;
        self.in_band_prev = true;
        // `chaos` deliberately survives: the disturbance that caused
        // the reset is exactly the kind of churn the hard-confirm
        // path must wait out. The trend EMAs reseed on re-prime.
    }

    /// Fold one block's smoothed `rate` (spanning `dt` seconds, with the
    /// decoder's groove position at `position_secs`) into the filter and
    /// return the current display value.
    pub(crate) fn update(&mut self, rate: f64, position_secs: f64, dt: f64) -> f64 {
        let phase = std::f64::consts::TAU * (position_secs.rem_euclid(REV_SECS) / REV_SECS);
        let (s1, c1) = phase.sin_cos();
        let (s2, c2) = (2.0 * phase).sin_cos();
        let (s3, c3) = (3.0 * phase).sin_cos();
        let (s4, c4) = (4.0 * phase).sin_cos();
        let wobble = self.a1 * c1
            + self.b1 * s1
            + self.a2 * c2
            + self.b2 * s2
            + self.a3 * c3
            + self.b3 * s3
            + self.a4 * c4
            + self.b4 * s4;
        let cancelled = rate - wobble;

        // Median prefilter: a decode spike from a dirty needle lasts a
        // quantum or two; the median deletes it at any amplitude
        // before the pole ever sees it.
        self.med_win[self.med_pos] = cancelled;
        self.med_pos = (self.med_pos + 1) % MED_WIN;
        if self.med_len < MED_WIN {
            self.med_len += 1;
        }
        let filtered = self.median();

        if !self.primed {
            // Re-lock (or fresh attach): hold the frozen display value
            // until the median window refills, then seed from the
            // median. Seeding from the *first* locked block let one
            // turnaround garbage block flash straight onto the readout
            // (on-rig: pitch flicking −119 % → −46 % → steady at every
            // stop/start). The ~50 ms of extra freeze is invisible.
            if self.med_len < MED_WIN {
                return self.smoothed;
            }
            self.smoothed = filtered;
            self.baseline = filtered;
            self.trend_fast = filtered;
            self.trend_slow = filtered;
            self.primed = true;
            return self.smoothed;
        }

        // Persistence-scaled pole: noise doesn't sustain, a hand on
        // the fader does. Sign-consistent out-of-band time opens the
        // pole from very-slow to snappy; anything shorter than the
        // confirmation window only ever meets the steady pole.
        let innovation = filtered - self.smoothed;
        // Steadiness snapshot *before* this quantum's innovation: a
        // move's own departure must not disqualify itself.
        let calm = self.chaos < CALM_MAX_INNOVATION;
        self.chaos = (self.chaos * (-dt / CALM_DECAY_TAU).exp()).max(innovation.abs());
        if innovation.abs() > MOVE_NOISE_BAND {
            let sign = innovation.signum();
            let same_sign = (sign - self.off_sign).abs() < f64::EPSILON;
            if self.in_band_prev || !same_sign {
                self.run_calm = calm;
            }
            if same_sign {
                self.off_secs += dt;
            } else {
                // A flip halves the clock rather than restarting it:
                // alternating noise still never confirms, but a real
                // move whose innovation rides the band edge (a 0.1 BPM
                // dial-in with residual ripple) accumulates net
                // progress instead of being reset by every ripple.
                self.off_sign = sign;
                self.off_secs = (self.off_secs * 0.5).max(dt);
            }
            self.in_band_prev = false;
        } else {
            self.off_secs = (self.off_secs - 2.0 * dt).max(0.0);
            self.in_band_prev = true;
        }

        // Trend detector: net sustained displacement of the median
        // output, as the separation of a fast/slow EMA pair. Confirms
        // a deliberate move long before the persistence clock — but
        // only past burst scale, with the display genuinely behind,
        // and only when the run departed from steady tracking.
        self.trend_fast += (1.0 - (-dt / TREND_EMA_FAST_TAU).exp()) * (filtered - self.trend_fast);
        self.trend_slow += (1.0 - (-dt / TREND_EMA_SLOW_TAU).exp()) * (filtered - self.trend_slow);
        let trend = self.trend_fast - self.trend_slow;
        let trending = trend.abs() >= TREND_CONFIRM_DISP
            && innovation.abs() >= TREND_MIN_INNOVATION
            && (trend > 0.0) == (innovation > 0.0);
        // A run only starts counting if it departed from steady
        // tracking; `run_calm` cannot change while the condition
        // holds (that would need an innovation-run restart, which
        // breaks the condition first), so gating the onset suffices.
        if trending && (self.trend_secs > 0.0 || self.run_calm) {
            self.trend_secs += dt;
        } else {
            self.trend_secs = 0.0;
        }
        let hard = self.trend_secs >= TREND_SUSTAIN_SECS;
        #[cfg(test)]
        {
            self.hard_quanta += u32::from(hard);
        }
        let gain = if hard {
            // Saturate the clock so the ordinary fast pole carries
            // the tail of the move once the trend condition releases.
            self.off_secs = self.off_secs.max(MOVE_CONFIRM_SECS);
            1.0 / TAU_MOVE
        } else {
            // Interpolate in the *response-rate* domain (1/τ), with a
            // quadratic urgency curve. τ-linear interpolation packed
            // nearly all of the speed-up into the last sliver of the
            // confirmation clock — on the rig the digits stalled
            // through the window, then lunged ("jumpy and slow at the
            // same time"). Rate-linear-in-u² builds responsiveness
            // progressively while keeping the low-urgency end (where
            // decode bursts briefly accumulate clock) close to steady.
            let urgency = (self.off_secs / MOVE_CONFIRM_SECS).clamp(0.0, 1.0);
            1.0 / TAU_STEADY + (1.0 / TAU_FAST - 1.0 / TAU_STEADY) * urgency * urgency
        };
        self.smoothed += (1.0 - (-dt * gain).exp()) * innovation;

        // Adapt the wobble fit whenever the error is small enough to
        // be wobble rather than a deliberate move. Deliberately NOT
        // gated on the move-confirmation clock: the uncancelled wobble
        // itself sustains sign for half a revolution and trips that
        // clock during the learning phase — gating on it deadlocks the
        // learning the canceller needs to make the wobble disappear.
        let err = cancelled - self.baseline;
        if err.abs() < ADAPT_GUARD {
            self.adapted_secs += dt;
            let mu = dt / LMS_TAU;
            self.a1 += mu * err * c1;
            self.b1 += mu * err * s1;
            self.a2 += mu * err * c2;
            self.b2 += mu * err * s2;
            self.a3 += mu * err * c3;
            self.b3 += mu * err * s3;
            self.a4 += mu * err * c4;
            self.b4 += mu * err * s4;
        }
        self.baseline += (dt / BASELINE_TAU) * err;
        self.smoothed
    }

    fn median(&self) -> f64 {
        let mut buf = [0.0f64; MED_WIN];
        buf[..self.med_len].copy_from_slice(&self.med_win[..self.med_len]);
        let s = &mut buf[..self.med_len];
        s.sort_unstable_by(f64::total_cmp);
        s[self.med_len / 2]
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
        // The beatmatch endgame: +0.057 % (0.1 BPM on a 175 BPM tune)
        // on a pressing with real wobble. A move this size is far
        // below the noise band, so by design it eases in on the very
        // slow steady pole (tuned against a real deck-A capture:
        // jitter immunity wins, per the operator's "smoothen much
        // harder" spec) — but it must actually get there, not stall.
        let mut f = DisplayRateFilter::new();
        run(&mut f, 15.0, 3.0, 0.003, 0.0, |_| 1.0);
        let step = 0.00057;
        let (out, _) = run(&mut f, 1.5, 0.2, 0.003, 0.0, |_| 1.0 + step);
        assert!(
            out - 1.0 > 0.3 * step,
            "fine dial-in too sluggish: moved {} of {step}",
            out - 1.0
        );
        let (out, _) = run(&mut f, 8.0, 0.2, 0.003, 0.0, |_| 1.0 + step);
        assert!(
            out - 1.0 > 0.9 * step,
            "fine dial-in never converged: at {} of {step}",
            out - 1.0
        );
    }

    /// The on-rig "jumpy and slow at the same time": a medium fader
    /// move (below the trend detector's hard-confirm scale) must read
    /// back *progressively* — measurable progress mid-confirmation,
    /// convergence soon after, and never a stall-then-lunge step. The
    /// τ-domain interpolation this replaces showed ~5 % of the move at
    /// 0.45 s and then covered the rest almost at once.
    #[test]
    fn medium_move_reads_back_progressively_without_lunge() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 8.0, 2.0, 0.002, 0.0, |_| 1.0);
        f.hard_quanta = 0;
        let blocks = (2.0 / DT) as usize;
        let mut pos = 8.0f64;
        let (mut prev, mut max_step) = (f.smoothed, 0.0f64);
        let (mut at_045, mut at_090) = (f64::NAN, f64::NAN);
        for b in 0..blocks {
            let t = b as f64 * DT;
            // Hand-like ramp: +1.2 % over 0.4 s, then hold.
            let rate = 1.0 + 0.012 * (t / 0.4).min(1.0);
            let out = f.update(rate, pos, DT);
            pos += rate * DT;
            max_step = max_step.max((out - prev).abs());
            prev = out;
            if at_045.is_nan() && t >= 0.45 {
                at_045 = out;
            }
            if at_090.is_nan() && t >= 0.90 {
                at_090 = out;
            }
        }
        assert_eq!(
            f.hard_quanta, 0,
            "medium move must stay on the ordinary path"
        );
        assert!(
            at_045 - 1.0 > 0.012 * 0.25,
            "stalled mid-confirmation: {:.4} of 0.012 at 0.45 s",
            at_045 - 1.0
        );
        assert!(
            at_090 - 1.0 > 0.012 * 0.8,
            "too slow to land: {:.4} of 0.012 at 0.90 s",
            at_090 - 1.0
        );
        assert!(
            max_step < 0.012 * 0.2,
            "stall-then-lunge: max per-quantum step {max_step:.5} on a 0.012 move"
        );
    }

    #[test]
    fn big_move_confirms_and_tracks_quickly() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.002, 0.0, |_| 1.0);
        // A 5 % nudge sustains, so the trend detector hard-confirms
        // it (median refill + sustain, ≈ 50 ms) and [`TAU_MOVE`]
        // catches it — within ~0.3 s the readout is essentially
        // there (the persistence clock alone took ~1.3 s). It still
        // deliberately does NOT show on the very next update: that
        // instant-snap path is what let decode spikes through as
        // whole-BPM display jumps.
        let pre = f.smoothed;
        let _ = f.update(1.05, 5.0, DT);
        let snapped = f.update(1.05, 5.0 + DT, DT);
        assert!(
            (snapped - pre).abs() < 1e-3,
            "step showed before the median window could vet it"
        );
        let (out, _) = run(&mut f, 0.3, 0.05, 0.002, 0.0, |_| 1.05);
        assert!((out - 1.05).abs() < 4e-3, "nudge lagged: {out}");
        assert!(f.hard_quanta > 0, "the nudge never hard-confirmed");
    }

    /// Fader-move readback contract, modelled on the real deck-A
    /// capture's 0 → +8 % move (≈13 %/s fader ramp): the trend
    /// detector must hard-confirm mid-ramp so the displayed value
    /// crosses +7.2 % within ≤ 0.75 s of the ramp start — the raw
    /// rate itself only crosses at ≈ 0.55 s, so the readout trails
    /// the decoder by under 0.2 s.
    #[test]
    fn trend_detector_confirms_a_fader_ramp_fast() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 10.0, 2.0, 0.002, 0.0, |_| 1.0);
        let blocks = (1.2 / DT) as usize;
        let mut pos = 10.0f64;
        let mut crossed = f64::NAN;
        for b in 0..blocks {
            let t = b as f64 * DT;
            let rate = 1.0 + 0.13 * t.min(0.6);
            let out = f.update(rate, pos, DT);
            pos += rate * DT;
            if crossed.is_nan() && out >= 1.072 {
                crossed = t;
            }
        }
        assert!(f.hard_quanta > 0, "ramp never hard-confirmed");
        assert!(
            crossed <= 0.75,
            "0→+8 readback too slow: crossed +7.2 % at {crossed:.3} s"
        );
    }

    /// Platter undulation — the slow speed oscillation a worn deck
    /// shows even when "steady" (quasi-period ≈ 1.8 s) — must never
    /// trip the hard-confirm path. Driven at 3× the slope measured on
    /// the real capture (1.5 %/s vs ~0.5 %/s) for margin, with a
    /// period detuned from the rev so the wobble canceller can't
    /// simply learn it away.
    #[test]
    fn platter_undulation_never_trend_confirms() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.0, 0.0, |_| 1.0);
        f.hard_quanta = 0;
        let blocks = (20.0 / DT) as usize;
        let mut pos = 5.0f64;
        for b in 0..blocks {
            let t = b as f64 * DT;
            let rate = 1.0 + 0.0045 * (std::f64::consts::TAU * t / 1.83).sin();
            let _ = f.update(rate, pos, DT);
            pos += rate * DT;
        }
        assert_eq!(
            f.hard_quanta, 0,
            "platter undulation tripped the fast move path"
        );
    }

    /// Dirty-needle decode bursts — the multi-quantum excursions seen
    /// all over the real capture's steady segments (0.5–1.5 % for
    /// 50–150 ms, too wide for the median, monotone like a move's
    /// onset) — must neither hard-confirm nor visibly move the
    /// readout.
    #[test]
    fn decode_bursts_never_trend_confirm() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.0, 0.0, |_| 1.0);
        f.hard_quanta = 0;
        let blocks = (10.0 / DT) as usize;
        let mut pos = 5.0f64;
        let mut worst = 0.0f64;
        for b in 0..blocks {
            let t = b as f64 * DT;
            // Every second, a 90 ms one-sided −1.5 % burst.
            let rate = if t.fract() < 0.09 { 0.985 } else { 1.0 };
            let out = f.update(rate, pos, DT);
            pos += rate * DT;
            worst = worst.max((out - 1.0).abs());
        }
        assert_eq!(f.hard_quanta, 0, "decode bursts tripped the fast move path");
        // The slow pole still eases fractionally toward a burst (same
        // as before the trend detector existed); the contract is that
        // the *fast* path never engages and the residual stays well
        // under the real capture's steady-state span (~0.26 %).
        assert!(worst < 2.5e-3, "decode bursts reached the readout: {worst}");
    }

    /// After scratching, the settle back to steady must glide in on
    /// the ordinary persistence clock — the fast path stays locked
    /// out by the calm gate (the post-scratch chase otherwise slams
    /// the readout onto every settling transient; on the real capture
    /// that doubled the t=45–59 steady span).
    #[test]
    fn scratch_settle_keeps_the_ordinary_pole() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.0, 0.0, |_| 1.0);
        let blocks = (3.0 / DT) as usize;
        let mut pos = 5.0f64;
        for b in 0..blocks {
            let t = b as f64 * DT;
            // ±30 % square-ish scratch swings, 4 Hz.
            let rate = if (t * 4.0).fract() < 0.5 { 1.3 } else { 0.7 };
            let _ = f.update(rate, pos, DT);
            pos += rate * DT;
        }
        let at_settle = f.hard_quanta;
        let mut out = 0.0;
        for _ in 0..((1.5 / DT) as usize) {
            out = f.update(1.03, pos, DT);
            pos += 1.03 * DT;
        }
        assert_eq!(
            f.hard_quanta, at_settle,
            "post-scratch settle engaged the fast move path"
        );
        assert!(
            (out - 1.03).abs() < 5e-3,
            "settle never converged on the clock pole: {out}"
        );
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
    fn reset_reseeds_from_median_within_a_window() {
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.0, 0.0, |_| 1.0);
        f.reset();
        // The display holds the frozen value while the median window
        // refills (≈50 ms), then seeds — fast BPM recovery after a
        // lift without trusting any single block.
        let mut out = 0.0;
        for _ in 0..MED_WIN {
            out = f.update(0.92, 0.0, DT);
        }
        assert!((out - 0.92).abs() < 1e-12, "re-lock did not reseed: {out}");
    }

    #[test]
    fn relock_garbage_block_never_reaches_the_readout() {
        // On-rig: at every stop/start the readout flashed −119 % →
        // −46 % → steady, because the post-reset seed trusted the
        // first locked block — a turnaround garbage block. The reseed
        // now goes through the median, so the flash must be gone.
        let mut f = DisplayRateFilter::new();
        run(&mut f, 5.0, 1.0, 0.0, 0.0, |_| 1.0);
        f.reset();
        let mut worst: f64 = 0.0;
        // Garbage first block (−119 %), then a clean platter at 0.92.
        let mut out = f.update(-0.1993, 0.0, DT);
        worst = worst.max((out - 1.0).abs());
        for _ in 0..(MED_WIN + 2) {
            out = f.update(0.92, 0.0, DT);
        }
        assert!(
            worst < 0.09,
            "garbage flashed on the readout during reseed: dev {worst}"
        );
        assert!(
            (out - 0.92).abs() < 1e-9,
            "did not settle on the platter rate: {out}"
        );
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
    fn decode_spikes_are_invisible_at_any_amplitude() {
        // The on-rig "left deck jumps by a whole BPM and quickly goes
        // back": a dirty needle's bad decode block produces a 1–2
        // quantum rate spike. The median prefilter must delete it and
        // the steady pole must not budge — the readout may not move by
        // more than ~0.02 BPM equivalent.
        let mut f = DisplayRateFilter::new();
        // Warm up on a wobble-free signal: this test isolates spike
        // rejection (the canceller's fit stays ~zero, so flat input
        // stays flat).
        run(&mut f, 12.0, 2.0, 0.0, 0.0, |_| 1.0);
        let mut pos = 12.0_f64;
        let mut worst = 0.0_f64;
        for i in 0..400 {
            // Every ~50 blocks, two consecutive spiked quanta of +0.6 %
            // (≈ +1 BPM at 175) — bigger than the old snap deadband.
            let spiked = i % 50 < 2;
            let rate = if spiked { 1.006 } else { 1.0 };
            let out = f.update(rate, pos, DT);
            pos += rate * DT;
            worst = worst.max((out - 1.0).abs());
        }
        assert!(
            worst < 1.5e-4,
            "decode spikes reached the readout: worst dev {worst}"
        );
    }

    /// On-rig capture replay (diagnostic, not a regression test): set
    /// `DUB_CAPTURE_WAV=/path/to/capture.wav` to feed a real
    /// `dub capture` recording through the PRODUCTION decode path — a
    /// real `TimecodeInput::drive` (decoder, smoother, anchor
    /// learning, policy) plus the DisplayRateFilter at 512-frame
    /// quanta — and write the per-quantum displayed pitch to
    /// `/tmp/dub_replay.csv` for offline analysis. The session-start
    /// calibration is triggered at the first confident block,
    /// mirroring the engine's classifier moment. No-op when the env
    /// var is unset.
    #[test]
    fn replay_capture_through_display_chain() {
        use std::fmt::Write as _;

        use ringbuf::traits::{Producer as _, Split as _};
        const QUANTUM: usize = 512;
        let Ok(path) = std::env::var("DUB_CAPTURE_WAV") else {
            return;
        };
        let mut reader = hound::WavReader::open(&path).expect("open capture wav");
        let spec = reader.spec();
        assert_eq!(spec.channels, 2, "capture must be stereo");
        let sr = spec.sample_rate as f32;
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
            hound::SampleFormat::Int => {
                let max = f32::from(i16::MAX);
                reader
                    .samples::<i16>()
                    .map(|s| f32::from(s.unwrap()) / max)
                    .collect()
            }
        };

        let rb = ringbuf::HeapRb::<f32>::new(spec.sample_rate as usize * 2);
        let (mut tx, rx) = rb.split();
        let cfg = crate::timecode::TimecodeInputConfig {
            input_sample_rate: sr,
            ..crate::timecode::TimecodeInputConfig::default()
        };
        let mut input = crate::timecode::TimecodeInput::new(rx, cfg);
        let mut display = DisplayRateFilter::new();

        let dtq = f64::from(QUANTUM as u32) / f64::from(sr);
        let mut csv = String::from("t,conf,amp,rate,lock,displayed_pitch\n");
        let mut displayed = f64::NAN;
        let mut calibration_started = false;
        let mut t = 0.0f64;
        for quantum in samples.chunks(QUANTUM * 2) {
            assert_eq!(tx.push_slice(quantum), quantum.len());
            let _ = input.drive();
            let Some(out) = input.last_output() else {
                t += dtq;
                continue;
            };
            if !calibration_started && out.confidence > 0.8 && out.amplitude > 0.05 {
                // Mirror the engine's auto-calibration trigger.
                input.begin_calibration(48_000);
                calibration_started = true;
            }
            let policy = input.policy();
            let lock = if !policy.is_engaged() {
                3
            } else if policy.consecutive_below() > 0 {
                2
            } else {
                1
            };
            if lock == 1 {
                displayed = display.update(input.last_display_rate(), out.position_secs, dtq);
            } else {
                display.reset();
            }
            let _ = writeln!(
                csv,
                "{:.4},{:.3},{:.4},{:+.5},{},{:+.4}",
                t,
                out.confidence,
                out.amplitude,
                out.rate,
                lock,
                (displayed - 1.0) * 100.0
            );
            t += dtq;
        }
        std::fs::write("/tmp/dub_replay.csv", csv).expect("write csv");
        println!("replayed {path} -> /tmp/dub_replay.csv (production drive path)");
    }

    /// Steadiness diagnostic (not a regression test): set
    /// `DUB_DIAG_WAV=/path/capture.wav` to dump, per render quantum, the
    /// **raw** decoder rate (pre-smoother), the **audible** rate
    /// (post-smoother + playback anchor), the **display** pitch
    /// (post-`DisplayRateFilter`), confidence, amplitude, groove
    /// position, and lock state — to `/tmp/dub_diag.csv`. Lets offline
    /// analysis separate genuine once-per-rev wobble from broadband
    /// decode noise, and see whether the `confidence < 0.6` smoother
    /// reset jolts the audible rate. No-op when the env var is unset.
    #[test]
    fn replay_capture_steadiness_diagnostic() {
        use std::fmt::Write as _;

        use ringbuf::traits::{Producer as _, Split as _};
        const QUANTUM: usize = 512;
        let Ok(path) = std::env::var("DUB_DIAG_WAV") else {
            return;
        };
        let mut reader = hound::WavReader::open(&path).expect("open capture wav");
        let spec = reader.spec();
        assert_eq!(spec.channels, 2, "capture must be stereo");
        let sr = spec.sample_rate as f32;
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
            hound::SampleFormat::Int => {
                let max = f32::from(i16::MAX);
                reader
                    .samples::<i16>()
                    .map(|s| f32::from(s.unwrap()) / max)
                    .collect()
            }
        };

        let rb = ringbuf::HeapRb::<f32>::new(spec.sample_rate as usize * 2);
        let (mut tx, rx) = rb.split();
        let cfg = crate::timecode::TimecodeInputConfig {
            input_sample_rate: sr,
            ..crate::timecode::TimecodeInputConfig::default()
        };
        let mut input = crate::timecode::TimecodeInput::new(rx, cfg);
        let mut display = DisplayRateFilter::new();

        let dtq = f64::from(QUANTUM as u32) / f64::from(sr);
        let mut csv = String::from(
            "t,conf,amp,raw_pct,audible_pct,disp_pct,pos,lock,abs_lock,abs_pos,drift_ms\n",
        );
        let mut displayed = f64::NAN;
        let mut calibration_started = false;
        // Drift accounting: the relative playhead is the integral of the
        // audible rate; the groove truth is the absolute LFSR position.
        // Their (anchored) difference is the sticker drift the engine
        // measures + heals — reproduced here from the raw capture so we
        // can see whether the abs lock holds and whether drift runs away.
        let mut rel_playhead = f64::NAN;
        let mut drift_anchor: Option<f64> = None;
        let mut t = 0.0f64;
        for quantum in samples.chunks(QUANTUM * 2) {
            assert_eq!(tx.push_slice(quantum), quantum.len());
            let _ = input.drive();
            let Some(out) = input.last_output() else {
                t += dtq;
                continue;
            };
            if !calibration_started && out.confidence > 0.8 && out.amplitude > 0.05 {
                input.begin_calibration(48_000);
                calibration_started = true;
            }
            let policy = input.policy();
            let lock = if !policy.is_engaged() {
                3
            } else if policy.consecutive_below() > 0 {
                2
            } else {
                1
            };
            if lock == 1 {
                displayed = display.update(input.last_display_rate(), out.position_secs, dtq);
            } else {
                display.reset();
            }
            // Relative-mode playhead: integrate the audible rate whenever
            // the deck is engaged (lock 1 or 2), exactly as the engine
            // advances a relative-locked deck.
            if policy.is_engaged() {
                if rel_playhead.is_nan() {
                    rel_playhead = out.position_secs;
                }
                rel_playhead += out.rate * dtq;
            }
            // Absolute groove truth + anchored drift (groove − playhead),
            // engine sign: positive = playhead lags the record.
            let abs_secs = out.abs_position_frames.map(|f| f / f64::from(sr));
            let abs_lock = u8::from(abs_secs.is_some());
            let drift_ms = match (abs_secs, rel_playhead.is_nan()) {
                (Some(a), false) => {
                    let off = a - rel_playhead;
                    let anchor = *drift_anchor.get_or_insert(off);
                    (off - anchor) * 1000.0
                }
                _ => f64::NAN,
            };
            let _ = writeln!(
                csv,
                "{:.4},{:.3},{:.4},{:+.5},{:+.5},{:+.4},{:.5},{},{},{:.5},{:+.3}",
                t,
                out.confidence,
                out.amplitude,
                (input.last_raw_rate() - 1.0) * 100.0,
                (out.rate - 1.0) * 100.0,
                (displayed - 1.0) * 100.0,
                out.position_secs,
                lock,
                abs_lock,
                abs_secs.unwrap_or(f64::NAN),
                drift_ms,
            );
            t += dtq;
        }
        std::fs::write("/tmp/dub_diag.csv", csv).expect("write csv");
        println!("replayed {path} -> /tmp/dub_diag.csv (raw/audible/display + abs_lock/drift)");
    }

    #[test]
    fn fresh_filter_seeds_from_median_after_one_window() {
        let mut f = DisplayRateFilter::new();
        let mut out = 0.0;
        for _ in 0..MED_WIN {
            out = f.update(1.35, 0.0, DT);
        }
        assert!((out - 1.35).abs() < 1e-12);
    }
}
