//
//  PerformancePadsView.swift
//  Dub
//
//  Per-deck performance pads, occupying the outer space beside each
//  deck's waveform (the "whole lot of nothing" the centred-cluster
//  layout left behind). Modelled on Serato Scratch Live's pad area:
//  the running waveform + its overview sit toward the centre (next to
//  the phase clock), and the cue / loop / quick-scratch / sampler
//  controls live out here on the deck's outer edge.
//
//  The CUE and LOOP rows are live. CUE: keys 1–4 set/recall a hot cue
//  on the master deck, Shift+key clears it (see `handleHotCue`); a pad
//  lights in the deck's tint when its slot holds a position. LOOP (M13):
//  click a length pad to fire a grid-snapped reverse loop of that many
//  bars (the bars just heard), ✕ exits. A momentary pad click is within
//  the §1 mouse rule (not a continuous performance gesture); keyboard
//  bindings may follow as a convenience (PRD §5.5). Quick Scratch +
//  Sampler (M17) remain honest placeholders, laid out now so the surface
//  reads as a real performance instrument rather than empty space.
//

import AppKit
import DubCore
import SwiftUI

struct PerformancePadsView: View {

    let side: DeckSide

    /// What this column draws. See `PerformancePadsState`.
    var state = PerformancePadsState()
    /// What its pads fire.
    var callbacks = PerformancePadsCallbacks()

    /// Hug the deck: deck A's pads (window-left) sit against their
    /// overview on the right; deck B's (window-right) sit against
    /// their overview on the left. Top-aligned vertically — the space
    /// below is the §9.6.1 canvas reserved for per-deck info chips,
    /// and growth downward is visible where growth from a centred
    /// column was not.
    private var frameAlignment: Alignment { side == .a ? .topTrailing : .topLeading }

    /// The column's content. Pulled out of `body` so `ViewThatFits` can
    /// offer it twice — plain, then wrapped in a scroll view.
    private var stack: some View {
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            CuePadSection(
                cues: state.cues,
                isPlaying: state.isPlaying,
                onCue: callbacks.onCue,
                onPreviewDown: callbacks.onPreviewDown,
                onPreviewUp: callbacks.onPreviewUp)
            LoopPadSection(
                activeBars: state.activeLoopBars,
                loopEngaged: state.loopEngaged,
                loopInArmed: state.loopInArmed,
                // Seven pads need 338 pt; this column is 224.
                wrap: true,
                onLoop: callbacks.onLoop,
                onLoopIn: callbacks.onLoopIn,
                onLoopOut: callbacks.onLoopOut,
                onExit: callbacks.onExit)
            if state.echoEnabled {
                EchoPadSection(
                    engaged: state.echoEngaged, onToggle: callbacks.onEchoToggle)
            }
            if state.rackEnabled {
                RackFxRow(
                    active: state.rackActive,
                    macro: state.rackMacro,
                    onToggle: callbacks.onRackToggle,
                    onMacro: callbacks.onRackMacro)
            }
        }
    }

    var body: some View {
        // Three guards against the overflow that hid the sampler pads
        // behind the FX bar for a whole milestone. `ViewThatFits`
        // scrolls rather than spilling when the pane is genuinely too
        // short; `.top` alignment means any future growth goes
        // downward instead of symmetrically out of both ends; and
        // `.clipped()` turns a silent overflow (drawn over a
        // neighbour) into a visible one. The real fix is that the
        // column now holds only per-deck controls — see GlobalRackBar.
        ViewThatFits(in: .vertical) {
            stack
            ScrollView(.vertical, showsIndicators: false) { stack }
        }
        .padding(.horizontal, DubSpacing.lg)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: frameAlignment)
        .clipped()
        .background(DubColor.surface0)
    }

}

