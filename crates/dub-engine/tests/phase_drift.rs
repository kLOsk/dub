//! MWM Phase against the record it sits on.
//!
//! Phase sends a speed-only tone: no position code, nothing to pull a
//! sample back under the sticker. Daniel's memory of it on Traktor: "it
//! worked but it was always off". This measures where the "off" comes
//! from, on takes recorded for it (2026-11-24): the Phase remote on a
//! control record, the needle on that record into deck A, the Phase
//! receiver into deck B, both on one clock while he juggled and
//! scratched. The record's position code is the truth.
//!
//! Three ways of following Phase, each measured against the record from
//! the same starting point:
//!
//! - **Dub as it is**: the engine with a detecting timecode input, as the
//!   app attaches it — the playhead integrates the decoded speed.
//! - **The decoder's own integral**: the raw rate summed, before the
//!   smoother and the lift policy (garbage through backspins — the engine
//!   never uses it there; shown for completeness).
//! - **Counting the tone's cycles**: Phase's tone is synthesised, so its
//!   phase is the remote's own count of the record's turning.
//!
//! What it found. **Serato mode** (1 kHz, against Serato vinyl; a clean
//! take): Phase is the record's motion 8 ms late (correlation 0.9998); it
//! reads the platter 0.56 % fast, which alone is 5.6 ms of drift a second.
//! With that scale taken out, its own count came back within 3 ms of the
//! record after 229 s of juggling and backspins, where the engine adding
//! up its speed lost 66 ms more. The engine now takes it out
//! (`PHASE_SERATO_MODE_FAST`), so its "counted" column ends −1.9 ms; the
//! "count" column stays the raw tone, scale and all. **Traktor mode** (2.5 kHz, against
//! Traktor MK1 vinyl, the clean take of 2026-11-25; the first, beside a
//! build, broke 143 times and measured nothing): 11 ms late (0.993), the
//! speed true to 0.01 %, and the count held within 7 ms over the last
//! 105 s of steady play, but two minutes of juggling put it 80 ms ahead of
//! the record (+91 ms at the end; adding up speed ended −16 ms). It gained
//! where the record's code goes unread (stops, turnarounds, the hand on
//! the record), and the tone never passed 5×, so it is not the 2.5 kHz
//! carrier running out of room. The remote was as loose in the Serato-mode
//! take, which held, so Dub asks for Serato mode.
//!
//! [`vinyl_count`] asks the other question: counting a real record's
//! carrier instead of reading its code (what it would do for Traktor MK2,
//! which Dub reads speed-only). On the clean takes it ends −3 and −21 ms
//! off over 310 s of steady play with fader moves (adding up speed: +37,
//! +85) and +36 ms over the juggling take (+106): about three times better
//! than speed. On the Traktor-mode take's MK1, juggled by hand, both end
//! about a second off. It loses whole carrier cycles — deck A's error moves in
//! exact 1.0 ms steps, one Serato cycle — to dust ticks, and more at stops
//! where the cartridge's output falls with the record's speed. Phase's tone
//! has neither.
//!
//! The captures are not in git (they record Serato's and NI's records);
//! the tests skip without them.
//!
//! ```text
//! cargo test --release -p dub-engine --test phase_drift -- --nocapture
//! ```

use std::f64::consts::{PI, TAU};
use std::path::PathBuf;
use std::sync::Arc;

use dub_engine::{Engine, RealtimeContext, TimecodeInputConfig};
use dub_io::Track;
use dub_timecode::{Decoder, Format};
use ringbuf::traits::{Producer as _, Split as _};
use ringbuf::HeapRb;

const BLOCK: usize = 512;
/// One turn of a 12" control record in groove seconds.
const TURN_SECS: f64 = 1.8;

struct Row {
    t: f64,
    /// Where the needle is on the record, when its code is locked.
    truth: Option<f64>,
    playhead: f64,
    /// The same engine with Phase counting on (Preferences ▸ Beta).
    counted: f64,
    integral: f64,
    count: f64,
    /// The record's own carrier cycles, counted the same way: a check on
    /// its position code (they part only where the code misread or the
    /// count slipped on a scratch).
    rec_count: f64,
}

