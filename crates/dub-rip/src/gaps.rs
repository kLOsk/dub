//! Auto gap detection over the capture envelope (M26b).
//!
//! Pure analysis: envelope chunks in, candidate track boundaries out.
//! It never touches the spill — the envelope the capture worker built
//! is frame-aligned by construction (`chunk[i]` covers frames
//! `i * frames_per_chunk ..`), which is the whole reason it exists.
//!
//! The detector is deliberately *adaptive* rather than thresholded at
//! a fixed dBFS: a well-pressed reissue sits ~55 dB under the music,
//! a played-out sound-system 7" maybe 30 dB, and a fixed −40 dBFS
//! line would either miss every gap on the second record or split the
//! first one mid-breakdown. The noise floor is measured from the side
//! itself and the gap line is set a margin above it.

use dub_peaks::PeakChunk;

/// Analysis resolution. 50 ms is short enough to place a boundary
/// inside a gap and long enough that one click doesn't move the cell:
/// a pop spans a few 1.33 ms chunks out of ~37, so the per-cell
/// median ignores it.
const CELL_SECS: f32 = 0.05;

/// The noise floor is read off the 20th-quietest cell (≈1 s), not a
/// percentile: a single 2 s gap is ~1 % of a side, so any percentile
/// coarse enough to be robust would land in the music instead. Taking
/// a *rank* rather than the minimum steps over the handful of
/// digital-silence cells an input underrun leaves behind (the record
/// tap zero-fills, by design).
const FLOOR_RANK_CELLS: usize = 20;

/// A silence run long enough to be an inter-track gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    /// First frame of the detected silence.
    pub start_frame: u64,
    /// Frame the music comes back at.
    pub end_frame: u64,
    /// Where the split should go — `end_frame` less the pre-roll, so
    /// the next track keeps the air before its first transient.
    pub boundary_frame: u64,
}

/// Tunables for [`detect_gaps`].
#[derive(Debug, Clone, Copy)]
pub struct GapConfig {
    /// Shortest silence treated as a track gap. Vinyl inter-track
    /// gaps run 1.5–3 s; dub drop-outs rarely hold true silence this
    /// long.
    pub min_gap_secs: f32,
    /// Shortest track the detector will propose. Far above
    /// [`crate::MIN_SEGMENT_SECS`] (which only bounds *manual*
    /// splits): a mid-track false positive is worse than a missed
    /// gap, because the operator has to notice it to undo it.
    pub min_track_secs: f32,
    /// How far before the returning music to place the boundary.
    pub pre_roll_secs: f32,
    /// Gap line, in dB above the measured noise floor.
    pub margin_db: f32,
    /// How far under the music a gap must sit, regardless of the
    /// floor estimate. This is what keeps a dub breakdown intact:
    /// when a side contains no true silence the floor estimate lands
    /// on the quietest *music*, and the margin alone would happily
    /// cut there.
    pub min_drop_db: f32,
    /// Give up unless the music sits at least this far above the
    /// noise floor. A side with no contrast (a locked groove, a dead
    /// input, a wall of noise) gets no proposals rather than
    /// arbitrary ones.
    pub min_contrast_db: f32,
}

impl Default for GapConfig {
    fn default() -> Self {
        Self {
            min_gap_secs: 1.2,
            min_track_secs: 30.0,
            pre_roll_secs: 0.3,
            margin_db: 8.0,
            min_drop_db: 20.0,
            min_contrast_db: 12.0,
        }
    }
}

