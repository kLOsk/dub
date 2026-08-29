//! Commit: slice the recorded side into segments, encode + tag each
//! as FLAC, import into the library pre-analyzed, archive the side,
//! and clean up the spill.
//!
//! Per-segment failures are collected, never raised — one broken
//! segment must not strand the rest of the side. The manifest is
//! saved after every segment so a crash mid-commit resumes exactly
//! where it stopped (segments with a `library_uuid` are skipped).

use std::path::{Path, PathBuf};

use dub_encode::{encode_flac_24bit, write_tags, TrackTags};
use dub_library::{analyze_compute_with_track, import_file, Library};

use crate::manifest::{self, RipManifest};
use crate::plan;
use crate::session::{RipError, ARCHIVE_FILE};

/// Result of committing one segment.
#[derive(Debug, Clone)]
pub struct SegmentOutcome {
    /// Segment index (0-based; track numbers are index + 1).
    pub index: usize,
    /// Encoded file path.
    pub file: PathBuf,
    /// Canonical library UUID once imported.
    pub library_uuid: Option<String>,
    /// First error hit for this segment, if any. A segment can be
    /// imported but carry an analysis error — the library then
    /// analyzes it lazily on first load like any other import.
    pub error: Option<String>,
}

/// Result of a whole commit pass.
#[derive(Debug, Clone)]
pub struct RipOutcome {
    /// One entry per segment, in side order.
    pub segments: Vec<SegmentOutcome>,
    /// Lossless side archive, once written.
    pub archive: Option<PathBuf>,
    /// Whether the WAV spill was deleted (only after every segment
    /// imported and the archive landed).
    pub spill_removed: bool,
    /// How many tracks from an earlier split of this side were removed
    /// from the library (M26b re-split). Always 0 for a first commit.
    pub replaced_removed: usize,
}

impl RipOutcome {
    /// True when every segment imported and the archive was written.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.archive.is_some() && self.segments.iter().all(|s| s.library_uuid.is_some())
    }
}

/// Where a segment is in the commit pass, reported as it happens.
///
/// The commit worker is the slowest thing in the rip flow (encode +
/// tag + import + full analysis, per track), so without this the UI
/// can only flip every dot at once when the whole pass returns —
/// minutes of apparent stall on a six-track side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitProgress {
    /// Work has started on this segment.
    Started {
        /// 0-based segment index.
        index: usize,
    },
    /// The segment imported (or was already imported).
    Finished {
        /// 0-based segment index.
        index: usize,
        /// Whether it landed in the library.
        imported: bool,
    },
    /// Every segment is done; the lossless side archive is encoding.
    ArchiveStarted,
}

