//! Beat-location evaluation metrics.
//!
//! dub's regression corpus (`tests/real_music_corpus.rs`) scores *tempo*
//! only — "is the reported BPM within 5 %." That cannot see a phase or
//! downbeat regression: a grid can carry the right BPM and still place
//! every beat on the off-beat. The survey in
//! `docs/investigations/BPM-DETECTOR-V2-INVESTIGATION.md` §7.4 flagged this
//! as the prerequisite for evaluating any grid-placement change (the
//! backbeat downbeat rule, a DP beat decoder, …): there is no
//! beat-*location* metric to measure them with.
//!
//! This module supplies the standard ones, operating on two slices of beat
//! times in seconds (a detected grid and a ground-truth annotation):
//!
//! * [`f_measure`] — windowed precision/recall (±[`F_MEASURE_TOL`] s). The
//!   primary, unambiguous phase metric.
//! * [`cemgil`] — Gaussian-weighted accuracy (σ = [`CEMGIL_SIGMA`] s); a
//!   smooth alternative to the hard F-measure window.
//! * [`continuity`] — CMLt/CMLc (correct at the annotated metrical level)
//!   and AMLt/AMLc (allowing double / half / off-beat interpretations).
//!   These separate a true regression from a mere octave/offbeat shift.
//!
//! The definitions follow Davies et al., *Evaluation Methods for Musical
//! Audio Beat Tracking Algorithms* (the same family `mir_eval.beat`
//! implements). They are pure functions with no audio or analysis
//! dependency, so they can grade any detector — `analyze_beat_grid`'s
//! output, an imported grid, or an external tracker's beats.

/// Default F-measure tolerance window (seconds): ±70 ms, the MIREX
/// convention.
pub const F_MEASURE_TOL: f64 = 0.07;
/// Default Cemgil Gaussian width (seconds): 40 ms.
pub const CEMGIL_SIGMA: f64 = 0.04;
/// Default continuity phase tolerance, as a fraction of the local
/// inter-annotation interval (17.5 %).
pub const CONTINUITY_PHASE_TOL: f64 = 0.175;
/// Default continuity period tolerance, as a fraction (17.5 %).
pub const CONTINUITY_PERIOD_TOL: f64 = 0.175;

/// Windowed beat F-measure: a detected beat counts as a true positive when
/// it lies within `tol` seconds of an annotation, matched one-to-one.
///
/// `F = 2·TP / (2·TP + FP + FN)`. Returns `0.0` when either side is empty.
/// Both inputs are sorted internally, so callers need not pre-sort.
#[must_use]
pub fn f_measure(detected: &[f64], annotated: &[f64], tol: f64) -> f64 {
    if detected.is_empty() || annotated.is_empty() {
        return 0.0;
    }
    let det = sorted(detected);
    let ann = sorted(annotated);
    let tp = count_matches(&det, &ann, tol);
    #[allow(clippy::cast_precision_loss)]
    let denom = (det.len() + ann.len()) as f64;
    2.0 * tp as f64 / denom
}

/// Cemgil accuracy: for each annotation, a Gaussian of the distance to the
/// nearest detected beat, normalized by the mean of the two beat counts.
///
/// Smoothly rewards near-misses where [`f_measure`]'s hard window would
/// score 0. Returns `0.0` when either side is empty.
#[must_use]
pub fn cemgil(detected: &[f64], annotated: &[f64], sigma: f64) -> f64 {
    if detected.is_empty() || annotated.is_empty() || sigma <= 0.0 {
        return 0.0;
    }
    let det = sorted(detected);
    let two_sigma_sq = 2.0 * sigma * sigma;
    let sum: f64 = annotated
        .iter()
        .map(|&a| {
            let d = nearest_distance(&det, a);
            (-d * d / two_sigma_sq).exp()
        })
        .sum();
    #[allow(clippy::cast_precision_loss)]
    let denom = 0.5 * (annotated.len() + det.len()) as f64;
    sum / denom
}

/// Continuity-based accuracy.
///
/// `cmlt`/`cmlc` score against the annotation at its own metrical level;
/// `amlt`/`amlc` take the best score over the annotation and its
/// double-tempo, half-tempo, and off-beat variants, so a consistent
/// octave/off-beat interpretation is not punished. The `*c` variants use
/// the longest *continuous* run of correctly-tracked beats; the `*t` (total)
/// variants count all correct beats, allowing the tracker to recover after
/// a slip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Continuity {
    /// Correct, annotated level, longest continuous segment.
    pub cmlc: f64,
    /// Correct, annotated level, total.
    pub cmlt: f64,
    /// Allowed metrical levels, longest continuous segment.
    pub amlc: f64,
    /// Allowed metrical levels, total.
    pub amlt: f64,
}

