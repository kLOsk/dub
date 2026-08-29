//! Split plan: track boundaries over a recorded side + per-segment
//! metadata. Pure data + validation — the capture worker and the
//! commit path both consume this, and the M26b auto gap detector
//! will *produce* one.

use std::ops::Range;

/// Shortest segment a split plan may produce, in seconds. Manual
/// splits stay permissive (reggae 7"s carry sub-minute skits and
/// version snippets); the M26b auto-detector applies its own, much
/// larger, minimum on top.
pub const MIN_SEGMENT_SECS: f32 = 5.0;

/// Per-segment metadata, filled manually in M26a (recognition fills
/// it in M26c). All fields optional: a rip commits fine untagged and
/// gets `filename`-source metadata in the library like any other
/// import.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrackMeta {
    /// Track title.
    pub title: Option<String>,
    /// Track artist.
    pub artist: Option<String>,
    /// Release / album title (usually shared across the side).
    pub album: Option<String>,
    /// Release year.
    pub year: Option<i32>,
    /// Genre (also seeds the analysis octave profile downstream).
    pub genre: Option<String>,
}

/// Errors from [`validate_boundaries`].
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SplitError {
    #[error("boundaries must be strictly increasing (index {index})")]
    NotIncreasing { index: usize },

    #[error("boundary {frame} outside the recording (0..{total_frames})")]
    OutOfRange { frame: u64, total_frames: u64 },

    #[error("segment {index} is {actual_secs:.2} s, shorter than the {min_secs} s minimum")]
    SegmentTooShort {
        index: usize,
        actual_secs: f32,
        min_secs: f32,
    },

    #[error("recording is empty; nothing to split")]
    EmptyRecording,
}

/// Validate a set of split boundaries (in frames) against the side
/// `start_frame..total_frames`. Boundaries are the frame where the
/// *next* track starts; N boundaries make N+1 segments. An empty
/// boundary list is valid (the whole side is one track).
///
/// `start_frame` is normally 0; it is the detected start of the side
/// once the lead-in groove has been trimmed off, and a boundary
/// inside the discarded lead-in is out of range like any other.
pub fn validate_boundaries(
    boundaries: &[u64],
    start_frame: u64,
    total_frames: u64,
    sample_rate: u32,
    min_segment_secs: f32,
) -> Result<(), SplitError> {
    if total_frames == 0 || start_frame >= total_frames {
        return Err(SplitError::EmptyRecording);
    }
    let min_frames = min_segment_frames(sample_rate, min_segment_secs);
    let mut prev = start_frame;
    for (index, &frame) in boundaries.iter().enumerate() {
        if frame <= start_frame || frame >= total_frames {
            return Err(SplitError::OutOfRange {
                frame,
                total_frames,
            });
        }
        if frame <= prev && index > 0 {
            return Err(SplitError::NotIncreasing { index });
        }
        let len = frame - prev;
        if len < min_frames {
            return Err(SplitError::SegmentTooShort {
                index,
                actual_secs: frames_to_secs(len, sample_rate),
                min_secs: min_segment_secs,
            });
        }
        prev = frame;
    }
    let tail = total_frames - prev;
    if tail < min_frames {
        return Err(SplitError::SegmentTooShort {
            index: boundaries.len(),
            actual_secs: frames_to_secs(tail, sample_rate),
            min_secs: min_segment_secs,
        });
    }
    Ok(())
}

/// Derive the per-segment frame ranges from a validated boundary
/// list. N boundaries → N+1 half-open ranges covering the whole
/// recording with no gaps and no overlap.
#[must_use]
pub fn segments(boundaries: &[u64], start_frame: u64, end_frame: u64) -> Vec<Range<u64>> {
    let mut out = Vec::with_capacity(boundaries.len() + 1);
    let mut prev = start_frame;
    for &b in boundaries {
        out.push(prev..b);
        prev = b;
    }
    out.push(prev..end_frame.max(prev));
    out
}

fn min_segment_frames(sample_rate: u32, min_segment_secs: f32) -> u64 {
    // Truncation is fine: sub-sample precision is meaningless here.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (f64::from(sample_rate) * f64::from(min_segment_secs)) as u64;
    frames
}

