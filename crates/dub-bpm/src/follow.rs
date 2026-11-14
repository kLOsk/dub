//! A beat grid that follows the band.
//!
//! [`crate::analyze_beat_grid`] lays one tempo evenly across a track.
//! That is the truth for a drum machine and wrong for a drummer: *Cherry
//! Bomb* and *Ain't No Mountain High Enough* sit hundreds of milliseconds
//! off any even grid within half a minute, whichever tempo and phase it
//! is given (2026-11-14, investigation doc §7.5a). This puts a line on
//! each beat the band actually played.
//!
//! The beats are found the way Ellis's tracker finds them (*Beat
//! Tracking by Dynamic Programming*, 2007): every onset frame is scored
//! as "a beat here" — its own onset strength, plus the best-scoring
//! earlier beat, less a penalty for how far the gap between the two is
//! from the track's period. The best chain of beats through the whole
//! track is read back from its end. The period is the even grid's: the
//! band is taken to play *around* one tempo, which is what makes a
//! chain through a quiet bar or a fill hold its place.
//!
//! A track the even grid already fits is left alone — [`follow_grid`]
//! answers `None`, and the even grid, exact and drift-free, stands.

use crate::beats::BeatGrid;
use crate::octave_profile::OctaveProfile;
use crate::BpmRange;
use dub_spectral::HOP_SIZE;

/// How strongly a gap between beats is held to the period: the penalty
/// is `TIGHTNESS × ln(gap / period)²`, against onset strengths measured
/// in standard deviations. Ellis's value. A band 3 % off its tempo pays
/// 0.09 a beat — nothing; a chain that would skip to the off-beat pays
/// 16.
const TIGHTNESS: f64 = 100.0;

/// The even grid stands unless the followed beats wander at least
/// this far against an even pulse ([`wander_secs`]). Not enough on its
/// own — a chain that loses count through a breakdown wanders too — but
/// it keeps out a steady track whose even grid merely sits on the wrong
/// hit.
const WANDER_SECS_TO_FOLLOW: f64 = 0.060;

/// The followed beats must also land on this much more onset than the
/// even grid's do. Measured over 125 real tracks (2026-11-14): steady,
/// programmed tracks read 1.0–1.1× — the even grid already has their
/// hits — and live reggae, soul and rock 1.5–3.9×. Between the two sit
/// swung and sample-loop tracks, which keep their even grid: a line that
/// wobbles for nothing is worse than one that is even.
const FIT_GAIN_TO_FOLLOW: f64 = 1.5;

/// The band's own beats for a track `grid` lays evenly, or `None` when
/// the even grid already fits (or nothing better is found) and should
/// stand.
///
/// The answer keeps `grid`'s tempo, quality and confidence — the tempo
/// is still the one the band plays around — and puts the 1 on the
/// followed beat nearest the even grid's 1.
#[must_use]
pub fn follow_grid(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    grid: &BeatGrid,
) -> Option<BeatGrid> {
    if grid.bpm <= 0.0 || grid.beats.len() < 8 || sample_rate == 0 {
        return None;
    }
    let (_, odf, _) = crate::offline::analyze_bpm_with_range_profile_and_odfs(
        samples,
        sample_rate,
        channels,
        BpmRange::DEFAULT,
        OctaveProfile::Default,
    )
    .ok()?;
    #[allow(clippy::cast_precision_loss)]
    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let frames = chain(&odf, 60.0 / grid.bpm * odf_sr)?;
    let mut beats: Vec<f64> = frames.iter().map(|&f| f / odf_sr).collect();
    // The onsets peak mid-attack; the lines go on the kick's visible
    // edge, as the even grid's do.
    let shift = crate::beats::kick_edge_offset_secs(&beats, samples, sample_rate, channels);
    for b in &mut beats {
        *b += shift;
    }
    beats.retain(|&b| b >= 0.0);
    if beats.len() < 8 || !worth_following(&odf, odf_sr, &beats, &grid.beats) {
        return None;
    }

    let bar = usize::from(grid.beats_per_bar.max(1));
    let one = grid.beats[usize::from(grid.bar_phase).min(grid.beats.len() - 1)];
    let nearest = beats
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - one).abs().total_cmp(&(b.1 - one).abs()))
        .map_or(0, |(i, _)| i);
    #[allow(clippy::cast_possible_truncation)]
    let bar_phase = (nearest % bar) as u8;
    Some(BeatGrid {
        beats,
        bar_phase,
        ..grid.clone()
    })
}

/// How many beats back the local tempo is read over: two bars. One
/// beat's gap on a live drummer jumps about; two bars move slowly enough
/// to read off a deck and fast enough to follow a band pushing.
const LOCAL_TEMPO_BEATS: usize = 8;

/// A grid whose gaps differ by less than this is even — every grid the
/// analyser or a tap lays — and has one tempo, its own.
const EVEN_GAP_TOLERANCE_SECS: f64 = 0.001;

