//! A recreation of the **Honsitak HK628** "sound effect IC" (1 trigger, 8
//! sounds) — the crude 80s keychain / toy / car-alarm chip: rifle, alarm,
//! dual-tone, bombs and electric (laser) guns. The sibling **HK623** is nearly
//! identical and can be added as a variant later.
//!
//! Unlike the analog SN76477 ([`crate::sn76477`]), the HK628 is a **digital ROM
//! sound-effect player**: a fixed 128 kHz clock steps a hard 1-bit output
//! through stored sequences. That is why it sounds "8-bit edgy", not round.
//!
//! We can't dump its ROM, so each sound is **recreated as a measured
//! step-sequence**: a list of `(pitch, level, duration)` steps captured from a
//! reference recording (`docs` / scratch analysis of `hk628.mp3`). A
//! [`Hk628`] voice plays a program as a hard square (or 1-bit LFSR noise per
//! step), snapping between steps — the crude stepped character of the chip —
//! with a light bit-crush and an optional slap-back (the "rifle echo").
//!
//! ## Real-time safety
//!
//! The slap-back ring is allocated in [`Hk628::new`]; programs are `&'static`
//! const data. `trigger` / `release` / `process_block` are integer + float math
//! over inline state — no allocation, locks, syscalls or transcendentals on the
//! audio thread. Verified under `assert_no_alloc`.

/// One step of an HK628 sound program.
#[derive(Debug, Clone, Copy)]
pub struct Hk628Step {
    /// Square-wave pitch in Hz; `<= 0` means a noise (LFSR) step.
    pub pitch: f32,
    /// Step level (`0..1`); rests use a low value.
    pub level: f32,
    /// Step duration in milliseconds.
    pub dur_ms: f32,
}

/// A named HK628 sound = a sequence of steps plus playback options.
#[derive(Debug, Clone, Copy)]
pub struct Hk628Program {
    /// Display name (one of the chip's 8 sounds).
    pub name: &'static str,
    /// The step sequence.
    pub steps: &'static [Hk628Step],
    /// Loop the steps until `total_ms` (true) or play once (false).
    pub repeat: bool,
    /// Total sound length in ms (only used when `repeat`).
    pub total_ms: f32,
    /// Apply the slap-back echo (the "rifle gun (echo)" sound).
    pub echo: bool,
    /// Output level.
    pub volume: f32,
}

/// Audible state (0 idle / 1 sounding, matching the other voices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hk628State {
    /// Silent.
    Idle,
    /// Making sound.
    Sounding,
}

impl Hk628State {
    /// Stable wire value (`0` idle · `1` sounding).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Hk628State::Idle => 0,
            Hk628State::Sounding => 1,
        }
    }
}

const DENORMAL_FLOOR: f32 = 1.0e-20;
const ENV_FLOOR: f32 = 1.0e-4;

/// Master output trim for the chip voice. The raw 1-bit square output sits near
/// full-scale (RMS ≈ peak ≈ 0.7–1.0, i.e. ~+0 to +5 LUFS), which is ~16 dB
/// hotter than the −14 LUFS the decoded tracks are normalised to. This trim
/// (≈ −16.5 dB) drops the bank to roughly −14 LUFS so the siren sits in the mix
/// like the music instead of blasting over it, and stops the louder programs
/// (e.g. Rifle Echo) clipping. Tunable by ear.
const OUTPUT_GAIN: f32 = 0.15;

/// One per-deck HK628 voice. Additive generator: [`Hk628::process_block`] sums
/// into the deck's output pair.
#[derive(Debug)]
pub struct Hk628 {
    sample_rate: f32,

    steps: &'static [Hk628Step],
    repeat: bool,
    step_idx: usize,
    step_frames_left: u32,
    cur_pitch: f32,
    cur_level: f32,
    playing: bool,

    sq_phase: f32,

    lfsr: u32,
    noise_value: f32,
    noise_div: u32,
    noise_acc: u32,

    env: f32,
    env_target: f32,
    attack_inc: f32,
    release_inc: f32,
    total_frames_left: u32,

