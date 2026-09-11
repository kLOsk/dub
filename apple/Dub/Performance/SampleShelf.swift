//
//  SampleShelf.swift
//  Dub
//
//  The sampler, on both surfaces.
//
//  Prep drew this first as a drop shelf — eight tiles, dashed while
//  empty, right-click to unload — and Performance kept a separate row of
//  four `TriggerPadGroup` pads bound from it in Preferences. Same files,
//  two racks, and a slot loaded on the shelf was not a pad you could
//  fire. It is one view now: Prep loads and auditions, Performance fires,
//  and the tile a hand learns in the one place is the tile it finds in
//  the other.
//
//  ## The gestures
//
//  * **Press** fires the slot on mouse-down, like every pad. Pressing a
//    sounding slot starts it over (the engine crossfades the retrigger).
//  * **Right-click while sounding stops it.** No menu — a hand mid-set
//    wants the horn gone, not a list to read.
//  * **Right-click while quiet offers the Quick Scratch tag and Unload.**
//    The destructive gesture only exists when there is nothing to
//    interrupt; the tag is prep work and lives on the tile because the
//    tile is where the sample is.
//  * **Drop** loads, onto empty and full alike. There is no Add button:
//    a sample arrives by being dragged from the track list or Finder,
//    which is where the DJ is already looking when they decide something
//    should be a stab.
//
//  Loading is Prep work by the placement rule, but the drop target costs
//  nothing to keep on Performance, and refusing it there would be a
//  gratuitous difference between two drawings of the same thing.
//

import SwiftUI

// MARK: - State

/// One slot as drawn.
struct SampleSlotState: Equatable, Identifiable {
    var index: Int
    /// Display name of the loaded sample; `nil` is an empty slot.
    var name: String?
    /// `true` while the one-shot is sounding.
    var playing: Bool = false
    /// How far through the take, `0...1`. `0` when idle.
    var progress: Double = 0
    /// The Quick Scratch pad this slot answers to (0-based), printed
    /// on the tile as `QS 1`; `nil` when it is a plain sampler slot.
    var quickScratch: Int?

    var id: Int { index }
}

/// Everything the shelf draws.
struct SampleShelfState: Equatable {
    var slots: [SampleSlotState]
    /// Where the pads fire — the master by default, or the deck(s) the
    /// DJ pinned the rack to. `nil` hides the pill: Prep is one deck,
    /// and a `→ A` there would be noise.
    var output: RackOutputState?

    init(slots: [SampleSlotState], output: RackOutputState? = nil) {
        self.slots = slots
        self.output = output
    }

    /// Eight idle slots from their names — fixtures and snapshots.
    init(names: [String?], output: RackOutputState? = nil) {
        self.init(
            slots: names.enumerated().map { SampleSlotState(index: $0.offset, name: $0.element) },
            output: output)
    }

    static let empty = SampleShelfState(names: Array(repeating: nil, count: SampleBank.count))
}

/// Everything it does.
struct SampleShelfCallbacks {
    var onTrigger: (_ index: Int) -> Void = { _ in }
    var onStop: (_ index: Int) -> Void = { _ in }
    /// A file dropped onto slot `index` — from the library or Finder.
    /// Dropping onto a filled slot replaces it.
    var onDrop: (_ index: Int, _ url: URL) -> Void = { _, _ in }
    var onUnload: (_ index: Int) -> Void = { _ in }
    /// Tag slot `index` with Quick Scratch pad `pad` (0-based), or
    /// `nil` to clear the tag.
    var onQuickScratch: (_ index: Int, _ pad: Int?) -> Void = { _, _ in }
    /// The pill's right-click: pin the rack's output or let it follow
    /// the master again.
    var onOutput: (_ output: RackOutput) -> Void = { _ in }
}

// MARK: - The shelf

/// A drop target over a pad bank.
///
/// Its outline is the state: dashed while empty, because that is what a
/// drop zone looks like, and solid once it holds something. Neither
/// neighbour on either surface changes its own border, which is what
/// keeps this one identifiable.
struct SampleShelf: View {
    let state: SampleShelfState
    var callbacks = SampleShelfCallbacks()

    /// Row height. Two of these plus a gap is `prepSectionContent`, the
    /// height every Prep section ends on, and what the rack bar's own
    /// height is derived from.
    static let tileHeight: CGFloat = 42

