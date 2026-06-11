//! Regression tests for the systematic rate bias that **correlated**
//! low-frequency noise (mains hum, rumble, DC offset) used to induce in
//! the phase-difference decoder. White noise leaves the coherent-sum
//! angle unbiased, but a hum component is nearly identical
//! sample-to-sample, so it adds a real-positive vector to
//! `Σ s·conj(s_prev)` and shrinks the measured rate toward zero — the
//! on-rig "fader at 0 reads −0.1…−0.2 %" report. The decoder's input
//! high-pass (see `INPUT_HP_HZ` in `decoder.rs`) removes it; these
//! tests pin the residual bias near zero and guard the slow-scratch
//! band the filter must not break.

use dub_timecode::{signal::Generator, Decoder, Format};

const SR: f32 = 48_000.0;
const BLOCK: usize = 64;

/// Mean decoded rate over ~2 s of carrier at `rate`, with an additive
/// per-channel contaminant supplied per sample index. The first half
/// second is excluded so the high-pass and ellipse EMA settle.
fn mean_rate(rate: f64, contaminant: impl Fn(usize) -> (f32, f32)) -> f64 {
    let mut gen = Generator::new(Format::SeratoCv02, SR);
    let mut dec = Decoder::new(Format::SeratoCv02, SR);
    let mut buf = [0.0f32; BLOCK * 2];
    let blocks = (2.0 * SR as f64 / BLOCK as f64) as usize;
    let mut sample_idx = 0usize;
    let mut sum = 0.0f64;
    let mut counted = 0usize;
    for b in 0..blocks {
        gen.render(&mut buf, rate, 0.35);
        for frame in buf.chunks_exact_mut(2) {
            let (l, r) = contaminant(sample_idx);
            frame[0] += l;
            frame[1] += r;
            sample_idx += 1;
        }
        let out = dec.process(&buf);
        if b > blocks / 4 {
            sum += out.rate;
            counted += 1;
        }
    }
    sum / counted as f64
}

fn hum(level: f32, hz: f32) -> impl Fn(usize) -> (f32, f32) {
    move |i| {
        let ph = std::f32::consts::TAU * hz * i as f32 / SR;
        // In-phase on both channels, like real induced mains hum.
        (level * ph.sin(), level * ph.sin())
    }
}

#[test]
fn clean_carrier_has_no_bias() {
    let bias = mean_rate(1.0, |_| (0.0, 0.0)) - 1.0;
    println!("clean bias: {:+.5}%", bias * 100.0);
    assert!(bias.abs() < 1e-4, "clean carrier biased: {bias}");
}

#[test]
fn mains_hum_no_longer_biases_the_rate() {
    // 50 Hz hum 30 dB under the 0.35 carrier (≈ 0.011 amplitude).
    // Pre-fix this read ≈ −0.09 %.
    let bias = mean_rate(1.0, hum(0.011, 50.0)) - 1.0;
    println!("hum −30 dB bias: {:+.5}%", bias * 100.0);
    assert!(bias.abs() < 1e-4, "hum bias survived the high-pass: {bias}");
}

#[test]
fn rectifier_hum_at_120hz_stays_inside_tolerance() {
    // 120 Hz (US rectifier hum) sits in the 120 Hz cutoff's transition
    // band — the documented trade for keeping slow-scratch response
    // (see `INPUT_HP_HZ`): expect ~−0.04 % at −30 dB hum, vs −0.09 %
    // unfiltered. Pin it under 0.06 % so a cutoff regression is caught;
    // a notch / per-region cutoff is the follow-up if a 60 Hz-land rig
    // ever shows a real offset.
    let bias = mean_rate(1.0, hum(0.011, 120.0)) - 1.0;
    println!("120 Hz hum bias: {:+.5}%", bias * 100.0);
    assert!(bias.abs() < 6e-4, "120 Hz hum bias too large: {bias}");
}