pub(crate) fn commit_session(
    session_dir: &Path,
    spill_path: &Path,
    manifest: &mut RipManifest,
    library: &mut Library,
    progress: &mut dyn FnMut(CommitProgress),
) -> Result<RipOutcome, RipError> {
    // A fully committed session (every segment imported, archive
    // written, spill already deleted) short-circuits — the retry
    // path must be callable without the audio still on disk.
    let fully_committed = !manifest.tracks.is_empty()
        && manifest.tracks.len() == manifest.boundaries_frames.len() + 1
        && manifest.tracks.iter().all(|t| t.library_uuid.is_some())
        && manifest.side_archive.is_some();
    if fully_committed && !spill_path.exists() {
        return Ok(RipOutcome {
            segments: manifest
                .tracks
                .iter()
                .enumerate()
                .map(|(index, t)| SegmentOutcome {
                    index,
                    file: session_dir.join(t.encoded_file.as_deref().unwrap_or_default()),
                    library_uuid: t.library_uuid.clone(),
                    error: None,
                })
                .collect(),
            archive: manifest.side_archive.as_ref().map(|f| session_dir.join(f)),
            spill_removed: false,
            replaced_removed: 0,
        });
    }

    let (samples, sample_rate) = read_spill(spill_path, &session_dir.join(ARCHIVE_FILE))?;
    let total_frames = (samples.len() / 2) as u64;
    // Trust the audio file over the manifest count (a crash between
    // WAV finalize and manifest save leaves them skewed).
    manifest.recorded_frames = total_frames;
    // The side may end before the recording does — the detector trims
    // the run-out groove off the last track (the full capture stays in
    // `side.flac`, so a re-split can reach past this).
    let side_end = manifest.side_end();
    plan::validate_boundaries(
        &manifest.boundaries_frames,
        side_end,
        sample_rate,
        plan::MIN_SEGMENT_SECS,
    )?;

    let ranges = plan::segments(&manifest.boundaries_frames, side_end);
    if manifest.tracks.len() != ranges.len() {
        manifest
            .tracks
            .resize_with(ranges.len(), crate::manifest::TrackEntry::default);
    }
    #[allow(clippy::cast_possible_truncation)]
    let track_total = ranges.len() as u32;

    let mut outcome = RipOutcome {
        segments: Vec::with_capacity(ranges.len()),
        archive: manifest.side_archive.as_ref().map(|f| session_dir.join(f)),
        spill_removed: false,
        replaced_removed: 0,
    };

    for (index, range) in ranges.iter().enumerate() {
        let entry = &manifest.tracks[index];
        let file_name = entry
            .encoded_file
            .clone()
            .unwrap_or_else(|| segment_file_name(index, &entry.meta, manifest.split_generation));
        let file = session_dir.join(&file_name);

        if entry.library_uuid.is_some() {
            // Already imported by an earlier pass — idempotent skip.
            progress(CommitProgress::Finished {
                index,
                imported: true,
            });
            outcome.segments.push(SegmentOutcome {
                index,
                file,
                library_uuid: entry.library_uuid.clone(),
                error: None,
            });
            continue;
        }
        progress(CommitProgress::Started { index });

        let start = usize::try_from(range.start * 2).unwrap_or(usize::MAX);
        let end = usize::try_from(range.end * 2).unwrap_or(usize::MAX);
        let pcm = &samples[start..end];
        #[allow(clippy::cast_possible_truncation)]
        let tags = tags_for(&entry.meta, index as u32 + 1, track_total);

        let result = commit_segment(library, pcm, sample_rate, &file, &tags);
        let (library_uuid, error) = match result {
            Ok((uuid, analysis_error)) => (Some(uuid), analysis_error),
            Err(e) => (None, Some(e)),
        };

        let entry = &mut manifest.tracks[index];
        if library_uuid.is_some() {
            entry.encoded_file = Some(file_name);
            entry.library_uuid.clone_from(&library_uuid);
        }
        manifest::save(session_dir, manifest)?;

        progress(CommitProgress::Finished {
            index,
            imported: library_uuid.is_some(),
        });
        outcome.segments.push(SegmentOutcome {
            index,
            file,
            library_uuid,
            error,
        });
    }

    if manifest.side_archive.is_none() {
        progress(CommitProgress::ArchiveStarted);
        let archive = session_dir.join(ARCHIVE_FILE);
        match encode_flac_24bit(&samples, sample_rate, 2, &archive) {
            Ok(()) => {
                manifest.side_archive = Some(ARCHIVE_FILE.to_string());
                manifest::save(session_dir, manifest)?;
                outcome.archive = Some(archive);
            }
            Err(e) => {
                // Archive failure is not fatal — the spill stays as
                // the archive until a retry succeeds.
                if let Some(seg) = outcome.segments.first_mut() {
                    let note = format!("side archive failed: {e}");
                    seg.error = Some(match seg.error.take() {
                        Some(prev) => format!("{prev}; {note}"),
                        None => note,
                    });
                }
            }
        }
    }

    // M26b re-split: the tracks an earlier split imported come out of
    // the library only now, with the replacement safely in. A failure
    // above leaves them in place — the operator keeps the old split
    // rather than losing both.
    if outcome.is_complete() && !manifest.replaced_uuids.is_empty() {
        // A re-split reuses segment file names, and library identity is
        // (volume, relative path) — so a new segment written to an old
        // segment's path comes back with that track's UUID. Removing
        // it here would delete the track this very commit imported.
        let kept: Vec<&str> = outcome
            .segments
            .iter()
            .filter_map(|s| s.library_uuid.as_deref())
            .collect();
        manifest
            .replaced_uuids
            .retain(|u| !kept.contains(&u.as_str()));

        let mut removed = Vec::new();
        for uuid in &manifest.replaced_uuids {
            // A track the operator already deleted by hand is not an
            // error; the goal state is "it is gone", and it is.
            match library.delete_track(uuid) {
                Ok(()) => removed.push(uuid.clone()),
                Err(dub_library::LibraryError::TrackNotFound { .. }) => removed.push(uuid.clone()),
                Err(e) => {
                    if let Some(seg) = outcome.segments.first_mut() {
                        let note = format!("could not remove the replaced track {uuid}: {e}");
                        seg.error = Some(match seg.error.take() {
                            Some(prev) => format!("{prev}; {note}"),
                            None => note,
                        });
                    }
                }
            }
        }
        manifest.replaced_uuids.retain(|u| !removed.contains(u));
        outcome.replaced_removed = removed.len();
        manifest::save(session_dir, manifest)?;
        // Segment files the new plan no longer references would
        // otherwise sit in a folder under ~/Music with no library row
        // — and get picked up as tracks by the next filesystem scan.
        remove_orphaned_segments(session_dir, manifest);
    }

    if outcome.is_complete() && spill_path.exists() {
        std::fs::remove_file(spill_path)?;
        outcome.spill_removed = true;
    }
    Ok(outcome)
}

