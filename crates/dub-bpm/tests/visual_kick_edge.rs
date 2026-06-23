//! Validate the "set the 1" visual kick-edge snap against the user's
//! hand-set grids. Runs only when the author's files are present.
//!
//! * Caru "Blaze Up Tha Dance" — crisp house kick. A tap anywhere near
//!   the kick must lock to the same edge (~0.082 s), within a few ms of
//!   the user's manual 0.0843 s, regardless of where in the bar it lands.
//! * Westside Connection "Bangin'" — soft sub-bass kick with no single
//!   visual edge. The detector must DECLINE (verbatim fallback) so the
//!   "1" stays where the user put it rather than snapping to a guess.

use dub_bpm::{analyze_beat_grid_with_profile, relatch_grid_at_downbeat_tap, OctaveProfile};
use dub_io::Track;

const BLAZE: &str = "/Users/klos/Music/Music/house/Caru - Blaze Up Tha Dance.mp3";
const BANGIN: &str = "/Users/klos/Music/The Best Of Westside Connection_ The Gangsta, The Killa, The Dope Dealer [2007] [320 kbps]/14. Bangin' (feat. Master P).mp3";

fn downbeat(path: &str, bpm: f64, tap: f64, profile: OctaveProfile) -> Option<f64> {
    let track = Track::load_from_path(path).ok()?;
    let grid = relatch_grid_at_downbeat_tap(
        track.samples(),
        track.sample_rate(),
        track.channels(),
        bpm,
        tap,
        profile,
    )
    .ok()?;
    Some(grid.beats[grid.bar_phase as usize])
}

#[test]
fn blaze_crisp_kick_locks_to_visual_edge() {
    if !std::path::Path::new(BLAZE).exists() {
        eprintln!("skip blaze: fixture not found");
        return;
    }
    // Ground truth: user_tap anchor 0.0843 s. Tap rough, all over the bar.
    let mut edges = Vec::new();
    for tap in [0.060, 0.075, 0.0843, 0.095, 0.110] {
        let d = downbeat(BLAZE, 136.0, tap, OctaveProfile::FourOnFloor).expect("relatch");
        eprintln!(
            "blaze tap {tap:.4} -> downbeat {d:.4} ({:+.1} ms vs 0.0843)",
            (d - 0.0843) * 1000.0
        );
        edges.push(d);
        assert!(
            (d - 0.0843).abs() < 0.010,
            "tap {tap:.4} must lock within 10 ms of the user's 0.0843; got {d:.4}"
        );
    }
    // Invariance: every tap lands on the same edge (within 3 ms).
    let lo = edges.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = edges.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (hi - lo) < 0.003,
        "all taps must lock to the same edge; spread {:.1} ms",
        (hi - lo) * 1000.0
    );
}

/// The AUTO grid (no taps) must phase itself onto the visual kick edges
/// the user aligns to by hand — the kick-edge shift replacing the old
/// forward-only amplitude-peak shift. A grid beat must land within a few
/// ms of each track's hand-set `user_tap` anchor.
#[test]
fn auto_grid_phases_onto_the_visual_kick_edge() {
    // (path, bpm, profile, user_tap anchor)
    let cases: [(&str, f64, OctaveProfile, f64); 4] = [
        (BLAZE, 136.0, OctaveProfile::FourOnFloor, 0.0843),
        (BANGIN, 88.0, OctaveProfile::HipHop, 0.1912),
        (
            "/Users/klos/Music/Music/DrumnBass/Rjd:Red Fox - Rude Boy Swing (original mix).mp3",
            174.0,
            OctaveProfile::DrumAndBass,
            0.1113,
        ),
        (
            "/Users/klos/Music/Music/DrumnBass/General Levy, MC Spyda, Eksman, Bru-C - Ten Toes (Original Mix).mp3",
            174.0,
            OctaveProfile::DrumAndBass,
            0.1660,
        ),
    ];
    for (path, _bpm, profile, anchor) in cases {
        if !std::path::Path::new(path).exists() {
            eprintln!("skip auto-grid: {path} not found");
            continue;
        }
        let track = Track::load_from_path(path).expect("decode");
        let grid = analyze_beat_grid_with_profile(
            track.samples(),
            track.sample_rate(),
            track.channels(),
            profile,
        )
        .expect("auto analyze");
        let period = 60.0 / grid.bpm;
        // Compare GRID PHASE (nearest beat to the anchor), not the
        // specific downbeat marker — bar-phase is a separate (backbeat refinement)
        // concern. Fold the anchor onto the grid's phase ring.
        let nearest = grid
            .beats
            .iter()
            .copied()
            .map(|b| ((b - anchor) / period).round().mul_add(-period, b - anchor))
            .map(f64::abs)
            .fold(f64::INFINITY, f64::min);
        eprintln!(
            "auto {path}\n   phase error {:+.1} ms   bar_phase {}",
            nearest * 1000.0,
            grid.bar_phase
        );
        assert!(
            nearest < 0.010,
            "auto grid must phase onto the user's kick edge within 10 ms; off by {:.1} ms ({path})",
            nearest * 1000.0
        );
        // Bar phase: the "1" is the first measurable beat → bar_phase 0,
        // matching every one of the user's hand-set grids (incl. Oppidan,
        // which backbeat refinement wrongly put on beat 1).
        assert_eq!(
            grid.bar_phase, 0,
            "the 1 must be the first measurable beat (bar_phase 0) for {path}; got {}",
            grid.bar_phase
        );
    }
}

#[test]
fn bangin_soft_kick_stays_verbatim() {
    if !std::path::Path::new(BANGIN).exists() {
        eprintln!("skip bangin: fixture not found");
        return;
    }
    // No clean edge → detector declines → the tap is kept verbatim.
    for tap in [0.180, 0.1912, 0.205] {
        let d = downbeat(BANGIN, 88.0, tap, OctaveProfile::HipHop).expect("relatch");
        eprintln!(
            "bangin tap {tap:.4} -> downbeat {d:.4} ({:+.1} ms)",
            (d - tap) * 1000.0
        );
        assert!(
            (d - tap).abs() < 0.002,
            "soft kick must stay verbatim at the tap {tap:.4}; got {d:.4}"
        );
    }
}