#[test]
fn dc_offset_no_longer_biases_the_rate() {
    // Pre-fix a 0.012 per-channel DC offset read ≈ −0.22 %.
    let bias = mean_rate(1.0, |_| (0.012, 0.012)) - 1.0;
    println!("DC 0.012 bias: {:+.5}%", bias * 100.0);
    assert!(bias.abs() < 1e-4, "DC bias survived the high-pass: {bias}");
}

#[test]
fn rumble_no_longer_biases_the_rate() {
    // Turntable rumble: 25 Hz, 20 dB under the carrier — louder than
    // the hum cases because real rumble often is.
    let bias = mean_rate(1.0, hum(0.035, 25.0)) - 1.0;
    println!("rumble bias: {:+.5}%", bias * 100.0);
    assert!(bias.abs() < 1e-4, "rumble bias survived: {bias}");
}

#[test]
fn bias_stays_small_across_pitch_range() {
    // ±8 % is the classic Technics range; ±50 % covers wide-range
    // decks (RP-8000 / ST-150 class). At −50 % the CV02 carrier sits
    // at 500 Hz — still 2.5× above the input high-pass cutoff.
    for pitch in [-0.50f64, -0.08, 0.0, 0.08, 0.50] {
        let rate = 1.0 + pitch;
        let bias = mean_rate(rate, hum(0.011, 50.0)) - rate;
        println!("pitch {:+.0}%: bias {:+.5}%", pitch * 100.0, bias * 100.0);
        assert!(bias.abs() < 2e-4, "bias at pitch {pitch}: {bias}");
    }
}

#[test]
fn quiet_slow_draw_keeps_confidence() {
    // The on-rig "timecode didn't react" case: a cartridge is a
    // velocity sensor, so a rate-0.1 draw outputs ~10× less than unity
    // play (0.035 for a 0.35 rig) *and* its 100 Hz carrier sits in the
    // high-pass transition band. The decode must still report enough
    // confidence to keep the lift policy engaged — a gated block here
    // pauses the deck mid-gesture and manufactures sticker drift.
    let mut gen = Generator::new(Format::SeratoCv02, SR);
    let mut dec = Decoder::new(Format::SeratoCv02, SR);
    let mut buf = [0.0f32; BLOCK * 2];
    let blocks = (1.0 * SR as f64 / BLOCK as f64) as usize;
    let mut conf_min = 1.0f32;
    for b in 0..blocks {
        gen.render(&mut buf, 0.1, 0.035);
        let out = dec.process(&buf);
        if b > blocks / 4 {
            conf_min = conf_min.min(out.confidence);
        }
    }
    println!("quiet slow draw: min conf {conf_min:.3}");
    assert!(
        conf_min > 0.7,
        "quiet slow draw fell under the presence gate: {conf_min}"
    );
}

#[test]
fn slow_scratch_still_decodes_through_the_high_pass() {
    // rate 0.12 puts the CV02 carrier at 120 Hz — inside the filter's
    // transition band. Amplitude drops but the decode must stay usable:
    // rate within a few percent and confidence above the lift policy's
    // engage band.
    let mut gen = Generator::new(Format::SeratoCv02, SR);
    let mut dec = Decoder::new(Format::SeratoCv02, SR);
    let mut buf = [0.0f32; BLOCK * 2];
    let blocks = (2.0 * SR as f64 / BLOCK as f64) as usize;
    let (mut rate_sum, mut conf_min, mut counted) = (0.0f64, 1.0f32, 0usize);
    for b in 0..blocks {
        gen.render(&mut buf, 0.12, 0.35);
        let out = dec.process(&buf);
        if b > blocks / 4 {
            rate_sum += out.rate;
            conf_min = conf_min.min(out.confidence);
            counted += 1;
        }
    }
    let mean = rate_sum / counted as f64;
    println!("slow scratch: mean rate {mean:.4}, min conf {conf_min:.3}");
    assert!((mean - 0.12).abs() < 0.005, "slow rate off: {mean}");
    assert!(
        conf_min > 0.6,
        "slow-scratch confidence collapsed: {conf_min}"
    );
}
