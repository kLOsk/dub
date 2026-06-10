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
//! By default the generator emits a **bare carrier** (no bitstream) —
//! relative-mode tests only need the carrier, and a flat envelope keeps
//! the phase tracker's job trivial. [`Generator::enable_absolute`] turns
//! on **AM modulation** of the format's LFSR bitstream (one bit per
//! carrier cycle, M6), producing realistic position-encoded signals with
//! a known ground-truth position for absolute-decode tests.
//!
//! Used for:
//! 1. Decoder unit tests (generate at known rate → decode → check).
//! 2. The `dub decode-timecode` CLI's `--synthetic` mode for offline
//!    diagnosis without a turntable.

use crate::absolute::{lfsr_fwd, lfsr_output_bit, lfsr_rev};
use crate::Format;

/// Absolute-position modulation state (M6). When present, the generator
/// AM-modulates the carrier with the format's LFSR bitstream — one bit
/// per carrier cycle — so tests have realistic position-encoded signals
/// with known ground-truth position.
struct AbsMod {
    taps: u32,
    bits: u32,
    /// LFSR state for the current cycle; its MSB is this cycle's bit.
    state: u32,
    /// Absolute position in cycles from the seed (signed; reverse motion
    /// below the start goes negative). Ground truth for decoder tests.
    position: i64,
    /// Fraction the low-bit cycles are attenuated by, in `(0, 1)`.
    depth: f32,
}

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
    /// Absolute-position AM modulation, when enabled (M6 testing).
    abs_mod: Option<AbsMod>,
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
            abs_mod: None,
        }
    }

    /// Enable absolute-position AM modulation: each carrier cycle carries
    /// one LFSR bit, low bits attenuated by `depth ∈ (0,1)`. The bitstream
    /// starts at the format's seed (position 0). Returns `false` (and
    /// stays a bare carrier) for a format with no decoded bitstream
    /// (Traktor MK2). Off by default so bare-carrier tests are unaffected.
    pub fn enable_absolute(&mut self, format: Format, depth: f32) -> bool {
        match (format.lfsr_taps(), format.lfsr_seed()) {
            (Some(taps), Some(seed)) => {
                self.abs_mod = Some(AbsMod {
                    taps,
                    bits: format.position_bits(),
                    state: seed,
                    position: 0,
                    depth: depth.clamp(0.0, 0.95),
                });
                true
            }
            _ => false,
        }
    }

    /// Current ground-truth absolute position in cycles, when absolute
    /// modulation is enabled.
    #[must_use]
    pub fn absolute_position(&self) -> Option<i64> {
        self.abs_mod.as_ref().map(|m| m.position)
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
            // Per-cycle AM (M6 — the xwax Serato convention): the data bit
            // rides the **primary** channel's amplitude (ch1/right for
            // Serato), read at the secondary's zero-crossing. ch0
            // (secondary) stays a clean full-amplitude quadrature
            // reference for timing/direction. LFSR bit 1 → full amplitude,
            // bit 0 → attenuated (non-inverted — the tracker's raw
            // `peak > ref` bit equals the LFSR bit directly). The bit is
            // constant across a cycle, so carrier phase — and thus `rate`
            // decode — is untouched.
            let ch1_amp = amplitude * self.cycle_amp_factor();
            // Compute ch0/ch1, then advance phase. f64 trig + f32 cast
            // at the very end keeps phase continuity across block
            // boundaries tight (≪ 1e-9 rad drift over seconds at 48 kHz).
            // ch0 = sin(φ), ch1 = cos(φ) — so the secondary (sin) crosses
            // zero going up exactly when the primary (cos) peaks positive.
            #[allow(clippy::cast_possible_truncation)]
            let ch0 = (self.phase.sin() as f32) * amplitude;
            #[allow(clippy::cast_possible_truncation)]
            let ch1 = (self.phase.cos() as f32) * ch1_amp;
            frame[0] = ch0;
            frame[1] = ch1;
            self.phase += phase_step;
            // Keep the accumulator small to avoid catastrophic
            // cancellation from cos/sin of large arguments. A wrap is a
            // cycle boundary — advance the LFSR bitstream in the same
            // direction the carrier moved.
            if self.phase >= two_pi {
                self.phase -= two_pi;
                self.advance_cycle(true);
            } else if self.phase < 0.0 {
                self.phase += two_pi;
                self.advance_cycle(false);
            }
        }
    }

    /// Primary-channel amplitude multiplier for the current cycle's LFSR
    /// bit. Non-inverted (the xwax Serato convention): LFSR bit 1 → full
    /// (1.0), bit 0 → attenuated (`1 - depth`). No modulation → 1.0.
    fn cycle_amp_factor(&self) -> f32 {
        match &self.abs_mod {
            Some(m) if lfsr_output_bit(m.state, m.bits) == 0 => 1.0 - m.depth,
            _ => 1.0,
        }
    }

    /// Advance the LFSR one cycle in the carrier's direction.
    fn advance_cycle(&mut self, forward: bool) {
        if let Some(m) = self.abs_mod.as_mut() {
            if forward {
                m.state = lfsr_fwd(m.state, m.taps, m.bits);
                m.position += 1;
            } else {
                m.state = lfsr_rev(m.state, m.taps, m.bits);
                m.position -= 1;
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
    fn absolute_modulation_advances_position_at_carrier_rate() {
        // One cycle per carrier period: ~1000 cycles in 1 s at unity for
        // Serato's 1 kHz carrier. The ground-truth position must track it.
        let sr = 48_000.0_f32;
        let mut g = Generator::new(Format::SeratoCv02, sr);
        assert!(g.enable_absolute(Format::SeratoCv02, 0.4));
        assert_eq!(g.absolute_position(), Some(0));
        let mut buf = vec![0.0f32; 48_000 * 2]; // 1 s
        g.render(&mut buf, 1.0, 0.5);
        let pos = g.absolute_position().unwrap();
        assert!(
            (pos - 1000).abs() <= 1,
            "1 s at unity should advance ~1000 cycles, got {pos}"
        );
        // The bitstream modulates the primary (ch1) amplitude only (the
        // xwax Serato convention), so per-cycle ch1 RMS is not constant.
        // ch0 (secondary) is a clean reference and stays flat.
        let ch1: Vec<f32> = buf.iter().skip(1).step_by(2).copied().collect();
        let mut lo = f32::INFINITY;
        let mut hi = 0.0f32;
        for chunk in ch1.chunks_exact(48) {
            let r = rms(chunk);
            lo = lo.min(r);
            hi = hi.max(r);
        }
        assert!(hi - lo > 0.02, "expected visible ch1 AM, lo={lo} hi={hi}");
    }

    #[test]
    fn absolute_modulation_reverses() {
        let sr = 48_000.0_f32;
        let mut g = Generator::new(Format::SeratoCv02, sr);
        g.enable_absolute(Format::SeratoCv02, 0.4);
        let mut buf = vec![0.0f32; 4_800 * 2]; // 0.1 s forward
        g.render(&mut buf, 1.0, 0.5);
        let fwd = g.absolute_position().unwrap();
        assert!(fwd > 50, "forward advanced, got {fwd}");
        g.render(&mut buf, -1.0, 0.5); // 0.1 s reverse
        let back = g.absolute_position().unwrap();
        assert!(
            back.abs() <= 1,
            "reverse should return to ~start, fwd={fwd} back={back}"
        );
    }

    #[test]
    fn absolute_disabled_for_mk2() {
        let mut g = Generator::new(Format::TraktorMk2, 48_000.0);
        assert!(
            !g.enable_absolute(Format::TraktorMk2, 0.4),
            "MK2 has no decoded bitstream"
        );
        assert_eq!(g.absolute_position(), None);
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
