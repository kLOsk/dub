//
//  StillpointModel.swift
//  Dub
//
//  Pure logic for the Stillpoint beatmatch aid (round 3, see
//  docs/investigations/BEATMATCH-AID-STILLPOINT.md): role inference
//  (master vs incoming), workflow-stage FSM, octave-folded phase /
//  tempo math, honesty gating, lock certification, deposits, and the
//  ride-stage pitch-trim coach. No UI imports; deterministic per
//  (now, inputs) so the whole machine is unit-testable.
//

import Foundation

// MARK: - Inputs

/// Per-deck signals sampled at one frame timestamp. Mirrors the
/// fields `DeckState` + `positionSnapshot` already provide; the
/// future grid→ODF swap replaces only the φ/Δ producers, never
/// these transport facts.
struct StillpointDeckInputs {
    var hasTrack = false
    var isPlaying = false
    var bpm: Double?
    var bpmConfidence: Double = 0
    var gridAnchorSecs: Double?
    var pitchPercent: Double?
    var pitchSettled = true
    var timecodeLockState: UInt8 = 0
    var isTimecodeDriven = false
    var playheadSecs: Double = 0
}

// MARK: - Output frame

enum StillpointStage: Equatable {
    case hidden, neutral, tempo, phase, ride
}

enum StillpointLock: Equatable {
    /// `white` = band seated and every gate holds, but the grid isn't
    /// trusted yet (pre-ODF clause) — as true as the inputs, no more.
    case none, white, green
}

struct StillpointDeposit {
    var ageSecs: Double
    var phaseMs: Double
}

struct StillpointCoach {
    /// true = "pitch up" (`+`), the incoming deck is running slow.
    var pitchUp: Bool
    var trimPercent: Double
    var armedAgoSecs: Double
}

/// Everything the renderer needs for one frame. The renderer never
/// reaches back into the engine.
struct StillpointFrame {
    var stage: StillpointStage = .hidden
    var prevStage: StillpointStage = .hidden
    /// 0→1 crossfade progress since the last stage change.
    var stageProgress: Double = 1
    /// nil while roles are unresolved (NEUTRAL / HIDDEN).
    var incomingIsA: Bool?
    /// Honesty multiplier for every element, floor 0.15.
    var alpha: Double = 1
    /// Live filtered phase offset, incoming − master, ms. nil when
    /// the hand owns the record or the carrier is gone.
    var phaseMs: Double?
    /// Stale-frozen ghost position (last honest offset) when live
    /// phase is nil. Never slides toward the line. The view fades it
    /// out over a few seconds — stale should look stale, then leave.
    var frozenPhaseMs: Double?
    var frozenAgeSecs: Double?
    /// Grid/BPM missing — band retracts, belt-only or empty.
    var withdrawn = false
    var deltaBpm: Double?
    var foldK: Int = 0
    /// Tempo-belt scroll offset in px (positive = upward) and the
    /// current velocity; frozen=true renders hollow ghost dots.
    var beltOffsetPx: Double = 0
    var beltVelocityPx: Double = 0
    var beltFrozen = false
    var lock: StillpointLock = .none
    var holdBeats: Double = 0
    /// Seconds since lock revoke, while the shatter animation runs.
    var lockBrokenAgo: Double?
    /// Seconds since the drop edge fired, while the materialize
    /// animation runs.
    var dropFiredAgo: Double?
    var deposits: [StillpointDeposit] = []
    var coach: StillpointCoach?
    var showFinePrint = true
    /// Half a beat in ms (fast fold domain) — the band's clamp range.
    var halfBeatMs: Double = 250
}

// MARK: - Tuning

/// Every threshold in one place. Values trace to the design doc;
/// the offline scrub harness is the place to recalibrate them.
enum StillpointTuning {
    // Geometry/scale (the model speaks ms; px conversion is the view's).
    // 0.75 px/ms per the round-4 rig verdict: at 1.5 the band's
    // near-match movement read as alarming although the drift was
    // inaudible — halved across the whole curve so the close-in
    // acceleration keeps its shape.
    static let pxPerMs = 0.75
    static let linearRangeMs = 40.0
    /// The pocket: |φ| within this sounds clean — beats don't have to
    /// match 100 %. ~the audibility threshold of a kick flam.
    static let pocketMs = 10.0

    // Roles.
    static let handEventDecaySecs = 10.0
    static let backjumpDecaySecs = 15.0
    static let playStartWindowSecs = 30.0
    static let tenureDiscount = 0.25
    static let tenureBeats = 64.0
    static let roleMargin = 0.6
    static let roleMarginHoldSecs = 2.0
    static let roleSwapHoldSecs = 5.0
    static let masterStopSwapSecs = 3.0
    static let neutralScoreFloor = 0.2
    static let neutralCleanPlaySecs = 60.0
    static let bootstrapHoldSecs = 1.0
    static let bootstrapStartSeparationSecs = 2.0

    // Stages.
    static let minDwellSecs = 1.2
    static let dropDebounceSecs = 0.25
    static let dropConfirmSecs = 0.15
    static let tempoCleanPlayBeats = 4.0
    static let rideInMs = 12.0
    static let rideInBeats = 8.0
    static let rideOutMs = 25.0
    static let rideOutBeats = 2.0
    static let recueHoldSecs = 0.6
    static let crossfadeSecs = 0.35

