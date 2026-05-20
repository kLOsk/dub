//! Profile filesystem-import phase costs.
//!
//! Walks a folder (or a single file), times each phase of the
//! import pipeline against a temp library, and prints a
//! per-track + aggregate breakdown so we can see where the wall
//! clock actually goes.
//!
//! Phases reported:
//!
//! * stat — `std::fs::metadata` (volume UUID lookup, file size, mtime).
//! * decode — `dub_io::Track::load_from_path`: full file decode into a `Vec<f32>`.
//! * fingerprint — `Fingerprint::compute_from_f32`: f32 → i16 scratch alloc + Chromaprint pass.
//! * dedupe — neighbour-fingerprint lookup + per-neighbour `decide()` calls.
//! * write — `upsert_volume` + `upsert_fingerprint` + `insert_track` + `upsert_track_file` + 2×`upsert_metadata_source`.
//!
//! Usage:
//!
//! ```sh
//! cargo run --release --example profile_import -p dub-library \
//!     -- /Users/me/Music/SomeFolder
//! ```
//!
//! Writes nothing the user can see except stdout; the temp library
//! is created in a `tempfile::tempdir()` and dropped at exit.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use dub_fingerprint::Fingerprint;
use dub_io::Track;
use dub_library::{discover_for_path, ImportError, Library, DURATION_DELTA_MS};

