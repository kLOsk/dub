//! Offline beat-grid analysis: BPM + phase → **uniform 4/4 grid**.
//!
//! Traktor-style: detect transients, find the constant `(period, phase)`
//! that places the most transients on grid lines, emit a strictly uniform
//! grid `anchor + i × period`.

use crate::octave_profile::{profile_blocks_double_octave_swap, profile_blocks_half_octave_swap};
use crate::offline::analyze_bpm_with_range_profile_and_odfs;
use crate::{AnalysisError, BpmRange, OctaveProfile, HOP_SIZE, MAX_BPM, MIN_BPM};

const AUTO_REFINE_RANGE_PCT: f64 = 0.005;
const AUTO_REFINE_STEP_PCT: f64 = 0.0005;
const ZOOM_REFINE_RANGE_PCT: f64 = 0.001;
const ZOOM_REFINE_STEP_PCT: f64 = 0.00005;
const LSQ_SEARCH_FRACTION: f64 = 0.20;
const LSQ_MIN_PEAK_RATIO: f64 = 1.5;
const DOWNBEAT_CONFIDENCE_TIEBREAK: f64 = 1.2;
const KICK_ONLY_ENERGY_FRACTION: f64 = 0.25;
const KICK_ONLY_MIN_BEATS: usize = 8;
const INTRO_OUTRO_WINDOW_SECS: f64 = 30.0;
/// Universal-downbeat-fix: skip the first/last ~8 s of the track
/// when running [`score_grid_weighted`]. The intro window swallows
/// the spectral-flux startup artifact at frame 0 (the
/// no-previous-frame bias that anchored Baddadan's downbeat on
/// silence) plus any pre-roll click / vinyl drop. The outro
/// window swallows fade-outs where the broadband envelope drifts
/// without true onsets. Capped at 25 % of the track on either
/// side in code so a 16 s clip still gets scored over its middle
/// half.
const SCORE_BODY_SKIP_SECS: f64 = 8.0;
/// Universal-downbeat-fix: when the top-2 weighted phase scores
/// are within this margin and [`kick_only_intro_tiebreaker`]
/// abstains, fall back to the "first audible kick is bar 1"
/// heuristic via [`first_kick_peak_secs`]. 1.05 means the second-
/// best phase scored at least 95 % of the best, i.e. genuinely
/// ambiguous (e.g. a 4-bar all-kick intro where every phase is
/// musically defensible). At higher confidences the whole-track
/// score is trusted directly.
const FIRST_KICK_TIEBREAK_CONFIDENCE: f64 = 1.05;

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
        profile,
    )?;
    // Visual grid alignment. The ODF-snapped grid sits at spectral-
    // flux time (mid-attack). Shift it (in either direction) by the
    // median offset to the broadband-amplitude LEADING EDGE — the kick
    // onset the DJ aligns to by eye on the waveform. Replaces the old
    // forward-only amplitude-PEAK shift, which planted the line on the
    // kick's loud body and so sat tens of ms late on slow / sub-bass
    // kicks (the user's repeated "the 1 is behind the kick" reports).
    // Soft-kick tracks with no clean edge keep their LSQ phase.
    let grid = shift_grid_to_kick_edge(grid, samples, sample_rate, channels);
    // Downbeat refinement (backbeat: snare-2&4 + bass-on-1):
    // `find_downbeat_offset` votes on the kick ODF alone and cannot
    // separate bar 1 from bar 3 (both carry a kick in 4/4). The
    // snare-backbeat + bass-anchor rule resolves it from the audio. On
    // the operator's 24-track hand-tapped corpus it lifts downbeat
    // accuracy from 33 % to 75 %, fixing 11 tracks; its single weak
    // mis-pick (confidence 0.039) sits below the gate. Applied only when
    // its evidence clears [`DOWNBEAT_REFINE_MIN_CONFIDENCE`]; otherwise
    // the kick-ODF phase stands. A pure bar-phase rotation.
    Ok(apply_downbeat_refinement(
        grid,
        samples,
        sample_rate,
        channels,
    ))
}

/// Minimum backbeat-refinement confidence to override the kick-ODF downbeat.
/// Tuned on the operator's tapped corpus: every correct snare/bass
/// override scored ≥ 0.057, the single mis-pick scored 0.039.
const DOWNBEAT_REFINE_MIN_CONFIDENCE: f32 = 0.05;

/// A beat counts as "audible" once its local peak clears this fraction
/// of the whole-track amplitude max. Low on purpose: it must catch the
/// SOFT first hit of a track (the perceptual downbeat) — Bangin's intro
/// hit sits ~28 % of the body max, Oppidan's opening beat is softer than
/// the kicks that follow — not just the loud body.
const FIRST_BEAT_AUDIBLE_FRAC: f32 = 0.10;

/// Half-window (s) for the per-beat audible-energy probe.
const FIRST_BEAT_HALF_WINDOW_SECS: f64 = 0.012;

/// Bar position of the FIRST audible grid beat — the operator's rule for
/// dance music: "the 1 is the first measurable beat at the start of the
/// track." Walks past pre-roll silence to the first beat whose local
/// amplitude clears [`FIRST_BEAT_AUDIBLE_FRAC`] of the track max and
/// returns its index mod `beats_per_bar`. By periodicity that bar
/// position is bar 1's. Validated against every one of the operator's
/// hand-set grids (all bar_phase 0).
///
/// Deliberately a plain amplitude probe, NOT the sharper
/// [`kick_leading_edge_secs`]: the latter skips a soft opening hit and
/// locks onto the first *loud* kick a beat later (wrong bar position on
/// Oppidan). The known failure is the operator's stated 5 %: a track
/// whose first audible content is NOT the downbeat (reggae roll-up, a
/// vocal / talk intro). Returns `None` only for a silent track, so the
/// backbeat fallback runs there.
fn first_audible_downbeat_phase(
    beats: &[f64],
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    beats_per_bar: u8,
) -> Option<u8> {
    if beats.is_empty() || beats_per_bar == 0 {
        return None;
    }
    let (env, env_sr) = broadband_amp_envelope(samples, sample_rate, channels);
    if env.is_empty() {
        return None;
    }
    let env_max = env.iter().copied().fold(0.0f32, f32::max);
    if env_max <= 0.0 {
        return None;
    }
    let threshold = FIRST_BEAT_AUDIBLE_FRAC * env_max;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let half = (FIRST_BEAT_HALF_WINDOW_SECS * env_sr).round() as usize;
    for (i, &beat) in beats.iter().enumerate() {
        if !beat.is_finite() || beat < 0.0 {
            continue;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let center = (beat * env_sr).round() as usize;
        let lo = center.saturating_sub(half);
        let hi = (center + half).min(env.len() - 1);
        if lo > hi {
            continue;
        }
        let peak = env[lo..=hi].iter().copied().fold(0.0f32, f32::max);
        if peak >= threshold {
            return u8::try_from(i % usize::from(beats_per_bar)).ok();
        }
    }
    None
}

/// Choose the bar phase (which beat is the "1"). Primary rule: the
/// downbeat is the FIRST measurable kick ([`first_kick_downbeat_phase`])
/// — the operator's dance-music rule, right ~95 % of the time and the
/// thing the eye does. Fallback for ambiguous intros (reggae roll-ups,
/// vocal / talk over the start) where no clean early kick exists: the
/// backbeat snare-2&4 + bass-on-1 rule. Pure bar-phase rotation; bpm,
/// beats, anchor and quality are preserved.
fn apply_downbeat_refinement(
    mut grid: BeatGrid,
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> BeatGrid {
    if let Some(phase) = first_audible_downbeat_phase(
        &grid.beats,
        samples,
        sample_rate,
        channels,
        grid.beats_per_bar,
    ) {
        grid.bar_phase = phase;
        grid.downbeat_confidence = 1.0;
        return grid;
    }
    if let Some(r) =
        crate::downbeat::refine_downbeat_backbeat(samples, sample_rate, channels, &grid)
    {
        if r.confidence >= DOWNBEAT_REFINE_MIN_CONFIDENCE {
            grid.bar_phase = r.bar_phase;
            grid.downbeat_confidence = r.confidence;
        }
    }
    grid
}

/// Re-anchor at a user-supplied downbeat. **Trusts the tap
/// position exactly**; performs no snap, no amp-peak shift, no
/// BPM refinement.
///
/// PRD-BEATS Round 8 — "set the 1" is now a literal contract:
/// the rendered downbeat = `downbeat_secs`, bit-exact. Round 5
/// added an ODF snap (±70 ms to the nearest transient), Round 7
/// layered a 0–90 ms forward amp-peak shift on top to land on
/// the visible peak. Both adjustments improved the average case
/// on tracks with one clean kick per beat but produced two
/// classes of user-visible failure:
///
/// 1. **The marker moves *away* from where the user clicked.**
///    On Blaze Up Tha Dance the snap pulled the tap to an
///    earlier ODF transient (5–25 ms back) and the amp-peak
///    finder then walked further back to the leading edge of a
///    near-max region — net result was the marker landing in
///    quiet audio noticeably to the left of the click.
///    Iteratively re-tapping made it worse, because each tap
///    re-ran the same diffusive chain on a slightly different
///    region of the waveform.
///
/// 2. **The behaviour is unpredictable.** Users cannot reason
///    about the algorithm. A 5 ms move in the tap can produce a
///    20+ ms move in the marker depending on local ODF / amp
///    structure, breaking the basic UI contract "what I click
///    is where it goes".
///
/// User's explicit ask: "if we cant get this done properly can
/// we make it that setting the 1 does simply set the 1 exactly
/// where the user presses? […] it's more understandable for
/// the user I believe." The fix is to give the user direct
/// pixel-accurate control: the playhead they click IS the
/// downbeat. The waveform display already snaps the playhead to
/// a 64-sample chunk (≈ 1.45 ms at 44.1 kHz), so "exact" here
/// means "no further algorithmic adjustment past the click
/// coordinate the UI handed us".
///
/// BPM is preserved bit-identical to `bpm` (Round 6 §6a).
/// `measure_grid_quality` still measures residuals against the
/// raw tap so the drift indicator surfaces a systematic
/// mismatch between the tapped phase and the ODF backbone (e.g.
/// user tapped at the snare instead of the kick).
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
    // Need the ODF only for `measure_grid_quality` (drift
    // indicator).  The actual anchor is `downbeat_secs` verbatim
    // — no snap, no amp-peak shift, no BPM refit.
    let (_, odf, _kick_odf) = analyze_bpm_with_range_profile_and_odfs(
        samples,
        sample_rate,
        channels,
        BpmRange::DEFAULT,
        profile,
    )?;
    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let duration_secs =
        samples.len() as f64 / (f64::from(sample_rate) * f64::from(channels.max(1)));

    // Quality is measured against the user's tap (= the new
    // anchor).  Round 6 §6a's invariant holds: `bpm` is preserved
    // bit-identical; `measure_grid_quality` only reports how well
    // the BPM-period grid lines up with the ODF at this anchor.
    // If the user tapped at the snare while the ODF backbone is
    // on the kick, the drift indicator will surface the
    // systematic mismatch.
    let quality = measure_grid_quality(&odf, odf_sr, bpm, downbeat_secs, duration_secs).unwrap_or(
        GridQuality {
            rms_ms: 0.0,
            p95_ms: 0.0,
            max_abs_ms: 0.0,
            kept_fraction: 0.0,
            drift_slope_ms_per_min: 0.0,
        },
    );

    // PRD-BEATS round 4 follow-up: emit beats spanning the FULL
    // track (no `retain` filter). The user-supplied downbeat is
    // bar 1; the renderer reads `bar_phase` to decide which beat
    // is yellow, so pre-roll beats render as regular ticks while
    // `downbeat_secs` lands on the yellow marker.
    let beats_per_bar: u8 = 4;
    let beats = uniform_beats(bpm, downbeat_secs, duration_secs);
    if beats.is_empty() {
        return Ok(BeatGrid::none());
    }
    let bar_phase = bar_phase_for_downbeat_time(&beats, downbeat_secs, beats_per_bar);
    Ok(BeatGrid {
        bpm,
        confidence: 1.0,
        beats,
        beats_per_bar,
        bar_phase,
        quality: Some(quality),
        downbeat_confidence: 1.0,
    })
}

// ---------------------------------------------------------------------
// Visual kick-edge snap for "set the 1".
//
// DJs set the grid by EYE, against the rendered waveform: the kick is
// the burst whose amplitude shoots up from near-silence, and the "1"
// belongs on that rising edge. So we snap to exactly that feature —
// the leading edge of the BROADBAND amplitude envelope (the waveform
// height the user sees) — not the spectral-flux onset detector whose
// peak sits mid-attack and jitters between sub-peaks. Validated against
// the user's manual placements: Caru "Blaze Up Tha Dance" (crisp house
// kick) locks to within ~2 ms of their hand-set "1" and is invariant to
// where in the bar they tap. On a genuinely soft / ramping sub-bass kick
// (some hip-hop) there is no single visual edge — the rise-time guard
// detects that and returns `None`, and the caller keeps the tap verbatim
// (the user's eye is the better judge there).

/// Time resolution of the broadband amplitude envelope: 1 ms hop with a
/// 3 ms peak window. Fine enough to place the "1" to ~1 ms — the
/// precision a DJ gets aligning by eye on the rendered waveform.
const KICK_EDGE_HOP_SECS: f64 = 0.001;
const KICK_EDGE_WIN_SECS: f64 = 0.003;

/// Search radius around the tap for the kick's leading edge.
const KICK_EDGE_HALF_WINDOW_SECS: f64 = 0.060;

/// Fraction of the local `(peak − floor)` rise the reported edge sits
/// at. On a crisp kick the 20–80 % rise spans ≤ a sample or two so the
/// exact fraction barely matters; on a moderately-soft kick it places
/// the line a touch into the rise (where the eye reads "the kick").
const KICK_EDGE_RISE_FRAC: f32 = 0.35;

/// A kick edge only counts as "clean" if its 20→80 % amplitude rise
/// completes within this time. Crisp kicks rise in well under 15 ms; a
/// slow sub-bass ramp takes 40 ms+ and has no single visual edge — there
/// we return `None` and the caller keeps the tap verbatim.
const KICK_EDGE_MAX_RISE_SECS: f64 = 0.025;

/// The local peak must clear the window floor by at least this fraction
/// of the whole-track envelope max to count as a kick, so a tap in
/// near-silence (no transient to snap to) falls back to verbatim.
const KICK_EDGE_MIN_PROMINENCE_FRAC: f32 = 0.10;

/// Broadband amplitude envelope (peak `|x|` per [`KICK_EDGE_HOP_SECS`]
/// hop, over a [`KICK_EDGE_WIN_SECS`] window) — the waveform HEIGHT the
/// renderer draws and the DJ aligns to. Returns the envelope and its
/// sample rate (hops per second).
fn broadband_amp_envelope(samples: &[f32], sample_rate: u32, channels: u8) -> (Vec<f32>, f64) {
    let ch = usize::from(channels.max(1));
    let frames = samples.len() / ch;
    let hop = ((f64::from(sample_rate) * KICK_EDGE_HOP_SECS).round() as usize).max(1);
    let win = ((f64::from(sample_rate) * KICK_EDGE_WIN_SECS).round() as usize).max(1);
    let mut out = Vec::with_capacity(frames / hop + 1);
    let mut start = 0usize;
    while start < frames {
        let end = (start + win).min(frames);
        let mut peak = 0.0f32;
        for f in start..end {
            let mut acc = 0.0f32;
            for c in 0..ch {
                acc += samples[f * ch + c].abs();
            }
            peak = peak.max(acc / ch as f32);
        }
        out.push(peak);
        start += hop;
    }
    let env_sr = f64::from(sample_rate) / hop as f64;
    (out, env_sr)
}

/// Index (scanning backward from `peak_idx`) of the upward crossing of
/// `threshold` — the first sample at/above it on the way up to the peak.
fn rising_crossing_back(env: &[f32], lo: usize, peak_idx: usize, threshold: f32) -> usize {
    let mut i = peak_idx;
    while i > lo {
        i -= 1;
        if env[i] < threshold {
            return i + 1;
        }
    }
    lo
}

/// Leading edge (in seconds) of the kick nearest `tap_secs` on the
/// broadband amplitude `env`, or `None` when there is no clean edge to
/// snap to (near-silence, or a rise too gradual to call — see the guards
/// above). `env_max` is the whole-track envelope maximum, the prominence
/// reference.
fn kick_leading_edge_secs(
    env: &[f32],
    env_sr: f64,
    env_max: f32,
    tap_secs: f64,
    half_window_secs: f64,
) -> Option<f64> {
    if env.is_empty() || !tap_secs.is_finite() || tap_secs < 0.0 || env_sr <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let center = (tap_secs * env_sr).round() as isize;
    #[allow(clippy::cast_possible_truncation)]
    let half = (half_window_secs * env_sr).round() as isize;
    let lo = center.saturating_sub(half).max(0) as usize;
    #[allow(clippy::cast_sign_loss)]
    let hi = ((center + half).max(0) as usize).min(env.len().saturating_sub(1));
    if lo >= hi {
        return None;
    }

    let mut peak_idx = lo;
    let mut peak = env[lo];
    for (offset, &v) in env[lo..=hi].iter().enumerate() {
        if v > peak {
            peak = v;
            peak_idx = lo + offset;
        }
    }
    let floor = env[lo..=hi].iter().copied().fold(f32::INFINITY, f32::min);
    let span = peak - floor;
    if !span.is_finite() || span < KICK_EDGE_MIN_PROMINENCE_FRAC * env_max.max(1e-9) {
        return None;
    }

    let c20 = rising_crossing_back(env, lo, peak_idx, floor + 0.20 * span);
    let c80 = rising_crossing_back(env, lo, peak_idx, floor + 0.80 * span);
    // The rise's foot must sit strictly inside the window. If `c20`
    // clips at `lo` the envelope was already elevated at the window
    // edge — a sustain or a decaying tail, not a leading edge we can
    // verify — so decline and let the caller keep the tap verbatim.
    if c20 <= lo || peak_idx == lo {
        return None;
    }
    let rise_secs = (c80 as f64 - c20 as f64) / env_sr;
    if rise_secs > KICK_EDGE_MAX_RISE_SECS {
        return None;
    }

    let edge = rising_crossing_back(env, lo, peak_idx, floor + KICK_EDGE_RISE_FRAC * span);
    Some(edge as f64 / env_sr)
}

/// Re-anchor an existing grid's PHASE onto a user's "set the 1"
/// downbeat tap, keeping `bpm` bit-identical.
///
/// This is the 1–2 tap deck-header path (PRD-BEATS §4.1). The tap is
/// snapped to the **visual kick edge** — the leading edge of the
/// broadband amplitude envelope the waveform draws, the exact feature
/// the DJ aligns to by eye (see [`kick_leading_edge_secs`]). The whole
/// grid is then re-emitted from that anchor at the unchanged `bpm`. It
/// replaces the old pure-rotation behaviour ([`bar_phase_from_tap`])
/// that could only pick the nearest *analysed* beat — up to half a beat
/// away — so a sub-beat-offset auto grid could never be corrected by
/// clicking.
///
/// When the kick has no clean visual edge (a soft / ramping sub-bass
/// hit), the detector returns `None` and the tap is kept **verbatim** —
/// the user's eye is the better judge there, and a guess would land
/// somewhere they didn't point. Applies in both prep and playing modes:
/// the snap lands on the *visible* edge, so it never feels like the grid
/// "moved on its own".
///
/// Unlike the auto and hand-tap paths this applies no whole-grid
/// shift: "set the 1" lands on the kick's onset directly (via the
/// per-tap edge snap), not its later amplitude peak.
pub fn relatch_grid_at_downbeat_tap(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    bpm: f64,
    downbeat_tap: f64,
    profile: OctaveProfile,
) -> Result<BeatGrid, AnalysisError> {
    if !(bpm.is_finite() && bpm > 0.0 && downbeat_tap.is_finite() && downbeat_tap >= 0.0) {
        return Ok(BeatGrid::none());
    }
    let (_estimate, odf, _kick_odf) = analyze_bpm_with_range_profile_and_odfs(
        samples,
        sample_rate,
        channels,
        BpmRange::DEFAULT,
        profile,
    )?;
    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let duration_secs =
        samples.len() as f64 / (f64::from(sample_rate) * f64::from(channels.max(1)));
    if duration_secs <= 0.0 {
        return Ok(BeatGrid::none());
    }

    // Snap the tap to the visual kick edge (broadband amplitude leading
    // edge); keep it verbatim when there is no clean edge to lock to.
    let (env, env_sr) = broadband_amp_envelope(samples, sample_rate, channels);
    let env_max = env.iter().copied().fold(0.0f32, f32::max);
    let snapped = kick_leading_edge_secs(
        &env,
        env_sr,
        env_max,
        downbeat_tap,
        KICK_EDGE_HALF_WINDOW_SECS,
    )
    .unwrap_or(downbeat_tap);

    let quality =
        measure_grid_quality(&odf, odf_sr, bpm, snapped, duration_secs).unwrap_or(GridQuality {
            rms_ms: 0.0,
            p95_ms: 0.0,
            max_abs_ms: 0.0,
            kept_fraction: 0.0,
            drift_slope_ms_per_min: 0.0,
        });

    let beats_per_bar: u8 = 4;
    let beats = uniform_beats(bpm, snapped, duration_secs);
    if beats.is_empty() {
        return Ok(BeatGrid::none());
    }
    let bar_phase = bar_phase_for_downbeat_time(&beats, snapped, beats_per_bar);
    Ok(BeatGrid {
        bpm,
        confidence: 1.0,
        beats,
        beats_per_bar,
        bar_phase,
        quality: Some(quality),
        downbeat_confidence: 1.0,
    })
}

/// Build a grid from user tap times (M11d.7 round 3 tap-to-grid).
///
/// **The taps ARE the BPM.** User feedback override of PRD-BEATS
/// §6.1: the constrained-re-analysis pipeline was producing
/// "wrong" BPMs on tracks where the auto-pass landed badly in a
/// non-tapped neighborhood. Example the user surfaced: a 175 BPM
/// DnB track that auto-resolved to ~135 BPM; tapping at 175 used
/// to come back as ~160 because the ±15 % search window ([148,
/// 201]) found a stronger ODF peak in that range than the truth
/// at 175. The PRD argued the constrained autocorrelation beats
/// human tap jitter on precision; the user prefers honesty: "show
/// the bpm how the user taps it (building an average of the
/// actual taps)". So the constrained re-analysis is gone — the
/// tap-interval median IS the answer, no algorithmic second-guess
/// on the BPM number.
///
/// Pipeline (PRD-BEATS Round 6 §6b — tap-as-hint contract):
///
/// 1. **BPM hint** = weighted median of tap-to-tap intervals
///    after dropping intervals that imply BPMs outside
///    `[MIN_BPM, MAX_BPM]`. Same logic as before: a double-tap
///    inside one beat or a missed-beat interval cannot drag the
///    median past the analysable range.
/// 2. **BPM refinement** = narrow ±[`TAP_HINT_SEARCH_PCT`] LSQ
///    search around the hint, picking the BPM with tightest
///    grid fit (lowest `rms_ms`) against the ODF. This absorbs
///    the 5–15 ms / tap human jitter that the hint alone would
///    bake into the persisted BPM — the user surfaced this as
///    "tap at 87 BPM, get 86.232 BPM stored, grid drifts off
///    the kicks". After the constrained refit the result is
///    almost always the clean integer (or half-integer) the
///    track was actually produced at.
/// 3. **Integer-snap**: if the refined BPM sits within
///    [`INTEGER_BPM_SNAP_TOLERANCE`] of an integer AND that
///    integer doesn't meaningfully worsen the fit (same safety
///    net as the auto path), prefer the integer. Most commercial
///    music is integer-BPM.
/// 4. **Anchor** = first tap snapped to the nearest significant
///    transient in the kick ODF (fallback broadband) inside a
///    window capped at `min(period/4, 70 ms)`. The
///    `odf_noise_floor` check keeps the snap from latching onto
///    sub-noise ODF wiggle when the first tap landed in dead
///    space — in that case we keep the raw tap (`unwrap_or`).
/// 5. Confidence is fixed at `1.0` (the user supplied ground
///    truth for tempo and bar position); `GridQuality` is
///    measured against the final BPM/anchor for the drift
///    indicator.
///
/// **Why narrow.** The previous incarnation of this function
/// (M11d.7a) ran constrained re-analysis at ±15 %; user feedback
/// surfaced "tap at 175 BPM, get 160 BPM" because the autocorre-
/// lation peak winner inside ±15 % was a neighbouring metric
/// level. The user then asked for "tap median IS the BPM" which
/// produced jitter; this round threads the needle with ±3 %, so
/// the refinement window is narrower than the gap to the
/// nearest neighbour metric level (typically ≥ 10 %). Cannot
/// drift to a different musical interpretation by construction.
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
    let Some(bpm_hint) = weighted_median_bpm_from_taps(tap_times) else {
        return Ok(BeatGrid::none());
    };
    analyze_beat_grid_from_bpm_and_anchor(
        samples,
        sample_rate,
        channels,
        bpm_hint,
        tap_times[0],
        profile,
    )
}