    // Belt.
    static let beltPitchPx = 56.0
    static let beltDeadbandBpm = 0.015
    static let beltSignFlipBpm = 0.03
    static let beltBaseVelocity = 3.0
    static let beltVelocityPerBpm = 55.0
    static let beltMaxVelocity = 110.0

    // Phase display.
    static let medianWindowSecs = 0.18
    static let motionFloorMs = 0.35

    // Slip — groove-truth tempo error, dφ/dt measured from the
    // playheads. The engine's pitch% readout is display-filtered
    // (median + persistence-gated pole) for steady header digits;
    // a strobe null needs the actual relative rate, not the digits.
    static let slipWindowSecs = 0.8
    static let slipMinSpanSecs = 0.35
    static let slipMinSamples = 8

    // Octave fold.
    static let foldAdoptTolerance = 0.06
    static let foldDropTolerance = 0.25
    static let foldStableSecs = 1.0

    // Certification.
    static let greenPhaseMs = 8.0
    static let greenPhaseBeats = 4.0
    static let greenDeltaBpm = 0.02
    static let greenMinConfidence = 0.5
    static let gridTrustConfidence = 0.8
    static let greenAttackSecs = 0.6

    // Deposits.
    static let depositFadeSecs = 8.0
    static let depositCap = 32

    // Coach.
    static let nudgeStepMs = 15.0
    static let nudgeCutFraction = 0.4
    static let nudgeLookbackSecs = 0.5
    static let nudgeDebounceBeats = 2.0
    static let handBlipMaxSecs = 1.2
    static let driftEmaBeats = 16.0
    static let coachWindowBeats = 64.0
    static let coachMinCorrections = 3
    static let coachMinDriftMsPerSec = 0.4
    static let coachTrimStep = 0.05
    static let coachTrimMin = 0.05
    static let coachTrimMax = 0.8

    // Honesty.
    static let confidenceFloor = 0.15
    static let degradedCarrierAlpha = 0.6
}

// MARK: - Engine

final class StillpointEngine {

    private typealias T = StillpointTuning

    private struct DeckTrack {
        var handDown = false
        var handDownSince: Double?
        var handEvents: [Double] = []
        var backjumpEvents: [Double] = []
        var playStartedAt: Double?
        var cleanPlaySince: Double?
        var backjumpDuringHold = false
        /// 3→1 edge seen on this frame (computed before prevLock is
        /// overwritten — the stage FSM reads it after event tracking).
        var releasedThisFrame = false
        /// stopped→playing edge this frame — an internal deck's drop.
        var startedThisFrame = false
        /// Backward jump > 1 beat this frame — an internal deck's re-cue.
        var backjumpThisFrame = false
        var prevLock: UInt8 = 0
        var prevPlaying = false
        var prevPlayhead: Double?
    }

    private var decks = [DeckTrack(), DeckTrack()]

    // Roles.
    private var incomingIdx: Int?
    private var marginHeldSince: Double?
    private var swapHeldSince: Double?
    private var masterStoppedSince: Double?
    private var bootstrapCandidate: Int?
    private var bootstrapSince = 0.0

    // Stage.
    private var stage: StillpointStage = .hidden
    private var prevStage: StillpointStage = .hidden
    private var stageChangedAt = -1e9
    private var pendingDropAt: Double?
    private var dropFiredAt: Double?

    // Fold.
    private var foldK = 0
    private var foldCandidate: Int?
    private var foldCandidateSince = 0.0

    // Phase.
    private var phaseRing: [(t: Double, ms: Double)] = []
    private var coachRing: [(t: Double, ms: Double)] = []
    private var displayPhaseMs: Double?
    private var frozenPhaseMs: Double?
    private var frozenAt: Double?
    private var fastBeatMs = 500.0
    /// Sticky half-beat wrap sign: ±1 once a side is shown, 0 unset.
    private var phaseDispSign = 0.0
    // Slip estimator (least-squares dφ/dt over slipWindowSecs).
    private var slipRing: [(t: Double, ms: Double)] = []
    private var slipMsPerSec: Double?

    // Belt.
    private var beltOffset = 0.0
    private var beltSign = 0.0
    private var beltVelocity = 0.0

    // Deposits.
    private var deposits: [(t: Double, ms: Double)] = []
    private var prevMasterBeatPhase: Double?
    private var masterBeatCrossed = false

    // Certification.
    private var phaseInGreenSince: Double?
    private var gatesPassSince: Double?
    private var lock: StillpointLock = .none
    private var lockBrokenAt: Double?
    private var holdBeats = 0.0

    // Ride hysteresis.
    private var phaseInRideSince: Double?
    private var phaseOutRideSince: Double?

    // Coach.
    private var driftEMA: Double?
    private var lastDriftSample: (t: Double, ms: Double)?
    private var corrections: [(t: Double, sign: Double)] = []
    private var lastNudgeAt = -1e9
    private var coach: (pitchUp: Bool, trim: Double, armedAt: Double, pitchAtArm: Double?)?
    private var handBlipPhaseMs: Double?
    private var handBlipStartedAt: Double?

    private var lastNow: Double?

    // MARK: Update

