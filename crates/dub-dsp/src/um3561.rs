//! A recreation of the **UMC UM3561** siren sound IC — the ubiquitous 3 V
//! toy/alarm chip behind countless police/ambulance/fire/machine-gun sirens.
//! Four sounds, selected on real hardware via the SEL1/SEL2 pins:
//!
//! 1. **Police** — a wailing pitch sweep up and down.
//! 2. **Fire Engine** — a wider, faster warble ("whoop").
//! 3. **Ambulance** — a two-tone "ee-aw" alternation.
//! 4. **Machine Gun** — a low square gated into rapid pulses.
//!
//! Like the [`crate::hk628`] chip it's a crude digital ROM player driving a
//! hard square output, so it reuses the same step-sequencer voice
//! ([`crate::hk628::Hk628`]) — these programs are just a different bank of
//! [`crate::hk628::Hk628Program`] step sequences. (The shared program/step
//! types are the generic "digital sound-effect chip" format; they'll be renamed
//! chip-agnostic when the Expert-mode chip selector lands.)
//!
//! The UM3561's sirens are continuous on real hardware; here each is a ~2.5 s
//! looped one-shot (tap to fire, re-tap to repeat), matching the pad model.

use crate::hk628::{Hk628Program, Hk628Step};

/// Number of UM3561 sounds.
pub const UM3561_PROGRAM_COUNT: usize = UM3561_PROGRAMS.len();

/// Get UM3561 program `id` (falls back to 0 if out of range).
#[must_use]
pub fn um3561_program(id: usize) -> &'static Hk628Program {
    UM3561_PROGRAMS.get(id).unwrap_or(&UM3561_PROGRAMS[0])
}

/// Display name for UM3561 program `id`.
#[must_use]
pub fn um3561_program_name(id: usize) -> &'static str {
    UM3561_PROGRAMS.get(id).map_or("", |p| p.name)
}

// Police "wail": a square ramping up then down, ~0.8 s per cycle, looped.
#[rustfmt::skip]
const POLICE_STEPS: [Hk628Step; 12] = [
    Hk628Step { pitch: 660.0,  level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 760.0,  level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 860.0,  level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 1090.0, level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 1180.0, level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 1090.0, level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 980.0,  level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 860.0,  level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 760.0,  level: 1.0, dur_ms: 70.0 },
    Hk628Step { pitch: 660.0,  level: 1.0, dur_ms: 70.0 },
];

// Fire engine: wider, faster warble.
#[rustfmt::skip]
const FIRE_STEPS: [Hk628Step; 10] = [
    Hk628Step { pitch: 520.0,  level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 700.0,  level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 900.0,  level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 1120.0, level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 1280.0, level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 1280.0, level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 1120.0, level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 900.0,  level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 700.0,  level: 1.0, dur_ms: 45.0 },
    Hk628Step { pitch: 520.0,  level: 1.0, dur_ms: 45.0 },
];

// Ambulance: two-tone "ee-aw".
#[rustfmt::skip]
const AMBULANCE_STEPS: [Hk628Step; 2] = [
    Hk628Step { pitch: 1000.0, level: 1.0, dur_ms: 380.0 },
    Hk628Step { pitch: 800.0,  level: 1.0, dur_ms: 380.0 },
];

// Machine gun: a low square gated on/off into rapid pulses (~16/s).
#[rustfmt::skip]
const MGUN_STEPS: [Hk628Step; 2] = [
    Hk628Step { pitch: 190.0, level: 1.0, dur_ms: 30.0 },
    Hk628Step { pitch: 190.0, level: 0.0, dur_ms: 32.0 },
];

const UM3561_PROGRAMS: [Hk628Program; 4] = [
    Hk628Program {
        name: "Police",
        steps: &POLICE_STEPS,
        repeat: true,
        total_ms: 2500.0,
        echo: false,
        volume: 0.7,
    },
    Hk628Program {
        name: "Fire Engine",
        steps: &FIRE_STEPS,
        repeat: true,
        total_ms: 2500.0,
        echo: false,
        volume: 0.7,
    },
    Hk628Program {
        name: "Ambulance",
        steps: &AMBULANCE_STEPS,
        repeat: true,
        total_ms: 2500.0,
        echo: false,
        volume: 0.7,
    },
    Hk628Program {
        name: "Machine Gun",
        steps: &MGUN_STEPS,
        repeat: true,
        total_ms: 1500.0,
        echo: false,
        volume: 0.8,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hk628::{Hk628, Hk628State};

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
    fn every_program_sounds() {
        for id in 0..UM3561_PROGRAM_COUNT {
            let mut v = Hk628::new(SR);
            v.trigger(um3561_program(id));
            let peak = render(&mut v, SR as usize / 2)
                .iter()
                .fold(0.0f32, |m, &x| m.max(x.abs()));
            // The voice is trimmed ~16 dB to sit at the music's -14 LUFS, so a
            // sounding program peaks ~0.1; 0.04 is comfortably above silence.
            assert!(
                peak > 0.04,
                "program {id} ({}) silent",
                um3561_program_name(id)
            );
            for s in render(&mut v, SR as usize) {
                assert!(s.is_finite());
            }
        }
    }

    #[test]
    fn machine_gun_pulses() {
        // The gated square should produce loud and near-silent windows.
        let mut v = Hk628::new(SR);
        v.trigger(um3561_program(3));
        let out = render(&mut v, SR as usize / 2);
        let mut loud = false;
        let mut quiet = false;
        for chunk in out.chunks(220) {
            let p = chunk.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
            if p > 0.05 {
                loud = true;
            }
            if p < 0.02 {
                quiet = true;
            }
        }
        assert!(loud && quiet, "machine gun not pulsing");
    }

    #[test]
    fn release_goes_idle() {
        let mut v = Hk628::new(SR);
        v.trigger(um3561_program(0));
        let _ = render(&mut v, 1_000);
        v.release();
        let _ = render(&mut v, SR as usize / 2);
        assert_eq!(v.state(), Hk628State::Idle);
    }

    /// Render the 4 UM3561 sounds to scratch WAVs for offline comparison.
    /// Ignored by default; run with `-- --ignored`.
    #[test]
    #[ignore]
    fn dump_wavs() {
        let dir = "/private/tmp/claude-501/-Users-klos-Development-dub/05fb1678-ea88-4fb6-bb52-d61e8c879ac7/scratchpad";
        for id in 0..UM3561_PROGRAM_COUNT {
            let mut v = Hk628::new(SR);
            v.trigger(um3561_program(id));
            let samples = render(&mut v, (SR * 2.0) as usize);
            write_wav(&format!("{dir}/um_{id}.wav"), &samples, SR as u32);
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
