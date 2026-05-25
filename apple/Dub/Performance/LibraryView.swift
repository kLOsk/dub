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
    var genreSortKey:    String { genre ?? "" }
    var sourceSortKey:   String { source }
    /// BPM sort: missing values pinned past every real BPM in
    /// either direction so they collect at one end of the table
    /// rather than punching holes through the middle. `Double`
    /// `.infinity` is the canonical "biggest plausible value"
    /// sentinel.
    var bpmSortKey:      Double { bpm ?? .infinity }
    /// Unknown imported duration is marshalled as `0` until lazy
    /// analysis measures the file. Sort unknowns after real tracks.
    var durationSortKey: UInt32 { durationMs == 0 ? UInt32.max : durationMs }
    var yearSortKey:     Int32  { year ?? Int32.max }
    var keySortKey:      String { key ?? "" }
    /// M11d.5 comment column. Missing values fold to `""` so
    /// header-click sort puts unannotated tracks first, matching
    /// the title / artist sort behaviour.
    var commentSortKey:  String { comment ?? "" }
    var versionTokensSortKey: String { versionTokens ?? "" }
    var composerSortKey: String { composer ?? "" }
    var trackNumberSortKey: Int32 { trackNumber ?? Int32.max }
}

// MARK: - Configurable library columns (PRD §8.5.3.1 lite)

/// Display + sort identity for a library browser column. Artist
/// and title are always shown; the trailing set is user-configurable
/// via header right-click and persisted in `@AppStorage`.
private enum LibraryColumnField: String, CaseIterable, Identifiable {
    case artist
    case title
    case duration
    case bpm
    case album
    case genre
    case year
    case key
    case comment
    case composer
    case trackNumber
    case versionTokens
    case source

    var id: String { rawValue }

    /// Always pinned left; not toggleable from the column picker.
    var isFixed: Bool {
        self == .artist || self == .title
    }

    static let fixedPrefix: [LibraryColumnField] = [.artist, .title]

    /// Default trailing columns: Length before BPM (user request).
    static let defaultTrailing: [LibraryColumnField] = [.duration, .bpm, .comment]

    /// Columns the user can show/hide via header right-click.
    static let configurable: [LibraryColumnField] = [
        .duration, .bpm, .album, .genre, .year, .key,
        .comment, .composer, .trackNumber, .versionTokens, .source,
    ]

    var pickerCategory: String {
        switch self {
        case .artist, .title, .source, .versionTokens:
            return "Library"
        case .album, .genre, .year, .comment, .composer, .trackNumber:
            return "ID3 metadata"
        case .duration, .bpm, .key:
            return "Analysis"
        }
    }

    var headerLabel: String {
        switch self {
        case .artist: return "Artist"
        case .title: return "Title"
        case .duration: return "Length"
        case .bpm: return "BPM"
        case .album: return "Album"
        case .genre: return "Genre"
        case .year: return "Year"
        case .key: return "Key"
        case .comment: return "Comment"
        case .composer: return "Composer"
        case .trackNumber: return "Track #"
        case .versionTokens: return "Version"
        case .source: return "Source"
        }
    }
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

    /// Visible column order (comma-separated raw values). Includes
    /// Artist + Title; user-reorderable via header drag (PRD §8.5.3.1).
    @AppStorage("libraryVisibleColumns") private var visibleColumnsStorage: String =
        "artist,title,duration,bpm,comment"

    /// Per-column widths keyed by `LibraryColumnField.rawValue` JSON.
    @AppStorage("libraryColumnWidths") private var columnWidthsStorage: String = ""

    /// In-progress resize width for one column. Header and row cells
    /// both use this preview so the table tracks the resize live.
    @State private var columnResizePreview: (field: LibraryColumnField, width: CGFloat)?

    /// Screen-space drag origin. Lives on `LibraryView`, not on the
    /// resize handle, so it survives header `NSHostingView` rebuilds
    /// while dragging. Width is derived from this anchor plus the
    /// current global X — not from `DragGesture` translation in the
    /// handle's local space (the handle moves when the column grows,
    /// which was causing ~1/3 travel and left-right jitter).
    @State private var columnResizeDragOrigin: (
        field: LibraryColumnField,
        width: CGFloat,
        globalX: CGFloat
    )?

    /// Header drag-to-reorder (PRD §8.5.3.1). Global frames come from
    /// `ColumnHeaderFramesKey` so hit-testing survives horizontal scroll.
    @State private var columnReorderDrag: LibraryColumnField?
    @State private var columnReorderDropTarget: LibraryColumnField?
    @State private var columnReorderInsertBefore: Bool = true
    /// Computed drop result while dragging. Rows do not use this
    /// until mouse-up, which keeps column reordering cheap even for
    /// large `LazyVStack` listings.
    @State private var columnReorderPendingOrder: [LibraryColumnField]?
    @State private var columnHeaderFrames: [LibraryColumnField: CGRect] = [:]

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
    @State private var activeSortColumn: LibraryColumnField? = .title
    @State private var sortAscending: Bool = true
    @State private var sortOrder: [KeyPathComparator<LibraryTrack>] = [
        KeyPathComparator(\.titleSortKey, order: .forward),
    ]

    /// Currently selected row ids (canonical UUIDs). Cmd+click toggles
    /// membership; Shift+click selects a contiguous range in the
    /// current sort order. The primary id drives Space-load.
    @State private var selectedTrackIds: Set<String> = []

    /// Anchor for Shift+click range selection in the current sort order.
    @State private var selectionAnchorId: String? = nil

    /// Primary selected row for Space-load + model sync.
    private var primarySelectedTrackId: String? {
        if let anchor = selectionAnchorId, selectedTrackIds.contains(anchor) {
            return anchor
        }
        return selectedTrackIds.sorted().first
    }

    /// Drives a minimal keyboard scroll — set only by ↑/↓, never
    /// on mouse click (centering the selection was the huge header
    /// gap in the screenshot).
    @State private var keyboardScrollTarget: LibraryTrack.ID?
    @State private var keyboardScrollDelta: Int = 0

    /// Bumped when in-memory row fields change (analysis patch,
    /// etc.) so the AppKit table body refreshes even though the
    /// track-id ordering is unchanged.
    @State private var tracksContentRevision: UInt64 = 0