    func update(now: Double, a: StillpointDeckInputs, b: StillpointDeckInputs) -> StillpointFrame {
        let inputs = [a, b]
        let dt = lastNow.map { max(0, now - $0) } ?? 0
        lastNow = now

        trackDeckEvents(now: now, inputs: inputs)
        resolveRoles(now: now, inputs: inputs)

        let live = (deckLive(inputs[0]), deckLive(inputs[1]))
        let fold = updateFold(now: now, inputs: inputs, live: live)
        let phase = updatePhase(now: now, inputs: inputs, live: live, fold: fold)
        let deltaEff = effectiveDeltaBpm(live: live, fold: fold)
        trackMasterBeat(live: live)
        updateStage(now: now, inputs: inputs, live: live, phaseMs: phase.live)
        updateBelt(now: now, dt: dt, inputs: inputs, deltaBpm: deltaEff)
        updateDeposits(now: now, inputs: inputs, live: live, phaseMs: phase.live)
        updateCertification(now: now, inputs: inputs, live: live,
                            phaseMs: phase.live, deltaBpm: deltaEff)
        updateCoach(now: now, inputs: inputs, live: live, phaseMs: phase.live)

        return makeFrame(now: now, inputs: inputs, live: live, fold: fold,
                         phase: phase, deltaEff: deltaEff)
    }

    // MARK: Per-deck live math

    private struct DeckLive {
        var liveBpm: Double
        var beatPhase: Double
        var beatSecs: Double
    }

    private func gridValid(_ d: StillpointDeckInputs) -> Bool {
        d.hasTrack && (d.bpm ?? 0) > 0 && d.gridAnchorSecs != nil
            && d.bpmConfidence >= T.confidenceFloor
    }

    private func deckLive(_ d: StillpointDeckInputs) -> DeckLive? {
        guard gridValid(d), let bpm = d.bpm, let anchor = d.gridAnchorSecs else { return nil }
        let liveBpm = bpm * (1 + (d.pitchPercent ?? 0) / 100)
        guard liveBpm > 0 else { return nil }
        var frac = (d.playheadSecs - anchor) * bpm / 60
        frac -= frac.rounded(.down)
        return DeckLive(liveBpm: liveBpm, beatPhase: frac, beatSecs: 60 / liveBpm)
    }

    /// The DJ's hand owns this record (or the carrier is gone): the
    /// phase readout is not live. Internal decks are never hand-held.
    private func phaseSuspended(_ d: StillpointDeckInputs) -> Bool {
        guard d.isTimecodeDriven else { return !d.isPlaying }
        return d.timecodeLockState == 3 || d.timecodeLockState == 0
            || d.pitchPercent == nil
    }

    // MARK: Deck event tracking

    private func trackDeckEvents(now: Double, inputs: [StillpointDeckInputs]) {
        for i in 0...1 {
            let d = inputs[i]
            var t = decks[i]
            t.releasedThisFrame = t.prevLock == 3 && d.timecodeLockState == 1
            let handNow = d.isTimecodeDriven && d.timecodeLockState == 3
            if handNow && !t.handDown {
                t.handEvents.append(now)
                t.handDownSince = now
                t.backjumpDuringHold = false
            }
            if !handNow { t.handDownSince = nil }
            t.handDown = handNow

            t.startedThisFrame = d.isPlaying && !t.prevPlaying
            if t.startedThisFrame { t.playStartedAt = now }
            if d.isPlaying && !handNow {
                if t.cleanPlaySince == nil { t.cleanPlaySince = now }
            } else {
                t.cleanPlaySince = nil
            }

            t.backjumpThisFrame = false
            if let prev = t.prevPlayhead, let bpm = d.bpm, bpm > 0 {
                let beatSecs = 60 / bpm
                if prev - d.playheadSecs > beatSecs {
                    t.backjumpEvents.append(now)
                    t.backjumpThisFrame = true
                    if t.handDown { t.backjumpDuringHold = true }
                }
            }
            t.prevPlayhead = d.playheadSecs

            t.handEvents.removeAll { now - $0 > T.handEventDecaySecs * 5 }
            t.backjumpEvents.removeAll { now - $0 > T.backjumpDecaySecs * 5 }
            t.prevLock = d.timecodeLockState
            t.prevPlaying = d.isPlaying
            decks[i] = t
        }
    }

    // MARK: Roles

    private func cleanPlaySecs(_ i: Int, now: Double) -> Double {
        decks[i].cleanPlaySince.map { now - $0 } ?? 0
    }

    private func tenured(_ i: Int, now: Double, inputs: [StillpointDeckInputs]) -> Bool {
        guard incomingIdx == 1 - i else { return false }
        let bpm = inputs[i].bpm ?? 0
        guard bpm > 0 else { return false }
        return cleanPlaySecs(i, now: now) * bpm / 60 >= T.tenureBeats
    }

