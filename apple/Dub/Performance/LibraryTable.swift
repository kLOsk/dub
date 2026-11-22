//
//  LibraryTable.swift
//  Dub
//
//  The library track list, as a real `NSTableView`.
//
//  ## Why
//
//  The previous table was SwiftUI rows in a `LazyVStack` inside an
//  `NSHostingView`, and it had no cell reuse. Measured on a ~116-row
//  library:
//
//  * changing a column width re-laid out every row — ~45 ms, which
//    capped a resize drag near 18 Hz even with nothing rebuilding;
//  * rebuilding the rows on top of that cost ~135 ms more, taking the
//    drag to 5 Hz;
//  * repainting one row's colour label rebuilt all of them, because
//    SwiftUI has no way to say "re-render row 42".
//
//  None of those are reachable from inside SwiftUI. `NSTableView`
//  makes column width a property of the table and reuses cells, so a
//  drag moves frames on the ~30 visible cells and a colour change is
//  `reloadData(forRowIndexes:columnIndexes:)`.
//
//  ## Keeping the look
//
//  Cell content is still the same SwiftUI — `LibraryColumnCell` and
//  `LibraryRowIndicators` — hosted in reused cells. That is why they
//  were made value-driven first: a reused cell is configured with
//  values and must not consult a live object afterwards.
//  `LibraryCellSnapshotTests` is the baseline.
//
//  The header is the exception: `LibraryHeaderTextCell` draws it in
//  AppKit, to match the SwiftUI `LibraryHeaderCell` that the header
//  baselines pin. `LibraryHeaderCell` is a reference rendering for
//  those baselines only; nothing on the live path draws it.
//
//  Two pieces of the old look are reproduced in AppKit drawing rather
//  than SwiftUI, because they belong to the row, not a cell:
//
//  * **Selection** is a flat `surface2` fill, not the system accent.
//    `selectionHighlightStyle = .none` and `LibraryTintedRowView`
//    paints it, so the previous appearance survives.
//  * **The colour label tint** sits *above* the selection fill at 0.18
//    opacity — that ordering is what keeps a selected coloured row
//    legible, and it is why both are drawn in one override.
//

import AppKit
import os
import DubCore
import SwiftUI

/// One column, as the table needs it.
struct LibraryTableColumnSpec: Equatable {
    let field: LibraryColumnField
    let title: String
    let width: CGFloat
    var isSortActive: Bool = false
    var sortAscending: Bool = true
    /// The `#` column is pinned and never user-reordered or resized.
    var isPinned: Bool = false
}

/// Callbacks the table fires. Not `Equatable`, so kept out of the specs.
struct LibraryTableCallbacks {
    var onToggleSort: (LibraryColumnField) -> Void = { _ in }
    var onColumnResized: (LibraryColumnField, CGFloat) -> Void = { _, _ in }
    var onColumnsReordered: ([LibraryColumnField]) -> Void = { _ in }
    var onSelectionChanged: () -> Void = {}
    /// Resolve a track's file URL for drag-out. `nil` when the source
    /// volume is unmounted — an unreachable row is simply not
    /// draggable, rather than vending a path the decoder would choke on.
    var dragURL: (LibraryTrack) -> URL? = { _ in nil }
    /// `true` while the open crate is in manual order, which is the
    /// only state where an in-list reorder has a persistent meaning.
    var crateReorderEnabled: () -> Bool = { false }
    /// Commit a reorder: the dragged ids and the 0-based insertion slot.
    var onCrateReorder: ([String], Int) -> Void = { _, _ in }
    /// Build the right-click menu for a row, or `nil` for no menu.
    var menuForRow: (Int) -> NSMenu? = { _ in nil }
    /// Build the header's column picker, or `nil` for no menu.
    var menuForHeader: () -> NSMenu? = { nil }
    /// Fired after a `scrollToTrackId` request has been acted on, so
    /// the caller can clear it.
    var onScrollHandled: () -> Void = {}
}

/// In-process drag type carrying the crate-reorder payload. Kept
/// distinct from `public.file-url` — the deck / sidebar drop — so an
/// in-list reorder only ever reacts to a row drag that started inside
/// the open crate, never to a Finder file drag.
enum LibraryCrateReorder {
    static let pasteboardType =
        NSPasteboard.PasteboardType("com.dub.crate-track-order")
    /// Sentinel prefixing the newline-joined ids, so a foreign payload
    /// on the same type cannot be mistaken for ours.
    static let sentinel = "DUBCRATE"

    static func payload(ids: [String]) -> Data {
        Data(([sentinel] + ids).joined(separator: "\n").utf8)
    }

    static func ids(from data: Data) -> [String]? {
        let parts = String(decoding: data, as: UTF8.self).split(separator: "\n").map(String.init)
        guard parts.first == sentinel else { return nil }
        return Array(parts.dropFirst())
    }
}

