//! Offline beat-grid analysis: BPM + phase → **uniform 4/4 grid**.
//!
//! This is the Traktor-style approach the PRD §8.3.1 spec asks for:
//! detect transients, find the constant `(period, phase)` that places
//! the most transients on grid lines, and emit a strictly uniform
//! grid `anchor + i × period`. The grid is mathematically a ruler;
//! displayed BPM and ruler period are by definition the same number.
//!
//! ## Algorithm
//!
//! 1. Spectral-flux onset detection function (ODF) from `dub_bpm`.
//! 2. Harmonic-summed autocorrelation → **coarse period**
//!    (`offline::analyze_bpm_with_range_and_profile`).
//! 3. **Local refinement**: search ±0.5 % around the coarse period
//!    in 0.05 % steps for the `(period, phase)` pair that maximises
//!    summed ODF energy at `phase + i × period`. Corrects the
//!    0.1–0.5 BPM rounding errors that accumulate to several
//!    hundred ms over a 4-minute track.
//! 4. Emit the uniform grid.
//!
//! Manual downbeat correction ([`latch_beat_grid_at_downbeat`])
//! pins the anchor at the user-supplied time and refines the
//! period locally around the current BPM, keeping the grid uniform.

use crate::offline::analyze_bpm_with_range_and_profile;
use crate::{AnalysisError, BpmRange, OctaveProfile, HOP_SIZE};

/// Period search bounds for `analyze_beat_grid_with_profile`, as a
/// fraction of the autocorrelation seed. ±0.5 % is enough to clean
/// up the granularity of the autocorrelation peak (the lag-domain
/// quantisation at the ODF sample rate is ~0.6 % at 128 BPM) while
/// staying narrow enough that the search stays in the same tempo
/// octave.
const AUTO_REFINE_RANGE_PCT: f64 = 0.005;
const AUTO_REFINE_STEP_PCT: f64 = 0.0005;

/// Period search bounds for [`latch_beat_grid_at_downbeat`]. Slightly
/// wider than the auto path because a user pressing "1" has likely
/// already noticed the auto BPM is off and nudged it in roughly the
/// right direction; refining ±1 % gives the press-1 action a bit of
/// "snap to the right answer" forgiveness.
const RELATCH_REFINE_RANGE_PCT: f64 = 0.010;
const RELATCH_REFINE_STEP_PCT: f64 = 0.0005;

/// Per-track beat grid. Returned by [`analyze_beat_grid`]; consumed
/// by the renderer to draw beat ticks on the waveform.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatGrid {
    /// Tempo, in beats per minute. The grid period is exactly
    /// `60.0 / bpm` — there is no separate "per-beat interval".
    pub bpm: f64,
    /// Tempo-estimator confidence in `[0.0, 1.0]`. `0.0` means
    /// "no periodic structure detected; `beats` is empty".
    pub confidence: f32,
    /// Uniform beat positions in seconds from sample 0. Invariant:
    /// `beats[i] == beats[0] + i × 60.0 / bpm` to within floating
    /// point. Beat 0 is the downbeat anchor.
    pub beats: Vec<f64>,
    /// Beats per bar. Fixed at 4 for v0; every 4th beat is the
    /// downbeat (the visual "1") by convention.
    pub beats_per_bar: u8,
}

impl BeatGrid {
    /// An empty grid. Returned when BPM detection fails (silence,
    /// non-musical input, too-short audio after the
    /// [`AnalysisError::TooShort`] gate).
    #[must_use]
    pub const fn none() -> Self {
        Self {
            bpm: 0.0,
            confidence: 0.0,
            beats: Vec::new(),
            beats_per_bar: 4,
        }
    }
}

