//! Offline beat-grid analysis: BPM + phase → **uniform 4/4 grid**.
//!
//! Traktor-style: detect transients, find the constant `(period, phase)`
//! that places the most transients on grid lines, emit a strictly uniform
//! grid `anchor + i × period`.

use crate::offline::analyze_bpm_with_range_profile_and_odfs;
use crate::{AnalysisError, BpmRange, OctaveProfile, HOP_SIZE, MAX_BPM, MIN_BPM};

const AUTO_REFINE_RANGE_PCT: f64 = 0.005;
const AUTO_REFINE_STEP_PCT: f64 = 0.0005;
const ZOOM_REFINE_RANGE_PCT: f64 = 0.001;
const ZOOM_REFINE_STEP_PCT: f64 = 0.00005;
const RELATCH_REFINE_RANGE_PCT: f64 = 0.010;
const RELATCH_REFINE_STEP_PCT: f64 = 0.0005;
const LSQ_SEARCH_FRACTION: f64 = 0.20;
const LSQ_MIN_PEAK_RATIO: f64 = 1.5;
const DOWNBEAT_CONFIDENCE_TIEBREAK: f64 = 1.2;
const KICK_ONLY_ENERGY_FRACTION: f64 = 0.25;
const KICK_ONLY_MIN_BEATS: usize = 8;
const INTRO_OUTRO_WINDOW_SECS: f64 = 30.0;

/// Residual statistics from the LSQ grid fit. Drives the M11d.7
/// drift-aware auto-lock decision in `dub-library`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridQuality {
    /// RMS residual in milliseconds.
    pub rms_ms: f32,
    /// 95th percentile of |residual| in milliseconds.
    pub p95_ms: f32,
    /// Worst single-beat residual in milliseconds.
    pub max_abs_ms: f32,
    /// Fraction of beats with a usable ODF peak in the search window.
    pub kept_fraction: f32,
    /// Signed linear drift of residuals vs time (ms per minute).
    pub drift_slope_ms_per_min: f32,
}

impl GridQuality {
    /// Whether the uniform grid fits tightly enough to auto-lock.
    #[must_use]
    pub fn auto_lock_safe(&self) -> bool {
        self.rms_ms < 8.0
            && self.p95_ms < 25.0
            && self.max_abs_ms < 50.0
            && self.kept_fraction > 0.75
            && self.drift_slope_ms_per_min.abs() < 3.0
    }

    /// "Trust me" quality used as the fallback when an external
    /// authority (e.g. user taps in `analyze_beat_grid_from_taps`)
    /// has supplied the grid and a residual measurement isn't
    /// possible. Equivalent to "no measurable residual" — passes
    /// `auto_lock_safe` so the M11d.7 lock heuristic doesn't flag
    /// a user-supplied grid with the drift warning by default.
    pub const PERFECT: Self = Self {
        rms_ms: 0.0,
        p95_ms: 0.0,
        max_abs_ms: 0.0,
        kept_fraction: 1.0,
        drift_slope_ms_per_min: 0.0,
    };
}

/// Per-track beat grid.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatGrid {
    /// Tempo in beats per minute.
    pub bpm: f64,
    /// Estimator confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Beat positions in seconds from sample 0.
    pub beats: Vec<f64>,
    /// Beats per bar (v0 always `4`).
    pub beats_per_bar: u8,
    /// PRD-BEATS C2 (round 4) — index into `beats` of the **first
    /// downbeat**. Phase is now a first-class scalar rather than
    /// being implied by "`beats[0]` is the downbeat":
    ///
    /// * `bar_phase == 0` means `beats[0]`, `beats[beats_per_bar]`,
    ///   `beats[2 * beats_per_bar]`, … are bar position 1 (yellow).
    /// * `bar_phase == 2` (in 4/4) means `beats[2]`, `beats[6]`, …
    ///   are bar position 1. `beats[0]` is then bar position 3.
    ///
    /// Invariant: `bar_phase < beats_per_bar`.
    ///
    /// All v1 analyzers stamp `bar_phase = 0`: the auto path still
    /// shifts the anchor so the first audible beat is the
    /// downbeat, and the tap path treats the first user tap as
    /// bar position 1. The field exists so `latch_beat_grid_at_
    /// downbeat` (set-the-1) can rotate phase without re-anchoring
    /// — the grid lines don't move, only which beat is yellow.
    pub bar_phase: u8,
    /// LSQ fit quality. `None` when analysis bailed before LSQ.
    pub quality: Option<GridQuality>,
    /// Kick-band bar-phase confidence (`best / second_best`).
    pub downbeat_confidence: f32,
}

impl BeatGrid {
    /// Empty grid (`confidence == 0`, no beats).
    #[must_use]
    pub const fn none() -> Self {
        Self {
            bpm: 0.0,
            confidence: 0.0,
            beats: Vec::new(),
            beats_per_bar: 4,
            bar_phase: 0,
            quality: None,
            downbeat_confidence: 0.0,
        }
    }
}

/// Index of the beat in `beats` closest to `target_secs`. Helper
/// shared by [`bar_phase_from_tap`] (the public set-the-1 entry
/// point) and the internal auto-grid bar-phase derivation in
/// [`analyze_uniform_grid_from_odf`] / [`latch_beat_grid_at_downbeat`]
/// / [`analyze_beat_grid_from_taps`]. Returns `0` for an empty
/// `beats` slice so callers can dispatch on it without a separate
/// guard.
#[must_use]
fn beat_index_nearest_to(beats: &[f64], target_secs: f64) -> usize {
    if beats.is_empty() {
        return 0;
    }
    let mut best_idx: usize = 0;
    let mut best_dist = f64::INFINITY;
    for (i, &b) in beats.iter().enumerate() {
        let d = (b - target_secs).abs();
        if d < best_dist {
            best_dist = d;
            best_idx = i;
        }
    }
    best_idx
}

/// Compute the [`BeatGrid::bar_phase`] that lands bar position 1
/// on the beat nearest `tap_time_secs`. Pure rotation — the
/// returned phase, when stored on the grid, leaves `bpm` and the
/// beat-position vector unchanged.
///
/// Used by the "set the 1" tap path (PRD-BEATS §4.1, round 4):
/// when the user taps the deck-header BPM with 1–2 taps the
/// algorithm-picked downbeat phase is replaced by their tap, but
/// the grid spacing and `beats[0]` stay where the auto analyzer
/// put them. The renderer redraws which markers are yellow on
/// the next frame.
///
/// Returns `0` for an empty grid or a non-finite tap (degenerate
/// inputs that the caller's UI already rejects, but cheap to
/// handle here so the function is total).
#[must_use]
pub fn bar_phase_from_tap(grid: &BeatGrid, tap_time_secs: f64) -> u8 {
    if grid.beats.is_empty() || grid.beats_per_bar == 0 || !tap_time_secs.is_finite() {
        return 0;
    }
    let best_idx = beat_index_nearest_to(&grid.beats, tap_time_secs);
    // Beat at `best_idx` becomes bar position 1; downbeats then
    // land on `best_idx`, `best_idx + beats_per_bar`, … which is
    // exactly `(idx mod beats_per_bar) == (best_idx mod
    // beats_per_bar)`.
    let bpb = u64::from(grid.beats_per_bar);
    u8::try_from((best_idx as u64) % bpb).unwrap_or(0)
}

