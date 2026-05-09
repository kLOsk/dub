//! `dub timecode-deck <track> --input-channels N,M [...]` —
//! the M5.3 live wiring demo.
//!
//! Wires:
//!
//! 1. `dub_audio::AudioInput` on the chosen input device (channel-mapped,
//!    e.g. SL3 deck A on `--input-channels 3,4`),
//! 2. The IOProc → consumer ringbuf moved into the engine via
//!    [`dub_engine::Engine::attach_timecode_input`],
//! 3. A track loaded onto deck 0 (off the audio thread),
//! 4. `dub_audio::AudioOutput` running the engine on the CoreAudio
//!    render thread.
//!
//! Result: real-platter timecode drives a loaded track in real time —
//! forward play plays forward, scratching scratches, lifting the
//! stylus mutes the deck.
//!
//! What this is **not**: a UI, a mixer, a calibration tool, or a
//! correctness reference. It's the smallest possible "make sound come
//! out from real timecode" rig so we can validate the live integration
//! before any of those higher-level concerns land. Stickiness on lift
//! is M5.4; multi-deck routing and external-mixer output is M5.5.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use dub_audio::AudioInput;
use dub_engine::{
    Engine, TimecodeInputConfig, DEFAULT_AMPLITUDE_THRESHOLD, DEFAULT_CONFIDENCE_THRESHOLD,
    DEFAULT_DISENGAGE_THRESHOLD, DEFAULT_STICKY_BLOCKS_TO_DISENGAGE,
};
use dub_io::Track;
use dub_timecode::Format;

use crate::input_cmds::{parse_input_args, InputArgs};

/// Default duration if `--duration` isn't given. 60 s is comfortably
/// long for a tactile validation run; the user can Ctrl-C earlier.
const DEFAULT_RUN_SECS: f64 = 60.0;

/// CLI options for `dub timecode-deck`. Built on top of the shared
/// [`InputArgs`] so the `--input-channels`/`--device`/`--sr` flags
/// are identical to `dub levels` and `dub capture`.
struct Opts {
    track: PathBuf,
    input: InputArgs,
    /// Output buffer size hint for CoreAudio output (frames). Smaller
    /// = lower output latency. None means "device default".
    output_buffer_size: Option<u32>,
    /// Upper hysteresis edge (engage threshold).
    confidence: f32,
    /// Lower hysteresis edge (disengage threshold).
    disengage: f32,
    /// Consecutive sub-disengage blocks required to disengage.
    sticky_blocks: u32,
    /// RMS floor below which the carrier is treated as dead.
    amplitude_threshold: f32,
    /// Wall-clock duration to run before stopping. Distinct from
    /// `InputArgs::duration` because we want timecode-deck to default
    /// to 60 s, not the 5 s default of capture/levels.
    duration_secs: f64,
}

