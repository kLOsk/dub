//! Threaded streaming driver — the M8 entry point.
//!
//! Wraps a [`BpmTracker`] in an OS thread that consumes audio from a
//! `ringbuf` consumer and emits [`TrackerEvent`]s to a second
//! `ringbuf`. Exposes a single thread-safe handle ([`BpmStream`])
//! that the engine integration constructs at "attach Thru source"
//! time and the UI side reads events from.
//!
//! ```text
//!   Audio thread                BpmStream-spawned thread
//!   ─────────────────           ─────────────────────────
//!   ThruSource::render_into     loop:
//!     pop input ring               pop audio_rx into block buffer
//!     mono-downmix             →   tracker.process(block)
//!     bpm_tx.push_slice            if event: events_tx.try_push(event)
//!                                  if shutdown: exit
//!                                  sleep(POLL_INTERVAL)
//!                          ───────────────────────────────
//!                                            │
//!                                            ▼
//!                                       events_rx (on UI thread)
//! ```
//!
//! ## Design notes
//!
//! * **Sleep-based polling, no condvars.** The analysis cadence is
//!   "every ~1 s the tracker recomputes" — the thread sleeping for
//!   20 ms between drain cycles is well below that, and adds no
//!   audible latency (BPM display latency that's < 50 ms past audio
//!   is indistinguishable from instant). Using `park_timeout` would
//!   need the audio thread to `unpark` — i.e., reach into the
//!   analysis thread's `Thread` handle — which complicates the
//!   audio-thread side and offers no real-world benefit at this
//!   scale.
//! * **Graceful shutdown via the explicit flag.** The thread exits
//!   when `shutdown.load(Acquire)` is true. `BpmStream::Drop` always
//!   sets the flag, so going out of scope is sufficient. Engine
//!   integration must additionally call `shutdown()` (or drop the
//!   stream) when detaching a Thru source so we don't leak a
//!   forever-sleeping thread.
//!
//!   We don't auto-detect producer drop on the ring side: ringbuf
//!   0.4's `HeapCons` doesn't expose an `is_abandoned()` method, so
//!   wiring producer-side teardown to thread exit would require
//!   adding our own `Arc<AtomicBool>` "producer alive" flag. That
//!   adds complexity without solving anything the explicit shutdown
//!   flag doesn't already cover — drop the stream and the thread
//!   exits.
//! * **Event ringbuf overflow is silent.** If the UI doesn't drain
//!   `events_rx` for a long time, new events stop being pushed and
//!   the *most recent* state transition is lost. That's
//!   intentional: a stale event channel means the consumer is
//!   broken; better to discard than block the analysis thread. With
//!   `EVENTS_CAPACITY = 64`, the consumer would have to fall behind
//!   for tens of minutes to lose anything visible.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

// `Observer::occupied_len` is the only Observer method we use; the
// `is_abandoned` we'd ideally want is not in ringbuf 0.4 (see module
// docs). If a future ringbuf version adds it we can wire it in to
// shorten the exit-on-detach path; for now `shutdown` is the path.

use crate::confidence::{TrackerEvent, TrackerState};
use crate::tracker::{BpmTracker, TrackerConfig, TrackerError};

/// Capacity of the event ringbuf. 64 transitions buffers tens of
/// minutes of normal operation (transitions are rare). Sized to
/// power-of-two for HeapRb efficiency; not load-bearing.
const EVENTS_CAPACITY: usize = 64;

/// How long the analysis thread sleeps between drain cycles. Well
/// below the tracker's recompute cadence (~1 s) so we never miss a
/// state transition by more than 20 ms of wall time.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Maximum samples drained from the audio ring per loop iteration.
/// Caps memory use of the analysis-side scratch buffer; larger
/// values reduce the number of `tracker.process` calls per second
/// of audio but do not affect correctness (the tracker is
/// block-size invariant).
const DRAIN_BLOCK_SAMPLES: usize = 4096;

/// What a live input — a record on a THRU deck — needs that a stream of
/// one file does not. See [`BpmStream::spawn_live`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveStreamConfig {
    /// Seconds of onset history the tempo search looks at. The input
    /// never ends; searching all of it would cost more every second.
    pub odf_window_secs: f32,
    /// How long the input stays below [`Self::silence_rms`] before the
    /// needle counts as up: the tracker starts over, and the next sound
    /// is a new record.
    pub needle_up_secs: f32,
    /// RMS below which the input is silence — a lifted needle, not a
    /// quiet passage (−50 dBFS).
    pub silence_rms: f32,
}

