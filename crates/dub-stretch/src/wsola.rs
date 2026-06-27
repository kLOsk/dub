//! Pure-Rust WSOLA time/pitch stretcher.
//!
//! WSOLA (Waveform-Similarity Overlap-Add, Verhelst & Roelands 1993) is a
//! time-domain stretcher: it splices whole windowed waveform segments at
//! similarity-matched offsets, so percussive transients survive intact — the
//! property that matters most for Dub's scratch / sound-system audience.
//! Time-domain means **no FFT**: every buffer is sized at construction and the
//! per-block step is writes plus a bounded cross-correlation search, so
//! [`process`](WsolaStretcher::process) is allocation-free and needs no
//! `unsafe`.
//!
//! ## Pipeline
//!
//! ```text
//!   input ──▶ [WSOLA time-stretch by S] ──▶ mid ──▶ [linear resample by r] ──▶ output
//! ```
//!
//! WSOLA alone preserves pitch while changing duration. To also shift pitch we
//! resample its output: with `S = time_ratio · pitch_scale` and `r =
//! pitch_scale`, the final stream has length `input · time_ratio` and pitch
//! `× pitch_scale`. When `pitch_scale == 1` the resampler reads at integer
//! positions and is bit-exact (pure time-stretch); when `time_ratio == 1` it
//! is a pure pitch shift (the key-lock case, `pitch_scale = 1 / rate`).
//!
//! All positions/cursors are `f64` (sample-accurate over long tracks); audio
//! stays `f32`. Frame counts are stereo frames; interleaved buffers index
//! `frame * 2 (+ 1)`.

use crate::TimeStretcher;

/// Analysis/synthesis window length as a fraction of the sample rate (~20 ms).
/// Long enough to carry bass periodicity, short enough not to badly smear
/// drum transients. The bench (M14.1) is where this gets tuned against real
/// material; kept as a single constant so that is a one-line change.
const WINDOW_SECS: f32 = 0.020;

/// Cross-correlation search radius as a fraction of the sample rate (~10 ms).
/// The waveform-similarity slack that lets WSOLA dodge phase discontinuities.
const SEARCH_SECS: f32 = 0.010;

/// Coarse stride for the similarity search: scan offsets in `COARSE_STEP` jumps,
/// then refine ±`COARSE_STEP` around the winner. The NCC main lobe is several
/// samples wide, so a coarse pass lands inside the winning basin and the refine
/// recovers the exact peak — cutting the search ~N× with no audible change.
const COARSE_STEP: usize = 4;

/// Ratios are clamped here. Key lock never engages outside roughly ±1 octave;
/// this just keeps the hop math finite and the buffers from being undersized.
const MIN_RATIO: f64 = 0.25;
const MAX_RATIO: f64 = 4.0;

/// Normalized cross-correlation of the candidate frame at `e` (mono-summed from
/// interleaved-stereo `in_buf`) against the precomputed mono prediction `pred`.
/// `inv_norm_t = 1/√Σpred²` is factored out of the loop. Higher = better match.
#[inline]
fn ncc_at(in_buf: &[f32], pred: &[f32], e: usize, inv_norm_t: f64) -> f64 {
    let mut dot = 0.0f64;
    let mut norm_c = 0.0f64;
    for (j, &p) in pred.iter().enumerate() {
        let c = f64::from(in_buf[(e + j) * 2] + in_buf[(e + j) * 2 + 1]);
        dot += c * f64::from(p);
        norm_c += c * c;
    }
    dot * inv_norm_t / (norm_c + 1e-12).sqrt()
}

/// Pure-Rust WSOLA time/pitch stretcher. See the module docs.
pub struct WsolaStretcher {
    // --- fixed config (set in `new`) ---
    /// Window / frame length in frames (even).
    wf: usize,
    /// Synthesis hop in frames (= `wf / 2`; constant 50 % synthesis overlap so
    /// the Hann window is COLA → unity-gain reconstruction).
    hs: usize,
    /// Cross-correlation search radius in frames.
    delta: usize,
    /// Hann window coefficients, length `wf`.
    hann: Box<[f32]>,
    /// Scratch holding the prediction region's mono signal (length `hs`),
    /// precomputed once per search so the inner NCC loop doesn't re-derive it
    /// for every candidate offset.
    pred_scratch: Box<[f32]>,

