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
        // The capture and the pitch settle are one job to the DJ.
        XCTAssertEqual(
            SignalStatus.calibration(hasInput: true, pitchSettled: false, calibrating: true, calibrated: false).text,
            "Calibrating…")
        XCTAssertEqual(
            SignalStatus.calibration(hasInput: true, pitchSettled: false, calibrating: false, calibrated: true).text,
            "Calibrating…")
        XCTAssertEqual(
            SignalStatus.calibration(
                hasInput: true, pitchSettled: false, calibrating: false, calibrated: false, turning: false).text,
            "Calibrating · play the record")
        // A recalibration on INT: the engine calls every mode but TC settled.
        XCTAssertEqual(
            SignalStatus.calibration(hasInput: true, pitchSettled: true, calibrating: true, calibrated: false).text,
            "Calibrating…")
        XCTAssertEqual(
            SignalStatus.calibration(
                hasInput: true, pitchSettled: false, calibrating: false, calibrated: false, weakChannel: 2).text,
            "Can't calibrate · right channel weak")
        XCTAssertEqual(
            SignalStatus.calibration(hasInput: false, pitchSettled: true, calibrating: false, calibrated: false).text,
            "")
        XCTAssertTrue(SignalStatus.measuring(hasInput: true, pitchSettled: false))
        XCTAssertFalse(SignalStatus.measuring(hasInput: false, pitchSettled: false))
    }

    /// The lines that keep the SIGNAL drawer open ask the DJ to do
    /// something; MK2's speed only is what the record is.
    func testWhichVinylLinesAsk() {
        XCTAssertFalse(SignalStatus.vinylAsks(vinyl: 1, synthetic: false, counted: false), "Serato")
        XCTAssertFalse(SignalStatus.vinylAsks(vinyl: 3, synthetic: false, counted: false), "MK2, speed only")
        XCTAssertFalse(SignalStatus.vinylAsks(vinyl: 1, synthetic: true, counted: true), "Phase, Serato mode")
        XCTAssertTrue(SignalStatus.vinylAsks(vinyl: 1, synthetic: true, counted: false), "Phase, Beta off")
        XCTAssertTrue(SignalStatus.vinylAsks(vinyl: 3, synthetic: true, counted: true), "Phase, Traktor mode")
    }

    /// Calibration names the record and its speed; the row says whether
    /// the deck reads its position or only its speed.
    func testTheVinylRow() {
        XCTAssertEqual(SignalStatus.vinylName(vinyl: 1, rpm: 33), "SERATO · 33")
        XCTAssertEqual(SignalStatus.vinylName(vinyl: 2, rpm: 45), "TRAKTOR MK1 · 45")
        XCTAssertEqual(SignalStatus.vinylName(vinyl: 3, rpm: 33), "TRAKTOR MK2 · 33")
        XCTAssertNil(SignalStatus.vinylName(vinyl: 0, rpm: 0))

        let serato = SignalStatus.vinyl(hasInput: true, vinyl: 1, rpm: 33, positionCode: true, detecting: false)
        XCTAssertEqual(serato.name, "SERATO · 33")
        XCTAssertTrue(serato.note.hasPrefix("Position"))
        let phase = SignalStatus.vinyl(hasInput: true, vinyl: 3, rpm: 33, positionCode: false, detecting: false)
        XCTAssertTrue(phase.note.hasPrefix("Speed only"))
        XCTAssertEqual(
            SignalStatus.vinyl(hasInput: true, vinyl: 0, rpm: 0, positionCode: false, detecting: true).name, "…")
        XCTAssertEqual(
            SignalStatus.vinyl(hasInput: false, vinyl: 1, rpm: 33, positionCode: true, detecting: false).name, "—")
    }

    /// MWM Phase is named for what it is, says whether it is counted, and
    /// in its Traktor mode (the count drifts through juggling) asks for
    /// Serato mode, its factory setting, instead.
    func testPhaseOnTheVinylRow() {
        XCTAssertEqual(SignalStatus.vinylName(vinyl: 1, rpm: 33, synthetic: true), "PHASE · 33")
        let counted = SignalStatus.vinyl(
            hasInput: true, vinyl: 1, rpm: 33, positionCode: false, detecting: false,
            synthetic: true, counted: true)
        XCTAssertEqual(counted.name, "PHASE · 33")
        XCTAssertTrue(counted.note.hasPrefix("Counted"))
        let off = SignalStatus.vinyl(
            hasInput: true, vinyl: 1, rpm: 33, positionCode: false, detecting: false,
            synthetic: true, counted: false)
        XCTAssertTrue(off.note.contains("Beta"))
        let traktorMode = SignalStatus.vinyl(
            hasInput: true, vinyl: 3, rpm: 33, positionCode: false, detecting: false,
            synthetic: true, counted: true)
        XCTAssertTrue(traktorMode.note.contains("Serato"))
    }
}
