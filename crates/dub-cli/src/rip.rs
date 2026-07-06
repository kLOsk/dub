//! `dub rip` — headless M26a vinyl-rip dogfood.
//!
//! Records the input device straight into a `dub-rip` session
//! (crash-safe WAV spill + live envelope), then splits, encodes
//! (FLAC + Vorbis tags), imports into the library pre-analyzed, and
//! archives the side. This is the whole rip pipeline minus the
//! review UI: split points come from `--splits`, metadata from
//! flags. Runs against the same rig as `dub capture`
//! (`--input-channels 3,4` = SL3 deck A).

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use dub_audio::AudioInput;
use dub_rip::{RipConfig, RipSession, RipState, TrackMeta};

use crate::input_cmds::parse_input_args;

struct RipArgs {
    out_parent: Option<PathBuf>,
    library: Option<PathBuf>,
    splits_secs: Vec<f64>,
    titles: Vec<String>,
    artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    year: Option<i32>,
    max_duration: Option<f64>,
}

pub fn run(args: &[String]) -> Result<()> {
    let (input_args, leftover) = parse_input_args(args)?;
    let rip_args = parse_rip_args(&leftover)?;

    // ---- Open the interface and hand its ring to the session ----
    let opts = input_args.to_options();
    let mut input = AudioInput::start_with_options(&opts).context("opening input device")?;
    if input.channels() != 2 {
        return Err(anyhow!(
            "rip needs a stereo pair; got {} channels (use --input-channels N,M)",
            input.channels()
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let sample_rate = input.sample_rate().round() as u32;

    let session_dir = session_dir(rip_args.out_parent.clone())?;
    let mut cfg = RipConfig::new(sample_rate, session_dir.clone());
    if let Some(max) = rip_args.max_duration {
        #[allow(clippy::cast_possible_truncation)]
        let max_f32 = max as f32;
        cfg.max_duration_secs = max_f32;
    }
    let mut session = RipSession::new(cfg).context("creating rip session")?;
    let rx = input
        .take_consumer()
        .ok_or_else(|| anyhow!("input consumer already taken"))?;
    session.arm(rx).context("arming capture")?;

    println!(
        "rip: device='{}' sr={} Hz  session={}",
        input.device_name(),
        sample_rate,
        session_dir.display()
    );

    // ---- Record ---------------------------------------------------
    if let Some(duration) = input_args.duration {
        session.start().context("starting capture")?;
        eprintln!("recording for {duration:.1} s …");
        let deadline = Instant::now() + Duration::from_secs_f64(duration);
        while Instant::now() < deadline {
            print_meter(&session);
            std::thread::sleep(Duration::from_millis(200));
        }
    } else {
        eprintln!("ARMED — cue the record, press Enter to start recording");
        wait_for_enter_blocking();
        session.start().context("starting capture")?;
        eprintln!("RECORDING — press Enter to stop");
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop_flag);
        std::thread::spawn(move || {
            wait_for_enter_blocking();
            flag.store(true, Ordering::Release);
        });
        while !stop_flag.load(Ordering::Acquire) {
            if !matches!(
                session.status().state,
                RipState::Recording | RipState::Armed
            ) {
                break; // max-duration cap or input loss stopped us
            }
            print_meter(&session);
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    if matches!(
        session.status().state,
        RipState::Recording | RipState::Armed
    ) {
        session.stop().context("stopping capture")?;
    }
    let status = session
        .wait_stopped(Duration::from_secs(30))
        .context("waiting for capture to finish")?;
    eprintln!();
    println!(
        "captured {:.1} s ({} frames), stopped: {:?}",
        status.elapsed_secs, status.recorded_frames, status.state
    );

    // ---- Split + metadata ------------------------------------------
    let boundaries: Vec<u64> = rip_args
        .splits_secs
        .iter()
        .map(|&s| secs_to_frames(s, sample_rate))
        .collect();
    session
        .set_splits(boundaries)
        .context("applying --splits")?;
    let segment_count = session.manifest().tracks.len().max(1);
    for i in 0..segment_count {
        let meta = TrackMeta {
            title: rip_args.titles.get(i).cloned(),
            artist: rip_args.artist.clone(),
            album: rip_args.album.clone(),
            genre: rip_args.genre.clone(),
            year: rip_args.year,
        };
        if meta != TrackMeta::default() {
            session.set_track_meta(i, meta)?;
        }
    }

    // ---- Commit ------------------------------------------------------
    let mut library = match &rip_args.library {
        Some(path) => dub_library::Library::open_at(path).context("opening library")?,
        None => dub_library::Library::open_default().context("opening default library")?,
    };
    println!("encoding + importing {segment_count} track(s) …");
    let outcome = session.commit(&mut library).context("commit failed")?;

    for seg in &outcome.segments {
        let name = seg
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        match (&seg.library_uuid, &seg.error) {
            (Some(uuid), None) => {
                println!(
                    "  {:>2}. {}  → {uuid}",
                    seg.index + 1,
                    name.unwrap_or_default()
                );
            }
            (Some(uuid), Some(err)) => {
                println!(
                    "  {:>2}. {}  → {uuid}  (analysis pending: {err})",
                    seg.index + 1,
                    name.unwrap_or_default()
                );
            }
            (None, err) => {
                println!(
                    "  {:>2}. FAILED: {}",
                    seg.index + 1,
                    err.clone().unwrap_or_else(|| "unknown".into())
                );
            }
        }
    }
    if let Some(archive) = &outcome.archive {
        println!("side archive: {}", archive.display());
    }
    if outcome.spill_removed {
        println!("spill removed (side archived losslessly)");
    }
    if outcome.is_complete() {
        println!("OK");
        Ok(())
    } else {
        Err(anyhow!(
            "rip incomplete — re-run `dub rip` semantics: session dir {} keeps the spill + manifest for retry",
            session_dir.display()
        ))
    }
}

fn parse_rip_args(leftover: &[String]) -> Result<RipArgs> {
    let mut out = RipArgs {
        out_parent: None,
        library: None,
        splits_secs: Vec::new(),
        titles: Vec::new(),
        artist: None,
        album: None,
        genre: None,
        year: None,
        max_duration: None,
    };
    let mut iter = leftover.iter();
    while let Some(arg) = iter.next() {
        let mut value = |flag: &str| -> Result<String> {
            iter.next()
                .cloned()
                .ok_or_else(|| anyhow!("{flag} expects a value"))
        };
        match arg.as_str() {
            "--out" | "-o" => out.out_parent = Some(PathBuf::from(value("--out")?)),
            "--library" => out.library = Some(PathBuf::from(value("--library")?)),
            "--splits" => {
                let raw = value("--splits")?;
                let parsed: Result<Vec<f64>, _> =
                    raw.split(',').map(|s| s.trim().parse::<f64>()).collect();
                out.splits_secs = parsed.context("--splits values must be seconds")?;
            }
            "--titles" => {
                out.titles = value("--titles")?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
            }
            "--artist" => out.artist = Some(value("--artist")?),
            "--album" => out.album = Some(value("--album")?),
            "--genre" => out.genre = Some(value("--genre")?),
            "--year" => {
                out.year = Some(value("--year")?.parse().context("--year not an integer")?);
            }
            "--max-duration" => {
                out.max_duration = Some(
                    value("--max-duration")?
                        .parse()
                        .context("--max-duration not a number")?,
                );
            }
            other => return Err(anyhow!("unknown rip flag: {other}")),
        }
    }
    Ok(out)
}

/// `<parent>/<YYYYMMDD-HHMMSS>` under `~/Music/Dub/Rips` by default.
/// The session dir is user-visible data: encoded tracks are imported
/// in place and the side archive lives here permanently.
fn session_dir(parent: Option<PathBuf>) -> Result<PathBuf> {
    let parent = match parent {
        Some(p) => p,
        None => dirs::audio_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow!("cannot resolve a music directory; pass --out DIR"))?
            .join("Dub")
            .join("Rips"),
    };
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    Ok(parent.join(stamp))
}

fn secs_to_frames(secs: f64, sample_rate: u32) -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (secs * f64::from(sample_rate)).round() as u64;
    frames
}

fn print_meter(session: &RipSession) {
    let status = session.status();
    let peak_db = 20.0 * f64::from(status.window_peak.max(1e-6)).log10();
    eprint!(
        "\r  {}  {:7.1} s   peak {:6.1} dBFS   ",
        match status.state {
            RipState::Recording => "REC",
            RipState::Armed => "ARM",
            _ => "···",
        },
        status.elapsed_secs,
        peak_db
    );
    let _ = std::io::stderr().flush();
}

fn wait_for_enter_blocking() {
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rip_flags() {
        let args: Vec<String> = [
            "--splits",
            "185.5,392",
            "--artist",
            "Sound Dimension",
            "--album",
            "Studio One",
            "--titles",
            "Real Rock, Version ,Dub",
            "--year",
            "1967",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let parsed = parse_rip_args(&args).unwrap();
        assert_eq!(parsed.splits_secs, vec![185.5, 392.0]);
        assert_eq!(parsed.artist.as_deref(), Some("Sound Dimension"));
        assert_eq!(
            parsed.titles,
            vec!["Real Rock".to_string(), "Version".into(), "Dub".into()]
        );
        assert_eq!(parsed.year, Some(1967));
    }

    #[test]
    fn rejects_unknown_flags_and_bad_values() {
        let bad: Vec<String> = vec!["--frobnicate".into()];
        assert!(parse_rip_args(&bad).is_err());
        let bad_splits: Vec<String> = vec!["--splits".into(), "abc".into()];
        assert!(parse_rip_args(&bad_splits).is_err());
    }

    #[test]
    fn secs_to_frames_rounds() {
        assert_eq!(secs_to_frames(1.0, 48_000), 48_000);
        assert_eq!(secs_to_frames(0.5, 44_100), 22_050);
    }
}
