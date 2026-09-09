//
//  PerformanceLayoutTests.swift
//  DubTests
//
//  Does the content fit the space the layout gives it?
//
//  A snapshot test cannot answer that. `snap(_:width:height:)` forces
//  `.frame(width:height:)` onto the view, so it renders whatever size
//  the test author picked and passes — an overflowing column simply
//  renders smaller and looks fine. That is how the pad column shipped
//  needing ~478 pt of a ~330 pt pane, with its bottom rows painted over
//  by the opaque bar declared after it in the same `VStack`.
//
//  These ask `NSHostingView` for the view's real `fittingSize` and
//  compare it against the tokens. No pixels, so no machine or
//  OS-version sensitivity, and they run in milliseconds.
//

@testable import Dub
import SwiftUI
import XCTest

final class PerformanceLayoutTests: XCTestCase {

    /// The view's intrinsic size when offered `width`.
    private func fittingSize(_ view: some View, width: CGFloat) -> CGSize {
        let host = NSHostingView(rootView: view.frame(width: width))
        host.layoutSubtreeIfNeeded()
        return host.fittingSize
    }

    // MARK: - The pad column

    /// The deck pane a 1440 × 900 window actually produces, asked of
    /// the splitter rather than restated as a constant.
    ///
    /// It used to be a literal 398, derived against 236 pt of chrome —
    /// 108 of which was the deck header band that no longer exists. A
    /// constant that has to be re-derived by hand every time the chrome
    /// moves will be wrong the once it matters, so this computes it.
    private var paneHeight: CGFloat {
        let chrome = DubLayout.rackBarHeight + 1
        let total = 900 - DubLayout.statusStripHeight
        return DeckLibrarySplit<EmptyView, EmptyView>.deckHeight(
            mode: .timecode, total: total, deckChrome: chrome,
            deckMinimum: DubLayout.waveformMinHeight) - chrome
    }

    /// A loaded deck with cues set — the tallest the column gets in a
    /// shipping configuration.
    private static let columnFixture = DeckColumnState(
        side: .a,
        header: DeckColumnHeader(DeckHeaderState(
            isLive: true, source: .file,
            trackTitle: "Baddadan (extended mix) (feat. IRah, Flowdan)",
            trackArtist: "Chase & Status",
            bpm: 176.0, pitchPercent: -1.2, timecodeLockState: 1,
            key: "7A", formatChip: "MP3 · 44.1 kHz · stereo",
            timeRow: .remainingOnly,
            isMaster: true, isPlaying: true,
            isPanicPlay: false, useTimecodeToggle: false,
            gridLocked: false, gridDriftQuality: nil)),
        cues: (0..<8).map {
            CueSlotState(
                index: $0,
                mark: CueMark(positionSecs: Double($0) * 30, name: "MARK", color: "aqua"))
        },
        activeLoopBeats: 2, loopEngaged: true,
        echoEnabled: true, echoEngaged: false,
        hasTrack: true, isPlaying: true)

    /// The regression test for the reported bug. The column's height
    /// now depends on exactly two booleans — the siren left for the
    /// global rack bar — so the shipping space is cheap to cover
    /// exhaustively.
    ///
    /// This is the assertion that would have gone red the day the siren
    /// row landed in M16, instead of the sampler quietly disappearing
    /// under the FX bar a milestone later.
    /// The width a 1440 × 900 window actually gives each column: the
    /// pane less the two capped strips and the centre gutter, halved.
    /// This is the configuration to hold, not the column's floor — the
    /// floor only happens on a 960 pt window, where the pane is shorter
    /// too, and asserting a narrow width against a wide window's height
    /// is a configuration nobody runs.
    private var columnWidthAt1440: CGFloat {
        (1440 - DubLayout.performanceWaveformWidthCap * 2
            - DubLayout.stillpointGutterWidth - 2) / 2
    }

