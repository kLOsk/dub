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
let sirenPresetKeys = ["Z", "X", "C", "V", "B", "N", "M", ","]

/// The siren: eight preset pads under a header carrying the unit
/// selector and the dub knob.
struct SirenRackGroup: View {
    let state: SirenRackState
    var onPreset: (_ index: Int) -> Void = { _ in }
    var onUnit: (_ unit: SirenUnit) -> Void = { _ in }
    var onDubMacro: (_ value: Double) -> Void = { _ in }

    private var tint: Color { DubColor.deckTint(state.focusedDeck) }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            header
            HStack(spacing: DubSpacing.sm) {
                ForEach(Array(state.presetNames.enumerated()), id: \.offset) { index, name in
                    presetPad(index: index, name: name)
                }
            }
        }
    }

    private var header: some View {
        HStack(spacing: DubSpacing.sm) {
            DubSectionLabel("SIREN", dot: state.sounding ? DubColor.siren : DubColor.divider)
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

    /// Names the deck the pads and keys land on. Not clickable: focus
    /// follows the master deck, and a second way to set it would be a
    /// competing notion of the same thing. To fire the siren on the
    /// other deck, make that deck master — same as with hot cues.
    private var focusPill: some View {
        Text(state.focusedDeck == .a ? "→ A" : "→ B")
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(tint)
            .padding(.horizontal, DubSpacing.xs)
            .padding(.vertical, 1)
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(tint.opacity(0.6), lineWidth: 1))
            .animation(.easeOut(duration: 0.12), value: state.focusedDeck)
            .help("The siren fires on the focused deck — the master, or deck A "
                + "when neither is. \(sirenPresetKeys.prefix(4).joined(separator: " ")) … "
                + "fire the same pads.")
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
                Text(key)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textPlaceholder)
            }
        }
        .onPressDown { onPreset(index) }
        .help("Fire the \(name) siren (\(key))")
    }
}
