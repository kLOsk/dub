//! Sticker-drift monitor: live measurement of relative-mode position
//! error against the absolute groove position.
//!
//! In relative mode the playhead is the *integral* of the decoded rate,
//! so any momentary disagreement between that integral and the record's
//! true motion — a scratch turnaround where coherence collapses, a slow
//! draw where the cartridge output (velocity-proportional) fades to
//! nothing and the lift policy freezes the deck while the record keeps
//! moving — leaves a permanent offset. DJs see it as **sticker drift**:
//! the kick no longer sits at the cue sticker after a few minutes of
//! scratching.
//!
//! The M6 absolute LFSR decode gives us the ground truth to *measure*
//! this continuously: while ABS-locked, `groove_position − playhead`
//! must be constant for the lifetime of an engagement — the constant is
//! simply "where on the record the DJ dropped". Any change **is** the
//! accumulated drift, bit-exact. The monitor anchors that offset on the
//! first locked observation and reports the deviation from then on.
//!
//! **Telling drift from a deliberate remap.** The absolute tracker
//! holds no lock during a scratch session, so all the drift the session
//! produced arrives as *one* offset jump at the first re-lock — easily
//! hundreds of ms. Magnitude therefore cannot distinguish "scratched
//! for 30 s" from "lifted the needle and dropped it elsewhere"; an
//! on-rig session measured −170 ms of true drift that a magnitude-only
//! threshold silently discarded as a remap. What does distinguish them
//! is **sustained silence**: a needle in the groove keeps producing
//! carrier (the dips at turnarounds last well under a second), while a
//! lift goes quiet for seconds. So the monitor re-anchors only when the
//! gap contained a sustained-silence lift *and* the offset moved
//! ([`REMAP_THRESHOLD_SECS`]) — a lift dropped back on the sticker
//! keeps the anchor and the running measurement. A magnitude fallback
//! ([`FALLBACK_REMAP_SECS`]) catches programmatic jumps (cue/seek/track
//! load) that bypass the needle entirely.
//!
//! **A needle skip keeps the sticker.** That is what relative mode is
//! for: a skip must not ruin the performance. A skip moves the needle by
//! whole turns of the record, so an offset that jumps by one or two turns
//! (within [`SKIP_TOLERANCE_SECS`]) moves the anchor by exactly those
//! turns: the track plays on from where the hand put it, and only what
//! is left over — motion the decode lost in the same gap — reads as
//! drift. Read as drift whole, a skip inside a scratch was healed toward
//! the skipped groove for three minutes.
//!
//! The engine heals what this measures (PRD §5.4.3); the monitor itself
//! never touches transport. **RT-safety**: a few scalar fields, pure
//! arithmetic.

/// After a sustained-silence gap, the needle reappearing more than
/// this far (groove seconds) from where the signal stopped means it
/// was physically relocated — re-anchor. A *held* record resumes at
/// the same groove position (hand wiggle ≪ this), and an in-place
/// scratch stroke spans well under a second of groove — both must
/// keep the anchor so their accumulated losses get healed. Offset
/// magnitude cannot make this call: a hold-heavy scratch session
/// legitimately accumulates more drift than any fixed offset
/// threshold, and re-anchoring on it both zeroed the readout and
/// silently disabled healing (the on-rig "a whole bar off and the
/// display says 0.2 ms").
const RELOCATION_SECS: f64 = 1.5;

/// Offset jumps beyond this re-anchor even without a lift — a cue
/// jump, seek, or track load moved the playhead programmatically.
const FALLBACK_REMAP_SECS: f64 = 5.0;

/// Net groove travel across a gap beyond which the gesture was a
/// *travel* move — a backspin, a spin-forward, a power-down ride. Those
/// physically skid the needle across grooves (normal vinyl behavior),
/// so a large offset jump after one is a needle skip, not decode loss:
/// re-anchor silently. Relative mode must never move the song to chase
/// a skipped needle — and any late correction after a release is
/// audibly a song jump (the on-rig report). In-place scratching stays
/// well under this, so precision healing is unaffected.
const TRAVEL_GESTURE_SECS: f64 = 3.0;

