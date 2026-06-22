//
//  TapToGridController.swift
//  Dub
//
//  PRD-BEATS §4.2 round 3 — deck-header BPM tap window.
//
//  Per-deck tap collector and committer. The controller owns the
//  open tap session: timing, dynamic idle expiry, rolling BPM
//  preview, and the dispatch to set-the-1 (1-2 taps) vs constrained
//  re-analysis (3+ taps).
//
//  Lock and transport gates are enforced by the *caller*
//  (`MainView.handleTapForGrid`) before we ever record a tap — see
//  PRD-BEATS §7 gates 8 and 9. The controller assumes any tap it
//  receives is allowed; it just has to dispatch correctly when the
//  session closes.
//

import Foundation

/// Per-deck published surface for an open tap session.
///
/// Owns the two pieces of UI state — current tap count and rolling
/// BPM preview — that the deck-header BPM column needs to render
/// while a tap-tempo session is in flight. **Deliberately split off
/// from `DeckState`** so a tap during playback does not invalidate
/// `WaveformAppModel.objectWillChange`. Pre-split, tapping the BPM
/// column flipped `deck.tapGridCount` on a `@Published` `DeckState`
/// inside `WaveformAppModel`, which re-evaluated every observer of
/// the app model — `PerformanceView` (rebuilding both
    /// `TrackOverviewView`s, both `WaveformView`s, the FX bar, and the
    /// entire `LibraryView`). The Metal renderer
/// itself was unaffected, but the SwiftUI cascade competed with
/// the render thread for main-actor time and stuttered the
/// waveform on every tap. By moving the two fields to a dedicated
/// per-deck `ObservableObject`, only the `DeckHeader` subscribed
/// to that deck's session invalidates on tap.
///
/// Writes happen exclusively from the per-deck `TapToGridController`
/// callbacks wired in `WaveformAppModel.wireTapToGridControllers()`.
/// `WaveformAppModel` holds one instance per deck as a non-published
/// `let` reference so model identity stays stable across the app
/// lifetime (no SwiftUI re-subscription churn).
@MainActor
final class TapSessionViewModel: ObservableObject {
    /// PRD-BEATS §4.2 — open tap-to-grid window count. `0` when no
    /// session is in flight. Drives the parenthesised count next
    /// to the BPM digit on the deck header.
    @Published var tapCount: Int = 0

    /// PRD-BEATS §4.2 round 3 + gate 12 — rolling BPM preview
    /// while a tap session is open. `nil` outside an active
    /// session or while the session has fewer than 3 taps. When
    /// non-nil, the BPM column renders this value in the
    /// "tapping" treatment (italic + accent colour) instead of
    /// the committed BPM.
    @Published var rollingBpm: Double? = nil

    /// Drop both fields back to "no open session". Cheap and
    /// idempotent; preferred entry point over zeroing the fields
    /// individually so future additions stay in lockstep.
    func reset() {
        if tapCount != 0 { tapCount = 0 }
        if rollingBpm != nil { rollingBpm = nil }
    }
}

/// Per-deck tap buffer for beatgrid correction via the BPM column.
@MainActor
final class TapToGridController {
    private struct Entry {
        let wallTime: Date
        let playheadSecs: Double
    }

    /// Called when the session window closes. Carries the buffered tap
    /// PLAYHEAD positions (used for the grid anchor) plus, for a ≥3-tap
    /// tempo session, the **wall-clock-derived BPM** (`nil` for a 1–2 tap
    /// set-the-1). Tempo MUST come from wall-clock — the same value the
    /// rolling preview shows — never from playhead-position deltas, which
    /// are scaled by the platter's live playback rate (the cause of the
    /// "committed BPM vastly off from the preview" bug).
    var onCommit: ((_ playheads: [Double], _ wallClockBpm: Double?) -> Void)?

    /// Called whenever the open tap count changes (0 = idle).
    var onTapCountChanged: ((Int) -> Void)?

    /// Called when the rolling BPM preview updates. `nil` payload
    /// means "the preview is no longer meaningful" (tap count
    /// dropped below `previewMinTapCount`, or the session ended).
    /// Emitted whenever the tap count advances from 3 onward (and
    /// once on the trailing edge with `nil` to clear the preview).
    /// PRD-BEATS §4.2 + gate 12.
    var onRollingBpmChanged: ((Double?) -> Void)?

    private var entries: [Entry] = []
    private var expiryTask: Task<Void, Never>?

    // PRD-BEATS §4.2 round 3: the fixed 2 s window cuts off slow
    // music. A 70 BPM session has ~857 ms tap intervals — three
    // taps spans 2.57 s before a fourth has any chance to arrive,
    // so the prior 2 s window committed as if the session were
    // closed mid-pattern. The dynamic rule keeps the 150 BPM case
    // snappy while allowing slow tempos to breathe.
    private static let minSessionWindowSecs: Double = 1.5
    /// Tap-window multiplier on the median interval. `1.5×` gives
    /// the user a half-beat of slop after the previous tap before
    /// the session expires — enough to clear short rhythmic
    /// hesitations without making slow sessions feel sticky.
    private static let sessionWindowIntervalMultiplier: Double = 1.5
    private static let maxEntries: Int = 16
    /// Tap count at which the rolling preview becomes meaningful.
    /// 3 taps = 2 intervals; the running median below this count
    /// would be a single noisy interval, so we wait.
    private static let previewMinTapCount: Int = 3

