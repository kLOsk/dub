//! Snare-on-2&4 + bass-on-1 downbeat refinement.
//!
//! Standard metrical-emphasis downbeat heuristic from the published MIR
//! literature: in most 4/4 popular music the snare/clap sits on beats 2 & 4
//! and the bass drum anchors beat 1. This is an independent implementation of
//! that long-established technique — Goto & Muraoka, "A Real-time Beat
//! Tracking System" (ICMC 1995); Davies & Plumbley, "A Spectral Difference
//! Approach to Downbeat Extraction" (EUSIPCO 2006); Hockman, Davies &
//! Fujinaga, "One in the Jungle" (ISMIR 2012) — not any vendor's product or
//! patent. See `docs/spec/LICENSE-DEPENDENCIES.md` for the prior-art note.
//!
//! ## Why this exists
//!
//! dub's default downbeat picker ([`crate::beats`]'s `find_downbeat_offset`)
//! votes on the kick-band ODF alone. That is enough when the kick lands on
//! bar position 1, but it cannot separate bar 1 from bar 3 — both carry a
//! kick in most 4/4 material — and it has no snare evidence at all. This
//! rule adds the constraint the kick ODF is missing: in almost all popular
//! music the **snare/clap sits on beats 2 & 4** and the **bass drum
//! anchors beat 1**. Two independent whole-track tests combine into a
//! single bar position:
//!
//! 1. **Backbeat parity** (the backbeat-parity test): the two
//!    bar positions carrying the most snare-band onset energy are 2 & 4.
//!    That fixes the downbeat to one of the *other* two positions.
//! 2. **Bass anchor** (the bass-anchor test): of those two remaining
//!    positions, the one with more kick-band onset energy is the 1.
//!
//! Unlike the kick-ODF picker this works off the **audio**, not the ODF,
//! so it can use the standard snare/kick passbands (snare 300 Hz – 2.5 kHz, kick
//! < 240 Hz) rather than the 8 log bands the ODF front-end happens to lay
//! out. It is an *opt-in* refinement: hand it a finished [`BeatGrid`] and
//! it returns the bar phase it judges to be the downbeat, leaving every
//! other grid property untouched — a pure rotation, exactly like
//! [`crate::bar_phase_from_tap`]. The caller decides whether to apply it
//! (e.g. only when [`DownbeatRefinement::confidence`] clears a bar).
//!
//! ## Scope
//!
//! Only 4/4 (`beats_per_bar == 4`) is handled — the snare-2&4 / bass-1
//! constraint is meterspecific and dub is 4/4 in v0. Other meters return
//! `None`. Analysis is off-RT, whole-track-in-RAM, single pass over the
//! audio.

use crate::BeatGrid;

/// The bar phase chosen by the snare/bass downbeat rule, plus a confidence.
///
/// `bar_phase` has the same meaning as [`BeatGrid::bar_phase`]: the index
/// into `beats` of the first downbeat (`beats[bar_phase]`,
/// `beats[bar_phase + beats_per_bar]`, … are bar position 1). To apply the
/// refinement, set `grid.bar_phase = refinement.bar_phase` — nothing else
/// changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownbeatRefinement {
    /// Bar phase in `0..beats_per_bar` judged to be the downbeat.
    pub bar_phase: u8,
    /// Agreement of the two tests, in `[0, 1]`. The geometric mean of the
    /// snare backbeat contrast and the kick anchor contrast — high only
    /// when *both* the snare-on-2&4 and the bass-on-1 evidence are
    /// decisive. `0` means "no usable evidence; do not trust `bar_phase`".
    pub confidence: f32,
}

