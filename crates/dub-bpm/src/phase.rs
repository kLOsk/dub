//! Where the beats of a live input fall, given its tempo.
//!
//! A THRU deck has no beat grid: the record has no file to analyse ahead
//! of time, only the onsets heard so far. With the tempo known (tracked,
//! tapped or scaled by the DJ), the phase is the one offset at which
//! beats one period apart keep landing on onsets. That offset projects
//! forward, which is what lets the waveform draw the next beats below a
//! playhead that has no future of its own.
//!
//! The search folds the onset strength function at the period: every
//! candidate offset scores the onsets found one, two, three periods back
//! from it, newer beats weighted more so a slow drift in the record's
//! speed counts against the old evidence first. A flat fold — no offset
//! clearly better than the rest — is no phase at all: drawing beats then
//! would be drawing noise.

/// Seconds of onsets the fold looks back over. Eight seconds is sixteen
/// beats at 120: enough that one loud fill does not decide the phase,
/// short enough that a record pitched by hand has not drifted off it.
pub const PHASE_WINDOW_SECS: f64 = 8.0;

/// How much better than the average offset the best one must score for
/// the phase to count. Measured against the fold, so it holds at any
/// level.
const MIN_CONTRAST: f64 = 1.35;

/// Weight of each beat back from the newest: `DECAY^k`.
const DECAY: f64 = 0.88;

/// Candidate offsets per onset hop. A hop is 512 samples (~11 ms); a
/// quarter of one keeps the drawn beat inside a waveform column.
const STEPS_PER_HOP: usize = 4;

/// The newest beat in `odf` at `period_secs`, as seconds from the start
/// of the slice, or `None` when the onsets do not settle on one phase.
///
/// `odf` is an onset strength function at `hop_secs` per value, oldest
/// first — the last [`PHASE_WINDOW_SECS`] of it is what counts. The
/// slice must hold at least four periods.
///
/// Each call answers on its own. A record whose off-beat is nearly as
/// strong as its beat can answer either, call to call — drawing it four
/// times a second, [`BeatFollower`] is the one to use.
#[must_use]
pub fn latest_beat(odf: &[f32], hop_secs: f64, period_secs: f64) -> Option<f64> {
    let fold = Fold::of(odf, hop_secs, period_secs)?;
    let (best, score) = fold.best();
    fold.has_phase(score).then(|| fold.secs_of(best))
}

/// How far a beat may move between two looks and still be the same
/// beat, in candidate offsets: two hops, ~21 ms. A kick's onset spreads
/// over a hop or two, and a record pitched by hand drifts a little.
const SAME_BEAT: f64 = (2 * STEPS_PER_HOP) as f64;

/// How much better than the beat the follower holds another phase must
/// score before it can take over. The reggae 12″'s skank scores within
/// a few percent of its kick; anything this close is a coin toss, and
/// a coin toss is what made the forecast jump (2026-11-14).
const SWITCH_RATIO: f64 = 1.2;

/// How long another phase must keep winning by [`SWITCH_RATIO`] before
/// the follower moves to it, in seconds of record: four bars at 120.
const SWITCH_SECS: f64 = 2.0;

/// How long the follower keeps its beat through onsets with no phase —
/// a fill, a drop to bass alone — before it lets go.
const HOLD_SECS: f64 = 1.0;

/// How long the beat the follower holds may be missing from the onsets
/// while another phase has them, before the follower moves. A
/// breakbeat's kicks drop out for a beat or two and come back; taking
/// the other phase at once and the held one back a look later is what
/// made the drum'n'bass forecast jump 100 ms there and back.
const LOST_SECS: f64 = 0.75;

/// A period this much off the last one is another tempo (the DJ scaled
/// it, or the tracker re-locked): start over rather than carry a beat
/// across.
const RETEMPO: f64 = 0.02;

