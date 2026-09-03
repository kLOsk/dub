//! Main-thread proxy for the audio engine.
//!
//! The audio thread owns the engine and decks. The main thread owns this
//! handle. Communication is one-way for mutations (ringbuf SPSC) and
//! read-only via atomic snapshots in the other direction. No locks, no
//! allocations on either side once the channel is created.
//!
//! ```text
//!  Main thread              ringbuf<Command>             Audio thread
//!  EngineHandle      ─────  try_push  ──────────►        Engine
//!     │                                                   │
//!     │             ◄────  Arc<DeckSharedState>  ────►    │
//!     └─ deck(i).position_frames()    atomic snapshot     └─ deck.render()
//! ```
//!
//! API is split deck-by-deck for ergonomics: `handle.deck(0).play()`,
//! `handle.deck(0).seek(48_000.0)`, etc. Each method returns
//! `Result<(), CommandError>` because the channel is bounded — if the audio
//! thread is gone or the buffer is full, the caller learns synchronously.

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::command::Command;
use crate::deck::DeckSharedState;
use crate::thru::{ThruAttachError, ThruInputConfig, ThruSource};
use crate::timecode::{AttachError as TimecodeAttachError, TimecodeInput, TimecodeInputConfig};
use crate::DECK_COUNT;
use dub_io::Track;

/// Capacity of the command channel. Sized for far more than any plausible
/// burst of human input — at 60 Hz UI updates a deck would have to send a
/// command on every frame for ~4 seconds to fill this. Real bursts are
/// dozens of commands at most (a complex performance gesture).
const COMMAND_CHANNEL_CAPACITY: usize = 256;

/// Capacity of the trash channel that ferries old `Arc<Track>` from the
/// audio thread back to the main thread for disposal. Sized so that a
/// reasonable UI (calling `reclaim()` at ≥ 1 Hz) never approaches it.
/// 32 means the user would have to perform 32 untracked load operations
/// without the UI ever calling `reclaim()` to overflow — a programming
/// error that we surface via the overflow counter rather than a silent
/// memory leak or a forbidden audio-thread `dealloc`.
const TRASH_CHANNEL_CAPACITY: usize = 32;

/// Capacity of the timecode-input trash channel (M5.4.5). Smaller than
/// the track trash because [`Box<TimecodeInput>`] is only displaced on
/// mid-stream re-attach (cartridge swap) — at most a handful per
/// session, never bursty. 8 is more than the absolute worst case (one
/// re-cal per deck per song over a tight set, well below half this) and
/// a hard programming-error backstop for anything beyond that.
const TIMECODE_TRASH_CHANNEL_CAPACITY: usize = 8;

/// Capacity of the thru-source trash channel (M7). Same sizing logic
/// as [`TIMECODE_TRASH_CHANNEL_CAPACITY`]: [`Box<ThruSource>`] is only
/// displaced on mid-stream re-attach (operator switches input pairs
/// or swaps cartridges on a Thru deck) — a handful per session at
/// most. 8 is the same hard backstop for programming errors.
const THRU_TRASH_CHANNEL_CAPACITY: usize = 8;

/// Seconds of mono audio the M8 BPM tee ring buffers. Sized so that
/// the analysis thread's 20 ms poll cadence — even with worst-case
/// OS scheduling jitter — never lets the ring overflow on a
/// healthy system. 1 s × 48 kHz × 4 bytes = 192 KB per deck, which
/// is well within budget.
pub const BPM_TEE_RING_CAPACITY_SECS: usize = 1;

/// Seconds of mono audio the M9 peaks tap ring buffers. Same
/// sizing rationale as [`BPM_TEE_RING_CAPACITY_SECS`]: the
/// decimator thread polls every 20 ms; 1 s of slack absorbs any
/// scheduling jitter. 192 KB per deck.
pub const PEAKS_TAP_RING_CAPACITY_SECS: usize = 1;

/// Seconds of **stereo** audio the M26 record tap ring buffers.
/// Larger than the mono telemetry rings because a dropped sample
/// here is a hole in the rip archive, not a telemetry blip: the
/// capture worker also does file I/O on its poll cadence, so a
/// filesystem stall (external-drive spin-up, Spotlight burst) must
/// be absorbable. 4 s × 48 kHz × 2 ch × 4 B ≈ 1.5 MB per session —
/// only ever allocated while a rip is armed.
pub const RECORD_TAP_RING_CAPACITY_SECS: usize = 4;

/// Errors from
/// [`EngineHandle::attach_thru_source_with_bpm_tracking`]. Carries
/// either an underlying `ThruAttachError`, a BPM tracker
/// configuration error, or a sample-rate mismatch between the
/// tracker config and the engine.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ThruAttachWithBpmError {
    #[error(transparent)]
    Thru(#[from] ThruAttachError),
    #[error("bpm tracker config rejected: {0}")]
    BadTrackerConfig(#[from] dub_bpm::TrackerError),
    #[error("bpm tracker sample rate {tracker_sr} Hz != engine SR {engine_sr} Hz")]
    SampleRateMismatch { tracker_sr: u32, engine_sr: u32 },
}

/// Errors from
/// [`EngineHandle::attach_thru_source_with_peaks_tracking`]. Carries
/// either an underlying `ThruAttachError`, a peak-stream
/// configuration error, or a sample-rate mismatch between the
/// peak-stream config and the engine.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ThruAttachWithPeaksError {
    #[error(transparent)]
    Thru(#[from] ThruAttachError),
    #[error("peak stream config rejected: {0}")]
    BadPeaksConfig(#[from] dub_peaks::PeakStreamError),
    #[error("peak stream sample rate {peaks_sr} Hz != engine SR {engine_sr} Hz")]
    SampleRateMismatch { peaks_sr: u32, engine_sr: u32 },
}

/// Errors from
/// [`EngineHandle::attach_thru_source_with_telemetry`]. Combined
/// surface that can carry any of the M8 (BPM) or M9 (peaks) errors.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum ThruAttachWithTelemetryError {
    #[error(transparent)]
    Thru(#[from] ThruAttachError),
    #[error("bpm tracker config rejected: {0}")]
    BadTrackerConfig(#[from] dub_bpm::TrackerError),
    #[error("bpm tracker sample rate {tracker_sr} Hz != engine SR {engine_sr} Hz")]
    BpmSampleRateMismatch { tracker_sr: u32, engine_sr: u32 },
    #[error("peak stream config rejected: {0}")]
    BadPeaksConfig(#[from] dub_peaks::PeakStreamError),
    #[error("peak stream sample rate {peaks_sr} Hz != engine SR {engine_sr} Hz")]
    PeaksSampleRateMismatch { peaks_sr: u32, engine_sr: u32 },
}

/// Which taps [`EngineHandle::attach_thru_source_with_taps`] wires
/// onto the [`ThruSource`] (M26). The M8/M9 convenience variants
/// cover the common telemetry combinations; this struct exists so
/// adding a tap doesn't mean another multiplicative attach variant.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThruTaps {
    /// Spawn an M8 BPM analysis thread fed by the mono tee.
    pub bpm: Option<dub_bpm::TrackerConfig>,
    /// Spawn an M9 peak-capture decimator fed by the mono tap.
    pub peaks: Option<dub_peaks::PeakStreamConfig>,
    /// Wire the M26 full-quality stereo record tap and hand its
    /// consumer end back for `dub-rip`'s capture worker.
    pub record: bool,
}

/// What [`EngineHandle::attach_thru_source_with_taps`] hands back:
/// one handle per requested tap, `None` for taps that weren't
/// requested.
///
/// `Debug` is hand-rolled to presence flags — the stream handles
/// own live threads and have no useful `Debug` of their own.
pub struct ThruTapHandles {
    /// M8 BPM stream. Owns its analysis thread; drop or `shutdown()`
    /// to stop tracking.
    pub bpm: Option<dub_bpm::BpmStream>,
    /// M9 peak stream. Owns its decimator thread.
    pub peaks: Option<dub_peaks::PeakStream>,
    /// Consumer end of the stereo record ring. The engine knows
    /// nothing about ripping — the caller hands this to the
    /// `dub-rip` capture worker (same ownership inversion as
    /// `AudioInput::take_consumer_pair`).
    pub record_rx: Option<ringbuf::HeapCons<f32>>,
}

impl std::fmt::Debug for ThruTapHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThruTapHandles")
            .field("bpm", &self.bpm.is_some())
            .field("peaks", &self.peaks.is_some())
            .field("record_rx", &self.record_rx.is_some())
            .finish()
    }
}

/// Errors that can occur sending a command from the UI thread.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// The audio thread is no longer draining commands (channel full).
    /// Indicates either a stuck audio thread or pathological input rate.
    /// Recoverable: caller may retry after the next render block.
    #[error("audio engine command channel is full")]
    ChannelFull,

    /// The deck index is out of range. Indicates a UI bug — log and
    /// ignore.
    #[error("deck index {idx} out of range (have {count})")]
    InvalidDeck { idx: u8, count: u8 },
}

