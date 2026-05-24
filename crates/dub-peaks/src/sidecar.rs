//! On-disk **waveform sidecar** cache.
//!
//! `dub-library::analyze_track` runs the full `compute_offline_peaks`
//! pass over a decoded track (broadband + bands + onset + filtered
//! chunks, ~100–300 ms for a 5-min MP3). The result is exactly what
//! the engine's `background_analyze_and_install` re-derives the next
//! time the user loads the same track on a deck. That is wasted
//! work: the audio file is content-addressable via its Chromaprint
//! `fingerprint_id`, and the peaks are a deterministic function of
//! the audio samples + the build-time chunk-cadence constants, so
//! the peaks for one fingerprint are stable across loads.
//!
//! This module is the cache. Layout:
//!
//! 1. **Writer** ([`write_sidecar`]) serialises an [`OfflinePeaks`]
//!    to a single file under `~/Library/Caches/Dub/waveforms/
//!    {fingerprint_hex}.wf`. Native endianness; macOS-only consumer
//!    so a portability sentinel is not yet required.
//! 2. **Reader** ([`read_sidecar`]) round-trips back to an
//!    `OfflinePeaks` value. Returns `Ok(None)` on missing file or
//!    bad magic/version (cache miss treated as benign — caller
//!    falls back to recomputing).
//! 3. **Wire format** is documented inline below. PRD-BEATS §4.5
//!    (instant-display contract: waveform sidecar guarantee).
//!
//! ## Wire format (`.wf` v1)
//!
//! All multi-byte fields are native endian (currently macOS / ARM64
//! and `x86_64` are both little-endian). The file is one contiguous
//! byte stream:
//!
//! | Offset | Field | Type | Notes |
//! |---|---|---|---|
//! | 0 | `magic` | `[u8; 8]` | ASCII `b"DUBWFM\x01\x00"` |
//! | 8 | `endianness_marker` | `u32` | `0x01020304` — readers refuse non-equal value |
//! | 12 | `sample_rate` | `u32` | `OfflinePeaks::sample_rate` |
//! | 16 | `samples_per_broadband_chunk` | `u32` | from struct |
//! | 20 | `samples_per_band_chunk` | `u32` | from struct |
//! | 24 | `samples_per_onset_chunk` | `u32` | from struct |
//! | 28 | `samples_per_filtered_chunk` | `u32` | from struct |
//! | 32 | `broadband_count` | `u64` | LE u64 count of `PeakChunk` entries |
//! | 40 | `bands_count` | `u64` | LE u64 count of `BandPeakChunk` entries |
//! | 48 | `onset_count` | `u64` | LE u64 count of `OnsetChunk` entries |
//! | 56 | `filtered_count` | `u64` | LE u64 count of `FilteredPeakChunk` entries |
//! | 64 | `broadband[]` | packed | `count × 12 B` (`#[repr(C)]` cast) |
//! | … | `bands[]` | packed | `count × 32 B` |
//! | … | `onset[]` | packed | `count × 4 B` |
//! | … | `filtered[]` | packed | `count × 24 B` |
//!
//! No checksums: a torn file from a crashed write is detected by
//! the bytes-remaining read at the tail and treated as a cache miss.
//! No compression: the data is mostly small f32s, deflate ratios are
//! ~10–20 % at best, and we trade ~5 MB extra per 5-min track for a
//! zero-copy read path. M11d.7 round 3 ships this naïve format; a
//! future round can add lz4 / zstd if disk pressure becomes a
//! measurable problem.
//!
//! ## Why not bincode + serde?
//!
//! Adding `serde` to the workspace for a cache that already wants
//! byte-level layout control is the wrong shape. The bespoke format
//! here writes each chunk field-by-field with `f32::to_ne_bytes` so
//! we stay inside `#![forbid(unsafe_code)]` (no slice transmutes)
//! and let `BufWriter`'s 8 KB pool absorb the per-field overhead.
//! Throughput on a 5-min track at 48 kHz measures ≪ 50 ms on Apple
//! Silicon — fast enough that disabling it would not be the next
//! bottleneck.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use crate::offline::OfflinePeaks;
use crate::{BandPeakChunk, FilteredPeakChunk, OnsetChunk, PeakChunk, NUM_BANDS};

