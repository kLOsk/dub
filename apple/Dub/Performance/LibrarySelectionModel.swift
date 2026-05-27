//
//  LibrarySelectionModel.swift
//  Dub
//
//  Per-app `ObservableObject` carrying the three "currently
//  selected library row" fields that previously lived on
//  `LibraryAppModel`. Split out for one reason — `LibraryView`
//  observes `LibraryAppModel`, but never *reads* these three
//  fields from its body (it only *writes* them via
//  `syncModelPrimarySelection`). Pre-split, writing to them
//  from inside `.onChange(of: selectedTrackIds)` round-tripped
//  back into a full `LibraryView` body re-evaluation cascade —
//  the residual "small glitch in the waveform when I select a
//  track" the user reported even after the M11d.6 round-1
//  `browserSelection` move and the row-click Task-deferral
//  collapse.
//
//  Moving the fields onto a model that `LibraryView` does NOT
//  observe means a row click is now a single body re-eval
//  (the local `@State selectedTrackIds` change) instead of two
//  (local `@State` change + the libraryModel cascade that
//  followed in the same tick). `MainView`'s `selectLibraryTrack`
//  / `loadBrowserSelectionIntoTargetDeck` / `recordLoadHistory`
//  paths read these fields directly off the new model; their
//  call sites are not on the playing-deck redraw critical path,
//  so the `objectWillChange` cascade from this model can be
//  noisier without affecting waveform smoothness.
//
//  `WaveformAppModel` owns the single instance via a `let`
//  (model identity stable for the app lifetime). All mutation
//  is `@MainActor`-isolated — there is exactly one writer
//  (`selectLibraryTrack`) plus the relocate / volume-reach
//  helpers that occasionally null the selection when a missing
//  file is dropped.
//

import Foundation
import DubCore

/// Three-field side-channel for "what library row is selected
/// right now". Observed by `MainView` consumers that *need* to
/// react (no current SwiftUI surface needs a live binding —
/// `MainView` reads these directly inside `loadTrack`-side
/// helpers). Deliberately NOT observed by `LibraryView`, which
/// is where the cascade-cost win lives.
@MainActor
final class LibrarySelectionModel: ObservableObject {

    /// File the user has highlighted in the library list or FS
    /// browser. `Space` loads this into the non-master, stopped
    /// deck (PRD §5.5).
    ///
    /// Previously lived on `LibraryAppModel`; moved here so a
    /// row click doesn't invalidate `LibraryView` a second time
    /// after the local `selectedTrackIds` `@State` write already
    /// scheduled a body re-eval. `FileBrowserView` observes the
    /// new model directly through `@ObservedObject var
    /// librarySelection` and so still sees `browserSelection`
    /// changes live for preview builds.
    @Published var browserSelection: URL? = nil

    /// Canonical UUID of the LibraryView's currently selected
    /// row, or `nil` when the current selection is a Finder drag.
    /// Used by [`recordLibraryLoadIfApplicable`] to decide
    /// whether a successful `loadTrack` deserves a
    /// `play_history` row.
    @Published var selectedLibraryTrackId: String? = nil

    /// M11c.2 — full row snapshot for the currently-selected
    /// library track. `LibraryView` writes this alongside
    /// `selectedLibraryTrackId` so the load path can stamp the
    /// track's key (and any future per-track-attribute) onto
    /// `DeckState` without an extra FFI round-trip.
    ///
    /// `nil` when no library row is selected (Finder-drag
    /// selection clears it). The snapshot is intentionally
    /// untracked vs. live library mutations: if the user
    /// analyzes the track *after* selecting but *before*
    /// loading it, the cached key may lag by one analysis
    /// cycle. The 10 Hz position poll's grid refresh covers BPM
    /// staleness for the same window; key staleness is a known
    /// minor cost of avoiding a per-load FFI lookup.
    @Published var selectedLibraryTrack: LibraryTrack? = nil
}