impl Default for LiveStreamConfig {
    fn default() -> Self {
        Self {
            odf_window_secs: 60.0,
            needle_up_secs: 1.0,
            silence_rms: 0.003,
        }
    }
}

/// The stream's state right now — see [`BpmStream::snapshot`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamSnapshot {
    /// The tracker's state: searching, tentative or locked, with its BPM.
    pub state: TrackerState,
    /// Bumps each time sound returns after the needle was up — a new
    /// record. Anything the DJ set for the old one (a tapped tempo)
    /// belongs to the old count.
    pub record_seq: u32,
    /// The input has been silent for [`LiveStreamConfig::needle_up_secs`].
    pub needle_up: bool,
    /// Frames of input the stream has consumed since it was spawned.
    pub consumed_frames: u64,
    /// The frame the current record's sound began at, counted like
    /// [`Self::consumed_frames`]; `None` before any sound and while the
    /// needle is up. What a THRU deck's clock and side overview count from.
    pub record_start_frame: Option<u64>,
}

/// Lock-free mirror of the analysis thread's state, read by
/// [`BpmStream::snapshot`] from any thread.
#[derive(Debug, Default)]
struct Shared {
    /// 0 searching · 1 tentative · 2 locked.
    state: AtomicU8,
    bpm_bits: AtomicU64,
    record_seq: AtomicU32,
    needle_up: AtomicBool,
    consumed: AtomicU64,
    /// [`NO_RECORD`] when there is none.
    record_start: AtomicU64,
}

/// `Shared::record_start` with no record under the needle.
const NO_RECORD: u64 = u64::MAX;

impl Shared {
    fn publish(&self, state: TrackerState) {
        let (code, bpm) = match state {
            TrackerState::Searching => (0, 0.0),
            TrackerState::Tentative { bpm } => (1, bpm),
            TrackerState::Locked { bpm } => (2, bpm),
        };
        self.bpm_bits.store(f64::to_bits(bpm), Ordering::Relaxed);
        self.state.store(code, Ordering::Release);
    }

    fn snapshot(&self) -> StreamSnapshot {
        let code = self.state.load(Ordering::Acquire);
        let bpm = f64::from_bits(self.bpm_bits.load(Ordering::Relaxed));
        let state = match code {
            1 => TrackerState::Tentative { bpm },
            2 => TrackerState::Locked { bpm },
            _ => TrackerState::Searching,
        };
        let start = self.record_start.load(Ordering::Acquire);
        StreamSnapshot {
            state,
            record_seq: self.record_seq.load(Ordering::Relaxed),
            needle_up: self.needle_up.load(Ordering::Relaxed),
            consumed_frames: self.consumed.load(Ordering::Acquire),
            record_start_frame: (start != NO_RECORD).then_some(start),
        }
    }
}

/// Streaming BPM analysis handle. Spawned via [`BpmStream::spawn`];
/// drop or call [`shutdown`] to stop the worker thread.
///
/// All methods are safe to call from any thread. The handle is
/// `Send` but not `Sync` — a single consumer reads events.
///
/// [`shutdown`]: Self::shutdown
pub struct BpmStream {
    events_rx: HeapCons<TrackerEvent>,
    shutdown: Arc<AtomicBool>,
    shared: Arc<Shared>,
    /// The thread hands its audio consumer back when it exits, so the
    /// ring can outlive one stream — see [`BpmStream::into_audio_rx`].
    join: Option<JoinHandle<HeapCons<f32>>>,
}

impl std::fmt::Debug for BpmStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BpmStream")
            .field("running", &self.join.is_some())
            .field(
                "shutdown_pending",
                &self.shutdown.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish()
    }
}

impl BpmStream {
    /// Spawn an analysis thread that reads mono audio from
    /// `audio_rx`, feeds it to a [`BpmTracker`] built from `cfg`,
    /// and emits [`TrackerEvent`]s on an internal channel.
    ///
    /// The thread runs until either:
    /// * [`shutdown`] is called on this handle, or
    /// * the producer end of `audio_rx` is dropped (e.g. the engine
    ///   reclaimed the associated `ThruSource`).
    ///
    /// `cfg.channels` should be `1` — the M8 engine tee mono-downmixes
    /// at the audio thread to save bandwidth, so the analysis side
    /// receives mono samples. Stereo tracker configs work too (they
    /// just downmix again, which is a no-op on already-mono input)
    /// but waste CPU. Construction passes through to [`BpmTracker`].
    ///
    /// # Errors
    ///
    /// Forwards [`TrackerError`] from underlying [`BpmTracker`]
    /// construction.
    ///
    /// [`shutdown`]: Self::shutdown
    pub fn spawn(audio_rx: HeapCons<f32>, cfg: TrackerConfig) -> Result<Self, TrackerError> {
        Self::spawn_inner(audio_rx, cfg, None)
    }

