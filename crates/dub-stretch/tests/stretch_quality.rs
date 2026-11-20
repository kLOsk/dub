//! What a stretched record sounds like, in numbers.
//!
//! Key lock plays a record at the platter's tempo and the record's own
//! pitch. Three things can go wrong in a way a DJ hears on a big system,
//! and each is measured here on a signal built to expose it:
//!
//! - **A held sub note wobbles or goes out of tune.** A stretcher joins
//!   pieces of the record; a join that does not continue the bass wave is a
//!   dip in the note, and a stretcher that cannot jump a whole bass wave
//!   lets the note follow the platter. Measured as the deepest dip of a held
//!   sine and how far its pitch sits from the record's.
//! - **A drum hit moves, doubles or fades.** Measured by finding each hit
//!   of a known shape in the output: how far it landed from where the
//!   tempo puts it, whether a second copy sits beside it, and how much of
//!   it arrived.
//! - **The top end dulls.** Measured as the level of noise between 12 and
//!   16 kHz against the same noise before stretching.
//!
//! The stretcher this replaced (WSOLA, fed a record already played fast or
//! slow by straight-line interpolation) read, on these same signals: sub
//! notes under 48 Hz off by the platter's whole pitch (133 cents at +8 %),
//! hits up to 11 ms off their beat with every one under 60 % strength,
//! and −5 dB at 12–16 kHz. `dub stretch-bench` still runs both, on these
//! signals or on a file.
//!
//! Run with `--nocapture` for the whole table.

use dub_stretch::{SpliceStretcher, TimeStretcher};
use realfft::RealFftPlanner;

const SR: f32 = 48_000.0;
const SR_F64: f64 = 48_000.0;

/// Platter rates a DJ holds a record at, as multiples of its own tempo.
const RATES: [f64; 6] = [0.92, 0.96, 0.98, 1.02, 1.04, 1.08];

fn drive<S: TimeStretcher>(stretcher: &mut S, input: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(input.len() + 8_192);
    let mut scratch = vec![0.0f32; 512 * 2];
    let mut cursor = 0usize;
    loop {
        let (consumed, produced) = stretcher.process(&input[cursor * 2..], &mut scratch);
        cursor += consumed;
        out.extend_from_slice(&scratch[..produced * 2]);
        if consumed == 0 && produced == 0 {
            return out;
        }
    }
}

/// The record at `rate` times its tempo and its own pitch, left channel.
fn key_locked<S: TimeStretcher>(stretcher: &mut S, mono: &[f32], rate: f64) -> Vec<f32> {
    stretcher.set_time_ratio(1.0 / rate);
    let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
    drive(stretcher, &stereo)
        .chunks_exact(2)
        .map(|f| f[0])
        .collect()
}

struct Lcg(u32);

impl Lcg {
    /// Uniform in −1..1.
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 8_388_608.0 - 1.0
    }
}

fn sine(hz: f64, secs: f64, level: f32) -> Vec<f32> {
    (0..(secs * SR_F64) as usize)
        .map(|i| (std::f64::consts::TAU * hz * i as f64 / SR_F64).sin() as f32 * level)
        .collect()
}

// ---------------------------------------------------------------- sub note

struct SubReading {
    /// Deepest dip of the note below its usual level, in dB (0 = none).
    deepest_dip_db: f64,
    /// How far the note's pitch sits from the record's, in cents.
    cents_off: f64,
}

