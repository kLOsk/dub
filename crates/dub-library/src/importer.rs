//! Filesystem importer (M11c).
//!
//! Walks a folder, decodes each audio file via `dub-io`, computes a
//! canonical Chromaprint fingerprint via `dub-fingerprint`, parses
//! ID3 / filename metadata, runs the §8.1 dedupe decision against
//! existing library rows, and writes the result into the SQLite
//! schema landed in M11a.
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
//!     decode samples + tags via dub_io::Track::load_from_path
//!         ▼
//!     compute Chromaprint fingerprint
//!         ▼
//!     find_fingerprint_neighbours(duration_ms, 200 ms)
//!         ▼
//!     for each neighbour: dedupe::decide(...)
//!         ├─ Merge          → register additional track_files row
//!         ├─ SiblingVersion → new tracks row + duplicate_link_track_id
//!         └─ Distinct       → fall through
//!         ▼
//!     no merge / sibling? → new canonical tracks row
//!         ▼
//!     write track_metadata_source('id3') from container tags
//!     write track_metadata_source('filename') from filename parser
//! ```
//!
//! # What this milestone delivers vs. defers
//!
//! Delivered:
//! * Walk-a-folder driver (recursive, deterministic alphabetical
//!   order, extension-filtered).
//! * Decode + fingerprint + dedupe + write for every supported
//!   audio format the workspace's symphonia features cover (WAV,
//!   MP3, FLAC, AIFF, ALAC, AAC).
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
//! * `analysis_cache` LUFS / waveform / has_active_grid (M10.5j +
//!   subsequent analysis pipeline).
//! * `track_beatgrids(source='auto')` cross-validation (needs
//!   `dub-bpm::analyze_bpm` wired in — slated for follow-up to
//!   M11c).
//! * Background / parallel scanning (v1.x; the M11c driver is
//!   single-threaded, deterministic, easy to reason about).
//! * Progress reporting beyond the returned [`ImportSummary`].

use std::path::{Path, PathBuf};

use dub_io::{LoadError, Track};
use uuid::Uuid;

use crate::db::{Library, StoredFingerprint};
use crate::dedupe::{decide, DedupeDecision, DedupeInput, DURATION_DELTA_MS};
use crate::error::{LibraryError, Result};
use crate::filename_parser::{self, ParsedFilename};
use crate::version_tokens::VersionToken;
use crate::volumes::discover_for_path;
use dub_fingerprint::Fingerprint;

/// File extensions the importer recognises as audio. Lowercase; the
/// matcher case-insensitives the candidate path. Mirrors the
/// `symphonia` features enabled in the workspace `Cargo.toml`.
const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "aif", "aiff", "m4a", "alac", "aac"];

