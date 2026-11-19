//! Where Traktor's and Serato's MP3 timelines start.
//!
//! An encoder puts a header frame at the head of most MP3s (`Xing` / `Info`
//! from LAME and ffmpeg, `VBRI` from Fraunhofer). It holds no audio. Dub's
//! decoder skips it; Traktor plays it as a frame of silence, so every
//! position Traktor stores for such a file is one frame (1,152 samples,
//! 26.1 ms at 44.1 kHz) later than the same moment in Dub. Measured on
//! Daniel's library (2026-11-18): Traktor's grid sat +27 ms from Dub's —
//! his own set-the-1 taps included — on MP3s with the frame, within ±4 ms
//! on those without it and on AIFF.
//!
//! Serato plays the header frame too, and also drops the decoder's own
//! 529-sample delay, which Dub keeps. Measured against Traktor's corrected
//! grids on 1,454 of Daniel's MP3s carrying both: Serato −16.9 ms (median)
//! on files without the frame, +11.4 ms on files with it; this model says
//! −12.0 and +14.1. Mixxx corrects Serato MP3 positions by the same 12 ms
//! in its simplest case (`SeratoTags::guessTimingOffsetMillis`).

/// The MP3 decoder delay: samples every standard Layer III decoder emits
/// before the encoder's first sample.
const DECODER_DELAY_SAMPLES: f64 = 529.0;

use std::io::Read;
use std::path::Path;

/// How far a frame-skipping decoder's timeline leads Traktor's for this
/// file, in seconds: one frame for an MP3 that opens with a header frame,
/// 0 for anything else (including a file that can't be read).
pub(crate) fn traktor_lead_secs(path: &Path) -> f64 {
    first_frame_of(path).map_or(0.0, |f| f.header_secs)
}

/// The same for Serato: the header frame, less the decoder delay Serato
/// drops. Negative on an MP3 without a header frame — there Serato's
/// positions are *earlier* than Dub's.
pub(crate) fn serato_lead_secs(path: &Path) -> f64 {
    first_frame_of(path).map_or(0.0, |f| {
        f.header_secs - DECODER_DELAY_SAMPLES / f64::from(f.sample_rate)
    })
}

