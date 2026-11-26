//! Sticker drift: does a sample stay where it sits on the record?
//!
//! A timecode deck should behave like a record: a sample sits at one
//! place in the groove for as long as the needle stays in it. Whatever
//! the DJ does in between — scratch, catch, spin back — the playhead and
//! the groove keep one fixed offset, and a needle skip must not move the
//! track either (that is the point of relative mode: the sticker holds).
//!
//! Two kinds of evidence, both through `Engine::render` with a track
//! loaded:
//!
//! - **Real captures** (`testdata/timecode/full/`, not in git — recordings
//!   of Serato's and Native Instruments' records; the tests skip without
//!   them). At every block where the LFSR lock says where the needle
//!   really is, how far `groove − playhead` has moved from where it
//!   started.
//! - **A synthetic juggle**, run everywhere: play a phrase, catch, spin
//!   back to the sticker, hold, let go, sixteen times, on a carrier that
//!   behaves like a cartridge (its level follows the record's speed and
//!   it turns over when the record reverses). After each pass the track
//!   must be where the record is.
//!
//! The numbers this answered (2026-11-24): ten seconds of real
//! scratching locked zero times and came out 22 ms off; short juggle
//! phrases stacked −4 ms a pass; Traktor vinyl had no absolute decode and
//! stacked +1.2 ms a pass without limit; the heal's 4 ms deadband left the
//! sticker resting up to 4 ms off. See PRD §5.4.3.
//!
//! ```text
//! cargo test --release -p dub-engine --test sticker_drift -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use dub_engine::{Engine, RealtimeContext, TimecodeInputConfig};
use dub_io::Track;
use dub_timecode::signal::Generator;
use dub_timecode::Format;
use ringbuf::traits::{Producer as _, Split as _};
use ringbuf::HeapRb;

const BLOCK: usize = 512;
const SR: f32 = 48_000.0;
/// One turn of a 12" control record in groove seconds (cut at 33⅓).
const TURN_SECS: f64 = 1.8;

/// An engine with a timecode input on deck 0 and a long silent track
/// started mid-way, so a spin back never reaches its start.
fn engine_for(format: Format, sample_rate: u32) -> (Engine, ringbuf::HeapProd<f32>) {
    #[allow(clippy::cast_precision_loss)]
    let srf = sample_rate as f32;
    let mut engine = Engine::new(srf, BLOCK);
    let rb = HeapRb::<f32>::new(sample_rate as usize * 2);
    let (tx, rx) = rb.split();
    let cfg = TimecodeInputConfig {
        input_sample_rate: srf,
        format,
        hold_until_calibrated: false,
        ..TimecodeInputConfig::default()
    };
    engine.attach_timecode_input(0, rx, cfg).expect("attach");
    let track = Arc::new(
        Track::from_interleaved(
            vec![0.0_f32; sample_rate as usize * 2 * 600],
            sample_rate,
            2,
        )
        .expect("track"),
    );
    engine.deck_mut(0).set_source(track);
    engine
        .deck_mut(0)
        .set_position_frames(f64::from(srf) * 300.0);
    (engine, tx)
}

// --- real captures ------------------------------------------------------

fn capture(name: &str) -> Option<Track> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/timecode/full")
        .join(name);
    if !path.exists() {
        eprintln!("skipped: {name} is not in testdata/timecode/full/");
        return None;
    }
    Some(Track::load_from_path(&path).unwrap_or_else(|e| panic!("load {}: {e}", path.display())))
}