fn parse_opts(args: &[String]) -> Result<Opts> {
    // Pull --confidence and --output-buffer-size out before delegating
    // to the shared input-args parser; everything else (device, channels,
    // input-channels, sr, duration) goes through the shared path.
    let mut filtered: Vec<String> = Vec::with_capacity(args.len());
    let mut confidence: f32 = DEFAULT_CONFIDENCE_THRESHOLD;
    let mut disengage: f32 = DEFAULT_DISENGAGE_THRESHOLD;
    let mut sticky_blocks: u32 = DEFAULT_STICKY_BLOCKS_TO_DISENGAGE;
    let mut amplitude_threshold: f32 = DEFAULT_AMPLITUDE_THRESHOLD;
    let mut output_buffer_size: Option<u32> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--confidence" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--confidence expects a number"))?;
                confidence = v
                    .parse::<f32>()
                    .with_context(|| format!("--confidence {v}"))?;
                i += 2;
            }
            "--disengage-threshold" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--disengage-threshold expects a number"))?;
                disengage = v
                    .parse::<f32>()
                    .with_context(|| format!("--disengage-threshold {v}"))?;
                i += 2;
            }
            "--sticky-blocks" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--sticky-blocks expects an integer"))?;
                sticky_blocks = v
                    .parse::<u32>()
                    .with_context(|| format!("--sticky-blocks {v}"))?;
                i += 2;
            }
            "--amplitude-threshold" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--amplitude-threshold expects a number"))?;
                amplitude_threshold = v
                    .parse::<f32>()
                    .with_context(|| format!("--amplitude-threshold {v}"))?;
                i += 2;
            }
            "--output-buffer-size" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--output-buffer-size expects an integer"))?;
                output_buffer_size = Some(
                    v.parse::<u32>()
                        .with_context(|| format!("--output-buffer-size {v}"))?,
                );
                i += 2;
            }
            _ => {
                filtered.push(args[i].clone());
                i += 1;
            }
        }
    }
    let (input, leftover) = parse_input_args(&filtered)?;
    let positional: Vec<&String> = leftover.iter().filter(|s| !s.starts_with("--")).collect();
    if positional.is_empty() {
        return Err(anyhow!(
            "usage: dub timecode-deck <track.wav> --input-channels N,M [--device NAME] \
             [--duration SECS] [--confidence T] [--disengage-threshold T] \
             [--sticky-blocks N] [--amplitude-threshold T] \
             [--output-buffer-size FRAMES]"
        ));
    }
    if positional.len() > 1 {
        return Err(anyhow!(
            "timecode-deck takes a single track path; got {}",
            positional.len()
        ));
    }
    if let Some(unknown) = leftover.iter().find(|s| s.starts_with("--")) {
        return Err(anyhow!("unknown flag: {unknown}"));
    }
    Ok(Opts {
        track: PathBuf::from(positional[0]),
        duration_secs: input.duration.unwrap_or(DEFAULT_RUN_SECS),
        input,
        output_buffer_size,
        confidence,
        disengage,
        sticky_blocks,
        amplitude_threshold,
    })
}

