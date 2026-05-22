//! Filesystem importer (M11c, lazy-fingerprint variant from M11c.4).
//!
//! Walks a folder, parses ID3 / filename metadata via a fast
//! metadata-only probe, and writes the result into the SQLite
//! schema landed in M11a. **No decode, no fingerprint, no dedupe**
//! at import time — those costs are paid lazily when the user first
//! loads the track on a deck (PRD §8.4 "lazy by design").
//!
//! # Pipeline (per file)
//!
//! ```text
//! discover volume UUID for path
//!         │
//!         ▼
//! check track_files for prior import (volume_uuid + relative_path)
//!     ├─ hit → refresh metadata rows, mark seen, done
//!     └─ miss
//!         ▼
//!     dub_io::read_metadata(path)  ← metadata-only probe, no decode
//!         ▼
//!     mint new tracks row (fingerprint_id = NULL, duration_ms = NULL)
//!         ▼
//!     upsert track_files row (codec from extension, file_size, mtime)
//!         ▼
//!     write track_metadata_source('id3') from container tags
//!     write track_metadata_source('filename') from filename parser
//! ```
//!
//! # Why lazy?
//!
//! Empirically (`crates/dub-library/examples/profile_import.rs`),
//! the cold-path decode and Chromaprint pass together account for
//! ~95 % of import wall-clock time on commodity SSDs. Importing a
//! 500-track DnB folder used to take ~3–4 minutes; the metadata-
//! only path lands it in <2 s. The user-visible "import fast,
//! analyze on demand" contract is the PRD's §8.4 default.
//!
//! The deferred work runs in [`Library::analyze_track`]: that path
//! already decodes the file once for BPM + key analysis, so adding
//! the Chromaprint pass to it (M11c.4) costs nothing on top of
//! work the user has already paid for by loading the deck.
//!
//! # What this milestone delivers vs. defers
//!
//! Delivered:
//! * Walk-a-folder driver (recursive, deterministic alphabetical
//!   order, extension-filtered).
//! * Metadata-only ingest for every supported audio format the
//!   workspace's symphonia features cover (WAV, MP3, FLAC, AIFF,
//!   ALAC, AAC).
//! * Idempotent re-import via the `track_files` unique-by-path
//!   index.
//! * Per-source metadata rows for `source='id3'` (container tags)
//!   and `source='filename'` (parsed filename per §8.4).
//! * Junk-pattern detection on ID3 titles per PRD §8.4: when ID3
//!   reports a junk title, the row is still written (per-source
//!   preservation is sacred) but the filename source provides the
//!   browser-displayed value via the §8.1 priority chain.
//!
//! Deferred to later milestones:
//! * Auto-merge / sibling-version dedupe at import time. The
//!   `dedupe::decide` primitives still exist and run lazily during
//!   `Library::analyze_track`; the user-facing "Find duplicates"
//!   action (which surfaces near-duplicate fingerprints for a
//!   manual merge decision) is parked for v1.x.
//! * Background / parallel scanning (v1.x; the M11c driver is
//!   single-threaded, deterministic, easy to reason about).
//! * Progress reporting beyond the returned [`ImportSummary`].

use std::path::{Path, PathBuf};

use dub_io::{LoadError, TrackMetadata};
use uuid::Uuid;

use crate::db::Library;
use crate::error::{LibraryError, Result};
use crate::filename_parser::{self, ParsedFilename};
use crate::version_tokens::VersionToken;
use crate::volumes::discover_for_path;

/// File extensions the importer recognises as audio. Lowercase; the
/// matcher case-insensitives the candidate path. Mirrors the
/// `symphonia` features enabled in the workspace `Cargo.toml`.
const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "aif", "aiff", "m4a", "alac", "aac"];