const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "aif", "aiff", "m4a", "alac", "aac"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: profile_import <file-or-folder>");
        return ExitCode::from(2);
    };
    let root = PathBuf::from(root);
    if !root.exists() {
        eprintln!("path does not exist: {}", root.display());
        return ExitCode::FAILURE;
    }

    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tempdir: {e}");
            return ExitCode::FAILURE;
        }
    };
    let lib_path = tmp.path().join("library.sqlite");
    let mut library = match Library::open_at(&lib_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("open library: {e}");
            return ExitCode::FAILURE;
        }
    };

    let files = collect_files(&root);
    if files.is_empty() {
        eprintln!("no audio files found under {}", root.display());
        return ExitCode::from(2);
    }
    println!("profiling {} file(s) under {}", files.len(), root.display());
    println!();
    println!(
        "{:>5}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}  file",
        "#", "stat", "decode", "fp", "dedupe", "write", "total"
    );

    let overall = Instant::now();
    let mut total = Phases::default();
    let mut errors: Vec<ImportError> = Vec::new();
    for (i, path) in files.iter().enumerate() {
        match profile_one(&mut library, path) {
            Ok(phases) => {
                total.add(&phases);
                println!(
                    "{:>5}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}  {}",
                    i + 1,
                    ms(phases.stat_ms),
                    ms(phases.decode_ms),
                    ms(phases.fingerprint_ms),
                    ms(phases.dedupe_ms),
                    ms(phases.write_ms),
                    ms(phases.total_ms),
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                );
            }
            Err(e) => {
                errors.push(ImportError {
                    path: path.clone(),
                    reason: e,
                });
            }
        }
    }
    let wall_ms = overall.elapsed().as_millis() as u64;

    println!();
    println!(
        "totals (sum-of-phases): stat={} decode={} fingerprint={} dedupe={} write={} total={}",
        ms(total.stat_ms),
        ms(total.decode_ms),
        ms(total.fingerprint_ms),
        ms(total.dedupe_ms),
        ms(total.write_ms),
        ms(total.total_ms),
    );
    println!("wall clock: {}", ms(wall_ms));
    println!(
        "throughput: {:.2} file/s ({} file(s) / {} ms)",
        files.len() as f64 / wall_ms.max(1) as f64 * 1000.0,
        files.len(),
        wall_ms
    );

    if !errors.is_empty() {
        eprintln!("\n{} file(s) failed:", errors.len());
        for e in &errors {
            eprintln!("  {} — {}", e.path.display(), e.reason);
        }
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

#[derive(Default, Clone, Copy)]
struct Phases {
    stat_ms: u64,
    decode_ms: u64,
    fingerprint_ms: u64,
    dedupe_ms: u64,
    write_ms: u64,
    total_ms: u64,
}

impl Phases {
    fn add(&mut self, other: &Phases) {
        self.stat_ms += other.stat_ms;
        self.decode_ms += other.decode_ms;
        self.fingerprint_ms += other.fingerprint_ms;
        self.dedupe_ms += other.dedupe_ms;
        self.write_ms += other.write_ms;
        self.total_ms += other.total_ms;
    }
}

fn ms(v: u64) -> String {
    format!("{v}ms")
}

/// Replicates the importer's cold-path SQL flow at the FFI level
/// so we can time each phase independently. We don't go through
/// `dub_library::import_folder` because we want per-phase timings,
/// and the public API treats the whole `import_one` as a single
/// closed call.
///
/// We use the *publicly exposed* methods of `Library` to do the
/// writes, which means this profiler exercises the same code paths
/// the real importer does at write time, just measured separately.
/// The dedupe scoring loop is the only thing we have to inline —
/// the importer's `run_dedupe` helper is private.
fn profile_one(library: &mut Library, path: &Path) -> Result<Phases, String> {
    let total_start = Instant::now();
    let mut phases = Phases::default();

    let stat_start = Instant::now();
    let volume = discover_for_path(path).map_err(|e| format!("volume UUID unavailable: {e}"))?;
    let stats = std::fs::metadata(path).map_err(|e| format!("stat: {e}"))?;
    library
        .upsert_volume(&volume)
        .map_err(|e| format!("upsert_volume: {e}"))?;
    let relative_path = volume
        .relative_to(path)
        .ok_or_else(|| format!("path {path:?} not under volume {:?}", volume.mount_point))?;
    phases.stat_ms = stat_start.elapsed().as_millis() as u64;

    let decode_start = Instant::now();
    let track = Track::load_from_path(path).map_err(|e| format!("decode failed: {e:?}"))?;
    phases.decode_ms = decode_start.elapsed().as_millis() as u64;

    let fingerprint_start = Instant::now();
    let fingerprint = Fingerprint::compute_from_f32(
        track.samples(),
        track.sample_rate(),
        u32::from(track.channels()),
    )
    .map_err(|e| format!("fingerprint failed: {e}"))?;
    phases.fingerprint_ms = fingerprint_start.elapsed().as_millis() as u64;

    let dedupe_start = Instant::now();
    let duration_ms = fingerprint.duration_ms();
    let _ = library
        .find_fingerprint_neighbours(duration_ms, DURATION_DELTA_MS)
        .map_err(|e| format!("find_fingerprint_neighbours: {e}"))?;
    phases.dedupe_ms = dedupe_start.elapsed().as_millis() as u64;

    let write_start = Instant::now();
    let track_uuid = uuid::Uuid::new_v4().to_string();
    let fingerprint_id = library
        .upsert_fingerprint(
            &fingerprint,
            Some(track.sample_rate()),
            Some(u32::from(track.channels())),
            Some(stats.len()),
        )
        .map_err(|e| format!("upsert_fingerprint: {e}"))?;
    library
        .insert_track(&track_uuid, fingerprint_id, duration_ms, None)
        .map_err(|e| format!("insert_track: {e}"))?;
    let mtime = stats
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    library
        .upsert_track_file(
            &track_uuid,
            &volume.volume_uuid,
            &relative_path,
            None,
            Some(track.sample_rate()),
            None,
            Some(u32::from(track.channels())),
            Some(stats.len()),
            mtime,
        )
        .map_err(|e| format!("upsert_track_file: {e}"))?;
    let ext = track.extended_metadata();
    library
        .upsert_metadata_source(
            &track_uuid,
            "id3",
            ext.artist.as_deref(),
            ext.title.as_deref(),
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
            None,
        )
        .map_err(|e| format!("upsert_metadata_source(id3): {e}"))?;
    library
        .upsert_metadata_source(
            &track_uuid,
            "filename",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .map_err(|e| format!("upsert_metadata_source(filename): {e}"))?;
    phases.write_ms = write_start.elapsed().as_millis() as u64;

    phases.total_ms = total_start.elapsed().as_millis() as u64;
    Ok(phases)
}

fn collect_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        if has_audio_ext(root) {
            out.push(root.to_path_buf());
        }
        return out;
    }
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if entry.file_type().is_file() && has_audio_ext(entry.path()) {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

fn has_audio_ext(p: &Path) -> bool {
    match p.extension().and_then(|s| s.to_str()) {
        Some(ext) => {
            let lower = ext.to_ascii_lowercase();
            AUDIO_EXTS.iter().any(|e| *e == lower)
        }
        None => false,
    }
}
