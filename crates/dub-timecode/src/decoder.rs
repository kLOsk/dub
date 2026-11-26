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
use crate::{Format, PlatterSpeed};

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

/// |rate| above which the long-lag sum reads the carrier through the
/// carrier band-pass ([`CARRIER_BP_Q`]). Below it the unfiltered sum
/// carries on as before.
const CARRIER_BAND_MIN_RATE: f64 = 0.8;

/// Q of each of the two band-pass sections around the carrier.
///
/// The long-lag sum adds every frequency's power at that frequency's
/// own angle, so anything beside the carrier line pulls the reading:
/// content below the carrier pulls it low, content above pulls it
/// high. Serato CV02 has almost nothing there (0.1 % of the power) and
/// reads true. Traktor MK1 on a real deck carries 1.1 % below the
/// carrier (its data sidebands, 300–1900 Hz), 0.4 % above, and 0.4 %
/// at the second harmonic; lopsided, they left it 0.1 % low at every
/// pitch once the ellipse was corrected (four captures, 2026-11-21).
/// Two sections at Q 2.5 cut the octave either side by 23 dB. The same
/// filter runs on both channels, so the carrier's own rotation and its
/// ellipse come through unchanged.
const CARRIER_BP_Q: f64 = 2.5;

/// How fast the band-pass centre follows the carrier, in seconds.
///
/// The centre sits on the carrier, not on its nominal frequency: left
/// at nominal with the platter at ±8 %, the carrier rides the peak's
/// slope and the content on its far side comes back in. It follows the
/// unfiltered reading, never its own output, so it cannot chase itself.
///
/// Slow on purpose: the centre moving shifts the filter's phase, which
/// reads as a frequency error while it moves.
const CARRIER_BP_FOLLOW_SECS: f64 = 0.25;

/// How close (in rate) the carrier must be to the band-pass centre for
/// the filtered sum to be read. Wider than a record's wobble and a
/// block's noise, far narrower than a brake or a scratch.
const CARRIER_BP_ON_CENTRE: f64 = 0.02;

/// Time on centre before the band-pass is read: several times its
/// ring-in (1.6 ms at Q 2.5 on a 1 kHz carrier).
const CARRIER_BP_SETTLE_SECS: f64 = 0.01;

/// Cap on the high-pass amplitude compensation (see the carrier-
/// presence block in [`Decoder::process`]). 8× covers carriers down to
/// ~40 Hz (CV02 rate ≈ 0.04); below that the signal is genuinely gone
/// and boosting further would only amplify noise.
const MAX_AMP_BOOST: f64 = 8.0;

/// Minimum |rate| for the compensation to apply: a stopped platter or
/// pure noise decodes to ≈ 0 rate and must never have its residue
/// boosted into "carrier present".
const AMP_COMP_MIN_RATE: f64 = 0.02;

