//! Echo-Out: a beat-synced capture + feedback delay (PRD §6.3).
//!
//! ## What it is
//!
//! The classic dub / Pioneer-DJM "echo out", run **100 % wet**: the DJ taps
//! the button on, the last *N* beats of the deck's output are captured, the
//! deck's dry signal is **muted**, and the captured loop repeats with a
//! feedback decay — the phrase echoes away to silence while the operator
//! brings in the next record. Tapping off un-mutes the deck, which has kept
//! playing underneath (slip-aware) and resumes at its advanced position.
//!
//! It is a **toggle**, not a hold: on → mute + echo; off → dry back.
//!
//! ## How it works (single write head)
//!
//! A stereo ring buffer is written continuously while **idle** so the last
//! `capacity` frames of the deck's output are always available ("warm
//! capture"). The read head trails the write head by exactly `delay_frames`,
//! so `ring[w − D]` is the audio from one echo-length ago.
//!
//! On **engage** we stop writing live input and instead recirculate:
//!
//! ```text
//! echo    = ring[w − D]                 // what we play this sample (wet)
//! ring[w] = lowpass(echo) · feedback    // what loops back, quieter + darker
//! ```
//!
//! Because the read trails the write by `D`, every `D` samples the read head
//! laps the buffer and re-reads content multiplied by `feedback` once more and
//! low-passed once more — the captured phrase repeats, decaying geometrically
//! per lap and getting progressively darker (a tape-echo character).
//!
//! ## Dry mute (100 % wet)
//!
//! While engaged the dry is muted (`out = wet`), so only the echo carries;
//! the deck keeps advancing underneath so off resumes at the slipped position.
//! This applies to **every** deck, **including Thru**: a Thru deck's live
//! record flows through the engine (input → bus → output), so "muting" it is
//! simply not writing the passthrough to the output — exactly like a file
//! deck. (The physical record keeps spinning, so off resumes at wherever the
//! needle now is.)
//!
//! ## Real-time safety
//!
//! Both rings are allocated once in [`EchoOut::new`] (off the audio thread).
//! `process_block`, `engage`, `release` and `set_params` are pure float math
//! over pre-allocated storage — no allocation, no locks, no syscalls, no
//! transcendental functions (the low-pass coefficient is computed off-RT by
//! the caller and handed in as `lp_coeff`). Verified under `assert_no_alloc`.

use std::f32::consts::PI;

/// Lowest musical tempo the ring is sized to hold the echo for. The feature
/// is a single **1-beat** echo-out, so the ring only needs one beat at the
/// slowest supported tempo (1 beat at 60 BPM = 1 s); below 60 BPM the echo
/// simply clamps to the ring length. 60 BPM comfortably covers the mixable
/// band. (A 1-beat ring is 4× smaller than the old 4-beat one.)
const MIN_BPM: f32 = 60.0;

/// Echo length the ring is sized for, in beats. One beat — the single
/// echo-out division (PRD §6.3).
const RING_BEATS: f32 = 1.0;

/// Hard ceiling on feedback. Strictly below 1.0 so the captured loop always
/// decays to silence in bounded time, no matter what the UI sends.
pub const MAX_FEEDBACK: f32 = 0.95;

/// Default feedback (PRD §6.3: 60 %).
pub const DEFAULT_FEEDBACK: f32 = 0.6;

/// Default feedback low-pass cutoff (PRD §6.3: 8 kHz).
pub const DEFAULT_LPF_HZ: f32 = 8_000.0;

/// Wet fade-in / dry fade-out ramp length. Long enough to declick the seam
/// where the echo replaces (and later gives back) the dry, short enough to
/// feel immediate.
const RAMP_MS: f32 = 3.0;

/// Below this magnitude a recirculated sample is flushed to zero, so the
/// feedback tail can't spin into denormal arithmetic as it decays.
const DENORMAL_FLOOR: f32 = 1.0e-20;

/// Below this input magnitude the muted dry is considered **silent** — the
/// needle is lifted (Thru), the track has ended (file), or there's a true
/// gap. ≈ −50 dBFS: a quiet-but-present record sits above it; a lifted needle
/// (noise floor) sits well below.
const INPUT_SILENCE_FLOOR: f32 = 3.0e-3;

/// Below this wet magnitude the captured echo loop has **finished** decaying
/// (≈ −80 dBFS).
const WET_SILENCE_FLOOR: f32 = 1.0e-4;

