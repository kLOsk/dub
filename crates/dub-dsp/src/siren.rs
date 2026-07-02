//! Dub Siren: a synthesised siren / sound-system FX generator with a built-in
//! slap-back delay (PRD §6.3, milestone M16).
//!
//! ## What it is
//!
//! The classic reggae / sound-system siren box (Rigsmith GS1) crossed with the
//! 80s "space gun" toy sound chips — a bank of preset sounds (siren, alarm,
//! laser, bomb, machine gun, …) you trigger by tapping a pad or a key. Unlike
//! [`crate::echo::EchoOut`] — which *processes* a deck's existing signal — the
//! siren is a **generator**: [`SirenVoice::process_block`] **sums** into the
//! deck's output pair (additive). It sounds with or without a track and is
//! unaffected by the deck's echo-out.
//!
//! ## Synthesis — modelled on the TI SN76477 "complex sound generator"
//!
//! The cheap toy gun/explosion chips of the era (SN76477, UM3561) build every
//! sound from the same handful of blocks, and this voice mirrors them:
//!
//! - an **oscillator** (the VCO) with a one-shot **pitch sweep** (laser zap,
//!   bomb whistle, riser) and a periodic **pitch LFO** (the SLF — siren wail,
//!   alarm, UFO);
//! - a **noise** source through a **low-pass filter** (the noise filter) — the
//!   percussive body of guns and explosions (filtered noise = "boom"/"thud",
//!   *not* a distorted tone);
//! - an **amplitude envelope** with a fast attack and an **exponential decay**
//!   (the cap-discharge envelope) — the natural fall of an explosion;
//! - a **tremolo** that can use a per-cycle **decay** wave to *retrigger* the
//!   shot — that's the machine gun (a gunshot fired ~10×/s);
//! - a **slap-back delay** for the dub echo.
//!
//! ```text
//! sweep = one-shot pitch glide;  lfo = periodic pitch mod
//! freq  = clamp(base·sweep + lfo·depth, …)
//! src   = osc(freq)·tone + noise        (noise gated by its onset delay)
//! body  = lowpass2(src)                 (the noise filter → boom/thud)
//! dry   = body · env · volume · trem    (env: attack then exp-decay; trem retriggers)
//! out  += dry + delay(dry)·mix          (ADDITIVE)
//! ```
//!
//! A preset is a tap one-shot: [`SirenVoice::engage`] starts it with a fixed
//! `gate_frames`, then the envelope decays and the slap-back rings out. The
//! voice reports [`SirenState::Idle`] only once both have fallen silent.
//!
//! ## Real-time safety
//!
//! Wavetables + delay ring are allocated once in [`SirenVoice::new`] (off-RT;
//! the only `sin`). Preset patches are resolved off-RT ([`siren_preset_patch`]
//! calls `exp`/`powf`); `engage`, `release` and `process_block` are pure float
//! math over pre-allocated storage — no allocation, locks, syscalls or
//! transcendentals. Noise is an integer xorshift PRNG. Phase is `f64`; buffers
//! `f32`. Feedback + filter paths are denormal-protected. Verified under
//! `assert_no_alloc`.

use std::f32::consts::PI;

use crate::echo::one_pole_coeff;

/// Wavetable length. Power of two so phase wraps with a bitmask.
const TABLE_LEN: usize = 2048;
const TABLE_MASK: usize = TABLE_LEN - 1;
const TABLE_COUNT: usize = 5;

/// Decay-wave steepness: `value = 2·e^(−DECAY_SHAPE·p) − 1` over one cycle.
/// Used as a tremolo wave to retrigger a percussive shot (machine gun).
const DECAY_SHAPE: f32 = 6.0;

/// Lowest oscillator frequency the sweep / LFO can reach (Hz).
pub const MIN_FREQ: f32 = 20.0;

/// Oscillator pitch is clamped below this fraction of the sample rate.
const MAX_FREQ_RATIO: f32 = 0.45;

/// Fastest pitch-LFO rate (Hz).
pub const MAX_LFO_RATE: f32 = 40.0;

/// Fastest tremolo / retrigger rate (Hz) — fast enough for a machine gun.
pub const MAX_TREM_RATE: f32 = 60.0;

/// Hard ceiling on the slap-back feedback (strictly below 1.0).
pub const MAX_FEEDBACK: f32 = 0.95;

/// Output level ceiling.
pub const MAX_VOLUME: f32 = 2.0;

/// Ceiling on the noise makeup gain (caps the boost for very low cutoffs).
const MAX_NOISE_MAKEUP: f32 = 16.0;

/// Longest slap-back delay the ring is sized for (seconds).
const MAX_DELAY_SECS: f32 = 1.0;

/// Below this magnitude a recirculated / filtered sample is flushed to zero so
/// the feedback + filter states can't drift into denormal arithmetic.
const DENORMAL_FLOOR: f32 = 1.0e-20;

/// Envelope level below which the voice is considered silent.
const ENV_FLOOR: f32 = 1.0e-4;

/// Delay-tail magnitude below which the slap-back is considered finished.
const WET_FLOOR: f32 = 1.0e-4;

/// How long the envelope *and* the tail must both stay silent before the voice
/// reports [`SirenState::Idle`] (gates the UI pad glow, not audio).
const QUIET_MS: f32 = 50.0;

/// Smallest envelope attack step, so a "0 ms" attack still ramps (declicks).
const MIN_RAMP_INC: f32 = 1.0e-5;

/// Exponential-decay target: the release falls to −60 dB over its length.
const DECAY_TARGET: f32 = 0.001;

/// Oscillator / LFO / tremolo waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SirenWave {
    /// Sine — smoothest.
    Sine,
    /// Triangle.
    Triangle,
    /// Square — two-tone / hard gate.
    Square,
    /// Saw — buzzy ramp.
    Saw,
    /// Per-cycle exponential decay — as a tremolo wave it retriggers a
    /// percussive shot each cycle (the machine gun).
    Decay,
}

impl SirenWave {
    /// Decode a wire value (`0` sine · `1` triangle · `2` square · `3` saw ·
    /// `4` decay); anything else falls back to [`SirenWave::Sine`].
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => SirenWave::Triangle,
            2 => SirenWave::Square,
            3 => SirenWave::Saw,
            4 => SirenWave::Decay,
            _ => SirenWave::Sine,
        }
    }

    /// Stable wire value.
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    #[inline]
    fn index(self) -> usize {
        self as usize
    }
}

/// Audible state, mirrored to the UI as a `u8` via [`SirenVoice::state_code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SirenState {
    /// Silent: envelope off and the delay tail finished.
    Idle,
    /// Sounding: gated, decaying, or the delay tail still ringing.
    Sounding,
}

impl SirenState {
    /// Stable wire value (`0` idle · `1` sounding).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            SirenState::Idle => 0,
            SirenState::Sounding => 1,
        }
    }
}

