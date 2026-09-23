//
//  PhaseMeter.swift
//  Dub
//
//  The beatmatch aid, round 4 (2026-09-16): Traktor's phase meter, one
//  beat long, standing in the gutter between the two running waveforms.
//
//  It replaces Stillpoint — three rounds of a cleverer instrument (role
//  inference, a workflow FSM, honesty gates, a pitch coach) that Daniel
//  called, in the end, worse than the standard. The standard is one
//  bar: half a beat ahead, half a beat behind, a marker for where the
//  other deck's beat sits against the master's. It does not care about
//  tempo. Tempo is matched by the numbers in the deck headers and by
//  the two strips scrolling at the same speed; the meter is for the
//  last few milliseconds, and for seeing which way to push.
//
//  Vertical, because the strips are: the centre line is collinear with
//  the strips' playheads (25 % from the top, PRD §9.1), and the meter
//  spans one beat of the gutter's height centred on it. The axis is the
//  one Stillpoint's round 6 settled on the rig — **late = above the
//  line**, the gap the room has opened ahead of you; push the record
//  and the marker settles down onto the line. Past half a beat it wraps
//  to the other end, as a phase does.
//
//  The maths is a pure function of both decks' grids and playheads, so
//  the whole thing is unit-testable; the canvas is a pure function of
//  one frame, so it snapshot-tests.
//

import AppKit
import SwiftUI

// MARK: - Model

/// One deck's contribution, sampled together with the other's.
struct PhaseMeterInputs: Equatable {
    var hasTrack = false
    var isPlaying = false
    /// The grid's BPM, in track time.
    var bpm: Double?
    /// The grid's first beat, in track seconds.
    var gridAnchorSecs: Double?
    /// The platter's pitch, so a beat's length in the room is known.
    var pitchPercent: Double?
    var playheadSecs: Double = 0

    /// Whether the deck has a grid to take a phase from.
    var hasGrid: Bool {
        hasTrack && (bpm ?? 0) > 0 && gridAnchorSecs != nil
    }

    /// Where the playhead sits inside its beat, 0 ≤ φ < 1 — from the
    /// grid, in track time, so the platter's pitch does not enter.
    var beatPhase: Double? {
        guard hasGrid, let bpm, let anchor = gridAnchorSecs else { return nil }
        let beats = (playheadSecs - anchor) * bpm / 60
        return beats - beats.rounded(.down)
    }

    /// A beat's length in the room, in ms — the grid's beat at the
    /// platter's pitch.
    var beatMs: Double? {
        guard hasGrid, let bpm else { return nil }
        let rate = 1 + (pitchPercent ?? 0) / 100
        guard rate > 0.05 else { return nil }
        return 60_000 / (bpm * rate)
    }
}

/// Everything the canvas draws.
struct PhaseMeterFrame: Equatable {
    /// The incoming deck's beat against the master's, in beats,
    /// −0.5 … 0.5. Negative = late (drawn above the line). `nil` when
    /// either deck has no grid.
    var phaseBeats: Double?
    /// The same, in the room's milliseconds — the master's beat length.
    var phaseMs: Double?
    /// The deck the marker belongs to — the one that is not the master.
    var incomingIsA = false
    /// Both playing and inside the lock window.
    var locked = false
}

enum PhaseMeter {
    /// Inside this the marker goes green: a beat placed within the
    /// tolerance a room stops hearing as two.
    static let lockWindowMs = 15.0

    /// Compare the incoming deck against the master. With no master
    /// declared, deck A is the reference.
    static func frame(a: PhaseMeterInputs, b: PhaseMeterInputs, masterIsA: Bool) -> PhaseMeterFrame {
        let master = masterIsA ? a : b
        let other = masterIsA ? b : a
        var out = PhaseMeterFrame(incomingIsA: !masterIsA)
        guard let pm = master.beatPhase, let po = other.beatPhase, let beatMs = master.beatMs
        else { return out }
        // Wrap the difference into (−0.5, 0.5]: past half a beat, the
        // nearer beat is the other one.
        var phi = po - pm
        phi -= (phi + 0.5).rounded(.down)
        if phi <= -0.5 { phi += 1 }
        out.phaseBeats = phi
        out.phaseMs = phi * beatMs
        out.locked = master.isPlaying && other.isPlaying && abs(phi * beatMs) <= lockWindowMs
        return out
    }
}

// MARK: - Driver

/// The gutter drawn once, the marker moved on a layer.
///
/// This was the whole canvas in a 60 Hz `TimelineView`: every frame
/// SwiftUI re-rendered the full-height gutter (≈ 72 × 2 400 px on the
/// rig's Retina panel) into a fresh RenderBox surface, and with the
/// window's layout churn gone that became the main thread's biggest
/// cost — 57 % of it asleep in `wait_for_allocations`, the strips
/// short of drawables behind it. The track, ticks and line never move;
/// only the marker and its letter do, and a layer's position is a
/// property the compositor applies without drawing anything.
///
/// `model` is not observed: nothing here is rebuilt by a publish. The
/// marker reads both decks on its own clock.
struct PhaseMeterView: View {
    let model: WaveformAppModel

