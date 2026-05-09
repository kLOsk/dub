//! Synthetic timecode signal generator.
//!
//! Produces a stereo carrier at the format's nominal frequency
//! matching the Serato CV02 convention observed on real cartridges:
//! `ch0 = A·sin(φ)`, `ch1 = A·cos(φ)` — ch0 *leads* ch1 by 90°.
//! The decoder treats `s = ch1 + j·ch0`, so the complex envelope
//! `s = A·e^(jφ)` rotates positively for forward stylus motion.
//! Reverse motion (manual rewind, scratch) decreases `φ`, so `s`
//! rotates the other way and the decoder reports negative rate.
//!
//! The generator does NOT yet AM-modulate the bit pattern. Relative
//! mode (M5.1) only needs the carrier; the bitstream is a byproduct
//! that lands when we want absolute position (M6). For now,
//! synthetically-generated signals decode trivially because there's no
//! amplitude envelope to confuse the phase tracker — exactly what we
//! want for the first round of TDD.
//!
//! Used for:
//! 1. Decoder unit tests (generate at known rate → decode → check).
//! 2. The `dub decode-timecode` CLI's `--synthetic` mode for offline
//!    diagnosis without a turntable.

use crate::Format;

/// Stateful generator. One instance per virtual deck.
///
/// Construct with [`Generator::new`], then call [`Generator::render`]
/// to fill a stereo buffer. The generator integrates phase across
/// calls, so consecutive blocks at the same rate produce a continuous
/// signal — no clicks at block boundaries, which would otherwise
/// poison decoder tests.
pub struct Generator {
    sample_rate: f32,
    carrier_hz: f32,
    /// Current phase of the local oscillator, in radians, in `[0, 2π)`.
    /// Stored as `f64` because phase accumulators are notorious for
    /// drift at `f32` precision over seconds-long renders.
    phase: f64,
}

impl Generator {
    /// Create a generator for the given timecode format and engine SR.
    ///
    /// # Panics
    /// `sample_rate` must be positive.
    #[must_use]
    pub fn new(format: Format, sample_rate: f32) -> Self {
        assert!(sample_rate > 0.0, "sample rate must be > 0");
        Self {
            sample_rate,
            carrier_hz: format.carrier_hz(),
            phase: 0.0,
        }
    }

    /// Reset phase to zero. Useful when restarting a test scenario at
    /// a known starting point.
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Fill a stereo (interleaved) buffer with timecode at `rate × unity`.
    ///
    /// `rate = 1.0` is forward unity; `rate = 0.0` is the stylus
    /// resting on the groove without rotation; `rate < 0.0` is reverse.
    /// Higher absolute values speed up the carrier proportionally.
    ///
    /// `amplitude` is the peak value; a real cartridge typically yields
    /// 0.3–0.7 in the engine's `[-1.0, 1.0]` linear domain depending on
    /// gain staging. Tests should pick a value that won't clip after
    /// any subsequent processing.
    ///
    /// # Panics
    /// `out.len()` must be even (interleaved stereo).
    pub fn render(&mut self, out: &mut [f32], rate: f64, amplitude: f32) {
        assert_eq!(out.len() % 2, 0, "interleaved stereo buffer required");
        let two_pi = std::f64::consts::TAU;
        let phase_step = two_pi * f64::from(self.carrier_hz) / f64::from(self.sample_rate) * rate;
        for frame in out.chunks_exact_mut(2) {
            // Compute ch0/ch1, then advance phase. f64 trig + f32 cast
            // at the very end keeps phase continuity across block
            // boundaries tight (≪ 1e-9 rad drift over seconds at 48 kHz).
            // ch0 = sin(φ), ch1 = cos(φ) — Serato CV02 convention so
            // the decoder reports +rate for forward play.
            #[allow(clippy::cast_possible_truncation)]
            let ch0 = (self.phase.sin() as f32) * amplitude;
            #[allow(clippy::cast_possible_truncation)]
            let ch1 = (self.phase.cos() as f32) * amplitude;
            frame[0] = ch0;
            frame[1] = ch1;
            self.phase += phase_step;
            // Keep the accumulator small to avoid catastrophic
            // cancellation from cos/sin of large arguments. A single
            // wrap is a no-op on the signal but a big help to f64.
            if self.phase >= two_pi {
                self.phase -= two_pi;
            } else if self.phase < 0.0 {
                self.phase += two_pi;
            }
        }
    }