/// A fully resolved, engine-domain siren preset — all coefficients, increments
/// and frame counts computed off-RT (see [`siren_preset_patch`]). Held in the
/// engine and handed to [`SirenVoice::engage`] by reference; never crosses the
/// command channel, so its size is free.
#[derive(Debug, Clone, Copy)]
pub struct SirenPatch {
    /// Oscillator waveform.
    pub osc_wave: SirenWave,
    /// Base pitch (Hz) before sweep / LFO.
    pub base_freq: f32,
    /// Pitch-LFO waveform.
    pub lfo_wave: SirenWave,
    /// Pitch-LFO rate (Hz).
    pub lfo_rate: f32,
    /// Pitch-LFO depth (Hz, ±).
    pub lfo_depth: f32,
    /// One-shot pitch glide: starting multiplier on `base_freq`.
    pub sweep_start_mult: f32,
    /// One-shot pitch glide: ending multiplier on `base_freq`.
    pub sweep_end_mult: f32,
    /// Per-sample multiplier carrying the glide from start to end (1.0 = none).
    pub sweep_step: f32,
    /// Glide duration in frames (0 = no sweep).
    pub sweep_frames: u32,
    /// Tremolo / retrigger waveform.
    pub trem_wave: SirenWave,
    /// Tremolo / retrigger rate (Hz).
    pub trem_rate: f32,
    /// Tremolo depth (`0` none .. `1` full gate).
    pub trem_depth: f32,
    /// White-noise blend once the noise gate opens (`0` pure tone .. `1` pure
    /// noise) — the body of guns / explosions.
    pub noise_amount: f32,
    /// Frames before the noise blends in (`0` = immediately).
    pub noise_onset_frames: u32,
    /// One-pole coefficient for the source low-pass (the "noise filter") — its
    /// starting value. Lower = darker / boomier. Noise makeup tracks it.
    pub source_lp_coeff: f32,
    /// Source low-pass coefficient it sweeps *to* while the noise sounds (the
    /// SN76477 explosion's filter falls as it decays — a darkening boom).
    pub source_lp_end_coeff: f32,
    /// Per-sample multiplier carrying the source-filter coefficient from start
    /// to end (1.0 = no sweep).
    pub source_lp_step: f32,
    /// Source-filter sweep length in frames (0 = no sweep), counted only while
    /// the noise is sounding.
    pub source_lp_sweep_frames: u32,
    /// Per-sample envelope attack step (linear).
    pub attack_inc: f32,
    /// Per-sample envelope release multiplier (exponential decay, `< 1`).
    pub release_coeff: f32,
    /// Slap-back length in frames.
    pub delay_frames: u32,
    /// Slap-back per-lap feedback (`0..MAX_FEEDBACK`).
    pub delay_feedback: f32,
    /// Slap-back wet mix (`0..1`).
    pub delay_mix: f32,
    /// One-pole low-pass coefficient for the feedback path.
    pub delay_lp_coeff: f32,
    /// Output level (`0..MAX_VOLUME`).
    pub volume: f32,
    /// Lo-fi sample-and-hold length in output frames (1 = off). Decimating the
    /// output aliases it — the crude, edgy "8-bit" character of the real chips.
    pub crush_hold: u32,
    /// Lo-fi amplitude quantization: number of levels per side (0 = off). Fewer
    /// = grittier / more "8-bit".
    pub crush_levels: f32,
    /// One-shot duration in frames (after which the envelope releases).
    pub gate_frames: u32,
}

/// One per-deck siren voice. Insert on the deck's output bus *after* the deck
/// (and its echo-out) have written their stereo pair; the siren sums on top.
#[derive(Debug)]
pub struct SirenVoice {
    tables: [Box<[f32]>; TABLE_COUNT],
    /// `TABLE_LEN / sample_rate` — a per-sample phase increment is one multiply.
    table_len_over_sr: f64,
    max_freq: f32,

    osc_wave: SirenWave,
    lfo_wave: SirenWave,
    trem_wave: SirenWave,
    osc_phase: f64,
    lfo_phase: f64,
    trem_phase: f64,
    base_freq: f32,
    lfo_rate: f32,
    lfo_depth: f32,

    sweep_mult: f32,
    sweep_step: f32,
    sweep_end_mult: f32,
    sweep_frames_left: u32,

    trem_rate: f32,
    trem_depth: f32,

    /// White-noise generator (xorshift), its blend + onset countdown, and the
    /// one-pole "noise filter" (the boom/thud shaper) with an optional cutoff
    /// sweep. Makeup gain (computed per sample from the current cutoff) restores
    /// the level the filter would otherwise throw away.
    noise_state: u32,
    noise_amount: f32,
    noise_onset_left: u32,
    source_lp_coeff: f32,
    source_lp_end_coeff: f32,
    source_lp_step: f32,
    source_lp_sweep_left: u32,
    source_lp1: f32,

    env: f32,
    env_target: f32,
    attack_inc: f32,
    release_coeff: f32,
    volume: f32,
    gate_active: bool,
    one_shot: bool,
    gate_frames_left: u32,

    delay: Box<[f32]>,
    delay_mask: usize,
    delay_write: usize,
    delay_frames: usize,
    delay_feedback: f32,
    delay_mix: f32,
    delay_lp_coeff: f32,
    delay_lp: f32,

    /// Lo-fi output stage: sample-and-hold (rate crush) + amplitude
    /// quantization (bit crush) — the crude "8-bit edgy" character.
    crush_hold: u32,
    crush_counter: u32,
    crush_held: f32,
    crush_levels: f32,

    state: SirenState,
    quiet_frames: usize,
    ready_frames: usize,
}