    delay: Box<[f32]>,
    delay_mask: usize,
    delay_write: usize,
    delay_frames: usize,
    delay_feedback: f32,
    delay_mix: f32,
    echo_on: bool,
    delay_lp: f32,

    crush_levels: f32,
    crush_hold: u32,
    crush_counter: u32,
    crush_held: f32,

    volume: f32,
    /// Playback-rate multiplier (the GS1 "Speed" knob): scales step durations,
    /// so >1 wails faster, <1 slower. 1.0 = the program's native timing.
    speed: f32,
    state: Hk628State,
    quiet_frames: usize,
    ready_frames: usize,
}

impl Hk628 {
    /// Allocate a voice for an output bus at `sample_rate`. Not RT-critical;
    /// call at setup (allocates the echo ring).
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cap = (sample_rate * 0.3).ceil() as usize;
        let cap = cap.next_power_of_two().max(1_024);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ready_frames = (sample_rate * 0.05).max(1.0) as usize;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let noise_div = (sample_rate / 12_000.0).round().max(1.0) as u32;
        Self {
            sample_rate,
            steps: &[],
            repeat: false,
            step_idx: 0,
            step_frames_left: 0,
            cur_pitch: 0.0,
            cur_level: 0.0,
            playing: false,
            sq_phase: 0.0,
            lfsr: 0x1_ace1,
            noise_value: 0.0,
            noise_div,
            noise_acc: 0,
            env: 0.0,
            env_target: 0.0,
            attack_inc: ramp_inc(2.0, sample_rate),
            release_inc: ramp_inc(10.0, sample_rate),
            total_frames_left: 0,
            delay: vec![0.0; cap].into_boxed_slice(),
            delay_mask: cap - 1,
            delay_write: 0,
            delay_frames: (sample_rate * 0.09) as usize,
            delay_feedback: 0.4,
            delay_mix: 0.4,
            echo_on: false,
            delay_lp: 0.0,
            crush_levels: 32.0,
            crush_hold: noise_div.max(1),
            crush_counter: 0,
            crush_held: 0.0,
            volume: 0.0,
            speed: 1.0,
            state: Hk628State::Idle,
            quiet_frames: 0,
            ready_frames,
        }
    }

    /// Trigger a program (a `&'static` const sound). Resets playback so each
    /// trigger is deterministic.
    pub fn trigger(&mut self, prog: &'static Hk628Program) {
        self.steps = prog.steps;
        self.repeat = prog.repeat;
        self.echo_on = prog.echo;
        self.volume = prog.volume.clamp(0.0, 2.0);
        self.step_idx = 0;
        self.playing = !prog.steps.is_empty();
        self.load_step(0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let total = (self.sample_rate * prog.total_ms / 1000.0 / self.speed).max(1.0) as u32;
        self.total_frames_left = total;
        self.sq_phase = 0.0;
        self.delay_lp = 0.0;
        self.crush_counter = 0;
        self.crush_held = 0.0;
        self.env = 0.0;
        self.env_target = 1.0;
        self.state = Hk628State::Sounding;
        self.quiet_frames = 0;
    }

    /// Stop the voice (envelope fades out; the echo tail rings on).
    pub fn release(&mut self) {
        self.env_target = 0.0;
        self.playing = false;
    }

    /// Current audible state.
    #[must_use]
    pub fn state(&self) -> Hk628State {
        self.state
    }

    /// Wire value for the UI indicator (`0` idle · `1` sounding).
    #[must_use]
    pub fn state_code(&self) -> u8 {
        self.state.code()
    }

    /// Set the playback-rate multiplier (GS1 "Speed"). Pure assignment, RT-safe;
    /// affects steps loaded after this call.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.25, 4.0);
    }

    fn load_step(&mut self, idx: usize) {
        if let Some(step) = self.steps.get(idx) {
            // Speed scales the chip clock: slower = longer steps AND lower pitch
            // (like a real ROM chip's clock, and the dub "tape slowdown" feel).
            // Noise steps (pitch <= 0) stay noise. <= 0 untouched by the scale.
            self.cur_pitch = if step.pitch > 0.0 {
                step.pitch * self.speed
            } else {
                step.pitch
            };
            self.cur_level = step.level.clamp(0.0, 1.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let f = (self.sample_rate * step.dur_ms / 1000.0 / self.speed).max(1.0) as u32;
            self.step_frames_left = f;
        }
    }

    /// Process one stereo block, summing into `out[f·stride + offset ..][..2]`.
    /// RT-safe: bounded loop, integer LFSR, no allocation.
    pub fn process_block(&mut self, out: &mut [f32], stride: usize, offset: usize) {
        debug_assert!(stride >= 2 && offset + 2 <= stride);
        if self.state == Hk628State::Idle && self.env <= 0.0 {
            return;
        }

        for frame in out.chunks_exact_mut(stride) {
            // Step the sequencer.
            if self.playing {
                if self.step_frames_left == 0 {
                    self.step_idx += 1;
                    if self.step_idx >= self.steps.len() {
                        if self.repeat {
                            self.step_idx = 0;
                        } else {
                            self.playing = false;
                            self.env_target = 0.0;
                        }
                    }
                    if self.playing {
                        self.load_step(self.step_idx);
                    }
                } else {
                    self.step_frames_left -= 1;
                }
                if self.total_frames_left == 0 {
                    self.playing = false;
                    self.env_target = 0.0;
                } else {
                    self.total_frames_left -= 1;
                }
            }

            // Envelope (short global attack/release; steps snap — crude on
            // purpose, the chip's stepped character).
            if self.env < self.env_target {
                self.env = (self.env + self.attack_inc).min(self.env_target);
            } else if self.env > self.env_target {
                self.env = (self.env - self.release_inc).max(self.env_target);
            }

            // Noise generator (1-bit LFSR, sample-and-held at the noise clock).
            self.noise_acc += 1;
            if self.noise_acc >= self.noise_div {
                self.noise_acc = 0;
                let bit = ((self.lfsr >> 16) ^ (self.lfsr >> 13)) & 1;
                self.lfsr = ((self.lfsr << 1) | bit) & 0x1_ffff;
                self.noise_value = if self.lfsr & 1 == 1 { 1.0 } else { -1.0 };
            }

            // Source: a hard square at the step pitch, or noise for noise steps.
            let src = if self.cur_pitch > 0.0 {
                if self.sq_phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            } else {
                self.noise_value
            };

            let dry = src * self.cur_level * self.env * self.volume;

            // Optional slap-back (the rifle echo).
            let wet = if self.echo_on {
                let read = self.delay_write.wrapping_sub(self.delay_frames) & self.delay_mask;
                let w = self.delay[read];
                self.delay_lp = flush(self.delay_lp + 0.5 * (w - self.delay_lp));
                self.delay[self.delay_write] = flush(dry + self.delay_lp * self.delay_feedback);
                self.delay_write = (self.delay_write + 1) & self.delay_mask;
                w
            } else {
                0.0
            };

            let mut s = dry + wet * self.delay_mix;

            // Lo-fi grit: amplitude quantization + sample-and-hold.
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

            // Advance the square phase.
            if self.cur_pitch > 0.0 {
                self.sq_phase += self.cur_pitch / self.sample_rate;
                if self.sq_phase >= 1.0 {
                    self.sq_phase -= 1.0;
                }
            }

            // Self-terminate once not playing, the envelope is silent, and the
            // echo tail (if any) has decayed.
            if !self.playing && self.env <= ENV_FLOOR && wet.abs() <= ENV_FLOOR {
                self.quiet_frames = self.quiet_frames.saturating_add(1);
                if self.quiet_frames >= self.ready_frames {
                    self.state = Hk628State::Idle;
                }
            } else {
                self.quiet_frames = 0;
            }

            let s = s * OUTPUT_GAIN;
            frame[offset] += s;
            frame[offset + 1] += s;
        }
    }
}

