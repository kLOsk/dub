//! `dub loop-check` and `dub echo-check` — what a deck did, measured from
//! recordings of its output (the rig checklist's `loop` items).
//!
//! The recordings come from `dub loopback --record`: a take played into a
//! deck through the interface, the deck's output recorded back. Times on the
//! command line are seconds of the recording; `dub loopback` prints when its
//! recording starts so a script can log presses on the same clock.
//!
//! Both checks compare the deck with itself rather than with a threshold,
//! because a deck plays music and music has edges of its own:
//!
//! - **A loop** replays audio the deck just played straight. Its period is
//!   where the recording repeats itself; a click at a seam is a jump sharper
//!   than the same drum hit had when it played straight.
//! - **Echo Out** is judged against a second, untouched recording of the
//!   same take from the same start. Where the two agree the deck played dry;
//!   where they part, Echo Out acted.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

/// Two channels of a recording, and their average.
#[derive(Debug, Clone)]
pub struct Recording {
    left: Vec<f32>,
    right: Vec<f32>,
    mono: Vec<f32>,
    rate: f64,
}

impl Recording {
    /// The first two channels of a WAV, float or integer.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_pair(path, 1, 2)
    }

    /// `path`, or `path:L,R` for that pair of channels (1-based) — a
    /// `dub loopback --input-channels 1,2,3,4` recording holds the deck's
    /// output and what the deck received side by side.
    pub fn open_spec(spec: &str) -> Result<Self> {
        match spec.rsplit_once(':') {
            Some((path, pair)) if pair.contains(',') => {
                let (l, r) = pair
                    .split_once(',')
                    .ok_or_else(|| anyhow!("{spec}: want path:L,R"))?;
                Self::open_pair(Path::new(path), l.trim().parse()?, r.trim().parse()?)
            }
            _ => Self::open(Path::new(spec)),
        }
    }

    /// Channels `l` and `r` (1-based) of a WAV, float or integer.
    pub fn open_pair(path: &Path, l: usize, r: usize) -> Result<Self> {
        let mut reader =
            hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
        let spec = reader.spec();
        let channels = usize::from(spec.channels);
        if l == 0 || r == 0 || l > channels || r > channels {
            bail!(
                "{} has {channels} channel(s); asked for {l},{r}",
                path.display()
            );
        }
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
            hound::SampleFormat::Int => {
                let full = (1_i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / full))
                    .collect::<Result<_, _>>()?
            }
        };
        let frames = samples.len() / channels;
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for f in 0..frames {
            left.push(samples[f * channels + l - 1]);
            right.push(samples[f * channels + r - 1]);
        }
        Ok(Self::from_stereo(left, right, f64::from(spec.sample_rate)))
    }

    fn from_stereo(left: Vec<f32>, right: Vec<f32>, rate: f64) -> Self {
        let mono = left
            .iter()
            .zip(&right)
            .map(|(l, r)| 0.5 * (l + r))
            .collect();
        Self {
            left,
            right,
            mono,
            rate,
        }
    }

    fn frames(&self) -> usize {
        self.mono.len()
    }

    fn at(&self, t: f64) -> isize {
        (t * self.rate).round() as isize
    }

    fn secs(&self, frames: isize) -> f64 {
        frames as f64 / self.rate
    }
}

/// Normalised correlation of `n` frames of `a` from `ia` with `b` from `ib`.
/// `None` when a window runs off either end or holds no signal.
fn corr(a: &[f32], ia: isize, b: &[f32], ib: isize, n: usize) -> Option<f64> {
    if ia < 0 || ib < 0 || ia as usize + n > a.len() || ib as usize + n > b.len() {
        return None;
    }
    let (a, b) = (
        &a[ia as usize..ia as usize + n],
        &b[ib as usize..ib as usize + n],
    );
    let (mut dot, mut ea, mut eb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        ea += x * x;
        eb += y * y;
    }
    if ea < 1e-7 || eb < 1e-7 {
        return None;
    }
    Some(f64::from(dot) / f64::from(ea * eb).sqrt())
}

/// The lag in `[lo, hi]` frames where `b` best matches `a`, searched coarse
/// then fine.
fn best_lag(
    a: &[f32],
    ia: isize,
    b: &[f32],
    ib: isize,
    n: usize,
    lo: isize,
    hi: isize,
) -> Option<(isize, f64)> {
    let consider = |lag: isize, best: &mut Option<(isize, f64)>| {
        if let Some(c) = corr(a, ia, b, ib + lag, n) {
            if best.is_none_or(|(_, bc)| c > bc) {
                *best = Some((lag, c));
            }
        }
    };
    let mut best = None;
    let mut lag = lo;
    while lag <= hi {
        consider(lag, &mut best);
        lag += 4;
    }
    let (coarse, _) = best?;
    for lag in (coarse - 4).max(lo)..=(coarse + 4).min(hi) {
        consider(lag, &mut best);
    }
    best
}

fn rms(x: &[f32], a: isize, b: isize) -> f64 {
    let (a, b) = (a.max(0) as usize, (b.max(0) as usize).min(x.len()));
    if b <= a {
        return 0.0;
    }
    let sum: f64 = x[a..b].iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / (b - a) as f64).sqrt()
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-9).log10()
}

