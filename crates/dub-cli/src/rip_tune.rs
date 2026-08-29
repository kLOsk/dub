//! `dub rip-tune` — what the M26b rip gates would do to a recording.
//!
//! The auto-capture and gap-detection thresholds are judgement calls
//! about how vinyl behaves: how far the run-out groove sits under the
//! music, how long an inter-track gap runs, how loud a needle drop is
//! through a phono stage. Guessing at them on the turntable means
//! rebuilding between takes; this reads one real recording instead and
//! reports what each gate decided and by how much.
//!
//! It drives the *real* detector and the *real* silence gate
//! (`dub_rip::detect_gaps` / `simulate_auto_capture`), so the numbers
//! are what would have happened, not a re-implementation that might
//! drift from the engine.
//!
//! Every threshold is overridable, so one recorded side can be swept
//! for a setting that holds — no rig, no rebuild.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use dub_peaks::{Decimator, PeakChunk, DEFAULT_SAMPLES_PER_CHUNK};
use dub_rip::{AutoCapture, GapConfig};

pub fn run(args: &[String]) -> Result<()> {
    let opts = parse_args(args)?;
    let (samples, sample_rate, channels, note) = load(&opts.path)?;
    if sample_rate == 0 || channels == 0 {
        return Err(anyhow!("unusable audio: {} Hz, {channels} ch", sample_rate));
    }
    // A capture usually runs past the record — a fixed --duration
    // leaves minutes of lifted-needle silence that drags every level
    // estimate down. --from / --to analyse just the side.
    let total_frames = samples.len() as u64 / u64::from(channels);
    let start_frame = opts
        .from_secs
        .map_or(0, |s| secs_to_frames(s, sample_rate).min(total_frames));
    let end_frame = opts
        .to_secs
        .map_or(total_frames, |s| {
            secs_to_frames(s, sample_rate).min(total_frames)
        })
        .max(start_frame);
    let ch = u64::from(channels);
    let samples = &samples[usize::try_from(start_frame * ch).unwrap_or(0)
        ..usize::try_from(end_frame * ch).unwrap_or(samples.len())];
    let frames = end_frame - start_frame;

    println!("file      : {}", opts.path.display());
    println!(
        "format    : {sample_rate} Hz, {channels} ch, {} ({frames} frames)",
        clock(frames_to_secs(frames, sample_rate))
    );
    println!("source    : {note}");
    if opts.from_secs.is_some() || opts.to_secs.is_some() {
        println!(
            "window    : {} – {}  (times below are relative to the file start)",
            clock(frames_to_secs(start_frame, sample_rate)),
            clock(frames_to_secs(end_frame, sample_rate))
        );
    }

    let envelope = envelope_of(samples, channels);
    // The detector's own measurement, not a second copy of it.
    let stats = dub_rip::analyze_gaps(&envelope, DEFAULT_SAMPLES_PER_CHUNK, sample_rate, &opts.gap);

    println!();
    println!("LEVELS  (50 ms cells, median chunk RMS)");
    println!(
        "  needle-down region {:>8}   (floor measured here, not over the whole file)",
        clock(f64::from(stats.played_secs))
    );
    println!(
        "  side ends          {:>8}   (music {} – {}; the run-out behind it is discarded)",
        clock(frames_to_secs(stats.music_end_frame, sample_rate)),
        clock(frames_to_secs(stats.music_start_frame, sample_rate)),
        clock(frames_to_secs(stats.music_end_frame, sample_rate)),
    );
    let trimmed = frames.saturating_sub(stats.music_end_frame);
    if trimmed > 0 {
        println!(
            "  run-out discarded  {:>8}",
            clock(frames_to_secs(trimmed, sample_rate))
        );
    }
    println!("  noise floor        {:>8.1} dBFS", stats.floor_db);
    println!("  music level        {:>8.1} dBFS", stats.music_db);
    println!(
        "  contrast           {:>8.1} dB",
        stats.music_db - stats.floor_db
    );
    println!(
        "  gap line           {:>8.1} dBFS   (floor +{:.1})",
        stats.threshold_db, opts.gap.margin_db
    );
    if !stats.usable {
        println!(
            "  ! contrast is under the {:.0} dB minimum — the detector proposes nothing here",
            opts.gap.min_contrast_db
        );
    }

    // ---- Auto-capture --------------------------------------------
    let sim = dub_rip::simulate_auto_capture(samples, sample_rate, &opts.auto);
    println!();
    match opts.auto.start_threshold {
        Some(t) => {
            println!(
                "AUTO-START  (trigger {:.1} dBFS, pre-roll {:.1} s)",
                20.0 * t.max(1e-7).log10(),
                opts.auto.pre_roll_secs
            );
            match sim.start_frame {
                Some(f) => println!(
                    "  fires at           {:>8}   (pre-roll keeps {:.1} s before it)",
                    clock(frames_to_secs(f, sample_rate)),
                    opts.auto.pre_roll_secs
                ),
                None => println!("  never fires — nothing in this side reaches the trigger"),
            }
        }
        None => println!("AUTO-START  disabled"),
    }

    println!();
    match opts.auto.silence_stop_secs {
        Some(secs) => {
            println!(
                "AUTO-STOP   ({secs:.0} s at {:.0} dB under the music)",
                opts.auto.silence_drop_db
            );
            match sim.stop_frame {
                Some(f) => println!(
                    "  fires at           {:>8}",
                    clock(frames_to_secs(f, sample_rate))
                ),
                None => println!("  never fires — the side never goes quiet for long enough"),
            }
            let closest = frames_to_secs(sim.longest_quiet_frames, sample_rate);
            let margin = f64::from(secs) - closest;
            println!(
                "  longest gap the music came back from: {closest:.1} s  ({margin:.1} s of margin)"
            );
            if closest > 0.0 && margin < 3.0 {
                println!(
                    "  !!  only {margin:.1} s of headroom — a record with longer gaps would \
                     stop mid-side"
                );
            }
        }
        None => println!("AUTO-STOP   disabled"),
    }

    // ---- Gaps -----------------------------------------------------
    let gaps = dub_rip::detect_gaps(
        &envelope,
        DEFAULT_SAMPLES_PER_CHUNK,
        sample_rate,
        frames,
        &opts.gap,
    );
    println!();
    println!(
        "GAPS  (min gap {:.1} s, min track {:.0} s, pre-roll {:.1} s)",
        opts.gap.min_gap_secs, opts.gap.min_track_secs, opts.gap.pre_roll_secs
    );
    if gaps.is_empty() {
        println!("  none — the whole side commits as one track");
    }
    for (i, gap) in gaps.iter().enumerate() {
        let start = frames_to_secs(gap.start_frame, sample_rate);
        let end = frames_to_secs(gap.end_frame, sample_rate);
        println!(
            "  {:>2}. {} – {}  ({:.1} s)  → split at {}",
            i + 1,
            clock(start),
            clock(end),
            end - start,
            clock(frames_to_secs(gap.boundary_frame, sample_rate)),
        );
    }

    // What a looser minimum would have kept, so a dropped gap is
    // visible rather than silently absent.
    let permissive = GapConfig {
        min_track_secs: 5.0,
        ..opts.gap
    };
    let all = dub_rip::detect_gaps(
        &envelope,
        DEFAULT_SAMPLES_PER_CHUNK,
        sample_rate,
        frames,
        &permissive,
    );
    if all.len() > gaps.len() {
        println!(
            "  ({} more gap(s) found but dropped by the {:.0} s minimum track length:",
            all.len() - gaps.len(),
            opts.gap.min_track_secs
        );
        for gap in &all {
            if !gaps.iter().any(|k| k.boundary_frame == gap.boundary_frame) {
                println!(
                    "     {} ({:.1} s)",
                    clock(frames_to_secs(gap.start_frame, sample_rate)),
                    frames_to_secs(gap.end_frame - gap.start_frame, sample_rate)
                );
            }
        }
        println!("  )");
    }

    // Where are the most gap-like places, and how high would the line
    // have to sit to catch them? Answers "are there gaps at all" for a
    // side the detector reports nothing on.
    if let Some(want) = opts.quietest {
        let (cells, cell_secs) =
            dub_rip::cell_levels_db(&envelope, DEFAULT_SAMPLES_PER_CHUNK, sample_rate);
        let window = ((opts.gap.min_gap_secs / cell_secs).ceil() as usize).max(1);
        // A gap on a worn inner groove is not uniformly quiet: crackle
        // spikes a handful of its cells. The loudest cell is the line
        // an all-cells-below rule needs; the 90th percentile is the
        // line a rule that tolerates a little debris needs. When the
        // two are far apart, the stretch is a gap the max is hiding.
        let mut candidates: Vec<(usize, f32, f32, f32)> = Vec::new();
        if cells.len() >= window {
            let mut scratch = vec![0.0_f32; window];
            for start in 0..=cells.len() - window {
                scratch.copy_from_slice(&cells[start..start + window]);
                scratch.sort_by(f32::total_cmp);
                let needed = scratch[window - 1];
                let p90 = scratch[(window * 9 / 10).min(window - 1)];
                let median = scratch[window / 2];
                candidates.push((start, needed, p90, median));
            }
        }
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
        println!();
        println!(
            "QUIETEST  {want} most gap-like {:.1} s stretches (non-overlapping)",
            opts.gap.min_gap_secs
        );
        println!("                 all cells    90 % of cells   median cell");
        let mut shown: Vec<usize> = Vec::new();
        for (start, needed, p90, median) in candidates {
            if shown.len() >= want {
                break;
            }
            if shown.iter().any(|&s| start.abs_diff(s) < window * 2) {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let at = start as f64 * f64::from(cell_secs);
            println!(
                "  {:>8}   needs {needed:>7.1}      {p90:>7.1}        {median:>7.1} dBFS",
                clock(at)
            );
            shown.push(start);
        }
    }

    if let Some(step) = opts.profile_secs {
        println!();
        println!("PROFILE  (median cell level per {step:.0} s)");
        let cells_per_step = ((step / 0.05).round() as usize).max(1);
        let mut cell_db: Vec<f32> = envelope
            .chunks(38)
            .map(|cell| {
                let mut v: Vec<f32> = cell.iter().map(|c| c.rms).collect();
                v.sort_by(f32::total_cmp);
                20.0 * v[v.len() / 2].max(1e-7).log10()
            })
            .collect();
        // Keep the tail even when it is a partial step.
        if cell_db.is_empty() {
            cell_db.push(-140.0);
        }
        for (i, group) in cell_db.chunks(cells_per_step).enumerate() {
            let mut sorted = group.to_vec();
            sorted.sort_by(f32::total_cmp);
            let median = sorted[sorted.len() / 2];
            let min = sorted[0];
            let max = sorted[sorted.len() - 1];
            #[allow(clippy::cast_precision_loss)]
            let at = i as f64 * f64::from(step);
            // A coarse bar makes the structure readable at a glance.
            let bar = "#".repeat((((median + 90.0) / 5.0).max(0.0) as usize).min(18));
            println!(
                "  {:>7}  {median:>7.1}  (min {min:>6.1}, max {max:>6.1})  {bar}",
                clock(at)
            );
        }
    }

    let boundaries: Vec<u64> = gaps.iter().map(|g| g.boundary_frame).collect();
    // The last track stops at the last music, not at the end of tape.
    let side_end = if stats.usable && stats.music_end_frame > 0 {
        stats.music_end_frame.min(frames)
    } else {
        frames
    };
    let ranges = dub_rip::segments(&boundaries, side_end);
    println!();
    println!("PLAN  {} track(s)", ranges.len());
    for (i, range) in ranges.iter().enumerate() {
        println!(
            "  {:>2}. {} – {}  ({})",
            i + 1,
            clock(frames_to_secs(range.start, sample_rate)),
            clock(frames_to_secs(range.end, sample_rate)),
            clock(frames_to_secs(range.end - range.start, sample_rate)),
        );
    }
    Ok(())
}

/// Load a side. Prefers the salvage reader for WAVs so an interrupted
/// spill reports its real length instead of what its header claims.
fn load(path: &Path) -> Result<(Vec<f32>, u32, u16, String)> {
    let is_wav = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    if is_wav {
        if let Ok((samples, info)) = dub_rip::read_spill_all(path) {
            let note = if info.was_unfinalized {
                "capture spill, header never finalized (salvaged)".to_string()
            } else {
                "capture WAV".to_string()
            };
            return Ok((samples, info.sample_rate, info.channels, note));
        }
    }
    let track = dub_io::Track::load_from_path(path)
        .map_err(|e| anyhow!("cannot decode {}: {e}", path.display()))?;
    let channels = u16::from(track.channels());
    Ok((
        track.samples().to_vec(),
        track.sample_rate(),
        channels,
        "decoded".to_string(),
    ))
}

/// Mono-downmix envelope, decimated exactly as the capture worker does
/// so chunk indices line up with the detector's expectations.
fn envelope_of(samples: &[f32], channels: u16) -> Vec<PeakChunk> {
    let channels = usize::from(channels.max(1));
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    let mut envelope = Vec::new();
    let mut decimator = Decimator::new(DEFAULT_SAMPLES_PER_CHUNK);
    decimator.feed(&mono, |chunk| envelope.push(chunk));
    envelope
}

fn secs_to_frames(secs: f64, sample_rate: u32) -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (secs.max(0.0) * f64::from(sample_rate)).round() as u64;
    frames
}