impl SirenVoice {
    /// Allocate a siren voice for an output bus at `sample_rate`. **Not RT-safe**
    /// — fills the wavetables (the only `sin`) and the delay ring.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let delay_cap = delay_capacity(sample_rate);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ready_frames = (sample_rate * QUIET_MS / 1000.0).max(1.0) as usize;
        Self {
            tables: [
                build_table(SirenWave::Sine),
                build_table(SirenWave::Triangle),
                build_table(SirenWave::Square),
                build_table(SirenWave::Saw),
                build_table(SirenWave::Decay),
            ],
            table_len_over_sr: f64::from(TABLE_LEN as u32) / f64::from(sample_rate),
            max_freq: sample_rate * MAX_FREQ_RATIO,
            osc_wave: SirenWave::Sine,
            lfo_wave: SirenWave::Sine,
            trem_wave: SirenWave::Sine,
            osc_phase: 0.0,
            lfo_phase: 0.0,
            trem_phase: 0.0,
            base_freq: 600.0,
            lfo_rate: 0.0,
            lfo_depth: 0.0,
            sweep_mult: 1.0,
            sweep_step: 1.0,
            sweep_end_mult: 1.0,
            sweep_frames_left: 0,
            trem_rate: 0.0,
            trem_depth: 0.0,
            noise_state: 0x2545_f491,
            noise_amount: 0.0,
            noise_onset_left: 0,
            source_lp_coeff: 1.0,
            source_lp_end_coeff: 1.0,
            source_lp_step: 1.0,
            source_lp_sweep_left: 0,
            source_lp1: 0.0,
            env: 0.0,
            env_target: 0.0,
            attack_inc: 0.01,
            release_coeff: 0.99,
            volume: 0.0,
            gate_active: false,
            one_shot: false,
            gate_frames_left: 0,
            delay: vec![0.0; delay_cap].into_boxed_slice(),
            delay_mask: delay_cap - 1,
            delay_write: 0,
            delay_frames: 1,
            delay_feedback: 0.0,
            delay_mix: 0.0,
            delay_lp_coeff: 1.0,
            delay_lp: 0.0,
            crush_hold: 1,
            crush_counter: 0,
            crush_held: 0.0,
            crush_levels: 0.0,
            state: SirenState::Idle,
            quiet_frames: 0,
            ready_frames,
        }
    }

    /// Start (or re-trigger) the voice with `patch` (a resolved preset). Phases
    /// and filter state reset so each trigger is deterministic; the attack ramp
    /// masks the reset transient. All inputs are clamped defensively.
    pub fn engage(&mut self, patch: &SirenPatch) {
        self.osc_wave = patch.osc_wave;
        self.lfo_wave = patch.lfo_wave;
        self.trem_wave = patch.trem_wave;
        self.base_freq = patch.base_freq.clamp(MIN_FREQ, self.max_freq);
        self.lfo_rate = patch.lfo_rate.clamp(0.0, MAX_LFO_RATE);
        self.lfo_depth = patch.lfo_depth.clamp(0.0, self.max_freq);

        let start = clamp_mult(patch.sweep_start_mult);
        self.sweep_end_mult = clamp_mult(patch.sweep_end_mult);
        self.sweep_step = if patch.sweep_step.is_finite() && patch.sweep_step > 0.0 {
            patch.sweep_step
        } else {
            1.0
        };
        self.sweep_frames_left = patch.sweep_frames;
        self.sweep_mult = start;

        self.trem_rate = patch.trem_rate.clamp(0.0, MAX_TREM_RATE);
        self.trem_depth = patch.trem_depth.clamp(0.0, 1.0);

        self.noise_amount = patch.noise_amount.clamp(0.0, 1.0);
        self.noise_onset_left = patch.noise_onset_frames;
        self.source_lp_coeff = patch.source_lp_coeff.clamp(0.0, 1.0);
        self.source_lp_end_coeff = patch.source_lp_end_coeff.clamp(0.0, 1.0);
        self.source_lp_step = if patch.source_lp_step.is_finite() && patch.source_lp_step > 0.0 {
            patch.source_lp_step
        } else {
            1.0
        };
        self.source_lp_sweep_left = patch.source_lp_sweep_frames;
        self.source_lp1 = 0.0;

        self.attack_inc = patch.attack_inc.max(MIN_RAMP_INC);
        self.release_coeff = patch.release_coeff.clamp(0.0, 0.999_99);
        self.volume = patch.volume.clamp(0.0, MAX_VOLUME);

        self.delay_frames = (patch.delay_frames as usize).clamp(1, self.delay_capacity());
        self.delay_feedback = patch.delay_feedback.clamp(0.0, MAX_FEEDBACK);
        self.delay_mix = patch.delay_mix.clamp(0.0, 1.0);
        self.delay_lp_coeff = patch.delay_lp_coeff.clamp(0.0, 1.0);
        self.delay_lp = 0.0;

        self.crush_hold = patch.crush_hold.max(1);
        self.crush_levels = patch.crush_levels.max(0.0);
        self.crush_counter = 0;
        self.crush_held = 0.0;

        self.osc_phase = 0.0;
        self.lfo_phase = 0.0;
        self.trem_phase = 0.0;
        self.env_target = 1.0;
        self.gate_active = true;
        self.one_shot = patch.gate_frames > 0;
        self.gate_frames_left = patch.gate_frames;
        self.state = SirenState::Sounding;
        self.quiet_frames = 0;
    }

    /// Release a sustained voice (or cut a one-shot short): the envelope decays
    /// while the delay tail rings on. Idempotent if not sounding.
    pub fn release(&mut self) {
        self.env_target = 0.0;
        self.gate_active = false;
    }

    /// Current audible state.
    #[must_use]
    pub fn state(&self) -> SirenState {
        self.state
    }

    /// Wire value for the UI indicator (`0` idle · `1` sounding).
    #[must_use]
    pub fn state_code(&self) -> u8 {
        self.state.code()
    }

    /// Process one stereo block, **summing** the siren into the deck's routed
    /// output pair `out[f·stride + offset ..][..2]` (centred mono). When idle
    /// and silent the call adds nothing.
    ///
    /// RT-safe: bounded loop, indexed loads/stores, no allocation.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);

        if self.state == SirenState::Idle && self.env <= 0.0 {
            return;
        }

        let len = TABLE_LEN as f64;
        let lfo_inc = f64::from(self.lfo_rate) * self.table_len_over_sr;
        let trem_inc = f64::from(self.trem_rate) * self.table_len_over_sr;
        let osc_table = &self.tables[self.osc_wave.index()];
        let lfo_table = &self.tables[self.lfo_wave.index()];
        let trem_table = &self.tables[self.trem_wave.index()];

        for frame in out.chunks_exact_mut(stride) {
            // One-shot countdown: when the gate elapses, begin the release.
            if self.one_shot && self.gate_active {
                if self.gate_frames_left == 0 {
                    self.gate_active = false;
                    self.env_target = 0.0;
                } else {
                    self.gate_frames_left -= 1;
                }
            }

            // Envelope: linear attack, exponential decay (the cap-discharge fall
            // of a real explosion / gunshot).
            if self.env < self.env_target {
                self.env = (self.env + self.attack_inc).min(self.env_target);
            } else if self.env > self.env_target {
                self.env *= self.release_coeff;
                if self.env <= ENV_FLOOR {
                    self.env = self.env_target;
                }
            }

            // One-shot pitch glide toward the end multiplier, then hold.
            if self.sweep_frames_left > 0 {
                self.sweep_mult *= self.sweep_step;
                self.sweep_frames_left -= 1;
            } else {
                self.sweep_mult = self.sweep_end_mult;
            }

            // Pitch = swept base + periodic LFO, clamped to a safe band.
            let lfo_v = read_table(lfo_table, self.lfo_phase);
            let freq = (self.base_freq * self.sweep_mult + lfo_v * self.lfo_depth)
                .clamp(MIN_FREQ, self.max_freq);
            let osc_inc = f64::from(freq) * self.table_len_over_sr;

            // Tremolo: `amp ∈ [1 − depth, 1]`. With the Decay wave each cycle
            // spikes then falls — a retriggered percussive shot (machine gun).
            let trem_v = read_table(trem_table, self.trem_phase);
            let amp_mod = 1.0 - self.trem_depth * (0.5 - 0.5 * trem_v);

            // Noise layer (xorshift), gated by its onset delay so e.g. the bomb
            // whistles first, then cracks into a noise explosion. While the noise
            // sounds, the source filter sweeps toward its end cutoff — the
            // SN76477 explosion's "darkening boom".
            let mut noise_v = 0.0;
            let mut tone_gain = 1.0;
            if self.noise_amount > 0.0 {
                if self.noise_onset_left > 0 {
                    self.noise_onset_left -= 1;
                } else {
                    noise_v = xorshift_noise(&mut self.noise_state);
                    tone_gain = 1.0 - self.noise_amount;
                    if self.source_lp_sweep_left > 0 {
                        self.source_lp_coeff *= self.source_lp_step;
                        self.source_lp_sweep_left -= 1;
                    } else {
                        self.source_lp_coeff = self.source_lp_end_coeff;
                    }
                }
            }

            let osc_v = read_table(osc_table, self.osc_phase);
            // Makeup-boost the noise *before* the filter so what survives the
            // low-pass is at full level (a low-pass on white noise otherwise
            // discards most of its energy → near-silence). Recomputed per sample
            // so it tracks the filter sweep.
            let makeup = if self.noise_amount > 0.0 {
                noise_makeup_gain(self.source_lp_coeff)
            } else {
                1.0
            };
            let src = osc_v * tone_gain + noise_v * makeup * self.noise_amount;

            // Source low-pass (the "noise filter", a single RC like the SN76477)
            // turns raw noise into a boom / thud instead of a hiss.
            self.source_lp1 =
                flush(self.source_lp1 + self.source_lp_coeff * (src - self.source_lp1));
            let body = self.source_lp1;

            let dry = (body * self.env * self.volume * amp_mod).clamp(-MAX_VOLUME, MAX_VOLUME);

            // Slap-back: read one tap back, low-pass the feedback, recirculate.
            let read = self.delay_write.wrapping_sub(self.delay_frames) & self.delay_mask;
            let wet = self.delay[read];
            self.delay_lp = flush(self.delay_lp + self.delay_lp_coeff * (wet - self.delay_lp));
            self.delay[self.delay_write] = flush(dry + self.delay_lp * self.delay_feedback);
            self.delay_write = (self.delay_write + 1) & self.delay_mask;

            let mut s = dry + wet * self.delay_mix;

            // Lo-fi output stage — the crude "8-bit edgy" character of the era's
            // chips: quantize the amplitude (bit crush), then sample-and-hold to
            // a lower rate (decimation → aliasing). Both off by default.
            if self.crush_levels > 0.0 {
                s = (s * self.crush_levels).round() / self.crush_levels;
            }
            if self.crush_hold > 1 {
                if self.crush_counter == 0 {
                    self.crush_held = s;
                    self.crush_counter = self.crush_hold;
                }
                self.crush_counter -= 1;
                s = self.crush_held;
            }

            self.osc_phase = wrap(self.osc_phase + osc_inc, len);
            self.lfo_phase = wrap(self.lfo_phase + lfo_inc, len);
            self.trem_phase = wrap(self.trem_phase + trem_inc, len);

            // Self-terminate once released, the envelope is silent and the tail
            // has decayed for the dwell window (turns the UI pad off).
            if !self.gate_active && self.env <= ENV_FLOOR && wet.abs() <= WET_FLOOR {
                self.quiet_frames = self.quiet_frames.saturating_add(1);
                if self.quiet_frames >= self.ready_frames {
                    self.state = SirenState::Idle;
                }
            } else {
                self.quiet_frames = 0;
            }

            frame[offset] += s;
            frame[offset + 1] += s;
        }
    }

    fn delay_capacity(&self) -> usize {
        self.delay_mask + 1
    }
}

/// Number of built-in siren presets (Simple mode, M16).
pub const SIREN_PRESET_COUNT: usize = SIREN_PRESETS.len();

/// Resolve preset `id` into an engine-domain [`SirenPatch`] at `sample_rate`.
/// **Not RT-safe** (calls `exp` / `powf`); the engine precomputes the whole bank
/// at construction. Out-of-range ids fall back to preset 0.
#[must_use]
pub fn siren_preset_patch(id: usize, sample_rate: f32) -> SirenPatch {
    let spec = SIREN_PRESETS.get(id).copied().unwrap_or(SIREN_PRESETS[0]);
    spec.resolve(sample_rate)
}

/// Display name for preset `id` (empty string if out of range).
#[must_use]
pub fn siren_preset_name(id: usize) -> &'static str {
    SIREN_PRESETS.get(id).map_or("", |s| s.name)
}

