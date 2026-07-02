//! In-memory track buffer.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, Tag};
use symphonia::core::probe::{Hint, ProbeResult};

/// Errors that can occur while loading a track.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be opened.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// The file is in a format symphonia could not probe.
    #[error("unsupported or corrupt format: {0}")]
    Format(String),

    /// The file contained no audio tracks.
    #[error("file contains no audio tracks")]
    NoAudioTrack,

    /// Decoding produced an unexpected channel layout (zero or > 2 channels).
    #[error("unsupported channel layout: {0} channels")]
    UnsupportedChannels(u8),

    /// Decoding produced no usable samples.
    #[error("decode produced no samples")]
    Empty,
}

impl From<SymphoniaError> for LoadError {
    fn from(e: SymphoniaError) -> Self {
        Self::Format(e.to_string())
    }
}

/// Interleaved sample storage — either a plain one-shot buffer or a
/// streaming decode-ahead buffer behind a watermark.
///
/// The `Streaming` variant is fully allocated up front; a monotonic
/// `decoded` watermark (in interleaved samples) publishes how much of
/// it holds real audio. Samples are stored as `f32` **bit patterns in
/// `AtomicU32`s** so the writer (the decode thread driving
/// [`StreamingLoad`]) and concurrent readers are race-free in safe
/// Rust — the crate keeps its `#![forbid(unsafe_code)]`. Relaxed
/// per-element loads/stores compile to plain moves on every target we
/// ship; the watermark's Release/Acquire pair is what orders them.
///
/// `total` is the number of interleaved samples the track claims to
/// contain: the container-declared length at allocation, corrected
/// once (downward) when the decode finishes or aborts. Duration and
/// end-of-track semantics key off `total`; audible reads key off
/// `decoded`, so a playhead past the watermark renders silence
/// rather than not-yet-decoded data ([`Track::frame`]'s existing
/// out-of-range contract).
///
/// Tracks decoded in one shot ([`Track::load_from_path`],
/// [`Track::from_interleaved`]) use `Complete` and none of this
/// machinery is observable.
enum SampleStore {
    /// One-shot decode — plain contiguous storage, zero overhead,
    /// borrowable via [`Track::samples`].
    Complete(Box<[f32]>),
    /// Streaming decode-ahead. Not borrowable as `&[f32]`; read via
    /// [`Track::frame`] or copy out via [`Track::samples_to_vec`].
    Streaming(StreamingStore),
}

struct StreamingStore {
    /// `f32` bit patterns.
    buf: Box<[AtomicU32]>,
    /// Interleaved samples visible to readers. Monotonic;
    /// Release-published by the writer, Acquire-read by readers.
    decoded: AtomicUsize,
    /// Interleaved samples the track claims to contain
    /// (`<= buf.len()`). Corrected once at decode completion.
    total: AtomicUsize,
}

impl std::fmt::Debug for SampleStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete(buf) => f
                .debug_struct("SampleStore::Complete")
                .field("len", &buf.len())
                .finish(),
            Self::Streaming(s) => f
                .debug_struct("SampleStore::Streaming")
                .field("capacity", &s.buf.len())
                .field("decoded", &s.decoded.load(Ordering::Acquire))
                .field("total", &s.total.load(Ordering::Acquire))
                .finish(),
        }
    }
}

impl SampleStore {
    /// Zero-filled streaming store awaiting a decode. `total` starts
    /// at the container-declared `capacity` and is corrected by
    /// [`Self::finalize`].
    fn preallocated(capacity: usize) -> Self {
        let buf: Box<[AtomicU32]> = (0..capacity).map(|_| AtomicU32::new(0)).collect();
        Self::Streaming(StreamingStore {
            buf,
            decoded: AtomicUsize::new(0),
            total: AtomicUsize::new(capacity),
        })
    }

    /// Interleaved samples visible to readers.
    fn decoded_len(&self) -> usize {
        match self {
            Self::Complete(buf) => buf.len(),
            Self::Streaming(s) => s.decoded.load(Ordering::Acquire),
        }
    }

    /// Interleaved samples the track claims to contain.
    fn total_len(&self) -> usize {
        match self {
            Self::Complete(buf) => buf.len(),
            Self::Streaming(s) => s.total.load(Ordering::Acquire),
        }
    }

    /// Read one interleaved sample. Callers bound `i` by
    /// [`Self::decoded_len`] first.
    fn sample(&self, i: usize) -> f32 {
        match self {
            Self::Complete(buf) => buf[i],
            Self::Streaming(s) => f32::from_bits(s.buf[i].load(Ordering::Relaxed)),
        }
    }

    /// Writer-side append (streaming stores only). Returns how many
    /// samples were accepted — clamped at capacity, so a container
    /// that under-declared its length gets the excess dropped and
    /// counted by the caller.
    fn append(&self, chunk: &[f32]) -> usize {
        let Self::Streaming(s) = self else {
            return 0;
        };
        let start = s.decoded.load(Ordering::Relaxed);
        let room = s.buf.len().saturating_sub(start);
        let n = chunk.len().min(room);
        for (i, &v) in chunk[..n].iter().enumerate() {
            s.buf[start + i].store(v.to_bits(), Ordering::Relaxed);
        }
        s.decoded.store(start + n, Ordering::Release);
        n
    }

    /// Close a streaming store: clamp `total` down to what actually
    /// decoded. Called on decode completion *and* on mid-file decode
    /// failure, so `is_fully_decoded` observers always unblock.
    fn finalize(&self) {
        if let Self::Streaming(s) = self {
            let decoded = s.decoded.load(Ordering::Acquire);
            s.total.fetch_min(decoded, Ordering::AcqRel);
        }
    }
}

