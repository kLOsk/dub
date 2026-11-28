//
//  DeckSignalPanel.swift
//  Dub
//
//  Per-deck timecode signal health as a deck-pane slide-out — the GUI
//  "dub scope", moved out of Preferences and onto the deck where a DJ
//  actually sound-checks. A slim SIGNAL tab sits on each deck's outer
//  edge; clicking it slides the panel over the deck column (an overlay,
//  so the waveform column never reflows).
//
//  The panel says what a DJ asks of it: is the control record locked,
//  and if not, what to check — a big state word with one line of advice,
//  the carrier on a scope, the two sides of the pair and the decode
//  quality as bars, the pitch (redesign, 2026-11-15; `SignalStatus` holds
//  the rules). Sticker drift shows in Debug builds only.
//
//  Calibration: the engine auto-calibrates the moment it's sure the
//  input is timecode (PRD §5.1.1), and the drawer opens by itself for a
//  deck's first calibration of the session, with the progress bar in it,
//  then slides away once the deck reads LOCKED (`SignalDrawer`, Daniel,
//  2026-11-25). **Calibrate** forces a fresh capture (after a cartridge
//  swap, or if the auto pass landed on a noisy window).
//

import SwiftUI
import DubCore

/// What a deck's SIGNAL drawer does by itself (Daniel, 2026-11-25).
///
/// It opens while a timecode deck makes its first calibration of the
/// session: the DJ sees the needle measured, learns where the signal
/// lives, and sees that it works. It slides away once the deck has
/// calibrated and read LOCKED for a moment. When something wants fixing
/// — no input, a weak side, Phase asking for a setting — it stays.
///
/// Only the first calibration. After that the tab's dot speaks: a drawer
/// that moved by itself mid-set would land on the pads under the DJ's
/// hands. And the DJ's own click on the tab or the × ends it at once:
/// from then on the drawer is theirs.
///
/// Held by the model, not the view: Prep takes the Performance surface
/// down, and the drawer has to come back the way it was.
@MainActor
final class SignalDrawer: ObservableObject {

    /// What the drawer goes by, read off the deck's telemetry.
    struct Deck: Equatable {
        /// The source switch is on TC.
        var timecode: Bool
        /// An input is routed to the deck.
        var hasInput: Bool
        /// Calibrated, the pitch settled: the deck has let its track go.
        var settled: Bool
        /// LOCKED, the record named, and the VINYL line asks nothing.
        var healthy: Bool
    }

    enum Want: Equatable {
        /// Not on TC: there is nothing to calibrate.
        case shut
        /// Measuring, or no input to measure.
        case open
        /// Calibrated, and something wants fixing.
        case stay
        /// Calibrated and locked.
        case go
    }

    static func want(_ deck: Deck) -> Want {
        guard deck.timecode else { return .shut }
        guard deck.hasInput, deck.settled else { return .open }
        return deck.healthy ? .go : .stay
    }

    @Published private(set) var open: Bool
    /// The panel's content is in the tree: from the moment it starts to
    /// slide in until it has slid away. Shut, its timers stop and AppKit
    /// has nothing of it to walk.
    @Published private(set) var showing: Bool
    /// The last move was the drawer's own, so it goes slower: the eye
    /// follows it to the tab and learns where it lives.
    private(set) var movedByItself = false
    /// Still looking after the session's first calibration.
    private(set) var minding: Bool
    private var healthySince: Date?
    private var hide: DispatchWorkItem?

    /// How long a calibrated deck reads locked before the drawer goes —
    /// long enough to see *Calibrated* and LOCKED turn green.
    static let healthyFor: TimeInterval = 1.5
    /// A click: quick, no bounce.
    static let clicked = Animation.spring(duration: 0.3)
    /// By itself: slower, so it is seen going.
    static let byItself = Animation.easeInOut(duration: 0.5)
    /// The panel leaves the tree once the slower of the two is over.
    private static let slideAway: TimeInterval = 0.6

