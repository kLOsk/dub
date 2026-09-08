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
}

struct LibraryTable: NSViewRepresentable {
    let tracks: [LibraryTrack]
    let columns: [LibraryTableColumnSpec]
    let rowSelection: LibraryRowSelection
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
        } else if coordinator.lastContentRevision != coordinator.pendingContentRevision {
            table.reloadData()
            coordinator.applySelection()
        }
        coordinator.lastContentRevision = coordinator.pendingContentRevision
        coordinator.syncSelectionFromModel()
    }

    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var parent: LibraryTable
        weak var table: NSTableView?
        var lastTrackIds: [String] = []
        var lastContentRevision: UInt64 = 0
        var pendingContentRevision: UInt64 = 0
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

            let wanted = specs.map { $0.field.rawValue }
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
            // Order the columns to match.
            for (target, spec) in specs.enumerated() {
                let id = NSUserInterfaceItemIdentifier(spec.field.rawValue)
                let current = table.column(withIdentifier: id)
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
                    let order = table.tableColumns.compactMap {
                        LibraryColumnField(rawValue: $0.identifier.rawValue)
                    }
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
