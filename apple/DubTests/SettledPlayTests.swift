import XCTest
@testable import Dub

final class SettledPlayTests: XCTestCase {
    func testADeckThatFlickersNeverCountsAsStarted() {
        var deck = SettledPlay()
        var t = 0.0
        for i in 0..<200 {
            t += 0.05
            XCTAssertFalse(deck.update(playing: i % 3 != 0, at: t))
            XCTAssertFalse(deck.isPlaying)
        }
    }

    func testADeckThatKeepsPlayingStartsOnceAfterTheHold() {
        var deck = SettledPlay()
        var starts = 0
        var t = 0.0
        while t < 3 {
            if deck.update(playing: true, at: t) { starts += 1 }
            if t < SettledPlay.holdSecs - 0.01 { XCTAssertFalse(deck.isPlaying) }
            t += 1.0 / 60.0
        }
        XCTAssertEqual(starts, 1)
        XCTAssertTrue(deck.isPlaying)
    }

    func testAShortDropoutDoesNotStopAPlayingDeck() {
        var deck = SettledPlay()
        deck.force(true)
        XCTAssertFalse(deck.update(playing: false, at: 10.0))
        XCTAssertFalse(deck.update(playing: false, at: 10.3))
        XCTAssertFalse(deck.update(playing: true, at: 10.4), "resuming is not a new start")
        XCTAssertTrue(deck.isPlaying)
        XCTAssertFalse(deck.update(playing: false, at: 11.0))
        XCTAssertFalse(deck.update(playing: false, at: 11.7))
        XCTAssertFalse(deck.isPlaying)
    }

    func testAStartFromDubCountsAtOnce() {
        var deck = SettledPlay()
        deck.force(true)
        XCTAssertTrue(deck.isPlaying)
        XCTAssertFalse(deck.update(playing: true, at: 0))
    }
}