    func tap(playheadSecs: Double) {
        let now = Date()
        if let last = entries.last,
           now.timeIntervalSince(last.wallTime) > currentSessionWindowSecs()
        {
            flushCommit()
            entries.removeAll(keepingCapacity: true)
            onRollingBpmChanged?(nil)
        }

        entries.append(Entry(wallTime: now, playheadSecs: playheadSecs))
        if entries.count > Self.maxEntries {
            entries.removeFirst(entries.count - Self.maxEntries)
        }
        onTapCountChanged?(entries.count)
        publishRollingBpmIfReady()
        scheduleExpiry()
    }

    func cancel() {
        expiryTask?.cancel()
        expiryTask = nil
        entries.removeAll(keepingCapacity: false)
        onTapCountChanged?(0)
        onRollingBpmChanged?(nil)
    }

    /// Forcibly close the open session. Returns the buffered
    /// playhead times. Used when the caller needs to commit *now*
    /// (e.g. user clicked elsewhere); the dynamic-window expiry
    /// task is cancelled and the buffer is cleared.
    func forceCommit() {
        expiryTask?.cancel()
        expiryTask = nil
        flushCommit()
        entries.removeAll(keepingCapacity: true)
        onTapCountChanged?(0)
        onRollingBpmChanged?(nil)
    }

    /// Commit exactly one tap, bypassing the buffered session
    /// entirely. Any open session (from a still-fresh playing-deck
    /// tap that hasn't yet expired) is dropped on the floor first
    /// so the caller's playhead time is the only one dispatched.
    ///
    /// PRD-BEATS §4.2 round 4 follow-up: paused-deck taps reject
    /// the 3+ tap upgrade upstream (`MainView.handleTapForGrid`),
    /// so there is no risk of losing a tap-tempo session by
    /// committing immediately. Using `forceCommit` for this case
    /// is wrong because it flushes the *buffered* session: if the
    /// user paused after a playing-deck tap that's still inside
    /// the 1.5 s idle window, `forceCommit` would dispatch the
    /// stale playing-tap playhead time instead of the fresh paused
    /// tap the user just clicked.
    func commitSingleTap(playheadSecs: Double) {
        expiryTask?.cancel()
        expiryTask = nil
        entries.removeAll(keepingCapacity: true)
        onRollingBpmChanged?(nil)
        onTapCountChanged?(0)
        onCommit?([playheadSecs], nil)
    }

    /// Returns the median tap interval (seconds) over the current
    /// open session, or `nil` if fewer than 2 taps are buffered.
    /// Used internally by the dynamic-window expiry calculation
    /// and exposed for the rolling-preview consumer to compute its
    /// own derived statistics if needed.
    func medianTapIntervalSecs() -> Double? {
        guard entries.count >= 2 else { return nil }
        var deltas: [Double] = []
        deltas.reserveCapacity(entries.count - 1)
        for i in 1..<entries.count {
            let dt = entries[i].wallTime.timeIntervalSince(entries[i - 1].wallTime)
            if dt > 0 {
                deltas.append(dt)
            }
        }
        guard !deltas.isEmpty else { return nil }
        deltas.sort()
        let mid = deltas.count / 2
        if deltas.count.isMultiple(of: 2) {
            return (deltas[mid - 1] + deltas[mid]) * 0.5
        }
        return deltas[mid]
    }

    /// Compute the active idle-expiry budget. `max(1.5 s, 1.5 ×
    /// median tap interval)`. PRD-BEATS §4.2 round 3, gate 10.
    private func currentSessionWindowSecs() -> Double {
        guard let medianInterval = medianTapIntervalSecs() else {
            return Self.minSessionWindowSecs
        }
        let dynamicWindow = Self.sessionWindowIntervalMultiplier * medianInterval
        return max(Self.minSessionWindowSecs, dynamicWindow)
    }

    /// Wall-clock tap tempo (BPM) for the open session, or `nil` if fewer
    /// than two taps / no positive interval. The single source of truth
    /// for committed tempo — identical to the rolling-preview value, so the
    /// committed grid BPM always matches what the DJ saw while tapping.
    /// Derived purely from `Entry.wallTime` deltas, never the playhead.
    func wallClockBpm() -> Double? {
        guard let median = medianTapIntervalSecs(), median > 0 else { return nil }
        return 60.0 / median
    }

    private func publishRollingBpmIfReady() {
        guard entries.count >= Self.previewMinTapCount, let bpm = wallClockBpm() else {
            return
        }
        // Sanity-clamp the preview to the Rust estimator's analyzable
        // MIN_BPM..MAX_BPM range (60..200). Outside it a tap is almost
        // certainly an accidental double-tap or a missed beat, and
        // committing it would build a grid the analyzer can't refine.
        // (Previously 40..240, which let a previewed slow-tempo session
        // commit a value the Rust path then silently dropped.)
        guard bpm >= 60.0, bpm <= 200.0 else {
            onRollingBpmChanged?(nil)
            return
        }
        onRollingBpmChanged?(bpm)
    }

    private func scheduleExpiry() {
        let windowSecs = currentSessionWindowSecs()
        expiryTask?.cancel()
        expiryTask = Task { @MainActor [weak self] in
            let nanos = UInt64(windowSecs * 1_000_000_000)
            try? await Task.sleep(nanoseconds: nanos)
            guard !Task.isCancelled else { return }
            guard let self = self else { return }
            self.flushCommit()
            self.entries.removeAll(keepingCapacity: true)
            self.onTapCountChanged?(0)
            self.onRollingBpmChanged?(nil)
        }
    }

    private func flushCommit() {
        guard !entries.isEmpty else { return }
        let playheads = entries.map(\.playheadSecs)
        // Tempo only for a real ≥3-tap session; 1–2 taps are set-the-1.
        let bpm = entries.count >= Self.previewMinTapCount ? wallClockBpm() : nil
        onCommit?(playheads, bpm)
    }
}
