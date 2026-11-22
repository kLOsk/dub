import SwiftUI
import XCTest
import simd

@testable import Dub

/// The playing lane is drawn twice: by Metal when a track is loaded, and
/// by SwiftUI when it is not. Those two have to be the same colour, or a
/// deck changes shade the moment a track lands — which it did, because
/// the Metal clear was `0.07, 0.07, 0.08` and the idle pane painted
/// `surface0`.
///
/// Neither side can import the other — the Metal layer deliberately does
/// not know about the design system — so the pair is held together here
/// instead of by a shared constant.
final class WaveformLaneColourTests: XCTestCase {

    /// Compared in device RGB, not sRGB: the Metal layer is untagged,
    /// so its clear reaches the screen as device values, and the token
    /// is authored in device RGB for that reason. Compared as sRGB the
    /// two matched in numbers and still differed on screen — the empty
    /// deck grey, the loaded one black (rig, 2026-11-15).
    func testTheIdleLaneMatchesTheMetalClearColour() {
        guard let device = NSColor(DubColor.waveformLane).usingColorSpace(.deviceRGB) else {
            return XCTFail("the lane token has no device RGB form")
        }
        let token = SIMD3<Float>(
            Float(device.redComponent), Float(device.greenComponent), Float(device.blueComponent))
        let clear = WaveformRenderer.laneClearRGB
        // One 8-bit step of tolerance: the token is authored as a hex
        // triple and the clear as floats, so they cannot be bit-equal.
        let tolerance: Float = 1.0 / 255.0
        XCTAssertEqual(token.x, clear.x, accuracy: tolerance, "red")
        XCTAssertEqual(token.y, clear.y, accuracy: tolerance, "green")
        XCTAssertEqual(token.z, clear.z, accuracy: tolerance, "blue")
    }

    /// The lane is a step *above* the app's ground, which is what makes
    /// the deck read as a surface set into the window rather than a hole
    /// in it. If these ever converge the distinction is gone and one of
    /// them should be deleted rather than left as a duplicate.
    func testTheLaneIsDistinctFromTheAppGround() {
        XCTAssertNotEqual(
            HotCueMarker.components(DubColor.waveformLane),
            HotCueMarker.components(DubColor.surface0))
    }

    // MARK: - Zoom / aggregation

    /// **The whole zoom ladder in one identity.**
    ///
    /// A drawn column covers `chunksPerColumn` peak chunks and occupies
    /// `pixelsPerDrawnColumn` device pixels, so
    ///
    ///     secsPerPixel = chunksPerColumn * peakDur / pixelsPerColumn
    ///
    /// and for the ladder to mean what its labels say, that has to be
    /// exactly proportional to zoom. Which lever moves is an
    /// implementation detail: above the one-pixel floor the pixel axis
    /// carries it, below the floor the aggregation does. The ratio is
    /// what must hold, at every rung.
    func testEveryZoomRungScalesSecondsPerPixelWithZoom() {
        for zoom in WaveformZoom.steps {
            let pixels = WaveformRenderer.effectivePixelsPerDrawnColumn(
                timeAxisZoom: zoom)
            let agg = Double(WaveformRenderer.columnAggregation(timeAxisZoom: zoom))
            XCTAssertEqual(
                agg / pixels, zoom, accuracy: 1e-9,
                "zoom \(zoom): \(agg) chunks over \(pixels) px is "
                    + "\(agg / pixels) s/px, not \(zoom)")
        }
    }

    /// The rung that prompted this: 0.25x has to show twice the audio
    /// of 0.5x. Both sit on the one-pixel column floor, so the pixel
    /// axis cannot express the difference and the aggregation must.
    /// Before this existed the two rungs drew an identical picture.
    func testQuarterZoomShowsTwiceTheAudioOfHalfZoom() {
        let half = 2.0    // 0.5x
        let quarter = 4.0 // 0.25x
        XCTAssertEqual(
            WaveformRenderer.effectivePixelsPerDrawnColumn(timeAxisZoom: half),
            WaveformRenderer.effectivePixelsPerDrawnColumn(timeAxisZoom: quarter),
            "both rungs are on the pixel floor; that is the premise")
        XCTAssertEqual(
            WaveformRenderer.columnAggregation(timeAxisZoom: quarter),
            2 * WaveformRenderer.columnAggregation(timeAxisZoom: half),
            "0.25x must fold twice the chunks into a column")
    }

    /// 0.5× on a 44.1 kHz record at zero pitch is zoom 2.0 exactly, the
    /// step between two and four chunks a column. A rate wobbling
    /// around zero must not redraw the strip each time it crosses.
    func testAZoomWobblingOnAStepKeepsItsAggregation() {
        for start: UInt32 in [2, 4] {
            var held: UInt32? = start
            for wobble in [-0.0005, 0.0004, -0.0003, 0.0005, 0.03, -0.03] {
                let agg = WaveformRenderer.columnAggregation(
                    timeAxisZoom: 2.0 * (1 + wobble), holding: held)
                XCTAssertEqual(agg, start, "a \(wobble * 100) % wobble moved it off \(start)")
                held = agg
            }
        }
    }

    /// The hold is a margin, not a latch: a real pitch move or a new
    /// zoom rung still gets the aggregation it asks for.
    func testAZoomWellPastAStepChangesAggregation() {
        XCTAssertEqual(WaveformRenderer.columnAggregation(timeAxisZoom: 2.16, holding: 2), 4)
        XCTAssertEqual(WaveformRenderer.columnAggregation(timeAxisZoom: 1.84, holding: 4), 2)
        XCTAssertEqual(WaveformRenderer.columnAggregation(timeAxisZoom: 4.0, holding: 2), 4)
        XCTAssertEqual(WaveformRenderer.columnAggregation(timeAxisZoom: 1.0, holding: 4), 2)
        XCTAssertEqual(WaveformRenderer.columnAggregation(timeAxisZoom: 2.0, holding: nil), 2)
    }

    /// Pitch never changes how much audio a column holds: anywhere a
    /// fader reaches, the strip keeps the aggregation of its zoom rung.
    func testPitchDoesNotChangeTheAggregation() {
        for zoom in WaveformZoom.steps {
            let rest = WaveformRenderer.columnAggregation(timeAxisZoom: zoom)
            for rate in [0.70, 0.92, 0.9995, 1.0005, 1.08, 1.35, 1.49] {
                XCTAssertEqual(
                    WaveformRenderer.columnAggregation(
                        timeAxisZoom: zoom * rate, restZoom: zoom, holding: nil),
                    rest, "zoom \(zoom) at rate \(rate)")
            }
        }
    }

    /// Odd aggregations would re-pair a transient chunk with a
    /// different neighbour frame to frame, which is the band-colour
    /// flicker the playhead snap exists to prevent.
    func testAggregationStaysEven() {
        for zoom in WaveformZoom.steps + [8.0, 16.0, 3.0, 5.0] {
            let agg = WaveformRenderer.columnAggregation(timeAxisZoom: zoom)
            XCTAssertEqual(agg % 2, 0, "zoom \(zoom) gave an odd aggregation \(agg)")
            XCTAssertGreaterThanOrEqual(agg, 2)
        }
    }

}
