import XCTest

@testable import Dub

/// Rig, 2026-09-23: both strips jumped now and then. A profile of the
/// app playing two timecode decks put 30 % of the main thread in
/// whole-window layout, because `deckA` / `deckB` republished on every
/// 30 Hz poll: the live pitch wobbles in its last digits and the input
/// levels never sit still, so every view observing the model — the
/// overview redrawing 480 bars, the library, the column — rebuilt
/// thirty times a second, and each late pass cost the strips a frame.
/// The numbers that move while a record plays are drawn on layers now
/// and read off the engine; the model does not carry the live pitch at
/// all (a held 0.1 % step still froze the window on every step of a
/// fader move), and the settled tempo moves in readout steps.
final class DeckPublishRateTests: XCTestCase {

    /// The settled tempo ignores drift under a readout step: a slow
    /// fader creep re-settles it every few polls, and each change is a
    /// model publish.
    func testDriftBelowAStepDoesNotMoveTheTempo() {
        var held: Double? = 2.03
        for settled in [2.05, 1.98, 2.10, 1.96, 2.07] {
            held = DeckState.heldPitch(settled, prev: held)
        }
        XCTAssertEqual(held, 2.03)
    }

    /// A real move is taken — the value it lands on, not a rounded one.
    func testAMoveIsTaken() {
        XCTAssertEqual(DeckState.heldPitch(2.4, prev: 2.03), 2.4)
        XCTAssertEqual(DeckState.heldPitch(1.9, prev: 2.03), 1.9)
    }

    /// Nothing settled yet: take it.
    func testTheFirstIsTaken() {
        XCTAssertEqual(DeckState.heldPitch(-3.2, prev: nil), -3.2)
    }

    /// The live BPM rule the layer readout uses is the header's rule:
    /// the live pitch inside the fader's band, the settled tempo
    /// outside it (a hand on the record), the base without either.
    func testLiveBpmFollowsThePitchInsideTheBand() {
        XCTAssertEqual(
            DeckHeaderState.liveBpm(base: 100, pitch: 8, tempoPitch: 0)!, 108, accuracy: 1e-9)
        XCTAssertEqual(
            DeckHeaderState.liveBpm(base: 100, pitch: 80, tempoPitch: 4)!, 104, accuracy: 1e-9)
        XCTAssertEqual(DeckHeaderState.liveBpm(base: 100, pitch: nil, tempoPitch: nil), 100)
        XCTAssertNil(DeckHeaderState.liveBpm(base: nil, pitch: 3, tempoPitch: nil))
    }
}
