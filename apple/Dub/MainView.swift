//
//  MainView.swift
//  Dub
//
//  Top-level SwiftUI view + the app-wide `WaveformAppModel` view
//  model that owns the shared `DubEngine` handle.
//
//  M10.5b refactor: the model is no longer just an engine
//  start/stop wrapper. It owns per-deck state (track info, position,
//  is-playing, load-error flash), drives a 30 Hz polling timer that
//  reads `engine.position(deck)` to keep the deck headers in sync,
//  derives the master deck per PRD §6.4, exposes the FS-browser
//  selection that `Space` loads, and routes drag-and-drop URLs to
//  the engine via `load_track + play`.
//
//  See `PerformanceView.swift` for the actual layout per PRD §9.2
//  and `PreferencesSheet.swift` for the engine-lifecycle controls.
//

import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers

import DubCore

/// Mode the engine is currently running in. Drives whether the
/// canonical two-deck performance surface (`.timecode`) or the
/// single-deck track-prep shell (`.prep`) is shown, and which
/// `DubEngine` lifecycle entry point gets called on Start.
///
/// PRD §3.1: auto-detect picks the default at launch; user can
/// override in Preferences.
enum EngineMode: String, CaseIterable, Identifiable {
    case timecode = "timecode"
    case prep = "prep"

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .timecode: return "Performance (Timecode)"
        case .prep:     return "Track Preparation"
        }
    }
}

/// Per-deck UI state. Driven by the model's 30 Hz polling loop +
/// load-track / play / pause calls. The performance view is a
/// pure function of one of these per deck.
///
/// All time values are wall-clock seconds. `nil`-able fields are
/// `nil` when the deck has no track loaded; the deck header
/// renders em-dashes in that case.
struct DeckState: Equatable {
    /// True once `load_track` has succeeded on this deck. Cleared
    /// when the engine stops or a load fails.
    var hasTrack: Bool = false

    /// True when the engine is advancing the playhead. Driven by
    /// the 30 Hz poll, not the UI's local pause/play state — keeps
    /// the chrome honest with the engine.
    var isPlaying: Bool = false

    /// True after the playhead reaches the end of the track. The
    /// engine stays at end (auto-stop, not auto-rewind) per PRD
    /// §6.1.3.
    var atEnd: Bool = false

    /// Filename stem of the loaded track. Used as the deck-header
    /// title fallback when the container has no ID3 / Vorbis /
    /// MP4 title tag.
    var displayName: String? = nil

    /// Track title parsed from the container's tag block (M10.5r).
    /// `nil` when the file is untagged — the deck header falls
    /// back to `displayName`.
    var trackTitle: String? = nil

    /// Track artist parsed from the container's tag block (M10.5r).
    /// `nil` when the file is untagged.
    var trackArtist: String? = nil

    /// Format / SR chip ("MP3 · 44.1 kHz · 2 ch"). `nil` until a
    /// track loads.
    var formatChip: String? = nil

    /// Total track duration. 0 if no track loaded.
    var durationSecs: Double = 0

    /// M10.5u. Estimated tempo for the loaded track, populated
    /// from `engine.beatGrid(deckIdx:)` after a successful
    /// `loadTrack`. `nil` when no track is loaded, when BPM
    /// analysis failed (silence / non-musical / too-short input),
    /// or when the estimator's confidence collapsed to zero. The
    /// deck-header BPM column renders `—` in that case.
    var bpm: Double? = nil

    /// M10.5u. Confidence the BPM estimator returned alongside
    /// `bpm`, in `[0, 1]`. `0` when no usable estimate exists.
    /// Currently informational only — the header doesn't gate
    /// the digits on this — but it's plumbed through so a future
    /// "BPM ?" low-confidence affordance can read it without
    /// another FFI poll.
    var bpmConfidence: Double = 0

    /// When set in the future, the deck pane renders a red overlay
    /// with a "deck is playing — lift the needle" message until
    /// this timestamp elapses. Used to surface a load failure
    /// caused by attempting to load into a playing deck (PRD §5.5
    /// + §6.4).
    var errorFlashUntil: Date? = nil

    /// Cached source URL for the loaded file. Used by drag-drop
    /// targeting + the FS browser to highlight which file is
    /// loaded on each deck.
    var sourceURL: URL? = nil

    /// M11d.3 — canonical UUID of the library track currently
    /// loaded on this deck, when the source was a library row
    /// (i.e. `selectedLibraryTrackId` matched the load URL).
    /// `nil` for Finder-drag loads that bypass the library. The
    /// LibraryView reads this to render the loaded-now A / B
    /// glyph in the leftmost gutter column per PRD §8.5.3,
    /// preventing the "I just loaded the track that's already
    /// playing" mistake and visually confirming Instant Doubles
    /// (§7.3).
    var loadedLibraryTrackId: String? = nil

    /// M11c.2 — canonical Camelot key of the loaded library track
    /// (`"8B"` for C major, `"5A"` for C minor, etc.). Stamped from
    /// the `selectedLibraryTrack` snapshot at load time alongside
    /// `loadedLibraryTrackId`. `nil` for Finder drags (no library
    /// row → no `track_keys` association), for tracks whose key
    /// analysis returned zero confidence, and for tracks that
    /// haven't been analyzed yet. The deck header's KEY column
    /// renders an em-dash in that case. Surfaced through
    /// `DeckHeaderState.key` so the deck header doesn't have to
    /// reach into the library on every render.
    var key: String? = nil

    /// M10.5d. `true` while a `load_track` FFI call is in flight on
    /// this deck (decode + offline-peaks-compute happens on a
    /// background `Task.detached` so the SwiftUI main actor stays
    /// responsive). Drives the deck-header source pill to its
    /// `.loading` variant ("LOADING…", amber dot) and gates
    /// concurrent loads (drag-drop + Space on a loading deck flash
    /// the load-error overlay). Cleared by the model on completion
    /// or error of the dispatched task. Independent from
    /// `hasTrack`: a deck mid-replace-load keeps `hasTrack = true`
    /// (the previous track still plays / renders) while `isLoading
    /// = true`; a cold first load has `hasTrack = false` *and*
    /// `isLoading = true`.
    var isLoading: Bool = false

    /// M10.6c. `true` while the engine is in Panic-Play (PRD
    /// §6.1.2): the deck is decoupled from its timecode input and
    /// running at a held last-known-velocity rate. Driven by the
    /// 30 Hz position poll (`PositionInfo.isPanicPlay`), set / cleared
    /// by `WaveformAppModel.{panic, cancelPanic}(side:)` for an
    /// optimistic round-trip, and auto-cleared by the engine when a
    /// clean LFSR re-lock is detected (PRD §6.1.2 auto-resume).
    /// The deck-header source pill flips to `TC · HOLD` and the
    /// Panic glyph fills while this is `true`; in two-deck Timecode
    /// mode the overview click-jump (PRD §6.1) is allowed only when
    /// this is `true`.
    var isPanicPlay: Bool = false

    static let empty = DeckState()

    /// `true` when the deck has a track but isn't currently
    /// playing — a valid target for `Space` load (PRD §6.4 + §5.5).
    var isStopped: Bool { !isPlaying }
}

/// View-model owning the shared `DubEngine` for the lifetime of the
/// app window. All mutations happen on the main actor (`@MainActor`).
@MainActor
final class WaveformAppModel: ObservableObject {

    // MARK: Engine handle

    let engine: DubEngine

    // MARK: Lifecycle config (driven by Preferences)

    @Published var availableDevices: [String] = []
    @Published var selectedDevice: String? = nil
    @Published var channelsAText: String = "1,2"
    /// Empty = single-deck mode (only in `.timecode`); always
    /// ignored in `.prep` (deck B stays off).
    @Published var channelsBText: String = ""
    @Published var palette: WaveformPalette = .serato

    /// Engine mode the next Start call will use. Auto-default
    /// computed at launch; user can override in Preferences.
    @Published var engineMode: EngineMode = .timecode

    /// Allow loading a track onto a *playing* deck while in
    /// Performance / Timecode mode. The PRD's default policy (§5.5,
    /// §6.4) is "no — the DJ must lift the needle / pause first",
    /// surfaced as a 200 ms red flash on the rejected pane. Some
    /// users want the rule relaxed (e.g. they're rehearsing
    /// transitions and want to drop a new file mid-play without
    /// pausing first). This toggle lets them opt out of the safety
    /// rule. **Prep mode always allows it** regardless of this
    /// setting — Prep is a single-deck rehearsal shell where the
    /// "deck is playing in front of a crowd" concern doesn't apply.
    ///
    /// Persisted in `UserDefaults` under
    /// `dub.allowLoadIntoRunningDeckInPerformance`. The setting
    /// applies on the next load attempt; in-flight loads are not
    /// retroactively affected.
    @Published var allowLoadIntoRunningDeckInPerformance: Bool {
        didSet {
            UserDefaults.standard.set(
                allowLoadIntoRunningDeckInPerformance,
                forKey: Self.kAllowLoadIntoRunningDeck)
        }
    }

    private static let kAllowLoadIntoRunningDeck = "dub.allowLoadIntoRunningDeckInPerformance"

    // MARK: Live engine state

    @Published private(set) var isRunning: Bool = false
    /// Most recent transient error to surface to the user. Mutated
    /// only via `surfaceError(_:)` so the auto-clear timer stays
    /// consistent. Status-strip + Preferences both read this.
    @Published private(set) var lastError: String? = nil
    /// True iff the most recent Start opened the engine in
    /// two-deck mode (Timecode + non-empty deck-B channels).
    @Published private(set) var twoDeckMode: Bool = false

    // MARK: Per-deck state (M10.5b)

    @Published private(set) var deckA: DeckState = .empty
    @Published private(set) var deckB: DeckState = .empty

    /// Master deck per PRD §6.4 (sticky single-master). `nil` only
    /// while the engine is stopped.
    @Published private(set) var masterDeck: DeckSide? = nil

    // MARK: FS-browser selection (M10.5b)

    /// File the user has highlighted in the FS browser. `Space`
    /// loads this into the non-master, stopped deck (PRD §5.5).
    @Published var browserSelection: URL? = nil

    // MARK: Library (M11d)

    /// Shared library handle backing the M11d browser. Construction
    /// is cheap (no SQLite connection until `openLibrary()` lands).
    /// The handle outlives any one browser view, so search results
    /// and import progress survive transient view churn (sidebar
    /// switches, window resize, etc.).
    let library: DubLibrary = DubLibrary()

    /// `true` once `library.openDefault()` has succeeded. Drives
    /// the browser's "Open library" affordance — until this flips,
    /// the LibraryView shows a one-shot "preparing library…"
    /// placeholder rather than a blank list (which a DJ would read
    /// as "Dub forgot everything").
    @Published private(set) var libraryIsOpen: Bool = false

    /// Total canonical-track count, refreshed after every import.
    /// Browser footer reads this directly.
    @Published private(set) var libraryTrackCount: UInt64 = 0

    /// Unix-seconds boundary for the "Just Imported" smart crate
    /// per PRD §8.5.2. Captured at app launch so a DJ who plugs in
    /// a USB stick 10 minutes before the gig sees exactly the
    /// tracks they imported during this session.
    let appLaunchUnixSeconds: Int64 = Int64(Date().timeIntervalSince1970)

    /// Most recent import outcome, surfaced in the LibraryView
    /// footer for ~5 s after an import-folder run completes.
    /// `nil` while no import has run this session.
    @Published var lastImportSummary: LibraryImportSummary? = nil

