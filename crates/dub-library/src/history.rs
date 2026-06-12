//! Mix-history capture state machine (M11d-history, PRD §8.2).
//!
//! Turns per-deck transport facts reported by the shell (`loaded`,
//! `play started`, `play ended`, `unloaded`) into `play_history`
//! rows, including the inferred deck-to-deck **transitions** that
//! back the Played From / Played Into surfaces.
//!
//! Why inference at all: the mixer is external (PRD §1), so the
//! software never sees the crossfader. The only honest signal is
//! deck transport state. A transition is recorded on **handover**:
//! the moment a deck stops while the other deck is still playing a
//! different track.
//!
//! Timecode wrinkle: in TC mode a deck "stops" every time the DJ
//! holds the record still or lifts the needle (carrier dropout ⇒
//! `DropoutHoldRate` ⇒ transport pause), so raw handover would fire
//! constantly during cueing. Two guards keep the data useful:
//!
//! * **Minimum-play gate** — the outgoing track must have
//!   accumulated [`MIN_TRANSITION_PLAY_MS`] of actual play time
//!   since it was loaded. Cue nudges stay under it; a track that
//!   was really played sails over it.
//! * **Consecutive-duplicate suppression** — re-stopping the same
//!   record during one mix-out records the same `from → to` edge
//!   only once. A genuine return (A→B … B→A … A→B) records all
//!   three.
//!
//! The tracker is deliberately clock-free: every event carries a
//! caller-supplied unix-millis timestamp, which keeps the state
//! machine deterministic under test.

use uuid::Uuid;

/// Minimum accumulated play time (since load) before a stopping
/// track is eligible to be the `from` side of a transition. 30 s
/// comfortably exceeds cue nudges and scratch holds while staying
/// under any real "this track was in the set" play.
pub const MIN_TRANSITION_PLAY_MS: i64 = 30_000;

/// `play_history.event_type` values the tracker emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryEventType {
    /// A track was loaded onto a deck.
    Load,
    /// A deck began playing its loaded track.
    PlayStart,
    /// A deck stopped playing; `duration_played_ms` carries the
    /// length of the segment that just ended.
    PlayEnd,
    /// This track was transitioned *into* (it kept playing while
    /// the other deck's track stopped).
    TransitionIn,
    /// This track was transitioned *out of* (it stopped while the
    /// other deck's track kept playing).
    TransitionOut,
}

impl HistoryEventType {
    /// Canonical string for the `play_history.event_type` CHECK
    /// constraint.
    pub fn as_str(&self) -> &'static str {
        match self {
            HistoryEventType::Load => "load",
            HistoryEventType::PlayStart => "play_start",
            HistoryEventType::PlayEnd => "play_end",
            HistoryEventType::TransitionIn => "transition_in",
            HistoryEventType::TransitionOut => "transition_out",
        }
    }
}

/// One pending `play_history` row. The session id is *not* carried
/// here — [`crate::Library::record_history`] stamps it on every row
/// so the tracker stays a pure event→rows function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryWrite {
    /// Canonical track id the event is attached to.
    pub track_id: String,
    /// Deck index (0 = A, 1 = B).
    pub deck: u32,
    /// Event discriminator.
    pub event: HistoryEventType,
    /// Caller-supplied unix-millis wall clock.
    pub timestamp_ms: i64,
    /// Length of the just-ended play segment (`PlayEnd` only).
    pub duration_played_ms: Option<i64>,
    /// Transition edge: the track the set moved away from. Set on
    /// **both** `TransitionIn` and `TransitionOut` rows so either
    /// row alone answers either direction of the §8.5 queries.
    pub from_track_id: Option<String>,
    /// Transition edge: the track the set moved to. Same
    /// both-rows contract as `from_track_id`.
    pub to_track_id: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct DeckSlot {
    track_id: Option<String>,
    /// `Some(ts)` while a play segment is open.
    playing_since_ms: Option<i64>,
    /// Total play time accumulated since this track was loaded.
    played_ms_since_load: i64,
}