#[allow(clippy::cast_precision_loss)]
fn frames_to_secs(frames: u64, sample_rate: u32) -> f32 {
    if sample_rate == 0 {
        return 0.0;
    }
    (frames as f64 / f64::from(sample_rate)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SR: u32 = 48_000;
    const MIN: f32 = MIN_SEGMENT_SECS;

    fn secs(s: u64) -> u64 {
        s * u64::from(SR)
    }

    #[test]
    fn empty_boundary_list_is_one_whole_side_segment() {
        validate_boundaries(&[], 0, secs(60), SR, MIN).unwrap();
        let segs = segments(&[], 0, secs(60));
        assert_eq!(segs, vec![0..secs(60)]);
    }

    #[test]
    fn three_track_side_validates_and_derives_ranges() {
        let bounds = [secs(180), secs(390)];
        validate_boundaries(&bounds, 0, secs(600), SR, MIN).unwrap();
        let segs = segments(&bounds, 0, secs(600));
        assert_eq!(
            segs,
            vec![0..secs(180), secs(180)..secs(390), secs(390)..secs(600)]
        );
    }

    /// A trimmed side starts after frame 0: the first segment runs
    /// from the side start, and a boundary inside the discarded
    /// lead-in is out of range like one past the end.
    #[test]
    fn a_trimmed_side_starts_at_its_side_start() {
        let bounds = [secs(200)];
        validate_boundaries(&bounds, secs(10), secs(400), SR, MIN).unwrap();
        assert_eq!(
            segments(&bounds, secs(10), secs(400)),
            vec![secs(10)..secs(200), secs(200)..secs(400)]
        );
        assert!(matches!(
            validate_boundaries(&[secs(5)], secs(10), secs(400), SR, MIN),
            Err(SplitError::OutOfRange { .. })
        ));
        // And the first segment is measured from the side start, not
        // from zero — 3 s of track behind a 10 s lead-in is too short.
        assert!(matches!(
            validate_boundaries(&[secs(13)], secs(10), secs(400), SR, MIN),
            Err(SplitError::SegmentTooShort { index: 0, .. })
        ));
    }

    #[test]
    fn rejects_empty_recording() {
        assert_eq!(
            validate_boundaries(&[], 0, 0, SR, MIN),
            Err(SplitError::EmptyRecording)
        );
    }

    #[test]
    fn rejects_boundary_at_zero_or_past_end() {
        assert!(matches!(
            validate_boundaries(&[0], 0, secs(60), SR, MIN),
            Err(SplitError::OutOfRange { .. })
        ));
        assert!(matches!(
            validate_boundaries(&[secs(60)], 0, secs(60), SR, MIN),
            Err(SplitError::OutOfRange { .. })
        ));
    }

    #[test]
    fn rejects_non_increasing_boundaries() {
        let err = validate_boundaries(&[secs(30), secs(30)], 0, secs(90), SR, MIN).unwrap_err();
        assert!(matches!(err, SplitError::NotIncreasing { index: 1 }));
    }

    #[test]
    fn rejects_too_short_first_middle_and_tail_segments() {
        // First segment 2 s.
        assert!(matches!(
            validate_boundaries(&[secs(2)], 0, secs(60), SR, MIN),
            Err(SplitError::SegmentTooShort { index: 0, .. })
        ));
        // Middle segment 3 s.
        assert!(matches!(
            validate_boundaries(&[secs(10), secs(13)], 0, secs(60), SR, MIN),
            Err(SplitError::SegmentTooShort { index: 1, .. })
        ));
        // Tail segment 1 s.
        assert!(matches!(
            validate_boundaries(&[secs(59)], 0, secs(60), SR, MIN),
            Err(SplitError::SegmentTooShort { index: 1, .. })
        ));
    }

    proptest! {
        /// Any boundary list the validator accepts must produce
        /// segments that tile the recording exactly: cover every
        /// frame, in order, without overlap, each ≥ the minimum.
        #[test]
        fn accepted_boundaries_tile_the_recording(
            // Up to 8 cuts over a side up to ~30 min.
            raw in proptest::collection::vec(1_u64..86_400_000, 0..8),
            total in 480_000_u64..86_400_000,
        ) {
            let mut bounds: Vec<u64> = raw.into_iter().filter(|&b| b < total).collect();
            bounds.sort_unstable();
            bounds.dedup();
            if validate_boundaries(&bounds, 0, total, SR, MIN).is_ok() {
                let segs = segments(&bounds, 0, total);
                prop_assert_eq!(segs.len(), bounds.len() + 1);
                prop_assert_eq!(segs[0].start, 0);
                prop_assert_eq!(segs[segs.len() - 1].end, total);
                let min_frames = u64::from(SR) * 5;
                for w in segs.windows(2) {
                    prop_assert_eq!(w[0].end, w[1].start);
                }
                for s in &segs {
                    prop_assert!(s.end - s.start >= min_frames);
                }
            }
        }
    }
}
