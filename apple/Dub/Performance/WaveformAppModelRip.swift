//
//  WaveformAppModelRip.swift
//  Dub
//
//  M26a — vinyl-rip lifecycle on `WaveformAppModel` (increment A:
//  manual splits, no auto gap detection, no recognition).
//
//  Flow: Prep mode + "Vinyl recording" toggle + a DJ interface →
//  RIP VINYL starts a Thru engine wired with the deck-0 record tap
//  (`startThruForRip`), creates a `DubRipSession`, and records the
//  side to a spill WAV. STOP tears the Thru engine down, returns to
//  the plain Prep output engine, and loads the spill onto deck A for
//  audition + manual split placement. Encode & Import spawns the
//  Rust commit worker (per-segment FLAC/tag/import) and the UI polls
//  it to done.
//
//  DEV builds only: engaging the Developer mode override
//  (`devForcedMode`) also overrides the DJ-interface requirement —
//  recording falls back to the dev-pinned input or the built-in
//  soundcard (a mono microphone records duplicated to both
//  channels), so the whole rip flow can be dogfooded on a bare
//  MacBook. Release keeps recording DJ-interface-only (PRD §5.2.7).
//
//  Stored properties live in `MainView.swift` (extensions can't add
//  storage); everything here runs on the main actor.
//

import Foundation
import SwiftUI

import DubCore

/// Everything `startThruForRip` needs, resolved up front so the
/// interface and dev-fallback paths share one open/create/start
/// sequence.
private struct RipCaptureRoute {
    let deviceName: String
    let channels: [UInt32]
    let outputUid: String?
}

/// UI-facing rip lifecycle phase. Coarser than the FFI's `RipPhase`
/// (armed/recording collapse into `.capture`; `stopped` becomes
/// `.review` once the spill is loaded for audition). `.failed`
/// covers both a capture failure and a failed commit — `ripStatus.
/// error` says which, and `ripSegments.isEmpty` distinguishes
/// "nothing recorded" from "retry the encode".
enum RipUiPhase: Equatable {
    case none
    case capture
    case review
    case encoding
    case done
    case failed
}

/// Value snapshot of `DubRipSession.status()` for the chrome —
/// published at 10 Hz while a session exists. Pure value type so the
/// rip views stay snapshot-testable without FFI calls.
struct RipUiStatus: Equatable {
    var phase: RipPhase
    var stopReason: RipStopReason
    var elapsedSecs: Double
    var levelPeak: Float
    var error: String?
    /// Where the side begins and ends inside the capture. Carried on
    /// the polled snapshot — and therefore in the `Equatable` the poll
    /// diffs — so moving a trim repaints the review overlay.
    var sideStartSecs: Double
    var sideEndSecs: Double

    init(_ status: RipSessionStatus) {
        phase = status.phase
        stopReason = status.stopReason
        elapsedSecs = status.elapsedSecs
        levelPeak = status.levelPeak
        error = status.error
        sideStartSecs = status.sideStartSecs
        sideEndSecs = status.sideEndSecs
    }
}

extension WaveformAppModel {

    /// Hard recording cap passed to every session: 40 min — beyond
    /// any vinyl side; a forgotten needle in the runout groove must
    /// not fill the disk (see `RipSessionConfig`).
    private static var ripMaxDurationSecs: Double { 2400 }

    private static var ripPollIntervalSecs: TimeInterval { 1.0 / 10.0 }

    /// Whether the RIP VINYL affordance is available right now:
    /// Prep mode, feature toggle on, and a DJ interface present —
    /// or, in DEV builds, the mode override engaged. Deliberately
    /// cheap: SwiftUI evaluates this on every body pass (the deck
    /// poll invalidates at up to 30 Hz), so the actual HAL input
    /// enumeration is deferred to the record click
    /// (`ripCaptureRoute`), which surfaces an error if no input
    /// exists after all.
    var canStartRipCapture: Bool {
        engineMode == .prep
            && vinylRecordingEnabled
            && ripSession == nil
            && (!performanceDevices.isEmpty || ripDevOverrideEngaged)
    }

