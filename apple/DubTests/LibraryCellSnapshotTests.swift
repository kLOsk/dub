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
        mode: KeyNotationMode = .camelot,
        sessionFrom: String? = nil
    ) -> LibraryColumnCell {
        LibraryColumnCell(
            field: field,
            state: LibraryCellState(
                track: track,
                sessionFromTitle: sessionFrom,
                keyNotationMode: mode))
    }

    // MARK: - Text columns

    func test_textCells_populated() {
        let track = LibraryTrack.fixture()
        let row = HStack(spacing: 0) {
            cell(.artist, track).frame(width: 120, alignment: .leading)
            cell(.title, track).frame(width: 180, alignment: .leading)
            cell(.album, track).frame(width: 140, alignment: .leading)
            cell(.genre, track).frame(width: 140, alignment: .leading)
        }
        snap(row, width: 580, named: "text-cells-populated")
    }

    /// Every optional column renders an em-dash, not an empty cell —
    /// "we have no value" has to be visible.
    func test_textCells_empty() {
        let track = LibraryTrack.fixture(
            title: nil, artist: nil, album: nil, genre: nil)
        let row = HStack(spacing: 0) {
            cell(.artist, track).frame(width: 120, alignment: .leading)
            cell(.title, track).frame(width: 180, alignment: .leading)
            cell(.album, track).frame(width: 140, alignment: .leading)
        }
        snap(row, width: 440, named: "text-cells-empty")
    }

    /// Session History rows carry a "← from" sub-label at the lowest
    /// layout priority, so the title never compresses for it.
    func test_titleCell_withSessionHistoryHint() {
        snap(
            cell(.title, .fixture(), sessionFrom: "Bow Down")
                .frame(width: 260, alignment: .leading),
            width: 260, named: "title-session-hint")
    }

    // MARK: - BPM glyph states

    /// The three-way glyph state: a grid disagreement (PRD §8.3) is
    /// independent and can co-render with lock or drift; lock and drift
    /// are mutually exclusive.
    func test_bpmCell_glyphStates() {
        let states: [(String, LibraryTrack)] = [
            ("plain", .fixture(bpm: 174)),
            ("locked", .fixture(bpm: 174, gridLocked: true)),
            ("drifting", .fixture(bpm: 174, gridDriftQuality: 5)),
            ("disagreement", .fixture(bpm: 92, bpmDisagreement: true)),
            ("disagreement-locked",
             .fixture(bpm: 92, gridLocked: true, bpmDisagreement: true)),
            ("none", .fixture(bpm: nil)),
        ]
        let row = HStack(spacing: 0) {
            ForEach(states, id: \.0) { entry in
                self.cell(.bpm, entry.1).frame(width: 90, alignment: .leading)
            }
        }
        snap(row, width: 540, named: "bpm-glyph-states")
    }

    // MARK: - Key notation

    func test_keyCell_bothNotations() {
        let track = LibraryTrack.fixture(key: "8B")
        let row = HStack(spacing: 0) {
            cell(.key, track, mode: .camelot).frame(width: 80, alignment: .leading)
            cell(.key, track, mode: .musical).frame(width: 100, alignment: .leading)
            cell(.key, .fixture(key: nil)).frame(width: 80, alignment: .leading)
        }
        snap(row, width: 260, named: "key-notations")
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

    func test_numericCells() {
        let track = LibraryTrack.fixture(year: 1996, durationMs: 213_000)
        let row = HStack(spacing: 0) {
            cell(.duration, track).frame(width: 60, alignment: .leading)
            cell(.year, track).frame(width: 60, alignment: .leading)
            // `#` is the only built-in that right-aligns.
            cell(.crateOrder, .fixture(crateOrdinal: 11))
                .frame(width: 44, alignment: .leading)
            cell(.duration, .fixture(durationMs: 0)).frame(width: 60, alignment: .leading)
        }
        snap(row, width: 224, named: "numeric-cells")
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
        isAnalyzed: Bool = true
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
            extras: [])
    }
}

/// The assembled row, built the way `LibraryTable` builds it: a
/// `LibraryTintedRowView` holding one `LibraryHostingCellView` per
/// column, gutter first.
///
/// It renders the production classes rather than a stand-in. A
/// stand-in row composed the same cells and could stay green while the
/// real table drifted away from it — which is exactly what happened to
/// the `LibraryRowView` these replaced.
///
/// Three details are pinned here, all of which look incidental:
///
/// * `DimUnanalyzed` is per *cell*, inside the column — an unanalyzed
///   row dims its cell text but not the gutter badges and not the
///   colour tint. Applying it to the row changes them.
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
            let cell = LibraryHostingCellView(
                identifier: NSUserInterfaceItemIdentifier(column.field.rawValue))
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

    /// The dim must reach the cell text but not the gutter badge and not
    /// the colour tint.
    func test_row_unanalyzed_dimsCellsOnly() {
        let track = LibraryTrack.fixture(color: "green", isAnalyzed: false)
        snap(state(track, deckA: true), named: "row-unanalyzed")
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
                LibraryHeaderState(title: "Key (♪)"),
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
