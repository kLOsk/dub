//! Stereo timecode decoder via coherent block phase-difference.
//!
//! ## Algorithm
//!
//! The two stereo channels of Serato Control CV02 vinyl carry the
//! same carrier sinusoid offset by 90°. Per the empirical convention
//! observed on real CV02 cartridges going through the SL3, channel 0
//! leads channel 1 by 90° at forward play — i.e. `ch0 ≈ A·sin(φ)`,
//! `ch1 ≈ A·cos(φ)`. We treat the sample pair as a single complex
//! sample `s = ch1 + j·ch0`, which is `A·exp(j·2π·f·t)` rotating
//! counter-clockwise (positive frequency) for forward stylus motion
//! and clockwise (negative) for reverse.
//!
//! Per-sample phase advance is therefore:
//!
//! ```text
//!   Δφ = arg(s_n · conj(s_{n-1}))  ≈  2π · f_inst · Δt
//! ```
//!
//! Summing `s_n · conj(s_{n-1})` over a block before taking `arg` is a
//! coherent average: noise (uncorrelated across samples) suppresses
//! by `√N`, signal adds linearly. With a 64-sample block at 48 kHz
//! that's a ~9 dB noise gain — enough to make the decoder work
//! happily on tape-quality timecode rips.
//!
//! Direction falls out for free: `f_inst < 0` ⇔ reverse motion. No
//! separate "forward/reverse" flag flip needed.
//!
//! Position is the integral of `f_inst` over time, normalized by the
//! nominal carrier frequency. We accumulate it in seconds-of-record
//! at unity speed so the engine can map deck position 1:1 in M5.3.
//!
//! ## Absolute position (M6)
//!
//! A decoder built with [`Decoder::with_absolute`] additionally
//! demodulates the AM bitstream riding on the carrier (one LFSR bit
//! per cycle) and, once locked, reports the **absolute groove
//! position** in [`DecodeOutput::abs_position_frames`]. Its per-block
//! *deltas* give bit-exact, drift-free velocity — immune to the
//! residual cartridge-ellipse bias the carrier-phase rate carries.
//! Deck behavior stays relative (no needle-drop); see `absolute.rs`.
//!
//! ## What this *doesn't* do (yet)
//!
//! - **Stickiness on lift** (M5.4). The decoder reports `confidence`
//!   today; the *policy* of "stop the deck and remember position"
//!   when confidence drops belongs in the integration layer, not here.
//! - **Calibration / amplitude AGC** (M6). We assume the input is
//!   nominally `±0.3..±0.7` after gain-staging. Real cartridges plus
//!   real preamps need an AGC; deferred.
//!
//! ## RT-safety
//!
//! [`Decoder::process`] is allocation-free and lock-free, so the
//! decoder is safe to run on the audio thread once the live wiring
//! lands in M5.3. Floating-point only — no transcendentals other than
//! `atan2` once per block. At 48 kHz / 64-frame blocks that's 750
//! atan2 calls/sec/deck — trivial.

use crate::absolute::AbsoluteTracker;
use crate::Format;

/// Output of one [`Decoder::process`] call. Caller drives deck
/// transport from these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodeOutput {
    /// Estimated playback rate over this block, normalized to nominal
    /// carrier frequency. `1.0` = forward unity, `-1.0` = reverse unity,
    /// `0.0` = stylus stationary on groove.
    ///
    /// At very high speeds (`|rate| > 0.5 · sample_rate / carrier_hz`,
    /// e.g. 24× at 48 kHz / 1 kHz) the per-sample phase advance
    /// approaches ±π and the estimate ambiguates with its alias —
    /// the decoder will return *some* number but it may be wrong by
    /// 2× the true rate. Real DJs scratch up to ~8×, well clear of
    /// the alias band.
    pub rate: f64,

    /// Cumulative position offset since [`Decoder::new`] (or the last
    /// [`Decoder::reset`]), measured in seconds at unity speed. This
    /// is the relative-mode position; absolute-mode position requires
    /// the bitstream decode (M6).
    pub position_secs: f64,

    /// RMS amplitude of the input signal, useful for stylus-lift
    /// detection. Falls below ~0.01 when the stylus is off the groove
    /// (assuming reasonable cartridge gain). Use this in the
    /// integration layer to drive the "stickiness" policy.
    pub amplitude: f32,

    /// Heuristic confidence in `[0, 1]`. `1.0` means the input is a
    /// pure complex exponential at some frequency (forward, reverse,
    /// or zero); `0.0` means uncorrelated noise. Below ~0.5 indicates
    /// noise/transients/crosstalk and the rate estimate should not
    /// drive deck transport.
    pub confidence: f32,

    /// Absolute groove position in **input frames at unity speed**,
    /// from the LFSR bitstream (M6). `Some` only while the absolute
    /// tracker is locked on a decoder built with
    /// [`Decoder::with_absolute`]; always `None` otherwise. Consumers
    /// use per-block *deltas* of this for bit-exact, drift-free
    /// velocity — never the value itself (deck behavior stays
    /// relative; no needle-drop, PRD §5.1).
    pub abs_position_frames: Option<f64>,

    /// Bit-agreement confidence of the absolute tracker in `[0, 1]`;
    /// `0.0` while unlocked or when absolute decoding is off.
    pub abs_confidence: f32,
}

/// Carrier-presence amplitude ramp used to gate [`DecodeOutput::confidence`]
/// (see [`Decoder::process`]). Phase coherence is meaningless without a
/// live carrier, so the reported confidence is scaled by a linear ramp on
/// RMS amplitude: zero at or below [`CARRIER_PRESENCE_OFF`], full at or
/// above [`CARRIER_PRESENCE_FULL`].
///
/// The levels are deliberately *permissive*: a cartridge is a velocity
/// sensor, so a slow scratch draw (rate ~0.1) natively outputs ~10× less
/// than unity play — gating those blocks pauses the deck mid-gesture
/// (the on-rig "timecode didn't react" + the dominant sticker-drift
/// mechanism: deck frozen while the record moves). The gate can afford
/// to be permissive because the input high-pass ([`INPUT_HP_HZ`])
/// removes the contaminants this ramp was built against *before* the
/// amplitude is measured: a stopped stylus / DC offset reads ~0 (the HP
/// blocks anything static) and −30…−20 dB mains hum lands ≤ 0.006 after
/// filtering — still under or barely over the off level, and its
/// coherence contribution is then negligible. A healthy carrier at
/// 0.1–0.5 is orders of magnitude above the full level.
const CARRIER_PRESENCE_OFF: f32 = 0.004;
const CARRIER_PRESENCE_FULL: f32 = 0.016;

/// Input high-pass cutoff (Hz). The coherent phase-difference estimator
/// is unbiased under *white* noise, but **correlated** low-frequency
/// contamination — DC offset, mains hum, turntable rumble — is nearly
/// identical sample-to-sample, so it contributes a real-*positive* term
/// to `Σ s·conj(s_prev)` that drags the block phase toward zero and
/// shrinks the decoded rate multiplicatively: 50 Hz hum just 30 dB under
/// the carrier reads ≈ −0.1 % across the whole pitch range (a fader at
/// true 0 displays −0.1…−0.2 %). High-passing both channels identically
/// removes the bias without touching the carrier: the lowest carrier in
/// normal play is ~920 Hz (CV02 at −8 %), and a common per-channel filter
/// preserves the Lissajous shape, so whitening and the ellipse correction
/// are unaffected.
///
/// The cutoff is a trade between hum rejection and **slow-scratch
/// response**: a slow draw at rate 0.1 puts the CV02 carrier at 100 Hz,
/// already ~10× quieter natively (velocity-proportional cartridge), and
/// every dB the filter takes there pushes the gesture toward the
/// presence gate — on-rig that read as "timecode didn't react" and is
/// the dominant sticker-drift mechanism (deck frozen mid-gesture while
/// the record moves). 120 Hz keeps 50/60 Hz ≥ 15 dB down in power
/// (residual bias ≤ 0.005 %, vs the −0.1 % defect) and DC/rumble fully
/// blocked, while costing a rate-0.1 draw only ~5 dB. US 120 Hz
/// rectifier hum is in the transition band and only drops ~3 dB —
/// revisit (notch, or per-region cutoff) if a 60 Hz-land rig shows a
/// residual offset.
const INPUT_HP_HZ: f64 = 120.0;

