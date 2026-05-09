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
use dub_engine::{Engine, RealtimeContext};
use dub_io::Track;

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
    assert_no_alloc::assert_no_alloc(|| {
        for i in 0..BLOCKS {
            // Every ~1000 blocks, push a small burst of commands. Sending
            // through `EngineHandle` MUST be alloc-free — it's a try_push
            // on a pre-allocated ringbuf, no boxing.
            if i.is_multiple_of(1_000) {
                for _ in 0..COMMANDS_PER_INTERVAL {
                    if handle.deck(0).set_gain(0.5).is_ok() {
                        total_commands_sent += 1;
                    }
                }
            }
            // Every ~5000 blocks, hot-load a track. This exercises both
            // the load command (sender side: try_push of an Arc<Track>)
            // and the trash channel (audio thread: take old Arc, push
            // it back through trash; main thread: reclaim drops it).
            if i.is_multiple_of(5_000) {
                let next = if i.is_multiple_of(10_000) {
                    track_a.clone()
                } else {
                    track_b.clone()
                };
                if handle.deck(0).load(next).is_ok() {
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
        "rt-audit: drained {total_commands_sent} cmds + {total_loads_sent} hot-loads during render"
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
    Ok(())
}
