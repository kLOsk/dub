//
//  ThruDeck.swift
//  Dub
//
//  What a deck on THRU draws in place of a track's (rig, 2026-11-14:
//  "there's no waveform, the deck still says no track loaded"). A live
//  record has no file, so no cues, no loops and no time remaining; what
//  it does have is a signal, a tempo and a side that grows as it plays.
//
//  - `ThruMeterPair` — the input, left and right. Replaces the hot cues:
//    a dead cartridge wire reads as one empty meter.
//  - `ThruBeatLamp` — flashes on the beats the waveform predicts.
//  - `ThruRecordClock` — how long the record under the needle has played.
//  - `ThruSideOverview` — the side so far, growing left to right, the
//    overview of a record that has no end yet.
//
//  All four tick on Core Animation layers, never in SwiftUI, for the
//  reason `LayerTickers.swift` gives: a SwiftUI tick re-lays-out the
//  window and costs the strips their frames.
//

import AppKit
import DubCore
import SwiftUI

// MARK: - Input meters

/// The THRU deck's two input meters, left over right.
struct ThruMeterPair: NSViewRepresentable {
    let engine: DubEngine?
    let deckIdx: UInt64
    let tint: Color

    func makeNSView(context: Context) -> ThruMeterPairView {
        let v = ThruMeterPairView()
        configure(v)
        return v
    }

    func updateNSView(_ v: ThruMeterPairView, context: Context) {
        configure(v)
    }

    private func configure(_ v: ThruMeterPairView) {
        if let engine {
            v.read = { [engine, deckIdx] in
                let t = engine.deckTelemetry(deckIdx: deckIdx)
                return ThruMeterPairView.Levels(
                    rms: [t.inputRmsLeft, t.inputRmsRight],
                    peak: [t.inputPeakLeft, t.inputPeakRight])
            }
        } else {
            v.read = { ThruMeterPairView.Levels(rms: [0, 0], peak: [0, 0]) }
        }
        v.tint = NSColor(tint)
    }
}

final class ThruMeterPairView: NSView {
    struct Levels {
        var rms: [Float]
        var peak: [Float]
    }

    var read: () -> Levels = { Levels(rms: [0, 0], peak: [0, 0]) }
    var tint: NSColor = .systemGreen {
        didSet { place() }
    }

    /// Thirty a second: a needle drop has to read as instant.
    private static let interval: TimeInterval = 1.0 / 30

    /// The scale: −48 dBFS at the left edge, 0 at the right.
    static let floorDb: Float = -48
    /// Where the bars turn hot — a line-level record peaks around here,
    /// and a phono stage clipping the interface is above it.
    static let hotDb: Float = -6

    private let tracks = [CALayer(), CALayer()]
    private let bars = [CALayer(), CALayer()]
    private let peaks = [CALayer(), CALayer()]
    private let labels = [CATextLayer(), CATextLayer()]
    private let hotLine = CALayer()
    private var levels = Levels(rms: [0, 0], peak: [0, 0])
    private var timer: Timer?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        let none: [String: CAAction] = [
            "bounds": NSNull(), "position": NSNull(), "frame": NSNull(),
            "backgroundColor": NSNull(), "contents": NSNull(), "string": NSNull(),
            "hidden": NSNull(),
        ]
        for (i, label) in labels.enumerated() {
            label.string = i == 0 ? "L" : "R"
            label.font = NSFont.systemFont(ofSize: 10, weight: .semibold)
            label.fontSize = 10
            label.foregroundColor = NSColor(DubColor.textTertiary).cgColor
            label.alignmentMode = .left
        }
        for l in tracks {
            l.backgroundColor = NSColor(DubColor.surface0).cgColor
            l.cornerRadius = 2
        }
        for l in bars { l.cornerRadius = 2 }
        for l in peaks { l.backgroundColor = NSColor(DubColor.textPrimary).cgColor }
        hotLine.backgroundColor = NSColor(DubColor.textTertiary).cgColor
        for l in tracks + bars + peaks + labels + [hotLine] {
            l.actions = none
            layer?.addSublayer(l)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        for l in labels { l.contentsScale = window?.backingScaleFactor ?? 2 }
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
        refresh()
    }

    override func layout() {
        super.layout()
        place()
    }

    private func refresh() {
        levels = read()
        place()
    }

    /// 0…1 along the scale for a linear level.
    static func fraction(_ linear: Float) -> CGFloat {
        guard linear > 0 else { return 0 }
        let db = 20 * log10(linear)
        return CGFloat(max(0, min(1, (db - floorDb) / -floorDb)))
    }

