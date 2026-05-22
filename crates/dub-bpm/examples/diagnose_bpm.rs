//! Diagnostic binary: report the BPM analyzer's verdict on real audio
//! files, plus the verdict under several narrowed `BpmRange`s to
//! expose octave-error behaviour.
//!
//! Built as part of the M11c.3 investigation (real-music BPM octave
//! errors): the synthetic `genre_octave` corpus passes, but real
//! mastered rap / hip-hop catalogs report at 2x the audible tempo.
//! This example loads each input file, runs `analyze_beat_grid` at
//! the default 60-200 BPM range and at several narrowed half/full
//! windows, and prints the scoring so we can confirm which candidate
//! the picker is choosing and how the alternative scores.
//!
//! # Usage
//!
//! Single file (one-off diagnosis):
//!
//! ```sh
//! cargo run --release --example diagnose_bpm -- path/to/file.mp3
//! ```
//!
//! Manifest (real-music regression corpus): TSV with one row per
//! track. Header row is required so columns can be reordered later
//! without breaking older manifests. Lines starting with `#` and
//! empty lines are ignored.
//!
//! ```text
//! path<TAB>expected_bpm<TAB>notes
//! /abs/path/track.mp3<TAB>95.0<TAB>west coast rap, hi-hat heavy
//! ```
//!
//! Then:
//!
//! ```sh
//! cargo run --release --example diagnose_bpm -- --manifest /path/to/corpus.tsv
//! ```
//!
//! Folder sweep (batch diagnosis of every audio file in a directory):
//!
//! ```sh
//! cargo run --release --example diagnose_bpm -- --folder /path/to/DrumnBass
//! ```
//!
//! Set `DUB_BPM_DEBUG=1` to dump per-lag PASS1/PASS2 scoring from
//! `estimate_tempo` on stderr (single-file / verbose modes only).
//!
//! The manifest mode prints per-track pass/fail (±5 % of expected,
//! halving the detected value when needed to absorb the octave
//! error we're hunting) and a final aggregate. Exit code is non-zero
//! iff any expected row failed.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use dub_bpm::{
    analyze_beat_grid, analyze_beat_grid_with_profile, analyze_bpm_with_range,
    octave_profile_from_label, BpmRange, OctaveProfile,
};
use dub_io::Track;

/// Acceptance tolerance for "the detected BPM matches the expected
/// BPM". 5 % is loose enough to absorb the natural shoulder-width
/// of the autocorrelation peak on real percussion plus the rounding
/// of human-curated `expected_bpm` values (often integer where the
/// track is `95.43` or similar).
const MATCH_TOLERANCE_FRACTION: f64 = 0.05;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        return ExitCode::from(2);
    }

    if args[0] == "--manifest" {
        let Some(manifest_path) = args.get(1) else {
            eprintln!("--manifest requires a path argument");
            print_usage();
            return ExitCode::from(2);
        };
        return run_manifest(Path::new(manifest_path));
    }

    if args[0] == "--folder" {
        let Some(folder_path) = args.get(1) else {
            eprintln!("--folder requires a path argument");
            print_usage();
            return ExitCode::from(2);
        };
        let profile = args
            .get(2)
            .map(|label| octave_profile_from_label(label))
            .unwrap_or(OctaveProfile::Default);
        return run_folder(Path::new(folder_path), profile);
    }

    let mut failed = false;
    for arg in &args {
        if let Err(err) = diagnose_one(Path::new(arg)) {
            eprintln!("\n!! {arg}: {err}");
            failed = true;
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print_usage() {
    eprintln!(
        "usage:\n  \
         diagnose_bpm <audio-file> [<audio-file>...]\n  \
         diagnose_bpm --manifest <corpus.tsv>\n  \
         diagnose_bpm --folder <directory> [profile-label]\n\n\
         env:\n  \
         DUB_BPM_DEBUG=1  dump per-lag PASS1/PASS2 scoring to stderr"
    );
}

/// Batch-scan every audio file directly under `folder` (non-recursive)
/// and print a compact `bpm  conf  filename` table on stdout.
fn run_folder(folder: &Path, profile: OctaveProfile) -> ExitCode {
    if !folder.is_dir() {
        eprintln!("--folder path is not a directory: {}", folder.display());
        return ExitCode::from(2);
    }

    let read = match fs::read_dir(folder) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("read_dir {}: {e}", folder.display());
            return ExitCode::from(2);
        }
    };

    let mut paths: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_audio_file(path))
        .collect();
    paths.sort();

    if paths.is_empty() {
        eprintln!("no audio files found in {}", folder.display());
        return ExitCode::from(2);
    }

    eprintln!(
        "scanning {} audio file(s) in {} (profile={profile:?})",
        paths.len(),
        folder.display()
    );

    let mut failed = false;
    for path in paths {
        match analyze_folder_row(&path, profile) {
            Ok(line) => println!("{line}"),
            Err(err) => {
                eprintln!("ERROR  {}  ({err})", path.display());
                failed = true;
            }
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "wav" | "aiff" | "aif" | "m4a" | "aac" | "alac"
            )
        })
}

