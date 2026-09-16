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
//  picker, and no channel fields in a shipping build — the Audio tab
//  is a read-only status line.
//
//  Six tabs down the left, one per thing the DJ is configuring — AUDIO
//  · DECKS · FX · LIBRARY · VINYL · KEYS — the shape of Traktor's
//  preferences and of System Settings, in Dub's own chrome. It replaced
//  (2026-09-16) one scroll of eight caps-titled blocks, each toggle
//  under a paragraph, where FX sat below the fold and the sheet read
//  as a list rather than a surface. A row is label and control on one
//  line, the explanation under it in the tertiary tone, so a tab scans
//  as a column of decisions rather than a page of prose. Every control
//  auto-applies; nothing here is committed by a button.
//
//  A DEV-only block (compiled in `#if DEBUG` only) on the Audio tab
//  adds a manual mode override and device/channel overrides so the
//  performance UI can be exercised on a Mac with no DJ interface. None
//  of it ships.
//
//  Opened via `⌘,` or the status-strip gear icon. Esc / Close dismiss.
//

import SwiftUI
import DubCore

/// The tabs, in sidebar order: what is played through first, what the
/// DJ plays with last.
enum PreferencesTab: String, CaseIterable, Identifiable {
    case audio
    case decks
    case fx
    case library
    case vinyl
    case keys

    var id: String { rawValue }

    var title: String {
        switch self {
        case .audio: return "Audio"
        case .decks: return "Decks"
        case .fx: return "FX"
        case .library: return "Library"
        case .vinyl: return "Vinyl"
        case .keys: return "Keys"
        }
    }

    /// One line under the tab's title: what the tab decides.
    var intro: String {
        switch self {
        case .audio: return "What the hardware decided. Dub configures audio from the interface you plug in."
        case .decks: return "How a deck behaves when a track lands on it."
        case .fx: return "The dub instruments on the surface, and the FX channel."
        case .library: return "The DJ apps whose libraries Dub reads into your one collection."
        case .vinyl: return "Recording records into the library, and naming what you ripped."
        case .keys: return "What the keyboard fires. Mapped on the live surface, listed here."
        }
    }
}

struct PreferencesSheet: View {

    @ObservedObject var model: WaveformAppModel
    /// The key map, for the Keys tab's list — observed so a rebind on
    /// the surface behind the sheet shows up here.
    @ObservedObject private var keymap = DubKeymapStore.shared
    @Environment(\.dismiss) private var dismiss

    /// The tab is remembered: a DJ who lives in FX opens on FX.
    @AppStorage("dub.preferencesTab") private var tab: PreferencesTab = .audio
    @State private var confirmResetKeys = false

    init(model: WaveformAppModel) {
        self.model = model
    }

