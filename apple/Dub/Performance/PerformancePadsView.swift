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
//  The CUE row is live: keys 1–4 set/recall a hot cue on the master
//  deck, Shift+key clears it (see `WaveformAppModel.handleHotCue`). A
//  pad lights in the deck's tint when its slot holds a position. The
//  rest are honest placeholders until the features land — Loops (M13),
//  Quick Scratch + Sampler (M17). They're laid out now so the surface
//  reads as a real performance instrument rather than empty space, and
//  so the final geometry is fixed before the controls become live.
//

import AppKit
import SwiftUI

struct PerformancePadsView: View {

    let side: DeckSide

    /// Hot cue positions (track seconds) for the four CUE pads; `nil`
    /// = empty slot. Drives which pads light. Fed from the deck's
    /// `hotCues`, set/recalled by the 1–4 keys.
    var cues: [Double?] = [nil, nil, nil, nil]

    /// Hug the deck: deck A's pads (window-left) sit against their
    /// overview on the right; deck B's (window-right) sit against
    /// their overview on the left.
    private var frameAlignment: Alignment { side == .a ? .trailing : .leading }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            cueGroup()
            padGroup("LOOP", keys: ["IN", "OUT", "½", "×2"])
            padGroup("QUICK SCRATCH", keys: side == .a ? ["Q", "W"] : ["E", "R"])
            padGroup("SAMPLER", keys: side == .a ? ["A", "S"] : ["D", "F"])
        }
        .padding(.horizontal, DubSpacing.xl)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: frameAlignment)
        .background(DubColor.surface0)
    }

    /// The live CUE row. No "soon" tag; pads light in the deck tint
    /// when set.
    @ViewBuilder
    private func cueGroup() -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            Text("CUE")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
            HStack(spacing: DubSpacing.sm) {
                ForEach(0..<4, id: \.self) { index in
                    pad("\(index + 1)", lit: index < cues.count && cues[index] != nil)
                }
            }
        }
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

    private func pad(_ glyph: String, lit: Bool = false) -> some View {
        hotCuePadCell(glyph, lit: lit)
    }
}

/// One pad cell — `glyph` centred, lit in `tint` when active. Shared
/// by the Performance pad panel, the Prep cue bar (magenta), and the
/// Prep loop bar (green), so a lit pad reads the same as its marker on
/// the waveform / overview.
@ViewBuilder
private func hotCuePadCell(_ glyph: String, lit: Bool, tint: Color = DubColor.hotCue)
    -> some View
{
    Text(glyph)
        .font(.system(size: 13, weight: .semibold, design: .rounded))
        .foregroundStyle(lit ? DubColor.textPrimary : DubColor.textTertiary)
        .frame(width: glyph.count > 1 ? 50 : 38, height: 36)
        .background(lit ? tint.opacity(0.24) : DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .stroke(lit ? tint : DubColor.divider, lineWidth: 1))
}

/// Horizontal, **clickable** hot cue pad row for Prep mode (where
/// the mouse is a first-class input, unlike Performance). Click an
/// empty pad to set a cue at the playhead, a set pad to jump to it,
/// ⇧-click to clear — mirroring the 1–4 / ⇧+1–4 keyboard gestures.
struct CuePadRow: View {

    let cues: [Double?]
    /// `(index, clear)` — `clear` is `true` when ⇧ is held at click.
    let onCue: (_ index: Int, _ clear: Bool) -> Void

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("CUE")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: PrepPadLayout.labelWidth, alignment: .leading)
            ForEach(0..<4, id: \.self) { index in
                let isSet = index < cues.count && cues[index] != nil
                hotCuePadCell("\(index + 1)", lit: isSet)
                    .onPressDown {
                        onCue(index, NSEvent.modifierFlags.contains(.shift))
                    }
                    .help(isSet
                        ? "Cue \(index + 1) — click to jump, ⇧-click to clear"
                        : "Cue \(index + 1) — click to set at the playhead")
            }
        }
    }
}

extension View {
    /// Fire `perform` on mouse-**down** (press), not mouse-up.
    ///
    /// Use for timing-sensitive taps — hot cues, the BPM tap — where
    /// the handler captures the live playhead (or a wall-clock tap
    /// timestamp) at the click instant. A SwiftUI `Button` fires its
    /// action on mouse-**up**, so on a playing deck the captured
    /// point lands the whole click-hold duration (tens of ms) too
    /// late; the keyboard path fires on key-down and is correct, so
    /// the two disagreed. Pressing down matches the keyboard and a
    /// real hardware pad (note-on = press). `enabled == false` makes
    /// it inert (left-click does nothing) while leaving any sibling
    /// `contextMenu` / right-click reachable.
    func onPressDown(enabled: Bool = true, perform: @escaping () -> Void) -> some View {
        modifier(PressDownModifier(enabled: enabled, perform: perform))
    }
}

private struct PressDownModifier: ViewModifier {
    let enabled: Bool
    let perform: () -> Void
    @State private var pressing = false

    func body(content: Content) -> some View {
        content
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in
                        guard enabled, !pressing else { return }
                        pressing = true
                        perform()
                    }
                    .onEnded { _ in pressing = false })
    }
}

/// Shared geometry for the Prep pad rows so the CUE and LOOP rows
/// line their pad columns up under a fixed-width label gutter.
private enum PrepPadLayout {
    static let labelWidth: CGFloat = 44
}

/// One reverse-loop length preset: a label and its length in bars.
private struct LoopPreset: Identifiable {
    let bars: Double
    let label: String
    var id: Double { bars }
}

/// Live LOOP pad row for Prep. Each length pad triggers a grid-snapped
/// **reverse** loop of that many bars — the bars just heard — on
/// mouse-down (like cues / transport). The active length lights green;
/// the ✕ pad exits the loop. Prep's loop role is *authoring + testing*
/// a region (PRD §3.1), so it's mouse-clickable, not keyboard-only.
struct LoopPadRow: View {

    /// Which length pad is lit (bars), or `nil` when no loop is active.
    let activeBars: Double?
    /// Trigger a reverse loop of `bars` bars.
    let onLoop: (_ bars: Double) -> Void
    /// Exit the active loop.
    let onExit: () -> Void

    private static let presets: [LoopPreset] = [
        LoopPreset(bars: 0.5, label: "½"),
        LoopPreset(bars: 1, label: "1"),
        LoopPreset(bars: 2, label: "2"),
        LoopPreset(bars: 4, label: "4"),
    ]

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("LOOP")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: PrepPadLayout.labelWidth, alignment: .leading)
            ForEach(Self.presets) { preset in
                hotCuePadCell(
                    preset.label,
                    lit: activeBars == preset.bars,
                    tint: DubColor.loop
                )
                .onPressDown { onLoop(preset.bars) }
                .help("Loop the last \(preset.label) bar\(preset.bars == 1 ? "" : "s")")
            }
            hotCuePadCell("✕", lit: false, tint: DubColor.loop)
                .onPressDown(enabled: activeBars != nil) { onExit() }
                .help("Exit loop")
                .opacity(activeBars == nil ? 0.5 : 1.0)
        }
    }
}

#Preview("Performance pads") {
    HStack(spacing: 1) {
        PerformancePadsView(side: .a, cues: [12.0, nil, 48.5, nil])
        PerformancePadsView(side: .b, cues: [nil, 4.0, nil, nil])
    }
    .frame(width: 800, height: 360)
    .background(DubColor.surface0)
}