    private func incomingScore(_ i: Int, now: Double, inputs: [StillpointDeckInputs]) -> Double {
        let t = decks[i]
        var s = 0.0
        let isTenured = tenured(i, now: now, inputs: inputs)
        // A tenured master throwing >3 disengages in 10 s is scratching
        // (performance), not cueing — those events score nothing.
        let recentHands = t.handEvents.filter { now - $0 < 10 }.count
        let performing = isTenured && recentHands > 3
        if !performing {
            let w = isTenured ? T.tenureDiscount : 1.0
            for e in t.handEvents { s += w * exp(-(now - e) / T.handEventDecaySecs) }
        }
        for e in t.backjumpEvents { s += 0.8 * exp(-(now - e) / T.backjumpDecaySecs) }
        if let p = t.playStartedAt, now - p < T.playStartWindowSecs {
            s += 0.5 * (1 - (now - p) / T.playStartWindowSecs)
        }
        s -= min(0.1 * cleanPlaySecs(i, now: now), 1.5)
        return max(0, s)
    }

    private func resolveRoles(now: Double, inputs: [StillpointDeckInputs]) {
        let sA = incomingScore(0, now: now, inputs: inputs)
        let sB = incomingScore(1, now: now, inputs: inputs)

        guard let inc = incomingIdx else {
            // Bootstrap rules — they need no timecode evidence, so
            // internal/file decks frame correctly too. The deck that
            // isn't playing is the one being prepped; when both play,
            // the later starter is the one coming in.
            var candidate: Int?
            let aPlay = inputs[0].isPlaying, bPlay = inputs[1].isPlaying
            if aPlay != bPlay {
                let idle = aPlay ? 1 : 0
                if inputs[idle].hasTrack { candidate = idle }
            } else if aPlay, bPlay,
                      let sa = decks[0].playStartedAt,
                      let sb = decks[1].playStartedAt,
                      abs(sa - sb) >= T.bootstrapStartSeparationSecs {
                candidate = sa < sb ? 1 : 0
            }
            if let c = candidate {
                if bootstrapCandidate != c {
                    bootstrapCandidate = c
                    bootstrapSince = now
                }
                if now - bootstrapSince >= T.bootstrapHoldSecs {
                    incomingIdx = c
                    bootstrapCandidate = nil
                    return
                }
            } else {
                bootstrapCandidate = nil
            }
            // General evidence path (hand events, re-cues).
            if abs(sA - sB) > T.roleMargin {
                if marginHeldSince == nil { marginHeldSince = now }
                if now - (marginHeldSince ?? now) >= T.roleMarginHoldSecs {
                    incomingIdx = sA > sB ? 0 : 1
                }
            } else {
                marginHeldSince = nil
            }
            return
        }

        let mas = 1 - inc
        if !inputs[mas].isPlaying {
            if masterStoppedSince == nil { masterStoppedSince = now }
        } else {
            masterStoppedSince = nil
        }
        let scores = [sA, sB]
        let anyHand = decks[0].handDown || decks[1].handDown
        if scores[mas] - scores[inc] > T.roleMargin && !anyHand {
            if swapHeldSince == nil { swapHeldSince = now }
        } else {
            swapHeldSince = nil
        }
        let swapByScore = swapHeldSince.map { now - $0 >= T.roleSwapHoldSecs } ?? false
        let swapByStop = masterStoppedSince.map { now - $0 >= T.masterStopSwapSecs } ?? false
        if swapByScore || swapByStop {
            incomingIdx = mas
            swapHeldSince = nil
            masterStoppedSince = nil
            resetMixState()
            return
        }

        // Decay to NEUTRAL — never mid-blend (PHASE/RIDE retain roles).
        if stage != .phase, stage != .ride,
           sA < T.neutralScoreFloor, sB < T.neutralScoreFloor,
           cleanPlaySecs(0, now: now) > T.neutralCleanPlaySecs,
           cleanPlaySecs(1, now: now) > T.neutralCleanPlaySecs {
            incomingIdx = nil
            resetMixState()
        }
    }

    private func resetMixState() {
        deposits.removeAll()
        corrections.removeAll()
        coach = nil
        driftEMA = nil
        lastDriftSample = nil
        revokeLock(at: nil)
    }

    // MARK: Octave fold

    private struct Fold {
        var k = 0
        var deltaBpm: Double?
    }

    private func updateFold(now: Double, inputs: [StillpointDeckInputs],
                            live: (DeckLive?, DeckLive?)) -> Fold {
        guard let inc = incomingIdx,
              let li = inc == 0 ? live.0 : live.1,
              let lm = inc == 0 ? live.1 : live.0 else {
            return Fold(k: foldK, deltaBpm: nil)
        }
        let ratio = li.liveBpm / lm.liveBpm
        let kRaw = Int(log2(ratio).rounded())
        let kClamped = max(-2, min(2, kRaw))
        let foldedRatio = ratio / pow(2, Double(kClamped))
        if kClamped != foldK, abs(foldedRatio - 1) < T.foldAdoptTolerance {
            if foldCandidate != kClamped {
                foldCandidate = kClamped
                foldCandidateSince = now
            } else if now - foldCandidateSince >= T.foldStableSecs {
                foldK = kClamped
                foldCandidate = nil
            }
        } else {
            foldCandidate = nil
        }
        // Fall back to no fold when the held ratio stops making sense.
        if abs(ratio / pow(2, Double(foldK)) - 1) > T.foldDropTolerance { foldK = 0 }

        let delta: Double
        if foldK >= 0 {
            delta = li.liveBpm - lm.liveBpm * pow(2, Double(foldK))
        } else {
            delta = li.liveBpm * pow(2, Double(-foldK)) - lm.liveBpm
        }
        return Fold(k: foldK, deltaBpm: delta)
    }

