import Foundation

/// One sampler pad's binding (M17, PRD §7.1).
struct SamplerSlot: Codable, Equatable {
    var url: URL
    /// Linear gain. The engine clamps to `[0, 4]`; unity by default so
    /// a freshly bound stab sounds at the level it was recorded.
    var gain: Double
    /// Which deck's output bus the one-shot sums onto. §7.1 makes this
    /// per-slot and defaults it to Deck A.
    var deck: DeckSide

    init(url: URL, gain: Double = 1.0, deck: DeckSide = .a) {
        self.url = url
        self.gain = gain
        self.deck = deck
    }
}

/// The four sampler pads (`A S D F`), as persisted.
///
/// Same shape and the same degradation contract as
/// [`QuickScratchSlots`] — see [`SlotPersistence`]. The two racks stay
/// separate types because their payloads differ (a sampler slot carries
/// gain and an output bus; a Quick Scratch slot carries a target deck)
/// and because §7.1 and §7.2 are deliberately different mechanisms.
struct SamplerSlots: Equatable {
    /// §7.1: four, keeping `A S D F` symmetric with Quick Scratch's
    /// `Q W E R`. The engine is the authority on the count at runtime
    /// (`sampler_slot_count`); this is the persisted width.
    static let count = 4

    /// Default key labels. **Not bound yet** — the keymap lands with
    /// the M18 remapping pass, so today the pads are driven from the
    /// Preferences rack.
    static let keyLabels = ["A", "S", "D", "F"]

    private var slots: [SamplerSlot?]

    init() {
        slots = Array(repeating: nil, count: Self.count)
    }

    init(persisted: String) {
        slots = SlotPersistence.decodeSlots(persisted, count: Self.count)
    }

    var persisted: String { SlotPersistence.encodeSlots(slots) }

    var all: [SamplerSlot?] { slots }

    /// The binding for `index`, or `nil` when unbound or out of range.
    func slot(_ index: Int) -> SamplerSlot? {
        slots.indices.contains(index) ? slots[index] : nil
    }

    /// Every bound URL, for adopting into the shared bank on load.
    var boundUrls: [URL] { slots.compactMap { $0?.url } }

    mutating func assign(url: URL, to index: Int) {
        guard slots.indices.contains(index) else { return }
        // Keep the slot's existing gain / output when re-pointing it at
        // a different sample: the DJ set those for the pad, not for the
        // file.
        let existing = slots[index]
        slots[index] = SamplerSlot(
            url: url,
            gain: existing?.gain ?? 1.0,
            deck: existing?.deck ?? .a)
    }

    mutating func setGain(_ gain: Double, for index: Int) {
        guard slots.indices.contains(index), var slot = slots[index] else { return }
        slot.gain = min(max(gain, 0.0), 4.0)
        slots[index] = slot
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
}