/// The take's sample rate and its two decks' stereo pairs, from the
/// four-channel float WAV `dub capture --input-channels 3,4,5,6` writes
/// (`dub_io` reads stereo only).
fn take(name: &str) -> Option<(u32, Vec<f32>, Vec<f32>)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/timecode/full")
        .join(name);
    if !path.exists() {
        eprintln!("skipped: {name} is not in testdata/timecode/full/");
        return None;
    }
    let mut wav =
        hound::WavReader::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    assert_eq!(wav.spec().channels, 4, "deck A pair then deck B pair");
    let sr = wav.spec().sample_rate;
    let samples: Vec<f32> = wav.samples::<f32>().map(|s| s.expect("sample")).collect();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for frame in samples.chunks_exact(4) {
        a.extend_from_slice(&frame[..2]);
        b.extend_from_slice(&frame[2..]);
    }
    Some((sr, a, b))
}

/// The carrier's unwrapped phase in turns, one value per millisecond.
/// Silence (a still Phase remote sends nothing) has no phase and adds
/// nothing.
fn turns_per_ms(pair: &[f32], sr: u32) -> Vec<f64> {
    let step = (sr / 1000) as usize;
    let (mut prev, mut turns, mut out) = (None::<f64>, 0.0, Vec::new());
    for (i, fr) in pair.chunks_exact(2).enumerate() {
        let (im, re) = (f64::from(fr[0]), f64::from(fr[1]));
        if re * re + im * im < 1e-4 {
            prev = None;
        } else {
            let angle = im.atan2(re);
            if let Some(p) = prev {
                turns += ((angle - p + PI).rem_euclid(TAU) - PI) / TAU;
            }
            prev = Some(angle);
        }
        if i % step == 0 {
            out.push(turns);
        }
    }
    out
}

/// Moments the input stream itself broke: the carrier's phase step jumps
/// on both decks within a millisecond. A record and a remote don't do
/// that together; a dropped or repeated buffer does — the SL 3's start-up
/// fault, or CoreAudio dropping input on a busy machine (the Traktor take
/// lost 0.35 s twice while a build ran beside it, 2026-11-24).
fn stream_faults(a: &[f32], b: &[f32], sr: u32) -> Vec<f64> {
    let breaks = |pair: &[f32]| {
        // Only where the carrier is strong. Near a stop a record's carrier
        // (and Phase's) sinks into noise whose phase jumps at random, and a
        // scratch puts both decks there at once — that is not the stream.
        let power = |fr: &[f32]| f64::from(fr[0]).powi(2) + f64::from(fr[1]).powi(2);
        let heard: Vec<f64> = pair
            .chunks_exact(2)
            .map(power)
            .filter(|&p| p >= 1e-4)
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let strong = 0.25 * heard.iter().sum::<f64>() / heard.len().max(1) as f64;
        let (mut out, mut prev, mut prev_step) = (Vec::new(), None::<f64>, None::<f64>);
        for (i, fr) in pair.chunks_exact(2).enumerate() {
            let (im, re) = (f64::from(fr[0]), f64::from(fr[1]));
            if re * re + im * im < strong {
                (prev, prev_step) = (None, None);
                continue;
            }
            let angle = im.atan2(re);
            if let Some(p) = prev {
                let step = (angle - p + PI).rem_euclid(TAU) - PI;
                if prev_step.is_some_and(|q| (step - q).abs() > 0.6) {
                    out.push(i);
                }
                prev_step = Some(step);
            }
            prev = Some(angle);
        }
        out
    };
    let (ba, bb) = (breaks(a), breaks(b));
    let near = (sr / 1000) as usize;
    let mut faults: Vec<f64> = Vec::new();
    for &i in &ba {
        let j = bb.partition_point(|&x| x + near < i);
        if bb.get(j).is_some_and(|&x| x <= i + near) {
            let t = i as f64 / f64::from(sr);
            if faults.last().is_none_or(|&last| t - last > 0.01) {
                faults.push(t);
            }
        }
    }
    faults
}