    // --- live controls ---
    time_ratio: f64,
    pitch_scale: f64,

    // --- input compacting buffer (interleaved stereo, `cap_in * 2`) ---
    in_buf: Box<[f32]>,
    cap_in: usize,
    in_fill: usize,
    /// Nominal analysis read position (frames, relative to `in_buf[0]`).
    analysis_pos: f64,
    /// Frame the previous synthesis frame was actually extracted from
    /// (relative to `in_buf[0]`) — the natural-continuation reference.
    last_extract: f64,
    /// Whether the first (un-searched) frame has been placed.
    primed: bool,

    // --- overlap-add accumulator (interleaved stereo, `wf * 2`) ---
    acc: Box<[f32]>,

    // --- time-stretched mid buffer feeding the resampler (`cap_mid * 2`) ---
    mid: Box<[f32]>,
    cap_mid: usize,
    mid_len: usize,
    /// Fractional resampler read position into `mid` (frames from `mid[0]`).
    rs_pos: f64,
}

impl WsolaStretcher {
    /// Construct a stretcher for `sample_rate` Hz at unity ratio. Allocates all
    /// working buffers up front; never allocates again.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8_000.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mut wf = (sr * WINDOW_SECS).round() as usize;
        wf = wf.max(64) & !1; // even, ≥ 64
        let hs = wf / 2;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let delta = ((sr * SEARCH_SECS).round() as usize).max(1);