/// Maximum long-lag span (samples) for the low-bias phase estimator,
/// and the |rate| below which it is used. The coherent lag-1
/// phase-difference is unbiased under white noise but **broadband
/// correlated** noise (vinyl surface noise / crackle: lag-1
/// autocorrelation ~0.8) adds a real-positive term to `Σ s·conj(s_prev)`
/// and shrinks the measured rate — ~−0.3 % at coherence 0.999 on a real
/// rig (tiny incoherence × highly-correlated noise ⇒ large shrink,
/// because shrink/incoherence ≈ ρ/(1−ρ)). Measuring the phase advance
/// over L samples instead leaves the carrier term intact while the
/// noise autocorrelation collapses (ρ^L), killing the bias ~20× at
/// L = 16. The lag is sized per format so the per-L-samples phase stays
/// unambiguous (|L·ω| < π) up to [`LONG_LAG_MAX_RATE`]; above that
/// (scratching) the lag-1 path takes over, where absolute rate
/// precision doesn't matter.
const LONG_LAG_MAX: usize = 16;

/// |rate| (from the lag-1 estimate) above which the long-lag phase
/// would risk ambiguity — fall back to lag-1.
const LONG_LAG_MAX_RATE: f64 = 1.2;

/// Pole for the smoothed roundness feeding the long-lag gate
/// (τ ≈ 130 ms at 64-frame chunks).
const ROUNDNESS_ALPHA: f64 = 0.01;

/// Minimum residual-ellipse roundness (`ellipse_factor`) for the
/// long-lag path. Post-whitening rigs sit near 1.0; an uncalibrated
/// cartridge's strong ellipse keeps the exact lag-1 correction model.
const LONG_LAG_MIN_ROUNDNESS: f64 = 0.99;

/// Cap on the high-pass amplitude compensation (see the carrier-
/// presence block in [`Decoder::process`]). 8× covers carriers down to
/// ~40 Hz (CV02 rate ≈ 0.04); below that the signal is genuinely gone
/// and boosting further would only amplify noise.
const MAX_AMP_BOOST: f64 = 8.0;

/// Minimum |rate| for the compensation to apply: a stopped platter or
/// pure noise decodes to ≈ 0 rate and must never have its residue
/// boosted into "carrier present".
const AMP_COMP_MIN_RATE: f64 = 0.02;

/// One 2nd-order Butterworth high-pass section (RBJ biquad, Direct
/// Form 1, `f64`). Allocation-free and branch-free per sample — RT-safe.
#[derive(Debug, Clone, Copy)]
struct BiquadHp {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BiquadHp {
    fn new(cutoff_hz: f64, sample_rate: f64) -> Self {
        let w0 = std::f64::consts::TAU * cutoff_hz / sample_rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / std::f64::consts::SQRT_2; // Q = 1/√2
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos_w0) / (2.0 * a0),
            b1: -(1.0 + cos_w0) / a0,
            b2: (1.0 + cos_w0) / (2.0 * a0),
            a1: -2.0 * cos_w0 / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Stateful timecode decoder. One instance per deck.
///
/// `Decoder` is `!Send` only by convention — there is nothing
/// non-`Send` inside; we just don't want to encourage handing the
/// same decoder to multiple threads. Wrap in an `Arc<Mutex>` if you
/// need cross-thread access (you almost certainly don't).
pub struct Decoder {
    sample_rate: f32,
    /// Nominal carrier frequency in Hz (cached from [`Format`]).
    carrier_hz: f32,
    /// Previous complex sample's real part (= file ch1, the cos
    /// component). Retained across `process` calls so the
    /// phase-difference formula has continuity at block boundaries.
    prev_re: f64,
    /// Previous complex sample's imag part (= file ch0, the sin component).
    prev_im: f64,
    /// Whether `prev_*` have been seeded with at least one sample.
    primed: bool,
    /// Ring of the last [`LONG_LAG_MAX`] whitened complex samples for
    /// the long-lag estimator (only `long_lag` entries are used).
    lag_hist: [(f64, f64); LONG_LAG_MAX],
    lag_pos: usize,
    /// Samples seen since reset — the long-lag sum is valid once this
    /// reaches `long_lag`.
    lag_warm: usize,
    /// Format-dependent long-lag span: `floor(sr / (2.5 · carrier_hz))`
    /// clamped to `[2, LONG_LAG_MAX]`, so `|rate| ≤` ~1.25 stays
    /// unambiguous (CV02 @48 k → 16, MK1 → 9, MK2 → 7).
    long_lag: usize,
    /// Slow EMA (τ ≈ 130 ms) of the residual-ellipse roundness used by
    /// the long-lag gate. Gating on the instantaneous roundness made
    /// the estimator path toggle per block on a dirty rig hovering at
    /// the threshold — alternating the ~0.3 % lag-1 bias on and off,
    /// which read as display jitter. Starts at 0 so a fresh decoder
    /// (and the single-block ellipse-law tests) is on the exact lag-1
    /// path until the estimate matures.
    roundness_smoothed: f64,
    /// Cumulative position in seconds-at-unity-speed.
    position_secs: f64,
    /// 2×2 channel-whitening matrix applied to `(ch1, ch0)` before the
    /// phase-difference, to correct a cartridge's L/R gain imbalance,
    /// quadrature-phase (azimuth) error, and crosstalk. An uncorrected
    /// cartridge turns the timecode Lissajous from a circle into an
    /// ellipse, which injects a counter-rotating image and biases the
    /// decoded rate **low** (`rate × 2G/(G²+1)`; a ~2 dB imbalance ≈
    /// −2.3 %). Whitening re-circularises the signal and removes the
    /// bias. Defaults to identity (no correction) until calibrated.
    /// See [`compute_whitening`].
    whitening: [[f64; 2]; 2],
    /// Smoothed estimate of the **residual ellipse** the carrier traces
    /// after `whitening`, measured directly from the per-block covariance:
    /// `er = (<re²>−<im²>)/(<re²>+<im²>)` is the gain-imbalance (axis-ratio)
    /// term, `ec = 2<re·im>/(<re²>+<im²>)` the azimuth (tilt) term. A real
    /// cartridge's L/R response drifts with frequency, so the static
    /// `whitening` (fixed at one frequency) leaves a residual ellipse that
    /// grows with pitch — biasing the decoded rate into a quadratic droop
    /// at the extremes (+8 %→+5 %, −8 %→−12 %) while coherence stays ~1.0.
    /// From `er`,`ec` we form the rate-distortion factor
    /// `R = √(1 − er² − ec²)` (which is `cos(azimuth)` for pure tilt and
    /// `2g/(1+g²)` for pure gain imbalance) and divide it back out. A
    /// self-calibrating fix — no sweep or gesture; it learns continuously
    /// as the platter moves. EMA-smoothed over confident blocks; primed on
    /// first lock.
    ellipse_er: f64,
    ellipse_ec: f64,
    /// Whether the ellipse estimate has been seeded from a confident block
    /// yet (so the first lock snaps instead of slewing from zero).
    ellipse_primed: bool,
    /// Master enable for the ellipse auto-correction (default on). Tests
    /// of the raw decode can disable it to exercise the uncorrected path.
    ellipse_correction: bool,
    /// Per-channel input high-pass (`[ch1/re, ch0/im]`) removing the
    /// DC / hum / rumble bias — see [`INPUT_HP_HZ`]. Feeds the
    /// whitening + phase-difference path only; the absolute tracker
    /// stays on the raw channels (its zero-crossing detectors do their
    /// own timing).
    input_hp: [BiquadHp; 2],
    /// Absolute-position tracker (M6), present only on a decoder built
    /// with [`Decoder::with_absolute`]. Fed per-sample from the
    /// `process` loop; its LUT is allocated at construction, off-RT.
    absolute: Option<AbsoluteTracker>,
}

impl Decoder {
    /// Create a decoder for the given timecode format and sample rate.
    ///
    /// As of M6 all three relative-mode formats are supported:
    /// [`Format::SeratoCv02`] (1 kHz carrier),
    /// [`Format::TraktorMk1`] (2 kHz, AM modulation), and
    /// [`Format::TraktorMk2`] (2.5 kHz, offset modulation). The
    /// algorithm is format-agnostic — the only per-format parameter
    /// the decoder uses today is the nominal carrier frequency,
    /// pulled from [`Format::carrier_hz`]. All three encode their
    /// stereo carrier in the same quadrature convention (`ch0 = sin`,
    /// `ch1 = cos`), validated empirically against real cartridges
    /// on the SL3 in M5.3 (Serato) and M6 (both Traktor generations).
    /// MK2's offset modulation rides as a vertical DC shift; the
    /// cartridge/preamp AC-couples it out before it reaches us, so
    /// the relative-mode math sees a clean 2.5 kHz carrier.
    ///
    /// Absolute-position decoding (the bitstream riding on top of
    /// the carrier) still isn't done — relative mode covers v1's
    /// scratch-DJ workflow.
    ///
    /// # Panics
    /// `sample_rate` must be positive.
    #[must_use]
    pub fn new(format: Format, sample_rate: f32) -> Self {
        assert!(sample_rate > 0.0, "sample rate must be > 0");
        Self {
            sample_rate,
            carrier_hz: format.carrier_hz(),
            prev_re: 0.0,
            prev_im: 0.0,
            primed: false,
            position_secs: 0.0,
            whitening: IDENTITY_WHITENING,
            ellipse_er: 0.0,
            ellipse_ec: 0.0,
            ellipse_primed: false,
            ellipse_correction: true,
            input_hp: [BiquadHp::new(INPUT_HP_HZ, f64::from(sample_rate)); 2],
            lag_hist: [(0.0, 0.0); LONG_LAG_MAX],
            lag_pos: 0,
            lag_warm: 0,
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            long_lag: ((sample_rate / (2.5 * format.carrier_hz())) as usize).clamp(2, LONG_LAG_MAX),
            roundness_smoothed: 0.0,
            absolute: None,
        }
    }

