//! Headless smoke test for the Dub engine.
//!
//! Two subcommands:
//!
//! - `smoke` — verify the engine constructs and renders a block of silence.
//! - `rt-audit` — render N blocks and print the wall-clock time + tick count.
//!   Sanity check that the RT path is callable from a binary.
//!
//! Real offline-render harnesses (full integration tests against fixtures)
//! land in M2 alongside the soak test infrastructure.

use std::env;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Result;
use dub_engine::{Engine, RealtimeContext};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("smoke");

    let result = match cmd {
        "smoke" => smoke(),
        "rt-audit" => rt_audit(),
        "version" => version(),
        other => {
            eprintln!("unknown subcommand: {other}");
            eprintln!("usage: dub <smoke|rt-audit|version>");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn smoke() -> Result<()> {
    println!("Dub CLI smoke test");
    println!("  engine version: {}", dub_engine::VERSION);
    println!("  ffi version:    {}", dub_ffi::FFI_VERSION);
    println!("  ffi greeting:   {}", dub_ffi::greeting());

    let mut engine = Engine::new(48_000.0, 64);
    let mut buffer = vec![1.0f32; 128];
    let mut rt = RealtimeContext::new();

    engine.render(&mut rt, &mut buffer);

    let nonzero = buffer.iter().filter(|s| **s != 0.0).count();
    if nonzero != 0 {
        anyhow::bail!("expected silent render, got {nonzero} non-zero samples");
    }

    println!("  rendered:       1 block, 64 frames stereo, all-zero output OK");
    println!("OK");
    Ok(())
}

fn rt_audit() -> Result<()> {
    const BLOCKS: u64 = 10_000;
    const SAMPLE_RATE: f32 = 48_000.0;
    const BLOCK_SIZE: usize = 64;

    println!("Dub CLI rt-audit");
    println!("  rendering {BLOCKS} blocks of {BLOCK_SIZE} stereo frames @ {SAMPLE_RATE} Hz");

    let mut engine = Engine::new(SAMPLE_RATE, BLOCK_SIZE);
    let mut buffer = vec![0.0f32; 2 * BLOCK_SIZE];
    let mut rt = RealtimeContext::new();

    let start = Instant::now();
    for _ in 0..BLOCKS {
        engine.render(&mut rt, &mut buffer);
    }
    let elapsed = start.elapsed();

    let total_seconds = (BLOCKS as f32 * BLOCK_SIZE as f32) / SAMPLE_RATE;
    let wall_seconds = elapsed.as_secs_f32();
    let realtime_factor = total_seconds / wall_seconds;

    println!("  ticks observed: {}", rt.ticks());
    println!("  rendered audio: {total_seconds:.3} s");
    println!("  wall time:      {wall_seconds:.6} s");
    println!("  realtime ×{realtime_factor:.0}");
    println!("OK");
    Ok(())
}

fn version() -> Result<()> {
    println!("dub-cli {}", env!("CARGO_PKG_VERSION"));
    println!("dub-engine {}", dub_engine::VERSION);
    println!("dub-ffi {}", dub_ffi::FFI_VERSION);
    Ok(())
}
