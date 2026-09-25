import XCTest

@testable import Dub

/// The review as a sleeve's tracklist (rig, 2026-09-25): numbering the
/// way a DJ reads a record, and a short piece at either end suggested —
/// never removed — as the lead-in or run-out.
final class RipTracklistTests: XCTestCase {

    private func seg(_ i: UInt32, _ start: Double, _ end: Double, dropped: Bool = false) -> RipSegmentUi {
        RipSegmentUi(index: i, startSecs: start, endSecs: end, dropped: dropped)
    }

    /// A left-out lead-in does not take A1 from the first real track.
    func testNumberingCountsOnlyWhatImports() {
        let state = RipReviewPanelState(
            mode: .review, sideDurationSecs: 900,
            segments: [seg(0, 0, 9, dropped: true), seg(1, 9, 400), seg(2, 400, 900)])
        XCTAssertNil(state.position(of: state.segments[0]))
        XCTAssertEqual(state.position(of: state.segments[1]), "A1")
        XCTAssertEqual(state.position(of: state.segments[2]), "A2")
        XCTAssertEqual(state.importTitle, "Import 2 tracks")
        XCTAssertEqual(state.summaryText, "15:00 · 2 of 3 import")
    }

    func testSideBNumbersB() {
        var state = RipReviewPanelState(
            mode: .review, sideDurationSecs: 600, segments: [seg(0, 0, 300), seg(1, 300, 600)])
        state.sideLetter = "B"
        XCTAssertEqual(state.position(of: state.segments[1]), "B2")
    }

    func testShortPiecesAreNamedByWhereTheySit() {
        let state = RipReviewPanelState(
            mode: .review, sideDurationSecs: 620,
            segments: [seg(0, 0, 9), seg(1, 9, 300), seg(2, 300, 312), seg(3, 312, 600), seg(4, 600, 615)])
        XCTAssertEqual(state.shortPieceReason(state.segments[0]), "probably the lead-in")
        XCTAssertNil(state.shortPieceReason(state.segments[1]))
        XCTAssertEqual(state.shortPieceReason(state.segments[2]), "very short for a track")
        XCTAssertEqual(state.shortPieceReason(state.segments[4]), "probably the run-out")
    }

    /// A suggestion that already matches the row is not offered again.
    func testASuggestionAlreadyTakenIsNotOffered() {
        var s = seg(0, 0, 300)
        s.title = "Stop Them Jah"
        let meta = RipSegmentMetadata(title: "Stop Them Jah", artist: "", album: "", genre: "", year: "")
        var state = RipReviewPanelState(mode: .review, sideDurationSecs: 300, segments: [s])
        state.recognition = RipRecognitionUi(finished: true, suggestions: [0: meta])
        XCTAssertNil(state.suggestion(for: s))
        state.recognition?.suggestions[0]?.artist = "Augustus Pablo"
        XCTAssertNotNil(state.suggestion(for: s))
    }
}