/// `(t, drift_ms)` at every block where the LFSR says where the needle
/// is, from the offset at the first such block while playing (+ = the
/// track behind the record). The first `secs` seconds of the capture.
fn real_drift(input: &Track, format: Format, secs: f64) -> Vec<(f64, f64)> {
    let sr = input.sample_rate();
    let srf = f64::from(sr);
    let (mut engine, mut tx) = engine_for(format, sr);
    let mut rt = RealtimeContext::new();
    let mut out = vec![0.0_f32; BLOCK * 2];
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = ((secs * srf) as usize).min(input.samples().len() / 2);
    let mut anchor: Option<f64> = None;
    let mut rows = Vec::new();
    for (i, quantum) in input.samples()[..frames * 2]
        .chunks_exact(BLOCK * 2)
        .enumerate()
    {
        assert_eq!(tx.push_slice(quantum), quantum.len());
        engine.render(&mut rt, &mut out);
        let groove = engine
            .timecode_last_output(0)
            .and_then(|o| o.abs_position_frames)
            .map(|f| f / srf);
        let (Some(groove), true) = (groove, engine.deck(0).is_playing()) else {
            continue;
        };
        let offset = groove - engine.deck(0).position_frames() / srf;
        let a = *anchor.get_or_insert(offset);
        #[allow(clippy::cast_precision_loss)]
        rows.push((((i + 1) * BLOCK) as f64 / srf, (offset - a) * 1000.0));
    }
    rows
}

/// Where the sticker rests from `from` seconds on: the median and the
/// worst |drift|. A dust tick can still cost a couple of milliseconds
/// for a moment — the hold slows the deck while the record turns on —
/// and the heal pulls it back within a second, so the median is what
/// says whether the sticker holds.
fn settled(rows: &[(f64, f64)], from: f64) -> (f64, f64) {
    let mut d: Vec<f64> = rows
        .iter()
        .filter(|r| r.0 > from)
        .map(|r| r.1.abs())
        .collect();
    assert!(!d.is_empty(), "no locked blocks after {from} s");
    d.sort_by(f64::total_cmp);
    (d[d.len() / 2], d[d.len() - 1])
}

fn print_every_half_second(rows: &[(f64, f64)]) {
    let mut last = -1.0;
    for &(t, d) in rows {
        if t - last >= 0.5 {
            println!("{t:>7.2} {d:>+8.2}");
            last = t;
        }
    }
}

/// Deck A, 2026-08: steady play, a stop and restart (31–33 s), then ten
/// seconds of back-and-forth scratching (34.4–43.9 s). The LFSR used to
/// find nothing in the scratch and the deck came out 22 ms behind; it now
/// locks on the strokes, both ways.
#[test]
fn real_scratching_comes_back_on_the_sticker() {
    let Some(input) = capture("sl3-deckA-60s.flac") else {
        return;
    };
    let rows = real_drift(&input, Format::SeratoCv02, 60.0);
    println!("\n=== Real scratching, Serato (t s, drift ms) ===");
    print_every_half_second(&rows);
    let in_scratch = rows.iter().filter(|r| r.0 > 34.6 && r.0 < 43.8).count();
    assert!(
        in_scratch > 20,
        "the LFSR barely locked inside the scratch ({in_scratch} blocks)"
    );
    let worst = rows.iter().map(|r| r.1.abs()).fold(0.0, f64::max);
    let (median, after) = settled(&rows, 45.0);
    println!(
        "scratch locks {in_scratch} blocks, worst {worst:.2} ms; after it median {median:.2}, worst {after:.2} ms"
    );
    assert!(worst < 15.0, "drift reached {worst:.1} ms");
    assert!(
        median < 0.3,
        "the sticker rests {median:.2} ms off after the scratch"
    );
    // At 51.1 s a dust tick drops the lock for 60 ms and the hold slows
    // the deck through it: ~3 ms, healed within a second.
    assert!(
        after < 4.0,
        "{after:.2} ms off the sticker after the scratch"
    );
}

