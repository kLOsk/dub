//! `dub diagnose` — the go-to debugger for grid / waveform / BPM
//! issues.
//!
//! Pulls together every signal we have about a track in one
//! report so the question "why did the grid land here?" or "why
//! did this tap move the bar?" can be answered without
//! launching the GUI or sprinkling NSLogs into the analyzer.
//!
//! ## Usage
//!
//! ```text
//! dub diagnose <path-or-track-id>     # full diagnosis
//! dub diagnose --list <query>         # find tracks by filename fragment
//! dub diagnose --grids <track-id>     # just the beat-grid rows
//! dub diagnose --taps <track-id>      # just the user-tap row (if any)
//! ```
//!
//! When called with a path, the tool:
//!
//! * Decodes the audio with `dub-io`.
//! * Runs the same `analyze_beat_grid_with_profile` pipeline the
//!   library uses for re-analyze, prints BPM, bar phase, fit
//!   quality, the first 8 beats with downbeat markers.
//! * Computes a broadband amplitude envelope and a low-band RMS
//!   envelope independently of the BPM pipeline. Prints their
//!   distribution stats, an ASCII plot of the opening 3 s, and
//!   the timestamps where the first really loud transient lands.
//!   This is the ground truth we compare the algorithm's
//!   chosen-downbeat against.
//! * Computes the offset between the algorithm's downbeat and the
//!   nearest amplitude peak, and emits a human-readable verdict.
//! * Looks the file up in the library by absolute path. If the
//!   track is registered, also prints the active beat-grid,
//!   every grid source (auto, user_tap, imported), the lock
//!   state, and the drift-quality slope.
//!
//! When called with a track UUID (a string of the form
//! `xxxxxxxx-xxxx-...`), the tool skips audio analysis and only
//! reports the library state. Useful when the file is on a
//! detached volume.
//!
//! ## Why one tool
//!
//! Beat-grid / BPM debugging crosses three crates (`dub-bpm`,
//! `dub-io`, `dub-library`) and the audio file on disk. Keeping
//! the diagnostic in one place removes the friction of "which
//! example do I run?" and makes it cheap to add new comparisons
//! as new failure modes show up. The output is wide on purpose:
//! when the bug is reported "after I tapped, the grid moved by
//! one beat at the chorus" you want the auto vs tap grids,
//! the bar_phase value, and the envelope all on one screen.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use dub_bpm::{analyze_beat_grid_with_profile, octave_profile_from_label, BeatGrid, OctaveProfile};
use dub_io::Track;
use dub_library::{ActiveBeatgrid, Library, TrackSortKey};
use rusqlite::params;

/// Top-level entry point bound to `dub diagnose ...` in `main.rs`.
pub fn run(args: &[String]) -> Result<()> {
    let mut mode = Mode::Full;
    let mut positional: Vec<String> = Vec::new();
    let mut profile_label: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--list" => mode = Mode::List,
            "--grids" => mode = Mode::GridsOnly,
            "--taps" => mode = Mode::TapsOnly,
            "--profile" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow!("--profile expects a label argument"))?;
                profile_label = Some(v.clone());
            }
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other if other.starts_with("--") => {
                return Err(anyhow!("unknown diagnose flag: {other}"));
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    if positional.is_empty() {
        print_usage();
        return Err(anyhow!("diagnose requires a path, track id, or query"));
    }
    let target = positional.join(" ");
    let profile = profile_label
        .as_deref()
        .map_or(OctaveProfile::Default, octave_profile_from_label);

    match mode {
        Mode::List => list_tracks(&target),
        Mode::GridsOnly => grids_only(&target),
        Mode::TapsOnly => taps_only(&target),
        Mode::Full => full_diagnosis(&target, profile),
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  dub diagnose <path-or-track-id> [--profile LABEL]   full diagnosis
  dub diagnose --list <query>                         find tracks by filename
  dub diagnose --grids <path-or-track-id>             beat-grid rows only
  dub diagnose --taps <path-or-track-id>              user-tap rows only

profile labels: default | reggae | roots | dub | dancehall | ragga | house | techno | garage |
                hip_hop | rap | trap | rnb | dnb | drum_and_bass | jungle"
    );
}

