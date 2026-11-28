//! `dub timecode-gen` — the rig's turntable motions, as a timecode WAV.
//!
//! Section 2 of `docs/RIG-CHECKLIST.md` moves the record: a needle lifted
//! and dropped elsewhere, the platter stopped by hand and started again,
//! hard scratching, the pitch fader at −8 and +8 %. Rendered from the
//! clean-room generator and played through the loop cable (`dub
//! loopback`) into a deck on TC, they show how Dub reacts to each motion
//! with nobody at the decks. Real vinyl's surface noise is not in them;
//! that takes a recorded `make rig` take.
//!
//! The level follows the platter, as a cartridge's does: it reads
//! velocity, so a slow drag is quiet and a stopped record is silent. A
//! hiss at −80 dBFS stands in for the interface's own floor.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use dub_timecode::signal::Generator;
use dub_timecode::Format;

const USAGE: &str = "usage: dub timecode-gen <out.wav> [--motion lift|stop|scratch|pitch|all] \
     [--format serato-cv02|traktor-mk1|traktor-mk2] [--sr HZ] [--level PEAK] [--am-depth D]";

/// Peak level at unity: a cartridge into a phono-level input, where the
/// SL 3 captures of the control records sit.
const DEFAULT_LEVEL: f32 = 0.35;

/// The interface's own floor, so a lifted needle reads as an input that
/// is there and quiet rather than as digital zero.
const HISS: f32 = 1e-4;

/// The fastest a hand moves a record, as a multiple of unity: past it
/// the cartridge level stops rising.
const LEVEL_RATE_CAP: f64 = 1.5;

/// One stretch of the record's motion.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Seg {
    /// The platter turning, its rate moving linearly from `from` to `to`
    /// (a steady speed when they match; a hand stop or start when not).
    Turn { secs: f64, from: f64, to: f64 },
    /// Back and forth, `depth × sin(2π·hz·t)` on top of nothing: whole
    /// strokes, so the needle ends where it started.
    Scratch { secs: f64, depth: f64, hz: f64 },
    /// The needle up for `secs`, then down `jump_secs` of groove further
    /// on (negative: back).
    Lifted { secs: f64, jump_secs: f64 },
}

impl Seg {
    fn secs(self) -> f64 {
        match self {
            Seg::Turn { secs, .. } | Seg::Scratch { secs, .. } | Seg::Lifted { secs, .. } => secs,
        }
    }
}

/// What a check needs to know about the file: where each stretch starts
/// and what it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub at_secs: f64,
    pub what: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Motion {
    Lift,
    Stop,
    Scratch,
    Pitch,
    All,
}

impl Motion {
    fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "lift" => Self::Lift,
            "stop" => Self::Stop,
            "scratch" => Self::Scratch,
            "pitch" => Self::Pitch,
            "all" => Self::All,
            _ => return None,
        })
    }

    /// The motion as stretches, each named for the run file.
    fn script(self) -> Vec<(Seg, &'static str)> {
        let play = |secs| {
            (
                Seg::Turn {
                    secs,
                    from: 1.0,
                    to: 1.0,
                },
                "play",
            )
        };
        match self {
            // Rig 2: "Lift the needle mid-play, drop it elsewhere".
            Self::Lift => vec![
                play(10.0),
                (
                    Seg::Lifted {
                        secs: 1.5,
                        jump_secs: 60.0,
                    },
                    "needle up, down 60 s on",
                ),
                play(10.0),
            ],
            // Rig 2: "Stop the platter by hand, start it".
            Self::Stop => vec![
                play(8.0),
                (
                    Seg::Turn {
                        secs: 0.4,
                        from: 1.0,
                        to: 0.0,
                    },
                    "hand stops the platter",
                ),
                (
                    Seg::Turn {
                        secs: 2.0,
                        from: 0.0,
                        to: 0.0,
                    },
                    "held still",
                ),
                (
                    Seg::Turn {
                        secs: 0.35,
                        from: 0.0,
                        to: 1.0,
                    },
                    "let go",
                ),
                play(8.0),
            ],
            // Rig 2: "Scratch hard": strokes three times unity, 2.5 a second.
            Self::Scratch => vec![
                play(5.0),
                (
                    Seg::Scratch {
                        secs: 6.0,
                        depth: 3.0,
                        hz: 2.5,
                    },
                    "scratching",
                ),
                play(5.0),
            ],
            // Rig 2 / 3: the fader stepped from −8 to +8 %.
            Self::Pitch => vec![
                play(5.0),
                (
                    Seg::Turn {
                        secs: 0.15,
                        from: 1.0,
                        to: 0.92,
                    },
                    "fader to -8 %",
                ),
                (
                    Seg::Turn {
                        secs: 8.0,
                        from: 0.92,
                        to: 0.92,
                    },
                    "at -8 %",
                ),
                (
                    Seg::Turn {
                        secs: 0.3,
                        from: 0.92,
                        to: 1.08,
                    },
                    "fader to +8 %",
                ),
                (
                    Seg::Turn {
                        secs: 8.0,
                        from: 1.08,
                        to: 1.08,
                    },
                    "at +8 %",
                ),
                (
                    Seg::Turn {
                        secs: 0.15,
                        from: 1.08,
                        to: 1.0,
                    },
                    "fader to 0",
                ),
                play(5.0),
            ],
            Self::All => [Self::Lift, Self::Stop, Self::Scratch, Self::Pitch]
                .into_iter()
                .flat_map(|m| {
                    let mut s = m.script();
                    s.push(play(2.0));
                    s
                })
                .collect(),
        }
    }
}