/// Number of **Benidub DS01E** MODE tones (the analog-siren unit).
pub const BENIDUB_PRESET_COUNT: usize = BENIDUB_PRESETS.len();

/// Resolve Benidub DS01E MODE `id` into a [`SirenPatch`] at `sample_rate`.
/// The DS01E is an *analog* oscillator siren (clean, no lo-fi crush), played
/// through the [`SirenVoice`] engine. Dry — its echo is the external PT2399
/// (the siren's onboard echo), so these carry no internal slap. **Not RT-safe**
/// (calls `exp`/`powf`); resolve off-RT. Out-of-range ids fall back to 0.
#[must_use]
pub fn benidub_preset_patch(id: usize, sample_rate: f32) -> SirenPatch {
    let spec = BENIDUB_PRESETS
        .get(id)
        .copied()
        .unwrap_or(BENIDUB_PRESETS[0]);
    spec.resolve(sample_rate)
}

/// Display name for Benidub DS01E MODE `id` (empty string if out of range).
#[must_use]
pub fn benidub_preset_name(id: usize) -> &'static str {
    BENIDUB_PRESETS.get(id).map_or("", |s| s.name)
}

/// Which engine renders a given UI preset slot. The variants are the three
/// vintage-FX voices; Simple mode currently routes everything to the HK628
/// (the toy chip the sounds are modelled on), with the generic siren and the
/// SN76477 reserved for Expert mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SirenRoute {
    /// The generic [`SirenVoice`]; the id indexes its own preset bank.
    Generic,
    /// The analog [`crate::sn76477::Sn76477`] model at this chip-preset index.
    Sn(usize),
    /// The digital [`crate::hk628::Hk628`] model at this program index.
    Hk(usize),
}

/// Route a UI preset slot to its renderer. Simple mode is the Honsitak HK628's
/// eight sounds (rifle / alarm / dual-tone / bombs / electric guns), so every
/// slot maps 1:1 to an HK628 program. (`Generic` / `Sn` are kept for the
/// Expert-mode chip selector.)
#[must_use]
pub fn siren_preset_route(id: usize) -> SirenRoute {
    SirenRoute::Hk(id)
}

/// A musical (human-unit) preset definition, resolved to a [`SirenPatch`].
#[derive(Debug, Clone, Copy)]
struct SirenPresetSpec {
    name: &'static str,
    osc_wave: SirenWave,
    base_freq: f32,
    lfo_wave: SirenWave,
    lfo_rate: f32,
    lfo_depth: f32,
    sweep_start_mult: f32,
    sweep_end_mult: f32,
    sweep_ms: f32,
    trem_wave: SirenWave,
    trem_rate: f32,
    trem_depth: f32,
    noise_amount: f32,
    noise_onset_ms: f32,
    /// Source low-pass cutoff (Hz) at the start. Low = boom/thud; high = open.
    source_lpf_hz: f32,
    /// Cutoff (Hz) the filter falls to over `source_lp_sweep_ms` while the noise
    /// sounds (= `source_lpf_hz` for no sweep). Falling = a darkening boom.
    source_lpf_end_hz: f32,
    source_lp_sweep_ms: f32,
    attack_ms: f32,
    release_ms: f32,
    delay_ms: f32,
    delay_feedback: f32,
    delay_mix: f32,
    delay_lpf_hz: f32,
    volume: f32,
    /// Lo-fi sample-rate the output is decimated to (Hz); `>= sr` (or 0) = off.
    crush_rate_hz: f32,
    /// Lo-fi output bit depth; `>= 16` (or 0) = off.
    crush_bits: f32,
    gate_ms: f32,
}

impl SirenPresetSpec {
    fn resolve(self, sr: f32) -> SirenPatch {
        let sweep_frames = ms_to_frames(self.sweep_ms, sr);
        let sweep_step = if sweep_frames > 0 && self.sweep_start_mult > 0.0 {
            (self.sweep_end_mult / self.sweep_start_mult).powf(1.0 / sweep_frames as f32)
        } else {
            1.0
        };
        let source_lp_coeff = one_pole_coeff(self.source_lpf_hz, sr);
        let source_lp_end_coeff = one_pole_coeff(self.source_lpf_end_hz, sr);
        let source_lp_sweep_frames = ms_to_frames(self.source_lp_sweep_ms, sr);
        let source_lp_step = if source_lp_sweep_frames > 0 && source_lp_coeff > 0.0 {
            (source_lp_end_coeff / source_lp_coeff).powf(1.0 / source_lp_sweep_frames as f32)
        } else {
            1.0
        };
        SirenPatch {
            osc_wave: self.osc_wave,
            base_freq: self.base_freq,
            lfo_wave: self.lfo_wave,
            lfo_rate: self.lfo_rate,
            lfo_depth: self.lfo_depth,
            sweep_start_mult: self.sweep_start_mult,
            sweep_end_mult: self.sweep_end_mult,
            sweep_step,
            sweep_frames,
            trem_wave: self.trem_wave,
            trem_rate: self.trem_rate,
            trem_depth: self.trem_depth,
            noise_amount: self.noise_amount,
            noise_onset_frames: ms_to_frames(self.noise_onset_ms, sr),
            source_lp_coeff,
            source_lp_end_coeff,
            source_lp_step,
            source_lp_sweep_frames,
            attack_inc: ramp_inc(self.attack_ms, sr),
            release_coeff: decay_coeff(self.release_ms, sr),
            delay_frames: ms_to_frames(self.delay_ms, sr).max(1),
            delay_feedback: self.delay_feedback,
            delay_mix: self.delay_mix,
            delay_lp_coeff: one_pole_coeff(self.delay_lpf_hz, sr),
            volume: self.volume,
            crush_hold: crush_hold_frames(self.crush_rate_hz, sr),
            crush_levels: crush_levels(self.crush_bits),
            gate_frames: ms_to_frames(self.gate_ms, sr).max(1),
        }
    }
}

/// Output sample-and-hold length for a lo-fi crush to `rate_hz` (1 = off).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn crush_hold_frames(rate_hz: f32, sample_rate: f32) -> u32 {
    if rate_hz > 0.0 && rate_hz < sample_rate {
        (sample_rate / rate_hz).round().max(1.0) as u32
    } else {
        1
    }
}

/// Amplitude quantization levels per side for a `bits`-bit crush (0 = off).
fn crush_levels(bits: f32) -> f32 {
    if (1.0..16.0).contains(&bits) {
        2.0f32.powf(bits - 1.0)
    } else {
        0.0
    }
}

/// Source cutoff that leaves a tonal preset effectively unfiltered.
const OPEN_LPF_HZ: f32 = 20_000.0;

