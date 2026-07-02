//! A behavioural model of the **Texas Instruments SN76477** "complex sound
//! generator" (1978) — the chip behind countless 70s/80s arcade + toy space
//! guns, lasers, explosions and sirens (Space Invaders, etc.), and a staple of
//! the dub / sound-system FX box.
//!
//! This is the *faithful* dub-siren engine (PRD §6.3 / Expert-mode groundwork):
//! it is built from the same functional blocks as the chip, so it sounds like
//! the chip — crude hard-square edges and 1-bit LFSR noise, with only the
//! chip's single-pole noise filter, **no** hi-fi band-limiting or smoothing.
//! That rawness is the point; our earlier generic voice sounded "round" because
//! it was band-limited and filtered.
//!
//! Modelled after MAME's `sn76477.cpp` behaviour (not a transistor/SPICE
//! model). Blocks:
//!
//! - **SLF** — super-low-frequency oscillator: a triangle (the VCO pitch
//!   control source → sirens) plus a square.
//! - **VCO** — a hard square with a voltage-controlled duty cycle; its pitch
//!   tracks a control voltage from the SLF (siren wail) or a one-shot pitch
//!   glide (laser zap / bomb whistle).
//! - **Noise** — a 17-bit LFSR clocked at the noise-clock rate and
//!   sample-and-held (the crunchy 1-bit grain), through a one-pole noise filter.
//! - **Mixer** — selects VCO / SLF / noise / combinations.
//! - **One-shot + envelope** — a fixed gate and an RC attack/decay; the
//!   envelope can be retriggered by the SLF for the machine-gun rhythm.
//!
//! ## Real-time safety
//!
//! `new` allocates nothing of note; `trigger`, `release` and `process_block`
//! are integer + float math over inline state — no allocation, locks, syscalls
//! or transcendentals on the audio thread (coefficients are resolved off-RT).
//! Verified under `assert_no_alloc`.

use crate::echo::one_pole_coeff;

/// Lowest VCO frequency the model will emit (Hz).
const MIN_VCO_FREQ: f32 = 20.0;

/// Below this envelope level the voice is silent.
const ENV_FLOOR: f32 = 1.0e-4;

/// −60 dB reference for the exponential attack/decay shaping.
const DECAY_TARGET: f32 = 0.001;

/// Which blocks the mixer routes to the output (the chip's 3-bit mixer select).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerMode {
    /// VCO only — tones, sirens, lasers.
    Vco,
    /// Noise only — explosions, gunshots.
    Noise,
    /// VCO + noise — gritty tones.
    VcoNoise,
    /// SLF square — slow pulsing.
    Slf,
}

/// Source of the VCO's pitch control voltage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CvSource {
    /// Held at the top of the range (fixed pitch).
    Fixed,
    /// The SLF triangle sweeps the pitch up and down (siren wail / UFO).
    Slf,
    /// A one-shot glide from the top of the range to the bottom (laser / bomb).
    SweepDown,
    /// A one-shot glide from the bottom of the range to the top (riser).
    SweepUp,
}

/// A fully resolved SN76477 patch (engine-domain; coefficients/increments are
/// computed off-RT by [`sn76477_preset`]). Held in the engine and handed to
/// [`Sn76477::trigger`] by reference, so its size is free.
#[derive(Debug, Clone, Copy)]
pub struct Sn76477Patch {
    /// Which blocks the mixer routes to the output.
    pub mixer: MixerMode,
    /// Source of the VCO pitch control voltage.
    pub cv_source: CvSource,
    /// VCO pitch range floor (Hz); the CV sweeps up from here.
    pub vco_min_freq: f32,
    /// VCO pitch range ceiling (Hz); the CV sweeps up to here.
    pub vco_max_freq: f32,
    /// VCO duty cycle (0..1); 0.5 = square, lower = thinner pulse.
    pub vco_duty: f32,
    /// One-shot pitch-glide per-sample multiplier for `SweepDown` / `SweepUp`.
    pub pitch_step: f32,
    /// Pitch-glide length in frames.
    pub pitch_frames: u32,
    /// SLF rate (Hz) — pitch wobble and the machine-gun retrigger clock.
    pub slf_freq: f32,
    /// Noise LFSR clock (Hz) — lower = coarser / crunchier grain.
    pub noise_clock_freq: f32,
    /// Noise one-pole filter coefficient (the chip's single-RC noise filter).
    pub noise_lp_coeff: f32,
    /// Envelope attack charge coefficient (per sample, exponential).
    pub attack_coeff: f32,
    /// Envelope decay coefficient (per sample, exponential).
    pub decay_coeff: f32,
    /// One-shot gate length in frames (0 = sustained until `release`).
    pub gate_frames: u32,
    /// Retrigger the gate every SLF cycle — the machine-gun rhythm.
    pub retrigger_from_slf: bool,
    /// When a `SweepDown` pitch glide finishes, switch the mixer to noise and
    /// re-strike the envelope — the bomb's whistle-then-explosion.
    pub explode_after_sweep: bool,
    /// Output level.
    pub volume: f32,
}

