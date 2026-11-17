//
//  OnboardingSheet.swift
//  Dub
//
//  The welcome guide (U-23, reworked 2026-11-17). It used to be three
//  pages about what Dub is; it had gone stale (it said audio needs no
//  setup after routing moved to Preferences ▸ Audio, and offered only a
//  folder import to a DJ whose music lives in Traktor). Now it is a setup
//  checklist that reads the rig as it stands:
//
//  1. Your rig — what Dub found: an interface (Performance) or none (Prep).
//  2. Audio — the microphone permission, explained before macOS asks, and
//     the interface's routing: ready for a box Dub knows, set right here
//     for one it doesn't.
//  3. Signal — drop the needle; a light per deck goes green when timecode
//     arrives. The wiring is checked at home, not at the gig.
//  4. Music — the DJ apps on this Mac to read (read-only), or a folder.
//  5. Keys — the default map, read from the keymap (so it is always the
//     map as it stands), and how to move a key.
//  6. Ready — the source switch, calibration, the manual.
//
//  Audio and signal only appear in Performance. Skippable at every step,
//  re-openable from Preferences (`.dubShowOnboarding`); finishing or
//  skipping persists `dub.hasCompletedOnboarding`. A new interface does
//  not reopen it (Daniel, 2026-11-17).
//

import AVFoundation
import DubCore
import SwiftUI

/// A page of the guide. Pure, so the order is tested.
enum OnboardingStep: Equatable {
    case rig, audio, signal, music, keys, ready

    static func steps(performance: Bool) -> [OnboardingStep] {
        performance
            ? [.rig, .audio, .signal, .music, .keys, .ready]
            : [.rig, .music, .keys, .ready]
    }
}

/// What the signal check says about one deck's input. Pure, so the rule
/// is tested; the row reads the engine ten times a second.
enum DeckSignalCheck: Equatable {
    /// The input isn't open — no permission, or the routing has no pair.
    case notOpen
    /// The pair is silent.
    case nothing
    /// Something arrives that is neither timecode nor music yet.
    case signal
    /// A record is playing, not a control record.
    case music
    /// Timecode arrives: the wiring is right.
    case timecode
    /// Timecode arrives and the deck is locked to it.
    case locked

    static func of(hasInput: Bool, rms: Float, sourceClass: UInt8, lockState: UInt8) -> Self {
        guard hasInput else { return .notOpen }
        guard InputSignal.segments(rms: rms) > 0 else { return .nothing }
        if lockState == 1 { return .locked }
        switch sourceClass {
        case 1: return .timecode
        case 2: return .music
        default: return .signal
        }
    }

    /// The wiring is proven.
    var isGood: Bool { self == .timecode || self == .locked }

    var word: String {
        switch self {
        case .notOpen: return "Input not open"
        case .nothing: return "Nothing yet — drop the needle"
        case .signal: return "Something arrives — keep the needle down"
        case .music: return "Music, not timecode — is the control record on?"
        case .timecode: return "Timecode arriving"
        case .locked: return "Timecode locked"
        }
    }

    var colour: Color {
        switch self {
        case .notOpen, .nothing: return DubColor.textTertiary
        case .signal: return DubColor.textSecondary
        case .music: return DubColor.stateTentative
        case .timecode, .locked: return DubColor.stateLocked
        }
    }
}

/// One deck's line in the signal check: the lamp, the deck, the word.
/// Value-driven, so a snapshot pins every state.
struct OnboardingSignalRow: View {
    let side: DeckSide
    let check: DeckSignalCheck

    var body: some View {
        HStack(spacing: DubSpacing.md) {
            Image(systemName: check.isGood ? "checkmark.circle.fill" : "circle.dotted")
                .font(.system(size: 18))
                .foregroundStyle(check.isGood ? DubColor.stateLocked : DubColor.textTertiary)
                .frame(width: 22)
            Text(side.label)
                .font(DubFont.caps)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textPrimary)
                .frame(width: 64, alignment: .leading)
            Text(check.word)
                .font(DubFont.body)
                .foregroundStyle(check.colour)
            Spacer(minLength: 0)
        }
        .padding(DubSpacing.md)
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}