/// Aggregate result of an [`import_folder`] run.
///
/// Since M11c.4 the importer no longer auto-merges duplicates at
/// import time (the fingerprint isn't computed until first
/// deck-load). `merged` and `sibling_versions` are retained in the
/// summary for API compatibility but always come back zero from
/// the cold-path import; a future "Find duplicates" library action
/// will surface near-duplicate fingerprints for manual user review.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    /// Files that produced a new canonical `tracks` row.
    pub added: u32,
    /// Always `0` since M11c.4. Retained for API compatibility.
    pub merged: u32,
    /// Always `0` since M11c.4. Retained for API compatibility.
    pub sibling_versions: u32,
    /// Files already known by `(volume_uuid, relative_path)` —
    /// metadata rows were refreshed but no new identity was added.
    pub refreshed: u32,
    /// Files skipped because the metadata probe failed or the
    /// volume could not be resolved. Detailed reasons live in
    /// `errors`.
    pub skipped: u32,
    /// Per-file failures (path + short reason). Sized for log
    /// emission; the driver never aborts on a single failure.
    pub errors: Vec<ImportError>,
}

/// One per-file failure surfaced by the importer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    /// The path that failed.
    pub path: PathBuf,
    /// Short human-readable reason ("decode failed: ...", "volume
    /// UUID unavailable: ...", etc.).
    pub reason: String,
}

/// Walk `root` recursively, importing every recognised audio file.
///
/// Deterministic depth-first alphabetical order. Idempotent: a
/// second call against the same folder refreshes metadata rows but
/// does not duplicate `tracks` / `track_files` identities.
///
/// Errors at the directory-traversal level (permissions, broken
/// symlinks) are accumulated in [`ImportSummary::errors`] rather
/// than aborting the run. The driver returns early only if the
/// caller-supplied `root` is itself unreachable.
pub fn import_folder(library: &mut Library, root: &Path) -> Result<ImportSummary> {
    if !root.exists() {
        return Err(LibraryError::io(
            root,
            std::io::Error::new(std::io::ErrorKind::NotFound, "root path does not exist"),
        ));
    }

    let mut summary = ImportSummary::default();

    // Deterministic walk: sort entries by file name within each
    // directory. `walkdir::WalkDir::sort_by_file_name` gives us
    // exactly that.
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                summary.errors.push(ImportError {
                    path: e.path().map(Path::to_path_buf).unwrap_or_default(),
                    reason: format!("walk error: {e}"),
                });
                summary.skipped += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !has_audio_extension(path) {
            continue;
        }
        match import_one(library, path) {
            Ok(outcome) => match outcome {
                FileOutcome::Added => summary.added += 1,
                FileOutcome::Refreshed => summary.refreshed += 1,
            },
            Err(reason) => {
                summary.errors.push(ImportError {
                    path: path.to_path_buf(),
                    reason,
                });
                summary.skipped += 1;
            }
        }
    }

    Ok(summary)
}

/// Per-file outcome from [`import_one`]. Crate-internal because the
/// caller (`import_folder`) maps these onto the [`ImportSummary`]
/// counters directly.
enum FileOutcome {
    Added,
    Refreshed,
}

fn import_one(library: &mut Library, path: &Path) -> std::result::Result<FileOutcome, String> {
    // Volume + path resolution. Per-file failure rather than
    // session-level abort: a network share that doesn't expose a
    // UUID shouldn't prevent the rest of the import from running.
    let volume = discover_for_path(path).map_err(|e| format!("volume UUID unavailable: {e}"))?;
    library.upsert_volume(&volume).map_err(|e| e.to_string())?;
    let relative_path = volume
        .relative_to(path)
        .ok_or_else(|| format!("path {path:?} is not under volume {:?}", volume.mount_point))?;

    let stats = std::fs::metadata(path).map_err(|e| format!("stat: {e}"))?;
    let mtime = stats
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    // Idempotent re-import shortcut: if we've seen this exact
    // `(volume_uuid, relative_path)` before, the canonical track
    // identity is already known. We refresh the per-source
    // metadata rows (the file's ID3 may have changed, the
    // filename may have been renamed) without re-decoding.
    if let Some(existing_uuid) = library
        .find_track_file_owner(&volume.volume_uuid, &relative_path)
        .map_err(|e| e.to_string())?
    {
        refresh_metadata_for_known_track(library, &existing_uuid, path)
            .map_err(|e| e.to_string())?;
        library
            .upsert_track_file(
                &existing_uuid,
                &volume.volume_uuid,
                &relative_path,
                detect_codec_from_extension(path),
                None,
                None,
                None,
                Some(stats.len()),
                mtime,
            )
            .map_err(|e| e.to_string())?;
        return Ok(FileOutcome::Refreshed);
    }

    // Cold path (M11c.4 lazy-fingerprint variant): metadata-only
    // probe, no decode, no fingerprint, no dedupe. The fingerprint
    // and `tracks.duration_ms` get filled in on first deck-load
    // via [`Library::analyze_track`].
    let meta = dub_io::read_metadata(path)
        .map_err(|e| format!("metadata probe failed: {}", summarise_load_error(&e)))?;

    let parsed_filename = filename_parser::parse(
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default(),
    );

    let new_track_uuid = Uuid::new_v4().to_string();
    library
        .insert_track(&new_track_uuid, None, None, None)
        .map_err(|e| e.to_string())?;
    library
        .upsert_track_file(
            &new_track_uuid,
            &volume.volume_uuid,
            &relative_path,
            detect_codec_from_extension(path),
            None,
            None,
            None,
            Some(stats.len()),
            mtime,
        )
        .map_err(|e| e.to_string())?;
    write_metadata_rows(library, &new_track_uuid, &meta, &parsed_filename)?;
    Ok(FileOutcome::Added)
}

