//
//  LibraryCellSnapshotTests.swift
//  DubTests
//
//  The library table's first test coverage.
//
//  Nothing under `apple/DubTests` touched `LibraryView` — no snapshot of
//  a row, a cell or the header. The table is about to move from a
//  hand-rolled `LazyVStack` to `NSTableView` with reused cells, and that
//  port promises identical pixels; these are what it gets judged
//  against, instead of someone's eye.
//
//  Deliberately per *cell*: a cell is a pure function of
//  `LibraryCellState`, so it snapshots without a `WaveformAppModel`, a
//  `LibraryAppModel` or an open library. Anything that still needs
//  those belongs to the row, not the cell.
//

@testable import Dub
import DubCore
import SnapshotTesting
import SwiftUI
import XCTest

final class LibraryCellSnapshotTests: XCTestCase {

    // Anti-aliasing is not reproducible across machines or OS
    // versions — see the note in PerformanceSnapshotTests.
    private func snap(
        _ view: some View,
        width: CGFloat,
        height: CGFloat = 28,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        let sized = view
            .frame(width: width, height: height, alignment: .leading)
            .background(DubColor.surface0)
        let host = NSHostingView(rootView: sized)
        host.frame = CGRect(x: 0, y: 0, width: width, height: height)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: host, as: .image(perceptualPrecision: 0.98), named: name,
            file: file, testName: testName, line: line)
    }

    private func cell(
        _ field: LibraryColumnField,
        _ track: LibraryTrack,
        moving: Bool = false
    ) -> LibraryColumnCell {
        LibraryColumnCell(
            field: field,
            state: LibraryCellState(track: track, movingGridsShown: moving))
    }

    /// The text columns are drawn by AppKit (`LibraryTextCellView`), so
    /// these lay the production cells out by hand, as `LibraryTable`'s
    /// row view does.
    @MainActor
    private func snapText(
        _ cells: [(field: LibraryColumnField, state: LibraryCellState, width: CGFloat)],
        height: CGFloat = 28,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        let row = NSTableRowView()
        row.frame = CGRect(x: 0, y: 0, width: cells.map(\.width).reduce(0, +), height: height)
        row.backgroundColor = NSColor(DubColor.surface0)
        var x: CGFloat = 0
        for entry in cells {
            let cell = LibraryCellView.make(
                column: NSUserInterfaceItemIdentifier(entry.field.rawValue))
            cell.configure(
                field: entry.field, state: LibraryRowState(cell: entry.state),
                actions: LibraryRowActions())
            cell.frame = CGRect(x: x, y: 0, width: entry.width, height: height)
            row.addSubview(cell)
            x += entry.width
        }
        row.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: row, as: .image(perceptualPrecision: 0.98), named: name,
            file: file, testName: testName, line: line)
    }

    private func state(
        _ track: LibraryTrack, mode: KeyNotationMode = .camelot, sessionFrom: String? = nil
    ) -> LibraryCellState {
        LibraryCellState(track: track, sessionFromTitle: sessionFrom, keyNotationMode: mode)
    }

    // MARK: - Text columns

    @MainActor
    func test_textCells_populated() {
        let track = state(.fixture())
        snapText(
            [(.artist, track, 120), (.title, track, 180), (.album, track, 140), (.genre, track, 140)],
            named: "text-cells-populated")
    }

    /// Every optional column renders an em-dash, not an empty cell —
    /// "we have no value" has to be visible.
    @MainActor
    func test_textCells_empty() {
        let track = state(.fixture(title: nil, artist: nil, album: nil, genre: nil))
        snapText(
            [(.artist, track, 120), (.title, track, 180), (.album, track, 140)],
            named: "text-cells-empty")
    }

    /// Session History rows carry a "← from" remark after the title; the
    /// line is cut at its end, so the remark gives way before the title.
    @MainActor
    func test_titleCell_withSessionHistoryHint() {
        snapText(
            [(.title, state(.fixture(), sessionFrom: "Bow Down"), 260)],
            named: "title-session-hint")
    }

    /// What a text column says, apart from how it is drawn.
    func test_textColumns_asValues() {
        let track = LibraryTrack.fixture(key: "8B", durationMs: 213_000, crateOrdinal: 11)
        XCTAssertNil(LibraryCellText.make(.bpm, state(track)))
        XCTAssertNil(LibraryCellText.make(.rating, state(track)))
        XCTAssertNil(LibraryCellText.make(.color, state(track)))

        let title = LibraryCellText.make(.title, state(track))
        XCTAssertEqual(title?.text, "Ain't No Mountain High Enough")
        XCTAssertEqual(title?.ink, .primary)
        XCTAssertEqual(title?.truncation, .byTruncatingMiddle)
        XCTAssertEqual(LibraryCellText.make(.title, state(.fixture(title: nil)))?.text, "Untitled")
        let hinted = LibraryCellText.make(.title, state(track, sessionFrom: "Bow Down"))
        XCTAssertEqual(hinted?.note, "← from Bow Down")
        XCTAssertEqual(hinted?.truncation, .byTruncatingTail)

        XCTAssertEqual(LibraryCellText.make(.artist, state(.fixture(artist: nil)))?.text, "—")
        XCTAssertEqual(LibraryCellText.make(.duration, state(track))?.text, "3:33")
        XCTAssertEqual(LibraryCellText.make(.duration, state(track))?.digits, true)

        let order = LibraryCellText.make(.crateOrder, state(track))
        XCTAssertEqual(order?.text, "12")
        XCTAssertEqual(order?.trailing, true)
        XCTAssertEqual(order?.ink, .tertiary)

        let key = LibraryCellText.make(.key, state(track, mode: .musical))
        XCTAssertEqual(key?.ink, .key("8B"))
        XCTAssertEqual(key?.toolTip, "8B")
        XCTAssertEqual(LibraryCellText.make(.key, state(.fixture(key: nil)))?.ink, .secondary)
    }

    // MARK: - BPM glyph states

    /// The glyph states. One warning mark at most — a disagreement and a
    /// drift together draw a single amber ⚠, not two (Daniel,
    /// 2026-11-16) — and the padlock beside it when the grid is locked.
    func test_bpmCell_glyphStates() {
        let states: [(String, LibraryTrack)] = [
            ("plain", .fixture(bpm: 174)),
            ("locked", .fixture(bpm: 174, gridLocked: true)),
            ("drifting", .fixture(bpm: 174, gridDriftQuality: 12)),
            ("disagreement", .fixture(bpm: 92, bpmDisagreement: true)),
            ("disagreement-locked",
             .fixture(bpm: 92, gridLocked: true, bpmDisagreement: true)),
            ("disagreement-drifting",
             .fixture(bpm: 92, gridDriftQuality: 12, bpmDisagreement: true)),
            ("none", .fixture(bpm: nil)),
        ]
        let row = HStack(spacing: 0) {
            ForEach(states, id: \.0) { entry in
                self.cell(.bpm, entry.1).frame(width: 90, alignment: .leading)
            }
        }
        snap(row, width: 630, named: "bpm-glyph-states")
    }

    /// A moving grid (the following grid, saved — schema v11): its BPM in
    /// `DubColor.gridMoving`; locked, the padlock too — moving, accepted.
    /// Drift is not warned of: the grid follows the band. With the
    /// following grid off, the same tracks read as even ones.
    func test_bpmCell_movingGrid() {
        let follows = LibraryTrack.fixture(
            bpm: 92, gridDriftQuality: 12, tempoMapKind: "followed",
            tempoBpmMin: 89.2, tempoBpmMax: 95.8)
        let accepted = LibraryTrack.fixture(
            bpm: 92, gridLocked: true, tempoMapKind: "followed",
            tempoBpmMin: 89.2, tempoBpmMax: 95.8)
        let row = HStack(spacing: 0) {
            self.cell(.bpm, follows, moving: true).frame(width: 90, alignment: .leading)
            self.cell(.bpm, accepted, moving: true).frame(width: 90, alignment: .leading)
            self.cell(.bpm, follows, moving: false).frame(width: 90, alignment: .leading)
            self.cell(.bpm, accepted, moving: false).frame(width: 90, alignment: .leading)
        }
        snap(row, width: 360, named: "bpm-moving-grid")
    }

    func test_movingGridHelp() {
        XCTAssertEqual(
            LibraryCellFormat.movingGridHelp(min: 89.2, max: 95.8, locked: false),
            "Follows the band · 89–96 BPM · lock to accept it")
        XCTAssertEqual(
            LibraryCellFormat.movingGridHelp(min: 92.1, max: 91.9, locked: true),
            "Follows the band · 92 BPM · locked: plays as saved")
        XCTAssertEqual(
            LibraryCellFormat.movingGridHelp(min: nil, max: nil, locked: false),
            "Follows the band · lock to accept it")
    }

    /// Worst first: a disagreement outranks drift, and a locked grid never
    /// warns of drift — it has been accepted.
    func test_bpmIssues_rankAndLock() {
        XCTAssertEqual(
            LibraryCellFormat.bpmIssues(disagreement: true, locked: false, drift: 12),
            [.disagreement, .drift])
        XCTAssertEqual(
            LibraryCellFormat.bpmIssues(disagreement: false, locked: false, drift: -9), [.drift])
        XCTAssertEqual(
            LibraryCellFormat.bpmIssues(disagreement: true, locked: true, drift: 12),
            [.disagreement])
        // Hey Ya read 5.6 and Daniel found its grid perfect: under the
        // limit, no mark.
        XCTAssertEqual(
            LibraryCellFormat.bpmIssues(disagreement: false, locked: false, drift: 5.6), [])
        XCTAssertEqual(
            LibraryCellFormat.bpmIssues(disagreement: false, locked: false, drift: 8), [.drift])
        XCTAssertEqual(
            LibraryCellFormat.bpmIssues(disagreement: false, locked: false, drift: nil), [])
    }

    // MARK: - Key notation

    @MainActor
    func test_keyCell_bothNotations() {
        let track = LibraryTrack.fixture(key: "8B")
        snapText(
            [
                (.key, state(track, mode: .camelot), 80),
                (.key, state(track, mode: .musical), 100),
                (.key, state(.fixture(key: nil)), 80),
            ],
            named: "key-notations")
    }

    // MARK: - Rating and colour

    /// Empty stars always draw, so the cell is a constant five glyphs
    /// wide whatever the rating.
    func test_ratingCell_allValues() {
        let row = HStack(spacing: 0) {
            ForEach(0..<6, id: \.self) { stars in
                self.cell(.rating, .fixture(rating: stars == 0 ? nil : Int32(stars)))
                    .frame(width: 92, alignment: .leading)
            }
        }
        snap(row, width: 552, named: "rating-values")
    }

    func test_colorCell_setAndUnset() {
        let row = HStack(spacing: 0) {
            LibraryColorCell(color: nil, onPick: { _ in })
                .frame(width: 44, alignment: .leading)
            ForEach(["red", "green", "blue", "purple"], id: \.self) { token in
                LibraryColorCell(color: token, onPick: { _ in })
                    .frame(width: 44, alignment: .leading)
            }
        }
        snap(row, width: 220, named: "color-swatches")
    }

    // MARK: - Numeric columns

    @MainActor
    func test_numericCells() {
        let track = state(.fixture(year: 1996, durationMs: 213_000))
        snapText(
            [
                (.duration, track, 60), (.year, track, 60),
                // `#` is the only built-in that right-aligns.
                (.crateOrder, state(.fixture(crateOrdinal: 11)), 44),
                (.duration, state(.fixture(durationMs: 0)), 60),
            ],
            named: "numeric-cells")
    }
}

