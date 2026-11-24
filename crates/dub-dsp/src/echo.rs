//! Echo-Out: a beat-synced capture + feedback delay (PRD §6.3).
//!
//! ## What it is
//!
//! The classic dub / Pioneer-DJM "echo out", run **100 % wet**: the DJ taps
//! the button on, the deck plays **the next beat** as normal while it is
//! captured, then its dry signal is **muted** and that beat repeats with a
//! feedback decay — the phrase echoes away to silence while the operator
//! brings in the next record. Tapping off un-mutes the deck, which has kept
//! playing underneath (slip-aware) and resumes at its advanced position.
//!
//! The beat *after* the press, not the one before it (Daniel, 2026-09-25):
//! the DJ hits the button on the beat they want to throw away, and hears it
//! go.
//!
//! It is a **toggle**, not a hold: on → mute + echo; off → dry back.
//!
//! ## Following the master (2026-11-22)
//!
//! Once the beat is in, the echo can follow another deck the way a tape
//! echo follows its motor: [`EchoOut::follow`] gives it the master's beat
//! length and, when the master has a grid, how far off its next beat is.
//! The delay then **glides** — the read head runs a little fast or slow,
//! so the repeats shift in pitch for as long as it moves, as tape does —
//! until a lap is one master beat and the captured beat's own beat line
//! lands on the master's. A master pitched up mid-echo speeds the repeats
//! up with it; a record dropped half a beat off pulls the echo onto its
//! beat in a short swoop. The glide is capped at [`MAX_GLIDE`] either way.
//! With nothing to follow the delay holds, which is the echo as it was.
//!
//! ## How it works (single write head)
//!
//! A stereo ring buffer is written continuously while **idle** so the last
//! `capacity` frames of the deck's output are always available ("warm
//! capture"). The read head trails the write head by exactly `delay_frames`,
//! so `ring[w − D]` is the audio from one echo-length ago.
//!
//! On **engage** the echo first *captures*: live input goes out dry for one
//! echo length and into the ring scaled by `feedback`, so when that beat is
//! in, the read head (`w − D`) sits on its first sample and the first repeat
//! is already one step down (beat · fb, then · fb², …). Then it stops
//! writing live input and recirculates:
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
/// Feedback while HOLD is on: just under endless. Over a 20 s record swap
/// at ~90 BPM (~30 laps) the loop loses about 1 dB and darkens a little
/// through the feedback low-pass — it carries the room across the gap, and
/// it can never grow.
pub const HOLD_FEEDBACK: f32 = 0.995;

/// The fastest the delay may glide, in frames of delay per frame: the
/// read head runs at most this much faster or slower than the tape, which
/// is the most the repeats are bent in pitch (±20 % ≈ ±3 semitones). It is
/// what a half-beat pull-in sounds like at its steepest.
pub const MAX_GLIDE: f64 = 0.2;

/// How much of the echo's offset from the master's beat one lap is aimed to
/// take out: the delay heads for `period − sign(offset) · min(PHASE_PULL ·
/// |offset|, brake)`, where `brake` is the most the glide can give back
/// before the next beat line arrives. Taking all of it (1.0) overshot by
/// about 53 ms; 2 and 3 rang; a fraction of the offset measured from the
/// lap's own length halved the miss each beat and left a half-beat throw
/// 11 ms off after five. At 0.8 with the brake it is on the beat by the
/// fourth repeat and does not overshoot.
const PHASE_PULL: f64 = 0.8;

/// The share of the glide's cap the brake counts on having: the glide
/// eases in and out, so it is not at full speed for the whole return.
const BRAKE_MARGIN: f64 = 0.6;

/// Seconds for the glide to settle on its aim: the motor's inertia. It
/// also smooths how the glide starts and stops, so the pitch bends rather
/// than steps.
const GLIDE_SETTLE_SECS: f64 = 0.04;

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
    /// Pressed: the deck plays the next beat dry while it is recorded.
    Capturing,
    /// On: dry muted (internal deck) and the captured loop repeating + decaying.
    Engaged,
}

impl EchoState {
    /// Stable wire value for the FFI / UI indicator (0 off · 1 engaged).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            EchoState::Idle => 0,
            EchoState::Capturing | EchoState::Engaged => 1,
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

