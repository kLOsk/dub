//! `dub stretch-bench` — offline measurement of the WSOLA key-lock engine.
//!
//! Listening is the final arbiter, but it isn't repeatable and it can't see
//! CPU. This subcommand is the *measurable* half: it runs a fixed corpus
//! through each backend across a sweep of playback rates and reports, per cell:
//! (M14.2–M14.4 also compared an opt-in Rubber Band backend here; WSOLA won by
//! ear and Rubber Band was dropped, so today the columns are resampler-only vs
//! our WSOLA.)
//!
//! - **CPU** — wall-clock ms per second of output, plus per-block p50/p99 and
//!   the xrun-risk (fraction of blocks over the audio-callback deadline) at one
//!   deck and simulated two decks. Offline + single-threaded, so `Instant`
//!   timing is legitimate (we are nowhere near the audio thread here).
//! - **latency** — the backend's algorithmic latency in frames.
//! - **quality proxies** — pitch deviation in cents (does key lock actually
//!   hold pitch?), a crest-factor ratio (transient preservation — the metric
//!   that matters for percussive material), and log-spectral distance to the
//!   source (timbre fidelity). Honest caveat: warble / phasiness / "does it
//!   still sound like a record" only the ear can judge — the live A/B switch
//!   (M14.3) is the real arbiter. These proxies gate regressions and rank CPU.
//!
//! Backends compared: **resampler-only** (no key lock — the naïve pitched
//! record, pitch shifts with rate) vs **DubOwn** (our WSOLA, pitch held).
//!
//! ## Rate → stretch mapping
//!
//! A key-lock deck at playback rate `R` plays the track at tempo `×R` with
//! pitch held. As a pure time-stretch that is a factor `S = 1 / R` (speed up →
//! compress). So the WSOLA backend runs at `time_ratio = 1/R`; the
//! resampler-only baseline simply reads `R` input frames per output frame
//! (tempo `×R` *and* pitch `×R`).
//!
//! Flags: `[--input <wav>]` runs a real file instead of the synthetic corpus;
//! `[--dump <dir>]` writes every output cell to a WAV for ear inspection;
//! `[--full]` runs the full rate sweep (default is a shorter representative
//! set).

use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use dub_stretch::{TimeStretcher, WsolaStretcher};
use realfft::RealFftPlanner;

/// Audio-callback block size used for per-block timing + the xrun deadline.
const BLOCK: usize = 512;

/// Playback rates a DJ might dial in (= pitch-fader excursions). `R > 1` =
/// faster/up, `R < 1` = slower/down. Reported as the rate; the WSOLA backend
/// runs at `1/R`.
const RATES_FULL: &[f64] = &[0.67, 0.84, 0.92, 0.94, 0.98, 1.02, 1.06, 1.08, 1.16, 1.33];
const RATES_SHORT: &[f64] = &[0.92, 0.94, 1.06, 1.08];

/// Which engine a bench row exercises.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    /// No key lock: a linear resampler (pitch shifts with rate). The baseline.
    ResamplerOnly,
    /// Our pure-Rust WSOLA, holding pitch.
    DubOwn,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Backend::ResamplerOnly => "resampler",
            Backend::DubOwn => "dub-wsola",
        }
    }
}

/// One corpus item: a deterministic signal plus its known fundamental (if it
/// has a single clear pitch, for the cents metric).
struct Corpus {
    name: &'static str,
    /// Interleaved stereo `f32`.
    signal: Vec<f32>,
    /// Hz, or `None` for inharmonic material (skip the pitch metric).
    fundamental: Option<f32>,
}

/// The deterministic quality metrics for one cell (excludes wall-clock so it
/// can be golden-snapshotted).
struct Quality {
    out_frames: usize,
    /// Pitch deviation from the *source* pitch, in cents. ≈ 0 means pitch held
    /// (good key lock); the resampler baseline lands near `1200·log2(R)`.
    pitch_cents: Option<f32>,
    /// Output crest factor ÷ source crest factor. 1.0 = transients fully
    /// preserved; < 1 = smearing.
    crest_ratio: f32,
    /// Log-spectral distance to the source's long-term spectrum (dB). Low =
    /// timbre preserved.
    lsd_db: f32,
}

