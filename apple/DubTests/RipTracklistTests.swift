import XCTest

@testable import Dub

/// The review as a tracklist (rig, 2026-09-25): numbering by place on the
/// side, and a short piece at either end suggested — never removed — as
/// the lead-in or run-out.
final class RipTracklistTests: XCTestCase {

    private func seg(_ i: UInt32, _ start: Double, _ end: Double, dropped: Bool = false) -> RipSegmentUi {
        RipSegmentUi(index: i, startSecs: start, endSecs: end, dropped: dropped)
    }

    /// Numbered by where the piece sits on the side, left-out ones
    /// included: the number only matches a map block to a row. A DJ rips
    /// the songs they want, so neither a sleeve position nor a count of
    /// the kept tracks means anything (rig, 2026-09-25).
    func testNumberingFollowsTheSideIncludingLeftOutPieces() {
        let state = RipReviewPanelState(
            mode: .review, sideDurationSecs: 900,
            segments: [seg(0, 0, 9, dropped: true), seg(1, 9, 400), seg(2, 400, 900)])
        XCTAssertEqual(state.position(of: state.segments[0]), "1")
        XCTAssertEqual(state.position(of: state.segments[1]), "2")
        XCTAssertEqual(state.position(of: state.segments[2]), "3")
        XCTAssertEqual(state.importTitle, "Import 2 tracks")
        XCTAssertEqual(state.summaryText, "15:00 · 2 of 3 import")
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

    /// A genre typed on one row is offered on the others before the
    /// library has it; never on its own row, whose list would move under
    /// the letters being typed; in the library's spelling when it has
    /// one (rig, 2026-11-27).
    func testAGenreTypedOnOneRowIsOfferedOnTheOthers() {
        var one = seg(0, 0, 300)
        one.genre = "Reggae "
        var two = seg(1, 300, 600)
        two.genre = "dubstep - wobble"
        let three = seg(2, 600, 900)
        var state = RipReviewPanelState(mode: .review, sideDurationSecs: 900, segments: [one, two, three])
        state.genres = ["Dubstep - Wobble", "Hip-Hop"]
        XCTAssertEqual(state.genres(for: three), ["Reggae", "Dubstep - Wobble", "Hip-Hop"])
        XCTAssertEqual(state.genres(for: one), ["Dubstep - Wobble", "Hip-Hop"])
        XCTAssertEqual(state.genres(for: two), ["Reggae", "Dubstep - Wobble", "Hip-Hop"])
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

    /// A row re-seeds on a re-cut or on Use all, and only then; the seed
    /// carries the names, so the re-seed takes the ones that just landed
    /// (rig, 2026-11-28: read from the row, Use all seeded them empty).
    func testARowReSeedsOnUseAllWithTheNamesThatLanded() {
        var before = seg(1, 9, 400)
        var after = before
        after.title = "Love Train"
        after.artist = "The O'Jays"
        // The row's own edit coming back through the poll: no re-seed.
        XCTAssertEqual(
            RipRowSeed(segment: before, revision: 3), RipRowSeed(segment: after, revision: 3))
        // Use all bumps the revision: a re-seed, with the new names.
        let landed = RipRowSeed(segment: after, revision: 4)
        XCTAssertNotEqual(RipRowSeed(segment: before, revision: 3), landed)
        XCTAssertEqual(landed.names.title, "Love Train")
        XCTAssertEqual(landed.names.artist, "The O'Jays")
        // A re-cut re-seeds too.
        before.endSecs = 380
        XCTAssertNotEqual(
            RipRowSeed(segment: before, revision: 3), RipRowSeed(segment: after, revision: 3))
    }

    /// A re-split is left, not discarded: nothing on disk goes, so the
    /// button says so and asks no "really?" (Daniel, 2026-11-28).
    func testAReSplitIsLeftAndANewRecordingIsDiscarded() {
        XCTAssertEqual(RipReviewPanelState.leaveTitle(isResplit: true, armed: false), "Leave re-split")
        XCTAssertEqual(RipReviewPanelState.leaveTitle(isResplit: true, armed: true), "Leave re-split")
        XCTAssertEqual(RipReviewPanelState.leaveTitle(isResplit: false, armed: false), "Discard side…")
        XCTAssertEqual(
            RipReviewPanelState.leaveTitle(isResplit: false, armed: true), "Really discard the side?")
    }
}