/// An in-memory audio track.
///
/// Audio is stored interleaved (`L, R, L, R, …` for stereo, `M, M, …` for mono),
/// 32-bit float, in the file's original sample rate. Resampling to engine SR
/// happens at the engine boundary if needed (PRD §4.4).
///
/// Tracks are immutable after construction; the engine accesses them via
/// `Arc<Track>` so multiple decks can hold the same track without copies.
/// (A streaming load fills the pre-allocated sample buffer behind a
/// watermark — see [`SampleStore`] — but the reader-visible contract is
/// unchanged: what you can read never mutates.)
///
/// ## Metadata
///
/// Tempo lives here as an optional field, filled in by callers that have
/// run BPM analysis. `dub-io` deliberately does *not* depend on
/// `dub-bpm` — that would force every audio-loading site to pay the
/// analysis cost up-front. Instead, a typical pipeline is:
///
/// ```ignore
/// let track = Track::load_from_path(p)?;
/// let est = dub_bpm::analyze_bpm(track.samples(), track.sample_rate(), track.channels())?;
/// let track = track.with_bpm(Some(est.bpm));
/// ```
///
/// See PRD §5.3 (M7.5 / library import).
#[derive(Debug, Clone)]
pub struct Track {
    store: Arc<SampleStore>,
    sample_rate: u32,
    channels: u8,
    bpm: Option<f64>,
    metadata: TrackMetadata,
}

/// Lightweight metadata-only snapshot of an audio file. Returned by
/// [`read_metadata`] for library browsing where decoding the whole
/// file would be wasteful, and exposed on every fully loaded
/// [`Track`] via the `title()` / `artist()` / `album()` accessors plus
/// the [`Track::extended_metadata`] surface for the larger field set.
///
/// Field-naming follows the [`StandardTagKey`] enum and maps onto the
/// columns in the `track_metadata_source` table documented in
/// `docs/spec/LIBRARY-SCHEMA.md`. The dub-library M11c importer is the
/// primary consumer of the extended fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackMetadata {
    /// Track title (`TIT2` ID3 / `TITLE` Vorbis / `©nam` MP4).
    pub title: Option<String>,
    /// Track artist (`TPE1` ID3 / `ARTIST` Vorbis / `©ART` MP4).
    pub artist: Option<String>,
    /// Album (`TALB` / `ALBUM` / `©alb`).
    pub album: Option<String>,
    /// Genre (`TCON` ID3 / `GENRE` Vorbis / `©gen` MP4).
    pub genre: Option<String>,
    /// Free-text comment (`COMM` ID3 / `COMMENT` Vorbis / `©cmt` MP4).
    /// Some apps (Mixed In Key) write structured data here; the
    /// importer stores the verbatim text.
    pub comment: Option<String>,
    /// Composer (`TCOM` ID3 / `COMPOSER` Vorbis / `©wrt` MP4).
    pub composer: Option<String>,
    /// Year (`TYER` ID3 / `DATE` Vorbis / `©day` MP4). Parsed to an
    /// `i32` from whatever ISO-style string the container carries
    /// (typical forms: `"1996"`, `"1996-04-23"`); `None` on parse
    /// failure rather than a partial value.
    pub year: Option<i32>,
    /// Track number (`TRCK` ID3 / `TRACKNUMBER` Vorbis / `trkn` MP4).
    /// Tag-side strings of the form `"3/12"` keep only the lead
    /// component.
    pub track_number: Option<i32>,
    /// BPM as reported by the file's tag (`TBPM` ID3 / `BPM` Vorbis /
    /// `tmpo` MP4). Stored alongside Dub's own measured grid in
    /// `track_beatgrids(source='id3')` per PRD §8.3.
    pub bpm: Option<f64>,
    /// Musical key in whatever notation the source uses (`TKEY`
    /// ID3 / `INITIALKEY` Vorbis). Browser normalises for display.
    pub key: Option<String>,
    /// Track-gain in dB from a `ReplayGain` frame (`RVA2` /
    /// `REPLAYGAIN_TRACK_GAIN`). The importer stores it but does
    /// not apply it (PRD §8.4: "Measured, not applied" in v1).
    pub gain_db: Option<f64>,
}

impl TrackMetadata {
    /// `true` when no field carries any tag text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.genre.is_none()
            && self.comment.is_none()
            && self.composer.is_none()
            && self.year.is_none()
            && self.track_number.is_none()
            && self.bpm.is_none()
            && self.key.is_none()
            && self.gain_db.is_none()
    }
}

/// Read tag metadata from an audio file *without* decoding samples.
///
/// Uses symphonia's probe to locate the metadata revision attached to
/// the container (`ID3v2` for MP3, MP4 atoms for AAC / ALAC, Vorbis
/// comments for FLAC / OGG, RIFF INFO for WAV). Cheap: opens the file,
/// reads the metadata block, closes the file.
///
/// Returns [`TrackMetadata::default`] when the file has no recognised
/// tags. Returns [`LoadError::Io`] / [`LoadError::Format`] for the
/// same cases [`Track::load_from_path`] would.
///
/// Intended for library browsing — the file-browser populates each
/// row's title / artist columns by calling this function lazily and
/// caching by URL.
///
/// # Errors
///
/// * [`LoadError::Io`] when the file can't be opened.
/// * [`LoadError::Format`] when symphonia can't probe the file's
///   container.
pub fn read_metadata(path: impl AsRef<Path>) -> Result<TrackMetadata, LoadError> {
    let path = path.as_ref();
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    Ok(extract_metadata(probed))
}

