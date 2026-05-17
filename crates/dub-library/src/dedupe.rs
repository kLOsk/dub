//! Version-aware dedupe decision per PRD §8.1.
//!
//! On import, when the candidate file's fingerprint is similar to an
//! existing track in the library, this module decides whether the
//! candidate is **the same recording** (auto-merge into the existing
//! `tracks` row, register a new `track_files` row pointing at the
//! same canonical track) or **a sibling version** of the same
//! recording (register as a separate `tracks` row with a
//! `duplicate_link_track_id` cross-reference, surfaced as a small
//! link glyph in the browser).
//!
//! Auto-merge fires only when **all** three conditions hold:
//!
//! 1. **Chromaprint similarity ≥ 0.98** (computed by
//!    [`dub_fingerprint::similarity`]).
//! 2. **Duration delta < 200 ms** between the candidate and the
//!    existing track's stored `fingerprints.duration_ms`.
//! 3. **No version-token mismatch** between the candidate and the
//!    existing track. A mismatch is any pair `(a, b)` where the
//!    parsed token sets are not equal; if both sets are empty, that
//!    is **not** a mismatch (the parser is allowed to miss tokens
//!    in unusual filename schemes, but if it sees one on one side
//!    and not the other, that is a real disqualifier).
//!
//! Per PRD §8.1: *"The cost of silently collapsing 'Clean' and
//! 'Dirty' into one row is 'the DJ played the explicit version at a
//! wedding'; we will not pay that cost."*

use std::collections::BTreeSet;

use dub_fingerprint::{similarity, Fingerprint};

use crate::version_tokens::{parse as parse_version_tokens, VersionToken};

/// Numeric threshold for Chromaprint similarity. PRD §8.1.
pub const SIMILARITY_THRESHOLD: f32 = 0.98;

/// Maximum allowed duration delta for auto-merge, in milliseconds.
/// PRD §8.1.
pub const DURATION_DELTA_MS: u32 = 200;

/// Outcome of the dedupe decision for one candidate against one
/// existing track row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupeDecision {
    /// Candidate is the same recording as the existing track. The
    /// caller should register the candidate's file as an additional
    /// `track_files` row against the existing `tracks.id`.
    Merge,
    /// Candidate is a distinct sibling version of the same
    /// recording. The caller should register the candidate as a
    /// new `tracks` row with `duplicate_link_track_id` pointing at
    /// the existing track.
    SiblingVersion {
        /// Reason for the refusal-to-merge, surfaced in the browser
        /// "potential duplicate" tooltip.
        reason: SiblingReason,
    },
    /// Candidate is a different recording entirely. Register as a
    /// new `tracks` row with no duplicate link.
    Distinct,
}

/// Why the dedupe decision refused to auto-merge a near-fingerprint-
/// match pair. Cardinality is small enough to enumerate; surfaces in
/// the browser tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiblingReason {
    /// Duration delta exceeded 200 ms.
    DurationDelta {
        /// Absolute duration delta in milliseconds.
        delta_ms: u32,
    },
    /// Version tokens differ between the two filenames / titles.
    VersionTokenMismatch {
        /// Tokens parsed from the candidate side.
        candidate_tokens: BTreeSet<VersionToken>,
        /// Tokens parsed from the existing-track side.
        existing_tokens: BTreeSet<VersionToken>,
    },
}

/// One side of the dedupe comparison: the candidate or the existing
/// track row.
#[derive(Debug, Clone)]
pub struct DedupeInput<'a> {
    /// The fingerprint to compare against.
    pub fingerprint: &'a Fingerprint,
    /// Duration in milliseconds. For the candidate, comes from the
    /// fingerprint compute step; for the existing track, comes from
    /// `fingerprints.duration_ms`.
    pub duration_ms: u32,
    /// Filename or title for version-token extraction. The caller
    /// may pass the filename, the ID3 title, or both joined; the
    /// parser tolerates either form.
    pub title_or_filename: &'a str,
}

/// Run the §8.1 dedupe decision. Pure function; no I/O. The caller
/// supplies the candidate (typically: the file being imported) and
/// the existing-track side (the row whose fingerprint was returned
/// as a near-match by the SQLite lookup).
pub fn decide(candidate: &DedupeInput<'_>, existing: &DedupeInput<'_>) -> DedupeDecision {
    let sim = similarity(candidate.fingerprint, existing.fingerprint);
    if sim < SIMILARITY_THRESHOLD {
        return DedupeDecision::Distinct;
    }

    // Fingerprint similarity says "same recording, allowing for
    // encoder + start-offset noise". From here, the disqualifiers
    // are about whether the user wants them merged at all.

    let delta_ms = candidate.duration_ms.abs_diff(existing.duration_ms);
    if delta_ms >= DURATION_DELTA_MS {
        return DedupeDecision::SiblingVersion {
            reason: SiblingReason::DurationDelta { delta_ms },
        };
    }

    let candidate_tokens = parse_version_tokens(candidate.title_or_filename);
    let existing_tokens = parse_version_tokens(existing.title_or_filename);
    if candidate_tokens != existing_tokens {
        return DedupeDecision::SiblingVersion {
            reason: SiblingReason::VersionTokenMismatch {
                candidate_tokens,
                existing_tokens,
            },
        };
    }

    DedupeDecision::Merge
}

#[cfg(test)]
mod tests {
    use super::*;
    use dub_fingerprint::Fingerprint;

