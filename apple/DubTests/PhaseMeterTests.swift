//
//  PhaseMeterTests.swift
//  DubTests
//
//  The phase meter's maths (round 4): the incoming deck's beat against
//  the master's, wrapped to half a beat either side, in beats and in
//  the room's milliseconds; and the canvas in its states.
//

import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

final class PhaseMeterTests: XCTestCase {

    /// A deck on a 120 BPM grid anchored at 0 — one beat every 500 ms.
    private func deck(at secs: Double, bpm: Double = 120, pitch: Double? = nil, playing: Bool = true) -> PhaseMeterInputs {
        var d = PhaseMeterInputs()
        d.hasTrack = true
        d.isPlaying = playing
        d.bpm = bpm
        d.gridAnchorSecs = 0
        d.pitchPercent = pitch
        d.playheadSecs = secs
        return d
    }

    func testInPhaseReadsZeroAndLocks() {
        let f = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 4.5), masterIsA: true)
        XCTAssertEqual(f.phaseBeats ?? 1, 0, accuracy: 1e-9)
        XCTAssertEqual(f.phaseMs ?? 1, 0, accuracy: 1e-6)
        XCTAssertTrue(f.locked)
        XCTAssertFalse(f.incomingIsA, "A is the master, so the marker is B's")
    }

    /// B a tenth of a beat behind A: late, negative, 50 ms at 120 BPM.
    func testLateIsNegativeInBeatsAndMilliseconds() {
        let f = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 9.95), masterIsA: true)
        XCTAssertEqual(f.phaseBeats ?? 0, -0.1, accuracy: 1e-9)
        XCTAssertEqual(f.phaseMs ?? 0, -50, accuracy: 1e-6)
        XCTAssertFalse(f.locked)
    }

    /// Past half a beat the nearer beat is the other one: 0.7 ahead reads
    /// as 0.3 behind, and exactly half a beat is +0.5, never −0.5.
    func testWrapsToTheNearerBeat() {
        let ahead = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 10.35), masterIsA: true)
        XCTAssertEqual(ahead.phaseBeats ?? 0, -0.3, accuracy: 1e-9)
        let half = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 10.25), masterIsA: true)
        XCTAssertEqual(half.phaseBeats ?? 0, 0.5, accuracy: 1e-9)
        let behindHalf = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 9.75), masterIsA: true)
        XCTAssertEqual(behindHalf.phaseBeats ?? 0, 0.5, accuracy: 1e-9)
    }

    /// The beat phase comes from the grid in track time; the platter's
    /// pitch changes only the milliseconds a beat is worth in the room.
    func testPitchScalesMillisecondsNotBeats() {
        let flat = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 9.9), masterIsA: true)
        let fast = PhaseMeter.frame(a: deck(at: 10.0, pitch: 10), b: deck(at: 9.9), masterIsA: true)
        XCTAssertEqual(flat.phaseBeats ?? 0, fast.phaseBeats ?? 1, accuracy: 1e-9)
        XCTAssertEqual(flat.phaseMs ?? 0, -100, accuracy: 1e-6)
        XCTAssertEqual(fast.phaseMs ?? 0, -100 / 1.1, accuracy: 1e-6)
    }

    /// The master is the reference: swap it and the sign flips, and the
    /// marker changes hands.
    func testMasterIsTheReference() {
        let aMaster = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 9.95), masterIsA: true)
        let bMaster = PhaseMeter.frame(a: deck(at: 10.0), b: deck(at: 9.95), masterIsA: false)
        XCTAssertEqual(aMaster.phaseBeats ?? 0, -(bMaster.phaseBeats ?? 0), accuracy: 1e-9)
        XCTAssertFalse(aMaster.incomingIsA)
        XCTAssertTrue(bMaster.incomingIsA)
    }

    /// No grid on either side, no phase — and a paused deck cannot lock,
    /// however close it sits.
    func testNoGridNoPhaseAndPausedNeverLocks() {
        var ungridded = deck(at: 3)
        ungridded.gridAnchorSecs = nil
        XCTAssertNil(PhaseMeter.frame(a: ungridded, b: deck(at: 3), masterIsA: true).phaseBeats)
        var empty = PhaseMeterInputs()
        empty.hasTrack = false
        XCTAssertNil(PhaseMeter.frame(a: deck(at: 3), b: empty, masterIsA: true).phaseBeats)
        let paused = PhaseMeter.frame(a: deck(at: 10), b: deck(at: 10, playing: false), masterIsA: true)
        XCTAssertEqual(paused.phaseBeats ?? 1, 0, accuracy: 1e-9)
        XCTAssertFalse(paused.locked)
    }

    // MARK: - Canvas

    private func snap(_ frame: PhaseMeterFrame, named name: String,
                      file: StaticString = #filePath, testName: String = #function, line: UInt = #line) {
        let view = PhaseMeterCanvas(frame: frame)
            .frame(width: DubLayout.phaseMeterGutterWidth, height: 560)
            .background(DubColor.surface0)
        let host = NSHostingView(rootView: view)
        host.frame = CGRect(x: 0, y: 0, width: DubLayout.phaseMeterGutterWidth, height: 560)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(of: host, as: .image(perceptualPrecision: 0.98), named: name,
                       file: file, testName: testName, line: line)
    }

    /// No grid: the track and the line, nothing on it.
    func test_canvas_empty() {
        snap(PhaseMeterFrame(), named: "empty")
    }

    /// B late by a fifth of a beat: the marker above the line in B's tint.
    func test_canvas_lateB() {
        snap(PhaseMeterFrame(phaseBeats: -0.2, phaseMs: -100, incomingIsA: false), named: "late-b")
    }

    /// Locked: on the line, green.
    func test_canvas_locked() {
        snap(PhaseMeterFrame(phaseBeats: 0.01, phaseMs: 5, incomingIsA: false, locked: true), named: "locked")
    }

    /// A early by nearly half a beat: the marker at the bottom end of the
    /// track in A's tint, one frame from wrapping to the top.
    func test_canvas_nearWrapA() {
        snap(PhaseMeterFrame(phaseBeats: 0.45, phaseMs: 225, incomingIsA: true), named: "near-wrap-a")
    }
}
