//! Reverse loop region math.
//!
//! A DJ presses a loop button to **repeat the passage just heard**, not
//! the one coming up. So we place the loop *behind* the playhead: round
//! the press to the nearest beat line — a press a hair late snaps back
//! to the line the DJ aimed at, never grabbing the freshly-started beat
//! — and take the `length_beats` grid lines before it. The caller jumps
//! the playhead back into the region (see [`wrap_into`]); because the
//! region is a whole number of beats the jump is phase-aligned and the
//! loop wrap is seamless.
//!
//! Pure and unit-agnostic: `playhead` and `beats` share a unit (seconds
//! in practice) and the returned region is in that same unit. Keeping
//! the snapping here — rather than in the FFI or the UI — lets it be
//! unit-tested directly and keeps the "reverse loop" definition next to
//! the audio-thread wrap that consumes it.

/// Index of the beat line nearest `target`. Ties round to the
/// **earlier** line, so a press a hair past a beat snaps back to it
/// (the reverse-loop "slightly late still grabs the bar you aimed at"
/// behaviour) rather than rounding up into the next beat.
fn nearest_beat_index(beats: &[f64], target: f64) -> Option<usize> {
    if beats.is_empty() {
        return None;
    }
    // First index whose beat is at or after `target`.
    let pp = beats.partition_point(|&b| b < target);
    if pp == 0 {
        return Some(0);
    }
    if pp >= beats.len() {
        return Some(beats.len() - 1);
    }
    let lo = pp - 1;
    let hi = pp;
    // `<=` makes a tie round to the earlier line.
    if target - beats[lo] <= beats[hi] - target {
        Some(lo)
    } else {
        Some(hi)
    }
}

/// Compute a grid-snapped **reverse** loop of `length_beats` beats for a
/// press at `playhead` over the ascending grid `beats`.
///
/// Returns `(loop_in, loop_out)` in the inputs' unit, where `loop_out`
/// is the beat line nearest the press and `loop_in` is `length_beats`
/// grid lines earlier. Returns `None` when `length_beats == 0` or the
/// grid is too short to hold the loop. Near the track start the window
/// is clamped forward so it still fits (loops the first `length_beats`
/// beats) rather than refusing the loop outright.
#[must_use]
pub fn reverse_loop_region(playhead: f64, beats: &[f64], length_beats: u32) -> Option<(f64, f64)> {
    let len = length_beats as usize;
    if len == 0 || beats.len() < len + 1 {
        return None;
    }
    let raw = nearest_beat_index(beats, playhead)?;
    // Clamp so the whole window lands on the grid: `out_idx` can't be
    // below `len` (no room behind) or above the last beat.
    let out_idx = raw.clamp(len, beats.len() - 1);
    let in_idx = out_idx - len;
    let (lin, lout) = (beats[in_idx], beats[out_idx]);
    if lout > lin {
        Some((lin, lout))
    } else {
        None
    }
}

