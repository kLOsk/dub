//! Musical-key detection (M11c.2).
//!
//! Per PRD §8.3.2. Given mono / stereo audio, returns a [`KeyEstimate`]
//! carrying the detected tonic + mode in canonical Camelot notation.
//! Pure-Rust on top of [`crate::SpectralFrameStream`]; no FFI; no
//! licensing encumbrance (Krumhansl-Kessler 1982 templates are
//! music-theory public-domain coefficients, not a third-party
//! library).
//!
//! ## Pipeline
//!
//! 1. **Downmix to mono** if the buffer is stereo. The chroma profile
//!    of L + R averaged is the same up to magnitude as either channel
//!    alone for normally-mixed material; combining halves the FFT
//!    cost and stabilises the chroma against hard-panned synths that
//!    would otherwise pull the profile sideways.
//! 2. **Key-specific STFT.** A 4096-point Hann-windowed FFT with
//!    1024-sample hop, separate from [`crate::SpectralFrameStream`]'s
//!    1024-point analysis-frame chain. Same `realfft` + Hann +
//!    `ln(1 + λ · |X|)` magnitude transform — just a longer window.
//!    The BPM pipeline can afford 1024 because it integrates across
//!    log-spaced bands; chroma cannot — at 44.1 kHz, a 1024-point
//!    FFT produces 43 Hz / bin, but a semitone at E4 (329 Hz) is
//!    only 19 Hz wide. With 1024-point bins, E4's spectral energy
//!    falls mostly into bins centred on D and F. A 4096-point FFT
//!    (~10.8 Hz / bin) puts 1.5–3 bins per semitone across the
//!    `[MIN_KEY_HZ, MAX_KEY_HZ]` band — enough for stable chroma.
//! 3. **Chroma extraction.** For each frame and each FFT bin in the
//!    tonal band, compute the *fractional* chromaticity of the bin
//!    centre and distribute the bin's compressed magnitude into the
//!    two neighbouring pitch classes by linear interpolation. Two
//!    consequences: (a) bins that sit on the boundary between two
//!    semitones don't get rounded entirely into one PC and lost
//!    from the other; (b) the chroma profile degrades gracefully
//!    as the FFT-resolution-vs-semitone ratio worsens at higher
//!    frequencies, instead of cliffing.
//! 4. **Frame-energy weighting.** Frames whose total tonal-band
//!    energy is below the median get zero weight. This drops silent
//!    intros / outros and unvoiced percussion-only sections (the
//!    BPM analyser handles those; they carry no harmonic content).
//! 5. **Time-average + L1 normalise** the chroma vector across the
//!    track.
//! 6. **24-template correlation.** For each of the 12 major + 12
//!    minor Krumhansl-Kessler 1982 profiles, rotated by the
//!    candidate tonic, compute the Pearson correlation against the
//!    observed chroma. Pick the argmax.
//! 7. **Confidence = gap between best and second-best correlation,
//!    normalised against the dynamic range.** A value of 0 means
//!    "the template fit is ambiguous; ignore this estimate";
//!    0.3+ is a strong fit.
//!
//! ## Honesty contract
//!
//! Identical to `dub-bpm`'s. An audio buffer that doesn't support a
//! statistically meaningful chroma profile (silence, < 5 s of audio,
//! pure-noise input, single-pitch test tone) returns
//! `Ok(KeyEstimate::none())` — i.e. confidence 0 — rather than an
//! arbitrary guess. The caller treats confidence-0 as "no key
//! detected" and stamps the analysis as complete-but-empty.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use thiserror::Error;

/// Reference frequency for A4 — the equal-temperament anchor.
pub const A4_HZ: f32 = 440.0;

/// Reference C used for the chromaticity formula. Choosing C rather
/// than A means `chromaticity(C_n) = 0` for every octave, so the
/// integer part of the chromaticity is directly the pitch class.
/// `C5 = 523.25 Hz` is well inside the analysis band; any pitch
/// class C choice would do — the formula `(12 * log2(f / C)) mod 12`
/// is octave-invariant.
const C_REF_HZ: f32 = 523.25;

/// FFT frame size for chroma extraction, in samples. 4096 was
/// chosen so the per-bin width at 44.1 kHz (~10.8 Hz) sits
/// comfortably under a semitone at the lowest analysis frequency
/// (~15 Hz at C2 → 1.5 bins / semitone). At 48 kHz the ratio is
/// ~12 Hz / bin, still well under any semitone in the analysis
/// band. Larger frames buy nothing — chroma is integrated
/// across all frames, so time resolution is irrelevant; smaller
/// frames break the binning. See module-level docs.
const KEY_FRAME_SIZE: usize = 4096;

/// Hop between consecutive analysis frames, in samples. 1024 =
/// 75 % overlap, matching the `SpectralFrameStream` overlap ratio
/// (50 %) once the larger frame is accounted for. The chroma
/// pipeline cares about steady-state averaging, not transient
/// localisation, so a longer hop would work — but 75 % overlap
/// suppresses Hann-window scalloping on near-edge tonal content.
const KEY_HOP_SIZE: usize = 1024;