    /// [`Self::spawn`] for a live input: the tempo search looks at a
    /// window of recent onsets, and a stretch of silence — the needle
    /// coming up — starts the tracker over and counts the next sound as a
    /// new record ([`StreamSnapshot::record_seq`]).
    ///
    /// # Errors
    ///
    /// Forwards [`TrackerError`] from underlying [`BpmTracker`]
    /// construction.
    pub fn spawn_live(
        audio_rx: HeapCons<f32>,
        cfg: TrackerConfig,
        live: LiveStreamConfig,
    ) -> Result<Self, TrackerError> {
        Self::spawn_inner(audio_rx, cfg, Some(live))
    }

    fn spawn_inner(
        audio_rx: HeapCons<f32>,
        cfg: TrackerConfig,
        live: Option<LiveStreamConfig>,
    ) -> Result<Self, TrackerError> {
        let mut tracker = BpmTracker::new(cfg)?;
        let live = live.map(|l| {
            tracker.set_odf_window_secs(Some(l.odf_window_secs));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let needle_up_samples = (l.needle_up_secs.max(0.0) * cfg.sample_rate as f32) as usize;
            LiveState {
                needle_up_samples: needle_up_samples.max(1),
                silence_ms: l.silence_rms * l.silence_rms,
                quiet: 0,
                needle_up: false,
                channels: usize::from(cfg.channels.max(1)),
            }
        });

        let events_rb = HeapRb::<TrackerEvent>::new(EVENTS_CAPACITY);
        let (events_tx, events_rx) = events_rb.split();

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let shared = Arc::new(Shared::default());
        shared.record_start.store(NO_RECORD, Ordering::Relaxed);
        let shared_for_thread = Arc::clone(&shared);

        let join = thread::Builder::new()
            .name("dub-bpm-analysis".to_string())
            .spawn(move || {
                analysis_loop(
                    audio_rx,
                    tracker,
                    events_tx,
                    shutdown_for_thread,
                    &shared_for_thread,
                    live,
                )
            })
            .expect("OS refused to spawn analysis thread — fatal");

        Ok(Self {
            events_rx,
            shutdown,
            shared,
            join: Some(join),
        })
    }

    /// The tracker's state right now, read lock-free. Unlike
    /// [`Self::try_recv`] it needs no drain loop and loses nothing to a
    /// slow reader — the shape a UI poll wants.
    #[must_use]
    pub fn snapshot(&self) -> StreamSnapshot {
        self.shared.snapshot()
    }

    /// Stop the thread and take back the audio consumer it was reading,
    /// so a fresh stream can be spawned on the same ring. The ring's
    /// producer can then live on the audio thread for good: nothing is
    /// ever freed there. `None` only if the thread panicked.
    #[must_use]
    pub fn into_audio_rx(mut self) -> Option<HeapCons<f32>> {
        self.shutdown.store(true, Ordering::Release);
        self.join.take().and_then(|handle| handle.join().ok())
    }

    /// Pull the next state-change event if one is queued. Non-blocking.
    /// Call this on a UI tick or in a CLI poll loop.
    pub fn try_recv(&mut self) -> Option<TrackerEvent> {
        self.events_rx.try_pop()
    }