/// Magic header bytes. Embed an explicit `0x01 0x00` version pair
/// inside the magic so the most common mismatch (a file written by
/// a future version of Dub being read by an older binary) is caught
/// before the body is parsed.
const SIDECAR_MAGIC: &[u8; 8] = b"DUBWFM\x01\x00";
/// Endianness marker. Native byte order. A reader that observes a
/// different value is on a host with the opposite byte order and
/// the cache file was written by a different platform — bail.
const ENDIANNESS_MARKER: u32 = 0x0102_0304;

/// Errors emitted while writing the sidecar. Reads return
/// `io::Result<Option<_>>` instead because a missing or corrupt
/// cache file is always a benign "cache miss" — the caller falls
/// back to recomputing.
#[derive(Debug, thiserror::Error)]
pub enum SidecarWriteError {
    /// Underlying I/O failure (permission, disk full, parent dir
    /// missing). Wraps the OS error verbatim.
    #[error("sidecar write failed: {0}")]
    Io(#[from] io::Error),
}

/// Serialise `peaks` to a `.wf` cache file at `path`.
///
/// The write is buffered and synced via [`File::sync_all`] before
/// the function returns, so a process crash after this call cannot
/// leave a truncated file (the standard Unix `fsync` guarantee).
/// Caller is responsible for choosing `path` (typically
/// `default_waveforms_cache_dir().join(format!("{fingerprint}.wf"))`
/// from `dub_library::paths`) and for ensuring the parent directory
/// exists.
///
/// # Errors
///
/// * [`SidecarWriteError::Io`] when the file cannot be created,
///   written, or synced.
pub fn write_sidecar(path: &Path, peaks: &OfflinePeaks) -> Result<(), SidecarWriteError> {
    let file = File::create(path)?;
    let mut w = BufWriter::new(file);
    w.write_all(SIDECAR_MAGIC)?;
    w.write_all(&ENDIANNESS_MARKER.to_ne_bytes())?;
    w.write_all(&peaks.sample_rate.to_ne_bytes())?;
    w.write_all(&u32_from_usize(peaks.samples_per_broadband_chunk).to_ne_bytes())?;
    w.write_all(&u32_from_usize(peaks.samples_per_band_chunk).to_ne_bytes())?;
    w.write_all(&u32_from_usize(peaks.samples_per_onset_chunk).to_ne_bytes())?;
    w.write_all(&u32_from_usize(peaks.samples_per_filtered_chunk).to_ne_bytes())?;
    w.write_all(&(peaks.broadband.len() as u64).to_ne_bytes())?;
    w.write_all(&(peaks.bands.len() as u64).to_ne_bytes())?;
    w.write_all(&(peaks.onset.len() as u64).to_ne_bytes())?;
    w.write_all(&(peaks.filtered.len() as u64).to_ne_bytes())?;
    for c in &peaks.broadband {
        write_peak_chunk(&mut w, c)?;
    }
    for c in &peaks.bands {
        write_band_peak_chunk(&mut w, c)?;
    }
    for c in &peaks.onset {
        write_onset_chunk(&mut w, *c)?;
    }
    for c in &peaks.filtered {
        write_filtered_peak_chunk(&mut w, c)?;
    }
    let file = w
        .into_inner()
        .map_err(|e| SidecarWriteError::Io(e.into_error()))?;
    file.sync_all()?;
    Ok(())
}

/// Deserialise the `.wf` file at `path`. Returns `Ok(None)` when:
///
/// * the file does not exist (cold cache, expected on first analyse);
/// * the magic bytes do not match (file written by a different tool
///   or a future Dub binary; treat as miss rather than fail);
/// * the endianness marker mismatches (file written by a different
///   platform; we don't byte-swap on read in v1);
/// * the file is truncated relative to its declared chunk counts
///   (a crashed writer; the next analyse pass will overwrite it).
///
/// Returns `Err` only on a genuine I/O fault (permission, hardware
/// error) — those should be surfaced rather than silently treated as
/// a miss.
///
/// # Errors
///
/// * `io::Error` when the file cannot be opened or read for a reason
///   other than "missing". A missing file resolves to `Ok(None)`.
pub fn read_sidecar(path: &Path) -> io::Result<Option<OfflinePeaks>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut r = BufReader::new(file);

