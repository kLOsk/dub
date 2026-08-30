//! `dub recognize` — name the tracks on a rip (M26c).
//!
//! Two input modes, because recognition has to be testable away from the
//! turntable for the same reason the gap gates do:
//!
//! - **a committed session directory** — fingerprints each committed
//!   segment and can write the result back with `--apply`.
//! - **a bare capture file** — decodes it, runs the shipped M26b
//!   auto-split over it, and recognises the segments that fall out. This
//!   is how a recorded side in `testdata/rip-baselines/` gets driven
//!   through the whole pipeline without a rig, exactly as `rip-tune`
//!   replays the gap gates. Read-only: there is no manifest to write to.
//!
//! Read-only by default in both modes: it prints what it found and
//! changes nothing, because a wrong match written into the library is
//! worse than no match at all.
//!
//! The AcoustID key comes from `--key` or `$DUB_ACOUSTID_KEY`. It is
//! deliberately *not* read from a file in the repo or the session dir —
//! a credential that lives next to the code gets committed eventually.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use dub_peaks::DEFAULT_SAMPLES_PER_CHUNK;
use dub_recognize::http::UreqHttp;
use dub_recognize::{Recognizer, SegmentAudio, SideRecognition};
use dub_rip::GapConfig;

use crate::rip_tune::{clock, envelope_of, frames_to_secs, load};

/// Env var holding the AcoustID application key.
const KEY_ENV: &str = "DUB_ACOUSTID_KEY";

/// One segment's audio, plus how to name it in the report.
struct Loaded {
    /// Index into the manifest's track list. In capture mode there is no
    /// manifest, and this is just the segment's position on the side.
    index: usize,
    /// What to print for this segment — its committed title, or its time
    /// range when the split came from the detector.
    label: String,
    pcm: Vec<i16>,
    sample_rate: u32,
    channels: u16,
}

pub fn run(args: &[String]) -> Result<()> {
    let opts = parse_args(args)?;

    let key = match opts.key {
        Some(k) => k,
        None => std::env::var(KEY_ENV).map_err(|_| {
            anyhow!(
                "no AcoustID key.\n  \
                 Register a free one at https://acoustid.org/new-application, then:\n    \
                 export {KEY_ENV}=your-key\n  \
                 or pass --key <key>. Do not put it in a file in the repo."
            )
        })?,
    };

    // A directory holding a rip.json is a session; anything else is
    // treated as a capture to split and recognise.
    let is_session = opts.path.join("rip.json").is_file();
    if opts.apply && !is_session {
        return Err(anyhow!(
            "--apply needs a rip session to write to; {} is a capture file",
            opts.path.display()
        ));
    }

    let loaded = if is_session {
        load_session(&opts.path)?
    } else {
        load_capture(&opts.path)?
    };
    if loaded.is_empty() {
        return Err(anyhow!("no audio to recognise"));
    }

    let segments: Vec<SegmentAudio<'_>> = loaded
        .iter()
        .map(|l| SegmentAudio {
            samples: &l.pcm,
            sample_rate: l.sample_rate,
            channels: l.channels,
        })
        .collect();

    println!(
        "recognising {} segment(s) from {}",
        segments.len(),
        opts.path.display()
    );
    println!("  (MusicBrainz allows one request a second, so this takes a moment)");

    // One second between requests is MusicBrainz's published limit and
    // the tightest of the three services, so it governs.
    let http = UreqHttp::new(std::time::Duration::from_secs(1));
    let result = Recognizer::new(&http, key)
        .with_release_lookup(opts.album)
        .recognize_side(&segments)?;
    report(&result, &loaded);

    if opts.apply {
        apply(&opts.path, &result, &loaded)?;
    } else if result.named > 0 && is_session {
        println!("\nnothing written — re-run with --apply to save this into rip.json");
    }
    println!("\n{} request(s) made", http.request_count());
    Ok(())
}