/// Offset jump beyond this after a travel gesture is treated as needle
/// skid. Below it, even a travel gesture heals (small decode loss).
const SKID_JUMP_SECS: f64 = 0.25;

/// One turn of the record at 33⅓ rpm, in seconds. Every pressing Dub
/// reads is a 12" cut at 33⅓, so a turn always holds the same groove;
/// at 45 it passes in 1/1.35 of the time, and the decoder reports
/// positions in track time (see [`DriftMonitor::set_platter_speed`]).
const TURN_SECS: f64 = 1.8;

/// How near a whole number of turns an offset jump must land to be a
/// skip. Decode loss across a gap is milliseconds to tens of them; a
/// skip is a turn give or take that.
const SKIP_TOLERANCE_SECS: f64 = 0.15;

/// The most turns one skip is taken for. Further than that the needle
/// was moved, and the lift and fallback rules decide.
const MAX_SKIP_TURNS: f64 = 2.0;

/// Contiguous near-silence longer than this means the needle left the
/// groove (or the platter sat stopped — harmless either way, since a
/// stop-and-release moves the offset by ~nothing and the small-jump
/// rule keeps the anchor). Turnaround dips last well under a second.
const LIFT_SILENCE_SECS: f64 = 1.5;

/// Anchored `groove − playhead` offset tracker. One per deck.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DriftMonitor {
    /// The offset captured at anchor time; NaN = unanchored.
    anchor: f64,
    /// Last measured drift in seconds; NaN until the first observation.
    drift: f64,
    /// Contiguous near-silent time so far (resets on signal).
    silent_secs: f64,
    /// Whether a sustained-silence lift occurred since the last
    /// observation — arms the remap rule.
    lift_seen: bool,
    /// The playhead was moved *deliberately* (seek, cue, load, internal
    /// play, panic hand-back) since the last observation — the next
    /// observation re-anchors unconditionally so the move is never
    /// counted as drift (or healed away).
    pending_remap: bool,
    /// Groove position at the most recent observation — the reference
    /// for the needle-relocation test after a silent gap. NaN until
    /// the first observation.
    last_groove: f64,
    /// One turn of the record in the seconds the groove is reported in:
    /// [`TURN_SECS`] at 33⅓, 1/1.35 of it at 45.
    turn_secs: f64,
}

impl DriftMonitor {
    pub(crate) const fn new() -> Self {
        Self {
            anchor: f64::NAN,
            drift: f64::NAN,
            silent_secs: 0.0,
            lift_seen: false,
            pending_remap: false,
            last_groove: f64::NAN,
            turn_secs: TURN_SECS,
        }
    }

    /// The platter turns `factor` times as fast as 33⅓ rpm (1.35 at 45),
    /// so a skip of one turn moves the groove by `TURN_SECS / factor` of
    /// track time.
    pub(crate) fn set_platter_speed(&mut self, factor: f64) {
        self.turn_secs = TURN_SECS / factor;
    }

    /// The playhead is about to move (or just moved) by command rather
    /// than by the platter: re-anchor on the next observation.
    pub(crate) fn note_remap(&mut self) {
        self.pending_remap = true;
    }

    /// Feed one block's carrier presence: `silent` when the amplitude
    /// is at the no-needle floor. Called every block, locked or not.
    pub(crate) fn note_block(&mut self, silent: bool, dt: f64) {
        if silent {
            self.silent_secs += dt;
            if self.silent_secs >= LIFT_SILENCE_SECS {
                self.lift_seen = true;
            }
        } else {
            self.silent_secs = 0.0;
        }
    }