    /// DEV builds: true while the Developer mode override is engaged
    /// — the switch that also unlocks built-in-soundcard recording.
    /// Always false in Release.
    var ripDevOverrideEngaged: Bool {
        #if DEBUG
        return devForcedMode != nil
        #else
        return false
        #endif
    }

    /// DEV-only built-in-soundcard fallback. When the Developer mode
    /// override is engaged (the same override that lets the
    /// performance UI run without a DJ interface), recording may use
    /// any raw HAL input: the system-default input first (the
    /// built-in microphone on a bare MacBook), else the first one
    /// reported. Uses `listRawInputDevices` because the classified
    /// list deliberately hides everything that isn't DJ-grade.
    /// Release builds always return `nil` — production recording
    /// stays DJ-interface-only (PRD §5.2.7).
    var ripDevInputFallback: RawInputDevice? {
        guard ripDevOverrideEngaged, performanceDevices.isEmpty else { return nil }
        let inputs = engine.listRawInputDevices().filter { $0.channels >= 1 }
        return inputs.first(where: { $0.isDefault }) ?? inputs.first
    }

    /// Resolved capture route: which device to open, which input
    /// pair to record, where the master goes. DJ interface → deck
    /// A's registry pair + master back through the interface. Dev
    /// fallback → plain first pair (a mono microphone duplicates its
    /// channel to both slots) + the dev-pinned or default output.
    private func ripCaptureRoute() -> RipCaptureRoute? {
        if let device = selectedInputDevice ?? performanceDevices.first {
            return RipCaptureRoute(
                deviceName: device.name,
                channels: engine.performanceRoutingFor(deviceName: device.name).deckAInput,
                outputUid: device.uid)
        }
        if let raw = ripDevInputFallback {
            return RipCaptureRoute(
                deviceName: raw.name,
                channels: raw.channels >= 2 ? [1, 2] : [1, 1],
                outputUid: selectedOutputDevice?.uid)
        }
        return nil
    }

    // MARK: Lifecycle

    /// RIP VINYL. Requests mic permission (same Serato-style gate as
    /// Performance mode), swaps the Prep output engine for a Thru
    /// engine with the deck-0 record tap wired, arms + starts a rip
    /// session, and flips the UI into `.capture`.
    func startRipCapture() {
        guard canStartRipCapture else { return }
        guard let route = ripCaptureRoute() else {
            surfaceError(
                ripDevOverrideEngaged
                    ? "No input device found — the Mac reports no usable audio input."
                    : "No DJ interface detected — connect one to record vinyl.")
            return
        }
        ensureInputPermission { [weak self] granted in
            guard let self else { return }
            guard granted else {
                self.surfaceError(
                    "Microphone access is required to record vinyl. "
                        + "Enable Dub under System Settings > Privacy & Security > Microphone.")
                return
            }
            self.openRipCapture(route: route)
        }
    }

    /// Open the Thru-for-rip engine on the resolved route, then
    /// create + start the session. Any failure falls back to the
    /// plain Prep engine so the DJ is never left silent.
    private func openRipCapture(route: RipCaptureRoute) {
        stop()
        do {
            try engine.startThruForRip(
                deviceName: route.deviceName,
                channels: route.channels,
                outputDeviceUid: route.outputUid)
            markEngineStartedForRipCapture()
            let session = try engine.createRipSession(
                deckIdx: 0,
                config: RipSessionConfig(
                    destDir: nil,
                    maxDurationSecs: Self.ripMaxDurationSecs,
                    autoStart: true,
                    autoStop: true))
            // M26b: RIP VINYL arms; the needle starts it. The worker
            // keeps a 1 s pre-roll while armed, so the drop lands on
            // the spill rather than being clipped off the front — and
            // the side ends itself in the run-out groove.
            ripSession = session
            ripLastGeneration = session.generation()
            ripSplits = []
            ripSegments = []
            ripJobs = nil
            ripStatus = RipUiStatus(session.status())
            ripPhase = .capture
            startRipPolling()
        } catch {
            surfaceError("Vinyl recording failed to start: \(Self.describeRip(error))")
            clearRipState()
            stop()
            startPrep()
        }
    }