#[derive(Clone, Copy)]
enum Mode {
    Full,
    List,
    GridsOnly,
    TapsOnly,
}

// ---------------------------------------------------------------
// Top-level modes
// ---------------------------------------------------------------

fn full_diagnosis(target: &str, profile: OctaveProfile) -> Result<()> {
    let resolved = resolve_target(target)?;
    print_header(&resolved);

    if let ResolvedRef::Path { path, .. } | ResolvedRef::PathInLibrary { path, .. } = &resolved {
        let track =
            Track::load_from_path(path).with_context(|| format!("decoding {}", path.display()))?;
        print_audio_summary(&track);
        let grid = analyze_beat_grid_with_profile(
            track.samples(),
            track.sample_rate(),
            track.channels(),
            profile,
        )
        .context("analyze_beat_grid_with_profile")?;
        println!("\n=== FRESH ANALYSIS ({profile:?} profile) ===");
        print_grid_summary(&grid);
        println!();
        print_envelope_section(&track, &grid);
    } else {
        println!("(no on-disk path available; skipping audio analysis)");
    }

    if let Some((library, track_id)) = open_library_for(&resolved)? {
        println!("\n=== LIBRARY ===");
        print_library_context(&library, &track_id)?;
    } else {
        println!(
            "\n=== LIBRARY ===\n(track not found in default library at {})",
            display_library_path()
        );
    }
    Ok(())
}

fn list_tracks(query: &str) -> Result<()> {
    let library = open_default_library()?;
    let rows = library
        .list_tracks_sorted(2000, 0, TrackSortKey::Title, true)
        .context("list_tracks_sorted")?;
    let q = query.to_lowercase();
    let mut hits = 0usize;
    println!("{:<36}  {:<50}  path", "track id", "title -- artist");
    for row in rows {
        let display = format!(
            "{} -- {}",
            row.title.as_deref().unwrap_or("(no title)"),
            row.artist.as_deref().unwrap_or("(no artist)")
        );
        let path = reconstruct_path_string(&row);
        let haystack = format!("{} {}", display.to_lowercase(), path.to_lowercase());
        if !haystack.contains(&q) {
            continue;
        }
        println!("{:<36}  {:<50}  {}", row.id, truncate(&display, 50), path);
        hits += 1;
    }
    println!("\n  {hits} match(es)");
    Ok(())
}

fn reconstruct_path_string(row: &dub_library::TrackRow) -> String {
    match (
        row.primary_volume_mount_point.as_deref(),
        row.primary_relative_path.as_deref(),
    ) {
        (Some(mount), Some(rel)) => {
            let mut p = PathBuf::from(mount);
            p.push(rel.trim_start_matches('/'));
            p.display().to_string()
        }
        _ => "(no file)".to_string(),
    }
}

fn grids_only(target: &str) -> Result<()> {
    let resolved = resolve_target(target)?;
    print_header(&resolved);
    let (library, track_id) = open_library_for(&resolved)?.ok_or_else(|| {
        anyhow!(
            "track not registered in {} (use `dub diagnose --list` to search)",
            display_library_path()
        )
    })?;
    print_beatgrids(&library, &track_id)?;
    Ok(())
}

fn taps_only(target: &str) -> Result<()> {
    let resolved = resolve_target(target)?;
    print_header(&resolved);
    let (library, track_id) = open_library_for(&resolved)?.ok_or_else(|| {
        anyhow!(
            "track not registered in {} (use `dub diagnose --list` to search)",
            display_library_path()
        )
    })?;
    let rows = read_beatgrid_rows(&library, &track_id)?;
    let taps: Vec<_> = rows
        .into_iter()
        .filter(|r| r.source == "user_tap")
        .collect();
    if taps.is_empty() {
        println!("\n(no user_tap rows for this track)");
    } else {
        println!("\nuser_tap rows ({} total):", taps.len());
        for row in taps {
            println!(
                "  bpm={:.3}  anchor={:.4}s  phase={}  active={}  captured_at={}",
                row.bpm,
                row.anchor_secs,
                row.bar_phase,
                if row.is_active { "1" } else { "0" },
                format_unix_secs(row.captured_at),
            );
        }
    }
    Ok(())
}

