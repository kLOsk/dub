//
//  DubKeymap.swift
//  Dub
//
//  Every binding in the app, in one table.
//
//  Before this there were four copies of the same knowledge and nothing
//  holding them together: three `[UInt16: Int]` dictionaries inline in
//  `KeyEventMonitorHost`'s event closure, and three hand-typed legend
//  arrays (`sirenPresetKeys` and the old Quick Scratch and sampler
//  tables') in the views. A pad's printed cap and the key that actually
//  fires it were separate facts maintained by hand.
//
//  ## Nothing you play has a default key
//
//  The sampler had `A S D F` reserved — drawn, never live — and Quick
//  Scratch had `Q W E R` live, each key bound to a slot with its own
//  target deck. The siren had `Z X C V B` until 2026-09-12, when Daniel
//  asked for them to go; the number row on the hot cues, `G` on the grid
//  tap and `⌘←→` on instant doubles followed on 2026-09-15, when map mode
//  landed and the pre-assigned rows became a default that had to be
//  un-learned. Every performance control now starts unbound and prints
//  no cap until the DJ maps it (`DubKeymapStore`); only the two app
//  conventions, Space and `⌘,`, ship bound.
//
//  `SirenRackGroup`'s own doc comment records what a hand-typed legend
//  costs: it shipped two siren racks advertising the same keys when only
//  the focused deck's ever fired — "the non-focused copy was advertising
//  keys that did nothing to it". With one table feeding both the
//  dispatcher and the cap, that class of bug is no longer writable.
//
//  ## Why a transport lives on a binding that only ever uses one
//
//  Only `.key` dispatches today. `.midi` and `.hid` are here because
//  profiles have to span all three, and a binding written without a
//  transport would need a migration the day the second one lands. It costs
//  one field now and saves a schema change later.
//
//  ## Modifiers: part of the key first, an argument second
//
//  A key carries every modifier held: `⇧←` (load deck A) and `←` are
//  two keys. When nothing is bound to the exact key, an unclaimed `⇧`
//  or `⌥` falls back to the plain key as an argument — `⇧` on a hot cue
//  means *clear it*, `⇧`/`⌥` on the BPM tap mean halve/double. Until
//  2026-11-17 only `⌘` was part of a key; the default map's `⇧`/`⌘⇧`
//  arrows needed the rest.
//

import Foundation

/// What a binding drives.
///
/// The `id` is what a saved profile will persist. **Never renumber or
/// rename one** — a stored map would silently rebind.
enum DubAction: Hashable {
    /// Space's default: the selection onto the deck that is not the master.
    case loadSelection
    /// The selection onto one deck, whichever is playing.
    case loadInto(DeckSide)
    case openPreferences
    /// A deck's loop off — the loop row's ✕, without finding it.
    case loopOff(DeckSide)
    /// Put the cursor in the library's search field.
    case focusSearch
    /// The library selection one row down (`true`) or up.
    case browse(next: Bool)
    /// The sidebar's selection one playlist down (`true`) or up.
    case sidebar(next: Bool)
    /// Open (`true`) or close the selected folder in the sidebar.
    case folder(open: Bool)
    /// A hot cue on whichever deck is the master — what a pad key was
    /// before the cues were per deck (2026-11-17). Kept so a key mapped
    /// then still works.
    case hotCue(Int)
    /// A hot cue on one deck: the default number row is split, 1–5 on A
    /// and 6–0 on B.
    case deckCue(DeckSide, Int)
    case sirenPreset(Int)
    /// `true` duplicates deck A onto B; `false` is the reverse.
    case instantDouble(toDeckB: Bool)
    /// A sampler slot, 0…7 — fires on the master deck like the tile.
    case sampler(Int)
    /// A deck's Quick Scratch pad, 0…3 (PRD §7.2) — the pads are per
    /// deck, which is the deck dimension the old `Q W E R` lacked.
    case quickScratch(DeckSide, Int)
    /// A DUB FX rack unit's IN/OUT toggle, in rack order 0…3 (Big Knob ·
    /// phaser · Space Echo · spring). Fires on whichever deck is the FX
    /// channel; nothing when none is.
    case fxToggle(Int)
    /// KICK the FX channel's spring tank.
    case fxKick
    /// A deck's ECHO OUT (tap-toggle).
    case echoOut(DeckSide)
    /// HOLD on a deck's echo-out (toggle) — the record swap.
    case echoHold(DeckSide)
    /// HOLD on the FX channel's Space Echo (toggle).
    case fxHold
    /// TAP the Space Echo's tempo, for SYNC with a real record.
    case fxTap
    /// A deck's key lock (M14). Per deck, like the platter it holds.
    case keyLock(DeckSide)
    /// Tap a deck's BPM, as clicking it does: on THRU the record's 1 and
    /// tempo (held until the needle comes up), on a track the grid's —
    /// 1–2 taps set the 1, 3+ the tempo. With `⇧` it halves a track's
    /// tempo, with `⌥` doubles it (the BPM menu's ½ and 2×). It absorbed
    /// Tap the grid on 2026-11-17, which did the same less well. The id
    /// stays `tempo.tap.*` so keys mapped before keep working.
    case tapTempo(DeckSide)