// The chroma pipeline deliberately does **not** use the BPM
// analyser's `ln(1 + λ · |X|)` compression. That compression
// flattens the spectral envelope (mid-amplitude leakage bins end
// up almost as loud as the tonal peak) which is exactly what the
// onset detector wants — but the *opposite* of what chroma
// extraction wants. A pure C4 tone produces a 4-bin Hann main
// lobe; under log-compression the leakage bin sitting on PC 11
// (B, one semitone below C) carries ~80 % of the peak bin's
// compressed magnitude, drowning the chroma profile in
// neighbouring-semitone bleed. Power (|X|²) sharpens the peak
// (the leakage bin drops to ~25 %), which is what we need to
// identify the tonal fundamental cleanly.

/// Lower edge of the tonal band, in Hz. ~ C2; below this lies sub-
/// bass / kick-drum fundamental, which carries no melodic content
/// and would bias every track towards `C major` (the strongest bin
/// is always at the kick).
pub const MIN_KEY_HZ: f32 = 65.0;

/// Upper edge of the tonal band, in Hz. ~ C8; above this lies
/// hi-hat / cymbal shimmer where pitch-class binning is unreliable
/// (the third harmonic of an 8 kHz cymbal lands in a totally
/// different pitch class than the fundamental).
pub const MAX_KEY_HZ: f32 = 4_186.0;

/// Minimum analysis window, in seconds. Shorter buffers don't carry
/// enough harmonic statistics for a stable chroma profile. 5 s is
/// the empirical floor below which Krumhansl-Kessler correlations
/// stop separating major from minor reliably.
pub const MIN_ANALYSIS_SECS: f64 = 5.0;

/// Krumhansl-Kessler (1982) major-key probe-tone profile, rooted at
/// PC 0 (= C major). Rotating these 12 values by the candidate
/// tonic gives the template for that key. These coefficients are
/// well-established music-theory data, not a third-party algorithm:
/// they are the average response listeners gave when asked how
/// well each of 12 chromatic pitches fits a major-key context.
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];

/// Krumhansl-Kessler (1982) minor-key probe-tone profile, rooted at
/// PC 0 (= C minor). Same source as [`MAJOR_PROFILE`].
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// Pitch class → Camelot major position. `PC 0` (C major) → `"8B"`;
/// the Camelot wheel walks fifths clockwise (8B → 9B → 10B → … →
/// 12B → 1B → … → 7B → 8B). Used in [`KeyEstimate::camelot`].
const CAMELOT_MAJOR: [&str; 12] = [
    "8B",  // C major
    "3B",  // C# / D♭ major
    "10B", // D major
    "5B",  // D# / E♭ major
    "12B", // E major
    "7B",  // F major
    "2B",  // F# / G♭ major
    "9B",  // G major
    "4B",  // G# / A♭ major
    "11B", // A major
    "6B",  // A# / B♭ major
    "1B",  // B major
];

/// Pitch class → Camelot minor position. The relative-minor of any
/// Camelot major position shares its number (so C major = 8B and
/// A minor = 8A). Both arrays computed independently so a typo in
/// one doesn't silently corrupt the other.
const CAMELOT_MINOR: [&str; 12] = [
    "5A",  // C minor
    "12A", // C# / D♭ minor
    "7A",  // D minor
    "2A",  // D# / E♭ minor
    "9A",  // E minor
    "4A",  // F minor
    "11A", // F# / G♭ minor
    "6A",  // G minor
    "1A",  // G# / A♭ minor
    "8A",  // A minor
    "3A",  // A# / B♭ minor
    "10A", // B minor
];

/// Pitch-class names for the musical-notation rendering path.
/// `♯` (U+266F) rather than ASCII `#` so the browser's typography
/// renders correctly without per-line escaping.
const PC_NAMES: [&str; 12] = [
    "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
];

/// All failures `analyze_key` can surface to its caller. Same
/// shape as `dub-bpm::AnalysisError` so the library-side
/// `analyze_track` pipeline can map both into one decode-failure
/// variant without per-DSP-crate plumbing.
#[derive(Debug, Error, PartialEq)]
pub enum KeyAnalysisError {
    /// Caller passed `sample_rate = 0`. Always a programmer error.
    #[error("sample rate must be > 0")]
    ZeroSampleRate,
    /// Caller passed a channel count we don't support. v1 ships
    /// mono + stereo only (matches `dub-bpm`).
    #[error("channels must be 1 (mono) or 2 (stereo); got {0}")]
    InvalidChannels(u8),
    /// `samples.len()` is not divisible by `channels`. The caller
    /// has a non-interleaved buffer; if we trust the channel count
    /// and divide blindly, we silently drop the trailing partial
    /// frame and the estimate disagrees with every other tool that
    /// reads the same file. Refuse and surface the mismatch.
    #[error("non-interleaved buffer: {sample_count} samples / {channels} channels has remainder")]
    NonInterleavedFrames {
        /// Total length of the supplied buffer.
        sample_count: usize,
        /// Channel count the caller declared.
        channels: u8,
    },
    /// Audio buffer is shorter than `MIN_ANALYSIS_SECS`. Reported
    /// as an error rather than a zero-confidence estimate because
    /// the input simply can't support the algorithm.
    #[error(
        "audio too short for key analysis: got {got_frames} frames at {sample_rate} Hz, \
         need at least {need_frames} ({need_secs:.1} s)"
    )]
    TooShort {
        /// Actual frame count.
        got_frames: usize,
        /// Frame floor at the supplied sample rate.
        need_frames: usize,
        /// Frame floor in seconds.
        need_secs: f64,
        /// Sample rate the floor was computed for.
        sample_rate: u32,
    },
}