fn extract_metadata(mut probed: ProbeResult) -> TrackMetadata {
    let mut out = TrackMetadata::default();
    // Container-level metadata (ID3 in MP3, MP4 atoms in m4a, RIFF
    // INFO in WAV). `metadata.get()` is consumed via `current()`;
    // we walk the most recent revision.
    if let Some(meta) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
        merge_metadata(&mut out, meta);
    }
    // Format-level metadata (Vorbis comments for OGG/FLAC, some
    // MP4 atoms). Symphonia keeps these on the format reader.
    if let Some(meta) = probed.format.metadata().current() {
        merge_metadata(&mut out, meta);
    }
    out
}

fn merge_metadata(out: &mut TrackMetadata, revision: &MetadataRevision) {
    for tag in revision.tags() {
        copy_tag(out, tag);
    }
}

fn copy_tag(out: &mut TrackMetadata, tag: &Tag) {
    let Some(key) = tag.std_key else { return };
    let value = tag.value.to_string();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    let owned = trimmed.to_string();
    match key {
        StandardTagKey::TrackTitle if out.title.is_none() => out.title = Some(owned),
        // `Artist` (TPE1 / TPE2-when-only-TPE2 is present in some
        // libraries) takes precedence over `AlbumArtist` (TPE2)
        // — the second arm only fills the slot when the first
        // didn't.
        StandardTagKey::Artist | StandardTagKey::AlbumArtist if out.artist.is_none() => {
            out.artist = Some(owned);
        }
        StandardTagKey::Album if out.album.is_none() => out.album = Some(owned),
        StandardTagKey::Genre if out.genre.is_none() => out.genre = Some(owned),
        StandardTagKey::Comment if out.comment.is_none() => out.comment = Some(owned),
        StandardTagKey::Composer if out.composer.is_none() => out.composer = Some(owned),
        StandardTagKey::Date if out.year.is_none() => {
            // The Date tag may carry a full ISO string ("1996-04-23"),
            // a four-digit year ("1996"), or junk. Take the first
            // 4 chars that parse as an i32; ignore otherwise.
            if let Some(prefix) = trimmed.get(..4) {
                if let Ok(year) = prefix.parse::<i32>() {
                    out.year = Some(year);
                }
            }
        }
        StandardTagKey::TrackNumber if out.track_number.is_none() => {
            // ID3 TRCK can be "3" or "3/12"; keep the lead.
            let lead = trimmed.split('/').next().unwrap_or(trimmed);
            if let Ok(n) = lead.parse::<i32>() {
                out.track_number = Some(n);
            }
        }
        StandardTagKey::Bpm if out.bpm.is_none() => {
            if let Ok(b) = trimmed.parse::<f64>() {
                if b > 0.0 && b < 500.0 {
                    out.bpm = Some(b);
                }
            }
        }
        // Note: symphonia 0.5.5's `StandardTagKey` does not expose a
        // musical-key variant; native ID3 `TKEY` / Vorbis `INITIALKEY`
        // reading would require matching on the format-specific
        // `tag.key` string. PRD §8.4 defers key detection to v1.x,
        // and Mixed In Key (the source the v1 importer cares about)
        // writes its key data into the `Comment` field which we read
        // above. The dedicated `key` column on TrackMetadata stays
        // `None` from container-tag reading; M11e Serato importer
        // populates it from Serato's `Autotags` GEOB frame.
        StandardTagKey::ReplayGainTrackGain if out.gain_db.is_none() => {
            // ReplayGain frames carry strings like "-7.20 dB". Strip
            // the unit suffix before parsing.
            let cleaned = trimmed
                .trim_end_matches("dB")
                .trim_end_matches("Db")
                .trim_end_matches("DB")
                .trim_end_matches("db")
                .trim();
            if let Ok(g) = cleaned.parse::<f64>() {
                out.gain_db = Some(g);
            }
        }
        _ => {}
    }
}

impl Track {
    /// Construct a `Track` directly from interleaved samples. Useful for tests.
    ///
    /// `samples.len()` must equal `frames * channels`. Channels must be 1 or 2.
    /// Returns `None` if the constraints are violated.
    #[must_use]
    pub fn from_interleaved(samples: Vec<f32>, sample_rate: u32, channels: u8) -> Option<Self> {
        if !(1..=2).contains(&channels) || sample_rate == 0 {
            return None;
        }
        let n = samples.len();
        if n == 0 || !n.is_multiple_of(usize::from(channels)) {
            return None;
        }
        Some(Self {
            store: Arc::new(SampleStore::Complete(samples.into_boxed_slice())),
            sample_rate,
            channels,
            bpm: None,
            metadata: TrackMetadata::default(),
        })
    }