struct LibraryTable: NSViewRepresentable {
    let tracks: [LibraryTrack]
    let columns: [LibraryTableColumnSpec]
    let rowSelection: LibraryRowSelection
    /// Bumped when in-memory row fields change without the id list
    /// changing — a colour label, a rating, an analysis patch. Without
    /// it those edits were saved and never repainted.
    let contentRevision: UInt64
    /// Resolves the live per-row values at configure time — the deck
    /// badges and the reachability verdict. Called for visible rows
    /// only, which is the point of the migration.
    let rowState: (LibraryTrack) -> LibraryRowState
    let rowActions: (LibraryTrack) -> LibraryRowActions
    /// One-shot scroll request: the row a reveal wants brought into
    /// view. Cleared through `onScrollHandled` once acted on, or the
    /// next update would scroll again on every unrelated change.
    var scrollToTrackId: String?
    /// The first search match from outside the list on screen: a line
    /// is drawn above it, between that list's matches and the rest.
    var dividerAboveTrackId: String?
    var callbacks = LibraryTableCallbacks()

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let table = LibraryNSTableView()
        table.dragChip = { [weak coordinator = context.coordinator] rows in
            MainActor.assumeIsolated { coordinator?.dragChip(rows: rows) }
        }
        table.style = .plain
        table.usesAlternatingRowBackgroundColors = false
        table.backgroundColor = NSColor(DubColor.surface0)
        table.gridStyleMask = []
        // Selection is drawn by `LibraryTintedRowView` as a flat
        // `surface2` fill, matching what the AppKit selection layer
        // painted before. The system accent would be a visible change.
        table.selectionHighlightStyle = .none
        table.allowsMultipleSelection = true
        table.allowsColumnResizing = true
        table.allowsColumnReordering = true
        table.columnAutoresizingStyle = .noColumnAutoresizing
        table.rowHeight = LibraryRowLayout.estimatedHeight
        // AppKit's default header is 28 pt for a 10 pt label; the
        // library's height is short on a laptop (2026-11-15).
        table.headerView = LibraryTableHeaderView(
            frame: NSRect(x: 0, y: 0, width: 0, height: LibraryRowLayout.headerHeight))
        table.intercellSpacing = .zero
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.menu = NSMenu()
        table.menu?.delegate = context.coordinator
        // The column picker. `NSTableHeaderView` has no menu of its
        // own, so right-clicking a header did nothing until this was
        // hung on it — the SwiftUI header it replaced carried the
        // picker in a `.contextMenu`.
        let headerMenu = NSMenu()
        headerMenu.delegate = context.coordinator
        table.headerView?.menu = headerMenu
        context.coordinator.headerMenu = headerMenu
        table.registerForDraggedTypes([LibraryCrateReorder.pasteboardType])
        // Rows drag out as file URLs and reorder in place; AppKit picks
        // the drag image from the real row views, which is what the old
        // per-column `onDrag` was approximating.
        table.setDraggingSourceOperationMask(.copy, forLocal: false)
        table.setDraggingSourceOperationMask(.move, forLocal: true)
        context.coordinator.table = table

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = NSColor(DubColor.surface0)
        scroll.borderType = .noBorder
        // One row of room past the last track: the status tab sits over
        // the right end of the bottom row, and at the end of a list the
        // last track scrolls clear of it. The header is tiled by the
        // scroll view itself, not by the top inset, so the automatic
        // insets have nothing else to do here.
        scroll.automaticallyAdjustsContentInsets = false
        scroll.contentInsets = NSEdgeInsets(
            top: 0, left: 0, bottom: LibraryRowLayout.estimatedHeight, right: 0)

        context.coordinator.syncColumns(columns)
        context.coordinator.observeColumnGeometry(table)
        return scroll
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        let coordinator = context.coordinator
        coordinator.parent = self
        guard let table = coordinator.table else { return }

        let ids = tracks.map(\.id)
        let tracksChanged = coordinator.lastTrackIds != ids
        coordinator.syncColumns(columns)
        // The text size is the DJ's setting; the rows follow it live.
        if table.rowHeight != LibraryRowLayout.estimatedHeight {
            table.rowHeight = LibraryRowLayout.estimatedHeight
            table.reloadData()
        }

        // The divider belongs to a row view, which a refill of the cells
        // does not rebuild.
        if coordinator.lastDividerAboveTrackId != dividerAboveTrackId {
            coordinator.lastDividerAboveTrackId = dividerAboveTrackId
            table.reloadData()
        }

        let refillStarted = LibraryListTiming.now
        if tracksChanged {
            coordinator.lastTrackIds = ids
            coordinator.refillRows(table)
            coordinator.applySelection()
        } else if coordinator.lastContentRevision != contentRevision {
            // Row *contents* changed under the same ids. The rows stay
            // where they are, and so does the selection.
            coordinator.refillRows(table)
        }
        LibraryListTiming.tableUpdated(refill: LibraryListTiming.now - refillStarted)
        coordinator.lastContentRevision = contentRevision
        coordinator.syncSelectionFromModel()

