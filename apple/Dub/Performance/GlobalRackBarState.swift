//
//  GlobalRackBarState.swift
//  Dub
//
//  State for the global rack bar — the siren and the sampler.
//
//  These two are grouped because they are the racks that are *not*
//  per-deck, however they used to be drawn. There is one siren, firing
//  the focused deck (its keys wait for map mode); the sampler's
//  eight slots fire the focused deck too. Drawing them once, in one
//  bar, is what the model has always said. Quick Scratch left the bar
//  when it became a per-deck row (PRD §7.2) — a scratch is done *on* a
//  deck, so its pads belong in that deck's column.
//

import SwiftUI

/// The siren box. Sounding is per deck in the engine, so that half is a
/// snapshot of the deck(s) the rack's output resolves to; the shot names,
/// the last shot and the knob are the box's own.
struct SirenRackState: Equatable {
    /// Where the keys fire — the master by default, or the deck(s) the
    /// DJ pinned the rack to.
    var output: RackOutputState
    /// The five shots, in fire order.
    var presetNames: [String]
    var sounding: Bool
    /// The shot last fired, as an index into `presetNames`. The display
    /// keeps its name after the tail; the key it lit sinks only while
    /// `sounding`. `nil` before the first press.
    var lastShot: Int? = nil
    /// Counts presses. A re-hit of the shot already sounding changes
    /// nothing else in this state, and the meter still has to kick.
    var fireCount: Int = 0
    /// The DUB knob, 0…1.
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
    var onSirenDubMacro: (_ value: Double) -> Void = { _ in }
    /// The siren pill's right-click: pin the rack's output or let it
    /// follow the master again.
    var onSirenOutput: (_ output: RackOutput) -> Void = { _ in }
    var sampler = SampleShelfCallbacks()
}
