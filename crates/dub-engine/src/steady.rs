//! Steadying: a band's record played at one tempo while the DJ needs it.
//!
//! A record a band played moves a percent or two around its tempo from
//! one bar to the next. That is its feel, and it is also what makes it
//! hard to mix: a record matched to it is matched for four bars. With
//! steadying on, a deck plays each beat of such a record in the time a
//! beat takes at the tempo the band keeps — the middle one of all its
//! beats — so for as long as it is on, the record keeps one tempo and a
//! second record can be laid against it. Off, the band plays as it played (Daniel, 2026-11-19:
//! "pull them in line during transition but keep them funky when you
//! dont transition").
//!
//! The deck does the stretching ([`crate::deck`]); this is what it reads:
//! where the band's beats are, and how long a beat should be. It is
//! written by the control thread when a grid that follows a band is laid
//! and read by the audio thread once a block, with no lock and nothing
//! for the audio thread to free — the beats sit in a fixed table of
//! atomics, and a version number that is odd while the table is being
//! written tells a reader to leave the record as it is for that block.
//!
//! The table is for one track, named by the address of its buffer. The
//! deck says which track it is playing and gets no steadying for any
//! other — a track loaded since, a Quick Scratch sample — until the
//! control thread has laid that one's beats.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

/// Beats the table holds: over two hours at 120 BPM.
pub const STEADY_CAPACITY: usize = 16_384;

/// A beat is played no more than this much faster or slower than the band
/// played it. A band moves a few percent; a gap further off than this is
/// not a beat of the band's — a break, a beat the tracker lost — and is
/// left alone.
const MAX_STEADYING: f64 = 0.08;

/// What a deck reads to steady the track it plays. See the module docs.
pub struct SteadyMap {
    /// Odd while [`Self::install`] or [`Self::clear`] is writing.
    version: AtomicU32,
    /// The track the beats are for: the address of its buffer, 0 for none.
    track: AtomicUsize,
    len: AtomicUsize,
    /// Seconds a beat lasts at the record's tempo, as `f64` bits.
    period_bits: AtomicU64,
    /// Where the band's beats are, in track seconds, as `f64` bits.
    beats: Box<[AtomicU64]>,
    /// The DJ's switch.
    on: AtomicBool,
    /// What the deck last did with it: [`SteadyState`] as `u8`.
    state: AtomicU8,
    /// How much faster than the platter the last block moved the
    /// playhead, as `f64` bits; 1 when it was not steadied.
    moved_bits: AtomicU64,
}

/// What a deck is doing about steadying, for its indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SteadyState {
    /// The switch is off.
    Off = 0,
    /// On, and the record plays as the band played it for now: nothing to
    /// steady it by, or a scratch, or the platter far from playing speed.
    Waiting = 1,
    /// On, and the record is being held to its tempo.
    Steadying = 2,
}