/// The level of a held `hz` sine over time, read one period at a time.
fn held_note_reading(out: &[f32], hz: f64) -> SubReading {
    let span = (SR_F64 / hz).round() as usize;
    let (mut i_sum, mut q_sum) = (vec![0.0f64; out.len() + 1], vec![0.0f64; out.len() + 1]);
    for (n, &s) in out.iter().enumerate() {
        let phase = std::f64::consts::TAU * hz * n as f64 / SR_F64;
        i_sum[n + 1] = i_sum[n] + f64::from(s) * phase.cos();
        q_sum[n + 1] = q_sum[n] + f64::from(s) * phase.sin();
    }
    let edge = (0.15 * SR_F64) as usize;
    let mut levels: Vec<f64> = (edge..out.len().saturating_sub(edge + span))
        .step_by(16)
        .map(|n| (i_sum[n + span] - i_sum[n]).hypot(q_sum[n + span] - q_sum[n]))
        .collect();
    let mut sorted = levels.clone();
    sorted.sort_by(f64::total_cmp);
    let usual = sorted[sorted.len() / 2].max(1e-12);
    for level in &mut levels {
        *level = 20.0 * (level.max(1e-12) / usual).log10();
    }
    // The note's own pitch: how fast its phase turns against the pitch it
    // should have, over the middle of the take.
    let turns: Vec<f64> = (edge..out.len().saturating_sub(edge + span))
        .step_by(span)
        .map(|n| (q_sum[n + span] - q_sum[n]).atan2(i_sum[n + span] - i_sum[n]))
        .collect();
    let mut turned = 0.0f64;
    for pair in turns.windows(2) {
        let mut step = pair[1] - pair[0];
        step -= std::f64::consts::TAU * (step / std::f64::consts::TAU).round();
        turned += step;
    }
    let secs = (turns.len().max(2) - 1) as f64 * span as f64 / SR_F64;
    let hz_off = -turned / std::f64::consts::TAU / secs;
    let cents_off = 1_200.0 * ((hz + hz_off) / hz).log2();
    let deepest = levels.iter().copied().fold(0.0f64, f64::min);
    SubReading {
        deepest_dip_db: -deepest,
        cents_off,
    }
}

// --------------------------------------------------------------- drum hits

const HIT_FRAMES: usize = 384;

/// One drum hit: 8 ms of noise dying away fast.
fn hit_shape() -> Vec<f32> {
    let mut noise = Lcg(0x00C0_FFEE);
    (0..HIT_FRAMES)
        .map(|k| noise.next() * 0.8 * (-(k as f32) / 96.0).exp())
        .collect()
}

/// A bass note and a chord with hits at uneven times; returns the hit times
/// in frames.
fn hits_over_a_bed(secs: f64) -> (Vec<f32>, Vec<usize>) {
    let frames = (secs * SR_F64) as usize;
    let mut signal = vec![0.0f32; frames];
    for (hz, level) in [(55.0, 0.35), (220.0, 0.08), (277.18, 0.08), (329.63, 0.08)] {
        for (s, v) in signal.iter_mut().zip(sine(hz, secs, level)) {
            *s += v;
        }
    }
    let shape = hit_shape();
    let mut gaps = Lcg(7);
    let mut at = (0.5 * SR_F64) as usize;
    let mut times = Vec::new();
    let tail = (0.5 * SR_F64) as usize;
    while at + HIT_FRAMES + tail < frames {
        for (k, &v) in shape.iter().enumerate() {
            signal[at + k] += v;
        }
        times.push(at);
        at += ((0.42 + 0.16 * f64::from(gaps.next())) * SR_F64) as usize;
    }
    (signal, times)
}

#[derive(Default)]
struct HitReading {
    hits: usize,
    /// Hits with a second copy at least a third as strong beside them.
    doubled: usize,
    /// Hits that arrived at under 60 % strength.
    faded: usize,
    /// How far hits landed from where the tempo puts them, in ms (+ = late).
    late_ms: Vec<f64>,
}

impl HitReading {
    fn median_late_ms(&self) -> f64 {
        let mut sorted = self.late_ms.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
    }

    /// Spread of the hits around their own median: the timing a DJ hears as
    /// a loose drummer. A constant offset is not in it.
    fn worst_off_ms(&self) -> f64 {
        let median = self.median_late_ms();
        self.late_ms
            .iter()
            .map(|l| (l - median).abs())
            .fold(0.0, f64::max)
    }
}

fn differenced(signal: &[f32]) -> Vec<f32> {
    std::iter::once(0.0)
        .chain(signal.windows(2).map(|w| w[1] - w[0]))
        .collect()
}

