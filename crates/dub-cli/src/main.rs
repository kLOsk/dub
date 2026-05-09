//! Headless smoke test + offline render harness for the Dub engine.
//!
//! Subcommands:
//!
//! - `smoke` — verify the engine constructs and renders a block of silence.
//! - `rt-audit` — render N blocks and print the wall-clock time + tick count.
//! - `version` — print engine + ffi version.
//! - `play <input> [-o <output>] [--rate R] [--gain G] [--sr ENGINE_SR]
//!         [--block-size N]` — load `<input>`, render through the engine
//!   into `<output>` (default: `<input>.dub.wav`). Proves the engine
//!   pipeline works end-to-end without needing CoreAudio. Real-time
//!   playback through CoreAudio lands in M1.4.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use dub_engine::{Engine, RealtimeContext};
use dub_io::Track;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("smoke");

    let result = match cmd {
        "smoke" => smoke(),
        "rt-audit" => rt_audit(),
        "version" => version(),
        "play" => play(&args[2..]),
        "help" | "-h" | "--help" => {
            print_help();
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            print_help();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    eprintln!("usage: dub <subcommand> [args]");
    eprintln!();
    eprintln!("subcommands:");
    eprintln!("  smoke        engine handshake + zero-render");
    eprintln!("  rt-audit     stress the render path under assert_no_alloc");
    eprintln!("  version      print versions");
    eprintln!("  play <input> [-o <output>] [--rate R] [--gain G] [--sr SR]");
    eprintln!("               [--block-size N]");
    eprintln!("                offline-render <input> through the engine to <output>");
}

fn smoke() -> Result<()> {
    println!("Dub CLI smoke test");
    println!("  engine version: {}", dub_engine::VERSION);
    println!("  io version:     {}", dub_io::VERSION);
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
    println!("dub-cli   {}", env!("CARGO_PKG_VERSION"));
    println!("dub-engine {}", dub_engine::VERSION);
    println!("dub-io    {}", dub_io::VERSION);
    println!("dub-ffi   {}", dub_ffi::FFI_VERSION);
    Ok(())
}

#[derive(Debug, Default)]
struct PlayOpts {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    rate: f64,
    gain: f32,
    engine_sr: f32,
    block_size: usize,
}

impl PlayOpts {
    fn parse(args: &[String]) -> Result<Self> {
        let mut opts = Self {
            rate: 1.0,
            gain: 1.0,
            engine_sr: 48_000.0,
            block_size: 64,
            ..Self::default()
        };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-o" | "--output" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| anyhow!("--output expects a value"))?;
                    opts.output = Some(PathBuf::from(v));
                    i += 2;
                }
                "--rate" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| anyhow!("--rate expects a value"))?;
                    opts.rate = v.parse().context("--rate not a number")?;
                    i += 2;
                }
                "--gain" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| anyhow!("--gain expects a value"))?;
                    opts.gain = v.parse().context("--gain not a number")?;
                    i += 2;
                }
                "--sr" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| anyhow!("--sr expects a value"))?;
                    opts.engine_sr = v.parse().context("--sr not a number")?;
                    i += 2;
                }
                "--block-size" => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| anyhow!("--block-size expects a value"))?;
                    opts.block_size = v.parse().context("--block-size not a number")?;
                    i += 2;
                }
                s if s.starts_with('-') => {
                    return Err(anyhow!("unknown flag: {s}"));
                }
                _ => {
                    if opts.input.is_none() {
                        opts.input = Some(PathBuf::from(&args[i]));
                    } else {
                        return Err(anyhow!("unexpected positional arg: {}", args[i]));
                    }
                    i += 1;
                }
            }
        }
        Ok(opts)
    }
}

fn default_output_path(input: &Path) -> PathBuf {
    let mut out = input.to_path_buf();
    let stem = input.file_stem().map_or_else(
        || std::ffi::OsString::from("dub"),
        std::ffi::OsStr::to_os_string,
    );
    let mut name = stem;
    name.push(".dub.wav");
    out.set_file_name(name);
    out
}

