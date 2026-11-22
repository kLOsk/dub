//
//  SourceControlView.swift
//  Dub
//
//  Per-deck source switch (PRD §5.1.1). A three-state Internal /
//  Timecode / Thru switch plus a status read-out, sized for the deck
//  header. The DJ clicks the mode they want — there is no auto-detection.
//
//  - INT  — play the loaded file on its own clock (Play button starts it).
//  - TC   — the control vinyl drives the loaded file.
//  - THRU — pass the live record on the platter straight through.
//  - DUB FX — the deck is the outboard rack on the mixer's send (F-38);
//    only drawn once the channel is enabled in Preferences.
//
//  Calibrating the needle is in the signal drawer (`DeckSignalPanel`),
//  beside the scope that shows what it did — not here: it is done once
//  per needle, and this switch is used all night (Daniel, 2026-11-21).
//  It works on INT as on TC: the stage rescue (P-33, 2026-11-15) is INT
//  to save the set, a new needle or control record, calibrate it while
//  INT plays, then TC by hand. While INT runs over a timecode input, a
//  lamp in the TC segment says whether the record under it is ready to
//  take the deck back.
//

import SwiftUI

/// Resolved source status for display. Mirrors the engine's active
/// `ControlMode` (plus a transient `calibrating` sub-state of Timecode);
/// there is no longer a `detecting` state because auto-detection was
/// removed.
enum SourceControlStatus: Equatable {
    case off          // engine stopped / no input
    case internalPlay // playing the file on its own clock
    case calibrating  // Timecode selected, capturing the whitening
    case timecode     // driven by the control vinyl
    case thru         // live record passthrough
    case fx           // the DUB FX channel: the rack on the mixer's send
}

/// While INT plays over a timecode input: can the record take the deck
/// back? Pure, so the rule is tested (`SourceControlReadinessTests`).
enum TimecodeReadiness: Equatable {
    /// Not on INT, or no carrier worth naming: no lamp.
    case none
    /// A calibration is measuring the needle.
    case measuring
    /// Calibrated and cleanly locked: TC picks the deck up where INT
    /// has it.
    case ready

    static func of(status: SourceControlStatus?, calibrating: Bool, calibrated: Bool, lockState: UInt8)
        -> Self
    {
        guard status == .internalPlay else { return .none }
        if calibrating { return .measuring }
        return calibrated && lockState == 1 ? .ready : .none
    }
}

struct SourceControlView: View {

    let status: SourceControlStatus
    /// The TC segment's lamp while INT runs (`TimecodeReadiness`).
    var readiness: TimecodeReadiness = .none
    /// Retained for call-site compatibility; the explicit switch no
    /// longer distinguishes "pinned" from "auto" (every mode is pinned).
    var overridden: Bool = false
    /// Whether the deck is currently advancing the playhead. Drives the
    /// INT segment's play/pause glyph.
    var isPlaying: Bool = false
    /// Deck the control belongs to — drives the active-segment tint.
    var side: DeckSide = .a
    /// Select Internal and start playing the loaded file.
    var onInternal: () -> Void = {}
    /// Pause internal playback (stays in Internal mode).
    var onPause: () -> Void = {}
    var onTimecode: () -> Void = {}
    var onThru: () -> Void = {}
    /// Whether the fourth position is drawn at all — the DUB FX channel is
    /// a Preferences opt-in, and a scratch set never wants the segment.
    var showFx: Bool = false
    var onFx: () -> Void = {}

    var body: some View {
        // No status dot and word beside the switch any more. It said
        // in text what the lit segment already says by position, and in
        // the two states the switch cannot show — no input, and
        // calibrating — the signal drawer says it better: that panel
        // reads live telemetry, where this read back the mode the user
        // had picked. Two places reporting the same thing from
        // different sources is one place too many, and the wrong one
        // was the one that could not see the signal.
        // Three-state switch. INT is the play/pause control: it shows ▶
        // to start internal playback and ⏸ while playing.
        HStack(spacing: 0) {
            intSegment
            segment("TC", active: isTimecodeActive, lamp: tcLamp, action: onTimecode)
            segment("THRU", active: isThruActive, action: onThru)
            if showFx {
                segment("DUB FX", active: status == .fx, action: onFx)
            }
        }
        .background(DubColor.surface2)
        .clipShape(Capsule())
        .overlay(Capsule().stroke(DubColor.divider, lineWidth: 1))
    }