/// One position of the deck's source switch, as the switch prints it.
struct OnboardingSwitchLine: View {
    let position: String
    let detail: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: DubSpacing.md) {
            Text(position)
                .font(DubFont.caps)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textPrimary)
                .frame(width: 44)
                .padding(.vertical, 3)
                .background(DubColor.surface2)
                .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
            Text(detail)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
    }
}

/// A checklist line: a tick, a warning or a dot, then what it means.
struct OnboardingStatusLine: View {
    enum Kind { case good, attention, neutral }
    let state: Kind
    let title: String
    let detail: String?

    var body: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            Image(systemName: symbol)
                .font(.system(size: 15))
                .foregroundStyle(colour)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
                if let detail {
                    Text(detail)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
        }
    }

    private var symbol: String {
        switch state {
        case .good: return "checkmark.circle.fill"
        case .attention: return "exclamationmark.triangle.fill"
        case .neutral: return "circle"
        }
    }

    private var colour: Color {
        switch state {
        case .good: return DubColor.stateLocked
        case .attention: return DubColor.stateTentative
        case .neutral: return DubColor.textTertiary
        }
    }
}

struct OnboardingSheet: View {

    @ObservedObject var model: WaveformAppModel
    @ObservedObject private var libraryModel: LibraryAppModel
    @ObservedObject private var keymap = DubKeymapStore.shared

    /// Called when the user finishes the last page or taps Skip. The
    /// host (MainView) persists the completion flag and dismisses.
    var onFinish: () -> Void

