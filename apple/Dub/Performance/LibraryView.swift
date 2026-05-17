//
//  LibraryView.swift
//  Dub
//
//  M11d.1 — Library browser shell. Replaces the M10.5b
//  `FileBrowserView` in the LIBRARY region of the Performance View.
//  Backed by the M11a–c SQLite catalog through `DubLibrary`
//  (`crates/dub-ffi`).
//
//  PRD §8.5 surface, staged across four sub-milestones:
//
//      M11d.1 (THIS) — Functional replacement: source tree (All
//        Tracks + Smart Crates skeleton + read-only-source
//        placeholders), virtualized track list with title /
//        artist / album / BPM / key / duration / source columns,
//        FTS5-backed substring search, "Import Folder…" affordance
//        wiring the M11c importer, Space + drag load paths
//        preserved via `model.browserSelection`.
//
//      M11d.2 — Smart Crates wired (Recently Played + Just
//        Imported populated; load hook in the deck transport
//        writes play_history rows), sortable columns.
//
//      M11d.3 — Per-row indicators: loaded-now `A` / `B` glyph,
//        grid-disagreement ⚠, potential-duplicate link, missing-
//        file glyph.
//
//      M11d.4 — Background missing-files scanner + Relocate panel
//        per §8.5.5.
//
//  v1.0 keeps the Dub Crates / Imported Sources / Real Records
//  sidebar nodes as visual placeholders (rendered greyscale + lock
//  glyph for Imported Sources, per §8.5.1) so the source-tree
//  shape doesn't churn on M11e / v1.1 / v1.x landing.
//
//  Drag-and-drop and Space load paths are preserved verbatim from
//  M10.5b: when the user selects a row, the view resolves the
//  canonical track UUID to its on-disk URL via
//  `model.selectLibraryTrack(_:)` which writes the URL into
//  `model.browserSelection`. The existing PerformanceView Space
//  shortcut and the existing AppKit drag path then work without
//  modification.

import AppKit
import SwiftUI
import UniformTypeIdentifiers

import DubCore

// MARK: - LibraryTrack convenience conformances

/// UniFFI's generated `LibraryTrack` already carries a `String id`
/// field — Swift's `Identifiable` conformance is a one-liner. Used
/// by SwiftUI `Table` to track row identity across sorts.
extension LibraryTrack: Identifiable {}

/// Computed accessors for SwiftUI `Table` sort. `KeyPathComparator`
/// requires `Comparable` end values; raw `Optional<String>` is not
/// `Comparable` in Swift's standard library. Folding to "" /
/// sentinel values keeps the column-header click sort working
/// while preserving the §8.5.3 "missing fields look empty, not
/// magically valuable" semantic.
extension LibraryTrack {
    var titleSortKey:    String { title ?? "" }
    var artistSortKey:   String { artist ?? "" }
    var albumSortKey:    String { album ?? "" }
    /// BPM sort: missing values pinned past every real BPM in
    /// either direction so they collect at one end of the table
    /// rather than punching holes through the middle. `Double`
    /// `.infinity` is the canonical "biggest plausible value"
    /// sentinel.
    var bpmSortKey:      Double { bpm ?? .infinity }
    var durationSortKey: UInt32 { durationMs }
    var yearSortKey:     Int32  { year ?? Int32.max }
}

/// Sections in the left-hand source tree per PRD §8.5.1.
///
/// `allTracks`, `recentlyPlayed`, `justImported` are wired in M11d.1;
/// the remaining sections render as disabled greyscale placeholders.
private enum LibrarySource: Hashable, Identifiable {
    case allTracks
    case recentlyPlayed
    case justImported
    case dubCratesPlaceholder
    case importedSourcesPlaceholder
    case realRecordsPlaceholder

    var id: Self { self }

    var label: String {
        switch self {
        case .allTracks:                   return "All Tracks"
        case .recentlyPlayed:              return "Recently Played"
        case .justImported:                return "Just Imported"
        case .dubCratesPlaceholder:        return "Dub Crates"
        case .importedSourcesPlaceholder:  return "Imported Sources"
        case .realRecordsPlaceholder:      return "Real Records"
        }
    }