/// The tempo the band is playing at `at_secs` on a followed grid: the
/// average over the [`LOCAL_TEMPO_BEATS`] beats up to the playhead (the
/// first ones, before that many have gone by). `None` for an even grid —
/// its tempo is the grid's — or one too short to read.
#[must_use]
pub fn local_bpm(beats: &[f64], at_secs: f64) -> Option<f64> {
    if beats.len() <= LOCAL_TEMPO_BEATS {
        return None;
    }
    let (mut least, mut most) = (f64::MAX, f64::MIN);
    for w in beats.windows(2) {
        least = least.min(w[1] - w[0]);
        most = most.max(w[1] - w[0]);
    }
    if most - least < EVEN_GAP_TOLERANCE_SECS {
        return None;
    }
    let at = beats
        .partition_point(|&b| b <= at_secs)
        .saturating_sub(1)
        .clamp(LOCAL_TEMPO_BEATS, beats.len() - 1);
    let span = beats[at] - beats[at - LOCAL_TEMPO_BEATS];
    #[allow(clippy::cast_precision_loss)]
    (span > 0.0).then(|| 60.0 * LOCAL_TEMPO_BEATS as f64 / span)
}

/// The best chain of beats through `odf` at about `period` frames
/// apart, as fractional frame positions, oldest first.
fn chain(odf: &[f32], period: f64) -> Option<Vec<f64>> {
    let n = odf.len();
    if period < 4.0 || (n as f64) < period * 8.0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = odf.iter().map(|&v| f64::from(v)).sum::<f64>() / n as f64;
    #[allow(clippy::cast_precision_loss)]
    let var = odf
        .iter()
        .map(|&v| (f64::from(v) - mean).powi(2))
        .sum::<f64>()
        / n as f64;
    let sd = var.sqrt();
    if sd <= 0.0 {
        return None;
    }
    let strength = |i: usize| (f64::from(odf[i]) - mean) / sd;

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (near, far) = (
        (period / 2.0).round() as usize,
        (period * 2.0).round() as usize,
    );
    let mut score = vec![0.0f64; n];
    let mut from = vec![usize::MAX; n];
    for t in 0..n {
        let mut best = (0.0f64, usize::MAX);
        if t >= near {
            let first = t.saturating_sub(far);
            for (k, &before) in score[first..=t - near].iter().enumerate() {
                let prev = first + k;
                #[allow(clippy::cast_precision_loss)]
                let gap = (t - prev) as f64;
                let s = before - TIGHTNESS * (gap / period).ln().powi(2);
                if s > best.0 {
                    best = (s, prev);
                }
            }
        }
        score[t] = strength(t) + best.0;
        from[t] = best.1;
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tail = n.saturating_sub(period.round() as usize);
    let mut at = (tail..n).max_by(|&a, &b| score[a].total_cmp(&score[b]))?;
    let mut frames = Vec::new();
    loop {
        frames.push(peak_within_frame(odf, at));
        if from[at] == usize::MAX {
            break;
        }
        at = from[at];
    }
    frames.reverse();
    Some(frames)
}

/// Frame `i`, moved by up to half a frame toward its stronger
/// neighbour: the parabola through the three onset values.
fn peak_within_frame(odf: &[f32], i: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let at = i as f64;
    if i == 0 || i + 1 >= odf.len() {
        return at;
    }
    let (a, b, c) = (
        f64::from(odf[i - 1]),
        f64::from(odf[i]),
        f64::from(odf[i + 1]),
    );
    let denom = a - 2.0 * b + c;
    if denom >= -1e-12 {
        return at;
    }
    at + (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
}

/// How far the followed beats wander against an even pulse: each
/// beat's distance from where its count puts it on the best-fitting
/// even line through all of them, smoothed over a couple of bars so a
/// swung or flammed hit does not count, as the spread between the 5th
/// and 95th percentile. A steady track reads a frame or so whatever
/// hit the chain rides; a band reads tens or hundreds of ms.
fn wander_secs(followed: &[f64]) -> f64 {
    let n = followed.len();
    if n < 16 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let (nf, mean_i) = (n as f64, (n as f64 - 1.0) / 2.0);
    let mean_t = followed.iter().sum::<f64>() / nf;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, &t) in followed.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let di = i as f64 - mean_i;
        num += di * (t - mean_t);
        den += di * di;
    }
    let slope = num / den;
    #[allow(clippy::cast_precision_loss)]
    let off: Vec<f64> = followed
        .iter()
        .enumerate()
        .map(|(i, &t)| t - (mean_t + (i as f64 - mean_i) * slope))
        .collect();
    let mut smooth: Vec<f64> = off
        .windows(9)
        .map(|w| {
            let mut w = w.to_vec();
            w.sort_by(f64::total_cmp);
            w[4]
        })
        .collect();
    smooth.sort_by(f64::total_cmp);
    smooth[smooth.len() * 95 / 100] - smooth[smooth.len() * 5 / 100]
}

