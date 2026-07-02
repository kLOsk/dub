//! A model of the **Princeton PT2399** — the digital echo IC at the heart of
//! nearly every modern dub-siren box (the Rigsmith GS1, the Benidub DS-1/DS-1e)
//! and countless dub delay pedals. It's the *siren's onboard echo*, not a rack
//! effect: a self-contained instrument's delay section (see the siren signal
//! architecture). Distinct from the [`crate::re201`] tape model — the PT2399 is
//! darker, grittier and dirtier, and it *dives in pitch* when you sweep the
//! delay knob (the classic dub move).
//!
//! ## What we model
//!
//! - **Variable delay**, smoothed toward its target so turning the Delay knob
//!   bends the tail's pitch (the chip changes its internal clock; we move a
//!   fractional read pointer, which sounds the same).
//! - **Degrading feedback** — each repeat loses highs (a fixed dub-voiced
//!   low-pass), gets a touch dirtier (soft-clip, which also bounds dub
//!   self-oscillation), and picks up the chip's faint companding hiss.
//! - **Delay-dependent lo-fi grit** — the real chip's internal sample rate
//!   drops as the delay lengthens, so longer settings alias and grain up; we
//!   model that with a sample-and-hold whose hold grows with the delay.
//!
//! It is mono (the PT2399 is a mono chip): [`Pt2399::process_block`] mono-sums
//! the input, echoes it, and writes dry + wet to both channels in place.
//!
//! ## Real-time safety
//!
//! The delay ring is allocated in [`Pt2399::new`]; every setter is pure
//! assignment (delay in ms → samples is a multiply; the feedback tone
//! coefficients are fixed at construction), so the engine can drive Delay /
//! Feedback / Mix straight from the audio thread — no allocation, lock or
//! transcendental in the loop. Feedback state is denormal-flushed. Verified
//! under `assert_no_alloc`.

use crate::echo::one_pole_coeff;

const DENORMAL_FLOOR: f32 = 1.0e-20;

/// Longest delay the ring holds (s). The real chip degrades hard past ~340 ms;
/// dub boxes push further, so we allow headroom.
const MAX_DELAY_SECS: f32 = 0.8;

/// Resolution of the FILTER knob → cutoff-coefficient table (precomputed so the
/// resonant HP-LP resolves without `tan`/`powf` on the audio thread).
const FILTER_LUT_LEN: usize = 65;

/// A PT2399 dub-echo voice.
#[derive(Debug)]
pub struct Pt2399 {
    sample_rate: f32,
    buf: Box<[f32]>,
    mask: usize,
    write: usize,
    max_delay_samples: f32,

    /// Smoothed delay and its target, in (fractional) samples.
    delay: f32,
    delay_target: f32,
    delay_smooth: f32,

    feedback: f32,
    mix: f32,
    /// ECHO CUT: while true the wet output is muted, but the echo loop keeps
    /// running underneath (rhythmic echo chopping). Pure flag, RT-safe.
    cut: bool,

    // DS01E **FILTER**: a resonant HP-LP shaping the echo tone, applied to the
    // wet read so it colours both the heard echo and the feedback (so repeats
    // progressively filter — the dub move). One knob: 0 = dark low-pass →
    // ~0.5 open → 1 = thin high-pass. Smoothed g for click-free sweeps.
    filter_g: f32,
    filter_g_target: f32,
    filter_g_smooth: f32,
    filter_k: f32,
    filter_is_hp: bool,
    filter_ic1: f32,
    filter_ic2: f32,
    filter_lut: [f32; FILTER_LUT_LEN],

    // Light DC blocker on the feedback loop (always on; stops DC build-up).
    fb_dc: f32,
    fb_hp_coeff: f32,

    // Delay-dependent lo-fi grit (sample-and-hold on the wet read).
    sh_held: f32,
    sh_counter: u32,

    noise: u32,
}

impl Pt2399 {
    /// Allocate a PT2399 echo for `sample_rate`. **Not RT-safe** (allocates the
    /// ring); call at setup.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_samples = (sample_rate * MAX_DELAY_SECS) as usize;
        let cap = max_samples.next_power_of_two().max(4);