    /// Like [`Decoder::new`], but with **absolute-position decoding**
    /// enabled (M6). Builds the format's position LUT — allocates a few
    /// MB, so this MUST run off the audio thread; `process` itself
    /// stays alloc-free. For a format whose bitstream we don't decode
    /// (Traktor MK2, and MK1 until Phase 2) this silently degrades to
    /// a plain relative decoder — `abs_position_frames` stays `None`.
    ///
    /// # Panics
    /// `sample_rate` must be positive.
    #[must_use]
    pub fn with_absolute(format: Format, sample_rate: f32) -> Self {
        let mut dec = Self::new(format, sample_rate);
        dec.absolute = AbsoluteTracker::new(format, sample_rate);
        dec
    }

    /// Whether the absolute tracker is currently locked to the LFSR
    /// bitstream. Diagnostic.
    #[must_use]
    pub fn absolute_locked(&self) -> bool {
        self.absolute.as_ref().is_some_and(AbsoluteTracker::locked)
    }

    /// Absolute-tracker acquisition diagnostics `(crossings, lut_hits,
    /// max_consecutive_hits)`, or `None` when absolute decoding is off.
    #[must_use]
    pub fn absolute_debug(&self) -> Option<(u64, u64, u32)> {
        self.absolute
            .as_ref()
            .map(AbsoluteTracker::debug_acquisition)
    }

    /// The locked Serato variant's name (e.g. `"serato_cd"`) when the
    /// absolute tracker has auto-detected one, else `None`.
    #[must_use]
    pub fn absolute_variant(&self) -> Option<&'static str> {
        self.absolute
            .as_ref()
            .and_then(AbsoluteTracker::locked_variant_name)
    }

    /// First 48 `(bit_mean, sliced_bit)` pairs from the absolute tracker.
    #[must_use]
    pub fn absolute_first_cycles(&self) -> Option<([f32; 48], u64)> {
        self.absolute
            .as_ref()
            .map(AbsoluteTracker::debug_first_cycles)
    }

    /// Install a channel-whitening matrix (from [`compute_whitening`]).
    /// Applied to every subsequent decoded sample. RT-safe — just
    /// stores four `f64`s.
    pub fn set_whitening(&mut self, whitening: [[f64; 2]; 2]) {
        self.whitening = whitening;
    }

    /// Reset whitening to identity (no channel correction).
    pub fn clear_whitening(&mut self) {
        self.whitening = IDENTITY_WHITENING;
    }

    /// Enable/disable the azimuth auto-correction (default enabled).
    pub fn set_ellipse_correction(&mut self, on: bool) {
        self.ellipse_correction = on;
    }

    /// The current residual-ellipse rate-distortion factor `R` (1.0 = no
    /// residual). Diagnostic.
    #[must_use]
    pub fn ellipse_factor(&self) -> f64 {
        (1.0 - self.ellipse_er * self.ellipse_er - self.ellipse_ec * self.ellipse_ec)
            .max(1e-6)
            .sqrt()
    }

    /// The currently-installed whitening matrix.
    #[must_use]
    pub fn whitening(&self) -> [[f64; 2]; 2] {
        self.whitening
    }

    /// Calibrate this deck from a captured buffer of its timecode
    /// signal (a clean spin of the control record). Computes and
    /// installs the channel-whitening matrix. See [`compute_whitening`].
    pub fn calibrate(&mut self, stereo: &[f32]) {
        self.whitening = compute_whitening(stereo);
    }

    /// Reset accumulated position and the prev-sample register. Useful
    /// when re-cueing the deck or recovering from a stylus lift.
    pub fn reset(&mut self) {
        self.prev_re = 0.0;
        self.prev_im = 0.0;
        self.primed = false;
        self.position_secs = 0.0;
        self.lag_pos = 0;
        self.lag_warm = 0;
        self.input_hp[0].reset();
        self.input_hp[1].reset();
        if let Some(t) = self.absolute.as_mut() {
            t.reset();
        }
    }

    /// Cumulative position in seconds-at-unity-speed.
    #[must_use]
    pub fn position_secs(&self) -> f64 {
        self.position_secs
    }

