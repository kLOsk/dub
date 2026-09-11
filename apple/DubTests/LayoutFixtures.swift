//
//  LayoutFixtures.swift
//  DubTests
//
//  Named state fixtures for the layout suites.
//
//  These exist because the old pad baseline rendered `PerformancePadsView`
//  with the *struct's* defaults, where `sirenEnabled` was false — a
//  configuration the app never runs. The test was green while the real
//  column overflowed its pane by ~144 pt and spilled behind the FX bar.
//
//  A fixture named for what production actually does is harder to drift
//  from than a default value, so tests take these rather than relying on
//  whatever a memberwise initialiser happens to leave unset.
//

@testable import Dub
import DubCore
import SwiftUI

extension PerformancePadsState {
    /// What the model produces at the shipping Preferences defaults:
    /// echo-out on, the per-deck FX rack off (dormant, UI-BACKLOG
    /// F-38), two cues set and a 2-bar loop running.
    static var shippingDefault: PerformancePadsState {
        PerformancePadsState(
            cues: [12.0, nil, 48.5, nil],
            activeLoopBars: 2,
            loopEngaged: true,
            loopInArmed: false,
            echoEnabled: true,
            echoEngaged: false,
            rackEnabled: false)
    }
}

extension GlobalRackBarState {
    /// A populated bar: the GS1 siren and three sampler slots loaded —
    /// one sounding, one tagged for Quick Scratch.
    static func fixture(focus: DeckSide) -> GlobalRackBarState {
        GlobalRackBarState(
            siren: SirenRackState(
                output: RackOutputState(focused: focus),
                presetNames: [
                    "Wail", "Alarm", "Laser", "Bomb",
                    "Riser", "Zap", "Siren", "Horn",
                ],
                sounding: false,
                unit: .gs1,
                dubMacro: 0.35),
            sampler: SampleShelfState(
                slots: [
                    SampleSlotState(index: 0, name: "Horn", playing: true, progress: 0.4),
                    SampleSlotState(index: 1, name: "Reload", quickScratch: 0),
                    SampleSlotState(index: 2),
                    SampleSlotState(index: 3),
                    SampleSlotState(index: 4, name: "Siren Up"),
                    SampleSlotState(index: 5),
                    SampleSlotState(index: 6),
                    SampleSlotState(index: 7),
                ],
                output: RackOutputState(focused: focus)))
    }
}