/// Wall-clock results for one cell.
struct Timing {
    cpu_ms_per_s: f64,
    p50_us: f64,
    p99_us: f64,
    xrun_1: f64,
    xrun_2: f64,
}

fn sine(freq: f32, sr: f32, frames: usize, amp: f32) -> impl Iterator<Item = f32> {
    (0..frames).map(move |i| {
        #[allow(clippy::cast_precision_loss)]
        let t = i as f32 / sr;
        (std::f32::consts::TAU * freq * t).sin() * amp
    })
}

/// Build the synthetic corpus at `sr`, each item `secs` long.
fn synthetic_corpus(sr: f32, secs: f32) -> Vec<Corpus> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (sr * secs) as usize;
    let mut out = Vec::new();

    // Percussive: a 4-on-the-floor of sharp-attack 60 Hz "kicks" with a
    // single-sample click at each onset — transient-rich, the core audience
    // material.
    {
        let mut s = vec![0.0f32; frames * 2];
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step = (sr * 0.25) as usize; // 4 hits/sec
        let mut onset = 0;
        while onset < frames {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let decay = (sr * 0.18) as usize;
            for j in 0..decay.min(frames - onset) {
                #[allow(clippy::cast_precision_loss)]
                let t = j as f32 / sr;
                let env = (-t / 0.05).exp();
                let v = (std::f32::consts::TAU * 60.0 * t).sin() * env * 0.7;
                s[(onset + j) * 2] += v;
                s[(onset + j) * 2 + 1] += v;
            }
            s[onset * 2] = 1.0; // click
            s[onset * 2 + 1] = 1.0;
            onset += step;
        }
        out.push(Corpus {
            name: "percussive",
            signal: s,
            fundamental: Some(60.0),
        });
    }

    // Tonal: a sustained chord with a dominant 220 Hz root — WSOLA's weak spot
    // (polyphonic tonal warble).
    {
        let mut s = vec![0.0f32; frames * 2];
        for (f, a) in [(220.0, 0.5), (277.18, 0.22), (329.63, 0.22), (440.0, 0.18)] {
            for (i, v) in sine(f, sr, frames, a).enumerate() {
                s[i * 2] += v;
                s[i * 2 + 1] += v;
            }
        }
        out.push(Corpus {
            name: "tonal",
            signal: s,
            fundamental: Some(220.0),
        });
    }

    // Sub-bass: a clean 55 Hz sine — low-frequency periodicity / phase.
    {
        let mut s = vec![0.0f32; frames * 2];
        for (i, v) in sine(55.0, sr, frames, 0.7).enumerate() {
            s[i * 2] = v;
            s[i * 2 + 1] = v;
        }
        out.push(Corpus {
            name: "sub-bass",
            signal: s,
            fundamental: Some(55.0),
        });
    }

    // Broadband: a deterministic 100 Hz → 8 kHz log chirp — full-spectrum.
    {
        let mut s = vec![0.0f32; frames * 2];
        let (f0, f1) = (100.0f32, 8_000.0f32);
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / sr;
            #[allow(clippy::cast_precision_loss)]
            let dur = frames as f32 / sr;
            let k = (f1 / f0).powf(t / dur);
            let phase = std::f32::consts::TAU * f0 * dur / (f1 / f0).ln() * (k - 1.0);
            let v = phase.sin() * 0.6;
            s[i * 2] = v;
            s[i * 2 + 1] = v;
        }
        out.push(Corpus {
            name: "broadband",
            signal: s,
            fundamental: None,
        });
    }

    out
}

/// Drive `produce` block-by-block until it returns 0, timing each block. The
/// closure writes up to `BLOCK` frames into its buffer and returns the count.
fn run_blocked<F: FnMut(&mut [f32]) -> usize>(mut produce: F) -> (Vec<f32>, Vec<f64>) {
    let mut out = Vec::new();
    let mut times = Vec::new();
    let mut block = vec![0.0f32; BLOCK * 2];
    loop {
        let t0 = Instant::now();
        let p = produce(&mut block);
        let dt = t0.elapsed().as_secs_f64() * 1e6;
        if p == 0 {
            break;
        }
        out.extend_from_slice(&block[..p * 2]);
        times.push(dt);
    }
    (out, times)
}