    /// Record one ABS-locked observation and return the current drift
    /// in seconds (positive = the playhead lags the record).
    pub(crate) fn observe(&mut self, groove_secs: f64, playhead_secs: f64) -> f64 {
        let offset = groove_secs - playhead_secs;
        let jump = offset - self.anchor;
        // Needle relocation = a silent gap *and* the groove resuming
        // far from where the signal stopped. A hold or an in-place
        // scratch resumes nearby and keeps the anchor, however much
        // drift the gap's strokes accumulated.
        let groove_gap = (groove_secs - self.last_groove).abs();
        let relocated = self.lift_seen && groove_gap > RELOCATION_SECS;
        // Travel gestures (backspins etc.) skid the needle across
        // grooves with no silence — a big jump after one is physical,
        // not decode loss. Re-anchor; never late-correct it as drift.
        let skidded = groove_gap > TRAVEL_GESTURE_SECS && jump.abs() > SKID_JUMP_SECS;
        // A skip: since the last reading the offset moved by whole turns.
        let step = if self.drift.is_nan() {
            jump
        } else {
            jump - self.drift
        };
        let turns = (step / self.turn_secs).round();
        let skipped = turns != 0.0
            && turns.abs() <= MAX_SKIP_TURNS
            && (step - turns * self.turn_secs).abs() < SKIP_TOLERANCE_SECS;
        if self.anchor.is_nan() || self.pending_remap || relocated {
            self.anchor = offset;
        } else if skipped {
            self.anchor += turns * self.turn_secs;
        } else if skidded || jump.abs() > FALLBACK_REMAP_SECS {
            self.anchor = offset;
        }
        self.pending_remap = false;
        self.lift_seen = false;
        self.silent_secs = 0.0;
        self.last_groove = groove_secs;
        self.drift = offset - self.anchor;
        self.drift
    }

    /// Last measured drift in seconds (NaN before the first locked
    /// observation). Held across ABS dropouts — the next observation
    /// after a relative-only gap reveals exactly what the gap cost.
    pub(crate) fn drift_secs(&self) -> f64 {
        self.drift
    }

    /// Forget the anchor and the reading (input detach / track eject).
    pub(crate) fn reset(&mut self) {
        self.anchor = f64::NAN;
        self.drift = f64::NAN;
        self.silent_secs = 0.0;
        self.lift_seen = false;
        self.pending_remap = false;
        self.last_groove = f64::NAN;
    }
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
mod tests {
    use super::*;

    const DT: f64 = 64.0 / 48_000.0;

    fn feed_silence(m: &mut DriftMonitor, secs: f64) {
        let blocks = (secs / DT).ceil() as usize;
        for _ in 0..blocks {
            m.note_block(true, DT);
        }
    }

    #[test]
    fn first_observation_anchors_at_zero_drift() {
        let mut m = DriftMonitor::new();
        assert!(m.drift_secs().is_nan());
        let d = m.observe(42.0, 10.0);
        assert!(d.abs() < 1e-12, "first observation must read 0: {d}");
    }

    #[test]
    fn accumulated_offset_change_reads_as_drift() {
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        // The playhead fell 12 ms behind the groove (e.g. turnaround
        // losses): groove advanced 5 s, playhead only 4.988.
        let d = m.observe(47.0, 14.988);
        assert!((d - 0.012).abs() < 1e-9, "drift misread: {d}");
    }

    #[test]
    fn scratch_session_drift_is_not_mistaken_for_a_remap() {
        // The on-rig failure: 30 s of scratching (no ABS lock, no
        // sustained silence — only sub-second turnaround dips), then
        // the first re-lock reveals −170 ms in one jump. That is
        // drift and must be reported, however large.
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        for _ in 0..200 {
            m.note_block(true, DT); // a turnaround dip…
            m.note_block(false, DT); // …signal returns immediately
        }
        let d = m.observe(72.0, 40.17);
        assert!(
            (d - (-0.17)).abs() < 1e-9,
            "scratch drift was discarded as a remap: {d}"
        );
    }

