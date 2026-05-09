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

use anyhow::Result;
use assert_no_alloc::AllocDisabler;
use dub_engine::{Engine, RealtimeContext};

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

    println!(
        "rt-audit: rendering {BLOCKS} blocks of {BLOCK_SIZE} stereo frames @ {SAMPLE_RATE} Hz"
    );

    let mut engine = Engine::new(SAMPLE_RATE, BLOCK_SIZE);
    let mut buffer = vec![0.0f32; 2 * BLOCK_SIZE];
    let mut rt = RealtimeContext::new();

    let start = Instant::now();
    assert_no_alloc::assert_no_alloc(|| {
        for _ in 0..BLOCKS {
            engine.render(&mut rt, &mut buffer);
            // Defeat dead-code elimination so the render call isn't
            // optimized away in release. This is essential for honest
            // performance measurement.
            black_box(&buffer);
        }
    });
    let elapsed = start.elapsed();

    let total_seconds = (BLOCKS as f32 * BLOCK_SIZE as f32) / SAMPLE_RATE;
    let realtime_factor = total_seconds / elapsed.as_secs_f32();

    println!(
        "rt-audit OK: {BLOCKS} blocks rendered in {:.3} ms (×{realtime_factor:.0} realtime)",
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}
