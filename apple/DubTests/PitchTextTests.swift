import XCTest

@testable import Dub

/// A platter resting at zero reads a few hundredths either side of it.
/// The readout must not flip its sign on a deck that is not moving.
final class PitchTextTests: XCTestCase {
    func testAValueThatRoundsToZeroHasOneSpelling() {
        for p in [-0.04, -0.0, 0.0, 0.04] {
            XCTAssertEqual(PitchText.signed(p), "+0.0", "\(p)")
        }
        XCTAssertEqual(PitchText.signed(-0.004, decimals: 2), "+0.00")
    }

    func testRealPitchKeepsItsSign() {
        XCTAssertEqual(PitchText.signed(-0.06), "-0.1")
        XCTAssertEqual(PitchText.signed(0.06), "+0.1")
        XCTAssertEqual(PitchText.signed(-7.96), "-8.0")
        XCTAssertEqual(PitchText.signed(-0.006, decimals: 2), "-0.01")
    }
}