    var systemImage: String {
        switch self {
        case .allTracks:                   return "music.note.list"
        case .recentlyPlayed:              return "clock.arrow.circlepath"
        case .justImported:                return "tray.and.arrow.down"
        case .dubCratesPlaceholder:        return "folder"
        case .importedSourcesPlaceholder:  return "lock.square"
        case .realRecordsPlaceholder:      return "opticaldisc"
        }
    }

    /// `false` for the v1.0 placeholders that render disabled.
    var isAvailable: Bool {
        switch self {
        case .allTracks, .recentlyPlayed, .justImported: return true
        default: return false
        }
    }

    /// Per §8.5.1, the sidebar groups sections under a heading.
    var group: String {
        switch self {
        case .allTracks:
            return "Library"
        case .recentlyPlayed, .justImported:
            return "Smart Crates"
        case .dubCratesPlaceholder:
            return "Dub Crates"
        case .importedSourcesPlaceholder:
            return "Imported Sources"
        case .realRecordsPlaceholder:
            return "Real Records"
        }
    }
}

/// Top-level library surface. Owns the source-tree selection state
/// and the search field; delegates persistent state (the
/// `DubLibrary` handle, import-in-progress flag, etc.) up to
/// `WaveformAppModel`.
struct LibraryView: View {

    @ObservedObject var model: WaveformAppModel

    /// Currently selected source-tree node. Drives which query the
    /// track list runs against.
    @State private var selectedSource: LibrarySource = .allTracks

    /// Current search input. Empty string → show the source's
    /// natural listing (no search filter). The PRD §8.5.4
    /// substring rule says "AND across whitespace-separated
    /// tokens", which is exactly what `DubLibrary.search`
    /// implements.
    @State private var searchText: String = ""

    /// Materialised track-row buffer. Refreshed whenever the
    /// source / search / library-state changes. M11d.2 still
    /// uses single-page fetches (5 000 rows); 100k-track
    /// libraries land paging at M11d.4.
    @State private var tracks: [LibraryTrack] = []

    /// `true` while the source is being queried. Drives the spinner
    /// at the top of the list.
    @State private var isLoading: Bool = false

    /// Current SwiftUI `Table` sort order. The Table view binds
    /// this to its column headers; on change, we re-query the
    /// library with the mapped `LibraryTrackSort` enum.
    @State private var sortOrder: [KeyPathComparator<LibraryTrack>] = [
        KeyPathComparator(\.title, order: .forward)
    ]

    /// Currently selected row id (canonical UUID). Used to keep
    /// the `Table` selection model in sync with
    /// `model.browserSelection` / `model.selectedLibraryTrackId`.
    @State private var selectedTrackId: LibraryTrack.ID? = nil

    /// M11d.4 — `true` while the Relocate sheet is presented.
    /// Bound to the missing-files footer button.
    @State private var showRelocateSheet: Bool = false

    /// Listing fetch limit. Sized for the M11d.1/2 single-page
    /// model; real virtualization + paging lands at M11d.4.
    private static let listingLimit: UInt32 = 5_000

    var body: some View {
        HStack(spacing: 0) {
            sidebar
                .frame(width: 200)
                .background(DubColor.surface1)
            Rectangle().fill(DubColor.divider).frame(width: 1)
            rightPane
        }
        .frame(minHeight: DubLayout.libraryMinHeight)
        .background(DubColor.surface0)
        .onAppear {
            refreshTracks()
        }
        .onChange(of: selectedSource) { _ in
            refreshTracks()
        }
        .onChange(of: model.libraryIsOpen) { _ in
            refreshTracks()
        }
        .onChange(of: model.libraryTrackCount) { _ in
            // Track count bumped → either an import just landed
            // or another window inserted rows. Refresh the visible
            // listing so the user sees the new rows immediately.
            refreshTracks()
        }
        .onChange(of: searchText) { _ in
            refreshTracks()
        }
        .sheet(isPresented: $showRelocateSheet) {
            RelocateSheet(model: model, isPresented: $showRelocateSheet)
        }
    }

