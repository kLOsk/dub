//
//  EngineHostTimeMapping.swift
//  Dub
//
//  M11d.6 round 11 — clock-domain bridge between Core Video's
//  `CVTimeStamp.hostTime` (mach_absolute_time ticks) and the
//  Rust engine's `host_time_ns` domain (`Instant::now`-based,
//  resolves to mach_continuous_time on macOS).
//
//  Used by `WaveformRenderThread` to convert each
//  `CVDisplayLink` callback's predicted vsync display time into
//  the engine's nanosecond domain so it can be passed to
//  `DubEngine.positionSnapshotAtHostTime` — that snapshot then
//  extrapolates the playhead to the moment the frame will
//  actually hit the panel, not the moment the renderer ran.
//
//  Clock-domain detail.
//  - `CVTimeStamp.hostTime` is mach_absolute_time ticks. Apple
//    documents these as the system clock (mach kernel) and
//    convertible to nanoseconds via `mach_timebase_info`.
//  - Rust's `Instant::now` on macOS resolves to
//    `mach_continuous_time`. That clock includes time spent in
//    sleep mode; mach_absolute_time pauses during sleep.
//  - For an awake session the two clocks agree, so a single
//    `offsetNs` captured at init suffices. Sleep-aware callers
//    can recompute the offset on
//    `NSApplication.didBecomeActiveNotification` if they expect
//    mid-set system suspends.
//

import DubCore
import Foundation
import Darwin

/// Captures, once, the offset between `mach_absolute_time` (the
/// clock Core Video reports vsync times in) and the engine's
/// `host_time_ns` (= ns since the Rust-side `Instant` origin,
/// resolves to mach_continuous_time on macOS). Use
/// `engineHostTimeNs(fromMachAbsoluteTicks:)` to translate a
/// future `CVTimeStamp.hostTime` into the engine's domain.
///
/// Thread-safe: all state is immutable after init. Lock-free,
/// allocation-free reads — safe to call from the render thread.
final class EngineHostTimeMapping: @unchecked Sendable {
    /// `mach_timebase_info_data_t.numer / denom` — multiplies
    /// `mach_absolute_time` ticks into nanoseconds. On Apple
    /// Silicon this is `125 / 3` (~41.67 ns/tick); on Intel it
    /// is `1 / 1` (1 ns/tick). Captured once at init.
    private let timebaseNumer: UInt64
    private let timebaseDenom: UInt64

    /// Signed offset added to a mach_absolute_time ns value to
    /// get an engine `host_time_ns` value. Captured at init by
    /// sampling both clocks at very close instants. For a
    /// non-sleeping session this stays exact (mach_absolute and
    /// mach_continuous agree).
    private let offsetNs: Int64

    init(engine: DubEngine) {
        // Snapshot the timebase once. `mach_timebase_info` is
        // documented as deterministic across an OS install, so
        // this is a one-time cost.
        var info = mach_timebase_info_data_t()
        _ = mach_timebase_info(&info)
        self.timebaseNumer = UInt64(info.numer)
        self.timebaseDenom = max(UInt64(info.denom), 1)

        // Sample mach_absolute_time on both sides of the FFI
        // call and use the midpoint. The Rust call's own
        // latency (~100 ns to ~1 µs) shows up as a corresponding
        // ~µs-magnitude offset error — well below the >> µs
        // CVDisplayLink-cadence accuracy we actually need to
        // collapse the visible-jitter source.
        let absBefore = mach_absolute_time()
        let engineRelativeNs = engine.currentHostTimeNs()
        let absAfter = mach_absolute_time()
        let absMidTicks = absBefore &+ (absAfter &- absBefore) / 2
        let absMidNs = absMidTicks * UInt64(info.numer) / max(UInt64(info.denom), 1)
        self.offsetNs = Int64(bitPattern: engineRelativeNs) &- Int64(bitPattern: absMidNs)
    }

    /// Translate a `CVTimeStamp.hostTime` (mach_absolute_time
    /// ticks) into the engine's `host_time_ns` domain.
    ///
    /// Saturates the result to a non-negative `UInt64`: the
    /// underlying clocks are monotonic, so a negative result
    /// would only happen on the very first vsync if the cached
    /// offset hasn't yet been computed — clamp to zero so the
    /// FFI's `saturating_sub(now, pub_host)` returns a sane
    /// `elapsed = 0` rather than wrapping past `u64::MAX`.
    func engineHostTimeNs(fromMachAbsoluteTicks ticks: UInt64) -> UInt64 {
        let ns = ticks * timebaseNumer / timebaseDenom
        let signed = Int64(bitPattern: ns) &+ offsetNs
        return signed > 0 ? UInt64(bitPattern: signed) : 0
    }

    /// Translate a mach_absolute_time tick value into the same
    /// `CFTimeInterval` (seconds-since-boot, mach_absolute_time
    /// domain) that `MTLCommandBuffer.present(_:atTime:)`
    /// expects — i.e. the unit `CACurrentMediaTime()` reports.
    /// M11d.6 round 12: used to pin the drawable's actual
    /// present-vsync to the same `CVTimeStamp.hostTime` we
    /// snapshotted the playhead for, so the rendered frame
    /// can never "leak" onto a different vsync than its
    /// extrapolation target.
    func cfTimeInterval(fromMachAbsoluteTicks ticks: UInt64) -> CFTimeInterval {
        let ns = ticks * timebaseNumer / timebaseDenom
        return CFTimeInterval(ns) / 1_000_000_000.0
    }

    /// Translate a CoreVideo `videoRefreshPeriod` (in
    /// `videoTimeScale` units) into mach_absolute_time ticks
    /// (the unit `CVTimeStamp.hostTime` lives in). Used by
    /// `WaveformRenderThread` to look one full vsync ahead of
    /// the CVDisplayLink callback's `inOutputTime` — see
    /// round-12 comment in the callback.
    ///
    /// Returns 0 (sentinel "fall back to 60 Hz default") if
    /// the inputs are invalid. The caller then uses
    /// `16_666_667` ns as a fixed-period fallback.
    func machAbsoluteTicks(
        forVideoRefreshPeriod videoRefreshPeriod: Int64,
        videoTimeScale: Int32
    ) -> UInt64 {
        guard videoRefreshPeriod > 0, videoTimeScale > 0 else { return 0 }
        // refresh_seconds = period / scale.
        // refresh_ns = refresh_seconds * 1e9.
        // refresh_ticks = refresh_ns * timebaseDenom / timebaseNumer.
        let periodNs = (UInt64(videoRefreshPeriod) &* 1_000_000_000) / UInt64(videoTimeScale)
        return periodNs &* timebaseDenom / timebaseNumer
    }
}