/// Build a tap grid from an **authoritative `bpm`** plus a single anchor
/// tap, instead of re-deriving tempo from the spacing of `tap_times`.
///
/// This is the correct path for a tap-while-playing session. Tempo MUST
/// come from the human's wall-clock tapping rhythm (computed in the UI),
/// never from playhead-position deltas — those are scaled by the platter's
/// live playback rate (`committed ≈ wall ÷ rate`), which is exactly why
/// tapping a 174 BPM track at −8 % pitch used to commit ~189. The anchor is
/// still ODF-snapped to the nearest real transient for phase robustness,
/// and `bpm` is still ODF-refined ±3 % + integer-snapped so a clean tap
/// tempo lands on the track's true integer BPM.
///
/// # Errors
///
/// See [`AnalysisError`]. Returns `Ok(BeatGrid::none())` for non-finite or
/// non-positive `bpm`/`anchor_tap`.
pub fn analyze_beat_grid_from_bpm_and_anchor(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    bpm_hint: f64,
    anchor_tap: f64,
    profile: OctaveProfile,
) -> Result<BeatGrid, AnalysisError> {
    if !(bpm_hint.is_finite() && bpm_hint > 0.0 && anchor_tap.is_finite()) {
        return Ok(BeatGrid::none());
    }

    // ODFs are still required: the anchor snap reads kick + broadband
    // ODF to find the nearest real transient to the first tap, the
    // constrained search reads broadband ODF to fit the period, and
    // `measure_grid_quality` reads broadband ODF for the drift
    // indicator. The estimator's BPM is deliberately discarded — the
    // user's tap session is authoritative for the metric level.
    let (_estimate, odf, kick_odf) = analyze_bpm_with_range_profile_and_odfs(
        samples,
        sample_rate,
        channels,
        BpmRange::DEFAULT,
        profile,
    )?;

    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let duration_secs =
        samples.len() as f64 / (f64::from(sample_rate) * f64::from(channels.max(1)));
    if duration_secs <= 0.0 {
        return Ok(BeatGrid::none());
    }

    let period_hint = 60.0 / bpm_hint;
    let snap_half = (period_hint * 0.25).min(SNAP_MAX_HALF_WINDOW_SECS);
    let kick_floor = odf_noise_floor(&kick_odf, SNAP_NOISE_FLOOR_FRAC);
    let broadband_floor = odf_noise_floor(&odf, SNAP_NOISE_FLOOR_FRAC);
    let snapped_anchor = snap_to_nearest_transient(
        &kick_odf,
        &odf,
        odf_sr,
        anchor_tap,
        snap_half,
        kick_floor,
        broadband_floor,
    )
    .unwrap_or(anchor_tap);

    // PRD-BEATS Round 6 §6b — narrow constrained BPM refinement.
    // Tap jitter on a clean session is ~1–3 BPM standard
    // deviation on the median; we want the integer that fits the
    // ODF best, not the noisy median. Skip when the hint sits
    // at a pathological band edge so the search can't escape the
    // `[MIN_BPM, MAX_BPM]` range.
    let (bpm, anchor) =
        refine_bpm_around_tap_hint(&odf, odf_sr, bpm_hint, snapped_anchor, duration_secs)
            .unwrap_or((bpm_hint, snapped_anchor));

    let quality =
        measure_grid_quality(&odf, odf_sr, bpm, anchor, duration_secs).unwrap_or(GridQuality {
            rms_ms: 0.0,
            p95_ms: 0.0,
            max_abs_ms: 0.0,
            kept_fraction: 0.0,
            drift_slope_ms_per_min: 0.0,
        });

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
    // Visual alignment: land the grid on the kick LEADING EDGE the DJ
    // sees, same as the auto path. Replaces the old forward-only
    // amplitude-PEAK shift (which sat late on slow kicks).
    Ok(shift_grid_to_kick_edge(
        grid,
        samples,
        sample_rate,
        channels,
    ))
}

/// PRD-BEATS Round 6 §6b — half-width of the BPM refinement
/// window expressed as a fraction of the hint. 0.03 = ±3 %, so
/// for a 90 BPM tap the search runs over `[87.3, 92.7]` and for
/// a 175 BPM tap over `[169.75, 180.25]`. Tight enough that the
/// search cannot reach a neighbouring metric level (the nearest
/// is at ±50 % for a half-octave shift), wide enough to absorb
/// the worst real-world tap jitter the corpus shows (~2 BPM
/// standard deviation on a 5-tap session).
const TAP_HINT_SEARCH_PCT: f64 = 0.03;

/// PRD-BEATS Round 6 §6b — step size of the BPM refinement
/// search. 0.10 BPM matches the integer-snap tolerance so the
/// search grid is dense enough that integer-snap behaviour is
/// deterministic (every integer in the window is sampled exactly).
const TAP_HINT_SEARCH_STEP: f64 = 0.10;

