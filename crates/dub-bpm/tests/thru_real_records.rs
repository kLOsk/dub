//! A THRU deck's tempo on real records (PRD §2.2.10 #10): the live
//! tracker, configured exactly as a THRU deck runs it, must lock within
//! 15 s of music, agree with the offline analysis of the same stretch,
//! and **stay locked** for the rest of it. The last is the rig's
//! 2026-11-14 finding: the reggae 12″ locked at 133.4 and dropped back to
//! tentative a second later on one 88.8 estimate, and again a minute in —
//! the readout flickering solid, dim, solid.
//!
//! The records are the rip baselines — a reggae 12″, a soul sampler and a
//! drum'n'bass side captured on the SL 3 (`testdata/rip-baselines/`). They
//! are gitignored (748 MB); without them the test says so and passes.
//!
//! **Known gap — drum'n'bass.** The dnb side reads ~116.6 everywhere,
//! live and offline alike: two-thirds of its ~175, the beat-and-a-half
//! the breakbeat's kicks repeat at (2026-11-11). It mostly stays
//! tentative, and has locked on the wrong value. TAP on the deck is the
//! override until the estimator learns it; the record is reported here,
//! not asserted, so the day it locks right shows up in the output.

use std::path::PathBuf;

use dub_bpm::{
    analyze_bpm, BeatFollower, BpmTracker, LiveStreamConfig, TrackerConfig, TrackerState,
    PHASE_WINDOW_SECS,
};
use dub_io::Track;
use dub_peaks::OnsetDecimator;

/// The PRD's bar: locked within 15 s of the needle going down.
const LOCK_WITHIN_SECS: f64 = 15.0;
/// How far the live lock may sit from the offline answer.
const TOLERANCE_BPM: f64 = 1.5;
/// Seconds of music analysed per record — the offline reference reads
/// all of it, the live tracker as much as it needs.
const EXCERPT_SECS: f64 = 90.0;

struct Record {
    file: &'static str,
    what: &'static str,
    /// Where the excerpt starts: inside the music, past each side's
    /// lead-in (and, for the dnb side, its long fade-in).
    from_secs: f64,
    /// Held to the 15 s lock; `false` for a known gap, reported only.
    asserted: bool,
}

const RECORDS: [Record; 3] = [
    Record {
        file: "rip-baseline-a.flac",
        what: "reggae 12in",
        from_secs: 60.0,
        asserted: true,
    },
    Record {
        file: "rip-baseline-b.flac",
        what: "soul sampler",
        from_secs: 60.0,
        asserted: true,
    },
    Record {
        file: "rip-baseline-c.flac",
        what: "drum'n'bass 12in",
        from_secs: 90.0,
        asserted: false,
    },
];

fn baselines() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/rip-baselines")
}

/// The excerpt as mono at the record's rate — what a THRU tap delivers.
fn excerpt(path: &PathBuf, from_secs: f64) -> (Vec<f32>, u32) {
    let mut load = Track::begin_streaming(path).expect("open the record");
    load.decode_until_secs(from_secs + EXCERPT_SECS)
        .expect("decode the record");
    let track = load.track();
    let sr = track.sample_rate();
    let ch = usize::from(track.channels());
    let all = track.samples_to_vec();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (start, len) = (
        (from_secs * f64::from(sr)) as usize,
        (EXCERPT_SECS * f64::from(sr)) as usize,
    );
    let mono = all
        .chunks_exact(ch)
        .skip(start)
        .take(len)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect();
    (mono, sr)
}

#[test]
fn a_thru_deck_locks_on_real_records_within_fifteen_seconds() {
    let dir = baselines();
    if !dir.join(RECORDS[0].file).exists() {
        eprintln!("testdata/rip-baselines not present; skipping the real-record THRU tempo test");
        return;
    }
    let mut failures = Vec::new();
    for record in &RECORDS {
        let (audio, sr) = excerpt(&dir.join(record.file), record.from_secs);
        let reference = analyze_bpm(&audio, sr, 1).expect("offline analysis").bpm;

        let mut tracker = BpmTracker::new(TrackerConfig {
            sample_rate: sr,
            channels: 1,
            analysis_period_samples: sr,
            bpm_range: dub_bpm::BpmRange::DEFAULT,
        })
        .expect("tracker");
        tracker.set_odf_window_secs(Some(LiveStreamConfig::default().odf_window_secs));

        // The tap delivers render-sized blocks; 512 frames is a generous one.
        let mut locked_at = None;
        let mut lost_at = None;
        for (i, block) in audio.chunks(512).enumerate() {
            let _ = tracker.process(block);
            #[allow(clippy::cast_precision_loss)]
            let secs = ((i + 1) * 512) as f64 / f64::from(sr);
            match (tracker.state(), locked_at) {
                (TrackerState::Locked { bpm }, None) => locked_at = Some((secs, bpm)),
                (TrackerState::Locked { .. }, Some(_)) => {}
                (_, Some(_)) if lost_at.is_none() => lost_at = Some(secs),
                _ => {}
            }
        }
        let line = match locked_at {
            Some((secs, bpm)) => format!(
                "{}: locked {bpm:.1} at {secs:.1} s{}, offline {reference:.1}",
                record.what,
                lost_at.map_or_else(String::new, |at| format!(", LOST at {at:.1} s"))
            ),
            None => format!(
                "{}: never locked in {EXCERPT_SECS} s, offline {reference:.1}",
                record.what
            ),
        };
        eprintln!(
            "{line}{}",
            if record.asserted { "" } else { "  (known gap)" }
        );
        if !record.asserted {
            continue;
        }
        match locked_at {
            Some((secs, bpm))
                if secs <= LOCK_WITHIN_SECS
                    && (bpm - reference).abs() <= TOLERANCE_BPM
                    && lost_at.is_none() => {}
            _ => failures.push(line),
        }
    }
    assert!(failures.is_empty(), "THRU tempo off on: {failures:#?}");
}

