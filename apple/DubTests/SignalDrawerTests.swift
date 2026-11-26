import XCTest

@testable import Dub

/// The SIGNAL drawer opens for a deck's first calibration of the session,
/// slides away once the deck reads LOCKED, stays when something wants
/// fixing, and never moves by itself again (Daniel, 2026-11-25).
@MainActor
final class SignalDrawerTests: XCTestCase {

    private typealias Deck = SignalDrawer.Deck

    private let measuring = Deck(timecode: true, hasInput: true, settled: false, healthy: false)
    private let locked = Deck(timecode: true, hasInput: true, settled: true, healthy: true)
    private let wantsFixing = Deck(timecode: true, hasInput: true, settled: true, healthy: false)
    private let noInput = Deck(timecode: true, hasInput: false, settled: true, healthy: false)
    private let internalPlay = Deck(timecode: false, hasInput: true, settled: true, healthy: false)
    private let t0 = Date(timeIntervalSinceReferenceDate: 0)

    private func at(_ seconds: TimeInterval) -> Date { t0.addingTimeInterval(seconds) }

    func testWhatADeckWants() {
        XCTAssertEqual(SignalDrawer.want(measuring), .open)
        XCTAssertEqual(SignalDrawer.want(noInput), .open)
        XCTAssertEqual(SignalDrawer.want(locked), .go)
        XCTAssertEqual(SignalDrawer.want(wantsFixing), .stay)
        XCTAssertEqual(SignalDrawer.want(internalPlay), .shut)
        XCTAssertEqual(
            SignalDrawer.want(Deck(timecode: false, hasInput: true, settled: false, healthy: false)), .shut,
            "THRU, DUB FX and INT have nothing to calibrate")
    }

    /// Rig, 2026-09-23: timecode on, needle down, no track yet — the
    /// deck was calibrating and the column said nothing. That is the
    /// first thing a DJ does at a gig, so the drawer opens for it.
    func testAnUncalibratedTimecodeDeckOpensItsDrawer() {
        let drawer = SignalDrawer()
        drawer.observe(measuring, at: t0)
        XCTAssertTrue(drawer.open)
        XCTAssertTrue(drawer.showing)
        XCTAssertTrue(drawer.movedByItself)
    }

    func testADeckWithNoInputOpensItToo() {
        let drawer = SignalDrawer()
        drawer.observe(noInput, at: t0)
        XCTAssertTrue(drawer.open)
    }

    func testItSlidesAwayOnceTheDeckHasReadLockedForAMoment() {
        let drawer = SignalDrawer()
        drawer.observe(measuring, at: t0)
        drawer.observe(locked, at: at(5))
        drawer.observe(locked, at: at(5 + SignalDrawer.healthyFor - 0.1))
        XCTAssertTrue(drawer.open, "long enough to see Calibrated turn green")
        drawer.observe(locked, at: at(5 + SignalDrawer.healthyFor))
        XCTAssertFalse(drawer.open)
        XCTAssertTrue(drawer.movedByItself)
        XCTAssertFalse(drawer.minding)
    }

    func testAMomentThatIsNotLockedStartsTheWaitAgain() {
        let drawer = SignalDrawer()
        drawer.observe(measuring, at: t0)
        drawer.observe(locked, at: at(1))
        drawer.observe(wantsFixing, at: at(2))
        drawer.observe(locked, at: at(2.25))
        drawer.observe(locked, at: at(2.25 + SignalDrawer.healthyFor - 0.1))
        XCTAssertTrue(drawer.open)
        drawer.observe(locked, at: at(2.25 + SignalDrawer.healthyFor))
        XCTAssertFalse(drawer.open)
    }

    func testItStaysOpenWhileSomethingWantsFixing() {
        let drawer = SignalDrawer()
        drawer.observe(measuring, at: t0)
        for s in stride(from: 5.0, through: 120, by: 0.25) {
            drawer.observe(wantsFixing, at: at(s))
        }
        XCTAssertTrue(drawer.open)
        XCTAssertTrue(drawer.minding, "and it goes once it is fixed")
    }

    /// A reconnected interface measures the needle again mid-set; the
    /// tab says so, the drawer stays where the DJ's hands are not.
    func testItNeverOpensByItselfAgain() {
        let drawer = SignalDrawer()
        drawer.observe(measuring, at: t0)
        drawer.observe(locked, at: at(5))
        drawer.observe(locked, at: at(5 + SignalDrawer.healthyFor))
        XCTAssertFalse(drawer.open)
        drawer.observe(measuring, at: at(600))
        drawer.observe(noInput, at: at(601))
        XCTAssertFalse(drawer.open)
    }

    func testTheDJsClickEndsIt() {
        let closed = SignalDrawer()
        closed.observe(measuring, at: t0)
        closed.close()
        XCTAssertFalse(closed.open)
        XCTAssertFalse(closed.movedByItself)
        closed.observe(measuring, at: at(1))
        XCTAssertFalse(closed.open, "closed while it measured, it stays closed")

        let opened = SignalDrawer()
        opened.observe(measuring, at: t0)
        opened.toggle()
        opened.toggle()
        XCTAssertTrue(opened.open)
        for s in stride(from: 1.0, through: 30, by: 0.25) {
            opened.observe(locked, at: at(s))
        }
        XCTAssertTrue(opened.open, "opened by the DJ, it waits for the DJ")
    }

    /// Calibrated in Prep, or on INT before the switch went to TC: the
    /// drawer has nothing to show and never opens for it.
    func testADeckCalibratedBeforeItWasSeenNeverOpens() {
        let drawer = SignalDrawer()
        drawer.observe(locked, at: t0)
        XCTAssertFalse(drawer.open)
        XCTAssertFalse(drawer.minding)
        drawer.observe(measuring, at: at(1))
        XCTAssertFalse(drawer.open)
    }

    func testOffTimecodeItStaysShutUntilTheSwitchGoesToTC() {
        let drawer = SignalDrawer()
        drawer.observe(internalPlay, at: t0)
        XCTAssertFalse(drawer.open)
        XCTAssertTrue(drawer.minding)
        drawer.observe(measuring, at: at(1))
        XCTAssertTrue(drawer.open)
        drawer.observe(internalPlay, at: at(2))
        XCTAssertFalse(drawer.open, "it opened itself, so it shuts itself for INT")
    }

    func testOpenAtLaunchItNeverMovesByItself() {
        let drawer = SignalDrawer(open: true)
        drawer.observe(locked, at: t0)
        drawer.observe(locked, at: at(10))
        XCTAssertTrue(drawer.open)
        XCTAssertTrue(drawer.showing)
    }

    /// The panel's content stays in the tree for the slide away, so it
    /// leaves with the drawer instead of vanishing before it moves.
    func testThePanelOutlastsItsSlideAway() {
        var pending: [DispatchWorkItem] = []
        let drawer = SignalDrawer(later: { _, work in pending.append(work) })
        drawer.observe(measuring, at: t0)
        drawer.close()
        XCTAssertTrue(drawer.showing, "gone before it slid")
        pending.forEach { $0.perform() }
        XCTAssertFalse(drawer.showing)
    }

    func testReopenedMidSlideItKeepsItsPanel() {
        var pending: [DispatchWorkItem] = []
        let drawer = SignalDrawer(later: { _, work in pending.append(work) })
        drawer.observe(measuring, at: t0)
        drawer.close()
        drawer.toggle()
        pending.forEach { $0.perform() }
        XCTAssertTrue(drawer.open)
        XCTAssertTrue(drawer.showing)
    }
}