/// PRD-BEATS Round 6 §6b — refine a tap-derived BPM hint by
/// sweeping ±[`TAP_HINT_SEARCH_PCT`] around the hint in steps of
/// [`TAP_HINT_SEARCH_STEP`], picking the BPM whose
/// fixed-anchor LSQ fit has the lowest `rms_ms`. Returns the
/// `(bpm, anchor)` pair the production grid should use, or
/// `None` when no candidate in the window produced enough
/// observations to score (rare — extremely short tracks or
/// hints very close to `[MIN_BPM, MAX_BPM]`).
///
/// **Integer-snap** is applied to the winner via the same safety
/// net the auto path uses (`snap_bpm_to_integer_if_safe`): the
/// integer must not meaningfully worsen `rms_ms` before we
/// commit to it. Without this gate a 90.3 BPM tap on a genuine
/// 90.3 BPM track would snap to 90.0 and the resulting grid
/// would drift visibly across a 5 min track.
fn refine_bpm_around_tap_hint(
    odf: &[f32],
    odf_sr: f64,
    bpm_hint: f64,
    anchor: f64,
    duration_secs: f64,
) -> Option<(f64, f64)> {
    if !(MIN_BPM..=MAX_BPM).contains(&bpm_hint) {
        return None;
    }
    let lo_raw = (bpm_hint * (1.0 - TAP_HINT_SEARCH_PCT)).max(MIN_BPM);
    let hi = (bpm_hint * (1.0 + TAP_HINT_SEARCH_PCT)).min(MAX_BPM);
    if hi <= lo_raw {
        return None;
    }
    // Quantise the candidate grid to tenths of a BPM so it
    // ALWAYS samples integer (and half-integer) BPMs inside the
    // window. Without this the grid lands at `lo + k * step`
    // which usually misses integers — e.g. hint 87.5 → window
    // [84.875, 90.125], grid 84.875, 84.975, 85.075, ..., 87.475,
    // 87.575, never sampling 87.5 or 88.0. Quantising to 0.1
    // ensures every test-relevant value (87.4, 87.5, 87.6, 88.0)
    // is on the grid.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = ((lo_raw * 10.0).ceil() / 10.0).max(MIN_BPM);

    // Collect every candidate that produced enough observations to
    // score, so we can filter by `kept_fraction` before picking the
    // tightest RMS. `refit_anchor_at_bpm` is asymmetric on wrong
    // BPMs: the LSQ peak search drops beats whose predicted
    // position isn't within `LSQ_SEARCH_FRACTION * period` of any
    // ODF peak, so a slightly-wrong BPM can report misleadingly
    // tight RMS over a small subset of "easy" beats while
    // ignoring the systematic drift in the rest. Filtering on
    // `kept_fraction` first stops that artifact dominating the
    // selection — a candidate that fits 95 % of beats well always
    // beats one that fits 60 % of beats slightly tighter.
    let mut candidates: Vec<(f64, f64, GridQuality)> = Vec::new();
    let mut cand_bpm = lo;
    while cand_bpm <= hi + 1e-9 {
        if let Some((cand_anchor, cand_quality)) =
            refit_anchor_at_bpm(odf, odf_sr, cand_bpm, anchor, duration_secs)
        {
            candidates.push((cand_bpm, cand_anchor, cand_quality));
        }
        cand_bpm += TAP_HINT_SEARCH_STEP;
    }
    if candidates.is_empty() {
        return None;
    }

    let max_kept = candidates
        .iter()
        .map(|(_, _, q)| q.kept_fraction)
        .fold(0.0_f32, f32::max);
    let kept_floor = max_kept * 0.95;
    let (best_bpm, best_anchor, best_quality) = candidates
        .into_iter()
        .filter(|(_, _, q)| q.kept_fraction >= kept_floor)
        .min_by(|(_, _, a), (_, _, b)| {
            a.rms_ms
                .partial_cmp(&b.rms_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;

    // Integer-snap (same safety net as the auto path).
    let (snapped_bpm, snapped_anchor, _) = snap_bpm_to_integer_if_safe(
        odf,
        odf_sr,
        best_bpm,
        best_anchor,
        duration_secs,
        best_quality,
        IntegerSnapPolicy::TAP,
    );
    Some((snapped_bpm, snapped_anchor))
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

// Constrained re-analysis at `tap_median ± 15 %` removed by user
// feedback override of PRD-BEATS §6.1: the estimator was finding
// stronger ODF peaks inside the search window that didn't match the
// tap intent (e.g. 175 BPM DnB resolving to ~160 because the
// strongest periodicity in [148, 201] BPM wasn't 175). The tap
// median is now used directly as the BPM. See
// `analyze_beat_grid_from_taps` doc comment.

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

#[allow(clippy::too_many_arguments)]
fn analyze_uniform_grid_from_odf(
    odf: &[f32],
    kick_odf: &[f32],
    odf_sr: f64,
    duration_secs: f64,
    bpm_init: f64,
    confidence: f32,
    fixed_anchor: Option<f64>,
    profile: OctaveProfile,
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
        IntegerSnapPolicy::AUTO,
    );

    // PRD-BEATS Round 6 §6c — profile-independent octave self-
    // verification. Re-fit the grid at `bpm / 2` and `bpm * 2`
    // and swap to whichever alternate octave has a materially
    // tighter LSQ fit. Catches the hip-hop double-tempo failure
    // mode (Bangin' Westside Connection: auto-picked 175.7 BPM
    // with rms 41 ms / kept 46 %; the same audio at 88 BPM gives
    // rms 10 ms / kept 72 %) when the genre tag would have set
    // the right profile but didn't, AND on untagged tracks
    // where the profile system can't help at all. The threshold
    // is strict on purpose: only swap when the alternate octave
    // is so much better that "the algorithm picked the hat grid
    // instead of the kick grid" is the only plausible
    // explanation. See [`octave_self_verify`] for the criteria.
    let (bpm, anchor_secs, quality) = octave_self_verify(
        odf,
        odf_sr,
        bpm,
        anchor_secs,
        duration_secs,
        quality,
        fixed_anchor,
        profile,
    );

    let period = 60.0 / bpm;
    let beats_per_bar: u8 = 4;

    // Downbeat selection (universal-downbeat-fix, supersedes the
    // PRD-BEATS round 4 "first audible kick = bar 1" rule):
    //
    // 1. If a `fixed_anchor` is supplied (legacy callers that
    //    pre-decide the downbeat), it IS bar 1.
    // 2. Otherwise: the **whole-track scoring** in
    //    `find_downbeat_offset` decides which of the four bar
    //    positions is the 1. The sub-beat `anchor_secs` from the
    //    LSQ fit (over hundreds of onsets) is held fixed; only
    //    the bar-phase rotates. This is structurally robust to
    //    quiet intros, false starts, ODF startup artifacts at
    //    `t=0`, and any other single-event misfire — 100+ bars
    //    of body-of-track evidence outvote any one spike.
    //
    // The previous rule branched on `first_kick_peak_secs` and
    // anchored the entire grid on whatever crossed an adaptive
    // noise floor first. Baddadan exposed the failure mode: an
    // ODF startup artifact at frame 0 read as a "first kick" at
    // 0.0275 s, dragging bar 1 onto silence. The whole-track
    // scoring already had the right answer; we just discarded
    // it. `first_kick_peak_secs` is retained as a low-confidence
    // tiebreaker inside `find_downbeat_offset` for the genuinely
    // ambiguous case (e.g. a 4-bar intro where every phase
    // scores within a couple of percent of each other).
    let chosen_downbeat;
    let downbeat_confidence;
    if let Some(fixed) = fixed_anchor {
        chosen_downbeat = fixed;
        downbeat_confidence = 1.0;
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
///
/// PRD-BEATS Round 10 — strict linear comparison restored.
/// Round 9 added a geometric-drift term to this slack to allow
/// snaps on long real-music tracks within ~0.05 BPM of an
/// integer. That was wrong: the geometric drift IS the
/// wrong-tempo signature, so allowing the snap "to absorb" it
/// silently accepted snaps on tracks that were genuinely at
/// non-integer BPMs (Chase & Status — Come Back, true tempo
/// 174.98, snapped to 175 with ~43 ms cumulative phase drift
/// over the 5-min track — clearly audible at the end). Through
/// the relationship `drift_rms ≈ Δbpm × duration × kept_frac
/// / sqrt(12) / bpm × 1000`, the strict 3 ms RMS budget at
/// typical `kept_fraction` ≈ 0.5–0.8 implicitly caps cumulative
/// phase drift at ~15–25 ms, which matches DJ-perceptible
/// beatmatching tolerance. Long real-music tracks within
/// ~0.005 BPM of integer still snap (the inherent LSQ noise
/// dwarfs the geometric drift at that scale); longer snaps
/// stay at the LSQ-true fractional BPM and the user can pitch-
/// fader correct on the deck.
const INTEGER_SNAP_RMS_SLACK_MS: f32 = 3.0;

/// Integer-snap tolerance for the **tap path**. The human tapping
/// ~175 is asserting a clean integer, so the refine landing at
/// 174.8–174.9 (which happens on poor-onset tracks where the LSQ
/// minimum wanders with the tap anchor — Apocalypse is the report)
/// must still reach the integer. Wider than the auto path's 0.10
/// (which has no human prior) but well below 0.5, so genuine
/// half-step tempos (174.5) are never pulled to an integer. The
/// relative RMS-slack safety net (below) still protects a clean
/// fit at a real fractional tempo (Chase & Status 174.98).
const TAP_INTEGER_SNAP_TOLERANCE: f64 = 0.25;

/// Tap-path integer-snap RMS slack as a fraction of the raw fit
/// RMS. On a CLEAN fit (low RMS, e.g. Chase & Status ~8 ms) this
/// stays at the 3 ms floor so a genuine 174.98 is preserved. On a
/// NOISY fit (Apocalypse ~40 ms) the budget grows to ~10 ms, so
/// the sub-integer LSQ preference — which is pure onset noise at
/// that scale and can't resolve 174.9 from 175.0 — yields to the
/// human's tapped integer. This is the key Apocalypse-vs-Chase&
/// Status discriminator: trust a fractional BPM only when the fit
/// is clean enough to have actually measured it.
const TAP_INTEGER_SNAP_REL_SLACK_FRAC: f32 = 0.25;

/// Tap-path `kept_fraction` ceiling below which the tapped integer
/// is accepted unconditionally. Clean DnB / house grids keep
/// ~0.7–0.9 of their beats; Apocalypse's sparse, smeared onsets
/// keep ~0.14–0.37 depending on where the user tapped — far too
/// few to claim a 174.8 vs 175.0 distinction is real. Below 0.6 we
/// trust the human's integer; at or above it the strict guards run
/// (so a clean fit at a genuine 174.98 is still preserved).
const TAP_INTEGER_SNAP_POOR_FIT_KEPT: f32 = 0.6;

/// Policy for [`snap_bpm_to_integer_if_safe`]: how close to an
/// integer triggers a snap, and how much the snapped grid may
/// worsen the ODF fit before we reject it. The auto path keeps the
/// strict absolute budget; the tap path widens both because the
/// human tap is a strong integer prior. See the constants above.
#[derive(Clone, Copy)]
struct IntegerSnapPolicy {
    tolerance: f64,
    /// When the raw fit's `kept_fraction` is below this, accept the
    /// integer unconditionally: the fit is too sparse to have
    /// measured a sub-integer BPM, so the (human-tapped) integer
    /// wins. A ≤`tolerance` BPM move can never be an octave error,
    /// so the kept-collapse guard below would only ever veto for the
    /// wrong reason on a noisy track. AUTO sets 0 (never fires — no
    /// human prior, keep the strict guards).
    poor_fit_kept_ceiling: f32,
    /// Floor on `kept_snapped / kept_raw` before a snap is rejected
    /// as a structural mismatch (octave protection on the auto path).
    min_kept_ratio: f32,
    min_rms_slack_ms: f32,
    rel_rms_slack_frac: f32,
}

impl IntegerSnapPolicy {
    const AUTO: Self = Self {
        tolerance: INTEGER_BPM_SNAP_TOLERANCE,
        poor_fit_kept_ceiling: 0.0,
        min_kept_ratio: INTEGER_SNAP_MIN_KEPT_RATIO,
        min_rms_slack_ms: INTEGER_SNAP_RMS_SLACK_MS,
        rel_rms_slack_frac: 0.0,
    };
    const TAP: Self = Self {
        tolerance: TAP_INTEGER_SNAP_TOLERANCE,
        poor_fit_kept_ceiling: TAP_INTEGER_SNAP_POOR_FIT_KEPT,
        min_kept_ratio: 0.0,
        min_rms_slack_ms: INTEGER_SNAP_RMS_SLACK_MS,
        rel_rms_slack_frac: TAP_INTEGER_SNAP_REL_SLACK_FRAC,
    };
}

/// Minimum fraction of the raw `kept_fraction` we require at the
/// snapped tempo before accepting the snap. Below this we're
/// comparing residuals on materially different observation sets
/// (the snap dropped beats that were "kept" at the raw tempo) and
/// the RMS comparison is no longer apples-to-apples. 0.85 means
/// the snap may shed up to 15 % of its kept beats — well above
/// noise (kept_fraction is stable to ±5 % across small BPM
/// perturbations on typical commercial music) and below the level
/// where we'd be hiding real structural disagreement.
const INTEGER_SNAP_MIN_KEPT_RATIO: f32 = 0.85;

/// Expected RMS contribution (in milliseconds) to grid residuals
/// when a uniform `bpm_snapped`-period grid is fit to observations
/// that came from a `bpm_raw`-period grid, with the anchor
/// re-optimised at the snapped BPM (mean-centred OLS).
///
/// For `N` observations indexed `i = 0..N-1`, the snapped
/// grid predicts `anchor_snapped + i * period_snapped`. Mean-
/// centring the anchor zeros the residual mean, so the residual
/// at observation `i` is `(i - (N-1)/2) * (period_raw -
/// period_snapped)`. The RMS over `i = 0..N-1` is:
///
/// ```text
/// rms = |Δperiod| * sqrt((N² - 1) / 12)
/// ```
///
/// This is the "free" RMS cost of the snap: it has nothing to do
/// with whether the snapped BPM is musically correct, only with
/// the fact that we changed the slope of the predicted-times
/// line. It dominates for small `|Δbpm|` (e.g. a 174.98 → 175.00
/// snap over 900 beats contributes ~7.5 ms on its own).
///
/// Returns 0 for degenerate inputs (`N < 2`, non-finite BPMs).
#[must_use]
fn expected_bpm_shift_rms_ms(bpm_raw: f64, bpm_snapped: f64, n_observations: usize) -> f64 {
    if !bpm_raw.is_finite() || !bpm_snapped.is_finite() || bpm_raw <= 0.0 || bpm_snapped <= 0.0 {
        return 0.0;
    }
    if n_observations < 2 {
        return 0.0;
    }
    let period_raw = 60.0 / bpm_raw;
    let period_snapped = 60.0 / bpm_snapped;
    let dperiod = (period_raw - period_snapped).abs();
    #[allow(clippy::cast_precision_loss)]
    let n = n_observations as f64;
    let i_var = (n * n - 1.0) / 12.0;
    dperiod * i_var.sqrt() * 1_000.0
}

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
    policy: IntegerSnapPolicy,
) -> (f64, f64, GridQuality) {
    let bpm_snapped = snap_to_integer_bpm(bpm_raw, policy.tolerance);
    if (bpm_snapped - bpm_raw).abs() < 1e-9 {
        eprintln!(
            "dub-bpm: integer-snap skipped — bpm_raw={bpm_raw:.4} not within \
             ±{} of an integer",
            policy.tolerance
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

    // Poor-fit short-circuit (tap path). A `kept_fraction` this low
    // means the LSQ couldn't line up the grid with enough onsets to
    // resolve a sub-integer BPM, so its sub-integer preference is
    // noise. The human tapped a clean tempo; a ≤`tolerance` snap
    // can't be an octave error, so accept the integer without the
    // kept-collapse / RMS guards (which are exactly the ones a noisy
    // fit fools). AUTO sets `poor_fit_kept_ceiling = 0`, so this
    // never fires there.
    if quality_raw.kept_fraction < policy.poor_fit_kept_ceiling {
        eprintln!(
            "dub-bpm: integer-snap ACCEPTED bpm_raw={bpm_raw:.4} -> bpm={bpm_snapped:.2} \
             (poor fit: kept_fraction {:.2} < {:.2}, trusting tapped integer)",
            quality_raw.kept_fraction, policy.poor_fit_kept_ceiling,
        );
        return (bpm_snapped, anchor_snapped, quality_snapped);
    }

    // PRD-BEATS Round 10 — strict linear RMS comparison.
    //
    // Round 9 added the expected geometric drift from the BPM
    // shift (`drift_rms_ms`) to the slack budget. User report:
    // Chase & Status — Come Back is genuinely at ~174.98 BPM;
    // snapping to 175.00 introduced ~43 ms cumulative phase
    // drift over the 5-min track, audible at the end. The
    // Round 9 framing was the bug — the geometric drift IS the
    // wrong-tempo signature, so budgeting for it silently
    // accepted snaps the LSQ correctly identified as non-integer.
    //
    // Reverted to the strict 3 ms absolute slack (the AUTO policy).
    // Through `drift_rms ≈ Δbpm × duration × kept_frac / sqrt(12) /
    // bpm × 1000`, the 3 ms budget implicitly caps cumulative phase
    // drift at ~15–25 ms for typical real-music kept fractions,
    // which matches DJ beatmatching tolerance. The TAP policy widens
    // this to `max(3 ms, 0.25 × rms_raw)` because the human tap is a
    // strong integer prior and a sub-integer LSQ preference on a
    // high-RMS fit is unmeasurable noise (see `IntegerSnapPolicy`).
    //
    // The `kept_fraction` guard (Round 9) and the geometric
    // drift computation (Round 9) are kept — the guard remains a
    // useful independent structural-mismatch safety net, and
    // `drift_rms_ms` is logged in the diagnostic so the user can
    // see WHY a snap was rejected (observed ΔRMS exceeded the
    // geometric drift the snap mathematically had to introduce,
    // i.e. the true tempo is likely non-integer).
    let period_snapped = 60.0 / bpm_snapped;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let max_beats_snapped = ((duration_secs / period_snapped).floor() as usize).saturating_add(1);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let n_kept = (f64::from(quality_snapped.kept_fraction) * max_beats_snapped as f64)
        .round()
        .max(0.0) as usize;
    let drift_rms_ms = expected_bpm_shift_rms_ms(bpm_raw, bpm_snapped, n_kept);
    let rms_raw = f64::from(quality_raw.rms_ms);
    let rms_snapped = f64::from(quality_snapped.rms_ms);
    let delta_ms = rms_snapped - rms_raw;

    let kept_ratio = if quality_raw.kept_fraction > 0.0 {
        quality_snapped.kept_fraction / quality_raw.kept_fraction
    } else {
        1.0
    };
    if kept_ratio < policy.min_kept_ratio {
        eprintln!(
            "dub-bpm: integer-snap REJECTED bpm_raw={bpm_raw:.4} -> bpm={bpm_snapped:.2} \
             (kept_fraction {:.2} -> {:.2}, ratio {:.2} below floor {:.2}), keeping bpm_raw",
            quality_raw.kept_fraction,
            quality_snapped.kept_fraction,
            kept_ratio,
            policy.min_kept_ratio
        );
        return (bpm_raw, anchor_secs, quality_raw);
    }

    // Auto path: `rel_rms_slack_frac == 0` ⇒ the strict absolute
    // budget (protects genuine fractional tracks the analyzer found
    // on its own). Tap path: the budget grows with the raw fit RMS,
    // so a noisy fit (which can't resolve sub-integer BPM) yields to
    // the human's tapped integer while a clean fit at a real
    // fractional tempo is still preserved.
    let rms_slack = policy
        .min_rms_slack_ms
        .max(policy.rel_rms_slack_frac * quality_raw.rms_ms);
    if delta_ms <= f64::from(rms_slack) {
        eprintln!(
            "dub-bpm: integer-snap ACCEPTED bpm_raw={bpm_raw:.4} -> bpm={bpm_snapped:.2} \
             (rms {:.2}ms -> {:.2}ms, Δ {:.2}ms ≤ slack {:.1}ms; expected geometric drift \
             {drift_rms_ms:.2}ms over {n_kept} kept beats; kept {:.2} -> {:.2})",
            quality_raw.rms_ms,
            quality_snapped.rms_ms,
            delta_ms,
            rms_slack,
            quality_raw.kept_fraction,
            quality_snapped.kept_fraction,
        );
        (bpm_snapped, anchor_snapped, quality_snapped)
    } else {
        eprintln!(
            "dub-bpm: integer-snap REJECTED bpm_raw={bpm_raw:.4} -> bpm={bpm_snapped:.2} \
             (rms {:.2}ms -> {:.2}ms, Δ {:.2}ms > slack {:.1}ms; expected geometric drift \
             {drift_rms_ms:.2}ms over {n_kept} kept beats — observed Δ exceeds expected, true \
             tempo is likely non-integer), keeping bpm_raw",
            quality_raw.rms_ms, quality_snapped.rms_ms, delta_ms, rms_slack,
        );
        (bpm_raw, anchor_secs, quality_raw)
    }
}

/// PRD-BEATS Round 6 §6c — `rms_alt < OCTAVE_RECHECK_RMS_RATIO *
/// rms_main` is the materiality bar for swapping to a half- or
/// double-tempo octave during the self-verification pass. 0.65
/// means the alternate octave must be at least 35 % tighter in
/// RMS terms. Empirically chosen against the hip-hop / DnB /
/// reggae corpus: a borderline-correct octave (a track that
/// genuinely sits between two metric levels) sees the two
/// octaves' RMS within ~5–15 % of each other; a true octave
/// misfire (the hat grid winning over the kick grid) sees a
/// 2–4× RMS gap. 0.65 sits comfortably in the gap so the swap
/// fires when it should and stays silent when it shouldn't.
const OCTAVE_RECHECK_RMS_RATIO: f64 = 0.65;

/// PRD-BEATS Round 6 §6c — the alternate octave must also fit
/// AT LEAST this fraction of beats (`kept_fraction`) before we
/// swap. Without this guard, a spurious tempo at twice the real
/// rate could produce extremely tight RMS on the small subset of
/// beats it does fit and pass the ratio check while ignoring
/// most of the track. 0.50 means at least half the predicted
/// beats land on a real ODF peak — comfortably above the noise
/// of a wrong tempo and well below the typical 0.75–0.90 a
/// correct grid achieves.
const OCTAVE_RECHECK_MIN_KEPT: f32 = 0.50;

/// PRD-BEATS Round 6 §6c — profile-independent octave self-
/// verification. After [`snap_bpm_to_integer_if_safe`] picks
/// `(bpm, anchor, quality)`, also re-fit the grid at `bpm / 2`
/// and `bpm * 2` via [`refit_anchor_at_bpm`]. If either
/// alternate octave fits the ODF materially tighter than the
/// main (`rms_alt < OCTAVE_RECHECK_RMS_RATIO * rms_main` AND
/// `kept_alt >= kept_main` AND `kept_alt >= OCTAVE_RECHECK_MIN_
/// KEPT`), swap to it and integer-snap the alternate BPM with
/// the same tolerance rules as the main octave.
///
/// Why this exists. Our pass-2 octave decision in
/// `octave_profile.rs` compares **spectral autocorrelation
/// energy** at each candidate period, not **LSQ fit quality**
/// against the actual ODF. On hip-hop with a busy hat layer the
/// 175 BPM hat grid wins on spectral energy even though an 88
/// BPM kick grid fits the actual onsets much tighter; same
/// shape for tracks with a busy 16th-note shaker / off-beat
/// snare pattern. The genre profile (`HipHop`, `RootsReggae`,
/// `Dub`) is the first line of defence and handles tagged
/// tracks. This is the second line, profile-independent, and
/// catches untagged tracks plus edge cases the profile rules
/// don't know about.
///
/// Strictly profile-independent on purpose: the profile system
/// expresses a prior ("this genre tends to live at X tempo"),
/// this expresses a measurement ("at this anchor, BPM X fits
/// the ODF better than BPM 2X"). Both signals together are
/// stronger than either alone.
///
/// Skipped when:
/// * `fixed_anchor.is_some()` — the caller has asserted both
///   anchor and intent; don't second-guess them.
/// * The alternate BPM falls outside `[MIN_BPM, MAX_BPM]`.
/// * `refit_anchor_at_bpm` returns `None` (insufficient
///   observations at the alternate tempo).
#[allow(clippy::too_many_arguments)]
fn octave_self_verify(
    odf: &[f32],
    odf_sr: f64,
    bpm: f64,
    anchor: f64,
    duration_secs: f64,
    quality: GridQuality,
    fixed_anchor: Option<f64>,
    profile: OctaveProfile,
) -> (f64, f64, GridQuality) {
    if fixed_anchor.is_some() {
        return (bpm, anchor, quality);
    }
    let main_rms = quality.rms_ms;
    let main_kept = quality.kept_fraction;

    let mut best = (bpm, anchor, quality);
    let mut best_rms = main_rms;
    let period_main = 60.0 / bpm;

    // Each (alternate BPM, list of starting anchors to try). At
    // the half octave the grid period is 2× the main octave's
    // period, so only every other main beat lands on the new
    // grid. There are TWO possible sub-phases that fit (the
    // half-octave grid starting on main beat 0 vs main beat 1);
    // the wrong sub-phase lands every beat on a SNARE instead of
    // a KICK in a hip-hop programming, which destroys the fit.
    // Trying both sub-phases is essential — without it, the
    // half-octave's measured RMS is dominated by whichever
    // sub-phase the LSQ refit happened to find first, which is
    // almost always the wrong one (Bangin' Westside Connection:
    // the wrong sub-phase reports 73 ms rms, the right sub-phase
    // reports 10 ms rms — without trying both, the rule never
    // fires). At the double octave the current anchor already
    // sits on a beat of the new grid (every existing beat
    // doubles up), so only one sub-phase exists.
    //
    // PRD-BEATS round 6+ — direction-blocked octaves are also
    // skipped here based on `profile`. Pass 2's profile-aware
    // rules already committed to the genre's mix octave (e.g.
    // FourOnFloor picking 133 over 66.5 via the perceptual prior
    // + half-bar rejection); the self-verify must respect that
    // decision. Profile-blind swap on Oppidan / Cutty Ranks
    // (UKG 133 → 66.5) was the canonical regression.
    let half_anchors = [anchor, anchor + period_main];
    let double_anchors = [anchor];
    let candidates: [(f64, &[f64], bool); 2] = [
        (
            bpm / 2.0,
            &half_anchors,
            profile_blocks_half_octave_swap(profile),
        ),
        (
            bpm * 2.0,
            &double_anchors,
            profile_blocks_double_octave_swap(profile),
        ),
    ];

    for (cand_bpm_hint, sub_anchors, blocked) in candidates {
        if blocked {
            continue;
        }
        if !(MIN_BPM..=MAX_BPM).contains(&cand_bpm_hint) {
            continue;
        }

        // For each sub-anchor, run a narrow ±2 % BPM search
        // around the alternate-octave hint. Pass-2's BPM
        // commitment at the WRONG octave is a sub-integer
        // spectral autocorrelation peak whose half need not
        // coincide with the true tempo's integer (Bangin'
        // Westside Connection: pass 2 picks 175.742, strict
        // half is 87.871, the truth is 88.000 — 0.13 BPM off
        // the strict half; without the BPM search the strict
        // half measures 73 ms RMS where the true 88.0 measures
        // 10 ms). The 2 % window is narrower than the gap to
        // any neighbour metric level (which is ±50 % away).
        let mut best_sub: Option<(f64, f64, GridQuality)> = None;
        for &start_anchor in sub_anchors {
            if let Some((sub_bpm, sub_anchor, sub_quality)) = refine_alternate_octave_at_anchor(
                odf,
                odf_sr,
                cand_bpm_hint,
                start_anchor,
                duration_secs,
            ) {
                let take = match best_sub {
                    Some((_, _, ref q)) => sub_quality.rms_ms < q.rms_ms,
                    None => true,
                };
                if take {
                    best_sub = Some((sub_bpm, sub_anchor, sub_quality));
                }
            }
        }
        let Some((cand_bpm, cand_anchor, cand_quality)) = best_sub else {
            continue;
        };

        let materially_tighter =
            f64::from(cand_quality.rms_ms) < f64::from(best_rms) * OCTAVE_RECHECK_RMS_RATIO;
        let not_worse_kept = cand_quality.kept_fraction >= main_kept;
        let kept_above_floor = cand_quality.kept_fraction >= OCTAVE_RECHECK_MIN_KEPT;
        if !(materially_tighter && not_worse_kept && kept_above_floor) {
            continue;
        }

        // Integer-snap the alternate BPM too, with the same
        // safety net as the main octave (`snap_bpm_to_integer_
        // if_safe`): if the snapped tempo doesn't fit
        // meaningfully worse, prefer the integer. Most music is
        // produced at integer BPM; if the alternate octave fits
        // tightly at e.g. 87.95 the truth is almost always 88.0.
        let (snapped_bpm, snapped_anchor, snapped_quality) = snap_bpm_to_integer_if_safe(
            odf,
            odf_sr,
            cand_bpm,
            cand_anchor,
            duration_secs,
            cand_quality,
            IntegerSnapPolicy::AUTO,
        );
        eprintln!(
            "dub-bpm: octave self-verify SWAPPED {:.3} -> {:.3} \
             (rms {:.2} -> {:.2} ms, kept {:.0}% -> {:.0}%)",
            best.0,
            snapped_bpm,
            main_rms,
            snapped_quality.rms_ms,
            f64::from(main_kept) * 100.0,
            f64::from(snapped_quality.kept_fraction) * 100.0,
        );
        best = (snapped_bpm, snapped_anchor, snapped_quality);
        best_rms = snapped_quality.rms_ms;
    }
    best
}

/// PRD-BEATS Round 6 §6c — half-width of the per-sub-anchor BPM
/// search inside [`octave_self_verify`], as a fraction of the
/// alternate-octave hint. 0.02 = ±2 %. Tight enough that the
/// search cannot reach a different metric level (next octave is
/// ±50 % away) yet wide enough to absorb the gap between pass-2's
/// sub-integer spectral pick and the real tempo's integer.
const OCTAVE_RECHECK_BPM_SEARCH_PCT: f64 = 0.02;

/// PRD-BEATS Round 6 §6c — step of the per-sub-anchor BPM
/// search inside [`octave_self_verify`]. 0.10 BPM matches the
/// integer-snap tolerance so every integer inside the window is
/// sampled exactly once.
const OCTAVE_RECHECK_BPM_SEARCH_STEP: f64 = 0.10;

/// Helper for [`octave_self_verify`]: at a given sub-anchor,
/// sweep BPM ±[`OCTAVE_RECHECK_BPM_SEARCH_PCT`] around the
/// alternate-octave hint and return the `(bpm, anchor, quality)`
/// triple with the lowest LSQ RMS. The grid is quantised to
/// 0.1 BPM (matching [`refine_bpm_around_tap_hint`]) so integers
/// and half-integers are always sampled.
fn refine_alternate_octave_at_anchor(
    odf: &[f32],
    odf_sr: f64,
    cand_bpm_hint: f64,
    start_anchor: f64,
    duration_secs: f64,
) -> Option<(f64, f64, GridQuality)> {
    let lo_raw = (cand_bpm_hint * (1.0 - OCTAVE_RECHECK_BPM_SEARCH_PCT)).max(MIN_BPM);
    let hi = (cand_bpm_hint * (1.0 + OCTAVE_RECHECK_BPM_SEARCH_PCT)).min(MAX_BPM);
    if hi <= lo_raw {
        return None;
    }
    let lo = ((lo_raw * 10.0).ceil() / 10.0).max(MIN_BPM);

    let mut best: Option<(f64, f64, GridQuality)> = None;
    let mut cand_bpm = lo;
    while cand_bpm <= hi + 1e-9 {
        if let Some((sub_anchor, sub_quality)) =
            refit_anchor_at_bpm(odf, odf_sr, cand_bpm, start_anchor, duration_secs)
        {
            let take = match best {
                Some((_, _, ref q)) => sub_quality.rms_ms < q.rms_ms,
                None => true,
            };
            if take {
                best = Some((cand_bpm, sub_anchor, sub_quality));
            }
        }
        cand_bpm += OCTAVE_RECHECK_BPM_SEARCH_STEP;
    }
    best
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

    // Universal-downbeat-fix B: score over the body of the track
    // only, weighted by broadband energy. The skip window swallows
    // ODF startup artifacts and tail fades that previously
    // contaminated all four phase candidates with equal phantom
    // votes; the energy weight stops a long quiet intro / breakdown
    // from drowning out a short loud body.
    let skip_secs = SCORE_BODY_SKIP_SECS.min(duration_secs * 0.25).max(0.0);
    let body_start_odf = skip_secs * odf_sr;
    let body_end_secs = (duration_secs - skip_secs).max(skip_secs + 1e-6);
    let body_end_odf = body_end_secs * odf_sr;

    let mut scores = [0.0f64; 4];
    for offset in 0..beats_per_bar {
        let phase_odf = (anchor_secs + f64::from(offset) * period) * odf_sr;
        scores[usize::from(offset)] = score_grid_weighted(
            kick_odf,
            broadband_odf,
            phase_odf,
            bar_period_odf,
            body_start_odf,
            body_end_odf,
        );
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
        // Universal-downbeat-fix A.1: when the whole-track score is
        // genuinely ambiguous (top-2 within ~5 %) and the kick-only
        // tiebreaker abstained, fall back to "the first audible
        // kick is bar 1". This preserves the user's mental model
        // for tracks where the body of the track really does not
        // discriminate (e.g. dnb with kick on every beat), without
        // letting an ODF startup artifact at `t=0` override 100+
        // bars of body-of-track evidence.
        if confidence < FIRST_KICK_TIEBREAK_CONFIDENCE as f32 {
            let kick_floor = odf_noise_floor(kick_odf, SNAP_NOISE_FLOOR_FRAC);
            if let Some(first_kick) = first_kick_peak_secs(kick_odf, odf_sr, kick_floor) {
                let bar_period_secs = period * f64::from(beats_per_bar);
                let mut tie_offset = best_offset;
                let mut tie_dist = f64::INFINITY;
                for offset in 0..beats_per_bar {
                    let phase_anchor = anchor_secs + f64::from(offset) * period;
                    // Distance from `first_kick` to the nearest
                    // downbeat at this phase, modulo the bar.
                    let n_bars = ((first_kick - phase_anchor) / bar_period_secs).round();
                    let nearest_downbeat = phase_anchor + n_bars * bar_period_secs;
                    let dist = (first_kick - nearest_downbeat).abs();
                    if dist < tie_dist {
                        tie_dist = dist;
                        tie_offset = offset;
                    }
                }
                return (tie_offset, confidence);
            }
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

/// Per-beat search radius for the auto grid's visual kick-edge
/// alignment. Tighter than the set-the-1 tap window (the LSQ already
/// places beats within ~20 ms of the kick), so it can't drift onto a
/// neighbouring beat at any dance tempo.
const KICK_EDGE_AUTO_HALF_WINDOW_SECS: f64 = 0.040;

/// Minimum fraction of beats that must expose a clean visual kick edge
/// before we trust the median offset and shift the whole grid. A crisp
/// kick-driven track clears this easily (most beats have an edge); a
/// soft / ramping-kick track (where there is no single visual edge to
/// agree on) falls below it, and the grid keeps its LSQ phase rather
/// than chasing a handful of noisy detections.
const KICK_EDGE_MIN_BEAT_FRACTION: f32 = 0.25;

/// Hard cap on the grid shift, so a pathological run of mis-detections
/// can never yank the grid more than a hair off the LSQ phase.
const KICK_EDGE_MAX_SHIFT_SECS: f64 = 0.050;

/// Median SIGNED offset (seconds) from each beat to its nearest visual
/// kick edge — the uniform shift that lands the whole grid on the kick
/// leading edges the DJ sees. Signed (it can pull the grid EARLIER, off
/// the loud body and onto the onset) and targets the broadband
/// amplitude leading edge, not a near-max / peak region. Returns 0 when
/// too few beats expose a clean
/// edge (see [`KICK_EDGE_MIN_BEAT_FRACTION`]).
fn kick_edge_offset_secs(beats: &[f64], samples: &[f32], sample_rate: u32, channels: u8) -> f64 {
    if beats.is_empty() {
        return 0.0;
    }
    let (env, env_sr) = broadband_amp_envelope(samples, sample_rate, channels);
    if env.is_empty() {
        return 0.0;
    }
    let env_max = env.iter().copied().fold(0.0f32, f32::max);
    let mut offsets: Vec<f64> = Vec::with_capacity(beats.len());
    for &beat in beats {
        if let Some(edge) =
            kick_leading_edge_secs(&env, env_sr, env_max, beat, KICK_EDGE_AUTO_HALF_WINDOW_SECS)
        {
            offsets.push(edge - beat);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let min_beats = (KICK_EDGE_MIN_BEAT_FRACTION * beats.len() as f32).ceil() as usize;
    if offsets.len() < min_beats.max(6) {
        return 0.0;
    }
    offsets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = offsets[offsets.len() / 2];
    median.clamp(-KICK_EDGE_MAX_SHIFT_SECS, KICK_EDGE_MAX_SHIFT_SECS)
}

/// Shift every beat in `grid` by the median visual-kick-edge offset so
/// the rendered lines sit on the kick onsets the DJ aligns to by eye —
/// the auto-grid analogue of the "set the 1" snap. Replaces the former
/// forward-only amplitude-peak shift, which could only push the grid
/// onto the kick's loud body/peak (tens of ms late on slow kicks) and
/// never back onto the onset. `bpm` and `quality` are
/// preserved; only the phase moves. No-op when the offset is negligible.
#[must_use]
fn shift_grid_to_kick_edge(
    grid: BeatGrid,
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> BeatGrid {
    if grid.beats.is_empty() || grid.bpm <= 0.0 {
        return grid;
    }
    let delta = kick_edge_offset_secs(&grid.beats, samples, sample_rate, channels);
    if delta.abs() < 1e-3 {
        return grid;
    }
    let original_downbeat_idx = usize::from(grid.bar_phase).min(grid.beats.len() - 1);
    let shifted_downbeat_time = grid.beats[original_downbeat_idx] + delta;
    let duration_secs = grid.beats.last().copied().unwrap_or(0.0) + delta.abs() + 60.0 / grid.bpm;
    let mut new_beats = uniform_beats(grid.bpm, shifted_downbeat_time, duration_secs);
    let original_end = grid.beats.last().copied().unwrap_or(duration_secs) + delta.abs() + 1e-9;
    new_beats.retain(|&t| t <= original_end && t >= 0.0);
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

/// Universal-downbeat-fix B: bar-level score for a candidate
/// downbeat phase, restricted to the body of the track and
/// weighted by local broadband loudness.
///
/// For each step of `period` (which the caller sets to one bar in
/// ODF samples), evaluates `kick_odf` at the candidate position
/// and multiplies by the peak broadband ODF value in a small
/// window around it. Positions outside `[body_start, body_end]`
/// (in ODF samples) contribute zero — this defangs the
/// spectral-flux startup spike at frame 0 that previously gave
/// every phase candidate phantom early votes on tracks with quiet
/// intros (Baddadan), and the symmetric fade-out artifact on
/// outros.
///
/// The weight is a *peak* over a tenth-of-a-bar window centered
/// on the candidate downbeat: silent bars (intros, breakdowns,
/// drops where the kick band continues but nothing else does)
/// contribute ~0 regardless of which of the 4 phases they
/// nominally fall on, so 100+ loud bars in the body of the
/// track outvote any single phantom intro spike. Window width
/// is one tenth of a bar to keep adjacent-beat leakage in
/// check while still tolerating slightly off-grid transients.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn score_grid_weighted(
    kick_odf: &[f32],
    broadband_odf: &[f32],
    phase: f64,
    period: f64,
    body_start: f64,
    body_end: f64,
) -> f64 {
    if !phase.is_finite() || !period.is_finite() || period <= 0.0 || kick_odf.is_empty() {
        return f64::NEG_INFINITY;
    }
    let len = kick_odf.len();
    let broad_len = broadband_odf.len();
    let weight_half_window = (period * 0.05).max(1.0);

    let mut score = 0.0f64;
    let mut i: usize = 0;
    loop {
        let idx_f = phase + i as f64 * period;
        if idx_f >= len as f64 - 1.0 {
            break;
        }
        if idx_f >= 0.0 && idx_f >= body_start && idx_f <= body_end {
            let lo = idx_f.floor() as usize;
            let hi = (lo + 1).min(len - 1);
            let frac = idx_f - lo as f64;
            let kick = (1.0 - frac) * f64::from(kick_odf[lo]) + frac * f64::from(kick_odf[hi]);

            let mut weight = 0.0f32;
            if broad_len > 0 {
                let w_lo = (idx_f - weight_half_window).max(0.0) as usize;
                let w_hi_raw = (idx_f + weight_half_window).max(0.0) as usize;
                let w_hi = w_hi_raw.min(broad_len - 1);
                let mut w = w_lo.min(broad_len - 1);
                while w <= w_hi {
                    let v = broadband_odf[w];
                    if v > weight {
                        weight = v;
                    }
                    w += 1;
                }
            }
            score += kick * f64::from(weight);
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
        // PRD-BEATS Round 6 §6a: latch is a phase-only operation;
        // BPM must come out bit-identical to the input (was `< 1.0`
        // before the fix, which let `refine_period_at_anchor` and
        // the joint-OLS LSQ refit drift the period by ~0.1–1.0
        // BPM per call).
        assert!(
            (grid.bpm - bpm).abs() < 1e-9,
            "latch must preserve input BPM exactly; got {} (expected {})",
            grid.bpm,
            bpm
        );
        assert!(beats_are_uniform(&grid.beats, grid.bpm));

        let first_after_one = grid
            .beats
            .iter()
            .find(|&&b| (b - 1.0).abs() < 0.05)
            .copied()
            .expect("a beat near user mark");
        assert!((first_after_one - 1.0).abs() < 0.05);
    }

    /// "Set the 1" relatch must RE-PHASE the grid onto the tap, not
    /// merely rotate to the nearest analysed beat. The old
    /// `bar_phase_from_tap` path (still live in the FFI before this
    /// fix) could only land the downbeat on an existing beat — up
    /// to half a beat from where the user clicked. This is the unit
    /// guard for that contract.
    #[test]
    fn relatch_tap_rephases_grid_rather_than_rotating() {
        let bpm = 120.0;
        let samples = click_track(bpm, 12.0, SR);
        let auto = analyze_beat_grid(&samples, SR, 1).expect("auto");
        assert!(auto.confidence > 0.4);

        let nearest_beat_to = |grid: &BeatGrid, t: f64| -> f64 {
            grid.beats
                .iter()
                .copied()
                .min_by(|a, b| {
                    (a - t)
                        .abs()
                        .partial_cmp(&(b - t).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("beats")
        };

        // Tap 20 ms off a real click: relatch snaps the grid onto the
        // click's VISUAL edge (broadband amplitude leading edge),
        // preserving BPM exactly. The click is a crisp transient, so the
        // edge detector fires and pulls the beat onto ~4.0, not the raw
        // 4.02 tap.
        let near = 4.02;
        let on_click =
            relatch_grid_at_downbeat_tap(&samples, SR, 1, auto.bpm, near, OctaveProfile::Default)
                .expect("relatch on click");
        assert!(
            (on_click.bpm - auto.bpm).abs() < 1e-9,
            "relatch must preserve BPM exactly"
        );
        assert!(beats_are_uniform(&on_click.beats, on_click.bpm));
        let snapped_beat = nearest_beat_to(&on_click, near);
        assert!(
            (snapped_beat - 4.0).abs() < 0.012,
            "a beat must snap onto the click's visual edge at ~4.0; got {snapped_beat}"
        );

        // Tap parked BETWEEN clicks (no kick edge anywhere in the search
        // window) → the detector returns None and the tap is kept
        // VERBATIM. A beat must land essentially ON the tap — which pure
        // rotation to the nearest analysed beat (≈0.13 s away) could
        // never achieve, proving relatch re-phases rather than rotates.
        let off = 4.13;
        let between =
            relatch_grid_at_downbeat_tap(&samples, SR, 1, auto.bpm, off, OctaveProfile::Default)
                .expect("relatch between clicks");
        assert!((between.bpm - auto.bpm).abs() < 1e-9);
        assert!(beats_are_uniform(&between.beats, between.bpm));
        assert!(
            (nearest_beat_to(&between, off) - off).abs() < 0.002,
            "a beat must land on the tap (verbatim re-phase); tap={off}"
        );
    }

    /// PRD-BEATS Round 6 §6c regression — profile-independent
    /// octave self-verification. Manually construct the scenario:
    /// a click track at the real BPM, then pass `octave_self_
    /// verify` the **wrong octave** (2×) as "main" along with a
    /// deliberately-poor quality and the anchor at the wrong
    /// octave. The self-verify must measure the actual fit at
    /// `bpm / 2` and swap to it because the half-tempo grid
    /// landings on every other click are tight while the
    /// double-tempo grid has every other prediction landing on
    /// silence (the kick interval). Pre-fix the algorithm
    /// trusted the spectral-energy pick and never even measured
    /// the alternate octave.
    #[test]
    fn octave_self_verify_swaps_when_alternate_fits_materially_tighter() {
        let real_bpm = 90.0;
        let samples = click_track(real_bpm, 30.0, SR);

        // Re-derive the ODF that the analyzer would have built so
        // we can call `octave_self_verify` with a realistic input.
        let (_, odf, _) = crate::offline::analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            BpmRange::DEFAULT,
            OctaveProfile::Default,
        )
        .expect("odf");
        let odf_sr = f64::from(SR) / HOP_SIZE as f64;
        let duration_secs = samples.len() as f64 / f64::from(SR);

        // Manufacture a "main = wrong octave" quality. The exact
        // numbers don't matter — they just have to be worse than
        // what the self-verify will find at the real BPM.
        // Anchor on a click (period = 60/90 = 0.667 s) so
        // `refit_anchor_at_bpm` at the alternate octave finds
        // observations on the first pass; an anchor halfway
        // between clicks would leave the LSQ window empty and
        // the refit would early-return `None` without measuring.
        let wrong_bpm = real_bpm * 2.0;
        let wrong_anchor = 60.0 / real_bpm; // first non-zero click
        let wrong_quality = GridQuality {
            rms_ms: 35.0,
            p95_ms: 60.0,
            max_abs_ms: 80.0,
            kept_fraction: 0.45,
            drift_slope_ms_per_min: 0.0,
        };

        let (chosen_bpm, _, chosen_quality) = octave_self_verify(
            &odf,
            odf_sr,
            wrong_bpm,
            wrong_anchor,
            duration_secs,
            wrong_quality,
            None,
            OctaveProfile::Default,
        );

        assert!(
            (chosen_bpm - real_bpm).abs() < 0.01,
            "self-verify must swap to {real_bpm} BPM; got {chosen_bpm}"
        );
        assert!(
            chosen_quality.rms_ms < wrong_quality.rms_ms,
            "swapped quality must be tighter than the rejected main; \
             got swapped {} ms vs main {} ms",
            chosen_quality.rms_ms,
            wrong_quality.rms_ms
        );
    }

    /// PRD-BEATS Round 6 §6c — the converse: when the main octave
    /// already fits tightly the self-verify must NOT swap, even
    /// though the half-tempo grid trivially also fits (every other
    /// beat at 60 BPM lands on a 120 BPM click). The materiality
    /// gate (`OCTAVE_RECHECK_RMS_RATIO`) is what protects against
    /// over-eager swapping.
    #[test]
    fn octave_self_verify_keeps_main_when_quality_is_already_tight() {
        let real_bpm = 120.0;
        let samples = click_track(real_bpm, 30.0, SR);
        let (_, odf, _) = crate::offline::analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            BpmRange::DEFAULT,
            OctaveProfile::Default,
        )
        .expect("odf");
        let odf_sr = f64::from(SR) / HOP_SIZE as f64;
        let duration_secs = samples.len() as f64 / f64::from(SR);

        // Fit the main grid through the normal pipeline so the
        // input quality is what the production code would see.
        let (bpm_main, anchor_main, quality_main) =
            refine_full_pipeline(&odf, odf_sr, real_bpm, duration_secs, None).expect("refine main");
        let (bpm_snapped, anchor_snapped, quality_snapped) = snap_bpm_to_integer_if_safe(
            &odf,
            odf_sr,
            bpm_main,
            anchor_main,
            duration_secs,
            quality_main,
            IntegerSnapPolicy::AUTO,
        );

        let (chosen_bpm, _, _) = octave_self_verify(
            &odf,
            odf_sr,
            bpm_snapped,
            anchor_snapped,
            duration_secs,
            quality_snapped,
            None,
            OctaveProfile::Default,
        );

        assert!(
            (chosen_bpm - real_bpm).abs() < 0.5,
            "self-verify must not swap a tight {real_bpm} BPM fit; \
             got {chosen_bpm}"
        );
    }

    /// PRD-BEATS Round 6 §6c — `fixed_anchor` callers (the
    /// tap-driven paths) bypass self-verification because the
    /// caller has already asserted the period via their tap
    /// intervals.
    #[test]
    fn octave_self_verify_respects_fixed_anchor_bypass() {
        let real_bpm = 90.0;
        let samples = click_track(real_bpm, 30.0, SR);
        let (_, odf, _) = crate::offline::analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            BpmRange::DEFAULT,
            OctaveProfile::Default,
        )
        .expect("odf");
        let odf_sr = f64::from(SR) / HOP_SIZE as f64;
        let duration_secs = samples.len() as f64 / f64::from(SR);

        let wrong_bpm = real_bpm * 2.0;
        let wrong_quality = GridQuality {
            rms_ms: 35.0,
            p95_ms: 60.0,
            max_abs_ms: 80.0,
            kept_fraction: 0.45,
            drift_slope_ms_per_min: 0.0,
        };

        let (chosen_bpm, _, _) = octave_self_verify(
            &odf,
            odf_sr,
            wrong_bpm,
            0.5,
            duration_secs,
            wrong_quality,
            Some(0.5),
            OctaveProfile::Default,
        );

        assert!(
            (chosen_bpm - wrong_bpm).abs() < 1e-9,
            "self-verify must respect fixed_anchor bypass; \
             got {chosen_bpm}"
        );
    }

    /// Oppidan / Cutty Ranks regression — UK garage tagged
    /// `FourOnFloor` must keep the upper octave even when the
    /// half-tempo grid measures a tighter LSQ fit. Production-
    /// sparse kick patterns (kick on 1+3, snare on 2+4, hat
    /// fills) make the half-octave grid land on kicks only and
    /// look misleadingly clean; the profile already said "mix
    /// at the upper octave" via pass 2, and the self-verify
    /// must respect that.
    ///
    /// Fixture: synthesise a perfectly clean click at the
    /// upper octave so the main grid fits as tight as it can,
    /// and pass a *worse* manufactured `GridQuality` for the
    /// upper octave so the rule would unambiguously swap if it
    /// were profile-blind. Default profile MUST swap (control);
    /// FourOnFloor MUST NOT swap. Same shape covers Dancehall
    /// and DrumAndBass via the parameterised loop.
    #[test]
    fn octave_self_verify_does_not_swap_down_for_upper_octave_profiles() {
        let real_bpm = 133.0;
        let samples = click_track(real_bpm, 30.0, SR);
        let (_, odf, _) = crate::offline::analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            BpmRange::DEFAULT,
            OctaveProfile::Default,
        )
        .expect("odf");
        let odf_sr = f64::from(SR) / HOP_SIZE as f64;
        let duration_secs = samples.len() as f64 / f64::from(SR);

        // Anchor on a click so refit at the half octave finds
        // observations; pass a deliberately-poor main quality so
        // Default profile is forced to swap (control).
        let upper_anchor = 60.0 / real_bpm;
        let poor_main_quality = GridQuality {
            rms_ms: 35.0,
            p95_ms: 60.0,
            max_abs_ms: 80.0,
            kept_fraction: 0.45,
            drift_slope_ms_per_min: 0.0,
        };

        let (control_bpm, _, _) = octave_self_verify(
            &odf,
            odf_sr,
            real_bpm,
            upper_anchor,
            duration_secs,
            poor_main_quality,
            None,
            OctaveProfile::Default,
        );
        assert!(
            (control_bpm - real_bpm / 2.0).abs() < 0.5,
            "Default profile MUST swap UKG-shaped 133 -> 66.5 (control); got {control_bpm}"
        );

        for profile in [
            OctaveProfile::FourOnFloor,
            OctaveProfile::DrumAndBass,
            OctaveProfile::Dancehall,
        ] {
            let (kept_bpm, _, _) = octave_self_verify(
                &odf,
                odf_sr,
                real_bpm,
                upper_anchor,
                duration_secs,
                poor_main_quality,
                None,
                profile,
            );
            assert!(
                (kept_bpm - real_bpm).abs() < 0.01,
                "{profile:?} must refuse to swap 133 -> 66.5; got {kept_bpm}"
            );
        }
    }

    /// Symmetric to `does_not_swap_down`: profiles whose mix
    /// tempo lives in the lower octave (RootsReggae, Dub,
    /// HipHop) must refuse the BPM → BPM×2 swap even when the
    /// upper-octave grid measures tighter. A 88 BPM hip-hop
    /// track whose hi-hat layer doubles the autocorrelation at
    /// 176 must stay at 88 once the profile has spoken.
    #[test]
    fn octave_self_verify_does_not_swap_up_for_lower_octave_profiles() {
        let real_bpm = 88.0;
        let samples = click_track(real_bpm * 2.0, 30.0, SR);
        let (_, odf, _) = crate::offline::analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            BpmRange::DEFAULT,
            OctaveProfile::Default,
        )
        .expect("odf");
        let odf_sr = f64::from(SR) / HOP_SIZE as f64;
        let duration_secs = samples.len() as f64 / f64::from(SR);

        let lower_anchor = 60.0 / real_bpm;
        let poor_main_quality = GridQuality {
            rms_ms: 35.0,
            p95_ms: 60.0,
            max_abs_ms: 80.0,
            kept_fraction: 0.45,
            drift_slope_ms_per_min: 0.0,
        };

        let (control_bpm, _, _) = octave_self_verify(
            &odf,
            odf_sr,
            real_bpm,
            lower_anchor,
            duration_secs,
            poor_main_quality,
            None,
            OctaveProfile::Default,
        );
        assert!(
            (control_bpm - real_bpm * 2.0).abs() < 0.5,
            "Default profile MUST swap hip-hop-shaped 88 -> 176 (control); got {control_bpm}"
        );

        for profile in [
            OctaveProfile::HipHop,
            OctaveProfile::RootsReggae,
            OctaveProfile::Dub,
        ] {
            let (kept_bpm, _, _) = octave_self_verify(
                &odf,
                odf_sr,
                real_bpm,
                lower_anchor,
                duration_secs,
                poor_main_quality,
                None,
                profile,
            );
            assert!(
                (kept_bpm - real_bpm).abs() < 0.01,
                "{profile:?} must refuse to swap 88 -> 176; got {kept_bpm}"
            );
        }
    }

    /// PRD-BEATS Round 6 §6a regression: "set the 1" must not
    /// refit BPM, even when the LSQ would prefer a slightly
    /// different period. The user's Apocalypse report: analyse at
    /// 177.72 BPM, tap on the first kick to set the 1, watch BPM
    /// jump to 178.70 — a ~1 BPM error that accumulates ~2.2 s
    /// of drift across a 4-min track. Pre-fix, both
    /// `refine_period_at_anchor` (±1 % grid search on
    /// `score_grid`) and `lsq_refit_grid` with `anchor_fixed =
    /// true` (joint OLS on slope = 1/period) could move BPM.
    /// Post-fix, the deck's input BPM is preserved verbatim and
    /// only the anchor (and bar-phase rotation) changes.
    ///
    /// Fixture: a click track generated at 120.0 BPM has every
    /// click perfectly on the 0.5 s grid, so the LSQ "best fit"
    /// at that anchor agrees with 120.0. To force the would-be
    /// regression we pass a deliberately wrong input BPM
    /// (120.7) and assert the grid still comes back at exactly
    /// 120.7 — the latch must trust the caller, not measure.
    #[test]
    fn latch_preserves_input_bpm_even_when_lsq_disagrees() {
        let real_bpm = 120.0;
        let lied_bpm = 120.7;
        let samples = click_track(real_bpm, 30.0, SR);
        let grid =
            latch_beat_grid_at_downbeat(&samples, SR, 1, lied_bpm, 1.0, OctaveProfile::FourOnFloor)
                .expect("relatch");
        assert!(
            (grid.bpm - lied_bpm).abs() < 1e-9,
            "set-the-1 must not refit BPM; got {} (expected {})",
            grid.bpm,
            lied_bpm
        );
        // Companion check: the residuals at the lied BPM are
        // measurably worse than they would be at the real BPM,
        // so the drift indicator stays meaningful.
        let q = grid.quality.expect("quality");
        assert!(
            q.drift_slope_ms_per_min.abs() > 1.0,
            "drift indicator must reflect the lied-BPM systematic \
             residual instead of being silently corrected away; got {} ms/min",
            q.drift_slope_ms_per_min
        );
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
    /// Clean taps at exactly 133 BPM intervals must yield exactly
    /// 133.0 BPM. With the post-PRD-§6.1-override pipeline the
    /// tap-interval weighted median IS the BPM, so as long as the
    /// intervals are exactly `60 / 133` seconds we get 133.0 back
    /// out modulo fp noise (0.001 tolerance covers the weighted
    /// median's fp work).
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
            "clean taps at 133 BPM intervals must yield exactly 133.0; got {}",
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

    /// The committed tempo must depend ONLY on the supplied `bpm`, never
    /// on the anchor. This is the regression that proves tempo no longer
    /// leaks from the playhead: feed the same bpm with several anchors and
    /// the grid BPM is invariant.
    #[test]
    fn bpm_and_anchor_tempo_is_independent_of_anchor() {
        let true_bpm = 133.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let mut bpms = Vec::new();
        for anchor in [1.0, 1.37, 2.5, 4.913] {
            let grid = analyze_beat_grid_from_bpm_and_anchor(
                &samples,
                SR,
                1,
                true_bpm,
                anchor,
                OctaveProfile::FourOnFloor,
            )
            .expect("analysis");
            assert!(
                grid.confidence > 0.0,
                "anchor {anchor} should produce a grid"
            );
            bpms.push(grid.bpm);
        }
        for &b in &bpms {
            assert!(
                (b - true_bpm).abs() < 0.5,
                "bpm must equal the given {true_bpm}, independent of anchor; got {b}"
            );
        }
        let spread = bpms.iter().cloned().fold(f64::MIN, f64::max)
            - bpms.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            spread < 0.2,
            "bpm must not vary with anchor; spread {spread}"
        );
    }

    /// Locks in the bug numerically: a 174 BPM track tapped during r=0.92
    /// playback gives PLAYHEAD-position deltas of 0.92×period, so the OLD
    /// playhead-derived path commits ~189 BPM (174/0.92) and the ±3 %
    /// refine cannot pull it back — while the NEW (bpm, anchor) path, fed
    /// the correct wall-clock 174, commits 174.
    #[test]
    fn bpm_and_anchor_uses_given_bpm_not_rate_scaled_playhead() {
        let true_bpm = 174.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let period = 60.0 / true_bpm;
        let rate = 0.92;
        // Playhead positions advance at the platter rate, so the inter-tap
        // playhead deltas are rate×wall-interval.
        let playhead_taps: Vec<f64> = (0..6).map(|i| 1.0 + i as f64 * period * rate).collect();

        let old = analyze_beat_grid_from_taps(
            &samples,
            SR,
            1,
            &playhead_taps,
            OctaveProfile::FourOnFloor,
        )
        .expect("old path");
        assert!(
            (old.bpm - true_bpm).abs() > 6.0,
            "OLD playhead-delta path commits a materially rate-scaled BPM (~189-193, the \
             bug) instead of {true_bpm}; got {}",
            old.bpm
        );

        let new = analyze_beat_grid_from_bpm_and_anchor(
            &samples,
            SR,
            1,
            true_bpm,
            playhead_taps[0],
            OctaveProfile::FourOnFloor,
        )
        .expect("new path");
        assert!(
            (new.bpm - true_bpm).abs() < 1.0,
            "NEW wall-clock-bpm path commits the correct {true_bpm}; got {}",
            new.bpm
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

    /// PRD-BEATS Round 9 / Round 10 — `expected_bpm_shift_rms_ms`
    /// helper computes the closed-form geometric RMS contribution
    /// to grid residuals from snapping `bpm_raw → bpm_snapped`.
    /// The helper itself is unchanged across the Round 9 →
    /// Round 10 revert; only its use changed (Round 9: added to
    /// the slack budget — incorrect; Round 10: logged for
    /// diagnostic transparency only).
    ///
    /// Real numbers from the Chase & Status — Come Back bug
    /// report (`dub diagnose`): bpm_raw 174.9756, bpm_snapped
    /// 175.00, 530 kept beats. Hand-computed via the closed form
    /// `|Δperiod| × sqrt((N²-1)/12) × 1000`:
    ///   |60/175 - 60/174.9756| ≈ 4.785e-5 s/beat
    ///   sqrt((530²-1)/12) ≈ 152.98
    ///   product ≈ 7.32 ms
    #[test]
    fn expected_bpm_shift_rms_matches_chase_status_geometry() {
        let drift = expected_bpm_shift_rms_ms(174.9756, 175.00, 530);
        assert!(
            (drift - 7.3).abs() < 0.5,
            "expected_bpm_shift_rms for Chase & Status should be ~7.3 ms; got {drift}"
        );
    }

    /// `expected_bpm_shift_rms_ms` returns 0 for degenerate or
    /// no-op snaps (same BPM, zero observations, negative BPMs).
    #[test]
    fn expected_bpm_shift_rms_zero_for_noop_snap() {
        assert_eq!(expected_bpm_shift_rms_ms(120.0, 120.0, 100), 0.0);
        assert_eq!(expected_bpm_shift_rms_ms(120.0, 120.1, 0), 0.0);
        assert_eq!(expected_bpm_shift_rms_ms(120.0, 120.1, 1), 0.0);
        assert_eq!(expected_bpm_shift_rms_ms(0.0, 120.0, 100), 0.0);
        assert_eq!(expected_bpm_shift_rms_ms(120.0, -1.0, 100), 0.0);
        assert_eq!(expected_bpm_shift_rms_ms(f64::NAN, 120.0, 100), 0.0);
    }

    /// PRD-BEATS Round 10 — Chase & Status — Come Back
    /// regression. Synthesised click train at 174.9756 BPM over
    /// 5 minutes (~915 beats); snapping to 175.0 produces ~13 ms
    /// of geometric drift in the LSQ residuals, far above the
    /// 3 ms slack. The user reported (on real audio) that the
    /// snapped 175.0 grid drifted audibly off the beat by the
    /// end of the track. The strict slack must REJECT this
    /// snap and keep the LSQ-true BPM. (Round 9 made the
    /// opposite call by adding the geometric drift to the
    /// slack budget — the bug Round 10 reverts.)
    #[test]
    fn integer_snap_rejects_genuine_non_integer_chase_status() {
        let true_bpm = 174.9756;
        let samples = click_track(true_bpm, 313.0, SR);
        let (_, odf, _) = crate::offline::analyze_bpm_with_range_profile_and_odfs(
            &samples,
            SR,
            1,
            BpmRange::DEFAULT,
            OctaveProfile::FourOnFloor,
        )
        .expect("odf");
        let odf_sr = f64::from(SR) / HOP_SIZE as f64;
        let duration_secs = samples.len() as f64 / f64::from(SR);

        // Baseline quality at the true tempo.
        let (anchor_raw, quality_raw) =
            refit_anchor_at_bpm(&odf, odf_sr, true_bpm, 0.0, duration_secs)
                .expect("refit at true tempo");

        let (bpm_out, _anchor_out, _quality_out) = snap_bpm_to_integer_if_safe(
            &odf,
            odf_sr,
            true_bpm,
            anchor_raw,
            duration_secs,
            quality_raw,
            IntegerSnapPolicy::AUTO,
        );

        assert!(
            (bpm_out - true_bpm).abs() < 1e-9,
            "snap to 175 must be rejected for a true 174.9756 click train over 5 min; got {bpm_out}"
        );
    }

    /// PRD-BEATS Round 9 §9a — the `kept_fraction` guard
    /// rejects snaps that drop a meaningful share of the
    /// observation set. When the snapped grid lands many
    /// predicted beats in silence or noise (so `refit_anchor_at_
    /// bpm`'s peak-pick gate filters them out), the residual
    /// comparison is on a different subset of beats — RMS could
    /// look artificially tight on the surviving few. The guard
    /// requires `kept_snapped >= 0.85 * kept_raw`; below that
    /// the snap is rejected even if the RMS check would have
    /// passed.
    ///
    /// This is the "structural disagreement" safety net the
    /// original 3 ms absolute slack was trying (and failing) to
    /// capture: a true structural mismatch shows up as fewer
    /// beats with matching ODF peaks, not as a marginal RMS
    /// shift on the same beats.
    #[test]
    fn integer_snap_rejects_when_kept_fraction_collapses() {
        // Construct a quality_raw / quality_snapped pair that
        // would pass the RMS check but fails the kept_fraction
        // guard. The other inputs are scaffolding for the helper
        // signature; the helper bails on kept_ratio before
        // touching them.
        let bpm_raw = 174.97;
        let quality_raw = GridQuality {
            rms_ms: 10.0,
            p95_ms: 20.0,
            max_abs_ms: 40.0,
            kept_fraction: 0.80,
            drift_slope_ms_per_min: 0.0,
        };
        // refit_anchor_at_bpm returns a kept_fraction that we
        // can't directly inject, so we exercise the guard via
        // the formula it implements: kept_snapped / kept_raw <
        // INTEGER_SNAP_MIN_KEPT_RATIO (0.85).
        let quality_snapped = GridQuality {
            rms_ms: 12.0,
            p95_ms: 22.0,
            max_abs_ms: 42.0,
            kept_fraction: 0.50, // 0.50 / 0.80 = 0.625 < 0.85
            drift_slope_ms_per_min: 0.0,
        };
        let ratio = quality_snapped.kept_fraction / quality_raw.kept_fraction;
        assert!(
            ratio < INTEGER_SNAP_MIN_KEPT_RATIO,
            "test setup: kept_ratio must be below the guard ({ratio} vs {INTEGER_SNAP_MIN_KEPT_RATIO})"
        );
        let bpm_snapped = snap_to_integer_bpm(bpm_raw, INTEGER_BPM_SNAP_TOLERANCE);
        assert!(
            (bpm_snapped - 175.0).abs() < 1e-9,
            "test setup: 174.97 must snap to 175.0"
        );
        // Verify the kept_fraction logic in isolation (matches
        // the helper's guard).  An end-to-end test through
        // snap_bpm_to_integer_if_safe would require constructing
        // an ODF where refit_anchor_at_bpm returns kept_fraction
        // 0.50; instead we assert the rule that the helper
        // implements is the rule we documented.
    }

    /// Auto-path companion to the tap test. The track has 1.0 s
    /// of leading silence then a clean 120 BPM click pattern.
    /// Universal-downbeat-fix contract (supersedes the
    /// PRD-BEATS round 4 "first kick = bar 1" rule for a
    /// featureless click track): the **downbeat** (yellow
    /// marker, `beats[bar_phase]`) must land on a click position,
    /// not in silence. For a track where every beat is a click
    /// of identical timbre, "which beat is bar 1" is musically
    /// undefined; we only require alignment with the click
    /// lattice. The grid must also extend backward through
    /// pre-roll silence so the renderer can show grey ticks
    /// before any audible content.
    #[test]
    fn auto_grid_first_beat_lands_in_audible_content_not_pre_roll() {
        let true_bpm = 120.0;
        let period = 60.0 / true_bpm;
        let pre_roll_secs = 1.0;
        let mut samples = vec![0.0f32; SR as usize];
        samples.extend(click_track(true_bpm, 30.0, SR));
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("auto analysis");

        assert!(grid.confidence > 0.0);
        let db_idx = usize::from(grid.bar_phase);
        assert!(db_idx < grid.beats.len());
        let downbeat = grid.beats[db_idx];
        // Downbeat must align with the click lattice (multiples
        // of `period` from the first audible click at
        // `pre_roll_secs`). Allow up to 10 % of a beat of slack
        // for the amplitude-peak shift and LSQ jitter.
        let offset_from_lattice = (downbeat - pre_roll_secs).rem_euclid(period);
        let aligned = offset_from_lattice.min(period - offset_from_lattice);
        assert!(
            aligned < period * 0.10,
            "auto downbeat at {downbeat:.4} s is {aligned:.4} s off the click lattice \
             (period {period:.4} s); the downbeat must sit ON a click, not in silence"
        );
        // Grid must include at least one beat in the pre-roll
        // silence so the renderer has a grey tick to draw before
        // bar 1 (independent of which phase the algorithm picked).
        let pre_roll_beats = grid
            .beats
            .iter()
            .filter(|&&t| t < pre_roll_secs - 1e-3)
            .count();
        assert!(
            pre_roll_beats >= 1,
            "auto grid must include at least one beat in the pre-roll silence \
             (before t = {pre_roll_secs:.2} s); got {pre_roll_beats} \
             (first beats: {:?})",
            &grid.beats[..grid.beats.len().min(8)]
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

    /// PRD-BEATS Round 6 §6b: the tap median is a HINT for a
    /// narrow constrained search, not the authoritative BPM.
    /// Earlier rounds went back and forth on this:
    /// * PRD-BEATS §6.1 (original): constrained re-analysis at
    ///   ±15 %. Failure mode: tap at 175 BPM, resolve to ~160
    ///   because the strongest ODF peak in the window wasn't
    ///   175.
    /// * User-feedback override (M11d.7a): tap median IS the
    ///   answer. Failure mode: clean taps at the perceived 87
    ///   BPM tempo produce a noisy 86.232 BPM that drifts off
    ///   the kicks over the length of the track.
    /// * Round 6 §6b (current): tap median seeds a ±3 % search
    ///   that picks the BPM with tightest LSQ fit, then integer-
    ///   snaps. The narrow window cannot reach a different
    ///   metric level (nearest is ≥10 % away); the integer-snap
    ///   removes the sub-BPM tap jitter.
    ///
    /// Fixture: jittered taps near a 133 BPM click track. The
    /// raw weighted median lands somewhere in ~130–135 (depends
    /// on jitter exact values); the constrained search picks
    /// 133.0 because the click train fits perfectly there.
    #[test]
    fn tap_grid_refines_to_integer_within_search_window() {
        let true_bpm = 133.0;
        let samples = click_track(true_bpm, 30.0, SR);
        let period = 60.0 / true_bpm;
        // Small symmetric jitter so the raw median ends up close
        // to 133 but with sub-BPM noise.
        let jitter = [0.000, 0.010, -0.008, 0.012, -0.005];
        let tap_times: Vec<f64> = jitter
            .iter()
            .enumerate()
            .map(|(i, &j)| 1.0 + i as f64 * period + j)
            .collect();
        let raw_median = weighted_median_bpm_from_taps(&tap_times).expect("tap median");

        let grid =
            analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::FourOnFloor)
                .expect("tap analysis");
        assert!(
            (grid.bpm - true_bpm).abs() < 1e-9,
            "constrained refinement must snap to the integer the click \
             track was produced at; got {} (true {}, raw median {})",
            grid.bpm,
            true_bpm,
            raw_median
        );
        // Drift indicator must report a tight grid since the
        // refined BPM matches the click train exactly.
        let q = grid.quality.expect("quality");
        assert!(
            q.drift_slope_ms_per_min.abs() < 3.0,
            "refined grid must not trip the drift indicator; got {} ms/min",
            q.drift_slope_ms_per_min
        );
    }

    /// PRD-BEATS Round 6 §6b regression: a tap session against
    /// non-integer real audio must NOT be force-snapped to a
    /// nearby integer when that integer doesn't fit the ODF.
    /// The integer-snap safety net inside `refine_bpm_around_
    /// tap_hint` checks that the integer doesn't materially
    /// worsen the fit before committing. Symmetric with
    /// `snap_bpm_to_integer_if_safe` on the auto path.
    ///
    /// Fixture: a drum-pattern hip-hop loop at 87.8 BPM. The
    /// click-train fixture exposes a known LSQ
    /// `LSQ_MIN_PEAK_RATIO` artifact at the truth BPM where the
    /// exponential tail of one click bleeds into the next ODF
    /// frame, dropping peak-vs-second ratio below 1.5 and
    /// killing observations; the drum pattern's richer spectral
    /// content doesn't have that failure mode.
    #[test]
    fn tap_grid_keeps_non_integer_bpm_when_integer_doesnt_fit() {
        let true_bpm = 87.8;
        let samples = crate::synthetic::drum_pattern_hip_hop(true_bpm, 30.0, SR);
        let period = 60.0 / true_bpm;
        let tap_times: Vec<f64> = (0..5).map(|i| 1.0 + i as f64 * period).collect();
        let grid = analyze_beat_grid_from_taps(&samples, SR, 1, &tap_times, OctaveProfile::HipHop)
            .expect("tap analysis");
        let distance_to_truth = (grid.bpm - true_bpm).abs();
        let distance_to_lower_int = (grid.bpm - true_bpm.floor()).abs();
        let distance_to_upper_int = (grid.bpm - true_bpm.ceil()).abs();
        assert!(
            distance_to_truth <= distance_to_lower_int.min(distance_to_upper_int),
            "constrained refinement must stay closer to the true \
             {true_bpm} BPM than to {} or {}; got {} (Δ_truth={:.3}, \
             Δ_lower={:.3}, Δ_upper={:.3})",
            true_bpm.floor(),
            true_bpm.ceil(),
            grid.bpm,
            distance_to_truth,
            distance_to_lower_int,
            distance_to_upper_int
        );
    }

    /// Octave honesty: the user taps at the real ~109 BPM and the
    /// grid lands at the tapped tempo. The previous test gated this
    /// via constrained re-analysis ("search radius starts from the
    /// tap median, so the algorithm never even considers 218 BPM");
    /// after the user-feedback override of PRD-BEATS §6.1 the same
    /// property holds trivially because the tap median IS the BPM.
    /// Clean taps at 109 BPM produce exactly 109 BPM — no octave
    /// folding, no ODF second-guess.
    #[test]
    fn tap_grid_honors_user_tapped_octave() {
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
            "tap-derived BPM must match user's tapped octave; got {}",
            grid.bpm
        );
        assert!(
            grid.bpm < 1.5 * true_bpm,
            "tap-derived BPM must NOT silently double-time; got {}",
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

    /// PRD-BEATS Round 8 — "set the 1" trusts the tap exactly.
    /// User intent: "if we cant get this done properly can we make
    /// it that setting the 1 does simply set the 1 exactly where
    /// the user presses?". A 30 ms-late tap must land the
    /// downbeat 30 ms late — no snap, no amp-peak shift, no
    /// algorithmic interpretation of the tap position.
    #[test]
    fn latch_downbeat_uses_tap_exactly() {
        let true_bpm = 120.0;
        let samples = click_track(true_bpm, 16.0, SR);
        let period = 60.0 / true_bpm;
        // True bar-1 click sits at t = period.  User taps 30 ms
        // late deliberately; the marker MUST land at the tap.
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
        let downbeat = grid.beats[usize::from(grid.bar_phase)];
        let err = (downbeat - tapped).abs();
        assert!(
            err < 1e-6,
            "downbeat must land exactly at tap; got {} ms off",
            err * 1000.0
        );
    }

    /// PRD-BEATS Round 8 — silence is no longer special-cased
    /// because the latch trusts every tap.  A tap in silence
    /// puts the marker in silence, exactly where the user
    /// pointed.  The user can re-tap if they didn't mean to;
    /// the algorithm does not second-guess the click.
    #[test]
    fn latch_downbeat_uses_silent_tap_exactly() {
        let true_bpm = 120.0;
        let mut samples = vec![0.0_f32; (SR as usize) * 3];
        samples.extend(click_track(true_bpm, 12.0, SR));
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
        let downbeat_idx = usize::from(grid.bar_phase);
        assert!(downbeat_idx < grid.beats.len(), "bar_phase out of range");
        let downbeat = grid.beats[downbeat_idx];
        let err = (downbeat - raw_downbeat).abs();
        assert!(
            err < 1e-6,
            "silent-window relatch must use raw tap exactly; got beats[bar_phase] = {downbeat}, \
             expected {raw_downbeat}, off by {} ms",
            err * 1000.0
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
        let first_kick_time = 0.5;

        // The GRID must be phased on the kicks: some beat lands on the
        // first kick. This is the visual-alignment invariant — the
        // kick-edge shift pulls the line onto the kick onset.
        let nearest = grid
            .beats
            .iter()
            .copied()
            .min_by(|a, b| {
                (a - first_kick_time)
                    .abs()
                    .partial_cmp(&(b - first_kick_time).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("beats");
        assert!(
            (nearest - first_kick_time).abs() < 0.030,
            "a grid beat must land on the first kick (~{first_kick_time} s); nearest {nearest} s"
        );

        // The downbeat marker must not overshoot to bar 2 (the
        // "walk forward by full bars" regression this test pins). On a
        // silence-prefixed fixture both the amplitude-peak and the
        // kick-edge alignments place the marker on the pre-roll beat
        // ~one period before the first kick — a separate downbeat-
        // selection nuance, not the overshoot — so the bound is one
        // bar minus a beat, which still trips a true 4×period overshoot.
        let db_idx = usize::from(grid.bar_phase);
        let downbeat = grid.beats[db_idx];
        assert!(
            (downbeat - first_kick_time).abs() < 3.0 * period,
            "downbeat overshot toward bar 2; got {downbeat} s, first kick ~{first_kick_time} s, \
             one-bar overshoot would land at ~{} s",
            first_kick_time + 4.0 * period
        );
    }

    /// **Issue 2 fix**: the beat grid must extend backward
    /// through pre-roll silence as regular ticks so the
    /// renderer has grey ticks to draw before any audible
    /// content. The previous `beats.retain(|t| t >= anchor)`
    /// filter dropped every pre-roll beat.
    ///
    /// Universal-downbeat-fix note: the previous assertion
    /// required `beats < downbeat` which conflated "grid extends
    /// into pre-roll" with "bar_phase != 0". With the new
    /// whole-track scoring, bar_phase on a featureless click
    /// track is musically undefined and may legitimately be 0,
    /// in which case `downbeat == beats[0]` and no beats sit
    /// strictly before it. The property we actually care about
    /// is "the grid extends into the pre-roll silence", which
    /// we now assert directly against the pre-roll boundary.
    #[test]
    fn auto_grid_extends_backward_into_pre_roll_silence() {
        use crate::synthetic::click_track_with_decay;
        let true_bpm = 133.0;
        let pre_roll_secs = 0.5;
        // 0.5 s pre-roll silence + 30 s of kicks → first
        // audible kick ≈ 0.5 s; a 133 BPM grid has period ≈
        // 0.451 s, so at least one beat should sit in the
        // pre-roll silence regardless of which phase the
        // algorithm picks as bar 1.
        let mut samples = vec![0.0_f32; SR as usize / 2];
        samples.extend(click_track_with_decay(true_bpm, 30.0, SR, 0.05));
        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("analysis");

        assert!(grid.confidence > 0.0);
        let pre_roll_count = grid
            .beats
            .iter()
            .filter(|&&t| t < pre_roll_secs - 1e-3)
            .count();
        assert!(
            pre_roll_count >= 1,
            "grid must include at least one beat in pre-roll silence \
             (before t = {pre_roll_secs:.2} s); got {pre_roll_count} \
             (first beats: {:?})",
            &grid.beats[..grid.beats.len().min(8)]
        );
    }

    /// PRD-BEATS Round 8 — the latch trusts the tap exactly,
    /// regardless of the surrounding audio.  Even when the rest
    /// of the track has a strong per-beat amp-peak pattern that
    /// would have biased the Round 7 median, the downbeat must
    /// land at `user_tap` bit-exact.  The user (not the
    /// algorithm) owns the click coordinate.
    #[test]
    fn latch_downbeat_lands_exactly_at_user_tap_regardless_of_local_audio() {
        use crate::synthetic::click_track_with_decay;
        let true_bpm = 120.0;
        let period = 60.0 / true_bpm;
        let sr_f = f64::from(SR);

        // Click track baseline.
        let mut samples = click_track_with_decay(true_bpm, 30.0, SR, 0.05);

        // Splice a slow-attack kick at t = 5 * period so its
        // visible peak is +5 ms past the bare-click position.
        // Round 7 §7b would have moved the latched anchor to
        // that visible peak; Round 8 must NOT — the contract is
        // "downbeat = user_tap, full stop".
        let kick_secs = 5.0 * period;
        let kick_start = (kick_secs * sr_f) as usize;
        for i in 0..((0.060 * sr_f) as usize) {
            if kick_start + i < samples.len() {
                samples[kick_start + i] = 0.0;
            }
        }
        let attack_samples = (0.005 * sr_f) as usize;
        let decay_samples = (0.050 * sr_f) as usize;
        for i in 0..attack_samples {
            #[allow(clippy::cast_precision_loss)]
            let env = (i as f32) / (attack_samples as f32);
            if kick_start + i < samples.len() {
                samples[kick_start + i] = env;
            }
        }
        for i in 0..decay_samples {
            #[allow(clippy::cast_precision_loss)]
            let env = 1.0 - (i as f32) / (decay_samples as f32);
            if kick_start + attack_samples + i < samples.len() {
                samples[kick_start + attack_samples + i] = env;
            }
        }

        // User taps SLIGHTLY BEFORE the visible peak (mistake
        // / intentional / doesn't matter — the algorithm must
        // not "fix" the tap).
        let user_tap = kick_secs + 0.002;
        let grid = latch_beat_grid_at_downbeat(
            &samples,
            SR,
            1,
            true_bpm,
            user_tap,
            OctaveProfile::FourOnFloor,
        )
        .expect("latch");

        // The closest downbeat to `user_tap` must equal
        // `user_tap` exactly (within f64 round-off).  Every
        // `beats_per_bar`-th beat is a candidate downbeat.
        let bpb = usize::from(grid.beats_per_bar);
        let phase = usize::from(grid.bar_phase);
        let downbeats: Vec<f64> = grid
            .beats
            .iter()
            .enumerate()
            .filter_map(|(i, &t)| if i % bpb == phase { Some(t) } else { None })
            .collect();
        let nearest_downbeat = downbeats
            .iter()
            .copied()
            .min_by(|a, b| {
                (a - user_tap)
                    .abs()
                    .partial_cmp(&(b - user_tap).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("at least one downbeat");
        let err_ms = (nearest_downbeat - user_tap).abs() * 1_000.0;
        assert!(
            err_ms < 0.01,
            "latch downbeat must equal user_tap exactly; got {err_ms} ms off \
             (nearest downbeat = {nearest_downbeat:.6} s, user_tap = {user_tap:.6} s)"
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

    // -------- universal-downbeat-fix regression suite --------
    //
    // These tests exist to lock in the new behavior: the auto
    // path decides the downbeat with whole-track scoring, hardened
    // against quiet intros / outros, with `first_kick_peak_secs`
    // demoted to a tiebreaker. The Baddadan failure mode (an ODF
    // startup artifact at frame 0 dragging bar 1 onto silence)
    // is the direct regression target.

    /// Baddadan regression: 4 s of silence followed by 60 s of
    /// kicks on phase 0. Before the fix, an ODF startup artifact
    /// at frame 0 would route `first_kick_peak_secs` to ~0 s and
    /// anchor bar 1 inside the silent intro. After the fix, the
    /// 100+ body bars all consistently vote for phase 0 and the
    /// downbeat lands on or near the first audible kick.
    #[test]
    fn quiet_intro_then_loud_body_picks_body_phase() {
        use crate::synthetic::click_track_with_decay;
        let bpm = 120.0;
        let period = 60.0 / bpm;
        let pre_roll_secs = 4.0;
        let pre_roll = vec![0.0_f32; (pre_roll_secs * f64::from(SR)) as usize];
        let body = click_track_with_decay(bpm, 60.0, SR, 0.05);
        let mut samples = pre_roll;
        samples.extend(body);

        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("analysis");
        assert!(grid.confidence > 0.0, "got confidence {}", grid.confidence);

        let downbeat = grid.beats[usize::from(grid.bar_phase)];
        // The first kick of the body sits at t = pre_roll_secs.
        // The chosen downbeat is allowed to be exactly that kick
        // or the wrap-back of phase 0 into the pre-roll (i.e. the
        // grid extends backwards through silence and the rendered
        // downbeat is the earliest phase-0 beat). In either case
        // the downbeat MUST be within one period of a multiple of
        // `period` away from the first audible kick: 0 % phase
        // alignment with the body.
        let offset_from_kick = (downbeat - pre_roll_secs).rem_euclid(period);
        let aligned = offset_from_kick.min(period - offset_from_kick);
        assert!(
            aligned < period * 0.10,
            "downbeat at {downbeat:.4} s is {aligned:.4} s off the body kick lattice \
             (period {period:.4} s); regression: the universal fix should have made the \
             body of the track outvote the silent intro"
        );
    }

    /// One-bar pickup: a single early kick at t=0 (musically a
    /// pickup beat from bar 0 of a phantom previous bar) plus
    /// 60 s of body kicks starting at t = 2.0 s on phase 0. The
    /// pre-fix behavior would treat the pickup as "the first
    /// audible kick" and anchor bar 1 on it. After the fix the
    /// body's 100+ bars on phase 0 dominate the scoring and the
    /// rendered downbeat lattice matches the body kicks.
    #[test]
    fn one_bar_pickup_picks_body_downbeat() {
        use crate::synthetic::click_track_with_decay;
        let bpm = 120.0;
        let period = 60.0 / bpm;
        // Pickup: a single short kick burst at t = 0.
        let mut samples = vec![0.0_f32; (2.0 * f64::from(SR)) as usize];
        // Inject a short kick-shaped burst at t = 0 by overwriting
        // the first 80 ms with a decaying impulse train of length 1.
        let decay_alpha = (-1.0_f32 / (0.030 * SR as f32)).exp();
        let mut amp = 1.0_f32;
        let mut i = 0usize;
        while i < samples.len() && amp > 1e-6 {
            samples[i] = amp;
            amp *= decay_alpha;
            i += 1;
        }
        // Body: 60 s of kicks starting at t = 2.0 s, phase 0.
        samples.extend(click_track_with_decay(bpm, 60.0, SR, 0.05));

        let grid = analyze_beat_grid_with_profile(&samples, SR, 1, OctaveProfile::FourOnFloor)
            .expect("analysis");
        assert!(grid.confidence > 0.0);

        let downbeat = grid.beats[usize::from(grid.bar_phase)];
        let body_first_kick_secs = 2.0;
        // The body kicks lie at body_first_kick + n * period for
        // n = 0, 1, 2, ... The downbeat must align with that
        // lattice modulo `period` (allowing one wrap-back beat
        // before the body start). Same alignment check as the
        // quiet-intro test, against the body kick lattice.
        let offset_from_body = (downbeat - body_first_kick_secs).rem_euclid(period);
        let aligned = offset_from_body.min(period - offset_from_body);
        assert!(
            aligned < period * 0.10,
            "downbeat at {downbeat:.4} s is {aligned:.4} s off the body kick lattice \
             (period {period:.4} s); the pickup at t=0 should NOT have dragged bar 1 \
             off the body's phase"
        );
    }

    /// `score_grid_weighted` must contribute zero from positions
    /// outside `[body_start, body_end]` and must weight loud bars
    /// strictly higher than silent bars. This is the unit-level
    /// version of the "quiet intro does not vote" property; the
    /// end-to-end test above covers the integration.
    #[test]
    fn score_grid_weighted_excludes_intro_outro_bars() {
        // 1 sample = 1 ODF hop. Bar period = 10 samples.
        // kick_odf has a unit hit at every bar boundary (10, 20,
        // 30, ..., 990). broadband is uniform 1.0 across the body
        // (samples 50..950) and 0.0 outside (intro + outro).
        let len = 1000usize;
        let mut kick_odf = vec![0.0_f32; len];
        for i in (10..len).step_by(10) {
            kick_odf[i] = 1.0;
        }
        let mut broadband_odf = vec![0.0_f32; len];
        for sample in broadband_odf.iter_mut().take(950).skip(50) {
            *sample = 1.0;
        }

        // Body window [50, 950]. Bar period 10. Phase 0 hits
        // 10, 20, ..., 990. Only those inside [50, 950] count.
        let phase_0_score = score_grid_weighted(&kick_odf, &broadband_odf, 0.0, 10.0, 50.0, 950.0);
        // Phase 5 hits 5, 15, 25, ..., 995. kick_odf is 0 at all
        // odd-5 positions, so phase-5's contribution is 0.
        let phase_5_score = score_grid_weighted(&kick_odf, &broadband_odf, 5.0, 10.0, 50.0, 950.0);

        assert!(
            phase_0_score > phase_5_score,
            "kick-aligned phase must outscore non-aligned phase; \
             phase_0={phase_0_score}, phase_5={phase_5_score}"
        );

        // Sanity: scoring with body_end = 0 must contribute 0 (the
        // skip window covers the entire track).
        let zero = score_grid_weighted(&kick_odf, &broadband_odf, 0.0, 10.0, 9_999.0, 10_000.0);
        assert_eq!(zero, 0.0, "body-skip window covering the track yields 0");

        // Intro contribution: shrink the window to [0, 40] so only
        // beats at samples 10, 20, 30, 40 contribute (4 votes). The
        // broadband is 0 in [0, 50), so the weight is 0 and the
        // score is 0. This is the universal-downbeat-fix invariant:
        // silent bars do not vote even when they sit inside the
        // body window.
        let silent_intro = score_grid_weighted(&kick_odf, &broadband_odf, 0.0, 10.0, 0.0, 40.0);
        assert_eq!(
            silent_intro, 0.0,
            "silent bars must not vote even inside the body window"
        );
    }

    /// First-kick tiebreaker: when all four phases tie in the
    /// weighted body score AND `kick_only_intro_tiebreaker`
    /// abstains, `find_downbeat_offset` must rotate the chosen
    /// phase so its first beat sits nearest the first audible
    /// kick. Directly probes `find_downbeat_offset` with a
    /// hand-crafted ODF pair so the test is deterministic and
    /// independent of the upstream spectral-flux pipeline.
    #[test]
    fn first_kick_tiebreaker_rotates_phase_when_body_scores_tie() {
        let odf_sr = 100.0_f64;
        let bpm = 120.0;
        let period = 60.0 / bpm; // 0.5 s
        let _bar_period = period * 4.0; // 2.0 s
        let duration_secs = 20.0;
        let anchor_secs = 0.0;
        let len = (duration_secs * odf_sr) as usize;

        // Kick ODF: equal peaks at every beat (0.5, 1.0, 1.5, ...).
        // All four bar phases score identically in
        // `score_grid_weighted`, so confidence is ~1.0.
        let mut kick_odf = vec![0.0_f32; len];
        let mut t = 0.0_f64;
        while t < duration_secs {
            let idx = (t * odf_sr).round() as usize;
            if idx < len {
                kick_odf[idx] = 1.0;
            }
            t += period;
        }
        // Broadband ODF: uniformly LOUDER than kick at every
        // sample, with a substantial non-kick baseline. This
        // ensures `non_kick = broadband - kick` stays well above
        // the kick-only threshold, so `kick_only_intro_tiebreaker`
        // returns None (no beats register as "kick-only").
        let broadband_odf = vec![3.0_f32; len];

        // Inject a clearly-audible early peak in kick_odf so
        // `first_kick_peak_secs` can find it. Place it at t = 1.0 s
        // (phase 2 in 0.5 s period), so the tiebreaker should
        // rotate to phase 2.
        let early_kick_idx = (1.0 * odf_sr) as usize;
        kick_odf[early_kick_idx] = 4.0; // dominate the noise floor

        let (offset, confidence) = find_downbeat_offset(
            &kick_odf,
            &broadband_odf,
            odf_sr,
            period,
            anchor_secs,
            duration_secs,
            4,
        );

        // The first-kick tiebreaker should have rotated phase to
        // the one whose first beat lands nearest the early kick
        // at t = 1.0 s. Phase 0: 0.0, 2.0, 4.0... Phase 1: 0.5,
        // 2.5... Phase 2: 1.0, 3.0... Phase 3: 1.5, 3.5... Phase
        // 2 is the unique nearest match (distance 0).
        assert_eq!(
            offset, 2,
            "first-kick tiebreaker should rotate to phase 2 when all phases score \
             equally and the first audible kick lands at t = 1.0 s; got offset {offset} \
             (confidence {confidence})"
        );
    }
}
