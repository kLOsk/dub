//! Offline reproduction of the on-rig forward scratch drift.
//!
//! The DJ baby-scratches IN PLACE — the vinyl returns to the same
//! physical groove spot every stroke — yet the song drifts FORWARD by
//! tens of ms per stroke cycle. This harness drives the full pipeline
//! (Generator → ringbuf → Decoder → RateSmoother → LiftPolicy → deck
//! transport) with physically honest carrier amplitude: a cartridge is
//! a velocity sensor, so amplitude scales with |rate| and the slow
//! backward draw outputs a much weaker carrier than the fast push.
//!
//! Exploratory: prints a results table, asserts only that the deck
//! engaged at least once. Run with:
//!
//! ```text
//! cargo test -p dub-engine --test scratch_drift -- --nocapture
//! ```

use std::sync::Arc;

use dub_engine::{Engine, RealtimeContext, TimecodeInputConfig};
use dub_io::Track;
use dub_timecode::signal::Generator;
use dub_timecode::Format;
use ringbuf::traits::{Producer as _, Split as _};
use ringbuf::HeapRb;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;
const N_CYCLES: usize = 30;

/// Cartridge physics: output level scales with |groove velocity|.
/// Unity play ≈ 0.35 peak; clamp to the engine's comfortable headroom.
fn cartridge_amplitude(rate: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let a = (0.35 * rate.abs()) as f32;
    a.clamp(0.0, 0.5)
}

/// One linear segment of the stroke profile: rate ramps `r0 → r1`
/// over `dur` seconds.
#[derive(Clone, Copy)]
struct Seg {
    r0: f64,
    r1: f64,
    dur: f64,
}

/// Build one scratch cycle, net groove displacement zero by
/// construction: push ramps 0→+p (60 ms), holds, ramps back (60 ms);
/// draw ramps 0→−d (80 ms), holds, ramps back (80 ms). The draw hold
/// `td` is solved so push and draw displace equal groove distance.
fn build_cycle(p: f64, d: f64) -> Vec<Seg> {
    const PUSH_RAMP: f64 = 0.060;
    const DRAW_RAMP: f64 = 0.080;
    const PUSH_HOLD: f64 = 0.300;
    // Push displacement: ramps contribute p·ramp/2 each.
    let push_disp = p * (PUSH_HOLD + PUSH_RAMP);
    let td = push_disp / d - DRAW_RAMP;
    assert!(td > 0.0, "draw hold must be positive (p={p}, d={d})");
    vec![
        Seg {
            r0: 0.0,
            r1: p,
            dur: PUSH_RAMP,
        },
        Seg {
            r0: p,
            r1: p,
            dur: PUSH_HOLD,
        },
        Seg {
            r0: p,
            r1: 0.0,
            dur: PUSH_RAMP,
        },
        Seg {
            r0: 0.0,
            r1: -d,
            dur: DRAW_RAMP,
        },
        Seg {
            r0: -d,
            r1: -d,
            dur: td,
        },
        Seg {
            r0: -d,
            r1: 0.0,
            dur: DRAW_RAMP,
        },
    ]
}

fn cycle_duration(segs: &[Seg]) -> f64 {
    segs.iter().map(|s| s.dur).sum()
}

/// Piecewise-linear rate at time `t` within the cycle.
fn rate_at(segs: &[Seg], t: f64) -> f64 {
    let mut t0 = 0.0;
    for s in segs {
        if t < t0 + s.dur {
            let frac = (t - t0) / s.dur;
            return s.r0 + (s.r1 - s.r0) * frac;
        }
        t0 += s.dur;
    }
    0.0
}

struct ProfileResult {
    engaged_ever: bool,
    start_pos_frames: f64,
    cycle_positions: Vec<f64>,
}

