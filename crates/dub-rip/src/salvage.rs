//! Reading back a spill that may never have been closed (M26b).
//!
//! `hound` writes the RIFF and `data` chunk sizes when the writer is
//! *finalized*. A rip that ends with a crash, a force-quit, or a
//! pulled power cable never gets there, so the bytes are on disk but
//! the header still claims the length it was created with. A reader
//! that trusts the header sees an empty recording and throws away a
//! side the DJ can't record again without setting the needle back
//! down.
//!
//! So: take the *format* from the header, take the *length* from the
//! file, and use the declared length only when it agrees with what is
//! actually there.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use dub_peaks::{Decimator, PeakChunk, DEFAULT_SAMPLES_PER_CHUNK};

use crate::session::RipError;

/// What a spill turned out to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpillInfo {
    /// Whole interleaved frames actually present in the file.
    pub frames: u64,
    /// Sample rate from the header.
    pub sample_rate: u32,
    /// Channel count from the header.
    pub channels: u16,
    /// True when the header under-reported the data on disk — i.e.
    /// the writer never finalized, i.e. the session was interrupted.
    pub was_unfinalized: bool,
}

const HEADER_PROBE_LIMIT: u64 = 1 << 16;

/// Inspect a capture spill without decoding it.
///
/// # Errors
///
/// [`RipError::SpillUnreadable`] if the file is missing, is not a
/// RIFF/WAVE file, or is not the 32-bit float stereo the record tap
/// writes.
pub fn probe(path: &Path) -> Result<SpillInfo, RipError> {
    let unreadable = |e: String| RipError::SpillUnreadable(e);
    let file = File::open(path).map_err(|e| unreadable(format!("{e}")))?;
    let file_len = file
        .metadata()
        .map_err(|e| unreadable(format!("{e}")))?
        .len();
    let mut reader = BufReader::new(file);

    let mut riff = [0_u8; 12];
    reader
        .read_exact(&mut riff)
        .map_err(|e| unreadable(format!("truncated header: {e}")))?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(unreadable("not a RIFF/WAVE file".into()));
    }

    let mut channels = 0_u16;
    let mut sample_rate = 0_u32;
    let mut bits = 0_u16;
    let mut format_tag = 0_u16;
    let mut pos = 12_u64;

    loop {
        if pos >= file_len || pos >= HEADER_PROBE_LIMIT {
            return Err(unreadable("no data chunk".into()));
        }
        let mut header = [0_u8; 8];
        reader
            .read_exact(&mut header)
            .map_err(|e| unreadable(format!("truncated chunk header: {e}")))?;
        let id = [header[0], header[1], header[2], header[3]];
        let declared = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        pos += 8;

        match &id {
            b"fmt " => {
                let mut fmt = vec![0_u8; declared as usize];
                reader
                    .read_exact(&mut fmt)
                    .map_err(|e| unreadable(format!("truncated fmt chunk: {e}")))?;
                if fmt.len() < 16 {
                    return Err(unreadable("short fmt chunk".into()));
                }
                format_tag = u16::from_le_bytes([fmt[0], fmt[1]]);
                channels = u16::from_le_bytes([fmt[2], fmt[3]]);
                sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
                bits = u16::from_le_bytes([fmt[14], fmt[15]]);
                // WAVE_FORMAT_EXTENSIBLE — which is what `hound`
                // actually writes for 32-bit float — keeps the real
                // format in the first two bytes of the SubFormat GUID.
                if format_tag == 0xFFFE && fmt.len() >= 26 {
                    format_tag = u16::from_le_bytes([fmt[24], fmt[25]]);
                }
                pos += u64::from(declared);
            }
            b"data" => {
                // Everything from here to EOF is audio, whatever the
                // header claims. A finalized file agrees; an
                // interrupted one says 0 (or a stale short length).
                let available = file_len.saturating_sub(pos);
                let declared = u64::from(declared);
                let was_unfinalized = declared == 0 || declared > available;
                let byte_len = if was_unfinalized { available } else { declared };
                if channels == 0 || bits == 0 {
                    return Err(unreadable("data chunk before fmt chunk".into()));
                }
                // WAVE_FORMAT_IEEE_FLOAT (3), or extensible (0xFFFE)
                // carrying float — the tap only ever writes the former.
                if format_tag != 3 || bits != 32 || channels != 2 {
                    return Err(unreadable(format!(
                        "unexpected spill format: {channels} ch, {bits}-bit, tag {format_tag}"
                    )));
                }
                let frame_bytes = u64::from(channels) * u64::from(bits / 8);
                return Ok(SpillInfo {
                    frames: byte_len / frame_bytes,
                    sample_rate,
                    channels,
                    was_unfinalized,
                });
            }
            _ => {
                // Skip any chunk we don't care about (LIST, fact …),
                // honouring the pad byte on odd lengths.
                let skip = u64::from(declared) + u64::from(declared % 2);
                reader
                    .seek(SeekFrom::Current(
                        i64::try_from(skip).map_err(|_| unreadable("absurd chunk size".into()))?,
                    ))
                    .map_err(|e| unreadable(format!("{e}")))?;
                pos += skip;
            }
        }
    }
}