/// A single key estimate. `confidence == 0.0` means "no key detected"
/// — `tonic_pc` / `is_major` are then arbitrary; callers must check
/// the confidence before using either.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyEstimate {
    /// Tonic as a chromatic pitch class. `0 = C`, `1 = C♯`, …,
    /// `11 = B`. Meaningful iff `confidence > 0.0`.
    pub tonic_pc: u8,
    /// `true` for a major key, `false` for minor. Meaningful iff
    /// `confidence > 0.0`.
    pub is_major: bool,
    /// Algorithm confidence in `[0.0, 1.0]`. Computed as the gap
    /// between the best and second-best template correlation,
    /// normalised against the worst correlation so the scale stays
    /// independent of the audio's overall energy. `0.0` = the top
    /// two keys are indistinguishable; treat as "no key".
    pub confidence: f32,
}

impl KeyEstimate {
    /// A null estimate. Returned when the analyser can't separate
    /// major from minor, when the chroma profile is degenerate
    /// (silence, single tone), or — per [`analyze_key`]'s honesty
    /// contract — when the input is non-musical.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            tonic_pc: 0,
            is_major: true,
            confidence: 0.0,
        }
    }

    /// Canonical Camelot notation for this key, e.g. `"8B"` for
    /// `C major`. Result is `""` for [`KeyEstimate::none`]; callers
    /// should not call this on a zero-confidence estimate.
    #[must_use]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn camelot(&self) -> &'static str {
        if self.confidence <= 0.0 {
            return "";
        }
        let pc = (self.tonic_pc % 12) as usize;
        if self.is_major {
            CAMELOT_MAJOR[pc]
        } else {
            CAMELOT_MINOR[pc]
        }
    }

    /// Musical-notation rendering, e.g. `"C major"` / `"A minor"`.
    /// Returned as an owned `String` because we concatenate the
    /// pitch-class name with the mode suffix. Empty string for
    /// the zero-confidence case.
    #[must_use]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn musical(&self) -> String {
        if self.confidence <= 0.0 {
            return String::new();
        }
        let pc = (self.tonic_pc % 12) as usize;
        let mode = if self.is_major { "major" } else { "minor" };
        format!("{} {}", PC_NAMES[pc], mode)
    }
}

/// Parse a Camelot string (e.g. `"8B"`, `"12a"`) into `(number,
/// is_major)`. Case-insensitive on the letter. Returns `None` for
/// any malformed input (empty, missing letter, out-of-range number,
/// non-`A|B` mode letter). Lives here so the `dub-library` cross-
/// validation logic can call it without duplicating the parse.
#[must_use]
pub fn parse_camelot(s: &str) -> Option<(u8, bool)> {
    let s = s.trim();
    if s.len() < 2 || s.len() > 3 {
        return None;
    }
    let last = s.as_bytes()[s.len() - 1];
    let is_major = match last {
        b'B' | b'b' => true,
        b'A' | b'a' => false,
        _ => return None,
    };
    let num_part = &s[..s.len() - 1];
    let num: u8 = num_part.parse().ok()?;
    if !(1..=12).contains(&num) {
        return None;
    }
    Some((num, is_major))
}

/// PRD §8.3.2 relative-major-aware key-disagreement predicate. Two
/// Camelot keys are considered to disagree iff they have different
/// numbers, regardless of the A/B suffix. Relative-major pairs (C
/// major = 8B vs A minor = 8A) share the number and are tolerated:
/// they're a legitimate Krumhansl-Kessler template ambiguity and
/// firing the ⚠ indicator on them would be noise. Parallel pairs
/// (C major = 8B vs C minor = 5A) have different numbers and
/// surface.
///
/// `None` for either side (or a malformed Camelot string) returns
/// `false` — we don't flag a disagreement we can't confirm.
#[must_use]
pub fn camelot_keys_disagree(a: &str, b: &str) -> bool {
    let (Some((na, _)), Some((nb, _))) = (parse_camelot(a), parse_camelot(b)) else {
        return false;
    };
    na != nb
}

