//! Does the beat tracker read a band's tempo, and only the band's?
//!
//! Follow mode puts a line on each beat a band played, and straightening a
//! drifting record to a steady tempo is built on the same reading. Two
//! questions decide whether that reading can be trusted, and each has a
//! ground truth on the operator's own music:
//!
//! - **A steady record must read steady.** A grid locked in Traktor is one
//!   tempo that the DJ found good enough to mix on. Tracked, such a record
//!   should give back that grid: beats on its lines, and a tempo that does
//!   not move. Whatever the tracker reads beyond that is its own wobble.
//! - **A record that drifts must read as it drifts.** The same records are
//!   bent by a known tempo curve — the stretcher plays them a percent fast
//!   and slow in turn — and the tracker should read the curve back.
//!
//! The corpus is the manifest `traktor_grid_corpus.rs` uses (see its header
//! for the query); this test is skipped without it:
//!
//! ```sh
//! DUB_BPM_TRAKTOR_CORPUS=/tmp/traktor-locked.tsv \
//!   cargo test --release -p dub-bpm --test follow_corpus -- --nocapture
//! ```
//!
//! `DUB_BPM_FOLLOW_EVERY=n` reads every nth song (default 6, about 300
//! songs and three minutes); 1 reads them all.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use dub_bpm::{band_pulse, followed_beats};
use dub_io::Track;
use dub_stretch::{SpliceStretcher, TimeStretcher};

const MANIFEST_ENV: &str = "DUB_BPM_TRAKTOR_CORPUS";
const EVERY_ENV: &str = "DUB_BPM_FOLLOW_EVERY";

/// A beat this close to its line is on it.
const ON_THE_BEAT_MS: f64 = 25.0;
/// Beats the tempo is read over, and the step between readings: four bars,
/// every bar. What a record is straightened by, so what has to be right.
const TEMPO_BEATS: usize = 16;
const TEMPO_STEP: usize = 4;

/// The drift put on: a band a percent either side of its tempo, there and
/// back every 45 seconds. 110 to 112 BPM is this much.
const WANDER: f64 = 0.01;
const WANDER_SECS: f64 = 45.0;

struct Song {
    path: PathBuf,
    bpm: f64,
    anchor_secs: f64,
}

/// One song read by each tracker: steady, and drifting.
type BothTrackers = [(Reading, Reading); 2];

/// What the tracker made of one record, against lines known to be right.
struct Reading {
    /// Share of tracked beats within [`ON_THE_BEAT_MS`] of a true line, or
    /// of the point half way between two.
    on_the_beat: f64,
    /// The tracked tempo's distance from the true tempo over four bars, as
    /// a share of the tempo: root mean square, and the worst reading.
    tempo_rms: f64,
    tempo_worst: f64,
    /// Where in the record the worst reading is, 0 its start and 1 its end.
    worst_at: f64,
    /// The same root mean square with the first and last 20 seconds left
    /// out: intros and endings are where a record has no beat to read.
    body_rms: f64,
    body_worst: f64,
}