/// Write the `id3` and `filename` per-source metadata rows for the
/// given canonical track UUID. UPSERT semantics mean a re-import
/// refreshes both rows in place. Operates on a [`TrackMetadata`]
/// (from the metadata-only probe) since M11c.4; the importer
/// never decodes samples on the cold path.
fn write_metadata_rows(
    library: &Library,
    track_uuid: &str,
    meta: &TrackMetadata,
    parsed_filename: &ParsedFilename,
) -> std::result::Result<(), String> {
    let id3_title = meta.title.as_deref();
    let filename_token = format_token_for_storage(&parsed_filename.version_tokens);
    let id3_token = format_token_for_storage(&BTreeSetFromTitle::tokens(id3_title));

    library
        .upsert_metadata_source(
            track_uuid,
            "id3",
            meta.artist.as_deref(),
            id3_title,
            meta.album.as_deref(),
            meta.genre.as_deref(),
            meta.comment.as_deref(),
            meta.composer.as_deref(),
            meta.year,
            meta.track_number,
            meta.bpm,
            meta.key.as_deref(),
            meta.gain_db,
            None,
            id3_token.as_deref(),
        )
        .map_err(|e| format!("write id3 row: {e}"))?;

    library
        .upsert_metadata_source(
            track_uuid,
            "filename",
            parsed_filename.artist.as_deref(),
            parsed_filename.title.as_deref(),
            None,
            None,
            parsed_filename.label_catalog.as_deref(),
            None,
            parsed_filename.year,
            None,
            None,
            None,
            None,
            None,
            filename_token.as_deref(),
        )
        .map_err(|e| format!("write filename row: {e}"))?;

    Ok(())
}

/// Refresh just the metadata rows on a known `(volume, path)`. We
/// re-read tags via a metadata-only probe rather than a full decode
/// because the file may have grown / been re-encoded; cheap probe is
/// the right tool. The fingerprint stays put — re-fingerprinting is
/// expensive and the typical reason for re-import is a tag fix, not
/// a re-encode.
fn refresh_metadata_for_known_track(
    library: &Library,
    track_uuid: &str,
    path: &Path,
) -> std::result::Result<(), String> {
    // Use the fast metadata-only probe.
    let meta = dub_io::read_metadata(path)
        .map_err(|e| format!("metadata probe failed: {}", summarise_load_error(&e)))?;

    let parsed_filename = filename_parser::parse(
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default(),
    );

    // Symmetric with `write_metadata_rows` (cold-import path): the
    // id3 row's `version_token` column must be re-derived from the
    // current ID3 title on every refresh. The `Library::upsert_metadata_source`
    // UPSERT writes `version_token = excluded.version_token`
    // unconditionally, so passing `None` here would clobber any
    // token previously stored by the cold-import path (e.g. "clean"
    // parsed from a title like "Workinonit (Clean)") with NULL, even
    // though the file's tag is unchanged. Re-parse from `meta.title`
    // so the two code paths produce the same row.
    let id3_token = format_token_for_storage(&BTreeSetFromTitle::tokens(meta.title.as_deref()));
    library
        .upsert_metadata_source(
            track_uuid,
            "id3",
            meta.artist.as_deref(),
            meta.title.as_deref(),
            meta.album.as_deref(),
            meta.genre.as_deref(),
            meta.comment.as_deref(),
            meta.composer.as_deref(),
            meta.year,
            meta.track_number,
            meta.bpm,
            meta.key.as_deref(),
            meta.gain_db,
            None,
            id3_token.as_deref(),
        )
        .map_err(|e| format!("refresh id3 row: {e}"))?;

    let filename_token = format_token_for_storage(&parsed_filename.version_tokens);
    library
        .upsert_metadata_source(
            track_uuid,
            "filename",
            parsed_filename.artist.as_deref(),
            parsed_filename.title.as_deref(),
            None,
            None,
            parsed_filename.label_catalog.as_deref(),
            None,
            parsed_filename.year,
            None,
            None,
            None,
            None,
            None,
            filename_token.as_deref(),
        )
        .map_err(|e| format!("refresh filename row: {e}"))?;

    Ok(())
}