/// Snare passband: low-pass corner (Hz). Snare = LPF 2.5 kHz then
/// HPF 300 Hz, isolating the snare/clap body + crack while rejecting the
/// kick fundamental and the brightest hats.
const SNARE_LPF_HZ: f64 = 2500.0;
/// Snare passband: high-pass corner (Hz).
const SNARE_HPF_HZ: f64 = 300.0;
/// Kick passband: outer low-pass corner (Hz). Kick = LPF 2.5 kHz
/// then LPF 240 Hz; the cascade steepens the rolloff so snare/hat energy
/// does not leak into the bass-drum measurement.
const KICK_LPF1_HZ: f64 = 2500.0;
/// Kick passband: inner low-pass corner (Hz).
const KICK_LPF2_HZ: f64 = 240.0;

/// One-pole envelope time constant (seconds). ~5 ms tracks a drum attack
/// without smearing adjacent hits; the positive first difference of this
/// envelope is the onset/attack signal differentiated for.
const ENV_TAU_SECS: f64 = 0.005;

/// Half-width of the per-beat measurement window, as a fraction of the
/// beat period. A hit's onset spike lands within a few tens of ms of the
/// beat; 15 % of the period (clamped) captures it without reaching into
/// the neighbouring beat (`2 × 0.09 s < 0.30 s`, the period at 200 BPM).
const WINDOW_PERIOD_FRACTION: f64 = 0.15;
/// Lower clamp on the window half-width (seconds).
const WINDOW_MIN_SECS: f64 = 0.02;
/// Upper clamp on the window half-width (seconds).
const WINDOW_MAX_SECS: f64 = 0.09;

/// Bar positions the rule resolves (4/4 only).
const BEATS_PER_BAR: usize = 4;

/// Refine a grid's downbeat with the snare-2&4 + bass-on-1 rule.
///
/// `samples` is interleaved (`L R L R …` for stereo, `M M …` for mono);
/// stereo is downmixed to mono internally (mean of L+R), matching
/// [`crate::analyze_bpm`]. `grid` supplies the beat positions whose phase
/// is being decided — its `bpm`, `beats`, and spacing are read but never
/// modified.
///
/// Returns `None` when the rule cannot decide: non-4/4 meter, fewer than
/// two bars of beats, a zero/invalid sample rate, malformed interleaving,
/// or no snare-band onset energy at all (e.g. silence, a beatless pad).
/// In every `None` case the caller should keep the grid's existing
/// `bar_phase`.
#[must_use]
pub fn refine_downbeat_backbeat(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    grid: &BeatGrid,
) -> Option<DownbeatRefinement> {
    if sample_rate == 0 || !(1..=2).contains(&channels) {
        return None;
    }
    if grid.beats_per_bar as usize != BEATS_PER_BAR {
        return None;
    }
    if grid.beats.len() < BEATS_PER_BAR * 2 {
        return None;
    }
    if !samples.len().is_multiple_of(usize::from(channels)) {
        return None;
    }

    let period = if grid.bpm.is_finite() && grid.bpm > 0.0 {
        60.0 / grid.bpm
    } else {
        // Fall back to the median inter-beat spacing if bpm is absent.
        median_spacing(&grid.beats)?
    };
    if !(period.is_finite() && period > 0.0) {
        return None;
    }

    let fs = f64::from(sample_rate);
    let window = (period * WINDOW_PERIOD_FRACTION).clamp(WINDOW_MIN_SECS, WINDOW_MAX_SECS);

    let mut snare_pos = [0.0f64; BEATS_PER_BAR];
    let mut kick_pos = [0.0f64; BEATS_PER_BAR];

    accumulate_band_energy(
        samples,
        channels,
        fs,
        &grid.beats,
        window,
        &mut snare_pos,
        &mut kick_pos,
    );

    decide(&snare_pos, &kick_pos)
}