    private static let sidebarWidth: CGFloat = 148

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            header
            Divider()
            HStack(spacing: 0) {
                sidebar
                    .frame(width: Self.sidebarWidth)
                Divider()
                    .padding(.horizontal, DubSpacing.lg)
                ScrollView {
                    VStack(alignment: .leading, spacing: DubSpacing.xl) {
                        tabHeading
                        tabContent
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.trailing, DubSpacing.sm)
                }
            }
            Divider()
            footer
        }
        .padding(DubSpacing.xl)
        // Clears a 900-pt window with its title bar; every tab fits
        // without scrolling at this size, which is the point of tabs.
        .frame(width: 760, height: 780)
        .background(DubColor.surface0)
        // "On" is the control accent everywhere else on the surface; the
        // switches say it in the same colour rather than the system blue.
        .tint(DubColor.controlAccent)
        .onAppear { sourceLocations = model.discoveredSourceLocations() }
        .onChange(of: model.seratoImportEnabled) { on in if on { model.scanEnabledSources() } }
        .onChange(of: model.traktorImportEnabled) { on in if on { model.scanEnabledSources() } }
        .onChange(of: model.rekordboxImportEnabled) { on in if on { model.scanEnabledSources() } }
        .onChange(of: model.itunesImportEnabled) { on in if on { model.scanEnabledSources() } }
    }

    /// Default locations of the external libraries, loaded once on appear so
    /// each row can show a "Found / Not found" status without re-statting the
    /// filesystem on every render.
    @State private var sourceLocations: [LibrarySourceLocation] = []

    // MARK: - Sidebar

    /// The tab list: quiet rows, the selected one lifted onto `surface3`
    /// in the primary tone — the library sidebar's own selection idiom,
    /// so the sheet reads as part of the app and not as a dialog.
    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(PreferencesTab.allCases) { entry in
                let selected = entry == tab
                Button {
                    tab = entry
                } label: {
                    Text(entry.title)
                        .font(DubFont.body.weight(selected ? .semibold : .regular))
                        .foregroundStyle(selected ? DubColor.textPrimary : DubColor.textSecondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, DubSpacing.md)
                        .padding(.vertical, 6)
                        .background(selected ? DubColor.surface3 : Color.clear)
                        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            }
            Spacer(minLength: 0)
        }
    }

    private var tabHeading: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            Text(tab.title)
                .font(DubFont.title)
                .foregroundStyle(DubColor.textPrimary)
            Text(tab.intro)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder
    private var tabContent: some View {
        switch tab {
        case .audio: audioTab
        case .decks: decksTab
        case .fx: fxTab
        case .library: libraryTab
        case .vinyl: vinylTab
        case .keys: keysTab
        }
    }

    // MARK: - Audio (read-only)

    /// Read-only summary of what the engine auto-selected. There is no
    /// control here on purpose: the hardware decides the mode and the
    /// registry decides the channels.
    private var audioTab: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("Now") {
                statusRow(label: "Mode", value: model.engineMode.displayName)
                statusRow(label: "Device", value: currentDeviceLabel)
                if model.engineMode == .timecode {
                    statusRow(label: "Decks", value: currentChannelLabel)
                }
                note("Connect a DJ interface to enter Performance mode — deck channels come from devices.toml. With none connected Dub runs Track Preparation through the built-in output. Add an interface to devices.toml to support new hardware.")
            }
            #if DEBUG
            devSection
            #endif
        }
    }

    private func statusRow(label: String, value: String) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Text(label)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: 72, alignment: .leading)
            Text(value)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
            Spacer(minLength: 0)
        }
        .padding(.vertical, 2)
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

    // MARK: - Decks

    /// The three per-deck behaviours: the load-into-playing guard
    /// (M10.5r, PRD §5.5 / §6.4), loudness auto-gain (PRD §8.4) and
    /// hot-cue grid snap.
    private var decksTab: some View {
        group("Loading") {
            toggleRow(
                "Allow loading onto a playing deck",
                "In Performance mode. When off, a drop or Space-load onto a running deck flashes the pane red — lift the needle or pause first (PRD §5.5). Prep always allows the load.",
                isOn: $model.allowLoadIntoRunningDeckInPerformance)
            toggleRow(
                "Auto-match loudness on load",
                "A track with a measured loudness (LUFS-I) loads at a gain that matches it to the reference level, so decks sit at one loudness without riding the trim. Off, tracks load untouched and you set level at the mixer. Loudness is measured and shown either way.",
                isOn: $model.loudnessAutoGainEnabled)
            toggleRow(
                "Snap cue points to the beat grid",
                "Setting a hot cue snaps the marker to the nearest beat line, so a rough tap lands clean. Off, the cue lands exactly at the playhead. Tracks with no analysed grid keep the raw position either way.",
                isOn: $model.cueSnapToGridEnabled, last: true)
        }
    }

    // MARK: - FX

    /// The dub instruments (PRD §6.3) and the FX channel (F-38).
    private var fxTab: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("On the decks") {
                toggleRow(
                    "Echo out",
                    "One ECHO OUT button per deck. Tap it and the tune cuts to 100 % wet — the last beat repeats and decays — while the deck keeps playing underneath; tap again and it picks up where it has got to.",
                    isOn: $model.echoOutEnabled, last: true)
            }
            group("On the rack bar") {
                toggleRow(
                    "Dub siren",
                    "The siren box: five shots — Rifle Gun, Alarm, Sine, Laser, Siren — and a DUB knob for the box's own echo. Three vintage chips recreated, through a PT2399 echo. Keys come from map mode.",
                    isOn: $model.sirenEnabled)
                toggleRow(
                    "Beat-match the siren's echo",
                    "Locks the siren's echo to the deck's tempo — one beat per repeat — instead of each shot's own slap-back, so the echoes fall in time with the tune.",
                    isOn: $model.sirenDelaySync, enabled: model.sirenEnabled, last: true)
            }
            group("The FX channel") {
                toggleRow(
                    "Dub FX channel",
                    "Adds a DUB FX position to each deck's source switch. Flip a deck to it and the deck stops being a turntable: the record is replaced by the vintage rack — Big Knob, phaser, Space Echo, spring — on the mixer's send, patched into the pair that deck's needle used, returning on its output pair. Every knob is the unit's own; ride them from a controller.",
                    isOn: $model.dubFxEnabled, last: true)
            }
        }
    }

    // MARK: - Library (Serato / Traktor / rekordbox / Apple Music import)

    /// Per-source import toggles. Enabling a source scans its default folder
    /// now and on every launch; tracks merge into the one library and the
    /// app's crates/playlists appear in the sidebar's Imported Sources
    /// section. Read-only on the source data.
    private var libraryTab: some View {
        group("Import from") {
            note("Dub reads each library read-only, merges its tracks into your collection, and shows that app's crates and playlists in the sidebar. Enabling a source scans its default folder now and on every launch.")
                .padding(.bottom, DubSpacing.xs)
            librarySourceRow("Serato", kind: .serato, isOn: $model.seratoImportEnabled)
            librarySourceRow("Traktor", kind: .traktor, isOn: $model.traktorImportEnabled)
            librarySourceRow("rekordbox", kind: .rekordbox, isOn: $model.rekordboxImportEnabled)
            librarySourceRow("Apple Music", kind: .itunes, isOn: $model.itunesImportEnabled, last: true)
        }
    }

    private func librarySourceRow(
        _ label: String,
        kind: ImportedSourceKind,
        isOn: Binding<Bool>,
        last: Bool = false
    ) -> some View {
        let loc = sourceLocations.first { $0.kind == kind.sourceTag }
        return VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: DubSpacing.md) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(label)
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                    Text(sourceStatusText(loc))
                        .font(DubFont.micro)
                        .foregroundStyle(
                            loc?.exists == true ? DubColor.textTertiary : DubColor.textPlaceholder)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: DubSpacing.md)
                if isOn.wrappedValue {
                    Button("Rescan") { model.scanEnabledSources() }
                        .buttonStyle(.plain)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textSecondary)
                        .disabled(loc?.exists != true || !model.libraryModel.libraryIsOpen)
                }
                Toggle("", isOn: isOn)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .controlSize(.small)
            }
            .padding(.vertical, DubSpacing.sm)
            if !last { Divider() }
        }
    }

    private func sourceStatusText(_ loc: LibrarySourceLocation?) -> String {
        guard let loc else { return "Not found." }
        return loc.exists ? "Found: \(loc.path)" : "Not found at \(loc.path)"
    }

    // MARK: - Vinyl (M26a rip + M26c recognition)

    /// Vinyl-recording feature toggle. When on, Prep mode grows a RIP
    /// VINYL row and — while a DJ interface is connected — the status
    /// strip shows a manual PREP / PERF switch so the DJ can hop into
    /// Prep to record without unplugging.
    private var vinylTab: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("Recording") {
                toggleRow(
                    "Vinyl recording",
                    "Record a side through your DJ interface in Prep mode. A PREP / PERF switch appears in the top bar while an interface is connected.",
                    isOn: $model.vinylRecordingEnabled, last: true)
            }
            group("Identify ripped tracks") {
                fieldRow(
                    "AcoustID key",
                    "Free from acoustid.org/new-application. Without it, Identify in the rip review panel does nothing. Naming a track costs one request and never contacts MusicBrainz.",
                    text: $model.acoustIdKey, placeholder: "AcoustID key")
                toggleRow(
                    "Also identify the pressing",
                    "Adds album, label and catalogue number by asking MusicBrainz at one request a second. Slower, and it often cannot settle on one release for a compilation — artist and title arrive without it.",
                    isOn: $model.ripIdentifyPressing, last: !model.ripIdentifyPressing)
                if model.ripIdentifyPressing {
                    fieldRow(
                        "Discogs token",
                        "Optional. Adds Discogs' style tags and pressing detail. Kept in your login Keychain, not in Dub's preferences.",
                        text: $model.discogsToken, placeholder: "Discogs token", last: true)
                }
            }
        }
    }

    // MARK: - Keys (M18 map mode)

    /// The map is made on the live surface — MAP in the top bar, click a
    /// control, press a key — so this tab lists rather than edits: every
    /// bound action with its cap, and the one thing the surface cannot
    /// do, which is start over.
    private var keysTab: some View {
        let bound = keymap.resolved.filter { $0.action.isRemappable }
        return VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("How to map") {
                note("Turn MAP on in the top bar. Every bindable control shows its cap; click one and press the key you want on it — the same key on another control moves it. Escape backs out, ⌫ leaves a control unbound. Nothing you play has a default key; Space loads the selection and ⌘, opens this sheet.")
            }
            group("Bound now") {
                if bound.isEmpty {
                    Text("Nothing mapped yet.")
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPlaceholder)
                        .padding(.vertical, DubSpacing.sm)
                } else {
                    ForEach(Array(bound.enumerated()), id: \.offset) { index, binding in
                        VStack(alignment: .leading, spacing: 0) {
                            HStack(spacing: DubSpacing.md) {
                                Text(displayName(binding.action))
                                    .font(DubFont.body)
                                    .foregroundStyle(DubColor.textPrimary)
                                Spacer(minLength: DubSpacing.md)
                                DubKeycap(key: binding.legend, size: .list)
                            }
                            .padding(.vertical, DubSpacing.xs)
                            if index < bound.count - 1 { Divider() }
                        }
                    }
                }
                // Only once something differs from the defaults; a fresh map
                // has nothing to reset.
                HStack {
                    Spacer(minLength: 0)
                    Button("Reset all bindings…") { confirmResetKeys = true }
                        .buttonStyle(.plain)
                        .font(DubFont.body)
                        .foregroundStyle(keymap.hasOverrides ? DubColor.stateError : DubColor.textPlaceholder)
                        .disabled(!keymap.hasOverrides)
                        .confirmationDialog(
                            "Reset every key binding?", isPresented: $confirmResetKeys,
                            titleVisibility: .visible
                        ) {
                            Button("Reset", role: .destructive) { keymap.reset() }
                        } message: {
                            Text("Every control you mapped goes back to having no key; Space and ⌘, come back.")
                        }
                }
                .padding(.top, DubSpacing.sm)
            }
        }
    }

    /// The name a binding prints in the list.
    private func displayName(_ action: DubAction) -> String {
        switch action {
        case .loadSelection: return "Load the selection"
        case .openPreferences: return "Preferences"
        case .tapGrid: return "Tap the grid"
        case .hotCue(let i): return "Hot cue \(i + 1)"
        case .sirenPreset(let i):
            let names = model.sirenPresetLabels
            return "Siren · \(i < names.count ? names[i] : "shot \(i + 1)")"
        case .instantDouble(let toB): return toB ? "Instant double → B" : "Instant double → A"
        case .sampler(let i):
            let name = model.sampleBank.slot(i).map { SampleBank.label(for: $0) }
            return "Sample \(i + 1)" + (name.map { " · \($0)" } ?? "")
        case .quickScratch(let side, let i): return "Quick Scratch \(side.label) · \(i + 1)"
        case .fxToggle(let i):
            return "FX · " + (FxRackUnit(rawValue: i)?.title.capitalized ?? "unit \(i + 1)") + " in/out"
        case .fxKick: return "FX · Kick the spring"
        case .echoOut(let side): return "Echo out \(side.label)"
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
            Text("DEBUG build — dev overrides on the Audio tab")
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
            // Signal quality lives on the decks now — the SIGNAL tab on
            // each deck pane's outer edge (`DeckSignalSlideOut`)
            // replaced the old sheet that was buried here.
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

    // MARK: - Rows

    /// A titled group of rows: caps heading, then the rows in a column.
    @ViewBuilder
    private func group<Content: View>(
        _ title: String,
        @ViewBuilder _ content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            Text(title.uppercased())
                .font(DubFont.caps)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textSecondary)
                .padding(.bottom, DubSpacing.xs)
            content()
        }
    }

    /// A setting: the name and the switch on one line, what it does
    /// underneath. `last` drops the hairline so a group ends clean.
    private func toggleRow(
        _ title: String,
        _ description: String,
        isOn: Binding<Bool>,
        enabled: Bool = true,
        last: Bool = false
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: DubSpacing.md) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(DubFont.body)
                        .foregroundStyle(enabled ? DubColor.textPrimary : DubColor.textTertiary)
                    Text(description)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DubSpacing.md)
                Toggle("", isOn: isOn)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .disabled(!enabled)
            }
            .padding(.vertical, DubSpacing.sm)
            if !last { Divider() }
        }
    }

    /// A setting that is a value: the name, the field, what it is for.
    private func fieldRow(
        _ title: String,
        _ description: String,
        text: Binding<String>,
        placeholder: String,
        last: Bool = false
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: DubSpacing.md) {
                    Text(title)
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                        .frame(width: 120, alignment: .leading)
                    TextField(placeholder, text: text)
                        .textFieldStyle(.roundedBorder)
                        .font(DubFont.body)
                }
                Text(description)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.vertical, DubSpacing.sm)
            if !last { Divider() }
        }
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(DubFont.micro)
            .foregroundStyle(DubColor.textTertiary)
            .fixedSize(horizontal: false, vertical: true)
    }

    #if DEBUG
    // MARK: - DEV overrides (never compiled into Release)

    /// Developer-only controls. Dub can't exercise Performance mode on
    /// built-in audio, so this block lets a developer force the mode
    /// and pin devices to drive the performance UI without a real DVS
    /// interface. Compiled out of shipping builds entirely.
    private var devSection: some View {
        group("Developer") {
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

                Picker("Output (Prep / Internal)", selection: devOutputBinding) {
                    Text("Auto (interface / built-in)").tag(Optional<String>.none)
                    ForEach(model.outputDevices, id: \.uid) { d in
                        Text(d.name).tag(Optional<String>.some(d.uid))
                    }
                }
                .pickerStyle(.menu)
                .disabled(model.engineMode == .timecode && !model.isInternalMixer)

                note("Dev-only: forces the mode and pins devices so the performance UI can be exercised without a real DJ interface. Performance source picks how the decks are driven — Timecode (control vinyl → loaded file, the product behaviour), Thru (real-record live passthrough), or Internal (both decks summed to the built-in soundcard, no input, each playing its file on its own clock — the no-hardware dogfood path). The output picker applies to Track Preparation and Internal; in Timecode / Thru the master always returns through the interface itself (deck A → 3+4, deck B → 5+6). While the override is engaged, Vinyl recording also works without an interface: it records from the pinned input or the built-in microphone (mono is duplicated to both channels) and monitors through the pinned or default output — mind the mic→speaker feedback loop. None of this ships in Release; production mode is hardware-derived only.")
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
