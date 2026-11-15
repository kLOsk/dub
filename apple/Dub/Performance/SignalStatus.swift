//
//  SignalStatus.swift
//  Dub
//
//  What the signal panel says about a deck's control record, in the
//  DJ's words: one state, one line of advice, levels in dB. Pure, so the
//  rules are tested rather than eyeballed (`SignalStatusTests`).
//
//  The panel used to print the decoder's own numbers — CONFIDENCE 0.00,
//  AMPLITUDE 0.000 — which answer a question nobody at a gig asks. The
//  one they ask is "is my needle and cable all right, and is it locked?"
//  (redesign, 2026-11-15; canvas "Dub Signal Panel").
//

import SwiftUI

struct SignalStatus: Equatable {
    enum Kind: Equatable { case locked, weak, noLock, noSignal, noInput }

    let kind: Kind
    /// The big word.
    let word: String
    /// One line of what is going on, or what to check.
    let advice: String

    /// Below this RMS a channel carries nothing worth calling a signal
    /// (about −50 dBFS).
    static let silence: Float = 0.003
    /// One channel this far under the other is a fault, not a balance:
    /// a cartridge wire, a headshell pin, a cable.
    static let imbalanceDb: Float = 12

    /// - Parameters:
    ///   - hasInput: an input is routed to the deck.
    ///   - lockState: the decoder's lock — 0 none, 1 locked, 2 degraded,
    ///     3 no lock / scratching.
    ///   - weakChannel: the last calibration refused a weak side — 0
    ///     none, 1 left, 2 right.
    ///   - rms: the input's RMS, left and right, linear.
    ///   - pitchPercent: the deck's pitch, for the direction.
    static func of(
        hasInput: Bool, lockState: UInt8, weakChannel: UInt8,
        rms: (left: Float, right: Float), pitchPercent: Double
    ) -> SignalStatus {
        guard hasInput else {
            return SignalStatus(
                kind: .noInput, word: "NO INPUT",
                advice: "No input for this deck. Pick one in Preferences ▸ Audio.")
        }
        if let side = weakSide(weakChannel: weakChannel, rms: rms) {
            return SignalStatus(
                kind: .weak, word: "WEAK",
                advice: "\(side) channel is low. Check the cable and the needle.")
        }
        switch lockState {
        case 1:
            let rate = 1 + pitchPercent / 100
            let direction = abs(rate) < 0.02 ? "stopped" : rate < 0 ? "reverse" : "forward"
            return SignalStatus(kind: .locked, word: "LOCKED", advice: "Timecode · \(direction)")
        case 2:
            return SignalStatus(
                kind: .weak, word: "WEAK",
                advice: "The signal is noisy. Check the needle and the cable.")
        case 3:
            return SignalStatus(
                kind: .noLock, word: "NO LOCK", advice: "Scratching, or the record is stopped.")
        default:
            return SignalStatus(
                kind: .noSignal, word: "NO SIGNAL",
                advice: "Needle down, record turning? Check the PHONO/LINE switch.")
        }
    }

    /// "Left" or "Right" when one side is failing — from the last
    /// calibration's refusal, or live: both sides heard, one far under.
    static func weakSide(weakChannel: UInt8, rms: (left: Float, right: Float)) -> String? {
        switch weakChannel {
        case 1: return "Left"
        case 2: return "Right"
        default: break
        }
        let loud = max(rms.left, rms.right)
        guard loud >= silence * 4 else { return nil }
        let quiet = min(rms.left, rms.right)
        guard decibels(quiet) < decibels(loud) - imbalanceDb else { return nil }
        return rms.left < rms.right ? "Left" : "Right"
    }

    /// dBFS of a linear level; −120 for silence.
    static func decibels(_ linear: Float) -> Float {
        linear > 1e-6 ? 20 * log10(linear) : -120
    }

    /// A level meter's figure: `−9 dB`, or `—` for nothing.
    static func levelFigure(_ linear: Float) -> String {
        linear < 1e-5 ? "—" : "\(Int(decibels(linear).rounded())) dB"
            .replacingOccurrences(of: "-", with: "−")
    }

    /// How full a level meter is: −60 dBFS empty, 0 full.
    static func levelFraction(_ linear: Float) -> Double {
        Double(min(1, max(0, (decibels(linear) + 60) / 60)))
    }

    var colour: Color {
        switch kind {
        case .locked: return DubColor.stateLocked
        case .weak: return DubColor.stateTentative
        case .noLock: return DubColor.textSecondary
        case .noSignal, .noInput: return DubColor.textTertiary
        }
    }

    /// The calibration line: what the deck is doing about it now. It
    /// measures by itself the first time it hears a turning record each
    /// session, so a date would say nothing.
    static func calibration(
        hasInput: Bool, pitchSettled: Bool, calibrating: Bool, calibrated: Bool
    ) -> (text: String, colour: Color) {
        guard hasInput else { return ("", DubColor.textTertiary) }
        if calibrating { return ("Calibrating… play the record", DubColor.stateTentative) }
        if !pitchSettled { return ("Measuring… play the record", DubColor.stateTentative) }
        if calibrated { return ("Calibrated", DubColor.stateLocked) }
        return ("Measures by itself at needle-down", DubColor.textTertiary)
    }
}
