//! Standalone RT-safety auditor.
//!
//! Hammers the engine's render path under `assert_no_alloc`. Aborts the
//! process if any allocation is observed during rendering.
//!
//! This is the binary form of the CI gate described in PRD §2.2.3. It runs
//! both as a pre-commit check (via `make rt-audit`) and as a CI step.

use std::hint::black_box;
use std::process::ExitCode;
use std::time::Instant;

use std::sync::Arc;

use anyhow::Result;
use assert_no_alloc::AllocDisabler;
use dub_engine::{Engine, RealtimeContext, TimecodeInputConfig};
use dub_io::Track;
use ringbuf::traits::{Producer as _, Split as _};
use ringbuf::HeapRb;

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rt-audit FAILED: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    const BLOCKS: u64 = 100_000;
    const SAMPLE_RATE: f32 = 48_000.0;
    const BLOCK_SIZE: usize = 64;
    const COMMANDS_PER_INTERVAL: u64 = 100;

    println!(
        "rt-audit: rendering {BLOCKS} blocks of {BLOCK_SIZE} stereo frames @ {SAMPLE_RATE} Hz \
         (with command-channel drain + trash routing)"
    );

    // Build the production engine variant — with a command channel — so
    // the drain path is part of the audit. Pre-stage commands periodically
    // to make sure draining itself is alloc-free.
    let (mut engine, mut handle) = Engine::new_with_handle(SAMPLE_RATE, BLOCK_SIZE);
    let mut buffer = vec![0.0f32; 2 * BLOCK_SIZE];
    let mut rt = RealtimeContext::new();

    // Pre-decoded fake tracks for the hot-swap test path. Allocations
    // here are pre-loop and outside `assert_no_alloc`. Inside the loop
    // we only send pre-cloned `Arc` values — `Arc::clone` is alloc-free.
    let track_a = Arc::new(Track::from_interleaved(vec![0.1f32; 16], 48_000, 2).unwrap());
    let track_b = Arc::new(Track::from_interleaved(vec![0.2f32; 16], 48_000, 2).unwrap());

    let start = Instant::now();
    let mut total_commands_sent: u64 = 0;
    let mut total_loads_sent: u64 = 0;
    let mut total_master_changes: u64 = 0;
    assert_no_alloc::assert_no_alloc(|| {
        for i in 0..BLOCKS {
            // Every ~1000 blocks, push a small burst of commands —
            // alternating decks so the audit covers both the deck-A and
            // deck-B paths through `apply_command`. Sending through
            // `EngineHandle` MUST be alloc-free (try_push on a pre-allocated
            // ringbuf, no boxing).
            if i.is_multiple_of(1_000) {
                for j in 0..COMMANDS_PER_INTERVAL {
                    let deck = (j as usize) & 1;
                    if handle.deck(deck).set_gain(0.5).is_ok() {
                        total_commands_sent += 1;
                    }
                }
            }
            // Master-gain churn (M4) — engine-wide command, no deck.
            // Toggle between two values every ~1500 blocks to make sure
            // the master path stays alloc-free under sustained traffic.
            if i.is_multiple_of(1_500) {
                let g = if i.is_multiple_of(3_000) { 1.0 } else { 0.7 };
                if handle.set_master_gain(g).is_ok() {
                    total_master_changes += 1;
                }
            }
            // Every ~5000 blocks, hot-load a track on each deck —
            // alternating which deck and which track. This exercises both
            // decks' load command path (sender: try_push of an Arc<Track>)
            // and the trash channel (audio thread: take old Arc, push
            // it back through trash; main thread: reclaim drops it).
            if i.is_multiple_of(5_000) {
                let target_deck = (i / 5_000) as usize & 1;
                let next = if i.is_multiple_of(10_000) {
                    track_a.clone()
                } else {
                    track_b.clone()
                };
                if handle.deck(target_deck).load(next, 1.0).is_ok() {
                    total_loads_sent += 1;
                }
            }
            engine.render(&mut rt, &mut buffer);
            // Defeat dead-code elimination so the render call isn't
            // optimized away in release. This is essential for honest
            // performance measurement.
            black_box(&buffer);
        }
    });
    let elapsed = start.elapsed();
    println!(
        "rt-audit: drained {total_commands_sent} cmds + {total_loads_sent} hot-loads \
         + {total_master_changes} master-gain changes during render"
    );
    let overflow = handle.trash_overflow_count();
    if overflow > 0 {
        anyhow::bail!("trash channel overflowed {overflow} times during audit");
    }

    let total_seconds = (BLOCKS as f32 * BLOCK_SIZE as f32) / SAMPLE_RATE;
    let realtime_factor = total_seconds / elapsed.as_secs_f32();

    println!(
        "rt-audit OK: {BLOCKS} blocks rendered in {:.3} ms (×{realtime_factor:.0} realtime)",
        elapsed.as_secs_f64() * 1000.0
    );

    // ============================================================
    //   M5.3 timecode-driven render path
    // ============================================================
    //
    // Build a fresh engine, attach a synthetic timecode input to deck
    // 0, prime the input ring with N blocks of forward-unity carrier,
    // then render with the producer continuously refilling. The render
    // path now exercises:
    //
    //   - Engine::drive_timecode_inputs (M5.3)
    //   - Decoder::process (M5.1) on the audio thread
    //   - Deck::set_rate / set_playing under decoder control
    //
    // Any allocation in those paths shows up here.
    println!();
    println!(
        "rt-audit: timecode path — rendering {} blocks with synthetic carrier on deck 0",
        TC_BLOCKS
    );
    let tc_elapsed = run_timecode_audit()?;
    let tc_total_secs = (TC_BLOCKS as f32 * BLOCK_SIZE as f32) / SAMPLE_RATE;
    let tc_factor = tc_total_secs / tc_elapsed.as_secs_f32();
    println!(
        "rt-audit OK: timecode path {TC_BLOCKS} blocks in {:.3} ms (\u{00d7}{tc_factor:.0} realtime)",
        tc_elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Number of blocks to render through the timecode path. Smaller than
/// the main loop because we also re-render the synthetic carrier on
/// the *producer* side (off-RT but still in-process), and we don't
/// need 100k blocks to surface an allocation regression.
const TC_BLOCKS: u64 = 10_000;

fn run_timecode_audit() -> Result<std::time::Duration> {
    const SAMPLE_RATE: f32 = 48_000.0;
    const BLOCK_SIZE: usize = 64;

    let mut engine = Engine::new(SAMPLE_RATE, BLOCK_SIZE);
    // Seed deck 0 with a constant-value track so we can let the
    // decoder freely drive transport without worrying about the
    // playhead leaving the source.
    let track = Arc::new(Track::from_interleaved(vec![0.1_f32; 48_000 * 2], 48_000, 2).unwrap());
    engine.deck_mut(0).set_source(track);

    // Build a 2-second-deep input ring. Deep enough that the producer
    // can refill ahead of the consumer without ever overflowing.
    let rb = HeapRb::<f32>::new((SAMPLE_RATE as usize) * 2 * 2);
    let (mut tx, rx) = rb.split();
    engine.attach_timecode_input(
        0,
        rx,
        TimecodeInputConfig {
            format: dub_timecode::Format::SeratoCv02,
            input_sample_rate: SAMPLE_RATE,
            max_block_frames: BLOCK_SIZE * 4,
            confidence_threshold: 0.7,
            disengage_threshold: 0.5,
            sticky_blocks_to_disengage: 1,
            amplitude_threshold: 0.001,
        },
    )?;

    let mut gen =
        dub_timecode::signal::Generator::new(dub_timecode::Format::SeratoCv02, SAMPLE_RATE);
    let mut sig = vec![0.0_f32; BLOCK_SIZE * 2];
    let mut buffer = vec![0.0_f32; BLOCK_SIZE * 2];
    let mut rt = RealtimeContext::new();

    // Prime: render once so the decoder is past its first-block prime
    // (avoids the false-positive of "first call allocates" on prev_*
    // that doesn't actually allocate but might fool a measurement).
    gen.render(&mut sig, 1.0, 0.5);
    let _ = tx.push_slice(&sig);
    engine.render(&mut rt, &mut buffer);

    let start = Instant::now();
    assert_no_alloc::assert_no_alloc(|| {
        for _ in 0..TC_BLOCKS {
            // Generator.render is alloc-free (M5.1 verified). Push
            // into the SPSC ring is alloc-free.
            gen.render(&mut sig, 1.0, 0.5);
            let _ = tx.push_slice(&sig);
            engine.render(&mut rt, &mut buffer);
            black_box(&buffer);
        }
    });
    Ok(start.elapsed())
}
