//! The **"Big Knob"** — King Tubby's bass-drop high-pass. Tubby swept an Altec
//! 9069B passive program filter live, hauling the low end out from under a tune
//! and slamming it back — the single most recognisable dub mixing-desk move.
//!
//! ## Model
//!
//! A 2-pole (12 dB/oct) high-pass built on a **TPT state-variable filter**
//! (Zavalishin/Simper topology), chosen because it stays stable under fast
//! cutoff modulation — exactly what a live sweep demands. The cutoff is fully
//! continuous (for the Advanced macro and Expert sweep) and also snaps to the
//! Altec's **11 stepped positions** ([`BIG_KNOB_STEPS`], ~70 Hz–7.5 kHz) for a
//! faithful Expert feel. Resonance is adjustable for character (the passive unit
//! is gentle; we default near Butterworth).
//!
//! It is an **insert**: [`BigKnobHpf::process_block`] filters the signal **in
//! place** (a high-pass is 100 % wet — at the lowest cutoff it's near-bypass;
//! swept up it removes the lows). Each channel keeps its own state so stereo is
//! preserved.
//!
//! ## Real-time safety
//!
//! The prewarped cutoff coefficient (`tan`) is resolved **off-RT** in
//! `set_cutoff_*` / `set_step`; `process_block` only runs the SVF recurrence
//! (adds, mults, one divide per frame) over a smoothed coefficient, so cutoff
//! moves are click-free without any transcendental on the audio thread. State is
//! denormal-flushed. No allocation, locks or syscalls in the loop. Verified
//! under `assert_no_alloc`.

use crate::echo::one_pole_coeff;

const DENORMAL_FLOOR: f32 = 1.0e-20;

/// The Altec 9069B's 11 stepped high-pass frequencies (Hz), log-spaced
/// ~70 Hz–7.5 kHz — the detents King Tubby swept.
pub const BIG_KNOB_STEPS: [f32; 11] = [
    70.0, 110.0, 160.0, 240.0, 360.0, 540.0, 800.0, 1_200.0, 1_800.0, 3_500.0, 7_500.0,
];

/// Number of stepped positions.
pub const BIG_KNOB_STEP_COUNT: usize = BIG_KNOB_STEPS.len();

/// Resolution of the macro→cutoff-coefficient table (precomputed so the
/// Advanced super-knob resolves without `tan`/`powf` on the audio thread).
const MACRO_LUT_LEN: usize = 65;

/// King Tubby's bass-drop high-pass filter.
#[derive(Debug)]
pub struct BigKnobHpf {
    sample_rate: f32,
    /// Smoothed prewarped cutoff coefficient and its target.
    g: f32,
    g_target: f32,
    g_smooth: f32,
    /// SVF damping = 1 / Q.
    k: f32,
    /// Integrator state, per channel.
    ic1: [f32; 2],
    ic2: [f32; 2],
    /// Precomputed prewarped cutoff `g` across the macro range [0,1] — so
    /// `set_macro` is a pure table lookup, RT-safe to call from the engine.
    macro_g: [f32; MACRO_LUT_LEN],
}

