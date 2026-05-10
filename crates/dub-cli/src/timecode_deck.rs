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
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::calibrate::{measure_inline, probe_carrier};
use crate::calibration::{
    default_calibration_dir, Calibration, CalibrationThresholds, DEFAULT_FINGERPRINT_TOLERANCE,
};
use crate::device_profiles;
use crate::input_cmds::{parse_input_args, InputArgs};

/// Default duration if `--duration` isn't given. 60 s is comfortably
/// long for a tactile validation run; the user can Ctrl-C earlier.
const DEFAULT_RUN_SECS: f64 = 60.0;

/// Length of the auto-startup carrier probe used to validate the
/// saved fingerprint against the rig in front of the user. 3 s
/// gives stable percentiles within < 1 % of the long-run values
/// from the original calibration, so it's a meaningful comparison
/// without holding the user up.
const PROBE_SECS: f64 = 3.0;

/// Detection timeout for the auto-startup probe. 30 s gives the
/// user time to walk to the turntable and drop the needle without
/// the timecode-deck startup feeling rushed.
const PROBE_DETECT_TIMEOUT_SECS: f64 = 30.0;

/// Length of the auto-startup full calibration phases. Match
/// `dub calibrate`'s defaults — auto-calibration produces a
/// JSON file indistinguishable from a manual `dub calibrate` run.
const AUTO_CARRIER_SECS: f64 = 10.0;
const AUTO_LIFT_SECS: f64 = 5.0;
const AUTO_DETECT_TIMEOUT_SECS: f64 = 30.0;

/// Surface a warning when the saved calibration is older than this.
/// 30 days is long enough that "set up at home, played one gig two
/// weeks ago" doesn't trigger a warning, but short enough to flag
/// "dusty stylus from six months of disuse" before the user gets
/// surprised by ghost noise.
const STALE_CALIBRATION_DAYS: f64 = 30.0;

/// CLI options for `dub timecode-deck`. Built on top of the shared
/// [`InputArgs`] so the `--input-channels`/`--device`/`--sr` flags
/// are identical to `dub levels` and `dub capture`.
struct Opts {
    track: PathBuf,
    input: InputArgs,
    /// Output buffer size hint for CoreAudio output (frames). Smaller
    /// = lower output latency. None means "device default".
    output_buffer_size: Option<u32>,
    /// M5.5.2 routing knobs. None = auto-resolve from the device name
    /// against the known-device table; Some(_) overrides for
    /// unknown devices or for testing alternative routings on a
    /// known one.
    /// Force the M4 internal mixer regardless of detected device.
    /// Mutually exclusive with `--deck-a-out-ch` / `--deck-b-out-ch`
    /// (mixing the two would silently change the routing semantics).
    internal_mixer: bool,
    /// Override the auto-detection by selecting a profile by its
    /// `name_pattern` (e.g. `--device-profile "SL 3"`). Useful when
    /// the user has multiple interfaces connected and the wrong one
    /// is the system default.
    device_profile: Option<String>,
    /// Explicit total output channel count. For unknown devices this
    /// is required when `--deck-a-out-ch`/`--deck-b-out-ch` are
    /// given; for known devices it overrides the profile's default
    /// (rare; mostly for debugging).
    output_channels: Option<u32>,
    /// 1-based first output channel for deck A's stereo pair (e.g.
    /// `--deck-a-out-ch 3` → ch 3+4). Mutually exclusive with
    /// `--internal-mixer`.
    deck_a_out_ch: Option<u32>,
    /// 1-based first output channel for deck B's stereo pair.
    deck_b_out_ch: Option<u32>,
    /// Per-threshold explicit overrides. `None` = auto-resolve from
    /// the saved calibration (or auto-calibrate if missing /
    /// fingerprint mismatch); `Some(v)` = take this value verbatim,
    /// independent of calibration. Partial overrides are supported
    /// so the user can pin one knob (e.g. amplitude=0.05 to test a
    /// loud venue) and let the rest auto-resolve.
    confidence: Option<f32>,
    disengage: Option<f32>,
    sticky_blocks: Option<u32>,
    amplitude_threshold: Option<f32>,
    /// Force fresh full measurement even if a matching calibration
    /// JSON exists. Use after a known cartridge / cabling change.
    recalibrate: bool,
    /// Skip the fingerprint probe at startup. Faster (~3 s saved)
    /// but loses rig-swap detection. Use only when iterating on
    /// other things and you know the rig is unchanged.
    no_probe: bool,
    /// Skip calibration entirely — fall back to the M5.3 defaults
    /// regardless of what's on disk. Mostly useful for regression
    /// testing the M5.3 path or for first-time users who want to
    /// hear the deck immediately without touching the calibrator.
    no_calibrate: bool,
    /// Wall-clock duration to run before stopping. Distinct from
    /// `InputArgs::duration` because we want timecode-deck to default
    /// to 60 s, not the 5 s default of capture/levels.
    duration_secs: f64,
}