        // FILTER knob → prewarped cutoff `g`. Below 0.5 = low-pass (200 Hz→16 kHz,
        // log); above 0.5 = high-pass (20 Hz→4 kHz, log). Both ≈ open at 0.5, so
        // the LP↔HP switch is seamless.
        let mut filter_lut = [0.0f32; FILTER_LUT_LEN];
        for (i, slot) in filter_lut.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let knob = i as f32 / (FILTER_LUT_LEN - 1) as f32;
            let fc = if knob < 0.5 {
                200.0 * 80.0_f32.powf(knob * 2.0)
            } else {
                20.0 * 200.0_f32.powf((knob - 0.5) * 2.0)
            };
            let fc = fc.clamp(10.0, sample_rate * 0.45);
            *slot = (std::f32::consts::PI * fc / sample_rate).tan();
        }

        let mut p = Self {
            sample_rate,
            buf: vec![0.0; cap].into_boxed_slice(),
            mask: cap - 1,
            write: 0,
            max_delay_samples: (cap - 4) as f32,
            delay: 0.0,
            delay_target: 0.0,
            delay_smooth: one_pole_coeff(18.0, sample_rate), // ~9 ms glide → pitch dive
            feedback: 0.4,
            mix: 0.5,
            cut: false,
            filter_g: 0.0,
            filter_g_target: 0.0,
            filter_g_smooth: one_pole_coeff(40.0, sample_rate), // click-free filter sweep
            filter_k: 0.9,                                      // Q ≈ 1.1, gentle resonance
            filter_is_hp: false,
            filter_ic1: 0.0,
            filter_ic2: 0.0,
            filter_lut,
            fb_dc: 0.0,
            fb_hp_coeff: one_pole_coeff(90.0, sample_rate),
            sh_held: 0.0,
            sh_counter: 0,
            noise: 0x2545_f491,
        };
        p.set_delay_ms(300.0);
        p.delay = p.delay_target; // snap at startup, don't dive from 0
        p.set_filter(0.3); // dub-dark low-pass default (~matches the old fixed tone)
        p.filter_g = p.filter_g_target; // snap, don't sweep from 0
        p
    }

    /// Delay time in ms. Smoothed toward the target, so sweeping it bends the
    /// tail's pitch. Pure (ms → samples is a multiply); RT-safe.
    pub fn set_delay_ms(&mut self, ms: f32) {
        let samples = (ms / 1000.0 * self.sample_rate).clamp(1.0, self.max_delay_samples);
        self.delay_target = samples;
    }

    /// Feedback / regeneration: 0 = one repeat, ~1.05 = dub self-oscillation
    /// (the soft-clip keeps it bounded). RT-safe.
    pub fn set_feedback(&mut self, fb: f32) {
        self.feedback = fb.clamp(0.0, 1.1);
    }

    /// Echo wet mix added on top of the dry (0..1). RT-safe.
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    /// The DS01E **FILTER** knob (0..1): one resonant filter shaping the echo
    /// tone — `0` = dark low-pass, `~0.5` = open, `1` = thin high-pass. RT-safe
    /// (cutoff comes from the precomputed `filter_lut`; smoothed for click-free
    /// sweeps).
    pub fn set_filter(&mut self, knob: f32) {
        let k = knob.clamp(0.0, 1.0);
        #[allow(clippy::cast_precision_loss)]
        let fi = k * (FILTER_LUT_LEN - 1) as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i = fi as usize;
        #[allow(clippy::cast_precision_loss)]
        let frac = fi - i as f32;
        self.filter_g_target = if i + 1 < FILTER_LUT_LEN {
            self.filter_lut[i] * (1.0 - frac) + self.filter_lut[i + 1] * frac
        } else {
            self.filter_lut[FILTER_LUT_LEN - 1]
        };
        self.filter_is_hp = k >= 0.5;
    }

    /// **ECHO CUT**: while `true`, mute the wet output for rhythmic chopping —
    /// the echo loop keeps recirculating underneath, so releasing brings the
    /// (evolved) echo straight back. Pure flag, RT-safe.
    pub fn set_echo_cut(&mut self, cut: bool) {
        self.cut = cut;
    }

    /// Clear the delay line and filter state.
    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
        self.fb_dc = 0.0;
        self.filter_ic1 = 0.0;
        self.filter_ic2 = 0.0;
        self.filter_g = self.filter_g_target;
        self.delay = self.delay_target;
        self.sh_counter = 0;
    }

    #[inline]
    fn read_frac(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, self.max_delay_samples);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i = d as usize;
        #[allow(clippy::cast_precision_loss)]
        let frac = d - i as f32;
        let a = self.buf[self.write.wrapping_sub(i) & self.mask];
        let b = self.buf[self.write.wrapping_sub(i + 1) & self.mask];
        a * (1.0 - frac) + b * frac
    }

    /// How many samples to hold the wet read for (lo-fi grit that worsens with
    /// the delay length, like the chip's falling internal clock).
    #[inline]
    fn grit_hold(&self) -> u32 {
        // ~1 (clean) at short delays, rising toward ~5 at the longest.
        let frac = (self.delay / self.max_delay_samples).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let h = 1 + (frac * 4.0) as u32;
        h
    }

    /// Process one stereo block in place, mono-summing the input and writing
    /// `dry + mix·echo` to both channels. RT-safe.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);
        for frame in out.chunks_exact_mut(stride) {
            let dry = 0.5 * (frame[offset] + frame[offset + 1]);

            self.delay = flush(self.delay + self.delay_smooth * (self.delay_target - self.delay));
            let read = self.read_frac(self.delay);

            // Lo-fi grit: sample-and-hold the wet read, longer hold at longer delay.
            let hold = self.grit_hold();
            if self.sh_counter == 0 {
                self.sh_held = read;
                self.sh_counter = hold;
            }
            self.sh_counter -= 1;
            let wet_raw = self.sh_held;

            // DS01E FILTER: a resonant HP-LP (TPT SVF) shaping the echo tone,
            // smoothed for click-free knob sweeps. Applied to the wet read, so
            // it colours both the heard echo and the feedback.
            self.filter_g = flush(
                self.filter_g + self.filter_g_smooth * (self.filter_g_target - self.filter_g),
            );
            let g = self.filter_g;
            let k = self.filter_k;
            let a1 = 1.0 / (1.0 + g * (g + k));
            let a2 = g * a1;
            let a3 = g * a2;
            let v3 = wet_raw - self.filter_ic2;
            let v1 = a1 * self.filter_ic1 + a2 * v3;
            let v2 = self.filter_ic2 + a2 * self.filter_ic1 + a3 * v3;
            self.filter_ic1 = flush(2.0 * v1 - self.filter_ic1);
            self.filter_ic2 = flush(2.0 * v2 - self.filter_ic2);
            let filtered = if self.filter_is_hp {
                wet_raw - k * v1 - v2 // high-pass
            } else {
                v2 // low-pass
            };

            // Light DC block on the loop, then soft-clip dirt.
            self.fb_dc = flush(self.fb_dc + self.fb_hp_coeff * (filtered - self.fb_dc));
            let echo = filtered - self.fb_dc;
            let colored = soft_clip(echo);

            // Faint companding hiss.
            self.noise = self
                .noise
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            #[allow(clippy::cast_precision_loss)]
            let hiss = ((self.noise >> 9) as f32 / (1u32 << 23) as f32 - 1.0) * 1.0e-4;

            // The loop keeps running even while cut (so the echo survives the
            // chop); ECHO CUT only mutes the wet output.
            self.buf[self.write] = flush(dry + colored * self.feedback + hiss);
            self.write = (self.write + 1) & self.mask;

            let wet_out = if self.cut { 0.0 } else { echo };
            let s = dry + wet_out * self.mix;
            frame[offset] = s;
            frame[offset + 1] = s;
        }
    }
}