    private var filled: Int { state.slots.filter { $0.name != nil }.count }
    private var tint: Color {
        guard let output = state.output else { return DubColor.deckATint }
        return output.tintDeck.map(DubColor.deckTint) ?? DubColor.controlAccent
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            heading

            LazyVGrid(
                columns: Array(
                    repeating: GridItem(.flexible(), spacing: DubSpacing.xs), count: 4),
                spacing: DubSpacing.xs
            ) {
                // 1-4 on top, 5-8 underneath — reading order. A pad
                // controller's bottom-up numbering was tried and read
                // wrong against the numbered empties on screen.
                ForEach(Self.slotOrder(count: state.slots.count), id: \.self) { index in
                    tile(state.slots[index])
                }
            }
        }
    }

    private var heading: some View {
        HStack(spacing: DubSpacing.sm) {
            SectionHeading(
                title: "SAMPLES", accent: tint,
                trailing: "\(filled) OF \(state.slots.count)")
            if let output = state.output {
                DubDeckPill(state: output, onSelect: callbacks.onOutput)
                    .help("Where samples sound — the master deck by default. "
                        + "Right-click to pin them to A, B or both.")
            }
        }
    }

    /// Slot indices in draw order: reading order, 1–4 over 5–8. A
    /// function rather than a range at the call site so the decision
    /// has one home and a test.
    static func slotOrder(count: Int) -> [Int] {
        Array(0..<count)
    }

    /// One slot. Empty slots are dashed — the drop-zone convention.
    /// An empty slot shows its number, the way an empty cue row shows
    /// its own; eight tiles each reading "drop" was the instruction
    /// printed eight times, and the dashed border already says the
    /// slot takes something.
    ///
    /// A sounding slot lights in the deck's tint and a sweep crosses it
    /// left to right with the take, so the eye can see how much horn
    /// is left without listening for the end.
    @ViewBuilder
    private func tile(_ slot: SampleSlotState) -> some View {
        let isEmpty = slot.name == nil
        let lit = slot.playing && !isEmpty
        VStack(spacing: 2) {
            Text(slot.name ?? "\(slot.index + 1)")
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
        .frame(height: Self.tileHeight)
        .padding(.horizontal, DubSpacing.xs)
        // Sweep first so it sits between the fill and the label:
        // backgrounds stack outward, each behind the last.
        .background(alignment: .leading) {
            if lit {
                GeometryReader { geo in
                    Rectangle()
                        .fill(tint.opacity(0.28))
                        .frame(width: geo.size.width * slot.progress)
                }
            }
        }
        .background(isEmpty ? Color.clear : lit ? tint.opacity(0.24) : DubColor.surface2)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .strokeBorder(
                    lit ? tint : DubColor.divider,
                    style: StrokeStyle(lineWidth: 1, dash: isEmpty ? [3, 3] : [])))
        .overlay(alignment: .topTrailing) {
            if let pad = slot.quickScratch {
                Text("QS\(pad + 1)")
                    .font(.system(size: 8, weight: .semibold, design: .monospaced))
                    .foregroundStyle(lit ? DubColor.textPrimary : tint)
                    .padding(.horizontal, 3)
                    .padding(.vertical, 1)
                    .background(DubColor.surface0.opacity(0.7))
                    .clipShape(RoundedRectangle(cornerRadius: 3, style: .continuous))
                    .padding(3)
            }
        }
        .contentShape(Rectangle())
        .onPressDown(enabled: !isEmpty) { callbacks.onTrigger(slot.index) }
        .onSecondaryClick {
            guard !isEmpty else { return .handled }
            if slot.playing {
                callbacks.onStop(slot.index)
                return .handled
            }
            return .menu(quietMenu(slot))
        }
        // Replacing is the same gesture as filling: dropping onto a
        // full slot overwrites it, so there is no "clear it first".
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            callbacks.onDrop(slot.index, url)
            return true
        }
        .help(help(slot))
    }

    /// The stopped-state menu: which Quick Scratch pad this sample sits
    /// on, and the way out. Flat rather than a submenu — five lines,
    /// read once.
    private func quietMenu(_ slot: SampleSlotState) -> [SecondaryMenuItem] {
        var items = (0..<SampleBank.quickScratchCount).map { pad in
            SecondaryMenuItem("Quick Scratch \(pad + 1)", checked: slot.quickScratch == pad) {
                callbacks.onQuickScratch(slot.index, slot.quickScratch == pad ? nil : pad)
            }
        }
        items.append(.separator)
        items.append(SecondaryMenuItem("Unload") { callbacks.onUnload(slot.index) })
        return items
    }

    private func help(_ slot: SampleSlotState) -> String {
        guard let name = slot.name else {
            return "Slot \(slot.index + 1) — drag a track here from the library or Finder"
        }
        let tag = slot.quickScratch.map { " · Quick Scratch \($0 + 1)" } ?? ""
        return slot.playing
            ? "\(name) — click to start over, right-click to stop"
            : "\(name)\(tag) — click to play, right-click for Quick Scratch / unload, "
                + "drop another to replace"
    }
}