    var id: String {
        switch self {
        case .loadSelection: return "transport.loadSelection"
        case .loadInto(let side): return "transport.load.\(side == .a ? "a" : "b")"
        case .openPreferences: return "app.preferences"
        case .loopOff(let side): return "loop.off.\(side == .a ? "a" : "b")"
        case .focusSearch: return "library.search"
        case .browse(let next): return "library.\(next ? "next" : "previous")"
        case .sidebar(let next): return "library.playlist.\(next ? "next" : "previous")"
        case .folder(let open): return "library.folder.\(open ? "open" : "close")"
        case .deckCue(let side, let i): return "cue.\(side == .a ? "a" : "b").\(i)"
        case .hotCue(let i): return "cue.\(i)"
        case .sirenPreset(let i): return "siren.preset.\(i)"
        case .instantDouble(let b): return "deck.instantDouble.\(b ? "b" : "a")"
        case .sampler(let i): return "sampler.\(i)"
        case .quickScratch(let side, let i): return "scratch.\(side == .a ? "a" : "b").\(i)"
        case .fxToggle(let i): return "fx.unit.\(i)"
        case .fxKick: return "fx.kick"
        case .echoOut(let side): return "echo.\(side == .a ? "a" : "b")"
        case .echoHold(let side): return "echo.hold.\(side == .a ? "a" : "b")"
        case .fxHold: return "fx.hold"
        case .fxTap: return "fx.tap"
        case .keyLock(let side): return "keylock.\(side == .a ? "a" : "b")"
        case .tapTempo(let side): return "tempo.tap.\(side == .a ? "a" : "b")"
        }
    }

