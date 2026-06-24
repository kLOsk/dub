//! `Serato Markers2` GEOB decoder (M11e) — hot cues + saved loops.
//!
//! The GEOB payload is a 2-byte version (`0x01 0x01`) followed by a base64
//! blob (often newline-wrapped, NUL-padded). The decoded blob is itself a
//! 2-byte version then a sequence of NUL-terminated-name entries:
//!
//! ```text
//! name\0   u32 body_len   body[body_len]
//! ```
//!
//! Entry bodies we decode (others — `COLOR`, `BPMLOCK`, `FLIP` — are skipped):
//! * `CUE`  : `00 index(1) position_be_u32_ms 00 color(3) 00 00 name\0`
//! * `LOOP` : `00 index(1) start_be_u32_ms end_be_u32_ms …trailer… name\0`
//!
//! Positions are **milliseconds**. Pure + panic-free; any framing error stops
//! the scan. Offsets here are validated against a real export by the
//! adapter's opt-in probe.

use base64::Engine as _;

/// A Serato hot cue.
#[derive(Debug, Clone, PartialEq)]
pub struct SeratoCue {
    /// Pad / hot-cue slot index.
    pub index: u8,
    /// Cue position in milliseconds.
    pub position_ms: u32,
    /// Cue label, if any.
    pub name: Option<String>,
}

/// A Serato saved loop.
#[derive(Debug, Clone, PartialEq)]
pub struct SeratoLoop {
    /// Loop slot index.
    pub index: u8,
    /// Loop start in milliseconds.
    pub start_ms: u32,
    /// Loop end in milliseconds.
    pub end_ms: u32,
    /// Loop label, if any.
    pub name: Option<String>,
}

/// Everything `Serato Markers2` carries that Dub imports.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SeratoMarkers {
    /// Hot cues, in stored order.
    pub cues: Vec<SeratoCue>,
    /// Saved loops, in stored order.
    pub loops: Vec<SeratoLoop>,
}

/// Decode a `Serato Markers2` GEOB payload (the bytes of the GEOB frame,
/// including the leading `0x01 0x01` version).
pub fn parse(payload: &[u8]) -> SeratoMarkers {
    let Some(decoded) = decode_base64_blob(payload) else {
        return SeratoMarkers::default();
    };
    parse_decoded(&decoded)
}

/// Strip the 2-byte version header, gather the base64 ASCII (ignoring
/// whitespace and trailing NULs), and decode it. `None` if there's nothing
/// decodable.
fn decode_base64_blob(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() <= 2 {
        return None;
    }
    let b64: Vec<u8> = payload[2..]
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace() && *b != 0)
        .collect();
    crate::serato::B64
        .decode(&b64)
        .ok()
        .filter(|d| !d.is_empty())
}

fn parse_decoded(data: &[u8]) -> SeratoMarkers {
    let mut out = SeratoMarkers::default();
    // Skip the inner 2-byte version header.
    let mut pos = 2usize.min(data.len());
    while pos < data.len() {
        // NUL-terminated entry name.
        let name_start = pos;
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            break;
        }
        let name = &data[name_start..pos];
        pos += 1; // skip NUL
        if name.is_empty() {
            break; // list terminator
        }
        // u32 body length.
        let Some(len_bytes) = data.get(pos..pos + 4) else {
            break;
        };
        let len =
            u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
        pos += 4;
        let Some(body) = data.get(pos..pos + len) else {
            break;
        };
        pos += len;
        match name {
            b"CUE" => {
                if let Some(cue) = parse_cue(body) {
                    out.cues.push(cue);
                }
            }
            b"LOOP" => {
                if let Some(lp) = parse_loop(body) {
                    out.loops.push(lp);
                }
            }
            _ => {}
        }
    }
    out
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read a NUL-terminated UTF-8 string starting at `at`; `None` if empty.
fn cstr(b: &[u8], at: usize) -> Option<String> {
    let slice = b.get(at..)?;
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    let s = String::from_utf8_lossy(&slice[..end]);
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// `00 index(1) position(4) 00 color(3) 00 00 name\0`
fn parse_cue(body: &[u8]) -> Option<SeratoCue> {
    let index = *body.get(1)?;
    let position_ms = be_u32(body, 2)?;
    let name = cstr(body, 12);
    Some(SeratoCue {
        index,
        position_ms,
        name,
    })
}

/// `00 index(1) start(4) end(4) …trailer… name\0`. Index / start / end are
/// well-defined; the trailer (loop color + lock flag) varies by version, so
/// the name is read as the trailing C-string from a conservative offset.
fn parse_loop(body: &[u8]) -> Option<SeratoLoop> {
    let index = *body.get(1)?;
    let start_ms = be_u32(body, 2)?;
    let end_ms = be_u32(body, 6)?;
    // Fixed prefix through end_ms is 10 bytes; the loop trailer is
    // ff ff ff ff 00 + 4-byte colour + 1-byte lock = 10 bytes, then the
    // NUL-terminated name. Fall back to scanning if the body is shorter.
    let name = cstr(body, 20).or_else(|| cstr(body, 10));
    Some(SeratoLoop {
        index,
        start_ms,
        end_ms,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap a decoded inner blob back into a GEOB payload (version + base64).
    fn wrap(inner: &[u8]) -> Vec<u8> {
        let b64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(inner);
        let mut out = vec![0x01, 0x01];
        out.extend_from_slice(b64.as_bytes());
        out
    }

    fn cue_entry(index: u8, ms: u32, name: &str) -> Vec<u8> {
        let mut body = vec![0x00, index];
        body.extend_from_slice(&ms.to_be_bytes());
        body.push(0x00);
        body.extend_from_slice(&[0xCC, 0x00, 0x00]); // colour
        body.extend_from_slice(&[0x00, 0x00]);
        body.extend_from_slice(name.as_bytes());
        body.push(0x00);
        let mut entry = b"CUE\0".to_vec();
        entry.extend_from_slice(&(body.len() as u32).to_be_bytes());
        entry.extend_from_slice(&body);
        entry
    }

    #[test]
    fn parses_cues() {
        let mut inner = vec![0x01, 0x01];
        inner.extend(cue_entry(0, 16000, "Intro"));
        inner.extend(cue_entry(3, 64000, "Drop"));
        inner.push(0x00); // terminator
        let m = parse(&wrap(&inner));
        assert_eq!(m.cues.len(), 2);
        assert_eq!(m.cues[0].index, 0);
        assert_eq!(m.cues[0].position_ms, 16000);
        assert_eq!(m.cues[0].name.as_deref(), Some("Intro"));
        assert_eq!(m.cues[1].index, 3);
        assert_eq!(m.cues[1].name.as_deref(), Some("Drop"));
    }

    #[test]
    fn empty_or_garbage_is_empty_not_panic() {
        assert_eq!(parse(&[]), SeratoMarkers::default());
        assert_eq!(parse(b"\x01\x01"), SeratoMarkers::default());
        assert_eq!(parse(b"\x01\x01!!!!notbase64"), SeratoMarkers::default());
    }
}