    @State private var index: Int = 0
    @State private var micStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .audio)
    @State private var sourceLocations: [LibrarySourceLocation] = []

    static let manualURL = URL(string: "https://dub-3ccc829c.mintlify.site/first-set")!
    private static let micSettingsURL = URL(
        string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!

    init(model: WaveformAppModel, onFinish: @escaping () -> Void) {
        self.model = model
        self._libraryModel = ObservedObject(wrappedValue: model.libraryModel)
        self.onFinish = onFinish
    }

    private var performance: Bool { model.engineMode == .timecode }
    private var steps: [OnboardingStep] { OnboardingStep.steps(performance: performance) }
    /// Clamped: an interface unplugged mid-guide drops two steps.
    private var position: Int { min(index, steps.count - 1) }
    private var step: OnboardingStep { steps[position] }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            header
            Divider()
            ScrollView {
                page
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                    .padding(.trailing, DubSpacing.sm)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            Divider()
            footer
        }
        .padding(DubSpacing.xl)
        .frame(width: 640, height: 620)
        .background(DubColor.surface0)
        .tint(DubColor.controlAccent)
        .onAppear {
            sourceLocations = model.discoveredSourceLocations()
            refreshMic()
        }
        .onChange(of: position) { _ in refreshMic() }
        .onChange(of: model.seratoImportEnabled) { on in if on { model.scanEnabledSources() } }
        .onChange(of: model.traktorImportEnabled) { on in if on { model.scanEnabledSources() } }
        .onChange(of: model.rekordboxImportEnabled) { on in if on { model.scanEnabledSources() } }
        .onChange(of: model.itunesImportEnabled) { on in if on { model.scanEnabledSources() } }
    }

    @ViewBuilder
    private var page: some View {
        switch step {
        case .rig: rigPage
        case .audio: audioPage
        case .signal: signalPage
        case .music: musicPage
        case .keys: keysPage
        case .ready: readyPage
        }
    }

    // MARK: - Pages

    private var rigPage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            title("Your rig")
            para("Dub is for scratch and sound-system DJs: your hands stay on the turntables and your mixer, and nothing matters more than it not letting you down on stage.")
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                if performance {
                    OnboardingStatusLine(
                        state: .good,
                        title: "\(model.activeInterface?.name ?? "A DJ interface") connected — Performance",
                        detail: "Your turntables drive the decks through timecode vinyl; your mixer does the mixing.")
                } else {
                    OnboardingStatusLine(
                        state: .neutral,
                        title: "No DJ interface — Prep",
                        detail: "Prepare tracks now through the Mac's own output: grids, cues, loops. Plug in an interface with at least four inputs and four outputs and Dub switches to Performance by itself.")
                }
            }
            .padding(DubSpacing.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            Button {
                model.refreshDevices()
            } label: {
                Label("Look for interfaces again", systemImage: "arrow.clockwise")
            }
            .buttonStyle(.plain)
            .font(DubFont.body)
            .foregroundStyle(DubColor.textSecondary)
        }
    }

    private var audioPage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            title("Audio")
            micLine
            if let device = model.activeInterface {
                if model.knownInterface(device) != nil {
                    OnboardingStatusLine(
                        state: .good,
                        title: "Dub knows the \(device.name) — routing is ready",
                        detail: "Each deck's inputs and outputs are already set. Preferences ▸ Audio shows them.")
                } else if model.isRunning {
                    OnboardingStatusLine(
                        state: model.hasSavedRouting(device) ? .good : .attention,
                        title: model.hasSavedRouting(device)
                            ? "You routed the \(device.name)"
                            : "Dub doesn't know the \(device.name) yet",
                        detail: "Tell it where each deck comes in and goes out. Drop the needle and the meter shows which pair carries the timecode.")
                    AudioRoutingSection(model: model, device: device)
                        .id(device.uid)
                }
            }
        }
    }

    @ViewBuilder
    private var micLine: some View {
        switch micStatus {
        case .authorized:
            OnboardingStatusLine(
                state: .good, title: "Dub can hear the interface",
                detail: "Microphone access is how macOS names an app reading audio inputs: the timecode comes in that way.")
        case .notDetermined:
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                OnboardingStatusLine(
                    state: .attention, title: "macOS will ask for microphone access",
                    detail: "That is how macOS names reading the interface's inputs, where the timecode comes in. Allow it, or the decks hear nothing.")
                Button("Allow access…") {
                    model.ensureInputPermission { _ in refreshMic() }
                }
                .padding(.leading, 34)
            }
        default:
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                OnboardingStatusLine(
                    state: .attention, title: "Microphone access is off",
                    detail: "Without it the decks hear nothing from the interface. Turn Dub on in System Settings ▸ Privacy & Security ▸ Microphone, then reopen Dub.")
                Button("Open System Settings") { NSWorkspace.shared.open(Self.micSettingsURL) }
                    .padding(.leading, 34)
            }
        }
    }

    private var signalPage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            title("Check the signal")
            para("Put the timecode records on, drop the needle on each and let them play. A deck turns green when its timecode arrives: the turntable, the cable and the routing are right.")
            TimelineView(.periodic(from: .now, by: 0.1)) { _ in
                VStack(spacing: DubSpacing.sm) {
                    ForEach(model.twoDeckMode ? [DeckSide.a, .b] : [.a], id: \.self) { side in
                        OnboardingSignalRow(side: side, check: check(side))
                    }
                }
            }
            note("Music instead of timecode on a deck is right for THRU, where a real record plays through. Nothing at all: check the turntable's cable and the routing on the step before.")
        }
    }

    private var musicPage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            title("Your music")
            para("Use the library you already have. Dub reads it and never writes to it: grids, cues and playlists stay as they are in that app.")
            VStack(spacing: 0) {
                sourceRow("Traktor", kind: .traktor, isOn: $model.traktorImportEnabled)
                sourceRow("Serato", kind: .serato, isOn: $model.seratoImportEnabled)
                sourceRow("rekordbox", kind: .rekordbox, isOn: $model.rekordboxImportEnabled)
                sourceRow("Apple Music", kind: .itunes, isOn: $model.itunesImportEnabled, last: true)
            }
            .padding(.horizontal, DubSpacing.md)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            HStack(spacing: DubSpacing.md) {
                Text("Or a folder of audio files:")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                Button {
                    presentImportFolderPicker()
                } label: {
                    Label("Import a folder…", systemImage: "tray.and.arrow.down")
                }
            }
            Text(libraryLine)
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
        }
    }

    private var keysPage: some View {
        let bound = keymap.resolved.filter { $0.action.isRemappable }
        return VStack(alignment: .leading, spacing: DubSpacing.md) {
            title("Keys")
            para("The left hand plays deck A, the right hand deck B. To move a key, turn MAP on in the top bar, click a control and press the key you want; actions with no button are in Preferences ▸ Keys.")
            Text("BOUND NOW")
                .font(DubFont.caps)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textSecondary)
            LazyVGrid(
                columns: [GridItem(.flexible(), alignment: .leading), GridItem(.flexible(), alignment: .leading)],
                alignment: .leading, spacing: DubSpacing.xs
            ) {
                ForEach(Array(bound.enumerated()), id: \.offset) { _, binding in
                    HStack(spacing: DubSpacing.sm) {
                        DubKeycap(key: binding.legend, size: .list)
                            .frame(minWidth: 44, alignment: .leading)
                        Text(PreferencesSheet.displayName(binding.action, model: model))
                            .font(DubFont.micro)
                            .foregroundStyle(DubColor.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
            }
            .padding(DubSpacing.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        }
    }

    private var readyPage: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            title("Ready")
            para("Each deck has a source switch at its top:")
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                OnboardingSwitchLine(position: "TC", detail: "The timecode record drives the track. What you'll use most.")
                OnboardingSwitchLine(position: "INT", detail: "The track plays on its own clock — also how you save a set when a needle fails.")
                OnboardingSwitchLine(position: "THRU", detail: "A real record on the platter plays straight through.")
            }
            para("Calibrate once per needle and control record: ↻ next to the switch, with the needle down and the record playing.")
            Link(destination: Self.manualURL) {
                Label("Your first set, in the manual", systemImage: "book")
            }
            .font(DubFont.body)
            note("This guide is in Preferences whenever you want it again.")
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
                ForEach(steps.indices, id: \.self) { i in
                    Circle()
                        .fill(i == position ? DubColor.controlAccent : DubColor.textPlaceholder)
                        .frame(width: 6, height: 6)
                }
            }
            Spacer(minLength: 0)
            if position > 0 {
                Button("Back") { index = position - 1 }
            }
            if position < steps.count - 1 {
                Button("Next") { index = position + 1 }
                    .keyboardShortcut(.defaultAction)
            } else {
                Button("Get started") { onFinish() }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    // MARK: - Helpers

    private func check(_ side: DeckSide) -> DeckSignalCheck {
        let t = model.engine.deckTelemetry(deckIdx: side.ffiDeckIdx)
        return .of(
            hasInput: t.hasTimecodeInput, rms: t.inputRms,
            sourceClass: t.sourceClass, lockState: t.lockState)
    }

    private func refreshMic() {
        micStatus = AVCaptureDevice.authorizationStatus(for: .audio)
    }

    private var libraryLine: String {
        if libraryModel.libraryImportInProgress { return "Reading…" }
        let n = libraryModel.libraryTrackCount
        return n > 0
            ? "\(n) tracks in your library."
            : "Your library is empty. You can add music later from the library or Preferences ▸ Library."
    }

    private func sourceRow(
        _ label: String, kind: ImportedSourceKind, isOn: Binding<Bool>, last: Bool = false
    ) -> some View {
        let loc = sourceLocations.first { $0.kind == kind.sourceTag }
        let found = loc?.exists == true
        return VStack(spacing: 0) {
            HStack(spacing: DubSpacing.md) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(label)
                        .font(DubFont.body)
                        .foregroundStyle(found ? DubColor.textPrimary : DubColor.textTertiary)
                    Text(found ? "Found on this Mac" : "Not found on this Mac")
                        .font(DubFont.micro)
                        .foregroundStyle(found ? DubColor.textTertiary : DubColor.textPlaceholder)
                }
                Spacer(minLength: DubSpacing.md)
                Toggle("", isOn: isOn)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .disabled(!found && !isOn.wrappedValue)
            }
            .padding(.vertical, DubSpacing.sm)
            if !last { Divider() }
        }
    }

    private func title(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(DubColor.textPrimary)
    }

    private func para(_ text: String) -> some View {
        Text(text)
            .font(DubFont.body)
            .foregroundStyle(DubColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(DubFont.micro)
            .foregroundStyle(DubColor.textTertiary)
            .fixedSize(horizontal: false, vertical: true)
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