/// Bounded soft-clip (Padé `tanh` approximation) — the chip's overdrive dirt,
/// and what caps dub self-oscillation. No transcendental; `|x|≥3 → ±1`.
#[inline]
fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
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

    /// Feed a single-sample impulse, render `n` frames, return the wet-only tail
    /// (dry subtracted from the first sample).
    fn impulse(p: &mut Pt2399, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let mut buf = [x, x];
            p.process_block(&mut buf, 2, 0);
            out.push(buf[0] - x);
        }
        out
    }

    #[test]
    fn produces_repeating_echoes() {
        let mut p = Pt2399::new(SR);
        p.set_delay_ms(120.0);
        p.set_feedback(0.6);
        p.set_mix(1.0);
        let out = impulse(&mut p, SR as usize);
        // A repeat lands near the delay time (120 ms = 5760 samples) ...
        let d = (SR * 0.120) as usize;
        let around = |c: usize| {
            out[c - 200..c + 200]
                .iter()
                .fold(0.0f32, |m, &x| m.max(x.abs()))
        };
        assert!(around(d) > 0.05, "no first repeat at the delay time");
        // ... and a second, quieter one near 2× the delay.
        assert!(around(2 * d) > 0.01, "no regenerated second repeat");
        assert!(around(2 * d) < around(d), "feedback didn't decay");
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn low_feedback_decays_to_silence() {
        let mut p = Pt2399::new(SR);
        p.set_delay_ms(100.0);
        p.set_feedback(0.3);
        p.set_mix(1.0);
        let out = impulse(&mut p, SR as usize * 3);
        let end = rms(&out[(SR as usize * 2)..]);
        assert!(end < 1e-3, "echo never decayed: {end}");
    }

    #[test]
    fn high_feedback_self_oscillates_bounded() {
        let mut p = Pt2399::new(SR);
        p.set_delay_ms(150.0);
        p.set_feedback(1.08);
        p.set_mix(1.0);
        let out = impulse(&mut p, SR as usize * 4);
        let late = rms(&out[(SR as usize * 3)..]);
        assert!(late > 1e-3, "self-oscillation died: {late}");
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 4.0));
    }

    #[test]
    fn repeats_get_darker_than_the_input() {
        // The feedback low-pass should make the tail lose high-frequency energy
        // relative to a bright input burst.
        let mut p = Pt2399::new(SR);
        p.set_delay_ms(100.0);
        p.set_feedback(0.7);
        p.set_mix(1.0);
        // Drive with alternating ±1 (near Nyquist, very bright) for a moment.
        let mut out = Vec::new();
        for i in 0..SR as usize {
            let x = if i < 2_000 {
                if i % 2 == 0 {
                    1.0
                } else {
                    -1.0
                }
            } else {
                0.0
            };
            let mut buf = [x, x];
            p.process_block(&mut buf, 2, 0);
            out.push(buf[0] - x);
        }
        // High-frequency content = energy of the first difference.
        let hf = |s: &[f32]| {
            let d: Vec<f32> = s.windows(2).map(|w| w[1] - w[0]).collect();
            rms(&d) / rms(s).max(1e-9)
        };
        // Later repeats are duller (lower HF ratio) than the early ones.
        let early = hf(&out[6_000..12_000]);
        let late = hf(&out[30_000..40_000]);
        assert!(
            late < early,
            "repeats did not darken: early {early} late {late}"
        );
    }

    #[test]
    fn delay_sweep_stays_finite() {
        // Sweeping the delay (the pitch-dive move) must not blow up.
        let mut p = Pt2399::new(SR);
        p.set_delay_ms(300.0);
        p.set_feedback(0.7);
        p.set_mix(1.0);
        let mut ok = true;
        for i in 0..SR as usize * 2 {
            if i == SR as usize / 2 {
                p.set_delay_ms(60.0); // slam shorter → dive up
            }
            let x = if i % 4000 == 0 { 0.8 } else { 0.0 };
            let mut buf = [x, x];
            p.process_block(&mut buf, 2, 0);
            ok &= buf[0].is_finite();
        }
        assert!(ok, "delay sweep produced non-finite output");
    }

    #[test]
    fn dry_is_preserved_first_sample() {
        let mut p = Pt2399::new(SR);
        p.set_mix(1.0);
        let mut buf = [0.6f32, 0.6];
        p.process_block(&mut buf, 2, 0);
        assert!((buf[0] - 0.6).abs() < 1e-3, "dry not preserved: {}", buf[0]);
    }

    #[test]
    fn filter_high_pass_cuts_the_lows() {
        // A low (120 Hz) tone: the FILTER's high-pass setting strips the lows
        // from the echo, the low-pass setting passes them — so the wet (echo)
        // energy is far lower with the HP setting.
        let wet_energy = |filter: f32| -> f64 {
            let mut p = Pt2399::new(SR);
            p.set_delay_ms(120.0);
            p.set_feedback(0.5);
            p.set_mix(1.0);
            p.set_filter(filter);
            let step = std::f32::consts::TAU * 120.0 / SR;
            let mut e = 0.0f64;
            for i in 0..SR as usize {
                let x = (step * i as f32).sin() * 0.5;
                let mut buf = [x, x];
                p.process_block(&mut buf, 2, 0);
                if i > SR as usize / 2 {
                    let wet = f64::from(buf[0] - x); // out = dry + wet
                    e += wet * wet;
                }
            }
            e
        };
        let lp = wet_energy(0.25); // low-pass passes 120 Hz
        let hp = wet_energy(0.9); // high-pass cuts 120 Hz
        assert!(
            lp > hp * 2.0,
            "HP filter did not cut the lows: lp {lp} hp {hp}"
        );
    }

    #[test]
    fn echo_cut_mutes_wet_then_echo_returns() {
        // ECHO CUT mutes the wet output while the loop keeps running, so when
        // released the (still-recirculating) echo comes straight back.
        let mut p = Pt2399::new(SR);
        p.set_delay_ms(100.0);
        p.set_feedback(0.8);
        p.set_mix(1.0);
        p.set_filter(0.3);
        // Prime the loop with an impulse, let it ring.
        let mut buf = [1.0f32, 1.0];
        p.process_block(&mut buf, 2, 0);
        for _ in 0..SR as usize / 4 {
            let mut b = [0.0f32, 0.0];
            p.process_block(&mut b, 2, 0);
        }
        // Cut: the wet output must go silent.
        p.set_echo_cut(true);
        let mut during = 0.0f32;
        for _ in 0..SR as usize / 4 {
            let mut b = [0.0f32, 0.0];
            p.process_block(&mut b, 2, 0);
            during = during.max(b[0].abs());
        }
        assert!(during < 1e-3, "echo cut did not mute the wet: {during}");
        // Release: the loop kept running, so the echo returns.
        p.set_echo_cut(false);
        let mut after = 0.0f32;
        for _ in 0..SR as usize / 4 {
            let mut b = [0.0f32, 0.0];
            p.process_block(&mut b, 2, 0);
            after = after.max(b[0].abs());
        }
        assert!(
            after > 1e-3,
            "echo did not return after cut released: {after}"
        );
    }

    /// Render a 600 Hz "siren stab" through the PT2399 with a few settings to
    /// scratch WAVs for offline listening. Ignored by default; run with
    /// `cargo test -p dub-dsp pt2399::tests::dump_wavs -- --ignored`.
    #[test]
    #[ignore]
    #[allow(clippy::type_complexity)]
    fn dump_wavs() {
        let dir = "/private/tmp/claude-501/-Users-klos-Development-dub/05fb1678-ea88-4fb6-bb52-d61e8c879ac7/scratchpad";
        // A 180 ms 600 Hz tone burst = a siren stab to feed the echo.
        let burst = |i: usize| -> f32 {
            if i < (SR * 0.18) as usize {
                (std::f32::consts::TAU * 600.0 * i as f32 / SR).sin() * 0.5
            } else {
                0.0
            }
        };
        // (name, delay_ms, feedback, mix, optional (sweep_at_frame, sweep_to_ms))
        let cases: [(&str, f32, f32, f32, Option<(usize, f32)>); 4] = [
            ("pt2399_slap", 90.0, 0.35, 0.6, None),
            ("pt2399_dub", 280.0, 0.78, 0.7, None),
            ("pt2399_selfosc", 180.0, 1.05, 0.8, None),
            (
                "pt2399_dive",
                350.0,
                0.75,
                0.7,
                Some(((SR * 0.4) as usize, 70.0)),
            ),
        ];
        for (name, delay, fb, mix, sweep) in cases {
            let mut p = Pt2399::new(SR);
            p.set_delay_ms(delay);
            p.set_feedback(fb);
            p.set_mix(mix);
            let mut samples = Vec::new();
            for i in 0..(SR * 4.0) as usize {
                if let Some((at, to)) = sweep {
                    if i == at {
                        p.set_delay_ms(to);
                    }
                }
                let x = burst(i);
                let mut frame = [x, x];
                p.process_block(&mut frame, 2, 0);
                samples.push(frame[0]);
            }
            write_wav(&format!("{dir}/{name}.wav"), &samples, SR as u32);
        }
    }

    /// Demonstrate the DS01E FILTER (a slow sweep LP→open→HP across the echo
    /// tail) and ECHO CUT (rhythmic chopping of the echo). Ignored; run with
    /// `cargo test -p dub-dsp pt2399::tests::dump_filter_cut_wavs -- --ignored`.
    #[test]
    #[ignore]
    fn dump_filter_cut_wavs() {
        let dir = "/private/tmp/claude-501/-Users-klos-Development-dub/05fb1678-ea88-4fb6-bb52-d61e8c879ac7/scratchpad";
        let stab = |i: usize| -> f32 {
            if i < (SR * 0.18) as usize {
                (std::f32::consts::TAU * 600.0 * i as f32 / SR).sin() * 0.5
            } else {
                0.0
            }
        };
        let total = (SR * 6.0) as usize;

        // FILTER sweep: dark low-pass → open → thin high-pass across the tail.
        {
            let mut p = Pt2399::new(SR);
            p.set_delay_ms(260.0);
            p.set_feedback(0.8);
            p.set_mix(0.7);
            let mut samples = Vec::with_capacity(total);
            for i in 0..total {
                #[allow(clippy::cast_precision_loss)]
                let knob = (0.08 + 0.9 * (i as f32 / total as f32)).min(0.98);
                if i % 64 == 0 {
                    p.set_filter(knob);
                }
                let mut frame = [stab(i), stab(i)];
                p.process_block(&mut frame, 2, 0);
                samples.push(frame[0]);
            }
            write_wav(
                &format!("{dir}/pt2399_filter_sweep.wav"),
                &samples,
                SR as u32,
            );
        }

        // ECHO CUT: after 1 s, chop the echo on/off every ~220 ms.
        {
            let mut p = Pt2399::new(SR);
            p.set_delay_ms(300.0);
            p.set_feedback(0.84);
            p.set_mix(0.8);
            p.set_filter(0.3);
            let chop = (SR * 0.22) as usize;
            let mut samples = Vec::with_capacity(total);
            for i in 0..total {
                if i > SR as usize && i % chop == 0 {
                    p.set_echo_cut((i / chop).is_multiple_of(2));
                }
                let mut frame = [stab(i), stab(i)];
                p.process_block(&mut frame, 2, 0);
                samples.push(frame[0]);
            }
            write_wav(&format!("{dir}/pt2399_echo_cut.wav"), &samples, SR as u32);
        }
    }

    fn write_wav(path: &str, samples: &[f32], rate: u32) {
        use std::io::Write;
        let mut d: Vec<u8> = Vec::new();
        let data_len = (samples.len() * 2) as u32;
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + data_len).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&rate.to_le_bytes());
        d.extend_from_slice(&(rate * 2).to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&data_len.to_le_bytes());
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            d.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::File::create(path).unwrap().write_all(&d).unwrap();
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut p = Pt2399::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = ((i as f32) * 0.02).sin();
        }
        assert_no_alloc::assert_no_alloc(|| {
            p.set_delay_ms(220.0);
            p.set_feedback(0.65);
            p.set_mix(0.6);
            p.set_filter(0.7);
            p.set_echo_cut(true);
            p.set_echo_cut(false);
            for _ in 0..256 {
                p.process_block(&mut buf, 4, 2);
            }
        });
    }
}
