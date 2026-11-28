//! `dub draw-check` — what the waveform strips drew, frame by frame.
//!
//! A Debug build of Dub started with `DUB_DRAW_LOG=<file>` writes one line
//! per drawn frame per deck (`apple/Dub/Waveform/DrawLog.swift`). This
//! reads it back and turns the rig's "by eye" waveform items into numbers:
//!
//! - **a jump**: the playhead moving between two frames by more than the
//!   deck's rate over the time between them explains (rig 3: "waveforms
//!   never jump left/right while a deck plays steadily");
//! - **a zoom change**: the seconds a pixel shows changing while the
//!   fader's pitch did not (rig 2: "the waveform keeps its zoom through
//!   stop and start", "scratch hard → the waveform keeps its zoom");
//! - **the beat lines**: how far the next line sits from the playhead, in
//!   pixels — it must slide smoothly, never step.
//!
//! The rate in the log is the deck's smoothed rate, which under timecode
//! lags the platter by a few milliseconds. Steady play explains every
//! step to well under a millisecond; a lift, a stop or a scratch shows up
//! as steps the rate explains less well, so the report gives each
//! stretch's worst step with its time, rather than a single verdict.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

const USAGE: &str =
    "usage: dub draw-check <draw.log> [--deck N] [--from SECS] [--to SECS] [--jump-ms MS]";

/// A step the rate leaves unexplained by more than this is a jump: a
/// millisecond of audio is a third of a pixel at the default zoom, so
/// two are the least an eye could catch.
const DEFAULT_JUMP_MS: f64 = 2.0;

/// The zoom may move this much, relative, before it counts as a change:
/// 0.1 % is under a pixel at the far end of a 660-column strip. A steady
/// carrier through the loop moves it ±0.02 %.
const ZOOM_TOLERANCE: f64 = 1e-3;

/// Frames further apart than this many median intervals are a pause in
/// drawing — a still deck needs no redraw — not a step to measure.
const GAP_FRAMES: f64 = 3.0;

/// One drawn frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    /// The vsync the frame was drawn for, seconds on the engine's clock.
    t: f64,
    deck: u32,
    has_track: bool,
    /// Track seconds under the playhead.
    playhead: f64,
    /// Track seconds per wall second, as the deck reported it.
    rate: f64,
    /// Track seconds one pixel of the time axis shows.
    secs_per_px: f64,
    /// Peak chunks folded into one drawn column.
    agg: u32,
    /// Pixels from the playhead to the next beat line (`NaN` without a
    /// grid).
    next_beat_px: f64,
}

fn parse_line(line: &str) -> Option<Frame> {
    if line.starts_with('#') || line.trim().is_empty() {
        return None;
    }
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 9 {
        return None;
    }
    let num = |i: usize| f[i].trim().parse::<f64>().ok();
    let t_ns = f[0].trim().parse::<u64>().ok()?;
    #[allow(clippy::cast_precision_loss)]
    let t = t_ns as f64 / 1e9;
    Some(Frame {
        t,
        deck: f[1].trim().parse().ok()?,
        has_track: f[2].trim() == "1",
        playhead: num(3)?,
        rate: num(4)?,
        secs_per_px: num(5)?,
        agg: f[6].trim().parse().ok()?,
        next_beat_px: num(7).unwrap_or(f64::NAN),
    })
}

