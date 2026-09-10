//
//  DubDeckPill.swift
//  Dub
//
//  `→ A` / `→ B`: the deck a global rack lands on.
//
//  The siren drew this privately first; the sampler needs the same
//  statement on its own header, and two hand-rolled copies of a pill is
//  how the pads ended up with five recipes for one rounded rect.
//

import SwiftUI

/// Names the deck a rack's pads fire on. Not clickable: focus follows
/// the master deck, and a second way to set it would be a competing
/// notion of the same thing. To fire on the other deck, make that deck
/// master — same as with hot cues.
struct DubDeckPill: View {
    let deck: DeckSide

    private var tint: Color { DubColor.deckTint(deck) }

    var body: some View {
        Text(deck == .a ? "→ A" : "→ B")
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(tint)
            .padding(.horizontal, DubSpacing.xs)
            .padding(.vertical, 1)
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(tint.opacity(0.6), lineWidth: 1))
            .animation(.easeOut(duration: 0.12), value: deck)
    }
}
