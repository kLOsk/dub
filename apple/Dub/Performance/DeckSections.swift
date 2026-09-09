//
//  DeckSections.swift
//  Dub
//
//  The control sections both surfaces draw.
//
//  These began inside `PrepRack` as private types, on the assumption
//  that Prep's surface was its own thing. It is not: Performance's
//  redesign puts the same marks and the same loop in its deck column,
//  and a hot cue has to look and behave identically wherever the DJ
//  meets it. PRD §3.1 — the `cueGroup()` / `CuePadRow` duplication was
//  removed for exactly this reason; re-creating it one surface later
//  would be the same mistake with a different filename.
//
//  What differs between the surfaces is *arrangement*, not component:
//  Prep gives the bank two fixed columns, Performance picks a count
//  from the width it was handed. So the column count is a parameter and
//  nothing else is.
//

import AppKit
import SwiftUI

// MARK: - State

/// One cue pad, as the bank draws it.
struct CueSlotState: Equatable, Identifiable {
    let index: Int
    /// `nil` is an empty pad.
    var mark: CueMark?
    var id: Int { index }

    var isSet: Bool { mark != nil }
}

/// The one thing the three sections do share: how a section announces
/// itself. A caps label and a hairline, nothing boxed.
struct SectionHeading: View {
    let title: String
    let accent: Color
    var trailing: String?
    var trailingAccent: Color?

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text(title)
                .font(DubFont.caps)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(accent)
            Rectangle()
                .fill(DubColor.divider)
                .frame(height: 1)
            if let trailing {
                Text(trailing)
                    .font(DubFont.micro)
                    .tracking(DubFont.capsTracking)
                    .foregroundStyle(trailingAccent ?? DubColor.textTertiary)
            }
        }
        .frame(height: 14)
    }
}

// MARK: - CUE — a list of marks

/// Four rows, unboxed. A cue is a *named position*, so a row carries the
/// name and the timecode with a colour spine down its leading edge; the
/// pad number is a small monospaced index, not the content.
///
/// An empty row is a dashed outline that says what a click will do. The
/// old surface drew a numbered square, which tells you nothing about
/// whether the pad holds anything or what pressing it does.
struct CueRowBank: View {
    let slots: [CueSlotState]
    let hasTrack: Bool
    let isPlaying: Bool
    let onCue: (Int, Bool) -> Void
    let onPreviewDown: (Int) -> Void
    let onPreviewUp: () -> Void
    let onRename: (Int) -> Void
    let onColor: (Int, String?) -> Void
    /// How many columns to split the bank across, or `nil` to pick the
    /// widest arrangement that fits. Prep fixes it at two so the
    /// section stays level with its neighbours; Performance's column
    /// has no neighbour and its width varies with the window.
    var columns: Int? = 2
    /// Height of one row, or `nil` to divide `contentHeight` between
    /// them. Prep divides; Performance sets it, because a row sized by
    /// its own text lands at about 18 pt — legible, but under any
    /// reasonable mouse target.
    var rowHeight: CGFloat?
    /// Pin the bank to a height, or let the rows size themselves.
    /// Prep pins it so the three sections stay level with one another;
    /// Performance's column has no neighbour to match, and a row that
    /// divides a fixed height gets shorter every time a cue is added.
    var contentHeight: CGFloat?

    private var setCount: Int { slots.filter(\.isSet).count }

    /// Rows per column, rounded up so the last column is the short one.
    private func perColumn(_ columns: Int) -> Int {
        max(1, Int((Double(slots.count) / Double(max(1, columns))).rounded(.up)))
    }

