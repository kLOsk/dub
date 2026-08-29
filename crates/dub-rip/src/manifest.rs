//! `rip.json` — the crash-safe session manifest.
//!
//! Rewritten atomically (tmp + rename) on every mutation so a crash
//! mid-rip loses at most the last edit, never the whole plan. The
//! recovery path (`RipSession` reopening a session dir) and the
//! idempotent commit retry (segments with a `library_uuid` are
//! skipped) both read this file.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use crate::plan::TrackMeta;

/// Manifest schema version. Bump on breaking shape changes; the
/// loader rejects unknown versions rather than misreading them.
pub const MANIFEST_VERSION: u32 = 1;

/// Name of the manifest file inside a session directory.
pub const MANIFEST_FILE: &str = "rip.json";

/// One segment's manifest entry: metadata + commit progress.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrackEntry {
    /// User/recognition metadata for this segment.
    pub meta: TrackMeta,
    /// Encoded file path relative to the session dir, once encoded.
    pub encoded_file: Option<String>,
    /// Canonical library track UUID once imported. Presence makes
    /// the commit retry skip this segment.
    pub library_uuid: Option<String>,
}

/// The `rip.json` payload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RipManifest {
    /// Schema version — see [`MANIFEST_VERSION`].
    pub version: u32,
    /// Capture sample rate in Hz.
    pub sample_rate: u32,
    /// Interleaved channel count (2 for the stereo record tap).
    pub channels: u16,
    /// Total recorded frames in the WAV spill / side archive.
    pub recorded_frames: u64,
    /// Split boundaries in frames (frame where the next track
    /// starts); N boundaries → N+1 segments.
    pub boundaries_frames: Vec<u64>,
    /// Per-segment metadata + commit progress. Length is
    /// `boundaries_frames.len() + 1` once splits are set.
    pub tracks: Vec<TrackEntry>,
    /// Relative path of the lossless side archive once written
    /// (`side.flac`).
    pub side_archive: Option<String>,
    /// Library UUIDs from an earlier split of this same side, waiting
    /// to be removed once the new split has fully imported (M26b
    /// re-split). Kept in the manifest rather than in memory so a
    /// crash mid-re-split cannot strand the old tracks invisibly.
    ///
    /// `serde(default)` — manifests written before M26b simply have
    /// none, so the schema version does not move.
    #[serde(default)]
    pub replaced_uuids: Vec<String>,
    /// How many times this side has been split. 0/1 is the original
    /// commit; each re-split increments it and suffixes the segment
    /// file names, so a re-split produces genuinely new tracks rather
    /// than overwriting the old ones in place.
    ///
    /// Library identity is (volume, relative path): reusing a path
    /// would reuse the track row, dragging the old split's hot cues
    /// and play history onto audio with different boundaries.
    #[serde(default)]
    pub split_generation: u32,
    /// Frame the *side* ends at, when the detector found the run-out.
    /// The last segment stops here and the groove noise behind it is
    /// discarded; `None` means the whole recording is the side, which
    /// is what a manual rip gets.
    ///
    /// The audio is not lost — `side.flac` archives the full capture,
    /// so a re-split can always reach back past this.
    ///
    /// `serde(default)` — pre-M26b manifests simply have none.
    #[serde(default)]
    pub side_end_frame: Option<u64>,
    /// Frame the *side* starts at, when the detector found the lead-in
    /// groove. The first segment starts here and the dead air, needle
    /// drop and lead-in groove before it are discarded; `None` means
    /// the side starts at the top of the recording, which is what a
    /// manual rip gets.
    ///
    /// Recoverable like the tail: `side.flac` archives the whole
    /// capture.
    ///
    /// `serde(default)` — pre-M26b manifests simply have none.
    #[serde(default)]
    pub side_start_frame: Option<u64>,
}

impl RipManifest {
    /// Frame the last track ends at: the detected end of the side, or
    /// the end of the recording when nothing trimmed it.
    #[must_use]
    pub fn side_end(&self) -> u64 {
        self.side_end_frame
            .filter(|&end| end > 0 && end <= self.recorded_frames)
            .unwrap_or(self.recorded_frames)
    }

    /// Frame the first track starts at: the detected start of the
    /// side, or 0 when nothing trimmed it.
    #[must_use]
    pub fn side_start(&self) -> u64 {
        self.side_start_frame
            .filter(|&start| start < self.side_end())
            .unwrap_or(0)
    }

    /// Fresh manifest for a new capture session.
    #[must_use]
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            version: MANIFEST_VERSION,
            sample_rate,
            channels,
            recorded_frames: 0,
            boundaries_frames: Vec::new(),
            tracks: Vec::new(),
            side_archive: None,
            replaced_uuids: Vec::new(),
            split_generation: 1,
            side_end_frame: None,
            side_start_frame: None,
        }
    }
}

/// Errors from manifest persistence.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest io failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("manifest malformed: {0}")]
    Malformed(#[from] serde_json::Error),

    #[error("manifest version {found} is newer than supported {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
}

/// Atomically write the manifest into `session_dir/rip.json`:
/// serialize to `rip.json.tmp`, fsync, rename over the old file. A
/// crash at any point leaves either the previous manifest or the new
/// one — never a torn file.
pub fn save(session_dir: &Path, manifest: &RipManifest) -> Result<(), ManifestError> {
    let tmp = session_dir.join("rip.json.tmp");
    let dst = session_dir.join(MANIFEST_FILE);
    let payload = serde_json::to_vec_pretty(manifest)?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&payload)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &dst)?;
    Ok(())
}

/// Load `session_dir/rip.json`.
pub fn load(session_dir: &Path) -> Result<RipManifest, ManifestError> {
    let bytes = fs::read(session_dir.join(MANIFEST_FILE))?;
    let manifest: RipManifest = serde_json::from_slice(&bytes)?;
    if manifest.version > MANIFEST_VERSION {
        return Err(ManifestError::UnsupportedVersion {
            found: manifest.version,
            supported: MANIFEST_VERSION,
        });
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RipManifest {
        let mut m = RipManifest::new(48_000, 2);
        m.recorded_frames = 1_000_000;
        m.boundaries_frames = vec![400_000];
        m.tracks = vec![
            TrackEntry {
                meta: TrackMeta {
                    title: Some("Real Rock".into()),
                    artist: Some("Sound Dimension".into()),
                    ..TrackMeta::default()
                },
                encoded_file: Some("01 Sound Dimension - Real Rock.flac".into()),
                library_uuid: Some("abc-123".into()),
            },
            TrackEntry::default(),
        ];
        m
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let m = sample();
        save(dir.path(), &m).unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded, m);
    }

    #[test]
    fn save_replaces_previous_manifest_atomically() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &RipManifest::new(48_000, 2)).unwrap();
        let m = sample();
        save(dir.path(), &m).unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded, m);
        assert!(
            !dir.path().join("rip.json.tmp").exists(),
            "tmp file must not survive a successful save"
        );
    }

    #[test]
    fn load_rejects_newer_version() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = sample();
        m.version = MANIFEST_VERSION + 1;
        save(dir.path(), &m).unwrap();
        let err = load(dir.path()).unwrap_err();
        assert!(matches!(err, ManifestError::UnsupportedVersion { .. }));
    }

    #[test]
    fn load_rejects_torn_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MANIFEST_FILE), b"{\"version\": 1, ").unwrap();
        let err = load(dir.path()).unwrap_err();
        assert!(matches!(err, ManifestError::Malformed(_)));
    }
}