    /// Runs the panel's removal once the slide is over.
    typealias Later = (_ delay: TimeInterval, _ work: DispatchWorkItem) -> Void
    /// The main queue in the app. Tests run the work themselves: a
    /// wall-clock wait on a busy test host's main queue overran its
    /// timeout (2026-11-26).
    private let later: Later

    /// `open`: `-signalOpen` starts it open, for profiling the panel
    /// without a click, and then it never moves by itself.
    init(open: Bool = false, later: @escaping Later = { delay, work in
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }) {
        self.open = open
        showing = open
        minding = !open
        self.later = later
    }

    /// The tab: the drawer is the DJ's from here on.
    func toggle() {
        minding = false
        move(to: !open, byItself: false)
    }

    /// The ×.
    func close() {
        minding = false
        move(to: false, byItself: false)
    }

    func observe(_ deck: Deck, at now: Date) {
        guard minding else { return }
        let want = Self.want(deck)
        if want != .go { healthySince = nil }
        switch want {
        case .shut:
            if open { move(to: false, byItself: true) }
        case .open:
            if !open { move(to: true, byItself: true) }
        case .stay, .go:
            // Calibrated before the drawer ever opened — in Prep, or on
            // INT before the switch went to TC. There was nothing to watch.
            guard open else {
                minding = false
                return
            }
            guard want == .go else { return }
            let since = healthySince ?? now
            healthySince = since
            if now.timeIntervalSince(since) >= Self.healthyFor {
                minding = false
                move(to: false, byItself: true)
            }
        }
    }

    private func move(to opening: Bool, byItself: Bool) {
        guard opening != open else { return }
        movedByItself = byItself
        hide?.cancel()
        hide = nil
        if opening {
            showing = true
        } else {
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, !self.open else { return }
                    self.showing = false
                }
            }
            hide = work
            later(Self.slideAway, work)
        }
        open = opening
    }
}

extension SignalDrawer.Deck {
    init(_ t: DeckTelemetry) {
        // 1 is TC (FFI `DeckTelemetry.controlMode`).
        timecode = t.controlMode == 1
        hasInput = t.hasTimecodeInput
        settled = !SignalStatus.measuring(hasInput: t.hasTimecodeInput, pitchSettled: t.pitchSettled)
        let status = SignalStatus.of(
            hasInput: t.hasTimecodeInput, lockState: t.lockState, weakChannel: t.weakChannel,
            rms: (t.inputRmsLeft, t.inputRmsRight), pitchPercent: t.pitchPercent)
        healthy = status.kind == .locked && !t.detectingVinyl
            && !SignalStatus.vinylAsks(vinyl: t.vinyl, synthetic: t.vinylSynthetic, counted: t.vinylCounted)
    }
}

/// Outer-edge tab + slide-out signal panel for one deck. Deck A slides
/// out from the window-left edge, deck B from the window-right.
struct DeckSignalSlideOut: View {

    /// The engine, not the model.
    ///
    /// Both views here only ever reached through to `model.engine`, but
    /// observing the model meant every published change anywhere in the
    /// app — playhead, deck state, telemetry heartbeat — re-rendered
    /// the panel *and* made a fresh `deckTelemetry` call across the
    /// FFI, because that call sat inside `body`. During playback that
    /// is continuous, and it is why the drawer felt laggy. Telemetry is
    /// polled on its own timer now and nothing else can trigger a
    /// redraw.
    let engine: DubEngine
    let side: DeckSide
    let deckIdx: UInt64
    /// Whether it is open, and what it does by itself. Changes a few
    /// times a session.
    @ObservedObject var drawer: SignalDrawer

    /// What the tab draws, kept apart from the telemetry it comes from:
    /// the levels differ on every poll, and storing the whole telemetry
    /// redrew the tab four times a second for nothing it shows.
    @State private var face = TabFace(dot: DubColor.textTertiary, measuring: nil)