/// Largest sample-to-sample jump on either channel in `[a, b)`, and where.
fn sharpest(rec: &Recording, a: isize, b: isize) -> (f32, isize) {
    let (mut best, mut at) = (0.0_f32, a);
    let end = b.min(rec.frames() as isize);
    let mut n = a.max(1);
    while n < end {
        let i = n as usize;
        let d = (rec.left[i] - rec.left[i - 1])
            .abs()
            .max((rec.right[i] - rec.right[i - 1]).abs());
        if d > best {
            best = d;
            at = n;
        }
        n += 1;
    }
    (best, at)
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.retain(|x| !x.is_nan());
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

// ---------------------------------------------------------------- loops

/// What one loop did.
#[derive(Debug, Clone)]
pub struct LoopReport {
    pub period_secs: f64,
    pub beats: f64,
    /// Median and worst likeness of each 0.1 s against one period on.
    pub alike_median: f64,
    pub alike_worst: f64,
    /// Likeness at half the period: high means the loop is shorter than asked.
    pub half_alike: Option<f64>,
    pub first_seam: Option<f64>,
    pub seams: Vec<f32>,
    /// The loop's start as it played straight, one period before the first seam.
    pub straight_at_start: Option<f32>,
    /// The junction where the loop gave the track back, and the sharpest
    /// jumps in the half second either side of it.
    pub exit: Option<(f64, f32, f32, f32)>,
}

impl LoopReport {
    /// The worst seam against the same hit played straight.
    pub fn seam_ratio(&self) -> Option<f64> {
        let worst = self.seams.iter().copied().fold(0.0_f32, f32::max);
        self.straight_at_start
            .filter(|s| *s > 0.0)
            .map(|s| f64::from(worst / s))
    }
}

/// Is the audio at frame `i` a replay of what played `period` frames before?
fn replays(rec: &Recording, i: isize, period: isize) -> Option<f64> {
    let n = rec.at(0.02) as usize;
    let slack = rec.at(0.001);
    best_lag(&rec.mono, i, &rec.mono, i - period, n, -slack, slack).map(|(_, c)| c)
}

/// Measure the loop that was on from `t_in` to `t_out` (seconds of the
/// recording), asked for `beats` beats at `bpm`.
pub fn measure_loop(
    rec: &Recording,
    bpm: f64,
    t_in: f64,
    t_out: f64,
    beats: f64,
) -> Option<LoopReport> {
    let expected = beats * 60.0 / bpm;
    let w0 = rec.at(t_in + expected + 0.3);
    let lag_hi = rec.at(expected * 1.1);
    let width = rec.at(2.0).min(rec.at(t_out - 0.15) - w0 - lag_hi);
    if width < rec.at(0.2) {
        return None;
    }
    let width = width as usize;
    let (lag, peak) = best_lag(
        &rec.mono,
        w0,
        &rec.mono,
        w0,
        width,
        rec.at(expected * 0.9),
        lag_hi,
    )?;
    // Sub-frame peak through the neighbours.
    let cm = corr(&rec.mono, w0, &rec.mono, w0 + lag - 1, width).unwrap_or(peak);
    let cp = corr(&rec.mono, w0, &rec.mono, w0 + lag + 1, width).unwrap_or(peak);
    let denom = cm - 2.0 * peak + cp;
    let frac = if denom.abs() > 1e-12 {
        0.5 * (cm - cp) / denom
    } else {
        0.0
    };
    let period_secs = (lag as f64 + frac) / rec.rate;
    let half_alike = (beats >= 2.0)
        .then(|| {
            corr(
                &rec.mono,
                w0,
                &rec.mono,
                w0 + rec.at(period_secs / 2.0),
                width,
            )
        })
        .flatten();

    // Likeness 0.1 s at a time, each allowed ±3 ms, so a slow speed wobble
    // between cycles does not count against an exact replay.
    let short = rec.at(0.1) as usize;
    let slack = rec.at(0.003);
    let mut local = Vec::new();
    let mut a = w0;
    while a + lag + slack + (short as isize) < rec.at(t_out - 0.15) {
        if let Some((_, c)) = best_lag(&rec.mono, a, &rec.mono, a, short, lag - slack, lag + slack)
        {
            local.push(c);
        }
        a += rec.at(0.05);
    }
    let alike_median = median(local.clone())?;
    let alike_worst = local.iter().copied().fold(f64::INFINITY, f64::min);

    // The first seam: from the press, the first moment the audio becomes and
    // stays a replay of one period before.
    let mut first_seam = None;
    let mut i = rec.at(t_in - 0.05);
    let ms = rec.at(0.001);
    while i < rec.at(t_in + period_secs + 0.6) {
        let ok = |j: isize| replays(rec, j, lag).is_some_and(|c| c > 0.9);
        if ok(i) && ok(i + 10 * ms) && ok(i + 30 * ms) {
            first_seam = Some(i);
            break;
        }
        i += ms;
    }
    let three = rec.at(0.003);
    let (mut seams, mut straight_at_start) = (Vec::new(), None);
    if let Some(s0) = first_seam {
        straight_at_start = Some(sharpest(rec, s0 - lag - three, s0 - lag + three).0);
        let mut k = 0;
        while s0 + k * lag < rec.at(t_out - 0.05) {
            let at = s0 + k * lag;
            seams.push(sharpest(rec, at - three, at + three).0);
            k += 1;
        }
    }

    // The exit: the first moment after the release that stays unlike one
    // period before.
    let mut exit = None;
    let mut i = rec.at(t_out - 0.3);
    while i < rec.at(t_out + 0.8) {
        let off = |j: isize| replays(rec, j, lag).is_some_and(|c| c < 0.5);
        if off(i) && off(i + 10 * ms) && off(i + 30 * ms) {
            let junction = sharpest(rec, i - 20 * ms, i + 5 * ms).0;
            let before = sharpest(rec, i - rec.at(0.5), i - 30 * ms).0;
            let after = sharpest(rec, i + 30 * ms, i + rec.at(0.5)).0;
            exit = Some((rec.secs(i), junction, before, after));
            break;
        }
        i += ms;
    }

    Some(LoopReport {
        period_secs,
        beats: period_secs * bpm / 60.0,
        alike_median,
        alike_worst,
        half_alike,
        first_seam: first_seam.map(|s| rec.secs(s)),
        seams,
        straight_at_start,
        exit,
    })
}

/// `dub loop-check <wav> --bpm B IN,OUT,BEATS...`
pub fn run_loop_check(args: &[String]) -> Result<()> {
    const USAGE: &str = "usage: dub loop-check <wav> --bpm B IN,OUT,BEATS [IN,OUT,BEATS ...]\n\
        IN and OUT are the seconds of the recording the loop was pressed on and off";
    let mut wav = None;
    let mut bpm = None;
    let mut loops = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bpm" => {
                bpm = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!(USAGE))?
                        .parse::<f64>()?,
                );
                i += 1;
            }
            a if a.contains(',') => {
                let v: Vec<f64> = a.split(',').map(str::parse).collect::<Result<_, _>>()?;
                let [t_in, t_out, beats] = v[..] else {
                    bail!("{a}: want IN,OUT,BEATS\n{USAGE}")
                };
                loops.push((t_in, t_out, beats));
            }
            a => wav = Some(a.to_string()),
        }
        i += 1;
    }
    let (Some(wav), Some(bpm)) = (wav, bpm) else {
        bail!(USAGE)
    };
    let rec = Recording::open(Path::new(&wav))?;
    println!("{wav}  {bpm} BPM  {:.1} s", rec.secs(rec.frames() as isize));
    for (t_in, t_out, beats) in loops {
        let Some(r) = measure_loop(&rec, bpm, t_in, t_out, beats) else {
            println!("{beats} beats: the loop is too short to measure");
            continue;
        };
        println!(
            "{beats} beats: period {:.4} s = {:.3} beats; cycles alike {:.3} median, {:.3} worst (0.1 s windows); half the length alike {}",
            r.period_secs,
            r.beats,
            r.alike_median,
            r.alike_worst,
            r.half_alike.map_or("—".into(), |h| format!("{h:.3}"))
        );
        match (r.first_seam, r.seam_ratio()) {
            (Some(s0), Some(ratio)) => println!(
                "   first seam {s0:.3} s ({:.0} ms after the press); sharpest jump at {} seams {:.3} worst, the loop's start played straight {:.3}: {ratio:.2}×",
                (s0 - t_in) * 1000.0,
                r.seams.len(),
                r.seams.iter().copied().fold(0.0_f32, f32::max),
                r.straight_at_start.unwrap_or(0.0)
            ),
            _ => println!("   seams not found"),
        }
        if let Some((at, junction, before, after)) = r.exit {
            println!(
                "   exit {at:.3} s ({:.0} ms after the press): jump at the junction {junction:.3}; the half second before {before:.3}, after {after:.3}",
                (at - t_out) * 1000.0
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- echo out

/// One ECHO OUT press as a script logged it: a name and seconds of the
/// recording. Names ending `-on` / starting `on-` engage, the rest release;
/// `inside-on` / `inside-off` are the press-again-inside-the-beat pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Press {
    pub name: String,
    pub at: f64,
}

/// Lines of `name seconds`; anything else (a `SKIPPED` press) is left out.
pub fn parse_presses(text: &str) -> Vec<Press> {
    text.lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            let (name, at) = (p.next()?, p.next()?.parse().ok()?);
            Some(Press {
                name: name.to_string(),
                at,
            })
        })
        .collect()
}

