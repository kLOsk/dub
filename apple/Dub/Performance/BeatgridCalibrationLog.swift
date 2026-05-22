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
enum BeatgridCalibrationLog {
    private static let fileName = "beatgrid-calibration.ndjson"

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

    static func logDownbeatMark(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        playheadSecs: Double,
        autoBpm: Double?,
        autoAnchorSecs: Double?
    ) {
        var payload: [String: Any] = [
            "event": "downbeat_mark",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "playheadSecs": playheadSecs,
        ]
        if let autoBpm { payload["autoBpm"] = autoBpm }
        if let autoAnchorSecs { payload["autoAnchorSecs"] = autoAnchorSecs }
        append(payload)
    }

    static func logDownbeatRelatched(
        side: String,
        trackId: String?,
        path: String?,
        title: String?,
        downbeatSecs: Double,
        resultBpm: Double,
        resultAnchorSecs: Double,
        autoBpm: Double?,
        autoAnchorSecs: Double?,
        durationSecs: Double
    ) {
        var payload: [String: Any] = [
            "event": "downbeat_relatch",
            "timestampMs": nowMs(),
            "side": side,
            "trackId": trackId as Any,
            "path": path as Any,
            "title": title as Any,
            "downbeatSecs": downbeatSecs,
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
        guard JSONSerialization.isValidJSONObject(payload),
              let data = try? JSONSerialization.data(withJSONObject: payload),
              let line = String(data: data, encoding: .utf8),
              let bytes = (line + "\n").data(using: .utf8)
        else { return }

        let url = logURL
        let dir = url.deletingLastPathComponent()
        try? FileManager.default.createDirectory(
            at: dir, withIntermediateDirectories: true)

        if FileManager.default.fileExists(atPath: url.path),
           let handle = try? FileHandle(forWritingTo: url) {
            defer { try? handle.close() }
            try? handle.seekToEnd()
            try? handle.write(contentsOf: bytes)
        } else {
            try? bytes.write(to: url, options: .atomic)
        }
    }
}
