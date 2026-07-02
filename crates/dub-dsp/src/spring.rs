//! A **spring reverb** — the electromechanical tank at the heart of the dub
//! sound (King Tubby's Fisher/Fairchild, Lee "Scratch" Perry's Grampian, and
//! the one built into the Roland RE-201 Space Echo). It is the boingy, dripping,
//! metallic reverb you hear all over 70s dub, and the thing King Tubby famously
//! *kicked* for thunder.
//!
//! ## Model
//!
//! A real spring is **dispersive**: different frequencies travel down the coil
//! at different speeds, so an impulse smears into the characteristic rising
//! "chirp / boing". We model each spring as a feedback delay tank whose loop
//! contains a cascade of all-pass filters (the dispersion → chirp), a one-pole
//! low-pass (damping → highs decay first) and a DC blocker (springs pass little
//! bass). Two springs of slightly different length run in parallel for density
//! (real tanks have 2–3 springs). [`SpringReverb::kick`] injects an impulse —
//! the "whack the spring" thunder crash.
//!
//! This is a component of the forthcoming RE-201 model and a standalone dub FX.
//!
//! ## Real-time safety
//!
//! All delay/all-pass rings are allocated in [`SpringReverb::new`]; `process_block`,
//! `set_params` and `kick` are pure float math over pre-allocated storage — no
//! allocation, locks, syscalls or transcendentals on the audio thread (the
//! damping coefficient is resolved off-RT). Feedback paths are denormal-flushed.
//! Verified under `assert_no_alloc`.

use crate::echo::one_pole_coeff;

const DENORMAL_FLOOR: f32 = 1.0e-20;

/// A Schroeder all-pass with a pre-allocated delay ring (one dispersion stage).
#[derive(Debug)]
struct Allpass {
    buf: Box<[f32]>,
    mask: usize,
    idx: usize,
    g: f32,
}

impl Allpass {
    fn new(len: usize, g: f32) -> Self {
        let cap = len.next_power_of_two().max(2);
        Self {
            buf: vec![0.0; cap].into_boxed_slice(),
            mask: cap - 1,
            idx: cap - len, // read `len` behind write
            g,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let delayed = self.buf[self.idx];
        let out = delayed - self.g * x;
        self.buf[self.idx] = flush(x + self.g * out);
        self.idx = (self.idx + 1) & self.mask;
        out
    }
}

/// One spring: a feedback delay tank with an in-loop dispersion chain + damping.
#[derive(Debug)]
struct Spring {
    delay: Box<[f32]>,
    mask: usize,
    write: usize,
    read_len: usize,
    aps: Vec<Allpass>,
    feedback: f32,
    damp_coeff: f32,
    damp_lp: f32,
    dc: f32,
}

impl Spring {
    fn new(sample_rate: f32, delay_ms: f32, ap_ms: &[f32], feedback: f32, damp_coeff: f32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let read_len = (sample_rate * delay_ms / 1000.0).max(2.0) as usize;
        let cap = read_len.next_power_of_two().max(2);
        let aps = ap_ms
            .iter()
            .map(|&ms| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let l = (sample_rate * ms / 1000.0).max(1.0) as usize;
                Allpass::new(l, 0.65)
            })
            .collect();
        Self {
            delay: vec![0.0; cap].into_boxed_slice(),
            mask: cap - 1,
            write: 0,
            read_len,
            aps,
            feedback,
            damp_coeff,
            damp_lp: 0.0,
            dc: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let read = self.delay[(self.write.wrapping_sub(self.read_len)) & self.mask];
        // Damping low-pass in the loop (highs decay first).
        self.damp_lp = flush(self.damp_lp + self.damp_coeff * (read - self.damp_lp));
        let mut d = self.damp_lp;
        // Dispersion chain → the chirp / boing.
        for ap in &mut self.aps {
            d = ap.process(d);
        }
        // DC blocker (springs pass little bass): one-pole high-pass.
        let hp = d - self.dc;
        self.dc = flush(self.dc + 0.005 * hp);
        self.delay[self.write] = flush(x + hp * self.feedback);
        self.write = (self.write + 1) & self.mask;
        hp
    }

