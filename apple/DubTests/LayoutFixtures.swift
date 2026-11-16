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

extension DeckColumnState {
    /// A loaded, playing deck with all eight cues named — the tallest the
    /// column gets in a shipping configuration.
    static func loadedFixture(side: DeckSide) -> DeckColumnState {
        DeckColumnState(
            side: side,
            header: DeckColumnHeader(DeckHeaderState(
                isLive: true, source: .file,
                trackTitle: "Baddadan (extended mix) (feat. IRah, Flowdan)",
                trackArtist: "Chase & Status",
                bpm: 176.0, pitchPercent: -1.2, timecodeLockState: 1,
                key: "7A", formatChip: "MP3 · 44.1 kHz · stereo",
                timeRow: .remainingOnly,
                isMaster: side == .a, isPlaying: true,
                isPanicPlay: false, useTimecodeToggle: false,
                gridLocked: false, gridDriftQuality: nil)),
            cues: (0..<8).map {
                CueSlotState(
                    index: $0,
                    mark: CueMark(positionSecs: Double($0) * 30, name: "MARK", color: "aqua"))
            },
            activeLoopBeats: 2, loopEngaged: true,
            echoEnabled: true, echoEngaged: false,
            hasTrack: true, isPlaying: true)
    }
}

extension GlobalRackBarState {
    /// A populated bar: the siren box after one shot, its knob part
    /// way up, and three sampler slots loaded — one sounding, one tagged
    /// for Quick Scratch.
    static func fixture(focus: DeckSide) -> GlobalRackBarState {
        GlobalRackBarState(
            siren: SirenRackState(
                output: RackOutputState(focused: focus),
                presetNames: ["Rifle Gun", "Alarm", "Sine", "Laser", "Siren"],
                sounding: false,
                lastShot: 3,
                dubMacro: 0.35),
            sampler: SampleShelfState(
                slots: [
                    SampleSlotState(index: 0, name: "Horn", playing: true, progress: 0.4),
                    SampleSlotState(index: 1, name: "Reload", quickScratch: 0),
                    SampleSlotState(index: 2),
                    SampleSlotState(index: 3),
                    SampleSlotState(index: 4, name: "Siren Up", fault: .missing("siren-up.wav")),
                    SampleSlotState(index: 5),
                    SampleSlotState(index: 6),
                    SampleSlotState(index: 7),
                ],
                output: RackOutputState(focused: focus)))
    }
}