    private func helpText(_ slot: CueSlotState) -> String {
        let n = slot.index + 1
        guard slot.isSet else { return "Hot cue \(n) — click to set at the playhead" }
        return isPlaying
            ? "Hot cue \(n) — click to jump, ⇧-click to clear"
            : "Hot cue \(n) — hold to preview, ⇧-click to clear"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            SectionHeading(
                title: "HOTCUE", accent: DubColor.hotCue,
                trailing: "\(setCount) OF \(slots.count)")
            if let columns {
                grid(columns: columns)
            } else {
                // The heading stays *outside* this — its hairline is a
                // `Rectangle`, which has an unbounded ideal width, and
                // `ViewThatFits` compares ideal sizes. With the heading
                // inside a candidate, every rung reports infinity, none
                // of them "fits", and the ladder silently falls through
                // to its last option — one column, however much room
                // there is. The same greedy `Rectangle` stretched LOOP
                // across Prep's whole surface once already.
                // Two columns is the widest arrangement, not three.
                // Eight cues split three ways gives a 3/3/2 grid, and a
                // ragged last column reads as a mistake; two columns of
                // four stay a rectangle at any width, and the extra
                // room goes to the names instead.
                ViewThatFits(in: .horizontal) {
                    grid(columns: 2)
                    grid(columns: 1)
                }
            }
        }
    }

    /// Column-major: 1-4 down the first column, 5-8 down the second.
    /// Reading order follows the number, which is what the DJ is
    /// looking for — the grid is an arrangement, not a sequence, so
    /// row-major would put cue 2 where the eye expects cue 5.
    private func grid(columns: Int) -> some View {
        let rows = perColumn(columns)
        return HStack(alignment: .top, spacing: DubSpacing.sm) {
            ForEach(0..<max(1, columns), id: \.self) { column in
                VStack(spacing: 2) {
                    ForEach(slots.filter { $0.index / rows == column }) { row($0) }
                }
                .frame(minWidth: DubLayout.cueRowMinWidth, alignment: .leading)
            }
        }
        .frame(height: contentHeight)
    }

    @ViewBuilder
    private func row(_ slot: CueSlotState) -> some View {
        let tint = DubColor.trackLabel(slot.mark?.color) ?? DubColor.hotCue
        HStack(spacing: DubSpacing.sm) {
            Rectangle()
                .fill(slot.isSet ? tint : DubColor.divider)
                .frame(width: 3)
            Text("\(slot.index + 1)")
                .font(.system(size: 12, weight: .bold, design: .monospaced))
                .foregroundStyle(slot.isSet ? tint : DubColor.textPlaceholder)
                .frame(width: 10)
            // "set at playhead" is an *instruction*, so it belongs only
            // on an empty slot. A set-but-unnamed cue — the common case,
            // since most are dropped mid-listen — was reading it back as
            // if that were its name. It shows its timecode instead, and
            // the name column stays empty until the DJ types one.
            Text(slot.isSet
                ? (slot.mark?.name ?? "")
                : (hasTrack ? "set at playhead" : "load a track"))
                .font(.system(size: 12, weight: slot.isSet ? .semibold : .regular))
                .foregroundStyle(slot.isSet ? DubColor.textPrimary : DubColor.textPlaceholder)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: DubSpacing.sm)
            Text(slot.mark.map { CueTimecode.format($0.positionSecs) } ?? "—")
                .font(.system(size: 10.5, weight: .regular, design: .monospaced))
                .foregroundStyle(slot.isSet ? DubColor.textSecondary : DubColor.textPlaceholder)
                // The timecode is fixed-width and must never wrap; the
                // name is what gives way when the column is tight.
                .fixedSize()
        }
        .padding(.trailing, DubSpacing.sm)
        // Either a set height or a share of `contentHeight` — see
        // `rowHeight`. Prep divides, so its four rows stay level with
        // the sections beside them.
        .frame(height: rowHeight)
        .frame(maxHeight: rowHeight == nil ? .infinity : nil)
        .background(slot.isSet ? DubColor.surface2 : Color.clear)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .strokeBorder(
                    slot.isSet ? DubColor.divider : DubColor.divider.opacity(0.7),
                    style: StrokeStyle(lineWidth: 1, dash: slot.isSet ? [] : [3, 3])))
        .contentShape(Rectangle())
        // A set cue on a *paused* deck previews while held — the CDJ
        // gesture. Everything else (setting an empty pad, ⇧-clearing,
        // jumping on a running deck) stays mouse-down, because a cue
        // handler has to capture the playhead at the press.
        .modifier(
            CueRowGesture(
                previewable: hasTrack && slot.isSet && !isPlaying,
                enabled: hasTrack,
                onDown: { onPreviewDown(slot.index) },
                onUp: onPreviewUp,
                onClick: { onCue(slot.index, NSEvent.modifierFlags.contains(.shift)) }))
        .help(helpText(slot))
        .contextMenu {
            if slot.isSet {
                Button("Rename…") { onRename(slot.index) }
                Menu("Colour") {
                    Button("None") { onColor(slot.index, nil) }
                    ForEach(Array(DubColor.trackLabelPalette.enumerated()), id: \.offset) {
                        _, entry in
                        Button(entry.token.capitalized) { onColor(slot.index, entry.token) }
                    }
                }
                Divider()
                Button("Clear") { onCue(slot.index, true) }
            }
        }
    }

}

