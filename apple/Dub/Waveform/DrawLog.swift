//
//  DrawLog.swift
//  Dub
//
//  One line per drawn frame per deck, to the file `DUB_DRAW_LOG` names —
//  Debug builds only. `dub draw-check` reads it back: a playhead that steps
//  further than the deck's rate explains is a jump, a time axis that
//  changes scale while the fader did not is a zoom change, and the next
//  beat line's distance from the playhead shows whether the lines slide or
//  step. The rig's "by eye" waveform items, as numbers (2026-11-28).
//
//  The signposts in `WaveformRenderer` carry more, but macOS keeps them
//  only in memory: they have to be dumped within moments of the run,
//  which a scripted run through the loop cable does not reliably do.
//

import Foundation

#if DEBUG
final class DrawLog: @unchecked Sendable {
    /// Open when the process was started with `DUB_DRAW_LOG=<path>`.
    static let shared: DrawLog? = ProcessInfo.processInfo.environment["DUB_DRAW_LOG"]
        .flatMap { $0.isEmpty ? nil : DrawLog(path: $0) }

    private let handle: FileHandle
    private let queue = DispatchQueue(label: "com.klos.dub.drawlog", qos: .utility)
    private let lock = NSLock()
    private var pending = ""
    private var flushScheduled = false

    private init?(path: String) {
        guard FileManager.default.createFile(atPath: path, contents: nil),
              let handle = FileHandle(forWritingAtPath: path)
        else { return nil }
        self.handle = handle
        let header = "# t_ns\tdeck\thas_track\tplayhead_s\trate\tsecs_per_px\tagg"
            + "\tnext_beat_px\tdrawn_above\tdrawn_below\n"
        handle.write(Data(header.utf8))
    }

    /// One frame of one deck. Called from the render threads; the file is
    /// written on a utility queue, half a second at a time, so a frame
    /// never waits on the disk.
    func record(
        deck: UInt64, targetNs: UInt64, hasTrack: Bool, playheadSecs: Double, rate: Double,
        secsPerPixel: Double, aggregation: Int, nextBeatPixels: Double,
        drawnAbove: Int, drawnBelow: Int
    ) {
        let beat = nextBeatPixels.isFinite ? String(format: "%.3f", nextBeatPixels) : "nan"
        let line = String(
            format: "%llu\t%llu\t%d\t%.6f\t%.5f\t%.9f\t%d\t%@\t%d\t%d\n",
            targetNs, deck, hasTrack ? 1 : 0, playheadSecs, rate, secsPerPixel,
            aggregation, beat, drawnAbove, drawnBelow)
        lock.lock()
        pending += line
        let schedule = !flushScheduled
        flushScheduled = true
        lock.unlock()
        if schedule {
            queue.asyncAfter(deadline: .now() + 0.5) { [weak self] in self?.flush() }
        }
    }

    private func flush() {
        lock.lock()
        let out = pending
        pending = ""
        flushScheduled = false
        lock.unlock()
        handle.write(Data(out.utf8))
    }
}
#endif