#[test]
fn the_tracker_reads_the_bands_tempo_and_only_the_bands() {
    let Some(manifest) = std::env::var_os(MANIFEST_ENV) else {
        eprintln!("{MANIFEST_ENV} is not set; skipping");
        return;
    };
    let every: usize = std::env::var(EVERY_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6)
        .max(1);
    let songs: Vec<Song> = read_manifest(&PathBuf::from(manifest))
        .into_iter()
        .step_by(every)
        .collect();
    assert!(!songs.is_empty(), "the manifest names no songs");

    let readings = read_all(&songs);
    let read: Vec<&BothTrackers> = readings.iter().flatten().collect();
    eprintln!("\n{} songs, {} tracked", songs.len(), read.len());
    assert!(!read.is_empty(), "no song could be read");

    let line = |name: &str, readings: &[&Reading]| {
        let on: Vec<f64> = readings.iter().map(|r| r.on_the_beat).collect();
        let rms: Vec<f64> = readings.iter().map(|r| r.tempo_rms).collect();
        let worst: Vec<f64> = readings.iter().map(|r| r.tempo_worst).collect();
        eprintln!(
            "  {name}: beats on a line or half way {:.0} % (median song; worst tenth {:.0} %) · \
             tempo over four bars off by {:.2} % rms (median; worst tenth {:.2} %), \
             {:.2} % at its worst (median; worst tenth {:.2} %)",
            centile(&on, 0.5) * 100.0,
            centile(&on, 0.1) * 100.0,
            centile(&rms, 0.5) * 100.0,
            centile(&rms, 0.9) * 100.0,
            centile(&worst, 0.5) * 100.0,
            centile(&worst, 0.9) * 100.0,
        );
    };
    let body = |name: &str, readings: &[&Reading]| {
        let rms: Vec<f64> = readings.iter().map(|r| r.body_rms).collect();
        let worst: Vec<f64> = readings.iter().map(|r| r.body_worst).collect();
        let share = |limit: f64| {
            #[allow(clippy::cast_precision_loss)]
            {
                worst.iter().filter(|w| **w <= limit).count() as f64 * 100.0 / worst.len() as f64
            }
        };
        eprintln!(
            "  {name}, first and last 20 s left out: {:.2} % rms (median; worst tenth {:.2} %), \
             {:.2} % at its worst (median; worst tenth {:.2} %) · never over 0.3 %: {:.0} % of \
             songs, never over 0.5 %: {:.0} %, never over 1 %: {:.0} %",
            centile(&rms, 0.5) * 100.0,
            centile(&rms, 0.9) * 100.0,
            centile(&worst, 0.5) * 100.0,
            centile(&worst, 0.9) * 100.0,
            share(0.003),
            share(0.005),
            share(0.01),
        );
    };
    if std::env::var_os("DUB_BPM_FOLLOW_DETAIL").is_some() {
        for (song, reading) in songs.iter().zip(&readings) {
            if let Some(reading) = reading {
                let pulse = &reading[1].0;
                eprintln!(
                    "DETAIL\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.1}\t{}",
                    pulse.on_the_beat,
                    pulse.tempo_rms * 100.0,
                    pulse.tempo_worst * 100.0,
                    pulse.worst_at,
                    pulse.body_worst * 100.0,
                    song.bpm,
                    song.path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                );
            }
        }
    }
    for (tracker, name) in TRACKERS.iter().enumerate() {
        eprintln!("{name}");
        line(
            "steady records",
            &read.iter().map(|r| &r[tracker].0).collect::<Vec<_>>(),
        );
        body(
            "steady records",
            &read.iter().map(|r| &r[tracker].0).collect::<Vec<_>>(),
        );
        line(
            "drifting a percent either way",
            &read.iter().map(|r| &r[tracker].1).collect::<Vec<_>>(),
        );
        body(
            "drifting a percent either way",
            &read.iter().map(|r| &r[tracker].1).collect::<Vec<_>>(),
        );
    }

    let (steady_within, steady_rms) =
        body_summary(&read.iter().map(|r| &r[1].0).collect::<Vec<_>>());
    let (drifting_within, drifting_rms) =
        body_summary(&read.iter().map(|r| &r[1].1).collect::<Vec<_>>());
    assert!(
        steady_within >= MIN_STEADY_WITHIN_HALF_A_PERCENT && steady_rms <= MAX_STEADY_RMS,
        "the pulse of a steady record: {:.0} % of songs within half a percent, {:.3} % rms",
        steady_within * 100.0,
        steady_rms * 100.0
    );
    assert!(
        drifting_within >= MIN_DRIFTING_WITHIN_HALF_A_PERCENT && drifting_rms <= MAX_DRIFTING_RMS,
        "the pulse of a drifting record: {:.0} % of songs within half a percent, {:.3} % rms",
        drifting_within * 100.0,
        drifting_rms * 100.0
    );
}

/// Floors for the pulse, set under what every sixth song of 1,767 read on
/// 2026-11-19: with the first and last 20 seconds left out, 91 % of steady
/// records never read more than half a percent off (0.03 % rms on the median
/// song), and 87 % of the same records drifting (0.06 %). The line on each
/// hit, for scale: 31 % and 27 %.
const MIN_STEADY_WITHIN_HALF_A_PERCENT: f64 = 0.87;
const MAX_STEADY_RMS: f64 = 0.0005;
const MIN_DRIFTING_WITHIN_HALF_A_PERCENT: f64 = 0.82;
const MAX_DRIFTING_RMS: f64 = 0.0009;

