//
//  AudioRouting.swift
//  Dub
//
//  Where each deck comes in and goes out on an interface Dub has no
//  map for (Preferences ▸ Audio; canvas "Dub Audio Routing",
//  2026-11-15). `devices.toml` knows two boxes and only the SL 3 is
//  played on; any other interface used to get one deck on inputs 1+2
//  and a summed output, with no way out short of a new build. Now the
//  DJ picks the pairs, once per interface, and the engine starts with
//  them (`set_routing_override`, FFI 87).
//
//  Pure, so the rules are tested (`AudioRoutingTests`). Channel numbers
//  are 1-based, as printed on the back of the interface; a pair is two
//  neighbours starting on an odd channel, the way every interface
//  labels its stereo pairs.
//

import DubCore
import Foundation

struct AudioRouting: Codable, Equatable {
    var twoDeck: Bool
    /// First channel of each deck's pair.
    var deckAIn: UInt32
    var deckBIn: UInt32
    var deckAOut: UInt32
    var deckBOut: UInt32

    /// The stereo pairs a device with `channels` channels offers, by
    /// their first channel: 1, 3, 5, …
    static func pairs(channels: UInt32) -> [UInt32] {
        guard channels >= 2 else { return [] }
        return Array(stride(from: UInt32(1), through: channels - 1, by: 2))
    }

    static func label(_ first: UInt32) -> String { "\(first) – \(first + 1)" }

    /// Where an unknown interface starts: two decks on the first two
    /// pairs when it has them, else one deck on the first.
    static func starting(inputs: UInt32, outputs: UInt32) -> AudioRouting {
        let two = inputs >= 4 && outputs >= 4
        return AudioRouting(twoDeck: two, deckAIn: 1, deckBIn: two ? 3 : 1, deckAOut: 1, deckBOut: two ? 3 : 1)
    }

    init(twoDeck: Bool, deckAIn: UInt32, deckBIn: UInt32, deckAOut: UInt32, deckBOut: UInt32) {
        self.twoDeck = twoDeck
        self.deckAIn = deckAIn
        self.deckBIn = deckBIn
        self.deckAOut = deckAOut
        self.deckBOut = deckBOut
    }

    /// What a start would use, read back from the engine.
    init(_ r: PerformanceRouting) {
        twoDeck = r.twoDeck
        deckAIn = r.deckAInput.first ?? 1
        deckBIn = r.deckBInput.first ?? 3
        deckAOut = r.deckAOutputFirst
        deckBOut = r.deckBOutputFirst
    }

    func ffi(outputChannels: UInt32) -> PerformanceRouting {
        PerformanceRouting(
            deckAInput: [deckAIn, deckAIn + 1],
            deckBInput: [deckBIn, deckBIn + 1],
            outputChannels: outputChannels,
            deckAOutputFirst: deckAOut,
            deckBOutputFirst: twoDeck ? deckBOut : deckAOut,
            twoDeck: twoDeck)
    }

    /// Which deck's input pair is wrong, for the picker to mark.
    enum Fault: Equatable { case deckBInput, deckBOutput, deckAInput, deckAOutput }

    /// Why this cannot run on a device with these channel counts, in the
    /// DJ's words — or nil when it can.
    func problem(inputs: UInt32, outputs: UInt32) -> (fault: Fault, message: String)? {
        if deckAIn + 1 > inputs {
            return (.deckAInput, "This interface has \(inputs) inputs; deck A can't take \(Self.label(deckAIn)).")
        }
        if deckAOut + 1 > outputs {
            return (.deckAOutput, "This interface has \(outputs) outputs; deck A can't go out on \(Self.label(deckAOut)).")
        }
        guard twoDeck else { return nil }
        if deckBIn + 1 > inputs {
            return (.deckBInput, "This interface has \(inputs) inputs; deck B can't take \(Self.label(deckBIn)).")
        }
        if deckBOut + 1 > outputs {
            return (.deckBOutput, "This interface has \(outputs) outputs; deck B can't go out on \(Self.label(deckBOut)).")
        }
        if deckAIn == deckBIn {
            return (.deckBInput,
                "Deck A already takes inputs \(Self.label(deckAIn)). Pick another pair for deck B, or switch to one deck.")
        }
        if deckAOut == deckBOut {
            return (.deckBOutput,
                "Deck A already goes out on \(Self.label(deckAOut)). Pick another pair for deck B, or switch to one deck.")
        }
        return nil
    }
}

/// The DJ's routings, one per interface, by its name — the name is what
/// the engine and `devices.toml` match on.
enum AudioRoutingStore {
    static let key = "dub.audioRouting.v1"

    static func load(_ device: String, defaults: UserDefaults = .standard) -> AudioRouting? {
        all(defaults)[device]
    }

    static func save(_ routing: AudioRouting?, for device: String, defaults: UserDefaults = .standard) {
        var map = all(defaults)
        map[device] = routing
        if let data = try? JSONEncoder().encode(map) {
            defaults.set(data, forKey: key)
        }
    }

    private static func all(_ defaults: UserDefaults) -> [String: AudioRouting] {
        guard let data = defaults.data(forKey: key),
            let map = try? JSONDecoder().decode([String: AudioRouting].self, from: data)
        else { return [:] }
        return map
    }
}
