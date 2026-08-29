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

use std::ops::Range;

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

/// Half-width of the vote that decides whether a cell is music, in
/// cells — ~0.5 s each side. Wide enough that no click carries a
/// majority, narrow enough to place the end of a side within a
/// second of the last note.
const MUSIC_VOTE_CELLS: usize = 10;

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
    ///
    /// 21 dB, measured — not the 8 dB first guessed. On a real
    /// pressing the groove between tracks is far noisier than the
    /// quietest groove on the side (dust, wear, the tail of the last
    /// tune): a 4-track reggae side measured its floor at −57.8 dBFS
    /// while its inter-track gaps sat at −40, and an 8 dB margin put
    /// the line 10 dB under every gap on the record.
    ///
    /// 21 is the centre of the plateau three real sides agree on — a
    /// reggae 12", a soul sampler and a drum'n'bass 45 all yield
    /// exactly their own track count anywhere in 20.4–22 dB, and each
    /// end of that window is where one of them starts to go wrong.
    pub margin_db: f32,
    /// Give up unless the music sits at least this far above the
    /// noise floor.
    ///
    /// This is what keeps a dub breakdown intact. When a side holds
    /// no true silence the floor estimate lands on the quietest
    /// *music* and every line drawn from it cuts there — so the
    /// answer is to refuse the side, not to bias the line. 24 dB sits
    /// between the 16.5 dB a breakdown-floored side measures and the
    /// 33 dB of the tightest real record on hand.
    ///
    /// An earlier cut instead capped the line at `music − 18 dB`.
    /// That cap binds whenever contrast is under ~38 dB, which is
    /// most records, so the safety net was silently the operative
    /// rule — and it tracks how loud the side was *cut* rather than
    /// how noisy it is. Measured on a quiet-mastered soul sampler it
    /// put the line at −43.2 dBFS against gaps at −38, and found one
    /// of the nine.
    pub min_contrast_db: f32,
}

impl Default for GapConfig {
    fn default() -> Self {
        Self {
            min_gap_secs: 1.5,
            min_track_secs: 30.0,
            pre_roll_secs: 0.3,
            margin_db: 21.0,
            min_contrast_db: 24.0,
        }
    }
}

/// What the detector measured off a side — the numbers every
/// threshold decision is made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GapAnalysis {
    /// Noise floor of the *needle-down* audio, dBFS.
    pub floor_db: f32,
    /// Music level (80th percentile cell), dBFS.
    pub music_db: f32,
    /// The line below which a cell counts as quiet, dBFS.
    pub threshold_db: f32,
    /// Seconds of the side judged to be under the stylus.
    pub played_secs: f32,
    /// First frame of music — everything before it is lead-in.
    pub music_start_frame: u64,
    /// Frame the last music ends at. Everything after it is run-out,
    /// and the side is committed as if the recording stopped here.
    pub music_end_frame: u64,
    /// False when the side has too little dynamic range to split.
    pub usable: bool,
}

/// The per-cell level view the detector works from: one median dBFS
/// value per ~50 ms, plus the seconds each cell covers.
///
/// Exposed for tuning tools, which must look at exactly what the
/// detector looks at rather than re-deriving it.
#[must_use]
pub fn cell_levels_db(
    envelope: &[PeakChunk],
    frames_per_chunk: usize,
    sample_rate: u32,
) -> (Vec<f32>, f32) {
    if envelope.is_empty() || frames_per_chunk == 0 || sample_rate == 0 {
        return (Vec::new(), CELL_SECS);
    }
    #[allow(clippy::cast_precision_loss)]
    let chunk_secs = frames_per_chunk as f32 / sample_rate as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let chunks_per_cell = ((CELL_SECS / chunk_secs).round() as usize).max(1);
    let mut scratch = Vec::with_capacity(chunks_per_cell);
    let cells = envelope
        .chunks(chunks_per_cell)
        .map(|cell| median_db(cell, &mut scratch))
        .collect();
    (cells, chunk_secs * chunks_per_cell as f32)
}