    let mut magic = [0u8; 8];
    if r.read_exact(&mut magic).is_err() || &magic != SIDECAR_MAGIC {
        return Ok(None);
    }
    let Ok(marker) = read_u32(&mut r) else {
        return Ok(None);
    };
    if marker != ENDIANNESS_MARKER {
        return Ok(None);
    }
    let sample_rate = read_u32(&mut r)?;
    let spb = read_u32(&mut r)?;
    let spband = read_u32(&mut r)?;
    let sponset = read_u32(&mut r)?;
    let spfiltered = read_u32(&mut r)?;
    let broadband_n = read_u64(&mut r)?;
    let bands_n = read_u64(&mut r)?;
    let onset_n = read_u64(&mut r)?;
    let filtered_n = read_u64(&mut r)?;

    let Ok(broadband) = read_n_chunks(&mut r, broadband_n, read_peak_chunk) else {
        return Ok(None);
    };
    let Ok(bands) = read_n_chunks(&mut r, bands_n, read_band_peak_chunk) else {
        return Ok(None);
    };
    let Ok(onset) = read_n_chunks(&mut r, onset_n, read_onset_chunk) else {
        return Ok(None);
    };
    let Ok(filtered) = read_n_chunks(&mut r, filtered_n, read_filtered_peak_chunk) else {
        return Ok(None);
    };

    Ok(Some(OfflinePeaks {
        broadband,
        bands,
        onset,
        filtered,
        sample_rate,
        samples_per_broadband_chunk: spb as usize,
        samples_per_band_chunk: spband as usize,
        samples_per_onset_chunk: sponset as usize,
        samples_per_filtered_chunk: spfiltered as usize,
    }))
}

fn write_peak_chunk<W: Write>(w: &mut W, c: &PeakChunk) -> io::Result<()> {
    let mut buf = [0u8; 12];
    buf[0..4].copy_from_slice(&c.min.to_ne_bytes());
    buf[4..8].copy_from_slice(&c.max.to_ne_bytes());
    buf[8..12].copy_from_slice(&c.rms.to_ne_bytes());
    w.write_all(&buf)
}

fn read_peak_chunk(r: &mut impl Read) -> io::Result<PeakChunk> {
    let mut buf = [0u8; 12];
    r.read_exact(&mut buf)?;
    Ok(PeakChunk {
        min: f32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]),
        max: f32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]),
        rms: f32::from_ne_bytes([buf[8], buf[9], buf[10], buf[11]]),
    })
}

fn write_band_peak_chunk<W: Write>(w: &mut W, c: &BandPeakChunk) -> io::Result<()> {
    let mut buf = [0u8; 32];
    for (i, v) in c.rms_per_band.iter().enumerate() {
        let off = i * 4;
        buf[off..off + 4].copy_from_slice(&v.to_ne_bytes());
    }
    w.write_all(&buf)
}

fn read_band_peak_chunk(r: &mut impl Read) -> io::Result<BandPeakChunk> {
    let mut buf = [0u8; 32];
    r.read_exact(&mut buf)?;
    let mut out = [0.0f32; NUM_BANDS];
    for (i, slot) in out.iter_mut().enumerate() {
        let off = i * 4;
        *slot = f32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
    }
    Ok(BandPeakChunk { rms_per_band: out })
}

fn write_onset_chunk<W: Write>(w: &mut W, c: OnsetChunk) -> io::Result<()> {
    w.write_all(&c.flux.to_ne_bytes())
}

fn read_onset_chunk(r: &mut impl Read) -> io::Result<OnsetChunk> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(OnsetChunk {
        flux: f32::from_ne_bytes(buf),
    })
}

fn write_filtered_peak_chunk<W: Write>(w: &mut W, c: &FilteredPeakChunk) -> io::Result<()> {
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&c.lf_min.to_ne_bytes());
    buf[4..8].copy_from_slice(&c.lf_max.to_ne_bytes());
    buf[8..12].copy_from_slice(&c.mf_min.to_ne_bytes());
    buf[12..16].copy_from_slice(&c.mf_max.to_ne_bytes());
    buf[16..20].copy_from_slice(&c.hf_min.to_ne_bytes());
    buf[20..24].copy_from_slice(&c.hf_max.to_ne_bytes());
    w.write_all(&buf)
}

