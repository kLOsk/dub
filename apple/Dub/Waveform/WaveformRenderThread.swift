//
//  WaveformRenderThread.swift
//  Dub
//
//  M11d.6 round 5 (Phase 3 of the off-main-thread waveform refactor).
//
//  Owns the per-deck `CVDisplayLink` + a dedicated serial
//  `DispatchQueue` (QoS `.userInteractive`). The display link's
//  output callback posts `drawIfNeeded()` to the queue per vsync,
//  but only if no draw is currently pending or in flight
//  (M11d.6 round 9 coalescing gate; see the `drawInFlight` field
//  on `Flags`). The queue runs strictly serially, and a slow frame
//  drops the next vsync rather than queueing a back-to-back paint
//  behind itself — uniform frame cadence is what the eye reads as
//  smooth motion, not 60 fps.
//
//  Lifecycle contract:
//  - `init` creates an inactive thread. No display link is started
//    until `setContinuous(true)` or `requestOneShot()` lands.
//  - `setContinuous(_:)` is idempotent. Repeated true/false calls
//    only allocate / tear down the `CVDisplayLink` on transitions.
//  - `requestOneShot()` is safe to call any time. If the link is
//    already running it's a near-no-op (one extra serial draw
//    queued); if the link is stopped, the queue runs a single
//    `drawIfNeeded` even with `isContinuous = false`.
//  - `setCurrentDisplay(_:)` retargets the active link to a new
//    `CGDirectDisplayID`. Called by `WaveformMetalHostView` via
//    its `onDisplayChange` closure on window migration.
//  - `shutdown()` is the cleanup hook. It stops the link, marks
//    shutdown, and drains the queue with `sync { }` so no
//    in-flight draw can still hold a reference to the renderer
//    after the call returns. Idempotent.
//
//  Concurrency model:
//  - The C callback is non-isolated; it captures `self` via an
//    unowned `Unmanaged<WaveformRenderThread>` raw pointer, balanced
//    by an explicit retain in `start()` and release in `stop()`.
//    This avoids a strong cycle between the link and the thread.
//  - Atomic flags (`isContinuous`, `oneShotRequested`,
//    `shutdownRequested`) are wrapped in `OSAllocatedUnfairLock`-
//    protected scalars to avoid pulling in `swift-atomics`. Reads
//    and writes happen at most ~2× per vsync (~120 Hz) which is
//    well within unfair-lock contention budgets.
//

import AppKit
import CoreVideo
import Foundation
import QuartzCore
import os

/// One render thread per waveform Metal view. The Coordinator on
/// `WaveformMetalView` (Phase 5) owns the instance; the
/// `WaveformRenderer` it draws via is also owned by that
/// Coordinator so we can shut both down together in
/// `dismantleNSView`.
final class WaveformRenderThread: @unchecked Sendable {

    // MARK: - Inputs

    /// Metal layer the renderer draws into. Owned by
    /// `WaveformMetalHostView`; this thread only writes via
    /// `nextDrawable()`.
    let metalLayer: CAMetalLayer
    /// The renderer that knows how to translate engine state into
    /// Metal draw calls. Phase 4 makes this `@unchecked Sendable`;
    /// Phase 3 ships the lifecycle scaffolding and treats the
    /// renderer as an opaque "drawable into a layer" surface.
    private let renderer: WaveformRenderer
    /// M11d.6 round 11 — converts the CVDisplayLink callback's
    /// predicted vsync `CVTimeStamp.hostTime` (mach_absolute_time
    /// ticks) into the engine's `host_time_ns` domain so the
    /// renderer can pass it to
    /// `DubEngine.positionSnapshotAtHostTime`. Owned by the
    /// thread; lock-free, allocation-free reads on the render
    /// queue. See `EngineHostTimeMapping` for the clock-domain
    /// derivation.
    private let hostTimeMapping: EngineHostTimeMapping
    /// Dedicated serial queue for all draws. QoS `.userInteractive`
    /// matches the audio thread's priority class without competing
    /// for the same scheduling slot — Metal's GPU command buffer
    /// submission is the only thing on this queue.
    private let renderQueue: DispatchQueue

    // MARK: - State