    /// **`@State`, not `let`.** A timer stored as a plain property is
    /// rebuilt every time SwiftUI recreates the struct, and this view
    /// sits inside the deck pane, which re-renders continuously while a
    /// deck plays. Each rebuild scheduled a fresh run-loop timer and
    /// re-subscribed `onReceive` to it — dozens per second, per deck,
    /// on the main thread in `.common` mode, which is why dragging the
    /// window was where it hurt most. `@State` creates it once and
    /// keeps it for the view's lifetime.
    @State private var dotTick =
        Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    private var shut: CGFloat {
        side == .a ? -DubLayout.deckSignalPanelWidth : DubLayout.deckSignalPanelWidth
    }

    var body: some View {
        HStack(spacing: 0) {
            if side == .a {
                panel
                tab
            } else {
                tab
                panel
            }
        }
        // The panel and its tab are one piece, *moved* — never laid out
        // again. Two earlier versions slid them as two motions: a `.move`
        // transition on the panel beside a layout change for the tab,
        // which ran at different speeds and opened a gap mid-slide, then
        // a width animated from zero, which re-laid the panel out on
        // every frame and wiped it open from its middle instead of
        // sliding it. An offset is a transform: both edges are the same
        // number, nothing beside it reflows, and nothing inside it is
        // laid out while it moves.
        .offset(x: drawer.open ? 0 : shut)
        .animation(drawer.movedByItself ? SignalDrawer.byItself : SignalDrawer.clicked, value: drawer.open)
        .frame(maxWidth: .infinity, maxHeight: .infinity,
               alignment: side == .a ? .leading : .trailing)
        // Nothing of it reaches past its deck pane, the slide included.
        .clipped()
        .onAppear(perform: poll)
        .onReceive(dotTick) { _ in poll() }
    }

    /// Full width always, so the slide has a fixed thing to move; its
    /// content only while `showing`.
    ///
    /// The width hangs on `Color.clear`, which is always there. It hung
    /// on a `Group` first, and a `Group`'s modifiers go to its children:
    /// with the content gone, the width went too, and the shut offset
    /// threw the tab off the screen (2026-11-26).
    private var panel: some View {
        Color.clear
            .frame(width: DubLayout.deckSignalPanelWidth)
            .overlay(alignment: .top) {
                if drawer.showing {
                    DeckSignalPanel(engine: engine, side: side, deckIdx: deckIdx) { drawer.close() }
                        // It slides in solid: no fade on top of the move.
                        .transition(.identity)
                }
            }
            .allowsHitTesting(drawer.open)
    }