    /// Load a track from a path. Decodes the entire file into RAM.
    ///
    /// Format detection uses the file extension as a hint plus symphonia's
    /// content sniffer. WAV/PCM is the only format guaranteed in M1; other
    /// formats are added per-milestone via symphonia features.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] if the file is missing, in an unsupported
    /// format, contains no audio tracks, has more than 2 channels, or
    /// produces no decodable samples.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();

        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }

        let mut probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        // Snapshot any container-level metadata before we move
        // `probed.format` below. Vorbis-style format-level tags are
        // read again after decode (the format reader may surface
        // additional revisions once it has walked the headers).
        let mut metadata = TrackMetadata::default();
        if let Some(meta) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
            merge_metadata(&mut metadata, meta);
        }
        if let Some(meta) = probed.format.metadata().current() {
            merge_metadata(&mut metadata, meta);
        }

        let mut format = probed.format;
        let primary = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(LoadError::NoAudioTrack)?;
        let track_id = primary.id;

        let codec_params = primary.codec_params.clone();
        let mut decoder =
            symphonia::default::get_codecs().make(&codec_params, &DecoderOptions::default())?;

        // Sample rate / channels MAY come from codec params (typical for
        // RIFF formats) or only become known after the first packet is
        // decoded (typical for ISO MP4 / AAC where the channel count
        // lives in the audio object type, not the sample entry box). We
        // try params first, then fall back to the first decoded buffer's
        // spec inside the loop below.
        let mut sample_rate = codec_params.sample_rate;
        let mut channels: Option<u8> = codec_params
            .channels
            .map(|c| u8::try_from(c.count()).unwrap_or(u8::MAX));

        let mut samples: Vec<f32> = Vec::new();
        let mut sample_buf: Option<SampleBuffer<f32>> = None;

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(SymphoniaError::ResetRequired) => {
                    return Err(LoadError::Format("decoder reset required".into()));
                }
                Err(e) => return Err(e.into()),
            };
            if packet.track_id() != track_id {
                continue;
            }

            let audio = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(e.into()),
            };

            // First decoded packet: lock in any spec fields we couldn't
            // get from codec_params. After this point, spec changes mean
            // a stream-format-change which we don't support yet.
            let spec = *audio.spec();
            if sample_rate.is_none() {
                sample_rate = Some(spec.rate);
            }
            if channels.is_none() {
                let count = spec.channels.count();
                channels = Some(u8::try_from(count).unwrap_or(u8::MAX));
            }

            // Capacity grows lazily here too — `audio.capacity()` is the
            // packet's max frame count, fixed for the codec.
            let buf = sample_buf
                .get_or_insert_with(|| SampleBuffer::<f32>::new(audio.capacity() as u64, spec));
            buf.copy_interleaved_ref(audio);
            samples.extend_from_slice(buf.samples());
        }

        let sample_rate =
            sample_rate.ok_or_else(|| LoadError::Format("no sample rate found".into()))?;
        let channels =
            channels.ok_or_else(|| LoadError::Format("no channel layout found".into()))?;
        if !(1..=2).contains(&channels) {
            return Err(LoadError::UnsupportedChannels(channels));
        }

        if samples.is_empty() {
            return Err(LoadError::Empty);
        }

        // The format reader may have surfaced additional Vorbis-style
        // metadata revisions while walking packets; merge them in
        // after decode completes.
        if let Some(meta) = format.metadata().current() {
            merge_metadata(&mut metadata, meta);
        }

        Ok(Self {
            store: Arc::new(SampleStore::Complete(samples.into_boxed_slice())),
            sample_rate,
            channels,
            bpm: None,
            metadata,
        })
    }

    /// Begin a streaming (decode-ahead) load.
    ///
    /// Probes the container, allocates the full-length sample buffer
    /// from the declared frame count, and returns a [`StreamingLoad`]
    /// whose [`StreamingLoad::track`] is immediately constructible —
    /// but silent until [`StreamingLoad::decode_until_secs`] makes the
    /// head audible. The intended sequence (`dub-ffi::load_track`):
    ///
    /// 1. `begin_streaming` + `decode_until_secs(HEAD)` on the caller
    ///    thread — O(head), independent of track length.
    /// 2. Install the track on the deck; playback is live.
    /// 3. Move the `StreamingLoad` to a worker thread and
    ///    [`StreamingLoad::finish`] — the watermark advances behind
    ///    the playhead at hundreds of times realtime.
    ///
    /// Falls back to a one-shot [`Self::load_from_path`] internally
    /// (returning an already-finished handle) when the container
    /// doesn't declare a usable frame count / sample rate / channel
    /// layout up front — rare in practice (headerless streams);
    /// MP3-with-Xing, WAV, AIFF, FLAC, ALAC and MP4/AAC all declare.
    ///
    /// Streaming tracks carry the container's *pre-decode* metadata
    /// revision only; the post-decode Vorbis-style revisions
    /// `load_from_path` merges at EOF are not awaited. Library
    /// metadata comes from [`read_metadata`] / the importers, so
    /// nothing user-visible keys off that difference.
    ///
    /// # Errors
    ///
    /// Same failure surface as [`Self::load_from_path`]: I/O,
    /// unsupported format, no audio track, unsupported channel
    /// layout.
    pub fn begin_streaming(path: impl AsRef<Path>) -> Result<StreamingLoad, LoadError> {
        let path = path.as_ref();

        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }
        let mut probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let mut metadata = TrackMetadata::default();
        if let Some(meta) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
            merge_metadata(&mut metadata, meta);
        }
        if let Some(meta) = probed.format.metadata().current() {
            merge_metadata(&mut metadata, meta);
        }

        let format = probed.format;
        let primary = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(LoadError::NoAudioTrack)?;
        let track_id = primary.id;
        let codec_params = primary.codec_params.clone();

        // Everything the pre-allocation needs must be declared up
        // front; otherwise fall back to the one-shot decode.
        let declared = (|| {
            let sample_rate = codec_params.sample_rate?;
            let channels = codec_params
                .channels
                .map(|c| u8::try_from(c.count()).unwrap_or(u8::MAX))?;
            let n_frames = codec_params.n_frames?;
            Some((sample_rate, channels, n_frames))
        })();
        let Some((sample_rate, channels, n_frames)) = declared else {
            let track = Self::load_from_path(path)?;
            return Ok(StreamingLoad::finished(track));
        };
        if !(1..=2).contains(&channels) {
            return Err(LoadError::UnsupportedChannels(channels));
        }
        // Guard the allocation against garbage frame counts from a
        // corrupt header (`n_frames` is attacker-ish input): anything
        // claiming more than MAX_PREALLOC_SAMPLES (~8 GiB of f32, ~
        // 6.7 h of 48 kHz stereo) falls back to the one-shot path,
        // which only allocates what actually decodes. A declared
        // zero also falls back (the one-shot path surfaces `Empty`
        // faithfully if nothing decodes).
        let capacity = u64::from(channels).checked_mul(n_frames);
        let capacity = match capacity {
            Some(c) if c > 0 && c <= MAX_PREALLOC_SAMPLES => usize::try_from(c).ok(),
            _ => None,
        };
        let Some(capacity) = capacity else {
            let track = Self::load_from_path(path)?;
            return Ok(StreamingLoad::finished(track));
        };

        let decoder =
            symphonia::default::get_codecs().make(&codec_params, &DecoderOptions::default())?;
        let track = Self {
            store: Arc::new(SampleStore::preallocated(capacity)),
            sample_rate,
            channels,
            bpm: None,
            metadata,
        };
        Ok(StreamingLoad {
            format: Some(format),
            decoder: Some(decoder),
            track_id,
            sample_buf: None,
            track,
            overflow: 0,
            done: false,
        })
    }

    /// Return a copy of this track with its BPM annotation updated.
    ///
    /// Cloning is cheap — the underlying sample buffer is shared via
    /// `Arc`. This is a builder-style helper because `Track` is
    /// otherwise immutable after construction, which keeps the engine
    /// thread free of mutation hazards.
    #[must_use]
    pub fn with_bpm(self, bpm: Option<f64>) -> Self {
        Self { bpm, ..self }
    }

    /// Tempo annotation, or `None` if BPM analysis has not been run on
    /// this track yet.
    #[must_use]
    pub fn bpm(&self) -> Option<f64> {
        self.bpm
    }

    /// Track title from container metadata (ID3 `TIT2`, Vorbis
    /// `TITLE`, MP4 `©nam`, …). `None` when the file is untagged.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.metadata.title.as_deref()
    }

    /// Track artist from container metadata. Falls back to album
    /// artist when only that tag was present.
    #[must_use]
    pub fn artist(&self) -> Option<&str> {
        self.metadata.artist.as_deref()
    }

    /// Album name from container metadata. `None` when the file is
    /// untagged.
    #[must_use]
    pub fn album(&self) -> Option<&str> {
        self.metadata.album.as_deref()
    }

    /// Full container-metadata snapshot (`genre` / `comment` /
    /// `composer` / `year` / `track_number` / `bpm` / `key` /
    /// `gain_db` in addition to the title / artist / album surfaced
    /// by the dedicated accessors). Used by the M11c library
    /// importer to populate the `track_metadata_source(source='id3')`
    /// row per `docs/spec/LIBRARY-SCHEMA.md`.
    #[must_use]
    pub fn extended_metadata(&self) -> &TrackMetadata {
        &self.metadata
    }

    /// Number of audio frames (one frame = one sample per channel).
    ///
    /// For a streaming load this is the *declared* length of the
    /// track (the container's frame count), independent of how much
    /// has decoded so far — duration display and end-of-track
    /// semantics stay stable while the tail streams in. Corrected
    /// once, at decode completion, if the container over-declared.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.store.total_len() / usize::from(self.channels)
    }

    /// Number of audio frames already decoded and audible. Equal to
    /// [`Self::frames`] for one-shot loads; trails it while a
    /// streaming load's tail is still decoding.
    #[must_use]
    pub fn decoded_frames(&self) -> usize {
        self.store.decoded_len() / usize::from(self.channels)
    }

    /// `true` once every frame the track claims to contain is
    /// audible. One-shot loads are born fully decoded; streaming
    /// loads flip to `true` when the tail decode completes (or
    /// aborts — a mid-file decode failure closes the track at the
    /// watermark rather than stranding observers).
    #[must_use]
    pub fn is_fully_decoded(&self) -> bool {
        self.store.decoded_len() >= self.store.total_len()
    }

    /// Sample rate the track was decoded at.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 1 (mono) or 2 (stereo).
    #[must_use]
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Length of the track in seconds (independent of any engine sample rate).
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        // Practical limit: a single track cannot exceed 2^52 frames (~3
        // million years at 48kHz), which is well within f64 mantissa range.
        #[allow(clippy::cast_precision_loss)]
        let frames_f = self.frames() as f64;
        frames_f / f64::from(self.sample_rate)
    }

    /// Borrow the raw interleaved sample buffer.
    ///
    /// Only one-shot loads ([`Self::load_from_path`],
    /// [`Self::from_interleaved`]) are borrowable; a track still
    /// owned by a [`StreamingLoad`] stores samples behind atomics
    /// and returns an **empty slice** here. Offline analysis of a
    /// streaming track should wait for [`Self::is_fully_decoded`]
    /// and copy out via [`Self::samples_to_vec`]; the audio thread
    /// reads through [`Self::frame`], which works for both.
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        match self.store.as_ref() {
            SampleStore::Complete(buf) => buf,
            SampleStore::Streaming(_) => &[],
        }
    }

    /// Copy the decoded interleaved samples into a fresh `Vec`.
    ///
    /// Works for both storage modes; the intended consumer is
    /// offline analysis (peaks / BPM) of a streaming track after
    /// [`Self::is_fully_decoded`], where [`Self::samples`] cannot
    /// borrow. O(n) copy — call from a background thread.
    #[must_use]
    pub fn samples_to_vec(&self) -> Vec<f32> {
        match self.store.as_ref() {
            SampleStore::Complete(buf) => buf.to_vec(),
            SampleStore::Streaming(_) => {
                let n = self.store.decoded_len();
                (0..n).map(|i| self.store.sample(i)).collect()
            }
        }
    }

    /// Read one stereo frame at the given integer position.
    ///
    /// Mono tracks are duplicated to both channels. Out-of-range
    /// positions return silence — including positions past a
    /// streaming load's decode watermark, so a needle drop ahead of
    /// the decode frontier renders silence and self-heals once the
    /// frontier passes it.
    #[must_use]
    pub fn frame(&self, frame_index: usize) -> [f32; 2] {
        let decoded_samples = self.store.decoded_len();
        match self.channels {
            1 => {
                if frame_index >= decoded_samples {
                    return [0.0, 0.0];
                }
                let s = self.store.sample(frame_index);
                [s, s]
            }
            2 => {
                if frame_index >= decoded_samples / 2 {
                    return [0.0, 0.0];
                }
                let i = frame_index * 2;
                [self.store.sample(i), self.store.sample(i + 1)]
            }
            // The constructor guarantees 1..=2.
            _ => unreachable!(),
        }
    }
}