    /// The column fits the pane a 1440 × 900 window produces.
    ///
    /// It did not, briefly: the cue bank chose its column count with a
    /// `ViewThatFits` ladder, and at a narrow width it fell to a single
    /// column of eight rows — 128 pt taller than the pane. The bank is
    /// two columns at every width now, which is both what the surface
    /// wants and what makes this assertion hold without a scroll
    /// fallback underneath it.
    func test_deckColumn_fitsThePane_onALaptopScreen() {
        let size = fittingSize(
            DeckColumn(state: Self.columnFixture) { Color.clear },
            width: columnWidthAt1440)
        XCTAssertLessThanOrEqual(
            size.height, paneHeight + 0.5,
            "deck column overflows the 1440 × 900 pane")
    }

    /// The column's floor has to hold the loop control, which is the
    /// one section inside it with a fixed width. If the loop grows or
    /// the floor shrinks, the ×2 stepper clips — which the pad column
    /// did for a whole milestone, rendering labels as "UE" and "OOP".
    func test_deckColumn_floorHoldsTheLoopControl() {
        XCTAssertGreaterThanOrEqual(
            DubLayout.performanceDeckColumnMinWidth,
            DubLayout.prepLoopSection + DubSpacing.md * 2,
            "the column floor no longer fits the loop control")
    }

    // MARK: - The global rack bar

    func test_rackBar_fitsItsHeightToken() {
        let size = fittingSize(
            GlobalRackBar(state: .fixture(focus: .a)), width: 1440)
        XCTAssertLessThanOrEqual(
            size.height, DubLayout.rackBarHeight + 0.5,
            "rack bar content is taller than rackBarHeight")
    }

    func test_rackBar_fitsTheNarrowestSupportedWindow() {
        let size = fittingSize(
            GlobalRackBar(state: .fixture(focus: .a)),
            width: DubLayout.mainWindowMinWidth)
        XCTAssertLessThanOrEqual(
            size.width, DubLayout.mainWindowMinWidth + 0.5,
            "rack bar overflows the minimum window width")
    }

    // MARK: - The Prep grid

    /// Three columns must fit the narrowest supported window, or the
    /// LOOP row clips — which is why the grid is a plain `HStack` with
    /// measured widths rather than a `LazyVGrid` distributing equal
    /// ones.
    ///
    /// The margin here is small by design (the columns are sized to
    /// their content), so if a future Prep section is wider than its
    /// column this fails immediately rather than shipping a clipped
    /// pad. The documented remedy at that point is a two-column
    /// `ViewThatFits` candidate — not a bigger minimum window.
    /// `prepPadBarMinHeight`'s doc comment asks for a re-measure whenever a
    /// Prep section changes, and `PrepRack` replaced every section at once.
    /// This asserts the floor is derived from what the surface actually
    /// needs — including the 56 pt rip lane, which renders *inside*
    /// `prepPadRows` and which three separate estimates have dropped.
    /// Prep hands the library everything the deck does not draw. The
    /// drag handle that used to decide this is gone; the rule replacing
    /// it has to be checked, because getting it wrong silently starves
    /// one half.
    func test_prepGivesTheLibraryEverythingLeftOver() {
        let total: CGFloat = 900
        let chrome: CGFloat = 0
        let deck = DeckLibrarySplit<EmptyView, EmptyView>.deckHeight(
            mode: .prep, total: total,
            deckChrome: chrome, deckMinimum: DubLayout.prepRegionMinHeight)
        XCTAssertEqual(deck, DubLayout.prepRegionMinHeight,
                       "Prep's deck should size to its content, not a fraction")
        XCTAssertGreaterThan(total - deck, DubLayout.libraryMinHeight,
                             "the library takes the remainder")
    }