    /// Slim always-visible toggle: vertical SIGNAL caps + the PRD §5.4
    /// tracking dot, so signal health is glanceable even while closed.
    /// Shut while the deck calibrates, it fills from the bottom in the
    /// deck's colour: the deck is holding its track, and the DJ who
    /// closed the drawer still sees why.
    private var tab: some View {
        Button {
            drawer.toggle()
        } label: {
            VStack(spacing: DubSpacing.sm) {
                // The deck's signal at a glance, panel shut: green
                // locked, amber weak, grey nothing.
                Circle()
                    .fill(face.dot)
                    .frame(width: 9, height: 9)
                Text("SIGNAL")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(drawer.open ? DubColor.textPrimary : DubColor.textTertiary)
                    .fixedSize()
                    .rotationEffect(.degrees(side == .a ? 90 : -90))
                    .frame(width: 10, height: 56)
            }
            .frame(width: Self.tabWidth, height: Self.tabHeight)
            .background(alignment: .bottom) { calibrationFill }
            .background(DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Toggle \(side.label) signal panel")
    }

    private static let tabWidth = DubLayout.deckSignalTabWidth
    private static let tabHeight: CGFloat = 96

    /// Scaled rather than sized: a transform, so a step of it lays
    /// nothing out. Eased across the poll interval so it fills rather
    /// than ticks.
    @ViewBuilder
    private var calibrationFill: some View {
        if let progress = face.measuring, !drawer.open {
            Rectangle()
                .fill(DubColor.deckTint(side).opacity(0.45))
                .frame(width: Self.tabWidth, height: Self.tabHeight)
                .scaleEffect(x: 1, y: max(0.04, progress), anchor: .bottom)
                .animation(.linear(duration: 0.25), value: progress)
        }
    }

    private func poll() {
        let t = engine.deckTelemetry(deckIdx: deckIdx)
        let measuring = SignalStatus.measuring(hasInput: t.hasTimecodeInput, pitchSettled: t.pitchSettled)
        let next = TabFace(
            dot: Self.dotColour(t),
            // In fiftieths: finer than the tab's 96 pt shows, coarse
            // enough that a still record redraws nothing.
            measuring: measuring ? (Double(t.measureProgress) * 50).rounded() / 50 : nil)
        if next != face { face = next }
        drawer.observe(SignalDrawer.Deck(t), at: Date())
    }

    /// The tab's dot, from the same rules as the panel's big word.
    static func dotColour(_ t: DeckTelemetry) -> Color {
        SignalStatus.of(
            hasInput: t.hasTimecodeInput, lockState: t.lockState, weakChannel: t.weakChannel,
            rms: (t.inputRmsLeft, t.inputRmsRight), pitchPercent: t.pitchPercent
        ).colour
    }
}

/// What the closed drawer's tab shows.
private struct TabFace: Equatable {
    var dot: Color
    /// How far the deck's calibration has got, while it measures.
    var measuring: Double?
}

/// The panel body: one deck's signal, as the DJ needs it — the state in
/// a word, the carrier on a scope, the two sides and the decode quality
/// as bars, the pitch, and the calibration line with its bar and button.
///
/// Everything live is drawn by `SignalScopeView`, an `NSView` (see its
/// file comment for why). The header and the button are SwiftUI: they
/// are controls, and they change only on a click.
struct DeckSignalPanel: View {

    let engine: DubEngine
    let side: DeckSide
    let deckIdx: UInt64
    var onClose: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            HStack {
                Text("SIGNAL · \(side.label)")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(DubColor.textSecondary)
                Spacer()
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(DubColor.textSecondary)
                        .frame(width: 24, height: 24)
                        .overlay(RoundedRectangle(cornerRadius: 5).stroke(DubColor.divider))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close the signal panel")
            }

            // As tall as the deck pane lets it be: the scope gives way
            // first, and CALIBRATE stays on screen.
            SignalScopeView(engine: engine, deckIdx: deckIdx, tint: DubColor.deckTint(side))
                .frame(minHeight: SignalScopeView.minHeight, maxHeight: SignalScopeView.height)
                .overlay(alignment: .bottomTrailing) {
                    Button { try? engine.calibrateDeck(deckIdx: deckIdx) } label: {
                        Text("CALIBRATE")
                            .font(.system(size: 10, weight: .bold))
                            .tracking(1.1)
                            .foregroundStyle(DubColor.textPrimary)
                            .padding(.horizontal, 10)
                            .frame(height: 26)
                            .background(RoundedRectangle(cornerRadius: 5).fill(DubColor.surface3))
                            .overlay(RoundedRectangle(cornerRadius: 5).stroke(DubColor.divider))
                    }
                    .buttonStyle(.plain)
                    .help("Measure the control record again — after a cartridge or cable change, "
                        + "or if the needle sounds wrong. Play the record while it measures.")
                }
                // Ahead of the spacer: all the room it can use, then the
                // spacer gets what is left.
                .layoutPriority(1)

            Spacer(minLength: 0)
        }
        .padding(DubSpacing.md)
        .frame(width: DubLayout.deckSignalPanelWidth)
        .frame(maxHeight: .infinity)
        // Solid: the pads showed through the old 97 % ground and read as
        // part of the panel (rig, 2026-11-15).
        .background(DubColor.surface2)
        .overlay(
            Rectangle()
                .fill(DubColor.divider)
                .frame(width: 1),
            alignment: side == .a ? .trailing : .leading)
    }
}

#Preview("Deck signal panel") {
    DeckSignalPanel(engine: WaveformAppModel().engine, side: .a, deckIdx: 0)
}
