//
//  LibraryView.swift
//  Dub
//
//  M11d.1 — Library browser shell. Replaces the M10.5b file-mode
//  browser in the LIBRARY region of the Performance View.
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
import Combine
import SwiftUI
import UniformTypeIdentifiers

import DubCore

// MARK: - LibraryTrack convenience conformances

/// UniFFI's generated `LibraryTrack` already carries a `String id`
/// field — Swift's `Identifiable` conformance is a one-liner. Used
/// by SwiftUI `Table` to track row identity across sorts.
///
/// `@retroactive` is the Swift 6 opt-in for declaring a
/// conformance on a type from another module to a protocol from
/// yet another module. The Apple shell intentionally owns this
/// conformance rather than the `DubCore` (UniFFI-generated)
/// module because:
///
/// 1. UniFFI's binding generator doesn't expose hooks to add
///    Swift protocol conformances to its generated structs, so
///    the conformance can't live next to the declaration without
///    forking the generated source.
/// 2. `Identifiable` is a SwiftUI-shaped affordance, not a core
///    library invariant — keeping it in the UI module mirrors
///    how the rest of `LibraryView`'s sort-key extensions sit.
///
/// If `dub-library`'s FFI ever ships its own `Identifiable`
/// conformance we'll have a duplicate-conformance compile error
/// that's straightforward to resolve by deleting this extension;
/// `@retroactive` here is exactly the contract the Swift 6
/// compiler asks us to make explicit.
extension LibraryTrack: @retroactive Identifiable {}

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
    /// Manual-order rank inside the crate this row was listed from.
    /// `crateOrdinal` is `nil` for every non-crate listing; folding
    /// those to `UInt32.max` keeps the comparator total even though
    /// the `#` column is never rendered outside a crate view. Sorting
    /// ascending on this key reproduces the FFI's ordinal order — the
    /// canonical manual order.
    var crateOrderSortKey: UInt32 { crateOrdinal ?? UInt32.max }
}

// MARK: - Configurable library columns (PRD §8.5.3.1 lite)

