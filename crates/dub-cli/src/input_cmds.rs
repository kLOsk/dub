//! `dub list-inputs`, `dub levels`, `dub capture` — audio-input CLIs.
//!
//! M5.2: the consumer-thread side of [`dub_audio::AudioInput`].
//!
//! - `list-inputs` enumerates HAL input devices.
//! - `levels` runs a live RMS-per-channel meter for a chosen device,
//!   useful for verifying SL3 / Audio 6 cabling without writing any
//!   files. Updates 20× per second on stderr so the user can see what
//!   their cartridge is sending in real time.
//! - `capture` writes the input device's stream to a 32-bit float
//!   stereo (or N-channel) WAV. The output is suitable as input to
//!   `dub decode-timecode <wav>` for offline timecode validation
//!   ahead of the live integration in M5.3.
//!
//! All three share an [`InputArgs`] parser so flags stay consistent
//! across commands (`--device`, `--channels`, `--buffer-size`, `--sr`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use dub_audio::{AudioInput, InputDeviceInfo, InputOptions};

/// Default capture/levels duration in seconds when `--duration` is
/// omitted. 5 seconds is enough to confirm signal flow without
/// holding the terminal too long.
const DEFAULT_DURATION_SECS: f64 = 5.0;

/// Flags shared by `levels` and `capture`.
#[derive(Debug, Default, Clone)]
pub struct InputArgs {
    pub device: Option<String>,
    pub channels: Option<u32>,
    pub buffer_size: Option<u32>,
    pub sample_rate: Option<f32>,
    pub duration: Option<f64>,
    /// 1-based device channel indices to capture. Length implies
    /// the output channel count. `Some(vec![3, 4])` reads device
    /// inputs 3 and 4 into output slots 0 and 1 — the natural
    /// setup for Serato SL3 turntable A. `None` keeps the default
    /// (`[1, 2]`-equivalent identity mapping).
    pub input_channels: Option<Vec<u32>>,
}

impl InputArgs {
    /// Apply this arg-set to a default [`InputOptions`]. Anything not
    /// set in `self` keeps its default.
    ///
    /// `--input-channels` overrides `--channels`: when present, the
    /// number of channels we open the AU with equals the number of
    /// entries in the map, and the map itself is converted from
    /// 1-based (user-facing) to 0-based (CoreAudio).
    pub(crate) fn to_options(&self) -> InputOptions {
        let (channels, channel_map) = match &self.input_channels {
            Some(v) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let map: Vec<i32> = v.iter().map(|&c| (c as i32) - 1).collect();
                #[allow(clippy::cast_possible_truncation)]
                (v.len() as u32, Some(map))
            }
            None => (self.channels.map_or(2, |c| c.max(1)), None),
        };
        InputOptions {
            device_name: self.device.clone(),
            channels,
            buffer_frames: self.buffer_size,
            sample_rate: self.sample_rate,
            channel_map,
            ..InputOptions::default()
        }
    }

    pub(crate) fn duration_secs(&self) -> f64 {
        self.duration.unwrap_or(DEFAULT_DURATION_SECS)
    }
}

/// Parse the shared input flags out of `args`. Unknown flags or
/// command-specific positionals are returned via the second tuple
/// element so callers can interpret them.
pub(crate) fn parse_input_args(args: &[String]) -> Result<(InputArgs, Vec<String>)> {
    let mut input = InputArgs::default();
    let mut leftover: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].as_str();
        match raw {
            "--device" | "-d" => {
                input.device = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--device expects a name"))?
                        .clone(),
                );
                i += 2;
            }
            "--channels" => {
                input.channels = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--channels expects an integer"))?
                        .parse()
                        .context("--channels not an integer")?,
                );
                i += 2;
            }
            "--buffer-size" => {
                input.buffer_size = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--buffer-size expects an integer"))?
                        .parse()
                        .context("--buffer-size not an integer")?,
                );
                i += 2;
            }
            "--sr" | "--sample-rate" => {
                input.sample_rate = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--sr expects a number"))?
                        .parse()
                        .context("--sr not a number")?,
                );
                i += 2;
            }
            "--duration" => {
                input.duration = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--duration expects seconds"))?
                        .parse()
                        .context("--duration not a number")?,
                );
                i += 2;
            }
            "--input-channels" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--input-channels expects N[,M,...]"))?;
                let parsed: Result<Vec<u32>, _> =
                    raw.split(',').map(|s| s.trim().parse::<u32>()).collect();
                let v = parsed.context("--input-channels values must be integers")?;
                if v.is_empty() {
                    return Err(anyhow!("--input-channels needs at least one channel"));
                }
                if v.contains(&0) {
                    return Err(anyhow!(
                        "--input-channels uses 1-based indices (no 0); use 1,2 not 0,1"
                    ));
                }
                input.input_channels = Some(v);
                i += 2;
            }
            _ => {
                leftover.push(args[i].clone());
                i += 1;
            }
        }
    }
    Ok((input, leftover))
}

