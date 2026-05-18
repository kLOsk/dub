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

/// Computed accessors for client-side column sort.
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
    /// M11d.5 comment column. Missing values fold to `""` so
    /// header-click sort puts unannotated tracks first, matching
    /// the title / artist sort behaviour.
    var commentSortKey:  String { comment ?? "" }
}

private enum TrackSortColumn: Hashable {
    case artist, title, bpm, comment
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

    /// `true` when the FFI returns rows in a meaningful natural
    /// order that a default column sort would destroy — Recently
    /// Played returns rows in descending `play_history.timestamp`
    /// order (most-recent first), Just Imported in descending
    /// `tracks.created_at` order. Applying the LibraryView's
    /// default title-ascending sort on top would clobber the
    /// reason the user opened the smart crate in the first place.
    /// `allTracks` returns rows ordered by `tracks.created_at`
    /// ASC, which is a stable but not user-meaningful order, so
    /// the default title sort still applies there.
    var preservesNaturalOrder: Bool {
        switch self {
        case .recentlyPlayed, .justImported: return true
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

    /// Notation mode for the Key column (M11c.2, PRD §8.3.2).
    /// Camelot is canonical; musical notation is opt-in via a
    /// click on the column header. `@AppStorage` makes the
    /// choice persist across app launches per PRD §9 ("settings
    /// the user has set stick").
    @AppStorage("libraryKeyNotationMode") private var keyNotationMode: KeyNotationMode = .camelot

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

    /// Client-side sort column + direction. Drives `sortOrder`
    /// which feeds `sortedTracks`. Empty `activeSortColumn` preserves
    /// the FFI's natural order (Recently Played / Just Imported).
    @State private var activeSortColumn: TrackSortColumn? = .title
    @State private var sortAscending: Bool = true
    @State private var sortOrder: [KeyPathComparator<LibraryTrack>] = [
        KeyPathComparator(\.titleSortKey, order: .forward),
    ]

    /// Currently selected row id (canonical UUID). Kept in sync with
    /// `model.selectedLibraryTrackId` via `onChange`.
    @State private var selectedTrackId: LibraryTrack.ID? = nil

    /// Drives a minimal keyboard scroll — set only by ↑/↓, never
    /// on mouse click (centering the selection was the huge header
    /// gap in the screenshot).
    @State private var keyboardScrollTarget: LibraryTrack.ID?
    @State private var keyboardScrollDelta: Int = 0

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
        .onChange(of: selectedSource) { newSource in
            // Smart crates with a meaningful natural order
            // (Recently Played, Just Imported) start out
            // *unsorted* so the FFI's recency order survives.
            // `allTracks` falls back to title-ascending as
            // before. The user can still click a column header
            // afterwards to override either default — that's
            // what the comparator binding is for.
            sortOrder = newSource.preservesNaturalOrder
                ? []
                : [KeyPathComparator(\LibraryTrack.titleSortKey, order: .forward)]
            if newSource.preservesNaturalOrder {
                activeSortColumn = nil
            } else {
                activeSortColumn = .title
                sortAscending = true
            }
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
        .onChange(of: model.analysisGeneration) { _ in
            // M11c.1 — a deck-load or batch analyze finished and
            // wrote at least one new grid. Re-fetch the current
            // listing so the BPM column lights up and the dim
            // overlay drops on the rows that just transitioned.
            // Preserve selection — the rows haven't moved, the
            // user shouldn't lose their Space-load target.
            refreshTracks(preserveSelection: true)
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
        // Empty comparator → preserve the FFI's natural order
        // (Recently Played / Just Imported). `sorted(using: [])`
        // would still be a stable no-op but constructing the
        // sorted copy is wasted work on every render.
        guard !sortOrder.isEmpty else { return tracks }
        return tracks.sorted(using: sortOrder)
    }

    /// Scrollable track list. Uses the same row pattern as
    /// `FileBrowserView` (full-row `onTapGesture` + AppKit
    /// `onDrag`) because SwiftUI `Table` was dropping ~4/5
    /// clicks and turning drags outside the Title column into
    /// arrow-key-style selection changes.
    private var trackList: some View {
        VStack(spacing: 0) {
            trackListHeader
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(sortedTracks) { track in
                            trackRow(for: track)
                                .id(track.id)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
                .onChange(of: keyboardScrollTarget) { targetId in
                    guard let targetId else { return }
                    let anchor = keyboardScrollDelta > 0
                        ? UnitPoint(x: 0.5, y: 0.92)
                        : UnitPoint(x: 0.5, y: 0.08)
                    proxy.scrollTo(targetId, anchor: anchor)
                    keyboardScrollTarget = nil
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(
            LibraryArrowKeyView(
                selectedTrackId: $selectedTrackId,
                trackIds: sortedTracks.map(\.id),
                onArrowNavigate: { trackId, delta in
                    keyboardScrollDelta = delta
                    keyboardScrollTarget = trackId
                })
        )
        .onChange(of: selectedTrackId) { newId in
            if let trackId = newId {
                let snapshot = tracks.first(where: { $0.id == trackId })
                model.selectLibraryTrack(trackId, snapshot: snapshot)
            } else {
                model.selectedLibraryTrackId = nil
                model.selectedLibraryTrack = nil
            }
        }
    }

    private var trackListHeader: some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: 36)
            sortHeader("Artist", column: .artist)
                .frame(minWidth: 120, maxWidth: .infinity, alignment: .leading)
            sortHeader("Title", column: .title)
                .frame(minWidth: 180, maxWidth: .infinity, alignment: .leading)
            sortHeader("BPM", column: .bpm)
                .frame(width: 60, alignment: .leading)
            sortHeader("Comment", column: .comment)
                .frame(minWidth: 140, maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, 2)
        .frame(height: 22)
        .background(DubColor.surface1)
        .overlay(alignment: .bottom) {
            Rectangle().fill(DubColor.divider).frame(height: 1)
        }
    }

    private func sortHeader(_ label: String, column: TrackSortColumn) -> some View {
        let isActive = activeSortColumn == column
        return Button {
            toggleSort(column)
        } label: {
            HStack(spacing: 3) {
                Text(label.uppercased())
                    .font(DubFont.micro.weight(.semibold))
                    .foregroundStyle(isActive ? DubColor.textPrimary : DubColor.textSecondary)
                if isActive {
                    Image(systemName: sortAscending ? "chevron.up" : "chevron.down")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(DubColor.textSecondary)
                }
            }
        }
        .buttonStyle(.plain)
    }

    private func toggleSort(_ column: TrackSortColumn) {
        if activeSortColumn == column {
            if sortAscending {
                sortAscending = false
            } else {
                activeSortColumn = nil
                sortAscending = true
                sortOrder = []
            }
        } else {
            activeSortColumn = column
            sortAscending = true
        }
        if activeSortColumn != nil {
            syncSortOrderFromHeader()
        }
    }

    private func syncSortOrderFromHeader() {
        guard let column = activeSortColumn else {
            sortOrder = []
            return
        }
        let order: SortOrder = sortAscending ? .forward : .reverse
        switch column {
        case .artist:
            sortOrder = [KeyPathComparator(\.artistSortKey, order: order)]
        case .title:
            sortOrder = [KeyPathComparator(\.titleSortKey, order: order)]
        case .bpm:
            sortOrder = [KeyPathComparator(\.bpmSortKey, order: order)]
        case .comment:
            sortOrder = [KeyPathComparator(\.commentSortKey, order: order)]
        }
    }

    @ViewBuilder
    private func trackRow(for track: LibraryTrack) -> some View {
        let isSelected = selectedTrackId == track.id
        let dragURL = libraryDragURL(for: track)
        HStack(spacing: 0) {
            rowIndicators(for: track)
                .frame(width: 36, alignment: .leading)
            Text(track.artist ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .modifier(DimUnanalyzed(track: track))
                .frame(minWidth: 120, maxWidth: .infinity, alignment: .leading)
            Text(displayTitle(track))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.middle)
                .modifier(DimUnanalyzed(track: track))
                .frame(minWidth: 180, maxWidth: .infinity, alignment: .leading)
            Text(formatBpm(track.bpm))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .monospacedDigit()
                .modifier(DimUnanalyzed(track: track))
                .frame(width: 60, alignment: .leading)
            Text(track.comment ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .help(track.comment ?? "")
                .modifier(DimUnanalyzed(track: track))
                .frame(minWidth: 140, maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.xs)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? DubColor.surface2 : Color.clear)
        .contentShape(Rectangle())
        .if(dragURL != nil) { view in
            view.onDrag {
                NSItemProvider(object: dragURL! as NSURL)
            }
        }
        .contextMenu {
            Button("Analyze") {
                Task { @MainActor in
                    await model.analyzeTracks([track.id], forceReanalyze: false)
                }
            }
            .disabled(model.analysisBatchTotal > 0)
            Button("Re-analyze") {
                Task { @MainActor in
                    await model.analyzeTracks([track.id], forceReanalyze: true)
                }
            }
            .disabled(model.analysisBatchTotal > 0)
        }
        .onTapGesture {
            NSApp.keyWindow?.makeFirstResponder(nil)
            selectedTrackId = track.id
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
        NSApp.keyWindow?.makeFirstResponder(nil)
        selectedTrackId = trackId
    }

    /// Resolve a track's drag URL synchronously. Returns `nil`
    /// when the source volume is unmounted or the canonical row
    /// no longer resolves to an on-disk path. Every table cell
    /// uses this to decide whether to install `.onDrag` at all:
    /// unreachable rows are non-draggable rather than dragging a
    /// sentinel that the deck loader would only reject after a
    /// decode failure. Pre-fix this returned a `/dev/null` fallback
    /// for the modifier, which violated the "drop target rejects
    /// cleanly" contract because `/dev/null` is a real filesystem
    /// path that hands the audio decoder a zero-byte stream — the
    /// deck would flash a decoder error mid-load instead of silently
    /// doing nothing. The visible row already carries the missing-
    /// file glyph (see `rowIndicators`) and the Space-load path
    /// in `selectLibraryTrack` already refuses unreachable tracks,
    /// so disabling drag here is consistent with the rest of the
    /// unreachable-row affordances.
    ///
    /// Drag uses the AppKit `onDrag { NSItemProvider }` path on
    /// every column (see `libraryRowCell`) so the OS snapshots the
    /// row under the cursor. SwiftUI's `.draggable(_:preview:)`
    /// rendered the preview at the row's layout position first and
    /// then animated it toward the cursor — the "fly-in from row"
    /// effect the user reported on library → deck drops.
    ///
    /// Pre-fix this called `library.trackPath(trackId:)` per
    /// row per render — a SQLite round-trip executed thousands
    /// of times when scrolling a 5 000 track listing. The fields
    /// we need (`primaryVolumeMountPoint`, `primaryRelativePath`)
    /// are already in the `LibraryTrack` row from the FFI's
    /// `TRACK_ROW_SELECT`, so reconstruct the URL locally and
    /// keep the FFI for paths the row doesn't carry (e.g. the
    /// `selectLibraryTrack` resolve-on-click guard, where we
    /// also want the FFI's "volume unmounted right now" check).
    private func libraryDragURL(for track: LibraryTrack) -> URL? {
        guard let mount = track.primaryVolumeMountPoint, !mount.isEmpty,
              let rel   = track.primaryRelativePath,    !rel.isEmpty
        else { return nil }
        // Mirror `Library::resolve_track_path` — concatenate
        // mount + relative without re-running the FFI. The
        // unmounted-volume case falls out naturally: an
        // unmounted volume publishes a nil mount point in the
        // row, so we return nil here without touching SQLite.
        let base = URL(fileURLWithPath: mount, isDirectory: true)
        return base.appendingPathComponent(rel)
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
            // M11c.1 — batch-analyze progress. Shown only while a
            // batch is in flight (single deck-load analyses bump
            // `analysisInFlightCount` without setting
            // `analysisBatchTotal`, so they don't crowd the
            // footer with one-second blips).
            if model.analysisBatchTotal > 0 {
                HStack(spacing: 4) {
                    ProgressView()
                        .scaleEffect(0.5)
                        .frame(width: 12, height: 12)
                    Text(analyzeProgressLine(
                        completed: model.analysisBatchCompleted,
                        total: model.analysisBatchTotal))
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textSecondary)
                }
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

    /// "Analyzing N of M…" — N is the 1 based index of the track
    /// currently being processed, M is the batch total. The
    /// model publishes `analysisBatchCompleted` as the count of
    /// tracks already *finished* (success or failure), so the
    /// active track is `completed + 1`, clamped to `total` so we
    /// never render "Analyzing 11 of 10…" on the boundary tick
    /// between the last per-track completion and the deferred
    /// batch cleanup.
    private func analyzeProgressLine(completed: UInt32, total: UInt32) -> String {
        guard total > 0 else { return "Analyzing…" }
        let active = min(completed + 1, total)
        return "Analyzing \(active) of \(total)…"
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

    private func refreshTracks(preserveSelection: Bool = false) {
        guard model.libraryIsOpen else {
            tracks = []
            selectedTrackId = nil
            return
        }
        // Switching source / search / library state invalidates the
        // visible selection — drop both the local Table selection
        // and the model-level browserSelection so a Space-load
        // doesn't fire on a row that's no longer in view.
        //
        // M11c.1 — when the refresh is triggered by an analysis-
        // completion bump (BPM landed on a row), the selection
        // *should* persist; the rows haven't moved. The caller
        // opts into that via `preserveSelection: true`.
        if !preserveSelection {
            selectedTrackId = nil
        }
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

// MARK: - DimUnanalyzed modifier

/// PRD §8.5.3 visual cue for the M11c.1 lazy-analysis lifecycle.
///
/// Tracks that have never been analyzed (no `analysis_cache` row
/// stamped yet) render at reduced opacity. Once auto-analysis
/// completes (whether or not a grid was found) the row flips
/// back to full opacity. Imported grids from a future Serato /
/// Traktor / rekordbox milestone also drop the dim because the
/// importer stamps `analysis_cache` as it lands the grid.
///
/// Implemented as a `ViewModifier` so the same opacity rule
/// applies uniformly to every column in the LibraryView table
/// without duplicating the `.opacity(...)` call across cells.
/// Notation mode for the LibraryView Key column. Persisted across
/// app launches via `@AppStorage("libraryKeyNotationMode")`.
/// Canonical Camelot is the default — it's the dominant
/// scratch-DJ convention (MixedInKey + Serato display) and the
/// notation Dub stores internally (`track_keys.key_notation`).
/// Musical notation (e.g. `C major`) is opt-in for users who
/// learned theory before they learned the wheel.
enum KeyNotationMode: String, CaseIterable, Identifiable {
    case camelot
    case musical

    var id: String { rawValue }

    /// Display name for the column header.
    var columnLabel: String {
        switch self {
        case .camelot: return "Key"
        case .musical: return "Key (♪)"
        }
    }

    /// Toggle helper — keeps the click handler's mutation logic
    /// out of the cell body so the cell stays a pure function of
    /// `(track, mode)`.
    var toggled: KeyNotationMode {
        switch self {
        case .camelot: return .musical
        case .musical: return .camelot
        }
    }
}

/// Render a Camelot key as either Camelot (default) or musical
/// notation (opt-in). Returns `"—"` for `nil` keys; the visual
/// "we have no key" cue is the em-dash, not an empty cell.
private extension LibraryView {
    var keyColumnHeader: String { keyNotationMode.columnLabel }

    func renderKey(_ camelot: String?) -> String {
        guard let camelot, !camelot.isEmpty else { return "—" }
        switch keyNotationMode {
        case .camelot: return camelot
        case .musical: return musicalFromCamelot(camelot) ?? camelot
        }
    }

    /// Tooltip = the *other* notation, so the user always sees
    /// both at a glance without re-clicking. Empty for nil keys.
    func keyTooltip(_ camelot: String?) -> String {
        guard let camelot, !camelot.isEmpty else { return "" }
        switch keyNotationMode {
        case .camelot: return musicalFromCamelot(camelot) ?? ""
        case .musical: return camelot
        }
    }

    /// Convert a canonical Camelot string (e.g. `"8B"`) to its
    /// musical equivalent (e.g. `"C major"`). Returns `nil` for
    /// malformed inputs; the renderer falls back to the raw
    /// Camelot string in that case.
    func musicalFromCamelot(_ camelot: String) -> String? {
        // Camelot → (pitch class, is_major).
        // Same wheel layout as `dub-spectral::key::CAMELOT_MAJOR`
        // and `CAMELOT_MINOR`. Kept here as a static lookup
        // because pushing this across the FFI for every row
        // render would be silly — the table is 24 entries.
        let table: [String: String] = [
            "8B": "C major", "3B": "C♯ major", "10B": "D major",
            "5B": "D♯ major", "12B": "E major", "7B": "F major",
            "2B": "F♯ major", "9B": "G major", "4B": "G♯ major",
            "11B": "A major", "6B": "A♯ major", "1B": "B major",
            "5A": "C minor", "12A": "C♯ minor", "7A": "D minor",
            "2A": "D♯ minor", "9A": "E minor", "4A": "F minor",
            "11A": "F♯ minor", "6A": "G minor", "1A": "G♯ minor",
            "8A": "A minor", "3A": "A♯ minor", "10A": "B minor",
        ]
        return table[camelot.uppercased()]
    }
}

private struct DimUnanalyzed: ViewModifier {
    let track: LibraryTrack
    func body(content: Content) -> some View {
        // 0.55 chosen to read as "this row is waiting for
        // analysis" without losing legibility of the title /
        // artist text. Slightly higher than the 0.40 we use for
        // disabled controls; rows are still selectable +
        // draggable, just visually deferred.
        content.opacity(track.isAnalyzed ? 1.0 : 0.55)
    }
}

// MARK: - Conditional view modifier

private extension View {
    @ViewBuilder
    func `if`<Transform: View>(
        _ condition: Bool,
        transform: (Self) -> Transform
    ) -> some View {
        if condition {
            transform(self)
        } else {
            self
        }
    }
}

// MARK: - Library arrow-key navigation

/// Local ↑/↓ handler for the library list.
private struct LibraryArrowKeyView: NSViewRepresentable {
    @Binding var selectedTrackId: LibraryTrack.ID?
    let trackIds: [LibraryTrack.ID]
    var onArrowNavigate: ((LibraryTrack.ID, Int) -> Void)?

    func makeCoordinator() -> Coordinator {
        Coordinator(
            selectedTrackId: $selectedTrackId,
            onArrowNavigate: onArrowNavigate)
    }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        context.coordinator.install()
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.trackIds = trackIds
        context.coordinator.onArrowNavigate = onArrowNavigate
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.uninstall()
    }

    @MainActor
    final class Coordinator {
        @Binding var selectedTrackId: LibraryTrack.ID?
        var trackIds: [LibraryTrack.ID] = []
        var onArrowNavigate: ((LibraryTrack.ID, Int) -> Void)?
        private var monitor: Any?

        init(
            selectedTrackId: Binding<LibraryTrack.ID?>,
            onArrowNavigate: ((LibraryTrack.ID, Int) -> Void)?
        ) {
            _selectedTrackId = selectedTrackId
            self.onArrowNavigate = onArrowNavigate
        }

        func install() {
            uninstall()
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) {
                [weak self] event in
                guard let self else { return event }
                guard !Self.isTextFirstResponder() else { return event }
                guard !event.modifierFlags.contains(.command) else { return event }
                let delta: Int?
                switch event.keyCode {
                case 126: delta = -1   // ↑
                case 125: delta = 1    // ↓
                default: delta = nil
                }
                guard let delta, self.moveSelection(by: delta) else { return event }
                return nil
            }
        }

        func uninstall() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }

        private func moveSelection(by delta: Int) -> Bool {
            guard !trackIds.isEmpty else { return false }
            let currentIdx: Int
            if let id = selectedTrackId, let idx = trackIds.firstIndex(of: id) {
                currentIdx = idx
            } else {
                let firstId = trackIds[0]
                NSApp.keyWindow?.makeFirstResponder(nil)
                selectedTrackId = firstId
                onArrowNavigate?(firstId, delta)
                return true
            }
            let next = max(0, min(trackIds.count - 1, currentIdx + delta))
            guard next != currentIdx else { return false }
            let nextId = trackIds[next]
            NSApp.keyWindow?.makeFirstResponder(nil)
            selectedTrackId = nextId
            onArrowNavigate?(nextId, delta)
            return true
        }

        private static func isTextFirstResponder() -> Bool {
            guard let responder = NSApp.keyWindow?.firstResponder else {
                return false
            }
            if responder is NSText || responder is NSTextView { return true }
            // SwiftUI `TextField` hosts an `NSTextField` — while the
            // user is editing search, ↑/↓ should move the caret, not
            // the library selection.
            if let field = responder as? NSTextField, field.isEditable {
                return true
            }
            return false
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
