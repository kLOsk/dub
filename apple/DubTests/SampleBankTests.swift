import XCTest

@testable import Dub

/// M17: the sampler's eight positional slots. Same concern as
/// `QuickScratchTests` — the persisted form is read while building a
/// view or handling a press, so a bad value has to mean "nothing
/// loaded", never a throw.
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

    // MARK: - The shelf's draw order

    /// 5–8 over 1–4, the way a pad controller's rows count up from the
    /// hand.
    func testShelfDrawsTheTopHalfFirst() {
        XCTAssertEqual(SampleShelf.slotOrder(count: 8), [4, 5, 6, 7, 0, 1, 2, 3])
        XCTAssertEqual(SampleShelf.slotOrder(count: 4), [2, 3, 0, 1])
    }

    func testShelfStateFromNamesIsIdle() {
        let state = SampleShelfState(names: ["Horn", nil, "Stab"])
        XCTAssertEqual(state.slots.map(\.name), ["Horn", nil, "Stab"])
        XCTAssertTrue(state.slots.allSatisfy { !$0.playing && $0.progress == 0 })
        XCTAssertNil(state.focusedDeck)
        XCTAssertEqual(SampleShelfState.empty.slots.count, SampleBank.count)
    }
}