/// **Currently unmounted.** The siren left Prep — it is a thing you play
/// over a record, not something you prepare — and `PrepRack` does not
/// draw it. Kept rather than deleted because F-37 mounts the Expert
/// panel on *Performance* for both decks, and the engine/FFI surface
/// behind it is already complete. Delete it if that plan changes; do not
/// let it rot unnoticed. (`SirenPadRow`, once kept beside it, is gone:
/// the box in `SirenRackGroup` is the siren now.)
///
/// The **Expert** siren panel (PRD §6.3): the individual knobs/buttons,
/// matching the real units' control surfaces. The echo section (TIME /
/// FEEDBACK / ECHO / FILTER / VOLUME + ECHO CUT) is the box's PT2399 and
/// colours every shot; below it, the controls that belong to one chip —
/// SPEED is the HK628 clock under Rifle Gun and Alarm, PITCH / RATE /
/// HOLD are the DS01E voice under Sine. Laser and Siren are fixed
/// SN76477 patches. Reads the deck's stored state; writes through model
/// methods (each pushes the full `set_siren_controls` / `set_siren_voice`).
/// An expandable section so it stays out of the way until needed.
struct SirenExpertPanel: View {
    /// Every write this panel makes, as closures.
    ///
    /// The panel took `WaveformAppModel` + `DeckSide` and called methods
    /// on them. It never read the model — only `deck` drives the render —
    /// but holding it made the panel unconstructible in a test, which is
    /// why Prep had no snapshot coverage at all. Same pattern as
    /// `PrepRipBarState`: values in, closures out.
    struct Callbacks {
        var onToggleExpert: () -> Void = {}
        var onDelay: (Double) -> Void = { _ in }
        var onFeedback: (Double) -> Void = { _ in }
        var onMix: (Double) -> Void = { _ in }
        var onFilter: (Double) -> Void = { _ in }
        var onVolume: (Double) -> Void = { _ in }
        var onEchoCut: (Bool) -> Void = { _ in }
        var onSpeed: (Double) -> Void = { _ in }
        var onPitch: (Int) -> Void = { _ in }
        var onRate: (Double) -> Void = { _ in }
        var onContinuous: (Bool) -> Void = { _ in }
    }

    /// The deck's current state (read). The only thing the body reads.
    let deck: DeckState
    var callbacks = Callbacks()

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            Button {
                callbacks.onToggleExpert()
            } label: {
                Text(deck.sirenExpertShown ? "EXPERT ▾" : "EXPERT ▸")
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
            }
            .buttonStyle(.plain)

            if deck.sirenExpertShown {
                // Shared echo section (the PT2399, every unit).
                knob("TIME", deck.sirenDelayMs, 50...1000) { callbacks.onDelay($0) }
                knob("FEEDBACK", deck.sirenFeedback, 0...1.0) { callbacks.onFeedback($0) }
                knob("ECHO", deck.sirenMix, 0...1) { callbacks.onMix($0) }
                knob("FILTER", deck.sirenFilter, 0...1) { callbacks.onFilter($0) }
                knob("VOLUME", deck.sirenVolume, 0...1.5) { callbacks.onVolume($0) }
                DubPadCell("ECHO CUT", size: .chip)
                    .onPressHold(
                        onDown: { callbacks.onEchoCut(true) },
                        onUp: { callbacks.onEchoCut(false) })
                    .help("Echo cut — hold to mute the echo (the loop keeps running underneath)")

                // The chip shots' clock (Rifle Gun · Alarm).
                knob("SPEED", deck.sirenSpeed, 0.25...4.0) { callbacks.onSpeed($0) }
                // The DS01E voice under Sine: PITCH / RATE / TRIGGER.
                HStack(spacing: DubSpacing.sm) {
                    Text("PITCH")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .frame(width: 70, alignment: .leading)
                    Picker(
                        "Pitch",
                        selection: Binding(
                            get: { deck.sirenPitchIndex },
                            set: { callbacks.onPitch($0) })
                    ) {
                        Text("Lo").tag(0)
                        Text("Mid").tag(1)
                        Text("Hi").tag(2)
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .controlSize(.mini)
                    .frame(width: 150)
                }
                knob("RATE", deck.sirenRate, 0...12.0) { callbacks.onRate($0) }
                Toggle(
                    isOn: Binding(
                        get: { deck.sirenContinuous },
                        set: { callbacks.onContinuous($0) })
                ) {
                    Text("HOLD (continuous)")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                }
                .toggleStyle(.switch)
                .controlSize(.mini)
            }
        }
    }

    /// One labelled mini-slider row.
    @ViewBuilder
    private func knob(
        _ label: String,
        _ value: Double,
        _ range: ClosedRange<Double>,
        _ set: @escaping (Double) -> Void
    ) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Text(label)
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .frame(width: 70, alignment: .leading)
            Slider(value: Binding(get: { value }, set: { set($0) }), in: range)
                .controlSize(.mini)
                .tint(DubColor.siren)
                .frame(maxWidth: 180)
        }
    }
}

/// Vintage-FX rack labels + accent colours, in `RackFx` order (index = slot id:
/// 0 Spring · 1 Space Echo · 2 Big Knob · 3 Phaser).
private let rackFxLabels = ["SPRING", "SPACE ECHO", "BIG KNOB", "PHASER"]
private let rackFxColors: [Color] = [
    DubColor.springFx, DubColor.spaceEcho, DubColor.bigKnob, DubColor.phaser,
]