/// How long the input *and* the wet tail must both stay silent before the
/// echo reports it is safe to auto-disengage. Long enough that a brief quiet
/// passage doesn't trip it; the wet tail's own decay supplies most of the
/// delay anyway.
const AUTO_OFF_MS: f32 = 300.0;

/// Engagement state, mirrored to the UI as a `u8` via [`EchoOut::state_code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoState {
    /// Off: transparent passthrough, ring warm-captures the dry.
    Idle,
    /// On: dry muted (internal deck) and the captured loop repeating + decaying.
    Engaged,
}

impl EchoState {
    /// Stable wire value for the FFI / UI indicator (0 off · 1 engaged).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            EchoState::Idle => 0,
            EchoState::Engaged => 1,
        }
    }
}

/// Per-deck echo-out processor. Insert on the deck's output bus, after the
/// deck has written its dry stereo pair.
#[derive(Debug)]
pub struct EchoOut {
    ring_l: Box<[f32]>,
    ring_r: Box<[f32]>,
    /// `capacity − 1`; capacity is a power of two so the write head wraps
    /// with a branchless bitmask.
    mask: usize,
    /// Write head; read index is always `(write − delay_frames) & mask`.
    write: usize,

    /// Active echo length in frames (`≤ capacity`). Snapshotted on engage.
    delay_frames: usize,
    /// Per-lap decay, clamped to `[0, MAX_FEEDBACK]`.
    feedback: f32,
    /// One-pole low-pass coefficient `a = 1 − e^(−2π·fc/sr)` for the
    /// feedback path, computed off-RT by the caller.
    lp_coeff: f32,
    lp_l: f32,
    lp_r: f32,

    state: EchoState,

    /// Applied wet/dry gains, ramped toward their targets each sample.
    wet_gain: f32,
    dry_gain: f32,
    wet_target: f32,
    /// Per-sample linear ramp step for both gains.
    ramp_inc: f32,

    /// Consecutive frames (while engaged) the muted input *and* the wet tail
    /// have both been silent. When it reaches `ready_frames` the echo reports
    /// it is safe to auto-disengage (input gone, echo finished).
    quiet_frames: usize,
    /// `quiet_frames` threshold (= `AUTO_OFF_MS` at the engine sample rate).
    ready_frames: usize,
}