extension LibraryTrack {
    /// A populated track for cell snapshots. Named arguments cover the
    /// fields the cells actually branch on; everything else is filler.
    static func fixture(
        id: String = "t1",
        title: String? = "Ain't No Mountain High Enough",
        artist: String? = "Marvin Gaye & Tammi Terrell",
        album: String? = "United",
        genre: String? = "Soul",
        year: Int32? = 1967,
        bpm: Double? = 128,
        key: String? = "8B",
        durationMs: UInt32 = 149_000,
        rating: Int32? = 3,
        color: String? = nil,
        crateOrdinal: UInt32? = nil,
        gridLocked: Bool = false,
        gridDriftQuality: Float? = nil,
        bpmDisagreement: Bool = false,
        isAnalyzed: Bool = true,
        tempoMapKind: String? = nil,
        tempoBpmMin: Double? = nil,
        tempoBpmMax: Double? = nil
    ) -> LibraryTrack {
        LibraryTrack(
            id: id,
            title: title,
            artist: artist,
            album: album,
            genre: genre,
            year: year,
            bpm: bpm,
            key: key,
            durationMs: durationMs,
            versionTokens: nil,
            potentialDuplicateId: nil,
            source: "library",
            primaryVolumeUuid: nil,
            primaryVolumeMountPoint: "/",
            primaryRelativePath: "Music/track.mp3",
            isAnalyzed: isAnalyzed,
            keyDisagreement: false,
            comment: nil,
            composer: nil,
            trackNumber: nil,
            gridLocked: gridLocked,
            gridDriftQuality: gridDriftQuality,
            rating: rating,
            color: color,
            crateOrdinal: crateOrdinal,
            bpmDisagreement: bpmDisagreement,
            tempoMapKind: tempoMapKind,
            tempoBpmMin: tempoBpmMin,
            tempoBpmMax: tempoBpmMax,
            extras: [])
    }
}

