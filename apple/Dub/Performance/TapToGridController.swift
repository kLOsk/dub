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

/// Per-deck tap buffer for beatgrid correction via the BPM column.
@MainActor
final class TapToGridController {
    private struct Entry {
        let wallTime: Date
        let playheadSecs: Double
    }

    /// Called when the session window closes. Playhead times only,
    /// in deck order.
    var onCommit: (([Double]) -> Void)?

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
        onCommit?([playheadSecs])
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

    private func publishRollingBpmIfReady() {
        guard entries.count >= Self.previewMinTapCount,
              let medianInterval = medianTapIntervalSecs(),
              medianInterval > 0
        else {
            return
        }
        let bpm = 60.0 / medianInterval
        // Sanity-clamp the preview to the same MIN/MAX BPM
        // bounds the Rust estimator enforces. Outside this range
        // a tap is almost certainly an accidental double-tap or
        // a missed beat; surfacing 1500 BPM in the preview is
        // worse than no preview at all.
        guard bpm >= 40.0, bpm <= 240.0 else {
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
        onCommit?(playheads)
    }
}
