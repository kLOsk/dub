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
//! ### Real-music robustness note
//!
//! The windowed-energy + centroid design is calibrated against the
//! synthetic fixtures in `tests/genre_octave.rs`. Real music may
//! introduce complications we don't currently address:
//!
//! 1. *Missed beats* (a kick dropping out for one bar) lower the
//!    mean of `L = P` slightly and could flip a marginal call.
//! 2. *K-S backbeat half-tempo* — patterns like real drum-n-bass
//!    (kick on 1+3, snare on 2+4 at 174 BPM) are structurally
//!    ambiguous between 174 and 87 BPM because the autocorrelation
//!    peaks at the K-K / S-S period (lag 64) just as strongly as at
//!    the K-S alternation period (lag 32). Resolving this requires
//!    a tempo / genre prior, M9+ scope. The user explicitly
//!    accepted this same limitation for dubstep at 140 → 70 BPM.
//!
//! M9+ real-music validation may motivate switching to a more
//! robust aggregator (trimmed mean, Hodges-Lehmann) and/or a richer
//! prior. The M8.1 acceptance gate is "the user's stated genre mix —
//! reggae 65, hip-hop 90/100, rolling dnb 174 — locks at the
//! correct octave"; we hit that with the windowed local energy +
//! harmonic mean + centroid refinement combination.
//!
//! Confidence is still the *normalized cross-correlation* at the
//! fundamental peak — `acf[P] / acf[0]`. For a perfectly periodic
//! signal this approaches 1.0; for noise it tends toward 0. Below
//! [`DETECTION_THRESHOLD`] we refuse the estimate entirely
//! (returning `None`) so the caller can't confuse "no detection"
//! with "very low confidence detection".

use crate::octave_profile::{
    profile_doubletime_rejected, profile_halftime_rejected, profile_skips_hiphop_doubletime_pass,
    profile_skips_skank_pass, profile_subdivision_rejected, OctaveProfile,
};
use crate::{BpmEstimate, BpmRange};

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

/// Perceptual tempo prior (M11c.3 octave-bias fix).
///
/// Real autocorrelation has structurally-equal peaks at every
/// octave of the true period, and the harmonic-mean scorer above
/// resolves that ambiguity correctly *only* when the underlying
/// ODF energy distributes as a clean pulse train. Real drum
/// patterns aren't pulse trains: a kick-snare-kick-snare backbeat
/// at 174 BPM has the K-K skip-1 period (lag 64 ≈ 87 BPM) and
/// the K-S alternation period (lag 32 ≈ 174 BPM) carrying
/// comparable autocorrelation mass, and a syncopated hip-hop
/// beat at 95 BPM has a hi-hat ostinato at the 190 BPM rate that
/// the autocorrelation legitimately reads as periodic. The
/// scorer picks the stronger peak even when the user-perceived
/// tempo is the weaker one.
///
/// The prior is a piecewise-linear weight that biases the
/// selection toward the "mixable band" — the BPM range working
/// DJs typically beatmatch in. Calibrated empirically against the
/// M11c.3 corpus (Westside Connection at 95, Chase & Status DnB
/// tracks at 172–177, synthetic 90/100 BPM hip-hop with strong
/// 8th-note hi-hat content):
///
/// * `bpm ≤ 60` → 0.20 (penalty floor — barely-musical territory)
/// * `60 → 95`  → linear 0.20 → 1.00 (lifts 80 to 0.66, 86 to 0.79,
///   87.59 to 0.83, 90 to 0.89, 92 to 0.93)
/// * `95 → 175` → 1.00 (plateau covers hip-hop 95–105, house,
///   techno, DnB 165–175)
/// * `175 → 200` → linear 1.00 → 0.30 (steep ramp catches the
///   double-time hip-hop case at 180 / 190 reliably: 180 → 0.86,
///   190 → 0.58)
/// * `bpm ≥ 200` → 0.20
///
/// **Why the plateau starts at 95.** Real DnB's K-K skip-1
/// autocorrelation peak lands at 86–89 BPM with raw harmonic-mean
/// score *larger* than the K-S alternation peak at 172–177 BPM.
/// Pass 2 (the qualifying-candidates step) admits both peaks
/// because the raw-score ratio across the octave sits in
/// [0.76, 0.94] for every DnB track in the M11c.3 corpus. The
/// prior must therefore drive `weight(low) × score(low) <
/// weight(high) × score(high)`, i.e. `weight(low) < ratio ×
/// weight(high)`. The worst-case clean DnB track is Total Science
/// / S.P.Y. "Gangsta" (Watch The Ride remix) — lag 59 / 87.59 BPM,
/// raw 5.121, vs lag 30 / 172.27 BPM, raw 4.454, ratio **0.870**.
/// The plateau at 95 BPM gives `weight(86) = 0.79`,
/// `weight(87.59) = 0.83`, `weight(88) = 0.84`, `weight(90) =
/// 0.89`, and `weight(92) = 0.93` — every clean DnB low-octave
/// peak in the corpus lands strictly below the required 0.87
/// margin while 95 BPM and above keep full plateau weight.
///
/// **Calibration history.** Three iterations of M11c.3a were
/// required to converge on the [95, 175] boundaries:
///
/// 1. Initial plateau `[90, 178]` resolved Bedhead / Baddadan /
///    Backbone / Apocalypse. Chase & Status "Come Back" still
///    detected at 87.59 BPM because `weight(87.59) = 0.94`,
///    insufficient to overcome that track's raw ratio of 0.922.
/// 2. Plateau `[92, 175]` fixed Come Back (raised the
///    `weight(87.59)` requirement to 0.89). Total Science /
///    S.P.Y. "Gangsta" still detected at 87.59 BPM because that
///    track's raw ratio is 0.870 — harsher than Come Back's
///    0.922 — and 0.89 × raw(low) still beat 1.00 × raw(high).
/// 3. Plateau `[95, 175]` (current) drops `weight(87.59)` to
///    0.83, below the 0.87 margin required by Total Science. All
///    eight clean DnB tracks in the corpus plus all rap doubling
///    cases now resolve correctly.
///
/// A separate "**genuinely mixed-tempo**" failure case remains
/// unresolved by the prior: Benny Page "Crying Out" (Serial
/// Killaz remix) is a DnB track with a literal half-tempo reggae
/// break in the middle. The autocorrelation accumulates real
/// energy at both the DnB rate (~175 BPM) and the reggae rate
/// (~87 BPM), and the raw-score ratio against the upper octave
/// drops to 0.757 — too low for any prior that doesn't also
/// break 90 BPM hip-hop. This is a structural limitation of
/// one-pass whole-track analysis and requires either per-section
/// beat-grids (M10.5p-grid scope) or the user reaching for
/// tap-to-grid (M11c.3b).
///
/// **Why the upper ramp starts at 175.** Real DnB tops out around
/// 178 BPM (drum-and-bass standards bracket 170–178; jungle pushes
/// to 175). Setting the upper ramp start at 175 keeps every DnB
/// candidate ≤ 175 BPM at full plateau weight while penalising the
/// 178–200 BPM band where the 2× hip-hop failure modes live. The
/// double-time hip-hop failure mode (90 BPM track misdetected as
/// 180) is resolved because the synthetic and real-music 8th-note
/// hi-hat candidates land at 180 BPM, well inside the ramp where
/// the weight drops to 0.86 vs 90 BPM's ramp weight of 0.95 — a
/// 10 % margin, large enough to flip every double-time case in the
/// corpus. The 175 → 200 ramp falls at 0.70 weight per 25 BPM,
/// steep enough to drive 190 BPM down to 0.58 and reliably flip
/// the rap 95-vs-190 case even when raw scores are nearly equal.
/// A few real DnB tracks at 176–178 BPM (jungle / hardcore
/// territory) will pick up a mild penalty of weight ~0.97 — small
/// enough that the lower octave at 88 BPM (weight 0.90) still
/// loses on weighted score whenever the raw ratio sits in its
/// observed [0.83, 0.94] band.
///
/// **Known limitation: mixed-tempo DnB with reggae breaks.**
/// Benny Page "Crying Out" accumulates real energy at both ~175 BPM
/// and ~87 BPM; no single prior resolves both. OG Reppa / We Multiply
/// (DnB ragga at ~135) may still land at ~90 when the 174 peak is
/// weak. Tap-to-grid (M11c.3b) remains the override.
fn tempo_prior_weight(bpm: f64) -> f64 {
    if !bpm.is_finite() || bpm <= 60.0 {
        return 0.20;
    }
    if bpm < 95.0 {
        return 0.20 + (bpm - 60.0) / 35.0 * 0.80;
    }
    if bpm <= 175.0 {
        return 1.00;
    }
    if bpm < 200.0 {
        return 1.00 - (bpm - 175.0) / 25.0 * 0.70;
    }
    0.20
}

