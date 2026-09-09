//
//  DeckSignalPanel.swift
//  Dub
//
//  Per-deck timecode signal health as a deck-pane slide-out — the GUI
//  "dub scope", moved out of Preferences and onto the deck where a DJ
//  actually sound-checks. A slim SIGNAL tab sits on each deck's outer
//  edge; clicking it slides the panel over the performance-pads area
//  (an overlay, so the waveform column never reflows).
//
//  Surfaces the engine's lock-free deck telemetry (FFI 33): carrier
//  confidence and amplitude, lock state, a rolling pitch-stability
//  trace, calibration state, and the sticker-drift readout. A healthy
//  carrier sits high on both bars with a green lock and a flat pitch
//  trace; dust, a worn stylus, or an uncalibrated needle makes the
//  trace jump.
//
//  Calibration: the engine auto-calibrates the moment it's sure the
//  input is timecode (PRD §5.1.1). **Calibrate** forces a fresh capture
//  (after a cartridge swap, or if the auto pass landed on a noisy
//  window); **Auto** hands the deck back to automatic source detection.
//

import SwiftUI
import DubCore

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

    /// Polled, not read inside `body`. Four times a second is plenty
    /// for a lock dot, and it holds whether the panel is open or shut.
    @State private var telemetry: DeckTelemetry?

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

    /// The one curve the drawer moves on — the panel's transition and
    /// the tab's position change are the same motion and must share it.
    static let slide = Animation.spring(duration: 0.25)

    /// `-signalOpen` starts the drawer open, for profiling it without
    /// a click. The panel is the most expensive thing on the surface
    /// when it is showing, so being able to measure it repeatably is
    /// worth one launch argument.
    @State private var open = ProcessInfo.processInfo.arguments.contains("-signalOpen")

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
        // One animation for the pair. The tab is laid out by this
        // `HStack`, so when the panel appears the tab's *position*
        // changes — and the panel arrived on a transition with its own
        // timing while the tab moved on this one. They ran at different
        // speeds and a gap opened between them mid-slide. Same curve,
        // same duration, driven by the same value.
        .animation(DeckSignalSlideOut.slide, value: open)
        .frame(maxWidth: .infinity, maxHeight: .infinity,
               alignment: side == .a ? .leading : .trailing)
    }

    /// Always in the tree, its *width* animated between zero and full.
    ///
    /// It used to be inserted and removed on a `.move` transition. That
    /// is a different mechanism from the layout change beside it: the
    /// `HStack` allocated the panel's full width the instant it
    /// appeared, so the tab jumped to its final position while the
    /// panel was still sliding in behind — the gap you could watch open
    /// and close. Animating the width makes the tab's position and the
    /// panel's edge the same number, so they cannot disagree.
    private var panel: some View {
        Group {
            if open {
                DeckSignalPanel(engine: engine, side: side, deckIdx: deckIdx)
            }
        }
        .frame(width: open ? DubLayout.deckSignalPanelWidth : 0)
        .clipped()
        .opacity(open ? 1 : 0)
    }

    /// Slim always-visible toggle: vertical SIGNAL caps + the PRD §5.4
    /// tracking dot, so signal health is glanceable even while closed.
    private var tab: some View {
        let t = telemetry ?? engine.deckTelemetry(deckIdx: deckIdx)
        return Button {
            open.toggle()
        } label: {
            VStack(spacing: DubSpacing.sm) {
                Circle()
                    .fill(lockColor(t.lockState, hasInput: t.hasTimecodeInput))
                    .frame(width: 6, height: 6)
                Text("SIGNAL")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(open ? DubColor.textPrimary : DubColor.textTertiary)
                    .fixedSize()
                    .rotationEffect(.degrees(side == .a ? 90 : -90))
                    .frame(width: 10, height: 56)
            }
            .frame(width: 18, height: 96)
            .background(DubColor.surface2.opacity(open ? 1.0 : 0.6))
            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Toggle deck \(side.label) signal panel")
        .onReceive(dotTick) { _ in telemetry = engine.deckTelemetry(deckIdx: deckIdx) }
    }
}

/// The panel body: one deck's signal health + calibration controls.
///
/// Everything live in here is drawn by `SignalScopeView`, an
/// `NSView`. That is not a stylistic choice — see its file comment for
/// the measurements. The short version: this window lays out on every
/// display cycle whether or not anything changed, and the cost of that
/// is proportional to how much view exists. Replacing the readouts with
/// `Color.clear` took the open drawer from 18 % to 4 %, while changing
/// how *often* they updated did nothing at all. So they are drawn.
struct DeckSignalPanel: View {

    let engine: DubEngine
    let side: DeckSide
    let deckIdx: UInt64

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            SignalScopeView(engine: engine, deckIdx: deckIdx)
                .frame(height: SignalScopeView.height)

            // The buttons stay SwiftUI: they are controls with actions,
            // they change only on click, and hand-rolling `NSButton`
            // would trade a real affordance for nothing measurable.
            HStack(spacing: DubSpacing.sm) {
                Button("Calibrate") { try? engine.calibrateDeck(deckIdx: deckIdx) }
                Button("Auto") { try? engine.setDeckAutoControl(deckIdx: deckIdx) }
                Spacer()
            }
            .font(DubFont.micro)

            Spacer(minLength: 0)
        }
        .padding(DubSpacing.md)
        .frame(width: DubLayout.deckSignalPanelWidth)
        .frame(maxHeight: .infinity)
        .background(DubColor.surface2.opacity(0.97))
        .overlay(
            Rectangle()
                .fill(DubColor.divider)
                .frame(width: 1),
            alignment: side == .a ? .trailing : .leading)
    }
}

/// Shared lock-state → colour/label mapping (PRD §5.4 tracking dot).
func lockColor(_ state: UInt8, hasInput: Bool) -> Color {
    guard hasInput else { return DubColor.textPlaceholder }
    switch state {
    case 1:  return DubColor.stateLocked
    case 2:  return DubColor.stateTentative
    case 3:  return DubColor.stateError
    default: return DubColor.textPlaceholder
    }
}

func lockLabel(_ state: UInt8, hasInput: Bool) -> String {
    guard hasInput else { return "No timecode input" }
    switch state {
    case 1:  return "Locked"
    case 2:  return "Degraded"
    case 3:  return "No lock / scratching"
    default: return "—"
    }
}

#Preview("Deck signal panel") {
    DeckSignalPanel(engine: WaveformAppModel().engine, side: .a, deckIdx: 0)
}
