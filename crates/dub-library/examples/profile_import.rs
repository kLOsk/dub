//! Profile filesystem-import wall-clock against the real importer.
//!
//! Walks a folder, runs `dub_library::import_folder` against a
//! tempfile library, and reports throughput + a per-file breakdown
//! of just the user-visible work the M11c.4 lazy-fingerprint
//! importer actually does (stat, metadata-only probe, SQL writes).
//!
//! Compared to the pre-M11c.4 importer this is fast: there is no
//! decode and no Chromaprint, so per-file cost should fall from
//! ~hundreds of ms (decode-bound) to ~tens of ms (metadata-probe
//! and SQL write bound). The remaining decode + fingerprint cost
//! is paid lazily by `Library::analyze_track` on first deck-load.
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

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use dub_library::{import_folder, Library};

const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "aif", "aiff", "m4a", "alac", "aac"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: profile_import <folder>");
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

    let n_audio = count_audio_files(&root);
    if n_audio == 0 {
        eprintln!("no audio files found under {}", root.display());
        return ExitCode::from(2);
    }
    println!(
        "profiling import of {n_audio} audio file(s) under {}",
        root.display()
    );

    let start = Instant::now();
    let summary = match import_folder(&mut library, &root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("import_folder failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let wall_ms = start.elapsed().as_millis() as u64;

    println!();
    println!("import summary:");
    println!("  added     : {}", summary.added);
    println!("  refreshed : {}", summary.refreshed);
    println!("  skipped   : {}", summary.skipped);
    if !summary.errors.is_empty() {
        println!("  errors    : {}", summary.errors.len());
        for e in summary.errors.iter().take(5) {
            println!("    {} - {}", e.path.display(), e.reason);
        }
        if summary.errors.len() > 5 {
            println!("    ... ({} more)", summary.errors.len() - 5);
        }
    }
    println!();
    println!("wall clock: {wall_ms} ms");
    let throughput = n_audio as f64 / (wall_ms.max(1) as f64) * 1000.0;
    println!("throughput: {throughput:.2} file/s ({n_audio} audio file(s) in {wall_ms} ms)");

    if !summary.errors.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn has_audio_ext(p: &std::path::Path) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(ext) => {
            let lo = ext.to_ascii_lowercase();
            AUDIO_EXTS.iter().any(|e| *e == lo)
        }
        None => false,
    }
}

fn count_audio_files(root: &std::path::Path) -> usize {
    if root.is_file() {
        return if has_audio_ext(root) { 1 } else { 0 };
    }
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok);
    walker
        .filter(|e| e.file_type().is_file() && has_audio_ext(e.path()))
        .count()
}
