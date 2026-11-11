import XCTest

@testable import Dub

/// SYNC on the Space Echo (2026-10-06): the repeat follows the music's
/// tempo, so a held echo carries the outgoing tune in time.
final class FxEchoSyncTests: XCTestCase {

    /// ¾ of a beat at 90 BPM: 500 ms — heads at 168 · 334 · 500, a
    /// sixteenth, an eighth and a dotted eighth.
    func testThreeQuartersOfABeat() throws {
        let ms = try XCTUnwrap(FxRackControls.syncedRepeatMs(bpm: 90, division: 0.75))
        XCTAssertEqual(ms, 500, accuracy: 0.001)
    }

    /// Longer than the tape: halved until it fits, so it stays on the grid.
    func testLongerThanTheTapeIsHalved() throws {
        // One beat at 70 BPM is 857 ms; the tape holds 750.
        let ms = try XCTUnwrap(FxRackControls.syncedRepeatMs(bpm: 70, division: 1))
        XCTAssertEqual(ms, 857.142 / 2, accuracy: 0.01)
        // ¾ at 55 BPM is 818 ms → ⅜ of a beat.
        let slow = try XCTUnwrap(FxRackControls.syncedRepeatMs(bpm: 55, division: 0.75))
        XCTAssertEqual(slow, 60_000 / 55 * 0.375, accuracy: 0.01)
    }

    /// The fader moves the tempo, and the repeat with it.
    func testTheRepeatFollowsThePitch() throws {
        let flat = try XCTUnwrap(FxRackControls.syncedRepeatMs(bpm: 80, division: 0.75))
        let up = try XCTUnwrap(FxRackControls.syncedRepeatMs(bpm: 80 * 1.08, division: 0.75))
        XCTAssertEqual(flat / up, 1.08, accuracy: 1e-9)
    }

    func testNoTempoNoRepeat() {
        XCTAssertNil(FxRackControls.syncedRepeatMs(bpm: 0, division: 0.75))
        XCTAssertNil(FxRackControls.syncedRepeatMs(bpm: .nan, division: 0.75))
    }

    /// Taps a beat apart at 75 BPM read 75.
    func testTapsReadTheirTempo() throws {
        let taps = (0..<4).map { Double($0) * 0.8 }
        XCTAssertEqual(try XCTUnwrap(FxRackControls.tappedBpm(taps)), 75, accuracy: 0.001)
    }

    func testOneTapIsNotATempo() {
        XCTAssertNil(FxRackControls.tappedBpm([10]))
        XCTAssertNil(FxRackControls.tappedBpm([]))
    }

    /// A pause over two seconds starts a new count: the old taps do not
    /// drag the new tempo.
    func testAPauseStartsANewCount() throws {
        let taps = [0, 0.5, 1.0, 5.0, 5.8, 6.6]
        XCTAssertEqual(try XCTUnwrap(FxRackControls.tappedBpm(taps)), 75, accuracy: 0.001)
    }

    func testTheSourceReadsAsPrinted() {
        XCTAssertEqual(FxEchoTempo(source: .deck(.a), bpm: 92.44).text, "A · 92.4")
        XCTAssertEqual(FxEchoTempo(source: .tap, bpm: 78).text, "TAP · 78.0")
    }
}
