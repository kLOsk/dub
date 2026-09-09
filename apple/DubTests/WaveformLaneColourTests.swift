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

    func testTheIdleLaneMatchesTheMetalClearColour() {
        let token = HotCueMarker.components(DubColor.waveformLane)
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
}