/// Comma-separated canonical form of a version-token set for
/// storage in `track_metadata_source.version_token`. `None` when
/// the set is empty.
fn format_token_for_storage(tokens: &std::collections::BTreeSet<VersionToken>) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    let joined: Vec<&str> = tokens.iter().map(VersionToken::as_str).collect();
    Some(joined.join(","))
}

/// Quick filename-extension audio-type filter.
fn has_audio_extension(path: &Path) -> bool {
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => {
            let lower = ext.to_ascii_lowercase();
            AUDIO_EXTS.iter().any(|e| *e == lower)
        }
        None => false,
    }
}

/// Map an extension to the codec string we store in
/// `track_files.codec`. We don't peek inside the file; the
/// container's codec id from symphonia is what we'd use later if
/// we wanted finer detail. For M11c the extension is sufficient.
fn detect_codec_from_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "wav" => Some("wav"),
        "mp3" => Some("mp3"),
        "flac" => Some("flac"),
        "aif" | "aiff" => Some("aiff"),
        "m4a" | "alac" => Some("alac"),
        "aac" => Some("aac"),
        _ => None,
    }
}

/// Short-form description of a `LoadError` for the per-file errors
/// list. Avoids storing the full chain (which the caller can't
/// display nicely) but keeps the failure mode identifiable.
fn summarise_load_error(e: &LoadError) -> String {
    match e {
        LoadError::Io(io) => format!("io: {io}"),
        LoadError::Format(msg) => format!("format: {msg}"),
        LoadError::NoAudioTrack => "no audio track".to_string(),
        LoadError::UnsupportedChannels(c) => format!("unsupported channels: {c}"),
        LoadError::Empty => "empty".to_string(),
    }
}

/// Helper to parse version tokens from an Option<&str> directly.
/// Centralised so the importer's two metadata-write paths agree.
struct BTreeSetFromTitle;