    /// The action a stored id names, or `nil` for an id no version of
    /// the app has issued — a profile from the future, or a typo.
    init?(id: String) {
        let parts = id.split(separator: ".").map(String.init)
        func side(_ s: String) -> DeckSide? { s == "a" ? .a : s == "b" ? .b : nil }
        switch (parts.count, parts.first ?? "") {
        case (2, "transport") where parts[1] == "loadSelection": self = .loadSelection
        case (3, "transport") where parts[1] == "load":
            guard let d = side(parts[2]) else { return nil }
            self = .loadInto(d)
        case (2, "app") where parts[1] == "preferences": self = .openPreferences
        // `grid.tap` (the master deck's, until 2026-11-17) and `grid.tap.a`
        // / `.b` (per deck, that one day) are gone: Tap the BPM does what
        // they did, ⇧ / ⌥ included. They parse to nothing.
        case (3, "loop") where parts[1] == "off":
            guard let d = side(parts[2]) else { return nil }
            self = .loopOff(d)
        case (2, "library") where parts[1] == "search": self = .focusSearch
        case (2, "library") where parts[1] == "next": self = .browse(next: true)
        case (2, "library") where parts[1] == "previous": self = .browse(next: false)
        case (3, "library") where parts[1] == "playlist" && ["next", "previous"].contains(parts[2]):
            self = .sidebar(next: parts[2] == "next")
        case (3, "library") where parts[1] == "folder" && ["open", "close"].contains(parts[2]):
            self = .folder(open: parts[2] == "open")
        case (3, "cue"):
            guard let d = side(parts[1]), let n = Int(parts[2]) else { return nil }
            self = .deckCue(d, n)
        case (2, "cue"):
            guard let n = Int(parts[1]) else { return nil }
            self = .hotCue(n)
        case (3, "siren") where parts[1] == "preset":
            guard let n = Int(parts[2]) else { return nil }
            self = .sirenPreset(n)
        case (3, "deck") where parts[1] == "instantDouble":
            guard let d = side(parts[2]) else { return nil }
            self = .instantDouble(toDeckB: d == .b)
        case (2, "sampler"):
            guard let n = Int(parts[1]) else { return nil }
            self = .sampler(n)
        case (3, "scratch"):
            guard let d = side(parts[1]), let n = Int(parts[2]) else { return nil }
            self = .quickScratch(d, n)
        case (3, "fx") where parts[1] == "unit":
            guard let n = Int(parts[2]) else { return nil }
            self = .fxToggle(n)
        case (2, "fx") where parts[1] == "kick": self = .fxKick
        case (2, "fx") where parts[1] == "hold": self = .fxHold
        case (2, "fx") where parts[1] == "tap": self = .fxTap
        case (3, "echo") where parts[1] == "hold":
            guard let d = side(parts[2]) else { return nil }
            self = .echoHold(d)
        case (2, "echo"):
            guard let d = side(parts[1]) else { return nil }
            self = .echoOut(d)
        case (2, "keylock"):
            guard let d = side(parts[1]) else { return nil }
            self = .keyLock(d)
        case (3, "tempo") where parts[1] == "tap":
            guard let d = side(parts[2]) else { return nil }
            self = .tapTempo(d)
        default: return nil
        }
    }

    /// A hot cue, on a deck or on the master.
    var isHotCue: Bool {
        switch self {
        case .hotCue, .deckCue: return true
        default: return false
        }
    }

    /// Whether map mode may rebind it. `⌘,` stays Preferences whatever
    /// happens — it is the way back to everything else.
    var isRemappable: Bool {
        self != .openPreferences
    }
}

/// A key as map mode captures it: the physical code (layout-independent,
/// as the pads' defaults are), the modifiers held, and the glyph the cap
/// prints — derived from the key at capture time so the legend is what
/// the DJ's own keyboard says.
///
/// `⇧`, `⌃` and `⌥` joined `⌘` as part of a key on 2026-11-17, for the
/// default map's `⇧←` (load A); `⌃` for whatever the DJ maps. A map saved
/// before then has no such fields; they decode as "not held".
struct DubKeyChord: Codable, Hashable {
    let code: UInt16
    var command: Bool = false
    var shift: Bool = false
    var control: Bool = false
    var option: Bool = false
    let legend: String

    init(
        code: UInt16, command: Bool = false, shift: Bool = false,
        control: Bool = false, option: Bool = false, legend: String
    ) {
        self.code = code
        self.command = command
        self.shift = shift
        self.control = control
        self.option = option
        self.legend = legend
    }

