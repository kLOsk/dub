//! `dub loopback` — play a recorded take out of one pair of an interface
//! and record another pair while it plays.
//!
//! This is the turntable with no hands in `docs/RIG-CHECKLIST.md`: the
//! SL 3's aux output cabled into a deck input feeds a running Dub a take
//! from `testdata/timecode/full/`, and the deck's output cabled back into
//! the aux input is what this records.
//!
//! The take goes to the converter sample for sample: no engine, no
//! resampler. The device's rate and buffer size are never changed,
//! because the device is the one Dub is playing on.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use dub_audio::{AudioInput, AudioOutput, OutputOptions};

use crate::input_cmds::{clock_excess, AllChannels, InputArgs};

const USAGE: &str = "usage: dub loopback <take> [--record <out.wav>] [--device NAME] \
     [--output-device NAME] [--output-channels N,M] [--input-channels N,M] \
     [--from SECS] [--secs SECS] [--repeat N] [--gain G] [--lead-in SECS] [--tail SECS] \
     [--take-channels N,M]";

/// Silence played before the take, so the recorder is rolling and a
/// deck sees a needle that is up before one that is down.
const DEFAULT_LEAD_IN_SECS: f64 = 1.0;

/// Recording kept after the take ends: an echo tail, a deck coasting.
const DEFAULT_TAIL_SECS: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
struct Args {
    take: PathBuf,
    /// The pair to play out of a wider WAV take, 1-based: a `make rig`
    /// take holds every input of the interface (deck A is 3,4 on the
    /// SL 3). `None` plays a stereo take as it is.
    take_channels: Option<(u32, u32)>,
    record: Option<PathBuf>,
    device: Option<String>,
    output_device: Option<String>,
    /// 1-based, as printed on the interface.
    output_channels: (u32, u32),
    /// 1-based.
    input_channels: Vec<u32>,
    from_secs: f64,
    secs: Option<f64>,
    repeat: u32,
    gain: f32,
    lead_in_secs: f64,
    tail_secs: f64,
}

fn parse_channels(raw: &str, flag: &str) -> Result<Vec<u32>> {
    let parsed: Result<Vec<u32>, _> = raw.split(',').map(|s| s.trim().parse::<u32>()).collect();
    let v = parsed.with_context(|| format!("{flag} values must be integers"))?;
    if v.contains(&0) {
        bail!("{flag} uses 1-based indices (no 0); use 1,2 not 0,1");
    }
    Ok(v)
}

