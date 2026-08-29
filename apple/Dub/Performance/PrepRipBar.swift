//
//  PrepRipBar.swift
//  Dub
//
//  M26a — the vinyl-rip control row at the top of the Prep pad bar.
//  Idle it's a single ● RIP VINYL button; while recording it shows
//  the elapsed clock + a live level meter (clip flash ≥ 0.99) + STOP;
//  after that a compact status line for done / failed. The review +
//  encoding phases replace the whole pad bar with `RipReviewPanel`,
//  so this bar only ever renders idle / recording / done / failed
//  in the app (the review/encoding cases exist for completeness).
//
//  Pure function of `PrepRipBarState` (no FFI, no model) so the
//  snapshot suite can render every state.
//

import SwiftUI

/// Value state for `PrepRipBar`.
struct PrepRipBarState: Equatable {
    enum Phase: Equatable {
        case idle
        /// Armed and waiting for the needle (M26b auto-start). The
        /// meter is live so you can see the stylus land even before
        /// the trigger fires.
        case armed
        case recording
        case review
        case encoding
        case done
        case failed
    }

    var phase: Phase
    /// Recorded duration (recording / review states).
    var elapsedSecs: Double = 0
    /// Absolute peak of the most recent capture window, `[0, 1]`.
    var levelPeak: Float = 0
    /// Secondary status text (done summary, stop-reason note).
    var statusText: String? = nil
    /// Failure message (failed state).
    var errorMessage: String? = nil

    /// Clip indicator — flashes the meter red.
    var isClipping: Bool { levelPeak >= 0.99 }

    /// `mm:ss` elapsed clock.
    var elapsedText: String { RipDuration.running(elapsedSecs) }
}

/// Tap targets, split from the state so the view stays a pure
/// value function.
struct PrepRipBarCallbacks {
    var onRecord: () -> Void = {}
    var onStop: () -> Void = {}
    var onDismiss: () -> Void = {}
    var onDiscard: () -> Void = {}
}

struct PrepRipBar: View {

    let state: PrepRipBarState
    var callbacks = PrepRipBarCallbacks()

