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
pub(crate) const REASON_SILENCE: u8 = 4;
/// Not a stop at all: the session was reopened from disk after an
/// interrupted rip, and never had a worker in this process.
pub(crate) const REASON_RECOVERED: u8 = 5;

/// Samples drained per poll cycle. 16384 samples = 8192 frames ≈
/// 170 ms at 48 kHz — a 20 ms cadence therefore keeps up with ~8×
/// realtime bursts before the record ring (4 s) even starts filling.
const SCRATCH_SAMPLES: usize = 16_384;

/// The record tap is stereo, always (see `ThruSource`).
const CHANNELS: u16 = 2;

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
    pub(crate) auto: crate::AutoCapture,
}

/// Ring of the last `capacity` samples seen while armed, so a trigger
/// can write the needle drop it was triggered *by*.
///
/// Without this the first thing on the spill is whatever came after
/// the threshold crossing — the drop transient and the opening
/// fraction of a second are simply gone, and no amount of split
/// editing brings them back.
struct PreRoll {
    buf: Vec<f32>,
    write: usize,
    filled: bool,
}

impl PreRoll {
    fn new(samples: usize) -> Self {
        Self {
            buf: vec![0.0; samples],
            write: 0,
            filled: false,
        }
    }

    fn push(&mut self, block: &[f32]) {
        if self.buf.is_empty() {
            return;
        }
        // Only the tail can survive; skip whatever it would overwrite.
        let block = if block.len() > self.buf.len() {
            self.filled = true;
            &block[block.len() - self.buf.len()..]
        } else {
            block
        };
        for &s in block {
            self.buf[self.write] = s;
            self.write = (self.write + 1) % self.buf.len();
            if self.write == 0 {
                self.filled = true;
            }
        }
    }

    /// Oldest-to-newest contents, as two slices (the ring seam).
    fn drain(&mut self) -> (Vec<f32>, Vec<f32>) {
        if self.buf.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let out = if self.filled {
            (
                self.buf[self.write..].to_vec(),
                self.buf[..self.write].to_vec(),
            )
        } else {
            (self.buf[..self.write].to_vec(), Vec::new())
        };
        self.write = 0;
        self.filled = false;
        out
    }
}

/// Tracks how long the input has sat far enough under the music to
/// count as the run-out groove.
///
/// Relative, not absolute: run-out noise on a played-out 45 can sit at
/// −40 dBFS, above any fixed gate worth setting, while a quiet passage
/// on a well-pressed record goes lower than one. What separates them
/// is distance from *this side's* music level.
struct SilenceGate {
    music_level: f32,
    quiet_frames: u64,
    /// Consecutive non-quiet frames. A run-out groove ticks and pops;
    /// those are not the music coming back, so they only reset the
    /// count once they persist.
    loud_frames: u64,
    stop_after_frames: u64,
    tolerate_frames: u64,
    drop_ratio: f32,
}

impl SilenceGate {
    fn new(sample_rate: u32, stop_after_secs: f32, drop_db: f32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let stop_after_frames = (f64::from(stop_after_secs) * f64::from(sample_rate)) as u64;
        Self {
            music_level: 0.0,
            quiet_frames: 0,
            loud_frames: 0,
            stop_after_frames,
            // 400 ms: longer than any click, shorter than a bar.
            tolerate_frames: u64::from(sample_rate) * 2 / 5,
            drop_ratio: 10.0_f32.powf(-drop_db / 20.0),
        }
    }

