//! `dub decode-timecode <wav>` — offline timecode-vinyl decoder.
//!
//! Reads a stereo WAV containing recorded timecode (Serato CV02 in v1)
//! and reports decoded rate / position / amplitude / confidence in
//! discrete time slices. Use this to validate the decoder against
//! real-world recordings before plugging in a turntable in M5.3.
//!
//! With `--synthetic` and no input path the CLI generates a known
//! signal and decodes it — a sanity check for the decoder math
//! independent of any audio interface.
//!
//! Output is a TSV-ish report (one slice per line) followed by a
//! summary verdict — "LOCKED" if confidence and amplitude stayed in
//! plausible ranges, "POOR" otherwise, with a short reason.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use dub_timecode::{
    deep_sweep, extract_cycles, sweep_conventions, sweep_xwax, DecodeOutput, Decoder, Format,
};

/// Default analysis-window size in milliseconds. Smaller = more
/// rate-tracking detail in the report; larger = better noise rejection.
/// 25 ms (= 1200 samples @ 48 kHz, ~25 carrier cycles) is a good
/// balance for human-readable output.
pub const DEFAULT_WINDOW_MS: f32 = 25.0;

/// Run `dub decode-timecode` with the parsed CLI options.
///
/// # Errors
/// Returns any error from WAV decoding, sample-format mismatches,
/// or filesystem I/O.
pub fn run(
    input: Option<&Path>,
    synthetic: bool,
    sweep: bool,
    window_ms: f32,
    max_lines: usize,
    format: Format,
) -> Result<()> {
    if synthetic {
        return run_synthetic(window_ms, max_lines, format);
    }
    let path = input.ok_or_else(|| {
        anyhow!(
            "usage: dub decode-timecode <wav> [--format serato-cv02|traktor-mk1|traktor-mk2] \
             [--window MS] [--head N] [--sweep]"
        )
    })?;
    if sweep {
        return run_sweep(path, format);
    }
    run_file(path, window_ms, max_lines, format)
}

