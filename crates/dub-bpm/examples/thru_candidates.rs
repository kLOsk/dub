//! Diagnostic: the THRU tempo search's candidates on a stretch of a
//! record, as a THRU deck would hear it — mono, the live estimator's
//! 60 s onset window, the DJ's default octave.
//!
//! ```sh
//! DUB_BPM_DEBUG=1 cargo run --release -p dub-bpm --example thru_candidates -- \
//!     testdata/rip-baselines/rip-baseline-c.flac 60 30,60,90
//! ```
//!
//! Arguments: the file, where the needle lands (seconds into the file),
//! and the moments after that to print the table at.
//!
//! `thru_candidates <file> octave 60,78,100` reads the whole file in each
//! octave instead, as the library analyses a track.

use dub_bpm::{analyze_beat_grid_in_octave, BpmEstimator, BpmRange, TempoOctave};
use dub_io::Track;

fn whole_file(path: &str, lows: &str) {
    let mut load = Track::begin_streaming(path).expect("open the file");
    load.decode_until_secs(f64::MAX).expect("decode the file");
    let track = load.track();
    let samples = track.samples_to_vec();
    for low in lows.split(',') {
        let low: f64 = low.parse().expect("octave low");
        let octave = TempoOctave::from_low(low).expect("octave");
        let grid =
            analyze_beat_grid_in_octave(&samples, track.sample_rate(), track.channels(), octave)
                .expect("analysis");
        eprintln!(
            "== octave {low}-{}: {:.2} BPM, confidence {:.3}",
            2.0 * low,
            grid.bpm,
            grid.confidence
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[2] == "octave" {
        whole_file(&args[1], &args[3]);
        return;
    }
    let [_, path, from, at] = args.as_slice() else {
        eprintln!("usage: thru_candidates <file> <from-secs> <secs,secs,...>");
        std::process::exit(2);
    };
    let from: f64 = from.parse().expect("from-secs");
    let mut at: Vec<f64> = at.split(',').map(|s| s.parse().expect("secs")).collect();
    at.sort_by(f64::total_cmp);
    let until = from + at.last().copied().unwrap_or(0.0);

    let mut load = Track::begin_streaming(path).expect("open the file");
    load.decode_until_secs(until).expect("decode the file");
    let track = load.track();
    let sr = track.sample_rate();
    let ch = usize::from(track.channels());
    let all = track.samples_to_vec();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frame = |secs: f64| ((secs * f64::from(sr)) as usize).min(all.len() / ch);
    #[allow(clippy::cast_precision_loss)]
    let mono: Vec<f32> = all[frame(from) * ch..frame(until) * ch]
        .chunks_exact(ch)
        .map(|f| f.iter().sum::<f32>() / ch as f32)
        .collect();

    let mut est = BpmEstimator::with_range(sr, BpmRange::DEFAULT).expect("estimator");
    est.set_window_secs(Some(60.0));
    let mut fed = 0usize;
    for secs in at {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let to = ((secs * f64::from(sr)) as usize).min(mono.len());
        for block in mono[fed..to].chunks(4096) {
            est.feed(block);
        }
        fed = to;
        eprintln!(
            "== {secs:.0} s after the needle ({:.0} s into the file)",
            from + secs
        );
        est.recompute();
        match est.current() {
            Some(e) => eprintln!("   -> {:.2} BPM, confidence {:.3}", e.bpm, e.confidence),
            None => eprintln!("   -> nothing"),
        }
    }
}