/// Compute the [`Continuity`] metrics of `detected` against `annotated`.
#[must_use]
pub fn continuity(
    detected: &[f64],
    annotated: &[f64],
    phase_tol: f64,
    period_tol: f64,
) -> Continuity {
    let det = sorted(detected);
    let ann = sorted(annotated);

    let (base_c, base_t) = continuity_level(&det, &ann, phase_tol, period_tol);

    let mut amlc = base_c;
    let mut amlt = base_t;
    for variant in annotation_variants(&ann) {
        let (c, t) = continuity_level(&det, &variant, phase_tol, period_tol);
        amlc = amlc.max(c);
        amlt = amlt.max(t);
    }

    Continuity {
        cmlc: base_c,
        cmlt: base_t,
        amlc,
        amlt,
    }
}

/// One-to-one greedy count of `a` entries within `tol` of a `b` entry.
/// Both slices must be sorted ascending. Correct when `tol` is below half
/// the smallest beat interval (always true for ±70 ms vs ≥ 300 ms beats).
fn count_matches(a: &[f64], b: &[f64], tol: f64) -> usize {
    let (mut i, mut j, mut m) = (0usize, 0usize, 0usize);
    while i < a.len() && j < b.len() {
        let d = a[i] - b[j];
        if d.abs() <= tol {
            m += 1;
            i += 1;
            j += 1;
        } else if d < 0.0 {
            i += 1;
        } else {
            j += 1;
        }
    }
    m
}