    private struct Flags {
        var isContinuous: Bool = false
        var oneShotRequested: Bool = false
        var shutdownRequested: Bool = false
        /// True while a `drawIfNeeded` block has been posted to
        /// `renderQueue` but hasn't started executing yet (or is
        /// executing now). M11d.6 round 9 — used by the
        /// CVDisplayLink callback to **coalesce** vsync ticks: if
        /// a previous draw is still in flight, drop the new vsync
        /// rather than queue another draw behind it. Without this,
        /// a single slow frame produces a burst pair (the slow
        /// draw finishes, the queued draw fires within ~1 ms, the
        /// next real vsync fires ~16 ms later) that the eye reads
        /// as 1-pixel grid jitter. Dropping a vsync gives one
        /// long ~33 ms gap instead, which is invisible — uniform
        /// motion is what matters, not 60 fps.
        var drawInFlight: Bool = false
    }
    private let flagsLock = OSAllocatedUnfairLock(initialState: Flags())

    /// `CVDisplayLink` instance. `nil` while continuous mode is
    /// off (so a paused deck pays zero recurring cost).
    private var displayLink: CVDisplayLink?
    /// `Unmanaged` retain balancing the unowned pointer we pass
    /// into the `CVDisplayLink` C callback. Cleared on `stop()`
    /// alongside the actual link teardown.
    private var displayLinkRetain: Unmanaged<WaveformRenderThread>?

    /// Tagged signpost log for Instruments. The hot path doesn't
    /// emit any signposts in release builds (we never call
    /// `signpost*` on the log); this is reserved for future
    /// Instruments wiring and keeps the type cheap to construct.
    private let log = OSLog(
        subsystem: "com.klos.dub.waveform", category: "RenderThread")

    // MARK: - Construction

    init(
        metalLayer: CAMetalLayer,
        renderer: WaveformRenderer,
        hostTimeMapping: EngineHostTimeMapping,
        label: String
    ) {
        self.metalLayer = metalLayer
        self.renderer = renderer
        self.hostTimeMapping = hostTimeMapping
        self.renderQueue = DispatchQueue(
            label: label,
            qos: .userInteractive,
            attributes: [],
            autoreleaseFrequency: .workItem,
            target: nil)
    }

    deinit {
        // Defensive: the host should have called `shutdown()`
        // explicitly via `dismantleNSView`. If they didn't, tear
        // the link down here. We can't call `shutdown()` because
        // it does a `renderQueue.sync` and a destructor that
        // blocks on a queue can deadlock if `deinit` is itself
        // running on that queue. Just stop the link non-blocking.
        if let displayLink {
            CVDisplayLinkStop(displayLink)
        }
        if let displayLinkRetain {
            displayLinkRetain.release()
        }
    }

    // MARK: - Public API

    /// Toggle continuous-mode rendering. `true` starts the
    /// `CVDisplayLink` and posts an immediate draw to cover the
    /// window between "I want to draw" and "first vsync arrives".
    /// `false` stops the link and lets the renderer go idle.
    func setContinuous(_ on: Bool) {
        // Acquire the lock, record intent, decide whether we
        // need to start or stop the link. We release the lock
        // before touching the `CVDisplayLink` itself to keep the
        // critical section short and to allow the link's C
        // callback (which acquires the same lock to read
        // `shutdownRequested`) to make progress.
        let needsStart: Bool
        let needsStop: Bool
        flagsLock.withLock { state in
            if state.isContinuous == on { return }
            state.isContinuous = on
        }
        // Reload the latest state outside the closure-by-ref
        // dance.
        let snapshot = flagsLock.withLock { $0 }
        needsStart = snapshot.isContinuous && displayLink == nil
        needsStop = !snapshot.isContinuous && displayLink != nil

        if needsStart {
            startDisplayLink()
            // Cover the gap between "link started" and "first
            // vsync fires" with a single warm-up draw, but go
            // through the same coalescing path the
            // `requestOneShot` API uses so an immediate first
            // vsync can't pile up behind this block.
            let shouldWarmUp: Bool = flagsLock.withLock { state in
                if state.shutdownRequested { return false }
                if state.drawInFlight { return false }
                state.drawInFlight = true
                return true
            }
            if shouldWarmUp {
                scheduleOneShot()
            }
        } else if needsStop {
            stopDisplayLink()
        }
    }

    /// Request a single draw on the render thread. Intended for
    /// generation bumps (palette change, peaks rebuild, beat-grid
    /// install) that need to repaint a paused deck once. Safe to
    /// call from any thread; the work always lands on
    /// `renderQueue`.
    func requestOneShot() {
        let shouldSchedule: Bool = flagsLock.withLock { state in
            if state.shutdownRequested { return false }
            state.oneShotRequested = true
            // M11d.6 round 9 — coalesce. If a CVDisplayLink draw
            // is already in flight, set the flag and let the
            // pending block pick it up; do not post a second
            // block behind it. `drawIfNeeded` reads
            // `oneShotRequested` under the same lock and clears
            // it before running the actual paint.
            if state.drawInFlight { return false }
            state.drawInFlight = true
            return true
        }
        guard shouldSchedule else { return }
        scheduleOneShot()
    }