/// Audible state (mirrors [`crate::siren::SirenState`] codes: 0 idle, 1 sounding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sn76477State {
    /// Silent.
    Idle,
    /// Making sound.
    Sounding,
}

impl Sn76477State {
    /// Stable wire value (`0` idle · `1` sounding).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Sn76477State::Idle => 0,
            Sn76477State::Sounding => 1,
        }
    }
}

/// One per-deck SN76477 voice. Like [`crate::siren::SirenVoice`] it is an
/// additive generator: [`Sn76477::process_block`] sums into the deck's output
/// pair.
#[derive(Debug)]
pub struct Sn76477 {
    sample_rate: f32,

    mixer: MixerMode,
    cv_source: CvSource,
    vco_min_freq: f32,
    vco_max_freq: f32,
    vco_duty: f32,
    vco_phase: f32,

    // One-shot pitch glide (for SweepUp/Down).
    pitch_mult: f32,
    pitch_step: f32,
    pitch_frames_left: u32,

    slf_freq: f32,
    slf_phase: f32,

    // Noise: 17-bit LFSR, clocked at `noise_clock_freq` (sample-and-held).
    lfsr: u32,
    noise_value: f32,
    noise_clock_period: f32,
    noise_clock_acc: f32,
    noise_lp_coeff: f32,
    noise_lp: f32,

    // One-shot + envelope.
    env: f32,
    env_target: f32,
    attack_coeff: f32,
    decay_coeff: f32,
    gate_active: bool,
    one_shot: bool,
    gate_frames: u32,
    gate_frames_left: u32,
    retrigger_from_slf: bool,
    prev_slf_high: bool,
    explode_after_sweep: bool,
    exploded: bool,

    volume: f32,
    state: Sn76477State,
    quiet_frames: usize,
    ready_frames: usize,
}