/// Full-pipeline run: fresh engine, app-default thresholds, 120 s
/// track seeked to the middle, then `N_CYCLES` zero-net-displacement
/// scratch cycles driven block by block.
fn run_profile(p: f64, d: f64) -> ProfileResult {
    let mut engine = Engine::new(SR, BLOCK);
    let rb = HeapRb::<f32>::new(48_000 * 2);
    let (mut tx, rx) = rb.split();
    // App-default thresholds (engage 0.8 / disengage 0.5 / sticky 4 /
    // amplitude floor 0.01). The tight non-default thresholds used by
    // the unit tests in lib.rs would hide the policy behavior we're
    // reproducing here.
    let cfg = TimecodeInputConfig {
        hold_until_calibrated: false,
        ..TimecodeInputConfig::default()
    };
    engine
        .attach_timecode_input(0, rx, cfg)
        .expect("attach should succeed");

    let track = Arc::new(
        Track::from_interleaved(vec![0.5_f32; 48_000 * 2 * 120], 48_000, 2)
            .expect("track construction"),
    );
    engine.deck_mut(0).set_source(track);
    // Middle of the 120 s track so backward motion can't clamp at 0.
    engine.deck_mut(0).set_position_frames(f64::from(SR) * 60.0);

    let mut gen = Generator::new(Format::SeratoCv02, SR);
    let mut rt = RealtimeContext::new();
    let mut sig = vec![0.0_f32; BLOCK * 2];
    let mut out = vec![0.0_f32; BLOCK * 2];

    let segs = build_cycle(p, d);
    let block_dt = f64::from(BLOCK as u32) / f64::from(SR);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let blocks_per_cycle = (cycle_duration(&segs) / block_dt).round() as usize;

    let mut engaged_ever = false;
    let start_pos_frames = engine.deck(0).position_frames();
    let mut cycle_positions = Vec::with_capacity(N_CYCLES);

    for _cycle in 0..N_CYCLES {
        // Exact groove-displacement integral for this cycle; the final
        // block's rate is corrected so the cycle sum is exactly zero —
        // the generator's phase (groove position) returns to its start.
        let mut disp = 0.0_f64;
        for i in 0..blocks_per_cycle {
            let rate = if i + 1 == blocks_per_cycle {
                -disp / block_dt
            } else {
                #[allow(clippy::cast_precision_loss)]
                rate_at(&segs, (i as f64 + 0.5) * block_dt)
            };
            disp += rate * block_dt;

            gen.render(&mut sig, rate, cartridge_amplitude(rate));
            let pushed = tx.push_slice(&sig);
            assert_eq!(pushed, sig.len(), "input ring must keep up");
            engine.render(&mut rt, &mut out);
            engaged_ever |= engine.deck(0).is_playing();
        }
        cycle_positions.push(engine.deck(0).position_frames());
    }

    ProfileResult {
        engaged_ever,
        start_pos_frames,
        cycle_positions,
    }
}

fn frames_to_ms(frames: f64) -> f64 {
    frames / f64::from(SR) * 1000.0
}

/// Block-by-block trace of profile A's third cycle: where exactly in
/// the stroke does the playhead error accumulate, and what was the
/// decoder/policy doing at that moment. Diagnostic; always passes.
#[test]
fn profile_a_block_trace() {
    let (p, d) = (2.0, 0.50);
    let mut engine = Engine::new(SR, BLOCK);
    let rb = HeapRb::<f32>::new(48_000 * 2);
    let (mut tx, rx) = rb.split();
    let cfg = TimecodeInputConfig {
        hold_until_calibrated: false,
        ..TimecodeInputConfig::default()
    };
    engine.attach_timecode_input(0, rx, cfg).expect("attach");
    let track = Arc::new(
        Track::from_interleaved(vec![0.5_f32; 48_000 * 2 * 120], 48_000, 2).expect("track"),
    );
    engine.deck_mut(0).set_source(track);
    engine.deck_mut(0).set_position_frames(f64::from(SR) * 60.0);

    let mut gen = Generator::new(Format::SeratoCv02, SR);
    let mut rt = RealtimeContext::new();
    let mut sig = vec![0.0_f32; BLOCK * 2];
    let mut out = vec![0.0_f32; BLOCK * 2];
    let segs = build_cycle(p, d);
    let block_dt = f64::from(BLOCK as u32) / f64::from(SR);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let blocks_per_cycle = (cycle_duration(&segs) / block_dt).round() as usize;

    let start = engine.deck(0).position_frames();
    let mut groove = 0.0_f64; // integral of true rate, in seconds
    println!("\n=== Profile A block trace (cycles 3-4) ===");
    println!(
        "{:>7} {:>7} | {:>6} {:>7} | {:>5} {:>7} | {:>9}",
        "t(s)", "true_r", "conf", "dec_r", "play", "deck_r", "err(ms)"
    );
    for cycle in 0..4 {
        let mut disp = 0.0_f64;
        for i in 0..blocks_per_cycle {
            let rate = if i + 1 == blocks_per_cycle {
                -disp / block_dt
            } else {
                #[allow(clippy::cast_precision_loss)]
                rate_at(&segs, (i as f64 + 0.5) * block_dt)
            };
            disp += rate * block_dt;
            gen.render(&mut sig, rate, cartridge_amplitude(rate));
            tx.push_slice(&sig);
            engine.render(&mut rt, &mut out);
            groove += rate * block_dt;
            if cycle >= 2 {
                let dec = engine.timecode_last_output(0);
                let (conf, dec_r) = dec.map_or((0.0, f64::NAN), |o| (o.confidence, o.rate));
                let err_ms =
                    ((engine.deck(0).position_frames() - start) / f64::from(SR) - groove) * 1000.0;
                #[allow(clippy::cast_precision_loss)]
                let t = (cycle * blocks_per_cycle + i) as f64 * block_dt;
                println!(
                    "{:>7.3} {:>+7.3} | {:>6.3} {:>+7.3} | {:>5} {:>+7.3} | {:>+9.2}",
                    t,
                    rate,
                    conf,
                    dec_r,
                    engine.deck(0).is_playing(),
                    engine.deck(0).rate(),
                    err_ms
                );
            }
        }
    }
}

