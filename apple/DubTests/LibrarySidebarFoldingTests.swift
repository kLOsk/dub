import DubCore
import XCTest

@testable import Dub

/// The library sidebar's folds (Daniel, 2026-11-16: "library left needs
/// more folding options … for example folding itunes folders in", "also
/// sections should be collapsible").
final class LibrarySidebarFoldingTests: XCTestCase {

    private func crate(_ id: Int64, _ name: String, parent: Int64? = nil, tracks: UInt64 = 3)
        -> LibraryImportedCrate
    {
        LibraryImportedCrate(id: id, source: "itunes", name: name, parentId: parent, trackCount: tracks)
    }

    /// An Apple Music library: a folder of albums, a playlist on its own,
    /// and a folder inside the folder.
    private var tree: [LibraryImportedCrate] {
        [
            crate(1, "Albums", tracks: 0),
            crate(2, "Barry White", parent: 1),
            crate(3, "Live", parent: 1, tracks: 0),
            crate(4, "Beach Boys Live", parent: 3),
            crate(5, "Warm up"),
        ]
    }

    /// Folders start folded: only the top of the tree shows.
    func testFoldersStartFolded() {
        let rows = SidebarFolding().visibleNodes(tree, kind: .itunes)
        XCTAssertEqual(rows.map(\.crate.name), ["Albums", "Warm up"])
        XCTAssertEqual(rows.map(\.hasChildren), [true, false])
        XCTAssertEqual(rows.map(\.isOpen), [false, false])
    }

    /// Opening a folder shows its children, indented; a folder inside it
    /// stays folded until it is opened too.
    func testOpeningAFolderShowsItsChildren() {
        var f = SidebarFolding()
        f.toggleFolder(.itunes, path: ["Albums"])
        var rows = f.visibleNodes(tree, kind: .itunes)
        XCTAssertEqual(rows.map(\.crate.name), ["Albums", "Barry White", "Live", "Warm up"])
        XCTAssertEqual(rows.map(\.depth), [0, 1, 1, 0])

        f.toggleFolder(.itunes, path: ["Albums", "Live"])
        rows = f.visibleNodes(tree, kind: .itunes)
        XCTAssertEqual(
            rows.map(\.crate.name), ["Albums", "Barry White", "Live", "Beach Boys Live", "Warm up"])
        XCTAssertEqual(rows[3].path, ["Albums", "Live", "Beach Boys Live"])

        f.toggleFolder(.itunes, path: ["Albums"])
        XCTAssertEqual(f.visibleNodes(tree, kind: .itunes).map(\.crate.name), ["Albums", "Warm up"],
                       "folding the outer folder hides everything under it")
    }

    /// A folder holds playlists, not tracks: its counter counts the
    /// playlists under it, nested folders' included (Daniel, 2026-11-16).
    /// A playlist — and a crate that holds tracks beside sub-crates —
    /// counts tracks.
    func testAFolderCountsItsPlaylists() {
        var f = SidebarFolding()
        f.toggleFolder(.itunes, path: ["Albums"])
        let rows = f.visibleNodes(tree, kind: .itunes)
        let albums = rows[0], barry = rows[1], live = rows[2]
        XCTAssertEqual(albums.count.value, 2, "Barry White, and Beach Boys Live inside Live")
        XCTAssertTrue(albums.count.isPlaylists)
        XCTAssertEqual(live.count.value, 1)
        XCTAssertEqual(barry.count.value, 3)
        XCTAssertFalse(barry.count.isPlaylists)

        let serato = [crate(1, "Crate", tracks: 12), crate(2, "Sub", parent: 1)]
        let parent = SidebarFolding().visibleNodes(serato, kind: .serato)[0]
        XCTAssertEqual(parent.count.value, 12)
        XCTAssertFalse(parent.count.isPlaylists)
    }

    /// A rescan re-imports the tree with new row ids; the folds are kept
    /// by name, so they survive it.
    func testAnOpenFolderSurvivesARescan() {
        var f = SidebarFolding()
        f.toggleFolder(.itunes, path: ["Albums"])
        let rescanned = tree.map { c in
            LibraryImportedCrate(
                id: c.id + 100, source: c.source, name: c.name,
                parentId: c.parentId.map { $0 + 100 }, trackCount: c.trackCount)
        }
        XCTAssertEqual(f.visibleNodes(rescanned, kind: .itunes).count, 4)
    }

    /// The same folder name in two sources is two folders.
    func testFoldsBelongToTheirSource() {
        var f = SidebarFolding()
        f.toggleFolder(.itunes, path: ["Albums"])
        XCTAssertTrue(f.isFolderOpen(.itunes, path: ["Albums"]))
        XCTAssertFalse(f.isFolderOpen(.traktor, path: ["Albums"]))
    }

    /// Sections and sources are open until folded, and every fold
    /// round-trips through the stored string.
    func testSectionsAndSourcesFoldAndPersist() {
        var f = SidebarFolding()
        XCTAssertFalse(f.isSectionFolded("Smart Crates"))
        XCTAssertFalse(f.isSourceFolded(.serato))
        f.toggleSection("Smart Crates")
        f.toggleSource(.serato)
        f.toggleFolder(.itunes, path: ["Albums", "Live"])

        let restored = SidebarFolding(raw: f.raw)
        XCTAssertEqual(restored, f)
        XCTAssertTrue(restored.isSectionFolded("Smart Crates"))
        XCTAssertTrue(restored.isSectionFolded("SMART CRATES"), "the heading's case does not matter")
        XCTAssertTrue(restored.isSourceFolded(.serato))
        XCTAssertTrue(restored.isFolderOpen(.itunes, path: ["Albums", "Live"]))

        f.toggleSection("Smart Crates")
        XCTAssertFalse(f.isSectionFolded("Smart Crates"))
        XCTAssertEqual(SidebarFolding(raw: "").raw, "")
    }
}