impl EchoOut {
    /// Allocate an echo processor for an output bus running at `sample_rate`.
    /// **Not RT-safe** — allocates the delay rings. Call at engine setup.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let cap = ring_capacity(sample_rate);
        let ramp_frames = (sample_rate * RAMP_MS / 1000.0).max(1.0);
        Self {
            ring_l: vec![0.0; cap].into_boxed_slice(),
            ring_r: vec![0.0; cap].into_boxed_slice(),
            mask: cap - 1,
            write: 0,
            delay_frames: 1,
            feedback: DEFAULT_FEEDBACK,
            lp_coeff: one_pole_coeff(DEFAULT_LPF_HZ, sample_rate),
            lp_l: 0.0,
            lp_r: 0.0,
            state: EchoState::Idle,
            wet_gain: 0.0,
            dry_gain: 1.0,
            wet_target: 0.0,
            ramp_inc: 1.0 / ramp_frames,
            quiet_frames: 0,
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            ready_frames: (sample_rate * AUTO_OFF_MS / 1000.0).max(1.0) as usize,
        }
    }

    /// Engage (toggle on) the echo. `delay_frames`, `feedback` and `lp_coeff`
    /// are fully resolved off-RT (delay from the deck's BPM × division;
    /// coefficient from the cutoff). The captured loop is whatever the ring
    /// has warm-captured up to now — no buffer reset, so an echo of the last
    /// N beats is audible immediately. Re-engaging swaps the length live.
    pub fn engage(&mut self, delay_frames: usize, feedback: f32, lp_coeff: f32) {
        self.delay_frames = delay_frames.clamp(1, self.capacity());
        self.feedback = feedback.clamp(0.0, MAX_FEEDBACK);
        self.lp_coeff = lp_coeff.clamp(0.0, 1.0);
        self.lp_l = 0.0;
        self.lp_r = 0.0;
        self.state = EchoState::Engaged;
        self.wet_target = 1.0;
        self.quiet_frames = 0;
    }

    /// Disengage (toggle off): the dry signal comes back (the deck has kept
    /// playing underneath, slip-aware) and the wet echo fades out. Idempotent
    /// on a deck that isn't engaged.
    pub fn release(&mut self) {
        self.state = EchoState::Idle;
        self.wet_target = 0.0;
    }

    /// Live-update feedback and low-pass while engaged or idle (UI sliders).
    /// `lp_coeff` is computed off-RT from the cutoff. The echo length only
    /// changes on a fresh [`EchoOut::engage`] (changing division re-triggers).
    pub fn set_params(&mut self, feedback: f32, lp_coeff: f32) {
        self.feedback = feedback.clamp(0.0, MAX_FEEDBACK);
        self.lp_coeff = lp_coeff.clamp(0.0, 1.0);
    }

    /// Current engagement state.
    #[must_use]
    pub fn state(&self) -> EchoState {
        self.state
    }

    /// Wire value for the UI indicator + auto-off policy: `0` off, `1`
    /// engaged (still audible — either the input is live or the tail is
    /// ringing), `2` engaged **and ready to auto-off** (the muted input and
    /// the wet tail have both been silent for `AUTO_OFF_MS`). The UI lights
    /// the pad for both `1` and `2`; the deck poll turns the echo off on `2`
    /// so a muted deck doesn't strand the operator in silence (e.g. Thru
    /// needle lifted + echo finished, or a file deck whose track ended).
    #[must_use]
    pub fn state_code(&self) -> u8 {
        match self.state {
            EchoState::Idle => 0,
            EchoState::Engaged if self.quiet_frames >= self.ready_frames => 2,
            EchoState::Engaged => 1,
        }
    }

    /// Process one stereo block in place on the deck's routed output pair.
    ///
    /// `out[f·stride + offset ..][..2]` is the deck's stereo channel for
    /// frame `f`. The deck has already summed its dry signal there; this
    /// rewrites the pair as `dry·dry_gain + wet`. While engaged the dry is
    /// muted (100 % wet) — for every deck, including Thru: a Thru deck's live
    /// record flows through the engine, so "muting" it is just not writing the
    /// passthrough to the output, exactly like a file deck.
    ///
    /// RT-safe: bounded loop, indexed loads/stores, no allocation.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);

        // Mute the dry while engaged; idle passes it through.
        let dry_target = if self.state == EchoState::Engaged {
            0.0
        } else {
            1.0
        };

        // Anything to do to the output? When fully off (idle, wet gone, dry
        // restored) the call is a transparent no-op that just keeps the ring
        // warm.
        let active = self.state == EchoState::Engaged || self.wet_gain > 0.0 || self.dry_gain < 1.0;

        for frame in out.chunks_exact_mut(stride) {
            let in_l = frame[offset];
            let in_r = frame[offset + 1];

            let read = (self.write.wrapping_sub(self.delay_frames)) & self.mask;
            let echo_l = self.ring_l[read];
            let echo_r = self.ring_r[read];

            match self.state {
                EchoState::Idle => {
                    // Warm capture: store the live dry, no recirculation.
                    self.ring_l[self.write] = in_l;
                    self.ring_r[self.write] = in_r;
                }
                EchoState::Engaged => {
                    // Recirculate the captured loop through the one-pole
                    // low-pass, then attenuate by feedback for the next lap.
                    let fl = self.lp_l + self.lp_coeff * (echo_l - self.lp_l);
                    let fr = self.lp_r + self.lp_coeff * (echo_r - self.lp_r);
                    self.lp_l = flush(fl);
                    self.lp_r = flush(fr);
                    self.ring_l[self.write] = flush(fl * self.feedback);
                    self.ring_r[self.write] = flush(fr * self.feedback);

                    // Auto-off watch: the muted input (`in_*`) AND the wet
                    // tail (`echo_*`) must both be silent. Accumulate while
                    // both are quiet; any sound on either resets it.
                    let in_mag = in_l.abs().max(in_r.abs());
                    let wet_mag = echo_l.abs().max(echo_r.abs());
                    if in_mag <= INPUT_SILENCE_FLOOR && wet_mag <= WET_SILENCE_FLOOR {
                        self.quiet_frames = self.quiet_frames.saturating_add(1);
                    } else {
                        self.quiet_frames = 0;
                    }
                }
            }
            self.write = (self.write + 1) & self.mask;

            self.wet_gain = ramp(self.wet_gain, self.wet_target, self.ramp_inc);
            self.dry_gain = ramp(self.dry_gain, dry_target, self.ramp_inc);

            if active {
                frame[offset] = in_l * self.dry_gain + echo_l * self.wet_gain;
                frame[offset + 1] = in_r * self.dry_gain + echo_r * self.wet_gain;
            }
        }
    }

    /// Ring length in frames.
    fn capacity(&self) -> usize {
        self.mask + 1
    }
}