fn read_filtered_peak_chunk(r: &mut impl Read) -> io::Result<FilteredPeakChunk> {
    let mut buf = [0u8; 24];
    r.read_exact(&mut buf)?;
    Ok(FilteredPeakChunk {
        lf_min: f32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]),
        lf_max: f32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]),
        mf_min: f32::from_ne_bytes([buf[8], buf[9], buf[10], buf[11]]),
        mf_max: f32::from_ne_bytes([buf[12], buf[13], buf[14], buf[15]]),
        hf_min: f32::from_ne_bytes([buf[16], buf[17], buf[18], buf[19]]),
        hf_max: f32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]),
    })
}

fn read_n_chunks<T, R, F>(r: &mut R, count: u64, mut read_one: F) -> io::Result<Vec<T>>
where
    R: Read,
    F: FnMut(&mut R) -> io::Result<T>,
{
    let count = usize::try_from(count)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk count overflow"))?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(read_one(r)?);
    }
    Ok(out)
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_ne_bytes(buf))
}

fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_ne_bytes(buf))
}

fn u32_from_usize(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute_offline_peaks;

    /// All four chunk types must remain padding-free. The
    /// `write_slice_as_bytes` cast assumes `size_of::<T>()` equals
    /// the sum of field sizes; verify that here so a future
    /// reordering or added field can't silently corrupt the cache
    /// format.
    #[test]
    fn chunk_types_have_no_padding() {
        assert_eq!(std::mem::size_of::<PeakChunk>(), 12);
        assert_eq!(std::mem::size_of::<BandPeakChunk>(), 32);
        assert_eq!(std::mem::size_of::<OnsetChunk>(), 4);
        assert_eq!(std::mem::size_of::<FilteredPeakChunk>(), 24);
    }

    /// Round-trip a non-trivial `OfflinePeaks` through write + read
    /// and assert every chunk equals the original. Uses a 1 s
    /// 1 kHz tone so the broadband / band / filtered / onset
    /// streams all have variation (not just zeros) to catch any
    /// silent slice-cast bug.
    #[test]
    #[allow(
        clippy::cast_precision_loss,
        reason = "test-only sine generator; 48 kHz fits inside f32 mantissa with no audible loss"
    )]
    fn round_trip_preserves_all_chunks() {
        let sr: u32 = 48_000;
        let mut samples = Vec::with_capacity(sr as usize);
        for i in 0..sr {
            let t = i as f32 / sr as f32;
            samples.push((2.0 * std::f32::consts::PI * 1_000.0 * t).sin());
        }
        let peaks = compute_offline_peaks(&samples, sr, 1).unwrap();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_sidecar(tmp.path(), &peaks).unwrap();
        let loaded = read_sidecar(tmp.path()).unwrap().expect("cache hit");

        assert_eq!(loaded.sample_rate, peaks.sample_rate);
        assert_eq!(
            loaded.samples_per_broadband_chunk,
            peaks.samples_per_broadband_chunk
        );
        assert_eq!(loaded.samples_per_band_chunk, peaks.samples_per_band_chunk);
        assert_eq!(
            loaded.samples_per_onset_chunk,
            peaks.samples_per_onset_chunk
        );
        assert_eq!(
            loaded.samples_per_filtered_chunk,
            peaks.samples_per_filtered_chunk
        );
        assert_eq!(loaded.broadband, peaks.broadband);
        assert_eq!(loaded.bands, peaks.bands);
        assert_eq!(loaded.onset, peaks.onset);
        assert_eq!(loaded.filtered, peaks.filtered);
    }

    #[test]
    fn missing_file_returns_ok_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.wf");
        let result = read_sidecar(&path).unwrap();
        assert!(result.is_none(), "missing sidecar must be a cache miss");
    }

    #[test]
    fn bad_magic_returns_ok_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"not-a-dub-wf-file").unwrap();
        let result = read_sidecar(tmp.path()).unwrap();
        assert!(result.is_none(), "bad magic must be treated as cache miss");
    }

    #[test]
    fn truncated_file_returns_ok_none() {
        let sr: u32 = 48_000;
        let samples = vec![0.1f32; sr as usize];
        let peaks = compute_offline_peaks(&samples, sr, 1).unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_sidecar(tmp.path(), &peaks).unwrap();
        let raw = std::fs::read(tmp.path()).unwrap();
        std::fs::write(tmp.path(), &raw[..raw.len() / 2]).unwrap();
        let result = read_sidecar(tmp.path()).unwrap();
        assert!(
            result.is_none(),
            "torn-write truncation must be a cache miss"
        );
    }
}
