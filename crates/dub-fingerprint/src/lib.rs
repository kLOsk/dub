//! Audio fingerprinting for Dub.
//!
//! Computes a per-recording fingerprint using the Chromaprint
//! algorithm (algorithm 2 — the same one AcoustID uses), via the
//! pure-Rust [`rusty_chromaprint`] implementation. M11b uses
//! fingerprints for **library-internal dedupe** (PRD §8.1). v1.1
//! will reuse the same primitives for Thru-mode real-record
//! recognition (PRD §5.2.5).
//!
//! # Why pure-Rust
//!
//! M11b chose `rusty_chromaprint` over an FFI wrapper around the
//! reference C library (`chromaprint`, LGPL-2.1) for the same
//! reasons `dub-bpm` chose pure-Rust over aubio at M7.5: license
//! isolation, no C build dep, no unsafe FFI surface, simpler
//! distribution. Our use case is similarity-based dedupe inside the
//! Dub library, not AcoustID database lookup, so cross-
//! implementation bit-identity is not a requirement.
//!
//! The algorithm itself is unchanged. `docs/LIBRARY-SCHEMA.md`
//! documents the Chromaprint parameters Dub uses (algorithm 2,
//! 11025 Hz mono, 4096-frame FFT, full-track window, raw
//! `uint32_t[]` storage) so a third party can re-derive Dub's
//! fingerprints with any algorithm-2-faithful implementation.
//!
//! # Typical use
//!
//! ```no_run
//! use dub_fingerprint::{Fingerprint, similarity};
//!
//! // From decoded f32 samples (interleaved if stereo).
//! let samples: Vec<f32> = vec![0.0; 44100 * 30];
//! let fp = Fingerprint::compute_from_f32(&samples, 44100, 1).unwrap();
//!
//! // Round-trip through the SQLite BLOB column.
//! let blob = fp.to_blob();
//! let restored = Fingerprint::from_blob(&blob, fp.duration_ms()).unwrap();
//!
//! // Similarity in [0.0, 1.0]. Library-dedupe threshold is 0.98.
//! let s = similarity(&fp, &restored);
//! assert!(s >= 0.999);
//! ```
//!
//! This crate intentionally exposes no `unsafe` code (the BLOB
//! serialization uses safe slice operations rather than transmute).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use thiserror::Error;

/// Library version reported by the crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errors the fingerprint subsystem can surface.
#[derive(Debug, Error)]
pub enum FingerprintError {
    /// Input parameters were rejected by the underlying Chromaprint
    /// implementation (typically: sample rate or channel count of 0).
    #[error("invalid fingerprint input: {reason}")]
    InvalidInput {
        /// Short human-readable description of the failure mode.
        reason: &'static str,
    },

    /// A fingerprint BLOB read from storage was malformed (not a
    /// multiple of 4 bytes, or empty).
    #[error("malformed fingerprint blob: {reason}")]
    MalformedBlob {
        /// Short human-readable description of the failure mode.
        reason: &'static str,
    },
}

/// Result alias for crate-internal use.
pub type Result<T> = std::result::Result<T, FingerprintError>;

/// A canonical fingerprint for one recording.
///
/// Stores the raw Chromaprint output (`Vec<u32>`) plus the source
/// duration. The duration is captured at compute time because
/// downstream consumers (the §8.1 dedupe check requires a duration
/// delta < 200 ms) need it independent of any per-file metadata,
/// which can disagree across encodings.
///
/// Serialisation: [`Fingerprint::to_blob`] / [`Fingerprint::from_blob`]
/// round-trip the items as little-endian `u32`s, matching the
/// `fingerprints.chromaprint_blob` column documented in
/// `docs/LIBRARY-SCHEMA.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    items: Vec<u32>,
    duration_ms: u32,
}

