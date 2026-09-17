//
//  FxChannelSnapshotTests.swift
//  DubTests
//
//  The DUB FX channel (F-38 stage 2): the pane a deck becomes when its
//  source switch is flipped to DUB FX — the input lane on the inner edge,
//  the four-unit rack on the outer — and the rack bar re-ordered around
//  it. The scope is the Metal live waveform and cannot snapshot; a flat
//  placeholder stands in at its size.
//
//  First run records new references and reports them as failures; the
//  committed PNGs make subsequent runs pass. Re-record intentionally
//  with `record: true` on a specific assertion.
//

import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

final class FxChannelSnapshotTests: XCTestCase {

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
            of: host, as: .image(perceptualPrecision: 0.98), named: name,
            file: file, testName: testName, line: line)
    }

    /// The pane at the 1440 × 900 deck row: a 486 pt column beside the
    /// strip at its cap.
    private static let paneWidth: CGFloat = 486 + DubLayout.performanceWaveformWidthCap
    private static let paneHeight: CGFloat = 560

    /// Deck B on DUB FX with the send patched in: the Big Knob, the Space
    /// Echo and the spring in, the phaser bypassed, the VU reading a
    /// healthy send.
    private static func fixture(side: DeckSide = .b) -> FxChannelState {
        var s = FxChannelState()
        s.side = side
        s.input = .send
        s.inputPair = side == .a ? "1–2" : "3–4"
        s.trimDb = 6
        s.inputRms = 0.08
        s.inputPeak = 0.2
        s.active = [true, true, true, false]
        // Still: the tape, the lamps and the coils hold one phase.
        s.motion = false
        return s
    }

    private func pane(_ state: FxChannelState) -> some View {
        FxChannelPane(state: state, columnWidth: 486) {
            DubColor.waveformLane
        }
        .background(DubColor.surface0)
    }

    func test_fxChannel_deckB() {
        snap(pane(Self.fixture()),
             width: Self.paneWidth, height: Self.paneHeight, named: "deck-b")
    }

    /// Deck A mirrors: the rack on the outer (left) edge, the lane inner.
    func test_fxChannel_deckA() {
        snap(pane(Self.fixture(side: .a)),
             width: Self.paneWidth, height: Self.paneHeight, named: "deck-a")
    }

    /// Every unit bypassed, a mic straight in, the trim flat: the faces
    /// go grey and dim but every dial and number stays legible.
    func test_fxChannel_allBypassed_micIn() {
        var s = Self.fixture()
        s.active = [false, false, false, false]
        s.input = .mic
        s.trimDb = 0
        s.inputRms = 0.01
        s.inputPeak = 0.02
        snap(pane(s),
             width: Self.paneWidth, height: Self.paneHeight, named: "bypassed-mic")
    }

    /// The Space Echo past unity intensity and the Big Knob at its top
    /// detent, the send clipping: RUNAWAY, the 7 500 Hz stop, the needle
    /// in the red and LIVE reading HOT.
    func test_fxChannel_runaway_hotInput() {
        var s = Self.fixture()
        s.controls.spaceEchoIntensity = 1.15
        s.controls.spaceEchoMode = .short
        s.controls.bigKnobStep = 10
        s.inputRms = 0.5
        s.inputPeak = 0.98
        snap(pane(s),
             width: Self.paneWidth, height: Self.paneHeight, named: "runaway-hot")
    }

    /// The bar with deck B as the channel: samples left, the siren box
    /// right under the rack, its pill reading `→ FX` because the siren
    /// fires into the rack.
    func test_globalRackBar_fxCorner() {
        var state = GlobalRackBarState.fixture(focus: .b)
        state.fxSide = .b
        state.siren?.output = RackOutputState(.b, focused: .b, fxDeck: .b)
        state.sampler.output = RackOutputState(.a, focused: .b, fxDeck: .b)
        snap(GlobalRackBar(state: state).background(DubColor.surface0),
             width: 1440, height: DubLayout.rackBarHeight,
             named: "rack-bar-fx-corner")
    }

    /// Deck A as the channel: the one case the blocks swap, the siren
    /// box leading so it stays under the rack on the left.
    func test_globalRackBar_fxCornerLeft() {
        var state = GlobalRackBarState.fixture(focus: .a)
        state.fxSide = .a
        state.siren?.output = RackOutputState(.a, focused: .a, fxDeck: .a)
        state.sampler.output = RackOutputState(.b, focused: .a, fxDeck: .a)
        snap(GlobalRackBar(state: state).background(DubColor.surface0),
             width: 1440, height: DubLayout.rackBarHeight,
             named: "rack-bar-fx-corner-left")
    }
}