fn hit_reading(out: &[f32], times: &[usize], rate: f64) -> HitReading {
    let shape = differenced(&hit_shape());
    let whole: f32 = shape.iter().map(|s| s * s).sum();
    let out = differenced(out);
    let reach = (0.040 * SR_F64) as isize;
    let mut reading = HitReading::default();
    for &time in times {
        let due = (time as f64 / rate).round() as isize;
        let strength_at = |lag: isize| -> f32 {
            let from = due + lag;
            if from < 0 || from as usize + shape.len() > out.len() {
                return 0.0;
            }
            let from = from as usize;
            shape
                .iter()
                .zip(&out[from..])
                .map(|(a, b)| a * b)
                .sum::<f32>()
                / whole
        };
        let strengths: Vec<f32> = (-reach..=reach).map(strength_at).collect();
        let (best, strength) = strengths
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .expect("a search range");
        let second = strengths
            .iter()
            .enumerate()
            .filter(|(lag, _)| lag.abs_diff(best) > 96)
            .map(|(_, s)| *s)
            .fold(0.0f32, f32::max);
        reading.hits += 1;
        reading.doubled += usize::from(second > 0.33);
        reading.faded += usize::from(strength < 0.6);
        reading
            .late_ms
            .push((best as isize - reach) as f64 * 1_000.0 / SR_F64);
    }
    reading
}

// ----------------------------------------------------------------- top end

/// Level of `out` against `source` between 12 and 16 kHz, in dB.
fn top_end_db(out: &[f32], source: &[f32]) -> f64 {
    band_db(out, source, 12_000, 16_000)
}

/// Level of `out` against `source` between 70 and 300 Hz, in dB: the
/// kick's body and the bass guitar.
///
/// A stretcher that joins bass and the rest separately plays the two a few
/// milliseconds apart for much of the time, and whatever both carry — the
/// range its splitting filter passes to both — partly cancels. Held notes
/// do not show it: every join of a pure tone is a whole number of its
/// waves, and the two stay in step. Noise shows the worst of it. The first
/// splice stretcher lost 0.3 to 1.7 dB below 170 Hz on four records, and
/// the DJ heard it before any test here did (2026-11-19).
fn bass_db(out: &[f32], source: &[f32]) -> f64 {
    band_db(out, source, 70, 300)
}

fn band_db(out: &[f32], source: &[f32], from_hz: usize, to_hz: usize) -> f64 {
    const N: usize = 16_384;
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(N);
    let band_power = |signal: &[f32]| -> f64 {
        let (mut total, mut windows) = (0.0f64, 0usize);
        let mut buf = fft.make_input_vec();
        let mut spectrum = fft.make_output_vec();
        for chunk in signal[N..signal.len() - N].chunks_exact(N) {
            for (i, (b, s)) in buf.iter_mut().zip(chunk).enumerate() {
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos();
                *b = s * w;
            }
            fft.process(&mut buf, &mut spectrum).expect("fft");
            let (lo, hi) = (from_hz * N / 48_000, to_hz * N / 48_000);
            total += spectrum[lo..hi]
                .iter()
                .map(|c| f64::from(c.norm_sqr()))
                .sum::<f64>();
            windows += 1;
        }
        total / windows as f64
    };
    10.0 * (band_power(out) / band_power(source)).log10()
}

// ------------------------------------------------------------------ report

struct Report {
    /// Worst held-note dip over the rates, per note, in dB.
    sub_dip_db: Vec<(f64, f64)>,
    /// Furthest a held note's pitch sat from the record's, in cents.
    sub_cents_off: f64,
    /// The same over the notes from 41 Hz (a bass guitar's low E) up.
    sub_cents_41_up: f64,
    doubled: usize,
    faded: usize,
    hits: usize,
    worst_hit_off_ms: f64,
    hit_offset_ms: f64,
    top_end_db: f64,
    /// Noise at 70–300 Hz against the record, at the rate that loses most.
    bass_db: f64,
}

