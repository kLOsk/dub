//
//  FxChannelState.swift
//  Dub
//
//  What the DUB FX channel pane draws and fires (F-38 stage 2). A deck
//  switched to DUB FX stops being a turntable: the record is replaced by
//  the vintage rack, on the mixer's send. Values in, closures out, like
//  `DeckColumn`, so the pane snapshot-tests without an engine.
//

import DubCore
import SwiftUI

/// What is patched into the channel's input pair: the mixer's aux send
/// or FX loop (the common case — deck A, the MC's mic and the siren are
/// then all inputs by the mixer's send knob), or a mic straight into the
/// interface for a battle mixer with no send. A label for now — the
/// engine takes the pair either way; stage 3 is the routing.
enum FxInputKind: String, Codable, CaseIterable {
    case send
    case mic

    var title: String {
        switch self {
        case .send: return "SEND"
        case .mic: return "MIC"
        }
    }
}

/// The rack's Expert controls — each unit's own knobs, in the unit's own
/// units (the macro is the Advanced shortcut over the same engine state;
/// Expert is the only mode the channel has). Defaults are a usable
/// starting point, not the engine's construction values: the channel
/// pushes these when the deck is switched to DUB FX.
struct FxRackControls: Equatable {
    /// The Altec's detent, 0…10 (`bigKnobStepHz` gives the Hz).
    var bigKnobStep: Int = 3
    /// Resonance, 0.5…8; ~0.7 is the passive unit.
    var bigKnobQ: Double = 0.7

    /// LFO rate in Hz, 0.05…10.
    var phaserRateHz: Double = 0.4
    var phaserDepth: Double = 0.7
    /// 0…0.95.
    var phaserFeedback: Double = 0.45
    var phaserMix: Double = 0.5

    var spaceEchoMode: SpaceEchoMode = .tripleReverb
    /// The longest head's delay, 20…750 ms; heads 1 and 2 follow at
    /// 0.337 / 0.668 of it.
    var spaceEchoRepeatMs: Double = 420
    /// Feedback, 0…1.2 — past 1 the tape self-oscillates.
    var spaceEchoIntensity: Double = 0.62
    var spaceEchoVolume: Double = 0.6
    var spaceEchoReverb: Double = 0.3
    /// Tape age, 0…2.
    var spaceEchoWowFlutter: Double = 0.3

    var springDecay: Double = 0.6
    /// 0 dark → 1 bright (`springToneHz` gives the Hz).
    var springTone: Double = 0.5
    var springWet: Double = 0.35

    /// The Space Echo's selector positions, in dial order.
    static let spaceEchoModes: [SpaceEchoMode] = [
        .reverb, .short, .long, .triple, .shortReverb, .longReverb, .tripleReverb,
    ]

    /// The tape self-oscillates past unity feedback.
    var spaceEchoRunaway: Bool { spaceEchoIntensity > 1.0 }
}

/// The four units in the rack's order — the engine's, top to bottom:
/// the inserts first, then the sends. `slot` is the `RackFx` index the
/// engaged flags are kept under.
enum FxRackUnit: Int, CaseIterable {
    case bigKnob
    case phaser
    case spaceEcho
    case spring

    /// The unit's index in `DeckState.rackActive` (`RackFx` order:
    /// Spring · SpaceEcho · BigKnob · Phaser).
    var slot: Int {
        switch self {
        case .spring: return 0
        case .spaceEcho: return 1
        case .bigKnob: return 2
        case .phaser: return 3
        }
    }

    var title: String {
        switch self {
        case .bigKnob: return "BIG KNOB"
        case .phaser: return "PHASER"
        case .spaceEcho: return "SPACE ECHO"
        case .spring: return "SPRING"
        }
    }

    var tint: Color {
        switch self {
        case .bigKnob: return DubColor.bigKnob
        case .phaser: return DubColor.phaser
        case .spaceEcho: return DubColor.spaceEcho
        case .spring: return DubColor.springFx
        }
    }
}

/// Everything the pane draws.
struct FxChannelState: Equatable {
    var side: DeckSide = .b
    /// The source switch's state — the switch stays at the top of the
    /// column so the way back is where it always was.
    var isPlaying: Bool = false
    var sourceOverridden: Bool = false
    var input: FxInputKind = .send
    /// The interface input pair the channel takes, as printed: `3–4`.
    var inputPair: String = ""
    var trimDb: Double = 0
    /// The live input's VU-ballistic RMS and held peak this poll, post-trim,
    /// linear — the needle and the HOT lamp. The one measured meter on the
    /// pane.
    var inputRms: Float = 0
    var inputPeak: Float = 0
    /// Engaged flags in `RackFx` order, as `DeckState.rackActive`.
    var active: [Bool] = [false, false, false, false]
    var controls: FxRackControls = FxRackControls()
    /// Whether the faces move — the tape runs, the sweep lamps breathe,
    /// RUNAWAY blinks, the coils shimmer. Off, every animated part sits
    /// at a fixed phase, which is what a snapshot needs to be a snapshot.
    var motion: Bool = true
    /// The keys on the four IN/OUT toggles (rack order) and on KICK, from
    /// the map; `nil` until bound.
    var toggleLegends: [String?] = [nil, nil, nil, nil]
    var kickLegend: String?

    func toggleLegend(_ unit: FxRackUnit) -> String? {
        unit.rawValue < toggleLegends.count ? toggleLegends[unit.rawValue] : nil
    }

    func isOn(_ unit: FxRackUnit) -> Bool {
        unit.slot < active.count && active[unit.slot]
    }

    /// The VU's needle, 0…1 along the siren meter's scale: 0 VU at
    /// −18 dBFS RMS, the scale −20…+3 VU, the red from +1.
    var vuLevel: Double {
        let amp = Double(max(inputRms, 1e-5))
        let vu = 20 * log10(amp) + 18
        return min(max((vu + 20) / 23, 0), 1)
    }

    /// HOT: the input peaked within 1 dB of full scale — the trim is too
    /// high for what the mixer is sending.
    var isHot: Bool { inputPeak >= 0.89 }

    /// The trim as the readout prints it.
    var trimText: String {
        String(format: "%@%.1f dB", trimDb >= 0 ? "+" : "−", abs(trimDb))
    }
}

/// What the pane fires. Separate from the state so the state stays
/// `Equatable`.
struct FxChannelCallbacks {
    var onSetInternal: () -> Void = {}
    var onPause: () -> Void = {}
    var onSetTimecode: () -> Void = {}
    var onSetThru: () -> Void = {}
    var onRecalibrate: () -> Void = {}
    var onInput: (_ kind: FxInputKind) -> Void = { _ in }
    var onTrim: (_ db: Double) -> Void = { _ in }
    /// The IN/OUT toggle on a unit's face.
    var onToggle: (_ unit: FxRackUnit) -> Void = { _ in }
    /// A knob moved: the whole set, with the one knob changed.
    var onControls: (_ controls: FxRackControls) -> Void = { _ in }
    /// KICK — Tubby's thunder.
    var onKick: () -> Void = {}
}
