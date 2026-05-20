//! Real-music BPM regression corpus (M11c.3 investigation).
//!
//! The synthetic `genre_octave.rs` tests prove the M8.1 algorithm
//! resolves a hand-crafted "hi-hat at 2× period" pattern at the
//! correct octave. Real mastered rap / hip-hop catalogs still hit
//! the octave error because the mix has more energy at the 2×
//! period than our synthetic kick-and-hi-hat fixture. This test
//! is the regression gate against *real* audio: when we ship a
//! picker improvement, the fail rate here must drop without
//! breaking the synthetic suite.
//!
//! # How to seed the corpus
//!
//! The corpus is intentionally *not* committed to the repo (the
//! audio files are large and most are not redistributable). Each
//! contributor maintains their own TSV manifest pointing at their
//! local audio. The manifest format matches the
//! `diagnose_bpm --manifest` CLI in the sibling `examples/`
//! directory:
//!
//! ```text
//! path<TAB>expected_bpm<TAB>notes
//! /Users/me/Music/track.mp3<TAB>95<TAB>west coast rap
//! ```
//!
//! Then point the test at the manifest:
//!
//! ```sh
//! DUB_BPM_REAL_CORPUS=/path/to/corpus.tsv \
//!   cargo nextest run -p dub-bpm real_music_corpus -- --nocapture
//! ```
//!
//! Without the env var the test silently no-ops so CI stays green
//! on machines that don't have the audio.
//!
//! # What this test asserts
//!
//! For every track in the manifest, `analyze_beat_grid` must
//! produce a BPM within 5 % of `expected_bpm`. Octave-error
//! mismatches (detected ≈ 2 × expected, 1/2 × expected, etc.) are
//! reported separately in the failure output so the fix surface
//! is obvious.
//!
//! The 5 % tolerance is the same as `examples/diagnose_bpm.rs`,
//! deliberately loose so human-curated `expected_bpm` values
//! ("95") match the algorithm's parabolic-refined value ("95.43").
//! It still rejects every octave error (½× = 47.5, 2× = 190 are
//! both well outside ±5 % of 95).

use std::env;
use std::fs;
use std::path::PathBuf;

use dub_bpm::analyze_beat_grid;
use dub_io::Track;

/// Same tolerance the diagnostic CLI uses.
const MATCH_TOLERANCE_FRACTION: f64 = 0.05;

/// Env-var name the test consults for the manifest path. Documented
/// in the module-level docs so future readers don't have to grep
/// for `env::var`.
const MANIFEST_ENV: &str = "DUB_BPM_REAL_CORPUS";

/// One row of the parsed manifest. Mirrors the diagnostic CLI's
/// internal `ManifestRow`; kept separate so the test crate stays
/// independent of the example binary.
struct ManifestRow {
    path: PathBuf,
    expected_bpm: f64,
    notes: String,
}

#[test]
fn real_music_corpus_matches_expected_bpm() {
    let Some(manifest_path) = env::var_os(MANIFEST_ENV) else {
        eprintln!(
            "{MANIFEST_ENV} not set; skipping real-music corpus regression. \
             See `crates/dub-bpm/tests/real_music_corpus.rs` for the manifest \
             schema."
        );
        return;
    };
    let manifest_path = PathBuf::from(manifest_path);

    let rows = read_manifest(&manifest_path)
        .unwrap_or_else(|e| panic!("read manifest {}: {e}", manifest_path.display()));

    assert!(
        !rows.is_empty(),
        "manifest {} has no track rows",
        manifest_path.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for row in &rows {
        let track = match Track::load_from_path(&row.path) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("LOAD-FAIL  {}  ({e})", row.path.display()));
                continue;
            }
        };
        let grid = match analyze_beat_grid(track.samples(), track.sample_rate(), track.channels()) {
            Ok(g) => g,
            Err(e) => {
                failures.push(format!("ANALYSIS-FAIL  {}  ({e})", row.path.display()));
                continue;
            }
        };

        let detected = grid.bpm;
        let expected = row.expected_bpm;
        if approx_equal(detected, expected) {
            continue;
        }

        let octave_tag = octave_label(detected, expected);
        let notes = if row.notes.is_empty() {
            String::new()
        } else {
            format!("  [{}]", row.notes)
        };
        failures.push(format!(
            "{:<8}  expect={expected:>7.2}  detect={detected:>7.2}  conf={:.3}  {}{notes}",
            octave_tag,
            grid.confidence,
            row.path.display(),
        ));
    }

    if !failures.is_empty() {
        let header = format!(
            "real-music corpus: {} of {} track(s) failed",
            failures.len(),
            rows.len()
        );
        panic!("{header}\n{}", failures.join("\n"));
    }
}

fn read_manifest(path: &std::path::Path) -> Result<Vec<ManifestRow>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let mut rows = Vec::new();
    let mut saw_header = false;
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if !saw_header {
            saw_header = true;
            if cols.first().map(|c| c.eq_ignore_ascii_case("path")) != Some(true) {
                return Err(format!(
                    "line {}: expected TSV header `path<TAB>expected_bpm<TAB>notes`",
                    lineno + 1
                ));
            }
            continue;
        }
        if cols.len() < 2 {
            return Err(format!(
                "line {}: expected at least 2 tab-separated columns",
                lineno + 1
            ));
        }
        let path = PathBuf::from(cols[0]);
        let expected_bpm = cols[1]
            .trim()
            .parse::<f64>()
            .map_err(|e| format!("line {}: bad expected_bpm `{}`: {e}", lineno + 1, cols[1]))?;
        let notes = cols
            .get(2)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        rows.push(ManifestRow {
            path,
            expected_bpm,
            notes,
        });
    }
    Ok(rows)
}

fn approx_equal(a: f64, b: f64) -> bool {
    if !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return false;
    }
    (a - b).abs() / b <= MATCH_TOLERANCE_FRACTION
}

fn octave_label(detected: f64, expected: f64) -> &'static str {
    for &(factor, label) in &[
        (2.0, "OCTAVE-2x"),
        (0.5, "OCTAVE-1/2"),
        (3.0, "OCTAVE-3x"),
        (1.0 / 3.0, "OCTAVE-1/3"),
        (4.0, "OCTAVE-4x"),
        (0.25, "OCTAVE-1/4"),
    ] {
        if approx_equal(detected, expected * factor) {
            return label;
        }
    }
    "FAIL"
}