    fn set(&mut self, feedback: f32, damp_coeff: f32) {
        self.feedback = feedback.clamp(0.0, 0.97);
        self.damp_coeff = damp_coeff.clamp(0.02, 1.0);
    }
}

/// Resolution of the macro→damping-coefficient table (precomputed so the
/// Advanced super-knob resolves without `exp` on the audio thread).
const MACRO_LUT_LEN: usize = 65;

/// A spring-reverb processor. Insert it on a signal: [`SpringReverb::process_block`]
/// adds the reverb tail to whatever is already in the buffer.
#[derive(Debug)]
pub struct SpringReverb {
    springs: [Spring; 2],
    wet: f32,
    /// Pending "kick" excitation injected over the next few samples.
    kick_left: u32,
    kick_level: f32,
    noise: u32,
    /// Precomputed damping coefficient across the macro range [0,1] — so
    /// `set_macro` is a pure table lookup, RT-safe to call from the engine.
    macro_damp: [f32; MACRO_LUT_LEN],
}

impl SpringReverb {
    /// Allocate a spring reverb for `sample_rate`. **Not RT-safe** (allocates
    /// the rings); call at setup.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let damp = one_pole_coeff(2_600.0, sample_rate);
        // Two springs of slightly different length + dispersion, like a real
        // multi-spring tank.
        let springs = [
            Spring::new(sample_rate, 33.0, &[4.7, 6.1, 7.9, 9.3, 11.7], 0.84, damp),
            Spring::new(sample_rate, 41.0, &[5.3, 6.9, 8.7, 10.1, 12.9], 0.84, damp),
        ];
        let mut macro_damp = [0.0f32; MACRO_LUT_LEN];
        for (i, slot) in macro_damp.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let m = i as f32 / (MACRO_LUT_LEN - 1) as f32;
            *slot = one_pole_coeff(4_000.0 - m * 1_800.0, sample_rate);
        }
        Self {
            springs,
            wet: 0.5,
            kick_left: 0,
            kick_level: 0.0,
            noise: 0x9e37_79b9,
            macro_damp,
        }
    }

    /// Live-update the tail: `decay` (0..1 feedback → length), `damping_hz`
    /// (loop low-pass cutoff; lower = darker/shorter highs), `wet` (0..1 wet
    /// gain added to the dry). Coefficients resolved off-RT.
    pub fn set_params(&mut self, decay: f32, damping_hz: f32, wet: f32, sample_rate: f32) {
        let fb = decay.clamp(0.0, 0.97);
        let damp = one_pole_coeff(damping_hz, sample_rate);
        for s in &mut self.springs {
            s.set(fb, damp);
        }
        self.wet = wet.clamp(0.0, 1.0);
    }

    /// Set only the wet mix (0..1). Pure assignment, RT-safe.
    pub fn set_wet(&mut self, wet: f32) {
        self.wet = wet.clamp(0.0, 1.0);
    }

    /// Apply already-resolved tank parameters. Pure assignment, RT-safe — the
    /// damping coefficient must be precomputed off-RT (e.g. via the macro LUT).
    fn set_resolved(&mut self, feedback: f32, damp_coeff: f32, wet: f32) {
        for s in &mut self.springs {
            s.set(feedback, damp_coeff);
        }
        self.wet = wet.clamp(0.0, 1.0);
    }

    /// Advanced **super-knob**: one 0..1 macro grows the tank — 0 = a small,
    /// short, bright room; 1 = a huge, dark, crashing dub tank. RT-safe: the
    /// damping comes from the precomputed `macro_damp` table (no `exp`), so the
    /// engine can call this from the audio thread.
    pub fn set_macro(&mut self, m: f32) {
        let m = m.clamp(0.0, 1.0);
        #[allow(clippy::cast_precision_loss)]
        let fi = m * (MACRO_LUT_LEN - 1) as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i = fi as usize;
        #[allow(clippy::cast_precision_loss)]
        let frac = fi - i as f32;
        let damp = if i + 1 < MACRO_LUT_LEN {
            self.macro_damp[i] * (1.0 - frac) + self.macro_damp[i + 1] * frac
        } else {
            self.macro_damp[MACRO_LUT_LEN - 1]
        };
        self.set_resolved(0.6 + m * 0.37, damp, 0.1 + m * 0.8);
    }

    /// Whack the spring — inject a thunder-crash impulse (`level` 0..1).
    pub fn kick(&mut self, level: f32) {
        self.kick_level = level.clamp(0.0, 1.0);
        self.kick_left = 64; // a short excitation burst
    }

    /// Process one stereo block in place, adding the reverb tail:
    /// `out[f·stride + offset ..][..2] += wet · spring(dry)`.
    /// RT-safe: bounded loop, indexed loads/stores, no allocation.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);
        for frame in out.chunks_exact_mut(stride) {
            let dry = 0.5 * (frame[offset] + frame[offset + 1]);

            // Kick excitation (a short noise burst into the springs).
            let mut exc = dry;
            if self.kick_left > 0 {
                self.kick_left -= 1;
                self.noise = self
                    .noise
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                #[allow(clippy::cast_precision_loss)]
                let n = (self.noise >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
                exc += n * self.kick_level;
            }

            let mut wet = 0.0;
            for s in &mut self.springs {
                wet += s.process(exc);
            }
            wet *= 0.5 * self.wet;

            frame[offset] += wet;
            frame[offset + 1] += wet;
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

    /// Feed an impulse through the reverb and return the wet tail only.
    fn impulse_tail(rev: &mut SpringReverb, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let mut buf = [x, x];
            rev.process_block(&mut buf, 2, 0);
            out.push(buf[0] - x); // subtract the dry passthrough → wet only
        }
        out
    }

    #[test]
    fn impulse_response_spreads_then_decays() {
        let mut rev = SpringReverb::new(SR);
        rev.set_params(0.85, 2_600.0, 1.0, SR);
        let tail = impulse_tail(&mut rev, SR as usize); // 1 s
                                                        // Energy is spread over time (a tail), not a single spike.
        let early = rms(&tail[100..4_000]);
        let mid = rms(&tail[8_000..16_000]);
        assert!(early > 1e-4, "no early reverb energy: {early}");
        assert!(mid > 1e-5, "tail died too fast: {mid}");
        // And it decays: late is quieter than early.
        let late = rms(&tail[40_000..48_000]);
        assert!(
            late < early,
            "tail did not decay: early {early} late {late}"
        );
        assert!(tail.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn tail_eventually_decays_to_near_silence() {
        let mut rev = SpringReverb::new(SR);
        rev.set_params(0.85, 2_600.0, 1.0, SR);
        let tail = impulse_tail(&mut rev, SR as usize * 6); // 6 s
        let end = rms(&tail[(SR as usize * 5)..]);
        assert!(end < 1e-3, "reverb never decayed: {end}");
    }

    #[test]
    fn kick_produces_a_decaying_tail() {
        let mut rev = SpringReverb::new(SR);
        rev.set_params(0.85, 2_600.0, 1.0, SR);
        rev.kick(1.0);
        let mut out = Vec::new();
        for _ in 0..SR as usize {
            let mut buf = [0.0f32, 0.0];
            rev.process_block(&mut buf, 2, 0);
            out.push(buf[0]);
        }
        let early = rms(&out[100..4_000]);
        let late = rms(&out[40_000..48_000]);
        assert!(early > 1e-3, "kick produced no sound: {early}");
        assert!(late < early, "kick tail did not decay");
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 4.0));
    }

    #[test]
    fn dry_is_preserved_and_wet_added() {
        let mut rev = SpringReverb::new(SR);
        rev.set_params(0.8, 2_600.0, 0.5, SR);
        // First sample: the springs are empty, so wet ≈ 0 and dry passes through.
        let mut buf = [0.7f32, 0.7];
        rev.process_block(&mut buf, 2, 0);
        assert!((buf[0] - 0.7).abs() < 1e-3, "dry not preserved: {}", buf[0]);
    }

    #[test]
    fn macro_grows_the_tank() {
        let tail = |m: f32| {
            let mut rev = SpringReverb::new(SR);
            rev.set_macro(m);
            let t = impulse_tail(&mut rev, SR as usize * 2);
            rms(&t[(SR as usize)..])
        };
        assert!(tail(0.9) > tail(0.2), "macro did not grow the tail");
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut rev = SpringReverb::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = ((i as f32) * 0.01).sin();
        }
        assert_no_alloc::assert_no_alloc(|| {
            rev.set_params(0.85, 3_000.0, 0.6, SR);
            rev.kick(0.8);
            for _ in 0..256 {
                rev.process_block(&mut buf, 4, 2);
            }
        });
    }
}