        let hann: Box<[f32]> = (0..wf)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let phase = std::f32::consts::TAU * (i as f32) / (wf as f32);
                0.5 * (1.0 - phase.cos())
            })
            .collect();

        // Compaction keeps `in_fill ≈ wf + 2·delta`; size generously so a
        // single `process` call can always fit a step's appetite plus slack.
        let cap_in = wf + 4 * delta + 4 * hs;
        // `mid` only ever holds a couple of synthesis hops between drains.
        let cap_mid = 4 * wf;

        Self {
            wf,
            hs,
            delta,
            hann,
            pred_scratch: vec![0.0; hs].into_boxed_slice(),
            time_ratio: 1.0,
            pitch_scale: 1.0,
            in_buf: vec![0.0; cap_in * 2].into_boxed_slice(),
            cap_in,
            in_fill: 0,
            analysis_pos: 0.0,
            last_extract: 0.0,
            primed: false,
            acc: vec![0.0; wf * 2].into_boxed_slice(),
            mid: vec![0.0; cap_mid * 2].into_boxed_slice(),
            cap_mid,
            mid_len: 0,
            rs_pos: 0.0,
        }
    }

    /// Effective WSOLA stretch factor (output ÷ input length of the time-domain
    /// stage), before the pitch resample.
    fn stretch(&self) -> f64 {
        (self.time_ratio * self.pitch_scale).clamp(MIN_RATIO, MAX_RATIO)
    }

    /// Search ±`delta` around nominal analysis frame `a` for the extraction
    /// offset whose overlap region best continues the previous frame (max
    /// normalized cross-correlation), **coarse-to-fine**. Returns a frame index
    /// `e` with `e + wf ≤ in_fill`. The prediction's mono signal + norm are
    /// precomputed once into `pred_scratch` so the inner loop isn't re-deriving
    /// them per candidate.
    fn find_extract(&mut self, a: usize) -> usize {
        let e_max = self.in_fill - self.wf; // safe: caller guarantees in_fill ≥ wf
        if !self.primed {
            return a.min(e_max);
        }
        let lo = a.saturating_sub(self.delta);
        let hi = (a + self.delta).min(e_max);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pred0 = self.last_extract.round() as usize + self.hs;

        // Precompute the prediction mono + its norm once.
        let mut norm_t = 0.0f64;
        for j in 0..self.hs {
            let p = self.in_buf[(pred0 + j) * 2] + self.in_buf[(pred0 + j) * 2 + 1];
            self.pred_scratch[j] = p;
            norm_t += f64::from(p) * f64::from(p);
        }
        let inv_norm_t = 1.0 / (norm_t + 1e-12).sqrt();
        let in_buf = &self.in_buf;
        let pred = &self.pred_scratch[..self.hs];

        let mut best_e = a.min(e_max);
        let mut best = f64::NEG_INFINITY;

        // Coarse pass — scan in COARSE_STEP jumps.
        let mut e = lo;
        while e <= hi {
            let s = ncc_at(in_buf, pred, e, inv_norm_t);
            if s > best {
                best = s;
                best_e = e;
            }
            e += COARSE_STEP;
        }
        // Refine ±(COARSE_STEP − 1) around the coarse winner.
        let flo = best_e.saturating_sub(COARSE_STEP - 1).max(lo);
        let fhi = (best_e + COARSE_STEP - 1).min(hi);
        let mut e = flo;
        while e <= fhi {
            let s = ncc_at(in_buf, pred, e, inv_norm_t);
            if s > best {
                best = s;
                best_e = e;
            }
            e += 1;
        }
        best_e
    }

    /// Run one WSOLA synthesis step: extract a frame, window + overlap-add it,
    /// and append `hs` finished frames to `mid`. Pulls more input from
    /// `input`/`cursor` on demand. Returns `false` only when truly starved
    /// (no buffered input and none left in `input`).
    fn synth_step(&mut self, input: &[f32], cursor: &mut usize, in_total: usize) -> bool {
        // Ensure enough lookahead for the search + window + prediction region.
        loop {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let need = self.analysis_pos.round() as usize + self.delta + self.wf;
            if self.in_fill >= need {
                break;
            }
            if self.append_input(input, cursor, in_total) == 0 {
                return false;
            }
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let a = self.analysis_pos.round() as usize;
        let e = self.find_extract(a);

        // Extract + Hann-window + overlap-add into `acc`.
        for j in 0..self.wf {
            let w = self.hann[j];
            self.acc[j * 2] += self.in_buf[(e + j) * 2] * w;
            self.acc[j * 2 + 1] += self.in_buf[(e + j) * 2 + 1] * w;
        }

        // Emit the `hs` finalized frames to `mid`.
        debug_assert!(
            self.mid_len + self.hs <= self.cap_mid,
            "mid buffer overflow"
        );
        let dst = self.mid_len * 2;
        self.mid[dst..dst + self.hs * 2].copy_from_slice(&self.acc[..self.hs * 2]);
        self.mid_len += self.hs;

        // Slide `acc` down by `hs`, zero the exposed tail.
        self.acc.copy_within(self.hs * 2..self.wf * 2, 0);
        for s in &mut self.acc[(self.wf - self.hs) * 2..self.wf * 2] {
            *s = 0.0;
        }

        self.last_extract = e as f64;
        self.primed = true;
        self.analysis_pos += self.hs as f64 / self.stretch();

        self.compact_input();
        true
    }

    /// Append up to free-space frames from `input[cursor..]`. Returns frames
    /// appended.
    fn append_input(&mut self, input: &[f32], cursor: &mut usize, in_total: usize) -> usize {
        let free = self.cap_in - self.in_fill;
        let avail = in_total - *cursor;
        let n = free.min(avail);
        if n == 0 {
            return 0;
        }
        let src = *cursor * 2;
        let dst = self.in_fill * 2;
        self.in_buf[dst..dst + n * 2].copy_from_slice(&input[src..src + n * 2]);
        self.in_fill += n;
        *cursor += n;
        n
    }

    /// Drop input frames that no future step or prediction can reference,
    /// shifting the rest down so `analysis_pos`/`last_extract` stay small.
    ///
    /// Must keep history before **both** cursors: the next search reads back to
    /// `analysis_pos − delta`, and the natural-continuation prediction reads
    /// from `last_extract`. During compression `analysis_pos` races ahead of
    /// `last_extract`, so flooring on `analysis_pos` alone would discard the
    /// prediction region (and push `last_extract` negative) — which collapses
    /// the similarity search into naïve OLA, i.e. resampling, shifting pitch.
    fn compact_input(&mut self) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let a = self.analysis_pos.round() as isize;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let le = self.last_extract.round() as isize;
        let floor = a.min(le) - self.delta as isize - 1;
        if floor <= 0 {
            return;
        }
        let floor = (floor as usize).min(self.in_fill);
        self.in_buf.copy_within(floor * 2..self.in_fill * 2, 0);
        self.in_fill -= floor;
        self.analysis_pos -= floor as f64;
        self.last_extract -= floor as f64;
    }

    /// Drop fully-consumed frames from the front of `mid`.
    fn compact_mid(&mut self) {
        let drop = self.rs_pos.floor();
        if drop < 1.0 {
            return;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let drop = (drop as usize).min(self.mid_len);
        self.mid.copy_within(drop * 2..self.mid_len * 2, 0);
        self.mid_len -= drop;
        self.rs_pos -= drop as f64;
    }
}