impl Fingerprint {
    /// Compute a fingerprint from `i16` PCM samples. This is the
    /// native input form for the Chromaprint algorithm. If you have
    /// `f32` samples, prefer [`Fingerprint::compute_from_f32`]
    /// which handles the conversion in-place.
    ///
    /// `channels` is `1` for mono and `2` for stereo. For stereo,
    /// samples are interleaved `[L0, R0, L1, R1, ...]`.
    pub fn compute(samples: &[i16], sample_rate: u32, channels: u32) -> Result<Self> {
        if sample_rate == 0 {
            return Err(FingerprintError::InvalidInput {
                reason: "sample_rate must be > 0",
            });
        }
        if !(1..=2).contains(&channels) {
            return Err(FingerprintError::InvalidInput {
                reason: "channels must be 1 or 2",
            });
        }
        let config = rusty_chromaprint::Configuration::preset_test1();
        let mut printer = rusty_chromaprint::Fingerprinter::new(&config);
        printer
            .start(sample_rate, channels)
            .map_err(|_| FingerprintError::InvalidInput {
                reason: "rusty_chromaprint rejected start() parameters",
            })?;
        printer.consume(samples);
        printer.finish();
        let items = printer.fingerprint().to_vec();
        // Duration in ms from the input sample count. Done here
        // rather than reading from the chromaprint config so the
        // duration is the recording's actual length, not a delayed
        // / silence-trimmed version.
        let frame_count = samples.len() as u64 / channels.max(1) as u64;
        let duration_ms = ((frame_count * 1000) / sample_rate.max(1) as u64) as u32;
        Ok(Self { items, duration_ms })
    }

    /// Compute a fingerprint from `f32` PCM samples. Allocates a
    /// scratch `Vec<i16>` and clamps + scales `[-1.0, 1.0]` →
    /// `[i16::MIN, i16::MAX]` before delegating to [`Self::compute`].
    ///
    /// Off the audio thread by construction — this is the importer
    /// / analysis path. Allocations are fine here.
    pub fn compute_from_f32(samples: &[f32], sample_rate: u32, channels: u32) -> Result<Self> {
        let mut scratch = Vec::with_capacity(samples.len());
        for &s in samples {
            // Saturating cast: any NaN or out-of-range value clamps
            // to the corresponding i16 bound rather than wrapping.
            let scaled = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round();
            scratch.push(scaled as i16);
        }
        Self::compute(&scratch, sample_rate, channels)
    }

    /// Raw `u32` items in the fingerprint.
    pub fn items(&self) -> &[u32] {
        &self.items
    }

    /// Source recording duration in milliseconds. Computed from the
    /// sample count at fingerprint time (not from any per-file
    /// metadata, which can disagree across encodings).
    pub fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    /// Number of items in the fingerprint. Roughly proportional to
    /// the recording duration; at the default Chromaprint
    /// configuration each item covers ≈124 ms.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `true` when the fingerprint produced no items (e.g. an input
    /// shorter than the configuration's framing requirement).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Serialise the fingerprint items to a little-endian byte
    /// vector suitable for the `fingerprints.chromaprint_blob`
    /// column. Format: `[item_0_le; item_1_le; ...]`, 4 bytes per
    /// item, no header. Length is recoverable as `bytes.len() / 4`.
    pub fn to_blob(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.items.len() * 4);
        for item in &self.items {
            out.extend_from_slice(&item.to_le_bytes());
        }
        out
    }

    /// Deserialise a fingerprint from a BLOB and a known duration.
    /// The duration comes from the `fingerprints.duration_ms`
    /// column rather than being re-derivable from the BLOB bytes,
    /// since item count → duration depends on the Chromaprint
    /// configuration in use at compute time.
    pub fn from_blob(bytes: &[u8], duration_ms: u32) -> Result<Self> {
        if bytes.is_empty() {
            return Err(FingerprintError::MalformedBlob {
                reason: "empty fingerprint blob",
            });
        }
        if !bytes.len().is_multiple_of(4) {
            return Err(FingerprintError::MalformedBlob {
                reason: "fingerprint blob length is not a multiple of 4 bytes",
            });
        }
        let mut items = Vec::with_capacity(bytes.len() / 4);
        for chunk in bytes.chunks_exact(4) {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(chunk);
            items.push(u32::from_le_bytes(buf));
        }
        Ok(Self { items, duration_ms })
    }
}