    /// The INT segment: a play/pause toggle that also *is* the
    /// Internal-mode selector. ▶ when stopped (click → select Internal +
    /// play), ⏸ while playing internally (click → pause, stay Internal).
    private var intSegment: some View {
        let playingInternally = status == .internalPlay && isPlaying
        // The deck source switch fires on mouse-**down**, like the
        // transport / cue / tap controls — INT is the internal
        // Play/Pause, so pressing it starts playback at the press
        // instant rather than on release (see `View.onPressDown`).
        // Lit only while it is *doing* something. Internal is the
        // deck's resting state whenever there is no timecode input, so
        // tinting on the mode alone left the segment permanently
        // orange and the switch reading as though a stopped deck were
        // engaged. Grey ▶ at rest, tinted ⏸ while running: the tint
        // means "this is what the deck is doing", the same as TC and
        // THRU, rather than "this is the mode it would use".
        return Image(systemName: playingInternally ? "pause.fill" : "play.fill")
            .font(.system(size: 9, weight: .bold))
            .foregroundStyle(playingInternally ? DubColor.surface0 : DubColor.textSecondary)
            .frame(minWidth: 22)
            .padding(.vertical, 4)
            .background(playingInternally ? DubColor.controlAccent : Color.clear)
            .onPressDown { playingInternally ? onPause() : onInternal() }
            .accessibilityAddTraits(.isButton)
            .help(playingInternally ? "Pause" : "Play internally")
    }

    private func segment(
        _ title: String, active: Bool, lamp: Color? = nil, action: @escaping () -> Void
    ) -> some View {
        Text(title)
            .font(DubFont.caps)
            .tracking(0.6)
            .foregroundStyle(active ? DubColor.surface0 : DubColor.textSecondary)
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 3)
            // In the segment's own leading padding: the lamp coming and
            // going must not move THRU or re-lay-out the column.
            .overlay(alignment: .leading) {
                if let lamp {
                    Circle().fill(lamp).frame(width: 5, height: 5).padding(.leading, 2)
                }
            }
            .background(active ? DubColor.controlAccent : Color.clear)
        .onPressDown(perform: action)
        .accessibilityAddTraits(.isButton)
    }

    private var isTimecodeActive: Bool { status == .timecode || status == .calibrating }
    private var isThruActive: Bool { status == .thru }

    /// Green: the record is calibrated and locked, TC takes the deck back
    /// where INT has it. Amber: the needle is being measured.
    private var tcLamp: Color? {
        switch readiness {
        case .none: return nil
        case .measuring: return DubColor.stateTentative
        case .ready: return DubColor.stateLocked
        }
    }

}

#Preview("Source control — states") {
    VStack(alignment: .leading, spacing: 12) {
        SourceControlView(status: .internalPlay)
        SourceControlView(status: .internalPlay, readiness: .measuring, isPlaying: true)
        SourceControlView(status: .internalPlay, readiness: .ready, isPlaying: true)
        SourceControlView(status: .calibrating)
        SourceControlView(status: .timecode)
        SourceControlView(status: .thru)
    }
    .padding()
    .background(DubColor.surface0)
}

// MARK: - Key Lock (master tempo) control — M14

/// Whether a deck's key lock is on. `resampler` = key lock OFF (the deck's
/// resampler handles rate, so pitch shifts with it); `ours` = our pure-Rust
/// WSOLA key lock (tempo follows the platter, pitch held).
enum KeyLockSelection: Equatable {
    case resampler
    case ours
}

/// Per-deck Key Lock control (PRD §6.1.1): a two-way RESAMP / OURS toggle plus a
/// status dot showing the engine's *actual* state (full green = engaged / pitch
/// held, dim green = standby / auto-bypassed during a scratch, grey = off). A
/// Rudimentary prep-mode pitch control for **testing** key lock without a
/// turntable (M14): tap a percent to set the deck's playback rate, then A/B the
/// key-lock engines above to hear pitch held (Ours / Rubber Band) vs shifted
/// (Resampler). `0` returns to unity. Not a performance control.
struct PitchTestView: View {
    let model: WaveformAppModel
    let side: DeckSide
    /// The deck's own store, observed: deck state is not published
    /// through the model (`ModelStores.swift`).
    @ObservedObject private var deck: DeckStore

    init(model: WaveformAppModel, side: DeckSide) {
        self.model = model
        self.side = side
        self._deck = ObservedObject(wrappedValue: model.deckStore(side))
    }

    private let steps: [Double] = [-10, -5, -2, 0, 2, 5, 10]

    /// On the model, not `@State` — see `DeckState.prepPitchPercent`.
    private var current: Double { deck.state.prepPitchPercent }

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            DubSectionLabel("PITCH %")
                .fixedSize()

            DubSegmentedControl(
                segments: steps.map { .init($0, label($0), width: .minWidth(26)) },
                selection: current,
                tint: DubColor.controlAccent,
                onSelect: { pct in model.setPrepPitch(side: side, percent: pct) })
        }
    }

    private func label(_ pct: Double) -> String {
        if pct == 0 { return "0" }
        return pct > 0 ? "+\(Int(pct))" : "\(Int(pct))"
    }

}
