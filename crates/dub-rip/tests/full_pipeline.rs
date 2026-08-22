//! End-to-end M26a pipeline: synthetic "side" pushed through the
//! record-tap ring → armed session → spill + envelope → manual
//! splits → commit → tagged FLACs imported, pre-analyzed, archived,
//! spill removed, retry idempotent.

use std::time::Duration;

use ringbuf::traits::{Producer, Split};
use ringbuf::HeapRb;

use dub_rip::{RipConfig, RipSession, RipState, StopReason, TrackMeta};

const SR: u32 = 44_100;
/// Three ~8 s "tracks" — long enough to satisfy MIN_SEGMENT_SECS,
/// short enough that analysis (chromaprint + grid + key + LUFS per
/// segment) keeps the test in single-digit seconds.
const SEG_SECS: u64 = 8;

fn seg_frames() -> u64 {
    SEG_SECS * u64::from(SR)
}

/// Interleaved stereo side: three tones at distinct frequencies and
/// levels. No gaps — splits are manual in M26a.
fn synthetic_side() -> Vec<f32> {
    let frames = usize::try_from(3 * seg_frames()).unwrap();
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let seg = i / usize::try_from(seg_frames()).unwrap();
        let (freq, amp) = match seg {
            0 => (220.0_f32, 0.7_f32),
            1 => (440.0, 0.5),
            _ => (880.0, 0.3),
        };
        #[allow(clippy::cast_precision_loss)]
        let t = i as f32 / SR as f32;
        let s = amp * (std::f32::consts::TAU * freq * t).sin();
        out.push(s);
        out.push(s);
    }
    out
}