/// Single pass over the audio: band-split into snare/kick, onset-envelope
/// each, and add each sample's attack energy to the bar position of the
/// nearest beat (when inside that beat's window).
fn accumulate_band_energy(
    samples: &[f32],
    channels: u8,
    fs: f64,
    beats: &[f64],
    window: f64,
    snare_pos: &mut [f64; BEATS_PER_BAR],
    kick_pos: &mut [f64; BEATS_PER_BAR],
) {
    let mut s_lpf = Biquad::lowpass(fs, SNARE_LPF_HZ);
    let mut s_hpf = Biquad::highpass(fs, SNARE_HPF_HZ);
    let mut k_lpf1 = Biquad::lowpass(fs, KICK_LPF1_HZ);
    let mut k_lpf2 = Biquad::lowpass(fs, KICK_LPF2_HZ);

    #[allow(clippy::cast_possible_truncation)]
    let env_a = (1.0 - (-1.0 / (ENV_TAU_SECS * fs)).exp()) as f32;
    let mut s_env = 0.0f32;
    let mut s_prev = 0.0f32;
    let mut k_env = 0.0f32;
    let mut k_prev = 0.0f32;

    let ch = usize::from(channels);
    // Pointer into `beats`: advanced so we never re-scan earlier beats.
    // Windows are non-overlapping (see `WINDOW_MAX_SECS`), so each sample
    // contributes to at most one beat.
    let mut bi = 0usize;

    for (frame, chunk) in samples.chunks_exact(ch).enumerate() {
        let x = if ch == 1 {
            chunk[0]
        } else {
            0.5 * (chunk[0] + chunk[1])
        };

        let s = s_hpf.process(s_lpf.process(x));
        let k = k_lpf2.process(k_lpf1.process(x));

        s_env += (s.abs() - s_env) * env_a;
        let s_att = (s_env - s_prev).max(0.0);
        s_prev = s_env;

        k_env += (k.abs() - k_env) * env_a;
        let k_att = (k_env - k_prev).max(0.0);
        k_prev = k_env;

        #[allow(clippy::cast_precision_loss)]
        let t = frame as f64 / fs;

        // Advance past beats whose window has fully ended.
        while bi < beats.len() && t > beats[bi] + window {
            bi += 1;
        }
        if bi >= beats.len() {
            break;
        }
        if t >= beats[bi] - window {
            let pos = bi % BEATS_PER_BAR;
            snare_pos[pos] += f64::from(s_att);
            kick_pos[pos] += f64::from(k_att);
        }
    }
}

/// Combine the two tests into a bar phase + confidence.
fn decide(
    snare_pos: &[f64; BEATS_PER_BAR],
    kick_pos: &[f64; BEATS_PER_BAR],
) -> Option<DownbeatRefinement> {
    let snare_even = snare_pos[0] + snare_pos[2];
    let snare_odd = snare_pos[1] + snare_pos[3];
    let snare_total = snare_even + snare_odd;
    if snare_total <= 0.0 {
        return None;
    }

    // Backbeat parity: the position *set* with more snare energy is 2 & 4,
    // so the downbeat is in the complementary set.
    let backbeats_odd = snare_odd > snare_even;
    let (a, b) = if backbeats_odd {
        (0usize, 2usize)
    } else {
        (1usize, 3usize)
    };

    // Bass anchor: of the two downbeat candidates, the one with more kick
    // energy is the 1.
    let downbeat = if kick_pos[a] >= kick_pos[b] { a } else { b };

    let snare_contrast = (snare_odd - snare_even).abs() / snare_total;
    let kick_ab = kick_pos[a] + kick_pos[b];
    let kick_contrast = if kick_ab > 0.0 {
        (kick_pos[a] - kick_pos[b]).abs() / kick_ab
    } else {
        0.0
    };

    #[allow(clippy::cast_possible_truncation)]
    let confidence = (snare_contrast * kick_contrast).sqrt() as f32;

    Some(DownbeatRefinement {
        bar_phase: u8::try_from(downbeat).unwrap_or(0),
        confidence: confidence.clamp(0.0, 1.0),
    })
}

/// Median spacing between consecutive beats. Fallback period source when
/// the grid carries no usable `bpm`.
fn median_spacing(beats: &[f64]) -> Option<f64> {
    if beats.len() < 2 {
        return None;
    }
    let mut diffs: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    diffs.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let mid = diffs.len() / 2;
    Some(diffs[mid])
}