/// `dub decode-timecode <wav> --sweep` — find the pressing's absolute
/// bitstream convention. Extracts the per-cycle AM bits from the capture
/// and scores every plausible polarity / bit-order / tap-polynomial
/// combination against the LFSR recurrence. The winning row (agreement
/// ≈ 1.0, a long consistent run) is the convention the decoder must use;
/// if every row sits near 0.5, the one-bit-per-cycle model itself
/// doesn't match this disc.
fn run_sweep(path: &Path, format: Format) -> Result<()> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 2 {
        return Err(anyhow!(
            "decode-timecode --sweep requires a stereo WAV; got {} channels",
            spec.channels
        ));
    }
    let interleaved = read_stereo_f32(&mut reader)
        .with_context(|| format!("reading samples from {}", path.display()))?;

    println!(
        "convention sweep: {}\n  sr={} Hz, fmt={:?} ({}-bit LFSR, carrier {} Hz)",
        path.display(),
        spec.sample_rate,
        format,
        format.position_bits(),
        format.carrier_hz(),
    );

    let cycles = extract_cycles(format, &interleaved);
    let forward_full = cycles.iter().filter(|c| c.forward && c.full).count();
    println!(
        "  carrier cycles extracted: {} ({} forward/full → {} bits)\n",
        cycles.len(),
        forward_full,
        forward_full.saturating_sub(1),
    );
    if forward_full < 256 {
        println!(
            "  too few clean cycles to judge a convention — is this the right \
             channel pair / a steady-play section?"
        );
        return Ok(());
    }

    let results = sweep_conventions(format, &cycles);
    println!(
        "  {:>4}  {:>8}  {:>4}  {:>9}  {:>10}  {:>11}  {:>8}",
        "rank", "polarity", "rev", "taps", "agreement", "longest-run", "balance"
    );
    for (i, r) in results.iter().enumerate().take(8) {
        println!(
            "  {:>4}  {:>8}  {:>4}  {:>9}  {:>9.1}%  {:>11}  {:>7.1}%",
            i + 1,
            if r.polarity_inverted {
                "inverted"
            } else {
                "normal"
            },
            if r.reversed { "yes" } else { "no" },
            if r.tap_reversed {
                "reciprocal"
            } else {
                "direct"
            },
            r.agreement * 100.0,
            r.longest_run,
            r.balance * 100.0,
        );
    }

    // Deep sweep: the per-cycle total-amplitude model above is just one
    // hypothesis. Try every other bit observable (per-channel, channel
    // difference, carrier phase) and bit rate (1..16 cycles per bit) to
    // find *where* and *at what rate* the data bit actually lives.
    const MAX_DECIMATION: usize = 16;
    println!("\n  deep sweep (observable × bit-rate × phase):");
    let deep = deep_sweep(format, &cycles, MAX_DECIMATION);
    println!(
        "  {:>10}  {:>5}  {:>5}  {:>10}  {:>11}  {:>8}",
        "observable", "cyc/b", "phase", "agreement", "longest-run", "balance"
    );
    for r in deep.iter().take(12) {
        let c = r.convention;
        println!(
            "  {:>10}  {:>5}  {:>5}  {:>9.1}%  {:>11}  {:>7.1}%",
            r.observable.label(),
            r.decimation,
            r.phase,
            c.agreement * 100.0,
            c.longest_run,
            c.balance * 100.0,
        );
    }

    // xwax-style decode: the REAL Serato algorithm — per-channel zero-
    // crossing timing, read the primary channel's peak at the secondary's
    // crossing. Sweeps both channel assignments and polarities.
    println!("\n  xwax-style decode (the real Serato algorithm, all variants):");
    let xwax = sweep_xwax(&interleaved, spec.sample_rate as f32);
    println!(
        "  {:>10}  {:>9}  {:>9}  {:>10}  {:>11}  {:>8}",
        "variant", "primary", "polarity", "agreement", "longest-run", "balance"
    );
    for r in xwax.iter().take(8) {
        let c = r.convention;
        println!(
            "  {:>10}  {:>9}  {:>9}  {:>9.1}%  {:>11}  {:>7.1}%",
            r.variant,
            if r.primary_right {
                "right/ch1"
            } else {
                "left/ch0"
            },
            if r.switch_polarity {
                "switched"
            } else {
                "normal"
            },
            c.agreement * 100.0,
            c.longest_run,
            c.balance * 100.0,
        );
    }

    println!();
    // A real decode is BALANCED (≈50% ones), high-agreement, and runs
    // long. A constant stream scores 100%/huge-run but ~0% balance — the
    // deadlock artifact that faked a hit before; reject it explicitly.
    let balanced = |b: f64| (0.4..=0.6).contains(&b);
    if let Some(best) = xwax.first() {
        let c = best.convention;
        if c.agreement > 0.95 && balanced(c.balance) {
            println!(
                "VERDICT: REAL DECODE — xwax algorithm, variant={}, primary={}, \
                 polarity={}, taps={}, agreement {:.1}%. This is the Serato bit \
                 encoding; wiring the tracker to read it this way.",
                best.variant,
                if best.primary_right {
                    "right/ch1"
                } else {
                    "left/ch0"
                },
                if best.switch_polarity {
                    "switched"
                } else {
                    "normal"
                },
                if c.tap_reversed {
                    "reciprocal"
                } else {
                    "direct"
                },
                c.agreement * 100.0,
            );
            return Ok(());
        }
    }
    let best_deep = deep.first().copied();
    match best_deep {
        Some(r)
            if r.convention.agreement > 0.95
                && r.convention.longest_run > 500
                && balanced(r.convention.balance) =>
        {
            let c = r.convention;
            println!(
                "VERDICT: data bit FOUND — observable={}, {} carrier cycle(s)/bit, \
                 phase {}, polarity={}, taps={}. This identifies the real CV02 \
                 encoding; next step is to teach the decoder to read it here.",
                r.observable.label(),
                r.decimation,
                r.phase,
                if c.polarity_inverted {
                    "inverted"
                } else {
                    "normal"
                },
                if c.tap_reversed {
                    "reciprocal"
                } else {
                    "direct"
                },
            );
        }
        _ => {
            println!(
                "VERDICT: no observable / bit-rate obeys the LFSR recurrence \
                 (all near chance). The bit isn't in per-cycle amplitude, \
                 channel difference, or carrier phase at any rate ≤ {MAX_DECIMATION}. \
                 The taps/seed or the cycle model itself need reference-source study."
            );
        }
    }
    Ok(())
}