/// What one echo did.
#[derive(Debug, Clone)]
pub struct EchoCycle {
    pub name: String,
    pub beat_secs: f64,
    /// From the press to the dry muting: one beat plays first.
    pub mute_after: f64,
    pub delay: f64,
    /// The beat that played, then the first three repeats, dB.
    pub levels: [f64; 4],
    pub back_after: Option<f64>,
    /// Where the returned tune sits against the untouched pass, ms.
    pub slip_ms: Option<f64>,
    /// Sharpest jump at the mute against the onset of the beat it repeats.
    pub mute_jump: f64,
    /// Sharpest jump at the return against the untouched pass there.
    pub back_jump: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct EchoReport {
    pub offset_ms: f64,
    pub aligned: f64,
    /// Press again inside the beat: worst and median likeness over 2 s, and
    /// how many 40 ms windows fell under 0.8.
    pub inside: Option<(f64, f64, usize, usize)>,
    pub cycles: Vec<EchoCycle>,
    pub after_last: Option<f64>,
}

/// Compare the pass with presses against the untouched one. `beat_at(t)` is
/// the deck's beat length at second `t` of the recording.
pub fn measure_echo(
    untouched: &Recording,
    echo: &Recording,
    presses: &[Press],
    beat_at: &dyn Fn(f64) -> f64,
) -> Result<EchoReport> {
    let first = presses.first().ok_or_else(|| anyhow!("no presses"))?.at;
    let a0 = echo.at(first - 3.0).max(0);
    let (offset, aligned) = best_lag(
        &echo.mono,
        a0,
        &untouched.mono,
        a0,
        echo.at(2.0) as usize,
        -echo.at(0.3),
        echo.at(0.3),
    )
    .ok_or_else(|| anyhow!("no audio to line the two passes up on before the first press"))?;

    // Likeness of 40 ms windows every 10 ms, each allowed ±3 ms: the passes
    // drift a millisecond or two apart over minutes.
    let (step, w, slack) = (echo.at(0.01), echo.at(0.04) as usize, echo.at(0.003));
    let like: Vec<f64> = (0..echo.frames() as isize / step)
        .map(|k| {
            let a = k * step;
            best_lag(&echo.mono, a, &untouched.mono, a + offset, w, -slack, slack)
                .map_or(f64::NAN, |(_, c)| c)
        })
        .collect();
    let like_at = |t: f64| like.get((t / 0.01) as usize).copied().unwrap_or(f64::NAN);
    let first_where = |from: f64, to: f64, hold: f64, cond: &dyn Fn(f64) -> bool| -> Option<f64> {
        let need = (hold / 0.01) as usize;
        let (mut run, mut start) = (0, 0);
        let (lo, hi) = (
            (from / 0.01) as usize,
            ((to / 0.01) as usize).min(like.len()),
        );
        for (k, &v) in like.iter().enumerate().take(hi).skip(lo) {
            if v.is_nan() {
                continue;
            }
            if cond(v) {
                if run == 0 {
                    start = k;
                }
                run += 1;
                if run >= need {
                    return Some(start as f64 * 0.01);
                }
            } else {
                run = 0;
            }
        }
        None
    };

    let inside = presses.iter().find(|p| p.name == "inside-on").map(|p| {
        let vals: Vec<f64> = (0..200)
            .map(|k| like_at(p.at + k as f64 * 0.01))
            .filter(|v| !v.is_nan())
            .collect();
        let worst = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let under = vals.iter().filter(|v| **v < 0.8).count();
        (
            worst,
            median(vals.clone()).unwrap_or(f64::NAN),
            under,
            vals.len(),
        )
    });

    let mut cycles = Vec::new();
    let mut on: Option<&Press> = None;
    for p in presses.iter().filter(|p| !p.name.starts_with("inside")) {
        let engages = p.name.ends_with("-on") || p.name.starts_with("on-");
        if engages {
            on = Some(p);
            continue;
        }
        let Some(press) = on.take() else { continue };
        let beat = beat_at(press.at);
        let Some(mute) = first_where(press.at + 0.02, p.at, 0.03, &|v| v < 0.5) else {
            continue;
        };
        let span = (p.at - mute - 1.0).clamp(0.2, 0.5);
        let delay = best_lag(
            &echo.mono,
            echo.at(mute),
            &echo.mono,
            echo.at(mute),
            echo.at(span) as usize,
            echo.at(0.25),
            echo.at(1.0),
        )
        .map_or(f64::NAN, |(l, _)| echo.secs(l));
        let levels = [mute - beat, mute, mute + beat, mute + 2.0 * beat]
            .map(|t| db(rms(&echo.mono, echo.at(t), echo.at(t + beat))));
        let back = first_where(p.at, p.at + 1.5, 0.1, &|v| v >= 0.85);
        let slip_ms = back.and_then(|b| {
            best_lag(
                &echo.mono,
                echo.at(b + 0.2),
                &untouched.mono,
                echo.at(b + 0.2) + offset,
                echo.at(0.2) as usize,
                -echo.at(0.05),
                echo.at(0.05),
            )
            .map(|(l, _)| echo.secs(l) * 1000.0)
        });
        let one = if delay > 1.5 * beat {
            delay / 2.0
        } else {
            delay
        };
        let ten = echo.at(0.01);
        let mute_jump = f64::from(sharpest(echo, echo.at(mute) - ten, echo.at(mute) + ten).0)
            / f64::from(
                sharpest(
                    untouched,
                    echo.at(mute - one) - ten + offset,
                    echo.at(mute - one) + ten + offset,
                )
                .0
                .max(1e-6),
            );
        let back_jump = back.map(|b| {
            f64::from(sharpest(echo, echo.at(p.at - 0.02), echo.at(b + 0.05)).0)
                / f64::from(
                    sharpest(
                        untouched,
                        echo.at(p.at - 0.02) + offset,
                        echo.at(b + 0.05) + offset,
                    )
                    .0
                    .max(1e-6),
                )
        });
        cycles.push(EchoCycle {
            name: press.name.clone(),
            beat_secs: beat,
            mute_after: mute - press.at,
            delay,
            levels,
            back_after: back.map(|b| b - p.at),
            slip_ms,
            mute_jump,
            back_jump,
        });
    }
    let after_last = presses
        .last()
        .map(|p| {
            (0..)
                .map(|k| p.at + 1.0 + f64::from(k) * 0.01)
                .take_while(|t| *t < echo.secs(echo.frames() as isize) - 0.5)
                .map(like_at)
                .collect::<Vec<_>>()
        })
        .and_then(median);
    Ok(EchoReport {
        offset_ms: echo.secs(offset) * 1000.0,
        aligned,
        inside,
        cycles,
        after_last,
    })
}

/// The deck's beat length over a take: `t rate` lines from
/// `dub decode-timecode --head`, read from `take_start` seconds into the
/// recording. Without them the beat is the track's.
fn beat_lengths(bpm: f64, rates: Option<(String, f64)>) -> Result<Box<dyn Fn(f64) -> f64>> {
    let Some((text, take_start)) = rates else {
        return Ok(Box::new(move |_| 60.0 / bpm));
    };
    let table: Vec<(f64, f64)> = text
        .lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
        })
        .collect();
    if table.is_empty() {
        bail!("no `time rate` lines in the rates file");
    }
    Ok(Box::new(move |t| {
        let take = t - take_start;
        let i = table
            .partition_point(|(tt, _)| *tt <= take)
            .saturating_sub(1);
        60.0 / (bpm * table[i].1)
    }))
}