        // Last, so the row indices the scroll resolves against are the
        // ones just reloaded. The clear is deferred: `onScrollHandled`
        // writes the caller's `@State`, and doing that inside
        // `updateNSView` is a mutation during a view update.
        if let scrollToTrackId {
            coordinator.scrollToTrack(id: scrollToTrackId)
            DispatchQueue.main.async { self.callbacks.onScrollHandled() }
        }
    }

    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate,
        NSMenuDelegate {
        var parent: LibraryTable
        weak var table: NSTableView?
        var lastTrackIds: [String] = []
        var lastDividerAboveTrackId: String?
        /// Identity, so `menuNeedsUpdate` can tell the header's menu
        /// from the rows'. Both share this coordinator as delegate.
        weak var headerMenu: NSMenu?
        var lastContentRevision: UInt64 = 0
        private var lastSpecs: [LibraryTableColumnSpec] = []
        private var applyingSelection = false
        private var observers: [NSObjectProtocol] = []

        init(_ parent: LibraryTable) {
            self.parent = parent
        }

        deinit {
            observers.forEach(NotificationCenter.default.removeObserver)
        }

        // MARK: - Columns

        /// Add, remove, reorder and resize `NSTableColumn`s to match the
        /// specs. Identifiers are the field `rawValue`, which is also
        /// what the widths persist under.
        func syncColumns(_ specs: [LibraryTableColumnSpec]) {
            guard let table, specs != lastSpecs else { return }
            defer { lastSpecs = specs }

            // The indicator gutter is always the leading column and is
            // never persisted, sorted, resized or reordered.
            if table.tableColumn(withIdentifier: LibraryGutterColumn.identifier) == nil {
                let gutter = NSTableColumn(identifier: LibraryGutterColumn.identifier)
                gutter.width = LibraryGutterColumn.width
                gutter.minWidth = LibraryGutterColumn.width
                gutter.maxWidth = LibraryGutterColumn.width
                gutter.title = ""
                gutter.headerCell = LibraryHeaderTextCell(textCell: "")
                table.addTableColumn(gutter)
            }

            let wanted = specs.map { $0.field.rawValue }
                + [LibraryGutterColumn.identifier.rawValue]
            for column in table.tableColumns
            where !wanted.contains(column.identifier.rawValue) {
                table.removeTableColumn(column)
            }
            for spec in specs {
                let id = NSUserInterfaceItemIdentifier(spec.field.rawValue)
                let column: NSTableColumn
                if let existing = table.tableColumn(withIdentifier: id) {
                    column = existing
                } else {
                    column = NSTableColumn(identifier: id)
                    column.headerCell = LibraryHeaderTextCell(textCell: spec.title)
                    table.addTableColumn(column)
                }
                column.title = spec.title
                column.minWidth = spec.isPinned
                    ? spec.width : LibraryColumnLayout.minWidth
                column.maxWidth = spec.isPinned
                    ? spec.width : LibraryColumnLayout.maxWidth
                if abs(column.width - spec.width) > 0.5 { column.width = spec.width }
                (column.headerCell as? LibraryHeaderTextCell)?.apply(spec)
            }
            // Order the columns to match, with the gutter pinned first.
            let gutterIndex = table.column(withIdentifier: LibraryGutterColumn.identifier)
            if gutterIndex > 0 { table.moveColumn(gutterIndex, toColumn: 0) }
            for (offset, spec) in specs.enumerated() {
                let id = NSUserInterfaceItemIdentifier(spec.field.rawValue)
                let current = table.column(withIdentifier: id)
                let target = offset + 1
                if current >= 0, current != target {
                    table.moveColumn(current, toColumn: target)
                }
            }
            table.headerView?.needsDisplay = true
        }

        /// Column drags and resizes are AppKit-native; these push the
        /// results back so they persist.
        func observeColumnGeometry(_ table: NSTableView) {
            let center = NotificationCenter.default
            observers.append(center.addObserver(
                forName: NSTableView.columnDidResizeNotification,
                object: table, queue: .main
            ) { [weak self] note in
                MainActor.assumeIsolated {
                    guard let self,
                          let column = note.userInfo?["NSTableColumn"] as? NSTableColumn,
                          let field = LibraryColumnField(rawValue: column.identifier.rawValue)
                    else { return }
                    self.parent.callbacks.onColumnResized(field, column.width)
                }
            })
            observers.append(center.addObserver(
                forName: NSTableView.columnDidMoveNotification,
                object: table, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self, let table = self.table else { return }
                    let order = table.tableColumns
                        .filter { $0.identifier != LibraryGutterColumn.identifier }
                        .compactMap { LibraryColumnField(rawValue: $0.identifier.rawValue) }
                    self.parent.callbacks.onColumnsReordered(order)
                }
            })
        }

        // MARK: - Data

        func numberOfRows(in tableView: NSTableView) -> Int { parent.tracks.count }

        func tableView(
            _ tableView: NSTableView,
            viewFor tableColumn: NSTableColumn?,
            row: Int
        ) -> NSView? {
            guard let tableColumn, parent.tracks.indices.contains(row) else { return nil }
            let track = parent.tracks[row]
            let state = parent.rowState(track)
            let actions = parent.rowActions(track)

            let id = tableColumn.identifier
            let cell = (tableView.makeView(withIdentifier: id, owner: self) as? LibraryCellView)
                ?? LibraryCellView.make(column: id)
            fill(cell, column: id, state: state, actions: actions)
            return cell
        }

        private func fill(
            _ cell: LibraryCellView, column id: NSUserInterfaceItemIdentifier,
            state: LibraryRowState, actions: LibraryRowActions
        ) {
            if id == LibraryGutterColumn.identifier {
                (cell as? LibraryHostingCellView)?.configureGutter(state: state, actions: actions)
            } else if let field = LibraryColumnField(rawValue: id.rawValue) {
                cell.configure(field: field, state: state, actions: actions)
            }
            // A track already played tonight is dimmed. Not its gutter:
            // the deck badge there says where the track is now.
            let alpha: CGFloat = state.cell.played && id != LibraryGutterColumn.identifier
                ? LibraryRowLayout.playedAlpha : 1
            if cell.alphaValue != alpha { cell.alphaValue = alpha }
        }

        private func rowTint(_ track: LibraryTrack) -> NSColor? {
            DubColor.trackLabel(track.color).map { NSColor($0).withAlphaComponent(0.18) }
        }

        /// Show the current tracks in the rows the table already has.
        ///
        /// `reloadData()` takes every row view off the table and puts it
        /// back, and each one then carried a SwiftUI host per column: with
        /// thirty rows on screen that is some three hundred hosts leaving
        /// and re-entering the window, and it held the main thread for
        /// half a second after every click on a playlist
        /// (`MainThreadWatchdog`, 2026-11-20: `NSTableRowData
        /// _addRowViewForVisibleRow` → `-[NSView _setWindow:]` → SwiftUI).
        /// The cells are configured by value, so a row that is to show a
        /// different track only needs new values: nothing leaves the
        /// window. The table is told the count changed, which adds or
        /// drops rows at the end, and every row view it holds — on screen
        /// or prepared just off it — is given the track now at its index.
        func refillRows(_ table: NSTableView) {
            table.noteNumberOfRowsChanged()
            let columns = table.tableColumns.map(\.identifier)
            table.enumerateAvailableRowViews { rowView, row in
                guard self.parent.tracks.indices.contains(row) else { return }
                let track = self.parent.tracks[row]
                (rowView as? LibraryTintedRowView)?.tint = self.rowTint(track)
                let state = self.parent.rowState(track)
                let actions = self.parent.rowActions(track)
                for (index, id) in columns.enumerated() {
                    guard let cell = rowView.view(atColumn: index) as? LibraryCellView
                    else { continue }
                    self.fill(cell, column: id, state: state, actions: actions)
                }
            }
        }

        func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
            let id = NSUserInterfaceItemIdentifier("dub.row")
            let view = (tableView.makeView(withIdentifier: id, owner: self)
                as? LibraryTintedRowView) ?? {
                let fresh = LibraryTintedRowView()
                fresh.identifier = id
                return fresh
            }()
            if parent.tracks.indices.contains(row) {
                view.tint = rowTint(parent.tracks[row])
                view.dividerAbove = parent.tracks[row].id == parent.dividerAboveTrackId
            } else {
                view.tint = nil
                view.dividerAbove = false
            }
            return view
        }

        /// The sort is tri-state (ascending → descending → off) and
        /// client-side, so header clicks route here rather than through
        /// `sortDescriptors`.
        func tableView(_ tableView: NSTableView, didClick tableColumn: NSTableColumn) {
            guard tableColumn.identifier != LibraryGutterColumn.identifier,
                  let field = LibraryColumnField(rawValue: tableColumn.identifier.rawValue)
            else { return }
            parent.callbacks.onToggleSort(field)
        }

        // MARK: - Drag out

        /// Always vends the file URL — the contract `MainView
        /// .addDroppedURLsToCrate` and the decks' `dropDestination`
        /// both read. While the open crate is in manual order it also
        /// vends the in-process reorder payload, so one drag can either
        /// load a deck or reorder in place depending on where it lands.
        func tableView(
            _ tableView: NSTableView,
            pasteboardWriterForRow row: Int
        ) -> NSPasteboardWriting? {
            guard parent.tracks.indices.contains(row),
                  let url = parent.callbacks.dragURL(parent.tracks[row])
            else { return nil }
            let item = NSPasteboardItem()
            item.setString(url.absoluteString, forType: .fileURL)
            if parent.callbacks.crateReorderEnabled() {
                let ids = MainActor.assumeIsolated { self.draggedIds(primaryRow: row) }
                item.setData(
                    LibraryCrateReorder.payload(ids: ids),
                    forType: LibraryCrateReorder.pasteboardType)
            }
            return item
        }

        /// The drag picture: a compact `♪ Artist — Title` chip, placed by
        /// `LibraryNSTableView.beginDraggingSession` before the session
        /// starts. Setting it here, once the session had begun, is what
        /// made it fly in from off-screen: AppKit animates each item from
        /// the frame it began with to whatever it is given later — see
        /// `LibraryNSTableView`.
        @MainActor
        func dragChip(rows: [Int]) -> NSImage? {
            guard let first = rows.first, parent.tracks.indices.contains(first) else { return nil }
            return LibraryDragChip.image(for: parent.tracks[first], extraCount: rows.count - 1)
        }

        func tableView(
            _ tableView: NSTableView,
            draggingSession session: NSDraggingSession,
            willBeginAt screenPoint: NSPoint,
            forRowIndexes rowIndexes: IndexSet
        ) {
            // One chip, where it was put; no gathering the rows into it.
            session.draggingFormation = .none
            session.animatesToStartingPositionsOnCancelOrFail = false
        }

        /// The dragged ids in visual order. A multi-row selection moves
        /// as a contiguous block; a drag on an unselected row carries
        /// just that row (Finder semantics).
        @MainActor
        private func draggedIds(primaryRow: Int) -> [String] {
            let primary = parent.tracks[primaryRow].id
            let selected = parent.rowSelection.selectedTrackIds
            guard selected.contains(primary), selected.count > 1 else { return [primary] }
            return parent.tracks.map(\.id).filter { selected.contains($0) }
        }

        // MARK: - Reorder drop

        func tableView(
            _ tableView: NSTableView,
            validateDrop info: NSDraggingInfo,
            proposedRow row: Int,
            proposedDropOperation dropOperation: NSTableView.DropOperation
        ) -> NSDragOperation {
            guard dropOperation == .above,
                  MainActor.assumeIsolated({ self.parent.callbacks.crateReorderEnabled() }),
                  info.draggingPasteboard.data(
                      forType: LibraryCrateReorder.pasteboardType) != nil
            else { return [] }
            return .move
        }

        func tableView(
            _ tableView: NSTableView,
            acceptDrop info: NSDraggingInfo,
            row: Int,
            dropOperation: NSTableView.DropOperation
        ) -> Bool {
            guard let data = info.draggingPasteboard.data(
                      forType: LibraryCrateReorder.pasteboardType),
                  let ids = LibraryCrateReorder.ids(from: data), !ids.isEmpty
            else { return false }
            MainActor.assumeIsolated { self.parent.callbacks.onCrateReorder(ids, row) }
            return true
        }

        // MARK: - Right-click menu

        /// Built when the menu opens, against `clickedRow` — which also
        /// gives Finder's "right-click outside the selection acts on
        /// that row alone" semantics for free.
        func menuNeedsUpdate(_ menu: NSMenu) {
            menu.removeAllItems()
            if menu === headerMenu {
                guard let built = MainActor.assumeIsolated({
                    self.parent.callbacks.menuForHeader()
                }) else { return }
                adopt(built.items, into: menu)
                return
            }
            guard let table, table.clickedRow >= 0 else { return }
            let row = table.clickedRow
            guard let built = MainActor.assumeIsolated({
                self.parent.callbacks.menuForRow(row)
            }) else { return }
            adopt(built.items, into: menu)
        }

        /// An `NSMenuItem` belongs to one menu, so the built items are
        /// moved across rather than copied.
        private func adopt(_ items: [NSMenuItem], into menu: NSMenu) {
            for item in items {
                item.menu?.removeItem(item)
                menu.addItem(item)
            }
        }

        // MARK: - Scroll

        @MainActor
        func scrollToTrack(id: String) {
            guard let table, let row = parent.tracks.firstIndex(where: { $0.id == id })
            else { return }
            table.scrollRowToVisible(row)
        }

        // MARK: - Selection

        // `NSTableViewDelegate` is not `@MainActor`-typed, but AppKit
        // delivers this on the main thread. `assumeIsolated` is the
        // documented zero-cost hop for exactly this case — same as the
        // clip-view bounds observer in the old container.
        func tableViewSelectionDidChange(_ notification: Notification) {
            MainActor.assumeIsolated { selectionDidChange() }
        }

        @MainActor
        private func selectionDidChange() {
            guard !applyingSelection, let table else { return }
            let ids = table.selectedRowIndexes.compactMap { index -> String? in
                parent.tracks.indices.contains(index) ? parent.tracks[index].id : nil
            }
            parent.rowSelection.selectedTrackIds = Set(ids)
            parent.rowSelection.selectionAnchorId =
                table.selectedRow >= 0 && parent.tracks.indices.contains(table.selectedRow)
                    ? parent.tracks[table.selectedRow].id
                    : nil
            parent.callbacks.onSelectionChanged()
        }

        /// Push the model's selection into the table without echoing it
        /// straight back out through `tableViewSelectionDidChange`.
        @MainActor
        func syncSelectionFromModel() {
            guard let table else { return }
            let wanted = parent.rowSelection.selectedTrackIds
            let current = Set(table.selectedRowIndexes.compactMap { index -> String? in
                parent.tracks.indices.contains(index) ? parent.tracks[index].id : nil
            })
            guard wanted != current else { return }
            applySelection()
        }

        @MainActor
        func applySelection() {
            guard let table else { return }
            let wanted = parent.rowSelection.selectedTrackIds
            var indexes = IndexSet()
            for (index, track) in parent.tracks.enumerated() where wanted.contains(track.id) {
                indexes.insert(index)
            }
            applyingSelection = true
            table.selectRowIndexes(indexes, byExtendingSelection: false)
            applyingSelection = false
        }
    }
}