/// Per-app-run mix-history tracker. Owns the session id (one UUID
/// per construction = one "gig / practice run" per PRD §8.2) and
/// the per-deck transport state needed to infer transitions.
#[derive(Debug)]
pub struct SessionTracker {
    session_id: String,
    decks: [DeckSlot; 2],
    /// Last recorded `(from, to)` edge, for consecutive-duplicate
    /// suppression.
    last_transition: Option<(String, String)>,
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionTracker {
    /// Fresh tracker with a new random session id.
    pub fn new() -> Self {
        SessionTracker {
            session_id: Uuid::new_v4().to_string(),
            decks: [DeckSlot::default(), DeckSlot::default()],
            last_transition: None,
        }
    }

    /// The opaque per-run marker stamped onto every row this
    /// tracker produces.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// A library track was loaded onto `deck`, replacing whatever
    /// was there. If the deck was mid-play, the old track's segment
    /// ends first (which may commit a handover transition).
    pub fn deck_loaded(
        &mut self,
        deck: u32,
        track_id: &str,
        timestamp_ms: i64,
    ) -> Vec<HistoryWrite> {
        let Some(idx) = slot_index(deck) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let was_playing = self.decks[idx].playing_since_ms.is_some();
        self.end_segment(idx, timestamp_ms, &mut out);
        self.decks[idx] = DeckSlot {
            track_id: Some(track_id.to_owned()),
            playing_since_ms: None,
            played_ms_since_load: 0,
        };
        out.push(HistoryWrite {
            track_id: track_id.to_owned(),
            deck,
            event: HistoryEventType::Load,
            timestamp_ms,
            duration_played_ms: None,
            from_track_id: None,
            to_track_id: None,
        });
        if was_playing {
            // Replace-load into a running deck: the engine keeps
            // the transport playing across the swap, so the shell's
            // polled edge detector never fires for the new track.
            // Open its segment here or the deck would be invisible
            // to later handover checks.
            self.decks[idx].playing_since_ms = Some(timestamp_ms);
            out.push(HistoryWrite {
                track_id: track_id.to_owned(),
                deck,
                event: HistoryEventType::PlayStart,
                timestamp_ms,
                duration_played_ms: None,
                from_track_id: None,
                to_track_id: None,
            });
        }
        out
    }

    /// The deck no longer holds a library track (ejected, or a
    /// non-library source — Finder drag, Thru — took it over). Ends
    /// any open segment; emits no row of its own (`play_history`
    /// has no unload event type).
    pub fn deck_unloaded(&mut self, deck: u32, timestamp_ms: i64) -> Vec<HistoryWrite> {
        let Some(idx) = slot_index(deck) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.end_segment(idx, timestamp_ms, &mut out);
        self.decks[idx] = DeckSlot::default();
        out
    }

    /// The deck's transport flipped to playing. No-op when the deck
    /// holds no library track or is already playing (idempotent —
    /// the shell reports edges from a polled snapshot).
    pub fn play_started(&mut self, deck: u32, timestamp_ms: i64) -> Vec<HistoryWrite> {
        let Some(idx) = slot_index(deck) else {
            return Vec::new();
        };
        let slot = &mut self.decks[idx];
        let Some(track_id) = slot.track_id.clone() else {
            return Vec::new();
        };
        if slot.playing_since_ms.is_some() {
            return Vec::new();
        }
        slot.playing_since_ms = Some(timestamp_ms);
        vec![HistoryWrite {
            track_id,
            deck,
            event: HistoryEventType::PlayStart,
            timestamp_ms,
            duration_played_ms: None,
            from_track_id: None,
            to_track_id: None,
        }]
    }

    /// The deck's transport flipped to stopped. Closes the open
    /// segment and runs the handover check (see module docs).
    pub fn play_ended(&mut self, deck: u32, timestamp_ms: i64) -> Vec<HistoryWrite> {
        let Some(idx) = slot_index(deck) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.end_segment(idx, timestamp_ms, &mut out);
        out
    }

    /// Close an open play segment on `idx`: emit `PlayEnd`, then
    /// commit a handover transition if the guards pass.
    fn end_segment(&mut self, idx: usize, timestamp_ms: i64, out: &mut Vec<HistoryWrite>) {
        let slot = &mut self.decks[idx];
        let (Some(track_id), Some(since)) = (slot.track_id.clone(), slot.playing_since_ms) else {
            return;
        };
        // A stale wall clock (NTP step) can put `since` in the
        // future; clamp rather than record a negative play.
        let segment_ms = (timestamp_ms - since).max(0);
        slot.played_ms_since_load += segment_ms;
        slot.playing_since_ms = None;
        let played_since_load = slot.played_ms_since_load;
        out.push(HistoryWrite {
            track_id: track_id.clone(),
            deck: idx as u32,
            event: HistoryEventType::PlayEnd,
            timestamp_ms,
            duration_played_ms: Some(segment_ms),
            from_track_id: None,
            to_track_id: None,
        });

        let other_idx = 1 - idx;
        let other = &self.decks[other_idx];
        let (Some(to_track), Some(_)) = (other.track_id.clone(), other.playing_since_ms) else {
            return;
        };
        // Instant doubles: same track on both decks is not a
        // transition.
        if to_track == track_id {
            return;
        }
        if played_since_load < MIN_TRANSITION_PLAY_MS {
            return;
        }
        let edge = (track_id.clone(), to_track.clone());
        if self.last_transition.as_ref() == Some(&edge) {
            return;
        }
        out.push(HistoryWrite {
            track_id: track_id.clone(),
            deck: idx as u32,
            event: HistoryEventType::TransitionOut,
            timestamp_ms,
            duration_played_ms: None,
            from_track_id: Some(track_id.clone()),
            to_track_id: Some(to_track.clone()),
        });
        out.push(HistoryWrite {
            track_id: to_track.clone(),
            deck: other_idx as u32,
            event: HistoryEventType::TransitionIn,
            timestamp_ms,
            duration_played_ms: None,
            from_track_id: Some(track_id),
            to_track_id: Some(to_track),
        });
        self.last_transition = Some(edge);
    }
}

fn slot_index(deck: u32) -> Option<usize> {
    (deck < 2).then_some(deck as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const T0: i64 = 1_700_000_000_000;
    const MIN: i64 = MIN_TRANSITION_PLAY_MS;

    fn events(writes: &[HistoryWrite]) -> Vec<HistoryEventType> {
        writes.iter().map(|w| w.event).collect()
    }

    #[test]
    fn load_emits_single_load_row() {
        let mut tr = SessionTracker::new();
        let w = tr.deck_loaded(0, "a", T0);
        assert_eq!(events(&w), [HistoryEventType::Load]);
        assert_eq!(w[0].track_id, "a");
        assert_eq!(w[0].deck, 0);
    }

    #[test]
    fn play_start_then_end_reports_segment_duration() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        let start = tr.play_started(0, T0 + 100);
        assert_eq!(events(&start), [HistoryEventType::PlayStart]);
        let end = tr.play_ended(0, T0 + 100 + 5_000);
        assert_eq!(events(&end), [HistoryEventType::PlayEnd]);
        assert_eq!(end[0].duration_played_ms, Some(5_000));
    }

    #[test]
    fn play_started_without_track_is_noop() {
        let mut tr = SessionTracker::new();
        assert!(tr.play_started(0, T0).is_empty());
        assert!(tr.play_ended(0, T0 + 1_000).is_empty());
    }

    #[test]
    fn play_started_twice_is_idempotent() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        assert_eq!(tr.play_started(0, T0).len(), 1);
        assert!(tr.play_started(0, T0 + 50).is_empty());
    }

