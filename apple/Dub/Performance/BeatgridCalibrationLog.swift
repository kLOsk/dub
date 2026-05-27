//
//  BeatgridCalibrationLog.swift
//  Dub
//
//  NDJSON log of manual beatgrid corrections for offline analysis.
//  After you grid a few tracks by hand, read
//  `~/Library/Application Support/Dub/beatgrid-calibration.ndjson`
//  to compare auto-detected vs corrected anchors/BPM.
//

import Foundation

/// One calibration event written as a single NDJSON line.
///
/// Writes are dispatched to a private serial background queue so the
/// commit-time callers (`commitTapGrid`, `loadTrack`, etc.) on the
/// main actor never block on disk I/O. PRD-BEATS §13 only requires
/// the log exist; the order across events is preserved by the
/// single-writer serial queue.
enum BeatgridCalibrationLog {
    private static let fileName = "beatgrid-calibration.ndjson"

    /// Serial writer queue. One file handle's worth of work at a
    /// time; bounded by the queue's FIFO discipline. `.utility` QoS
    /// keeps log writes off the responsive-UI / user-interactive
    /// scheduling buckets so they never preempt audio-thread
    /// supporting tasks or the Metal render thread.
    private static let writerQueue = DispatchQueue(
        label: "dub.beatgrid-calibration.writer",
        qos: .utility)

