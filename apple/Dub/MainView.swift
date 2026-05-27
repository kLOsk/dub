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

/// Immediate LibraryView row patch after a successful
/// `analyze_track` call. Keeps the BPM column in sync with prep-
/// mode deck loads without waiting for the async listing refetch.
struct LibraryRowAnalysisUpdate: Equatable {
    let trackId: String
    let bpm: Double?
    let key: String?
    let isAnalyzed: Bool
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

    /// M10.5b `peaks_generation` mirror for SwiftUI consumers.
    /// Refreshed by the 30 Hz poll so `TrackOverviewView` can
    /// re-decimate when the offline-peaks worker lands after
    /// `load_track` (Phase 4 bumps the engine counter ~10–30 ms
    /// after `hasTrack` flips true).
    var peaksGeneration: UInt64 = 0

    /// Bumped on every successful overview / scrub seek so paused
    /// decks force a one-shot Metal redraw (`WaveformView` runs
    /// on-demand when not playing).
    var seekGeneration: UInt64 = 0

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

    /// M11d.6 auto-detected grid captured at first analysis (before
    /// manual edits). Used by the calibration log.
    var autoGridBpm: Double? = nil
    var autoGridAnchorSecs: Double? = nil
    var autoGridCaptured: Bool = false
    /// `"auto"`, `"user_tap"`, import source, or `"pending_auto"`.
    var beatGridLoadSource: String? = nil
    /// Count of manual nudge actions this session (phase / BPM / tap).
    var manualGridEditCount: Int = 0

    /// M11d.7 — stamped from library row at load time.
    var gridLocked: Bool = false
    var gridDriftQuality: Float? = nil

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

    /// FS-browser selection now lives on `LibraryAppModel` so a
    /// library / browser row click does not fire
    /// `WaveformAppModel.objectWillChange` and therefore does not
    /// cascade through `PerformanceView` / both waveform Metal
    /// views / both `TrackOverviewView`s. See
    /// `LibraryAppModel.browserSelection` for the full rationale.
    /// Callers should read / write `librarySelection.browserSelection`.

    // MARK: Library (M11d)

    /// Shared library handle backing the M11d browser. Construction
    /// is cheap (no SQLite connection until `openLibrary()` lands).
    /// The handle outlives any one browser view, so search results
    /// and import progress survive transient view churn (sidebar
    /// switches, window resize, etc.).
    let library: DubLibrary = DubLibrary()

    /// M11d.7 per-deck tap-to-grid controllers.
    private let tapToGridA = TapToGridController()
    private let tapToGridB = TapToGridController()

    /// Per-deck published surfaces for the deck-header BPM column
    /// tap-session indicators (`(N)` count chip + italic rolling
    /// preview). Held as plain `let`s (not `@Published`) so a tap
    /// only invalidates the `DeckHeader` that observes the matching
    /// session — `PerformanceView`, `LibraryView`,
    /// `FileBrowserView`, and both `TrackOverviewView`s stay inert.
    /// See `TapSessionViewModel`'s doc for the rationale.
    let tapSessionA = TapSessionViewModel()
    let tapSessionB = TapSessionViewModel()

    private func tapToGrid(for side: DeckSide) -> TapToGridController {
        side == .a ? tapToGridA : tapToGridB
    }

    func tapSession(for side: DeckSide) -> TapSessionViewModel {
        side == .a ? tapSessionA : tapSessionB
    }

    private func wireTapToGridControllers() {
        tapToGridA.onTapCountChanged = { [weak self] count in
            self?.tapSessionA.tapCount = count
        }
        tapToGridA.onCommit = { [weak self] taps in
            self?.commitTapGrid(side: .a, playheadTimes: taps)
        }
        tapToGridA.onRollingBpmChanged = { [weak self] bpm in
            self?.tapSessionA.rollingBpm = bpm
        }
        tapToGridB.onTapCountChanged = { [weak self] count in
            self?.tapSessionB.tapCount = count
        }
        tapToGridB.onCommit = { [weak self] taps in
            self?.commitTapGrid(side: .b, playheadTimes: taps)
        }
        tapToGridB.onRollingBpmChanged = { [weak self] bpm in
            self?.tapSessionB.rollingBpm = bpm
        }
    }

    /// PRD-BEATS §4.2 + §7 gates 8 & 9. The deck-header BPM column
    /// is the only entry point for tap-tempo. We gate the tap here
    /// rather than inside `TapToGridController` so the controller
    /// stays a pure session collector: locked-grid rejection (§3.5
    /// "lock is absolute") and transport-playing precondition for
    /// the 3+ tap dispatch (§4.2 "no audible reference") live in
    /// the UI layer where the deck state is authoritative.
    func handleTapForGrid(_ side: DeckSide) {
        guard isRunning else { return }
        let deck = state(for: side)
        guard deck.hasTrack, deck.bpm != nil else { return }
        if deck.gridLocked {
            return
        }
        let tapController = tapToGrid(for: side)
        let isPlaying = deck.isPlaying
        let playhead = engine.position(deckIdx: side.ffiDeckIdx).elapsedSecs
        // Paused decks dispatch a single set-the-1 immediately
        // through a dedicated path that drops any stale buffered
        // session first. PRD-BEATS §4.2 + gate 9 already rejects
        // the 3+ tap upgrade on paused decks (the user can't hear
        // the track), so the 1.5 s idle window adds zero value;
        // worse, when the user pauses inside a still-fresh playing
        // tap window the buffered playing-tap playhead would leak
        // into the paused commit and the yellow downbeat would
        // land at the prior tap's position instead of the user's
        // current click. `commitSingleTap` cancels that buffer
        // before dispatching the fresh playhead, and the
        // `persistTapGrid` `seekGeneration` bump forces a paused
        // MTKView redraw on the same vsync so the marker snaps
        // into place without requiring Play.
        if !isPlaying {
            tapController.commitSingleTap(playheadSecs: playhead)
            return
        }
        // Playing-deck tap session: 1.5 s idle window so the user
        // can extend a 1-tap set-the-1 into a 3+ tap constrained
        // re-analysis without us committing too early. Locked-grid
        // rejection (§3.5) and the upgrade precondition
        // (§4.2 / gate 9) already passed above.
        tapController.tap(playheadSecs: playhead)
    }

    /// Library / analysis / relocate UI surface, owned as a
    /// separate `ObservableObject` so library mutations don't
    /// invalidate `PerformanceView`. Held as a plain `let` (not
    /// `@Published`) — observers subscribe via
    /// `model.libraryModel` directly. See `LibraryAppModel` for
    /// the field-by-field rationale.
    let libraryModel = LibraryAppModel()

    /// M11d.6 round 2 — selection side-channel split out of
    /// `LibraryAppModel`. Holds `browserSelection`,
    /// `selectedLibraryTrackId`, `selectedLibraryTrack`. The
    /// rationale lives in `LibrarySelectionModel`'s header
    /// comment; tl;dr the three fields are **never read** from
    /// `LibraryView`'s body, only written, so observing them
    /// from there is a pure wasted-cascade cost on every row
    /// click. The new model is observed only by call sites that
    /// actually consume the selection (the deck load path,
    /// the Space-loader, FileBrowserView in previews).
    let librarySelection = LibrarySelectionModel()

    /// Unix-seconds boundary for the "Just Imported" smart crate
    /// per PRD §8.5.2. Captured at app launch so a DJ who plugs in
    /// a USB stick 10 minutes before the gig sees exactly the
    /// tracks they imported during this session.
    let appLaunchUnixSeconds: Int64 = Int64(Date().timeIntervalSince1970)

    /// M11c.1 — set of track UUIDs currently in flight. Guards
    /// against double-analyzing the same track when the user
    /// rapid-fires Space + Right-click → Analyze, and is consulted
    /// before queueing each batch-analyze entry.
    private var analyzingTrackIds: Set<String> = []

    // MARK: Private state

    /// Sticky master from the previous round when neither deck is
    /// currently playing. Starts at `.a` so the cold-launch UI has
    /// a definite anchor.
    private var stickyMaster: DeckSide = .a
    private var lastPlayStart: [DeckSide: Date] = [:]

