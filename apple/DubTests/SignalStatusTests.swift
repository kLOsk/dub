import XCTest

@testable import Dub

/// The signal panel's words: one state, one line of advice, in the
/// order a DJ needs them — no input at all, then a failing side, then
/// the lock (redesign, 2026-11-15).
final class SignalStatusTests: XCTestCase {

    private func status(
        input: Bool = true, lock: UInt8 = 1, weak: UInt8 = 0,
        left: Float = 0.2, right: Float = 0.2, pitch: Double = 0
    ) -> SignalStatus {
        SignalStatus.of(
            hasInput: input, lockState: lock, weakChannel: weak,
            rms: (left, right), pitchPercent: pitch)
    }

    func testAHealthyRecordIsLocked() {
        let s = status()
        XCTAssertEqual(s.kind, .locked)
        XCTAssertEqual(s.advice, "Timecode · forward")
        XCTAssertEqual(status(pitch: -200).advice, "Timecode · reverse")
        XCTAssertEqual(status(pitch: -100).advice, "Timecode · stopped")
    }

    func testNoRoutedInputSaysWhereToFixIt() {
        let s = status(input: false, lock: 0)
        XCTAssertEqual(s.kind, .noInput)
        XCTAssertTrue(s.advice.contains("Preferences"))
    }

    func testSilenceAsksAboutTheNeedle() {
        let s = status(lock: 0, left: 0, right: 0)
        XCTAssertEqual(s.kind, .noSignal)
        XCTAssertTrue(s.advice.contains("PHONO/LINE"))
    }

    /// A dead side wins over the lock: the decoder can hold a lock on
    /// one channel for a while, and the fault is still the DJ's to fix.
    func testAFailingSideIsNamed() {
        let live = status(lock: 1, left: 0.2, right: 0.01)
        XCTAssertEqual(live.kind, .weak)
        XCTAssertTrue(live.advice.hasPrefix("Right"), live.advice)
        XCTAssertTrue(status(weak: 1).advice.hasPrefix("Left"))
    }

    /// Two quiet sides are not an imbalance, and a small difference is
    /// a balance, not a fault.
    func testQuietOrSlightlyUnevenIsNotWeak() {
        XCTAssertNil(SignalStatus.weakSide(weakChannel: 0, rms: (0.002, 0.0001)))
        XCTAssertNil(SignalStatus.weakSide(weakChannel: 0, rms: (0.2, 0.1)))
    }

    func testScratchingIsNotAFault() {
        XCTAssertEqual(status(lock: 3).kind, .noLock)
    }

    func testLevelsReadInDecibels() {
        XCTAssertEqual(SignalStatus.levelFigure(0), "—")
        XCTAssertEqual(SignalStatus.levelFigure(0.5), "−6 dB")
        XCTAssertEqual(SignalStatus.levelFraction(1), 1, accuracy: 1e-9)
        XCTAssertEqual(SignalStatus.levelFraction(0.001), 0, accuracy: 1e-9)
    }

    /// The calibration line says what the deck is doing now; it measures
    /// by itself each session, so there is no date to show.
    func testTheCalibrationLine() {
        XCTAssertEqual(
            SignalStatus.calibration(hasInput: true, pitchSettled: true, calibrating: false, calibrated: true).text,
            "Calibrated")
        XCTAssertTrue(
            SignalStatus.calibration(hasInput: true, pitchSettled: false, calibrating: false, calibrated: false)
                .text.hasPrefix("Measuring"))
        XCTAssertEqual(
            SignalStatus.calibration(hasInput: false, pitchSettled: true, calibrating: false, calibrated: false).text,
            "")
    }
}
