import XCTest

@testable import Dub

/// Preferences ▸ Audio on an interface Dub has no map for (FFI 87).
final class AudioRoutingTests: XCTestCase {

    func testPairsAreTheBackPanelsStereoPairs() {
        XCTAssertEqual(AudioRouting.pairs(channels: 4), [1, 3])
        XCTAssertEqual(AudioRouting.pairs(channels: 6), [1, 3, 5])
        XCTAssertEqual(AudioRouting.pairs(channels: 5), [1, 3])
        XCTAssertEqual(AudioRouting.pairs(channels: 1), [])
        XCTAssertEqual(AudioRouting.label(3), "3 – 4")
    }

    func testAFourByFourBoxStartsOnTwoDecks() {
        let r = AudioRouting.starting(inputs: 4, outputs: 4)
        XCTAssertTrue(r.twoDeck)
        XCTAssertEqual([r.deckAIn, r.deckBIn, r.deckAOut, r.deckBOut], [1, 3, 1, 3])
        XCTAssertNil(r.problem(inputs: 4, outputs: 4))
        XCTAssertFalse(AudioRouting.starting(inputs: 2, outputs: 4).twoDeck)
    }

    func testTwoDecksOnOnePairIsTheClash() {
        var r = AudioRouting.starting(inputs: 4, outputs: 4)
        r.deckBIn = 1
        XCTAssertEqual(r.problem(inputs: 4, outputs: 4)?.fault, .deckBInput)
        XCTAssertEqual(
            r.problem(inputs: 4, outputs: 4)?.message,
            "Deck A already takes inputs 1 – 2. Pick another pair for deck B, or switch to one deck.")
        r.twoDeck = false
        XCTAssertNil(r.problem(inputs: 4, outputs: 4), "one deck has nothing to clash with")
        r.twoDeck = true
        r.deckBIn = 3
        r.deckBOut = 1
        XCTAssertEqual(r.problem(inputs: 4, outputs: 4)?.fault, .deckBOutput)
    }

    /// A routing saved for a bigger box, on a smaller one.
    func testAPairPastTheLastChannelIsNamed() {
        let r = AudioRouting(twoDeck: true, deckAIn: 1, deckBIn: 5, deckAOut: 1, deckBOut: 3)
        XCTAssertEqual(
            r.problem(inputs: 4, outputs: 4)?.message,
            "This interface has 4 inputs; deck B can't take 5 – 6.")
    }

    func testTheEngineGetsWholePairsAndOneDeckDoublesItsOutput() {
        var r = AudioRouting.starting(inputs: 4, outputs: 4)
        let two = r.ffi(outputChannels: 4)
        XCTAssertEqual(two.deckAInput, [1, 2])
        XCTAssertEqual(two.deckBInput, [3, 4])
        XCTAssertEqual(two.outputChannels, 4)
        r.twoDeck = false
        XCTAssertEqual(r.ffi(outputChannels: 4).deckBOutputFirst, 1)
        XCTAssertEqual(AudioRouting(two), AudioRouting.starting(inputs: 4, outputs: 4))
    }

    func testTheStoreKeepsOneRoutingPerInterface() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "AudioRoutingTests"))
        defaults.removePersistentDomain(forName: "AudioRoutingTests")
        let r = AudioRouting.starting(inputs: 4, outputs: 4)
        AudioRoutingStore.save(r, for: "UMC404HD 192k", defaults: defaults)
        XCTAssertEqual(AudioRoutingStore.load("UMC404HD 192k", defaults: defaults), r)
        XCTAssertNil(AudioRoutingStore.load("Other Box", defaults: defaults))
        AudioRoutingStore.save(nil, for: "UMC404HD 192k", defaults: defaults)
        XCTAssertNil(AudioRoutingStore.load("UMC404HD 192k", defaults: defaults))
        defaults.removePersistentDomain(forName: "AudioRoutingTests")
    }
}
