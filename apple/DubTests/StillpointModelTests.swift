//
//  StillpointModelTests.swift
//  DubTests
//
//  Exercises the Stillpoint engine (role inference, stage FSM,
//  octave fold, certification gates, coach) against scripted
//  two-deck traces at 60 Hz — the property-test prerequisite the
//  design doc demands before any rig time.
//

import XCTest

@testable import Dub

final class StillpointModelTests: XCTestCase {

    // MARK: - Rig harness

    /// A scripted two-deck rig: mutate `a`/`b` between `run` calls;
    /// playheads advance at each deck's pitched rate while playing.
    private final class Rig {
        let engine = StillpointEngine()
        var now = 0.0
        var a = StillpointDeckInputs()
        var b = StillpointDeckInputs()
        var frame = StillpointFrame()
        /// Simulates a display-pitch lie: B's groove advances at this
        /// rate regardless of what `b.pitchPercent` claims.
        var bTrueRate: Double?

        @discardableResult
        func run(_ secs: Double) -> StillpointFrame {
            let dt = 1.0 / 60.0
            var t = 0.0
            while t < secs {
                now += dt
                if a.isPlaying { a.playheadSecs += dt * (1 + (a.pitchPercent ?? 0) / 100) }
                if b.isPlaying {
                    b.playheadSecs += dt * (bTrueRate ?? (1 + (b.pitchPercent ?? 0) / 100))
                }
                frame = engine.update(now: now, a: a, b: b)
                t += dt
            }
            return frame
        }

        /// Hand-blip deck B (lockState 3 for `secs`), the role signal.
        func blipB(_ secs: Double = 0.3) {
            b.timecodeLockState = 3
            run(secs)
            b.timecodeLockState = 1
        }
    }

    private func tcDeck(bpm: Double, conf: Double = 0.9,
                        pitch: Double = 0) -> StillpointDeckInputs {
        var d = StillpointDeckInputs()
        d.hasTrack = true
        d.isPlaying = true
        d.bpm = bpm
        d.bpmConfidence = conf
        d.gridAnchorSecs = 0
        d.pitchPercent = pitch
        d.pitchSettled = true
        d.timecodeLockState = 1
        d.isTimecodeDriven = true
        return d
    }

    /// Both decks at `bpm`, B blipped so it becomes the incoming deck;
    /// runs until the clean-play path lands the FSM in PHASE.
    private func mixRig(bpm: Double = 120, confB: Double = 0.9,
                        pitchB: Double = 0, bOffsetSecs: Double = 0) -> Rig {
        let rig = Rig()
        rig.a = tcDeck(bpm: bpm)
        rig.b = tcDeck(bpm: bpm, conf: confB, pitch: pitchB)
        rig.b.playheadSecs = bOffsetSecs
        rig.run(1)
        rig.blipB()
        rig.run(4)
        return rig
    }

    private func internalDeck(bpm: Double, conf: Double = 0.9,
                              playing: Bool = true) -> StillpointDeckInputs {
        var d = StillpointDeckInputs()
        d.hasTrack = true
        d.isPlaying = playing
        d.bpm = bpm
        d.bpmConfidence = conf
        d.gridAnchorSecs = 0
        d.pitchPercent = playing ? 0 : nil
        d.isTimecodeDriven = false
        return d
    }

    // MARK: - Internal (file) mode

    func test_internal_idleLoadedDeckIsIncoming() {
        let rig = Rig()
        rig.a = internalDeck(bpm: 120)
        rig.b = internalDeck(bpm: 120, playing: false)
        rig.run(1.5)
        XCTAssertEqual(rig.frame.incomingIsA, false,
                       "the deck that isn't playing is the one being prepped")
        XCTAssertEqual(rig.frame.stage, .tempo)
    }

    func test_internal_pressPlayIsTheDrop() {
        let rig = Rig()
        rig.a = internalDeck(bpm: 120)
        rig.b = internalDeck(bpm: 120, playing: false)
        rig.run(1.5)
        rig.b.isPlaying = true
        rig.b.pitchPercent = 0
        rig.run(0.2)
        XCTAssertEqual(rig.frame.stage, .phase)
        XCTAssertNotNil(rig.frame.dropFiredAgo,
                        "pressing play on a file deck is the drop")
        rig.run(1)
        XCTAssertNotNil(rig.frame.phaseMs, "band live after the internal drop")
    }

    func test_internal_seekBackRecues() {
        let rig = Rig()
        rig.a = internalDeck(bpm: 120)
        rig.b = internalDeck(bpm: 120, playing: false)
        rig.run(1.5)
        rig.b.isPlaying = true
        rig.b.pitchPercent = 0
        rig.run(2)
        XCTAssertEqual(rig.frame.stage, .phase)
        rig.b.playheadSecs -= 2.0
        rig.run(0.2)
        XCTAssertEqual(rig.frame.stage, .tempo,
                       "a seek back > 1 beat is the internal re-cue")
    }

