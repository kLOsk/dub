import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

/// Baselines for Prep's control surface.
///
/// These began as a "before" record of the old two-column pad grid, taken
/// so the contrast package would land as a reviewable image diff rather
/// than a claim. They did that job; the grid is gone and they now pin
/// `PrepRack`.
///
/// What they are actually guarding is the thing the redesign exists for:
/// **the three sections must not converge**. A list of marks, a boxed
/// instrument and a dashed drop zone are three recognisable objects with
/// the labels covered. If a future change gives them one shared frame
/// again, these images say so.
///
/// Sections that moved to Performance — the siren row, its Expert panel,
/// echo out — are covered by `PerformanceSnapshotTests` instead, which is
/// where they now render.
@MainActor
final class PrepPadSnapshotTests: XCTestCase {

    private func snap(
        _ view: some View,
        width: CGFloat,
        height: CGFloat,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        // `prepPadRows` pads the bar; matching it here means the
        // snapshot frames the surface the way the app does rather than
        // running it to the pixel edge.
        let sized = view
            .padding(.horizontal, DubSpacing.lg)
            .padding(.vertical, DubSpacing.sm)
            .frame(width: width, height: height, alignment: .topLeading)
            .background(DubColor.surface0)
        let host = NSHostingView(rootView: sized)
        host.frame = CGRect(x: 0, y: 0, width: width, height: height)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: host, as: .image(perceptualPrecision: 0.98), named: name,
            file: file, testName: testName, line: line)
    }

    /// Roughly the pad bar's real width at the minimum window.
    private static let surfaceWidth: CGFloat = 900

    private func mark(_ secs: Double, _ name: String?, _ color: String? = nil) -> CueMark {
        CueMark(positionSecs: secs, name: name, color: color)
    }

    // MARK: - The whole surface

    /// What the DJ opens Prep to with nothing loaded. Every section has to
    /// say what it needs rather than sitting inert.
    func test_prepRack_noTrack() {
        snap(
            PrepRack(state: PrepRackState()),
            width: Self.surfaceWidth, height: 230, named: "rack-no-track")
    }

    /// A track loaded, two cues named and coloured, a 2-bar loop running,
    /// samples in the bank. The state the surface is designed around.
    func test_prepRack_working() {
        let state = PrepRackState(
            cues: (0..<8).map { i in
                switch i {
                case 0: return CueSlotState(index: 0, mark: mark(0, "INTRO", "aqua"))
                case 1: return CueSlotState(index: 1, mark: mark(102.3, "FIRST VERSE", "orange"))
                case 4: return CueSlotState(index: 4, mark: mark(190.0, "BREAK", "green"))
                default: return CueSlotState(index: i)
                }
            },
            activeLoopBeats: 4,
            loopEngaged: true,
            sampleSlots: ["Air Horn", "Reload", nil, "Siren Up", nil, nil, nil, nil],
            hasTrack: true)
        snap(
            PrepRack(state: state),
            width: Self.surfaceWidth, height: 230, named: "rack-working")
    }

    /// The three silhouettes with nothing set: an unboxed list of dashed
    /// rows, a boxed instrument, a dashed drop zone. This is the image to
    /// look at if anyone asks whether the sections still read as different
    /// objects.
    func test_prepRack_loadedButEmpty() {
        snap(
            PrepRack(state: PrepRackState(hasTrack: true)),
            width: Self.surfaceWidth, height: 230, named: "rack-loaded-empty")
    }

    /// An unnamed cue is the common case — most are dropped mid-listen and
    /// never labelled — so the row must still read without a name.
    func test_prepRack_unnamedCues() {
        let state = PrepRackState(
            cues: (0..<8).map { i in
                i < 3
                    ? CueSlotState(index: i, mark: mark(Double(i) * 40 + 12.4, nil))
                    : CueSlotState(index: i)
            },
            hasTrack: true)
        snap(
            PrepRack(state: state),
            width: Self.surfaceWidth, height: 230, named: "rack-unnamed-cues")
    }

    /// A sub-beat loop. These were unreachable before FFI 68 — the bars
    /// wrapper rounded them to a whole beat — so the window has scrolled
    /// to show 1/4 lit.
    func test_prepRack_subBeatLoop() {
        snap(
            PrepRack(state: PrepRackState(activeLoopBeats: 0.25, loopEngaged: true,
                                          hasTrack: true)),
            width: Self.surfaceWidth, height: 230, named: "rack-sub-beat-loop")
    }

    /// A long sample name has to truncate inside the shelf rather than
    /// pushing it wider — the one string on this surface whose length is
    /// not ours to choose.
    func test_prepRack_longSampleNames() {
        let state = PrepRackState(
            sampleSlots: [
                "Amen Break Full Length Reference Bounce 24bit",
                "Horn", nil, "Reload Siren (Benidub DS01E, long tail)",
                nil, nil, nil, nil,
            ],
            hasTrack: true)
        snap(
            PrepRack(state: state),
            width: Self.surfaceWidth, height: 230, named: "rack-long-sample-names")
    }

    // MARK: - Formatting

    /// Tenths, because a cue set by ear is placed to about that precision.
    func testCueTimecodeFormatsToTenths() {
        XCTAssertEqual(CueTimecode.format(0), "0:00.0")
        XCTAssertEqual(CueTimecode.format(102.34), "1:42.3")
        // Binary float: 102.3 - 102 is 0.2999…, so truncating the
        // remainder rendered this as 1:42.2.
        XCTAssertEqual(CueTimecode.format(102.3), "1:42.3")
        XCTAssertEqual(CueTimecode.format(0.999), "0:01.0")
        XCTAssertEqual(CueTimecode.format(3599.94), "59:59.9")
        // Rounding carries across the minute rather than clamping.
        XCTAssertEqual(CueTimecode.format(3599.95), "60:00.0")
        XCTAssertEqual(CueTimecode.format(59.97), "1:00.0")
    }

    /// A cue that has not been set has no time, and must not render as
    /// `0:00.0` — that reads as a mark at the top of the track.
    func testCueTimecodeRefusesNonsense() {
        XCTAssertEqual(CueTimecode.format(-1), "—")
        XCTAssertEqual(CueTimecode.format(.nan), "—")
        XCTAssertEqual(CueTimecode.format(.infinity), "—")
    }
}
