//! `Serato Autotags` GEOB decoder (M11e) — BPM + gain.
//!
//! The payload is a short header then three ASCII decimal strings (BPM,
//! auto-gain dB, manual-gain dB), NUL-separated, sometimes base64-wrapped and
//! sometimes raw depending on the Serato version. Rather than pin one exact
//! framing, we decode tolerantly: base64-or-raw, then collect the ASCII
//! float tokens in order (version / separator bytes fall out as delimiters).
//! BPM is redundant with `database V2`'s `tbpm` and the beat grid; the gain
//! is the field worth keeping. Pure + panic-free.

use base64::Engine as _;

/// BPM + gain distilled from a `Serato Autotags` payload.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SeratoAutotags {
    /// Stored BPM, if a positive value was present.
    pub bpm: Option<f64>,
    /// Auto-gain in dB (Serato's loudness-normalisation suggestion).
    pub auto_gain_db: Option<f64>,
    /// Manual gain offset in dB.
    pub gain_db: Option<f64>,
}

/// Decode a `Serato Autotags` GEOB payload.
pub fn parse(payload: &[u8]) -> SeratoAutotags {
    if payload.len() <= 2 {
        return SeratoAutotags::default();
    }
    let body = decode_base64_or_raw(&payload[2..]);
    let floats = float_tokens(&body);
    SeratoAutotags {
        bpm: floats.first().copied().filter(|b| *b > 0.0),
        auto_gain_db: floats.get(1).copied(),
        gain_db: floats.get(2).copied(),
    }
}

/// If the bytes are valid base64, decode them; otherwise return them as-is.
fn decode_base64_or_raw(bytes: &[u8]) -> Vec<u8> {
    let b64: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace() && *b != 0)
        .collect();
    crate::serato::B64
        .decode(&b64)
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| bytes.to_vec())
}

/// Split on NUL + control bytes (which include the version header) and parse
/// each ASCII chunk as an `f64`, preserving order.
fn float_tokens(body: &[u8]) -> Vec<f64> {
    body.split(|&b| b == 0 || b < 0x20)
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_nul_separated() {
        // version(2) + "115.00\0-3.20\00.00\0"
        let mut p = vec![0x01, 0x01];
        p.extend_from_slice(b"115.00\x00-3.20\x000.00\x00");
        let a = parse(&p);
        assert!((a.bpm.unwrap() - 115.0).abs() < 1e-6);
        assert!((a.auto_gain_db.unwrap() + 3.20).abs() < 1e-6);
        assert!((a.gain_db.unwrap()).abs() < 1e-9);
    }

    #[test]
    fn empty_is_default_never_panics() {
        assert_eq!(parse(&[]), SeratoAutotags::default());
        assert_eq!(parse(b"\x01\x01"), SeratoAutotags::default());
    }
}