/// How far each look moves the beat toward where its onsets put it.
/// The onsets' own jitter — a hop here and there — averages out.
const FOLLOW: f64 = 0.3;

/// Beats back the follower reads its beat's position from, and how
/// fast they count less. Which phase is the beat is decided over the
/// whole window; where that beat is now is read from the last few. A
/// band 2 % off the tempo it was given has moved 10 ms a beat, and
/// eight seconds of that folded together is a smear the peak sticks in
/// and then lurches out of — 77 ms at a time.
const TRACK_BEATS: f64 = 4.0;
const TRACK_DECAY: f64 = 0.6;

/// Seconds of looks the record's pace is read over: long enough that
/// one late snare does not tilt it, short enough to hear a band push.
const PACE_SECS: f64 = 4.0;

/// Looks kept for the pace: [`PACE_SECS`] at four a second, and room
/// for a faster caller.
const TRAIL: usize = 32;

/// The fewest looks a pace is read from — two seconds' worth.
const PACE_MIN_LOOKS: usize = 8;

/// Looks closer together than this add nothing to the pace: a caller
/// asking every frame must not squeeze [`PACE_SECS`] into a fraction of
/// a second of the trail.
const PACE_SPACING_SECS: f64 = 0.2;

/// The most the record's own pace may differ from the tempo it was
/// given: past this the tempo is wrong, not the band.
const MAX_SLIP: f64 = 0.04;

/// The beats of a live input, followed from look to look.
///
/// [`latest_beat`] picks the strongest phase every time it is asked.
/// That is right once and jumpy when drawn: on a record whose off-beat
/// is nearly as strong as its beat, the two trade places look to look
/// and the forecast leaps half a beat back and forth — 25 times in 85 s
/// of the reggae 12″ with its tempo locked. The follower keeps the beat
/// it has. It follows that beat as it drifts, smoothing the onsets'
/// jitter, and gives it up only when another phase has clearly won for
/// [`SWITCH_SECS`], when the beat it holds is no longer in the onsets,
/// or when the onsets have had no phase for [`HOLD_SECS`].
///
/// It also reads the record's own pace. A band does not play to the
/// tracker's figure: the soul sampler runs ~2 % ahead of its locked
/// 112.9 for a stretch, its beat sliding ~24 ms a second against the
/// forecast, and a follower that only corrects the phase trails it and
/// catches up in jumps. The pace is the slope of where the beat has
/// landed over the last [`PACE_SECS`] — read off the looks, never fed
/// back into the folds, so it cannot ring — and the beats are carried
/// and forecast at the period it implies.
#[derive(Debug, Clone, Default)]
pub struct BeatFollower {
    /// The newest beat at the last look, in the caller's seconds.
    beat: Option<f64>,
    /// The period the caller gave.
    period: f64,
    /// How far the beats drift against `period`, seconds per second:
    /// they arrive every `period × (1 + slip)`.
    slip: f64,
    /// When the onsets last had a phase.
    phased_at: f64,
    /// Another phase winning, as one of its beats, and since when.
    challenger: Option<(f64, f64)>,
    /// Since when the held beat has been missing from the onsets.
    lost_since: Option<f64>,
    /// Where recent looks saw the beat — `(when, where)`, unwrapped
    /// against `period` so it moves only as the beat drifts. A ring.
    trail: [(f64, f64); TRAIL],
    trail_len: usize,
    trail_next: usize,
}

/// Where a follower puts the beats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FollowedBeats {
    /// The newest beat at or before the newest onset, in the caller's
    /// seconds.
    pub last_beat_secs: f64,
    /// The period the beats are arriving at: the given one, as the
    /// record's own pace has it.
    pub period_secs: f64,
}