    /// Retarget the active `CVDisplayLink` to `displayID`. Safe
    /// to call from the main thread (the host view does on
    /// `viewDidMoveToWindow` and on
    /// `NSWindow.didChangeScreenNotification`). No-op if the
    /// link is not currently running.
    func setCurrentDisplay(_ displayID: CGDirectDisplayID) {
        guard let link = displayLink else { return }
        CVDisplayLinkSetCurrentCGDisplay(link, displayID)
    }

    /// Stop the link, mark shutdown, drain the queue, and detach
    /// from the renderer. Must be called from the main thread
    /// (typically `dismantleNSView`). Idempotent — repeated calls
    /// after the first are cheap no-ops.
    func shutdown() {
        // First flip the shutdown flag so any vsync callback
        // already in flight short-circuits when it reaches the
        // queue.
        flagsLock.withLock { $0.shutdownRequested = true }
        stopDisplayLink()
        // Drain the queue. Any draw still queued runs but its
        // `drawIfNeeded` will observe `shutdownRequested` and
        // return without touching the renderer. The blocking
        // `sync` guarantees no Metal command buffer is in flight
        // when we return.
        renderQueue.sync {}
    }

    // MARK: - Internals

    /// `targetMachAbsoluteTicks` is the **target vsync host
    /// time** (= the `CVTimeStamp.hostTime` we extrapolate the
    /// playhead to and that we pin the drawable's present to);
    /// `nil` for warm-up / one-shot paths that have no vsync to
    /// target.
    private func scheduleOneShot(targetMachAbsoluteTicks: UInt64? = nil) {
        renderQueue.async { [weak self] in
            self?.drawIfNeeded(targetMachAbsoluteTicks: targetMachAbsoluteTicks)
        }
    }

    /// Single-frame draw. Called from `renderQueue` only.
    ///
    /// `targetMachAbsoluteTicks` is the **target vsync host
    /// time** (= the `CVTimeStamp.hostTime` we extrapolate the
    /// playhead to and that we pin the drawable's present to,
    /// M11d.6 round 12). `nil` for warm-up / generation-bump
    /// one-shots, in which case the renderer falls back to
    /// "extrapolate-to-now" / "present-at-next-vsync".
    /// Translated to the engine's `host_time_ns` domain and to
    /// a `CFTimeInterval` for `present(atTime:)` via
    /// `hostTimeMapping` before being handed to the renderer.
    private func drawIfNeeded(targetMachAbsoluteTicks: UInt64? = nil) {
        // M11d.6 round 9 — clear the coalescing flag at the END
        // of this block, not the start. The whole point of the
        // flag is to prevent a CVDisplayLink callback that fires
        // *while a draw is still running* from queueing a second
        // draw behind it; clearing at the start would let that
        // second draw queue and produce the burst pair the round
        // is trying to eliminate. `defer` runs even on early
        // return so paused / shutdown paths still release the
        // coalescing gate for the next vsync.
        defer { flagsLock.withLock { $0.drawInFlight = false } }
        let (run, _) = flagsLock.withLock { state -> (Bool, Bool) in
            if state.shutdownRequested {
                return (false, false)
            }
            let oneShot = state.oneShotRequested
            state.oneShotRequested = false
            return (state.isContinuous || oneShot, oneShot)
        }
        guard run else { return }
        // Thread sanity signpost. Instruments → System Trace shows
        // this event on the render queue's worker thread. If it
        // ever lands on the main thread we have a regression.
        os_signpost(.event, log: log, name: "drawIfNeeded.thread",
                    "isMain=%d", Thread.isMainThread ? 1 : 0)
        let size = metalLayer.drawableSize
        let targetEngineHostTimeNs: UInt64? = targetMachAbsoluteTicks.map {
            hostTimeMapping.engineHostTimeNs(fromMachAbsoluteTicks: $0)
        }
        let targetPresentSeconds: CFTimeInterval? = targetMachAbsoluteTicks.map {
            hostTimeMapping.cfTimeInterval(fromMachAbsoluteTicks: $0)
        }
        // `nextDrawable()` can return `nil` when the window is
        // hidden, minimised, or off-screen. The render thread
        // simply drops the frame; no allocation, no crash.
        renderer.drawIfPossible(
            into: metalLayer,
            drawableSize: size,
            targetEngineHostTimeNs: targetEngineHostTimeNs,
            targetPresentSeconds: targetPresentSeconds)
    }