/// Main-thread handle to control a running [`crate::Engine`].
///
/// Drop semantics: dropping the handle drops the producer end of the
/// command channel and the consumer end of the trash channel. Any
/// `Arc<Track>` left in the trash buffer is dropped here on the main
/// thread when this struct is dropped — *not* on the audio thread.
pub struct EngineHandle {
    tx: HeapProd<Command>,
    trash_rx: HeapCons<Arc<Track>>,
    /// Main-thread receive end of the M5.4.5 timecode-input trash
    /// channel. The audio thread sends displaced `Box<TimecodeInput>`
    /// here when an `AttachTimecodeInput` command lands on a slot
    /// that's already filled (mid-stream re-cal). Drained by
    /// [`Self::reclaim`] alongside the track trash.
    timecode_trash_rx: HeapCons<Box<TimecodeInput>>,
    /// Main-thread receive end of the M7 thru-source trash channel.
    /// Symmetric to `timecode_trash_rx`. The audio thread sends
    /// displaced `Box<ThruSource>` here when an `AttachThruSource`
    /// command lands on a slot that's already filled. Drained by
    /// [`Self::reclaim`] alongside the other two trash channels.
    thru_trash_rx: HeapCons<Box<ThruSource>>,
    overflow_counter: Arc<AtomicU64>,
    /// Counter for `Box<TimecodeInput>` instances the audio thread
    /// had to forget because the timecode trash channel was full.
    /// **Should always be zero in correct usage** — see
    /// [`Self::timecode_trash_overflow_count`].
    timecode_trash_overflow: Arc<AtomicU64>,
    /// Counter for `Box<ThruSource>` instances the audio thread had
    /// to forget because the thru-source trash channel was full.
    /// **Should always be zero in correct usage** — see
    /// [`Self::thru_trash_overflow_count`].
    thru_trash_overflow: Arc<AtomicU64>,
    deck_shared: [Arc<DeckSharedState>; DECK_COUNT],
    /// Cached engine sample rate. Used by [`Self::attach_timecode_input`]
    /// and [`Self::attach_thru_source`] to validate the input config
    /// without forcing the caller to re-supply what's already an
    /// engine invariant. Set once at construction; never changes.
    engine_sample_rate: f32,
}

/// Bundle returned to the engine constructor: the rx end of the command
/// channel, the tx ends of all three trash channels, and the shared
/// overflow counters. All owned by `Engine` after construction.
pub(crate) struct EngineSide {
    pub(crate) cmd_rx: HeapCons<Command>,
    pub(crate) trash_tx: HeapProd<Arc<Track>>,
    pub(crate) timecode_trash_tx: HeapProd<Box<TimecodeInput>>,
    pub(crate) thru_trash_tx: HeapProd<Box<ThruSource>>,
    pub(crate) overflow_counter: Arc<AtomicU64>,
    pub(crate) timecode_trash_overflow: Arc<AtomicU64>,
    pub(crate) thru_trash_overflow: Arc<AtomicU64>,
}

impl EngineHandle {
    /// Construct a handle / engine-side bundle. Used by
    /// [`crate::Engine::new_with_handle`]; not part of the public API on
    /// its own because the engine-side bundle is internal.
    #[must_use]
    pub(crate) fn new(
        deck_shared: [Arc<DeckSharedState>; DECK_COUNT],
        engine_sample_rate: f32,
    ) -> (Self, EngineSide) {
        let cmd_buffer = HeapRb::<Command>::new(COMMAND_CHANNEL_CAPACITY);
        let (cmd_tx, cmd_rx) = cmd_buffer.split();
        let trash_buffer = HeapRb::<Arc<Track>>::new(TRASH_CHANNEL_CAPACITY);
        let (trash_producer, trash_consumer) = trash_buffer.split();
        let timecode_trash_buffer =
            HeapRb::<Box<TimecodeInput>>::new(TIMECODE_TRASH_CHANNEL_CAPACITY);
        let (timecode_trash_producer, timecode_trash_consumer) = timecode_trash_buffer.split();
        let thru_trash_buffer = HeapRb::<Box<ThruSource>>::new(THRU_TRASH_CHANNEL_CAPACITY);
        let (thru_trash_producer, thru_trash_consumer) = thru_trash_buffer.split();
        let overflow_counter = Arc::new(AtomicU64::new(0));
        let timecode_trash_overflow = Arc::new(AtomicU64::new(0));
        let thru_trash_overflow = Arc::new(AtomicU64::new(0));
        let handle = Self {
            tx: cmd_tx,
            trash_rx: trash_consumer,
            timecode_trash_rx: timecode_trash_consumer,
            thru_trash_rx: thru_trash_consumer,
            overflow_counter: overflow_counter.clone(),
            timecode_trash_overflow: timecode_trash_overflow.clone(),
            thru_trash_overflow: thru_trash_overflow.clone(),
            deck_shared,
            engine_sample_rate,
        };
        let engine_side = EngineSide {
            cmd_rx,
            trash_tx: trash_producer,
            timecode_trash_tx: timecode_trash_producer,
            thru_trash_tx: thru_trash_producer,
            overflow_counter,
            timecode_trash_overflow,
            thru_trash_overflow,
        };
        (handle, engine_side)
    }