/// The assembled row, built the way `LibraryTable` builds it: a
/// `LibraryTintedRowView` holding one `LibraryCellView` per column,
/// gutter first.
///
/// It renders the production classes rather than a stand-in. A
/// stand-in row composed the same cells and could stay green while the
/// real table drifted away from it — which is exactly what happened to
/// the `LibraryRowView` these replaced.
///
/// Two details are pinned here, both of which look incidental:
///
/// * The row tint is faint on purpose, because it sits *above* the
///   selection fill.
/// * That ordering is what keeps a selected coloured row legible, and
///   it is drawn in one override, so only a selected-and-coloured row
///   catches a regression in it.
@MainActor
final class LibraryRowSnapshotTests: XCTestCase {

    /// The table has no outer horizontal padding — the gutter column
    /// starts at x = 0 and `intercellSpacing` is zero.
    private static let columns: [(field: LibraryColumnField, width: CGFloat)] = [
        (.artist, 120), (.title, 180), (.duration, 52),
        (.bpm, 56), (.rating, 92), (.color, 44),
    ]

    private static var totalWidth: CGFloat {
        LibraryGutterColumn.width + columns.map(\.width).reduce(0, +)
    }

    /// Assembles the real row. Cell frames are laid out by hand because
    /// `NSTableView` is what normally does it, and hosting a whole
    /// table would drag a scroll view and a data source into a test
    /// about how one row draws.
    private func rowView(_ state: LibraryRowState, selected: Bool) -> NSTableRowView {
        let height = LibraryRowLayout.estimatedHeight
        let row = LibraryTintedRowView()
        row.frame = CGRect(x: 0, y: 0, width: Self.totalWidth, height: height)
        row.backgroundColor = NSColor(DubColor.surface0)
        row.tint = DubColor.trackLabel(state.track.color)
            .map { NSColor($0).withAlphaComponent(0.18) }
        row.isSelected = selected

        let actions = LibraryRowActions()
        var x: CGFloat = 0
        let gutter = LibraryHostingCellView(identifier: LibraryGutterColumn.identifier)
        gutter.configureGutter(state: state, actions: actions)
        gutter.frame = CGRect(x: x, y: 0, width: LibraryGutterColumn.width, height: height)
        row.addSubview(gutter)
        x += LibraryGutterColumn.width

        for column in Self.columns {
            let cell = LibraryCellView.make(
                column: NSUserInterfaceItemIdentifier(column.field.rawValue))
            cell.configure(field: column.field, state: state, actions: actions)
            cell.frame = CGRect(x: x, y: 0, width: column.width, height: height)
            row.addSubview(cell)
            x += column.width
        }
        row.layoutSubtreeIfNeeded()
        return row
    }

