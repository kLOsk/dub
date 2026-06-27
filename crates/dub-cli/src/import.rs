//! `dub import` — headless library import (M12b).
//!
//! Drives the same adapters the Apple shell calls over FFI, against the
//! default library DB (`~/Library/Application Support/Dub/library.sqlite`).
//! Useful for validating an importer end-to-end against a real export
//! without standing up the UI.
//!
//! ```text
//! dub import --traktor   <collection.nml>   # Traktor NML (M12b)
//! dub import --serato    <_Serato_ dir>      # Serato library (M11e)
//! dub import --itunes    <Library.xml>       # iTunes / Apple Music (M12c)
//! dub import --rekordbox <rekordbox.xml>     # rekordbox XML export (M12d)
//! dub import --folder    <music-dir>         # recursive folder walk (M11c)
//! ```

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

use dub_library::{ImportSummary, Library};

pub fn run(args: &[String]) -> Result<()> {
    let mut traktor: Option<PathBuf> = None;
    let mut serato: Option<PathBuf> = None;
    let mut itunes: Option<PathBuf> = None;
    let mut rekordbox: Option<PathBuf> = None;
    let mut folder: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--traktor" => {
                traktor = Some(PathBuf::from(next_value(args, i, "--traktor")?));
                i += 2;
            }
            "--serato" => {
                serato = Some(PathBuf::from(next_value(args, i, "--serato")?));
                i += 2;
            }
            "--itunes" => {
                itunes = Some(PathBuf::from(next_value(args, i, "--itunes")?));
                i += 2;
            }
            "--rekordbox" => {
                rekordbox = Some(PathBuf::from(next_value(args, i, "--rekordbox")?));
                i += 2;
            }
            "--folder" => {
                folder = Some(PathBuf::from(next_value(args, i, "--folder")?));
                i += 2;
            }
            other => return Err(anyhow!("import: unexpected argument '{other}'\n{USAGE}")),
        }
    }

    // Exactly one source required.
    let chosen = [
        traktor.is_some(),
        serato.is_some(),
        itunes.is_some(),
        rekordbox.is_some(),
        folder.is_some(),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if chosen != 1 {
        return Err(anyhow!("{USAGE}"));
    }

    let mut library = Library::open_default().context("opening default library")?;
    println!(
        "library: {}",
        dub_library::default_library_db_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<unknown>".to_string())
    );

    let summary = if let Some(nml) = traktor {
        println!("importing Traktor NML: {}", nml.display());
        dub_library::import_traktor(&mut library, &nml).context("traktor import")?
    } else if let Some(dir) = serato {
        println!("importing Serato library: {}", dir.display());
        dub_library::import_serato(&mut library, &dir).context("serato import")?
    } else if let Some(xml) = itunes {
        println!("importing iTunes library: {}", xml.display());
        dub_library::import_itunes(&mut library, &xml).context("itunes import")?
    } else if let Some(xml) = rekordbox {
        println!("importing rekordbox XML: {}", xml.display());
        dub_library::import_rekordbox(&mut library, &xml).context("rekordbox import")?
    } else {
        let dir = folder.expect("folder is Some when the others are None");
        println!("importing folder: {}", dir.display());
        dub_library::import_folder(&mut library, &dir).context("folder import")?
    };

    print_summary(&summary);
    Ok(())
}

const USAGE: &str = "usage: dub import (--traktor <nml> | --serato <dir> | \
    --itunes <xml> | --rekordbox <xml> | --folder <dir>)";

fn print_summary(s: &ImportSummary) {
    println!("  added:     {}", s.added);
    println!("  refreshed: {}", s.refreshed);
    println!("  skipped:   {}", s.skipped);
    if !s.errors.is_empty() {
        println!("  errors ({}):", s.errors.len());
        for e in s.errors.iter().take(20) {
            println!("    {} — {}", e.path.display(), e.reason);
        }
        if s.errors.len() > 20 {
            println!("    … and {} more", s.errors.len() - 20);
        }
    }
    println!("OK");
}

fn next_value<'a>(args: &'a [String], i: usize, flag: &str) -> Result<&'a str> {
    args.get(i + 1)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("{flag} expects a path"))
}
