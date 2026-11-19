//! The analyser scored against grids a DJ locked in Traktor.
//!
//! The operator's Traktor lock means "this grid is right, or good enough
//! to mix on" — for the tempo and for where the lines sit. It says
//! nothing about which beat is the 1 (he never corrected that in
//! Traktor), and an *unlocked* grid is his mark for a song that drifts,
//! so neither is scored here. Serato grids are no reference at all.
//!
//! On 2026-11-19 the detector as it then stood — a tempo prior over
//! 60–200 BPM and nine genre-specific rejection rules — agreed with
//! 62.4 % of 1,758 such grids on the tempo. Most misses were not
//! octaves: 16 % read two-thirds (128 as 85.3) and 7 % four-thirds
//! (70 as 93). With one octave per genre family and no rules
//! ([`dub_bpm::TempoOctave`]) the same songs come out at 96.3 %. This test
//! is that measurement, kept so the next change to tempo or placement is
//! scored the same way instead of against the 94 fixture songs alone.
//!
//! # Running it
//!
//! The corpus is the operator's own music, so it is not in the repo.
//! Write the manifest from the library database (read-only):
//!
//! ```sh
//! sqlite3 -separator "$(printf '\t')" \
//!   "file:$HOME/Library/Application Support/Dub/library.sqlite?mode=ro" "
//!   SELECT rtrim(v.last_known_mount_point, '/') || '/' || f.relative_path,
//!          COALESCE((SELECT genre FROM track_metadata_source m
//!                     WHERE m.track_id = g.track_id
//!                       AND TRIM(COALESCE(genre, '')) != ''
//!                     ORDER BY source = 'id3' DESC LIMIT 1), ''),
//!          g.bpm, g.anchor_secs
//!     FROM track_beatgrids g
//!     JOIN track_files f ON f.track_id = g.track_id AND f.is_missing = 0
//!     JOIN volumes v USING (volume_uuid)
//!    WHERE g.source = 'traktor' AND g.source_locked = 1
//!    GROUP BY g.track_id" > /tmp/traktor-locked.tsv
//! ```
//!
//! and point the test at it (about twelve minutes for 1,800 songs):
//!
//! ```sh
//! DUB_BPM_TRAKTOR_CORPUS=/tmp/traktor-locked.tsv \
//!   cargo nextest run -p dub-bpm --release --test traktor_grid_corpus --no-capture
//! ```
//!
//! Without the variable the test says so and passes.
//!
//! # What it asserts
//!
//! * **Tempo**: the share of songs whose tempo is Traktor's, within 2 %.
//!   Every miss is named by its ratio (half, double, 2:3, 4:3 …).
//! * **Placement**, on the songs whose tempo agrees to 0.05 BPM: the
//!   share whose lines sit within 25 ms of Traktor's in the middle of
//!   the song, and how many sit half a beat off.
//!
//! * **Rough taps**: each song is also "tapped" the way a DJ taps — a
//!   tempo 8 % out (under on every other song, over on the rest) and one
//!   tap 35 ms late on a beat in the middle of the song — and the grid
//!   that comes back is held to Traktor's tempo within 0.05 BPM and its
//!   lines within 25 ms. This is tap tempo's contract: the taps are a
//!   hint, the song is the answer.
//!
//! * **Drift warning**: a locked grid is one he mixed on, so Dub's drift
//!   reading of it ([`dub_bpm::GridQuality::drifts`], measured the way a
//!   set-the-1 on a playing deck measures it) should seldom warn. The
//!   same grid with its tempo made 12 ms a minute slow should nearly
//!   always warn. Both shares are held, and so is how often Dub warns
//!   about its own grid.
//!
//! The floors are a little under what was measured, so they catch a
//! regression and not a rounding. Songs are analysed as the app would:
//! in the octave of their genre, or the default range without one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use dub_bpm::{
    analyze_beat_grid_from_bpm_and_anchor, analyze_beat_grid_with_profile, fit_grid_at_playing_tap,
    octave_profile_from_genre, OctaveProfile,
};
use dub_io::Track;

const MANIFEST_ENV: &str = "DUB_BPM_TRAKTOR_CORPUS";