impl BigKnobHpf {
    /// Allocate a Big Knob filter for `sample_rate`. Defaults near-bypass
    /// (lowest cutoff) at Butterworth Q.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut macro_g = [0.0f32; MACRO_LUT_LEN];
        for (i, slot) in macro_g.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let m = i as f32 / (MACRO_LUT_LEN - 1) as f32;
            let hz = (20.0 * 400.0_f32.powf(m)).clamp(10.0, sample_rate * 0.45);
            *slot = (std::f32::consts::PI * hz / sample_rate).tan();
        }
        let mut f = Self {
            sample_rate,
            g: 0.0,
            g_target: 0.0,
            g_smooth: one_pole_coeff(40.0, sample_rate),
            k: std::f32::consts::SQRT_2, // Q = 0.707
            ic1: [0.0; 2],
            ic2: [0.0; 2],
            macro_g,
        };
        f.set_cutoff_hz(BIG_KNOB_STEPS[0]);
        f.g = f.g_target; // snap, don't ramp from 0 at startup
        f
    }

    /// Set the cutoff in Hz (continuous). Resolved to the prewarped coefficient
    /// off-RT; `process_block` ramps to it click-free.
    pub fn set_cutoff_hz(&mut self, hz: f32) {
        let fc = hz.clamp(10.0, self.sample_rate * 0.45);
        self.g_target = (std::f32::consts::PI * fc / self.sample_rate).tan();
    }

    /// Set the cutoff from a normalised 0..1 macro position (log-mapped
    /// ~20 Hz–8 kHz) — the surface the Advanced super-knob / Expert sweep drive.
    pub fn set_cutoff_norm(&mut self, norm: f32) {
        let n = norm.clamp(0.0, 1.0);
        let hz = 20.0 * 400.0_f32.powf(n); // 20 * (8000/20)^n
        self.set_cutoff_hz(hz);
    }

    /// Snap to one of the Altec's [`BIG_KNOB_STEPS`] detents (clamped).
    pub fn set_step(&mut self, step: usize) {
        let i = step.min(BIG_KNOB_STEP_COUNT - 1);
        self.set_cutoff_hz(BIG_KNOB_STEPS[i]);
    }

    /// Resonance (Q): 0.5 (gentle, passive-like) up to ~8 (sharp). Off-RT.
    pub fn set_resonance(&mut self, q: f32) {
        self.k = 1.0 / q.clamp(0.5, 8.0);
    }

    /// Advanced **super-knob**: one 0..1 macro performs the whole bass-drop —
    /// sweeps the cutoff up (log) and leans in a touch of resonance for the
    /// whistle as it climbs. 0 = open/near-bypass, 1 = lows fully gone.
    /// RT-safe: the cutoff comes from the precomputed `macro_g` table, so the
    /// engine can call this from the audio thread (no `tan`/`powf`).
    pub fn set_macro(&mut self, m: f32) {
        let m = m.clamp(0.0, 1.0);
        #[allow(clippy::cast_precision_loss)]
        let fi = m * (MACRO_LUT_LEN - 1) as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i = fi as usize;
        #[allow(clippy::cast_precision_loss)]
        let frac = fi - i as f32;
        self.g_target = if i + 1 < MACRO_LUT_LEN {
            self.macro_g[i] * (1.0 - frac) + self.macro_g[i + 1] * frac
        } else {
            self.macro_g[MACRO_LUT_LEN - 1]
        };
        self.set_resonance(0.707 + m * 1.3);
    }

    /// Clear filter state.
    pub fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
        self.g = self.g_target;
    }

    /// High-pass the stereo block **in place**. RT-safe: one divide per frame,
    /// otherwise adds/mults; coefficient ramps toward target (no clicks).
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);
        for frame in out.chunks_exact_mut(stride) {
            self.g = flush(self.g + self.g_smooth * (self.g_target - self.g));
            let a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
            let a2 = self.g * a1;
            let a3 = self.g * a2;
            for ch in 0..2 {
                let x = frame[offset + ch];
                let v3 = x - self.ic2[ch];
                let v1 = a1 * self.ic1[ch] + a2 * v3;
                let v2 = self.ic2[ch] + a2 * self.ic1[ch] + a3 * v3;
                self.ic1[ch] = flush(2.0 * v1 - self.ic1[ch]);
                self.ic2[ch] = flush(2.0 * v2 - self.ic2[ch]);
                frame[offset + ch] = x - self.k * v1 - v2; // high-pass output
            }
        }
    }
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn rms(a: &[f32]) -> f32 {
        if a.is_empty() {
            return 0.0;
        }
        (a.iter().map(|&x| x * x).sum::<f32>() / a.len() as f32).sqrt()
    }

    /// Run a steady sine at `freq` through the filter; return steady-state RMS.
    fn tone_rms(f: &mut BigKnobHpf, freq: f32, n: usize) -> f32 {
        let mut out = Vec::with_capacity(n);
        let step = std::f32::consts::TAU * freq / SR;
        for i in 0..n {
            let s = (step * i as f32).sin();
            let mut buf = [s, s];
            f.process_block(&mut buf, 2, 0);
            out.push(buf[0]);
        }
        // Skip the settling transient.
        rms(&out[n / 2..])
    }

    #[test]
    fn passes_highs_blocks_lows() {
        let mut f = BigKnobHpf::new(SR);
        f.set_cutoff_hz(1_000.0);
        let low = tone_rms(&mut f, 60.0, 8_000);
        let mut f2 = BigKnobHpf::new(SR);
        f2.set_cutoff_hz(1_000.0);
        let high = tone_rms(&mut f2, 6_000.0, 8_000);
        // A 60 Hz tone (well below 1 kHz cutoff) is crushed; 6 kHz passes.
        assert!(low < 0.1, "low end not attenuated: {low}");
        assert!(high > 0.6, "highs not passed: {high}");
    }

    #[test]
    fn near_bypass_at_lowest_cutoff() {
        let mut f = BigKnobHpf::new(SR);
        f.set_step(0); // ~70 Hz
        let mid = tone_rms(&mut f, 1_000.0, 8_000);
        // Everything above ~70 Hz should pass essentially untouched (~0.707 RMS
        // for a unit sine).
        assert!(mid > 0.66, "mid attenuated at lowest cutoff: {mid}");
    }

    #[test]
    fn higher_step_removes_more() {
        let probe = |step: usize| {
            let mut f = BigKnobHpf::new(SR);
            f.set_step(step);
            tone_rms(&mut f, 500.0, 8_000)
        };
        // A 500 Hz tone survives a low step, is gutted by a high one.
        assert!(probe(2) > probe(8), "higher step did not attenuate more");
    }

    #[test]
    fn channels_are_independent() {
        let mut f = BigKnobHpf::new(SR);
        f.set_cutoff_hz(500.0);
        let step = std::f32::consts::TAU * 2_000.0 / SR;
        let mut r_energy = 0.0f32;
        for i in 0..4_000 {
            let s = (step * i as f32).sin();
            let mut buf = [s, 0.0]; // signal left, silence right
            f.process_block(&mut buf, 2, 0);
            r_energy += buf[1].abs();
        }
        assert!(r_energy < 1e-3, "silent right channel leaked: {r_energy}");
    }

    #[test]
    fn cutoff_sweep_is_click_free() {
        let mut f = BigKnobHpf::new(SR);
        f.set_step(0);
        let step = std::f32::consts::TAU * 800.0 / SR;
        let mut prev = 0.0f32;
        let mut max_jump = 0.0f32;
        for i in 0..48_000 {
            // Jam the cutoff to the top partway through — a worst-case jump.
            if i == 12_000 {
                f.set_cutoff_hz(7_500.0);
            }
            let s = (step * i as f32).sin();
            let mut buf = [s, s];
            f.process_block(&mut buf, 2, 0);
            if i > 100 {
                max_jump = max_jump.max((buf[0] - prev).abs());
            }
            prev = buf[0];
        }
        // The smoothed coefficient keeps sample-to-sample motion bounded — no
        // click from the cutoff slam.
        assert!(max_jump < 0.5, "cutoff jump clicked: {max_jump}");
    }

    #[test]
    fn output_stays_finite_with_resonance() {
        let mut f = BigKnobHpf::new(SR);
        f.set_resonance(8.0);
        f.set_cutoff_hz(300.0);
        let step = std::f32::consts::TAU * 300.0 / SR;
        for i in 0..20_000 {
            let s = (step * i as f32).sin();
            let mut buf = [s, s];
            f.process_block(&mut buf, 2, 0);
            assert!(buf[0].is_finite() && buf[1].is_finite());
        }
    }

    #[test]
    fn macro_performs_the_drop() {
        let mut open = BigKnobHpf::new(SR);
        open.set_macro(0.0);
        let bass_open = tone_rms(&mut open, 200.0, 8_000);
        let mut dropped = BigKnobHpf::new(SR);
        dropped.set_macro(1.0);
        let bass_dropped = tone_rms(&mut dropped, 200.0, 8_000);
        assert!(
            bass_open > 0.6,
            "macro 0 should be near-bypass: {bass_open}"
        );
        assert!(
            bass_dropped < 0.15,
            "macro 1 should drop the bass: {bass_dropped}"
        );
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut f = BigKnobHpf::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = ((i as f32) * 0.05).sin();
        }
        assert_no_alloc::assert_no_alloc(|| {
            f.set_cutoff_norm(0.7);
            f.set_resonance(2.0);
            f.set_step(5);
            for _ in 0..256 {
                f.process_block(&mut buf, 4, 2);
            }
        });
    }
}
