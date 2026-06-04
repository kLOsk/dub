//
//  Placeholders.swift
//  Dub
//
//  M10.3 placeholders for regions that future milestones will
//  populate with real content:
//
//  * `FXBarPlaceholder`   — split per-deck, lit by M15 (Echo-Out) /
//                            M16 (Dub Siren) / M17 (Sampler + Quick
//                            Scratch).
//
//  Each placeholder is a visible-but-honest dim block with a tiny
//  "Coming soon" caption. The goal is to make the unshipped slots
//  obvious to anyone looking at the running app (in DJ language, not
//  our milestone codes — see U-12 / U-22), and to make the layout
//  sizes wrong *now* if they would be wrong later. The milestone
//  that owes each slot is noted in code comments only.
//

import SwiftUI

/// Two-column FX bar placeholder — one column per deck — sized to
/// `DubLayout.fxBarHeight`. The real implementation arrives across
/// M15 / M16 / M17; until then we show the slot allocation so the
/// outer layout is finalised.
struct FXBarPlaceholder: View {
    var body: some View {
        HStack(spacing: 1) {
            deckColumn(.a)
            deckColumn(.b)
        }
        .frame(height: DubLayout.fxBarHeight)
        .background(DubColor.divider)
    }

    @ViewBuilder
    private func deckColumn(_ side: DeckSide) -> some View {
        HStack(spacing: DubSpacing.md) {
            modulePlaceholder("ECHO-OUT")   // M15
            modulePlaceholder("DUB SIREN")  // M16
            scratchPlaceholder(side: side)
            samplerPlaceholder(side: side)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, DubSpacing.lg)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(DubColor.surface2)
    }

    /// User-facing caption for an unshipped slot. Deliberately names
    /// the state in DJ language, never our milestone codes (U-12).
    private static let comingSoonCaption = "Coming soon"

    @ViewBuilder
    private func modulePlaceholder(_ label: String) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            Text(label)
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
            Text(Self.comingSoonCaption)
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textPlaceholder)
        }
        .padding(.horizontal, DubSpacing.md)
        .padding(.vertical, DubSpacing.sm)
        .frame(width: 96, height: 56, alignment: .topLeading)
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
    }

    @ViewBuilder
    private func scratchPlaceholder(side: DeckSide) -> some View {
        // Q/W for A, E/R for B (matches the Figma exploration and
        // PRD §7 Quick Scratch keymap intent — final binding lives
        // with M17).
        let keys = (side == .a) ? ["Q", "W"] : ["E", "R"]
        keyCapGroup(label: "SCRATCH", keys: keys)   // M17
    }

    @ViewBuilder
    private func samplerPlaceholder(side: DeckSide) -> some View {
        let keys = (side == .a) ? ["A", "S"] : ["D", "F"]
        keyCapGroup(label: "SAMPLER", keys: keys)   // M17
    }

    @ViewBuilder
    private func keyCapGroup(label: String, keys: [String]) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            HStack(spacing: DubSpacing.xs) {
                Text(label)
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
                Text(Self.comingSoonCaption)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textPlaceholder)
            }
            HStack(spacing: DubSpacing.xs) {
                ForEach(keys, id: \.self) { k in keyCap(k) }
            }
        }
        .padding(.horizontal, DubSpacing.sm)
        .padding(.vertical, DubSpacing.xs)
    }

    @ViewBuilder
    private func keyCap(_ glyph: String) -> some View {
        Text(glyph)
            .font(.system(size: 12, weight: .semibold, design: .monospaced))
            .foregroundStyle(DubColor.textTertiary)
            .frame(width: 28, height: 28)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
    }
}

#Preview("FX bar") {
    FXBarPlaceholder()
        .frame(width: 1440)
}