/// Tempos within this fraction of each other are the same tempo.
const SAME_TEMPO: f64 = 0.02;
/// Lines are only compared where the two tempos would not drift apart
/// over a song.
const SAME_GRID_BPM: f64 = 0.05;
/// A DJ does not hear a line this close to the beat as off it.
const ON_THE_BEAT_MS: f64 = 25.0;

/// Measured 96.3 % on 1,767 songs (2026-11-19).
const MIN_TEMPO_AGREEMENT: f64 = 0.95;
/// Measured 87.0 % on the 1,631 of them within 0.05 BPM of Traktor, 58
/// half a beat off (84.7 % and 105 before `move_off_the_off_beat`).
const MIN_ON_THE_BEAT: f64 = 0.85;
/// Measured 94.3 %: rough taps that come back at Traktor's tempo.
const MIN_TAPS_FIND_THE_TEMPO: f64 = 0.92;
/// Measured 85.9 %: …and with their lines on Traktor's.
const MIN_TAPS_FIND_THE_BEAT: f64 = 0.82;

/// Measured 7.0 %: grids he locked that Dub would warn drift.
const MAX_LOCKED_GRIDS_WARNED: f64 = 0.09;
/// Measured 92.3 %: the same grids, made to drift, that Dub warns of.
const MIN_DRIFT_WARNED: f64 = 0.89;
/// Measured 4.7 %: Dub's own grids that Dub warns of.
const MAX_OWN_GRIDS_WARNED: f64 = 0.06;
/// The drift put on a locked grid to see it caught, ms per minute: half
/// as much again as the warning's limit.
const DRIFT_PUT_ON_MS_PER_MIN: f64 = 12.0;

struct Song {
    path: PathBuf,
    profile: OctaveProfile,
    bpm: f64,
    anchor_secs: f64,
}

enum Outcome {
    Unreadable,
    NoGrid,
    Read {
        bpm: f64,
        anchor_secs: f64,
        duration_secs: f64,
        /// `(bpm, first beat)` of the grid a rough tap gave.
        tapped: Option<(f64, f64)>,
        /// Whether Dub warns that its own grid drifts.
        own_grid_warned: bool,
        /// Whether Dub warns of Traktor's grid as locked, and of the same
        /// grid [`DRIFT_PUT_ON_MS_PER_MIN`] slow. `None` where the song
        /// has too little beat to measure either.
        locked_grid_warned: Option<(bool, bool)>,
    },
}

/// How far out the tapped tempo is, either way.
const TAP_TEMPO_OUT: f64 = 0.08;
/// How late the one tap falls after a beat.
const TAP_LATE_SECS: f64 = 0.035;

