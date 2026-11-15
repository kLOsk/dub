import AppKit
import XCTest

@testable import Dub

/// The filter boxes size to their content (2026-09-22, the DJ's ask):
/// a flat 168 pt truncated every GENRE row while COLOUR sat half
/// empty. What has to hold is that the box is wide enough for its own
/// widest row, never narrower than the floor, and never wide enough to
/// push its neighbours off the scrolling bar.
final class LibraryFilterBoxWidthTests: XCTestCase {

    private func width(
        header: String = "Genre",
        labels: [String],
        counts: [Int]? = nil,
        swatch: Bool = false,
        maxWidth: CGFloat = DubLayout.libraryFilterBoxMaxWidth
    ) -> CGFloat {
        LibraryFilterBoxWidth.width(
            header: header,
            labels: labels,
            counts: counts ?? labels.map { _ in 1 },
            hasSwatch: swatch,
            maxWidth: maxWidth)
    }

    /// Short values don't shrink the box past the floor — five stars
    /// and a clear button still have to fit.
    func testShortContentSitsOnTheFloor() {
        XCTAssertEqual(
            width(header: "Key", labels: ["7A", "6A", "12B"]),
            DubLayout.libraryFilterBoxMinWidth)
        XCTAssertEqual(
            width(header: "BPM", labels: []),
            DubLayout.libraryFilterBoxMinWidth)
    }

    /// A long value grows the box rather than truncating, and the
    /// widest row is what sets it.
    func testTheWidestRowSetsTheWidth() {
        let runaway = String(repeating: "Drum & Bass - Irie ", count: 4)
        let short = width(labels: ["Dub"])
        let long = width(labels: ["Dub", runaway])
        XCTAssertGreaterThan(long, short)
        XCTAssertEqual(long, width(labels: [runaway]))
    }

    /// Past the ceiling the box stops growing: the bar scrolls, and one
    /// runaway genre would otherwise push every box after it out of reach.
    func testTheCeilingHolds() {
        let w = width(labels: [String(repeating: "Reggae Revival ", count: 24)])
        XCTAssertEqual(w, DubLayout.libraryFilterBoxMaxWidth)
    }

    /// The count sits at the row's trailing edge, so a four-digit count
    /// needs room the label doesn't have to give up.
    func testACountIsPartOfTheRow() {
        // A label long enough to be off the floor, or both clamp to it
        // and the count has nothing to widen.
        let label = String(repeating: "Drum & Bass - Irie ", count: 2)
        XCTAssertGreaterThan(
            width(labels: [label], counts: [1234]),
            width(labels: [label], counts: [1]))
    }

    /// COLOUR draws a swatch the other boxes don't.
    func testTheSwatchIsCountedOnlyWhereItIsDrawn() {
        let labels = [String(repeating: "Some fairly long label here ", count: 2)]
        XCTAssertGreaterThan(
            width(header: "Color", labels: labels, swatch: true),
            width(header: "Color", labels: labels, swatch: false))
    }

    /// The row's furniture is measured as drawn: a box sized for
    /// "Nice - Warmup - Friendly" truncated it on the rig (2026-11-15)
    /// because one stack gap, the spacer and the scroller were left out.
    /// The label alone must leave room for at least those.
    func testARowLeavesRoomForItsFurniture() {
        let label = "Nice - Warmup - Friendly"
        let text = (label as NSString).size(
            withAttributes: [.font: NSFont.systemFont(ofSize: LibraryFilterBoxWidth.textSize)]).width
        let furniture = LibraryFilterBoxWidth.glyphSize + DubSpacing.xs * 3 + 2 + DubSpacing.sm * 2
            + NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy)
        XCTAssertGreaterThanOrEqual(width(labels: [label]), text + furniture)
    }

    /// On a wide bar a box may grow to the absolute ceiling; on a narrow
    /// one it gets a share of the bar, so one long COMMENT can't push
    /// every other box out of sight. Never under the floor.
    func testTheCeilingFollowsTheBar() {
        XCTAssertEqual(LibraryFilterBoxWidth.ceiling(barWidth: nil), DubLayout.libraryFilterBoxMaxWidth)
        XCTAssertEqual(LibraryFilterBoxWidth.ceiling(barWidth: 4000), DubLayout.libraryFilterBoxMaxWidth)
        let narrow = LibraryFilterBoxWidth.ceiling(barWidth: 600)
        XCTAssertLessThan(narrow, DubLayout.libraryFilterBoxMaxWidth)
        XCTAssertEqual(LibraryFilterBoxWidth.ceiling(barWidth: 100), DubLayout.libraryFilterBoxMinWidth)

        let runaway = [String(repeating: "Reggae Revival ", count: 24)]
        XCTAssertEqual(width(labels: runaway, maxWidth: narrow), narrow)
    }

    /// The rows are read on stage: at least the track list's size.
    func testTheRowsAreAsBigAsTheTrackList() {
        XCTAssertGreaterThanOrEqual(LibraryFilterBoxWidth.textSize, 14)
    }

    /// The folded FILTER line names what is set, box by box, and skips
    /// a box with nothing ticked.
    func testTheFoldedLineSaysWhatIsFiltered() {
        XCTAssertEqual(
            LibraryFilterSummary.text([
                ("Genre", ["Dub", "Reggae"]), ("Key", []), ("BPM", ["160–180"]), ("Rating", ["≥ 3★"]),
            ]),
            "GENRE Dub, Reggae · BPM 160–180 · RATING ≥ 3★")
        XCTAssertEqual(LibraryFilterSummary.text([]), "")
    }

    /// Ticking a row must not resize the box under the pointer — the
    /// clear button's width is reserved whether or not it is showing,
    /// which is what makes the width a pure function of the facets.
    func testTheHeaderAloneCanSetTheWidth() {
        let w = width(header: "Composer", labels: ["a"])
        XCTAssertGreaterThanOrEqual(w, DubLayout.libraryFilterBoxMinWidth)
        XCTAssertLessThanOrEqual(w, DubLayout.libraryFilterBoxMaxWidth)
    }
}