/// Similarity in `[0.0, 1.0]` between two fingerprints, computed as
/// `1.0 - (bit_distance / total_bits)` over the best alignment in a
/// small sliding window.
///
/// The window covers ±30 items (≈ ±3.7 s at the default
/// configuration) which absorbs the typical start-offset variation
/// between two encodings of the same recording (silence trim, lead-
/// in differences, etc.). Out-of-window alignment requires the
/// caller to fall back to [`rusty_chromaprint::match_fingerprints`]
/// (used by v1.1 Thru-mode real-record recognition, where the
/// turntable start position is arbitrary).
///
/// **Library-dedupe threshold per PRD §8.1: ≥ 0.98.**
///
/// Returns `0.0` if either fingerprint is empty.
pub fn similarity(a: &Fingerprint, b: &Fingerprint) -> f32 {
    similarity_with_window(a, b, 30)
}

/// Variant of [`similarity`] with an explicit sliding-window radius
/// in items. Exposed for v1.1 real-record recognition which needs a
/// larger window (the turntable start position is arbitrary).
pub fn similarity_with_window(a: &Fingerprint, b: &Fingerprint, window_items: usize) -> f32 {
    if a.items.is_empty() || b.items.is_empty() {
        return 0.0;
    }
    let mut best = 0.0_f32;
    // Range of relative offsets to test. `offset = +n` means b is
    // shifted `n` items later than a; `offset = -n` is the reverse.
    let range = window_items as isize;
    for offset in -range..=range {
        let s = similarity_at_offset(&a.items, &b.items, offset);
        if s > best {
            best = s;
        }
    }
    best
}

