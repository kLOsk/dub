//
//  StillpointSnapshotTests.swift
//  DubTests
//
//  Canonical Stillpoint frames rendered to PNGs — the four faces a
//  DJ actually meets (tempo belt, post-drop band + ladder, certified
//  ride, stale-frozen hold) plus the coach. First run records
//  references; committed PNGs make subsequent runs pass.
//

import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

final class StillpointSnapshotTests: XCTestCase {

    private func snap(
        _ frame: StillpointFrame,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        let view = StillpointCanvas(frame: frame)
            .frame(width: 120, height: 500)
            .background(DubColor.surface0)
        let host = NSHostingView(rootView: view)
        host.frame = CGRect(x: 0, y: 0, width: 120, height: 500)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(of: host, as: .image, named: name,
                       file: file, testName: testName, line: line)
    }

    func test_tempoFace_beltCreeping() {
        var f = StillpointFrame()
        f.stage = .tempo
        f.prevStage = .tempo
        f.incomingIsA = false
        f.deltaBpm = -0.8
        f.beltOffsetPx = 17
        f.beltVelocityPx = -47
        snap(f, named: "tempo-belt")
    }

    func test_phaseFace_bandLow_ladderLeaking() {
        var f = StillpointFrame()
        f.stage = .phase
        f.prevStage = .phase
        f.incomingIsA = false
        f.phaseMs = -22
        f.deltaBpm = -0.12
        snap(f, named: "phase-band-ladder")
    }

    func test_rideFace_certifiedGreen() {
        var f = StillpointFrame()
        f.stage = .ride
        f.prevStage = .ride
        f.incomingIsA = false
        f.phaseMs = 1
        f.deltaBpm = 0
        f.lock = .green
        f.holdBeats = 24
        f.showFinePrint = false
        snap(f, named: "ride-green")
    }

    func test_phaseFace_staleFrozenGhost() {
        var f = StillpointFrame()
        f.stage = .phase
        f.prevStage = .phase
        f.incomingIsA = false
        f.phaseMs = nil
        f.frozenPhaseMs = -18
        f.deltaBpm = nil
        snap(f, named: "phase-stale-ghost")
    }

    func test_rideFace_coachArmed() {
        var f = StillpointFrame()
        f.stage = .ride
        f.prevStage = .ride
        f.incomingIsA = true
        f.phaseMs = -6
        f.deltaBpm = -0.1
        f.lock = .none
        f.coach = StillpointCoach(pitchUp: true, trimPercent: 0.2, armedAgoSecs: 1)
        snap(f, named: "ride-coach")
    }
}
