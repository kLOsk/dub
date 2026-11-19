//
//  PreferencesSheet.swift
//  Dub
//
//  Preferences sheet. Dub is opinionated about audio (PRD §3): the
//  engine mode is derived from the hardware, not chosen by the user.
//  Plug in a DJ interface and the app runs Performance mode with the
//  deck channels pulled from `devices.toml`; with no interface it runs
//  Track Preparation on the built-in output; hot-plugging switches
//  live. There is therefore NO Prep/Performance switch and no input
//  picker. The one exception to "the registry decides the channels"
//  is an interface `devices.toml` does not know: the Audio tab lets the
//  DJ pick its pairs (2026-11-15, FFI 87) — two shipped maps cannot
//  cover the interfaces DJs actually own.
//
//  Seven tabs down the left, one per thing the DJ is configuring — AUDIO
//  · DECKS · FX · LIBRARY · VINYL · KEYS · ALPHA — the shape of Traktor's
//  preferences and of System Settings, in Dub's own chrome. It replaced
//  (2026-09-16) one scroll of eight caps-titled blocks, each toggle
//  under a paragraph, where FX sat below the fold and the sheet read
//  as a list rather than a surface. A row is label and control on one
//  line, the explanation under it in the tertiary tone, so a tab scans
//  as a column of decisions rather than a page of prose. Every control
//  auto-applies; nothing here is committed by a button.
//
//  ALPHA holds what works but is not finished, behind one master switch
//  that is off by default (2026-11-12, DUB FX first).
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
import UniformTypeIdentifiers

/// The tabs, in sidebar order: what is played through first, what the
/// DJ plays with last.
enum PreferencesTab: String, CaseIterable, Identifiable {
    case audio
    case decks
    case fx
    case library
    case vinyl
    case keys
    case alpha

    var id: String { rawValue }

    var title: String {
        switch self {
        case .audio: return "Audio"
        case .decks: return "Decks"
        case .fx: return "FX"
        case .library: return "Library"
        case .vinyl: return "Vinyl"
        case .keys: return "Keys"
        case .alpha: return "Alpha"
        }
    }

    /// One line under the tab's title: what the tab decides.
    var intro: String {
        switch self {
        case .audio: return "Where each deck comes in and goes out. Dub sets this up from the interface you plug in."
        case .decks: return "How a deck behaves when a track lands on it."
        case .fx: return "The dub instruments on the surface."
        case .library: return "The DJ apps whose libraries Dub reads into your one collection."
        case .vinyl: return "Recording records into the library, and naming what you ripped."
        case .keys: return "What the keyboard fires. Mapped on the live surface, listed here."
        case .alpha: return "Features that work but aren't finished. They will change; leave them off at a gig."
        }
    }
}

struct PreferencesSheet: View {

    @ObservedObject var model: WaveformAppModel
    /// The key map, for the Keys tab's list — observed so a rebind on
    /// the surface behind the sheet shows up here.
    @ObservedObject private var keymap = DubKeymapStore.shared
    /// For the Matching window, which lives on the library model.
    @ObservedObject private var libraryModel: LibraryAppModel
    @Environment(\.dismiss) private var dismiss

    /// The tab is remembered: a DJ who lives in FX opens on FX.
    @AppStorage("dub.preferencesTab") private var tab: PreferencesTab = .audio
    @State private var confirmResetKeys = false