impl BeatFollower {
    /// A follower with no beat yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget the beat: a new record, or another input.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Look at the onsets again and answer where the beats are — `None`
    /// while there is no phase to draw.
    ///
    /// `odf` is as for [`latest_beat`], with `odf[0]` at `start_secs`.
    /// Looks must move forward in time.
    pub fn update(
        &mut self,
        odf: &[f32],
        start_secs: f64,
        hop_secs: f64,
        period_secs: f64,
    ) -> Option<FollowedBeats> {
        if self.beat.is_some() && (period_secs / self.period - 1.0).abs() > RETEMPO {
            self.reset();
        }
        self.period = period_secs;
        let Some(fold) = Fold::of(odf, hop_secs, period_secs) else {
            self.reset();
            return None;
        };
        let now = start_secs + fold.secs_of(0.0);
        let (best, best_score) = fold.best();
        if !fold.has_phase(best_score) {
            if self.beat.is_some() && now - self.phased_at <= HOLD_SECS {
                return self.newest(now);
            }
            self.reset();
            return None;
        }
        self.phased_at = now;
        let Some(beat) = self.newest(now).map(|b| b.last_beat_secs) else {
            return self.take(start_secs + fold.secs_of(best), now);
        };

        let held = fold.offset_of(now - beat);
        let smear = fold.same_phase();
        let (_, held_score) = fold.best_near(held, smear);
        let recent = Fold::over(odf, hop_secs, self.pace(), TRACK_BEATS, TRACK_DECAY)?;
        let (seen, _) = recent.best_near(recent.offset_of(now - beat), SAME_BEAT);
        if fold.has_phase(held_score) {
            self.lost_since = None;
        }
        if fold.distance(best, held) <= smear {
            self.challenger = None;
        } else if !fold.has_phase(held_score) {
            let since = *self.lost_since.get_or_insert(now);
            if now - since >= LOST_SECS {
                return self.take(start_secs + fold.secs_of(best), now);
            }
            return self.newest(now);
        } else if best_score >= held_score * SWITCH_RATIO {
            let other = start_secs + fold.secs_of(best);
            let since = match self.challenger {
                Some((was, since)) if fold.distance(fold.offset_of(now - was), best) <= smear => {
                    since
                }
                _ => now,
            };
            if now - since >= SWITCH_SECS {
                return self.take(other, now);
            }
            self.challenger = Some((other, since));
        } else {
            self.challenger = None;
        }
        let error = wrap(start_secs + recent.secs_of(seen) - beat, self.pace());
        self.beat = Some(beat + FOLLOW * error);
        self.saw(now, beat + error);
        self.newest(now)
    }

    fn take(&mut self, beat: f64, now: f64) -> Option<FollowedBeats> {
        self.beat = Some(beat);
        self.challenger = None;
        self.lost_since = None;
        self.trail_len = 0;
        self.saw(now, beat);
        self.newest(now)
    }

    /// The period the beats are arriving at.
    fn pace(&self) -> f64 {
        self.period * (1.0 + self.slip)
    }

    /// The held beat carried forward to the newest one at or before `now`.
    fn newest(&self, now: f64) -> Option<FollowedBeats> {
        let beat = self.beat?;
        let pace = self.pace();
        Some(FollowedBeats {
            last_beat_secs: now - (now - beat).rem_euclid(pace),
            period_secs: pace,
        })
    }

