//! A **Mu-Tron Bi-Phase**-style phaser — Lee "Scratch" Perry's Black Ark swirl.
//! A cascade of all-pass stages whose notch frequencies are swept by an LFO;
//! summed with the dry signal, the moving notches give the liquid phasing all
//! over dub vocals and skanks.
//!
//! ## Model
//!
//! Six first-order all-pass stages (the Bi-Phase ran six per side) sharing a
//! sweeping coefficient, with a resonance feedback path for sharper notches and
//! a dry/wet mix. The real Bi-Phase has *two* phasers with independent sweep
//! generators; we capture that as a **stereo** pair whose LFOs are offset, so a
//! mono input comes out wide and swirling.
//!
//! It is an **insert**: [`Phaser::process_block`] rewrites the signal **in
//! place** (dry + phased wet).
//!
//! ## Real-time safety
//!
//! The all-pass coefficient depends on the swept break frequency via `tan`, so
//! we precompute it across the sweep range into a LUT in [`Phaser::set_range`]
//! (off-RT) and the LFO drives a sine LUT; `process_block` only does table reads
//! plus the all-pass recurrence — no transcendental, allocation, lock or syscall
//! on the audio thread. Feedback state is denormal-flushed. Verified under
//! `assert_no_alloc`.

const DENORMAL_FLOOR: f32 = 1.0e-20;

/// Max all-pass stages (Mu-Tron Bi-Phase = 6 per side).
const MAX_STAGES: usize = 6;
/// LFO sine LUT length (power of two).
const SINE_LEN: usize = 1024;
/// All-pass coefficient LUT resolution across the sweep range.
const COEFF_LEN: usize = 512;

/// A Mu-Tron Bi-Phase-style stereo phaser.
#[derive(Debug)]
pub struct Phaser {
    sample_rate: f32,
    stages: usize,

    // Per-channel all-pass state + feedback memory.
    ap_state: [[f32; MAX_STAGES]; 2],
    fb_state: [f32; 2],

    // Swept all-pass coefficient as a function of LFO position.
    coeff_lut: Box<[f32]>,
    fmin: f32,
    fmax: f32,

    // LFO.
    sine: Box<[f32]>,
    lfo_phase: [f64; 2],
    lfo_inc: f64,

    depth: f32,
    feedback: f32,
    mix: f32,
}