    private func snap(
        _ state: LibraryRowState,
        selected: Bool = false,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        assertSnapshot(
            of: rowView(state, selected: selected),
            as: .image(perceptualPrecision: 0.98), named: name,
            file: file, testName: testName, line: line)
    }

    private func state(
        _ track: LibraryTrack = .fixture(),
        deckA: Bool = false,
        deckB: Bool = false,
        unreachable: Bool = false
    ) -> LibraryRowState {
        LibraryRowState(
            cell: LibraryCellState(track: track),
            isOnDeckA: deckA,
            isOnDeckB: deckB,
            showsUnreachableWarning: unreachable,
            unreachableTooltip: "Source volume is offline — plug it back in or use Relocate.")
    }

    func test_row_plain() {
        snap(state(), named: "row-plain")
    }

    /// Both badges can show at once — Instant Doubles puts one track on
    /// both decks.
    func test_row_onBothDecks() {
        snap(state(deckA: true, deckB: true), named: "row-both-decks")
    }

    func test_row_unreachableAndDuplicate() {
        var track = LibraryTrack.fixture()
        track.potentialDuplicateId = "t2"
        snap(state(track, unreachable: true), named: "row-unreachable-duplicate")
    }

    func test_row_colourTinted() {
        snap(state(.fixture(color: "red")), named: "row-colour-tinted")
    }

