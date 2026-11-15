import XCTest

@testable import Dub

/// The stage rescue (P-33, 2026-11-15): INT saves the set, the needle is
/// calibrated while INT plays, and the TC segment says when the record is
/// ready to take the deck back.
final class SourceControlReadinessTests: XCTestCase {

    func testACalibratedLockedRecordUnderIntIsReady() {
        XCTAssertEqual(
            TimecodeReadiness.of(status: .internalPlay, calibrating: false, calibrated: true, lockState: 1),
            .ready)
    }

    func testAMeasuringNeedleUnderIntSaysSo() {
        XCTAssertEqual(
            TimecodeReadiness.of(status: .internalPlay, calibrating: true, calibrated: true, lockState: 1),
            .measuring)
    }

    /// No lamp for a needle that is up, scratching, or not yet measured:
    /// handing back then would give the deck to a record that cannot
    /// drive it.
    func testNoLampUntilTheRecordCanDrive() {
        for lock: UInt8 in [0, 2, 3] {
            XCTAssertEqual(
                TimecodeReadiness.of(status: .internalPlay, calibrating: false, calibrated: true, lockState: lock),
                .none, "lock state \(lock)")
        }
        XCTAssertEqual(
            TimecodeReadiness.of(status: .internalPlay, calibrating: false, calibrated: false, lockState: 1),
            .none)
    }

    /// The lamp is for the hand-back only; on TC the segment is lit.
    func testNoLampOffInt() {
        for status: SourceControlStatus? in [nil, .timecode, .calibrating, .thru, .fx, .off] {
            XCTAssertEqual(
                TimecodeReadiness.of(status: status, calibrating: false, calibrated: true, lockState: 1),
                .none)
        }
    }

    func testRecalibrateIsOfferedOnIntAsWellAsTc() {
        XCTAssertTrue(SourceControlView.offersRecalibrate(.internalPlay))
        XCTAssertTrue(SourceControlView.offersRecalibrate(.timecode))
        XCTAssertTrue(SourceControlView.offersRecalibrate(.calibrating))
        XCTAssertFalse(SourceControlView.offersRecalibrate(.thru))
        XCTAssertFalse(SourceControlView.offersRecalibrate(.fx))
        XCTAssertFalse(SourceControlView.offersRecalibrate(.off))
    }
}