    // MARK: - CVDisplayLink lifecycle

    private func startDisplayLink() {
        guard displayLink == nil else { return }
        var link: CVDisplayLink?
        let create = CVDisplayLinkCreateWithActiveCGDisplays(&link)
        guard create == kCVReturnSuccess, let link else {
            NSLog(
                "WaveformRenderThread: CVDisplayLinkCreateWithActiveCGDisplays "
                + "failed (\(create))")
            return
        }
        let retain = Unmanaged.passRetained(self)
        let ptr = retain.toOpaque()
        let callback: CVDisplayLinkOutputCallback = {
            _, _, inOutputTime, _, _, userInfo in
            guard let userInfo else { return kCVReturnSuccess }
            let me = Unmanaged<WaveformRenderThread>
                .fromOpaque(userInfo).takeUnretainedValue()
            // **M11d.6 round 11.** Capture the predicted vsync
            // display time `inOutputTime.pointee.hostTime` (in
            // mach_absolute_time ticks) and hand it to the
            // render queue. The renderer will translate it into
            // the engine's host_time_ns domain and pass it to
            // `positionSnapshotAtHostTime`, so the playhead the
            // shader sees is what the audio is playing *when
            // this frame actually lights up the panel* — not
            // what's playing when the renderer happens to run.
            // CVDisplayLink schedules these on the panel's
            // crystal-locked refresh interval, so consecutive
            // frames are exactly `1/refreshRate` apart with no
            // CPU-jitter contribution.
            //
            // **Round 12 lookahead reverted (round 13).**
            // Targeting `inOutputTime + refreshTicks` (one
            // vsync ahead) interacted catastrophically with the
            // coalescing gate: whenever a CVDisplayLink callback
            // got coalesced (`drawInFlight = true`), the next
            // callback's `inOutputTime` had advanced by two
            // refresh periods relative to the previous draw's
            // snapshot, and the +1 lookahead amplified that
            // into a `Δplayhead = 33 ms` "forward jump" inside a
            // single 17 ms render interval. Net: doubled the
            // visible jitter the lookahead was meant to fix.
            // Going back to "target the upcoming vsync, accept
            // the occasional missed render" — every coalesced
            // callback now produces the *correct* Δplayhead of
            // exactly one refresh period.
            let targetTicks = inOutputTime.pointee.hostTime
            // M11d.6 round 9 — coalesce vsyncs. The serial render
            // queue plus a slow frame would otherwise let a second
            // vsync's draw stack up behind a still-running first,
            // producing a burst pair (1–5 ms apart) that reads as
            // 1-pixel grid jitter when the surrounding frames are
            // at the usual 16 ms cadence. We accept "drop a vsync"
            // over "render twice almost-simultaneously" — uniform
            // motion is the user-visible invariant, not 60 fps.
            let shouldPost = me.flagsLock.withLock { state -> Bool in
                if state.shutdownRequested { return false }
                if state.drawInFlight { return false }
                state.drawInFlight = true
                return true
            }
            guard shouldPost else { return kCVReturnSuccess }
            // Post directly onto the render queue. No main-runloop
            // hop, no DispatchQueue.main work — this is the whole
            // point of the refactor.
            me.renderQueue.async {
                me.drawIfNeeded(targetMachAbsoluteTicks: targetTicks)
            }
            return kCVReturnSuccess
        }
        CVDisplayLinkSetOutputCallback(link, callback, ptr)
        // Retarget to the host window's current screen if we
        // already know one. Phase 2's host view fires the
        // closure on `viewDidMoveToWindow`, so by the time we
        // start the link the most recent display id has already
        // arrived via `setCurrentDisplay`. In the rare case it
        // hasn't, leaving the link on the active-displays
        // default is correct.
        CVDisplayLinkStart(link)
        self.displayLink = link
        self.displayLinkRetain = retain
    }

    private func stopDisplayLink() {
        if let link = displayLink {
            CVDisplayLinkStop(link)
            displayLink = nil
        }
        if let retain = displayLinkRetain {
            retain.release()
            displayLinkRetain = nil
        }
    }
}
