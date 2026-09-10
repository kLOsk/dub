//
//  GlobalRackBarState.swift
//  Dub
//
//  State for the global rack bar — the siren, Quick Scratch and the
//  sampler.
//
//  These three are grouped because they are the racks that are *not*
//  per-deck, however they used to be drawn. There is one siren keymap
//  (`Z X C V B N M ,`) and it fires the focused deck; the sampler's
//  eight slots fire the focused deck too; Quick Scratch is a four-slot
//  table where each slot carries its own target deck, so an index-keyed
//  press has never needed a deck argument. Drawing them once, in one
//  bar, is what the model has always said.
//

import DubCore
import SwiftUI

/// The siren block. State is genuinely per-deck in the engine, so this
/// is a snapshot of whichever deck currently has focus.
struct SirenRackState: Equatable {
    /// The deck the pads fire on — `model.focusedDeckForGridNudge`,
    /// which is exactly what the keyboard already drives.
    var focusedDeck: DeckSide
    var presetNames: [String]
    var sounding: Bool
    var unit: SirenUnit = .gs1
    var dubMacro: Double = 0
}

/// One Quick Scratch pad.
struct TriggerPadState: Equatable, Identifiable {
    var index: Int
    /// The key cap printed under the name.
    var key: String
    /// Display name of the bound sample; `nil` is an empty slot.
    var sampleName: String?
    /// Which deck bus the slot lands on; `nil` when unbound.
    var deck: DeckSide?

    var id: Int { index }
}

/// Everything the bar draws.
struct GlobalRackBarState: Equatable {
    /// `nil` hides the siren block — the Preferences feature gate is
    /// off, or this is Prep, where the siren and its Expert panel have
    /// their own column and a second copy would be duplicate chrome.
    var siren: SirenRackState?
    var quickScratch: [TriggerPadState] = []
    /// The sampler — the same shelf Prep loads, firing here.
    var sampler: SampleShelfState = .empty
}

/// What the bar fires. Separate from the state so the state stays
/// `Equatable`.
struct GlobalRackBarCallbacks {
    var onSirenPreset: (_ index: Int) -> Void = { _ in }
    var onSirenUnit: (_ unit: SirenUnit) -> Void = { _ in }
    var onSirenDubMacro: (_ value: Double) -> Void = { _ in }
    var onQuickScratch: (_ index: Int) -> Void = { _ in }
    var sampler = SampleShelfCallbacks()
}