    static var logURL: URL {
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask).first!
        return base
            .appendingPathComponent("Dub", isDirectory: true)
            .appendingPathComponent(fileName)
    }

    static func logAutoGrid(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        artist: String?,
        durationSecs: Double,
        source: String,
        bpm: Double,
        anchorSecs: Double,
        confidence: Double,
        beatCount: Int
    ) {
        append([
            "event": "track_auto_grid",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "artist": artist as Any,
            "durationSecs": durationSecs,
            "gridSource": source,
            "autoBpm": bpm,
            "autoAnchorSecs": anchorSecs,
            "autoConfidence": confidence,
            "autoBeatCount": beatCount,
        ])
    }

    static func logManualAdjust(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        action: String,
        tier: BeatgridNudgeTier,
        delta: Double,
        playheadSecs: Double,
        autoBpm: Double?,
        autoAnchorSecs: Double?,
        resultBpm: Double,
        resultAnchorSecs: Double,
        editIndex: Int
    ) {
        var payload: [String: Any] = [
            "event": "manual_adjust",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "action": action,
            "tier": tier.rawValue,
            "delta": delta,
            "playheadSecs": playheadSecs,
            "resultBpm": resultBpm,
            "resultAnchorSecs": resultAnchorSecs,
            "editIndex": editIndex,
        ]
        if let autoBpm {
            payload["autoBpm"] = autoBpm
            payload["bpmCorrection"] = resultBpm - autoBpm
        }
        if let autoAnchorSecs {
            payload["autoAnchorSecs"] = autoAnchorSecs
            payload["anchorCorrectionMs"] = (resultAnchorSecs - autoAnchorSecs) * 1000.0
        }
        append(payload)
    }

    static func logTapDownbeat(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        downbeatSecs: Double,
        tapCount: Int,
        autoBpm: Double?,
        autoAnchorSecs: Double?,
        resultBpm: Double,
        resultAnchorSecs: Double
    ) {
        var payload: [String: Any] = [
            "event": "tap_downbeat",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "downbeatSecs": downbeatSecs,
            "tapCount": tapCount,
            "resultBpm": resultBpm,
            "resultAnchorSecs": resultAnchorSecs,
        ]
        if let autoBpm { payload["autoBpm"] = autoBpm }
        if let autoAnchorSecs { payload["autoAnchorSecs"] = autoAnchorSecs }
        append(payload)
    }

    static func logTapTempoRecalc(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        tapTimes: [Double],
        autoBpm: Double?,
        autoAnchorSecs: Double?,
        resultBpm: Double,
        resultAnchorSecs: Double,
        durationSecs: Double
    ) {
        var payload: [String: Any] = [
            "event": "tap_tempo_recalc",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "tapTimes": tapTimes,
            "resultBpm": resultBpm,
            "resultAnchorSecs": resultAnchorSecs,
            "durationSecs": durationSecs,
        ]
        if let autoBpm {
            payload["autoBpm"] = autoBpm
            payload["bpmError"] = resultBpm - autoBpm
        }
        if let autoAnchorSecs {
            payload["autoAnchorSecs"] = autoAnchorSecs
            payload["anchorErrorMs"] = (resultAnchorSecs - autoAnchorSecs) * 1000.0
        }
        append(payload)
    }

    static func logBarBeatMark(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        beat: Int,
        playheadSecs: Double,
        autoBpm: Double?,
        autoAnchorSecs: Double?
    ) {
        var payload: [String: Any] = [
            "event": "bar_beat_mark",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "beat": beat,
            "playheadSecs": playheadSecs,
        ]
        if let autoBpm { payload["autoBpm"] = autoBpm }
        if let autoAnchorSecs { payload["autoAnchorSecs"] = autoAnchorSecs }
        append(payload)
    }

    static func logBarCalibrated(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        beatSecs: [Double],
        computedBpm: Double,
        autoBpm: Double?,
        autoAnchorSecs: Double?,
        durationSecs: Double
    ) {
        let d12 = (beatSecs[1] - beatSecs[0]) * 1000.0
        let d23 = (beatSecs[2] - beatSecs[1]) * 1000.0
        let d34 = (beatSecs[3] - beatSecs[2]) * 1000.0
        let bpm12 = 60_000.0 / d12
        let bpm23 = 60_000.0 / d23
        let bpm34 = 60_000.0 / d34
        let bpmBar = 60_000.0 * 3.0 / ((beatSecs[3] - beatSecs[0]) * 1000.0)
        let intervals = [d12, d23, d34]
        let spread = (intervals.max() ?? 0) - (intervals.min() ?? 0)

        var payload: [String: Any] = [
            "event": "bar_calibrated",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "durationSecs": durationSecs,
            "beat1Secs": beatSecs[0],
            "beat2Secs": beatSecs[1],
            "beat3Secs": beatSecs[2],
            "beat4Secs": beatSecs[3],
            "interval12Ms": d12,
            "interval23Ms": d23,
            "interval34Ms": d34,
            "bpmFrom12": bpm12,
            "bpmFrom23": bpm23,
            "bpmFrom34": bpm34,
            "bpmFromBar": bpmBar,
            "computedBpm": computedBpm,
            "intervalSpreadMs": spread,
        ]
        if let autoBpm {
            payload["autoBpm"] = autoBpm
            payload["bpmError"] = computedBpm - autoBpm
        }
        if let autoAnchorSecs {
            payload["autoAnchorSecs"] = autoAnchorSecs
            payload["anchorErrorMs"] = (beatSecs[0] - autoAnchorSecs) * 1000.0
        }
        append(payload)
    }

    static func logFinalized(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        autoBpm: Double?,
        autoAnchorSecs: Double?,
        finalBpm: Double,
        finalAnchorSecs: Double,
        editCount: Int,
        durationSecs: Double
    ) {
        var payload: [String: Any] = [
            "event": "track_finalized",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "finalBpm": finalBpm,
            "finalAnchorSecs": finalAnchorSecs,
            "editCount": editCount,
            "durationSecs": durationSecs,
        ]
        if let autoBpm {
            payload["autoBpm"] = autoBpm
            payload["bpmCorrection"] = finalBpm - autoBpm
        }
        if let autoAnchorSecs {
            payload["autoAnchorSecs"] = autoAnchorSecs
            payload["anchorCorrectionMs"] = (finalAnchorSecs - autoAnchorSecs) * 1000.0
        }
        append(payload)
    }

    private static func nowMs() -> Int {
        Int(Date().timeIntervalSince1970 * 1000)
    }

    private static func append(_ payload: [String: Any]) {
        // Serialise the payload on the caller's thread (cheap; no
        // file I/O) so the captured value is a plain `Data` and
        // doesn't carry across the queue boundary as an
        // `Any`-typed dictionary. The disk write itself is moved
        // to the writer queue.
        guard JSONSerialization.isValidJSONObject(payload),
              let data = try? JSONSerialization.data(withJSONObject: payload),
              let line = String(data: data, encoding: .utf8),
              let bytes = (line + "\n").data(using: .utf8)
        else { return }

        let url = logURL
        writerQueue.async {
            let dir = url.deletingLastPathComponent()
            try? FileManager.default.createDirectory(
                at: dir, withIntermediateDirectories: true)

            if FileManager.default.fileExists(atPath: url.path),
               let handle = try? FileHandle(forWritingTo: url) {
                defer { try? handle.close() }
                _ = try? handle.seekToEnd()
                try? handle.write(contentsOf: bytes)
            } else {
                try? bytes.write(to: url, options: .atomic)
            }
        }
    }
}