/// Measure a side without proposing anything — what
/// [`detect_gaps`] decides from. Exposed so tuning tools report the
/// detector's own numbers instead of re-deriving them and drifting.
#[must_use]
pub fn analyze(
    envelope: &[PeakChunk],
    frames_per_chunk: usize,
    sample_rate: u32,
    cfg: &GapConfig,
) -> GapAnalysis {
    let unusable = GapAnalysis {
        floor_db: -140.0,
        music_db: -140.0,
        threshold_db: -140.0,
        played_secs: 0.0,
        music_start_frame: 0,
        music_end_frame: 0,
        usable: false,
    };
    if envelope.is_empty() || frames_per_chunk == 0 || sample_rate == 0 {
        return unusable;
    }
    let Some(measured) = measure(envelope, frames_per_chunk, sample_rate, cfg) else {
        return unusable;
    };
    measured.analysis
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
    let Some(m) = measure(envelope, frames_per_chunk, sample_rate, cfg) else {
        return Vec::new();
    };
    if !m.analysis.usable {
        return Vec::new();
    }
    let (cells, cell_secs, cell_frames, played) = (m.cells, m.cell_secs, m.cell_frames, m.played);
    let threshold_db = m.analysis.threshold_db;
    // The side ends at the last music, not at the end of the tape.
    let side_end = m.analysis.music_end_frame.min(total_frames);

    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let min_gap_cells = ((cfg.min_gap_secs / cell_secs).ceil() as usize).max(1);
    let pre_roll_frames = secs_to_frames(cfg.pre_roll_secs, sample_rate);
    let min_track_frames = secs_to_frames(cfg.min_track_secs, sample_rate);

    // Only the needle-down span can hold a gap. Lead-in and run-out
    // are silence with no track on the far side of them, and a run
    // touching either end of the span is one of those.
    let mut candidates = Vec::new();
    let mut run_start: Option<usize> = None;
    for index in played.clone().chain(std::iter::once(played.end)) {
        let quiet = index < played.end && cells[index] < threshold_db;
        match (quiet, run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                run_start = None;
                if start == played.start || index == played.end || index - start < min_gap_cells {
                    continue;
                }
                let start_frame = (start as u64 * cell_frames).min(side_end);
                let end_frame = (index as u64 * cell_frames).min(side_end);
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
        if gap.boundary_frame == 0 || gap.boundary_frame >= side_end {
            continue;
        }
        if gap.boundary_frame - prev_boundary < min_track_frames {
            continue;
        }
        if side_end - gap.boundary_frame < min_track_frames {
            continue;
        }
        prev_boundary = gap.boundary_frame;
        kept.push(gap);
    }
    kept
}

/// Everything the threshold decision needs, measured once.
struct Measured {
    cells: Vec<f32>,
    cell_secs: f32,
    cell_frames: u64,
    /// Cell span the needle was down and playing music.
    played: Range<usize>,
    analysis: GapAnalysis,
}

fn measure(
    envelope: &[PeakChunk],
    frames_per_chunk: usize,
    sample_rate: u32,
    cfg: &GapConfig,
) -> Option<Measured> {
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
        return None;
    }

    let mut ranked = cells.clone();
    ranked.sort_by(f32::total_cmp);
    let music_db = percentile(&ranked, 0.80);

    // Two passes, because the floor and the region that defines it
    // are circular: the floor must be measured where the needle was
    // down, and finding the music needs a line drawn from the floor.
    //
    // Pass 1 keeps a lifted stylus out. A capture usually ends with
    // the needle on the rest, and that preamp hiss sits ~20 dB below
    // groove noise — taking it as the floor puts the gap line under
    // every real gap on the side. 20 dB under the music still counts
    // as playing here: this only has to tell "record under the
    // stylus" from a lifted one, not music from silence.
    let coarse = music_span(&cells, music_db - 20.0).unwrap_or(0..cells.len());
    let coarse_floor = floor_of(&cells[coarse.clone()]);

    // Pass 2 keeps the run-out out, which pass 1 cannot: run-out
    // groove noise on a worn side sits *within* 20 dB of the music
    // (measured at 20.4 dB on a soul sampler), so it reads as
    // needle-down and drags the floor, invents gaps, and rides along
    // on the last track. Re-cut the region at the first and last
    // genuine music instead.
    let played = music_span(&cells, coarse_floor + cfg.margin_db).unwrap_or(coarse);
    let floor_db = floor_of(&cells[played.clone()]);
    #[allow(clippy::cast_precision_loss)]
    let played_secs = played.len() as f32 * cell_secs;

    Some(Measured {
        analysis: GapAnalysis {
            floor_db,
            music_db,
            threshold_db: floor_db + cfg.margin_db,
            played_secs,
            music_start_frame: played.start as u64 * cell_frames,
            music_end_frame: played.end as u64 * cell_frames,
            usable: music_db - floor_db >= cfg.min_contrast_db,
        },
        cells,
        cell_secs,
        cell_frames,
        played,
    })
}