/// The vintage-FX rack panel (PRD §6.3 — the King Tubby / Lee Perry processing
/// chain). One row per effect: a tap-toggle engage button + a single **macro**
/// "super-knob" (Advanced mode) that fans across the effect's params on a
/// hand-tuned curve. Spring + Space Echo are reverb/echo sends; Big Knob +
/// Phaser are inserts. (Expert per-param panels arrive later.)
///
/// The toggle is a momentary tap (within the §1 mouse rule). The macro slider
/// is the one-knob Advanced control; live-riding it from a real controller is
/// the intended performance path, the on-screen slider is for prep/testing.
struct RackFxRow: View {
    /// Engaged flag per slot (index = `RackFx` id).
    let active: [Bool]
    /// Macro position per slot, 0..1.
    let macro: [Double]
    /// Toggle slot `index`.
    let onToggle: (_ index: Int) -> Void
    /// Set slot `index`'s macro to `value`.
    let onMacro: (_ index: Int, _ value: Double) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(active.contains(true) ? DubColor.bigKnob : DubColor.divider)
                    .frame(width: 7, height: 7)
                Text("FX RACK")
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
            }
            ForEach(0..<rackFxLabels.count, id: \.self) { slot in
                slotRow(slot)
            }
        }
    }

    @ViewBuilder
    private func slotRow(_ slot: Int) -> some View {
        let on = slot < active.count && active[slot]
        let tint = rackFxColors[slot]
        HStack(spacing: DubSpacing.sm) {
            DubPadCell(size: .slot, lit: on, tint: tint) {
                Text(rackFxLabels[slot])
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
            }
                .onPressDown { onToggle(slot) }
                .help("\(rackFxLabels[slot]) — tap to engage / bypass")
            Slider(
                value: Binding(
                    get: { slot < macro.count ? macro[slot] : 0.5 },
                    set: { onMacro(slot, $0) }),
                in: 0...1)
                .controlSize(.mini)
                .tint(tint)
                // Compresses rather than clipping: 92 pt of label plus
                // a fixed 104 pt slider overruns the 224 pt deck
                // column. Prep's wider column still gets the full 104.
                .frame(minWidth: 60, maxWidth: 104)
                .help("Macro — one knob, curated for a good-sounding result")
        }
    }
}

extension View {
    /// Fire `perform` on mouse-**down** (press), not mouse-up.
    ///
    /// Use for timing-sensitive taps — hot cues, the BPM tap — where
    /// the handler captures the live playhead (or a wall-clock tap
    /// timestamp) at the click instant. A SwiftUI `Button` fires its
    /// action on mouse-**up**, so on a playing deck the captured
    /// point lands the whole click-hold duration (tens of ms) too
    /// late; the keyboard path fires on key-down and is correct, so
    /// the two disagreed. Pressing down matches the keyboard and a
    /// real hardware pad (note-on = press). `enabled == false` makes
    /// it inert (left-click does nothing) while leaving any sibling
    /// `contextMenu` / right-click reachable.
    func onPressDown(enabled: Bool = true, perform: @escaping () -> Void) -> some View {
        modifier(PressDownModifier(enabled: enabled, perform: perform))
    }

    /// Fire `onDown` on mouse-down and `onUp` on release — a momentary
    /// press-and-hold (e.g. ECHO CUT: cut while held, restore on release).
    func onPressHold(
        onDown: @escaping () -> Void,
        onUp: @escaping () -> Void
    ) -> some View {
        modifier(PressHoldModifier(onDown: onDown, onUp: onUp))
    }
}

private struct PressHoldModifier: ViewModifier {
    let onDown: () -> Void
    let onUp: () -> Void
    @State private var pressing = false

    func body(content: Content) -> some View {
        content
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in
                        guard !pressing else { return }
                        pressing = true
                        onDown()
                    }
                    .onEnded { _ in
                        pressing = false
                        onUp()
                    })
    }
}

private struct PressDownModifier: ViewModifier {
    let enabled: Bool
    let perform: () -> Void
    @State private var pressing = false

    func body(content: Content) -> some View {
        content
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in
                        guard enabled, !pressing else { return }
                        pressing = true
                        perform()
                    }
                    .onEnded { _ in pressing = false })
    }
}


#Preview("Performance pads") {
    HStack(spacing: 1) {
        PerformancePadsView(
            side: .a, state: PerformancePadsState(cues: [12.0, nil, 48.5, nil]))
        PerformancePadsView(
            side: .b, state: PerformancePadsState(cues: [nil, 4.0, nil, nil]))
    }
    .frame(width: 800, height: 360)
    .background(DubColor.surface0)
}
