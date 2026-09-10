import Foundation

/// The sampler: eight positional slots, each holding a sample file or
/// nothing (M17 §7.1).
///
/// One table for both surfaces. Prep's SAMPLES shelf loads slot *n*;
/// Performance's SAMPLER fires slot *n*; Quick Scratch (§7.2) binds a
/// key to whichever of these files it wants. There used to be two
/// things here — a plain list ("the bank") and a separate four-pad
/// binding layer configured in Preferences with its own gain and output
/// — and a slot loaded on the shelf was not, by itself, a pad you could
/// fire. Now it is.
///
/// **Slots are positional.** Unloading slot 2 leaves 3 where it was: a
/// pad is a place the hand learns, and a list that slid left on every
/// unload would move every pad above it. The earlier list-backed bank
/// did exactly that, against its own doc comment.
struct SampleBank: Equatable {
    /// Eight, laid out on the shelf as two rows of four (5–8 over 1–4).
    /// The engine is the authority at runtime (`sampler_slot_count`);
    /// this is the persisted width.
    static let count = 8

    private var slots: [URL?]

    init() {
        slots = Array(repeating: nil, count: Self.count)
    }

    /// A bank with `urls` in slots 0, 1, 2… — how a legacy list-form
    /// bank migrates, and a convenience for tests.
    init(urls: [URL]) {
        self.init()
        for (index, url) in urls.prefix(Self.count).enumerated() {
            slots[index] = url
        }
    }

    /// Restore from [`persisted`]. Unreadable input comes back empty
    /// rather than throwing — the bank is read while building a view.
    ///
    /// The plain `[URL]` list the bank used to be is a valid `[URL?]`
    /// table with no gaps, so a pre-positional bank lands in slot order
    /// and nobody's samples vanish on upgrade.
    init(persisted: String) {
        slots = SlotPersistence.decodeSlots(persisted, count: Self.count)
    }

    var persisted: String { SlotPersistence.encodeSlots(slots) }

    /// Every slot, in slot order; `nil` is empty.
    var all: [URL?] { slots }

    /// The loaded files, in slot order — what Quick Scratch binds from.
    var loaded: [URL] { slots.compactMap { $0 } }

    var isEmpty: Bool { loaded.isEmpty }

    /// The file in slot `index`, or `nil` when empty or out of range.
    func slot(_ index: Int) -> URL? {
        slots.indices.contains(index) ? slots[index] : nil
    }

    /// Put `url` in slot `index`, replacing whatever was there. Dropping
    /// onto a full slot is the same gesture as filling an empty one.
    mutating func load(_ url: URL, into index: Int) {
        guard slots.indices.contains(index) else { return }
        slots[index] = url
    }

    mutating func unload(_ index: Int) {
        guard slots.indices.contains(index) else { return }
        slots[index] = nil
    }

    /// Display name for a slot — the filename, which is what a DJ
    /// recognises. Full path is the tooltip's job.
    static func label(for url: URL) -> String {
        url.lastPathComponent
    }
}

/// Encoding for the slot tables (M17).
///
/// Extracted because the sampler and Quick Scratch need the same
/// contract, and it is the contract rather than the payload that
/// matters: a preferences string that is empty, truncated, or written
/// by another build must resolve to *nothing bound* rather than
/// throwing at the call site, which for these types is a keypress or
/// a view build.
enum SlotPersistence {

    /// Decode a fixed-width slot table, padding or trimming to `count`.
    ///
    /// Normalising rather than rejecting means changing the slot count
    /// later (§7.1 keeps widening on the table) does not reset
    /// everyone's bindings.
    static func decodeSlots<Slot: Decodable>(_ persisted: String, count: Int) -> [Slot?] {
        var slots = [Slot?](repeating: nil, count: count)
        guard let data = persisted.data(using: .utf8),
            let decoded = try? JSONDecoder().decode([Slot?].self, from: data)
        else {
            return slots
        }
        for (index, slot) in decoded.prefix(count).enumerated() {
            slots[index] = slot
        }
        return slots
    }

    static func encodeSlots<Slot: Encodable>(_ slots: [Slot?]) -> String {
        guard let data = try? JSONEncoder().encode(slots),
            let text = String(data: data, encoding: .utf8)
        else {
            return ""
        }
        return text
    }
}
