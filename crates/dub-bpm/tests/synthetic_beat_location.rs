//! End-to-end beat-*location* regression on synthetic genre patterns.
//!
//! The other suites score tempo only (`real_music_corpus`, `known_bpm`,
//! `genre_octave`) or downbeat *latching from a tap*
//! (`brown_paper_bag_set_the_one`). Nothing wired the synthetic
//! generators through `analyze_beat_grid` and scored the resulting beat
//! *positions* with the new `eval` metrics — so a phase slip or an
//! octave error in the auto grid had no synthetic guard. This adds one.
//!
//! Honest scope: the synthetic hip-hop / dnb patterns are *symmetric*
//! between bar positions 1 and 3 (kick on both), so they cannot test
//! the backbeat refinement's 1-vs-3 resolution — that needs the
//! asymmetry of real music (the `beat_location_corpus` path). Here we
//! assert what the synthetic ground truth *can* support — tempo +
//! beat-phase via F-measure — and print the auto/refined downbeat
//! placement as a diagnostic for inspection (no over-assertion on the
//! genuinely ambiguous quantity).

use dub_bpm::{analyze_beat_grid, eval, refine_downbeat_backbeat, synthetic};

const SR: u32 = 44_100;
const DUR: f64 = 24.0;

/// Ground-truth beat times for a pattern that starts its first bar at
/// t = 0: a beat every `60/bpm` seconds across the buffer.
fn truth_beats(bpm: f64, dur: f64) -> Vec<f64> {
    let beat = 60.0 / bpm;
    let n = (dur / beat) as usize;
    (0..n).map(|k| k as f64 * beat).collect()
}

/// Analyze a mono pattern and return (grid, F-measure vs ground truth).
fn analyze(samples: &[f32], bpm: f64) -> (dub_bpm::BeatGrid, f64) {
    let grid = analyze_beat_grid(samples, SR, 1).expect("grid must lock");
    let f = eval::f_measure(&grid.beats, &truth_beats(bpm, DUR), eval::F_MEASURE_TOL);
    (grid, f)
}

/// Distance from a detected downbeat time to the nearest true bar-1
/// (true bars start at multiples of `4 × 60/bpm`), in ms.
fn downbeat_err_ms(downbeat_secs: f64, bpm: f64) -> f64 {
    let bar = 4.0 * 60.0 / bpm;
    let phase = (downbeat_secs / bar).fract();
    let d = phase.min(1.0 - phase); // wrap to nearest bar boundary
    d * bar * 1000.0
}

fn report_downbeat(name: &str, grid: &dub_bpm::BeatGrid, samples: &[f32], bpm: f64) {
    let auto = grid.beats[grid.bar_phase as usize];
    let auto_err = downbeat_err_ms(auto, bpm);
    let refined = refine_downbeat_backbeat(samples, SR, 1, grid);
    let refined_str = refined.as_ref().map_or_else(
        || "none".to_string(),
        |r| {
            let db = grid.beats[r.bar_phase as usize];
            format!(
                "bar_phase={} downbeat={:.3}s (Δbar1={:.0}ms, conf={:.2})",
                r.bar_phase,
                db,
                downbeat_err_ms(db, bpm),
                r.confidence
            )
        },
    );
    println!(
        "  [{name}] bpm={:.2} conf={:.2} | auto bar_phase={} downbeat={:.3}s (Δbar1={:.0}ms) | refined: {refined_str}",
        grid.bpm, grid.confidence, grid.bar_phase, auto, auto_err
    );
}

#[test]
fn hip_hop_beat_phase_locks() {
    let bpm = 93.0;
    let s = synthetic::drum_pattern_hip_hop(bpm, DUR, SR);
    let (grid, f) = analyze(&s, bpm);
    report_downbeat("hip-hop", &grid, &s, bpm);
    assert!(
        f > 0.9,
        "hip-hop beat phase/tempo regressed: F-measure = {f:.3} (want > 0.9)"
    );
}

#[test]
fn dnb_rolling_beat_phase_locks() {
    let bpm = 174.0;
    let s = synthetic::drum_pattern_drum_n_bass(bpm, DUR, SR);
    let (grid, f) = analyze(&s, bpm);
    report_downbeat("dnb", &grid, &s, bpm);
    assert!(
        f > 0.9,
        "rolling dnb beat phase/tempo regressed: F-measure = {f:.3} (want > 0.9)"
    );
}

/// One-drop is the adversarial case: beat 1 is silent, the "drop"
/// (kick + cross-stick) is on beat 3, the only steady pulse is the
/// off-beat skank. Diagnostic only — we report the metrics so a
/// regression in how the rebuilt detector + downbeat method handle the
/// hardest reggae case is visible, without asserting a number the
/// synthetic ground truth can't justify.
#[test]
fn one_drop_diagnostic() {
    let bpm = 75.0;
    let s = synthetic::drum_pattern_reggae_one_drop(bpm, DUR, SR);
    let grid = analyze_beat_grid(&s, SR, 1).expect("grid must lock");
    let f = eval::f_measure(&grid.beats, &truth_beats(bpm, DUR), eval::F_MEASURE_TOL);
    let cont = eval::continuity(
        &grid.beats,
        &truth_beats(bpm, DUR),
        eval::CONTINUITY_PHASE_TOL,
        eval::CONTINUITY_PERIOD_TOL,
    );
    println!("  [one-drop DIAGNOSTIC] F-measure={f:.3} continuity={cont:?}");
    report_downbeat("one-drop", &grid, &s, bpm);
    // The synthetic one-drop's only continuous pulse is the off-beat
    // skank, so an on-beat F-measure of ~0 is expected here (the grid
    // locks half a beat off). What we *can* guard is that the detector
    // still locks to a metrically-valid pulse — AMLt (which allows the
    // off-beat interpretation) must stay high. A drop here means the
    // rebuilt detector stopped tracking reggae's pulse at all, which is
    // a real regression even though the on-beat phase is unresolved
    // without the low-end an actual reggae mix carries.
    assert!(
        cont.amlt > 0.8,
        "one-drop: detector lost the reggae pulse entirely (AMLt = {:.3})",
        cont.amlt
    );
}