    private enum CodingKeys: String, CodingKey { case code, command, shift, control, option, legend }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        code = try c.decode(UInt16.self, forKey: .code)
        command = try c.decodeIfPresent(Bool.self, forKey: .command) ?? false
        shift = try c.decodeIfPresent(Bool.self, forKey: .shift) ?? false
        control = try c.decodeIfPresent(Bool.self, forKey: .control) ?? false
        option = try c.decodeIfPresent(Bool.self, forKey: .option) ?? false
        legend = try c.decode(String.self, forKey: .legend)
    }

    /// The cap glyph for a key event. Letters and digits print themselves
    /// (upper-case); the keys with no printable name get their macOS
    /// symbols; modifiers lead in the order macOS prints them, ⌃⌥⇧⌘.
    /// `characters` is the key with no modifier applied — `1`, not `!`.
    static func legend(
        code: UInt16, characters: String?, command: Bool,
        shift: Bool = false, control: Bool = false, option: Bool = false
    ) -> String {
        let special: [UInt16: String] = [
            49: "SPACE", 36: "↩", 76: "⌅", 48: "⇥", 51: "⌫", 117: "⌦",
            123: "←", 124: "→", 125: "↓", 126: "↑", 115: "↖", 119: "↘",
            116: "⇞", 121: "⇟",
            122: "F1", 120: "F2", 99: "F3", 118: "F4", 96: "F5", 97: "F6",
            98: "F7", 100: "F8", 101: "F9", 109: "F10", 103: "F11", 111: "F12",
        ]
        let base: String
        if let name = special[code] {
            base = name
        } else if let ch = characters, !ch.isEmpty, ch.unicodeScalars.allSatisfy({ $0.value >= 0x20 }) {
            base = ch.uppercased()
        } else {
            base = "#\(code)"
        }
        return (control ? "⌃" : "") + (option ? "⌥" : "") + (shift ? "⇧" : "")
            + (command ? "⌘" : "") + base
    }
}

/// How a binding arrives. Only `.key` dispatches today — see the file
/// header for why the other two exist already.
enum DubTransport: String, Codable, Hashable {
    case key
    case midi
    case hid
}

/// One binding.
struct DubBinding: Hashable {
    /// How the event is recognised.
    ///
    /// A physical `code` survives a non-QWERTY layout, which is why the
    /// pads use it. `character` follows the printed letter instead, and is
    /// used only where the mnemonic matters more than the position.
    enum Match: Hashable {
        case code(UInt16)
        case character(String)
    }

    let action: DubAction
    var transport: DubTransport = .key
    let match: Match
    /// The modifiers that are part of the key. A press must hold exactly
    /// these; see `DubKeymap.binding(...)` for the fallback that lets an
    /// unbound `⇧`/`⌥` reach a plain key as an argument.
    var requiresCommand: Bool = false
    var requiresShift: Bool = false
    var requiresControl: Bool = false
    var requiresOption: Bool = false
    /// The glyph the DJ reads on the cap. One source for the printed
    /// legend and the dispatch.
    let legend: String
    /// `false` for a slot that is reserved but does not fire yet. The pad
    /// still renders, so the omission is visible rather than silent, but
    /// it must not advertise a key that would do nothing. Nothing is
    /// reserved today; the flag stays for map mode.
    var isLive: Bool = true

    /// Whether any modifier other than `⌘` is part of the key.
    var hasArgumentModifiers: Bool { requiresShift || requiresControl || requiresOption }

    /// A default bound by physical key.
    static func key(
        _ action: DubAction, _ code: UInt16, _ legend: String,
        command: Bool = false, shift: Bool = false, control: Bool = false
    ) -> DubBinding {
        DubBinding(
            action: action, match: .code(code), requiresCommand: command,
            requiresShift: shift, requiresControl: control,
            legend: (control ? "⌃" : "") + (shift ? "⇧" : "") + (command ? "⌘" : "") + legend)
    }
}

/// The map: the defaults below, overlaid with what the DJ bound in map
/// mode (`DubKeymapStore`). `bindings` is the resolved table every call
/// site reads — the dispatcher and the printed caps alike — so a rebind
/// moves the key and the legend together, and nothing had to change at
/// the call sites when remapping landed (M18).
enum DubKeymap {