fn run_file(path: &Path, window_ms: f32, max_lines: usize, format: Format) -> Result<()> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 2 {
        return Err(anyhow!(
            "decode-timecode requires a stereo WAV; got {} channels",
            spec.channels
        ));
    }
    let sample_rate = spec.sample_rate as f32;
    if !(32_000.0..=192_000.0).contains(&sample_rate) {
        eprintln!(
            "warning: unusual sample rate {} Hz — decoder is tuned for 44.1–96 kHz",
            spec.sample_rate
        );
    }

    println!(
        "decode-timecode: {}\n  sr={} Hz, ch={}, bps={}, fmt={:?}",
        path.display(),
        spec.sample_rate,
        spec.channels,
        spec.bits_per_sample,
        spec.sample_format
    );

    // Read everything into memory. Real-world timecode WAVs are short
    // (< 2 min for a captured groove); we don't need streaming for the
    // offline tool.
    let interleaved = read_stereo_f32(&mut reader)
        .with_context(|| format!("reading samples from {}", path.display()))?;

    decode_and_report(format, sample_rate, &interleaved, window_ms, max_lines)
}

fn run_synthetic(window_ms: f32, max_lines: usize, format: Format) -> Result<()> {
    use dub_timecode::signal::Generator;
    let sample_rate = 48_000.0_f32;
    println!(
        "decode-timecode: SYNTHETIC ({:?}, sr={} Hz, no input file)",
        format, sample_rate
    );
    println!("  scenario: 1 s @ 1.0× → 1 s @ 0.5× → 1 s @ -1.0× → 1 s silence");

    let mut g = Generator::new(format, sample_rate);
    if g.enable_absolute(format, 0.15) {
        println!("  absolute: LFSR AM modulation on ch0 (depth 0.15)");
    }
    let one_sec = 48_000_usize;
    let mut buf = vec![0.0f32; one_sec * 2 * 4];
    let (a, rest) = buf.split_at_mut(one_sec * 2);
    let (b, rest) = rest.split_at_mut(one_sec * 2);
    let (c, d) = rest.split_at_mut(one_sec * 2);
    g.render(a, 1.0, 0.5);
    g.render(b, 0.5, 0.5);
    g.render(c, -1.0, 0.5);
    for s in d.iter_mut() {
        *s = 0.0;
    }
    decode_and_report(format, sample_rate, &buf, window_ms, max_lines)
}

fn decode_and_report(
    format: Format,
    sample_rate: f32,
    interleaved: &[f32],
    window_ms: f32,
    max_lines: usize,
) -> Result<()> {
    // M6: absolute LFSR decoding on, mirroring the engine's deck path —
    // this tool is the offline rig for validating real-vinyl captures.
    let mut decoder = Decoder::with_absolute(format, sample_rate);
    // Calibrate channel whitening from the capture, like the engine does
    // on a clean spin. The absolute tracker derives its cycle boundaries
    // from the whitened carrier; without this the bit windows misalign on
    // any real channel imbalance and the LFSR never locks.
    decoder.calibrate(interleaved);

    let window_frames = ((window_ms / 1000.0) * sample_rate).round().max(64.0) as usize;
    let total_frames = interleaved.len() / 2;
    let total_secs = total_frames as f64 / f64::from(sample_rate);

    println!(
        "  format: {format:?} (carrier {} Hz)\n  window: {window_ms:.1} ms ({window_frames} frames)\n  total:  {total_secs:.3} s ({total_frames} frames)\n",
        format.carrier_hz()
    );
    println!("  t(s)\trate\tposition(s)\tamp\tconfidence\tabs(s)\tabsconf");

    let mut printed = 0_usize;
    let mut hidden = 0_usize;
    let mut summary = SummaryStats::default();
    let mut t_frames = 0_usize;

    while t_frames + window_frames <= total_frames {
        let start = t_frames * 2;
        let end = (t_frames + window_frames) * 2;
        let block = &interleaved[start..end];
        let out = decoder.process(block);
        summary.update(&out);
        #[allow(clippy::cast_precision_loss)]
        summary.update_abs(&out, window_frames as f64);

        let t_secs = t_frames as f64 / f64::from(sample_rate);
        if printed < max_lines {
            let abs = out.abs_position_frames.map_or_else(
                || "      -".to_string(),
                |f| format!("{:8.3}", f / f64::from(sample_rate)),
            );
            println!(
                "  {t_secs:6.3}\t{:+.4}\t{:+.4}\t{:.3}\t{:.3}\t{abs}\t{:.2}",
                out.rate, out.position_secs, out.amplitude, out.confidence, out.abs_confidence
            );
            printed += 1;
        } else {
            hidden += 1;
        }
        t_frames += window_frames;
    }
    if hidden > 0 {
        println!(
            "  ... ({hidden} more windows omitted; pass --head {} to see all)",
            printed + hidden
        );
    }

    summary.report();
    if let Some(variant) = decoder.absolute_variant() {
        println!("  absolute variant locked: {variant}");
    }
    if let Some((crossings, lut_hits, max_consec)) = decoder.absolute_debug() {
        println!(
            "  abs acquisition: {crossings} cycle crossings, {lut_hits} LUT hits, \
             max consecutive sequence-hits {max_consec} (lock needs 32)"
        );
    }
    if let Some((means, bits)) = decoder.absolute_first_cycles() {
        let mn = means.iter().cloned().fold(f32::INFINITY, f32::min);
        let mx = means.iter().cloned().fold(0.0_f32, f32::max);
        let bitstr: String = (0..48)
            .map(|i| if (bits >> i) & 1 == 1 { '1' } else { '0' })
            .collect();
        println!("  first-48 bit_means: min={mn:.4} max={mx:.4}");
        println!("  first-48 sliced bits: {bitstr}");
        print!("  first-12 means:");
        for m in means.iter().take(12) {
            print!(" {m:.4}");
        }
        println!();
    }
    Ok(())
}