    var body: some View {
        HStack(spacing: DubSpacing.md) {
            switch state.phase {
            case .idle:      idleRow
            case .armed:     armedRow
            case .recording: recordingRow
            case .review:    statusRow(dot: DubColor.stateLocked,
                                       label: "SIDE RECORDED — \(state.elapsedText)")
            case .encoding:  statusRow(dot: DubColor.stateTentative,
                                       label: "ENCODING…")
            case .done:      doneRow
            case .failed:    failedRow
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, DubSpacing.md)
        .padding(.vertical, DubSpacing.sm)
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel))
    }

    // MARK: Rows

    private var idleRow: some View {
        Button(action: callbacks.onRecord) {
            HStack(spacing: DubSpacing.sm) {
                Circle()
                    .fill(DubColor.stateError)
                    .frame(width: 8, height: 8)
                Text("RIP VINYL")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(DubColor.textPrimary)
            }
            .padding(.horizontal, DubSpacing.md)
            .padding(.vertical, DubSpacing.xs)
            .background(DubColor.surface2)
            .clipShape(Capsule())
        }
        .buttonStyle(.plain)
        .help("Record the record playing on deck A's turntable")
    }

    /// Armed: the rip is live but the stylus hasn't landed. Same
    /// shape as the recording row (meter + STOP) so nothing jumps
    /// when the trigger fires — only the label and colour change.
    private var armedRow: some View {
        HStack(spacing: DubSpacing.md) {
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(DubColor.stateTentative)
                    .frame(width: 8, height: 8)
                Text("ARMED")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(DubColor.stateTentative)
            }
            Text("drop the needle")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
            RipLevelMeter(level: state.levelPeak, clipping: false)
                .frame(width: 140, height: 8)
            Spacer(minLength: 0)
            Button(action: callbacks.onStop) {
                Text("CANCEL")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(DubColor.textPrimary)
                    .padding(.horizontal, DubSpacing.md)
                    .padding(.vertical, DubSpacing.xs)
                    .background(DubColor.surface2)
                    .clipShape(Capsule())
            }
            .buttonStyle(.plain)
            .help("Cancel — nothing has been recorded yet")
        }
    }

    private var recordingRow: some View {
        HStack(spacing: DubSpacing.md) {
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(DubColor.stateError)
                    .frame(width: 8, height: 8)
                Text("REC")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(DubColor.stateError)
            }
            Text(state.elapsedText)
                .font(DubFont.numericLarge)
                .monospacedDigit()
                .foregroundStyle(DubColor.textPrimary)
            RipLevelMeter(level: state.levelPeak, clipping: state.isClipping)
                .frame(width: 140, height: 8)
            if state.isClipping {
                Text("CLIP")
                    .font(DubFont.caps)
                    .tracking(1.0)
                    .foregroundStyle(DubColor.stateError)
            }
            Spacer(minLength: 0)
            Button(action: callbacks.onStop) {
                HStack(spacing: DubSpacing.xs) {
                    Image(systemName: "stop.fill")
                        .font(.system(size: 9, weight: .bold))
                    Text("STOP")
                        .font(DubFont.caps)
                        .tracking(1.2)
                }
                .foregroundStyle(DubColor.textPrimary)
                .padding(.horizontal, DubSpacing.md)
                .padding(.vertical, DubSpacing.xs)
                .background(DubColor.stateError.opacity(0.35))
                .clipShape(Capsule())
            }
            .buttonStyle(.plain)
            .help("Stop recording and review the side")
        }
    }

    private var doneRow: some View {
        HStack(spacing: DubSpacing.md) {
            statusRow(dot: DubColor.stateLocked,
                      label: state.statusText ?? "IMPORTED")
            Spacer(minLength: 0)
            dismissButton(title: "Done", action: callbacks.onDismiss)
        }
    }

    private var failedRow: some View {
        HStack(spacing: DubSpacing.md) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 10))
                .foregroundStyle(DubColor.stateError)
            Text(state.errorMessage ?? "Recording failed.")
                .font(DubFont.body)
                .foregroundStyle(DubColor.stateError)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            dismissButton(title: "Dismiss", action: callbacks.onDiscard)
        }
    }

    private func statusRow(dot: Color, label: String) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Circle()
                .fill(dot)
                .frame(width: 8, height: 8)
            Text(label)
                .font(DubFont.caps)
                .tracking(1.0)
                .foregroundStyle(DubColor.textPrimary)
            if let status = state.statusText, state.phase != .done {
                Text(status)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
            }
        }
    }

    private func dismissButton(title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .padding(.horizontal, DubSpacing.md)
                .padding(.vertical, 2)
                .background(DubColor.surface2)
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
    }
}

/// Horizontal peak meter for the recording row. Value-driven; the
/// clip state paints the whole bar red so a hot cartridge is
/// unmissable at arm's length.
struct RipLevelMeter: View {
    /// `[0, 1]` peak of the last capture window.
    let level: Float
    let clipping: Bool

    var body: some View {
        GeometryReader { geo in
            let fraction = CGFloat(max(0, min(1, level)))
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(DubColor.surface2)
                Capsule()
                    .fill(fillColor)
                    .frame(width: max(2, geo.size.width * fraction))
            }
        }
    }

    private var fillColor: Color {
        if clipping { return DubColor.stateError }
        if level >= 0.85 { return DubColor.stateTentative }
        return DubColor.stateLocked
    }
}

#Preview("idle") {
    PrepRipBar(state: PrepRipBarState(phase: .idle))
        .padding()
        .background(DubColor.surface0)
        .frame(width: 720)
}

#Preview("recording") {
    PrepRipBar(state: PrepRipBarState(
        phase: .recording, elapsedSecs: 754, levelPeak: 0.72))
        .padding()
        .background(DubColor.surface0)
        .frame(width: 720)
}

#Preview("failed") {
    PrepRipBar(state: PrepRipBarState(
        phase: .failed,
        errorMessage: "Input lost — the interface was disconnected."))
        .padding()
        .background(DubColor.surface0)
        .frame(width: 720)
}