    /// Selection is a flat `surface2` fill, not the system accent.
    func test_row_selected() {
        snap(state(), selected: true, named: "row-selected")
    }

    /// The one that catches the ordering: tint over fill, not under.
    /// Reversed, the 0.18 tint disappears beneath the selection and a
    /// selected coloured row loses its label.
    func test_row_selectedAndColourTinted() {
        snap(state(.fixture(color: "red")), selected: true, named: "row-selected-colour-tinted")
    }
}

/// Column headers. Sizes mirror the header's real insets: leading 8 and
/// trailing 14, so usable width is `w − 22` — 14 pt narrower than the
/// cell beneath, which is deliberate and would be easy to lose in the
/// `NSTableView` port.
final class LibraryHeaderSnapshotTests: XCTestCase {

    private func snap(
        _ states: [LibraryHeaderState],
        width: CGFloat,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        let row = HStack(spacing: 0) {
            ForEach(Array(states.enumerated()), id: \.offset) { _, state in
                LibraryHeaderCell(state: state)
                    .padding(.leading, LibraryColumnLayout.columnLeadingInset)
                    .frame(
                        width: max(0, 120 - LibraryColumnLayout.columnTrailingInset),
                        alignment: .leading)
                    .padding(.trailing, LibraryColumnLayout.columnTrailingInset)
                    .frame(width: 120, alignment: .leading)
            }
        }
        .frame(height: LibraryRowLayout.headerHeight)
        .background(DubColor.surface1)
        let host = NSHostingView(rootView: row)
        host.frame = CGRect(
            x: 0, y: 0, width: width, height: LibraryRowLayout.headerHeight)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: host, as: .image(perceptualPrecision: 0.98), named: name,
            file: file, testName: testName, line: line)
    }

    func test_header_sortStates() {
        snap(
            [
                LibraryHeaderState(title: "Artist"),
                LibraryHeaderState(title: "Title", isActive: true, ascending: true),
                LibraryHeaderState(title: "BPM", isActive: true, ascending: false),
                LibraryHeaderState(title: "Key · Musical"),
            ],
            width: 480, named: "header-sort-states")
    }

    /// A label wider than its column truncates; the trailing inset is
    /// what keeps it off the divider.
    func test_header_truncation() {
        snap(
            [LibraryHeaderState(title: "Serato Beatgrid Offset", isActive: true)],
            width: 120, named: "header-truncation")
    }
}