/// Number of HK628 sounds.
pub const HK628_PROGRAM_COUNT: usize = HK628_PROGRAMS.len();

/// Get HK628 program `id` (falls back to 0 if out of range).
#[must_use]
pub fn hk628_program(id: usize) -> &'static Hk628Program {
    HK628_PROGRAMS.get(id).unwrap_or(&HK628_PROGRAMS[0])
}

/// Display name for HK628 program `id`.
#[must_use]
pub fn hk628_program_name(id: usize) -> &'static str {
    HK628_PROGRAMS.get(id).map_or("", |p| p.name)
}

// --- The 8 sounds, measured from the reference recording -------------------
//
// Pitches/durations are read from the per-sound contour analysis. `pitch = 0`
// is a noise step. These are first-pass and tuned against the reference by
// rendering + re-analysing (see the `dump_wavs` test).

#[rustfmt::skip]
const RIFLE_STEPS: [Hk628Step; 16] = [
    Hk628Step { pitch: 3150.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 424.0,  level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 3150.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 416.0,  level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 3150.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 2756.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 2450.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 2205.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 2004.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 1837.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 1696.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 1575.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 1378.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 1225.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 1102.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 0.0,    level: 1.0, dur_ms: 25.0 },
];

#[rustfmt::skip]
const ALARM_STEPS: [Hk628Step; 6] = [
    Hk628Step { pitch: 816.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 689.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 612.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 711.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 848.0, level: 1.0, dur_ms: 25.0 },
    Hk628Step { pitch: 958.0, level: 1.0, dur_ms: 25.0 },
];