    /// The resolved table: defaults with the DJ's overrides applied.
    static var bindings: [DubBinding] { DubKeymapStore.shared.resolved }

    /// The default map — what a profile is diffed against.
    ///
    /// From 2026-09-15 to 2026-11-17 only Space and `⌘,` shipped bound
    /// ("we don't need a default keymap"): a DJ's hands know their own
    /// keys. Daniel set this map on 2026-11-17 so Dub plays from the
    /// keyboard out of the box; every key here moves in map mode or
    /// Preferences ▸ Keys like any other, and a key the DJ bound wins
    /// over a default that wanted it.
    ///
    /// The layout splits the keyboard down the middle, left hand for deck
    /// A, right hand for deck B: hot cues on the number row, the BPM tap
    /// on the outer keys of the row below (Q / P), echo out on its middle
    /// (T / Y), loop off next to the tap (W / O), Quick Scratch on the
    /// home row (A S D F / H J K L), the sampler on the bottom row. Loads
    /// and doubles ride the arrows: `⇧` loads, `⌘⇧` doubles onto that
    /// side's deck — Serato's keys on a Mac, where its "control" is `⌘`
    /// (2026-11-17; `⌃⇧` first, which also collided with macOS's Spaces).
    /// `⌘⇧↑↓` walk the sidebar as Serato's crates; `⌘←→` close and open
    /// a folder. Plain `←` / `→` stay free: Daniel maps echo out there.
    static let defaults: [DubBinding] = {
        var map: [DubBinding] = [
            DubBinding(action: .loadSelection, match: .code(49), legend: "SPACE"),
            DubBinding(
                action: .openPreferences, match: .character(","),
                requiresCommand: true, legend: "⌘,"),
            .key(.focusSearch, 3, "F", command: true),
            .key(.browse(next: true), 125, "↓"),
            .key(.browse(next: false), 126, "↑"),
            .key(.sidebar(next: true), 125, "↓", command: true, shift: true),
            .key(.sidebar(next: false), 126, "↑", command: true, shift: true),
            .key(.folder(open: true), 124, "→", command: true),
            .key(.folder(open: false), 123, "←", command: true),
            .key(.loadInto(.a), 123, "←", shift: true),
            .key(.loadInto(.b), 124, "→", shift: true),
            .key(.instantDouble(toDeckB: false), 123, "←", command: true, shift: true),
            .key(.instantDouble(toDeckB: true), 124, "→", command: true, shift: true),
            .key(.tapTempo(.a), 12, "Q"),
            .key(.tapTempo(.b), 35, "P"),
            .key(.loopOff(.a), 13, "W"),
            .key(.loopOff(.b), 31, "O"),
            .key(.echoOut(.a), 17, "T"),
            .key(.echoOut(.b), 16, "Y"),
        ]
        // 1–5 on A, 6–0 on B. macOS key codes are not in row order.
        let cueKeys: [(UInt16, String)] = [
            (18, "1"), (19, "2"), (20, "3"), (21, "4"), (23, "5"),
            (22, "6"), (26, "7"), (28, "8"), (25, "9"), (29, "0"),
        ]
        for (i, key) in cueKeys.enumerated() {
            map.append(.key(.deckCue(i < 5 ? .a : .b, i % 5), key.0, key.1))
        }
        let scratchKeys: [(UInt16, String)] = [
            (0, "A"), (1, "S"), (2, "D"), (3, "F"), (4, "H"), (38, "J"), (40, "K"), (37, "L"),
        ]
        for (i, key) in scratchKeys.enumerated() {
            map.append(.key(.quickScratch(i < 4 ? .a : .b, i % 4), key.0, key.1))
        }
        let sampleKeys: [(UInt16, String)] = [
            (6, "Z"), (7, "X"), (8, "C"), (9, "V"), (11, "B"), (45, "N"), (46, "M"), (43, ","),
        ]
        for (i, key) in sampleKeys.enumerated() {
            map.append(.key(.sampler(i), key.0, key.1))
        }
        return map
    }()