/// Absorbing a fault re-zeroes the count against the record's code, so
/// every fault inside the measured stretch hides whatever the count had
/// drifted by then: a take with them measures nothing. The first
/// Traktor-mode take (2026-11-24) had 143 (a build ran beside the
/// capture) and once read as Phase holding within 1.2 ms.
fn warn_late_faults(faults: &[f64], start: f64) {
    let late = faults.iter().filter(|&&f| f > start).count();
    if late > 0 {
        println!(
            "!! {late} stream fault(s) after {start} s: the drift columns below are not a \
             measurement — record the take again with nothing else running"
        );
    }
}

/// Speed over 10 ms, every millisecond, in carrier turns per second.
fn speeds(turns: &[f64]) -> Vec<f64> {
    turns.windows(11).map(|w| (w[10] - w[0]) * 100.0).collect()
}

/// Pearson correlation of `a` against `b` shifted `lag` samples later.
fn correlation(a: &[f64], b: &[f64], lag: usize) -> f64 {
    let n = a.len().min(b.len()).saturating_sub(lag);
    let (a, b) = (&a[..n], &b[lag..lag + n]);
    #[allow(clippy::cast_precision_loss)]
    let (ma, mb) = (
        a.iter().sum::<f64>() / n as f64,
        b.iter().sum::<f64>() / n as f64,
    );
    let num: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let den = (a.iter().map(|x| (x - ma).powi(2)).sum::<f64>()
        * b.iter().map(|y| (y - mb).powi(2)).sum::<f64>())
    .sqrt();
    num / den
}

fn rows(sr: u32, a: &[f32], b: &[f32], record: Format, phase: Format) -> Vec<Row> {
    let srf = f64::from(sr);
    #[allow(clippy::cast_precision_loss)]
    let srf32 = sr as f32;

    let engine_with = |count: bool| {
        let mut engine = Engine::new(srf32, BLOCK);
        let (tx, rx) = HeapRb::<f32>::new(sr as usize * 2).split();
        let cfg = TimecodeInputConfig {
            input_sample_rate: srf32,
            detect_vinyl: true,
            count_synthetic: count,
            ..TimecodeInputConfig::default()
        };
        engine.attach_timecode_input(0, rx, cfg).expect("attach");
        let silence = vec![0.0_f32; sr as usize * 2 * 900];
        engine.deck_mut(0).set_source(Arc::new(
            Track::from_interleaved(silence, sr, 2).expect("track"),
        ));
        engine.deck_mut(0).set_position_frames(srf * 450.0);
        (engine, tx)
    };
    let (mut engine, mut tx) = engine_with(false);
    let (mut counted_engine, mut counted_tx) = engine_with(true);

    let mut truth_dec = Decoder::with_absolute(record, srf32);
    let mut raw_dec = Decoder::new(phase, srf32);
    let carrier = f64::from(phase.carrier_hz());
    let rec_carrier = f64::from(record.carrier_hz());
    let (mut rec_prev, mut rec_cycles) = (None::<f64>, 0.0_f64);
    let mut rt = RealtimeContext::new();
    let mut out = vec![0.0_f32; BLOCK * 2];
    let (mut prev_angle, mut cycles) = (None::<f64>, 0.0_f64);
    let mut rows = Vec::new();
    for (i, (qa, qb)) in a
        .chunks_exact(BLOCK * 2)
        .zip(b.chunks_exact(BLOCK * 2))
        .enumerate()
    {
        let t_out = truth_dec.process(qa);
        let truth = t_out
            .abs_position_frames
            .filter(|_| t_out.abs_confidence >= 0.9)
            .map(|f| f / srf);
        let integral = raw_dec.process(qb).position_secs;
        for fr in qa.chunks_exact(2) {
            let (im, re) = (f64::from(fr[0]), f64::from(fr[1]));
            if re * re + im * im < 1e-4 {
                rec_prev = None;
                continue;
            }
            let angle = im.atan2(re);
            if let Some(p) = rec_prev {
                rec_cycles += ((angle - p + PI).rem_euclid(TAU) - PI) / TAU;
            }
            rec_prev = Some(angle);
        }
        for fr in qb.chunks_exact(2) {
            let (im, re) = (f64::from(fr[0]), f64::from(fr[1]));
            if re * re + im * im < 1e-4 {
                prev_angle = None;
                continue;
            }
            let angle = im.atan2(re);
            if let Some(p) = prev_angle {
                cycles += ((angle - p + PI).rem_euclid(TAU) - PI) / TAU;
            }
            prev_angle = Some(angle);
        }
        assert_eq!(tx.push_slice(qb), qb.len());
        engine.render(&mut rt, &mut out);
        assert_eq!(counted_tx.push_slice(qb), qb.len());
        counted_engine.render(&mut rt, &mut out);
        #[allow(clippy::cast_precision_loss)]
        rows.push(Row {
            t: ((i + 1) * BLOCK) as f64 / srf,
            truth,
            playhead: engine.deck(0).position_frames() / srf,
            counted: counted_engine.deck(0).position_frames() / srf,
            integral,
            count: cycles / carrier,
            rec_count: rec_cycles / rec_carrier,
        });
    }
    rows
}