/// The `FLOOR_RANK_CELLS`-th quietest cell of a span, in dBFS.
fn floor_of(cells: &[f32]) -> f32 {
    if cells.is_empty() {
        return -140.0;
    }
    let mut sorted = cells.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[FLOOR_RANK_CELLS.min(sorted.len() - 1)]
}

/// First..last cell above `line` with at least half of the ~1 s
/// around it also above — the span of *sustained* signal.
///
/// The vote is what separates signal from debris, and both passes
/// need it. A worn run-out ticks over the gap line for a cell or two
/// at a time: measured on a drum'n'bass 45, a single such tick 4 s
/// into the run-out ended the run-out as far as a bare threshold
/// could tell, and the 93 s behind it committed as a second track.
/// The same lone tick out on the lifted-needle tail, where a bare
/// threshold stretched the needle-down span to the end of the
/// capture, dropped the floor estimate 12 dB into preamp hiss and
/// took a real gap on a reggae side with it.
fn music_span(cells: &[f32], line: f32) -> Option<Range<usize>> {
    let voted = |index: usize| {
        let lo = index.saturating_sub(MUSIC_VOTE_CELLS);
        let hi = (index + MUSIC_VOTE_CELLS + 1).min(cells.len());
        let window = &cells[lo..hi];
        window.iter().filter(|&&db| db >= line).count() * 2 > window.len()
    };
    let first = (0..cells.len()).find(|&i| voted(i))?;
    let last = (first..cells.len()).rfind(|&i| voted(i))?;
    Some(first..last + 1)
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

    /// rms for a dBFS level.
    fn at(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    /// A quiet stretch shaped like a real one: mostly groove noise
    /// with crackle riding over it, one cell in five.
    ///
    /// Flat runs are what the first fixtures used and they model the
    /// wrong thing. What decides a gap is its *loudest* cell, and
    /// what sets the floor is its quietest — on a soul sampler those
    /// were 8 dB apart inside the same 1.5 s (a −52 median under a
    /// −44 worst). A flat run collapses the two and makes every
    /// threshold look further from the edge than it is.
    fn quiet(worst_db: f32, groove_db: f32, secs: f32) -> Vec<(f32, f32)> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cells = (secs / 0.05).round() as usize;
        (0..cells)
            .map(|i| {
                (
                    if i % 5 == 2 {
                        at(worst_db)
                    } else {
                        at(groove_db)
                    },
                    0.05,
                )
            })
            .collect()
    }

    /// Flatten `(rms, secs)` runs and quiet stretches into one side.
    fn side(parts: &[Vec<(f32, f32)>]) -> Vec<PeakChunk> {
        env(&parts.iter().flatten().copied().collect::<Vec<_>>())
    }

    fn music(db: f32, secs: f32) -> Vec<(f32, f32)> {
        vec![(at(db), secs)]
    }

    /// **Record A** — a 4-track reggae 12" (SL 3 + phono chain,
    /// 48 kHz): music at −18 dBFS, inter-track gaps whose loudest
    /// cells run −38 to −40.3, groove noise at −57, and 60 s of
    /// run-out at the end.
    ///
    /// This is the case the first cut of the detector got wrong. It
    /// set the line at floor + 8 dB = −50 and found nothing at all,
    /// because a gap between tracks is ~18 dB noisier than the
    /// quietest groove: dust, wear, and the tail of the last tune.
    /// The shallowest gap here is −38.0, which is what fixes the
    /// margin's upper end: 23 dB of margin splits this side into
    /// five.
    #[test]
    fn record_a_reggae_12_yields_its_four_tracks() {
        let e = side(&[
            quiet(-45.0, -57.0, 4.0), // lead-in groove
            music(-18.0, 193.0),
            quiet(-40.0, -57.0, 1.6),
            music(-18.0, 165.0),
            quiet(-40.3, -57.0, 2.8),
            music(-18.0, 157.0),
            quiet(-38.0, -57.0, 1.7), // the shallowest gap on the side
            music(-18.0, 105.0),
            quiet(-40.0, -52.0, 60.0), // run-out
        ]);
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());

        assert_eq!(gaps.len(), 3, "expected three inter-track gaps: {gaps:?}");
        let mins = |f: u64| secs_of(f) / 60.0;
        assert!((mins(gaps[0].boundary_frame) - 3.30).abs() < 0.1);
        assert!((mins(gaps[1].boundary_frame) - 6.11).abs() < 0.1);
        assert!((mins(gaps[2].boundary_frame) - 8.77).abs() < 0.1);
    }

    /// **Record B** — a 10-track soul sampler, and the side that
    /// retired the `music − 18 dB` cap.
    ///
    /// It is cut quiet (music at −25.3 dBFS against record A's −18)
    /// while its gaps sit at −38 to −44, so a line drawn 18 dB under
    /// the music landed at −43.2 — under eight of its nine gaps. The
    /// detector proposed one boundary and committed the record as two
    /// tracks. Drawn from the floor instead, at −58.4 + 21, the line
    /// lands at −37.4 and every gap clears it.
    #[test]
    fn record_b_quiet_soul_sampler_yields_its_ten_tracks() {
        // The nine gaps as measured, loudest cell each.
        let worst = [
            -44.2, -43.2, -42.7, -43.0, -41.6, -38.1, -38.1, -40.1, -38.9,
        ];
        let lengths = [7.7, 8.3, 4.6, 9.2, 2.9, 2.2, 1.9, 4.9, 1.7];
        let tracks = [
            235.0, 179.0, 195.0, 172.0, 299.0, 224.0, 176.0, 227.0, 216.0,
        ];

        let mut parts = vec![quiet(-46.0, -58.4, 11.0)]; // lead-in
        for i in 0..9 {
            parts.push(music(-25.3, tracks[i]));
            parts.push(quiet(worst[i], -58.4, lengths[i]));
        }
        parts.push(music(-25.3, 198.0)); // track 10
        parts.push(quiet(-45.0, -58.4, 136.0)); // run-out
        let e = side(&parts);

        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert_eq!(gaps.len(), 9, "expected nine inter-track gaps: {gaps:?}");

        // Every proposal has to leave a plausible track behind it.
        let mut prev = 0.0_f32;
        for gap in &gaps {
            let at = secs_of(gap.boundary_frame);
            assert!(
                at - prev > 100.0,
                "{:.1} s track before {at:.1} s",
                at - prev
            );
            prev = at;
        }
    }

    /// **Record C** — a drum'n'bass 45, one track a side, and the
    /// side that proved the run-out has to be cut off rather than
    /// merely ignored.
    ///
    /// Its music ends at 5:47 and the capture runs to 7:24. A single
    /// lead-out tick 4 s into that run-out split it into "gap, then
    /// signal", so the run-out stopped looking like the end of the
    /// side: the detector proposed a boundary at 5:51 and the
    /// remaining 93 s of groove noise committed as track 2.
    #[test]
    fn record_c_dnb_45_commits_one_track_and_drops_the_run_out() {
        let music_secs = 332.0;
        let e = side(&[
            quiet(-44.0, -53.0, 3.0), // lead-in
            music(-10.9, music_secs),
            quiet(-48.0, -53.0, 4.0),
            vec![(at(-30.0), 0.05)], // the lead-out tick
            quiet(-48.0, -53.0, 93.0),
        ]);

        let cfg = GapConfig::default();
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &cfg);
        assert!(gaps.is_empty(), "run-out proposed as a track: {gaps:?}");

        // And the side ends at the music, not at the end of the tape.
        let a = analyze(&e, FPC, SR, &cfg);
        let end = secs_of(a.music_end_frame);
        assert!(
            (end - (music_secs + 3.0)).abs() < 2.0,
            "side ends at {end:.1} s, music ends at {:.1}",
            music_secs + 3.0
        );
    }

    /// A tick out on the lifted-needle tail must not drag the floor
    /// estimate into preamp hiss.
    ///
    /// Bounding the needle-down span by the outermost cell above a
    /// line — rather than by the outermost *sustained* one — let a
    /// single pop 10 minutes into the silence after the record
    /// stretch the span to the end of the capture. The floor then
    /// read −70 instead of −57, the gap line fell 13 dB, and a real
    /// gap on record A went missing.
    #[test]
    fn a_tick_on_the_lifted_needle_tail_does_not_move_the_floor() {
        let e = side(&[
            music(-18.0, 120.0),
            quiet(-40.0, -57.0, 2.0),
            music(-18.0, 120.0),
            quiet(-52.0, -70.0, 300.0), // stylus on the rest
            vec![(at(-35.0), 0.05)],    // a knock against the deck
            quiet(-52.0, -70.0, 60.0),
        ]);

        let a = analyze(&e, FPC, SR, &GapConfig::default());
        assert!(
            a.floor_db > -62.0,
            "floor read the lifted-needle hiss: {:.1} dBFS",
            a.floor_db
        );
        let gaps = detect_gaps(&e, FPC, SR, total(&e), &GapConfig::default());
        assert_eq!(gaps.len(), 1, "expected the one real gap: {gaps:?}");
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
