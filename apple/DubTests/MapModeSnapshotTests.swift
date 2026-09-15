//
//  MapModeSnapshotTests.swift
//  DubTests
//
//  Map mode (M18): MAP on, every bindable control draws its cap and a
//  ring, the clicked one pulses and asks for a key. The mode reaches the
//  value-driven views as an environment value, which is what a snapshot
//  sets here — no model, no key events.
//
//  First run records new references and reports them as failures; the
//  committed PNGs make subsequent runs pass. Re-record intentionally
//  with `record: true` on a specific assertion.
//

import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

final class MapModeSnapshotTests: XCTestCase {

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

    /// The bar with MAP on and the siren's LASER key armed: the four other
    /// keys and every loaded tile ring dashed with their caps (two bound,
    /// the rest `—`), LASER solid and asking. `motion` is off so the
    /// armed ring holds one phase.
    func test_rackBar_mapMode_sirenKeyArmed() {
        var state = GlobalRackBarState.fixture(focus: .a)
        state.siren?.legends = ["Z", "", "", "", "B"]
        state.sampler.slots[0].legend = "A"
        let mapping = DubMapping(armed: .sirenPreset(3), revision: 1, motion: false)
        snap(
            GlobalRackBar(state: state)
                .background(DubColor.surface0)
                .environment(\.dubMapping, mapping),
            width: 1440, height: DubLayout.rackBarHeight,
            named: "rack-bar-map-armed")
    }

    /// Map mode off, keys bound: the caps print on the siren keys and the
    /// tile, and nothing else about the bar changes.
    func test_rackBar_boundCaps_mapModeOff() {
        var state = GlobalRackBarState.fixture(focus: .a)
        state.siren?.legends = ["Z", "X", "C", "V", "B"]
        state.sampler.slots[0].legend = "A"
        state.sampler.slots[1].legend = "S"
        snap(
            GlobalRackBar(state: state).background(DubColor.surface0),
            width: 1440, height: DubLayout.rackBarHeight,
            named: "rack-bar-bound-caps")
    }

    /// The status strip's MAP button, off and on.
    func test_statusStrip_mapButton() {
        let base = StatusStripState(
            engineVersion: "0.0.1", sampleRate: 48_000, isRunning: true,
            clockText: "21:47", power: PowerState(isCharging: true, percent: 87),
            lastError: nil)
        var on = base
        on.mapMode = true
        snap(
            VStack(spacing: 1) {
                StatusStrip(state: base, openPreferences: nil, openAbout: nil)
                StatusStrip(state: on, openPreferences: nil, openAbout: nil)
            },
            width: 900, height: DubLayout.statusStripHeight * 2 + 1,
            named: "map-button-off-on")
    }
}