/// One cell of the track list: one column of one row.
///
/// Two kinds, chosen by the column and never by the row, so a column's
/// cells are all one class and the table can hand any of them to any
/// row: `LibraryTextCellView` for a column that is a line of text, and
/// `LibraryHostingCellView` for the ones that are more — the gutter's
/// badges, the BPM with its marks, the stars, the colour swatch.
class LibraryCellView: NSTableCellView {
    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// Show `field` of the row. Cheap when the cell already shows it.
    func configure(field: LibraryColumnField, state: LibraryRowState, actions: LibraryRowActions) {}

    /// The cell for a column, by its identifier (a field's `rawValue`, or
    /// the gutter's).
    static func make(column id: NSUserInterfaceItemIdentifier) -> LibraryCellView {
        if let field = LibraryColumnField(rawValue: id.rawValue), LibraryCellText.isText(field) {
            return LibraryTextCellView(identifier: id)
        }
        return LibraryHostingCellView(identifier: id)
    }
}

/// A cell that is one line of text, drawn by AppKit.
///
/// Every cell used to be a SwiftUI host. A host is a whole view graph of
/// its own, and giving it a new value costs a third of a millisecond
/// whatever the value is; a screen of rows is some three hundred of
/// them, which was 80 ms of every click on a playlist once the larger
/// costs were gone (`LibraryBench`, 2026-11-20). Most columns are a
/// string in a colour. For those this sets a label's text, which costs
/// about nothing, and the hosts are left to the columns that need them.
final class LibraryTextCellView: LibraryCellView {
    private struct Shown: Equatable {
        var text: LibraryCellText
        var size: CGFloat
    }