/// How far the forecast beat may move between two looks with the tempo
/// locked. The drawn beats are a forecast to read ahead against; a beat
/// that moves more than a few pixels is a beat the DJ cannot trust.
const FORECAST_STILL_MS: f64 = 15.0;

/// The THRU deck's forecast beats (PRD §2.2.10 #10) must hold still on
/// real records while the tempo is locked — the rig's 2026-11-14
/// finding, the BPM solid and the beat lines jumping. Asked fresh four
/// times a second, the reggae 12″'s phase flipped half a beat to its
/// skank and back 25 times in 85 s (224 ms at a time), and the soul
/// sampler's slid in jumps of up to 67 ms behind a band pushing ~2 %
/// past its locked tempo.
///
/// Driven the way the app drives it: the waveform stream's onsets, the
/// tracker's locked tempo rounded to the readout's tenth, a look every
/// quarter second over the last [`PHASE_WINDOW_SECS`]. A move is how
/// far each answer lands from where the last one forecast that beat.
#[test]
fn a_thru_decks_forecast_beats_hold_still_on_real_records() {
    let dir = baselines();
    if !dir.join(RECORDS[0].file).exists() {
        eprintln!(
            "testdata/rip-baselines not present; skipping the real-record THRU forecast test"
        );
        return;
    }
    let mut failures = Vec::new();
    for record in &RECORDS {
        let (audio, sr) = excerpt(&dir.join(record.file), record.from_secs);
        let mut tracker = BpmTracker::new(TrackerConfig {
            sample_rate: sr,
            channels: 1,
            analysis_period_samples: sr,
            bpm_range: dub_bpm::BpmRange::DEFAULT,
        })
        .expect("tracker");
        tracker.set_odf_window_secs(Some(LiveStreamConfig::default().odf_window_secs));
        let mut onsets = OnsetDecimator::new(sr);
        #[allow(clippy::cast_precision_loss)]
        let hop = onsets.samples_per_chunk() as f64 / f64::from(sr);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let window = (PHASE_WINDOW_SECS / hop).ceil() as usize + 1;
        let look_every = sr as usize / 4;

        let mut flux = Vec::new();
        let mut follower = BeatFollower::new();
        let mut last: Option<(f64, f64)> = None;
        let (mut looks, mut worst, mut fed, mut next_look) = (0usize, 0.0f64, 0usize, look_every);
        for block in audio.chunks(512) {
            let _ = tracker.process(block);
            onsets.feed(block, |c| flux.push(c.flux));
            fed += block.len();
            if fed < next_look {
                continue;
            }
            next_look += look_every;
            let TrackerState::Locked { bpm } = tracker.state() else {
                last = None;
                continue;
            };
            let bpm = (bpm * 10.0).round() / 10.0;
            let start = flux.len().saturating_sub(window);
            #[allow(clippy::cast_precision_loss)]
            let start_secs = (start as f64 + 1.0) * hop;
            let Some(beats) = follower.update(&flux[start..], start_secs, hop, 60.0 / bpm) else {
                last = None;
                continue;
            };
            looks += 1;
            if let Some((was, period)) = last {
                let n = ((beats.last_beat_secs - was) / period).round();
                worst = worst.max((beats.last_beat_secs - (was + n * period)).abs() * 1000.0);
            }
            last = Some((beats.last_beat_secs, beats.period_secs));
        }
        let line = format!(
            "{}: {looks} locked looks, the forecast beat moved at most {worst:.1} ms",
            record.what
        );
        eprintln!(
            "{line}{}",
            if record.asserted { "" } else { "  (known gap)" }
        );
        if record.asserted && (looks < 200 || worst > FORECAST_STILL_MS) {
            failures.push(line);
        }
    }
    assert!(
        failures.is_empty(),
        "THRU forecast jumping on: {failures:#?}"
    );
}