    /// Echo length the press asked for, in frames (`≤ capacity`): the length
    /// of the capture.
    delay_frames: usize,
    /// The delay the read head trails by now, in (fractional) frames. Equal
    /// to `delay_frames` until [`EchoOut::follow`] moves it.
    delay: f64,
    /// How fast `delay` is moving, frames per frame: the read head runs at
    /// `1 − glide` of the tape's speed. Eased toward `glide_aim`.
    glide: f64,
    /// Where the glide is heading, set once a block by `follow`.
    glide_aim: f64,
    /// The delay `follow` aims at, or `None` to hold.
    delay_aim: Option<f64>,
    /// Per-frame easing of `glide` toward `glide_aim` (`GLIDE_SETTLE_SECS`).
    glide_ease: f64,
    /// Frames of delay error the glide closes per frame before the cap.
    glide_gain: f64,
    /// Ring index of the captured beat's beat line: the press, or the
    /// outgoing deck's beat line if one fell inside the capture. Moves on
    /// each lap to where the sample is written back.
    marker: Option<f64>,
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
    /// Frames of the capture beat still to record while `Capturing`.
    capture_left: usize,
    /// HOLD: recirculate at [`HOLD_FEEDBACK`] instead of `feedback`.
    hold: bool,
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
            delay: 1.0,
            glide: 0.0,
            glide_aim: 0.0,
            delay_aim: None,
            glide_ease: 1.0 - (-1.0 / (GLIDE_SETTLE_SECS * f64::from(sample_rate)).max(1.0)).exp(),
            glide_gain: 0.25 / (GLIDE_SETTLE_SECS * f64::from(sample_rate)).max(1.0),
            marker: None,
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
            capture_left: 0,
            hold: false,
        }
    }

    /// Engage (toggle on) the echo. `delay_frames`, `feedback` and `lp_coeff`
    /// are fully resolved off-RT (delay from the deck's BPM × division;
    /// coefficient from the cutoff). From off, the deck plays the next
    /// `delay_frames` dry while they are captured, and the echo of exactly
    /// that beat begins as it ends. Re-engaging while echoing swaps the
    /// length live.
    pub fn engage(&mut self, delay_frames: usize, feedback: f32, lp_coeff: f32) {
        self.delay_frames = delay_frames.clamp(1, self.capacity());
        self.feedback = feedback.clamp(0.0, MAX_FEEDBACK);
        self.lp_coeff = lp_coeff.clamp(0.0, 1.0);
        self.quiet_frames = 0;
        if self.state == EchoState::Engaged {
            return;
        }
        self.lp_l = 0.0;
        self.lp_r = 0.0;
        self.state = EchoState::Capturing;
        self.capture_left = self.delay_frames;
        #[allow(clippy::cast_precision_loss)]
        {
            self.delay = self.delay_frames as f64;
        }
        self.glide = 0.0;
        self.glide_aim = 0.0;
        self.delay_aim = None;
        // The press is the beat line until the deck says otherwise.
        #[allow(clippy::cast_precision_loss)]
        {
            self.marker = Some(self.write as f64);
        }
    }

    /// The outgoing deck's beat line passed `frame` frames into the block
    /// about to be processed. During the capture it becomes the point the
    /// echo lines up with the master's beat; afterwards it is ignored.
    pub fn mark_beat(&mut self, frame: usize) {
        if self.state == EchoState::Capturing && frame < self.capture_left {
            #[allow(clippy::cast_precision_loss)]
            {
                self.marker = Some(((self.write + frame) & self.mask) as f64);
            }
        }
    }

    /// Follow a master for the next block: `period` is its beat in output
    /// frames, `to_next_beat` how many output frames until its next beat
    /// line (`None` without a grid, which follows the tempo only). `None`
    /// for `master` holds the delay where it is. Call once a block, before
    /// [`EchoOut::process_block`]; ignored until the beat is captured.
    pub fn follow(&mut self, master: Option<(f64, Option<f64>)>) {
        if self.state != EchoState::Engaged {
            return;
        }
        let Some((period, to_next_beat)) = master else {
            self.delay_aim = None;
            return;
        };
        #[allow(clippy::cast_precision_loss)]
        let longest = (self.capacity() - 4) as f64;
        if !period.is_finite() || period < 2.0 {
            self.delay_aim = None;
            return;
        }
        let period = period.min(longest);
        let mut aim = period;
        if let (Some(to_beat), Some(marker)) = (to_next_beat, self.marker) {
            if to_beat.is_finite() {
                // When the captured beat line next comes round, against when
                // the master's does — the nearer way round.
                let arrives = self.ring_distance(self.read_position(), marker);
                // The miss less what the delay's current stretch adds to
                // it: how far the echo's beat sits off the master's for
                // good. Only a lap of other than one master beat moves that.
                let offset = wrap_half(arrives - to_beat - (self.delay - period), period);
                // Stretch the laps against it, but never further from one
                // beat than the glide can bring them back from before the
                // offset is gone, or the echo swings past the beat.
                let brake = (2.0 * MAX_GLIDE * BRAKE_MARGIN * period * offset.abs()).sqrt();
                aim = period - offset.signum() * (PHASE_PULL * offset.abs()).min(brake);
            }
        }
        self.delay_aim = Some(aim.clamp(2.0, longest));
    }

    /// Where the read head is, as a fractional ring index.
    fn read_position(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let cap = self.capacity() as f64;
        #[allow(clippy::cast_precision_loss)]
        let w = self.write as f64;
        (w - self.delay).rem_euclid(cap)
    }

    /// Frames forward from ring position `from` to ring position `to`.
    fn ring_distance(&self, from: f64, to: f64) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let cap = self.capacity() as f64;
        (to - from).rem_euclid(cap)
    }

    /// The delay the read head trails by, in frames (fractional while it
    /// glides).
    #[must_use]
    pub fn delay(&self) -> f64 {
        self.delay
    }

    /// Disengage (toggle off): the dry signal comes back (the deck has kept
    /// playing underneath, slip-aware) and the wet echo fades out. Idempotent
    /// on a deck that isn't engaged.
    pub fn release(&mut self) {
        self.state = EchoState::Idle;
        self.wet_target = 0.0;
        // The next throw starts as a throw, not still held.
        self.hold = false;
    }

    /// HOLD (toggle): keep the captured beat repeating at just-under-endless
    /// feedback, for the record swap of traditional one-turntable dub —
    /// throw the last phrase, hold it while the needle is lifted and the
    /// next record goes down, release (or turn the echo off) to let it go.
    /// Off returns to the set feedback, so a released hold decays the
    /// normal way. Cleared by [`EchoOut::release`].
    pub fn set_hold(&mut self, on: bool) {
        self.hold = on;
    }

    /// Whether HOLD is on.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.hold
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
            EchoState::Capturing => 1,
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

        // Anything to do to the output? When fully off (idle, wet gone, dry
        // restored) the call is a transparent no-op that just keeps the ring
        // warm.
        let active = self.state != EchoState::Idle || self.wet_gain > 0.0 || self.dry_gain < 1.0;

        for frame in out.chunks_exact_mut(stride) {
            let in_l = frame[offset];
            let in_r = frame[offset + 1];
            // A frame's gains follow the state it *started* in: the last
            // sample of the capture beat is still dry, and the echo begins
            // on the sample after it. Idle and capturing pass the dry.
            let dry_target = if self.state == EchoState::Engaged {
                0.0
            } else {
                1.0
            };
            let wet_target = self.wet_target;

            // The glide eases toward its aim and moves the delay; the read
            // head is where it lands, between two frames.
            if self.state == EchoState::Engaged {
                if let Some(aim) = self.delay_aim {
                    self.glide_aim =
                        ((aim - self.delay) * self.glide_gain).clamp(-MAX_GLIDE, MAX_GLIDE);
                } else {
                    self.glide_aim = 0.0;
                }
                self.glide += (self.glide_aim - self.glide) * self.glide_ease;
                self.delay = (self.delay + self.glide).clamp(2.0, self.longest_delay());
                if let Some(marker) = self.marker {
                    // The read head stepped `1 − glide` frames to get here; if
                    // the beat line was in that step it is read now, and
                    // written back where the write head is — less however
                    // far past it the read head already was, or the line
                    // would creep a fraction of a frame every lap.
                    #[allow(clippy::cast_precision_loss)]
                    let cap = self.capacity() as f64;
                    let since = (self.read_position() - marker).rem_euclid(cap);
                    if since < 1.0 - self.glide {
                        #[allow(clippy::cast_precision_loss)]
                        let here = self.write as f64;
                        self.marker = Some((here - since).rem_euclid(cap));
                    }
                }
            }
            let (echo_l, echo_r) = self.read_interpolated();

            match self.state {
                EchoState::Idle => {
                    // Warm capture: store the live dry, no recirculation.
                    self.ring_l[self.write] = in_l;
                    self.ring_r[self.write] = in_r;
                }
                EchoState::Capturing => {
                    // The beat after the press: play it dry, and record it
                    // already one feedback step down. Recorded at unity, the
                    // first lap replayed the beat at full level — the same
                    // beat twice, a stutter — before the decay began (rig,
                    // 2026-11-11). A delay's first repeat is the first
                    // step of the decay.
                    let feedback = if self.hold {
                        HOLD_FEEDBACK
                    } else {
                        self.feedback
                    };
                    self.ring_l[self.write] = in_l * feedback;
                    self.ring_r[self.write] = in_r * feedback;
                    self.capture_left = self.capture_left.saturating_sub(1);
                    if self.capture_left == 0 {
                        // It is in: from the next sample the read head sits
                        // on its first frame. Mute the dry, bring the wet.
                        self.state = EchoState::Engaged;
                        self.wet_target = 1.0;
                    }
                }
                EchoState::Engaged => {
                    // Recirculate the captured loop through the one-pole
                    // low-pass, then attenuate by feedback for the next lap.
                    let fl = self.lp_l + self.lp_coeff * (echo_l - self.lp_l);
                    let fr = self.lp_r + self.lp_coeff * (echo_r - self.lp_r);
                    self.lp_l = flush(fl);
                    self.lp_r = flush(fr);
                    let feedback = if self.hold {
                        HOLD_FEEDBACK
                    } else {
                        self.feedback
                    };
                    self.ring_l[self.write] = flush(fl * feedback);
                    self.ring_r[self.write] = flush(fr * feedback);

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

            self.wet_gain = ramp(self.wet_gain, wet_target, self.ramp_inc);
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

    /// The longest delay the ring can hold with room to interpolate.
    fn longest_delay(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let cap = self.capacity() as f64;
        cap - 4.0
    }

    /// The ring read `delay` frames behind the write head, between frames
    /// by straight line. At a whole-frame delay it is the frame itself.
    fn read_interpolated(&self) -> (f32, f32) {
        let pos = self.read_position();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i0 = (pos.floor() as usize) & self.mask;
        let i1 = (i0 + 1) & self.mask;
        #[allow(clippy::cast_possible_truncation)]
        let frac = (pos - pos.floor()) as f32;
        (
            self.ring_l[i0] + (self.ring_l[i1] - self.ring_l[i0]) * frac,
            self.ring_r[i0] + (self.ring_r[i1] - self.ring_r[i0]) * frac,
        )
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

/// `x` brought into `[−period/2, period/2)`: the nearer way round a beat.
fn wrap_half(x: f64, period: f64) -> f64 {
    let r = x.rem_euclid(period);
    if r >= period / 2.0 {
        r - period
    } else {
        r
    }
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

    /// Rig, 2026-09-25: the echo is of the beat *after* the press, not
    /// the one before it. The deck plays that beat dry — nothing muted,
    /// no wet — then goes 100 % wet on exactly that beat.
    #[test]
    fn engage_plays_the_next_beat_dry_then_echoes_it() {
        let mut echo = EchoOut::new(SR);
        let delay = 1_000usize;
        // The beat before the press: loud. It must never be heard as echo.
        for _ in 0..delay {
            let _ = step(&mut echo, 0.9);
        }
        echo.engage(delay, 0.5, 1.0);
        assert_eq!(echo.state(), EchoState::Capturing);
        assert_eq!(echo.state_code(), 1, "the pad lights on the press");
        // The beat after the press passes dry, untouched.
        let beat: Vec<f32> = (0..delay)
            .map(|i| (i as f32 / delay as f32) * 0.4)
            .collect();
        for (i, &x) in beat.iter().enumerate() {
            let y = step(&mut echo, x);
            assert!(
                (y - x).abs() < 1e-6,
                "capture beat sample {i} not dry: {x} -> {y}"
            );
        }
        assert_eq!(echo.state(), EchoState::Engaged);
        // Then the dry mutes and that beat repeats (past the 3 ms
        // cross-fade), not the 0.9 before the press — one feedback step
        // down already.
        let ramp = (SR * RAMP_MS / 1000.0) as usize + 2;
        for (i, &x) in beat.iter().enumerate() {
            let y = step(&mut echo, 0.77);
            if i > ramp {
                assert!(
                    (y - 0.5 * x).abs() < 1e-3,
                    "echo sample {i}: {y}, want the captured {x} at feedback 0.5"
                );
            }
        }
    }

    /// Throw one click — the outgoing beat line, at the press — and let it
    /// repeat while `master(frame)` says what to follow each 256-frame
    /// block. Returns the frames at which the repeats arrive.
    fn click_repeats(
        delay: usize,
        blocks: usize,
        master: impl Fn(usize) -> Option<(f64, Option<f64>)>,
    ) -> Vec<f64> {
        let mut echo = EchoOut::new(SR);
        echo.engage(delay, 0.9, 1.0);
        let block = 256usize;
        let mut out = Vec::with_capacity(blocks * block);
        let mut frame = 0usize;
        for _ in 0..blocks {
            echo.follow(master(frame));
            let mut buf = vec![0.0f32; block * 2];
            if frame == 0 {
                buf[0] = 1.0;
                buf[1] = 1.0;
            }
            echo.process_block(&mut buf, 2, 0);
            out.extend(buf.chunks_exact(2).map(|c| c[0]));
            frame += block;
        }
        // Each repeat is a click spread over a frame or two by the glide;
        // its arrival is the weighted middle of that spread.
        let mut arrivals = Vec::new();
        let mut i = delay;
        while i < out.len() {
            if out[i].abs() > 1e-3 {
                let end = (i + 4).min(out.len());
                let (mut w, mut at) = (0.0f64, 0.0f64);
                for (j, x) in out.iter().enumerate().take(end).skip(i) {
                    w += f64::from(x.abs());
                    at += j as f64 * f64::from(x.abs());
                }
                arrivals.push(at / w);
                i = end + delay / 4;
            } else {
                i += 1;
            }
        }
        arrivals
    }

    /// A master pitched up mid-echo speeds the repeats up with it: the
    /// laps settle on its beat, a 120 BPM throw following 131 BPM.
    #[test]
    fn the_repeats_follow_the_masters_tempo() {
        let arrivals = click_repeats(24_000, 1_200, |_| Some((22_000.0, None)));
        let laps: Vec<f64> = arrivals.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(laps.len() > 8, "{laps:?}");
        for lap in &laps[4..] {
            assert!(
                (lap - 22_000.0).abs() < 3.0,
                "a lap of {lap}, not 22 000: {laps:?}"
            );
        }
    }

    /// A master whose beat falls half a beat from the echo's: the echo
    /// pulls onto it within a few beats, and stays.
    #[test]
    fn the_repeats_pull_onto_the_masters_beat() {
        let period = 24_000.0;
        // The master's beats fall half a beat after the press, then every
        // beat: `to_next_beat` counted from the frame the block starts.
        let master_beat = |frame: usize| {
            let since = (frame as f64 - period / 2.0).rem_euclid(period);
            Some((period, Some((period - since) % period)))
        };
        let arrivals = click_repeats(24_000, 1_500, master_beat);
        assert!(arrivals.len() > 12, "{arrivals:?}");
        let off = |at: f64| {
            let e = (at - period / 2.0).rem_euclid(period);
            e.min(period - e)
        };
        assert!(off(arrivals[0]) > 11_000.0, "starts half a beat off");
        // On the beat by the fourth repeat, and no swing past it after.
        for (n, &at) in arrivals.iter().enumerate().skip(3) {
            assert!(
                off(at) < 48.0,
                "repeat {n} lands {} frames off the master's beat: {arrivals:?}",
                off(at)
            );
        }
    }

    /// The pull bends the repeats' pitch, and no more than [`MAX_GLIDE`]:
    /// the delay never moves faster than that many frames per frame, which
    /// is how far the read head runs off the tape's speed.
    #[test]
    fn the_pull_never_bends_further_than_the_cap() {
        let period = 24_000.0;
        let mut echo = EchoOut::new(SR);
        echo.engage(24_000, 0.9, 1.0);
        let block = 256usize;
        let mut frame = 0usize;
        let mut last = echo.delay();
        let mut fastest = 0.0f64;
        for _ in 0..1_500 {
            let since = (frame as f64 - period / 2.0).rem_euclid(period);
            echo.follow(Some((period, Some((period - since) % period))));
            let mut buf = vec![0.0f32; block * 2];
            echo.process_block(&mut buf, 2, 0);
            fastest = fastest.max((echo.delay() - last).abs() / block as f64);
            last = echo.delay();
            frame += block;
        }
        assert!(
            fastest > MAX_GLIDE * 0.9,
            "the pull used the glide: {fastest}"
        );
        assert!(
            fastest <= MAX_GLIDE + 1e-9,
            "glided {fastest} frames per frame"
        );
    }

    /// The master at another tempo *and* off the beat: a 120 BPM throw
    /// under a 125 BPM record dropped a quarter beat out settles on both.
    #[test]
    fn the_repeats_take_the_masters_tempo_and_beat_together() {
        let period = 23_040.0;
        let master_beat = |frame: usize| {
            let since = (frame as f64 - period / 4.0).rem_euclid(period);
            Some((period, Some((period - since) % period)))
        };
        let arrivals = click_repeats(24_000, 1_500, master_beat);
        let off = |at: f64| wrap_half(at - period / 4.0, period).abs();
        for (n, &at) in arrivals.iter().enumerate().skip(5) {
            assert!(off(at) < 48.0, "repeat {n} lands {} frames off", off(at));
        }
        let last = arrivals[arrivals.len() - 1] - arrivals[arrivals.len() - 2];
        assert!((last - period).abs() < 2.0, "settled on laps of {last}");
    }

    /// With nothing to follow the delay holds, and the echo is the one it
    /// always was.
    #[test]
    fn with_nothing_to_follow_the_delay_holds() {
        let arrivals = click_repeats(24_000, 600, |_| None);
        for lap in arrivals.windows(2).map(|w| w[1] - w[0]) {
            assert!((lap - 24_000.0).abs() < 1e-6, "lap {lap}");
        }
    }

    fn rms(xs: &[f32]) -> f32 {
        (xs.iter().map(|x| x * x).sum::<f32>() / xs.len().max(1) as f32).sqrt()
    }

    /// Engage at ~90 BPM, capture one beat of a 200 Hz tone, then feed the
    /// silence of a lifted needle for `secs`. Returns the output.
    fn throw_and_swap(hold: bool, secs: f32) -> Vec<f32> {
        let mut echo = EchoOut::new(SR);
        let beat = (SR * 60.0 / 90.0) as usize;
        echo.engage(beat, DEFAULT_FEEDBACK, one_pole_coeff(DEFAULT_LPF_HZ, SR));
        for i in 0..beat {
            let _ = step(&mut echo, 0.5 * (i as f32 * 2.0 * PI * 200.0 / SR).sin());
        }
        echo.set_hold(hold);
        (0..(SR * secs) as usize)
            .map(|_| step(&mut echo, 0.0))
            .collect()
    }

    /// Traditional one-turntable dub (Daniel, 2026-10-06): throw the last
    /// phrase into the echo, HOLD it while the record is swapped — 10 to
    /// 20 s — then drop the needle on the next tune. Held, the echo is
    /// still there at the end of a 20 s swap, within 3 dB, and never louder
    /// than it started.
    #[test]
    fn hold_keeps_the_echo_alive_through_a_record_swap() {
        let out = throw_and_swap(true, 20.0);
        let sr = SR as usize;
        let early = rms(&out[sr..2 * sr]);
        let late = rms(&out[19 * sr..20 * sr]);
        assert!(early > 0.05, "nothing captured: {early}");
        assert!(late > early * 0.7, "held echo faded: {early} → {late}");
        assert!(late <= early * 1.01, "held echo grew: {early} → {late}");
    }

    /// The contrast: at the default feedback it is gone long before the
    /// next record is down — the reason HOLD exists.
    #[test]
    fn without_hold_the_echo_is_gone_before_the_swap_ends() {
        let out = throw_and_swap(false, 20.0);
        let sr = SR as usize;
        let early = rms(&out[sr..2 * sr]);
        let late = rms(&out[10 * sr..11 * sr]);
        assert!(
            late < early * 0.01,
            "echo outlived the swap: {early} → {late}"
        );
    }

    /// HOLD off: the held loop decays the normal way.
    #[test]
    fn releasing_hold_lets_the_echo_decay() {
        let mut echo = EchoOut::new(SR);
        let beat = (SR * 60.0 / 90.0) as usize;
        echo.engage(beat, DEFAULT_FEEDBACK, one_pole_coeff(DEFAULT_LPF_HZ, SR));
        for i in 0..beat {
            let _ = step(&mut echo, 0.5 * (i as f32 * 2.0 * PI * 200.0 / SR).sin());
        }
        echo.set_hold(true);
        let held: Vec<f32> = (0..SR as usize * 3).map(|_| step(&mut echo, 0.0)).collect();
        echo.set_hold(false);
        let after: Vec<f32> = (0..SR as usize * 10)
            .map(|_| step(&mut echo, 0.0))
            .collect();
        let sr = SR as usize;
        assert!(rms(&after[9 * sr..]) < rms(&held[2 * sr..]) * 0.01);
    }

    /// Turning the echo off clears HOLD, so the next throw decays as
    /// normal instead of hanging on forever.
    #[test]
    fn turning_the_echo_off_clears_hold() {
        let mut echo = EchoOut::new(SR);
        echo.engage(1_000, DEFAULT_FEEDBACK, 1.0);
        echo.set_hold(true);
        echo.release();
        assert!(!echo.is_held());
    }

    /// Pressed again inside the capture beat: nothing was muted, nothing
    /// echoes — the deck just carries on.
    #[test]
    fn release_while_capturing_never_echoes() {
        let mut echo = EchoOut::new(SR);
        echo.engage(1_000, 0.6, 1.0);
        for _ in 0..300 {
            let _ = step(&mut echo, 0.5);
        }
        echo.release();
        assert_eq!(echo.state(), EchoState::Idle);
        for i in 0..3_000 {
            let y = step(&mut echo, 0.25);
            assert!((y - 0.25).abs() < 1e-6, "sample {i} carried an echo: {y}");
        }
    }

    #[test]
    fn engaged_repeats_captured_loop_with_decay() {
        let mut echo = EchoOut::new(SR);
        let delay = 2_000usize;
        let fb = 0.5f32;

        // Engage, low-pass bypassed (coeff = 1 → identity) so only feedback
        // shapes the tail. The captured beat is the one after the press: a
        // unit impulse inside it — clear of the 3 ms cross-fade at the seam
        // where the echo takes over — and silence around it.
        echo.engage(delay, fb, 1.0);
        let mut out = Vec::with_capacity(7_500);
        for i in 0..7_500 {
            out.push(step(&mut echo, if i == 500 { 1.0 } else { 0.0 }));
        }

        // The impulse played dry at 500; it comes back one beat later, and
        // once a beat after that, decaying.
        let e0 = out[500];
        let e1 = out[2_500];
        let e2 = out[4_500];
        let e3 = out[6_500];

        assert!((e0 - 1.0).abs() < 1e-6, "the throw itself {e0} != 1.0");
        assert!((e1 - fb).abs() < 1e-3, "1st echo {e1} != {fb}");
        assert!((e2 - fb * fb).abs() < 1e-3, "2nd echo {e2} != {}", fb * fb);
        assert!(
            (e3 - fb * fb * fb).abs() < 1e-3,
            "3rd echo {e3} != {}",
            fb * fb * fb
        );
    }

    /// Rig, 2026-11-11: the first repeat came back at full level — the
    /// thrown beat twice at the same volume, a stutter — and only then
    /// decayed. Every repeat, the first included, is quieter than the
    /// throw.
    #[test]
    fn the_first_repeat_is_already_quieter_than_the_throw() {
        let mut echo = EchoOut::new(SR);
        let beat = 4_800usize;
        echo.engage(beat, DEFAULT_FEEDBACK, one_pole_coeff(DEFAULT_LPF_HZ, SR));
        let tone = |i: usize| 0.5 * (i as f32 * 2.0 * PI * 200.0 / SR).sin();
        let thrown: Vec<f32> = (0..beat).map(|i| step(&mut echo, tone(i))).collect();
        let first: Vec<f32> = (0..beat).map(|_| step(&mut echo, 0.0)).collect();
        let ratio = rms(&first) / rms(&thrown);
        assert!(
            ratio < 0.7,
            "first repeat at {ratio:.2} of the throw — want about {DEFAULT_FEEDBACK}"
        );
        assert!(
            ratio > DEFAULT_FEEDBACK * 0.85,
            "first repeat at {ratio:.2} of the throw — dropped too far"
        );
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
        assert!(
            (last - 0.5).abs() < 0.06,
            "dry not muted (100% wet): {last}"
        );
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
        assert!(
            (last - 0.5).abs() < 1e-3,
            "dry not restored after off: {last}"
        );
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