impl Phaser {
    /// Allocate a phaser for `sample_rate` with sensible Black-Ark defaults.
    /// **Not RT-safe** (builds the LUTs).
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut sine = vec![0.0f32; SINE_LEN].into_boxed_slice();
        for (i, s) in sine.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let p = i as f32 / SINE_LEN as f32;
            *s = (p * std::f32::consts::TAU).sin();
        }
        let mut p = Self {
            sample_rate,
            stages: MAX_STAGES,
            ap_state: [[0.0; MAX_STAGES]; 2],
            fb_state: [0.0; 2],
            coeff_lut: vec![0.0; COEFF_LEN].into_boxed_slice(),
            fmin: 200.0,
            fmax: 2_000.0,
            sine,
            // Right LFO a quarter-cycle ahead → stereo width.
            lfo_phase: [0.0, SINE_LEN as f64 * 0.25],
            lfo_inc: 0.0,
            depth: 1.0,
            feedback: 0.35,
            mix: 0.5,
        };
        p.set_range(200.0, 2_000.0);
        p.set_rate(0.4);
        p
    }

    /// Sweep range (Hz) of the notches. Rebuilds the coefficient LUT off-RT.
    pub fn set_range(&mut self, fmin: f32, fmax: f32) {
        self.fmin = fmin.clamp(20.0, self.sample_rate * 0.45);
        self.fmax = fmax.clamp(self.fmin + 1.0, self.sample_rate * 0.45);
        let ratio = self.fmax / self.fmin;
        for (i, c) in self.coeff_lut.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / (COEFF_LEN - 1) as f32;
            let fc = self.fmin * ratio.powf(t); // log sweep
            let g = (std::f32::consts::PI * fc / self.sample_rate).tan();
            *c = (g - 1.0) / (g + 1.0); // first-order all-pass coefficient
        }
    }

    /// LFO rate in Hz (~0.05–10).
    pub fn set_rate(&mut self, hz: f32) {
        let r = hz.clamp(0.01, 10.0);
        self.lfo_inc = f64::from(r * SINE_LEN as f32 / self.sample_rate);
    }

    /// Sweep depth 0..1 (how much of the range the LFO traverses).
    pub fn set_depth(&mut self, depth: f32) {
        self.depth = depth.clamp(0.0, 1.0);
    }

    /// Resonance / feedback 0..~0.95 (sharper, more vocal notches).
    pub fn set_feedback(&mut self, fb: f32) {
        self.feedback = fb.clamp(0.0, 0.95);
    }

    /// Dry/wet mix 0..1 (0 = bypass, 0.5 = classic phaser).
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    /// Number of all-pass stages (1..=6); more = more notches, thicker.
    pub fn set_stages(&mut self, stages: usize) {
        self.stages = stages.clamp(1, MAX_STAGES);
    }

    /// Advanced **super-knob**: one 0..1 macro from subtle to gushing — raises
    /// wet mix, resonance feedback, sweep depth and rate together so it always
    /// sounds musical without juggling four controls.
    pub fn set_macro(&mut self, m: f32) {
        let m = m.clamp(0.0, 1.0);
        self.set_mix(0.2 + m * 0.5);
        self.set_feedback(m * 0.85);
        self.set_depth(0.4 + m * 0.6);
        self.set_rate(0.2 + m * 0.6);
    }

    /// Clear all filter/LFO state.
    pub fn reset(&mut self) {
        self.ap_state = [[0.0; MAX_STAGES]; 2];
        self.fb_state = [0.0; 2];
        self.lfo_phase = [0.0, SINE_LEN as f64 * 0.25];
    }

    #[inline]
    fn coeff_at(&self, lfo: f32) -> f32 {
        // lfo in [-1,1] → position in [0,1] scaled by depth (centred).
        let pos = (0.5 + 0.5 * lfo * self.depth).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let idx = (pos * (COEFF_LEN - 1) as f32) as usize;
        self.coeff_lut[idx]
    }

    /// Phase the stereo block **in place**: `out = (1-mix)·dry + mix·phased`.
    /// RT-safe: LUT reads + all-pass recurrence only.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);
        for frame in out.chunks_exact_mut(stride) {
            for ch in 0..2 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let lfo = self.sine[(self.lfo_phase[ch] as usize) & (SINE_LEN - 1)];
                let c = self.coeff_at(lfo);

                let x = frame[offset + ch];
                let mut s = x + self.fb_state[ch] * self.feedback;
                for stage in 0..self.stages {
                    let st = self.ap_state[ch][stage];
                    let y = c * s + st;
                    self.ap_state[ch][stage] = flush(s - c * y);
                    s = y;
                }
                self.fb_state[ch] = flush(s);
                frame[offset + ch] = x * (1.0 - self.mix) + s * self.mix;

                self.lfo_phase[ch] += self.lfo_inc;
                if self.lfo_phase[ch] >= SINE_LEN as f64 {
                    self.lfo_phase[ch] -= SINE_LEN as f64;
                }
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

    /// White-ish noise from a deterministic LCG (no `Math.random`).
    fn noise(n: usize) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                #[allow(clippy::cast_precision_loss)]
                let v = (state >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
                v
            })
            .collect()
    }

    fn run(p: &mut Phaser, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());
        for &x in input {
            let mut buf = [x, x];
            p.process_block(&mut buf, 2, 0);
            out.push(buf[0]);
        }
        out
    }

    #[test]
    fn bypass_at_zero_mix() {
        let mut p = Phaser::new(SR);
        p.set_mix(0.0);
        let input = noise(4_000);
        let out = run(&mut p, &input);
        for (a, b) in input.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-6, "not bypassed at mix 0");
        }
    }

    #[test]
    fn alters_the_signal() {
        let mut p = Phaser::new(SR);
        p.set_mix(0.5);
        p.set_feedback(0.4);
        let input = noise(8_000);
        let out = run(&mut p, &input);
        // The phased output differs from the dry input but stays finite/bounded.
        let diff = rms(&input
            .iter()
            .zip(out.iter())
            .map(|(a, b)| a - b)
            .collect::<Vec<_>>());
        assert!(diff > 1e-3, "phaser had no effect: {diff}");
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn notches_move_over_time() {
        // With the LFO sweeping, the filtering early differs from later.
        let mut p = Phaser::new(SR);
        p.set_rate(2.0);
        p.set_mix(0.5);
        let input = noise(SR as usize);
        let out = run(&mut p, &input);
        let early = rms(&out[1_000..5_000]);
        let mid = rms(&out[20_000..24_000]);
        assert!(
            (early - mid).abs() > 1e-3,
            "no time variation from the LFO: {early} vs {mid}"
        );
    }

    #[test]
    fn stereo_channels_differ() {
        // Offset LFOs → a mono input yields different L and R.
        let mut p = Phaser::new(SR);
        p.set_rate(1.0);
        p.set_mix(0.5);
        let mut diff = 0.0f32;
        let inp = noise(8_000);
        for &x in &inp {
            let mut buf = [x, x];
            p.process_block(&mut buf, 2, 0);
            diff += (buf[0] - buf[1]).abs();
        }
        assert!(diff > 1.0, "channels identical — no stereo width: {diff}");
    }

    #[test]
    fn high_feedback_stays_bounded() {
        let mut p = Phaser::new(SR);
        p.set_feedback(0.95);
        p.set_mix(0.5);
        let input = noise(SR as usize * 2);
        let out = run(&mut p, &input);
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 8.0));
    }

    #[test]
    fn macro_increases_swirl() {
        let deviation = |m: f32| {
            let mut p = Phaser::new(SR);
            p.set_macro(m);
            let input = noise(8_000);
            let out = run(&mut p, &input);
            rms(&input
                .iter()
                .zip(out.iter())
                .map(|(a, b)| a - b)
                .collect::<Vec<_>>())
        };
        assert!(
            deviation(0.9) > deviation(0.1),
            "macro did not deepen the effect"
        );
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut p = Phaser::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = ((i as f32) * 0.03).sin();
        }
        assert_no_alloc::assert_no_alloc(|| {
            p.set_rate(0.6);
            p.set_depth(0.8);
            p.set_feedback(0.5);
            p.set_mix(0.6);
            p.set_stages(4);
            for _ in 0..256 {
                p.process_block(&mut buf, 4, 2);
            }
        });
    }
}
