import AppKit
import DubCore
import XCTest

@testable import Dub

/// The library row's right-click menu.
///
/// `LibraryRowMenu`'s own header has claimed this file existed since
/// the `NSTableView` migration; it did not. The class is pure
/// construction, so it tests without an `NSWindow` to host the popup —
/// which is the reason it was lifted out of the old scroll container's
/// coordinator in the first place.
///
/// The two things worth pinning: the labels count the *live* selection
/// (a SwiftUI `.contextMenu` captured a stale one, which is why this is
/// AppKit at all), and the crate "Move…" items are index arithmetic
/// against the visible order, which goes wrong quietly at the edges.
@MainActor
final class LibraryRowMenuTests: XCTestCase {

    private func rows(_ n: Int) -> [LibraryTrack] {
        (0..<n).map { LibraryTrack.fixture(id: "t\($0)") }
    }

    private func builder(
        tracks: [LibraryTrack]? = nil,
        selected: Set<String> = [],
        batchInProgress: Bool = false,
        crateId: Int64? = nil
    ) -> LibraryRowMenu {
        let menu = LibraryRowMenu()
        menu.tracks = tracks ?? rows(5)
        menu.selectedTrackIds = selected
        menu.analysisBatchInProgress = batchInProgress
        menu.crateId = crateId
        // Wired by default: an unwired item is deliberately disabled,
        // which would mask what the enablement tests are checking.
        menu.onAnalyzeRequested = { _ in }
        menu.onSetGridLocked = { _, _ in }
        menu.onCrateRemove = { _ in }
        menu.onCrateMove = { _, _ in }
        return menu
    }

    private func item(_ title: String, in menu: NSMenu) -> NSMenuItem? {
        menu.items.first { $0.title == title }
    }

    private func fire(
        _ title: String, in menu: NSMenu,
        file: StaticString = #filePath, line: UInt = #line
    ) throws {
        let entry = try XCTUnwrap(item(title, in: menu), title, file: file, line: line)
        let target = try XCTUnwrap(
            entry.target as? LibraryMenuActionTarget,
            "\(title) has no action target", file: file, line: line)
        target.dubMenuPerform(entry)
    }

    // MARK: - Analyze target set

    /// Finder semantics: right-clicking inside a multi-selection acts
    /// on the whole selection.
    func testRightClickInsideTheSelectionActsOnAllOfIt() {
        let builder = builder(selected: ["t0", "t1", "t2"])
        XCTAssertEqual(
            Set(builder.analyzeTargets(rightClickedTrack: .fixture(id: "t1"))),
            ["t0", "t1", "t2"])
    }

    /// …and right-clicking outside it acts on that row alone, without
    /// disturbing the selection.
    func testRightClickOutsideTheSelectionActsOnThatRowAlone() {
        let builder = builder(selected: ["t0", "t1", "t2"])
        XCTAssertEqual(
            builder.analyzeTargets(rightClickedTrack: .fixture(id: "t4")), ["t4"])
    }

    func testASingleSelectionActsOnTheClickedRow() {
        let builder = builder(selected: ["t1"])
        XCTAssertEqual(
            builder.analyzeTargets(rightClickedTrack: .fixture(id: "t1")), ["t1"])
    }

    // MARK: - Analyze label

    func testAnalyzeVerbFollowsWhetherTheRowIsAnalyzed() {
        let builder = builder()
        XCTAssertEqual(
            builder.analyzeMenuLabel(
                rightClickedTrack: .fixture(isAnalyzed: false), allCount: 1, unlockedCount: 1),
            "Analyze")
        XCTAssertEqual(
            builder.analyzeMenuLabel(
                rightClickedTrack: .fixture(isAnalyzed: true), allCount: 1, unlockedCount: 1),
            "Re-analyze")
    }

    func testMultiSelectionLabelCountsTheSelection() {
        XCTAssertEqual(
            builder().analyzeMenuLabel(
                rightClickedTrack: .fixture(), allCount: 5, unlockedCount: 5),
            "Re-analyze Selected (5)")
    }

    /// PRD-BEATS §4.4 — a mixed-lock selection says how many rows the
    /// pass will skip *before* the click, not after.
    func testMixedLockSelectionLabelShowsBothCounts() {
        XCTAssertEqual(
            builder().analyzeMenuLabel(
                rightClickedTrack: .fixture(), allCount: 5, unlockedCount: 3),
            "Re-analyze Selected (3 of 5)")
    }

    // MARK: - Analyze enablement

    func testAnalyzeIsDisabledWhileAnotherBatchRuns() {
        let menu = builder(batchInProgress: true).menu(for: .fixture(id: "t0"))
        XCTAssertEqual(item("Re-analyze", in: menu)?.isEnabled, false)
    }

    /// Every target locked means nothing to do — the item stays visible
    /// so the reason is legible, but it must not click.
    func testAnalyzeIsDisabledWhenEveryTargetIsLocked() {
        let locked = LibraryTrack.fixture(id: "t0", gridLocked: true)
        let builder = builder(tracks: [locked])
        let menu = builder.menu(for: locked)
        XCTAssertEqual(item("Re-analyze", in: menu)?.isEnabled, false)
    }