/// What the strip did between two frames.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Step {
    /// When, seconds since the first frame of the selection.
    at: f64,
    /// Track milliseconds the playhead moved beyond what the rate explains.
    unexplained_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Report {
    frames: usize,
    /// Median frame interval, ms.
    frame_ms: f64,
    worst: Option<Step>,
    jumps: Vec<Step>,
    /// Frames where the zoom changed, with the relative change.
    zoom_changes: Vec<(f64, f64)>,
    /// Frames where the column aggregation changed.
    agg_changes: Vec<f64>,
    /// Largest move of the next beat line between frames, beyond what the
    /// playhead's own move accounts for, in pixels.
    beat_wobble_px: f64,
    /// Times the strip stopped drawing (a still deck), and the longest.
    pauses: usize,
    longest_pause_s: f64,
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Measure `frames` (one deck, in time order) against `jump_ms`.
fn check(frames: &[Frame], jump_ms: f64) -> Report {
    let loaded: Vec<&Frame> = frames.iter().filter(|f| f.has_track).collect();
    let mut r = Report {
        frames: loaded.len(),
        ..Report::default()
    };
    let Some(first) = loaded.first() else {
        return r;
    };
    let t0 = first.t;
    r.frame_ms = median(loaded.windows(2).map(|w| (w[1].t - w[0].t) * 1e3).collect());
    let gap = r.frame_ms * GAP_FRAMES / 1e3;
    for w in loaded.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dt = b.t - a.t;
        if dt <= 0.0 {
            continue;
        }
        if dt > gap {
            r.pauses += 1;
            r.longest_pause_s = r.longest_pause_s.max(dt);
            continue;
        }
        // Between two frames the deck ran at some rate between the two it
        // reported: a hand stopping or starting the platter inside a frame
        // moves the playhead anywhere in that band. Only a move outside it
        // is a step the eye sees.
        let moved = b.playhead - a.playhead;
        let slowest = a.rate.min(b.rate) * dt;
        let fastest = a.rate.max(b.rate) * dt;
        let outside = if moved > fastest {
            moved - fastest
        } else if moved < slowest {
            moved - slowest
        } else {
            0.0
        };
        let step = Step {
            at: b.t - t0,
            unexplained_ms: outside * 1e3,
        };
        if r.worst
            .is_none_or(|s| step.unexplained_ms.abs() > s.unexplained_ms.abs())
        {
            r.worst = Some(step);
        }
        if step.unexplained_ms.abs() > jump_ms {
            r.jumps.push(step);
        }
        if a.secs_per_px > 0.0 {
            let rel = b.secs_per_px / a.secs_per_px - 1.0;
            if rel.abs() > ZOOM_TOLERANCE {
                r.zoom_changes.push((b.t - t0, rel));
            }
        }
        if a.agg != b.agg {
            r.agg_changes.push(b.t - t0);
        }
        // The same line, slid by the playhead's move in pixels. Once the
        // playhead passes it (either way) the next line is another line,
        // a whole beat away — not a wobble.
        if a.next_beat_px.is_finite() && b.next_beat_px.is_finite() && b.secs_per_px > 0.0 {
            let slide = moved / b.secs_per_px;
            let expected = a.next_beat_px - slide;
            if moved >= 0.0 && expected > 0.5 {
                r.beat_wobble_px = r.beat_wobble_px.max((b.next_beat_px - expected).abs());
            }
        }
    }
    r
}

#[derive(Debug, Clone, PartialEq)]
struct Args {
    log: PathBuf,
    deck: Option<u32>,
    from: Option<f64>,
    to: Option<f64>,
    jump_ms: f64,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let mut log = None;
    let mut deck = None;
    let mut from = None;
    let mut to = None;
    let mut jump_ms = DEFAULT_JUMP_MS;
    let mut i = 0;
    while i < args.len() {
        let value = || {
            args.get(i + 1)
                .ok_or_else(|| anyhow!("{} needs a value\n{USAGE}", args[i]))
        };
        match args[i].as_str() {
            "--deck" => {
                deck = Some(value()?.parse().context("--deck expects 0 or 1")?);
                i += 2;
            }
            "--from" => {
                from = Some(value()?.parse().context("--from expects seconds")?);
                i += 2;
            }
            "--to" => {
                to = Some(value()?.parse().context("--to expects seconds")?);
                i += 2;
            }
            "--jump-ms" => {
                jump_ms = value()?.parse().context("--jump-ms expects milliseconds")?;
                i += 2;
            }
            s if s.starts_with('-') => bail!("unknown flag {s}\n{USAGE}"),
            s => {
                if log.replace(PathBuf::from(s)).is_some() {
                    bail!("one log file\n{USAGE}");
                }
                i += 1;
            }
        }
    }
    Ok(Args {
        log: log.ok_or_else(|| anyhow!("{USAGE}"))?,
        deck,
        from,
        to,
        jump_ms,
    })
}

fn read_frames(path: &Path) -> Result<Vec<Frame>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut frames: Vec<Frame> = text.lines().filter_map(parse_line).collect();
    frames.sort_by(|a, b| a.t.total_cmp(&b.t));
    Ok(frames)
}