    // MARK: - Sidebar

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            sidebarHeader
            Divider().overlay(DubColor.divider)
            sidebarTree
        }
    }

    private var sidebarHeader: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("LIBRARY")
                .font(DubFont.caps)
                .tracking(1.2)
                .foregroundStyle(DubColor.textSecondary)
            Spacer(minLength: 0)
            Text("\(model.libraryTrackCount)")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .help("Total tracks in library")
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.sm)
        .background(DubColor.surface2)
    }

    private var sidebarTree: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                section(
                    heading: "Library",
                    entries: [.allTracks])
                section(
                    heading: "Smart Crates",
                    entries: [.recentlyPlayed, .justImported])
                section(
                    heading: "Dub Crates",
                    entries: [.dubCratesPlaceholder])
                section(
                    heading: "Imported Sources",
                    entries: [.importedSourcesPlaceholder])
                section(
                    heading: "Real Records",
                    entries: [.realRecordsPlaceholder])
            }
            .padding(.vertical, DubSpacing.xs)
        }
    }

    @ViewBuilder
    private func section(heading: String, entries: [LibrarySource]) -> some View {
        Text(heading.uppercased())
            .font(DubFont.caps)
            .tracking(1.0)
            .foregroundStyle(DubColor.textTertiary)
            .padding(.horizontal, DubSpacing.lg)
            .padding(.top, DubSpacing.sm)
            .padding(.bottom, 2)
        ForEach(entries) { entry in
            sidebarRow(entry)
        }
    }

    private func sidebarRow(_ entry: LibrarySource) -> some View {
        let isSelected = entry == selectedSource && entry.isAvailable
        return HStack(spacing: DubSpacing.sm) {
            Image(systemName: entry.systemImage)
                .frame(width: 16)
                .foregroundStyle(
                    entry.isAvailable
                        ? (isSelected ? DubColor.textPrimary : DubColor.textSecondary)
                        : DubColor.textPlaceholder)
            Text(entry.label)
                .font(DubFont.body)
                .foregroundStyle(
                    entry.isAvailable
                        ? DubColor.textPrimary
                        : DubColor.textPlaceholder)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            if !entry.isAvailable {
                Image(systemName: "lock.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(DubColor.textPlaceholder)
                    .help("Coming in a later milestone (M11e / v1.1).")
            }
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.xs)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? DubColor.surface2 : Color.clear)
        .contentShape(Rectangle())
        .onTapGesture {
            if entry.isAvailable {
                selectedSource = entry
            }
        }
    }

    // MARK: - Right pane (search + track list)

    private var rightPane: some View {
        VStack(spacing: 0) {
            toolbar
            Divider().overlay(DubColor.divider)
            trackListContainer
            Divider().overlay(DubColor.divider)
            footer
        }
        .background(DubColor.surface0)
    }

    private var toolbar: some View {
        HStack(spacing: DubSpacing.sm) {
            // Search field. Per §8.5.4 it's a plain substring search,
            // not a typeahead — we wait for the user to commit a
            // character then re-query. SwiftUI's `TextField`
            // delivers each keystroke through `onChange`, which is
            // fast enough on FTS5 to feel typeahead-y in practice.
            HStack(spacing: 4) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(DubColor.textTertiary)
                TextField("Search artist, title, album, comment", text: $searchText)
                    .textFieldStyle(.plain)
                    .font(DubFont.body)
                if !searchText.isEmpty {
                    Button {
                        searchText = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(DubColor.textTertiary)
                    }
                    .buttonStyle(.plain)
                    .help("Clear search")
                }
            }
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 4)
            .background(DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: 4))

            Spacer(minLength: 0)

            Button {
                presentImportFolderPicker()
            } label: {
                Label("Import Folder…", systemImage: "tray.and.arrow.down")
            }
            .controlSize(.small)
            .disabled(!model.libraryIsOpen || model.libraryImportInProgress)
            .help(
                model.libraryImportInProgress
                    ? "An import is already running."
                    : "Add a folder of audio files to the library.")
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.sm)
        .background(DubColor.surface2)
    }

    @ViewBuilder
    private var trackListContainer: some View {
        if !model.libraryIsOpen {
            placeholderPane(
                title: "Preparing library…",
                subtitle: "The library will be ready in a moment.")
        } else if isLoading && tracks.isEmpty {
            placeholderPane(
                title: "Loading…",
                subtitle: nil)
        } else if tracks.isEmpty {
            placeholderPane(
                title: emptyTitle,
                subtitle: emptySubtitle)
        } else {
            trackList
        }
    }

    /// Sorted track buffer for the SwiftUI `Table`. Sort happens
    /// client-side against the in-memory `tracks` snapshot
    /// (~5 000 rows fits comfortably; M11d.4 paging swaps this
    /// for an FFI `listTracksSorted` round-trip when the buffer
    /// exceeds the page size). Client-side sort gives instant
    /// header-click feedback — the FFI is reserved for the
    /// initial fetch + page boundaries.
    private var sortedTracks: [LibraryTrack] {
        tracks.sorted(using: sortOrder)
    }

    /// SwiftUI `Table` per PRD §8.5.3. Columns auto-render sort
    /// indicators on click; the `sortOrder` binding flips the
    /// `KeyPathComparator` direction. Selection is bound to
    /// `selectedTrackId` so clicking a row routes through
    /// `model.selectLibraryTrack(_:)` for Space + drag plumbing.
    ///
    /// The leftmost gutter column carries the M11d.3 indicator
    /// glyphs: loaded-now `A` / `B` badge, missing-file glyph,
    /// potential-duplicate link. The grid-disagreement glyph is
    /// reserved on the BPM column but always off in v1.0 — the
    /// auto-grid that would drive it is gated on the M11c
    /// `dub-bpm` follow-up that wires offline analysis into the
    /// importer.
    private var trackList: some View {
        Table(sortedTracks, selection: $selectedTrackId, sortOrder: $sortOrder) {
            TableColumn("") { track in
                rowIndicators(for: track)
            }
            .width(36)

            TableColumn("Title", value: \.titleSortKey) { track in
                titleCell(for: track)
            }
            .width(min: 180, ideal: 280)

            TableColumn("Artist", value: \.artistSortKey) { track in
                Text(track.artist ?? "—")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .width(min: 100, ideal: 160)

            TableColumn("Album", value: \.albumSortKey) { track in
                Text(track.album ?? "—")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .width(min: 100, ideal: 160)

            TableColumn("BPM", value: \.bpmSortKey) { track in
                Text(formatBpm(track.bpm))
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                    .monospacedDigit()
            }
            .width(60)

            TableColumn("Key") { track in
                Text(track.key ?? "—")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
            }
            .width(40)

            TableColumn("Length", value: \.durationSortKey) { track in
                Text(formatDuration(track.durationMs))
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                    .monospacedDigit()
            }
            .width(60)

            TableColumn("Year", value: \.yearSortKey) { track in
                Text(track.year.map { String($0) } ?? "—")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                    .monospacedDigit()
            }
            .width(50)

            TableColumn("Source") { track in
                Text(track.source.uppercased())
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
            }
            .width(70)
        }
        .onChange(of: selectedTrackId) { newId in
            if let trackId = newId {
                model.selectLibraryTrack(trackId)
            } else {
                model.selectedLibraryTrackId = nil
            }
        }
    }

    /// PRD §8.5.3 leftmost-gutter indicators. Order, top to
    /// bottom of visual priority:
    ///
    /// * `A` / `B` accent-tinted badge — track is loaded on
    ///   that deck right now. Two badges when both decks carry
    ///   the same track (Instant Doubles, §7.3).
    /// * Link glyph — `potentialDuplicateId` is non-nil; sibling-
    ///   version per §8.1 dedupe. Click navigates to the sibling.
    /// * Dim red exclamation — primary volume isn't reachable
    ///   per the model's `volumeReachability` cache; the track
    ///   is currently missing.
    ///
    /// Glyphs are deliberately small (10–11 pt) so they fit in
    /// the 36 pt gutter without crowding the title row.
    @ViewBuilder
    private func rowIndicators(for track: LibraryTrack) -> some View {
        HStack(spacing: 2) {
            if model.deckA.loadedLibraryTrackId == track.id {
                deckBadge("A", tint: DubColor.deckATint)
            }
            if model.deckB.loadedLibraryTrackId == track.id {
                deckBadge("B", tint: DubColor.deckBTint)
            }
            if track.potentialDuplicateId != nil {
                Button {
                    if let sibling = track.potentialDuplicateId {
                        navigateToSibling(sibling)
                    }
                } label: {
                    Image(systemName: "link")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(DubColor.textSecondary)
                }
                .buttonStyle(.plain)
                .help("Potential duplicate — click to jump to sibling.")
            }
            if model.libraryIsOpen && !model.isTrackReachable(track) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.red.opacity(0.65))
                    .help(missingFileTooltip(for: track))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func deckBadge(_ letter: String, tint: Color) -> some View {
        Text(letter)
            .font(.system(size: 9, weight: .bold, design: .rounded))
            .foregroundStyle(.white)
            .frame(width: 13, height: 13)
            .background(tint)
            .clipShape(RoundedRectangle(cornerRadius: 2))
            .help("Loaded on deck \(letter).")
    }

    private func missingFileTooltip(for track: LibraryTrack) -> String {
        if track.primaryVolumeMountPoint == nil {
            return "Source volume is not mounted."
        }
        return "Source volume is offline — plug it back in or use Relocate."
    }

    /// Highlight a track id in the visible list, scrolling it
    /// into view if necessary. Used by the sibling-version link
    /// glyph. When the sibling isn't currently visible (e.g.
    /// filtered out by an active search) the navigation no-ops
    /// gracefully — clearing the search would surface it but
    /// auto-clearing on click would be too aggressive.
    private func navigateToSibling(_ trackId: String) {
        guard tracks.contains(where: { $0.id == trackId }) else { return }
        selectedTrackId = trackId
    }

    /// Resolve a track's drag URL synchronously. Returns `nil`
    /// when the source volume is unmounted or the canonical row
    /// no longer resolves to an on-disk path. The Title column
    /// uses this to decide whether to install `.draggable` at
    /// all: unreachable rows are non-draggable rather than
    /// dragging a sentinel that the deck loader would only
    /// reject after a decode failure. Pre-fix this returned a
    /// `/dev/null` fallback for the modifier, which violated the
    /// "drop target rejects cleanly" contract because
    /// `/dev/null` is a real filesystem path that hands the
    /// audio decoder a zero-byte stream — the deck would flash
    /// a decoder error mid-load instead of silently doing
    /// nothing. The visible row already carries the missing-
    /// file glyph (see `rowIndicators`) and the Space-load path
    /// in `selectLibraryTrack` already refuses unreachable
    /// tracks, so disabling drag here is consistent with the
    /// rest of the unreachable-row affordances. The previous
    /// M10.5b AppKit drag path (`onDrag { NSItemProvider }`)
    /// only existed because SwiftUI's `.draggable` rendered an
    /// animation we didn't want; SwiftUI Table's row drag
    /// respects the cursor anchor, so the modern API is fine
    /// here.
    private func libraryDragURL(for track: LibraryTrack) -> URL? {
        guard let path = (try? model.library.trackPath(trackId: track.id)) ?? nil
        else { return nil }
        return URL(fileURLWithPath: path)
    }

    /// Title-column cell. Conditionally installs `.draggable`
    /// so unreachable tracks (`libraryDragURL` → `nil`) simply
    /// don't participate in drag-and-drop. The shared title /
    /// subtitle layout is the same on both branches; only the
    /// drag affordance differs. Extracted from the inline
    /// column body so the conditional doesn't fight the column
    /// builder's type inference.
    @ViewBuilder
    private func titleCell(for track: LibraryTrack) -> some View {
        let body = VStack(alignment: .leading, spacing: 1) {
            Text(displayTitle(track))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.middle)
            Text(displaySubtitle(track))
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        if let dragURL = libraryDragURL(for: track) {
            body.draggable(dragURL) {
                Text(displayTitle(track))
                    .font(DubFont.body)
                    .padding(4)
                    .background(DubColor.surface2)
            }
        } else {
            body
        }
    }

    private var footer: some View {
        HStack(spacing: DubSpacing.md) {
            if let summary = model.lastImportSummary {
                Text(importSummaryLine(summary))
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
            } else if model.libraryImportInProgress {
                Text("Importing…")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
            }
            if model.missingTrackCount > 0 {
                Button(action: { showRelocateSheet = true }) {
                    HStack(spacing: 4) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red.opacity(0.85))
                        Text(missingFooterLine(model.missingTrackCount))
                            .underline()
                    }
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
                }
                .buttonStyle(.plain)
                .help("Open the Relocate panel to point Dub at the directory that holds the missing files.")
            }
            Spacer(minLength: 0)
            Text("\(tracks.count) shown · \(model.libraryTrackCount) total")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.xs)
        .background(DubColor.surface1)
    }

    private func missingFooterLine(_ n: UInt64) -> String {
        let label = n == 1 ? "track missing" : "tracks missing"
        return "\(n) \(label) · Click to relocate"
    }

    // MARK: - Helpers

    private var emptyTitle: String {
        if !searchText.isEmpty {
            return "No matches for “\(searchText)”."
        }
        switch selectedSource {
        case .allTracks:
            return "Library is empty."
        case .recentlyPlayed:
            return "No play history yet."
        case .justImported:
            return "No imports this session."
        default:
            return "Not available in this build."
        }
    }

    private var emptySubtitle: String? {
        if !searchText.isEmpty {
            return "Try a shorter or different query."
        }
        switch selectedSource {
        case .allTracks:
            return "Use “Import Folder…” to add tracks."
        case .recentlyPlayed:
            return "Tracks you load on a deck show up here."
        case .justImported:
            return "Tracks imported this session show up here."
        default:
            return nil
        }
    }

    @ViewBuilder
    private func placeholderPane(title: String, subtitle: String?) -> some View {
        VStack(spacing: DubSpacing.sm) {
            Spacer()
            Text(title)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textTertiary)
            if let subtitle {
                Text(subtitle)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textPlaceholder)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func displayTitle(_ track: LibraryTrack) -> String {
        if let t = track.title, !t.isEmpty { return t }
        return "Untitled"
    }

    private func displaySubtitle(_ track: LibraryTrack) -> String {
        let artist = (track.artist?.isEmpty == false) ? track.artist! : "Unknown Artist"
        if let album = track.album, !album.isEmpty {
            return "\(artist) · \(album)"
        }
        return artist
    }

    private func formatBpm(_ bpm: Double?) -> String {
        guard let b = bpm, b > 0 else { return "—" }
        return String(format: "%.1f", b)
    }

    private func formatDuration(_ ms: UInt32) -> String {
        let totalSecs = Int(ms) / 1000
        let minutes = totalSecs / 60
        let seconds = totalSecs % 60
        return String(format: "%d:%02d", minutes, seconds)
    }

    private func importSummaryLine(_ summary: LibraryImportSummary) -> String {
        // Compact one-line summary; the full per-file `errors`
        // list lands in a v1.x detail sheet.
        var parts: [String] = []
        parts.append("\(summary.added) added")
        if summary.merged > 0 { parts.append("\(summary.merged) merged") }
        if summary.siblingVersions > 0 { parts.append("\(summary.siblingVersions) sibling") }
        if summary.refreshed > 0 { parts.append("\(summary.refreshed) refreshed") }
        if summary.skipped > 0 { parts.append("\(summary.skipped) skipped") }
        return "Last import: \(parts.joined(separator: ", "))"
    }

    // MARK: - Async refresh

    private func refreshTracks() {
        guard model.libraryIsOpen else {
            tracks = []
            selectedTrackId = nil
            return
        }
        // Switching source / search / library state invalidates the
        // visible selection — drop both the local Table selection
        // and the model-level browserSelection so a Space-load
        // doesn't fire on a row that's no longer in view.
        selectedTrackId = nil
        let source = selectedSource
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let limit = Self.listingLimit
        let library = model.library
        let since = model.appLaunchUnixSeconds

        isLoading = true
        Task.detached(priority: .userInitiated) {
            let rows: [LibraryTrack]
            do {
                if !query.isEmpty {
                    rows = try library.search(query: query, limit: limit)
                } else {
                    switch source {
                    case .allTracks:
                        rows = try library.listTracks(limit: limit, offset: 0)
                    case .recentlyPlayed:
                        rows = try library.recentlyPlayed(limit: limit)
                    case .justImported:
                        rows = try library.justImported(
                            sinceUnixSecs: since, limit: limit)
                    default:
                        rows = []
                    }
                }
            } catch {
                rows = []
            }
            await MainActor.run {
                self.tracks = rows
                self.isLoading = false
                // Recompute the per-volume reachability cache for
                // the volumes referenced by the new track set. One
                // syscall per unique mount point — cheap on a
                // typical 3 to 5 volume DJ rig.
                self.model.refreshVolumeReachability(for: rows)
            }
        }
    }

    // MARK: - NSOpenPanel (Import Folder…)

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

// MARK: - Relocate sheet

/// Modal sheet driving the M11d.4 Relocate workflow per PRD §8.5.5.
///
/// The user points Dub at a directory; the sheet walks the directory
/// via the FFI's `try_relocate_candidate`, which decodes each
/// candidate file, computes its Chromaprint fingerprint, and commits
/// a fresh `track_files` row for every missing track that matches
/// the candidate's `(fingerprint similarity ≥ 0.98, |duration| < 200 ms)`
/// pair. The original (missing) `track_files` row is left intact —
/// PRD §8.5.5 mandates that metadata is never deleted when a file
/// goes missing, so when the touring SSD comes back online the
/// previous path resurrects on its own.
private struct RelocateSheet: View {
    @ObservedObject var model: WaveformAppModel
    @Binding var isPresented: Bool
    @State private var lastRunSummary: (matched: UInt32, unmatched: UInt32)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            HStack(spacing: DubSpacing.sm) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red.opacity(0.85))
                Text("Relocate Missing Files")
                    .font(DubFont.title)
            }
            Text(headline)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            if let summary = lastRunSummary {
                summaryView(summary)
            }

            if model.relocateInProgress {
                HStack(spacing: DubSpacing.sm) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Scanning candidate folder…")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textSecondary)
                }
            }

            HStack {
                Button("Close") { isPresented = false }
                    .keyboardShortcut(.cancelAction)
                Spacer()
                Button(action: presentMatchFolderPicker) {
                    Text("Match Folder…")
                }
                .keyboardShortcut(.defaultAction)
                .disabled(model.relocateInProgress || model.missingTrackCount == 0)
            }
        }
        .padding(DubSpacing.xl)
        .frame(minWidth: 460, idealWidth: 520, maxWidth: 640)
        .background(DubColor.surface0)
    }

    private var headline: String {
        if model.missingTrackCount == 0 {
            return "All known files are reachable. Nothing to relocate right now."
        }
        let n = model.missingTrackCount
        let label = n == 1 ? "track is" : "tracks are"
        return "\(n) \(label) currently flagged as missing. Pick a folder Dub should search — files matching by fingerprint and duration will be reattached without disturbing the original library entries."
    }

    @ViewBuilder
    private func summaryView(_ summary: (matched: UInt32, unmatched: UInt32)) -> some View {
        if summary.matched == 0 && summary.unmatched == 0 {
            EmptyView()
        } else {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    Image(systemName: summary.matched > 0
                          ? "checkmark.circle.fill"
                          : "info.circle")
                        .foregroundStyle(summary.matched > 0 ? Color.green : DubColor.textSecondary)
                    Text(
                        summary.matched > 0
                            ? "Relocated \(summary.matched) of \(summary.matched + summary.unmatched) missing tracks."
                            : "No matches in the supplied folder."
                    )
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textPrimary)
                }
                if summary.unmatched > 0 {
                    Text("\(summary.unmatched) still missing. Try another folder or re-import.")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .padding(.leading, 20)
                }
            }
            .padding(.vertical, DubSpacing.xs)
        }
    }

    private func presentMatchFolderPicker() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Match"
        panel.message = "Choose the folder that now holds the relocated files."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { @MainActor in
            let result = await model.runRelocate(matchingFolder: url)
            lastRunSummary = result
        }
    }
}
