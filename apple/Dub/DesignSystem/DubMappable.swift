//
//  DubMappable.swift
//  Dub
//
//  Map mode over the live interface (M18): turn MAP on, click a control,
//  press a key. Not a Preferences screen — the controls are mapped where
//  they are played, so both surfaces have it by construction.
//
//  A control opts in with `.mappable(action)`. Off, the modifier is
//  nothing. With map mode on, the control's own press is switched off and
//  the control draws its cap and a ring; a click arms it — the ring goes
//  solid and says PRESS A KEY — and `KeyEventMonitorHost` hands the next
//  key to the store. Escape backs out; ⌫ leaves the control unbound.
//
//  The mode travels down the view tree as an environment value rather
//  than as state on every pad, so the value-driven views stay
//  value-driven and a snapshot can set it directly.
//

import SwiftUI

/// Map mode's presence in the view tree: `nil` when it is off.
struct DubMapping {
    /// The control waiting for a key.
    var armed: DubAction?
    /// The revision of the map the legends came from — bumped on every
    /// rebind so a control re-reads its cap.
    var revision: Int = 0
    /// Click on a control: arm it.
    var arm: (DubAction) -> Void = { _ in }
    /// Whether the armed ring pulses. Off for a snapshot, which needs one
    /// frame to be the frame.
    var motion: Bool = true
}

private struct DubMappingKey: EnvironmentKey {
    static let defaultValue: DubMapping? = nil
}

extension EnvironmentValues {
    var dubMapping: DubMapping? {
        get { self[DubMappingKey.self] }
        set { self[DubMappingKey.self] = newValue }
    }
}

extension View {
    /// Mark a control as bindable in map mode. Outside map mode this
    /// changes nothing about the view.
    func mappable(_ action: DubAction) -> some View {
        modifier(DubMappableModifier(action: action))
    }
}

/// The ring, the cap and the armed pulse.
private struct DubMappableModifier: ViewModifier {
    let action: DubAction
    @Environment(\.dubMapping) private var mapping
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulse = false

    func body(content: Content) -> some View {
        if let mapping {
            let armed = mapping.armed == action
            let legend = DubKeymap.legend(for: action)
            let pulsing = armed && mapping.motion && !reduceMotion
            content
                // The control's own press is off: a click means "map me".
                .allowsHitTesting(false)
                .overlay {
                    RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                        .strokeBorder(
                            DubColor.controlAccent.opacity(
                                armed ? (pulsing && !pulse ? 0.45 : 1) : 0.55),
                            style: StrokeStyle(lineWidth: armed ? 2 : 1, dash: armed ? [] : [4, 3]))
                        .animation(
                            pulsing
                                ? .easeInOut(duration: 0.5).repeatForever(autoreverses: true)
                                : .default,
                            value: pulse)
                }
                .overlay(alignment: .topLeading) {
                    capBadge(legend: legend, armed: armed)
                        .padding(2)
                }
                .contentShape(Rectangle())
                .onPressDown { mapping.arm(action) }
                .onAppear { pulse = pulsing }
                .onChange(of: pulsing) { now in pulse = now }
                .help(armed
                      ? "Press the key for this control — Escape cancels, ⌫ unbinds"
                      : "Click to map this control to a key")
                .accessibilityAddTraits(.isButton)
                .accessibilityLabel(armed ? "Waiting for a key" : "Map to a key")
        } else {
            content
        }
    }

    /// The cap the control prints now, or the ask while it waits.
    @ViewBuilder
    private func capBadge(legend: String?, armed: Bool) -> some View {
        if armed {
            Text("PRESS A KEY")
                .font(.system(size: 8, weight: .bold))
                .tracking(0.6)
                .lineLimit(1)
                .fixedSize()
                .foregroundStyle(DubColor.surface0)
                .padding(.horizontal, 4)
                .padding(.vertical, 2)
                .background(DubColor.controlAccent)
                .clipShape(RoundedRectangle(cornerRadius: 3, style: .continuous))
        } else {
            DubKeycap(key: legend ?? "", bound: legend != nil)
        }
    }
}
