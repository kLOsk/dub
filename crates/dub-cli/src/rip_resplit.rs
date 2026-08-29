//! `dub rip-resplit` — split an already-committed side again, from
//! its lossless archive (M26b).
//!
//! For the mistake the review screen cannot catch: the split looked
//! right, the tracks imported, and only later — listening — does a
//! boundary turn out to be wrong. `side.flac` is the whole side, so
//! fixing it needs no record and no turntable.
//!
//! Replace semantics: the new segments import first, and only then do
//! the tracks from the earlier split come out of the library. A
//! failure part-way leaves the originals in place — the operator ends
//! up with the old split, never with nothing.

use std::io::IsTerminal as _;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use dub_rip::{GapConfig, RipSession, TrackMeta};

pub fn run(args: &[String]) -> Result<()> {
    let opts = parse_args(args)?;
    let mut session = RipSession::resplit_from_archive(opts.session_dir.clone())
        .context("reopening the session archive")?;

    let sample_rate = session.manifest().sample_rate;
    let total_frames = session.manifest().recorded_frames;
    let replacing = session.manifest().replaced_uuids.len();
    println!(
        "re-split: {}  ({}, {} track(s) from the previous split)",
        opts.session_dir.display(),
        mmss(frames_to_secs(total_frames, sample_rate)),
        replacing
    );

    if opts.auto_split {
        let gap_cfg = GapConfig {
            min_track_secs: opts
                .min_track_secs
                .unwrap_or(GapConfig::default().min_track_secs),
            min_gap_secs: opts
                .min_gap_secs
                .unwrap_or(GapConfig::default().min_gap_secs),
            ..GapConfig::default()
        };
        let count = session.auto_split(&gap_cfg).context("auto split")?;
        println!("auto-split: {count} track(s)");
    } else {
        let boundaries: Vec<u64> = opts
            .splits_secs
            .iter()
            .map(|&s| secs_to_frames(s, sample_rate))
            .collect();
        session
            .set_splits(boundaries)
            .context("applying --splits")?;
    }

    let segment_count = session.manifest().tracks.len().max(1);
    // Against the *side*, not the tape: after an auto re-split the
    // detector has trimmed the lead-in and run-out, and printing the
    // plan against `total_frames` disagreed with what commit writes.
    let manifest = session.manifest();
    let ranges = dub_rip::segments(
        &manifest.boundaries_frames,
        manifest.side_start(),
        manifest.side_end(),
    );
    println!("new plan: {} track(s)", ranges.len());
    for (i, range) in ranges.iter().enumerate() {
        println!(
            "  {:>2}. {} – {}  ({})",
            i + 1,
            mmss(frames_to_secs(range.start, sample_rate)),
            mmss(frames_to_secs(range.end, sample_rate)),
            mmss(frames_to_secs(range.end - range.start, sample_rate)),
        );
    }

    for i in 0..segment_count {
        let meta = TrackMeta {
            title: opts.titles.get(i).cloned(),
            artist: opts.artist.clone(),
            album: opts.album.clone(),
            genre: opts.genre.clone(),
            year: opts.year,
        };
        if meta != TrackMeta::default() {
            session.set_track_meta(i, meta)?;
        }
    }

    // This removes library tracks. Confirm when a human is watching.
    if !opts.assume_yes && std::io::stdin().is_terminal() {
        eprintln!(
            "press Enter to import {segment_count} track(s) and remove the {replacing} \
             from the previous split, Ctrl-C to abort"
        );
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    let mut library = match &opts.library {
        Some(path) => dub_library::Library::open_at(path).context("opening library")?,
        None => dub_library::Library::open_default().context("opening default library")?,
    };
    let outcome = session.commit(&mut library).context("commit failed")?;

    for seg in &outcome.segments {
        let name = seg
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match (&seg.library_uuid, &seg.error) {
            (Some(uuid), None) => println!("  {:>2}. {name}  → {uuid}", seg.index + 1),
            (Some(uuid), Some(err)) => {
                println!(
                    "  {:>2}. {name}  → {uuid}  (analysis pending: {err})",
                    seg.index + 1
                );
            }
            (None, err) => println!(
                "  {:>2}. {name}  FAILED: {}",
                seg.index + 1,
                err.as_deref().unwrap_or("unknown")
            ),
        }
    }
    if outcome.is_complete() {
        println!(
            "replaced {} track(s) from the previous split",
            outcome.replaced_removed
        );
    } else {
        println!(
            "commit incomplete — the previous split's tracks were left in place; \
             re-run to retry"
        );
    }
    Ok(())
}

struct Opts {
    session_dir: PathBuf,
    splits_secs: Vec<f64>,
    auto_split: bool,
    min_track_secs: Option<f32>,
    min_gap_secs: Option<f32>,
    titles: Vec<String>,
    artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    year: Option<i32>,
    library: Option<PathBuf>,
    assume_yes: bool,
}

fn parse_args(args: &[String]) -> Result<Opts> {
    let mut out = Opts {
        session_dir: PathBuf::new(),
        splits_secs: Vec::new(),
        auto_split: false,
        min_track_secs: None,
        min_gap_secs: None,
        titles: Vec::new(),
        artist: None,
        album: None,
        genre: None,
        year: None,
        library: None,
        assume_yes: false,
    };
    let mut dir: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |flag: &str| -> Result<String> {
            iter.next()
                .cloned()
                .ok_or_else(|| anyhow!("{flag} expects a value"))
        };
        match arg.as_str() {
            "--splits" => {
                let raw = value("--splits")?;
                let parsed: Result<Vec<f64>, _> =
                    raw.split(',').map(|s| s.trim().parse::<f64>()).collect();
                out.splits_secs = parsed.context("--splits values must be seconds")?;
            }
            "--auto-split" => out.auto_split = true,
            "--min-track" => {
                out.min_track_secs = Some(value("--min-track")?.parse().context("--min-track")?);
            }
            "--min-gap" => {
                out.min_gap_secs = Some(value("--min-gap")?.parse().context("--min-gap")?);
            }
            "--titles" => {
                out.titles = value("--titles")?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
            }
            "--artist" => out.artist = Some(value("--artist")?),
            "--album" => out.album = Some(value("--album")?),
            "--genre" => out.genre = Some(value("--genre")?),
            "--year" => out.year = Some(value("--year")?.parse().context("--year")?),
            "--library" => out.library = Some(PathBuf::from(value("--library")?)),
            "--yes" | "-y" => out.assume_yes = true,
            other if other.starts_with("--") => {
                return Err(anyhow!("unknown rip-resplit flag: {other}"));
            }
            other => dir = Some(PathBuf::from(other)),
        }
    }
    if out.auto_split && !out.splits_secs.is_empty() {
        return Err(anyhow!("--auto-split and --splits are mutually exclusive"));
    }
    out.session_dir = dir.ok_or_else(|| {
        anyhow!(
            "usage: dub rip-resplit <session-dir> [--splits S,S | --auto-split] \
             [--titles ...] [--artist NAME] [--library PATH] [-y]"
        )
    })?;
    Ok(out)
}

fn secs_to_frames(secs: f64, sample_rate: u32) -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (secs * f64::from(sample_rate)).round() as u64;
    frames
}

fn frames_to_secs(frames: u64, sample_rate: u32) -> f64 {
    if sample_rate == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let secs = frames as f64 / f64::from(sample_rate);
    secs
}

fn mmss(secs: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total = secs.max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_resplit_invocation() {
        let args: Vec<String> = [
            "/Music/Dub/Rips/20260822-090000",
            "--splits",
            "182.5,401",
            "--artist",
            "Augustus Pablo",
            "-y",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let opts = parse_args(&args).unwrap();
        assert_eq!(
            opts.session_dir,
            PathBuf::from("/Music/Dub/Rips/20260822-090000")
        );
        assert_eq!(opts.splits_secs, vec![182.5, 401.0]);
        assert_eq!(opts.artist.as_deref(), Some("Augustus Pablo"));
        assert!(opts.assume_yes);
    }

    #[test]
    fn rejects_missing_dir_and_conflicting_modes() {
        assert!(parse_args(&[]).is_err());
        let both: Vec<String> = ["dir", "--auto-split", "--splits", "60"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(parse_args(&both).is_err());
    }
}
