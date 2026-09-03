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
        // Anti-aliasing is not reproducible across machines or OS
        // versions. Eight baselines here were failing with a *visually
        // identical* render — 310 differing bytes out of 1.25 MB, max
        // delta 16/255 — and were misdiagnosed as views that had moved.
        // `perceptualPrecision` tolerates that per-pixel noise; a real
        // layout change moves far more than this and still fails.
            of: host, as: .image(perceptualPrecision: 0.98), named: name,
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

    // Sizes come from the layout tokens, never from a hand-picked
    // number. The previous baseline rendered at 560 × 360 with the
    // struct's *defaults* — where the siren was off — so it was green
    // on a configuration nobody runs, while the real column needed
    // ~478 pt of a ~330 pt pane and spilled behind the FX bar. A
    // snapshot forced to `.frame(width:height:)` cannot catch an
    // overflow anyway; it just renders the smaller thing correctly.
    // `PerformanceLayoutTests` is what actually asserts fit.

    /// Deck A exactly as the 1440 × 900 surface lays it out: the column
    /// token wide, and as tall as the 0.60 split leaves the pane.
    func test_performancePads_deckA_production() {
        snap(deckBg(PerformancePadsView(side: .a, state: .shippingDefault)),
             width: DubLayout.performancePadColumnWidth, height: 398,
             named: "pads-deck-a-production")
    }

    /// The same column in the shortest pane the window can produce.
    func test_performancePads_deckA_minimumWindow() {
        snap(deckBg(PerformancePadsView(side: .a, state: .shippingDefault)),
             width: DubLayout.performancePadColumnWidth, height: 218,
             named: "pads-deck-a-minimum")
    }

    /// The dormant FX rack (UI-BACKLOG F-38) flipped on — the tallest
    /// the column can get, and the case that exercises the scroll
    /// fallback. Recorded now so the hidden path has a baseline before
    /// anyone un-hides it.
    func test_performancePads_deckB_rackEnabled() {
        var state = PerformancePadsState.shippingDefault
        state.rackEnabled = true
        snap(deckBg(PerformancePadsView(side: .b, state: state)),
             width: DubLayout.performancePadColumnWidth, height: 398,
             named: "pads-deck-b-rack-on")
    }

    // MARK: - Global rack bar

    func test_globalRackBar_focusDeckA() {
        snap(deckBg(GlobalRackBar(state: .fixture(focus: .a))),
             width: 1440, height: DubLayout.rackBarHeight,
             named: "rack-bar-focus-deck-a")
    }

    /// Same state, focus moved. Pins that the `→ B` pill and its tint
    /// follow the master deck.
    func test_globalRackBar_focusDeckB() {
        snap(deckBg(GlobalRackBar(state: .fixture(focus: .b))),
             width: 1440, height: DubLayout.rackBarHeight,
             named: "rack-bar-focus-deck-b")
    }

    /// The narrowest window the layout supports.
    func test_globalRackBar_narrow() {
        snap(deckBg(GlobalRackBar(state: .fixture(focus: .a))),
             width: DubLayout.mainWindowMinWidth, height: DubLayout.rackBarHeight,
             named: "rack-bar-narrow")
    }

    /// Prep, where the siren has its own column and the bar carries
    /// only the two trigger racks.
    func test_globalRackBar_noSiren() {
        var state = GlobalRackBarState.fixture(focus: .a)
        state.siren = nil
        snap(deckBg(GlobalRackBar(state: state)),
             width: 1440, height: DubLayout.rackBarHeight,
             named: "rack-bar-no-siren")
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

    func test_deckHeader_withSourceControl_thru() {
        // THRU: a live record straight through. The third switch
        // position, added when auto-detection was deferred in favour of
        // an explicit INT · TC · THRU choice (PRD §5.1.1). This replaced
        // a "· PINNED" case that no longer renders differently —
        // `SourceControlView.overridden` is documented dead, because
        // with an explicit switch every mode is pinned.
        let state = DeckHeaderState(
            isLive: true, source: .file,
            trackTitle: "Bow Down", trackArtist: "Westside Connection",
            bpm: 92.9, pitchPercent: -2.3, timecodeLockState: 1,
            sourceControl: .thru,
            sourceControlOverridden: false,
            key: "7A",
            formatChip: "MP3 · 44.1 kHz · stereo",
            timeRow: .remainingOnly,
            isMaster: true, isPlaying: true,
            isPanicPlay: false, useTimecodeToggle: false,
            gridLocked: false, gridDriftQuality: nil)
        snap(deckBg(DeckHeader(side: .a, state: state)),
             width: 720, height: 108, named: "with-source-control-thru")
    }

    // MARK: - Source control (Internal/Timecode switch + status)

    func test_sourceControl_allStates() {
        // One row per real state. `overridden` used to add a "· PINNED"
        // variant; the explicit switch retired it (every mode is pinned),
        // so a second `.timecode` row would render identically.
        let states: [(SourceControlStatus, Bool)] = [
            (.off, false), (.internalPlay, false), (.calibrating, false),
            (.timecode, false), (.thru, false),
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
