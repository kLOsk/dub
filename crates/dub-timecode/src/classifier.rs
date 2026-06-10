//! Source classifier — is a deck's input a **timecode** control
//! record or a **real record** (music)?
//!
//! This is the heart of automatic source detection (PRD §5.1.1): the
//! engine watches each deck's decoded signal and decides whether to
//! drive a loaded file from the control vinyl (Timecode) or pass the
//! audio straight through (Thru / real record). It also gates
//! auto-calibration — we only whiten a deck we're sure is timecode.
//!
//! The discriminator is essentially free: the decoder already reports
//! a **quadrature confidence**. A timecode carrier is a coherent
//! rotating phasor, so its confidence sits near 1.0; broadband music
//! has no consistent rotation, so its confidence collapses toward 0.
//! Two guards stop false positives:
//!
//!   * an **amplitude floor** (no signal ⇒ Silence, not Record), and
//!   * a **rate band** — a sustained mono tone (e.g. a bass note) also
//!     reads high confidence but does *not rotate* (rate ≈ 0), so we
//!     require the carrier to be turning at a plausible platter speed.
//!
//! A short **sustain** requirement means a transient coincidence can't
//! flip the deck into Timecode.

use crate::DecodeOutput;

/// What the classifier believes a deck's input is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceClass {
    /// No usable signal on the input (needle up, or stopped platter).
    Silence,
    /// A coherent timecode carrier turning at a plausible speed.
    Timecode,
    /// Broadband audio — a real record (drive Thru).
    Record,
}

/// Streaming classifier. Feed it one [`DecodeOutput`] per audio block;
/// it returns the current belief with hysteresis.
#[derive(Debug, Clone)]
pub struct SourceClassifier {
    amp_floor: f32,
    conf_lock: f32,
    rate_min: f64,
    rate_max: f64,
    confirm_blocks: u32,
    streak: u32,
    current: SourceClass,
}

impl Default for SourceClassifier {
    fn default() -> Self {
        Self {
            // Matches the lift policy's carrier-dead RMS floor.
            amp_floor: 0.01,
            // Well below a clean carrier (~1.0) and an uncalibrated one
            // (~0.85), well above music (~0.05–0.3).
            conf_lock: 0.6,
            // Plausible platter band for *detection* (needle dropped on
            // a spinning record). Excludes near-zero "mono tone" rates.
            rate_min: 0.05,
            rate_max: 4.0,
            // ~8 blocks ≈ 40–170 ms depending on buffer — long enough to
            // reject a transient, short enough to feel instant.
            confirm_blocks: 8,
            streak: 0,
            current: SourceClass::Silence,
        }
    }
}

impl SourceClassifier {
    /// A classifier with default thresholds (see the struct docs).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current belief without consuming a new block.
    #[must_use]
    pub fn current(&self) -> SourceClass {
        self.current
    }

    /// Feed one decoded block; returns the (possibly updated) belief.
    pub fn update(&mut self, out: &DecodeOutput) -> SourceClass {
        if out.amplitude < self.amp_floor {
            self.streak = 0;
            self.current = SourceClass::Silence;
            return self.current;
        }
        let looks_timecode = out.confidence >= self.conf_lock
            && out.rate.abs() >= self.rate_min
            && out.rate.abs() <= self.rate_max;
        if looks_timecode {
            self.streak = self.streak.saturating_add(1);
            if self.streak >= self.confirm_blocks {
                self.current = SourceClass::Timecode;
            }
            // Until confirmed, hold the previous belief (don't flap).
        } else {
            // Carrier present but incoherent / not rotating → music.
            self.streak = 0;
            self.current = SourceClass::Record;
        }
        self.current
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
mod tests {
    use super::*;
    use crate::signal::Generator;
    use crate::{compute_whitening, Decoder, Format};

    fn classify_buffer(dec: &mut Decoder, clf: &mut SourceClassifier, buf: &[f32], block: usize) {
        for chunk in buf.chunks(block * 2) {
            let out = dec.process(chunk);
            clf.update(&out);
        }
    }

    #[test]
    fn clean_timecode_classifies_as_timecode() {
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut clf = SourceClassifier::new();
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.5);
        classify_buffer(&mut dec, &mut clf, &buf, 256);
        assert_eq!(clf.current(), SourceClass::Timecode);
    }

    #[test]
    fn uncalibrated_imbalanced_timecode_still_classifies_as_timecode() {
        // Even a 2 dB-imbalanced cartridge (confidence ~0.85) must read
        // as timecode — that's what lets us detect *then* calibrate.
        let sr = 48_000.0_f32;
        let mut gen = Generator::new(Format::SeratoCv02, sr);
        let mut buf = vec![0.0f32; 4_800 * 2];
        gen.render(&mut buf, 1.0, 0.5);
        for frame in buf.chunks_exact_mut(2) {
            frame[1] *= 1.26;
        }
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut clf = SourceClassifier::new();
        classify_buffer(&mut dec, &mut clf, &buf, 256);
        assert_eq!(clf.current(), SourceClass::Timecode);
        // sanity: the whitening from this buffer is non-identity.
        let w = compute_whitening(&buf);
        assert!((w[0][0] - w[1][1]).abs() > 1e-2, "should detect imbalance");
    }

    #[test]
    fn broadband_noise_classifies_as_record() {
        // Two independent noise channels (deterministic xorshift) → no
        // coherent rotation → low confidence → Record.
        let sr = 48_000.0_f32;
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        let mut buf = vec![0.0f32; 4_800 * 2];
        for s in &mut buf {
            *s = next() * 0.6;
        }
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut clf = SourceClassifier::new();
        classify_buffer(&mut dec, &mut clf, &buf, 256);
        assert_eq!(clf.current(), SourceClass::Record);
    }

    #[test]
    fn mono_tone_is_not_timecode() {
        // A sustained mono tone reads high confidence but doesn't rotate
        // (rate ≈ 0) — the rate band must keep it out of Timecode.
        let sr = 48_000.0_f32;
        let f = 110.0_f64;
        let mut buf = vec![0.0f32; 4_800 * 2];
        for (i, frame) in buf.chunks_exact_mut(2).enumerate() {
            let x = (std::f64::consts::TAU * f * i as f64 / f64::from(sr)).sin() as f32 * 0.5;
            frame[0] = x;
            frame[1] = x;
        }
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut clf = SourceClassifier::new();
        classify_buffer(&mut dec, &mut clf, &buf, 256);
        assert_ne!(clf.current(), SourceClass::Timecode);
    }

    #[test]
    fn silence_classifies_as_silence() {
        let sr = 48_000.0_f32;
        let buf = vec![0.0f32; 4_800 * 2];
        let mut dec = Decoder::new(Format::SeratoCv02, sr);
        let mut clf = SourceClassifier::new();
        classify_buffer(&mut dec, &mut clf, &buf, 256);
        assert_eq!(clf.current(), SourceClass::Silence);
    }
}