/// The **Benidub DS01E** analog-siren voice — the unit's **MODE** selector's
/// four classic DS01 tones (Sine 1 / Sine 2 / Test Tone / Square), played
/// through [`SirenVoice`]. Clean (no lo-fi crush, no noise) and **dry** — the
/// echo is the external PT2399 (the DS01E's echo section). On the real unit
/// **PITCH** (3 base freqs) and **RATE** (LFO speed) are separate controls; these
/// specs bake mid-pitch / mid-rate defaults that those controls will override.
#[rustfmt::skip]
const BENIDUB_PRESETS: [SirenPresetSpec; 4] = [
    // MODE 1 — Sine 1: the classic siren, fat + smooth (round, meaty).
    SirenPresetSpec {
        name: "Sine 1", osc_wave: SirenWave::Sine, base_freq: 500.0,
        lfo_wave: SirenWave::Triangle, lfo_rate: 5.0, lfo_depth: 280.0,
        sweep_start_mult: 1.0, sweep_end_mult: 1.0, sweep_ms: 0.0,
        trem_wave: SirenWave::Sine, trem_rate: 0.0, trem_depth: 0.0,
        noise_amount: 0.0, noise_onset_ms: 0.0,
        source_lpf_hz: OPEN_LPF_HZ, source_lpf_end_hz: OPEN_LPF_HZ, source_lp_sweep_ms: 0.0,
        attack_ms: 6.0, release_ms: 160.0,
        delay_ms: 250.0, delay_feedback: 0.3, delay_mix: 0.0, delay_lpf_hz: 6_000.0,
        volume: 0.5, crush_rate_hz: 0.0, crush_bits: 0.0, gate_ms: 2_500.0,
    },
    // MODE 2 — Sine 2: raw + mean, thinner with more bite (higher, faster).
    SirenPresetSpec {
        name: "Sine 2", osc_wave: SirenWave::Sine, base_freq: 760.0,
        lfo_wave: SirenWave::Triangle, lfo_rate: 7.0, lfo_depth: 360.0,
        sweep_start_mult: 1.0, sweep_end_mult: 1.0, sweep_ms: 0.0,
        trem_wave: SirenWave::Sine, trem_rate: 0.0, trem_depth: 0.0,
        noise_amount: 0.0, noise_onset_ms: 0.0,
        source_lpf_hz: OPEN_LPF_HZ, source_lpf_end_hz: OPEN_LPF_HZ, source_lp_sweep_ms: 0.0,
        attack_ms: 4.0, release_ms: 150.0,
        delay_ms: 250.0, delay_feedback: 0.3, delay_mix: 0.0, delay_lpf_hz: 6_000.0,
        volume: 0.5, crush_rate_hz: 0.0, crush_bits: 0.0, gate_ms: 2_500.0,
    },
    // MODE 3 — Test Tone: the beep, gated on/off by the LFO (amplitude, no
    // pitch mod). Square tremolo at the LFO rate = the steady beep-beep.
    SirenPresetSpec {
        name: "Test Tone", osc_wave: SirenWave::Sine, base_freq: 880.0,
        lfo_wave: SirenWave::Sine, lfo_rate: 0.0, lfo_depth: 0.0,
        sweep_start_mult: 1.0, sweep_end_mult: 1.0, sweep_ms: 0.0,
        trem_wave: SirenWave::Square, trem_rate: 6.0, trem_depth: 1.0,
        noise_amount: 0.0, noise_onset_ms: 0.0,
        source_lpf_hz: OPEN_LPF_HZ, source_lpf_end_hz: OPEN_LPF_HZ, source_lp_sweep_ms: 0.0,
        attack_ms: 3.0, release_ms: 120.0,
        delay_ms: 250.0, delay_feedback: 0.3, delay_mix: 0.0, delay_lpf_hz: 6_000.0,
        volume: 0.5, crush_rate_hz: 0.0, crush_bits: 0.0, gate_ms: 2_500.0,
    },
    // MODE 4 — Square: high-low square-modulated pitch (two-tone). A square is
    // perceptually much louder than a sine at equal amplitude, so its level is
    // dropped well below the sine modes to sit level with them.
    SirenPresetSpec {
        name: "Square", osc_wave: SirenWave::Square, base_freq: 560.0,
        lfo_wave: SirenWave::Square, lfo_rate: 5.0, lfo_depth: 180.0,
        sweep_start_mult: 1.0, sweep_end_mult: 1.0, sweep_ms: 0.0,
        trem_wave: SirenWave::Sine, trem_rate: 0.0, trem_depth: 0.0,
        noise_amount: 0.0, noise_onset_ms: 0.0,
        source_lpf_hz: OPEN_LPF_HZ, source_lpf_end_hz: OPEN_LPF_HZ, source_lp_sweep_ms: 0.0,
        attack_ms: 4.0, release_ms: 140.0,
        delay_ms: 250.0, delay_feedback: 0.3, delay_mix: 0.0, delay_lpf_hz: 6_000.0,
        volume: 0.22, crush_rate_hz: 0.0, crush_bits: 0.0, gate_ms: 2_500.0,
    },
];

/// The built-in classic sounds (Simple mode), covering the GS1 + SN76477
/// families — sirens, alarms, lasers, bombs, guns. Order is the UI pad /
/// keyboard order; the FFI fires by index, so keep it stable.
const SIREN_PRESETS: [SirenPresetSpec; 8] = [
    // Classic up-down wail (the user-approved one).
    SirenPresetSpec {
        name: "Siren",
        osc_wave: SirenWave::Sine,
        base_freq: 700.0,
        lfo_wave: SirenWave::Triangle,
        lfo_rate: 5.5,
        lfo_depth: 350.0,
        sweep_start_mult: 1.0,
        sweep_end_mult: 1.0,
        sweep_ms: 0.0,
        trem_wave: SirenWave::Sine,
        trem_rate: 0.0,
        trem_depth: 0.0,
        noise_amount: 0.0,
        noise_onset_ms: 0.0,
        source_lpf_hz: OPEN_LPF_HZ,
        source_lpf_end_hz: OPEN_LPF_HZ,
        source_lp_sweep_ms: 0.0,
        attack_ms: 8.0,
        release_ms: 180.0,
        delay_ms: 320.0,
        delay_feedback: 0.5,
        delay_mix: 0.4,
        delay_lpf_hz: 6_000.0,
        volume: 0.85,
        crush_rate_hz: 0.0,
        crush_bits: 0.0,
        gate_ms: 1_400.0,
    },
    // Two-tone emergency alarm.
    SirenPresetSpec {
        name: "Alarm",
        osc_wave: SirenWave::Square,
        base_freq: 660.0,
        lfo_wave: SirenWave::Square,
        lfo_rate: 7.0,
        lfo_depth: 180.0,
        sweep_start_mult: 1.0,
        sweep_end_mult: 1.0,
        sweep_ms: 0.0,
        trem_wave: SirenWave::Sine,
        trem_rate: 0.0,
        trem_depth: 0.0,
        noise_amount: 0.0,
        noise_onset_ms: 0.0,
        source_lpf_hz: 6_000.0,
        source_lpf_end_hz: 6_000.0,
        source_lp_sweep_ms: 0.0,
        attack_ms: 4.0,
        release_ms: 120.0,
        delay_ms: 260.0,
        delay_feedback: 0.45,
        delay_mix: 0.4,
        delay_lpf_hz: 5_000.0,
        volume: 0.8,
        crush_rate_hz: 0.0,
        crush_bits: 0.0,
        gate_ms: 2_000.0,
    },
    // SN76477 "phasor": a square VCO zapped down in pitch. Classic "pew".
    SirenPresetSpec {
        name: "Laser",
        osc_wave: SirenWave::Square,
        base_freq: 420.0,
        lfo_wave: SirenWave::Sine,
        lfo_rate: 0.0,
        lfo_depth: 0.0,
        sweep_start_mult: 6.0,
        sweep_end_mult: 0.22,
        sweep_ms: 220.0,
        trem_wave: SirenWave::Sine,
        trem_rate: 0.0,
        trem_depth: 0.0,
        noise_amount: 0.0,
        noise_onset_ms: 0.0,
        source_lpf_hz: 6_000.0,
        source_lpf_end_hz: 6_000.0,
        source_lp_sweep_ms: 0.0,
        attack_ms: 2.0,
        release_ms: 90.0,
        delay_ms: 200.0,
        delay_feedback: 0.5,
        delay_mix: 0.5,
        delay_lpf_hz: 8_000.0,
        volume: 0.85,
        crush_rate_hz: 11_000.0,
        crush_bits: 6.0,
        gate_ms: 260.0,
    },
    // Bomb drop: a sine whistle falls for ~0.7 s, then the noise gate opens into
    // a noise explosion whose filter falls from bright (~1.8 kHz) to dark
    // (~300 Hz) as it decays — the SN76477 "darkening boom".
    SirenPresetSpec {
        name: "Bomb",
        osc_wave: SirenWave::Square,
        base_freq: 540.0,
        lfo_wave: SirenWave::Sine,
        lfo_rate: 0.0,
        lfo_depth: 0.0,
        sweep_start_mult: 2.4,
        sweep_end_mult: 0.1,
        sweep_ms: 700.0,
        trem_wave: SirenWave::Sine,
        trem_rate: 0.0,
        trem_depth: 0.0,
        noise_amount: 0.95,
        noise_onset_ms: 650.0,
        source_lpf_hz: 1_800.0,
        source_lpf_end_hz: 300.0,
        source_lp_sweep_ms: 700.0,
        attack_ms: 6.0,
        release_ms: 800.0,
        delay_ms: 300.0,
        delay_feedback: 0.45,
        delay_mix: 0.4,
        delay_lpf_hz: 1_800.0,
        volume: 0.75,
        crush_rate_hz: 9_000.0,
        crush_bits: 6.0,
        gate_ms: 760.0,
    },
    // Machine gun: a noise shot retriggered ~12×/s by the Decay tremolo — each
    // cycle a bright crack that decays (SN76477 gunshot, repeated).
    SirenPresetSpec {
        name: "Machine Gun",
        osc_wave: SirenWave::Square,
        base_freq: 110.0,
        lfo_wave: SirenWave::Sine,
        lfo_rate: 0.0,
        lfo_depth: 0.0,
        sweep_start_mult: 1.0,
        sweep_end_mult: 1.0,
        sweep_ms: 0.0,
        trem_wave: SirenWave::Decay,
        trem_rate: 12.0,
        trem_depth: 1.0,
        noise_amount: 0.9,
        noise_onset_ms: 0.0,
        source_lpf_hz: 2_200.0,
        source_lpf_end_hz: 2_200.0,
        source_lp_sweep_ms: 0.0,
        attack_ms: 1.0,
        release_ms: 120.0,
        delay_ms: 250.0,
        delay_feedback: 0.35,
        delay_mix: 0.28,
        delay_lpf_hz: 2_500.0,
        volume: 0.55,
        crush_rate_hz: 8_000.0,
        crush_bits: 5.0,
        gate_ms: 1_400.0,
    },
    // Lickshot: one noise crack whose filter snaps from bright (~3 kHz) to a
    // dark thud, the echo giving the "lickshot" tail.
    SirenPresetSpec {
        name: "Lickshot",
        osc_wave: SirenWave::Square,
        base_freq: 300.0,
        lfo_wave: SirenWave::Sine,
        lfo_rate: 0.0,
        lfo_depth: 0.0,
        sweep_start_mult: 5.0,
        sweep_end_mult: 0.25,
        sweep_ms: 55.0,
        trem_wave: SirenWave::Sine,
        trem_rate: 0.0,
        trem_depth: 0.0,
        noise_amount: 0.85,
        noise_onset_ms: 0.0,
        source_lpf_hz: 3_000.0,
        source_lpf_end_hz: 900.0,
        source_lp_sweep_ms: 120.0,
        attack_ms: 1.0,
        release_ms: 200.0,
        delay_ms: 270.0,
        delay_feedback: 0.5,
        delay_mix: 0.55,
        delay_lpf_hz: 3_000.0,
        volume: 0.6,
        crush_rate_hz: 10_000.0,
        crush_bits: 6.0,
        gate_ms: 70.0,
    },
    // Fast wobble.
    SirenPresetSpec {
        name: "UFO",
        osc_wave: SirenWave::Sine,
        base_freq: 800.0,
        lfo_wave: SirenWave::Sine,
        lfo_rate: 11.0,
        lfo_depth: 250.0,
        sweep_start_mult: 1.0,
        sweep_end_mult: 1.0,
        sweep_ms: 0.0,
        trem_wave: SirenWave::Sine,
        trem_rate: 0.0,
        trem_depth: 0.0,
        noise_amount: 0.0,
        noise_onset_ms: 0.0,
        source_lpf_hz: OPEN_LPF_HZ,
        source_lpf_end_hz: OPEN_LPF_HZ,
        source_lp_sweep_ms: 0.0,
        attack_ms: 8.0,
        release_ms: 200.0,
        delay_ms: 340.0,
        delay_feedback: 0.6,
        delay_mix: 0.5,
        delay_lpf_hz: 7_000.0,
        volume: 0.8,
        crush_rate_hz: 0.0,
        crush_bits: 0.0,
        gate_ms: 2_000.0,
    },
    // Rising take-off sweep.
    SirenPresetSpec {
        name: "Riser",
        osc_wave: SirenWave::Saw,
        base_freq: 200.0,
        lfo_wave: SirenWave::Sine,
        lfo_rate: 0.0,
        lfo_depth: 0.0,
        sweep_start_mult: 0.5,
        sweep_end_mult: 4.0,
        sweep_ms: 1_200.0,
        trem_wave: SirenWave::Sine,
        trem_rate: 0.0,
        trem_depth: 0.0,
        noise_amount: 0.0,
        noise_onset_ms: 0.0,
        source_lpf_hz: 7_000.0,
        source_lpf_end_hz: 7_000.0,
        source_lp_sweep_ms: 0.0,
        attack_ms: 10.0,
        release_ms: 200.0,
        delay_ms: 360.0,
        delay_feedback: 0.6,
        delay_mix: 0.5,
        delay_lpf_hz: 7_000.0,
        volume: 0.85,
        crush_rate_hz: 0.0,
        crush_bits: 0.0,
        gate_ms: 1_400.0,
    },
];

