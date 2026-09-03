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

    /// Preferences gate; when off the siren block is not rendered.
    var sirenEnabled: Bool = false
    var sirenPresetNames: [String] = []
    var sirenSounding: Bool = false
    var sirenDubMacro: Double = 0
    var sirenUnit: SirenUnit = .gs1

    /// Preferences gate, default off (the per-deck rack is dormant
    /// pending the FX-channel rebuild — UI-BACKLOG F-38).
    var rackEnabled: Bool = false
    var rackActive: [Bool] = [false, false, false, false]
    var rackMacro: [Double] = [0.5, 0.5, 0.5, 0.5]
}

/// What the pads fire. Separate from the state so the state stays
/// `Equatable`.
struct PerformancePadsCallbacks {
    /// Fire a grid-snapped reverse loop of `bars` bars — the bars just
    /// heard.
    var onLoop: (_ bars: Double) -> Void = { _ in }
    var onLoopIn: () -> Void = {}
    var onLoopOut: () -> Void = {}
    var onExit: () -> Void = {}
    var onEchoToggle: () -> Void = {}
    var onSirenPreset: (_ index: Int) -> Void = { _ in }
    var onSirenDubMacro: (_ value: Double) -> Void = { _ in }
    var onSirenUnit: (_ unit: SirenUnit) -> Void = { _ in }
    var onRackToggle: (_ index: Int) -> Void = { _ in }
    var onRackMacro: (_ index: Int, _ value: Double) -> Void = { _, _ in }
}
