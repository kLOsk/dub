//
//  AudioRoutingSection.swift
//  Dub
//
//  Preferences ▸ Audio in Performance: the interface, and where each
//  deck comes in and goes out (canvas "Dub Audio Routing", 2026-11-15).
//  A box `devices.toml` knows and someone has tried is shown read-only.
//  Any other box — unknown, or known only by a guess nobody has tried —
//  the DJ routes: a pair per deck in and out, one deck or two, a meter on each
//  input saying what arrives there — timecode, music or nothing — so the
//  right pair finds itself, and a test tone on each output to see which
//  mixer channel lights. Applying reconnects the interface, which
//  empties the decks, so nothing here moves while a deck plays.
//

import DubCore
import SwiftUI

struct AudioRoutingSection: View {
    let model: WaveformAppModel
    let device: AudioDeviceInfo
    @ObservedObject private var deckA: DeckStore
    @ObservedObject private var deckB: DeckStore

    @State private var draft: AudioRouting
    @State private var inputs: UInt32 = 0
    @State private var copied = false
    private let known: KnownInterfaceInfo?

    init(model: WaveformAppModel, device: AudioDeviceInfo) {
        self.model = model
        self.device = device
        self._deckA = ObservedObject(wrappedValue: model.deckStore(.a))
        self._deckB = ObservedObject(wrappedValue: model.deckStore(.b))
        self.known = model.knownInterface(device)
        self._draft = State(initialValue: AudioRoutingStore.load(device.name) ?? model.currentRouting(device))
    }

    /// The DJ sets the routing unless Dub ships a map someone has tried
    /// on the real box. A shipped guess ("KNOWN · UNTESTED") is where the
    /// pickers start, and stays the DJ's to correct: it was read off a
    /// manual, and a wrong one used to be locked in.
    private var editable: Bool { known?.verified != true }