/// Build a single-cycle wavetable for `wave` (the only `sin` call site).
fn build_table(wave: SirenWave) -> Box<[f32]> {
    let mut table = vec![0.0f32; TABLE_LEN];
    for (i, s) in table.iter_mut().enumerate() {
        let p = i as f32 / TABLE_LEN as f32; // [0, 1)
        *s = match wave {
            SirenWave::Sine => (2.0 * PI * p).sin(),
            // Rises -1 → +1 over the first half, falls back: peak at centre.
            SirenWave::Triangle => {
                if p < 0.5 {
                    -1.0 + 4.0 * p
                } else {
                    3.0 - 4.0 * p
                }
            }
            SirenWave::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            SirenWave::Saw => 2.0 * p - 1.0,
            // Exponential decay across the cycle: +1 at the start (the shot's
            // transient) falling toward -1 (silence) by the cycle's end.
            SirenWave::Decay => 2.0 * (-DECAY_SHAPE * p).exp() - 1.0,
        };
    }
    table.into_boxed_slice()
}

/// Linearly interpolate `table` (length [`TABLE_LEN`]) at fractional `phase` in
/// `[0, TABLE_LEN)`.
#[inline]
fn read_table(table: &[f32], phase: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i0 = phase as usize & TABLE_MASK;
    let frac = (phase - phase.floor()) as f32;
    let i1 = (i0 + 1) & TABLE_MASK;
    table[i0] + frac * (table[i1] - table[i0])
}

/// Advance a wavetable phase, wrapping into `[0, len)`.
#[inline]
fn wrap(phase: f64, len: f64) -> f64 {
    if phase >= len {
        phase - len
    } else {
        phase
    }
}

/// Slap-back ring capacity: a power-of-two frame count holding `MAX_DELAY_SECS`.
fn delay_capacity(sample_rate: f32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (sample_rate * MAX_DELAY_SECS).ceil() as usize;
    frames.next_power_of_two().max(1_024)
}

/// Per-sample linear attack step for a `millis`-long ramp at `sample_rate`.
fn ramp_inc(millis: f32, sample_rate: f32) -> f32 {
    let frames = (sample_rate * millis / 1000.0).max(1.0);
    (1.0 / frames).max(MIN_RAMP_INC)
}

/// Per-sample exponential-decay multiplier reaching −60 dB over `millis`.
fn decay_coeff(millis: f32, sample_rate: f32) -> f32 {
    let frames = (sample_rate * millis / 1000.0).max(1.0);
    DECAY_TARGET.powf(1.0 / frames).clamp(0.0, 0.999_99)
}

/// Makeup gain that restores white-noise level after a one-pole low-pass with
/// coefficient `c`: the filter's noise RMS gain is `sqrt(c/(2−c))`, so the
/// inverse `sqrt((2−c)/c)` brings the filtered noise back to ≈ unity RMS.
fn noise_makeup_gain(c: f32) -> f32 {
    let c = c.clamp(1.0e-4, 1.0);
    ((2.0 - c) / c).sqrt().clamp(1.0, MAX_NOISE_MAKEUP)
}

/// Convert a `millis` duration to a frame count at `sample_rate` (rounded ≥ 0).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ms_to_frames(millis: f32, sample_rate: f32) -> u32 {
    (sample_rate * millis / 1000.0).round().max(0.0) as u32
}

/// Keep a pitch multiplier finite and positive.
#[inline]
fn clamp_mult(m: f32) -> f32 {
    if m.is_finite() && m > 0.0 {
        m.min(64.0)
    } else {
        1.0
    }
}