/// Delete encoded segments the current plan does not reference.
/// Never touches the side archive — that is the source a future
/// re-split reads from.
fn remove_orphaned_segments(session_dir: &Path, manifest: &RipManifest) {
    let live: Vec<&str> = manifest
        .tracks
        .iter()
        .filter_map(|t| t.encoded_file.as_deref())
        .collect();
    let Ok(entries) = std::fs::read_dir(session_dir) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == ARCHIVE_FILE || !name.to_ascii_lowercase().ends_with(".flac") {
            continue;
        }
        if !live.contains(&name) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Encode + tag + import + pre-analyze one segment. Returns the
/// library UUID and, separately, a non-fatal analysis error (the
/// track is imported either way; analysis re-runs lazily on load).
fn commit_segment(
    library: &mut Library,
    pcm: &[f32],
    sample_rate: u32,
    file: &Path,
    tags: &TrackTags,
) -> Result<(String, Option<String>), String> {
    encode_flac_24bit(pcm, sample_rate, 2, file).map_err(|e| format!("encode failed: {e}"))?;
    write_tags(file, tags).map_err(|e| format!("tagging failed: {e}"))?;

    let imported = import_file(library, file).map_err(|e| format!("import failed: {e}"))?;

    let analysis_error = analyze_segment(library, &imported.uuid, pcm, sample_rate).err();
    Ok((imported.uuid, analysis_error))
}

/// Pre-analyze from the PCM already in hand (FLAC is lossless, so
/// this is bit-identical to decoding the file back). Populates
/// fingerprint, beat grid, key, LUFS, and the waveform sidecar so
/// ripped tracks arrive in the library "prepared".
fn analyze_segment(
    library: &Library,
    uuid: &str,
    pcm: &[f32],
    sample_rate: u32,
) -> Result<(), String> {
    let job = library
        .analyze_prepare(uuid)
        .map_err(|e| format!("analysis prepare failed: {e}"))?;
    let track = dub_io::Track::from_interleaved(pcm.to_vec(), sample_rate, 2)
        .ok_or_else(|| "analysis input rejected by dub-io".to_string())?;
    let computed = analyze_compute_with_track(&job, &track)
        .map_err(|e| format!("analysis compute failed: {e}"))?;
    library
        .analyze_commit(&job, computed)
        .map_err(|e| format!("analysis commit failed: {e}"))?;
    Ok(())
}

fn tags_for(meta: &crate::plan::TrackMeta, track_number: u32, track_total: u32) -> TrackTags {
    TrackTags {
        title: meta.title.clone(),
        artist: meta.artist.clone(),
        album: meta.album.clone(),
        year: meta.year,
        genre: meta.genre.clone(),
        track_number: Some(track_number),
        track_total: Some(track_total),
        ..TrackTags::default()
    }
}

/// `NN Artist - Title.flac`, falling back to `NN Track.flac` for
/// untagged segments. Sanitized for the filesystem; the library
/// derives `filename`-source metadata from this, so keep it human.
fn segment_file_name(index: usize, meta: &crate::plan::TrackMeta, generation: u32) -> String {
    let n = index + 1;
    let stem = match (&meta.artist, &meta.title) {
        (Some(artist), Some(title)) => format!("{artist} - {title}"),
        (None, Some(title)) => title.clone(),
        (Some(artist), None) => format!("{artist} - Track {n}"),
        (None, None) => format!("Track {n}"),
    };
    let stem = sanitize_file_stem(&stem);
    // A re-split must not land on the previous split's path: library
    // identity is (volume, relative path), so the same path would hand
    // the new audio the old track's row, cues and history included.
    if generation > 1 {
        format!("{n:02} {stem} (v{generation}).flac")
    } else {
        format!("{n:02} {stem}.flac")
    }
}

/// Strip path separators and control characters; collapse the result
/// so a metadata string can never escape the session directory.
fn sanitize_file_stem(stem: &str) -> String {
    let cleaned: String = stem
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0'..='\x1f' => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "Track".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Read the spill for encoding.
///
/// Goes through the salvage reader, not `hound`'s iterator: a session
/// recovered after a crash has a header that never learned how long
/// the recording was, and committing what the header claims would
/// throw the side away at the last step. Format validation lives in
/// [`crate::salvage::probe`].
///
/// Falls back to the lossless side archive when the spill is gone: a
/// committed session keeps only `side.flac`, and that is what a
/// re-split re-encodes from (M26b).
fn read_spill(spill_path: &Path, archive_path: &Path) -> Result<(Vec<f32>, u32), RipError> {
    if spill_path.exists() {
        let (samples, info) = crate::salvage::read_all(spill_path)?;
        return Ok((samples, info.sample_rate));
    }
    let track = dub_io::Track::load_from_path(archive_path).map_err(|e| {
        RipError::SpillUnreadable(format!(
            "no spill and the side archive is unreadable ({}): {e}",
            archive_path.display()
        ))
    })?;
    if track.channels() != 2 {
        return Err(RipError::SpillUnreadable(format!(
            "side archive is {} ch, expected stereo",
            track.channels()
        )));
    }
    Ok((track.samples().to_vec(), track.sample_rate()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::TrackMeta;

    #[test]
    fn file_name_uses_artist_and_title_when_present() {
        let meta = TrackMeta {
            artist: Some("Sound Dimension".into()),
            title: Some("Real Rock".into()),
            ..TrackMeta::default()
        };
        assert_eq!(
            segment_file_name(0, &meta, 1),
            "01 Sound Dimension - Real Rock.flac"
        );
    }

    #[test]
    fn file_name_falls_back_per_missing_field() {
        assert_eq!(
            segment_file_name(2, &TrackMeta::default(), 1),
            "03 Track 3.flac"
        );
        let title_only = TrackMeta {
            title: Some("Version".into()),
            ..TrackMeta::default()
        };
        assert_eq!(segment_file_name(9, &title_only, 1), "10 Version.flac");
        // A re-split writes beside the previous split, never over it.
        assert_eq!(segment_file_name(9, &title_only, 2), "10 Version (v2).flac");
    }

    #[test]
    fn file_name_cannot_escape_the_session_dir() {
        let hostile = TrackMeta {
            artist: Some("../../etc".into()),
            title: Some("pass/wd:x".into()),
            ..TrackMeta::default()
        };
        let name = segment_file_name(0, &hostile, 1);
        assert!(!name.contains('/') && !name.contains('\\') && !name.contains(':'));
        assert!(!name.starts_with('.'));
    }

    #[test]
    fn empty_metadata_sanitizes_to_a_usable_stem() {
        assert_eq!(sanitize_file_stem("///"), "---");
        assert_eq!(sanitize_file_stem("  .  "), "Track");
    }
}