#[test]
fn record_split_commit_full_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("rip-session");
    let mut cfg = RipConfig::new(SR, session_dir.clone());
    cfg.poll_interval = Duration::from_millis(1);
    let mut session = RipSession::new(cfg).unwrap();

    // ---- Capture ------------------------------------------------
    let ring = HeapRb::<f32>::new(1 << 20);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    session.start().unwrap();

    let side = synthetic_side();
    let mut pushed = 0;
    while pushed < side.len() {
        pushed += tx.push_slice(&side[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    // Give the worker one cycle to drain the tail, then stop.
    std::thread::sleep(Duration::from_millis(5));
    session.stop().unwrap();
    let status = session.wait_stopped(Duration::from_secs(10)).unwrap();

    assert_eq!(status.state, RipState::Stopped(StopReason::Manual));
    assert_eq!(status.recorded_frames, 3 * seg_frames());
    let expected_chunks = usize::try_from(3 * seg_frames()).unwrap() / 64;
    assert_eq!(session.envelope_len(), expected_chunks);
    // The envelope carries the level staircase (0.7 / 0.5 / 0.3
    // amplitude tones), which is what the M26b gap detector will
    // read. Spot-check segment RMS ordering.
    let envelope = session.envelope_from(0);
    let seg_chunks = expected_chunks / 3;
    let mean_rms = |range: std::ops::Range<usize>| {
        let n = range.len() as f32;
        envelope[range].iter().map(|c| c.rms).sum::<f32>() / n
    };
    let (a, b, c) = (
        mean_rms(0..seg_chunks),
        mean_rms(seg_chunks..2 * seg_chunks),
        mean_rms(2 * seg_chunks..3 * seg_chunks),
    );
    assert!(
        a > b && b > c,
        "envelope must track the level staircase: {a} {b} {c}"
    );
    assert!(session.spill_path().exists());

    // ---- Split + metadata ----------------------------------------
    session
        .set_splits(vec![seg_frames(), 2 * seg_frames()])
        .unwrap();
    session
        .set_track_meta(
            0,
            TrackMeta {
                title: Some("Real Rock".into()),
                artist: Some("Sound Dimension".into()),
                album: Some("Studio One Side A".into()),
                year: Some(1967),
                genre: Some("Reggae".into()),
            },
        )
        .unwrap();
    session
        .set_track_meta(
            1,
            TrackMeta {
                title: Some("Version".into()),
                ..TrackMeta::default()
            },
        )
        .unwrap();

    // ---- Commit ---------------------------------------------------
    let mut library = dub_library::Library::open_at(&dir.path().join("library.sqlite")).unwrap();
    let outcome = session.commit(&mut library).unwrap();

    assert_eq!(outcome.segments.len(), 3);
    for seg in &outcome.segments {
        assert!(
            seg.library_uuid.is_some(),
            "segment {} failed: {:?}",
            seg.index,
            seg.error
        );
        assert!(seg.file.exists(), "encoded file missing: {:?}", seg.file);
    }
    assert!(outcome.is_complete());
    assert!(outcome.spill_removed);
    assert!(!session.spill_path().exists());
    let archive = outcome.archive.clone().unwrap();
    assert!(archive.exists());

    // Filenames derive from metadata.
    assert!(outcome.segments[0]
        .file
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("01 Sound Dimension - Real Rock"));

    // The encoded segment decodes to exactly the segment length.
    let track = dub_io::Track::load_from_path(&outcome.segments[0].file).expect("segment decodes");
    assert_eq!(track.frames() as u64, seg_frames());

    // Manifest carries the committed uuids (crash-safe resume state).
    let manifest = session.manifest();
    assert!(manifest
        .tracks
        .iter()
        .all(|t| t.library_uuid.is_some() && t.encoded_file.is_some()));
    assert_eq!(manifest.side_archive.as_deref(), Some("side.flac"));

    // ---- Retry is idempotent --------------------------------------
    let uuids: Vec<String> = outcome
        .segments
        .iter()
        .map(|s| s.library_uuid.clone().unwrap())
        .collect();
    let again = session.commit(&mut library).unwrap();
    let uuids_again: Vec<String> = again
        .segments
        .iter()
        .map(|s| s.library_uuid.clone().unwrap())
        .collect();
    assert_eq!(uuids, uuids_again, "retry must not re-import");
}

#[test]
fn producer_drop_fail_safe_stops_with_captured_audio() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RipConfig::new(SR, dir.path().join("s"));
    cfg.poll_interval = Duration::from_millis(1);
    let mut session = RipSession::new(cfg).unwrap();

    let ring = HeapRb::<f32>::new(1 << 16);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    session.start().unwrap();

    let tone: Vec<f32> = vec![0.5; 44_100 * 2];
    let mut pushed = 0;
    while pushed < tone.len() {
        pushed += tx.push_slice(&tone[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(tx); // engine detached mid-rip

    let status = session.wait_stopped(Duration::from_secs(10)).unwrap();
    assert_eq!(status.state, RipState::Stopped(StopReason::InputLost));
    assert_eq!(status.recorded_frames, 44_100);
    assert!(session.spill_path().exists(), "salvage WAV must survive");
}

#[test]
fn splits_rejected_while_recording_and_when_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RipConfig::new(SR, dir.path().join("s"));
    cfg.poll_interval = Duration::from_millis(1);
    let mut session = RipSession::new(cfg).unwrap();

    let ring = HeapRb::<f32>::new(1 << 16);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    session.start().unwrap();
    assert!(
        session.set_splits(vec![1_000]).is_err(),
        "splits must be rejected while recording"
    );

    let tone: Vec<f32> = vec![0.4; 44_100 * 2 * 12];
    let mut pushed = 0;
    while pushed < tone.len() {
        pushed += tx.push_slice(&tone[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(5));
    session.stop().unwrap();
    session.wait_stopped(Duration::from_secs(10)).unwrap();

    // 12 s recording: a 1 s head segment violates MIN_SEGMENT_SECS.
    assert!(session.set_splits(vec![u64::from(SR)]).is_err());
    // A clean 6/6 split is fine.
    session.set_splits(vec![6 * u64::from(SR)]).unwrap();
}

/// M26b: the same capture path, but the side carries real inter-track
/// silence — the detector must find it through the worker-built
/// envelope, not a synthetic one.
#[test]
fn auto_split_finds_the_gaps_in_a_side() {
    use dub_rip::GapConfig;

    const TRACK_SECS: u64 = 6;
    const GAP_SECS: f64 = 1.5;

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RipConfig::new(SR, dir.path().join("s"));
    cfg.poll_interval = Duration::from_millis(1);
    let mut session = RipSession::new(cfg).unwrap();

    let ring = HeapRb::<f32>::new(1 << 20);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    session.start().unwrap();
    assert!(
        session.auto_split(&GapConfig::default()).is_err(),
        "auto split must be rejected while recording"
    );

    // Three tones separated by groove noise 54 dB down. The noise is
    // a deterministic ±0.002 alternation so the RMS is exact.
    let mut side: Vec<f32> = Vec::new();
    let push_tone = |side: &mut Vec<f32>, secs: u64| {
        for i in 0..secs * u64::from(SR) {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / SR as f32;
            let s = 0.5 * (std::f32::consts::TAU * 330.0 * t).sin();
            side.push(s);
            side.push(s);
        }
    };
    let push_noise = |side: &mut Vec<f32>| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = (GAP_SECS * f64::from(SR)) as u64;
        for i in 0..frames {
            let s = if i % 2 == 0 { 0.002 } else { -0.002 };
            side.push(s);
            side.push(s);
        }
    };
    push_tone(&mut side, TRACK_SECS);
    push_noise(&mut side);
    push_tone(&mut side, TRACK_SECS);
    push_noise(&mut side);
    push_tone(&mut side, TRACK_SECS);

    let mut pushed = 0;
    while pushed < side.len() {
        pushed += tx.push_slice(&side[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(5));
    session.stop().unwrap();
    session.wait_stopped(Duration::from_secs(10)).unwrap();

    // Short tracks, so the production 30 s minimum has to come down.
    let gap_cfg = GapConfig {
        min_track_secs: 5.0,
        min_gap_secs: 1.0,
        ..GapConfig::default()
    };

    let gaps = session.detect_gaps(&gap_cfg);
    assert_eq!(gaps.len(), 2, "expected two gaps: {gaps:?}");

    let secs = |frame: u64| frame as f64 / f64::from(SR);
    for (i, gap) in gaps.iter().enumerate() {
        let expect_start = (i as f64 + 1.0) * f64::from(TRACK_SECS as u32) + i as f64 * GAP_SECS;
        assert!(
            (secs(gap.start_frame) - expect_start).abs() < 0.2,
            "gap {i} starts at {} s, expected ~{expect_start}",
            secs(gap.start_frame)
        );
        assert!(
            (secs(gap.end_frame) - (expect_start + GAP_SECS)).abs() < 0.2,
            "gap {i} ends at {} s",
            secs(gap.end_frame)
        );
        // Split sits inside the silence, a pre-roll before the music.
        assert!(gap.boundary_frame > gap.start_frame && gap.boundary_frame < gap.end_frame);
    }

    assert_eq!(session.auto_split(&gap_cfg).unwrap(), 3);
    assert_eq!(
        session.manifest().boundaries_frames,
        gaps.iter().map(|g| g.boundary_frame).collect::<Vec<_>>(),
        "auto split must install exactly the detected boundaries"
    );
    assert_eq!(session.manifest().tracks.len(), 3);
}

/// M26b: hands-off capture. Arm, drop the needle, walk away — the
/// worker starts on the first sound and stops itself in the run-out.
#[test]
fn auto_start_fires_on_the_needle_drop_and_keeps_the_pre_roll() {
    use dub_rip::AutoCapture;

    const PRE_ROLL_SECS: f32 = 0.5;

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RipConfig::new(SR, dir.path().join("s"));
    cfg.poll_interval = Duration::from_millis(1);
    cfg.auto = AutoCapture {
        start_threshold: Some(0.05),
        pre_roll_secs: PRE_ROLL_SECS,
        silence_stop_secs: None,
        silence_drop_db: 25.0,
    };
    let mut session = RipSession::new(cfg).unwrap();

    let ring = HeapRb::<f32>::new(1 << 20);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    // No start() call anywhere in this test — the needle does it.

    // 2 s of lead-in groove noise, well under the trigger.
    let quiet: Vec<f32> = (0..2 * u64::from(SR))
        .flat_map(|i| {
            let s = if i % 2 == 0 { 0.002 } else { -0.002 };
            [s, s]
        })
        .collect();
    let mut pushed = 0;
    while pushed < quiet.len() {
        pushed += tx.push_slice(&quiet[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(
        session.status().state,
        RipState::Armed,
        "groove noise must not trip the trigger"
    );
    assert_eq!(session.status().recorded_frames, 0);

    // The music arrives.
    let tone: Vec<f32> = (0..3 * u64::from(SR))
        .flat_map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / SR as f32;
            let s = 0.5 * (std::f32::consts::TAU * 220.0 * t).sin();
            [s, s]
        })
        .collect();
    let mut pushed = 0;
    while pushed < tone.len() {
        pushed += tx.push_slice(&tone[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(10));
    session.stop().unwrap();
    let status = session.wait_stopped(Duration::from_secs(10)).unwrap();

    assert_eq!(status.state, RipState::Stopped(StopReason::Manual));
    // The tone, plus the pre-roll that was already in flight when the
    // trigger fired. Without the pre-roll ring the needle drop and the
    // first transient would be gone.
    let tone_frames = 3 * u64::from(SR);
    let pre_roll_frames = (PRE_ROLL_SECS * SR as f32) as u64;
    assert!(
        status.recorded_frames > tone_frames,
        "no pre-roll: recorded {} frames for a {tone_frames}-frame tone",
        status.recorded_frames
    );
    assert!(
        status.recorded_frames <= tone_frames + pre_roll_frames + u64::from(SR) / 2,
        "pre-roll ran long: {} frames",
        status.recorded_frames
    );
}

#[test]
fn silence_auto_stop_fires_in_the_run_out_but_not_between_tracks() {
    use dub_rip::AutoCapture;

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RipConfig::new(SR, dir.path().join("s"));
    cfg.poll_interval = Duration::from_millis(1);
    cfg.auto = AutoCapture {
        start_threshold: None,
        pre_roll_secs: 0.0,
        // Short enough to keep the test quick; long enough that the
        // 1.5 s inter-track gap below must not trip it.
        silence_stop_secs: Some(3.0),
        silence_drop_db: 25.0,
    };
    let mut session = RipSession::new(cfg).unwrap();

    let ring = HeapRb::<f32>::new(1 << 22);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    session.start().unwrap();

    let tone = |secs: f64| -> Vec<f32> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = (secs * f64::from(SR)) as u64;
        (0..frames)
            .flat_map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / SR as f32;
                let s = 0.5 * (std::f32::consts::TAU * 220.0 * t).sin();
                [s, s]
            })
            .collect()
    };
    let groove = |secs: f64| -> Vec<f32> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = (secs * f64::from(SR)) as u64;
        (0..frames)
            .flat_map(|i| {
                let s = if i % 2 == 0 { 0.002 } else { -0.002 };
                [s, s]
            })
            .collect()
    };

    // Track, inter-track gap, track — the gap must not stop us.
    let mut side = tone(4.0);
    side.extend(groove(1.5));
    side.extend(tone(4.0));
    let mut pushed = 0;
    while pushed < side.len() {
        pushed += tx.push_slice(&side[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(
        session.status().state,
        RipState::Recording,
        "a 1.5 s gap must not end the side"
    );

    // Now the run-out: groove noise past the timeout.
    let run_out = groove(5.0);
    let mut pushed = 0;
    while pushed < run_out.len() {
        pushed += tx.push_slice(&run_out[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }

    let status = session.wait_stopped(Duration::from_secs(10)).unwrap();
    assert_eq!(status.state, RipState::Stopped(StopReason::Silence));
    // Everything up to the auto-stop is kept — including the run-out
    // that triggered it; trimming is the split plan's job.
    assert!(status.recorded_frames >= 9 * u64::from(SR));
}

/// M26b: the crash case. A session dies mid-side — no stop, no
/// finalize, `rip.json` still claiming zero frames — and comes back
/// with its audio, its envelope, and a working commit path.
#[test]
fn interrupted_session_recovers_from_disk_and_commits() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("rip-session");
    let mut cfg = RipConfig::new(SR, session_dir.clone());
    cfg.poll_interval = Duration::from_millis(1);
    let mut session = RipSession::new(cfg).unwrap();

    let ring = HeapRb::<f32>::new(1 << 20);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    session.start().unwrap();
    let side = synthetic_side();
    let mut pushed = 0;
    while pushed < side.len() {
        pushed += tx.push_slice(&side[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(10));
    session.stop().unwrap();
    session.wait_stopped(Duration::from_secs(10)).unwrap();
    let recorded = session.manifest().recorded_frames;
    drop(session);

    // Simulate the crash: the manifest never learned the length (it is
    // only synced at wait_stopped) and the WAV header was never
    // rewritten, because nothing got to finalize it.
    let manifest_path = session_dir.join("rip.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["recorded_frames"] = serde_json::json!(0);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let spill = session_dir.join("side.raw.wav");
    let mut bytes = std::fs::read(&spill).unwrap();
    bytes[4..8].copy_from_slice(&0_u32.to_le_bytes());
    let data_at = bytes.windows(4).position(|w| w == b"data").unwrap();
    bytes[data_at + 4..data_at + 8].copy_from_slice(&0_u32.to_le_bytes());
    std::fs::write(&spill, &bytes).unwrap();

    // ---- The next launch finds it ---------------------------------
    let found = dub_rip::list_recoverable(dir.path());
    assert_eq!(found.len(), 1, "unfinished rip must be offered: {found:?}");
    assert_eq!(found[0].session_dir, session_dir);
    assert!(found[0].was_interrupted, "header was never finalized");
    assert_eq!(found[0].frames, recorded);

    // ---- and reopens it -------------------------------------------
    let mut session = RipSession::from_session_dir(session_dir.clone()).unwrap();
    let status = session.status();
    assert_eq!(status.state, RipState::Stopped(StopReason::Recovered));
    assert_eq!(status.recorded_frames, recorded);
    assert_eq!(
        session.envelope_len(),
        usize::try_from(recorded).unwrap() / 64,
        "envelope must be rebuilt from the spill"
    );

    // The plan still works, and so does the commit.
    session
        .set_splits(vec![seg_frames(), 2 * seg_frames()])
        .unwrap();
    let mut library = dub_library::Library::open_at(&dir.path().join("library.sqlite")).unwrap();
    let outcome = session.commit(&mut library).unwrap();
    assert!(
        outcome.is_complete(),
        "recovered rip must commit: {outcome:?}"
    );
    assert_eq!(outcome.segments.len(), 3);

    // Committed: the spill is gone, so it is no longer on offer.
    assert!(dub_rip::list_recoverable(dir.path()).is_empty());
}

/// M26b / R-39: the commit reports each segment as it goes. Without
/// this the UI can only flip every dot when the whole pass returns,
/// which on a six-track side is minutes of apparent stall.
#[test]
fn commit_reports_progress_per_segment() {
    use dub_rip::CommitProgress;

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RipConfig::new(SR, dir.path().join("s"));
    cfg.poll_interval = Duration::from_millis(1);
    let mut session = RipSession::new(cfg).unwrap();

    let ring = HeapRb::<f32>::new(1 << 20);
    let (mut tx, rx) = ring.split();
    session.arm(rx).unwrap();
    session.start().unwrap();
    let side = synthetic_side();
    let mut pushed = 0;
    while pushed < side.len() {
        pushed += tx.push_slice(&side[pushed..]);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(10));
    session.stop().unwrap();
    session.wait_stopped(Duration::from_secs(10)).unwrap();
    session
        .set_splits(vec![seg_frames(), 2 * seg_frames()])
        .unwrap();

    let mut library = dub_library::Library::open_at(&dir.path().join("library.sqlite")).unwrap();
    let mut events = Vec::new();
    let outcome = session
        .commit_with_progress(&mut library, &mut |event| events.push(event))
        .unwrap();
    assert!(outcome.is_complete());

    // Every segment starts and finishes, in order, before the archive.
    let expected: Vec<CommitProgress> = (0..3)
        .flat_map(|index| {
            [
                CommitProgress::Started { index },
                CommitProgress::Finished {
                    index,
                    imported: true,
                },
            ]
        })
        .chain(std::iter::once(CommitProgress::ArchiveStarted))
        .collect();
    assert_eq!(events, expected, "progress stream out of order");

    // Retrying a fully committed session short-circuits before the
    // segment loop (the spill is gone by then), so it reports nothing
    // at all rather than replaying a pass that isn't happening.
    let mut retry_events = Vec::new();
    let retry = session
        .commit_with_progress(&mut library, &mut |event| retry_events.push(event))
        .unwrap();
    assert!(
        retry_events.is_empty(),
        "a no-op retry must not fake progress: {retry_events:?}"
    );
    assert_eq!(retry.segments.len(), 3, "retry still reports the segments");
}