/// Measure one take and print the report. `start` is when both decks
/// have calibrated and play steadily.
fn report(name: &str, record: Format, phase: Format, start: f64) {
    let Some((sr, a, b)) = take(name) else {
        return;
    };
    println!("\n=== {name}");

    // Is deck B the remote on deck A's record? Its motion has to be the
    // record's, a few milliseconds late.
    let (va, vb) = (speeds(&turns_per_ms(&a, sr)), speeds(&turns_per_ms(&b, sr)));
    let lag = (0..80)
        .max_by(|&x, &y| correlation(&va, &vb, x).total_cmp(&correlation(&va, &vb, y)))
        .unwrap_or(0);
    let fit = correlation(&va, &vb, lag);
    println!("Phase trails the record by {lag} ms (correlation {fit:.4})");
    assert!(
        fit > 0.95,
        "deck B does not move with deck A's record — the wrong remote?"
    );

    let faults = stream_faults(&a, &b, sr);
    if !faults.is_empty() {
        println!(
            "input stream broke {} time(s), first at {:.2} s, last at {:.2} s — absorbed below",
            faults.len(),
            faults[0],
            faults[faults.len() - 1]
        );
    }
    warn_late_faults(&faults, start);
    let rows = rows(sr, &a, &b, record, phase);
    let Some(first) = rows.iter().position(|r| r.t > start && r.truth.is_some()) else {
        panic!("the record never locked after {start} s");
    };
    let r0 = &rows[first];
    let t0 = r0.truth.unwrap_or_default();
    // A needle skip moves the truth by whole turns; Phase's remote turns
    // with the record and knows nothing of it. Take skips out of the
    // truth so they don't read as Phase drifting.
    let (mut skips, mut skip_secs, mut last_offset, mut last_locked) = (0, 0.0, 0.0, r0.t);
    println!(
        "{:>7} {:>9} {:>10} {:>10} {:>10} {:>10} {:>10}   (ms ahead of the record)",
        "t", "record", "dub", "counted", "integral", "count", "rec count"
    );
    let (mut last_t, mut finals) = (-10.0, [0.0_f64; 4]);
    for r in &rows[first..] {
        let Some(truth) = r.truth else {
            continue;
        };
        let count_moved = r.count - r0.count;
        let offset = count_moved - (truth - t0 + skip_secs);
        // Across a stream fault every series lost or repeated the same
        // audio, but the record's code says where the needle really was:
        // take the step out, as the live deck (which holds its speed
        // through a gap) would never see it.
        if faults.iter().any(|&f| f > last_locked - 0.05 && f <= r.t) {
            skip_secs += offset - last_offset;
        } else if (offset - last_offset).abs() > TURN_SECS / 2.0 {
            let turns = ((offset - last_offset) / TURN_SECS).round();
            skip_secs += turns * TURN_SECS;
            skips += 1;
            println!("{:>7.2}  needle skip, {turns:+} turn(s)", r.t);
        }
        let moved = truth - t0 + skip_secs;
        finals = [
            (r.playhead - r0.playhead - moved) * 1000.0,
            (r.counted - r0.counted - moved) * 1000.0,
            (r.integral - r0.integral - moved) * 1000.0,
            (count_moved - moved) * 1000.0,
        ];
        last_offset = count_moved - moved;
        last_locked = r.t;
        if r.t - last_t >= 2.0 {
            println!(
                "{:>7.2} {:>9.2} {:>+10.1} {:>+10.1} {:>+10.1} {:>+10.1} {:>+10.1}",
                r.t,
                truth,
                finals[0],
                finals[1],
                finals[2],
                finals[3],
                (r.rec_count - r0.rec_count - moved) * 1000.0
            );
            last_t = r.t;
        }
    }
    println!(
        "end: dub {:+.1} ms · dub counted {:+.1} ms · integral {:+.1} ms · count {:+.1} ms · \
         {skips} needle skip(s)",
        finals[0], finals[1], finals[2], finals[3]
    );
}