    // MARK: Phase

    private struct Phase {
        /// Filtered, motion-floored display value; nil = suspended.
        var live: Double?
    }

    private func rawPhaseMs(inputs: [StillpointDeckInputs],
                            live: (DeckLive?, DeckLive?)) -> Double? {
        guard let inc = incomingIdx,
              let li = inc == 0 ? live.0 : live.1,
              let lm = inc == 0 ? live.1 : live.0,
              !phaseSuspended(inputs[inc]), !phaseSuspended(inputs[1 - inc])
        else { return nil }
        // Fold into the faster deck's beat domain (doc §2, math).
        let f = pow(2, Double(abs(foldK)))
        var diff: Double
        if foldK >= 0 {
            var foldedMaster = lm.beatPhase * f
            foldedMaster -= foldedMaster.rounded(.down)
            diff = li.beatPhase - foldedMaster
            fastBeatMs = li.beatSecs * 1000
        } else {
            var foldedInc = li.beatPhase * f
            foldedInc -= foldedInc.rounded(.down)
            diff = foldedInc - lm.beatPhase
            fastBeatMs = lm.beatSecs * 1000
        }
        diff -= diff.rounded()
        return diff * fastBeatMs
    }

    private func updatePhase(now: Double, inputs: [StillpointDeckInputs],
                             live: (DeckLive?, DeckLive?), fold: Fold) -> Phase {
        guard let raw = rawPhaseMs(inputs: inputs, live: live) else {
            // Freeze at the last honest offset; never slide to the line.
            if displayPhaseMs != nil {
                frozenPhaseMs = displayPhaseMs
                frozenAt = now
            }
            displayPhaseMs = nil
            phaseRing.removeAll()
            phaseDispSign = 0
            slipRing.removeAll()
            slipMsPerSec = nil
            return Phase(live: nil)
        }
        frozenPhaseMs = nil
        // Sticky half-beat wrap: crossing zero flips the shown side
        // immediately, but crossing the ±half-beat rail pins the band
        // beyond the rail until the phase has come 0.15 beat past it —
        // no teleport, no mid-wrap median garbage, no fake "nudges".
        var show = raw
        let rawSign: Double = raw < 0 ? -1 : 1
        if phaseDispSign == 0 { phaseDispSign = rawSign }
        if rawSign != phaseDispSign {
            if abs(raw) <= 0.35 * fastBeatMs {
                phaseDispSign = rawSign
                // A hysteresis release far from zero is a legitimate
                // discontinuity — drop the mixed-side history.
                if abs(raw) > 0.15 * fastBeatMs {
                    phaseRing.removeAll()
                    coachRing.removeAll()
                    slipRing.removeAll()
                    slipMsPerSec = nil
                    displayPhaseMs = nil
                    lastDriftSample = nil
                }
            } else {
                show = raw + phaseDispSign * fastBeatMs
            }
        }
        phaseRing.append((now, show))
        phaseRing.removeAll { now - $0.t > T.medianWindowSecs }
        coachRing.append((now, show))
        coachRing.removeAll { now - $0.t > 1.2 }
        // Slip only exists while both grooves are moving — a paused
        // deck reads as "stopped", not "slow"; its tempo question is
        // answered by the pitch/grid prediction instead.
        if inputs[0].isPlaying, inputs[1].isPlaying {
            slipRing.append((now, show))
            slipRing.removeAll { now - $0.t > T.slipWindowSecs }
            slipMsPerSec = slipSlope()
        } else {
            slipRing.removeAll()
            slipMsPerSec = nil
        }
        let sorted = phaseRing.map(\.ms).sorted()
        let median = sorted[sorted.count / 2]
        if let cur = displayPhaseMs {
            if abs(median - cur) >= T.motionFloorMs { displayPhaseMs = median }
        } else {
            displayPhaseMs = median
        }
        return Phase(live: displayPhaseMs)
    }

    private func slipSlope() -> Double? {
        guard slipRing.count >= T.slipMinSamples,
              let first = slipRing.first, let last = slipRing.last,
              last.t - first.t >= T.slipMinSpanSecs else { return nil }
        let n = Double(slipRing.count)
        let mt = slipRing.reduce(0.0) { $0 + $1.t } / n
        let mv = slipRing.reduce(0.0) { $0 + $1.ms } / n
        var num = 0.0
        var den = 0.0
        for s in slipRing {
            num += (s.t - mt) * (s.ms - mv)
            den += (s.t - mt) * (s.t - mt)
        }
        guard den > 0 else { return nil }
        return num / den
    }

    /// Tempo error for everything the gutter shows: measured slip
    /// (groove truth, instant under the fader) when the phase readout
    /// is live, else the pitch/grid prediction (a paused deck has no
    /// slip to measure). Sign: + = incoming fast, like fold.deltaBpm.
    private func effectiveDeltaBpm(live: (DeckLive?, DeckLive?),
                                   fold: Fold) -> Double? {
        if let slip = slipMsPerSec, let inc = incomingIdx,
           let lm = inc == 0 ? live.1 : live.0 {
            return slip * lm.liveBpm / 1000
        }
        return fold.deltaBpm
    }

    // MARK: Stage FSM

    private func setStage(_ s: StillpointStage, now: Double) {
        guard s != stage else { return }
        prevStage = stage
        stage = s
        stageChangedAt = now
    }