/// How far a 0 bit pulls the data channel down. Shallow enough that the
/// rate reads true: at 0.5 (the generator's unit-test depth) Dub's decoder
/// read a steady unity carrier 0.4 % fast offline, and the deck +0.7 %
/// through the loop, wandering, which a real pressing does not; at 0.15 it
/// reads 1.0000, −8 % and +8 % exactly, and the position still decodes
/// (2026-11-28).
const DEFAULT_AM_DEPTH: f32 = 0.15;

/// Render `motion` as interleaved stereo at `sr`, with its marks. `am_depth`
/// 0 is a bare carrier, with no position in it.
fn render(
    motion: Motion,
    format: Format,
    sr: u32,
    level: f32,
    am_depth: f32,
) -> (Vec<f32>, Vec<Mark>) {
    #[allow(clippy::cast_precision_loss)]
    let srf = sr as f32;
    let mut gen = Generator::new(format, srf);
    if am_depth > 0.0 {
        gen.enable_absolute(format, am_depth);
    }
    let script = motion.script();
    let total: f64 = script.iter().map(|(s, _)| s.secs()).sum();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut out = Vec::with_capacity((total * f64::from(sr)) as usize * 2 + 2);
    let mut marks = Vec::with_capacity(script.len());
    let mut noise: u32 = 0x2545_f491;
    let mut frame = [0.0_f32; 2];
    let mut elapsed = 0.0_f64;
    for (seg, name) in script {
        marks.push(Mark {
            at_secs: elapsed,
            what: name.to_string(),
        });
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = (seg.secs() * f64::from(sr)).round() as usize;
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f64 / f64::from(sr);
            let rate = match seg {
                Seg::Turn { secs, from, to } => from + (to - from) * (t / secs).min(1.0),
                Seg::Scratch { depth, hz, .. } => depth * (std::f64::consts::TAU * hz * t).sin(),
                Seg::Lifted { .. } => 0.0,
            };
            let lifted = matches!(seg, Seg::Lifted { .. });
            #[allow(clippy::cast_possible_truncation)]
            let amp = if lifted {
                0.0
            } else {
                level * (rate.abs().min(LEVEL_RATE_CAP) as f32)
            };
            gen.render(&mut frame, rate, amp);
            for s in &mut frame {
                noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                #[allow(clippy::cast_precision_loss)]
                let n = (noise >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                *s += n * 2.0 * HISS;
            }
            out.extend_from_slice(&frame);
        }
        if let Seg::Lifted { jump_secs, .. } = seg {
            #[allow(clippy::cast_possible_truncation)]
            let cycles = (jump_secs * f64::from(format.carrier_hz())).round() as i64;
            gen.skip_cycles(cycles);
        }
        elapsed += seg.secs();
    }
    (out, marks)
}