/// Analyse a buffer and return its uniform beat grid (default profile).
pub fn analyze_beat_grid(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> Result<BeatGrid, AnalysisError> {
    analyze_beat_grid_with_profile(samples, sample_rate, channels, OctaveProfile::Default)
}

/// Like [`analyze_beat_grid`] but applies a genre-derived
/// [`OctaveProfile`] during tempo estimation (M11c.3d).
pub fn analyze_beat_grid_with_profile(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    profile: OctaveProfile,
) -> Result<BeatGrid, AnalysisError> {
    let (estimate, odf) = analyze_bpm_with_range_and_profile(
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
    let Some((bpm, anchor_secs)) = refine_period_and_phase(
        &odf,
        odf_sr,
        estimate.bpm,
        AUTO_REFINE_RANGE_PCT,
        AUTO_REFINE_STEP_PCT,
    ) else {
        return Ok(BeatGrid::none());
    };
    let beats = uniform_beats(bpm, anchor_secs, duration_secs);
    if beats.is_empty() {
        return Ok(BeatGrid::none());
    }
    Ok(BeatGrid {
        bpm,
        confidence: estimate.confidence,
        beats,
        beats_per_bar: 4,
    })
}

/// Re-anchor the grid at a user-supplied downbeat time (M11d.6).
///
/// Keeps the supplied BPM as a starting point and refines the
/// period in ±1 % around it for the value that maximises summed
/// ODF energy on grid lines `downbeat_secs + i × period`. The
/// resulting grid is **uniform**.
///
/// This is the "press 1 at the kick" path. The user has scrubbed
/// to a confirmed downbeat; the function builds a 4/4 ruler
/// originating at that exact time.
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
    let (_, odf) = analyze_bpm_with_range_and_profile(
        samples,
        sample_rate,
        channels,
        BpmRange::DEFAULT,
        profile,
    )?;
    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let duration_secs =
        samples.len() as f64 / (f64::from(sample_rate) * f64::from(channels.max(1)));
    let refined_bpm = refine_period_at_anchor(
        &odf,
        odf_sr,
        bpm,
        downbeat_secs,
        RELATCH_REFINE_RANGE_PCT,
        RELATCH_REFINE_STEP_PCT,
    )
    .unwrap_or(bpm);
    let beats = uniform_beats(refined_bpm, downbeat_secs, duration_secs);
    if beats.is_empty() {
        return Ok(BeatGrid::none());
    }
    Ok(BeatGrid {
        bpm: refined_bpm,
        confidence: 1.0,
        beats,
        beats_per_bar: 4,
    })
}

/// Build a uniform `anchor + i × (60/bpm)` grid clamped to
/// `[0, duration_secs]`. The anchor is walked into the first
/// period so the first emitted beat always sits in `[0, period)`.
///
/// Used by the renderer-facing `BeatGrid` whenever a uniform grid
/// is desired (library row loads, BPM nudge, manual install).
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

/// Search around `bpm_init` for the `(period, phase)` pair that
/// maximises summed ODF energy at grid positions. Returns
/// `(bpm, anchor_secs)` or `None` on degenerate input.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn refine_period_and_phase(
    odf: &[f32],
    odf_sr: f64,
    bpm_init: f64,
    range_pct: f64,
    step_pct: f64,
) -> Option<(f64, f64)> {
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
    let (period, phase, _) = best?;
    let bpm = 60.0 * odf_sr / period;
    let anchor_secs = phase / odf_sr;
    Some((bpm, anchor_secs))
}

/// Search around `bpm_init` for the period that maximises summed
/// ODF energy on the grid `anchor_secs + i × period`. Returns the
/// refined BPM or `None` on degenerate input.
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

/// For a fixed period, find the integer-sample phase in
/// `[0, period_int)` that maximises grid energy, then parabolic-
/// refine to a fractional phase.
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

/// Sum ODF samples on the grid `phase + i × period` for `i = 0..`,
/// with linear interpolation between adjacent ODF samples so the
/// score is smooth in `period`.
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

/// Median inter-beat interval → BPM. Empty when fewer than two
/// beats. Kept public for callers that want to derive a tempo
/// estimate from a hand-built beat list (e.g. importer adapters).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::octave_profile::OctaveProfile;
    use crate::synthetic::click_track;

    const SR: u32 = 48_000;

    /// Returns `true` if every consecutive pair of beats sits at the
    /// expected uniform period within a tight tolerance.
    fn beats_are_uniform(beats: &[f64], bpm: f64) -> bool {
        if beats.len() < 2 {
            return true;
        }
        let period = 60.0 / bpm;
        beats
            .windows(2)
            .all(|w| ((w[1] - w[0]) - period).abs() < 1e-9)
    }

    #[test]
    fn click_120_bpm_emits_uniform_grid_at_500_ms() {
        let samples = click_track(120.0, 16.0, SR);
        let grid = analyze_beat_grid(&samples, SR, 1).expect("analysis");

        assert!(grid.confidence > 0.5, "got confidence {}", grid.confidence);
        assert!(
            (grid.bpm - 120.0).abs() < 0.1,
            "BPM should be ≈120, got {}",
            grid.bpm
        );
        assert!(grid.beats.len() > 10, "expected many beats");
        assert!(
            beats_are_uniform(&grid.beats, grid.bpm),
            "beats not uniform"
        );

        let first = grid.beats[0];
        let nearest_click = (first / 0.5).round() * 0.5;
        assert!(
            (first - nearest_click).abs() < 0.025,
            "first beat should be within 25 ms of a click position; got {first} s"
        );
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
        assert!(
            (first_aligned - 0.25).abs() < 0.03,
            "first aligned beat should be ≈0.25 s, got {first_aligned}"
        );
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

        let last = *grid.beats.last().expect("beats");
        let nearest_click = (last / (60.0 / true_bpm)).round() * (60.0 / true_bpm);
        assert!(
            (last - nearest_click).abs() < 0.025,
            "last beat should sit on a click; got {last}, nearest {nearest_click}"
        );
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
}
