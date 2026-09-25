//
//  ModelStores.swift
//  Dub
//
//  The model's fast-changing state, split out so a change rebuilds only
//  the views that show it.
//
//  `WaveformAppModel` is one `ObservableObject` that nearly every view
//  observes, so any `@Published` change on it re-evaluated the root, the
//  performance surface and the library together — ~90 ms a time on the
//  rig with two timecode decks playing, which the Metal strips paid for
//  in dropped frames (rig, 2026-09-23). The decks and the sampler's
//  voices change far more often than anything else on it, so they live
//  here, each its own object. The model keeps `deckA` / `deckB` /
//  `samplerVoices` as plain accessors onto these, so its code reads as
//  before but no longer fires its own `objectWillChange` for them.
//
//  A view that reads deck or sampler state has to observe the store it
//  reads — `DeckScope` for a region, or the store itself for a view
//  SwiftUI may skip (its inputs look unchanged when the deck changes).
//

import Combine
import DubCore
import os
import SwiftUI

/// One deck's state. Published on any change; see `PublishRateGuard`.
final class DeckStore: ObservableObject {
    @Published var state: DeckState = .empty {
        didSet {
            #if DEBUG
            rateGuard.note(old: oldValue, new: state, playing: state.isPlaying)
            #endif
        }
    }

    #if DEBUG
    private let rateGuard: PublishRateGuard<DeckState>

    init(label: String) {
        rateGuard = PublishRateGuard(label: label)
    }
    #else
    init(label: String) {}
    #endif
}

/// The sampler's voices: lamps and sweeps. Changes every poll while a
/// sample sounds, which is exactly why it is not on the model.
final class SamplerStore: ObservableObject {
    @Published var voices: [SamplerSlotTelemetry] = []
}

/// Rebuilds `content` when either deck or the sampler changes, and
/// only then. A region of the performance surface that reads deck state
/// sits in one; the surface itself does not observe the stores, so the
/// library, the status strip and the root are left alone.
///
/// Both decks, not one: the regions read across them (the Quick Scratch
/// badge checks the other deck's tune, the rack bar both decks' sirens),
/// and a region watching one deck would go stale on the other's change.
/// The deck columns are `.equatable()`, so the deck that did not change
/// is not rebuilt anyway.
struct DeckScope<Content: View>: View {
    @ObservedObject var a: DeckStore
    @ObservedObject var b: DeckStore
    @ObservedObject var sampler: SamplerStore
    @ViewBuilder var content: () -> Content

    var body: some View { content() }
}

#if DEBUG
/// Warns when a store publishes more than a few times a second while a
/// deck plays, and names the fields that moved. The churn this split
/// removed — a live pitch, an input level — was invisible until a
/// profile found it; this makes the next one say so on the console.
final class PublishRateGuard<Value> {
    /// Publishes a second above which steady play is not steady.
    static var limitPerSecond: Int { 4 }
    /// How long the rate has to hold before it is reported, so a burst
    /// from one gesture (a load, a loop press) is not.
    static var sustainSecs: Double { 2 }

    private let label: String
    private var times: [TimeInterval] = []
    private var overSince: TimeInterval?
    private var lastReport: TimeInterval = 0
    private var changed: [String: Int] = [:]
    private let log = Logger(subsystem: "com.dub.app", category: "publish")

    init(label: String) {
        self.label = label
    }

    func note(old: Value, new: Value, playing: Bool) {
        let now = ProcessInfo.processInfo.systemUptime
        times.append(now)
        times.removeAll { now - $0 > 1 }
        guard playing, times.count > Self.limitPerSecond else {
            overSince = nil
            changed.removeAll()
            return
        }
        overSince = overSince ?? now
        for field in Self.changedFields(old, new) { changed[field, default: 0] += 1 }
        guard let since = overSince, now - since >= Self.sustainSecs,
              now - lastReport > 10
        else { return }
        lastReport = now
        let fields = changed.sorted { $0.value > $1.value }.prefix(5)
            .map { "\($0.key)×\($0.value)" }.joined(separator: " ")
        log.warning("\(self.label, privacy: .public) publishing \(self.times.count, privacy: .public)/s while playing — every observer rebuilds each time. Changing: \(fields, privacy: .public)")
    }

    /// The top-level fields that differ, by label.
    static func changedFields(_ old: Value, _ new: Value) -> [String] {
        zip(Mirror(reflecting: old).children, Mirror(reflecting: new).children).compactMap { a, b in
            String(describing: a.value) == String(describing: b.value) ? nil : a.label
        }
    }
}
#endif