    private func dwellOK(_ now: Double) -> Bool {
        now - stageChangedAt >= T.minDwellSecs
    }

    private func updateStage(now: Double, inputs: [StillpointDeckInputs],
                             live: (DeckLive?, DeckLive?), phaseMs: Double?) {
        let bothGrids = gridValid(inputs[0]) && gridValid(inputs[1])
        if !bothGrids {
            setStage(.hidden, now: now)   // exempt from dwell — retract honestly
            pendingDropAt = nil
            return
        }
        guard let inc = incomingIdx else {
            if stage != .neutral, dwellOK(now) || stage == .hidden {
                setStage(.neutral, now: now)
            }
            return
        }
        let mas = 1 - inc
        let incBeatSecs = (inc == 0 ? live.0 : live.1)?.beatSecs ?? 0.5
        let masBeatSecs = (inc == 0 ? live.1 : live.0)?.beatSecs ?? 0.5

        switch stage {
        case .hidden, .neutral:
            setStage(.tempo, now: now)

        case .tempo:
            let d = inputs[inc]
            // Internal decks have no needle: pressing play IS the drop.
            if !d.isTimecodeDriven, decks[inc].startedThisFrame,
               inputs[mas].isPlaying {
                dropFiredAt = now
                deposits.removeAll()
                setStage(.phase, now: now)
                return
            }
            // Timecode drop edge: hand release into play while the
            // master plays.
            let releasedNow = decks[inc].releasedThisFrame
                && d.isPlaying && inputs[mas].isPlaying
            if releasedNow && pendingDropAt == nil { pendingDropAt = now }
            if d.timecodeLockState == 3 { pendingDropAt = nil }
            if let p = pendingDropAt, now - p >= T.dropConfirmSecs {
                pendingDropAt = nil
                dropFiredAt = now
                deposits.removeAll()   // re-zero at release
                setStage(.phase, now: now)
                return
            }
            // No-timecode path (button-started / internal decks).
            if !inputs[inc].isTimecodeDriven || pendingDropAt == nil {
                let cleanBeats = cleanPlaySecs(inc, now: now) / max(incBeatSecs, 1e-6)
                if cleanBeats >= T.tempoCleanPlayBeats, inputs[mas].isPlaying,
                   !decks[inc].handDown, dwellOK(now) {
                    setStage(.phase, now: now)
                }
            }

        case .phase, .ride:
            // Re-cue: a long hold WITH a backjump. A nudge touch
            // (<600 ms, no jump) can never flip the face. A 3→1 edge
            // here is a nudge release — the drop never re-fires.
            if let since = decks[inc].handDownSince,
               now - since > T.recueHoldSecs, decks[inc].backjumpDuringHold {
                setStage(.tempo, now: now)
                revokeLock(at: now)
                return
            }
            // Internal re-cue: a seek back > 1 beat (there is no hold
            // concept without a needle).
            if !inputs[inc].isTimecodeDriven, decks[inc].backjumpThisFrame {
                setStage(.tempo, now: now)
                revokeLock(at: now)
                return
            }
            guard let p = phaseMs else { break }
            if stage == .phase {
                if abs(p) < T.rideInMs {
                    if phaseInRideSince == nil { phaseInRideSince = now }
                    if now - (phaseInRideSince ?? now) >= T.rideInBeats * masBeatSecs,
                       dwellOK(now) {
                        setStage(.ride, now: now)
                    }
                } else {
                    phaseInRideSince = nil
                }
            } else {
                if abs(p) > T.rideOutMs {
                    if phaseOutRideSince == nil { phaseOutRideSince = now }
                    if now - (phaseOutRideSince ?? now) >= T.rideOutBeats * masBeatSecs,
                       dwellOK(now) {
                        setStage(.phase, now: now)
                    }
                } else {
                    phaseOutRideSince = nil
                }
            }
        }
    }

    // MARK: Belt

    private func updateBelt(now: Double, dt: Double,
                            inputs: [StillpointDeckInputs], deltaBpm: Double?) {
        guard stage == .tempo, let inc = incomingIdx,
              let delta = deltaBpm, !decks[inc].handDown,
              inputs[inc].pitchPercent != nil || !inputs[inc].isTimecodeDriven else {
            beltVelocity = 0
            return
        }
        let mag = abs(delta)
        if mag < T.beltDeadbandBpm {
            beltVelocity = 0
            return
        }
        let sign: Double = delta > 0 ? 1 : -1
        // Sticky sign: a flip needs to clear the wider threshold so a
        // near-match never flickers direction.
        if beltSign != 0, sign != beltSign, mag <= T.beltSignFlipBpm {
            beltVelocity = 0
            return
        }
        beltSign = sign
        beltVelocity = sign * min(T.beltBaseVelocity + T.beltVelocityPerBpm * mag,
                                  T.beltMaxVelocity)
        beltOffset += beltVelocity * dt
        beltOffset -= (beltOffset / T.beltPitchPx).rounded(.down) * T.beltPitchPx
    }

    // MARK: Master-beat clock + deposits