    /// Decode one stereo block (interleaved). The block can be any
    /// length ≥ 1 frame; longer blocks give better noise rejection
    /// but the decoder also tolerates per-sample calls.
    ///
    /// # Panics
    /// `stereo.len()` must be even.
    #[allow(clippy::too_many_lines)] // one sequential per-block DSP pipeline
    pub fn process(&mut self, stereo: &[f32]) -> DecodeOutput {
        assert_eq!(stereo.len() % 2, 0, "interleaved stereo buffer required");
        let n_frames = stereo.len() / 2;
        if n_frames == 0 {
            return DecodeOutput {
                rate: 0.0,
                position_secs: self.position_secs,
                amplitude: 0.0,
                confidence: 0.0,
                abs_position_frames: self
                    .absolute
                    .as_ref()
                    .and_then(AbsoluteTracker::position_frames),
                abs_confidence: self
                    .absolute
                    .as_ref()
                    .map_or(0.0, AbsoluteTracker::confidence),
            };
        }

        // Accumulators: `acc` is the coherent sum of consecutive-sample
        // phase-difference vectors; `mag_acc` is the sum of |s|² used
        // both for amplitude RMS and confidence normalization.
        let mut acc_re = 0.0_f64;
        let mut acc_im = 0.0_f64;
        // Per-channel power Σ re², Σ im² and the inter-channel
        // cross-correlation Σ re·im — the residual ellipse's covariance,
        // used to linearise the rate (see the post-loop correction).
        let mut sq_re_acc = 0.0_f64;
        let mut sq_im_acc = 0.0_f64;
        let mut cross_acc = 0.0_f64;
        let mut samples_consumed = 0_usize;
        let mut acc_l_re = 0.0_f64;
        let mut acc_l_im = 0.0_f64;
        let mut lag_pairs = 0_usize;

        let w = self.whitening;
        for frame in stereo.chunks_exact(2) {
            // Serato CV02 convention (verified against a real cartridge
            // on an SL3): file ch0 ≈ A·sin(φ), file ch1 ≈ A·cos(φ).
            // Map ch1 → real, ch0 → imag so `s = re + j·im = A·e^(jφ)`
            // rotates the *positive* direction at forward play.
            // High-passed first: DC / hum / rumble are sample-to-sample
            // correlated and bias the coherent sum's angle toward zero
            // (≈ −0.1 % rate at −30 dB hum) — see [`INPUT_HP_HZ`].
            let re_raw = self.input_hp[0].process(f64::from(frame[1]));
            let im_raw = self.input_hp[1].process(f64::from(frame[0]));
            // Channel whitening (identity until calibrated) corrects the
            // cartridge ellipse before the phase-difference.
            let re = w[0][0] * re_raw + w[0][1] * im_raw;
            let im = w[1][0] * re_raw + w[1][1] * im_raw;
            // |s|² = re² + im²; Pythagorean amplitude regardless of phase.
            sq_re_acc += re * re;
            sq_im_acc += im * im;
            cross_acc += re * im;

            if self.primed {
                // s_curr · conj(s_prev) = (re + j·im)·(prev_re − j·prev_im)
                //                       = (re·prev_re + im·prev_im)
                //                       + j·(im·prev_re − re·prev_im)
                acc_re += re * self.prev_re + im * self.prev_im;
                acc_im += im * self.prev_re - re * self.prev_im;
                samples_consumed += 1;
            }
            // Long-lag coherent sum: s_curr · conj(s_{n−long_lag}).
            // The ring always advances; the sum only counts once the
            // ring holds `long_lag` real samples.
            if self.lag_warm >= self.long_lag {
                let (hre, him) = self.lag_hist[self.lag_pos];
                acc_l_re += re * hre + im * him;
                acc_l_im += im * hre - re * him;
                lag_pairs += 1;
            }
            self.lag_hist[self.lag_pos] = (re, im);
            self.lag_pos = (self.lag_pos + 1) % self.long_lag;
            self.lag_warm = self.lag_warm.saturating_add(1);
            self.prev_re = re;
            self.prev_im = im;
            self.primed = true;
            // Absolute tracker (M6) reads the AM position bit off the
            // **whitened** carrier phasor `(re, im)` computed just above —
            // the same high-passed, decorrelated phasor the relative path
            // uses. The per-cycle RMS read decodes real vinyl ~95 % vs
            // ~80 % for the raw zero-crossing peak (validated by
            // `--sweep`). Alloc-free.
            if let Some(t) = self.absolute.as_mut() {
                t.on_sample(re, im);
            }
        }

        // Block-level instantaneous frequency from the coherent sum's
        // argument. `samples_consumed` is `n_frames` on a primed
        // decoder; on the very first call it's `n_frames − 1` (we lose
        // one sample of phase-diff to bootstrap `prev_*`).
        let dt = 1.0 / f64::from(self.sample_rate);
        let phase_lag1 = if samples_consumed > 0 {
            acc_im.atan2(acc_re)
        } else {
            0.0
        };
        let nominal = f64::from(self.carrier_hz);
        let rate_lag1 = phase_lag1 / (std::f64::consts::TAU * dt) / nominal;

        let mag_acc = sq_re_acc + sq_im_acc;

        // Confidence numerator first: the ellipse estimate must adapt
        // from THIS block's covariance before the rate path is chosen,
        // or the roundness gate runs one block stale (a fresh decoder
        // would route an uncalibrated ellipse through the long-lag
        // path on its very first block).
        let coherent_mag = (acc_re * acc_re + acc_im * acc_im).sqrt();
        let coherence = if mag_acc > 1e-12 {
            #[allow(clippy::cast_possible_truncation)]
            ((coherent_mag / mag_acc).clamp(0.0, 1.0) as f32)
        } else {
            0.0
        };
        if coherence > 0.9 {
            self.update_ellipse_estimate(sq_re_acc, sq_im_acc, cross_acc, mag_acc);
            self.roundness_smoothed +=
                ROUNDNESS_ALPHA * (self.ellipse_factor() - self.roundness_smoothed);
        }

        // Prefer the long-lag phase (per-sample equivalent) inside the
        // unambiguous band AND while the (whitened) residual ellipse is
        // round: same carrier term, ~20× less correlated-noise shrink —
        // this is what puts a true 0.00 % at fader 0 on a real (noisy)
        // record. A strongly elliptical signal (uncalibrated cartridge)
        // stays on the lag-1 path, whose ellipse-bias model and
        // correction are exact; the long-lag weighting follows a
        // different (uncorrected) law, but at roundness ≥
        // [`LONG_LAG_MIN_ROUNDNESS`] the residual is second-order.
        let use_long_lag = lag_pairs > 0
            && rate_lag1.abs() < LONG_LAG_MAX_RATE
            && self.roundness_smoothed > LONG_LAG_MIN_ROUNDNESS;
        #[allow(clippy::cast_precision_loss)]
        let phase_diff = if use_long_lag {
            acc_l_im.atan2(acc_l_re) / self.long_lag as f64
        } else {
            phase_lag1
        };
        let inst_freq_hz = phase_diff / (std::f64::consts::TAU * dt);
        let rate = inst_freq_hz / nominal;

        // Amplitude is RMS of |s| over the block. Note: |s|² = L²+R²
        // is *constant* (= A²) for a perfect quadrature signal, so RMS
        // here ≈ A, not A/√2 — which is what we want as the "carrier
        // amplitude" reading.
        #[allow(clippy::cast_precision_loss)]
        let mean_sq = mag_acc / (n_frames as f64);
        // Velocity-honest amplitude: undo the input high-pass's known
        // attenuation at the *estimated* carrier frequency. A slow draw
        // is doubly quiet — the cartridge outputs less (velocity
        // sensor) AND its low carrier sits in the filter's transition
        // band — and gating on the filtered amplitude paused the deck
        // through slow backward draws (the velocity dead zone: profile
        // C of `dub-engine/tests/scratch_drift.rs`, +700 ms of forward
        // sticker drift per cycle). Dividing out |H(carrier)| reports
        // what the needle actually picks up. The boost is capped
        // ([`MAX_AMP_BOOST`]) and gated on a meaningfully nonzero rate
        // so silence, DC, and a stopped platter (rate ≈ 0) are never
        // amplified into "carrier present".
        #[allow(clippy::cast_possible_truncation)]
        let amplitude = (mean_sq.sqrt() * hp_compensation(rate.abs() * nominal)) as f32;

        // Self-calibrating residual-ellipse correction: applied only
        // on the lag-1 path — its `atan(tan/R)` model is specific to
        // the lag-1 amplitude weighting (the estimate itself adapted
        // above, before the path was chosen). Skipped on a scratch
        // (huge phase_diff, where `tan` blows up and we don't trust
        // the rate anyway) and on low-coherence blocks.
        let rate = if !use_long_lag
            && self.ellipse_correction
            && coherence > 0.9
            && phase_diff.abs() < 1.4
        {
            self.ellipse_corrected_rate(phase_diff, rate, nominal, dt)
        } else {
            rate
        };
        // Phase coherence alone is *not* carrier presence: a perfectly
        // **static** phasor — a stopped stylus, a DC offset, or mains hum
        // — is maximally coherent (s_curr ≈ s_prev ⇒ |Σ s·conj(s_prev)|
        // ≈ Σ|s|²), so coherence reads ~1.0 even with the platter still
        // and the level near zero. That produced the "confidence high
        // with no amplitude" reading. Gate coherence by a carrier-
        // presence ramp on amplitude so a silent / very-low-level input
        // reports low confidence; a healthy carrier (~0.1–0.5, well above
        // the floor) is unaffected.
        let presence = ((amplitude - CARRIER_PRESENCE_OFF)
            / (CARRIER_PRESENCE_FULL - CARRIER_PRESENCE_OFF))
            .clamp(0.0, 1.0);
        let confidence = coherence * presence;

        // Integrate position. Block duration in seconds at the engine
        // SR (NOT scaled by rate — `rate` already encodes how fast the
        // record is moving relative to nominal).
        #[allow(clippy::cast_precision_loss)]
        let block_secs_real = (n_frames as f64) * dt;
        // `rate` is normalized vs nominal carrier; multiplying by real
        // seconds gives "seconds of record advanced" which is what we
        // want for relative position.
        self.position_secs += rate * block_secs_real;

        DecodeOutput {
            rate,
            position_secs: self.position_secs,
            amplitude,
            confidence,
            abs_position_frames: self
                .absolute
                .as_ref()
                .and_then(AbsoluteTracker::position_frames),
            abs_confidence: self
                .absolute
                .as_ref()
                .map_or(0.0, AbsoluteTracker::confidence),
        }
    }