impl Sn76477 {
    /// Allocate a voice for an output bus at `sample_rate`. Not RT-critical
    /// (no heap), but call at setup.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ready_frames = (sample_rate * 0.05).max(1.0) as usize;
        Self {
            sample_rate,
            mixer: MixerMode::Vco,
            cv_source: CvSource::Fixed,
            vco_min_freq: 200.0,
            vco_max_freq: 800.0,
            vco_duty: 0.5,
            vco_phase: 0.0,
            pitch_mult: 1.0,
            pitch_step: 1.0,
            pitch_frames_left: 0,
            slf_freq: 5.0,
            slf_phase: 0.0,
            lfsr: 0x1_ace1,
            noise_value: 0.0,
            noise_clock_period: 1.0,
            noise_clock_acc: 0.0,
            noise_lp_coeff: 1.0,
            noise_lp: 0.0,
            env: 0.0,
            env_target: 0.0,
            attack_coeff: 0.01,
            decay_coeff: 0.99,
            gate_active: false,
            one_shot: false,
            gate_frames: 0,
            gate_frames_left: 0,
            retrigger_from_slf: false,
            prev_slf_high: false,
            explode_after_sweep: false,
            exploded: false,
            volume: 0.0,
            state: Sn76477State::Idle,
            quiet_frames: 0,
            ready_frames,
        }
    }

    /// Trigger the voice with a resolved patch. Resets the oscillators + glide
    /// so each shot is deterministic; the attack masks the reset.
    pub fn trigger(&mut self, patch: &Sn76477Patch) {
        self.mixer = patch.mixer;
        self.cv_source = patch.cv_source;
        self.vco_min_freq = patch.vco_min_freq.max(MIN_VCO_FREQ);
        self.vco_max_freq = patch.vco_max_freq.max(self.vco_min_freq);
        self.vco_duty = patch.vco_duty.clamp(0.02, 0.98);
        self.vco_phase = 0.0;

        self.pitch_step = if patch.pitch_step.is_finite() && patch.pitch_step > 0.0 {
            patch.pitch_step
        } else {
            1.0
        };
        self.pitch_frames_left = patch.pitch_frames;
        self.pitch_mult = match patch.cv_source {
            CvSource::SweepUp => 0.0,
            _ => 1.0,
        };

        self.slf_freq = patch.slf_freq.clamp(0.05, 200.0);
        self.slf_phase = 0.0;

        self.noise_clock_period =
            (self.sample_rate / patch.noise_clock_freq.clamp(200.0, self.sample_rate)).max(1.0);
        self.noise_clock_acc = 0.0;
        self.noise_lp_coeff = patch.noise_lp_coeff.clamp(0.0, 1.0);
        self.noise_lp = 0.0;

        self.attack_coeff = patch.attack_coeff.clamp(1.0e-5, 1.0);
        self.decay_coeff = patch.decay_coeff.clamp(0.0, 0.999_99);
        self.volume = patch.volume.clamp(0.0, 2.0);

        self.env = 0.0;
        self.env_target = 1.0;
        self.gate_active = true;
        self.one_shot = patch.gate_frames > 0;
        self.gate_frames = patch.gate_frames.max(1);
        self.gate_frames_left = patch.gate_frames;
        self.retrigger_from_slf = patch.retrigger_from_slf;
        self.prev_slf_high = false;
        self.explode_after_sweep = patch.explode_after_sweep;
        self.exploded = false;

        self.state = Sn76477State::Sounding;
        self.quiet_frames = 0;
    }

    /// Stop a sustained voice (envelope decays out).
    pub fn release(&mut self) {
        self.env_target = 0.0;
        self.gate_active = false;
    }

    /// Current audible state.
    #[must_use]
    pub fn state(&self) -> Sn76477State {
        self.state
    }

    /// Wire value for the UI indicator (`0` idle · `1` sounding).
    #[must_use]
    pub fn state_code(&self) -> u8 {
        self.state.code()
    }

    /// Process one stereo block, summing into `out[f·stride + offset ..][..2]`.
    /// RT-safe: bounded loop, integer LFSR, no allocation.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);
        if self.state == Sn76477State::Idle && self.env <= 0.0 {
            return;
        }

        let slf_inc = self.slf_freq / self.sample_rate;

        for frame in out.chunks_exact_mut(stride) {
            // SLF triangle (0..1) + square; drives siren CV and the MG retrigger.
            let slf_tri = if self.slf_phase < 0.5 {
                self.slf_phase * 2.0
            } else {
                2.0 - self.slf_phase * 2.0
            };
            let slf_high = self.slf_phase < 0.5;

            // Machine-gun retrigger: restart the gate on each SLF rising edge.
            if self.retrigger_from_slf && slf_high && !self.prev_slf_high {
                self.env = 0.0;
                self.env_target = 1.0;
                self.gate_active = true;
                self.gate_frames_left = self.gate_frames;
            }
            self.prev_slf_high = slf_high;

            // One-shot countdown.
            if self.one_shot && self.gate_active {
                if self.gate_frames_left == 0 {
                    self.gate_active = false;
                    self.env_target = 0.0;
                } else {
                    self.gate_frames_left -= 1;
                }
            }

            // Envelope: exponential attack (charge) then decay.
            if self.env < self.env_target {
                self.env += self.attack_coeff * (self.env_target - self.env);
            } else if self.env > self.env_target {
                self.env *= self.decay_coeff;
                if self.env <= ENV_FLOOR {
                    self.env = self.env_target;
                }
            }

            // One-shot pitch glide for the laser/bomb/riser CV.
            if self.pitch_frames_left > 0 {
                self.pitch_mult *= self.pitch_step;
                self.pitch_frames_left -= 1;
                // Bomb: when the whistle's glide finishes, switch to a noise
                // explosion and re-strike the envelope so it booms + decays.
                if self.pitch_frames_left == 0 && self.explode_after_sweep && !self.exploded {
                    self.exploded = true;
                    self.mixer = MixerMode::Noise;
                    self.env = 1.0;
                    self.env_target = 0.0;
                    self.gate_active = false;
                    self.one_shot = false;
                }
            }

            // VCO control voltage in [0, 1].
            let cv = match self.cv_source {
                CvSource::Fixed => 1.0,
                CvSource::Slf => slf_tri,
                CvSource::SweepDown => self.pitch_mult.clamp(0.0, 1.0),
                CvSource::SweepUp => self.pitch_mult.clamp(0.0, 1.0),
            };
            let vco_freq = (self.vco_min_freq + (self.vco_max_freq - self.vco_min_freq) * cv)
                .max(MIN_VCO_FREQ);

            // Hard-square VCO with duty (the crude edge — no band-limiting).
            let vco_out = if self.vco_phase < self.vco_duty {
                1.0
            } else {
                -1.0
            };

            // Noise: clock the LFSR at its rate, sample-and-hold between clocks,
            // then the one-pole noise filter. Always run (the mixer can switch
            // to noise mid-sound for the bomb explosion).
            self.noise_clock_acc += 1.0;
            if self.noise_clock_acc >= self.noise_clock_period {
                self.noise_clock_acc -= self.noise_clock_period;
                // 17-bit maximal LFSR (taps 17, 14).
                let bit = ((self.lfsr >> 16) ^ (self.lfsr >> 13)) & 1;
                self.lfsr = ((self.lfsr << 1) | bit) & 0x1_ffff;
                self.noise_value = if self.lfsr & 1 == 1 { 1.0 } else { -1.0 };
            }
            self.noise_lp += self.noise_lp_coeff * (self.noise_value - self.noise_lp);

            // Mixer.
            let mixed = match self.mixer {
                MixerMode::Vco => vco_out,
                MixerMode::Noise => self.noise_lp,
                MixerMode::VcoNoise => 0.5 * vco_out + 0.7 * self.noise_lp,
                MixerMode::Slf => {
                    if slf_high {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };

            let s = mixed * self.env * self.volume;

            // Advance phases.
            self.vco_phase += vco_freq / self.sample_rate;
            if self.vco_phase >= 1.0 {
                self.vco_phase -= 1.0;
            }
            self.slf_phase += slf_inc;
            if self.slf_phase >= 1.0 {
                self.slf_phase -= 1.0;
            }

            // Self-terminate once released + silent (UI pad off). Retriggering
            // voices keep sounding.
            if !self.gate_active && !self.retrigger_from_slf && self.env <= ENV_FLOOR {
                self.quiet_frames = self.quiet_frames.saturating_add(1);
                if self.quiet_frames >= self.ready_frames {
                    self.state = Sn76477State::Idle;
                }
            } else {
                self.quiet_frames = 0;
            }

            frame[offset] += s;
            frame[offset + 1] += s;
        }
    }
}