/// Share of songs whose tempo never reads more than half a percent off in
/// the body, and the median song's rms there.
fn body_summary(readings: &[&Reading]) -> (f64, f64) {
    let rms: Vec<f64> = readings.iter().map(|r| r.body_rms).collect();
    let within = readings.iter().filter(|r| r.body_worst <= 0.005).count();
    #[allow(clippy::cast_precision_loss)]
    (within as f64 / readings.len() as f64, centile(&rms, 0.5))
}

/// The two readings compared: a line on each hit, and the pulse underneath.
const TRACKERS: [&str; 2] = ["a line on each hit (follow)", "the pulse (band_pulse)"];

fn track(tracker: usize, samples: &[f32], rate: u32, channels: u8, bpm: f64) -> Option<Vec<f64>> {
    if tracker == 0 {
        followed_beats(samples, rate, channels, bpm)
    } else {
        band_pulse(samples, rate, channels, bpm)
    }
}

fn centile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let at = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[at]
}

fn read_manifest(path: &PathBuf) -> Vec<Song> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read manifest {}: {e}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            Some(Song {
                path: PathBuf::from(cols.first()?),
                bpm: cols.get(2)?.trim().parse().ok()?,
                anchor_secs: cols.get(3)?.trim().parse().ok()?,
            })
        })
        .collect()
}

fn read_all(songs: &[Song]) -> Vec<Option<BothTrackers>> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<BothTrackers>>> = Mutex::new(songs.iter().map(|_| None).collect());
    let workers =
        std::thread::available_parallelism().map_or(4, |n| n.get().saturating_sub(2).max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(song) = songs.get(i) else {
                    break;
                };
                let reading = read(song);
                out.lock().expect("no worker panics holding this")[i] = reading;
            });
        }
    });
    out.into_inner().expect("no worker panics holding this")
}

fn read(song: &Song) -> Option<BothTrackers> {
    let track = Track::load_from_path(&song.path).ok()?;
    let (rate, channels) = (track.sample_rate(), track.channels());
    let sr = f64::from(rate);
    #[allow(clippy::cast_precision_loss)]
    let secs = track.frames() as f64 / sr;
    if !(60.0..=600.0).contains(&secs) || song.bpm <= 0.0 {
        return None;
    }
    let period = 60.0 / song.bpm;
    let first = song.anchor_secs.rem_euclid(period);
    let lines: Vec<f64> = (0..)
        .map(|n| first + f64::from(n) * period)
        .take_while(|&t| t < secs)
        .collect();

    // The same record, drifting.
    let stereo: Vec<f32> = (0..track.frames()).flat_map(|i| track.frame(i)).collect();
    let (bent, played_at) = bend(&stereo, rate, |t| {
        1.0 + WANDER * (std::f64::consts::TAU * t / WANDER_SECS).sin()
    });
    let bent_lines: Vec<f64> = lines.iter().map(|&t| played_at(t)).collect();

    let both = |tracker: usize| -> Option<(Reading, Reading)> {
        let steady = self::track(tracker, track.samples(), rate, channels, song.bpm)?;
        let drifting = self::track(tracker, &bent, rate, 2, song.bpm)?;
        Some((
            reading(&steady, &lines, period)?,
            reading(&drifting, &bent_lines, period)?,
        ))
    };
    Some([both(0)?, both(1)?])
}