/// One 2nd-order section (RBJ biquad, Direct Form 1, `f64`): a
/// Butterworth high-pass or a unity-peak band-pass. Allocation-free and
/// branch-free per sample — RT-safe.
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

    /// Unity-peak band-pass at `center_hz`.
    fn band_pass(center_hz: f64, q: f64, sample_rate: f64) -> Self {
        let mut section = Self {
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        };
        section.retune_band_pass(center_hz, q, sample_rate);
        section
    }

    /// Move a band-pass to `center_hz`, keeping its state.
    fn retune_band_pass(&mut self, center_hz: f64, q: f64, sample_rate: f64) {
        let w0 = std::f64::consts::TAU * center_hz / sample_rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b1 = 0.0;
        self.b2 = -alpha / a0;
        self.a1 = -2.0 * cos_w0 / a0;
        self.a2 = (1.0 - alpha) / a0;
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
    /// The control record this decoder reads.
    format: Format,
    /// The carrier at unity playback, in Hz: the format's carrier times
    /// the platter speed. Everything the decoder reports is relative to
    /// it, so at 45 rpm a resting platter reads rate 1.0.
    carrier_hz: f32,
    /// Groove speed relative to 33⅓ rpm (1.0, or 1.35 at 45) — see
    /// [`Decoder::set_platter_speed`]. Divides the absolute position,
    /// which the tracker counts in 33⅓ groove frames, into track frames.
    speed: f64,
    /// How fast the source sends its carrier against its format's, at the
    /// record's own speed: 1.0 for a record — see
    /// [`Decoder::set_carrier_scale`].
    carrier_scale: f64,
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
    /// The same ring for the band-passed carrier.
    lag_hist_bp: [(f64, f64); LONG_LAG_MAX],
    /// Two band-pass sections per channel, centred on the carrier —
    /// see [`CARRIER_BP_Q`].
    carrier_bp: [[BiquadHp; 2]; 2],
    /// Where the band-pass sits, as a rate (1.0 = nominal carrier).
    carrier_bp_rate: f64,
    /// How long the carrier has sat on the band-pass centre without a
    /// break — see [`CARRIER_BP_SETTLE_SECS`].
    carrier_bp_settled_secs: f64,
    lag_pos: usize,
    /// Samples seen since reset — the long-lag sum is valid once this
    /// reaches `long_lag`.
    lag_warm: usize,
    /// Format-dependent long-lag span: `floor(sr / (2.5 · carrier_hz))`
    /// clamped to `[2, LONG_LAG_MAX]`, so `|rate| ≤` ~1.25 stays
    /// unambiguous (CV02 @48 k → 16, MK1 → 9, MK2 → 7).
    long_lag: usize,
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
    /// The carrier's own cycles counted, when the position is that — see
    /// [`Decoder::set_counting`].
    count: Option<CycleCount>,
}

/// A counted carrier: its unwrapped phase in turns, and the previous raw
/// `(ch1, ch0)` sample for the next phase step.
#[derive(Debug, Clone, Copy, Default)]
struct CycleCount {
    turns: f64,
    prev: (f64, f64),
}