/// Rebuild the live capture envelope by streaming the spill.
///
/// The envelope only ever existed in the capture worker's memory, so a
/// recovered session has to earn it back. Same decimation as the live
/// path (mono downmix, [`DEFAULT_SAMPLES_PER_CHUNK`] frames per
/// chunk), which is what keeps `chunk[i]` ↔ frame `i * 64` true for
/// the split plan.
///
/// # Errors
///
/// [`RipError::SpillUnreadable`] if the file cannot be probed or read.
pub fn rebuild_envelope(path: &Path) -> Result<(Vec<PeakChunk>, SpillInfo), RipError> {
    let info = probe(path)?;
    let mut reader =
        hound::WavReader::open(path).map_err(|e| RipError::SpillUnreadable(format!("{e}")))?;

    let mut envelope = Vec::new();
    let mut decimator = Decimator::new(DEFAULT_SAMPLES_PER_CHUNK);
    let mut mono = Vec::with_capacity(4096);
    let mut pending: Option<f32> = None;
    let mut frames_seen = 0_u64;

    // `samples()` stops at the header's declared length, which is the
    // one thing we don't trust — but it is correct whenever the file
    // was finalized, and for the interrupted case we fall back below.
    for sample in reader.samples::<f32>() {
        let sample = sample.map_err(|e| RipError::SpillUnreadable(format!("{e}")))?;
        match pending.take() {
            None => pending = Some(sample),
            Some(left) => {
                mono.push(f32::midpoint(left, sample));
                frames_seen += 1;
                if mono.len() == mono.capacity() {
                    decimator.feed(&mono, |chunk| envelope.push(chunk));
                    mono.clear();
                }
            }
        }
    }
    if frames_seen < info.frames {
        read_raw_tail(
            path,
            &info,
            frames_seen,
            &mut mono,
            &mut decimator,
            &mut envelope,
        )?;
    }
    if !mono.is_empty() {
        decimator.feed(&mono, |chunk| envelope.push(chunk));
    }
    Ok((envelope, info))
}