/// Analyse a buffer and return its musical key.
///
/// `samples` is interleaved (`L R L R …` for stereo). Stereo is
/// downmixed to mono internally.
///
/// # Errors
///
/// See [`KeyAnalysisError`]. A successfully analysed but non-
/// musical input returns `Ok(KeyEstimate::none())`, not an error.
#[allow(clippy::too_many_lines)]
pub fn analyze_key(
    samples: &[f32],
    sample_rate: u32,
    channels: u8,
) -> Result<KeyEstimate, KeyAnalysisError> {
    if sample_rate == 0 {
        return Err(KeyAnalysisError::ZeroSampleRate);
    }
    if !(1..=2).contains(&channels) {
        return Err(KeyAnalysisError::InvalidChannels(channels));
    }
    if !samples.len().is_multiple_of(usize::from(channels)) {
        return Err(KeyAnalysisError::NonInterleavedFrames {
            sample_count: samples.len(),
            channels,
        });
    }

    let frames = samples.len() / usize::from(channels);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let need_frames = (MIN_ANALYSIS_SECS * f64::from(sample_rate)).ceil() as usize + KEY_FRAME_SIZE;
    if frames < need_frames {
        return Ok(KeyEstimate::none());
    }

    let mono: &[f32];
    let downmixed_storage: Vec<f32>;
    if channels == 1 {
        mono = samples;
    } else {
        downmixed_storage = samples
            .chunks_exact(2)
            .map(|c| 0.5 * (c[0] + c[1]))
            .collect();
        mono = &downmixed_storage;
    }

    // First pass: stream the mono buffer through the chroma STFT
    // once. For each frame, compute per-frame total tonal energy
    // and the per-pitch-class chroma; record both. We can't combine
    // "drop quiet frames" and "accumulate chroma" into a single pass
    // because the energy threshold is the median across all frames,
    // which we don't know until the pass finishes.
    let bin_to_chroma = build_bin_chroma_table(sample_rate);
    let mut stft = ChromaStft::new();

    let mut frame_energies: Vec<f32> = Vec::new();
    let mut frame_chromas: Vec<[f32; 12]> = Vec::new();
    stft.process(mono, |mags| {
        let mut chroma = [0.0_f32; 12];
        let mut energy = 0.0_f32;
        for (bin, &slot) in bin_to_chroma.iter().enumerate() {
            let Some(BinChroma {
                lower_pc,
                upper_pc,
                lower_weight,
            }) = slot
            else {
                continue;
            };
            let m = mags[bin];
            chroma[lower_pc as usize] += m * lower_weight;
            chroma[upper_pc as usize] += m * (1.0 - lower_weight);
            energy += m;
        }
        frame_energies.push(energy);
        frame_chromas.push(chroma);
    });

    if frame_chromas.is_empty() {
        return Ok(KeyEstimate::none());
    }

    // Compute the energy gate as the median frame energy. Frames
    // below the gate (silent intros, breakdowns, pauses) contribute
    // nothing to the aggregate chroma. Median is robust against
    // outlier-loud frames (a single hot kick on an otherwise quiet
    // section) without the extra plumbing a percentile would need.
    let median = median_f32(&frame_energies);

    let mut total = [0.0_f32; 12];
    let mut weight_sum = 0.0_f32;
    for (chroma, &energy) in frame_chromas.iter().zip(frame_energies.iter()) {
        if energy < median || energy <= 0.0 {
            continue;
        }
        for k in 0..12 {
            total[k] += chroma[k];
        }
        weight_sum += energy;
    }

    if weight_sum <= 0.0 {
        return Ok(KeyEstimate::none());
    }

    // L1-normalise the aggregate chroma so the correlation against
    // the templates is scale-invariant. Without this the
    // correlation magnitude is dominated by overall track loudness
    // and confidence numbers stop comparing across tracks.
    let chroma_sum: f32 = total.iter().sum();
    if chroma_sum <= 0.0 {
        return Ok(KeyEstimate::none());
    }
    for v in &mut total {
        *v /= chroma_sum;
    }

    // 24-template correlation. For each candidate (tonic, mode),
    // rotate the appropriate Krumhansl-Kessler profile by the
    // tonic pitch class and compute the Pearson correlation
    // against the observed normalised chroma.
    let mut best: (f32, u8, bool) = (f32::NEG_INFINITY, 0, true);
    let mut second_best: f32 = f32::NEG_INFINITY;
    let mut worst: f32 = f32::INFINITY;
    for is_major in [true, false] {
        let template = if is_major {
            &MAJOR_PROFILE
        } else {
            &MINOR_PROFILE
        };
        for tonic in 0u8..12 {
            let mut rotated = [0.0_f32; 12];
            for k in 0..12 {
                rotated[k] = template[(k + 12 - tonic as usize) % 12];
            }
            let r = pearson_12(&total, &rotated);
            if r > best.0 {
                second_best = best.0;
                best = (r, tonic, is_major);
            } else if r > second_best {
                second_best = r;
            }
            if r < worst {
                worst = r;
            }
        }
    }

    // Confidence: gap between the top two correlations, normalised
    // against the dynamic range observed across all 24 templates.
    // Stays in `[0, 1]` independent of overall track energy. The
    // 0.0 lower bound also catches the "two ties at the top"
    // degenerate case.
    let range = best.0 - worst;
    let confidence = if range > 1e-6 {
        ((best.0 - second_best) / range).clamp(0.0, 1.0)
    } else {
        0.0
    };

    Ok(KeyEstimate {
        tonic_pc: best.1,
        is_major: best.2,
        confidence,
    })
}