/// Below this |s|² (an amplitude of 0.01) there is no carrier to count:
/// a still Phase remote sends silence, and noise there has no phase.
const COUNT_FLOOR_SQ: f64 = 1e-4;

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
            format,
            carrier_hz: format.carrier_hz(),
            speed: 1.0,
            carrier_scale: 1.0,
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
            lag_hist_bp: [(0.0, 0.0); LONG_LAG_MAX],
            carrier_bp: [[BiquadHp::band_pass(
                f64::from(format.carrier_hz()),
                CARRIER_BP_Q,
                f64::from(sample_rate),
            ); 2]; 2],
            carrier_bp_rate: 1.0,
            carrier_bp_settled_secs: 0.0,
            lag_pos: 0,
            lag_warm: 0,
            long_lag: long_lag_for(sample_rate, format.carrier_hz()),
            absolute: None,
            count: None,
        }
    }

    /// Report the position as the carrier's own cycles counted, instead of
    /// a decoded position code. For a **synthesised** carrier only — MWM
    /// Phase sends a tone whose phase is its remote's count of the record's
    /// turning, so counting it is Phase's own record of where the record
    /// is, as exact as the remote. On real vinyl the count loses whole
    /// cycles to dust ticks and more where the record stops and the
    /// cartridge's output falls away (−3 to −21 ms over five minutes of
    /// steady play, +36 ms over four of juggling, against the record's own
    /// code — `dub-engine/tests/phase_drift.rs`), so a position code is the
    /// answer there instead.
    ///
    /// While counting, [`DecodeOutput::abs_position_frames`] is the count
    /// (from wherever counting started) at full confidence, so the engine
    /// drives the deck from its exact deltas and heals against it, as it
    /// does on a position code. Turning it on starts the count at zero;
    /// turning it on again when it is on changes nothing. RT-safe.
    pub fn set_counting(&mut self, on: bool) {
        if !on {
            self.count = None;
        } else if self.count.is_none() {
            self.count = Some(CycleCount::default());
        }
    }

    /// Whether the position is the counted carrier ([`Self::set_counting`]).
    #[must_use]
    pub fn counting(&self) -> bool {
        self.count.is_some()
    }

    /// The control record this decoder reads.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }

    /// The carrier at unity playback in Hz: the format's carrier times
    /// the platter speed. `rate × carrier_hz()` is the carrier the
    /// needle is reading right now.
    #[must_use]
    pub fn carrier_hz(&self) -> f32 {
        self.carrier_hz
    }

    /// Read the record as turning at `speed`. At 45 rpm the carrier runs
    /// 1.35× its 33⅓ frequency; with the speed set, that reads as rate
    /// 1.0 and every position comes out in track time, so the long-lag
    /// estimator and the carrier band-pass work where they were tuned.
    /// Left at 33⅓, a 45 rpm platter reads 1.35 and drops to the lag-1
    /// estimator, which carries a −0.3 % bias on real vinyl.
    ///
    /// Restarts the estimators' history (a few milliseconds to refill).
    /// RT-safe: scalar writes and filter retunes, no allocation.
    pub fn set_platter_speed(&mut self, speed: PlatterSpeed) {
        self.speed = speed.factor();
        self.retune();
    }

    /// The source sends its carrier `scale` times its format's at the
    /// record's own speed, so read it as that: rate, position and the
    /// count all come out at the record's speed. 1.0 for a record (the
    /// default). MWM Phase's Serato mode sends its 1 kHz 0.564 % fast.
    /// The position code's fix is the groove's and stays as it is.
    ///
    /// Restarts the estimators' history, as [`Self::set_platter_speed`]
    /// does. RT-safe.
    pub fn set_carrier_scale(&mut self, scale: f64) {
        self.carrier_scale = scale;
        self.retune();
    }

    /// Put the unity carrier where the platter speed and the carrier scale
    /// say, and retune what depends on it.
    #[allow(clippy::cast_possible_truncation)]
    fn retune(&mut self) {
        self.carrier_hz =
            (f64::from(self.format.carrier_hz()) * self.speed * self.carrier_scale) as f32;
        self.long_lag = long_lag_for(self.sample_rate, self.carrier_hz);
        self.lag_pos = 0;
        self.lag_warm = 0;
        self.carrier_bp_rate = 1.0;
        self.carrier_bp_settled_secs = 0.0;
        for section in self.carrier_bp.iter_mut().flatten() {
            section.retune_band_pass(
                f64::from(self.carrier_hz),
                CARRIER_BP_Q,
                f64::from(self.sample_rate),
            );
            section.reset();
        }
        self.ellipse_primed = false;
    }

    /// The absolute position in track frames: the counted carrier while
    /// counting, else the position code's fix, or `None` while it has
    /// none (or there is no tracker).
    fn abs_position_frames(&self) -> Option<f64> {
        if let Some(count) = self.count {
            return Some(count.turns / f64::from(self.carrier_hz) * f64::from(self.sample_rate));
        }
        self.absolute
            .as_ref()
            .and_then(AbsoluteTracker::position_frames)
            .map(|frames| frames / self.speed)
    }

    /// How sure the absolute position is: full while counting (the count
    /// is the source's own), else the position code's agreement.
    fn abs_confidence(&self) -> f32 {
        if self.count.is_some() {
            return 1.0;
        }
        self.absolute
            .as_ref()
            .map_or(0.0, AbsoluteTracker::confidence)
    }

    /// Like [`Decoder::new`], but with **absolute-position decoding**
    /// enabled (M6). Builds the format's position LUT — allocates a few
    /// MB, so this MUST run off the audio thread; `process` itself
    /// stays alloc-free. For a format whose bitstream we don't decode
    /// (Traktor MK2) this silently degrades to
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
        for section in self.carrier_bp.iter_mut().flatten() {
            section.reset();
        }
        self.carrier_bp_settled_secs = 0.0;
        if let Some(count) = self.count.as_mut() {
            *count = CycleCount::default();
        }
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
                abs_position_frames: self.abs_position_frames(),
                abs_confidence: self.abs_confidence(),
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
        let mut band_lag_re = 0.0_f64;
        let mut band_lag_im = 0.0_f64;
        let mut band_sq_re = 0.0_f64;
        let mut band_sq_im = 0.0_f64;
        let mut band_cross = 0.0_f64;
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
            let (mut band_re, mut band_im) = (re, im);
            for section in &mut self.carrier_bp[0] {
                band_re = section.process(band_re);
            }
            for section in &mut self.carrier_bp[1] {
                band_im = section.process(band_im);
            }
            band_sq_re += band_re * band_re;
            band_sq_im += band_im * band_im;
            band_cross += band_re * band_im;
            if self.lag_warm >= self.long_lag {
                let (hre, him) = self.lag_hist[self.lag_pos];
                acc_l_re += re * hre + im * him;
                acc_l_im += im * hre - re * him;
                let (hre, him) = self.lag_hist_bp[self.lag_pos];
                band_lag_re += band_re * hre + band_im * him;
                band_lag_im += band_im * hre - band_re * him;
                lag_pairs += 1;
            }
            self.lag_hist[self.lag_pos] = (re, im);
            self.lag_hist_bp[self.lag_pos] = (band_re, band_im);
            self.lag_pos = (self.lag_pos + 1) % self.long_lag;
            self.lag_warm = self.lag_warm.saturating_add(1);
            // Counting (a synthesised carrier, `set_counting`): this
            // sample's phase step, s · conj(s_prev), added up — on the raw
            // input, not the high-passed one: the high-pass shifts the
            // carrier's phase more the slower it turns, and each pass
            // through zero speed left a third of a cycle behind (0.15 ms a
            // juggle pass at 2.5 kHz). A synthesised carrier has no rumble
            // or hum to filter. Exact while the carrier is there; through
            // silence the count holds.
            if let Some(count) = self.count.as_mut() {
                let (raw_re, raw_im) = (f64::from(frame[1]), f64::from(frame[0]));
                let (prev_re, prev_im) = count.prev;
                if raw_re * raw_re + raw_im * raw_im >= COUNT_FLOOR_SQ
                    && prev_re * prev_re + prev_im * prev_im >= COUNT_FLOOR_SQ
                {
                    let step = (raw_im * prev_re - raw_re * prev_im)
                        .atan2(raw_re * prev_re + raw_im * prev_im);
                    count.turns += step / std::f64::consts::TAU;
                }
                count.prev = (raw_re, raw_im);
            }
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
        // from THIS block's covariance before the rate is corrected
        // with it, or a fresh decoder's first block goes uncorrected.
        let coherent_mag = (acc_re * acc_re + acc_im * acc_im).sqrt();
        let coherence = if mag_acc > 1e-12 {
            #[allow(clippy::cast_possible_truncation)]
            ((coherent_mag / mag_acc).clamp(0.0, 1.0) as f32)
        } else {
            0.0
        };
        // The ellipse is measured on the signal the rate is read from.
        // Near playing speed that is the band-passed carrier: a bump on
        // the record is a burst of low, one-sided power, and read off
        // the unfiltered covariance it passed for a very oval carrier —
        // the correction then held a steady +6.6 % at +3.8 % for a
        // second (Serato capture, 2026-11-21).
        //
        // The band-pass is only read while it sits on the carrier. It
        // adds 1.6 ms of delay, and a carrier sliding off its centre
        // picks up phase on the way: through a brake or a scratch the
        // filtered reading trails by several percent. So the unfiltered
        // long-lag rate says where the carrier is, the centre follows
        // that, and the filtered sum is used once the two agree.
        #[allow(clippy::cast_precision_loss)]
        let rate_wide = acc_l_im.atan2(acc_l_re)
            / self.long_lag as f64
            / (std::f64::consts::TAU * dt)
            / nominal;
        let near_playing_speed = lag_pairs > 0
            && rate_lag1.abs() > CARRIER_BAND_MIN_RATE
            && rate_lag1.abs() < LONG_LAG_MAX_RATE;
        let on_centre = near_playing_speed
            && coherence > 0.9
            && (rate_wide.abs() - self.carrier_bp_rate).abs() < CARRIER_BP_ON_CENTRE;
        // …and has sat there long enough to have rung in: its first
        // milliseconds after a start, a needle drop or a scratch are
        // the filter's own transient, not the carrier.
        #[allow(clippy::cast_precision_loss)]
        if on_centre {
            self.carrier_bp_settled_secs += n_frames as f64 * dt;
        } else {
            self.carrier_bp_settled_secs = 0.0;
        }
        let carrier_band = self.carrier_bp_settled_secs > CARRIER_BP_SETTLE_SECS;
        if coherence > 0.9 {
            if carrier_band {
                self.update_ellipse_estimate(
                    band_sq_re,
                    band_sq_im,
                    band_cross,
                    band_sq_re + band_sq_im,
                );
            } else {
                self.update_ellipse_estimate(sq_re_acc, sq_im_acc, cross_acc, mag_acc);
            }
        }

        // Prefer the long-lag phase (per-sample equivalent) inside the
        // unambiguous band: same carrier term, ~20× less
        // correlated-noise shrink — this is what puts a true 0.00 % at
        // fader 0 on a real (noisy) record.
        //
        // A residual ellipse biases it by the same law as lag-1, only
        // at the lag's angle: the carrier's image component adds
        // `|b|²·e^(−jLω)` to the sum, so `tan θ_measured = R·tan(Lω)`.
        // Dividing `R` back out of the real part undoes it in every
        // quadrant. This path used to be left uncorrected and gated on
        // roundness > 0.99 instead; a cartridge sitting on that gate
        // flapped between two estimators that disagreed by over 1 %
        // (Traktor vinyl, 2026-11-21), and one just inside it read up
        // to 0.15 % off toward ±8 %.
        let use_long_lag = lag_pairs > 0 && rate_lag1.abs() < LONG_LAG_MAX_RATE;
        #[allow(clippy::cast_precision_loss)]
        let phase_diff = if use_long_lag {
            let r = if self.ellipse_correction && self.ellipse_primed && coherence > 0.9 {
                self.ellipse_factor()
            } else {
                1.0
            };
            // Near playing speed the band-passed carrier is the one
            // to read: what sits beside the carrier line pulls the
            // unfiltered sum off it (see [`CARRIER_BP_Q`]).
            let (l_re, l_im) = if carrier_band {
                (band_lag_re, band_lag_im)
            } else {
                (acc_l_re, acc_l_im)
            };
            l_im.atan2(r * l_re) / self.long_lag as f64
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

        if near_playing_speed && coherence > 0.9 {
            let follow = 1.0 - (-block_secs_real / CARRIER_BP_FOLLOW_SECS).exp();
            self.carrier_bp_rate += follow * (rate_wide.abs() - self.carrier_bp_rate);
            self.carrier_bp_rate = self
                .carrier_bp_rate
                .clamp(CARRIER_BAND_MIN_RATE, LONG_LAG_MAX_RATE);
            let center = nominal * self.carrier_bp_rate;
            for section in self.carrier_bp.iter_mut().flatten() {
                section.retune_band_pass(center, CARRIER_BP_Q, f64::from(self.sample_rate));
            }
        }

        DecodeOutput {
            rate,
            position_secs: self.position_secs,
            amplitude,
            confidence,
            abs_position_frames: self.abs_position_frames(),
            abs_confidence: self.abs_confidence(),
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
                // ~70 ms at 64-frame blocks. Fast enough to reach a new
                // pitch's ellipse within a fraction of a second —
                // otherwise a reading taken soon after a pitch move
                // keeps the *previous* pitch's azimuth (opposite sign)
                // and skews the extremes. Not faster: the estimate now
                // corrects the steady long-lag rate too, where its
                // block-to-block noise would land in the reading (0.15
                // measured a little worse on four real captures).
                const ELLIPSE_ALPHA: f64 = 0.02;
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

/// The long-lag span for a carrier at unity: `floor(sr / (2.5 · carrier))`
/// clamped to `[2, LONG_LAG_MAX]`, so `|rate| ≤` ~1.25 stays unambiguous.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn long_lag_for(sample_rate: f32, carrier_hz: f32) -> usize {
    ((sample_rate / (2.5 * carrier_hz)) as usize).clamp(2, LONG_LAG_MAX)
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
    fn channel_imbalance_biases_rate_and_whitening_removes_it() {
        let sr = 48_000.0_f32;
        let g = 1.26_f32; // ≈ 2 dB imbalance
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.5); // clean unity carrier
        imbalance_ch1(&mut buf, g);

        // Uncalibrated AND with the self-calibrating ellipse correction
        // disabled, the long-lag angle θ = L·ω reads
        // `atan2(R·sin θ, cos θ)` with `R = 2G/(G²+1)`. (With the
        // correction on — the default — this same imbalance is fixed
        // automatically; see
        // `ellipse_correction_fixes_gain_imbalance_without_calibration`.)
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        dec.set_ellipse_correction(false);
        let raw = dec.process(&buf);
        let r = 2.0 * f64::from(g) / (f64::from(g) * f64::from(g) + 1.0);
        let theta = 16.0 * std::f64::consts::TAU * 1000.0 / 48_000.0;
        let predicted = (r * theta.sin()).atan2(theta.cos()) / theta;
        assert!(
            (predicted - 1.0).abs() > 0.004,
            "sanity: predicted bias {predicted}"
        );
        assert!(
            (raw.rate - predicted).abs() < 0.002,
            "raw rate {} should match the ellipse law, {predicted}",
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

    /// Mean decoded rate over one second of 64-frame blocks, after a
    /// short warm-up — the engine's own block size.
    fn mean_rate_on_ellipse(format: Format, rate: f64, g: f32) -> f64 {
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(format, sr);
        let mut buf = vec![0.0f32; 60_000 * 2];
        gen.render(&mut buf, rate, 0.5);
        imbalance_ch1(&mut buf, g);
        let mut dec = Decoder::new(format, sr);
        let rates: Vec<f64> = buf.chunks_exact(128).map(|c| dec.process(c).rate).collect();
        let settled = &rates[rates.len() - 750..];
        settled.iter().sum::<f64>() / 750.0
    }

    #[test]
    fn a_slightly_oval_carrier_reads_true_across_the_pitch_range() {
        // Daniel's rig, 2026-11-21: Serato vinyl read up to 0.15 % off
        // toward ±8 %, by a different amount on each deck. A cartridge
        // that is a little out of round (roundness ≈ 0.995 here) took
        // the long-lag path, which had no ellipse correction.
        for format in [Format::SeratoCv02, Format::TraktorMk1] {
            for rate in [0.92, 0.96, 1.0, 1.04, 1.08] {
                let got = mean_rate_on_ellipse(format, rate, 1.105);
                assert!(
                    (got / rate - 1.0).abs() < 2e-4,
                    "{format:?} at {rate}: read {got}"
                );
            }
        }
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn content_beside_the_carrier_does_not_pull_the_reading() {
        // Daniel's Traktor vinyl, 2026-11-21: a round carrier, 1 % of
        // the power in data sidebands below it and 0.4 % at the second
        // harmonic. The long-lag sum weighed all of it and read 0.1 %
        // low at every pitch.
        for rate in [0.92, 1.0, 1.08] {
            let sr = 48_000.0_f64;
            let w = std::f64::consts::TAU * 2000.0 * rate / sr;
            let mut dec = Decoder::new(Format::TraktorMk1, 48_000.0);
            let (mut sum, mut n) = (0.0, 0.0);
            for block in 0..1500_u32 {
                let mut buf = [0.0_f32; 128];
                for (i, frame) in buf.chunks_exact_mut(2).enumerate() {
                    let t = f64::from(block * 64 + i as u32);
                    let (mut re, mut im) = ((w * t).cos(), (w * t).sin());
                    for (ratio, amp) in [(0.55, 0.06), (0.7, 0.06), (0.85, 0.06), (2.0, 0.065)] {
                        re += amp * (ratio * w * t).cos();
                        im += amp * (ratio * w * t).sin();
                    }
                    frame[0] = (0.3 * im) as f32;
                    frame[1] = (0.3 * re) as f32;
                }
                let out = dec.process(&buf);
                if block >= 750 {
                    sum += out.rate;
                    n += 1.0;
                }
            }
            let err = (sum / n / rate - 1.0) * 100.0;
            assert!(err.abs() < 0.02, "rate {rate}: {err:+.3} % off");
        }
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn a_shaken_stylus_does_not_pull_the_reading_at_pitch() {
        // Bass from the sound system frequency-modulates the carrier:
        // even sidebands 100 Hz either side. A band-pass left on the
        // nominal carrier weighs them unevenly once the platter is
        // pitched, and reads 0.3 % low here.
        let rate = 1.066_f64;
        let w = std::f64::consts::TAU * 1000.0 * rate / 48_000.0;
        let wm = std::f64::consts::TAU * 100.0 / 48_000.0;
        let mut dec = Decoder::new(Format::SeratoCv02, 48_000.0);
        let (mut sum, mut n) = (0.0, 0.0);
        for block in 0..3000_u32 {
            let mut buf = [0.0_f32; 128];
            for (i, frame) in buf.chunks_exact_mut(2).enumerate() {
                let t = f64::from(block * 64 + i as u32);
                let phase = w * t + 0.5 * (wm * t).sin();
                frame[0] = (0.3 * phase.sin()) as f32;
                frame[1] = (0.3 * phase.cos()) as f32;
            }
            let out = dec.process(&buf);
            if block >= 1500 {
                sum += out.rate;
                n += 1.0;
            }
        }
        let err = (sum / n / rate - 1.0) * 100.0;
        assert!(err.abs() < 0.08, "{err:+.3} % off");
    }

    #[test]
    fn the_reading_does_not_jump_where_roundness_crosses_a_threshold() {
        // Same day, Traktor vinyl: at some pitches the residual ellipse
        // sat on the 0.99 roundness gate, the decoder flapped between
        // its two estimators, and they disagreed by over 1 %. Either
        // side of that old gate must now read the same.
        for format in [Format::SeratoCv02, Format::TraktorMk1] {
            let rounder = mean_rate_on_ellipse(format, 1.05, 1.14);
            let ovaler = mean_rate_on_ellipse(format, 1.05, 1.16);
            assert!(
                (rounder - ovaler).abs() < 2e-4,
                "{format:?}: {rounder} against {ovaler}"
            );
            assert!((rounder / 1.05 - 1.0).abs() < 2e-4, "{format:?}: {rounder}");
        }
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
    fn frequency_dependent_azimuth_causes_a_pitch_dependent_error() {
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
            // 1 kHz carrier) while the rate reads off, by an amount
            // that grows with the pitch — the field signature.
            assert!(
                out.confidence > 0.99,
                "coherence stays high: {}",
                out.confidence
            );
            let error = (measured_pitch - p * 100.0).abs();
            if p == 0.0 {
                assert!(error < 0.05, "no azimuth, no error: {measured_pitch:.2}");
            } else {
                assert!(
                    error > 0.3,
                    "expected a pitch error at {p}, got {measured_pitch:.2}"
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

    /// Mean rate over the last second of `secs` of a carrier at `rate`
    /// (33⅓ terms), read in 64-frame blocks with the speed set to `speed`.
    fn mean_rate_at(format: Format, speed: PlatterSpeed, rate: f64, secs: f64) -> f64 {
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(format, sr);
        let mut dec = Decoder::new(format, sr);
        dec.set_platter_speed(speed);
        let mut buf = vec![0.0f32; 64 * 2];
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let blocks = (secs * 48_000.0 / 64.0) as usize;
        let (mut sum, mut n) = (0.0, 0.0);
        for b in 0..blocks {
            gen.render(&mut buf, rate, 0.5);
            let out = dec.process(&buf);
            if b + 750 >= blocks {
                sum += out.rate;
                n += 1.0;
            }
        }
        sum / n
    }

    #[test]
    fn a_45_rpm_platter_reads_unity_once_the_speed_is_set() {
        for format in [Format::SeratoCv02, Format::TraktorMk1, Format::TraktorMk2] {
            let at_45 = PlatterSpeed::Rpm45.factor();
            let read = mean_rate_at(format, PlatterSpeed::Rpm45, at_45, 2.0);
            assert!((read - 1.0).abs() < 2e-4, "{format:?} at 45 read {read}");
            let pitched = mean_rate_at(format, PlatterSpeed::Rpm45, at_45 * 1.08, 2.0);
            assert!(
                (pitched - 1.08).abs() < 3e-4,
                "{format:?} +8 % read {pitched}"
            );
            let unset = mean_rate_at(format, PlatterSpeed::Rpm33, at_45, 2.0);
            assert!(
                (unset - at_45).abs() < 0.01,
                "{format:?} unset read {unset}"
            );
        }
    }

    #[test]
    fn counting_a_synthesised_carrier_follows_every_gesture() {
        // A Phase-like tone (flat, no code) through a juggle: play, a
        // scratch, a spin back, the remote held still (silence), play on.
        // The count is where the record is, to well under a millisecond.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::TraktorMk2, sr);
        let mut dec = Decoder::new(Format::TraktorMk2, sr);
        dec.set_counting(true);
        let mut buf = vec![0.0f32; 64 * 2];
        let mut truth = 0.0_f64;
        let block = 64.0 / 48_000.0;
        let mut play = |dec: &mut Decoder, gen: &mut Generator, truth: &mut f64, rate: f64| {
            gen.render(&mut buf, rate, 0.33);
            dec.process(&buf);
            *truth += rate * block;
        };
        for _ in 0..375 {
            play(&mut dec, &mut gen, &mut truth, 1.0);
        }
        #[allow(clippy::cast_precision_loss)]
        for i in 0..1500 {
            let rate = 2.5 * (f64::from(i) / 1500.0 * std::f64::consts::TAU * 6.0).sin();
            play(&mut dec, &mut gen, &mut truth, rate);
        }
        for _ in 0..300 {
            play(&mut dec, &mut gen, &mut truth, -4.0);
        }
        let silence = vec![0.0f32; 64 * 2];
        for _ in 0..200 {
            dec.process(&silence);
        }
        for _ in 0..375 {
            play(&mut dec, &mut gen, &mut truth, 1.0);
        }
        let counted = dec.process(&[]).abs_position_frames.expect("counting") / 48_000.0;
        assert!(
            (counted - truth).abs() < 5e-4,
            "counted {counted:.5} s, the record moved {truth:.5} s"
        );
        assert!((dec.process(&[]).abs_confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn counting_is_off_until_asked_and_restarts_only_when_turned_on() {
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::TraktorMk2, sr);
        let mut dec = Decoder::new(Format::TraktorMk2, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.33);
        assert_eq!(dec.process(&buf).abs_position_frames, None);
        dec.set_counting(true);
        gen.render(&mut buf, 1.0, 0.33);
        let p = dec.process(&buf).abs_position_frames.expect("counting");
        dec.set_counting(true);
        gen.render(&mut buf, 1.0, 0.33);
        let q = dec.process(&buf).abs_position_frames.expect("counting");
        assert!(
            (q - p - 4_800.0).abs() < 1.0,
            "a second on reset it: {p} → {q}"
        );
    }

    #[test]
    fn at_45_the_absolute_position_moves_in_track_time() {
        // The tracker counts 33⅓ groove frames; at 45 a second of play
        // covers 1.35 s of them, and the decoder hands the deck one.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        assert!(gen.enable_absolute(Format::SeratoCv02, 0.4));
        let mut dec = Decoder::with_absolute(Format::SeratoCv02, sr);
        dec.set_platter_speed(PlatterSpeed::Rpm45);
        let mut buf = vec![0.0f32; 4_800 * 2];
        let at_45 = PlatterSpeed::Rpm45.factor();
        for _ in 0..3 {
            gen.render(&mut buf, at_45, 0.5);
            dec.process(&buf);
        }
        let p0 = dec.process(&[]).abs_position_frames.expect("locked");
        for _ in 0..10 {
            gen.render(&mut buf, at_45, 0.5);
            dec.process(&buf);
        }
        let p1 = dec.process(&[]).abs_position_frames.expect("still locked");
        assert!(
            (p1 - p0 - 48_000.0).abs() < 2.0,
            "one second at 45 advanced {} frames",
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