/// Number of built-in SN76477 sounds.
pub const SN76477_PRESET_COUNT: usize = SN76477_PRESETS.len();

/// Resolve SN76477 preset `id` at `sample_rate` (off-RT — `exp`/`powf`).
#[must_use]
pub fn sn76477_preset(id: usize, sample_rate: f32) -> Sn76477Patch {
    let spec = SN76477_PRESETS
        .get(id)
        .copied()
        .unwrap_or(SN76477_PRESETS[0]);
    spec.resolve(sample_rate)
}

/// Display name for SN76477 preset `id`.
#[must_use]
pub fn sn76477_preset_name(id: usize) -> &'static str {
    SN76477_PRESETS.get(id).map_or("", |s| s.name)
}

/// Musical (human-unit) SN76477 preset, resolved to an [`Sn76477Patch`].
#[derive(Debug, Clone, Copy)]
struct Sn76477Spec {
    name: &'static str,
    mixer: MixerMode,
    cv_source: CvSource,
    vco_min_freq: f32,
    vco_max_freq: f32,
    vco_duty: f32,
    /// Pitch-glide length (ms) for SweepUp/Down.
    pitch_ms: f32,
    slf_freq: f32,
    noise_clock_freq: f32,
    noise_filter_hz: f32,
    attack_ms: f32,
    decay_ms: f32,
    gate_ms: f32,
    retrigger_from_slf: bool,
    explode_after_sweep: bool,
    volume: f32,
}