fn parse_args(args: &[String]) -> Result<Args> {
    let mut out = Args {
        take: PathBuf::new(),
        take_channels: None,
        record: None,
        device: None,
        output_device: None,
        output_channels: (1, 2),
        input_channels: vec![1, 2],
        from_secs: 0.0,
        secs: None,
        repeat: 1,
        gain: 1.0,
        lead_in_secs: DEFAULT_LEAD_IN_SECS,
        tail_secs: DEFAULT_TAIL_SECS,
    };
    let mut take: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = || -> Result<&str> {
            args.get(i + 1)
                .map(String::as_str)
                .ok_or_else(|| anyhow!("{flag} expects a value"))
        };
        let seconds = || -> Result<f64> {
            let secs: f64 = value()?
                .parse()
                .with_context(|| format!("{flag} not a number"))?;
            if !secs.is_finite() || secs < 0.0 {
                bail!("{flag} must be zero or more seconds");
            }
            Ok(secs)
        };
        match flag {
            "--record" | "-o" => out.record = Some(PathBuf::from(value()?)),
            "--device" | "-d" => out.device = Some(value()?.to_string()),
            "--output-device" => out.output_device = Some(value()?.to_string()),
            "--output-channels" => {
                let v = parse_channels(value()?, flag)?;
                let [left, right] = v[..] else {
                    bail!("--output-channels takes one pair, like 1,2");
                };
                if left == right {
                    bail!("--output-channels needs two different channels");
                }
                out.output_channels = (left, right);
            }
            "--input-channels" => {
                let v = parse_channels(value()?, flag)?;
                if v.len() < 2 {
                    bail!("--input-channels needs at least a pair, like 1,2");
                }
                out.input_channels = v;
            }
            "--take-channels" => {
                let v = parse_channels(value()?, flag)?;
                let [left, right] = v[..] else {
                    bail!("--take-channels takes one pair, like 3,4");
                };
                out.take_channels = Some((left, right));
            }
            "--from" => out.from_secs = seconds()?,
            "--secs" => out.secs = Some(seconds()?),
            "--lead-in" => out.lead_in_secs = seconds()?,
            "--tail" => out.tail_secs = seconds()?,
            "--repeat" => {
                out.repeat = value()?.parse().context("--repeat not an integer")?;
                if out.repeat == 0 {
                    bail!("--repeat must be at least 1");
                }
            }
            "--gain" => {
                out.gain = value()?.parse().context("--gain not a number")?;
                if !out.gain.is_finite() || out.gain < 0.0 {
                    bail!("--gain must be zero or more");
                }
            }
            other if other.starts_with('-') => bail!("unknown loopback flag: {other}\n{USAGE}"),
            other => {
                if take.is_some() {
                    bail!("unexpected positional arg: {other}\n{USAGE}");
                }
                take = Some(PathBuf::from(other));
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    out.take = take.ok_or_else(|| anyhow!("{USAGE}"))?;
    Ok(out)
}

/// The pair `(left, right)`, 1-based, of an interleaved `channels`-wide
/// take, as interleaved stereo.
fn pick_pair(samples: &[f32], channels: usize, (left, right): (u32, u32)) -> Vec<f32> {
    let (l, r) = (left as usize - 1, right as usize - 1);
    samples
        .chunks_exact(channels)
        .flat_map(|frame| [frame[l], frame[r]])
        .collect()
}

/// One pair of a WAV take wider than stereo, and its rate. Read here
/// rather than through `dub_io`, which decodes mono and stereo only.
fn read_pair(path: &Path, pair: (u32, u32)) -> Result<(Vec<f32>, u32)> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("reading {}", path.display()))?;
    let spec = reader.spec();
    let channels = u32::from(spec.channels);
    if pair.0 > channels || pair.1 > channels {
        bail!(
            "--take-channels {},{}: {} has {channels} channel(s)",
            pair.0,
            pair.1,
            path.display()
        );
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let full_scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / full_scale))
                .collect::<Result<_, _>>()?
        }
    };
    Ok((
        pick_pair(&samples, channels as usize, pair),
        spec.sample_rate,
    ))
}

/// The frames `[from, from + secs)` of an interleaved stereo take.
fn excerpt(samples: &[f32], sample_rate: u32, from_secs: f64, secs: Option<f64>) -> Result<&[f32]> {
    let frames = samples.len() / 2;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "seconds are validated non-negative and a take is far below 2^52 frames"
    )]
    let to_frames = |s: f64| (s * f64::from(sample_rate)).round() as usize;
    let start = to_frames(from_secs);
    if start >= frames {
        bail!(
            "--from {from_secs} s is past the end of the take ({:.1} s)",
            frames as f64 / f64::from(sample_rate)
        );
    }
    let end = secs.map_or(frames, |s| (start + to_frames(s)).min(frames));
    if end == start {
        bail!("--secs leaves nothing to play");
    }
    Ok(&samples[start * 2..end * 2])
}

/// What the player tells the thread that waits on it.
#[derive(Debug, Default)]
struct Progress {
    /// Frames of the take that have gone out (repeats counted).
    played: AtomicU64,
    done: AtomicBool,
}

/// Plays a stereo take onto one pair of a wider output, on the audio
/// thread: nothing here allocates, locks or blocks.
struct TakePlayer {
    /// Interleaved stereo.
    take: Arc<[f32]>,
    /// 0-based slots in the output frame.
    left: usize,
    right: usize,
    gain: f32,
    lead_in_frames: usize,
    cursor: usize,
    passes_left: u32,
    progress: Arc<Progress>,
}

