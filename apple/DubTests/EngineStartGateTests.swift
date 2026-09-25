import XCTest

@testable import Dub

/// Connecting the SL3 froze the app for 10–13 s (rig, 2026-09-23): the
/// device probe and the engine start ran on the main thread while the
/// interface's driver came up. Both run on a background queue now, and
/// the start commits back on the main thread — unless something newer
/// has happened meanwhile. These pin that rule.
final class EngineStartGateTests: XCTestCase {

    /// The plain case: a start that nothing overtook commits.
    func testAnUncontestedStartCommits() {
        var gate = EngineStartGate()
        let t = gate.begin(device: "SL3")
        XCTAssertEqual(gate.connecting, "SL3")
        XCTAssertTrue(gate.finish(t))
        XCTAssertNil(gate.connecting)
    }

    /// A stop while the start is in flight — the DJ switched mode, or
    /// pulled the cable. The stop owns the engine; the start must not
    /// mark it running when it lands.
    func testAStopOvertakesAStart() {
        var gate = EngineStartGate()
        let t = gate.begin(device: "SL3")
        gate.cancel()
        XCTAssertNil(gate.connecting)
        XCTAssertFalse(gate.finish(t))
    }

    /// Two starts: only the newer commits, and it is what shows as
    /// connecting until it does.
    func testOnlyTheNewestStartCommits() {
        var gate = EngineStartGate()
        let first = gate.begin(device: "SL3")
        let second = gate.begin(device: "SL3 #2")
        XCTAssertFalse(gate.finish(first))
        XCTAssertEqual(gate.connecting, "SL3 #2")
        XCTAssertTrue(gate.finish(second))
        XCTAssertNil(gate.connecting)
    }

    /// Nothing in flight: a stop has nothing to overtake.
    func testInFlightTracksTheStart() {
        var gate = EngineStartGate()
        XCTAssertFalse(gate.inFlight)
        let t = gate.begin(device: "SL3")
        XCTAssertTrue(gate.inFlight)
        _ = gate.finish(t)
        XCTAssertFalse(gate.inFlight)
    }
}
