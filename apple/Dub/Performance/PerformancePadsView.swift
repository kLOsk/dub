//
//  PerformancePadsView.swift
//  Dub
//
//  Per-deck performance pads, occupying the outer space beside each
//  deck's waveform (the "whole lot of nothing" the centred-cluster
//  layout left behind). Modelled on Serato Scratch Live's pad area:
//  the running waveform + its overview sit toward the centre (next to
//  the phase clock), and the cue / loop / quick-scratch / sampler
//  controls live out here on the deck's outer edge.
//
//  These are honest placeholders until the features land — Loops
//  (M13), Quick Scratch + Sampler (M17), hot cues (a later
//  milestone). They're laid out now so the surface reads as a real
//  performance instrument rather than empty space, and so the final
//  geometry is fixed before the controls become live.
//

import SwiftUI

struct PerformancePadsView: View {

    let side: DeckSide

    /// Hug the deck: deck A's pads (window-left) sit against their
    /// overview on the right; deck B's (window-right) sit against
    /// their overview on the left.
    private var frameAlignment: Alignment { side == .a ? .trailing : .leading }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            padGroup("CUE", keys: ["1", "2", "3", "4"])
            padGroup("LOOP", keys: ["IN", "OUT", "½", "×2"])
            padGroup("QUICK SCRATCH", keys: side == .a ? ["Q", "W"] : ["E", "R"])
            padGroup("SAMPLER", keys: side == .a ? ["A", "S"] : ["D", "F"])
        }
        .padding(.horizontal, DubSpacing.xl)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: frameAlignment)
        .background(DubColor.surface0)
    }

    @ViewBuilder
    private func padGroup(_ label: String, keys: [String]) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            HStack(spacing: DubSpacing.sm) {
                Text(label)
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
                Text("soon")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textPlaceholder)
            }
            HStack(spacing: DubSpacing.sm) {
                ForEach(keys, id: \.self) { pad($0) }
            }
        }
    }

    private func pad(_ glyph: String) -> some View {
        Text(glyph)
            .font(.system(size: 13, weight: .semibold, design: .rounded))
            .foregroundStyle(DubColor.textTertiary)
            .frame(width: glyph.count > 1 ? 50 : 38, height: 36)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(DubColor.divider, lineWidth: 1))
    }
}

#Preview("Performance pads") {
    HStack(spacing: 1) {
        PerformancePadsView(side: .a)
        PerformancePadsView(side: .b)
    }
    .frame(width: 800, height: 360)
    .background(DubColor.surface0)
}