    /// Generate a synthetic mono test tone (matches the fingerprint
    /// crate's tests) so we can build genuine fingerprints rather
    /// than handcraft `Fingerprint` instances. `compute_from_f32`
    /// is the only path that builds a `Fingerprint`; the dedupe
    /// tests must work against that public surface.
    fn tone(freq_hz: f32, sample_rate: u32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate as f32 * duration_secs) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / sample_rate as f32;
            out.push(0.5 * (2.0 * std::f32::consts::PI * freq_hz * t).sin());
        }
        out
    }

    fn make_fp(freq: f32, secs: f32) -> Fingerprint {
        let samples = tone(freq, 11025, secs);
        Fingerprint::compute_from_f32(&samples, 11025, 1).expect("compute")
    }

    #[test]
    fn distinct_recordings_register_as_distinct() {
        let a = make_fp(220.0, 12.0);
        let b = make_fp(1760.0, 12.0);
        let decision = decide(
            &DedupeInput {
                fingerprint: &a,
                duration_ms: 12_000,
                title_or_filename: "Track A.mp3",
            },
            &DedupeInput {
                fingerprint: &b,
                duration_ms: 12_000,
                title_or_filename: "Track B.mp3",
            },
        );
        assert_eq!(decision, DedupeDecision::Distinct);
    }

    #[test]
    fn identical_recordings_with_same_tokens_merge() {
        // Same file imported twice (same tone, same title, same
        // duration). Must auto-merge.
        let fp = make_fp(440.0, 12.0);
        let candidate = DedupeInput {
            fingerprint: &fp,
            duration_ms: 12_000,
            title_or_filename: "Lady.mp3",
        };
        let existing = DedupeInput {
            fingerprint: &fp,
            duration_ms: 12_000,
            title_or_filename: "Lady.mp3",
        };
        assert_eq!(decide(&candidate, &existing), DedupeDecision::Merge);
    }

    #[test]
    fn clean_vs_dirty_refuses_to_merge() {
        // The load-bearing test for PRD §8.1. Near-identical
        // fingerprints (we use the same one), same duration,
        // but a version-token mismatch.
        let fp = make_fp(440.0, 12.0);
        let decision = decide(
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady (Clean).mp3",
            },
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady (Dirty).mp3",
            },
        );
        match decision {
            DedupeDecision::SiblingVersion {
                reason: SiblingReason::VersionTokenMismatch { .. },
            } => {}
            other => panic!("expected VersionTokenMismatch, got {other:?}"),
        }
    }

    #[test]
    fn clean_vs_instrumental_refuses_to_merge() {
        let fp = make_fp(440.0, 12.0);
        let decision = decide(
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady (Clean).mp3",
            },
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady (Instrumental).mp3",
            },
        );
        assert!(matches!(
            decision,
            DedupeDecision::SiblingVersion {
                reason: SiblingReason::VersionTokenMismatch { .. }
            }
        ));
    }

    #[test]
    fn radio_edit_vs_extended_mix_refuses_to_merge() {
        // High-similarity fingerprints (same source), but the
        // radio-edit is structurally shorter than the extended mix;
        // either the duration disqualifies or the token does. We
        // assert the dedupe refuses, regardless of which gate fires.
        let fp_radio = make_fp(330.0, 4.0); // 4 s "radio edit"
        let fp_extended = make_fp(330.0, 8.0); // 8 s "extended mix"
        let decision = decide(
            &DedupeInput {
                fingerprint: &fp_radio,
                duration_ms: 4_000,
                title_or_filename: "Track (Radio Edit).mp3",
            },
            &DedupeInput {
                fingerprint: &fp_extended,
                duration_ms: 8_000,
                title_or_filename: "Track (Extended Mix).mp3",
            },
        );
        assert!(matches!(decision, DedupeDecision::SiblingVersion { .. }));
    }

    #[test]
    fn duration_delta_within_threshold_allows_merge() {
        // Same recording, two encodings that differ by ≈150 ms in
        // reported duration (within the 200 ms threshold). Same
        // title. Must merge.
        let fp = make_fp(440.0, 12.0);
        let decision = decide(
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady.mp3",
            },
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_150,
                title_or_filename: "Lady.mp3",
            },
        );
        assert_eq!(decision, DedupeDecision::Merge);
    }

    #[test]
    fn duration_delta_at_threshold_refuses_merge() {
        let fp = make_fp(440.0, 12.0);
        let decision = decide(
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady.mp3",
            },
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_200,
                title_or_filename: "Lady.mp3",
            },
        );
        match decision {
            DedupeDecision::SiblingVersion {
                reason: SiblingReason::DurationDelta { delta_ms },
            } => assert_eq!(delta_ms, 200),
            other => panic!("expected DurationDelta, got {other:?}"),
        }
    }

    #[test]
    fn missing_tokens_on_both_sides_is_not_a_mismatch() {
        // The parser misses tokens on unusual filenames; that's not
        // a dedupe-disqualifier as long as both sides agree (both
        // empty sets). High-similarity fingerprint + matching
        // duration + both-empty token sets must merge.
        let fp = make_fp(440.0, 12.0);
        let decision = decide(
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Donuts.mp3",
            },
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Donuts.mp3",
            },
        );
        assert_eq!(decision, DedupeDecision::Merge);
    }

    #[test]
    fn one_sided_token_refuses_to_merge() {
        // If "Lady.mp3" is in the library, importing "Lady (Clean).mp3"
        // must refuse to merge — the user is telling us this is the
        // clean cut, and the existing row has no claim on that fact.
        let fp = make_fp(440.0, 12.0);
        let decision = decide(
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady (Clean).mp3",
            },
            &DedupeInput {
                fingerprint: &fp,
                duration_ms: 12_000,
                title_or_filename: "Lady.mp3",
            },
        );
        assert!(matches!(
            decision,
            DedupeDecision::SiblingVersion {
                reason: SiblingReason::VersionTokenMismatch { .. }
            }
        ));
    }
}