/// Perceptual prior adjusted for [`OctaveProfile`]. The dub profile
/// lifts the 65–80 BPM band to full weight so a rejected ~140 BPM
/// peak does not lose to a weaker ~93 BPM harmonic (Blind Prophet).
/// Roots reggae keeps the default prior: lifting the whole 60–100
/// band would flip true ~95 BPM one-drop tracks to ~72 half-time.
fn tempo_prior_weight_with_profile(bpm: f64, profile: OctaveProfile) -> f64 {
    match profile {
        OctaveProfile::Dub if (65.0..=80.0).contains(&bpm) => 1.0,
        _ => tempo_prior_weight(bpm),
    }
}

/// Returns `true` when a qualifying ~117 BPM candidate should be
/// **discarded** because a credible 4/4 DnB sibling exists at the
/// 3:2 ratio (M11c.3b).
///
/// Dub / reggae-influenced DnB often produces a strong ODF peak at
/// `high_bpm × 2/3` (dotted-quarter / skank subdivision) that beats
/// the true quarter-note pulse at `high_bpm`. The M11c.3 perceptual
/// prior does not help because 117 BPM sits at full plateau weight.
///
/// **Product rule (PRD / user corpus): working DJs mix in 4/4 only.**
/// A detection in the 115–119 BPM band that is exactly a 3:2
/// subdivision of a qualifying 168–182 BPM peak is a *false tempo*
/// for our audience, not a genre we need to preserve. Genuinely
/// triplet-feel material is out of scope: if someone plays it,
/// a wrong BPM is expected and tap-to-grid (M11c.3b UI) is the
/// override. We therefore **hard-reject** (exclude from pass 2)
/// rather than soft-penalise triplet candidates.
///
/// The raw-ratio gate ([`TRIPLET_SIBLING_MIN_RAW_RATIO`]) still
/// prevents false flips on genuine 120 BPM house where a 180 BPM
/// hi-hat harmonic is present but much weaker than the true beat.
fn triplet_subdivision_rejected(
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !(TRIPLET_LOW_BPM_MIN..=TRIPLET_LOW_BPM_MAX).contains(&candidate_bpm) {
        return false;
    }
    for &(other_bpm, other_raw) in qualified {
        if !(TRIPLET_HIGH_BPM_MIN..=TRIPLET_HIGH_BPM_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = other_bpm / candidate_bpm;
        if (ratio - TRIPLET_RATIO_TARGET).abs() > TRIPLET_RATIO_TOLERANCE * TRIPLET_RATIO_TARGET {
            continue;
        }
        if other_raw >= candidate_raw * TRIPLET_SIBLING_MIN_RAW_RATIO {
            return true;
        }
    }
    false
}

/// Lower band for the 3:2 triplet-subdivision rejection pass.
/// Kept narrow (115–119) so genuine 120 BPM hip-hop / house
/// candidates do not trigger the linked half-time penalty.
const TRIPLET_LOW_BPM_MIN: f64 = 115.0;
const TRIPLET_LOW_BPM_MAX: f64 = 119.0;

/// Upper (DnB-rate) band paired with the lower triplet band.
const TRIPLET_HIGH_BPM_MIN: f64 = 168.0;
const TRIPLET_HIGH_BPM_MAX: f64 = 182.0;

/// Target BPM ratio (3:2) between the DnB sibling and the skank
/// subdivision candidate.
const TRIPLET_RATIO_TARGET: f64 = 1.5;

/// Relative tolerance on the 3:2 ratio (±4 %).
const TRIPLET_RATIO_TOLERANCE: f64 = 0.04;

/// The DnB sibling must carry at least this fraction of the lower
/// candidate's raw score before we reject the triplet peak.
/// Calibrated against Gold Dust (weakest M11c.3 triplet case:
/// raw(178) / raw(117) ≈ 0.93).
const TRIPLET_SIBLING_MIN_RAW_RATIO: f64 = 0.85;

/// Score multiplier applied to the ~87–89 BPM half-time peak that
/// often wins after the triplet candidate is hard-rejected (see
/// `linked_halftime_penalty`).
const LINKED_HALFTIME_REJECTION_FACTOR: f64 = 0.75;

/// Lower band for the reggae skank / one-drop double-time rejection
/// pass (M11c.3c). Kept at 80 so the ~87 BPM DnB half-time peak
/// does not pair against skank candidates in the 118–160 band.
const SKANK_LOW_BPM_MIN: f64 = 60.0;
const SKANK_LOW_BPM_MAX: f64 = 80.0;

/// Upper (skank-rate) band paired with the one-drop lower band.
/// Capped at 160 so DnB at 170+ and the triplet pass at 117/178
/// stay untouched.
const SKANK_HIGH_BPM_MIN: f64 = 118.0;
const SKANK_HIGH_BPM_MAX: f64 = 160.0;

/// Target BPM ratio (2:1) between the skank subdivision and the
/// one-drop root tempo.
const SKANK_RATIO_TARGET: f64 = 2.0;

/// Relative tolerance on the 2:1 ratio (±4 %).
const SKANK_RATIO_TOLERANCE: f64 = 0.04;

/// The one-drop sibling must carry at least this fraction of the
/// upper candidate's raw score before we reject the skank peak.
/// Calibrated against Natural Mystic (raw(65) / raw(129) ≈ 1.07)
/// and Smile Jamaica (raw(74) / raw(148) ≈ 1.00).
const SKANK_SIBLING_MIN_RAW_RATIO: f64 = 0.85;

/// When the lower octave does not quite beat the skank peak on raw
/// score but the two peaks differ by at least this fraction, treat
/// the skank rate as a false tempo (Murderer: raw gap ≈ 6 %). The
/// sibling band is kept tight (71–75) so 140 BPM click trains whose
/// half-time harmonic lands near 70 BPM are not flipped.
const SKANK_COMPETING_PEAK_MIN_GAP: f64 = 0.05;
const SKANK_GAP_SIBLING_MIN: f64 = 71.0;
const SKANK_GAP_SIBLING_MAX: f64 = 75.0;

/// Dancehall / ragga tracks whose true tempo sits at ~90 BPM often
/// carry a phantom peak at ~180 BPM with nearly equal raw score
/// (Beenie Man "Who Am I"). Reject only when the siblings are
/// within [`DANCEHALL_DOUBLE_RAW_TOLERANCE`] of each other so DnB
/// at 178 (where the 89 peak is *stronger* on raw) still resolves
/// to 172 via the existing prior.
const DANCEHALL_HIGH_BPM_MIN: f64 = 175.0;
const DANCEHALL_HIGH_BPM_MAX: f64 = 185.0;
const DANCEHALL_LOW_BPM_MIN: f64 = 85.0;
const DANCEHALL_LOW_BPM_MAX: f64 = 95.0;
const DANCEHALL_DOUBLE_RAW_TOLERANCE: f64 = 0.05;

/// When a ~117 BPM triplet candidate is hard-rejected because a
/// credible 3:2 sibling exists in the DnB band, the same track
/// often also carries a strong half-time peak at `high_bpm / 2`
/// (~87 BPM). Excluding only the triplet leaves that half-time
/// peak to win (Gold Dust: 117 rejected → 89 beat 178). This pass
/// applies [`LINKED_HALFTIME_REJECTION_FACTOR`] to the half-time
/// octave of the DnB sibling whenever a triplet candidate in the
/// same cluster was rejected **and** the DnB sibling is weaker on
/// raw score than the half-time peak (the DnB failure-mode shape).
fn linked_halftime_penalty(
    candidate_bpm: f64,
    candidate_raw: f64,
    triplet_penalized_lows: &[f64],
    qualified: &[(f64, f64)],
) -> f64 {
    if !(85.0..=92.0).contains(&candidate_bpm) {
        return 1.0;
    }
    for &triplet_bpm in triplet_penalized_lows {
        for &(high_bpm, high_raw) in qualified {
            if !(TRIPLET_HIGH_BPM_MIN..=TRIPLET_HIGH_BPM_MAX).contains(&high_bpm) {
                continue;
            }
            // DnB half-time peaks beat the DnB sibling on raw score;
            // dancehall at ~90 BPM does the opposite (178 wins on raw).
            // Penalise only the DnB-shaped case so "Who Am I" keeps 89.
            if high_raw > candidate_raw {
                continue;
            }
            if hiphop_doubletime_rejected(high_bpm, high_raw, qualified) {
                continue;
            }
            let triplet_ratio = high_bpm / triplet_bpm;
            if (triplet_ratio - TRIPLET_RATIO_TARGET).abs()
                > TRIPLET_RATIO_TOLERANCE * TRIPLET_RATIO_TARGET
            {
                continue;
            }
            let octave_ratio = high_bpm / candidate_bpm;
            if (octave_ratio - 2.0).abs() <= 0.04 * 2.0 {
                return LINKED_HALFTIME_REJECTION_FACTOR;
            }
        }
    }
    1.0
}

/// Returns `true` when a qualifying skank-rate candidate in
/// [118, 160] BPM should be **discarded** because a credible
/// one-drop sibling exists at the 2:1 ratio in [60, 80] BPM
/// (M11c.3c).
///
/// Roots reggae autocorrelation locks onto the hi-hat skank at
/// ~130 BPM instead of the kick one-drop at ~65 BPM because both
/// peaks sit at full plateau prior weight while 65 BPM is crushed
/// by the lower ramp. When the lower octave carries comparable
/// raw mass, the skank peak is a false tempo for our audience.
fn skank_doubletime_rejected(
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !(SKANK_HIGH_BPM_MIN..=SKANK_HIGH_BPM_MAX).contains(&candidate_bpm) {
        return false;
    }
    for &(other_bpm, other_raw) in qualified {
        if !(SKANK_LOW_BPM_MIN..=SKANK_LOW_BPM_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = candidate_bpm / other_bpm;
        if (ratio - SKANK_RATIO_TARGET).abs() > SKANK_RATIO_TOLERANCE * SKANK_RATIO_TARGET {
            continue;
        }
        if other_raw >= candidate_raw * SKANK_SIBLING_MIN_RAW_RATIO
            && other_raw > candidate_raw * 1.001
        {
            return true;
        }
        let raw_gap = (candidate_raw - other_raw).abs() / candidate_raw.max(other_raw);
        if other_raw >= candidate_raw * SKANK_SIBLING_MIN_RAW_RATIO
            && raw_gap >= SKANK_COMPETING_PEAK_MIN_GAP
            && (SKANK_GAP_SIBLING_MIN..=SKANK_GAP_SIBLING_MAX).contains(&other_bpm)
        {
            return true;
        }
    }
    false
}

/// Returns `true` when a ~180 BPM candidate should be discarded
/// because an ~90 BPM sibling exists at the 2:1 ratio with nearly
/// equal raw score (dancehall double-time, M11c.3c).
fn dancehall_doubletime_rejected(
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !(DANCEHALL_HIGH_BPM_MIN..=DANCEHALL_HIGH_BPM_MAX).contains(&candidate_bpm) {
        return false;
    }
    for &(other_bpm, other_raw) in qualified {
        if !(DANCEHALL_LOW_BPM_MIN..=DANCEHALL_LOW_BPM_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = candidate_bpm / other_bpm;
        if (ratio - SKANK_RATIO_TARGET).abs() > SKANK_RATIO_TOLERANCE * SKANK_RATIO_TARGET {
            continue;
        }
        // DnB half-time peaks beat the 178 phantom on raw score; only
        // reject when the upper octave is genuinely stronger (dancehall).
        if candidate_raw <= other_raw * 1.01 {
            continue;
        }
        let raw_gap = (candidate_raw - other_raw).abs() / candidate_raw.max(other_raw);
        if raw_gap <= DANCEHALL_DOUBLE_RAW_TOLERANCE {
            return true;
        }
    }
    false
}

/// Hip-hop / soul tracks whose true tempo sits at ~85–92 BPM often
/// carry a phantom peak at the 2× hi-hat rate (160–185 BPM) with
/// nearly equal raw score (M11c.3e rap corpus). Reject when a
/// credible 2:1 sibling in the mixable rap band exists within
/// [`HIP_HOP_NEAR_TIE_MAX_GAP`]. DnB at 172+ is spared because its
/// half-time sibling wins on raw with a much larger gap (~13 %).
const HIP_HOP_HIGH_BPM_MIN: f64 = 160.0;
const HIP_HOP_HIGH_BPM_MAX: f64 = 185.0;
const HIP_HOP_LOW_BPM_MIN: f64 = 80.0;
const HIP_HOP_LOW_BPM_MAX: f64 = 95.0;
const HIP_HOP_OCTAVE_RATIO: f64 = 2.0;
const HIP_HOP_OCTAVE_TOLERANCE: f64 = 0.04;
/// Sibling must reach this fraction of the upper peak's raw score
/// (Cappadonna: 86 / 172 ≈ 0.96).
const HIP_HOP_SIBLING_MIN_RAW_RATIO: f64 = 0.96;
/// Near-tie gate: real rap 2× errors show gaps ≥ 0.5 % (Charles
/// Bradley ≈ 0.6 %); the rolling-DnB synthetic fixture ties at
/// ≈ 0.3 % and is spared by the lower bound.
const HIP_HOP_NEAR_TIE_MIN_GAP: f64 = 0.005;
const HIP_HOP_NEAR_TIE_MAX_GAP: f64 = 0.06;

/// When several peaks cluster in the DnB core band, the near-tie at
/// 2× is structural (rolling kick grid), not a rap hi-hat phantom.
const DNB_CORE_BPM_MIN: f64 = 168.0;
const DNB_CORE_BPM_MAX: f64 = 182.0;
const DNB_CORE_CLUSTER_MIN_PEERS: f64 = 0.85;

fn dnb_core_cluster_sibling(
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !(DNB_CORE_BPM_MIN..=DNB_CORE_BPM_MAX).contains(&candidate_bpm) {
        return false;
    }
    let peer_count = qualified
        .iter()
        .filter(|&&(other_bpm, other_raw)| {
            other_bpm != candidate_bpm
                && (DNB_CORE_BPM_MIN..=DNB_CORE_BPM_MAX).contains(&other_bpm)
                && other_raw >= candidate_raw * DNB_CORE_CLUSTER_MIN_PEERS
        })
        .count();
    peer_count >= 2
}

fn hiphop_doubletime_rejected(
    candidate_bpm: f64,
    candidate_raw: f64,
    qualified: &[(f64, f64)],
) -> bool {
    if !(HIP_HOP_HIGH_BPM_MIN..=HIP_HOP_HIGH_BPM_MAX).contains(&candidate_bpm) {
        return false;
    }
    if dnb_core_cluster_sibling(candidate_bpm, candidate_raw, qualified) {
        return false;
    }
    for &(other_bpm, other_raw) in qualified {
        if !(HIP_HOP_LOW_BPM_MIN..=HIP_HOP_LOW_BPM_MAX).contains(&other_bpm) {
            continue;
        }
        let ratio = candidate_bpm / other_bpm;
        if (ratio - HIP_HOP_OCTAVE_RATIO).abs() > HIP_HOP_OCTAVE_TOLERANCE * HIP_HOP_OCTAVE_RATIO {
            continue;
        }
        if other_raw < candidate_raw * HIP_HOP_SIBLING_MIN_RAW_RATIO {
            continue;
        }
        let raw_gap = (candidate_raw - other_raw).abs() / candidate_raw.max(other_raw);
        if (HIP_HOP_NEAR_TIE_MIN_GAP..=HIP_HOP_NEAR_TIE_MAX_GAP).contains(&raw_gap) {
            return true;
        }
    }
    false
}

/// Estimate tempo from an ODF, restricting the search to `range`.
///
/// `odf_sample_rate` is the ODF's own sample rate in Hz — typically
/// `audio_sr / HOP_SIZE`. The function does no audio-rate scaling
/// itself; it operates purely in ODF time.
///
/// Returns `None` when the ODF is too short, contains no energy, or has
/// no peak above [`DETECTION_THRESHOLD`].
pub(crate) fn estimate_tempo(
    odf: &[f32],
    odf_sample_rate: f64,
    range: BpmRange,
    profile: OctaveProfile,
) -> Option<BpmEstimate> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lag_min = ((60.0 * odf_sample_rate) / range.max).floor() as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lag_max = ((60.0 * odf_sample_rate) / range.min).ceil() as usize;
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

    let acf_zero = acf_raw[0];
    if acf_zero < 1e-12 {
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
    const WINDOW: usize = 2;

    let local = |lag: usize| -> f64 {
        if lag > max_lag {
            return 0.0;
        }
        let lo = lag.saturating_sub(WINDOW);
        let hi = (lag + WINDOW).min(max_lag);
        let mut sum = 0.0f64;
        for &v in &acf_raw[lo..=hi] {
            sum += v;
        }
        sum
    };

    // Pass 1: harmonic-mean local-energy score for every candidate
    // lag. We hold every score (not just the running max) because
    // pass 2 (the M11c.3 perceptual-prior octave-disambiguation
    // step) needs to compare the chosen peak against any
    // octave-related sibling that *also* clears a high-confidence
    // gate. Storing the full array costs `O(lag_max - lag_min)`
    // f64s — at the default 60–200 BPM range and our typical ODF
    // rate (~86 Hz), that's ~60 entries, sub-microsecond memory.
    let n_candidates = lag_max - lag_min + 1;
    let mut raw_scores: Vec<f64> = Vec::with_capacity(n_candidates);
    let mut max_raw_score = f64::NEG_INFINITY;
    let mut best_lo_raw: Option<usize> = None;

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
            score_sum += local(probe);
            k_count += 1;
        }
        let raw_score = if k_count == 0 {
            f64::NEG_INFINITY
        } else if k_count < MIN_HARMONICS_FOR_MEAN {
            local(lo)
        } else {
            #[allow(clippy::cast_precision_loss)]
            let count_f = k_count as f64;
            score_sum / count_f
        };
        let raw_score = match &tempogram {
            Some(w) if raw_score.is_finite() => raw_score * w[lo - lag_min],
            _ => raw_score,
        };
        raw_scores.push(raw_score);

        // Track the raw maximum separately from the prior-weighted
        // selection so the "peak qualifying" gate in pass 2 can
        // reject off-peak lags whose harmonic mean happened to land
        // a couple of partial-peak windows.
        if raw_score.is_finite() {
            let better = if !max_raw_score.is_finite() {
                true
            } else {
                let tie_window = max_raw_score.abs() * SCORE_TIE_REL_TOL;
                if raw_score > max_raw_score + tie_window {
                    true
                } else if (raw_score - max_raw_score).abs() <= tie_window {
                    best_lo_raw.is_none_or(|prev| lo < prev)
                } else {
                    false
                }
            };
            if better {
                max_raw_score = raw_score;
                best_lo_raw = Some(lo);
            }
        }
    }

    let best_lo_raw = best_lo_raw?;

    const OCTAVE_CANDIDATE_THRESHOLD: f64 = 0.70;
    let qualify_threshold = max_raw_score * OCTAVE_CANDIDATE_THRESHOLD;

    if std::env::var("DUB_BPM_DEBUG").is_ok() {
        eprintln!(
            "DEBUG: lag_min={lag_min} lag_max={lag_max} odf_sr={odf_sample_rate:.3} \
             best_lo_raw={best_lo_raw} max_raw={max_raw_score:.6} qualify={qualify_threshold:.6}"
        );
        for (i, &s) in raw_scores.iter().enumerate() {
            let lo = lag_min + i;
            #[allow(clippy::cast_precision_loss)]
            let bpm = 60.0 * odf_sample_rate / lo as f64;
            let ratio = s / max_raw_score;
            if ratio > 0.45 {
                eprintln!("  PASS1 lag={lo:3} bpm={bpm:6.2} raw={s:.6} ratio={ratio:.3}");
            }
        }
    }

    // Pass 2: M11c.3 octave-prior selection.
    //
    // Among candidates whose raw harmonic-mean score clears
    // `OCTAVE_CANDIDATE_THRESHOLD × max_raw_score`, pick the one
    // with the highest *prior-weighted* score. The threshold gates
    // out off-peak lags whose harmonic-mean partially benefits from
    // a single harmonic landing in a peak window (the spike-train
    // synthetic test exposes this at `lo = 75` with raw_score ≈ 0.5
    // × max). Any genuine octave-octave-conflict case has raw_score
    // ratios well above 0.7 — see the perceptual-prior doc-comment.
    //
    // The first pass's "prefer smaller lag on tie" tiebreak is
    // preserved verbatim here, so the synthetic 120-BPM-pulse-train
    // tests that depend on the smaller-lag-wins behaviour still
    // resolve identically inside their narrow BPM ranges.

    // Collect every qualifying candidate before scoring so the
    // triplet-subdivision pass can inspect sibling pairs.
    let mut qualified: Vec<(usize, f64, f64)> = Vec::new();
    for (idx, &raw_score) in raw_scores.iter().enumerate() {
        if !raw_score.is_finite() || raw_score < qualify_threshold {
            continue;
        }
        let lo = lag_min + idx;
        #[allow(clippy::cast_precision_loss)]
        let candidate_bpm = 60.0 * odf_sample_rate / lo as f64;
        qualified.push((idx, raw_score, candidate_bpm));
    }

    let qualified_pairs: Vec<(f64, f64)> =
        qualified.iter().map(|&(_, raw, bpm)| (bpm, raw)).collect();

    let triplet_rejected_lows: Vec<f64> = qualified
        .iter()
        .filter(|&&(_, raw, bpm)| triplet_subdivision_rejected(bpm, raw, &qualified_pairs))
        .map(|&(_, _, bpm)| bpm)
        .collect();

    let mut best_lo: Option<usize> = None;
    let mut best_score = f64::NEG_INFINITY;

    for &(idx, raw_score, candidate_bpm) in &qualified {
        let lo = lag_min + idx;
        if triplet_subdivision_rejected(candidate_bpm, raw_score, &qualified_pairs) {
            if std::env::var("DUB_BPM_DEBUG").is_ok() {
                eprintln!(
                    "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} REJECTED (triplet subdivision)"
                );
            }
            continue;
        }
        if !profile_skips_skank_pass(profile)
            && skank_doubletime_rejected(candidate_bpm, raw_score, &qualified_pairs)
        {
            if std::env::var("DUB_BPM_DEBUG").is_ok() {
                eprintln!(
                    "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} REJECTED (skank double-time)"
                );
            }
            continue;
        }
        if dancehall_doubletime_rejected(candidate_bpm, raw_score, &qualified_pairs) {
            if std::env::var("DUB_BPM_DEBUG").is_ok() {
                eprintln!(
                    "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} REJECTED (dancehall double-time)"
                );
            }
            continue;
        }
        if !profile_skips_hiphop_doubletime_pass(profile)
            && hiphop_doubletime_rejected(candidate_bpm, raw_score, &qualified_pairs)
        {
            if std::env::var("DUB_BPM_DEBUG").is_ok() {
                eprintln!(
                    "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} REJECTED (hip-hop double-time)"
                );
            }
            continue;
        }
        if profile_doubletime_rejected(profile, candidate_bpm, raw_score, &qualified_pairs) {
            if std::env::var("DUB_BPM_DEBUG").is_ok() {
                eprintln!(
                    "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} REJECTED (genre profile double-time)"
                );
            }
            continue;
        }
        if profile_halftime_rejected(profile, candidate_bpm, raw_score, &qualified_pairs) {
            if std::env::var("DUB_BPM_DEBUG").is_ok() {
                eprintln!(
                    "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} REJECTED (genre profile half-time)"
                );
            }
            continue;
        }
        if profile_subdivision_rejected(profile, candidate_bpm, raw_score, &qualified_pairs) {
            if std::env::var("DUB_BPM_DEBUG").is_ok() {
                eprintln!(
                    "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} REJECTED (genre profile subdivision)"
                );
            }
            continue;
        }
        let halftime_penalty = linked_halftime_penalty(
            candidate_bpm,
            raw_score,
            &triplet_rejected_lows,
            &qualified_pairs,
        );
        let score =
            raw_score * tempo_prior_weight_with_profile(candidate_bpm, profile) * halftime_penalty;

        if std::env::var("DUB_BPM_DEBUG").is_ok() {
            eprintln!(
                "  PASS2 lag={lo:3} bpm={candidate_bpm:6.2} raw={raw_score:.6} prior={:.3} \
                 halftime={halftime_penalty:.3} weighted={score:.6}",
                tempo_prior_weight_with_profile(candidate_bpm, profile)
            );
        }

        let better = if !best_score.is_finite() {
            true
        } else {
            let tie_window = best_score.abs() * SCORE_TIE_REL_TOL;
            if score > best_score + tie_window {
                true
            } else if (score - best_score).abs() <= tie_window {
                best_lo.is_none_or(|prev| lo < prev)
            } else {
                false
            }
        };
        if better {
            best_score = score;
            best_lo = Some(lo);
        }
    }

    // Defensive fallback: if pass 2 picked nothing (shouldn't be
    // reachable — the raw winner always qualifies against itself),
    // honour the raw winner so we never accidentally degrade to
    // None on a healthy input.
    let best_lo = best_lo.unwrap_or(best_lo_raw);

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

    /// Helper for the `tempo_prior_weight_*` tests below: assert
    /// the prior at `bpm` is within `±tol` of `expected`. Uses the
    /// exact piecewise formula so calibration drift is caught
    /// immediately, but with a small tolerance to permit minor
    /// breakpoint re-shaping (e.g. moving 90 → 89.5) without
    /// having to rewrite every test value.
    fn assert_prior(bpm: f64, expected: f64, tol: f64) {
        let actual = tempo_prior_weight(bpm);
        assert!(
            (actual - expected).abs() <= tol,
            "tempo_prior_weight({bpm}) = {actual:.4}, expected {expected:.4} ± {tol}"
        );
    }

    /// The full-weight plateau [95, 175] must cover hip-hop / rap
    /// (95–105), house / techno (115–135), and drum-and-bass
    /// (165–175). If any of these slip below 1.0 the picker stops
    /// flipping octaves correctly on real-music corpus tracks.
    #[test]
    fn tempo_prior_plateau_covers_mixable_genres() {
        for &bpm in &[95.0, 100.0, 120.0, 130.0, 165.0, 170.0, 174.0, 175.0] {
            assert_prior(bpm, 1.00, 1e-9);
        }
    }

    /// The lower ramp (60 → 95) penalises slow tempi enough to
    /// flip every clean real-DnB half-time peak in the M11c.3
    /// corpus. The worst-case clean DnB track (Total Science /
    /// S.P.Y. "Gangsta" at lag 59 / 87.59 BPM) has a raw-score
    /// ratio of 0.870 against its 172 BPM upper octave, so
    /// prior(87.59) must stay strictly below 0.87 to make 172
    /// win on weighted score.
    #[test]
    fn tempo_prior_lower_ramp_flips_dnb_halftime() {
        let p86 = tempo_prior_weight(86.0);
        assert!(
            p86 < 0.85,
            "prior(86) = {p86:.4}; must stay < 0.85 so DnB at 172 \
             wins the octave duel against the 86 K-K skip-1 peak"
        );
        let p87_59 = tempo_prior_weight(87.59);
        assert!(
            p87_59 < 0.87,
            "prior(87.59) = {p87_59:.4}; the worst-case clean DnB \
             lower-octave peak in the M11c.3 corpus has raw ratio \
             0.870 against its 172 BPM sibling — a softer prior \
             re-introduces the half-time bug on this track"
        );
        // And it must not drop *too* far either — a real 86 BPM
        // hip-hop track with no 172-rate counterpart should still
        // get detected at 86, not get suppressed to silence.
        assert!(
            p86 > 0.65,
            "prior(86) = {p86:.4}; dropping below 0.65 starts \
             losing legitimate 85–88 BPM hip-hop / boom-bap"
        );
    }

    /// The upper ramp (178 → 200) drops fast enough to flip a
    /// rap-at-2x peak (95 BPM → detected as 190) back to the
    /// perceived tempo. 190 BPM must end up well below 1.0 so the
    /// 95 BPM candidate wins on weighted score even when the
    /// raw-score gap favours 190 by a few percent.
    #[test]
    fn tempo_prior_upper_ramp_flips_rap_doubletime() {
        let p190 = tempo_prior_weight(190.0);
        assert!(
            p190 < 0.65,
            "prior(190) = {p190:.4}; must stay < 0.65 so rap at \
             95 wins the octave duel against the 190 hi-hat peak"
        );
        let p180 = tempo_prior_weight(180.0);
        assert!(
            p180 < 0.95,
            "prior(180) = {p180:.4}; the synthetic hip-hop-at-90 \
             fixture needs prior(90) > prior(180); raising 180 \
             into the plateau re-introduces the 2× detection bug"
        );
    }

    #[test]
    fn tempo_prior_floor_at_extremes() {
        assert_prior(40.0, 0.20, 1e-9);
        assert_prior(60.0, 0.20, 1e-9);
        assert_prior(220.0, 0.20, 1e-9);
        // Non-finite inputs must not panic and must return the floor
        // weight — see the `!bpm.is_finite()` guard at the top of
        // `tempo_prior_weight`.
        assert!((tempo_prior_weight(f64::NAN) - 0.20).abs() < 1e-9);
        assert!((tempo_prior_weight(f64::INFINITY) - 0.20).abs() < 1e-9);
        assert!((tempo_prior_weight(f64::NEG_INFINITY) - 0.20).abs() < 1e-9);
    }

    #[test]
    fn triplet_subdivision_hard_rejects_when_dnb_sibling_is_credible() {
        // Shape taken from Gold Dust (Bou remix): 117 wins on raw
        // score but 178 clears the sibling gate at raw ratio 0.93.
        let qualified = [(117.45, 2.784), (178.21, 2.590), (172.27, 1.929)];
        assert!(triplet_subdivision_rejected(117.45, 2.784, &qualified));
        assert!(!triplet_subdivision_rejected(178.21, 2.590, &qualified));
    }

    #[test]
    fn triplet_subdivision_spares_120_when_180_harmonic_is_weak() {
        // Genuine 120 BPM house: hi-hat harmonic at 180 exists but
        // is too weak to trigger the flip.
        let qualified = [(120.0, 10.0), (180.0, 5.0)];
        assert!(!triplet_subdivision_rejected(120.0, 10.0, &qualified));
    }

    #[test]
    fn linked_halftime_penalty_fires_after_triplet_cluster_rejection() {
        // Gold Dust shape: 117 triplet rejected, 89 is half of 178.
        let qualified = [(117.45, 2.784), (178.21, 2.590), (89.10, 2.805)];
        let rejected = [117.45];
        assert!(
            (linked_halftime_penalty(89.10, 2.805, &rejected, &qualified)
                - LINKED_HALFTIME_REJECTION_FACTOR)
                .abs()
                < 1e-9
        );
        assert!((linked_halftime_penalty(89.10, 2.805, &[], &qualified) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn linked_halftime_spares_dancehall_when_high_octave_wins_on_raw() {
        // Who Am I shape: 178 beats 89 on raw — no DnB-style penalty.
        let qualified = [(117.45, 2.733), (178.21, 3.631), (89.10, 3.572)];
        let rejected = [117.45];
        assert!((linked_halftime_penalty(89.10, 3.572, &rejected, &qualified) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn skank_doubletime_hard_rejects_when_one_drop_sibling_is_credible() {
        // Natural Mystic shape: 129 skank vs 65 one-drop (lower wins raw).
        let qualified = [(129.20, 4.217), (65.42, 4.498), (97.51, 2.886)];
        assert!(skank_doubletime_rejected(129.20, 4.217, &qualified));
        assert!(!skank_doubletime_rejected(65.42, 4.498, &qualified));
    }

    #[test]
    fn skank_doubletime_spares_clean_pulse_train_octave_ties() {
        // Synthetic 120 BPM pulse train: 60 BPM sibling ties on raw.
        let qualified = [(120.0, 5.0), (60.0, 5.0)];
        assert!(!skank_doubletime_rejected(120.0, 5.0, &qualified));
    }

    #[test]
    fn skank_doubletime_rejects_competing_peak_when_gap_is_large() {
        let murderer = [(143.55, 5.008), (71.78, 4.705)];
        assert!(skank_doubletime_rejected(143.55, 5.008, &murderer));
        let click = [(127.84, 91.0), (63.92, 85.0)];
        assert!(!skank_doubletime_rejected(127.84, 91.0, &click));
        let house = [(140.62, 104.4), (70.31, 104.2)];
        assert!(!skank_doubletime_rejected(140.62, 104.4, &house));
    }

    #[test]
    fn skank_doubletime_spares_dnb_at_170_plus() {
        let qualified = [(172.27, 4.454), (86.13, 5.121)];
        assert!(!skank_doubletime_rejected(172.27, 4.454, &qualified));
    }

    #[test]
    fn dancehall_doubletime_rejects_near_equal_180_sibling() {
        let qualified = [(178.21, 3.631), (89.10, 3.572)];
        assert!(dancehall_doubletime_rejected(178.21, 3.631, &qualified));
    }

    #[test]
    fn dancehall_doubletime_spares_dnb_when_low_octave_dominates_raw() {
        // Baddadan shape: 89 raw beats 178 — reject would break 172 pick.
        let qualified = [(178.21, 2.899), (172.27, 2.947), (87.59, 3.144)];
        assert!(!dancehall_doubletime_rejected(178.21, 2.899, &qualified));
    }

    #[test]
    fn hiphop_doubletime_rejects_rap_corpus_shapes() {
        let cappadonna = [(172.27, 2.451), (86.13, 2.365)];
        assert!(hiphop_doubletime_rejected(172.27, 2.451, &cappadonna));

        let charizma = [(178.21, 2.003), (89.10, 2.022)];
        assert!(hiphop_doubletime_rejected(178.21, 2.003, &charizma));

        let bradley = [(178.21, 2.398), (90.67, 2.383)];
        assert!(hiphop_doubletime_rejected(178.21, 2.398, &bradley));

        let daydream = [(166.71, 2.775), (83.35, 2.928)];
        assert!(hiphop_doubletime_rejected(166.71, 2.775, &daydream));
    }

    #[test]
    fn hiphop_doubletime_spares_dnb_when_half_time_gap_is_large() {
        let gangsta = [(172.27, 4.454), (86.13, 5.121)];
        assert!(!hiphop_doubletime_rejected(172.27, 4.454, &gangsta));
    }

    #[test]
    fn hiphop_doubletime_spares_dnb_core_cluster() {
        let rolling = [
            (175.78, 150.915),
            (170.45, 145.0),
            (178.21, 144.0),
            (86.54, 150.407),
        ];
        assert!(!hiphop_doubletime_rejected(175.78, 150.915, &rolling));
        let charizma = [(178.21, 2.003), (172.27, 1.496), (89.10, 2.022)];
        assert!(hiphop_doubletime_rejected(178.21, 2.003, &charizma));
    }

    #[test]
    fn hiphop_doubletime_spares_synthetic_dnb_perfect_tie() {
        let perfect = [(175.78, 150.915), (86.54, 150.407)];
        assert!(!hiphop_doubletime_rejected(175.78, 150.915, &perfect));
    }

    #[test]
    fn hiphop_doubletime_spares_chestra_at_112() {
        let qualified = [(112.35, 2.319), (95.70, 1.170), (56.18, 1.050)];
        assert!(!hiphop_doubletime_rejected(112.35, 2.319, &qualified));
    }

    #[test]
    fn empty_odf_returns_none() {
        assert!(estimate_tempo(&[], 100.0, BpmRange::DEFAULT, OctaveProfile::Default).is_none());
    }

    #[test]
    fn flat_odf_returns_none() {
        let odf = vec![0.0f32; 500];
        assert!(estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, OctaveProfile::Default).is_none());
    }

    #[test]
    fn perfectly_periodic_odf_recovers_period() {
        // Synthetic ODF: a pulse train every 50 samples at odf_sr = 100
        // → period 0.5 s → 120 BPM exactly.
        let mut odf = vec![0.0f32; 1000];
        for i in (0..odf.len()).step_by(50) {
            odf[i] = 1.0;
        }
        let est = estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, OctaveProfile::Default)
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
        let est = estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, OctaveProfile::Default)
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
        assert!(estimate_tempo(&odf, 100.0, BpmRange::DEFAULT, OctaveProfile::Default).is_none());
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
        let est =
            estimate_tempo(&odf, 100.0, narrow, OctaveProfile::Default).expect("should detect");
        assert!(
            est.bpm >= 60.0 && est.bpm <= 90.0,
            "narrow-range BPM should stay inside [60, 90]; got {}",
            est.bpm
        );
    }
}