/// Analyze a track and emit a uniform 4/4 beat grid (default octave profile).
pub fn analyze_beat_grid(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> Result<BeatGrid, AnalysisError> {
    analyze_beat_grid_with_profile(samples, sample_rate, channels, OctaveProfile::Default)
}

/// Like [`analyze_beat_grid`] with an explicit [`OctaveProfile`].
pub fn analyze_beat_grid_with_profile(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    profile: OctaveProfile,
) -> Result<BeatGrid, AnalysisError> {
    let (estimate, odf, kick_odf) = analyze_bpm_with_range_profile_and_odfs(
        samples,
        sample_rate,
        channels,
        BpmRange::DEFAULT,
        profile,
    )?;
    if estimate.confidence <= 0.0 {
        return Ok(BeatGrid::none());
    }

    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let duration_secs =
        samples.len() as f64 / (f64::from(sample_rate) * f64::from(channels.max(1)));

    let grid = analyze_uniform_grid_from_odf(
        &odf,
        &kick_odf,
        odf_sr,
        duration_secs,
        estimate.bpm,
        estimate.confidence,
        None,
    )?;
    // PRD-BEATS round 4 follow-up — visual grid alignment. The
    // ODF-snapped grid sits at perceptual-onset time (the spectral
    // flux peaks during the attack ramp). Shift it forward by the
    // median offset to the broadband amplitude peak so the
    // rendered line lands inside the visible kick, matching
    // Serato / Rekordbox / Traktor.
    Ok(shift_grid_to_amplitude_peak(
        grid,
        samples,
        sample_rate,
        channels,
    ))
}

/// Re-anchor at a user-supplied downbeat; refine period locally.
pub fn latch_beat_grid_at_downbeat(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    bpm: f64,
    downbeat_secs: f64,
    profile: OctaveProfile,
) -> Result<BeatGrid, AnalysisError> {
    if !(bpm.is_finite() && bpm > 0.0 && downbeat_secs.is_finite() && downbeat_secs >= 0.0) {
        return Ok(BeatGrid::none());
    }
    let (_, odf, kick_odf) = analyze_bpm_with_range_profile_and_odfs(
        samples,
        sample_rate,
        channels,
        BpmRange::DEFAULT,
        profile,
    )?;
    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let duration_secs =
        samples.len() as f64 / (f64::from(sample_rate) * f64::from(channels.max(1)));

    // M11d.7c: snap the user-supplied downbeat to the nearest
    // significant transient. The 1-tap downbeat path used to take
    // `downbeat_secs` raw, which baked the user's reaction-time
    // jitter (~25–40 ms) plus the audio output latency (~5–15 ms)
    // straight into the grid anchor — the "1 lands on a non-
    // transient" the user reported on tracks where auto put the
    // downbeat on bar-position 2 and they tapped once to relatch.
    // Window matches the 3+ tap path: ±min(period/4, 70 ms). When
    // the window is silent (intro pad, breakdown, false tap) the
    // snap returns `None` and we keep the raw tap so we never
    // drag the anchor onto sub-noise ODF wiggle.
    let period = 60.0 / bpm;
    let snap_half = (period * 0.25).min(SNAP_MAX_HALF_WINDOW_SECS);
    let kick_floor = odf_noise_floor(&kick_odf, SNAP_NOISE_FLOOR_FRAC);
    let broadband_floor = odf_noise_floor(&odf, SNAP_NOISE_FLOOR_FRAC);
    let snapped_downbeat = snap_to_nearest_transient(
        &kick_odf,
        &odf,
        odf_sr,
        downbeat_secs,
        snap_half,
        kick_floor,
        broadband_floor,
    )
    .unwrap_or(downbeat_secs);

    let refined_bpm = refine_period_at_anchor(
        &odf,
        odf_sr,
        bpm,
        snapped_downbeat,
        RELATCH_REFINE_RANGE_PCT,
        RELATCH_REFINE_STEP_PCT,
    )
    .unwrap_or(bpm);

    let (_, quality) = lsq_refit_grid(
        &odf,
        odf_sr,
        refined_bpm,
        snapped_downbeat,
        duration_secs,
        LSQ_SEARCH_FRACTION,
        true,
    )
    .unwrap_or((
        refined_bpm,
        GridQuality {
            rms_ms: 0.0,
            p95_ms: 0.0,
            max_abs_ms: 0.0,
            kept_fraction: 0.0,
            drift_slope_ms_per_min: 0.0,
        },
    ));

    // PRD-BEATS round 4 follow-up: emit beats spanning the FULL
    // track (no `retain` filter). The user-supplied downbeat is
    // bar 1; the renderer reads `bar_phase` to decide which beat
    // is yellow, so pre-roll beats render as regular ticks while
    // `snapped_downbeat` lands on the yellow marker. Replaces the
    // earlier "beats[0] == snapped_downbeat" convention which
    // dropped every pre-roll beat.
    let beats_per_bar: u8 = 4;
    let beats = uniform_beats(refined_bpm, snapped_downbeat, duration_secs);
    if beats.is_empty() {
        return Ok(BeatGrid::none());
    }
    let bar_phase = bar_phase_for_downbeat_time(&beats, snapped_downbeat, beats_per_bar);
    let grid = BeatGrid {
        bpm: refined_bpm,
        confidence: 1.0,
        beats,
        beats_per_bar,
        bar_phase,
        quality: Some(quality),
        downbeat_confidence: 1.0,
    };
    Ok(shift_grid_to_amplitude_peak(
        grid,
        samples,
        sample_rate,
        channels,
    ))
}

/// Build a grid from user tap times (M11d.7 round 3 tap-to-grid).
///
/// **The tap is a search hint, never the answer.** The user's
/// taps tell the algorithm where to look (a ±15 % BPM neighborhood
/// around the tap-interval median); the algorithm tells the user
/// the precise answer (the strongest real autocorrelation peak in
/// that neighborhood, snapped to integer if safe). A 2 s window
/// of 3–8 taps cannot beat a full-track spectral-flux estimator on
/// tempo precision — human reaction-time jitter (~25 ms 1σ) on
/// each tap edge contaminates the tap-interval median, which is why
/// PRD-BEATS §6.1 demands constrained re-analysis rather than
/// tap-derived BPM substitution.
///
/// Pipeline:
///
/// 1. **BPM seed** = weighted median of tap-to-tap intervals after
///    dropping intervals that imply BPMs outside `[MIN_BPM,
///    MAX_BPM]` (an accidental double-tap inside one beat must not
///    drag the median to 1200 BPM).
/// 2. **Constrained re-analysis.** Build a `BpmRange` at
///    `[seed × 0.85, seed × 1.15]` and run the full estimator
///    inside it. The estimator returns the strongest real
///    autocorrelation peak inside the window. ±15 % is comfortably
///    wider than human tap noise (~5 BPM 1σ at 100 BPM) and well
///    inside the half/double-time octaves (at ±50 % / +100 %), so
///    octave-error correction lands in the right octave by
///    construction. Replaces the old `reconcile_tap_bpm_with_hint`
///    branch logic entirely.
/// 3. **Integer-BPM snap if safe** (`snap_to_integer_bpm` —
///    ±0.10 tolerance). Eliminates the 0.02-BPM jitter that the
///    estimator otherwise leaves on integer dance tempos.
/// 4. **Anchor** = first tap snapped to the nearest significant
///    transient in the kick ODF (fallback broadband) inside a
///    window capped at `min(period/4, 70 ms)`. The
///    `odf_noise_floor` check keeps the snap from latching onto
///    sub-noise ODF wiggle when the first tap landed in dead
///    space — in that case we keep the raw tap (`unwrap_or`).
/// 5. Confidence is fixed at `1.0` (the user supplied ground
///    truth for tempo neighborhood and bar position);
///    `GridQuality` is measured against the final grid for the
///    drift indicator.
///
/// **No `bpm_hint` parameter.** Idempotence by construction: the
/// function takes only the new tap session's data, never prior
/// `user_tap` BPM or any "previous session" state. Each invocation
/// is independent of all prior tap sessions (PRD-BEATS §4.6).
pub fn analyze_beat_grid_from_taps(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    tap_times: &[f64],
    profile: OctaveProfile,
) -> Result<BeatGrid, AnalysisError> {
    if tap_times.len() < 3 {
        return Ok(BeatGrid::none());
    }
    let Some(seed_bpm) = weighted_median_bpm_from_taps(tap_times) else {
        return Ok(BeatGrid::none());
    };

    let lo = (seed_bpm * (1.0 - TAP_SEARCH_RADIUS_FRACTION)).max(MIN_BPM);
    let hi = (seed_bpm * (1.0 + TAP_SEARCH_RADIUS_FRACTION)).min(MAX_BPM);
    let range = BpmRange::new(lo, hi).unwrap_or(BpmRange::DEFAULT);
    let (estimate, odf, kick_odf) =
        analyze_bpm_with_range_profile_and_odfs(samples, sample_rate, channels, range, profile)?;
    if estimate.confidence <= 0.0 || estimate.bpm <= 0.0 {
        return Ok(BeatGrid::none());
    }
    let bpm_raw = estimate.bpm;

    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let duration_secs =
        samples.len() as f64 / (f64::from(sample_rate) * f64::from(channels.max(1)));
    if duration_secs <= 0.0 {
        return Ok(BeatGrid::none());
    }

    let period_raw = 60.0 / bpm_raw;
    let snap_half = (period_raw * 0.25).min(SNAP_MAX_HALF_WINDOW_SECS);
    let kick_floor = odf_noise_floor(&kick_odf, SNAP_NOISE_FLOOR_FRAC);
    let broadband_floor = odf_noise_floor(&odf, SNAP_NOISE_FLOOR_FRAC);
    let anchor_raw = snap_to_nearest_transient(
        &kick_odf,
        &odf,
        odf_sr,
        tap_times[0],
        snap_half,
        kick_floor,
        broadband_floor,
    )
    .unwrap_or(tap_times[0]);

    // Integer-BPM snap with anchor refit. Same contract as the
    // auto path (`snap_bpm_to_integer_if_safe`): only snap when
    // residuals don't get worse. This preserves the M11d.7a fix
    // for the 133.02 / 87.95 / 174.04 grid-drift class on tracks
    // where the constrained autocorrelation lands sub-BPM off.
    let quality_raw = measure_grid_quality(&odf, odf_sr, bpm_raw, anchor_raw, duration_secs)
        .unwrap_or(GridQuality::PERFECT);
    let (bpm, anchor, quality) = snap_bpm_to_integer_if_safe(
        &odf,
        odf_sr,
        bpm_raw,
        anchor_raw,
        duration_secs,
        quality_raw,
    );

    // PRD-BEATS round 4 follow-up: emit beats spanning the FULL
    // track. The user's first tap (snapped to the nearest
    // transient) IS bar 1; `bar_phase` carries that information
    // explicitly so pre-roll beats can render as regular ticks
    // without confusing the downbeat. The previous behaviour
    // dropped every beat before the anchor.
    let beats_per_bar: u8 = 4;
    let beats = uniform_beats(bpm, anchor, duration_secs);
    if beats.is_empty() {
        return Ok(BeatGrid::none());
    }
    let bar_phase = bar_phase_for_downbeat_time(&beats, anchor, beats_per_bar);

    let grid = BeatGrid {
        bpm,
        confidence: 1.0,
        beats,
        beats_per_bar,
        bar_phase,
        quality: Some(quality),
        downbeat_confidence: 1.0,
    };
    Ok(shift_grid_to_amplitude_peak(
        grid,
        samples,
        sample_rate,
        channels,
    ))
}

/// Hard cap on the snap radius so a tap can never travel further
/// than ~70 ms toward a louder transient. At 133 BPM (period ≈
/// 451 ms) the `period/4` cap (≈ 113 ms) would otherwise dominate;
/// 70 ms keeps us inside human tap reaction noise (~25–40 ms) with
/// margin while leaving headroom for the ODF onset bias (the
/// spectral-flux peak lands ~10–20 ms before the click body, see
/// the `click_track` golden tests for the existing 25 ms slack).
const SNAP_MAX_HALF_WINDOW_SECS: f64 = 0.070;

/// How close a tap-derived BPM must be to a whole number before we
/// quantise to that integer. Covers the "x.0x" / "x.9x" range the
/// user asked for. 0.10 is comfortably wider than typical human
/// tap-noise (a 5 ms median interval error at 133 BPM ≈ 1.5 BPM,
/// reduced by the weighted median over N intervals) and tight
/// enough to leave 0.5-increment tempos (87.5, 128.5) alone.
const INTEGER_BPM_SNAP_TOLERANCE: f64 = 0.10;

/// If `bpm` is within `tolerance` of the nearest integer value,
/// quantise to that integer; otherwise return `bpm` unchanged.
/// Used in the tap path to absorb the sub-BPM jitter that
/// otherwise accumulates into visible grid drift over the length
/// of a track.
#[must_use]
fn snap_to_integer_bpm(bpm: f64, tolerance: f64) -> f64 {
    if !bpm.is_finite() || bpm <= 0.0 {
        return bpm;
    }
    let nearest = bpm.round();
    if (bpm - nearest).abs() <= tolerance {
        nearest
    } else {
        bpm
    }
}

/// ±15 % search radius around the tap-interval median for
/// constrained re-analysis (PRD-BEATS §6.1). Wider than human tap
/// noise (~5 BPM 1σ at 100 BPM) so the true tempo lands inside
/// even after reaction-time scatter, and tight enough to stay
/// well inside the half/double-time octaves (50 % / 100 %) so the
/// estimator can never silently snap up or down an octave from
/// what the user intended. Replaces `TAP_BPM_HINT_TOLERANCE`
/// (which gated the now-removed `reconcile_tap_bpm_with_hint`).
const TAP_SEARCH_RADIUS_FRACTION: f64 = 0.15;

// `reconcile_tap_bpm_with_hint` removed in M11d.7 round 3. The
// constrained-re-analysis path inside `analyze_beat_grid_from_taps`
// runs the full estimator at `tap_median ± 15 %` and uses the
// returned `BpmEstimate.bpm` directly — there is no longer any
// "previous BPM as hint" branch. See PRD-BEATS §6.1.

/// Emit strictly uniform beat timestamps for `(bpm, anchor, duration)`.
#[must_use]
pub fn uniform_beats(bpm: f64, anchor_secs: f64, duration_secs: f64) -> Vec<f64> {
    if !bpm.is_finite() || bpm <= 0.0 || !duration_secs.is_finite() || duration_secs <= 0.0 {
        return Vec::new();
    }
    let period = 60.0 / bpm;
    if !period.is_finite() || period <= 0.0 {
        return Vec::new();
    }
    if !anchor_secs.is_finite() {
        return Vec::new();
    }
    let mut first = anchor_secs - (anchor_secs / period).floor() * period;
    if first < 0.0 {
        first += period;
    }
    if first >= period {
        first -= period;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_count = ((duration_secs / period) as usize).saturating_add(8);
    let mut beats = Vec::with_capacity(max_count);
    let mut t = first;
    while t <= duration_secs && beats.len() < max_count {
        beats.push(t);
        t += period;
    }
    beats
}

/// Median BPM from consecutive beat intervals.
#[must_use]
pub fn median_bpm_from_beats(beats: &[f64]) -> Option<f64> {
    if beats.len() < 2 {
        return None;
    }
    let mut intervals: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    intervals.retain(|dt| dt.is_finite() && *dt > 0.0);
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = intervals[intervals.len() / 2];
    Some(60.0 / median)
}

fn analyze_uniform_grid_from_odf(
    odf: &[f32],
    kick_odf: &[f32],
    odf_sr: f64,
    duration_secs: f64,
    bpm_init: f64,
    confidence: f32,
    fixed_anchor: Option<f64>,
) -> Result<BeatGrid, AnalysisError> {
    let Some((bpm_raw, anchor_secs, quality_raw)) =
        refine_full_pipeline(odf, odf_sr, bpm_init, duration_secs, fixed_anchor)
    else {
        return Ok(BeatGrid::none());
    };

    // M11d.7b round 2: snap LSQ-best BPM to the nearest integer
    // when within ±0.10. Real-world ODF peaks add 0.01–0.05 BPM
    // jitter to the LSQ output; the user's 133.02 detection on a
    // 133.0 track is the canonical case, and that 0.02 BPM error
    // accumulates to ~45 ms of visible grid drift over a 5-min
    // track ("the grid hits the transients at the start of the
    // track but is 1–2 mm off at the end"). The vast majority of
    // commercial music is produced at integer BPM, so unconditional
    // snap is almost always right. Safety net: re-measure quality
    // at the snapped tempo (refitting the anchor for the new BPM
    // — see `refit_anchor_at_bpm`) and fall back to the LSQ result
    // if the snap meaningfully worsens the fit. This protects the
    // rare track that genuinely sits at, say, 133.07 BPM (analogue
    // tape masters, live recordings).
    let (bpm, anchor_secs, quality) = snap_bpm_to_integer_if_safe(
        odf,
        odf_sr,
        bpm_raw,
        anchor_secs,
        duration_secs,
        quality_raw,
    );
    let period = 60.0 / bpm;
    let beats_per_bar: u8 = 4;

    // PRD-BEATS round 4 follow-up: the downbeat is chosen by a
    // two-tier rule:
    //
    // 1. If a `fixed_anchor` is supplied (legacy callers that
    //    pre-decide the downbeat), it IS bar 1.
    // 2. Otherwise: the **first audible kick** in the track is
    //    bar 1 by convention. This matches the user's mental
    //    model ("the first kick of the song is the 1") and
    //    almost all popular music (dance, rock, pop). The
    //    detector reads `kick_odf` (log-band 0, ~30–68 Hz, where
    //    a hi-hat or pad intro has no energy) and refuses to
    //    fire below an adaptive noise floor, so a non-kick
    //    intro falls through to the kick that actually starts
    //    the groove.
    // 3. Fallback when the kick band is silent for the entire
    //    track (rare; pure-melody piece): use
    //    `find_downbeat_offset` over the spectral-flux ODF and
    //    take its best-scored phase + the LSQ anchor.
    //
    // The previous behaviour ("walk anchor forward by full bars
    // until past `audible_start`, then drop pre-roll beats")
    // over-shot whenever bar 1's kick sat less than one bar after
    // the silence/audio boundary (the Oppidan case the user
    // reported: first marker on bar 2 because the loop jumped a
    // bar instead of landing on the first kick).
    let kick_floor = odf_noise_floor(kick_odf, SNAP_NOISE_FLOOR_FRAC);
    let chosen_downbeat;
    let downbeat_confidence;
    if let Some(fixed) = fixed_anchor {
        chosen_downbeat = fixed;
        downbeat_confidence = 1.0;
    } else if let Some(first_kick) = first_kick_peak_secs(kick_odf, odf_sr, kick_floor) {
        chosen_downbeat = first_kick;
        // Confidence comes from the find_downbeat_offset scoring
        // even when we override its chosen phase — it's the
        // measure of "how distinct is the strongest bar-phase
        // from the second strongest", which is meaningful for
        // the drift-aware lock heuristic regardless of who picks
        // the actual `i mod 4`.
        let (_offset, conf) = find_downbeat_offset(
            kick_odf,
            odf,
            odf_sr,
            period,
            anchor_secs,
            duration_secs,
            beats_per_bar,
        );
        downbeat_confidence = conf;
    } else {
        let (offset, conf) = find_downbeat_offset(
            kick_odf,
            odf,
            odf_sr,
            period,
            anchor_secs,
            duration_secs,
            beats_per_bar,
        );
        chosen_downbeat = anchor_secs + f64::from(offset) * period;
        downbeat_confidence = conf;
    }

    // Emit beats spanning the FULL track. PRD-BEATS round 4
    // follow-up: the grid extends backward through pre-roll
    // silence too — the C2 `bar_phase` field makes the downbeat
    // an explicit property, no longer tied to `beats[0]`, so
    // pre-roll beats render as regular ticks while the yellow
    // downbeat lands on the first audible kick. The previous
    // `retain(|t| t >= anchor)` filter dropped every pre-roll
    // beat, which the user reported as "no beatgrid before the
    // first kick of the second bar".
    let beats = uniform_beats(bpm, chosen_downbeat, duration_secs);
    if beats.is_empty() {
        return Ok(BeatGrid::none());
    }

    let bar_phase = bar_phase_for_downbeat_time(&beats, chosen_downbeat, beats_per_bar);

    Ok(BeatGrid {
        bpm,
        confidence,
        beats,
        beats_per_bar,
        bar_phase,
        quality: Some(quality),
        downbeat_confidence,
    })
}

/// Compute the bar-phase scalar that lands bar 1 on the beat
/// nearest `downbeat_secs`. Shared by every grid-construction
/// path (auto, latch, taps) so they agree on "the yellow marker
/// is the beat closest to the chosen downbeat, mod beats per
/// bar". Returns `0` for degenerate inputs (empty beats, zero
/// `beats_per_bar`, or non-finite `downbeat_secs`).
#[must_use]
fn bar_phase_for_downbeat_time(beats: &[f64], downbeat_secs: f64, beats_per_bar: u8) -> u8 {
    if beats.is_empty() || beats_per_bar == 0 || !downbeat_secs.is_finite() {
        return 0;
    }
    let idx = beat_index_nearest_to(beats, downbeat_secs);
    let bpb = u64::from(beats_per_bar);
    u8::try_from((idx as u64) % bpb).unwrap_or(0)
}

/// Maximum RMS-residual increase (in ms) we tolerate when snapping
/// an auto-detected BPM to the nearest integer. 3 ms is roughly
/// half a sample at 44.1 kHz and well inside the per-beat
/// measurement noise — any worse than that and we're forcing a
/// fundamentally wrong tempo onto the grid (the rare genuinely
/// non-integer track) and should keep the LSQ result instead.
const INTEGER_SNAP_RMS_SLACK_MS: f32 = 3.0;

/// Snap `bpm_raw` to the nearest integer if it sits within
/// [`INTEGER_BPM_SNAP_TOLERANCE`] **and** the grid at the snapped
/// tempo (with anchor refit, see below) measures nearly as well
/// as the LSQ result against the ODF (see
/// [`INTEGER_SNAP_RMS_SLACK_MS`]). Returns the chosen
/// `(bpm, anchor, quality)` triple so the rest of the pipeline
/// uses a self-consistent grid and the drift indicator reflects
/// the actual fit, not the LSQ-best fit at a tempo we just
/// discarded.
///
/// **Anchor refit.** `lsq_refit_grid` is a joint OLS over
/// `(slope=1/period, intercept=anchor)`, so the LSQ-best anchor
/// is only optimal at the LSQ-best BPM — changing BPM in
/// isolation shifts every predicted beat time and inflates the
/// residuals by an artificial systematic-drift term that has
/// nothing to do with whether the integer tempo is correct. The
/// snap helper therefore re-optimises the anchor at the snapped
/// BPM (anchor = mean(t_i) − period · mean(i)) before measuring
/// quality. Without this the safety net mis-rejects the snap on
/// true-integer tracks — which is exactly the bug the user hit
/// when 133.02 refused to round to 133.0 after analysis.
fn snap_bpm_to_integer_if_safe(
    odf: &[f32],
    odf_sr: f64,
    bpm_raw: f64,
    anchor_secs: f64,
    duration_secs: f64,
    quality_raw: GridQuality,
) -> (f64, f64, GridQuality) {
    let bpm_snapped = snap_to_integer_bpm(bpm_raw, INTEGER_BPM_SNAP_TOLERANCE);
    if (bpm_snapped - bpm_raw).abs() < 1e-9 {
        eprintln!(
            "dub-bpm: integer-snap skipped — bpm_raw={bpm_raw:.4} not within \
             ±{INTEGER_BPM_SNAP_TOLERANCE} of an integer"
        );
        return (bpm_raw, anchor_secs, quality_raw);
    }
    let Some((anchor_snapped, quality_snapped)) =
        refit_anchor_at_bpm(odf, odf_sr, bpm_snapped, anchor_secs, duration_secs)
    else {
        eprintln!(
            "dub-bpm: integer-snap aborted — refit at {bpm_snapped:.2} returned None \
             (insufficient observations), keeping bpm_raw={bpm_raw:.4}"
        );
        return (bpm_raw, anchor_secs, quality_raw);
    };
    if quality_snapped.rms_ms <= quality_raw.rms_ms + INTEGER_SNAP_RMS_SLACK_MS {
        eprintln!(
            "dub-bpm: integer-snap ACCEPTED bpm_raw={bpm_raw:.4} -> bpm={bpm_snapped:.2} \
             (rms {:.2}ms -> {:.2}ms, slack {:.1}ms)",
            quality_raw.rms_ms, quality_snapped.rms_ms, INTEGER_SNAP_RMS_SLACK_MS
        );
        (bpm_snapped, anchor_snapped, quality_snapped)
    } else {
        eprintln!(
            "dub-bpm: integer-snap REJECTED bpm_raw={bpm_raw:.4} -> bpm={bpm_snapped:.2} \
             (rms {:.2}ms -> {:.2}ms exceeds slack {:.1}ms), keeping bpm_raw",
            quality_raw.rms_ms, quality_snapped.rms_ms, INTEGER_SNAP_RMS_SLACK_MS
        );
        (bpm_raw, anchor_secs, quality_raw)
    }
}

/// Refit the grid anchor (intercept) at a fixed `bpm`, then
/// measure the resulting `GridQuality`. Used by the integer-BPM
/// snap to compare apples-to-apples against the joint-LSQ result
/// instead of holding an anchor that was optimal at a slightly
/// different tempo.
///
/// Two iterations: the first pass finds the observed peaks
/// inside windows centred on the initial anchor; the second pass
/// re-centres the windows on the freshly-refit anchor so a
/// systematic-drift residual (the typical signature of a small
/// BPM mismatch) doesn't bias the observation set. Empirically
/// converges in one or two passes for realistic LSQ snap shifts
/// (≤ 0.10 BPM ⇒ at most a few ms of anchor drift).
fn refit_anchor_at_bpm(
    odf: &[f32],
    odf_sr: f64,
    bpm: f64,
    anchor_init: f64,
    duration_secs: f64,
) -> Option<(f64, GridQuality)> {
    if !bpm.is_finite() || bpm <= 0.0 || !anchor_init.is_finite() {
        return None;
    }
    let period = 60.0 / bpm;
    let half_window = LSQ_SEARCH_FRACTION * period;
    let mut anchor = anchor_init;
    let mut observations: Vec<(f64, f64)> = Vec::new();
    let mut beat_count: i64 = 0;

    for _ in 0..2 {
        beat_count = ((duration_secs - anchor) / period).floor() as i64;
        if beat_count < 2 {
            return None;
        }
        observations.clear();
        for i in 0..=beat_count {
            let predicted = anchor + f64::from(i as i32) * period;
            if let Some((peak_secs, peak_val, second_val)) =
                strongest_peak_in_window(odf, predicted, half_window, odf_sr)
            {
                if second_val <= 0.0
                    || f64::from(peak_val) / f64::from(second_val) >= LSQ_MIN_PEAK_RATIO
                {
                    observations.push((f64::from(i as i32), peak_secs));
                }
            }
        }
        if observations.len() < 3 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = observations.len() as f64;
        let mean_t: f64 = observations.iter().map(|(_, t)| t).sum::<f64>() / n;
        let mean_i: f64 = observations.iter().map(|(i, _)| i).sum::<f64>() / n;
        anchor = mean_t - period * mean_i;
    }

    if observations.is_empty() || beat_count < 2 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let kept_fraction = observations.len() as f32 / (beat_count as f32 + 1.0);
    let residuals: Vec<f64> = observations
        .iter()
        .map(|(i, t)| t - (anchor + i * period))
        .collect();
    Some((
        anchor,
        quality_from_residuals(&residuals, &observations, kept_fraction),
    ))
}

/// First ODF time (in seconds) where the half-wave-rectified
/// spectral flux crosses an adaptive noise floor. Retained as a
/// generic "where does the audible content begin" utility for
/// possible reuse in pre-roll trim heuristics; the auto downbeat
/// path no longer uses it (PRD-BEATS round 4 follow-up replaced
/// the broadband-audibility walk with [`first_kick_peak_secs`]).
///
/// Threshold = 10 % of the 95th-percentile ODF magnitude over the
/// entire track. Percentile (not max) keeps a single loud transient
/// from compressing the threshold; 10 % is comfortably above the
/// noise floor of real recordings without being so high that a
/// soft pad intro reads as silence. Returns `None` if the ODF is
/// empty or entirely zero (degenerate track) — callers fall back
/// to t = 0 and use the raw anchor.
#[cfg(test)]
#[must_use]
fn first_audible_secs(odf: &[f32], odf_sr: f64) -> Option<f64> {
    if odf.is_empty() || !odf_sr.is_finite() || odf_sr <= 0.0 {
        return None;
    }
    let mut sorted: Vec<f32> = odf.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95_idx = ((sorted.len() * 95) / 100).min(sorted.len() - 1);
    let p95 = sorted[p95_idx];
    if !p95.is_finite() || p95 <= 0.0 {
        return None;
    }
    let threshold = p95 * 0.10;
    odf.iter()
        .position(|&v| v.is_finite() && v >= threshold)
        .map(|i| i as f64 / odf_sr)
}

/// Time of the **first kick** in the track (parabolically refined
/// peak of the kick-band ODF, in seconds). Used by the auto grid
/// pipeline to place bar 1 on the user's perceived "1": walks
/// `kick_odf` forward until a sample crosses `noise_floor`, then
/// follows the monotone-rising run to the local maximum and
/// applies the same parabolic refinement [`parabolic_peak_in_window`]
/// uses for sub-frame accuracy.
///
/// Why kick band (band 0, ~30–68 Hz) and not the broadband ODF:
/// a track that starts with a hi-hat / vocal / pad intro before
/// the kick drops would otherwise read its first audible-anything
/// as bar 1, which is rarely musically correct. The kick band
/// has no hi-hat or vocal energy, so the function fires on the
/// actual first kick.
///
/// Returns `None` when the kick ODF is entirely below
/// `noise_floor` (tracks with no audible kick — rare; pure melody
/// piece). Callers fall back to the [`find_downbeat_offset`]
/// best-scored phase in that case.
#[must_use]
fn first_kick_peak_secs(kick_odf: &[f32], odf_sr: f64, noise_floor: f32) -> Option<f64> {
    if kick_odf.is_empty() || !odf_sr.is_finite() || odf_sr <= 0.0 {
        return None;
    }
    let n = kick_odf.len();
    // Walk until the first sample crosses the noise floor.
    let mut i = 0usize;
    while i < n {
        let v = kick_odf[i];
        if v.is_finite() && v > noise_floor {
            break;
        }
        i += 1;
    }
    if i >= n {
        return None;
    }
    // Now climb to the local maximum (monotone-rising run from
    // here; stop at the first non-rising sample). Plain max-scan
    // would catch a louder secondary peak elsewhere in the
    // attack envelope rather than the first kick.
    let mut peak_idx = i;
    while peak_idx + 1 < n {
        let next = kick_odf[peak_idx + 1];
        if !next.is_finite() || next < kick_odf[peak_idx] {
            break;
        }
        peak_idx += 1;
    }
    // Parabolic refinement around the discrete peak.
    let refined_idx = if peak_idx > 0 && peak_idx + 1 < n {
        let y0 = f64::from(kick_odf[peak_idx - 1]);
        let y1 = f64::from(kick_odf[peak_idx]);
        let y2 = f64::from(kick_odf[peak_idx + 1]);
        let denom = 2.0 * (y0 - 2.0 * y1 + y2);
        if denom.abs() > 1e-9 {
            #[allow(clippy::cast_precision_loss)]
            let base = peak_idx as f64;
            let frac = ((y0 - y2) / denom).clamp(-1.0, 1.0);
            base + frac
        } else {
            #[allow(clippy::cast_precision_loss)]
            {
                peak_idx as f64
            }
        }
    } else {
        #[allow(clippy::cast_precision_loss)]
        {
            peak_idx as f64
        }
    };
    Some(refined_idx / odf_sr)
}

/// Fraction of the track-wide p95 ODF magnitude that a snap-window
/// peak must clear to count as a real transient (see
/// [`snap_to_nearest_transient`]). 12 % sits comfortably above the
/// kick / broadband noise floor of real recordings (master-bus
/// hiss, tape rumble, room tone all read ≲ 5 % of p95) and well
/// below the energy of even soft musical onsets (a soft snare or
/// hi-hat ghosts at 20–40 % of the loudest transient). Tuned
/// empirically against the click-track golden in
/// [`snap_to_nearest_transient_recovers_click_position`] and the
/// silent-window regression in
/// [`snap_to_nearest_transient_rejects_silent_window`].
const SNAP_NOISE_FLOOR_FRAC: f32 = 0.12;

/// 95th-percentile-derived noise floor for `odf`. Returns the
/// threshold that [`parabolic_peak_in_window`] should treat as
/// "still silence". Returns `0.0` (no rejection) for empty or
/// all-non-finite ODFs so the caller's snap degenerates to the
/// pre-noise-floor behaviour rather than producing a hard `None`
/// in the degenerate case (a degenerate ODF on a real track
/// means the analyzer failed upstream; the snap can't recover
/// it, but it shouldn't make things worse either).
#[must_use]
fn odf_noise_floor(odf: &[f32], fraction_of_p95: f32) -> f32 {
    if odf.is_empty() || !fraction_of_p95.is_finite() || fraction_of_p95 <= 0.0 {
        return 0.0;
    }
    let mut sorted: Vec<f32> = odf.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return 0.0;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95_idx = ((sorted.len() * 95) / 100).min(sorted.len() - 1);
    let p95 = sorted[p95_idx];
    if !p95.is_finite() || p95 <= 0.0 {
        return 0.0;
    }
    p95 * fraction_of_p95
}

fn refine_full_pipeline(
    odf: &[f32],
    odf_sr: f64,
    bpm_init: f64,
    duration_secs: f64,
    fixed_anchor: Option<f64>,
) -> Option<(f64, f64, GridQuality)> {
    let (period, _phase) = sweep_and_parabolic(
        odf,
        odf_sr,
        bpm_init,
        AUTO_REFINE_RANGE_PCT,
        AUTO_REFINE_STEP_PCT,
    )?;
    let bpm_coarse = 60.0 * odf_sr / period;
    let (period, phase) = sweep_and_parabolic(
        odf,
        odf_sr,
        bpm_coarse,
        ZOOM_REFINE_RANGE_PCT,
        ZOOM_REFINE_STEP_PCT,
    )?;
    let mut bpm = 60.0 * odf_sr / period;
    let mut anchor = phase / odf_sr;

    if let Some(fixed) = fixed_anchor {
        anchor = fixed;
    }

    if let Some((bpm_refined, quality)) = lsq_refit_grid(
        odf,
        odf_sr,
        bpm,
        anchor,
        duration_secs,
        LSQ_SEARCH_FRACTION,
        fixed_anchor.is_some(),
    ) {
        bpm = bpm_refined;
        Some((bpm, anchor, quality))
    } else {
        Some((
            bpm,
            anchor,
            GridQuality {
                rms_ms: 999.0,
                p95_ms: 999.0,
                max_abs_ms: 999.0,
                kept_fraction: 0.0,
                drift_slope_ms_per_min: 999.0,
            },
        ))
    }
}

fn sweep_and_parabolic(
    odf: &[f32],
    odf_sr: f64,
    bpm_init: f64,
    range_pct: f64,
    step_pct: f64,
) -> Option<(f64, f64)> {
    let (period_init, phase_init, _) =
        discrete_period_search(odf, odf_sr, bpm_init, range_pct, step_pct)?;
    parabolic_refine_period(odf, period_init, phase_init, step_pct)
}

fn parabolic_refine_period(
    odf: &[f32],
    period: f64,
    phase: f64,
    step_pct: f64,
) -> Option<(f64, f64)> {
    let step = period * step_pct;
    let p0 = period - step;
    let p1 = period;
    let p2 = period + step;
    if p0 < 2.0 || (p2 * 2.0) >= odf.len() as f64 {
        return Some((period, phase));
    }
    let (_, s0) = best_phase_for_period(odf, p0);
    let (_, s1) = best_phase_for_period(odf, p1);
    let (_, s2) = best_phase_for_period(odf, p2);
    let denom = 2.0 * (s0 - 2.0 * s1 + s2);
    if denom.abs() < 1e-9 || (s1 - s0.max(s2)) / s1.abs().max(1e-9) < 1e-3 {
        return Some((period, phase));
    }
    let offset = ((s0 - s2) / denom).clamp(-1.0, 1.0);
    let refined_period = period + offset * step;
    let (refined_phase, _) = best_phase_for_period(odf, refined_period);
    Some((refined_period, refined_phase))
}

fn discrete_period_search(
    odf: &[f32],
    odf_sr: f64,
    bpm_init: f64,
    range_pct: f64,
    step_pct: f64,
) -> Option<(f64, f64, f64)> {
    if bpm_init <= 0.0 || odf_sr <= 0.0 || odf.is_empty() || step_pct <= 0.0 {
        return None;
    }
    let period_init = (60.0 * odf_sr) / bpm_init;
    if period_init < 2.0 || (period_init * 2.0) as usize >= odf.len() {
        return None;
    }

    let mut best: Option<(f64, f64, f64)> = None;
    let mut frac = -range_pct;
    while frac <= range_pct + 1e-12 {
        let period = period_init * (1.0 + frac);
        if period >= 2.0 && (period * 2.0) < odf.len() as f64 {
            let (phase, score) = best_phase_for_period(odf, period);
            let take = match best {
                Some((_, _, best_score)) => score > best_score,
                None => true,
            };
            if take {
                best = Some((period, phase, score));
            }
        }
        frac += step_pct;
    }
    best
}

fn refine_period_at_anchor(
    odf: &[f32],
    odf_sr: f64,
    bpm_init: f64,
    anchor_secs: f64,
    range_pct: f64,
    step_pct: f64,
) -> Option<f64> {
    if bpm_init <= 0.0 || odf_sr <= 0.0 || odf.is_empty() || step_pct <= 0.0 {
        return None;
    }
    let period_init = (60.0 * odf_sr) / bpm_init;
    if period_init < 2.0 {
        return None;
    }
    let anchor_odf = anchor_secs * odf_sr;
    if !anchor_odf.is_finite() {
        return None;
    }

    let mut best: Option<(f64, f64)> = None;
    let mut frac = -range_pct;
    while frac <= range_pct + 1e-12 {
        let period = period_init * (1.0 + frac);
        if period >= 2.0 {
            let score = score_grid(odf, anchor_odf, period);
            let take = match best {
                Some((_, best_score)) => score > best_score,
                None => true,
            };
            if take {
                best = Some((period, score));
            }
        }
        frac += step_pct;
    }
    let (period, _) = best?;
    Some(60.0 * odf_sr / period)
}

fn lsq_refit_grid(
    odf: &[f32],
    odf_sr: f64,
    bpm_init: f64,
    anchor_init: f64,
    duration_secs: f64,
    search_fraction: f64,
    anchor_fixed: bool,
) -> Option<(f64, GridQuality)> {
    let mut bpm = bpm_init;
    let mut anchor = anchor_init;
    let mut last_quality = None;

    for _ in 0..3 {
        let period = 60.0 / bpm;
        let half_window = search_fraction * period;
        let beat_count = ((duration_secs - anchor) / period).floor() as i64;
        if beat_count < 2 {
            return None;
        }

        let mut observations: Vec<(f64, f64)> = Vec::new();
        for i in 0..=beat_count {
            let predicted = anchor + f64::from(i as i32) * period;
            if let Some((peak_secs, peak_val, second_val)) =
                strongest_peak_in_window(odf, predicted, half_window, odf_sr)
            {
                if second_val <= 0.0
                    || f64::from(peak_val) / f64::from(second_val) >= LSQ_MIN_PEAK_RATIO
                {
                    observations.push((f64::from(i as i32), peak_secs));
                }
            }
        }

        if observations.len() < 3 {
            return None;
        }

        let kept_fraction = observations.len() as f32 / (beat_count as f32 + 1.0);

        let (slope, intercept, residuals) = if anchor_fixed {
            let mut num = 0.0;
            let mut den = 0.0;
            for &(i, t) in &observations {
                num += (t - anchor) * i;
                den += i * i;
            }
            if den < 1e-12 {
                return None;
            }
            let slope = num / den;
            let mut residuals = Vec::with_capacity(observations.len());
            for &(i, t) in &observations {
                residuals.push(t - (anchor + i * slope));
            }
            (slope, anchor, residuals)
        } else {
            ols_line(&observations)?
        };

        if slope <= 0.0 {
            return None;
        }

        let new_bpm = 60.0 / slope;
        if !new_bpm.is_finite() || new_bpm <= 0.0 {
            return None;
        }

        let quality = quality_from_residuals(&residuals, &observations, kept_fraction);
        last_quality = Some(quality);

        if (new_bpm - bpm).abs() < 1e-9 {
            return Some((new_bpm, quality));
        }
        bpm = new_bpm;
        anchor = intercept;
    }

    last_quality.map(|q| (bpm, q))
}

fn ols_line(observations: &[(f64, f64)]) -> Option<(f64, f64, Vec<f64>)> {
    let n = observations.len() as f64;
    let sum_i: f64 = observations.iter().map(|(i, _)| i).sum();
    let sum_t: f64 = observations.iter().map(|(_, t)| t).sum();
    let sum_i2: f64 = observations.iter().map(|(i, _)| i * i).sum();
    let sum_it: f64 = observations.iter().map(|(i, t)| i * t).sum();
    let denom = n * sum_i2 - sum_i * sum_i;
    if denom.abs() < 1e-12 {
        return None;
    }
    let slope = (n * sum_it - sum_i * sum_t) / denom;
    let intercept = (sum_t - slope * sum_i) / n;
    let residuals: Vec<f64> = observations
        .iter()
        .map(|(i, t)| t - (intercept + i * slope))
        .collect();
    Some((slope, intercept, residuals))
}

fn quality_from_residuals(
    residuals: &[f64],
    observations: &[(f64, f64)],
    kept_fraction: f32,
) -> GridQuality {
    let ms: Vec<f64> = residuals.iter().map(|r| r.abs() * 1000.0).collect();
    let n = ms.len() as f64;
    let rms_ms = ((ms.iter().map(|x| x * x).sum::<f64>() / n).sqrt()) as f32;
    let mut sorted = ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95_ms = sorted[(sorted.len() as f64 * 0.95).floor() as usize] as f32;
    let max_abs_ms = *sorted.last().unwrap_or(&0.0) as f32;

    let mean_t: f64 = observations.iter().map(|(_, t)| t).sum::<f64>() / n;
    let mean_r: f64 = residuals.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for ((_, t), r) in observations.iter().zip(residuals.iter()) {
        let dt = t - mean_t;
        num += dt * (r - mean_r);
        den += dt * dt;
    }
    let slope_secs_per_sec = if den > 1e-12 { num / den } else { 0.0 };
    let drift_slope_ms_per_min = (slope_secs_per_sec * 60.0 * 1000.0) as f32;

    GridQuality {
        rms_ms,
        p95_ms,
        max_abs_ms,
        kept_fraction,
        drift_slope_ms_per_min,
    }
}

fn strongest_peak_in_window(
    odf: &[f32],
    center_secs: f64,
    half_window_secs: f64,
    odf_sr: f64,
) -> Option<(f64, f32, f32)> {
    let center = center_secs * odf_sr;
    let half = half_window_secs * odf_sr;
    if !center.is_finite() || half <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = (center - half).floor().max(0.0) as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let hi = (center + half).ceil() as usize;
    let hi = hi.min(odf.len().saturating_sub(1));
    if lo >= hi {
        return None;
    }
    let mut best_idx = lo;
    let mut best_val = odf[lo];
    let mut second_val = 0.0f32;
    #[allow(clippy::needless_range_loop)]
    for idx in lo..=hi {
        let v = odf[idx];
        if v > best_val {
            second_val = best_val;
            best_val = v;
            best_idx = idx;
        } else if v > second_val {
            second_val = v;
        }
    }
    let peak_secs = best_idx as f64 / odf_sr;
    Some((peak_secs, best_val, second_val))
}

fn find_downbeat_offset(
    kick_odf: &[f32],
    broadband_odf: &[f32],
    odf_sr: f64,
    period: f64,
    anchor_secs: f64,
    duration_secs: f64,
    beats_per_bar: u8,
) -> (u8, f32) {
    let bar_period_odf = period * f64::from(beats_per_bar) * odf_sr;
    let mut scores = [0.0f64; 4];
    for offset in 0..beats_per_bar {
        let phase_odf = (anchor_secs + f64::from(offset) * period) * odf_sr;
        scores[usize::from(offset)] = score_grid(kick_odf, phase_odf, bar_period_odf);
    }
    let mut ranked: Vec<(u8, f64)> = (0..beats_per_bar)
        .map(|o| (o, scores[usize::from(o)]))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let best_offset = ranked[0].0;
    let best_score = ranked[0].1;
    let second_score = ranked.get(1).map(|(_, s)| *s).unwrap_or(0.0);
    let confidence = if second_score > 1e-9 {
        (best_score / second_score) as f32
    } else {
        1.0
    };

    if confidence < DOWNBEAT_CONFIDENCE_TIEBREAK as f32 {
        if let Some(tie_offset) = kick_only_intro_tiebreaker(
            kick_odf,
            broadband_odf,
            odf_sr,
            period,
            anchor_secs,
            duration_secs,
            beats_per_bar,
        ) {
            return (tie_offset, confidence);
        }
    }
    (best_offset, confidence)
}

fn kick_only_intro_tiebreaker(
    kick_odf: &[f32],
    broadband_odf: &[f32],
    odf_sr: f64,
    period: f64,
    anchor_secs: f64,
    duration_secs: f64,
    beats_per_bar: u8,
) -> Option<u8> {
    let beat_count = ((duration_secs / period).floor() as usize).saturating_add(1);
    if beat_count < KICK_ONLY_MIN_BEATS {
        return None;
    }

    let mut non_kick = vec![0.0f64; beat_count];
    #[allow(clippy::needless_range_loop)]
    for i in 0..beat_count {
        let t = anchor_secs + i as f64 * period;
        let kick = peak_value_at(kick_odf, t, period * 0.1, odf_sr);
        let broad = peak_value_at(broadband_odf, t, period * 0.1, odf_sr);
        non_kick[i] = (broad - kick).max(0.0);
    }

    let mut sorted = non_kick.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    if median <= 1e-9 {
        return None;
    }
    let threshold = median * KICK_ONLY_ENERGY_FRACTION;

    let kick_only: Vec<bool> = non_kick.iter().map(|&v| v <= threshold).collect();

    let intro_end = INTRO_OUTRO_WINDOW_SECS.min(duration_secs);
    let outro_start = (duration_secs - INTRO_OUTRO_WINDOW_SECS).max(0.0);

    let mut best_offset = 0u8;
    let mut best_hits = 0u32;
    for offset in 0..beats_per_bar {
        let mut hits = 0u32;
        let mut i = 0i64;
        loop {
            let beat_idx = i * i64::from(beats_per_bar) + i64::from(offset);
            if beat_idx < 0 {
                i += 1;
                continue;
            }
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let idx = beat_idx as usize;
            if idx >= beat_count {
                break;
            }
            let t = anchor_secs + beat_idx as f64 * period;
            let in_intro = t <= intro_end;
            let in_outro = t >= outro_start;
            if (in_intro || in_outro) && kick_only[idx] {
                hits += 1;
            }
            i += 1;
        }
        if hits > best_hits {
            best_hits = hits;
            best_offset = offset;
        }
    }

    if best_hits > 0 {
        Some(best_offset)
    } else {
        None
    }
}

fn peak_value_at(odf: &[f32], center_secs: f64, half_window_secs: f64, odf_sr: f64) -> f64 {
    strongest_peak_in_window(odf, center_secs, half_window_secs, odf_sr)
        .map(|(_, v, _)| f64::from(v))
        .unwrap_or(0.0)
}

/// Snap a tap time to the nearest ODF transient inside
/// `±half_window_secs`. Searches the kick ODF first (the user is
/// almost always tapping the kick), falls back to the broadband
/// ODF if the kick band has nothing usable in the window. Returns
/// `None` only when both ODFs are flat in the window — in which
/// case the caller should keep the raw tap.
///
/// Sub-frame resolution: applies parabolic interpolation around
/// the discrete peak so the snapped time is accurate to ~1 ms at
/// 44.1 kHz / HOP=512 instead of the 11.6 ms frame floor.
/// Snap a tap to the nearest significant transient (kick first,
/// broadband fallback). Returns `None` when neither ODF has a
/// peak above its respective noise floor inside the window — in
/// that case the caller must use the raw tap so we don't drag
/// the grid onto a sub-noise wiggle. The earlier "any peak with
/// `best_val > 0.0` wins" behavior caused the user-reported "1
/// lands on a non-transient" because every silent window has
/// some non-zero ODF energy and parabolic refinement happily
/// landed on the loudest grain of noise.
fn snap_to_nearest_transient(
    kick_odf: &[f32],
    broadband_odf: &[f32],
    odf_sr: f64,
    tap_secs: f64,
    half_window_secs: f64,
    kick_noise_floor: f32,
    broadband_noise_floor: f32,
) -> Option<f64> {
    if !tap_secs.is_finite() || tap_secs < 0.0 || half_window_secs <= 0.0 {
        return None;
    }
    if let Some(t) = parabolic_peak_in_window(
        kick_odf,
        tap_secs,
        half_window_secs,
        odf_sr,
        kick_noise_floor,
    ) {
        return Some(t);
    }
    parabolic_peak_in_window(
        broadband_odf,
        tap_secs,
        half_window_secs,
        odf_sr,
        broadband_noise_floor,
    )
}

/// Find the index of the strongest ODF sample inside the window,
/// then refine its position with parabolic interpolation against
/// its two neighbours for sub-frame accuracy. Returns `None` when
/// the best sample inside the window is below `noise_floor`,
/// keeping snaps off pre-roll silence, breakdown bars, and the
/// dead space between musical sections.
fn parabolic_peak_in_window(
    odf: &[f32],
    center_secs: f64,
    half_window_secs: f64,
    odf_sr: f64,
    noise_floor: f32,
) -> Option<f64> {
    let center = center_secs * odf_sr;
    let half = half_window_secs * odf_sr;
    if !center.is_finite() || half <= 0.0 || odf.is_empty() {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = (center - half).floor().max(0.0) as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let hi = ((center + half).ceil() as usize).min(odf.len().saturating_sub(1));
    if lo >= hi {
        return None;
    }

    let mut best_idx = lo;
    let mut best_val = odf[lo];
    for (offset, &v) in odf[(lo + 1)..=hi].iter().enumerate() {
        if v > best_val {
            best_val = v;
            best_idx = lo + 1 + offset;
        }
    }
    if !best_val.is_finite() || best_val <= noise_floor.max(0.0) {
        return None;
    }

    // Parabolic refinement: requires both neighbours.
    let refined_idx = if best_idx > 0 && best_idx + 1 < odf.len() {
        let y0 = f64::from(odf[best_idx - 1]);
        let y1 = f64::from(odf[best_idx]);
        let y2 = f64::from(odf[best_idx + 1]);
        let denom = 2.0 * (y0 - 2.0 * y1 + y2);
        if denom.abs() > 1e-9 {
            let frac = ((y0 - y2) / denom).clamp(-1.0, 1.0);
            best_idx as f64 + frac
        } else {
            best_idx as f64
        }
    } else {
        best_idx as f64
    };

    Some(refined_idx / odf_sr)
}

/// Window (in seconds) over which [`amplitude_peak_offset_secs`]
/// searches forward from each ODF-aligned beat for the broadband
/// amplitude peak. Sized for typical kick / snare attack ramps:
/// the spectral-flux ODF peaks during the rising edge of the
/// attack (since it tracks the *derivative* of band magnitudes),
/// while the visible amplitude peak lands 5–25 ms later when the
/// membrane / sample reaches max displacement. 30 ms covers
/// almost every popular-music transient with a few ms of margin
/// for parabolic refinement noise.
const AMPLITUDE_PEAK_SHIFT_WINDOW_SECS: f64 = 0.030;

/// Fraction of beats (sorted by peak amplitude, loudest first)
/// that contribute to the median amplitude-peak offset. 50 %
/// excludes silent breakdowns, ghost notes, and beats that fall
/// inside a melody-only section where the broadband envelope is
/// driven by sustained tones rather than transients. Keeps the
/// median anchored on the percussive backbone of the track.
const AMPLITUDE_PEAK_TOP_BEATS_FRACTION: f64 = 0.5;

/// Median offset (seconds) between each beat's grid time and the
/// nearest broadband amplitude peak in
/// `[beat_secs, beat_secs + AMPLITUDE_PEAK_SHIFT_WINDOW_SECS]`.
///
/// PRD-BEATS round 4 follow-up — visual grid alignment.
/// `snap_to_nearest_transient` lands grid lines at the spectral-
/// flux ODF peak (the perceptual onset, ~5–25 ms before the
/// visible amplitude peak). Most reference DJ apps (Serato,
/// Rekordbox, Traktor) place grid lines at the visible peak so
/// they sit inside the "fat" part of the kick where the line
/// reads at glance distance. This helper computes a single
/// per-track shift the caller adds to the anchor; uniform shift
/// preserves the LSQ-best period and only moves the rendered
/// phase.
///
/// Robustness:
/// * Only the loudest [`AMPLITUDE_PEAK_TOP_BEATS_FRACTION`] of
///   beats contribute. Breakdowns / silent intros get filtered
///   out automatically.
/// * Returns `0.0` when fewer than 3 beats have a measurable
///   broadband peak inside the window (the caller's grid is
///   already where it should be).
/// * Result is clamped to `[0, window]` so a degenerate beat
///   whose "peak" is at the window edge can't drag the entire
///   grid past the next ODF peak.
///
/// `samples` is interleaved per-channel audio (the same layout
/// the BPM analyzer consumes); `channels` is the channel count
/// (1 = mono, 2 = stereo); `sample_rate` is the audio sample
/// rate in Hz.
#[must_use]
fn amplitude_peak_offset_secs(
    beats: &[f64],
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> f64 {
    if beats.is_empty() || samples.is_empty() || sample_rate == 0 || channels == 0 {
        return 0.0;
    }
    let chans = usize::from(channels);
    let frames_total = samples.len() / chans;
    if frames_total == 0 {
        return 0.0;
    }
    let sr_f = f64::from(sample_rate);
    let window_frames = (AMPLITUDE_PEAK_SHIFT_WINDOW_SECS * sr_f).ceil().max(1.0) as usize;

    // For each beat: find the max |sample| inside the window and
    // record (peak_amp, offset_secs).
    let mut offsets: Vec<(f32, f64)> = Vec::with_capacity(beats.len());
    for &beat in beats {
        if !beat.is_finite() || beat < 0.0 {
            continue;
        }
        let start_frame_f = beat * sr_f;
        if !start_frame_f.is_finite() {
            continue;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let start_frame = start_frame_f.round() as usize;
        if start_frame >= frames_total {
            continue;
        }
        let end_frame = (start_frame + window_frames).min(frames_total);
        let mut peak_amp = 0.0f32;
        let mut peak_frame = start_frame;
        for f in start_frame..end_frame {
            let frame_offset = f * chans;
            let mut frame_max = 0.0f32;
            for c in 0..chans {
                let v = samples[frame_offset + c].abs();
                if v > frame_max {
                    frame_max = v;
                }
            }
            if frame_max > peak_amp {
                peak_amp = frame_max;
                peak_frame = f;
            }
        }
        if peak_amp > 0.0 {
            #[allow(clippy::cast_precision_loss)]
            let offset_frames = (peak_frame - start_frame) as f64;
            offsets.push((peak_amp, offset_frames / sr_f));
        }
    }

    if offsets.len() < 3 {
        return 0.0;
    }

    // Top-half-by-amplitude only — kill silent/ghost beats.
    offsets.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let take_n = ((offsets.len() as f64 * AMPLITUDE_PEAK_TOP_BEATS_FRACTION).ceil() as usize)
        .max(1)
        .min(offsets.len());
    let mut top_offsets: Vec<f64> = offsets[..take_n].iter().map(|(_, o)| *o).collect();
    top_offsets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = top_offsets[top_offsets.len() / 2];

    median.clamp(0.0, AMPLITUDE_PEAK_SHIFT_WINDOW_SECS)
}

/// Shift every beat in `grid` forward by the median amplitude-peak
/// offset measured against `samples`. Recomputes `bar_phase` for
/// the new beat positions so the previously-yellow beat stays
/// yellow after the shift (uniform translation = bar phase is
/// preserved, but `uniform_beats`' wrap-back may renumber the
/// array). The LSQ `bpm` and `quality` fields stay untouched —
/// the shift moves only the rendered phase, not the algorithm's
/// internal best-fit residuals.
///
/// No-op when the offset is negligible (< 1 ms) or the grid is
/// empty.
#[must_use]
fn shift_grid_to_amplitude_peak(
    grid: BeatGrid,
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> BeatGrid {
    if grid.beats.is_empty() || grid.bpm <= 0.0 {
        return grid;
    }
    let delta = amplitude_peak_offset_secs(&grid.beats, samples, sample_rate, channels);
    if delta < 1e-3 {
        return grid;
    }
    // Pick the original downbeat's time *before* shifting so the
    // post-shift bar_phase recomputation lands on the same musical
    // event (now shifted by `delta`). `beats[bar_phase]` is the
    // first downbeat in the original array.
    let original_downbeat_idx = usize::from(grid.bar_phase).min(grid.beats.len() - 1);
    let original_downbeat_time = grid.beats[original_downbeat_idx];
    let shifted_downbeat_time = original_downbeat_time + delta;

    // Anchor the new uniform grid at the shifted downbeat so the
    // bar-phase computation below lands on the same musical event.
    let duration_secs = grid.beats.last().copied().unwrap_or(0.0) + delta + 60.0 / grid.bpm;
    let mut new_beats = uniform_beats(grid.bpm, shifted_downbeat_time, duration_secs);
    // Trim any wrap-back beat that landed past the original track
    // duration (uniform_beats was given a generous duration so we
    // don't accidentally lose the last beat to floating-point
    // truncation; bound it back to what the original grid covered).
    let original_end = grid.beats.last().copied().unwrap_or(duration_secs) + delta + 1e-9;
    new_beats.retain(|&t| t <= original_end);
    if new_beats.is_empty() {
        return grid;
    }

    let new_bar_phase =
        bar_phase_for_downbeat_time(&new_beats, shifted_downbeat_time, grid.beats_per_bar);

    BeatGrid {
        beats: new_beats,
        bar_phase: new_bar_phase,
        ..grid
    }
}

/// Measure how tightly a candidate `(bpm, anchor)` fits the ODF
/// without modifying either. Mirrors `lsq_refit_grid`'s residual
/// math but holds the grid fixed — used to populate `GridQuality`
/// for user-supplied grids (taps, manual entry).
fn measure_grid_quality(
    odf: &[f32],
    odf_sr: f64,
    bpm: f64,
    anchor: f64,
    duration_secs: f64,
) -> Option<GridQuality> {
    if !bpm.is_finite() || bpm <= 0.0 || !anchor.is_finite() {
        return None;
    }
    let period = 60.0 / bpm;
    let half_window = LSQ_SEARCH_FRACTION * period;
    let beat_count = ((duration_secs - anchor) / period).floor() as i64;
    if beat_count < 2 {
        return None;
    }

    let mut observations: Vec<(f64, f64)> = Vec::new();
    let mut residuals: Vec<f64> = Vec::new();
    for i in 0..=beat_count {
        let predicted = anchor + f64::from(i as i32) * period;
        if let Some((peak_secs, peak_val, second_val)) =
            strongest_peak_in_window(odf, predicted, half_window, odf_sr)
        {
            if second_val <= 0.0
                || f64::from(peak_val) / f64::from(second_val) >= LSQ_MIN_PEAK_RATIO
            {
                observations.push((f64::from(i as i32), peak_secs));
                residuals.push(peak_secs - predicted);
            }
        }
    }

    if observations.len() < 3 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let kept_fraction = observations.len() as f32 / (beat_count as f32 + 1.0);
    Some(quality_from_residuals(
        &residuals,
        &observations,
        kept_fraction,
    ))
}

fn weighted_median_bpm_from_taps(tap_times: &[f64]) -> Option<f64> {
    if tap_times.len() < 2 {
        return None;
    }
    let mut intervals: Vec<f64> = tap_times.windows(2).map(|w| w[1] - w[0]).collect();
    // Drop intervals that map to BPMs outside the analyzable
    // range. An accidental double-tap inside a single beat
    // produces an interval like 50 ms (1200 BPM) which would
    // otherwise drag the unweighted median way off the real
    // tempo, and a "missed beat" pair produces an interval long
    // enough to fall under `MIN_BPM`. Both are clearly not the
    // tempo the user was tapping; the rest of the intervals
    // are.
    let min_interval = 60.0 / MAX_BPM;
    let max_interval = 60.0 / MIN_BPM;
    intervals.retain(|dt| dt.is_finite() && *dt >= min_interval && *dt <= max_interval);
    if intervals.is_empty() {
        return None;
    }
    let raw_median = {
        let mut sorted = intervals.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    };
    let mut weighted: Vec<(f64, f64)> = intervals
        .iter()
        .map(|&dt| {
            let deviation = ((dt - raw_median) / raw_median).abs();
            let weight = if deviation > 0.25 {
                0.25
            } else {
                1.0 - deviation
            };
            (dt, weight)
        })
        .collect();
    weighted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let total_weight: f64 = weighted.iter().map(|(_, w)| w).sum();
    let mut acc = 0.0;
    for (dt, w) in weighted {
        acc += w;
        if acc >= total_weight * 0.5 {
            return Some(60.0 / dt);
        }
    }
    Some(60.0 / raw_median)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn best_phase_for_period(odf: &[f32], period: f64) -> (f64, f64) {
    let period_int = period.ceil().max(1.0) as usize;
    let mut best_phase = 0usize;
    let mut best_score = f64::NEG_INFINITY;
    for phi in 0..period_int {
        let score = score_grid(odf, phi as f64, period);
        if score > best_score {
            best_score = score;
            best_phase = phi;
        }
    }
    let prev_phi = if best_phase == 0 {
        period_int - 1
    } else {
        best_phase - 1
    };
    let next_phi = (best_phase + 1) % period_int;
    let y0 = score_grid(odf, prev_phi as f64, period);
    let y1 = best_score;
    let y2 = score_grid(odf, next_phi as f64, period);
    let denom = 2.0 * (y0 - 2.0 * y1 + y2);
    let frac_offset = if denom.abs() > 1e-9 {
        ((y0 - y2) / denom).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    (best_phase as f64 + frac_offset, best_score)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn score_grid(odf: &[f32], phase: f64, period: f64) -> f64 {
    if !phase.is_finite() || !period.is_finite() || period <= 0.0 || odf.is_empty() {
        return f64::NEG_INFINITY;
    }
    let len = odf.len();
    let mut score = 0.0f64;
    let mut i: usize = 0;
    loop {
        let idx_f = phase + i as f64 * period;
        if idx_f >= len as f64 - 1.0 {
            break;
        }
        if idx_f >= 0.0 {
            let lo = idx_f.floor() as usize;
            let hi = (lo + 1).min(len - 1);
            let frac = idx_f - lo as f64;
            let val = (1.0 - frac) * f64::from(odf[lo]) + frac * f64::from(odf[hi]);
            score += val;
        }
        i += 1;
        if i > 4 * len {
            break;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::octave_profile::OctaveProfile;
    use crate::synthetic::click_track;

    const SR: u32 = 48_000;

    fn beats_are_uniform(beats: &[f64], bpm: f64) -> bool {
        if beats.len() < 2 {
            return true;
        }
        let period = 60.0 / bpm;
        beats
            .windows(2)
            .all(|w| ((w[1] - w[0]) - period).abs() < 1e-9)
    }

    /// PRD-BEATS C2 (round 4) — `bar_phase_from_tap` is the new
    /// "set the 1" primitive. It must return the index of the
    /// beat nearest the tap modulo `beats_per_bar`, leaving the
    /// grid spacing untouched. Pure rotation contract: callers
    /// stamp the returned value onto `BeatGrid.bar_phase` and the
    /// renderer flips which beats are yellow on the next frame.
    #[test]
    fn bar_phase_rotates_to_beat_nearest_tap() {
        // 120 BPM grid → 0.5 s period; beats at 0.0, 0.5, 1.0,
        // 1.5, … so beats[3] = 1.5 s. A tap at 1.49 s should
        // pick beats[3] and return `3 mod 4 == 3`.
        let mut grid = BeatGrid::none();
        grid.bpm = 120.0;
        grid.beats = (0..16).map(|i| i as f64 * 0.5).collect();
        grid.beats_per_bar = 4;
        grid.confidence = 1.0;

        assert_eq!(bar_phase_from_tap(&grid, 0.0), 0);
        assert_eq!(bar_phase_from_tap(&grid, 0.5), 1);
        assert_eq!(bar_phase_from_tap(&grid, 1.49), 3);
        assert_eq!(bar_phase_from_tap(&grid, 2.0), 0);
        assert_eq!(bar_phase_from_tap(&grid, 2.6), 1);
        // Tap miles past the last beat snaps to the last beat
        // (idx 15); 15 mod 4 == 3. Renderer then marks beats[3]
        // / [7] / [11] / [15] as downbeats.
        assert_eq!(bar_phase_from_tap(&grid, 999.0), 3);
    }

    #[test]
    fn bar_phase_from_tap_is_zero_on_degenerate_inputs() {
        // Empty grid → 0 (renderer fall-back path).
        assert_eq!(bar_phase_from_tap(&BeatGrid::none(), 1.5), 0);

        let mut grid = BeatGrid::none();
        grid.bpm = 120.0;
        grid.beats = vec![0.0, 0.5, 1.0];
        grid.beats_per_bar = 4;
        // Non-finite tap → 0 (caller's UI rejects this earlier
        // but be total).
        assert_eq!(bar_phase_from_tap(&grid, f64::NAN), 0);
        assert_eq!(bar_phase_from_tap(&grid, f64::INFINITY), 0);
    }

    #[test]
    fn click_120_bpm_emits_uniform_grid_at_500_ms() {
        let samples = click_track(120.0, 16.0, SR);
        let grid = analyze_beat_grid(&samples, SR, 1).expect("analysis");

        assert!(grid.confidence > 0.5, "got confidence {}", grid.confidence);
        assert!((grid.bpm - 120.0).abs() < 0.15);
        assert!(grid.beats.len() > 10);
        assert!(beats_are_uniform(&grid.beats, grid.bpm));

        let first = grid.beats[0];
        let nearest_click = (first / 0.5).round() * 0.5;
        assert!((first - nearest_click).abs() < 0.025);
    }

    #[test]
    fn phase_offset_quarter_second_recovered() {
        let bpm = 120.0;
        let mut samples = vec![0.0f32; SR as usize / 4];
        samples.extend(click_track(bpm, 12.0, SR));
        let grid = analyze_beat_grid(&samples, SR, 1).expect("analysis");

        assert!(grid.confidence > 0.4);
        assert!((grid.bpm - bpm).abs() < 0.1);
        assert!(beats_are_uniform(&grid.beats, grid.bpm));

        let first_aligned = grid
            .beats
            .iter()
            .find(|&&b| b >= 0.20)
            .copied()
            .expect("should have a beat after t=0.2s");
        assert!((first_aligned - 0.25).abs() < 0.03);
    }

    #[test]
    fn silence_returns_no_beats_or_zero_confidence() {
        let samples = vec![0.0f32; (SR * 12) as usize];
        let grid = analyze_beat_grid(&samples, SR, 1).expect("silence is valid input");
        assert!(grid.confidence < 0.3);
    }

    #[test]
    fn refined_grid_does_not_drift_over_a_long_track() {
        let true_bpm = 133.0;
        let samples = click_track(true_bpm, 60.0, SR);
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("analysis");
        assert!(grid.confidence > 0.4);
        assert!(grid.beats.len() > 100);
        assert!(beats_are_uniform(&grid.beats, grid.bpm));
        assert!(
            (grid.bpm - true_bpm).abs() < 0.05,
            "BPM should be within 0.05 of truth, got {}",
            grid.bpm
        );

        let last = *grid.beats.last().expect("beats");
        let nearest_click = (last / (60.0 / true_bpm)).round() * (60.0 / true_bpm);
        assert!(
            (last - nearest_click).abs() < 0.025,
            "last beat should sit on a click; got {last}, nearest {nearest_click}"
        );

        let quality = grid.quality.expect("quality");
        assert!(quality.auto_lock_safe());
    }

    #[test]
    fn relatch_at_user_downbeat_emits_uniform_grid() {
        let bpm = 120.0;
        let samples = click_track(bpm, 8.0, SR);
        let grid = latch_beat_grid_at_downbeat(&samples, SR, 1, bpm, 1.0, OctaveProfile::Default)
            .expect("relatch");
        assert!((grid.bpm - bpm).abs() < 1.0);
        assert!(beats_are_uniform(&grid.beats, grid.bpm));

        let first_after_one = grid
            .beats
            .iter()
            .find(|&&b| (b - 1.0).abs() < 0.05)
            .copied()
            .expect("a beat near user mark");
        assert!((first_after_one - 1.0).abs() < 0.05);
    }

    #[test]
    fn uniform_beats_fills_to_duration() {
        let beats = uniform_beats(120.0, 0.0, 10.0);
        assert!(!beats.is_empty());
        for w in beats.windows(2) {
            assert!(((w[1] - w[0]) - 0.5).abs() < 1e-12);
        }
        assert!(*beats.last().unwrap() <= 10.0);
        assert!(*beats.last().unwrap() > 9.4);
    }

    #[test]
    fn uniform_beats_walks_anchor_back_into_first_period() {
        let beats = uniform_beats(120.0, 5.25, 10.0);
        assert!(beats[0] >= 0.0 && beats[0] < 0.5);
        assert!((beats[0] - 0.25).abs() < 1e-9);
    }

    #[test]
    fn uniform_beats_rejects_degenerate_inputs() {
        assert!(uniform_beats(0.0, 0.0, 10.0).is_empty());
        assert!(uniform_beats(-120.0, 0.0, 10.0).is_empty());
        assert!(uniform_beats(120.0, 0.0, 0.0).is_empty());
        assert!(uniform_beats(120.0, f64::NAN, 10.0).is_empty());
    }

    #[test]
    fn grid_quality_auto_lock_on_click_track() {
        let samples = click_track(128.0, 60.0, SR);
        let grid = analyze_beat_grid(&samples, SR, 1).expect("analysis");
        let q = grid.quality.expect("quality");
        assert!(q.rms_ms < 8.0);
        assert!(q.auto_lock_safe());
    }

    /// Regression for the bug "tap-to-grid prefers ODF over user
    /// taps" (M11d.7a). The user taps 5 clean beats at 133 BPM on
    /// a click track; the resulting grid must respect 133.0 to
    /// within 0.1 BPM and the drift indicator must not trip.
    /// Pre-fix the pipeline ran `analyze_bpm_with_range_profile_*`
    /// over ±10 % followed by a full LSQ refit, which dragged the
    /// answer to ~132.88 on real tracks.
    ///
    /// M11d.7a round 2: with the integer-BPM snap in place, the
    /// resulting tempo must land **exactly** on the integer (no
    /// more 133.02 drift). 0.001 tolerance leaves room for fp
    /// noise from the weighted median.
    #[test]
    fn tap_grid_respects_user_bpm_within_0_1() {
        let true_bpm = 133.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let period = 60.0 / true_bpm;
        // Five clean taps starting one beat in. Tap[0] purposely
        // 6 ms late so the snap has to find the click.
        let tap_times: Vec<f64> = (0..5).map(|i| 1.0 + 0.006 + i as f64 * period).collect();
        let grid =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis");

        assert!(grid.confidence > 0.0, "should accept user taps");
        assert!(
            (grid.bpm - true_bpm).abs() < 0.001,
            "integer-BPM snap must land tap tempo exactly on 133.0; got {}",
            grid.bpm
        );
        let q = grid.quality.expect("quality");
        assert!(
            q.drift_slope_ms_per_min.abs() < 3.0,
            "drift must not trip on a clean tap session; got {} ms/min",
            q.drift_slope_ms_per_min
        );
    }

    /// Anchor must snap from the (intentionally late) first tap
    /// onto the actual click. Tolerance matches the existing
    /// `click_120_bpm_emits_uniform_grid_at_500_ms` check (25 ms)
    /// because spectral-flux onsets land a few frames before the
    /// click body. Crucially, the snapped anchor must be **closer
    /// to the click than the raw tap was** — that's the whole
    /// point of the snap.
    #[test]
    fn tap_grid_snaps_anchor_closer_than_raw_tap() {
        let true_bpm = 120.0;
        let samples = click_track(true_bpm, 16.0, SR);
        let period = 60.0 / true_bpm;
        let tap_offset = 0.030;
        let tap_times: Vec<f64> = (0..5)
            .map(|i| 1.0 + tap_offset + i as f64 * period)
            .collect();
        let grid =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis");

        assert!(grid.confidence > 0.0);
        let anchor = grid.beats[0];
        let nearest_click = (anchor / period).round() * period;
        let anchor_err = (anchor - nearest_click).abs();
        assert!(
            anchor_err < 0.025,
            "anchor must land within 25 ms of nearest click; got delta {} ms",
            anchor_err * 1000.0
        );
        assert!(
            anchor_err < tap_offset,
            "snap must pull closer than the raw {} ms tap offset; got delta {} ms",
            tap_offset * 1000.0,
            anchor_err * 1000.0
        );
    }

    /// One mistapped beat (skipped a kick → 2× interval) must not
    /// drag BPM more than 0.5 from the user's intent. Weighted
    /// median is what guards us; we never run OLS on raw taps.
    #[test]
    fn tap_grid_rejects_outlier_interval() {
        let true_bpm = 128.0;
        let samples = click_track(true_bpm, 20.0, SR);
        let period = 60.0 / true_bpm;
        // Four clean taps then one that skipped a beat.
        let tap_times: Vec<f64> = vec![
            1.0,
            1.0 + period,
            1.0 + 2.0 * period,
            1.0 + 3.0 * period,
            1.0 + 5.0 * period,
        ];
        let grid =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis");

        assert!(grid.confidence > 0.0);
        assert!(
            (grid.bpm - true_bpm).abs() < 0.5,
            "outlier interval must not drag BPM; got {}",
            grid.bpm
        );
    }

    /// The first tap must be tagged as the yellow downbeat (via
    /// `bar_phase`), and the grid must extend backward through
    /// the pre-roll silence as regular ticks. Round 4 follow-up
    /// flipped the contract from "no beats before the anchor"
    /// (which dropped pre-roll beats entirely) to "downbeat is
    /// the explicit bar phase, pre-roll beats render as grey
    /// ticks" so the user no longer sees an empty waveform
    /// before bar 1.
    #[test]
    fn tap_grid_first_beat_lands_on_snapped_anchor_not_pre_roll() {
        let true_bpm = 120.0;
        // Two beats of leading silence, then a click track from t = 1.0.
        let mut samples = vec![0.0f32; SR as usize];
        samples.extend(click_track(true_bpm, 12.0, SR));
        let period = 60.0 / true_bpm;
        // User taps from t = 1.0 onward, on the clicks.
        let tap_times: Vec<f64> = (0..5).map(|i| 1.0 + i as f64 * period).collect();
        let grid =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis");

        assert!(grid.confidence > 0.0);
        // Downbeat = beats[bar_phase], must land on the first
        // tap (≈ 1.0 s) within 25 ms snap tolerance.
        let db_idx = usize::from(grid.bar_phase);
        assert!(db_idx < grid.beats.len());
        let downbeat = grid.beats[db_idx];
        assert!(
            (downbeat - 1.0).abs() < 0.025,
            "downbeat must land on the user's tap; got {downbeat}, expected ≈ 1.0"
        );
        // Beats DO extend backward — pre-roll silence renders
        // as regular ticks at the algorithm's chosen phase.
        // beats[0] is the wrap-back into [0, period); for a
        // 0.5 s period that's 0.0–0.5.
        assert!(
            grid.beats[0] < period + 0.025,
            "beats[0] should be the first wrap-back position; got {}",
            grid.beats[0]
        );
        // And the array must include at least one beat before
        // the downbeat (otherwise the renderer has nothing to
        // grey-tick in the pre-roll).
        let pre_roll_beats = grid.beats.iter().filter(|&&t| t < downbeat - 1e-3).count();
        assert!(
            pre_roll_beats >= 1,
            "grid must include pre-roll beats so the renderer can show grey ticks; \
             got {pre_roll_beats} beats before the downbeat"
        );
    }

    /// Integer-BPM snap fires on the auto path too. A clean
    /// 133 BPM click track must produce **exactly** 133.0 after
    /// analysis; pre-fix the LSQ output was 133.02 (its noise
    /// floor on real ODF peaks), and 0.02 BPM accumulates to
    /// ~45 ms of visible grid drift over a 5-min track. The user
    /// saw this as "the grid hits the transient at the start of
    /// the track but is 1–2 mm off at the end."
    #[test]
    fn auto_grid_snaps_clean_integer_bpm_through_lsq() {
        let true_bpm = 133.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("auto analysis");

        assert!(grid.confidence > 0.0);
        assert!(
            (grid.bpm - true_bpm).abs() < 0.001,
            "auto path must snap LSQ result to integer BPM (got {})",
            grid.bpm
        );
    }

    /// Auto-path companion to the tap test. The track has 1.0 s
    /// of leading silence then a clean 120 BPM click pattern.
    /// PRD-BEATS round 4 follow-up: the **downbeat** (yellow
    /// marker, `beats[bar_phase]`) must land on the first
    /// audible click, while the grid extends backward through
    /// pre-roll silence so the renderer can show grey ticks
    /// before bar 1. The previous contract anchored `beats[0]`
    /// at the first audible content, which dropped every
    /// pre-roll beat and produced the "no grid before bar 1"
    /// UX the user flagged on the Oppidan track.
    #[test]
    fn auto_grid_first_beat_lands_in_audible_content_not_pre_roll() {
        let true_bpm = 120.0;
        let mut samples = vec![0.0f32; SR as usize];
        samples.extend(click_track(true_bpm, 30.0, SR));
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("auto analysis");

        assert!(grid.confidence > 0.0);
        let db_idx = usize::from(grid.bar_phase);
        assert!(db_idx < grid.beats.len());
        let downbeat = grid.beats[db_idx];
        // Downbeat must land at (or very near, allowing for the
        // amplitude-peak shift) the first audible click ~1.0 s.
        // Amplitude shift caps at AMPLITUDE_PEAK_SHIFT_WINDOW_SECS
        // so 50 ms slack covers it generously.
        assert!(
            (0.975..=1.05).contains(&downbeat),
            "auto downbeat must land on the first audible click; got {downbeat}"
        );
        // Grid must include pre-roll beats so the renderer can
        // show grey ticks before bar 1.
        let pre_roll_beats = grid.beats.iter().filter(|&&t| t < downbeat - 1e-3).count();
        assert!(
            pre_roll_beats >= 1,
            "auto grid must include pre-roll beats; got {pre_roll_beats}"
        );
    }

    /// `first_audible_secs` finds the first ODF time above the
    /// adaptive threshold. With 1 s of silence followed by a
    /// unit-step ODF, the audible start must lie at or just before
    /// the silence/signal boundary (sub-frame slack only).
    #[test]
    fn first_audible_secs_locates_silence_to_signal_edge() {
        let sr = 100.0;
        let mut odf = vec![0.0_f32; 100];
        odf.extend(std::iter::repeat_n(1.0_f32, 200));
        let t = first_audible_secs(&odf, sr).expect("audible time");
        assert!(
            (0.95..=1.05).contains(&t),
            "audible start should be ~1.0 s, got {t}"
        );
    }

    /// Pure silence (or a totally flat ODF) returns `None` so the
    /// caller falls back to the raw anchor — never inserts a spurious
    /// boundary that would drop every beat.
    #[test]
    fn first_audible_secs_returns_none_for_silence() {
        let silence = vec![0.0_f32; 1000];
        assert!(first_audible_secs(&silence, 100.0).is_none());
        assert!(first_audible_secs(&[], 100.0).is_none());
    }

    /// Integer-BPM snap: a tap median of 133.02 must quantise to
    /// 133.0 (the user's "133.0x → 133" request). 132.85 stays
    /// outside the tolerance and is returned unchanged.
    #[test]
    fn snap_to_integer_bpm_quantises_near_integer() {
        assert!((snap_to_integer_bpm(133.02, 0.10) - 133.0).abs() < 1e-12);
        assert!((snap_to_integer_bpm(132.98, 0.10) - 133.0).abs() < 1e-12);
        assert!((snap_to_integer_bpm(133.10, 0.10) - 133.0).abs() < 1e-12);
        assert!((snap_to_integer_bpm(132.85, 0.10) - 132.85).abs() < 1e-12);
        // Don't touch 0.5-increment tempos.
        assert!((snap_to_integer_bpm(128.5, 0.10) - 128.5).abs() < 1e-12);
        // Don't touch degenerate input.
        assert_eq!(snap_to_integer_bpm(0.0, 0.10), 0.0);
        assert!(snap_to_integer_bpm(f64::NAN, 0.10).is_nan());
    }

    /// Snap helper finds the ODF onset peak when the tap is in
    /// the window. The peak lands a few ms before the click body
    /// (spectral flux marks the onset, not the body) — accept up
    /// to 25 ms slack, matching the existing click-track tests.
    #[test]
    fn snap_to_nearest_transient_recovers_click_position() {
        let bpm = 120.0;
        let samples = click_track(bpm, 8.0, SR);
        let (_, odf, kick_odf) = analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            crate::BpmRange::DEFAULT,
            OctaveProfile::FourOnFloor,
        )
        .expect("odfs");
        let odf_sr = f64::from(SR) / crate::HOP_SIZE as f64;
        let kick_floor = odf_noise_floor(&kick_odf, SNAP_NOISE_FLOOR_FRAC);
        let bb_floor = odf_noise_floor(&odf, SNAP_NOISE_FLOOR_FRAC);
        let snapped =
            snap_to_nearest_transient(&kick_odf, &odf, odf_sr, 1.025, 0.070, kick_floor, bb_floor)
                .expect("snap should find peak");
        let err = (snapped - 1.0).abs();
        assert!(
            err < 0.025,
            "snapped tap must be within 25 ms of true click; got {snapped}"
        );
        let raw_tap_err = (1.025_f64 - 1.0).abs();
        assert!(
            err < raw_tap_err,
            "snap must pull closer than raw tap; got {} ms vs raw {} ms",
            err * 1000.0,
            raw_tap_err * 1000.0
        );
    }

    /// Regression for the user-reported "tap latches to transients
    /// where there are none". A tap that lands in pre-roll silence
    /// must NOT produce a snap result; the caller has to fall back
    /// to the raw tap (or skip the snap altogether). Pre-fix, any
    /// non-zero ODF sample inside the window would win and parabolic
    /// refinement happily landed on the loudest grain of noise.
    #[test]
    fn snap_to_nearest_transient_rejects_silent_window() {
        let bpm = 120.0;
        let mut samples = vec![0.0_f32; (SR as usize) * 2];
        samples.extend(click_track(bpm, 8.0, SR));
        let (_, odf, kick_odf) = analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            crate::BpmRange::DEFAULT,
            OctaveProfile::FourOnFloor,
        )
        .expect("odfs");
        let odf_sr = f64::from(SR) / crate::HOP_SIZE as f64;
        let kick_floor = odf_noise_floor(&kick_odf, SNAP_NOISE_FLOOR_FRAC);
        let bb_floor = odf_noise_floor(&odf, SNAP_NOISE_FLOOR_FRAC);
        // 0.5 s into the leading silence, no real transient inside
        // ±70 ms.
        let snapped =
            snap_to_nearest_transient(&kick_odf, &odf, odf_sr, 0.5, 0.070, kick_floor, bb_floor);
        assert!(
            snapped.is_none(),
            "snap must reject silent window; got Some({snapped:?})"
        );
    }

    /// Constrained re-analysis contract (PRD-BEATS §6.1): sloppy
    /// taps at ~133 BPM with up to ±10 ms reaction-time jitter
    /// must still resolve to exactly 133.0 because the estimator
    /// runs the full autocorrelator over the tap-median ± 15 %
    /// neighborhood and finds the strongest real periodicity in
    /// that window (the tap median is just a search hint). This
    /// is the replacement for the old "hint preserves auto BPM"
    /// test that exercised the now-removed reconciliation branch.
    #[test]
    fn tap_grid_constrained_search_locks_clean_integer_bpm_through_jitter() {
        let true_bpm = 133.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let period = 60.0 / true_bpm;
        let jitter = [0.000, 0.010, -0.008, 0.012, -0.005];
        let tap_times: Vec<f64> = jitter
            .iter()
            .enumerate()
            .map(|(i, &j)| 1.0 + i as f64 * period + j)
            .collect();
        let grid =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis");
        assert!(
            (grid.bpm - true_bpm).abs() < 1e-9,
            "constrained re-analysis must lock 133.0 through tap jitter; got {}",
            grid.bpm
        );
    }

    /// Octave-error correction (PRD-BEATS §6.1 row 3): the user
    /// taps at the real ~109 BPM but the search radius starts
    /// from the tap median, so the algorithm never even considers
    /// 218 BPM. Constrained re-analysis fixes octave errors by
    /// confining the search, not by post-hoc reconciliation.
    #[test]
    fn tap_grid_constrained_search_resolves_octave_at_tap_neighborhood() {
        let true_bpm = 109.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let period = 60.0 / true_bpm;
        // User taps at the real tempo. We do NOT pass any prior
        // octave-doubled BPM; constraint is purely from taps.
        let tap_times: Vec<f64> = (0..6).map(|i| 1.0 + i as f64 * period).collect();
        let grid =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis");
        assert!(
            (grid.bpm - true_bpm).abs() < 0.5,
            "constrained search must land at user's tapped octave; got {}",
            grid.bpm
        );
        assert!(
            grid.bpm < 1.5 * true_bpm,
            "constrained search must NOT silently double-time; got {}",
            grid.bpm
        );
    }

    /// Idempotence (PRD-BEATS §4.6): each tap session is fully
    /// independent. Running `analyze_beat_grid_from_taps` twice
    /// with identical taps must produce identical grids — no
    /// hidden previous-BPM hint state, no drift between calls.
    #[test]
    fn tap_grid_is_idempotent_across_repeated_sessions() {
        let true_bpm = 128.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let period = 60.0 / true_bpm;
        let tap_times: Vec<f64> = (0..5).map(|i| 1.0 + i as f64 * period).collect();
        let grid_a =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis A");
        let grid_b =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis B");
        assert!(
            (grid_a.bpm - grid_b.bpm).abs() < 1e-12,
            "BPM must be identical across sessions; got {} vs {}",
            grid_a.bpm,
            grid_b.bpm
        );
        assert_eq!(
            grid_a.beats.len(),
            grid_b.beats.len(),
            "beat count must match across sessions"
        );
        for (a, b) in grid_a.beats.iter().zip(grid_b.beats.iter()) {
            assert!(
                (a - b).abs() < 1e-12,
                "beat timestamps must match across sessions; got {} vs {}",
                a,
                b
            );
        }
    }

    /// An accidental double-tap inside one beat produces a sub-50-ms
    /// interval (= 1200+ BPM) that pre-fix dragged the unweighted
    /// median to absurd values. After filtering intervals to the
    /// `[MIN_BPM, MAX_BPM]` range the surviving intervals carry the
    /// real tempo.
    #[test]
    fn weighted_median_drops_double_tap_pollution() {
        let true_bpm = 130.0;
        let period = 60.0 / true_bpm;
        // Four clean taps then a double-tap inside the next beat.
        let tap_times: Vec<f64> = vec![
            1.0,
            1.0 + period,
            1.0 + 2.0 * period,
            1.0 + 3.0 * period,
            1.0 + 4.0 * period,
            1.0 + 4.0 * period + 0.040, // 40 ms after = 1500 BPM interval
        ];
        let bpm =
            weighted_median_bpm_from_taps(&tap_times).expect("median should survive double-tap");
        assert!(
            (bpm - true_bpm).abs() < 1.0,
            "double-tap interval must be dropped; got median {bpm}"
        );
    }

    /// Regression for the user-reported "set the 1 lands on a non-
    /// transient" path. The 1-tap downbeat relatch must snap to the
    /// nearest significant kick when the user's tap landed inside
    /// the snap window but a few ms late.
    #[test]
    fn latch_downbeat_snaps_to_kick_within_window() {
        let true_bpm = 120.0;
        let samples = click_track(true_bpm, 16.0, SR);
        let period = 60.0 / true_bpm;
        // True bar-1 click sits at t = period (the first beat of
        // the click_track helper). User taps 30 ms late.
        let true_downbeat = period;
        let tapped = true_downbeat + 0.030;
        let grid = latch_beat_grid_at_downbeat(
            &samples,
            SR,
            1,
            true_bpm,
            tapped,
            OctaveProfile::FourOnFloor,
        )
        .expect("relatch");
        assert!(grid.confidence > 0.0);
        let first_beat = grid.beats[0];
        let nearest_click = (first_beat / period).round() * period;
        let snapped_err = (first_beat - nearest_click).abs();
        assert!(
            snapped_err < 0.025,
            "snapped downbeat must land within 25 ms of nearest click; got {} ms",
            snapped_err * 1000.0
        );
        assert!(
            snapped_err < (tapped - true_downbeat).abs(),
            "snap must pull closer than the raw 30 ms tap offset; got {} ms",
            snapped_err * 1000.0
        );
    }

    /// 1-tap downbeat relatch into pure silence (false tap during
    /// a breakdown, or a tap in the intro of a slow-fade track)
    /// must preserve the raw tap rather than dragging it onto a
    /// sub-noise ODF wiggle. Companion to
    /// `snap_to_nearest_transient_rejects_silent_window` at the
    /// `latch_beat_grid_at_downbeat` integration level.
    #[test]
    fn latch_downbeat_preserves_raw_tap_in_silence() {
        let true_bpm = 120.0;
        let mut samples = vec![0.0_f32; (SR as usize) * 3];
        samples.extend(click_track(true_bpm, 12.0, SR));
        // Tap at 1.0 s — middle of the pre-roll silence; no real
        // transient within ±70 ms.
        let raw_downbeat = 1.0;
        let grid = latch_beat_grid_at_downbeat(
            &samples,
            SR,
            1,
            true_bpm,
            raw_downbeat,
            OctaveProfile::FourOnFloor,
        )
        .expect("relatch");
        assert!(grid.confidence > 0.0);
        // PRD-BEATS round 4 follow-up: beats now span the full
        // track, so `beats[0]` is the first wrap-back position
        // in `[0, period)`. The user's tap is the **downbeat**,
        // not necessarily `beats[0]`; the renderer reads
        // `bar_phase` to mark it yellow. Assert that the beat at
        // `bar_phase` lands within the amplitude-peak shift
        // window (30 ms) of the raw tap — the per-track shift
        // moves the entire grid by ~5–25 ms forward so it
        // renders inside the visible kick body, which applies
        // here too even though the tap itself sat in silence.
        let downbeat_idx = usize::from(grid.bar_phase);
        assert!(downbeat_idx < grid.beats.len(), "bar_phase out of range");
        let downbeat = grid.beats[downbeat_idx];
        assert!(
            (downbeat - raw_downbeat).abs() <= AMPLITUDE_PEAK_SHIFT_WINDOW_SECS + 1e-3,
            "silent-window relatch must preserve raw tap (modulo amplitude-peak shift) \
             as the downbeat; got beats[bar_phase] = {downbeat}, expected ≈ {raw_downbeat}"
        );
    }

    // ============================================================
    // PRD-BEATS round 4 follow-up — Oppidan regression triad.
    //
    // User screenshot showed three concurrent issues on a track
    // with ~0.5 s of pre-roll silence then a 4-on-the-floor
    // kick pattern at 133 BPM:
    //
    //   1. First yellow marker landed on bar 2's first kick,
    //      not bar 1's.
    //   2. No grid markers in pre-roll (the renderer had nothing
    //      to draw before bar 2).
    //   3. The grid line aligned with the rising edge of each
    //      kick (the spectral-flux ODF peak) instead of the
    //      visible amplitude peak inside the kick body.
    //
    // The three tests below pin the fix for each issue
    // independently so future regressions trip the exact one.
    // ============================================================

    /// **Issue 1 fix**: the yellow downbeat marker
    /// (`beats[bar_phase]`) must land on the FIRST kick of bar
    /// 1, not bar 2. Builds an Oppidan-shaped fixture: pre-roll
    /// silence + kick-only 4/4 pattern. The first kick is the
    /// downbeat by convention; the previous
    /// "walk anchor forward by full bars" loop overshot to
    /// bar 2 whenever bar 1's kick sat less than one bar after
    /// the silence boundary.
    #[test]
    fn auto_grid_downbeat_lands_on_first_kick_not_second_bar() {
        use crate::synthetic::click_track_with_decay;
        let true_bpm = 133.0;
        let period = 60.0 / true_bpm;
        // 0.5 s pre-roll silence + 30 s of kicks. 50 ms decay
        // mimics a kick-drum envelope better than the default
        // 5 ms click (which has its amplitude peak at the same
        // hop as the ODF peak, defeating the amplitude-shift
        // test below).
        let mut samples = vec![0.0_f32; SR as usize / 2];
        samples.extend(click_track_with_decay(true_bpm, 30.0, SR, 0.05));
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("analysis");

        assert!(grid.confidence > 0.0);
        let db_idx = usize::from(grid.bar_phase);
        let downbeat = grid.beats[db_idx];
        let first_kick_time = 0.5;
        // Downbeat must land within one period of the first kick
        // (would fail at ~one bar = 4 × period off if the
        // overshoot regression returned).
        assert!(
            (downbeat - first_kick_time).abs() < period,
            "downbeat must land on the first kick of bar 1; \
             got {downbeat} s, first kick at ~{first_kick_time} s, \
             one-bar overshoot would land at ~{} s",
            first_kick_time + 4.0 * period
        );
    }

    /// **Issue 2 fix**: the beat grid must extend backward
    /// through pre-roll silence as regular ticks. Same fixture
    /// as above; the assertion is "at least one beat sits
    /// before the downbeat" so the renderer has grey ticks to
    /// draw in the pre-roll region. The previous
    /// `beats.retain(|t| t >= anchor)` filter dropped every
    /// pre-roll beat.
    #[test]
    fn auto_grid_extends_backward_into_pre_roll_silence() {
        use crate::synthetic::click_track_with_decay;
        let true_bpm = 133.0;
        // 0.5 s pre-roll silence + 30 s of kicks → first
        // audible kick ≈ 0.5 s; a 133 BPM grid has period ≈
        // 0.451 s, so at least one beat should sit in the
        // pre-roll silence (wrap-back of the chosen downbeat
        // back into `[0, period)`).
        let mut samples = vec![0.0_f32; SR as usize / 2];
        samples.extend(click_track_with_decay(true_bpm, 30.0, SR, 0.05));
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("analysis");

        assert!(grid.confidence > 0.0);
        let downbeat = grid.beats[usize::from(grid.bar_phase)];
        let pre_roll_count = grid.beats.iter().filter(|&&t| t < downbeat - 1e-3).count();
        assert!(
            pre_roll_count >= 1,
            "grid must include at least one pre-roll beat (for grey-tick \
             rendering); got {pre_roll_count} (beats: {:?})",
            &grid.beats[..grid.beats.len().min(8)]
        );
    }

    /// **Issue 3 fix**: the rendered grid must land at the
    /// visible amplitude peak, not the spectral-flux ODF peak.
    /// `click_track_with_decay` produces an exponentially
    /// decaying kick (peak amplitude immediately at impulse
    /// time); spectral-flux peaks ~1 hop after impulse time
    /// because flux measures the *change* in magnitude. The
    /// amplitude-peak shift should pull the rendered grid back
    /// toward (or onto) the impulse position. We assert the
    /// shift produced a non-zero adjustment by comparing the
    /// median amplitude-peak offset to the un-shifted ODF-peak
    /// grid output: it must be strictly positive (the
    /// amplitude peak is at or after the snap position).
    #[test]
    fn amplitude_peak_offset_pulls_grid_toward_visible_peak() {
        use crate::synthetic::click_track_with_decay;
        let true_bpm = 120.0;
        let samples = click_track_with_decay(true_bpm, 20.0, SR, 0.05);
        // Build a uniform grid at the ODF-peak time (run the
        // auto path first, then strip the amplitude shift to
        // get the "before" grid).
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("analysis");
        assert!(grid.confidence > 0.0);

        // Re-measure the amplitude-peak offset on the final
        // grid. This must be near zero (already shifted), and
        // the offset measured on a HYPOTHETICAL grid shifted
        // back by 10 ms must be larger. That ordering only
        // holds if the shift is doing work.
        let measured = amplitude_peak_offset_secs(&grid.beats, &samples, SR, 1);
        let shifted_back: Vec<f64> = grid.beats.iter().map(|t| (t - 0.010).max(0.0)).collect();
        let shifted_back_offset = amplitude_peak_offset_secs(&shifted_back, &samples, SR, 1);
        assert!(
            shifted_back_offset > measured,
            "amplitude-peak shift must reduce the median ODF-peak-to-amp-peak \
             gap; shifted_back={shifted_back_offset} ms vs measured={measured} ms"
        );
    }

    /// `first_kick_peak_secs` finds the parabolic peak of the
    /// first kick-band ODF lobe above the noise floor.
    /// Synthetic test: silence → unit impulse on a single ODF
    /// sample at index 100, surrounded by smaller side lobes.
    /// Peak must land at exactly index 100 (or sub-sample
    /// nearby), corresponding to `t = 100 / odf_sr`.
    #[test]
    fn first_kick_peak_secs_finds_first_kick() {
        let odf_sr = 100.0_f64;
        let mut kick_odf = vec![0.0_f32; 500];
        kick_odf[99] = 0.3;
        kick_odf[100] = 1.0;
        kick_odf[101] = 0.3;
        // Decoy peak later in the track — must NOT win.
        kick_odf[300] = 0.9;
        let floor = 0.1_f32;
        let t = first_kick_peak_secs(&kick_odf, odf_sr, floor).expect("found peak");
        assert!(
            (t - 1.0).abs() < 0.01,
            "first kick at index 100 (1.0 s); got {t}"
        );
    }

    /// `first_kick_peak_secs` returns `None` when the kick
    /// band is entirely below the noise floor (instrumental
    /// piece with no percussion, dead air, etc.). The auto
    /// path falls back to `find_downbeat_offset` in that case.
    #[test]
    fn first_kick_peak_secs_returns_none_below_noise_floor() {
        let odf_sr = 100.0_f64;
        let kick_odf = vec![0.05_f32; 500];
        let floor = 0.1_f32;
        assert!(first_kick_peak_secs(&kick_odf, odf_sr, floor).is_none());
    }
}
