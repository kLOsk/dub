import XCTest

@testable import Dub

/// While macOS asks whether Dub may use the microphone, the idle decks
/// say so. They used to read "ENGINE STOPPED · Open Preferences (⌘,) to
/// pick an input device", and nothing in Preferences helps until the
/// prompt is answered (every rebuilt Debug app asks again; rig,
/// 2026-11-27 and 28).
final class MicrophoneWaitTests: XCTestCase {

    func testAWaitingDeckNamesThePromptNotPreferences() {
        XCTAssertEqual(
            PerformanceView.idleCaption(side: .a, running: false, mode: .timecode, awaitingMicrophone: true),
            "WAITING FOR MICROPHONE ACCESS")
        XCTAssertEqual(
            PerformanceView.idleCaption(side: .b, running: false, mode: .timecode, awaitingMicrophone: true),
            "WAITING FOR MICROPHONE ACCESS")
        let hint = PerformanceView.idleHint(
            side: .a, running: false, mode: .timecode, awaitingMicrophone: true, lastError: nil)
        XCTAssertTrue(hint.contains("Allow"), hint)
        XCTAssertFalse(hint.contains("Preferences"), hint)
    }

    /// The prompt outranks a start error left from before it: until it is
    /// answered, that is the only thing the DJ can act on.
    func testThePromptComesBeforeAnOldError() {
        let hint = PerformanceView.idleHint(
            side: .a, running: false, mode: .timecode, awaitingMicrophone: true,
            lastError: "audio_unit.start (input) failed")
        XCTAssertTrue(hint.contains("Allow"), hint)
    }

    /// Answered, or never asked, the stopped engine reads as before: the
    /// real start error when there is one, else the nudge to Preferences.
    func testAStoppedEngineReadsAsBefore() {
        XCTAssertEqual(
            PerformanceView.idleCaption(side: .a, running: false, mode: .timecode, awaitingMicrophone: false),
            "ENGINE STOPPED")
        XCTAssertEqual(
            PerformanceView.idleHint(
                side: .a, running: false, mode: .timecode, awaitingMicrophone: false, lastError: "boom"),
            "boom")
        XCTAssertTrue(
            PerformanceView.idleHint(
                side: .a, running: false, mode: .timecode, awaitingMicrophone: false, lastError: nil)
                .contains("Preferences"))
    }

    /// A running engine never shows the wait, even if a rip's prompt from
    /// Prep is still open.
    func testARunningEngineShowsItsOwnState() {
        XCTAssertEqual(
            PerformanceView.idleCaption(side: .a, running: true, mode: .prep, awaitingMicrophone: true),
            "NO TRACK LOADED")
    }
}