    #[test]
    fn needle_lift_to_a_new_spot_reanchors() {
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        feed_silence(&mut m, 2.0); // needle up
                                   // Dropped 30 s away: a deliberate remap, not drift.
        let d = m.observe(75.0, 11.0);
        assert!(d.abs() < 1e-12, "lift+drop must re-anchor: {d}");
        // Accounting continues from the new anchor.
        let d = m.observe(76.0, 11.997);
        assert!((d - 0.003).abs() < 1e-9, "post-remap drift misread: {d}");
    }

    #[test]
    fn lift_back_onto_the_sticker_keeps_the_measurement() {
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        let d = m.observe(47.0, 14.95); // 50 ms of real drift so far
        assert!((d - 0.05).abs() < 1e-9);
        feed_silence(&mut m, 3.0); // lift (or stop-and-hold)…
                                   // …and back on the sticker: offset ≈ unchanged, so the
                                   // running total must survive, not reset to zero.
        let d = m.observe(47.5, 15.44);
        assert!(
            (d - 0.06).abs() < 1e-9,
            "sticker re-drop lost the total: {d}"
        );
    }

    #[test]
    fn programmatic_jump_reanchors_without_a_lift() {
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        // A cue/seek moved the playhead 30 s with no silence at all.
        let d = m.observe(43.0, 41.0);
        assert!(d.abs() < 1e-12, "seek must re-anchor via fallback: {d}");
    }

    #[test]
    fn hold_heavy_scratch_session_keeps_anchor_and_reports_full_drift() {
        // The on-rig "a whole bar off and the display says 0.2 ms":
        // scratching the same spot with the record *held stopped*
        // between strokes. Holds are silent (≥ lift threshold), and
        // the accumulated stroke losses exceed any fixed offset
        // threshold — but the needle resumes at (nearly) the same
        // groove position every time, so the anchor must survive and
        // the full drift must be reported (and healed), not swallowed
        // by a re-anchor.
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        // 30 strokes: each holds silent for 2 s (arms the lift rule),
        // the groove wanders ±0.3 s in place, and the playhead loses
        // ~15 ms per stroke — 450 ms total, way past any offset gate.
        let mut playhead_deficit = 0.0;
        for stroke in 0..30 {
            feed_silence(&mut m, 2.0);
            playhead_deficit += 0.015;
            let groove = 42.0 + if stroke % 2 == 0 { 0.3 } else { -0.3 };
            let d = m.observe(groove, groove - 32.0 - playhead_deficit);
            assert!(
                (d - playhead_deficit).abs() < 1e-9,
                "stroke {stroke}: drift swallowed — read {d}, expected {playhead_deficit}"
            );
        }
    }

    #[test]
    fn backspin_needle_skid_reanchors_instead_of_jumping_the_song() {
        // A backspin throws the groove back ~10 s with no silence. The
        // tracked playhead followed most of it, but the record and the
        // track came out 2 s apart — not a whole number of turns, so not
        // a clean skip, and after that much travel not decode loss
        // either. Relative mode re-anchors silently; healing it would
        // audibly jump the song a beat after the release (the on-rig
        // report).
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        let d = m.observe(32.0, 2.0); // groove −10 s, playhead −8 s
        assert!(d.abs() < 1e-12, "skid healed as drift: {d}");
        // Precision accounting continues from the new anchor.
        let d = m.observe(33.0, 2.99);
        assert!((d - 0.01).abs() < 1e-9, "post-skid drift misread: {d}");
    }

    #[test]
    fn a_skip_during_a_backspin_keeps_the_sticker_and_heals_the_rest() {
        // The same backspin, but the needle jumped one groove and the
        // decode lost 30 ms besides: the turn is kept, the 30 ms healed.
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        let d = m.observe(32.0 - TURN_SECS, 0.03); // groove −11.8, playhead −9.97
        assert!((d - (-0.03)).abs() < 1e-9, "backspin skip misread: {d}");
    }

