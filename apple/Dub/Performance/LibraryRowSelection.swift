import Foundation

/// Selection state for the library row list.
///
/// ## Why this is a class held in `@State`
///
/// `LibraryView` holds a `LibraryRowSelection` instance in `@State`.
/// `@State` preserves the reference across body re-evaluations, so the
/// same selection outlives every `LibraryView.body` invocation — but
/// because the state *is* the reference, mutating a property on it
/// invalidates nothing. That is the entire point of the type: the
/// selection can change without SwiftUI re-rendering anything.
///
/// It is deliberately **not** an `ObservableObject`. Nothing observes
/// it: `NSTableView` owns both the selection and the highlight, and
/// pushes changes out through `LibraryTableCallbacks.onSelectionChanged`
/// rather than through a publisher. Adding `@Published` back would
/// re-open the path this type exists to close.
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
/// By routing the selection through a non-observed class reference,
/// the click handler mutates it without invalidating any SwiftUI view
/// in the tree, and `syncModelPrimarySelection()` is called explicitly
/// at every write site so the model-side `librarySelection` keeps up.
@MainActor
final class LibraryRowSelection {
    /// Currently selected row ids (canonical UUIDs). Cmd+click
    /// toggles membership; Shift+click selects a contiguous range
    /// in the current sort order. The primary id drives
    /// Space-load.
    var selectedTrackIds: Set<String> = []

    /// Anchor for Shift+click range selection in the current sort
    /// order.
    var selectionAnchorId: String? = nil
}