    private var outputs: UInt32 { device.outputChannels }
    private var playing: Bool { deckA.state.isPlaying || deckB.state.isPlaying }
    private var problem: (fault: AudioRouting.Fault, message: String)? {
        editable && inputs > 0 ? draft.problem(inputs: inputs, outputs: outputs) : nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xl) {
            section("Interface") { interfaceCard }
            section("Routing") {
                if editable {
                    HStack {
                        Spacer(minLength: 0)
                        deckCountSwitch
                    }
                }
                routingTable
                if let problem {
                    clash(problem.message)
                }
                Text(
                    editable
                        ? "The meter reads what arrives on that pair — timecode, music or nothing — so drop the needle and the right pair finds itself. TEST sends a short tone to that pair to see which mixer channel lights."
                        : "Channel numbers as printed on the back of the interface. Dub ships this interface's map, so there is nothing to set. TEST sends a short tone to a deck's pair to see which mixer channel lights."
                )
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            }
            if editable {
                HStack(alignment: .firstTextBaseline) {
                    Text(
                        playing
                            ? "Locked while a deck plays."
                            : "Applies when you change it: the interface reconnects and both decks empty."
                    )
                    .font(DubFont.micro)
                    .foregroundStyle(playing ? DubColor.stateTentative : DubColor.textTertiary)
                    Spacer(minLength: DubSpacing.md)
                    Button("Forget this interface") {
                        model.setAudioRouting(nil, for: device)
                        draft = model.currentRouting(device)
                    }
                    .buttonStyle(.plain)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.controlAccent)
                    .disabled(playing || !model.hasSavedRouting(device))
                }
                shareRow
            }
        }
        .onAppear { inputs = model.inputChannelCount(device) }
        .onChange(of: draft) { next in
            guard editable, inputs > 0, next.problem(inputs: inputs, outputs: outputs) == nil,
                next != model.currentRouting(device) || !model.hasSavedRouting(device)
            else { return }
            model.setAudioRouting(next, for: device)
        }
    }

    // MARK: - Interface

    private var interfaceCard: some View {
        HStack(spacing: DubSpacing.md) {
            Image(systemName: "rectangle.connected.to.line.below")
                .foregroundStyle(DubColor.textSecondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(known?.displayName ?? device.name)
                    .font(DubFont.body.weight(.semibold))
                    .foregroundStyle(DubColor.textPrimary)
                Text("\(inputs > 0 ? "\(inputs) in · " : "")\(outputs) out · Performance mode")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
            }
            Spacer(minLength: DubSpacing.md)
            badge
        }
        .padding(.horizontal, 14)
        .padding(.vertical, DubSpacing.md)
        .background(DubColor.surface1)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(DubColor.divider, lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var badge: some View {
        let (word, colour): (String, Color) = {
            switch known?.verified {
            case true?: return ("KNOWN · TESTED", DubColor.stateLocked)
            case false?:
                return (model.hasSavedRouting(device) ? "SET BY YOU" : "KNOWN · UNTESTED", DubColor.stateTentative)
            case nil: return (model.hasSavedRouting(device) ? "SET BY YOU" : "NOT SET UP", DubColor.stateTentative)
            }
        }()
        return HStack(spacing: 6) {
            Circle().fill(colour).frame(width: 6, height: 6)
            Text(word).font(DubFont.caps).tracking(0.6)
        }
        .foregroundStyle(colour)
        .padding(.horizontal, 9)
        .padding(.vertical, 3)
        .overlay(Capsule().stroke(colour.opacity(0.45), lineWidth: 1))
    }

    // MARK: - Routing

    private var deckCountSwitch: some View {
        HStack(spacing: 0) {
            countSegment("TWO DECKS", on: draft.twoDeck, enabled: inputs >= 4 && outputs >= 4) {
                draft = AudioRouting(
                    twoDeck: true, deckAIn: 1, deckBIn: 3, deckAOut: 1, deckBOut: 3)
            }
            countSegment("ONE DECK", on: !draft.twoDeck, enabled: true) {
                draft.twoDeck = false
            }
        }
        .background(DubColor.surface2)
        .clipShape(Capsule())
        .overlay(Capsule().stroke(DubColor.divider, lineWidth: 1))
        .disabled(playing)
    }

    private func countSegment(_ title: String, on: Bool, enabled: Bool, action: @escaping () -> Void)
        -> some View
    {
        Button(action: action) {
            Text(title)
                .font(DubFont.caps)
                .tracking(0.6)
                .foregroundStyle(on ? DubColor.surface0 : DubColor.textSecondary)
                .padding(.horizontal, DubSpacing.md)
                .padding(.vertical, 4)
                .background(on ? DubColor.controlAccent : Color.clear)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
    }

    private var routingTable: some View {
        VStack(spacing: 0) {
            HStack(spacing: DubSpacing.md) {
                Text("DECK").frame(width: 44, alignment: .leading)
                Text("NEEDLE IN").frame(maxWidth: .infinity, alignment: .leading)
                Text("TO THE MIXER").frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .font(DubFont.caps)
            .tracking(0.6)
            .foregroundStyle(DubColor.textTertiary)
            Divider()
            deckRow(.a)
            if draft.twoDeck || !editable {
                Divider()
                deckRow(.b)
            }
        }
        .background(DubColor.surface1)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(DubColor.divider, lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private func deckRow(_ side: DeckSide) -> some View {
        let isA = side == .a
        let shown = editable ? draft : model.currentRouting(device)
        let inFirst = isA ? shown.deckAIn : shown.deckBIn
        let outFirst = isA ? shown.deckAOut : shown.deckBOut
        let inFault = problem?.fault == (isA ? .deckAInput : .deckBInput)
        let outFault = problem?.fault == (isA ? .deckAOutput : .deckBOutput)
        return HStack(spacing: DubSpacing.md) {
            Text(isA ? "A" : "B")
                .font(.system(size: 15, weight: .heavy))
                .foregroundStyle(isA ? DubColor.deckATint : DubColor.deckBTint)
                .frame(width: 44, alignment: .leading)
            HStack(spacing: DubSpacing.sm) {
                pairPicker(
                    "Deck \(isA ? "A" : "B") input", first: inFirst, channels: inputs, fault: inFault
                ) { v in
                    if isA { draft.deckAIn = v } else { draft.deckBIn = v }
                }
                InputSignal(engine: model.engine, deckIdx: side.ffiDeckIdx)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: DubSpacing.sm) {
                pairPicker(
                    "Deck \(isA ? "A" : "B") output", first: outFirst, channels: outputs, fault: outFault
                ) { v in
                    if isA { draft.deckAOut = v } else { draft.deckBOut = v }
                }
                Button("TEST") { model.testTone(side) }
                    .buttonStyle(.plain)
                    .font(DubFont.caps)
                    .tracking(0.6)
                    .foregroundStyle(DubColor.textPrimary)
                    .padding(.horizontal, 9)
                    .padding(.vertical, 5)
                    .background(DubColor.surface2)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(DubColor.divider, lineWidth: 1))
                    .disabled(isA ? deckA.state.isPlaying : deckB.state.isPlaying)
                    .help("A short tone on deck \(isA ? "A" : "B")'s pair — which mixer channel lights?")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(inFault || outFault ? DubColor.stateError.opacity(0.08) : Color.clear)
    }

    /// A pair, read-only on a known interface, a menu on any other.
    @ViewBuilder
    private func pairPicker(
        _ label: String, first: UInt32, channels: UInt32, fault: Bool,
        set: @escaping (UInt32) -> Void
    ) -> some View {
        if !editable || channels == 0 {
            Text(AudioRouting.label(first))
                .font(DubFont.numericInline)
                .foregroundStyle(DubColor.textPrimary)
                .accessibilityLabel("\(label) \(AudioRouting.label(first))")
        } else {
            Picker(label, selection: Binding(get: { first }, set: set)) {
                ForEach(AudioRouting.pairs(channels: channels), id: \.self) { p in
                    Text(AudioRouting.label(p)).tag(p)
                }
            }
            .labelsHidden()
            .pickerStyle(.menu)
            .fixedSize()
            .disabled(playing)
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .stroke(fault ? DubColor.stateError : Color.clear, lineWidth: 1))
        }
    }

    /// A routing that works is worth more than this rig: copied as an
    /// entry for the list of interfaces Dub ships a map for, it can be
    /// sent in, and the next DJ with this box sets nothing up.
    @ViewBuilder
    private var shareRow: some View {
        if model.hasSavedRouting(device),
            let entry = draft.registryEntry(deviceName: device.name, outputChannels: outputs)
        {
            HStack(alignment: .firstTextBaseline) {
                Text("Works on your rig? Copy this interface's map and send it in, and Dub will know the box.")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: DubSpacing.md)
                Button(copied ? "Copied" : "Copy map") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(entry, forType: .string)
                    copied = true
                }
                .buttonStyle(.plain)
                .font(DubFont.micro)
                .foregroundStyle(DubColor.controlAccent)
            }
        }
    }

    private func clash(_ message: String) -> some View {
        HStack(alignment: .top, spacing: DubSpacing.sm) {
            Image(systemName: "exclamationmark.circle")
                .foregroundStyle(DubColor.stateError)
            Text(message + " Dub keeps the last routing that worked until this is fixed.")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(DubSpacing.sm)
        .background(DubColor.stateError.opacity(0.08))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(DubColor.stateError.opacity(0.4), lineWidth: 1))
        .accessibilityElement(children: .combine)
    }

    private func section<C: View>(_ title: String, @ViewBuilder _ content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            Text(title.uppercased())
                .font(DubFont.caps)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textSecondary)
            content()
        }
    }
}

/// What arrives on a deck's input pair right now: five segments and a
/// word. Read off the engine ten times a second — only while the sheet
/// is open. The welcome guide's signal check shares `segments`.
struct InputSignal: View {
    let engine: DubEngine
    let deckIdx: UInt64

    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.1)) { _ in
            let t = engine.deckTelemetry(deckIdx: deckIdx)
            let lit = Self.segments(rms: t.inputRms)
            let (word, colour) = Self.word(sourceClass: t.sourceClass, lit: lit, hasInput: t.hasTimecodeInput)
            HStack(spacing: DubSpacing.sm) {
                HStack(spacing: 2) {
                    ForEach(0..<5, id: \.self) { i in
                        RoundedRectangle(cornerRadius: 1)
                            .fill(i < lit ? colour : DubColor.surface3)
                            .frame(width: 4, height: 10)
                    }
                }
                Text(word)
                    .font(DubFont.micro)
                    .foregroundStyle(colour)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Input: \(word)")
        }
    }

    /// One segment per 8 dB above −48 dBFS RMS.
    static func segments(rms: Float) -> Int {
        guard rms > 1e-6 else { return 0 }
        let db = 20 * log10(rms)
        return max(0, min(5, Int(((db + 48) / 8).rounded(.up))))
    }

    static func word(sourceClass: UInt8, lit: Int, hasInput: Bool) -> (String, Color) {
        guard hasInput else { return ("not open", DubColor.textTertiary) }
        guard lit > 0 else { return ("nothing", DubColor.textTertiary) }
        switch sourceClass {
        case 1: return ("timecode", DubColor.stateLocked)
        case 2: return ("music, not timecode", DubColor.stateTentative)
        default: return ("signal", DubColor.textSecondary)
        }
    }
}