    var body: some View {
        ZStack {
            PhaseMeterCanvas(frame: PhaseMeterFrame())
                .accessibilityHidden(true)
            PhaseMeterMarker(read: { [model] in PhaseMeterView.frame(model) })
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(DubColor.surface0)
    }

    @MainActor
    static func frame(_ model: WaveformAppModel) -> PhaseMeterFrame {
        PhaseMeter.frame(
            a: inputs(model, model.deckA, idx: 0),
            b: inputs(model, model.deckB, idx: 1),
            masterIsA: model.masterDeck != .b)
    }

    @MainActor
    private static func inputs(_ model: WaveformAppModel, _ deck: DeckState, idx: UInt64) -> PhaseMeterInputs {
        var i = PhaseMeterInputs()
        i.hasTrack = deck.hasTrack
        i.isPlaying = deck.isPlaying
        i.bpm = deck.bpm
        i.gridAnchorSecs = deck.gridAnchorSecs
        // The engine's, not the model's: the model holds pitch to the
        // readout's 0.1 % steps, and the meter integrates the rate.
        i.pitchPercent = deck.isPlaying
            ? model.engine.deckTelemetry(deckIdx: idx).pitchPercent : nil
        i.playheadSecs = model.engine.positionSnapshot(deckIdx: idx).playheadSecsUnclamped
        return i
    }
}

// MARK: - Canvas

/// The meter: a narrow track one beat tall on the playhead line, the
/// marker on it. Pure — a function of the frame and the size.
struct PhaseMeterCanvas: View {
    let frame: PhaseMeterFrame

    /// The playheads' line, PRD §9.1.
    static let lineFraction: CGFloat = 0.25
    /// One beat, in points, at full height; shrinks on a short gutter so
    /// the half-beat above the line stays on screen.
    static let beatHeight: CGFloat = 160
    static let trackWidth: CGFloat = 10
    static let markerWidth: CGFloat = 24
    static let markerHeight: CGFloat = 4

    /// Where everything sits in a gutter of `size` — shared with the
    /// live marker layer, so the marker lands on the track drawn here.
    struct Geometry {
        let lineY: CGFloat
        let beat: CGFloat
        let cx: CGFloat
        let track: CGRect

        /// The marker for a phase of `phi` beats.
        func markerRect(phi: Double) -> CGRect {
            let y = lineY + CGFloat(phi) * beat
            return CGRect(
                x: cx - PhaseMeterCanvas.markerWidth / 2, y: y - PhaseMeterCanvas.markerHeight / 2,
                width: PhaseMeterCanvas.markerWidth, height: PhaseMeterCanvas.markerHeight)
        }

        /// Centre of the A / B letter under the track.
        var letterCentre: CGPoint { CGPoint(x: cx, y: track.maxY + 10) }
    }

    static func geometry(_ size: CGSize) -> Geometry {
        let lineY = size.height * lineFraction
        let beat = min(beatHeight, size.height * lineFraction * 1.8)
        let cx = size.width / 2
        return Geometry(
            lineY: lineY, beat: beat, cx: cx,
            track: CGRect(x: cx - trackWidth / 2, y: lineY - beat / 2, width: trackWidth, height: beat))
    }

    static func tint(_ frame: PhaseMeterFrame) -> Color {
        frame.locked
            ? DubColor.stateLocked
            : (frame.incomingIsA ? DubColor.deckATint : DubColor.deckBTint)
    }

