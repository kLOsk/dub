//! Streaming BPM estimator.
//!
//! Fed audio blocks over time; emits the current best estimate at any
//! moment. The M8 milestone will wrap this in a non-RT analysis thread
//! per Thru deck and add a confidence state machine
//! (`searching` / `tentative` / `locked`).
//!
//! M7.5 scope: just the core. The estimator simply re-runs the tempo
//! search every block. That's O(odf_len) per block in the inner loop,
//! which is cheap enough offline but will be throttled when M8 adds
//! the per-deck thread. Throttling lives there, not here, because the
//! optimal cadence depends on the consumer's confidence-state logic
//! (PRD §5.2.3).

use crate::onset::OnsetDetector;
use crate::tempo::estimate_tempo;
use crate::{BpmEstimate, BpmRange, OctaveProfile, HOP_SIZE};

/// Errors that can occur while constructing a streaming estimator.
#[derive(Debug, thiserror::Error)]
pub enum BpmEstimatorError {
    /// Sample rate is zero.
    #[error("invalid sample rate: 0")]
    ZeroSampleRate,
}

/// Streaming tempo estimator. Feed audio with [`process`], read the
/// latest estimate with [`current`], wipe state with [`reset`].
///
/// All audio is treated as mono. Callers with stereo input must downmix
/// before calling [`process`] — that's the streaming analogue of
/// [`crate::analyze_bpm`]'s internal downmix.
///
/// [`process`]: Self::process
/// [`current`]: Self::current
/// [`reset`]: Self::reset
pub struct BpmEstimator {
    detector: OnsetDetector,
    odf_sample_rate: f64,
    last_estimate: Option<BpmEstimate>,
    bpm_range: BpmRange,
    /// ODF samples the tempo search sees — the newest ones; `None` for
    /// all of it (a file: the whole track is the evidence).
    odf_window: Option<usize>,
}

impl BpmEstimator {
    /// Construct a fresh estimator at the given audio sample rate
    /// using the default 60–200 BPM search range.
    ///
    /// # Errors
    ///
    /// Returns [`BpmEstimatorError::ZeroSampleRate`] if `sample_rate`
    /// is zero.
    pub fn new(sample_rate: u32) -> Result<Self, BpmEstimatorError> {
        Self::with_range(sample_rate, BpmRange::DEFAULT)
    }

    /// Like [`Self::new`] but constrains the tempo search to
    /// `bpm_range`.
    ///
    /// # Errors
    ///
    /// Returns [`BpmEstimatorError::ZeroSampleRate`] if `sample_rate`
    /// is zero.
    pub fn with_range(sample_rate: u32, bpm_range: BpmRange) -> Result<Self, BpmEstimatorError> {
        if sample_rate == 0 {
            return Err(BpmEstimatorError::ZeroSampleRate);
        }
        Ok(Self {
            detector: OnsetDetector::new(sample_rate),
            odf_sample_rate: f64::from(sample_rate) / HOP_SIZE as f64,
            last_estimate: None,
            bpm_range,
            odf_window: None,
        })
    }