/// Routes a cue row to press-and-hold or to mouse-down. One or the
/// other — a view carrying both would have the hold gesture swallow the
/// click that sets an empty pad.
struct CueRowGesture: ViewModifier {
    let previewable: Bool
    let enabled: Bool
    let onDown: () -> Void
    let onUp: () -> Void
    let onClick: () -> Void

    func body(content: Content) -> some View {
        if previewable {
            content.onPressHold(onDown: onDown, onUp: onUp)
        } else {
            content.onPressDown(enabled: enabled, perform: onClick)
        }
    }
}

// MARK: - LOOP — a size selector

/// Loop lengths in **beats**, ascending, windowed three at a time.
///
/// Ascending left-to-right: shorter on the left, longer on the right,
/// which is the direction `÷2` and `×2` sit and the direction every
/// hardware loop control runs.
///
/// The two steppers do not move a window — they **resize the running
/// loop**, keeping its start, so `÷2` gives you the first half of what
/// is currently looping rather than a new loop somewhere else. The three
/// visible buttons follow whatever length is engaged, so the neighbours
/// you would reach next are always the ones on screen.
///
/// Pressing the lit length again exits the loop. A loop you engaged with
/// one press should not need a different control to leave.
struct LoopEngine: View {
    let activeBeats: Double?
    let engaged: Bool
    let hasTrack: Bool
    /// Draw the section heading, or leave it to the caller. Prep's
    /// rack wants the heading inside; Performance pairs LOOP with ECHO
    /// side by side and heads both from outside so the two hairlines
    /// line up.
    var showsHeading: Bool = true
    /// Outer height of the boxed control, padding included. Prep gives
    /// it `prepSectionContent` so the section stays level with its
    /// neighbours; Performance's column has no neighbour to match and a
    /// shorter box buys height the cue rows want.
    ///
    /// The buttons inset from it by `DubSpacing.sm` on each edge. They
    /// used to be a fixed 64 inside an 88 box, which gave the same air
    /// by accident; equalising the box to the echo button's height took
    /// that away and left the stroke sitting on the buttons.
    var contentHeight: CGFloat = DubLayout.prepSectionContent
    let onLoop: (Double) -> Void
    let onScale: (_ double: Bool) -> Void
    let onExit: () -> Void

    static let sizes: [Double] = [0.125, 0.25, 0.5, 1, 2, 4, 8, 16]
    static let windowSize = 3
    /// Where the window sits with nothing engaged: 1 · 2 · 4, the
    /// lengths a DJ reaches for first.
    private static let idleStart = 3

