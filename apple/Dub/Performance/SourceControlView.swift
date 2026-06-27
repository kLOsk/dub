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
//
//  The small ↻ (shown on TC) recalibrates the needle.
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
}

struct SourceControlView: View {

    let status: SourceControlStatus
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
    var onRecalibrate: () -> Void = {}

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(dotColor)
                    .frame(width: 7, height: 7)
                Text(statusLabel)
                    .font(DubFont.caps)
                    .tracking(0.6)
                    .foregroundStyle(DubColor.textSecondary)
                    .fixedSize()
            }

            // Three-state switch. INT is the play/pause control: it
            // shows ▶ to start internal playback and ⏸ while playing.
            HStack(spacing: 0) {
                intSegment
                segment("TC", active: isTimecodeActive, action: onTimecode)
                segment("THRU", active: isThruActive, action: onThru)
            }
            .background(DubColor.surface2)
            .clipShape(Capsule())
            .overlay(Capsule().stroke(DubColor.divider, lineWidth: 1))

            if status == .timecode || status == .calibrating {
                Button(action: onRecalibrate) {
                    Image(systemName: "arrow.triangle.2.circlepath")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(DubColor.textSecondary)
                }
                .buttonStyle(.plain)
                .help("Recalibrate this needle")
            }
        }
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
        return Image(systemName: playingInternally ? "pause.fill" : "play.fill")
            .font(.system(size: 9, weight: .bold))
            .foregroundStyle(isInternalActive ? DubColor.surface0 : DubColor.textSecondary)
            .frame(minWidth: 22)
            .padding(.vertical, 4)
            .background(isInternalActive ? DubColor.deckTint(side) : Color.clear)
            .onPressDown { playingInternally ? onPause() : onInternal() }
            .accessibilityAddTraits(.isButton)
            .help(playingInternally ? "Pause" : "Play internally")
    }

    private func segment(_ title: String, active: Bool, action: @escaping () -> Void) -> some View {
        Text(title)
            .font(DubFont.caps)
            .tracking(0.6)
            .foregroundStyle(active ? DubColor.surface0 : DubColor.textSecondary)
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 3)
            .background(active ? DubColor.deckTint(side) : Color.clear)
            .onPressDown(perform: action)
            .accessibilityAddTraits(.isButton)
    }

    private var isInternalActive: Bool { status == .internalPlay }
    private var isTimecodeActive: Bool { status == .timecode || status == .calibrating }
    private var isThruActive: Bool { status == .thru }

    private var statusLabel: String {
        switch status {
        case .off: return "OFF"
        case .internalPlay: return "INTERNAL"
        case .calibrating: return "CALIBRATING…"
        case .timecode: return "TIMECODE"
        case .thru: return "THRU"
        }
    }

    private var dotColor: Color {
        switch status {
        case .off: return DubColor.textPlaceholder
        case .internalPlay: return DubColor.textSecondary
        case .calibrating: return DubColor.stateTentative
        case .timecode: return DubColor.stateLocked
        case .thru: return DubColor.stateLocked
        }
    }
}

#Preview("Source control — states") {
    VStack(alignment: .leading, spacing: 12) {
        SourceControlView(status: .internalPlay)
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
/// Prep / dev surface; clickable (the no-mouse rule is Performance-only).
struct KeyLockControlView: View {
    @ObservedObject var model: WaveformAppModel
    let side: DeckSide

    /// The engine's published key-lock state (0 off · 1 standby · 2 engaged),
    /// polled at the signal panel's 20 Hz cadence.
    @State private var indicatorState: UInt8 = 0
    private let tick = Timer.publish(every: 1.0 / 20.0, on: .main, in: .common).autoconnect()

    var body: some View {
        let selection = model.keyLockSelection(side)
        HStack(spacing: DubSpacing.sm) {
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(dotColor)
                    .frame(width: 7, height: 7)
                Text("KEY LOCK")
                    .font(DubFont.caps)
                    .tracking(0.6)
                    .foregroundStyle(DubColor.textSecondary)
                    .fixedSize()
            }

            HStack(spacing: 0) {
                segment("OFF", active: selection == .resampler) {
                    model.setKeyLockSelection(side: side, .resampler)
                }
                segment("ON", active: selection == .ours) {
                    model.setKeyLockSelection(side: side, .ours)
                }
            }
            .background(DubColor.surface2)
            .clipShape(Capsule())
            .overlay(Capsule().stroke(DubColor.divider, lineWidth: 1))
        }
        .onReceive(tick) { _ in
            indicatorState = model.engine.deckTelemetry(deckIdx: side.ffiDeckIdx).keyLockState
        }
    }

    private func segment(_ title: String, active: Bool, action: @escaping () -> Void) -> some View {
        Text(title)
            .font(DubFont.caps)
            .tracking(0.6)
            .foregroundStyle(active ? DubColor.surface0 : DubColor.textSecondary)
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 3)
            .background(active ? DubColor.deckTint(side) : Color.clear)
            .onPressDown(perform: action)
            .accessibilityAddTraits(.isButton)
    }

    private var dotColor: Color {
        switch indicatorState {
        case 2: return DubColor.stateLocked // engaged — pitch held
        case 1: return DubColor.stateLocked.opacity(0.4) // standby — auto-bypassed
        default: return DubColor.textPlaceholder // off
        }
    }
}

/// Rudimentary prep-mode pitch control for **testing** key lock without a
/// turntable (M14): tap a percent to set the deck's playback rate, then A/B the
/// key-lock engines above to hear pitch held (Ours / Rubber Band) vs shifted
/// (Resampler). `0` returns to unity. Not a performance control.
struct PitchTestView: View {
    @ObservedObject var model: WaveformAppModel
    let side: DeckSide

    @State private var current: Double = 0

    private let steps: [Double] = [-10, -5, -2, 0, 2, 5, 10]

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("PITCH %")
                .font(DubFont.caps)
                .tracking(0.6)
                .foregroundStyle(DubColor.textSecondary)
                .fixedSize()

            HStack(spacing: 0) {
                ForEach(steps, id: \.self) { pct in
                    segment(label(pct), active: current == pct) {
                        current = pct
                        model.setPrepPitch(side: side, percent: pct)
                    }
                }
            }
            .background(DubColor.surface2)
            .clipShape(Capsule())
            .overlay(Capsule().stroke(DubColor.divider, lineWidth: 1))
        }
    }

    private func label(_ pct: Double) -> String {
        if pct == 0 { return "0" }
        return pct > 0 ? "+\(Int(pct))" : "\(Int(pct))"
    }

    private func segment(_ title: String, active: Bool, action: @escaping () -> Void) -> some View {
        Text(title)
            .font(DubFont.caps)
            .tracking(0.6)
            .foregroundStyle(active ? DubColor.surface0 : DubColor.textSecondary)
            .frame(minWidth: 26)
            .padding(.vertical, 3)
            .background(active ? DubColor.deckTint(side) : Color.clear)
            .onPressDown(perform: action)
            .accessibilityAddTraits(.isButton)
    }
}
