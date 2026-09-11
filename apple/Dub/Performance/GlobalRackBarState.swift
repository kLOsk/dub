//
//  GlobalRackBarState.swift
//  Dub
//
//  State for the global rack bar — the siren and the sampler.
//
//  These two are grouped because they are the racks that are *not*
//  per-deck, however they used to be drawn. There is one siren keymap
//  (`Z X C V B N M ,`) and it fires the focused deck; the sampler's
//  eight slots fire the focused deck too. Drawing them once, in one
//  bar, is what the model has always said. Quick Scratch left the bar
//  when it became a per-deck row (PRD §7.2) — a scratch is done *on* a
//  deck, so its pads belong in that deck's column.
//

import DubCore
import SwiftUI

/// The siren block. State is genuinely per-deck in the engine, so this
/// is a snapshot of the deck the rack's output resolves to.
struct SirenRackState: Equatable {
    /// Where the pads fire — the master by default, or the deck(s) the
    /// DJ pinned the rack to.
    var output: RackOutputState
    var presetNames: [String]
    var sounding: Bool
    var unit: SirenUnit = .gs1
    var dubMacro: Double = 0
}

/// Everything the bar draws.
struct GlobalRackBarState: Equatable {
    /// `nil` hides the siren block — the Preferences feature gate is
    /// off, or this is Prep, where the siren and its Expert panel have
    /// their own column and a second copy would be duplicate chrome.
    var siren: SirenRackState?
    /// The sampler — the same shelf Prep loads, firing here.
    var sampler: SampleShelfState = .empty
}

/// What the bar fires. Separate from the state so the state stays
/// `Equatable`.
struct GlobalRackBarCallbacks {
    var onSirenPreset: (_ index: Int) -> Void = { _ in }
    var onSirenUnit: (_ unit: SirenUnit) -> Void = { _ in }
    var onSirenDubMacro: (_ value: Double) -> Void = { _ in }
    /// The siren pill's right-click: pin the rack's output or let it
    /// follow the master again.
    var onSirenOutput: (_ output: RackOutput) -> Void = { _ in }
    var sampler = SampleShelfCallbacks()
}