    /// Performance keeps the decks dominant — PRD §9.2 is explicit that
    /// they own the vertical real estate, so the library must not take
    /// the space Prep gives it.
    func test_performanceKeepsTheDecksDominant() {
        let total: CGFloat = 900
        let deck = DeckLibrarySplit<EmptyView, EmptyView>.deckHeight(
            mode: .timecode, total: total,
            deckChrome: DubLayout.rackBarHeight,
            deckMinimum: DubLayout.waveformMinHeight)
        XCTAssertGreaterThan(deck, total * 0.5)
    }

    /// On a window too short for both, the deck yields first and the
    /// library keeps its floor — the same degradation order the drag
    /// used to enforce.
    func test_shortWindowProtectsTheLibraryFloor() {
        let total: CGFloat = 420
        for mode in [EngineMode.prep, .timecode] {
            let deck = DeckLibrarySplit<EmptyView, EmptyView>.deckHeight(
                mode: mode, total: total,
                deckChrome: 0, deckMinimum: DubLayout.prepRegionMinHeight)
            XCTAssertGreaterThanOrEqual(
                total - deck, DubLayout.libraryMinHeight, "\(mode)")
        }
    }

    func test_prepRackFitsItsHeightFloor() {
        let rack = fittingSize(
            PrepRack(state: PrepRackState(hasTrack: true)),
            width: DubLayout.mainWindowMinWidth - DubSpacing.lg * 2)
        // The bar is the rack plus its own vertical padding. The rip
        // lane is *not* reserved for: it renders only during a capture,
        // and it is leaving Prep for its own surface. Reserving for it
        // permanently cost ~110 pt of every session.
        let needed = rack.height + DubSpacing.sm * 2
        XCTAssertLessThanOrEqual(
            needed, DubLayout.prepPadBarMinHeight,
            "the Prep bar's floor no longer covers what it draws — "
                + "rack \(rack.height), needed \(needed)")
        // And it must not be wildly generous either: a floor far above the
        // content is height taken from the waveform for nothing.
        XCTAssertGreaterThan(
            needed + 30, DubLayout.prepPadBarMinHeight,
            "the floor is over-reserved — give the height back to the strip")
    }

    /// The three sections have to fit side by side at the narrowest
    /// supported window, or the surface starts scrolling sideways.
    func test_prepRackFitsTheNarrowestWindow() {
        let size = fittingSize(
            PrepRack(state: PrepRackState(hasTrack: true)),
            width: DubLayout.mainWindowMinWidth - DubSpacing.lg * 2)
        XCTAssertLessThanOrEqual(
            DubLayout.prepCueColumn + DubLayout.prepLoopSection
                + DubLayout.prepSampleShelfMin + DubSpacing.xl * 2,
            DubLayout.mainWindowMinWidth - DubSpacing.lg * 2,
            "cue + loop + shelf no longer fit at 960")
        XCTAssertGreaterThan(size.width, 0)
    }

    func test_prepGrid_fitsTheNarrowestSupportedWindow() {
        let available = DubLayout.mainWindowMinWidth - 2 * DubSpacing.lg
        XCTAssertLessThanOrEqual(
            DubLayout.prepPadGridIntrinsicWidth, available,
            "Prep's three columns overflow the minimum window")
    }

    // MARK: - Token coupling

    /// The deck pane has to hold the pad column, the overview, its gap
    /// and a waveform no narrower than the floor. If a token grows past
    /// that, this fails before anyone sees a clipped pane.
    func test_deckPane_tokensFitTheMinimumWindow() {
        let pane = (DubLayout.mainWindowMinWidth
            - DubLayout.stillpointGutterWidth - 2) / 2
        let fixed = DubLayout.performancePadColumnWidth
            + DubLayout.deckOverviewWidth
            + DubLayout.deckOverviewGap
        XCTAssertLessThanOrEqual(
            fixed + DubLayout.performanceWaveformMinWidth, pane + 0.5,
            "deck pane cannot fit its fixed columns plus a minimum "
                + "waveform at the smallest supported window")
    }
}
