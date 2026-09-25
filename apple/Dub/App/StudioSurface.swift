//
//  StudioSurface.swift
//  Dub
//
//  PREP · REC · PERF — the status strip's surface switch.
//
//  Recording a side used to be entered from a RIP VINYL pill in Prep's
//  lane and left only by finishing or discarding it; on the rig there
//  was no telling which mode the app was in, nor a way back to Prep
//  that did not throw the side away (2026-09-25). The switch names the
//  three surfaces where MAP sat — MAP has no work to do while recording
//  — and each move says what it does to a recording in flight.
//

enum StudioSurface: Equatable, Hashable {
    case prep
    case record
    case perf
}

enum StudioSurfaceRules {
    static func current(ripPhase: RipUiPhase, engineMode: EngineMode) -> StudioSurface {
        if ripPhase != .none { return .record }
        return engineMode == .prep ? .prep : .perf
    }

    /// The surfaces that cannot be chosen now, and why — the switch
    /// greys them and says so on hover.
    ///
    /// - `armed`: the recording is waiting for the needle; nothing has
    ///   been captured, so leaving cancels it.
    /// - `canRecord`: vinyl recording is on and there is an input.
    static func disabled(
        ripPhase: RipUiPhase, armed: Bool, canRecord: Bool
    ) -> [StudioSurface: String] {
        var out: [StudioSurface: String] = [:]
        switch ripPhase {
        case .capture where !armed:
            out[.prep] = "Recording — press STOP first"
        case .encoding:
            out[.prep] = "Importing — wait for it to finish"
        default:
            break
        }
        if ripPhase != .none {
            out[.perf] = "Finish or leave the recording first"
        } else if !canRecord {
            out[.record] = "Connect a DJ interface to record vinyl"
        }
        return out
    }

    /// What leaving the recording for Prep does to it, by phase.
    enum Leave: Equatable {
        /// Armed, nothing captured: cancel it.
        case cancel
        /// A take to review: keep it on disk; the recovery banner offers
        /// it back.
        case park
        /// Imported, or failed with nothing kept: close it.
        case dismiss
        /// Recording or importing: not allowed (the switch is disabled).
        case refuse
    }

    static func leave(ripPhase: RipUiPhase, armed: Bool, hasSegments: Bool) -> Leave {
        switch ripPhase {
        case .none: return .dismiss
        case .capture: return armed ? .cancel : .refuse
        case .review: return .park
        case .failed: return hasSegments ? .park : .dismiss
        case .encoding: return .refuse
        case .done: return .dismiss
        }
    }
}
