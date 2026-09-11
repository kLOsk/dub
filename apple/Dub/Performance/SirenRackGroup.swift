//
//  SirenRackGroup.swift
//  Dub
//
//  The siren block of the global rack bar: preset pads, unit selector,
//  and the DUB super-knob.
//
//  Previously this was drawn twice, once per deck, with the same eight
//  key caps on both. That was a lie about the keyboard: `sirenPresetKeys`
//  has no deck parameter and `KeyEventMonitorHost` dispatches to the
//  focused deck only, so pressing `Z` has always fired exactly one
//  siren. The non-focused copy was advertising keys that did nothing to
//  it. One rack, with the deck it fires on named on its face.
//

import DubCore
import SwiftUI

/// Keyboard bindings for the siren presets, in fire order. One row of
/// keys because there is one keymap — the handler resolves the deck at
/// press time, not the pad.
let sirenPresetKeys = DubKeymap.legends({ .sirenPreset($0) }, count: 8)

/// The siren: eight preset pads under a header carrying the unit
/// selector and the dub knob.
struct SirenRackGroup: View {
    let state: SirenRackState
    var onPreset: (_ index: Int) -> Void = { _ in }
    var onUnit: (_ unit: SirenUnit) -> Void = { _ in }
    var onDubMacro: (_ value: Double) -> Void = { _ in }
    var onOutput: ((_ output: RackOutput) -> Void)?

    private var tint: Color {
        state.output.tintDeck.map(DubColor.deckTint) ?? DubColor.controlAccent
    }

    /// The block's width: the eight preset pads and the gaps between
    /// them. Pinned, because the heading's rule is flexible and would
    /// otherwise make the block's ideal width unbounded — the bar then
    /// hands the siren every spare point and squeezes the shelf to its
    /// floor, which is exactly what the shelf's own floor was added to
    /// stop. The rule fills what the pill, the unit switch and the knob
    /// leave, to the pads' edge and no further.
    static let width: CGFloat =
        (DubPadCell<Text>.Size.preset.width ?? 64) * 8 + DubSpacing.sm * 7

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            header
            HStack(spacing: DubSpacing.sm) {
                ForEach(Array(state.presetNames.enumerated()), id: \.offset) { index, name in
                    presetPad(index: index, name: name)
                }
            }
        }
        .frame(width: Self.width, alignment: .leading)
    }

    /// The same heading the shelf and the deck sections use — title,
    /// rule, state — so the bar's two blocks read as one family. The
    /// rule takes what the pill, the unit switch and the knob leave.
    private var header: some View {
        HStack(spacing: DubSpacing.sm) {
            SectionHeading(
                title: "DUB SIREN", accent: DubColor.siren,
                trailing: state.sounding ? "● ON" : "○ OFF",
                trailingAccent: state.sounding ? DubColor.siren : DubColor.textPlaceholder)
            focusPill
            DubSegmentedControl(
                segments: [
                    .init(SirenUnit.gs1, "GS1"),
                    .init(SirenUnit.ds01e, "DS01E"),
                    .init(SirenUnit.sn76477, "SN76477"),
                ],
                selection: state.unit,
                tint: tint,
                onSelect: onUnit)
                .help("Siren unit — GS1 (toy-chip shots) · Benidub DS01E (analog) · SN76477 chip")
            dubKnob
        }
    }

    /// Names the deck(s) the pads and keys land on, and takes the
    /// override — see `DubDeckPill`.
    private var focusPill: some View {
        DubDeckPill(state: state.output, onSelect: onOutput)
            .help("Where the siren fires — the master deck by default. "
                + "Right-click to pin it to A, B or both. "
                + "\(sirenPresetKeys.prefix(4).joined(separator: " ")) … fire the same pads.")
    }

    private var dubKnob: some View {
        HStack(spacing: DubSpacing.xs) {
            Text("DUB")
                .font(DubFont.micro)
                .foregroundStyle(state.dubMacro > 0 ? DubColor.siren : DubColor.textTertiary)
            Slider(
                value: Binding(get: { state.dubMacro }, set: { onDubMacro($0) }),
                in: 0...1)
                .controlSize(.mini)
                .tint(DubColor.siren)
                .frame(minWidth: 60, maxWidth: 140)
                .help("Dub super-knob — one knob adds the siren's own echo "
                    + "(delay + feedback). 0 = dry.")
        }
    }

    @ViewBuilder
    private func presetPad(index: Int, name: String) -> some View {
        let key = index < sirenPresetKeys.count ? sirenPresetKeys[index] : ""
        DubPadCell(size: .preset) {
            VStack(spacing: 1) {
                Text(name.uppercased())
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                DubKeycap(key: key)
            }
        }
        .onPressDown { onPreset(index) }
        .help("Fire the \(name) siren (\(key))")
    }
}
