import Foundation

/// One loaded sampler slot as persisted: the file, and the Quick
/// Scratch pad it sits on, if any.
struct SampleSlot: Codable, Equatable {
    var url: URL
    /// Quick Scratch pad (0-based, `< SampleBank.quickScratchCount`)
    /// this sample answers to on the deck columns, or `nil`. One slot
    /// per pad — see `SampleBank.setQuickScratch`.
    var quickScratch: Int?

    init(url: URL, quickScratch: Int? = nil) {
        self.url = url
        self.quickScratch = quickScratch
    }
}

/// The sampler: eight positional slots, each holding a sample file or
/// nothing (M17 §7.1).
///
/// One table for both surfaces. Prep's SAMPLES shelf loads slot *n*;
/// Performance's SAMPLER fires slot *n*; and a slot **tagged** with a
/// Quick Scratch pad (§7.2) is what that pad puts on a deck. There used
/// to be two things here — a plain list ("the bank") and a separate
/// four-pad binding layer configured in Preferences with its own gain
/// and output — and a slot loaded on the shelf was not, by itself, a
/// pad you could fire. Now it is, and Quick Scratch is a tag on it
/// rather than a third table.
///
/// **Slots are positional.** Unloading slot 2 leaves 3 where it was: a
/// pad is a place the hand learns, and a list that slid left on every
/// unload would move every pad above it. The earlier list-backed bank
/// did exactly that, against its own doc comment.
struct SampleBank: Equatable {
    /// Eight, laid out on the shelf as two rows of four (1–4 over 5–8).
    /// The engine is the authority at runtime (`sampler_slot_count`);
    /// this is the persisted width.
    static let count = 8
    /// Quick Scratch pads per deck column. Four: a row of them sits
    /// under the loop, and four is what a hand finds without looking.
    static let quickScratchCount = 4

    private var slots: [SampleSlot?]

    init() {
        slots = Array(repeating: nil, count: Self.count)
    }

    /// A bank with `urls` in slots 0, 1, 2… — how a legacy list-form
    /// bank migrates, and a convenience for tests.
    init(urls: [URL]) {
        self.init()
        for (index, url) in urls.prefix(Self.count).enumerated() {
            slots[index] = SampleSlot(url: url)
        }
    }

    /// Restore from [`persisted`]. Unreadable input comes back empty
    /// rather than throwing — the bank is read while building a view.
    ///
    /// Three generations of this string exist: the plain `[URL]` list,
    /// the positional `[URL?]` table, and the `[SampleSlot?]` table with
    /// tags. The first two are the same JSON shape (strings and nulls),
    /// and decode into the third with no tags, so nobody's samples
    /// vanish on upgrade.
    init(persisted: String) {
        let tagged: [SampleSlot?] = SlotPersistence.decodeSlots(persisted, count: Self.count)
        if tagged.contains(where: { $0 != nil }) {
            slots = tagged
            return
        }
        let plain: [URL?] = SlotPersistence.decodeSlots(persisted, count: Self.count)
        slots = plain.map { $0.map { SampleSlot(url: $0) } }
    }

    var persisted: String { SlotPersistence.encodeSlots(slots) }

    /// Every slot, in slot order; `nil` is empty.
    var all: [URL?] { slots.map { $0?.url } }

    /// The loaded files, in slot order.
    var loaded: [URL] { slots.compactMap { $0?.url } }

    var isEmpty: Bool { loaded.isEmpty }

    /// The file in slot `index`, or `nil` when empty or out of range.
    func slot(_ index: Int) -> URL? {
        slots.indices.contains(index) ? slots[index]?.url : nil
    }

    /// Put `url` in slot `index`, replacing whatever was there. Dropping
    /// onto a full slot is the same gesture as filling an empty one, and
    /// the slot keeps its Quick Scratch tag — the tag names the pad, not
    /// the file.
    mutating func load(_ url: URL, into index: Int) {
        guard slots.indices.contains(index) else { return }
        slots[index] = SampleSlot(url: url, quickScratch: slots[index]?.quickScratch)
    }

    mutating func unload(_ index: Int) {
        guard slots.indices.contains(index) else { return }
        slots[index] = nil
    }

    // MARK: - Quick Scratch tags

    /// The Quick Scratch pad slot `index` answers to, if any.
    func quickScratchTag(_ index: Int) -> Int? {
        slots.indices.contains(index) ? slots[index]?.quickScratch : nil
    }

    /// The sampler slot on Quick Scratch pad `pad`, or `nil` when the
    /// pad is empty.
    func quickScratchSlot(pad: Int) -> Int? {
        slots.firstIndex { $0?.quickScratch == pad }
    }

    /// Tag slot `index` with Quick Scratch pad `pad`, or clear its tag
    /// with `nil`. A pad names one slot, so the tag moves: tagging slot
    /// 5 with pad 2 untags whichever slot had pad 2. An empty slot
    /// cannot be tagged.
    mutating func setQuickScratch(_ pad: Int?, for index: Int) {
        guard slots.indices.contains(index), slots[index] != nil else { return }
        if let pad {
            guard (0..<Self.quickScratchCount).contains(pad) else { return }
            for i in slots.indices where slots[i]?.quickScratch == pad {
                slots[i]?.quickScratch = nil
            }
        }
        slots[index]?.quickScratch = pad
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