/// Aggregate result of an [`import_folder`] run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    /// Files that produced a new canonical `tracks` row.
    pub added: u32,
    /// Files that merged into an existing canonical `tracks` row
    /// (additional `track_files` row written; canonical identity
    /// unchanged).
    pub merged: u32,
    /// Files registered as sibling versions (new `tracks` row with
    /// `duplicate_link_track_id` populated).
    pub sibling_versions: u32,
    /// Files already known by `(volume_uuid, relative_path)` —
    /// metadata rows were refreshed but no new identity was added.
    pub refreshed: u32,
    /// Files skipped because decoding failed or the volume could
    /// not be resolved. Detailed reasons live in `errors`.
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
                FileOutcome::Merged => summary.merged += 1,
                FileOutcome::SiblingVersion => summary.sibling_versions += 1,
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
    Merged,
    SiblingVersion,
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
        // We still update the file row's last_seen_at + mtime via
        // a touch upsert; the track owner stays the same.
        let stats = std::fs::metadata(path).map_err(|e| format!("stat: {e}"))?;
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
                stats
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64),
            )
            .map_err(|e| e.to_string())?;
        return Ok(FileOutcome::Refreshed);
    }

    // Cold path: decode + fingerprint + dedupe + write.
    let track = Track::load_from_path(path)
        .map_err(|e| format!("decode failed: {}", summarise_load_error(&e)))?;
    let fingerprint = Fingerprint::compute_from_f32(
        track.samples(),
        track.sample_rate(),
        u32::from(track.channels()),
    )
    .map_err(|e| format!("fingerprint failed: {e}"))?;

    let stats = std::fs::metadata(path).map_err(|e| format!("stat: {e}"))?;
    let mtime = stats
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let parsed_filename = filename_parser::parse(
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default(),
    );

    let duration_ms = fingerprint.duration_ms();
    let neighbours = library
        .find_fingerprint_neighbours(duration_ms, DURATION_DELTA_MS)
        .map_err(|e| e.to_string())?;

    // Stable display string for the dedupe call: prefer the parsed
    // filename, fall back to the ID3 title. The dedupe parser only
    // looks for version tokens, so either suffices in practice.
    let candidate_display = composed_display_string(&parsed_filename, track.title());

    let (dedupe_outcome, sibling_target) = run_dedupe(
        library,
        &fingerprint,
        duration_ms,
        &candidate_display,
        &neighbours,
    )?;

    let codec = detect_codec_from_extension(path);
    let sample_rate = Some(track.sample_rate());
    let channels = Some(u32::from(track.channels()));

    let outcome = match dedupe_outcome {
        DedupeResolution::MergeInto(existing_track_uuid) => {
            library
                .upsert_track_file(
                    &existing_track_uuid,
                    &volume.volume_uuid,
                    &relative_path,
                    codec,
                    sample_rate,
                    None,
                    channels,
                    Some(stats.len()),
                    mtime,
                )
                .map_err(|e| e.to_string())?;
            write_metadata_rows(library, &existing_track_uuid, &track, &parsed_filename)?;
            FileOutcome::Merged
        }
        DedupeResolution::SiblingOf(_) | DedupeResolution::Distinct => {
            // Either way, we mint a new canonical track. Sibling
            // case additionally sets the duplicate_link.
            let new_track_uuid = Uuid::new_v4().to_string();
            let fingerprint_id = library
                .upsert_fingerprint(
                    &fingerprint,
                    Some(track.sample_rate()),
                    Some(u32::from(track.channels())),
                    Some(stats.len()),
                )
                .map_err(|e| e.to_string())?;
            let duplicate_link = match dedupe_outcome {
                DedupeResolution::SiblingOf(ref existing) => Some(existing.as_str()),
                _ => None,
            };
            library
                .insert_track(&new_track_uuid, fingerprint_id, duration_ms, duplicate_link)
                .map_err(|e| e.to_string())?;
            library
                .upsert_track_file(
                    &new_track_uuid,
                    &volume.volume_uuid,
                    &relative_path,
                    codec,
                    sample_rate,
                    None,
                    channels,
                    Some(stats.len()),
                    mtime,
                )
                .map_err(|e| e.to_string())?;
            write_metadata_rows(library, &new_track_uuid, &track, &parsed_filename)?;
            let _ = sibling_target;
            if matches!(dedupe_outcome, DedupeResolution::SiblingOf(_)) {
                FileOutcome::SiblingVersion
            } else {
                FileOutcome::Added
            }
        }
    };
    Ok(outcome)
}

/// Internal dedupe resolution carrying any neighbour track UUID we
/// need to write to a foreign key.
enum DedupeResolution {
    /// Auto-merge: register an additional `track_files` row against
    /// the given existing track UUID.
    MergeInto(String),
    /// Sibling version: new `tracks` row with
    /// `duplicate_link_track_id` pointing at the given UUID.
    SiblingOf(String),
    /// Distinct: new `tracks` row, no duplicate link.
    Distinct,
}

/// Iterate the duration-neighbour fingerprints and pick the best
/// dedupe outcome. The first auto-merge candidate wins; if none
/// merge, the first sibling-version candidate wins (so the user
/// gets a single deterministic "potential duplicate" link even
/// when several near-matches exist); otherwise Distinct.
fn run_dedupe(
    library: &Library,
    candidate_fp: &Fingerprint,
    candidate_duration_ms: u32,
    candidate_display: &str,
    neighbours: &[StoredFingerprint],
) -> std::result::Result<(DedupeResolution, Option<String>), String> {
    let mut first_sibling: Option<String> = None;
    for neighbour in neighbours {
        // The neighbour fingerprint row tells us the fingerprint
        // and duration; we still need the track UUID and a display
        // string for the version-token check.
        let owner = library
            .find_track_owner_by_fingerprint_id(neighbour.id)
            .map_err(|e| e.to_string())?;
        let Some((owner_uuid, owner_display)) = owner else {
            // Fingerprint row exists with no tracks pointing at it
            // (orphaned by a delete + re-import race). Skip; we
            // don't auto-resurrect orphans.
            continue;
        };
        let decision = decide(
            &DedupeInput {
                fingerprint: candidate_fp,
                duration_ms: candidate_duration_ms,
                title_or_filename: candidate_display,
            },
            &DedupeInput {
                fingerprint: &neighbour.fingerprint,
                duration_ms: neighbour.fingerprint.duration_ms(),
                title_or_filename: &owner_display,
            },
        );
        match decision {
            DedupeDecision::Merge => {
                return Ok((DedupeResolution::MergeInto(owner_uuid), None));
            }
            DedupeDecision::SiblingVersion { .. } => {
                if first_sibling.is_none() {
                    first_sibling = Some(owner_uuid);
                }
            }
            DedupeDecision::Distinct => {}
        }
    }
    if let Some(uuid) = first_sibling {
        Ok((DedupeResolution::SiblingOf(uuid.clone()), Some(uuid)))
    } else {
        Ok((DedupeResolution::Distinct, None))
    }
}

