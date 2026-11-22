import XCTest
@testable import Dub

final class PlayedTallyTests: XCTestCase {
    func testATrackCountsOnceAfterThirtySecondsOfPlaying() {
        var tally = PlayedTally()
        var reported: [String] = []
        for _ in 0..<(40 * 30) {
            if let id = tally.tick(trackId: "t1", playing: true, dt: 1.0 / 30) { reported.append(id) }
        }
        XCTAssertEqual(reported, ["t1"])
    }

    func testCueingAndPausedTimeDoNotCount() {
        var tally = PlayedTally()
        for _ in 0..<(20 * 30) {
            XCTAssertNil(tally.tick(trackId: "t1", playing: true, dt: 1.0 / 30))
        }
        for _ in 0..<(600 * 30) {
            XCTAssertNil(tally.tick(trackId: "t1", playing: false, dt: 1.0 / 30))
        }
    }

    func testANewTrackStartsFromNothing() {
        var tally = PlayedTally()
        for _ in 0..<(29 * 30) { _ = tally.tick(trackId: "t1", playing: true, dt: 1.0 / 30) }
        for _ in 0..<(29 * 30) {
            XCTAssertNil(tally.tick(trackId: "t2", playing: true, dt: 1.0 / 30))
        }
        XCTAssertNil(tally.tick(trackId: nil, playing: true, dt: 1.0 / 30))
    }

    func testAStalledPollIsNotPlayingTime() {
        var tally = PlayedTally()
        XCTAssertNil(tally.tick(trackId: "t1", playing: true, dt: 120))
    }
}
