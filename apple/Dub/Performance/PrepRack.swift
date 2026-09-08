//
//  PrepRack.swift
//  Dub
//
//  Prep's control surface.
//
//  ## These are three different things, and they are drawn as three
//  different things
//
//  The first attempt at this file gave CUE, LOOP and SAMPLES a shared
//  module wrapper — same border, same header row, same padding — which is
//  the flat-hierarchy mistake one level up from where it started: the
//  contents differed and the containers made them look alike again. There
//  is no `RackModule` here on purpose. Each section owns its own chrome,
//  its own placement and its own idea of what an item is:
//
//  * **CUE** is a *list of marks*. Unboxed — the rows are the objects, and
//    a frame around them would just be a second border inside a pad bar
//    inside a deck pane. Full width of its column, four rows deep.
//  * **LOOP** is an *instrument*. Boxed, recessed, panel-like, built
//    around a numeral in a well. It is the only thing here that looks like
//    a unit, because it is the only thing here that behaves like one.
//  * **SAMPLES** is a *drop target over a list*. Its border is dashed when
//    empty because that is what a drop zone looks like, and solid once it
//    holds something. Neither of the other two has a state where its own
//    outline changes.
//
//  Cover the labels: a list, a panel, and a drop zone are still three
//  recognisable objects. That is the test this file has to keep passing.
//
//  ## Why only these three
//
//  Prep is couch work with no rig attached. The FX are not here — echo
//  out, the siren and its Expert panel, Quick Scratch and the vintage rack
//  are things you do to a record playing in front of people. Prep owns
//  preparing marks and getting sounds into the bank.
//
//  There is **no beatgrid editor**, and there is not going to be one.
//  Setting the 1 — the deck-header BPM tap, which re-anchors the grid to
//  the visible kick — plus Analyze covers what a DJ actually does to a
//  grid. The six FFI calls behind a full editor stay unmounted rather than
//  becoming a surface nobody asked for. Prep is these three sections.
//
//  ## Values in, closures out
//
//  No section takes the model. That is what makes the surface
//  snapshot-testable, which `PrepPadSnapshotTests` could not be until
//  `SirenExpertPanel` stopped holding `WaveformAppModel`.
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

/// Everything Prep's surface draws.
struct PrepRackState: Equatable {
    var cues: [CueSlotState] = (0..<8).map { CueSlotState(index: $0) }
    /// Beats of the loop currently running; `nil` when none is.
    var activeLoopBeats: Double?
    var loopEngaged: Bool = false
    /// Eight sample slots; `nil` is empty. Fixed slots rather than a
    /// growing list, because a slot is what a pad binds to.
    var sampleSlots: [String?] = Array(repeating: nil, count: 8)
    /// Cue and loop controls do nothing without a deck loaded, and should
    /// say so rather than fail quietly.
    var hasTrack: Bool = false
    /// A paused deck previews a cue while the pad is held; a running one
    /// jumps and keeps going.
    var isPlaying: Bool = false
}

/// Everything it does.
struct PrepRackCallbacks {
    var onCue: (_ index: Int, _ clear: Bool) -> Void = { _, _ in }
    /// Mouse-down on a set cue while the deck is paused: play from it.
    var onPreviewDown: (_ index: Int) -> Void = { _ in }
    /// Mouse-up: stop and return to the mark.
    var onPreviewUp: () -> Void = {}
    var onRenameCue: (_ index: Int) -> Void = { _ in }
    var onColorCue: (_ index: Int, _ token: String?) -> Void = { _, _ in }
    var onLoop: (_ beats: Double) -> Void = { _ in }
    /// Halve or double the running loop, keeping its start.
    var onScaleLoop: (_ double: Bool) -> Void = { _ in }
    /// Pressing the lit length again leaves the loop.
    var onExitLoop: () -> Void = {}
    /// A file dropped onto slot `index` — from the library or Finder.
    /// Dropping onto a filled slot replaces it.
    var onDropSample: (_ index: Int, _ url: URL) -> Void = { _, _ in }
    var onUnloadSample: (_ index: Int) -> Void = { _ in }
}

// MARK: - The surface

struct PrepRack: View {
    let state: PrepRackState
    var callbacks = PrepRackCallbacks()

    var body: some View {
        HStack(alignment: .top, spacing: DubSpacing.xl) {
            CueBank(
                slots: state.cues,
                hasTrack: state.hasTrack,
                isPlaying: state.isPlaying,
                onCue: callbacks.onCue,
                onPreviewDown: callbacks.onPreviewDown,
                onPreviewUp: callbacks.onPreviewUp,
                onRename: callbacks.onRenameCue,
                onColor: callbacks.onColorCue)
                .frame(width: DubLayout.prepCueColumn, alignment: .leading)

            LoopEngine(
                activeBeats: state.activeLoopBeats,
                engaged: state.loopEngaged,
                hasTrack: state.hasTrack,
                onLoop: callbacks.onLoop,
                onScale: callbacks.onScaleLoop,
                onExit: callbacks.onExitLoop)
                .frame(width: DubLayout.prepLoopSection, alignment: .leading)

            // Takes the rest. There is no fourth section coming, so
            // reserving trailing space would just be the dead area this
            // redesign exists to remove — and the shelf is the one thing
            // here that genuinely improves with width, because a filename
            // is the only string on the surface whose length is not ours
            // to choose.
            SampleShelf(
                slots: state.sampleSlots,
                onDrop: callbacks.onDropSample,
                onUnload: callbacks.onUnloadSample)
                .frame(
                    minWidth: DubLayout.prepSampleShelfMin,
                    maxWidth: .infinity,
                    alignment: .leading)
        }
    }
}