/// One bin's contribution to the chroma vector. Each FFT bin
/// contributes to two adjacent pitch classes, weighted by its
/// fractional chromaticity: a bin sitting exactly on PC `n` puts
/// all of its magnitude on PC `n`; a bin halfway between PC `n`
/// and PC `n+1` splits 50/50. This avoids the "bin lands in the
/// gap between two semitones" failure mode that a round-to-nearest
/// rule has at moderate FFT resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BinChroma {
    lower_pc: u8,
    upper_pc: u8,
    /// Weight on `lower_pc`. The upper PC gets `1.0 - lower_weight`.
    /// Always in `[0.0, 1.0]`.
    lower_weight: f32,
}

/// Build the per-bin chroma-distribution table for `sample_rate`.
/// Bins outside `[MIN_KEY_HZ, MAX_KEY_HZ]` (sub-bass, supra-cymbal)
/// map to `None`; bins inside carry a `Some(BinChroma)` describing
/// which two pitch classes share their magnitude and in what ratio.
/// Hoisted out of the per-frame loop because the mapping is
/// constant for the entire analysis pass.
fn build_bin_chroma_table(sample_rate: u32) -> Vec<Option<BinChroma>> {
    let half_spectrum = KEY_FRAME_SIZE / 2 + 1;
    let mut table = Vec::with_capacity(half_spectrum);
    #[allow(clippy::cast_precision_loss)]
    let bin_hz = sample_rate as f32 / KEY_FRAME_SIZE as f32;
    for b in 0..half_spectrum {
        #[allow(clippy::cast_precision_loss)]
        let freq = b as f32 * bin_hz;
        if !(MIN_KEY_HZ..=MAX_KEY_HZ).contains(&freq) {
            table.push(None);
            continue;
        }
        // Chromaticity in `[0, 12)`: octave-invariant pitch-class
        // coordinate. `C` family (any C_n) maps to 0; A → 9; B → 11.
        // Decompose into integer + fractional parts: integer = nearest
        // lower PC, fractional = how far past the lower PC the bin
        // sits. The fractional part is the weight on the *upper* PC.
        let chroma_idx = (12.0 * (freq / C_REF_HZ).log2()).rem_euclid(12.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lower_pc = chroma_idx.floor() as u8;
        let frac_upper = chroma_idx - f32::from(lower_pc);
        let upper_pc = (lower_pc + 1) % 12;
        table.push(Some(BinChroma {
            lower_pc,
            upper_pc,
            lower_weight: 1.0 - frac_upper,
        }));
    }
    table
}

/// Dedicated chroma STFT pipeline. Same shape as
/// [`crate::SpectralFrameStream`] but with a 4096-point window and
/// 1024-sample hop (see module-level docs for the resolution
/// rationale).
struct ChromaStft {
    r2c: Arc<dyn RealToComplex<f32>>,
    fft_in: Vec<f32>,
    fft_out: Vec<Complex<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    window: Vec<f32>,
    input_buffer: Vec<f32>,
    input_read_pos: usize,
    /// Per-bin power spectrum `|X[b]|²` for the current frame.
    /// Power (squared magnitude) rather than `ln(1 + λ · |X|)`:
    /// see module-level comment for why log-compression is wrong
    /// for chroma.
    power_mags: Vec<f32>,
}

impl ChromaStft {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(KEY_FRAME_SIZE);
        let fft_in = r2c.make_input_vec();
        let fft_out = r2c.make_output_vec();
        let fft_scratch = r2c.make_scratch_vec();

        #[allow(clippy::cast_precision_loss)]
        let nf = KEY_FRAME_SIZE as f32;
        let window: Vec<f32> = (0..KEY_FRAME_SIZE)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let phase = std::f32::consts::TAU * (i as f32) / nf;
                0.5 * (1.0 - phase.cos())
            })
            .collect();

        Self {
            r2c,
            fft_in,
            fft_out,
            fft_scratch,
            window,
            input_buffer: Vec::with_capacity(KEY_FRAME_SIZE * 4),
            input_read_pos: 0,
            power_mags: vec![0.0; KEY_FRAME_SIZE / 2 + 1],
        }
    }

    fn process<F>(&mut self, block: &[f32], mut on_frame: F)
    where
        F: FnMut(&[f32]),
    {
        self.input_buffer.extend_from_slice(block);
        while self.input_buffer.len() - self.input_read_pos >= KEY_FRAME_SIZE {
            let start = self.input_read_pos;
            for i in 0..KEY_FRAME_SIZE {
                self.fft_in[i] = self.input_buffer[start + i] * self.window[i];
            }
            self.r2c
                .process_with_scratch(&mut self.fft_in, &mut self.fft_out, &mut self.fft_scratch)
                .expect("FFT can't fail on correctly-sized buffers");
            for (b, slot) in self.power_mags.iter_mut().enumerate() {
                *slot = self.fft_out[b].norm_sqr();
            }
            on_frame(&self.power_mags);
            self.input_read_pos += KEY_HOP_SIZE;
        }
        // Compact the unread tail back to the front to keep the
        // buffer bounded across calls. Cheap — at most KEY_FRAME_SIZE
        // - 1 samples move.
        if self.input_read_pos > 0 {
            let remaining = self.input_buffer.len() - self.input_read_pos;
            if remaining > 0 {
                self.input_buffer.copy_within(self.input_read_pos.., 0);
            }
            self.input_buffer.truncate(remaining);
            self.input_read_pos = 0;
        }
    }
}