    /// Feed one drained block. Returns true when the side is over.
    fn feed(&mut self, peak: f32, frames: u64, sample_rate: u32) -> bool {
        let _ = (frames, sample_rate);
        // Session peak-hold, deliberately without decay. An earlier
        // version decayed over 30 s, which meant a run-out groove
        // stopped looking quiet after ~24 s — the reference had sunk
        // to meet it — and the counter reset forever. Measured on a
        // real side: auto-stop never fired while the stylus sat in the
        // locked groove, and only triggered once the needle was
        // physically lifted. The reference has to stay where the music
        // was, because the question is "has the side ended", not "how
        // loud is it right now".
        if peak > self.music_level {
            self.music_level = peak;
        }
        if peak < self.music_level * self.drop_ratio {
            self.loud_frames = 0;
            self.quiet_frames += frames;
        } else {
            self.loud_frames += frames;
            // Only a sustained return of signal ends the quiet run;
            // measured on a real side, run-out ticks every few seconds
            // otherwise held the stop off for twice the timeout.
            if self.loud_frames >= self.tolerate_frames {
                self.quiet_frames = 0;
            }
        }
        self.quiet_frames >= self.stop_after_frames
    }
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

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pre_roll_samples = (f64::from(cfg.auto.pre_roll_secs) * f64::from(cfg.sample_rate))
        as usize
        * usize::from(CHANNELS);
    let mut pre_roll = PreRoll::new(if cfg.auto.start_threshold.is_some() {
        pre_roll_samples
    } else {
        0
    });
    let mut silence = cfg
        .auto
        .silence_stop_secs
        .map(|secs| SilenceGate::new(cfg.sample_rate, secs, cfg.auto.silence_drop_db));

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

        let mut recording = shared.state.load(Ordering::Acquire) == STATE_RECORDING;
        let n = rx.pop_slice(&mut scratch);

        // Needle drop: the first block over the threshold starts the
        // rip, and the pre-roll ring goes down ahead of it.
        if !recording && n > 0 {
            if let Some(threshold) = cfg.auto.start_threshold {
                if block_peak(&scratch[..n]) >= threshold
                    && shared
                        .state
                        .compare_exchange(
                            STATE_ARMED,
                            STATE_RECORDING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                {
                    recording = true;
                    let (older, newer) = pre_roll.drain();
                    // The ring holds seconds; `append` works on one
                    // drained block at a time (its mono scratch is
                    // sized for exactly that), so feed it in blocks.
                    for part in [older, newer] {
                        for block in part.chunks(SCRATCH_SAMPLES) {
                            if let Err(e) =
                                append(shared, &mut writer, &mut decimator, &mut mono, block)
                            {
                                shared.fail(e);
                                let _ = writer.finalize();
                                return;
                            }
                        }
                    }
                }
            }
        }

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
            if let Some(gate) = silence.as_mut() {
                let frames = (n & !1) as u64 / u64::from(CHANNELS);
                if gate.feed(block_peak(&scratch[..n]), frames, cfg.sample_rate) {
                    break REASON_SILENCE;
                }
            }
        } else if !recording && n > 0 {
            // Armed: hold the tail in the pre-roll ring and discard the
            // rest, so recording starts at the needle drop rather than
            // at whatever backlog sat in the ring.
            pre_roll.push(&scratch[..n]);
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
    // `block` must fit the caller's mono scratch — at most
    // `SCRATCH_SAMPLES`. The pre-roll flush chunks for this reason.
    debug_assert!(block.len() / usize::from(CHANNELS) <= mono.len());
    // Whole frames only; the record tap pushes interleaved pairs so
    // an odd count can only come from a torn producer — the trailing
    // sample would belong to the next block anyway.
    let samples = block.len() & !1;
    let frames = samples / 2;

    let peak = block_peak(&block[..samples]);

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

/// Absolute peak of one drained block.
fn block_peak(block: &[f32]) -> f32 {
    block.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()))
}

/// What the hands-off gates would have done to an already-recorded
/// side (M26b tuning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutoCaptureSim {
    /// Frame the needle-drop trigger would fire at, before pre-roll.
    pub start_frame: Option<u64>,
    /// Frame the run-out gate would stop at.
    pub stop_frame: Option<u64>,
    /// Longest quiet stretch the music came *back* from, in frames.
    /// This is the "how close did it come" number: the run-out that
    /// actually stopped the side is excluded, because a stretch that
    /// ended the recording says nothing about the margin.
    pub longest_quiet_frames: u64,
}