#[test]
fn scratch_in_place_forward_drift() {
    let profiles: [(&str, f64, f64); 5] = [
        ("A (moderate)      P=2.0 D=0.50", 2.0, 0.50),
        ("B (slow draw)     P=2.0 D=0.20", 2.0, 0.20),
        ("C (v.slow draw)   P=2.0 D=0.08", 2.0, 0.08),
        ("D (hard push)     P=3.0 D=0.50", 3.0, 0.50),
        ("E (gentle ctrl)   P=0.5 D=0.50", 0.5, 0.50),
    ];

    let mut any_engaged = false;
    let mut rows = Vec::new();

    for (name, p, d) in profiles {
        let res = run_profile(p, d);
        any_engaged |= res.engaged_ever;
        let end = *res.cycle_positions.last().expect("at least one cycle ran");
        let total_ms = frames_to_ms(end - res.start_pos_frames);
        #[allow(clippy::cast_precision_loss)]
        let per_cycle_ms = total_ms / N_CYCLES as f64;
        rows.push((name, p, d, total_ms, per_cycle_ms, res.engaged_ever));

        if name.starts_with("B") {
            println!("\nProfile B per-cycle playhead positions (s):");
            println!("  start: {:>10.4}", res.start_pos_frames / f64::from(SR));
            for (i, pos) in res.cycle_positions.iter().enumerate() {
                let drift_ms = frames_to_ms(pos - res.start_pos_frames);
                println!(
                    "  cycle {:>2}: {:>10.4}  (cum drift {:>+9.2} ms)",
                    i + 1,
                    pos / f64::from(SR),
                    drift_ms
                );
            }
        }
    }

    println!("\n=== Scratch-in-place drift, {N_CYCLES} cycles, net groove displacement zero ===");
    println!("  positive = playhead drifts FORWARD relative to the groove\n");
    println!(
        "  {:<32} {:>14} {:>16} {:>9}",
        "profile", "total drift", "drift/cycle", "engaged"
    );
    for (name, _p, _d, total_ms, per_cycle_ms, engaged) in &rows {
        println!(
            "  {:<32} {:>+11.2} ms {:>+13.2} ms {:>9}",
            name, total_ms, per_cycle_ms, engaged
        );
    }
    println!();

    assert!(
        any_engaged,
        "pipeline never engaged on any profile — harness is not driving the deck"
    );

    // Regression bounds — the on-rig symptom was +30 ms/cycle on
    // profile B (and +742 on C) before the 64-frame sub-block decode +
    // velocity-honest amplitude fixes. Anything approaching those
    // numbers again is the bug coming back, not noise.
    for (name, _p, _d, _total, per_cycle_ms, _engaged) in &rows {
        let bound = if name.starts_with('C') { 8.0 } else { 2.0 };
        assert!(
            per_cycle_ms.abs() < bound,
            "scratch drift regression on {name}: {per_cycle_ms:+.2} ms/cycle (bound ±{bound})"
        );
    }
}
