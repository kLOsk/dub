//! Tempo estimation from an onset detection function.
//!
//! Given the ODF emitted by [`crate::onset::OnsetDetector`], find the
//! tempo by autocorrelating in the lag range corresponding to
//! `[MIN_BPM, MAX_BPM]` and picking the integer lag whose
//! harmonic-mean **windowed local energy** is largest. A centroid
//! refinement on the picked lag's neighbourhood then yields the
//! fractional sub-bin BPM that the confidence tracker reports.
//!
//! ## Why windowed local-energy (M8.1+)
//!
//! Pure autocorrelation has equal-magnitude peaks at every integer
//! multiple of the true period (the classic "octave ambiguity"). For
//! a 128 BPM pulse train, `acf[P]`, `acf[2P]`, `acf[3P]`… are all
//! ≈ equal, and a naïve picker chooses one almost at random.
//!
//! M7.5 used a *harmonic sum* score: for each candidate `L`, sum
//! `acf[k·L]` for `k = 1, 2, …` up to `max_lag`. That correctly
//! suppresses the "picked 2P instead of P" failure mode on pure
//! pulse trains — at `2P`, the odd harmonics `3P, 5P, …` land at
//! non-peak lags and contribute zero, so `SUM(2P) < SUM(P)`. The
//! hidden cost: smaller `L` gets summed `floor(max_lag / L)` terms,
//! so faster tempos systematically score higher when their own odd
//! harmonics happen to fall on real peaks. That happens whenever
//! the ODF has sub-beat ostinato — hi-hats on every 8th note in
//! hip-hop, ride patterns in house, etc. The M8 production bug was
//! exactly this: a Diamond D track at 100 BPM was detected as 200
//! BPM because the hi-hat ODF energy made every `k · P/2` lag a
//! real acf peak, and the sum at `L = P/2` had `2×` the harmonics
//! of `L = P`.
//!
//! M8.1 fix part 1: score by the harmonic *mean*, not sum. Mean
//! removes the "more terms = bigger score" bias completely.
//!
//! M8.1 fix part 2 — *windowed local energy*. Real beat periods are
//! almost never integer multiples of the ODF sample interval. For
//! 140 BPM @ 48 kHz the true period is 40.18 ODF samples, so the
//! discrete spike pattern lands most consecutive-beat pairs in bin
//! 40 with a few in bin 41 — and analogously, skip-1 pairs land
//! mostly in bin 80 with a few in 81. The *total* energy under each
//! periodic peak is the same (as it must be for a periodic signal),
//! but the distribution across bins differs at each harmonic.
//! Parabolic-vertex height of either smoothed or raw ACF depends on
//! this distribution asymmetry — a wider shoulder pulls the vertex
//! up — so it consistently overshoots at `2P` versus `P`. That
//! structural bias was the regression that broke
//! `click_track_works_at_44100_hz` and the streaming tests during
//! M8.1 development.
//!
//! Window sum (`local(lag) = sum acf_raw[lag-W..=lag+W]`, with
//! `W = 2`) is *invariant* to where the energy sits within the
//! window. The total integrates the same way regardless of bin
//! split, so on clean periodic signals `score(P) ≡ score(2P)` to
//! within float epsilon, and the smaller-lag tiebreak fires only on
//! those genuine octave ties without ever swallowing real signal.
//!
//! M8.1 fix part 3 — *centroid refinement*. The same invariance
//! that makes the score robust erases sub-integer position info, so
//! the reported BPM is finally a centroid (energy-weighted mean
//! position) over the picked lag's window. Centroid evaluates to
//! the true continuous lag for any bin-split distribution of
//! periodic-peak energy, which is what gets the 128 and 174 BPM
//! synthetic tracks to land within `±1 BPM` of ground truth.
//!
//! ### Octaves
//!
//! Which octave a beat is counted in is not in the audio: a kick-snare
//! backbeat at 174 BPM repeats just as strongly at 87, and a 128 BPM
//! four-on-the-floor at 64. Between M11c.3 and 2026-11-19 a tempo prior
//! over 60–200 BPM and nine genre-specific rejection rules tried to
//! guess; scored against 1,758 grids the operator had locked in Traktor
//! they agreed on 62 %, and most of the misses were not octaves at all
//! but two-thirds and four-thirds readings the rules produced by
//! throwing the right candidate away. The search is now one octave
//! wide ([`TempoOctave`]), chosen by genre or by the DJ, and the score
//! above is the whole decision: 96 % on the same songs.
//!
//! One octave still holds a tempo and its two-thirds or four-thirds,
//! and the harmonic score cannot part them when they nearly tie: a
//! beat's third harmonic is its two-thirds rival's beat. Those ties are
//! settled on the beat's duple hierarchy — its quarters, halves, two
//! and four beats ([`settle_three_two`]). A half or double reading is a
//! press of ×2; a two-thirds one is a number a DJ cannot fix at a
//! glance.
//!
//! Confidence is still the *normalized cross-correlation* at the
//! fundamental peak — `acf[P] / acf[0]`. For a perfectly periodic
//! signal this approaches 1.0; for noise it tends toward 0. Below
//! [`DETECTION_THRESHOLD`] we refuse the estimate entirely
//! (returning `None`) so the caller can't confuse "no detection"
//! with "very low confidence detection".

use crate::{BpmEstimate, BpmRange, TempoOctave};

