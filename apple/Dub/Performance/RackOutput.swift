//
//  RackOutput.swift
//  Dub
//
//  Where a global rack lands: the siren's and the sampler's `→ A` pill.
//
//  By default a rack follows the master deck — the one the crowd is
//  hearing — so a horn or a siren comes out of the channel that is up.
//  That is right nearly always and wrong sometimes: a DJ mixing on B
//  who wants the siren on A's channel, or a horn over *both* channels
//  at once. The pill is where the rule shows, so the pill is where it
//  is overridden: right-click → Auto · A · B · A+B. The override is
//  remembered per rack.
//

import Foundation

/// A rack's output rule.
enum RackOutput: String, Codable, CaseIterable {
    /// Follow the master deck (default).
    case auto
    case a
    case b
    /// Both deck buses at once.
    case both

    var menuTitle: String {
        switch self {
        case .auto: return "Auto — follows the master deck"
        case .a: return "Deck A"
        case .b: return "Deck B"
        case .both: return "A + B"
        }
    }
}

/// A rack's output rule resolved against the current master, as the
/// pill draws it and the rack fires it.
struct RackOutputState: Equatable {
    var output: RackOutput
    /// The master deck, which `.auto` resolves to.
    var focused: DeckSide
    /// The deck that is the DUB FX channel, if one is (F-38). A rack
    /// landing there goes *through* the rack — the siren through the
    /// Space Echo — and the pill says so: `→ FX`.
    var fxDeck: DeckSide? = nil

    init(_ output: RackOutput = .auto, focused: DeckSide, fxDeck: DeckSide? = nil) {
        self.output = output
        self.focused = focused
        self.fxDeck = fxDeck
    }

    /// The decks the rack lands on, in order. One for a single deck,
    /// both for `A+B`.
    var decks: [DeckSide] {
        switch output {
        case .auto: return [focused]
        case .a: return [.a]
        case .b: return [.b]
        case .both: return [.a, .b]
        }
    }

    /// The deck whose state a rack shows when it can only show one —
    /// the siren's unit and knob. `A+B` shows A's; the writes go to both.
    var primary: DeckSide { decks[0] }

    /// `true` when the DJ pinned the rack rather than letting it follow.
    var isPinned: Bool { output != .auto }

    /// What the pill prints.
    var label: String {
        switch decks {
        case [.a]: return fxDeck == .a ? "→ FX" : "→ A"
        case [.b]: return fxDeck == .b ? "→ FX" : "→ B"
        default: return "→ A+B"
        }
    }

    /// Whether the rack lands on the FX channel — fires into the rack.
    var intoFx: Bool {
        decks.count == 1 && decks[0] == fxDeck
    }

    /// The menu's title for an option, naming the FX channel where a
    /// deck letter would otherwise mislead.
    func menuTitle(_ option: RackOutput) -> String {
        switch option {
        case .a where fxDeck == .a: return "Deck A — the FX channel"
        case .b where fxDeck == .b: return "Deck B — the FX channel"
        default: return option.menuTitle
        }
    }

    /// The deck the pill's tint comes from; `nil` for `A+B`, which has
    /// no one deck's colour.
    var tintDeck: DeckSide? {
        decks.count == 1 ? decks[0] : nil
    }
}