/// Direct-form-I biquad. Off-RT analysis filter (snare/kick band splits);
/// no `RealtimeContext` because this never touches the audio thread.
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// RBJ-cookbook low-pass at corner `fc` (Hz), Butterworth Q.
    fn lowpass(fs: f64, fc: f64) -> Self {
        Self::cookbook(fs, fc, false)
    }

    /// RBJ-cookbook high-pass at corner `fc` (Hz), Butterworth Q.
    fn highpass(fs: f64, fc: f64) -> Self {
        Self::cookbook(fs, fc, true)
    }

    fn cookbook(fs: f64, fc: f64, highpass: bool) -> Self {
        use std::f64::consts::PI;
        const Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
        let w0 = 2.0 * PI * (fc / fs).min(0.49);
        let cos0 = w0.cos();
        let sin0 = w0.sin();
        let alpha = sin0 / (2.0 * Q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if highpass {
            let c = (1.0 + cos0) / 2.0;
            (c, -(1.0 + cos0), c)
        } else {
            let c = (1.0 - cos0) / 2.0;
            (c, 1.0 - cos0, c)
        };
        let a1 = -2.0 * cos0;
        let a2 = 1.0 - alpha;
        #[allow(clippy::cast_possible_truncation)]
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// Add a windowed tone burst at `t_secs` to a mono buffer — a stand-in
    /// for a drum hit whose energy sits in one band.
    fn add_hit(buf: &mut [f32], fs: f64, t_secs: f64, freq: f64, amp: f32) {
        let dur = 0.08;
        let start = (t_secs * fs) as usize;
        let len = (dur * fs) as usize;
        for n in 0..len {
            let i = start + n;
            if i >= buf.len() {
                break;
            }
            #[allow(clippy::cast_precision_loss)]
            let tt = n as f64 / fs;
            // Fast attack, exponential decay so the onset is a sharp spike.
            let env = (-tt / 0.03).exp();
            #[allow(clippy::cast_possible_truncation)]
            let s = (amp * env as f32) * (2.0 * std::f64::consts::PI * freq * tt).sin() as f32;
            buf[i] += s;
        }
    }

    /// Build a 4/4 grid at `bpm` with `n_beats` beats from t=0.
    fn grid_at(bpm: f64, n_beats: usize, bar_phase: u8) -> BeatGrid {
        let period = 60.0 / bpm;
        #[allow(clippy::cast_precision_loss)]
        let beats = (0..n_beats).map(|i| i as f64 * period).collect();
        BeatGrid {
            bpm,
            confidence: 1.0,
            beats,
            beats_per_bar: 4,
            bar_phase,
            quality: None,
            downbeat_confidence: 0.0,
        }
    }

    const KICK_HZ: f64 = 55.0;
    const SNARE_HZ: f64 = 1000.0;

    #[test]
    fn resolves_downbeat_when_kick_on_1_and_3() {
        // The case the kick-ODF picker can't solve: kick on bar positions
        // 0 AND 2 (musical 1 & 3), snare on 1 & 3 (musical 2 & 4). Only the
        // snare backbeat + the louder beat-1 kick disambiguate to phase 0.
        let bpm = 120.0;
        let period = 60.0 / bpm;
        let n_beats = 32; // 8 bars
        let fs = f64::from(SR);
        let mut buf = vec![0.0f32; ((n_beats as f64 + 1.0) * period * fs) as usize];

        for i in 0..n_beats {
            let t = i as f64 * period;
            match i % 4 {
                0 => add_hit(&mut buf, fs, t, KICK_HZ, 1.0), // strong beat-1 kick
                2 => add_hit(&mut buf, fs, t, KICK_HZ, 0.6), // weaker beat-3 kick
                1 | 3 => add_hit(&mut buf, fs, t, SNARE_HZ, 0.9), // backbeat snare
                _ => unreachable!(),
            }
        }

        let grid = grid_at(bpm, n_beats, 2); // deliberately wrong phase
        let r = refine_downbeat_backbeat(&buf, SR, 1, &grid).expect("should decide");
        assert_eq!(r.bar_phase, 0, "kick-on-1&3 case must resolve to phase 0");
        assert!(
            r.confidence > 0.2,
            "confidence should be decisive, got {}",
            r.confidence
        );
    }

    #[test]
    fn resolves_rotated_pattern() {
        // Snare on positions 0 & 2, kick on position 1 → backbeats are the
        // even set, downbeat candidates {1,3}, kick picks 1.
        let bpm = 100.0;
        let period = 60.0 / bpm;
        let n_beats = 32;
        let fs = f64::from(SR);
        let mut buf = vec![0.0f32; ((n_beats as f64 + 1.0) * period * fs) as usize];

        for i in 0..n_beats {
            let t = i as f64 * period;
            match i % 4 {
                1 => add_hit(&mut buf, fs, t, KICK_HZ, 1.0),
                0 | 2 => add_hit(&mut buf, fs, t, SNARE_HZ, 0.9),
                _ => {}
            }
        }

        let grid = grid_at(bpm, n_beats, 0);
        let r = refine_downbeat_backbeat(&buf, SR, 1, &grid).expect("should decide");
        assert_eq!(r.bar_phase, 1, "rotated pattern must resolve to phase 1");
    }

    #[test]
    fn stereo_downmix_matches_mono() {
        let bpm = 120.0;
        let period = 60.0 / bpm;
        let n_beats = 16;
        let fs = f64::from(SR);
        let mut mono = vec![0.0f32; ((n_beats as f64 + 1.0) * period * fs) as usize];
        for i in 0..n_beats {
            let t = i as f64 * period;
            match i % 4 {
                0 => add_hit(&mut mono, fs, t, KICK_HZ, 1.0),
                1 | 3 => add_hit(&mut mono, fs, t, SNARE_HZ, 0.9),
                _ => {}
            }
        }
        let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();

        let grid = grid_at(bpm, n_beats, 0);
        let m = refine_downbeat_backbeat(&mono, SR, 1, &grid).expect("mono decides");
        let s = refine_downbeat_backbeat(&stereo, SR, 2, &grid).expect("stereo decides");
        assert_eq!(m.bar_phase, s.bar_phase);
    }

    #[test]
    fn silence_returns_none() {
        let grid = grid_at(120.0, 32, 0);
        let buf = vec![0.0f32; SR as usize * 4];
        assert!(refine_downbeat_backbeat(&buf, SR, 1, &grid).is_none());
    }

    #[test]
    fn non_four_four_returns_none() {
        let mut grid = grid_at(120.0, 32, 0);
        grid.beats_per_bar = 3;
        let buf = vec![0.1f32; SR as usize * 4];
        assert!(refine_downbeat_backbeat(&buf, SR, 1, &grid).is_none());
    }

    #[test]
    fn too_few_beats_returns_none() {
        let grid = grid_at(120.0, 4, 0); // one bar
        let buf = vec![0.1f32; SR as usize * 4];
        assert!(refine_downbeat_backbeat(&buf, SR, 1, &grid).is_none());
    }

    #[test]
    fn odd_stereo_length_returns_none() {
        let grid = grid_at(120.0, 32, 0);
        let buf = vec![0.1f32; 1001]; // not a multiple of 2
        assert!(refine_downbeat_backbeat(&buf, SR, 2, &grid).is_none());
    }

    #[test]
    fn lowpass_attenuates_above_corner() {
        // Sanity on the biquad: a 5 kHz tone is strongly cut by a 240 Hz LPF.
        let fs = f64::from(SR);
        let mut lp = Biquad::lowpass(fs, 240.0);
        let mut peak = 0.0f32;
        for n in 0..SR as usize {
            let t = n as f64 / fs;
            let x = (2.0 * std::f64::consts::PI * 5000.0 * t).sin() as f32;
            peak = peak.max(lp.process(x).abs());
        }
        assert!(
            peak < 0.1,
            "5 kHz should be cut by a 240 Hz LPF, got {peak}"
        );
    }
}