fn report<S: TimeStretcher>(name: &str, make: impl Fn() -> S) -> Report {
    println!("\n== {name}");
    let mut out = Report {
        sub_dip_db: Vec::new(),
        sub_cents_off: 0.0,
        sub_cents_41_up: 0.0,
        doubled: 0,
        faded: 0,
        hits: 0,
        worst_hit_off_ms: 0.0,
        hit_offset_ms: 0.0,
        top_end_db: 0.0,
        bass_db: 0.0,
    };

    println!("held sub note, deepest dip in dB (cents off the record's pitch):");
    print!("   Hz ");
    for rate in RATES {
        print!("   x{rate:<10.2}");
    }
    println!();
    for hz in [33.0, 41.2, 49.0, 55.0, 65.4, 82.4] {
        print!("{hz:>5.1} ");
        let mut worst = 0.0f64;
        for rate in RATES {
            let got = key_locked(&mut make(), &sine(hz, 6.0, 0.6), rate);
            let reading = held_note_reading(&got, hz);
            print!(
                "  {:>5.1} ({:>+4.0} c)",
                reading.deepest_dip_db, reading.cents_off
            );
            out.sub_cents_off = if reading.cents_off.abs() > out.sub_cents_off.abs() {
                reading.cents_off
            } else {
                out.sub_cents_off
            };
            if hz > 41.0 && reading.cents_off.abs() > out.sub_cents_41_up.abs() {
                out.sub_cents_41_up = reading.cents_off;
            }
            worst = worst.max(reading.deepest_dip_db);
        }
        println!();
        out.sub_dip_db.push((hz, worst));
    }

    println!("drum hits over a bass note and a chord:");
    let (bed, times) = hits_over_a_bed(24.0);
    let mut offsets = Vec::new();
    for rate in RATES {
        let got = key_locked(&mut make(), &bed, rate);
        let reading = hit_reading(&got, &times, rate);
        println!(
            "  x{rate:.2}: {} hits, {} doubled, {} faded, {:+.1} ms offset, up to {:.1} ms off that",
            reading.hits,
            reading.doubled,
            reading.faded,
            reading.median_late_ms(),
            reading.worst_off_ms(),
        );
        out.hits += reading.hits;
        out.doubled += reading.doubled;
        out.faded += reading.faded;
        out.worst_hit_off_ms = out.worst_hit_off_ms.max(reading.worst_off_ms());
        offsets.push(reading.median_late_ms());
    }
    out.hit_offset_ms = offsets
        .iter()
        .copied()
        .fold(0.0, |a: f64, b| if b.abs() > a.abs() { b } else { a });

    let mut noise = Lcg(99);
    let hiss: Vec<f32> = (0..(6.0 * SR_F64) as usize)
        .map(|_| noise.next() * 0.3)
        .collect();
    let mut dullest = 0.0f64;
    print!("noise at 12–16 kHz against the record, dB:");
    for rate in RATES {
        let got = key_locked(&mut make(), &hiss, rate);
        let db = top_end_db(&got, &hiss);
        out.bass_db = out.bass_db.min(bass_db(&got, &hiss));
        print!("  x{rate:.2} {db:+.2}");
        dullest = dullest.min(db);
    }
    println!();
    out.top_end_db = dullest;
    println!(
        "noise at 70–300 Hz against the record, at its worst: {:+.2} dB",
        out.bass_db
    );
    out
}

#[test]
fn a_locked_record_keeps_its_bass_its_hits_and_its_top_end() {
    let read = report("splice", || SpliceStretcher::new(SR));

    for (hz, dip) in &read.sub_dip_db {
        assert!(*dip <= 1.0, "a held {hz} Hz note dips {dip:.1} dB");
    }
    // 33 Hz is the bottom of a five-string bass; its wave is as long as
    // the furthest a join may jump, so it is the hardest note to hold.
    assert!(
        read.sub_cents_off.abs() <= 10.0,
        "a held sub note sits {:.0} cents off the record's pitch",
        read.sub_cents_off
    );
    assert!(
        read.sub_cents_41_up.abs() <= 3.0,
        "a held note from 41 Hz up sits {:.0} cents off the record's pitch",
        read.sub_cents_41_up
    );

    assert_eq!(read.doubled, 0, "hits played twice, of {}", read.hits);
    assert_eq!(read.faded, 0, "hits under 60 % strength, of {}", read.hits);
    assert!(
        read.worst_hit_off_ms <= 3.0,
        "a hit lands {:.1} ms off its beat",
        read.worst_hit_off_ms
    );
    assert!(
        read.hit_offset_ms.abs() <= 1.5,
        "hits sit {:+.1} ms off the tempo as a whole",
        read.hit_offset_ms
    );

    assert!(
        // Noise is the worst a record can do here: nothing in it repeats,
        // so no join of it is ever in step. Music measured on the same day
        // lost under 0.2 dB.
        read.bass_db >= -0.8,
        "noise at 70–300 Hz comes out {:+.2} dB",
        read.bass_db
    );
    assert!(
        read.top_end_db >= -0.3,
        "noise at 12–16 kHz comes out {:+.2} dB",
        read.top_end_db
    );
}
