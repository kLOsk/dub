//
//  OnboardingSheet.swift
//  Dub
//
//  U-23 — first-run experience. Before this, opening Dub for the
//  first time dropped the user into an empty library with no
//  explanation of the hardware model or how to add music. For a
//  tool with a strict rig prerequisite (a ≥4-in/4-out interface for
//  Performance mode) that was hostile.
//
//  The flow is three short pages: what Dub is, how it picks audio
//  (automatically — there is no input/channel picker by design, see
//  PreferencesSheet), and how to add music. It is skippable at any
//  point and re-openable from Preferences (it posts
//  `.dubShowOnboarding`, which MainView observes). Completion /skip
//  persists `dub.hasCompletedOnboarding` so it never reappears
//  unbidden.
//

import SwiftUI
import DubCore

struct OnboardingSheet: View {

    @ObservedObject var model: WaveformAppModel

    /// Called when the user finishes the last page or taps Skip. The
    /// host (MainView) persists the completion flag and dismisses.
    var onFinish: () -> Void

    @State private var step: Int = 0
    private static let lastStep = 2

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            header
            Divider()
            page
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            Divider()
            footer
        }
        .padding(DubSpacing.xl)
        .frame(width: 540, height: 560)
        .background(DubColor.surface0)
    }

    @ViewBuilder
    private var page: some View {
        switch step {
        case 0:  welcomePage
        case 1:  audioPage
        default: libraryPage
        }
    }

    // MARK: - Pages

    private var welcomePage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            Text("A timecode-vinyl instrument")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(DubColor.textPrimary)
            Text("Dub is built for scratch and sound-system DJs. Your hands stay on the turntable and your mixer — Dub drives the records and stays out of the way.")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                principle("hand.raised.slash",
                          "No mouse on stage",
                          "Every performance gesture is the turntable, the mixer, or the keyboard.")
                principle("slider.horizontal.3",
                          "Your mixer is the mixer",
                          "EQ, crossfade, gain and cue live on your hardware — Dub adds no software mixer.")
                principle("shield.lefthalf.filled",
                          "Reliability first",
                          "A crash on stage ends the night. Dub trades flash for never letting you down.")
            }
            .padding(.top, DubSpacing.xs)
            Spacer(minLength: 0)
        }
    }

    private var audioPage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            Text("Audio configures itself")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(DubColor.textPrimary)
            Text("Connect a DJ interface with at least four inputs and four outputs and Dub runs Performance mode, pulling the deck channels from its device registry. With no interface connected it runs Track Preparation through the built-in output. Hot-plugging switches modes live — there is nothing to pick here.")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            VStack(alignment: .leading, spacing: DubSpacing.xs) {
                statusRow(label: "Mode", value: model.engineMode.displayName)
                statusRow(label: "Device", value: currentDeviceLabel)
            }
            .padding(DubSpacing.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))

            Button {
                model.refreshDevices()
            } label: {
                Label("Re-scan audio devices", systemImage: "arrow.clockwise")
            }
            .buttonStyle(.plain)
            .font(DubFont.body)
            .foregroundStyle(DubColor.deckATint)
            Spacer(minLength: 0)
        }
    }

    private var libraryPage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            Text("Add your music")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(DubColor.textPrimary)
            Text("Point Dub at a folder of audio files. It scans them into the library, detects BPM and key, and builds beat grids in the background. You can always import more later from the library’s Import Folder button.")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            Button {
                presentImportFolderPicker()
            } label: {
                Label("Import a folder…", systemImage: "tray.and.arrow.down")
            }
            .controlSize(.large)

            if model.libraryModel.libraryTrackCount > 0 {
                Text("\(model.libraryModel.libraryTrackCount) tracks in your library.")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
            } else {
                Text("Your library is empty — import a folder to get started, or skip and do it later.")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
    }

    // MARK: - Header / footer

    private var header: some View {
        HStack {
            Text("Welcome to Dub")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(DubColor.textPrimary)
            Spacer()
            Button("Skip") { onFinish() }
                .buttonStyle(.plain)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .keyboardShortcut(.cancelAction)
        }
    }

    private var footer: some View {
        HStack(spacing: DubSpacing.md) {
            HStack(spacing: DubSpacing.xs) {
                ForEach(0...Self.lastStep, id: \.self) { i in
                    Circle()
                        .fill(i == step ? DubColor.deckATint : DubColor.textPlaceholder)
                        .frame(width: 6, height: 6)
                }
            }
            Spacer(minLength: 0)
            if step > 0 {
                Button("Back") { step -= 1 }
            }
            if step < Self.lastStep {
                Button("Next") { step += 1 }
                    .keyboardShortcut(.defaultAction)
            } else {
                Button("Get started") { onFinish() }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    // MARK: - Helpers

    private func principle(_ symbol: String, _ title: String, _ detail: String) -> some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            Image(systemName: symbol)
                .font(.system(size: 15))
                .foregroundStyle(DubColor.deckATint)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
                Text(detail)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func statusRow(label: String, value: String) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Text(label)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: 64, alignment: .leading)
            Text(value)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
            Spacer(minLength: 0)
        }
    }

    /// Mirrors `PreferencesSheet.currentDeviceLabel`: the human-
    /// readable name of whatever the engine auto-selected.
    private var currentDeviceLabel: String {
        switch model.engineMode {
        case .timecode:
            return model.selectedInputDevice?.name
                ?? model.performanceDevices.first?.name
                ?? "No interface connected"
        case .prep:
            if let uid = model.selectedOutputUID,
               let dev = model.outputDevices.first(where: { $0.uid == uid }) {
                return dev.name
            }
            return "Built-in output (system default)"
        }
    }

    private func presentImportFolderPicker() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Import"
        panel.message = "Choose a folder of audio files to add to the library."
        if panel.runModal() == .OK, let url = panel.url {
            Task { @MainActor in
                await model.importLibraryFolder(url)
            }
        }
    }
}

#Preview {
    OnboardingSheet(model: WaveformAppModel(), onFinish: {})
}
