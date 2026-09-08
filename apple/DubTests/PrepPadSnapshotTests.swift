import SnapshotTesting
import SwiftUI
import XCTest

@testable import Dub

/// Baselines for the Prep pad bar, recorded **before** the surface is
/// touched.
///
/// `CURRENT.md` named the gap and the reason: `SirenExpertPanel` took
/// `WaveformAppModel` directly, so nothing under `PrepPadGrid` could be
/// constructed in a test and Prep shipped with no pixel coverage at all —
/// while Performance's pad column has had baselines since C-31. The panel
/// is value-driven now, so the whole bar is reachable.
///
/// These exist to make the next changes reviewable. The contrast package
/// alters `DubPadCell` itself, which is every control on this surface; the
/// rack overhaul replaces the arrangement. Both should land as an image
/// diff someone can look at, not as a claim.
///
/// Widths are the real Prep tokens (`prepTransportColumn` 344,
/// `prepFxColumnMin` 288), so a section that stops fitting its column
/// fails here rather than in the app.
@MainActor
final class PrepPadSnapshotTests: XCTestCase {

    private func snap(
        _ view: some View,
        width: CGFloat,
        height: CGFloat,
        named name: String,
        file: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line
    ) {
        let sized = view
            .frame(width: width, height: height, alignment: .topLeading)
            .background(DubColor.surface0)
        let host = NSHostingView(rootView: sized)
        host.frame = CGRect(x: 0, y: 0, width: width, height: height)
        host.layoutSubtreeIfNeeded()
        assertSnapshot(
            of: host, as: .image(perceptualPrecision: 0.98), named: name,
            file: file, testName: testName, line: line)
    }

    // MARK: - Transport column (the 344 pt column, PRD §3.1 prep tooling)

    /// Resting state: no cues set, no loop, echo idle. This is what the DJ
    /// actually opens Prep to, and the state the contrast package is aimed
    /// at — every control here is currently `textTertiary` on `surface1`.
    func test_prepTransport_idle() {
        snap(
            VStack(alignment: .leading, spacing: DubSpacing.lg) {
                CuePadSection(cues: [nil, nil, nil, nil], onCue: { _, _ in })
                LoopPadSection(
                    activeBars: nil, loopEngaged: false, loopInArmed: false,
                    onLoop: { _ in }, onLoopIn: {}, onLoopOut: {}, onExit: {})
                EchoPadSection(engaged: false, onToggle: {})
            },
            width: DubLayout.prepTransportColumn, height: 210,
            named: "prep-transport-idle")
    }

    /// Everything lit at once: two cues set, a 2-bar loop running, echo
    /// engaged. Pins the lit/unlit contrast the redesign turns on.
    func test_prepTransport_engaged() {
        snap(
            VStack(alignment: .leading, spacing: DubSpacing.lg) {
                CuePadSection(cues: [0, 102.4, nil, nil], onCue: { _, _ in })
                LoopPadSection(
                    activeBars: 2, loopEngaged: true, loopInArmed: false,
                    onLoop: { _ in }, onLoopIn: {}, onLoopOut: {}, onExit: {})
                EchoPadSection(engaged: true, onToggle: {})
            },
            width: DubLayout.prepTransportColumn, height: 210,
            named: "prep-transport-engaged")
    }

    /// `IN` taken and `OUT` armed — the one state where a disabled pad sits
    /// beside an enabled one, which is what the caller-side `.opacity(0.5)`
    /// currently makes indistinguishable.
    func test_prepTransport_loopInArmed() {
        snap(
            LoopPadSection(
                activeBars: nil, loopEngaged: false, loopInArmed: true,
                onLoop: { _ in }, onLoopIn: {}, onLoopOut: {}, onExit: {}),
            width: DubLayout.prepTransportColumn, height: 70,
            named: "prep-loop-in-armed")
    }

    // MARK: - FX column (the 288 pt column)

    /// GS1's eight presets over two rows, plus the DUB slider. This is the
    /// baseline that carries the truncation — `ELECTRIC GUN` is scaled to
    /// about 7.7 pt inside a 64 pt pad here.
    func test_prepSiren_gs1() {
        snap(
            SirenPadRow(
                names: [
                    "Rifle Gun", "Rifle Echo", "Alarm", "Dual Tone",
                    "Bomb 1", "Bomb 2", "Electric Gun", "Electric Gun 2",
                ],
                sounding: false,
                onPreset: { _ in },
                dubMacro: 0,
                unit: .gs1),
            width: DubLayout.prepFxColumnMin, height: 168,
            named: "prep-siren-gs1")
    }

    /// DS01E publishes four presets where GS1 has eight, so the row halves
    /// and every pad moves. Pinned deliberately: the redesign proposes a
    /// fixed eight-slot bay precisely so this stops happening.
    func test_prepSiren_ds01e_isShorter() {
        snap(
            SirenPadRow(
                names: ["Siren", "Horn", "Whistle", "Zap"],
                sounding: true,
                onPreset: { _ in },
                dubMacro: 0.62,
                unit: .ds01e),
            width: DubLayout.prepFxColumnMin, height: 128,
            named: "prep-siren-ds01e")
    }

    /// Collapsed — the default, and all most sessions ever see of it.
    func test_prepSirenExpert_collapsed() {
        snap(
            SirenExpertPanel(deck: DeckState()),
            width: DubLayout.prepFxColumnMin, height: 28,
            named: "prep-expert-collapsed")
    }

    /// Expanded on GS1: the shared PT2399 echo section plus SPEED.
    func test_prepSirenExpert_gs1Expanded() {
        var deck = DeckState()
        deck.sirenExpertShown = true
        deck.sirenUnit = .gs1
        snap(
            SirenExpertPanel(deck: deck),
            width: DubLayout.prepFxColumnMin, height: 244,
            named: "prep-expert-gs1")
    }

    /// Expanded on DS01E, which adds the PITCH switch, RATE and HOLD — the
    /// tallest this panel ever gets, and the case `prepPadBarMinHeight`'s
    /// doc comment is worried about.
    func test_prepSirenExpert_ds01eExpanded() {
        var deck = DeckState()
        deck.sirenExpertShown = true
        deck.sirenUnit = .ds01e
        snap(
            SirenExpertPanel(deck: deck),
            width: DubLayout.prepFxColumnMin, height: 320,
            named: "prep-expert-ds01e")
    }
}
