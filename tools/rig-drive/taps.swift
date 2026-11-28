// taps X Y COUNT INTERVAL_MS — COUNT clicks on one spot, a press every
// INTERVAL_MS (tap tempo). Prints each press's offset from the first, ms.
import CoreGraphics
import Foundation

let a = CommandLine.arguments
guard a.count == 5, let x = Double(a[1]), let y = Double(a[2]), let count = Int(a[3]),
      let interval = Double(a[4]) else {
    FileHandle.standardError.write("usage: taps X Y COUNT INTERVAL_MS\n".data(using: .utf8)!)
    exit(2)
}
let p = CGPoint(x: x, y: y)
let src = CGEventSource(stateID: .hidSystemState)
func post(_ t: CGEventType) {
    CGEvent(mouseEventSource: src, mouseType: t, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
}
post(.mouseMoved); usleep(150_000)
let start = Date()
for i in 0..<count {
    let due = start.addingTimeInterval(Double(i) * interval / 1000)
    let wait = due.timeIntervalSinceNow
    if wait > 0 { usleep(UInt32(wait * 1_000_000)) }
    post(.leftMouseDown)
    print(String(format: "%.1f", Date().timeIntervalSince(start) * 1000))
    usleep(40_000); post(.leftMouseUp)
}