/// Raw row returned by `read_beatgrid_rows` — mirrors the schema
/// of `track_beatgrids` directly so the diagnose output reflects
/// what the engine actually reads.
#[derive(Debug, Clone)]
struct BeatgridRow {
    source: String,
    bpm: f64,
    anchor_secs: f64,
    bar_phase: u8,
    is_active: bool,
    captured_at: i64,
}

/// All beatgrid rows for `track_id`, newest-source-first ordering
/// (active first, then by `captured_at DESC` as a tiebreak). Uses
/// the public `Library::connection` accessor so we don't have to
/// add a new method to `dub-library` just for the diagnose tool.
fn read_beatgrid_rows(library: &Library, track_id: &str) -> Result<Vec<BeatgridRow>> {
    let mut stmt = library
        .connection()
        .prepare(
            "SELECT source, bpm, anchor_secs, bar_phase, is_active, captured_at \
             FROM track_beatgrids \
             WHERE track_id = ?1 \
             ORDER BY is_active DESC, captured_at DESC",
        )
        .context("preparing track_beatgrids SELECT")?;
    let rows = stmt
        .query_map(params![track_id], |r| {
            Ok(BeatgridRow {
                source: r.get(0)?,
                bpm: r.get(1)?,
                anchor_secs: r.get(2)?,
                bar_phase: r.get::<_, i64>(3)? as u8,
                is_active: r.get::<_, i64>(4)? == 1,
                captured_at: r.get(5)?,
            })
        })
        .context("query track_beatgrids")?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.context("decoding track_beatgrids row")?);
    }
    Ok(out)
}

// ---------------------------------------------------------------
// Audio + grid output
// ---------------------------------------------------------------

fn print_header(r: &ResolvedRef) {
    match r {
        ResolvedRef::Path { path, .. } => {
            println!("file:        {}", path.display());
            println!("library:     (not registered)");
        }
        ResolvedRef::PathInLibrary { path, track_id } => {
            println!("file:        {}", path.display());
            println!("track id:    {track_id}");
            println!("library:     {}", display_library_path());
        }
        ResolvedRef::IdOnly { track_id, path } => {
            println!("track id:    {track_id}");
            if let Some(p) = path {
                println!("path:        {}", p.display());
            } else {
                println!("path:        (unresolved — volume not mounted)");
            }
            println!("library:     {}", display_library_path());
        }
    }
}

fn print_audio_summary(track: &Track) {
    let sr = track.sample_rate();
    let ch = track.channels();
    let frames = track.samples().len() / usize::from(ch.max(1));
    let duration = frames as f64 / f64::from(sr);
    println!(
        "audio:       {} Hz, {} ch, {:.2} s ({} frames)",
        sr, ch, duration, frames
    );
}

fn print_grid_summary(grid: &BeatGrid) {
    println!("  bpm:                  {:.3}", grid.bpm);
    println!("  bpm confidence:       {:.4}", grid.confidence);
    println!("  downbeat confidence:  {:.4}", grid.downbeat_confidence);
    println!("  bar_phase:            {}", grid.bar_phase);
    println!("  beats_per_bar:        {}", grid.beats_per_bar);
    println!("  total beats:          {}", grid.beats.len());
    if let Some(q) = grid.quality {
        println!(
            "  fit quality:          rms={:.3} ms  p95={:.3} ms  max={:.3} ms  kept={:.0}%  drift={:+.2} ms/min",
            q.rms_ms,
            q.p95_ms,
            q.max_abs_ms,
            q.kept_fraction * 100.0,
            q.drift_slope_ms_per_min,
        );
    }
    println!("\nfirst 8 beats (* = downbeat per bar_phase)");
    let bpb = usize::from(grid.beats_per_bar.max(1));
    let phase = usize::from(grid.bar_phase);
    for (i, t) in grid.beats.iter().take(8).enumerate() {
        let is_downbeat = bpb > 0 && i % bpb == phase;
        let marker = if is_downbeat { "*" } else { " " };
        println!("  {marker} beat[{i:>2}] @ {t:8.4} s");
    }
}

// ---------------------------------------------------------------
// Envelope (broadband + lowband)
// ---------------------------------------------------------------

const ENVELOPE_HOP_SECS: f64 = 0.010;
const ENVELOPE_WINDOW_SECS: f64 = 0.020;
const LOWBAND_WINDOW_SECS: f64 = 0.020;