/// Pre-allocation ceiling for streaming loads, in interleaved f32
/// samples (~8 GiB / ~6.7 h of 48 kHz stereo). A header declaring
/// more is treated as corrupt and the load falls back to the
/// one-shot path, which only allocates what actually decodes.
const MAX_PREALLOC_SAMPLES: u64 = 2 * 1024 * 1024 * 1024;

/// Driver for a streaming (decode-ahead) [`Track`] load. Created by
/// [`Track::begin_streaming`]; owns the symphonia decode state and
/// is the **single writer** to the shared sample buffer.
///
/// `Send` (symphonia's `FormatReader` / `Decoder` are `Send`), so the
/// intended pattern is: decode the head on the calling thread, then
/// move the handle into a background thread to [`Self::finish`] the
/// tail while the deck already plays.
pub struct StreamingLoad {
    // `None` only for the already-finished fallback handle.
    format: Option<Box<dyn FormatReader>>,
    decoder: Option<Box<dyn Decoder>>,
    track_id: u32,
    sample_buf: Option<SampleBuffer<f32>>,
    track: Track,
    /// Interleaved samples dropped because the container declared
    /// fewer frames than it actually decodes (encoder-padding
    /// mismatches). Logged at completion.
    overflow: u64,
    done: bool,
}

impl StreamingLoad {
    /// Wrap an already-complete track (the fallback path for
    /// containers the streaming loader can't pre-allocate for).
    fn finished(track: Track) -> Self {
        Self {
            format: None,
            decoder: None,
            track_id: 0,
            sample_buf: None,
            track,
            overflow: 0,
            done: true,
        }
    }