    private func place() {
        let b = bounds
        let labelW: CGFloat = 14
        let gap: CGFloat = 4
        let barH = max(4, (b.height - gap) / 2)
        let width = max(0, b.width - labelW)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for i in 0..<2 {
            let y = CGFloat(i) * (barH + gap)
            labels[i].frame = CGRect(x: 0, y: y + (barH - 12) / 2, width: labelW, height: 12)
            tracks[i].frame = CGRect(x: labelW, y: y, width: width, height: barH)
            let rms = i < levels.rms.count ? levels.rms[i] : 0
            let peak = i < levels.peak.count ? levels.peak[i] : 0
            let fill = Self.fraction(rms) * width
            bars[i].isHidden = fill < 1
            bars[i].frame = CGRect(x: labelW, y: y, width: fill, height: barH)
            let hot = rms > 0 && 20 * log10(rms) > Self.hotDb
            bars[i].backgroundColor = (hot ? NSColor(DubColor.stateError) : tint).cgColor
            let p = Self.fraction(peak) * width
            peaks[i].isHidden = p < 1
            peaks[i].frame = CGRect(x: labelW + max(0, p - 2), y: y, width: 2, height: barH)
        }
        let hotX = labelW + CGFloat((Self.hotDb - Self.floorDb) / -Self.floorDb) * width
        hotLine.frame = CGRect(x: hotX, y: 0, width: 1, height: b.height)
        CATransaction.commit()
    }
}

// MARK: - Beat lamp

/// One lamp that flashes on each beat the waveform predicts — the same
/// phase, so what the eye sees on the strip and on the lamp agree.
struct ThruBeatLamp: NSViewRepresentable {
    let engine: DubEngine?
    let deckIdx: UInt64
    /// The tempo the DJ is using; `nil` leaves the lamp dark.
    let bpm: Double?
    let tint: Color

    func makeNSView(context: Context) -> ThruBeatLampView {
        let v = ThruBeatLampView()
        configure(v)
        return v
    }

    func updateNSView(_ v: ThruBeatLampView, context: Context) {
        configure(v)
    }

    private func configure(_ v: ThruBeatLampView) {
        v.tint = NSColor(tint)
        guard let engine, let bpm else {
            v.phase = { nil }
            v.now = { 0 }
            return
        }
        v.phase = { [engine, deckIdx] in
            let p = engine.thruBeatPhase(deckIdx: deckIdx, bpm: bpm)
            return p.found ? (p.lastBeatSecs, 60 / bpm) : nil
        }
        v.now = { [engine, deckIdx] in
            Double(engine.peaksLen(deckIdx: deckIdx)) * engine.peaksChunkDurationSecs(deckIdx: deckIdx)
        }
    }
}

final class ThruBeatLampView: NSView {
    /// `(newest beat, period)` in live-waveform seconds.
    var phase: () -> (Double, Double)? = { nil }
    /// Where the live waveform is now, in the same seconds.
    var now: () -> Double = { 0 }
    var tint: NSColor = .white

    private static let interval: TimeInterval = 1.0 / 60
    /// The phase is refetched this often; between fetches the beat is
    /// counted forward from the last one found.
    private static let phaseRefresh: CFTimeInterval = 0.5
    /// How long the lamp stays lit on each beat.
    private static let flashSecs: Double = 0.09

    private let lamp = CALayer()
    private var timer: Timer?
    private var cached: (Double, Double)?
    private var fetchedAt: CFTimeInterval = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        lamp.actions = ["backgroundColor": NSNull(), "frame": NSNull(), "bounds": NSNull(), "position": NSNull()]
        lamp.borderWidth = 1
        lamp.borderColor = NSColor(DubColor.divider).cgColor
        layer?.addSublayer(lamp)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
        refresh()
    }

    override func layout() {
        super.layout()
        let side = min(bounds.width, bounds.height)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        lamp.frame = CGRect(
            x: (bounds.width - side) / 2, y: (bounds.height - side) / 2, width: side, height: side)
        lamp.cornerRadius = side / 2
        CATransaction.commit()
    }

    private func refresh() {
        let t = CACurrentMediaTime()
        if t - fetchedAt > Self.phaseRefresh {
            fetchedAt = t
            cached = phase()
        }
        var lit = false
        if let (last, period) = cached, period > 0 {
            let since = (now() - last).truncatingRemainder(dividingBy: period)
            let into = since < 0 ? since + period : since
            lit = into < Self.flashSecs
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        lamp.backgroundColor = (lit ? tint : NSColor(DubColor.surface0)).cgColor
        CATransaction.commit()
    }
}

// MARK: - Record clock

/// How long the record under the needle has played, `--:--` while the
/// needle is up. On a layer, read twice a second, like the track clock.
struct ThruRecordClock: NSViewRepresentable {
    let engine: DubEngine
    let deckIdx: UInt64
    let size: CGFloat
    let color: Color