#[derive(Default)]
struct SummaryStats {
    n: u64,
    rate_min: f64,
    rate_max: f64,
    amp_min: f32,
    amp_max: f32,
    conf_min: f32,
    conf_max: f32,
    conf_sum: f64,
    locked_windows: u64,
    /// Analysis-window frames, set on the first update — needed to turn
    /// consecutive absolute fixes into a per-window advance.
    window_frames: f64,
    abs_windows: u64,
    /// Consecutive abs-fix pairs whose delta matched `rate × window`
    /// within half a carrier cycle — a *coherent* incremental track.
    abs_continuous: u64,
    /// Consecutive abs-fix pairs with an implausible delta — each one
    /// is a fresh acquisition at an unrelated position (the spurious-
    /// lock signature from the first on-rig run).
    abs_jumps: u64,
    prev_abs: Option<f64>,
}

impl SummaryStats {
    fn update_abs(&mut self, o: &DecodeOutput, window_frames: f64) {
        if self.window_frames == 0.0 {
            self.window_frames = window_frames;
        }
        let Some(p) = o.abs_position_frames else {
            self.prev_abs = None;
            return;
        };
        self.abs_windows += 1;
        if let Some(prev) = self.prev_abs {
            let expected = o.rate * window_frames;
            // Half a Serato carrier cycle (~24 frames at 48 kHz) of slack
            // absorbs sub-cycle phase noise; anything beyond a quarter
            // window is a re-acquisition, not tracking.
            let err = (p - prev - expected).abs();
            if err < window_frames * 0.25 {
                self.abs_continuous += 1;
            } else {
                self.abs_jumps += 1;
            }
        }
        self.prev_abs = Some(p);
    }

    fn update(&mut self, o: &DecodeOutput) {
        if self.n == 0 {
            self.rate_min = o.rate;
            self.rate_max = o.rate;
            self.amp_min = o.amplitude;
            self.amp_max = o.amplitude;
            self.conf_min = o.confidence;
            self.conf_max = o.confidence;
        } else {
            self.rate_min = self.rate_min.min(o.rate);
            self.rate_max = self.rate_max.max(o.rate);
            self.amp_min = self.amp_min.min(o.amplitude);
            self.amp_max = self.amp_max.max(o.amplitude);
            self.conf_min = self.conf_min.min(o.confidence);
            self.conf_max = self.conf_max.max(o.confidence);
        }
        self.n += 1;
        self.conf_sum += f64::from(o.confidence);
        // "Locked" = confidence > 0.5 AND amplitude > 0.01 — the
        // decoder thinks it's tracking a real carrier. Sub-threshold
        // windows are stylus-lifted or noise.
        if o.confidence > 0.5 && o.amplitude > 0.01 {
            self.locked_windows += 1;
        }
    }

