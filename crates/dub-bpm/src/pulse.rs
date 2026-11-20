//! The pulse a band keeps: its tempo, bar by bar.
//!
//! [`crate::follow`] puts a line on each hit a band played. That is the
//! band's *feel* — a snare laid back, a pushed kick — and read four bars at
//! a time it says the tempo moves by a percent or two on a record a drum
//! machine made (`tests/follow_corpus.rs`). Straightening a drifting record
//! needs the other thing: the tempo the band is keeping underneath, which
//! moves slowly or not at all.
//!
//! This reads that. The record is looked at through a window four bars
//! wide, one bar at a time. In each window every tempo near the record's
//! own and every place the beat could fall is scored by how much onset a
//! comb of beats at that tempo and place lands on. One best answer per
//! window would jump about wherever a fill or a breakdown blurs the comb,
//! so the answers are chosen together: the path through all the windows
//! with the most onset, less a price for every change of tempo from one
//! bar to the next and for every beat that does not fall where the bar
//! before put it (Viterbi's search). A steady record has no reason to pay
//! that price and reads flat; a band that drifts pays a little, steadily.
//!
//! The beats come out evenly spaced within each bar, between places the
//! search fixed. Which of the two — beat or off-beat — the pulse sits on
//! is not decided here: a comb on the off-beats of a reggae record lands
//! on as much as one on its beats, and the tempo is the same either way.

use dub_spectral::HOP_SIZE;

/// How far either side of the record's tempo a bar's tempo is looked for.
const TEMPO_REACH: f64 = 0.08;
/// The step between tempos tried, as a share of the record's tempo.
const TEMPO_STEP: f64 = 0.0025;
/// The step between places the beat is tried at, in onset frames.
const PHASE_STEP: f64 = 0.5;
/// Beats either side of a window's middle its comb reaches: two bars.
const WINDOW_HALF_BEATS: i32 = 8;
/// Beats from one window to the next: a bar.
const HOP_BEATS: f64 = 4.0;
/// Tempo steps a bar may differ from the bar before, and what each costs,
/// squared, against onset measured in standard deviations.
const TEMPO_MOVES: isize = 3;
const TEMPO_COST: f32 = 0.4;
/// Phase steps a bar's beat may fall from where the bar before put it, and
/// what each costs, squared.
const PHASE_MOVES: isize = 3;
const PHASE_COST: f32 = 0.5;

/// The beats of the pulse a band keeps around `bpm`, in seconds. `None`
/// for a record too short to read or without onsets.
#[must_use]
pub fn band_pulse(samples: &[f32], sample_rate: u32, channels: u8, bpm: f64) -> Option<Vec<f64>> {
    if bpm <= 0.0 || sample_rate == 0 {
        return None;
    }
    let (odf, _) = crate::offline::onset_envelopes(samples, sample_rate, channels).ok()?;
    #[allow(clippy::cast_precision_loss)]
    let odf_sr = f64::from(sample_rate) / HOP_SIZE as f64;
    let halves = pulse_halves(&odf, 60.0 / bpm * odf_sr)?;
    let frames = on_the_louder_half(&halves, &odf);
    Some(on_the_kick(&frames, odf_sr, samples, sample_rate, channels))
}

/// [`band_pulse`] for a caller that knows where the beat is: of the two
/// sets the pulse could sit on, the one nearer `beats` — any beats on the
/// right side of the bar, however roughly placed. `odf` is the record's
/// onset envelope at `odf_sr` frames a second.
pub(crate) fn pulse_near(
    odf: &[f32],
    odf_sr: f64,
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    bpm: f64,
    beats: &[f64],
) -> Option<Vec<f64>> {
    if bpm <= 0.0 || odf_sr <= 0.0 {
        return None;
    }
    let halves = pulse_halves(odf, 60.0 / bpm * odf_sr)?;
    let frames: Vec<f64> = beats.iter().map(|&b| b * odf_sr).collect();
    // How far `frames` stand, in all, from the nearest half beat of one
    // parity.
    let away = |parity: usize| -> f64 {
        let set: Vec<f64> = halves.iter().skip(parity).step_by(2).copied().collect();
        frames
            .iter()
            .map(|&at| {
                let after = set.partition_point(|&h| h < at);
                let before = after.checked_sub(1).map_or(f64::MAX, |i| at - set[i]);
                set.get(after).map_or(before, |&h| before.min(h - at))
            })
            .sum()
    };
    let parity = usize::from(away(1) < away(0));
    let frames: Vec<f64> = halves.into_iter().skip(parity).step_by(2).collect();
    Some(on_the_kick(&frames, odf_sr, samples, sample_rate, channels))
}