    /// Note where this look saw the beat, and read the pace again: the
    /// least-squares slope of the last [`PACE_SECS`] of them.
    fn saw(&mut self, now: f64, beat: f64) {
        let newest = self.trail[(self.trail_next + TRAIL - 1) % TRAIL];
        if self.trail_len > 0 && now - newest.0 < PACE_SPACING_SECS {
            return;
        }
        let at = if self.trail_len == 0 {
            beat
        } else {
            beat + ((newest.1 - beat) / self.period).round() * self.period
        };
        self.trail[self.trail_next] = (now, at);
        self.trail_next = (self.trail_next + 1) % TRAIL;
        self.trail_len = (self.trail_len + 1).min(TRAIL);

        let (mut n, mut st, mut sx, mut stt, mut stx) = (0usize, 0.0, 0.0, 0.0, 0.0);
        for i in 0..self.trail_len {
            let (when, x) = self.trail[(self.trail_next + TRAIL - 1 - i) % TRAIL];
            if now - when > PACE_SECS {
                break;
            }
            let t = when - now;
            n += 1;
            st += t;
            sx += x - at;
            stt += t * t;
            stx += t * (x - at);
        }
        if n < PACE_MIN_LOOKS {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = n as f64;
        let spread = n * stt - st * st;
        if spread > 0.0 {
            self.slip = ((n * stx - st * sx) / spread).clamp(-MAX_SLIP, MAX_SLIP);
        }
    }
}

/// `d` folded into `(-period/2, period/2]`.
fn wrap(d: f64, period: f64) -> f64 {
    let r = d.rem_euclid(period);
    if r > period / 2.0 {
        r - period
    } else {
        r
    }
}

/// The onsets folded at one period: a score for every candidate offset
/// back from the newest onset, a quarter-hop apart.
struct Fold {
    scores: Vec<f64>,
    mean: f64,
    /// Index of the newest onset.
    last: f64,
    hop_secs: f64,
}

impl Fold {
    /// The fold [`latest_beat`] decides on: [`PHASE_WINDOW_SECS`] of
    /// beats, each [`DECAY`] of the one after.
    fn of(odf: &[f32], hop_secs: f64, period_secs: f64) -> Option<Self> {
        Self::over(odf, hop_secs, period_secs, f64::INFINITY, DECAY)
    }

    /// A fold over at most `beats` beats back, each `decay` of the one
    /// after.
    fn over(odf: &[f32], hop_secs: f64, period_secs: f64, beats: f64, decay: f64) -> Option<Self> {
        if !(hop_secs > 0.0 && period_secs > 0.0) || odf.is_empty() {
            return None;
        }
        let period = period_secs / hop_secs;
        #[allow(clippy::cast_precision_loss)]
        let len = odf.len() as f64;
        if period < 2.0 || len < period * 4.0 {
            return None;
        }
        let window = (PHASE_WINDOW_SECS / hop_secs).min(len);
        let last = len - 1.0;
        let beats_back = (window / period).floor().max(1.0).min(beats);

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let candidates = (period * STEPS_PER_HOP as f64).ceil() as usize;
        let mut scores = Vec::with_capacity(candidates);
        for c in 0..candidates {
            #[allow(clippy::cast_precision_loss)]
            let offset = c as f64 / STEPS_PER_HOP as f64;
            let mut score = 0.0;
            let mut weight = 1.0;
            let mut k = 0.0;
            while k < beats_back {
                let at = last - offset - k * period;
                if at < 0.0 {
                    break;
                }
                score += weight * strength_near(odf, at);
                weight *= decay;
                k += 1.0;
            }
            scores.push(score);
        }
        #[allow(clippy::cast_precision_loss)]
        let mean = scores.iter().sum::<f64>() / candidates as f64;
        Some(Self {
            scores,
            mean,
            last,
            hop_secs,
        })
    }

    /// The best candidate and its score.
    fn best(&self) -> (f64, f64) {
        let mut best = (0.0, f64::MIN);
        for (c, &s) in self.scores.iter().enumerate() {
            if s > best.1 {
                #[allow(clippy::cast_precision_loss)]
                let c = c as f64;
                best = (c, s);
            }
        }
        best
    }