/// Below this normalized-cross-correlation ratio we declare "no
/// detection" rather than reporting a low-confidence estimate. Tuned to
/// pass the single-click / silence honesty tests while not rejecting
/// genuinely weak-but-real beats. Re-evaluate when the streaming driver
/// arrives in M8 and we have real-music data to calibrate against.
const DETECTION_THRESHOLD: f64 = 0.05;

/// How the ODF baseline is removed before autocorrelation.
///
/// [`DetrendMode::Global`] (default) subtracts a single whole-ODF mean —
/// cheap, and correct when the ODF floor is flat. [`DetrendMode::Local`]
/// subtracts an **asymmetric sliding-window mean** (the Mixxx / qm-dsp
/// `adaptiveThreshold`, `p_pre = 8` / `p_post = 7`), which additionally
/// removes *slow baseline drift* — build-ups, risers, filter sweeps —
/// that the global mean leaves in to drag the autocorrelation.
///
/// `Local` is an **experiment** pending a corpus A/B win
/// (`docs/investigations/BPM-DETECTOR-V2-INVESTIGATION.md` §7.4 item 2).
/// It is selected only when the environment variable
/// `DUB_BPM_DETREND=local` is set, so the default Classic behaviour is
/// byte-for-byte unchanged. The maintainer runs the real-music corpus
/// with and without the var to decide promotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetrendMode {
    Global,
    Local,
}

/// Window half-widths for [`DetrendMode::Local`] (qm-dsp `adaptiveThreshold`
/// defaults): `p_pre` samples before, `p_post` after the current sample.
const DETREND_LOCAL_PRE: usize = 8;
const DETREND_LOCAL_POST: usize = 7;

/// Select the detrend mode from the environment. Default `Global`; only
/// `DUB_BPM_DETREND=local` switches to the experimental local detrend.
fn detrend_mode_from_env() -> DetrendMode {
    match std::env::var("DUB_BPM_DETREND").as_deref() {
        Ok("local") => DetrendMode::Local,
        _ => DetrendMode::Global,
    }
}

/// Detrend + half-wave rectify the ODF, removing the baseline that would
/// otherwise dominate the autocorrelation at every lag.
fn detrend_odf(odf: &[f32], mode: DetrendMode) -> Vec<f32> {
    match mode {
        DetrendMode::Global => {
            #[allow(clippy::cast_precision_loss)]
            let mean = odf.iter().sum::<f32>() / odf.len() as f32;
            odf.iter().map(|&v| (v - mean).max(0.0)).collect()
        }
        DetrendMode::Local => local_mean_detrend(odf, DETREND_LOCAL_PRE, DETREND_LOCAL_POST),
    }
}

/// Subtract an asymmetric sliding-window mean (`[i - p_pre, i + p_post]`,
/// clamped at the edges) from each ODF sample, then half-wave rectify.
/// Unlike the global mean this tracks and removes a slowly-moving ODF
/// floor, so the autocorrelation sees only transient-relative energy.
fn local_mean_detrend(odf: &[f32], p_pre: usize, p_post: usize) -> Vec<f32> {
    let n = odf.len();
    let mut out = vec![0.0f32; n];
    for (i, slot) in out.iter_mut().enumerate() {
        let lo = i.saturating_sub(p_pre);
        let hi = (i + p_post + 1).min(n);
        let mut sum = 0.0f32;
        for &v in &odf[lo..hi] {
            sum += v;
        }
        #[allow(clippy::cast_precision_loss)]
        let local_mean = sum / (hi - lo) as f32;
        *slot = (odf[i] - local_mean).max(0.0);
    }
    out
}