    /// The shared track handle. Playable as soon as enough head has
    /// decoded; positions past the watermark render silence.
    #[must_use]
    pub fn track(&self) -> Track {
        self.track.clone()
    }

    /// `true` once the decode has reached end-of-stream (or the
    /// store filled up) and the track's length has been finalized.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Decode until at least `secs` seconds of audio are audible (or
    /// the stream ends). Returns immediately when the watermark is
    /// already past the target.
    ///
    /// # Errors
    ///
    /// Same decode-failure surface as [`Track::load_from_path`].
    /// [`LoadError::Empty`] when the stream ends with zero decoded
    /// samples. On any error the store is finalized (closed at the
    /// current watermark) so `is_fully_decoded` observers unblock.
    pub fn decode_until_secs(&mut self, secs: f64) -> Result<(), LoadError> {
        let target_frames = (secs * f64::from(self.track.sample_rate)).ceil();
        // Saturating f64 → usize conversion for absurd targets. The
        // 2^53 pivot is exactly representable in f64 and far above
        // any real allocation (MAX_PREALLOC_SAMPLES).
        let target_frames = if target_frames >= 9_007_199_254_740_992.0 {
            usize::MAX
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                target_frames.max(0.0) as usize
            }
        };
        let target_samples = target_frames.saturating_mul(usize::from(self.track.channels));
        self.decode_until_samples(target_samples)
    }

    /// Decode the remainder of the stream and finalize the track's
    /// length. Call from a background thread — this is the
    /// O(track-length) part.
    ///
    /// # Errors
    ///
    /// Same surface as [`Self::decode_until_secs`]; the store is
    /// finalized even on error, so the track stays coherent (it
    /// simply ends at the watermark).
    pub fn finish(mut self) -> Result<(), LoadError> {
        self.decode_until_samples(usize::MAX)
    }

    fn decode_until_samples(&mut self, target_samples: usize) -> Result<(), LoadError> {
        if self.done {
            return Ok(());
        }
        let result = self.decode_loop(target_samples);
        if let Err(_) | Ok(true) = &result {
            // EOF or failure: close the store at the watermark so
            // duration matches what actually decoded and
            // `is_fully_decoded` observers unblock. (A short read
            // that merely hit `target_samples` stays open.)
            self.track.store.finalize();
            self.done = true;
            if self.overflow > 0 {
                eprintln!(
                    "dub-io: streaming decode produced {} samples past the \
                     container-declared length; excess dropped",
                    self.overflow
                );
            }
            if self.track.store.decoded_len() == 0 {
                return Err(LoadError::Empty);
            }
        }
        result.map(|_| ())
    }

    /// Inner packet loop. `Ok(true)` = end of stream, `Ok(false)` =
    /// target reached with stream still open.
    fn decode_loop(&mut self, target_samples: usize) -> Result<bool, LoadError> {
        let (Some(format), Some(decoder)) = (self.format.as_mut(), self.decoder.as_mut()) else {
            return Ok(true);
        };
        while self.track.store.decoded_len() < target_samples {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(true);
                }
                Err(SymphoniaError::ResetRequired) => {
                    return Err(LoadError::Format("decoder reset required".into()));
                }
                Err(e) => return Err(e.into()),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let audio = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(e.into()),
            };
            let spec = *audio.spec();
            let buf = self
                .sample_buf
                .get_or_insert_with(|| SampleBuffer::<f32>::new(audio.capacity() as u64, spec));
            buf.copy_interleaved_ref(audio);
            let chunk = buf.samples();
            let accepted = self.track.store.append(chunk);
            if accepted < chunk.len() {
                self.overflow += (chunk.len() - accepted) as u64;
                // Store full — declared length reached; everything
                // further would be dropped anyway.
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn from_interleaved_constructs_stereo() {
        let track = Track::from_interleaved(vec![0.1, 0.2, 0.3, 0.4], 48_000, 2).unwrap();
        assert_eq!(track.frames(), 2);
        assert_eq!(track.channels(), 2);
        assert_eq!(track.sample_rate(), 48_000);
        assert!((track.duration_seconds() - (2.0 / 48_000.0)).abs() < 1e-9);
    }

    #[test]
    fn from_interleaved_constructs_mono() {
        let track = Track::from_interleaved(vec![0.1, 0.2, 0.3], 44_100, 1).unwrap();
        assert_eq!(track.frames(), 3);
        assert_eq!(track.channels(), 1);
    }

    #[test]
    fn from_interleaved_rejects_bad_layouts() {
        // Mismatched length for stereo
        assert!(Track::from_interleaved(vec![0.1, 0.2, 0.3], 48_000, 2).is_none());
        // Empty
        assert!(Track::from_interleaved(vec![], 48_000, 2).is_none());
        // Too many channels
        assert!(Track::from_interleaved(vec![0.0; 6], 48_000, 6).is_none());
        // Zero sample rate
        assert!(Track::from_interleaved(vec![0.1, 0.2], 0, 2).is_none());
    }

    #[test]
    fn frame_at_returns_silence_past_end() {
        let track = Track::from_interleaved(vec![0.5, -0.5], 48_000, 2).unwrap();
        // Exact f32 equality is correct: these are literal stored samples.
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(track.frame(0), [0.5, -0.5]);
            assert_eq!(track.frame(1), [0.0, 0.0]);
            assert_eq!(track.frame(usize::MAX), [0.0, 0.0]);
        }
    }

    #[test]
    fn mono_frame_duplicates_to_stereo() {
        let track = Track::from_interleaved(vec![0.7, -0.3], 48_000, 1).unwrap();
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(track.frame(0), [0.7, 0.7]);
            assert_eq!(track.frame(1), [-0.3, -0.3]);
        }
    }

    #[test]
    fn bpm_defaults_to_none() {
        let track = Track::from_interleaved(vec![0.1, 0.2], 48_000, 2).unwrap();
        assert!(track.bpm().is_none());
    }

    #[test]
    fn with_bpm_attaches_and_overrides() {
        let track = Track::from_interleaved(vec![0.1, 0.2], 48_000, 2).unwrap();
        let with = track.clone().with_bpm(Some(128.0));
        assert_eq!(with.bpm(), Some(128.0));
        // Original is unchanged (builder-style).
        assert!(track.bpm().is_none());

        // Override clears.
        let cleared = with.with_bpm(None);
        assert!(cleared.bpm().is_none());
    }

    proptest! {
        #[test]
        fn frame_never_panics(
            samples in proptest::collection::vec(-1.0f32..=1.0, 0..1024),
            channels in 1u8..=2,
            sample_rate in 8_000u32..=192_000,
            idx in 0usize..1_000_000,
        ) {
            // Trim samples to a multiple of `channels`.
            let n = samples.len();
            let trimmed = n - (n % usize::from(channels));
            let mut samples = samples;
            samples.truncate(trimmed);

            if let Some(track) = Track::from_interleaved(samples, sample_rate, channels) {
                let _ = track.frame(idx);
            }
        }
    }

    #[test]
    fn read_metadata_on_untagged_wav_returns_empty() {
        // Plain hound-generated WAV has no INFO chunk → all tag
        // slots remain `None`, `is_empty()` is `true`. Guards the
        // "no metadata returns Some(default), not Err" contract.
        let path = std::env::temp_dir().join("dub-io-test-untagged.wav");
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for _ in 0..480 {
                writer.write_sample(0i16).unwrap();
            }
            writer.finalize().unwrap();
        }

        let metadata = super::read_metadata(&path).expect("metadata probe");
        assert!(metadata.is_empty());
        assert!(metadata.title.is_none());
        assert!(metadata.artist.is_none());
        assert!(metadata.album.is_none());

        // Round-trip through Track also exposes None.
        let track = Track::load_from_path(&path).expect("load WAV");
        assert!(track.title().is_none());
        assert!(track.artist().is_none());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_metadata_on_missing_file_returns_err() {
        let bogus = std::env::temp_dir().join("dub-io-test-does-not-exist.wav");
        let result = super::read_metadata(&bogus);
        assert!(matches!(result, Err(LoadError::Io(_))));
    }

    #[test]
    fn loads_a_real_wav_file() {
        // Generate a 0.1 s mono i16 WAV with hound, load it back, check
        // round-trip *and* that samples are properly normalized to f32 in
        // [-1.0, 1.0]. This guards against the (real) bug where a naive
        // cast from i16 → f32 would yield values up to 32767.
        let path = std::env::temp_dir().join("dub-io-test-sine.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..4_800i32 {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / 48_000.0;
                let s = 0.5 * (t * 440.0 * std::f32::consts::TAU).sin();
                #[allow(clippy::cast_possible_truncation)]
                let q = (s * f32::from(i16::MAX)) as i16;
                writer.write_sample(q).unwrap();
            }
            writer.finalize().unwrap();
        }

        let track = Track::load_from_path(&path).expect("load WAV");
        assert_eq!(track.sample_rate(), 48_000);
        assert_eq!(track.channels(), 1);
        assert_eq!(track.frames(), 4_800);

        let peak = track
            .samples()
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0f32, f32::max);
        assert!(
            peak < 1.0,
            "peak should be < 1.0 (was {peak}); decoder likely failed to normalize"
        );
        assert!(
            peak > 0.4 && peak < 0.55,
            "peak should be ~0.5 (was {peak})"
        );

        std::fs::remove_file(&path).ok();
    }

    /// Write a `secs`-long stereo 16-bit WAV of two distinct sine
    /// tones (L 440 Hz / R 660 Hz) for streaming-load tests.
    fn write_stereo_wav(path: &Path, secs: f64) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = (secs * 48_000.0) as u32;
        for i in 0..frames {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 48_000.0;
            for freq in [440.0f32, 660.0] {
                let s = 0.4 * (t * freq * std::f32::consts::TAU).sin();
                #[allow(clippy::cast_possible_truncation)]
                let q = (s * f32::from(i16::MAX)) as i16;
                writer.write_sample(q).unwrap();
            }
        }
        writer.finalize().unwrap();
    }

    #[test]
    // Strict float equality is the point: streaming and one-shot
    // decodes must be BYTE-identical, not merely close.
    #[allow(clippy::float_cmp)]
    fn streaming_load_is_byte_identical_to_one_shot() {
        // The RT-equivalence contract: once a streaming load
        // finishes, every frame read must match the one-shot decode
        // exactly — same bytes, same length, same duration.
        let path = std::env::temp_dir().join("dub-io-test-streaming-eq.wav");
        write_stereo_wav(&path, 2.0);

        let one_shot = Track::load_from_path(&path).expect("one-shot load");
        let mut streaming = Track::begin_streaming(&path).expect("begin streaming");
        streaming.decode_until_secs(0.25).expect("head decode");
        let track = streaming.track();

        // Head phase: declared length visible immediately, watermark
        // trails it, positions past the watermark render silence.
        assert_eq!(track.frames(), one_shot.frames(), "declared length");
        assert!(track.decoded_frames() >= 12_000, "head must be audible");
        assert!(
            track.decoded_frames() < track.frames(),
            "watermark must trail the declared length after a head decode"
        );
        assert!(!track.is_fully_decoded());
        assert_eq!(
            track.frame(track.frames() - 1),
            [0.0, 0.0],
            "past-watermark reads must be silence"
        );
        assert!(
            track.samples().is_empty(),
            "streaming tracks are not borrowable as a slice"
        );
        // The decoded head is already byte-identical.
        for idx in [0usize, 1, 4_800, 11_999] {
            assert_eq!(track.frame(idx), one_shot.frame(idx), "head frame {idx}");
        }

        streaming.finish().expect("tail decode");

        assert!(track.is_fully_decoded());
        assert_eq!(track.decoded_frames(), one_shot.frames());
        for idx in (0..one_shot.frames()).step_by(997) {
            assert_eq!(track.frame(idx), one_shot.frame(idx), "frame {idx}");
        }
        assert_eq!(track.frame(one_shot.frames()), [0.0, 0.0]);
        assert_eq!(
            track.samples_to_vec(),
            one_shot.samples(),
            "copied-out samples must match the one-shot buffer"
        );
        assert!((track.duration_seconds() - one_shot.duration_seconds()).abs() < 1e-12);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    // Strict float equality is the point — published frames must be
    // final, not approximately final.
    #[allow(clippy::float_cmp)]
    fn streaming_tail_decodes_concurrently_with_reads() {
        // The deck-load pattern: head on the caller thread, tail on
        // a worker, readers polling the shared track throughout. The
        // watermark must only grow, and every frame below it must
        // already carry final data.
        let path = std::env::temp_dir().join("dub-io-test-streaming-conc.wav");
        write_stereo_wav(&path, 2.0);

        let one_shot = Track::load_from_path(&path).expect("one-shot load");
        let mut streaming = Track::begin_streaming(&path).expect("begin streaming");
        streaming.decode_until_secs(0.1).expect("head decode");
        let track = streaming.track();

        let worker = std::thread::spawn(move || streaming.finish());

        let mut last_watermark = 0usize;
        while !track.is_fully_decoded() {
            let watermark = track.decoded_frames();
            assert!(watermark >= last_watermark, "watermark must be monotonic");
            if watermark > 0 {
                let idx = watermark - 1;
                assert_eq!(
                    track.frame(idx),
                    one_shot.frame(idx),
                    "published frames must be final"
                );
            }
            last_watermark = watermark;
            std::thread::yield_now();
        }
        worker.join().unwrap().expect("tail decode");
        assert_eq!(track.decoded_frames(), one_shot.frames());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn streaming_short_file_finishes_during_head_decode() {
        // A file shorter than the requested head: decode_until hits
        // EOF, finalizes, and finish() is a no-op.
        let path = std::env::temp_dir().join("dub-io-test-streaming-short.wav");
        write_stereo_wav(&path, 0.05);

        let mut streaming = Track::begin_streaming(&path).expect("begin streaming");
        streaming.decode_until_secs(4.0).expect("head decode");
        assert!(streaming.is_done());
        let track = streaming.track();
        assert!(track.is_fully_decoded());
        assert_eq!(track.frames(), 2_400);
        streaming.finish().expect("finish after EOF is a no-op");

        std::fs::remove_file(&path).ok();
    }
}
