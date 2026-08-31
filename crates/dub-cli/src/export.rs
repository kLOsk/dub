//! `dub export` — get your library back out (M11f).
//!
//! PRD §8.6 calls this the load-bearing anti-lock-in commitment:
//! "making leaving easy is the load-bearing behaviour: the DJ trusts a
//! tool that doesn't try to trap them." Two formats, and the difference
//! matters:
//!
//! - **rekordbox XML** carries essentially everything — grid, key,
//!   cues, loops, colours, playlists — and is read by Serato, Traktor,
//!   rekordbox and Lexicon, so it is the one that actually moves a
//!   collection.
//! - **M3U8** carries file paths and nothing else. Universal, and
//!   lossy by construction.
//!
//! Exports every track Dub holds a file for, not just the curated
//! collection — see `Library::list_tracks_for_export` for why.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

use dub_library::m3u::{entries_from_tracks, write_m3u8};
use dub_library::rekordbox_export::{collection_to_parsed, crate_to_parsed, write_xml};
use dub_library::Library;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Rekordbox,
    M3u8,
}

pub fn run(args: &[String]) -> Result<()> {
    let opts = parse_args(args)?;
    let library = match &opts.library {
        Some(path) => Library::open_at(path),
        None => Library::open_default(),
    }
    .context("opening the library")?;

    // Resolve the crate up front so a typo fails before we truncate an
    // output file.
    let selected = match &opts.crate_name {
        Some(name) => {
            let crates = library.list_crates().context("listing crates")?;
            let found = crates
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    let known: Vec<&str> = crates.iter().map(|c| c.name.as_str()).collect();
                    if known.is_empty() {
                        anyhow!("no crate named {name:?}; this library has no crates")
                    } else {
                        anyhow!("no crate named {name:?}; found: {}", known.join(", "))
                    }
                })?;
            Some((found.id, found.name.clone()))
        }
        None => None,
    };

    let file = std::fs::File::create(&opts.out)
        .with_context(|| format!("creating {}", opts.out.display()))?;
    let mut sink = std::io::BufWriter::new(file);

    let count = match opts.format {
        Format::Rekordbox => {
            let parsed = match &selected {
                Some((id, name)) => crate_to_parsed(&library, *id, name),
                None => collection_to_parsed(&library),
            }
            .context("reading the library")?;
            let n = parsed.tracks.len();
            write_xml(&parsed, &mut sink).map_err(|e| anyhow!("{e}"))?;
            n
        }
        Format::M3u8 => {
            let rows = match &selected {
                Some((id, _)) => library.list_crate_tracks(*id),
                None => library.list_tracks_for_export(u32::MAX, 0),
            }
            .context("reading the library")?;
            let entries = entries_from_tracks(&library, &rows).context("resolving paths")?;
            let n = entries.len();
            write_m3u8(&entries, &mut sink)?;
            n
        }
    };
    // Flush before reporting success: a BufWriter dropped on error
    // would report a write that never landed.
    std::io::Write::flush(&mut sink).context("flushing the export")?;

    let what = selected
        .as_ref()
        .map_or_else(|| "the collection".to_string(), |(_, n)| format!("{n:?}"));
    println!(
        "exported {count} track(s) from {what} to {}",
        opts.out.display()
    );
    if opts.format == Format::M3u8 {
        println!("  (M3U8 carries paths only — grid, cues and key need --rekordbox)");
    }
    Ok(())
}

struct Opts {
    format: Format,
    out: PathBuf,
    crate_name: Option<String>,
    library: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Opts> {
    let mut format = None;
    let mut out = None;
    let mut crate_name = None;
    let mut library = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |what: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| anyhow!("{what} expects a value"))
        };
        match arg.as_str() {
            "--rekordbox" => format = Some(Format::Rekordbox),
            "--m3u8" | "--m3u" => format = Some(Format::M3u8),
            "--crate" => crate_name = Some(value("--crate")?),
            "--library" => library = Some(PathBuf::from(value("--library")?)),
            other if other.starts_with("--") => {
                return Err(anyhow!("unknown export flag: {other}"))
            }
            other => out = Some(PathBuf::from(other)),
        }
    }
    Ok(Opts {
        format: format.ok_or_else(usage)?,
        out: out.ok_or_else(usage)?,
        crate_name,
        library,
    })
}

fn usage() -> anyhow::Error {
    anyhow!(
        "usage: dub export --rekordbox <out.xml> | --m3u8 <out.m3u8>\n  \
         [--crate NAME]    export one crate instead of the whole library\n  \
         [--library PATH]  a library other than the default\n\n  \
         --rekordbox carries grid, key, cues, loops, colours and playlists,\n  \
         and is read by Serato, Traktor, rekordbox and Lexicon.\n  \
         --m3u8 carries file paths only."
    )
}
