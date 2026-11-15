//
//  OutputMeter.swift
//  Dub
//
//  Two slim meters in the centre gutter, under the phase meter: what is
//  leaving the soundcard for deck A and deck B, right now (2026-11-15;
//  canvas "Dub Output Meters").
//
//  The question it answers is the one a DJ asks when the mixer channel
//  is up and nothing comes out — most often echo out left on after its
//  tail has died, the record still playing underneath. So the meter
//  reads the deck's output pair after everything (track, echo out, the
//  rack, the siren and samples routed to it, master gain), and when it
//  is empty for a reason it says the reason in the meter itself.
//
//  No master meter: Dub never mixes. A and B go to two mixer channels
//  and the mixer has its own master. No gain state either: auto gain
//  sets it.
//

import AppKit
import DubCore
import SwiftUI

/// One deck's meter, as drawn. Pure, so the rules are tested
/// (`OutputMeterTests`).
struct OutputMeterReading: Equatable {
    enum Kind: Equatable {
        /// Signal is leaving the pair.
        case sending
        /// Echo out is on: what leaves is the tail only.
        case echoTail
        /// Echo out is on and its tail is gone — nothing leaves, and the
        /// record plays on underneath. The stage confusion this is for.
        case echoSilent
        /// The deck has no output pair, or the engine is not running.
        case noOutput
    }

    enum Tone: Equatable { case green, amber, red }

    static let segments = 24
    static let stepDb: Float = 2
    static let floorDb: Float = -48
    /// Above this a segment is amber: a line-level peak lives here.
    static let amberDb: Float = -9
    /// The top segment: at the soundcard's ceiling.
    static let redDb: Float = -2

    let kind: Kind
    /// Segments lit, 0 … `segments`.
    let lit: Int

    static func of(peak: Float, routed: Bool, echoEngaged: Bool) -> Self {
        guard routed else { return Self(kind: .noOutput, lit: 0) }
        let lit = litSegments(peak)
        if echoEngaged {
            return Self(kind: lit == 0 ? .echoSilent : .echoTail, lit: lit)
        }
        return Self(kind: .sending, lit: lit)
    }

    /// Segments lit for a linear peak: one per 2 dB above −48 dBFS.
    static func litSegments(_ peak: Float) -> Int {
        guard peak > 1e-6 else { return 0 }
        let db = 20 * log10(peak)
        let steps = ((db - floorDb) / stepDb).rounded()
        return Int(max(0, min(Float(segments), steps)))
    }

    /// Segment `index`, 0 at the bottom.
    static func tone(_ index: Int) -> Tone {
        let top = floorDb + Float(index + 1) * stepDb
        if top > redDb { return .red }
        if top > amberDb { return .amber }
        return .green
    }

    /// Written up the empty meter when it is empty for a reason.
    var word: String? {
        switch kind {
        case .echoSilent: return "ECHO OUT"
        case .noOutput: return "NO OUTPUT"
        case .sending, .echoTail: return nil
        }
    }

    func accessibilityText(deck: String) -> String {
        switch kind {
        case .noOutput: return "Deck \(deck) has no output"
        case .echoSilent: return "Deck \(deck) silent, echo out on"
        case .echoTail: return "Deck \(deck) sending its echo tail"
        case .sending: return lit == 0 ? "Deck \(deck) silent" : "Deck \(deck) sending"
        }
    }
}

// MARK: - View

/// The pair, mounted in the gutter under the phase meter. It reads the
/// engine on its own timer, not through the model: a level never sits
/// still, and carried in `DeckState` it would republish the deck 30×/s.
struct OutputMeterPair: NSViewRepresentable {
    let engine: DubEngine?

    func makeNSView(context: Context) -> OutputMeterPairView {
        let v = OutputMeterPairView()
        configure(v)
        return v
    }

    func updateNSView(_ v: OutputMeterPairView, context: Context) {
        configure(v)
    }

    func configure(_ v: OutputMeterPairView) {
        guard let engine else {
            v.read = { [OutputMeterReading.of(peak: 0, routed: false, echoEngaged: false), .of(peak: 0, routed: false, echoEngaged: false)] }
            return
        }
        v.read = { [engine] in
            (0..<2).map { idx in
                let t = engine.deckTelemetry(deckIdx: UInt64(idx))
                return .of(peak: t.outputPeak, routed: t.outputRouted, echoEngaged: t.echoState != 0)
            }
        }
    }
}

final class OutputMeterPairView: NSView {
    var read: () -> [OutputMeterReading] = { [] }

    /// Where the meters stand: under the phase meter's letter, down to
    /// the gutter's foot. Below this height there is no meter worth
    /// drawing, and none is.
    static let minHeight: CGFloat = 48
    static let trackWidth: CGFloat = 9
    static let trackGap: CGFloat = 6
    static let segmentGap: CGFloat = 2
    static let letterHeight: CGFloat = 14
    /// Air under the phase meter's A / B letter.
    static let topGap: CGFloat = 16