/// Load the committed tracks of a rip session.
fn load_session(dir: &Path) -> Result<Vec<Loaded>> {
    let manifest = dub_rip::load_manifest(dir)
        .with_context(|| format!("reading {}", dir.join("rip.json").display()))?;
    if manifest.tracks.is_empty() {
        return Err(anyhow!("that session has no committed tracks"));
    }

    // A segment with no encoded file was never committed; it is reported
    // rather than skipped silently, since a partial side changes what
    // the vote can see.
    let mut out = Vec::new();
    for (i, entry) in manifest.tracks.iter().enumerate() {
        let Some(rel) = entry.encoded_file.as_ref() else {
            eprintln!("  track {} has no encoded file — skipping", i + 1);
            continue;
        };
        let path = dir.join(rel);
        let track = dub_io::Track::load_from_path(&path)
            .map_err(|e| anyhow!("decoding {}: {e}", path.display()))?;
        out.push(Loaded {
            index: i,
            label: entry
                .meta
                .title
                .clone()
                .unwrap_or_else(|| "(untitled)".to_string()),
            pcm: to_i16(track.samples()),
            sample_rate: track.sample_rate(),
            channels: u16::from(track.channels()),
        });
    }
    Ok(out)
}

/// Decode a capture and split it with the shipped auto-split gates.
///
/// This is the offline path: whatever the detector would have produced
/// on the rig is what gets recognised, so a bad split shows up here as a
/// bad match rather than hiding until someone rips a record.
fn load_capture(path: &Path) -> Result<Vec<Loaded>> {
    if !path.is_file() {
        return Err(anyhow!(
            "{} is neither a rip session (no rip.json) nor a file",
            path.display()
        ));
    }
    let (samples, sample_rate, channels, note) = load(path)?;
    let frames = samples.len() as u64 / u64::from(channels.max(1));
    let envelope = envelope_of(&samples, channels);
    let cfg = GapConfig::default();

    let stats = dub_rip::analyze_gaps(&envelope, DEFAULT_SAMPLES_PER_CHUNK, sample_rate, &cfg);
    let gaps = dub_rip::detect_gaps(
        &envelope,
        DEFAULT_SAMPLES_PER_CHUNK,
        sample_rate,
        frames,
        &cfg,
    );
    let boundaries: Vec<u64> = gaps.iter().map(|g| g.boundary_frame).collect();
    // Same both-end trim the committed path applies: a side that the
    // detector refused keeps its whole capture.
    let (side_start, side_end) = if stats.usable && stats.music_end_frame > 0 {
        (
            stats.side_start_frame.min(frames),
            stats.music_end_frame.min(frames),
        )
    } else {
        (0, frames)
    };
    let ranges = dub_rip::segments(&boundaries, side_start, side_end);

    println!(
        "{} — {} at {} Hz, {} track(s) from auto-split{}",
        path.display(),
        clock(frames_to_secs(frames, sample_rate)),
        sample_rate,
        ranges.len(),
        if stats.usable {
            String::new()
        } else {
            " (side refused — recognising it whole)".to_string()
        }
    );

    if !note.is_empty() {
        println!("  source: {note}");
    }

    let ch = usize::from(channels.max(1));
    Ok(ranges
        .iter()
        .enumerate()
        .map(|(i, range)| {
            let (from, to) = (range.start as usize * ch, range.end as usize * ch);
            Loaded {
                index: i,
                label: format!(
                    "{} – {}",
                    clock(frames_to_secs(range.start, sample_rate)),
                    clock(frames_to_secs(range.end, sample_rate))
                ),
                pcm: to_i16(&samples[from.min(samples.len())..to.min(samples.len())]),
                sample_rate,
                channels,
            }
        })
        .collect())
}

/// Chromaprint takes i16; captures and rips are 24-bit in f32.
fn to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
        .collect()
}

