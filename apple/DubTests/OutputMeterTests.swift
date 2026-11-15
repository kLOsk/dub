import SnapshotTesting
import XCTest

@testable import Dub

/// The gutter's output meters: what leaves each deck's pair, and why
/// nothing does (2026-11-15, canvas "Dub Output Meters").
final class OutputMeterTests: XCTestCase {

    func testAPlayingDeckLightsSegmentsBy2Db() {
        XCTAssertEqual(OutputMeterReading.litSegments(1), 24)
        XCTAssertEqual(OutputMeterReading.litSegments(0.5), 21)  // −6 dB
        XCTAssertEqual(OutputMeterReading.litSegments(0.004), 0)  // −48 dB
        XCTAssertEqual(OutputMeterReading.litSegments(0), 0)
        XCTAssertEqual(OutputMeterReading.of(peak: 0.5, routed: true, echoEngaged: false).kind, .sending)
    }

    func testTheScaleTurnsAmberThenRedAtTheTop() {
        XCTAssertEqual(OutputMeterReading.tone(0), .green)
        XCTAssertEqual(OutputMeterReading.tone(18), .green)  // −10 … −12
        XCTAssertEqual(OutputMeterReading.tone(19), .amber)  // top at −8
        XCTAssertEqual(OutputMeterReading.tone(22), .amber)
        XCTAssertEqual(OutputMeterReading.tone(23), .red)
    }

    /// Echo out on and its tail ringing: the tail is what leaves.
    func testAnEchoTailIsMarkedAsATail() {
        let r = OutputMeterReading.of(peak: 0.1, routed: true, echoEngaged: true)
        XCTAssertEqual(r.kind, .echoTail)
        XCTAssertNil(r.word)
    }

    /// The stage confusion: echo out still on, nothing leaving.
    func testASpentEchoSaysSo() {
        let r = OutputMeterReading.of(peak: 0.001, routed: true, echoEngaged: true)
        XCTAssertEqual(r.kind, .echoSilent)
        XCTAssertEqual(r.word, "ECHO OUT")
    }

    func testNoPairIsNoOutputWhateverElseIsGoingOn() {
        let r = OutputMeterReading.of(peak: 0.5, routed: false, echoEngaged: true)
        XCTAssertEqual(r.kind, .noOutput)
        XCTAssertEqual(r.word, "NO OUTPUT")
    }

    /// A deck simply stopped is silent with nothing to explain.
    func testAStoppedDeckIsJustEmpty() {
        let r = OutputMeterReading.of(peak: 0, routed: true, echoEngaged: false)
        XCTAssertEqual(r.kind, .sending)
        XCTAssertEqual(r.lit, 0)
        XCTAssertNil(r.word)
    }

    func testTheMetersStandUnderThePhaseMeter() throws {
        let size = CGSize(width: DubLayout.phaseMeterGutterWidth, height: 360)
        let rect = try XCTUnwrap(OutputMeterPairView.meterRect(in: size))
        XCTAssertGreaterThan(rect.minY, PhaseMeterCanvas.geometry(size).track.maxY + 10)
        XCTAssertLessThan(rect.maxY, size.height)
        XCTAssertLessThanOrEqual(rect.width, DubLayout.phaseMeterGutterWidth)
        // A gutter too short for a meter worth reading gets none.
        XCTAssertNil(OutputMeterPairView.meterRect(in: CGSize(width: 36, height: 120)))
    }

    /// The four states of the canvas, side by side.
    @MainActor
    func testSnapshotTheStates() {
        let states: [[OutputMeterReading]] = [
            [.of(peak: 0.45, routed: true, echoEngaged: false), .of(peak: 0.2, routed: true, echoEngaged: false)],
            [.of(peak: 0.08, routed: true, echoEngaged: true), .of(peak: 0.25, routed: true, echoEngaged: false)],
            [.of(peak: 0, routed: true, echoEngaged: true), .of(peak: 0.28, routed: true, echoEngaged: false)],
            [.of(peak: 0.35, routed: true, echoEngaged: false), .of(peak: 0, routed: false, echoEngaged: false)],
        ]
        let host = NSView(frame: CGRect(x: 0, y: 0, width: 4 * 52, height: 360))
        host.wantsLayer = true
        host.layer?.backgroundColor = NSColor(DubColor.surface0).cgColor
        for (i, state) in states.enumerated() {
            let v = OutputMeterPairView(
                frame: CGRect(x: CGFloat(i) * 52 + 8, y: 0, width: DubLayout.phaseMeterGutterWidth, height: 360))
            v.read = { state }
            v.refresh()
            host.addSubview(v)
        }
        assertSnapshot(of: host, as: .image(perceptualPrecision: 0.98), named: "output-meter-states")
    }
}
