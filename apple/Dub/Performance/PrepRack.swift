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
            CueRowBank(
                slots: state.cues,
                hasTrack: state.hasTrack,
                isPlaying: state.isPlaying,
                onCue: callbacks.onCue,
                onPreviewDown: callbacks.onPreviewDown,
                onPreviewUp: callbacks.onPreviewUp,
                onRename: callbacks.onRenameCue,
                onColor: callbacks.onColorCue,
                columns: 2,
                contentHeight: DubLayout.prepSectionContent)
                .frame(width: DubLayout.prepCueColumn, alignment: .leading)

            // **Loops are off Prep for now.** The control is built and
            // its column is held at full width — hidden and untappable
            // rather than removed, so the shelf beside it does not
            // spread into a gap that is going to be filled again.
            // `.hidden()` rather than a spacer of some guessed size:
            // the space stays exactly what will occupy it, and
            // re-mounting is deleting one modifier. Performance keeps
            // its own loop pads — this is a Prep decision only.
            LoopEngine(
                activeBeats: state.activeLoopBeats,
                engaged: state.loopEngaged,
                hasTrack: state.hasTrack,
                onLoop: callbacks.onLoop,
                onScale: callbacks.onScaleLoop,
                onExit: callbacks.onExitLoop)
                .frame(width: DubLayout.prepLoopSection, alignment: .leading)
                .hidden()
                .allowsHitTesting(false)

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
                // 5-8 on top, 1-4 underneath. A numbered bank counts
                // *up* from the row nearest the hand, the way a pad
                // controller's rows do — reading the grid left-to-right
                // top-to-bottom put 1 furthest from the fingers.
                ForEach(Array(Self.slotOrder(count: slots.count)), id: \.self) { index in
                    slotTile(index: index, name: slots[index])
                }
            }
        }
    }

    /// Slot indices in draw order: the second half first, so the
    /// grid's *bottom* row is 1-4. Derived rather than hard-coded so a
    /// bank of a different size still splits down the middle.
    static func slotOrder(count: Int) -> [Int] {
        let half = count / 2
        return Array(half..<count) + Array(0..<half)
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
