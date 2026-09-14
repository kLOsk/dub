//
//  SirenDisplay.swift
//  Dub
//
//  The siren box's window: a moving-coil VU meter. A Sifam face in
//  aged cream, the black scale with its red +3 zone, the shot's name
//  printed where "VU" would be, and a needle that rests on its left
//  stop, throws into the red on a shot and falls back through the
//  repeats.
//
//  **The needle is eye candy, by decision.** The siren bus has no level
//  tap — the app knows `siren_state` (0/1), the last shot, and what the
//  DUB knob set. The ballistics here are drawn from those: a VU-speed
//  rise on a press, a kick once per echo repeat while it sounds, and a
//  tail that decays by the feedback per delay after it stops. It looks
//  like what the box is doing; it does not measure it, and the file
//  says so rather than letting a reader assume a meter is metering.
//
//  The window is the only light object on the surface. That is a
//  choice too: it is the siren's one readout, and the cream pulls the
//  eye the way the real meter does on a black preamp.
//

import SwiftUI

/// The meter: the last shot on the dial, the echo line under it, the
/// needle driven by `sounding` and each press.
struct SirenDisplay: View {
    /// The last shot's name; `nil` before the first press (the dial then
    /// says READY).
    let shot: String?
    let sounding: Bool
    /// Presses so far — each one re-throws the needle, including a
    /// re-hit of the shot already sounding.
    var fireCount: Int = 0
    /// `false` when the knob is at DRY — no repeats, no tail.
    let echoOn: Bool
    let delayMs: Float
    /// 0…1; the tail decays by this per delay.
    let feedback: Float

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// When the needle last threw, and when the siren last stopped.
    /// `nil` until an edge is seen live — a view that starts out
    /// `sounding` (a snapshot, a relaunch mid-tail) draws a still needle
    /// in the red rather than a swing from nowhere.
    @State private var firedAt: Date?
    @State private var releasedAt: Date?
    /// The tail has died away: the timeline stops ticking.
    @State private var resting = true
    @State private var restTask: Task<Void, Never>?

    /// VU ballistics: full swing in ~0.3 s at the standard, faster here
    /// because a shot is short.
    private static let rise: TimeInterval = 0.06
    /// The fall with no echo to hold it up.
    private static let dryFall: TimeInterval = 0.25
    /// Where a shot throws the needle: into the red.
    private static let throwLevel = 0.9

    private var delay: TimeInterval { max(TimeInterval(delayMs) / 1000, 0.05) }
    private var repeats: Bool { echoOn && feedback > 0.02 }

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: resting && !sounding)) { context in
            SirenMeterFace(
                title: (shot ?? "READY").uppercased(),
                echoLine: echoOn ? "ECHO \(Int(delayMs.rounded())) ms" : "DRY",
                level: needleLevel(at: context.date))
        }
        .frame(width: DubLayout.sirenDisplayWidth, height: DubLayout.sirenDisplayHeight)
        .background(DubColor.displayWell)
        .clipShape(RoundedRectangle(cornerRadius: 2, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .stroke(DubColor.surface2, lineWidth: 1))
        .onChange(of: sounding) { now in now ? threw() : stopped() }
        .onChange(of: fireCount) { _ in threw() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(shot.map { "Siren meter, last shot \($0)" } ?? "Siren meter, ready")
        .accessibilityValue(sounding ? "sounding" : "at rest")
    }

    /// 0 = on the left stop, 1 = the right end of the scale.
    private func needleLevel(at now: Date) -> Double {
        guard let fired = firedAt else {
            return sounding ? Self.throwLevel : 0
        }
        if sounding {
            let t = now.timeIntervalSince(fired)
            let swing = 1 - exp(-t / Self.rise)
            // A kick at every repeat: up on the beat of the delay, easing
            // off until the next one.
            let kick = (repeats && !reduceMotion) ? 0.22 * (1 - fraction(t / delay)) : 0
            return min(1, (Self.throwLevel - 0.15 + kick) * swing)
        }
        guard let released = releasedAt else { return 0 }
        // The tail — and, for a press the engine never sounded (stopped,
        // or the gate off), the rise and the fall overlapping into one
        // flick of the needle.
        let swing = 1 - exp(-now.timeIntervalSince(fired) / Self.rise)
        let te = max(0, now.timeIntervalSince(released))
        let tail: Double
        if repeats {
            let kick = reduceMotion ? 1 : 0.7 + 0.3 * (1 - fraction(te / delay))
            tail = pow(Double(feedback), te / delay) * kick
        } else {
            tail = exp(-te / Self.dryFall)
        }
        return min(1, Self.throwLevel * swing * tail)
    }

    private func fraction(_ x: Double) -> Double { x - floor(x) }

    /// How long the tail is audible after the siren stops — when the
    /// needle can be left alone and the timeline paused.
    private var tailSeconds: TimeInterval {
        guard repeats else { return Self.dryFall * 4 }
        let fb = Double(min(feedback, 0.97))
        // Down to 1 % of the throw.
        return delay * log(0.01) / log(fb) + delay
    }

    private func threw() {
        firedAt = .now
        resting = false
        if sounding {
            restTask?.cancel()
            releasedAt = nil
        } else {
            // Nothing will sound (engine stopped, gate off): a flick and
            // back to rest, so the timeline does not tick on for ever.
            releasedAt = .now
            scheduleRest()
        }
    }

    private func stopped() {
        releasedAt = .now
        scheduleRest()
    }

    private func scheduleRest() {
        restTask?.cancel()
        let tail = tailSeconds
        restTask = Task { @MainActor in
            try? await Task.sleep(for: .seconds(tail))
            guard !Task.isCancelled else { return }
            resting = true
        }
    }
}