/// Goertzel magnitude of `x` at the tone whose period is exactly `lag`
/// samples — the Fourier-tempogram bin for the tempo an ACF lag
/// represents. `|Σ_n x[n] · e^{-j 2π n / lag}|`, computed with the
/// Goertzel recurrence so the inner loop is trig-free.
fn goertzel_mag(x: &[f32], lag: usize) -> f64 {
    if lag < 2 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let omega = 2.0 * std::f64::consts::PI / lag as f64;
    let coeff = 2.0 * omega.cos();
    let mut s1 = 0.0f64;
    let mut s2 = 0.0f64;
    for &v in x {
        let s = f64::from(v) + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt()
}

/// Whether the DFT×ACF cross-tempogram experiment is enabled
/// (`DUB_BPM_TEMPOGRAM=1`). Default off — Classic behaviour is unchanged.
fn tempogram_enabled_from_env() -> bool {
    std::env::var("DUB_BPM_TEMPOGRAM").as_deref() == Ok("1")
}

/// Per-candidate cross-tempogram weights for lags `[lag_min, lag_max]`,
/// normalized so the strongest is `1.0`; `None` when the ODF has no
/// periodic energy.
///
/// The autocorrelation has peaks at every **sub-multiple** of the true
/// period — the half-tempo `174 → 87` failure mode the genre rule-forest
/// exists to undo. A Fourier tempogram peaks at integer **multiples** of
/// the tempo instead, so it is ~zero at the sub-harmonic. Multiplying the
/// ACF score by the normalized DFT magnitude keeps only the periodicity
/// present in **both**, suppressing the sub-harmonic peak the ACF alone
/// cannot. EXPERIMENT pending a corpus A/B win
/// (`docs/investigations/BPM-DETECTOR-V2-INVESTIGATION.md` §7.4 item 1).
///
/// **Empirical note (smoke test).** This only helps a *pure* sub-harmonic
/// artifact — a slow octave with no real ODF periodicity. When the wrong
/// octave is a *real* periodicity (a hi-hat ostinato at 2×, DnB's
/// kick-kick spacing at ½×), the DFT magnitude there is genuinely high, so
/// the blanket weight can *boost* the wrong octave: with this enabled, the
/// `genre_octave` hip-hop-90 fixture flips to 180 (the real hi-hat rate).
/// Pure click tracks (`known_bpm`) are unaffected. So this is *not* a clean
/// win — it reinforces §4's overlapping-classes finding, and any promotion
/// would need a sub-harmonic-restricted variant proven on the real corpus.
fn tempogram_weights(detrended: &[f32], lag_min: usize, lag_max: usize) -> Option<Vec<f64>> {
    if lag_max < lag_min {
        return None;
    }
    let mut w: Vec<f64> = (lag_min..=lag_max)
        .map(|lag| goertzel_mag(detrended, lag))
        .collect();
    let max = w.iter().copied().fold(0.0f64, f64::max);
    if max <= 0.0 {
        return None;
    }
    for v in &mut w {
        *v /= max;
    }
    Some(w)
}

/// Internal helper: unbiased autocorrelation at a single lag.
///
/// Returns `sum(x[i] * x[i + lag]) / (N - lag)` so longer lags
/// aren't penalised by having fewer terms to sum. This gives every
/// lag a "fair" per-pair magnitude, which matters for the
/// harmonic-mean score: biased ACF (with a `1 - lag/N` taper)
/// would shift the picker toward shorter lags structurally and
/// re-introduce the M8 hip-hop 2× failure mode that the log-band
/// ODF was designed to eliminate.
fn autocorr_at(detrended: &[f32], lag: usize) -> f64 {
    let n = detrended.len() - lag;
    let mut sum = 0.0f64;
    for i in 0..n {
        sum += f64::from(detrended[i]) * f64::from(detrended[i + lag]);
    }
    #[allow(clippy::cast_precision_loss)]
    {
        sum / (n as f64)
    }
}

/// How many harmonics deep to look. `acf` is computed up to
/// `HARMONIC_DEPTH * lag_max`; deeper means more octave-error
/// suppression at proportional cost.
const HARMONIC_DEPTH: usize = 4;

/// Cap on the number of harmonics scored per candidate. The harmonic
/// mean is *not* invariant under additional harmonics that drift past
/// the 3-tap smoothing window — if we include them, fast tempos (with
/// many fittable harmonics) get dragged down by drift while slow
/// tempos (with few harmonics) keep clean ones. That bias flips the
/// 140 BPM case to 70 BPM. Fixing the count at 4 puts every candidate
/// on equal footing in the same drift regime, which is exactly what
/// the harmonic-mean score requires for fair comparison across L.
///
/// 4 is chosen because:
///
/// * `4 × T_lag_max = max_lag` at our parameters, so the slowest
///   candidate naturally sees 4 harmonics without needing extension.
/// * For dnb at 174 BPM and shorter, 4 harmonics is `4 / (174 / 60)`
///   ≈ 1.4 seconds of acf evidence — enough to reject sub-multiples.
/// * For hip-hop at 100 BPM, 4 harmonics is plenty to distinguish
///   `L = P` (4 full-tempo peaks) from `L = P/2` (alternating
///   full/half peaks). The genre_octave acceptance tests confirm.
const MAX_HARMONICS: usize = 4;

/// Below this many harmonics-in-range, the mean isn't statistically
/// meaningful; we fall back to the fundamental alone. Hit only at
/// the very slowest candidates where `2 · lag_max > max_lag`.
const MIN_HARMONICS_FOR_MEAN: usize = 2;

/// Relative tolerance for the "true tie → faster-tempo" preference.
/// When two candidates' harmonic-mean scores agree within this
/// fraction of the larger score, we prefer the *smaller* refined
/// lag (faster BPM).
///
/// 1 % is appropriate here because the biased autocorrelation
/// already gives every octave its structural attenuation
/// (`(N-2P)/(N-P) ≈ 5–10 %` at typical 8–10 s ODFs). The remaining
/// noise to absorb is just float-epsilon (~1e-12) plus tiny
/// streaming-mode residuals from ODF length changing between
/// recomputes. 1 % gives margin without re-introducing the bias
/// the parabolic refinement was meant to remove.
///
/// Pure pulse trains where `P` and `2P` are *exactly* equal in
/// biased ACF (mathematically impossible — they can't tie unless
/// `N = ∞`) would fall into this window; that's the
/// pulse-train-octave case we explicitly want to default to the
/// faster tempo on.
const SCORE_TIE_REL_TOL: f64 = 0.01;

/// The lags searched: every whole ODF lag whose tempo the caller's
/// `range` allows and that sits in `octave`.
///
/// A `range` already narrower than an octave is the caller's own
/// decision about where the tempo is (a tap, a test), and is searched
/// as given. Anything wider is cut to the octave — `[low, 2 × low)`,
/// half-open, so no lag and its double are ever both candidates.
fn search_lags(odf_sample_rate: f64, range: BpmRange, octave: TempoOctave) -> (usize, usize) {
    let beat = 60.0 * odf_sample_rate;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = |lag: f64| lag.max(0.0) as usize;
    let (range_min, range_max) = (
        whole((beat / range.max).floor()),
        whole((beat / range.min).ceil()),
    );
    if range.max < range.min * 2.0 {
        return (range_min, range_max);
    }
    let octave_min = whole((beat / octave.high()).floor()) + 1;
    let octave_max = whole((beat / octave.low()).floor());
    (range_min.max(octave_min), range_max.min(octave_max))
}

/// Every candidate lag in one octave, scored: what [`estimate_tempo`]
/// picks from, kept apart so a diagnostic can read the whole table.
pub(crate) struct LagScores {
    /// The first lag scored; `scores[i]` is lag `lag_min + i`.
    pub(crate) lag_min: usize,
    /// Harmonic-mean local energy per lag (`NEG_INFINITY` where none).
    pub(crate) scores: Vec<f64>,
    /// The raw autocorrelation, lag 0 up to [`Self::max_lag`].
    pub(crate) acf_raw: Vec<f64>,
    /// The last lag `acf_raw` holds.
    pub(crate) max_lag: usize,
}

/// Window half-width of a peak's local energy; see [`score_lags`].
const WINDOW: usize = 2;

impl LagScores {
    /// A peak's total energy: `acf_raw` summed over `lag ± WINDOW`.
    pub(crate) fn local(&self, lag: usize) -> f64 {
        if lag > self.max_lag {
            return 0.0;
        }
        let lo = lag.saturating_sub(WINDOW);
        let hi = (lag + WINDOW).min(self.max_lag);
        self.acf_raw[lo..=hi].iter().sum()
    }
}

/// Score every lag `search_lags` allows. `None` when there is too
/// little ODF for the slowest candidate, or no energy in it at all.
pub(crate) fn score_lags(
    odf: &[f32],
    odf_sample_rate: f64,
    range: BpmRange,
    octave: TempoOctave,
) -> Option<LagScores> {
    let (lag_min, lag_max) = search_lags(odf_sample_rate, range, octave);
    if lag_min < 2 || lag_max <= lag_min {
        return None;
    }

    if odf.len() < lag_max * 2 {
        return None;
    }

    // Detrend + half-wave rectify. Removes the baseline that would
    // otherwise dominate autocorrelation at every lag. Default is the
    // whole-ODF global mean; `DUB_BPM_DETREND=local` opts into the
    // experimental sliding-window detrend (see [`DetrendMode`]).
    let detrended = detrend_odf(odf, detrend_mode_from_env());

    // Pre-compute autocorrelation up to HARMONIC_DEPTH × lag_max (or
    // the end of the ODF, whichever comes first) so the harmonic-sum
    // step is just an array lookup per candidate.
    let max_lag = (lag_max * HARMONIC_DEPTH).min(detrended.len().saturating_sub(1));
    let mut acf_raw = vec![0.0f64; max_lag + 1];
    for (lag, slot) in acf_raw.iter_mut().enumerate() {
        *slot = autocorr_at(&detrended, lag);
    }

    if acf_raw[0] < 1e-12 {
        // No energy → no detection. Covers silence and post-detrending
        // flat signals.
        return None;
    }

    // Per-candidate windowed local-energy scoring.
    //
    // For each integer lag `lo` in [lag_min, lag_max], compute the
    // peak's *total local energy*:
    //
    //   local(lo) = sum of acf_raw over [lo - W, lo + W]
    //
    // and score by the harmonic mean of `local(k · lo)` for
    // `k = 1..=MAX_HARMONICS`. The picker chooses the best integer
    // lag; a final parabolic refinement (3-pt vertex of `local`
    // around the best lag) gives sub-sample BPM precision.
    //
    // Why this and not the smoothed-ACF parabolic-vertex height that
    // earlier M8.1 iterations used: when the true beat period is
    // fractional (e.g. 140 BPM @ 48 kHz → P = 40.18 lag), the
    // discrete ODF spike pattern lands most consecutive-beat pairs
    // in bin 40 with a few in bin 41, and analogously bin 80 vs 81
    // for the skip-1 pairs. The *total* energy under each periodic
    // peak is the same (as it should be for a periodic signal), but
    // the distribution across bins differs: bin 40 has a sharp
    // left shoulder (one tall bin, one shorter bin) while bin 80 is
    // more even (two near-equal bins). Parabolic-vertex height of
    // either smoothed or raw ACF depends on this distribution
    // asymmetry — a wider shoulder pulls the vertex up — so it
    // consistently overshoots at 2P versus P. That's the structural
    // bias that earlier iterations papered over with broad
    // tie-tolerance / biased-raw tiebreaks. The 140 BPM @ 48 kHz
    // 10-second click test exposed that paper as too thin: the gap
    // grew with ODF length until it exceeded any reasonable
    // tolerance.
    //
    // Window sum is *invariant* to where the energy sits within the
    // window — it just integrates. With `W = 2` (5-bin window) the
    // worst-case fractional period gives both bins of the bin-split
    // energy plus quiet bins on each side. Adjacent harmonic windows
    // don't overlap as long as the harmonic spacing exceeds `2W +
    // 1 = 5`; at `MAX_HARMONICS = 4` and `lag_min ≈ 29` (200 BPM at
    // our typical ODF rates), the 4th-harmonic windows around `4·lo`
    // and `4·(lo+1)` are 4 lag apart and 5 wide — touching but not
    // overlapping. (For the slowest tempos near `lag_max ≈ 94` only
    // 1–2 harmonics fit anyway.)
    //
    // On clean periodic signals `score(P) = score(2P)` within float
    // epsilon, so the smaller-lag tiebreak fires only on those
    // genuine octave ties without ever swallowing real signal.
    //
    // Cost: 5 ACF lookups per harmonic instead of 1. With
    // `MAX_HARMONICS = 4` and ~67 integer lag candidates in the
    // 60–200 BPM range at our ODF rates, that's `67 · 4 · 5 = 1340`
    // lookups per `recompute()` — well under the M8 1 ms budget.
    let mut table = LagScores {
        lag_min,
        scores: Vec::with_capacity(lag_max - lag_min + 1),
        acf_raw,
        max_lag,
    };

    // Optional DFT×ACF cross-tempogram weighting (`DUB_BPM_TEMPOGRAM=1`).
    // When enabled, each candidate's harmonic-mean ACF score is multiplied
    // by the normalized Fourier-tempogram magnitude at that tempo, which
    // suppresses the sub-harmonic (half-tempo) peak the ACF alone retains.
    // Off by default: `tempogram` is `None` and the fold is a no-op.
    let tempogram = if tempogram_enabled_from_env() {
        tempogram_weights(&detrended, lag_min, lag_max)
    } else {
        None
    };

    for lo in lag_min..=lag_max {
        let mut score_sum = 0.0f64;
        let mut k_count = 0usize;
        for k in 1..=MAX_HARMONICS {
            let probe = k * lo;
            if probe > max_lag {
                break;
            }
            score_sum += table.local(probe);
            k_count += 1;
        }
        let raw_score = if k_count == 0 {
            f64::NEG_INFINITY
        } else if k_count < MIN_HARMONICS_FOR_MEAN {
            table.local(lo)
        } else {
            #[allow(clippy::cast_precision_loss)]
            let count_f = k_count as f64;
            score_sum / count_f
        };
        let raw_score = match &tempogram {
            Some(w) if raw_score.is_finite() => raw_score * w[lo - lag_min],
            _ => raw_score,
        };
        table.scores.push(raw_score);
    }
    Some(table)
}

/// How close a three-two rival must score to the winner to be weighed
/// against it by [`duple_support`].
const THREE_TWO_TIE: f64 = 0.9;
/// The multiples of a beat that repeat in duple time: the quarter, the
/// half, the beat, two and four beats. No third: three times a lag is
/// exactly what makes a two-thirds rival score as well as the beat.
const DUPLE_MULTIPLES: [f64; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
/// Lag ratios of the three-two family inside one octave.
const THREE_TWO_RATIOS: [f64; 4] = [4.0 / 3.0, 0.75, 1.5, 2.0 / 3.0];

/// How strongly `lag`'s duple hierarchy repeats: the mean local energy
/// at [`DUPLE_MULTIPLES`] of it.
fn duple_support(table: &LagScores, lag: usize) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let lags = DUPLE_MULTIPLES
        .iter()
        .map(|k| (k * lag as f64).round() as usize)
        .filter(|&l| l <= table.max_lag);
    let (sum, n) = lags.fold((0.0, 0usize), |(sum, n), l| (sum + table.local(l), n + 1));
    #[allow(clippy::cast_precision_loss)]
    let mean = sum / n.max(1) as f64;
    mean
}

/// The winner, or a rival in its three-two family (two-thirds,
/// four-thirds and their octaves) that scores nearly as well and whose
/// duple hierarchy repeats more strongly.
///
/// A half or double reading is a press of ×2 away; a two-thirds one is
/// a number a DJ cannot fix at a glance (Daniel, 2026-11-28). Where the
/// two score within [`THREE_TWO_TIE`] of each other the harmonic score
/// cannot tell them apart — its third harmonic is the rival's beat — so
/// they are weighed on the beat's quarters, halves, doubles and bars
/// instead. Read as a THRU deck reads (no genre, a minute of music),
/// 1,769 songs locked in Traktor went from 54 three-two readings to 32,
/// and from 70 to 41 on their first 20 s
/// (`tests/traktor_grid_corpus.rs`).
fn settle_three_two(table: &LagScores, winner: usize) -> usize {
    let winner_score = table.scores[winner - table.lag_min];
    if winner_score <= 0.0 {
        return winner;
    }
    let (mut best, mut best_support) = (winner, duple_support(table, winner));
    for (i, &score) in table.scores.iter().enumerate() {
        let lag = table.lag_min + i;
        if score < THREE_TWO_TIE * winner_score {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = lag as f64 / winner as f64;
        if !THREE_TWO_RATIOS
            .iter()
            .any(|r| (ratio / r - 1.0).abs() <= 0.03)
        {
            continue;
        }
        let support = duple_support(table, lag);
        if support > best_support {
            (best, best_support) = (lag, support);
        }
    }
    best
}

/// Estimate tempo from an ODF: the strongest periodicity inside
/// `octave`, within `range`.
///
/// `odf_sample_rate` is the ODF's own sample rate in Hz — typically
/// `audio_sr / HOP_SIZE`. The function does no audio-rate scaling
/// itself; it operates purely in ODF time.
///
/// There is no prior and no genre rule here. Which octave a tempo is
/// counted in is not in the audio; the caller says, with `octave`
/// (see [`crate::OctaveProfile::tempo_octave`]). Inside one octave the
/// harmonic-mean score below picks the same tempo family as a
/// hand-locked Traktor grid on 96 % of 1,758 songs.
pub(crate) fn estimate_tempo(
    odf: &[f32],
    odf_sample_rate: f64,
    range: BpmRange,
    octave: TempoOctave,
) -> Option<BpmEstimate> {
    let table = score_lags(odf, odf_sample_rate, range, octave)?;
    let LagScores {
        lag_min,
        ref scores,
        ref acf_raw,
        max_lag,
    } = table;
    let acf_zero = acf_raw[0];
    let local = |lag: usize| table.local(lag);

    // The best wins, and a tie goes to the faster tempo.
    let mut max_raw_score = f64::NEG_INFINITY;
    let mut best_lo: Option<usize> = None;
    for (i, &raw_score) in scores.iter().enumerate() {
        let lo = lag_min + i;
        if raw_score.is_finite() {
            let better = if !max_raw_score.is_finite() {
                true
            } else {
                let tie_window = max_raw_score.abs() * SCORE_TIE_REL_TOL;
                if raw_score > max_raw_score + tie_window {
                    true
                } else if (raw_score - max_raw_score).abs() <= tie_window {
                    best_lo.is_none_or(|prev| lo < prev)
                } else {
                    false
                }
            };
            if better {
                max_raw_score = raw_score;
                best_lo = Some(lo);
            }
        }
    }

    let best_lo = settle_three_two(&table, best_lo?);

    if std::env::var("DUB_BPM_DEBUG").is_ok() {
        eprintln!(
            "DEBUG: lag_min={lag_min} lag_max={} odf_sr={odf_sample_rate:.3} \
             best_lo={best_lo} max_raw={max_raw_score:.6}",
            lag_min + scores.len() - 1
        );
        for (i, &s) in scores.iter().enumerate() {
            let lo = lag_min + i;
            #[allow(clippy::cast_precision_loss)]
            let bpm = 60.0 * odf_sample_rate / lo as f64;
            let ratio = s / max_raw_score;
            if ratio > 0.45 {
                eprintln!(
                    "  lag={lo:3} bpm={bpm:6.2} raw={s:.6} ratio={ratio:.3} L={:.3} duple={:.3}",
                    local(lo) / acf_zero,
                    duple_support(&table, lo) / acf_zero,
                );
            }
        }
    }

    // Sub-integer refinement: centroid of the local-energy window
    // on the *raw* ACF around the picked integer lag.
    //
    // The window-sum scoring above is by design invariant to where
    // the energy sits within the window — that's what made it
    // robust against bin-split asymmetry. But that same invariance
    // erases the sub-integer position information we need for
    // continuous BPM output. Without sub-integer refinement, the
    // reported BPM lands on the ODF integer-lag grid, which has
    // ~1.5–3 BPM steps in the 60–200 BPM range — coarse enough to
    // jitter the confidence-tracker hysteresis and to fail the
    // 128 / 174 BPM ± 1 acceptance gates.
    //
    // Centroid recovers the underlying fractional position because
    // it's the energy-weighted mean of bin indices. For a periodic
    // signal at fractional period P that lands `c` consecutive
    // pairs in bin `floor(P)` and `(1-c)` in bin `ceil(P)`, the
    // centroid evaluates to `floor(P) · c + ceil(P) · (1 - c) =
    // P` — the true continuous lag. The window radius `W = 2`
    // captures both flanking bins plus one quiet bin on each side
    // (which contribute nothing to the weighted sum, so the
    // centroid stays valid).
    #[allow(clippy::cast_precision_loss)]
    let best_lag_f = {
        let lo_f = best_lo as f64;
        let lo = best_lo.saturating_sub(WINDOW);
        let hi = (best_lo + WINDOW).min(max_lag);
        let mut weight_sum = 0.0f64;
        let mut moment = 0.0f64;
        for (offset, &w) in acf_raw[lo..=hi].iter().enumerate() {
            if w > 0.0 {
                weight_sum += w;
                moment += (lo + offset) as f64 * w;
            }
        }
        if weight_sum > 0.0 {
            moment / weight_sum
        } else {
            lo_f
        }
    };

    // Confidence uses the raw (unsmoothed) ACF at the picked lag's
    // local maximum, so a clean periodic signal yields confidence
    // near 1.0 regardless of how the smoothing distributed the peak
    // across adjacent bins. Sample ±1 around the picked lag to
    // capture the underlying peak height even when split across bins.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let near = best_lag_f.round() as usize;
    let raw_peak = {
        let lower = if near > 0 { acf_raw[near - 1] } else { 0.0 };
        let mid = acf_raw[near.min(max_lag)];
        let upper = if near < max_lag {
            acf_raw[near + 1]
        } else {
            0.0
        };
        lower.max(mid).max(upper)
    };
    let ratio = raw_peak / acf_zero;
    if ratio < DETECTION_THRESHOLD {
        return None;
    }

    let bpm = 60.0 * odf_sample_rate / best_lag_f;
    #[allow(clippy::cast_possible_truncation)]
    let confidence = ratio.clamp(0.0, 1.0) as f32;

    Some(BpmEstimate { bpm, confidence })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_detrend_matches_legacy_behavior() {
        // The default path must be byte-for-byte the old global-mean
        // detrend so Classic corpus results are unchanged.
        let odf = [0.0f32, 2.0, 0.0, 4.0, 0.0, 0.0];
        #[allow(clippy::cast_precision_loss)]
        let mean = odf.iter().sum::<f32>() / odf.len() as f32;
        let expected: Vec<f32> = odf.iter().map(|&v| (v - mean).max(0.0)).collect();
        let got = detrend_odf(&odf, DetrendMode::Global);
        for (g, e) in got.iter().zip(expected.iter()) {
            assert!((g - e).abs() < 1e-7);
        }
    }

    #[test]
    fn local_detrend_zeros_a_constant() {
        let odf = vec![0.7f32; 64];
        let out = local_mean_detrend(&odf, 8, 7);
        assert!(
            out.iter().all(|&v| v.abs() < 1e-6),
            "a constant ODF must detrend to ~0"
        );
    }

    #[test]
    fn local_detrend_removes_slow_ramp_keeps_spike() {
        // Slow linear baseline ramp + one sharp spike. The local detrend
        // flattens the ramp and keeps the spike; the global mean leaves a
        // large residual baseline at the ramp's high end.
        let n = 200usize;
        #[allow(clippy::cast_precision_loss)]
        let mut odf: Vec<f32> = (0..n).map(|i| i as f32 * 0.05).collect();
        odf[100] += 20.0;

        let local = local_mean_detrend(&odf, 8, 7);
        assert!(
            local[100] > 10.0,
            "spike must survive local detrend, got {}",
            local[100]
        );
        let local_baseline: f32 = local[40..60].iter().sum::<f32>() / 20.0;
        assert!(
            local_baseline < 0.5,
            "ramp baseline should be flattened by local detrend, got {local_baseline}"
        );

        let global = detrend_odf(&odf, DetrendMode::Global);
        let global_tail: f32 = global[180..200].iter().sum::<f32>() / 20.0;
        assert!(
            global_tail > local_baseline,
            "global detrend leaves more baseline drift ({global_tail}) than local ({local_baseline})"
        );
    }

    #[test]
    fn local_detrend_is_non_negative() {
        let odf = [5.0f32, 0.0, 0.0, 0.0, 0.0];
        let out = local_mean_detrend(&odf, 2, 2);
        assert!(
            out.iter().all(|&v| v >= 0.0),
            "half-wave rectify must clamp negatives"
        );
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn sine_period(n: usize, period: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / period).sin() as f32)
            .collect()
    }

    #[test]
    fn goertzel_selects_fundamental_not_subharmonic() {
        // A pure period-40 sinusoid: the DFT magnitude at lag 40 (the true
        // period) must dominate lag 80 (the half-tempo sub-harmonic). This
        // is the property that suppresses the 174→87 octave error.
        let x = sine_period(4000, 40.0);
        let at_fundamental = goertzel_mag(&x, 40);
        let at_subharmonic = goertzel_mag(&x, 80);
        assert!(
            at_fundamental > 10.0 * at_subharmonic,
            "DFT at the true period ({at_fundamental}) must dominate the \
             half-tempo sub-harmonic ({at_subharmonic})"
        );
    }

    #[test]
    fn tempogram_weights_peak_at_true_period() {
        let x = sine_period(2000, 50.0);
        let w = tempogram_weights(&x, 30, 120).expect("periodic signal has weights");
        let max = w.iter().copied().fold(0.0f64, f64::max);
        assert!(
            (max - 1.0).abs() < 1e-9,
            "weights must be normalized to max 1.0"
        );
        let argmax = w
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0
            + 30;
        assert!(
            argmax.abs_diff(50) <= 1,
            "peak weight should land at lag ~50, got {argmax}"
        );
    }

    #[test]
    fn tempogram_weights_none_on_silence() {
        let x = vec![0.0f32; 1000];
        assert!(tempogram_weights(&x, 30, 120).is_none());
    }

    #[test]
    fn tempogram_off_by_default() {
        assert!(
            !tempogram_enabled_from_env(),
            "the cross-tempogram experiment must be off unless DUB_BPM_TEMPOGRAM=1"
        );
    }

    #[test]
    fn empty_odf_returns_none() {
        assert!(estimate_tempo(&[], 100.0, BpmRange::DEFAULT, TempoOctave::DEFAULT).is_none());
    }

    #[test]
    fn flat_odf_returns_none() {
        let odf = vec![0.0f32; 500];
        assert!(estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, TempoOctave::DEFAULT).is_none());
    }

    #[test]
    fn perfectly_periodic_odf_recovers_period() {
        // Synthetic ODF: a pulse train every 50 samples at odf_sr = 100
        // → period 0.5 s → 120 BPM exactly.
        let mut odf = vec![0.0f32; 1000];
        for i in (0..odf.len()).step_by(50) {
            odf[i] = 1.0;
        }
        let est = estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, TempoOctave::DEFAULT)
            .expect("should detect");
        assert!(
            (est.bpm - 120.0).abs() < 0.5,
            "expected ~120 BPM, got {}",
            est.bpm
        );
        assert!(est.confidence > 0.5);
    }

    #[test]
    fn period_at_lag_min_boundary_doesnt_panic() {
        // A periodic ODF whose period sits at the search boundary —
        // make sure we handle the "can't take y₋₁ at lag_min" branch
        // without panic. We don't assert an exact BPM here: a pure
        // pulse train at the boundary lag has equally-strong
        // autocorrelation at every multiple of the period (the classic
        // octave ambiguity), and choosing which one is "correct" is
        // an M8+ concern that needs musical context priors.
        let mut odf = vec![0.0f32; 1000];
        for i in (0..odf.len()).step_by(30) {
            odf[i] = 1.0;
        }
        let est = estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, TempoOctave::DEFAULT)
            .expect("should detect *some* tempo");
        assert!(
            est.bpm >= crate::MIN_BPM && est.bpm <= crate::MAX_BPM,
            "tempo out of search range: {}",
            est.bpm
        );
    }

    #[test]
    fn one_spike_no_periodicity_returns_none() {
        // Single spike at the start, then flat — exactly the single-
        // click case in the integration tests. Must return None
        // (confidence 0), not a phantom tempo.
        let mut odf = vec![0.0f32; 1000];
        odf[100] = 1.0;
        assert!(estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, TempoOctave::DEFAULT).is_none());
    }

    #[test]
    fn narrow_range_constrains_search() {
        // 120 BPM pulse train, but the search range only covers
        // [60, 90]. The estimator must report the half-tempo at 60
        // BPM (the only candidate inside the range), not the true
        // 120 BPM that lies outside it.
        let mut odf = vec![0.0f32; 2000];
        for i in (0..odf.len()).step_by(50) {
            odf[i] = 1.0;
        }
        let narrow = BpmRange::new(60.0, 90.0).unwrap();
        let est = estimate_tempo(&odf, 100.0, narrow, TempoOctave::DEFAULT).expect("should detect");
        assert!(
            est.bpm >= 60.0 && est.bpm <= 90.0,
            "narrow-range BPM should stay inside [60, 90]; got {}",
            est.bpm
        );
    }

    /// A pulse every `period` ODF samples, every `accent`-th one louder.
    fn pulses(len: usize, period: usize, accent: usize) -> Vec<f32> {
        let mut odf = vec![0.0f32; len];
        for (n, i) in (0..len).step_by(period).enumerate() {
            odf[i] = if n % accent == 0 { 1.0 } else { 0.6 };
        }
        odf
    }

    fn octave(low: f64) -> TempoOctave {
        TempoOctave::from_low(low).unwrap()
    }

    #[test]
    fn the_octave_decides_between_half_and_double() {
        // 120 BPM pulses at odf_sr 100, every other one accented: 60,
        // 120 and (out of reach) 240 are all "the tempo".
        let odf = pulses(4000, 50, 2);
        let read = |low: f64| {
            estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, octave(low))
                .expect("should detect")
                .bpm
        };
        assert!((read(78.0) - 120.0).abs() < 0.5, "78-156: {}", read(78.0));
        assert!(
            (read(100.0) - 120.0).abs() < 0.5,
            "100-200: {}",
            read(100.0)
        );
        assert!((read(60.0) - 60.0).abs() < 0.5, "60-120: {}", read(60.0));
    }

    #[test]
    fn a_strong_half_tempo_does_not_cost_a_song_its_tempo() {
        // The shape that read 128 as 85.3 on every untagged
        // four-on-the-floor song: the bar accent makes the half tempo
        // score a hair higher than the tempo, a reggae rule then threw
        // the tempo out, and a prior threw the half out after it.
        let odf = pulses(4000, 50, 2);
        let est = estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, TempoOctave::DEFAULT)
            .expect("should detect");
        assert!((est.bpm - 120.0).abs() < 0.5, "got {}", est.bpm);
    }

    #[test]
    fn an_octave_never_holds_a_lag_and_its_double() {
        for low in TempoOctave::CHOICES {
            for sr in [86.132_812_5, 93.75, 100.0] {
                let (min, max) = search_lags(sr, BpmRange::DEFAULT, octave(low));
                assert!(min >= 2 && max > min, "{low} @ {sr}: {min}..={max}");
                assert!(max < 2 * min, "{low} @ {sr}: {min}..={max}");
                let bpm = |lag: usize| 60.0 * sr / lag as f64;
                assert!(bpm(max) >= low, "{low} @ {sr}: slowest {}", bpm(max));
                assert!(bpm(min) < 2.0 * low, "{low} @ {sr}: fastest {}", bpm(min));
            }
        }
    }

    #[test]
    fn a_range_under_an_octave_is_searched_as_given() {
        // A tap or a test that says "between 60 and 90" has decided;
        // the octave has nothing to add.
        let narrow = BpmRange::new(60.0, 90.0).unwrap();
        assert_eq!(search_lags(100.0, narrow, TempoOctave::DEFAULT), (66, 100));
    }

    /// A table with the winner at lag 48 and one rival, and energy at
    /// `beat`'s duple multiples twice as strong as at the winner's.
    fn tie(rival: usize, rival_score: f64, beat: usize) -> LagScores {
        let mut acf_raw = vec![0.0; 301];
        acf_raw[0] = 1.0;
        for k in [4, 2, 1] {
            acf_raw[48 / k] += 0.5;
            acf_raw[beat / k] += 1.0;
        }
        for k in [2, 4] {
            acf_raw[48 * k] += 0.5;
            acf_raw[beat * k] += 1.0;
        }
        let mut scores = vec![0.1; 36];
        scores[48 - 37] = 1.0;
        scores[rival - 37] = rival_score;
        LagScores {
            lag_min: 37,
            scores,
            acf_raw,
            max_lag: 300,
        }
    }

    /// Rig, 2026-11-28 (Daniel): a half or double tempo is a press of ×2,
    /// a two-thirds one is not. A four-thirds rival that nearly ties and
    /// whose quarters, halves and bars repeat more strongly is the beat.
    #[test]
    fn a_near_tie_in_the_three_two_family_goes_to_the_duple_beat() {
        assert_eq!(settle_three_two(&tie(64, 0.95, 64), 48), 64, "four-thirds");
        assert_eq!(settle_three_two(&tie(72, 0.95, 72), 48), 72, "three-halves");
    }

    #[test]
    fn only_a_near_tie_in_the_three_two_family_is_weighed() {
        assert_eq!(
            settle_three_two(&tie(64, 0.85, 64), 48),
            48,
            "a rival well behind keeps losing"
        );
        assert_eq!(
            settle_three_two(&tie(56, 0.99, 56), 48),
            48,
            "seven-sixths is not a three-two rival"
        );
        assert_eq!(
            settle_three_two(&tie(64, 0.95, 48), 48),
            48,
            "the winner's own hierarchy is the stronger"
        );
    }
}