impl TimeStretcher for WsolaStretcher {
    fn set_pitch_scale(&mut self, scale: f64) {
        if scale.is_finite() {
            self.pitch_scale = scale.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    fn set_time_ratio(&mut self, ratio: f64) {
        if ratio.is_finite() {
            self.time_ratio = ratio.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> (usize, usize) {
        let in_total = input.len() / 2;
        let out_cap = output.len() / 2;
        let r = self.pitch_scale.clamp(MIN_RATIO, MAX_RATIO);
        let mut cursor = 0usize;
        let mut produced = 0usize;

        while produced < out_cap {
            // Make sure the resampler has a frame and its successor available.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            while self.rs_pos.floor() as usize + 1 >= self.mid_len {
                if !self.synth_step(input, &mut cursor, in_total) {
                    return (cursor, produced);
                }
            }

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i0 = self.rs_pos.floor() as usize;
            #[allow(clippy::cast_possible_truncation)]
            let frac = (self.rs_pos - i0 as f64) as f32;
            let a0 = i0 * 2;
            let l = self.mid[a0] + (self.mid[a0 + 2] - self.mid[a0]) * frac;
            let rr = self.mid[a0 + 1] + (self.mid[a0 + 3] - self.mid[a0 + 1]) * frac;
            output[produced * 2] = l;
            output[produced * 2 + 1] = rr;
            produced += 1;

            self.rs_pos += r;
            self.compact_mid();
        }

        (cursor, produced)
    }

    fn max_output_for(&self, in_frames: usize) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let stretched = (in_frames as f64 * self.time_ratio).ceil() as usize;
        stretched + self.wf + 2
    }

    fn latency_frames(&self) -> usize {
        self.wf
    }

    fn reset(&mut self) {
        for s in self.in_buf.iter_mut() {
            *s = 0.0;
        }
        for s in self.acc.iter_mut() {
            *s = 0.0;
        }
        for s in self.mid.iter_mut() {
            *s = 0.0;
        }
        self.in_fill = 0;
        self.analysis_pos = 0.0;
        self.last_extract = 0.0;
        self.primed = false;
        self.mid_len = 0;
        self.rs_pos = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_no_alloc::AllocDisabler;
    use realfft::RealFftPlanner;

    // One global allocator for the whole `dub-stretch` test binary; lets the
    // RT-safety test below catch any allocation inside `process`.
    #[global_allocator]
    static A: AllocDisabler = AllocDisabler;

    const SR: f32 = 48_000.0;

    /// Interleaved-stereo sine (both channels identical), `frames` long.
    fn sine(freq: f32, frames: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; frames * 2];
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let s = (std::f32::consts::TAU * freq * i as f32 / SR).sin() * 0.5;
            v[i * 2] = s;
            v[i * 2 + 1] = s;
        }
        v
    }

    /// Push all of `input` through `s` and collect every output frame.
    fn drive(s: &mut WsolaStretcher, input: &[f32]) -> Vec<f32> {
        let in_frames = input.len() / 2;
        let mut out = Vec::new();
        let mut cursor = 0usize;
        let mut scratch = vec![0.0f32; 8192 * 2];
        let mut guard = 0;
        loop {
            let (c, p) = s.process(&input[cursor * 2..], &mut scratch);
            cursor += c;
            out.extend_from_slice(&scratch[..p * 2]);
            guard += 1;
            assert!(guard < 1_000_000, "drive did not terminate");
            if c == 0 && p == 0 {
                debug_assert!(cursor <= in_frames);
                break;
            }
        }
        out
    }

    /// RMS of the left channel of an interleaved-stereo buffer.
    fn rms_left(buf: &[f32]) -> f32 {
        let frames = buf.len() / 2;
        if frames == 0 {
            return 0.0;
        }
        let sum: f64 = (0..frames).map(|i| f64::from(buf[i * 2]).powi(2)).sum();
        #[allow(clippy::cast_possible_truncation)]
        {
            (sum / frames as f64).sqrt() as f32
        }
    }

    /// Dominant frequency (Hz) of the left channel's middle `N` frames, via a
    /// Hann-windowed FFT peak-bin search.
    fn fundamental_hz(buf: &[f32]) -> f32 {
        const N: usize = 16_384;
        let frames = buf.len() / 2;
        assert!(
            frames >= N,
            "need ≥ {N} frames to measure pitch, got {frames}"
        );
        let start = (frames - N) / 2;

        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(N);
        let mut indata = r2c.make_input_vec();
        for (i, x) in indata.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let w = 0.5 * (1.0 - (std::f32::consts::TAU * i as f32 / N as f32).cos());
            *x = buf[(start + i) * 2] * w;
        }
        let mut spectrum = r2c.make_output_vec();
        r2c.process(&mut indata, &mut spectrum).unwrap();

        let mut best = 1usize;
        let mut best_mag = 0.0f32;
        for (i, c) in spectrum.iter().enumerate().skip(1) {
            let m = c.norm_sqr();
            if m > best_mag {
                best_mag = m;
                best = i;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        {
            best as f32 * SR / N as f32
        }
    }

    #[test]
    fn unity_preserves_pitch_amplitude_and_length() {
        let input = sine(440.0, 48_000);
        let mut s = WsolaStretcher::new(SR);
        let out = drive(&mut s, &input);

        // Length within a window of the input.
        let in_frames = input.len() / 2;
        let out_frames = out.len() / 2;
        assert!(
            out_frames.abs_diff(in_frames) <= 2 * s.wf,
            "unity length drifted: in {in_frames} out {out_frames}"
        );
        // Pitch unchanged.
        assert!((fundamental_hz(&out) - 440.0).abs() < 8.0);
        // Amplitude (COLA reconstruction) preserved.
        assert!((rms_left(&out) - rms_left(&input)).abs() < 0.03);
    }

    #[test]
    fn length_scales_with_time_ratio() {
        for &ratio in &[0.75_f64, 1.25, 1.5] {
            let input = sine(220.0, 40_000);
            let mut s = WsolaStretcher::new(SR);
            s.set_time_ratio(ratio);
            let out = drive(&mut s, &input);
            let out_frames = out.len() / 2;
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let expected = (40_000.0 * ratio) as usize;
            assert!(
                out_frames.abs_diff(expected) <= 3 * s.wf,
                "ratio {ratio}: expected ~{expected}, got {out_frames}"
            );
        }
    }

    #[test]
    #[ignore = "diagnostic; run with --ignored --nocapture"]
    fn diag_pitch_sweep() {
        for &freq in &[220.0_f32, 440.0] {
            for &ratio in &[0.92_f64, 0.943, 0.98, 1.0, 1.02, 1.064, 1.08, 0.8, 1.25] {
                let input = sine(freq, 96_000);
                let mut s = WsolaStretcher::new(SR);
                s.set_time_ratio(ratio);
                let out = drive(&mut s, &input);
                let f = fundamental_hz(&out);
                let cents = 1200.0 * (f / freq).log2();
                println!("freq={freq:>5} ratio={ratio:>6.3} -> {f:>7.1} Hz  ({cents:+6.1} cents)");
            }
        }
    }

    #[test]
    fn pitch_constant_under_time_stretch() {
        // Time-stretch must NOT move pitch (the whole point of key lock).
        // Includes mild *compression* ratios (< 1, just below unity) — the
        // regime the M14.1 bench caught degenerating into resampling before
        // the compaction fix. `ratio < 1` is the common key-lock case
        // (pitching a record UP → time_ratio = 1/rate < 1).
        for &ratio in &[0.8_f64, 0.92, 0.94, 0.98, 1.0, 1.06, 1.2, 1.5] {
            let input = sine(440.0, 96_000);
            let mut s = WsolaStretcher::new(SR);
            s.set_time_ratio(ratio);
            let out = drive(&mut s, &input);
            let f = fundamental_hz(&out);
            assert!(
                (f - 440.0).abs() < 12.0,
                "time-stretch ×{ratio} moved pitch to {f} Hz (want 440)"
            );
        }
    }

    #[test]
    fn pitch_shifts_with_pitch_scale() {
        // time_ratio = 1, pitch_scale = p → duration fixed, pitch × p.
        for &(p, want) in &[(0.84_f64, 369.6_f32), (1.19, 523.6)] {
            let input = sine(440.0, 96_000);
            let mut s = WsolaStretcher::new(SR);
            s.set_pitch_scale(p);
            let out = drive(&mut s, &input);
            // Duration preserved (time_ratio still 1).
            assert!(out.len().abs_diff(input.len()) <= 4 * s.wf * 2);
            let f = fundamental_hz(&out);
            assert!(
                (f - want).abs() < 14.0,
                "pitch_scale {p} gave {f} Hz (want ~{want})"
            );
        }
    }

    #[test]
    fn process_is_alloc_free() {
        let mut s = WsolaStretcher::new(SR);
        s.set_time_ratio(1.3);
        // Warm up off the assert: prime buffers to steady state.
        let warm = sine(330.0, 8_192);
        let _ = drive(&mut s, &warm);

        let input = sine(330.0, 2_048);
        let mut output = vec![0.0f32; 2_048 * 2];
        assert_no_alloc::assert_no_alloc(|| {
            let _ = s.process(&input, &mut output);
        });
    }

    #[test]
    fn reset_returns_to_initial_state() {
        let mut s = WsolaStretcher::new(SR);
        s.set_time_ratio(1.4);
        let _ = drive(&mut s, &sine(440.0, 20_000));
        s.reset();
        assert_eq!(s.in_fill, 0);
        assert_eq!(s.mid_len, 0);
        assert!(!s.primed);
        assert!((s.analysis_pos).abs() < f64::EPSILON);
        assert!((s.rs_pos).abs() < f64::EPSILON);
    }

    #[test]
    fn golden_signature_stable() {
        // Deterministic signature over a fixed tone + a click, at a fixed
        // ratio. Rounded so float formatting can't drift the snapshot.
        let mut input = sine(440.0, 48_000);
        input[24_000 * 2] = 1.0; // a transient click mid-stream
        input[24_000 * 2 + 1] = 1.0;
        let mut s = WsolaStretcher::new(SR);
        s.set_time_ratio(1.25);
        let out = drive(&mut s, &input);

        let sig = format!(
            "frames={} peak={:.3} rms={:.4} fundamental_hz={:.0}",
            out.len() / 2,
            out.iter().fold(0.0f32, |m, &x| m.max(x.abs())),
            rms_left(&out),
            fundamental_hz(&out),
        );
        insta::assert_snapshot!(sig, @"frames=58559 peak=1.000 rms=0.3527 fundamental_hz=439");
    }
}