    /// `true` while an import is in progress. Drives the
    /// browser's progress indicator and disables the
    /// "Import Folder…" button to prevent overlapping runs (the
    /// importer is safe to run twice but the UX is confusing).
    @Published private(set) var libraryImportInProgress: Bool = false

    /// Canonical UUID of the LibraryView's currently selected
    /// row, or `nil` when the current selection is a Finder drag.
    /// Used by [`recordLibraryLoadIfApplicable`] to decide whether
    /// a successful `loadTrack` deserves a `play_history` row.
    /// Kept in lockstep with [`browserSelection`] inside
    /// [`selectLibraryTrack`].
    @Published var selectedLibraryTrackId: String? = nil

    /// M11c.2 — full row snapshot for the currently-selected
    /// library track. LibraryView writes this alongside
    /// `selectedLibraryTrackId` so the load path can stamp the
    /// track's key (and any future per-track-attribute) onto
    /// `DeckState` without an extra FFI round-trip. `nil` when
    /// no library row is selected (Finder-drag selection clears
    /// it). The snapshot is intentionally untracked vs. live
    /// library mutations: if the user analyzes the track *after*
    /// selecting but *before* loading it, the cached key may lag
    /// by one analysis cycle. The 30 Hz position poll's grid
    /// refresh covers BPM staleness for the same window; key
    /// staleness is a known minor cost of avoiding a per-load
    /// FFI lookup.
    @Published var selectedLibraryTrack: LibraryTrack? = nil

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
    @Published private(set) var volumeReachability: [String: Bool] = [:]

    /// M11d.4 — count of canonical tracks whose every
    /// `track_files` row has been flagged as missing by the
    /// background scanner. Drives the LibraryView footer:
    /// `247 tracks missing. Click to relocate.` Refreshed by
    /// the scanner after each batch and after a Relocate run.
    @Published private(set) var missingTrackCount: UInt64 = 0

    /// M11c.1 — analysis-completion generation counter. Bumped
    /// every time `ensureTrackAnalyzed` or `analyzeTracks` finishes
    /// a run that wrote at least one new grid. LibraryView
    /// observes this via `.onChange` and re-runs `refreshTracks()`
    /// so the BPM badge / dim-state on the affected rows lands
    /// without a per-row push channel. A single counter is enough
    /// because the work happens on a background actor; the
    /// LibraryView's debounced refresh path collapses bursts.
    @Published private(set) var analysisGeneration: UInt64 = 0

    /// M11c.1 — count of analyses currently in flight, batch or
    /// not. Drives the spinner-vs-quiescent decision on the
    /// LibraryView footer ("any work happening at all?"). NOT
    /// the right value for "N of M" progress — analyses inside
    /// `analyzeTracks` run serially, so this counter is at most 1
    /// for the duration of a batch even when 200 tracks are
    /// queued. Use `analysisBatchCompleted` for the visible "N of
    /// M" line.
    @Published private(set) var analysisInFlightCount: UInt32 = 0

    /// M11c.1 — number of tracks already processed in the current
    /// batch (post-fix for the "Analyzing 5 of 5…" bug where the
    /// view tried to derive `done` from `analysisInFlightCount`).
    /// Incremented after each track in `analyzeTracks` finishes —
    /// success or failure both count, because the user-visible
    /// thing is "how much of the batch is left". Reset to 0 when
    /// the batch starts; the deferred cleanup also zeroes it when
    /// the batch ends.
    @Published private(set) var analysisBatchCompleted: UInt32 = 0

    /// M11c.1 — total tracks queued for the current batch-analyze
    /// run. The view renders `"Analyzing \(analysisBatchCompleted
    /// + 1) of \(analysisBatchTotal)…"` while the batch is live.
    /// `0` when no batch is active (single-deck-load analyses
    /// fire through `ensureTrackAnalyzed` and don't show a batch
    /// progress line).
    @Published private(set) var analysisBatchTotal: UInt32 = 0

    /// M11c.1 — set of track UUIDs currently in flight. Guards
    /// against double-analyzing the same track when the user
    /// rapid-fires Space + Right-click → Analyze, and is consulted
    /// before queueing each batch-analyze entry.
    private var analyzingTrackIds: Set<String> = []

    /// M11d.4 — `true` while a Relocate run is in progress.
    /// Drives the Relocate sheet's progress indicator and
    /// disables the "Match Folder…" button.
    @Published private(set) var relocateInProgress: Bool = false

    /// M11d.4 — number of missing tracks that found a match on
    /// the last Relocate run. Surfaced in the Relocate sheet
    /// post-run.
    @Published var lastRelocateMatches: UInt32 = 0

    /// M11d.4 — number of missing tracks left unmatched after
    /// the last Relocate run (i.e. the user must point Dub at a
    /// different folder, or the file is truly gone).
    @Published var lastRelocateUnmatched: UInt32 = 0

    // MARK: Private state

    /// Sticky master from the previous round when neither deck is
    /// currently playing. Starts at `.a` so the cold-launch UI has
    /// a definite anchor.
    private var stickyMaster: DeckSide = .a
    private var lastPlayStart: [DeckSide: Date] = [:]

    /// Polling timer for `engine.position(deck)`. ~30 Hz keeps the
    /// track-time row smooth without hammering the FFI; the
    /// audio-thread playhead is sampled by the timer-published
    /// snapshot inside `RunningState`. Disabled when the engine
    /// isn't running.
    private var pollTimer: Timer?
    private static let pollIntervalSecs: TimeInterval = 1.0 / 30.0

    /// Throttles the lazy `engine.beatGrid` poll in `readDeckState`
    /// while BPM is still pending. The Metal renderer already
    /// refreshes the grid every draw frame until latched; hitting
    /// the FFI on every 30 Hz deck poll as well was redundant main-
    /// thread work that occasionally stacked with a Metal draw and
    /// delayed transport clicks.
    private var bpmPollTick: [DeckSide: UInt] = [.a: 0, .b: 0]

    /// Pending auto-clear task for `lastError`. Cancelled if a new
    /// error supersedes the previous one within the visibility
    /// window.
    private var lastErrorClearTask: Task<Void, Never>?
    private static let errorVisibilitySecs: UInt64 = 5_000_000_000

    /// M11d.4 — long-lived background missing-files scanner.
    /// Started lazily after the first library open, cancelled
    /// on app shutdown. Runs at `.background` priority with a
    /// 30 s tick inside a batch and a 5 min nap when there's
    /// nothing to do, per PRD §8.5.5 "rate-limited so it does
    /// not trash SSD lifetime".
    private var libraryScannerTask: Task<Void, Never>?

    // MARK: Init / deinit

    init() {
        self.engine = DubEngine()
        self.allowLoadIntoRunningDeckInPerformance =
            UserDefaults.standard.bool(forKey: Self.kAllowLoadIntoRunningDeck)
        applyAutoDetect()
        // Only enumerate input devices when we actually need them
        // (Timecode mode). Prep mode never touches the input HAL,
        // which is the whole point of the auto-detect — the user
        // never sees a microphone-permission prompt on a Mac with
        // no external interface plugged in.
        if engineMode == .timecode {
            refreshDevices()
        }
    }

    deinit {
        libraryScannerTask?.cancel()
        engine.stopEngine()
    }

    // MARK: Device list + auto-detect

    func refreshDevices() {
        availableDevices = engine.listInputDevices()
        if selectedDevice == nil, let first = availableDevices.first {
            selectedDevice = first
        }
    }

    /// Pick a default `engineMode` based on what's plugged in.
    ///
    /// **Permission-safe.** Uses [`DubEngine.hasExternalAudioInterface`]
    /// which queries CoreAudio transport-type metadata only — no
    /// AudioUnit instantiation, no device-name reads on input-
    /// capable devices, nothing that would tickle macOS's
    /// microphone-permission TCC layer. PRD §3.1: external
    /// interface present → Performance / Timecode; none present →
    /// Track Preparation / output-only (no input touched at all).
    ///
    /// "External" here is defined by transport type — USB,
    /// Thunderbolt, FireWire, PCI, AVB — i.e. the bus types DVS
    /// interfaces actually use. The previous heuristic (string-
    /// match device names against built-in-mic patterns) called
    /// `listInputDevices` which itself triggered the TCC prompt on
    /// macOS 14+; that was the regression the user reported in
    /// M10.5b shakedown.
    private func applyAutoDetect() {
        engineMode = engine.hasExternalAudioInterface() ? .timecode : .prep
    }

    // MARK: Engine lifecycle

    /// Apply the current Preferences config to the engine — start
    /// it if stopped, restart it if running. This is the single
    /// engine-lifecycle entry point used everywhere in M10.5b:
    /// `MainView.onAppear` calls it for the cold-boot auto-start,
    /// and every Preferences `onChange` (mode / device / channels)
    /// calls it so the new config takes effect with zero clicks.
    ///
    /// Use `stop()` for the explicit user-stop path. Don't call
    /// `start()` directly anymore — `applyConfig` is the only
    /// caller that knows whether a restart-vs-fresh-start is needed.
    func applyConfig() {
        // Just-in-time device enumeration. The auto-detect at init
        // *intentionally* skipped `refreshDevices()` when Prep mode
        // was picked, so the user never saw the mic-permission prompt
        // on a Mac with no external interface. The moment the user
        // (or some onChange handler) selects Timecode mode, we need
        // a device list — call `refreshDevices()` here so the
        // Preferences picker has something to show. This is the
        // explicit-user-action point where macOS's TCC prompt may
        // fire, and that's the right time for it.
        if engineMode == .timecode && availableDevices.isEmpty {
            refreshDevices()
        }
        let wasRunning = isRunning
        if wasRunning {
            stop()
        }
        start()
    }

    func start() {
        surfaceError(nil)
        switch engineMode {
        case .timecode: startTimecode()
        case .prep:     startPrep()
        }
        if isRunning { startPolling() }
    }

    func stop() {
        stopPolling()
        engine.stopEngine()
        isRunning = false
        twoDeckMode = false
        deckA = .empty
        deckB = .empty
        masterDeck = nil
        stickyMaster = .a
        lastPlayStart.removeAll()
    }

    private func startTimecode() {
        guard let device = selectedDevice, !device.isEmpty else {
            surfaceError("Pick an input device first.")
            return
        }
        let channelsA: [UInt32]
        switch parseChannels(channelsAText, side: "A") {
        case .success(let cs): channelsA = cs
        case .failure(let msg):
            surfaceError(msg)
            return
        }
        let trimmedB = channelsBText.trimmingCharacters(in: .whitespaces)
        do {
            if trimmedB.isEmpty {
                try engine.startThru(deviceName: device, channels: channelsA)
                twoDeckMode = false
            } else {
                let channelsB: [UInt32]
                switch parseChannels(trimmedB, side: "B") {
                case .success(let cs): channelsB = cs
                case .failure(let msg):
                    surfaceError(msg)
                    return
                }
                try engine.startThruTwoDeck(
                    deviceName: device, channelsA: channelsA, channelsB: channelsB)
                twoDeckMode = true
            }
            isRunning = true
            masterDeck = stickyMaster
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Unexpected error: \(error.localizedDescription)")
        }
    }

    private func startPrep() {
        do {
            try engine.startEngine(outputChannels: 2)
            isRunning = true
            twoDeckMode = false
            masterDeck = stickyMaster
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Unexpected error: \(error.localizedDescription)")
        }
    }