/// Whether `followed` beats the `even` grid by enough to replace it.
fn worth_following(odf: &[f32], odf_sr: f64, followed: &[f64], even: &[f64]) -> bool {
    let onset_at = |beats: &[f64]| -> f64 {
        let sum: f64 = beats
            .iter()
            .map(|&b| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let i = (b * odf_sr).round().max(0.0) as usize;
                (i.saturating_sub(1)..=i + 1)
                    .filter_map(|j| odf.get(j))
                    .fold(0.0f32, |m, &v| m.max(v))
            })
            .map(f64::from)
            .sum();
        #[allow(clippy::cast_precision_loss)]
        let n = beats.len().max(1) as f64;
        sum / n
    };
    let (wander, gain) = (
        wander_secs(followed),
        onset_at(followed) / onset_at(even).max(1e-12),
    );
    wander >= WANDER_SECS_TO_FOLLOW && gain >= FIT_GAIN_TO_FOLLOW
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beats::uniform_beats;
    use crate::synthetic::click_track;

    const SR: u32 = 48_000;

    fn even_grid(bpm: f64, secs: f64) -> BeatGrid {
        let mut grid = BeatGrid::none();
        grid.bpm = bpm;
        grid.confidence = 1.0;
        grid.beats = uniform_beats(bpm, 0.0, secs);
        grid.beats_per_bar = 4;
        grid
    }

    /// A band that starts at 126 and ends at 132 — five percent over two
    /// minutes. No even grid holds it; the followed beats sit on the
    /// clicks it played.
    #[test]
    fn a_band_that_speeds_up_gets_a_line_on_every_beat() {
        let sr = f64::from(SR);
        let secs = 120.0;
        let mut samples = vec![0.0f32; (secs * sr) as usize];
        let mut played = Vec::new();
        let mut t = 0.5f64;
        while t < secs - 1.0 {
            played.push(t);
            let at = (t * sr) as usize;
            for (i, s) in samples[at..].iter_mut().take(2_400).enumerate() {
                *s += (-(i as f32) / 600.0).exp();
            }
            t += 60.0 / (126.0 + 6.0 * t / secs);
        }
        let followed = follow_grid(&samples, SR, 1, &even_grid(129.0, secs)).expect("followed");
        let on = played
            .iter()
            .filter(|&&p| followed.beats.iter().any(|b| (b - p).abs() < 0.015))
            .count();
        assert!(
            on * 100 >= played.len() * 95,
            "{on} of {} played beats have a line within 15 ms",
            played.len()
        );
        assert!(
            followed.beats.len() <= played.len() + 4,
            "{} lines for {} beats",
            followed.beats.len(),
            played.len()
        );
    }

    /// On a followed grid the tempo is the band's at the playhead: 100
    /// where it plays 100, 110 where it has sped up to 110.
    #[test]
    fn the_local_tempo_is_the_bands_at_the_playhead() {
        let mut beats = vec![0.0];
        for i in 0..32 {
            let bpm = if i < 16 { 100.0 } else { 110.0 };
            beats.push(beats[i] + 60.0 / bpm);
        }
        let early = local_bpm(&beats, beats[12]).expect("followed");
        let late = local_bpm(&beats, beats[30]).expect("followed");
        assert!((early - 100.0).abs() < 0.01, "{early}");
        assert!((late - 110.0).abs() < 0.01, "{late}");
        // Before two bars have gone by, the first two bars' tempo.
        assert!((local_bpm(&beats, 0.0).expect("followed") - 100.0).abs() < 0.01);
    }

    /// An even grid has no local tempo of its own.
    #[test]
    fn an_even_grid_has_one_tempo() {
        assert_eq!(local_bpm(&uniform_beats(174.0, 0.0, 60.0), 20.0), None);
        assert_eq!(local_bpm(&[0.0, 0.5, 1.0], 0.5), None);
    }

    /// A steady track keeps its even grid: nothing to follow.
    #[test]
    fn a_steady_track_keeps_its_even_grid() {
        let samples = click_track(174.0, 60.0, SR);
        assert!(follow_grid(&samples, SR, 1, &even_grid(174.0, 60.0)).is_none());
    }

    /// The 1 stays where the even grid had it.
    #[test]
    fn the_one_stays_on_the_even_grids_one() {
        let sr = f64::from(SR);
        let secs = 90.0;
        let mut samples = vec![0.0f32; (secs * sr) as usize];
        let mut t = 0.5f64;
        while t < secs - 1.0 {
            let at = (t * sr) as usize;
            for (i, s) in samples[at..].iter_mut().take(2_400).enumerate() {
                *s += (-(i as f32) / 600.0).exp();
            }
            t += 60.0 / (100.0 + 5.0 * t / secs);
        }
        let mut even = even_grid(102.5, secs);
        even.beats = uniform_beats(102.5, 0.5, secs);
        even.bar_phase = 2;
        let one = even.beats[2];
        let followed = follow_grid(&samples, SR, 1, &even).expect("followed");
        let got = followed.beats[usize::from(followed.bar_phase)];
        let bar = 4.0 * 60.0 / 102.5;
        let off = (got - one).rem_euclid(bar).min((one - got).rem_euclid(bar));
        assert!(off < 0.15, "the 1 moved {off:.3} s round the bar");
    }
}
