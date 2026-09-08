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

/// The assembled row: gutter, cells, colour tint.
///
/// These pin the two details most likely to drift in the `NSTableView`
/// port, both of which look incidental:
///
/// * `DimUnanalyzed` is per *cell*, inside the width frame — an
///   unanalyzed row dims its cell text but not the gutter badges and
///   not the colour tint. Applying it to a row view changes them.
/// * The row tint is faint on purpose. Selection paints in AppKit
///   *beneath* the row, so a heavier tint makes a selected coloured
///   row unreadable.
final class LibraryRowSnapshotTests: XCTestCase {

    private static let columns: [LibraryRowColumn] = [
        LibraryRowColumn(field: .artist, width: 120),
        LibraryRowColumn(field: .title, width: 180),
        LibraryRowColumn(field: .duration, width: 52),
        LibraryRowColumn(field: .bpm, width: 56),
        LibraryRowColumn(field: .rating, width: 92),
        LibraryRowColumn(field: .color, width: 44),
    ]

    private static var totalWidth: CGFloat {
        LibraryColumnLayout.gutterWidth
            + columns.map(\.width).reduce(0, +)
            + DubSpacing.lg * 2
    }

    private func snap(
        _ state: LibraryRowState,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        let row = LibraryRowView(
            state: state,
            columns: Self.columns,
            totalWidth: Self.totalWidth)
            .background(DubColor.surface0)
        let host = NSHostingView(rootView: row)
        host.frame = CGRect(
            x: 0, y: 0,
            width: Self.totalWidth, height: LibraryRowLayout.estimatedHeight)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: host, as: .image(perceptualPrecision: 0.98), named: name,
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
}
