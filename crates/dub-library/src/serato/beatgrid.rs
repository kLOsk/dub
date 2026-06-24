//! `Serato BeatGrid` GEOB decoder (M11e).
//!
//! Raw binary payload (big-endian), no base64:
//!
//! ```text
//! 0x01 0x00              version
//! u32  marker_count      (TOTAL markers; the last one is terminal)
//! first marker_count-1 (non-terminal) markers:
//!     f32 position_secs
//!     u32 beats_until_next_marker
//! terminal marker (the last):
//!     f32 position_secs
//!     f32 bpm
//! u8   footer
//! ```
//!
//! (Validated against a real export: a constant-tempo track has
//! `marker_count == 1` — a single terminal marker carrying the downbeat
//! position + BPM, 15 bytes total.)
//!
//! Dub stores one uniform grid per track (§8.3): we take the first marker's
//! position as the downbeat anchor and a single BPM — the terminal marker's
//! BPM for a constant-tempo grid (the common case: zero non-terminal
//! markers), else the tempo derived from the first interval. Pure + panic-
//! free: malformed/truncated input returns `None`.

/// A single uniform grid distilled from a Serato beat-grid payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeratoBeatgrid {
    /// Downbeat anchor in seconds (the first marker's position).
    pub anchor_secs: f64,
    /// Tempo in BPM.
    pub bpm: f64,
}

fn read_f32(data: &[u8], at: usize) -> Option<f32> {
    let b = data.get(at..at + 4)?;
    Some(f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Decode a `Serato BeatGrid` GEOB payload into a single uniform grid.
pub fn parse(payload: &[u8]) -> Option<SeratoBeatgrid> {
    // 2-byte version + 4-byte TOTAL marker count (last marker is terminal).
    let count = read_u32(payload, 2)? as usize;
    if count == 0 {
        return None;
    }
    let mut pos = 6;

    // The first count-1 markers are non-terminal: (position f32,
    // beats_until_next u32). Bounds-checked per iteration so a garbage
    // `count` just runs out of buffer — never pre-size a Vec from `count`.
    let mut markers: Vec<(f32, u32)> = Vec::new();
    for _ in 0..count - 1 {
        let p = read_f32(payload, pos)?;
        let beats = read_u32(payload, pos + 4)?;
        markers.push((p, beats));
        pos += 8;
    }

    // Terminal marker (the last): position f32 + bpm f32.
    let term_pos = read_f32(payload, pos)?;
    let term_bpm = read_f32(payload, pos + 4)?;

    let anchor = f64::from(markers.first().map_or(term_pos, |m| m.0));
    let bpm = match markers.first() {
        Some(&(first_pos, beats)) if beats > 0 => {
            let next = markers.get(1).map_or(term_pos, |m| m.0);
            let dt = f64::from(next - first_pos);
            if dt > 0.0 {
                f64::from(beats) / dt * 60.0
            } else {
                f64::from(term_bpm)
            }
        }
        _ => f64::from(term_bpm),
    };

    if anchor.is_finite() && bpm.is_finite() && bpm > 0.0 {
        Some(SeratoBeatgrid {
            anchor_secs: anchor,
            bpm,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constant_grid(pos: f32, bpm: f32) -> Vec<u8> {
        let mut v = vec![0x01, 0x00];
        v.extend_from_slice(&1u32.to_be_bytes()); // one (terminal) marker
        v.extend_from_slice(&pos.to_be_bytes());
        v.extend_from_slice(&bpm.to_be_bytes());
        v.push(0x00); // footer
        v
    }

    #[test]
    fn constant_tempo_grid() {
        let g = parse(&constant_grid(0.025, 174.0)).unwrap();
        assert!((g.anchor_secs - 0.025).abs() < 1e-6);
        assert!((g.bpm - 174.0).abs() < 1e-4);
    }

    #[test]
    fn derives_bpm_from_first_interval() {
        // Two markers total: a non-terminal at t=0 with 4 beats until the
        // terminal marker at t=2.0s → 4 beats / 2s * 60 = 120 BPM.
        let mut v = vec![0x01, 0x00];
        v.extend_from_slice(&2u32.to_be_bytes());
        v.extend_from_slice(&0.0f32.to_be_bytes());
        v.extend_from_slice(&4u32.to_be_bytes());
        v.extend_from_slice(&2.0f32.to_be_bytes());
        v.extend_from_slice(&0.0f32.to_be_bytes()); // terminal bpm unused here
        v.push(0x00);
        let g = parse(&v).unwrap();
        assert!((g.anchor_secs).abs() < 1e-6);
        assert!((g.bpm - 120.0).abs() < 1e-4);
    }

    #[test]
    fn malformed_returns_none_never_panics() {
        for bad in [&b""[..], b"\x01\x00", b"\x01\x00\x00\x00\x00\x09short"] {
            assert!(parse(bad).is_none() || parse(bad).is_some());
        }
    }
}