#[test]
fn the_analyser_agrees_with_grids_locked_in_traktor() {
    let Some(manifest) = std::env::var_os(MANIFEST_ENV) else {
        eprintln!(
            "{MANIFEST_ENV} not set; skipping the Traktor grid corpus. \
             See crates/dub-bpm/tests/traktor_grid_corpus.rs for how to write one."
        );
        return;
    };
    let songs = read_manifest(&PathBuf::from(manifest));
    assert!(!songs.is_empty(), "the manifest names no songs");

    let outcomes = analyse_all(&songs);

    let mut by_ratio: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut by_family: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (mut scored, mut same_tempo) = (0usize, 0usize);
    let (mut placed, mut on_the_beat, mut half_a_beat_off) = (0usize, 0usize, 0usize);
    let mut offsets_ms: Vec<f64> = Vec::new();
    let (mut tapped, mut tap_tempo, mut tap_on_the_beat) = (0usize, 0usize, 0usize);
    let (mut own_warned, mut measured, mut locked_warned, mut drift_warned) =
        (0usize, 0usize, 0usize, 0usize);

    for (song, outcome) in songs.iter().zip(&outcomes) {
        let (bpm, anchor_secs, duration_secs) = match outcome {
            Outcome::Unreadable => continue,
            Outcome::NoGrid => {
                scored += 1;
                *by_ratio.entry("no grid").or_default() += 1;
                by_family
                    .entry(format!("{:?}", song.profile))
                    .or_default()
                    .1 += 1;
                continue;
            }
            Outcome::Read {
                bpm,
                anchor_secs,
                duration_secs,
                tapped: tap,
                own_grid_warned,
                locked_grid_warned,
            } => {
                own_warned += usize::from(*own_grid_warned);
                if let Some((as_locked, drifting)) = *locked_grid_warned {
                    measured += 1;
                    locked_warned += usize::from(as_locked);
                    drift_warned += usize::from(drifting);
                }
                tapped += 1;
                if let Some((tap_bpm, tap_anchor)) = *tap {
                    if (tap_bpm - song.bpm).abs() <= SAME_GRID_BPM {
                        tap_tempo += 1;
                        let ms = offset_ms(tap_bpm, tap_anchor, song, duration_secs / 2.0);
                        if ms.abs() <= ON_THE_BEAT_MS {
                            tap_on_the_beat += 1;
                        }
                    }
                }
                (*bpm, *anchor_secs, *duration_secs)
            }
        };
        scored += 1;
        let ratio = ratio_name(bpm, song.bpm);
        *by_ratio.entry(ratio).or_default() += 1;
        let family = by_family.entry(format!("{:?}", song.profile)).or_default();
        family.1 += 1;
        if ratio != "same" {
            continue;
        }
        same_tempo += 1;
        family.0 += 1;

        if (bpm - song.bpm).abs() > SAME_GRID_BPM {
            continue;
        }
        placed += 1;
        let ms = offset_ms(bpm, anchor_secs, song, duration_secs / 2.0);
        offsets_ms.push(ms.abs());
        if ms.abs() <= ON_THE_BEAT_MS {
            on_the_beat += 1;
        }
        if ms.abs() >= 0.4 * 60_000.0 / song.bpm {
            half_a_beat_off += 1;
        }
    }

    let share = |n: usize, of: usize| 100.0 * n as f64 / of.max(1) as f64;
    eprintln!("\n{} songs in the manifest, {scored} analysed", songs.len());
    eprintln!("tempo vs Traktor:");
    let mut ratios: Vec<_> = by_ratio.into_iter().collect();
    ratios.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    for (ratio, n) in ratios {
        eprintln!("  {ratio:8} {n:5}  {:5.1} %", share(n, scored));
    }
    eprintln!("by genre family:");
    for (family, (same, all)) in &by_family {
        eprintln!(
            "  {family:12} {same:5} of {all:5}  {:5.1} %",
            share(*same, *all)
        );
    }
    offsets_ms.sort_by(f64::total_cmp);
    let pick = |q: f64| {
        offsets_ms
            .get(((offsets_ms.len() as f64 * q) as usize).min(offsets_ms.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0)
    };
    eprintln!(
        "placement, {placed} songs within {SAME_GRID_BPM} BPM of Traktor: \
         {:.1} % within {ON_THE_BEAT_MS} ms (median {:.1} ms, 90th percentile {:.1} ms), \
         {half_a_beat_off} half a beat off",
        share(on_the_beat, placed),
        pick(0.5),
        pick(0.9),
    );

    eprintln!(
        "rough taps ({:.0} % out on the tempo, {:.0} ms late), {tapped} songs: \
         {:.1} % come back at Traktor's tempo within {SAME_GRID_BPM} BPM, \
         {:.1} % with their lines within {ON_THE_BEAT_MS} ms too",
        TAP_TEMPO_OUT * 100.0,
        TAP_LATE_SECS * 1000.0,
        share(tap_tempo, tapped),
        share(tap_on_the_beat, tapped),
    );

    eprintln!(
        "drift warning: {:.1} % of {measured} locked grids warned, {:.1} % once made to drift \
         {DRIFT_PUT_ON_MS_PER_MIN} ms/min; {:.1} % of Dub's own {tapped} grids warned",
        share(locked_warned, measured),
        share(drift_warned, measured),
        share(own_warned, tapped),
    );

    assert!(scored > 0, "no song in the manifest could be read");
    let tempo = same_tempo as f64 / scored as f64;
    assert!(
        tempo >= MIN_TEMPO_AGREEMENT,
        "tempo agrees with Traktor on {:.1} % of {scored} songs; the floor is {:.0} %",
        tempo * 100.0,
        MIN_TEMPO_AGREEMENT * 100.0
    );
    let beat = on_the_beat as f64 / placed.max(1) as f64;
    assert!(
        beat >= MIN_ON_THE_BEAT,
        "lines sit within {ON_THE_BEAT_MS} ms of Traktor's on {:.1} % of {placed} songs; \
         the floor is {:.0} %",
        beat * 100.0,
        MIN_ON_THE_BEAT * 100.0
    );
    let taps_tempo = tap_tempo as f64 / tapped.max(1) as f64;
    assert!(
        taps_tempo >= MIN_TAPS_FIND_THE_TEMPO,
        "rough taps come back at Traktor's tempo on {:.1} % of {tapped} songs; \
         the floor is {:.0} %",
        taps_tempo * 100.0,
        MIN_TAPS_FIND_THE_TEMPO * 100.0
    );
    let taps_beat = tap_on_the_beat as f64 / tapped.max(1) as f64;
    assert!(
        taps_beat >= MIN_TAPS_FIND_THE_BEAT,
        "rough taps come back on Traktor's lines on {:.1} % of {tapped} songs; \
         the floor is {:.0} %",
        taps_beat * 100.0,
        MIN_TAPS_FIND_THE_BEAT * 100.0
    );
    let locked = locked_warned as f64 / measured.max(1) as f64;
    assert!(
        locked <= MAX_LOCKED_GRIDS_WARNED,
        "Dub warns of drift on {:.1} % of {measured} grids locked in Traktor; \
         the ceiling is {:.0} %",
        locked * 100.0,
        MAX_LOCKED_GRIDS_WARNED * 100.0
    );
    let drifting = drift_warned as f64 / measured.max(1) as f64;
    assert!(
        drifting >= MIN_DRIFT_WARNED,
        "Dub warns of {:.1} % of {measured} grids drifting {DRIFT_PUT_ON_MS_PER_MIN} ms/min; \
         the floor is {:.0} %",
        drifting * 100.0,
        MIN_DRIFT_WARNED * 100.0
    );
    let own = own_warned as f64 / tapped.max(1) as f64;
    assert!(
        own <= MAX_OWN_GRIDS_WARNED,
        "Dub warns of drift on {:.1} % of its own {tapped} grids; the ceiling is {:.0} %",
        own * 100.0,
        MAX_OWN_GRIDS_WARNED * 100.0
    );
}

/// `path<TAB>genre<TAB>bpm<TAB>anchor_secs`, no header.
fn read_manifest(path: &PathBuf) -> Vec<Song> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read manifest {}: {e}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .enumerate()
        .map(|(n, line)| {
            let cols: Vec<&str> = line.split('\t').collect();
            let number = |i: usize| -> f64 {
                cols.get(i)
                    .and_then(|c| c.trim().parse().ok())
                    .unwrap_or_else(|| panic!("line {}: column {} is not a number", n + 1, i + 1))
            };
            Song {
                path: PathBuf::from(cols[0]),
                profile: octave_profile_from_genre(cols.get(1).copied().unwrap_or("")),
                bpm: number(2),
                anchor_secs: number(3),
            }
        })
        .collect()
}