    fn report(&self) {
        if self.n == 0 {
            println!("\nverdict: NO WINDOWS — input shorter than analysis window");
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let conf_avg = self.conf_sum / self.n as f64;
        #[allow(clippy::cast_precision_loss)]
        let lock_pct = 100.0 * self.locked_windows as f64 / self.n as f64;
        println!(
            "\nsummary across {} windows:\n  rate range:  {:+.4} .. {:+.4}\n  amp range:   {:.4} .. {:.4}\n  confidence:  {:.3} .. {:.3} (avg {:.3})\n  locked:      {}/{} ({:.1}%)",
            self.n,
            self.rate_min,
            self.rate_max,
            self.amp_min,
            self.amp_max,
            self.conf_min,
            self.conf_max,
            conf_avg,
            self.locked_windows,
            self.n,
            lock_pct,
        );
        #[allow(clippy::cast_precision_loss)]
        let abs_pct = 100.0 * self.abs_windows as f64 / self.n as f64;
        println!(
            "  absolute:    {}/{} windows with an LFSR fix ({:.1}%) — {} continuous, {} jumps",
            self.abs_windows, self.n, abs_pct, self.abs_continuous, self.abs_jumps,
        );
        if self.abs_jumps > self.abs_continuous && self.abs_windows > 4 {
            println!(
                "  NOTE: abs fixes are mostly re-acquisitions at unrelated positions — \
                 the bitstream convention (polarity / bit order) likely doesn't match \
                 this pressing. The deck is protected (chain rule drops these), but \
                 absolute mode is effectively off."
            );
        }
        // Verdict heuristic — calibrated to the synthetic test scenarios.
        // Real-world tolerance lands in M5.3 once we have actual cartridge captures.
        let verdict = if conf_avg > 0.9 && lock_pct > 80.0 {
            "LOCKED — decoder tracked the carrier across most of the input"
        } else if lock_pct > 50.0 {
            "PARTIAL — significant locked sections; check unlocked windows for transients/silence"
        } else {
            "POOR — decoder did not lock onto a carrier; likely wrong format, wrong channel, or no signal"
        };
        println!("verdict: {verdict}");
    }
}

/// Read a (potentially integer-PCM) WAV into normalized stereo f32.
fn read_stereo_f32(
    reader: &mut hound::WavReader<std::io::BufReader<std::fs::File>>,
) -> Result<Vec<f32>> {
    let spec = reader.spec();
    let mut out: Vec<f32> = Vec::with_capacity(reader.len() as usize);
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                out.push(s.context("reading float sample")?);
            }
        }
        hound::SampleFormat::Int => {
            // Normalize to ±1.0 based on the bit depth.
            let scale = 1.0_f32 / ((1_i64 << (spec.bits_per_sample - 1)) as f32);
            for s in reader.samples::<i32>() {
                let v = s.context("reading int sample")?;
                #[allow(clippy::cast_precision_loss)]
                out.push(v as f32 * scale);
            }
        }
    }
    Ok(out)
}

/// Argument parser used by `main`.
pub fn parse_args(args: &[String]) -> Result<(Option<PathBuf>, bool, bool, f32, usize, Format)> {
    let mut input: Option<PathBuf> = None;
    let mut synthetic = false;
    let mut sweep = false;
    let mut window_ms = DEFAULT_WINDOW_MS;
    let mut max_lines = 40_usize;
    let mut format = Format::SeratoCv02;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--synthetic" | "--self-test" => {
                synthetic = true;
                i += 1;
            }
            "--sweep" => {
                sweep = true;
                i += 1;
            }
            "--window" | "--window-ms" => {
                window_ms = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--window expects a value in ms"))?
                    .parse()
                    .context("--window not a number")?;
                i += 2;
            }
            "--head" => {
                max_lines = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--head expects an integer"))?
                    .parse()
                    .context("--head not an integer")?;
                i += 2;
            }
            "--format" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    anyhow!("--format expects 'serato-cv02', 'traktor-mk1', or 'traktor-mk2'")
                })?;
                format = Format::from_cli_arg(v).ok_or_else(|| {
                    anyhow!(
                        "unknown --format '{v}' (supported: serato-cv02, traktor-mk1, traktor-mk2)"
                    )
                })?;
                i += 2;
            }
            s if s.starts_with('-') => {
                return Err(anyhow!("unknown flag: {s}"));
            }
            _ => {
                if input.is_some() {
                    return Err(anyhow!("unexpected positional arg: {}", args[i]));
                }
                input = Some(PathBuf::from(&args[i]));
                i += 1;
            }
        }
    }
    Ok((input, synthetic, sweep, window_ms, max_lines, format))
}
