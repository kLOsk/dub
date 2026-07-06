//! The off-RT capture worker: drains the engine's stereo record-tap
//! ring into a crash-safe 32-bit-float WAV spill while accumulating
//! the live peak envelope.
//!
//! Spill-to-disk, not RAM: a crash 24 minutes into a one-take rip
//! must not lose audio (the WAV survives for salvage), and a 25-min
//! side would otherwise pin ~576 MB while the engine may hold a
//! loaded deck. Analysis reads the file back lazily at commit.
//!
//! The mono-downmix envelope is fed from the *recorded* samples only,
//! so `chunk[i]` maps exactly to WAV frame `i * 64` — the alignment
//! the split plan depends on. (The deck's own live `PeakStream`
//! starts at Thru attach, not record start, so it cannot give this.)

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use ringbuf::traits::{Consumer, Observer};
use ringbuf::HeapCons;

use dub_peaks::{Decimator, PeakChunk, DEFAULT_SAMPLES_PER_CHUNK};

// Session state as seen by pollers. Discriminants are stable so the
// session can snapshot them from an AtomicU8.
pub(crate) const STATE_IDLE: u8 = 0;
pub(crate) const STATE_ARMED: u8 = 1;
pub(crate) const STATE_RECORDING: u8 = 2;
pub(crate) const STATE_STOPPED: u8 = 3;
pub(crate) const STATE_FAILED: u8 = 4;

pub(crate) const CMD_NONE: u8 = 0;
pub(crate) const CMD_START: u8 = 1;
pub(crate) const CMD_STOP: u8 = 2;

pub(crate) const REASON_NONE: u8 = 0;
pub(crate) const REASON_MANUAL: u8 = 1;
pub(crate) const REASON_MAX_DURATION: u8 = 2;
pub(crate) const REASON_INPUT_LOST: u8 = 3;

/// Samples drained per poll cycle. 16384 samples = 8192 frames ≈
/// 170 ms at 48 kHz — a 20 ms cadence therefore keeps up with ~8×
/// realtime bursts before the record ring (4 s) even starts filling.
const SCRATCH_SAMPLES: usize = 16_384;

/// State shared between the capture worker and the session/pollers.
/// Everything the 10 Hz status poll reads is an atomic; the growing
/// envelope sits behind a Mutex that only the worker (append) and
/// off-RT pollers (snapshot) touch — nothing here is reachable from
/// the audio thread.
pub(crate) struct CaptureShared {
    pub(crate) state: AtomicU8,
    pub(crate) command: AtomicU8,
    pub(crate) recorded_frames: AtomicU64,
    /// |peak| of the most recent poll window while recording
    /// (f32 bits). Drives the UI level meter.
    pub(crate) window_peak_bits: AtomicU32,
    pub(crate) stop_reason: AtomicU8,
    pub(crate) envelope: Mutex<Vec<PeakChunk>>,
    pub(crate) failure: Mutex<Option<String>>,
}

impl CaptureShared {
    pub(crate) fn new() -> Self {
        Self {
            state: AtomicU8::new(STATE_IDLE),
            command: AtomicU8::new(CMD_NONE),
            recorded_frames: AtomicU64::new(0),
            window_peak_bits: AtomicU32::new(0),
            stop_reason: AtomicU8::new(REASON_NONE),
            envelope: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
        }
    }

    /// Envelope guard, poisoning-proof: the worker never panics while
    /// holding the lock by construction, but a poisoned lock must not
    /// take the session down with it — the data is still valid.
    pub(crate) fn envelope_guard(&self) -> MutexGuard<'_, Vec<PeakChunk>> {
        self.envelope
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn failure_guard(&self) -> MutexGuard<'_, Option<String>> {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fail(&self, message: String) {
        *self.failure_guard() = Some(message);
        self.state.store(STATE_FAILED, Ordering::Release);
    }
}

/// Worker parameters, fixed at spawn.
pub(crate) struct CaptureConfig {
    pub(crate) wav_path: PathBuf,
    pub(crate) sample_rate: u32,
    pub(crate) max_frames: u64,
    pub(crate) poll_interval: Duration,
}

/// Spawn the capture worker. It creates the WAV spill immediately
/// (so `Armed` already owns the file), then loops: drain ring →
/// (while recording) append to WAV + feed envelope → honor
/// start/stop commands and the max-duration cap. Producer-side ring
/// closure (engine detached / audio stopped) is a fail-safe stop
/// with what was captured, not an error — see the module docs.
pub(crate) fn spawn(
    mut rx: HeapCons<f32>,
    shared: Arc<CaptureShared>,
    cfg: CaptureConfig,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("dub-rip-capture".into())
        .spawn(move || run(&mut rx, &shared, &cfg))
}