    /// Polling timer for the deck-chrome `@Published` mirrors
    /// (`hasTrack`, `isPlaying`, `durationSecs`, `peaksGeneration`,
    /// `errorFlashUntil`, etc.). Runs at ~10 Hz — the deck-header
    /// chrome doesn't need frame-accurate updates because the
    /// **time row reads `engine.position(deckIdx:)` directly via
    /// the `LiveDeckTimeText` `TimelineView` subtree** and the
    /// Metal waveform refreshes off its own `CVDisplayLink` /
    /// peak-generation observer. Pre-fix the timer ran at 30 Hz,
    /// which meant `WaveformAppModel.deckA` / `.deckB` republished
    /// 30× per second on top of any genuine state change. The
    /// resulting SwiftUI invalidation cascade competed with the
    /// 60 Hz Metal render thread for main-actor time and shaved a
    /// visible margin off waveform smoothness during playback.
    /// 10 Hz keeps the worst-case latency for chrome that **does**
    /// react to polled values (PRD §6.1.2 Panic-Play pill, M11
    /// peaks-generation swap, `errorFlashUntil` clear) ≤100 ms —
    /// well inside human perception while cutting the cascade
    /// frequency by 3×. Disabled when the engine isn't running.
    private var pollTimer: Timer?
    private static let pollIntervalSecs: TimeInterval = 1.0 / 10.0

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
        wireTapToGridControllers()
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
        // `Timer.scheduledTimer` takes a `@Sendable` closure that
        // the compiler treats as non-isolated. We need to call
        // `pollDecks()` (MainActor-isolated, mutates `@Published`
        // `deckA` / `deckB`) without paying for a `Task { @MainActor
        // in … }` dispatch hop on every 30 Hz tick. The timer is
        // explicitly added to `RunLoop.main` below, so the callback
        // is guaranteed to fire on the main thread —
        // `MainActor.assumeIsolated` encodes that runtime invariant
        // for the type system. The asserting form is intentional:
        // if a future change ever attaches the timer to a non-main
        // runloop, we want a loud trap, not a silent `@Published`
        // race that the user would only see as occasional missed
        // 30 Hz frames or stale BPM digits.
        let timer = Timer.scheduledTimer(
            withTimeInterval: Self.pollIntervalSecs, repeats: true
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.pollDecks()
            }
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
        // M11d.6 round 5 — lock-free FFI snapshot. The position
        // read no longer contends with library / load-track work
        // because it bypasses the engine mutex entirely.
        let pos = engine.positionSnapshot(deckIdx: side.ffiDeckIdx)
        let nowPlaying = pos.isPlaying
        if nowPlaying, !prev.isPlaying {
            lastPlayStart[side] = Date()
        }
        var next = prev
        next.hasTrack = pos.hasTrack
        next.isPlaying = nowPlaying
        next.atEnd = pos.atEnd
        next.durationSecs = pos.durationSecs
        next.peaksGeneration = engine.peaksGeneration(deckIdx: side.ffiDeckIdx)
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
                    // Deck just resolved a BPM that the library row
                    // had been waiting on. The lazy
                    // `ensureTrackAnalyzed` triggered at load time
                    // is what eventually writes that BPM into
                    // `track_beatgrids`, but if `recordLoad`
                    // happened to throw (legacy do/catch swallow)
                    // OR the analyze finishes *after* the engine's
                    // own analyzer, the library row would never get
                    // a BPM until the user re-selected the track.
                    // Calling `ensureTrackAnalyzed` here is
                    // idempotent (the in-flight set + the
                    // `is_track_analyzed` cache early-out makes
                    // repeat calls free) and closes that gap so
                    // the library catches up the same render tick
                    // the deck header lights up.
                    if let trackId = next.loadedLibraryTrackId {
                        ensureTrackAnalyzed(trackId: trackId)
                    }
                }
            }
        } else {
            bpmPollTick[side] = 0
        }
        captureAutoGridIfNeeded(side: side, deck: &next)
        return next
    }

    /// Stamp the first auto-analysed grid into `DeckState` and log
    /// it for offline beatgrid calibration. Skips tracks that loaded
    /// a pre-existing user/import grid.
    private func captureAutoGridIfNeeded(side: DeckSide, deck: inout DeckState) {
        guard deck.hasTrack, !deck.autoGridCaptured, deck.bpm != nil else { return }
        let grid = engine.beatGrid(deckIdx: side.ffiDeckIdx)
        guard grid.confidence > 0,
              grid.bpm > 0,
              let firstBeat = grid.beats.first
        else { return }

        deck.autoGridCaptured = true
        let source = deck.beatGridLoadSource ?? "auto"
        if source == "pending_auto" {
            deck.beatGridLoadSource = "auto"
        }
        guard source == "auto" || source == "pending_auto" else { return }

        deck.autoGridBpm = grid.bpm
        deck.autoGridAnchorSecs = Double(firstBeat)
        BeatgridCalibrationLog.logAutoGrid(
            side: "\(side)",
            trackId: deck.loadedLibraryTrackId,
            path: deck.sourceURL?.path,
            title: deck.trackTitle ?? deck.displayName,
            artist: deck.trackArtist,
            durationSecs: deck.durationSecs,
            source: deck.beatGridLoadSource ?? "auto",
            bpm: grid.bpm,
            anchorSecs: Double(firstBeat),
            confidence: Double(grid.confidence),
            beatCount: grid.beats.count)
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

        finalizeBeatgridSessionIfNeeded(side: side, deck: target)

        let preloadedGrid = libraryBeatGridForPendingLoad(url: url)

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
        starting.autoGridBpm = nil
        starting.autoGridAnchorSecs = nil
        starting.autoGridCaptured = false
        starting.beatGridLoadSource = preloadedGrid?.source ?? "pending_auto"
        starting.manualGridEditCount = 0
        tapToGrid(for: side).cancel()
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
        let genreForLoad = libraryGenreForPendingLoad(url: url)
        let result: Result<Void, Error> = await Task.detached(priority: .userInitiated) {
            do {
                try engineRef.loadTrack(
                    deckIdx: deckIdx,
                    path: url.path,
                    libraryBeatGrid: preloadedGrid,
                    genre: genreForLoad
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
            // PRD-BEATS §4.5 instant-display contract: whenever
            // the library has ANY analyzed grid for this track
            // (auto, imported, or user_tap), publish the library
            // row's BPM the same render tick the load handshake
            // resolves. There is no "wait for the engine to re-
            // confirm" branch any more. The engine's later poll
            // tick reports the same number (the engine's grid is
            // built from the library row via `synthesise_beat_grid`
            // for non-auto / locked rows; for auto / unlocked rows
            // the engine re-runs analysis but the BPM should not
            // change for a deterministic track + algorithm pair,
            // and the previous "show — until re-analysis lands"
            // branch was the source of the user's "BPM doesn't
            // appear immediately" complaint).
            //
            // Library-row → deck-header parity is gate 14 in
            // PRD-BEATS §7. The library row publisher and the
            // engine poll publisher must agree the same render
            // frame the load completes; this branch is the
            // single source of truth at load time.
            if let supplied = preloadedGrid {
                next.bpm = supplied.bpm
                next.bpmConfidence = 1.0
            } else {
                next.bpm = nil
                next.bpmConfidence = 0
            }
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
        guard !libraryModel.libraryIsOpen else { return }
        do {
            try library.openDefault()
            libraryModel.libraryIsOpen = true
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
        guard libraryModel.libraryIsOpen else { return }
        if let count = try? library.trackCount() {
            libraryModel.libraryTrackCount = count
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
        var next = libraryModel.volumeReachability
        // Drop entries for mount points no longer in view so the
        // cache stays bounded.
        next = next.filter { mountPoints.contains($0.key) }
        for m in mountPoints {
            var isDir: ObjCBool = false
            let exists = FileManager.default.fileExists(atPath: m, isDirectory: &isDir)
            next[m] = exists && isDir.boolValue
        }
        if next != libraryModel.volumeReachability {
            libraryModel.volumeReachability = next
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
        return libraryModel.volumeReachability[m] == true
    }

    // MARK: - M11d.4 Missing-files scanner

    /// Refresh `missingTrackCount` from the library. Cheap
    /// (`COUNT(*)` over a small partial index). Called on
    /// app launch, after every import, and after each scanner
    /// batch + Relocate run.
    func refreshMissingTrackCount() {
        guard libraryModel.libraryIsOpen else { return }
        if let n = try? library.missingTrackCount() {
            libraryModel.missingTrackCount = n
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
        guard libraryModel.libraryIsOpen else { return 0 }
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
                // One write per scanned row regardless of whether
                // `is_missing` flipped: `mark_file_state` also
                // advances `last_checked_at`, and that's what
                // `list_files_for_scan`'s stalest-first ORDER BY
                // uses to avoid re-picking the same rows on the
                // next pass. The previous if/else here branched on
                // `isMissing != r.wasMissing` but both arms called
                // `markFileState` with identical arguments — pure
                // dead code from an in-flight refactor. PRD §8.5.5
                // (rate-limited scanner) is unaffected: cadence is
                // governed by `startMissingFilesScanner`'s sleep
                // schedule, not by skipping writes.
                do {
                    try library.markFileState(
                        fileId: r.fileId,
                        isMissing: isMissing,
                        timestampUnixSecs: now
                    )
                } catch {
                    continue
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
        guard libraryModel.libraryIsOpen else { return (0, 0) }
        if libraryModel.relocateInProgress { return (0, 0) }
        libraryModel.relocateInProgress = true
        defer { libraryModel.relocateInProgress = false }

        let library = self.library
        let folderPath = folder.path
        let result: (UInt32, UInt32) = await Task.detached(priority: .userInitiated) {
            return Self.relocateImpl(library: library, folderPath: folderPath)
        }.value
        libraryModel.lastRelocateMatches = result.0
        libraryModel.lastRelocateUnmatched = result.1
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
        guard libraryModel.libraryIsOpen else {
            surfaceError("Library is not open yet.")
            return
        }
        if libraryModel.libraryImportInProgress {
            surfaceError("An import is already running.")
            return
        }
        libraryModel.libraryImportInProgress = true
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
        libraryModel.libraryImportInProgress = false
        switch result {
        case .success(let summary):
            libraryModel.lastImportSummary = summary
            refreshLibraryStats()
            refreshMissingTrackCount()
            let changed = summary.added
                + summary.refreshed
                + summary.merged
                + summary.siblingVersions
            if changed == 0 {
                if let first = summary.errors.first {
                    surfaceError("Import skipped \(summary.skipped) file(s): \(first)")
                } else {
                    surfaceError("Import found no supported audio files.")
                }
            }
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
        guard libraryModel.libraryIsOpen else { return }
        do {
            if let path = try library.trackPath(trackId: trackId) {
                librarySelection.browserSelection = URL(fileURLWithPath: path)
                librarySelection.selectedLibraryTrackId = trackId
                librarySelection.selectedLibraryTrack = snapshot
            } else {
                librarySelection.browserSelection = nil
                librarySelection.selectedLibraryTrackId = nil
                librarySelection.selectedLibraryTrack = nil
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
    private func libraryGenreForPendingLoad(url: URL) -> String? {
        guard libraryModel.libraryIsOpen,
              let trackId = librarySelection.selectedLibraryTrackId,
              let snap = librarySelection.selectedLibraryTrack,
              snap.id == trackId
        else { return nil }
        do {
            guard let resolved = try library.trackPath(trackId: trackId) else {
                return nil
            }
            let resolvedUrl = URL(fileURLWithPath: resolved).standardizedFileURL
            guard resolvedUrl == url.standardizedFileURL else {
                return nil
            }
            return snap.genre
        } catch {
            return nil
        }
    }

    private func libraryBeatGridForPendingLoad(url: URL) -> LibraryBeatGrid? {
        guard libraryModel.libraryIsOpen, let trackId = librarySelection.selectedLibraryTrackId else {
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
            let grid = try library.activeBeatGrid(trackId: trackId)
            return grid
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
        guard libraryModel.libraryIsOpen else {
            var cleared = state(for: side)
            cleared.loadedLibraryTrackId = nil
            setState(cleared, for: side)
            return
        }
        guard let trackId = resolveLibraryTrackId(for: url) else {
            var cleared = state(for: side)
            cleared.loadedLibraryTrackId = nil
            setState(cleared, for: side)
            return
        }
        do {
            // Stamp the loaded-now glyph + write the play_history
            // row. The track id comes from the browser selection
            // when it matches the load URL, or from a reverse
            // path lookup for library drags that bypassed selection.
            var stamped = state(for: side)
            stamped.loadedLibraryTrackId = trackId
            if librarySelection.selectedLibraryTrackId == trackId,
               let snap = librarySelection.selectedLibraryTrack, snap.id == trackId
            {
                stamped.key = snap.key
                stamped.gridLocked = snap.gridLocked
                stamped.gridDriftQuality = snap.gridDriftQuality
            }
            setState(stamped, for: side)

            let deck: UInt32 = (side == .a) ? 0 : 1
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            try library.recordLoad(trackId: trackId, deck: deck, timestampMs: nowMs)
        } catch {
            surfaceError("Failed to record play history: \(error.localizedDescription)")
        }
        // PRD-BEATS §4.5: lazy analyse fires regardless of whether
        // `recordLoad` succeeded. A transient `play_history` write
        // failure (DB lock, FK in flight while the importer is
        // still committing the row) must not swallow the library
        // analysis the deck-load is asking for — without this
        // separation, the deck would surface its engine-computed
        // BPM in the header but the library row would never get
        // updated because the `ensureTrackAnalyzed` call sits
        // inside the same `do` block as the failing `try`.
        ensureTrackAnalyzed(trackId: trackId)
    }

    /// Resolve a canonical library track id for a deck load URL.
    /// Prefers the current browser selection when its path matches;
    /// falls back to a reverse lookup so library drags still stamp
    /// `loadedLibraryTrackId` and fire lazy analysis.
    private func resolveLibraryTrackId(for url: URL) -> String? {
        let normalized = url.standardizedFileURL
        if let selected = librarySelection.selectedLibraryTrackId,
           let path = try? library.trackPath(trackId: selected),
           URL(fileURLWithPath: path).standardizedFileURL == normalized
        {
            return selected
        }
        if let id = try? library.trackIdForPath(path: normalized.path), !id.isEmpty {
            return id
        }
        return nil
    }

    private func publishLibraryRowAnalysisUpdate(
        trackId: String,
        outcome: LibraryAnalysisOutcome,
        refreshLoadedDecks: Bool = false
    ) {
        let bpm: Double? =
            (outcome.wroteGrid && outcome.gridAutoIsActive && outcome.bpm > 0)
            ? outcome.bpm : nil
        let key: String? =
            (outcome.wroteKey && !outcome.camelot.isEmpty) ? outcome.camelot : nil
        libraryModel.libraryRowAnalysisUpdate = LibraryRowAnalysisUpdate(
            trackId: trackId,
            bpm: bpm,
            key: key,
            isAnalyzed: true)
        libraryModel.analysisGeneration &+= 1
        if refreshLoadedDecks {
            refreshLoadedDecksAfterLibraryAnalysis(trackId: trackId, outcome: outcome)
        }
    }

    /// When library analysis finishes for a track that is already
    /// loaded on a deck, push the new grid into the engine and
    /// refresh deck chrome without a reload. Only invoked from
    /// explicit batch re-analyze — never from lazy deck-load
    /// analysis, which would race set-the-1 and reset `bar_phase`.
    private func refreshLoadedDecksAfterLibraryAnalysis(
        trackId: String,
        outcome: LibraryAnalysisOutcome
    ) {
        guard isRunning, libraryModel.libraryIsOpen else { return }
        let activeGrid = try? library.activeBeatGrid(trackId: trackId)
        for side in [DeckSide.a, DeckSide.b] {
            var deck = state(for: side)
            guard deck.loadedLibraryTrackId == trackId, deck.hasTrack else { continue }
            if outcome.wroteKey, outcome.keyAutoIsActive, !outcome.camelot.isEmpty {
                deck.key = outcome.camelot
            }
            guard let grid = activeGrid, grid.bpm > 0 else {
                setState(deck, for: side)
                continue
            }
            guard !deck.gridLocked, !grid.gridLocked else {
                setState(deck, for: side)
                continue
            }
            do {
                try engine.installBeatGridWithPhase(
                    deckIdx: side.ffiDeckIdx,
                    bpm: grid.bpm,
                    anchorSecs: grid.anchorSecs,
                    barPhase: grid.barPhase)
            } catch {
                setState(deck, for: side)
                continue
            }
            deck.bpm = grid.bpm
            deck.bpmConfidence = 1.0
            deck.gridDriftQuality = grid.gridDriftQuality
            deck.seekGeneration &+= 1
            setState(deck, for: side)
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
        guard libraryModel.libraryIsOpen else { return }
        guard !analyzingTrackIds.contains(trackId) else { return }
        // Cheap synchronous check on the calling actor — if the
        // track is already analyzed, skip the background task
        // entirely. Avoids spawning a Task for every Space-load
        // on a fully-analyzed library.
        if let analyzed = try? library.isTrackAnalyzed(trackId: trackId), analyzed {
            return
        }
        analyzingTrackIds.insert(trackId)
        libraryModel.analysisInFlightCount &+= 1
        let library = self.library
        // Swift 6 strict-concurrency: capture `[weak self]` on the
        // main-actor hop, not on the outer @Sendable Task body. The
        // detached body only touches `library` (Sendable) and the
        // `trackId` String; it doesn't need `self`. Reaching back
        // to the actor-isolated instance across the actor boundary
        // is done via the inner closure's own weak capture.
        Task.detached(priority: .background) {
            let result: Result<LibraryAnalysisOutcome, Error>
            do {
                let outcome = try library.analyzeTrack(trackId: trackId)
                result = .success(outcome)
            } catch {
                result = .failure(error)
            }
            await MainActor.run { [weak self] in
                guard let self = self else { return }
                self.analyzingTrackIds.remove(trackId)
                if self.libraryModel.analysisInFlightCount > 0 {
                    self.libraryModel.analysisInFlightCount -= 1
                }
                switch result {
                case .success(let outcome):
                    self.publishLibraryRowAnalysisUpdate(
                        trackId: trackId, outcome: outcome)
                case .failure(let err):
                    // A fresh track shouldn't be locked, but a
                    // racing manual lock toggle between the
                    // `isTrackAnalyzed` check and the actual
                    // analyze call could land us here. Silent
                    // skip per PRD-BEATS §13 (no error toast).
                    if case LibraryFfiError.GridLocked = err {
                        return
                    }
                    self.surfaceError(
                        "Analysis failed for track: \(err.localizedDescription)")
                }
            }
        }
    }

    /// M11d.7 — toggle grid lock from the library context menu.
    ///
    /// PRD-BEATS §3.5 + §7 gate 8 (defense in depth): when the
    /// flipped track is currently loaded on either deck, mirror
    /// the new lock state into the engine's per-deck `grid_locked`
    /// so the engine's `install_beat_grid_from_taps` /
    /// `relatch_beat_grid_at_downbeat` independently reject the
    /// next call. Without this, a user could toggle the lock from
    /// the library, then tap the BPM column before the next
    /// `loadTrack` runs — the Swift gate would still see the new
    /// lock (it reads `DeckState.gridLocked`, which we update
    /// below), but the Rust engine wouldn't, and a raced tap
    /// would slip past defense in depth.
    func setGridLocked(trackId: String, locked: Bool) async {
        guard libraryModel.libraryIsOpen else { return }
        do {
            try library.setGridLocked(trackId: trackId, locked: locked)
            libraryModel.analysisGeneration &+= 1
        } catch {
            surfaceError("Grid lock failed: \(error.localizedDescription)")
            return
        }
        for side in [DeckSide.a, DeckSide.b] {
            var deck = state(for: side)
            guard deck.loadedLibraryTrackId == trackId else { continue }
            deck.gridLocked = locked
            setState(deck, for: side)
            if isRunning {
                _ = try? engine.setDeckGridLocked(
                    deckIdx: side.ffiDeckIdx, locked: locked)
            }
        }
    }

    /// Wrap a transient `@Published` "in-progress" flag so SwiftUI
    /// is guaranteed at least one render tick where the flag is
    /// observable before `work` runs (and one before the reset).
    ///
    /// Regression class this exists to prevent: a `set + work +
    /// reset` sequence that runs entirely inside one main-actor
    /// turn (no real suspension point in `work`, or a `work`
    /// body that short-circuits) is published as a single batch
    /// by `ObservableObject`. SwiftUI commits all three changes
    /// into one render pass and the indicator never appears —
    /// the user sees nothing happen even though the work
    /// completed. The `analysisBatchTotal` footer-pill bug
    /// (Re-analyze on a track with a lazy analysis in flight
    /// silently skipped, the pill never showing) was an
    /// instance of this; this helper standardises the
    /// mitigation.
    ///
    /// Implementation: a one-frame sleep (≈ 16 ms at 60 Hz)
    /// between `set()` and `work()` releases the main actor
    /// long enough for SwiftUI's runloop pass to commit the
    /// publish. `Task.yield()` alone is not sufficient because
    /// SwiftUI's render queue is driven by the runloop, not the
    /// cooperative task scheduler. The post-`work` reset goes
    /// through `defer` so an exception in `work` still clears
    /// the flag.
    ///
    /// Apply this anywhere you'd otherwise write
    /// `flag = true; defer { flag = false }; work()` for
    /// transient progress flags. Long-running work that itself
    /// awaits a real I/O suspension point (e.g. `Task.detached`)
    /// doesn't need it, but using the helper consistently keeps
    /// the contract documented at the call site.
    @MainActor
    func withVisibleTransientFlag<T>(
        set: @MainActor () -> Void,
        reset: @MainActor () -> Void,
        _ work: @MainActor () async throws -> T
    ) async rethrows -> T {
        set()
        try? await Task.sleep(nanoseconds: 16_000_000)
        defer { reset() }
        return try await work()
    }

    /// M11c.1 — batch-analyze entry point. Drives the LibraryView
    /// right-click context menu's single Analyze / Re-analyze
    /// action (collapsed in round 3; see §4.4). Iterates serially
    /// so each analysis releases the library lock before the next
    /// acquires it (other UI queries can interleave). Updates the
    /// footer progress counters as it goes.
    ///
    /// PRD-BEATS §3.5 + §4.4: lock-is-absolute. Re-analyze is the
    /// only entry point now (the `forceReanalyze` parameter was
    /// removed when the Rust `analyze_track(force: bool)` signature
    /// collapsed in round 3). The batch always runs analysis on
    /// every selected track; the Rust side returns
    /// `LibraryFfiError.GridLocked` for any locked track and we
    /// surface that as a silent skip (per §13: no error toast).
    /// The right-click menu greys out the menu item for locked
    /// tracks so the user never reaches a state where they ask
    /// for re-analyze on a locked grid and get a refusal — this
    /// fallback exists only for batches where a track was locked
    /// concurrently between menu and dispatch.
    func analyzeTracks(_ trackIds: [String]) async {
        guard libraryModel.libraryIsOpen, !trackIds.isEmpty else { return }
        let library = self.library
        await withVisibleTransientFlag(
            set: {
                self.libraryModel.analysisBatchTotal = UInt32(trackIds.count)
                self.libraryModel.analysisBatchCompleted = 0
            },
            reset: {
                self.libraryModel.analysisBatchTotal = 0
                self.libraryModel.analysisBatchCompleted = 0
            }
        ) {
            await self.analyzeTracksBody(trackIds, library: library)
        }
    }

    private func analyzeTracksBody(_ trackIds: [String], library: DubLibrary) async {
        for trackId in trackIds {
            // Coalesce with any concurrent lazy analysis (e.g. one
            // kicked off by `recordLibraryLoadIfApplicable` or the
            // `readDeckState` BPM-resolution catch-up). Before this
            // wait the contains-check used to `continue` immediately
            // after bumping `analysisBatchCompleted` — for a single
            // track that path set `analysisBatchTotal = 1`, bumped
            // completed to 1, and ran the `defer` reset all in one
            // runloop turn, so SwiftUI never got a render tick where
            // `analysisBatchTotal > 0` was true and the footer pill
            // never appeared. Polling here yields between checks so
            // the @Published assignment above gets a chance to
            // propagate; once the lazy run clears the set we
            // re-enter the analyze normally (PRD-BEATS §4.4 treats
            // "Re-analyze" as idempotent — running the analyser
            // again on a fresh result is cheap and matches what the
            // user clicked for). The 10 s cap keeps us from
            // hanging the batch on a wedged background task.
            var waitTicks = 0
            while analyzingTrackIds.contains(trackId), waitTicks < 200 {
                try? await Task.sleep(nanoseconds: 50_000_000)
                waitTicks &+= 1
            }
            if analyzingTrackIds.contains(trackId) {
                libraryModel.analysisBatchCompleted &+= 1
                continue
            }
            analyzingTrackIds.insert(trackId)
            libraryModel.analysisInFlightCount &+= 1
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
            if libraryModel.analysisInFlightCount > 0 { libraryModel.analysisInFlightCount -= 1 }
            libraryModel.analysisBatchCompleted &+= 1
            switch result {
            case .success(let outcome):
                publishLibraryRowAnalysisUpdate(
                    trackId: trackId, outcome: outcome, refreshLoadedDecks: true)
            case .failure(let err):
                if case LibraryFfiError.GridLocked = err {
                    continue
                }
                surfaceError("Analysis failed for track: \(err.localizedDescription)")
            }
        }
    }

    func loadBrowserSelectionIntoTargetDeck() async {
        guard isRunning else {
            surfaceError("Engine not running.")
            return
        }
        guard let url = librarySelection.browserSelection else {
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
    /// or the deck has no track loaded. Genuine mirror of
    /// `play(side:)`: the engine-mode switch + per-mode rate-reset /
    /// PanicPlay engagement must be identical, otherwise restart
    /// from a non-unity-rate state (a Prep-mode mouse-scratch leaves
    /// the deck at the last scratch velocity) would resume playback
    /// at that velocity instead of 1.0×. The bug pre-fix: this
    /// function called `engine.play` directly without resetting the
    /// rate; after a scratch, the user heard the track at scratch
    /// velocity from 0:00 instead of normal speed. Mirroring `play`
    /// also keeps Timecode-mode semantics consistent — Casual
    /// Restart in a Timecode session should engage Panic for the
    /// same reason a Casual Play does (the TT is disconnected /
    /// stopped; the deck must drive its own playback).
    func restart(side: DeckSide) {
        guard isRunning else { return }
        let deck = state(for: side)
        guard deck.hasTrack else { return }
        do {
            try engine.seek(deckIdx: side.ffiDeckIdx, positionSecs: 0)
            switch engineMode {
            case .prep:
                try engine.setDeckRate(deckIdx: side.ffiDeckIdx, rate: 1.0)
                try engine.play(deckIdx: side.ffiDeckIdx)
                var s = state(for: side)
                s.atEnd = false
                s.isPlaying = true
                setState(s, for: side)
            case .timecode:
                try engine.panicPlay(deckIdx: side.ffiDeckIdx)
                var s = state(for: side)
                s.atEnd = false
                s.isPlaying = true
                s.isPanicPlay = true
                setState(s, for: side)
            }
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
        /// Most recent cursor offset (in audio seconds) reported
        /// by the gesture overlay. Reset to 0 on begin. Used by
        /// the watchdog for stall detection, not by the end path —
        /// scratch end leaves the engine at its naturally coasted
        /// position rather than snapping back to a predicted
        /// cursor offset (avoids the visible release-time jump).
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
            startedAt: Date
        ) {
            self.side = side
            self.priorIsPlaying = priorIsPlaying
            self.engagedPanic = engagedPanic
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
    ///
    /// **Why no seek-back to the gesture cursor.** A pre-existing
    /// "snap to `scratchStartElapsed + lastEventOffsetSecs`" step
    /// produced a visible left-jump at release: during the
    /// watchdog's stall-decay (`scratchRateStallDecay`, ~0.7 per
    /// 16 ms tick) the engine plays out a brief platter-coast
    /// past where the cursor last logically pointed. That coast
    /// is what the user sees and hears, so the natural release
    /// position is wherever the rate integration landed —
    /// snapping back to the cursor's logical position warped the
    /// playhead backward by exactly the coast displacement. The
    /// drift from EMA lag itself is bounded by `τ × rate` (≈ 30 ms
    /// at unity, imperceptible) so the legacy "drift-compensation"
    /// rationale doesn't justify the visible glitch.
    func scratchEnd(side: DeckSide) {
        // Drop from the watchdog map before restoring transport.
        // `Timer.invalidate()` does not suppress a `scratchTick`
        // already queued on the run loop; an in-flight tick that
        // still holds a `ScratchState` must fail the dictionary
        // identity check in `scratchTick` rather than clobber the
        // unity rate we set below ("pitch dropped after scrub").
        guard let ended = scratchStates.removeValue(forKey: side) else { return }

        scratchingDeck = scratchStates.isEmpty ? nil : scratchStates.keys.first
        if scratchStates.isEmpty {
            scratchTimer?.invalidate()
            scratchTimer = nil
        }

        let idx = side.ffiDeckIdx
        do {
            try engine.setDeckRate(deckIdx: idx, rate: 1.0)
        } catch let error as EngineError {
            surfaceError(describe(error))
        } catch {
            surfaceError("Scratch end failed: \(error.localizedDescription)")
        }

        // Resume transport mirroring `play(side:)` / `restart(side:)`.
        // Pre-fix we always called `engine.play` here; in Timecode
        // mode the next `DropoutHoldRate` block paused the deck for
        // lack of a carrier because `isPanicPlay` stayed false.
        // `scratchBegin` engages Panic only in Timecode mode (so the
        // scratch rate sticks); the resume path must stay decoupled
        // the same way. Do not blanket-cancel that panic before
        // resume — `cancelPanic` belongs only on the paused-at-end
        // branch where we tear down the temporary engagement.
        if ended.priorIsPlaying {
            do {
                switch engineMode {
                case .prep:
                    try engine.play(deckIdx: idx)
                    var s = state(for: side)
                    s.isPlaying = true
                    s.isPanicPlay = false
                    setState(s, for: side)
                case .timecode:
                    try engine.panicPlay(deckIdx: idx)
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
                surfaceError("Scratch end failed: \(error.localizedDescription)")
            }
        } else {
            if ended.engagedPanic {
                cancelPanic(side: side)
            }
            pause(side: side)
        }
    }

    private func ensureScratchTimerRunning() {
        if scratchTimer != nil { return }
        // Same MainActor contract as `startPolling()`: the timer
        // fires on `RunLoop.main` (see `.common` below) but the
        // closure is `@Sendable`, so `scratchTick()` needs an
        // explicit isolation expression before it touches
        // `scratchStates` or calls `engine.setDeckRate`.
        let timer = Timer.scheduledTimer(
            withTimeInterval: Self.scratchTickIntervalSecs, repeats: true
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.scratchTick()
            }
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
        let sides = Array(scratchStates.keys)
        for side in sides {
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
            // `scratchEnd` removes the map entry before it sends
            // unity rate; a queued tick may still hold `state`.
            // `!= nil` is insufficient if a new scratch re-used
            // the side — require the same object identity.
            guard scratchStates[side] === state else { continue }
            state.smoothedRate = next
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
            s.seekGeneration &+= 1
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

    /// M11c.3b — re-anchor the master deck's beat grid at the
    /// playhead and optionally halve / double the BPM (`G` /
    /// `Shift+G` / `Option+G`). Persists `user_tap` when the deck
    /// holds a library track.
    func applyTapToGrid(halve: Bool, double: Bool) {
        guard isRunning else { return }
        let side = masterDeck ?? stickyMaster
        var deck = state(for: side)
        guard deck.hasTrack else { return }

        let pos = engine.position(deckIdx: side.ffiDeckIdx)
        let anchor = pos.elapsedSecs

        let currentBpm: Double
        if let bpm = deck.bpm, bpm > 0 {
            currentBpm = bpm
        } else {
            let grid = engine.beatGrid(deckIdx: side.ffiDeckIdx)
            guard grid.confidence > 0, grid.bpm > 0 else { return }
            currentBpm = grid.bpm
        }

        let newBpm: Double
        if halve {
            newBpm = currentBpm / 2.0
        } else if double {
            newBpm = currentBpm * 2.0
        } else {
            newBpm = currentBpm
        }
        guard newBpm.isFinite, newBpm > 0 else { return }

        do {
            try engine.installBeatGrid(
                deckIdx: side.ffiDeckIdx,
                bpm: newBpm,
                anchorSecs: anchor)
        } catch let error as EngineError {
            surfaceError(describe(error))
            return
        } catch {
            surfaceError("Beat grid update failed: \(error.localizedDescription)")
            return
        }

        let postInstallGrid = engine.beatGrid(deckIdx: side.ffiDeckIdx)
        let installedBarPhase = postInstallGrid.barPhase

        if libraryModel.libraryIsOpen, let trackId = deck.loadedLibraryTrackId {
            let library = self.library
            // `[weak self]` lives on the main-actor closure, not on
            // the outer @Sendable Task body. The detached body only
            // touches Sendable locals (`library`, `trackId`,
            // `anchor`, `newBpm`, `installedBarPhase`); reaching
            // back to the actor-isolated instance happens inside
            // `MainActor.run` where weak capture is safe.
            Task.detached(priority: .background) {
                let persistError: String?
                do {
                    try library.upsertUserTapBeatgrid(
                        trackId: trackId,
                        anchorSecs: anchor,
                        bpm: newBpm,
                        barPhase: installedBarPhase)
                    persistError = nil
                } catch {
                    persistError = error.localizedDescription
                }
                await MainActor.run { [weak self] in
                    guard let self else { return }
                    if let persistError {
                        self.surfaceError(
                            "Saved grid on deck but library write failed: \(persistError)")
                    } else {
                        self.libraryModel.analysisGeneration &+= 1
                        self.libraryModel.libraryRowAnalysisUpdate = LibraryRowAnalysisUpdate(
                            trackId: trackId,
                            bpm: newBpm,
                            key: nil,
                            isAnalyzed: true)
                    }
                }
            }
        }

        deck.bpm = newBpm
        deck.bpmConfidence = 1.0
        setState(deck, for: side)
    }

    /// M11d.6 — which deck the manual beatgrid nudge controls
    /// (arrow-key shortcuts + on-screen ◀ ▶ + − buttons) target.
    /// Mirrors `applyTapToGrid`'s rule: prefer the active master
    /// deck, otherwise fall back to the sticky master (deck A by
    /// default). Always returns a side — callers don't have to
    /// branch on `nil` — but `nudgeBeatGrid*` early-out when the
    /// deck holds no track.
    var focusedDeckForGridNudge: DeckSide {
        masterDeck ?? stickyMaster
    }

    /// M11d.6 — manual phase nudge for the focused deck's beat
    /// grid. Persists `user_tap` when the deck holds a library track.
    func nudgeBeatGridPhase(
        _ side: DeckSide,
        deltaSecs: Double,
        tier: BeatgridNudgeTier
    ) {
        guard isRunning else { return }
        var deck = state(for: side)
        guard deck.hasTrack else { return }

        do {
            try engine.nudgeBeatGridPhase(
                deckIdx: side.ffiDeckIdx, deltaSecs: deltaSecs)
        } catch let error as EngineError {
            surfaceError(describe(error))
            return
        } catch {
            surfaceError("Grid nudge failed: \(error.localizedDescription)")
            return
        }

        persistNudgedGrid(
            side: side,
            deck: &deck,
            action: "phase",
            tier: tier,
            delta: deltaSecs)
    }

    /// M11d.6 — manual BPM stretch / shrink for the focused deck's
    /// beat grid.
    func nudgeBeatGridBpm(
        _ side: DeckSide,
        deltaBpm: Double,
        tier: BeatgridNudgeTier
    ) {
        guard isRunning else { return }
        var deck = state(for: side)
        guard deck.hasTrack else { return }

        do {
            try engine.nudgeBeatGridBpm(
                deckIdx: side.ffiDeckIdx, deltaBpm: deltaBpm)
        } catch let error as EngineError {
            surfaceError(describe(error))
            return
        } catch {
            surfaceError("Grid nudge failed: \(error.localizedDescription)")
            return
        }

        persistNudgedGrid(
            side: side,
            deck: &deck,
            action: "bpm",
            tier: tier,
            delta: deltaBpm)
    }

    /// M11d.7 — commit a tap-to-grid window (downbeat or tempo).
    ///
    /// PRD-BEATS §4.2 round 3 + gate 8: the Swift gate inside
    /// `handleTapForGrid` already rejects taps on locked decks
    /// before they reach the controller. This commit hook is a
    /// defense-in-depth backstop for the case where the lock
    /// flipped during an open session (so the buffered taps land
    /// on a now-locked deck). In that case the engine returns
    /// `EngineError.GridLocked`; we treat it as a silent no-op
    /// per PRD-BEATS §13 (no error toast).
    func commitTapGrid(side: DeckSide, playheadTimes: [Double]) {
        guard isRunning, !playheadTimes.isEmpty else { return }
        var deck = state(for: side)
        guard deck.hasTrack else { return }

        let genre = librarySelection.selectedLibraryTrack?.genre
        // PRD-BEATS §4.1, Round 8: 1–2 tap "set the 1" places
        // the downbeat at `tap_secs` bit-exact. The engine does
        // not snap, shift, or refine — the user owns the click
        // coordinate, which is exactly what UI users expect.
        let downbeat = playheadTimes[0]

        do {
            if playheadTimes.count >= 3 {
                try engine.installBeatGridFromTaps(
                    deckIdx: side.ffiDeckIdx,
                    tapTimes: playheadTimes,
                    genre: genre)
            } else {
                try engine.setBarPhase(
                    deckIdx: side.ffiDeckIdx,
                    tapSecs: downbeat,
                    genre: genre)
            }
        } catch EngineError.GridLocked {
            return
        } catch let error as EngineError {
            surfaceError(describe(error))
            return
        } catch {
            surfaceError("Tap-to-grid failed: \(error.localizedDescription)")
            return
        }

        deck = state(for: side)
        let grid = engine.beatGrid(deckIdx: side.ffiDeckIdx)
        guard grid.confidence > 0, grid.bpm > 0 else { return }

        if playheadTimes.count >= 3 {
            BeatgridCalibrationLog.logTapTempoRecalc(
                side: "\(side)",
                trackId: deck.loadedLibraryTrackId,
                path: deck.sourceURL?.path,
                title: deck.trackTitle ?? deck.displayName,
                tapTimes: playheadTimes,
                autoBpm: deck.autoGridBpm,
                autoAnchorSecs: deck.autoGridAnchorSecs,
                resultBpm: grid.bpm,
                resultAnchorSecs: grid.beats.first ?? downbeat,
                durationSecs: deck.durationSecs)
        } else {
            BeatgridCalibrationLog.logTapDownbeat(
                side: "\(side)",
                trackId: deck.loadedLibraryTrackId,
                path: deck.sourceURL?.path,
                title: deck.trackTitle ?? deck.displayName,
                downbeatSecs: downbeat,
                tapCount: playheadTimes.count,
                autoBpm: deck.autoGridBpm,
                autoAnchorSecs: deck.autoGridAnchorSecs,
                resultBpm: grid.bpm,
                resultAnchorSecs: grid.beats.first ?? downbeat)
        }

        persistTapGrid(
            side: side,
            deck: &deck,
            grid: grid,
            playheadTimes: playheadTimes)
    }

    private func persistTapGrid(
        side: DeckSide,
        deck: inout DeckState,
        grid: BeatGrid,
        playheadTimes: [Double]
    ) {
        guard let anchor = grid.beats.first else { return }
        deck.manualGridEditCount += 1
        deck.bpm = grid.bpm
        deck.bpmConfidence = Double(grid.confidence)
        // Paused decks render on-demand (`WaveformView.continuously
        // Rendering = false`), so `WaveformRenderer.refreshBeatGrid
        // IfNeeded` only runs when `MTKView.setNeedsDisplay` fires.
        // SwiftUI invokes that via `updateNSView` whenever
        // `seekGeneration` or `peaksGeneration` change. Without
        // bumping `seekGeneration` here the Rust engine's
        // `bar_phase` rotation lands correctly but the user has
        // to hit Play before the yellow downbeat marker moves —
        // exactly the user-reported "i need to play the deck
        // before the grid change shows" bug. Identical fix to the
        // M11d.6 `persistNudgedGrid` defense; the playing-deck
        // path is unaffected because CVDisplayLink redraws
        // continuously regardless of this counter.
        deck.seekGeneration &+= 1
        setState(deck, for: side)

        guard libraryModel.libraryIsOpen, let trackId = deck.loadedLibraryTrackId else { return }
        let library = self.library
        let quality = grid.quality
        let barPhase = grid.barPhase
        // `[weak self]` on the main-actor closure, not the outer
        // detached body — see ensureTrackAnalyzed comment for the
        // Swift-6 strict-concurrency rationale.
        Task.detached(priority: .background) {
            let persistError: String?
            do {
                try library.upsertUserTapBeatgrid(
                    trackId: trackId,
                    anchorSecs: anchor,
                    bpm: grid.bpm,
                    barPhase: barPhase)
                // PRD-BEATS §3.5 + user-reported regression: user
                // taps must NEVER auto-lock the grid. The auto-lock
                // heuristic is meant for the initial auto-analyse
                // pass — it freezes a tight-LSQ grid that the
                // estimator is confident about. Applying that same
                // heuristic to a user-supplied tap means the user
                // taps once to set the 1, the LSQ refit produces
                // tight residuals (because the grid spacing didn't
                // change), `auto_lock_safe()` returns true, and
                // the deck silently locks itself. The user then
                // can't tap again to refine, can't 2x/½ from the
                // header, can't reset to auto — the only way back
                // is the library context menu. Refresh ONLY the
                // drift indicator so the ⚠ marker stays accurate;
                // the lock flag is left untouched.
                if let quality {
                    try library.setGridDriftQuality(
                        trackId: trackId,
                        driftSlopeMsPerMin: quality.driftSlopeMsPerMin)
                }
                persistError = nil
            } catch {
                persistError = error.localizedDescription
            }
            await MainActor.run { [weak self] in
                guard let self else { return }
                if let persistError {
                    self.surfaceError(
                        "Saved grid on deck but library write failed: \(persistError)")
                } else {
                    self.libraryModel.analysisGeneration &+= 1
                    if let q = quality {
                        var d = self.state(for: side)
                        d.gridDriftQuality = q.driftSlopeMsPerMin
                        // Deliberately do NOT touch `d.gridLocked`
                        // here. See the equivalent comment on the
                        // library-write branch above — the user
                        // explicitly invoked a tap edit, so they
                        // own the lock state.
                        self.setState(d, for: side)
                    }
                    self.libraryModel.libraryRowAnalysisUpdate = LibraryRowAnalysisUpdate(
                        trackId: trackId,
                        bpm: grid.bpm,
                        key: nil,
                        isAnalyzed: true)
                }
            }
        }
    }

    /// Octave-shift the active library beatgrid for the deck loaded
    /// on `side`: multiply BPM by `multiplier` (2.0 for "2×", 0.5
    /// for "½") and re-install the result on the engine while
    /// keeping the visible downbeat anchored at the same musical
    /// position. Backs the deck-header BPM right-click menu.
    ///
    /// Library-only operation: the user_tap row is the source of
    /// truth, so the deck must have a `loadedLibraryTrackId`. If
    /// the track has never been analysed (no active grid), surfaces
    /// a hint asking the user to analyse first. Locked grids are
    /// silently refused — the menu already disables itself in that
    /// case, this guard is defense in depth.
    ///
    /// When the same track is loaded on the other deck we refresh
    /// it too: the library row is shared, so both deck headers must
    /// show the same BPM.
    func scaleLoadedDeckBpm(side: DeckSide, multiplier: Double) {
        guard isRunning, libraryModel.libraryIsOpen else { return }
        let deck = state(for: side)
        guard deck.hasTrack, !deck.gridLocked else { return }
        guard let trackId = deck.loadedLibraryTrackId else {
            surfaceError("Scale BPM: track is not in the library.")
            return
        }
        guard multiplier.isFinite, multiplier > 0 else { return }

        let result: LibraryBeatGrid?
        do {
            result = try library.scaleActiveBeatGrid(
                trackId: trackId,
                multiplier: multiplier)
        } catch {
            surfaceError("Scale BPM failed: \(error.localizedDescription)")
            return
        }
        guard let grid = result else {
            surfaceError("Scale BPM: analyze the track first.")
            return
        }
        installLibraryGridOnLoadedDecks(
            trackId: trackId,
            grid: grid,
            bumpEditCount: true)
    }

    /// Deck-header BPM right-click → "Lock grid" / "Unlock grid".
    /// Mirrors the library row's lock toggle (`setGridLocked`) so
    /// the user can flip the lock without leaving the performance
    /// surface. Necessary because the auto-analyse pass no longer
    /// auto-locks (user feedback: "for now disable auto grid
    /// lock") — without this menu entry there's no performance-
    /// time path to lock a freshly-tuned grid before a gig.
    ///
    /// Idempotent w.r.t. the lock flag itself: reads the current
    /// state and writes its inverse. When the same track is
    /// loaded on the other deck we mirror the new state there too
    /// (same one-row-per-track invariant as `setGridLocked`).
    func toggleLoadedDeckGridLocked(side: DeckSide) {
        let deck = state(for: side)
        guard deck.hasTrack, let trackId = deck.loadedLibraryTrackId else { return }
        let newLocked = !deck.gridLocked
        Task { @MainActor in
            await self.setGridLocked(trackId: trackId, locked: newLocked)
        }
    }

    /// Deck-header BPM context menu "Reset to auto" entry. Drops
    /// the user's manual tap edits and reverts to the auto grid.
    ///
    /// Round 10 contract change: "Reset" now runs a FRESH analysis
    /// (same code path as the library "Reanalyze" right-click
    /// entry), not just a row-flip back to whatever auto grid the
    /// DB happens to have cached. The previous implementation
    /// (`reset_active_beatgrid_to_auto` in `dub-library`) demoted
    /// the active `user_tap` row and re-activated the existing
    /// `auto` row verbatim — but the existing `auto` row might
    /// have been written months ago under a stale algorithm
    /// version (e.g. Round 9's geometric-drift-aware integer-snap
    /// which mis-snapped Chase & Status — Come Back to 175 BPM
    /// before being reverted in Round 10). A user who hit Reset
    /// after that revert would resurrect the stale 175 BPM,
    /// directly contradicting what Round 10 fixed.
    ///
    /// User intent for "Reset" is "throw away my edits and give me
    /// the current-algorithm baseline", which is exactly what
    /// `analyzeTracks([trackId])` delivers. The Rust analyse path
    /// demotes any active `user_tap` row unconditionally and
    /// writes a fresh `auto` row, then `publishLibraryRow
    /// AnalysisUpdate(refreshLoadedDecks: true)` installs it on
    /// the deck without a reload. Same UX surface (footer pill,
    /// row refresh, BPM digit updates immediately) and same
    /// "lock is absolute" gate — the front-end early-out on
    /// `deck.gridLocked` is kept so the operation feels instant
    /// (the Rust side would also refuse with `GridLocked`, but
    /// flashing an error toast for a state the menu already
    /// communicates is hostile).
    func resetLoadedDeckBeatGrid(side: DeckSide) {
        guard isRunning, libraryModel.libraryIsOpen else { return }
        let deck = state(for: side)
        guard deck.hasTrack, !deck.gridLocked else { return }
        guard let trackId = deck.loadedLibraryTrackId else {
            surfaceError("Reset grid: track is not in the library.")
            return
        }
        Task { @MainActor in
            await self.analyzeTracks([trackId])
        }
    }

    /// Push the given library beatgrid onto every loaded deck whose
    /// `loadedLibraryTrackId == trackId`. Mirrors the persist tail
    /// used by `refreshLoadedDecksAfterLibraryAnalysis` so the
    /// waveform redraws on the next vsync (paused decks) and the
    /// deck-header BPM digit updates without a reload. Also
    /// publishes a `LibraryRowAnalysisUpdate` so the LibraryView
    /// row reflects the new BPM immediately.
    private func installLibraryGridOnLoadedDecks(
        trackId: String,
        grid: LibraryBeatGrid,
        bumpEditCount: Bool
    ) {
        guard grid.bpm > 0 else { return }
        for side in [DeckSide.a, DeckSide.b] {
            var deck = state(for: side)
            guard deck.loadedLibraryTrackId == trackId, deck.hasTrack else { continue }
            do {
                try engine.installBeatGridWithPhase(
                    deckIdx: side.ffiDeckIdx,
                    bpm: grid.bpm,
                    anchorSecs: grid.anchorSecs,
                    barPhase: grid.barPhase)
            } catch {
                continue
            }
            deck.bpm = grid.bpm
            deck.bpmConfidence = 1.0
            deck.gridDriftQuality = grid.gridDriftQuality
            deck.gridLocked = grid.gridLocked
            if bumpEditCount {
                deck.manualGridEditCount &+= 1
            }
            // M11d.6 fix mirror: paused decks render on-demand,
            // and the renderer only repaints when seekGeneration /
            // peaksGeneration change. Bumping it here forces an
            // immediate MTKView redraw so the new grid lines up on
            // the same vsync.
            deck.seekGeneration &+= 1
            setState(deck, for: side)
        }
        libraryModel.libraryRowAnalysisUpdate = LibraryRowAnalysisUpdate(
            trackId: trackId,
            bpm: grid.bpm,
            key: nil,
            isAnalyzed: true)
        libraryModel.analysisGeneration &+= 1
    }

    /// Write a finalized calibration record when unloading a track
    /// that received manual grid edits.
    private func finalizeBeatgridSessionIfNeeded(side: DeckSide, deck: DeckState) {
        guard deck.manualGridEditCount > 0 else { return }
        let grid = engine.beatGrid(deckIdx: side.ffiDeckIdx)
        guard grid.confidence > 0,
              grid.bpm > 0,
              let firstBeat = grid.beats.first
        else { return }

        BeatgridCalibrationLog.logFinalized(
            side: "\(side)",
            trackId: deck.loadedLibraryTrackId,
            path: deck.sourceURL?.path,
            title: deck.trackTitle ?? deck.displayName,
            autoBpm: deck.autoGridBpm,
            autoAnchorSecs: deck.autoGridAnchorSecs,
            finalBpm: grid.bpm,
            finalAnchorSecs: Double(firstBeat),
            editCount: deck.manualGridEditCount,
            durationSecs: deck.durationSecs)
    }

    /// Mirror the freshly-nudged grid back into `DeckState`
    /// (BPM column) and persist a `user_tap` row when the deck
    /// holds a library track. Both nudge entry points share this
    /// tail so the on-screen BPM digit refreshes immediately and a
    /// reload picks the nudged grid back up.
    private func persistNudgedGrid(
        side: DeckSide,
        deck: inout DeckState,
        action: String,
        tier: BeatgridNudgeTier,
        delta: Double
    ) {
        let grid = engine.beatGrid(deckIdx: side.ffiDeckIdx)
        guard grid.confidence > 0, grid.bpm > 0, let firstBeat = grid.beats.first
        else { return }
        let newBpm = grid.bpm
        let newAnchor = Double(firstBeat)
        let playhead = engine.position(deckIdx: side.ffiDeckIdx).elapsedSecs

        deck.manualGridEditCount &+= 1
        BeatgridCalibrationLog.logManualAdjust(
            side: "\(side)",
            trackId: deck.loadedLibraryTrackId,
            path: deck.sourceURL?.path,
            title: deck.trackTitle ?? deck.displayName,
            action: action,
            tier: tier,
            delta: delta,
            playheadSecs: playhead,
            autoBpm: deck.autoGridBpm,
            autoAnchorSecs: deck.autoGridAnchorSecs,
            resultBpm: newBpm,
            resultAnchorSecs: newAnchor,
            editIndex: deck.manualGridEditCount)

        // M11d.6 fix — paused decks render on-demand. The renderer
        // already refetches the grid when `beatGridGeneration`
        // changes (bumped by the FFI nudge), but on-demand redraws
        // only fire when `seekGeneration` / `peaksGeneration`
        // change. Without this bump the Rust grid updates
        // correctly but `MTKView.setNeedsDisplay` never runs on a
        // paused deck, so the user sees no visual change. The
        // playing-deck path is unaffected: it already redraws
        // continuously via `CVDisplayLink`.
        deck.seekGeneration &+= 1

        let nudgedBarPhase = grid.barPhase
        if libraryModel.libraryIsOpen, let trackId = deck.loadedLibraryTrackId {
            let library = self.library
            // `[weak self]` on the main-actor closure, not the outer
            // detached body — see ensureTrackAnalyzed comment for the
            // Swift-6 strict-concurrency rationale.
            Task.detached(priority: .background) {
                let persistError: String?
                do {
                    try library.upsertUserTapBeatgrid(
                        trackId: trackId,
                        anchorSecs: newAnchor,
                        bpm: newBpm,
                        barPhase: nudgedBarPhase)
                    persistError = nil
                } catch {
                    persistError = error.localizedDescription
                }
                await MainActor.run { [weak self] in
                    guard let self else { return }
                    if let persistError {
                        self.surfaceError(
                            "Nudged grid on deck but library write failed: \(persistError)")
                    } else {
                        self.libraryModel.analysisGeneration &+= 1
                        self.libraryModel.libraryRowAnalysisUpdate = LibraryRowAnalysisUpdate(
                            trackId: trackId,
                            bpm: newBpm,
                            key: nil,
                            isAnalyzed: true)
                    }
                }
            }
        }

        deck.bpm = newBpm
        deck.bpmConfidence = 1.0
        setState(deck, for: side)
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
        case .NoTrackLoaded:        return "No track loaded on that deck."
        case .InvalidBeatGridParams: return "Invalid beat grid — check BPM and anchor."
        case .GridLocked:           return "Grid is locked — unlock from the library row to edit."
        }
    }
}

// MARK: - Top-level shell

/// Top-level shell: the performance surface plus a `⌘,`-triggered
/// Preferences sheet.
struct MainView: View {

    @StateObject private var model = WaveformAppModel()
    @State private var showingPreferences: Bool = false
    @State private var showingAbout: Bool = false
    @State private var showLaunchSplash: Bool = true

    var body: some View {
        ZStack {
            PerformanceView(
                model: model,
                openPreferences: { showingPreferences = true },
                openAbout: { showingAbout = true })
                .frame(minWidth: 960, minHeight: 600)

            if showLaunchSplash {
                LaunchSplashOverlay()
                    .transition(.opacity)
                    .zIndex(1)
            }

            if showingAbout {
                AboutOverlay(onDismiss: { showingAbout = false })
                    .transition(.opacity)
                    .zIndex(2)
            }
        }
            .sheet(isPresented: $showingPreferences) {
                PreferencesSheet(model: model)
            }
            .background(
                KeyEventMonitorHost(
                    showingPreferences: $showingPreferences,
                    model: model)
            )
            .onReceive(NotificationCenter.default.publisher(for: .dubShowAbout)) { _ in
                showingAbout = true
            }
            .onReceive(NotificationCenter.default.publisher(for: .dubShowPreferences)) { _ in
                showingPreferences = true
            }
            .task {
                await runColdBoot()
            }
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
    }

    /// Cold-boot sequence: spin up engine + library, keep the splash
    /// visible for at least ~900 ms so it doesn't flash subliminally,
    /// then fade out.
    private func runColdBoot() async {
        let minimumSplashNs: UInt64 = 900_000_000
        let bootStart = DispatchTime.now()

        if !model.isRunning {
            model.applyConfig()
        }
        model.openLibraryIfNeeded()

        let elapsed = DispatchTime.now().uptimeNanoseconds - bootStart.uptimeNanoseconds
        if elapsed < minimumSplashNs {
            try? await Task.sleep(nanoseconds: minimumSplashNs - elapsed)
        }

        withAnimation(.easeOut(duration: 0.35)) {
            showLaunchSplash = false
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
///
/// ## SwiftUI ↔ AppKit bridge contract
///
/// **Snapshot props**: `model: WaveformAppModel` — captured once
/// at `makeNSView`; reads always go through this reference, so
/// model identity must not change for the host's lifetime.
///
/// **Binding props**: `showingPreferences: Bool` — flipped by the
/// `⌘,` handler. Bindings are captured by reference, so
/// `updateNSView` is a deliberate no-op (the latest binding is
/// always reachable through the captured `$showingPreferences`).
///
/// **Closure props**: callbacks are wired in `Coordinator.install`
/// (`onSpace`, `onCmdComma`, `onTapGrid`). Each closure spawns
/// `Task { @MainActor in … }` because the NSEvent monitor's
/// handler block has no implicit actor isolation.
///
/// **Lifecycle**: the monitor handle lives on the Coordinator;
/// `dismantleNSView` releases it via `NSEvent.removeMonitor`.
/// Without this, the monitor outlives the SwiftUI view tree and
/// fires against a torn-down model.
///
/// **Event filtering**: the handler returns `nil` (consume event)
/// when it acts, the original event otherwise. Editable text
/// fields keep their keyDown by detecting the first-responder
/// kind before consuming Space (otherwise typing a space in the
/// search field would load a track).
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
            },
            onTapGrid: { halve, double in
                Task { @MainActor in
                    model.applyTapToGrid(halve: halve, double: double)
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
            onCmdComma: @escaping () -> Bool,
            onTapGrid: @escaping (_ halve: Bool, _ double: Bool) -> Bool
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
                if !isCmd, event.charactersIgnoringModifiers?.lowercased() == "g" {
                    let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
                    let halve = flags.contains(.shift)
                    let double = flags.contains(.option)
                    if onTapGrid(halve, double) { return nil }
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