impl Sn76477Spec {
    fn resolve(self, sr: f32) -> Sn76477Patch {
        let pitch_frames = ms_to_frames(self.pitch_ms, sr);
        // Glide spans the full [min,max] CV range (0..1) over `pitch_ms`.
        let pitch_step = match self.cv_source {
            CvSource::SweepDown if pitch_frames > 0 => {
                // 1 → ~0 exponentially.
                DECAY_TARGET.powf(1.0 / pitch_frames as f32)
            }
            CvSource::SweepUp if pitch_frames > 0 => {
                // 0→1 handled by climbing toward 1: model as mult growth from a
                // small seed; use the inverse of a decay so it rises.
                (1.0 / DECAY_TARGET).powf(1.0 / pitch_frames as f32)
            }
            _ => 1.0,
        };
        Sn76477Patch {
            mixer: self.mixer,
            cv_source: self.cv_source,
            vco_min_freq: self.vco_min_freq,
            vco_max_freq: self.vco_max_freq,
            vco_duty: self.vco_duty,
            pitch_step,
            pitch_frames,
            slf_freq: self.slf_freq,
            noise_clock_freq: self.noise_clock_freq,
            noise_lp_coeff: one_pole_coeff(self.noise_filter_hz, sr),
            attack_coeff: charge_coeff(self.attack_ms, sr),
            decay_coeff: decay_coeff(self.decay_ms, sr),
            gate_frames: ms_to_frames(self.gate_ms, sr).max(1),
            retrigger_from_slf: self.retrigger_from_slf,
            explode_after_sweep: self.explode_after_sweep,
            volume: self.volume,
        }
    }
}