fn analyze_folder_row(
    path: &Path,
    profile: OctaveProfile,
) -> Result<String, Box<dyn std::error::Error>> {
    let track = Track::load_from_path(path)?;
    let grid = analyze_beat_grid_with_profile(
        track.samples(),
        track.sample_rate(),
        track.channels(),
        profile,
    )?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    Ok(format!(
        "{:<8.3}  {:<6.4}  {}",
        grid.bpm, grid.confidence, name
    ))
}

/// One-off diagnostic on a single file: dump the picker's verdict
/// and the verdict under several narrow ranges that bracket the
/// half / double tempi we'd expect on mainstream urban material.
fn diagnose_one(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n==============================================================");
    println!("file: {}", path.display());
    println!("==============================================================");

    let load_start = std::time::Instant::now();
    let track = Track::load_from_path(path)?;
    let load_ms = load_start.elapsed().as_millis();
    let duration_secs =
        track.samples().len() as f64 / f64::from(track.sample_rate()) / f64::from(track.channels());
    println!(
        "  loaded in {load_ms} ms ({:.1}s, {} Hz, {} ch)",
        duration_secs,
        track.sample_rate(),
        track.channels()
    );

    let bg_start = std::time::Instant::now();
    let grid = analyze_beat_grid(track.samples(), track.sample_rate(), track.channels())?;
    let bg_ms = bg_start.elapsed().as_millis();
    println!("\n  analyze_beat_grid (default 60-200 BPM):");
    println!("    bpm:         {:.3}", grid.bpm);
    println!("    confidence:  {:.4}", grid.confidence);
    println!("    beats:       {}", grid.beats.len());
    println!("    elapsed:     {bg_ms} ms");

    let sweeps = [
        ("60-110  (slow hip-hop / reggae half-time)", 60.0, 110.0),
        ("80-100  (classic rap)", 80.0, 100.0),
        ("85-105  (g-funk / west coast)", 85.0, 105.0),
        ("130-180 (full tempo / drum-n-bass)", 130.0, 180.0),
    ];
    println!("\n  forced-range sweep (analyze_bpm_with_range):");
    for (label, lo, hi) in sweeps {
        let range = match BpmRange::new(lo, hi) {
            Ok(r) => r,
            Err(e) => {
                println!("    {label:42}  bad range: {e}");
                continue;
            }
        };
        match analyze_bpm_with_range(
            track.samples(),
            track.sample_rate(),
            track.channels(),
            range,
        ) {
            Ok(est) => {
                println!(
                    "    {label:42}  bpm={:.2}  conf={:.3}",
                    est.bpm, est.confidence
                );
            }
            Err(e) => {
                println!("    {label:42}  err: {e}");
            }
        }
    }

    Ok(())
}

/// Per-row summary emitted by the manifest path. The integration
/// test (`tests/real_music_corpus.rs`) computes the same structure
/// over the same manifest format so the CLI and the test agree on
/// "what counts as a pass".
struct CorpusRow {
    path: PathBuf,
    expected_bpm: f64,
    notes: String,
    detected_bpm: f64,
    confidence: f32,
    /// True iff detected matches expected within
    /// [`MATCH_TOLERANCE_FRACTION`] *at the same octave*. Octave
    /// errors are accounted for separately so they show up as a
    /// distinct failure mode (the whole reason we're building the
    /// corpus).
    matched: bool,
    octave_error_factor: Option<f64>,
}