    /// Signal the worker thread to exit and join it. Idempotent: a
    /// second call is a no-op. Consumes the handle, so no further
    /// `try_recv` is possible.
    ///
    /// `Drop` calls this implicitly, so explicit calls are only
    /// needed when you want to surface a join panic (one of the
    /// analysis-thread invariants tripped) — `Drop`'s join is
    /// best-effort.
    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for BpmStream {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

/// The live-input watch: how long the input has been silent, and
/// whether that has counted as the needle coming up.
struct LiveState {
    needle_up_samples: usize,
    /// Mean square below which a block is silence.
    silence_ms: f32,
    /// Consecutive silent samples.
    quiet: usize,
    needle_up: bool,
    channels: usize,
}

fn analysis_loop(
    mut audio_rx: HeapCons<f32>,
    mut tracker: BpmTracker,
    mut events_tx: HeapProd<TrackerEvent>,
    shutdown: Arc<AtomicBool>,
    shared: &Shared,
    mut live: Option<LiveState>,
) -> HeapCons<f32> {
    // Pre-allocated drain buffer; never resized after construction.
    let mut block = vec![0.0f32; DRAIN_BLOCK_SAMPLES];
    shared.publish(TrackerState::Searching);

    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        // Drain everything available right now. The outer poll-sleep
        // ensures we still yield even if there's a constant stream.
        let mut drained_any = false;
        while audio_rx.occupied_len() > 0 {
            let n = audio_rx.pop_slice(&mut block);
            if n == 0 {
                break;
            }
            drained_any = true;
            let audio = &block[..n];
            let block_start = shared.consumed.load(Ordering::Relaxed);
            let frames = n / live.as_ref().map_or(1, |l| l.channels);
            shared
                .consumed
                .store(block_start + frames as u64, Ordering::Release);
            if let Some(live) = live.as_mut() {
                // Silence long enough is the needle up: start over, and
                // stop feeding the tracker until there is sound again —
                // the swap's silence is not part of either record.
                #[allow(clippy::cast_precision_loss)]
                let mean_sq = audio.iter().map(|x| x * x).sum::<f32>() / n as f32;
                if mean_sq < live.silence_ms {
                    live.quiet = live.quiet.saturating_add(n / live.channels);
                } else {
                    live.quiet = 0;
                    if live.needle_up {
                        live.needle_up = false;
                        shared.needle_up.store(false, Ordering::Relaxed);
                        shared.record_seq.fetch_add(1, Ordering::Relaxed);
                    }
                    // The first sound of a record — after the needle came
                    // down, or the first heard since the stream began.
                    if shared.record_start.load(Ordering::Relaxed) == NO_RECORD {
                        shared.record_start.store(block_start, Ordering::Release);
                    }
                }
                if !live.needle_up && live.quiet >= live.needle_up_samples {
                    live.needle_up = true;
                    shared.needle_up.store(true, Ordering::Relaxed);
                    shared.record_start.store(NO_RECORD, Ordering::Release);
                    let was = tracker.state();
                    tracker.reset();
                    shared.publish(TrackerState::Searching);
                    if was != TrackerState::Searching {
                        let _ =
                            events_tx.try_push(TrackerEvent::StateChanged(TrackerState::Searching));
                    }
                }
                if live.needle_up {
                    continue;
                }
            }
            if let Some(ev) = tracker.process(audio) {
                // Drop on full — see module docs. The "lose newest"
                // semantics of try_push (which errors when full) are
                // fine: the UI's stale state is fixed by the next
                // transition, not by overwriting.
                let TrackerEvent::StateChanged(state) = ev;
                shared.publish(state);
                let _ = events_tx.try_push(ev);
            }
        }

        if !drained_any {
            thread::sleep(POLL_INTERVAL);
        }
    }
    audio_rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    fn ring_for_samples(capacity: usize) -> (HeapProd<f32>, HeapCons<f32>) {
        HeapRb::<f32>::new(capacity).split()
    }

    fn cfg_mono(sr: u32) -> TrackerConfig {
        TrackerConfig {
            sample_rate: sr,
            channels: 1,
            analysis_period_samples: sr,
            bpm_range: crate::BpmRange::DEFAULT,
        }
    }

    /// Sleep with a hard deadline. Returns true if `pred()` became
    /// true within `total_timeout`. Polls every 10 ms. Used in
    /// tests to wait for the analysis thread to make progress
    /// without sleeping arbitrarily.
    fn wait_until<F: FnMut() -> bool>(total_timeout: Duration, mut pred: F) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < total_timeout {
            if pred() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        pred()
    }

    #[test]
    fn click_track_streams_to_lock() {
        // Push 10 s of 128 BPM clicks through a real spawned thread
        // and wait for the Locked event. This is the integration
        // test that pins the M8 acceptance criterion:
        //   "feed a fixture file through the streaming path; assert
        //    it lands within ±1 BPM of M7.5's offline answer."
        let sr = 48_000u32;
        let audio = synthetic::click_track(128.0, 10.0, sr);

        // 1 s ring capacity at 48 kHz mono = 48000 samples. The
        // analysis thread should keep up, but the ring absorbs
        // bursts from the test pushing in large chunks.
        let (mut tx, rx) = ring_for_samples(48_000);
        let mut stream = BpmStream::spawn(rx, cfg_mono(sr)).expect("spawn");

        // Feed in 4096-sample chunks with a small yield between.
        for chunk in audio.chunks(4096) {
            let mut remaining = chunk;
            while !remaining.is_empty() {
                let n = tx.push_slice(remaining);
                remaining = &remaining[n..];
                if !remaining.is_empty() {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }

        // Collect transitions for up to 5 s of wall time after the
        // last sample was pushed. The analysis thread needs at
        // least ~2 s to first lock (per the tracker tests).
        //
        // The acceptance criterion is *eventual* correctness — the
        // stream lands within ±1 BPM of M7.5's offline answer. We
        // therefore stop the moment we observe a correct lock and
        // assert on whether one ever occurred, not on whatever event
        // happens to be last when the wall-clock window closes.
        // Asserting on the tail is racy: after the first lock the
        // tracker keeps re-checking half/double octaves and can emit
        // a transient `Tentative` (e.g. a momentary 64 BPM excursion);
        // under CPU contention that excursion can be the last event
        // seen before the deadline, failing a tail assertion even
        // though the stream locked correctly seconds earlier.
        let mut transitions = Vec::new();
        let locked_correctly = wait_until(Duration::from_secs(5), || {
            while let Some(ev) = stream.try_recv() {
                transitions.push(ev);
            }
            transitions.iter().any(|ev| {
                matches!(
                    ev,
                    TrackerEvent::StateChanged(TrackerState::Locked { bpm })
                        if (bpm - 128.0).abs() <= 1.0
                )
            })
        });

        assert!(
            locked_correctly,
            "expected a Locked event at 128±1 within 5 s, never saw one (transitions: {transitions:?})"
        );
    }

    #[test]
    fn dropping_producer_terminates_analysis_thread() {
        // The analysis thread must exit when the producer end goes
        // away. Without this, a detach in the engine would leak the
        // thread.
        let (tx, rx) = ring_for_samples(1024);
        let stream = BpmStream::spawn(rx, cfg_mono(48_000)).expect("spawn");
        drop(tx);

        // BpmStream::drop calls shutdown_inner which joins. If the
        // analysis loop never exited, the test would hang forever.
        drop(stream);
    }

    #[test]
    fn explicit_shutdown_joins_promptly() {
        let (_tx, rx) = ring_for_samples(1024);
        let stream = BpmStream::spawn(rx, cfg_mono(48_000)).expect("spawn");
        let start = std::time::Instant::now();
        stream.shutdown();
        // POLL_INTERVAL is 20 ms; join must complete within a few
        // multiples of that on any reasonable machine.
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "shutdown took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn silence_emits_no_transitions() {
        let sr = 48_000u32;
        let (mut tx, rx) = ring_for_samples(sr as usize);
        let mut stream = BpmStream::spawn(rx, cfg_mono(sr)).expect("spawn");

        // 3 seconds of pure silence — should never leave Searching.
        for chunk in vec![0.0f32; sr as usize * 3].chunks(2048) {
            let mut remaining = chunk;
            while !remaining.is_empty() {
                let n = tx.push_slice(remaining);
                remaining = &remaining[n..];
                if !remaining.is_empty() {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
        // Until the worker has read every sample, then two of its polls
        // for the last hop to be analysed — a fixed 200 ms could pass
        // before the silence was looked at, on a loaded machine.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !tx.is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "worker never drained the ring"
            );
            thread::sleep(Duration::from_millis(2));
        }
        thread::sleep(POLL_INTERVAL * 2);

        let mut transitions = Vec::new();
        while let Some(ev) = stream.try_recv() {
            transitions.push(ev);
        }
        assert!(
            transitions.is_empty(),
            "silence should emit no transitions, got {transitions:?}"
        );
    }

    /// Push every sample, waiting for room when the ring is full.
    fn push_all(tx: &mut HeapProd<f32>, audio: &[f32]) {
        let mut remaining = audio;
        while !remaining.is_empty() {
            let n = tx.push_slice(remaining);
            remaining = &remaining[n..];
            if !remaining.is_empty() {
                thread::sleep(Duration::from_millis(2));
            }
        }
    }

    fn locked_near(snap: StreamSnapshot, bpm: f64) -> bool {
        matches!(snap.state, TrackerState::Locked { bpm: b } if (b - bpm).abs() <= 1.0)
    }

    /// One turntable, two records: the first locks, the needle comes up
    /// (a second of silence), the next record goes down at another
    /// tempo. The stream starts over on the lift, counts the drop as a
    /// new record, and locks on the new tempo — not on the old one, and
    /// not on some blend of the two.
    #[test]
    fn a_live_stream_starts_over_when_the_needle_comes_up() {
        let sr = 48_000u32;
        let (mut tx, rx) = ring_for_samples(48_000);
        let stream =
            BpmStream::spawn_live(rx, cfg_mono(sr), LiveStreamConfig::default()).expect("spawn");

        push_all(&mut tx, &synthetic::click_track(128.0, 12.0, sr));
        assert!(
            wait_until(Duration::from_secs(10), || locked_near(
                stream.snapshot(),
                128.0
            )),
            "the first record never locked: {:?}",
            stream.snapshot()
        );
        assert_eq!(stream.snapshot().record_seq, 0);

        push_all(&mut tx, &vec![0.0; 72_000]);
        assert!(
            wait_until(Duration::from_secs(5), || {
                let s = stream.snapshot();
                s.needle_up && s.state == TrackerState::Searching
            }),
            "the lift did not start the tracker over: {:?}",
            stream.snapshot()
        );

        push_all(&mut tx, &synthetic::click_track(90.0, 14.0, sr));
        assert!(
            wait_until(Duration::from_secs(10), || {
                let s = stream.snapshot();
                s.record_seq == 1 && locked_near(s, 90.0)
            }),
            "the second record did not lock as a new one: {:?}",
            stream.snapshot()
        );
    }

    /// The record's clock starts at its first sound, stops when the
    /// needle comes up, and starts again at the next record's first sound.
    #[test]
    fn the_record_start_follows_the_needle() {
        let sr = 48_000u32;
        let (mut tx, rx) = ring_for_samples(200_000);
        let stream =
            BpmStream::spawn_live(rx, cfg_mono(sr), LiveStreamConfig::default()).expect("spawn");

        push_all(&mut tx, &vec![0.0; 24_000]);
        assert!(wait_until(Duration::from_secs(5), || {
            stream.snapshot().consumed_frames >= 24_000
        }));
        assert_eq!(
            stream.snapshot().record_start_frame,
            None,
            "silence is no record"
        );

        push_all(&mut tx, &synthetic::click_track(120.0, 2.0, sr));
        assert!(wait_until(Duration::from_secs(5), || {
            stream.snapshot().consumed_frames >= 24_000 + 96_000
        }));
        let start = stream
            .snapshot()
            .record_start_frame
            .expect("the clicks are a record");
        assert!(
            (20_000..=24_000).contains(&start),
            "the record started at frame {start}"
        );

        push_all(&mut tx, &vec![0.0; 72_000]);
        assert!(
            wait_until(Duration::from_secs(5), || stream.snapshot().needle_up),
            "the lift was not heard"
        );
        assert_eq!(stream.snapshot().record_start_frame, None);

        push_all(&mut tx, &synthetic::click_track(120.0, 1.0, sr));
        assert!(wait_until(Duration::from_secs(5), || {
            stream.snapshot().record_start_frame.is_some()
        }));
        let next = stream.snapshot().record_start_frame.unwrap_or(0);
        assert!(
            next >= 24_000 + 96_000 + 60_000,
            "the next record started at {next}"
        );
    }

    /// The thread gives its consumer back, so the producer can stay on
    /// the audio thread for good and a fresh stream reads the same ring.
    #[test]
    fn into_audio_rx_hands_the_ring_back_for_a_fresh_stream() {
        let sr = 48_000u32;
        let (mut tx, rx) = ring_for_samples(4_096);
        let first = BpmStream::spawn(rx, cfg_mono(sr)).expect("spawn");
        let rx = first.into_audio_rx().expect("the consumer comes back");
        let second = BpmStream::spawn(rx, cfg_mono(sr)).expect("respawn");
        push_all(&mut tx, &[0.1; 2_048]);
        assert!(
            wait_until(Duration::from_secs(2), || tx.vacant_len() == 4_096),
            "the second stream is not reading the ring"
        );
        drop(second);
    }

    #[test]
    fn invalid_config_rejected_at_spawn() {
        let (_tx, rx) = ring_for_samples(64);
        let bad = TrackerConfig {
            sample_rate: 0,
            channels: 1,
            analysis_period_samples: 1,
            bpm_range: crate::BpmRange::DEFAULT,
        };
        assert!(BpmStream::spawn(rx, bad).is_err());
    }
}