/// Bring `pos` into the half-open window `[lo, lo + len)` by adding or
/// subtracting whole multiples of `len`. Used both for the reverse-loop
/// jump-back on engage and the per-block wrap. `len <= 0` returns `pos`
/// unchanged.
#[must_use]
pub fn wrap_into(pos: f64, lo: f64, len: f64) -> f64 {
    if len <= 0.0 {
        return pos;
    }
    let mut p = pos;
    if p >= lo + len {
        let k = ((p - lo) / len).floor();
        p -= k * len;
    } else if p < lo {
        let k = ((lo - p) / len).ceil();
        p += k * len;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple 4/4 grid, 1 beat per unit, beats 0..=8 (two bars).
    fn grid() -> Vec<f64> {
        (0..=8).map(f64::from).collect()
    }

    #[test]
    fn nearest_rounds_to_closest_with_tie_to_earlier() {
        let g = grid();
        assert_eq!(nearest_beat_index(&g, 6.4), Some(6));
        assert_eq!(nearest_beat_index(&g, 6.6), Some(7));
        // Exact tie rounds back to the earlier line.
        assert_eq!(nearest_beat_index(&g, 6.5), Some(6));
        // Exactly on a line.
        assert_eq!(nearest_beat_index(&g, 4.0), Some(4));
        // Before the first / after the last clamp to the ends.
        assert_eq!(nearest_beat_index(&g, -3.0), Some(0));
        assert_eq!(nearest_beat_index(&g, 99.0), Some(8));
    }

    #[test]
    fn one_bar_loop_a_hair_late_grabs_the_bar_just_heard() {
        let g = grid();
        // Pressed at 6.4 — a hair past beat 6 — for a 1-bar (4-beat)
        // loop. loop_out snaps back to beat 6, loop_in is 4 beats
        // earlier (beat 2): the bar that just played.
        assert_eq!(reverse_loop_region(6.4, &g, 4), Some((2.0, 6.0)));
    }

    #[test]
    fn more_than_half_a_beat_late_rounds_up_to_the_next_line() {
        let g = grid();
        // 6.6 is closer to beat 7 → loop_out = 7, loop_in = 3.
        assert_eq!(reverse_loop_region(6.6, &g, 4), Some((3.0, 7.0)));
    }

    #[test]
    fn near_track_start_clamps_forward_to_fit() {
        let g = grid();
        // Pressed at 1.2 (nearest beat 1) with a 4-beat loop — only
        // one beat behind, so clamp to the first 4 beats [0, 4].
        assert_eq!(reverse_loop_region(1.2, &g, 4), Some((0.0, 4.0)));
    }

    #[test]
    fn half_and_two_bar_lengths() {
        let g = grid();
        // ½ bar = 2 beats ending at beat 6 → [4, 6].
        assert_eq!(reverse_loop_region(6.1, &g, 2), Some((4.0, 6.0)));
        // 2 bars = 8 beats ending at beat 8 → [0, 8].
        assert_eq!(reverse_loop_region(7.9, &g, 8), Some((0.0, 8.0)));
    }

    #[test]
    fn rejects_zero_length_or_too_short_grid() {
        let g = grid();
        assert_eq!(reverse_loop_region(4.0, &g, 0), None);
        // A 9-beat grid can't hold a 16-beat loop.
        assert_eq!(reverse_loop_region(4.0, &g, 16), None);
        // Empty / single-beat grids can't loop.
        assert_eq!(reverse_loop_region(0.0, &[], 4), None);
        assert_eq!(reverse_loop_region(0.0, &[1.0], 4), None);
    }

    #[test]
    fn non_uniform_grid_spans_exactly_n_beats() {
        // Beats drifting wider over time (a track that slows).
        let g = [0.0, 1.0, 2.1, 3.3, 4.6, 6.0];
        // Nearest to 4.7 is beat index 4 (4.6); 2-beat loop → [2.1, 4.6].
        let (lin, lout) = reverse_loop_region(4.7, &g, 2).unwrap();
        assert!((lin - 2.1).abs() < 1e-9);
        assert!((lout - 4.6).abs() < 1e-9);
    }

    #[test]
    fn wrap_into_forward_reverse_and_multiple() {
        // Inside the window is unchanged.
        assert!((wrap_into(3.0, 2.0, 4.0) - 3.0).abs() < 1e-9);
        // Past the end wraps back one length.
        assert!((wrap_into(6.4, 2.0, 4.0) - 2.4).abs() < 1e-9);
        // Before the start wraps forward.
        assert!((wrap_into(1.0, 2.0, 4.0) - 5.0).abs() < 1e-9);
        // Far past wraps multiple lengths in one shot.
        assert!((wrap_into(14.5, 2.0, 4.0) - 2.5).abs() < 1e-9);
        // Degenerate length is a no-op.
        assert!((wrap_into(5.0, 2.0, 0.0) - 5.0).abs() < 1e-9);
    }
}
