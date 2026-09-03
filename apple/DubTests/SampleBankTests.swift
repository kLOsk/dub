import XCTest

@testable import Dub

/// M17: the shared bank both racks bind from, and the sampler's own
/// slot table. Same concern as `QuickScratchTests` — the persisted
/// form is read while building a sheet or handling a press, so a bad
/// value has to mean "nothing bound", never a throw.
final class SampleBankTests: XCTestCase {

    private let horn = URL(fileURLWithPath: "/Music/fx/horn.wav")
    private let stab = URL(fileURLWithPath: "/Music/fx/stab.aiff")

    // MARK: - Bank

    func testAddsInOrderAndIgnoresDuplicates() {
        var bank = SampleBank()
        XCTAssertTrue(bank.isEmpty)
        bank.add(horn)
        bank.add(stab)
        bank.add(horn)

        XCTAssertEqual(bank.all, [horn, stab], "the same horn twice is one entry")
    }

    func testRemoveDropsOnlyThatEntry() {
        var bank = SampleBank(urls: [horn, stab])
        bank.remove(horn)
        XCTAssertEqual(bank.all, [stab])
    }

    /// A binding older than the bank — or hand-edited — must appear in
    /// the list rather than being audibly bound but invisible.
    func testAdoptBringsInAnExistingBindingWithoutReordering() {
        var bank = SampleBank(urls: [stab])
        bank.adopt(horn)
        bank.adopt(stab)
        XCTAssertEqual(bank.all, [stab, horn])
    }

    func testBankRoundTripsThroughItsPersistedForm() {
        let bank = SampleBank(urls: [horn, stab])
        XCTAssertEqual(SampleBank(persisted: bank.persisted), bank)
    }

    func testMalformedBankDegradesToEmpty() {
        for junk in ["", "not json", "{}", "[1, 2]"] {
            XCTAssertTrue(SampleBank(persisted: junk).isEmpty, "junk: \(junk)")
        }
    }

    func testLabelIsTheFilename() {
        XCTAssertEqual(SampleBank.label(for: horn), "horn.wav")
    }

    // MARK: - Sampler slots

    func testSamplerStartsEmptyWithFourSlots() {
        let slots = SamplerSlots()
        XCTAssertEqual(slots.all.count, SamplerSlots.count)
        XCTAssertTrue(slots.all.allSatisfy { $0 == nil })
        XCTAssertEqual(SamplerSlots.keyLabels.count, SamplerSlots.count)
    }

    func testSamplerSlotDefaultsToUnityGainOnDeckA() {
        var slots = SamplerSlots()
        slots.assign(url: horn, to: 0)
        XCTAssertEqual(slots.slot(0)?.gain, 1.0)
        XCTAssertEqual(slots.slot(0)?.deck, .a)
    }

    /// Gain and output belong to the *pad*, not the file: re-pointing a
    /// pad at a different sample keeps the level the DJ dialled in.
    func testRebindingASlotKeepsItsGainAndOutput() {
        var slots = SamplerSlots()
        slots.assign(url: horn, to: 1)
        slots.setGain(0.4, for: 1)
        slots.setDeck(.b, for: 1)

        slots.assign(url: stab, to: 1)
        XCTAssertEqual(slots.slot(1)?.url, stab)
        XCTAssertEqual(slots.slot(1)?.gain, 0.4)
        XCTAssertEqual(slots.slot(1)?.deck, .b)
    }

    func testGainIsClampedToTheEnginesRange() {
        var slots = SamplerSlots()
        slots.assign(url: horn, to: 0)
        slots.setGain(-3, for: 0)
        XCTAssertEqual(slots.slot(0)?.gain, 0.0, "no phase inversion against the deck")
        slots.setGain(99, for: 0)
        XCTAssertEqual(slots.slot(0)?.gain, 4.0)
    }

    func testSamplerOutOfRangeIndicesAreIgnored() {
        var slots = SamplerSlots()
        slots.assign(url: horn, to: 9)
        slots.setGain(2, for: 9)
        slots.setDeck(.b, for: 9)
        slots.clear(9)
        XCTAssertNil(slots.slot(9))
        XCTAssertTrue(slots.all.allSatisfy { $0 == nil })
    }

    func testSamplerRoundTripsThroughItsPersistedForm() {
        var slots = SamplerSlots()
        slots.assign(url: horn, to: 0)
        slots.setGain(0.75, for: 0)
        slots.setDeck(.b, for: 0)
        slots.assign(url: stab, to: 3)

        let restored = SamplerSlots(persisted: slots.persisted)
        XCTAssertEqual(restored, slots)
        XCTAssertEqual(restored.slot(0)?.gain, 0.75)
        XCTAssertEqual(restored.slot(0)?.deck, .b)
        XCTAssertNil(restored.slot(1))
    }

    func testMalformedSamplerTableDegradesToEmpty() {
        for junk in ["", "not json", "[]", "[{\"url\":1}]"] {
            let slots = SamplerSlots(persisted: junk)
            XCTAssertEqual(slots.all.count, SamplerSlots.count, "junk: \(junk)")
            XCTAssertTrue(slots.all.allSatisfy { $0 == nil }, "junk: \(junk)")
        }
    }

    func testBoundUrlsAreWhatTheBankAdopts() {
        var slots = SamplerSlots()
        slots.assign(url: horn, to: 0)
        slots.assign(url: stab, to: 2)
        XCTAssertEqual(slots.boundUrls, [horn, stab])

        var quick = QuickScratchSlots()
        quick.assign(url: horn, to: 3)
        XCTAssertEqual(quick.boundUrls, [horn])
    }
}