/// Continuity-correct fraction of `detected` against one reference level.
/// Returns `(longest_continuous, total)`, each already normalized to
/// `[0, 1]` by `max(len(detected), len(reference))`.
fn continuity_level(
    detected: &[f64],
    reference: &[f64],
    phase_tol: f64,
    period_tol: f64,
) -> (f64, f64) {
    if detected.len() < 2 || reference.len() < 2 {
        return (0.0, 0.0);
    }
    let mut correct = vec![false; detected.len()];

    for i in 0..detected.len() {
        let k = nearest_index(reference, detected[i]);
        let ref_int = if k == 0 {
            reference[1] - reference[0]
        } else {
            reference[k] - reference[k - 1]
        };
        if ref_int <= 0.0 {
            continue;
        }
        let phase = (detected[i] - reference[k]).abs() < phase_tol * ref_int;

        if i == 0 {
            // No predecessor: the first beat is correct on phase alone, so
            // a perfectly-tracked grid scores 1.0.
            correct[i] = phase;
            continue;
        }

        // Predecessor must align to the immediately preceding reference
        // beat, and the detected interval must match the reference interval.
        let phase_prev = k > 0 && (detected[i - 1] - reference[k - 1]).abs() < phase_tol * ref_int;
        let det_int = detected[i] - detected[i - 1];
        let period = (1.0 - det_int / ref_int).abs() < period_tol;
        correct[i] = phase && phase_prev && period;
    }

    let mut longest = 0usize;
    let mut run = 0usize;
    let mut total = 0usize;
    for &c in &correct {
        if c {
            run += 1;
            total += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let n = detected.len().max(reference.len()) as f64;
    #[allow(clippy::cast_precision_loss)]
    {
        (longest as f64 / n, total as f64 / n)
    }
}

/// Double-tempo, off-beat, and the two half-tempo phasings of `reference`,
/// for the AML (allowed-metrical-level) family.
fn annotation_variants(reference: &[f64]) -> Vec<Vec<f64>> {
    let mut variants = Vec::new();
    if reference.len() < 2 {
        return variants;
    }

    // Double tempo: interleave the midpoints.
    let mut double = Vec::with_capacity(reference.len() * 2);
    for w in reference.windows(2) {
        double.push(w[0]);
        double.push(0.5 * (w[0] + w[1]));
    }
    double.push(*reference.last().expect("len >= 2"));
    variants.push(double);

    // Off-beat: shift each beat by half its forward interval.
    let mut offbeat = Vec::with_capacity(reference.len());
    for w in reference.windows(2) {
        offbeat.push(0.5 * (w[0] + w[1]));
    }
    variants.push(offbeat);

    // Half tempo, both phasings.
    variants.push(reference.iter().copied().step_by(2).collect());
    variants.push(reference.iter().copied().skip(1).step_by(2).collect());

    variants
}

/// Index of the nearest entry in a sorted slice.
fn nearest_index(beats: &[f64], x: f64) -> usize {
    match beats.binary_search_by(|b| b.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(i) => i,
        Err(i) => {
            if i == 0 {
                0
            } else if i >= beats.len() {
                beats.len() - 1
            } else if (x - beats[i - 1]).abs() <= (beats[i] - x).abs() {
                i - 1
            } else {
                i
            }
        }
    }
}

/// Absolute distance from `x` to the nearest entry in a sorted slice.
fn nearest_distance(beats: &[f64], x: f64) -> f64 {
    if beats.is_empty() {
        return f64::INFINITY;
    }
    let i = nearest_index(beats, x);
    (beats[i] - x).abs()
}

/// Sorted ascending copy.
fn sorted(beats: &[f64]) -> Vec<f64> {
    let mut v = beats.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A uniform beat sequence: `n` beats at `period` s from `offset`.
    fn beats(n: usize, period: f64, offset: f64) -> Vec<f64> {
        #[allow(clippy::cast_precision_loss)]
        (0..n).map(|i| offset + i as f64 * period).collect()
    }

    #[test]
    fn f_measure_perfect_is_one() {
        let b = beats(50, 0.5, 0.0);
        assert!((f_measure(&b, &b, F_MEASURE_TOL) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn f_measure_offbeat_is_zero() {
        let ann = beats(50, 0.5, 0.0);
        let det = beats(50, 0.5, 0.25); // half a beat off
        assert!(f_measure(&det, &ann, F_MEASURE_TOL) < 1e-9);
    }

    #[test]
    fn f_measure_within_window_counts() {
        let ann = beats(50, 0.5, 0.0);
        let det = beats(50, 0.5, 0.05); // 50 ms off, inside ±70 ms
        assert!((f_measure(&det, &ann, F_MEASURE_TOL) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn f_measure_empty_is_zero() {
        let b = beats(10, 0.5, 0.0);
        assert_eq!(f_measure(&[], &b, F_MEASURE_TOL), 0.0);
        assert_eq!(f_measure(&b, &[], F_MEASURE_TOL), 0.0);
    }

    #[test]
    fn f_measure_double_tempo_is_two_thirds() {
        // Detected has 2× the beats; every annotation matches but half the
        // detections are spurious. P = 0.5, R = 1 → F = 2/3.
        let ann = beats(25, 0.5, 0.0);
        let det = beats(50, 0.25, 0.0);
        let f = f_measure(&det, &ann, F_MEASURE_TOL);
        assert!((f - 2.0 / 3.0).abs() < 0.02, "expected ~0.667, got {f}");
    }

    #[test]
    fn cemgil_perfect_is_one() {
        let b = beats(40, 0.5, 0.0);
        assert!((cemgil(&b, &b, CEMGIL_SIGMA) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cemgil_decays_with_offset() {
        let ann = beats(40, 0.5, 0.0);
        let near = cemgil(&beats(40, 0.5, 0.02), &ann, CEMGIL_SIGMA);
        let far = cemgil(&beats(40, 0.5, 0.06), &ann, CEMGIL_SIGMA);
        assert!(
            near > far,
            "closer beats must score higher: {near} vs {far}"
        );
        assert!(near < 1.0 && far > 0.0);
    }

    #[test]
    fn continuity_perfect_is_all_ones() {
        let b = beats(50, 0.5, 0.0);
        let c = continuity(&b, &b, CONTINUITY_PHASE_TOL, CONTINUITY_PERIOD_TOL);
        assert!((c.cmlt - 1.0).abs() < 1e-9, "cmlt {}", c.cmlt);
        assert!((c.cmlc - 1.0).abs() < 1e-9, "cmlc {}", c.cmlc);
        assert!((c.amlt - 1.0).abs() < 1e-9, "amlt {}", c.amlt);
    }

    #[test]
    fn continuity_offbeat_splits_cml_and_aml() {
        // Off-beat tracking: wrong at the annotated level (cmlt≈0) but a
        // valid metrical interpretation (amlt≈1).
        let ann = beats(50, 0.5, 0.0);
        let det = beats(50, 0.5, 0.25);
        let c = continuity(&det, &ann, CONTINUITY_PHASE_TOL, CONTINUITY_PERIOD_TOL);
        assert!(c.cmlt < 0.1, "offbeat cmlt should be ~0, got {}", c.cmlt);
        assert!(c.amlt > 0.9, "offbeat amlt should be ~1, got {}", c.amlt);
    }

    #[test]
    fn continuity_half_tempo_splits_cml_and_aml() {
        // Detecting every other beat: wrong period at the annotated level
        // (cmlt low) but a valid half-tempo interpretation (amlt high).
        let ann = beats(50, 0.5, 0.0);
        let det = beats(25, 1.0, 0.0);
        let c = continuity(&det, &ann, CONTINUITY_PHASE_TOL, CONTINUITY_PERIOD_TOL);
        assert!(
            c.cmlt < 0.6,
            "half-tempo cmlt should be reduced, got {}",
            c.cmlt
        );
        assert!(c.amlt > 0.9, "half-tempo amlt should be ~1, got {}", c.amlt);
    }

    #[test]
    fn continuity_total_recovers_after_a_slip() {
        // Insert one spurious beat; cmlt (total) should stay high while
        // cmlc (longest run) drops because the run is broken in two.
        let ann = beats(40, 0.5, 0.0);
        let mut det = ann.clone();
        det.insert(20, 9.873); // a beat that lands off-grid
        let c = continuity(&det, &ann, CONTINUITY_PHASE_TOL, CONTINUITY_PERIOD_TOL);
        assert!(
            c.cmlt > c.cmlc,
            "total {} should exceed longest-run {}",
            c.cmlt,
            c.cmlc
        );
    }
}
