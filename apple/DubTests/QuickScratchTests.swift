import XCTest

@testable import Dub

/// M17 §7.2. The trigger itself is the library's load path; what is
/// new — and what can strand a DJ mid-set — is the persisted slot
/// table. A decode that throws, or one that returns the wrong number
/// of slots, must not take the keys down with it.
final class QuickScratchTests: XCTestCase {

    private let url = URL(fileURLWithPath: "/Music/breaks/apache.aiff")

    func testStartsAsFourEmptySlots() {
        let slots = QuickScratchSlots()
        XCTAssertEqual(slots.all.count, QuickScratchSlots.count)
        XCTAssertTrue(slots.all.allSatisfy { $0 == nil })
    }

    func testAssignAndClearASlot() {
        var slots = QuickScratchSlots()
        slots.assign(url: url, deck: .b, to: 2)

        XCTAssertEqual(slots.slot(2)?.url, url)
        XCTAssertEqual(slots.slot(2)?.deck, .b)
        XCTAssertNil(slots.slot(0))

        slots.clear(2)
        XCTAssertNil(slots.slot(2))
    }

    /// §7.2: "Each slot has a target deck (default: Deck A)".
    func testDefaultsToDeckA() {
        var slots = QuickScratchSlots()
        slots.assign(url: url, to: 0)
        XCTAssertEqual(slots.slot(0)?.deck, .a)
    }

    func testOutOfRangeIndicesAreIgnoredRatherThanTrapping() {
        var slots = QuickScratchSlots()
        slots.assign(url: url, to: 9)
        slots.assign(url: url, to: -1)
        slots.clear(9)
        XCTAssertNil(slots.slot(9))
        XCTAssertNil(slots.slot(-1))
        XCTAssertTrue(slots.all.allSatisfy { $0 == nil })
    }

    func testRoundTripsThroughItsPersistedForm() {
        var slots = QuickScratchSlots()
        slots.assign(url: url, deck: .b, to: 1)
        slots.assign(url: URL(fileURLWithPath: "/Music/horn.wav"), to: 3)

        let restored = QuickScratchSlots(persisted: slots.persisted)
        XCTAssertEqual(restored, slots)
        XCTAssertEqual(restored.slot(1)?.deck, .b)
        XCTAssertEqual(restored.slot(3)?.url.lastPathComponent, "horn.wav")
    }

    /// A preferences string that is empty, truncated or from another
    /// build degrades to "no slots bound", never to a crash on a
    /// keypress mid-set.
    func testMalformedPersistedFormDegradesToEmpty() {
        for junk in ["", "not json", "[]", "{\"slots\":[]}", "[{\"url\":1}]"] {
            let slots = QuickScratchSlots(persisted: junk)
            XCTAssertEqual(slots.all.count, QuickScratchSlots.count, "junk: \(junk)")
            XCTAssertTrue(slots.all.allSatisfy { $0 == nil }, "junk: \(junk)")
        }
    }

    /// A table saved when the slot count differs is padded or trimmed
    /// rather than rejected, so widening to six slots later (§7.1 notes
    /// it stays on the table) does not reset everyone's bindings.
    func testAShorterOrLongerSavedTableIsNormalised() {
        var two = QuickScratchSlots()
        two.assign(url: url, to: 0)
        let truncated = String(
            data: try! JSONEncoder().encode([two.slot(0)]), encoding: .utf8)!

        let restored = QuickScratchSlots(persisted: truncated)
        XCTAssertEqual(restored.all.count, QuickScratchSlots.count)
        XCTAssertEqual(restored.slot(0)?.url, url)
        XCTAssertNil(restored.slot(3))
    }
}
