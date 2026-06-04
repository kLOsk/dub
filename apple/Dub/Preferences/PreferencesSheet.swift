//
//  PreferencesSheet.swift
//  Dub
//
//  Preferences sheet. Dub is opinionated about audio (PRD §3): the
//  engine mode is derived from the hardware, not chosen by the user.
//  Plug in a DJ interface and the app runs Performance mode with the
//  deck channels pulled from `devices.toml`; with no interface it runs
//  Track Preparation on the built-in output; hot-plugging switches
//  live. There is therefore NO Prep/Performance switch, no input
//  picker, and no channel fields in a shipping build — the sheet only
//  shows a read-only status line plus the track-loading safety toggle.
//
//  A DEV-only block (compiled in `#if DEBUG` only) adds a manual mode
//  override and device/channel overrides so the performance UI can be
//  exercised on a Mac with no DJ interface. None of it ships.
//
//  Opened via `⌘,` or the status-strip gear icon. Esc / Close dismiss.
//

import SwiftUI
import DubCore

struct PreferencesSheet: View {

    @ObservedObject var model: WaveformAppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            header
            Divider()
            statusSection
            #if DEBUG
            devSection
            #endif
            loadBehaviourSection
            Spacer(minLength: 0)
            Divider()
            footer
        }
        .padding(DubSpacing.xl)
        .frame(width: 520, height: 600)
        .background(DubColor.surface0)
    }

    // MARK: - Status (read-only)

    /// Read-only summary of what the engine auto-selected. There is no
    /// control here on purpose: the hardware decides the mode and the
    /// registry decides the channels.
    private var statusSection: some View {
        section(title: "AUDIO") {
            VStack(alignment: .leading, spacing: DubSpacing.xs) {
                statusRow(label: "Mode", value: model.engineMode.displayName)
                statusRow(label: "Device", value: currentDeviceLabel)
                if model.engineMode == .timecode {
                    statusRow(label: "Decks", value: currentChannelLabel)
                }
                Text("Dub configures audio automatically: connect a DJ interface to enter Performance mode (deck channels come from devices.toml); with none connected it runs Track Preparation through the built-in output. Add an interface to devices.toml to support new hardware.")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, DubSpacing.xs)
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

    /// Human-readable label for the device currently in play.
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

    /// Registry-resolved deck channels for the active interface, shown
    /// read-only so the user can see what Dub picked.
    private var currentChannelLabel: String {
        guard let device = model.selectedInputDevice ?? model.performanceDevices.first
        else { return "—" }
        let r = model.engine.performanceRoutingFor(deviceName: device.name)
        let a = r.deckAInput.map(String.init).joined(separator: "+")
        guard r.twoDeck else { return "A \(a) (single deck)" }
        let b = r.deckBInput.map(String.init).joined(separator: "+")
        return "A \(a) · B \(b)"
    }

    // MARK: - Track loading safety

    /// Load-into-playing-deck guard toggle (M10.5r). PRD §5.5 + §6.4
    /// default the engine to refusing a load on a running deck; this
    /// toggle lets the user opt out of the safety rule in
    /// Performance mode. Prep mode is unaffected — it's a single-deck
    /// rehearsal shell where the rule never applied.
    private var loadBehaviourSection: some View {
        section(title: "TRACK LOADING") {
            VStack(alignment: .leading, spacing: DubSpacing.xs) {
                Toggle(isOn: $model.allowLoadIntoRunningDeckInPerformance) {
                    Text("Allow loading onto a playing deck (Performance mode)")
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                }
                .toggleStyle(.switch)
                Text("When off, a drop / Space-load onto a deck that is currently playing flashes the pane red — the DJ has to lift the needle or pause first (PRD §5.5). Prep mode always allows the load regardless of this setting.")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: - Header / footer

    private var header: some View {
        HStack {
            Text("Preferences")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(DubColor.textPrimary)
            Spacer()
            #if DEBUG
            Text("DEBUG build — dev overrides shown")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textPlaceholder)
            #endif
        }
    }

    private var footer: some View {
        HStack(spacing: DubSpacing.md) {
            if let err = model.lastError {
                Text(err)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.stateError)
                    .lineLimit(2)
            }
            Spacer(minLength: 0)
            // U-23 — let users re-open the first-run guide. Dismiss
            // this sheet first; MainView brings onboarding up on the
            // next tick (two sheets can't present at once).
            Button("Show Welcome Guide") {
                dismiss()
                NotificationCenter.default.post(name: .dubShowOnboarding, object: nil)
            }
            .buttonStyle(.plain)
            .font(DubFont.body)
            .foregroundStyle(DubColor.textSecondary)
            // Single Close button bound only to `.cancelAction` (Esc).
            // Everything in this sheet either auto-applies or is
            // read-only, so there is nothing to manually commit.
            Button("Close") { dismiss() }
                .keyboardShortcut(.cancelAction)
        }
    }

    // MARK: - Helpers

    @ViewBuilder
    private func section<Content: View>(
        title: String,
        @ViewBuilder _ content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            Text(title)
                .font(DubFont.caps)
                .tracking(1.0)
                .foregroundStyle(DubColor.textSecondary)
            content()
        }
    }

    #if DEBUG
    // MARK: - DEV overrides (never compiled into Release)

    /// Developer-only controls. Dub can't exercise Performance mode on
    /// built-in audio, so this block lets a developer force the mode
    /// and pin devices to drive the performance UI without a real DVS
    /// interface. Compiled out of shipping builds entirely.
    private var devSection: some View {
        section(title: "DEVELOPER") {
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                Picker("Mode override", selection: devModeBinding) {
                    Text("Auto (hardware)").tag(EngineMode?.none)
                    Text("Force Track Preparation").tag(EngineMode?.some(.prep))
                    Text("Force Performance").tag(EngineMode?.some(.timecode))
                }
                .pickerStyle(.menu)

                Picker("Performance source", selection: devSourceBinding) {
                    ForEach(PerformanceSource.allCases) { src in
                        Text(src.displayName).tag(src)
                    }
                }
                .pickerStyle(.menu)
                .disabled(model.engineMode != .timecode)

                HStack(spacing: DubSpacing.sm) {
                    Picker("Input", selection: devInputBinding) {
                        if model.performanceDevices.isEmpty {
                            Text("No DJ interfaces found").tag(Optional<String>.none)
                        } else {
                            ForEach(model.performanceDevices, id: \.uid) { d in
                                Text(d.name).tag(Optional<String>.some(d.uid))
                            }
                        }
                    }
                    .pickerStyle(.menu)
                    Button {
                        model.refreshDevices()
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .help("Re-scan devices")
                }

                Picker("Output (Prep only)", selection: devOutputBinding) {
                    Text("Auto (interface / built-in)").tag(Optional<String>.none)
                    ForEach(model.outputDevices, id: \.uid) { d in
                        Text(d.name).tag(Optional<String>.some(d.uid))
                    }
                }
                .pickerStyle(.menu)
                .disabled(model.engineMode == .timecode)

                Text("Dev-only: forces the mode and pins devices so the performance UI can be exercised without a real DJ interface. Performance source picks how the decks are driven — Timecode (control vinyl → loaded file, the product behaviour) or Thru (real-record live passthrough). The output picker applies to Track Preparation only — in Performance mode the master always returns through the interface itself (deck A → 3+4, deck B → 5+6). None of this ships in Release; production mode is hardware-derived only.")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var devModeBinding: Binding<EngineMode?> {
        Binding(
            get: { model.devForcedMode },
            set: { model.devForcedMode = $0 }  // didSet re-detects + restarts
        )
    }

    private var devSourceBinding: Binding<PerformanceSource> {
        Binding(
            get: { model.devForcedSource },
            set: { model.devForcedSource = $0 }  // didSet restarts in timecode mode
        )
    }

    private var devInputBinding: Binding<String?> {
        Binding(
            get: { model.selectedInputUID },
            set: {
                model.selectedInputUID = $0
                model.applyConfig()
            }
        )
    }

    private var devOutputBinding: Binding<String?> {
        Binding(
            get: { model.selectedOutputUID },
            set: { model.selectedOutputUID = $0 }  // onChange in MainView applies
        )
    }
    #endif
}

#Preview {
    PreferencesSheet(model: WaveformAppModel())
}