    @FocusState private var searchFocused: Bool

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
        .onChange(of: model.libraryRowAnalysisUpdate) { update in
            guard let update else { return }
            applyAnalysisUpdate(update)
        }
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            handleLibraryDrop(providers)
        }
        .background {
            LibraryTextFocusDismissMonitor()
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
            searchFocused = false
            NSApp.keyWindow?.makeFirstResponder(nil)
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
        .contentShape(Rectangle())
        .onTapGesture {
            searchFocused = false
            NSApp.keyWindow?.makeFirstResponder(nil)
        }
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
                    .focused($searchFocused)
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
            LibraryTableScrollContainer(
                tableWidth: tableContentWidth,
                columnOrderKey: visibleColumns.map(\.rawValue).joined(separator: ","),
                headerStateKey: columnReorderHeaderStateKey,
                tracksContentRevision: tracksContentRevision,
                selectedTrackIds: selectedTrackIds,
                header: AnyView(trackListHeader),
                rows: AnyView(trackRowsStack),
                trackIds: sortedTracks.map(\.id),
                visibleTracks: sortedTracks,
                menu: LibraryTableMenu(
                    analysisBatchInProgress: model.analysisBatchTotal > 0,
                    onAnalyzeRequested: { ids in
                        Task { @MainActor in
                            await model.analyzeTracks(ids)
                        }
                    },
                    onSetGridLocked: { trackId, locked in
                        Task { @MainActor in
                            await model.setGridLocked(trackId: trackId, locked: locked)
                        }
                    }),
                scroll: LibraryTableScroll(
                    scrollToTrackId: keyboardScrollTarget,
                    scrollDelta: keyboardScrollDelta,
                    onScrollHandled: { keyboardScrollTarget = nil })
            )
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(
            LibraryArrowKeyView(
                selectedTrackIds: $selectedTrackIds,
                selectionAnchorId: $selectionAnchorId,
                trackIds: sortedTracks.map(\.id),
                onArrowNavigate: { trackId, delta in
                    keyboardScrollDelta = delta
                    keyboardScrollTarget = trackId
                },
                onSelectionChanged: syncModelPrimarySelection)
            .allowsHitTesting(false)
        )
        .onChange(of: selectedTrackIds) { _ in
            // Defer the FFI path lookup so row highlight paints on
            // the same frame as the click instead of waiting for
            // `library.trackPath` on the main thread first.
            Task { @MainActor in
                syncModelPrimarySelection()
            }
        }
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            handleLibraryDrop(providers)
        }
    }

    private var trackRowsStack: some View {
        LazyVStack(spacing: 0) {
            ForEach(sortedTracks) { track in
                trackRow(for: track)
                    .id(track.id)
            }
        }
    }

    /// Changes when the header-only reorder affordance changes.
    /// The scroll container uses this to refresh only the sticky
    /// header while a column is being dragged, leaving the row host
    /// alone until the drop commits.
    private var columnReorderHeaderStateKey: String {
        [
            columnReorderDrag?.rawValue ?? "",
            columnReorderDropTarget?.rawValue ?? "",
            columnReorderInsertBefore ? "before" : "after",
        ].joined(separator: "|")
    }

    /// Gutter + columns + horizontal padding. Header and rows share
    /// this width, including the in-progress resize preview.
    private var tableContentWidth: CGFloat {
        let columnSum = visibleColumns.map { columnWidth($0) }.reduce(0, +)
        return 36 + columnSum + DubSpacing.lg * 2
    }

    private var trackListHeader: some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: 36)
                .overlay(alignment: .trailing) {
                    columnHeaderDivider
                }
            ForEach(visibleColumns) { field in
                resizableColumnHeader(for: field)
            }
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, 2)
        .frame(height: 22)
        .frame(width: tableContentWidth, alignment: .leading)
        .background(DubColor.surface1)
        .contentShape(Rectangle())
        .contextMenu {
            libraryColumnContextMenu()
        }
        .overlay(alignment: .bottom) {
            Rectangle().fill(DubColor.divider).frame(height: 1)
        }
        .onTapGesture {
            searchFocused = false
            NSApp.keyWindow?.makeFirstResponder(nil)
        }
        .onPreferenceChange(ColumnHeaderFramesKey.self) { columnHeaderFrames = $0 }
    }

    private func resizableColumnHeader(for field: LibraryColumnField) -> some View {
        let width = columnWidth(field)
        let isReorderSource = columnReorderDrag == field
        let isReorderTarget = columnReorderDropTarget == field
            && columnReorderDrag != nil
            && columnReorderDrag != field
        return ZStack(alignment: .leading) {
            sortHeaderContent(for: field)
                .padding(.leading, LibraryColumnLayout.columnLeadingInset)
                .frame(
                    width: max(0, width - LibraryColumnLayout.resizeHandleTotalWidth),
                    alignment: .leading
                )
                .padding(.trailing, LibraryColumnLayout.resizeHandleTotalWidth)
                .simultaneousGesture(columnReorderGesture(for: field))
            HStack(spacing: 0) {
                Spacer(minLength: 0)
                LibraryColumnResizeHandle(
                    onDragChanged: { startX, locationX in
                        columnResizeLive(
                            field: field,
                            dragStartGlobalX: startX,
                            locationGlobalX: locationX)
                    },
                    onDragEnded: { startX, locationX in
                        endColumnResize(
                            field: field,
                            dragStartGlobalX: startX,
                            locationGlobalX: locationX)
                    }
                )
            }
        }
        .frame(width: width, alignment: .leading)
        .frame(maxHeight: .infinity)
        .opacity(isReorderSource ? 0.72 : 1)
        .overlay {
            RoundedRectangle(cornerRadius: 3)
                .stroke(
                    isReorderSource
                        ? DubColor.deckATint
                        : .clear,
                    lineWidth: 2
                )
                .padding(1)
        }
        .background(
            isReorderSource
                ? DubColor.deckATint.opacity(0.10)
                : Color.clear
        )
        .overlay(alignment: .trailing) {
            columnHeaderDivider
        }
        .overlay(alignment: columnReorderInsertBefore ? .leading : .trailing) {
            if isReorderTarget {
                Rectangle()
                    .fill(DubColor.deckATint)
                    .frame(width: 3)
                    .shadow(color: DubColor.deckATint.opacity(0.5), radius: 2)
            }
        }
        .background(
            GeometryReader { proxy in
                Color.clear.preference(
                    key: ColumnHeaderFramesKey.self,
                    value: [field: proxy.frame(in: .global)]
                )
            }
        )
        .contentShape(Rectangle())
        .contextMenu {
            libraryColumnContextMenu()
        }
    }

    /// User-ordered visible columns. Artist + Title are always present
    /// but may be swapped relative to each other (PRD §8.5.3.1).
    private var visibleColumns: [LibraryColumnField] {
        let raw = visibleColumnsStorage
            .split(separator: ",")
            .map { String($0).trimmingCharacters(in: .whitespaces) }
        var parsed = raw.compactMap { LibraryColumnField(rawValue: $0) }
        if parsed.isEmpty {
            parsed = LibraryColumnField.fixedPrefix + LibraryColumnField.defaultTrailing
        }
        // Migrate older trailing-only storage (`duration,bpm,…`).
        if !parsed.contains(.artist) || !parsed.contains(.title) {
            parsed = LibraryColumnField.fixedPrefix + parsed.filter { !$0.isFixed }
        }
        var seen = Set<String>()
        parsed = parsed.filter { seen.insert($0.rawValue).inserted }
        for fixed in LibraryColumnField.fixedPrefix where !parsed.contains(fixed) {
            parsed.insert(fixed, at: 0)
        }
        return parsed
    }

    private func persistColumnOrder(_ columns: [LibraryColumnField]) {
        var cols = columns
        for fixed in LibraryColumnField.fixedPrefix where !cols.contains(fixed) {
            cols.insert(fixed, at: 0)
        }
        visibleColumnsStorage = cols.map(\.rawValue).joined(separator: ",")
    }

    private func setColumnVisibility(_ field: LibraryColumnField, visible: Bool) {
        guard !field.isFixed else { return }
        var cols = visibleColumns
        let isVisible = cols.contains(field)
        if visible {
            guard !isVisible else { return }
            cols.append(field)
            persistColumnOrder(cols)
        } else {
            let removable = cols.filter { !$0.isFixed }
            guard isVisible, removable.count > 1 else { return }
            cols.removeAll { $0 == field }
            if activeSortColumn == field {
                activeSortColumn = nil
                sortOrder = []
            }
            persistColumnOrder(cols)
        }
    }

    private func columnVisibilityBinding(_ field: LibraryColumnField) -> Binding<Bool> {
        Binding(
            get: { visibleColumns.contains(field) },
            set: { setColumnVisibility(field, visible: $0) }
        )
    }

    private func columnReorderGesture(for field: LibraryColumnField) -> some Gesture {
        DragGesture(minimumDistance: 8, coordinateSpace: .global)
            .onChanged { value in
                guard columnResizeDragOrigin == nil else { return }
                if columnReorderDrag == nil {
                    columnReorderDrag = field
                    columnReorderPendingOrder = visibleColumns
                }
                guard let source = columnReorderDrag else { return }
                if let hit = columnHit(atGlobalX: value.location.x) {
                    var transaction = Transaction()
                    transaction.disablesAnimations = true
                    withTransaction(transaction) {
                        columnReorderDropTarget = hit.field
                        columnReorderInsertBefore = hit.insertBefore
                        columnReorderPendingOrder = previewColumnOrder(
                            moving: source,
                            over: hit.field,
                            insertBefore: hit.insertBefore
                        )
                    }
                }
            }
            .onEnded { _ in
                defer {
                    columnReorderDrag = nil
                    columnReorderDropTarget = nil
                    columnReorderPendingOrder = nil
                }
                guard columnResizeDragOrigin == nil,
                      let order = columnReorderPendingOrder,
                      order != visibleColumns
                else { return }
                persistColumnOrder(order)
            }
    }

    private func columnHit(atGlobalX x: CGFloat) -> (
        field: LibraryColumnField,
        insertBefore: Bool
    )? {
        for (field, frame) in columnHeaderFrames {
            guard frame.minX <= x, x <= frame.maxX else { continue }
            return (field, x < frame.midX)
        }
        return nil
    }

    private func previewColumnOrder(
        moving source: LibraryColumnField,
        over target: LibraryColumnField,
        insertBefore: Bool
    ) -> [LibraryColumnField] {
        var cols = visibleColumns
        guard let fromIdx = cols.firstIndex(of: source) else { return cols }
        guard source != target else { return cols }
        let moved = cols.remove(at: fromIdx)
        guard var toIdx = cols.firstIndex(of: target) else {
            cols.append(moved)
            return cols
        }
        if !insertBefore {
            toIdx += 1
        }
        toIdx = min(max(0, toIdx), cols.count)
        cols.insert(moved, at: toIdx)
        return cols
    }

    private func sortHeaderContent(for field: LibraryColumnField) -> some View {
        let label = field == .key ? keyColumnHeader : field.headerLabel
        let isActive = activeSortColumn == field
        return Button {
            toggleSort(field)
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
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private func libraryColumnContextMenu() -> some View {
        columnVisibilityPicker()
        if visibleColumns.contains(.key) {
            Divider()
            Button("Toggle Key Notation (\(keyNotationMode == .camelot ? "Musical" : "Camelot"))") {
                keyNotationMode = keyNotationMode.toggled
            }
        }
    }

    @ViewBuilder
    private func columnVisibilityPicker() -> some View {
        Section("Fixed") {
            Toggle("Artist", isOn: .constant(true))
                .toggleStyle(.checkbox)
                .disabled(true)
            Toggle("Title", isOn: .constant(true))
                .toggleStyle(.checkbox)
                .disabled(true)
        }
        ForEach(columnPickerCategories, id: \.self) { category in
            Section(category) {
                ForEach(LibraryColumnField.configurable.filter { $0.pickerCategory == category }) { candidate in
                    Toggle(isOn: columnVisibilityBinding(candidate)) {
                        Text(candidate.headerLabel)
                    }
                    .toggleStyle(.checkbox)
                }
            }
        }
    }

    private var columnPickerCategories: [String] {
        ["Analysis", "ID3 metadata", "Library"]
    }

    private var columnHeaderDivider: some View {
        Rectangle()
            .fill(DubColor.divider)
            .frame(width: 1)
            .frame(maxHeight: .infinity)
    }

    private func toggleSort(_ column: LibraryColumnField) {
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
        case .duration:
            sortOrder = [KeyPathComparator(\.durationSortKey, order: order)]
        case .bpm:
            sortOrder = [KeyPathComparator(\.bpmSortKey, order: order)]
        case .album:
            sortOrder = [KeyPathComparator(\.albumSortKey, order: order)]
        case .genre:
            sortOrder = [KeyPathComparator(\.genreSortKey, order: order)]
        case .year:
            sortOrder = [KeyPathComparator(\.yearSortKey, order: order)]
        case .key:
            sortOrder = [KeyPathComparator(\.keySortKey, order: order)]
        case .comment:
            sortOrder = [KeyPathComparator(\.commentSortKey, order: order)]
        case .versionTokens:
            sortOrder = [KeyPathComparator(\.versionTokensSortKey, order: order)]
        case .source:
            sortOrder = [KeyPathComparator(\.sourceSortKey, order: order)]
        case .composer:
            sortOrder = [KeyPathComparator(\.composerSortKey, order: order)]
        case .trackNumber:
            sortOrder = [KeyPathComparator(\.trackNumberSortKey, order: order)]
        }
    }

    private func defaultColumnWidth(_ field: LibraryColumnField) -> CGFloat {
        switch field {
        case .artist: return 120
        case .title: return 180
        case .duration: return 52
        case .bpm: return 56
        case .year: return 48
        case .key: return 56
        case .comment, .album, .genre, .versionTokens, .source: return 140
        case .composer: return 120
        case .trackNumber: return 52
        }
    }

    private var parsedColumnWidths: [String: CGFloat] {
        guard let data = columnWidthsStorage.data(using: .utf8),
              let dict = try? JSONDecoder().decode([String: CGFloat].self, from: data)
        else {
            return [:]
        }
        return dict
    }

    private func storedColumnWidth(_ field: LibraryColumnField) -> CGFloat {
        let stored = parsedColumnWidths[field.rawValue]
        let width = stored ?? defaultColumnWidth(field)
        return clampColumnWidth(width)
    }

    private func columnWidth(_ field: LibraryColumnField) -> CGFloat {
        if let preview = columnResizePreview, preview.field == field {
            return preview.width
        }
        return storedColumnWidth(field)
    }

    private func clampColumnWidth(_ width: CGFloat) -> CGFloat {
        min(max(width, LibraryColumnLayout.minWidth), LibraryColumnLayout.maxWidth)
    }

    private func columnResizeLive(
        field: LibraryColumnField,
        dragStartGlobalX: CGFloat,
        locationGlobalX: CGFloat
    ) {
        if columnResizeDragOrigin?.field != field {
            columnResizeDragOrigin = (field, storedColumnWidth(field), dragStartGlobalX)
        }
        guard let origin = columnResizeDragOrigin, origin.field == field else { return }
        columnResizePreview = (
            field,
            clampColumnWidth(origin.width + locationGlobalX - origin.globalX)
        )
    }

    private func endColumnResize(
        field: LibraryColumnField,
        dragStartGlobalX: CGFloat,
        locationGlobalX: CGFloat
    ) {
        if columnResizeDragOrigin?.field != field {
            columnResizeDragOrigin = (field, storedColumnWidth(field), dragStartGlobalX)
        }
        guard let origin = columnResizeDragOrigin, origin.field == field else {
            columnResizeDragOrigin = nil
            columnResizePreview = nil
            return
        }
        persistColumnWidth(
            field,
            to: origin.width + locationGlobalX - origin.globalX
        )
        columnResizeDragOrigin = nil
    }

    private func persistColumnWidth(_ field: LibraryColumnField, to width: CGFloat) {
        let clamped = clampColumnWidth(width)
        columnResizePreview = nil
        var dict = parsedColumnWidths
        dict[field.rawValue] = clamped
        guard let data = try? JSONEncoder().encode(dict),
              let encoded = String(data: data, encoding: .utf8)
        else {
            return
        }
        columnWidthsStorage = encoded
    }

    @ViewBuilder
    private func columnCell(for field: LibraryColumnField, track: LibraryTrack) -> some View {
        switch field {
        case .artist:
            Text(track.artist ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .title:
            Text(displayTitle(track))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .duration:
            Text(formatDuration(track.durationMs))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .monospacedDigit()
        case .bpm:
            HStack(spacing: 4) {
                Text(formatBpm(track.bpm))
                if track.gridLocked {
                    Image(systemName: "lock.fill")
                        .font(.system(size: 9))
                        .foregroundStyle(DubColor.textSecondary)
                } else if let drift = track.gridDriftQuality, abs(drift) >= 3 {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.system(size: 9))
                        .foregroundStyle(.orange)
                        .help(
                            "May drift over a long mix · right-click → Lock grid to accept")
                }
            }
            .font(DubFont.body)
            .foregroundStyle(DubColor.textSecondary)
            .monospacedDigit()
        case .album:
            Text(track.album ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .genre:
            Text(track.genre ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .year:
            Text(track.year.map { String($0) } ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .monospacedDigit()
        case .key:
            Text(renderKey(track.key))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .help(keyTooltip(track.key))
        case .comment:
            Text(track.comment ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .help(track.comment ?? "")
        case .versionTokens:
            Text(track.versionTokens ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .source:
            Text(track.source)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .composer:
            Text(track.composer ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .trackNumber:
            Text(track.trackNumber.map { String($0) } ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .monospacedDigit()
        }
    }

    @ViewBuilder
    private func trackRow(for track: LibraryTrack) -> some View {
        let dragURL = libraryDragURL(for: track)
        // Selection highlight is painted by the AppKit
        // `LibrarySelectionLayerView` beneath the row host so a
        // click never has to round-trip through SwiftUI's view
        // diff before the colour appears. See
        // `LibraryTableScrollContainer` for the layer wiring.
        HStack(spacing: 0) {
            rowIndicators(for: track)
                .frame(width: 36, alignment: .leading)
            ForEach(visibleColumns) { field in
                columnCell(for: field, track: track)
                    .padding(.leading, LibraryColumnLayout.columnLeadingInset)
                    .modifier(DimUnanalyzed(track: track))
                    .frame(width: columnWidth(field), alignment: .leading)
            }
        }
        .padding(.horizontal, DubSpacing.lg)
        .frame(
            width: tableContentWidth,
            height: LibraryRowLayout.estimatedHeight,
            alignment: .leading)
        .contentShape(Rectangle())
        .if(dragURL != nil) { view in
            view.onDrag {
                if !selectedTrackIds.contains(track.id) {
                    selectedTrackIds = [track.id]
                    selectionAnchorId = track.id
                    Task { @MainActor in
                        syncModelPrimarySelection()
                    }
                }
                return NSItemProvider(object: dragURL! as NSURL)
            }
        }
        .onTapGesture(count: 1) {
            handleRowClick(track)
        }
        // Right-click menu is built by AppKit in
        // `LibraryDocumentWrapper.menu(for:)` instead of the
        // SwiftUI `.contextMenu` modifier. SwiftUI's `.contextMenu`
        // closure is empirically captured at the moment the row is
        // first attached and is NOT re-evaluated when upstream
        // `@State` (notably `selectedTrackIds`) changes, because
        // the AppKit selection layer (see
        // `LibraryTableScrollContainer.updateNSView`) deliberately
        // skips reassigning `bodyHost.rootView` on selection-only
        // changes for click latency. Building the menu in AppKit
        // reads the live selection set at right-click time and so
        // always reflects the actual multi-selection, including
        // the "Re-analyze Selected (N)" label.
    }

    private func handleRowClick(_ track: LibraryTrack) {
        searchFocused = false
        NSApp.keyWindow?.makeFirstResponder(nil)
        let flags = NSEvent.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if flags.contains(.command) {
            if selectedTrackIds.contains(track.id) {
                selectedTrackIds.remove(track.id)
                if selectionAnchorId == track.id {
                    selectionAnchorId = selectedTrackIds.sorted().first
                }
            } else {
                selectedTrackIds.insert(track.id)
                selectionAnchorId = track.id
            }
        } else if flags.contains(.shift) {
            let anchor = selectionAnchorId ?? primarySelectedTrackId ?? track.id
            selectRange(from: anchor, to: track.id)
            selectionAnchorId = track.id
        } else {
            selectedTrackIds = [track.id]
            selectionAnchorId = track.id
        }
    }

    private func selectRange(from anchorId: String, to targetId: String) {
        let ids = sortedTracks.map(\.id)
        guard let a = ids.firstIndex(of: anchorId),
              let b = ids.firstIndex(of: targetId)
        else {
            selectedTrackIds = [targetId]
            return
        }
        let lo = min(a, b)
        let hi = max(a, b)
        selectedTrackIds = Set(ids[lo...hi])
    }

    private func syncModelPrimarySelection() {
        if let trackId = primarySelectedTrackId,
           let snapshot = tracks.first(where: { $0.id == trackId })
        {
            model.selectLibraryTrack(trackId, snapshot: snapshot)
        } else if selectedTrackIds.isEmpty {
            model.selectedLibraryTrackId = nil
            model.selectedLibraryTrack = nil
        }
    }

    private func applyAnalysisUpdate(_ update: LibraryRowAnalysisUpdate) {
        guard let idx = tracks.firstIndex(where: { $0.id == update.trackId }) else {
            return
        }
        tracks[idx] = tracks[idx].patchedAfterAnalysis(update)
        tracksContentRevision &+= 1
    }

    /// Finder drag-and-drop onto the library listing. Folders are
    /// walked recursively; individual audio files import via the
    /// same `import_folder` entry point (WalkDir yields one file).
    ///
    /// `provider.loadItem` callbacks fire on an arbitrary queue, so
    /// the URL collector is serialised through a dedicated queue
    /// before the main-actor import hop.
    private func handleLibraryDrop(_ providers: [NSItemProvider]) -> Bool {
        guard model.libraryIsOpen, !model.libraryImportInProgress else { return false }
        let collector = DispatchQueue(label: "com.dub.library-drop-collector")
        var urls: [URL] = []
        let group = DispatchGroup()
        for provider in providers {
            group.enter()
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
                defer { group.leave() }
                let resolved: URL?
                if let url = item as? URL {
                    resolved = url
                } else if let data = item as? Data,
                          let url = URL(dataRepresentation: data, relativeTo: nil)
                {
                    resolved = url
                } else {
                    resolved = nil
                }
                guard let resolved else { return }
                collector.sync { urls.append(resolved) }
            }
        }
        group.notify(queue: .main) {
            let collected: [URL] = collector.sync { urls }
            let importable = collected.filter { url in
                var isDir: ObjCBool = false
                guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir)
                else { return false }
                if isDir.boolValue { return true }
                return Self.isSupportedAudioFile(url)
            }
            guard !importable.isEmpty else { return }
            Task { @MainActor in
                for url in importable {
                    await model.importLibraryFolder(url)
                }
            }
        }
        return !providers.isEmpty
    }

    private static func isSupportedAudioFile(_ url: URL) -> Bool {
        let ext = url.pathExtension.lowercased()
        return ["mp3", "wav", "flac", "aiff", "aif", "m4a", "aac", "alac", "ogg"].contains(ext)
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
        selectedTrackIds = [trackId]
        selectionAnchorId = trackId
        syncModelPrimarySelection()
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
        .contentShape(Rectangle())
        .onTapGesture {
            searchFocused = false
            NSApp.keyWindow?.makeFirstResponder(nil)
        }
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
        return String(format: "%.0f", b)
    }

    private func formatDuration(_ ms: UInt32) -> String {
        guard ms > 0 else { return "—" }
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
            selectedTrackIds = []
            selectionAnchorId = nil
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
            selectedTrackIds = []
            selectionAnchorId = nil
        }
        let preservedSelection = selectedTrackIds
        let preservedAnchor = selectionAnchorId
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
                let message = error.localizedDescription
                await MainActor.run {
                    model.surfaceError("Library refresh failed: \(message)")
                }
                rows = []
            }
            await MainActor.run {
                self.tracks = rows
                self.tracksContentRevision &+= 1
                self.isLoading = false
                if preserveSelection {
                    let visible = Set(rows.map(\.id))
                    self.selectedTrackIds = preservedSelection.intersection(visible)
                    if let anchor = preservedAnchor,
                       self.selectedTrackIds.contains(anchor)
                    {
                        self.selectionAnchorId = anchor
                    } else {
                        self.selectionAnchorId = self.selectedTrackIds.sorted().first
                    }
                    self.syncModelPrimarySelection()
                }
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

private struct LibraryColumnLayout {
    static let minWidth: CGFloat = 48
    static let maxWidth: CGFloat = 480
    /// Interactive hit target on the trailing edge (wider than the
    /// 1 px divider so the resize cursor is easy to acquire).
    static let resizeHandleHitWidth: CGFloat = 14
    /// Extends the hit zone left into the column label area.
    static let resizeHandleHitPadding: CGFloat = 6
    static var resizeHandleTotalWidth: CGFloat {
        resizeHandleHitPadding + resizeHandleHitWidth
    }
    /// Breathing room between the column divider (`|`) and the
    /// header label / row text. Without this the uppercase micro
    /// headers sit flush against the 1 px rule.
    static let columnLeadingInset: CGFloat = DubSpacing.sm
}

private struct ColumnHeaderFramesKey: PreferenceKey {
    static var defaultValue: [LibraryColumnField: CGRect] = [:]
    static func reduce(
        value: inout [LibraryColumnField: CGRect],
        nextValue: () -> [LibraryColumnField: CGRect]
    ) {
        value.merge(nextValue()) { _, new in new }
    }
}

/// Drag handle on the trailing edge of a library column header.
private struct LibraryColumnResizeHandle: View {
    let onDragChanged: (_ dragStartGlobalX: CGFloat, _ locationGlobalX: CGFloat) -> Void
    let onDragEnded: (_ dragStartGlobalX: CGFloat, _ locationGlobalX: CGFloat) -> Void

    var body: some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: LibraryColumnLayout.resizeHandleHitPadding)
            Rectangle()
                .fill(Color.clear)
                .frame(width: LibraryColumnLayout.resizeHandleHitWidth)
        }
        .frame(maxHeight: .infinity)
        .contentShape(Rectangle())
            .onHover { hovering in
                if hovering {
                    NSCursor.resizeLeftRight.push()
                } else {
                    NSCursor.pop()
                }
            }
            .highPriorityGesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .global)
                    .onChanged { value in
                        onDragChanged(value.startLocation.x, value.location.x)
                    }
                    .onEnded { value in
                        onDragEnded(value.startLocation.x, value.location.x)
                    }
            )
    }
}

private enum LibraryRowLayout {
    static let estimatedHeight: CGFloat = 28
    static let headerHeight: CGFloat = 22
}

/// Right-click-menu inputs for `LibraryTableScrollContainer`.
/// Bundles the batch-in-progress predicate (used to grey out
/// "Re-analyze" while another batch runs) and the two action
/// callbacks. Grouped so the SwiftUI call site doesn't need to
/// pass three loose params, and so the menu contract stays
/// readable in one place.
///
/// The callbacks fire on the main thread from the AppKit
/// NSMenu dispatch — callers MUST own their own
/// `Task { @MainActor in … }` if they touch model state.
struct LibraryTableMenu {
    let analysisBatchInProgress: Bool
    let onAnalyzeRequested: ([String]) -> Void
    let onSetGridLocked: (String, Bool) -> Void
}

/// One-shot programmatic-scroll request for
/// `LibraryTableScrollContainer`. The container scrolls the
/// matching row into view on the next `updateNSView` cycle then
/// invokes `onScrollHandled` so the parent can clear its
/// `@State` binding (otherwise the next render would scroll
/// again on every keystroke).
struct LibraryTableScroll {
    let scrollToTrackId: String?
    let scrollDelta: Int
    let onScrollHandled: () -> Void
}

/// Sticky header + vertically/horizontally scrollable rows. The body
/// `NSScrollView` owns the horizontal scroller; the header clips and
/// tracks the body's horizontal offset.
///
/// ## SwiftUI ↔ AppKit bridge contract
///
/// **Snapshot props** (read at `updateNSView` time, latest value
/// wins, no re-read between updates):
///
/// * `tableWidth`, `columnOrderKey`, `headerStateKey` — geometry +
///   header layout. A change forces the header to rebuild.
/// * `tracksContentRevision` — bumped by the parent whenever
///   in-memory row fields change (analysis patch, lock toggle).
///   Used to force the body host to re-render without changing the
///   row id list. Without this, AppKit caches the row view.
/// * `header`, `rows` (both `AnyView`) — the SwiftUI sub-trees the
///   AppKit hosts wrap. Re-built on every parent render.
/// * `trackIds` — id-order array. Used by selection + scroll math.
/// * `visibleTracks` — full row snapshots. Used by the menu builder
///   for `gridLocked` / `isAnalyzed` per row.
/// * `selectedTrackIds` — read by the AppKit selection highlight
///   layer at `updateNSView` time AND by the menu builder at
///   right-click time (live, not at SwiftUI build time — that was
///   the multi-select-label staleness bug).
/// * `menu: LibraryTableMenu` — bundles the menu's batch
///   in-progress predicate and the two action callbacks. See the
///   struct's own doc comment for the actor contract.
/// * `scroll: LibraryTableScroll` — bundles the one-shot
///   programmatic scroll request + handler. See the struct's own
///   doc comment for the one-shot reset protocol.
///
/// **Lifecycle**: `Coordinator` owns the `NSScrollView`s, the
/// document wrapper, the selection layer, and the menu-action
/// anchor list. `dismantleNSView` calls `Coordinator.uninstall()`
/// which drops the document wrapper's menu builder closure (so it
/// can't fire on a torn-down coordinator) and clears the scroll
/// view's document view references.
///
/// **Click semantics**: row selection paints via the AppKit
/// `LibrarySelectionLayerView` (NOT a SwiftUI `.background`) so
/// the highlight appears on the same frame the click lands. Hit
/// testing on the layer returns `nil` so clicks pass through to
/// the SwiftUI row underneath. The right-click NSMenu is owned by
/// the `LibraryDocumentWrapper` and built fresh on every event
/// via `Coordinator.buildMenu(for:rowHeight:)`.
private struct LibraryTableScrollContainer: NSViewRepresentable {
    let tableWidth: CGFloat
    let columnOrderKey: String
    let headerStateKey: String
    let tracksContentRevision: UInt64
    let selectedTrackIds: Set<String>
    let header: AnyView
    let rows: AnyView
    let trackIds: [String]
    /// Live snapshot of the rows currently visible to the user
    /// (post-search, post-sort). Used by `LibraryDocumentWrapper`
    /// to build the right-click menu for whichever row the user
    /// clicks on. Kept separate from `trackIds` because the menu
    /// needs `gridLocked` + `isAnalyzed` per track, not just the
    /// identity.
    let visibleTracks: [LibraryTrack]
    /// Right-click menu wiring (analyse + lock toggle + batch
    /// state). Grouped so the call site doesn't have to thread
    /// three loose callbacks through every refactor and so the
    /// menu contract has a single named home (see `LibraryTableMenu`).
    let menu: LibraryTableMenu
    /// Programmatic scroll request (keyboard arrow navigation).
    /// Grouped so the one-shot "scroll to this id then clear me"
    /// protocol stays explicit at the call site.
    let scroll: LibraryTableScroll

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> NSView {
        let headerHost = NSHostingView(rootView: header)
        headerHost.translatesAutoresizingMaskIntoConstraints = false

        let headerWrapper = NSView()
        headerWrapper.wantsLayer = true
        headerWrapper.layer?.masksToBounds = true
        headerWrapper.translatesAutoresizingMaskIntoConstraints = false
        headerWrapper.addSubview(headerHost)

        let bodyScroll = NSScrollView()
        bodyScroll.hasVerticalScroller = true
        bodyScroll.hasHorizontalScroller = true
        bodyScroll.autohidesScrollers = true
        bodyScroll.scrollerStyle = .legacy
        bodyScroll.borderType = .noBorder
        bodyScroll.drawsBackground = false
        bodyScroll.translatesAutoresizingMaskIntoConstraints = false

        // Selection highlights are painted by an AppKit layer that
        // lives BELOW the SwiftUI body host inside the scroll view's
        // document. Painting selection here (instead of as a SwiftUI
        // `.background` per row) means selection changes never have
        // to round-trip through SwiftUI's view diff — clicks paint on
        // the same frame, Finder-style.
        //
        // Frame-based layout (not Auto Layout) deliberately mirrors
        // the existing `updateWidths` / `updateBodyHeight` path. An
        // earlier attempt at Auto Layout collapsed the host to 1 pt
        // because the wrapper had no intrinsic width constraints.
        let initialBounds = NSRect(x: 0, y: 0, width: max(tableWidth, 1), height: 1)
        let documentWrapper = LibraryDocumentWrapper(frame: initialBounds)
        documentWrapper.translatesAutoresizingMaskIntoConstraints = true
        documentWrapper.autoresizesSubviews = true

        let selectionLayer = LibrarySelectionLayerView(frame: documentWrapper.bounds)
        selectionLayer.translatesAutoresizingMaskIntoConstraints = true
        selectionLayer.autoresizingMask = [.width, .height]
        selectionLayer.fillColor = NSColor(DubColor.surface2)
        selectionLayer.rowHeight = LibraryRowLayout.estimatedHeight

        let bodyHost = NSHostingView(rootView: rows)
        bodyHost.translatesAutoresizingMaskIntoConstraints = true
        bodyHost.autoresizingMask = [.width, .height]
        bodyHost.frame = documentWrapper.bounds

        // Order matters: selection layer first so it sits BELOW the
        // SwiftUI host. The host's row backgrounds are clear, so the
        // colored rects show through.
        documentWrapper.addSubview(selectionLayer)
        documentWrapper.addSubview(bodyHost)
        bodyScroll.documentView = documentWrapper

        let widthConstraint = headerHost.widthAnchor.constraint(
            equalToConstant: max(tableWidth, 1))
        let leadingConstraint = headerHost.leadingAnchor.constraint(
            equalTo: headerWrapper.leadingAnchor)

        NSLayoutConstraint.activate([
            headerWrapper.heightAnchor.constraint(equalToConstant: LibraryRowLayout.headerHeight),
            leadingConstraint,
            headerHost.topAnchor.constraint(equalTo: headerWrapper.topAnchor),
            headerHost.heightAnchor.constraint(equalToConstant: LibraryRowLayout.headerHeight),
            widthConstraint,
        ])

        let stack = NSStackView(views: [headerWrapper, bodyScroll])
        stack.orientation = .vertical
        stack.spacing = 0
        stack.translatesAutoresizingMaskIntoConstraints = false
        headerWrapper.setContentHuggingPriority(.required, for: .vertical)
        bodyScroll.setContentHuggingPriority(.defaultLow, for: .vertical)

        context.coordinator.install(
            bodyScroll: bodyScroll,
            headerHost: headerHost,
            bodyHost: bodyHost,
            documentWrapper: documentWrapper,
            selectionLayer: selectionLayer,
            headerWidthConstraint: widthConstraint,
            headerLeadingConstraint: leadingConstraint
        )
        context.coordinator.recordSnapshot(
            tableWidth: tableWidth,
            columnOrderKey: columnOrderKey,
            headerStateKey: headerStateKey,
            tracksContentRevision: tracksContentRevision,
            selectedTrackIds: selectedTrackIds,
            trackIds: trackIds)
        context.coordinator.updateMenuState(
            visibleTracks: visibleTracks,
            selectedTrackIds: selectedTrackIds,
            analysisBatchInProgress: menu.analysisBatchInProgress,
            onAnalyzeRequested: menu.onAnalyzeRequested,
            onSetGridLocked: menu.onSetGridLocked)
        context.coordinator.attachMenuBuilder(rowHeight: LibraryRowLayout.estimatedHeight)
        context.coordinator.updateBodyHeight()
        context.coordinator.updateSelectionHighlights(
            selectedTrackIds: selectedTrackIds, trackIds: trackIds)
        return stack
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        let coordinator = context.coordinator
        let tableWidthChanged = coordinator.lastTableWidth != tableWidth
        let tracksChanged = coordinator.lastTrackIds != trackIds
        let columnsChanged = coordinator.lastColumnOrderKey != columnOrderKey
        let headerStateChanged = coordinator.lastHeaderStateKey != headerStateKey
        let tracksContentChanged = coordinator.lastTracksContentRevision != tracksContentRevision
        let selectionChanged = coordinator.lastSelectedTrackIds != selectedTrackIds
        // Selection deliberately NOT in `bodyPresentationChanged` —
        // reassigning `bodyHost.rootView` triggers a full SwiftUI
        // diff over every row (AnyView wraps defeat structural
        // diffing). Selection paints via the AppKit layer instead,
        // so clicks land on the next frame.
        let bodyPresentationChanged = tracksChanged
            || tableWidthChanged
            || columnsChanged
            || tracksContentChanged

        if tableWidthChanged || tracksChanged || columnsChanged || headerStateChanged {
            coordinator.headerHost?.rootView = header
            coordinator.headerWidthConstraint?.constant = max(tableWidth, 1)
        }

        if bodyPresentationChanged {
            coordinator.bodyHost?.rootView = rows
            coordinator.updateWidths(tableWidth)
            if tracksChanged {
                coordinator.updateBodyHeight()
            }
        } else if tableWidthChanged {
            coordinator.updateWidths(tableWidth)
        }

        if selectionChanged || tracksChanged {
            coordinator.updateSelectionHighlights(
                selectedTrackIds: selectedTrackIds, trackIds: trackIds)
        }

        coordinator.recordSnapshot(
            tableWidth: tableWidth,
            columnOrderKey: columnOrderKey,
            headerStateKey: headerStateKey,
            tracksContentRevision: tracksContentRevision,
            selectedTrackIds: selectedTrackIds,
            trackIds: trackIds)
        // The right-click menu reads from the Coordinator at
        // `menu(for:)` time, so it ALWAYS sees the latest selection
        // / lock state even though the SwiftUI rows themselves
        // are not re-rendered on selection changes (intentional —
        // the AppKit selection layer paints highlights instead).
        coordinator.updateMenuState(
            visibleTracks: visibleTracks,
            selectedTrackIds: selectedTrackIds,
            analysisBatchInProgress: menu.analysisBatchInProgress,
            onAnalyzeRequested: menu.onAnalyzeRequested,
            onSetGridLocked: menu.onSetGridLocked)
        coordinator.syncHeaderOffset()

        if let id = scroll.scrollToTrackId {
            coordinator.scrollToTrack(id: id, trackIds: trackIds, delta: scroll.scrollDelta)
            DispatchQueue.main.async {
                scroll.onScrollHandled()
            }
        }
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.uninstall()
    }

    final class Coordinator {
        weak var bodyScroll: NSScrollView?
        weak var headerHost: NSHostingView<AnyView>?
        weak var bodyHost: NSHostingView<AnyView>?
        weak var documentWrapper: LibraryDocumentWrapper?
        weak var selectionLayer: LibrarySelectionLayerView?
        weak var headerWidthConstraint: NSLayoutConstraint?
        weak var headerLeadingConstraint: NSLayoutConstraint?
        private var boundsObserver: NSObjectProtocol?
        fileprivate var lastTableWidth: CGFloat = -1
        fileprivate var lastColumnOrderKey: String = ""
        fileprivate var lastHeaderStateKey: String = ""
        fileprivate var lastTracksContentRevision: UInt64 = 0
        fileprivate var lastSelectedTrackIds: Set<String> = []
        fileprivate var lastTrackIds: [String] = []

        /// Live state consulted by `LibraryDocumentWrapper.menu(for:)`
        /// at right-click time. Refreshed on every `updateNSView`
        /// pass so the menu always reflects the user's current
        /// selection, not whatever state happened to be captured
        /// when the row was first laid out.
        fileprivate var visibleTracks: [LibraryTrack] = []
        fileprivate var menuSelectedTrackIds: Set<String> = []
        fileprivate var menuAnalysisBatchInProgress: Bool = false
        fileprivate var onAnalyzeRequested: (([String]) -> Void)?
        fileprivate var onSetGridLocked: ((String, Bool) -> Void)?
        /// Anchors the closures the menu items invoke. NSMenuItem
        /// holds only a weak target reference; without keeping the
        /// targets alive on the Coordinator the actions would
        /// segfault the moment the menu actually dispatches.
        private var menuActionAnchors: [LibraryMenuActionTarget] = []

        func recordSnapshot(
            tableWidth: CGFloat,
            columnOrderKey: String,
            headerStateKey: String,
            tracksContentRevision: UInt64,
            selectedTrackIds: Set<String>,
            trackIds: [String]
        ) {
            lastTableWidth = tableWidth
            lastColumnOrderKey = columnOrderKey
            lastHeaderStateKey = headerStateKey
            lastTracksContentRevision = tracksContentRevision
            lastSelectedTrackIds = selectedTrackIds
            lastTrackIds = trackIds
        }

        func install(
            bodyScroll: NSScrollView,
            headerHost: NSHostingView<AnyView>,
            bodyHost: NSHostingView<AnyView>,
            documentWrapper: LibraryDocumentWrapper,
            selectionLayer: LibrarySelectionLayerView,
            headerWidthConstraint: NSLayoutConstraint,
            headerLeadingConstraint: NSLayoutConstraint
        ) {
            self.bodyScroll = bodyScroll
            self.headerHost = headerHost
            self.bodyHost = bodyHost
            self.documentWrapper = documentWrapper
            self.selectionLayer = selectionLayer
            self.headerWidthConstraint = headerWidthConstraint
            self.headerLeadingConstraint = headerLeadingConstraint
            bodyScroll.contentView.postsBoundsChangedNotifications = true
            boundsObserver = NotificationCenter.default.addObserver(
                forName: NSView.boundsDidChangeNotification,
                object: bodyScroll.contentView,
                queue: .main
            ) { [weak self] _ in
                self?.syncHeaderOffset()
            }
        }

        func uninstall() {
            if let boundsObserver {
                NotificationCenter.default.removeObserver(boundsObserver)
            }
        }

        func syncHeaderOffset() {
            guard let bodyScroll else { return }
            let x = bodyScroll.contentView.bounds.origin.x
            headerLeadingConstraint?.constant = -x
        }

        func updateWidths(_ tableWidth: CGFloat) {
            guard let documentWrapper else { return }
            var frame = documentWrapper.frame
            frame.size.width = max(tableWidth, 1)
            documentWrapper.frame = frame
            // bodyHost + selectionLayer follow via `autoresizingMask`.
        }

        func updateBodyHeight() {
            guard let bodyHost, let documentWrapper, let bodyScroll else { return }
            bodyHost.invalidateIntrinsicContentSize()
            let height = max(bodyHost.fittingSize.height, 1)
            var frame = documentWrapper.frame
            frame.size.height = height
            documentWrapper.frame = frame
            // bodyScroll re-evaluates scrollable area off `documentView.frame`.
            bodyScroll.documentView = documentWrapper
        }

        /// O(n) projection of the selected-id set onto sorted row
        /// indices. Cheap — `selectionLayer.draw(_:)` only fills
        /// the intersect of dirty rect × selected rectangles, so a
        /// click invalidates one or two rows worth of pixels.
        func updateSelectionHighlights(
            selectedTrackIds: Set<String>,
            trackIds: [String]
        ) {
            guard let selectionLayer else { return }
            if selectedTrackIds.isEmpty {
                selectionLayer.selectedRowIndices = IndexSet()
                return
            }
            var indices = IndexSet()
            for (i, id) in trackIds.enumerated() where selectedTrackIds.contains(id) {
                indices.insert(i)
            }
            selectionLayer.selectedRowIndices = indices
        }

        /// Snapshot the menu-time inputs so the AppKit
        /// `menu(for:)` override (which fires asynchronously
        /// from the SwiftUI render pass) reads consistent state.
        /// Called on every `updateNSView`.
        func updateMenuState(
            visibleTracks: [LibraryTrack],
            selectedTrackIds: Set<String>,
            analysisBatchInProgress: Bool,
            onAnalyzeRequested: @escaping ([String]) -> Void,
            onSetGridLocked: @escaping (String, Bool) -> Void
        ) {
            self.visibleTracks = visibleTracks
            self.menuSelectedTrackIds = selectedTrackIds
            self.menuAnalysisBatchInProgress = analysisBatchInProgress
            self.onAnalyzeRequested = onAnalyzeRequested
            self.onSetGridLocked = onSetGridLocked
        }

        /// Wire the document wrapper to ask us for an `NSMenu` on
        /// every right-click. The closure captures the Coordinator
        /// weakly so we don't leak it; it reads the live
        /// `visibleTracks` + `menuSelectedTrackIds` set at click
        /// time, which is the entire point — SwiftUI's
        /// `.contextMenu` body is captured at row-attach time and
        /// would have shown stale single-row state.
        func attachMenuBuilder(rowHeight: CGFloat) {
            documentWrapper?.menuBuilder = { [weak self] event in
                self?.buildMenu(for: event, rowHeight: rowHeight)
            }
        }

        /// Translate the right-click into a row index and build the
        /// matching `NSMenu`. Returns `nil` (no menu) when the
        /// click misses every row.
        fileprivate func buildMenu(for event: NSEvent, rowHeight: CGFloat) -> NSMenu? {
            guard let documentWrapper else { return nil }
            let point = documentWrapper.convert(event.locationInWindow, from: nil)
            let idx = Int(floor(point.y / rowHeight))
            guard idx >= 0, idx < visibleTracks.count else { return nil }
            let track = visibleTracks[idx]
            return buildContextMenu(for: track)
        }

        /// Pure menu construction: no AppKit display side effects.
        /// Split out so it can be unit-tested via the
        /// `visibleTracks` / `menuSelectedTrackIds` inputs without
        /// needing an `NSWindow` to host the popup.
        private func buildContextMenu(for track: LibraryTrack) -> NSMenu {
            let menu = NSMenu()
            menu.autoenablesItems = false
            let targets = analyzeTargets(rightClickedTrack: track)
            let unlocked = targets.filter { !isLocked($0) }
            let label = analyzeMenuLabel(
                rightClickedTrack: track,
                allCount: targets.count,
                unlockedCount: unlocked.count)
            let analyzeItem = NSMenuItem(title: label, action: nil, keyEquivalent: "")
            if !menuAnalysisBatchInProgress, !unlocked.isEmpty,
               let onAnalyze = onAnalyzeRequested
            {
                let target = LibraryMenuActionTarget { onAnalyze(unlocked) }
                menuActionAnchors.append(target)
                analyzeItem.target = target
                analyzeItem.action = #selector(LibraryMenuActionTarget.dubMenuPerform(_:))
                analyzeItem.isEnabled = true
            } else {
                analyzeItem.isEnabled = false
            }
            menu.addItem(analyzeItem)
            menu.addItem(.separator())
            let lockTitle = track.gridLocked ? "Unlock grid" : "Lock grid"
            let lockItem = NSMenuItem(title: lockTitle, action: nil, keyEquivalent: "")
            if let onSetGridLocked {
                let target = LibraryMenuActionTarget {
                    onSetGridLocked(track.id, !track.gridLocked)
                }
                menuActionAnchors.append(target)
                lockItem.target = target
                lockItem.action = #selector(LibraryMenuActionTarget.dubMenuPerform(_:))
                lockItem.isEnabled = true
            } else {
                lockItem.isEnabled = false
            }
            menu.addItem(lockItem)
            // Drop the previous run's anchors once a new menu is
            // built — the anchors are only needed while the menu
            // is visible, and AppKit keeps the in-flight menu
            // retained via its own dispatcher. Without this the
            // anchor list would grow unbounded over a long
            // session.
            menuActionAnchors = Array(menuActionAnchors.suffix(8))
            return menu
        }

        /// Multi-selection acts on the full selection when the
        /// right-clicked row IS part of it; otherwise the menu acts
        /// only on the right-clicked row (Finder semantics).
        private func analyzeTargets(rightClickedTrack track: LibraryTrack) -> [String] {
            if menuSelectedTrackIds.contains(track.id), menuSelectedTrackIds.count > 1 {
                return Array(menuSelectedTrackIds)
            }
            return [track.id]
        }

        private func isLocked(_ trackId: String) -> Bool {
            visibleTracks.first(where: { $0.id == trackId })?.gridLocked ?? false
        }

        /// PRD-BEATS §4.4 label. Reflects mixed-lock selections as
        /// "Re-analyze Selected (3 of 5)" so the user can see how
        /// many rows the analyse pass will skip before clicking.
        private func analyzeMenuLabel(
            rightClickedTrack track: LibraryTrack,
            allCount: Int,
            unlockedCount: Int
        ) -> String {
            let verb = track.isAnalyzed ? "Re-analyze" : "Analyze"
            if allCount > 1 {
                if unlockedCount < allCount {
                    return "\(verb) Selected (\(unlockedCount) of \(allCount))"
                }
                return "\(verb) Selected (\(allCount))"
            }
            return verb
        }

        func scrollToTrack(id: String, trackIds: [String], delta: Int) {
            guard let bodyScroll,
                  let idx = trackIds.firstIndex(of: id)
            else { return }
            let clipView = bodyScroll.contentView
            let rowY = CGFloat(idx) * LibraryRowLayout.estimatedHeight
            let viewport = clipView.documentVisibleRect.height
            var targetY = rowY
            if delta > 0 {
                targetY = rowY - viewport * 0.92
            } else if delta < 0 {
                targetY = rowY - viewport * 0.08
            }
            let maxY = max(0, clipView.documentRect.height - viewport)
            targetY = min(max(0, targetY), maxY)
            clipView.scroll(to: NSPoint(x: clipView.bounds.origin.x, y: targetY))
            bodyScroll.reflectScrolledClipView(clipView)
        }
    }
}

/// Container for the AppKit selection layer + the SwiftUI body
/// host inside the library's scroll view. Flipped so row 0 sits
/// at `y = 0` (top), matching `LibraryRowLayout.estimatedHeight`
/// arithmetic used elsewhere in the coordinator.
///
/// Owns the right-click context menu. The actual menu construction
/// lives in `LibraryTableScrollContainer.Coordinator` (so it can
/// read the live selection + lock state); this view's only job is
/// to receive the `menu(for:)` callback from AppKit and forward
/// it. Building the menu in AppKit (instead of SwiftUI's
/// `.contextMenu`) is what gives the "Re-analyze Selected (N)"
/// label its always-fresh count: SwiftUI's modifier captures
/// state when the row first attaches and is not re-evaluated by
/// the AppKit selection-layer optimisation (see
/// `LibraryTableScrollContainer.updateNSView`).
private final class LibraryDocumentWrapper: NSView {
    var menuBuilder: ((NSEvent) -> NSMenu?)?

    override var isFlipped: Bool { true }

    override func menu(for event: NSEvent) -> NSMenu? {
        if let menu = menuBuilder?(event) {
            return menu
        }
        return super.menu(for: event)
    }
}

/// Tiny `NSObject` wrapper so an `NSMenuItem` can fire a Swift
/// closure. `NSMenuItem` only accepts an Objective-C-style
/// (`target`, `action:`) pair; this class bridges that to a
/// `() -> Void` closure. Lifetime is managed by the Coordinator
/// (see `menuActionAnchors`) because `NSMenuItem.target` is weak.
///
/// IMPORTANT: the action method MUST NOT be named `perform(_:)`.
/// Selector `perform:` collides with the long-deprecated
/// `-[NSObject perform:]` method (an old alias for
/// `performSelector:` that still lives in the Obj-C runtime).
/// When AppKit dispatches the menu item action by selector
/// `perform:`, the runtime resolves it against the base NSObject
/// implementation instead of our override (the signatures differ
/// enough that the override does not actually shadow it) and the
/// closure silently never fires. The menu item visibly clicks,
/// no exception is thrown, and "nothing happens" — a UI bug
/// class that wasted multiple debugging rounds before the
/// collision was identified. Using a project-scoped name like
/// `dubMenuPerform:` sidesteps the collision entirely.
private final class LibraryMenuActionTarget: NSObject {
    private let work: () -> Void

    init(_ work: @escaping () -> Void) {
        self.work = work
    }

    @objc func dubMenuPerform(_ sender: Any?) {
        work()
    }
}

/// Paints library row-selection rectangles in AppKit so a click
/// never has to wait for SwiftUI's view diff to repaint the row
/// background. The layer is positioned below the SwiftUI body
/// host inside `LibraryDocumentWrapper`; rows themselves draw a
/// clear background so the fill shows through.
///
/// Rows are assumed uniform-height — same assumption the rest of
/// the table makes (see `LibraryRowLayout.estimatedHeight` and
/// the `scrollToTrack` arithmetic). If row heights diverge in the
/// future, this view will need a row-frame oracle, but today it's
/// strictly `y = i * rowHeight`.
private final class LibrarySelectionLayerView: NSView {
    var rowHeight: CGFloat = 28 {
        didSet {
            if rowHeight != oldValue { needsDisplay = true }
        }
    }

    var fillColor: NSColor = .clear {
        didSet { needsDisplay = true }
    }

    var selectedRowIndices: IndexSet = [] {
        didSet {
            guard selectedRowIndices != oldValue else { return }
            invalidateRows(in: oldValue.union(selectedRowIndices))
        }
    }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { false }
    override var acceptsFirstResponder: Bool { false }

    /// Crucial — this layer is decorative only. Returning `nil`
    /// from `hitTest` lets every mouse event pass straight through
    /// to the SwiftUI host above. Without this the selection
    /// rects would swallow clicks and break tap-to-select.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func draw(_ dirtyRect: NSRect) {
        guard !selectedRowIndices.isEmpty else { return }
        fillColor.setFill()
        for idx in selectedRowIndices {
            let rowRect = NSRect(
                x: 0, y: CGFloat(idx) * rowHeight,
                width: bounds.width, height: rowHeight)
            if rowRect.intersects(dirtyRect) {
                rowRect.fill()
            }
        }
    }

    /// Invalidate only the strips that changed selection state.
    /// Keeps the per-click repaint to two row-height bands instead
    /// of the whole scrollable area (which on a long library is
    /// many thousands of points tall).
    private func invalidateRows(in indices: IndexSet) {
        guard !indices.isEmpty else { return }
        for idx in indices {
            let rowRect = NSRect(
                x: 0, y: CGFloat(idx) * rowHeight,
                width: bounds.width, height: rowHeight)
            setNeedsDisplay(rowRect)
        }
    }
}