    private let label = NSTextField(labelWithString: "")
    private var shown: Shown?

    /// `NSTextFieldCell` sets its text this far in from each side of its
    /// frame. SwiftUI's `Text` does not, so the label's frame reaches
    /// that far out and the text stands where it stood.
    private static let fragmentPadding: CGFloat = 2

    override init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(identifier: identifier)
        // Not `usesSingleLineMode`: it sets the line by a baseline of its
        // own, 4 pt high in a row of 18 pt text. One line comes from the
        // paragraph style's truncation instead.
        label.cell?.truncatesLastVisibleLine = true
        label.autoresizingMask = []
        addSubview(label)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func configure(field: LibraryColumnField, state: LibraryRowState, actions: LibraryRowActions) {
        guard let text = LibraryCellText.make(field, state.cell) else { return }
        let next = Shown(text: text, size: LibraryRowLayout.textSize)
        guard next != shown else { return }
        let resized = shown?.size != next.size
        shown = next
        label.attributedStringValue = Self.attributed(text, size: next.size)
        let toolTip = (text.toolTip?.isEmpty ?? true) ? nil : text.toolTip
        if label.toolTip != toolTip { label.toolTip = toolTip }
        if resized { place() }
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        place()
    }

    /// The label with its line of text on the row's middle, as SwiftUI
    /// centres a `Text`, starting at the column's inset. The text field
    /// draws from the top of its frame, so the frame is hung from the
    /// top: half of what the row has over the line's own height above it.
    private func place() {
        let metrics = Self.metrics(shown?.size ?? LibraryRowLayout.textSize)
        let above = ((bounds.height - metrics.line) / 2 * 2).rounded() / 2
        let inset = LibraryColumnLayout.columnLeadingInset - Self.fragmentPadding
        label.frame = NSRect(
            x: inset, y: bounds.height - above - metrics.frame,
            width: max(0, bounds.width - inset + Self.fragmentPadding), height: metrics.frame)
    }