    var body: some View {
        Canvas { ctx, size in
            let g = Self.geometry(size)
            let lineY = g.lineY
            let beat = g.beat
            let track = g.track

            // The track: a recessed well, quarter-beat ticks either side.
            let well = Path(roundedRect: track, cornerRadius: Self.trackWidth / 2)
            ctx.fill(well, with: .color(DubColor.surface2))
            ctx.stroke(well, with: .color(DubColor.divider), lineWidth: 1)
            for q in [-0.25, 0.25] {
                let y = lineY + CGFloat(q) * beat
                var tick = Path()
                tick.move(to: CGPoint(x: track.minX - 4, y: y))
                tick.addLine(to: CGPoint(x: track.minX, y: y))
                tick.move(to: CGPoint(x: track.maxX, y: y))
                tick.addLine(to: CGPoint(x: track.maxX + 4, y: y))
                ctx.stroke(tick, with: .color(DubColor.divider), lineWidth: 1)
            }

            // The line: across the whole gutter, so it reads as the same
            // line the two strips draw their playheads on.
            var line = Path()
            line.move(to: CGPoint(x: 0, y: lineY))
            line.addLine(to: CGPoint(x: size.width, y: lineY))
            ctx.stroke(line, with: .color(DubColor.playheadAccent.opacity(0.35)), lineWidth: 1)
            var onTrack = Path()
            onTrack.move(to: CGPoint(x: track.minX - 6, y: lineY))
            onTrack.addLine(to: CGPoint(x: track.maxX + 6, y: lineY))
            ctx.stroke(onTrack, with: .color(DubColor.playheadAccent), lineWidth: 1)

            guard let phi = frame.phaseBeats else { return }
            let tint = Self.tint(frame)

            // The marker. At half a beat it is at an end of the track and
            // the next frame puts it at the other — the wrap a phase has.
            drawMarker(ctx, rect: g.markerRect(phi: phi), tint: tint, alpha: 1)

            // Whose beat the marker is.
            let letter = Text(frame.incomingIsA ? "A" : "B")
                .font(.system(size: 9, weight: .bold))
                .foregroundColor(tint.opacity(0.9))
            ctx.draw(letter, at: g.letterCentre)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement()
        .accessibilityLabel(accessibilityText)
    }

    private func drawMarker(_ ctx: GraphicsContext, rect: CGRect, tint: Color, alpha: Double) {
        let bar = Path(roundedRect: rect, cornerRadius: Self.markerHeight / 2)
        ctx.fill(bar, with: .color(tint.opacity(alpha)))
        if frame.locked {
            ctx.stroke(bar, with: .color(tint.opacity(0.5)), lineWidth: 3)
        }
    }

    private var accessibilityText: String {
        guard let ms = frame.phaseMs else { return "Phase meter, no grid" }
        let deck = frame.incomingIsA ? "A" : "B"
        if frame.locked { return "Deck \(deck) in phase" }
        return String(format: "Deck %@ %@ by %.0f milliseconds", deck, ms < 0 ? "late" : "early", abs(ms))
    }
}

// MARK: - Live marker

/// The marker and its letter on Core Animation layers, read at 60 Hz.
/// A tick that changes nothing commits nothing; one that does moves a
/// layer — no SwiftUI update, no RenderBox surface, no layout.
struct PhaseMeterMarker: NSViewRepresentable {
    let read: () -> PhaseMeterFrame

    func makeNSView(context: Context) -> PhaseMeterMarkerView {
        let v = PhaseMeterMarkerView()
        v.read = read
        return v
    }

    func updateNSView(_ v: PhaseMeterMarkerView, context: Context) {
        v.read = read
    }
}

final class PhaseMeterMarkerView: NSView {
    var read: () -> PhaseMeterFrame = { PhaseMeterFrame() }

    private static let interval: TimeInterval = 1.0 / 60.0

    private let marker = CAShapeLayer()
    private let letter = CATextLayer()
    private var shown: PhaseMeterFrame?
    private var shownSize: CGSize = .zero
    private var timer: Timer?

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        let none: [String: CAAction] = [
            "position": NSNull(), "bounds": NSNull(), "hidden": NSNull(), "path": NSNull(),
            "fillColor": NSNull(), "strokeColor": NSNull(), "lineWidth": NSNull(),
            "string": NSNull(), "foregroundColor": NSNull(), "contents": NSNull(), "frame": NSNull(),
        ]
        marker.actions = none
        letter.actions = none
        letter.font = NSFont.systemFont(ofSize: 9, weight: .bold)
        letter.fontSize = 9
        letter.alignmentMode = .center
        marker.isHidden = true
        letter.isHidden = true
        layer?.addSublayer(marker)
        layer?.addSublayer(letter)
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        setAccessibilityLabel("Phase meter, no grid")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        letter.contentsScale = window?.backingScaleFactor ?? 2
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        letter.contentsScale = window?.backingScaleFactor ?? 2
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
        refresh()
    }

    override func layout() {
        super.layout()
        shown = nil
        refresh()
    }

    private func refresh() {
        let frame = read()
        let size = bounds.size
        guard frame != shown || size != shownSize else { return }
        shown = frame
        shownSize = size
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }
        guard let phi = frame.phaseBeats, size.width > 0, size.height > 0 else {
            marker.isHidden = true
            letter.isHidden = true
            setAccessibilityLabel("Phase meter, no grid")
            return
        }
        let g = PhaseMeterCanvas.geometry(size)
        let tint = NSColor(PhaseMeterCanvas.tint(frame))
        let rect = g.markerRect(phi: phi)
        marker.frame = rect
        marker.path = CGPath(
            roundedRect: CGRect(origin: .zero, size: rect.size),
            cornerWidth: rect.height / 2, cornerHeight: rect.height / 2, transform: nil)
        marker.fillColor = tint.cgColor
        marker.strokeColor = frame.locked ? tint.withAlphaComponent(0.5).cgColor : nil
        marker.lineWidth = frame.locked ? 3 : 0
        marker.isHidden = false
        let h: CGFloat = 12
        letter.frame = CGRect(x: g.letterCentre.x - 10, y: g.letterCentre.y - h / 2, width: 20, height: h)
        letter.string = frame.incomingIsA ? "A" : "B"
        letter.foregroundColor = tint.withAlphaComponent(0.9).cgColor
        letter.isHidden = false
        let deck = frame.incomingIsA ? "A" : "B"
        if frame.locked {
            setAccessibilityLabel("Deck \(deck) in phase")
        } else if let ms = frame.phaseMs {
            setAccessibilityLabel(String(
                format: "Deck %@ %@ by %.0f milliseconds", deck, ms < 0 ? "late" : "early", abs(ms)))
        }
    }
}