/// Find the inter-track gaps in a captured side.
///
/// `total_frames` bounds the result (the envelope's last chunk may
/// overhang the recording); every returned `boundary_frame` is a
/// valid split for a recording of that length, so the output can go
/// straight into [`crate::validate_boundaries`].
///
/// Silence before the first music (needle drop, lead-in groove) and
/// after the last (run-out) is not a gap — there is no track on the
/// far side of it.
#[must_use]
pub fn detect_gaps(
    envelope: &[PeakChunk],
    frames_per_chunk: usize,
    sample_rate: u32,
    total_frames: u64,
    cfg: &GapConfig,
) -> Vec<Gap> {
    if envelope.is_empty() || frames_per_chunk == 0 || sample_rate == 0 || total_frames == 0 {
        return Vec::new();
    }

    #[allow(clippy::cast_precision_loss)]
    let chunk_secs = frames_per_chunk as f32 / sample_rate as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let chunks_per_cell = ((CELL_SECS / chunk_secs).round() as usize).max(1);
    let cell_secs = chunk_secs * chunks_per_cell as f32;
    let cell_frames = (chunks_per_cell * frames_per_chunk) as u64;

    let mut scratch = Vec::with_capacity(chunks_per_cell);
    let cells: Vec<f32> = envelope
        .chunks(chunks_per_cell)
        .map(|cell| median_db(cell, &mut scratch))
        .collect();
    // Two cells cannot hold a gap with music on both sides.
    if cells.len() < 3 {
        return Vec::new();
    }

    let mut ranked = cells.clone();
    ranked.sort_by(f32::total_cmp);
    let floor_db = ranked[FLOOR_RANK_CELLS.min(ranked.len() - 1)];
    let music_db = percentile(&ranked, 0.80);
    if music_db - floor_db < cfg.min_contrast_db {
        return Vec::new();
    }
    let threshold_db = (floor_db + cfg.margin_db).min(music_db - cfg.min_drop_db);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let min_gap_cells = ((cfg.min_gap_secs / cell_secs).ceil() as usize).max(1);
    let pre_roll_frames = secs_to_frames(cfg.pre_roll_secs, sample_rate);
    let min_track_frames = secs_to_frames(cfg.min_track_secs, sample_rate);

    let mut candidates = Vec::new();
    let mut run_start: Option<usize> = None;
    for index in 0..=cells.len() {
        let quiet = cells.get(index).is_some_and(|&db| db < threshold_db);
        match (quiet, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                run_start = None;
                // A run touching either end is lead-in or run-out:
                // silence with no track on the far side of it.
                if start == 0 || index == cells.len() || index - start < min_gap_cells {
                    continue;
                }
                let start_frame = (start as u64 * cell_frames).min(total_frames);
                let end_frame = (index as u64 * cell_frames).min(total_frames);
                candidates.push(Gap {
                    start_frame,
                    end_frame,
                    boundary_frame: end_frame.saturating_sub(pre_roll_frames).max(start_frame),
                });
            }
            _ => {}
        }
    }

    // Keep only boundaries that leave a real track on both sides.
    // Dropping a boundary merges its snippet into the following
    // track, which is what the operator would do by hand.
    let mut kept: Vec<Gap> = Vec::with_capacity(candidates.len());
    let mut prev_boundary = 0_u64;
    for gap in candidates {
        if gap.boundary_frame == 0 || gap.boundary_frame >= total_frames {
            continue;
        }
        if gap.boundary_frame - prev_boundary < min_track_frames {
            continue;
        }
        if total_frames - gap.boundary_frame < min_track_frames {
            continue;
        }
        prev_boundary = gap.boundary_frame;
        kept.push(gap);
    }
    kept
}

/// Median chunk level of one cell, in dBFS. The median (not the max)
/// is what makes a click in the run-out groove harmless: a pop spans
/// a few chunks out of dozens.
fn median_db(cell: &[PeakChunk], scratch: &mut Vec<f32>) -> f32 {
    scratch.clear();
    scratch.extend(cell.iter().map(|c| c.rms));
    scratch.sort_by(f32::total_cmp);
    to_db(scratch[scratch.len() / 2])
}

/// `p` of the way through an ascending-sorted slice.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn percentile(sorted: &[f32], p: f32) -> f32 {
    let last = sorted.len() - 1;
    sorted[((last as f32 * p).round() as usize).min(last)]
}