    /// Get an ergonomic command builder for the given deck.
    #[must_use]
    pub fn deck(&mut self, idx: usize) -> DeckCommand<'_> {
        DeckCommand { handle: self, idx }
    }

    /// Read-only snapshot of the given deck. Cheap (atomic load).
    #[must_use]
    pub fn deck_state(&self, idx: usize) -> Option<DeckSnapshot> {
        let shared = self.deck_shared.get(idx)?;
        Some(DeckSnapshot {
            position_frames: shared.load_position(),
            is_playing: shared.load_playing(),
            at_end: shared.load_at_end(),
            is_panic_play: shared.load_panic_play(),
            duration_secs: shared.load_duration_secs(),
            has_track: shared.load_has_track(),
        })
    }

    /// Drain anything the audio thread has bounced back through either
    /// trash channel and drop it here on the main thread. Returns the
    /// total number of items reclaimed across both channels.
    ///
    /// Three channels share this entry point because they have
    /// identical drain semantics ("call regularly, items dropped
    /// here") and most callers shouldn't have to know about the
    /// split:
    ///
    /// - **Track trash** — old `Arc<Track>` from `Command::DeckLoad`.
    /// - **TimecodeInput trash (M5.4.5)** — old `Box<TimecodeInput>`
    ///   from `Command::AttachTimecodeInput` lands here when a slot
    ///   was replaced.
    /// - **ThruSource trash (M7)** — old `Box<ThruSource>` from
    ///   `Command::AttachThruSource` lands here when a slot was
    ///   replaced.
    ///
    /// Call regularly from the UI (e.g. once per UI frame, or before
    /// every track-load / timecode-attach / thru-attach attempt). It
    /// is also called automatically by [`DeckCommand::load`],
    /// [`Self::attach_timecode_input`], and
    /// [`Self::attach_thru_source`] before sending so purely UI-
    /// driven workflows are safe without explicit calls.
    pub fn reclaim(&mut self) -> usize {
        let mut n = 0;
        while let Some(_arc) = self.trash_rx.try_pop() {
            n += 1;
        }
        while let Some(_boxed) = self.timecode_trash_rx.try_pop() {
            n += 1;
        }
        while let Some(_boxed) = self.thru_trash_rx.try_pop() {
            n += 1;
        }
        n
    }

    /// Total number of times the audio thread had to forget an old
    /// `Arc<Track>` because the track trash channel was full when a
    /// new track was loaded. **Should always be zero in correct
    /// usage.** Non-zero values mean memory has been leaked and the
    /// UI is not calling [`reclaim`](Self::reclaim) frequently enough.
    #[must_use]
    pub fn trash_overflow_count(&self) -> u64 {
        self.overflow_counter.load(Ordering::Relaxed)
    }

    /// Total number of times the audio thread had to forget an old
    /// `Box<TimecodeInput>` because the timecode-input trash channel
    /// was full when a new timecode input was attached (M5.4.5).
    /// **Should always be zero in correct usage.** Non-zero only if
    /// re-calibration commands are issued faster than [`Self::reclaim`]
    /// can drain — implausibly fast given a single re-cal already
    /// takes seconds.
    #[must_use]
    pub fn timecode_trash_overflow_count(&self) -> u64 {
        self.timecode_trash_overflow.load(Ordering::Relaxed)
    }

    /// Total number of times the audio thread had to forget an old
    /// `Box<ThruSource>` because the thru-source trash channel was
    /// full when a new Thru source was attached (M7).
    /// **Should always be zero in correct usage.** Same sizing
    /// argument as the timecode trash: re-attaching a Thru source
    /// is a session-scale event, far below the 8-slot capacity.
    #[must_use]
    pub fn thru_trash_overflow_count(&self) -> u64 {
        self.thru_trash_overflow.load(Ordering::Relaxed)
    }

    /// Set the engine-wide master gain on the debug internal mixer.
    /// `1.0` is unity. PRD §5.3: external-mixer mode (M5+) bypasses the
    /// master, so this command is a no-op there; for v1's debug mixer
    /// it scales the summed-stereo bus.
    ///
    /// # Errors
    /// [`CommandError::ChannelFull`] if the audio thread is not draining
    /// (recoverable; retry on the next render block).
    pub fn set_master_gain(&mut self, gain: f32) -> Result<(), CommandError> {
        self.send(Command::SetMasterGain { gain })
    }

    /// Bind a sample to sampler slot `slot` (M17, PRD §7.1).
    ///
    /// `source` must already be at the engine's sample rate — convert
    /// with `dub_io::resample_track` at bind time. The displaced sample
    /// (if any) comes back through the trash channel.
    ///
    /// # Errors
    /// [`CommandError::ChannelFull`] if the audio thread is not
    /// draining; the `Arc` is handed back with the error so the caller
    /// can retry or drop it on its own thread.
    pub fn sampler_load(
        &mut self,
        slot: usize,
        source: Arc<Track>,
    ) -> Result<(), (CommandError, Arc<Track>)> {
        let Ok(slot) = Self::check_sampler_slot(slot) else {
            return Err((CommandError::InvalidDeck { idx: 0, count: 0 }, source));
        };
        match self.tx.try_push(Command::SamplerLoad { slot, source }) {
            Ok(()) => Ok(()),
            Err(Command::SamplerLoad { source, .. }) => Err((CommandError::ChannelFull, source)),
            Err(_) => unreachable!("try_push returns the command it was given"),
        }
    }

    /// Unbind sampler slot `slot`.
    ///
    /// # Errors
    /// [`CommandError::ChannelFull`] / [`CommandError::InvalidDeck`].
    pub fn sampler_clear(&mut self, slot: usize) -> Result<(), CommandError> {
        let slot = Self::check_sampler_slot(slot)?;
        self.send(Command::SamplerClear { slot })
    }

    /// Fire sampler slot `slot`'s one-shot.
    ///
    /// # Errors
    /// [`CommandError::ChannelFull`] / [`CommandError::InvalidDeck`].
    pub fn sampler_trigger(&mut self, slot: usize) -> Result<(), CommandError> {
        let slot = Self::check_sampler_slot(slot)?;
        self.send(Command::SamplerTrigger { slot })
    }

    /// Stop sampler slot `slot` early, ramping out.
    ///
    /// # Errors
    /// [`CommandError::ChannelFull`] / [`CommandError::InvalidDeck`].
    pub fn sampler_stop(&mut self, slot: usize) -> Result<(), CommandError> {
        let slot = Self::check_sampler_slot(slot)?;
        self.send(Command::SamplerStop { slot })
    }

    /// Set sampler slot `slot`'s linear gain.
    ///
    /// # Errors
    /// [`CommandError::ChannelFull`] / [`CommandError::InvalidDeck`].
    pub fn sampler_set_gain(&mut self, slot: usize, gain: f32) -> Result<(), CommandError> {
        let slot = Self::check_sampler_slot(slot)?;
        self.send(Command::SamplerSetGain { slot, gain })
    }

    /// Choose the deck bus sampler slot `slot` sums onto.
    ///
    /// # Errors
    /// [`CommandError::ChannelFull`] / [`CommandError::InvalidDeck`].
    pub fn sampler_set_output_deck(
        &mut self,
        slot: usize,
        deck: usize,
    ) -> Result<(), CommandError> {
        let slot = Self::check_sampler_slot(slot)?;
        let deck = self.check_deck(deck)?;
        self.send(Command::SamplerSetOutputDeck { slot, deck })
    }

    /// Validate a sampler slot index. Reuses [`CommandError::InvalidDeck`]
    /// rather than minting a parallel error: the caller's recovery is the
    /// same either way, and one variant keeps the FFI mapping single.
    fn check_sampler_slot(slot: usize) -> Result<u8, CommandError> {
        if slot < crate::sampler::SAMPLER_SLOTS {
            #[allow(clippy::cast_possible_truncation)]
            return Ok(slot as u8);
        }
        #[allow(clippy::cast_possible_truncation)]
        Err(CommandError::InvalidDeck {
            idx: 0,
            count: crate::sampler::SAMPLER_SLOTS as u8,
        })
    }

    /// Instant Doubles (M17, PRD §7.3): put deck `from`'s track onto
    /// deck `to` at `from`'s current playhead.
    ///
    /// Not a [`DeckCommand`] method because it names two decks; the
    /// duplication itself happens on the audio thread, which is what
    /// makes the alignment sample-accurate (see
    /// [`Command::DeckInstantDouble`]).
    ///
    /// # Errors
    /// [`CommandError::InvalidDeck`] if either index is out of range;
    /// [`CommandError::ChannelFull`] if the audio thread is not
    /// draining (recoverable; retry on the next render block).
    pub fn instant_double(&mut self, from: usize, to: usize) -> Result<(), CommandError> {
        let from = self.check_deck(from)?;
        let to = self.check_deck(to)?;
        self.send(Command::DeckInstantDouble { from, to })
    }

    /// Attach a [`TimecodeInput`] to deck `idx` mid-stream (M5.4.5).
    ///
    /// This is the command-channel counterpart to
    /// [`crate::Engine::attach_timecode_input`] — it has identical
    /// semantics on the audio side, but unlike the engine-side
    /// `&mut self` method it can be called *while audio is running*.
    /// The DJ-takeover use case requires this: the incoming DJ wires
    /// up deck A, audio starts, and deck B's calibrator finishes
    /// minutes later when the previous DJ finally hands over the
    /// turntable; the calibrator's result attaches mid-stream
    /// without disturbing deck A.
    ///
    /// The [`TimecodeInput`] is constructed on this (main) thread —
    /// `TimecodeInput::new` allocates a scratch buffer and a decoder
    /// — and boxed so the [`Command`] enum stays an 8-byte payload.
    /// The audio thread takes the box out of the command and slots
    /// it into `engine.timecode_inputs[idx]`. Any displaced
    /// `TimecodeInput` (re-attach to a filled slot) is bounced back
    /// through the timecode-input trash channel for disposal here on
    /// the main thread — never dropped on the audio thread.
    ///
    /// Auto-drains both trash channels before sending so the user
    /// never has to call [`Self::reclaim`] manually for the attach
    /// path to stay safe.
    ///
    /// # Errors
    /// - [`TimecodeAttachError::InvalidDeck`] if `idx >= DECK_COUNT`.
    /// - One of [`TimecodeAttachError::SampleRateMismatch`],
    ///   [`TimecodeAttachError::BadBlockSize`],
    ///   [`TimecodeAttachError::InvalidHysteresis`], or
    ///   [`TimecodeAttachError::InvalidAmplitudeThreshold`] if
    ///   `config.validate` rejects.
    /// - [`TimecodeAttachError::ChannelFull`] (M5.4.5) if the SPSC
    ///   command channel is full. Recoverable; retry next frame.
    ///
    /// On success the returned `Ok(())` does **not** mean the
    /// attach has taken effect — only that the command is enqueued.
    /// The audio thread applies it on its next block (~5–10 ms at
    /// 48 kHz / 256 frames). Observability: poll
    /// [`crate::Engine::timecode_last_output`] (via shared snapshot
    /// in M10) to see when decoding starts on the new deck.
    pub fn attach_timecode_input(
        &mut self,
        idx: usize,
        rx: ringbuf::HeapCons<f32>,
        config: TimecodeInputConfig,
    ) -> Result<(), TimecodeAttachError> {
        if idx >= DECK_COUNT {
            return Err(TimecodeAttachError::InvalidDeck {
                idx,
                count: DECK_COUNT,
            });
        }
        config.validate(self.engine_sample_rate)?;
        // Drain trash before sending so the timecode-trash ring stays
        // available for the audio thread to push the displaced
        // (if any) box back through.
        self.reclaim();

        // u8 narrowing is safe — DECK_COUNT is 2 today and capped
        // far below 256 by the array sizing convention.
        #[allow(clippy::cast_possible_truncation)]
        let idx_u8 = idx as u8;
        let input = Box::new(TimecodeInput::new(rx, config));
        self.tx
            .try_push(Command::AttachTimecodeInput { idx: idx_u8, input })
            .map_err(|_cmd| TimecodeAttachError::ChannelFull)
    }

    /// Attach a [`ThruSource`] to deck `idx` mid-stream (M7).
    ///
    /// Command-channel counterpart to
    /// [`crate::Engine::attach_thru_source`]. Lets the operator wire
    /// up a Thru deck while the engine is already running — the M7
    /// CLI uses this from `dub thru` at startup (after
    /// `AudioOutput::start` consumes the engine, the engine-side
    /// `&mut` method is no longer reachable) and the future M15+ FX
    /// subsystem may use it for runtime input reconfiguration.
    ///
    /// The [`ThruSource`] is constructed on this (main) thread —
    /// [`crate::ThruSource::new`] allocates a scratch buffer — and
    /// boxed so the [`Command`] enum stays an 8-byte payload. The
    /// audio thread takes the box out of the command and slots it
    /// into `engine.thru_sources[idx]`. Any displaced
    /// [`ThruSource`] (re-attach to a filled slot) is bounced back
    /// through the thru-source trash channel for disposal here on
    /// the main thread — never dropped on the audio thread.
    ///
    /// Internally builds the [`ThruSource`] from
    /// `(rx, config, engine_declick_envelope)` so the caller doesn't
    /// have to thread the envelope through. The engine sample rate
    /// cached in the handle drives config validation; mismatches are
    /// caught here (off-RT) and reported as
    /// [`ThruAttachError::SampleRateMismatch`].
    ///
    /// Auto-drains all three trash channels before sending so the
    /// thru-source trash ring has space to absorb any displaced
    /// predecessor on the audio side.
    ///
    /// # Errors
    /// - [`ThruAttachError::InvalidDeck`] if `idx >= DECK_COUNT`.
    /// - [`ThruAttachError::SampleRateMismatch`] / `BadBlockSize` if
    ///   `config.validate` rejects.
    /// - [`ThruAttachError::ChannelFull`] if the SPSC command channel
    ///   is full. Recoverable; retry next frame.
    ///
    /// On success the returned `Ok(())` does **not** mean the attach
    /// has taken effect — only that the command is enqueued. The
    /// audio thread applies it on its next render block (~5–10 ms at
    /// 48 kHz / 256 frames).
    pub fn attach_thru_source(
        &mut self,
        idx: usize,
        rx: ringbuf::HeapCons<f32>,
        config: ThruInputConfig,
    ) -> Result<(), ThruAttachError> {
        self.attach_thru_source_inner(idx, rx, config, None, None, None)
    }

    /// Shared trunk of every thru-attach variant: deck check, config
    /// validation, trash reclaim, [`ThruSource`] construction with
    /// whichever tap producers are handed in, command push. Keeping
    /// exactly one push path means a new tap can never fork the
    /// attach semantics.
    fn attach_thru_source_inner(
        &mut self,
        idx: usize,
        rx: ringbuf::HeapCons<f32>,
        config: ThruInputConfig,
        bpm_tx: Option<ringbuf::HeapProd<f32>>,
        peaks_tx: Option<ringbuf::HeapProd<f32>>,
        record_tx: Option<ringbuf::HeapProd<f32>>,
    ) -> Result<(), ThruAttachError> {
        if idx >= DECK_COUNT {
            return Err(ThruAttachError::InvalidDeck {
                idx,
                count: DECK_COUNT,
            });
        }
        config.validate(self.engine_sample_rate)?;
        // Drain trash before sending so the thru-trash ring stays
        // available for the audio thread to push any displaced
        // predecessor back through.
        self.reclaim();

        let mut source = ThruSource::new(rx, config);
        if let Some(tx) = bpm_tx {
            source = source.with_bpm_tee(tx, config.max_block_frames);
        }
        if let Some(tx) = peaks_tx {
            source = source.with_peaks_tap(tx);
        }
        if let Some(tx) = record_tx {
            source = source.with_record_tap(tx);
        }

        #[allow(clippy::cast_possible_truncation)]
        let idx_u8 = idx as u8;
        self.tx
            .try_push(Command::AttachThruSource {
                idx: idx_u8,
                source: Box::new(source),
            })
            .map_err(|_cmd| ThruAttachError::ChannelFull)
    }

    /// Attach a [`ThruSource`] **and** spawn an M8 BPM analysis
    /// thread for it. Convenience wrapper over [`attach_thru_source`]
    /// that wires the mono-downmix tee + analysis worker in one
    /// call.
    ///
    /// Returns a [`dub_bpm::BpmStream`] handle that the caller polls
    /// for [`dub_bpm::TrackerEvent`]s. The handle owns the worker
    /// thread; drop it (or call `shutdown()`) when detaching the
    /// Thru source to stop tracking — there is no automatic linkage
    /// from engine detach to thread teardown (the engine doesn't
    /// know about the handle).
    ///
    /// The BPM tee ring is sized to [`BPM_TEE_RING_CAPACITY_SECS`]
    /// seconds of mono audio at the engine sample rate, so a
    /// brief analysis-thread stall (GC pause on the UI side, OS
    /// scheduler hiccup) won't lose samples.
    ///
    /// `tracker` should usually be [`dub_bpm::TrackerConfig::at`]
    /// with `channels = 1` (the audio thread already downmixes —
    /// see [`ThruSource::with_bpm_tee`]).
    ///
    /// # Errors
    ///
    /// - Forwards [`ThruAttachError`] from the underlying attach
    ///   (deck index out of range, SR mismatch, channel full, …).
    /// - [`ThruAttachWithBpmError::BadTrackerConfig`] if the BPM
    ///   tracker config is invalid (zero sample rate, bad channel
    ///   count, zero analysis period).
    /// - [`ThruAttachWithBpmError::SampleRateMismatch`] if the BPM
    ///   tracker sample rate doesn't match the engine sample rate.
    ///
    /// # Errors on partial failure
    ///
    /// If the underlying [`attach_thru_source`] succeeds but
    /// `BpmStream::spawn` then fails (extremely rare — the OS
    /// would have to refuse a thread), the engine ends up with a
    /// Thru source attached but no analysis thread. The error is
    /// returned so the caller can decide whether to detach.
    pub fn attach_thru_source_with_bpm_tracking(
        &mut self,
        idx: usize,
        rx: ringbuf::HeapCons<f32>,
        config: ThruInputConfig,
        tracker: dub_bpm::TrackerConfig,
    ) -> Result<dub_bpm::BpmStream, ThruAttachWithBpmError> {
        if idx >= DECK_COUNT {
            return Err(ThruAttachError::InvalidDeck {
                idx,
                count: DECK_COUNT,
            }
            .into());
        }
        config.validate(self.engine_sample_rate)?;

        // Validate the tracker SR matches the engine SR. The
        // BpmTracker accepts any positive SR (it'll happily analyze
        // off-rate samples) so we have to gate it here.
        let tracker_sr = tracker.sample_rate;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let engine_sr_u32 = self.engine_sample_rate as u32;
        if tracker_sr != engine_sr_u32 {
            return Err(ThruAttachWithBpmError::SampleRateMismatch {
                tracker_sr,
                engine_sr: engine_sr_u32,
            });
        }

        // Tee ring sized to 1 second of mono audio. Power-of-two
        // round-up keeps HeapRb happy; the exact size isn't
        // load-bearing — the analysis thread polls every 20 ms so
        // even at coarsest scheduling it consumes ≥ 50 chunks per
        // ring-full.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let tee_capacity = (self.engine_sample_rate as usize)
            .saturating_mul(BPM_TEE_RING_CAPACITY_SECS)
            .max(1024);
        let tee_rb = ringbuf::HeapRb::<f32>::new(tee_capacity);
        let (bpm_tx, bpm_rx) = ringbuf::traits::Split::split(tee_rb);

        self.attach_thru_source_inner(idx, rx, config, Some(bpm_tx), None, None)?;

        let stream = dub_bpm::BpmStream::spawn(bpm_rx, tracker)
            .map_err(ThruAttachWithBpmError::BadTrackerConfig)?;
        Ok(stream)
    }

    /// Attach a [`ThruSource`] **and** spawn an M9 peak-capture
    /// decimator thread for it. Convenience wrapper over
    /// [`attach_thru_source`] that wires the mono-downmix tap + the
    /// off-RT decimator in one call.
    ///
    /// Returns a [`dub_peaks::PeakStream`] handle whose
    /// [`dub_peaks::PeakStream::buffer`] is the live, lock-free
    /// peak buffer M10 will render and CLI tools can dump.
    ///
    /// The peaks tap ring is sized to [`PEAKS_TAP_RING_CAPACITY_SECS`]
    /// seconds at the engine sample rate — a brief decimator stall
    /// (GC pause, OS scheduler hiccup) cannot lose samples on any
    /// healthy system.
    ///
    /// # Errors
    ///
    /// * Forwards [`ThruAttachError`] from the underlying attach.
    /// * [`ThruAttachWithPeaksError::BadPeaksConfig`] if the peak
    ///   stream config is invalid.
    /// * [`ThruAttachWithPeaksError::SampleRateMismatch`] if the
    ///   peak stream sample rate doesn't match the engine sample
    ///   rate.
    ///
    /// # Errors on partial failure
    ///
    /// If [`attach_thru_source`] succeeds but `PeakStream::spawn`
    /// fails (extremely rare — the OS would have to refuse a
    /// thread), the engine ends up with a Thru source attached but
    /// no decimator thread. The error is returned so the caller can
    /// decide whether to detach.
    ///
    /// [`attach_thru_source`]: Self::attach_thru_source
    pub fn attach_thru_source_with_peaks_tracking(
        &mut self,
        idx: usize,
        rx: ringbuf::HeapCons<f32>,
        config: ThruInputConfig,
        peaks_cfg: dub_peaks::PeakStreamConfig,
    ) -> Result<dub_peaks::PeakStream, ThruAttachWithPeaksError> {
        if idx >= DECK_COUNT {
            return Err(ThruAttachError::InvalidDeck {
                idx,
                count: DECK_COUNT,
            }
            .into());
        }
        config.validate(self.engine_sample_rate)?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let engine_sr_u32 = self.engine_sample_rate as u32;
        if peaks_cfg.sample_rate != engine_sr_u32 {
            return Err(ThruAttachWithPeaksError::SampleRateMismatch {
                peaks_sr: peaks_cfg.sample_rate,
                engine_sr: engine_sr_u32,
            });
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let tap_capacity = (self.engine_sample_rate as usize)
            .saturating_mul(PEAKS_TAP_RING_CAPACITY_SECS)
            .max(1024);
        let tap_rb = ringbuf::HeapRb::<f32>::new(tap_capacity);
        let (peaks_tx, peaks_rx) = ringbuf::traits::Split::split(tap_rb);

        self.attach_thru_source_inner(idx, rx, config, None, Some(peaks_tx), None)?;

        let stream = dub_peaks::PeakStream::spawn(peaks_rx, peaks_cfg)
            .map_err(ThruAttachWithPeaksError::BadPeaksConfig)?;
        Ok(stream)
    }

    /// Attach a [`ThruSource`] with **both** M8 BPM tracking and M9
    /// peak capture in one call. Wires a single mono-downmix pass
    /// on the audio thread feeding two off-RT analysis threads —
    /// strictly cheaper than calling the BPM- and peaks-only
    /// variants in sequence (the M8/M9 variants each instantiate a
    /// `ThruSource`; combining them gives one source with both
    /// taps).
    ///
    /// Returns `(BpmStream, PeakStream)`. Drop the tuple to stop
    /// both analysis threads.
    ///
    /// # Errors
    ///
    /// Combined surface from both subsystems (see
    /// [`ThruAttachWithTelemetryError`]).
    ///
    /// # Errors on partial failure
    ///
    /// The attach happens first; if either stream spawn fails after
    /// that, the engine is left with a Thru source attached but
    /// missing one or both analysis threads. The error indicates
    /// which subsystem failed.
    #[allow(clippy::missing_panics_doc, clippy::similar_names)]
    pub fn attach_thru_source_with_telemetry(
        &mut self,
        idx: usize,
        rx: ringbuf::HeapCons<f32>,
        config: ThruInputConfig,
        tracker: dub_bpm::TrackerConfig,
        peaks_cfg: dub_peaks::PeakStreamConfig,
    ) -> Result<(dub_bpm::BpmStream, dub_peaks::PeakStream), ThruAttachWithTelemetryError> {
        let mut handles = self.attach_thru_source_with_taps(
            idx,
            rx,
            config,
            ThruTaps {
                bpm: Some(tracker),
                peaks: Some(peaks_cfg),
                record: false,
            },
        )?;
        let bpm_stream = handles
            .bpm
            .take()
            .expect("bpm stream present: requested in taps");
        let peaks_stream = handles
            .peaks
            .take()
            .expect("peaks stream present: requested in taps");
        Ok((bpm_stream, peaks_stream))
    }

    /// Attach a [`ThruSource`] with any combination of taps (M26):
    /// M8 BPM tracking, M9 peak capture, and/or the M26 stereo
    /// record tap for vinyl ripping. One [`ThruSource`], one
    /// command push, one handle per requested tap — see
    /// [`ThruTaps`] / [`ThruTapHandles`].
    ///
    /// The record ring is sized to [`RECORD_TAP_RING_CAPACITY_SECS`]
    /// seconds of stereo audio so the `dub-rip` capture worker's
    /// file I/O can stall briefly without losing rip samples.
    ///
    /// # Errors
    ///
    /// Combined surface (see [`ThruAttachWithTelemetryError`]);
    /// subsystem errors are only possible for taps that were
    /// requested.
    ///
    /// # Errors on partial failure
    ///
    /// The attach happens first; if a stream spawn fails after that
    /// (the OS would have to refuse a thread), the engine is left
    /// with a Thru source attached but missing that analysis
    /// thread. The error indicates which subsystem failed.
    #[allow(clippy::similar_names)]
    pub fn attach_thru_source_with_taps(
        &mut self,
        idx: usize,
        rx: ringbuf::HeapCons<f32>,
        config: ThruInputConfig,
        taps: ThruTaps,
    ) -> Result<ThruTapHandles, ThruAttachWithTelemetryError> {
        if idx >= DECK_COUNT {
            return Err(ThruAttachError::InvalidDeck {
                idx,
                count: DECK_COUNT,
            }
            .into());
        }
        config.validate(self.engine_sample_rate)?;

        // Subsystem SR gates run before any ring is allocated, in
        // the same bpm-then-peaks order the M8/M9 variants used.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let engine_sr_u32 = self.engine_sample_rate as u32;
        if let Some(tracker) = &taps.bpm {
            if tracker.sample_rate != engine_sr_u32 {
                return Err(ThruAttachWithTelemetryError::BpmSampleRateMismatch {
                    tracker_sr: tracker.sample_rate,
                    engine_sr: engine_sr_u32,
                });
            }
        }
        if let Some(peaks_cfg) = &taps.peaks {
            if peaks_cfg.sample_rate != engine_sr_u32 {
                return Err(ThruAttachWithTelemetryError::PeaksSampleRateMismatch {
                    peaks_sr: peaks_cfg.sample_rate,
                    engine_sr: engine_sr_u32,
                });
            }
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let sr_usize = self.engine_sample_rate as usize;

        let (bpm_tx, bpm_rx) = match &taps.bpm {
            Some(_) => {
                let capacity = sr_usize
                    .saturating_mul(BPM_TEE_RING_CAPACITY_SECS)
                    .max(1024);
                let (tx, rx) = ringbuf::traits::Split::split(ringbuf::HeapRb::<f32>::new(capacity));
                (Some(tx), Some(rx))
            }
            None => (None, None),
        };
        let (peaks_tx, peaks_rx) = match &taps.peaks {
            Some(_) => {
                let capacity = sr_usize
                    .saturating_mul(PEAKS_TAP_RING_CAPACITY_SECS)
                    .max(1024);
                let (tx, rx) = ringbuf::traits::Split::split(ringbuf::HeapRb::<f32>::new(capacity));
                (Some(tx), Some(rx))
            }
            None => (None, None),
        };
        let (record_tx, record_rx) = if taps.record {
            // Stereo: two samples per frame.
            let capacity = sr_usize
                .saturating_mul(2)
                .saturating_mul(RECORD_TAP_RING_CAPACITY_SECS)
                .max(2048);
            let (tx, rx) = ringbuf::traits::Split::split(ringbuf::HeapRb::<f32>::new(capacity));
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        self.attach_thru_source_inner(idx, rx, config, bpm_tx, peaks_tx, record_tx)?;

        let bpm = match (taps.bpm, bpm_rx) {
            (Some(tracker), Some(tee_rx)) => Some(
                dub_bpm::BpmStream::spawn(tee_rx, tracker)
                    .map_err(ThruAttachWithTelemetryError::BadTrackerConfig)?,
            ),
            _ => None,
        };
        let peaks = match (taps.peaks, peaks_rx) {
            (Some(peaks_cfg), Some(tap_rx)) => Some(
                dub_peaks::PeakStream::spawn(tap_rx, peaks_cfg)
                    .map_err(ThruAttachWithTelemetryError::BadPeaksConfig)?,
            ),
            _ => None,
        };

        Ok(ThruTapHandles {
            bpm,
            peaks,
            record_rx,
        })
    }

    fn send(&mut self, cmd: Command) -> Result<(), CommandError> {
        self.tx.try_push(cmd).map_err(|_| CommandError::ChannelFull)
    }

    fn check_deck(&self, idx: usize) -> Result<u8, CommandError> {
        if idx < self.deck_shared.len() {
            #[allow(clippy::cast_possible_truncation)]
            return Ok(idx as u8);
        }
        #[allow(clippy::cast_possible_truncation)]
        Err(CommandError::InvalidDeck {
            idx: idx as u8,
            count: self.deck_shared.len() as u8,
        })
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        // Belt-and-braces: any Arcs still in the trash buffer would be
        // dropped by the ringbuf itself when the consumer goes out of
        // scope (running on this main thread, which is fine), but
        // explicit reclaim makes ordering clear and surfaces overflow
        // diagnostics if the consumer wants to log them.
        self.reclaim();
    }
}

/// Lightweight snapshot of one deck's transport state, suitable for UI
/// rendering. All fields are read with `Relaxed` ordering — consistent
/// per-field but not consistent across fields. That's the right trade-off
/// for a 60 Hz UI: tearing on transport changes is invisible.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeckSnapshot {
    /// Current playhead in track frames.
    pub position_frames: f64,
    /// Whether the deck is currently advancing the playhead.
    pub is_playing: bool,
    /// Whether the playhead is past either end of the loaded track.
    pub at_end: bool,
    /// M10.6b. Whether the deck is currently in Panic-Play state
    /// (PRD §6.1.2) — i.e. running at a held last-known-velocity
    /// rate with timecode-driven transport updates suppressed.
    /// Flips automatically back to `false` when the engine sees a
    /// clean `LiftIntent::Locked` (auto-resume) or the UI issues
    /// `cancel_panic_play`.
    pub is_panic_play: bool,
    /// M11d.6 round 5. Loaded track's wall-clock duration. Zero
    /// when no track is loaded. Published lock-free alongside
    /// the position atomics so the off-main waveform renderer can
    /// synthesise a full `PositionInfo` without consulting the
    /// FFI engine mutex.
    pub duration_secs: f64,
    /// M11d.6 round 5. Mirrors `source.is_some()` from the audio
    /// thread. Pairs with `duration_secs`.
    pub has_track: bool,
}

/// Per-deck command builder. Returned by [`EngineHandle::deck`]; consumes
/// itself on each command so a single call site can only emit one command,
/// preventing accidental fan-out.
pub struct DeckCommand<'h> {
    handle: &'h mut EngineHandle,
    idx: usize,
}

