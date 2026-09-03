//
//  PadSections.swift
//  Dub
//
//  The transport pad sections — CUE, LOOP, ECHO OUT — shared by both
//  surfaces.
//
//  These existed twice: `PerformancePadsView.cueGroup()` / `loopGroup()`
//  with the label above the pads, and `CuePadRow` / `LoopPadRow` with
//  the label in a 44 pt inline gutter, near-identical bodies otherwise.
//  Same for `EchoPadRow` / `PrepEchoPadRow`. PRD §3.1 is explicit that
//  Prep and Performance are "parameterized variants of the same deck
//  surface, not parallel implementations", so the sections are shared
//  and each surface arranges them: Performance stacks them in one
//  narrow column, Prep spreads them across a three-column grid.
//
//  The label sits above the pads rather than beside them. That is what
//  buys the ~52 pt per row which makes Prep's three columns fit the
//  minimum window — and it lets every section align on one left edge
//  instead of two.
//

import AppKit
import SwiftUI

/// One reverse-loop length preset: a label and its length in bars.
struct LoopPreset: Identifiable {
    let bars: Double
    let label: String
    var id: Double { bars }
}

/// The four hot-cue pads. Click sets or jumps, ⇧-click clears.
struct CuePadSection: View {
    let cues: [Double?]
    let onCue: (_ index: Int, _ clear: Bool) -> Void

    var body: some View {
        DubSectionPanel("CUE") {
            HStack(spacing: DubSpacing.sm) {
                ForEach(0..<4, id: \.self) { index in
                    let isSet = index < cues.count && cues[index] != nil
                    DubPadCell("\(index + 1)", size: .glyph, lit: isSet)
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
}

/// Loop lengths plus the manual in / out / exit pads.
struct LoopPadSection: View {
    let activeBars: Double?
    let loopEngaged: Bool
    let loopInArmed: Bool
    /// Break the manual pads onto a second row. Performance's 224 pt
    /// column cannot hold all seven side by side (they need 338); Prep's
    /// 344 pt column can.
    var wrap: Bool = false
    let onLoop: (_ bars: Double) -> Void
    let onLoopIn: () -> Void
    let onLoopOut: () -> Void
    let onExit: () -> Void

    private static let presets: [LoopPreset] = [
        LoopPreset(bars: 0.5, label: "½"),
        LoopPreset(bars: 1, label: "1"),
        LoopPreset(bars: 2, label: "2"),
        LoopPreset(bars: 4, label: "4"),
    ]

    private var canExit: Bool { activeBars != nil || loopEngaged || loopInArmed }

    var body: some View {
        DubSectionPanel("LOOP") {
            if wrap {
                VStack(alignment: .leading, spacing: DubSpacing.sm) {
                    HStack(spacing: DubSpacing.sm) { lengths }
                    HStack(spacing: DubSpacing.sm) { manual }
                }
            } else {
                HStack(spacing: DubSpacing.sm) {
                    lengths
                    manual
                }
            }
        }
    }

    @ViewBuilder
    private var lengths: some View {
        ForEach(Self.presets) { preset in
            DubPadCell(
                preset.label, size: .glyph,
                lit: activeBars == preset.bars, tint: DubColor.loop)
                .onPressDown { onLoop(preset.bars) }
                .help("Loop the last \(preset.label) bar\(preset.bars == 1 ? "" : "s")")
        }
    }

    /// The escape hatch from the beat-length grab, for a track the
    /// analyser could not grid or one whose grid disagrees with the bar
    /// you want.
    @ViewBuilder
    private var manual: some View {
        DubPadCell("IN", size: .word, lit: loopInArmed, tint: DubColor.loop)
            .onPressDown { onLoopIn() }
            .help("Set the loop start at the playhead")
        DubPadCell("OUT", size: .word, tint: DubColor.loop)
            .onPressDown(enabled: loopInArmed) { onLoopOut() }
            .help("Close the loop at the playhead and start it")
            .opacity(loopInArmed ? 1.0 : 0.5)
        DubPadCell("✕", size: .glyph, tint: DubColor.loop)
            .onPressDown(enabled: canExit) { onExit() }
            .help("Exit loop")
            .opacity(canExit ? 1.0 : 0.5)
    }
}

/// The single 1-beat echo-out toggle (M15, PRD §6.3).
struct EchoPadSection: View {
    let engaged: Bool
    let onToggle: () -> Void

    var body: some View {
        DubSectionPanel("ECHO") {
            DubPadCell("ECHO OUT", size: .hugging, lit: engaged, tint: DubColor.echo)
                .onPressDown { onToggle() }
                .help("Echo out — 1 beat, 100% wet. Tap on, tap off.")
        }
    }
}
