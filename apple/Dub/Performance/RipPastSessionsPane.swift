//
//  RipPastSessionsPane.swift
//  Dub
//
//  M26b — the "Real Records" browser pane: every committed rip
//  session, newest first, each re-splittable from its lossless
//  `side.flac` archive (UI-BACKLOG R-44).
//
//  Re-splitting reopens the side into the *existing* review panel in
//  the Prep column above, which is on screen at the same time as this
//  pane — `PerformanceView` stacks `waveformRegion` and `LibraryView`
//  in one VStack. So there is no navigation here, no sheet and no
//  second review surface: picking a row swaps what the column above
//  is showing, the same in-place substitution the rest of the rip
//  feature uses.
//
//  Value-driven like every other rip view (state struct + callbacks +
//  a pure `View`), so the snapshot suite renders it without a session.
//

import SwiftUI

/// One committed rip as the pane renders it. Mirrors the FFI
/// `RipResplittable` as a plain value.
struct RipPastSessionUi: Equatable, Identifiable {
    /// The session directory, which is also its identity.
    let sessionDir: String
    /// Directory name — the capture timestamp, `YYYYMMDD-HHMMSS`.
    let name: String
    var recordedSecs: Double
    var trackCount: UInt32
    /// How many times this side has been split; 1 is the first commit.
    var splitGeneration: UInt32

    var id: String { sessionDir }

    /// `20260828-221148` → `28 Aug 2026, 22:11`. Falls back to the raw
    /// name rather than inventing a date, so an unexpected directory
    /// name is visible instead of silently mislabelled.
    var dateText: String {
        let parts = name.split(separator: "-")
        guard parts.count == 2, parts[0].count == 8, parts[1].count >= 4,
              let year = Int(parts[0].prefix(4)),
              let month = Int(parts[0].dropFirst(4).prefix(2)),
              let day = Int(parts[0].dropFirst(6).prefix(2)),
              let hour = Int(parts[1].prefix(2)),
              let minute = Int(parts[1].dropFirst(2).prefix(2))
        else { return name }
        var components = DateComponents()
        components.year = year
        components.month = month
        components.day = day
        components.hour = hour
        components.minute = minute
        guard let date = Calendar.current.date(from: components) else { return name }
        let formatter = DateFormatter()
        formatter.dateFormat = "d MMM yyyy, HH:mm"
        return formatter.string(from: date)
    }

    var durationText: String { RipDuration.text(recordedSecs) }

    var trackCountText: String {
        trackCount == 1 ? "1 track" : "\(trackCount) tracks"
    }
}

/// Everything the pane draws.
struct RipPastSessionsPaneState: Equatable {
    var sessions: [RipPastSessionUi] = []
    var isLoading: Bool = false
    /// Why re-split is unavailable right now (wrong mode, a capture in
    /// flight). `nil` means the rows are live.
    var blockedReason: String? = nil
}

struct RipPastSessionsPaneCallbacks {
    var resplit: (RipPastSessionUi) -> Void = { _ in }
    var revealInFinder: (RipPastSessionUi) -> Void = { _ in }
}

struct RipPastSessionsPane: View {

    let state: RipPastSessionsPaneState
    var callbacks = RipPastSessionsPaneCallbacks()

    /// Two-step arm, so a click in the browser cannot silently clobber
    /// whatever the DJ has loaded on deck A in the column above. Same
    /// pattern as the review panel's Discard.
    @State private var armed: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let reason = state.blockedReason {
                Text(reason)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
                    .padding(.horizontal, DubSpacing.lg)
                    .padding(.vertical, DubSpacing.sm)
                Divider().overlay(DubColor.divider)
            }
            if state.sessions.isEmpty {
                emptyState
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(state.sessions) { session in
                            row(session)
                            Divider().overlay(DubColor.divider.opacity(0.5))
                        }
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(DubColor.surface0)
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            Text(state.isLoading ? "Looking for past rips…" : "No records ripped yet.")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
            if !state.isLoading {
                Text("Rip a side from the Prep bar and it will show up here, "
                     + "ready to split again from its lossless archive.")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
            }
        }
        .padding(DubSpacing.lg)
    }

    private func row(_ session: RipPastSessionUi) -> some View {
        let live = state.blockedReason == nil
        let isArmed = armed == session.id
        return HStack(spacing: DubSpacing.md) {
            Image(systemName: "opticaldisc")
                .frame(width: 16)
                .foregroundStyle(DubColor.textSecondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(session.dateText)
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
                HStack(spacing: DubSpacing.sm) {
                    Text(session.durationText)
                    Text("·")
                    Text(session.trackCountText)
                    if session.splitGeneration > 1 {
                        Text("·")
                        Text("split \(session.splitGeneration)×")
                    }
                }
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
            }
            Spacer(minLength: DubSpacing.md)
            Button(isArmed ? "Really re-split?" : "Re-split") {
                if isArmed {
                    armed = nil
                    callbacks.resplit(session)
                } else {
                    armed = session.id
                    Task { @MainActor in
                        try? await Task.sleep(nanoseconds: 3_000_000_000)
                        if armed == session.id { armed = nil }
                    }
                }
            }
            .buttonStyle(.plain)
            .font(DubFont.caps)
            .padding(.horizontal, DubSpacing.md)
            .padding(.vertical, DubSpacing.xs)
            .background(isArmed ? DubColor.stateError.opacity(0.25) : DubColor.surface2)
            .foregroundStyle(live ? DubColor.textPrimary : DubColor.textPlaceholder)
            .disabled(!live)
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.sm)
        .contentShape(Rectangle())
        .contextMenu {
            Button("Reveal in Finder") { callbacks.revealInFinder(session) }
        }
    }
}

/// `m:ss` for the rip surfaces. Was copied into four value types
/// before it was worth naming.
enum RipDuration {
    /// A fixed length — a side, a segment. Rounds to nearest.
    static func text(_ secs: Double) -> String {
        clock(Int(secs.rounded()))
    }

    /// A *running* clock. Truncates, because a counter that reads 1:00
    /// while the 59th second is still going is wrong in a way a
    /// duration label is not.
    static func running(_ secs: Double) -> String {
        clock(Int(secs.rounded(.down)))
    }

    private static func clock(_ total: Int) -> String {
        String(format: "%d:%02d", max(0, total) / 60, max(0, total) % 60)
    }
}
