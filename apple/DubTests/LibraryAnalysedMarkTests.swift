import XCTest
@testable import Dub

final class LibraryAnalysedMarkTests: XCTestCase {
    private func row(
        analysed: Bool, deckA: Bool = false, deckB: Bool = false, warning: Bool = false
    ) -> LibraryRowState {
        LibraryRowState(
            cell: LibraryCellState(track: .fixture(isAnalyzed: analysed)),
            isOnDeckA: deckA, isOnDeckB: deckB, showsUnreachableWarning: warning)
    }

    func testOnlyAnAnalysedTrackCarriesTheMark() {
        XCTAssertTrue(row(analysed: true).showsAnalysedMark)
        XCTAssertFalse(row(analysed: false).showsAnalysedMark)
    }

    func testTheMarkGivesWayWhenTheGutterIsFull() {
        XCTAssertTrue(row(analysed: true, deckA: true).showsAnalysedMark)
        XCTAssertFalse(row(analysed: true, deckA: true, deckB: true).showsAnalysedMark)
        XCTAssertFalse(row(analysed: true, deckA: true, warning: true).showsAnalysedMark)
    }
}
