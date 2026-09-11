//! Quick Scratch (PRD §7.2): a sampler slot on the deck, and the way
//! back.
//!
//! Engaging puts a sample on the deck at 0, under the needle, and
//! **parks** whatever the deck was playing. Releasing puts the parked
//! track back. What "back" means depends on what the deck was doing
//! when the sample went on:
//!
//! * **Playing, other deck idle** → the tune is **doubled onto the
//!   other deck** (Instant Doubles, §7.3) and keeps playing there, out
//!   of the other mixer channel, on that deck's internal clock at this
//!   deck's pitch, while the sample is scratched and cut against it on
//!   this one. Release doubles it back — this deck takes the tune at
//!   the other deck's *live* position, in sync — and the other deck
//!   **reverts to what it had**: its own track at its own position, its
//!   own control mode. The DJ tried the alternative (the host playing
//!   on, to cover the crossfade back) and preferred the revert.
//! * **Playing, other deck also playing** → doubling would kill the
//!   other tune mid-mix, so the tune runs on silently underneath at the
//!   rate it had (a ghost clock) and comes back *in time*. The mix
//!   continues on the other deck anyway.
//! * **Paused / cued** → the tune comes back to the exact frame it was
//!   parked at. The idle deck holding the next record cued up gets its
//!   cue back.
//!
//! None of it is a setting. The decks already know which it is. The
//! ghost clock runs under a doubled park too: if the other deck gets
//! loaded over during the scratch, release falls back to it, so "back
//! in time" holds regardless.
//!
//! The swap itself is the Instant Doubles trick (§7.3): the sampler
//! slot's `Arc<Track>` is already decoded and at the engine rate, so
//! engaging is a refcount bump and a `swap_source` on the audio thread
//! — no decode, no file read, sample-accurate and instant. The parked
//! track is kept by its own `Arc` clone and handed back the same way;
//! nothing here allocates or frees on the audio thread.

use std::sync::Arc;

use dub_io::Track;

/// What a deck was doing before Quick Scratch took it over.
pub struct ParkedTrack {
    /// The track that was on the deck. `None` when the deck was empty —
    /// releasing then clears the deck rather than restoring anything.
    pub track: Option<Arc<Track>>,
    /// Where it is, in track frames. Advances while [`Self::ghost_rate`]
    /// is set; frozen otherwise.
    pub position_frames: f64,
    /// The rate the tune keeps running at underneath, in the deck's
    /// units (audio seconds per real second). `None` freezes.
    pub ghost_rate: Option<f64>,
    /// Restored on release in internal mode. Under timecode the
    /// platter overwrites it on the next block anyway.
    pub was_playing: bool,
    /// The parked track's own load-time gain, put back with it.
    pub gain: f32,
    /// Which sampler slot is on the deck in its place.
    pub slot: u8,
    /// The deck the tune was doubled onto, when it was. Release doubles
    /// it back from there if that deck still holds it.
    pub doubled_to: Option<u8>,
    /// What the host deck had before the tune was doubled onto it,
    /// given back at release.
    pub host: Option<HostPark>,
}

/// The host deck's own state, parked while it plays a doubled tune.
pub struct HostPark {
    /// Its track; `None` for an empty deck, which is cleared again.
    pub track: Option<Arc<Track>>,
    /// Its playhead, track frames — a cued record gets its cue back.
    pub position_frames: f64,
    /// Whether it was playing (it was idle, so normally not).
    pub playing: bool,
    /// Its rate, restored for internal mode.
    pub rate: f64,
    /// Its track's own load-time gain.
    pub gain: f32,
}

impl ParkedTrack {
    /// Run the ghost clock for one block of `frames` engine frames.
    /// Stops at the end of the track rather than running past it — a
    /// tune that finished while parked comes back at its end, which is
    /// where a deck that played it out would be.
    pub fn advance(&mut self, frames: usize, engine_sample_rate: f64) {
        let (Some(rate), Some(track)) = (self.ghost_rate, self.track.as_ref()) else {
            return;
        };
        if engine_sample_rate <= 0.0 {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let delta = rate * frames as f64 * f64::from(track.sample_rate()) / engine_sample_rate;
        #[allow(clippy::cast_precision_loss)]
        let end = track.frames() as f64;
        self.position_frames = (self.position_frames + delta).clamp(0.0, end);
    }

    /// The parked position in track seconds, for the deck header.
    #[must_use]
    pub fn position_secs(&self) -> f64 {
        self.track.as_ref().map_or(0.0, |track| {
            let sr = f64::from(track.sample_rate());
            if sr > 0.0 {
                self.position_frames / sr
            } else {
                0.0
            }
        })
    }
}

/// Value the deck's `quick_scratch_slot` atomic holds when nothing is
/// engaged. Slots are `0..SAMPLER_SLOTS`, so this is unambiguous.
pub const QUICK_SCRATCH_NONE: u8 = u8::MAX;

#[cfg(test)]
mod tests {
    use super::*;

    fn track(frames: usize, sr: u32) -> Arc<Track> {
        Arc::new(Track::from_interleaved(vec![0.1; frames * 2], sr, 2).unwrap())
    }

    fn parked(track: Option<Arc<Track>>, rate: Option<f64>) -> ParkedTrack {
        ParkedTrack {
            track,
            position_frames: 1_000.0,
            ghost_rate: rate,
            was_playing: rate.is_some(),
            gain: 1.0,
            slot: 0,
            doubled_to: None,
            host: None,
        }
    }

    /// Slip: the tune runs on underneath at its rate, converting
    /// engine frames to track frames like the deck does.
    #[test]
    fn a_ghost_clock_runs_at_the_parked_rate_in_track_frames() {
        let mut p = parked(Some(track(100_000, 44_100)), Some(1.0));
        p.advance(480, 48_000.0);
        assert!(
            (p.position_frames - 1_441.0).abs() < 1e-6,
            "{}",
            p.position_frames
        );

        let mut pitched = parked(Some(track(100_000, 48_000)), Some(1.05));
        pitched.advance(1_000, 48_000.0);
        assert!((pitched.position_frames - 2_050.0).abs() < 1e-6);
    }

    /// Freeze: a paused deck's parked position does not move.
    #[test]
    fn a_frozen_park_does_not_move() {
        let mut p = parked(Some(track(100_000, 48_000)), None);
        p.advance(4_800, 48_000.0);
        assert!((p.position_frames - 1_000.0).abs() < 1e-9);
    }

    #[test]
    fn the_ghost_stops_at_the_end_of_the_track() {
        let mut p = parked(Some(track(2_000, 48_000)), Some(1.0));
        p.advance(48_000, 48_000.0);
        assert!((p.position_frames - 2_000.0).abs() < 1e-9);
        assert!((p.position_secs() - 2_000.0 / 48_000.0).abs() < 1e-9);
    }

    #[test]
    fn an_empty_park_has_nothing_to_advance() {
        let mut p = parked(None, Some(1.0));
        p.advance(480, 48_000.0);
        assert!((p.position_frames - 1_000.0).abs() < 1e-9);
        assert!(p.position_secs().abs() < 1e-9);
    }
}
