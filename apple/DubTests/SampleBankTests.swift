import XCTest

@testable import Dub

/// M17: the sampler's eight positional slots, and the Quick Scratch
/// tags on them. The persisted form is read while building a view or
/// handling a press, so a bad value has to mean "nothing loaded",
/// never a throw.
final class SampleBankTests: XCTestCase {

    private let horn = URL(fileURLWithPath: "/Music/fx/horn.wav")
    private let stab = URL(fileURLWithPath: "/Music/fx/stab.aiff")
    private let rewind = URL(fileURLWithPath: "/Music/fx/rewind.mp3")

    func testStartsEmptyWithEightSlots() {
        let bank = SampleBank()
        XCTAssertTrue(bank.isEmpty)
        XCTAssertEqual(bank.all.count, SampleBank.count)
        XCTAssertEqual(SampleBank.count, 8)
        XCTAssertTrue(bank.all.allSatisfy { $0 == nil })
    }

    /// A pad is a place the hand learns. Unloading slot 2 must leave
    /// slot 3 where it was — the list-backed bank this replaced slid
    /// everything left, against its own doc comment.
    func testSlotsArePositional() {
        var bank = SampleBank()
        bank.load(horn, into: 1)
        bank.load(stab, into: 2)
        bank.load(rewind, into: 5)

        bank.unload(1)
        XCTAssertNil(bank.slot(1))
        XCTAssertEqual(bank.slot(2), stab, "slot 3 stays put")
        XCTAssertEqual(bank.slot(5), rewind)
        XCTAssertEqual(bank.loaded, [stab, rewind], "loaded files come back in slot order")
    }

    /// Dropping onto a full slot is the same gesture as filling an
    /// empty one.
    func testLoadingOntoAFullSlotReplacesIt() {
        var bank = SampleBank()
        bank.load(horn, into: 0)
        bank.load(stab, into: 0)
        XCTAssertEqual(bank.slot(0), stab)
        XCTAssertEqual(bank.loaded, [stab])
    }

    func testOutOfRangeIndicesAreIgnored() {
        var bank = SampleBank()
        bank.load(horn, into: 9)
        bank.load(horn, into: -1)
        bank.unload(9)
        XCTAssertNil(bank.slot(9))
        XCTAssertNil(bank.slot(-1))
        XCTAssertTrue(bank.isEmpty)
    }

    func testRoundTripsThroughItsPersistedFormWithGaps() {
        var bank = SampleBank()
        bank.load(horn, into: 0)
        bank.load(stab, into: 6)
        let restored = SampleBank(persisted: bank.persisted)
        XCTAssertEqual(restored, bank)
        XCTAssertNil(restored.slot(1))
        XCTAssertEqual(restored.slot(6), stab)
    }

    /// The bank used to be a plain list. That JSON is a gap-free slot
    /// table, so an upgrade keeps everyone's samples in the order they
    /// had them.
    func testLegacyListFormLandsInSlotOrder() {
        let legacy = "[\"\(horn.absoluteString)\",\"\(stab.absoluteString)\"]"
        let bank = SampleBank(persisted: legacy)
        XCTAssertEqual(bank.slot(0), horn)
        XCTAssertEqual(bank.slot(1), stab)
        XCTAssertNil(bank.slot(2))
    }

    func testMalformedBankDegradesToEmpty() {
        for junk in ["", "not json", "{}", "[1, 2]"] {
            let bank = SampleBank(persisted: junk)
            XCTAssertTrue(bank.isEmpty, "junk: \(junk)")
            XCTAssertEqual(bank.all.count, SampleBank.count, "junk: \(junk)")
        }
    }

    func testUrlsInitialiserFillsFromSlotZero() {
        let bank = SampleBank(urls: [horn, stab])
        XCTAssertEqual(bank.slot(0), horn)
        XCTAssertEqual(bank.slot(1), stab)
        XCTAssertEqual(bank.loaded, [horn, stab])
    }

    func testLabelIsTheFilename() {
        XCTAssertEqual(SampleBank.label(for: horn), "horn.wav")
    }

    // MARK: - Quick Scratch tags