/// The classic SN76477 sounds (datasheet application circuits). Indices are
/// referenced by `siren::siren_preset_route`, so keep them stable.
const SN76477_PRESETS: [Sn76477Spec; 7] = [
    // 0 — Phasor / laser: VCO zapped down in pitch.
    Sn76477Spec {
        name: "Laser",
        mixer: MixerMode::Vco,
        cv_source: CvSource::SweepDown,
        vco_min_freq: 120.0,
        vco_max_freq: 2_400.0,
        vco_duty: 0.5,
        pitch_ms: 240.0,
        slf_freq: 5.0,
        noise_clock_freq: 20_000.0,
        noise_filter_hz: 8_000.0,
        attack_ms: 1.0,
        decay_ms: 120.0,
        gate_ms: 280.0,
        retrigger_from_slf: false,
        explode_after_sweep: false,
        volume: 0.7,
    },
    // 1 — Explosion: filtered noise, slow decay, no tone.
    Sn76477Spec {
        name: "Explosion",
        mixer: MixerMode::Noise,
        cv_source: CvSource::Fixed,
        vco_min_freq: 200.0,
        vco_max_freq: 200.0,
        vco_duty: 0.5,
        pitch_ms: 0.0,
        slf_freq: 5.0,
        noise_clock_freq: 6_000.0,
        noise_filter_hz: 1_400.0,
        attack_ms: 2.0,
        decay_ms: 900.0,
        gate_ms: 40.0,
        retrigger_from_slf: false,
        explode_after_sweep: false,
        volume: 0.9,
    },
    // 2 — Bomb drop: a falling VCO whistle that explodes into noise at the
    // bottom (whistle → boom in one trigger).
    Sn76477Spec {
        name: "Bomb",
        mixer: MixerMode::Vco,
        cv_source: CvSource::SweepDown,
        vco_min_freq: 80.0,
        vco_max_freq: 1_200.0,
        vco_duty: 0.5,
        pitch_ms: 700.0,
        slf_freq: 5.0,
        noise_clock_freq: 6_000.0,
        noise_filter_hz: 700.0,
        attack_ms: 6.0,
        decay_ms: 700.0,
        gate_ms: 720.0,
        retrigger_from_slf: false,
        explode_after_sweep: true,
        volume: 0.8,
    },
    // 3 — Machine gun: filtered-noise gunshot retriggered by the SLF.
    Sn76477Spec {
        name: "Machine Gun",
        mixer: MixerMode::Noise,
        cv_source: CvSource::Fixed,
        vco_min_freq: 200.0,
        vco_max_freq: 200.0,
        vco_duty: 0.5,
        pitch_ms: 0.0,
        slf_freq: 11.0,
        noise_clock_freq: 8_000.0,
        noise_filter_hz: 2_000.0,
        attack_ms: 0.5,
        decay_ms: 55.0,
        gate_ms: 35.0,
        retrigger_from_slf: true,
        explode_after_sweep: false,
        volume: 0.8,
    },
    // 4 — Siren: VCO wailing up/down under the SLF triangle.
    Sn76477Spec {
        name: "Siren",
        mixer: MixerMode::Vco,
        cv_source: CvSource::Slf,
        vco_min_freq: 440.0,
        vco_max_freq: 1_050.0,
        vco_duty: 0.5,
        pitch_ms: 0.0,
        slf_freq: 4.5,
        noise_clock_freq: 20_000.0,
        noise_filter_hz: 8_000.0,
        attack_ms: 8.0,
        decay_ms: 200.0,
        gate_ms: 1_600.0,
        retrigger_from_slf: false,
        explode_after_sweep: false,
        volume: 0.55,
    },
    // 5 — UFO: a fast VCO wobble under the SLF.
    Sn76477Spec {
        name: "UFO",
        mixer: MixerMode::Vco,
        cv_source: CvSource::Slf,
        vco_min_freq: 500.0,
        vco_max_freq: 1_400.0,
        vco_duty: 0.3,
        pitch_ms: 0.0,
        slf_freq: 11.0,
        noise_clock_freq: 20_000.0,
        noise_filter_hz: 8_000.0,
        attack_ms: 8.0,
        decay_ms: 200.0,
        gate_ms: 1_800.0,
        retrigger_from_slf: false,
        explode_after_sweep: false,
        volume: 0.5,
    },
    // 6 — Gunshot / lickshot: a single short filtered-noise crack.
    Sn76477Spec {
        name: "Gunshot",
        mixer: MixerMode::Noise,
        cv_source: CvSource::Fixed,
        vco_min_freq: 200.0,
        vco_max_freq: 200.0,
        vco_duty: 0.5,
        pitch_ms: 0.0,
        slf_freq: 5.0,
        noise_clock_freq: 8_000.0,
        noise_filter_hz: 2_500.0,
        attack_ms: 0.5,
        decay_ms: 160.0,
        gate_ms: 30.0,
        retrigger_from_slf: false,
        explode_after_sweep: false,
        volume: 0.9,
    },
];

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ms_to_frames(millis: f32, sample_rate: f32) -> u32 {
    (sample_rate * millis / 1000.0).round().max(0.0) as u32
}

/// Per-sample exponential charge coefficient reaching ~99.9% in `millis`.
fn charge_coeff(millis: f32, sample_rate: f32) -> f32 {
    1.0 - decay_coeff(millis, sample_rate)
}

