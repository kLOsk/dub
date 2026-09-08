import AppKit
import DubCore
import XCTest

@testable import Dub

/// The header's column picker.
///
/// Same shape as `LibraryRowMenu`: pure construction, no AppKit
/// display side effects, so it tests without an `NSWindow` to host a
/// popup. What matters here is that the menu's *state* matches the
/// live column set at build time — a stale checkmark or a hidden
/// "can't remove the last column" rule reads as a menu item that
/// clicks and does nothing.
@MainActor
final class LibraryColumnMenuTests: XCTestCase {

    private func builder(
        visible: [LibraryColumnField] = [.artist, .title, .duration, .bpm],
        camelot: Bool = true
    ) -> LibraryColumnMenu {
        let menu = LibraryColumnMenu()
        menu.visibleColumns = visible
        menu.keyNotationIsCamelot = camelot
        // Wired by default: an unwired item is deliberately disabled,
        // which would mask what the enablement tests are checking.
        menu.onSetVisibility = { _, _ in }
        menu.onToggleKeyNotation = {}
        return menu
    }

    /// Depth-first walk, so an assertion can name an item without
    /// caring which section or submenu it landed in.
    private func item(_ title: String, in menu: NSMenu) -> NSMenuItem? {
        for item in menu.items {
            if item.title == title { return item }
            if let sub = item.submenu, let hit = self.item(title, in: sub) { return hit }
        }
        return nil
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

    // MARK: - Checkmarks

    func testVisibleColumnIsChecked() {
        let menu = builder().menu()
        XCTAssertEqual(item("BPM", in: menu)?.state, .on)
    }

    func testHiddenColumnIsUnchecked() {
        let menu = builder().menu()
        XCTAssertEqual(item("Genre", in: menu)?.state, .off)
    }

    /// Artist and Title are always shown, so they read as checked but
    /// refuse the click rather than silently no-op'ing.
    func testFixedColumnsAreCheckedAndDisabled() {
        let menu = builder().menu()
        for title in ["Artist", "Title"] {
            let entry = item(title, in: menu)
            XCTAssertEqual(entry?.state, .on, title)
            XCTAssertEqual(entry?.isEnabled, false, title)
        }
    }

    // MARK: - Toggling

    func testShowingAHiddenColumnRequestsTrue() throws {
        var got: (LibraryColumnField, Bool)?
        let builder = builder()
        builder.onSetVisibility = { got = ($0, $1) }
        try fire("Genre", in: builder.menu())
        XCTAssertEqual(got?.0, .genre)
        XCTAssertEqual(got?.1, true)
    }

    func testHidingAVisibleColumnRequestsFalse() throws {
        var got: (LibraryColumnField, Bool)?
        let builder = builder()
        builder.onSetVisibility = { got = ($0, $1) }
        try fire("BPM", in: builder.menu())
        XCTAssertEqual(got?.0, .bpm)
        XCTAssertEqual(got?.1, false)
    }

    /// `setColumnVisibility` refuses to remove the last removable
    /// column. The menu has to show that, or the item clicks and
    /// nothing happens.
    func testTheLastRemovableColumnCannotBeHidden() {
        let menu = builder(visible: [.artist, .title, .bpm]).menu()
        XCTAssertEqual(item("BPM", in: menu)?.isEnabled, false)
        XCTAssertEqual(item("Genre", in: menu)?.isEnabled, true, "adding one is still fine")
    }

    // MARK: - Key notation

    func testKeyNotationToggleNamesTheOtherNotation() {
        let camelot = builder(visible: [.artist, .title, .key, .bpm], camelot: true).menu()
        XCTAssertNotNil(item("Toggle Key Notation (Musical)", in: camelot))

        let musical = builder(visible: [.artist, .title, .key, .bpm], camelot: false).menu()
        XCTAssertNotNil(item("Toggle Key Notation (Camelot)", in: musical))
    }

    func testKeyNotationToggleIsAbsentWithoutTheKeyColumn() {
        let menu = builder(visible: [.artist, .title, .bpm]).menu()
        XCTAssertNil(item("Toggle Key Notation (Musical)", in: menu))
    }

    func testKeyNotationToggleFires() throws {
        var fired = false
        let builder = builder(visible: [.artist, .title, .key, .bpm])
        builder.onToggleKeyNotation = { fired = true }
        try fire("Toggle Key Notation (Musical)", in: builder.menu())
        XCTAssertTrue(fired)
    }

    // MARK: - Registry columns

    /// The deeper groups come off the Rust registry, so the submenu
    /// has to be built from `LibraryColumnCatalog`, not a Swift list
    /// that would drift from it.
    func testMoreColumnsSubmenuMirrorsTheCatalog() throws {
        let more = try XCTUnwrap(
            item(LibraryColumnMenu.moreColumnsTitle, in: builder().menu()))
        XCTAssertEqual(
            more.submenu?.items.map(\.title),
            LibraryColumnCatalog.shared.groups)
    }

    func testARegistryColumnTogglesByItsStableId() throws {
        guard let info = LibraryColumnCatalog.shared.columns.first else {
            throw XCTSkip("the registry published no configurable columns")
        }
        var got: (LibraryColumnField, Bool)?
        let builder = builder()
        builder.onSetVisibility = { got = ($0, $1) }
        // Searched inside the submenu: a registry label may collide
        // with a fixed-set one, and the walk would find that instead.
        let more = try XCTUnwrap(
            item(LibraryColumnMenu.moreColumnsTitle, in: builder.menu()))
        let submenu = try XCTUnwrap(more.submenu)
        try fire(info.label, in: submenu)
        XCTAssertEqual(got?.0, .extra(info.id))
        XCTAssertEqual(got?.1, true)
    }

    /// Building the menu twice must not leave the first build's action
    /// targets holding the only reference — `NSMenuItem.target` is
    /// weak, and a dropped anchor is an item that clicks and does
    /// nothing.
    func testRebuildingKeepsItsActionTargetsAlive() throws {
        var count = 0
        let builder = builder()
        builder.onSetVisibility = { _, _ in count += 1 }
        _ = builder.menu()
        try fire("Genre", in: builder.menu())
        XCTAssertEqual(count, 1)
    }
}
