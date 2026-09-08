import Foundation

/// One Quick Scratch binding (M17, PRD §7.2): a sample file and the
/// deck it loads onto.
struct QuickScratchSlot: Codable, Equatable {
    var url: URL
    /// Target deck. §7.2 defaults to Deck A and makes it per-slot,
    /// because the DJ scratches the sample on one turntable while the
    /// other keeps playing.
    var deck: DeckSide

    init(url: URL, deck: DeckSide = .a) {
        self.url = url
        self.deck = deck
    }
}

/// The four Quick Scratch slots (`Q W E R`), as persisted.
///
/// A value type with its own encoding rather than a bag of
/// `UserDefaults` keys: the table is read on a keypress mid-set, so
/// "the saved form was written by another build" has to resolve to
/// *empty slots* rather than to a throw at the call site.
struct QuickScratchSlots: Equatable {
    /// §7.1 explains the number: four keeps `Q W E R` symmetric with
    /// the sampler's `A S D F`, and four has been enough for the
    /// target user's drop / siren / horn / vocal-stab workflow.
    static let count = 4

    private var slots: [QuickScratchSlot?]

    init() {
        slots = Array(repeating: nil, count: Self.count)
    }

    /// Restore from [`persisted`]. Anything unreadable — empty,
    /// truncated, a shape from a future build — comes back as empty
    /// slots. See [`SlotPersistence`], which the sampler rack shares.
    init(persisted: String) {
        slots = SlotPersistence.decodeSlots(persisted, count: Self.count)
    }

    /// The table in the form the shell stores in `UserDefaults`.
    var persisted: String { SlotPersistence.encodeSlots(slots) }

    var all: [QuickScratchSlot?] { slots }

    /// The binding for `index`, or `nil` when unbound or out of range.
    /// Out of range is `nil` rather than a trap: the caller is a
    /// keypress handler, and a stray index mid-set should do nothing.
    func slot(_ index: Int) -> QuickScratchSlot? {
        slots.indices.contains(index) ? slots[index] : nil
    }

    /// Every bound URL, for adopting into the shared bank on load.
    var boundUrls: [URL] { slots.compactMap { $0?.url } }

    mutating func assign(url: URL, deck: DeckSide = .a, to index: Int) {
        guard slots.indices.contains(index) else { return }
        slots[index] = QuickScratchSlot(url: url, deck: deck)
    }

    mutating func setDeck(_ deck: DeckSide, for index: Int) {
        guard slots.indices.contains(index), var slot = slots[index] else { return }
        slot.deck = deck
        slots[index] = slot
    }

    mutating func clear(_ index: Int) {
        guard slots.indices.contains(index) else { return }
        slots[index] = nil
    }

    /// Key labels, in slot order — the default `Q W E R` binding.
    /// Rebinding arrives with the M18 key-remapping pass.
    /// Derived, not typed — see `DubKeymap`.
    static var keyLabels: [String] {
        DubKeymap.legends({ .quickScratch($0) }, count: count)
    }
}
