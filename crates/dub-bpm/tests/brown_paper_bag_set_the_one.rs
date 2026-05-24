//! Brown Paper Bag — "set the 1" regression (Roni Size / DnB).
//!
//! Runs against the author's local copy when present. Exercises the
//! full auto-analyze → user-tap relatch path that the deck-header BPM
//! column uses for 1–2 tap sessions.

use dub_bpm::{analyze_beat_grid, latch_beat_grid_at_downbeat, OctaveProfile};
use dub_io::Track;

const BROWN_PAPER_BAG: &str = "/Users/klos/Music/Music/DrumnBass/Roni Size:Reprazent - Brown Paper Bag (Crissy Criss Plastic Bag instrumental remix).mp3";

/// Scan the first `window_secs` for the strongest broadband peak —
/// proxy for "user parked playhead on first kick".
fn first_strong_peak_secs(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
    window_secs: f64,
) -> f64 {
    let ch = usize::from(channels.max(1));
    let max_frames =
        ((window_secs * f64::from(sample_rate)).round() as usize).min(samples.len() / ch);
    let mut best_mag = 0.0f32;
    let mut best_frame = 0usize;
    for frame in 0..max_frames {
        let base = frame * ch;
        let l = samples[base];
        let r = if ch > 1 { samples[base + 1] } else { l };
        let mag = (0.5 * (l + r)).abs();
        if mag > best_mag {
            best_mag = mag;
            best_frame = frame;
        }
    }
    best_frame as f64 / f64::from(sample_rate)
}

#[test]
fn brown_paper_bag_set_the_one_latches_downbeat_on_first_kick() {
    if !std::path::Path::new(BROWN_PAPER_BAG).exists() {
        eprintln!("skip brown_paper_bag_set_the_one: fixture not found at {BROWN_PAPER_BAG}");
        return;
    }

    let track = Track::load_from_path(BROWN_PAPER_BAG).expect("decode mp3");
    let samples = track.samples();
    let sr = track.sample_rate();
    let ch = track.channels();

    let auto = analyze_beat_grid(samples, sr, ch).expect("auto analyze");
    assert!(auto.confidence > 0.0, "auto grid must lock");
    assert!(
        auto.bpm > 160.0 && auto.bpm < 190.0,
        "expected ~174 BPM dnb, got {}",
        auto.bpm
    );

    let tap_secs = first_strong_peak_secs(samples, sr, ch, 8.0);
    eprintln!(
        "brown paper bag: auto bpm={:.2} bar_phase={} beats[0]={:.4} tap(first kick)={:.4}",
        auto.bpm,
        auto.bar_phase,
        auto.beats.first().copied().unwrap_or(f64::NAN),
        tap_secs
    );

    // Old pure-rotation path (what broke in round 4): nearest existing
    // grid beat to the tap — can land in silence when auto anchor is
    // offset from kicks.
    let old_phase = dub_bpm::bar_phase_from_tap(&auto, tap_secs);
    let old_downbeat = auto.beats[old_phase as usize];
    eprintln!(
        "pure rotation: downbeat at {:.4} (Δtap={:.1} ms)",
        old_downbeat,
        (old_downbeat - tap_secs).abs() * 1000.0
    );

    let latched =
        latch_beat_grid_at_downbeat(samples, sr, ch, auto.bpm, tap_secs, OctaveProfile::Default)
            .expect("relatch");
    let new_downbeat = latched.beats[latched.bar_phase as usize];
    eprintln!(
        "relatch: downbeat at {:.4} (Δtap={:.1} ms) bpm={:.2}",
        new_downbeat,
        (new_downbeat - tap_secs).abs() * 1000.0,
        latched.bpm
    );

    // Relatch must put the yellow marker on the audible kick the user
    // tapped, not on a flat grid line half a beat away.
    assert!(
        (new_downbeat - tap_secs).abs() < 0.05,
        "relatch downbeat {new_downbeat} must land within 50 ms of tap {tap_secs}"
    );

    // Document the failure mode we fixed: pure rotation can miss by
    // nearly a full beat when auto grid is misaligned.
    if (old_downbeat - tap_secs).abs() > 0.08 {
        eprintln!(
            "confirmed: pure rotation would have missed the kick by {:.1} ms",
            (old_downbeat - tap_secs).abs() * 1000.0
        );
    }
}
