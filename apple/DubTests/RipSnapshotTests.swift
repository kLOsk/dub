//
//  RipSnapshotTests.swift
//  DubTests
//
//  M26a — snapshot coverage for the vinyl-rip chrome: the Prep rip
//  bar states, the review/encode panel, the per-segment card, and
//  the split-marker overlay over a synthetic bucket field. All the
//  rip views are pure functions of value structs (no FFI), so the
//  suite renders them without the engine.
//
//  First run records new references and reports them as failures;
//  the committed PNGs make subsequent runs pass (same contract as
//  PerformanceSnapshotTests).
//

import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

final class RipSnapshotTests: XCTestCase {

    /// Render a SwiftUI view at a fixed size to a deterministic PNG.
    /// (Mirrors `PerformanceSnapshotTests.snap` — kept local because
    /// the helper is deliberately file-private there.)
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
        v.padding(DubSpacing.md).background(DubColor.surface0)
    }

    // MARK: - PrepRipBar

    func test_prepRipBar_idle() {
        snap(deckBg(PrepRipBar(state: PrepRipBarState(phase: .idle))),
             width: 720, height: 64, named: "idle")
    }

    /// M26b: armed and waiting for the needle. The meter is live so a
    /// silent input is visibly silent before the trigger ever fires.
    func test_prepRipBar_armed() {
        let state = PrepRipBarState(phase: .armed, levelPeak: 0.02)
        snap(deckBg(PrepRipBar(state: state)),
             width: 720, height: 64, named: "armed")
    }

    func test_prepRipBar_recording() {
        let state = PrepRipBarState(
            phase: .recording, elapsedSecs: 754, levelPeak: 0.72)
        snap(deckBg(PrepRipBar(state: state)),
             width: 720, height: 64, named: "recording")
    }

    func test_prepRipBar_recording_clipping() {
        let state = PrepRipBarState(
            phase: .recording, elapsedSecs: 61, levelPeak: 0.995)
        snap(deckBg(PrepRipBar(state: state)),
             width: 720, height: 64, named: "recording-clip")
    }

    func test_prepRipBar_failed() {
        let state = PrepRipBarState(
            phase: .failed,
            errorMessage: "Input lost — the interface was disconnected.")
        snap(deckBg(PrepRipBar(state: state)),
             width: 720, height: 64, named: "failed")
    }

    // MARK: - RipRecoveryBanner (M26b)

    func test_ripRecoveryBanner_interrupted() {
        let state = RipRecoveryBannerState(recordedSecs: 1_324, wasInterrupted: true)
        snap(deckBg(RipRecoveryBanner(state: state)),
             width: 720, height: 56, named: "recovery-interrupted")
    }

    func test_ripRecoveryBanner_abandonedWithOthers() {
        let state = RipRecoveryBannerState(
            recordedSecs: 812, wasInterrupted: false, others: 2)
        snap(deckBg(RipRecoveryBanner(state: state)),
             width: 720, height: 56, named: "recovery-abandoned")
    }

    // MARK: - RipReviewPanel

    private var threeSegments: [RipSegmentUi] {
        [
            RipSegmentUi(index: 0, startSecs: 0, endSecs: 452,
                         title: "King Tubby Meets Rockers Uptown",
                         artist: "Augustus Pablo",
                         album: "King Tubbys Meets Rockers Uptown",
                         genre: "Dub", year: "1976"),
            RipSegmentUi(index: 1, startSecs: 452, endSecs: 878,
                         title: "Stop Them Jah",
                         artist: "Augustus Pablo",
                         album: "", genre: "Dub", year: ""),
            RipSegmentUi(index: 2, startSecs: 878, endSecs: 1361),
        ]
    }

    func test_ripReviewPanel_threeSegments() {
        let state = RipReviewPanelState(
            mode: .review,
            sideDurationSecs: 1361,
            segments: threeSegments)
        snap(deckBg(RipReviewPanel(state: state)),
             width: 900, height: 260, named: "review-3-segments")
    }

    func test_ripReviewPanel_encodingOneFailed() {
        let state = RipReviewPanelState(
            mode: .failed,
            sideDurationSecs: 1361,
            segments: threeSegments,
            jobDots: [.done, .failed, .done],
            overallStatus: "Track 2 failed: encoder error.")
        snap(deckBg(RipReviewPanel(state: state)),
             width: 900, height: 260, named: "encoding-one-failed")
    }

    func test_ripReviewPanel_encodingRunning() {
        let state = RipReviewPanelState(
            mode: .encoding,
            sideDurationSecs: 1361,
            segments: threeSegments,
            jobDots: [.running, .running, .running],
            overallStatus: "Encoding 3 tracks…")
        snap(deckBg(RipReviewPanel(state: state)),
             width: 900, height: 260, named: "encoding-running")
    }

    func test_ripReviewPanel_done() {
        let state = RipReviewPanelState(
            mode: .done,
            sideDurationSecs: 1361,
            segments: threeSegments,
            jobDots: [.done, .done, .done])
        snap(deckBg(RipReviewPanel(state: state)),
             width: 900, height: 260, named: "done")
    }

    // MARK: - RipSegmentCard

    func test_ripSegmentCard_empty() {
        let segment = RipSegmentUi(index: 1, startSecs: 452, endSecs: 878)
        snap(deckBg(RipSegmentCard(segment: segment)),
             width: 280, height: 220, named: "empty")
    }

    func test_ripSegmentCard_filled() {
        let segment = RipSegmentUi(
            index: 0, startSecs: 0, endSecs: 452,
            title: "King Tubby Meets Rockers Uptown",
            artist: "Augustus Pablo",
            album: "King Tubbys Meets Rockers Uptown",
            genre: "Dub", year: "1976")
        snap(deckBg(RipSegmentCard(segment: segment)),
             width: 280, height: 220, named: "filled")
    }

    // MARK: - Split-marker overlay

    /// Deterministic synthetic side envelope so the overlay is
    /// judged against a realistic bucket field.
    private var syntheticBuckets: [OverviewBucket] {
        (0..<480).map { i in
            let t = Float(i) / 480
            let a = 0.25 + 0.6 * abs(sin(t * 23)) * (0.5 + 0.5 * t)
            return OverviewBucket(peak: a, rms: a * 0.7)
        }
    }

    private func overlayField(selected: UInt32?) -> some View {
        ZStack {
            RipLiveOverviewCanvas(buckets: syntheticBuckets)
            RipSplitMarkerOverlay(
                markers: [
                    RipMarkerUi(id: 1, secs: 452),
                    RipMarkerUi(id: 2, secs: 878),
                ],
                durationSecs: 1361,
                initialSelectedId: selected)
        }
        .frame(height: DubLayout.deckOverviewHeight)
    }

    func test_ripSplitOverlay_twoMarkers() {
        snap(overlayField(selected: nil),
             width: 900, height: DubLayout.deckOverviewHeight,
             named: "two-markers")
    }

    func test_ripSplitOverlay_selected() {
        snap(overlayField(selected: 2),
             width: 900, height: DubLayout.deckOverviewHeight,
             named: "selected")
    }
}