/// Ring capacity: a power-of-two frame count holding `RING_BEATS` at
/// `MIN_BPM` (a 1-beat echo at 60 BPM = 1 s), rounded up.
fn ring_capacity(sample_rate: f32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (RING_BEATS * 60.0 / MIN_BPM * sample_rate).ceil() as usize;
    frames.next_power_of_two().max(1_024)
}

/// One-pole low-pass coefficient `a = 1 − e^(−2π·fc/sr)`. Computed off the
/// audio thread (it calls `exp`); the audio thread only multiplies by it.
#[must_use]
pub fn one_pole_coeff(cutoff_hz: f32, sample_rate: f32) -> f32 {
    let fc = cutoff_hz.clamp(20.0, sample_rate * 0.45);
    1.0 - (-2.0 * PI * fc / sample_rate).exp()
}

#[inline]
fn ramp(current: f32, target: f32, step: f32) -> f32 {
    if current < target {
        (current + step).min(target)
    } else if current > target {
        (current - step).max(target)
    } else {
        current
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

    #[global_allocator]
    static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

    /// Feed one mono sample (duplicated to both channels) and return the
    /// processed left output. Drives a 1-frame block so absolute sample
    /// indexing across the test is trivial.
    fn step(echo: &mut EchoOut, x: f32) -> f32 {
        let mut buf = [x, x];
        echo.process_block(&mut buf, 2, 0);
        buf[0]
    }

    #[test]
    fn idle_is_transparent() {
        let mut echo = EchoOut::new(SR);
        for i in 0..1000 {
            let x = ((i as f32) * 0.01).sin();
            let y = step(&mut echo, x);
            assert!((y - x).abs() < 1e-7, "idle changed sample {i}: {x} -> {y}");
        }
        assert_eq!(echo.state(), EchoState::Idle);
    }

    #[test]
    fn engaged_repeats_captured_loop_with_decay() {
        let mut echo = EchoOut::new(SR);
        let delay = 2_000usize;
        let fb = 0.5f32;

        // Warm capture: a unit impulse at sample 0, then silence to 1000.
        let _ = step(&mut echo, 1.0);
        for _ in 1..1_000 {
            let _ = step(&mut echo, 0.0);
        }

        // Engage, low-pass bypassed (coeff = 1 → identity) so only feedback
        // shapes the tail. Feed silence; the wet is the only output.
        echo.engage(delay, fb, 1.0);
        let mut out = Vec::with_capacity(7_000);
        for _ in 0..7_000 {
            out.push(step(&mut echo, 0.0));
        }

        let peak_at = |abs_sample: usize| out[abs_sample - 1_000];
        let e1 = peak_at(2_000);
        let e2 = peak_at(4_000);
        let e3 = peak_at(6_000);

        assert!((e1 - 1.0).abs() < 1e-3, "1st echo {e1} != 1.0");
        assert!((e2 - fb).abs() < 1e-3, "2nd echo {e2} != {fb}");
        assert!((e3 - fb * fb).abs() < 1e-3, "3rd echo {e3} != {}", fb * fb);
    }

    #[test]
    fn engaged_mutes_dry_to_100_percent_wet() {
        let mut echo = EchoOut::new(SR);
        // Warm with steady DC; engage. Within the first lap the wet equals the
        // captured 0.5 — and the output equals JUST that (dry muted), not
        // 0.5 + 0.5 = 1.0 (which is what layering would give).
        for _ in 0..3_000 {
            let _ = step(&mut echo, 0.5);
        }
        echo.engage(2_000, 0.6, one_pole_coeff(DEFAULT_LPF_HZ, SR));
        let mut last = 0.0;
        for _ in 0..512 {
            last = step(&mut echo, 0.5);
        }
        // ≈ 0.5 (pure wet) proves the dry is muted; layering would be ≈ 1.0.
        assert!((last - 0.5).abs() < 0.06, "dry not muted (100% wet): {last}");
    }

    #[test]
    fn disengage_restores_the_dry() {
        let mut echo = EchoOut::new(SR);
        for _ in 0..3_000 {
            let _ = step(&mut echo, 0.5);
        }
        echo.engage(2_000, 0.6, one_pole_coeff(DEFAULT_LPF_HZ, SR));
        for _ in 0..1_000 {
            let _ = step(&mut echo, 0.5);
        }
        echo.release();
        assert_eq!(echo.state(), EchoState::Idle);
        // After the un-mute ramp the dry is fully back and the echo is gone.
        let mut last = 0.0;
        for _ in 0..512 {
            last = step(&mut echo, 0.5);
        }
        assert!((last - 0.5).abs() < 1e-3, "dry not restored after off: {last}");
    }

    #[test]
    fn feedback_is_clamped_below_unity() {
        let mut echo = EchoOut::new(SR);
        echo.engage(100, 5.0, 1.0); // absurd feedback request
        assert!(echo.feedback <= MAX_FEEDBACK);
    }

    #[test]
    fn reports_ready_to_auto_off_only_when_input_and_tail_both_silent() {
        let mut echo = EchoOut::new(SR);
        // Warm with DC so the captured loop is loud, then engage.
        for _ in 0..2_000 {
            let _ = step(&mut echo, 0.5);
        }
        echo.engage(1_000, 0.6, one_pole_coeff(DEFAULT_LPF_HZ, SR));

        // Tail is loud right after engage → still "engaged" (1), not ready.
        let _ = step(&mut echo, 0.0);
        assert_eq!(echo.state_code(), 1);

        // Silent input (needle lifted): once the tail has decayed AND the
        // quiet window has elapsed, it reports ready to auto-off (2).
        for _ in 0..(SR as usize * 3 / 2) {
            let _ = step(&mut echo, 0.0);
        }
        assert_eq!(echo.state_code(), 2, "never reported ready to auto-off");

        // Sound back on the input (record dropped again) resets it to 1 —
        // we must not auto-off a deck that's audible again.
        let _ = step(&mut echo, 0.5);
        assert_eq!(echo.state_code(), 1, "loud input did not reset auto-off");
    }

    #[test]
    fn fifty_toggle_cycles_stay_finite_and_off() {
        // PRD acceptance §14: engage 50 times in a row, no degradation.
        let mut echo = EchoOut::new(SR);
        for c in 0..50 {
            for i in 0..600 {
                let x = if i < 100 { (c as f32 * 0.1).sin() } else { 0.0 };
                let y = step(&mut echo, x);
                assert!(y.is_finite(), "cycle {c} sample {i} not finite");
            }
            echo.engage(500, 0.7, one_pole_coeff(6_000.0, SR));
            for _ in 0..1_000 {
                assert!(step(&mut echo, 0.0).is_finite());
            }
            echo.release();
            for _ in 0..512 {
                assert!(step(&mut echo, 0.0).is_finite());
            }
            assert_eq!(echo.state(), EchoState::Idle, "cycle {c} did not turn off");
        }
    }

    #[test]
    fn delay_longer_than_ring_is_clamped() {
        let mut echo = EchoOut::new(SR);
        let huge = echo.capacity() * 4;
        echo.engage(huge, 0.6, 1.0);
        assert_eq!(echo.delay_frames, echo.capacity());
        for _ in 0..1_000 {
            assert!(step(&mut echo, 0.1).is_finite());
        }
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut echo = EchoOut::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        for (i, s) in buf.iter_mut().enumerate() {
            *s = ((i as f32) * 0.005).sin();
        }
        assert_no_alloc::assert_no_alloc(|| {
            echo.engage(1_500, 0.7, one_pole_coeff(8_000.0, SR));
            for _ in 0..64 {
                echo.process_block(&mut buf, 4, 2);
            }
            echo.set_params(0.5, one_pole_coeff(4_000.0, SR));
            for _ in 0..64 {
                echo.process_block(&mut buf, 4, 2);
            }
            echo.release();
            for _ in 0..256 {
                echo.process_block(&mut buf, 4, 2);
            }
        });
    }

    #[test]
    fn capacity_is_power_of_two_and_holds_one_beat() {
        // Sized for a 1-beat echo at MIN_BPM (1 beat at 60 BPM = 1 s).
        let cap = ring_capacity(SR);
        assert!(cap.is_power_of_two());
        assert!(cap as f32 >= SR, "ring too short for a 1-beat echo: {cap}");
    }

    #[test]
    fn one_pole_coeff_is_monotonic_in_cutoff() {
        let low = one_pole_coeff(1_000.0, SR);
        let mid = one_pole_coeff(8_000.0, SR);
        let high = one_pole_coeff(16_000.0, SR);
        assert!(low < mid && mid < high);
        assert!((0.0..=1.0).contains(&low) && (0.0..=1.0).contains(&high));
    }
}