#[derive(Debug, Clone, PartialEq)]
struct Args {
    out: PathBuf,
    motion: Motion,
    format: Format,
    sr: u32,
    level: f32,
    am_depth: f32,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let mut out = None;
    let mut motion = Motion::All;
    let mut format = Format::SeratoCv02;
    let mut sr = 48_000;
    let mut level = DEFAULT_LEVEL;
    let mut am_depth = DEFAULT_AM_DEPTH;
    let mut i = 0;
    while i < args.len() {
        let value = || {
            args.get(i + 1)
                .ok_or_else(|| anyhow!("{} needs a value\n{USAGE}", args[i]))
        };
        match args[i].as_str() {
            "--motion" => {
                let v = value()?;
                motion =
                    Motion::parse(v).ok_or_else(|| anyhow!("unknown --motion '{v}'\n{USAGE}"))?;
                i += 2;
            }
            "--format" => {
                let v = value()?;
                format = Format::from_cli_arg(v)
                    .ok_or_else(|| anyhow!("unknown --format '{v}'\n{USAGE}"))?;
                i += 2;
            }
            "--sr" => {
                sr = value()?.parse().context("--sr expects an integer")?;
                if sr < 8_000 {
                    bail!("--sr {sr} is not an audio rate");
                }
                i += 2;
            }
            "--level" => {
                level = value()?.parse().context("--level expects a number")?;
                if !(0.0..=1.0).contains(&level) || level == 0.0 {
                    bail!("--level must be in (0, 1]");
                }
                i += 2;
            }
            "--am-depth" => {
                am_depth = value()?.parse().context("--am-depth expects a number")?;
                if !(0.0..0.95).contains(&am_depth) {
                    bail!("--am-depth must be in [0, 0.95)");
                }
                i += 2;
            }
            s if s.starts_with('-') => bail!("unknown flag {s}\n{USAGE}"),
            s => {
                if out.replace(PathBuf::from(s)).is_some() {
                    bail!("one output file\n{USAGE}");
                }
                i += 1;
            }
        }
    }
    Ok(Args {
        out: out.ok_or_else(|| anyhow!("{USAGE}"))?,
        motion,
        format,
        sr,
        level,
        am_depth,
    })
}

fn write_wav(path: &Path, samples: &[f32], sr: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sr,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)
        .with_context(|| format!("creating {}", path.display()))?;
    for &s in samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