/// Play `stereo` at `speed(t)` times its tempo, `t` in the record's own
/// seconds, pitch held. Returns the result and the map from a moment of the
/// record to the moment it is heard.
fn bend(stereo: &[f32], rate: u32, speed: impl Fn(f64) -> f64) -> (Vec<f32>, impl Fn(f64) -> f64) {
    #[allow(clippy::cast_precision_loss)]
    let mut stretcher = SpliceStretcher::new(rate as f32);
    let sr = f64::from(rate);
    let mut out = Vec::with_capacity(stereo.len() + stereo.len() / 50);
    let mut scratch = vec![0.0f32; 480 * 2];
    // (frames heard, frames of the record the tempo has got through).
    let mut map: Vec<(f64, f64)> = vec![(0.0, 0.0)];
    let (mut cursor, mut record) = (0usize, 0.0f64);
    loop {
        let now = speed(record / sr);
        stretcher.set_time_ratio(1.0 / now);
        let (consumed, produced) = stretcher.process(&stereo[cursor * 2..], &mut scratch);
        cursor += consumed;
        out.extend_from_slice(&scratch[..produced * 2]);
        #[allow(clippy::cast_precision_loss)]
        {
            record += produced as f64 * now;
            map.push(((out.len() / 2) as f64, record));
        }
        if consumed == 0 && produced == 0 {
            break;
        }
    }
    let played_at = move |record_secs: f64| -> f64 {
        let target = record_secs * sr;
        let i = map
            .partition_point(|&(_, r)| r < target)
            .clamp(1, map.len() - 1);
        let ((h0, r0), (h1, r1)) = (map[i - 1], map[i]);
        let within = if r1 > r0 {
            (target - r0) / (r1 - r0)
        } else {
            0.0
        };
        (h0 + within * (h1 - h0)) / sr
    };
    (out, played_at)
}

/// `tracked` beats against the true `lines`.
fn reading(tracked: &[f64], lines: &[f64], period: f64) -> Option<Reading> {
    if tracked.len() < 2 * TEMPO_BEATS || lines.len() < 2 * TEMPO_BEATS {
        return None;
    }
    let nearest = |t: f64| -> f64 {
        let i = lines.partition_point(|&l| l < t);
        let before = lines
            .get(i.wrapping_sub(1))
            .map_or(f64::MAX, |l| (t - l).abs());
        let after = lines.get(i).map_or(f64::MAX, |l| (l - t).abs());
        before.min(after)
    };
    let (from, to) = (lines[0], lines[lines.len() - 1]);
    let inside: Vec<f64> = tracked
        .iter()
        .copied()
        .filter(|&t| t >= from && t <= to)
        .collect();
    if inside.len() < 2 * TEMPO_BEATS {
        return None;
    }
    let on = inside
        .iter()
        .filter(|&&t| {
            // On a line, or half way between two: which of the two a
            // tracker sits on is not what is measured here.
            let off = nearest(t);
            off.min((off - period / 2.0).abs()) * 1_000.0 <= ON_THE_BEAT_MS
        })
        .count();

    // The true tempo at a moment: over the same four bars of true lines.
    let tempo_of = |beats: &[f64], i: usize| -> f64 {
        #[allow(clippy::cast_precision_loss)]
        {
            TEMPO_BEATS as f64 / (beats[i + TEMPO_BEATS] - beats[i])
        }
    };
    let (mut sum, mut worst, mut count) = (0.0f64, 0.0f64, 0usize);
    let (mut body_sum, mut body_worst, mut body_count, mut worst_at) =
        (0.0f64, 0.0f64, 0usize, 0.0f64);
    let mut i = 0;
    while i + TEMPO_BEATS < inside.len() {
        let middle = (inside[i] + inside[i + TEMPO_BEATS]) / 2.0;
        // The true lines' four bars centred on the same moment.
        let centre = lines.partition_point(|&l| l < middle);
        let start = centre.saturating_sub(TEMPO_BEATS / 2);
        if start + TEMPO_BEATS < lines.len() {
            let off = tempo_of(&inside, i) / tempo_of(lines, start) - 1.0;
            sum += off * off;
            if off.abs() > worst {
                worst = off.abs();
                worst_at = (middle - from) / (to - from);
            }
            count += 1;
            if middle > from + 20.0 && middle < to - 20.0 {
                body_sum += off * off;
                body_worst = body_worst.max(off.abs());
                body_count += 1;
            }
        }
        i += TEMPO_STEP;
    }
    if count == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some(Reading {
        on_the_beat: on as f64 / inside.len() as f64,
        tempo_rms: (sum / count as f64).sqrt(),
        tempo_worst: worst,
        worst_at,
        body_rms: (body_sum / body_count.max(1) as f64).sqrt(),
        body_worst,
    })
}
