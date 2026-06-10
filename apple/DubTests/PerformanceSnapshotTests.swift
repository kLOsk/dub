//
//  PerformanceSnapshotTests.swift
//  DubTests
//
//  C-31 — the first snapshot suite. Renders the app's internal SwiftUI
//  chrome to PNGs (in DubTests/__Snapshots__/) so visual changes are
//  reviewable and regression-checked without running the app. Covers
//  the deck header (states + the non-mirrored two-deck layout) and the
//  performance pads. Metal views (the waveform) can't snapshot and are
//  deliberately out of scope.
//
//  First run records new references and reports them as failures; the
//  committed PNGs make subsequent runs pass. Re-record intentionally
//  with `record: true` on a specific assertion.
//

import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

final class PerformanceSnapshotTests: XCTestCase {

    /// Render a SwiftUI view at a fixed size to a deterministic PNG.
    private func snap(
        _ view: some View,
        width: CGFloat,
        height: CGFloat,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        let sized = view.frame(width: width, height: height)
        let host = NSHostingView(rootView: sized)
        host.frame = CGRect(x: 0, y: 0, width: width, height: height)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: host, as: .image, named: name,
            file: file, testName: testName, line: line)
    }

    private func deckBg<V: View>(_ v: V) -> some View {
        v.background(DubColor.surface0)
    }

    // MARK: - Deck header states

    func test_deckHeader_idle() {
        snap(deckBg(DeckHeader(side: .a, state: .idle)),
             width: 720, height: 108, named: "idle")
    }

    func test_deckHeader_fileTimecodePlaying() {
        // A timecode-driven file deck: PITCH shown (live pitch), live
        // BPM, key, green tracking dot (lock state 1).
        let state = DeckHeaderState(
            isLive: true, source: .file,
            trackTitle: "Bow Down", trackArtist: "Westside Connection",
            bpm: 92.9, pitchPercent: -2.3, timecodeLockState: 1,
            key: "7A",
            formatChip: "MP3 · 44.1 kHz · stereo",
            timeRow: .remainingOnly,
            isMaster: true, isPlaying: true,
            isPanicPlay: false, useTimecodeToggle: false,
            gridLocked: false, gridDriftQuality: nil)
        snap(deckBg(DeckHeader(side: .a, state: state)),
             width: 720, height: 108, named: "file-timecode-playing")
    }

    func test_deckHeader_loading() {
        let state = DeckHeaderState(
            isLive: true, source: .loading,
            trackTitle: "Cheddar", trackArtist: nil,
            bpm: nil, key: nil,
            formatChip: nil, timeRow: nil,
            isMaster: false, isPlaying: false,
            isPanicPlay: false, useTimecodeToggle: false,
            gridLocked: false, gridDriftQuality: nil)
        snap(deckBg(DeckHeader(side: .b, state: state)),
             width: 720, height: 108, named: "loading")
    }

    /// The headline change to verify: deck A and deck B headers are
    /// **identical left-to-right** (not mirrored).
    func test_deckHeaders_twoDeck_notMirrored() {
        func state(_ title: String, _ artist: String, _ bpm: Double, _ key: String, _ pitch: Double) -> DeckHeaderState {
            DeckHeaderState(
                isLive: true, source: .file,
                trackTitle: title, trackArtist: artist,
                bpm: bpm, pitchPercent: pitch, timecodeLockState: 1,
                key: key,
                formatChip: "MP3 · 44.1 kHz · stereo",
                timeRow: .remainingOnly,
                isMaster: true, isPlaying: true,
                isPanicPlay: false, useTimecodeToggle: false,
                gridLocked: false, gridDriftQuality: nil)
        }
        let row = HStack(spacing: 1) {
            DeckHeader(side: .a, state: state("Bow Down", "Westside Connection", 92.9, "7A", -2.3))
            DeckHeader(side: .b, state: state("Cheddar", "WC feat. Cube", 93.0, "7B", 1.4))
        }
        snap(deckBg(row), width: 1440, height: 108, named: "two-deck-not-mirrored")
    }

    // MARK: - Performance pads

    func test_performancePads_deckA() {
        snap(deckBg(PerformancePadsView(side: .a)),
             width: 320, height: 360, named: "pads-deck-a")
    }

    func test_deckHeader_withSourceControl() {
        // Auto-selected TIMECODE: the row-3 switch carries the source +
        // tracking dot, so the row-1 FILE pill is suppressed (no
        // duplicate source chrome).
        let state = DeckHeaderState(
            isLive: true, source: .file,
            trackTitle: "Bow Down", trackArtist: "Westside Connection",
            bpm: 92.9, pitchPercent: -2.3, timecodeLockState: 1,
            sourceControl: .timecode,
            key: "7A",
            formatChip: "MP3 · 44.1 kHz · stereo",
            timeRow: .remainingOnly,
            isMaster: true, isPlaying: true,
            isPanicPlay: false, useTimecodeToggle: false,
            gridLocked: false, gridDriftQuality: nil)
        snap(deckBg(DeckHeader(side: .a, state: state)),
             width: 720, height: 108, named: "with-source-control-timecode")
    }

    func test_deckHeader_withSourceControl_pinned() {
        // User-pinned TIMECODE: the switch shows "· PINNED" (override),
        // distinguishing it from the auto-selected case above.
        let state = DeckHeaderState(
            isLive: true, source: .file,
            trackTitle: "Bow Down", trackArtist: "Westside Connection",
            bpm: 92.9, pitchPercent: -2.3, timecodeLockState: 1,
            sourceControl: .timecode,
            sourceControlOverridden: true,
            key: "7A",
            formatChip: "MP3 · 44.1 kHz · stereo",
            timeRow: .remainingOnly,
            isMaster: true, isPlaying: true,
            isPanicPlay: false, useTimecodeToggle: false,
            gridLocked: false, gridDriftQuality: nil)
        snap(deckBg(DeckHeader(side: .a, state: state)),
             width: 720, height: 108, named: "with-source-control-pinned")
    }

    // MARK: - Source control (Internal/Timecode switch + status)

    func test_sourceControl_allStates() {
        let states: [(SourceControlStatus, Bool)] = [
            (.internalPlay, false), (.detecting, false), (.calibrating, false),
            (.timecode, false), (.timecode, true), (.thru, false),
        ]
        let stack = VStack(alignment: .leading, spacing: 12) {
            ForEach(0..<states.count, id: \.self) { i in
                SourceControlView(status: states[i].0, overridden: states[i].1)
            }
        }
        .padding()
        snap(deckBg(stack), width: 440, height: 260, named: "source-control-states")
    }
}