/// # Errors
/// Bad arguments, or the file could not be written.
pub fn run(args: &[String]) -> Result<()> {
    let args = parse_args(args)?;
    let (samples, marks) = render(args.motion, args.format, args.sr, args.level, args.am_depth);
    write_wav(&args.out, &samples, args.sr)?;
    #[allow(clippy::cast_precision_loss)]
    let secs = samples.len() as f64 / 2.0 / f64::from(args.sr);
    println!("wrote {} ({secs:.1} s, {} Hz)", args.out.display(), args.sr);
    for m in marks {
        println!("  {:7.2} s  {}", m.at_secs, m.what);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    fn window(samples: &[f32], from: f64, to: f64) -> &[f32] {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at = |t: f64| ((t * f64::from(SR)) as usize * 2).min(samples.len());
        &samples[at(from)..at(to)]
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0_f32, |m, v| m.max(v.abs()))
    }

    /// Upward zero crossings of the left channel, per second: the carrier
    /// frequency the deck would read.
    fn carrier_hz(x: &[f32]) -> f64 {
        let left: Vec<f32> = x.chunks_exact(2).map(|f| f[0]).collect();
        let ups = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        #[allow(clippy::cast_precision_loss)]
        let secs = left.len() as f64 / f64::from(SR);
        #[allow(clippy::cast_precision_loss)]
        let hz = ups as f64 / secs;
        hz
    }

    fn mark(marks: &[Mark], what: &str) -> f64 {
        marks.iter().find(|m| m.what == what).unwrap().at_secs
    }

    #[test]
    fn a_lifted_needle_is_the_floor_and_lands_further_on() {
        let (out, marks) = render(
            Motion::Lift,
            Format::SeratoCv02,
            SR,
            DEFAULT_LEVEL,
            DEFAULT_AM_DEPTH,
        );
        let up = mark(&marks, "needle up, down 60 s on");
        assert!(peak(window(&out, up + 0.05, up + 1.45)) < 3.0 * HISS);
        assert!(peak(window(&out, 1.0, 9.0)) > 0.3);
        assert!(peak(window(&out, up + 1.6, up + 9.0)) > 0.3);

        // The groove under the needle moved on 60 s.
        let mut gen = Generator::new(Format::SeratoCv02, 48_000.0);
        gen.enable_absolute(Format::SeratoCv02, 0.5);
        let mut buf = vec![0.0_f32; (10 * SR) as usize * 2];
        gen.render(&mut buf, 1.0, DEFAULT_LEVEL);
        let before = gen.absolute_position().unwrap();
        gen.skip_cycles(60_000);
        assert_eq!(gen.absolute_position().unwrap() - before, 60_000);
    }

    #[test]
    fn a_stopped_platter_is_silent_and_starts_again() {
        let (out, marks) = render(
            Motion::Stop,
            Format::SeratoCv02,
            SR,
            DEFAULT_LEVEL,
            DEFAULT_AM_DEPTH,
        );
        let held = mark(&marks, "held still");
        assert!(peak(window(&out, held + 0.05, held + 1.95)) < 3.0 * HISS);
        let go = mark(&marks, "let go");
        let hz = carrier_hz(window(&out, go + 1.0, go + 7.0));
        assert!((hz - 1_000.0).abs() < 5.0, "back at unity: {hz} Hz");
    }

    #[test]
    fn the_pitch_steps_move_the_carrier_by_eight_percent() {
        let (out, marks) = render(
            Motion::Pitch,
            Format::SeratoCv02,
            SR,
            DEFAULT_LEVEL,
            DEFAULT_AM_DEPTH,
        );
        let low = mark(&marks, "at -8 %");
        let high = mark(&marks, "at +8 %");
        let lo = carrier_hz(window(&out, low + 0.5, low + 7.5));
        let hi = carrier_hz(window(&out, high + 0.5, high + 7.5));
        assert!((lo - 920.0).abs() < 3.0, "{lo}");
        assert!((hi - 1_080.0).abs() < 3.0, "{hi}");
    }

    #[test]
    fn a_scratch_runs_both_ways_and_ends_where_it_began() {
        let (out, marks) = render(
            Motion::Scratch,
            Format::SeratoCv02,
            SR,
            DEFAULT_LEVEL,
            DEFAULT_AM_DEPTH,
        );
        let from = mark(&marks, "scratching");
        // Hard strokes are louder than unity play, up to the cap.
        assert!(peak(window(&out, from, from + 6.0)) > DEFAULT_LEVEL * 1.4);
        // Whole strokes: the motion's integral, and so the groove
        // position, comes back to where the scratch began.
        let strokes: f64 = (0..6 * 48_000)
            .map(|i| 3.0 * (std::f64::consts::TAU * 2.5 * f64::from(i) / 48_000.0).sin())
            .sum::<f64>()
            / 48_000.0;
        assert!(strokes.abs() < 1e-6, "{strokes}");
    }

    /// The default modulation leaves the rate true through Dub's own
    /// decoder (at 0.5 it read a steady unity carrier 0.4 % fast, and a
    /// deck's pitch and its waveform's zoom wandered with it).
    #[test]
    fn the_decoder_reads_the_default_modulation_at_its_true_rate() {
        let (out, marks) = render(
            Motion::Pitch,
            Format::SeratoCv02,
            SR,
            DEFAULT_LEVEL,
            DEFAULT_AM_DEPTH,
        );
        let median_rate = |from: f64, to: f64| {
            let mut dec = dub_timecode::Decoder::with_absolute(Format::SeratoCv02, 48_000.0);
            dec.calibrate(&out);
            let mut rates: Vec<f64> = window(&out, from, to)
                .chunks(1_200 * 2)
                .map(|b| dec.process(b).rate)
                .skip(4)
                .collect();
            rates.sort_by(f64::total_cmp);
            rates[rates.len() / 2]
        };
        let unity = median_rate(0.5, 4.5);
        let low = median_rate(mark(&marks, "at -8 %"), mark(&marks, "at -8 %") + 7.5);
        let high = median_rate(mark(&marks, "at +8 %"), mark(&marks, "at +8 %") + 7.5);
        assert!((unity - 1.0).abs() < 0.001, "unity read {unity}");
        assert!((low - 0.92).abs() < 0.001, "-8 % read {low}");
        assert!((high - 1.08).abs() < 0.001, "+8 % read {high}");
    }

    #[test]
    fn all_is_every_motion_in_turn() {
        let (out, marks) = render(
            Motion::All,
            Format::SeratoCv02,
            SR,
            DEFAULT_LEVEL,
            DEFAULT_AM_DEPTH,
        );
        for what in [
            "needle up, down 60 s on",
            "held still",
            "scratching",
            "at +8 %",
        ] {
            assert!(marks.iter().any(|m| m.what == what), "{what} missing");
        }
        #[allow(clippy::cast_precision_loss)]
        let secs = out.len() as f64 / 2.0 / f64::from(SR);
        let want: f64 = Motion::All.script().iter().map(|(s, _)| s.secs()).sum();
        assert!((secs - want).abs() < 0.01, "{secs} vs {want}");
    }

    #[test]
    fn args_name_the_file_and_choose_the_motion() {
        let a = parse_args(&s(&[
            "take.wav",
            "--motion",
            "stop",
            "--format",
            "traktor-mk2",
        ]))
        .unwrap();
        assert_eq!(a.out, PathBuf::from("take.wav"));
        assert_eq!(a.motion, Motion::Stop);
        assert_eq!(a.format, Format::TraktorMk2);
        assert_eq!(a.sr, 48_000);
        assert!(parse_args(&s(&[])).is_err());
        assert!(parse_args(&s(&["a.wav", "--motion", "spin"])).is_err());
        assert!(parse_args(&s(&["a.wav", "--level", "0"])).is_err());
        assert!(parse_args(&s(&["a.wav", "b.wav"])).is_err());
    }
}