/// Read the frames a truncated header hid from `hound`, straight off
/// the file at the known data offset.
fn read_raw_tail(
    path: &Path,
    info: &SpillInfo,
    from_frame: u64,
    mono: &mut Vec<f32>,
    decimator: &mut Decimator,
    envelope: &mut Vec<PeakChunk>,
) -> Result<(), RipError> {
    let unreadable = |e: String| RipError::SpillUnreadable(e);
    let frame_bytes = u64::from(info.channels) * 4;
    let mut file = File::open(path).map_err(|e| unreadable(format!("{e}")))?;
    let data_start = data_offset(path)?;
    file.seek(SeekFrom::Start(data_start + from_frame * frame_bytes))
        .map_err(|e| unreadable(format!("{e}")))?;

    let mut reader = BufReader::new(file);
    let mut frame = vec![0_u8; usize::try_from(frame_bytes).unwrap_or(8)];
    for _ in from_frame..info.frames {
        if reader.read_exact(&mut frame).is_err() {
            break;
        }
        let left = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
        let right = f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
        mono.push(f32::midpoint(left, right));
        if mono.len() == mono.capacity() {
            decimator.feed(mono, |chunk| envelope.push(chunk));
            mono.clear();
        }
    }
    Ok(())
}

/// Byte offset of the `data` chunk's payload.
fn data_offset(path: &Path) -> Result<u64, RipError> {
    let unreadable = |e: String| RipError::SpillUnreadable(e);
    let file = File::open(path).map_err(|e| unreadable(format!("{e}")))?;
    let mut reader = BufReader::new(file);
    let mut riff = [0_u8; 12];
    reader
        .read_exact(&mut riff)
        .map_err(|e| unreadable(format!("{e}")))?;
    let mut pos = 12_u64;
    loop {
        let mut header = [0_u8; 8];
        reader
            .read_exact(&mut header)
            .map_err(|e| unreadable(format!("no data chunk: {e}")))?;
        let declared = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        pos += 8;
        if &header[0..4] == b"data" {
            return Ok(pos);
        }
        let skip = u64::from(declared) + u64::from(declared % 2);
        reader
            .seek(SeekFrom::Current(
                i64::try_from(skip).map_err(|_| unreadable("absurd chunk size".into()))?,
            ))
            .map_err(|e| unreadable(format!("{e}")))?;
        pos += skip;
    }
}

/// Decimate already-decoded interleaved samples into a capture
/// envelope, exactly as the live worker does (mono downmix,
/// [`DEFAULT_SAMPLES_PER_CHUNK`] frames per chunk) so `chunk[i]` still
/// maps to frame `i * 64`. Used when the source is the side archive
/// rather than the spill (M26b re-split).
#[must_use]
pub fn envelope_from_samples(samples: &[f32], channels: u8) -> Vec<PeakChunk> {
    let channels = usize::from(channels.max(1));
    let mut envelope = Vec::new();
    let mut decimator = Decimator::new(DEFAULT_SAMPLES_PER_CHUNK);
    #[allow(clippy::cast_precision_loss)]
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    decimator.feed(&mono, |chunk| envelope.push(chunk));
    envelope
}

/// Read the whole spill as interleaved samples, header length or not.
///
/// The commit path uses this rather than `hound`'s iterator for the
/// same reason [`probe`] exists: a recovered rip's header under-reports
/// the audio, and a commit that believed it would encode silence — or
/// nothing at all — over a perfectly good recording.
///
/// # Errors
///
/// [`RipError::SpillUnreadable`] if the file cannot be probed or read.
pub fn read_all(path: &Path) -> Result<(Vec<f32>, SpillInfo), RipError> {
    let unreadable = |e: String| RipError::SpillUnreadable(e);
    let info = probe(path)?;
    let data_start = data_offset(path)?;
    let mut file = File::open(path).map_err(|e| unreadable(format!("{e}")))?;
    file.seek(SeekFrom::Start(data_start))
        .map_err(|e| unreadable(format!("{e}")))?;

    let sample_count = usize::try_from(info.frames * u64::from(info.channels))
        .map_err(|_| unreadable("spill too large for memory".into()))?;
    let mut bytes = vec![0_u8; sample_count * 4];
    let mut reader = BufReader::new(file);
    reader
        .read_exact(&mut bytes)
        .map_err(|e| unreadable(format!("short read: {e}")))?;

    let samples = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok((samples, info))
}

