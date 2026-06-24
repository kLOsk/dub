//
//  LibraryAppModel.swift
//  Dub
//
//  Per-library `ObservableObject` carrying every `@Published` field
//  the LibraryView (and the import / analysis / relocate sub-flows
//  it owns) reads or writes. Split out of `WaveformAppModel` so a
//  library mutation (import progress tick, analysis-generation
//  bump, row patch, missing-track recount, …) does **not** fire
//  `WaveformAppModel.objectWillChange` and therefore does not
//  invalidate `PerformanceView`. Pre-split, every analysis row
//  patch rebuilt the entire performance surface (both
//  `WaveformView` wrappers, both `TrackOverviewView`s, both
//  `DeckHeader`s, the FX bar, the status strip, and the 3 049-line
//  `LibraryView` itself). The Metal renderer kept its own state
//  cache so it wasn't *visually* affected, but the SwiftUI
//  invalidation cascade still ran on the main actor and competed
//  with the renderer for runloop time — the kind of contention
//  that takes a couple of frames off waveform smoothness during
//  long imports or batch-analyze runs.
//
//  WaveformAppModel still owns the `library` connection itself
//  (it sits on the actor's `init` and outlives the LibraryView's
//  lifetime). This model just publishes the UI-observable bits
//  that derive from library / analysis activity. Consumers:
//
//  * `LibraryView` — observes via `@ObservedObject`, reads all
//    fields, drives selection writes.
//  * `WaveformAppModel` — owns a single instance as a `let`
//    (model identity stable for the app lifetime), mutates the
//    fields from its import / scanner / analysis / relocate
//    helpers.
//

import Foundation
import DubCore

/// The external DJ apps Dub imports from, each surfaced as a sidebar
/// node under "Imported Sources". `rawValue` is the schema `source`
/// tag every FFI call uses (`listImportedCrates`, `listTracksBySource`,
/// `importSerato`, …), so it must match the Rust `source` strings.
enum ImportedSourceKind: String, CaseIterable, Hashable, Identifiable {
    case serato
    case traktor
    case itunes

    var id: String { rawValue }

    /// Schema / FFI `source` tag.
    var sourceTag: String { rawValue }

    /// Sidebar display name.
    var label: String {
        switch self {
        case .serato:  return "Serato"
        case .traktor: return "Traktor"
        // The app is "Apple Music" now; the library file is still the
        // iTunes-format XML it (or legacy iTunes) wrote.
        case .itunes:  return "Apple Music"
        }
    }

    /// SF Symbol for the source's sidebar row.
    var systemImage: String {
        switch self {
        case .serato:  return "s.square.fill"
        case .traktor: return "t.square.fill"
        case .itunes:  return "music.note"
        }
    }
}

/// One imported source for the sidebar: its "all tracks from this
/// source" count plus its flat crate/playlist tree (nesting via
/// `parentId`, document order). Built by
/// `WaveformAppModel.reloadImportedSources()` from the FFI mirror.
struct ImportedSourceGroup: Identifiable {
    let kind: ImportedSourceKind
    var trackCount: UInt64
    var crates: [LibraryImportedCrate]
    var id: ImportedSourceKind { kind }
}

/// Library / analysis / relocate UI state split out of
/// `WaveformAppModel` so updates here don't invalidate
/// `PerformanceView`. See file header for the why.
///
/// All mutation happens on the main actor — every writer is a
/// `@MainActor`-isolated method on `WaveformAppModel` (the
/// importer, scanner, analyzer, relocate helper). The class is
/// `@MainActor` to keep that invariant explicit in the type
/// system. Fields use plain `@Published var` (no `private(set)`)
/// because there is exactly one writer (`WaveformAppModel`) and
/// the call sites read more naturally with direct assignment
/// (`libraryModel.libraryTrackCount = count`) than through a
/// setter chain. The single-writer invariant is a documentation
/// concern, not a type-system one.
@MainActor
/// M11d-history — one reveal-in-browser request, emitted when the
/// DJ clicks the deck header's "↝ usually: <track>" hint (PRD §9.5
/// row 3). `token` differs per click so revealing the same track
/// twice still fires the browser's `.onChange`.
struct LibraryRevealRequest: Equatable {
    let trackId: String
    let token: UUID
}

final class LibraryAppModel: ObservableObject {

    /// `true` once `library.openDefault()` has succeeded. Drives
    /// the browser's "Open library" affordance — until this flips,
    /// the LibraryView shows a one-shot "preparing library…"
    /// placeholder rather than a blank list (which a DJ would read
    /// as "Dub forgot everything").
    @Published var libraryIsOpen: Bool = false

    /// M11d-history — pending reveal-in-browser request from the
    /// deck header's hint click. LibraryView consumes it via
    /// `.onChange`: select + scroll when the track is listed,
    /// falling back to All Tracks with the search cleared.
    @Published var revealTrackRequest: LibraryRevealRequest? = nil

    /// Total canonical-track count, refreshed after every import.
    /// Browser footer reads this directly.
    @Published var libraryTrackCount: UInt64 = 0

    /// Most recent import outcome, surfaced in the LibraryView
    /// footer for ~5 s after an import-folder run completes.
    /// `nil` while no import has run this session.
    @Published var lastImportSummary: LibraryImportSummary? = nil

    /// `true` while an import is in progress. Drives the
    /// browser's progress indicator and disables the
    /// "Import Folder…" button to prevent overlapping runs (the
    /// importer is safe to run twice but the UX is confusing).
    @Published var libraryImportInProgress: Bool = false

