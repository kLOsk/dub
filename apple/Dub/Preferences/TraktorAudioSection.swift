//
//  TraktorAudioSection.swift
//  Dub
//
//  Preferences ▸ Audio, only while a Traktor Audio 6 is plugged in: the
//  box's own Direct Thru and Phono switches per deck (`TraktorAudioBox`).
//  The switches show what the box says, never a value Dub remembers —
//  the box is the one that acts on it.
//

import SwiftUI

struct TraktorAudioSection: View {
    @StateObject private var panel = TraktorAudioPanel()

    var body: some View {
        if panel.connected {
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                Text("TRAKTOR AUDIO 6")
                    .font(DubFont.caps)
                    .tracking(DubFont.capsTracking)
                    .foregroundStyle(DubColor.textSecondary)
                table
                Text(
                    "These switches live in the box. Direct Thru wires a deck's input straight to its output and mutes Dub there, so leave it off to play from Dub or to use Dub's own THRU with effects. Phono is for a turntable plugged straight into the box; switch it off for a line-level source."
                )
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var table: some View {
        VStack(spacing: 0) {
            ForEach(Array(TraktorAudioSwitch.Channel.allCases.enumerated()), id: \.element) { row, channel in
                if row > 0 { Divider() }
                HStack(spacing: DubSpacing.lg) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Deck \(channel.letter)")
                            .font(DubFont.body)
                            .foregroundStyle(DubColor.textPrimary)
                        Text("Channels \(channel.pair)")
                            .font(DubFont.micro)
                            .foregroundStyle(DubColor.textTertiary)
                    }
                    Spacer(minLength: DubSpacing.md)
                    toggle("Direct Thru", TraktorAudioSwitch(kind: .directThru, channel: channel))
                    toggle("Phono", TraktorAudioSwitch(kind: .phono, channel: channel))
                }
                .padding(.horizontal, 14)
                .padding(.vertical, DubSpacing.sm)
            }
        }
        .background(DubColor.surface1)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(DubColor.divider, lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private func toggle(_ title: String, _ which: TraktorAudioSwitch) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Text(title)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
            Toggle(
                "",
                isOn: Binding(
                    get: { panel.states[which] ?? false },
                    set: { panel.set(which, on: $0) }
                )
            )
            .labelsHidden()
            .toggleStyle(.switch)
            .controlSize(.small)
            .disabled(panel.states[which] == nil)
            .accessibilityLabel("\(title), deck \(which.channel.letter)")
        }
    }
}

/// Talks to the box off the main thread and publishes what it answers.
@MainActor
final class TraktorAudioPanel: ObservableObject {
    @Published private(set) var connected = false
    /// A switch the box did not answer for is absent.
    @Published private(set) var states: [TraktorAudioSwitch: Bool] = [:]

    private let queue = DispatchQueue(label: "dub.traktor-audio-box")

    init() {
        refresh()
    }

    func set(_ which: TraktorAudioSwitch, on: Bool) {
        states[which] = on
        refresh(setting: (which, on))
    }

    /// Always ends by reading the box back, so a switch the box refused
    /// falls back to where the box really is.
    private func refresh(setting change: (TraktorAudioSwitch, Bool)? = nil) {
        queue.async { [weak self] in
            let box = TraktorAudioBox()
            if let (which, on) = change { box?.set(which, on: on) }
            var read: [TraktorAudioSwitch: Bool] = [:]
            if let box {
                for kind in TraktorAudioSwitch.Kind.allCases {
                    for channel in TraktorAudioSwitch.Channel.allCases {
                        let which = TraktorAudioSwitch(kind: kind, channel: channel)
                        read[which] = box.read(which)
                    }
                }
            }
            let found = box != nil
            Task { @MainActor [weak self] in
                self?.connected = found
                self?.states = read
            }
        }
    }
}
