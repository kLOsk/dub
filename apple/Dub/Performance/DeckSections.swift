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
            // `fixedSize` on both texts: an `HStack` offers a `Text`
            // less than it wants before it squeezes the rule, because a
            // `Text` can truncate and a line cannot — so at the echo's
            // 96 pt the heading read "E… ○…" over a full-length rule.
            // The words are the heading; the rule takes what is left.
            Text(title)
                .font(DubFont.caps)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(accent)
                .fixedSize()
            Rectangle()
                .fill(DubColor.divider)
                .frame(height: 1)
            if let trailing {
                Text(trailing)
                    .font(DubFont.micro)
                    .tracking(DubFont.capsTracking)
                    .foregroundStyle(trailingAccent ?? DubColor.textTertiary)
                    .fixedSize()
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
    /// How many columns to split the bank across.
    ///
    /// Fixed, not adaptive. This was briefly a `ViewThatFits` ladder
    /// picking the widest arrangement that fitted — and `ViewThatFits`
    /// *builds every candidate* to measure it, so the bank rendered its
    /// eight rows two or three times over on every layout pass. Inside
    /// a column whose own width is negotiated against the waveform,
    /// that measurement never settled and the main thread pinned a core
    /// with the app idle.
    var columns: Int = 4
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

    /// Rows needed to hold every slot at `columns` per row, rounded up
    /// so the last row is the short one.
    private func rowCount(_ columns: Int) -> Int {
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
            grid(columns: columns)
        }
    }

    /// Row-major: 1-4 across the top, 5-8 across the bottom — the
    /// same shape as a hardware pad bank, which is the arrangement the
    /// hand already knows. The bank was four rows of two and read down
    /// each column; at two rows of four, reading across is what matches
    /// the numbering, so the fill order flips with the shape.
    private func grid(columns: Int) -> some View {
        let cols = max(1, columns)
        return VStack(spacing: 2) {
            ForEach(0..<rowCount(cols), id: \.self) { line in
                HStack(spacing: DubSpacing.sm) {
                    ForEach(slots.filter { $0.index / cols == line }) { row($0) }
                }
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
                // The name and the spacer are both flexible, and at
                // four cells to a row the spacer was winning — cue
                // names truncated to a single character while blank
                // space sat next to them. The name is the reason the
                // cell exists, so it takes the slack first and the
                // spacer gets what is left over.
                .layoutPriority(1)
            Spacer(minLength: DubSpacing.xs)
            Text(slot.mark.map { CueTimecode.format($0.positionSecs) } ?? "—")
                .font(.system(size: 10.5, weight: .regular, design: .monospaced))
                .foregroundStyle(slot.isSet ? DubColor.textSecondary : DubColor.textPlaceholder)
                // The timecode is fixed-width and must never wrap; the
                // name is what gives way when the column is tight.
                .fixedSize()
        }
        .padding(.trailing, DubSpacing.sm)
        // Four to a row now, so a cell takes an equal share of the
        // width instead of claiming a column minimum. The name is the
        // part that gives way; number, colour bar and timecode are
        // fixed and always readable.
        .frame(minWidth: DubLayout.cueCellMinWidth, maxWidth: .infinity)
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

/// Loop lengths in **beats**, ascending, windowed four at a time.
///
/// Ascending left-to-right: shorter on the left, longer on the right,
/// which is the direction `÷2` and `×2` sit and the direction every
/// hardware loop control runs.
///
/// The two steppers do not move a window — they **resize the running
/// loop**, keeping its start, so `÷2` gives you the first half of what
/// is currently looping rather than a new loop somewhere else. The four
/// visible buttons follow whatever length is engaged, so the neighbours
/// you would reach next are always the ones on screen.
///
/// Pressing the lit length again exits the loop. A loop you engaged with
/// one press should not need a different control to leave.
struct LoopEngine: View {
    let activeBeats: Double?
    let engaged: Bool
    let hasTrack: Bool
    /// Height of the row. The steppers and the size buttons fill it
    /// edge to edge — there is no box around them any more. There
    /// was: a stroked frame with the buttons inset 8 pt inside it,
    /// which put a 28 pt control beside the 44 pt echo button and
    /// read as two sizes of thing however exactly the frames matched.
    /// The buttons are their own container now, at the echo's height.
    var height: CGFloat = DubLayout.deckColumnLoopHeight
    let onLoop: (Double) -> Void
    let onScale: (_ double: Bool) -> Void
    let onExit: () -> Void

    static let sizes: [Double] = [0.125, 0.25, 0.5, 1, 2, 4, 8, 16]
    static let windowSize = 4

    /// Compact: the loop shares its row with the SCRATCH pads and the
    /// echo, and the three have to clear the column's floor together
    /// (`test_deckColumn_floorHoldsTheTriggerRow`). 34 / 40 were the
    /// sizes when the loop had the row to itself.
    private static let stepperWidth: CGFloat = 28
    private static let sizeButtonMinWidth: CGFloat = 32

    /// The narrowest the row lays out at: both steppers, the four size
    /// buttons at their floor, and the gaps between. The size buttons
    /// have no ceiling — the row takes whatever width the column hands
    /// it, up to the echo button beside it — so this is the only
    /// number the column's floor has to clear.
    static let minWidth: CGFloat =
        stepperWidth * 2 + sizeButtonMinWidth * CGFloat(windowSize) + DubSpacing.sm * 2
    /// Where the window sits with nothing engaged: 1 · 2 · 4 · 8, the
    /// lengths a DJ reaches for first.
    private static let idleStart = 3

    /// Sit the window under the engaged length so its neighbours are
    /// reachable without a stepper press. An even window has no true
    /// centre: `idx - windowSize / 2` puts the running length third of
    /// four, which leaves two *shorter* lengths on its left. That is
    /// the direction loop work actually travels — engage 4, then halve
    /// into a build — so the asymmetry falls the useful way.
    /// Where the window sits while nothing is engaged. `nil` is the
    /// default position; the browse arrows move it.
    ///
    /// View state rather than model state on purpose: which four
    /// lengths you are *looking at* before you commit to one is a
    /// property of looking, not of the deck. Engaging a loop puts the
    /// window back under the running length, so the browse position
    /// never survives to confuse the next glance.
    @State private var browseStart: Int?

    private static var lastStart: Int { sizes.count - windowSize }

    private var windowStart: Int {
        guard let active = activeBeats,
              let idx = Self.sizes.firstIndex(where: { abs($0 - active) < 1e-9 })
        else { return min(max(browseStart ?? Self.idleStart, 0), Self.lastStart) }
        return min(max(idx - Self.windowSize / 2, 0), Self.sizes.count - Self.windowSize)
    }

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            stepper("÷2", double: false)
            sizeButtons
            stepper("×2", double: true)
        }
        .frame(height: height)
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
                    // Flexible both ways. Echo out sits beside the loop
                    // at every width, so the pair has to clear the
                    // column's floor and the size buttons are what
                    // gives; above it the row runs the whole way to the
                    // echo and the four lengths share the width.
                    .frame(minWidth: Self.sizeButtonMinWidth, maxWidth: .infinity)
                    .frame(maxHeight: .infinity)
                    // Unlit lengths get a fill of their own — the
                    // group used to be bare inside the box, so the
                    // three sizes read as text on the surface while the
                    // steppers beside them read as buttons.
                    .background(on ? DubColor.loop.opacity(0.26) : DubColor.surface3)
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
    /// Two controls in one slot.
    ///
    /// **Engaged** it is `÷2` / `×2` — it resizes the running loop,
    /// keeping its start. **Idle** it is `‹` / `›` — it walks the window
    /// along the ladder, so you can bring the length you want under
    /// your finger *before* committing to it. Idle, the halve/double
    /// pair had nothing to act on and sat dead; this is the same slot
    /// doing the job the state actually allows.
    private func stepper(_ glyph: String, double: Bool) -> some View {
        let next = activeBeats.map { double ? $0 * 2 : $0 / 2 }
        let resizable = engaged && next.map { $0 >= 0.125 - 1e-9 && $0 <= 16 + 1e-9 } ?? false
        let start = windowStart
        let browsable = !engaged && (double ? start < Self.lastStart : start > 0)
        let enabled = resizable || browsable
        return Text(engaged ? glyph : (double ? "›" : "‹"))
            .font(.system(
                size: engaged ? 13 : 15, weight: .medium,
                design: engaged ? .monospaced : .rounded))
            .foregroundStyle(enabled ? DubColor.textSecondary : DubColor.textPlaceholder)
            // Fills the row rather than a fixed 64: the row runs at
            // the echo button's height, and a stepper taller than it
            // hangs out of both ends.
            .frame(width: Self.stepperWidth)
            .frame(maxHeight: .infinity)
            .background(DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(DubColor.divider, lineWidth: 1))
            .contentShape(Rectangle())
            .onPressDown(enabled: enabled) {
                if engaged {
                    onScale(double)
                } else {
                    browseStart = min(max(start + (double ? 1 : -1), 0), Self.lastStart)
                }
            }
            .help(engaged
                ? (double
                    ? "Double the running loop"
                    : "Halve the running loop, keeping its start")
                : (double ? "Longer lengths" : "Shorter lengths"))
    }

    /// `1/8` rather than `0.125` — a DJ reads loop sizes as fractions.
    static func label(_ beats: Double) -> String {
        if beats >= 1 { return String(Int(beats)) }
        return "1/\(Int((1 / beats).rounded()))"
    }
}