    /// M11d.3 — per-volume reachability cache. The LibraryView
    /// reads this to drive the missing-file indicator without a
    /// `FileManager.fileExists` round-trip per visible row. Keys
    /// are mount paths (e.g. `"/"`, `"/Volumes/Touring SSD"`);
    /// values are `true` when the mount point is a directory
    /// that currently exists, `false` when it does not. A track
    /// is missing iff its primary volume's mount point is absent
    /// from the cache (no recorded answer yet) or maps to
    /// `false`. Recomputed on a coarse cadence in
    /// `refreshVolumeReachability()` rather than per-keystroke
    /// or per-frame.
    @Published var volumeReachability: [String: Bool] = [:]

    /// M11d.4 — count of canonical tracks whose every
    /// `track_files` row has been flagged as missing by the
    /// background scanner. Drives the LibraryView footer:
    /// `247 tracks missing. Click to relocate.` Refreshed by
    /// the scanner after each batch and after a Relocate run.
    @Published var missingTrackCount: UInt64 = 0

    /// M11c.1 — analysis-completion generation counter. Bumped
    /// every time `ensureTrackAnalyzed` or `analyzeTracks` finishes
    /// a *successful* per-track analysis, regardless of whether
    /// the analyzer actually placed a grid or a key (silence and
    /// non-musical input still flip `analysis_cache.analyzed_at`
    /// and so still need a LibraryView refresh to un-dim the row).
    /// Failed analyses **do not** bump the counter — `analyze_track`
    /// writes nothing to the library on failure, so a failure-
    /// triggered refresh would just repaint identical rows.
    /// LibraryView observes this via `.onChange` and re-runs
    /// `refreshTracks()` so the BPM badge / dim-state on the
    /// affected rows lands without a per-row push channel. A single
    /// counter is enough because the work happens on a background
    /// actor; the LibraryView's debounced refresh path collapses
    /// bursts.
    @Published var analysisGeneration: UInt64 = 0

    /// M11c.1 — immediate row patch for the LibraryView BPM / key /
    /// dim-state columns. Emitted alongside `analysisGeneration`
    /// so the browser can update the affected row before the async
    /// listing refetch completes.
    @Published var libraryRowAnalysisUpdate: LibraryRowAnalysisUpdate?

    /// M11c.1 — count of analyses currently in flight, batch or
    /// not. Drives the spinner-vs-quiescent decision on the
    /// LibraryView footer ("any work happening at all?"). NOT
    /// the right value for "N of M" progress — analyses inside
    /// `analyzeTracks` run serially, so this counter is at most 1
    /// for the duration of a batch even when 200 tracks are
    /// queued. Use `analysisBatchCompleted` for the visible "N of
    /// M" line.
    @Published var analysisInFlightCount: UInt32 = 0

    /// M11c.1 — number of tracks already processed in the current
    /// batch (post-fix for the "Analyzing 5 of 5…" bug where the
    /// view tried to derive `done` from `analysisInFlightCount`).
    /// Incremented after each track in `analyzeTracks` finishes —
    /// success or failure both count, because the user-visible
    /// thing is "how much of the batch is left". Reset to 0 when
    /// the batch starts; the deferred cleanup also zeroes it when
    /// the batch ends.
    @Published var analysisBatchCompleted: UInt32 = 0

    /// M11c.1 — total tracks queued for the current batch-analyze
    /// run. The view renders `"Analyzing \(analysisBatchCompleted
    /// + 1) of \(analysisBatchTotal)…"` while the batch is live.
    /// `0` when no batch is active (single-deck-load analyses
    /// fire through `ensureTrackAnalyzed` and don't show a batch
    /// progress line).
    @Published var analysisBatchTotal: UInt32 = 0

    /// M11d.4 — `true` while a Relocate run is in progress.
    /// Drives the Relocate sheet's progress indicator and
    /// disables the "Match Folder…" button.
    @Published var relocateInProgress: Bool = false

    /// M11d.4 — number of missing tracks that found a match on
    /// the last Relocate run. Surfaced in the Relocate sheet
    /// post-run.
    @Published var lastRelocateMatches: UInt32 = 0

    /// M11d.4 — number of missing tracks left unmatched after
    /// the last Relocate run (i.e. the user must point Dub at a
    /// different folder, or the file is truly gone).
    @Published var lastRelocateUnmatched: UInt32 = 0

    /// M11d-next — user-created Dub crates (PRD §8.5.1), flat list
    /// ordered case-insensitively by name (the FFI sorts). Drives
    /// the sidebar's "Dub Crates" section. Refreshed on library
    /// open and after every crate mutation (create / rename /
    /// delete / add / remove / reorder) by `reloadCrates()`.
    @Published var crates: [LibraryCrate] = []

    /// M11d-next — bumped whenever a crate's *membership or order*
    /// changes (add / remove / reorder). LibraryView observes this
    /// via `.onChange` and re-runs `refreshTracks()` so a crate
    /// that's currently selected reflects the edit immediately,
    /// without a per-crate push channel. The crate *list* itself
    /// (names / counts) is observed directly through `crates`.
    @Published var crateContentGeneration: UInt64 = 0

    /// Imported-source nodes (Serato / Traktor / iTunes) for the
    /// sidebar's "Imported Sources" section, one per source that has
    /// data (or, once Preferences toggles land, is enabled). Each
    /// carries its track count + flat crate tree. Refreshed on library
    /// open and after every source import by
    /// `WaveformAppModel.reloadImportedSources()`.
    @Published var importedSources: [ImportedSourceGroup] = []
}
