//
//  SirenRackGroup.swift
//  Dub
//
//  The siren block of the global rack bar, drawn as the thing a dub DJ
//  owns: a black box on top of the mixer with a display window, five
//  keys and one knob.
//
//  Before this it was eight name-over-keycap pads, a unit switch and a
//  stock slider — the sample shelf's tiles in a different tint, beside
//  the sample shelf. With the bank cut to five shots and the unit
//  selector gone (one flat bank, each shot routed to its chip by the
//  engine), the block became the one thing on the surface that is
//  framed, the one thing with a meter, and the one thing with a dial.
//  Cover the labels and it is still the FX unit.
//
//  One rack, with the deck it fires on named on its face: a single
//  siren, and `KeyEventMonitorHost` dispatches `.sirenPreset` to the
//  rack's output once map mode binds it — so a cap drawn here is a cap
//  that fires, or nothing.
//

import DubCore
import SwiftUI

/// Keyboard legends for the siren shots, in fire order — all empty until
/// map mode binds them; the caps then print whatever the DJ picked. One
/// row because there is one keymap — the handler resolves the deck at
/// press time, not the key.
let sirenPresetKeys = DubKeymap.legends({ .sirenPreset($0) }, count: 5)

/// The siren box: display · keys · DUB knob on a faceplate, under the
/// bar's shared heading.
struct SirenRackGroup: View {
    let state: SirenRackState
    var onPreset: (_ index: Int) -> Void = { _ in }
    var onDubMacro: (_ value: Double) -> Void = { _ in }
    var onOutput: ((_ output: RackOutput) -> Void)?

    private static let plateInset: CGFloat = 12
    private static let plateGap: CGFloat = 14
    private static let keyGap: CGFloat = 6

    /// The plate's width, summed from its parts. Pinned, because the
    /// heading's rule is flexible and would otherwise make the block's
    /// ideal width unbounded — the bar then hands the siren every spare
    /// point and squeezes the shelf to its floor.
    static let width: CGFloat =
        plateInset * 2 + DubLayout.sirenDisplayWidth + plateGap
        + DubLayout.sirenKeyWidth * 5 + keyGap * 4 + plateGap
        + DubLayout.sirenKnobCellWidth

    /// What the knob has set — the same curve the engine applies, so the
    /// readout and the sound cannot disagree.
    private var dub: SirenDubControls {
        sirenDubMacroControls(macroValue: Float(state.dubMacro))
    }

    private var lastShotName: String? {
        state.lastShot.flatMap { state.presetNames.indices.contains($0) ? state.presetNames[$0] : nil }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            header
            plate
        }
        .frame(width: Self.width, alignment: .leading)
    }

    /// The same heading the shelf and the deck sections use — title,
    /// rule, state — so the bar's two blocks read as one family. The
    /// trailing names the shot while it sounds rather than saying ON.
    private var header: some View {
        HStack(spacing: DubSpacing.sm) {
            SectionHeading(
                title: "DUB SIREN", accent: DubColor.siren,
                trailing: state.sounding ? "● \((lastShotName ?? "ON").uppercased())" : "○ OFF",
                trailingAccent: state.sounding ? DubColor.siren : DubColor.textPlaceholder)
            DubDeckPill(state: state.output, onSelect: onOutput)
                .help("Where the siren fires — the master deck by default. "
                    + "Right-click to pin it to A, B or both.")
        }
    }

    private var plate: some View {
        HStack(spacing: Self.plateGap) {
            SirenDisplay(
                shot: lastShotName, sounding: state.sounding, fireCount: state.fireCount,
                echoOn: state.dubMacro > 0, delayMs: dub.delayMs, feedback: dub.feedback)
            HStack(spacing: Self.keyGap) {
                ForEach(Array(state.presetNames.enumerated()), id: \.offset) { index, name in
                    key(index: index, name: name)
                }
            }
            knobCell
        }
        .padding(.horizontal, Self.plateInset)
        .padding(.vertical, DubSpacing.sm)
        .frame(width: Self.width, height: DubLayout.sirenPlateHeight)
        // The deck's own ground (`DeckColumn`, `DeckHeader`): the box is
        // a piece of the same rig, one step below the bar it sits on,
        // not a hole punched through to the window.
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: 3, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 3, style: .continuous)
                .stroke(DubColor.plateEdge, lineWidth: 1))
    }

    @ViewBuilder
    private func key(index: Int, name: String) -> some View {
        let legend = index < sirenPresetKeys.count ? sirenPresetKeys[index] : ""
        DubKey(
            title: name, legend: legend,
            down: state.sounding && state.lastShot == index)
            .onPressDown { onPreset(index) }
            .help(legend.isEmpty ? "Fire \(name)" : "Fire \(name) (\(legend))")
    }

    /// The DUB knob. The delay and feedback it has set are on the meter's
    /// echo line and in the tooltip; a readout under the knob was one
    /// number too many on the plate.
    private var knobCell: some View {
        DubKnob(value: state.dubMacro, low: "DRY", high: "DUB", onChange: onDubMacro)
            .help(state.dubMacro > 0
                ? "DUB — \(Int(dub.delayMs.rounded())) ms · \(Int((dub.feedback * 100).rounded())) % feedback. "
                    + "Slows and deepens the chip shots and adds the box's own echo, "
                    + "longer, louder and darker as it turns. Double-click for DRY."
                : "DUB — DRY: no echo at all. Turn it up to slow and deepen the chip "
                    + "shots and add the box's own echo. Double-click for DRY.")
            .frame(width: DubLayout.sirenKnobCellWidth)
    }
}