/// Resigns library search focus on any click that lands outside
/// an editable text field. SwiftUI `@FocusState` alone is
/// unreliable with AppKit-hosted table rows.
///
/// ## SwiftUI ↔ AppKit bridge contract
///
/// **Snapshot props**: none. The bridge has no SwiftUI inputs; it
/// exists purely as a parent-attached background that installs a
/// global mouse-down monitor on `makeNSView`.
///
/// **Closure props**: none. Side-effect-only — the monitor calls
/// `NSApp.keyWindow?.makeFirstResponder(nil)` directly.
///
/// **Lifecycle**: the local-monitor handle is owned by the
/// `Coordinator`. `dismantleNSView` calls `uninstall()` which
/// removes the monitor; without this the monitor outlives the
/// SwiftUI view tree and fires against a stale window.
///
/// **Hit-test contract**: the hit-test for "is the click on an
/// editable text view?" runs at the `contentView` of the key
/// window using `event.locationInWindow` (NOT converted). Using
/// converted coordinates produced false positives on the search
/// field and broke the dismiss.
private struct LibraryTextFocusDismissMonitor: NSViewRepresentable {
    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        context.coordinator.install()
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.uninstall()
    }

    final class Coordinator {
        private var monitor: Any?

        func install() {
            guard monitor == nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { event in
                guard Self.isEditableTextFirstResponder() else { return event }
                guard !Self.eventTargetsEditableText(event) else { return event }
                NSApp.keyWindow?.makeFirstResponder(nil)
                return event
            }
        }

        func uninstall() {
            if let monitor {
                NSEvent.removeMonitor(monitor)
            }
            monitor = nil
        }

        private static func isEditableTextFirstResponder() -> Bool {
            guard let responder = NSApp.keyWindow?.firstResponder else { return false }
            if responder is NSTextView { return true }
            if let field = responder as? NSTextField, field.isEditable {
                return true
            }
            return false
        }

        private static func eventTargetsEditableText(_ event: NSEvent) -> Bool {
            // `NSView.hitTest(_:)` expects the point in the
            // receiver's superview coordinate space. Window
            // coordinates ARE the contentView's effective parent
            // space, so pass `locationInWindow` directly — do not
            // convert into contentView-local coordinates first.
            guard let window = event.window,
                  let contentView = window.contentView
            else { return false }
            var candidate: NSView? = contentView.hitTest(event.locationInWindow)
            while let view = candidate {
                if view is NSTextView { return true }
                if let field = view as? NSTextField, field.isEditable { return true }
                candidate = view.superview
            }
            return false
        }
    }
}

