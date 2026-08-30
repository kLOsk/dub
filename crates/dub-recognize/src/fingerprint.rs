//! The TEST2 fingerprint AcoustID expects, in the form it expects it.
//!
//! Two things differ from `dub-fingerprint`, and both matter:
//!
//! **Preset.** Library dedupe (M11b) runs `preset_test1`, which only has
//! to be self-consistent — it compares Dub's fingerprints against Dub's
//! own. AcoustID's database is built on **TEST2**, so a TEST1
//! fingerprint submitted there matches nothing. Recognition therefore
//! computes its own from the same PCM; the stored dedupe blobs are
//! untouched and there is no schema change.
//!
//! **Encoding.** AcoustID does not take the raw `u32` sub-fingerprints.
//! It takes Chromaprint's compressed representation — a bit-packed
//! delta encoding with a normal and an exception plane — base64'd in the
//! URL-safe alphabet with no padding. `rusty_chromaprint` exposes the
//! compressor, so the wire format is a base64 call on top of it rather
//! than a reimplementation.

use base64::Engine as _;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};

use crate::error::RecognizeError;

/// A fingerprint ready to hand to AcoustID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcoustIdFingerprint {
    /// Base64url, unpadded — exactly what goes in the `fingerprint` field.
    pub encoded: String,
    /// Whole seconds, what goes in the `duration` field. AcoustID uses
    /// it to disambiguate, and a wrong duration quietly costs matches.
    pub duration_secs: u32,
}

/// Compute the AcoustID fingerprint for one track's PCM.
///
/// `samples` is interleaved. Anything under ~10 s is refused: Chromaprint
/// needs a real span to be distinctive, and AcoustID's index will not
/// match a fragment — better a clear error than a confident wrong answer.
pub fn fingerprint(
    samples: &[i16],
    sample_rate: u32,
    channels: u16,
) -> Result<AcoustIdFingerprint, RecognizeError> {
    if sample_rate == 0 || channels == 0 {
        return Err(RecognizeError::Fingerprint(
            "zero sample rate or channel count".into(),
        ));
    }
    let frames = samples.len() / channels as usize;
    let duration_secs = frames as u32 / sample_rate;
    if duration_secs < MIN_DURATION_SECS {
        return Err(RecognizeError::TooShort {
            secs: duration_secs,
            min: MIN_DURATION_SECS,
        });
    }

    let config = Configuration::preset_test2();
    let mut printer = Fingerprinter::new(&config);
    printer
        .start(sample_rate, u32::from(channels))
        .map_err(|e| RecognizeError::Fingerprint(e.to_string()))?;
    printer.consume(samples);
    printer.finish();

    let raw = printer.fingerprint();
    if raw.is_empty() {
        return Err(RecognizeError::Fingerprint(
            "fingerprinter produced nothing".into(),
        ));
    }
    let compressed = FingerprintCompressor::from(&config).compress(raw);
    Ok(AcoustIdFingerprint {
        encoded: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(compressed),
        duration_secs,
    })
}

/// Shortest span worth submitting. AcoustID's own client refuses to
/// look up less than this.
pub const MIN_DURATION_SECS: u32 = 10;

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone at `freq` — deterministic, and distinct enough between
    /// frequencies that two of them must not fingerprint alike.
    fn tone(freq: f32, secs: u32, sr: u32) -> Vec<i16> {
        let n = (secs * sr) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let v = (std::f32::consts::TAU * freq * t).sin()
                    + 0.4 * (std::f32::consts::TAU * freq * 2.5 * t).sin();
                (v * 8000.0) as i16
            })
            .collect()
    }

    #[test]
    fn produces_base64url_without_padding() {
        let fp = fingerprint(&tone(440.0, 20, 44_100), 44_100, 1).unwrap();
        assert_eq!(fp.duration_secs, 20);
        assert!(!fp.encoded.is_empty());
        // URL-safe alphabet, and unpadded — AcoustID rejects '+' and '/'
        // in the fingerprint field, and a trailing '=' survives URL
        // encoding badly.
        assert!(
            !fp.encoded.contains('+') && !fp.encoded.contains('/') && !fp.encoded.contains('='),
            "not base64url-unpadded: {}",
            &fp.encoded[..fp.encoded.len().min(40)]
        );
    }

    #[test]
    fn the_compressed_form_starts_with_the_chromaprint_header() {
        // Chromaprint's compressed format opens with a version byte
        // (algorithm - 1, so 1 for TEST2) followed by a 24-bit length.
        // Decoding the base64 back is the cheapest way to prove we are
        // sending the real wire format and not raw u32s.
        let fp = fingerprint(&tone(440.0, 20, 44_100), 44_100, 1).unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&fp.encoded)
            .expect("valid base64url");
        assert!(bytes.len() > 4, "compressed fingerprint is too short");
        assert_eq!(bytes[0], 1, "expected the TEST2 algorithm byte");
        let len = u32::from(bytes[1]) << 16 | u32::from(bytes[2]) << 8 | u32::from(bytes[3]);
        assert!(len > 0, "header claims an empty fingerprint");
    }

    #[test]
    fn different_audio_fingerprints_differently() {
        let a = fingerprint(&tone(440.0, 20, 44_100), 44_100, 1).unwrap();
        let b = fingerprint(&tone(660.0, 20, 44_100), 44_100, 1).unwrap();
        assert_ne!(a.encoded, b.encoded);
    }

    #[test]
    fn the_same_audio_fingerprints_identically() {
        let a = fingerprint(&tone(440.0, 20, 44_100), 44_100, 1).unwrap();
        let b = fingerprint(&tone(440.0, 20, 44_100), 44_100, 1).unwrap();
        assert_eq!(a, b, "fingerprinting must be deterministic");
    }

    #[test]
    fn a_fragment_is_refused_rather_than_looked_up() {
        // Better a clear error than a confident wrong match.
        let err = fingerprint(&tone(440.0, 3, 44_100), 44_100, 1).unwrap_err();
        assert!(
            matches!(err, RecognizeError::TooShort { secs: 3, .. }),
            "{err}"
        );
    }

    #[test]
    fn stereo_duration_counts_frames_not_samples() {
        let mono = tone(440.0, 20, 44_100);
        let stereo: Vec<i16> = mono.iter().flat_map(|&s| [s, s]).collect();
        let fp = fingerprint(&stereo, 44_100, 2).unwrap();
        assert_eq!(
            fp.duration_secs, 20,
            "a 20 s stereo track must report 20 s, not 40"
        );
    }
}