/// Pearson correlation between two length-12 vectors.
fn pearson_12(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let mean_a: f32 = a.iter().sum::<f32>() / 12.0;
    let mean_b: f32 = b.iter().sum::<f32>() / 12.0;
    let mut num = 0.0_f32;
    let mut den_a = 0.0_f32;
    let mut den_b = 0.0_f32;
    for k in 0..12 {
        let da = a[k] - mean_a;
        let db = b[k] - mean_b;
        num += da * db;
        den_a += da * da;
        den_b += db * db;
    }
    let den = (den_a * den_b).sqrt();
    if den <= 1e-12 {
        0.0
    } else {
        num / den
    }
}

/// Median of a `f32` slice. `NaN`-free input assumed (the chroma
/// pipeline can't produce `NaN`s; squared magnitudes of finite
/// floats stay finite). Clones the input because `select_nth`
/// requires `&mut`; the buffer is small (~frame-count) so the
/// clone is cheap.
fn median_f32(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    let mid = v.len() / 2;
    v.select_nth_unstable_by(mid, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    v[mid]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// Synthesise a sustained sine wave at `freq_hz` for `secs`.
    /// Pure tone → its chroma profile is dominated by the one
    /// pitch class the tone falls into.
    fn sine(freq_hz: f32, secs: f32, sample_rate: u32) -> Vec<f32> {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation
        )]
        let n = (secs * sample_rate as f32) as usize;
        #[allow(clippy::cast_precision_loss)]
        let dt = 1.0 / sample_rate as f32;
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 * dt;
                (TAU * freq_hz * t).sin() * 0.5
            })
            .collect()
    }

    /// Synthesise a chord triad (root + third + fifth) for `secs`
    /// at `sample_rate`. `third_semitones` distinguishes major
    /// (4 semitones) from minor (3 semitones).
    fn triad(root_hz: f32, third_semitones: f32, secs: f32, sample_rate: u32) -> Vec<f32> {
        let s1 = sine(root_hz, secs, sample_rate);
        let s2 = sine(
            root_hz * 2.0_f32.powf(third_semitones / 12.0),
            secs,
            sample_rate,
        );
        let s3 = sine(root_hz * 2.0_f32.powf(7.0 / 12.0), secs, sample_rate);
        s1.iter()
            .zip(s2.iter())
            .zip(s3.iter())
            .map(|((&a, &b), &c)| (a + b + c) / 3.0)
            .collect()
    }

    /// I-IV-V-I major-key chord progression in `root_hz`'s key.
    /// Total length `secs`; each chord one quarter. Sweeps the
    /// full diatonic set of the key — what Krumhansl-Kessler
    /// templates need to disambiguate the key from its relative
    /// minor. A pure-triad fixture cannot do this (its pitch-
    /// class set is shared with the relative minor up to template
    /// weighting), so unit tests targeting "this key" use this
    /// helper.
    fn major_i_iv_v_i(root_hz: f32, secs: f32, sample_rate: u32) -> Vec<f32> {
        let quarter = secs / 4.0;
        let i = triad(root_hz, 4.0, quarter, sample_rate);
        let iv = triad(
            root_hz * 2.0_f32.powf(5.0 / 12.0),
            4.0,
            quarter,
            sample_rate,
        );
        let v = triad(
            root_hz * 2.0_f32.powf(7.0 / 12.0),
            4.0,
            quarter,
            sample_rate,
        );
        let mut out = Vec::with_capacity(i.len() * 4);
        out.extend_from_slice(&i);
        out.extend_from_slice(&iv);
        out.extend_from_slice(&v);
        out.extend_from_slice(&i);
        out
    }

    #[test]
    fn zero_sample_rate_rejected() {
        assert_eq!(
            analyze_key(&[0.0; 1000], 0, 1),
            Err(KeyAnalysisError::ZeroSampleRate)
        );
    }

    #[test]
    fn invalid_channels_rejected() {
        assert_eq!(
            analyze_key(&[0.0; 1000], 48_000, 0),
            Err(KeyAnalysisError::InvalidChannels(0))
        );
        assert_eq!(
            analyze_key(&[0.0; 1000], 48_000, 3),
            Err(KeyAnalysisError::InvalidChannels(3))
        );
    }

    #[test]
    fn non_interleaved_stereo_rejected() {
        // 1001 samples / 2 channels = 500 remainder 1 → reject.
        assert!(matches!(
            analyze_key(&[0.0; 1001], 48_000, 2),
            Err(KeyAnalysisError::NonInterleavedFrames { .. })
        ));
    }

    #[test]
    fn too_short_returns_zero_confidence() {
        // 1 second of audio at 48 kHz is well under the 5 s floor;
        // analyse but report `none()`.
        let est = analyze_key(&sine(440.0, 1.0, 48_000), 48_000, 1).unwrap();
        assert!(est.confidence == 0.0, "got confidence {}", est.confidence);
    }

    #[test]
    fn silence_returns_zero_confidence() {
        let buf = vec![0.0_f32; 48_000 * 6];
        let est = analyze_key(&buf, 48_000, 1).unwrap();
        assert!(est.confidence == 0.0, "got confidence {}", est.confidence);
    }

    #[test]
    fn pure_c_major_triad_classifies_into_a_diatonic_family() {
        // Pure-triad inputs are an algorithmically unsolvable
        // case for Krumhansl-Kessler: the C-E-G chroma set is
        // consistent with C major (8B), its relative A minor (8A)
        // — the documented relative-key ambiguity PRD §8.3.2
        // exempts — and also with E minor (9A) which shares the
        // same set up to template weighting. Real-world music
        // carries passing tones and IV / V chords that break the
        // ambiguity; this unit-level case verifies only that the
        // analyser lands on a *diatonically related* key, not a
        // wildly distant one. The progression-based tests below
        // exercise the tonic-correct path.
        let sr = 48_000;
        let audio = triad(261.63, 4.0, 8.0, sr);
        let est = analyze_key(&audio, sr, 1).unwrap();
        assert!(est.confidence > 0.0);
        let (num, _) = parse_camelot(est.camelot()).expect("valid camelot for confident estimate");
        assert!(
            num == 8 || num == 9,
            "C-E-G triad must land in Camelot family 8 (C maj / A min) \
             or family 9 (G maj / E min); got {}",
            est.camelot()
        );
    }

    #[test]
    fn c_major_progression_recovers_camelot_8b() {
        // I-IV-V-I in C major sweeps the entire C-major
        // diatonic set (C-D-E-F-G-A-B). With every note of the
        // scale represented and the C tonic doubled by the I and
        // IV chords, Krumhansl-Kessler resolves cleanly to C
        // major. This is the bar a real-world key analyser must
        // clear, and the fixture is the minimum-viable musical
        // material that can support that bar.
        let sr = 48_000;
        let audio = major_i_iv_v_i(261.63, 12.0, sr);
        let est = analyze_key(&audio, sr, 1).unwrap();
        assert!(
            est.confidence > 0.0,
            "C major progression must yield confident estimate"
        );
        assert_eq!(
            est.tonic_pc,
            0,
            "tonic must be C (got PC {}, Camelot {})",
            est.tonic_pc,
            est.camelot()
        );
        assert!(
            est.is_major,
            "must classify as major (got Camelot {})",
            est.camelot()
        );
        assert_eq!(est.camelot(), "8B");
    }

    #[test]
    fn g_major_progression_recovers_camelot_9b() {
        // G major I-IV-V-I (G-C-D-G). Different tonic, different
        // diatonic set — proves the algorithm tracks the actual
        // key rather than always landing on C. Camelot 9B = G
        // major.
        let sr = 48_000;
        let audio = major_i_iv_v_i(392.0, 12.0, sr);
        let est = analyze_key(&audio, sr, 1).unwrap();
        assert!(est.confidence > 0.0);
        assert_eq!(
            est.camelot(),
            "9B",
            "G major progression must classify as 9B (got {})",
            est.camelot()
        );
    }

    #[test]
    fn stereo_input_is_downmixed() {
        // Stereo of the C major progression must produce the same
        // estimate as mono (down to floating-point noise) — the
        // downmix is a noiseless `(L + R) / 2`.
        let sr = 48_000;
        let mono = major_i_iv_v_i(261.63, 12.0, sr);
        let stereo: Vec<f32> = mono.iter().flat_map(|&s| [s, s]).collect();
        let est_mono = analyze_key(&mono, sr, 1).unwrap();
        let est_stereo = analyze_key(&stereo, sr, 2).unwrap();
        assert_eq!(est_mono.tonic_pc, est_stereo.tonic_pc);
        assert_eq!(est_mono.is_major, est_stereo.is_major);
    }

    #[test]
    fn chroma_table_covers_tonal_band_only() {
        let table = build_bin_chroma_table(48_000);
        assert!(table[0].is_none(), "DC bin must be below MIN_KEY_HZ");
        assert!(
            table.last().unwrap().is_none(),
            "Nyquist bin must be above MAX_KEY_HZ"
        );
        assert!(
            table.iter().any(Option::is_some),
            "tonal band must produce hits"
        );
    }

    #[test]
    fn chroma_table_places_a440_on_pc_9() {
        // Regression test for the sign-of-shift error that broke the
        // first M11c.2 draft. A4 must contribute most of its weight
        // to PC 9 (A). At 4096-point FFT / 44.1 kHz, A4 lands on
        // bin ~41 (≈440 Hz exactly because 4096 * 110.25 / 44100 ≈
        // 41), and the chromaticity formula resolves it to pure
        // PC 9 with `lower_weight ≈ 1.0`.
        let table = build_bin_chroma_table(44_100);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let bin_440 = (440.0_f32 / (44_100.0_f32 / KEY_FRAME_SIZE as f32)).round() as usize;
        let bc = table[bin_440].expect("A4 must be inside tonal band");
        let pc_with_more_weight = if bc.lower_weight >= 0.5 {
            bc.lower_pc
        } else {
            bc.upper_pc
        };
        assert_eq!(
            pc_with_more_weight, 9,
            "bin closest to A4 must put majority weight on PC 9 (A); got BinChroma {bc:?}"
        );

        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let bin_c4 = (261.63_f32 / (44_100.0_f32 / KEY_FRAME_SIZE as f32)).round() as usize;
        let bc = table[bin_c4].expect("C4 must be inside tonal band");
        let pc_with_more_weight = if bc.lower_weight >= 0.5 {
            bc.lower_pc
        } else {
            bc.upper_pc
        };
        assert_eq!(
            pc_with_more_weight, 0,
            "bin closest to C4 must put majority weight on PC 0 (C); got BinChroma {bc:?}"
        );
    }

    #[test]
    fn camelot_round_trip_through_parse_camelot() {
        for pc in 0u8..12 {
            for is_major in [true, false] {
                let est = KeyEstimate {
                    tonic_pc: pc,
                    is_major,
                    confidence: 0.5,
                };
                let camelot = est.camelot();
                let (num, mode) = parse_camelot(camelot).expect("valid camelot");
                assert!(
                    (1..=12).contains(&num),
                    "Camelot number out of range: {num}"
                );
                assert_eq!(mode, is_major, "mode round-trip mismatch on {camelot}");
            }
        }
    }

    #[test]
    fn camelot_keys_disagree_tolerates_relative_pair() {
        // C major (8B) vs A minor (8A) share the wheel number — the
        // K-K templates can't disambiguate them reliably, and DJs
        // mix between them as if they're the same key.
        assert!(!camelot_keys_disagree("8B", "8A"));
        assert!(!camelot_keys_disagree("12A", "12B"));
    }

    #[test]
    fn camelot_keys_disagree_flags_parallel_pair() {
        // C major (8B) vs C minor (5A) share the tonic pitch class
        // but live in different wheel families. This is a real
        // disagreement worth surfacing.
        assert!(camelot_keys_disagree("8B", "5A"));
        // Different wheel halves entirely.
        assert!(camelot_keys_disagree("1A", "7B"));
    }

    #[test]
    fn camelot_keys_disagree_returns_false_for_malformed_input() {
        // We don't want a typo'd Camelot string to fire the ⚠
        // indicator; better to under-flag than over-flag.
        assert!(!camelot_keys_disagree("", "8B"));
        assert!(!camelot_keys_disagree("8B", "C major"));
        assert!(!camelot_keys_disagree("13B", "8B"));
        assert!(!camelot_keys_disagree("8X", "8B"));
    }

    #[test]
    fn parse_camelot_accepts_both_cases() {
        assert_eq!(parse_camelot("8b"), Some((8, true)));
        assert_eq!(parse_camelot("12a"), Some((12, false)));
        assert_eq!(parse_camelot("  3B  "), Some((3, true)));
    }

    #[test]
    fn camelot_arrays_form_a_consistent_wheel() {
        // Sanity check: every pitch class must produce a unique
        // Camelot major position 1..12 (and same for minor). A
        // typo'd array entry would let two pitch classes land on
        // the same wheel slot — the test asserts a permutation.
        let mut majors: Vec<&str> = CAMELOT_MAJOR.to_vec();
        majors.sort_unstable();
        majors.dedup();
        assert_eq!(
            majors.len(),
            12,
            "Camelot major positions must be 12 distinct slots"
        );

        let mut minors: Vec<&str> = CAMELOT_MINOR.to_vec();
        minors.sort_unstable();
        minors.dedup();
        assert_eq!(
            minors.len(),
            12,
            "Camelot minor positions must be 12 distinct slots"
        );

        // Relative-key pairing: position 8 must be C major + A minor.
        assert_eq!(CAMELOT_MAJOR[0], "8B");
        assert_eq!(CAMELOT_MINOR[9], "8A");
    }
}