    func makeNSView(context: Context) -> LayerClockView {
        let v = LayerClockView()
        configure(v)
        return v
    }

    func updateNSView(_ v: LayerClockView, context: Context) {
        configure(v)
    }

    private func configure(_ v: LayerClockView) {
        v.read = { [engine, deckIdx] in
            Self.text(secs: engine.thruTempo(deckIdx: deckIdx).recordSecs)
        }
        v.setStyle(
            font: NSFont.monospacedSystemFont(ofSize: size, weight: .medium),
            color: NSColor(color))
    }

    /// `mm:ss`, or `--:--` with no record.
    static func text(secs: Double) -> String {
        guard secs >= 0, secs.isFinite else { return "--:--" }
        let whole = Int(secs.rounded(.down))
        return String(format: "%02d:%02d", whole / 60, whole % 60)
    }
}

// MARK: - The side so far

/// The record on a THRU deck as it has played: the envelope from the
/// needle drop to now, growing left to right against minute marks. The
/// axis starts at six minutes — a 12" side — and stretches a minute at a
/// time past it, so a side never runs off the end and a short one does
/// not fill the strip from its first second. When the needle comes up
/// the side stays drawn until the next record makes a sound.
struct ThruSideOverview: NSViewRepresentable {
    let engine: DubEngine
    let deckIdx: UInt64
    let tint: Color
    let height: CGFloat

    func makeNSView(context: Context) -> ThruSideOverviewView {
        let v = ThruSideOverviewView()
        v.engine = engine
        v.deckIdx = deckIdx
        v.tint = NSColor(tint)
        return v
    }

    func updateNSView(_ v: ThruSideOverviewView, context: Context) {
        v.engine = engine
        v.deckIdx = deckIdx
        v.tint = NSColor(tint)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: ThruSideOverviewView, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 300, height: height)
    }
}

final class ThruSideOverviewView: NSView {
    var engine: DubEngine?
    var deckIdx: UInt64 = 0
    var tint: NSColor = .white {
        didSet { if oldValue != tint { draw(redrawTicks: false) } }
    }

    private static let interval: TimeInterval = 1.0
    /// Buckets across a full axis — the track overview's count.
    private static let bucketCount = 480
    /// At most this many chunks per tick, so arriving mid-side catches up
    /// over a few seconds instead of stalling one tick.
    private static let maxChunksPerTick: UInt64 = 200_000

    private let ground = CALayer()
    private let envelope = CAShapeLayer()
    private let ticks = CAShapeLayer()
    private let head = CALayer()
    private let emptyLine = CAShapeLayer()
    private var tickLabels: [CATextLayer] = []
    private var timer: Timer?

    private var tiler = RipEnvelopeTiler()
    private var sideStartChunk: UInt64?
    private var generation: UInt64?
    private var playing = false
    private var axisSecs: Double = 360
    private var buckets: [OverviewBucket] = []
    private var filled: CGFloat = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        let none: [String: CAAction] = [
            "bounds": NSNull(), "position": NSNull(), "frame": NSNull(), "path": NSNull(),
            "contents": NSNull(), "string": NSNull(), "fillColor": NSNull(),
            "strokeColor": NSNull(), "hidden": NSNull(),
        ]
        ground.backgroundColor = NSColor(DubColor.surface0).cgColor
        ticks.strokeColor = NSColor(DubColor.divider).cgColor
        ticks.lineWidth = 1
        emptyLine.strokeColor = NSColor(DubColor.divider).cgColor
        emptyLine.lineDashPattern = [3, 4]
        emptyLine.lineWidth = 1
        for l in [ground, ticks, envelope, head, emptyLine] as [CALayer] {
            l.actions = none
            layer?.addSublayer(l)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
        refresh()
    }

    override func layout() {
        super.layout()
        draw(redrawTicks: true)
    }

    private func refresh() {
        guard let engine else { return }
        let gen = engine.peaksGeneration(deckIdx: deckIdx)
        if gen != generation {
            generation = gen
            reset(at: nil)
        }
        let chunkDur = engine.peaksChunkDurationSecs(deckIdx: deckIdx)
        guard chunkDur > 0 else { return }
        let tempo = engine.thruTempo(deckIdx: deckIdx)
        playing = tempo.recordStartSecs >= 0
        if playing {
            let start = UInt64((tempo.recordStartSecs / chunkDur).rounded(.down))
            if start != sideStartChunk { reset(at: start) }
        }
        if let start = sideStartChunk, playing {
            let data = engine.peaksExtend(
                deckIdx: deckIdx, startIdx: start + tiler.chunksFetched,
                maxChunks: Self.maxChunksPerTick)
            if !data.isEmpty { tiler.feed(data) }
        }
        let secs = Double(tiler.chunksFetched) * chunkDur
        let previousAxis = axisSecs
        axisSecs = Self.axisSecs(forElapsed: secs)
        let fraction = min(1, secs / axisSecs)
        let count = max(1, Int((Double(Self.bucketCount) * fraction).rounded()))
        buckets = tiler.chunksFetched > 0 ? tiler.buckets(count: count) : []
        filled = CGFloat(fraction)
        draw(redrawTicks: previousAxis != axisSecs)
    }