    // MARK: Polling

    private func startPolling() {
        stopPolling()
        // Use a tolerance so the timer can coalesce with other
        // main-runloop work; 30 Hz is the *target*, slightly less
        // is fine for the track-time row.
        let timer = Timer.scheduledTimer(
            withTimeInterval: Self.pollIntervalSecs, repeats: true
        ) { [weak self] _ in
            self?.pollDecks()
        }
        timer.tolerance = Self.pollIntervalSecs * 0.25
        RunLoop.main.add(timer, forMode: .common)
        pollTimer = timer
    }

    private func stopPolling() {
        pollTimer?.invalidate()
        pollTimer = nil
    }

    private func pollDecks() {
        guard isRunning else { return }
        let newA = readDeckState(side: .a, prev: deckA)
        let newB = readDeckState(side: .b, prev: deckB)
        if newA != deckA { deckA = newA }
        if newB != deckB { deckB = newB }
        recomputeMaster()
    }

    private func readDeckState(side: DeckSide, prev: DeckState) -> DeckState {
        let pos = engine.position(deckIdx: side.ffiDeckIdx)
        let nowPlaying = pos.isPlaying
        if nowPlaying, !prev.isPlaying {
            lastPlayStart[side] = Date()
        }
        var next = prev
        next.hasTrack = pos.hasTrack
        next.isPlaying = nowPlaying
        next.atEnd = pos.atEnd
        next.durationSecs = pos.durationSecs
        // M11d.5 round 5 — `elapsedSecs` / `remainingSecs` are no
        // longer carried through `DeckState`. The deck-header time
        // text (`LiveDeckTimeText`) and the Track-Overview playhead
        // each read `engine.position(deckIdx:)` directly from
        // inside their own `TimelineView`, so per-second M:SS
        // rollover no longer flows through the `@Published`
        // `model.deck{A,B}` channel and no longer invalidates
        // `PerformanceView`'s body. Round 3 left a 1 Hz residual
        // republish that the user perceived as a subtle leftward
        // jump every second; this fully eliminates that path. See
        // `LiveDeckTimeText` in `DeckHeader.swift` and the
        // `TimelineView` wrapping `TrackOverviewView`'s Canvas for
        // the new consumer-side wiring.
        next.isPanicPlay = pos.isPanicPlay
        // Clear stale error flash once it elapses; the deck pane
        // will hide the overlay automatically when it observes
        // `Date() > errorFlashUntil`.
        if let until = next.errorFlashUntil, Date() >= until {
            next.errorFlashUntil = nil
        }
        // M10.5v — `load_track` no longer blocks on
        // `analyze_beat_grid`; the engine spawns the BPM compute
        // on a detached worker thread so playback can start
        // immediately (PRD §6.4 "load never blocks playback").
        // The grid arrives asynchronously, so we poll for it
        // here.  Once we've captured a valid grid we stop polling
        // (the `next.bpm != nil` condition latches).  Tracks with
        // no detectable BPM (silence / too-short / non-musical)
        // keep polling — the cost is one FFI call returning an
        // empty `BeatGrid` per tick (~µs), well below the budget.
        if next.hasTrack, next.bpm == nil {
            let tick = (bpmPollTick[side] ?? 0) &+ 1
            bpmPollTick[side] = tick
            // ~1 Hz is enough for the deck-header BPM chip to light
            // up; the Metal beat-grid pass handles its own refresh.
            if tick % 30 == 0 {
                let grid = engine.beatGrid(deckIdx: side.ffiDeckIdx)
                if grid.confidence > 0, grid.bpm > 0 {
                    next.bpm = grid.bpm
                    next.bpmConfidence = Double(grid.confidence)
                }
            }
        } else {
            bpmPollTick[side] = 0
        }
        return next
    }

    // MARK: Master deck (PRD §6.4)

    private func recomputeMaster() {
        // Single-deck modes (Prep, single-channel Timecode) only
        // ever have deck A. Pinning master to .a keeps the MASTER
        // chip stable and stops the non-master Space-load logic
        // from ever picking the non-existent deck B.
        guard twoDeckMode else {
            if masterDeck != .a { masterDeck = .a }
            stickyMaster = .a
            return
        }
        let aPlaying = deckA.isPlaying
        let bPlaying = deckB.isPlaying
        let newMaster: DeckSide
        switch (aPlaying, bPlaying) {
        case (true, false): newMaster = .a
        case (false, true): newMaster = .b
        case (true, true):
            let aTs = lastPlayStart[.a] ?? .distantPast
            let bTs = lastPlayStart[.b] ?? .distantPast
            newMaster = (aTs >= bTs) ? .a : .b
        case (false, false): newMaster = stickyMaster
        }
        if masterDeck != newMaster {
            masterDeck = newMaster
        }
        stickyMaster = newMaster
    }

    // MARK: Track load + transport

    /// Load a track onto `side` (M10.5d background-load).
    ///
    /// **Refuses (and red-flashes the deck pane) when** the target
    /// deck is currently playing **and** the load-into-playing-deck
    /// guard is active (PRD §5.5 + §6.4 — the user must lift the
    /// needle / pause first). M10.5r relaxed the guard:
    ///
    /// * Prep mode always allows the load — Prep is a single-deck
    ///   rehearsal shell, not a stage workflow.
    /// * Performance / Timecode mode respects
    ///   `allowLoadIntoRunningDeckInPerformance` — the user opts in
    ///   from Preferences if they want to drop tracks mid-play.
    ///
    /// Also refuses when another load is already in flight on the
    /// same deck (avoids racing two decoders against each other
    /// and stomping the deck's `Arc<Track>`).
    ///
    /// **Concurrency (M10.5v).** `engine.loadTrack` is the Rust
    /// FFI, which since M10.5v returns the *instant* the audio
    /// thread receives the new `Arc<Track>` (~50 ms decode +
    /// near-zero swap). Offline peaks (~10–30 ms) and
    /// `analyze_beat_grid` (~100–400 ms) run on a detached
    /// `std::thread` inside the engine, so playback is never
    /// gated on analysis (PRD §6.4 "load never blocks playback").
    /// We still wrap the FFI call in `Task.detached` so the
    /// decode itself doesn't block the SwiftUI main actor.
    /// Returns `true` once the engine has accepted the track and
    /// playback is possible — the waveform + BPM may still be a
    /// few frames behind.
    ///
    /// **Optimistic UI.** Title + format chip flip to the *new*
    /// file before decode starts (so the deck immediately reads
    /// "Loading… MyTrack.mp3"); duration / has-track land once the
    /// FFI call returns. The previous track's waveform is cleared
    /// at swap time (engine sets `peaks[idx] = None`); the
    /// renderer's `peaksGeneration` mismatch handler resets the
    /// view at that moment, then re-populates when the detached
    /// peaks thread installs the new data. The BPM column shows
    /// "—" until the detached BPM thread finishes, then populates
    /// via the 30 Hz position poll.
    @discardableResult
    func loadTrack(side: DeckSide, url: URL) async -> Bool {
        guard isRunning else {
            surfaceError("Engine not running. Open Preferences (⌘,) and Start.")
            return false
        }
        let target = state(for: side)
        if target.isPlaying, !canLoadIntoPlayingDeck() {
            flashLoadError(side: side)
            return false
        }
        if target.isLoading {
            flashLoadError(side: side)
            surfaceError("Deck \(side.label) is already loading a track. Wait or load onto the other deck.")
            return false
        }

        // Optimistic UI: header pill flips to LOADING + new file
        // basename appears before the decode work starts. We clear
        // the old tag-derived title / artist so the header doesn't
        // show stale metadata from the previous track during the
        // ~50 ms decode window.
        var starting = target
        starting.isLoading = true
        starting.sourceURL = url
        starting.displayName = url.deletingPathExtension().lastPathComponent
        starting.trackTitle = nil
        starting.trackArtist = nil
        starting.bpm = nil
        starting.bpmConfidence = 0
        starting.key = nil
        starting.errorFlashUntil = nil
        setState(starting, for: side)

        let deckIdx = side.ffiDeckIdx
        let engineRef = engine
        // M11d.5 round 4 — single source of truth for the deck's
        // beat grid. If the load came from a library row (selection
        // + Space, or a library drag where the user selected the
        // row first), look up the active row in `track_beatgrids`
        // and hand it to the engine in the same FFI call. The
        // engine's background worker then installs the library
        // row's `(bpm, anchor_secs)` directly and skips the
        // ~100–400 ms `dub_bpm::analyze_beat_grid` step. The
        // DeckHeader and the LibraryView now read the same number
        // by construction (closes UI-BACKLOG C-26), and the
        // renderer's `confidence > 0` latch on the beat-grid poll
        // fires on the first Metal frame after load instead of
        // polling indefinitely on tracks the engine analyser
        // legitimately rejects (closes UI-BACKLOG B-25). The
        // lookup is a single indexed SELECT, sub-millisecond on
        // any size of library; safe on the main actor.
        //
        // Returns `nil` for the Finder-drag case (no library row),
        // for the library-load-of-fresh-track case (no auto row
        // yet — `ensureTrackAnalyzed` will write one a few seconds
        // after load completes, so subsequent loads of the same
        // file take the fast path), and for the explicit
        // "library row exists but no active grid yet" case
        // (silence-track that the analyser legitimately
        // produced no grid for). All three cases fall through to
        // the engine's existing `analyze_beat_grid` path with no
        // behaviour change.
        let preloadedGrid = libraryBeatGridForPendingLoad(url: url)
        let result: Result<Void, Error> = await Task.detached(priority: .userInitiated) {
            do {
                try engineRef.loadTrack(
                    deckIdx: deckIdx,
                    path: url.path,
                    libraryBeatGrid: preloadedGrid
                )
                return .success(())
            } catch {
                return .failure(error)
            }
        }.value

        switch result {
        case .success:
            var next = state(for: side)
            next.hasTrack = true
            next.atEnd = false
            next.isPlaying = false
            next.isLoading = false
            if let info = engine.trackInfo(deckIdx: deckIdx) {
                next.durationSecs = info.durationSecs
                next.formatChip = formatChip(for: url, info: info)
                next.trackTitle = info.title.isEmpty ? nil : info.title
                next.trackArtist = info.artist.isEmpty ? nil : info.artist
            }
            // M10.5v — BPM analysis is no longer awaited inline.
            // The engine returns from `load_track` the instant the
            // audio thread can play (decode + Arc<Track> install,
            // ~50 ms total) and spawns peaks + `analyze_beat_grid`
            // on a detached worker thread.  The deck-header BPM
            // column is populated by `readDeckState` on the next
            // 30 Hz poll tick(s) once the grid lands.
            next.bpm = nil
            next.bpmConfidence = 0
            setState(next, for: side)
            recomputeMaster()
            // M11d.2: record a play_history row when the source
            // URL came from the library (i.e. the user clicked a
            // row in LibraryView, which populated
            // `selectedLibraryTrackId`). Finder drags don't write
            // history because there's no library row yet; the
            // background importer can pull them in later.
            recordLibraryLoadIfApplicable(side: side, url: url)
            return true
        case .failure(let error):
            var failed = state(for: side)
            failed.isLoading = false
            setState(failed, for: side)
            if let engineError = error as? EngineError {
                surfaceError(describe(engineError))
            } else {
                surfaceError("Unexpected load error: \(error.localizedDescription)")
            }
            return false
        }
    }