/// Replay a recorded side through the real trigger and silence gate.
///
/// This exists so thresholds can be tuned against one real recording
/// instead of against the turntable: it drives the same `SilenceGate`
/// and the same peak comparison the worker uses, in the same block
/// size, so what it reports is what would have happened.
#[must_use]
pub fn simulate(samples: &[f32], sample_rate: u32, cfg: &crate::AutoCapture) -> AutoCaptureSim {
    let mut sim = AutoCaptureSim::default();
    if sample_rate == 0 || samples.is_empty() {
        return sim;
    }
    let mut gate = cfg
        .silence_stop_secs
        .map(|secs| SilenceGate::new(sample_rate, secs, cfg.silence_drop_db));
    let mut frame = 0_u64;
    let mut recording = cfg.start_threshold.is_none();

    for block in samples.chunks(SCRATCH_SAMPLES) {
        let frames = (block.len() & !1) as u64 / u64::from(CHANNELS);
        let peak = block_peak(block);
        if !recording {
            if let Some(threshold) = cfg.start_threshold {
                if peak >= threshold {
                    recording = true;
                    sim.start_frame = Some(frame);
                }
            }
        }
        if recording && sim.stop_frame.is_none() {
            if let Some(gate) = gate.as_mut() {
                let before = gate.quiet_frames;
                if gate.feed(peak, frames, sample_rate) {
                    sim.stop_frame = Some(frame + frames);
                } else if gate.quiet_frames == 0 && before > 0 {
                    // Music came back: that stretch survived, so it is
                    // a real measure of the margin.
                    sim.longest_quiet_frames = sim.longest_quiet_frames.max(before);
                }
            }
        }
        frame += frames;
    }
    sim
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AutoCapture;

    const SR: u32 = 44_100;

    fn push(out: &mut Vec<f32>, secs: f64, amp: f32) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = (secs * f64::from(SR)) as u64;
        for i in 0..frames {
            let s = if i % 2 == 0 { amp } else { -amp };
            out.push(s);
            out.push(s);
        }
    }

    #[test]
    fn simulate_reports_trigger_stop_and_real_margin() {
        // lead-in, track, gap, track, run-out.
        let mut side = Vec::new();
        push(&mut side, 3.0, 0.002);
        push(&mut side, 5.0, 0.5);
        push(&mut side, 2.0, 0.002);
        push(&mut side, 5.0, 0.5);
        push(&mut side, 8.0, 0.002);

        let cfg = AutoCapture {
            start_threshold: Some(0.05),
            pre_roll_secs: 1.0,
            silence_stop_secs: Some(4.0),
            silence_drop_db: 25.0,
        };
        let sim = simulate(&side, SR, &cfg);

        let secs = |frames: u64| frames as f64 / f64::from(SR);
        let start = sim.start_frame.expect("trigger fires on the first music");
        assert!(
            (secs(start) - 3.0).abs() < 0.3,
            "trigger at {} s, expected ~3",
            secs(start)
        );
        let stop = sim.stop_frame.expect("run-out stops the side");
        // Music ends at 15 s (3 lead-in + 5 + 2 gap + 5), so the 4 s
        // timeout lands at 19 — plus up to one block of granularity.
        assert!(
            (secs(stop) - 19.0).abs() < 0.5,
            "stop at {} s, expected ~19",
            secs(stop)
        );
        // The 2 s inter-track gap is the only stretch the music came
        // back from; the run-out that ended the side must not count.
        assert!(
            (secs(sim.longest_quiet_frames) - 2.0).abs() < 0.4,
            "margin measured from the wrong stretch: {} s",
            secs(sim.longest_quiet_frames)
        );
    }

    #[test]
    fn simulate_without_gates_reports_nothing() {
        let mut side = Vec::new();
        push(&mut side, 2.0, 0.5);
        let sim = simulate(&side, SR, &AutoCapture::manual());
        assert_eq!(sim.start_frame, None);
        assert_eq!(sim.stop_frame, None);
    }
}