    /// Self-calibrating residual-ellipse rate correction. After
    /// whitening, any residual ellipse the carrier traces biases the
    /// rate by a factor `R` — `measured_ω = atan(R·tan(ω_true))`. We
    /// read the ellipse straight off the per-block covariance: `er`
    /// (gain imbalance) and `ec` (azimuth), EMA them on confident
    /// blocks (primed on first lock), form `R = √(1 − er² − ec²)`, and
    /// divide it back out to linearise the rate across the whole pitch
    /// range. Self-calibrating — no sweep or gesture; it learns as the
    /// platter moves.
    #[allow(clippy::too_many_arguments)] // private per-block scratch
    /// Fold one block's covariance into the residual-ellipse EMA.
    /// Adaptation is split from the correction math so it keeps
    /// learning while the long-lag estimator (which needs no lag-1
    /// ellipse correction) is driving the rate.
    fn update_ellipse_estimate(
        &mut self,
        sq_re_acc: f64,
        sq_im_acc: f64,
        cross_acc: f64,
        mag_acc: f64,
    ) {
        if mag_acc > 1e-12 {
            let er = (sq_re_acc - sq_im_acc) / mag_acc;
            let ec = 2.0 * cross_acc / mag_acc;
            if self.ellipse_primed {
                // The residual ellipse is steady at a given pitch, so a
                // fairly fast pole is safe (and downstream rate
                // smoothing soaks up any per-block noise). Fast enough
                // that it converges to the new pitch's ellipse within
                // a fraction of a second — otherwise a reading taken
                // soon after a pitch move retains the *previous*
                // pitch's azimuth (opposite sign), skewing the extremes
                // asymmetrically.
                const ELLIPSE_ALPHA: f64 = 0.15;
                self.ellipse_er += ELLIPSE_ALPHA * (er - self.ellipse_er);
                self.ellipse_ec += ELLIPSE_ALPHA * (ec - self.ellipse_ec);
            } else {
                self.ellipse_er = er;
                self.ellipse_ec = ec;
                self.ellipse_primed = true;
            }
        }
    }

    fn ellipse_corrected_rate(&mut self, phase_diff: f64, rate: f64, nominal: f64, dt: f64) -> f64 {
        let omega_nominal = std::f64::consts::TAU * nominal * dt;
        let r = (1.0 - self.ellipse_er * self.ellipse_er - self.ellipse_ec * self.ellipse_ec)
            .clamp(1e-6, 1.0)
            .sqrt();
        if omega_nominal > 0.0 {
            (phase_diff.tan() / r).atan() / omega_nominal
        } else {
            rate
        }
    }
}

/// Inverse magnitude of the input high-pass at `carrier_hz`, capped at
/// [`MAX_AMP_BOOST`], unity below [`AMP_COMP_MIN_RATE`]-equivalent
/// carriers so silence / DC / a stopped platter is never boosted into
/// "carrier present". See the carrier-presence block in
/// [`Decoder::process`].
fn hp_compensation(carrier_hz: f64) -> f64 {
    if carrier_hz < AMP_COMP_MIN_RATE * 1_000.0 {
        return 1.0;
    }
    let x = carrier_hz / INPUT_HP_HZ;
    let x2 = x * x;
    let hp_mag = x2 / (1.0 + x2 * x2).sqrt();
    1.0 / hp_mag.max(1.0 / MAX_AMP_BOOST)
}

/// Identity whitening — no channel correction.
pub const IDENTITY_WHITENING: [[f64; 2]; 2] = [[1.0, 0.0], [0.0, 1.0]];

/// Compute a channel-whitening matrix from a buffer of interleaved
/// stereo timecode (one clean spin of the control record).
///
/// Measures the 2×2 covariance of `(ch1, ch0)` and returns its
/// power-preserving inverse square root, which maps the cartridge's
/// elliptical Lissajous back to a circle — correcting gain imbalance,
/// quadrature-phase (azimuth) error, and crosstalk in one transform.
/// Feeding the result to [`Decoder::set_whitening`] removes the
/// `2G/(G²+1)` rate bias an uncorrected cartridge introduces.
///
/// Returns [`IDENTITY_WHITENING`] for an empty or degenerate buffer
/// (no usable signal), so an uncalibratable capture is a safe no-op.
#[must_use]
pub fn compute_whitening(stereo: &[f32]) -> [[f64; 2]; 2] {
    let n = stereo.len() / 2;
    let (mut saa, mut sbb, mut sab) = (0.0_f64, 0.0_f64, 0.0_f64);
    let (mut sre, mut sim) = (0.0_f64, 0.0_f64);
    for frame in stereo.chunks_exact(2) {
        let re = f64::from(frame[1]); // ch1
        let im = f64::from(frame[0]); // ch0
        saa += re * re;
        sbb += im * im;
        sab += re * im;
        sre += re;
        sim += im;
    }
    whitening_from_covariance(saa, sbb, sab, sre, sim, n)
}

/// Whitening matrix from **running covariance sums** — the same math as
/// [`compute_whitening`] but from accumulated `Σre²`, `Σim²`, `Σre·im`
/// plus the channel means `Σre`, `Σim` over `n` frames (`re = ch1`,
/// `im = ch0`). Lets the audio thread calibrate incrementally without
/// buffering a window. Returns [`IDENTITY_WHITENING`] for an empty or
/// degenerate accumulation.
///
/// The means are used to **DC-remove** the covariance: a real cartridge
/// / preamp often sits on a small DC offset, and folding that offset
/// into the raw second moments biases the cross term `sab`, which skews
/// the whitening so forward and reverse pitch get corrected by different
/// amounts (a reported ±pitch asymmetry). Subtracting the per-channel
/// mean measures the AC carrier ellipse only. For a clean,
/// zero-mean signal the subtraction is a no-op.
#[must_use]
#[allow(clippy::many_single_char_names)] // 2×2 linear-algebra scratch
pub fn whitening_from_covariance(
    saa: f64,
    sbb: f64,
    sab: f64,
    sre: f64,
    sim: f64,
    n: usize,
) -> [[f64; 2]; 2] {
    if n == 0 {
        return IDENTITY_WHITENING;
    }
    #[allow(clippy::cast_precision_loss)]
    let inv = 1.0 / n as f64;
    let mre = sre * inv;
    let mim = sim * inv;
    // Mean-subtracted covariance [[a, b], [b, c]].
    let a = saa * inv - mre * mre;
    let b = sab * inv - mre * mim;
    let c = sbb * inv - mim * mim;
    let det = a * c - b * b;
    if !det.is_finite() || det <= 1e-18 || !a.is_finite() || !c.is_finite() {
        return IDENTITY_WHITENING;
    }
    let tr = a + c;
    let s = ((a - c) * (a - c) + 4.0 * b * b).sqrt();
    // Scale to preserve average carrier power (= trace) after whitening.
    let k = (0.5 * tr).sqrt();
    if s < 1e-12 {
        // Already isotropic (a circle) — a pure scalar normalisation.
        let f = k / (0.5 * tr).sqrt();
        return [[f, 0.0], [0.0, f]];
    }
    // Inverse square root of the covariance via Sylvester's formula.
    let l1 = 0.5 * (tr + s);
    let l2 = 0.5 * (tr - s);
    let f1 = 1.0 / l1.sqrt();
    let f2 = 1.0 / l2.sqrt();
    let d = l1 - l2; // = s
    let off = (f1 * b - f2 * b) / d;
    [
        [k * (f1 * (a - l2) - f2 * (a - l1)) / d, k * off],
        [k * off, k * (f1 * (c - l2) - f2 * (c - l1)) / d],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::Generator;

    /// Generate `n_frames` of timecode at `rate`, decode it, return
    /// the final [`DecodeOutput`].
    fn roundtrip(rate: f64, n_frames: usize) -> DecodeOutput {
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; n_frames * 2];
        gen.render(&mut buf, rate, 0.5);
        dec.process(&buf)
    }

    /// Tolerance on the rate estimate: at unity, our coherent sum's
    /// argument resolves down to `~ 1/√(N·SNR)` rad ≈ a few mrad for
    /// noiseless synthetic input over thousands of samples. Tighten
    /// this once we have a noise model (M5.4).
    const RATE_TOL: f64 = 0.005;

    #[test]
    fn unity_rate_decodes_to_unity() {
        let out = roundtrip(1.0, 4_800);
        assert!(
            (out.rate - 1.0).abs() < RATE_TOL,
            "rate = {} (want ≈1.0)",
            out.rate
        );
        assert!(out.confidence > 0.99, "confidence = {}", out.confidence);
        assert!((out.amplitude - 0.5).abs() < 0.01);
    }

    /// Scale one channel — emulates a cartridge whose L/R gains don't
    /// match, warping the timecode Lissajous from a circle to an
    /// ellipse.
    fn imbalance_ch1(buf: &mut [f32], g: f32) {
        for frame in buf.chunks_exact_mut(2) {
            frame[1] *= g;
        }
    }

    #[test]
    fn channel_imbalance_biases_rate_low_and_whitening_removes_it() {
        let sr = 48_000.0_f32;
        let g = 1.26_f32; // ≈ 2 dB imbalance
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.5); // clean unity carrier
        imbalance_ch1(&mut buf, g);

        // Uncalibrated AND with the self-calibrating ellipse correction
        // disabled: rate reads LOW by ≈ 2G/(G²+1) — the user's "−2.3 %"
        // reproduced from a known imbalance. (With the correction on — the
        // default — this same imbalance is now fixed automatically; see
        // `ellipse_correction_fixes_gain_imbalance_without_calibration`.)
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        dec.set_ellipse_correction(false);
        let raw = dec.process(&buf);
        let predicted = 2.0 * f64::from(g) / (f64::from(g) * f64::from(g) + 1.0);
        assert!(predicted < 0.99, "sanity: predicted bias {predicted}");
        assert!(
            (raw.rate - predicted).abs() < 0.01,
            "raw rate {} should match 2G/(G²+1) = {predicted}",
            raw.rate
        );

        // Calibrate from the same signal → rate unbiased, scope circular.
        let mut dec2 = Decoder::new(Format::SeratoCv02, sr);
        dec2.calibrate(&buf);
        let fixed = dec2.process(&buf);
        assert!(
            (fixed.rate - 1.0).abs() < RATE_TOL,
            "whitened rate {} should be ≈ 1.0",
            fixed.rate
        );
        assert!(
            fixed.confidence > 0.99,
            "whitened confidence {}",
            fixed.confidence
        );
    }