    /// Load the FS-browser selection into the appropriate target
    /// deck. PRD §5.5 — bound to `Space` in `MainView`.
    ///
    /// Target deck selection:
    /// * Two-deck (Timecode + non-empty deck-B channels) → the
    ///   non-master deck.
    /// * Single-deck (Timecode single-channel **or** Prep) → deck
    ///   A. Prep mode by definition has no deck B, and single-
    ///   channel Timecode never spins one up, so "non-master" isn't
    ///   meaningful and Space loads onto the only deck that exists.
    // MARK: - Library access (M11d)

    /// Open the canonical library at
    /// `~/Library/Application Support/Dub/library.sqlite`. Safe to
    /// call repeatedly; the FFI handle is idempotent on re-open.
    /// Called once from `MainView.onAppear`.
    func openLibraryIfNeeded() {
        guard !libraryIsOpen else { return }
        do {
            try library.openDefault()
            libraryIsOpen = true
            refreshLibraryStats()
            refreshMissingTrackCount()
            startMissingFilesScanner()
        } catch {
            surfaceError("Failed to open library: \(error.localizedDescription)")
        }
    }

    /// Refresh `libraryTrackCount`. Cheap (`SELECT COUNT(*) FROM
    /// tracks`); called on app launch and after every import.
    func refreshLibraryStats() {
        guard libraryIsOpen else { return }
        if let count = try? library.trackCount() {
            libraryTrackCount = count
        }
    }

    /// Recompute the per-volume reachability cache for the set of
    /// mount points present in the supplied track list. Each
    /// unique non-nil mount point hits the filesystem exactly
    /// once via `FileManager.fileExists(atPath:isDirectory:)`.
    /// `nil` mount points (volumes the library has on record but
    /// can't currently locate) implicitly map to unreachable
    /// without a syscall.
    ///
    /// Called by the LibraryView whenever the displayed track
    /// set changes (source switch, search, post-import refresh).
    /// Per-frame polling is intentionally avoided — an SSD
    /// staying plugged in is the common case and we don't want
    /// to syscall every scroll tick.
    func refreshVolumeReachability(for tracks: [LibraryTrack]) {
        var mountPoints = Set<String>()
        for t in tracks {
            if let m = t.primaryVolumeMountPoint, !m.isEmpty {
                mountPoints.insert(m)
            }
        }
        var next = volumeReachability
        // Drop entries for mount points no longer in view so the
        // cache stays bounded.
        next = next.filter { mountPoints.contains($0.key) }
        for m in mountPoints {
            var isDir: ObjCBool = false
            let exists = FileManager.default.fileExists(atPath: m, isDirectory: &isDir)
            next[m] = exists && isDir.boolValue
        }
        if next != volumeReachability {
            volumeReachability = next
        }
    }

    /// `true` when the supplied track resolves to a path on a
    /// currently-reachable volume per the cached reachability
    /// map. Used by the LibraryView to render the missing-file
    /// glyph. Conservative — returns `false` for any track
    /// without a primary mount point on record, or whose mount
    /// point hasn't been probed yet (the LibraryView calls
    /// `refreshVolumeReachability` *before* rendering the first
    /// frame, so "not yet probed" is a transient state and a
    /// false positive on the glyph is acceptable in that window).
    func isTrackReachable(_ track: LibraryTrack) -> Bool {
        guard let m = track.primaryVolumeMountPoint else { return false }
        return volumeReachability[m] == true
    }

    // MARK: - M11d.4 Missing-files scanner

    /// Refresh `missingTrackCount` from the library. Cheap
    /// (`COUNT(*)` over a small partial index). Called on
    /// app launch, after every import, and after each scanner
    /// batch + Relocate run.
    func refreshMissingTrackCount() {
        guard libraryIsOpen else { return }
        if let n = try? library.missingTrackCount() {
            missingTrackCount = n
        }
    }

    /// Run one scanner pass. Pulls a batch of `track_files`
    /// rows (stalest first per PRD §8.5.5), probes each path
    /// via `FileManager.fileExists`, and writes the verdict
    /// back through the FFI. Returns the number of files
    /// checked so the caller can decide whether more work
    /// remains.
    ///
    /// Implemented as `async` so the call site (the periodic
    /// scanner Task) can await without blocking the main
    /// actor. The actual filesystem + FFI work is dispatched
    /// to a detached background task; results are merged on
    /// the main actor.
    @discardableResult
    func scanMissingFilesBatch(batchSize: UInt32 = 100) async -> UInt32 {
        guard libraryIsOpen else { return 0 }
        let library = self.library
        let now = Int64(Date().timeIntervalSince1970)
        let processed: UInt32 = await Task.detached(priority: .utility) {
            let rows: [LibraryFileScanRow]
            do {
                rows = try library.listFilesForScan(batchSize: batchSize)
            } catch {
                return 0
            }
            var count: UInt32 = 0
            for r in rows {
                let isMissing: Bool
                if let mount = r.mountPoint, !mount.isEmpty {
                    let abs = (mount as NSString).appendingPathComponent(r.relativePath)
                    isMissing = !FileManager.default.fileExists(atPath: abs)
                } else {
                    isMissing = true
                }
                if isMissing != r.wasMissing {
                    do {
                        try library.markFileState(
                            fileId: r.fileId,
                            isMissing: isMissing,
                            timestampUnixSecs: now
                        )
                    } catch {
                        continue
                    }
                } else {
                    do {
                        try library.markFileState(
                            fileId: r.fileId,
                            isMissing: isMissing,
                            timestampUnixSecs: now
                        )
                    } catch {
                        continue
                    }
                }
                count += 1
            }
            return count
        }.value
        refreshMissingTrackCount()
        return processed
    }

    /// Kick off the long-lived scanner Task that walks the
    /// library on a low-priority cadence per PRD §8.5.5:
    /// ~100 files / 30 s. A scratch DJ's library tops out
    /// around 50–100 k tracks, so a full pass takes
    /// ~15–30 min, which is well below the "drive ejected,
    /// drive re-mounted" event timescale.
    func startMissingFilesScanner() {
        guard libraryScannerTask == nil else { return }
        libraryScannerTask = Task.detached(priority: .background) { [weak self] in
            // Initial fast pass while the user is staring at
            // the browser, then a slow steady-state cadence.
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            while !Task.isCancelled {
                let processed = await self?.scanMissingFilesBatch(batchSize: 100) ?? 0
                let nextDelayNs: UInt64 = processed == 0
                    ? 5 * 60 * 1_000_000_000   // nothing to do → 5 min nap
                    : 30 * 1_000_000_000        // batch processed → 30 s
                try? await Task.sleep(nanoseconds: nextDelayNs)
            }
        }
    }

    /// Stop the scanner. Called on app shutdown.
    func stopMissingFilesScanner() {
        libraryScannerTask?.cancel()
        libraryScannerTask = nil
    }

    /// Run a Relocate pass per PRD §8.5.5. Walks the supplied
    /// directory (recursively), and for each candidate audio
    /// file compares its computed fingerprint + duration +
    /// filename against the set of currently-missing tracks.
    /// On match, registers a new `track_files` row pointing at
    /// the relocated path (the original row stays on record
    /// so the touring SSD can resurrect it later).
    ///
    /// Matching rules mirror PRD §8.1's dedupe:
    ///   * Chromaprint similarity ≥ 0.98, **or**
    ///   * filename match (basename equality), **and**
    ///   * duration delta < 200 ms in both cases.
    ///
    /// Returns `(matched, unmatched)` so the Relocate sheet can
    /// surface a "matched 42 of 247 missing tracks" line.
    @discardableResult
    func runRelocate(matchingFolder folder: URL) async -> (matched: UInt32, unmatched: UInt32) {
        guard libraryIsOpen else { return (0, 0) }
        if relocateInProgress { return (0, 0) }
        relocateInProgress = true
        defer { relocateInProgress = false }

        let library = self.library
        let folderPath = folder.path
        let result: (UInt32, UInt32) = await Task.detached(priority: .userInitiated) {
            return Self.relocateImpl(library: library, folderPath: folderPath)
        }.value
        lastRelocateMatches = result.0
        lastRelocateUnmatched = result.1
        refreshMissingTrackCount()
        return result
    }