/// Advance an xorshift32 PRNG and return white noise in `[-1, 1)`. Integer math
/// with one cast — RT-safe, no allocation. Takes the state field by `&mut` so
/// the caller can run it while the wavetable slices are borrowed.
#[inline]
#[allow(clippy::cast_precision_loss)]
fn xorshift_noise(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    ((x >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
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

    // The crate's test-binary `#[global_allocator]` is declared in `echo.rs`.

    fn render(voice: &mut SirenVoice, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let mut buf = [0.0f32, 0.0f32];
            voice.process_block(&mut buf, 2, 0);
            out.push(buf[0]);
        }
        out
    }

    const SCRATCH_DIR: &str =
        "/private/tmp/claude-501/-Users-klos-Development-dub/05fb1678-ea88-4fb6-bb52-d61e8c879ac7/scratchpad";

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

    /// Render every Benidub DS01E MODE tone for offline listening. Ignored;
    /// run with `cargo test -p dub-dsp dump_benidub_wavs -- --ignored`.
    #[test]
    #[ignore]
    fn dump_benidub_wavs() {
        for id in 0..BENIDUB_PRESET_COUNT {
            let patch = benidub_preset_patch(id, SR);
            let mut v = SirenVoice::new(SR);
            v.engage(&patch);
            let samples = render(&mut v, (SR * 3.0) as usize);
            let slug = benidub_preset_name(id)
                .to_lowercase()
                .replace([' ', '-'], "_");
            write_wav(
                &format!("{SCRATCH_DIR}/benidub_{id}_{slug}.wav"),
                &samples,
                SR as u32,
            );
        }
    }

    /// Render the Advanced DUB super-knob sweep on a wailing HK628 preset
    /// (Alarm) through the PT2399 echo, at macro 0 / 0.3 / 0.6 / 1.0 — mirrors
    /// `siren_dub_macro()` in dub-ffi. Ignored; run with
    /// `cargo test -p dub-dsp dump_dub_macro_wavs -- --ignored`.
    #[test]
    #[ignore]
    fn dump_dub_macro_wavs() {
        for (label, m) in [("00", 0.0f32), ("03", 0.3), ("06", 0.6), ("10", 1.0)] {
            let speed = 1.0 - m * 0.35;
            let delay_ms = 90.0 + m * 330.0;
            let feedback = m * 0.85;
            let mix = m * 0.8;
            let mut voice = crate::hk628::Hk628::new(SR);
            voice.set_speed(speed);
            voice.trigger(crate::hk628::hk628_program(2)); // Alarm (a wail)
            let mut echo = crate::pt2399::Pt2399::new(SR);
            echo.set_delay_ms(delay_ms);
            echo.set_feedback(feedback);
            echo.set_mix(mix);
            let mut samples = Vec::new();
            for _ in 0..(SR * 4.0) as usize {
                let mut buf = [0.0f32, 0.0f32];
                voice.process_block(&mut buf, 2, 0);
                echo.process_block(&mut buf, 2, 0); // mix 0 = transparent (dry)
                samples.push(buf[0]);
            }
            write_wav(
                &format!("{SCRATCH_DIR}/dub_macro_{label}.wav"),
                &samples,
                SR as u32,
            );
        }
    }

    fn count_zero_crossings(samples: &[f32]) -> usize {
        let mut crossings = 0;
        for w in samples.windows(2) {
            if (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0) {
                crossings += 1;
            }
        }
        crossings
    }

    /// A bare patch — one oscillator, no LFO / sweep / tremolo / noise / delay,
    /// source filter wide open — that a test can mutate for a single feature.
    fn bare_patch() -> SirenPatch {
        SirenPatch {
            osc_wave: SirenWave::Sine,
            base_freq: 440.0,
            lfo_wave: SirenWave::Sine,
            lfo_rate: 0.0,
            lfo_depth: 0.0,
            sweep_start_mult: 1.0,
            sweep_end_mult: 1.0,
            sweep_step: 1.0,
            sweep_frames: 0,
            trem_wave: SirenWave::Sine,
            trem_rate: 0.0,
            trem_depth: 0.0,
            noise_amount: 0.0,
            noise_onset_frames: 0,
            source_lp_coeff: one_pole_coeff(OPEN_LPF_HZ, SR),
            source_lp_end_coeff: one_pole_coeff(OPEN_LPF_HZ, SR),
            source_lp_step: 1.0,
            source_lp_sweep_frames: 0,
            attack_inc: ramp_inc(2.0, SR),
            release_coeff: decay_coeff(20.0, SR),
            delay_frames: 1,
            delay_feedback: 0.0,
            delay_mix: 0.0,
            delay_lp_coeff: 1.0,
            volume: 1.0,
            crush_hold: 1,
            crush_levels: 0.0,
            gate_frames: 0, // sustained unless overridden
        }
    }

    #[test]
    fn idle_is_an_additive_no_op() {
        let mut voice = SirenVoice::new(SR);
        let mut buf = [0.1f32, -0.2, 0.3, -0.4];
        let before = buf;
        voice.process_block(&mut buf, 2, 0);
        assert_eq!(buf, before);
        assert_eq!(voice.state(), SirenState::Idle);
    }

    #[test]
    fn engage_makes_sound_and_sets_state() {
        let mut voice = SirenVoice::new(SR);
        voice.engage(&bare_patch());
        assert_eq!(voice.state(), SirenState::Sounding);
        let peak = render(&mut voice, 4_000)
            .iter()
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.3, "no sound: {peak}");
    }

    #[test]
    fn output_is_additive_not_replacing() {
        let mk = || {
            let mut v = SirenVoice::new(SR);
            let mut p = bare_patch();
            p.base_freq = 330.0;
            v.engage(&p);
            v
        };
        let mut solo = mk();
        let mut over = mk();
        for i in 0..512 {
            let bias = (i as f32 * 0.001).sin();
            let mut a = [0.0f32, 0.0];
            let mut b = [bias, bias];
            solo.process_block(&mut a, 2, 0);
            over.process_block(&mut b, 2, 0);
            assert!((b[0] - (bias + a[0])).abs() < 1e-6, "not additive at {i}");
        }
    }

    #[test]
    fn oscillator_frequency_is_accurate_across_sample_rates() {
        for &sr in &[44_100.0f32, 96_000.0] {
            let mut voice = SirenVoice::new(sr);
            let mut p = bare_patch();
            p.base_freq = 1_000.0;
            p.source_lp_coeff = one_pole_coeff(OPEN_LPF_HZ, sr);
            p.attack_inc = ramp_inc(1.0, sr);
            voice.engage(&p);
            let _ = render(&mut voice, sr as usize / 100);
            let window = render(&mut voice, sr as usize / 2);
            let measured = count_zero_crossings(&window) as f32 / 2.0 / 0.5;
            assert!(
                (measured - 1_000.0).abs() < 6.0,
                "freq {measured} at sr {sr}"
            );
        }
    }

    #[test]
    fn pitch_sweep_glides_downward() {
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.base_freq = 300.0;
        p.sweep_start_mult = 8.0;
        p.sweep_end_mult = 0.25;
        p.sweep_frames = ms_to_frames(300.0, SR);
        p.sweep_step = (0.25f32 / 8.0).powf(1.0 / p.sweep_frames as f32);
        voice.engage(&p);
        let early = count_zero_crossings(&render(&mut voice, 1_000));
        let _ = render(&mut voice, SR as usize / 2);
        let late = count_zero_crossings(&render(&mut voice, 1_000));
        assert!(
            early > late * 3,
            "sweep did not fall: early {early} late {late}"
        );
    }

    #[test]
    fn tremolo_gates_amplitude() {
        // Square tremolo at 10 Hz, full depth: first half-cycle loud, second
        // half silent (square table: +1 then −1).
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.base_freq = 400.0;
        p.trem_wave = SirenWave::Square;
        p.trem_rate = 10.0;
        p.trem_depth = 1.0;
        p.attack_inc = ramp_inc(1.0, SR);
        voice.engage(&p);
        let out = render(&mut voice, SR as usize / 10);
        let on = out[200..2_000].iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        let off = out[2_600..4_600]
            .iter()
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(on > 0.3, "tremolo on-phase too quiet: {on}");
        assert!(off < 0.02, "tremolo off-phase not gated: {off}");
    }

    #[test]
    fn decay_tremolo_retriggers_percussive_shots() {
        // The Decay wave spikes then falls each cycle: at 10 Hz, the start of a
        // cycle is loud and the end is near-silent — a machine-gun shot rhythm.
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.noise_amount = 1.0;
        p.trem_wave = SirenWave::Decay;
        p.trem_rate = 10.0; // 100 ms / shot
        p.trem_depth = 1.0;
        voice.engage(&p);
        let out = render(&mut voice, SR as usize / 5); // two shots
        let shot = out[100..600].iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        let gap = out[4_200..4_700]
            .iter()
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(shot > 0.2, "shot transient too quiet: {shot}");
        assert!(
            gap < shot * 0.3,
            "no decay between shots: shot {shot} gap {gap}"
        );
    }

    #[test]
    fn source_low_pass_darkens_noise() {
        // The same noise sounds darker (fewer zero crossings) through a low
        // cutoff than wide open — that's what turns hiss into a boom.
        let bright = {
            let mut v = SirenVoice::new(SR);
            let mut p = bare_patch();
            p.noise_amount = 1.0;
            p.source_lp_coeff = one_pole_coeff(OPEN_LPF_HZ, SR);
            v.engage(&p);
            count_zero_crossings(&render(&mut v, 8_000))
        };
        let dark = {
            let mut v = SirenVoice::new(SR);
            let mut p = bare_patch();
            p.noise_amount = 1.0;
            // No sweep, so start == end (a steady dark filter).
            p.source_lp_coeff = one_pole_coeff(600.0, SR);
            p.source_lp_end_coeff = one_pole_coeff(600.0, SR);
            v.engage(&p);
            count_zero_crossings(&render(&mut v, 8_000))
        };
        assert!(
            dark * 2 < bright,
            "low-pass did not darken noise: bright {bright} dark {dark}"
        );
    }

    #[test]
    fn release_is_exponential_and_reaches_zero() {
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.release_coeff = decay_coeff(100.0, SR);
        voice.engage(&p);
        let _ = render(&mut voice, 4_000); // reach full
        let full = voice.env;
        voice.release();
        let _ = render(&mut voice, SR as usize / 20); // 50 ms in
        let mid = voice.env;
        assert!(
            mid < full && mid > 0.0,
            "not decaying: full {full} mid {mid}"
        );
        let _ = render(&mut voice, SR as usize / 2);
        assert!(
            voice.env <= 0.0,
            "release never reached zero: {}",
            voice.env
        );
    }

    #[test]
    fn no_click_on_engage() {
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.base_freq = 50.0;
        p.attack_inc = ramp_inc(5.0, SR);
        voice.engage(&p);
        let out = render(&mut voice, 256);
        assert!(out[0].abs() < 1e-3, "jump at gate-on: {}", out[0]);
        for w in out.windows(2) {
            assert!((w[1] - w[0]).abs() < 0.015, "discontinuity in attack");
        }
    }

    #[test]
    fn one_shot_auto_releases_and_goes_idle() {
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.gate_frames = 500;
        voice.engage(&p);
        let _ = render(&mut voice, 100);
        assert_eq!(voice.state(), SirenState::Sounding);
        let _ = render(&mut voice, SR as usize / 2);
        assert_eq!(voice.state(), SirenState::Idle, "one-shot never went idle");
    }

    #[test]
    fn all_presets_sound_then_go_idle() {
        for id in 0..SIREN_PRESET_COUNT {
            let mut voice = SirenVoice::new(SR);
            let patch = siren_preset_patch(id, SR);
            voice.engage(&patch);
            let mut peak = 0.0f32;
            for _ in 0..400 {
                let mut buf = [0.0f32, 0.0];
                voice.process_block(&mut buf, 2, 0);
                peak = peak.max(buf[0].abs());
            }
            assert!(
                peak > 0.05,
                "preset {id} ({}) silent",
                siren_preset_name(id)
            );
            for s in render(&mut voice, SR as usize * 14) {
                assert!(s.is_finite(), "preset {id} non-finite");
            }
            assert_eq!(
                voice.state(),
                SirenState::Idle,
                "preset {id} ({}) never went idle",
                siren_preset_name(id)
            );
        }
    }

    #[test]
    fn crush_decimates_and_quantizes_the_output() {
        // Sample-and-hold (rate crush) → adjacent samples repeat; bit-crush →
        // values snap to a small set of levels. That stair-stepping is the
        // "8-bit edgy" character.
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.crush_hold = 6; // hold each output for 6 frames
        p.crush_levels = 8.0; // ~4-bit
        voice.engage(&p);
        let out = render(&mut voice, 600);
        let repeats = out
            .windows(2)
            .filter(|w| (w[0] - w[1]).abs() < 1e-12)
            .count();
        assert!(
            repeats > 300,
            "rate-crush did not hold samples: {repeats} repeats"
        );
        // Every value must sit on the 1/8 quantization grid.
        for &s in &out {
            let q = (s * 8.0).round() / 8.0;
            assert!((s - q).abs() < 1e-6, "value {s} not quantized");
        }
    }

    #[test]
    fn noise_presets_are_audible() {
        // Regression: the bomb / machine-gun / lickshot are filtered-noise
        // sounds; without makeup gain the low-pass left them near-silent. Each
        // must reach a healthy peak somewhere in its life.
        for id in [3usize, 4, 5] {
            let mut voice = SirenVoice::new(SR);
            voice.engage(&siren_preset_patch(id, SR));
            let peak = render(&mut voice, SR as usize * 2)
                .iter()
                .fold(0.0f32, |m, &x| m.max(x.abs()));
            assert!(
                peak > 0.3,
                "preset {id} ({}) too quiet: peak {peak}",
                siren_preset_name(id)
            );
        }
    }

    #[test]
    fn preset_table_is_named_and_in_range() {
        assert_eq!(SIREN_PRESET_COUNT, 8);
        for id in 0..SIREN_PRESET_COUNT {
            assert!(!siren_preset_name(id).is_empty(), "preset {id} unnamed");
        }
        assert_eq!(siren_preset_name(99), "");
        let _ = siren_preset_patch(99, SR);
    }

    #[test]
    fn wavetables_have_the_right_shape() {
        let sine = build_table(SirenWave::Sine);
        let tri = build_table(SirenWave::Triangle);
        let sq = build_table(SirenWave::Square);
        let saw = build_table(SirenWave::Saw);
        let decay = build_table(SirenWave::Decay);
        assert!(sine[0].abs() < 1e-6 && (sine[TABLE_LEN / 4] - 1.0).abs() < 1e-3);
        assert!((tri[TABLE_LEN / 2] - 1.0).abs() < 1e-2 && (tri[0] + 1.0).abs() < 1e-2);
        assert!((sq[0] - 1.0).abs() < 1e-6 && (sq[TABLE_LEN - 1] + 1.0).abs() < 1e-6);
        assert!((saw[0] + 1.0).abs() < 1e-6 && saw[TABLE_LEN - 1] > 0.99);
        // Decay: starts at +1, ends near −1, monotonically falling.
        assert!((decay[0] - 1.0).abs() < 1e-3 && decay[TABLE_LEN - 1] < -0.9);
        assert!(decay[TABLE_LEN / 2] < decay[0]);
    }

    #[test]
    fn noise_layer_is_broadband_and_gated_by_onset() {
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.noise_amount = 1.0;
        voice.engage(&p);
        let crossings = count_zero_crossings(&render(&mut voice, 4_000));
        assert!(
            crossings > 500,
            "noise not broadband: {crossings} crossings"
        );

        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.noise_amount = 1.0;
        p.noise_onset_frames = 4_800; // 100 ms
        voice.engage(&p);
        let out = render(&mut voice, 9_600);
        let late = out[5_000..9_000]
            .iter()
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(late > 0.3, "noise burst too quiet after onset: {late}");
    }

    #[test]
    fn extreme_values_stay_finite_and_bounded() {
        let mut voice = SirenVoice::new(SR);
        let mut p = bare_patch();
        p.base_freq = 100.0;
        p.lfo_wave = SirenWave::Saw;
        p.lfo_rate = 30.0;
        p.lfo_depth = 1.0e6;
        p.noise_amount = 1.0;
        p.sweep_start_mult = 1.0e6;
        p.sweep_end_mult = 1.0e-6;
        p.sweep_step = 0.99;
        p.sweep_frames = 5_000;
        voice.engage(&p);
        for s in render(&mut voice, 8_000) {
            assert!(
                s.is_finite() && s.abs() <= MAX_VOLUME + 1e-3,
                "out of bounds: {s}"
            );
        }
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut voice = SirenVoice::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        let bomb = siren_preset_patch(3, SR);
        let gun = siren_preset_patch(4, SR);
        assert_no_alloc::assert_no_alloc(|| {
            voice.engage(&bomb);
            for _ in 0..64 {
                voice.process_block(&mut buf, 4, 2);
            }
            voice.engage(&gun);
            for _ in 0..64 {
                voice.process_block(&mut buf, 4, 2);
            }
            voice.release();
            for _ in 0..256 {
                voice.process_block(&mut buf, 4, 2);
            }
        });
    }

    #[test]
    fn siren_wave_round_trips_through_u8() {
        for w in [
            SirenWave::Sine,
            SirenWave::Triangle,
            SirenWave::Square,
            SirenWave::Saw,
            SirenWave::Decay,
        ] {
            assert_eq!(SirenWave::from_u8(w.code()), w);
        }
    }
}