    /// A pad names one slot: tagging another slot with the same pad
    /// moves the tag rather than leaving two samples answering to it.
    func testAQuickScratchPadNamesOneSlot() {
        var bank = SampleBank(urls: [horn, stab, rewind])
        bank.setQuickScratch(1, for: 0)
        XCTAssertEqual(bank.quickScratchTag(0), 1)
        XCTAssertEqual(bank.quickScratchSlot(pad: 1), 0)

        bank.setQuickScratch(1, for: 2)
        XCTAssertNil(bank.quickScratchTag(0), "the tag moved")
        XCTAssertEqual(bank.quickScratchSlot(pad: 1), 2)

        bank.setQuickScratch(nil, for: 2)
        XCTAssertNil(bank.quickScratchSlot(pad: 1))
    }

    func testTagsRespectTheirBounds() {
        var bank = SampleBank(urls: [horn])
        bank.setQuickScratch(0, for: 5)
        XCTAssertNil(bank.quickScratchTag(5), "an empty slot cannot be tagged")
        bank.setQuickScratch(SampleBank.quickScratchCount, for: 0)
        XCTAssertNil(bank.quickScratchTag(0), "no such pad")
        bank.setQuickScratch(-1, for: 0)
        XCTAssertNil(bank.quickScratchTag(0))
    }

    /// The tag names the pad, not the file: dropping a new file onto a
    /// tagged slot keeps the pad; unloading the slot clears it.
    func testTagsSurviveAReplaceAndNotAnUnload() {
        var bank = SampleBank(urls: [horn])
        bank.setQuickScratch(2, for: 0)
        bank.load(stab, into: 0)
        XCTAssertEqual(bank.quickScratchTag(0), 2)
        bank.unload(0)
        XCTAssertNil(bank.quickScratchSlot(pad: 2))
    }

    func testTagsRoundTripAndTheUntaggedFormStillLoads() {
        var bank = SampleBank(urls: [horn, stab])
        bank.setQuickScratch(3, for: 1)
        let restored = SampleBank(persisted: bank.persisted)
        XCTAssertEqual(restored, bank)
        XCTAssertEqual(restored.quickScratchTag(1), 3)

        // The positional `[URL?]` form the bank wrote before tags.
        let untagged = "[\"\(horn.absoluteString)\",null,\"\(stab.absoluteString)\"]"
        let migrated = SampleBank(persisted: untagged)
        XCTAssertEqual(migrated.slot(0), horn)
        XCTAssertNil(migrated.slot(1))
        XCTAssertEqual(migrated.slot(2), stab)
        XCTAssertNil(migrated.quickScratchTag(0))
    }

    // MARK: - Where a rack lands

    /// Auto follows the master; a pin does not; `A+B` is both, and
    /// shows A's state where only one deck's can be shown.
    func testRackOutputResolvesAgainstTheMaster() {
        XCTAssertEqual(RackOutputState(.auto, focused: .b).decks, [.b])
        XCTAssertEqual(RackOutputState(.auto, focused: .b).label, "→ B")
        XCTAssertFalse(RackOutputState(.auto, focused: .b).isPinned)

        let pinned = RackOutputState(.a, focused: .b)
        XCTAssertEqual(pinned.decks, [.a])
        XCTAssertEqual(pinned.label, "→ A")
        XCTAssertTrue(pinned.isPinned)
        XCTAssertEqual(pinned.tintDeck, .a)

        let both = RackOutputState(.both, focused: .b)
        XCTAssertEqual(both.decks, [.a, .b])
        XCTAssertEqual(both.primary, .a)
        XCTAssertEqual(both.label, "→ A+B")
        XCTAssertNil(both.tintDeck, "no one deck's colour")
    }

    func testRackOutputRoundTripsItsRawValue() {
        for output in RackOutput.allCases {
            XCTAssertEqual(RackOutput(rawValue: output.rawValue), output)
        }
        XCTAssertNil(RackOutput(rawValue: "sideways"))
    }

    // MARK: - The shelf's draw order

    /// Reading order: 1–4 over 5–8. The controller-style bottom-up
    /// numbering was tried and reverted.
    func testShelfDrawsInReadingOrder() {
        XCTAssertEqual(SampleShelf.slotOrder(count: 8), [0, 1, 2, 3, 4, 5, 6, 7])
        XCTAssertEqual(SampleShelf.slotOrder(count: 4), [0, 1, 2, 3])
    }

    func testShelfStateFromNamesIsIdle() {
        let state = SampleShelfState(names: ["Horn", nil, "Stab"])
        XCTAssertEqual(state.slots.map(\.name), ["Horn", nil, "Stab"])
        XCTAssertTrue(state.slots.allSatisfy { !$0.playing && $0.progress == 0 })
        XCTAssertNil(state.output)
        XCTAssertEqual(SampleShelfState.empty.slots.count, SampleBank.count)
    }
}
