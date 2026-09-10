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
//  * **Right-click while quiet offers Unload.** The destructive gesture
//    only exists when there is nothing to interrupt.
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

    var id: Int { index }
}

/// Everything the shelf draws.
struct SampleShelfState: Equatable {
    var slots: [SampleSlotState]
    /// The deck the pads fire on — the master. `nil` hides the pill:
    /// Prep is one deck, and a `→ A` there would be noise.
    var focusedDeck: DeckSide?

    init(slots: [SampleSlotState], focusedDeck: DeckSide? = nil) {
        self.slots = slots
        self.focusedDeck = focusedDeck
    }

    /// Eight idle slots from their names — fixtures and snapshots.
    init(names: [String?], focusedDeck: DeckSide? = nil) {
        self.init(
            slots: names.enumerated().map { SampleSlotState(index: $0.offset, name: $0.element) },
            focusedDeck: focusedDeck)
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
    private var tint: Color { DubColor.deckTint(state.focusedDeck ?? .a) }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            heading

            LazyVGrid(
                columns: Array(
                    repeating: GridItem(.flexible(), spacing: DubSpacing.xs), count: 4),
                spacing: DubSpacing.xs
            ) {
                // 5-8 on top, 1-4 underneath. A numbered bank counts
                // *up* from the row nearest the hand, the way a pad
                // controller's rows do — reading the grid left-to-right
                // top-to-bottom put 1 furthest from the fingers.
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
            if let deck = state.focusedDeck {
                DubDeckPill(deck: deck)
                    .help("Samples sound on the focused deck — the master, or deck A "
                        + "when neither is.")
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
        .contentShape(Rectangle())
        .onPressDown(enabled: !isEmpty) { callbacks.onTrigger(slot.index) }
        .onSecondaryClick {
            guard !isEmpty else { return .handled }
            if slot.playing {
                callbacks.onStop(slot.index)
                return .handled
            }
            return .menu([SecondaryMenuItem("Unload") { callbacks.onUnload(slot.index) }])
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

    private func help(_ slot: SampleSlotState) -> String {
        guard let name = slot.name else {
            return "Slot \(slot.index + 1) — drag a track here from the library or Finder"
        }
        return slot.playing
            ? "\(name) — click to start over, right-click to stop"
            : "\(name) — click to play, right-click to unload, drop another to replace"
    }
}