    /// Thirty a second: echo out switching on has to read as instant.
    private static let interval: TimeInterval = 1.0 / 30

    private(set) var shown: [OutputMeterReading] = []
    private var timer: Timer?

    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override init(frame: NSRect) {
        super.init(frame: frame)
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

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

    /// Read once; draw only when a segment or a state changed.
    func refresh() {
        let next = read()
        guard next != shown else { return }
        shown = next
        setAccessibilityLabel(
            zip(["A", "B"], next).map { $1.accessibilityText(deck: $0) }.joined(separator: ", "))
        needsDisplay = true
    }

    /// The meters' column: from under the phase meter to the foot, the
    /// letters below it. `nil` when the gutter is too short.
    static func meterRect(in size: CGSize) -> CGRect? {
        let phase = PhaseMeterCanvas.geometry(size)
        let top = phase.letterCentre.y + topGap
        let bottom = size.height - letterHeight - 4
        guard bottom - top >= minHeight else { return nil }
        let width = trackWidth * 2 + trackGap
        return CGRect(x: (size.width - width) / 2, y: top, width: width, height: bottom - top)
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let column = Self.meterRect(in: bounds.size) else { return }
        let n = OutputMeterReading.segments
        let segH = (column.height - CGFloat(n - 1) * Self.segmentGap) / CGFloat(n)
        guard segH >= 1 else { return }
        for (i, reading) in shown.prefix(2).enumerated() {
            let x = column.minX + CGFloat(i) * (Self.trackWidth + Self.trackGap)
            let track = CGRect(x: x, y: column.minY, width: Self.trackWidth, height: column.height)
            if let word = reading.word {
                drawWord(word, in: track, colour: Self.wordColour(reading.kind))
            } else {
                for s in 0..<n {
                    let y = column.maxY - CGFloat(s + 1) * segH - CGFloat(s) * Self.segmentGap
                    let rect = CGRect(x: x, y: y, width: Self.trackWidth, height: segH)
                    Self.segmentColour(s, lit: s < reading.lit, kind: reading.kind).setFill()
                    NSBezierPath(roundedRect: rect, xRadius: 1, yRadius: 1).fill()
                }
            }
            let letter = NSAttributedString(
                string: i == 0 ? "A" : "B",
                attributes: [
                    .font: NSFont.systemFont(ofSize: 10, weight: .heavy),
                    .foregroundColor: Self.letterColour(reading.kind),
                ])
            let ls = letter.size()
            letter.draw(at: CGPoint(x: track.midX - ls.width / 2, y: column.maxY + 4))
        }
    }

    /// The reason, written up the meter, in a dashed outline the width
    /// of a segment column and a little more.
    private func drawWord(_ word: String, in track: CGRect, colour: NSColor) {
        let box = track.insetBy(dx: -2, dy: 0)
        let outline = NSBezierPath(roundedRect: box.insetBy(dx: 0.5, dy: 0.5), xRadius: 3, yRadius: 3)
        outline.setLineDash([3, 2], count: 2, phase: 0)
        outline.lineWidth = 1
        colour.withAlphaComponent(0.7).setStroke()
        outline.stroke()

        let text = NSAttributedString(
            string: word,
            attributes: [
                .font: NSFont.systemFont(ofSize: 9, weight: .heavy),
                .foregroundColor: colour,
                .kern: 1.2,
            ])
        let size = text.size()
        guard size.width <= box.height - 6 else { return }
        NSGraphicsContext.saveGraphicsState()
        let t = NSAffineTransform()
        t.translateX(by: box.midX, yBy: box.midY)
        // Reads bottom to top, as a fader's legend does.
        t.rotate(byDegrees: -90)
        t.concat()
        text.draw(at: CGPoint(x: -size.width / 2, y: -size.height / 2))
        NSGraphicsContext.restoreGraphicsState()
    }

    static func segmentColour(_ index: Int, lit: Bool, kind: OutputMeterReading.Kind) -> NSColor {
        guard lit else { return NSColor(DubColor.surface2) }
        if kind == .echoTail, OutputMeterReading.tone(index) != .red {
            return NSColor(DubColor.echo)
        }
        switch OutputMeterReading.tone(index) {
        case .green: return NSColor(DubColor.stateLocked)
        case .amber: return NSColor(DubColor.stateTentative)
        case .red: return NSColor(DubColor.stateError)
        }
    }

    static func wordColour(_ kind: OutputMeterReading.Kind) -> NSColor {
        kind == .noOutput ? NSColor(DubColor.stateError) : NSColor(DubColor.echo)
    }

    static func letterColour(_ kind: OutputMeterReading.Kind) -> NSColor {
        switch kind {
        case .sending: return NSColor(DubColor.textSecondary)
        case .echoTail, .echoSilent: return NSColor(DubColor.echo)
        case .noOutput: return NSColor(DubColor.stateError)
        }
    }
}