fn frames_to_secs(frames: u64, sample_rate: u32) -> f64 {
    if sample_rate == 0 {
        return 0.0;
    }
    frames as f64 / f64::from(sample_rate)
}

/// `m:ss.d` — tight enough to place a split by eye.
fn clock(secs: f64) -> String {
    let total = secs.max(0.0);
    let mins = (total / 60.0).floor();
    let rest = total - mins * 60.0;
    format!("{mins:.0}:{rest:04.1}")
}

struct Opts {
    path: PathBuf,
    profile_secs: Option<f32>,
    quietest: Option<usize>,
    from_secs: Option<f64>,
    to_secs: Option<f64>,
    gap: GapConfig,
    auto: AutoCapture,
}

fn parse_args(args: &[String]) -> Result<Opts> {
    let mut path: Option<PathBuf> = None;
    let mut gap = GapConfig::default();
    let mut auto = AutoCapture::default();
    let mut profile_secs: Option<f32> = None;
    let mut quietest: Option<usize> = None;
    let mut from_secs: Option<f64> = None;
    let mut to_secs: Option<f64> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |flag: &str| -> Result<String> {
            iter.next()
                .cloned()
                .ok_or_else(|| anyhow!("{flag} expects a value"))
        };
        match arg.as_str() {
            "--min-gap" => gap.min_gap_secs = value("--min-gap")?.parse().context("--min-gap")?,
            "--min-track" => {
                gap.min_track_secs = value("--min-track")?.parse().context("--min-track")?;
            }
            "--pre-roll" => {
                gap.pre_roll_secs = value("--pre-roll")?.parse().context("--pre-roll")?;
            }
            "--margin-db" => {
                gap.margin_db = value("--margin-db")?.parse().context("--margin-db")?;
            }
            "--min-contrast-db" => {
                gap.min_contrast_db = value("--min-contrast-db")?
                    .parse()
                    .context("--min-contrast-db")?;
            }
            "--start-threshold-db" => {
                let db: f32 = value("--start-threshold-db")?
                    .parse()
                    .context("--start-threshold-db")?;
                auto.start_threshold = Some(10.0_f32.powf(db / 20.0));
            }
            "--from" => from_secs = Some(value("--from")?.parse().context("--from")?),
            "--to" => to_secs = Some(value("--to")?.parse().context("--to")?),
            "--quietest" => {
                quietest = Some(value("--quietest")?.parse().context("--quietest")?);
            }
            "--profile" => {
                profile_secs = Some(value("--profile")?.parse().context("--profile")?);
            }
            "--no-auto-start" => auto.start_threshold = None,
            "--silence-secs" => {
                auto.silence_stop_secs =
                    Some(value("--silence-secs")?.parse().context("--silence-secs")?);
            }
            "--no-auto-stop" => auto.silence_stop_secs = None,
            "--silence-drop-db" => {
                auto.silence_drop_db = value("--silence-drop-db")?
                    .parse()
                    .context("--silence-drop-db")?;
            }
            other if other.starts_with("--") => {
                return Err(anyhow!("unknown rip-tune flag: {other}"));
            }
            other => path = Some(PathBuf::from(other)),
        }
    }
    Ok(Opts {
        profile_secs,
        quietest,
        from_secs,
        to_secs,
        path: path.ok_or_else(|| {
            anyhow!(
                "usage: dub rip-tune <side.wav> [--min-gap S] [--min-track S] \
                 [--margin-db DB] [--min-contrast-db DB] [--start-threshold-db DB] \
                 [--silence-secs S] [--silence-drop-db DB]"
            )
        })?,
        gap,
        auto,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats_side_positions() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(61.25), "1:01.2");
        assert_eq!(clock(1_361.0), "22:41.0");
    }

    #[test]
    fn parses_overrides() {
        let args: Vec<String> = [
            "side.wav",
            "--min-gap",
            "2.0",
            "--min-track",
            "45",
            "--silence-drop-db",
            "30",
            "--no-auto-start",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let opts = parse_args(&args).unwrap();
        assert_eq!(opts.path, PathBuf::from("side.wav"));
        assert!((opts.gap.min_gap_secs - 2.0).abs() < f32::EPSILON);
        assert!((opts.gap.min_track_secs - 45.0).abs() < f32::EPSILON);
        assert!((opts.auto.silence_drop_db - 30.0).abs() < f32::EPSILON);
        assert!(opts.auto.start_threshold.is_none());
    }

    #[test]
    fn requires_a_file() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--frobnicate".to_string()]).is_err());
    }
}