    #[test]
    fn handover_records_transition_pair_with_both_edges() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "b", T0 + MIN);
        tr.play_started(1, T0 + MIN);
        // Deck A stops after > MIN of play while B keeps playing.
        let w = tr.play_ended(0, T0 + MIN + 10_000);
        assert_eq!(
            events(&w),
            [
                HistoryEventType::PlayEnd,
                HistoryEventType::TransitionOut,
                HistoryEventType::TransitionIn,
            ]
        );
        let out = &w[1];
        assert_eq!(out.track_id, "a");
        assert_eq!(out.deck, 0);
        assert_eq!(out.from_track_id.as_deref(), Some("a"));
        assert_eq!(out.to_track_id.as_deref(), Some("b"));
        let inn = &w[2];
        assert_eq!(inn.track_id, "b");
        assert_eq!(inn.deck, 1);
        assert_eq!(inn.from_track_id.as_deref(), Some("a"));
        assert_eq!(inn.to_track_id.as_deref(), Some("b"));
    }

    #[test]
    fn cue_stab_under_min_play_records_no_transition() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "b", T0 + 1_000);
        // B nudged for 2 s while A plays — cueing, not a handover.
        tr.play_started(1, T0 + 1_000);
        let w = tr.play_ended(1, T0 + 3_000);
        assert_eq!(events(&w), [HistoryEventType::PlayEnd]);
    }

    #[test]
    fn min_play_gate_is_cumulative_across_segments() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.deck_loaded(1, "b", T0);
        tr.play_started(1, T0);
        // A plays in two segments that only *together* cross MIN.
        tr.play_started(0, T0);
        tr.play_ended(0, T0 + MIN / 2);
        tr.play_started(0, T0 + MIN / 2 + 1_000);
        let w = tr.play_ended(0, T0 + MIN + 2_000);
        assert_eq!(
            events(&w),
            [
                HistoryEventType::PlayEnd,
                HistoryEventType::TransitionOut,
                HistoryEventType::TransitionIn,
            ]
        );
    }

    #[test]
    fn re_stopping_same_record_suppresses_duplicate_edge() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "b", T0 + MIN);
        tr.play_started(1, T0 + MIN);
        let first = tr.play_ended(0, T0 + MIN + 1_000);
        assert_eq!(first.len(), 3);
        // DJ pulls A back for a tail, stops it again: same edge,
        // suppressed.
        tr.play_started(0, T0 + MIN + 2_000);
        let again = tr.play_ended(0, T0 + MIN + 8_000);
        assert_eq!(events(&again), [HistoryEventType::PlayEnd]);
    }

    #[test]
    fn genuine_return_edge_is_not_suppressed() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "b", T0 + MIN);
        tr.play_started(1, T0 + MIN);
        assert_eq!(tr.play_ended(0, T0 + MIN + 1_000).len(), 3); // a→b
        tr.play_started(0, T0 + 2 * MIN);
        let back = tr.play_ended(1, T0 + 3 * MIN); // b→a
        assert_eq!(back.len(), 3);
        // …and forward again: a→b is a real new edge after b→a.
        tr.play_started(1, T0 + 3 * MIN + 1_000);
        let fwd = tr.play_ended(0, T0 + 4 * MIN);
        assert_eq!(fwd.len(), 3);
    }

    #[test]
    fn instant_doubles_same_track_records_no_transition() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "a", T0 + MIN);
        tr.play_started(1, T0 + MIN);
        let w = tr.play_ended(0, T0 + 2 * MIN);
        assert_eq!(events(&w), [HistoryEventType::PlayEnd]);
    }

    #[test]
    fn load_replace_while_playing_commits_handover_before_load() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "b", T0 + MIN);
        tr.play_started(1, T0 + MIN);
        // A still spinning when the DJ loads C over it. The engine
        // keeps the transport running across the swap, so the
        // tracker opens C's segment itself (no play edge will come
        // from the shell's poll).
        let w = tr.deck_loaded(0, "c", T0 + 2 * MIN);
        assert_eq!(
            events(&w),
            [
                HistoryEventType::PlayEnd,
                HistoryEventType::TransitionOut,
                HistoryEventType::TransitionIn,
                HistoryEventType::Load,
                HistoryEventType::PlayStart,
            ]
        );
        assert_eq!(w[3].track_id, "c");
        assert_eq!(w[4].track_id, "c");
    }

    #[test]
    fn replace_loaded_track_keeps_playing_and_can_transition_later() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "b", T0);
        tr.play_started(1, T0);
        // Replace playing A with C; the deck never stops.
        tr.deck_loaded(0, "c", T0 + 2 * MIN);
        // C plays past the gate, then the DJ hands over to B.
        let w = tr.play_ended(0, T0 + 3 * MIN + 1_000);
        assert_eq!(
            events(&w),
            [
                HistoryEventType::PlayEnd,
                HistoryEventType::TransitionOut,
                HistoryEventType::TransitionIn,
            ]
        );
        assert_eq!(w[1].from_track_id.as_deref(), Some("c"));
        assert_eq!(w[1].to_track_id.as_deref(), Some("b"));
    }

    #[test]
    fn load_into_paused_deck_does_not_open_a_segment() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        let w = tr.deck_loaded(0, "c", T0 + 1_000);
        assert_eq!(events(&w), [HistoryEventType::Load]);
        // No phantom segment: a stop on the other deck can't see
        // this deck as playing.
        assert!(tr.play_ended(0, T0 + 2_000).is_empty());
    }

    #[test]
    fn unload_ends_segment_and_clears_slot() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        let w = tr.deck_unloaded(0, T0 + 5_000);
        assert_eq!(events(&w), [HistoryEventType::PlayEnd]);
        // Slot is empty: a later play flip emits nothing.
        assert!(tr.play_started(0, T0 + 6_000).is_empty());
    }

    #[test]
    fn stopping_while_other_deck_paused_records_no_transition() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        tr.deck_loaded(1, "b", T0);
        // B loaded but never playing at the moment A stops.
        let w = tr.play_ended(0, T0 + 2 * MIN);
        assert_eq!(events(&w), [HistoryEventType::PlayEnd]);
    }

    #[test]
    fn out_of_range_deck_is_ignored() {
        let mut tr = SessionTracker::new();
        assert!(tr.deck_loaded(2, "a", T0).is_empty());
        assert!(tr.play_started(7, T0).is_empty());
    }

    #[test]
    fn clock_step_backwards_clamps_duration_to_zero() {
        let mut tr = SessionTracker::new();
        tr.deck_loaded(0, "a", T0);
        tr.play_started(0, T0);
        let w = tr.play_ended(0, T0 - 10_000);
        assert_eq!(w[0].duration_played_ms, Some(0));
    }

    proptest! {
        /// Arbitrary event soup never panics and never emits a
        /// degenerate transition (from == to, or either edge id
        /// missing), and play segments are never negative.
        #[test]
        fn tracker_invariants_hold_for_arbitrary_event_sequences(
            steps in proptest::collection::vec(
                (0u32..4, 0u32..2, 0usize..3, 0i64..120_000),
                0..64,
            )
        ) {
            let tracks = ["a", "b", "c"];
            let mut tr = SessionTracker::new();
            let mut now = T0;
            for (op, deck, track, dt) in steps {
                now += dt;
                let writes = match op {
                    0 => tr.deck_loaded(deck, tracks[track], now),
                    1 => tr.play_started(deck, now),
                    2 => tr.play_ended(deck, now),
                    _ => tr.deck_unloaded(deck, now),
                };
                for w in &writes {
                    if let Some(d) = w.duration_played_ms {
                        prop_assert!(d >= 0);
                    }
                    if matches!(
                        w.event,
                        HistoryEventType::TransitionIn | HistoryEventType::TransitionOut
                    ) {
                        let from = w.from_track_id.as_deref();
                        let to = w.to_track_id.as_deref();
                        prop_assert!(from.is_some() && to.is_some());
                        prop_assert_ne!(from, to);
                    }
                }
            }
        }
    }
}