#[test]
fn phase_in_serato_mode_against_serato_vinyl() {
    report(
        "phase-vs-serato-1124-serato-mode-4ch.wav",
        Format::SeratoCv02,
        Format::SeratoCv02,
        10.0,
    );
}

#[test]
fn phase_in_traktor_mode_against_traktor_vinyl() {
    report(
        "phase-vs-traktor-1125-traktor-mode-4ch.wav",
        Format::TraktorMk1,
        Format::TraktorMk2,
        8.0,
    );
}

/// Counting a real record's carrier, measured against that record's own
/// position code: what counting would do for a record Dub can only read
/// speed-only (Traktor MK2, no public position code). Each column is one
/// way of following the record from the same start; needle skips (whole
/// turns in the code) and input-stream faults are taken out per column.
fn vinyl_count(name: &str, format: Format, start: f64) {
    const FLOORS: [f64; 3] = [1e-3, 1e-4, 1e-6];
    let Some((sr, a, b)) = take(name) else {
        return;
    };
    let srf = f64::from(sr);
    #[allow(clippy::cast_precision_loss)]
    let srf32 = sr as f32;
    let carrier = f64::from(format.carrier_hz());
    let faults = stream_faults(&a, &b, sr);
    warn_late_faults(&faults, start);
    for (deck, pair) in [("A", &a), ("B", &b)] {
        let mut truth_dec = Decoder::with_absolute(format, srf32);
        let mut speed_dec = Decoder::new(format, srf32);
        let mut counts = [(None::<(f64, f64)>, 0.0_f64); FLOORS.len()];
        // (t, truth, [count secs per floor], speed integral)
        let mut rows: Vec<(f64, Option<f64>, [f64; 3], f64)> = Vec::new();
        for (i, block) in pair.chunks_exact(BLOCK * 2).enumerate() {
            let out = truth_dec.process(block);
            let truth = out
                .abs_position_frames
                .filter(|_| out.abs_confidence >= 0.9)
                .map(|f| f / srf);
            let integral = speed_dec.process(block).position_secs;
            for fr in block.chunks_exact(2) {
                let (re, im) = (f64::from(fr[1]), f64::from(fr[0]));
                for ((prev, turns), floor) in counts.iter_mut().zip(FLOORS) {
                    if re * re + im * im >= floor {
                        if let Some((pr, pi)) = *prev {
                            *turns += (im * pr - re * pi).atan2(re * pr + im * pi) / TAU;
                        }
                        *prev = Some((re, im));
                    } else {
                        *prev = None;
                    }
                }
            }
            #[allow(clippy::cast_precision_loss)]
            rows.push((
                ((i + 1) * BLOCK) as f64 / srf,
                truth,
                [
                    counts[0].1 / carrier,
                    counts[1].1 / carrier,
                    counts[2].1 / carrier,
                ],
                integral,
            ));
        }
        let Some(first) = rows.iter().position(|r| r.0 > start && r.1.is_some()) else {
            println!("\n=== {name} deck {deck}: no position code (not a record)");
            continue;
        };
        let (t0, truth0, c0, i0) = (
            rows[first].0,
            rows[first].1.unwrap_or_default(),
            rows[first].2,
            rows[first].3,
        );
        println!("\n=== {name} deck {deck} ({format:?})");
        println!(
            "{:>7} {:>9} {:>10} {:>10} {:>10} {:>10}   (ms ahead of the record)",
            "t", "record", "count 1e-3", "count 1e-4", "count 1e-6", "speed sum"
        );
        // Per column: steps taken out (skips, faults), last offset.
        let mut fix = [(0.0_f64, 0.0_f64); 4];
        let (mut last_locked, mut last_print, mut skips) = (t0, -10.0, 0);
        let mut offsets = [0.0_f64; 4];
        for r in &rows[first..] {
            let Some(truth) = r.1 else {
                continue;
            };
            let moved = truth - truth0;
            let raw = [
                r.2[0] - c0[0] - moved,
                r.2[1] - c0[1] - moved,
                r.2[2] - c0[2] - moved,
                r.3 - i0 - moved,
            ];
            let fault = faults.iter().any(|&f| f > last_locked - 0.05 && f <= r.0);
            for (k, (taken, last)) in fix.iter_mut().enumerate() {
                let step = raw[k] - *taken - *last;
                if fault {
                    *taken += step;
                } else if step.abs() > TURN_SECS / 2.0 {
                    *taken += (step / TURN_SECS).round() * TURN_SECS;
                    if k == 1 {
                        skips += 1;
                    }
                }
                *last = raw[k] - *taken;
                offsets[k] = *last * 1000.0;
            }
            last_locked = r.0;
            if r.0 - last_print >= 10.0 {
                println!(
                    "{:>7.1} {:>9.2} {:>+10.1} {:>+10.1} {:>+10.1} {:>+10.1}",
                    r.0, truth, offsets[0], offsets[1], offsets[2], offsets[3]
                );
                last_print = r.0;
            }
        }
        println!(
            "end: count {:+.1} / {:+.1} / {:+.1} ms · speed sum {:+.1} ms · {skips} needle skip(s), {} stream fault(s)",
            offsets[0], offsets[1], offsets[2], offsets[3], faults.len()
        );
    }
}

