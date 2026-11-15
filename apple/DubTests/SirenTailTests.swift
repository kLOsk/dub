import XCTest

@testable import Dub

/// The TAIL row fills in step with the DUB knob (rig, 2026-11-15): by
/// the echo's own repeats it read as nothing for half the knob and then
/// everything at once, because the tail itself grows fastest at the top.
final class SirenTailTests: XCTestCase {

    private func level(_ i: Int, _ m: Double) -> Double {
        SirenTail.level(repeat: i, on: true, amount: m)
    }

    /// How much of the row past the shot is lit: the sum of the fills.
    private func lit(_ m: Double) -> Double {
        (1..<12).map { level($0, m) > 0.08 ? 1.0 : 0.0 }.reduce(0, +)
    }

    func testOnePercentLightsAlmostNothing() {
        XCTAssertEqual(level(0, 0.01), 1, "the shot")
        XCTAssertLessThan(level(1, 0.01), 0.2)
        XCTAssertEqual(level(2, 0.01), 0.08)
    }

    /// Equal turns of the knob light equal stretches of the row.
    func testTheRowIsLinearInTheKnob() {
        for m in stride(from: 0.0, through: 1.0, by: 0.1) {
            XCTAssertEqual(lit(m), (m * 11).rounded(.up), accuracy: 0.001, "at \(m)")
        }
    }

    func testTheTopOfTheKnobLightsTheWholeRow() {
        XCTAssertTrue((1..<12).allSatisfy { level($0, 1) > 0.3 })
    }

    /// Lit segments fade along the row, the way a tail does.
    func testTheRowFadesAlongTheTail() {
        let levels = (0..<12).map { level($0, 1) }
        for i in 1..<levels.count { XCTAssertLessThanOrEqual(levels[i], levels[i - 1]) }
    }

    func testDryIsDark() {
        XCTAssertEqual(SirenTail.level(repeat: 0, on: false, amount: 0.5), 0.08)
        XCTAssertEqual(SirenTail.level(repeat: 3, on: false, amount: 0.5), 0.08)
    }
}