    /// STOP. Asks the capture worker to finish the spill; the poll
    /// observes the `stopped` phase and runs the review transition
    /// (same path the max-duration cap and an input loss take).
    func stopRip() {
        guard let session = ripSession else { return }
        do {
            try session.stop()
        } catch {
            surfaceError("Couldn't stop the recording: \(Self.describeRip(error))")
        }
    }

    /// Encode & Import (also Retry — the FFI call is idempotent over
    /// already-imported segments). Pauses any audition playback and
    /// hands the session to the commit worker; the poll drives
    /// `.encoding` → `.done` / `.failed`.
    func confirmRip() {
        guard let session = ripSession else { return }
        guard ripPhase == .review || ripPhase == .failed else { return }
        ripAuditionTask?.cancel()
        if deckA.isPlaying { pause(side: .a) }
        do {
            try session.confirmEncodeAndImport(library: library)
            ripJobs = session.jobProgress()
            ripPhase = .encoding
        } catch {
            surfaceError("Encode failed to start: \(Self.describeRip(error))")
        }
    }

    /// Cancel / Discard. The destructive confirm lives in the views
    /// (two-step armed button) — by the time this runs the operator
    /// has confirmed. Cancels the session (which removes the session
    /// directory), unloads the spill if deck A is auditioning it
    /// (via a clean Prep restart), and returns to normal Prep.
    func cancelRip() {
        guard let session = ripSession else { return }
        ripAuditionTask?.cancel()
        let wasCapturing = ripPhase == .capture
        let deckHoldsSpill = deckA.sourceURL?.path
            .hasPrefix(session.sessionDir()) ?? false
        do {
            try session.cancel()
        } catch {
            surfaceError("Couldn't discard the recording: \(Self.describeRip(error))")
        }
        clearRipState()
        if wasCapturing || deckHoldsSpill {
            stop()
            startPrep()
        }
    }

    /// Dismiss a finished (`.done`) or capture-failed rip without
    /// touching deck A. Used by the bar's dismiss affordance and the
    /// done-state auto-clear.
    func dismissRip() {
        guard let session = ripSession else {
            clearRipState()
            return
        }
        if ripPhase == .failed {
            // A failed session holds no imported data worth keeping;
            // cancelling removes the session dir.
            try? session.cancel()
        }
        clearRipState()
    }

    private func clearRipState() {
        stopRipPolling()
        ripDoneClearTask?.cancel()
        ripDoneClearTask = nil
        ripAuditionTask?.cancel()
        ripAuditionTask = nil
        ripSession = nil
        ripStatus = nil
        ripSplits = []
        ripSegments = []
        ripJobs = nil
        ripLastGeneration = 0
        ripPhase = .none
    }

    // MARK: Poll

    func startRipPolling() {
        stopRipPolling()
        let timer = Timer.scheduledTimer(
            withTimeInterval: Self.ripPollIntervalSecs, repeats: true
        ) { [weak self] _ in
            // Scheduled on the main runloop below — same
            // `assumeIsolated` contract as the deck poll.
            MainActor.assumeIsolated {
                self?.ripPollTick()
            }
        }
        timer.tolerance = Self.ripPollIntervalSecs * 0.25
        RunLoop.main.add(timer, forMode: .common)
        ripPollTimer = timer
    }

    func stopRipPolling() {
        ripPollTimer?.invalidate()
        ripPollTimer = nil
    }

