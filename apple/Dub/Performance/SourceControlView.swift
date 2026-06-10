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
        return Button(action: { playingInternally ? onPause() : onInternal() }) {
            Image(systemName: playingInternally ? "pause.fill" : "play.fill")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(isInternalActive ? DubColor.surface0 : DubColor.textSecondary)
                .frame(minWidth: 22)
                .padding(.vertical, 4)
                .background(isInternalActive ? DubColor.deckTint(side) : Color.clear)
        }
        .buttonStyle(.plain)
        .help(playingInternally ? "Pause" : "Play internally")
    }

    private func segment(_ title: String, active: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(DubFont.caps)
                .tracking(0.6)
                .foregroundStyle(active ? DubColor.surface0 : DubColor.textSecondary)
                .padding(.horizontal, DubSpacing.sm)
                .padding(.vertical, 3)
                .background(active ? DubColor.deckTint(side) : Color.clear)
        }
        .buttonStyle(.plain)
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
