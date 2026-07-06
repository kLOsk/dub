//! Round-trip, input-validation, and clamping tests for `encode_flac_24bit`.
//!
//! Round-trips decode with symphonia — the same decoder the app ships — so
//! these tests also prove the encoded files are readable by the Dub engine.

use std::f32::consts::TAU;
use std::path::Path;

use dub_encode::{encode_flac_24bit, EncodeError};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// One 24-bit step is 2^-23; encode rounds (≤ half a step) and symphonia's
/// i24→f32 conversion is exact, so 2^-22 leaves comfortable headroom.
const TOLERANCE: f32 = 1.0 / 4_194_304.0;

struct Decoded {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: usize,
    n_frames: Option<u64>,
    bits_per_sample: Option<u32>,
}

fn decode_flac(path: &Path) -> Decoded {
    let file = std::fs::File::open(path).expect("open encoded file");
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .expect("probe FLAC container");
    let mut format = probed.format;
    let track = format.default_track().expect("default track").clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("make FLAC decoder");

    let mut samples = Vec::new();
    let mut sample_rate = 0;
    let mut channels = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => panic!("packet read failed: {e}"),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = decoder.decode(&packet).expect("decode packet");
        let spec = *decoded.spec();
        sample_rate = spec.rate;
        channels = spec.channels.count();
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buf.samples());
    }

    Decoded {
        samples,
        sample_rate,
        channels,
        n_frames: track.codec_params.n_frames,
        bits_per_sample: track.codec_params.bits_per_sample,
    }
}

fn sine_interleaved(frames: usize, freq: f32, sample_rate: f32, channels: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        let s = 0.5 * (TAU * freq * n as f32 / sample_rate).sin();
        for _ in 0..channels {
            out.push(s);
        }
    }
    out
}

fn assert_round_trip(input: &[f32], sample_rate: u32, channels: u8, frames: usize) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("roundtrip.flac");
    encode_flac_24bit(input, sample_rate, channels, &path).expect("encode");

    let decoded = decode_flac(&path);
    assert_eq!(decoded.sample_rate, sample_rate);
    assert_eq!(decoded.channels, usize::from(channels));
    assert_eq!(decoded.bits_per_sample, Some(24));
    assert_eq!(
        decoded.n_frames,
        Some(frames as u64),
        "duration must be exact"
    );
    assert_eq!(
        decoded.samples.len(),
        input.len(),
        "sample count must be exact"
    );

    for (i, (a, b)) in input.iter().zip(&decoded.samples).enumerate() {
        assert!(
            (a - b).abs() <= TOLERANCE,
            "sample {i} outside 24-bit tolerance: input {a} decoded {b}"
        );
    }
}

#[test]
fn stereo_sine_round_trips_within_24bit_tolerance() {
    let frames = 3 * 44_100;
    let input = sine_interleaved(frames, 440.0, 44_100.0, 2);
    assert_round_trip(&input, 44_100, 2, frames);
}

#[test]
fn mono_sine_round_trips_within_24bit_tolerance() {
    let frames = 3 * 44_100;
    let input = sine_interleaved(frames, 440.0, 44_100.0, 1);
    assert_round_trip(&input, 44_100, 1, frames);
}

#[test]
fn out_of_range_samples_clamp_instead_of_wrapping() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("clamp.flac");

    let mut input = vec![1.5_f32; 2048];
    input.extend(std::iter::repeat_n(-1.5_f32, 2048));
    encode_flac_24bit(&input, 44_100, 1, &path).expect("encode");

    let decoded = decode_flac(&path);
    assert_eq!(decoded.samples.len(), input.len());
    for (i, &s) in decoded.samples[..2048].iter().enumerate() {
        assert!(
            (0.9999..=1.0).contains(&s),
            "sample {i} wrapped or under-clamped: {s}"
        );
    }
    for (i, &s) in decoded.samples[2048..].iter().enumerate() {
        assert!(
            (-1.0..=-0.9999).contains(&s),
            "sample {i} wrapped or under-clamped: {s}"
        );
    }
}

// Validation must reject bad input before touching the filesystem, so a
// nonexistent directory doubles as the probe for that ordering.
fn unwritable() -> &'static Path {
    Path::new("/nonexistent-dub-encode-test/out.flac")
}

#[test]
fn zero_channels_is_invalid_input() {
    let err = encode_flac_24bit(&[0.0; 4], 44_100, 0, unwritable()).unwrap_err();
    assert!(matches!(err, EncodeError::InvalidInput(_)), "got {err:?}");
}

#[test]
fn more_than_two_channels_is_invalid_input() {
    let err = encode_flac_24bit(&[0.0; 6], 44_100, 3, unwritable()).unwrap_err();
    assert!(matches!(err, EncodeError::InvalidInput(_)), "got {err:?}");
}

#[test]
fn empty_samples_is_invalid_input() {
    let err = encode_flac_24bit(&[], 44_100, 2, unwritable()).unwrap_err();
    assert!(matches!(err, EncodeError::InvalidInput(_)), "got {err:?}");
}

#[test]
fn zero_sample_rate_is_invalid_input() {
    let err = encode_flac_24bit(&[0.0; 4], 0, 2, unwritable()).unwrap_err();
    assert!(matches!(err, EncodeError::InvalidInput(_)), "got {err:?}");
}

#[test]
fn non_frame_aligned_stereo_is_invalid_input() {
    let err = encode_flac_24bit(&[0.0; 5], 44_100, 2, unwritable()).unwrap_err();
    assert!(matches!(err, EncodeError::InvalidInput(_)), "got {err:?}");
}
