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
//  The beatgrid editor belongs here and does not exist yet: PRD §3.1 names
//  it as Prep's reason to exist and six FFI calls sit behind no surface.
//  Nothing here assumes three sections forever.
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
    var cues: [CueSlotState] = (0..<4).map { CueSlotState(index: $0) }
    /// Bars of the loop currently running; `nil` when none is.
    var activeLoopBars: Double?
    var loopEngaged: Bool = false
    /// `IN` taken, waiting for `OUT`.
    var loopInArmed: Bool = false
    /// Sample names in the shared bank, in bank order.
    var sampleNames: [String] = []
    /// Cue and loop controls do nothing without a deck loaded, and should
    /// say so rather than fail quietly.
    var hasTrack: Bool = false
}

/// Everything it does.
struct PrepRackCallbacks {
    var onCue: (_ index: Int, _ clear: Bool) -> Void = { _, _ in }
    var onRenameCue: (_ index: Int) -> Void = { _ in }
    var onColorCue: (_ index: Int, _ token: String?) -> Void = { _, _ in }
    var onLoop: (_ bars: Double) -> Void = { _ in }
    var onLoopStep: (_ double: Bool) -> Void = { _ in }
    var onLoopIn: () -> Void = {}
    var onLoopOut: () -> Void = {}
    var onLoopExit: () -> Void = {}
    var onAddSamples: () -> Void = {}
    var onRemoveSample: (_ index: Int) -> Void = { _ in }
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
                onCue: callbacks.onCue,
                onRename: callbacks.onRenameCue,
                onColor: callbacks.onColorCue)
                .frame(width: DubLayout.prepCueColumn, alignment: .leading)

            LoopEngine(
                activeBars: state.activeLoopBars,
                engaged: state.loopEngaged,
                inArmed: state.loopInArmed,
                hasTrack: state.hasTrack,
                onLoop: callbacks.onLoop,
                onStep: callbacks.onLoopStep,
                onIn: callbacks.onLoopIn,
                onOut: callbacks.onLoopOut,
                onExit: callbacks.onLoopExit)
                .frame(width: DubLayout.prepLoopSection, alignment: .leading)

            SampleShelf(
                names: state.sampleNames,
                onAdd: callbacks.onAddSamples,
                onRemove: callbacks.onRemoveSample)
                .frame(
                    minWidth: DubLayout.prepSampleShelfMin,
                    maxWidth: DubLayout.prepSampleShelfMax,
                    alignment: .leading)

            Spacer(minLength: 0)
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
    let onCue: (Int, Bool) -> Void
    let onRename: (Int) -> Void
    let onColor: (Int, String?) -> Void

    private var setCount: Int { slots.filter(\.isSet).count }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            SectionHeading(
                title: "CUE", accent: DubColor.hotCue,
                trailing: "\(setCount) OF 4")
            VStack(spacing: DubSpacing.xs) {
                ForEach(slots) { row($0) }
            }
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
        }
        .padding(.trailing, DubSpacing.sm)
        .frame(height: 30)
        .background(slot.isSet ? DubColor.surface2 : Color.clear)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .strokeBorder(
                    slot.isSet ? DubColor.divider : DubColor.divider.opacity(0.7),
                    style: StrokeStyle(lineWidth: 1, dash: slot.isSet ? [] : [3, 3])))
        .contentShape(Rectangle())
        .onPressDown(enabled: hasTrack) {
            onCue(slot.index, NSEvent.modifierFlags.contains(.shift))
        }
        .help(slot.isSet
            ? "Cue \(slot.index + 1) — click to jump, ⇧-click to clear"
            : "Cue \(slot.index + 1) — click to set at the playhead")
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

// MARK: - LOOP — an instrument

/// The only boxed thing on the surface, because it is the only thing that
/// behaves like a unit: a size readout you drive, rather than a set of
/// independent buttons.
///
/// A big numeral in a recessed well is the anchor — nothing else here is a
/// number that size, so the block is identifiable before you read a label.
/// The ACTIVE lamp reports whether audio is *actually* looping, which the
/// old single `lit` flag could not tell apart from "this length is
/// selected".
private struct LoopEngine: View {
    let activeBars: Double?
    let engaged: Bool
    let inArmed: Bool
    let hasTrack: Bool
    let onLoop: (Double) -> Void
    let onStep: (Bool) -> Void
    let onIn: () -> Void
    let onOut: () -> Void
    let onExit: () -> Void