#[rustfmt::skip]
const DUALTONE_STEPS: [Hk628Step; 2] = [
    Hk628Step { pitch: 1002.0, level: 1.0, dur_ms: 75.0 },
    Hk628Step { pitch: 760.0,  level: 1.0, dur_ms: 75.0 },
];

#[rustfmt::skip]
const BOMB1_STEPS: [Hk628Step; 3] = [
    Hk628Step { pitch: 2205.0, level: 1.0, dur_ms: 125.0 },
    Hk628Step { pitch: 1050.0, level: 1.0, dur_ms: 100.0 },
    Hk628Step { pitch: 2004.0, level: 1.0, dur_ms: 250.0 },
];

#[rustfmt::skip]
const BOMB2_STEPS: [Hk628Step; 21] = [
    Hk628Step { pitch: 2205.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 2004.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1837.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1696.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1575.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1470.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1378.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1297.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1225.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1160.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1102.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1050.0, level: 1.0, dur_ms: 73.0 },
    Hk628Step { pitch: 1002.0, level: 1.0, dur_ms: 73.0 },
    // Impact: rapid high/low buzz crackle.
    Hk628Step { pitch: 3150.0, level: 1.0, dur_ms: 14.0 },
    Hk628Step { pitch: 400.0,  level: 1.0, dur_ms: 14.0 },
    Hk628Step { pitch: 3150.0, level: 1.0, dur_ms: 14.0 },
    Hk628Step { pitch: 400.0,  level: 1.0, dur_ms: 14.0 },
    Hk628Step { pitch: 3150.0, level: 1.0, dur_ms: 14.0 },
    Hk628Step { pitch: 400.0,  level: 1.0, dur_ms: 14.0 },
    // Noise wash tail.
    Hk628Step { pitch: 0.0, level: 1.0, dur_ms: 220.0 },
    Hk628Step { pitch: 0.0, level: 0.6, dur_ms: 160.0 },
];

#[rustfmt::skip]
const ELECGUN1_STEPS: [Hk628Step; 7] = [
    Hk628Step { pitch: 420.0,  level: 1.0, dur_ms: 40.0 },
    Hk628Step { pitch: 0.0,    level: 0.9, dur_ms: 20.0 },
    Hk628Step { pitch: 420.0,  level: 1.0, dur_ms: 40.0 },
    Hk628Step { pitch: 0.0,    level: 0.9, dur_ms: 20.0 },
    Hk628Step { pitch: 2756.0, level: 1.0, dur_ms: 15.0 },
    Hk628Step { pitch: 424.0,  level: 1.0, dur_ms: 40.0 },
    Hk628Step { pitch: 0.0,    level: 0.9, dur_ms: 20.0 },
];