    func test_internal_laterStarterIsIncoming() {
        let rig = Rig()
        rig.a = internalDeck(bpm: 120)
        var b = internalDeck(bpm: 120, playing: false)
        b.hasTrack = false                  // nothing loaded yet
        rig.b = b
        rig.run(5)
        XCTAssertNil(rig.frame.incomingIsA)
        rig.b = internalDeck(bpm: 120)      // loads + starts mid-set
        rig.b.playheadSecs = rig.a.playheadSecs
        rig.run(1.5)
        XCTAssertEqual(rig.frame.incomingIsA, false,
                       "both playing → the later starter is incoming")
    }

    func test_internal_seeksNeverArmCoach() {
        let rig = Rig()
        rig.a = internalDeck(bpm: 120)
        rig.b = internalDeck(bpm: 120, playing: false)
        rig.b.playheadSecs = -0.060
        rig.run(1.5)
        rig.b.isPlaying = true
        rig.b.pitchPercent = 0
        rig.run(2)
        // Three seek-corrections toward the line — jumps, not nudges.
        for _ in 0..<3 {
            rig.run(3)
            rig.b.playheadSecs += 0.055
            rig.run(1)
            rig.b.playheadSecs -= 0.055
        }
        rig.run(1)
        XCTAssertNil(rig.frame.coach,
                     "no needle, no nudges — seeks must never arm the coach")
    }

    func test_wrapPinsAtTheRailInsteadOfTeleporting() {
        // The incoming runs fast from +180 ms: the display watches it
        // climb to the +rail, must pin past it (no teleport), then
        // release to the far side only 0.15 beat past the rail.
        let rig = mixRig(pitchB: 0.48, bOffsetSecs: 0.180)
        XCTAssertGreaterThan(rig.frame.phaseMs ?? 0, 150, "established pre-rail")
        rig.run(11)
        XCTAssertGreaterThan(rig.frame.phaseMs ?? 0, 245,
                             "just past the rail the band pins, no sign flip")
        rig.run(15)
        XCTAssertLessThan(rig.frame.phaseMs ?? 0, 0,
                          "0.15 beat past the rail the display releases")
        XCTAssertNil(rig.frame.coach, "wrap transit never arms the coach")
    }

    // MARK: - Roles

    func test_handEventsFrameTheIncomingDeck() {
        let rig = Rig()
        rig.a = tcDeck(bpm: 120)
        rig.b = tcDeck(bpm: 120)
        rig.run(1)
        XCTAssertNil(rig.frame.incomingIsA, "no evidence yet — no frame")
        rig.blipB()
        rig.run(2.5)
        XCTAssertEqual(rig.frame.incomingIsA, false,
                       "the deck the hand touches is the incoming deck")
    }

    // MARK: - Stage FSM

    func test_cleanPlayLandsInPhase() {
        let rig = mixRig()
        XCTAssertEqual(rig.frame.stage, .phase)
    }

    func test_dropEdgeFiresFromTempo() {
        let rig = mixRig()
        // Re-cue: long hold + backjump exits to TEMPO…
        rig.b.timecodeLockState = 3
        rig.b.isPlaying = false
        rig.b.playheadSecs -= 2.0
        rig.run(1.0)
        XCTAssertEqual(rig.frame.stage, .tempo, "hold + backjump = re-cue")
        // …and the release into play is the drop edge.
        rig.b.timecodeLockState = 1
        rig.b.isPlaying = true
        rig.run(0.25)
        XCTAssertEqual(rig.frame.stage, .phase)
        XCTAssertNotNil(rig.frame.dropFiredAgo, "drop materialize fired")
    }

    func test_nudgeTouchNeverFlipsTheFace() {
        let rig = mixRig()
        rig.b.timecodeLockState = 3
        rig.run(0.3)                       // short touch, no backjump
        rig.b.timecodeLockState = 1
        rig.run(0.3)
        XCTAssertEqual(rig.frame.stage, .phase,
                       "a nudge touch can never flip the face")
    }

    func test_rideHysteresis_12in_25out() {
        let rig = mixRig(bOffsetSecs: 0.002)
        rig.run(5)                          // |φ| ≈ 2 ms < 12, ≥ 8 beats
        XCTAssertEqual(rig.frame.stage, .ride)
        rig.b.playheadSecs += 0.030         // slip to ≈ +32 ms > 25
        rig.run(2)
        XCTAssertEqual(rig.frame.stage, .phase)
    }

    // MARK: - Phase math

    func test_octaveFold_halftimeBlendReadsLocked() {
        let rig = Rig()
        rig.a = tcDeck(bpm: 87)             // master, halftime
        rig.b = tcDeck(bpm: 174)            // incoming, double
        rig.run(1)
        rig.blipB()
        rig.run(4)
        XCTAssertEqual(rig.frame.foldK, 1)
        XCTAssertEqual(rig.frame.deltaBpm ?? 99, 0, accuracy: 0.01,
                       "87↔174 is a matched blend after folding")
        XCTAssertEqual(rig.frame.phaseMs ?? 99, 0, accuracy: 1.5,
                       "band frozen on the line at a perfect 2:1 lock")
    }