/// Right-click context menu for a library row. PRD-BEATS §4.4
/// "single Re-analyze entry" — collapses Analyze and Re-analyze
/// into one verb that depends on the row's current state. Lives in
/// its own `View` struct (rather than inline in
/// `LibraryView.trackRow.contextMenu { ... }`) for one reason:
/// SwiftUI's `.contextMenu` body closure does NOT reliably
/// re-evaluate when the parent's `@State` changes. Empirically the
/// closure runs once when the row is first attached and the
/// captured selection then sticks across subsequent multi-selects.
/// Threading the selection through an explicit child view forces
/// SwiftUI's identity diffing to recreate the menu body whenever
/// the inputs change, so the visible label "(N of M)" tracks the
/// live selection rather than whatever was selected when the row
/// first appeared.
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

// MARK: - LibraryTrack analysis patch

private extension LibraryTrack {
    func patchedAfterAnalysis(_ update: LibraryRowAnalysisUpdate) -> LibraryTrack {
        LibraryTrack(
            id: id,
            title: title,
            artist: artist,
            album: album,
            genre: genre,
            year: year,
            bpm: update.bpm ?? bpm,
            key: update.key ?? key,
            durationMs: durationMs,
            versionTokens: versionTokens,
            potentialDuplicateId: potentialDuplicateId,
            source: source,
            primaryVolumeUuid: primaryVolumeUuid,
            primaryVolumeMountPoint: primaryVolumeMountPoint,
            primaryRelativePath: primaryRelativePath,
            isAnalyzed: update.isAnalyzed || isAnalyzed,
            keyDisagreement: keyDisagreement,
            comment: comment,
            composer: composer,
            trackNumber: trackNumber,
            gridLocked: gridLocked,
            gridDriftQuality: gridDriftQuality)
    }
}