/// Every command-sending method below can fail with the same two errors:
/// [`CommandError::ChannelFull`] (audio thread is not draining fast enough
/// or has stopped) or [`CommandError::InvalidDeck`] (UI sent a bad index).
/// Both are recoverable from the caller's perspective.
impl DeckCommand<'_> {
    /// Start (or resume) playback.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn play(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckPlay { idx })
    }

    /// Pause playback. Position is preserved.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn pause(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckPause { idx })
    }

    /// Move the playhead to the given track-frame position. Negative or
    /// out-of-range values are accepted and silently render silence
    /// (matches a real record being lifted off the platter).
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn seek(self, position_frames: f64) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSeek {
            idx,
            position_frames,
        })
    }

    /// Set the playback rate. `1.0` = forward unity, `-1.0` = reverse
    /// unity, `2.0` = double-speed, `0.0` = stopped.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_rate(self, rate: f64) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetRate { idx, rate })
    }

    /// Enable / disable key lock (master tempo) on this deck (M14).
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_key_lock(self, on: bool) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetKeyLock { idx, on })
    }

    /// Select the time-stretch engine for key lock on this deck (M14 live A/B).
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_stretch_backend(
        self,
        backend: dub_stretch::StretchBackend,
    ) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle
            .send(Command::DeckSetStretchBackend { idx, backend })
    }

    /// Engage a loop over `[in_frames, out_frames)` (track frames).
    /// The grid-snapped reverse-loop region is computed off-RT by the
    /// caller; the deck jumps the playhead into the region if needed
    /// and wraps per block with a seam crossfade.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_loop(self, in_frames: f64, out_frames: f64) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetLoop {
            idx,
            in_frames,
            out_frames,
        })
    }

    /// Disengage any active loop; playback continues forward.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn clear_loop(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckClearLoop { idx })
    }

    /// Engage (or re-trigger) the M15 echo-out FX (PRD §6.3). `delay_frames`
    /// (the echo length on the output bus, at engine sample rate),
    /// `feedback`, and the feedback low-pass coefficient `lp_coeff` are all
    /// resolved off-RT by the caller from the deck's BPM, the chosen beat
    /// division, and the cutoff. Sending again while engaged re-triggers the
    /// capture at the new length.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn engage_echo(
        self,
        delay_frames: u32,
        feedback: f32,
        lp_coeff: f32,
    ) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckEngageEcho {
            idx,
            delay_frames,
            feedback,
            lp_coeff,
        })
    }

    /// Toggle echo-out **off**: the dry signal comes back (the deck has kept
    /// playing underneath, slip-aware) and the wet echo fades out. Idempotent
    /// on a deck that isn't engaged.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn release_echo(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckReleaseEcho { idx })
    }

    /// Live-update echo-out `feedback` and feedback low-pass (`lp_coeff`,
    /// resolved off-RT from the cutoff) while held or idle. The echo length
    /// changes only on a fresh [`Self::engage_echo`].
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_echo_params(self, feedback: f32, lp_coeff: f32) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetEchoParams {
            idx,
            feedback,
            lp_coeff,
        })
    }

    /// Fire M16 dub-siren preset `preset_id` (Simple mode, PRD §6.3) as a tap
    /// one-shot. The preset bank is precomputed off-RT in the engine; this just
    /// carries the index. Out-of-range ids are ignored on the audio thread.
    /// `delay_frames_override` (`0` = use the preset's own slap-back time) is the
    /// beat-matched echo length, resolved off-RT by the caller.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn fire_siren_preset(
        self,
        preset_id: u8,
        delay_frames_override: u32,
    ) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckFireSirenPreset {
            idx,
            preset_id,
            delay_frames_override,
        })
    }

    /// Stop the dub-siren: the oscillator fades out (release ramp) while the
    /// slap-back tail rings on. Idempotent on a non-sounding deck.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn release_siren(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckReleaseSiren { idx })
    }

    /// Set a vintage-FX rack slot on this deck: toggle it `active` and apply its
    /// Advanced super-knob (`macro_value`, 0..1). The macro is RT-safe to apply
    /// (each effect precomputed its transcendental coefficients off-RT), so the
    /// audio thread just fans it across the effect's params.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_rack_fx(
        self,
        slot: crate::FxSlot,
        active: bool,
        macro_value: f32,
    ) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetRackFx {
            idx,
            slot,
            active,
            macro_value,
        })
    }

    /// Set the dub-siren's live controls on this deck: `speed` (HK628 chip
    /// clock), `delay_ms`/`feedback`/`mix`/`filter`/`echo_cut` (onboard PT2399
    /// echo), and `volume` (output). All applied with pure RT-safe setters; the
    /// echo is skipped when `mix` is ~0 and not cut. Both the Advanced super-knob
    /// and Expert knobs route through here.
    ///
    /// # Errors
    /// See impl-level docs.
    #[allow(clippy::too_many_arguments)]
    pub fn set_siren_controls(
        self,
        speed: f32,
        delay_ms: f32,
        feedback: f32,
        mix: f32,
        volume: f32,
        filter: f32,
        echo_cut: bool,
    ) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetSirenControls {
            idx,
            speed,
            delay_ms,
            feedback,
            mix,
            volume,
            filter,
            echo_cut,
        })
    }

    /// Pick which siren unit this deck plays (HK628 shots or the DS01E).
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_siren_unit(self, unit: crate::SirenUnit) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetSirenUnit { idx, unit })
    }

    /// Set the Benidub DS01E voice controls on this deck: `pitch_factor` (PITCH),
    /// `rate_hz` (RATE) and `continuous` (TRIGGER latch). Applied to the MODE
    /// patch at fire time.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_siren_voice(
        self,
        pitch_factor: f32,
        rate_hz: f32,
        continuous: bool,
    ) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetSirenVoice {
            idx,
            pitch_factor,
            rate_hz,
            continuous,
        })
    }

    /// M10.6b Panic-Play engage (PRD §6.1.2). Decouples the deck
    /// from any attached timecode input and pins it to its last-
    /// known good velocity (or unity forward if no policy is
    /// attached / no velocity has been observed). Auto-cancels on a
    /// clean LFSR re-lock; manual-cancels via
    /// [`Self::cancel_panic_play`].
    ///
    /// Sending `panic_play` on a deck that is already in panic
    /// state is harmless — the engine just re-captures the held
    /// rate from `LiftPolicy::last_locked_rate()` and re-pins. The
    /// FFI surface exposes this as a "click once to engage, click
    /// again to cancel" pattern; the engine doesn't enforce that —
    /// the UI does.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn panic_play(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckPanicPlay { idx })
    }

    /// Panic-Play manual cancel (PRD §6.1.2 / M10.6d). Clears the
    /// engaged flag and hands transport authority back to the
    /// timecode driver — does **not** touch `is_playing` / `rate`.
    /// A healthy carrier keeps the deck playing (Serato INT→ABS);
    /// a silent carrier pauses it on the next block via the
    /// existing `DropoutHoldRate` path. Idempotent on decks not in
    /// panic state (engine drops the command silently).
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn cancel_panic_play(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckCancelPanicPlay { idx })
    }

    /// Set the linear deck gain.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_gain(self, gain: f32) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetGain { idx, gain })
    }

    /// Pin this deck's control mode (the Internal/Timecode switch),
    /// suppressing auto source-detection until [`Self::auto_control_mode`].
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn set_control_mode(self, mode: crate::ControlMode) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckSetControlMode { idx, mode })
    }

    /// Release this deck back to automatic source detection.
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn auto_control_mode(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckAutoControlMode { idx })
    }

    /// Start a channel-whitening calibration capture on this deck's
    /// timecode input (manual "Recalibrate").
    ///
    /// # Errors
    /// See impl-level docs.
    pub fn calibrate_timecode(self) -> Result<(), CommandError> {
        let idx = self.handle.check_deck(self.idx)?;
        self.handle.send(Command::DeckCalibrateTimecode { idx })
    }

    /// Hot-load a track onto this deck while the engine is running.
    ///
    /// Auto-drains the trash channel before sending so the user never
    /// has to call [`EngineHandle::reclaim`] manually for the load
    /// path to stay safe.
    ///
    /// On the audio thread this swaps the deck's `Arc<Track>`; the
    /// previous Arc (if any) is sent back through the trash channel
    /// for disposal here on the main thread, never dropped on the
    /// audio thread.
    ///
    /// Position/play state are not implicitly reset — pair this with
    /// [`DeckCommand::seek`] / [`DeckCommand::pause`] to set up the
    /// new track's transport.
    ///
    /// `gain` is the linear deck gain applied atomically with the swap
    /// (auto-gain / loudness normalization). Pass `1.0` for unity when
    /// the track has no stored loudness (unanalyzed / Finder drag); the
    /// engine always overwrites the previous track's gain so a fresh
    /// load never inherits a stale normalization.
    ///
    /// # Errors
    /// On [`CommandError::ChannelFull`] the rejected `Arc<Track>` is
    /// returned back to the caller (cheaper to retry than to re-decode).
    /// On [`CommandError::InvalidDeck`] the Arc is dropped here on the
    /// main thread.
    pub fn load(self, source: Arc<Track>, gain: f32) -> Result<(), (CommandError, Arc<Track>)> {
        let idx = match self.handle.check_deck(self.idx) {
            Ok(idx) => idx,
            Err(e) => return Err((e, source)),
        };
        self.handle.reclaim();
        self.handle
            .tx
            .try_push(Command::DeckLoad { idx, source, gain })
            .map_err(|cmd| match cmd {
                Command::DeckLoad { source, .. } => (CommandError::ChannelFull, source),
                // Unreachable: we just constructed a DeckLoad above.
                other => unreachable!("ringbuf returned wrong Command variant: {other:?}"),
            })
    }

    /// Read the current snapshot for this deck. Cheap (atomic loads).
    #[must_use]
    pub fn snapshot(&self) -> DeckSnapshot {
        // Already validated in deck() that idx is in range, but defend
        // against future API changes.
        self.handle.deck_state(self.idx).unwrap_or(DeckSnapshot {
            position_frames: 0.0,
            is_playing: false,
            at_end: false,
            is_panic_play: false,
            duration_secs: 0.0,
            has_track: false,
        })
    }
}
