//
//  TriggerPadGroup.swift
//  Dub
//
//  A row of momentary trigger pads — Quick Scratch or the sampler.
//
//  These pads used to be static chrome: `lit: false`, no callbacks, and
//  a hardcoded `Q W` / `E R` split across the two decks that
//  contradicted the model, where all four slots are global and each
//  carries its own target deck. They fire now, and the pad's tint says
//  which deck bus it lands on.
//
//  A momentary pad press is explicitly inside the PRD §1 mouse rule —
//  it is not a continuous performance gesture — so clicking one is
//  fine, and the keyboard is a convenience on top (§5.5).
//

import SwiftUI

/// One rack's four pads under a section label.
struct TriggerPadGroup: View {
    let title: String
    let pads: [TriggerPadState]
    let onTrigger: (_ index: Int) -> Void
    /// Right-click action, when the rack has one (the sampler stops a
    /// running voice). `nil` omits the context menu.
    var onSecondary: ((_ index: Int) -> Void)?
    /// Shown on a right-click menu item, when `onSecondary` is set.
    var secondaryTitle: String = "Stop"

    var body: some View {
        DubSectionPanel(title) {
            HStack(spacing: DubSpacing.sm) {
                ForEach(pads) { pad in
                    padCell(pad)
                }
            }
        }
    }

    @ViewBuilder
    private func padCell(_ pad: TriggerPadState) -> some View {
        let bound = pad.sampleName != nil
        DubPadCell(
            size: .preset,
            lit: bound,
            tint: DubColor.deckTint(pad.deck ?? .a)
        ) {
            VStack(spacing: 1) {
                Text(pad.sampleName ?? "—")
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                DubKeycap(key: pad.key, bound: pad.keyBound, lit: bound)
            }
        }
        .onPressDown(enabled: bound) { onTrigger(pad.index) }
        .opacity(bound ? 1.0 : 0.55)
        .help(helpText(pad))
        .contextMenu {
            if bound, let onSecondary {
                Button(secondaryTitle) { onSecondary(pad.index) }
            }
        }
    }

    private func helpText(_ pad: TriggerPadState) -> String {
        guard let name = pad.sampleName else {
            return "Empty slot — bind a sample in Preferences (⌘,)."
        }
        let deck = (pad.deck ?? .a) == .a ? "A" : "B"
        return pad.keyBound
            ? "\(name) → deck \(deck) (\(pad.key))"
            : "\(name) → deck \(deck). Click to fire; the \(pad.key) key "
                + "arrives with the key-remapping pass."
    }
}