    #[test]
    fn ellipse_correction_fixes_gain_imbalance_without_calibration() {
        // The self-calibrating ellipse correction (default on) removes a
        // gain-imbalance bias with no whitening calibration at all — the
        // `R = 2g/(1+g²)` factor is measured straight off the covariance.
        let sr = 48_000.0_f32;
        let g = 1.26_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.5);
        imbalance_ch1(&mut buf, g);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let out = dec.process(&buf);
        assert!(
            (out.rate - 1.0).abs() < RATE_TOL,
            "auto-corrected rate {} should be ≈ 1.0",
            out.rate
        );
    }

    #[test]
    fn whitening_is_noop_on_a_balanced_signal() {
        // A clean (already circular) signal should calibrate to ≈ identity.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.5);
        let w = compute_whitening(&buf);
        // Off-diagonals ≈ 0, diagonals ≈ equal (a scalar) — no skew.
        assert!(
            w[0][1].abs() < 1e-3 && w[1][0].abs() < 1e-3,
            "off-diag {w:?}"
        );
        assert!((w[0][0] - w[1][1]).abs() < 1e-3, "diag mismatch {w:?}");
    }

    #[test]
    fn frequency_dependent_azimuth_causes_quadratic_pitch_droop() {
        // Reproduces the field bug: a calibrated deck reads ~0% at
        // nominal but droops at the pitch extremes (observed +8%→+5%,
        // −8%→−12%). Cause: the cartridge's inter-channel quadrature
        // phase (azimuth) varies with frequency, so a static whitening
        // (corrected at the calibration frequency) leaves a residual
        // azimuth proportional to pitch — a symmetric quadratic droop,
        // invisible to the coherence metric at a 1 kHz carrier.
        let sr = 48_000.0_f32;
        let carrier = 1000.0_f64;
        let beta = 3.0_f64; // radians of azimuth per unit pitch
        for &p in &[-0.08_f64, 0.0, 0.08] {
            let f = carrier * (1.0 + p);
            let w = std::f64::consts::TAU * f / f64::from(sr);
            let az = beta * p; // residual azimuth after nominal calibration
            let n = 24_000;
            let mut buf = vec![0.0f32; n * 2];
            for k in 0..n {
                #[allow(clippy::cast_precision_loss)]
                let phi = w * k as f64;
                #[allow(clippy::cast_possible_truncation)]
                {
                    buf[2 * k] = (phi + az).sin() as f32; // ch0, azimuth error
                    buf[2 * k + 1] = phi.cos() as f32; // ch1
                }
            }
            let mut dec = Decoder::new(Format::SeratoCv02, sr);
            dec.set_ellipse_correction(false); // show the raw, uncorrected bug
            let out = dec.process(&buf);
            let measured_pitch = (out.rate - 1.0) * 100.0;
            // Coherence stays ~1.0 (the metric is blind to azimuth at a
            // 1 kHz carrier) while the rate droops — exactly the field
            // signature. At ±8% the measured pitch is pulled toward zero
            // by ~3%, asymmetrically (more on the slow side), matching
            // the reported +8%→+5% / −8%→−12%.
            assert!(
                out.confidence > 0.99,
                "coherence stays high: {}",
                out.confidence
            );
            if p > 0.0 {
                assert!(
                    measured_pitch < p * 100.0 - 2.0,
                    "expected droop at +pitch, got {measured_pitch:.2}"
                );
            } else if p < 0.0 {
                assert!(
                    measured_pitch < p * 100.0 - 2.0,
                    "expected larger droop at -pitch, got {measured_pitch:.2}"
                );
            }
        }
    }

    #[test]
    fn azimuth_auto_correction_linearises_pitch() {
        // The self-calibrating azimuth correction (default on) reads the
        // residual quadrature error straight off the carrier and inverts
        // the droop, so the measured pitch tracks the platter linearly
        // across the range — matching Traktor's exact ±8 % instead of our
        // +5 % / −12 % droop. No slope/sweep is supplied.
        let sr = 48_000.0_f32;
        let carrier = 1000.0_f64;
        let beta = 3.0_f64;
        for &p in &[-0.08_f64, -0.04, 0.0, 0.04, 0.08] {
            let f = carrier * (1.0 + p);
            let w = std::f64::consts::TAU * f / f64::from(sr);
            let az = beta * p;
            let n = 24_000;
            let mut buf = vec![0.0f32; n * 2];
            for k in 0..n {
                #[allow(clippy::cast_precision_loss)]
                let phi = w * k as f64;
                #[allow(clippy::cast_possible_truncation)]
                {
                    buf[2 * k] = (phi + az).sin() as f32;
                    buf[2 * k + 1] = phi.cos() as f32;
                }
            }
            let mut dec = Decoder::new(Format::SeratoCv02, sr);
            let out = dec.process(&buf);
            let measured_pitch = (out.rate - 1.0) * 100.0;
            assert!(
                (measured_pitch - p * 100.0).abs() < 0.5,
                "auto-corrected pitch {measured_pitch:.2}% should track {:.1}%",
                p * 100.0
            );
        }
    }

    #[test]
    fn whitening_is_dc_offset_invariant() {
        // Calibration must measure the AC carrier ellipse, not the
        // operating point: a per-channel DC offset (from the cartridge /
        // preamp) must not change the computed whitening. Pre-fix the
        // offset leaked into the cross term and skewed the correction so
        // forward and reverse pitch were scaled differently (the reported
        // ±pitch asymmetry on a calibrated deck).
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 8_000 * 2];
        gen.render(&mut buf, 1.0, 0.4);
        let w_clean = compute_whitening(&buf);

        let mut biased = buf.clone();
        for frame in biased.chunks_exact_mut(2) {
            frame[0] += 0.10; // ch0 DC
            frame[1] -= 0.07; // ch1 DC
        }
        let w_biased = compute_whitening(&biased);

        for i in 0..2 {
            for j in 0..2 {
                assert!(
                    (w_clean[i][j] - w_biased[i][j]).abs() < 2e-3,
                    "whitening[{i}][{j}] drifted with a DC offset: clean {} vs biased {}",
                    w_clean[i][j],
                    w_biased[i][j]
                );
            }
        }
    }

    #[test]
    fn half_rate_decodes_to_half() {
        let out = roundtrip(0.5, 4_800);
        assert!(
            (out.rate - 0.5).abs() < RATE_TOL,
            "rate = {} (want ≈0.5)",
            out.rate
        );
        assert!(out.confidence > 0.99);
    }

    #[test]
    fn double_rate_decodes_to_double() {
        let out = roundtrip(2.0, 4_800);
        assert!(
            (out.rate - 2.0).abs() < RATE_TOL,
            "rate = {} (want ≈2.0)",
            out.rate
        );
    }

    #[test]
    fn reverse_unity_decodes_to_negative_unity() {
        let out = roundtrip(-1.0, 4_800);
        assert!(
            (out.rate - (-1.0)).abs() < RATE_TOL,
            "rate = {} (want ≈-1.0)",
            out.rate
        );
        assert!(out.confidence > 0.99);
    }

    #[test]
    fn stopped_decodes_to_zero_rate() {
        // At rate=0 the generator emits DC (constant ch0=0, ch1=A) —
        // but a real velocity cartridge outputs *silence* on a stopped
        // groove, and the input high-pass treats the synthetic DC the
        // same way: amplitude collapses and the presence gate drops
        // confidence, so a stopped platter reads as no-carrier exactly
        // like on real hardware. The cold-start DC *step* excites a
        // brief filter transient in the very first block (a real stop
        // decays smoothly and never sees it), so the sustained-stop
        // contract is asserted on the second block. The rate must
        // still settle to ~0 rather than some garbage value.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 0.0, 0.5);
        let _ = dec.process(&buf);
        gen.render(&mut buf, 0.0, 0.5);
        let out = dec.process(&buf);
        assert!(out.rate.abs() < 1e-3, "rate = {}", out.rate);
        assert!(
            out.confidence < 0.5,
            "stopped platter should read as no-carrier, conf = {}",
            out.confidence
        );
    }

    #[test]
    fn silence_yields_low_confidence() {
        // No signal at all → low amplitude, undefined frequency.
        // We accept any rate (it's nonsense by definition) but
        // confidence MUST be near zero so the integration layer
        // can ignore the output.
        let mut dec = Decoder::new(Format::SeratoCv02, 48_000.0);
        let buf = vec![0.0f32; 4_800 * 2];
        let out = dec.process(&buf);
        assert!(out.amplitude < 1e-6, "amplitude = {}", out.amplitude);
        assert!(out.confidence < 0.01, "confidence = {}", out.confidence);
    }

    #[test]
    fn static_low_level_input_is_not_confident() {
        // A stationary, low-level input — a stopped stylus, a DC offset,
        // or mains hum — is perfectly phase-*coherent* (s_curr ≈ s_prev)
        // yet is NOT a live carrier. The amplitude presence gate must
        // keep confidence low so the signal-quality readout doesn't show
        // a misleading "1.00" with the platter still and the level near
        // zero (the reported regression).
        let mut dec = Decoder::new(Format::SeratoCv02, 48_000.0);
        // Constant DC well below the carrier-presence floor (0.02).
        let buf = vec![0.005f32; 4_800 * 2];
        let out = dec.process(&buf);
        assert!(out.amplitude < 0.02, "amplitude = {}", out.amplitude);
        assert!(
            out.confidence < 0.3,
            "static low-level input should read low confidence, got {}",
            out.confidence
        );
    }

    #[test]
    fn position_integrates_at_unity() {
        // 1 second at unity rate should advance position by 1 second.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 48_000 * 2];
        gen.render(&mut buf, 1.0, 0.5);
        let out = dec.process(&buf);
        assert!(
            (out.position_secs - 1.0).abs() < 0.01,
            "position = {} (want ≈1.0s)",
            out.position_secs
        );
    }

    #[test]
    fn position_is_signed_under_reverse() {
        // 0.5 s forward + 0.5 s reverse → final position ≈ 0.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 24_000 * 2];

        gen.render(&mut buf, 1.0, 0.5);
        dec.process(&buf);
        gen.render(&mut buf, -1.0, 0.5);
        let out = dec.process(&buf);

        assert!(
            out.position_secs.abs() < 0.02,
            "net position = {} (want ≈0)",
            out.position_secs
        );
    }

    #[test]
    fn block_size_independent_at_unity() {
        // Decoding the same input as one big block vs many small
        // blocks should give the same final position (within a few
        // mrad of phase, i.e. ~1 µs of record time).
        let sr = 48_000.0_f32;
        let mut gen_big = Generator::new(Format::SeratoCv02, sr);
        let mut dec_big = Decoder::new(Format::SeratoCv02, sr);
        let mut big = vec![0.0f32; 9_600 * 2];
        gen_big.render(&mut big, 1.0, 0.5);
        let big_out = dec_big.process(&big);

        let mut gen_small = Generator::new(Format::SeratoCv02, sr);
        let mut dec_small = Decoder::new(Format::SeratoCv02, sr);
        let mut small = vec![0.0f32; 64 * 2];
        let mut last = DecodeOutput {
            rate: 0.0,
            position_secs: 0.0,
            amplitude: 0.0,
            confidence: 0.0,
            abs_position_frames: None,
            abs_confidence: 0.0,
        };
        for _ in 0..(9_600 / 64) {
            gen_small.render(&mut small, 1.0, 0.5);
            last = dec_small.process(&small);
        }
        assert!(
            (big_out.position_secs - last.position_secs).abs() < 1e-3,
            "big={} small={}",
            big_out.position_secs,
            last.position_secs
        );
    }

    #[test]
    fn process_is_alloc_free() {
        // Steady-state RT use: process() called over and over on the
        // audio thread. Must not allocate.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 64 * 2];
        gen.render(&mut buf, 1.0, 0.5);
        // Prime the decoder once outside the assertion.
        dec.process(&buf);

        assert_no_alloc::assert_no_alloc(|| {
            for _ in 0..100 {
                gen.render(&mut buf, 1.0, 0.5);
                let _ = dec.process(&buf);
            }
        });
    }

    /// M6: Traktor round-trip parity. The decoder math is format-
    /// agnostic — the only per-format parameter today is `carrier_hz`
    /// — so the same property tests should pass at 2 kHz (MK1) and
    /// 2.5 kHz (MK2) as at 1 kHz (Serato). Both are covered because
    /// their carriers differ by 25%; decoding MK2 vinyl with an MK1
    /// nominal would silently play back at +25% speed — exactly the
    /// bug class these property tests catch.
    fn roundtrip_format(format: Format, rate: f64, n_frames: usize) -> DecodeOutput {
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(format, sr);
        let mut dec = Decoder::new(format, sr);
        let mut buf = vec![0.0f32; n_frames * 2];
        gen.render(&mut buf, rate, 0.5);
        dec.process(&buf)
    }

    #[test]
    fn traktor_mk1_unity_decodes_to_unity() {
        let out = roundtrip_format(Format::TraktorMk1, 1.0, 4_800);
        assert!(
            (out.rate - 1.0).abs() < RATE_TOL,
            "MK1 rate = {} (want ≈1.0)",
            out.rate
        );
        assert!(out.confidence > 0.99);
        assert!((out.amplitude - 0.5).abs() < 0.01);
    }

    #[test]
    fn traktor_mk2_unity_decodes_to_unity() {
        let out = roundtrip_format(Format::TraktorMk2, 1.0, 4_800);
        assert!(
            (out.rate - 1.0).abs() < RATE_TOL,
            "MK2 rate = {} (want ≈1.0)",
            out.rate
        );
        assert!(out.confidence > 0.99);
    }

    #[test]
    fn traktor_mk1_reverse_decodes_negative() {
        let out = roundtrip_format(Format::TraktorMk1, -1.0, 4_800);
        assert!(
            (out.rate - (-1.0)).abs() < RATE_TOL,
            "MK1 reverse = {}",
            out.rate
        );
    }

    #[test]
    fn traktor_mk2_reverse_decodes_negative() {
        let out = roundtrip_format(Format::TraktorMk2, -1.0, 4_800);
        assert!(
            (out.rate - (-1.0)).abs() < RATE_TOL,
            "MK2 reverse = {}",
            out.rate
        );
    }

    #[test]
    fn traktor_mk2_4x_rate_clears_alias_band() {
        // At 2.5 kHz carrier, alias band starts at 0.5·SR/carrier
        // = 9.6× rate. Real DJ scratching tops out ~8× so we have
        // headroom — but it's the tightest of our three formats.
        // Pin a 4× rate test so any future regression that drops
        // alias-safety below 5× shows up as a test failure.
        let out = roundtrip_format(Format::TraktorMk2, 4.0, 4_800);
        assert!(
            (out.rate - 4.0).abs() < RATE_TOL * 4.0,
            "MK2 4× rate = {} (want ≈4.0, well clear of alias band)",
            out.rate
        );
    }

    #[test]
    fn traktor_position_integrates_at_unity_for_both_generations() {
        // 1 second at unity rate should advance position by 1 second
        // for both MK1 (2 kHz) and MK2 (2.5 kHz). If the
        // generator-decoder loop ever desynchronises on carrier, this
        // test moves first.
        for format in [Format::TraktorMk1, Format::TraktorMk2] {
            let sr = 48_000.0_f32;
            let mut gen = Generator::new(format, sr);
            let mut dec = Decoder::new(format, sr);
            let mut buf = vec![0.0f32; 48_000 * 2];
            gen.render(&mut buf, 1.0, 0.5);
            let out = dec.process(&buf);
            assert!(
                (out.position_secs - 1.0).abs() < 0.01,
                "{format:?} position = {} (want ≈1.0s)",
                out.position_secs
            );
        }
    }

    #[test]
    fn mk2_vinyl_decoded_as_mk1_plays_back_too_fast_by_25_percent() {
        // Critical regression test. M6 was originally shipped with
        // MK2 set to 2 kHz instead of 2.5 kHz — silent mis-routing,
        // playback would have been at 80% speed on MK2 vinyl. To
        // catch any future refactor that accidentally collapses MK1
        // and MK2 to the same carrier, we *deliberately* feed an
        // MK2-generated signal to an MK1-configured decoder and
        // assert the rate comes back at +25% (= 2500/2000), not
        // +0%. If MK1 and MK2 ever share a carrier, this test will
        // break — which is the right time to revisit format
        // proliferation.
        let sr = 48_000.0_f32;
        let mut gen_mk2 = Generator::new(Format::TraktorMk2, sr);
        let mut dec_mk1 = Decoder::new(Format::TraktorMk1, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen_mk2.render(&mut buf, 1.0, 0.5);
        let out = dec_mk1.process(&buf);
        let expected = 2500.0 / 2000.0;
        assert!(
            (out.rate - expected).abs() < 0.01,
            "MK2-vinyl-as-MK1-decoder rate = {} (want ≈{} = wrong-by-25%)",
            out.rate,
            expected
        );
    }

    #[test]
    fn traktor_silence_yields_low_confidence() {
        for format in [Format::TraktorMk1, Format::TraktorMk2] {
            let mut dec = Decoder::new(format, 48_000.0);
            let buf = vec![0.0f32; 4_800 * 2];
            let out = dec.process(&buf);
            assert!(
                out.confidence < 0.01,
                "{format:?} confidence on silence = {}",
                out.confidence
            );
        }
    }

    #[test]
    fn default_decoder_reports_no_absolute_position() {
        let out = roundtrip(1.0, 4_800);
        assert_eq!(out.abs_position_frames, None);
        assert!(out.abs_confidence < f32::EPSILON);
    }

    #[test]
    fn with_absolute_decodes_position_once_locked() {
        // Full-pipeline M6 check: AM-modulated generator → decoder with
        // absolute enabled. After enough signal to acquire, the output
        // carries an absolute position whose per-block delta matches
        // the rendered rate exactly.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        assert!(gen.enable_absolute(Format::SeratoCv02, 0.15));
        let mut dec = Decoder::with_absolute(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 4_800 * 2]; // 100 ms blocks
        gen.render(&mut buf, 1.0, 0.5);
        let first = dec.process(&buf);
        // Relative output unaffected by the AM modulation.
        assert!((first.rate - 1.0).abs() < RATE_TOL, "rate {}", first.rate);
        assert!(first.confidence > 0.9);

        gen.render(&mut buf, 1.0, 0.5);
        let second = dec.process(&buf);
        let p0 = second.abs_position_frames.expect("locked after 200 ms");
        assert!(second.abs_confidence > 0.9);

        gen.render(&mut buf, 1.0, 0.5);
        let third = dec.process(&buf);
        let p1 = third.abs_position_frames.expect("stays locked");
        // One 100 ms block at unity = 4 800 input frames of groove.
        assert!(
            (p1 - p0 - 4_800.0).abs() < 1.0,
            "abs delta {} should be ≈ 4800 frames",
            p1 - p0
        );
    }

    #[test]
    fn with_absolute_falls_back_to_relative_for_mk2() {
        // MK2's bitstream isn't decoded — `with_absolute` must degrade
        // to a plain relative decoder, not panic or mis-report.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::TraktorMk2, sr);
        let mut dec = Decoder::with_absolute(Format::TraktorMk2, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.5);
        let out = dec.process(&buf);
        assert!((out.rate - 1.0).abs() < RATE_TOL);
        assert_eq!(out.abs_position_frames, None);
        assert!(!dec.absolute_locked());
    }

    #[test]
    fn process_with_absolute_is_alloc_free() {
        // The LUT allocation happens in `with_absolute`; the hot path
        // must stay clean with the tracker running and locked.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        gen.enable_absolute(Format::SeratoCv02, 0.4);
        let mut dec = Decoder::with_absolute(Format::SeratoCv02, sr);
        let mut big = vec![0.0f32; 24_000 * 2];
        gen.render(&mut big, 1.0, 0.5);
        dec.process(&big); // acquire lock outside the assertion
        assert!(dec.absolute_locked());

        let mut buf = vec![0.0f32; 64 * 2];
        assert_no_alloc::assert_no_alloc(|| {
            for _ in 0..100 {
                gen.render(&mut buf, 1.0, 0.5);
                let _ = dec.process(&buf);
            }
        });
        assert!(dec.absolute_locked());
    }

    #[test]
    fn varying_rate_tracks_continuously() {
        // Slew the rate from 1.0 down to 0.0 in 100 steps. While the
        // decoder reports a confident lock, the rate must track within
        // tolerance — this is the closest unit-test approximation of a
        // real scratch. Near zero the carrier sweeps into the input
        // high-pass's stopband and confidence drops; the engine's lift
        // policy ignores those blocks, so they're excluded here too —
        // but the confident band must reach well below half speed.
        // 64-frame blocks: the design cadence, and since the sub-block
        // decode fix it is also exactly what the engine feeds the
        // decoder regardless of the render quantum.
        let sr = 48_000.0_f32;
        let block = 64_usize;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; block * 2];
        let mut max_err = 0.0_f64;
        let mut slowest_confident = 1.0_f64;
        for step in 0..100i32 {
            let want = 1.0 - f64::from(step) * 0.01;
            gen.render(&mut buf, want, 0.5);
            let out = dec.process(&buf);
            if out.confidence > 0.9 {
                // Tight tracking is promised while the carrier is
                // clear of the high-pass transition band; below rate
                // 0.2 (carrier < 200 Hz) the filter's group delay
                // during this very fast sweep (1.0 → 0 in 133 ms)
                // distorts the instantaneous frequency — those blocks
                // stay confident (amplitude compensation) but are
                // judged with a wider band.
                let err = (out.rate - want).abs();
                if want >= 0.2 && err > max_err {
                    max_err = err;
                }
                assert!(err < 0.08, "gross error at rate {want}: {err}");
                slowest_confident = want;
            }
        }
        assert!(max_err < 0.02, "max rate err = {max_err}");
        assert!(
            slowest_confident <= 0.12,
            "lock lost too early in the slowdown: {slowest_confident}"
        );
    }
}