    /// The best candidate within `reach` of `around`, the fold wrapping
    /// round at the period. A flat top — [`strength_near`] gives one
    /// hop's spike to the quarter-hops either side alike — answers its
    /// middle, not its newest edge, which would lean every beat late.
    fn best_near(&self, around: f64, reach: f64) -> (f64, f64) {
        #[allow(clippy::cast_precision_loss)]
        let n = self.scores.len() as f64;
        #[allow(clippy::cast_possible_truncation)]
        let (from, to) = (
            (around - reach).round() as i64,
            (around + reach).round() as i64,
        );
        let score = |c: i64| {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let at = (c as f64).rem_euclid(n) as usize;
            self.scores.get(at).copied().unwrap_or(f64::MIN)
        };
        let (mut first, mut last, mut top) = (from, from, f64::MIN);
        for c in from..=to {
            let s = score(c);
            if s > top {
                (first, last, top) = (c, c, s);
            } else if s == top && last == c - 1 {
                last = c;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let middle = (first + last) as f64 / 2.0;
        (middle.rem_euclid(n), top)
    }

    /// How near another candidate may be and still be the same beat to
    /// the decision of which phase is the beat: a sixth of the period.
    /// Over the whole window a drifting beat smears; another phase — the
    /// skank, a quarter-beat hat — sits a quarter of a beat or more away.
    fn same_phase(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let n = self.scores.len() as f64;
        (n / 6.0).max(SAME_BEAT)
    }

    fn has_phase(&self, score: f64) -> bool {
        score > 0.0 && score >= self.mean * MIN_CONTRAST
    }

    /// Seconds from `odf[0]` of candidate `c`.
    fn secs_of(&self, c: f64) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let steps = STEPS_PER_HOP as f64;
        (self.last - c / steps) * self.hop_secs
    }

    /// The candidate a beat `back_secs` before the newest onset falls on.
    fn offset_of(&self, back_secs: f64) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let (steps, n) = (STEPS_PER_HOP as f64, self.scores.len() as f64);
        (back_secs / self.hop_secs * steps).rem_euclid(n)
    }

    /// Candidates between `a` and `b` the short way round.
    fn distance(&self, a: f64, b: f64) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let n = self.scores.len() as f64;
        let d = (a - b).abs().rem_euclid(n);
        d.min(n - d)
    }
}