fn print_envelope_section(track: &Track, grid: &BeatGrid) {
    let env = broadband_envelope(track.samples(), track.sample_rate(), track.channels());
    let lowband = lowband_rms_envelope(track.samples(), track.sample_rate(), track.channels());
    println!("=== ENVELOPE ===");
    print_envelope_stats("broadband", &env);
    print_first_significant(&env, "broadband", 0.50);
    print_envelope_stats("low-band ", &lowband);
    print_first_significant(&lowband, "low-band ", 0.40);
    print_envelope_plot("broadband", &env, 3.0, 5);
    print_envelope_plot("low-band ", &lowband, 3.0, 5);

    if grid.beats.is_empty() {
        return;
    }
    let phase = usize::from(grid.bar_phase).min(grid.beats.len() - 1);
    let downbeat = grid.beats[phase];
    let period = if grid.bpm > 0.0 { 60.0 / grid.bpm } else { 0.0 };

    println!("\n=== DOWNBEAT ALIGNMENT ===");
    println!("  chosen downbeat: {downbeat:.4} s   (beats[bar_phase])");
    println!("  period:          {period:.4} s   (60 / bpm)");
    let first_peak = first_envelope_peak(&env, 0.50);
    let first_low = first_envelope_peak(&lowband, 0.40);
    if let Some((t, v)) = first_peak {
        let offset_ms = (downbeat - t) * 1_000.0;
        println!(
            "  first broadband peak ≥ 50% of max: {t:.4} s  (amp={v:.3})  Δ={:+.1} ms",
            offset_ms
        );
    }
    if let Some((t, v)) = first_low {
        let offset_ms = (downbeat - t) * 1_000.0;
        println!(
            "  first low-band  peak ≥ 40% of max: {t:.4} s  (rms={v:.4})  Δ={:+.1} ms",
            offset_ms
        );
    }
    println!(
        "  verdict: {}",
        verdict(downbeat, first_peak, first_low, period, grid)
    );
}

fn broadband_envelope(samples: &[f32], sr: u32, ch: u8) -> Vec<f32> {
    let channels = usize::from(ch.max(1));
    let frames = samples.len() / channels;
    let hop = ((f64::from(sr) * ENVELOPE_HOP_SECS).round() as usize).max(1);
    let win = ((f64::from(sr) * ENVELOPE_WINDOW_SECS).round() as usize).max(1);
    let mut out = Vec::with_capacity(frames / hop + 1);
    let mut start = 0usize;
    while start < frames {
        let end = (start + win).min(frames);
        let mut peak = 0.0f32;
        for f in start..end {
            let mut acc = 0.0f32;
            for c in 0..channels {
                acc += samples[f * channels + c].abs();
            }
            let avg = acc / channels as f32;
            if avg > peak {
                peak = avg;
            }
        }
        out.push(peak);
        start += hop;
    }
    out
}

fn lowband_rms_envelope(samples: &[f32], sr: u32, ch: u8) -> Vec<f32> {
    let channels = usize::from(ch.max(1));
    let frames = samples.len() / channels;
    let hop = ((f64::from(sr) * ENVELOPE_HOP_SECS).round() as usize).max(1);
    let win = ((f64::from(sr) * LOWBAND_WINDOW_SECS).round() as usize).max(1);
    let mut out = Vec::with_capacity(frames / hop + 1);
    let mut start = 0usize;
    while start < frames {
        let end = (start + win).min(frames);
        let mut acc = 0.0f64;
        let mut prev = 0.0f64;
        let mut count = 0u64;
        for f in start..end {
            let mut mono = 0.0f64;
            for c in 0..channels {
                mono += f64::from(samples[f * channels + c]);
            }
            mono /= channels as f64;
            let lp = 0.95 * prev + 0.05 * mono;
            acc += lp * lp;
            prev = lp;
            count += 1;
        }
        let rms = if count > 0 {
            (acc / count as f64).sqrt() as f32
        } else {
            0.0
        };
        out.push(rms);
        start += hop;
    }
    out
}