    /// Current phase, in radians in `[0, 2π)`.
    #[must_use]
    pub fn phase(&self) -> f64 {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(buf: &[f32]) -> f32 {
        if buf.is_empty() {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = buf.len() as f64;
        let sum: f64 = buf.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        #[allow(clippy::cast_possible_truncation)]
        ((sum / n).sqrt() as f32)
    }

    #[test]
    fn unity_render_is_quadrature() {
        // 1 second at unity rate, 48 kHz, amplitude 0.5.
        let mut g = Generator::new(Format::SeratoCv02, 48_000.0);
        let mut buf = vec![0.0f32; 48_000 * 2];
        g.render(&mut buf, 1.0, 0.5);
        // RMS of a pure sine of amplitude 0.5 = 0.5 / √2 ≈ 0.3536.
        let l_rms = rms(&buf.iter().step_by(2).copied().collect::<Vec<_>>());
        let r_rms = rms(&buf.iter().skip(1).step_by(2).copied().collect::<Vec<_>>());
        assert!((l_rms - 0.5 / 2.0_f32.sqrt()).abs() < 0.01);
        assert!((r_rms - 0.5 / 2.0_f32.sqrt()).abs() < 0.01);
    }

    #[test]
    fn quadrature_relationship_holds() {
        // L and R are π/2 apart in phase: L²+R² = A² (Pythagoras),
        // and L·R averages to zero over a full cycle.
        let mut g = Generator::new(Format::SeratoCv02, 48_000.0);
        let mut buf = vec![0.0f32; 48_000 * 2];
        g.render(&mut buf, 1.0, 0.7);
        let mut max_unit_circle_err: f32 = 0.0;
        for frame in buf.chunks_exact(2) {
            let r2 = frame[0] * frame[0] + frame[1] * frame[1];
            // Should equal A² = 0.49 ± float roundoff.
            let err = (r2 - 0.49).abs();
            if err > max_unit_circle_err {
                max_unit_circle_err = err;
            }
        }
        assert!(
            max_unit_circle_err < 1e-4,
            "max |L²+R² - A²| = {max_unit_circle_err}"
        );
    }

    #[test]
    fn phase_advances_continuously_across_blocks() {
        // Rendering two short blocks should be bit-equivalent to
        // rendering one combined block. This is what guarantees
        // synthetic signals don't introduce phase discontinuities at
        // block boundaries, which would corrupt decoder tests.
        let mut g1 = Generator::new(Format::SeratoCv02, 48_000.0);
        let mut combined = vec![0.0f32; 256 * 2];
        g1.render(&mut combined, 1.0, 0.5);

        let mut g2 = Generator::new(Format::SeratoCv02, 48_000.0);
        let mut split = vec![0.0f32; 256 * 2];
        g2.render(&mut split[..128 * 2], 1.0, 0.5);
        g2.render(&mut split[128 * 2..], 1.0, 0.5);

        for (a, b) in combined.iter().zip(split.iter()) {
            assert!((a - b).abs() < 1e-6, "drift {a} vs {b}");
        }
    }

    #[test]
    fn zero_rate_emits_dc_offset_signal() {
        // Rate=0 means the stylus isn't moving — phase is frozen at
        // 0, so the output is constant ch0=sin(0)=0, ch1=cos(0)=1·A.
        let mut g = Generator::new(Format::SeratoCv02, 48_000.0);
        let mut buf = vec![0.0f32; 64 * 2];
        g.render(&mut buf, 0.0, 0.5);
        for frame in buf.chunks_exact(2) {
            assert!(frame[0].abs() < 1e-6);
            assert!((frame[1] - 0.5).abs() < 1e-6);
        }
    }
}