/// The onset strength at fractional index `at`: the larger of the two
/// hops either side. Taking the larger rather than interpolating keeps a
/// one-hop spike from scoring half at the quarter-hop offsets around it,
/// which would smear the fold.
fn strength_near(odf: &[f32], at: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = at.floor() as usize;
    let a = odf.get(i).copied().unwrap_or(0.0);
    let b = odf.get(i + 1).copied().unwrap_or(0.0);
    f64::from(a.max(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOP: f64 = 512.0 / 48_000.0;

    /// Spikes every `period_secs`, the first at `first_secs`, over `secs`.
    fn clicks(period_secs: f64, first_secs: f64, secs: f64) -> Vec<f32> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (secs / HOP) as usize;
        let mut odf = vec![0.05f32; n];
        let mut t = first_secs;
        while t < secs {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = (t / HOP).round() as usize;
            if i < n {
                odf[i] = 1.0;
            }
            t += period_secs;
        }
        odf
    }

    /// The newest click on a 120 BPM track is the beat.
    #[test]
    fn a_click_track_puts_the_beat_on_the_click() {
        let odf = clicks(0.5, 0.13, 10.0);
        let beat = latest_beat(&odf, HOP, 0.5).expect("clicks have a phase");
        // The newest click before 10 s: 0.13 + 19 × 0.5.
        assert!((beat - 9.63).abs() < HOP * 1.5, "beat at {beat}");
    }

    /// A tempo a little off — a record pitched a touch since the tracker
    /// read it — still lands on the newest click, because the newest
    /// beats weigh most.
    #[test]
    fn a_slightly_wrong_tempo_still_finds_the_newest_beat() {
        let odf = clicks(0.5, 0.2, 10.0);
        let beat = latest_beat(&odf, HOP, 0.5 * 1.004).expect("close enough to fold");
        assert!((beat - 9.7).abs() < HOP * 2.5, "beat at {beat}");
    }

    /// Double the tempo the clicks are at: every other drawn beat falls
    /// between clicks, and the phase still sits on a click.
    #[test]
    fn doubling_the_tempo_keeps_the_beat_on_a_click() {
        let odf = clicks(0.5, 0.2, 10.0);
        let beat = latest_beat(&odf, HOP, 0.25).expect("half the period still folds");
        let from_click = ((beat - 0.2) / 0.5).fract() * 0.5;
        assert!(
            !(HOP * 1.5..=0.25 - HOP * 1.5).contains(&from_click),
            "beat at {beat}"
        );
    }

    /// Steady noise has no phase: nothing is drawn rather than a guess.
    #[test]
    fn flat_onsets_have_no_phase() {
        let mut seed = 12_345u32;
        let odf: Vec<f32> = (0..900)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                0.5 + f32::from(u16::try_from(seed >> 16 & 0xff).unwrap_or(0)) / 2550.0
            })
            .collect();
        assert_eq!(latest_beat(&odf, HOP, 0.5), None);
    }

    /// Too little heard yet, or nonsense input: no phase.
    #[test]
    fn too_little_input_has_no_phase() {
        let odf = clicks(0.5, 0.1, 1.5);
        assert_eq!(latest_beat(&odf, HOP, 0.5), None);
        assert_eq!(latest_beat(&[], HOP, 0.5), None);
        assert_eq!(latest_beat(&odf, HOP, 0.0), None);
    }

    /// Each look the way a THRU deck asks: the last window of onsets
    /// every quarter second, through `follower` and through
    /// [`latest_beat`]. Returns both answers per look, in seconds of
    /// the whole `odf`.
    fn looks(odf: &[f32], period: f64, from_secs: f64) -> Vec<(Option<f64>, Option<f64>)> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (window, step, from) = (
            (PHASE_WINDOW_SECS / HOP).ceil() as usize + 1,
            (0.25 / HOP).round() as usize,
            (from_secs / HOP) as usize,
        );
        let mut follower = BeatFollower::new();
        let mut out = Vec::new();
        let mut len = from;
        while len <= odf.len() {
            let start = len.saturating_sub(window);
            #[allow(clippy::cast_precision_loss)]
            let start_secs = start as f64 * HOP;
            let slice = &odf[start..len];
            out.push((
                follower
                    .update(slice, start_secs, HOP, period)
                    .map(|b| b.last_beat_secs),
                latest_beat(slice, HOP, period).map(|b| start_secs + b),
            ));
            len += step;
        }
        out
    }

    /// The largest move of a beat between consecutive answers, the
    /// period folded out.
    fn biggest_move(beats: &[Option<f64>], period: f64) -> f64 {
        beats
            .windows(2)
            .filter_map(|w| Some(wrap(w[1]? - w[0]?, period).abs()))
            .fold(0.0, f64::max)
    }

    /// A kick on the beat and a skank on the off-beat, the skank a hair
    /// stronger in every other two seconds — the reggae 12″'s shape.
    /// Asked fresh, the phase flips half a beat each time the skank
    /// edges ahead; the follower stays on the beat it took.
    #[test]
    fn an_off_beat_nearly_as_strong_does_not_pull_the_forecast_over() {
        let period = 0.45;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (40.0 / HOP) as usize;
        let mut odf = vec![0.05f32; n];
        let mut t = 0.1;
        while t < 40.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let skank = if ((t / 2.0) as u32).is_multiple_of(2) {
                0.97
            } else {
                1.03
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (kick, off) = (
                (t / HOP).round() as usize,
                ((t + period / 2.0) / HOP).round() as usize,
            );
            if off < n {
                odf[kick] = 1.0;
                odf[off] = skank;
            }
            t += period;
        }
        let answers = looks(&odf, period, 10.0);
        let followed: Vec<_> = answers.iter().map(|a| a.0).collect();
        let fresh: Vec<_> = answers.iter().map(|a| a.1).collect();
        assert!(
            followed.iter().all(Option::is_some),
            "the follower lost its beat"
        );
        assert!(
            biggest_move(&fresh, period) > period * 0.4,
            "the fresh answer should flip on this input — the test lost its bite"
        );
        assert!(
            biggest_move(&followed, period) < HOP,
            "the forecast moved {:.1} ms",
            biggest_move(&followed, period) * 1000.0
        );
    }

    /// The beat really moves — the kick now lands where the skank was
    /// and the old phase is gone. The follower goes with it promptly.
    #[test]
    fn a_beat_that_really_moves_is_followed() {
        let period = 0.5;
        let mut odf = clicks(period, 0.1, 30.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let half = (15.0 / HOP) as usize;
        let moved = clicks(period, 0.35, 30.0);
        odf[half..].copy_from_slice(&moved[half..]);
        let answers = looks(&odf, period, 10.0);
        // Ten seconds after the move, the old phase has left the window.
        let late = answers.last().and_then(|a| a.0).expect("a beat at the end");
        let from_new = wrap(late - 0.35, period).abs();
        assert!(
            from_new < HOP * 1.5,
            "still {:.0} ms off the new beat",
            from_new * 1000.0
        );
    }

    /// Onsets that land a hop early or late at random: the answers
    /// asked fresh wander with them; the follower's settle.
    #[test]
    fn the_onsets_jitter_is_smoothed() {
        let period = 0.5;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (30.0 / HOP) as usize;
        let mut odf = vec![0.05f32; n];
        let mut seed = 7u32;
        let mut t = 0.2;
        while t < 30.0 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let jitter = f64::from(seed >> 16 & 3) - 1.5;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = (t / HOP + jitter).round() as usize;
            if i < n {
                odf[i] = 1.0;
            }
            t += period;
        }
        let answers = looks(&odf, period, 10.0);
        let followed: Vec<_> = answers.iter().map(|a| a.0).collect();
        let fresh: Vec<_> = answers.iter().map(|a| a.1).collect();
        assert!(
            biggest_move(&followed, period) < biggest_move(&fresh, period) * 0.6,
            "followed {:.1} ms, fresh {:.1} ms",
            biggest_move(&followed, period) * 1000.0,
            biggest_move(&fresh, period) * 1000.0
        );
    }

    /// A bar of nothing — a breakdown — keeps the beat for a moment and
    /// then lets it go; the music coming back finds it again.
    #[test]
    fn a_silence_lets_the_beat_go() {
        let period = 0.5;
        let mut odf = clicks(period, 0.1, 30.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (from, to) = ((14.0 / HOP) as usize, (24.0 / HOP) as usize);
        for v in &mut odf[from..to] {
            *v = 0.05;
        }
        let answers = looks(&odf, period, 10.0);
        let during = answers[(23.0 - 10.0) as usize * 4].0;
        assert_eq!(during, None, "a beat drawn over ten seconds of nothing");
        assert!(
            answers.last().and_then(|a| a.0).is_some(),
            "the beat did not come back"
        );
    }

    /// A band playing 2 % ahead of the tempo it was given: its beat
    /// slides ~10 ms a beat against the forecast. The follower keeps up
    /// in small steps and never lets go — no jump back to catch up — and
    /// forecasts at the band's pace, so the beats it draws two seconds
    /// ahead land on the clicks, where the given period alone would put
    /// them 40 ms late.
    #[test]
    fn a_band_ahead_of_its_tempo_is_followed_without_jumps() {
        let (given, played) = (0.5, 0.49);
        let odf = clicks(played, 0.1, 40.0);
        let answers = looks(&odf, given, 10.0);
        let followed: Vec<_> = answers.iter().map(|a| a.0).collect();
        assert!(
            followed.iter().all(Option::is_some),
            "the follower lost its beat"
        );
        let worst = biggest_move(&followed, given);
        assert!(
            worst < SAME_BEAT / 4.0 * HOP,
            "the forecast moved {:.1} ms",
            worst * 1000.0
        );
        let late = followed.last().copied().flatten().expect("a beat");
        let k = ((late - 0.1) / played).round();
        let off = (late - (0.1 + k * played)).abs();
        assert!(off < HOP * 1.5, "{:.1} ms behind the band", off * 1000.0);

        let mut follower = BeatFollower::new();
        let (mut worst_ahead, mut worst_given): (f64, f64) = (0.0, 0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (window, step, mut len) = (
            (PHASE_WINDOW_SECS / HOP).ceil() as usize + 1,
            (0.25 / HOP).round() as usize,
            (10.0 / HOP) as usize,
        );
        while len + (2.5 / HOP) as usize <= odf.len() {
            let start = len - window;
            #[allow(clippy::cast_precision_loss)]
            let (start_secs, now) = (start as f64 * HOP, (len - 1) as f64 * HOP);
            let b = follower.update(&odf[start..len], start_secs, HOP, given);
            if now > 16.0 {
                let b = b.expect("a beat");
                let off = |ahead: f64| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let near = (ahead / HOP).round() as usize;
                    #[allow(clippy::cast_precision_loss)]
                    (near - 30..near + 30)
                        .filter(|&i| odf.get(i) == Some(&1.0))
                        .map(|i| (i as f64 * HOP - ahead).abs())
                        .fold(f64::MAX, f64::min)
                };
                worst_ahead = worst_ahead.max(off(b.last_beat_secs + 4.0 * b.period_secs));
                worst_given = worst_given.max(off(b.last_beat_secs + 4.0 * given));
            }
            len += step;
        }
        // The clicks sit on whole hops, so they wobble up to half a hop
        // round the band's time — no forecast can know which way.
        assert!(
            worst_given > HOP * 2.0,
            "at the given period the forecast should miss — the test lost its bite"
        );
        assert!(
            worst_ahead < HOP * 2.0,
            "two seconds ahead, {:.1} ms off (the given period: {:.1} ms)",
            worst_ahead * 1000.0,
            worst_given * 1000.0
        );
    }

    /// Asked at every new onset instead of four times a second, the
    /// follower reads the same pace: it is read over seconds of record,
    /// not over a count of looks.
    #[test]
    fn looking_more_often_reads_the_same_pace() {
        let (given, played) = (0.5, 0.49);
        let odf = clicks(played, 0.1, 30.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let window = (PHASE_WINDOW_SECS / HOP).ceil() as usize + 1;
        let pace = |step: usize| {
            let mut follower = BeatFollower::new();
            let mut last = None;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mut len = (10.0 / HOP) as usize;
            while len <= odf.len() {
                let start = len - window;
                #[allow(clippy::cast_precision_loss)]
                let start_secs = start as f64 * HOP;
                last = follower
                    .update(&odf[start..len], start_secs, HOP, given)
                    .or(last);
                len += step;
            }
            last.expect("a beat").period_secs
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let quarter = (0.25 / HOP).round() as usize;
        let (every_look, every_hop) = (pace(quarter), pace(1));
        assert!(
            (every_hop - played).abs() < 0.002 && (every_look - played).abs() < 0.002,
            "pace {every_hop:.4} looking every hop, {every_look:.4} four times a second"
        );
    }

    /// Another tempo is another follower: the DJ halving it starts over
    /// rather than carrying a beat across.
    #[test]
    fn a_new_tempo_starts_over() {
        let odf = clicks(0.5, 0.1, 12.0);
        let mut follower = BeatFollower::new();
        assert!(follower.update(&odf, 0.0, HOP, 0.5).is_some());
        assert!(follower.update(&odf, 0.0, HOP, 1.0).is_some());
        assert!((follower.period - 1.0).abs() < 1e-9);
        assert!(follower.challenger.is_none());
    }
}