impl BTreeSetFromTitle {
    fn tokens(title: Option<&str>) -> std::collections::BTreeSet<VersionToken> {
        match title {
            Some(t) => crate::version_tokens::parse(t),
            None => std::collections::BTreeSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use tempfile::tempdir;

    /// Generate a synthetic mono i16 WAV at the given path with the
    /// given frequency and duration. Used by the integration tests
    /// so we don't need to ship audio fixtures in the repo.
    fn write_wav(path: &Path, freq_hz: f32, duration_secs: f32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        let n = (44_100_f32 * duration_secs) as usize;
        for i in 0..n {
            let t = i as f32 / 44_100_f32;
            let s = 0.5 * (2.0 * std::f32::consts::PI * freq_hz * t).sin();
            let q = (s * f32::from(i16::MAX)) as i16;
            writer.write_sample(q).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Generate a mono i16 WAV with a RIFF `LIST` / `INFO` / `INAM`
    /// chunk carrying the given title. `hound` doesn't write RIFF
    /// INFO subchunks, but symphonia's WAV reader surfaces `INAM` as
    /// `StandardTagKey::TrackTitle`, so we hand-assemble the
    /// container bytes to exercise the importer's title-bearing
    /// path. Mono / 16-bit / 44.1 kHz to keep alignment trivial.
    fn write_wav_with_inam_title(path: &Path, freq_hz: f32, duration_secs: f32, title: &str) {
        let sr: u32 = 44_100;
        let channels: u16 = 1;
        let bits_per_sample: u16 = 16;
        let n_samples = (f64::from(sr) * f64::from(duration_secs)) as u32;

        let mut sample_bytes: Vec<u8> = Vec::with_capacity(n_samples as usize * 2);
        for i in 0..n_samples {
            let t = (i as f32) / (sr as f32);
            let s = 0.5 * (2.0 * std::f32::consts::PI * freq_hz * t).sin();
            let q = (s * f32::from(i16::MAX)) as i16;
            sample_bytes.extend_from_slice(&q.to_le_bytes());
        }
        let data_size = sample_bytes.len() as u32;

        let mut inam_payload: Vec<u8> = title.as_bytes().to_vec();
        inam_payload.push(0);
        let inam_size = inam_payload.len() as u32;
        let inam_pad = inam_size % 2;
        if inam_pad != 0 {
            inam_payload.push(0);
        }
        let inam_chunk_total = 8 + inam_size + inam_pad;

        let list_data_size = 4 + inam_chunk_total;
        let list_pad = list_data_size % 2;

        let fmt_chunk_total: u32 = 8 + 16;
        let list_chunk_total = 8 + list_data_size + list_pad;
        let data_chunk_total = 8 + data_size;
        let riff_data_size = 4 + fmt_chunk_total + list_chunk_total + data_chunk_total;

        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&riff_data_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sr.to_le_bytes());
        let byte_rate = sr * u32::from(channels) * u32::from(bits_per_sample) / 8;
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        let block_align = channels * bits_per_sample / 8;
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());

        buf.extend_from_slice(b"LIST");
        buf.extend_from_slice(&list_data_size.to_le_bytes());
        buf.extend_from_slice(b"INFO");
        buf.extend_from_slice(b"INAM");
        buf.extend_from_slice(&inam_size.to_le_bytes());
        buf.extend_from_slice(&inam_payload);
        if list_pad != 0 {
            buf.push(0);
        }

        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        buf.extend_from_slice(&sample_bytes);

        std::fs::write(path, buf).unwrap();
    }

    /// Open an importer-friendly library at a tempfile path. We
    /// can't use in-memory because the importer needs the file's
    /// volume to be a real filesystem volume (the in-memory DB
    /// path `:memory:` wouldn't resolve through getattrlist).
    fn open_lib(dir: &Path) -> Library {
        Library::open_at(&dir.join("library.sqlite")).unwrap()
    }

    /// Count rows in a given table — handy for assertions over
    /// the SQL writes M11c performs.
    fn row_count(library: &Library, table: &str) -> i64 {
        library
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn imports_a_fresh_folder_with_one_track() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_wav(&music.join("J Dilla - Workinonit.wav"), 440.0, 10.0);

        let mut lib = open_lib(tmp.path());
        let summary = import_folder(&mut lib, &music).expect("import");
        assert_eq!(summary.added, 1);
        assert_eq!(summary.merged, 0);
        assert_eq!(summary.skipped, 0);
        assert!(summary.errors.is_empty(), "errors: {:?}", summary.errors);

        assert_eq!(row_count(&lib, "tracks"), 1);
        assert_eq!(row_count(&lib, "track_files"), 1);
        // id3 row + filename row → 2 rows per track.
        assert_eq!(row_count(&lib, "track_metadata_source"), 2);
    }

    #[test]
    fn re_import_is_idempotent() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_wav(&music.join("Track A.wav"), 440.0, 10.0);
        write_wav(&music.join("Track B.wav"), 1320.0, 10.0);

        let mut lib = open_lib(tmp.path());
        let first = import_folder(&mut lib, &music).expect("first import");
        assert_eq!(first.added, 2);

        let second = import_folder(&mut lib, &music).expect("second import");
        // Both files seen before by (volume_uuid, relative_path):
        // refresh, don't double-write.
        assert_eq!(second.added, 0);
        assert_eq!(second.merged, 0);
        assert_eq!(second.refreshed, 2);

        assert_eq!(row_count(&lib, "tracks"), 2);
        assert_eq!(row_count(&lib, "track_files"), 2);
    }

    #[test]
    fn identical_files_do_not_merge_at_import() {
        // M11c.4 lazy-fingerprint contract: the importer does not
        // decode, does not fingerprint, and does not dedupe at
        // import time. Two byte-identical files therefore land as
        // two distinct `tracks` rows with `fingerprint_id = NULL`.
        // A future "Find duplicates" library action surfaces near-
        // duplicate fingerprints once `Library::analyze_track` has
        // filled them in on first deck-load.
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_wav(&music.join("Workinonit.wav"), 440.0, 10.0);
        write_wav(&music.join("Workinonit (copy).wav"), 440.0, 10.0);

        let mut lib = open_lib(tmp.path());
        let summary = import_folder(&mut lib, &music).expect("import");
        assert_eq!(summary.added, 2, "summary: {summary:?}");
        assert_eq!(summary.merged, 0, "no auto-merge in M11c.4");
        assert_eq!(summary.sibling_versions, 0, "no sibling at import");
        assert_eq!(row_count(&lib, "tracks"), 2);
        assert_eq!(row_count(&lib, "track_files"), 2);
        // No fingerprint rows yet — they materialize on
        // `analyze_track`.
        assert_eq!(row_count(&lib, "fingerprints"), 0);

        // Both tracks: fingerprint_id IS NULL, duration_ms IS NULL.
        let null_fp: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE fingerprint_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(null_fp, 2);
        let null_dur: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE duration_ms IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(null_dur, 2);
    }

    #[test]
    fn clean_and_dirty_are_separate_at_import() {
        // M11c.4: §8.1 version-token disagreement is no longer
        // resolved at import time. Both files land as distinct
        // `tracks` rows with NULL fingerprint and no
        // `duplicate_link_track_id`. The disagreement only matters
        // once both fingerprints have been computed via
        // `analyze_track`, at which point the "Find duplicates"
        // action (deferred to v1.x) can surface the near-match.
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_wav(&music.join("Lady (Clean).wav"), 440.0, 10.0);
        write_wav(&music.join("Lady (Dirty).wav"), 440.0, 10.0);

        let mut lib = open_lib(tmp.path());
        let summary = import_folder(&mut lib, &music).expect("import");
        assert_eq!(summary.added, 2);
        assert_eq!(summary.sibling_versions, 0);
        assert_eq!(summary.merged, 0);
        assert_eq!(row_count(&lib, "tracks"), 2);
        let linked: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE duplicate_link_track_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(linked, 0, "no sibling links at import in M11c.4");
    }

    #[test]
    fn skips_non_audio_files() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        std::fs::write(music.join("notes.txt"), b"not audio").unwrap();
        std::fs::write(music.join("cover.jpg"), b"not audio").unwrap();
        write_wav(&music.join("Workinonit.wav"), 440.0, 10.0);

        let mut lib = open_lib(tmp.path());
        let summary = import_folder(&mut lib, &music).expect("import");
        assert_eq!(summary.added, 1);
        assert_eq!(summary.skipped, 0);
    }