    /// A line's height in the system font — ascender to descender — and
    /// the whole points of frame that hold it.
    private static var knownMetrics: [CGFloat: (line: CGFloat, frame: CGFloat)] = [:]
    private static func metrics(_ size: CGFloat) -> (line: CGFloat, frame: CGFloat) {
        if let known = knownMetrics[size] { return known }
        let font = NSFont.systemFont(ofSize: size)
        let line = font.ascender - font.descender + font.leading
        let metrics = (line: line, frame: line.rounded(.up))
        knownMetrics[size] = metrics
        return metrics
    }

    private static let primary = NSColor(DubColor.textPrimary)
    private static let secondary = NSColor(DubColor.textSecondary)
    private static let tertiary = NSColor(DubColor.textTertiary)
    private static var keyColors: [String: NSColor] = [:]

    private static func color(_ ink: LibraryCellText.Ink) -> NSColor {
        switch ink {
        case .primary: return primary
        case .secondary: return secondary
        case .tertiary: return tertiary
        case .key(let notation):
            if let known = keyColors[notation] { return known }
            let color = DubColor.camelotKey(notation).map { NSColor($0) } ?? secondary
            keyColors[notation] = color
            return color
        }
    }

    private static func attributed(_ text: LibraryCellText, size: CGFloat) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = text.truncation
        paragraph.alignment = text.trailing ? .right : .left
        let font = text.digits
            ? NSFont.monospacedDigitSystemFont(ofSize: size, weight: .regular)
            : NSFont.systemFont(ofSize: size)
        let line = NSMutableAttributedString(
            string: text.text,
            attributes: [
                .font: font, .foregroundColor: color(text.ink),
                .kern: LibraryRowLayout.tracking, .paragraphStyle: paragraph,
            ])
        if let note = text.note {
            line.append(NSAttributedString(
                string: "  " + note,
                attributes: [
                    .font: NSFont.systemFont(ofSize: max(size - 3, 10)),
                    .foregroundColor: tertiary, .paragraphStyle: paragraph,
                ]))
        }
        return line
    }
}

/// A reused cell hosting the SwiftUI `LibraryColumnCell`.
///
/// Reconfiguring assigns a new `rootView` — a structural diff of one
/// small concrete view, not the `AnyView` re-host of an entire table
/// that this migration exists to remove. It is skipped when the cell
/// already shows what it is asked to: most rows have no badge, no stars
/// and no colour, and those cells look the same from one list to the
/// next. What the view fires goes through `actions`, which is always the
/// current row's, so a view kept from another row still acts on this one.
final class LibraryHostingCellView: LibraryCellView {
    /// What the host was last given, as far as it shows.
    private enum Shown: Equatable {
        case gutter(
            deckA: Bool, deckB: Bool, duplicate: Bool, warning: Bool, analysed: Bool, toolTip: String)
        case rating(Int)
        case color(String?)
        case cell(LibraryColumnField, LibraryCellState, size: CGFloat)
    }

    private let host: LibraryCellHost
    private var shown: Shown?
    private var actions = LibraryRowActions()