/// Display + sort identity for a library browser column. Artist
/// and title are always shown; the trailing set is user-configurable
/// via header right-click and persisted in `@AppStorage`.
private enum LibraryColumnField: String, CaseIterable, Identifiable {
    /// Manual-order rank column (`#`). Injected as a fixed leading
    /// column only while a Dub crate is selected; never part of the
    /// user-configurable / persisted column set, so it stays out of
    /// `fixedPrefix`, `defaultTrailing`, and `configurable`. Sorting
    /// it ascending *is* the crate's manual order, and that state is
    /// what gates drag-to-reorder (`isCrateManualOrder`).
    case crateOrder
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
        case .artist, .title, .source, .versionTokens, .crateOrder:
            return "Library"
        case .album, .genre, .year, .comment, .composer, .trackNumber:
            return "ID3 metadata"
        case .duration, .bpm, .key:
            return "Analysis"
        }
    }

    var headerLabel: String {
        switch self {
        case .crateOrder: return "#"
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
    /// M11d-history — this app run's set list, newest-first, with
    /// the "← from <track>" transition annotation on rows that
    /// were mixed into from the other deck.
    case sessionHistory
    case justImported
    /// A user-created Dub crate (M11d-next, PRD §8.5.1). Carries the
    /// crate id only; the display name + track count are looked up
    /// live from `libraryModel.crates` so a rename doesn't strand a
    /// stale label in the selection state.
    case dubCrate(id: Int64)
    /// A top-level imported-source node (Serato / Traktor / iTunes):
    /// selecting it lists "all tracks from this source". Its crates /
    /// playlists render as children rows.
    case importedSource(kind: ImportedSourceKind)
    /// A crate / playlist under an imported source (read-only). Carries
    /// the `imported_crates.id`; the display name is looked up live from
    /// `libraryModel.importedSources`, the same way `dubCrate` does.
    case importedCrate(id: Int64)
    case realRecordsPlaceholder

    var id: Self { self }

    var label: String {
        switch self {
        case .allTracks:                   return "All Tracks"
        case .recentlyPlayed:              return "Recently Played"
        case .sessionHistory:              return "Session History"
        case .justImported:                return "Just Imported"
        case .dubCrate:                    return "Crate"
        case .importedSource(let kind):    return kind.label
        case .importedCrate:               return "Playlist"
        case .realRecordsPlaceholder:      return "Real Records"
        }
    }

    var systemImage: String {
        switch self {
        case .allTracks:                   return "music.note.list"
        case .recentlyPlayed:              return "clock.arrow.circlepath"
        case .sessionHistory:              return "calendar.day.timeline.left"
        case .justImported:                return "tray.and.arrow.down"
        case .dubCrate:                    return "square.stack.fill"
        case .importedSource(let kind):    return kind.systemImage
        case .importedCrate:               return "list.bullet"
        case .realRecordsPlaceholder:      return "opticaldisc"
        }
    }

    /// The backing crate id when this source is a Dub crate.
    var crateId: Int64? {
        if case let .dubCrate(id) = self { return id }
        return nil
    }

    /// The backing imported-crate id when this source is an imported
    /// crate / playlist (read-only mirror), else `nil`.
    var importedCrateId: Int64? {
        if case let .importedCrate(id) = self { return id }
        return nil
    }

    /// `false` for the v1.0 placeholders that render disabled.
    var isAvailable: Bool {
        switch self {
        case .allTracks, .recentlyPlayed, .sessionHistory, .justImported: return true
        case .dubCrate, .importedSource, .importedCrate: return true
        default:                                         return false
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
        // A Dub crate's ordinal order is user-defined (it's a
        // playlist); the default title sort would clobber the order
        // the user just dragged into place, exactly like the smart
        // crates' recency order. Session History is the set's
        // play order — same contract.
        case .recentlyPlayed, .sessionHistory, .justImported, .dubCrate, .importedCrate:
            return true
        default: return false
        }
    }

    /// Per §8.5.1, the sidebar groups sections under a heading.
    var group: String {
        switch self {
        case .allTracks:
            return "Library"
        case .recentlyPlayed, .sessionHistory, .justImported:
            return "Smart Crates"
        case .dubCrate:
            return "Dub Crates"
        case .importedSource, .importedCrate:
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
    /// Library / analysis / relocate state — split out of
    /// `WaveformAppModel` so library mutations don't invalidate
    /// `PerformanceView`. The parent (PerformanceView) passes
    /// the app model's `libraryModel` here. Every library-side
    /// field LibraryView observes (`libraryIsOpen`,
    /// `libraryTrackCount`, `lastImportSummary`,
    /// `libraryImportInProgress`, `selectedLibraryTrackId`,
    /// `selectedLibraryTrack`, `volumeReachability`,
    /// `missingTrackCount`, `analysisGeneration`,
    /// `libraryRowAnalysisUpdate`, `analysisInFlightCount`,
    /// `analysisBatchCompleted`, `analysisBatchTotal`,
    /// `relocateInProgress`, `lastRelocateMatches`,
    /// `lastRelocateUnmatched`) is reached through this object.
    /// LibraryView's body therefore only re-evaluates on library
    /// activity; the `WaveformAppModel` observation above stays
    /// because LibraryView also reads cross-cutting fields like
    /// `engine`, `isRunning`, and `browserSelection` from there.
    @ObservedObject var libraryModel: LibraryAppModel

    /// Currently selected source-tree node. Drives which query the
    /// track list runs against.
    @State private var selectedSource: LibrarySource = .allTracks

    /// Imported-source nodes (Serato / Traktor / Apple Music) the user
    /// has collapsed in the sidebar. Membership = collapsed, so the
    /// default (empty) shows every source expanded with its playlist
    /// tree visible. Toggled by the disclosure triangle on each source
    /// row; selecting the row still lists that source's tracks.
    @State private var collapsedImportedSources: Set<ImportedSourceKind> = []

    /// M11d-history — per-row "← from <track>" annotations for the
    /// Session History source, keyed by track id. Populated only by
    /// the `.sessionHistory` query branch (empty for every other
    /// source), so a non-empty lookup is also the "render the
    /// annotation" gate.
    @State private var sessionFromTitles: [String: String] = [:]

    /// M11d-history — reveal staged across an async listing refresh.
    /// Set when the deck-header hint targets a track that isn't in
    /// the current listing (crate open, search active); consumed —
    /// and always cleared — by the next `refreshTracks` completion.
    @State private var pendingRevealTrackId: String? = nil

    /// M11d-next — id of the crate currently in inline-rename mode
    /// (its sidebar row shows a `TextField` instead of a label), or
    /// `nil` when no crate is being renamed. Set when the user picks
    /// "Rename" from a crate's context menu or right after creating
    /// a crate.
    @State private var renamingCrateId: Int64?

    /// Working text for the in-progress inline rename. Committed on
    /// submit / focus-loss, discarded on Escape.
    @State private var crateRenameText: String = ""

    /// Focus binding for the inline-rename text field so a fresh
    /// create / rename drops the caret straight into the field.
    @FocusState private var crateRenameFocused: Bool

    /// M11d-next — crate id currently under a drag (drop-to-add
    /// highlight), or `nil`. Drives the sidebar row's accent ring.
    @State private var crateDropTargetId: Int64?


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

    /// Selection state owned by the view, deliberately stored on
    /// `@State` rather than `@StateObject` / `@ObservedObject`.
    /// `@State` holds the class reference for the lifetime of the
    /// view but does **not** subscribe to its
    /// `objectWillChange` publisher, so mutating
    /// `rowSelection.selectedTrackIds` or
    /// `rowSelection.selectionAnchorId` no longer triggers a
    /// `LibraryView.body` re-eval. See
    /// `LibraryRowSelection`'s header for the full rationale —
    /// this is the lever that takes the row-click main-thread
    /// cost off the playing waveform. Subviews that need to
    /// observe selection changes (e.g. the AppKit selection
    /// layer) subscribe to `rowSelection` directly via Combine.
    @State private var rowSelection = LibraryRowSelection()

    /// Primary selected row for Space-load + model sync.
    private var primarySelectedTrackId: String? {
        if let anchor = rowSelection.selectionAnchorId,
           rowSelection.selectedTrackIds.contains(anchor)
        {
            return anchor
        }
        return rowSelection.selectedTrackIds.sorted().first
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
            // Pick the default sort for the new source. The user can
            // still click any column header afterwards to override it.
            if newSource.crateId != nil {
                // A crate opens in its manual order: sort by the `#`
                // column ascending. This is the only sort state that
                // enables drag-to-reorder; the user can click any
                // other header to view the crate sorted (drag then
                // disables) and click `#` to come back.
                activeSortColumn = .crateOrder
                sortAscending = true
                sortOrder = [KeyPathComparator(\LibraryTrack.crateOrderSortKey, order: .forward)]
            } else if newSource.preservesNaturalOrder {
                // Smart crates with a meaningful natural order
                // (Recently Played, Just Imported) start out
                // *unsorted* so the FFI's recency order survives.
                activeSortColumn = nil
                sortOrder = []
            } else {
                // `allTracks` falls back to title-ascending.
                activeSortColumn = .title
                sortAscending = true
                sortOrder = [KeyPathComparator(\LibraryTrack.titleSortKey, order: .forward)]
            }
            refreshTracks()
        }
        .onChange(of: libraryModel.libraryIsOpen) { _ in
            refreshTracks()
        }
        .onChange(of: libraryModel.libraryTrackCount) { _ in
            // Track count bumped → either an import just landed
            // or another window inserted rows. Refresh the visible
            // listing so the user sees the new rows immediately.
            // B-8 — preserve the user's highlighted row: the bump
            // can fire mid-session (background insert, another
            // window) and must not yank the selection the user is
            // about to Space-load. The rows the user sees haven't
            // moved; new rows just append.
            refreshTracks(preserveSelection: true)
        }
        .task(id: searchText) {
            // B-9 — debounce FTS5 queries: wait 250 ms after the
            // last keystroke before re-querying, so a fast-typed
            // word fires one query + one list rebuild instead of
            // one per character. Changing `searchText` cancels the
            // in-flight task automatically, so only the final
            // keystroke survives the sleep.
            try? await Task.sleep(nanoseconds: 250_000_000)
            guard !Task.isCancelled else { return }
            refreshTracks()
        }
        .onChange(of: libraryModel.analysisGeneration) { _ in
            // M11c.1 — a deck-load or batch analyze finished and
            // wrote at least one new grid. Re-fetch the current
            // listing so the BPM column lights up and the dim
            // overlay drops on the rows that just transitioned.
            // Preserve selection — the rows haven't moved, the
            // user shouldn't lose their Space-load target.
            refreshTracks(preserveSelection: true)
        }
        .onChange(of: libraryModel.libraryRowAnalysisUpdate) { update in
            guard let update else { return }
            applyAnalysisUpdate(update)
        }
        .onChange(of: libraryModel.revealTrackRequest) { request in
            guard let request else { return }
            revealTrack(request.trackId)
        }
        .onChange(of: libraryModel.crateContentGeneration) { _ in
            // A crate's membership / order changed (add / remove /
            // reorder / delete). Only the currently-open crate view
            // needs to re-fetch; other sources are unaffected.
            guard selectedSource.crateId != nil else { return }
            refreshTracks(preserveSelection: true)
        }
        // M11d.6 round 3 — the previously-present
        // `.onChange(of: sortOrder) { _ in recomputeSortedTracks() }`
        // turned every sidebar swap into a **double** rootView
        // swap. The first one fires when `.onChange(of:
        // selectedSource)` flips `sortOrder` (resort the OLD
        // tracks buffer); the second fires ~50 ms later when the
        // FFI completes and `refreshTracks` lands the NEW rows.
        // Both swaps go through `NSHostingView.rootView = rows`
        // on a 5 000-row `LazyVStack`, which is the dominant
        // main-thread block on a sidebar click. We now call
        // `recomputeSortedTracks()` explicitly at every site
        // that mutates `sortOrder` WITHOUT a paired
        // `refreshTracks()` (the column-header sort toggles and
        // the hide-column path); the sidebar-swap path skips
        // the intermediate recompute and lets `refreshTracks`'s
        // own MainActor block do the single combined swap when
        // the new rows arrive.
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            handleLibraryDrop(providers)
        }
        .background {
            LibraryTextFocusDismissMonitor()
        }
        .sheet(isPresented: $showRelocateSheet) {
            RelocateSheet(model: model,
                          libraryModel: libraryModel,
                          isPresented: $showRelocateSheet)
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
            Text("\(libraryModel.libraryTrackCount)")
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
                    entries: [.recentlyPlayed, .sessionHistory, .justImported])
                dubCratesSection
                importedSourcesSection
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

    // MARK: - Dub Crates section (M11d-next, PRD §8.5.1)

    /// The editable "Dub Crates" section: a header carrying a `+`
    /// create affordance, then one row per crate (dynamic from
    /// `libraryModel.crates`). Each row is a drop target for tracks,
    /// supports inline rename, and carries a Rename / Delete context
    /// menu. Empty-state shows a faint hint so the section never
    /// reads as broken when the user has no crates yet.
    @ViewBuilder
    private var dubCratesSection: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("DUB CRATES")
                .font(DubFont.caps)
                .tracking(1.0)
                .foregroundStyle(DubColor.textTertiary)
            Spacer(minLength: 0)
            Button {
                createCrateAndRename()
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(DubColor.textSecondary)
            }
            .buttonStyle(.plain)
            .disabled(!libraryModel.libraryIsOpen)
            .help("New crate")
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.top, DubSpacing.sm)
        .padding(.bottom, 2)

        if libraryModel.crates.isEmpty {
            Text("No crates yet — drag tracks here.")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textPlaceholder)
                .padding(.horizontal, DubSpacing.lg)
                .padding(.vertical, DubSpacing.xs)
        } else {
            ForEach(libraryModel.crates, id: \.id) { crate in
                crateRow(crate)
            }
        }
    }

    private func crateRow(_ crate: LibraryCrate) -> some View {
        let source = LibrarySource.dubCrate(id: crate.id)
        let isSelected = selectedSource == source
        let isDropTarget = crateDropTargetId == crate.id
        let isRenaming = renamingCrateId == crate.id
        return HStack(spacing: DubSpacing.sm) {
            Image(systemName: source.systemImage)
                .frame(width: 16)
                .foregroundStyle(isSelected ? DubColor.textPrimary : DubColor.textSecondary)
            if isRenaming {
                TextField("Crate name", text: $crateRenameText)
                    .textFieldStyle(.plain)
                    .font(DubFont.body)
                    .focused($crateRenameFocused)
                    .onSubmit { commitCrateRename(crate.id) }
                    .onExitCommand { cancelCrateRename() }
                    .onChange(of: crateRenameFocused) { focused in
                        if !focused && renamingCrateId == crate.id {
                            commitCrateRename(crate.id)
                        }
                    }
            } else {
                Text(crate.name)
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 0)
                Text("\(crate.trackCount)")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .monospacedDigit()
            }
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.xs)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? DubColor.surface2 : Color.clear)
        .overlay {
            if isDropTarget {
                RoundedRectangle(cornerRadius: 4)
                    .stroke(DubColor.deckATint, lineWidth: 1.5)
                    .padding(.horizontal, DubSpacing.sm)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            guard !isRenaming else { return }
            searchFocused = false
            NSApp.keyWindow?.makeFirstResponder(nil)
            selectedSource = source
        }
        .onDrop(of: [.fileURL], isTargeted: dropTargetBinding(for: crate.id)) { providers in
            handleCrateDrop(providers, crateId: crate.id)
        }
        .contextMenu {
            Button("Rename") { beginCrateRename(crate) }
            Button("Delete", role: .destructive) { confirmDeleteCrate(crate) }
        }
    }

    private func dropTargetBinding(for crateId: Int64) -> Binding<Bool> {
        Binding(
            get: { crateDropTargetId == crateId },
            set: { active in crateDropTargetId = active ? crateId : nil }
        )
    }

    // MARK: - Imported Sources section (Serato / Traktor / iTunes)

    /// The read-only "Imported Sources" section: one top-level row per
    /// imported source that has data, with its crate / playlist tree
    /// indented beneath. Hidden entirely when nothing is imported, so the
    /// sidebar doesn't carry a dead heading. Selecting a source row lists
    /// all of that source's tracks; selecting a crate lists that crate's
    /// members (read-only — no rename / drop / reorder, unlike Dub crates).
    @ViewBuilder
    private var importedSourcesSection: some View {
        if !libraryModel.importedSources.isEmpty {
            Text("IMPORTED SOURCES")
                .font(DubFont.caps)
                .tracking(1.0)
                .foregroundStyle(DubColor.textTertiary)
                .padding(.horizontal, DubSpacing.lg)
                .padding(.top, DubSpacing.sm)
                .padding(.bottom, 2)
            ForEach(libraryModel.importedSources) { group in
                importedSourceRow(group)
                if !collapsedImportedSources.contains(group.kind) {
                    ForEach(flattenedImportedCrates(group.crates), id: \.crate.id) { node in
                        importedCrateRow(node.crate, depth: node.depth)
                    }
                }
            }
        }
    }

    /// Flatten an imported source's crate tree (flat list + `parentId`,
    /// document order) into rows tagged with nesting depth, so a folder's
    /// children render indented under it. Roots first, depth-first.
    private func flattenedImportedCrates(
        _ crates: [LibraryImportedCrate]
    ) -> [(crate: LibraryImportedCrate, depth: Int)] {
        var childrenByParent: [Int64: [LibraryImportedCrate]] = [:]
        var roots: [LibraryImportedCrate] = []
        for c in crates {
            if let parent = c.parentId {
                childrenByParent[parent, default: []].append(c)
            } else {
                roots.append(c)
            }
        }
        var out: [(crate: LibraryImportedCrate, depth: Int)] = []
        func visit(_ c: LibraryImportedCrate, _ depth: Int) {
            out.append((c, depth))
            for child in childrenByParent[c.id] ?? [] {
                visit(child, depth + 1)
            }
        }
        for root in roots { visit(root, 0) }
        return out
    }

    private func importedSourceRow(_ group: ImportedSourceGroup) -> some View {
        let source = LibrarySource.importedSource(kind: group.kind)
        let isSelected = selectedSource == source
        let canCollapse = !group.crates.isEmpty
        let isCollapsed = collapsedImportedSources.contains(group.kind)
        return HStack(spacing: DubSpacing.sm) {
            // Disclosure triangle: only a source with playlists/crates
            // has anything to collapse. Sources without keep an empty
            // slot of the same width so their icons stay aligned.
            Group {
                if canCollapse {
                    Button {
                        toggleImportedSourceCollapsed(group.kind)
                    } label: {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(DubColor.textTertiary)
                            .rotationEffect(.degrees(isCollapsed ? 0 : 90))
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help(isCollapsed ? "Show playlists" : "Hide playlists")
                }
            }
            .frame(width: 10)
            Image(systemName: group.kind.systemImage)
                .frame(width: 16)
                .foregroundStyle(isSelected ? DubColor.textPrimary : DubColor.textSecondary)
            Text(group.kind.label)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            Text("\(group.trackCount)")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .monospacedDigit()
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.xs)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? DubColor.surface2 : Color.clear)
        .contentShape(Rectangle())
        .onTapGesture {
            searchFocused = false
            NSApp.keyWindow?.makeFirstResponder(nil)
            selectedSource = source
        }
    }

    /// Toggle whether an imported source's playlist tree is shown.
    private func toggleImportedSourceCollapsed(_ kind: ImportedSourceKind) {
        if collapsedImportedSources.contains(kind) {
            collapsedImportedSources.remove(kind)
        } else {
            collapsedImportedSources.insert(kind)
        }
    }

    private func importedCrateRow(_ crate: LibraryImportedCrate, depth: Int) -> some View {
        let source = LibrarySource.importedCrate(id: crate.id)
        let isSelected = selectedSource == source
        return HStack(spacing: DubSpacing.sm) {
            Image(systemName: "list.bullet")
                .frame(width: 16)
                .foregroundStyle(isSelected ? DubColor.textPrimary : DubColor.textSecondary)
            Text(crate.name)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            Text("\(crate.trackCount)")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .monospacedDigit()
        }
        .padding(.leading, DubSpacing.lg + CGFloat(depth + 1) * DubSpacing.md)
        .padding(.trailing, DubSpacing.lg)
        .padding(.vertical, DubSpacing.xs)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? DubColor.surface2 : Color.clear)
        .contentShape(Rectangle())
        .onTapGesture {
            searchFocused = false
            NSApp.keyWindow?.makeFirstResponder(nil)
            selectedSource = source
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
                    .help("Coming soon.")
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

            Menu {
                Button {
                    presentImportFolderPicker()
                } label: {
                    Label("Folder…", systemImage: "folder")
                }
                Divider()
                Button {
                    presentImportedSourcePicker(.serato)
                } label: {
                    Label("Serato Library…", systemImage: ImportedSourceKind.serato.systemImage)
                }
                Button {
                    presentImportedSourcePicker(.traktor)
                } label: {
                    Label("Traktor collection.nml…", systemImage: ImportedSourceKind.traktor.systemImage)
                }
                Button {
                    presentImportedSourcePicker(.itunes)
                } label: {
                    Label("iTunes Library.xml…", systemImage: ImportedSourceKind.itunes.systemImage)
                }
            } label: {
                Label("Import", systemImage: "tray.and.arrow.down")
            }
            .menuIndicator(.visible)
            .fixedSize()
            .controlSize(.small)
            .disabled(!libraryModel.libraryIsOpen || libraryModel.libraryImportInProgress)
            .help(
                libraryModel.libraryImportInProgress
                    ? "An import is already running."
                    : "Import a folder, or a Serato / Traktor / iTunes library.")
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.sm)
        .background(DubColor.surface2)
    }

    @ViewBuilder
    private var trackListContainer: some View {
        if !libraryModel.libraryIsOpen {
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
    ///
    /// Memoised in `@State` and refreshed via
    /// `recomputeSortedTracks(...)` whenever `tracks` or
    /// `sortOrder` changes. Pre-memoisation this was a computed
    /// property that re-sorted on every body re-eval — and was
    /// read **three times** per body re-eval (`trackList`'s
    /// `trackIds` / `visibleTracks` props plus `trackRowsStack`'s
    /// `ForEach`). With 5 000 rows under a non-empty comparator
    /// that's ~3–6 ms of pure CPU sort cost the LibraryView body
    /// had to pay every time the user clicked a row (state
    /// changes → SwiftUI body invalidation → triple-sort), which
    /// stretched the main-runloop tick wide enough to make the
    /// playing waveform skip a vsync.
    @State private var sortedTracks: [LibraryTrack] = []

    /// Memoised cache of `sortedTracks.map(\.id)`. Used by both
    /// `LibraryTableScrollContainer.trackIds` (read twice per
    /// body re-eval, once in `trackList` and once in
    /// `selectionMonitor`) and `selectRange`. Refreshed alongside
    /// `sortedTracks` in `recomputeSortedTracks()`. Same
    /// motivation as `sortedTracks`'s memoisation — avoids a
    /// 5 000-element `[String]` allocation per body re-eval that
    /// was visible in the main-thread cost of every library
    /// click.
    @State private var sortedTrackIds: [String] = []

    /// Scrollable track list. Uses a full-row `onTapGesture` +
    /// AppKit `onDrag` row pattern because SwiftUI `Table` was dropping ~4/5
    /// clicks and turning drags outside the Title column into
    /// arrow-key-style selection changes.
    private var trackList: some View {
        VStack(spacing: 0) {
            LibraryTableScrollContainer(
                tableWidth: tableContentWidth,
                columnOrderKey: displayedColumns.map(\.rawValue).joined(separator: ","),
                headerStateKey: columnReorderHeaderStateKey,
                tracksContentRevision: tracksContentRevision,
                rowSelection: rowSelection,
                header: AnyView(trackListHeader),
                rows: AnyView(trackRowsStack),
                trackIds: sortedTrackIds,
                visibleTracks: sortedTracks,
                menu: LibraryTableMenu(
                    analysisBatchInProgress: libraryModel.analysisBatchTotal > 0,
                    onAnalyzeRequested: { ids in
                        Task { @MainActor in
                            await model.analyzeTracks(ids)
                        }
                    },
                    onSetGridLocked: { trackId, locked in
                        Task { @MainActor in
                            await model.setGridLocked(trackId: trackId, locked: locked)
                        }
                    },
                    crateId: selectedSource.crateId,
                    onCrateRemove: { ids in
                        guard let crateId = selectedSource.crateId else { return }
                        for id in ids {
                            model.removeTrackFromCrate(crateId, trackId: id)
                        }
                    },
                    // Move… is offered only in manual order, mirroring
                    // the drag-reorder gate. With a foreign column sort
                    // active, "Move Up" against the sorted view wouldn't
                    // map to a persistent manual position, so the items
                    // are withheld (Remove from Crate stays available).
                    onCrateMove: isCrateManualOrder
                        ? { trackId, move in
                            guard let crateId = selectedSource.crateId else { return }
                            let reordered = reorderedCrateIds(
                                moving: trackId, move: move, current: sortedTrackIds)
                            model.setCrateOrder(crateId, orderedTrackIds: reordered)
                        }
                        : nil),
                scroll: LibraryTableScroll(
                    scrollToTrackId: keyboardScrollTarget,
                    scrollDelta: keyboardScrollDelta,
                    onScrollHandled: { keyboardScrollTarget = nil }),
                crateReorderEnabled: isCrateManualOrder,
                onCrateReorder: { ids, slot in
                    performCrateReorder(draggedIds: ids, toSlot: slot)
                }
            )
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(
            LibraryArrowKeyView(
                rowSelection: rowSelection,
                trackIds: sortedTrackIds,
                onArrowNavigate: { trackId, delta in
                    keyboardScrollDelta = delta
                    keyboardScrollTarget = trackId
                },
                onSelectionChanged: syncModelPrimarySelection)
            .allowsHitTesting(false)
        )
        // M11d.6 round 4 — the previously-present
        // `.onChange(of: selectedTrackIds)` handler was deleted
        // alongside the migration of selection to
        // `LibraryRowSelection`. With selection living on a
        // non-observed class reference, `LibraryView.body` no
        // longer re-evaluates on selection changes, so
        // `.onChange` would never fire. `syncModelPrimarySelection()`
        // is now called explicitly at every write site
        // (`handleRowClick`, `selectRange`, `navigateToSibling`,
        // `refreshTracks`, and the arrow-key Coordinator) so the
        // model-side `librarySelection` stays in lock-step.
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
        let columnSum = displayedColumns.map { columnWidth($0) }.reduce(0, +)
        return 36 + columnSum + DubSpacing.lg * 2
    }

    private var trackListHeader: some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: 36)
                .overlay(alignment: .trailing) {
                    columnHeaderDivider
                }
            ForEach(displayedColumns) { field in
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

    /// `true` while a manual Dub crate is the selected source.
    private var isCrateView: Bool { selectedSource.crateId != nil }

    /// Columns actually rendered. A crate view pins the `#`
    /// (manual-order) column ahead of the user-configurable set;
    /// every other source renders `visibleColumns` unchanged. Kept
    /// separate from `visibleColumns` so the `#` column never leaks
    /// into the persisted column order / picker / header-reorder
    /// machinery.
    private var displayedColumns: [LibraryColumnField] {
        isCrateView ? [.crateOrder] + visibleColumns : visibleColumns
    }

    /// `true` when the open crate is showing its manual order — i.e.
    /// no foreign column sort is active. Manual order is the only
    /// state where drag-to-reorder has a persistent meaning, so this
    /// gates both the reorder drag payload and the drop targets.
    /// Both the explicit `#`-ascending sort and the unsorted FFI
    /// order (`activeSortColumn == nil`) count as manual order; they
    /// render identically because `crateOrdinal` is dense `0..n`.
    private var isCrateManualOrder: Bool {
        guard isCrateView else { return false }
        if activeSortColumn == nil { return true }
        return activeSortColumn == .crateOrder && sortAscending
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
                // Replaces the removed `.onChange(of: sortOrder)`
                // handler — see the comment block at the
                // `selectedSource` onChange for why we route the
                // resort manually now.
                recomputeSortedTracks()
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
                // The `#` column is pinned; it can't be a reorder
                // source (and it's not in the persisted order anyway).
                guard field != .crateOrder else { return }
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
        // M11d.6 round 3 — replaces the removed
        // `.onChange(of: sortOrder)` handler. Refresh
        // `sortedTracks` / `sortedTrackIds` exactly once on the
        // tick the user clicked the header, instead of as a
        // side effect of the SwiftUI observation chain (which
        // also fired on sidebar swaps and caused the
        // double rootView swap).
        recomputeSortedTracks()
    }

    private func syncSortOrderFromHeader() {
        guard let column = activeSortColumn else {
            sortOrder = []
            return
        }
        let order: SortOrder = sortAscending ? .forward : .reverse
        switch column {
        case .crateOrder:
            sortOrder = [KeyPathComparator(\.crateOrderSortKey, order: order)]
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
        case .crateOrder: return 40
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
        case .crateOrder:
            Text(track.crateOrdinal.map { String($0 + 1) } ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textTertiary)
                .monospacedDigit()
                .frame(maxWidth: .infinity, alignment: .trailing)
        case .artist:
            Text(track.artist ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
                .truncationMode(.middle)
        case .title:
            HStack(spacing: DubSpacing.sm) {
                Text(displayTitle(track))
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                // M11d-history: Session History rows show which
                // track this one was mixed in from. The dict is
                // only populated by that source, so other sources
                // never pay the extra view.
                if let from = sessionFromTitles[track.id] {
                    Text("← from \(from)")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(-1)
                }
            }
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
            ForEach(displayedColumns) { field in
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
            view.onDrag { [rowSelection] in
                if !rowSelection.selectedTrackIds.contains(track.id) {
                    rowSelection.selectedTrackIds = [track.id]
                    rowSelection.selectionAnchorId = track.id
                    // The previous `.onChange(of: selectedTrackIds)`
                    // ran `syncModelPrimarySelection()` here, but
                    // that handler is gone now that selection lives
                    // on the non-observed `rowSelection`. The drag
                    // path is rare enough that we just skip the
                    // model sync on hand-off — the deck-load
                    // pathway re-reads selection on its own.
                }
                return makeRowDragProvider(for: track, dragURL: dragURL!)
            }
        }
        // The reorder DROP target + the insertion line are handled in
        // AppKit on `LibraryDocumentWrapper`, not here. A per-row
        // SwiftUI `.onDrop` cannot paint live feedback in this table:
        // the rows are hosted in an `NSHostingView` whose `rootView`
        // is only re-assigned on a track/column/width change (the
        // perf optimization that keeps sidebar swaps cheap), so any
        // `@State` the drop delegate flips never repaints mid-drag.
        // Doing it in AppKit (like the selection layer) gives a live
        // insertion line and reliable end-of-list handling.
        .onTapGesture(count: 1) {
            handleRowClick(track)
        }
        // Right-click menu is built by AppKit in
        // `LibraryDocumentWrapper.menu(for:)` instead of the
        // SwiftUI `.contextMenu` modifier. SwiftUI's `.contextMenu`
        // closure is empirically captured at the moment the row is
        // first attached and is NOT re-evaluated when selection
        // changes — and as of M11d.6 round 4 selection mutations
        // don't even fire a `LibraryView.body` re-eval at all
        // (selection lives on `rowSelection`, a `LibraryRowSelection`
        // held on `@State` for ownership without observation).
        // Building the menu in AppKit reads the live selection set
        // on `rowSelection.selectedTrackIds` at right-click time
        // (see `LibraryTableScrollContainer.Coordinator`'s
        // computed `menuSelectedTrackIds` property), so the
        // multi-select label is always accurate even though the
        // SwiftUI render path is skipped.
    }

    /// In-process drag type carrying the crate-reorder payload. Kept
    /// distinct from `public.file-url` (the deck/sidebar drop) so the
    /// in-list reorder drop targets only ever react to a row drag
    /// that originated inside the open crate, never to a Finder file
    /// drag. The payload is a newline-joined list whose first line is
    /// the `DUBCRATE` sentinel followed by the dragged track ids in
    /// visual order. A custom reverse-DNS identifier is fine for an
    /// in-process drag without a UTI declaration in Info.plist.
    fileprivate static let crateReorderType =
        UTType(exportedAs: "com.dub.crate-track-order")

    /// Builds the drag item provider for a track row. It always vends
    /// the file URL (deck-load + add-to-crate drops), and, while the
    /// open crate is in manual order, *also* vends the reorder payload
    /// so the same drag can either load a deck or reorder in place
    /// depending on where it lands.
    private func makeRowDragProvider(for track: LibraryTrack, dragURL: URL) -> NSItemProvider {
        let provider = NSItemProvider(object: dragURL as NSURL)
        guard isCrateManualOrder else { return provider }
        let ids = orderedSelectedIds(includingPrimary: track.id)
        let payload = (["DUBCRATE"] + ids).joined(separator: "\n")
        provider.registerDataRepresentation(
            forTypeIdentifier: Self.crateReorderType.identifier,
            visibility: .ownProcess
        ) { completion in
            completion(Data(payload.utf8), nil)
            return nil
        }
        return provider
    }

    /// The dragged track ids in current visual order. A multi-row
    /// selection moves as a contiguous block; a drag on an unselected
    /// row (or a single selection) carries just that row.
    private func orderedSelectedIds(includingPrimary primary: String) -> [String] {
        let selected = rowSelection.selectedTrackIds
        if selected.contains(primary), selected.count > 1 {
            return sortedTrackIds.filter { selected.contains($0) }
        }
        return [primary]
    }

    /// Commits a drag-reorder to a 0-based insertion slot in `0…count`
    /// (the value the AppKit document wrapper computes from the drop's
    /// y-position; `count` means "after the last row"). Removes the
    /// dragged block from the current manual order and reinserts it at
    /// the slot, anchored to the first non-dragged row at or after it
    /// so dropping into the middle of a multi-selection still lands
    /// predictably. The `crateContentGeneration` bump that
    /// `setCrateOrder` triggers re-fetches the crate so the `#` ranks
    /// resettle to a dense `0..n`.
    private func performCrateReorder(draggedIds: [String], toSlot slot: Int) {
        guard let crateId = selectedSource.crateId else { return }
        let draggedSet = Set(draggedIds)
        guard !draggedSet.isEmpty else { return }
        let current = sortedTrackIds

        // The row that currently occupies `slot` (skipping any dragged
        // rows) becomes the insertion anchor; nil means append.
        var anchorId: String?
        var probe = max(0, min(slot, current.count))
        while probe < current.count {
            if !draggedSet.contains(current[probe]) {
                anchorId = current[probe]
                break
            }
            probe += 1
        }

        var order = current
        order.removeAll { draggedSet.contains($0) }
        let insertAt: Int
        if let anchorId, let idx = order.firstIndex(of: anchorId) {
            insertAt = idx
        } else {
            insertAt = order.count
        }
        order.insert(contentsOf: draggedIds, at: insertAt)
        guard order != current else { return }
        model.setCrateOrder(crateId, orderedTrackIds: order)
    }

    /// Decode a `DUBCRATE`-tagged reorder payload off an item
    /// provider and hand the dragged track ids back on the main
    /// actor. Item-provider loads complete on an arbitrary queue, so
    /// the completion hops to the main thread before mutating order.
    private func loadCrateReorderPayload(
        from provider: NSItemProvider,
        then apply: @escaping ([String]) -> Void
    ) {
        provider.loadDataRepresentation(
            forTypeIdentifier: Self.crateReorderType.identifier
        ) { data, _ in
            guard let data,
                  let text = String(data: data, encoding: .utf8)
            else { return }
            let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
            guard lines.first == "DUBCRATE" else { return }
            let ids = Array(lines.dropFirst()).filter { !$0.isEmpty }
            guard !ids.isEmpty else { return }
            DispatchQueue.main.async { apply(ids) }
        }
    }

    private func handleRowClick(_ track: LibraryTrack) {
        searchFocused = false
        NSApp.keyWindow?.makeFirstResponder(nil)
        let flags = NSEvent.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if flags.contains(.command) {
            if rowSelection.selectedTrackIds.contains(track.id) {
                rowSelection.selectedTrackIds.remove(track.id)
                if rowSelection.selectionAnchorId == track.id {
                    rowSelection.selectionAnchorId =
                        rowSelection.selectedTrackIds.sorted().first
                }
            } else {
                rowSelection.selectedTrackIds.insert(track.id)
                rowSelection.selectionAnchorId = track.id
            }
        } else if flags.contains(.shift) {
            let anchor = rowSelection.selectionAnchorId
                ?? primarySelectedTrackId
                ?? track.id
            selectRange(from: anchor, to: track.id)
            rowSelection.selectionAnchorId = track.id
        } else {
            rowSelection.selectedTrackIds = [track.id]
            rowSelection.selectionAnchorId = track.id
        }
        // Replaces the removed `.onChange(of: selectedTrackIds)`
        // sink. `rowSelection` is owned via `@State` so
        // `LibraryView.body` no longer re-evaluates on the click
        // tick — and `.onChange` requires that body re-eval to
        // fire. Calling `syncModelPrimarySelection()` here keeps
        // the model-side `librarySelection` in lock-step with the
        // visible selection without that body re-eval cost.
        syncModelPrimarySelection()
    }

    private func selectRange(from anchorId: String, to targetId: String) {
        let ids = sortedTrackIds
        guard let a = ids.firstIndex(of: anchorId),
              let b = ids.firstIndex(of: targetId)
        else {
            rowSelection.selectedTrackIds = [targetId]
            return
        }
        let lo = min(a, b)
        let hi = max(a, b)
        rowSelection.selectedTrackIds = Set(ids[lo...hi])
    }

    private func syncModelPrimarySelection() {
        if let trackId = primarySelectedTrackId,
           let snapshot = tracks.first(where: { $0.id == trackId })
        {
            model.selectLibraryTrack(trackId, snapshot: snapshot)
        } else if rowSelection.selectedTrackIds.isEmpty {
            // Write through `librarySelection` (the new
            // side-channel) instead of `libraryModel` — both
            // fields no longer live on `LibraryAppModel`. See
            // `LibrarySelectionModel`'s header for the cascade-
            // cost rationale.
            model.librarySelection.selectedLibraryTrackId = nil
            model.librarySelection.selectedLibraryTrack = nil
        }
    }

    private func applyAnalysisUpdate(_ update: LibraryRowAnalysisUpdate) {
        guard let idx = tracks.firstIndex(where: { $0.id == update.trackId }) else {
            return
        }
        tracks[idx] = tracks[idx].patchedAfterAnalysis(update)
        recomputeSortedTracks()
        tracksContentRevision &+= 1
    }

    /// Refresh the memoised `sortedTracks` and `sortedTrackIds`
    /// from the current `tracks` + `sortOrder`. Call sites:
    ///
    /// * `refreshTracks(...)` after the FFI listing returns a
    ///   new `rows` buffer.
    /// * `applyAnalysisUpdate(...)` after a per-row patch so the
    ///   memoised array reflects the new field values (BPM, key,
    ///   analyzed-flag, etc.) the next time the LazyVStack
    ///   ForEach reads it.
    /// * `toggleSort(...)` and the hide-column path inside
    ///   `setColumnVisibility(...)` so a column-header click
    ///   resort lands on the same tick the user clicked.
    ///   M11d.6 round 3 deliberately dropped the
    ///   `.onChange(of: sortOrder)` handler that used to live
    ///   here — that handler caused a redundant rootView swap
    ///   on every sidebar click (first against the stale
    ///   tracks buffer, then again ~50 ms later against the
    ///   real rows). Sidebar swaps now flip `sortOrder` and
    ///   call `refreshTracks(...)`; the resort happens once,
    ///   when `refreshTracks` lands the new rows.
    /// * `.onAppear` so an early-render of the body sees a
    ///   populated cache before the async first fetch lands.
    ///
    /// Calling this from inside a body re-eval is safe — the
    /// `@State` writes coalesce into a single follow-up
    /// invalidation that fires after the current re-eval
    /// completes, which is the normal SwiftUI write-during-body
    /// idiom. The cost is one sort + one map per real change,
    /// down from three sorts + two maps per body re-eval pre-fix.
    private func recomputeSortedTracks() {
        let sorted = sortOrder.isEmpty
            ? tracks
            : tracks.sorted(using: sortOrder)
        if sorted != sortedTracks {
            sortedTracks = sorted
            sortedTrackIds = sorted.map(\.id)
        }
    }

    /// Finder drag-and-drop onto the library listing. Folders are
    /// walked recursively; individual audio files import via the
    /// same `import_folder` entry point (WalkDir yields one file).
    ///
    /// `provider.loadItem` callbacks fire on an arbitrary queue, so
    /// the URL collector is serialised through a dedicated queue
    /// before the main-actor import hop.
    private func handleLibraryDrop(_ providers: [NSItemProvider]) -> Bool {
        // A crate-reorder drag that lands in the list's empty area
        // (below the last row, or anywhere not over a specific row)
        // means "move to the end". The dragged provider also carries
        // a file URL, so without this guard the drop would fall
        // through to the importer and re-import the track's own file.
        // Detecting the reorder payload here is what makes the
        // end-of-playlist drop work.
        if isCrateManualOrder,
           let reorderProvider = providers.first(where: {
               $0.hasItemConformingToTypeIdentifier(Self.crateReorderType.identifier)
           })
        {
            loadCrateReorderPayload(from: reorderProvider) { ids in
                performCrateReorder(draggedIds: ids, toSlot: sortedTrackIds.count)
            }
            return true
        }
        guard libraryModel.libraryIsOpen, !libraryModel.libraryImportInProgress else { return false }
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

    // MARK: - Dub crate actions (M11d-next)

    /// Create a crate with a unique default name, then drop the user
    /// straight into inline-rename so the common "create + name it"
    /// flow is one gesture. The default-name uniqueness avoids a
    /// name-conflict error when the user spams the `+` button.
    private func createCrateAndRename() {
        let name = uniqueDefaultCrateName()
        guard let id = model.createCrate(named: name) else { return }
        beginInlineRename(crateId: id, seed: name)
    }

    /// "New Crate", then "New Crate 2", "New Crate 3", … skipping any
    /// already taken among the current top-level crates.
    private func uniqueDefaultCrateName() -> String {
        let existing = Set(libraryModel.crates.map(\.name))
        let base = "New Crate"
        if !existing.contains(base) { return base }
        var n = 2
        while existing.contains("\(base) \(n)") { n += 1 }
        return "\(base) \(n)"
    }

    private func beginCrateRename(_ crate: LibraryCrate) {
        beginInlineRename(crateId: crate.id, seed: crate.name)
    }

    private func beginInlineRename(crateId: Int64, seed: String) {
        crateRenameText = seed
        renamingCrateId = crateId
        // Defer focus to the next runloop tick so the TextField has
        // actually been installed by the time we request first
        // responder (SwiftUI attaches it on the same body pass that
        // flips `renamingCrateId`).
        DispatchQueue.main.async { crateRenameFocused = true }
    }

    private func commitCrateRename(_ crateId: Int64) {
        guard renamingCrateId == crateId else { return }
        let newName = crateRenameText.trimmingCharacters(in: .whitespacesAndNewlines)
        renamingCrateId = nil
        crateRenameFocused = false
        guard !newName.isEmpty else { return }
        // Skip the FFI write when nothing changed (a plain focus-loss
        // on an untouched field).
        if libraryModel.crates.first(where: { $0.id == crateId })?.name == newName { return }
        model.renameCrate(crateId, to: newName)
    }

    private func cancelCrateRename() {
        renamingCrateId = nil
        crateRenameFocused = false
    }

    /// Compute the full member-id order after moving `trackId` per
    /// `move` within `current` (the visible crate order). Returns
    /// `current` unchanged when the move is a no-op (already at the
    /// edge, or the id isn't present). The result is handed straight
    /// to `setCrateTrackOrder`, which rewrites ordinals 0..n.
    private func reorderedCrateIds(
        moving trackId: String, move: CrateMove, current: [String]
    ) -> [String] {
        guard let from = current.firstIndex(of: trackId) else { return current }
        var ids = current
        ids.remove(at: from)
        let to: Int
        switch move {
        case .up:     to = max(0, from - 1)
        case .down:   to = min(ids.count, from + 1)
        case .top:    to = 0
        case .bottom: to = ids.count
        }
        ids.insert(trackId, at: to)
        return ids
    }

    /// Confirm-then-delete a crate. Deleting cascades the crate's
    /// membership (and any child crates) so a misclick is worth one
    /// modal. The tracks themselves are never touched.
    private func confirmDeleteCrate(_ crate: LibraryCrate) {
        let alert = NSAlert()
        alert.messageText = "Delete the crate \"\(crate.name)\"?"
        alert.informativeText =
            "This removes the crate and its track list. The tracks themselves stay in your library."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Delete")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let wasSelected = selectedSource.crateId == crate.id
        model.deleteCrate(crate.id)
        if wasSelected {
            selectedSource = .allTracks
        }
    }

    /// Drop handler for a crate sidebar row. Collects the dragged
    /// file URLs (same payload the track rows produce for deck
    /// loads) and adds the matching library tracks to the crate.
    private func handleCrateDrop(_ providers: [NSItemProvider], crateId: Int64) -> Bool {
        guard libraryModel.libraryIsOpen else { return false }
        let collector = DispatchQueue(label: "com.dub.crate-drop-collector")
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
            guard !collected.isEmpty else { return }
            Task { @MainActor in
                model.addDroppedURLsToCrate(crateId, urls: collected)
            }
        }
        return !providers.isEmpty
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
            if libraryModel.libraryIsOpen && !model.isTrackReachable(track) {
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

    /// M11d-history — reveal-in-browser for the deck header's
    /// "↝ usually" hint. If the track is in the current listing,
    /// select + scroll directly. Otherwise stage it as a pending
    /// reveal and fall back to All Tracks with the search cleared —
    /// unlike `navigateToSibling`, auto-clearing is the *point*
    /// here: the click is an explicit "take me to this track", not
    /// an in-list affordance. The source/search mutation triggers
    /// the normal refresh path, whose completion consumes the
    /// pending id (and silently drops it when the track no longer
    /// exists in the library).
    private func revealTrack(_ trackId: String) {
        if sortedTrackIds.contains(trackId) {
            completeReveal(trackId)
            return
        }
        pendingRevealTrackId = trackId
        if !searchText.isEmpty {
            searchText = ""
        }
        if selectedSource != .allTracks {
            selectedSource = .allTracks
        } else {
            refreshTracks()
        }
    }

    /// Select + scroll one revealed track. Mirrors the keyboard-
    /// navigation path: selection via `rowSelection`, scroll via
    /// `keyboardScrollTarget` (consumed by `LibraryTableScroll`).
    private func completeReveal(_ trackId: String) {
        NSApp.keyWindow?.makeFirstResponder(nil)
        rowSelection.selectedTrackIds = [trackId]
        rowSelection.selectionAnchorId = trackId
        syncModelPrimarySelection()
        keyboardScrollTarget = trackId
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
        rowSelection.selectedTrackIds = [trackId]
        rowSelection.selectionAnchorId = trackId
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
            if let summary = libraryModel.lastImportSummary {
                Text(importSummaryLine(summary))
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
            } else if libraryModel.libraryImportInProgress {
                Text("Importing…")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
            }
            // M11c.1 — batch-analyze progress. Shown only while a
            // batch is in flight (single deck-load analyses bump
            // `analysisInFlightCount` without setting
            // `analysisBatchTotal`, so they don't crowd the
            // footer with one-second blips).
            if libraryModel.analysisBatchTotal > 0 {
                HStack(spacing: 4) {
                    ProgressView()
                        .scaleEffect(0.5)
                        .frame(width: 12, height: 12)
                    Text(analyzeProgressLine(
                        completed: libraryModel.analysisBatchCompleted,
                        total: libraryModel.analysisBatchTotal))
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textSecondary)
                }
            }
            if libraryModel.missingTrackCount > 0 {
                Button(action: { showRelocateSheet = true }) {
                    HStack(spacing: 4) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red.opacity(0.85))
                        Text(missingFooterLine(libraryModel.missingTrackCount))
                            .underline()
                    }
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
                }
                .buttonStyle(.plain)
                .help("Open the Relocate panel to point Dub at the directory that holds the missing files.")
            }
            Spacer(minLength: 0)
            Text("\(tracks.count) shown · \(libraryModel.libraryTrackCount) total")
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
        case .sessionHistory:
            return "Nothing played this session yet."
        case .justImported:
            return "No imports this session."
        case .dubCrate:
            return "This crate is empty."
        case .importedSource(let kind):
            return "No \(kind.label) tracks."
        case .importedCrate:
            return "This playlist is empty."
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
            return "Use the Import menu (or Preferences ▸ Libraries) to add tracks."
        case .recentlyPlayed:
            return "Tracks you load on a deck show up here."
        case .sessionHistory:
            return "Tracks you play this session show up here, in set order."
        case .justImported:
            return "Tracks imported this session show up here."
        case .dubCrate:
            return "Drag tracks onto the crate in the sidebar to add them."
        case .importedSource(let kind):
            return "Enable and scan \(kind.label) in Preferences ▸ Libraries."
        case .importedCrate:
            return nil
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
        guard libraryModel.libraryIsOpen else {
            tracks = []
            recomputeSortedTracks()
            rowSelection.selectedTrackIds = []
            rowSelection.selectionAnchorId = nil
            syncModelPrimarySelection()
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
            rowSelection.selectedTrackIds = []
            rowSelection.selectionAnchorId = nil
            syncModelPrimarySelection()
        }
        let preservedSelection = rowSelection.selectedTrackIds
        let preservedAnchor = rowSelection.selectionAnchorId
        let source = selectedSource
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let limit = Self.listingLimit
        let library = model.library
        let since = model.appLaunchUnixSeconds

        isLoading = true
        Task.detached(priority: .userInitiated) {
            let rows: [LibraryTrack]
            // M11d-history: per-row "← from <track>" annotations,
            // populated only by the Session History source so the
            // dict doubles as the "is this source active" gate at
            // render time.
            var fromTitles: [String: String] = [:]
            do {
                if !query.isEmpty {
                    rows = try library.search(query: query, limit: limit)
                } else {
                    switch source {
                    case .allTracks:
                        rows = try library.listTracks(limit: limit, offset: 0)
                    case .recentlyPlayed:
                        rows = try library.recentlyPlayed(limit: limit)
                    case .sessionHistory:
                        let plays = try library.sessionHistory(limit: limit)
                        rows = plays.map(\.track)
                        for play in plays {
                            if let from = play.fromTrackTitle {
                                fromTitles[play.track.id] = from
                            }
                        }
                    case .justImported:
                        rows = try library.justImported(
                            sinceUnixSecs: since, limit: limit)
                    case .dubCrate(let crateId):
                        rows = try library.crateTracks(crateId: crateId)
                    case .importedSource(let kind):
                        rows = try library.listTracksBySource(
                            source: kind.sourceTag, limit: limit, offset: 0)
                    case .importedCrate(let id):
                        rows = try library.importedCrateTracks(importedCrateId: id)
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
            // Immutable snapshot — the @Sendable MainActor closure
            // can't capture the mutated local `var` directly.
            let resolvedFromTitles = fromTitles
            await MainActor.run {
                self.tracks = rows
                self.sessionFromTitles = resolvedFromTitles
                self.recomputeSortedTracks()
                self.tracksContentRevision &+= 1
                self.isLoading = false
                // M11d-history: a reveal staged across this refresh
                // lands now that the rows are in. Always clears —
                // a track that vanished from the library drops the
                // reveal silently rather than ambushing a later
                // unrelated refresh.
                if let pending = self.pendingRevealTrackId {
                    self.pendingRevealTrackId = nil
                    if rows.contains(where: { $0.id == pending }) {
                        self.completeReveal(pending)
                    }
                }
                if preserveSelection {
                    let visible = Set(rows.map(\.id))
                    self.rowSelection.selectedTrackIds =
                        preservedSelection.intersection(visible)
                    if let anchor = preservedAnchor,
                       self.rowSelection.selectedTrackIds.contains(anchor)
                    {
                        self.rowSelection.selectionAnchorId = anchor
                    } else {
                        self.rowSelection.selectionAnchorId =
                            self.rowSelection.selectedTrackIds.sorted().first
                    }
                    self.syncModelPrimarySelection()
                }
            }
            // Volume reachability has to run on the main thread
            // (it mutates `libraryModel.volumeReachability`), but
            // it's an O(rows) scan + per-mount-point `stat(2)`
            // that has no business piggy-backing on the same
            // runloop tick that ships the new rows into the
            // table. Pre-fix it shared the `MainActor.run` block
            // above, so the visible row body re-eval + AppKit
            // selection-layer redraw + NSHostingView LazyVStack
            // rebuild + `stat` syscalls all serialised onto the
            // same vsync window — the largest single source of
            // residual main-thread block on a sidebar swap. The
            // reachability map drives the per-row missing-file
            // glyph ONLY, so a one-runloop-tick delay is
            // invisible to the user (the glyph just paints on
            // the next CVDisplayLink frame instead of this one).
            await MainActor.run {
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

    /// Manual picker for an external source. Serato is a folder (the
    /// `_Serato_` directory); Traktor / iTunes are single files
    /// (`collection.nml` / `Library.xml`). The Preferences ▸ Libraries
    /// toggles are the primary, auto-scanning path — this is the
    /// pick-it-anywhere fallback (e.g. a library on an external drive).
    private func presentImportedSourcePicker(_ kind: ImportedSourceKind) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.prompt = "Import"
        switch kind {
        case .serato:
            panel.canChooseFiles = false
            panel.canChooseDirectories = true
            panel.message = "Choose a “_Serato_” folder to import."
        case .traktor:
            panel.canChooseFiles = true
            panel.canChooseDirectories = false
            panel.message = "Choose a Traktor “collection.nml” to import."
        case .itunes:
            panel.canChooseFiles = true
            panel.canChooseDirectories = false
            panel.message = "Choose an iTunes “Library.xml” to import."
        }
        if panel.runModal() == .OK, let url = panel.url {
            Task { @MainActor in
                await model.importExternalSource(kind, path: url.path)
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
    /// Non-nil when the visible listing is a Dub crate, enabling the
    /// crate-specific "Remove from Crate" / "Move…" menu items
    /// (M11d-next). `nil` for All Tracks / smart crates / search.
    var crateId: Int64? = nil
    /// Remove the given track ids from the open crate. Selection-aware
    /// (matches the analyze target set). No-op when `crateId == nil`.
    var onCrateRemove: (([String]) -> Void)? = nil
    /// Reorder the right-clicked track within the open crate.
    var onCrateMove: ((String, CrateMove) -> Void)? = nil
}

/// Where a "Move…" crate context-menu item places the track within
/// its crate (M11d-next reorder without a drag, friendlier to verify
/// than a drag-and-drop reorder in the bespoke AppKit table).
enum CrateMove {
    case up
    case down
    case top
    case bottom
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
/// * `rowSelection: LibraryRowSelection` — the shared,
///   non-observed selection store. The Coordinator subscribes to
///   `rowSelection.$selectedTrackIds` via Combine so the AppKit
///   selection layer repaints when the user clicks a row even
///   though `LibraryView.body` no longer re-evaluates on
///   selection changes (selection lives on a class reference,
///   owned via `@State` for identity, deliberately not observed
///   by SwiftUI). The right-click menu reads
///   `rowSelection.selectedTrackIds` live so the
///   multi-select label is always accurate at popup time.
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
    let rowSelection: LibraryRowSelection
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
    /// `true` while the visible crate is in manual order, enabling the
    /// AppKit drag-to-reorder drop target + insertion line.
    let crateReorderEnabled: Bool
    /// Commit handler for a reorder drop: `(draggedIds, insertionSlot)`
    /// where `insertionSlot` is 0-based in `0…count`.
    let onCrateReorder: (([String], Int) -> Void)?

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
        // Width follows the wrapper, but height is pinned to the
        // content (set in `updateBodyHeight`). The wrapper itself is
        // grown to fill the viewport so its empty bottom area is a
        // valid reorder drop target (see `updateBodyHeight`); letting
        // the host stretch with it would make SwiftUI re-center the
        // rows in the taller frame and desync them from the AppKit
        // selection layer.
        bodyHost.autoresizingMask = [.width]
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
            headerLeadingConstraint: leadingConstraint,
            rowSelection: rowSelection
        )
        context.coordinator.recordSnapshot(
            tableWidth: tableWidth,
            columnOrderKey: columnOrderKey,
            headerStateKey: headerStateKey,
            tracksContentRevision: tracksContentRevision,
            selectedTrackIds: rowSelection.selectedTrackIds,
            trackIds: trackIds)
        context.coordinator.updateMenuState(
            visibleTracks: visibleTracks,
            menu: menu)
        context.coordinator.attachMenuBuilder(rowHeight: LibraryRowLayout.estimatedHeight)
        context.coordinator.updateBodyHeight()
        context.coordinator.updateSelectionHighlights(
            selectedTrackIds: rowSelection.selectedTrackIds, trackIds: trackIds)
        context.coordinator.updateReorder(
            enabled: crateReorderEnabled,
            rowHeight: LibraryRowLayout.estimatedHeight,
            trackCount: trackIds.count,
            onReorder: onCrateReorder)
        return stack
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        let coordinator = context.coordinator
        let liveSelection = rowSelection.selectedTrackIds
        let tableWidthChanged = coordinator.lastTableWidth != tableWidth
        let tracksChanged = coordinator.lastTrackIds != trackIds
        let columnsChanged = coordinator.lastColumnOrderKey != columnOrderKey
        let headerStateChanged = coordinator.lastHeaderStateKey != headerStateKey
        let tracksContentChanged = coordinator.lastTracksContentRevision != tracksContentRevision
        let selectionChanged = coordinator.lastSelectedTrackIds != liveSelection
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
                // Cache must be in sync **before**
                // `updateSelectionHighlights` below — otherwise
                // the highlight code would resolve indices
                // against the previous row set's lookup table
                // and produce ghost highlights for missing IDs.
                coordinator.rebuildTrackIdLookup(trackIds: trackIds)
            }
        } else if tableWidthChanged {
            coordinator.updateWidths(tableWidth)
        }

        if selectionChanged || tracksChanged {
            coordinator.updateSelectionHighlights(
                selectedTrackIds: liveSelection, trackIds: trackIds)
        }

        coordinator.recordSnapshot(
            tableWidth: tableWidth,
            columnOrderKey: columnOrderKey,
            headerStateKey: headerStateKey,
            tracksContentRevision: tracksContentRevision,
            selectedTrackIds: liveSelection,
            trackIds: trackIds)
        // The right-click menu reads from the Coordinator at
        // `menu(for:)` time, so it ALWAYS sees the latest selection
        // / lock state even though the SwiftUI rows themselves
        // are not re-rendered on selection changes (intentional —
        // the AppKit selection layer paints highlights instead).
        coordinator.updateMenuState(
            visibleTracks: visibleTracks,
            menu: menu)
        coordinator.updateReorder(
            enabled: crateReorderEnabled,
            rowHeight: LibraryRowLayout.estimatedHeight,
            trackCount: trackIds.count,
            onReorder: onCrateReorder)
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

    @MainActor
    final class Coordinator {
        weak var bodyScroll: NSScrollView?
        weak var headerHost: NSHostingView<AnyView>?
        weak var bodyHost: NSHostingView<AnyView>?
        weak var documentWrapper: LibraryDocumentWrapper?
        weak var selectionLayer: LibrarySelectionLayerView?
        weak var headerWidthConstraint: NSLayoutConstraint?
        weak var headerLeadingConstraint: NSLayoutConstraint?
        private var boundsObserver: NSObjectProtocol?
        /// The shared, non-observed `LibraryRowSelection` instance
        /// the parent view also points at. Stored so the menu
        /// builder can read live selection at right-click time and
        /// so the Combine subscription below has a stable
        /// reference to subscribe to.
        fileprivate weak var rowSelection: LibraryRowSelection?
        /// Subscription on `rowSelection.$selectedTrackIds`. Fires
        /// the AppKit selection-layer refresh whenever
        /// `handleRowClick`, arrow-key navigation, or any other
        /// site writes to the selection — without re-evaluating
        /// `LibraryView.body`. Held here so it lives as long as
        /// the Coordinator.
        private var selectionCancellable: AnyCancellable?
        /// `trackId → row index` cache keyed off the current
        /// `lastTrackIds` array. Refreshed in `recordSnapshot(...)`
        /// whenever the visible track set changes (sidebar swap,
        /// search, analysis-update row patch). Lets
        /// `updateSelectionHighlights` resolve the IndexSet via
        /// O(selected.count) Dictionary lookups instead of an
        /// O(rows × selected.count) enumeration over the full
        /// 5 000-row trackIds array — material at the millisecond
        /// scale once selections grow beyond a single row via
        /// Shift-click.
        fileprivate var trackIdToIndex: [String: Int] = [:]
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
        /// Live selection at right-click time. Read through a
        /// computed property so the menu builder always sees the
        /// current state on `rowSelection`, even when the
        /// SwiftUI render pass that triggered the right click
        /// has not re-evaluated `LibraryView.body` (selection
        /// changes no longer fire a body re-eval; see
        /// `LibraryRowSelection`).
        private var menuSelectedTrackIds: Set<String> {
            rowSelection?.selectedTrackIds ?? []
        }
        fileprivate var menuAnalysisBatchInProgress: Bool = false
        fileprivate var onAnalyzeRequested: (([String]) -> Void)?
        fileprivate var onSetGridLocked: ((String, Bool) -> Void)?
        /// Crate context (M11d-next): non-nil id + callbacks when the
        /// visible listing is a Dub crate, so the right-click menu
        /// can offer "Remove from Crate" + "Move…".
        fileprivate var menuCrateId: Int64?
        fileprivate var onCrateRemove: (([String]) -> Void)?
        fileprivate var onCrateMove: ((String, CrateMove) -> Void)?
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

        /// Refresh the `trackIdToIndex` lookup. Called from
        /// `LibraryTableScrollContainer.updateNSView` when
        /// `tracksChanged` is true, **before**
        /// `updateSelectionHighlights` reads the table — so the
        /// O(selected.count) lookup path in the highlight code
        /// always sees an in-sync cache.
        ///
        /// The Dictionary rebuild is O(N) on the 5 000-row buffer
        /// but only fires on a real row-set change (sidebar swap,
        /// search edit, refresh-tracks bump). Per-click selection
        /// changes against the same row set reuse the cached
        /// lookup without rebuilding.
        func rebuildTrackIdLookup(trackIds: [String]) {
            var lookup = [String: Int](minimumCapacity: trackIds.count)
            for (i, id) in trackIds.enumerated() {
                lookup[id] = i
            }
            trackIdToIndex = lookup
        }

        func install(
            bodyScroll: NSScrollView,
            headerHost: NSHostingView<AnyView>,
            bodyHost: NSHostingView<AnyView>,
            documentWrapper: LibraryDocumentWrapper,
            selectionLayer: LibrarySelectionLayerView,
            headerWidthConstraint: NSLayoutConstraint,
            headerLeadingConstraint: NSLayoutConstraint,
            rowSelection: LibraryRowSelection
        ) {
            self.bodyScroll = bodyScroll
            self.headerHost = headerHost
            self.bodyHost = bodyHost
            self.documentWrapper = documentWrapper
            self.selectionLayer = selectionLayer
            self.headerWidthConstraint = headerWidthConstraint
            self.headerLeadingConstraint = headerLeadingConstraint
            self.rowSelection = rowSelection
            bodyScroll.contentView.postsBoundsChangedNotifications = true
            boundsObserver = NotificationCenter.default.addObserver(
                forName: NSView.boundsDidChangeNotification,
                object: bodyScroll.contentView,
                queue: .main
            ) { [weak self] _ in
                // `queue: .main` already delivers this on the main
                // thread, but `NotificationCenter`'s closure type
                // is not `@MainActor`-typed, so Swift 6 won't let
                // us call a MainActor method without an explicit
                // hop. `assumeIsolated` is the zero-cost
                // dispatch-free hop documented for exactly this
                // case.
                MainActor.assumeIsolated {
                    self?.syncHeaderOffset()
                    // Fires on scroll AND on viewport resize. The
                    // helper guards on a changed clip height, so the
                    // common scroll case is a no-op and only an actual
                    // resize re-fills the wrapper's drop-target area.
                    self?.syncWrapperFillHeight()
                }
            }
            // M11d.6 round 4 — selection is now mutated on
            // `rowSelection`, a class that `LibraryView` owns via
            // `@State` (so its body does NOT re-evaluate on
            // selection changes). Without this subscription, the
            // AppKit selection layer would never see a click
            // because `updateNSView` wouldn't be invoked. We
            // subscribe to `$selectedTrackIds` here (which fires
            // synchronously on every assignment via Combine's
            // `@Published`) and route the new value through
            // `updateSelectionHighlights` so the layer redraws on
            // the same tick the click landed on.
            selectionCancellable = rowSelection.$selectedTrackIds
                .dropFirst()
                .sink { [weak self] newIds in
                    guard let self else { return }
                    self.lastSelectedTrackIds = newIds
                    self.updateSelectionHighlights(
                        selectedTrackIds: newIds,
                        trackIds: self.lastTrackIds)
                }
        }

        func uninstall() {
            if let boundsObserver {
                NotificationCenter.default.removeObserver(boundsObserver)
            }
            selectionCancellable = nil
            rowSelection = nil
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

        /// Last measured SwiftUI content height (sum of row heights).
        /// Cached so the cheap viewport-resize path can re-grow the
        /// wrapper without re-running the (relatively pricey)
        /// `fittingSize` measurement on every scroll/resize tick.
        private var lastContentHeight: CGFloat = 1
        /// Last clip-view height the wrapper was filled against, so
        /// `syncHeaderOffset`'s bounds observer can tell a viewport
        /// resize apart from a plain scroll and skip redundant work.
        fileprivate var lastViewportHeight: CGFloat = -1

        func updateBodyHeight() {
            guard let bodyHost, let documentWrapper, let bodyScroll else { return }
            bodyHost.invalidateIntrinsicContentSize()
            let contentHeight = max(bodyHost.fittingSize.height, 1)
            lastContentHeight = contentHeight

            // The hosting view (the rows) is pinned to the content
            // height and top-aligned. The wrapper, however, is grown
            // to at least the viewport height so its empty area below
            // the last row is still part of the AppKit reorder drop
            // target — dragging into the black space past the end now
            // resolves to the "append" slot with a live insertion
            // line, instead of falling through to the file-import path.
            var bodyFrame = bodyHost.frame
            bodyFrame.size.height = contentHeight
            bodyHost.frame = bodyFrame

            let viewportHeight = bodyScroll.contentView.bounds.height
            lastViewportHeight = viewportHeight
            var frame = documentWrapper.frame
            frame.size.height = max(contentHeight, viewportHeight)
            documentWrapper.frame = frame
            // bodyScroll re-evaluates scrollable area off `documentView.frame`.
            bodyScroll.documentView = documentWrapper
        }

        /// Cheap viewport-resize handler: re-fills the wrapper to the
        /// new clip height using the cached content height, without a
        /// `fittingSize` re-measure. The hosting view stays at the
        /// content height (rows top-aligned); only the wrapper's
        /// drop-target area grows or shrinks.
        func syncWrapperFillHeight() {
            guard let documentWrapper, let bodyScroll else { return }
            let viewportHeight = bodyScroll.contentView.bounds.height
            guard viewportHeight != lastViewportHeight else { return }
            lastViewportHeight = viewportHeight
            var frame = documentWrapper.frame
            frame.size.height = max(lastContentHeight, viewportHeight)
            documentWrapper.frame = frame
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
            // Resolve indices via `trackIdToIndex` so the cost is
            // O(selectedTrackIds.count) instead of O(trackIds.count).
            // Pre-fix this enumerated all 5 000 trackIds and
            // ran a `Set.contains` per row on every selection
            // change — including the ~10 Hz `pollDecks` cascade
            // chain that can land while the user is mid-Shift-
            // click range select. Falls back to the linear scan
            // if the lookup table wasn't populated yet (e.g.
            // first `updateNSView` before any `recordSnapshot`).
            var indices = IndexSet()
            if !trackIdToIndex.isEmpty {
                for id in selectedTrackIds {
                    if let i = trackIdToIndex[id] {
                        indices.insert(i)
                    }
                }
            } else {
                for (i, id) in trackIds.enumerated() where selectedTrackIds.contains(id) {
                    indices.insert(i)
                }
            }
            selectionLayer.selectedRowIndices = indices
        }

        /// Snapshot the menu-time inputs so the AppKit
        /// `menu(for:)` override (which fires asynchronously
        /// from the SwiftUI render pass) reads consistent state.
        /// Called on every `updateNSView`.
        ///
        /// Selection deliberately is *not* a parameter: the menu
        /// builder reads `rowSelection.selectedTrackIds` live at
        /// right-click time, which always reflects the latest
        /// state regardless of whether `LibraryView` re-evaluated
        /// recently. That's what makes the multi-select
        /// "Re-analyze Selected (N)" label correct even when
        /// selection is mutated without a SwiftUI body re-eval.
        func updateMenuState(
            visibleTracks: [LibraryTrack],
            menu: LibraryTableMenu
        ) {
            self.visibleTracks = visibleTracks
            self.menuAnalysisBatchInProgress = menu.analysisBatchInProgress
            self.onAnalyzeRequested = menu.onAnalyzeRequested
            self.onSetGridLocked = menu.onSetGridLocked
            self.menuCrateId = menu.crateId
            self.onCrateRemove = menu.onCrateRemove
            self.onCrateMove = menu.onCrateMove
        }

        /// Push the crate drag-reorder config onto the document
        /// wrapper. The wrapper is the AppKit drop target + paints the
        /// insertion line; this keeps its row-height / member-count /
        /// enable flag in sync as the visible listing changes.
        func updateReorder(
            enabled: Bool,
            rowHeight: CGFloat,
            trackCount: Int,
            onReorder: (([String], Int) -> Void)?
        ) {
            guard let wrapper = documentWrapper else { return }
            wrapper.reorderRowHeight = rowHeight
            wrapper.reorderTrackCount = trackCount
            wrapper.onReorderDrop = onReorder
            wrapper.reorderEnabled = enabled
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

            // M11d-next — crate-specific items, only when the visible
            // listing is a Dub crate. "Remove from Crate" is
            // selection-aware (matches the analyze target set); the
            // "Move…" items act on the single right-clicked row and
            // grey out at the edges.
            if menuCrateId != nil {
                menu.addItem(.separator())
                appendCrateItems(to: menu, rightClickedTrack: track)
            }

            // Drop the previous run's anchors once a new menu is
            // built — the anchors are only needed while the menu
            // is visible, and AppKit keeps the in-flight menu
            // retained via its own dispatcher. Without this the
            // anchor list would grow unbounded over a long
            // session.
            menuActionAnchors = Array(menuActionAnchors.suffix(16))
            return menu
        }

        /// Build the "Remove from Crate" + "Move…" block. Split out
        /// of `buildContextMenu` to keep that method readable. Move
        /// items are disabled at the list edges (can't move the top
        /// row up, etc.) using the right-clicked row's index in the
        /// visible (ordinal) order.
        private func appendCrateItems(to menu: NSMenu, rightClickedTrack track: LibraryTrack) {
            let removeTargets = analyzeTargets(rightClickedTrack: track)
            let removeTitle = removeTargets.count > 1
                ? "Remove from Crate (\(removeTargets.count))"
                : "Remove from Crate"
            let removeItem = NSMenuItem(title: removeTitle, action: nil, keyEquivalent: "")
            if let onCrateRemove {
                let target = LibraryMenuActionTarget { onCrateRemove(removeTargets) }
                menuActionAnchors.append(target)
                removeItem.target = target
                removeItem.action = #selector(LibraryMenuActionTarget.dubMenuPerform(_:))
                removeItem.isEnabled = true
            } else {
                removeItem.isEnabled = false
            }
            menu.addItem(removeItem)

            guard let onCrateMove else { return }
            let index = visibleTracks.firstIndex(where: { $0.id == track.id })
            let count = visibleTracks.count
            menu.addItem(.separator())
            let moves: [(String, CrateMove, Bool)] = [
                ("Move Up", .up, (index ?? 0) > 0),
                ("Move Down", .down, (index ?? count) < count - 1),
                ("Move to Top", .top, (index ?? 0) > 0),
                ("Move to Bottom", .bottom, (index ?? count) < count - 1),
            ]
            for (title, move, enabled) in moves {
                let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
                if enabled {
                    let target = LibraryMenuActionTarget { onCrateMove(track.id, move) }
                    menuActionAnchors.append(target)
                    item.target = target
                    item.action = #selector(LibraryMenuActionTarget.dubMenuPerform(_:))
                    item.isEnabled = true
                } else {
                    item.isEnabled = false
                }
                menu.addItem(item)
            }
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

    // MARK: - Crate drag-reorder (AppKit drop target + insertion line)
    //
    // The reorder DROP is handled here, not in a per-row SwiftUI
    // `.onDrop`, because the rows live in an `NSHostingView` whose
    // `rootView` is only re-assigned on a track/column/width change
    // (the perf optimization in `LibraryTableScrollContainer`). A
    // SwiftUI drop delegate flipping `@State` would never repaint the
    // insertion line mid-drag. Painting in AppKit — like the selection
    // layer below — gives live feedback and reliable end-of-list drops.
    //
    // The drag SOURCE stays in SwiftUI (`makeRowDragProvider`); it puts
    // a `DUBCRATE`-tagged payload on the drag pasteboard under the
    // private reorder UTI. This wrapper registers for exactly that
    // type, so Finder file drags (file-URL only) fall through to the
    // library importer and deck-load drags hit the deck's own
    // `dropDestination(for: URL.self)`.

    /// Enabled only while the visible crate is in manual order.
    var reorderEnabled = false {
        didSet {
            if reorderEnabled != oldValue { refreshReorderRegistration() }
        }
    }
    var reorderRowHeight: CGFloat = 28
    /// Member count of the visible crate; clamps the insertion slot.
    var reorderTrackCount = 0
    /// Called on a committed drop with the dragged ids and the 0-based
    /// insertion slot in `0…count`. The SwiftUI side performs the
    /// `setCrateOrder` write.
    var onReorderDrop: (([String], Int) -> Void)?

    static let reorderPasteboardType =
        NSPasteboard.PasteboardType("com.dub.crate-track-order")

    private lazy var insertionLine: NSView = {
        let view = NSView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor(DubColor.deckATint).cgColor
        view.layer?.cornerRadius = 1
        view.isHidden = true
        return view
    }()

    override var isFlipped: Bool { true }

    override func menu(for event: NSEvent) -> NSMenu? {
        if let menu = menuBuilder?(event) {
            return menu
        }
        return super.menu(for: event)
    }

    func refreshReorderRegistration() {
        if reorderEnabled {
            registerForDraggedTypes([Self.reorderPasteboardType])
        } else {
            unregisterDraggedTypes()
            hideInsertionLine()
        }
    }

    private func hasReorderPayload(_ sender: NSDraggingInfo) -> Bool {
        sender.draggingPasteboard.types?.contains(Self.reorderPasteboardType) ?? false
    }

    /// 0-based insertion slot from the drop's y-position. Top half of a
    /// row inserts before it, bottom half after, so the slot ranges
    /// `0…count` and the list's end is always reachable.
    private func slot(for sender: NSDraggingInfo) -> Int {
        guard reorderRowHeight > 0 else { return 0 }
        let point = convert(sender.draggingLocation, from: nil)
        let raw = Int(((point.y + reorderRowHeight / 2) / reorderRowHeight).rounded(.down))
        return max(0, min(reorderTrackCount, raw))
    }

    private func showInsertionLine(at slot: Int) {
        if insertionLine.superview !== self {
            addSubview(insertionLine, positioned: .above, relativeTo: nil)
        }
        let y = CGFloat(slot) * reorderRowHeight
        insertionLine.frame = NSRect(x: 0, y: max(0, y - 1), width: bounds.width, height: 2)
        insertionLine.isHidden = false
    }

    private func hideInsertionLine() {
        insertionLine.isHidden = true
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard reorderEnabled, hasReorderPayload(sender) else { return [] }
        showInsertionLine(at: slot(for: sender))
        return .copy
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard reorderEnabled, hasReorderPayload(sender) else { return [] }
        showInsertionLine(at: slot(for: sender))
        return .copy
    }

    override func draggingExited(_ sender: NSDraggingInfo?) {
        hideInsertionLine()
    }

    override func draggingEnded(_ sender: NSDraggingInfo) {
        hideInsertionLine()
    }

    override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool {
        reorderEnabled && hasReorderPayload(sender)
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        defer { hideInsertionLine() }
        guard reorderEnabled, hasReorderPayload(sender) else { return false }
        let targetSlot = slot(for: sender)
        guard let data = sender.draggingPasteboard.data(forType: Self.reorderPasteboardType),
              let text = String(data: data, encoding: .utf8)
        else { return false }
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        guard lines.first == "DUBCRATE" else { return false }
        let ids = Array(lines.dropFirst()).filter { !$0.isEmpty }
        guard !ids.isEmpty else { return false }
        onReorderDrop?(ids, targetSlot)
        return true
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
            gridDriftQuality: gridDriftQuality,
            crateOrdinal: crateOrdinal)
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
/// **Selection prop** (read + mutated by the arrow-key handler):
///
/// * `rowSelection: LibraryRowSelection` — the shared, non-observed
///   selection state. Mutating
///   `rowSelection.selectedTrackIds` or
///   `rowSelection.selectionAnchorId` here is what arrow-key
///   navigation actually does. The Coordinator that
///   `LibraryTableScrollContainer` owns subscribes to this same
///   instance via Combine, so the AppKit selection layer
///   repaints on the next vsync without `LibraryView` having to
///   re-evaluate.
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
    let rowSelection: LibraryRowSelection
    let trackIds: [String]
    var onArrowNavigate: ((String, Int) -> Void)?
    var onSelectionChanged: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(
            rowSelection: rowSelection,
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
        let rowSelection: LibraryRowSelection
        var trackIds: [String] = []
        var onArrowNavigate: ((String, Int) -> Void)?
        var onSelectionChanged: () -> Void
        private var monitor: Any?

        init(
            rowSelection: LibraryRowSelection,
            onArrowNavigate: ((String, Int) -> Void)?,
            onSelectionChanged: @escaping () -> Void
        ) {
            self.rowSelection = rowSelection
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
            let primary = rowSelection.selectionAnchorId
                ?? rowSelection.selectedTrackIds.sorted().first
            let currentIdx: Int
            if let id = primary, let idx = trackIds.firstIndex(of: id) {
                currentIdx = idx
            } else {
                let firstId = trackIds[0]
                NSApp.keyWindow?.makeFirstResponder(nil)
                rowSelection.selectedTrackIds = [firstId]
                rowSelection.selectionAnchorId = firstId
                onSelectionChanged()
                onArrowNavigate?(firstId, delta)
                return true
            }
            let next = max(0, min(trackIds.count - 1, currentIdx + delta))
            guard next != currentIdx else {
                // U-16 — hard stop at the top/bottom of the listing.
                // Match the rest of macOS arrow-key table navigation
                // and beep so the user knows the press registered but
                // there's nowhere further to go.
                NSSound.beep()
                return false
            }
            let nextId = trackIds[next]
            NSApp.keyWindow?.makeFirstResponder(nil)
            rowSelection.selectedTrackIds = [nextId]
            rowSelection.selectionAnchorId = nextId
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
    /// Library/analysis state passed through from the parent.
    /// See `LibraryView.libraryModel`'s doc comment for the
    /// performance rationale.
    @ObservedObject var libraryModel: LibraryAppModel
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

            if libraryModel.relocateInProgress {
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
                .disabled(libraryModel.relocateInProgress || libraryModel.missingTrackCount == 0)
            }
        }
        .padding(DubSpacing.xl)
        .frame(minWidth: 460, idealWidth: 520, maxWidth: 640)
        .background(DubColor.surface0)
    }

    private var headline: String {
        if libraryModel.missingTrackCount == 0 {
            return "All known files are reachable. Nothing to relocate right now."
        }
        let n = libraryModel.missingTrackCount
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