/// Traktor MK1 vinyl, a 12-minute take of held fader steps: before
/// 2026-11-24 Traktor had no absolute decode and nothing measured the
/// drift at all. The first three minutes: it locks, and holds.
#[test]
fn real_traktor_vinyl_holds_the_sticker() {
    let Some(input) = capture("sl3-traktor-left.wav") else {
        return;
    };
    let rows = real_drift(&input, Format::TraktorMk1, 180.0);
    println!("\n=== Real Traktor MK1, held steps (t s, drift ms) ===");
    print_every_half_second(&rows);
    assert!(
        rows.len() > 10_000,
        "Traktor MK1 locked on only {} blocks",
        rows.len()
    );
    let (median, worst) = settled(&rows, 10.0);
    println!(
        "locked {} blocks; median {median:.2}, worst {worst:.2} ms",
        rows.len()
    );
    assert!(median < 0.3, "the sticker rests {median:.2} ms off");
    assert!(worst < 3.0, "Traktor drift reached {worst:.2} ms");
}

// --- the synthetic juggle -----------------------------------------------

/// Rate ramps `r0 → r1` over `dur` seconds.
#[derive(Clone, Copy)]
struct Seg {
    r0: f64,
    r1: f64,
    dur: f64,
}

/// One juggle pass, net zero groove travel: let it play `play` seconds,
/// catch it, spin back to the sticker at up to `spin` × speed, hold it
/// there, let go.
fn juggle_pass(play: f64, spin: f64) -> Vec<Seg> {
    let catch = 0.05;
    let release = 0.06;
    let forward = play + catch / 2.0 + release / 2.0;
    let (up, down) = (0.10, 0.15);
    let hold_back = (forward - spin * (up + down) / 2.0) / spin;
    assert!(hold_back > 0.0, "spin too fast for the phrase");
    vec![
        Seg {
            r0: 0.0,
            r1: 1.0,
            dur: release,
        },
        Seg {
            r0: 1.0,
            r1: 1.0,
            dur: play,
        },
        Seg {
            r0: 1.0,
            r1: 0.0,
            dur: catch,
        },
        Seg {
            r0: 0.0,
            r1: -spin,
            dur: up,
        },
        Seg {
            r0: -spin,
            r1: -spin,
            dur: hold_back,
        },
        Seg {
            r0: -spin,
            r1: 0.0,
            dur: down,
        },
        Seg {
            r0: 0.0,
            r1: 0.0,
            dur: 0.3,
        },
    ]
}

fn rate_at(segs: &[Seg], t: f64) -> f64 {
    let mut t0 = 0.0;
    for s in segs {
        if t < t0 + s.dur {
            return s.r0 + (s.r1 - s.r0) * (t - t0) / s.dur;
        }
        t0 += s.dur;
    }
    0.0
}

/// A needle skip during a pass: `turns` whole turns of the record, in
/// the middle of that pass's segment `seg`.
#[derive(Clone, Copy)]
struct Skip {
    pass: usize,
    seg: usize,
    turns: i64,
}