/// An interrupted session found on disk, as offered to the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverableRip {
    /// Session directory holding `rip.json` + `side.raw.wav`.
    pub session_dir: std::path::PathBuf,
    /// Frames actually present in the spill.
    pub frames: u64,
    /// Capture sample rate.
    pub sample_rate: u32,
    /// True when the spill's header was never finalized — the session
    /// died mid-recording rather than being left at the review screen.
    pub was_interrupted: bool,
}

impl RecoverableRip {
    /// Recorded length in seconds.
    #[must_use]
    pub fn secs(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let secs = self.frames as f64 / f64::from(self.sample_rate);
        secs
    }
}

/// A committed rip, offered back for re-splitting.
///
/// The complement of [`RecoverableRip`]: commit deletes the spill only
/// once every segment has imported, so a session with no spill and a
/// `side.flac` is one that finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResplittableRip {
    /// Session directory holding `rip.json` + `side.flac`.
    pub session_dir: std::path::PathBuf,
    /// Frames in the archived side.
    pub frames: u64,
    /// Capture sample rate.
    pub sample_rate: u32,
    /// Tracks the last split produced.
    pub track_count: u32,
    /// How many times this side has been split (1 = the first commit).
    pub split_generation: u32,
}

/// Committed rips under `parent`, newest first.
///
/// Unlike [`list_recoverable`] this reads the manifest rather than the
/// audio: the archive is a FLAC and decoding one per row to count its
/// frames would make listing a season of ripping cost minutes.
#[must_use]
pub fn list_resplittable(parent: &Path) -> Vec<ResplittableRip> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out: Vec<ResplittableRip> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|dir| dir.is_dir())
        .filter_map(|dir| {
            // A spill still on disk means the rip never finished; that
            // is `list_recoverable`'s business, not this one.
            if dir.join(crate::SPILL_FILE).is_file() {
                return None;
            }
            let manifest = crate::manifest::load(&dir).ok()?;
            let archive = manifest.side_archive.as_ref()?;
            if !dir.join(archive).is_file() {
                return None;
            }
            Some(ResplittableRip {
                session_dir: dir,
                frames: manifest.recorded_frames,
                sample_rate: manifest.sample_rate,
                track_count: u32::try_from(manifest.tracks.len()).unwrap_or(u32::MAX),
                split_generation: manifest.split_generation.max(1),
            })
        })
        .collect();
    // Session dirs are timestamp-named, so name order is time order.
    out.sort_by(|a, b| b.session_dir.cmp(&a.session_dir));
    out
}

