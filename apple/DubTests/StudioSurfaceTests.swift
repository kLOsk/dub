import XCTest

@testable import Dub

/// PREP · REC · PERF (rig, 2026-09-25): which surface the app is on,
/// which moves are allowed, and what leaving does to a recording —
/// never throwing away a take the DJ has not discarded.
final class StudioSurfaceTests: XCTestCase {

    func testTheSurfaceFollowsTheRecordingThenTheEngine() {
        XCTAssertEqual(StudioSurfaceRules.current(ripPhase: .review, engineMode: .prep), .record)
        XCTAssertEqual(StudioSurfaceRules.current(ripPhase: .none, engineMode: .prep), .prep)
        XCTAssertEqual(StudioSurfaceRules.current(ripPhase: .none, engineMode: .timecode), .perf)
    }

    /// Mid-take the only way out is STOP; while importing, waiting.
    func testPrepIsBlockedWhileRecordingOrImporting() {
        XCTAssertNotNil(StudioSurfaceRules.disabled(ripPhase: .capture, armed: false, canRecord: true)[.prep])
        XCTAssertNil(StudioSurfaceRules.disabled(ripPhase: .capture, armed: true, canRecord: true)[.prep])
        XCTAssertNotNil(StudioSurfaceRules.disabled(ripPhase: .encoding, armed: false, canRecord: true)[.prep])
        XCTAssertNil(StudioSurfaceRules.disabled(ripPhase: .review, armed: false, canRecord: true)[.prep])
    }

    func testPerfWaitsForTheRecordingAndRecNeedsAnInput() {
        XCTAssertNotNil(StudioSurfaceRules.disabled(ripPhase: .review, armed: false, canRecord: true)[.perf])
        XCTAssertNil(StudioSurfaceRules.disabled(ripPhase: .none, armed: false, canRecord: true)[.perf])
        XCTAssertNotNil(StudioSurfaceRules.disabled(ripPhase: .none, armed: false, canRecord: false)[.record])
    }

    /// A take is parked, not discarded; an armed take with nothing
    /// captured is cancelled; an imported one is closed.
    func testLeavingKeepsAnyTake() {
        XCTAssertEqual(StudioSurfaceRules.leave(ripPhase: .review, armed: false, hasSegments: true), .park)
        XCTAssertEqual(StudioSurfaceRules.leave(ripPhase: .failed, armed: false, hasSegments: true), .park)
        XCTAssertEqual(StudioSurfaceRules.leave(ripPhase: .failed, armed: false, hasSegments: false), .dismiss)
        XCTAssertEqual(StudioSurfaceRules.leave(ripPhase: .capture, armed: true, hasSegments: false), .cancel)
        XCTAssertEqual(StudioSurfaceRules.leave(ripPhase: .capture, armed: false, hasSegments: false), .refuse)
        XCTAssertEqual(StudioSurfaceRules.leave(ripPhase: .encoding, armed: false, hasSegments: true), .refuse)
        XCTAssertEqual(StudioSurfaceRules.leave(ripPhase: .done, armed: false, hasSegments: true), .dismiss)
    }
}