/// Run one backend over `input` at playback rate `rate`, returning the output
/// and the per-block timings.
fn run_backend(backend: Backend, input: &[f32], sr: f32, rate: f64) -> (Vec<f32>, Vec<f64>) {
    let in_frames = input.len() / 2;
    match backend {
        Backend::DubOwn => {
            let mut s = WsolaStretcher::new(sr);
            s.set_time_ratio(1.0 / rate); // S = 1/R
            let mut cursor = 0usize;
            run_blocked(|block| {
                let mut got = 0usize;
                while got < BLOCK {
                    let (c, p) = s.process(&input[cursor * 2..], &mut block[got * 2..]);
                    cursor += c;
                    got += p;
                    if c == 0 && p == 0 {
                        break;
                    }
                }
                got
            })
        }
        Backend::ResamplerOnly => {
            let mut pos = 0.0f64; // fractional read position, source frames
            run_blocked(|block| {
                let mut got = 0usize;
                while got < BLOCK {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let i0 = pos.floor() as usize;
                    if i0 + 1 >= in_frames {
                        break;
                    }
                    #[allow(clippy::cast_possible_truncation)]
                    let frac = (pos - i0 as f64) as f32;
                    let a = i0 * 2;
                    block[got * 2] = input[a] + (input[a + 2] - input[a]) * frac;
                    block[got * 2 + 1] = input[a + 1] + (input[a + 3] - input[a + 1]) * frac;
                    got += 1;
                    pos += rate;
                }
                got
            })
        }
    }
}

/// `n`-point forward FFT magnitude of the Hann-windowed left channel starting
/// at frame `start`. `n` must be ≤ available frames.
fn mag_spectrum(buf: &[f32], start: usize, n: usize) -> Vec<f32> {
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(n);
    let mut indata = r2c.make_input_vec();
    for (i, x) in indata.iter_mut().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let w = 0.5 * (1.0 - (std::f32::consts::TAU * i as f32 / n as f32).cos());
        *x = buf[(start + i) * 2] * w;
    }
    let mut spectrum = r2c.make_output_vec();
    r2c.process(&mut indata, &mut spectrum).expect("fft");
    spectrum.iter().map(|c| c.norm()).collect()
}