/// Find rip sessions under `parent` that still hold un-committed
/// audio, newest directory name first.
///
/// The test is simply *does the spill still exist*: commit deletes it
/// only after every segment imported, so a surviving `side.raw.wav` is
/// exactly the set of rips that never finished. Empty spills (armed,
/// never recorded) are skipped — there is nothing to offer.
#[must_use]
pub fn list_recoverable(parent: &Path) -> Vec<RecoverableRip> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out: Vec<RecoverableRip> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|dir| dir.is_dir() && dir.join(crate::MANIFEST_FILE).is_file())
        .filter_map(|dir| {
            let info = probe(&dir.join(crate::SPILL_FILE)).ok()?;
            (info.frames > 0).then_some(RecoverableRip {
                session_dir: dir,
                frames: info.frames,
                sample_rate: info.sample_rate,
                was_interrupted: info.was_unfinalized,
            })
        })
        .collect();
    // Session dirs are timestamp-named, so name order is time order.
    out.sort_by(|a, b| b.session_dir.cmp(&a.session_dir));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    const SR: u32 = 44_100;

    fn write_spill(path: &Path, frames: u64) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: SR,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / SR as f32;
            let s = 0.5 * (std::f32::consts::TAU * 220.0 * t).sin();
            w.write_sample(s).unwrap();
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    /// Reproduce what a killed process leaves: the audio is all there,
    /// the RIFF and data sizes were never written back.
    fn unfinalize(path: &Path) {
        let mut bytes = std::fs::read(path).unwrap();
        // RIFF size at offset 4, data size in the 8 bytes before the
        // payload (this file has exactly one fmt chunk before it).
        bytes[4..8].copy_from_slice(&0_u32.to_le_bytes());
        let data_at = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("data chunk");
        bytes[data_at + 4..data_at + 8].copy_from_slice(&0_u32.to_le_bytes());
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&bytes).unwrap();
    }

    #[test]
    fn probe_reads_a_finalized_spill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("side.raw.wav");
        write_spill(&path, 5_000);

        let info = probe(&path).unwrap();
        assert_eq!(info.frames, 5_000);
        assert_eq!(info.sample_rate, SR);
        assert_eq!(info.channels, 2);
        assert!(!info.was_unfinalized);
    }

    #[test]
    fn probe_salvages_an_interrupted_spill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("side.raw.wav");
        write_spill(&path, 5_000);
        unfinalize(&path);

        let info = probe(&path).unwrap();
        assert_eq!(
            info.frames, 5_000,
            "a crash must not cost the audio already on disk"
        );
        assert!(info.was_unfinalized);
    }

    #[test]
    fn rebuild_envelope_matches_the_recording_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("side.raw.wav");
        write_spill(&path, 64 * 100);

        let (envelope, info) = rebuild_envelope(&path).unwrap();
        assert_eq!(info.frames, 6_400);
        assert_eq!(envelope.len(), 100, "one chunk per 64 frames");
        assert!(envelope.iter().all(|c| c.rms > 0.0));
    }

    #[test]
    fn rebuild_envelope_recovers_an_interrupted_spill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("side.raw.wav");
        write_spill(&path, 64 * 100);
        unfinalize(&path);

        let (envelope, info) = rebuild_envelope(&path).unwrap();
        assert_eq!(info.frames, 6_400);
        assert_eq!(envelope.len(), 100);
        assert!(envelope.iter().all(|c| c.rms > 0.0));
    }

    #[test]
    fn probe_rejects_what_is_not_a_spill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.wav");
        std::fs::write(&path, b"this is not a wav file at all").unwrap();
        assert!(probe(&path).is_err());
        assert!(probe(&dir.path().join("missing.wav")).is_err());
    }
    /// The two listings are complements: an unfinished rip belongs to
    /// `list_recoverable`, a committed one to `list_resplittable`, and
    /// neither should ever show the same session.
    #[test]
    fn resplittable_lists_committed_sessions_only() {
        let root = tempfile::tempdir().unwrap();

        let commit = |name: &str, tracks: usize, archive: bool| {
            let dir = root.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let mut m = crate::manifest::RipManifest::new(SR, 2);
            m.recorded_frames = 60 * u64::from(SR);
            m.tracks = vec![crate::manifest::TrackEntry::default(); tracks];
            if archive {
                m.side_archive = Some(crate::ARCHIVE_FILE.to_string());
                std::fs::write(dir.join(crate::ARCHIVE_FILE), b"not really flac").unwrap();
            }
            crate::manifest::save(&dir, &m).unwrap();
            dir
        };

        commit("20260101-120000", 3, true);
        let unfinished = commit("20260102-120000", 1, true);
        write_spill(&unfinished.join(crate::SPILL_FILE), 1000);
        commit("20260103-120000", 2, false); // committed but no archive
        commit("20260104-120000", 4, true);

        let found = list_resplittable(root.path());
        let names: Vec<String> = found
            .iter()
            .map(|r| {
                r.session_dir
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec!["20260104-120000", "20260101-120000"],
            "expected the two committed-with-archive sessions, newest first"
        );
        assert_eq!(found[0].track_count, 4);
        assert_eq!(found[1].track_count, 3);

        // And the unfinished one is the other listing's business.
        let recoverable = list_recoverable(root.path());
        assert_eq!(recoverable.len(), 1);
        assert!(recoverable[0].session_dir.ends_with("20260102-120000"));
    }
}
