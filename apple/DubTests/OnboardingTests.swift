//
//  OnboardingTests.swift
//  DubTests
//
//  The welcome guide's rules (reworked 2026-11-17): which steps a rig
//  gets, what the signal check calls an input, and how its rows look.
//

@testable import Dub
import SnapshotTesting
import SwiftUI
import XCTest

final class OnboardingTests: XCTestCase {

    /// Performance gets the audio and signal steps; Prep, with no
    /// interface to route or hear, skips straight to the music.
    func testStepsFollowTheRig() {
        XCTAssertEqual(
            OnboardingStep.steps(performance: true),
            [.rig, .audio, .signal, .music, .keys, .ready])
        XCTAssertEqual(OnboardingStep.steps(performance: false), [.rig, .music, .keys, .ready])
    }

    /// Green means the wiring is proven: timecode arriving, or locked.
    /// Silence, music and a closed input are not.
    func testSignalCheck() {
        let loud: Float = 0.1
        XCTAssertEqual(DeckSignalCheck.of(hasInput: false, rms: loud, sourceClass: 1, lockState: 1), .notOpen)
        XCTAssertEqual(DeckSignalCheck.of(hasInput: true, rms: 0, sourceClass: 1, lockState: 0), .nothing)
        XCTAssertEqual(DeckSignalCheck.of(hasInput: true, rms: 1e-4, sourceClass: 1, lockState: 0), .nothing,
                       "below the meter's first segment is silence")
        XCTAssertEqual(DeckSignalCheck.of(hasInput: true, rms: loud, sourceClass: 0, lockState: 0), .signal)
        XCTAssertEqual(DeckSignalCheck.of(hasInput: true, rms: loud, sourceClass: 2, lockState: 0), .music)
        XCTAssertEqual(DeckSignalCheck.of(hasInput: true, rms: loud, sourceClass: 1, lockState: 0), .timecode)
        XCTAssertEqual(DeckSignalCheck.of(hasInput: true, rms: loud, sourceClass: 1, lockState: 1), .locked)
        XCTAssertEqual(
            ([.notOpen, .nothing, .signal, .music, .timecode, .locked] as [DeckSignalCheck]).map(\.isGood),
            [false, false, false, false, true, true] as [Bool])
    }

    /// Every state of the signal row and the checklist line, one image.
    func testSignalRowsAndStatusLines() {
        let view = VStack(alignment: .leading, spacing: DubSpacing.sm) {
            ForEach(
                [DeckSignalCheck.notOpen, .nothing, .signal, .music, .timecode, .locked],
                id: \.self
            ) { check in
                OnboardingSignalRow(side: check == .locked ? .b : .a, check: check)
            }
            OnboardingStatusLine(
                state: .good, title: "Dub knows the Audio 10 — routing is ready",
                detail: "Each deck's inputs and outputs are already set.")
            OnboardingStatusLine(
                state: .attention, title: "macOS will ask for microphone access",
                detail: "Allow it, or the decks hear nothing.")
            OnboardingStatusLine(
                state: .neutral, title: "No DJ interface — Prep", detail: "Prepare tracks now.")
            OnboardingSwitchLine(position: "THRU", detail: "A real record on the platter plays straight through.")
        }
        .padding(DubSpacing.lg)
        .frame(width: 560, alignment: .topLeading)
        .background(DubColor.surface0)
        .environment(\.colorScheme, .dark)

        let host = NSHostingView(rootView: view)
        host.frame = CGRect(x: 0, y: 0, width: 560, height: 600)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: host, as: .image(perceptualPrecision: 0.98), named: "onboarding-signal-and-status")
    }
}