    override init(identifier: NSUserInterfaceItemIdentifier) {
        host = LibraryCellHost(rootView: AnyView(EmptyView()))
        super.init(identifier: identifier)
        host.translatesAutoresizingMaskIntoConstraints = false
        addSubview(host)
        NSLayoutConstraint.activate([
            // Leading inset only, matching the row cells. The header's
            // extra trailing inset is deliberate and lives there.
            host.leadingAnchor.constraint(
                equalTo: leadingAnchor, constant: LibraryColumnLayout.columnLeadingInset),
            host.trailingAnchor.constraint(equalTo: trailingAnchor),
            host.topAnchor.constraint(equalTo: topAnchor),
            host.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configureGutter(state: LibraryRowState, actions: LibraryRowActions) {
        self.actions = actions
        let next = Shown.gutter(
            deckA: state.isOnDeckA, deckB: state.isOnDeckB,
            duplicate: state.track.potentialDuplicateId != nil,
            warning: state.showsUnreachableWarning, analysed: state.showsAnalysedMark,
            toolTip: state.unreachableTooltip)
        guard next != shown else { return }
        shown = next
        host.rootView = AnyView(
            LibraryRowIndicators(
                state: state, onDuplicateJump: { [weak self] in self?.actions.onDuplicateJump() }))
    }

    override func configure(
        field: LibraryColumnField,
        state: LibraryRowState,
        actions: LibraryRowActions
    ) {
        self.actions = actions
        let next: Shown
        switch field {
        case .rating: next = .rating(Int(state.track.rating ?? 0))
        case .color: next = .color(state.track.color)
        default: next = .cell(field, state.cell, size: LibraryRowLayout.textSize)
        }
        guard next != shown else { return }
        shown = next
        let through = LibraryCellActions(
            onRate: { [weak self] rating in self?.actions.cell.onRate(rating) },
            onColor: { [weak self] token in self?.actions.cell.onColor(token) })
        host.rootView = AnyView(
            LibraryColumnCell(field: field, state: state.cell, actions: through)
                .frame(maxWidth: .infinity, alignment: .leading))
    }
}

/// A cell's SwiftUI host, telling `LibraryListTiming` what it costs: its
/// layout, and each time it enters or leaves the window.
final class LibraryCellHost: NSHostingView<AnyView> {
    override func layout() {
        guard LibraryListTiming.isTiming else { return super.layout() }
        let started = LibraryListTiming.now
        super.layout()
        LibraryListTiming.cellLaidOut(took: LibraryListTiming.now - started)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        LibraryListTiming.cellMoved()
    }
}

/// Draws the selection fill and the colour-label tint, in that order.
///
/// Both live here because the ordering is load-bearing: the tint sits
/// *above* the selection at 0.18 opacity, which is what keeps a
/// selected coloured row legible. Before the migration the selection
/// was an AppKit layer beneath the SwiftUI host and the tint a SwiftUI
/// background — same stack, two places.
final class LibraryTintedRowView: NSTableRowView {
    var tint: NSColor? {
        didSet { if tint != oldValue { needsDisplay = true } }
    }

    /// The line between a search's matches from the list on screen and
    /// its matches from the rest of the library.
    var dividerAbove = false {
        didSet { if dividerAbove != oldValue { needsDisplay = true } }
    }

    override var isSelected: Bool {
        didSet { if isSelected != oldValue { needsDisplay = true } }
    }

    override func drawBackground(in dirtyRect: NSRect) {
        super.drawBackground(in: dirtyRect)
        if isSelected {
            NSColor(DubColor.surface2).setFill()
            bounds.fill()
        }
        if let tint {
            tint.setFill()
            bounds.fill()
        }
        if dividerAbove {
            NSColor(DubColor.textTertiary).setFill()
            let top = isFlipped ? bounds.minY : bounds.maxY - 2
            NSRect(x: 0, y: top, width: bounds.width, height: 2).fill()
        }
    }

    /// Selection is painted in `drawBackground` so the tint can sit on
    /// top of it; this must not paint again.
    override func drawSelection(in dirtyRect: NSRect) {}


}

/// The header background and hairline. Column labels are drawn by
/// `LibraryHeaderTextCell`.
final class LibraryTableHeaderView: NSTableHeaderView {
    override func draw(_ dirtyRect: NSRect) {
        NSColor(DubColor.surface1).setFill()
        dirtyRect.fill()
        super.draw(dirtyRect)
        NSColor(DubColor.divider).setFill()
        NSRect(x: 0, y: bounds.maxY - 1, width: bounds.width, height: 1).fill()
    }
}

/// One column header: the caps label, its sort chevron, and the
/// trailing divider — drawn to match `LibraryHeaderCell`, which the
/// snapshot baselines pin.
final class LibraryHeaderTextCell: NSTableHeaderCell {
    private var isSortActive = false
    private var sortAscending = true

    func apply(_ spec: LibraryTableColumnSpec) {
        stringValue = spec.title
        isSortActive = spec.isSortActive
        sortAscending = spec.sortAscending
    }

    override func draw(withFrame cellFrame: NSRect, in controlView: NSView) {
        NSColor(DubColor.surface1).setFill()
        cellFrame.fill()

        let colour = NSColor(isSortActive ? DubColor.textPrimary : DubColor.textSecondary)
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 10, weight: .semibold),
            .foregroundColor: colour,
            .kern: DubFont.capsTracking,
        ]
        let text = stringValue.uppercased() as NSString
        let size = text.size(withAttributes: attributes)
        let x = cellFrame.minX + LibraryColumnLayout.columnLeadingInset
        let available = cellFrame.width
            - LibraryColumnLayout.columnLeadingInset
            - LibraryColumnLayout.columnTrailingInset
        text.draw(
            in: NSRect(
                x: x, y: cellFrame.midY - size.height / 2,
                width: max(0, min(size.width, available)), height: size.height),
            withAttributes: attributes)

        if isSortActive {
            let symbol = sortAscending ? "chevron.up" : "chevron.down"
            let config = NSImage.SymbolConfiguration(pointSize: 8, weight: .bold)
            if let image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
                .withSymbolConfiguration(config) {
                let point = NSPoint(
                    x: x + min(size.width, available) + 3,
                    y: cellFrame.midY - image.size.height / 2)
                image.draw(
                    in: NSRect(origin: point, size: image.size),
                    from: .zero, operation: .sourceOver, fraction: 1,
                    respectFlipped: true,
                    hints: [.interpolation: NSImageInterpolation.high])
            }
        }

        NSColor(DubColor.divider).setFill()
        NSRect(x: cellFrame.maxX - 1, y: cellFrame.minY, width: 1, height: cellFrame.height)
            .fill()
    }
}

/// Identifier for the leading indicator gutter. Not a `LibraryColumnField`:
/// it is never persisted, sorted, resized or reordered, and
/// `LibraryColumnField.init?(rawValue:)` deliberately rejects ids the
/// registry does not know. `NSTableView` still needs it to be a column.
enum LibraryGutterColumn {
    static let identifier = NSUserInterfaceItemIdentifier("dub.gutter")
    static let width = LibraryColumnLayout.gutterWidth
}


/// The picture a dragged library row shows: `♪ Artist — Title` on a
/// tinted chip. Deliberately small — it follows the pointer, so it has
/// to say what is being dragged without covering the drop target.
enum LibraryDragChip {
    static func image(for track: LibraryTrack, extraCount: Int = 0) -> NSImage {
        let artist = track.artist ?? "—"
        var text = "\(artist) — \(LibraryCellFormat.displayTitle(track))"
        if extraCount > 0 { text += "   +\(extraCount)" }

        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byTruncatingTail
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 11, weight: .medium),
            .foregroundColor: NSColor(DubColor.textPrimary),
            .paragraphStyle: paragraph,
        ]