impl TakePlayer {
    fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        let take_frames = self.take.len() / 2;
        let mut played = 0_u64;
        for frame in out.chunks_exact_mut(channels) {
            if self.lead_in_frames > 0 {
                self.lead_in_frames -= 1;
                continue;
            }
            if self.passes_left == 0 {
                break;
            }
            frame[self.left] = self.take[self.cursor * 2] * self.gain;
            frame[self.right] = self.take[self.cursor * 2 + 1] * self.gain;
            played += 1;
            self.cursor += 1;
            if self.cursor == take_frames {
                self.cursor = 0;
                self.passes_left -= 1;
            }
        }
        if played > 0 {
            self.progress.played.fetch_add(played, Ordering::Relaxed);
        }
        if self.passes_left == 0 {
            self.progress.done.store(true, Ordering::Release);
        }
    }
}

/// Seconds since the Unix epoch, for lining a recording up with events a
/// script logs while it runs.
fn unix_secs(t: SystemTime) -> f64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// Where a take was found in a recording, and how it came back.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Echo {
    /// Frame of the recording at which the take's first frame sits.
    at_frame: usize,
    /// 1.0 is the take itself; a deck in between reads near 0.
    correlation: f64,
    gain_db: f64,
}

/// −60 dBFS. Below it a recording is an open input, not a signal.
const SILENCE_PEAK: f32 = 1e-3;

/// Frames of the take compared against the recording. A fifth of a
/// second of timecode is hundreds of carrier cycles and its data bits
/// make it unlike any other stretch.
const ECHO_WINDOW_FRAMES: usize = 8192;

/// Find `take` (one channel) in `recording` (one channel).
///
/// The window starts where the take first gets loud, so a take that
/// opens with a lifted needle is matched on its signal, not its silence.
fn find_echo(take: &[f32], recording: &[f32]) -> Option<Echo> {
    let peak = take.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    if peak <= 0.0 {
        return None;
    }
    let start = take.iter().position(|s| s.abs() > peak * 0.1)?;
    let window = &take[start..(start + ECHO_WINDOW_FRAMES).min(take.len())];
    if recording.len() < window.len() {
        return None;
    }
    // A deck with nothing loaded returns its converter's noise floor,
    // which correlates with anything a little and means nothing.
    if recording.iter().all(|s| s.abs() < SILENCE_PEAK) {
        return None;
    }
    let energy = |x: &[f32]| x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>();
    let take_energy = energy(window);

    let mut rec_energy = energy(&recording[..window.len()]);
    let mut best: Option<(usize, f64, f64)> = None;
    for lag in 0..=recording.len() - window.len() {
        let rec = &recording[lag..lag + window.len()];
        let dot: f64 = window
            .iter()
            .zip(rec)
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum();
        let norm = (take_energy * rec_energy).sqrt();
        if norm > 0.0 {
            let correlation = dot / norm;
            if best.is_none_or(|(_, c, _)| correlation > c) {
                best = Some((lag, correlation, dot / take_energy));
            }
        }
        if lag + window.len() < recording.len() {
            let leaving = f64::from(recording[lag]);
            let entering = f64::from(recording[lag + window.len()]);
            rec_energy = (rec_energy - leaving * leaving + entering * entering).max(0.0);
        }
    }
    let (lag, correlation, gain) = best?;
    Some(Echo {
        at_frame: lag.saturating_sub(start),
        correlation,
        gain_db: 20.0 * gain.abs().max(1e-10).log10(),
    })
}

/// How far past the lead-in the take is looked for. The comparison costs
/// the window at every frame searched, so a minute of recording searched
/// whole took longer than the minute; the round trip is milliseconds.
const ECHO_SEARCH_SECS: f64 = 3.0;

/// Above this the recording is the take: converters and a cable move
/// the correlation by thousandths, a deck in between by nearly all of it.
const SAME_SIGNAL_CORRELATION: f64 = 0.9;

fn describe_echo(echo: Option<Echo>, sample_rate: f64) -> String {
    match echo {
        Some(e) if e.correlation >= SAME_SIGNAL_CORRELATION => format!(
            "the take came back as it went out: {:+.1} dB, {:.3} s into the recording \
             (correlation {:.3})",
            e.gain_db,
            e.at_frame as f64 / sample_rate,
            e.correlation
        ),
        Some(e) => format!(
            "the recording is not the take (correlation {:.2}): right through a deck, \
             wrong for a cable from output to input",
            e.correlation
        ),
        None => "nothing to compare: the take or the recording is silent".to_string(),
    }
}