/// `dub echo-check <untouched.wav> <echo.wav> --presses FILE --bpm B [--rates FILE --take-start S]`
pub fn run_echo_check(args: &[String]) -> Result<()> {
    const USAGE: &str =
        "usage: dub echo-check <untouched.wav[:L,R]> <echo.wav[:L,R]> --presses FILE \
        --bpm B [--rates FILE --take-start SECS]\n\
        On a THRU deck the untouched pass is what the deck received: rec.wav:3,4 rec.wav:1,2";
    let (mut wavs, mut presses, mut bpm, mut rates, mut take_start) =
        (Vec::new(), None, None, None, 0.0);
    let mut i = 0;
    while i < args.len() {
        let value = || args.get(i + 1).cloned().ok_or_else(|| anyhow!(USAGE));
        match args[i].as_str() {
            "--presses" => presses = Some(value()?),
            "--bpm" => bpm = Some(value()?.parse::<f64>()?),
            "--rates" => rates = Some(value()?),
            "--take-start" => take_start = value()?.parse()?,
            a => {
                wavs.push(a.to_string());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let ([untouched, echo], Some(presses), Some(bpm)) = (&wavs[..], presses, bpm) else {
        bail!(USAGE)
    };
    let untouched = Recording::open_spec(untouched)?;
    let echo = Recording::open_spec(echo)?;
    let presses = parse_presses(
        &std::fs::read_to_string(&presses).with_context(|| format!("reading {presses}"))?,
    );
    let rates = rates
        .map(|p| std::fs::read_to_string(&p).map(|t| (t, take_start)))
        .transpose()?;
    let beat_at = beat_lengths(bpm, rates)?;
    let r = measure_echo(&untouched, &echo, &presses, &*beat_at)?;

    println!("lined up: the echo pass {:+.2} ms against the untouched one, alike {:.3} before the first press", r.offset_ms, r.aligned);
    if let Some((worst, med, under, n)) = r.inside {
        println!("press again inside the beat: the next 2 s alike {med:.3} median, {worst:.3} worst; {under} of {n} windows under 0.8");
    }
    println!("press       beat   mute after      delay   beat → repeats (dB)        back after  slip     jump mute / back");
    for c in &r.cycles {
        println!(
            "{:<10} {:.3}  {:.3} ({:.1} b)  {:.2} b   {:5.1} {:5.1} {:5.1} {:5.1}   {}  {}  {:.2}× / {}",
            c.name,
            c.beat_secs,
            c.mute_after,
            c.mute_after / c.beat_secs,
            c.delay / c.beat_secs,
            c.levels[0],
            c.levels[1],
            c.levels[2],
            c.levels[3],
            c.back_after.map_or("   never".into(), |b| format!("{b:.3} s")),
            c.slip_ms.map_or("  —   ".into(), |s| format!("{s:+.1} ms")),
            c.mute_jump,
            c.back_jump.map_or("—".into(), |b| format!("{b:.2}×")),
        );
    }
    let col =
        |f: &dyn Fn(&EchoCycle) -> f64, part: &[EchoCycle]| median(part.iter().map(f).collect());
    let summary = |label: &str, part: &[EchoCycle]| {
        let rep_steps: Vec<f64> = part
            .iter()
            .flat_map(|c| [c.levels[2] - c.levels[1], c.levels[3] - c.levels[2]])
            .collect();
        println!(
            "{label}: mute after {:.2} beats; first repeat {:+.1} dB under the beat; later repeats {:+.1} dB each (worst {:+.1}); back after {:.3} s; slip {:+.1} ms",
            col(&|c| c.mute_after / c.beat_secs, part).unwrap_or(f64::NAN),
            col(&|c| c.levels[1] - c.levels[0], part).unwrap_or(f64::NAN),
            median(rep_steps.clone()).unwrap_or(f64::NAN),
            rep_steps.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            col(&|c| c.back_after.unwrap_or(f64::NAN), part).unwrap_or(f64::NAN),
            col(&|c| c.slip_ms.unwrap_or(f64::NAN), part).unwrap_or(f64::NAN),
        );
    };
    if r.cycles.len() >= 20 {
        summary("first 10", &r.cycles[..10]);
        summary("last 10 ", &r.cycles[r.cycles.len() - 10..]);
    }
    summary(&format!("all {}  ", r.cycles.len()), &r.cycles);
    if let Some(a) = r.after_last {
        println!("after the last release the passes match at {a:.3}");
    }
    Ok(())
}

// ---------------------------------------------------------------- thru

/// The deck's output against what it received, one window.
#[derive(Debug, Clone)]
pub struct ThruWindow {
    pub at: f64,
    pub delay_ms: f64,
    pub alike: f64,
    pub gain_db: f64,
}

/// Every `every` seconds, where in the deck's output (`out`) the next second
/// of its input (`input`) turns up, how alike, and how much louder. Silent
/// windows are skipped.
pub fn measure_thru(input: &Recording, out: &Recording, every: f64) -> Vec<ThruWindow> {
    let n = input.at(1.0) as usize;
    let mut windows = Vec::new();
    let mut t = 0.5;
    while input.at(t + 1.2) < input.frames() as isize {
        let i = input.at(t);
        if rms(&input.mono, i, i + n as isize) > 1e-3 {
            if let Some((lag, alike)) = best_lag(&input.mono, i, &out.mono, i, n, 0, input.at(0.1))
            {
                let gain = db(rms(&out.mono, i + lag, i + lag + n as isize)
                    / rms(&input.mono, i, i + n as isize));
                windows.push(ThruWindow {
                    at: t,
                    delay_ms: input.secs(lag) * 1000.0,
                    alike,
                    gain_db: gain,
                });
            }
        }
        t += every;
    }
    windows
}

/// Start times of 10 ms stretches where the input carries signal and the
/// output, at the measured delay, falls more than `drop_db` under it: a
/// dropout, or Echo Out muting the deck.
pub fn find_gaps(
    input: &Recording,
    out: &Recording,
    delay_frames: isize,
    drop_db: f64,
) -> Vec<f64> {
    let w = input.at(0.01);
    let mut gaps = Vec::new();
    let mut i = 0;
    while i + w + delay_frames < input.frames() as isize {
        let a = rms(&input.mono, i, i + w);
        if a > 3e-3 {
            let b = rms(&out.mono, i + delay_frames, i + delay_frames + w);
            if db(b) < db(a) - drop_db {
                gaps.push(input.secs(i));
            }
        }
        i += w;
    }
    gaps
}

/// `dub thru-check <rec.wav> [--in L,R] [--out L,R] [--every SECS]`
pub fn run_thru_check(args: &[String]) -> Result<()> {
    const USAGE: &str = "usage: dub thru-check <rec.wav> [--in 3,4] [--out 1,2] [--every SECS]\n\
        rec.wav from `dub loopback --input-channels 1,2,3,4`: the deck's output on 1,2, what it received on 3,4";
    let (mut wav, mut inp, mut outp, mut every) =
        (None, "3,4".to_string(), "1,2".to_string(), 10.0);
    let mut i = 0;
    while i < args.len() {
        let value = || args.get(i + 1).cloned().ok_or_else(|| anyhow!(USAGE));
        match args[i].as_str() {
            "--in" => inp = value()?,
            "--out" => outp = value()?,
            "--every" => every = value()?.parse()?,
            a => {
                wav = Some(a.to_string());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let wav = wav.ok_or_else(|| anyhow!(USAGE))?;
    let input = Recording::open_spec(&format!("{wav}:{inp}"))?;
    let out = Recording::open_spec(&format!("{wav}:{outp}"))?;
    let windows = measure_thru(&input, &out, every);
    if windows.is_empty() {
        bail!("no signal on the input channels {inp}");
    }
    let delay = median(windows.iter().map(|w| w.delay_ms).collect()).unwrap_or(f64::NAN);
    let (lo, hi) = windows
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), w| {
            (a.min(w.delay_ms), b.max(w.delay_ms))
        });
    println!(
        "{} windows: delay through the deck {delay:.2} ms (from {lo:.2} to {hi:.2}); alike {:.4} median, {:.4} worst; gain {:+.2} dB median",
        windows.len(),
        median(windows.iter().map(|w| w.alike).collect()).unwrap_or(f64::NAN),
        windows.iter().map(|w| w.alike).fold(f64::INFINITY, f64::min),
        median(windows.iter().map(|w| w.gain_db).collect()).unwrap_or(f64::NAN),
    );
    for w in windows.iter().filter(|w| w.alike < 0.99) {
        println!(
            "   at {:.1} s: alike {:.3}, delay {:.2} ms, gain {:+.1} dB",
            w.at, w.alike, w.delay_ms, w.gain_db
        );
    }
    let gaps = find_gaps(&input, &out, input.at(delay / 1000.0), 20.0);
    let mut runs: Vec<(f64, f64)> = Vec::new();
    for g in gaps {
        match runs.last_mut() {
            Some((_, end)) if g - *end < 0.015 => *end = g + 0.01,
            _ => runs.push((g, g + 0.01)),
        }
    }
    println!(
        "{} stretch(es) where the output fell 20 dB under the input:",
        runs.len()
    );
    for (a, b) in runs.iter().take(30) {
        println!("   {a:.2}–{b:.2} s ({:.0} ms)", (b - a) * 1000.0);
    }
    Ok(())
}

// ---------------------------------------------------------------- dropouts

/// Start times of `window`-second stretches quieter than `floor_db` with
/// both neighbours (within 50 ms) louder than `around_db`: a hole in
/// playing music. Music fades and breaks; it does not drop 25 dB for a few
/// milliseconds and come straight back.
pub fn find_dropouts(rec: &Recording, window: f64, floor_db: f64, around_db: f64) -> Vec<f64> {
    let w = rec.at(window).max(1);
    let reach = rec.at(0.05);
    let level = |a: isize| db(rms(&rec.mono, a, a + w));
    let mut holes = Vec::new();
    let mut i = reach;
    while i + w + reach < rec.frames() as isize {
        let hole =
            level(i) < floor_db && level(i - reach) > around_db && level(i + reach) > around_db;
        if hole
            && holes
                .last()
                .is_none_or(|&last: &f64| rec.secs(i) - last > 0.05)
        {
            holes.push(rec.secs(i));
        }
        i += w;
    }
    holes
}

/// `dub dropout-check <wav[:L,R]> [--window-ms 5] [--floor-db -60] [--around-db -35]`
pub fn run_dropout_check(args: &[String]) -> Result<()> {
    const USAGE: &str =
        "usage: dub dropout-check <wav[:L,R]> [--window-ms 5] [--floor-db -60] [--around-db -35]";
    let (mut wav, mut window_ms, mut floor, mut around) = (None, 5.0, -60.0, -35.0);
    let mut i = 0;
    while i < args.len() {
        let value = || args.get(i + 1).cloned().ok_or_else(|| anyhow!(USAGE));
        match args[i].as_str() {
            "--window-ms" => window_ms = value()?.parse()?,
            "--floor-db" => floor = value()?.parse()?,
            "--around-db" => around = value()?.parse()?,
            a => {
                wav = Some(a.to_string());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let wav = wav.ok_or_else(|| anyhow!(USAGE))?;
    let rec = Recording::open_spec(&wav)?;
    let holes = find_dropouts(&rec, window_ms / 1000.0, floor, around);
    println!(
        "{wav}: {:.1} s, {} dropout(s) — {window_ms} ms under {floor} dB between neighbours over {around} dB",
        rec.secs(rec.frames() as isize),
        holes.len()
    );
    for t in holes.iter().take(40) {
        println!("   {t:.3} s");
    }
    Ok(())
}

/// What a THRU deck's tempo readout showed at one moment of a replay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoLook {
    /// Seconds of the recording.
    pub secs: f64,
    pub state: dub_bpm::TrackerState,
    /// Bumps each time sound returns after the needle was up.
    pub record_seq: u32,
    pub needle_up: bool,
}

/// Replays `mono` through the live tempo tracker exactly as a THRU deck
/// runs it (`DubEngine::start_live_analysis`: one channel, the default
/// range, a fresh analysis every second, [`dub_bpm::LiveStreamConfig`]'s
/// defaults), and looks at it every `every` seconds of audio.
///
/// The real [`dub_bpm::BpmStream`] does the work on its own thread; the
/// replay hands it `every` seconds at a time and waits for it to take
/// them, so a look sees exactly the audio before it, however fast the
/// replay runs.
pub fn replay_thru_tempo(mono: &[f32], rate: u32, every: f64) -> Result<Vec<TempoLook>> {
    use ringbuf::traits::{Producer, Split};
    let step = ((every * f64::from(rate)).round() as usize).max(1);
    let (mut tx, rx) = ringbuf::HeapRb::<f32>::new(step * 2).split();
    let stream = dub_bpm::BpmStream::spawn_live(
        rx,
        dub_bpm::TrackerConfig {
            sample_rate: rate,
            channels: 1,
            analysis_period_samples: rate.max(1),
            bpm_range: dub_bpm::BpmRange::DEFAULT,
        },
        dub_bpm::LiveStreamConfig::default(),
    )
    .map_err(|e| anyhow!("tracker: {e:?}"))?;
    let mut looks = Vec::new();
    let mut fed = 0usize;
    for chunk in mono.chunks(step) {
        let mut rest = chunk;
        while !rest.is_empty() {
            let n = tx.push_slice(rest);
            rest = &rest[n..];
            if !rest.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        fed += chunk.len();
        let started = std::time::Instant::now();
        let snap = loop {
            let snap = stream.snapshot();
            if snap.consumed_frames >= fed as u64 {
                break snap;
            }
            if started.elapsed() > std::time::Duration::from_secs(10) {
                bail!(
                    "the tracker stopped taking audio at {:.1} s",
                    fed as f64 / f64::from(rate)
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        looks.push(TempoLook {
            secs: fed as f64 / f64::from(rate),
            state: snap.state,
            record_seq: snap.record_seq,
            needle_up: snap.needle_up,
        });
    }
    Ok(looks)
}

/// The estimator's own reading after each analysis (once a second of
/// audio), and the tracker state it led to: why a deck shows what it does.
/// Synchronous, without the needle-up watch — start it inside the music.
pub fn raw_thru_estimates(
    mono: &[f32],
    rate: u32,
) -> Result<Vec<(f64, dub_bpm::BpmEstimate, dub_bpm::TrackerState)>> {
    let mut tracker = dub_bpm::BpmTracker::new(dub_bpm::TrackerConfig {
        sample_rate: rate,
        channels: 1,
        analysis_period_samples: rate.max(1),
        bpm_range: dub_bpm::BpmRange::DEFAULT,
    })
    .map_err(|e| anyhow!("tracker: {e:?}"))?;
    tracker.set_odf_window_secs(Some(dub_bpm::LiveStreamConfig::default().odf_window_secs));
    let mut out = Vec::new();
    let (mut seen, mut fed) = (0usize, 0usize);
    for block in mono.chunks(512) {
        let _ = tracker.process(block);
        fed += block.len();
        if fed / rate as usize > seen {
            seen = fed / rate as usize;
            if let Some(e) = tracker.last_estimate() {
                out.push((fed as f64 / f64::from(rate), e, tracker.state()));
            }
        }
    }
    Ok(out)
}

fn describe_tempo(state: dub_bpm::TrackerState) -> String {
    match state {
        dub_bpm::TrackerState::Searching => "LISTENING".to_string(),
        dub_bpm::TrackerState::Tentative { bpm } => format!("tentative {bpm:.1}"),
        dub_bpm::TrackerState::Locked { bpm } => format!("LOCKED {bpm:.1}"),
    }
}

/// `dub thru-tempo <wav[:L,R]> [--from S] [--secs S] [--every S]` — what
/// a THRU deck's tempo readout would have shown for a recording of what
/// it received: every change, then a line per `--every` seconds.
pub fn run_thru_tempo(args: &[String]) -> Result<()> {
    const USAGE: &str =
        "usage: dub thru-tempo <wav[:L,R]> [--from S] [--secs S] [--every 1] [--raw]";
    let (mut wav, mut from, mut secs, mut every) = (None, 0.0, f64::INFINITY, 1.0);
    let mut raw = false;
    let mut i = 0;
    while i < args.len() {
        let value = || args.get(i + 1).cloned().ok_or_else(|| anyhow!(USAGE));
        match args[i].as_str() {
            "--from" => from = value()?.parse()?,
            "--secs" => secs = value()?.parse()?,
            "--every" => every = value()?.parse()?,
            "--raw" => {
                raw = true;
                i += 1;
                continue;
            }
            a => {
                wav = Some(a.to_string());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let wav = wav.ok_or_else(|| anyhow!(USAGE))?;
    let rec = Recording::open_spec(&wav)?;
    let a = rec.at(from).clamp(0, rec.frames() as isize) as usize;
    let b = if secs.is_finite() {
        rec.at(from + secs).clamp(0, rec.frames() as isize) as usize
    } else {
        rec.frames()
    };
    if raw {
        for (t, e, state) in raw_thru_estimates(&rec.mono[a..b.max(a)], rec.rate as u32)? {
            println!(
                "{:7.2} s  estimate {:6.1} confidence {:.3}  → {}",
                from + t,
                e.bpm,
                e.confidence,
                describe_tempo(state)
            );
        }
        return Ok(());
    }
    let looks = replay_thru_tempo(&rec.mono[a..b.max(a)], rec.rate as u32, 0.25)?;
    let mut last: Option<String> = None;
    let mut next_line = every;
    for look in &looks {
        let shown = if look.needle_up {
            "needle up".to_string()
        } else {
            describe_tempo(look.state)
        };
        let changed = last.as_deref() != Some(shown.as_str());
        if changed || look.secs + 1e-9 >= next_line {
            println!(
                "{:7.2} s  {shown}{}  (record {})",
                from + look.secs,
                if changed { "  ←" } else { "" },
                look.record_seq
            );
            while next_line <= look.secs + 1e-9 {
                next_line += every;
            }
        }
        last = Some(shown);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// Deterministic noise in −0.5..0.5: music enough for correlation.
    fn noise(n: usize, mut seed: u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / 16_777_216.0 - 0.5
            })
            .collect()
    }

    fn stereo(m: Vec<f32>) -> Recording {
        Recording::from_stereo(m.clone(), m, RATE)
    }

    /// 3 s straight, a loop of the last `period` before 3 s for 4 s, then
    /// new audio.
    fn looped(period: f64, click: f32) -> Recording {
        let p = (period * RATE) as usize;
        let mut x = noise(3 * RATE as usize, 1);
        let lp: Vec<f32> = x[x.len() - p..].to_vec();
        for i in 0..(4.0 * RATE) as usize {
            let mut s = lp[i % p];
            if i % p == 0 {
                s += click;
            }
            x.push(s);
        }
        x.extend(noise(2 * RATE as usize, 2));
        stereo(x)
    }

    #[test]
    fn a_loop_is_its_length_and_its_seams_are_found() {
        let rec = looped(0.5, 0.0);
        let r = measure_loop(&rec, 120.0, 2.98, 7.0, 1.0).expect("measurable");
        assert!((r.period_secs - 0.5).abs() < 1e-4, "{r:?}");
        assert!((r.beats - 1.0).abs() < 1e-3);
        assert!(r.alike_median > 0.99);
        let s0 = r.first_seam.expect("a seam");
        assert!((s0 - 3.0).abs() < 0.002, "first seam at {s0}");
        assert_eq!(r.seams.len(), 8);
        let exit = r.exit.expect("an exit");
        assert!((exit.0 - 7.0).abs() < 0.01, "exit at {}", exit.0);
    }

    #[test]
    fn a_click_at_the_seam_shows() {
        let clean = measure_loop(&looped(0.5, 0.0), 120.0, 2.98, 7.0, 1.0).expect("clean");
        let clicked = measure_loop(&looped(0.5, 1.5), 120.0, 2.98, 7.0, 1.0).expect("clicked");
        assert!(clean.seam_ratio().expect("ratio") < 1.3, "{clean:?}");
        assert!(clicked.seam_ratio().expect("ratio") > 1.8, "{clicked:?}");
    }

    #[test]
    fn a_loop_of_two_is_not_a_loop_of_one() {
        let r = measure_loop(&looped(1.0, 0.0), 120.0, 2.98, 7.0, 2.0).expect("measurable");
        assert!((r.beats - 2.0).abs() < 1e-3);
        assert!(r.half_alike.expect("half") < 0.2);
    }

    #[test]
    fn presses_parse_and_skip_what_did_not_happen() {
        let p = parse_presses("on-0 12.5\noff-0 14.5\non-1 SKIPPED\n\ngarbage\n");
        assert_eq!(
            p,
            vec![
                Press {
                    name: "on-0".into(),
                    at: 12.5
                },
                Press {
                    name: "off-0".into(),
                    at: 14.5
                },
            ]
        );
    }

    /// Untouched noise, and the same with an echo: one beat plays dry from
    /// the press, then that beat repeats at 0.6× each time until the release,
    /// when the dry is back where it would have been.
    fn echo_pair(beat: f64, press: f64, release: f64) -> (Recording, Recording, Vec<Press>) {
        let n = (12.0 * RATE) as usize;
        let dry = noise(n, 7);
        let (b, p, r) = (
            (beat * RATE) as usize,
            (press * RATE) as usize,
            (release * RATE) as usize,
        );
        let mut out = dry.clone();
        for i in p + b..r {
            let k = (i - p) / b;
            out[i] = dry[p + (i - p) % b] * 0.6_f32.powi(k as i32);
        }
        let presses = vec![
            Press {
                name: "on-0".into(),
                at: press,
            },
            Press {
                name: "off-0".into(),
                at: release,
            },
        ];
        (stereo(dry), stereo(out), presses)
    }

    #[test]
    fn an_echo_plays_one_beat_then_repeats_it_quieter_and_gives_the_tune_back() {
        let (untouched, echo, presses) = echo_pair(0.5, 6.0, 8.6);
        let r = measure_echo(&untouched, &echo, &presses, &|_| 0.5).expect("measured");
        assert!(r.offset_ms.abs() < 0.1 && r.aligned > 0.99, "{r:?}");
        let c = &r.cycles[0];
        assert!(
            (c.mute_after - 0.5).abs() < 0.03,
            "mute after {}",
            c.mute_after
        );
        assert!(
            (c.delay - 0.5).abs() < 0.002 || (c.delay - 1.0).abs() < 0.002,
            "delay {}",
            c.delay
        );
        let step = c.levels[2] - c.levels[1];
        assert!(
            (step - 20.0 * 0.6_f64.log10()).abs() < 0.3,
            "repeat step {step} dB"
        );
        assert!(c.back_after.expect("back") < 0.06);
        assert!(c.slip_ms.expect("slip").abs() < 0.1);
        assert!(r.after_last.expect("after") > 0.99);
    }

    #[test]
    fn thru_finds_the_delay_and_a_dropout() {
        let input = noise((6.0 * RATE) as usize, 3);
        let delay = (0.004 * RATE) as usize;
        let mut out = vec![0.0; delay];
        out.extend(input.iter().map(|s| s * 0.5));
        out.truncate(input.len());
        let gap = (3.0 * RATE) as usize;
        for s in &mut out[gap + delay..gap + delay + (0.05 * RATE) as usize] {
            *s = 0.0;
        }
        let (input, out) = (stereo(input), stereo(out));
        let w = measure_thru(&input, &out, 1.0);
        assert!(!w.is_empty());
        assert!(w.iter().all(|w| (w.delay_ms - 4.0).abs() < 0.05), "{w:?}");
        assert!((w[0].gain_db + 6.02).abs() < 0.05);
        let gaps = find_gaps(&input, &out, delay as isize, 20.0);
        assert!(
            !gaps.is_empty() && gaps.iter().all(|g| (*g - 3.0).abs() < 0.06),
            "{gaps:?}"
        );
    }

    #[test]
    fn a_hole_in_music_is_found_and_a_fade_is_not() {
        let mut x = noise((4.0 * RATE) as usize, 9);
        let hole = (1.5 * RATE) as usize;
        for s in &mut x[hole..hole + (0.008 * RATE) as usize] {
            *s = 0.0;
        }
        // A one-second fade to silence and back: a break, not a dropout.
        let fade = (3.0 * RATE) as usize;
        for (k, s) in x[fade - RATE as usize..fade].iter_mut().enumerate() {
            *s *= 1.0 - k as f32 / RATE as f32;
        }
        let holes = find_dropouts(&stereo(x), 0.005, -60.0, -35.0);
        assert_eq!(holes.len(), 1, "{holes:?}");
        assert!((holes[0] - 1.5).abs() < 0.01);
    }

    #[test]
    fn a_beat_length_follows_the_take() {
        let f = beat_lengths(120.0, Some(("0.0 1.0\n10.0 1.1\n".into(), 2.0))).expect("table");
        assert!((f(3.0) - 0.5).abs() < 1e-9);
        assert!((f(13.0) - 0.5 / 1.1).abs() < 1e-9);
        assert!((beat_lengths(120.0, None).expect("flat")(99.0) - 0.5).abs() < 1e-9);
    }
}