    private func trackMasterBeat(live: (DeckLive?, DeckLive?)) {
        masterBeatCrossed = false
        guard let inc = incomingIdx, let lm = inc == 0 ? live.1 : live.0 else {
            prevMasterBeatPhase = nil
            return
        }
        if let prev = prevMasterBeatPhase, lm.beatPhase < prev {
            masterBeatCrossed = true
        }
        prevMasterBeatPhase = lm.beatPhase
    }

    private func updateDeposits(now: Double, inputs: [StillpointDeckInputs],
                                live: (DeckLive?, DeckLive?), phaseMs: Double?) {
        deposits.removeAll { now - $0.t > T.depositFadeSecs }
        guard stage == .phase || stage == .ride,
              let inc = incomingIdx, masterBeatCrossed else { return }
        let mas = 1 - inc
        // Master beat instant. Stamp iff every gate passes right now —
        // a gap in the chain is the instrument abstaining.
        let masterTouched = inputs[mas].isTimecodeDriven
            && inputs[mas].timecodeLockState >= 2
        guard let p = phaseMs, !masterTouched, !decks[inc].handDown else { return }
        deposits.append((now, p))
        if deposits.count > T.depositCap { deposits.removeFirst() }
    }

    // MARK: Certification

    private func revokeLock(at now: Double?) {
        if lock != .none, let now { lockBrokenAt = now }
        lock = .none
        holdBeats = 0
        gatesPassSince = nil
        phaseInGreenSince = nil
    }

    private func updateCertification(now: Double, inputs: [StillpointDeckInputs],
                                     live: (DeckLive?, DeckLive?),
                                     phaseMs: Double?, deltaBpm: Double?) {
        guard stage == .phase || stage == .ride,
              let inc = incomingIdx,
              let lm = inc == 0 ? live.1 : live.0,
              let p = phaseMs, let delta = deltaBpm else {
            revokeLock(at: now)
            return
        }
        func deckGates(_ d: StillpointDeckInputs) -> Bool {
            guard d.bpmConfidence >= T.greenMinConfidence else { return false }
            if d.isTimecodeDriven {
                guard d.timecodeLockState == 1, d.pitchSettled else { return false }
            }
            return true
        }
        let gates = deckGates(inputs[0]) && deckGates(inputs[1])
            && abs(delta) < T.greenDeltaBpm
        guard abs(p) < T.greenPhaseMs, gates else {
            // Any gate failing cools the lock the same frame.
            revokeLock(at: now)
            return
        }
        if phaseInGreenSince == nil { phaseInGreenSince = now }
        let sustained = now - (phaseInGreenSince ?? now)
            >= T.greenPhaseBeats * lm.beatSecs
        guard sustained else { return }
        if gatesPassSince == nil { gatesPassSince = now }
        guard now - (gatesPassSince ?? now) >= T.greenAttackSecs else { return }
        // Pre-ODF grid-trust clause: seated but untrusted grids render
        // white, never green. White-before-green.
        let trusted = inputs[0].bpmConfidence >= T.gridTrustConfidence
            && inputs[1].bpmConfidence >= T.gridTrustConfidence
        lock = trusted ? .green : .white
        if masterBeatCrossed { holdBeats += 1 }
    }

    // MARK: Coach

