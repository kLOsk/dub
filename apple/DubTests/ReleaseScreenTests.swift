//
//  ReleaseScreenTests.swift
//  DubTests
//
//  The Performance surface at the screen sizes 1.0 is released for
//  (2026-11-15): the smallest common Mac, the most common one, and a
//  large one. Every Mac a DJ brings sits between the first and last.
//
//  Two kinds of check per size:
//
//  * **Fit** — measured, no pixels. The deck columns at the width the
//    real `PerformanceView.perfColumnWidth` gives them, the rack bar
//    under them, and what is left for the library, in track rows.
//  * **Snapshot** — the deck row and the rack bar drawn at that width,
//    so a change that crowds the small screen shows up as a picture
//    diff before it is pushed.
//
//  The whole window is not snapshotted: `PerformanceView` runs on the
//  live app model, which reads the user's settings, library and audio
//  engine. The deck row here is the real `DeckColumn`s and
//  `GlobalRackBar` around plain lanes where the Metal strips go —
//  Metal does not snapshot.
//

import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

final class ReleaseScreenTests: XCTestCase {

    /// A full-screen Performance window, in points.
    struct Screen {
        let name: String
        let width: CGFloat
        let height: CGFloat
        /// Track rows the library must show with the FILTER bar folded —
        /// what each size shows as of 2026-11-15, so a change that takes
        /// rows away fails here. 1280 × 800 is at the library's 200 pt
        /// floor already: anything taller in the deck squeezes the deck.
        let minRows: Int
        /// …and with FILTER open, which folds the rack bar.
        let minRowsFiltering: Int
    }

    /// 13″ on "Larger Text" — a DJ reading a dark stage; Intel 13″ and
    /// M1 Air at default (the commonest DJ laptop); 16″ M-series full
    /// screen, below the notch.
    static let screens = [
        Screen(name: "1280x800", width: 1280, height: 800, minRows: 2, minRowsFiltering: 2),
        Screen(name: "1440x900", width: 1440, height: 900, minRows: 6, minRowsFiltering: 3),
        Screen(name: "1728x1080", width: 1728, height: 1080, minRows: 12, minRowsFiltering: 9),
    ]

    /// The library's own chrome with the FILTER bar folded — measured
    /// off the running app, since `LibraryView` needs the live model.
    static let libraryChrome = DubLayout.libraryChromeHeight
    /// What opening the FILTER bar adds in a library `height` tall: the
    /// boxes, at the height the app gives them there, and their padding.
    static func filterBoxes(libraryHeight height: CGFloat) -> CGFloat {
        LibraryFilterBoxHeight.height(libraryHeight: height) + LibraryFilterBoxHeight.padding
    }

    /// Stands in for the track overview at the height the app draws it.
    /// A bare `Color.clear` is flexible and measures 0 pt tall, which
    /// under-measured the column by the overview's height (2026-11-15).
    static var overview: some View {
        DubColor.waveformLane.frame(height: DubLayout.deckColumnOverviewHeight)
    }

    private func fittingSize(_ view: some View, width: CGFloat) -> CGSize {
        let host = NSHostingView(rootView: view.frame(width: width))
        host.layoutSubtreeIfNeeded()
        return host.fittingSize
    }

    private func columnWidth(_ screen: Screen) -> CGFloat {
        PerformanceView.perfColumnWidth(paneWidth: screen.width)
    }

    /// What each Metal strip is left with beside the two columns and the
    /// gutter — capped, and squeezed once the columns reach their floor
    /// (at 1280 the columns sit on it and the strips give way).
    private func laneWidth(_ screen: Screen) -> CGFloat {
        let free = screen.width - columnWidth(screen) * 2
            - DubLayout.phaseMeterGutterWidth - 4
        return min(DubLayout.performanceWaveformWidthCap, (free / 2).rounded(.down))
    }

    /// The taller of the two columns, as `DeckColumnHeightKey` takes it.
    private func columnHeight(_ screen: Screen) -> CGFloat {
        [DeckSide.a, .b].map { side in
            fittingSize(
                DeckColumn(state: .loadedFixture(side: side)) { Self.overview },
                width: columnWidth(screen)
            ).height
        }.max() ?? 0
    }

    /// Height the library gets: the window less the status strip, the
    /// deck region and the rack bar, each with its divider — what
    /// `DeckLibrarySplit` hands over.
    private func libraryHeight(_ screen: Screen, rackFolded: Bool = false) -> CGFloat {
        let total = screen.height - DubLayout.statusStripHeight - 1
        let rack = rackFolded ? DubLayout.rackBarFoldedHeight : DubLayout.rackBarHeight
        let deck = DeckLibrarySplit<EmptyView, EmptyView>.deckHeight(
            mode: .timecode, total: total,
            deckChrome: rack + 1,
            deckMinimum: columnHeight(screen))
        return total - deck
    }

    private func rows(_ space: CGFloat) -> Int {
        max(0, Int((space / LibraryRowLayout.estimatedHeight).rounded(.down)))
    }

    // MARK: - Fit

    /// The deck columns are never squeezed: the library's floor is met
    /// without the deck region giving way.
    func test_theDeckColumnsFitEveryReleaseScreen() {
        for screen in Self.screens {
            let total = screen.height - DubLayout.statusStripHeight - 1
            let wanted = columnHeight(screen) + DubLayout.rackBarHeight + 1
            XCTAssertLessThanOrEqual(
                wanted + DubLayout.libraryMinHeight, total + 0.5,
                "\(screen.name): the deck needs \(Int(wanted)) pt of \(Int(total))")
        }
    }