/// Beats at onset `frames`, in seconds, set on the kick's visible edge as
/// the even grid's are.
fn on_the_kick(
    frames: &[f64],
    odf_sr: f64,
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> Vec<f64> {
    let mut beats: Vec<f64> = frames.iter().map(|&f| f / odf_sr).collect();
    let shift = crate::beats::kick_edge_offset_secs(&beats, samples, sample_rate, channels);
    for beat in &mut beats {
        *beat += shift;
    }
    beats.retain(|&b| b >= 0.0);
    beats
}

/// Every other half beat is a beat. Which, for a caller with nothing else
/// to go by: the set with more onset on it, over the whole record at
/// once. That is right for most music and a coin toss on a record that
/// plays its off-beats as loud as its beats.
fn on_the_louder_half(halves: &[f64], odf: &[f32]) -> Vec<f64> {
    let onset_on = |parity: usize| -> f64 {
        halves
            .iter()
            .skip(parity)
            .step_by(2)
            .map(|&at| f64::from(onset_at(odf, at)))
            .sum()
    };
    let parity = usize::from(onset_on(1) > onset_on(0));
    halves.iter().skip(parity).step_by(2).copied().collect()
}

/// `odf` at a place between frames.
fn onset_at(odf: &[f32], at: f64) -> f32 {
    if at < 0.0 {
        return 0.0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = at as usize;
    let (Some(&a), Some(&b)) = (odf.get(i), odf.get(i + 1)) else {
        return 0.0;
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let frac = (at - i as f64) as f32;
    a + (b - a) * frac
}

/// How much onset a comb lands on whose teeth are half of `period` apart,
/// its middle tooth at `middle`. The teeth count for less toward the
/// window's ends, so a bar entering or leaving the window does not jolt the
/// answer.
///
/// A tooth on every half beat, not every beat: a comb on the beats and a
/// comb on the off-beats then score the same, and nothing pulls the path
/// from one to the other. With a tooth per beat it slid half a beat through
/// a breakdown wherever the off-beat was the louder afterwards, and read
/// that slide as two percent of tempo.
fn comb(odf: &[f32], middle: f64, period: f64) -> f32 {
    let mut sum = 0.0f32;
    let reach = 2 * WINDOW_HALF_BEATS;
    for tooth in -reach..=reach {
        #[allow(clippy::cast_precision_loss)]
        let weight = 0.5 + 0.5 * (std::f32::consts::PI * tooth as f32 / (reach + 1) as f32).cos();
        sum += weight * onset_at(odf, middle + f64::from(tooth) * period / 2.0);
    }
    sum
}

/// The pulse's half beats in onset frames — the beats and the places
/// midway between them, which of the two not yet said — for a record
/// whose tempo is about one beat every `period` frames.
#[allow(clippy::too_many_lines)]
fn pulse_halves(odf: &[f32], period: f64) -> Option<Vec<f64>> {
    #[allow(clippy::cast_precision_loss)]
    let len = odf.len() as f64;
    if period < 4.0 || len < period * 32.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let reach = (TEMPO_REACH / TEMPO_STEP).round() as isize;
    #[allow(clippy::cast_precision_loss)]
    let periods: Vec<f64> = (-reach..=reach)
        .map(|i| period * (1.0 + i as f64 * TEMPO_STEP))
        .collect();
    let tempos = periods.len();
    let longest = periods[tempos - 1];
    // Places run a little past a quarter of a beat either side — the comb
    // repeats every half beat — so one near the edge still has neighbours
    // to move to.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let half_places = (longest / 4.0 / PHASE_STEP).ceil() as isize + PHASE_MOVES;
    #[allow(clippy::cast_sign_loss)]
    let places = (2 * half_places + 1) as usize;
    #[allow(clippy::cast_precision_loss)]
    let place_of = |j: usize| (j as isize - half_places) as f64 * PHASE_STEP;
    let hop = HOP_BEATS * period;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let windows = (len / hop).floor() as usize;
    if windows < 4 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let middle_of = |k: usize| (k as f64 + 0.5) * hop;
    let states = tempos * places;
    // Whether a place is within reach of a quarter of a beat at this tempo.
    let valid = |t: usize, j: usize| place_of(j).abs() <= periods[t] / 4.0 + PHASE_STEP * 2.0;

    // Onset every tempo and place lands on, window by window, less the
    // window's own average.
    let mut emit = vec![0.0f32; windows * states];
    let mut spreads = Vec::with_capacity(windows);
    for k in 0..windows {
        let middle = middle_of(k);
        let row = &mut emit[k * states..(k + 1) * states];
        let (mut sum, mut sum_sq, mut count) = (0.0f64, 0.0f64, 0usize);
        for t in 0..tempos {
            for j in 0..places {
                if !valid(t, j) {
                    continue;
                }
                let e = comb(odf, middle + place_of(j), periods[t]);
                row[t * places + j] = e;
                sum += f64::from(e);
                sum_sq += f64::from(e) * f64::from(e);
                count += 1;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = sum / count as f64;
        #[allow(clippy::cast_precision_loss)]
        spreads.push((sum_sq / count as f64 - mean * mean).max(0.0).sqrt());
        #[allow(clippy::cast_possible_truncation)]
        for e in row.iter_mut() {
            *e -= mean as f32;
        }
    }
    // One scale for the whole record: how far a comb on the beat stands
    // above one off it in a usual bar. Measured against that, a bar with a
    // beat pulls its weight and a breakdown — whose answers barely differ —
    // pulls hardly at all, so the path runs straight through it instead of
    // chasing a sweep or a voice.
    let mut sorted = spreads.clone();
    sorted.sort_by(f64::total_cmp);
    let usual = sorted[sorted.len() / 2];
    if usual <= 1e-12 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let scale = (1.0 / usual) as f32;

    let mut score = vec![f32::NEG_INFINITY; states];
    let mut before = vec![f32::NEG_INFINITY; states];
    let mut from = vec![u32::MAX; windows * states];

    for k in 0..windows {
        std::mem::swap(&mut score, &mut before);
        for t in 0..tempos {
            for j in 0..places {
                let state = t * places + j;
                if !valid(t, j) {
                    score[state] = f32::NEG_INFINITY;
                    continue;
                }
                let here = emit[k * states + state] * scale;
                if k == 0 {
                    score[state] = here;
                    continue;
                }
                let mut best = (f32::NEG_INFINITY, u32::MAX);
                #[allow(clippy::cast_possible_wrap)]
                let (t_i, tempos_i) = (t as isize, tempos as isize);
                for t_before in (t_i - TEMPO_MOVES).max(0)..=(t_i + TEMPO_MOVES).min(tempos_i - 1) {
                    #[allow(clippy::cast_sign_loss)]
                    let tb = t_before as usize;
                    let between = (periods[tb] + periods[t]) / 2.0;
                    // The place in the bar before from which whole half
                    // beats at the tempo between the two bars land exactly
                    // here.
                    let half_beats = ((hop + place_of(j)) / (between / 2.0)).round();
                    let came_from = hop + place_of(j) - half_beats * between / 2.0;
                    #[allow(clippy::cast_possible_truncation)]
                    let centre = (came_from / PHASE_STEP).round() as isize + half_places;
                    #[allow(clippy::cast_precision_loss)]
                    let tempo_cost = TEMPO_COST * ((t_before - t_i) * (t_before - t_i)) as f32;
                    #[allow(clippy::cast_possible_wrap)]
                    let places_i = places as isize;
                    for j_before in
                        (centre - PHASE_MOVES).max(0)..=(centre + PHASE_MOVES).min(places_i - 1)
                    {
                        #[allow(clippy::cast_sign_loss)]
                        let prev = tb * places + j_before as usize;
                        if before[prev] == f32::NEG_INFINITY {
                            continue;
                        }
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let missed =
                            ((place_of(j_before as usize) - came_from) / PHASE_STEP) as f32;
                        let total = before[prev] - tempo_cost - PHASE_COST * missed * missed;
                        if total > best.0 {
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                best = (total, prev as u32);
                            }
                        }
                    }
                }
                score[state] = if best.1 == u32::MAX {
                    f32::NEG_INFINITY
                } else {
                    here + best.0
                };
                from[k * states + state] = best.1;
            }
        }
    }

    // Read the best path back, and settle each bar's beat between the
    // places tried: the top of the parabola through its neighbours.
    let mut state = (0..states).max_by(|&a, &b| score[a].total_cmp(&score[b]))?;
    if score[state] == f32::NEG_INFINITY {
        return None;
    }
    let mut fixed = vec![(0.0f64, 0.0f64); windows];
    for k in (0..windows).rev() {
        let (t, j) = (state / places, state % places);
        let at = middle_of(k) + place_of(j);
        let (a, b, c) = (
            f64::from(comb(odf, at - PHASE_STEP, periods[t])),
            f64::from(comb(odf, at, periods[t])),
            f64::from(comb(odf, at + PHASE_STEP, periods[t])),
        );
        let bend = a - 2.0 * b + c;
        let nudge = if bend < -1e-12 {
            (0.5 * (a - c) / bend).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        fixed[k] = (at + nudge * PHASE_STEP, periods[t]);
        if k > 0 {
            state = from[k * states + state] as usize;
        }
    }

    // Half beats evenly spaced from one fixed place to the next.
    let mut halves = Vec::new();
    let (first, first_period) = fixed[0];
    #[allow(clippy::cast_possible_truncation)]
    let lead = (first / (first_period / 2.0)).floor() as i32;
    for n in (1..=lead).rev() {
        halves.push(first - f64::from(n) * first_period / 2.0);
    }
    for pair in fixed.windows(2) {
        let ((a, pa), (b, pb)) = (pair[0], pair[1]);
        let count = ((b - a) / ((pa + pb) / 4.0)).round().max(1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for n in 0..count as usize {
            #[allow(clippy::cast_precision_loss)]
            halves.push(a + (b - a) * n as f64 / count);
        }
    }
    let (last, last_period) = fixed[windows - 1];
    let mut at = last;
    while at < len {
        halves.push(at);
        at += last_period / 2.0;
    }
    Some(halves)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// A click on every beat of `beats`, mono.
    fn clicks(beats: &[f64], secs: f64) -> Vec<f32> {
        let sr = f64::from(SR);
        let mut samples = vec![0.0f32; (secs * sr) as usize];
        for &beat in beats {
            let at = (beat * sr) as usize;
            for (i, s) in samples[at..].iter_mut().take(2_400).enumerate() {
                *s += (-(i as f32) / 600.0).exp();
            }
        }
        samples
    }

    /// A record whose off-beats are the louder — a skank over a soft kick
    /// — is a coin toss by loudness, and lands on the off-beat. Told
    /// roughly where the beats are, the pulse sits on them.
    #[test]
    fn told_where_the_beat_is_the_pulse_sits_on_it() {
        let secs = 40.0;
        let beats: Vec<f64> = (0..76).map(|i| 1.0 + f64::from(i) * 0.5).collect();
        let offs: Vec<f64> = beats.iter().map(|b| b + 0.25).collect();
        let mut samples = clicks(&beats, secs);
        for (s, off) in samples.iter_mut().zip(clicks(&offs, secs)) {
            *s += 2.0 * off;
        }
        let (odf, _) = crate::offline::onset_envelopes(&samples, SR, 1).unwrap();
        let odf_sr = f64::from(SR) / HOP_SIZE as f64;
        // The hint: every beat, 40 ms late.
        let hint: Vec<f64> = beats.iter().map(|b| b + 0.04).collect();
        let pulse = pulse_near(&odf, odf_sr, &samples, SR, 1, 120.0, &hint).unwrap();
        let off_beat = |pulse: &[f64]| {
            pulse
                .iter()
                .filter(|&&p| p > 5.0 && p < 35.0)
                .map(|p| {
                    let nearest = beats.iter().map(|b| (b - p).abs()).fold(f64::MAX, f64::min);
                    nearest
                })
                .fold(0.0, f64::max)
        };
        assert!(
            off_beat(&pulse) < 0.06,
            "the pulse is up to {:.0} ms from a beat",
            off_beat(&pulse) * 1000.0
        );
        let alone = band_pulse(&samples, SR, 1, 120.0).unwrap();
        assert!(
            off_beat(&alone) > 0.15,
            "by loudness alone this record reads on its off-beats, or the test proves nothing"
        );
    }

    /// Tempo over four bars at each bar of `beats`, in BPM.
    fn tempos(beats: &[f64]) -> Vec<f64> {
        beats
            .windows(17)
            .step_by(4)
            .map(|w| 960.0 / (w[16] - w[0]))
            .collect()
    }

    #[test]
    fn a_drum_machine_reads_one_tempo() {
        let played: Vec<f64> = (0..230).map(|n| 0.4 + f64::from(n) * 0.5).collect();
        let pulse = band_pulse(&clicks(&played, 120.0), SR, 1, 120.0).expect("a pulse");
        for tempo in tempos(&pulse[8..pulse.len() - 8]) {
            assert!(
                (tempo - 120.0).abs() < 0.06,
                "read {tempo:.2} BPM on a steady 120"
            );
        }
    }

    #[test]
    fn a_band_that_drifts_reads_as_it_drifts() {
        // 110 to 112 and back, twice in two minutes.
        let secs = 120.0;
        let tempo_at = |t: f64| 111.0 + (std::f64::consts::TAU * t / 60.0).sin();
        let mut played = Vec::new();
        let mut t = 0.4;
        while t < secs - 1.0 {
            played.push(t);
            t += 60.0 / tempo_at(t);
        }
        let pulse = band_pulse(&clicks(&played, secs), SR, 1, 111.0).expect("a pulse");
        let inner = &pulse[8..pulse.len() - 8];
        for (i, tempo) in tempos(inner).into_iter().enumerate() {
            let middle = (inner[i * 4] + inner[i * 4 + 16]) / 2.0;
            // Over four bars the true tempo is its own average there.
            let truth = tempo_at(middle);
            assert!(
                (tempo - truth).abs() < 0.25,
                "at {middle:.0} s read {tempo:.2} BPM, the band is at {truth:.2}"
            );
        }
    }
}