fn parse_opts(args: &[String]) -> Result<Opts> {
    // Pull threshold/calibration flags out before delegating to the
    // shared input-args parser; everything else (device, channels,
    // input-channels, sr, duration) goes through the shared path.
    let mut filtered: Vec<String> = Vec::with_capacity(args.len());
    let mut confidence: Option<f32> = None;
    let mut disengage: Option<f32> = None;
    let mut sticky_blocks: Option<u32> = None;
    let mut amplitude_threshold: Option<f32> = None;
    let mut output_buffer_size: Option<u32> = None;
    let mut recalibrate = false;
    let mut no_probe = false;
    let mut no_calibrate = false;
    let mut internal_mixer = false;
    let mut device_profile: Option<String> = None;
    let mut output_channels: Option<u32> = None;
    let mut deck_a_out_ch: Option<u32> = None;
    let mut deck_b_out_ch: Option<u32> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--confidence" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--confidence expects a number"))?;
                confidence = Some(
                    v.parse::<f32>()
                        .with_context(|| format!("--confidence {v}"))?,
                );
                i += 2;
            }
            "--disengage-threshold" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--disengage-threshold expects a number"))?;
                disengage = Some(
                    v.parse::<f32>()
                        .with_context(|| format!("--disengage-threshold {v}"))?,
                );
                i += 2;
            }
            "--sticky-blocks" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--sticky-blocks expects an integer"))?;
                sticky_blocks = Some(
                    v.parse::<u32>()
                        .with_context(|| format!("--sticky-blocks {v}"))?,
                );
                i += 2;
            }
            "--amplitude-threshold" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--amplitude-threshold expects a number"))?;
                amplitude_threshold = Some(
                    v.parse::<f32>()
                        .with_context(|| format!("--amplitude-threshold {v}"))?,
                );
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
            "--recalibrate" => {
                recalibrate = true;
                i += 1;
            }
            "--no-probe" => {
                no_probe = true;
                i += 1;
            }
            "--no-calibrate" => {
                no_calibrate = true;
                i += 1;
            }
            "--internal-mixer" => {
                internal_mixer = true;
                i += 1;
            }
            "--device-profile" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--device-profile expects a name"))?;
                device_profile = Some(v.clone());
                i += 2;
            }
            "--output-channels" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--output-channels expects an integer"))?;
                output_channels = Some(
                    v.parse::<u32>()
                        .with_context(|| format!("--output-channels {v}"))?,
                );
                i += 2;
            }
            "--deck-a-out-ch" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--deck-a-out-ch expects an integer (1-based)"))?;
                deck_a_out_ch = Some(
                    v.parse::<u32>()
                        .with_context(|| format!("--deck-a-out-ch {v}"))?,
                );
                i += 2;
            }
            "--deck-b-out-ch" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--deck-b-out-ch expects an integer (1-based)"))?;
                deck_b_out_ch = Some(
                    v.parse::<u32>()
                        .with_context(|| format!("--deck-b-out-ch {v}"))?,
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
             [--output-buffer-size FRAMES] [--recalibrate] [--no-probe] [--no-calibrate] \
             [--internal-mixer | (--deck-a-out-ch N --deck-b-out-ch N [--output-channels N])] \
             [--device-profile NAME]"
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
    if recalibrate && no_calibrate {
        return Err(anyhow!(
            "--recalibrate and --no-calibrate are mutually exclusive"
        ));
    }
    if internal_mixer && (deck_a_out_ch.is_some() || deck_b_out_ch.is_some()) {
        return Err(anyhow!(
            "--internal-mixer and --deck-a-out-ch / --deck-b-out-ch are mutually \
             exclusive: internal-mixer pins both decks to ch 1+2"
        ));
    }
    if internal_mixer && device_profile.is_some() {
        return Err(anyhow!(
            "--internal-mixer and --device-profile are mutually exclusive"
        ));
    }
    // Mixed-set sanity: one of deck-a/deck-b without the other is
    // almost always a typo. Require both or neither so the routing
    // is symmetric and the user can see what they specified.
    if deck_a_out_ch.is_some() != deck_b_out_ch.is_some() {
        return Err(anyhow!(
            "--deck-a-out-ch and --deck-b-out-ch must be specified together"
        ));
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
        recalibrate,
        no_probe,
        no_calibrate,
        internal_mixer,
        device_profile,
        output_channels,
        deck_a_out_ch,
        deck_b_out_ch,
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

    // 4b. Resolve thresholds: load saved calibration if present and
    //     fingerprint matches, auto-calibrate otherwise. CLI flag
    //     overrides applied last so partial overrides work
    //     (calibration handles 3 of 4 thresholds, user pins one).
    //
    //     Calibration must happen BEFORE we hand the input consumer
    //     to the engine, because the calibration helpers consume
    //     samples from the same input device. After
    //     `take_consumer()` is called, the input is engine-owned.
    let resolved = resolve_thresholds(&mut input, &opts)?;

    // 5. Hand the input ringbuf consumer to the engine.
    let rx = input
        .take_consumer()
        .ok_or_else(|| anyhow!("AudioInput consumer already taken"))?;
    let cfg = TimecodeInputConfig {
        format: Format::SeratoCv02,
        input_sample_rate: input_sr,
        // CoreAudio output blocks vary; 4096 is a safe upper bound.
        max_block_frames: 4096,
        confidence_threshold: resolved.engage,
        disengage_threshold: resolved.disengage,
        sticky_blocks_to_disengage: resolved.sticky_blocks_to_disengage,
        amplitude_threshold: resolved.amplitude,
    };
    engine
        .attach_timecode_input(0, rx, cfg)
        .context("attaching timecode input to deck 0")?;
    println!(
        "timecode:     format=SeratoCv02 engage={:.3} disengage={:.3} \
         sticky={} blocks amp_floor={:.4}",
        resolved.engage,
        resolved.disengage,
        resolved.sticky_blocks_to_disengage,
        resolved.amplitude,
    );

    // 6. Resolve output routing: known-device auto-detect, manual
    //    per-deck flags, or the M4 internal-mixer fallback. See
    //    `resolve_output_routing` for the full priority order.
    let routing = resolve_output_routing(&device, &opts)?;
    println!("{}", routing.describe());

    // 7. Move the engine onto the audio thread. From here, AudioOutput
    //    drives Engine::render_routed which drives the decoder which
    //    drives deck transport — no main-thread participation in the
    //    audio path.
    let output_opts = dub_audio::OutputOptions {
        channels: routing.channels,
        buffer_frames: opts.output_buffer_size,
        sample_rate: None,
        channel_map: None,
    };
    let output = dub_audio::AudioOutput::start_with_options(engine, &output_opts, routing.routing)
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

/// Resolved output routing. Captured ahead of `AudioOutput::start_with_options`
/// so we can print a clear "what we chose, and why" line before any
/// audio starts — saves the user from wondering why deck B is silent
/// on an unknown interface.
struct ResolvedOutputRouting {
    /// Total channels to open the AU with.
    channels: u32,
    /// Per-deck routing handed to `Engine::render_routed`.
    routing: dub_engine::OutputRouting,
    /// Human-readable summary, printed at startup.
    summary: String,
}

impl ResolvedOutputRouting {
    fn describe(&self) -> &str {
        &self.summary
    }
}

/// Resolve the M5.5.2 output routing in priority order:
///
/// 1. `--internal-mixer` → 2-ch internal mixer (debug only). Loud and
///    explicit; mutually exclusive with all other routing flags.
/// 2. Explicit `--deck-a-out-ch` + `--deck-b-out-ch` → manual routing
///    over `--output-channels` (or the device's reported channel
///    count). Most permissive — works for unknown devices.
/// 3. `--device-profile NAME` → look up the profile by exact pattern
///    and apply its routing. Useful when the system default is the
///    wrong device.
/// 4. Auto-detect by `device.device_name` against
///    `device_profiles::KNOWN_DEVICES`. The path users hit when they
///    plug in their SL3 and run `dub timecode-deck` with no flags.
/// 5. Fallback (unknown device, no flags) → 2-ch internal mixer with a
///    loud warning. Matches Serato's "preparation mode" semantics for
///    laptop-only situations: the user can hear playback but should
///    not run a live set.
fn resolve_output_routing(
    device: &dub_audio::DeviceInfo,
    opts: &Opts,
) -> Result<ResolvedOutputRouting> {
    if opts.internal_mixer {
        return Ok(ResolvedOutputRouting {
            channels: 2,
            routing: dub_engine::INTERNAL_MIXER_ROUTING,
            summary: "output routing: internal mixer (2 ch, both decks → ch 1+2)\n\
                 ⚠️  --internal-mixer is debug-only; not for live performance"
                .to_string(),
        });
    }

    if let (Some(a), Some(b)) = (opts.deck_a_out_ch, opts.deck_b_out_ch) {
        let a0 = device_profiles::one_based_to_zero_based(a)
            .ok_or_else(|| anyhow!("--deck-a-out-ch must be ≥ 1 (1-based), got {a}"))?;
        let b0 = device_profiles::one_based_to_zero_based(b)
            .ok_or_else(|| anyhow!("--deck-b-out-ch must be ≥ 1 (1-based), got {b}"))?;
        let channels = opts.output_channels.unwrap_or(device.channels);
        if channels < 2 {
            return Err(anyhow!(
                "--output-channels must be ≥ 2; got {channels} (device reports {} ch)",
                device.channels
            ));
        }
        if a0 + 2 > channels || b0 + 2 > channels {
            return Err(anyhow!(
                "deck-a-out-ch={a} or deck-b-out-ch={b} doesn't fit in {channels} channels \
                 (each deck takes 2 channels). Pass --output-channels N if your device has \
                 more outputs than the default detected."
            ));
        }
        return Ok(ResolvedOutputRouting {
            channels,
            routing: [Some(a0), Some(b0)],
            summary: format!(
                "output routing: manual ({} ch, deck A → ch {}+{}, deck B → ch {}+{})",
                channels,
                a,
                a + 1,
                b,
                b + 1,
            ),
        });
    }

    let profile = if let Some(pattern) = opts.device_profile.as_deref() {
        device_profiles::profile_by_pattern(pattern).ok_or_else(|| {
            anyhow!(
                "--device-profile {pattern:?} not found in known-device table; \
                 known patterns: {}",
                device_profiles::KNOWN_DEVICES
                    .iter()
                    .map(|d| d.name_pattern)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?
    } else if let Some(p) = device_profiles::match_device(&device.device_name) {
        p
    } else {
        // Unknown device, no manual routing — fall back to internal
        // mixer with a loud warning. Per the M5.5.2 design call: this
        // is preparation-mode-equivalent (the user can audition tracks
        // but the routing isn't right for an external mixer).
        return Ok(ResolvedOutputRouting {
            channels: 2,
            routing: dub_engine::INTERNAL_MIXER_ROUTING,
            summary: format!(
                "output routing: unknown device '{}' — falling back to internal mixer.\n\
                 ⚠️  no recognised interface profile; deck audio is summed to ch 1+2.\n\
                 ⚠️  for an external mixer, pass --deck-a-out-ch / --deck-b-out-ch (1-based) \
                 + --output-channels N, or --device-profile <name> if your interface is \
                 listed in the known-device table",
                device.device_name
            ),
        });
    };

    let channels = opts.output_channels.unwrap_or(profile.output_channels);
    if profile.deck_a_first_channel + 2 > channels || profile.deck_b_first_channel + 2 > channels {
        return Err(anyhow!(
            "device profile '{}' wants {} channels but --output-channels {} is too small",
            profile.display_name,
            profile.output_channels,
            channels
        ));
    }
    let verified_note = if profile.verified {
        ""
    } else {
        "\n⚠️  this profile is unverified against real hardware — double-check the routing"
    };
    Ok(ResolvedOutputRouting {
        channels,
        routing: [
            Some(profile.deck_a_first_channel),
            Some(profile.deck_b_first_channel),
        ],
        summary: format!(
            "output routing: {} ({} ch, deck A → ch {}+{}, deck B → ch {}+{}){}",
            profile.display_name,
            channels,
            profile.deck_a_first_channel + 1,
            profile.deck_a_first_channel + 2,
            profile.deck_b_first_channel + 1,
            profile.deck_b_first_channel + 2,
            verified_note,
        ),
    })
}

/// Resolve the four lift-policy thresholds from (in priority order):
///
/// 1. Explicit CLI overrides (`--confidence`, `--amplitude-threshold`,
///    `--disengage-threshold`, `--sticky-blocks`). Applied last so
///    a partial override always wins over the auto-resolved value.
/// 2. Saved calibration JSON, validated against the current rig
///    via a brief carrier probe. Mismatch (cartridge swap, preamp
///    change, …) triggers automatic recalibration.
/// 3. A fresh full calibration if no JSON exists yet, or
///    `--recalibrate` was passed.
/// 4. The M5.3 defaults if `--no-calibrate` was passed (or the user
///    cancels the calibration flow).
///
/// This is the entry point for the user's "auto-detect different
/// rigs" requirement: even if the same SL3 is used across cartridge
/// swaps, the fingerprint catches the change at startup and the
/// thresholds are re-derived in place.
fn resolve_thresholds(input: &mut AudioInput, opts: &Opts) -> Result<CalibrationThresholds> {
    let format = Format::SeratoCv02;
    let dir = default_calibration_dir().context("resolving default calibration dir")?;
    let path = Calibration::path_for(input.device_name(), format, &dir);

    // Bypass-everything modes first.
    if opts.no_calibrate {
        println!("calibration: skipped (--no-calibrate); using M5.3 defaults");
        return Ok(apply_overrides(default_thresholds(), opts));
    }

    // Force-fresh path. Same as "no file exists" but ignores any
    // existing JSON. We still save the new measurement (overwrites
    // the old file), preserving the always-on "what is this rig"
    // record on disk.
    if opts.recalibrate {
        println!("calibration: --recalibrate forced; running fresh measurement");
        let cal = run_full_calibration(input, format)?;
        save_calibration(&cal, &path);
        return Ok(apply_overrides(cal.thresholds, opts));
    }

    // Try to load. Missing → run a fresh calibration.
    let cal = match Calibration::load(&path) {
        Ok(c) => c,
        Err(_) => {
            println!(
                "calibration: no JSON at {} — running first-time calibration",
                path.display()
            );
            let cal = run_full_calibration(input, format)?;
            save_calibration(&cal, &path);
            return Ok(apply_overrides(cal.thresholds, opts));
        }
    };

    let age_days = calibration_age_days(&cal.calibrated_at);
    if age_days > STALE_CALIBRATION_DAYS {
        eprintln!(
            "  ⚠ calibration is {age_days:.0} days old (>{:.0}); consider \
             `dub timecode-deck ... --recalibrate` for the current venue.",
            STALE_CALIBRATION_DAYS
        );
    }

    // Probe path. Skipping the probe gives faster startup but no
    // rig-swap detection — explicit opt-in via --no-probe.
    if opts.no_probe {
        println!(
            "calibration: loaded {} (probe skipped); engage={:.3} amp={:.4}",
            path.display(),
            cal.thresholds.engage,
            cal.thresholds.amplitude
        );
        return Ok(apply_overrides(cal.thresholds, opts));
    }

    println!(
        "calibration: loaded {} (calibrated {})",
        path.display(),
        cal.calibrated_at
    );
    let observed = match probe_carrier(input, format, PROBE_SECS, PROBE_DETECT_TIMEOUT_SECS) {
        Ok(fp) => fp,
        Err(e) => {
            eprintln!("  ⚠ probe failed: {e:#}\n  using saved thresholds without verification");
            return Ok(apply_overrides(cal.thresholds, opts));
        }
    };
    let delta = cal.fingerprint.max_relative_delta(&observed);
    if cal
        .fingerprint
        .matches(&observed, DEFAULT_FINGERPRINT_TOLERANCE)
    {
        println!(
            "  ✓ fingerprint matches (max delta {:.1}%); engage={:.3} amp={:.4}",
            delta * 100.0,
            cal.thresholds.engage,
            cal.thresholds.amplitude
        );
        return Ok(apply_overrides(cal.thresholds, opts));
    }

    // Mismatch — auto-recalibrate. The user explicitly requested
    // this behavior: "the user can always play on a new cartridge,
    // it must work."
    println!(
        "  ✗ fingerprint differs by {:.1}% (saved {:.4}/{:.4}/{:.3} vs observed \
         {:.4}/{:.4}/{:.3}) — recalibrating",
        delta * 100.0,
        cal.fingerprint.carrier_amp_p50,
        cal.fingerprint.carrier_amp_p95,
        cal.fingerprint.carrier_conf_p50,
        observed.carrier_amp_p50,
        observed.carrier_amp_p95,
        observed.carrier_conf_p50,
    );
    let new_cal = run_full_calibration(input, format)?;
    save_calibration(&new_cal, &path);
    Ok(apply_overrides(new_cal.thresholds, opts))
}

/// M5.3 defaults — the floor that every higher-priority source
/// (saved JSON, fresh measurement) overrides.
fn default_thresholds() -> CalibrationThresholds {
    CalibrationThresholds {
        engage: DEFAULT_CONFIDENCE_THRESHOLD,
        disengage: DEFAULT_DISENGAGE_THRESHOLD,
        amplitude: DEFAULT_AMPLITUDE_THRESHOLD,
        sticky_blocks_to_disengage: DEFAULT_STICKY_BLOCKS_TO_DISENGAGE,
    }
}

/// Apply per-knob CLI overrides on top of an auto-resolved set.
/// Each override replaces exactly one value, leaving the others
/// untouched — partial overrides ("auto-everything except force
/// amplitude=0.05") are first-class.
fn apply_overrides(base: CalibrationThresholds, opts: &Opts) -> CalibrationThresholds {
    CalibrationThresholds {
        engage: opts.confidence.unwrap_or(base.engage),
        disengage: opts.disengage.unwrap_or(base.disengage),
        amplitude: opts.amplitude_threshold.unwrap_or(base.amplitude),
        sticky_blocks_to_disengage: opts
            .sticky_blocks
            .unwrap_or(base.sticky_blocks_to_disengage),
    }
}

/// Run a full calibration against an open `AudioInput` and return
/// the populated [`Calibration`]. A wrapper around
/// [`measure_inline`] that pins the auto-startup defaults.
fn run_full_calibration(input: &mut AudioInput, format: Format) -> Result<Calibration> {
    println!();
    println!("=== auto-calibration ===");
    let cal = measure_inline(
        input,
        format,
        AUTO_CARRIER_SECS,
        AUTO_LIFT_SECS,
        AUTO_DETECT_TIMEOUT_SECS,
    )?;
    println!(
        "  derived: engage={:.3} disengage={:.3} amp={:.4} sticky={} (SNR {:.0}×)",
        cal.thresholds.engage,
        cal.thresholds.disengage,
        cal.thresholds.amplitude,
        cal.thresholds.sticky_blocks_to_disengage,
        cal.snr_margin,
    );
    println!("=== end calibration ===");
    println!();
    Ok(cal)
}

/// Save the calibration to disk; report failures as warnings rather
/// than fatal errors. The user's session can proceed even if disk
/// is full / read-only / sandboxed — they just lose the persistence
/// for next startup. This trade-off keeps the calibration flow
/// "always recoverable" for a live performance setup.
fn save_calibration(cal: &Calibration, path: &std::path::Path) {
    match cal.save(path) {
        Ok(()) => println!("  saved → {}", path.display()),
        Err(e) => eprintln!(
            "  ⚠ failed to save calibration to {}: {e:#}",
            path.display()
        ),
    }
}

/// Difference in days between `calibrated_at` (RFC-3339) and now.
/// Returns 0.0 if `calibrated_at` is unparseable so the freshness
/// warning never spuriously fires for older / future-schema files.
fn calibration_age_days(calibrated_at: &str) -> f64 {
    let parsed = OffsetDateTime::parse(calibrated_at, &Rfc3339).ok();
    let Some(t) = parsed else {
        return 0.0;
    };
    let now = OffsetDateTime::now_utc();
    let dur = now - t;
    dur.as_seconds_f64() / 86_400.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::SCHEMA_VERSION;

    fn opts_default() -> Opts {
        Opts {
            track: PathBuf::new(),
            input: InputArgs::default(),
            output_buffer_size: None,
            confidence: None,
            disengage: None,
            sticky_blocks: None,
            amplitude_threshold: None,
            recalibrate: false,
            no_probe: false,
            no_calibrate: false,
            internal_mixer: false,
            device_profile: None,
            output_channels: None,
            deck_a_out_ch: None,
            deck_b_out_ch: None,
            duration_secs: 0.0,
        }
    }

    fn dev(name: &str, channels: u32) -> dub_audio::DeviceInfo {
        dub_audio::DeviceInfo {
            device_name: name.to_string(),
            sample_rate: 48_000.0,
            channels,
            buffer_frames: 256,
            #[cfg(target_os = "macos")]
            buffer_frame_range: dub_audio::BufferFrameRange { min: 64, max: 4096 },
        }
    }

    #[test]
    fn apply_overrides_replaces_only_set_fields() {
        let base = CalibrationThresholds {
            engage: 0.95,
            disengage: 0.50,
            amplitude: 0.12,
            sticky_blocks_to_disengage: 4,
        };
        let mut opts = opts_default();
        opts.amplitude_threshold = Some(0.05);
        let r = apply_overrides(base, &opts);
        // amplitude overridden, everything else preserved.
        assert!((r.amplitude - 0.05).abs() < 1e-6);
        assert!((r.engage - 0.95).abs() < 1e-6);
        assert!((r.disengage - 0.50).abs() < 1e-6);
        assert_eq!(r.sticky_blocks_to_disengage, 4);
    }

    #[test]
    fn apply_overrides_no_explicit_keeps_base() {
        let base = CalibrationThresholds {
            engage: 0.95,
            disengage: 0.50,
            amplitude: 0.12,
            sticky_blocks_to_disengage: 4,
        };
        let opts = opts_default();
        let r = apply_overrides(base, &opts);
        assert!((r.engage - base.engage).abs() < 1e-6);
        assert!((r.amplitude - base.amplitude).abs() < 1e-6);
    }

    #[test]
    fn apply_overrides_full_override_wins() {
        let base = CalibrationThresholds {
            engage: 0.95,
            disengage: 0.50,
            amplitude: 0.12,
            sticky_blocks_to_disengage: 4,
        };
        let mut opts = opts_default();
        opts.confidence = Some(0.80);
        opts.disengage = Some(0.40);
        opts.amplitude_threshold = Some(0.03);
        opts.sticky_blocks = Some(8);
        let r = apply_overrides(base, &opts);
        assert!((r.engage - 0.80).abs() < 1e-6);
        assert!((r.disengage - 0.40).abs() < 1e-6);
        assert!((r.amplitude - 0.03).abs() < 1e-6);
        assert_eq!(r.sticky_blocks_to_disengage, 8);
    }

    #[test]
    fn calibration_age_days_recent_is_near_zero() {
        let now = OffsetDateTime::now_utc().format(&Rfc3339).unwrap();
        let age = calibration_age_days(&now);
        assert!(age.abs() < 0.01, "expected near 0, got {age}");
    }

    #[test]
    fn calibration_age_days_unparseable_is_zero() {
        // Garbage string should be treated as "fresh" — we'd rather
        // miss a freshness warning than spuriously cry wolf.
        let age = calibration_age_days("not-a-date");
        assert!(age.abs() < f64::EPSILON);
    }

    #[test]
    fn calibration_age_days_30_days_ago_returns_30() {
        let past = OffsetDateTime::now_utc() - time::Duration::days(30);
        let s = past.format(&Rfc3339).unwrap();
        let age = calibration_age_days(&s);
        // Tolerance for sub-second clock drift across the test's
        // own runtime.
        assert!((age - 30.0).abs() < 0.01, "expected ~30, got {age}");
    }

    #[test]
    fn parse_opts_explicit_thresholds_round_trip() {
        // Sanity check: --confidence on the CLI lands as
        // Some(_) (used to test the override path).
        let s = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let args = s(&[
            "--confidence",
            "0.93",
            "--amplitude-threshold",
            "0.04",
            "--input-channels",
            "3,4",
            "track.wav",
        ]);
        let opts = parse_opts(&args).unwrap();
        assert_eq!(opts.confidence, Some(0.93));
        assert_eq!(opts.amplitude_threshold, Some(0.04));
        assert!(opts.disengage.is_none());
        assert!(opts.sticky_blocks.is_none());
    }

    #[test]
    fn parse_opts_recalibrate_flag() {
        let s = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let opts = parse_opts(&s(&["--recalibrate", "--input-channels", "3,4", "t.wav"])).unwrap();
        assert!(opts.recalibrate);
        assert!(!opts.no_probe);
        assert!(!opts.no_calibrate);
    }

    #[test]
    fn parse_opts_recalibrate_and_no_calibrate_conflict() {
        let s = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let r = parse_opts(&s(&[
            "--recalibrate",
            "--no-calibrate",
            "--input-channels",
            "3,4",
            "t.wav",
        ]));
        assert!(r.is_err(), "mutually-exclusive flags should error");
    }

    // --- M5.5.2 output-routing resolution tests ------------------------

    #[test]
    fn resolve_output_routing_internal_mixer_flag() {
        let mut opts = opts_default();
        opts.internal_mixer = true;
        let r = resolve_output_routing(&dev("SL 3", 6), &opts).unwrap();
        assert_eq!(r.channels, 2);
        assert_eq!(r.routing, dub_engine::INTERNAL_MIXER_ROUTING);
        assert!(
            r.summary.contains("internal mixer") && r.summary.contains("debug-only"),
            "expected debug warning, got: {}",
            r.summary
        );
    }

    #[test]
    fn resolve_output_routing_manual_overrides() {
        let mut opts = opts_default();
        opts.deck_a_out_ch = Some(3);
        opts.deck_b_out_ch = Some(5);
        let r = resolve_output_routing(&dev("Mystery USB DAC", 6), &opts).unwrap();
        // Device has 6 channels by default → channels=6.
        assert_eq!(r.channels, 6);
        // 1-based → 0-based: 3 → 2, 5 → 4.
        assert_eq!(r.routing, [Some(2), Some(4)]);
        assert!(r.summary.contains("manual"), "got: {}", r.summary);
    }

    #[test]
    fn resolve_output_routing_manual_with_explicit_channels() {
        let mut opts = opts_default();
        opts.deck_a_out_ch = Some(1);
        opts.deck_b_out_ch = Some(3);
        opts.output_channels = Some(4);
        let r = resolve_output_routing(&dev("Mystery USB DAC", 8), &opts).unwrap();
        assert_eq!(r.channels, 4);
        assert_eq!(r.routing, [Some(0), Some(2)]);
    }

    #[test]
    fn resolve_output_routing_manual_oob_errors() {
        let mut opts = opts_default();
        opts.deck_a_out_ch = Some(5);
        opts.deck_b_out_ch = Some(7);
        // Device only has 4 channels; deck B at ch 7 doesn't fit.
        let r = resolve_output_routing(&dev("Mystery USB DAC", 4), &opts);
        assert!(r.is_err(), "deck-b-out-ch=7 with 4ch device should error");
    }

    #[test]
    fn resolve_output_routing_auto_detects_sl3() {
        let opts = opts_default();
        let r = resolve_output_routing(&dev("Rane SL 3", 6), &opts).unwrap();
        assert_eq!(r.channels, 6);
        assert_eq!(r.routing, [Some(2), Some(4)]); // deck A 3+4, deck B 5+6
        assert!(r.summary.contains("Serato SL 3"), "got: {}", r.summary);
        assert!(
            !r.summary.contains("unverified"),
            "SL 3 is verified, should not warn: {}",
            r.summary
        );
    }

    #[test]
    fn resolve_output_routing_auto_detects_audio6_warns_unverified() {
        let opts = opts_default();
        let r = resolve_output_routing(&dev("Traktor Audio 6", 6), &opts).unwrap();
        assert_eq!(r.routing, [Some(0), Some(2)]);
        assert!(
            r.summary.contains("unverified"),
            "Audio 6 should warn until validated: {}",
            r.summary
        );
    }

    #[test]
    fn resolve_output_routing_unknown_device_falls_back_internal() {
        let opts = opts_default();
        let r = resolve_output_routing(&dev("MacBook Pro Speakers", 2), &opts).unwrap();
        assert_eq!(r.channels, 2);
        assert_eq!(r.routing, dub_engine::INTERNAL_MIXER_ROUTING);
        assert!(
            r.summary.contains("unknown device") && r.summary.contains("internal mixer"),
            "expected fallback summary, got: {}",
            r.summary
        );
    }

    #[test]
    fn resolve_output_routing_device_profile_override() {
        // User has the SL3 connected but their default output is the
        // built-in MacBook (oversight). --device-profile lets them
        // pin the routing without changing macOS audio settings.
        let mut opts = opts_default();
        opts.device_profile = Some("SL 3".to_string());
        let r = resolve_output_routing(&dev("MacBook Pro Speakers", 2), &opts).unwrap();
        // We pin the SL3 profile but the *device* only has 2 outputs
        // — that's an error; the user must also pass --output-channels
        // or fix their default device. Pin the error semantic.
        // Actually the user's profile says SL3 (output_channels=6),
        // and we don't override-check against the device, we just
        // open the AU with `channels`. The macOS default-output AU
        // can still be opened with N channels even if the underlying
        // device has fewer (the AU aggregates), so this is the
        // user's own footgun. We pass through and let CoreAudio
        // reject if it must.
        assert_eq!(r.channels, 6);
        assert_eq!(r.routing, [Some(2), Some(4)]);
    }

    #[test]
    fn parse_opts_routing_flags_round_trip() {
        let s = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let opts = parse_opts(&s(&[
            "--deck-a-out-ch",
            "3",
            "--deck-b-out-ch",
            "5",
            "--output-channels",
            "6",
            "--input-channels",
            "3,4",
            "t.wav",
        ]))
        .unwrap();
        assert_eq!(opts.deck_a_out_ch, Some(3));
        assert_eq!(opts.deck_b_out_ch, Some(5));
        assert_eq!(opts.output_channels, Some(6));
    }

    #[test]
    fn parse_opts_internal_mixer_with_deck_flags_errors() {
        let s = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let r = parse_opts(&s(&[
            "--internal-mixer",
            "--deck-a-out-ch",
            "3",
            "--deck-b-out-ch",
            "5",
            "--input-channels",
            "3,4",
            "t.wav",
        ]));
        assert!(r.is_err(), "internal-mixer + deck flags must conflict");
    }

    #[test]
    fn parse_opts_partial_deck_flags_errors() {
        let s = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let r = parse_opts(&s(&[
            "--deck-a-out-ch",
            "3",
            "--input-channels",
            "3,4",
            "t.wav",
        ]));
        assert!(
            r.is_err(),
            "deck-a alone (without deck-b) must error to avoid asymmetric routing"
        );
    }

    /// Avoid unused-import lint when calibration types pull in.
    #[allow(dead_code)]
    fn _keep_schema_version_alive() {
        let _ = SCHEMA_VERSION;
    }
}