    private func ripPollTick() {
        guard let session = ripSession else {
            stopRipPolling()
            return
        }
        let status = session.status()
        let ui = RipUiStatus(status)
        if ripStatus != ui { ripStatus = ui }

        let gen = session.generation()
        if gen != ripLastGeneration {
            ripLastGeneration = gen
            ripSplits = session.splitMarkers()
            ripSegments = session.segments()
        }

        switch status.phase {
        case .idle, .armed:
            break
        case .recording:
            if ripPhase != .capture { ripPhase = .capture }
        case .stopped:
            // Covers manual stop, the max-duration cap, input loss
            // (interface unplug / mode switch), and the M26b run-out
            // auto-stop — the spill up to the stop is intact in all
            // of them.
            if ripPhase == .capture {
                if status.recordedFrames == 0 {
                    // Cancelled while armed: the needle never landed,
                    // so there is nothing to review. Discarding beats
                    // dropping the operator into an empty review
                    // screen with a zero-length side.
                    cancelRip()
                } else {
                    finishCaptureIntoReview(session: session)
                }
            }
        case .encoding:
            ripJobs = session.jobProgress()
            if ripPhase != .encoding { ripPhase = .encoding }
        case .done:
            ripJobs = session.jobProgress()
            if ripPhase != .done { ripCompleted() }
        case .failed:
            ripJobs = session.jobProgress()
            if ripPhase != .failed { handleRipFailure() }
        }
    }

    /// Capture finished → tear down the Thru engine, restart the
    /// plain Prep output engine, and load the spill WAV onto deck A
    /// for audition + split placement.
    private func finishCaptureIntoReview(session: DubRipSession) {
        stop()
        startPrep()
        ripPhase = .review
        ripSplits = session.splitMarkers()
        ripSegments = session.segments()
        ripLastGeneration = session.generation()
        let spill = URL(fileURLWithPath: session.sessionDir())
            .appendingPathComponent("side.raw.wav")
        Task { @MainActor [weak self] in
            _ = await self?.loadTrack(side: .a, url: spill)
        }
    }

