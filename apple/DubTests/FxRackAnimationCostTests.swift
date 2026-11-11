import SwiftUI
import XCTest

@testable import Dub

/// The FX rack's animations run on Core Animation (FxLayerAnimations).
/// As `TimelineView`s they cost the main thread a third of a core while
/// the units were in — 1 003 ms of CPU over 3 s on the Intel MacBook
/// Pro, against 21 ms after (2026-10-03). The ceiling here is fifteen
/// times the measured cost; the old code fails it by three.
final class FxRackAnimationCostTests: XCTestCase {

    /// Main-thread CPU spent while the rack sits on screen with every unit
    /// in and animating — the cost of its animations, not of building it.
    func testTimeTheRackWhileItAnimates() {
        var s = FxChannelState()
        s.side = .b
        s.motion = true
        s.active = [true, true, true, true]
        var c = FxRackControls()
        c.spaceEchoIntensity = 1.1
        s.controls = c
        let window = NSWindow(
            contentRect: CGRect(x: 0, y: 0, width: 486, height: 560),
            styleMask: [.titled], backing: .buffered, defer: false)
        let host = NSHostingView(rootView: FxRackColumn(state: s).frame(width: 486, height: 560))
        window.contentView = host
        window.orderFront(nil)
        RunLoop.main.run(until: Date().addingTimeInterval(1))
        let before = Self.mainThreadCPU()
        RunLoop.main.run(until: Date().addingTimeInterval(3))
        let used = Self.mainThreadCPU() - before
        window.orderOut(nil)
        XCTAssertLessThan(used, 0.3, "the rack's animations cost \(Int(used * 1000)) ms of main thread in 3 s")
    }

    private static func mainThreadCPU() -> Double {
        var info = thread_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<thread_basic_info>.size / MemoryLayout<integer_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                thread_info(mach_thread_self(), thread_flavor_t(THREAD_BASIC_INFO), $0, &count)
            }
        }
        guard kr == KERN_SUCCESS else { return 0 }
        return Double(info.user_time.seconds) + Double(info.user_time.microseconds) / 1e6
            + Double(info.system_time.seconds) + Double(info.system_time.microseconds) / 1e6
    }
}