    /// Actions with no control on the surface for map mode to click, so
    /// Preferences ▸ Keys binds them instead. Before this list, the grid
    /// tap and instant doubles had handlers and names but nothing could
    /// reach them (from 2026-09-15, when their defaults went).
    static let buttonless: [DubAction] = [
        .loadSelection, .loadInto(.a), .loadInto(.b),
        .instantDouble(toDeckB: false), .instantDouble(toDeckB: true),
        .loopOff(.a), .loopOff(.b),
        .focusSearch, .browse(next: true), .browse(next: false),
        .sidebar(next: true), .sidebar(next: false), .folder(open: true), .folder(open: false),
    ]

    private static var byAction: [DubAction: DubBinding] {
        DubKeymapStore.shared.resolvedByAction
    }

    // MARK: - Dispatch

    /// The binding a key event drives, or `nil` for a key Dub does not own.
    ///
    /// Two passes. First the exact key, every modifier included: `⇧←`
    /// is its own key (load deck A). Then, only if nothing holds the exact
    /// key and `⌃` is not down, the plain key with `⌘` as pressed: `⇧` and
    /// `⌥` reach it as arguments — `⇧` on a hot cue clears it, `⇧`/`⌥` on
    /// the grid tap halve/double. The dispatcher reads them only when the
    /// binding didn't claim them (`argumentShift` / `argumentOption`).
    ///
    /// Reserved bindings deliberately return `nil`: the cap is drawn so the
    /// slot is visible, but the key must fall through to the rest of the
    /// app rather than being swallowed by an action that does nothing.
    static func binding(
        forKeyCode code: UInt16,
        character: String?,
        command: Bool,
        shift: Bool = false,
        control: Bool = false,
        option: Bool = false
    ) -> DubBinding? {
        let live = bindings.filter { $0.transport == .key && $0.isLive && fires($0, code, character) }
        if let exact = live.first(where: {
            $0.requiresCommand == command && $0.requiresShift == shift
                && $0.requiresControl == control && $0.requiresOption == option
        }) {
            return exact
        }
        guard !control else { return nil }
        if let plain = live.first(where: { $0.requiresCommand == command && !$0.hasArgumentModifiers }) {
            return plain
        }
        // Serato clears a cue with ⌘⇧ + its key; Dub's ⇧ alone does too.
        // ⌘⇧ on a cue key nothing else holds reaches the cue, ⇧ as clear.
        guard command, shift else { return nil }
        return live.first {
            !$0.requiresCommand && !$0.hasArgumentModifiers && $0.action.isHotCue
        }
    }

    /// The action of `binding(...)` — what most callers need.
    static func action(
        forKeyCode code: UInt16,
        character: String?,
        command: Bool,
        shift: Bool = false,
        control: Bool = false,
        option: Bool = false
    ) -> DubAction? {
        binding(
            forKeyCode: code, character: character, command: command,
            shift: shift, control: control, option: option)?.action
    }

    private static func fires(_ binding: DubBinding, _ code: UInt16, _ character: String?) -> Bool {
        switch binding.match {
        case .code(let c): return c == code
        case .character(let ch): return ch.caseInsensitiveCompare(character ?? "") == .orderedSame
        }
    }

    // MARK: - Rendering

    /// The cap glyph for an action, or `nil` when nothing is bound.
    static func legend(for action: DubAction) -> String? {
        byAction[action]?.legend
    }

    /// Whether the action's key fires today. A reserved slot renders its
    /// cap dimmed rather than pretending.
    static func isLive(_ action: DubAction) -> Bool {
        byAction[action]?.isLive ?? false
    }

    /// Legends for an indexed family, in order — what a pad row draws.
    static func legends(_ make: (Int) -> DubAction, count: Int) -> [String] {
        (0..<count).map { legend(for: make($0)) ?? "" }
    }
}
