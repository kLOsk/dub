import XCTest

@testable import Dub

/// A THRU deck's BPM: the record's tempo as the tracker hears it, unless
/// the DJ tapped one — which holds for that record and gives way when the
/// needle comes up and the next one goes down (Daniel, 2026-11-11).
final class ThruTempoTests: XCTestCase {

    private func thruDeck() -> DeckState {
        var deck = DeckState()
        deck.controlMode = 2
        return deck
    }

    func testOffThruThereIsNoThruTempo() {
        var deck = DeckState()
        deck.controlMode = 1
        deck.thruBpm = 92.5
        XCTAssertNil(deck.thruReadout)
    }

    /// Listening, then a tentative number, then a locked one.
    func testTheTrackerReadsTentativeThenLocked() {
        var deck = thruDeck()
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: nil))
        deck.thruBpm = 133.4
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 133.4, locked: false))
        deck.thruLocked = true
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 133.4, locked: true))
    }

    /// The tracker read a reggae record at double time; the DJ taps the
    /// half. The tap wins while that record plays, whatever the tracker
    /// keeps saying.
    func testATapWinsForTheRecordItWasTappedOn() {
        var deck = thruDeck()
        deck.thruBpm = 133.4
        deck.thruLocked = true
        deck.thruRecordSeq = 3
        deck.thruTap = DeckState.ThruTap(bpm: 66.7, recordSeq: 3)
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 66.7, tapped: true, locked: true))
        XCTAssertEqual(deck.thruReadout?.tempo, 66.7)
    }

    /// The needle comes up, the next record goes down: the tracker counts
    /// a new record, and the old tap no longer speaks for it.
    func testTheNextRecordRetiresTheTap() {
        var deck = thruDeck()
        deck.thruRecordSeq = 4
        deck.thruTap = DeckState.ThruTap(bpm: 66.7, recordSeq: 3)
        deck.thruBpm = 174.0
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 174.0, tapped: false, locked: false))
    }

    /// Four taps half a second apart are 120.
    func testFourTapsMakeATempo() {
        XCTAssertEqual(FxRackControls.tappedBpm([0, 0.5, 1.0, 1.5]) ?? 0, 120, accuracy: 0.01)
        XCTAssertNil(FxRackControls.tappedBpm([0]))
    }
}
