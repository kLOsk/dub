//! A model of the **Roland RE-201 Space Echo** — the dub centrepiece. A loop of
//! tape runs past one record head and **three** playback heads at fixed
//! positions, giving three delay taps whose times all scale together with the
//! *Repeat Rate* (tape speed). *Intensity* feeds the playback back onto the tape
//! (self-oscillating when pushed); each pass loses highs and softly saturates,
//! so repeats darken and smear. A **mode** switch selects which heads are live
//! and whether the onboard **spring reverb** ([`crate::spring::SpringReverb`])
//! is in circuit. This is the tape upgrade to the clean digital slap-back of the
//! M15 echo-out.
//!
//! ## What we model
//!
//! - **Three playback heads** at fixed delay ratios, all scaled by Repeat Rate.
//! - **Wow & flutter** — slow + fast pitch drift from tape-speed variation
//!   (fractional, interpolated read taps modulated by two LFOs).
//! - **Tape saturation** — a bounded soft-clip in the feedback path (also what
//!   keeps self-oscillation from running away).
//! - **Head/tape tone** — a low-pass (HF loss) + DC-blocking high-pass in the
//!   feedback path, so each repeat gets darker and thinner.
//! - **Onboard spring reverb** — the RE-201's built-in tank, fed the wet mix.
//!
//! ## Real-time safety
//!
//! The tape ring, sine LUT (wow/flutter) and the reverb's rings are all
//! allocated in [`Re201::new`]. `process_block` and every `set_*` are pure float
//! math over pre-allocated storage — no allocation, locks, syscalls or
//! transcendentals on the audio thread (sines come from the LUT; saturation is a
//! Padé approximation; filter coefficients resolve off-RT). Feedback states are
//! denormal-flushed. Verified under `assert_no_alloc`.

use crate::echo::one_pole_coeff;
use crate::spring::SpringReverb;

const DENORMAL_FLOOR: f32 = 1.0e-20;

/// Length of the wow/flutter sine LUT (power of two for cheap masking).
const SINE_LEN: usize = 1024;

/// Longest playback-head delay the tape ring can hold, in seconds. The Repeat
/// Rate sets head 3 up to this; heads 1 and 2 are shorter fractions of it.
const MAX_DELAY_SECS: f32 = 0.75;

/// Fixed delay ratios of the three playback heads relative to the longest.
const HEAD_RATIOS: [f32; 3] = [0.337, 0.668, 1.0];

/// RE-201 mode: which playback heads are live and whether the spring reverb is
/// in circuit. A representative subset of the unit's 12-position selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Re201Mode {
    /// Reverb only — no tape echo.
    Reverb,
    /// Head 1 (shortest).
    Short,
    /// Heads 2 + 3.
    Long,
    /// All three heads.
    Triple,
    /// Head 1 + reverb.
    ShortReverb,
    /// Heads 2 + 3 + reverb.
    LongReverb,
    /// All three heads + reverb.
    TripleReverb,
}

impl Re201Mode {
    /// `(head1, head2, head3, reverb)` for this mode.
    fn config(self) -> ([bool; 3], bool) {
        match self {
            Re201Mode::Reverb => ([false, false, false], true),
            Re201Mode::Short => ([true, false, false], false),
            Re201Mode::Long => ([false, true, true], false),
            Re201Mode::Triple => ([true, true, true], false),
            Re201Mode::ShortReverb => ([true, false, false], true),
            Re201Mode::LongReverb => ([false, true, true], true),
            Re201Mode::TripleReverb => ([true, true, true], true),
        }
    }
}

/// A Roland RE-201 Space Echo. Insert it on a signal: [`Re201::process_block`]
/// **adds** the wet echo + reverb on top of the dry already in the buffer (the
/// unit's direct path is clean; only the echoes are tape-coloured).
#[derive(Debug)]
pub struct Re201 {
    sample_rate: f32,
    tape: Box<[f32]>,
    mask: usize,
    write: usize,
    max_delay_samples: f32,

    /// Per-head delay in (fractional) samples; index 2 is the longest.
    head_delays: [f32; 3],
    heads_active: [bool; 3],
    /// Index of the longest *active* head — feedback recirculates from this one
    /// tap, so loop gain tracks Intensity regardless of how many heads are live
    /// (the wet output still sums all active heads for the multi-tap texture).
    fb_head_idx: usize,

    intensity: f32,
    echo_volume: f32,

    // Tape tone in the feedback path.
    fb_lp: f32,
    fb_lp_coeff: f32,
    fb_dc: f32,
    fb_hp_coeff: f32,

    // Wow & flutter.
    sine: Box<[f32]>,
    wow_phase: f64,
    flutter_phase: f64,
    wow_inc: f64,
    flutter_inc: f64,
    wow_depth: f32,
    flutter_depth: f32,

