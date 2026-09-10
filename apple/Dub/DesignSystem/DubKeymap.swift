//
//  DubKeymap.swift
//  Dub
//
//  Every binding in the app, in one table.
//
//  Before this there were four copies of the same knowledge and nothing
//  holding them together: three `[UInt16: Int]` dictionaries inline in
//  `KeyEventMonitorHost`'s event closure, and three hand-typed legend
//  arrays (`sirenPresetKeys`, `QuickScratchSlots.keyLabels`, the old
//  sampler's) in the views. A pad's printed cap and the key that
//  actually fires it were separate facts maintained by hand.
//
//  ## The sampler has no bindings here
//
//  It had `A S D F` reserved — drawn, never live. The sampler is eight
//  slots now and a straight extension collides with `G` (the grid tap),
//  so rather than pick a second row nobody has asked for, the sampler's
//  keys wait for map mode, where the DJ picks them. Until then it is
//  mouse-driven, and the pads print no cap at all rather than a dimmed
//  promise.
//
//  `SirenRackGroup`'s own doc comment records what that costs: it shipped
//  two siren racks advertising the same eight keys when only the focused
//  deck's ever fired — "the non-focused copy was advertising keys that did
//  nothing to it". With one table feeding both the dispatcher and the cap,
//  that class of bug is no longer writable.
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
    case quickScratch(Int)
    /// `true` duplicates deck A onto B; `false` is the reverse.
    case instantDouble(toDeckB: Bool)

    var id: String {
        switch self {
        case .loadSelection: return "transport.loadSelection"
        case .openPreferences: return "app.preferences"
        case .tapGrid: return "grid.tap"
        case .hotCue(let i): return "cue.\(i)"
        case .sirenPreset(let i): return "siren.preset.\(i)"
        case .quickScratch(let i): return "quickScratch.\(i)"
        case .instantDouble(let b): return "deck.instantDouble.\(b ? "b" : "a")"
        }
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

/// The default map.
///
/// A `enum` namespace rather than a stored table because remapping is not
/// built yet (M18). When it is, this becomes the *default* a profile is
/// diffed against, and `DubKeymap.bindings` becomes a lookup on the active
/// profile — no call site has to change for that.
enum DubKeymap {

    /// Bottom letter row, in fire order. Layout-independent keyCodes.
    private static let sirenCodes: [UInt16] = [6, 7, 8, 9, 11, 45, 46, 43]
    private static let sirenLegends = ["Z", "X", "C", "V", "B", "N", "M", ","]
    /// `Q W E R`.
    private static let quickScratchCodes: [UInt16] = [12, 13, 14, 15]
    private static let quickScratchLegends = ["Q", "W", "E", "R"]

    static let bindings: [DubBinding] = {
        var all: [DubBinding] = [
            DubBinding(action: .loadSelection, match: .code(49), legend: "SPACE"),
            DubBinding(
                action: .openPreferences, match: .character(","),
                requiresCommand: true, legend: "⌘,"),
            DubBinding(action: .tapGrid, match: .character("g"), legend: "G"),
            DubBinding(
                action: .instantDouble(toDeckB: true), match: .code(124),
                requiresCommand: true, legend: "⌘→"),
            DubBinding(
                action: .instantDouble(toDeckB: false), match: .code(123),
                requiresCommand: true, legend: "⌘←"),
        ]
        // Number row 1–8. The codes are not contiguous past 4 — macOS
        // orders them 18,19,20,21,23,22,26,28 — so they are listed
        // rather than computed, which is also how a typo becomes
        // visible.
        let cueCodes: [UInt16] = [18, 19, 20, 21, 23, 22, 26, 28]
        for (i, code) in cueCodes.enumerated() {
            all.append(
                DubBinding(action: .hotCue(i), match: .code(code), legend: "\(i + 1)"))
        }
        for (i, code) in sirenCodes.enumerated() {
            all.append(
                DubBinding(
                    action: .sirenPreset(i), match: .code(code),
                    legend: sirenLegends[i]))
        }
        for (i, code) in quickScratchCodes.enumerated() {
            all.append(
                DubBinding(
                    action: .quickScratch(i), match: .code(code),
                    legend: quickScratchLegends[i]))
        }
        return all
    }()

    private static let byAction: [DubAction: DubBinding] =
        Dictionary(bindings.map { ($0.action, $0) }, uniquingKeysWith: { first, _ in first })

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