    #[test]
    fn a_skip_inside_a_scratch_keeps_the_sticker() {
        // Scratching in place (no long travel, no silence) and the needle
        // jumps a groove: the offset moves by one turn plus the 20 ms the
        // decode lost meanwhile. The turn is the skip and is kept; the
        // 20 ms is drift.
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        let d = m.observe(42.5 + TURN_SECS, 10.48);
        assert!((d - 0.02).abs() < 1e-9, "skip read as drift: {d}");
        // And the accounting carries on from there.
        let d = m.observe(43.5 + TURN_SECS, 11.48);
        assert!((d - 0.02).abs() < 1e-9, "after the skip: {d}");
    }

    #[test]
    fn skips_either_way_and_two_turns_keep_the_sticker() {
        for turns in [-2.0, -1.0, 1.0, 2.0] {
            let mut m = DriftMonitor::new();
            m.observe(42.0, 10.0);
            m.observe(43.0, 10.99); // 10 ms of drift so far
            let d = m.observe(43.2 + turns * TURN_SECS, 11.185);
            assert!(
                (d - 0.015).abs() < 1e-9,
                "{turns} turns: the skip moved the reading to {d}"
            );
        }
    }

    #[test]
    fn at_45_a_turn_is_shorter_in_track_time() {
        // At 45 one turn passes in 1.8 / 1.35 = 1.333 s of track time. A
        // skip of one turn there is a skip; a 33⅓ turn's 1.8 s is not.
        let turn_45 = TURN_SECS / 1.35;
        let mut m = DriftMonitor::new();
        m.set_platter_speed(1.35);
        m.observe(42.0, 10.0);
        let d = m.observe(42.5 + turn_45, 10.48);
        assert!((d - 0.02).abs() < 1e-9, "a 45 skip read as drift: {d}");
        let mut m = DriftMonitor::new();
        m.set_platter_speed(1.35);
        m.observe(42.0, 10.0);
        let d = m.observe(42.5 + TURN_SECS, 10.48);
        assert!(
            (d - 0.02 - TURN_SECS).abs() < 1e-9,
            "a 33⅓ turn kept at 45: {d}"
        );
    }

    #[test]
    fn a_jump_between_whole_turns_is_not_a_skip() {
        // 0.9 s is half a turn: no skip makes that. With no lift and no
        // long travel it stays drift, to be reported and healed.
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        let d = m.observe(43.9, 11.0);
        assert!((d - 0.9).abs() < 1e-9, "half a turn taken for a skip: {d}");
    }

    #[test]
    fn small_loss_during_travel_gesture_still_heals() {
        // A clean (no-skid) long backward ride: groove travels far but
        // the decode only lost 80 ms — under the skid threshold, so it
        // is decode loss and must be reported/healed.
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        let d = m.observe(32.0, 0.08); // groove −10 s, playhead −9.92 s
        assert!((d - (-0.08)).abs() < 1e-9, "travel loss swallowed: {d}");
    }

    #[test]
    fn deliberate_seek_reanchors_via_note_remap() {
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        let d = m.observe(47.0, 14.95); // 50 ms of real drift so far
        assert!((d - 0.05).abs() < 1e-9);
        // A cue jump of 1 s — too small for the magnitude fallback, no
        // lift involved. The command path flags it; the move must be
        // absorbed, not reported (or healed) as drift.
        m.note_remap();
        let d = m.observe(48.0, 14.95);
        assert!(d.abs() < 1e-12, "seek counted as drift: {d}");
        // Accounting continues from the new anchor afterwards.
        let d = m.observe(49.0, 15.94);
        assert!((d - 0.01).abs() < 1e-9, "post-seek drift misread: {d}");
    }

    #[test]
    fn reset_clears_anchor_and_reading() {
        let mut m = DriftMonitor::new();
        m.observe(42.0, 10.0);
        m.observe(47.0, 14.9);
        m.reset();
        assert!(m.drift_secs().is_nan());
        let d = m.observe(100.0, 3.0);
        assert!(d.abs() < 1e-12, "post-reset must re-anchor at 0: {d}");
    }
}