    init(model: WaveformAppModel) {
        self.model = model
        self._libraryModel = ObservedObject(wrappedValue: model.libraryModel)
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
        // An action armed on the Keys tab must not keep waiting for a key
        // once the sheet is gone and nothing shows that it is.
        .onDisappear { if !model.mapMode { model.cancelMapping() } }
        .onChange(of: tab) { _ in if !model.mapMode { model.cancelMapping() } }
        .onChange(of: model.seratoImportEnabled) { _ in model.sourceSwitched(.serato) }
        .onChange(of: model.traktorImportEnabled) { _ in model.sourceSwitched(.traktor) }
        .onChange(of: model.rekordboxImportEnabled) { _ in model.sourceSwitched(.rekordbox) }
        .onChange(of: model.itunesImportEnabled) { _ in model.sourceSwitched(.itunes) }
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
        case .alpha: alphaTab
        }
    }

    // MARK: - Audio

    /// The hardware still decides the mode. In Performance the interface
    /// and its routing: read-only for a box `devices.toml` knows, set by
    /// the DJ for any other (`AudioRoutingSection`, FFI 87).
    private var audioTab: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            if model.engineMode == .timecode, model.isRunning, let device = model.activeInterface {
                AudioRoutingSection(model: model, device: device)
                    .id(device.uid)
            } else {
                group("Now") {
                    statusRow(label: "Mode", value: model.engineMode.displayName)
                    statusRow(label: "Device", value: currentDeviceLabel)
                    note("Connect a DJ interface to enter Performance mode, and its routing shows here. With none connected Dub runs Track Preparation through the built-in output.")
                }
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

    // MARK: - Decks

    /// The three per-deck behaviours: the load-into-playing guard
    /// (M10.5r, PRD §5.5 / §6.4), loudness auto-gain (PRD §8.4) and
    /// hot-cue grid snap.
    private var decksTab: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("Browsing") {
                toggleRow(
                    "Browse mode",
                    "In Performance. Click anywhere in the library and it slides up over the deck controls, to just under the track overviews, so you see more of the crate. Loading a track onto a deck, or clicking outside the library, slides it back down.",
                    isOn: $browseMode, last: true)
            }
            group("Screen") {
                toggleRow(
                    "Keep the menu bar away in full screen",
                    "The menu bar and Dock stay hidden even when the pointer touches the edge of the screen, so nothing slides over the decks mid-set. The keys still work: ⌘, for Preferences, ⌃⌘F to leave full screen, ⌘Q to quit (Dub asks first). Off, macOS shows them when you reach the edge.",
                    isOn: $hideMenuBar, last: true)
                    .onChange(of: hideMenuBar) { _ in MainWindowController.applyMenuBarPreference() }
            }
            loadingGroup
        }
    }

    @AppStorage(PerformanceView.browseModeKey) private var browseMode = false
    @AppStorage(MainWindowController.hideMenuBarKey) private var hideMenuBar = true

    private var loadingGroup: some View {
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
                isOn: $model.cueSnapToGridEnabled)
            toggleRow(
                "Show what you usually play next",
                "In Performance, the deck shows “↝ usually:” and the track you have most often mixed into from the loaded one, between elapsed and remaining. Click it to find that track in the library. Learned from your own mixes; nothing leaves this Mac.",
                isOn: $model.historyHintEnabled, last: true)
        }
    }

    // MARK: - FX

    /// The dub instruments (PRD §6.3). The FX channel (F-38) is on Alpha.
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
                    "Locks the siren's echo to the deck's tempo — one beat per repeat, or half a beat when a beat is longer than the echo can hold — instead of the DUB knob's time, so the repeats fall in time with the tune.",
                    isOn: $model.sirenDelaySync, enabled: model.sirenEnabled, last: true)
            }
        }
    }

    // MARK: - Alpha

    /// The master switch, then each unfinished feature's own. With the
    /// master off a feature's switch reads off and greyed, whatever it
    /// was left at — that is what the surface does too — and comes back
    /// as it was when the master goes on.
    private var alphaTab: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("Alpha") {
                toggleRow(
                    "Alpha features",
                    "Lets the switches below turn their features on. Off, every one of them is gone from the surface.",
                    isOn: $model.alphaFeaturesEnabled, last: true)
            }
            group("In progress") {
                toggleRow(
                    "Dub FX channel",
                    "Adds a DUB FX position to each deck's source switch, and HOLD to each deck's echo. Flip a deck to it and the deck stops being a turntable: the record is replaced by the vintage rack — Big Knob, phaser, Space Echo, spring — on the mixer's send, patched into the pair that deck's needle used, returning on its output pair. Every knob is the unit's own; ride them from a controller.",
                    isOn: model.alphaFeaturesEnabled ? $model.dubFxEnabled : .constant(false),
                    enabled: model.alphaFeaturesEnabled)
                toggleRow(
                    "Following beat grid",
                    "For records played by a band. When a track loads and its beats wander off any even grid — live reggae, soul, rock — each beat gets its own line where the band played it. A steady track keeps its even grid. Nothing is saved: turn this off and load the track again to get the even grid back.",
                    isOn: model.alphaFeaturesEnabled ? $model.followingGridEnabled : .constant(false),
                    enabled: model.alphaFeaturesEnabled, last: true)
            }
        }
    }

    // MARK: - Library (Serato / Traktor / rekordbox / Apple Music import)

    /// Per-source import toggles. Enabling a source scans its default folder
    /// now and on every launch; tracks merge into the one library and the
    /// app's crates/playlists appear in the sidebar's Imported Sources
    /// section. Read-only on the source data.
    private var libraryTab: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("Import from") {
                note("Dub reads each library read-only, merges its tracks into your collection, and shows that app's crates and playlists in the sidebar. Enabling a source scans its default folder now and on every launch.")
                    .padding(.bottom, DubSpacing.xs)
                librarySourceRow(
                    "Serato", kind: .serato, isOn: $model.seratoImportEnabled,
                    detail: "Serato DJ 4's new library, or the _Serato_ folder of Serato 3 and earlier: Dub reads whichever Serato wrote last. Smart crates are left out.")
                librarySourceRow("Traktor", kind: .traktor, isOn: $model.traktorImportEnabled)
                librarySourceRow("rekordbox", kind: .rekordbox, isOn: $model.rekordboxImportEnabled)
                librarySourceRow("Apple Music", kind: .itunes, isOn: $model.itunesImportEnabled, last: true)
                note("Apple Music keeps each of its libraries apart (hold ⌥ as you open Music to switch). Dub reads the one you choose here when it starts; right-click Apple Music in the sidebar to read it again after you change it. In Music, turn on Settings ▸ Files ▸ Share Library XML with other applications; Music then keeps that library's XML current, and Choose… takes it, or a library you exported with File ▸ Library ▸ Export Library….")
                    .padding(.top, DubSpacing.xs)
            }
            group("Matching") {
                note("The Matching smart crate lists what mixes with the track playing now, nearest first, and follows it as the music changes.")
                    .padding(.bottom, DubSpacing.xs)
                matchStepperRow(
                    "Tempo",
                    "How far a track's BPM may be from the playing one, either side.",
                    value: "± \(LibraryView.bpmFormat(libraryModel.matchWindow.bpm)) BPM",
                    decrement: { libraryModel.matchWindow.bpm = max(MatchWindow.bpmRange.lowerBound, libraryModel.matchWindow.bpm - 0.5) },
                    increment: { libraryModel.matchWindow.bpm = min(MatchWindow.bpmRange.upperBound, libraryModel.matchWindow.bpm + 0.5) })
                matchStepperRow(
                    "Key",
                    "Steps round the Camelot wheel. 0: the same key only. 1: a step either way, and the relative major or minor. Each step more widens it again.",
                    value: Self.keyStepsLabel(libraryModel.matchWindow.keySteps),
                    decrement: { libraryModel.matchWindow.keySteps = max(0, libraryModel.matchWindow.keySteps - 1) },
                    increment: { libraryModel.matchWindow.keySteps = min(MatchWindow.keyStepsRange.upperBound, libraryModel.matchWindow.keySteps + 1) })
                toggleRow(
                    "Half and double time",
                    "Also match a track at half or double the tempo: an 87 BPM track beside a 174 one.",
                    isOn: $libraryModel.matchWindow.halfDouble)
                toggleRow(
                    "Match the pitched tempo",
                    "Match the tempo the deck plays at now, with your platter's pitch on it. Off, Matching uses the playing track's own BPM.",
                    isOn: $libraryModel.matchWindow.followPitch, last: true)
            }
            // Daniel, 2026-11-19: half, double and "three-quarter" tempos on
            // a newly added library. The octave is the DJ's to set, as in
            // Traktor; the genre only overrides it where it names one.
            group("Tempo") {
                matchStepperRow(
                    "Tempo range",
                    "A beat can be counted at half or double speed: 70 or 140, 87 or 174. Dub reads every song inside one range, so the two never trade places. This range is for songs whose genre doesn't settle it.",
                    value: Self.tempoRangeLabel(model.tempoRangeLow),
                    decrement: { stepTempoRange(by: -1) },
                    increment: { stepTempoRange(by: 1) },
                    last: true)
                note("A genre that does settle it has its own range: drum & bass 100–200, house, garage, grime and dubstep 88–176, dancehall 75–150, hip-hop 68–136, reggae and dub 60–120.")
                    .padding(.bottom, DubSpacing.xs)
                note("The range applies to songs analysed from now on. A song that already has a grid keeps it: right-click its BPM on the deck for 2× or ½, or Re-analyze it in the library.")
            }
            // Daniel, 2026-11-16: with more than one app on, say whose grid
            // and whose cues a track gets. 2026-11-18: and let the DJ set
            // the order (`Library::import_order`, FFI 90).
            group("When a track is in more than one app") {
                note("A track you find through any of them is the same track: Dub matches them by the file.")
                    .padding(.bottom, DubSpacing.xs)
                importOrderRows
                    .padding(.bottom, DubSpacing.sm)
                note("Beat grid. A grid you fix in Dub comes first. Then \(orderedAppNames), then Dub's own analysis. Apple Music has no grids. Dub still analyses every track and marks its BPM with ⚠ when the grid in use disagrees with that analysis. A grid the app locked arrives locked.")
                    .padding(.bottom, DubSpacing.xs)
                note("Key. A key you set in Dub comes first, then \(orderedAppNames), then Dub's own.")
                    .padding(.bottom, DubSpacing.xs)
                note("Hot cues. Cues you set in Dub come first. A track you haven't touched in Dub shows the hot cues of the first app in the same order that has any: \(orderedAppNames). One app's set, never a mix, and the HOTCUE heading says which. Your first change to one of those cues makes the set yours: the change sticks, and later edits in that app no longer reach Dub's pads.")
                    .padding(.bottom, DubSpacing.xs)
                note("Dub never writes to these apps. What you change in Dub stays in Dub; File ▸ Export Library As… takes it elsewhere.")
            }
        }
    }

    private func librarySourceRow(
        _ label: String,
        kind: ImportedSourceKind,
        isOn: Binding<Bool>,
        detail: String? = nil,
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
                    if let detail {
                        Text(detail)
                            .font(DubFont.micro)
                            .foregroundStyle(DubColor.textTertiary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: DubSpacing.md)
                if kind == .itunes {
                    if model.itunesLibraryPath != nil {
                        Button("Default") {
                            model.chooseItunesLibrary(nil)
                            sourceLocations = model.discoveredSourceLocations()
                        }
                        .buttonStyle(.plain)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textSecondary)
                        .help("Follow the library at Apple Music's default location again.")
                    }
                    Button("Choose…") { chooseItunesLibrary() }
                        .buttonStyle(.plain)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textSecondary)
                        .help("Pick which Apple Music library Dub follows.")
                }
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

    /// A value set by − / +: name and what it does, the value, the pair.
    private func matchStepperRow(
        _ title: String, _ description: String, value: String,
        decrement: @escaping () -> Void, increment: @escaping () -> Void, last: Bool = false
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: DubSpacing.md) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                    Text(description)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DubSpacing.md)
                Text(value)
                    .font(DubFont.body)
                    .monospacedDigit()
                    .foregroundStyle(DubColor.textPrimary)
                    .frame(minWidth: 72, alignment: .trailing)
                ControlGroup {
                    Button(action: decrement) { Image(systemName: "minus") }
                    Button(action: increment) { Image(systemName: "plus") }
                }
                .controlSize(.small)
                .frame(width: 64)
            }
            .padding(.vertical, DubSpacing.sm)
            if !last { Divider() }
        }
    }

    /// The three apps in the DJ's order, each with a button to move it up.
    private var importOrderRows: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Which app comes first")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .padding(.top, DubSpacing.sm)
            Text("Its beat grid, key and hot cues win when a track has them from more than one app. Changing it updates every track you haven't locked or set yourself.")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, DubSpacing.xs)
            ForEach(Array(model.importOrder.enumerated()), id: \.element) { index, source in
                HStack(spacing: DubSpacing.md) {
                    Text("\(index + 1)")
                        .font(DubFont.body)
                        .monospacedDigit()
                        .foregroundStyle(DubColor.textTertiary)
                        .frame(width: 16, alignment: .trailing)
                    Text(Self.appName(source))
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                    Spacer(minLength: DubSpacing.md)
                    Button {
                        var order = model.importOrder
                        order.swapAt(index, index - 1)
                        model.setImportOrder(order)
                    } label: {
                        Image(systemName: "chevron.up")
                    }
                    .controlSize(.small)
                    .disabled(index == 0)
                    .help("Move \(Self.appName(source)) up")
                }
                .padding(.vertical, 3)
            }
        }
    }

    private var orderedAppNames: String {
        let names = model.importOrder.map(Self.appName)
        guard names.count == 3 else { return names.joined(separator: ", ") }
        return "\(names[0]), then \(names[1]), then \(names[2])"
    }

    static func appName(_ source: String) -> String {
        switch source {
        case "serato": return "Serato"
        case "rekordbox": return "rekordbox"
        case "traktor": return "Traktor"
        default: return source
        }
    }

    static func tempoRangeLabel(_ low: Double) -> String {
        "\(Int(low))–\(Int(low * 2)) BPM"
    }

    /// Move the tempo range one choice down or up, stopping at the ends.
    private func stepTempoRange(by step: Int) {
        let choices = WaveformAppModel.tempoRangeChoices
        let current = choices.firstIndex(of: model.tempoRangeLow) ?? 2
        let next = min(max(current + step, 0), choices.count - 1)
        model.setTempoRangeLow(choices[next])
    }

    static func keyStepsLabel(_ steps: Int) -> String {
        steps == 0 ? "Same key" : "± \(steps) step\(steps == 1 ? "" : "s")"
    }

    /// Pick the Apple Music library XML Dub follows. Choosing turns the
    /// source on: picking a library is asking for it.
    private func chooseItunesLibrary() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.xml]
        panel.prompt = "Follow"
        panel.message = "Choose the Apple Music library XML Dub should follow."
        if let current = model.itunesLibraryLocation?.path {
            panel.directoryURL = URL(fileURLWithPath: current).deletingLastPathComponent()
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        model.chooseItunesLibrary(url.path)
        if !model.itunesImportEnabled { model.itunesImportEnabled = true }
        sourceLocations = model.discoveredSourceLocations()
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
                    "Record a side through your DJ interface in Prep mode. A PREP · REC · PERF switch appears in the top bar; REC starts a rip.",
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

    /// Controls on the surface are mapped there — MAP in the top bar,
    /// click a control, press a key. What has no control to click (load
    /// into a deck, instant doubles, the grid tap, loop off, the library
    /// keys) is mapped here, the same way: click its cap, press the key.
    /// Below that, every other bound action with its cap, and the reset.
    private var keysTab: some View {
        let buttonless = Set(DubKeymap.buttonless)
        let bound = keymap.resolved.filter {
            $0.action.isRemappable && !buttonless.contains($0.action)
        }
        return VStack(alignment: .leading, spacing: DubSpacing.xl) {
            group("How to map") {
                note("Turn MAP on in the top bar. Every bindable control shows its cap; click one and press the key you want on it — the same key on another control moves it. Escape backs out, ⌫ leaves a control unbound. Dub starts with a full map — the left hand plays deck A, the right hand deck B — and every key in it moves the same way.")
            }
            group("Actions with no button") {
                note("These have nothing on the surface to click, so map them here: click a cap, then press the key.")
                    .padding(.bottom, DubSpacing.xs)
                ForEach(Array(DubKeymap.buttonless.enumerated()), id: \.offset) { index, action in
                    buttonlessRow(action, last: index == DubKeymap.buttonless.count - 1)
                }
            }
            group("Modifiers") {
                note("A key with a modifier is a key of its own: ⇧← loads deck A while ← does something else, and ⌘J and J are two keys. When nothing is bound to the key with ⇧ or ⌥ held, they change what the plain key does instead: ⇧ with a hot cue's key clears that cue (so does ⌘⇧, as in Serato); ⇧ with Tap the BPM halves a track's tempo, ⌥ doubles it.")
            }
            group("Bound on the surface") {
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
                            Text("Every key goes back to Dub's own map; what you mapped yourself is forgotten.")
                        }
                }
                .padding(.top, DubSpacing.sm)
            }
        }
    }

    /// One action with no button: its name, what it does, and its cap,
    /// which arms it for the next key — PRESS A KEY while it waits.
    private func buttonlessRow(_ action: DubAction, last: Bool) -> some View {
        let armed = model.mapArmed == action
        let legend = DubKeymap.legend(for: action)
        return VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: DubSpacing.md) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(displayName(action))
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                    if let detail = Self.buttonlessDetail(action) {
                        Text(detail)
                            .font(DubFont.micro)
                            .foregroundStyle(DubColor.textTertiary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: DubSpacing.md)
                Button {
                    model.armFromPreferences(action)
                } label: {
                    if armed {
                        Text("PRESS A KEY")
                            .font(.system(size: 9, weight: .bold))
                            .tracking(0.6)
                            .foregroundStyle(DubColor.surface0)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 4)
                            .background(DubColor.controlAccent)
                            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
                    } else if let legend {
                        DubKeycap(key: legend, size: .list)
                    } else {
                        Text("Set key")
                            .font(DubFont.micro)
                            .foregroundStyle(DubColor.textSecondary)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 4)
                            .overlay(
                                RoundedRectangle(cornerRadius: 4, style: .continuous)
                                    .strokeBorder(
                                        DubColor.controlAccent.opacity(0.55),
                                        style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
                    }
                }
                .buttonStyle(.plain)
                .help(armed
                      ? "Press the key — Escape cancels, ⌫ unbinds"
                      : "Click, then press the key for this action")
            }
            .padding(.vertical, DubSpacing.xs)
            if !last { Divider() }
        }
    }

    /// The line under a no-button action's name, where the name alone
    /// leaves a question.
    static func buttonlessDetail(_ action: DubAction) -> String? {
        switch action {
        case .loadSelection:
            return "The empty one; A if both are. With both loaded, the deck that isn't the master."
        case .loadInto:
            return "Refused while that deck plays, as for Space."
        case .instantDouble:
            return "The other deck's track, at its playhead."
        case .focusSearch:
            return "Type, then the next-track key (an arrow or F-key) leaves the field."
        case .sidebar:
            return "In the sidebar on the left; a folder is a stop too."
        case .folder:
            return "Close on a playlist goes up to its folder."
        default:
            return nil
        }
    }

    /// The name a binding prints in the list.
    private func displayName(_ action: DubAction) -> String {
        Self.displayName(action, model: model)
    }

    /// Shared with the welcome guide's keys step, so an action reads the
    /// same wherever it is listed.
    static func displayName(_ action: DubAction, model: WaveformAppModel) -> String {
        switch action {
        case .loadSelection: return "Load into the empty deck"
        case .loadInto(let side): return "Load into \(side == .a ? "the left deck (A)" : "the right deck (B)")"
        case .openPreferences: return "Preferences"
        case .loopOff(let side): return "Loop off \(side.label)"
        case .loopSize(let side, let i):
            let beats = LoopEngine.sizes.indices.contains(i) ? LoopEngine.sizes[i] : 0
            return "Loop \(LoopEngine.label(beats)) \(side.label)"
        case .loopScale(let side, let double): return "Loop \(double ? "×2" : "÷2") \(side.label)"
        case .focusSearch: return "Jump to search"
        case .browse(let next): return next ? "Next track" : "Previous track"
        case .hotCue(let i): return "Hot cue \(i + 1) · master deck"
        case .deckCue(let side, let i): return "Hot cue \(i + 1) · \(side.label)"
        case .sidebar(let next): return next ? "Next playlist" : "Previous playlist"
        case .folder(let open): return open ? "Open folder" : "Close folder"
        case .sirenPreset(let i):
            let names = model.sirenPresetLabels
            return "Siren · \(i < names.count ? names[i] : "shot \(i + 1)")"
        case .instantDouble(let toB): return toB ? "Double onto B (from A)" : "Double onto A (from B)"
        case .sampler(let i):
            let name = model.sampleBank.label(i)
            return "Sample \(i + 1)" + (name.map { " · \($0)" } ?? "")
        case .quickScratch(let side, let i): return "Quick Scratch \(side.label) · \(i + 1)"
        case .fxToggle(let i):
            return "FX · " + (FxRackUnit(rawValue: i)?.title.capitalized ?? "unit \(i + 1)") + " in/out"
        case .fxKick: return "FX · Kick the spring"
        case .echoOut(let side): return "Echo out \(side.label)"
        case .echoHold(let side): return "Echo hold \(side.label)"
        case .fxHold: return "FX · Space Echo hold"
        case .fxTap: return "FX · Space Echo tap tempo"
        case .keyLock(let side): return "Key lock \(side.label)"
        case .tapTempo(let side): return "Tap the BPM \(side.label) · ⇧ ½ · ⌥ 2×"
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
