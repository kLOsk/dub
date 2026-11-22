import XCTest
@testable import Dub

/// A search finds the record wherever it is, and shows the list on
/// screen first (Daniel, 2026-11-22).
final class LibrarySearchGroupsTests: XCTestCase {
    func testMatchesFromTheListOnScreenComeFirstAndBothGroupsKeepTheirOrder() {
        let sortedByTitle = ["a", "b", "c", "d", "e"]
        let grouped = LibrarySearchGroups.hereFirst(
            sortedByTitle, id: { $0 }, elsewhere: ["a", "d"])
        XCTAssertEqual(grouped, ["b", "c", "e", "a", "d"])
    }

    func testWithNothingFromElsewhereTheListIsLeftAlone() {
        XCTAssertEqual(
            LibrarySearchGroups.hereFirst(["b", "a"], id: { $0 }, elsewhere: []), ["b", "a"])
        XCTAssertNil(LibrarySearchGroups.firstElsewhere(["b", "a"], elsewhere: []))
    }

    func testTheDividerSitsAboveTheFirstMatchFromElsewhere() {
        XCTAssertEqual(
            LibrarySearchGroups.firstElsewhere(["b", "c", "a", "d"], elsewhere: ["a", "d"]), "a")
        // A filter can hide every match from elsewhere.
        XCTAssertNil(LibrarySearchGroups.firstElsewhere(["b", "c"], elsewhere: ["a"]))
    }

    func testTheCaptionSaysWhatWasFoundWhere() {
        XCTAssertEqual(
            LibrarySearchGroups.caption(
                shown: ["b", "c", "a"], elsewhere: ["a"], searching: true, inCollection: false),
            "2 in this list · 1 elsewhere")
        XCTAssertEqual(
            LibrarySearchGroups.caption(
                shown: ["a"], elsewhere: ["a"], searching: true, inCollection: true),
            "0 in your collection · 1 elsewhere")
        XCTAssertNil(
            LibrarySearchGroups.caption(
                shown: ["a"], elsewhere: [], searching: false, inCollection: true))
    }
}
