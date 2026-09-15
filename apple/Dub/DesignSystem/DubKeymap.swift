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
//  ## Modifiers are not part of the binding
//
//  `⇧` on a hot cue means *clear it*, and `⇧`/`⌥` on the grid tap mean
//  halve/double. Those are arguments to the action, not different
//  bindings, so matching ignores them and the handler reads them. `⌘` is
//  the exception — it genuinely selects a different action (`⌘←` is
//  Instant Doubles, `←` is not), so it is matched.
//

import Foundation

/// What a binding drives.
///
/// The `id` is what a saved profile will persist. **Never renumber or
/// rename one** — a stored map would silently rebind.
enum DubAction: Hashable {
    case loadSelection
    case openPreferences
    case tapGrid
    case hotCue(Int)
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

    var id: String {
        switch self {
        case .loadSelection: return "transport.loadSelection"
        case .openPreferences: return "app.preferences"
        case .tapGrid: return "grid.tap"
        case .hotCue(let i): return "cue.\(i)"
        case .sirenPreset(let i): return "siren.preset.\(i)"
        case .instantDouble(let b): return "deck.instantDouble.\(b ? "b" : "a")"
        case .sampler(let i): return "sampler.\(i)"
        case .quickScratch(let side, let i): return "scratch.\(side == .a ? "a" : "b").\(i)"
        case .fxToggle(let i): return "fx.unit.\(i)"
        case .fxKick: return "fx.kick"
        case .echoOut(let side): return "echo.\(side == .a ? "a" : "b")"
        }
    }

    /// The action a stored id names, or `nil` for an id no version of
    /// the app has issued — a profile from the future, or a typo.
    init?(id: String) {
        let parts = id.split(separator: ".").map(String.init)
        func side(_ s: String) -> DeckSide? { s == "a" ? .a : s == "b" ? .b : nil }
        switch (parts.count, parts.first ?? "") {
        case (2, "transport") where parts[1] == "loadSelection": self = .loadSelection
        case (2, "app") where parts[1] == "preferences": self = .openPreferences
        case (2, "grid") where parts[1] == "tap": self = .tapGrid
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
        case (2, "echo"):
            guard let d = side(parts[1]) else { return nil }
            self = .echoOut(d)
        default: return nil
        }
    }

    /// Whether map mode may rebind it. `⌘,` stays Preferences whatever
    /// happens — it is the way back to everything else.
    var isRemappable: Bool {
        self != .openPreferences
    }
}

/// A key as map mode captures it: the physical code (layout-independent,
/// as the pads' defaults are), whether `⌘` was down, and the glyph the
/// cap prints — derived from the key at capture time so the legend is
/// what the DJ's own keyboard says.
struct DubKeyChord: Codable, Hashable {
    let code: UInt16
    var command: Bool = false
    let legend: String

    /// The cap glyph for a key event. Letters and digits print themselves
    /// (upper-case); the keys with no printable name get their macOS
    /// symbols; a `⌘` chord carries the command glyph.
    static func legend(code: UInt16, characters: String?, command: Bool) -> String {
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
        return command ? "⌘" + base : base
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
    /// used only where the mnemonic matters more than the position — the
    /// `G` of "grid".
    enum Match: Hashable {
        case code(UInt16)
        case character(String)
    }

    let action: DubAction
    var transport: DubTransport = .key
    let match: Match
    /// `⌘` is matched because it selects a different action. Every other
    /// modifier is an argument — see the file header.
    var requiresCommand: Bool = false
    /// The glyph the DJ reads on the cap. One source for the printed
    /// legend and the dispatch.
    let legend: String
    /// `false` for a slot that is reserved but does not fire yet. The pad
    /// still renders, so the omission is visible rather than silent, but
    /// it must not advertise a key that would do nothing. Nothing is
    /// reserved today; the flag stays for map mode.
    var isLive: Bool = true
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
    /// Two entries, and both are app conventions rather than performance
    /// bindings: `⌘,` is Preferences on every Mac, and Space loads the
    /// selection on either surface. Everything the DJ *plays* — hot cues,
    /// the grid tap, instant doubles, the sampler, Quick Scratch, the
    /// siren, echo, the FX rack — starts unbound and is mapped in map
    /// mode (decided 2026-09-15: "we don't need a default keymap"). The
    /// number row, `G` and `⌘←→` used to be pre-assigned; a DJ's hands
    /// know their own keys, and a default that had to be un-learned was
    /// worse than none.
    static let defaults: [DubBinding] = [
        DubBinding(action: .loadSelection, match: .code(49), legend: "SPACE"),
        DubBinding(
            action: .openPreferences, match: .character(","),
            requiresCommand: true, legend: "⌘,"),
    ]

    private static var byAction: [DubAction: DubBinding] {
        DubKeymapStore.shared.resolvedByAction
    }

    // MARK: - Dispatch

    /// The action a key event drives, or `nil` for a key Dub does not own.
    ///
    /// Reserved bindings deliberately return `nil`: the cap is drawn so the
    /// slot is visible, but the key must fall through to the rest of the
    /// app rather than being swallowed by an action that does nothing.
    static func action(
        forKeyCode code: UInt16,
        character: String?,
        command: Bool
    ) -> DubAction? {
        for binding in bindings where binding.transport == .key && binding.isLive {
            guard binding.requiresCommand == command else { continue }
            switch binding.match {
            case .code(let c) where c == code:
                return binding.action
            case .character(let ch)
                where ch.caseInsensitiveCompare(character ?? "") == .orderedSame:
                return binding.action
            default:
                continue
            }
        }
        return nil
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
