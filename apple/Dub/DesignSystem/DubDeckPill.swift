//
//  DubDeckPill.swift
//  Dub
//
//  `→ A` / `→ B` / `→ A+B`: the deck(s) a global rack lands on.
//
//  The siren drew this privately first; the sampler needs the same
//  statement on its own header, and two hand-rolled copies of a pill is
//  how the pads ended up with five recipes for one rounded rect.
//
//  Not clickable — it *is* right-clickable. Focus follows the master
//  deck, and a click that set it would be a competing notion of the
//  same thing; the override lives in a menu because it is a rule you
//  change once, not a control you play. A pinned rack draws filled so
//  "set" and "following" read as different states across the booth.
//

import SwiftUI

/// Names the deck(s) a rack's pads fire on, and takes the override.
struct DubDeckPill: View {
    let state: RackOutputState
    /// Right-click → Auto · A · B · A+B. `nil` draws a plain pill with
    /// no menu (a snapshot, a preview).
    var onSelect: ((RackOutput) -> Void)?

    private var tint: Color {
        state.tintDeck.map(DubColor.deckTint) ?? DubColor.textSecondary
    }

    var body: some View {
        Text(state.label)
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(state.isPinned ? DubColor.textPrimary : tint)
            .padding(.horizontal, DubSpacing.xs)
            .padding(.vertical, 1)
            .background(state.isPinned ? tint.opacity(0.28) : Color.clear)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(tint.opacity(state.isPinned ? 1 : 0.6), lineWidth: 1))
            .animation(.easeOut(duration: 0.12), value: state)
            .contentShape(Rectangle())
            .onSecondaryClick {
                guard let onSelect else { return .handled }
                return .menu(RackOutput.allCases.map { option in
                    SecondaryMenuItem(option.menuTitle, checked: state.output == option) {
                        onSelect(option)
                    }
                })
            }
    }
}