fn play(args: &[String]) -> Result<()> {
    let opts = PlayOpts::parse(args)?;
    let input = opts
        .input
        .as_ref()
        .ok_or_else(|| anyhow!("usage: dub play <input> [-o <output>] [--rate R] ..."))?;
    let output = opts
        .output
        .clone()
        .unwrap_or_else(|| default_output_path(input));

    println!("Dub CLI offline play");
    println!("  input:        {}", input.display());
    println!("  output:       {}", output.display());
    println!("  engine SR:    {} Hz", opts.engine_sr);
    println!("  block size:   {} frames", opts.block_size);
    println!("  rate:         {}", opts.rate);
    println!("  gain:         {}", opts.gain);

    let track = Track::load_from_path(input).context("loading input")?;
    println!(
        "  track:        {} frames @ {} Hz, {} ch ({:.3} s)",
        track.frames(),
        track.sample_rate(),
        track.channels(),
        track.duration_seconds()
    );

    // Offline render: generate enough output frames to cover the track at
    // the user's requested rate. Negative rates render in reverse from the
    // end of the track.
    let track = std::sync::Arc::new(track);
    let mut engine = Engine::new(opts.engine_sr, opts.block_size);
    engine.deck_mut(0).set_source(track.clone());
    engine.deck_mut(0).set_gain(opts.gain);
    engine.deck_mut(0).set_rate(opts.rate);
    engine.deck_mut(0).set_playing(true);

    if opts.rate < 0.0 {
        engine
            .deck_mut(0)
            .set_position_frames((track.frames() - 1) as f64);
    }

    let abs_rate = opts.rate.abs().max(1e-12);
    // Output frames needed to cover the track exactly once at rate=R.
    // n_track_frames * (engine_sr / track_sr) / |rate|.
    let track_sr = f64::from(track.sample_rate());
    let engine_sr = f64::from(opts.engine_sr);
    let total_output_frames =
        ((track.frames() as f64) * (engine_sr / track_sr) / abs_rate).ceil() as u64;
    let total_blocks = total_output_frames.div_ceil(opts.block_size as u64);

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: opts.engine_sr.round() as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&output, spec).context("opening output WAV")?;

    let mut buffer = vec![0.0f32; 2 * opts.block_size];
    let mut rt = RealtimeContext::new();
    let mut peak: f32 = 0.0;
    let mut rms_acc: f64 = 0.0;
    let mut n_samples: u64 = 0;

    let start = Instant::now();
    for _ in 0..total_blocks {
        engine.render(&mut rt, &mut buffer);
        for sample in &buffer {
            writer.write_sample(*sample).context("writing sample")?;
            let abs = sample.abs();
            if abs > peak {
                peak = abs;
            }
            rms_acc += f64::from(*sample) * f64::from(*sample);
            n_samples += 1;
        }
    }
    let elapsed = start.elapsed();
    writer.finalize().context("finalizing output WAV")?;

    let rms = (rms_acc / (n_samples as f64).max(1.0)).sqrt();
    let total_output_secs = (n_samples as f64 / 2.0) / engine_sr;
    let realtime_factor = total_output_secs / elapsed.as_secs_f64().max(1e-12);

    println!(
        "  rendered:     {} blocks, {} samples",
        total_blocks, n_samples
    );
    println!("  output dur:   {total_output_secs:.3} s");
    println!("  wall:         {:.3} ms", elapsed.as_secs_f64() * 1000.0);
    println!("  realtime ×{realtime_factor:.0}");
    println!(
        "  peak:         {peak:.4} ({:.2} dBFS)",
        20.0 * peak.log10()
    );
    println!("  rms:          {rms:.4} ({:.2} dBFS)", 20.0 * rms.log10());
    println!("OK");
    Ok(())
}