fn analyse_all(songs: &[Song]) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let outcomes: Mutex<Vec<Option<Outcome>>> = Mutex::new(songs.iter().map(|_| None).collect());
    let workers =
        std::thread::available_parallelism().map_or(4, |n| n.get().saturating_sub(2).max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(song) = songs.get(i) else {
                    break;
                };
                let outcome = analyse(song, i);
                outcomes.lock().expect("no worker panics holding this")[i] = Some(outcome);
            });
        }
    });
    outcomes
        .into_inner()
        .expect("no worker panics holding this")
        .into_iter()
        .map(|o| o.unwrap_or(Outcome::Unreadable))
        .collect()
}

fn analyse(song: &Song, index: usize) -> Outcome {
    let Ok(track) = Track::load_from_path(&song.path) else {
        return Outcome::Unreadable;
    };
    let (samples, rate, channels) = (track.samples(), track.sample_rate(), track.channels());
    let duration_secs = samples.len() as f64 / (f64::from(rate) * f64::from(channels.max(1)));
    match analyze_beat_grid_with_profile(samples, rate, channels, song.profile) {
        Ok(grid) if grid.confidence > 0.0 && !grid.beats.is_empty() => Outcome::Read {
            bpm: grid.bpm,
            anchor_secs: grid.beats[0],
            duration_secs,
            tapped: rough_tap(song, index, samples, rate, channels, duration_secs),
            own_grid_warned: grid.quality.is_some_and(|quality| quality.drifts()),
            locked_grid_warned: locked_grid_warned(song, samples, rate, channels, duration_secs),
        },
        _ => Outcome::NoGrid,
    }
}

