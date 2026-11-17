//
//  MatchingCrateTests.swift
//  DubTests
//
//  The Matching smart crate (Daniel, 2026-11-17): keys on the Camelot
//  wheel, however the library spelled them, and the window round the
//  playing track that Preferences ▸ Library sets.
//

@testable import Dub
import XCTest

final class MatchingCrateTests: XCTestCase {

    private func key(_ s: String) -> String? { CamelotKey(s)?.notation }

    /// Camelot, musical notation and Open Key all land on the one wheel.
    func testEverySpellingLandsOnTheWheel() {
        XCTAssertEqual(key("8A"), "8A")
        XCTAssertEqual(key("12b"), "12B")
        XCTAssertEqual(key("Am"), "8A")
        XCTAssertEqual(key("A minor"), "8A")
        XCTAssertEqual(key("C"), "8B")
        XCTAssertEqual(key("Cmaj"), "8B")
        XCTAssertEqual(key("F#m"), "11A")
        XCTAssertEqual(key("Bbm"), "3A")
        XCTAssertEqual(key("Db"), "3B")
        XCTAssertEqual(key("Em"), "9A")
        XCTAssertEqual(key("B"), "1B")
        XCTAssertEqual(key("1d"), "8B", "Open Key 1d is C major")
        XCTAssertEqual(key("1m"), "8A")
        XCTAssertEqual(key("6d"), "1B")
        XCTAssertNil(CamelotKey("13A"))
        XCTAssertNil(CamelotKey("H"))
        XCTAssertNil(CamelotKey(""))
        XCTAssertNil(CamelotKey(nil))
    }

    /// A step either way round the wheel, or across to the relative major
    /// / minor, is one; the wheel wraps at 12.
    func testDistanceRoundTheWheel() {
        let a8 = CamelotKey(number: 8, minor: true)
        XCTAssertEqual(a8.distance(to: a8), 0)
        XCTAssertEqual(a8.distance(to: CamelotKey(number: 9, minor: true)), 1)
        XCTAssertEqual(a8.distance(to: CamelotKey(number: 7, minor: true)), 1)
        XCTAssertEqual(a8.distance(to: CamelotKey(number: 8, minor: false)), 1)
        XCTAssertEqual(a8.distance(to: CamelotKey(number: 10, minor: true)), 2)
        XCTAssertEqual(a8.distance(to: CamelotKey(number: 9, minor: false)), 2)
        XCTAssertEqual(
            CamelotKey(number: 12, minor: true).distance(to: CamelotKey(number: 1, minor: true)), 1)
        XCTAssertEqual(a8.neighbours(within: 1).map(\.notation), ["8A", "7A", "8B", "9A"])
        XCTAssertEqual(a8.neighbours(within: 0).map(\.notation), ["8A"])
    }

    private let ref = MatchReference(
        trackId: "playing", title: "T", artist: nil, bpm: 124,
        key: CamelotKey(number: 8, minor: true), onDecks: ["playing", "other-deck"])

    /// The default window: one key step, ±3 BPM. What is on the decks
    /// never shows; nearer keys rank first.
    func testTheDefaultWindow() {
        XCTAssertNotNil(TrackMatching.rank(bpm: 124, key: "8A", id: "x", against: ref))
        XCTAssertNotNil(TrackMatching.rank(bpm: 127, key: "9A", id: "x", against: ref))
        XCTAssertNotNil(TrackMatching.rank(bpm: 121, key: "Am", id: "x", against: ref))
        XCTAssertNil(TrackMatching.rank(bpm: 127.5, key: "8A", id: "x", against: ref), "past ±3")
        XCTAssertNil(TrackMatching.rank(bpm: 124, key: "10A", id: "x", against: ref), "two steps")
        XCTAssertNil(TrackMatching.rank(bpm: 124, key: "8A", id: "other-deck", against: ref))
        XCTAssertNil(TrackMatching.rank(bpm: nil, key: "8A", id: "x", against: ref))
        XCTAssertNil(TrackMatching.rank(bpm: 124, key: nil, id: "x", against: ref))
        let same = TrackMatching.rank(bpm: 126, key: "8A", id: "x", against: ref)
        let near = TrackMatching.rank(bpm: 124, key: "9A", id: "y", against: ref)
        XCTAssertEqual(same?.keyDistance, 0)
        XCTAssertEqual(near?.keyDistance, 1)
    }

    /// Preferences ▸ Library widens or narrows both.
    func testTheWindowIsTheDJs() {
        let tight = MatchWindow(bpm: 1, keySteps: 0)
        XCTAssertNil(TrackMatching.rank(bpm: 124, key: "9A", id: "x", against: ref, window: tight))
        XCTAssertNil(TrackMatching.rank(bpm: 125.5, key: "8A", id: "x", against: ref, window: tight))
        XCTAssertNotNil(TrackMatching.rank(bpm: 125, key: "8A", id: "x", against: ref, window: tight))
        let wide = MatchWindow(bpm: 6, keySteps: 2)
        XCTAssertNotNil(TrackMatching.rank(bpm: 130, key: "10A", id: "x", against: ref, window: wide))
        XCTAssertEqual(LibraryView.bpmFormat(3), "3")
        XCTAssertEqual(LibraryView.bpmFormat(2.5), "2.5")
    }

    /// Half and double time are off until the DJ turns them on.
    func testHalfAndDoubleTime() {
        let dub = MatchReference(
            trackId: "p", title: nil, artist: nil, bpm: 87,
            key: CamelotKey(number: 8, minor: true), onDecks: ["p"])
        XCTAssertNil(TrackMatching.rank(bpm: 174, key: "8A", id: "x", against: dub))
        let both = MatchWindow(halfDouble: true)
        XCTAssertEqual(TrackMatching.rank(bpm: 174, key: "8A", id: "x", against: dub, window: both)?.bpmDelta, 0)
        XCTAssertNotNil(TrackMatching.rank(bpm: 44, key: "8A", id: "x", against: dub, window: both), "88 at double")
        XCTAssertNil(TrackMatching.rank(bpm: 120, key: "8A", id: "x", against: dub, window: both))
        XCTAssertFalse(MatchWindow().followPitch, "the track's own BPM by default")
    }

    /// The loop row is mappable: each length on the ladder and both
    /// steppers, per deck, with ids that round-trip. None ships with a key.
    func testTheLoopRowMaps() {
        for action: DubAction in [
            .loopSize(.a, 0), .loopSize(.b, 7), .loopScale(.a, double: true), .loopScale(.b, double: false),
        ] {
            XCTAssertEqual(DubAction(id: action.id), action, action.id)
        }
        XCTAssertEqual(DubAction.loopSize(.a, 5).id, "loop.size.a.5")
        XCTAssertEqual(DubAction.loopScale(.b, double: true).id, "loop.double.b")
        XCTAssertEqual(LoopEngine.sizes.count, 8)
    }
}
