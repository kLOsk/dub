//! A band-limited square for the siren chips.
//!
//! The HK628 and SN76477 voices drew a naive square — `±1` by the phase —
//! whose every edge is a full-height step landing on the sample grid. Its
//! harmonics above Nyquist fold back as tones that are not in the series:
//! at the Rifle Gun's 3150 Hz the folded energy sat ~3 dB under the real
//! harmonics, and on the Laser's sweep the aliases ran the opposite way to
//! the pitch (siren review, 2026-11-14). On a big rig that is the fizzy edge
//! that pierces, and it is not the chips' character — the HK628 clocks at
//! 128 kHz and the SN76477 is analog.
//!
//! PolyBLEP rounds each edge over the two samples around it with a
//! polynomial step residual: the same harmonics up to Nyquist, the fold-back
//! down ~40 dB, and two multiply-adds per edge. RT-safe, no state beyond the
//! caller's phase.

/// The polynomial band-limited step residual at phase `t` for a phase
/// increment `dt` per sample (both in cycles): non-zero only within one
/// sample either side of the wrap at `t = 0`.
#[inline]
#[must_use]
pub(crate) fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        0.0
    } else if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// A 50 % square at phase `phase` (`0..1`, high in the first half) with
/// phase increment `dt`, both edges band-limited.
#[inline]
#[must_use]
pub(crate) fn square(phase: f32, dt: f32) -> f32 {
    pulse(phase, dt, 0.5)
}

/// A pulse high for the first `duty` of the cycle (`0 < duty < 1`), both
/// edges band-limited — the SN76477's VCO, whose duty is a knob.
#[inline]
#[must_use]
pub(crate) fn pulse(phase: f32, dt: f32, duty: f32) -> f32 {
    let naive = if phase < duty { 1.0 } else { -1.0 };
    let falling = if phase >= duty {
        phase - duty
    } else {
        phase - duty + 1.0
    };
    naive + poly_blep(phase, dt) - poly_blep(falling, dt)
}

#[cfg(test)]
pub(crate) mod measure {
    //! What the voice tests measure with.

    /// Magnitude of `freq` in `x` (Goertzel), normalised so a full-scale
    /// sine reads ~0.5.
    #[must_use]
    pub(crate) fn tone(x: &[f32], freq: f32, sample_rate: f32) -> f32 {
        let w = std::f32::consts::TAU * freq / sample_rate;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in x {
            let s0 = f64::from(v) + f64::from(coeff) * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let power = s1 * s1 + s2 * s2 - f64::from(coeff) * s1 * s2;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let mag = (power.max(0.0).sqrt() / x.len() as f64) as f32;
        mag
    }

    /// Mean absolute level of `x`.
    #[must_use]
    pub(crate) fn mean_abs(x: &[f32]) -> f32 {
        if x.is_empty() {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = x.len() as f32;
        x.iter().map(|v| v.abs()).sum::<f32>() / n
    }
}

#[cfg(test)]
mod tests {
    use super::measure::tone;
    use super::*;

    fn render(freq: f32, band_limited: bool) -> Vec<f32> {
        let sr = 48_000.0;
        let dt = freq / sr;
        let mut phase = 0.0f32;
        (0..48_000)
            .map(|_| {
                let v = if band_limited {
                    square(phase, dt)
                } else if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                };
                phase += dt;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                v
            })
            .collect()
    }

    /// At 3150 Hz the 15th harmonic (47 250 Hz) folds to 750 Hz — a tone
    /// that is not in the series. Band-limited, it is gone; the
    /// fundamental is not touched.
    #[test]
    fn the_band_limited_square_drops_the_fold_back() {
        let naive = render(3_150.0, false);
        let blep = render(3_150.0, true);
        let sr = 48_000.0;
        let fund = tone(&blep, 3_150.0, sr);
        let alias_naive = tone(&naive, 750.0, sr) / tone(&naive, 3_150.0, sr);
        let alias_blep = tone(&blep, 750.0, sr) / fund;
        assert!(
            (fund - tone(&naive, 3_150.0, sr)).abs() < fund * 0.05,
            "fundamental moved"
        );
        assert!(
            alias_blep < alias_naive * 0.1,
            "fold-back {:.1} dB, naive {:.1} dB",
            20.0 * alias_blep.log10(),
            20.0 * alias_naive.log10()
        );
    }

    #[test]
    fn the_band_limited_square_stays_in_range() {
        for &f in &[30.0, 440.0, 3_150.0, 9_000.0] {
            assert!(render(f, true).iter().all(|v| v.abs() <= 1.0 + 1e-3));
        }
    }
}