    func test_lateIncomingReadsNegativePhase() {
        let rig = mixRig(bOffsetSecs: -0.020)
        XCTAssertEqual(rig.frame.phaseMs ?? 0, -20, accuracy: 2,
                       "late = negative = band below the line = push")
    }

    // MARK: - Certification honesty

    func test_falseGreenImpossible_lowConfidence() {
        let rig = mixRig(confB: 0.4)        // perfect alignment, weak grid
        rig.run(6)
        XCTAssertEqual(rig.frame.lock, StillpointLock.none,
                       "no confidence → no lock, no matter how seated")
    }

    func test_whiteBeforeGreen_untrustedGrid() {
        let rig = mixRig(confB: 0.6)        // above gate, below trust
        rig.run(6)
        XCTAssertEqual(rig.frame.lock, .white,
                       "seated + gates but untrusted grid renders white")
    }

    func test_greenNeedsTrustedGridsAndHoldGrows() {
        let rig = mixRig()
        rig.run(6)
        XCTAssertEqual(rig.frame.lock, .green)
        XCTAssertGreaterThan(rig.frame.holdBeats, 2)
        // Any gate failing cools it the same frame(s).
        rig.b.pitchSettled = false
        rig.run(0.2)
        XCTAssertEqual(rig.frame.lock, StillpointLock.none)
        XCTAssertNotNil(rig.frame.lockBrokenAgo, "revoke is an event")
    }

    // MARK: - Honesty under the hand

    func test_handFreezesPhaseAtLastHonestOffset() {
        let rig = mixRig(bOffsetSecs: -0.020)
        let before = rig.frame.deposits.count
        rig.b.timecodeLockState = 3
        rig.run(0.4)
        XCTAssertNil(rig.frame.phaseMs)
        XCTAssertEqual(rig.frame.frozenPhaseMs ?? 0, -20, accuracy: 3,
                       "ghost stays at the last honest offset")
        XCTAssertLessThanOrEqual(rig.frame.deposits.count, before,
                                 "deposits abstain while the hand is down")
        rig.b.timecodeLockState = 1
    }

    // MARK: - Tempo belt

    func test_beltReadsGrooveTruthNotDisplayPitch() {
        // The engine's pitch% is display-filtered (lags, steps). The
        // belt must read the measured slip from the playheads instead:
        // here B's pitch INPUT claims 0.0 while the groove actually
        // runs 1% fast — the belt still has to move.
        let rig = Rig()
        rig.a = tcDeck(bpm: 120)
        rig.b = tcDeck(bpm: 120)
        rig.bTrueRate = 1.01
        rig.run(1)
        rig.blipB()
        rig.run(2.7)
        XCTAssertEqual(rig.frame.stage, .tempo)
        XCTAssertGreaterThan(rig.frame.beltVelocityPx, 0,
                             "slip is groove truth — a lying pitch readout can't freeze the belt")
        XCTAssertGreaterThan(rig.frame.deltaBpm ?? 0, 0.5,
                             "Δ fine print comes from measured slip")
    }

    func test_beltMovesWithTempoErrorAndFreezesAtNull() {
        let rig = Rig()
        rig.a = tcDeck(bpm: 120)
        rig.b = tcDeck(bpm: 120, pitch: 1.0)
        rig.b.isPlaying = false             // cued, not yet dropped
        rig.run(1)
        rig.blipB()
        rig.run(2.5)
        XCTAssertEqual(rig.frame.stage, .tempo)
        XCTAssertGreaterThan(rig.frame.beltVelocityPx, 0,
                             "incoming fast → belt climbs")
        rig.b.pitchPercent = 0
        rig.run(0.5)
        XCTAssertEqual(rig.frame.beltVelocityPx, 0,
                       "tempo matched → the world freezes")
    }

    // MARK: - Ride coach

    func test_coachQuotesThreeSameWayNudges() {
        let rig = mixRig(pitchB: -0.2)      // ≈ −2 ms/s drift: B runs slow
        for _ in 0..<3 {
            rig.run(9)                       // drift out ≈ −20 ms
            rig.b.playheadSecs += 0.018      // push it back near the line
            rig.run(0.6)
        }
        rig.run(1)
        let coach = rig.frame.coach
        XCTAssertNotNil(coach, "three same-way pushes + agreeing drift arm the coach")
        XCTAssertEqual(coach?.pitchUp, true, "kept pushing → pitch up")
        XCTAssertEqual(coach?.trimPercent ?? 0, 0.2, accuracy: 0.1)
        // Clears silently once the fader answers the call.
        rig.b.pitchPercent = 0
        rig.run(1)
        XCTAssertNil(rig.frame.coach)
    }
}