// MARK: - Library arrow-key navigation

/// Local ↑/↓ handler for the library list.
///
/// ## SwiftUI ↔ AppKit bridge contract
///
/// **Snapshot props** (read by the Coordinator at `updateNSView`):
///
/// * `trackIds: [String]` — current id-order array. The arrow-key
///   handler uses this to translate "selection + delta" into "next
///   track id". Stale `trackIds` means arrow keys jump to a
///   wrong-but-old id; always pass the post-sort, post-filter list.
///
/// **Binding props** (two-way; the handler mutates them):
///
/// * `selectedTrackIds: Set<String>` — selection state. Mutated by
///   arrow + shift-arrow.
/// * `selectionAnchorId: String?` — shift-selection anchor.
///   Mutated on plain arrow (anchor = new selection) and left
///   alone on shift-arrow (anchor preserved).
///
/// **Closure props**:
///
/// * `onArrowNavigate(String, Int)?` — fires AFTER selection
///   mutation, with the new selected id + direction (-1 / +1).
///   Used by the parent to scroll the just-selected row into view
///   and clear `keyboardScrollTarget` once handled.
/// * `onSelectionChanged()` — fires AFTER every selection mutation
///   so the model's primary-selection mirror stays in sync.
///
/// **Lifecycle**: the Coordinator installs an `NSEvent`
/// local-monitor for `.keyDown`; `dismantleNSView` removes it.
///
/// **Hit-test contract**: parent uses `.allowsHitTesting(false)`
/// on this view so mouse events fall through to the row tree
/// beneath. The key-down monitor doesn't care — it fires on the
/// first responder, which is the table region as long as no text
/// field has focus (see `LibraryTextFocusDismissMonitor`).
private struct LibraryArrowKeyView: NSViewRepresentable {
    @Binding var selectedTrackIds: Set<String>
    @Binding var selectionAnchorId: String?
    let trackIds: [String]
    var onArrowNavigate: ((String, Int) -> Void)?
    var onSelectionChanged: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(
            selectedTrackIds: $selectedTrackIds,
            selectionAnchorId: $selectionAnchorId,
            onArrowNavigate: onArrowNavigate,
            onSelectionChanged: onSelectionChanged)
    }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        context.coordinator.install()
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.trackIds = trackIds
        context.coordinator.onArrowNavigate = onArrowNavigate
        context.coordinator.onSelectionChanged = onSelectionChanged
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.uninstall()
    }

    @MainActor
    final class Coordinator {
        @Binding var selectedTrackIds: Set<String>
        @Binding var selectionAnchorId: String?
        var trackIds: [String] = []
        var onArrowNavigate: ((String, Int) -> Void)?
        var onSelectionChanged: () -> Void
        private var monitor: Any?

        init(
            selectedTrackIds: Binding<Set<String>>,
            selectionAnchorId: Binding<String?>,
            onArrowNavigate: ((String, Int) -> Void)?,
            onSelectionChanged: @escaping () -> Void
        ) {
            _selectedTrackIds = selectedTrackIds
            _selectionAnchorId = selectionAnchorId
            self.onArrowNavigate = onArrowNavigate
            self.onSelectionChanged = onSelectionChanged
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
            let primary = selectionAnchorId ?? selectedTrackIds.sorted().first
            let currentIdx: Int
            if let id = primary, let idx = trackIds.firstIndex(of: id) {
                currentIdx = idx
            } else {
                let firstId = trackIds[0]
                NSApp.keyWindow?.makeFirstResponder(nil)
                selectedTrackIds = [firstId]
                selectionAnchorId = firstId
                onSelectionChanged()
                onArrowNavigate?(firstId, delta)
                return true
            }
            let next = max(0, min(trackIds.count - 1, currentIdx + delta))
            guard next != currentIdx else { return false }
            let nextId = trackIds[next]
            NSApp.keyWindow?.makeFirstResponder(nil)
            selectedTrackIds = [nextId]
            selectionAnchorId = nextId
            onSelectionChanged()
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