    /// Internal worker for `runRelocate`. Pure function over
    /// the FFI handle; called from a detached Task so it never
    /// blocks the main actor. Each candidate file is handed to
    /// `try_relocate_candidate` which does the heavy lifting
    /// (decode + fingerprint + similarity + duration check +
    /// relocate commit) on the Rust side where the dedupe
    /// primitives already live.
    ///
    /// Limit `5 000` matches the browser's "All Tracks" cap;
    /// libraries bigger than that need multiple Relocate
    /// passes, which is acceptable workflow-wise.
    private nonisolated static func relocateImpl(library: DubLibrary, folderPath: String) -> (UInt32, UInt32) {
        // Snapshot the pre-pass count so we can compute
        // `matched = before - after` without trusting per-call
        // success returns (a hung FK insert still bumps
        // `matched` correctly because the count drops only on a
        // successful relocate_track).
        let totalBefore: UInt64
        do {
            totalBefore = try library.missingTrackCount()
        } catch {
            return (0, 0)
        }
        if totalBefore == 0 { return (0, 0) }

        let audioExts: Set<String> = [
            "wav", "flac", "aif", "aiff", "mp3", "m4a", "aac", "ogg", "opus",
        ]
        let enumerator = FileManager.default.enumerator(
            at: URL(fileURLWithPath: folderPath),
            includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles]
        )
        guard let walker = enumerator else {
            return (0, UInt32(min(totalBefore, UInt64(UInt32.max))))
        }
        for case let url as URL in walker {
            if !audioExts.contains(url.pathExtension.lowercased()) { continue }
            _ = try? library.tryRelocateCandidate(
                absolutePath: url.path,
                limitMissing: 5_000
            )
        }
        let totalAfter = (try? library.missingTrackCount()) ?? totalBefore
        let matched = totalBefore > totalAfter ? totalBefore - totalAfter : 0
        return (
            UInt32(min(matched, UInt64(UInt32.max))),
            UInt32(min(totalAfter, UInt64(UInt32.max)))
        )
    }

    /// Walk the supplied folder via the M11c importer. Runs on a
    /// detached background queue so the UI stays responsive; the
    /// completion handler hops back to the main actor to update
    /// `libraryTrackCount` and `lastImportSummary`. Idempotent —
    /// re-importing the same folder refreshes metadata without
    /// duplicating identity rows (proven by
    /// `re_import_is_idempotent` in `dub-library`).
    func importLibraryFolder(_ folder: URL) async {
        guard libraryIsOpen else {
            surfaceError("Library is not open yet.")
            return
        }
        if libraryImportInProgress {
            surfaceError("An import is already running.")
            return
        }
        libraryImportInProgress = true
        let library = self.library
        let path = folder.path
        let result: Result<LibraryImportSummary, Error> = await Task.detached(priority: .userInitiated) {
            do {
                let s = try library.importFolder(path: path)
                return .success(s)
            } catch {
                return .failure(error)
            }
        }.value
        libraryImportInProgress = false
        switch result {
        case .success(let summary):
            lastImportSummary = summary
            refreshLibraryStats()
            refreshMissingTrackCount()
        case .failure(let err):
            surfaceError("Import failed: \(err.localizedDescription)")
        }
    }

    /// Resolve a canonical track id to its on-disk URL and store it
    /// in `browserSelection` so the existing Space-load + drag
    /// paths (PRD §6.4) Just Work. Surfaces a polite error when
    /// the file is currently unreachable (volume unmounted, track
    /// deleted) instead of writing a bogus URL.
    func selectLibraryTrack(_ trackId: String) {
        selectLibraryTrack(trackId, snapshot: nil)
    }

    /// Selection variant that also takes the full `LibraryTrack`
    /// row snapshot, so the load path can stamp the track's key
    /// (and future per-track attributes) onto `DeckState` without
    /// an extra FFI round-trip. LibraryView calls this when the
    /// user clicks a row, passing the row it already has in
    /// memory. The id-only overload above is kept for callers
    /// that haven't yet been threaded with row snapshots.
    func selectLibraryTrack(_ trackId: String, snapshot: LibraryTrack?) {
        guard libraryIsOpen else { return }
        do {
            if let path = try library.trackPath(trackId: trackId) {
                browserSelection = URL(fileURLWithPath: path)
                selectedLibraryTrackId = trackId
                selectedLibraryTrack = snapshot
            } else {
                browserSelection = nil
                selectedLibraryTrackId = nil
                selectedLibraryTrack = nil
                surfaceError("Track is unreachable — the source volume may be unmounted.")
            }
        } catch {
            surfaceError("Failed to resolve track: \(error.localizedDescription)")
        }
    }

    /// Write a `play_history` row when the just-loaded URL came
    /// from the M11d library (i.e. the user clicked a row in
    /// `LibraryView` rather than dragging a file from Finder).
    /// The matching is done by URL equality against the
    /// previously-cached `selectedLibraryTrackId` — robust to
    /// the Apple shell's path-normalisation foibles because
    /// both sides went through the same
    /// `library.trackPath(trackId:)` → URL conversion.
    ///
    /// The deck index is mapped from `DeckSide` to the
    /// `(0 = A, 1 = B)` convention `play_history.deck`
    /// enforces. `timestamp_ms` is unix-millis from the Swift
    /// wall clock.
    ///
    /// Failures are surfaced silently to `lastError` instead of
    /// flashing the deck pane — a missed history row is a
    /// cosmetic glitch on the smart-crate, not a load failure
    /// the DJ needs to see.
    /// M11d.5 round 4 — fetch the active row from
    /// `track_beatgrids` for the track that's about to be loaded,
    /// if any. Called from `loadTrack` to feed the new
    /// `DubEngine.loadTrack(deckIdx:path:libraryBeatGrid:)` FFI so
    /// the engine adopts the library's stored `(bpm, anchor_secs)`
    /// instead of running `dub_bpm::analyze_beat_grid` from
    /// scratch.
    ///
    /// Returns `nil` in three legitimate cases, all of which fall
    /// back to the engine's own analyser without any visible UX
    /// change:
    ///
    /// * **Library not open.** The Apple shell can run without an
    ///   open library (early-launch state before the user has
    ///   picked one); the engine path is independent.
    /// * **No matching selection.** The current
    ///   `selectedLibraryTrackId` doesn't resolve to `url`, which
    ///   means the load came via Finder drag or a stale selection.
    ///   The engine analyses the file and `ensureTrackAnalyzed`
    ///   then writes the result back to `track_beatgrids` so the
    ///   *next* load of the same file gets the fast path.
    /// * **Track in library but unanalyzed (or silent).** The row
    ///   exists in `tracks` but `track_beatgrids` has no
    ///   `is_active = 1` row yet (or the analyser legitimately
    ///   found no grid — silence, non-musical input). The engine
    ///   runs analysis as before; same write-back as the previous
    ///   case once `ensureTrackAnalyzed` lands.
    ///
    /// The lookup is a single SELECT against the partial unique
    /// index on `track_beatgrids(track_id) WHERE is_active = 1`,
    /// well under a millisecond on any size of library, so it's
    /// safe to call on the main actor immediately before the
    /// detached `Task` that runs `loadTrack`.
    private func libraryBeatGridForPendingLoad(url: URL) -> LibraryBeatGrid? {
        guard libraryIsOpen, let trackId = selectedLibraryTrackId else {
            return nil
        }
        do {
            guard let resolved = try library.trackPath(trackId: trackId) else {
                return nil
            }
            let resolvedUrl = URL(fileURLWithPath: resolved).standardizedFileURL
            guard resolvedUrl == url.standardizedFileURL else {
                return nil
            }
            return try library.activeBeatGrid(trackId: trackId)
        } catch {
            // A failed read here is non-fatal: the engine will
            // analyse the file itself, and any database problem
            // worth surfacing (lock contention, schema mismatch)
            // will resurface from a hundred other call sites. We
            // log so a dogfooding session can spot a regression,
            // but we do not block the load.
            print("dub: libraryBeatGridForPendingLoad failed for \(trackId): \(error)")
            return nil
        }
    }

    private func recordLibraryLoadIfApplicable(side: DeckSide, url: URL) {
        // Always clear the previous loaded-now glyph for this
        // deck; we re-set it below if the load came from the
        // library. Finder drags leave the field nil, which the
        // browser reads as "no library track loaded here".
        var next = state(for: side)
        next.loadedLibraryTrackId = nil
        setState(next, for: side)

        guard libraryIsOpen, let trackId = selectedLibraryTrackId else { return }
        do {
            if let path = (try library.trackPath(trackId: trackId)) {
                let cached = URL(fileURLWithPath: path).standardizedFileURL
                guard cached == url.standardizedFileURL else { return }
            } else {
                return
            }
            // Stamp the loaded-now glyph + write the play_history
            // row. Same URL-equality guard is the source of truth
            // for both: if the user changed selection between
            // selection and load, neither side runs. The key (if
            // any) comes from the LibraryTrack snapshot that
            // LibraryView captured alongside the id — see
            // `selectLibraryTrack(_:snapshot:)`.
            var stamped = state(for: side)
            stamped.loadedLibraryTrackId = trackId
            if let snap = selectedLibraryTrack, snap.id == trackId {
                stamped.key = snap.key
            }
            setState(stamped, for: side)

            let deck: UInt32 = (side == .a) ? 0 : 1
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            try library.recordLoad(trackId: trackId, deck: deck, timestampMs: nowMs)
            // M11c.1 — fire-and-forget lazy analysis for the
            // freshly-loaded track. Runs on a detached background
            // task so the deck keeps playing while `dub-bpm`
            // chews through the file (~1–3 s for a 3 minute MP3).
            // Idempotent: the inner guard against
            // `analyzingTrackIds` collapses races, and the
            // `is_track_analyzed` check skips no-ops without
            // touching the heavy decode path.
            ensureTrackAnalyzed(trackId: trackId)
        } catch {
            surfaceError("Failed to record play history: \(error.localizedDescription)")
        }
    }

    /// M11c.1 — kick off lazy analysis for `trackId` if it has
    /// not been analyzed yet. Returns immediately; analysis runs
    /// on a background task and bumps `analysisGeneration` when
    /// the row finishes so the LibraryView refreshes its BPM /
    /// dim cue. Safe to call repeatedly: the in-flight set
    /// guarantees one analysis run per track at a time, and the
    /// `is_track_analyzed` predicate skips fast-path once the
    /// track has been processed once.
    func ensureTrackAnalyzed(trackId: String) {
        guard libraryIsOpen else { return }
        guard !analyzingTrackIds.contains(trackId) else { return }
        // Cheap synchronous check on the calling actor — if the
        // track is already analyzed, skip the background task
        // entirely. Avoids spawning a Task for every Space-load
        // on a fully-analyzed library.
        if let analyzed = try? library.isTrackAnalyzed(trackId: trackId), analyzed {
            return
        }
        analyzingTrackIds.insert(trackId)
        analysisInFlightCount &+= 1
        let library = self.library
        Task.detached(priority: .background) { [weak self] in
            let result: Result<LibraryAnalysisOutcome, Error>
            do {
                let outcome = try library.analyzeTrack(trackId: trackId)
                result = .success(outcome)
            } catch {
                result = .failure(error)
            }
            await MainActor.run {
                guard let self = self else { return }
                self.analyzingTrackIds.remove(trackId)
                if self.analysisInFlightCount > 0 {
                    self.analysisInFlightCount -= 1
                }
                switch result {
                case .success:
                    self.analysisGeneration &+= 1
                case .failure(let err):
                    // Per-track failure is surfaced silently to
                    // the error toast; the next Space-load
                    // retries (the track stays unanalyzed in
                    // analysis_cache because analyze_track only
                    // stamps on success).
                    self.surfaceError(
                        "Analysis failed for track: \(err.localizedDescription)")
                }
            }
        }
    }

    /// M11c.1 — batch-analyze entry point. Drives the LibraryView
    /// right-click context menu's "Analyze Selected" /
    /// "Re-analyze Selected" actions. Iterates serially so each
    /// analysis releases the library lock before the next acquires
    /// it (other UI queries can interleave). Skips tracks already
    /// analyzed unless `forceReanalyze` is `true`. Updates the
    /// footer progress counters as it goes.
    func analyzeTracks(_ trackIds: [String], forceReanalyze: Bool) async {
        guard libraryIsOpen, !trackIds.isEmpty else { return }
        let library = self.library
        analysisBatchTotal = UInt32(trackIds.count)
        analysisBatchCompleted = 0
        defer {
            self.analysisBatchTotal = 0
            self.analysisBatchCompleted = 0
        }
        for trackId in trackIds {
            if analyzingTrackIds.contains(trackId) {
                analysisBatchCompleted &+= 1
                continue
            }
            if !forceReanalyze {
                if let analyzed = try? library.isTrackAnalyzed(trackId: trackId), analyzed {
                    analysisBatchCompleted &+= 1
                    continue
                }
            }
            analyzingTrackIds.insert(trackId)
            analysisInFlightCount &+= 1
            let result = await Task.detached(priority: .userInitiated) {
                () -> Result<LibraryAnalysisOutcome, Error> in
                do {
                    let outcome = try library.analyzeTrack(trackId: trackId)
                    return .success(outcome)
                } catch {
                    return .failure(error)
                }
            }.value
            analyzingTrackIds.remove(trackId)
            if analysisInFlightCount > 0 { analysisInFlightCount -= 1 }
            analysisBatchCompleted &+= 1
            // Bump the generation *per track* (not once at the
            // end of the batch as the pre-fix code did) so the
            // LibraryView's BPM / key badges land row by row as
            // the batch runs. Users analyzing a 200 track folder
            // need to see progress; a single end-of-batch refresh
            // makes the table look frozen.
            analysisGeneration &+= 1
            if case .failure(let err) = result {
                surfaceError("Analysis failed for track: \(err.localizedDescription)")
            }
        }
    }

    func loadBrowserSelectionIntoTargetDeck() async {
        guard isRunning else {
            surfaceError("Engine not running.")
            return
        }
        guard let url = browserSelection else {
            surfaceError("Select a file in the browser first.")
            return
        }
        // Single-click in the browser now selects folders too (so
        // the highlight follows keyboard intuition) — but Space
        // shouldn't try to load a folder as audio. Skip with a
        // polite hint instead of letting the FFI return a decode
        // error.
        var isDir: ObjCBool = false
        if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir),
           isDir.boolValue {
            surfaceError("Selected entry is a folder — double-click it to enter, or pick an audio file inside.")
            return
        }
        let candidate = spaceLoadTarget()
        let target = state(for: candidate)
        if target.isPlaying, !canLoadIntoPlayingDeck() {
            flashLoadError(side: candidate)
            return
        }
        _ = await loadTrack(side: candidate, url: url)
    }

    /// `true` when a load is allowed to land on a deck that is
    /// currently playing. See `loadTrack(side:url:)` for the policy:
    ///
    /// * **Prep mode** always allows the load — Prep is a single-
    ///   deck rehearsal shell, no audience-cue concern.
    /// * **Single-deck Performance** (no `twoDeckMode`) — pre-
    ///   M11d.5 this enforced the "lift the needle" safety which
    ///   the user-perceived as "Space-load is broken" because the
    ///   auto-play-on-drop made the only deck "playing" within ~50
    ///   ms of the previous load. With only one deck on the
    ///   surface there is no master / cue split, so the safety has
    ///   nothing to protect; treat it like Prep and always allow.
    /// * **Two-deck Performance** — the canonical PRD §5.5 + §6.4
    ///   safety still applies. The user must lift the needle /
    ///   pause first; the 200 ms red flash signals the rejection.
    ///   `allowLoadIntoRunningDeckInPerformance` is the user-level
    ///   opt-out for users who consciously want to relax even this
    ///   case (rehearsing transitions, etc.).
    private func canLoadIntoPlayingDeck() -> Bool {
        switch engineMode {
        case .prep:
            return true
        case .timecode:
            if !twoDeckMode { return true }
            return allowLoadIntoRunningDeckInPerformance
        }
    }

    /// The deck Space-load targets in the current engine config.
    /// See `loadBrowserSelectionIntoTargetDeck` for the rules.
    private func spaceLoadTarget() -> DeckSide {
        guard twoDeckMode else { return .a }
        let m = masterDeck ?? stickyMaster
        return m == .a ? .b : .a
    }

    /// Start audible playback on `side`. M11d.5: in Performance /
    /// Timecode mode this also engages Panic Play so the next
    /// `DropoutHoldRate` render block doesn't pause the deck for
    /// lack of a timecode carrier. Pre-fix, calling `play` in
    /// Performance mode appeared to do nothing because the engine
    /// dutifully started the transport then immediately paused it
    /// on the next audio render (no carrier → DropoutHoldRate
    /// pauses the deck per PRD §6.1.2). Engaging Panic here makes
    /// the play button "just work" without a timecode platter
    /// connected, which is the dominant use case during pre-alpha
    /// dogfooding. The user can later disengage Panic to hand
    /// control to a timecode driver once the disengage UI ships
    /// (see `DeckHeaderState.useTimecodeToggle` doc comment).
    func play(side: DeckSide) {
        guard isRunning else { return }
        // M11d.5: Performance mode's PanicPlay path requires a
        // loaded track (engine returns an error otherwise — the
        // platter has nothing to advance). Prep allows playing
        // out an empty deck via `engine.play`, which is fine
        // because that path is a no-op when there's no source
        // attached. Guard here so a stray click on a cold deck
        // doesn't flash a confusing error banner.
        let deck = state(for: side)
        guard deck.hasTrack else { return }
        do {
            switch engineMode {
            case .prep:
                try engine.setDeckRate(deckIdx: side.ffiDeckIdx, rate: 1.0)
                try engine.play(deckIdx: side.ffiDeckIdx)
                var s = state(for: side)
                s.isPlaying = true
                setState(s, for: side)
            case .timecode:
                try engine.panicPlay(deckIdx: side.ffiDeckIdx)
                var s = state(for: side)
                s.isPlaying = true
                s.isPanicPlay = true
                setState(s, for: side)
            }
            lastPlayStart[side] = Date()
            recomputeMaster()
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Play failed: \(error.localizedDescription)")
        }
    }

    /// Pause `side`. M11d.5: in Performance mode this also clears
    /// any engaged Panic flag so the deck doesn't surprise the
    /// user by silently re-starting when they later disengage
    /// Panic from the (forthcoming) INT/ABS button. The pair
    /// `play` + `pause` round-trips cleanly: Pause then Play
    /// leaves the deck in the same logical state Play would have
    /// produced from cold.
    func pause(side: DeckSide) {
        guard isRunning else { return }
        let idx = side.ffiDeckIdx
        // Silence first — `try_push` is non-blocking and the audio
        // thread picks this up within one buffer. Keep this ahead of
        // any `@Published` state writes so a main-thread frame busy
        // in Metal draw doesn't defer the command behind UI work.
        do {
            try engine.pause(deckIdx: idx)
        } catch let error as EngineError {
            surfaceError(describe(error))
            return
        } catch {
            surfaceError("Pause failed: \(error.localizedDescription)")
            return
        }
        if engineMode == .timecode, state(for: side).isPanicPlay {
            try? engine.cancelPanicPlay(deckIdx: idx)
        }
        var s = state(for: side)
        s.isPlaying = false
        s.isPanicPlay = false
        setState(s, for: side)
        recomputeMaster()
    }

    /// M10.6a Casual-Play "Restart" (PRD §6.1.3). Seeks the deck to
    /// 0:00 and resumes playback. No-op if the engine isn't running
    /// or the deck has no track loaded. Mirror of `play(side:)` for
    /// error handling + master recomputation.
    func restart(side: DeckSide) {
        guard isRunning else { return }
        let deck = state(for: side)
        guard deck.hasTrack else { return }
        do {
            try engine.seek(deckIdx: side.ffiDeckIdx, positionSecs: 0)
            try engine.play(deckIdx: side.ffiDeckIdx)
            var s = state(for: side)
            s.atEnd = false
            s.isPlaying = true
            setState(s, for: side)
            lastPlayStart[side] = Date()
            recomputeMaster()
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Restart failed: \(error.localizedDescription)")
        }
    }

    /// M10.6a zoomed click-scrub (PRD §6.1). Given a signed offset
    /// in seconds relative to the current playhead, clamp into the
    /// track's `[0, durationSecs]` range and seek the engine there.
    /// `WaveformView` only invokes this when the parent
    /// `PerformanceView` opts in (Prep mode in M10.6a; Timecode-mode
    /// click-scrub is intentionally disabled per the PRD).
    ///
    /// **Reads the engine directly, not the polled `DeckState`.**
    /// `DeckState` no longer carries the playhead (M11d.5 round 5
    /// removed `elapsedSecs` / `remainingSecs` to stop the
    /// per-second republish from invalidating the deck pane); the
    /// jog seek queries `engine.position(deckIdx:)` here at the
    /// moment of the gesture so it gets the freshest sub-sample-
    /// accurate playhead available.
    func scrub(side: DeckSide, relativeSecs: TimeInterval) {
        guard isRunning else { return }
        let deck = state(for: side)
        guard deck.hasTrack, deck.durationSecs > 0 else { return }
        let pos = engine.position(deckIdx: side.ffiDeckIdx)
        let target = max(0, min(pos.durationSecs, pos.elapsedSecs + relativeSecs))
        seekDeck(side: side, absoluteSecs: target)
    }

    // MARK: - Vinyl-style mouse scratch (M10.5s)
    //
    // PRD §1 / §6.1 — the zoomed-waveform drag is a *scratch*
    // gesture: audio plays only while the mouse is moving, and only
    // at the rate the mouse is moving (left = reverse, right /
    // down = forward). Mouse-still ⇒ silence, identical to a record
    // sitting under a stationary stylus. The previous M10.5r
    // implementation was a seek-and-play loop that ran the deck at
    // 1× under the cursor — that violated the "feels like a
    // turntable" expectation and is gone as of M10.5s.
    //
    // Implementation:
    //
    //   1. `scratchBegin(side:)` — capture pre-scratch transport.
    //      In Timecode mode, engage Panic Play so the timecode
    //      driver doesn't fight `setDeckRate` every block. Pin
    //      `is_playing = true` (so the audio thread renders the
    //      deck), `rate = 0` (so the playhead is frozen until the
    //      first move). Spin up a 60 Hz polling timer.
    //   2. `scratchPointerOffset(side:offsetSecs:)` — the view
    //      reports the cursor's running offset (in audio seconds)
    //      from the drag's start point. The timer reads this
    //      between ticks; nothing else.
    //   3. Timer tick — compute `rate = Δoffset / Δrealtime` since
    //      the previous tick and `setDeckRate(rate)`. When the
    //      mouse is held still both deltas collapse to ~0 and the
    //      deck reads the same sample frame block-after-block,
    //      which the platter de-click in the engine smooths to
    //      silence.
    //   4. `scratchEnd(side:)` — stop the timer, cancel Panic Play
    //      (if we engaged it), restore the pre-scratch transport
    //      (rate = 1.0, set_playing to whatever it was before).
    //
    // **Position drift.** We deliberately don't seek during the
    // drag — the engine's own playhead integration accumulates
    // `rate × block_size` per block, so position naturally tracks
    // the cursor. Seeking every tick would fire a 2 ms de-click
    // on every block (`set_position_frames` ramps amplitude to
    // zero and back), turning the scratch into a tremolo. Drift
    // is bounded by the rate-conversion accuracy of
    // `set_deck_rate`'s SR math; in practice it's <10 ms over a
    // multi-second scratch, well below the visual resolution of
    // the waveform.

    /// Per-deck scratch state. `nil` ⇒ no scratch in flight on
    /// that side. Stored as a class so the polling timer's `[weak
    /// self]` closure doesn't need to chase a per-side enum case
    /// on every tick.
    ///
    /// M10.5t rework: the rate is now derived **per-event** in
    /// `scratchPointerOffset(side:offsetSecs:)` from the elapsed
    /// real-time since the previous event, then low-pass filtered
    /// with an exponential moving average. The old 60 Hz polling
    /// timer's aliasing (sampling a high-rate event stream at a
    /// fixed cadence produced periodic rate spikes that read as
    /// audible "jumping" — confirmed in pre-M10.5t dogfood) is
    /// gone. The timer that remains is a low-rate watchdog whose
    /// only job is to ramp the rate to zero when the cursor
    /// stops moving (no `onChanged` event for > stallThresholdSecs)
    /// so a stationary mouse plays silence like a stationary
    /// platter.
    private final class ScratchState {
        let side: DeckSide
        let priorIsPlaying: Bool
        let engagedPanic: Bool
        /// Playhead position (seconds) captured at scratch begin.
        /// Used on end to snap the engine to the gesture cursor
        /// before restoring unity rate, so any rate-integration
        /// drift doesn't carry into resumed playback.
        let scratchStartElapsed: Double
        /// Most recent cursor offset (in audio seconds) reported
        /// by the gesture overlay. Reset to 0 on begin.
        var lastEventOffsetSecs: Double = 0
        /// Wall-clock time of the most recent `scratchPointerOffset`
        /// call. Used both to compute `Δt` for the per-event rate
        /// and by the watchdog to detect "cursor still".
        var lastEventAt: Date
        /// Smoothed instantaneous rate. Updated by each gesture
        /// event and ramped toward zero by the watchdog timer
        /// when no event fires for a while. Cached so the
        /// watchdog doesn't have to round-trip through the engine
        /// to check what rate is currently in flight.
        var smoothedRate: Double = 0

        init(
            side: DeckSide,
            priorIsPlaying: Bool,
            engagedPanic: Bool,
            scratchStartElapsed: Double,
            startedAt: Date
        ) {
            self.side = side
            self.priorIsPlaying = priorIsPlaying
            self.engagedPanic = engagedPanic
            self.scratchStartElapsed = scratchStartElapsed
            self.lastEventAt = startedAt
        }
    }

    /// Deck currently receiving a vinyl-style scratch gesture.
    /// Published so `WaveformView` can keep the Metal draw loop
    /// running during scratch even when the deck was paused before
    /// the drag began.
    @Published private(set) var scratchingDeck: DeckSide? = nil

    /// In-flight scratch per deck. Keyed by side so deck A and B
    /// can be scratched independently (rare in practice — the
    /// user has one mouse — but the model doesn't enforce
    /// exclusivity, the view layer does).
    private var scratchStates: [DeckSide: ScratchState] = [:]

    /// Watchdog timer that ramps the rate toward zero on each
    /// deck whose cursor has been still for longer than
    /// `scratchStallThresholdSecs`. Runs only while ≥ 1 scratch
    /// is in flight; lazily torn down by `scratchEnd`.
    private var scratchTimer: Timer?
    /// Watchdog fires at this cadence. Must be << the typical
    /// gesture event rate so we don't fight the per-event rate
    /// path on a steady drag, but fast enough that "cursor held
    /// still after a fast scratch" responds within one perceptual
    /// frame.
    private static let scratchTickIntervalSecs: TimeInterval = 1.0 / 60.0
    /// If no `scratchPointerOffset` event has fired within this
    /// window, the watchdog treats the cursor as "still" and
    /// ramps the deck's rate toward zero. 50 ms is longer than a
    /// smooth drag's inter-arrival time (≈ 8–17 ms at 60–120 Hz)
    /// but tolerates main-thread stalls from Metal redraw without
    /// falsely decaying the rate mid-drag; still short enough that
    /// holding the cursor still after a fast scratch reads as
    /// immediate silence.
    private static let scratchStallThresholdSecs: TimeInterval = 0.050
    /// Smoothing time constant for the **time-invariant** EMA on
    /// the scratch's per-event instantaneous rate (M11d.5 round 6).
    ///
    /// Previously this was a fixed `alpha = 0.35` per-event constant,
    /// which weighted each event equally regardless of how long it
    /// took to arrive. On a 60 Hz cursor stream that gives the
    /// intended ~50 ms smoothing window, but when macOS delivers
    /// gesture events in bursts (multiple `onChanged` within ~1 ms
    /// after the main thread was busy — a normal occurrence under
    /// any Canvas / Metal redraw load) each burst event still landed
    /// 35 % of the new instantaneous rate even though its `dt` was
    /// 1/16 of a normal frame. The result was that `delta / dt`
    /// spiked toward the rate clamp on each burst sample, the EMA
    /// pulled the smoothed rate up by several ×, and the user heard
    /// "scrubbing is accelerating crazy" — sudden audio rate surges
    /// on what felt like a steady physical drag.
    ///
    /// Fix: compute the EMA alpha from `dt` as
    /// `1 − exp(−dt / scratchRateEMATauSecs)`. Each event then
    /// contributes proportional to the slice of time it represents,
    /// so a 1-ms burst event lands ~2 % weight (vs the old 35 %)
    /// while a normal 16.67-ms event lands ~28 % — the same as the
    /// pre-rework feel for steady drags. Bursts get averaged into
    /// the running rate at their fair time weight, not at the
    /// inflated event-count weight.
    ///
    /// `0.030 s` (30 ms) chosen so the smoothing window is ~2 frames
    /// at 60 Hz: short enough to read as direct (a deliberate
    /// direction change is in the output rate within 2 frames),
    /// long enough that random burst patterns average out cleanly.
    private static let scratchRateEMATauSecs: Double = 0.030
    /// Multiplicative decay applied to the smoothed rate on each
    /// watchdog tick when the cursor is still. Picked so the rate
    /// halves in ~3 ticks (≈ 50 ms): fast enough to read as
    /// "let go of the platter" but smooth enough that the engine
    /// doesn't see a discontinuity that the platter de-click
    /// might otherwise punch through to the speakers.
    private static let scratchRateStallDecay: Double = 0.7

    /// Begin a vinyl-style scratch on `side`. Captures the pre-
    /// scratch transport, engages Panic Play (Timecode mode only),
    /// freezes the playhead via `rate = 0` + `playing = true`, and
    /// spins up the rate-from-velocity polling timer.
    ///
    /// Idempotent on a deck that's already scratching — the second
    /// begin is a no-op so the lazy-begin pattern in the gesture
    /// overlay (begin on every `onChanged` until we see one)
    /// doesn't clobber the captured prior state.
    func scratchBegin(side: DeckSide) {
        guard isRunning else { return }
        let deck = state(for: side)
        guard deck.hasTrack else { return }
        if scratchStates[side] != nil { return }

        let prior = deck.isPlaying
        var engagedPanic = false
        if engineMode == .timecode && !deck.isPanicPlay {
            // Decouple from the timecode driver so our setDeckRate
            // sticks. `panic` updates `isPanicPlay` optimistically.
            panic(side: side)
            engagedPanic = true
        }

        do {
            try engine.setDeckRate(deckIdx: side.ffiDeckIdx, rate: 0.0)
        } catch {
            surfaceError("Scratch start failed: \(error.localizedDescription)")
            return
        }

        if !state(for: side).isPlaying {
            do {
                try engine.play(deckIdx: side.ffiDeckIdx)
                var s = state(for: side)
                s.isPlaying = true
                setState(s, for: side)
            } catch {
                surfaceError("Scratch start failed: \(error.localizedDescription)")
            }
        }

        scratchStates[side] = ScratchState(
            side: side,
            priorIsPlaying: prior,
            engagedPanic: engagedPanic,
            scratchStartElapsed: engine.position(deckIdx: side.ffiDeckIdx).elapsedSecs,
            startedAt: Date())
        ensureScratchTimerRunning()
        // Publish last — flipping `scratchingDeck` enables the 60 Hz
        // Metal draw loop and can invalidate a large SwiftUI subtree;
        // keep it after the engine commands so the first scrub sample
        // lands before we pay for rendering.
        scratchingDeck = side
    }

    /// Report the mouse cursor's running offset (in audio seconds)
    /// from the scratch's start point. Positive = forward; negative
    /// = reverse. Each call drives an immediate per-event rate
    /// update so the engine never sees a 60 Hz-aliased velocity
    /// (M10.5t — pre-rework this lived in `scratchTick` and produced
    /// audible "jumping" when the event stream and the tick clock
    /// beat against each other).
    func scratchPointerOffset(side: DeckSide, offsetSecs: Double) {
        guard let state = scratchStates[side] else { return }
        let now = Date()
        let dt = now.timeIntervalSince(state.lastEventAt)
        let delta = offsetSecs - state.lastEventOffsetSecs
        state.lastEventOffsetSecs = offsetSecs
        state.lastEventAt = now

        // First event after begin has `delta == 0` (offsetSecs is 0
        // by definition at the drag's start). Skip the rate update
        // — using `dt` here would compute a zero rate based on a
        // potentially long gap since scratchBegin, and using the
        // raw `delta` would compute a meaningless rate from an
        // artificial zero baseline. The next event provides the
        // first real velocity sample.
        guard dt > 0, abs(delta) > 0 else { return }

        // Floor dt so coalesced cursor events (macOS sometimes
        // delivers several gesture events within < 1 ms of each
        // other after a stall) don't divide by an absurdly small
        // number on the way to `instantRate`. The time-invariant
        // EMA below also limits how much such a sample can move
        // the smoothed rate (its `alpha` collapses toward zero as
        // `dt` shrinks), but flooring `dt` here keeps
        // `instantRate` itself from saturating.
        let effectiveDt = max(dt, 0.001)
        let instantRate = delta / effectiveDt
        // Time-invariant exponential moving average. See the
        // `scratchRateEMATauSecs` doc above for why this replaced
        // the previous fixed `alpha = 0.35`. The identity
        // `1 − exp(−dt/τ)` is the standard continuous-time RC-style
        // low-pass discretised at irregular event timestamps; for
        // very small `dt` it linearises to `dt/τ`, which is what
        // forces burst events to land sub-3 % weight on the
        // smoothed rate.
        let alpha = 1.0 - exp(-dt / Self.scratchRateEMATauSecs)
        let smoothed =
            alpha * instantRate + (1.0 - alpha) * state.smoothedRate
        // Clamp to a sane range so a glitched event burst doesn't
        // send the playhead off to lunch. ±8× is the upper bound
        // a turntablist would ever hand-spin a platter at; the
        // engine itself accepts wider but the resampler quality
        // falls off past ~4×.
        let clamped = max(-8.0, min(8.0, smoothed))
        state.smoothedRate = clamped
        try? engine.setDeckRate(
            deckIdx: side.ffiDeckIdx,
            rate: clamped)
    }

    /// End an in-flight scratch on `side`. Stops the watchdog
    /// timer for this deck, sets rate back to 1.0, restores the
    /// pre-scratch play / pause state, and cancels Panic Play if
    /// we engaged it on `scratchBegin`. No-op on a side that
    /// isn't currently scratching.
    func scratchEnd(side: DeckSide) {
        guard scratchStates[side] != nil else { return }

        // Tear down the watchdog before restoring transport. A tick
        // that's already iterating holds a strong ref to the deck's
        // ScratchState and can otherwise issue a decayed rate *after*
        // we send unity here — the "pitch dropped after scrub" bug.
        if scratchStates.count == 1 {
            scratchTimer?.invalidate()
            scratchTimer = nil
        }

        guard let ended = scratchStates.removeValue(forKey: side) else { return }
        scratchingDeck = scratchStates.isEmpty ? nil : scratchStates.keys.first
        if scratchStates.isEmpty {
            scratchTimer?.invalidate()
            scratchTimer = nil
        }

        let idx = side.ffiDeckIdx
        let targetSecs = ended.scratchStartElapsed + ended.lastEventOffsetSecs
        do {
            seekDeck(side: side, absoluteSecs: targetSecs)
            try engine.setDeckRate(deckIdx: idx, rate: 1.0)
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Scratch end failed: \(error.localizedDescription)")
        }

        if ended.engagedPanic {
            cancelPanic(side: side)
        }
        if ended.priorIsPlaying {
            do {
                try engine.play(deckIdx: idx)
                var s = state(for: side)
                s.isPlaying = true
                setState(s, for: side)
            } catch let error as EngineError {
                surfaceError(describe(error))
            } catch {
                surfaceError("Scratch end failed: \(error.localizedDescription)")
            }
        } else {
            pause(side: side)
        }
    }

    private func ensureScratchTimerRunning() {
        if scratchTimer != nil { return }
        let timer = Timer.scheduledTimer(
            withTimeInterval: Self.scratchTickIntervalSecs, repeats: true
        ) { [weak self] _ in
            self?.scratchTick()
        }
        // No tolerance — the watchdog catches cursor-still windows
        // and needs predictable cadence to ramp the rate down on
        // a known schedule.
        RunLoop.main.add(timer, forMode: .common)
        scratchTimer = timer
    }

    private func scratchTick() {
        guard !scratchStates.isEmpty else {
            scratchTimer?.invalidate()
            scratchTimer = nil
            return
        }
        let now = Date()
        for side in scratchStates.keys {
            guard let state = scratchStates[side] else { continue }
            let stalledFor = now.timeIntervalSince(state.lastEventAt)
            guard stalledFor > Self.scratchStallThresholdSecs else { continue }
            // Cursor is still — ramp the rate toward zero. The
            // multiplicative decay produces a brief audible
            // run-out (matching how a real platter coasts after
            // the DJ lifts their finger) rather than slamming to
            // a hard zero, which the audio thread's own platter
            // de-click would otherwise need to absorb in one
            // block.
            if state.smoothedRate == 0 { continue }
            var next = state.smoothedRate * Self.scratchRateStallDecay
            if abs(next) < 0.01 { next = 0 }
            state.smoothedRate = next
            // scratchEnd may have removed this deck while we were
            // computing `next`; don't clobber a unity-rate restore.
            guard scratchStates[side] != nil else { continue }
            try? engine.setDeckRate(
                deckIdx: side.ffiDeckIdx,
                rate: next)
        }
    }

    /// Shared seek + optimistic UI update. Used by the overview's
    /// click-to-jump (PRD §6.1) and the Casual-Play restart path.
    /// Surfaces engine errors in the status strip rather than
    /// throwing.
    func seekDeck(side: DeckSide, absoluteSecs: TimeInterval) {
        guard isRunning else { return }
        let deck = state(for: side)
        guard deck.hasTrack, deck.durationSecs > 0 else { return }
        let clamped = max(0, min(deck.durationSecs, absoluteSecs))
        do {
            try engine.seek(deckIdx: side.ffiDeckIdx, positionSecs: clamped)
            var s = state(for: side)
            s.atEnd = false
            setState(s, for: side)
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Seek failed: \(error.localizedDescription)")
        }
    }

    // MARK: Panic Play (PRD §6.1.2 / M10.6c)

    /// Engage Panic Play on `side`. Engine decouples the deck from
    /// its timecode input and holds the last-known forward velocity
    /// (M10.6b engine logic). UI-side we set `isPanicPlay` optimistically
    /// so the deck header pill / glyph flip without waiting for the
    /// next 30 Hz poll round-trip; the poll then keeps the field
    /// authoritative (in particular, picking up an engine-side
    /// auto-cancel on clean LFSR re-lock).
    ///
    /// No-op if the engine isn't running or the deck has no track —
    /// Panic Play needs audible audio to recover *to*. The deck-header
    /// glyph is gated to the same conditions, so this is mostly a
    /// defence-in-depth check.
    func panic(side: DeckSide) {
        guard isRunning else { return }
        let deck = state(for: side)
        guard deck.hasTrack else { return }
        do {
            try engine.panicPlay(deckIdx: side.ffiDeckIdx)
            var s = state(for: side)
            s.isPanicPlay = true
            s.isPlaying = true
            setState(s, for: side)
            lastPlayStart[side] = Date()
            recomputeMaster()
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Panic Play failed: \(error.localizedDescription)")
        }
    }

    /// Cancel Panic Play on `side` — Serato INT→ABS toggle.
    ///
    /// PRD §6.1.2 / M10.6d: the engine clears its panic flag but
    /// *does not* touch deck transport. The next render block lets
    /// the timecode driver re-engage on a healthy carrier (deck
    /// keeps playing) or pause the deck via the existing
    /// `DropoutHoldRate` arm on a silent / broken cartridge. We
    /// mirror that here: clear `isPanicPlay` optimistically but
    /// leave `isPlaying` alone — the 30 Hz poll will read whatever
    /// the engine decides ≤33 ms from now.
    func cancelPanic(side: DeckSide) {
        guard isRunning else { return }
        do {
            try engine.cancelPanicPlay(deckIdx: side.ffiDeckIdx)
            var s = state(for: side)
            s.isPanicPlay = false
            setState(s, for: side)
            recomputeMaster()
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Cancel Panic failed: \(error.localizedDescription)")
        }
    }

    /// Timecode-mode primary-button toggle (M10.6d UI redesign).
    /// Mirrors Serato's INT/ABS button: tap once to switch from
    /// platter-driven playback to internal (panic engaged), tap
    /// again to switch back. The deck-header transport button uses
    /// this directly when `engineMode == .timecode` and a track is
    /// loaded; Prep mode still routes through `play` / `pause`.
    func panicToggle(side: DeckSide) {
        if state(for: side).isPanicPlay {
            cancelPanic(side: side)
        } else {
            panic(side: side)
        }
    }

    // MARK: Helpers

    /// Single sink for surfaceable user-facing errors. Updates
    /// `lastError` and schedules a `Task` to clear it after
    /// `errorVisibilitySecs`, cancelling any prior pending clear.
    /// Passing `nil` clears immediately.
    func surfaceError(_ message: String?) {
        lastErrorClearTask?.cancel()
        lastErrorClearTask = nil
        lastError = message
        guard message != nil else { return }
        let task = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: Self.errorVisibilitySecs)
            guard let self else { return }
            if !Task.isCancelled {
                self.lastError = nil
            }
        }
        lastErrorClearTask = task
    }

    private func flashLoadError(side: DeckSide) {
        // 200 ms red flash per PRD §5.5: "deck is playing — lift the
        // needle". Long enough to register, short enough not to
        // intrude on the next attempt.
        var s = state(for: side)
        s.errorFlashUntil = Date().addingTimeInterval(0.2)
        setState(s, for: side)
    }

    private func state(for side: DeckSide) -> DeckState {
        switch side {
        case .a: return deckA
        case .b: return deckB
        }
    }

    private func setState(_ next: DeckState, for side: DeckSide) {
        switch side {
        case .a: deckA = next
        case .b: deckB = next
        }
    }

    private func formatChip(for url: URL, info: TrackInfo) -> String {
        let ext = url.pathExtension.uppercased()
        let sr = String(format: "%.1f kHz", Double(info.sampleRate) / 1000.0)
        let ch = info.channels == 1 ? "mono" : "stereo"
        return "\(ext) · \(sr) · \(ch)"
    }

    // MARK: Channel parsing

    private enum ParseResult {
        case success([UInt32])
        case failure(String)
    }

    private func parseChannels(_ text: String, side: String) -> ParseResult {
        let parts = text.split(separator: ",").map {
            $0.trimmingCharacters(in: .whitespaces)
        }
        guard parts.count == 2 else {
            return .failure(
                "Deck \(side) channels need exactly two values, e.g. '1,2' or '3,4'.")
        }
        var out: [UInt32] = []
        for p in parts {
            guard let v = UInt32(p), v >= 1 else {
                return .failure(
                    "Deck \(side): '\(p)' is not a 1-based channel number.")
            }
            out.append(v)
        }
        return .success(out)
    }

    private func describe(_ error: EngineError) -> String {
        switch error {
        case .DeviceNotFound:       return "Device not found."
        case .InvalidChannels:      return "Invalid / overlapping channels — use two 1-based numbers per deck."
        case .AudioStartFailed:     return "Audio start failed."
        case .AlreadyRunning:       return "Engine already running."
        case .NotRunning:           return "Engine not running."
        case .InvalidDeckIndex:     return "Invalid deck index."
        case .TrackDecodeFailed:    return "Couldn't decode that track."
        case .CommandChannelFull:   return "Audio thread is overloaded — try again."
        case .EngineNotRunning:     return "Engine isn't running — Start it from Preferences (⌘,)."
        }
    }
}

