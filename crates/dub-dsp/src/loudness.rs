//! Integrated-loudness metering (ITU-R BS.1770-4 / EBU R128) and
//! the loudness-normalization gain it feeds.
//!
//! **This is an offline / analysis-time module, not an RT block.**
//! [`measure_integrated_loudness`] walks a whole decoded track and
//! allocates a per-block scratch vector — it runs on the library
//! analysis worker (`dub-library::analyze_track`), never on the
//! audio thread. The per-sample [`Biquad`] filtering itself is
//! alloc-free, but the gating pass collects block energies into a
//! `Vec`, so the crate-wide "no alloc in the inner loop" rule is
//! deliberately relaxed here. The result is a single number
//! persisted to `analysis_cache.lufs_i`; the *application* of the
//! derived gain happens on the deck at load time and is a plain
//! multiply (RT-safe), well away from this code.
//!
//! ## Why we measure, then normalize on load
//!
//! Auto-gain is applied **once, at load**, only when a stored LUFS
//! value already exists for the track. Analysis-in-flight is
//! store-only: a track analysed while it plays gets its loudness
//! written to the library but the live deck level is never touched
//! (retroactively jumping the level of a tune live in front of an
//! audience is unacceptable). The normalization gain therefore
//! takes effect the *next* time the track is loaded. See
//! `dub-library::Library::track_normalization_gain` for the read
//! path and `dub-ffi::DubEngine::load_track` for the apply path.

/// Reference loudness Dub normalizes toward, in LUFS. `-14` matches
/// the streaming-era convention (Spotify / YouTube / Tidal) and
/// leaves comfortable headroom on a club system versus the broadcast
/// `-23` LUFS target, which would run audibly quiet against records.
pub const DEFAULT_TARGET_LUFS: f64 = -14.0;

/// True-peak ceiling (dBFS) the normalization gain refuses to push a
/// track above. `-1.0` leaves 1 dB of headroom. We currently measure
/// **sample** peak (see [`LoudnessMeasurement::sample_peak_dbfs`]),
/// which can under-read the true inter-sample peak by up to ~0.5 dB,
/// so this is a slightly soft guarantee until oversampled true-peak
/// metering lands.
pub const CEILING_DBFS: f64 = -1.0;

/// Largest attenuation / boost (dB) the normalization gain is allowed
/// to apply, before the ceiling limiter. Bounds pathological inputs
/// (a near-silent stem measuring `-50` LUFS would otherwise ask for
/// +36 dB of boost). Real music sits well inside this band.
const MIN_GAIN_DB: f64 = -24.0;
const MAX_GAIN_DB: f64 = 24.0;

/// Result of one [`measure_integrated_loudness`] pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessMeasurement {
    /// Gated integrated loudness in LUFS, or `None` when the input
    /// is too short (under one 400 ms block) or so quiet that every
    /// block falls below the `-70 LUFS` absolute gate (silence /
    /// non-musical input). A `None` here is persisted as
    /// `has_lufs = 0`, and the load path then applies unity gain.
    pub lufs_i: Option<f64>,
    /// Sample peak across all channels, in dBFS. `-inf` is clamped to
    /// a finite floor. Stored in `analysis_cache.true_peak_dbtp` and
    /// used to keep the normalization gain from clipping. This is a
    /// sample peak, not a true (oversampled inter-sample) peak — the
    /// column name anticipates a future upgrade to true-peak metering.
    pub sample_peak_dbfs: f64,
}

/// dBFS value reported for pure-silence input (peak == 0), instead of
/// `-inf`, so the value round-trips through SQLite `REAL` cleanly.
const SILENCE_PEAK_DBFS: f64 = -120.0;

