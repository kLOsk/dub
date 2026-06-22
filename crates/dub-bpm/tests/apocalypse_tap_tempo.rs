//! Local reproduction: Apocalypse tap-tempo should land on 175.0,
//! not 175.10. Runs only when the author's file is present.

use dub_bpm::{analyze_beat_grid_from_bpm_and_anchor, OctaveProfile};
use dub_io::Track;

const APOCALYPSE: &str =
    "/Users/klos/Music/Music/DrumnBass/Excel - Apocalypse (Nick The Lot remix).mp3";

fn first_strong_peak_secs(samples: &[f32], sr: u32, ch: u8, window_secs: f64) -> f64 {
    let ch = usize::from(ch.max(1));
    let max_frames = ((window_secs * f64::from(sr)).round() as usize).min(samples.len() / ch);
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
    best_frame as f64 / f64::from(sr)
}

#[test]
fn apocalypse_tap_175_lands_on_integer() {
    if !std::path::Path::new(APOCALYPSE).exists() {
        eprintln!("skip apocalypse_tap_tempo: fixture not found");
        return;
    }
    let track = Track::load_from_path(APOCALYPSE).expect("decode");
    let samples = track.samples();
    let sr = track.sample_rate();
    let ch = track.channels();
    let first_kick = first_strong_peak_secs(samples, sr, ch, 8.0);

    // The app taps WHILE PLAYING, so the anchor is a mid-track tap,
    // and `genre` may be nil at tap time → Default profile. On this
    // poor-onset DnB track the LSQ minimum wanders to 174.8/174.9
    // depending on the tap anchor; the tap-path integer snap must
    // pull every one of them to the clean 175.0 the human tapped.
    for profile in [OctaveProfile::DrumAndBass, OctaveProfile::Default] {
        for anchor in [first_kick, 30.013, 61.027, 95.5] {
            for hint in [170.0, 174.2, 175.0, 175.6, 180.0] {
                let grid =
                    analyze_beat_grid_from_bpm_and_anchor(samples, sr, ch, hint, anchor, profile)
                        .expect("tap grid");
                eprintln!(
                    "{profile:?} anchor {anchor:6.2} hint {hint:6.2} -> bpm {:.4}",
                    grid.bpm
                );
                assert!(
                    (grid.bpm - 175.0).abs() < 0.001,
                    "tap on a DnB 175.0 track must snap to 175.0, got {:.4} \
                     ({profile:?}, anchor {anchor:.2}, hint {hint:.2})",
                    grid.bpm
                );
            }
        }
    }
}
