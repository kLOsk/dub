//! Beat-*location* regression corpus.
//!
//! The sibling `real_music_corpus.rs` scores tempo only; it cannot catch a
//! phase or downbeat regression (a grid can carry the right BPM and still
//! sit on the off-beat). This harness grades the actual beat positions
//! `analyze_beat_grid` produces against ground-truth annotations, using the
//! standard metrics in [`dub_bpm::eval`] — the prerequisite the field
//! survey flagged for evaluating any grid-placement change
//! (`docs/investigations/BPM-DETECTOR-V2-INVESTIGATION.md` §7.4).
//!
//! # How to seed the corpus
//!
//! Like the BPM corpus, the audio and annotations are intentionally *not*
//! committed. Point the test at a TSV manifest:
//!
//! ```sh
//! DUB_BPM_BEAT_CORPUS=/path/to/beats.tsv \
//!   cargo test -p dub-bpm --test beat_location_corpus -- --nocapture
//! ```
//!
//! Manifest format (tab-separated; a leading `path` header row is
//! optional, `#` lines ignored):
//!
//! ```text
//! audio_path<TAB>beats_path[<TAB>profile]
//! /music/track.flac<TAB>/annot/track.beats<TAB>roots
//! ```
//!
//! `beats_path` is a plain-text annotation file with one beat time in
//! seconds per line (an optional second column — e.g. the beat-in-bar
//! number — is ignored), the MIREX/`.beats` convention. Without the env
//! var the test no-ops so CI stays green on machines without the data.
//!
//! # What it asserts
//!
//! The corpus-mean F-measure (±70 ms) must stay at or above
//! [`MEAN_F_MEASURE_FLOOR`]. Per-track F-measure, CMLt, and AMLt are
//! printed so a regression's nature (phase slip vs octave/off-beat) is
//! visible. Adjust the floor as the corpus and detector evolve.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use dub_bpm::{analyze_beat_grid_with_profile, eval, octave_profile_from_label, OctaveProfile};
use dub_io::Track;

const MANIFEST_ENV: &str = "DUB_BPM_BEAT_CORPUS";

/// Corpus-mean F-measure floor. Lenient on purpose; tighten as the
/// annotated corpus grows and the detector improves.
const MEAN_F_MEASURE_FLOOR: f64 = 0.5;

struct ManifestRow {
    audio: PathBuf,
    beats: PathBuf,
    profile: OctaveProfile,
}

#[test]
fn beat_location_corpus_tracks_annotations() {
    let Some(manifest_path) = env::var_os(MANIFEST_ENV) else {
        eprintln!(
            "{MANIFEST_ENV} not set; skipping beat-location corpus. See \
             `crates/dub-bpm/tests/beat_location_corpus.rs` for the manifest schema."
        );
        return;
    };
    let manifest_path = PathBuf::from(manifest_path);
    let rows = read_manifest(&manifest_path)
        .unwrap_or_else(|e| panic!("read manifest {}: {e}", manifest_path.display()));
    assert!(
        !rows.is_empty(),
        "manifest {} has no rows",
        manifest_path.display()
    );

    let mut f_sum = 0.0;
    let mut scored = 0usize;
    let mut report: Vec<String> = Vec::new();

    for row in &rows {
        let track = match Track::load_from_path(&row.audio) {
            Ok(t) => t,
            Err(e) => {
                report.push(format!("LOAD-FAIL  {}  ({e})", row.audio.display()));
                continue;
            }
        };
        let grid = match analyze_beat_grid_with_profile(
            track.samples(),
            track.sample_rate(),
            track.channels(),
            row.profile,
        ) {
            Ok(g) => g,
            Err(e) => {
                report.push(format!("ANALYSIS-FAIL  {}  ({e})", row.audio.display()));
                continue;
            }
        };

        let annotated = match read_annotation(&row.beats) {
            Ok(b) => b,
            Err(e) => {
                report.push(format!("ANNOT-FAIL  {}  ({e})", row.beats.display()));
                continue;
            }
        };

        let f = eval::f_measure(&grid.beats, &annotated, eval::F_MEASURE_TOL);
        let c = eval::continuity(
            &grid.beats,
            &annotated,
            eval::CONTINUITY_PHASE_TOL,
            eval::CONTINUITY_PERIOD_TOL,
        );
        f_sum += f;
        scored += 1;
        report.push(format!(
            "F={f:.3}  CMLt={:.3}  AMLt={:.3}  bpm={:>7.2}  {}",
            c.cmlt,
            c.amlt,
            grid.bpm,
            row.audio.display()
        ));
    }

    for line in &report {
        eprintln!("{line}");
    }
    assert!(
        scored > 0,
        "no tracks could be scored (all loads/analyses failed)"
    );

    #[allow(clippy::cast_precision_loss)]
    let mean_f = f_sum / scored as f64;
    eprintln!("beat-location corpus: mean F-measure = {mean_f:.3} over {scored} track(s)");
    assert!(
        mean_f >= MEAN_F_MEASURE_FLOOR,
        "mean F-measure {mean_f:.3} fell below floor {MEAN_F_MEASURE_FLOOR:.3}"
    );
}

fn read_manifest(path: &Path) -> Result<Vec<ManifestRow>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let mut rows = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        // Tolerate an optional `path`/`audio` header row.
        if lineno == 0
            && cols
                .first()
                .map(|c| c.eq_ignore_ascii_case("path") || c.eq_ignore_ascii_case("audio"))
                == Some(true)
        {
            continue;
        }
        if cols.len() < 2 {
            return Err(format!(
                "line {}: expected `audio_path<TAB>beats_path[<TAB>profile]`",
                lineno + 1
            ));
        }
        let profile = cols
            .get(2)
            .map(|p| octave_profile_from_label(p))
            .unwrap_or(OctaveProfile::Default);
        rows.push(ManifestRow {
            audio: PathBuf::from(cols[0]),
            beats: PathBuf::from(cols[1]),
            profile,
        });
    }
    Ok(rows)
}

/// Read a `.beats` annotation file: one beat time in seconds per line,
/// ignoring any trailing columns and `#` comments.
fn read_annotation(path: &Path) -> Result<Vec<f64>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let mut beats = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let first = line.split([' ', '\t', ',']).next().unwrap_or("");
        let t: f64 = first
            .parse()
            .map_err(|e| format!("parse beat time {first:?}: {e}"))?;
        beats.push(t);
    }
    if beats.is_empty() {
        return Err("annotation file has no beat times".into());
    }
    Ok(beats)
}
