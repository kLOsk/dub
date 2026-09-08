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

    /// 1440 × 900 minus 236 pt of fixed chrome, at the 0.60 deck-heavy
    /// split.
    private let paneHeight: CGFloat = 398

    /// The regression test for the reported bug. The column's height
    /// now depends on exactly two booleans — the siren left for the
    /// global rack bar — so the shipping space is cheap to cover
    /// exhaustively.
    ///
    /// This is the assertion that would have gone red the day the siren
    /// row landed in M16, instead of the sampler quietly disappearing
    /// under the FX bar a milestone later.
    func test_padColumn_fitsThePane_inEveryShippingConfiguration() {
        for echo in [true, false] {
            var state = PerformancePadsState.shippingDefault
            state.echoEnabled = echo
            let size = fittingSize(
                PerformancePadsView(side: .a, state: state),
                width: DubLayout.performancePadColumnWidth)
            XCTAssertLessThanOrEqual(
                size.height, paneHeight + 0.5,
                "pad column overflows the 1440×900 pane (echo: \(echo))")
        }
    }

    /// The one configuration that does *not* fit at 1440 × 900, stated
    /// deliberately rather than left to be discovered.
    ///
    /// The dormant per-deck FX rack adds ~173 pt as a fifth row. It is
    /// off by default and slated to become a dedicated FX *channel*
    /// that replaces a deck (UI-BACKLOG F-38) rather than stacking onto
    /// one, so shrinking it now would be work thrown away. Until then
    /// `ViewThatFits` scrolls it, which is the whole reason that
    /// fallback is there.
    ///
    /// If this assertion ever fails, the rack got shorter or the pane
    /// got taller — delete the scroll fallback and fold this case back
    /// into the test above.
    func test_padColumn_withDormantRack_stillNeedsTheScrollFallback() {
        var state = PerformancePadsState.shippingDefault
        state.rackEnabled = true
        let size = fittingSize(
            PerformancePadsView(side: .a, state: state),
            width: DubLayout.performancePadColumnWidth)
        XCTAssertGreaterThan(size.height, paneHeight)
    }

    /// The column must fit the width token it is framed at, or the
    /// LOOP row clips its own pads — which the 320 pt baseline did for
    /// a whole milestone, rendering labels as "UE" and "OOP".
    func test_padColumn_fitsItsWidthToken() {
        let size = fittingSize(
            PerformancePadsView(side: .a, state: .shippingDefault),
            width: DubLayout.performancePadColumnWidth)
        XCTAssertLessThanOrEqual(
            size.width, DubLayout.performancePadColumnWidth + 0.5,
            "pad column is wider than performancePadColumnWidth")
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
    func test_prepRackFitsItsHeightFloor() {
        let rack = fittingSize(
            PrepRack(state: PrepRackState(hasTrack: true)),
            width: DubLayout.mainWindowMinWidth - DubSpacing.lg * 2)
        // The bar is the rack + its own vertical padding + the rip lane.
        let needed = rack.height + DubSpacing.sm * 2 + 56
        XCTAssertLessThanOrEqual(
            needed, DubLayout.prepPadBarMinHeight,
            "the Prep bar's floor no longer covers what it draws — "
                + "rack \(rack.height), needed \(needed)")
        // And it must not be wildly generous either: a floor far above the
        // content is height taken from the waveform for nothing.
        XCTAssertGreaterThan(
            needed + 60, DubLayout.prepPadBarMinHeight,
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