    /// Search only the newest `secs` of onsets — for a live input, where
    /// the history only grows. `None` searches everything fed since the
    /// last [`reset`](Self::reset).
    pub fn set_window_secs(&mut self, secs: Option<f32>) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let window = secs.map(|s| (f64::from(s.max(1.0)) * self.odf_sample_rate) as usize);
        self.odf_window = window;
    }

    /// Feed a block of mono audio and refresh the cached estimate.
    ///
    /// Equivalent to [`feed`] followed by [`recompute`]. The aggregate
    /// shape is convenient for offline-style callers; streaming
    /// callers (e.g. M8's `BpmTracker`) typically call [`feed`] on
    /// every block but throttle [`recompute`] to a slower cadence
    /// because the tempo search is much more expensive than the
    /// onset-detection feed.
    ///
    /// [`feed`]: Self::feed
    /// [`recompute`]: Self::recompute
    pub fn process(&mut self, mono_block: &[f32]) {
        self.feed(mono_block);
        self.recompute();
    }

    /// Feed a block of mono audio into the onset detector. **Cheap**
    /// (O(block_len) plus one FFT per hop boundary). Does not update
    /// the cached estimate; call [`recompute`] for that.
    ///
    /// Block size is arbitrary; the onset detector buffers and emits
    /// at hop boundaries (`HOP_SIZE = 512` samples).
    ///
    /// [`recompute`]: Self::recompute
    pub fn feed(&mut self, mono_block: &[f32]) {
        self.detector.process(mono_block);
        // Trim in steps of a quarter window, not every hop: the drain
        // moves the whole buffer, so this keeps it to one move per
        // quarter window of audio.
        if let Some(window) = self.odf_window {
            if self.detector.odf().len() > window + window / 4 {
                self.detector.keep_last(window);
            }
        }
    }

    /// ODF samples currently held for the tempo search.
    #[must_use]
    pub fn odf_len(&self) -> usize {
        self.detector.odf().len()
    }

    /// Run the tempo-search step against the current ODF and update
    /// the cached estimate. **Expensive** (O(odf_len × max_lag)). For
    /// streaming use, call this on a slow cadence (~once per second);
    /// for offline analysis, [`process`] is the convenient aggregate.
    ///
    /// If the search produces no detection, the cached estimate is
    /// *not* cleared — once we've reported a tempo, growing ODF
    /// length alone shouldn't erase it. M8's `BpmTracker` is the
    /// authoritative place that decides "the signal has actually
    /// gone away" via the confidence state machine.
    ///
    /// [`process`]: Self::process
    pub fn recompute(&mut self) {
        if let Some(est) = estimate_tempo(
            self.detector.odf(),
            self.odf_sample_rate,
            self.bpm_range,
            OctaveProfile::Default,
        ) {
            self.last_estimate = Some(est);
        }
    }

    /// Current best estimate, or `None` if the estimator has never
    /// produced a confident result.
    #[must_use]
    pub fn current(&self) -> Option<BpmEstimate> {
        self.last_estimate
    }

    /// Wipe all state. Intended for use when the underlying audio
    /// source changes (e.g. a different deck is loaded).
    pub fn reset(&mut self) {
        self.detector.reset();
        self.last_estimate = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    #[test]
    fn new_rejects_zero_sample_rate() {
        assert!(matches!(
            BpmEstimator::new(0),
            Err(BpmEstimatorError::ZeroSampleRate)
        ));
    }

    #[test]
    fn fresh_estimator_has_no_current_estimate() {
        let e = BpmEstimator::new(48_000).unwrap();
        assert!(e.current().is_none());
    }

    #[test]
    fn does_not_panic_on_empty_block() {
        let mut e = BpmEstimator::new(48_000).unwrap();
        e.process(&[]);
        assert!(e.current().is_none());
    }

    #[test]
    fn reset_clears_estimate() {
        let mut e = BpmEstimator::new(48_000).unwrap();
        let audio = synthetic::click_track(120.0, 6.0, 48_000);
        e.process(&audio);
        assert!(e.current().is_some());
        e.reset();
        assert!(e.current().is_none());
    }

    /// A live input is analysed over a window, not its whole history: an
    /// hour of THRU must cost what a minute costs.
    #[test]
    fn a_window_bounds_the_onset_history() {
        let mut e = BpmEstimator::new(48_000).unwrap();
        e.set_window_secs(Some(10.0));
        let audio = synthetic::click_track(120.0, 40.0, 48_000);
        for chunk in audio.chunks(4096) {
            e.feed(chunk);
        }
        let window = (10.0 * 48_000.0 / HOP_SIZE as f64) as usize;
        assert!(
            e.odf_len() <= window + window / 4,
            "history grew to {} ODF samples, window {window}",
            e.odf_len()
        );
        e.recompute();
        let est = e
            .current()
            .expect("a windowed search still finds the tempo");
        assert!((est.bpm - 120.0).abs() < 1.0, "got {}", est.bpm);
    }

    #[test]
    fn small_blocks_eventually_produce_an_estimate() {
        let mut e = BpmEstimator::new(48_000).unwrap();
        let audio = synthetic::click_track(120.0, 6.0, 48_000);
        for chunk in audio.chunks(256) {
            e.process(chunk);
        }
        let est = e
            .current()
            .expect("should have estimate after 6s of clicks");
        assert!((est.bpm - 120.0).abs() < 1.0, "got {}", est.bpm);
    }
}
