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
//  Cell content is still the same SwiftUI — `LibraryColumnCell`,
//  `LibraryRowView`'s indicators, `LibraryHeaderCell` — hosted in
//  reused cells. That is why those three were made value-driven first:
//  a reused cell is configured with values and must not consult a live
//  object afterwards. `LibraryCellSnapshotTests` is the baseline.
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
    var callbacks = LibraryTableCallbacks()

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let table = NSTableView()
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
        table.headerView = LibraryTableHeaderView()
        table.intercellSpacing = .zero
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.menu = NSMenu()
        table.menu?.delegate = context.coordinator
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

        if tracksChanged {
            coordinator.lastTrackIds = ids
            table.reloadData()
            coordinator.applySelection()
        } else if coordinator.lastContentRevision != contentRevision {
            // Row *contents* changed under the same ids. Reloading data
            // keeps the selection, so it does not need re-applying.
            table.reloadData()
        }
        coordinator.lastContentRevision = contentRevision
        coordinator.syncSelectionFromModel()
    }

    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate,
        NSMenuDelegate {
        var parent: LibraryTable
        weak var table: NSTableView?
        var lastTrackIds: [String] = []
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
            let cell = (tableView.makeView(withIdentifier: id, owner: self)
                as? LibraryHostingCellView) ?? LibraryHostingCellView(identifier: id)
            if id == LibraryGutterColumn.identifier {
                cell.configureGutter(state: state, actions: actions)
            } else if let field = LibraryColumnField(rawValue: id.rawValue) {
                cell.configure(field: field, state: state, actions: actions)
            }
            return cell
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
                view.tint = DubColor.trackLabel(parent.tracks[row].color)
                    .map { NSColor($0).withAlphaComponent(0.18) }
            } else {
                view.tint = nil
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

        /// The drag picture: a compact `♪ Artist — Title` chip.
        ///
        /// **KNOWN ISSUE — the image animates in from off-screen.** The
        /// chip's content and drop behaviour are correct; only its
        /// entrance is wrong. Ruled out by measurement, so do not
        /// re-try these:
        ///
        ///   * `draggingFrame` is applied and correct. Logged set vs.
        ///     read-back: identical, centred on the pointer in screen
        ///     coordinates, e.g. set (618,1791,293,24) with the pointer
        ///     at (775,1803).
        ///   * Coordinate space is not the cause. Table-space with
        ///     `for: tableView`, and screen-space with `for: nil`, both
        ///     fly. The start position tracked the *display* the window
        ///     was on, which is what identified the space.
        ///   * `setDraggingFrame(_:contents:)` is ignored outright;
        ///     only the `draggingFrame` property takes.
        ///   * `draggingFormation = .none` and
        ///     `animatesToStartingPositionsOnCancelOrFail = false` do
        ///     not stop it.
        ///   * Overriding `draggingImageComponents` on `NSTableCellView`
        ///     yields no components at all (drag shows only the drop
        ///     badge); `NSTableRowView` has no such property.
        ///   * There is no competing SwiftUI drag — `trackRowsStack`
        ///     and its `.onDrag` are dead code on this path.
        ///
        /// Next thing to try is an isolated sample project, not another
        /// substitution in here.
        ///
        /// Two things matter here, both learned the hard way.
        ///
        /// `imageComponentsProvider` rather than an override of
        /// `NSTableCellView.draggingImageComponents` — that override
        /// was never collected and the drag showed only the drop badge.
        /// AppKit calls this closure itself, when it needs the image.
        ///
        /// And the frame is in **screen coordinates**, centred on the
        /// pointer. `enumerateDraggingItems(for:)` is documented to
        /// re-base frames into the given view's space, but it does not
        /// here: passing the table and a table-space point put the chip
        /// at that point on the *desktop*, which showed up as the image
        /// flying in from a different screen edge depending on which
        /// display the window was on. Passing `nil` asks for screen
        /// coordinates explicitly, and `screenPoint` is already in them.
        ///
        /// AppKit animates the image from whatever frame the item
        /// carries to the cursor, so any other position visibly flies.
        /// Note it only takes via the `draggingFrame` property;
        /// `setDraggingFrame(_:contents:)` was ignored outright.
        func tableView(
            _ tableView: NSTableView,
            draggingSession session: NSDraggingSession,
            willBeginAt screenPoint: NSPoint,
            forRowIndexes rowIndexes: IndexSet
        ) {
            let rows = Array(rowIndexes)
            let tracks = MainActor.assumeIsolated { self.parent.tracks }
            guard let first = rows.first, tracks.indices.contains(first) else { return }
            let chip = LibraryDragChip.image(
                for: tracks[first], extraCount: rows.count - 1)
            let centre = screenPoint

            // `.none` is load-bearing. The default formation *gathers*
            // the items toward the cursor, and that gather is the
            // animation — it runs off the frames AppKit had before this
            // method ran, so setting a correct frame here did nothing.
            // Verified: the frame reads back exactly as set, centred on
            // the pointer, and the image still flew in until this line.
            session.draggingFormation = .none
            session.animatesToStartingPositionsOnCancelOrFail = false
            session.enumerateDraggingItems(
                options: [], for: nil, classes: [NSPasteboardItem.self],
                searchOptions: [:]
            ) { item, index, _ in
                guard index == 0 else {
                    // One chip for the whole drag; the rest collapse
                    // into it rather than stacking N images.
                    item.imageComponentsProvider = nil
                    item.draggingFrame = NSRect(origin: centre, size: .zero)
                    return
                }
                item.draggingFrame = NSRect(
                    x: centre.x - chip.size.width / 2,
                    y: centre.y - chip.size.height / 2,
                    width: chip.size.width,
                    height: chip.size.height)
                item.imageComponentsProvider = {
                    let component = NSDraggingImageComponent(key: .icon)
                    component.contents = chip
                    component.frame = NSRect(origin: .zero, size: chip.size)
                    return [component]
                }
            }
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
            guard let table, table.clickedRow >= 0 else { return }
            let row = table.clickedRow
            guard let built = MainActor.assumeIsolated({
                self.parent.callbacks.menuForRow(row)
            }) else { return }
            for item in built.items {
                built.removeItem(item)
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

/// A reused cell hosting the SwiftUI `LibraryColumnCell`.
///
/// Reconfiguring assigns a new `rootView` — a structural diff of one
/// small concrete view, not the `AnyView` re-host of an entire table
/// that this migration exists to remove.
final class LibraryHostingCellView: NSTableCellView {
    private let host: NSHostingView<AnyView>

    init(identifier: NSUserInterfaceItemIdentifier) {
        host = NSHostingView(rootView: AnyView(EmptyView()))
        super.init(frame: .zero)
        self.identifier = identifier
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
        host.rootView = AnyView(
            LibraryRowIndicators(state: state, onDuplicateJump: actions.onDuplicateJump))
    }

    func configure(
        field: LibraryColumnField,
        state: LibraryRowState,
        actions: LibraryRowActions
    ) {
        host.rootView = AnyView(
            LibraryColumnCell(field: field, state: state.cell, actions: actions.cell)
                // Per cell, inside the column — an unanalyzed row dims
                // its text but not the gutter badges or the row tint.
                .modifier(DimUnanalyzed(isAnalyzed: state.track.isAnalyzed))
                .frame(maxWidth: .infinity, alignment: .leading))
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