/// Hamming-distance similarity of two `u32` slices aligned at the
/// given offset. `offset = 0` aligns `a[0]` with `b[0]`. The
/// comparison span is the overlap of the two slices after the
/// shift; uncompared items count as neither match nor mismatch
/// (they're outside the overlap, not "wrong").
fn similarity_at_offset(a: &[u32], b: &[u32], offset: isize) -> f32 {
    let (a_start, b_start) = if offset >= 0 {
        (0_usize, offset as usize)
    } else {
        ((-offset) as usize, 0_usize)
    };
    if a_start >= a.len() || b_start >= b.len() {
        return 0.0;
    }
    let span = (a.len() - a_start).min(b.len() - b_start);
    if span == 0 {
        return 0.0;
    }
    // Require at least 8 items of overlap (≈1 s) before we trust
    // a comparison. Below that the bit count is too small to be
    // meaningful and noise can produce spurious high scores.
    if span < 8 {
        return 0.0;
    }
    let mut diff_bits: u64 = 0;
    for i in 0..span {
        diff_bits += (a[a_start + i] ^ b[b_start + i]).count_ones() as u64;
    }
    let total_bits = (span as u64) * 32;
    1.0 - (diff_bits as f32 / total_bits as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a synthetic mono test tone. Returns interleaved-free
    /// `f32` samples in `[-0.5, 0.5]`.
    fn tone(freq_hz: f32, sample_rate: u32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate as f32 * duration_secs) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / sample_rate as f32;
            out.push(0.5 * (2.0 * std::f32::consts::PI * freq_hz * t).sin());
        }
        out
    }

    #[test]
    fn version_is_nonempty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn compute_produces_nonempty_fingerprint_for_long_input() {
        // 15 s of a 440 Hz sine is well over Chromaprint's framing
        // minimum (~3 s); we must produce items.
        let samples = tone(440.0, 11025, 15.0);
        let fp = Fingerprint::compute_from_f32(&samples, 11025, 1).expect("compute");
        assert!(!fp.is_empty(), "long input must produce items");
        assert!(fp.duration_ms() > 14_000 && fp.duration_ms() < 16_000);
    }

    #[test]
    fn compute_rejects_zero_sample_rate() {
        let samples = tone(440.0, 11025, 5.0);
        let err = Fingerprint::compute_from_f32(&samples, 0, 1).unwrap_err();
        matches!(err, FingerprintError::InvalidInput { .. });
    }

    #[test]
    fn compute_rejects_unsupported_channel_count() {
        let samples = tone(440.0, 11025, 5.0);
        assert!(Fingerprint::compute_from_f32(&samples, 11025, 0).is_err());
        assert!(Fingerprint::compute_from_f32(&samples, 11025, 3).is_err());
    }

    #[test]
    fn same_input_produces_identical_fingerprint() {
        // The whole library-dedupe story collapses if the algorithm
        // isn't deterministic. Two compute calls on the same input
        // must yield byte-identical fingerprints.
        let samples = tone(880.0, 11025, 10.0);
        let a = Fingerprint::compute_from_f32(&samples, 11025, 1).unwrap();
        let b = Fingerprint::compute_from_f32(&samples, 11025, 1).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn similarity_of_identical_fingerprints_is_one() {
        let samples = tone(220.0, 11025, 12.0);
        let fp = Fingerprint::compute_from_f32(&samples, 11025, 1).unwrap();
        let s = similarity(&fp, &fp);
        assert!(s > 0.9999, "expected ≈1.0, got {s}");
    }

    #[test]
    fn similarity_of_different_tones_below_dedupe_threshold() {
        // Two pure tones at very different frequencies must not
        // dedupe-merge. The threshold is PRD §8.1's 0.98 floor for
        // auto-merge; we require comfortably below it.
        let a_samples = tone(220.0, 11025, 12.0);
        let b_samples = tone(1760.0, 11025, 12.0);
        let a = Fingerprint::compute_from_f32(&a_samples, 11025, 1).unwrap();
        let b = Fingerprint::compute_from_f32(&b_samples, 11025, 1).unwrap();
        let s = similarity(&a, &b);
        assert!(
            s < 0.98,
            "two distinct tones must not auto-merge (got similarity {s})"
        );
    }

    #[test]
    fn blob_round_trip_preserves_fingerprint() {
        let samples = tone(440.0, 11025, 10.0);
        let fp = Fingerprint::compute_from_f32(&samples, 11025, 1).unwrap();
        let blob = fp.to_blob();
        assert_eq!(blob.len(), fp.len() * 4);
        let restored = Fingerprint::from_blob(&blob, fp.duration_ms()).unwrap();
        assert_eq!(fp, restored);
        assert!(similarity(&fp, &restored) > 0.9999);
    }

    #[test]
    fn from_blob_rejects_truncated_input() {
        // Three bytes is not a multiple of 4 → malformed.
        let err = Fingerprint::from_blob(&[0x12, 0x34, 0x56], 1000).unwrap_err();
        matches!(err, FingerprintError::MalformedBlob { .. });
    }

    #[test]
    fn from_blob_rejects_empty_input() {
        let err = Fingerprint::from_blob(&[], 1000).unwrap_err();
        matches!(err, FingerprintError::MalformedBlob { .. });
    }

    #[test]
    fn similarity_finds_shifted_alignment() {
        // Shift the second copy of the same recording forward by
        // ~1 s (Chromaprint items at default config are ~124 ms,
        // so 8 items ≈ 1 s). The sliding-window similarity must
        // still pin the match near 1.0.
        let samples = tone(330.0, 11025, 12.0);
        let fp = Fingerprint::compute_from_f32(&samples, 11025, 1).unwrap();
        // Build a "shifted" fingerprint by dropping the first 8 items.
        let shifted = Fingerprint {
            items: fp.items[8..].to_vec(),
            duration_ms: fp.duration_ms - 1000,
        };
        let s = similarity(&fp, &shifted);
        assert!(
            s > 0.99,
            "sliding-window similarity must absorb ~1 s shift, got {s}"
        );
    }

    #[test]
    fn similarity_with_empty_fingerprint_is_zero() {
        let samples = tone(440.0, 11025, 10.0);
        let fp = Fingerprint::compute_from_f32(&samples, 11025, 1).unwrap();
        let empty = Fingerprint {
            items: vec![],
            duration_ms: 0,
        };
        assert_eq!(similarity(&fp, &empty), 0.0);
        assert_eq!(similarity(&empty, &fp), 0.0);
    }
}
