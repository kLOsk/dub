import XCTest

@testable import Dub

/// Rig, 2026-09-24: in rip review a track dragged from the library
/// loaded onto deck A and played under the recording's trims and split
/// markers. While a recording owns deck A nothing else may load there —
/// and the refusal says what ends the recording, because that was the
/// other thing the DJ could not find.
final class RipLoadGuardTests: XCTestCase {

    func testNoRecordingLoadsFreely() {
        XCTAssertNil(WaveformAppModel.loadRefusal(ripPhase: .none, ripOwned: false))
    }

    func testEveryRecordingPhaseRefusesALibraryLoad() {
        for phase: RipUiPhase in [.capture, .review, .encoding, .done, .failed] {
            XCTAssertNotNil(
                WaveformAppModel.loadRefusal(ripPhase: phase, ripOwned: false), "\(phase)")
        }
    }

    /// The recording's own loads — the spill for audition, a re-split
    /// side — are what the phase is for.
    func testTheRecordingsOwnLoadsGoThrough() {
        for phase: RipUiPhase in [.capture, .review, .encoding, .done, .failed] {
            XCTAssertNil(WaveformAppModel.loadRefusal(ripPhase: phase, ripOwned: true))
        }
    }

    /// The message names the way out.
    func testTheRefusalSaysHowToFinish() {
        let review = WaveformAppModel.loadRefusal(ripPhase: .review, ripOwned: false) ?? ""
        XCTAssertTrue(review.contains("Encode & Import"), review)
        XCTAssertTrue(review.contains("Discard"), review)
    }
}
