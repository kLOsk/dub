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
    let frames = samples.len() as u64 / u64::from(channels);

    println!("file      : {}", opts.path.display());
    println!(
        "format    : {sample_rate} Hz, {channels} ch, {} ({frames} frames)",
        clock(frames_to_secs(frames, sample_rate))
    );
    println!("source    : {note}");

    let envelope = envelope_of(&samples, channels);
    let stats = level_stats(&envelope);

    println!();
    println!("LEVELS  (50 ms cells, median chunk RMS)");
    println!("  noise floor        {:>8.1} dBFS", stats.floor_db);
    println!("  music level        {:>8.1} dBFS", stats.music_db);
    println!("  contrast           {:>8.1} dB", stats.contrast());
    let gap_line = (stats.floor_db + opts.gap.margin_db).min(stats.music_db - opts.gap.min_drop_db);
    println!(
        "  gap line           {:>8.1} dBFS   (floor +{:.0}, capped at music −{:.0})",
        gap_line, opts.gap.margin_db, opts.gap.min_drop_db
    );
    if stats.contrast() < opts.gap.min_contrast_db {
        println!(
            "  ! contrast is under the {:.0} dB minimum — the detector proposes nothing here",
            opts.gap.min_contrast_db
        );
    }

    // ---- Auto-capture --------------------------------------------
    let sim = dub_rip::simulate_auto_capture(&samples, sample_rate, &opts.auto);
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

    let boundaries: Vec<u64> = gaps.iter().map(|g| g.boundary_frame).collect();
    let ranges = dub_rip::segments(&boundaries, frames);
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

struct Levels {
    floor_db: f32,
    music_db: f32,
}

impl Levels {
    fn contrast(&self) -> f32 {
        self.music_db - self.floor_db
    }
}

/// Mirrors the detector's own estimator: 50 ms cells at the median
/// chunk level, floor from the 20th-quietest cell, music at p80.
fn level_stats(envelope: &[PeakChunk]) -> Levels {
    let per_cell = 38; // ≈50 ms at 64-frame chunks / 48 kHz
    let mut cells: Vec<f32> = envelope
        .chunks(per_cell)
        .map(|cell| {
            let mut v: Vec<f32> = cell.iter().map(|c| c.rms).collect();
            v.sort_by(f32::total_cmp);
            20.0 * v[v.len() / 2].max(1e-7).log10()
        })
        .collect();
    if cells.is_empty() {
        return Levels {
            floor_db: -140.0,
            music_db: -140.0,
        };
    }
    cells.sort_by(f32::total_cmp);
    let last = cells.len() - 1;
    Levels {
        floor_db: cells[20.min(last)],
        music_db: cells[((last as f32) * 0.8).round() as usize],
    }
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
    gap: GapConfig,
    auto: AutoCapture,
}

fn parse_args(args: &[String]) -> Result<Opts> {
    let mut path: Option<PathBuf> = None;
    let mut gap = GapConfig::default();
    let mut auto = AutoCapture::default();
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
            "--min-drop-db" => {
                gap.min_drop_db = value("--min-drop-db")?.parse().context("--min-drop-db")?;
            }
            "--start-threshold-db" => {
                let db: f32 = value("--start-threshold-db")?
                    .parse()
                    .context("--start-threshold-db")?;
                auto.start_threshold = Some(10.0_f32.powf(db / 20.0));
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
        path: path.ok_or_else(|| {
            anyhow!(
                "usage: dub rip-tune <side.wav> [--min-gap S] [--min-track S] \
                 [--margin-db DB] [--min-drop-db DB] [--start-threshold-db DB] \
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