    /// Centre the window on the engaged length so both neighbours are
    /// reachable without a stepper press.
    private var windowStart: Int {
        guard let active = activeBeats,
              let idx = Self.sizes.firstIndex(where: { abs($0 - active) < 1e-9 })
        else { return Self.idleStart }
        return min(max(idx - Self.windowSize / 2, 0), Self.sizes.count - Self.windowSize)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            if showsHeading {
                SectionHeading(
                    title: "LOOP", accent: DubColor.loop,
                    trailing: engaged ? "● ACTIVE" : "○ IDLE",
                    trailingAccent: engaged ? DubColor.loop : DubColor.textPlaceholder)
            }

            // A box, but an empty one. LOOP is the only section that
            // is a single instrument rather than a set of slots, and it
            // needs an edge to say so. What it did not need was a
            // *fill* behind that edge as well: the controls inside are
            // filled, the box around them is a stroke on the bare
            // surface, and the section stops reading as a slab.
            HStack(spacing: DubSpacing.sm) {
                stepper("÷2", double: false)
                sizeButtons
                stepper("×2", double: true)
            }
            .frame(height: contentHeight - DubSpacing.sm * 2)
            .padding(.horizontal, DubSpacing.md)
            .padding(.vertical, DubSpacing.sm)
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.card, style: .continuous)
                    .stroke(DubColor.divider, lineWidth: 1))
        }
    }

    private var sizeButtons: some View {
        let start = windowStart
        return HStack(spacing: 0) {
            ForEach(0..<Self.windowSize, id: \.self) { offset in
                let index = start + offset
                let beats = Self.sizes[index]
                let on = engaged && (activeBeats.map { abs($0 - beats) < 1e-9 } ?? false)
                Text(Self.label(beats))
                    .font(.system(size: 15, weight: .semibold, design: .rounded))
                    .foregroundStyle(on ? DubColor.textPrimary : DubColor.textSecondary)
                    // Flexible, not fixed. Performance keeps echo out
                    // beside the loop at every width, so the pair has
                    // to clear the column's floor — the size buttons
                    // are what gives. Prep hands the control its full
                    // width and they render at 52 as before.
                    .frame(minWidth: 40, maxWidth: 52)
                    .frame(maxHeight: .infinity)
                    .background(on ? DubColor.loop.opacity(0.26) : Color.clear)
                    .overlay(alignment: .trailing) {
                        if offset < Self.windowSize - 1 {
                            Rectangle().fill(DubColor.divider).frame(width: 1)
                        }
                    }
                    .contentShape(Rectangle())
                    .onPressDown(enabled: hasTrack) {
                        if on { onExit() } else { onLoop(beats) }
                    }
                    .help(on
                        ? "Looping \(Self.label(beats)) — press again to exit"
                        : "Loop the last \(Self.label(beats)) beat"
                            + (beats == 1 ? "" : "s"))
            }
        }
        .background(DubColor.surface0)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .stroke(DubColor.divider, lineWidth: 1))
    }

    /// Resizes the *running* loop. Dead without one — there is nothing
    /// to halve — and dead at the ends of the range.
    private func stepper(_ glyph: String, double: Bool) -> some View {
        let next = activeBeats.map { double ? $0 * 2 : $0 / 2 }
        let enabled = engaged && next.map { $0 >= 0.125 - 1e-9 && $0 <= 16 + 1e-9 } ?? false
        return Text(glyph)
            .font(.system(size: 12, weight: .medium, design: .monospaced))
            .foregroundStyle(enabled ? DubColor.textSecondary : DubColor.textPlaceholder)
            // Fills the box rather than a fixed 64: Performance runs
            // this control at the echo button's height, and a stepper
            // taller than its own box hangs out of both ends of it.
            .frame(width: 34)
            .frame(maxHeight: .infinity)
            .background(DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(DubColor.divider, lineWidth: 1))
            .contentShape(Rectangle())
            .onPressDown(enabled: enabled) { onScale(double) }
            .help(double
                ? "Double the running loop"
                : "Halve the running loop, keeping its start")
    }

    /// `1/8` rather than `0.125` — a DJ reads loop sizes as fractions.
    static func label(_ beats: Double) -> String {
        if beats >= 1 { return String(Int(beats)) }
        return "1/\(Int((1 / beats).rounded()))"
    }
}