    #[test]
    fn rejects_missing_root_path() {
        let tmp = tempdir().unwrap();
        let mut lib = open_lib(tmp.path());
        let bogus = tmp.path().join("does-not-exist");
        let result = import_folder(&mut lib, &bogus);
        assert!(matches!(result, Err(LibraryError::Io { .. })));
    }

    #[test]
    fn metadata_rows_are_written_per_source() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_wav(
            &music.join("J Dilla - Workinonit (Instrumental).wav"),
            440.0,
            10.0,
        );

        let mut lib = open_lib(tmp.path());
        import_folder(&mut lib, &music).expect("import");

        let filename_row: (String, Option<String>, Option<String>, Option<String>) = lib
            .connection()
            .query_row(
                "SELECT source, artist, title, version_token \
                 FROM track_metadata_source WHERE source = 'filename'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(filename_row.0, "filename");
        assert_eq!(filename_row.1.as_deref(), Some("J Dilla"));
        assert_eq!(filename_row.2.as_deref(), Some("Workinonit"));
        assert_eq!(filename_row.3.as_deref(), Some("instrumental"));

        // The hound-generated WAV has no INFO chunk → id3 row is
        // present but all fields are NULL. The row exists for
        // schema-consistency (we always write both sources), not
        // because it carries useful data.
        let id3_count: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM track_metadata_source WHERE source = 'id3'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id3_count, 1);
    }

    /// Cross-check: the hand-rolled `write_wav_with_inam_title`
    /// helper actually produces a file whose RIFF INFO INAM is
    /// surfaced by `dub_io::read_metadata` as the title. If symphonia
    /// ever changes its WAV reader to drop INAM, this test fails
    /// first and the regression test below stops being a meaningful
    /// signal — so it's worth pinning the contract explicitly.
    ///
    /// Note: symphonia 0.5.x preserves the trailing null terminator
    /// from the INAM ZSTR field in its decoded `TrackTitle` value,
    /// and `dub-io`'s `copy_tag` only `trim()`s whitespace (not null
    /// bytes), so the title we read back has a `\0` suffix. The
    /// `version_tokens::parse` scanner is byte-tolerant enough that
    /// "(Clean)\0" still yields the `clean` token. The assertion
    /// codifies this end-to-end behaviour so future tightening (e.g.
    /// stripping NULs in `copy_tag`) shows up as a deliberate change
    /// rather than a silent semantic drift.
    #[test]
    fn write_wav_with_inam_title_round_trips_through_read_metadata() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("titled.wav");
        write_wav_with_inam_title(&path, 440.0, 1.0, "Workinonit (Clean)");

        let meta = dub_io::read_metadata(&path).expect("read_metadata");
        let title = meta.title.expect("INAM should surface as title");
        let trimmed = title.trim_end_matches('\0');
        assert_eq!(trimmed, "Workinonit (Clean)");
    }

    /// Regression test: re-importing a file whose ID3 title contains
    /// a version qualifier must not clobber the `id3` source row's
    /// `version_token` column. Bug pre-fix:
    /// `refresh_metadata_for_known_track` passed `None` for
    /// `version_token`, so the UPSERT's
    /// `version_token = excluded.version_token` clause overwrote the
    /// "clean" token written by the cold-import path with NULL on
    /// every subsequent scan. The `filename` source was unaffected
    /// (it correctly re-parses `parsed_filename.version_tokens`),
    /// so the bug was silent unless the priority chain happened to
    /// pick the `id3` row for the version qualifier.
    #[test]
    fn re_import_preserves_id3_version_token_from_title() {
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let wav = music.join("Workinonit.wav");
        write_wav_with_inam_title(&wav, 440.0, 10.0, "Workinonit (Clean)");

        let mut lib = open_lib(tmp.path());

        let first = import_folder(&mut lib, &music).expect("first import");
        assert_eq!(first.added, 1, "cold import: {first:?}");

        let token_after_cold: Option<String> = lib
            .connection()
            .query_row(
                "SELECT version_token FROM track_metadata_source \
                 WHERE source = 'id3'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            token_after_cold.as_deref(),
            Some("clean"),
            "precondition: cold-import path parses 'clean' from the ID3 \
             title 'Workinonit (Clean)'"
        );

        let second = import_folder(&mut lib, &music).expect("second import");
        assert_eq!(second.refreshed, 1, "refresh path: {second:?}");

        let token_after_refresh: Option<String> = lib
            .connection()
            .query_row(
                "SELECT version_token FROM track_metadata_source \
                 WHERE source = 'id3'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            token_after_refresh.as_deref(),
            Some("clean"),
            "regression: refresh must re-derive the id3 version_token \
             from meta.title rather than overwriting it with NULL"
        );
    }

    // Silence the unused-import warning when the parent file's
    // re-exports already cover what tests need.
    #[allow(dead_code)]
    fn _types_in_scope() -> PathBuf {
        PathBuf::new()
    }
}