/// `dub loopback <take> [--record <out.wav>] …`
///
/// # Errors
/// A take that can't be read or isn't stereo, a device that can't be
/// opened or runs at another rate, or a WAV that can't be written.
pub fn run(args: &[String]) -> Result<()> {
    let args = parse_args(args)?;

    let (samples, sample_rate) = match args.take_channels {
        Some(pair) => read_pair(&args.take, pair)?,
        None => {
            let track = dub_io::Track::load_from_path(&args.take)
                .with_context(|| format!("reading {}", args.take.display()))?;
            if track.channels() != 2 {
                bail!(
                    "{} has {} channel(s); a take is one deck's stereo pair \
                     (--take-channels N,M picks one out of a wider WAV)",
                    args.take.display(),
                    track.channels()
                );
            }
            (track.samples().to_vec(), track.sample_rate())
        }
    };
    let take: Arc<[f32]> = Arc::from(excerpt(&samples, sample_rate, args.from_secs, args.secs)?);
    drop(samples);
    let take_frames = take.len() / 2;
    let take_secs = take_frames as f64 / f64::from(sample_rate);

    let (out_left, out_right) = args.output_channels;
    let out_channels = out_left.max(out_right);
    let output_device = args.output_device.clone().or_else(|| args.device.clone());

    let mut input = match &args.record {
        Some(_) => {
            let input_args = InputArgs {
                device: args.device.clone(),
                input_channels: Some(args.input_channels.clone()),
                ..InputArgs::default()
            };
            let opts = input_args.to_all_channel_options()?;
            Some(AudioInput::start_with_options(&opts).context("opening input device")?)
        }
        None => None,
    };
    // Frame 0 of the recording. Loading a long take takes a second or
    // more, so a script that times its presses from launching this command
    // is that much late against the recording; it times them from here.
    let recording_started = unix_secs(SystemTime::now());
    if let Some(input) = &input {
        if (f64::from(input.sample_rate()) - f64::from(sample_rate)).abs() > 0.5 {
            bail!(
                "input device '{}' runs at {} Hz and the take is {} Hz",
                input.device_name(),
                input.sample_rate(),
                sample_rate
            );
        }
    }

    let progress = Arc::new(Progress::default());
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "validated non-negative seconds"
    )]
    let lead_in_frames = (args.lead_in_secs * f64::from(sample_rate)).round() as usize;
    let mut player = TakePlayer {
        take: Arc::clone(&take),
        left: (out_left - 1) as usize,
        right: (out_right - 1) as usize,
        gain: args.gain,
        lead_in_frames,
        cursor: 0,
        passes_left: args.repeat,
        progress: Arc::clone(&progress),
    };
    let out_opts = OutputOptions {
        channels: out_channels.max(2),
        device_name: output_device,
        ..OutputOptions::default()
    };
    #[allow(
        clippy::cast_precision_loss,
        reason = "audio sample rates are exact in f32"
    )]
    let output = AudioOutput::start_with_source(sample_rate as f32, &out_opts, move |buf, ch| {
        player.render(buf, ch);
    })
    .context("opening output device")?;
    let take_starts = unix_secs(SystemTime::now()) + args.lead_in_secs;

    println!(
        "loopback: playing {} ({take_secs:.1} s × {}) to '{}' channels {out_left},{out_right} \
         at {sample_rate} Hz, buffer {} frames",
        args.take.display(),
        args.repeat,
        output.device_name(),
        output.buffer_frames()
    );

    println!("take starts at unix {take_starts:.3}");

    let mut writer = match (&args.record, &input) {
        (Some(path), Some(input)) => {
            println!(
                "recording '{}' channels {:?} to {}",
                input.device_name(),
                args.input_channels,
                path.display()
            );
            println!("recording starts at unix {recording_started:.3}");
            #[allow(
                clippy::cast_possible_truncation,
                reason = "channel counts are single digits"
            )]
            let spec = hound::WavSpec {
                channels: input.channels() as u16,
                sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            Some(hound::WavWriter::create(path, spec).context("opening the recording")?)
        }
        _ => None,
    };

    let expected = Duration::from_secs_f64(args.lead_in_secs + take_secs * f64::from(args.repeat));
    let deadline = expected.mul_f64(1.2) + Duration::from_secs(5);
    let tail = Duration::from_secs_f64(args.tail_secs);
    let rec_channels = input.as_ref().map_or(0, |i| i.channels() as usize);
    let block_frames = 4096_usize;
    let mut buf = vec![0.0_f32; block_frames * rec_channels.max(1)];
    let mut reader = input.as_ref().map(|i| AllChannels::new(i, block_frames));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a few seconds of frames"
    )]
    let echo_frames =
        lead_in_frames + (ECHO_SEARCH_SECS * f64::from(sample_rate)) as usize + ECHO_WINDOW_FRAMES;
    let mut first_channel: Vec<f32> = Vec::with_capacity(echo_frames);
    let mut recorded: u64 = 0;
    let mut peak: f32 = 0.0;

    let start = Instant::now();
    let mut done_at: Option<Instant> = None;
    loop {
        if done_at.is_none() && progress.done.load(Ordering::Acquire) {
            done_at = Some(Instant::now());
        }
        if done_at.is_some_and(|t| t.elapsed() >= tail) {
            break;
        }
        if start.elapsed() > deadline {
            bail!(
                "the take did not finish in {:.0} s: the output device stopped calling back \
                 ({} callbacks, {} of {} frames played)",
                deadline.as_secs_f64(),
                output.callback_count(),
                progress.played.load(Ordering::Relaxed),
                take_frames as u64 * u64::from(args.repeat)
            );
        }
        let (Some(input), Some(reader), Some(writer)) =
            (input.as_mut(), reader.as_mut(), writer.as_mut())
        else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        let n = reader.read(input, &mut buf);
        if n == 0 {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        for frame in buf[..n * rec_channels].chunks_exact(rec_channels) {
            if first_channel.len() < echo_frames {
                first_channel.push(frame[0]);
            }
            for s in frame {
                writer.write_sample(*s).context("writing sample")?;
                peak = peak.max(s.abs());
            }
        }
        recorded += n as u64;
    }
    let wall = start.elapsed();
    if let Some(writer) = writer {
        writer.finalize().context("finalizing the recording")?;
    }

    println!();
    println!(
        "  played:     {} frames, {} output callbacks",
        progress.played.load(Ordering::Relaxed),
        output.callback_count()
    );
    if let Some(input) = &input {
        let sr = f64::from(sample_rate);
        println!(
            "  recorded:   {recorded} frames ({:.3} s), overflow={}",
            recorded as f64 / sr,
            input.overflow_count()
        );
        println!(
            "  peak:       {peak:.4} ({:.2} dBFS)",
            20.0 * f64::from(peak.max(1e-10)).log10()
        );
        let take_left: Vec<f32> = take.chunks_exact(2).map(|f| f[0] * args.gain).collect();
        println!(
            "  {}",
            describe_echo(find_echo(&take_left, &first_channel), sr)
        );
        if input.overflow_count() > 0 {
            eprintln!(
                "warning: input ringbuf overflowed {} times — the recording is missing samples",
                input.overflow_count()
            );
        }
        if let Some(excess) = clock_excess(recorded as f64 / sr, wall.as_secs_f64()) {
            eprintln!(
                "warning: the device delivered {:+.2} % more audio than time passed — \
                 do not measure pitch from this recording",
                excess * 100.0
            );
        }
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

    fn player(take: &[f32], left: usize, right: usize, lead_in: usize, passes: u32) -> TakePlayer {
        TakePlayer {
            take: Arc::from(take),
            left,
            right,
            gain: 1.0,
            lead_in_frames: lead_in,
            cursor: 0,
            passes_left: passes,
            progress: Arc::new(Progress::default()),
        }
    }

    #[test]
    fn defaults_are_the_aux_pair_both_ways() {
        let a = parse_args(&s(&["take.flac"])).unwrap();
        assert_eq!(a.output_channels, (1, 2));
        assert_eq!(a.input_channels, vec![1, 2]);
        assert_eq!(a.repeat, 1);
        assert!(a.record.is_none());
        assert!(a.take_channels.is_none());
    }

    #[test]
    fn parses_every_flag() {
        let a = parse_args(&s(&[
            "--device",
            "SL 3",
            "take.flac",
            "--record",
            "out.wav",
            "--output-channels",
            "5,6",
            "--input-channels",
            "1,2,3,4",
            "--from",
            "10",
            "--secs",
            "2.5",
            "--repeat",
            "3",
            "--gain",
            "0.5",
            "--lead-in",
            "0",
            "--tail",
            "4",
            "--take-channels",
            "3,4",
        ]))
        .unwrap();
        assert_eq!(a.take_channels, Some((3, 4)));
        assert_eq!(a.take, PathBuf::from("take.flac"));
        assert_eq!(a.record, Some(PathBuf::from("out.wav")));
        assert_eq!(a.device.as_deref(), Some("SL 3"));
        assert_eq!(a.output_channels, (5, 6));
        assert_eq!(a.input_channels, vec![1, 2, 3, 4]);
        assert!((a.from_secs - 10.0).abs() < 1e-9);
        assert_eq!(a.secs, Some(2.5));
        assert_eq!(a.repeat, 3);
        assert!((a.gain - 0.5).abs() < 1e-6);
        assert!(a.lead_in_secs.abs() < 1e-9);
        assert!((a.tail_secs - 4.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_what_cannot_be_played() {
        for bad in [
            &["--record", "out.wav"][..],
            &["take.flac", "other.flac"],
            &["take.flac", "--output-channels", "1"],
            &["take.flac", "--output-channels", "2,2"],
            &["take.flac", "--output-channels", "0,1"],
            &["take.flac", "--input-channels", "3"],
            &["take.flac", "--repeat", "0"],
            &["take.flac", "--gain", "-1"],
            &["take.flac", "--from", "-2"],
            &["take.flac", "--loud"],
            &["take.flac", "--secs"],
            &["take.flac", "--take-channels", "3"],
            &["take.flac", "--take-channels", "0,1"],
        ] {
            assert!(parse_args(&s(bad)).is_err(), "{bad:?} should be refused");
        }
    }

    /// A `make rig` take holds every input of the interface; a deck's
    /// pair out of it is what goes back through the loop.
    #[test]
    fn a_pair_comes_out_of_a_wider_take() {
        // Three frames of six channels: frame f, channel c is 10f + c.
        let take: Vec<f32> = (0..3)
            .flat_map(|f| (1..=6).map(move |c| (10 * f + c) as f32))
            .collect();
        assert_eq!(
            pick_pair(&take, 6, (3, 4)),
            vec![3.0, 4.0, 13.0, 14.0, 23.0, 24.0]
        );
        assert_eq!(
            pick_pair(&take, 6, (6, 5)),
            vec![6.0, 5.0, 16.0, 15.0, 26.0, 25.0]
        );
    }

    #[test]
    fn a_take_written_by_the_coach_reads_back_as_its_pair() {
        let path =
            std::env::temp_dir().join(format!("dub-loopback-pair-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 6,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for f in 0..4 {
            for c in 1..=6 {
                w.write_sample((10 * f + c) as f32).unwrap();
            }
        }
        w.finalize().unwrap();
        let (pair, rate) = read_pair(&path, (5, 6)).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(pair, vec![5.0, 6.0, 15.0, 16.0, 25.0, 26.0, 35.0, 36.0]);
        assert!(
            read_pair(&path, (7, 8)).is_err(),
            "the take has six channels"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn excerpt_cuts_whole_frames() {
        let take: Vec<f32> = (0..20).map(|i| i as f32).collect();
        // 10 frames at 2 Hz: seconds 1..3 are frames 2..6.
        assert_eq!(
            excerpt(&take, 2, 1.0, Some(2.0)).unwrap(),
            &take[4..12],
            "from and secs are in seconds of the take"
        );
        assert_eq!(excerpt(&take, 2, 4.0, None).unwrap(), &take[16..]);
        assert_eq!(excerpt(&take, 2, 4.0, Some(60.0)).unwrap(), &take[16..]);
        assert!(excerpt(&take, 2, 5.0, None).is_err());
        assert!(excerpt(&take, 2, 1.0, Some(0.0)).is_err());
    }

    #[test]
    fn the_take_lands_on_its_pair_and_nowhere_else() {
        let mut p = player(&[0.1, 0.2, 0.3, 0.4], 4, 5, 0, 1);
        let mut out = [9.0_f32; 12];
        p.render(&mut out, 6);
        assert_eq!(
            out,
            [0.0, 0.0, 0.0, 0.0, 0.1, 0.2, 0.0, 0.0, 0.0, 0.0, 0.3, 0.4]
        );
        assert!(p.progress.done.load(Ordering::Acquire));
        assert_eq!(p.progress.played.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn lead_in_is_silence_and_spans_callbacks() {
        let mut p = player(&[0.5, -0.5], 0, 1, 3, 1);
        let mut out = [9.0_f32; 4];
        p.render(&mut out, 2);
        assert_eq!(out, [0.0; 4]);
        assert!(!p.progress.done.load(Ordering::Acquire));
        p.render(&mut out, 2);
        assert_eq!(out, [0.0, 0.0, 0.5, -0.5]);
        assert!(p.progress.done.load(Ordering::Acquire));
    }

    #[test]
    fn repeats_run_back_to_back_then_silence() {
        let mut p = player(&[0.1, 0.1, 0.2, 0.2], 0, 1, 0, 2);
        let mut out = [9.0_f32; 12];
        p.render(&mut out, 2);
        assert_eq!(
            out,
            [0.1, 0.1, 0.2, 0.2, 0.1, 0.1, 0.2, 0.2, 0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(p.progress.played.load(Ordering::Relaxed), 4);
        p.render(&mut out, 2);
        assert_eq!(out, [0.0; 12], "after the last pass the pair is silent");
        assert_eq!(p.progress.played.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn gain_scales_both_sides() {
        let mut p = player(&[0.4, -0.8], 0, 1, 0, 1);
        p.gain = 0.5;
        let mut out = [0.0_f32; 2];
        p.render(&mut out, 2);
        assert_eq!(out, [0.2, -0.4]);
    }

    /// Deterministic noise in −1..1.
    fn noise(n: usize, mut seed: u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / 8_388_608.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn finds_a_delayed_quieter_copy() {
        let take = noise(2_000, 7);
        let mut recording = vec![0.0_f32; 700];
        recording.extend(take.iter().map(|s| s * 0.5));
        recording.extend(std::iter::repeat_n(0.0, 300));

        let echo = find_echo(&take, &recording).expect("the copy is there");
        assert_eq!(echo.at_frame, 700);
        assert!(echo.correlation > 0.999, "{echo:?}");
        assert!((echo.gain_db + 6.02).abs() < 0.01, "{echo:?}");
    }

    #[test]
    fn a_silent_head_does_not_move_the_answer() {
        let mut take = vec![0.0_f32; 400];
        take.extend(noise(2_000, 3));
        let mut recording = vec![0.0_f32; 250];
        recording.extend(take.iter().copied());

        let echo = find_echo(&take, &recording).expect("the copy is there");
        assert_eq!(
            echo.at_frame, 250,
            "the take's first frame, not its first sound"
        );
        assert!(echo.correlation > 0.999);
    }

    #[test]
    fn another_signal_is_not_the_take() {
        let echo = find_echo(&noise(2_000, 1), &noise(6_000, 2)).expect("both have signal");
        assert!(echo.correlation < 0.2, "{echo:?}");
        assert!(describe_echo(Some(echo), 48_000.0).contains("not the take"));
    }

    #[test]
    fn unix_seconds_keep_milliseconds() {
        let t = UNIX_EPOCH + Duration::from_millis(1_764_250_000_123);
        assert!((unix_secs(t) - 1_764_250_000.123).abs() < 1e-6);
    }

    #[test]
    fn silence_has_no_echo() {
        assert!(find_echo(&[0.0; 100], &noise(500, 1)).is_none());
        assert!(find_echo(&noise(500, 1), &[0.0; 10]).is_none());
        let floor: Vec<f32> = noise(2_000, 5).iter().map(|s| s * 1e-4).collect();
        assert!(
            find_echo(&noise(500, 1), &floor).is_none(),
            "a noise floor is not a signal"
        );
        assert!(describe_echo(None, 48_000.0).contains("silent"));
    }
}
