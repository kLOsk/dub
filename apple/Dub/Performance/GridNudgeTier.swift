//
//  GridNudgeTier.swift
//  Dub
//
//  Modifier-driven step sizes for manual beatgrid nudge buttons.
//  Regular click = default step; Shift = coarse; Shift+Option = fine.
//

import AppKit

/// Which step size a grid-nudge action uses, derived from held
/// modifier keys at click / repeat time.
enum BeatgridNudgeTier: String, Sendable {
    case regular
    case coarse
    case fine

    init(modifiers: NSEvent.ModifierFlags) {
        let flags = modifiers.intersection(.deviceIndependentFlagsMask)
        if flags.contains(.shift), flags.contains(.option) {
            self = .fine
        } else if flags.contains(.shift) {
            self = .coarse
        } else {
            self = .regular
        }
    }

    /// Phase nudge in seconds. Positive moves the grid later.
    var phaseStepSecs: Double {
        switch self {
        case .regular: return 0.005
        case .coarse:  return 0.025
        case .fine:    return 0.001
        }
    }

    /// BPM nudge magnitude.
    var bpmStep: Double {
        switch self {
        case .regular: return 0.1
        case .coarse:  return 0.5
        case .fine:    return 0.01
        }
    }

    /// M11d.6 rotate which beat is the visual downbeat. Unused since
    /// `setDownbeatAtPlayhead` re-anchors instead; kept for tier API
    /// symmetry if we add explicit downbeat_phase later.
    var downbeatBeats: Int {
        switch self {
        case .regular: return 1
        case .coarse:  return 4
        case .fine:    return -1
        }
    }
}