/// Entry point dispatched from `main`.
///
/// # Errors
/// Track decode, audio device open, attach errors, or HAL failures.
pub fn run(args: &[String]) -> Result<()> {
    let opts = parse_opts(args)?;

    // 1. Load the track off the audio thread.
    let track = Track::load_from_path(&opts.track)
        .with_context(|| format!("loading track {}", opts.track.display()))?;
    println!(
        "track:        {} ({} frames @ {} Hz, {} ch, {:.3} s)",
        opts.track.display(),
        track.frames(),
        track.sample_rate(),
        track.channels(),
        track.duration_seconds()
    );

    // 2. Open the input device. SR will be the device's hardware
    //    nominal — we lean on dub-audio's M5.2 invariant that the
    //    AudioUnit and device agree.
    let input_opts = opts.input.to_options();
    let mut input =
        AudioInput::start_with_options(&input_opts).context("opening input device for timecode")?;
    let input_sr = input.sample_rate();
    println!(
        "input:        device='{}' sr={input_sr} Hz channels={} buffer={} frames",
        input.device_name(),
        input.channels(),
        input.buffer_frames(),
    );

    // 3. The engine MUST run at the input SR for v1 (no SR conversion
    //    between input and engine). The output device gets aligned to
    //    the same SR by `AudioOutput::start_with_buffer_size` below;
    //    we just print the *current* nominal here for the user's
    //    reference. If the output device can't honor `engine_sr`
    //    `AudioOutput` will fail loudly rather than ship audible drift.
    let device = dub_audio::query_default_output().context("querying default output")?;
    if (device.sample_rate - input_sr).abs() > 0.5 {
        println!(
            "note: output device currently at {} Hz, will be retuned to {input_sr} Hz \
             (engine SR) so playback runs on a single clock — no SRC.",
            device.sample_rate
        );
    }
    let engine_sr = input_sr;
    let engine_block = 256_usize;
    let mut engine = Engine::new(engine_sr, engine_block);
    println!(
        "engine:       sr={engine_sr} Hz block={engine_block} frames\n\
         output:       device sr={} Hz (target {engine_sr} Hz) buffer={} frames",
        device.sample_rate, device.buffer_frames,
    );

    // 4. Configure deck 0 with the track. Crucially we do NOT
    //    set_playing(true) — the decoder will do that on the first
    //    locked block (see `Engine::drive_timecode_inputs`).
    {
        let deck = engine.deck_mut(0);
        deck.set_source(Arc::new(track));
        deck.set_gain(1.0);
    }

    // 5. Hand the input ringbuf consumer to the engine.
    let rx = input
        .take_consumer()
        .ok_or_else(|| anyhow!("AudioInput consumer already taken"))?;
    let cfg = TimecodeInputConfig {
        format: Format::SeratoCv02,
        input_sample_rate: input_sr,
        // CoreAudio output blocks vary; 4096 is a safe upper bound.
        max_block_frames: 4096,
        confidence_threshold: opts.confidence,
        disengage_threshold: opts.disengage,
        sticky_blocks_to_disengage: opts.sticky_blocks,
        amplitude_threshold: opts.amplitude_threshold,
    };
    engine
        .attach_timecode_input(0, rx, cfg)
        .context("attaching timecode input to deck 0")?;
    println!(
        "timecode:     format=SeratoCv02 engage={:.2} disengage={:.2} \
         sticky={} blocks amp_floor={:.4}",
        opts.confidence, opts.disengage, opts.sticky_blocks, opts.amplitude_threshold,
    );

    // 6. Move the engine onto the audio thread. From here, AudioOutput
    //    drives Engine::render which drives the decoder which drives
    //    deck transport — no main-thread participation in the audio path.
    let output = dub_audio::AudioOutput::start_with_buffer_size(engine, opts.output_buffer_size)
        .context("starting CoreAudio output for timecode-deck")?;
    let achieved = output.buffer_frames();
    let latency_ms = output.latency_seconds() * 1000.0;
    println!("output buffer: {achieved} frames -> {latency_ms:.2} ms one-way latency");
    println!();
    println!(
        "running for {:.1} s — drop the needle and play.",
        opts.duration_secs
    );
    println!("(Ctrl-C to stop early)");

    // 7. Sleep the wall-clock duration, sampling stats every 0.5 s so
    //    the user gets live feedback.
    let start = Instant::now();
    let total = Duration::from_secs_f64(opts.duration_secs);
    let mut next_tick = start + Duration::from_millis(500);
    while start.elapsed() < total {
        let now = Instant::now();
        if now >= next_tick {
            let cb = output.callback_count();
            let in_cb = input.callback_count();
            let in_of = input.overflow_count();
            print_stats(&output, &input, cb, in_cb, in_of);
            next_tick += Duration::from_millis(500);
        }
        // Coarse sleep — the polling rate above is ≥ 2 Hz.
        thread::sleep(Duration::from_millis(50));
    }

    // 8. Final summary.
    let elapsed = start.elapsed().as_secs_f64();
    let cb = output.callback_count();
    let in_cb = input.callback_count();
    let in_of = input.overflow_count();
    println!();
    println!("done — {elapsed:.3} s wall");
    println!("  output callbacks: {cb}");
    println!("  input  callbacks: {in_cb} (overflow={in_of})");
    if cb == 0 {
        anyhow::bail!("CoreAudio output never fired a callback — device probably failed");
    }
    if in_cb == 0 {
        anyhow::bail!(
            "input device delivered no callbacks. SR mismatch or TCC permissions? \
             See `dub levels --input-channels {:?}` for a quick check.",
            opts.input.input_channels.as_deref().unwrap_or(&[1, 2])
        );
    }
    println!("OK");
    Ok(())
}

fn print_stats(
    output: &dub_audio::AudioOutput,
    input: &AudioInput,
    out_cb: u64,
    in_cb: u64,
    in_of: u64,
) {
    // Single-line refresh on stderr — keeps stdout clean for `tee`.
    let buf_ms = (f64::from(output.buffer_frames()) / f64::from(output.sample_rate())) * 1000.0;
    let avail_frames = (input.available() as f64) / f64::from(input.channels().max(1));
    eprintln!(
        "  out_cb={out_cb} buf={buf_ms:.2}ms in_cb={in_cb} in_overflow={in_of} \
         in_buffered={avail_frames:.0} frames"
    );
}