fn run(rx: &mut HeapCons<f32>, shared: &CaptureShared, cfg: &CaptureConfig) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: cfg.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = match hound::WavWriter::create(&cfg.wav_path, spec) {
        Ok(w) => w,
        Err(e) => {
            shared.fail(format!("cannot create WAV spill: {e}"));
            return;
        }
    };

    let mut scratch = vec![0.0_f32; SCRATCH_SAMPLES];
    let mut mono = vec![0.0_f32; SCRATCH_SAMPLES / 2];
    let mut decimator = Decimator::new(DEFAULT_SAMPLES_PER_CHUNK);
    shared.state.store(STATE_ARMED, Ordering::Release);

    let reason = loop {
        match shared.command.swap(CMD_NONE, Ordering::AcqRel) {
            CMD_STOP => break REASON_MANUAL,
            // Armed → Recording; a start while already recording is a
            // harmless no-op (compare_exchange fails, nothing changes).
            CMD_START => {
                let _ = shared.state.compare_exchange(
                    STATE_ARMED,
                    STATE_RECORDING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            _ => {}
        }

        let recording = shared.state.load(Ordering::Acquire) == STATE_RECORDING;
        let n = rx.pop_slice(&mut scratch);

        if recording && n > 0 {
            if let Err(e) = append(
                shared,
                &mut writer,
                &mut decimator,
                &mut mono,
                &scratch[..n],
            ) {
                shared.fail(e);
                let _ = writer.finalize();
                return;
            }
            if shared.recorded_frames.load(Ordering::Acquire) >= cfg.max_frames {
                break REASON_MAX_DURATION;
            }
        } else if !recording && n > 0 {
            // Armed: drain and discard so recording starts at "now",
            // not at whatever backlog sat in the ring. (M26b's
            // needle-drop pre-roll will keep a short history here.)
            shared.window_peak_bits.store(0, Ordering::Release);
        }

        // Producer gone (engine detached / audio device stopped) and
        // nothing left to drain: fail-safe stop with what we have.
        if n == 0 && !rx.write_is_held() {
            break REASON_INPUT_LOST;
        }

        std::thread::sleep(cfg.poll_interval);
    };

    // Final drain: anything still in the ring at stop time belongs to
    // the rip (up to the max-duration cap).
    if shared.state.load(Ordering::Acquire) == STATE_RECORDING {
        while shared.recorded_frames.load(Ordering::Acquire) < cfg.max_frames {
            let n = rx.pop_slice(&mut scratch);
            if n == 0 {
                break;
            }
            if let Err(e) = append(
                shared,
                &mut writer,
                &mut decimator,
                &mut mono,
                &scratch[..n],
            ) {
                shared.fail(e);
                let _ = writer.finalize();
                return;
            }
        }
    }

    if let Err(e) = writer.finalize() {
        shared.fail(format!("cannot finalize WAV spill: {e}"));
        return;
    }
    shared.stop_reason.store(reason, Ordering::Release);
    shared.state.store(STATE_STOPPED, Ordering::Release);
}

/// Append one drained block: WAV samples, envelope chunks, frame
/// count, level-meter peak. Returns a human-readable reason on I/O
/// failure.
fn append(
    shared: &CaptureShared,
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    decimator: &mut Decimator,
    mono: &mut [f32],
    block: &[f32],
) -> Result<(), String> {
    // Whole frames only; the record tap pushes interleaved pairs so
    // an odd count can only come from a torn producer — the trailing
    // sample would belong to the next block anyway.
    let samples = block.len() & !1;
    let frames = samples / 2;

    let mut peak = 0.0_f32;
    for &s in &block[..samples] {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
    }

    for &s in &block[..samples] {
        writer
            .write_sample(s)
            .map_err(|e| format!("WAV write failed: {e}"))?;
    }

    for i in 0..frames {
        mono[i] = 0.5 * (block[i * 2] + block[i * 2 + 1]);
    }
    {
        let mut envelope = shared.envelope_guard();
        decimator.feed(&mono[..frames], |chunk| envelope.push(chunk));
    }

    shared
        .recorded_frames
        .fetch_add(frames as u64, Ordering::AcqRel);
    shared
        .window_peak_bits
        .store(peak.to_bits(), Ordering::Release);
    Ok(())
}
