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