    private func updateCoach(now: Double, inputs: [StillpointDeckInputs],
                             live: (DeckLive?, DeckLive?), phaseMs: Double?) {
        guard stage == .phase || stage == .ride, let inc = incomingIdx,
              let lm = inc == 0 ? live.1 : live.0 else {
            handBlipPhaseMs = nil
            handBlipStartedAt = nil
            return
        }
        let beat = lm.beatSecs

        // Nudge detection requires a needle: only a timecode deck has
        // a platter to push or drag. Internal seeks are jumps, not
        // corrections — they must never feed the coach.
        let hasNeedle = inputs[inc].isTimecodeDriven

        // Hand-blip nudges: remember φ at hand-down, compare once the
        // phase readout resumes after release. Holds longer than the
        // blip window are cuts/scratches — never tallied.
        if decks[inc].handDown {
            if handBlipPhaseMs == nil {
                handBlipPhaseMs = frozenPhaseMs ?? phaseMs
                handBlipStartedAt = now
            }
        } else if let before = handBlipPhaseMs, let started = handBlipStartedAt {
            if now - started > T.handBlipMaxSecs + 1.0 {
                handBlipPhaseMs = nil
                handBlipStartedAt = nil
            } else if let after = phaseMs {
                let blipLen = now - started
                handBlipPhaseMs = nil
                handBlipStartedAt = nil
                if blipLen <= T.handBlipMaxSecs,
                   abs(before) - abs(after) >= T.nudgeStepMs,
                   now - lastNudgeAt >= T.nudgeDebounceBeats * beat {
                    recordCorrection(sign: after - before > 0 ? 1 : -1, now: now)
                }
            }
        }

        guard let p = phaseMs else { return }

        // Step nudges: a fast jump toward zero that cuts the error.
        if hasNeedle,
           let past = coachRing.first(where: { now - $0.t >= T.nudgeLookbackSecs })
            ?? coachRing.first, now - past.t >= T.nudgeLookbackSecs * 0.8 {
            let before = past.ms
            if abs(before) >= T.nudgeStepMs,
               abs(p) <= abs(before) * (1 - T.nudgeCutFraction),
               abs(p - before) >= T.nudgeStepMs,
               now - lastNudgeAt >= T.nudgeDebounceBeats * beat {
                recordCorrection(sign: p - before > 0 ? 1 : -1, now: now)
            }
        }

        // Residual tempo: dφ/dt strictly between corrections — nudges
        // are phase steps; inter-nudge drift is uncontaminated tempo.
        // Sampled only well inside the rails (a pinned band carries no
        // usable drift information).
        guard abs(p) < fastBeatMs * 0.3 else {
            lastDriftSample = nil
            return
        }
        if now - lastNudgeAt > T.nudgeLookbackSecs, !decks[inc].handDown {
            if let prev = lastDriftSample, now > prev.t {
                let rate = (p - prev.ms) / (now - prev.t)   // ms/s
                if abs(rate) < 50 {                          // discard step glitches
                    let tau = T.driftEmaBeats * beat
                    let alpha = min(1, (now - prev.t) / tau)
                    driftEMA = (driftEMA ?? rate) * (1 - alpha) + rate * alpha
                }
            }
            lastDriftSample = (now, p)
        } else {
            lastDriftSample = nil
        }

        corrections.removeAll { now - $0.t > T.coachWindowBeats * beat }

        // Arm: the last N corrections all the same way AND the measured
        // inter-nudge drift agrees AND the drift is real — the trim
        // floor must never quote noise (a 0.05 call needs ≥0.4 ms/s).
        if coach == nil, corrections.count >= T.coachMinCorrections,
           let drift = driftEMA, abs(drift) >= T.coachMinDriftMsPerSec {
            let lastSigns = corrections.suffix(T.coachMinCorrections).map(\.sign)
            if let s = lastSigns.first, lastSigns.allSatisfy({ $0 == s }),
               s == (drift < 0 ? 1 : -1) {
                let trim = min(max((abs(drift) / 10 / T.coachTrimStep).rounded()
                                   * T.coachTrimStep, T.coachTrimMin), T.coachTrimMax)
                if trim >= T.coachTrimMin {
                    coach = (pitchUp: s > 0, trim: trim, armedAt: now,
                             pitchAtArm: inputs[inc].pitchSettled
                                 ? inputs[inc].pitchPercent : nil)
                }
            }
        }

        // Clear: the fader moved at least half the suggestion in the
        // called direction (silent fade), or the drift sign flipped.
        if let c = coach {
            if let drift = driftEMA, (drift < 0 ? 1.0 : -1.0) != (c.pitchUp ? 1.0 : -1.0),
               abs(drift) > 0.3 {
                coach = nil
                corrections.removeAll()
            } else if let base = c.pitchAtArm, let cur = inputs[inc].pitchPercent,
                      inputs[inc].pitchSettled {
                let moved = cur - base
                if (c.pitchUp && moved >= c.trim / 2) || (!c.pitchUp && moved <= -c.trim / 2) {
                    coach = nil
                    corrections.removeAll()
                }
            }
        }
    }

    private func recordCorrection(sign: Double, now: Double) {
        corrections.append((now, sign))
        lastNudgeAt = now
        lastDriftSample = nil
    }

    // MARK: Frame assembly

    private func makeFrame(now: Double, inputs: [StillpointDeckInputs],
                           live: (DeckLive?, DeckLive?), fold: Fold,
                           phase: Phase, deltaEff: Double?) -> StillpointFrame {
        var f = StillpointFrame()
        f.stage = stage
        f.prevStage = prevStage
        f.stageProgress = min(1, max(0, (now - stageChangedAt) / T.crossfadeSecs))
        f.incomingIsA = incomingIdx.map { $0 == 0 }
        f.withdrawn = !(gridValid(inputs[0]) && gridValid(inputs[1]))
        f.phaseMs = phase.live
        f.frozenPhaseMs = phase.live == nil ? frozenPhaseMs : nil
        f.frozenAgeSecs = phase.live == nil ? frozenAt.map { now - $0 } : nil
        f.deltaBpm = deltaEff
        f.foldK = fold.k
        f.beltOffsetPx = beltOffset
        f.beltVelocityPx = beltVelocity
        if let inc = incomingIdx {
            f.beltFrozen = decks[inc].handDown
                || (inputs[inc].isTimecodeDriven && inputs[inc].pitchPercent == nil)
        }
        f.lock = lock
        f.holdBeats = holdBeats
        f.lockBrokenAgo = lockBrokenAt.flatMap { now - $0 <= 0.45 ? now - $0 : nil }
        f.dropFiredAgo = dropFiredAt.flatMap { now - $0 <= 0.5 ? now - $0 : nil }
        f.deposits = deposits.map { StillpointDeposit(ageSecs: now - $0.t, phaseMs: $0.ms) }
        f.coach = coach.map {
            StillpointCoach(pitchUp: $0.pitchUp, trimPercent: $0.trim,
                            armedAgoSecs: now - $0.armedAt)
        }
        var alpha = min(inputs[0].bpmConfidence, inputs[1].bpmConfidence)
        for d in inputs where d.isTimecodeDriven && d.timecodeLockState == 2 {
            alpha = min(alpha, T.degradedCarrierAlpha)
        }
        f.alpha = max(T.confidenceFloor, min(1, alpha))
        f.showFinePrint = stage != .ride || lock == .none
        f.halfBeatMs = fastBeatMs / 2
        return f
    }
}
