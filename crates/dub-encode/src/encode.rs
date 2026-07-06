//! Interleaved `f32` → 24-bit FLAC file encoding via `flacenc` (pure Rust).

use std::path::Path;

use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use flacenc::source::MemSource;

use crate::error::EncodeError;

/// Full scale for 24-bit signed audio (2^23); `i24::MAX` is one step below.
const SCALE_24BIT: f32 = 8_388_608.0;
const MAX_24BIT: f32 = 8_388_607.0;

/// Encode interleaved `f32` samples to a 24-bit FLAC file at `out_path`.
///
/// `interleaved` is LRLR… for stereo (`channels == 2`) or a plain sample
/// sequence for mono (`channels == 1`); its length must be a multiple of the
/// channel count. Samples are expected in `[-1.0, 1.0]`; values outside that
/// range are clamped to full scale, never wrapped.
///
/// # Errors
///
/// [`EncodeError::InvalidInput`] for an unsupported channel count (0 or > 2),
/// zero sample rate, an empty buffer, or a non-frame-aligned buffer — all
/// checked before anything touches the filesystem. [`EncodeError::Encode`] if
/// the FLAC encoder rejects the stream, [`EncodeError::Io`] if writing the
/// output file fails.
pub fn encode_flac_24bit(
    interleaved: &[f32],
    sample_rate: u32,
    channels: u8,
    out_path: &Path,
) -> Result<(), EncodeError> {
    if channels == 0 || channels > 2 {
        return Err(EncodeError::InvalidInput(format!(
            "unsupported channel count {channels} (mono or stereo only)"
        )));
    }
    if sample_rate == 0 {
        return Err(EncodeError::InvalidInput(
            "sample rate must be non-zero".to_owned(),
        ));
    }
    if interleaved.is_empty() {
        return Err(EncodeError::InvalidInput("no samples to encode".to_owned()));
    }
    if !interleaved.len().is_multiple_of(usize::from(channels)) {
        return Err(EncodeError::InvalidInput(format!(
            "sample count {} is not a multiple of channel count {channels}",
            interleaved.len()
        )));
    }

    // No dither: 24-bit quantization noise sits near -144 dBFS, ~90 dB below
    // vinyl surface noise (roughly -50..-70 dBFS), so dither would only shape
    // noise the medium already buries. Round-to-nearest is enough.
    let quantized: Vec<i32> = interleaved
        .iter()
        .map(|&s| {
            // Clamp after scaling so hot input (>= 1.0) saturates at i24::MAX
            // instead of wrapping. The cast is lossless: the clamped range
            // fits i32 exactly (and non-finite input degrades to 0/full scale
            // via Rust's saturating float casts).
            (s * SCALE_24BIT).round().clamp(-SCALE_24BIT, MAX_24BIT) as i32
        })
        .collect();

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| EncodeError::Encode(format!("encoder config rejected: {e}")))?;
    // u32 → usize is a widening cast on every target we build for.
    let source =
        MemSource::from_samples(&quantized, usize::from(channels), 24, sample_rate as usize);
    let mut stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| EncodeError::Encode(e.to_string()))?;

    // flacenc shrinks StreamInfo's min_block_size to the short final frame,
    // which makes decoders (symphonia included) classify the stream as
    // variable-blocksize and then reject its frame-numbered headers — the
    // file becomes undecodable by the app. The FLAC spec exempts the last
    // frame from the fixed block size, so force min == max like libFLAC does.
    stream
        .stream_info_mut()
        .set_block_sizes(config.block_size, config.block_size)
        .map_err(|e| EncodeError::Encode(format!("stream info rejected: {e}")))?;

    let mut sink = ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| EncodeError::Encode(e.to_string()))?;
    std::fs::write(out_path, sink.as_slice())?;
    Ok(())
}