/// The one thing the three sections do share: how a section announces
/// itself. A caps label and a hairline, nothing boxed.
private struct SectionHeading: View {
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
private struct CueBank: View {
    let slots: [CueSlotState]
    let hasTrack: Bool
    let isPlaying: Bool
    let onCue: (Int, Bool) -> Void
    let onPreviewDown: (Int) -> Void
    let onPreviewUp: () -> Void
    let onRename: (Int) -> Void
    let onColor: (Int, String?) -> Void

    private var setCount: Int { slots.filter(\.isSet).count }

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
            // Two columns of four. Eight in one column would be taller
            // than the loop unit beside it and push the library down;
            // two columns keep the section the height of its neighbour.
            HStack(alignment: .top, spacing: DubSpacing.sm) {
                ForEach(0..<2, id: \.self) { column in
                    VStack(spacing: 2) {
                        ForEach(slots.filter { $0.index / 4 == column }) { row($0) }
                    }
                }
            }
            .frame(height: DubLayout.prepSectionContent)
        }
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
            Text(slot.mark?.name ?? (hasTrack ? "set at playhead" : "load a track"))
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
        // No fixed height: four rows divide `prepSectionContent`, which
        // is what keeps this section level with the other two.
        .frame(maxHeight: .infinity)
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
private struct CueRowGesture: ViewModifier {
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

/// `m:ss.t` — tenths, because a cue set by ear is placed to about that
/// precision and more digits would read as false confidence.
enum CueTimecode {
    static func format(_ secs: Double) -> String {
        guard secs.isFinite, secs >= 0 else { return "—" }
        // Round to tenths *first*, then split. Truncating the remainder
        // renders 102.3 as `1:42.2`, because 102.3 - 102 is 0.2999… in
        // binary — an off-by-a-tenth on every cue whose position lands
        // near a boundary, which is most of them.
        let tenthsTotal = (secs * 10).rounded()
        let whole = Int(tenthsTotal / 10)
        let tenths = Int(tenthsTotal) % 10
        return String(format: "%d:%02d.%d", whole / 60, whole % 60, tenths)
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
private struct LoopEngine: View {
    let activeBeats: Double?
    let engaged: Bool
    let hasTrack: Bool
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
            SectionHeading(
                title: "LOOP", accent: DubColor.loop,
                trailing: engaged ? "● ACTIVE" : "○ IDLE",
                trailingAccent: engaged ? DubColor.loop : DubColor.textPlaceholder)

            HStack(spacing: DubSpacing.sm) {
                stepper("÷2", double: false)
                sizeButtons
                stepper("×2", double: true)
            }
            .frame(height: DubLayout.prepSectionContent)
            .padding(.horizontal, DubSpacing.md)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.card, style: .continuous))
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
                    .frame(width: 52, height: 64)
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
            .frame(width: 34, height: 64)
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

// MARK: - SAMPLES — a drop target over a list

/// Prep owns getting sounds *into* the bank; Performance fires them.
///
/// Its outline is the state: dashed while the bank is empty, because that
/// is what a drop zone looks like, and solid once it holds something.
/// Neither of the other two sections changes its own border, which is what
/// keeps this one identifiable.
///
/// The sampler and Quick Scratch racks both bind from this one list, so a
/// horn added here is available to a sampler pad and a Quick Scratch key
/// at once. It lived only in Preferences, which is the wrong home for work
/// done while auditioning.
private struct SampleShelf: View {
    let slots: [String?]
    let onDrop: (Int, URL) -> Void
    let onUnload: (Int) -> Void

    private var filled: Int { slots.compactMap { $0 }.count }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            SectionHeading(
                title: "SAMPLES", accent: DubColor.deckATint,
                trailing: "\(filled) OF \(slots.count)")

            LazyVGrid(
                columns: Array(
                    repeating: GridItem(.flexible(), spacing: DubSpacing.xs), count: 4),
                spacing: DubSpacing.xs
            ) {
                ForEach(Array(slots.enumerated()), id: \.offset) { index, name in
                    slotTile(index: index, name: name)
                }
            }
        }
    }

    /// One slot. Empty slots are dashed — the drop-zone convention —
    /// and there is no Add button: a sample arrives by being dragged
    /// from the track list, which is where the DJ is already looking
    /// when they decide something should be a stab.
    private func slotTile(index: Int, name: String?) -> some View {
        let isEmpty = name == nil
        // An empty slot shows its number, the way an empty cue row
        // shows its own. Eight tiles each reading "drop" was the
        // instruction printed eight times; the dashed border already
        // says the slot takes something.
        return VStack(spacing: 2) {
            Text(name ?? "\(index + 1)")
                .font(.system(
                    size: isEmpty ? 13 : 11,
                    weight: isEmpty ? .medium : .semibold,
                    design: isEmpty ? .monospaced : .default))
                .foregroundStyle(isEmpty ? DubColor.textPlaceholder : DubColor.textPrimary)
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .minimumScaleFactor(0.85)
        }
        .frame(maxWidth: .infinity)
        .frame(height: 42)
        .padding(.horizontal, DubSpacing.xs)
        .background(isEmpty ? Color.clear : DubColor.surface2)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .strokeBorder(
                    DubColor.divider,
                    style: StrokeStyle(lineWidth: 1, dash: isEmpty ? [3, 3] : [])))
        // Replacing is the same gesture as filling: dropping onto a
        // full slot overwrites it, so there is no "clear it first".
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            onDrop(index, url)
            return true
        }
        .contextMenu {
            if !isEmpty {
                Button("Unload") { onUnload(index) }
            }
        }
        .help(isEmpty
            ? "Slot \(index + 1) — drag a track here from the library"
            : "\(name ?? "") — drop another to replace, right-click to unload")
    }
}