fn report(result: &SideRecognition, loaded: &[Loaded]) {
    println!();
    println!(
        "NAMED     {} of {} track(s)",
        result.named,
        result.segments.len()
    );
    match &result.release {
        Some(rel) => {
            println!("RELEASE");
            println!("  title    {}", rel.title);
            if let Some(a) = &rel.artist {
                println!("  artist   {a}");
            }
            for (label, value) in [
                ("date", rel.date.as_deref()),
                ("label", rel.label.as_deref()),
                ("cat no", rel.catalog_number.as_deref()),
            ] {
                if let Some(v) = value {
                    println!("  {label:<8} {v}");
                }
            }
            println!("  mbid     {}", rel.mbid);
            if let Some(side) = result.side {
                println!("  side     {side}");
            }
            println!(
                "  matched  {} of {} segment(s)",
                result.explained,
                result.segments.len()
            );
        }
        None if result.segments.is_empty() => {}
        None => println!("RELEASE   not looked up (pass --album to identify the pressing)"),
    }
    if result.unresolved > 0 {
        println!(
            "  note     {} recording(s) MusicBrainz would not answer for — \
             this side was voted on incomplete data",
            result.unresolved
        );
    }

    println!();
    println!("TRACKS");
    for (seg, l) in result.segments.iter().zip(loaded) {
        match &seg.named {
            Some(t) => {
                let num = t
                    .number
                    .as_deref()
                    .map_or_else(String::new, |n| format!("{n}  "));
                let artist = t.artist.as_deref().unwrap_or("(unknown artist)");
                println!(
                    "  {:>2}. {:<24} → {num}{artist} — {}",
                    l.index + 1,
                    l.label,
                    t.title
                );
            }
            None => {
                let why = seg.note.as_deref().unwrap_or("no match");
                println!("  {:>2}. {:<28} → — ({why})", l.index + 1, l.label);
                // Show what was on offer, so an operator can tell "not in
                // the database" from "matched something on another release".
                for c in seg.candidates.iter().take(2) {
                    let name = c.title.as_deref().unwrap_or(&c.recording_mbid);
                    println!("      considered {name} ({:.2})", c.score);
                }
            }
        }
    }
}

fn apply(session_dir: &Path, result: &SideRecognition, loaded: &[Loaded]) -> Result<()> {
    if result.named == 0 {
        println!("\nnothing to apply — nothing was identified");
        return Ok(());
    }
    let mut manifest = dub_rip::load_manifest(session_dir)?;
    let mut written = 0;
    for (seg, l) in result.segments.iter().zip(loaded) {
        let Some(named) = &seg.named else { continue };
        let Some(entry) = manifest.tracks.get_mut(l.index) else {
            continue;
        };
        // Artist and title are the deliverable and are written whenever
        // they were found. Album only exists if a release was asked for.
        entry.meta.title = Some(named.title.clone());
        if let Some(a) = &named.artist {
            entry.meta.artist = Some(a.clone());
        }
        if let Some(rel) = &result.release {
            entry.meta.album = Some(rel.title.clone());
            // MusicBrainz dates are often just a year; take the leading
            // four digits and only when they parse.
            if let Some(year) = rel
                .date
                .as_deref()
                .and_then(|d| d.get(..4))
                .and_then(|y| y.parse::<i32>().ok())
            {
                entry.meta.year = Some(year);
            }
        }
        written += 1;
    }
    dub_rip::save_manifest(session_dir, &manifest)?;
    println!("\napplied to {written} track(s) in rip.json");
    println!(
        "  re-run `dub rip-resplit {} --auto-split` to re-encode with the new tags,",
        session_dir.display()
    );
    println!("  or edit further in the app's review panel.");
    Ok(())
}

struct Opts {
    path: PathBuf,
    key: Option<String>,
    apply: bool,
    album: bool,
}

fn parse_args(args: &[String]) -> Result<Opts> {
    let mut path: Option<PathBuf> = None;
    let mut key = None;
    let mut apply = false;
    let mut album = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--key" => {
                key = Some(
                    iter.next()
                        .cloned()
                        .ok_or_else(|| anyhow!("--key expects a value"))?,
                );
            }
            "--apply" => apply = true,
            "--album" => album = true,
            other if other.starts_with("--") => {
                return Err(anyhow!("unknown recognize flag: {other}"));
            }
            other => path = Some(PathBuf::from(other)),
        }
    }
    Ok(Opts {
        path: path.ok_or_else(|| {
            anyhow!(
                "usage: dub recognize <session-dir | side.wav> [--key KEY] [--album] [--apply]\n  \
                 names each track's artist + title from AcoustID — no MusicBrainz, no throttle.\n  \
                 --album  also identify the pressing (album / label / cat no / A1-B3 numbers),\n           \
                 at one MusicBrainz request a second.\n  \
                 --apply  write the result into a session's rip.json.\n  \
                 a session dir recognises its committed tracks; any decodable audio file is\n  \
                 auto-split first and is read-only.\n  \
                 the key may also come from ${KEY_ENV}"
            )
        })?,
        key,
        apply,
        album,
    })
}