    func testAnalyzeRequestsOnlyTheUnlockedRows() throws {
        var requested: [String]?
        let tracks = [
            LibraryTrack.fixture(id: "t0"),
            LibraryTrack.fixture(id: "t1", gridLocked: true),
            LibraryTrack.fixture(id: "t2"),
        ]
        let builder = builder(tracks: tracks, selected: ["t0", "t1", "t2"])
        builder.onAnalyzeRequested = { requested = $0 }
        try fire("Re-analyze Selected (2 of 3)", in: builder.menu(for: tracks[0]))
        XCTAssertEqual(requested.map(Set.init), ["t0", "t2"])
    }

    // MARK: - Grid lock

    func testLockItemNamesTheOtherState() {
        let unlocked = LibraryTrack.fixture(id: "t0", gridLocked: false)
        XCTAssertNotNil(item("Lock grid", in: builder().menu(for: unlocked)))

        let locked = LibraryTrack.fixture(id: "t0", gridLocked: true)
        XCTAssertNotNil(item("Unlock grid", in: builder(tracks: [locked]).menu(for: locked)))
    }

    func testLockFiresWithTheInvertedState() throws {
        var got: (String, Bool)?
        let track = LibraryTrack.fixture(id: "t0", gridLocked: false)
        let builder = builder()
        builder.onSetGridLocked = { got = ($0, $1) }
        try fire("Lock grid", in: builder.menu(for: track))
        XCTAssertEqual(got?.0, "t0")
        XCTAssertEqual(got?.1, true)
    }

    // MARK: - Sample lineage (R-49)

    func testSampleLookUpIsDisabledWithoutArtistOrTitle() {
        let bare = LibraryTrack.fixture(id: "t0", title: nil, artist: nil)
        let menu = builder(tracks: [bare]).menu(for: bare)
        XCTAssertEqual(item(SampleLineage.actionTitle, in: menu)?.isEnabled, false)
    }

    func testSampleLookUpIsEnabledWithEitherOne() {
        let titleOnly = LibraryTrack.fixture(id: "t0", artist: nil)
        let menu = builder(tracks: [titleOnly]).menu(for: titleOnly)
        XCTAssertEqual(item(SampleLineage.actionTitle, in: menu)?.isEnabled, true)
    }

    // MARK: - Crate items

    func testCrateItemsAreAbsentOutsideACrate() {
        let menu = builder(crateId: nil).menu(for: .fixture(id: "t0"))
        XCTAssertNil(item("Remove from Crate", in: menu))
        XCTAssertNil(item("Move Up", in: menu))
    }

    func testCrateItemsAppearInsideACrate() {
        let menu = builder(crateId: 7).menu(for: .fixture(id: "t1"))
        XCTAssertNotNil(item("Remove from Crate", in: menu))
        XCTAssertNotNil(item("Move Up", in: menu))
    }

    func testRemoveLabelCountsAMultiSelection() {
        let menu = builder(selected: ["t0", "t1"], crateId: 7).menu(for: .fixture(id: "t0"))
        XCTAssertNotNil(item("Remove from Crate (2)", in: menu))
    }

    /// The edge cases the index arithmetic gets wrong quietly: the
    /// first row cannot move up, the last cannot move down.
    func testMoveItemsAreDisabledAtTheFirstRow() {
        let menu = builder(crateId: 7).menu(for: .fixture(id: "t0"))
        XCTAssertEqual(item("Move Up", in: menu)?.isEnabled, false)
        XCTAssertEqual(item("Move to Top", in: menu)?.isEnabled, false)
        XCTAssertEqual(item("Move Down", in: menu)?.isEnabled, true)
        XCTAssertEqual(item("Move to Bottom", in: menu)?.isEnabled, true)
    }

    func testMoveItemsAreDisabledAtTheLastRow() {
        let menu = builder(crateId: 7).menu(for: .fixture(id: "t4"))
        XCTAssertEqual(item("Move Up", in: menu)?.isEnabled, true)
        XCTAssertEqual(item("Move to Top", in: menu)?.isEnabled, true)
        XCTAssertEqual(item("Move Down", in: menu)?.isEnabled, false)
        XCTAssertEqual(item("Move to Bottom", in: menu)?.isEnabled, false)
    }

    func testAMiddleRowMovesBothWays() {
        let menu = builder(crateId: 7).menu(for: .fixture(id: "t2"))
        for title in ["Move Up", "Move Down", "Move to Top", "Move to Bottom"] {
            XCTAssertEqual(item(title, in: menu)?.isEnabled, true, title)
        }
    }

    func testMoveFiresWithTheClickedRowAndDirection() throws {
        var got: (String, CrateMove)?
        let builder = builder(crateId: 7)
        builder.onCrateMove = { got = ($0, $1) }
        try fire("Move to Bottom", in: builder.menu(for: .fixture(id: "t2")))
        XCTAssertEqual(got?.0, "t2")
        XCTAssertEqual(got?.1, .bottom)
    }

    /// `NSMenuItem.target` is weak, so the builder has to outlive the
    /// build — a dropped anchor is an item that clicks and does
    /// nothing, which is the bug its `anchors` list exists to prevent.
    func testActionTargetsSurviveRepeatedBuilds() throws {
        var count = 0
        let builder = builder()
        builder.onSetGridLocked = { _, _ in count += 1 }
        for _ in 0..<20 { _ = builder.menu(for: .fixture(id: "t0")) }
        try fire("Lock grid", in: builder.menu(for: .fixture(id: "t0")))
        XCTAssertEqual(count, 1)
    }
}