    /// The strips keep their floor: at the narrow end they are what
    /// gives way to the columns, and below the floor a scratch
    /// waveform stops being readable.
    func test_theWaveformStripsKeepTheirFloor() {
        for screen in Self.screens {
            XCTAssertGreaterThanOrEqual(
                laneWidth(screen), DubLayout.performanceWaveformMinWidth,
                "\(screen.name): each strip is \(laneWidth(screen)) pt")
        }
    }

    /// The rack bar fits each screen's width.
    func test_theRackBarFitsEveryReleaseScreen() {
        for screen in Self.screens {
            let size = fittingSize(GlobalRackBar(state: .fixture(focus: .a)), width: screen.width)
            XCTAssertLessThanOrEqual(size.width, screen.width + 0.5, screen.name)
            XCTAssertLessThanOrEqual(size.height, DubLayout.rackBarHeight + 0.5, screen.name)
        }
    }

    /// The library shows enough of the crate to pick the next record.
    func test_theLibraryShowsEnoughTracks() {
        for screen in Self.screens {
            let shown = rows(libraryHeight(screen) - Self.libraryChrome)
            XCTAssertGreaterThanOrEqual(
                shown, screen.minRows,
                "\(screen.name): the library shows \(shown) track rows with the filter folded")
        }
    }

    /// With FILTER open the rack bar folds, and the rows that buys are
    /// what this holds. Before the fold, 1280 and 1440 both showed none.
    /// On 1280 × 800 the fold alone was not enough (~312 pt of library
    /// against ~316 of chrome and boxes); the boxes give way there.
    func test_anOpenFilterStillShowsTracks() {
        for screen in Self.screens {
            let library = libraryHeight(screen, rackFolded: true)
            let space = library - Self.libraryChrome - Self.filterBoxes(libraryHeight: library)
            XCTAssertGreaterThanOrEqual(
                rows(space), screen.minRowsFiltering,
                "\(screen.name): \(rows(space)) track rows with the filter open")
        }
    }

    /// The fold rule: an open filter folds the bar; the DJ's own fold
    /// always holds; opening it over the filter wins.
    func test_anOpenFilterFoldsTheRackBar() {
        let fold = PerformanceView.rackIsFolded
        XCTAssertFalse(fold(false, false, false), "filter closed: as the DJ left it")
        XCTAssertTrue(fold(true, false, false), "the DJ's own fold")
        XCTAssertTrue(fold(false, true, false), "filter open folds it")
        XCTAssertFalse(fold(false, true, true), "opened over the filter")
        XCTAssertTrue(fold(true, true, true), "the DJ's own fold still holds")
    }

    /// The boxes keep their full height where there is room and never
    /// drop under their floor.
    func test_filterBoxesGiveWayOnlyOnAShortLibrary() {
        XCTAssertEqual(LibraryFilterBoxHeight.height(libraryHeight: 800), DubLayout.libraryFilterBoxHeight)
        XCTAssertEqual(LibraryFilterBoxHeight.height(libraryHeight: nil), DubLayout.libraryFilterBoxHeight)
        XCTAssertEqual(LibraryFilterBoxHeight.height(libraryHeight: 100), DubLayout.libraryFilterBoxMinHeight)
        let mid = LibraryFilterBoxHeight.height(libraryHeight: 312)
        XCTAssertLessThan(mid, DubLayout.libraryFilterBoxHeight)
        XCTAssertGreaterThanOrEqual(mid, DubLayout.libraryFilterBoxMinHeight)
    }

    /// Browse mode raises the library to just under the lower of the
    /// two overviews, never lowers it below its slot, and leaves it be
    /// when it is not raised or there is no overview to stop under.
    func test_browseModeRaisesTheLibraryToTheOverviews() {
        let top = PerformanceView.libraryTop
        XCTAssertEqual(top(500, [180, 184], false), 500)
        XCTAssertEqual(top(500, [180, 184], true), 184 + DubSpacing.sm)
        XCTAssertEqual(top(500, [], true), 500)
        XCTAssertEqual(top(150, [180], true), 150, "never below its own slot")
    }

    /// The track count on the FILTER line.
    func test_theStatusCountsTracks() {
        XCTAssertEqual(LibraryStatusCount.text(listed: 108, shown: 108, filtered: false), "108 tracks")
        XCTAssertEqual(LibraryStatusCount.text(listed: 1, shown: 1, filtered: false), "1 track")
        XCTAssertEqual(LibraryStatusCount.text(listed: 108, shown: 12, filtered: true), "12 of 108")
    }

    // MARK: - Snapshots

    func test_deckRowAndRackBar_onEveryReleaseScreen() {
        for screen in Self.screens {
            let column = columnWidth(screen)
            let height = columnHeight(screen)
            let view = VStack(spacing: 1) {
                HStack(spacing: 1) {
                    DeckColumn(state: .loadedFixture(side: .a)) { Self.overview }
                        .frame(width: column)
                    DubColor.waveformLane.frame(width: laneWidth(screen))
                    DubColor.surface0.frame(width: DubLayout.phaseMeterGutterWidth)
                    DubColor.waveformLane.frame(width: laneWidth(screen))
                    DeckColumn(state: .loadedFixture(side: .b)) { Self.overview }
                        .frame(width: column)
                }
                .frame(height: height)
                GlobalRackBar(state: .fixture(focus: .a))
                    .frame(height: DubLayout.rackBarHeight)
            }
            .background(DubColor.divider)
            let size = CGSize(width: screen.width, height: height + 1 + DubLayout.rackBarHeight)
            let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
            host.frame = CGRect(origin: .zero, size: size)
            host.layoutSubtreeIfNeeded()
            assertSnapshot(
                of: host, as: .image(perceptualPrecision: 0.98), named: screen.name)
        }
    }
}