fn print_envelope_stats(label: &str, envelope: &[f32]) {
    if envelope.is_empty() {
        return;
    }
    let mut s = envelope.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |q: f64| s[((s.len() - 1) as f64 * q).round() as usize];
    println!(
        "  {label} stats: min={:.4} p25={:.4} p50={:.4} p75={:.4} p95={:.4} max={:.4}",
        pct(0.0),
        pct(0.25),
        pct(0.50),
        pct(0.75),
        pct(0.95),
        pct(1.0)
    );
}

fn print_first_significant(envelope: &[f32], label: &str, fraction_of_max: f32) {
    let max = envelope
        .iter()
        .copied()
        .fold(0.0f32, |a, b| if b > a { b } else { a });
    let threshold = max * fraction_of_max;
    for (i, v) in envelope.iter().enumerate() {
        if *v >= threshold {
            println!(
                "  first {label} hop ≥ {:.0}% of max ({:.3}): t={:.4} s  v={:.3}",
                fraction_of_max * 100.0,
                max,
                i as f64 * ENVELOPE_HOP_SECS,
                v
            );
            return;
        }
    }
    println!(
        "  {label} never crossed {:.0}% of max ({:.3})",
        fraction_of_max * 100.0,
        max
    );
}

fn print_envelope_plot(label: &str, envelope: &[f32], duration_secs: f64, every_n: usize) {
    println!(
        "\n  {label} envelope, first {duration_secs:.1} s, {} ms hop:",
        (ENVELOPE_HOP_SECS * every_n as f64 * 1_000.0).round() as i64
    );
    let n = ((duration_secs / ENVELOPE_HOP_SECS).round() as usize).min(envelope.len());
    let max = envelope
        .iter()
        .copied()
        .fold(0.0f32, |a, b| if b > a { b } else { a })
        .max(1e-6);
    for i in (0..n).step_by(every_n) {
        let t = i as f64 * ENVELOPE_HOP_SECS;
        let v = envelope[i];
        let bar_width = ((v / max) * 60.0).clamp(0.0, 60.0) as usize;
        let bar = "#".repeat(bar_width);
        println!("    {t:6.3} s  {v:.4}  {bar}");
    }
}

fn first_envelope_peak(envelope: &[f32], fraction_of_max: f32) -> Option<(f64, f32)> {
    let max = envelope
        .iter()
        .copied()
        .fold(0.0f32, |a, b| if b > a { b } else { a });
    if max <= 0.0 {
        return None;
    }
    let threshold = max * fraction_of_max;
    envelope
        .iter()
        .enumerate()
        .find(|(_, v)| **v >= threshold)
        .map(|(i, v)| (i as f64 * ENVELOPE_HOP_SECS, *v))
}

fn verdict(
    downbeat: f64,
    first_peak: Option<(f64, f32)>,
    first_low: Option<(f64, f32)>,
    period: f64,
    grid: &BeatGrid,
) -> String {
    let Some((peak_t, _)) = first_peak.or(first_low) else {
        return "no envelope landmark — degenerate audio".into();
    };
    let dt = downbeat - peak_t;
    if dt.abs() < period * 0.5 {
        format!(
            "downbeat sits within ±half a beat of the first audible transient (Δ={:+.0} ms)",
            dt * 1_000.0
        )
    } else if period > 0.0 {
        let beats = dt / period;
        format!(
            "downbeat is {:+.2} beats away from the first audible transient \
             (period={period:.3}s, conf={:.3}) — likely bar-phase or first-kick mis-pick",
            beats, grid.downbeat_confidence
        )
    } else {
        "no period (grid has zero BPM)".into()
    }
}

// ---------------------------------------------------------------
// Library inspection
// ---------------------------------------------------------------

fn print_library_context(library: &Library, track_id: &str) -> Result<()> {
    let active = library.active_beatgrid_for_track(track_id)?;
    if let Some(a) = &active {
        println!("active beatgrid:");
        print_active_beatgrid(a);
    } else {
        println!("active beatgrid: (none — track never analyzed or grids absent)");
    }
    print_beatgrids(library, track_id)?;
    Ok(())
}