/// # Errors
/// Bad arguments, or the log cannot be read.
pub fn run(args: &[String]) -> Result<()> {
    let args = parse_args(args)?;
    let frames = read_frames(&args.log)?;
    let Some(start) = frames.first().map(|f| f.t) else {
        bail!(
            "{}: no frames (was Dub started with DUB_DRAW_LOG?)",
            args.log.display()
        );
    };
    let decks: Vec<u32> = match args.deck {
        Some(d) => vec![d],
        None => {
            let mut d: Vec<u32> = frames.iter().map(|f| f.deck).collect();
            d.sort_unstable();
            d.dedup();
            d
        }
    };
    for deck in decks {
        let chosen: Vec<Frame> = frames
            .iter()
            .filter(|f| f.deck == deck)
            .filter(|f| args.from.is_none_or(|s| f.t - start >= s))
            .filter(|f| args.to.is_none_or(|s| f.t - start <= s))
            .copied()
            .collect();
        let r = check(&chosen, args.jump_ms);
        let offset = chosen.first().map_or(0.0, |f| f.t - start);
        println!(
            "deck {}: {} frames with a track, a frame every {:.1} ms",
            if deck == 0 { "A" } else { "B" },
            r.frames,
            r.frame_ms
        );
        if let Some(w) = r.worst {
            println!(
                "  worst step: {:+.2} ms unexplained at {:.2} s",
                w.unexplained_ms,
                w.at + offset
            );
        }
        println!(
            "  steps over {:.1} ms: {}{}",
            args.jump_ms,
            r.jumps.len(),
            list_times(r.jumps.iter().map(|s| s.at + offset))
        );
        println!(
            "  zoom changes: {}{}",
            r.zoom_changes.len(),
            list_times(r.zoom_changes.iter().map(|z| z.0 + offset))
        );
        println!(
            "  column aggregation changes: {}{}",
            r.agg_changes.len(),
            list_times(r.agg_changes.iter().map(|t| t + offset))
        );
        if r.beat_wobble_px > 0.0 {
            println!("  beat line wobble: {:.2} px at most", r.beat_wobble_px);
        }
        if r.pauses > 0 {
            println!(
                "  drawing paused {} times (a still deck), longest {:.2} s",
                r.pauses, r.longest_pause_s
            );
        }
    }
    Ok(())
}