    private static let ladder: [(bars: Double, label: String)] = [
        (0.5, "½"), (1, "1"), (2, "2"), (4, "4"),
    ]
    private var canExit: Bool { activeBars != nil || engaged || inArmed }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            SectionHeading(
                title: "LOOP", accent: DubColor.loop,
                trailing: engaged ? "● ACTIVE" : "○ IDLE",
                trailingAccent: engaged ? DubColor.loop : DubColor.textPlaceholder)

            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                HStack(spacing: DubSpacing.sm) {
                    readout
                    VStack(spacing: DubSpacing.xs) {
                        stepper("×2", double: true)
                        stepper("÷2", double: false)
                    }
                    ladderControl
                }
                HStack(spacing: DubSpacing.xs) {
                    DubPadCell("IN", size: .word, lit: inArmed,
                               enabled: hasTrack, tint: DubColor.loop)
                        .onPressDown(enabled: hasTrack) { onIn() }
                        .help("Set the loop start at the playhead")
                    DubPadCell("OUT", size: .word, enabled: inArmed, tint: DubColor.loop)
                        .onPressDown(enabled: inArmed) { onOut() }
                        .help("Close the loop at the playhead and start it")
                    DubPadCell("EXIT", size: .word, enabled: canExit, tint: DubColor.loop)
                        .onPressDown(enabled: canExit) { onExit() }
                        .help("Exit the loop")
                    Spacer(minLength: 0)
                }
                Text("reverse — loops the bars just played")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
            }
            .padding(DubSpacing.md)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.card, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.card, style: .continuous)
                    .stroke(DubColor.divider, lineWidth: 1))
        }
    }

    private var readout: some View {
        VStack(spacing: 0) {
            Text(activeBars.map(Self.barLabel) ?? "—")
                .font(.system(size: 30, weight: .semibold, design: .monospaced))
                .foregroundStyle(activeBars == nil ? DubColor.textPlaceholder : DubColor.loop)
            Text("BARS")
                .font(DubFont.micro)
                .tracking(1.6)
                .foregroundStyle(DubColor.textTertiary)
        }
        .frame(width: 90, height: 64)
        .background(DubColor.surface0)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .stroke(DubColor.divider, lineWidth: 1))
    }

    private func stepper(_ glyph: String, double: Bool) -> some View {
        Text(glyph)
            .font(.system(size: 12, weight: .medium, design: .monospaced))
            .foregroundStyle(activeBars == nil ? DubColor.textPlaceholder : DubColor.textSecondary)
            .frame(width: 32, height: 30)
            .background(DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(DubColor.divider, lineWidth: 1))
            .contentShape(Rectangle())
            .onPressDown(enabled: activeBars != nil) { onStep(double) }
            .help(double ? "Double the loop length" : "Halve the loop length")
    }

    /// One container, four segments — deliberately not four pads. Picking
    /// a length is a single choice among four, and drawing it as one
    /// object says so.
    private var ladderControl: some View {
        HStack(spacing: 0) {
            ForEach(Array(Self.ladder.enumerated()), id: \.offset) { offset, entry in
                let on = activeBars == entry.bars
                Text(entry.label)
                    .font(.system(size: 14, weight: .semibold, design: .rounded))
                    .foregroundStyle(on ? DubColor.textPrimary : DubColor.textSecondary)
                    .frame(width: 40, height: 64)
                    .background(on ? DubColor.loop.opacity(0.26) : Color.clear)
                    .overlay(alignment: .trailing) {
                        if offset < Self.ladder.count - 1 {
                            Rectangle().fill(DubColor.divider).frame(width: 1)
                        }
                    }
                    .contentShape(Rectangle())
                    .onPressDown(enabled: hasTrack) { onLoop(entry.bars) }
                    .help("Loop the last \(entry.label) bar\(entry.bars == 1 ? "" : "s")")
            }
        }
        .background(DubColor.surface0)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .stroke(DubColor.divider, lineWidth: 1))
    }

    static func barLabel(_ bars: Double) -> String {
        bars == 0.5 ? "½" : String(Int(bars))
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
    let names: [String]
    let onAdd: () -> Void
    let onRemove: (Int) -> Void

    private var isEmpty: Bool { names.isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            SectionHeading(
                title: "SAMPLES", accent: DubColor.deckATint,
                trailing: isEmpty ? "EMPTY" : "\(names.count) IN BANK")

            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                if isEmpty {
                    Text("Air horns, stabs, sirens, drops. Add them once — the sampler pads and Quick Scratch keys both bind from here.")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(Array(names.enumerated()), id: \.offset) { index, name in
                                sampleRow(index: index, name: name)
                            }
                        }
                    }
                    .frame(maxHeight: 84)
                }
                addButton
            }
            .padding(DubSpacing.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.card, style: .continuous)
                    .strokeBorder(
                        isEmpty ? DubColor.divider.opacity(0.8) : DubColor.divider,
                        style: StrokeStyle(lineWidth: 1, dash: isEmpty ? [4, 4] : [])))
        }
    }

    private func sampleRow(index: Int, name: String) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Text(name)
                .font(.system(size: 12))
                .foregroundStyle(DubColor.textPrimary)
                .lineLimit(1)
            Spacer(minLength: DubSpacing.sm)
            Button("Remove") { onRemove(index) }
                .buttonStyle(.plain)
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
        }
        .padding(.horizontal, DubSpacing.sm)
        .frame(height: 24)
        .background(DubColor.surface2)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
    }

    private var addButton: some View {
        Button(action: onAdd) {
            Text("Add Samples…")
                .font(.system(size: 12, weight: .semibold, design: .rounded))
                .foregroundStyle(DubColor.textPrimary)
                .padding(.horizontal, DubSpacing.lg)
                .frame(height: 28)
                .background(DubColor.surface2)
                .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                        .stroke(DubColor.divider, lineWidth: 1))
        }
        .buttonStyle(.plain)
    }
}
