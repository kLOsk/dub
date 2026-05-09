//! Verifies dub-io can decode every M3 target format.
//!
//! Each fixture under `tests/fixtures/` is the same source signal — a
//! 0.5-second 440 Hz stereo tone at 44.1 kHz, amplitude 0.3 — re-encoded
//! into the listed format with `ffmpeg` (see `tests/fixtures/README.md`).
//!
//! What we assert per format:
//!
//! 1. Load succeeds (the format is recognized and decoded).
//! 2. Sample rate is preserved.
//! 3. Channel count is preserved.
//! 4. Frame count is *approximately* the source length (lossy codecs
//!    insert priming/padding samples, so we allow a generous margin).
//! 5. Peak amplitude is plausibly the source's amplitude (lossy codecs
//!    can attenuate or pre-ring slightly; tolerance widens accordingly).
//! 6. RMS energy is non-trivial (rules out "decoded to silence" failures
//!    that wouldn't trip lazy assertions).
//!
//! These are not bit-exact tests — that's the wrong bar for lossy codecs.
//! The bar is "this format opens, decodes to plausible audio, and the
//! engine can play it." Fingerprint/identity testing happens elsewhere.

use std::path::{Path, PathBuf};

use dub_io::Track;

const SOURCE_SR: u32 = 44_100;
const SOURCE_CH: u8 = 2;
const SOURCE_FRAMES: usize = 22_050;
const SOURCE_PEAK: f32 = 0.3;

fn fixture(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.join("tests").join("fixtures").join(name)
}

fn track_peak(track: &Track) -> f32 {
    track
        .samples()
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0f32, f32::max)
}

fn track_rms(track: &Track) -> f32 {
    let n = track.samples().len();
    if n == 0 {
        return 0.0;
    }
    let acc: f64 = track
        .samples()
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum();
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    {
        (acc / n as f64).sqrt() as f32
    }
}

/// Tolerance bands for one format. Lossless = tight, lossy = relaxed.
struct Bounds {
    /// Max difference in frame count from the source length.
    frame_slack: usize,
    /// Acceptable peak range (lossy can attenuate or pre-ring slightly).
    peak_min: f32,
    peak_max: f32,
    /// Minimum RMS — guards against "decodes to silence" regressions.
    rms_min: f32,
}

impl Bounds {
    /// Lossless reconstruction: frame-count exact, peak ≈ source.
    fn lossless() -> Self {
        Self {
            frame_slack: 0,
            peak_min: SOURCE_PEAK - 0.001,
            peak_max: SOURCE_PEAK + 0.001,
            rms_min: 0.20,
        }
    }

    /// Lossy reconstruction: priming/padding allowed, peak softer.
    fn lossy() -> Self {
        Self {
            // MP3/AAC may add up to ~2 frames of priming + arbitrary
            // tail padding. 4096 samples is plenty.
            frame_slack: 4_096,
            peak_min: 0.20,
            peak_max: 0.45,
            rms_min: 0.15,
        }
    }
}

fn assert_format(name: &str, bounds: &Bounds) {
    let path = fixture(name);
    let track =
        Track::load_from_path(&path).unwrap_or_else(|e| panic!("load {name} failed: {e:?}"));

    assert_eq!(
        track.sample_rate(),
        SOURCE_SR,
        "{name}: sample rate mismatch"
    );
    assert_eq!(track.channels(), SOURCE_CH, "{name}: channel mismatch");

    let dframe = track.frames().abs_diff(SOURCE_FRAMES);
    assert!(
        dframe <= bounds.frame_slack,
        "{name}: frame count {} too far from {} (allow ±{})",
        track.frames(),
        SOURCE_FRAMES,
        bounds.frame_slack,
    );

    let peak = track_peak(&track);
    assert!(
        peak >= bounds.peak_min && peak <= bounds.peak_max,
        "{name}: peak {peak} outside [{}, {}]",
        bounds.peak_min,
        bounds.peak_max,
    );

    let rms = track_rms(&track);
    assert!(
        rms >= bounds.rms_min,
        "{name}: rms {rms} below {}",
        bounds.rms_min,
    );
}

#[test]
fn wav_loads() {
    assert_format("tone.wav", &Bounds::lossless());
}

#[test]
fn aiff_loads() {
    assert_format("tone.aiff", &Bounds::lossless());
}

#[test]
fn flac_loads() {
    assert_format("tone.flac", &Bounds::lossless());
}

#[test]
fn mp3_loads() {
    assert_format("tone.mp3", &Bounds::lossy());
}

#[test]
fn aac_in_m4a_loads() {
    assert_format("tone-aac.m4a", &Bounds::lossy());
}

#[test]
fn alac_in_m4a_loads() {
    // ALAC is lossless but in an MP4 container that may report the
    // padded length. The codec itself produces bit-exact samples once
    // priming is consumed, but symphonia returns the full encoded
    // stream including priming, so we use lossy frame slack.
    assert_format(
        "tone-alac.m4a",
        &Bounds {
            frame_slack: 4_096,
            peak_min: SOURCE_PEAK - 0.001,
            peak_max: SOURCE_PEAK + 0.001,
            rms_min: 0.20,
        },
    );
}