fn to_db(rms: f32) -> f32 {
    // −140 dB floor: keeps digital silence finite without touching
    // any level a phono chain can produce.
    20.0 * rms.abs().max(1e-7).log10()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn secs_to_frames(secs: f32, sample_rate: u32) -> u64 {
    (f64::from(secs) * f64::from(sample_rate)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{validate_boundaries, MIN_SEGMENT_SECS};

    const SR: u32 = 48_000;
    const FPC: usize = 64;

    /// Build an envelope from `(rms, secs)` runs. `min`/`max` are
    /// filled at a plausible crest factor so a detector that reads
    /// them instead of `rms` still sees the same shape.
    fn env(runs: &[(f32, f32)]) -> Vec<PeakChunk> {
        let mut out = Vec::new();
        for &(rms, secs) in runs {
            let chunks = (secs * SR as f32 / FPC as f32).round() as usize;
            out.extend(std::iter::repeat_n(
                PeakChunk {
                    min: -rms * 1.4,
                    max: rms * 1.4,
                    rms,
                },
                chunks,
            ));
        }
        out
    }

    fn total(envelope: &[PeakChunk]) -> u64 {
        (envelope.len() * FPC) as u64
    }

    fn secs_of(frame: u64) -> f32 {
        frame as f32 / SR as f32
    }

    /// Music at −14 dBFS over a −50 dBFS pressing — the ordinary case.
    const MUSIC: f32 = 0.2;
    const FLOOR: f32 = 0.003;

    #[test]
    fn one_gap_between_two_tracks_yields_one_boundary() {
        let e = env(&[(MUSIC, 60.0), (FLOOR, 2.0), (MUSIC, 60.0)]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());

        assert_eq!(gaps.len(), 1, "expected exactly one gap: {gaps:?}");
        let g = gaps[0];
        assert!(
            (secs_of(g.start_frame) - 60.0).abs() < 0.2,
            "gap starts at {} s",
            secs_of(g.start_frame)
        );
        assert!(
            (secs_of(g.end_frame) - 62.0).abs() < 0.2,
            "gap ends at {} s",
            secs_of(g.end_frame)
        );
        // Boundary sits a pre-roll ahead of the returning music.
        assert!(
            (secs_of(g.boundary_frame) - 61.7).abs() < 0.2,
            "boundary at {} s",
            secs_of(g.boundary_frame)
        );
    }

    #[test]
    fn pause_shorter_than_min_gap_is_not_a_split() {
        let e = env(&[(MUSIC, 60.0), (FLOOR, 0.5), (MUSIC, 60.0)]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert!(gaps.is_empty(), "0.5 s pause split the track: {gaps:?}");
    }

    #[test]
    fn breakdown_above_the_noise_floor_is_not_a_split() {
        // A dub breakdown: everything drops out but the echo tail
        // holds ~−30 dBFS. On a real side this can be the quietest
        // thing on the record, so the floor estimate lands *on it* —
        // only the drop-below-the-music rule saves the track.
        let e = env(&[(MUSIC, 60.0), (0.03, 4.0), (MUSIC, 60.0)]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert!(gaps.is_empty(), "breakdown was split: {gaps:?}");
    }

    #[test]
    fn lead_in_and_run_out_are_not_gaps() {
        let e = env(&[(FLOOR, 6.0), (MUSIC, 90.0), (FLOOR, 8.0)]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert!(gaps.is_empty(), "edge silence became a split: {gaps:?}");
    }

    #[test]
    fn gap_too_close_to_the_previous_split_is_dropped() {
        // 60 s track, gap, 10 s snippet, gap, 60 s track. With a 30 s
        // minimum the snippet cannot stand alone, so only the first
        // gap survives and the snippet rides with track 2.
        let e = env(&[
            (MUSIC, 60.0),
            (FLOOR, 2.0),
            (MUSIC, 10.0),
            (FLOOR, 2.0),
            (MUSIC, 60.0),
        ]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert_eq!(gaps.len(), 1, "expected one surviving gap: {gaps:?}");
        assert!((secs_of(gaps[0].boundary_frame) - 61.7).abs() < 0.3);
    }

    #[test]
    fn gap_too_close_to_the_end_is_dropped() {
        let e = env(&[(MUSIC, 90.0), (FLOOR, 2.0), (MUSIC, 8.0)]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert!(gaps.is_empty(), "8 s tail became a track: {gaps:?}");
    }

    #[test]
    fn noisy_pressing_still_splits() {
        // Played-out sound-system 45: surface noise at −40 dBFS,
        // music cut hot at −12. A fixed −45 dBFS gate would never
        // see this gap.
        let e = env(&[(0.25, 60.0), (0.01, 2.5), (0.25, 60.0)]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert_eq!(gaps.len(), 1, "loud pressing missed its gap: {gaps:?}");
    }

    #[test]
    fn no_contrast_yields_no_proposals() {
        let e = env(&[(0.05, 200.0)]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert!(gaps.is_empty(), "flat side produced splits: {gaps:?}");
    }

    #[test]
    fn clicks_inside_the_gap_do_not_break_the_run() {
        // Two full-scale pops, 2 ms each, inside a 2 s gap.
        let mut e = env(&[(MUSIC, 60.0), (FLOOR, 2.0), (MUSIC, 60.0)]);
        let pop = (60.5 * SR as f32 / FPC as f32) as usize;
        for i in 0..2 {
            for c in 0..2 {
                e[pop + i * 300 + c] = PeakChunk {
                    min: -0.9,
                    max: 0.9,
                    rms: 0.8,
                };
            }
        }
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert_eq!(gaps.len(), 1, "clicks defeated the gap: {gaps:?}");
    }

    #[test]
    fn empty_and_degenerate_input_is_handled() {
        let cfg = GapConfig::default();
        assert!(detect_gaps(&[], FPC, SR, 0, &cfg).is_empty());
        let e = env(&[(MUSIC, 10.0)]);
        assert!(detect_gaps(&e, FPC, 0, total(&e), &cfg).is_empty());
        assert!(detect_gaps(&e, 0, SR, total(&e), &cfg).is_empty());
    }

    #[test]
    fn proposed_boundaries_are_a_valid_split_plan() {
        let e = env(&[
            (FLOOR, 4.0),
            (MUSIC, 200.0),
            (FLOOR, 2.0),
            (MUSIC, 180.0),
            (FLOOR, 2.2),
            (MUSIC, 240.0),
            (FLOOR, 6.0),
        ]);
        let total_frames = total(&e);
        let gaps = detect_gaps(&e, FPC, SR, total_frames, &GapConfig::default());
        assert_eq!(gaps.len(), 2, "expected two gaps: {gaps:?}");

        let bounds: Vec<u64> = gaps.iter().map(|g| g.boundary_frame).collect();
        validate_boundaries(&bounds, total_frames, SR, MIN_SEGMENT_SECS)
            .expect("auto boundaries must be a valid plan");
    }
}