fn first_frame_of(path: &Path) -> Option<FirstFrame> {
    let is_mp3 = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    if !is_mp3 {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    // An ID3 tag with artwork and Traktor's own frame runs to a few hundred
    // KB; read past it and a little more.
    file.take(4 << 20).read_to_end(&mut head).ok()?;
    first_frame(&head)
}

struct FirstFrame {
    /// The header frame's duration; 0 when the first frame is audio.
    header_secs: f64,
    sample_rate: u32,
}

/// The duration of the header frame `bytes` opens with (after any ID3v2
/// tag), or `None` when the first frame is audio.
#[cfg(test)]
fn header_frame_secs(bytes: &[u8]) -> Option<f64> {
    first_frame(bytes)
        .map(|f| f.header_secs)
        .filter(|s| *s > 0.0)
}

fn first_frame(bytes: &[u8]) -> Option<FirstFrame> {
    let start = id3v2_len(bytes);
    // Some files pad between the tag and the first frame; look a little way.
    let window = bytes.get(start..)?;
    let at = (0..window.len().saturating_sub(4).min(64 * 1024))
        .find(|&i| window[i] == 0xFF && window[i + 1] & 0xE0 == 0xE0)?;
    let frame = &window[at..];
    let h = Header::parse(frame)?;
    let has = |offset: usize, tag: &[u8]| frame.get(offset..offset + 4) == Some(tag);
    let xing = h.xing_offset();
    let is_header = has(xing, b"Xing") || has(xing, b"Info") || has(36, b"VBRI");
    let header_secs = if is_header {
        f64::from(h.samples_per_frame) / f64::from(h.sample_rate)
    } else {
        0.0
    };
    Some(FirstFrame {
        header_secs,
        sample_rate: h.sample_rate,
    })
}

fn id3v2_len(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    let size = bytes[6..10]
        .iter()
        .fold(0usize, |acc, b| (acc << 7) | usize::from(b & 0x7F));
    let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

struct Header {
    mpeg1: bool,
    mono: bool,
    sample_rate: u32,
    samples_per_frame: u32,
}

impl Header {
    /// Layer III only: the layer every DJ MP3 is.
    fn parse(b: &[u8]) -> Option<Self> {
        let b1 = *b.get(1)?;
        let b2 = *b.get(2)?;
        let b3 = *b.get(3)?;
        let version = (b1 >> 3) & 0b11; // 3 = MPEG 1, 2 = MPEG 2, 0 = MPEG 2.5
        let layer = (b1 >> 1) & 0b11; // 1 = Layer III
        if version == 1 || layer != 1 {
            return None;
        }
        let rate_index = usize::from((b2 >> 2) & 0b11);
        let base = [44_100u32, 48_000, 32_000].get(rate_index).copied()?;
        let sample_rate = match version {
            3 => base,
            2 => base / 2,
            _ => base / 4,
        };
        let mpeg1 = version == 3;
        Some(Self {
            mpeg1,
            mono: (b3 >> 6) == 0b11,
            sample_rate,
            samples_per_frame: if mpeg1 { 1152 } else { 576 },
        })
    }

    /// Where `Xing` / `Info` sits: after the 4-byte header and the side
    /// information, whose size depends on version and channels.
    fn xing_offset(&self) -> usize {
        4 + match (self.mpeg1, self.mono) {
            (true, false) => 32,
            (true, true) | (false, false) => 17,
            (false, true) => 9,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MPEG-1 Layer III, 128 kbps, 44.1 kHz, joint stereo.
    const FRAME_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x64];

    fn mp3(tag_len: usize, marker: Option<(usize, &[u8])>) -> Vec<u8> {
        let mut out = Vec::new();
        if tag_len > 0 {
            out.extend_from_slice(b"ID3\x04\x00\x00");
            let s = u32::try_from(tag_len).unwrap();
            out.extend([
                (s >> 21) as u8 & 0x7F,
                (s >> 14) as u8 & 0x7F,
                (s >> 7) as u8 & 0x7F,
                s as u8 & 0x7F,
            ]);
            out.resize(10 + tag_len, 0);
        }
        let mut frame = FRAME_HEADER.to_vec();
        frame.resize(417, 0);
        if let Some((at, tag)) = marker {
            frame[at..at + 4].copy_from_slice(tag);
        }
        out.extend(frame);
        out
    }

    #[test]
    fn a_lame_header_frame_is_one_frame_of_lead() {
        let secs = header_frame_secs(&mp3(300, Some((36, b"Info")))).unwrap();
        assert!((secs - 1152.0 / 44_100.0).abs() < 1e-12);
        assert!(header_frame_secs(&mp3(0, Some((36, b"Xing")))).is_some());
        assert!(header_frame_secs(&mp3(0, Some((36, b"VBRI")))).is_some());
    }

    #[test]
    fn an_audio_first_frame_is_no_lead() {
        assert_eq!(header_frame_secs(&mp3(300, None)), None);
        // Xing at the wrong offset for this header is audio that happens
        // to contain the bytes.
        assert_eq!(header_frame_secs(&mp3(0, Some((100, b"Xing")))), None);
    }

    #[test]
    fn not_an_mp3_is_no_lead() {
        assert_eq!(header_frame_secs(b""), None);
        assert_eq!(header_frame_secs(b"FORM....AIFF"), None);
        assert_eq!(traktor_lead_secs(Path::new("/nonexistent/a.aiff")), 0.0);
        assert_eq!(serato_lead_secs(Path::new("/nonexistent/a.aiff")), 0.0);
    }

    #[test]
    fn serato_drops_the_decoder_delay_on_top() {
        let dir = tempfile::tempdir().unwrap();
        let with = dir.path().join("with.mp3");
        let without = dir.path().join("without.mp3");
        std::fs::write(&with, mp3(300, Some((36, b"Info")))).unwrap();
        std::fs::write(&without, mp3(300, None)).unwrap();
        let delay = 529.0 / 44_100.0;
        let frame = 1152.0 / 44_100.0;
        assert!((serato_lead_secs(&with) - (frame - delay)).abs() < 1e-12);
        assert!((serato_lead_secs(&without) + delay).abs() < 1e-12);
        assert!((traktor_lead_secs(&with) - frame).abs() < 1e-12);
        assert_eq!(traktor_lead_secs(&without), 0.0);
    }
}