/// Write the `id3` and `filename` per-source metadata rows for the
/// given canonical track UUID. UPSERT semantics mean a re-import
/// refreshes both rows in place.
fn write_metadata_rows(
    library: &Library,
    track_uuid: &str,
    track: &Track,
    parsed_filename: &ParsedFilename,
) -> std::result::Result<(), String> {
    let ext = track.extended_metadata();
    let id3_title = ext.title.as_deref();
    let filename_token = format_token_for_storage(&parsed_filename.version_tokens);
    let id3_token = format_token_for_storage(&BTreeSetFromTitle::tokens(id3_title));

    library
        .upsert_metadata_source(
            track_uuid,
            "id3",
            ext.artist.as_deref(),
            id3_title,
            ext.album.as_deref(),
            ext.genre.as_deref(),
            ext.comment.as_deref(),
            ext.composer.as_deref(),
            ext.year,
            ext.track_number,
            ext.bpm,
            ext.key.as_deref(),
            ext.gain_db,
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

/// Compose the title-or-filename string the dedupe parser scans
/// for version tokens. Prefers the parsed filename (carries the
/// `(Clean)` / `(Dirty)` / etc. tag verbatim); falls back to the
/// ID3 title; falls back to "" so the dedupe still runs on
/// fingerprint + duration alone.
fn composed_display_string(parsed: &ParsedFilename, id3_title: Option<&str>) -> String {
    match (parsed.title.as_deref(), id3_title) {
        (Some(t), _) => {
            let mut out = String::new();
            if let Some(a) = parsed.artist.as_deref() {
                out.push_str(a);
                out.push_str(" - ");
            }
            out.push_str(t);
            // Re-attach a synthetic "(VERSION)" tail when the
            // parsed-filename token set is non-empty so the
            // version_tokens parser inside dedupe re-discovers
            // them on the composed string.
            if !parsed.version_tokens.is_empty() {
                out.push_str(" (");
                let mut first = true;
                for tok in &parsed.version_tokens {
                    if !first {
                        out.push(' ');
                    }
                    out.push_str(tok.as_str());
                    first = false;
                }
                out.push(')');
            }
            out
        }
        (None, Some(id3)) => id3.to_string(),
        (None, None) => String::new(),
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
    fn two_identical_files_merge() {
        // Same audio content, different filenames → fingerprint
        // matches, duration matches, no version-token mismatch:
        // §8.1 auto-merge. Second file lands as additional
        // track_files row against the same tracks row.
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_wav(&music.join("Workinonit.wav"), 440.0, 10.0);
        write_wav(&music.join("Workinonit (copy).wav"), 440.0, 10.0);

        let mut lib = open_lib(tmp.path());
        let summary = import_folder(&mut lib, &music).expect("import");
        assert_eq!(
            (summary.added, summary.merged),
            (1, 1),
            "summary: {summary:?}"
        );
        assert_eq!(row_count(&lib, "tracks"), 1);
        assert_eq!(row_count(&lib, "track_files"), 2);
    }

    #[test]
    fn clean_and_dirty_register_as_siblings() {
        // Same audio content (so fingerprint matches), same
        // duration, but version-token disagreement on the
        // filename: §8.1 must refuse to merge and instead register
        // a `duplicate_link_track_id`.
        let tmp = tempdir().unwrap();
        let music = tmp.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        write_wav(&music.join("Lady (Clean).wav"), 440.0, 10.0);
        write_wav(&music.join("Lady (Dirty).wav"), 440.0, 10.0);

        let mut lib = open_lib(tmp.path());
        let summary = import_folder(&mut lib, &music).expect("import");
        assert_eq!(summary.added, 1);
        assert_eq!(summary.sibling_versions, 1);
        assert_eq!(summary.merged, 0);
        assert_eq!(row_count(&lib, "tracks"), 2);
        assert_eq!(row_count(&lib, "track_files"), 2);
        // The sibling row has duplicate_link_track_id populated.
        let linked: i64 = lib
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE duplicate_link_track_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(linked, 1);
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