fn run_manifest(manifest_path: &Path) -> ExitCode {
    let manifest = match read_manifest(manifest_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("failed to read manifest {}: {e}", manifest_path.display());
            return ExitCode::FAILURE;
        }
    };

    if manifest.is_empty() {
        eprintln!("manifest {} has no rows", manifest_path.display());
        return ExitCode::from(2);
    }

    println!(
        "corpus: {} ({} track(s))\n",
        manifest_path.display(),
        manifest.len()
    );

    let mut rows: Vec<CorpusRow> = Vec::with_capacity(manifest.len());
    let mut had_load_error = false;
    for entry in manifest {
        match analyze_for_corpus(&entry.path, entry.expected_bpm, entry.notes) {
            Ok(row) => rows.push(row),
            Err(err) => {
                eprintln!("!! {}: load/analysis failed: {err}", entry.path.display());
                had_load_error = true;
            }
        }
    }

    print_corpus_summary(&rows);

    let any_failed = rows.iter().any(|r| !r.matched);
    if had_load_error || any_failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

struct ManifestRow {
    path: PathBuf,
    expected_bpm: f64,
    notes: String,
}

fn read_manifest(manifest_path: &Path) -> Result<Vec<ManifestRow>, String> {
    let text = fs::read_to_string(manifest_path).map_err(|e| format!("read failed: {e}"))?;

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

fn analyze_for_corpus(
    path: &Path,
    expected_bpm: f64,
    notes: String,
) -> Result<CorpusRow, Box<dyn std::error::Error>> {
    let track = Track::load_from_path(path)?;
    let grid = analyze_beat_grid(track.samples(), track.sample_rate(), track.channels())?;

    let (matched, octave_error_factor) = classify(grid.bpm, expected_bpm);

    Ok(CorpusRow {
        path: path.to_path_buf(),
        expected_bpm,
        notes,
        detected_bpm: grid.bpm,
        confidence: grid.confidence,
        matched,
        octave_error_factor,
    })
}

/// Returns `(matched_at_octave, octave_factor_if_distinct_octave_match)`.
///
/// `matched` is true iff `detected` is within
/// [`MATCH_TOLERANCE_FRACTION`] of `expected` directly. When the
/// direct match fails, we check the 2x, 1/2, 3x, 1/3, 4x, 1/4 octave
/// candidates; if any of those matches, we report it as the octave
/// factor so the user sees the error mode in the summary table.
fn classify(detected: f64, expected: f64) -> (bool, Option<f64>) {
    if approx_equal(detected, expected) {
        return (true, None);
    }
    for &factor in &[2.0, 0.5, 3.0, 1.0 / 3.0, 4.0, 0.25] {
        if approx_equal(detected, expected * factor) {
            return (false, Some(factor));
        }
    }
    (false, None)
}

fn approx_equal(a: f64, b: f64) -> bool {
    if !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return false;
    }
    (a - b).abs() / b <= MATCH_TOLERANCE_FRACTION
}

fn print_corpus_summary(rows: &[CorpusRow]) {
    println!(
        "{:<6}  {:>8}  {:>8}  {:>6}  {:>8}  {:<8}  path",
        "result", "expect", "detect", "conf", "octave", "notes"
    );
    let mut pass = 0usize;
    let mut octave = 0usize;
    let mut other = 0usize;
    for row in rows {
        let (tag, octave_str) = if row.matched {
            pass += 1;
            ("PASS", String::from("-"))
        } else if let Some(factor) = row.octave_error_factor {
            octave += 1;
            (
                "OCTAVE",
                if (factor - 2.0).abs() < 1e-6 {
                    "2x".to_string()
                } else if (factor - 0.5).abs() < 1e-6 {
                    "1/2".to_string()
                } else {
                    format!("{factor:.2}x")
                },
            )
        } else {
            other += 1;
            ("FAIL", String::from("-"))
        };
        let filename = row
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| row.path.display().to_string());
        println!(
            "{tag:<6}  {:>8.2}  {:>8.2}  {:>6.3}  {octave_str:>8}  {:<8}  {filename}",
            row.expected_bpm,
            row.detected_bpm,
            row.confidence,
            truncate(&row.notes, 8),
        );
    }
    println!(
        "\n  {} pass / {} octave error / {} other failure / {} total",
        pass,
        octave,
        other,
        rows.len()
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}