/// Per-sample exponential decay multiplier reaching −60 dB over `millis`.
fn decay_coeff(millis: f32, sample_rate: f32) -> f32 {
    let frames = (sample_rate * millis / 1000.0).max(1.0);
    DECAY_TARGET.powf(1.0 / frames).clamp(0.0, 0.999_99)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(voice: &mut Sn76477, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let mut buf = [0.0f32, 0.0f32];
            voice.process_block(&mut buf, 2, 0);
            out.push(buf[0]);
        }
        out
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    fn count_zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
            .count()
    }

    #[test]
    fn idle_is_additive_no_op() {
        let mut v = Sn76477::new(SR);
        let mut buf = [0.2f32, -0.3, 0.4, -0.1];
        let before = buf;
        v.process_block(&mut buf, 2, 0);
        assert_eq!(buf, before);
    }

    #[test]
    fn every_preset_sounds_then_idles() {
        for id in 0..SN76477_PRESET_COUNT {
            let mut v = Sn76477::new(SR);
            v.trigger(&sn76477_preset(id, SR));
            let early = render(&mut v, SR as usize / 4);
            assert!(
                peak(&early) > 0.1,
                "preset {id} ({}) silent",
                sn76477_preset_name(id)
            );
            for s in render(&mut v, SR as usize * 6) {
                assert!(s.is_finite(), "preset {id} non-finite");
            }
            // Retriggering voices (machine gun) sustain; the rest must idle.
            if !sn76477_preset(id, SR).retrigger_from_slf {
                assert_eq!(
                    v.state(),
                    Sn76477State::Idle,
                    "preset {id} ({}) never idled",
                    sn76477_preset_name(id)
                );
            }
        }
    }

    #[test]
    fn vco_is_a_hard_square() {
        // A fixed-pitch VCO output should be (near) ±something — only a few
        // distinct magnitudes, i.e. edgy, not a smooth sine.
        let mut v = Sn76477::new(SR);
        let mut p = sn76477_preset(4, SR); // Siren
        p.cv_source = CvSource::Fixed;
        p.mixer = MixerMode::Vco;
        v.trigger(&p);
        let _ = render(&mut v, 2_000); // pass the attack
        let out = render(&mut v, 400);
        // Hard square: most samples sit near +v or −v (few in-between).
        let vmax = peak(&out);
        let near_rail = out.iter().filter(|&&x| x.abs() > 0.6 * vmax).count();
        assert!(
            near_rail > 360,
            "VCO not a hard square: {near_rail}/400 at rail"
        );
    }

    #[test]
    fn laser_pitch_falls() {
        let mut v = Sn76477::new(SR);
        v.trigger(&sn76477_preset(0, SR)); // Laser, SweepDown
        let early = count_zero_crossings(&render(&mut v, 1_500));
        let late = count_zero_crossings(&render(&mut v, 1_500));
        assert!(
            early > late * 2,
            "laser pitch did not fall: {early} -> {late}"
        );
    }

    #[test]
    fn noise_is_broadband() {
        // The explosion is a noise source (low-passed): far more zero crossings
        // than a pure tone of the same length, but not a tone.
        let mut v = Sn76477::new(SR);
        v.trigger(&sn76477_preset(1, SR)); // Explosion (noise)
        let crossings = count_zero_crossings(&render(&mut v, 8_000));
        assert!(
            crossings > 150,
            "noise not broadband: {crossings} crossings"
        );
    }

    #[test]
    fn machine_gun_pulses() {
        // Retriggered envelope → loud bursts separated by quiet gaps.
        let mut v = Sn76477::new(SR);
        v.trigger(&sn76477_preset(3, SR)); // Machine Gun, slf 11 Hz
        let out = render(&mut v, SR as usize / 2);
        // Window the RMS; there should be both loud and near-silent windows.
        let win = 256;
        let mut loud = false;
        let mut quiet = false;
        for chunk in out.chunks(win) {
            let p = peak(chunk);
            if p > 0.2 {
                loud = true;
            }
            if p < 0.02 {
                quiet = true;
            }
        }
        assert!(
            loud && quiet,
            "machine gun not pulsing (loud {loud} quiet {quiet})"
        );
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut v = Sn76477::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        let laser = sn76477_preset(0, SR);
        let gun = sn76477_preset(3, SR);
        assert_no_alloc::assert_no_alloc(|| {
            v.trigger(&laser);
            for _ in 0..64 {
                v.process_block(&mut buf, 4, 2);
            }
            v.trigger(&gun);
            for _ in 0..256 {
                v.process_block(&mut buf, 4, 2);
            }
            v.release();
            for _ in 0..64 {
                v.process_block(&mut buf, 4, 2);
            }
        });
    }
}