    private func ripCompleted() {
        ripPhase = .done
        // Surface the imported tracks in the browser: the track-count
        // bump triggers LibraryView's listing refetch, and the
        // analysis generation covers rows that were already listed.
        refreshLibraryStats()
        refreshMissingTrackCount()
        libraryModel.analysisGeneration &+= 1
        // Auto-clear the confirmation after a beat; dismiss also works.
        ripDoneClearTask?.cancel()
        ripDoneClearTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            guard let self, !Task.isCancelled else { return }
            if self.ripPhase == .done { self.dismissRip() }
        }
    }

    /// A capture failure leaves the Thru engine potentially dead;
    /// fall back to the Prep output engine so the DJ isn't stranded
    /// silent. An encode failure keeps the review data (segments +
    /// spill on deck A) so Retry can re-run the commit.
    private func handleRipFailure() {
        let failedDuringCapture = ripPhase == .capture
        ripPhase = .failed
        if failedDuringCapture {
            stop()
            startPrep()
        }
    }

    // MARK: Split edits (review phase)

    /// Add a split at `secs`. Returns `false` when the FFI rejects
    /// the boundary (out of range / segment under the 5 s minimum /
    /// duplicate) so the overlay can flash.
    @discardableResult
    func addRipSplit(atSecs secs: Double) -> Bool {
        guard let session = ripSession else { return false }
        do {
            _ = try session.addSplit(secs: secs)
            refreshRipSplits(session)
            return true
        } catch {
            return false
        }
    }

    /// Add a split at deck A's current playhead (review-panel header
    /// button). Flashes the status strip on rejection since the
    /// button has no band to flash.
    func addRipSplitAtPlayhead() {
        let secs = engine.positionSnapshot(deckIdx: 0).elapsedSecs
        if !addRipSplit(atSecs: secs) {
            surfaceError("Can't split there — segments need at least 5 seconds.")
        }
    }

    // MARK: Recovery (M26b)

    /// Look for unfinished rips. Called on Prep entry: the spill
    /// outlives a crash by design, and commit deletes it only once
    /// every segment imported, so anything still holding one is work
    /// the operator never got to finish.
    func refreshRecoverableRips() {
        guard vinylRecordingEnabled, ripSession == nil else {
            ripRecoverable = []
            return
        }
        ripRecoverable = engine.listRecoverableRipSessions(ripsDir: nil)
    }

    /// Reopen an unfinished rip straight into review. No engine, no
    /// record tap, no Thru session — nothing more will be recorded
    /// into it, so this works from plain Prep.
    func resumeRip(_ recoverable: RipRecoverable) {
        guard ripSession == nil else { return }
        do {
            let session = try engine.resumeRipSession(sessionDir: recoverable.sessionDir)
            ripSession = session
            ripLastGeneration = session.generation()
            ripSplits = session.splitMarkers()
            ripSegments = session.segments()
            ripJobs = nil
            ripStatus = RipUiStatus(session.status())
            ripPhase = .review
            ripRecoverable = []
            startRipPolling()
            // Audition needs the side on deck A, exactly as it is
            // after a live capture.
            let side = Self.ripSideAudioURL(sessionDir: session.sessionDir())
            Task { @MainActor [weak self] in
                _ = await self?.loadTrack(side: .a, url: side)
            }
        } catch {
            surfaceError("Couldn't reopen that rip: \(Self.describeRip(error))")
            ripRecoverable = []
        }
    }

    /// Reopen a committed rip for a fresh split (M26b, R-44).
    ///
    /// Near-identical to `resumeRip`, with one difference that matters:
    /// the audition file is the **archive**, not the spill. Commit
    /// deletes the spill once every segment has imported, so a
    /// committed session has only `side.flac` — which is the whole
    /// capture, lead-in and run-out included, so a re-split can reach
    /// back past the previous split's trims.
    func resplitRip(sessionDir: String) {
        guard ripSession == nil, engineMode == .prep else { return }
        do {
            let session = try engine.resplitRipSession(sessionDir: sessionDir)
            ripSession = session
            ripLastGeneration = session.generation()
            ripSplits = session.splitMarkers()
            ripSegments = session.segments()
            ripJobs = nil
            ripStatus = RipUiStatus(session.status())
            ripPhase = .review
            ripRecoverable = []
            startRipPolling()
            let side = Self.ripSideAudioURL(sessionDir: session.sessionDir())
            Task { @MainActor [weak self] in
                _ = await self?.loadTrack(side: .a, url: side)
            }
        } catch {
            surfaceError("Couldn't reopen that rip: \(Self.describeRip(error))")
        }
    }

    /// The side's audio for auditioning: the spill while it still
    /// exists, else the archive. One rule, used by every path that
    /// puts a rip on deck A — a committed session has no spill, and a
    /// live one has no archive until commit writes it.
    static func ripSideAudioURL(sessionDir: String) -> URL {
        let dir = URL(fileURLWithPath: sessionDir)
        let spill = dir.appendingPathComponent("side.raw.wav")
        if FileManager.default.fileExists(atPath: spill.path) { return spill }
        return dir.appendingPathComponent("side.flac")
    }

    /// Reload the committed-rip list behind the "Real Records" node.
    ///
    /// Detached, unlike `refreshRecoverableRips`: that one only ever
    /// sees unfinished sessions (a handful), while this list grows
    /// with every record ever ripped and stats a directory per entry.
    func refreshPastRips() {
        libraryModel.pastRipsLoading = true
        let engine = self.engine
        Task.detached(priority: .userInitiated) { [weak self] in
            let found = engine.listResplittableRipSessions(ripsDir: nil)
            let rows = found.map {
                RipPastSessionUi(
                    sessionDir: $0.sessionDir,
                    name: $0.name,
                    recordedSecs: $0.recordedSecs,
                    trackCount: $0.trackCount,
                    splitGeneration: $0.splitGeneration)
            }
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.libraryModel.pastRips = rows
                self.libraryModel.pastRipsLoading = false
            }
        }
    }

    /// Leave an unfinished rip on disk but stop offering it this
    /// session. Deliberately non-destructive: the audio is
    /// irreplaceable without setting the needle back down, so nothing
    /// here deletes it.
    func dismissRecoverableRips() {
        ripRecoverable = []
    }

    /// Replace the plan with auto-detected gaps (review-panel
    /// button). Reports what happened in the status strip: an empty
    /// result is a legitimate answer — a continuous mix side, or a
    /// pressing whose gaps are buried in surface noise — and the
    /// operator still has the markers by hand.
    func autoSplitRip() {
        guard let session = ripSession else { return }
        do {
            let count = try session.autoSplit()
            refreshRipSplits(session)
            if count == 0 {
                surfaceError("No track gaps found — place splits by hand.")
            }
        } catch {
            surfaceError("Couldn't auto-split: \(Self.describeRip(error))")
        }
    }

    /// Move split `id` to `secs`. Non-final drag moves are throttled
    /// by the overlay; the model just forwards. Returns `false` on
    /// rejection (the overlay's local echo snaps back on the next
    /// generation refetch).
    /// Move where the side starts — the end of the discarded lead-in.
    /// `false` when the FFI refuses (the trim would swallow a split or
    /// leave a segment under the minimum).
    @discardableResult
    func moveRipSideStart(toSecs secs: Double) -> Bool {
        guard let session = ripSession else { return false }
        do {
            try session.setSideStart(secs: secs)
            refreshRipSplits(session)
            return true
        } catch {
            return false
        }
    }

    /// Move where the side ends — the start of the discarded run-out.
    @discardableResult
    func moveRipSideEnd(toSecs secs: Double) -> Bool {
        guard let session = ripSession else { return false }
        do {
            try session.setSideEnd(secs: secs)
            refreshRipSplits(session)
            return true
        } catch {
            return false
        }
    }

    @discardableResult
    func moveRipSplit(id: UInt32, toSecs secs: Double) -> Bool {
        guard let session = ripSession else { return false }
        do {
            try session.moveSplit(id: id, secs: secs)
            refreshRipSplits(session)
            return true
        } catch {
            return false
        }
    }

    func removeRipSplit(id: UInt32) {
        guard let session = ripSession else { return }
        do {
            try session.removeSplit(id: id)
            refreshRipSplits(session)
        } catch {
            surfaceError("Couldn't remove the split: \(Self.describeRip(error))")
        }
    }

    /// Stamp per-segment metadata (review-panel card fields, debounced
    /// by the card). Empty strings arrive as `nil` — the FFI treats
    /// `nil` as "unset".
    func setRipSegmentMetadata(
        index: UInt32,
        title: String?,
        artist: String?,
        album: String?,
        genre: String?,
        year: Int32?
    ) {
        guard let session = ripSession else { return }
        do {
            try session.setSegmentMetadata(
                index: index, title: title, artist: artist,
                album: album, genre: genre, year: year)
            refreshRipSplits(session)
        } catch {
            surfaceError("Couldn't save track info: \(Self.describeRip(error))")
        }
    }

    /// Immediate refetch after a successful local edit — snappier
    /// than waiting for the next 10 Hz generation diff.
    private func refreshRipSplits(_ session: DubRipSession) {
        ripSplits = session.splitMarkers()
        ripSegments = session.segments()
        ripLastGeneration = session.generation()
        // The side's bounds ride on the status, so a trim edit has to
        // pull it too — otherwise the shaded region lags a poll tick
        // behind the bracket that moved it.
        ripStatus = RipUiStatus(session.status())
    }

    // MARK: Audition

    /// Audition the spill from `fromSecs`: seek + play + auto-pause
    /// after 6 s. Re-triggering cancels the previous auto-pause.
    /// Marker double-clicks pass `secs − 3` so the boundary sits in
    /// the middle of the window; segment cards pass their edges.
    func ripAudition(fromSecs: Double) {
        guard engineMode == .prep, deckA.hasTrack else { return }
        ripAuditionTask?.cancel()
        seekDeck(side: .a, absoluteSecs: max(0, fromSecs))
        if !deckA.isPlaying { play(side: .a) }
        ripAuditionTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 6_000_000_000)
            guard let self, !Task.isCancelled else { return }
            self.pause(side: .a)
        }
    }

    // MARK: Error text

    /// Human text for `RipFfiError`s (whose default
    /// `localizedDescription` is the reflected enum dump).
    static func describeRip(_ error: Error) -> String {
        if let rip = error as? RipFfiError {
            switch rip {
            case .NotCapturing(let m), .InvalidConfig(let m),
                 .InvalidState(let m), .InvalidSplit(let m),
                 .InvalidSegment(let m), .CaptureFailed(let m),
                 .WriteFailed(let m), .ImportFailed(let m):
                return m
            }
        }
        if let engineError = error as? EngineError {
            return engineError.localizedDescription
        }
        return error.localizedDescription
    }
}