/// A transposed-Direct-Form-II biquad. Stateful across a single
/// channel's filtering pass; reset between channels.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// The two K-weighting stages, recomputed for the input sample rate.
///
/// Stage 1 is the BS.1770 high-shelf "head" filter (+~4 dB above
/// ~1.7 kHz); stage 2 is the ~38 Hz high-pass ("RLB" weighting). The
/// coefficient formulae are the standard bilinear-transform designs
/// used by libebur128, parameterised by `fs` so 44.1 kHz, 48 kHz and
/// 96 kHz tracks all weight correctly (a fixed 48 kHz coefficient set
/// would mistune everything else).
fn k_weighting_filters(fs: f64) -> (Biquad, Biquad) {
    use std::f64::consts::PI;

    // Stage 1 — high shelf.
    let f0 = 1681.974450955533;
    let g = 3.999843853973347;
    let q = 0.7071752369554196;
    let k = (PI * f0 / fs).tan();
    let vh = 10.0_f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad::new(
        (vh + vb * k / q + k * k) / a0,
        2.0 * (k * k - vh) / a0,
        (vh - vb * k / q + k * k) / a0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    );

    // Stage 2 — high pass (b = {1, -2, 1}).
    let f0 = 38.13547087602444;
    let q = 0.5003270373238773;
    let k = (PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let hpf = Biquad::new(
        1.0,
        -2.0,
        1.0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    );

    (shelf, hpf)
}

/// Measure the gated integrated loudness of an interleaved buffer.
///
/// `samples` is interleaved by `channels` (mono or stereo in Dub's
/// v1 path; higher channel counts are folded with unit weights,
/// which is correct for the L/R pair and a harmless approximation
/// otherwise). `sample_rate` is the track's native rate.
///
/// Follows BS.1770-4: K-weight each channel, take 400 ms
/// mean-square blocks at 100 ms hops (75 % overlap), then apply the
/// two-stage gate (`-70 LUFS` absolute, then `-10 LU` relative to
/// the absolute-gated mean) before averaging the surviving blocks.
#[must_use]
pub fn measure_integrated_loudness(
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
) -> LoudnessMeasurement {
    let ch = channels.max(1) as usize;
    let fs = f64::from(sample_rate.max(1));
    let frames = samples.len() / ch;

    let sample_peak_dbfs = sample_peak(samples);

    let block_frames = (0.4 * fs).round() as usize;
    let hop_frames = (0.1 * fs).round() as usize;
    if block_frames == 0 || hop_frames == 0 || frames < block_frames {
        return LoudnessMeasurement {
            lufs_i: None,
            sample_peak_dbfs,
        };
    }

    // K-weight every channel into a contiguous per-channel buffer.
    // `weighted[c][i]` holds the K-weighted sample for channel `c`,
    // frame `i`. One allocation per channel — offline only.
    let (shelf0, hpf0) = k_weighting_filters(fs);
    let mut weighted: Vec<Vec<f64>> = Vec::with_capacity(ch);
    for c in 0..ch {
        let mut shelf = shelf0;
        let mut hpf = hpf0;
        shelf.reset();
        hpf.reset();
        let mut col = vec![0.0_f64; frames];
        for (i, slot) in col.iter_mut().enumerate() {
            let x = f64::from(samples[i * ch + c]);
            *slot = hpf.process(shelf.process(x));
        }
        weighted.push(col);
    }

    // Block-energy z_j = Σ_c (mean square of K-weighted channel c).
    // Channel weights G_c are 1.0 for L/R (and we extend that to all
    // channels — see the doc note above).
    let n_blocks = (frames - block_frames) / hop_frames + 1;
    let mut block_energy: Vec<f64> = Vec::with_capacity(n_blocks);
    for b in 0..n_blocks {
        let start = b * hop_frames;
        let end = start + block_frames;
        let mut z = 0.0;
        for col in &weighted {
            let mut sum_sq = 0.0;
            for &s in &col[start..end] {
                sum_sq += s * s;
            }
            z += sum_sq / block_frames as f64;
        }
        block_energy.push(z);
    }

    // Loudness of a block from its energy. The `-0.691` offset is the
    // BS.1770 K-weighting calibration constant.
    let loudness = |z: f64| -> f64 {
        if z > 0.0 {
            -0.691 + 10.0 * z.log10()
        } else {
            f64::NEG_INFINITY
        }
    };

    // Absolute gate at -70 LUFS.
    let abs_gated: Vec<f64> = block_energy
        .iter()
        .copied()
        .filter(|&z| loudness(z) >= -70.0)
        .collect();
    if abs_gated.is_empty() {
        return LoudnessMeasurement {
            lufs_i: None,
            sample_peak_dbfs,
        };
    }

    // Relative gate: -10 LU below the mean loudness of the
    // absolute-gated blocks.
    let mean_abs = abs_gated.iter().sum::<f64>() / abs_gated.len() as f64;
    let relative_threshold = loudness(mean_abs) - 10.0;
    let gated: Vec<f64> = abs_gated
        .into_iter()
        .filter(|&z| loudness(z) >= relative_threshold)
        .collect();
    if gated.is_empty() {
        return LoudnessMeasurement {
            lufs_i: None,
            sample_peak_dbfs,
        };
    }

    let mean_gated = gated.iter().sum::<f64>() / gated.len() as f64;
    let lufs_i = loudness(mean_gated);

    LoudnessMeasurement {
        lufs_i: lufs_i.is_finite().then_some(lufs_i),
        sample_peak_dbfs,
    }
}

/// Sample peak across an interleaved buffer, in dBFS. Pure silence
/// reports [`SILENCE_PEAK_DBFS`] rather than `-inf`.
fn sample_peak(samples: &[f32]) -> f64 {
    let peak = samples.iter().fold(0.0_f32, |acc, &s| acc.max(s.abs()));
    if peak > 0.0 {
        20.0 * f64::from(peak).log10()
    } else {
        SILENCE_PEAK_DBFS
    }
}

/// Loudness-normalization gain in dB to bring a track measured at
/// `lufs_i` toward `target_lufs`, bounded to a sane range and then
/// limited so the resulting sample peak does not exceed `ceiling_dbfs`.
///
/// The ceiling limit only ever *reduces* upward gain — attenuation
/// passes through untouched — so a hot master is brought down to
/// target while a track that would clip is left just under the
/// ceiling instead.
#[must_use]
pub fn normalization_gain_db(
    lufs_i: f64,
    sample_peak_dbfs: f64,
    target_lufs: f64,
    ceiling_dbfs: f64,
) -> f64 {
    let desired = (target_lufs - lufs_i).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
    let headroom = ceiling_dbfs - sample_peak_dbfs;
    desired.min(headroom)
}

/// Convert a gain in dB to a linear multiplier (deck gain is linear).
#[must_use]
pub fn db_to_linear(db: f64) -> f32 {
    10.0_f64.powf(db / 20.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate an interleaved stereo sine at `freq` Hz with linear
    /// `amplitude`, `secs` long, at 48 kHz.
    fn sine(freq: f64, amplitude: f32, secs: f64) -> (Vec<f32>, u32, u16) {
        let fs = 48_000_u32;
        let frames = (secs * f64::from(fs)) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f64 / f64::from(fs);
            let s = amplitude * (2.0 * std::f64::consts::PI * freq * t).sin() as f32;
            out.push(s);
            out.push(s);
        }
        (out, fs, 2)
    }

    #[test]
    fn silence_has_no_loudness() {
        let buf = vec![0.0_f32; 48_000 * 2];
        let m = measure_integrated_loudness(&buf, 48_000, 2);
        assert!(m.lufs_i.is_none(), "silence must gate out to None");
        assert_eq!(m.sample_peak_dbfs, SILENCE_PEAK_DBFS);
    }

    #[test]
    fn too_short_returns_none() {
        // 100 ms — under one 400 ms block.
        let (buf, fs, ch) = sine(1000.0, 0.5, 0.1);
        let m = measure_integrated_loudness(&buf, fs, ch);
        assert!(m.lufs_i.is_none());
    }

    #[test]
    fn one_khz_tone_loudness_matches_bs1770_reference() {
        // A 1 kHz, 0.5-amplitude (-6 dBFS) sine in BOTH channels.
        // BS.1770 reference: a 0 dBFS 1 kHz dual-mono sine reads
        // ≈ 0 LUFS (the two correlated channels sum +3 dB, the
        // -0.691 offset and the +0.69 dB K-weight at 1 kHz cancel),
        // so -6 dBFS lands at ≈ -6 LUFS. Assert a tight band around
        // that known value — this pins the calibration, not just a
        // sanity range.
        let (buf, fs, ch) = sine(1000.0, 0.5, 4.0);
        let m = measure_integrated_loudness(&buf, fs, ch);
        let lufs = m.lufs_i.expect("a sustained tone must produce a value");
        assert!(
            (-7.0..-5.0).contains(&lufs),
            "1 kHz -6 dBFS dual-mono tone should read ≈ -6 LUFS, got {lufs}"
        );
    }

    #[test]
    fn louder_input_measures_higher() {
        let (quiet, fs, ch) = sine(1000.0, 0.1, 4.0);
        let (loud, _, _) = sine(1000.0, 0.5, 4.0);
        let lq = measure_integrated_loudness(&quiet, fs, ch).lufs_i.unwrap();
        let ll = measure_integrated_loudness(&loud, fs, ch).lufs_i.unwrap();
        // 5× amplitude ≈ +14 dB; assert a clear monotonic gap.
        assert!(
            ll > lq + 10.0,
            "louder tone must read >10 LU higher: {lq} vs {ll}"
        );
    }

    #[test]
    fn gain_brings_measurement_toward_target() {
        // A track at -20 LUFS with a -6 dBFS peak, target -14:
        // wants +6 dB, and the ceiling (-1) allows up to +5 dB
        // (−1 − (−6)). So the gain is ceiling-limited to +5 dB.
        let g = normalization_gain_db(-20.0, -6.0, -14.0, -1.0);
        assert!(
            (g - 5.0).abs() < 1e-9,
            "expected ceiling-limited +5 dB, got {g}"
        );
    }

    #[test]
    fn gain_attenuates_hot_master_without_ceiling_interference() {
        // -8 LUFS master, peak -0.2 dBFS, target -14: wants -6 dB.
        // Attenuation is never limited by the ceiling.
        let g = normalization_gain_db(-8.0, -0.2, -14.0, -1.0);
        assert!((g + 6.0).abs() < 1e-9, "expected -6 dB, got {g}");
        assert!(db_to_linear(g) < 1.0);
    }

    #[test]
    fn gain_is_bounded_for_near_silent_input() {
        // -60 LUFS would naively ask for +46 dB; clamp to +24 then
        // ceiling-limit against a quiet peak.
        let g = normalization_gain_db(-60.0, -40.0, -14.0, -1.0);
        assert!(g <= 24.0 + 1e-9);
    }

    #[test]
    fn db_linear_roundtrip() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(6.0206) - 2.0).abs() < 1e-3);
        assert!((db_to_linear(-6.0206) - 0.5).abs() < 1e-3);
    }
}