    // Onboard spring reverb.
    reverb: SpringReverb,
    reverb_on: bool,
}

impl Re201 {
    /// Allocate an RE-201 for `sample_rate`. **Not RT-safe** (allocates the tape
    /// ring, sine LUT and the reverb rings); call at setup.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max_samples = (sample_rate * MAX_DELAY_SECS * 1.05) as usize;
        let cap = max_samples.next_power_of_two().max(4);

        let mut sine = vec![0.0f32; SINE_LEN].into_boxed_slice();
        for (i, s) in sine.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let phase = i as f32 / SINE_LEN as f32;
            *s = (phase * std::f32::consts::TAU).sin();
        }

        let mut re = Self {
            sample_rate,
            tape: vec![0.0; cap].into_boxed_slice(),
            mask: cap - 1,
            write: 0,
            max_delay_samples: (cap - 4) as f32,
            head_delays: [0.0; 3],
            heads_active: [true, true, true],
            fb_head_idx: 2,
            intensity: 0.5,
            echo_volume: 0.7,
            fb_lp: 0.0,
            fb_lp_coeff: one_pole_coeff(3_000.0, sample_rate),
            fb_dc: 0.0,
            fb_hp_coeff: one_pole_coeff(120.0, sample_rate),
            sine,
            wow_phase: 0.0,
            flutter_phase: 0.0,
            wow_inc: f64::from(0.6 * SINE_LEN as f32 / sample_rate),
            flutter_inc: f64::from(6.3 * SINE_LEN as f32 / sample_rate),
            wow_depth: 0.0,
            flutter_depth: 0.0,
            reverb: SpringReverb::new(sample_rate),
            reverb_on: false,
        };
        re.set_repeat_rate(300.0);
        re.set_reverb(0.0);
        re.set_mode(Re201Mode::TripleReverb);
        re
    }

    /// Select which heads/reverb are in circuit.
    pub fn set_mode(&mut self, mode: Re201Mode) {
        let (heads, reverb) = mode.config();
        self.heads_active = heads;
        // Longest active head drives the feedback (0 if none — reverb-only,
        // where echo is silent and feedback never engages anyway).
        self.fb_head_idx = heads
            .iter()
            .enumerate()
            .filter(|(_, &a)| a)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0);
        self.reverb_on = reverb;
    }

    /// Repeat Rate: set the longest head's delay in ms (heads 1 & 2 follow at
    /// fixed ratios). Resolved to fractional samples off-RT.
    pub fn set_repeat_rate(&mut self, longest_delay_ms: f32) {
        let longest =
            (longest_delay_ms / 1000.0 * self.sample_rate).clamp(2.0, self.max_delay_samples);
        for (d, &r) in self.head_delays.iter_mut().zip(HEAD_RATIOS.iter()) {
            *d = (longest * r).max(1.0);
        }
    }

    /// Intensity (feedback): 0 = single repeat, ~1.1 = self-oscillation. The
    /// in-loop soft-clip keeps it bounded.
    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.clamp(0.0, 1.2);
    }

    /// Echo Volume: wet level of the tape repeats (0..1).
    pub fn set_echo_volume(&mut self, vol: f32) {
        self.echo_volume = vol.clamp(0.0, 1.0);
    }

    /// Tape age: scales wow + flutter depth (0 = pristine, 1 = worn).
    pub fn set_wow_flutter(&mut self, amount: f32) {
        let a = amount.clamp(0.0, 2.0);
        self.wow_depth = 0.004 * a;
        self.flutter_depth = 0.0018 * a;
    }

    /// Onboard spring reverb wet mix (0..1). Only audible in a reverb mode.
    pub fn set_reverb(&mut self, mix: f32) {
        self.reverb
            .set_params(0.82, 2_800.0, mix.clamp(0.0, 1.0), self.sample_rate);
    }

    /// Set only the onboard reverb's wet level (0..1). RT-safe: the reverb's
    /// decay/damping are fixed at construction so no coefficient is recomputed.
    pub fn set_reverb_wet(&mut self, mix: f32) {
        self.reverb.set_wet(mix);
    }

    /// Advanced **super-knob**: one 0..1 macro = "dub intensity". Turns up
    /// feedback (toward self-oscillation), wet level, tape wow/flutter and the
    /// onboard reverb together — 0 = a clean short slap, 1 = a self-oscillating
    /// wash. Leaves Repeat Rate and Mode under separate control. RT-safe: every
    /// setter it calls is pure assignment (reverb wet only, no coefficient
    /// recompute), so the engine can call this from the audio thread.
    pub fn set_macro(&mut self, m: f32) {
        let m = m.clamp(0.0, 1.0);
        self.set_intensity(0.2 + m * 0.95);
        self.set_echo_volume(0.4 + m * 0.5);
        self.set_wow_flutter(m);
        self.set_reverb_wet(m * 0.6);
    }

    /// Clear the tape and all filter/LFO state. Bounded but touches the whole
    /// ring; call off-RT.
    pub fn reset(&mut self) {
        self.tape.fill(0.0);
        self.write = 0;
        self.fb_lp = 0.0;
        self.fb_dc = 0.0;
        self.wow_phase = 0.0;
        self.flutter_phase = 0.0;
    }

    #[inline]
    fn lut(&self, phase: f64) -> f32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let idx = (phase as usize) & (SINE_LEN - 1);
        self.sine[idx]
    }

    /// Read the tape `delay` fractional samples behind the write head (linear
    /// interpolation).
    #[inline]
    fn read_frac(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, self.max_delay_samples);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i = d as usize;
        #[allow(clippy::cast_precision_loss)]
        let frac = d - i as f32;
        let a = self.tape[self.write.wrapping_sub(i) & self.mask];
        let b = self.tape[self.write.wrapping_sub(i + 1) & self.mask];
        a * (1.0 - frac) + b * frac
    }

    /// Process one stereo block in place, **adding** the wet echo + reverb:
    /// `out[f·stride + offset ..][..2] += echo + reverb`. RT-safe: bounded loop,
    /// indexed loads/stores, no allocation.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);
        for frame in out.chunks_exact_mut(stride) {
            let dry = 0.5 * (frame[offset] + frame[offset + 1]);

            // Wow + flutter modulate the read position (→ pitch wobble).
            let wow = self.lut(self.wow_phase);
            let flutter = self.lut(self.flutter_phase);
            let modulation = 1.0 + wow * self.wow_depth + flutter * self.flutter_depth;
            self.wow_phase += self.wow_inc;
            if self.wow_phase >= SINE_LEN as f64 {
                self.wow_phase -= SINE_LEN as f64;
            }
            self.flutter_phase += self.flutter_inc;
            if self.flutter_phase >= SINE_LEN as f64 {
                self.flutter_phase -= SINE_LEN as f64;
            }

            // Sum the active playback heads for the wet output; capture the
            // longest active head separately as the single feedback source.
            let mut echo = 0.0;
            let mut fb_in = 0.0;
            for (i, (d, &active)) in self
                .head_delays
                .iter()
                .zip(self.heads_active.iter())
                .enumerate()
            {
                if active {
                    let r = self.read_frac(d * modulation);
                    echo += r;
                    if i == self.fb_head_idx {
                        fb_in = r;
                    }
                }
            }

            // Feedback path: HF loss, low-end thinning, tape saturation.
            self.fb_lp = flush(self.fb_lp + self.fb_lp_coeff * (fb_in - self.fb_lp));
            self.fb_dc = flush(self.fb_dc + self.fb_hp_coeff * (self.fb_lp - self.fb_dc));
            let coloured = soft_clip(self.fb_lp - self.fb_dc);
            self.tape[self.write] = flush(dry + coloured * self.intensity);
            self.write = (self.write + 1) & self.mask;

            let wet = echo * self.echo_volume;
            frame[offset] += wet;
            frame[offset + 1] += wet;
        }

        if self.reverb_on {
            self.reverb.process_block(out, stride, offset);
        }
    }
}