/// Dominant frequency (Hz) of the left channel via FFT peak-bin search.
fn fundamental_hz(buf: &[f32], sr: f32) -> Option<f32> {
    let frames = buf.len() / 2;
    let n = 16_384.min(prev_pow2(frames));
    if n < 1_024 {
        return None;
    }
    let start = (frames - n) / 2;
    let mag = mag_spectrum(buf, start, n);
    let (mut best, mut best_mag) = (1usize, 0.0f32);
    for (i, &m) in mag.iter().enumerate().skip(1) {
        if m > best_mag {
            best_mag = m;
            best = i;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    Some(best as f32 * sr / n as f32)
}

fn prev_pow2(x: usize) -> usize {
    if x == 0 {
        return 0;
    }
    1usize << (usize::BITS - 1 - x.leading_zeros())
}

/// Crest factor (peak ÷ RMS) of the left channel.
fn crest(buf: &[f32]) -> f32 {
    let frames = buf.len() / 2;
    if frames == 0 {
        return 0.0;
    }
    let mut peak = 0.0f32;
    let mut sq = 0.0f64;
    for i in 0..frames {
        let v = buf[i * 2];
        peak = peak.max(v.abs());
        sq += f64::from(v) * f64::from(v);
    }
    #[allow(clippy::cast_possible_truncation)]
    let rms = (sq / frames as f64).sqrt() as f32;
    if rms <= 1e-9 {
        0.0
    } else {
        peak / rms
    }
}

/// Energy-normalized long-term log-magnitude spectrum, averaged over windows.
fn avg_log_spectrum(buf: &[f32], sr: f32) -> Vec<f32> {
    let _ = sr;
    let frames = buf.len() / 2;
    let n = 4_096;
    if frames < n {
        return Vec::new();
    }
    let windows = 16.min(frames / n);
    let bins = n / 2 + 1;
    let mut acc = vec![0.0f64; bins];
    for w in 0..windows {
        let start = w * (frames - n) / windows.max(1);
        let mag = mag_spectrum(buf, start, n);
        for (a, &m) in acc.iter_mut().zip(&mag) {
            *a += f64::from(m) * f64::from(m); // power
        }
    }
    // Normalize to unit total energy so only spectral *shape* matters, then dB.
    let total: f64 = acc.iter().sum::<f64>().max(1e-12);
    acc.iter()
        .map(|&p| {
            #[allow(clippy::cast_possible_truncation)]
            {
                (10.0 * (p / total + 1e-12).log10()) as f32
            }
        })
        .collect()
}

/// RMS log-spectral distance (dB) between two equal-length log spectra.
fn lsd(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return f32::NAN;
    }
    let mut sq = 0.0f64;
    for (x, y) in a.iter().zip(b) {
        let d = f64::from(x - y);
        sq += d * d;
    }
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    {
        (sq / a.len() as f64).sqrt() as f32
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx]
}

/// Compute the deterministic quality metrics for one output vs its source.
fn quality(out: &[f32], source: &[f32], fundamental: Option<f32>, sr: f32) -> Quality {
    let pitch_cents = fundamental.and_then(|f0| {
        fundamental_hz(out, sr).map(|measured| {
            #[allow(clippy::cast_possible_truncation)]
            {
                (1200.0 * (f64::from(measured) / f64::from(f0)).log2()) as f32
            }
        })
    });
    let crest_ratio = {
        let cs = crest(source);
        if cs <= 1e-6 {
            0.0
        } else {
            crest(out) / cs
        }
    };
    let lsd_db = lsd(&avg_log_spectrum(out, sr), &avg_log_spectrum(source, sr));
    Quality {
        out_frames: out.len() / 2,
        pitch_cents,
        crest_ratio,
        lsd_db,
    }
}

/// Timings → reportable CPU stats. `out_frames` for the per-second normalizer.
fn timing(times: &[f64], out_frames: usize, sr: f32) -> Timing {
    let mut sorted = times.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let total_us: f64 = times.iter().sum();
    #[allow(clippy::cast_precision_loss)]
    let out_secs = out_frames as f64 / f64::from(sr);
    let deadline_us = f64::from(BLOCK as u32) / f64::from(sr) * 1e6;
    #[allow(clippy::cast_precision_loss)]
    let n = times.len().max(1) as f64;
    let xrun_1 = times.iter().filter(|&&t| t > deadline_us).count() as f64 / n * 100.0;
    let xrun_2 = times.iter().filter(|&&t| 2.0 * t > deadline_us).count() as f64 / n * 100.0;
    Timing {
        cpu_ms_per_s: if out_secs > 0.0 {
            total_us / 1000.0 / out_secs
        } else {
            0.0
        },
        p50_us: percentile(&sorted, 50.0),
        p99_us: percentile(&sorted, 99.0),
        xrun_1,
        xrun_2,
    }
}

fn fmt_cents(c: Option<f32>) -> String {
    c.map_or_else(|| "    -".to_string(), |v| format!("{v:+5.0}"))
}

/// Load an interleaved-stereo `f32` buffer from a WAV (mono is duplicated).
fn load_wav(path: &Path) -> Result<(Vec<f32>, f32)> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let chans = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / f32::from(i16::MAX);
            reader
                .samples::<i16>()
                .map(|r| r.map(|s| f32::from(s) * scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mut out = Vec::with_capacity(samples.len() / chans * 2);
    for frame in samples.chunks(chans) {
        let l = frame.first().copied().unwrap_or(0.0);
        let r = if chans >= 2 { frame[1] } else { l };
        out.push(l);
        out.push(r);
    }
    #[allow(clippy::cast_precision_loss)]
    Ok((out, spec.sample_rate as f32))
}

/// Write an interleaved-stereo `f32` buffer to a float WAV.
fn dump_wav(dir: &Path, name: &str, buf: &[f32], sr: f32) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{name}.wav"));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sr as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(&path, spec)?;
    for &s in buf {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

/// Backends to print in the live table.
fn table_backends() -> Vec<Backend> {
    vec![Backend::ResamplerOnly, Backend::DubOwn]
}

pub fn run(args: &[String]) -> Result<()> {
    let mut input_path: Option<String> = None;
    let mut dump_dir: Option<String> = None;
    let mut rates = RATES_SHORT;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                input_path = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--input needs a path"))?
                        .clone(),
                );
                i += 2;
            }
            "--dump" => {
                dump_dir = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("--dump needs a dir"))?
                        .clone(),
                );
                i += 2;
            }
            "--full" => {
                rates = RATES_FULL;
                i += 1;
            }
            other => return Err(anyhow!("unknown stretch-bench flag: {other}")),
        }
    }

    let corpus = if let Some(p) = &input_path {
        let (signal, sr) = load_wav(Path::new(p))?;
        vec![(
            Corpus {
                name: "input",
                signal,
                fundamental: None,
            },
            sr,
        )]
    } else {
        synthetic_corpus(48_000.0, 6.0)
            .into_iter()
            .map(|c| (c, 48_000.0))
            .collect()
    };

    println!("stretch-bench — block={BLOCK}, backends: resampler-only vs dub-wsola");
    println!("  (rate R = playback speed; WSOLA runs at time_ratio 1/R; pitch cents = deviation from source)");
    if cfg!(debug_assertions) {
        println!("  ⚠ DEBUG build — CPU columns are ~5–10× slower than release. Run with");
        println!("    `cargo run --release -p dub-cli -- stretch-bench` for representative CPU.");
    }
    println!();

    for (item, sr) in &corpus {
        println!(
            "== {} ({:.1}s @ {:.0} Hz) ==",
            item.name,
            item.signal.len() as f32 / 2.0 / sr,
            sr
        );
        println!(
            "  {:>5} {:>10} {:>8} {:>8} {:>8} {:>8} {:>8} {:>5} {:>6} {:>6} {:>6}",
            "rate",
            "backend",
            "ms/s",
            "p50us",
            "p99us",
            "xrun1%",
            "xrun2%",
            "lat",
            "cents",
            "crest",
            "lsd"
        );
        for &rate in rates {
            for &backend in &table_backends() {
                let (out, times) = run_backend(backend, &item.signal, *sr, rate);
                let q = quality(&out, &item.signal, item.fundamental, *sr);
                let t = timing(&times, q.out_frames, *sr);
                let lat = match backend {
                    Backend::ResamplerOnly => 0,
                    Backend::DubOwn => WsolaStretcher::new(*sr).latency_frames(),
                };
                println!(
                    "  {:>5.2} {:>10} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>5} {:>6} {:>6.3} {:>6.2}",
                    rate, backend.label(), t.cpu_ms_per_s, t.p50_us, t.p99_us, t.xrun_1, t.xrun_2,
                    lat, fmt_cents(q.pitch_cents), q.crest_ratio, q.lsd_db
                );
                if let Some(dir) = &dump_dir {
                    let name = format!("{}_{}_{:.2}", item.name, backend.label(), rate);
                    dump_wav(Path::new(dir), &name, &out, *sr)?;
                }
            }
        }
        println!();
    }

    if input_path.is_none() {
        println!(
            "note: quality metrics are deterministic; CPU varies per run. Pitch cents ≈ 0 for"
        );
        println!("      dub-wsola (key lock holds pitch) vs ≈1200·log2(R) for resampler-only.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pair the golden snapshots.
    const BACKENDS: &[Backend] = &[Backend::ResamplerOnly, Backend::DubOwn];

    /// Build the deterministic quality lines for a tiny sweep (no timing) so
    /// the golden is stable and fast.
    fn quality_summary() -> String {
        let mut lines = Vec::new();
        for item in synthetic_corpus(48_000.0, 3.0) {
            for &rate in &[0.94_f64, 1.06] {
                for &backend in BACKENDS {
                    let (out, _) = run_backend(backend, &item.signal, 48_000.0, rate);
                    let q = quality(&out, &item.signal, item.fundamental, 48_000.0);
                    lines.push(format!(
                        "{:<10} {:>4.2} {:<10} frames={} cents={} crest={:.2} lsd={:.1}",
                        item.name,
                        rate,
                        backend.label(),
                        q.out_frames,
                        fmt_cents(q.pitch_cents).trim(),
                        q.crest_ratio,
                        q.lsd_db
                    ));
                }
            }
        }
        lines.join("\n")
    }

    #[test]
    fn resampler_shifts_pitch_wsola_holds_it() {
        // The headline correctness claim: at rate R the resampler baseline
        // moves pitch by ~1200·log2(R) cents while WSOLA keeps it near 0.
        let corpus = synthetic_corpus(48_000.0, 3.0);
        let tonal = corpus.iter().find(|c| c.name == "tonal").unwrap();
        let rate = 1.06_f64;
        let expected_shift = 1200.0 * rate.log2(); // ≈ +100.8 cents

        let (res, _) = run_backend(Backend::ResamplerOnly, &tonal.signal, 48_000.0, rate);
        let (wso, _) = run_backend(Backend::DubOwn, &tonal.signal, 48_000.0, rate);
        let qr = quality(&res, &tonal.signal, tonal.fundamental, 48_000.0);
        let qw = quality(&wso, &tonal.signal, tonal.fundamental, 48_000.0);

        let rc = qr.pitch_cents.unwrap();
        let wc = qw.pitch_cents.unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let exp = expected_shift as f32;
        assert!(
            (rc - exp).abs() < 30.0,
            "resampler cents {rc} vs expected ~{exp}"
        );
        assert!(wc.abs() < 30.0, "wsola should hold pitch, got {wc} cents");
    }

    #[test]
    fn golden_quality_metrics() {
        // Deterministic quality metrics (no timing). The story: dub-wsola holds
        // pitch (cents ≈ 0 for clean tones) and timbre (low lsd) where the
        // resampler shifts both; transients survive (crest ≥ 1.0).
        insta::assert_snapshot!(quality_summary(), @r###"
        percussive 0.94 resampler  frames=153191 cents=-130 crest=1.00 lsd=3.5
        percussive 0.94 dub-wsola  frames=151679 cents=-41 crest=1.09 lsd=1.2
        percussive 1.06 resampler  frames=135849 cents=+124 crest=1.00 lsd=4.2
        percussive 1.06 dub-wsola  frames=134879 cents=-41 crest=1.17 lsd=1.8
        tonal      0.94 resampler  frames=153191 cents=-97 crest=1.00 lsd=6.4
        tonal      0.94 dub-wsola  frames=151679 cents=-2 crest=1.00 lsd=0.7
        tonal      1.06 resampler  frames=135849 cents=+110 crest=1.00 lsd=6.5
        tonal      1.06 dub-wsola  frames=134879 cents=-2 crest=1.00 lsd=0.7
        sub-bass   0.94 resampler  frames=153191 cents=-73 crest=1.00 lsd=1.7
        sub-bass   0.94 dub-wsola  frames=151679 cents=+21 crest=1.00 lsd=0.2
        sub-bass   1.06 resampler  frames=135849 cents=+110 crest=1.00 lsd=3.3
        sub-bass   1.06 dub-wsola  frames=134879 cents=+21 crest=1.00 lsd=0.2
        broadband  0.94 resampler  frames=153191 cents=- crest=1.01 lsd=18.6
        broadband  0.94 dub-wsola  frames=151679 cents=- crest=1.00 lsd=7.9
        broadband  1.06 resampler  frames=135849 cents=- crest=1.01 lsd=17.8
        broadband  1.06 dub-wsola  frames=134879 cents=- crest=1.00 lsd=14.0
        "###);
    }
}