/// The track's distance from the record after each pass, in ms (+ = the
/// track behind). A skip moves the record under the needle without the
/// hand moving it, so it is not in the record's travel the sticker sees.
fn juggle(format: Format, play: f64, spin: f64, passes: usize, skips: &[Skip]) -> Vec<f64> {
    let (mut engine, mut tx) = engine_for(format, 48_000);
    let mut gen = Generator::new(format, SR);
    assert!(
        gen.enable_absolute(format, 0.25),
        "{format:?} has no bitstream"
    );
    let mut rt = RealtimeContext::new();
    let mut sig = vec![0.0_f32; BLOCK * 2];
    let mut out = vec![0.0_f32; BLOCK * 2];
    let dt = BLOCK as f64 / f64::from(SR);
    let mut noise = 0x1234_5678_u32;
    let mut render = |engine: &mut Engine, gen: &mut Generator, rate: f64| {
        // A cartridge reads velocity: its level follows the speed, and the
        // carrier turns over when the record reverses.
        #[allow(clippy::cast_possible_truncation)]
        let amp = (0.35 * rate).clamp(-0.5, 0.5) as f32;
        gen.render(&mut sig, rate, amp);
        for s in &mut sig {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            #[allow(clippy::cast_possible_truncation)]
            let n = (f64::from(noise >> 8) / f64::from(1u32 << 24) - 0.5) as f32;
            *s += n * 0.003;
        }
        assert_eq!(tx.push_slice(&sig), sig.len());
        engine.render(&mut rt, &mut out);
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    for _ in 0..(3.0 / dt) as usize {
        render(&mut engine, &mut gen, 1.0);
    }
    let segs = juggle_pass(play, spin);
    let turn_cycles = (f64::from(format.carrier_hz()) * TURN_SECS).round();
    let mut errors = Vec::new();
    let mut sticker: Option<f64> = None;
    let mut groove_ms = 0.0_f64;
    for pass in 0..passes {
        let total: f64 = segs.iter().map(|s| s.dur).sum();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let blocks = (total / dt).round() as usize;
        let mut pending: Vec<Skip> = skips.iter().copied().filter(|s| s.pass == pass).collect();
        for b in 0..blocks {
            #[allow(clippy::cast_precision_loss)]
            let t = (b as f64 + 0.5) * dt;
            pending.retain(|s| {
                let start: f64 = segs[..s.seg].iter().map(|g| g.dur).sum();
                if t < start + segs[s.seg].dur / 2.0 {
                    return true;
                }
                #[allow(clippy::cast_possible_truncation)]
                gen.skip_cycles(s.turns * turn_cycles as i64);
                false
            });
            let rate = rate_at(&segs, t);
            groove_ms += rate * dt * 1000.0;
            render(&mut engine, &mut gen, rate);
        }
        let track_ms = engine.deck(0).position_frames() / f64::from(SR) * 1000.0;
        match sticker {
            None => sticker = Some(track_ms - groove_ms),
            Some(s) => errors.push(groove_ms - (track_ms - s)),
        }
    }
    errors
}

/// The typical pass lands within a millisecond of the sticker, and none
/// further than 4.
fn assert_holds(label: &str, errors: &[f64]) {
    let mut abs: Vec<f64> = errors.iter().map(|x| x.abs()).collect();
    abs.sort_by(f64::total_cmp);
    let (median, worst) = (abs[abs.len() / 2], abs[abs.len() - 1]);
    assert!(
        median < 1.0,
        "{label}: the typical pass is {median:.2} ms off"
    );
    assert!(worst < 4.0, "{label}: a pass is {worst:.2} ms off");
}

fn show(label: &str, errors: &[f64]) {
    let shown: Vec<String> = errors.iter().map(|x| format!("{x:+.1}")).collect();
    println!("{label}: {}", shown.join(" "));
}

/// Short phrases are the hard case: the LFSR has the least time to lock
/// before the next catch. Every pass must land on the sticker.
///
/// The spin back locks too, given ~50 clean cycles going one way. A short
/// one sometimes doesn't, and runs on the relative path, which under-reads
/// a fast spin's travel by ~0.7 % while the speed ramps (the rate
/// smoother's lag); the next phrase heals it at the 1 % cap. So: the
/// typical pass within a millisecond, every pass within 4.
#[test]
fn juggling_holds_the_sticker_on_both_vinyls() {
    println!("\n=== Juggle: track against record after each pass (ms) ===");
    for format in [Format::SeratoCv02, Format::TraktorMk1] {
        for (play, spin) in [(1.5, 4.0), (0.6, 4.0), (2.5, 8.0)] {
            let e = juggle(format, play, spin, 16, &[]);
            show(
                &format!("{} play {play:.1} s spin {spin:.0}x", format.cli_name()),
                &e,
            );
            assert_holds(&format!("{format:?} play {play} spin {spin}"), &e);
        }
    }
}

/// A skip moves the needle a whole turn. The sticker must hold: no jump
/// to the new groove, and no heal creeping the track there afterwards.
#[test]
fn a_needle_skip_keeps_the_sticker() {
    println!("\n=== Juggle with needle skips (ms) ===");
    let skips = [
        // Back a turn during the spin back, forward a turn while it plays.
        Skip {
            pass: 3,
            seg: 4,
            turns: 1,
        },
        Skip {
            pass: 7,
            seg: 1,
            turns: -1,
        },
        Skip {
            pass: 11,
            seg: 4,
            turns: 2,
        },
    ];
    for format in [Format::SeratoCv02, Format::TraktorMk1] {
        let e = juggle(format, 1.5, 4.0, 16, &skips);
        show(format.cli_name(), &e);
        assert_holds(&format!("{format:?} with skips"), &e);
    }
}

/// MWM Phase, juggled: its tone is flat at any speed, silent while the
/// remote is still, and carries no position code. With counting on (as
/// Preferences ▸ Beta ▸ MWM Phase sets it) the deck follows the tone's
/// own cycles and every pass lands on the sticker; adding up its speed
/// instead is what Daniel heard on Traktor as "always off".
fn phase_juggle(count: bool, play: f64, spin: f64, passes: usize) -> Vec<f64> {
    let mut engine = Engine::new(SR, BLOCK);
    let (mut tx, rx) = HeapRb::<f32>::new(48_000 * 2).split();
    let cfg = TimecodeInputConfig {
        input_sample_rate: SR,
        detect_vinyl: true,
        count_synthetic: count,
        hold_until_calibrated: false,
        ..TimecodeInputConfig::default()
    };
    engine.attach_timecode_input(0, rx, cfg).expect("attach");
    let track = Arc::new(
        Track::from_interleaved(vec![0.0_f32; 48_000 * 2 * 600], 48_000, 2).expect("track"),
    );
    engine.deck_mut(0).set_source(track);
    engine.deck_mut(0).set_position_frames(48_000.0 * 300.0);

    let mut gen = Generator::new(Format::TraktorMk2, SR);
    let mut rt = RealtimeContext::new();
    let mut sig = vec![0.0_f32; BLOCK * 2];
    let mut out = vec![0.0_f32; BLOCK * 2];
    let dt = BLOCK as f64 / f64::from(SR);
    let mut render = |engine: &mut Engine, gen: &mut Generator, rate: f64| {
        if rate.abs() < 1e-3 {
            sig.fill(0.0);
        } else {
            gen.render(&mut sig, rate, 0.33);
        }
        assert_eq!(tx.push_slice(&sig), sig.len());
        engine.render(&mut rt, &mut out);
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    for _ in 0..(3.0 / dt) as usize {
        render(&mut engine, &mut gen, 1.0);
    }
    assert_eq!(
        engine
            .timecode_last_output(0)
            .is_some_and(|o| o.abs_position_frames.is_some()),
        count,
        "the deck did not name Phase and count it (count = {count})"
    );
    let segs = juggle_pass(play, spin);
    let (mut errors, mut sticker, mut groove_ms) = (Vec::new(), None::<f64>, 0.0_f64);
    for _ in 0..passes {
        let total: f64 = segs.iter().map(|s| s.dur).sum();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let blocks = (total / dt).round() as usize;
        for b in 0..blocks {
            #[allow(clippy::cast_precision_loss)]
            let rate = rate_at(&segs, (b as f64 + 0.5) * dt);
            groove_ms += rate * dt * 1000.0;
            render(&mut engine, &mut gen, rate);
        }
        let track_ms = engine.deck(0).position_frames() / f64::from(SR) * 1000.0;
        match sticker {
            None => sticker = Some(track_ms - groove_ms),
            Some(s) => errors.push(groove_ms - (track_ms - s)),
        }
    }
    errors
}

#[test]
fn phase_counted_holds_the_sticker() {
    println!("\n=== Phase juggle: track against record after each pass (ms) ===");
    for (play, spin) in [(1.5, 4.0), (0.6, 4.0), (2.5, 8.0)] {
        let counted = phase_juggle(true, play, spin, 16);
        let added = phase_juggle(false, play, spin, 16);
        show(
            &format!("counted  play {play:.1} s spin {spin:.0}x"),
            &counted,
        );
        show(
            &format!("speed    play {play:.1} s spin {spin:.0}x"),
            &added,
        );
        assert_holds(&format!("Phase counted, play {play} spin {spin}"), &counted);
    }
}
