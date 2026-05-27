import Combine
import Foundation

/// Selection state for the library row list.
///
/// ## Why this is an `ObservableObject` owned via `@State`
///
/// In `LibraryView` we hold a `LibraryRowSelection` instance using
/// `@State`, **not** `@StateObject` or `@ObservedObject`. That is
/// the entire point of the type: `@State` preserves the class
/// reference across body re-evaluations (so the same selection
/// outlives every `LibraryView.body` invocation), but `@State`
/// does **not** subscribe to the class's `objectWillChange`
/// publisher. As a result, mutating
/// `rowSelection.selectedTrackIds` or
/// `rowSelection.selectionAnchorId` does not trigger a
/// `LibraryView` body re-eval.
///
/// Subviews that need to *observe* the selection (e.g. the
/// footer's "N selected" label, or any row-decoration view that
/// renders highlight chrome) declare it as `@ObservedObject` and
/// will re-render on each change. The track row list itself uses
/// the AppKit `LibrarySelectionLayerView` to paint the highlight
/// directly, bypassing SwiftUI entirely, so it doesn't need to
/// observe at all.
///
/// ## What problem this solves
///
/// Pre-fix, every row click cascaded through `LibraryView.body`
/// because `selectedTrackIds` and `selectionAnchorId` lived as
/// `@State` on the view. The re-eval reconstructed the entire
/// HStack (sidebar + divider + right pane + footer), allocated
/// new `AnyView` wrappers around the row list and header, and
/// rebuilt all of SwiftUI's modifier chain. That ~3 to 5 ms of
/// main-thread work was enough to drop a Metal vsync during
/// waveform playback, which the user saw as a "waveform jump on
/// row click".
///
/// By routing the selection state through a non-observed class
/// reference, the click handler can mutate selection state
/// without invalidating any SwiftUI view in the tree. The
/// `LibrarySelectionLayerView` repaints from its coordinator on
/// the same tick (via the
/// `LibraryTableScrollContainer.updateNSView` path), and
/// `syncModelPrimarySelection()` is called explicitly at every
/// write site so the model-side `librarySelection` keeps up.
@MainActor
final class LibraryRowSelection: ObservableObject {
    /// Currently selected row ids (canonical UUIDs). Cmd+click
    /// toggles membership; Shift+click selects a contiguous range
    /// in the current sort order. The primary id drives
    /// Space-load.
    @Published var selectedTrackIds: Set<String> = []

    /// Anchor for Shift+click range selection in the current sort
    /// order.
    @Published var selectionAnchorId: String? = nil
}
