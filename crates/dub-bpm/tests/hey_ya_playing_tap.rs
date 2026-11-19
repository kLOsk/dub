//! The operator's taps on *Hey Ya*, 2026-11-19, replayed.
//!
//! He set the 1 twice while the song played — "i hit it but it always
//! jumps to the next half beat". The log has the taps: 17.888 s and
//! 26.197 s. Each sits at the foot of a hit whose level then climbs for
//! some 90 ms (a strum into the drum): the hit starts about 20 ms after
//! the tap and is at full height about 100 ms after it. The grid came
//! back 215 ms after the tap, on the guitar strums between the beats:
//! they collect more onsets than the beats do, and the tap went to the
//! strongest thing within reach instead of the hit it was by.
//!
//! The same grid is the one he then found marked "may drift over a long
//! mix" and called perfect; the test holds that it is not marked.
//!
//! Local audio; without the file the test says so and passes.

use dub_bpm::fit_grid_at_playing_tap;
use dub_io::Track;

const HEY_YA: &str = "/Users/klos/Desktop/Musik/Requests/Outkast - Hey Ya.mp3";

/// The grid's tempo when he tapped: the song at half time.
const BPM: f64 = 79.489_997_863_769_53;

/// The taps, seconds into the song.
const TAPS: [f64; 2] = [17.888, 26.197];

/// A line belongs to the hit a tap was for if it sits from just before
/// the tap to where that hit is at full height.
const HIT_BEFORE_TAP_SECS: f64 = 0.010;
const HIT_AFTER_TAP_SECS: f64 = 0.110;

#[test]
fn a_tap_ahead_of_the_beat_on_hey_ya_lands_on_that_beat() {
    if !std::path::Path::new(HEY_YA).exists() {
        eprintln!("skip hey_ya_playing_tap: fixture not found at {HEY_YA}");
        return;
    }
    let track = Track::load_from_path(HEY_YA).expect("decode");
    let mut lines = Vec::new();
    for tap in TAPS {
        let grid = fit_grid_at_playing_tap(
            track.samples(),
            track.sample_rate(),
            track.channels(),
            BPM,
            tap,
        )
        .expect("grid");
        let line = grid
            .beats
            .iter()
            .copied()
            .min_by(|a, b| (a - tap).abs().total_cmp(&(b - tap).abs()))
            .expect("beats");
        eprintln!(
            "tap {tap:.3} -> line {line:.3} ({:+.0} ms from the tap)",
            (line - tap) * 1000.0
        );
        assert!(
            (tap - HIT_BEFORE_TAP_SECS..=tap + HIT_AFTER_TAP_SECS).contains(&line),
            "tap {tap}: the line by it is at {line}, not on the hit that was tapped"
        );
        // "it says may drift over a long mix but i find the grid
        // perfect": this grid, at Serato's two-decimal tempo, moves about
        // 4 ms a minute against the song. It read 5.6 against a limit
        // of 3.
        let quality = grid.quality.expect("a fitted grid has a quality");
        eprintln!(
            "tap {tap:.3}: drift {:+.1} ms/min",
            quality.drift_slope_ms_per_min
        );
        assert!(
            !quality.drifts(),
            "tap {tap}: a grid the operator calls perfect is marked as drifting \
             ({:.1} ms/min)",
            quality.drift_slope_ms_per_min
        );
        let one = grid.beats[usize::from(grid.bar_phase)];
        let bars = (line - one) / (4.0 * 60.0 / BPM);
        assert!(
            (bars - bars.round()).abs() < 0.02,
            "tap {tap}: the tapped beat is the 1 (the 1 is at {one})"
        );
        lines.push(line.rem_euclid(60.0 / BPM / 2.0));
    }
    assert!(
        (lines[0] - lines[1]).abs() < 0.010,
        "both taps give the same lines: {lines:?}"
    );
}