/// The first few times, for a line of the report.
fn list_times(times: impl Iterator<Item = f64>) -> String {
    let all: Vec<f64> = times.collect();
    if all.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = all.iter().take(8).map(|t| format!("{t:.2}")).collect();
    let more = if all.len() > 8 {
        format!(" … (+{})", all.len() - 8)
    } else {
        String::new()
    };
    format!(" at {} s{more}", shown.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t: f64, playhead: f64, rate: f64) -> Frame {
        Frame {
            t,
            deck: 0,
            has_track: true,
            playhead,
            rate,
            secs_per_px: 0.0029,
            agg: 2,
            next_beat_px: f64::NAN,
        }
    }

    /// Sixty frames a second of steady play, the playhead exactly where
    /// the rate puts it.
    fn steady(n: usize, rate: f64) -> Vec<Frame> {
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f64 / 60.0;
                frame(t, 10.0 + t * rate, rate)
            })
            .collect()
    }

    #[test]
    fn steady_play_has_no_jump() {
        let r = check(&steady(600, 1.0), DEFAULT_JUMP_MS);
        assert_eq!(r.frames, 600);
        assert!((r.frame_ms - 16.67).abs() < 0.1, "{}", r.frame_ms);
        assert!(r.jumps.is_empty());
        assert!(r.worst.unwrap().unexplained_ms.abs() < 1e-6);
        assert!(r.zoom_changes.is_empty() && r.agg_changes.is_empty());
    }

    #[test]
    fn a_playhead_that_steps_is_a_jump_at_its_time() {
        let mut f = steady(300, 1.0);
        for x in &mut f[120..] {
            x.playhead += 0.015;
        }
        let r = check(&f, DEFAULT_JUMP_MS);
        assert_eq!(r.jumps.len(), 1);
        let j = r.jumps[0];
        assert!((j.unexplained_ms - 15.0).abs() < 1e-6, "{j:?}");
        assert!((j.at - 2.0).abs() < 1e-9, "{j:?}");
    }

    /// The platter stopped by hand inside a frame: the playhead moves less
    /// than the old rate says but within the band the two rates span.
    #[test]
    fn a_stop_inside_a_frame_is_no_jump() {
        let mut f = steady(120, 1.0);
        let at = f[60].playhead + 0.002;
        for x in &mut f[61..] {
            x.playhead = at;
            x.rate = 0.0;
        }
        let r = check(&f, DEFAULT_JUMP_MS);
        assert!(r.jumps.is_empty(), "{:?}", r.jumps);
    }

    /// A still deck is not redrawn; the frame after the pause is not a
    /// step across it.
    #[test]
    fn a_pause_in_drawing_is_reported_not_measured() {
        let mut f = steady(120, 1.0);
        f.drain(40..100);
        let r = check(&f, DEFAULT_JUMP_MS);
        assert!(r.jumps.is_empty(), "{:?}", r.jumps);
        assert_eq!(r.pauses, 1);
        assert!(
            (r.longest_pause_s - 1.0).abs() < 0.02,
            "{}",
            r.longest_pause_s
        );
    }

    #[test]
    fn a_pitched_deck_is_explained_by_its_rate() {
        let r = check(&steady(600, 1.08), DEFAULT_JUMP_MS);
        assert!(r.jumps.is_empty(), "{:?}", r.jumps);
    }

    #[test]
    fn a_zoom_that_changes_is_counted_and_float_noise_is_not() {
        let mut f = steady(120, 1.0);
        f[30].secs_per_px *= 1.0 + 1e-4;
        for x in &mut f[60..] {
            x.secs_per_px *= 1.5;
            x.agg = 4;
        }
        let r = check(&f, DEFAULT_JUMP_MS);
        assert_eq!(r.zoom_changes.len(), 1);
        assert!((r.zoom_changes[0].1 - 0.5).abs() < 1e-9);
        assert_eq!(r.agg_changes.len(), 1);
    }

    #[test]
    fn frames_without_a_track_are_left_out() {
        let mut f = steady(100, 1.0);
        for x in &mut f[..50] {
            x.has_track = false;
            x.playhead = 0.0;
        }
        let r = check(&f, DEFAULT_JUMP_MS);
        assert_eq!(r.frames, 50);
        assert!(r.jumps.is_empty());
    }

    /// Going back past a beat makes an earlier line the next one: a whole
    /// beat's jump in the number, not a wobble on screen.
    #[test]
    fn a_line_passed_backwards_is_not_a_wobble() {
        let mut f = steady(40, -1.0);
        let spp = f[0].secs_per_px;
        for x in &mut f {
            let beat = if x.playhead > 9.8 { 10.0 } else { 9.6 };
            x.next_beat_px = (beat - x.playhead) / spp;
        }
        let r = check(&f, DEFAULT_JUMP_MS);
        assert!(r.beat_wobble_px < 1e-6, "{}", r.beat_wobble_px);
    }

    #[test]
    fn a_beat_line_that_slides_with_the_playhead_does_not_wobble() {
        let mut f = steady(120, 1.0);
        let spp = f[0].secs_per_px;
        for x in &mut f {
            let beat = 10.5;
            x.next_beat_px = (beat - x.playhead) / spp;
        }
        let r = check(&f[..20], DEFAULT_JUMP_MS);
        assert!(r.beat_wobble_px < 1e-6, "{}", r.beat_wobble_px);
        f[10].next_beat_px += 3.0;
        let r = check(&f[..20], DEFAULT_JUMP_MS);
        assert!(
            (r.beat_wobble_px - 3.0).abs() < 1e-6,
            "{}",
            r.beat_wobble_px
        );
    }

    #[test]
    fn a_log_line_reads_back() {
        let f = parse_line("1500000000\t1\t1\t12.5\t0.98\t0.0029\t2\t41.5\t130\t130").unwrap();
        assert_eq!(f.deck, 1);
        assert!(f.has_track);
        assert!((f.t - 1.5).abs() < 1e-12);
        assert!((f.next_beat_px - 41.5).abs() < 1e-12);
        let nan = parse_line("1\t0\t1\t0\t1\t0.0029\t2\tnan\t130\t130").unwrap();
        assert!(nan.next_beat_px.is_nan());
        assert!(parse_line("# t_ns\tdeck").is_none());
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn args_take_the_log_and_a_window() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        let a = parse_args(&s(&["d.log", "--deck", "1", "--from", "3", "--to", "9.5"])).unwrap();
        assert_eq!(a.deck, Some(1));
        assert_eq!((a.from, a.to), (Some(3.0), Some(9.5)));
        assert!((a.jump_ms - DEFAULT_JUMP_MS).abs() < 1e-12);
        assert!(parse_args(&s(&[])).is_err());
        assert!(parse_args(&s(&["d.log", "--deck"])).is_err());
    }
}