// MARK: - Top-level shell

/// Top-level shell: the performance surface plus a `⌘,`-triggered
/// Preferences sheet.
struct MainView: View {

    @StateObject private var model = WaveformAppModel()
    @State private var showingPreferences: Bool = false

    var body: some View {
        PerformanceView(model: model, openPreferences: { showingPreferences = true })
            .frame(minWidth: 960, minHeight: 600)
            .sheet(isPresented: $showingPreferences) {
                PreferencesSheet(model: model)
            }
            .background(
                KeyEventMonitorHost(
                    showingPreferences: $showingPreferences,
                    model: model)
            )
            // M10.5b "no Apply button" UX: every Preferences-driven
            // config change auto-applies. `applyConfig()` starts the
            // engine when stopped and restarts it when running, so
            // the user only ever needs to *change* a setting; the
            // engine catches up on its own.
            .onChange(of: model.engineMode) { _ in
                model.applyConfig()
            }
            .onChange(of: model.selectedDevice) { _ in
                model.applyConfig()
            }
            .onAppear {
                // Cold-boot auto-start: if a valid config exists for
                // the auto-detected mode (Prep always works; Timecode
                // works as long as `selectedDevice` is set), spin up
                // the engine. If start fails (no device + Timecode
                // selected), `surfaceError` will display the reason
                // in the status strip and the user can open
                // Preferences from the gear icon to fix it.
                if !model.isRunning {
                    model.applyConfig()
                }
                // M11d.1: open the library handle on cold boot so
                // the LibraryView can render rows without forcing
                // the user through an explicit "open library"
                // affordance. Idempotent; safe if the engine
                // applyConfig path also touched something library-
                // adjacent in a future milestone.
                model.openLibraryIfNeeded()
            }
    }
}