/// Bounded soft-clip (Padé `tanh` approximation) — tape compression that also
/// caps self-oscillation. No transcendental; `|x|≥3 → ±1`.
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

    /// Feed a single-sample impulse, render `n` frames, return the mono output.
    fn impulse(re: &mut Re201, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let mut buf = [x, x];
            re.process_block(&mut buf, 2, 0);
            out.push(buf[0]);
        }
        out
    }

    #[test]
    fn produces_delayed_repeats() {
        let mut re = Re201::new(SR);
        re.set_mode(Re201Mode::Triple);
        re.set_repeat_rate(120.0);
        re.set_intensity(0.5);
        re.set_echo_volume(1.0);
        let out = impulse(&mut re, SR as usize);
        // Beyond the dry impulse there must be at least one strong echo tap.
        let peak = out[200..].iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.1, "no audible echo repeat: {peak}");
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn low_intensity_decays_to_silence() {
        let mut re = Re201::new(SR);
        re.set_mode(Re201Mode::Triple);
        re.set_repeat_rate(120.0);
        re.set_intensity(0.3);
        re.set_echo_volume(1.0);
        let out = impulse(&mut re, SR as usize * 3);
        let end = rms(&out[(SR as usize * 2)..]);
        assert!(end < 1e-3, "echo never decayed: {end}");
    }

    #[test]
    fn high_intensity_self_oscillates_but_stays_bounded() {
        let mut re = Re201::new(SR);
        re.set_mode(Re201Mode::Triple);
        re.set_repeat_rate(120.0);
        re.set_intensity(1.15);
        re.set_echo_volume(1.0);
        let out = impulse(&mut re, SR as usize * 4);
        // Sustains (does not die) ...
        let late = rms(&out[(SR as usize * 3)..]);
        assert!(late > 1e-3, "self-oscillation died: {late}");
        // ... but the in-loop soft-clip keeps it bounded and finite.
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 4.0));
    }

    #[test]
    fn repeat_rate_sets_first_echo_time() {
        let first_echo = |ms: f32| {
            let mut re = Re201::new(SR);
            re.set_mode(Re201Mode::Short);
            re.set_repeat_rate(ms);
            re.set_intensity(0.4);
            re.set_echo_volume(1.0);
            let out = impulse(&mut re, SR as usize);
            out[100..]
                .iter()
                .position(|&x| x.abs() > 0.1)
                .map(|p| p + 100)
                .unwrap_or(usize::MAX)
        };
        let fast = first_echo(80.0);
        let slow = first_echo(300.0);
        assert!(
            fast < slow,
            "slower repeat rate must delay the echo: {fast} vs {slow}"
        );
    }

    #[test]
    fn reverb_only_mode_has_no_discrete_echo_but_a_tail() {
        let mut re = Re201::new(SR);
        re.set_mode(Re201Mode::Reverb);
        re.set_reverb(1.0);
        re.set_echo_volume(1.0);
        let out = impulse(&mut re, SR as usize);
        // A diffuse reverb tail exists ...
        let tail = rms(&out[2_000..20_000]);
        assert!(tail > 1e-4, "no reverb tail: {tail}");
        // ... and it decays.
        let late = rms(&out[(SR as usize - 4_000)..]);
        assert!(late < tail, "reverb did not decay");
    }

    #[test]
    fn dry_is_preserved_first_sample() {
        let mut re = Re201::new(SR);
        re.set_mode(Re201Mode::Triple);
        re.set_echo_volume(1.0);
        // Tape empty → echo ≈ 0 → dry passes through untouched.
        let mut buf = [0.6f32, 0.6];
        re.process_block(&mut buf, 2, 0);
        assert!((buf[0] - 0.6).abs() < 1e-4, "dry not preserved: {}", buf[0]);
    }

    #[test]
    fn wow_flutter_stays_finite() {
        let mut re = Re201::new(SR);
        re.set_mode(Re201Mode::Triple);
        re.set_repeat_rate(150.0);
        re.set_intensity(0.6);
        re.set_echo_volume(1.0);
        re.set_wow_flutter(1.5);
        let out = impulse(&mut re, SR as usize * 2);
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn macro_increases_dub_intensity() {
        let tail = |m: f32| {
            let mut re = Re201::new(SR);
            re.set_mode(Re201Mode::Triple);
            re.set_repeat_rate(150.0);
            re.set_macro(m);
            let out = impulse(&mut re, SR as usize * 2);
            rms(&out[(SR as usize)..])
        };
        assert!(tail(0.9) > tail(0.2), "macro did not increase sustain");
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut re = Re201::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = ((i as f32) * 0.02).sin();
        }
        assert_no_alloc::assert_no_alloc(|| {
            re.set_mode(Re201Mode::TripleReverb);
            re.set_repeat_rate(220.0);
            re.set_intensity(0.7);
            re.set_echo_volume(0.8);
            re.set_wow_flutter(0.5);
            re.set_reverb(0.4);
            for _ in 0..256 {
                re.process_block(&mut buf, 4, 2);
            }
        });
    }

    /// Render a few RE-201 textures to scratch WAVs for offline listening.
    /// Ignored by default; run with `-- --ignored`.
    #[test]
    #[ignore]
    fn dump_wavs() {
        let dir = "/private/tmp/claude-501/-Users-klos-Development-dub/05fb1678-ea88-4fb6-bb52-d61e8c879ac7/scratchpad";
        let cases: [(&str, Re201Mode, f32, f32); 4] = [
            ("re201_short", Re201Mode::Short, 150.0, 0.45),
            ("re201_triple", Re201Mode::Triple, 220.0, 0.6),
            ("re201_selfosc", Re201Mode::Triple, 180.0, 1.12),
            ("re201_dub", Re201Mode::TripleReverb, 260.0, 0.7),
        ];
        for (name, mode, rate, intensity) in cases {
            let mut re = Re201::new(SR);
            re.set_mode(mode);
            re.set_repeat_rate(rate);
            re.set_intensity(intensity);
            re.set_echo_volume(0.8);
            re.set_wow_flutter(0.6);
            re.set_reverb(0.5);
            let samples = impulse(&mut re, (SR * 4.0) as usize);
            write_wav(&format!("{dir}/{name}.wav"), &samples, SR as u32);
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
}
