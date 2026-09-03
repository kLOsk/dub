import Foundation

/// The DJ's working set of sample files, shared by both trigger racks
/// (M17 §7.1 sampler, §7.2 Quick Scratch).
///
/// One bank rather than a file picker per slot: the same horn tends to
/// be wanted on a sampler pad *and* on a Quick Scratch key, and picking
/// it twice in two different rows is the kind of small friction that
/// makes a feature feel unfinished. Slots still store their own URL, so
/// this is a convenience layer over binding — not a level of
/// indirection the slots depend on.
struct SampleBank: Equatable {
    private var urls: [URL]

    init(urls: [URL] = []) {
        self.urls = []
        for url in urls {
            add(url)
        }
    }

    /// Restore from [`persisted`]. Unreadable input comes back empty
    /// rather than throwing — the bank is read while building the
    /// Preferences sheet.
    init(persisted: String) {
        self.init(urls: SlotPersistence.decodeList(persisted))
    }

    var persisted: String { SlotPersistence.encodeList(urls) }

    /// The files, in the order they were added.
    var all: [URL] { urls }

    var isEmpty: Bool { urls.isEmpty }

    /// Add a file. Duplicates are ignored, so adding the same horn
    /// twice leaves one entry.
    mutating func add(_ url: URL) {
        guard !urls.contains(url) else { return }
        urls.append(url)
    }

    mutating func remove(_ url: URL) {
        urls.removeAll { $0 == url }
    }

    /// Ensure `url` is present without disturbing existing order.
    ///
    /// Called for every URL found in a slot on load: bindings persisted
    /// before the bank existed (or edited by hand) must appear in the
    /// list rather than being invisible in the UI while audibly bound.
    mutating func adopt(_ url: URL) {
        add(url)
    }

    /// Display name for a bank row — the filename, which is what a DJ
    /// recognises. Full path is the tooltip's job.
    static func label(for url: URL) -> String {
        url.lastPathComponent
    }
}

/// Encoding for the slot tables and the bank (M17).
///
/// Extracted because both racks and the bank need the same contract,
/// and it is the contract rather than the payload that matters: a
/// preferences string that is empty, truncated, or written by another
/// build must resolve to *nothing bound* rather than throwing at the
/// call site, which for these types is a keypress or a sheet build.
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

    /// Decode a plain list (the bank). Same degradation contract.
    static func decodeList<Element: Decodable>(_ persisted: String) -> [Element] {
        guard let data = persisted.data(using: .utf8),
            let decoded = try? JSONDecoder().decode([Element].self, from: data)
        else {
            return []
        }
        return decoded
    }

    static func encodeList<Element: Encodable>(_ elements: [Element]) -> String {
        guard let data = try? JSONEncoder().encode(elements),
            let text = String(data: data, encoding: .utf8)
        else {
            return ""
        }
        return text
    }
}