/// `dub list-inputs` — enumerate HAL input devices.
///
/// # Errors
/// HAL enumeration failures.
pub fn list_inputs() -> Result<()> {
    let devices = dub_audio::list_input_devices().context("enumerating input devices")?;
    if devices.is_empty() {
        println!("(no input devices found)");
        return Ok(());
    }
    let default_name = dub_audio::query_default_input().ok().map(|d| d.name);
    println!("input devices ({}):", devices.len());
    for d in &devices {
        let star = if Some(&d.name) == default_name.as_ref() {
            " [default]"
        } else {
            ""
        };
        let InputDeviceInfo {
            name,
            sample_rate,
            channels,
            buffer_frames,
            buffer_frame_range,
        } = d;
        println!(
            "  {name}{star}\n    sr={sample_rate} Hz  channels={channels}  buffer={buffer_frames} frames (range {}-{})",
            buffer_frame_range.min, buffer_frame_range.max
        );
    }
    Ok(())
}

/// `dub levels [--device NAME] [--duration SECS] [--channels N]`
///
/// Live per-channel RMS meter, refreshed 20× per second. ASCII bar
/// per channel scaled to ~50 columns at full scale (`-1.0`..`1.0`).
///
/// # Errors
/// Any HAL or device-open error.
pub fn levels(args: &[String]) -> Result<()> {
    let (input_args, leftover) = parse_input_args(args)?;
    if !leftover.is_empty() {
        return Err(anyhow!("unexpected args: {:?}", leftover));
    }
    let opts = input_args.to_options();
    let mut input = AudioInput::start_with_options(&opts).context("opening input device")?;

    println!(
        "levels: device='{}' sr={} Hz channels={} buffer={} frames -> {:.2} ms",
        input.device_name(),
        input.sample_rate(),
        input.channels(),
        input.buffer_frames(),
        input.latency_seconds() * 1000.0
    );
    println!(
        "duration: {:.1} s   (Ctrl-C to stop early)",
        input_args.duration_secs()
    );
    println!();

    let channels = input.channels() as usize;
    let block_frames = 4096_usize;
    let mut buf = vec![0.0_f32; block_frames * channels];
    let mut rms_acc = vec![0.0_f64; channels];
    let mut peak = vec![0.0_f32; channels];
    let mut samples_per_channel: u64 = 0;

    let start = Instant::now();
    let total = Duration::from_secs_f64(input_args.duration_secs());
    let refresh = Duration::from_millis(50); // 20 Hz redraw
    let mut next_refresh = Instant::now() + refresh;

    while start.elapsed() < total {
        let n = input.read_into(&mut buf);
        if n == 0 {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        // n is samples, not frames.
        let frames = n / channels;
        for f in 0..frames {
            for (ch, acc) in rms_acc.iter_mut().enumerate().take(channels) {
                let s = buf[f * channels + ch];
                *acc += f64::from(s) * f64::from(s);
                if s.abs() > peak[ch] {
                    peak[ch] = s.abs();
                }
            }
        }
        samples_per_channel += frames as u64;

        if Instant::now() >= next_refresh {
            redraw_levels(&rms_acc, &peak, samples_per_channel);
            // Decay peak holds slightly so they tick down between
            // redraws. The RMS accumulator we keep cumulative so the
            // average is over the whole capture.
            for p in peak.iter_mut() {
                *p *= 0.85;
            }
            next_refresh = Instant::now() + refresh;
        }
    }

    eprintln!();
    let cb_count = input.callback_count();
    println!(
        "callbacks: {cb_count} (overflow={})",
        input.overflow_count()
    );
    if input.overflow_count() > 0 {
        eprintln!(
            "warning: input ringbuf overflowed {} times — consumer thread fell behind",
            input.overflow_count()
        );
    }
    if cb_count == 0 {
        return Err(anyhow!(zero_callback_message()));
    }
    Ok(())
}

/// Diagnostic for the "0 callbacks" failure mode. We hit this for two
/// real-world reasons; the message lists both so the user doesn't waste
/// an hour like we did:
///
/// 1. **Sample-rate mismatch** between the AudioUnit and the device's
///    hardware nominal SR. CoreAudio HAL silently delivers nothing
///    when these disagree. `AudioInput::start_with_options` now forces
///    a match, so this should be impossible after the M5.2 fix — but
///    we keep the hint for unknown driver edge cases.
/// 2. **TCC microphone permission** never granted to the responsible
///    parent process (Cursor, Terminal.app, …).
fn zero_callback_message() -> String {
    "device fired ZERO callbacks\n  \
     possible causes:\n    \
       1. macOS microphone permission not granted to the parent app\n       \
          (System Settings → Privacy & Security → Microphone), or\n    \
       2. driver-level sample-rate mismatch: try `--sr` matching the\n       \
          rate shown by `dub list-inputs` for this device."
        .to_string()
}

fn redraw_levels(rms_acc: &[f64], peak: &[f32], samples: u64) {
    if samples == 0 {
        return;
    }
    let mut line = String::new();
    for (ch, acc) in rms_acc.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let rms = (acc / samples as f64).sqrt();
        // Convert linear → dBFS for human-readable scaling.
        let rms_db = if rms > 1e-10 {
            20.0 * rms.log10()
        } else {
            -100.0
        };
        let peak_db = if peak[ch] > 1e-10 {
            20.0 * f64::from(peak[ch]).log10()
        } else {
            -100.0
        };
        // Bar mapping: -60 dBFS → 0 cols, 0 dBFS → 50 cols.
        let cols = (((rms_db + 60.0) / 60.0) * 50.0).clamp(0.0, 50.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bar_n = cols.round() as usize;
        let bar: String = "#".repeat(bar_n) + &" ".repeat(50 - bar_n);
        line.push_str(&format!(
            "ch{ch}: {rms_db:6.1} dB rms  {peak_db:6.1} dB peak  [{bar}]\n"
        ));
    }
    // Clear previous block by rewriting same number of lines via ANSI
    // cursor-up. One trailing newline at end of block to keep terminals
    // that don't honor the escape from collapsing onto a single line.
    eprint!("\x1b[{}A", rms_acc.len());
    eprint!("{line}");
}

/// `dub capture <output.wav> [--device NAME] [--duration SECS] ...`
///
/// Capture the input device to a 32-bit float WAV.
///
/// # Errors
/// Any HAL, device-open, or WAV-write error.
pub fn capture(args: &[String]) -> Result<()> {
    let (input_args, leftover) = parse_input_args(args)?;
    let mut output: Option<PathBuf> = None;
    let mut iter = leftover.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                let v = iter
                    .next()
                    .ok_or_else(|| anyhow!("--output expects a path"))?;
                output = Some(PathBuf::from(v));
            }
            other if other.starts_with("--") => {
                return Err(anyhow!("unknown capture flag: {other}"));
            }
            other => {
                if output.is_some() {
                    return Err(anyhow!("unexpected positional arg: {other}"));
                }
                output = Some(PathBuf::from(other));
            }
        }
    }
    let output = output.ok_or_else(|| {
        anyhow!("usage: dub capture <output.wav> [--device NAME] [--duration SECS] ...")
    })?;

    let opts = input_args.to_options();
    let mut input = AudioInput::start_with_options(&opts).context("opening input device")?;

    println!(
        "capture: device='{}' sr={} Hz channels={} buffer={} frames",
        input.device_name(),
        input.sample_rate(),
        input.channels(),
        input.buffer_frames()
    );
    println!("output: {}", output.display());
    println!("duration: {:.1} s", input_args.duration_secs());

    let spec = hound::WavSpec {
        channels: input.channels() as u16,
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        sample_rate: input.sample_rate().round() as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&output, spec).context("opening output WAV")?;

    let channels = input.channels() as usize;
    let block_frames = 4096_usize;
    let mut buf = vec![0.0_f32; block_frames * channels];

    let start = Instant::now();
    let total = Duration::from_secs_f64(input_args.duration_secs());
    let mut samples_written: u64 = 0;
    let mut peak: f32 = 0.0;

    while start.elapsed() < total {
        let n = input.read_into(&mut buf);
        if n == 0 {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        for s in &buf[..n] {
            writer.write_sample(*s).context("writing sample")?;
            let abs = s.abs();
            if abs > peak {
                peak = abs;
            }
        }
        samples_written += n as u64;
    }
    writer.finalize().context("finalizing WAV")?;

    let elapsed = start.elapsed();
    let frames = samples_written / channels as u64;
    let cb_count = input.callback_count();
    println!();
    println!(
        "  callbacks:  {cb_count} (overflow={})",
        input.overflow_count()
    );
    println!(
        "  captured:   {frames} frames ({:.3} s)",
        frames as f64 / f64::from(input.sample_rate())
    );
    println!("  wall:       {:.3} s", elapsed.as_secs_f64());
    println!(
        "  peak:       {:.4} ({:.2} dBFS)",
        peak,
        20.0 * f64::from(peak.max(1e-10)).log10()
    );
    if input.overflow_count() > 0 {
        eprintln!(
            "warning: input ringbuf overflowed {} times — capture missing samples",
            input.overflow_count()
        );
    }
    if cb_count == 0 {
        return Err(anyhow!(zero_callback_message()));
    }
    println!("OK");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_string()).collect()
    }

    #[test]
    fn parses_device_and_duration() {
        let (input, leftover) =
            parse_input_args(&s(&["--device", "Audio 6", "--duration", "3.5"])).unwrap();
        assert_eq!(input.device.as_deref(), Some("Audio 6"));
        assert!((input.duration_secs() - 3.5).abs() < 1e-9);
        assert!(leftover.is_empty());
    }

    #[test]
    fn parses_channels_and_buffer_size() {
        let (input, _) =
            parse_input_args(&s(&["--channels", "4", "--buffer-size", "128"])).unwrap();
        assert_eq!(input.channels, Some(4));
        assert_eq!(input.buffer_size, Some(128));
    }

    #[test]
    fn parses_sr_aliases() {
        for flag in ["--sr", "--sample-rate"] {
            let (input, _) = parse_input_args(&s(&[flag, "96000"])).unwrap();
            assert!((input.sample_rate.unwrap() - 96_000.0).abs() < 0.5);
        }
    }

    #[test]
    fn rejects_missing_value() {
        for flag in [
            "--device",
            "--channels",
            "--buffer-size",
            "--sr",
            "--duration",
        ] {
            assert!(
                parse_input_args(&s(&[flag])).is_err(),
                "{flag} alone should error"
            );
        }
    }

    #[test]
    fn parses_input_channels_pair() {
        let (input, _) = parse_input_args(&s(&["--input-channels", "3,4"])).unwrap();
        assert_eq!(input.input_channels.as_deref(), Some(&[3_u32, 4_u32][..]));
    }

    #[test]
    fn parses_input_channels_quad() {
        let (input, _) = parse_input_args(&s(&["--input-channels", "1,2,3,4"])).unwrap();
        assert_eq!(input.input_channels.as_deref(), Some(&[1, 2, 3, 4][..]));
    }

    #[test]
    fn rejects_zero_in_input_channels() {
        let r = parse_input_args(&s(&["--input-channels", "0,1"]));
        assert!(r.is_err(), "0 must be rejected as 1-based index");
    }

    #[test]
    fn rejects_non_numeric_input_channels() {
        let r = parse_input_args(&s(&["--input-channels", "3,foo"]));
        assert!(r.is_err());
    }

    #[test]
    fn input_channels_overrides_channels_count() {
        // Even if --channels says 6, --input-channels 3,4 wins:
        // we open 2 channels with a map of [2, 3] (0-based).
        let (input, _) =
            parse_input_args(&s(&["--channels", "6", "--input-channels", "3,4"])).unwrap();
        let opts = input.to_options();
        assert_eq!(opts.channels, 2);
        assert_eq!(opts.channel_map.as_deref(), Some(&[2_i32, 3_i32][..]));
    }

    #[test]
    fn input_channels_translates_to_zero_based_map() {
        let input = InputArgs {
            input_channels: Some(vec![3, 4]),
            ..InputArgs::default()
        };
        let opts = input.to_options();
        assert_eq!(opts.channels, 2);
        assert_eq!(opts.channel_map.as_deref(), Some(&[2_i32, 3_i32][..]));
    }

    #[test]
    fn passes_unknown_flags_through_as_leftover() {
        let (input, leftover) =
            parse_input_args(&s(&["--device", "X", "-o", "out.wav", "extra-positional"])).unwrap();
        assert_eq!(input.device.as_deref(), Some("X"));
        assert_eq!(leftover, ["-o", "out.wav", "extra-positional"]);
    }

    #[test]
    fn to_options_default_channels_2() {
        let opts = InputArgs::default().to_options();
        assert_eq!(opts.channels, 2);
        assert!(opts.device_name.is_none());
        assert!(opts.buffer_frames.is_none());
        assert!(opts.sample_rate.is_none());
    }

    #[test]
    fn to_options_clamps_channels_to_min_one() {
        let a = InputArgs {
            channels: Some(0),
            ..InputArgs::default()
        };
        let opts = a.to_options();
        assert_eq!(opts.channels, 1);
    }

    #[test]
    fn duration_default_is_5s() {
        let a = InputArgs::default();
        assert!((a.duration_secs() - DEFAULT_DURATION_SECS).abs() < 1e-9);
    }
}
