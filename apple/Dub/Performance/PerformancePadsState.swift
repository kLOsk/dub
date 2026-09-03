//
//  PerformancePadsState.swift
//  Dub
//
//  The value snapshot `PerformancePadsView` renders from, and the
//  callbacks it fires.
//
//  Both deck panes used to construct the pad view with a twenty-
//  argument initialiser, written out twice — once for deck A, once for
//  deck B — which meant every new control was a two-site edit and the
//  two lists could silently disagree. Splitting state from callbacks
//  follows the same convention as `DeckHeaderState` /
//  `DeckHeaderCallbacks` and `RipReviewPanelState`: closures are not
//  `Equatable`, so keeping them out of the state struct lets SwiftUI
//  skip re-rendering a pad column whose state has not moved.
//

import DubCore
import SwiftUI

/// Everything one deck's pad column draws. A pure snapshot of
/// `WaveformAppModel` + that deck's `DeckState` — no engine handles, no
/// closures — so the column snapshot-tests without a live model.
struct PerformancePadsState: Equatable {
    /// Hot-cue positions in track seconds; `nil` is an empty slot.
    var cues: [Double?] = [nil, nil, nil, nil]

    /// Active reverse-loop length in bars; `nil` is no loop.
    var activeLoopBars: Double?
    /// A loop is running — a manual in/out region lights no length pad,
    /// so ✕ needs its own signal.
    var loopEngaged: Bool = false
    /// A manual Loop In is armed, waiting for OUT.
    var loopInArmed: Bool = false

    /// Preferences gate; when off the ECHO OUT button is not rendered.
    var echoEnabled: Bool = true
    var echoEngaged: Bool = false

    // The siren is not here. It is one instrument with one keymap
    // firing the focused deck, so it lives once in `GlobalRackBar`
    // rather than twice in the deck columns. Its ~134 pt per column is
    // most of what used to overflow this pane.

    /// Preferences gate, default off (the per-deck rack is dormant
    /// pending the FX-channel rebuild — UI-BACKLOG F-38).
    var rackEnabled: Bool = false
    var rackActive: [Bool] = [false, false, false, false]
    var rackMacro: [Double] = [0.5, 0.5, 0.5, 0.5]
}

/// What the pads fire. Separate from the state so the state stays
/// `Equatable`.
struct PerformancePadsCallbacks {
    /// Set / jump a hot cue; `clear` is a ⇧-click. Performance gained
    /// this when the CUE row moved onto the shared `CuePadSection` —
    /// Prep's pads were always clickable and there is no reason the
    /// same pad should be inert on the other surface (PRD §6.2.1: a
    /// hot-cue press is a momentary trigger, so the mouse is fine).
    var onCue: (_ index: Int, _ clear: Bool) -> Void = { _, _ in }
    /// Fire a grid-snapped reverse loop of `bars` bars — the bars just
    /// heard.
    var onLoop: (_ bars: Double) -> Void = { _ in }
    var onLoopIn: () -> Void = {}
    var onLoopOut: () -> Void = {}
    var onExit: () -> Void = {}
    var onEchoToggle: () -> Void = {}
    var onRackToggle: (_ index: Int) -> Void = { _ in }
    var onRackMacro: (_ index: Int, _ value: Double) -> Void = { _, _ in }
}