        let padding: CGFloat = 8
        let icon: CGFloat = 11
        let gap: CGFloat = 6
        let height: CGFloat = 24
        let natural = (text as NSString).size(withAttributes: attributes).width
        let textWidth = min(natural, 260)
        let width = padding + icon + gap + textWidth + padding

        let image = NSImage(size: NSSize(width: width, height: height))
        image.lockFocus()
        let body = NSRect(x: 0, y: 0, width: width, height: height)
            .insetBy(dx: 0.5, dy: 0.5)
        let path = NSBezierPath(roundedRect: body, xRadius: 4, yRadius: 4)
        NSColor(DubColor.surface2).setFill()
        path.fill()
        NSColor(DubColor.deckATint).setStroke()
        path.lineWidth = 1
        path.stroke()

        if let note = NSImage(systemSymbolName: "music.note", accessibilityDescription: nil)?
            .withSymbolConfiguration(
                NSImage.SymbolConfiguration(pointSize: icon, weight: .medium))?
            .withSymbolConfiguration(
                NSImage.SymbolConfiguration(paletteColors: [NSColor(DubColor.deckATint)]))
        {
            note.draw(in: NSRect(
                x: padding, y: (height - icon) / 2 - 1, width: icon, height: icon))
        }
        (text as NSString).draw(
            in: NSRect(
                x: padding + icon + gap, y: (height - 14) / 2,
                width: textWidth, height: 14),
            withAttributes: attributes)
        image.unlockFocus()
        return image
    }
}

/// The library's table, for one reason: to place the drag image *before*
/// the drag session begins.
///
/// The chip flew in from off-screen for months. Every attempt set it in
/// `draggingSession(_:willBeginAt:)`, and every one of those frames read
/// back correct — but by then AppKit had started the session with the
/// table's own row images at their row positions, and it animates an
/// item from the frame it began with to any it is given later. The table
/// begins its drag through this view's own
/// `beginDraggingSession(with:event:source:)`, so the items can be given
/// their final frame and image here, and there is nothing to animate
/// from: the chip is under the pointer where the row was grabbed, from
/// the first frame, the way Finder's is (rig, 2026-09-25).
final class LibraryNSTableView: NSTableView {
    /// The chip for the rows being dragged, in drag order.
    var dragChip: ([Int]) -> NSImage? = { _ in nil }

    override func beginDraggingSession(
        with items: [NSDraggingItem], event: NSEvent, source: NSDraggingSource
    ) -> NSDraggingSession {
        let point = convert(event.locationInWindow, from: nil)
        let grabbed = row(at: point)
        let rows: [Int]
        if grabbed >= 0, selectedRowIndexes.contains(grabbed) {
            rows = [grabbed] + selectedRowIndexes.filter { $0 != grabbed }
        } else {
            rows = grabbed >= 0 ? [grabbed] : Array(selectedRowIndexes)
        }
        if let chip = dragChip(rows) {
            for (index, item) in items.enumerated() {
                if index == 0 {
                    // The pointer sits on the chip's note, as it sits on a
                    // file's icon in a Finder drag.
                    let frame = NSRect(
                        x: point.x - 14, y: point.y - chip.size.height / 2,
                        width: chip.size.width, height: chip.size.height)
                    item.setDraggingFrame(frame, contents: chip)
                } else {
                    // One chip for the whole drag; the chip says how many.
                    item.setDraggingFrame(NSRect(origin: point, size: .zero), contents: nil)
                }
            }
        }
        Self.log.notice("drag begins with \(items.count, privacy: .public) item(s), chip placed before the session")
        return super.beginDraggingSession(with: items, event: event, source: source)
    }

    override func layout() {
        LibraryListTiming.tableLayout()
        super.layout()
    }

    private static let log = Logger(subsystem: "com.dub.app", category: "drag")
}