/// The grid a DJ's rough tap gives: a tempo [`TAP_TEMPO_OUT`] under
/// (even songs) or over (odd), one tap [`TAP_LATE_SECS`] after Traktor's
/// beat in the middle of the song.
fn rough_tap(
    song: &Song,
    index: usize,
    samples: &[f32],
    rate: u32,
    channels: u8,
    duration_secs: f64,
) -> Option<(f64, f64)> {
    let out = if index.is_multiple_of(2) {
        1.0 - TAP_TEMPO_OUT
    } else {
        1.0 + TAP_TEMPO_OUT
    };
    let hint = (song.bpm * out).clamp(dub_bpm::MIN_BPM, dub_bpm::MAX_BPM);
    let period = 60.0 / song.bpm;
    let beat =
        song.anchor_secs + ((duration_secs / 2.0 - song.anchor_secs) / period).round() * period;
    let grid =
        analyze_beat_grid_from_bpm_and_anchor(samples, rate, channels, hint, beat + TAP_LATE_SECS)
            .ok()?;
    (!grid.beats.is_empty()).then(|| (grid.bpm, grid.beats[0]))
}

/// Whether Dub warns of drift on Traktor's grid as locked, and on the
/// same grid with its lines falling [`DRIFT_PUT_ON_MS_PER_MIN`] later
/// each minute. Both are measured as a set-the-1 on a playing deck
/// measures a grid: at the given tempo, from a beat in the middle.
fn locked_grid_warned(
    song: &Song,
    samples: &[f32],
    rate: u32,
    channels: u8,
    duration_secs: f64,
) -> Option<(bool, bool)> {
    let period = 60.0 / song.bpm;
    let beat =
        song.anchor_secs + ((duration_secs / 2.0 - song.anchor_secs) / period).round() * period;
    let warned = |bpm: f64| -> Option<bool> {
        let grid = fit_grid_at_playing_tap(samples, rate, channels, bpm, beat).ok()?;
        Some(grid.quality?.drifts())
    };
    Some((
        warned(song.bpm)?,
        warned(song.bpm / (1.0 + DRIFT_PUT_ON_MS_PER_MIN / 60_000.0))?,
    ))
}

/// What `bpm` is to the reference: the same tempo, or which wrong one.
fn ratio_name(bpm: f64, reference: f64) -> &'static str {
    const RATIOS: [(&str, f64); 7] = [
        ("same", 1.0),
        ("half", 0.5),
        ("double", 2.0),
        ("2:3", 2.0 / 3.0),
        ("3:2", 1.5),
        ("3:4", 0.75),
        ("4:3", 4.0 / 3.0),
    ];
    RATIOS
        .iter()
        .find(|(_, ratio)| (bpm / (reference * ratio) - 1.0).abs() <= SAME_TEMPO)
        .map_or("other", |(name, _)| name)
}

/// How far (ms, signed) the analysed line nearest Traktor's beat at
/// `at_secs` sits from it.
fn offset_ms(bpm: f64, anchor_secs: f64, song: &Song, at_secs: f64) -> f64 {
    let theirs = 60.0 / song.bpm;
    let their_beat = song.anchor_secs + ((at_secs - song.anchor_secs) / theirs).round() * theirs;
    let ours = 60.0 / bpm;
    let our_beat = anchor_secs + ((their_beat - anchor_secs) / ours).round() * ours;
    (our_beat - their_beat) * 1000.0
}