#[test]
fn counting_real_vinyl_against_its_code() {
    vinyl_count(
        "phase-vs-serato-1124-serato-mode-4ch.wav",
        Format::SeratoCv02,
        10.0,
    );
    vinyl_count(
        "phase-vs-traktor-1125-traktor-mode-4ch.wav",
        Format::TraktorMk1,
        8.0,
    );
    vinyl_count("sl3-serato-1122-0844-4ch.wav", Format::SeratoCv02, 10.0);
}

/// The calibration tells Phase from a record by how flat its level is
/// (`SYNTHETIC_MAX_SPREAD`, 0.3 % in `timecode.rs`): chunk to chunk Phase
/// varies by hundredths of a percent, a record by several percent.
#[test]
fn phase_is_flat_and_records_are_not() {
    let spread = |pair: &[f32], sr: u32, format: Format, from: f64| {
        #[allow(clippy::cast_precision_loss)]
        let mut dec = Decoder::with_absolute(format, sr as f32);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let start = (from * f64::from(sr)) as usize * 2;
        let levels: Vec<f64> = pair[start..start + sr as usize * 4]
            .chunks_exact(128)
            .map(|c| f64::from(dec.process(c).amplitude))
            .collect();
        let levels = &levels[levels.len() / 2..];
        #[allow(clippy::cast_precision_loss)]
        let n = levels.len() as f64;
        let mean = levels.iter().sum::<f64>() / n;
        (levels.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / n).sqrt() / mean * 100.0
    };
    for (name, record, phase, at) in [
        (
            "phase-vs-serato-1124-serato-mode-4ch.wav",
            Format::SeratoCv02,
            Format::SeratoCv02,
            20.0,
        ),
        (
            "phase-vs-traktor-1125-traktor-mode-4ch.wav",
            Format::TraktorMk1,
            Format::TraktorMk2,
            10.0,
        ),
    ] {
        let Some((sr, a, b)) = take(name) else {
            continue;
        };
        let (rec, ph) = (spread(&a, sr, record, at), spread(&b, sr, phase, at));
        println!("{name}: record {rec:.3} %, Phase {ph:.3} %");
        assert!(ph < 0.1, "Phase varies {ph:.3} %");
        assert!(rec > 3.0, "the record varies only {rec:.3} %");
    }
}