#[rustfmt::skip]
const ELECGUN2_STEPS: [Hk628Step; 7] = [
    Hk628Step { pitch: 2004.0, level: 1.0, dur_ms: 30.0 },
    Hk628Step { pitch: 648.0,  level: 1.0, dur_ms: 40.0 },
    Hk628Step { pitch: 0.0,    level: 0.6, dur_ms: 20.0 },
    Hk628Step { pitch: 711.0,  level: 1.0, dur_ms: 30.0 },
    Hk628Step { pitch: 648.0,  level: 1.0, dur_ms: 30.0 },
    Hk628Step { pitch: 2004.0, level: 1.0, dur_ms: 30.0 },
    Hk628Step { pitch: 0.0,    level: 0.6, dur_ms: 20.0 },
];

const HK628_PROGRAMS: [Hk628Program; 8] = [
    Hk628Program {
        name: "Rifle Gun",
        steps: &RIFLE_STEPS,
        repeat: true,
        total_ms: 1486.0,
        echo: false,
        volume: 0.9,
    },
    Hk628Program {
        name: "Rifle Echo",
        steps: &RIFLE_STEPS,
        repeat: true,
        total_ms: 1486.0,
        echo: true,
        volume: 0.85,
    },
    Hk628Program {
        name: "Alarm",
        steps: &ALARM_STEPS,
        repeat: true,
        total_ms: 1520.0,
        echo: false,
        volume: 0.7,
    },
    Hk628Program {
        name: "Dual Tone",
        steps: &DUALTONE_STEPS,
        repeat: true,
        total_ms: 1530.0,
        echo: false,
        volume: 0.7,
    },
    Hk628Program {
        name: "Bomb 1",
        steps: &BOMB1_STEPS,
        repeat: true,
        total_ms: 1509.0,
        echo: false,
        volume: 0.7,
    },
    Hk628Program {
        name: "Bomb 2",
        steps: &BOMB2_STEPS,
        repeat: false,
        total_ms: 1500.0,
        echo: false,
        volume: 0.8,
    },
    Hk628Program {
        name: "Electric Gun 1",
        steps: &ELECGUN1_STEPS,
        repeat: true,
        total_ms: 1530.0,
        echo: false,
        volume: 0.75,
    },
    Hk628Program {
        name: "Electric Gun 2",
        steps: &ELECGUN2_STEPS,
        repeat: true,
        total_ms: 1520.0,
        echo: false,
        volume: 0.7,
    },
];

// --- HK623 -----------------------------------------------------------------
//
// The HK623 is the HK628's near-twin: same 7 sounds, but slot 4 is a
// **Telephone Ring** instead of the Dual Tone (and the chip clocks at 100 kHz
// vs 128 kHz — negligible in our absolute-Hz recreation). Reuses the HK628 step
// banks; only the telephone ring is new.

/// Number of HK623 sounds.
pub const HK623_PROGRAM_COUNT: usize = HK623_PROGRAMS.len();

/// Get HK623 program `id` (falls back to 0 if out of range).
#[must_use]
pub fn hk623_program(id: usize) -> &'static Hk628Program {
    HK623_PROGRAMS.get(id).unwrap_or(&HK623_PROGRAMS[0])
}

/// Display name for HK623 program `id`.
#[must_use]
pub fn hk623_program_name(id: usize) -> &'static str {
    HK623_PROGRAMS.get(id).map_or("", |p| p.name)
}

// Telephone ring: a fast two-tone warble in ring bursts (ring … pause … ring).
#[rustfmt::skip]
const TELEPHONE_STEPS: [Hk628Step; 13] = [
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 28.0 },
    Hk628Step { pitch: 0.0,    level: 0.0, dur_ms: 320.0 }, // pause between rings
];

const HK623_PROGRAMS: [Hk628Program; 8] = [
    HK628_PROGRAMS[0], // Rifle Gun
    HK628_PROGRAMS[1], // Rifle Echo
    HK628_PROGRAMS[2], // Alarm
    Hk628Program {
        name: "Telephone Ring",
        steps: &TELEPHONE_STEPS,
        repeat: true,
        total_ms: 2400.0,
        echo: false,
        volume: 0.7,
    },
    HK628_PROGRAMS[4], // Bomb 1
    HK628_PROGRAMS[5], // Bomb 2
    HK628_PROGRAMS[6], // Electric Gun 1
    HK628_PROGRAMS[7], // Electric Gun 2
];