// MARK: - Keyboard event monitor

/// Hidden NSView host that installs an `NSEvent.addLocalMonitorForEvents`
/// handler at view-mount. Keyboard shortcuts placed on SwiftUI
/// `Button`s with `.opacity(0)` are unreliable — when a child view
/// (the FileBrowserView's scroll-view, a TextField, etc.) holds
/// keyboard focus, the synthetic Button doesn't fire. NSEvent's
/// local monitor intercepts every keyDown delivered to the
/// application before any first responder gets it, which is the
/// only way to make `Space` work the way `⌘,` does in macOS.
private struct KeyEventMonitorHost: NSViewRepresentable {
    @Binding var showingPreferences: Bool
    let model: WaveformAppModel

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        context.coordinator.install(
            onSpace: {
                Task { @MainActor in
                    await model.loadBrowserSelectionIntoTargetDeck()
                }
                return true
            },
            onCmdComma: {
                Task { @MainActor in
                    showingPreferences.toggle()
                }
                return true
            })
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        // Bindings are captured by reference; no per-update work
        // required — the monitor stays installed for the
        // coordinator's lifetime.
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.uninstall()
    }

    @MainActor
    final class Coordinator {
        private var monitor: Any?

        func install(
            onSpace: @escaping () -> Bool,
            onCmdComma: @escaping () -> Bool
        ) {
            uninstall()
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self else { return event }
                let isCmd = event.modifierFlags.contains(.command)
                if isCmd, event.charactersIgnoringModifiers == "," {
                    if onCmdComma() { return nil }
                    return event
                }
                // Don't intercept Space while the user is typing
                // into a TextField (Preferences channel fields,
                // future search boxes, etc.). `⌘,` is a global
                // shortcut so it always wins.
                if self.isTextFirstResponder() {
                    return event
                }
                // `keyCode 49` is the spacebar on every Apple keyboard
                // layout (the keyCodes are layout-independent for the
                // physical-key tier of NSEvent).
                if !isCmd, event.keyCode == 49 {
                    if onSpace() { return nil }
                }
                return event
            }
        }

        func uninstall() {
            if let m = monitor { NSEvent.removeMonitor(m) }
            monitor = nil
        }

        private func isTextFirstResponder() -> Bool {
            guard let responder = NSApp.keyWindow?.firstResponder else {
                return false
            }
            return responder is NSText || responder is NSTextView
        }

        deinit {
            if let m = monitor { NSEvent.removeMonitor(m) }
        }
    }
}

#Preview {
    MainView()
        .frame(width: 1440, height: 900)
}
