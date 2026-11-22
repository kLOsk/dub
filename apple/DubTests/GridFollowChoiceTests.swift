import XCTest
@testable import Dub

/// The DJ's word on one track's beat grid: follow the band, keep the
/// even grid, or leave it to Dub (Daniel, 2026-11-22).
final class GridFollowChoiceTests: XCTestCase {
    func testTheWordAndTheChoiceRoundTrip() {
        for choice in GridFollowChoice.allCases {
            XCTAssertEqual(GridFollowChoice(word: choice.word), choice)
        }
        XCTAssertEqual(GridFollowChoice(word: nil), .automatic)
        XCTAssertEqual(GridFollowChoice(word: true), .follow)
        XCTAssertEqual(GridFollowChoice(word: false), .even)
    }

    func testTheMenuReadsTheSameInTheLibraryAndOnTheDeck() {
        XCTAssertEqual(GridFollowChoice.menuTitle, "Beat grid")
        XCTAssertEqual(
            GridFollowChoice.allCases.map(\.title),
            ["Automatic", "Follow the band", "Even grid"])
    }

    func testTheDriftWarningSaysWhatIsWrongAndWhatToDo() {
        let sentence = LibraryCellFormat.BpmIssue.drift.sentence
        XCTAssertTrue(sentence.contains("tempo is slightly off"))
        XCTAssertTrue(sentence.contains("Re-analyze"))
        XCTAssertTrue(sentence.contains("Lock grid"))
    }
}