impl std::fmt::Debug for SteadyMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SteadyMap")
            .field("len", &self.len.load(Ordering::Relaxed))
            .field("on", &self.on.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for SteadyMap {
    fn default() -> Self {
        Self::new()
    }
}

impl SteadyMap {
    /// An empty table, the switch off. Allocates the table once.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: AtomicU32::new(0),
            track: AtomicUsize::new(0),
            len: AtomicUsize::new(0),
            period_bits: AtomicU64::new(0),
            beats: (0..STEADY_CAPACITY).map(|_| AtomicU64::new(0)).collect(),
            on: AtomicBool::new(false),
            state: AtomicU8::new(SteadyState::Off as u8),
            moved_bits: AtomicU64::new(1.0f64.to_bits()),
        }
    }

    /// Lay the beats of the band on the track whose buffer is at `track`.
    /// Control thread only, one caller at a time. Beats past
    /// [`STEADY_CAPACITY`] are left off; fewer than two, or beats that do
    /// not run forward, clear the table.
    ///
    /// The tempo the record is held to is the band's own: the middle one
    /// of the gaps between its beats. Not the even grid's — that is one
    /// number fitted to the whole record, and on *Mary Jane* it is half a
    /// percent off a band that never moves (92.5 against 93.0), which
    /// would make steadying slow a steady record down.
    pub fn install(&self, track: usize, beats_secs: &[f64]) {
        let n = beats_secs.len().min(STEADY_CAPACITY);
        let mut gaps: Vec<f64> = beats_secs[..n].windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(f64::total_cmp);
        let period = gaps.get(gaps.len() / 2).copied().unwrap_or(0.0);
        if !(period.is_finite() && period > 0.0) {
            self.clear();
            return;
        }
        self.version.fetch_add(1, Ordering::AcqRel);
        self.track.store(track, Ordering::Release);
        self.period_bits.store(period.to_bits(), Ordering::Release);
        for (slot, &beat) in self.beats.iter().zip(&beats_secs[..n]) {
            slot.store(beat.to_bits(), Ordering::Release);
        }
        self.len.store(n, Ordering::Release);
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    /// Forget the beats: no track is steadied until the next
    /// [`Self::install`]. Control thread only.
    pub fn clear(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
        self.track.store(0, Ordering::Release);
        self.len.store(0, Ordering::Release);
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    /// Turn the DJ's switch.
    pub fn set_on(&self, on: bool) {
        self.on.store(on, Ordering::Release);
    }

    /// Whether the DJ's switch is on.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    /// The tempo a steadied record is held to, in BPM at the record's own
    /// speed. `None` with no beats laid.
    #[must_use]
    pub fn bpm(&self) -> Option<f64> {
        let period = f64::from_bits(self.period_bits.load(Ordering::Acquire));
        (self.len.load(Ordering::Acquire) >= 2 && period > 0.0).then(|| 60.0 / period)
    }

    /// What the deck last did with the switch.
    #[must_use]
    pub fn state(&self) -> SteadyState {
        match self.state.load(Ordering::Relaxed) {
            1 => SteadyState::Waiting,
            2 => SteadyState::Steadying,
            _ => SteadyState::Off,
        }
    }

    pub(crate) fn store_state(&self, state: SteadyState, moved: f64) {
        self.state.store(state as u8, Ordering::Relaxed);
        self.moved_bits.store(moved.to_bits(), Ordering::Relaxed);
    }

    /// How much faster than the platter the deck's playhead is moving:
    /// what steadying asked of the last block, 1 when it asked nothing.
    /// The playhead's speed over this is the platter's.
    #[must_use]
    pub fn moved(&self) -> f64 {
        f64::from_bits(self.moved_bits.load(Ordering::Relaxed))
    }

    /// How much faster than the band played it the track at `track` is to
    /// be played at `secs`, so that the beat there lasts what a beat
    /// should: above 1 where the band dragged, below where it pushed.
    /// `None` when the switch is off, the beats are for another track or
    /// are being written, or `secs` is not between two beats of the
    /// band's. Audio thread: no lock, no allocation, a binary search.
    #[must_use]
    pub fn factor_at(&self, track: usize, secs: f64) -> Option<f64> {
        if !self.is_on() {
            return None;
        }
        let version = self.version.load(Ordering::Acquire);
        if version % 2 == 1 || track == 0 || self.track.load(Ordering::Acquire) != track {
            return None;
        }
        let n = self.len.load(Ordering::Acquire).min(self.beats.len());
        let period = f64::from_bits(self.period_bits.load(Ordering::Acquire));
        let beat = |i: usize| f64::from_bits(self.beats[i].load(Ordering::Acquire));
        // The first beat after `secs`.
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if beat(mid) <= secs {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let factor = (lo > 0 && lo < n).then(|| (beat(lo) - beat(lo - 1)) / period);
        if self.version.load(Ordering::Acquire) != version {
            return None;
        }
        factor.filter(|f| (f - 1.0).abs() <= MAX_STEADYING)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: usize = 0x1000;

    /// Beats at 100 BPM, every fourth gap 3 % long.
    fn dragging() -> Vec<f64> {
        let mut at = 1.0;
        (0..64)
            .map(|i| {
                let here = at;
                at += if i % 4 == 3 { 0.618 } else { 0.6 };
                here
            })
            .collect()
    }

    #[test]
    fn a_beat_the_band_dragged_is_played_faster() {
        let map = SteadyMap::new();
        map.install(TRACK, &dragging());
        map.set_on(true);
        assert!((map.bpm().unwrap() - 100.0).abs() < 1e-9, "the usual gap");
        let beats = dragging();
        let even = map.factor_at(TRACK, beats[0] + 0.1).unwrap();
        let dragged = map.factor_at(TRACK, beats[3] + 0.1).unwrap();
        assert!((even - 1.0).abs() < 1e-9, "an even beat is left: {even}");
        assert!((dragged - 1.03).abs() < 1e-9, "a long beat: {dragged}");
    }

    #[test]
    fn nothing_is_steadied_with_the_switch_off_or_outside_the_beats() {
        let map = SteadyMap::new();
        let beats = dragging();
        map.install(TRACK, &beats);
        assert_eq!(map.factor_at(TRACK, 5.0), None, "off");
        map.set_on(true);
        assert!(map.factor_at(TRACK, 5.0).is_some());
        assert_eq!(map.factor_at(TRACK, 0.5), None, "before the first beat");
        assert_eq!(
            map.factor_at(TRACK, beats[beats.len() - 1] + 0.1),
            None,
            "after the last"
        );
    }

    #[test]
    fn another_track_is_not_steadied_by_these_beats() {
        let map = SteadyMap::new();
        map.install(TRACK, &dragging());
        map.set_on(true);
        assert_eq!(map.factor_at(TRACK + 8, 5.0), None);
        assert_eq!(map.factor_at(0, 5.0), None);
        map.clear();
        assert_eq!(map.factor_at(TRACK, 5.0), None);
    }

    #[test]
    fn a_gap_that_is_no_beat_of_the_bands_is_left_alone() {
        let map = SteadyMap::new();
        // A bar's break between the second and third beat.
        map.install(TRACK, &[1.0, 1.6, 4.0, 4.6, 5.2]);
        map.set_on(true);
        assert!(map.factor_at(TRACK, 1.3).is_some());
        assert_eq!(map.factor_at(TRACK, 3.0), None);
    }

    #[test]
    fn beats_that_are_not_a_grid_clear_the_table() {
        let map = SteadyMap::new();
        map.install(TRACK, &dragging());
        map.set_on(true);
        map.install(TRACK, &[1.0]);
        assert_eq!(map.factor_at(TRACK, 5.0), None);
        assert_eq!(map.bpm(), None);
        map.install(TRACK, &dragging());
        map.install(TRACK, &[5.0, 5.0, 5.0, 5.0]);
        assert_eq!(map.factor_at(TRACK, 5.0), None);
    }
}