fn print_active_beatgrid(a: &ActiveBeatgrid) {
    println!("  source:         {}", a.source);
    println!("  bpm:            {:.3}", a.bpm);
    println!("  anchor:         {:.4} s", a.anchor_secs);
    println!("  bar_phase:      {}", a.bar_phase);
    println!("  grid_locked:    {}", a.grid_locked);
    if let Some(q) = a.grid_drift_quality {
        println!("  drift quality:  {:+.3} ms/min", q);
    } else {
        println!("  drift quality:  (none)");
    }
    println!("  captured_at:    {}", format_unix_secs(a.captured_at));
    if let Some(p) = &a.waveform_sidecar_path {
        println!("  waveform .wf:   {p}");
    } else {
        println!("  waveform .wf:   (none — sidecar will be rebuilt on next analyze)");
    }
}

fn print_beatgrids(library: &Library, track_id: &str) -> Result<()> {
    let rows = read_beatgrid_rows(library, track_id)?;
    if rows.is_empty() {
        println!("\nbeatgrid rows: (none)");
        return Ok(());
    }
    println!("\nbeatgrid rows ({} total):", rows.len());
    println!(
        "  {:<10}  {:>3}  {:>9}  {:>10}  {:>5}  captured_at",
        "source", "act", "bpm", "anchor", "phase"
    );
    for r in rows {
        println!(
            "  {:<10}  {:>3}  {:>9.3}  {:>10.4}  {:>5}  {}",
            r.source,
            if r.is_active { "1" } else { "0" },
            r.bpm,
            r.anchor_secs,
            r.bar_phase,
            format_unix_secs(r.captured_at),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------
// Target resolution + library helpers
// ---------------------------------------------------------------

enum ResolvedRef {
    /// A path that exists on disk but is not in the library.
    Path { path: PathBuf },
    /// A path that exists on disk AND is registered in the library.
    PathInLibrary { path: PathBuf, track_id: String },
    /// A UUID-shaped string. Path is `Some` only if a `track_files`
    /// row exists for a currently-mounted volume.
    IdOnly {
        track_id: String,
        path: Option<PathBuf>,
    },
}

fn resolve_target(target: &str) -> Result<ResolvedRef> {
    let looks_like_uuid = is_uuid_shape(target);
    if looks_like_uuid {
        let library = open_default_library()?;
        let path = library.resolve_track_path(target).unwrap_or(None);
        Ok(ResolvedRef::IdOnly {
            track_id: target.to_string(),
            path,
        })
    } else {
        let path = PathBuf::from(target);
        if !path.exists() {
            return Err(anyhow!(
                "no such file (or unrecognised id): {}",
                path.display()
            ));
        }
        let abs = path.canonicalize().unwrap_or(path);
        let library = open_default_library_optional();
        if let Some(library) = library {
            let id = library.track_id_for_absolute_path(&abs).unwrap_or(None);
            if let Some(id) = id {
                return Ok(ResolvedRef::PathInLibrary {
                    path: abs,
                    track_id: id,
                });
            }
        }
        Ok(ResolvedRef::Path { path: abs })
    }
}

fn open_library_for(r: &ResolvedRef) -> Result<Option<(Library, String)>> {
    match r {
        ResolvedRef::PathInLibrary { track_id, .. } => {
            let library = open_default_library()?;
            Ok(Some((library, track_id.clone())))
        }
        ResolvedRef::IdOnly { track_id, .. } => {
            let library = open_default_library()?;
            Ok(Some((library, track_id.clone())))
        }
        ResolvedRef::Path { .. } => Ok(None),
    }
}

fn open_default_library() -> Result<Library> {
    Library::open_default().context("opening default library")
}

fn open_default_library_optional() -> Option<Library> {
    Library::open_default().ok()
}

fn display_library_path() -> String {
    dub_library::default_library_db_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<no library path>".to_string())
}

fn is_uuid_shape(s: &str) -> bool {
    // Loose check: 36 chars, hyphens at the right places.
    s.len() == 36
        && s.as_bytes().iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn format_unix_secs(secs: i64) -> String {
    use time::OffsetDateTime;
    OffsetDateTime::from_unix_timestamp(secs)
        .map(|t| {
            t.format(
                &time::format_description::parse(
                    "[year]-[month]-[day] [hour]:[minute]:[second] UTC",
                )
                .unwrap(),
            )
            .unwrap_or_else(|_| secs.to_string())
        })
        .unwrap_or_else(|_| secs.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}