    private func reset(at start: UInt64?) {
        tiler = RipEnvelopeTiler()
        sideStartChunk = start
        buckets = []
        filled = 0
        axisSecs = Self.axisSecs(forElapsed: 0)
    }

    /// Six minutes, or the next whole minute past where the side is.
    static func axisSecs(forElapsed secs: Double) -> Double {
        max(360, ((secs + 30) / 60).rounded(.up) * 60)
    }

    private func draw(redrawTicks: Bool) {
        let b = bounds
        let pad = OverviewLayout.endPadding
        let axisW = max(0, b.width - 2 * pad)
        let labelH: CGFloat = 12
        let laneH = max(0, b.height - labelH)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        ground.frame = CGRect(x: 0, y: 0, width: b.width, height: laneH)

        if redrawTicks || tickLabels.isEmpty {
            let path = CGMutablePath()
            let minutes = Int(axisSecs / 60)
            // A mark a minute up to ten; past that, every other.
            let every = minutes > 10 ? 2 : 1
            var labelIndex = 0
            for m in stride(from: 0, through: minutes, by: every) {
                let x = pad + axisW * CGFloat(Double(m * 60) / axisSecs)
                path.move(to: CGPoint(x: x, y: 0))
                path.addLine(to: CGPoint(x: x, y: laneH))
                if labelIndex >= tickLabels.count {
                    let label = CATextLayer()
                    label.font = NSFont.systemFont(ofSize: 9, weight: .medium)
                    label.fontSize = 9
                    label.foregroundColor = NSColor(DubColor.textTertiary).cgColor
                    label.contentsScale = window?.backingScaleFactor ?? 2
                    label.actions = ["position": NSNull(), "bounds": NSNull(), "string": NSNull(), "hidden": NSNull()]
                    layer?.addSublayer(label)
                    tickLabels.append(label)
                }
                let label = tickLabels[labelIndex]
                label.isHidden = false
                label.string = m == 0 ? "0" : "\(m)m"
                label.frame = CGRect(x: x + 2, y: laneH, width: 30, height: labelH)
                labelIndex += 1
            }
            for extra in tickLabels.dropFirst(labelIndex) { extra.isHidden = true }
            ticks.path = path
            ticks.frame = b
        }

        let filledW = axisW * filled
        if buckets.isEmpty {
            envelope.path = nil
            head.isHidden = true
            let line = CGMutablePath()
            line.move(to: CGPoint(x: pad, y: laneH / 2))
            line.addLine(to: CGPoint(x: pad + axisW, y: laneH / 2))
            emptyLine.path = line
            emptyLine.isHidden = false
        } else {
            emptyLine.isHidden = true
            envelope.path = Self.envelopePath(
                buckets: buckets, x: pad, width: filledW, baseline: laneH, height: laneH - 3)
            envelope.fillColor = tint.withAlphaComponent(0.75).cgColor
            envelope.strokeColor = tint.cgColor
            envelope.lineWidth = 1
            head.isHidden = !playing
            head.backgroundColor = NSColor(DubColor.textPrimary).cgColor
            head.frame = CGRect(x: pad + filledW - 1, y: 0, width: 2, height: laneH)
        }
        envelope.frame = b
        emptyLine.frame = b
        CATransaction.commit()
    }

    /// The energy envelope over `width`, the rip overview's blend of RMS
    /// and peak so a breakdown dips and a drop rises.
    static func envelopePath(
        buckets: [OverviewBucket], x: CGFloat, width: CGFloat, baseline: CGFloat, height: CGFloat
    ) -> CGPath {
        let path = CGMutablePath()
        let n = buckets.count
        guard n > 0, width > 0 else { return path }
        var maxEnergy: Float = 1e-6
        let energy = buckets.map { b -> Float in
            let e = 0.22 * b.peak + 0.78 * b.rms
            maxEnergy = max(maxEnergy, e)
            return e
        }
        let norm = 0.9 / maxEnergy
        path.move(to: CGPoint(x: x, y: baseline))
        for (i, e) in energy.enumerated() {
            let px = x + width * CGFloat(i) / CGFloat(n)
            let h = CGFloat(min(1, max(0, e * norm))) * height
            path.addLine(to: CGPoint(x: px, y: baseline - max(1, h)))
        }
        path.addLine(to: CGPoint(x: x + width, y: baseline))
        path.closeSubpath()
        return path
    }
}
