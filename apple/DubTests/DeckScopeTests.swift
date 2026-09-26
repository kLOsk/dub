import SwiftUI
import XCTest

@testable import Dub

/// Rig, 2026-09-26: after the model split, controls reacted seconds late
/// or not at all. The first thing to hold down: a `DeckScope` re-runs
/// its content when a deck store changes.
final class DeckScopeTests: XCTestCase {
    final class Counter { var n = 0 }

    func testADeckStoreChangeReRunsTheScope() {
        let a = DeckStore(label: "a"), b = DeckStore(label: "b"), sampler = SamplerStore()
        let counter = Counter()
        let view = DeckScope(a: a, b: b, sampler: sampler) {
            counter.n += 1
            return Text(a.state.isPlaying ? "playing" : "stopped")
        }
        let host = NSHostingView(rootView: view)
        host.frame = CGRect(x: 0, y: 0, width: 200, height: 40)
        let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        let before = counter.n
        var s = a.state
        s.isPlaying = true
        a.state = s
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        host.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(counter.n, before, "the scope did not re-run on a deck change")
    }
}