fn ramp_inc(millis: f32, sample_rate: f32) -> f32 {
    let frames = (sample_rate * millis / 1000.0).max(1.0);
    1.0 / frames
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

    const SR: f32 = 22_050.0;

    fn render(voice: &mut Hk628, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let mut buf = [0.0f32, 0.0f32];
            voice.process_block(&mut buf, 2, 0);
            out.push(buf[0]);
        }
        out
    }

    #[test]
    #[ignore]
    fn measure_loudness() {
        // Render every HK628 program looped to ~3s and report peak/RMS/LUFS so
        // we can match the siren to the -14 LUFS music target. Run with
        // `cargo test -p dub-dsp measure_loudness -- --ignored --nocapture`.
        let sr = 48_000.0_f32;
        for id in 0..HK628_PROGRAM_COUNT {
            let mut v = Hk628::new(sr);
            // Force a sustained read by re-triggering each time it idles.
            let mut samples = Vec::new();
            for _ in 0..6 {
                v.trigger(hk628_program(id));
                samples.extend(render(&mut v, (sr * 0.5) as usize));
            }
            let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
            let rms = (samples.iter().map(|&x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
            let stereo: Vec<f32> = samples.iter().flat_map(|&s| [s, s]).collect();
            let lufs = crate::loudness::measure_integrated_loudness(&stereo, sr as u32, 2).lufs_i;
            println!(
                "prog {id:2} {:>14}  peak {peak:.3}  rms {rms:.3}  lufs {lufs:?}",
                hk628_program_name(id)
            );
        }
    }

    #[test]
    fn idle_is_additive_no_op() {
        let mut v = Hk628::new(SR);
        let mut buf = [0.2f32, -0.3, 0.4, -0.1];
        let before = buf;
        v.process_block(&mut buf, 2, 0);
        assert_eq!(buf, before);
    }

    #[test]
    fn every_program_sounds_then_idles() {
        for id in 0..HK628_PROGRAM_COUNT {
            let mut v = Hk628::new(SR);
            v.trigger(hk628_program(id));
            let peak = render(&mut v, SR as usize / 2)
                .iter()
                .fold(0.0f32, |m, &x| m.max(x.abs()));
            // The voice is trimmed ~16 dB (OUTPUT_GAIN) to sit at the music's
            // -14 LUFS, so a sounding program peaks ~0.1; 0.04 clears silence.
            assert!(
                peak > 0.04,
                "program {id} ({}) silent",
                hk628_program_name(id)
            );
            for s in render(&mut v, SR as usize * 4) {
                assert!(s.is_finite(), "program {id} non-finite");
            }
            assert_eq!(
                v.state(),
                Hk628State::Idle,
                "program {id} ({}) never idled",
                hk628_program_name(id)
            );
        }
    }

    #[test]
    fn process_block_is_alloc_free() {
        let mut v = Hk628::new(SR);
        let mut buf = vec![0.0f32; 256 * 4];
        assert_no_alloc::assert_no_alloc(|| {
            v.trigger(hk628_program(0));
            for _ in 0..256 {
                v.process_block(&mut buf, 4, 2);
            }
            v.trigger(hk628_program(5));
            for _ in 0..256 {
                v.process_block(&mut buf, 4, 2);
            }
        });
    }

    /// Render every program to a 22.05 kHz mono WAV in the scratch dir for
    /// offline comparison against the reference recording. Ignored by default;
    /// run with `cargo test -p dub-dsp dump_wavs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dump_wavs() {
        let dir = "/private/tmp/claude-501/-Users-klos-Development-dub/05fb1678-ea88-4fb6-bb52-d61e8c879ac7/scratchpad";
        for id in 0..HK628_PROGRAM_COUNT {
            let mut v = Hk628::new(SR);
            v.trigger(hk628_program(id));
            let samples = render(&mut v, (SR * 1.7) as usize);
            write_wav(&format!("{dir}/hk_{id}.wav"), &samples, SR as u32);
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
        d.extend_from_slice(&1u16.to_le_bytes()); // PCM
        d.extend_from_slice(&1u16.to_le_bytes()); // mono
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
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&d).unwrap();
    }
}
