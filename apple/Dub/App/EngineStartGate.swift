//
//  EngineStartGate.swift
//  Dub
//
//  Which background engine start may commit.
//
//  Connecting a USB DJ interface froze the app for 10–13 s on the rig
//  (2026-09-23): the device probe and the timecode engine start ran on
//  the main thread while the interface's driver came up, and every
//  CoreAudio question waited on it. They run on a serial background
//  queue now and commit on the main thread — but by the time a start
//  lands, the DJ may have switched mode or pulled the cable, and a stop
//  or a newer start owns the engine. Each start takes a ticket; only
//  the newest, un-cancelled ticket commits.
//

struct EngineStartGate {
    private var generation: UInt64 = 0
    /// The device a start is bringing up, for the status strip.
    private(set) var connecting: String?

    var inFlight: Bool { connecting != nil }

    /// A start is about to go to the background. Supersedes any other.
    mutating func begin(device: String) -> UInt64 {
        generation &+= 1
        connecting = device
        return generation
    }

    /// A stop: whatever start is in flight must not commit.
    mutating func cancel() {
        generation &+= 1
        connecting = nil
    }

    /// A start has landed. `true` = it is still the one wanted; commit.
    mutating func finish(_ ticket: UInt64) -> Bool {
        guard ticket == generation else { return false }
        connecting = nil
        return true
    }
}
